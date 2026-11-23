//! glTF 2.0 writer: `.glb`, or `.gltf` JSON plus an external `.bin`.
//!
//! The whole [`Scene`] model is carried: node hierarchy and transforms (TRS or matrix as stored),
//! cameras, indexed triangle meshes (`POSITION`, `NORMAL`, `TEXCOORD_0`), material factors and texture
//! references, and translation/rotation/scale keyframes. What the model never held (skins, morph
//! targets, extensions, samplers) is not invented; the loss report says so once.

use std::collections::HashMap;

use serde_json::{Map, Number, Value, json};

use crate::util::Budget;
use crate::write::{ExportFile, ExportOptions, Target, charge};
use crate::{Error, Interpolation, Projection, Report, Resolver, Result, Scene, TargetPath, Transform, Units};

const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// A JSON number that reads back as the same `f32` (shortest decimal text of the `f32`).
fn num(x: f32) -> Value {
    let text = format!("{x}");
    text.parse::<f64>().ok().and_then(Number::from_f64).map_or(Value::Null, Value::Number)
}

fn nums(v: &[f32]) -> Value {
    Value::Array(v.iter().map(|x| num(*x)).collect())
}

/// Percent-encodes a relative URI (path separators stay).
fn uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The growing binary buffer with its views and accessors.
#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> Result<usize> {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let offset = self.data.len();
        if offset.checked_add(bytes.len()).is_none_or(|e| e > u32::MAX as usize - 8) {
            return Err(Error::Limit("the glTF buffer would exceed 4 GiB".into()));
        }
        self.data.extend_from_slice(bytes);
        let mut v = json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len()});
        if let (Some(t), Some(o)) = (target, v.as_object_mut()) {
            o.insert("target".into(), json!(t));
        }
        self.views.push(v);
        Ok(self.views.len() - 1)
    }

    /// An `f32` accessor over `data` (`comps` floats per element). `bounds` adds the min/max glTF requires
    /// for `POSITION` and animation input.
    fn floats(&mut self, data: &[f32], comps: usize, ty: &str, target: Option<u32>, bounds: bool) -> Result<usize> {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, target)?;
        let count = data.len() / comps.max(1);
        let mut a = json!({"bufferView": view, "componentType": 5126, "count": count, "type": ty});
        if bounds && let Some(o) = a.as_object_mut() {
            let mut min = vec![f32::INFINITY; comps];
            let mut max = vec![f32::NEG_INFINITY; comps];
            for el in data.chunks_exact(comps) {
                for (k, x) in el.iter().enumerate() {
                    if let (Some(lo), Some(hi)) = (min.get_mut(k), max.get_mut(k)) {
                        *lo = lo.min(*x);
                        *hi = hi.max(*x);
                    }
                }
            }
            o.insert("min".into(), nums(&min));
            o.insert("max".into(), nums(&max));
        }
        self.accessors.push(a);
        Ok(self.accessors.len() - 1)
    }

    fn indices(&mut self, idx: &[u32]) -> Result<usize> {
        let bytes: Vec<u8> = idx.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER))?;
        self.accessors.push(json!({"bufferView": view, "componentType": 5125, "count": idx.len(), "type": "SCALAR"}));
        Ok(self.accessors.len() - 1)
    }
}

fn flat3(v: &[[f32; 3]]) -> Vec<f32> {
    v.iter().flatten().copied().collect()
}

fn flat2(v: &[[f32; 2]]) -> Vec<f32> {
    v.iter().flatten().copied().collect()
}

fn mime(uri: &str) -> Option<&'static str> {
    let ext = uri.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        _ => None,
    }
}

/// The glTF material slot for a texture slot the model holds, and whether the mapping is an approximation.
fn slot_for(slot: &str) -> Option<(&'static str, bool)> {
    match slot {
        "baseColor" | "map_Kd" => Some(("baseColor", false)),
        "metallicRoughness" => Some(("metallicRoughness", false)),
        "normal" => Some(("normal", false)),
        "occlusion" => Some(("occlusion", false)),
        "emissive" | "map_Ke" => Some(("emissive", false)),
        "bump" | "map_Bump" | "map_bump" | "norm" => Some(("normal", true)),
        _ => None,
    }
}

struct Images {
    /// uri → texture index
    textures: HashMap<String, usize>,
    images: Vec<Value>,
    texture_list: Vec<Value>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write(
    scene: &Scene,
    opts: &ExportOptions,
    stem: &str,
    resolver: &mut dyn Resolver,
    budget: &mut Budget,
    report: &mut Report,
    textures_out: &mut Vec<String>,
) -> Result<Vec<ExportFile>> {
    let glb = opts.target == Target::Glb;
    // the size estimate: mesh data and animation keys (built twice while the container is assembled)
    let key_bytes: usize = scene.animations.iter().flat_map(|a| &a.channels).map(|c| (c.times.len() + c.values.len()) * 4).sum();
    charge(budget, scene.mesh_bytes().saturating_add(key_bytes).saturating_mul(2).saturating_add(scene.nodes.len().saturating_mul(256)).saturating_add(4096))?;

    if scene.units == Units::Unspecified {
        report.approximated("units", "the source states no unit or axis; the numbers are written unchanged as glTF metres, Y up");
    }
    let mut bin = Bin::default();
    let mut root = Map::new();
    let mut asset = Map::new();
    asset.insert("version".into(), json!("2.0"));
    asset.insert("generator".into(), json!(opts.generator));
    if let Some(c) = &scene.asset.copyright {
        asset.insert("copyright".into(), json!(c));
    }
    root.insert("asset".into(), Value::Object(asset));

    // materials and textures
    let mut imgs = Images { textures: HashMap::new(), images: Vec::new(), texture_list: Vec::new() };
    let mut materials = Vec::new();
    for (mi, m) in scene.materials.iter().enumerate() {
        let mut pbr = Map::new();
        pbr.insert("baseColorFactor".into(), nums(&m.base_color));
        pbr.insert("metallicFactor".into(), num(m.metallic));
        pbr.insert("roughnessFactor".into(), num(m.roughness));
        let mut mat = Map::new();
        if let Some(n) = &m.name {
            mat.insert("name".into(), json!(n));
        }
        if m.emissive != [0.0; 3] {
            mat.insert("emissiveFactor".into(), nums(&m.emissive));
        }
        if m.double_sided {
            mat.insert("doubleSided".into(), json!(true));
        }
        if m.base_color[3] < 1.0 {
            mat.insert("alphaMode".into(), json!("BLEND"));
            report.approximated(format!("materials[{mi}].alphaMode"), "the scene model has no alpha mode: an alpha below 1 is written as BLEND");
        }
        for t in &m.textures {
            let path = format!("materials[{mi}].{}", t.slot);
            let Some(uri) = t.uri.as_deref() else {
                report.unsupported(path, "an embedded texture image is not part of the scene model, so it cannot be written");
                continue;
            };
            let Some((slot, approx)) = slot_for(&t.slot) else {
                report.unsupported(path, format!("texture slot {:?} has no glTF material slot and is dropped", t.slot));
                continue;
            };
            if approx {
                report.approximated(path.clone(), format!("{:?} is written as the glTF normal map slot (it may be a bump map)", t.slot));
            }
            let index = match imgs.textures.get(uri) {
                Some(&i) => i,
                None => {
                    let mut image = Map::new();
                    let embedded = if glb && opts.embed_textures {
                        match (mime(uri), resolver.resolve(uri)) {
                            (Some(mt), Some(bytes)) => {
                                charge(budget, bytes.len())?;
                                let view = bin.view(&bytes, None)?;
                                image.insert("bufferView".into(), json!(view));
                                image.insert("mimeType".into(), json!(mt));
                                true
                            }
                            (None, _) => {
                                report.approximated(path.clone(), format!("{uri}: only PNG and JPEG can be embedded; left as a reference"));
                                false
                            }
                            (_, None) => {
                                report.approximated(path.clone(), format!("{uri}: the image was not available to embed; left as a reference"));
                                false
                            }
                        }
                    } else {
                        false
                    };
                    if !embedded {
                        image.insert("uri".into(), json!(uri_encode(uri)));
                        textures_out.push(uri.to_string());
                    }
                    imgs.images.push(Value::Object(image));
                    imgs.texture_list.push(json!({"source": imgs.images.len() - 1}));
                    imgs.textures.insert(uri.to_string(), imgs.texture_list.len() - 1);
                    imgs.texture_list.len() - 1
                }
            };
            let mut info = json!({"index": index});
            if t.texcoord != 0
                && let Some(o) = info.as_object_mut()
            {
                o.insert("texCoord".into(), json!(t.texcoord));
            }
            match slot {
                "baseColor" => {
                    pbr.insert("baseColorTexture".into(), info);
                }
                "metallicRoughness" => {
                    pbr.insert("metallicRoughnessTexture".into(), info);
                }
                "normal" => {
                    mat.insert("normalTexture".into(), info);
                }
                "occlusion" => {
                    mat.insert("occlusionTexture".into(), info);
                }
                _ => {
                    mat.insert("emissiveTexture".into(), info);
                }
            }
        }
        mat.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
        materials.push(Value::Object(mat));
    }

    // meshes (a mesh without a drawable primitive is dropped, with a note)
    let mut mesh_map: Vec<Option<usize>> = Vec::with_capacity(scene.meshes.len());
    let mut meshes = Vec::new();
    for (mi, mesh) in scene.meshes.iter().enumerate() {
        let mut prims = Vec::new();
        for (pi, p) in mesh.primitives.iter().enumerate() {
            if p.positions.is_empty() || p.indices.is_empty() {
                report.approximated(format!("meshes[{mi}].primitives[{pi}]"), "the primitive has no triangles and is not written");
                continue;
            }
            let mut attrs = Map::new();
            attrs.insert("POSITION".into(), json!(bin.floats(&flat3(&p.positions), 3, "VEC3", Some(ARRAY_BUFFER), true)?));
            if !p.normals.is_empty() {
                attrs.insert("NORMAL".into(), json!(bin.floats(&flat3(&p.normals), 3, "VEC3", Some(ARRAY_BUFFER), false)?));
            }
            if !p.texcoords.is_empty() {
                attrs.insert("TEXCOORD_0".into(), json!(bin.floats(&flat2(&p.texcoords), 2, "VEC2", Some(ARRAY_BUFFER), false)?));
            }
            let mut prim = Map::new();
            prim.insert("attributes".into(), Value::Object(attrs));
            prim.insert("indices".into(), json!(bin.indices(&p.indices)?));
            if let Some(m) = p.material {
                prim.insert("material".into(), json!(m));
            }
            prims.push(Value::Object(prim));
        }
        if prims.is_empty() {
            report.approximated(format!("meshes[{mi}]"), "the mesh has no drawable primitive and is not written");
            mesh_map.push(None);
            continue;
        }
        let mut m = Map::new();
        if let Some(n) = &mesh.name {
            m.insert("name".into(), json!(n));
        }
        m.insert("primitives".into(), Value::Array(prims));
        meshes.push(Value::Object(m));
        mesh_map.push(Some(meshes.len() - 1));
    }

    // cameras
    let cameras: Vec<Value> = scene
        .cameras
        .iter()
        .map(|c| {
            let mut o = Map::new();
            if let Some(n) = &c.name {
                o.insert("name".into(), json!(n));
            }
            match c.projection {
                Projection::Perspective { yfov, aspect_ratio, znear, zfar } => {
                    let mut p = Map::new();
                    p.insert("yfov".into(), num(yfov));
                    p.insert("znear".into(), num(znear));
                    if let Some(a) = aspect_ratio {
                        p.insert("aspectRatio".into(), num(a));
                    }
                    if let Some(z) = zfar {
                        p.insert("zfar".into(), num(z));
                    }
                    o.insert("type".into(), json!("perspective"));
                    o.insert("perspective".into(), Value::Object(p));
                }
                Projection::Orthographic { xmag, ymag, znear, zfar } => {
                    o.insert("type".into(), json!("orthographic"));
                    o.insert("orthographic".into(), json!({"xmag": num(xmag), "ymag": num(ymag), "znear": num(znear), "zfar": num(zfar)}));
                }
            }
            Value::Object(o)
        })
        .collect();

    // nodes
    let mut nodes = Vec::with_capacity(scene.nodes.len());
    for node in &scene.nodes {
        let mut o = Map::new();
        if let Some(n) = &node.name {
            o.insert("name".into(), json!(n));
        }
        if !node.children.is_empty() {
            o.insert("children".into(), json!(node.children));
        }
        match node.transform {
            Transform::Matrix(m) => {
                o.insert("matrix".into(), nums(&m));
            }
            Transform::Trs { translation, rotation, scale } => {
                if translation != [0.0; 3] {
                    o.insert("translation".into(), nums(&translation));
                }
                if rotation != [0.0, 0.0, 0.0, 1.0] {
                    o.insert("rotation".into(), nums(&rotation));
                }
                if scale != [1.0; 3] {
                    o.insert("scale".into(), nums(&scale));
                }
            }
        }
        if let Some(m) = node.mesh.and_then(|m| mesh_map.get(m).copied().flatten()) {
            o.insert("mesh".into(), json!(m));
        }
        if let Some(c) = node.camera {
            o.insert("camera".into(), json!(c));
        }
        nodes.push(Value::Object(o));
    }

    // animations
    let mut animations = Vec::new();
    for (ai, a) in scene.animations.iter().enumerate() {
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        for (ci, ch) in a.channels.iter().enumerate() {
            if ch.times.is_empty() {
                report.approximated(format!("animations[{ai}].channels[{ci}]"), "the channel has no keys and is not written");
                continue;
            }
            let (comps, ty, path) = match ch.target {
                TargetPath::Translation => (3, "VEC3", "translation"),
                TargetPath::Rotation => (4, "VEC4", "rotation"),
                TargetPath::Scale => (3, "VEC3", "scale"),
            };
            let input = bin.floats(&ch.times, 1, "SCALAR", None, true)?;
            let output = bin.floats(&ch.values, comps, ty, None, false)?;
            let interpolation = match ch.interpolation {
                Interpolation::Linear => "LINEAR",
                Interpolation::Step => "STEP",
                Interpolation::CubicSpline => "CUBICSPLINE",
            };
            samplers.push(json!({"input": input, "output": output, "interpolation": interpolation}));
            channels.push(json!({"sampler": samplers.len() - 1, "target": {"node": ch.node, "path": path}}));
        }
        if channels.is_empty() {
            continue;
        }
        let mut o = Map::new();
        if let Some(n) = &a.name {
            o.insert("name".into(), json!(n));
        }
        o.insert("channels".into(), Value::Array(channels));
        o.insert("samplers".into(), Value::Array(samplers));
        animations.push(Value::Object(o));
    }

    // assemble the document
    let mut put = |key: &str, v: Vec<Value>| {
        if !v.is_empty() {
            root.insert(key.into(), Value::Array(v));
        }
    };
    put("accessors", std::mem::take(&mut bin.accessors));
    put("animations", animations);
    put("bufferViews", std::mem::take(&mut bin.views));
    put("cameras", cameras);
    put("images", imgs.images);
    put("materials", materials);
    put("meshes", meshes);
    put("nodes", nodes);
    put("textures", imgs.texture_list);
    root.insert("scene".into(), json!(0));
    root.insert("scenes".into(), json!([{"nodes": scene.roots}]));
    let bin_name = format!("{stem}.bin");
    if !bin.data.is_empty() {
        let mut b = json!({"byteLength": bin.data.len()});
        if !glb && let Some(o) = b.as_object_mut() {
            o.insert("uri".into(), json!(uri_encode(&bin_name)));
        }
        root.insert("buffers".into(), json!([b]));
    }
    let doc = Value::Object(root);
    let json_bytes = if glb { serde_json::to_vec(&doc) } else { serde_json::to_vec_pretty(&doc) }.map_err(|e| Error::Invalid(format!("glTF JSON: {e}")))?;

    if !glb {
        let mut files = vec![ExportFile { name: format!("{stem}.gltf"), data: json_bytes }];
        if !bin.data.is_empty() {
            files.push(ExportFile { name: bin_name, data: bin.data });
        }
        return Ok(files);
    }
    Ok(vec![ExportFile { name: format!("{stem}.glb"), data: glb_container(json_bytes, bin.data)? }])
}

fn pad(data: &mut Vec<u8>, with: u8) {
    while !data.len().is_multiple_of(4) {
        data.push(with);
    }
}

fn glb_container(mut json: Vec<u8>, mut bin: Vec<u8>) -> Result<Vec<u8>> {
    pad(&mut json, b' ');
    pad(&mut bin, 0);
    let total = 12usize
        .checked_add(8 + json.len())
        .and_then(|t| if bin.is_empty() { Some(t) } else { t.checked_add(8 + bin.len()) })
        .filter(|t| *t <= u32::MAX as usize)
        .ok_or_else(|| Error::Limit("the GLB would exceed 4 GiB".into()))?;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    Ok(out)
}
