//! glTF 2.0 reader (`.gltf` JSON with external or data-URI buffers, and `.glb`).
//!
//! Carried: scene and node hierarchy, TRS / matrix transforms, perspective and orthographic cameras,
//! triangle meshes (triangles, strips and fans; positions, normals, `TEXCOORD_0`; sparse accessors),
//! material factors and texture references, translation / rotation / scale animation (linear, step,
//! cubic spline). Everything else is reported (skins, morph targets, weights animation, other vertex
//! attributes, point and line primitives, extensions, extra scenes). A *required* extension the reader
//! cannot honour is an error: the file cannot be shown correctly without it.

use std::borrow::Cow;

use serde_json::Value;

use crate::util::{Budget, base64_decode, percent_decode, safe_relative_uri};
use crate::{
    Animation, Asset, Camera, Channel, Error, Format, Interpolation, Limits, Material, Mesh, Node, Primitive, Projection, Resolver, Result, Scene, TargetPath,
    TextureRef, Transform, Units, invalid,
};

const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

pub(crate) fn read(data: &[u8], resolver: &mut dyn Resolver, limits: &Limits) -> Result<Scene> {
    let (json, bin) = split_container(data)?;
    let root: Value = serde_json::from_slice(json).map_err(|e| Error::Invalid(format!("glTF JSON: {e}")))?;
    if !root.is_object() {
        return invalid("glTF root is not an object");
    }
    let mut r = Reader {
        root: &root,
        bin,
        resolver,
        limits,
        budget: Budget::new(limits.max_memory_bytes),
        buffers: Vec::new(),
        scene: Scene::empty(Format::Gltf, Units::Meters, limits.max_report_items),
    };
    r.run()?;
    Ok(r.scene)
}

fn u32_at(b: &[u8], off: usize) -> Result<u32> {
    let s = b.get(off..off.checked_add(4).ok_or_else(|| Error::Invalid("offset overflow".into()))?).ok_or_else(|| Error::Invalid("truncated GLB".into()))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Split a `.glb` into its JSON and BIN chunks, or pass a `.gltf` JSON text through.
fn split_container(data: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    if u32_at(data, 0).ok() != Some(GLB_MAGIC) {
        let text = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
        return Ok((text, None));
    }
    let version = u32_at(data, 4)?;
    if version != 2 {
        return Err(Error::Unsupported(format!("GLB version {version}")));
    }
    let total = u32_at(data, 8)? as usize;
    if total < 12 || total > data.len() {
        return invalid("GLB length field is inconsistent with the file size");
    }
    let body = data.get(..total).unwrap_or(data);
    let mut off = 12usize;
    let mut json: Option<&[u8]> = None;
    let mut bin: Option<&[u8]> = None;
    let mut first = true;
    while off < body.len() {
        let len = u32_at(body, off)? as usize;
        let kind = u32_at(body, off + 4)?;
        let start = off + 8;
        let end = start.checked_add(len).ok_or_else(|| Error::Invalid("GLB chunk length overflows".into()))?;
        let payload = body.get(start..end).ok_or_else(|| Error::Invalid("GLB chunk exceeds the file".into()))?;
        match kind {
            CHUNK_JSON if first => json = Some(payload),
            CHUNK_JSON => return invalid("second JSON chunk in GLB"),
            CHUNK_BIN if json.is_some() && bin.is_none() => bin = Some(payload),
            _ if first => return invalid("first GLB chunk is not JSON"),
            _ => {} // unknown or repeated chunks are skipped, as the spec says
        }
        first = false;
        off = end.checked_add(3).ok_or_else(|| Error::Invalid("GLB chunk length overflows".into()))? & !3;
    }
    let json = json.ok_or_else(|| Error::Invalid("GLB has no JSON chunk".into()))?;
    Ok((json, bin))
}

fn component_size(ct: u64) -> Option<usize> {
    match ct {
        5120 | 5121 => Some(1),
        5122 | 5123 => Some(2),
        5125 | 5126 => Some(4),
        _ => None,
    }
}

fn type_components(t: &str) -> Option<usize> {
    match t {
        "SCALAR" => Some(1),
        "VEC2" => Some(2),
        "VEC3" => Some(3),
        "VEC4" => Some(4),
        _ => None,
    }
}

struct Reader<'a> {
    root: &'a Value,
    bin: Option<&'a [u8]>,
    resolver: &'a mut dyn Resolver,
    limits: &'a Limits,
    budget: Budget,
    /// Lazily loaded buffers (`None` = not loaded yet).
    buffers: Vec<Option<Cow<'a, [u8]>>>,
    scene: Scene,
}

fn arr<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn opt_str<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

fn opt_usize(v: &Value, key: &str) -> Result<Option<usize>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_u64().and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| Error::Invalid(format!("{key} is not a non-negative integer"))),
    }
}

fn req_usize(v: &Value, key: &str) -> Result<usize> {
    opt_usize(v, key)?.ok_or_else(|| Error::Invalid(format!("missing {key}")))
}

fn num_f32(v: &Value, what: &str) -> Result<f32> {
    let f = v.as_f64().ok_or_else(|| Error::Invalid(format!("{what} is not a number")))? as f32;
    if f.is_finite() { Ok(f) } else { invalid(format!("{what} is out of range")) }
}

fn opt_f32(v: &Value, key: &str) -> Result<Option<f32>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => num_f32(x, key).map(Some),
    }
}

fn floats<const N: usize>(v: &Value, key: &str) -> Result<Option<[f32; N]>> {
    let Some(a) = v.get(key) else { return Ok(None) };
    let a = a.as_array().ok_or_else(|| Error::Invalid(format!("{key} is not an array")))?;
    if a.len() != N {
        return invalid(format!("{key} needs {N} numbers"));
    }
    let mut out = [0.0f32; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = num_f32(x, key)?;
    }
    Ok(Some(out))
}

fn idx_in(i: usize, len: usize, what: &str) -> Result<usize> {
    if i < len { Ok(i) } else { invalid(format!("{what} index {i} out of range ({len})")) }
}

struct Accessor {
    count: usize,
    comps: usize,
    ctype: u64,
    normalized: bool,
    /// Tightly packed little-endian elements.
    data: Vec<u8>,
}

impl<'a> Reader<'a> {
    fn name(&mut self, v: &Value) -> Result<Option<String>> {
        opt_str(v, "name").map(|s| self.budget.string(s)).transpose()
    }

    /// Report the `extensions` of one object (they are not interpreted).
    fn note_extensions(&mut self, v: &Value, path: &str) {
        if let Some(ext) = v.get("extensions").and_then(Value::as_object) {
            for k in ext.keys() {
                self.scene.report.unsupported(format!("{path}.extensions.{k}"), "extension is not interpreted");
            }
        }
    }

    fn run(&mut self) -> Result<()> {
        let root = self.root;
        let asset = root.get("asset").ok_or_else(|| Error::Invalid("missing asset".into()))?;
        let version = opt_str(asset, "version").ok_or_else(|| Error::Invalid("missing asset.version".into()))?;
        if !version.starts_with("2.") && version != "2" {
            return Err(Error::Unsupported(format!("glTF version {version}")));
        }
        self.scene.asset = Asset {
            generator: opt_str(asset, "generator").map(|s| self.budget.string(s)).transpose()?,
            copyright: opt_str(asset, "copyright").map(|s| self.budget.string(s)).transpose()?,
            version: Some(self.budget.string(version)?),
        };
        if let Some(e) = arr(root, "extensionsRequired").iter().next() {
            let name = e.as_str().unwrap_or("?");
            return Err(Error::Unsupported(format!("required extension {name}")));
        }
        for e in arr(root, "extensionsUsed") {
            self.scene.report.unsupported(format!("extensionsUsed.{}", e.as_str().unwrap_or("?")), "extension is not interpreted");
        }
        self.note_extensions(root, "root");
        if !arr(root, "skins").is_empty() {
            self.scene.report.unsupported("skins", "skins and joints are not carried");
        }
        self.materials()?;
        self.cameras()?;
        self.meshes()?;
        self.nodes()?;
        self.animations()?;
        Ok(())
    }

    fn materials(&mut self) -> Result<()> {
        let mats = arr(self.root, "materials");
        if mats.len() > self.limits.max_nodes {
            return Err(Error::Limit("too many materials".into()));
        }
        for (i, m) in mats.iter().enumerate() {
            self.budget.take(1, std::mem::size_of::<Material>())?;
            let mut mat = Material::named(self.name(m)?);
            let path = format!("materials[{i}]");
            self.note_extensions(m, &path);
            if let Some(pbr) = m.get("pbrMetallicRoughness") {
                if let Some(c) = floats::<4>(pbr, "baseColorFactor")? {
                    mat.base_color = c;
                }
                mat.metallic = opt_f32(pbr, "metallicFactor")?.unwrap_or(1.0);
                mat.roughness = opt_f32(pbr, "roughnessFactor")?.unwrap_or(1.0);
                self.texture(pbr.get("baseColorTexture"), "baseColor", &mut mat)?;
                self.texture(pbr.get("metallicRoughnessTexture"), "metallicRoughness", &mut mat)?;
            }
            self.texture(m.get("normalTexture"), "normal", &mut mat)?;
            self.texture(m.get("occlusionTexture"), "occlusion", &mut mat)?;
            self.texture(m.get("emissiveTexture"), "emissive", &mut mat)?;
            if let Some(e) = floats::<3>(m, "emissiveFactor")? {
                mat.emissive = e;
            }
            mat.double_sided = m.get("doubleSided").and_then(Value::as_bool).unwrap_or(false);
            match opt_str(m, "alphaMode") {
                None | Some("OPAQUE") => {}
                Some(mode) => self.scene.report.approximated(format!("{path}.alphaMode"), format!("alpha mode {mode} is not carried")),
            }
            self.scene.materials.push(mat);
        }
        Ok(())
    }

    fn texture(&mut self, info: Option<&Value>, slot: &str, mat: &mut Material) -> Result<()> {
        let Some(info) = info else { return Ok(()) };
        let index = req_usize(info, "index")?;
        let textures = arr(self.root, "textures");
        let tex = textures.get(index).ok_or_else(|| Error::Invalid(format!("texture index {index} out of range")))?;
        if tex.get("sampler").is_some() {
            self.scene.report.approximated("textures.sampler", "texture samplers (filtering, wrapping) are not carried");
        }
        let Some(source) = opt_usize(tex, "source")? else {
            self.scene.report.unsupported(format!("textures[{index}]"), "texture without a source image (extension) is not carried");
            return Ok(());
        };
        let img = arr(self.root, "images").get(source).ok_or_else(|| Error::Invalid(format!("image index {source} out of range")))?;
        let (uri, embedded) = match opt_str(img, "uri") {
            Some(u) if u.starts_with("data:") => (None, true),
            Some(u) => {
                let decoded = percent_decode(u)?;
                if safe_relative_uri(&decoded) {
                    (Some(self.budget.string(&decoded)?), false)
                } else {
                    self.scene.report.unsupported(format!("images[{source}].uri"), format!("unsafe image path {decoded:?} dropped"));
                    return Ok(());
                }
            }
            None => (None, true),
        };
        self.budget.take(1, std::mem::size_of::<TextureRef>())?;
        let slot = self.budget.string(slot)?;
        mat.textures.push(TextureRef { slot, uri, embedded, texcoord: opt_usize(info, "texCoord")?.unwrap_or(0) as u32 });
        Ok(())
    }

    fn cameras(&mut self) -> Result<()> {
        for (i, c) in arr(self.root, "cameras").iter().enumerate() {
            self.budget.take(1, std::mem::size_of::<Camera>())?;
            let name = self.name(c)?;
            self.note_extensions(c, &format!("cameras[{i}]"));
            let projection = match opt_str(c, "type") {
                Some("perspective") => {
                    let p = c.get("perspective").ok_or_else(|| Error::Invalid("perspective camera without parameters".into()))?;
                    Projection::Perspective {
                        yfov: opt_f32(p, "yfov")?.ok_or_else(|| Error::Invalid("missing yfov".into()))?,
                        aspect_ratio: opt_f32(p, "aspectRatio")?,
                        znear: opt_f32(p, "znear")?.ok_or_else(|| Error::Invalid("missing znear".into()))?,
                        zfar: opt_f32(p, "zfar")?,
                    }
                }
                Some("orthographic") => {
                    let p = c.get("orthographic").ok_or_else(|| Error::Invalid("orthographic camera without parameters".into()))?;
                    let g = |k: &str| opt_f32(p, k)?.ok_or_else(|| Error::Invalid(format!("missing {k}")));
                    Projection::Orthographic { xmag: g("xmag")?, ymag: g("ymag")?, znear: g("znear")?, zfar: g("zfar")? }
                }
                other => return invalid(format!("camera type {other:?}")),
            };
            self.scene.cameras.push(Camera { name, projection });
        }
        Ok(())
    }

    fn buffer(&mut self, index: usize) -> Result<usize> {
        let bufs = arr(self.root, "buffers");
        let b = bufs.get(index).ok_or_else(|| Error::Invalid(format!("buffer index {index} out of range")))?;
        if self.buffers.len() < bufs.len() {
            self.buffers.resize(bufs.len(), None);
        }
        if self.buffers.get(index).is_some_and(Option::is_some) {
            return Ok(index);
        }
        let byte_length = req_usize(b, "byteLength")?;
        let data: Cow<'a, [u8]> = match opt_str(b, "uri") {
            None => {
                if index != 0 {
                    return invalid(format!("buffer {index} has no uri and is not the GLB BIN chunk"));
                }
                Cow::Borrowed(self.bin.ok_or_else(|| Error::Missing("GLB BIN chunk".into()))?)
            }
            Some(u) => match u.strip_prefix("data:") {
                Some(rest) => {
                    let (_, payload) = rest.split_once(";base64,").ok_or_else(|| Error::Invalid("only base64 data URIs are supported".into()))?;
                    Cow::Owned(base64_decode(payload, &mut self.budget)?)
                }
                None => {
                    let decoded = percent_decode(u)?;
                    if !safe_relative_uri(&decoded) {
                        return invalid(format!("unsafe buffer path {decoded:?}"));
                    }
                    let bytes = self.resolver.resolve(&decoded).ok_or_else(|| Error::Missing(decoded.clone()))?;
                    if bytes.len() > self.limits.max_input_bytes {
                        return Err(Error::Limit(format!("buffer {decoded:?} is too large")));
                    }
                    self.budget.take(bytes.len(), 1)?;
                    Cow::Owned(bytes)
                }
            },
        };
        if data.len() < byte_length {
            return invalid(format!("buffer {index} is {} bytes, byteLength says {byte_length}", data.len()));
        }
        if let Some(slot) = self.buffers.get_mut(index) {
            *slot = Some(data);
        }
        Ok(index)
    }

    /// Resolve a buffer view to `(buffer index, absolute byte offset, view length, stride)`.
    fn view(&mut self, view_idx: usize) -> Result<(usize, usize, usize, Option<usize>)> {
        let v = arr(self.root, "bufferViews").get(view_idx).ok_or_else(|| Error::Invalid(format!("bufferView {view_idx} out of range")))?;
        let buf = self.buffer(req_usize(v, "buffer")?)?;
        let off = opt_usize(v, "byteOffset")?.unwrap_or(0);
        let len = req_usize(v, "byteLength")?;
        let stride = opt_usize(v, "byteStride")?;
        let blen = arr(self.root, "buffers").get(buf).map(|b| req_usize(b, "byteLength")).transpose()?.unwrap_or(0);
        let end = off.checked_add(len).ok_or_else(|| Error::Invalid("bufferView range overflows".into()))?;
        if end > blen {
            return invalid(format!("bufferView {view_idx} exceeds buffer {buf}"));
        }
        if let Some(s) = stride
            && !(4..=252).contains(&s)
        {
            return invalid(format!("bufferView {view_idx} byteStride {s} out of range"));
        }
        Ok((buf, off, len, stride))
    }

    fn buffer_slice(&self, buf: usize) -> &[u8] {
        self.buffers.get(buf).and_then(Option::as_ref).map_or(&[], |c| c.as_ref())
    }

    /// Read an accessor into tightly packed bytes (sparse substitution applied).
    fn accessor(&mut self, index: usize) -> Result<Accessor> {
        let a = arr(self.root, "accessors").get(index).ok_or_else(|| Error::Invalid(format!("accessor {index} out of range")))?;
        let count = req_usize(a, "count")?;
        let ctype = a.get("componentType").and_then(Value::as_u64).ok_or_else(|| Error::Invalid("missing componentType".into()))?;
        let csize = component_size(ctype).ok_or_else(|| Error::Invalid(format!("component type {ctype}")))?;
        let tname = opt_str(a, "type").ok_or_else(|| Error::Invalid("missing accessor type".into()))?;
        let comps = type_components(tname).ok_or_else(|| Error::Unsupported(format!("accessor type {tname}")))?;
        let normalized = a.get("normalized").and_then(Value::as_bool).unwrap_or(false);
        let elem = comps * csize;
        self.budget.take(count, elem)?;
        let total = count * elem;
        let mut data = vec![0u8; total];
        if let Some(vi) = opt_usize(a, "bufferView")? {
            let (buf, voff, vlen, stride) = self.view(vi)?;
            let aoff = opt_usize(a, "byteOffset")?.unwrap_or(0);
            let stride = stride.unwrap_or(elem);
            if stride < elem {
                return invalid("byteStride smaller than the element");
            }
            if count > 0 {
                let span = (count - 1).checked_mul(stride).and_then(|s| s.checked_add(elem)).and_then(|s| s.checked_add(aoff));
                if span.is_none_or(|s| s > vlen) {
                    return invalid(format!("accessor {index} exceeds its bufferView"));
                }
            }
            let bytes = self.buffer_slice(buf);
            for i in 0..count {
                let s = voff + aoff + i * stride;
                let src = bytes.get(s..s + elem).ok_or_else(|| Error::Invalid("accessor reads past the buffer".into()))?;
                if let Some(dst) = data.get_mut(i * elem..(i + 1) * elem) {
                    dst.copy_from_slice(src);
                }
            }
        }
        if let Some(sp) = a.get("sparse") {
            self.apply_sparse(sp, count, elem, &mut data)?;
        }
        Ok(Accessor { count, comps, ctype, normalized, data })
    }

    fn apply_sparse(&mut self, sp: &Value, count: usize, elem: usize, data: &mut [u8]) -> Result<()> {
        let n = req_usize(sp, "count")?;
        if n > count {
            return invalid("sparse count exceeds the accessor count");
        }
        let idx = sp.get("indices").ok_or_else(|| Error::Invalid("sparse without indices".into()))?;
        let val = sp.get("values").ok_or_else(|| Error::Invalid("sparse without values".into()))?;
        let ict = idx.get("componentType").and_then(Value::as_u64).ok_or_else(|| Error::Invalid("missing sparse componentType".into()))?;
        let isz = match ict {
            5121 | 5123 | 5125 => component_size(ict).unwrap_or(4),
            _ => return invalid("sparse indices must be unsigned"),
        };
        let (ib, ioff, ilen, _) = self.view(req_usize(idx, "bufferView")?)?;
        let ioff2 = ioff + opt_usize(idx, "byteOffset")?.unwrap_or(0);
        if opt_usize(idx, "byteOffset")?.unwrap_or(0).checked_add(n * isz).is_none_or(|e| e > ilen) {
            return invalid("sparse indices exceed their bufferView");
        }
        let (vb, voff, vlen, _) = self.view(req_usize(val, "bufferView")?)?;
        let voff2 = voff + opt_usize(val, "byteOffset")?.unwrap_or(0);
        if opt_usize(val, "byteOffset")?.unwrap_or(0).checked_add(n * elem).is_none_or(|e| e > vlen) {
            return invalid("sparse values exceed their bufferView");
        }
        let ibytes = self.buffer_slice(ib).get(ioff2..ioff2 + n * isz).ok_or_else(|| Error::Invalid("sparse indices past the buffer".into()))?.to_vec();
        let vbytes = self.buffer_slice(vb).get(voff2..voff2 + n * elem).ok_or_else(|| Error::Invalid("sparse values past the buffer".into()))?;
        for k in 0..n {
            let ibs = &ibytes[k * isz..(k + 1) * isz];
            let target = match isz {
                1 => ibs[0] as usize,
                2 => u16::from_le_bytes([ibs[0], ibs[1]]) as usize,
                _ => u32::from_le_bytes([ibs[0], ibs[1], ibs[2], ibs[3]]) as usize,
            };
            if target >= count {
                return invalid("sparse index out of range");
            }
            let dst = data.get_mut(target * elem..(target + 1) * elem).ok_or_else(|| Error::Invalid("sparse target out of range".into()))?;
            dst.copy_from_slice(&vbytes[k * elem..(k + 1) * elem]);
        }
        Ok(())
    }

    /// Accessor → flat `f32` (integers converted, normalised where flagged).
    fn accessor_f32(&mut self, index: usize, comps: usize) -> Result<Vec<f32>> {
        let a = self.accessor(index)?;
        if a.comps != comps {
            return invalid(format!("accessor {index} has {} components, {comps} expected", a.comps));
        }
        let n = a.count * a.comps;
        self.budget.take(n, 4)?;
        let mut out = Vec::with_capacity(n);
        let csize = component_size(a.ctype).unwrap_or(4);
        for c in a.data.chunks_exact(csize) {
            let v = match (a.ctype, a.normalized) {
                (5126, _) => f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                (5120, true) => (f32::from(c[0] as i8) / 127.0).max(-1.0),
                (5120, false) => f32::from(c[0] as i8),
                (5121, true) => f32::from(c[0]) / 255.0,
                (5121, false) => f32::from(c[0]),
                (5122, true) => (f32::from(i16::from_le_bytes([c[0], c[1]])) / 32767.0).max(-1.0),
                (5122, false) => f32::from(i16::from_le_bytes([c[0], c[1]])),
                (5123, true) => f32::from(u16::from_le_bytes([c[0], c[1]])) / 65535.0,
                (5123, false) => f32::from(u16::from_le_bytes([c[0], c[1]])),
                (_, true) => u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / u32::MAX as f32,
                (_, false) => u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32,
            };
            if !v.is_finite() {
                return invalid(format!("accessor {index} holds a non-finite value"));
            }
            out.push(v);
        }
        self.budget.give(a.data.len(), 1);
        Ok(out)
    }

    fn accessor_indices(&mut self, index: usize) -> Result<Vec<u32>> {
        let a = self.accessor(index)?;
        if a.comps != 1 || !matches!(a.ctype, 5121 | 5123 | 5125) {
            return invalid("indices must be a scalar unsigned accessor");
        }
        self.budget.take(a.count, 4)?;
        let csize = component_size(a.ctype).unwrap_or(4);
        let out: Vec<u32> = a
            .data
            .chunks_exact(csize)
            .map(|c| match csize {
                1 => u32::from(c[0]),
                2 => u32::from(u16::from_le_bytes([c[0], c[1]])),
                _ => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
            })
            .collect();
        self.budget.give(a.data.len(), 1);
        Ok(out)
    }

    fn meshes(&mut self) -> Result<()> {
        let meshes = arr(self.root, "meshes");
        if meshes.len() > self.limits.max_nodes {
            return Err(Error::Limit("too many meshes".into()));
        }
        for (mi, m) in meshes.iter().enumerate() {
            let name = self.name(m)?;
            let mut mesh = Mesh { name, primitives: Vec::new() };
            if m.get("weights").is_some() {
                self.scene.report.unsupported(format!("meshes[{mi}].weights"), "morph target weights are not carried");
            }
            for (pi, p) in arr(m, "primitives").iter().enumerate() {
                let path = format!("meshes[{mi}].primitives[{pi}]");
                if let Some(prim) = self.primitive(p, &path)? {
                    mesh.primitives.push(prim);
                }
            }
            self.scene.meshes.push(mesh);
        }
        Ok(())
    }

    fn primitive(&mut self, p: &Value, path: &str) -> Result<Option<Primitive>> {
        self.note_extensions(p, path);
        let mode = opt_usize(p, "mode")?.unwrap_or(4);
        if !(4..=6).contains(&mode) {
            let kind = ["points", "lines", "line loops", "line strips"].get(mode).copied().unwrap_or("unknown mode");
            self.scene.report.unsupported(format!("{path}.mode"), format!("{kind} primitive is not carried"));
            return Ok(None);
        }
        if !arr(p, "targets").is_empty() {
            self.scene.report.unsupported(format!("{path}.targets"), "morph targets are not carried");
        }
        let attrs = p.get("attributes").and_then(Value::as_object).ok_or_else(|| Error::Invalid("primitive without attributes".into()))?;
        let pos_idx = attrs
            .get("POSITION")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| Error::Invalid("primitive without POSITION".into()))?;
        let flat = self.accessor_f32(pos_idx, 3)?;
        let positions: Vec<[f32; 3]> = flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        drop(flat); // the retained copy stays charged (the transient peak is at most twice the charge)
        let vcount = positions.len();
        let mut prim = Primitive { positions, ..Primitive::default() };
        for (k, v) in attrs {
            let idx = v.as_u64().and_then(|n| usize::try_from(n).ok());
            match (k.as_str(), idx) {
                ("POSITION", _) => {}
                ("NORMAL", Some(i)) => {
                    let f = self.accessor_f32(i, 3)?;
                    if f.len() != vcount * 3 {
                        return invalid(format!("{path}: NORMAL count differs from POSITION"));
                    }
                    prim.normals = f.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                }
                ("TEXCOORD_0", Some(i)) => {
                    let f = self.accessor_f32(i, 2)?;
                    if f.len() != vcount * 2 {
                        return invalid(format!("{path}: TEXCOORD_0 count differs from POSITION"));
                    }
                    prim.texcoords = f.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
                }
                (other, _) => self.scene.report.unsupported(format!("{path}.attributes.{other}"), "vertex attribute is not carried"),
            }
        }
        let raw = match opt_usize(p, "indices")? {
            Some(i) => self.accessor_indices(i)?,
            None => {
                self.budget.take(vcount, 4)?;
                (0..u32::try_from(vcount).map_err(|_| Error::Limit("too many vertices".into()))?).collect()
            }
        };
        if let Some(bad) = raw.iter().find(|&&i| i as usize >= vcount) {
            return invalid(format!("{path}: index {bad} out of range ({vcount} vertices)"));
        }
        prim.indices = match mode {
            4 => {
                if raw.len() % 3 != 0 {
                    return invalid(format!("{path}: triangle index count {} is not a multiple of 3", raw.len()));
                }
                raw
            }
            5 | 6 => {
                if raw.len() < 3 {
                    Vec::new()
                } else {
                    let tris = raw.len() - 2;
                    self.budget.take(tris * 3, 4)?;
                    let mut out = Vec::with_capacity(tris * 3);
                    for i in 0..tris {
                        let tri = if mode == 6 {
                            [raw[0], raw[i + 1], raw[i + 2]]
                        } else if i % 2 == 0 {
                            [raw[i], raw[i + 1], raw[i + 2]]
                        } else {
                            [raw[i + 1], raw[i], raw[i + 2]]
                        };
                        out.extend_from_slice(&tri);
                    }
                    self.scene.report.approximated(
                        format!("{path}.mode"),
                        if mode == 6 { "triangle fan expanded to a triangle list" } else { "triangle strip expanded to a triangle list" },
                    );
                    out
                }
            }
            _ => Vec::new(),
        };
        if let Some(mat) = opt_usize(p, "material")? {
            prim.material = Some(idx_in(mat, self.scene.materials.len(), "material")?);
        }
        Ok(Some(prim))
    }

    fn nodes(&mut self) -> Result<()> {
        let nodes = arr(self.root, "nodes");
        let n = nodes.len();
        if n > self.limits.max_nodes {
            return Err(Error::Limit(format!("{n} nodes, the limit is {}", self.limits.max_nodes)));
        }
        for (i, nd) in nodes.iter().enumerate() {
            let path = format!("nodes[{i}]");
            self.budget.take(1, std::mem::size_of::<Node>())?;
            let name = self.name(nd)?;
            self.note_extensions(nd, &path);
            if nd.get("skin").is_some() {
                self.scene.report.unsupported(format!("{path}.skin"), "skin binding is not carried");
            }
            if nd.get("weights").is_some() {
                self.scene.report.unsupported(format!("{path}.weights"), "morph target weights are not carried");
            }
            let transform = if let Some(m) = floats::<16>(nd, "matrix")? {
                Transform::Matrix(m)
            } else {
                Transform::Trs {
                    translation: floats::<3>(nd, "translation")?.unwrap_or([0.0; 3]),
                    rotation: floats::<4>(nd, "rotation")?.unwrap_or([0.0, 0.0, 0.0, 1.0]),
                    scale: floats::<3>(nd, "scale")?.unwrap_or([1.0; 3]),
                }
            };
            let mesh = opt_usize(nd, "mesh")?.map(|m| idx_in(m, self.scene.meshes.len(), "mesh")).transpose()?;
            let camera = opt_usize(nd, "camera")?.map(|c| idx_in(c, self.scene.cameras.len(), "camera")).transpose()?;
            let mut children = Vec::new();
            for c in arr(nd, "children") {
                let ci = c.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| Error::Invalid("child index is not an integer".into()))?;
                self.budget.take(1, std::mem::size_of::<usize>())?;
                children.push(idx_in(ci, n, "child")?);
            }
            self.scene.nodes.push(Node { name, parent: None, children, transform, mesh, camera });
        }
        // parents (a node has at most one) — no cycles — bounded depth
        for i in 0..n {
            let kids = self.scene.nodes.get(i).map(|nd| nd.children.clone()).unwrap_or_default();
            for c in kids {
                if c == i {
                    return invalid(format!("node {i} is its own child"));
                }
                let Some(child) = self.scene.nodes.get_mut(c) else { return invalid("child out of range") };
                if child.parent.is_some() {
                    return invalid(format!("node {c} has more than one parent"));
                }
                child.parent = Some(i);
            }
        }
        let scene_roots = self.scene_roots(n)?;
        let mut visited = 0usize;
        let mut stack: Vec<(usize, usize)> = self.scene.nodes.iter().enumerate().filter(|(_, nd)| nd.parent.is_none()).map(|(i, _)| (i, 1)).collect();
        while let Some((i, depth)) = stack.pop() {
            visited += 1;
            if depth > self.limits.max_depth {
                return Err(Error::Limit(format!("node nesting deeper than {}", self.limits.max_depth)));
            }
            if let Some(nd) = self.scene.nodes.get(i) {
                stack.extend(nd.children.iter().map(|&c| (c, depth + 1)));
            }
        }
        if visited != n {
            return invalid("the node graph has a cycle");
        }
        self.scene.roots = scene_roots;
        Ok(())
    }

    fn scene_roots(&mut self, n: usize) -> Result<Vec<usize>> {
        let scenes = arr(self.root, "scenes");
        if scenes.is_empty() {
            return Ok(self.scene.nodes.iter().enumerate().filter(|(_, nd)| nd.parent.is_none()).map(|(i, _)| i).collect());
        }
        let chosen = opt_usize(self.root, "scene")?.unwrap_or(0);
        let sc = scenes.get(chosen).ok_or_else(|| Error::Invalid(format!("scene {chosen} out of range")))?;
        for (i, other) in scenes.iter().enumerate() {
            if i != chosen && !arr(other, "nodes").is_empty() {
                self.scene.report.unsupported(format!("scenes[{i}]"), "only the default scene is imported");
            }
        }
        let mut roots = Vec::new();
        for r in arr(sc, "nodes") {
            let ri = r.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| Error::Invalid("scene node is not an integer".into()))?;
            roots.push(idx_in(ri, n, "scene node")?);
            self.budget.take(1, std::mem::size_of::<usize>())?;
        }
        Ok(roots)
    }

    fn animations(&mut self) -> Result<()> {
        let anims = arr(self.root, "animations");
        if anims.len() > self.limits.max_nodes {
            return Err(Error::Limit("too many animations".into()));
        }
        for (ai, a) in anims.iter().enumerate() {
            let mut anim = Animation { name: self.name(a)?, channels: Vec::new() };
            let samplers = arr(a, "samplers");
            for (ci, ch) in arr(a, "channels").iter().enumerate() {
                let path = format!("animations[{ai}].channels[{ci}]");
                let target = ch.get("target").ok_or_else(|| Error::Invalid("channel without target".into()))?;
                let Some(node) = opt_usize(target, "node")? else {
                    self.scene.report.unsupported(format!("{path}.target"), "channel without a node (extension target) is not carried");
                    continue;
                };
                let node = idx_in(node, self.scene.nodes.len(), "animation node")?;
                let tp = match opt_str(target, "path") {
                    Some("translation") => TargetPath::Translation,
                    Some("rotation") => TargetPath::Rotation,
                    Some("scale") => TargetPath::Scale,
                    Some("weights") => {
                        self.scene.report.unsupported(format!("{path}.target.path"), "morph weight animation is not carried");
                        continue;
                    }
                    other => return invalid(format!("{path}: animation path {other:?}")),
                };
                let sampler = samplers.get(req_usize(ch, "sampler")?).ok_or_else(|| Error::Invalid("sampler out of range".into()))?;
                let interpolation = match opt_str(sampler, "interpolation") {
                    None | Some("LINEAR") => Interpolation::Linear,
                    Some("STEP") => Interpolation::Step,
                    Some("CUBICSPLINE") => Interpolation::CubicSpline,
                    Some(other) => return invalid(format!("{path}: interpolation {other}")),
                };
                let times = self.accessor_f32(req_usize(sampler, "input")?, 1)?;
                if times.windows(2).any(|w| w[1] < w[0]) {
                    return invalid(format!("{path}: keyframe times are not increasing"));
                }
                let comps = if tp == TargetPath::Rotation { 4 } else { 3 };
                let values = self.accessor_f32(req_usize(sampler, "output")?, comps)?;
                let per_key = if interpolation == Interpolation::CubicSpline { 3 * comps } else { comps };
                if values.len() != times.len().saturating_mul(per_key) {
                    return invalid(format!("{path}: {} keys but {} output values", times.len(), values.len()));
                }
                anim.channels.push(Channel { node, target: tp, interpolation, times, values });
            }
            self.scene.animations.push(anim);
        }
        Ok(())
    }
}
