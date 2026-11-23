use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;

pub(crate) fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

pub(crate) fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Buffer: positions (36) | indices u16 (6 + 2 pad) | times (8) | translations (24) = 76 bytes.
pub(crate) fn triangle_bin() -> Vec<u8> {
    let mut b = f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    b.extend_from_slice(&[0, 0, 1, 0, 2, 0, 0, 0]);
    b.extend(f32s(&[0.0, 1.0]));
    b.extend(f32s(&[0.0, 0.0, 0.0, 0.0, 2.0, 0.0]));
    b
}

pub(crate) fn triangle_json(buffer_uri: Option<&str>) -> String {
    let uri = buffer_uri.map(|u| format!(r#""uri":"{u}","#)).unwrap_or_default();
    format!(
        r#"{{
  "asset": {{"version":"2.0","generator":"test","copyright":"(c) test"}},
  "scene": 0,
  "scenes": [{{"nodes":[0]}}],
  "nodes": [
    {{"name":"root","children":[1,2],"translation":[1,2,3]}},
    {{"name":"tri","mesh":0,"rotation":[0,0,0,1],"scale":[2,2,2]}},
    {{"name":"cam","camera":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,5,0,0,1]}}
  ],
  "cameras": [{{"type":"perspective","perspective":{{"yfov":0.8,"znear":0.1,"zfar":100}}}}],
  "meshes": [{{"name":"m","primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
  "materials": [{{"name":"red","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1],"metallicFactor":0.0}}}}],
  "animations": [{{"name":"move","channels":[{{"sampler":0,"target":{{"node":1,"path":"translation"}}}}],
                  "samplers":[{{"input":2,"output":3}}]}}],
  "accessors": [
    {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},
    {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},
    {{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR"}},
    {{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}}
  ],
  "bufferViews": [
    {{"buffer":0,"byteOffset":0,"byteLength":36}},
    {{"buffer":0,"byteOffset":36,"byteLength":6}},
    {{"buffer":0,"byteOffset":44,"byteLength":8}},
    {{"buffer":0,"byteOffset":52,"byteLength":24}}
  ],
  "buffers": [{{{uri}"byteLength":76}}]
}}"#
    )
}

pub(crate) fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut j = json.as_bytes().to_vec();
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    let mut b = bin.to_vec();
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + b.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON_TEST.to_le_bytes());
    out.extend_from_slice(&j);
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN_TEST.to_le_bytes());
    out.extend_from_slice(&b);
    out
}
const CHUNK_JSON_TEST: u32 = 0x4E4F_534A;
const CHUNK_BIN_TEST: u32 = 0x004E_4942;

pub(crate) fn limits() -> Limits {
    Limits::default()
}

fn check_triangle_scene(s: &Scene) {
    assert_eq!(s.format, Format::Gltf);
    assert_eq!(s.units, Units::Meters);
    assert_eq!(s.asset.generator.as_deref(), Some("test"));
    assert_eq!(s.asset.copyright.as_deref(), Some("(c) test"));
    assert_eq!(s.nodes.len(), 3);
    assert_eq!(s.roots, vec![0]);
    assert_eq!(s.nodes[0].children, vec![1, 2]);
    assert_eq!(s.nodes[1].parent, Some(0));
    assert_eq!(s.nodes[0].transform, Transform::Trs { translation: [1.0, 2.0, 3.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] });
    assert_eq!(s.nodes[1].transform, Transform::Trs { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [2.0; 3] });
    assert!(matches!(s.nodes[2].transform, Transform::Matrix(m) if m[12] == 5.0));
    assert_eq!(s.nodes[2].camera, Some(0));
    assert_eq!(s.cameras[0].projection, Projection::Perspective { yfov: 0.8, aspect_ratio: None, znear: 0.1, zfar: Some(100.0) });
    let p = &s.meshes[0].primitives[0];
    assert_eq!(p.positions, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    assert_eq!(p.indices, vec![0, 1, 2]);
    assert_eq!(p.material, Some(0));
    assert_eq!(s.materials[0].base_color, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.materials[0].metallic, 0.0);
    let ch = &s.animations[0].channels[0];
    assert_eq!((ch.node, ch.target, ch.interpolation), (1, TargetPath::Translation, Interpolation::Linear));
    assert_eq!(ch.times, vec![0.0, 1.0]);
    assert_eq!(ch.values, vec![0.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
    assert!(s.report.is_clean(), "{:?}", s.report);
}

#[test]
fn gltf_data_uri_hierarchy_camera_mesh_animation_material() {
    let uri = format!("data:application/octet-stream;base64,{}", b64(&triangle_bin()));
    let s = read(triangle_json(Some(&uri)).as_bytes(), Some("a.gltf"), &mut NoResources, &limits()).unwrap();
    check_triangle_scene(&s);
}

#[test]
fn glb_and_external_bin_read_the_same() {
    let from_glb = read(&glb(&triangle_json(None), &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    check_triangle_scene(&from_glb);
    let bin = triangle_bin();
    let mut asked = Vec::new();
    let mut res = |u: &str| {
        asked.push(u.to_string());
        Some(bin.clone())
    };
    let ext = read(triangle_json(Some("buf%20a.bin")).as_bytes(), Some("a.gltf"), &mut res, &limits()).unwrap();
    assert_eq!(asked, vec!["buf a.bin".to_string()]);
    assert_eq!(ext, {
        let mut e = from_glb.clone();
        e.report = ext.report.clone();
        e
    });
}

#[test]
fn missing_external_buffer_is_an_error_and_unsafe_paths_are_not_asked() {
    let e = read(triangle_json(Some("a.bin")).as_bytes(), None, &mut NoResources, &limits()).unwrap_err();
    assert!(matches!(e, Error::Missing(_)), "{e}");
    for bad in ["../a.bin", "/etc/passwd", "file:///x", "a/../../b", "..\\x"] {
        let mut called = false;
        let mut res = |_: &str| {
            called = true;
            Some(Vec::new())
        };
        let e = read(triangle_json(Some(bad)).as_bytes(), None, &mut res, &limits()).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{bad}: {e}");
        assert!(!called, "{bad} reached the resolver");
    }
}

#[test]
fn unsupported_elements_are_reported_not_dropped() {
    let json = triangle_json(None)
        .replace(r#""nodes": ["#, r#""skins":[{"joints":[0]}], "extensionsUsed":["KHR_materials_unlit"], "nodes": ["#)
        .replace(r#""name":"tri","mesh":0"#, r#""name":"tri","mesh":0,"skin":0,"weights":[0.5],"extensions":{"EXT_x":{}}"#)
        .replace(r#""attributes":{"POSITION":0}"#, r#""attributes":{"POSITION":0,"COLOR_0":0},"targets":[{"POSITION":0}]"#);
    let s = read(&glb(&json, &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    let paths: Vec<&str> = s.report.items.iter().map(|i| i.path.as_str()).collect();
    for want in [
        "skins",
        "extensionsUsed.KHR_materials_unlit",
        "nodes[1].skin",
        "nodes[1].weights",
        "nodes[1].extensions.EXT_x",
        "meshes[0].primitives[0].attributes.COLOR_0",
        "meshes[0].primitives[0].targets",
    ] {
        assert!(paths.contains(&want), "missing {want} in {paths:?}");
    }
    assert!(!s.report.is_clean());
}

#[test]
fn required_extension_is_an_error() {
    let json = triangle_json(None).replace(r#""scene": 0,"#, r#""scene":0,"extensionsRequired":["KHR_draco_mesh_compression"],"#);
    let e = read(&glb(&json, &triangle_bin()), None, &mut NoResources, &limits()).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "{e}");
}

#[test]
fn strips_fans_and_non_triangle_modes() {
    let strip = triangle_json(None).replace(r#""indices":1,"#, r#""indices":1,"mode":5,"#);
    let s = read(&glb(&strip, &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    assert_eq!(s.meshes[0].primitives[0].indices, vec![0, 1, 2]);
    assert!(s.report.items.iter().any(|i| i.severity == Severity::Approximated));
    let lines = triangle_json(None).replace(r#""indices":1,"#, r#""indices":1,"mode":1,"#);
    let s = read(&glb(&lines, &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    assert!(s.meshes[0].primitives.is_empty());
    assert!(s.report.items.iter().any(|i| i.path.ends_with(".mode") && i.severity == Severity::Unsupported));
}

#[test]
fn sparse_accessor_substitutes_values() {
    // accessor 0 becomes sparse on top of the same view: element 1 replaced by the values at bytes 52..64
    let json = triangle_json(None).replace(
        r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}"#,
        r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","sparse":{"count":1,"indices":{"bufferView":1,"componentType":5123},"values":{"bufferView":3}}}"#,
    );
    // view 1 holds u16 [0,1,2]: sparse index 0 -> replaced by the first translation (0,0,0), harmless; use index offset
    let s = read(&glb(&json, &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    assert_eq!(s.meshes[0].primitives[0].positions[0], [0.0, 0.0, 0.0]);
    let bad = json.replace(r#""bufferView":1,"componentType":5123}"#, r#""bufferView":1,"byteOffset":2,"componentType":5123}"#);
    // index 1 replaced by translation[0] = (0,0,0)
    let s = read(&glb(&bad, &triangle_bin()), None, &mut NoResources, &limits()).unwrap();
    assert_eq!(s.meshes[0].primitives[0].positions[1], [0.0, 0.0, 0.0]);
}

#[test]
fn hostile_gltf_is_an_error() {
    let ok = triangle_json(None);
    let cases: Vec<(&str, String)> = vec![
        ("cycle", ok.replace(r#""name":"tri","mesh":0"#, r#""name":"tri","children":[0],"mesh":0"#)),
        ("self-child", ok.replace(r#""name":"cam","camera":0"#, r#""name":"cam","children":[2],"camera":0"#)),
        ("two parents", ok.replace(r#""name":"cam","camera":0"#, r#""name":"cam","children":[1],"camera":0"#)),
        ("child range", ok.replace(r#""children":[1,2]"#, r#""children":[1,99]"#)),
        ("accessor range", ok.replace(r#""indices":1"#, r#""indices":42"#)),
        ("huge count", ok.replace(r#""count":3,"type":"VEC3""#, r#""count":4294967295,"type":"VEC3""#)),
        ("view past buffer", ok.replace(r#""byteLength":36"#, r#""byteLength":3600"#)),
        ("bad version", ok.replace(r#""version":"2.0""#, r#""version":"1.0""#)),
        ("bad type", ok.replace(r#""type":"SCALAR""#, r#""type":"MAT4""#)),
        ("non-finite", ok.replace(r#""scale":[2,2,2]"#, r#""scale":[1e999,2,2]"#)),
        ("indices range", ok.replace(r#""componentType":5123,"count":3"#, r#""componentType":5123,"count":2"#)),
        ("stride", ok.replace(r#""buffer":0,"byteOffset":0,"byteLength":36"#, r#""buffer":0,"byteOffset":0,"byteLength":36,"byteStride":1000"#)),
    ];
    for (what, json) in cases {
        let r = catch_unwind(AssertUnwindSafe(|| read(&glb(&json, &triangle_bin()), None, &mut NoResources, &limits())));
        assert!(matches!(r, Ok(Err(_))), "{what}: {r:?}");
    }
}

#[test]
fn memory_budget_stops_json_amplification() {
    // many meshes that each re-read the same accessor: the shared budget must stop it
    let prim = r#"{"attributes":{"POSITION":0}}"#;
    let meshes: Vec<String> = (0..200).map(|_| format!(r#"{{"primitives":[{prim}]}}"#)).collect();
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"meshes":[{}],"accessors":[{{"bufferView":0,"componentType":5126,"count":99999,"type":"VEC3"}}],
            "bufferViews":[{{"buffer":0,"byteLength":1199988}}],"buffers":[{{"byteLength":1199988}}]}}"#,
        meshes.join(",")
    );
    let bin = vec![0u8; 1_199_988];
    let tight = Limits { max_memory_bytes: 20 << 20, ..Limits::default() };
    let e = read(&glb(&json, &bin), None, &mut NoResources, &tight).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
}

#[test]
fn deep_nesting_and_node_count_are_limited() {
    let n = 300;
    let nodes: Vec<String> = (0..n).map(|i| if i + 1 < n { format!(r#"{{"children":[{}]}}"#, i + 1) } else { "{}".into() }).collect();
    let json = format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{}]}}"#, nodes.join(","));
    let e = read(json.as_bytes(), Some("x.gltf"), &mut NoResources, &limits()).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
    let few = Limits { max_nodes: 10, ..Limits::default() };
    let e = read(json.as_bytes(), Some("x.gltf"), &mut NoResources, &few).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
}

#[test]
fn glb_header_hostility() {
    let good = glb(&triangle_json(None), &triangle_bin());
    for cut in [0, 3, 11, 12, 19, 20, 40, good.len() - 1] {
        let r = catch_unwind(AssertUnwindSafe(|| read(&good[..cut], Some("a.glb"), &mut NoResources, &limits())));
        assert!(r.is_ok(), "panic at cut {cut}");
    }
    let mut bad = good.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read(&bad, None, &mut NoResources, &limits()).is_err());
    let mut bad = good.clone();
    bad[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read(&bad, None, &mut NoResources, &limits()).is_err());
    let mut bad = good;
    bad[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert!(matches!(read(&bad, None, &mut NoResources, &limits()), Err(Error::Unsupported(_))));
}

// ---- OBJ -------------------------------------------------------------------------------------------

pub(crate) const OBJ: &str = "# a quad and a triangle\nmtllib m.mtl\no quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nusemtl red\nf 1/1/1 2/2/1 3/3/1 4/4/1\no tri\nusemtl blue\nf -4 -3 -2\nl 1 2\ns 1\n";
pub(crate) const MTL: &str = "newmtl red\nKd 1 0 0\nd 0.5\nmap_Kd tex/red.png\nNs 10\nnewmtl blue\nKd 0 0 1\nKe 0.1 0.1 0.1\n";

#[test]
fn obj_reads_polygons_materials_and_reports_the_rest() {
    let mut res = |u: &str| (u == "m.mtl").then(|| MTL.as_bytes().to_vec());
    let s = read(OBJ.as_bytes(), Some("a.obj"), &mut res, &limits()).unwrap();
    assert_eq!(s.format, Format::Obj);
    assert_eq!(s.nodes.len(), 2);
    assert_eq!(s.nodes[0].name.as_deref(), Some("quad"));
    let quad = &s.meshes[0].primitives[0];
    assert_eq!(quad.positions.len(), 4);
    assert_eq!(quad.indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(quad.normals.len(), 4);
    assert_eq!(quad.texcoords.len(), 4);
    let tri = &s.meshes[1].primitives[0];
    assert_eq!(tri.positions, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
    assert!(tri.normals.is_empty() && tri.texcoords.is_empty());
    let red = &s.materials[quad.material.unwrap()];
    assert_eq!(red.name.as_deref(), Some("red"));
    assert_eq!(red.base_color, [1.0, 0.0, 0.0, 0.5]);
    assert_eq!(red.textures[0].uri.as_deref(), Some("tex/red.png"));
    assert_eq!(s.materials[tri.material.unwrap()].emissive, [0.1; 3]);
    let reported: Vec<&str> = s.report.items.iter().map(|i| i.path.as_str()).collect();
    assert!(reported.iter().any(|p| p.ends_with(": l")), "{reported:?}");
    assert!(reported.iter().any(|p| p.ends_with(": s")), "{reported:?}");
    assert!(reported.iter().any(|p| p.contains("Ns")), "{reported:?}");
    assert!(reported.iter().any(|p| p.ends_with(": f")), "{reported:?}");
}

#[test]
fn obj_without_mtl_keeps_the_material_reference() {
    let s = read(OBJ.as_bytes(), Some("a.obj"), &mut NoResources, &limits()).unwrap();
    assert_eq!(s.materials.len(), 2);
    assert_eq!(s.materials[0].name.as_deref(), Some("red"));
    assert!(s.report.items.iter().any(|i| i.path.contains("mtllib")));
}

#[test]
fn hostile_obj_is_an_error() {
    for (what, text) in [
        ("zero index", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n"),
        ("range", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n"),
        ("negative range", "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -9 1 2\n"),
        ("two verts", "v 0 0 0\nv 1 0 0\nf 1 2\n"),
        ("nan", "v nan 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"),
        ("inf", "v 1e999 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"),
        ("short v", "v 1 2\n"),
        ("no faces", "v 0 0 0\n"),
        ("huge index", "v 0 0 0\nv 0 0 0\nv 0 0 0\nf 99999999999999999999 1 2\n"),
        ("parts", "v 0 0 0\nv 0 0 0\nv 0 0 0\nf 1/1/1/1 2 3\n"),
    ] {
        let r = catch_unwind(AssertUnwindSafe(|| read(text.as_bytes(), Some("x.obj"), &mut NoResources, &limits())));
        assert!(matches!(r, Ok(Err(_))), "{what}: {r:?}");
    }
}

#[test]
fn obj_unsafe_mtllib_is_reported_and_not_resolved() {
    let mut called = false;
    let mut res = |_: &str| {
        called = true;
        None
    };
    let text = "mtllib ../../etc/passwd\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let s = read(text.as_bytes(), Some("a.obj"), &mut res, &limits()).unwrap();
    assert!(!called);
    assert!(s.report.items.iter().any(|i| i.path.contains("mtllib")));
}

#[test]
fn obj_memory_budget() {
    let mut text = String::from("v 0 0 0\nv 1 0 0\nv 0 1 0\n");
    for _ in 0..5000 {
        text.push_str("f 1 2 3\n");
    }
    let tight = Limits { max_memory_bytes: 1000, ..Limits::default() };
    let e = read(text.as_bytes(), Some("a.obj"), &mut NoResources, &tight).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
}

// ---- STL -------------------------------------------------------------------------------------------

pub(crate) fn binary_stl(tris: &[[[f32; 3]; 3]], attr: u16) -> Vec<u8> {
    let mut b = vec![0u8; 80];
    b[..5].copy_from_slice(b"solid");
    b.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for t in tris {
        b.extend(f32s(&[0.0, 0.0, 1.0]));
        for v in t {
            b.extend(f32s(v));
        }
        b.extend_from_slice(&attr.to_le_bytes());
    }
    b
}

pub(crate) const TRI: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

#[test]
fn binary_stl_even_when_the_header_says_solid() {
    let data = binary_stl(&[TRI, TRI], 0);
    assert_eq!(detect(&data, None), Some(Format::Stl));
    let s = read(&data, None, &mut NoResources, &limits()).unwrap();
    assert_eq!(s.format, Format::Stl);
    assert_eq!(s.triangle_count(), 2);
    assert_eq!(s.vertex_count(), 6);
    let p = &s.meshes[0].primitives[0];
    assert_eq!(p.positions[1], [1.0, 0.0, 0.0]);
    assert_eq!(p.normals[0], [0.0, 0.0, 1.0]);
    assert_eq!(p.indices, vec![0, 1, 2, 3, 4, 5]);
    assert!(s.report.is_clean(), "{:?}", s.report);
}

#[test]
fn binary_stl_attribute_bytes_and_truncation() {
    let s = read(&binary_stl(&[TRI], 0x8000), None, &mut NoResources, &limits()).unwrap();
    assert!(s.report.items.iter().any(|i| i.severity == Severity::Unsupported));
    let data = binary_stl(&[TRI, TRI], 0);
    assert!(read(&data[..data.len() - 1], Some("a.stl"), &mut NoResources, &limits()).is_err());
    let mut huge = data.clone();
    huge[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read(&huge, Some("a.stl"), &mut NoResources, &limits()).is_err());
    let mut nan = data;
    nan[84 + 12..84 + 16].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(read(&nan, Some("a.stl"), &mut NoResources, &limits()).is_err());
}

#[test]
fn ascii_stl_with_two_solids() {
    let text = "solid one\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid one\nsolid two\n facet normal 0 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\nendsolid two\n";
    let s = read(text.as_bytes(), Some("a.stl"), &mut NoResources, &limits()).unwrap();
    assert_eq!(s.nodes.len(), 2);
    assert_eq!(s.nodes[1].name.as_deref(), Some("two"));
    assert_eq!(s.meshes[1].primitives[0].normals[0], [0.0, 0.0, -1.0]);
    assert!(s.report.items.iter().any(|i| i.severity == Severity::Approximated));
    for bad in [
        "solid x\nfacet normal 0 0 1\nvertex 0 0 0\nendsolid x\n",
        "solid x\n",
        "endsolid\n",
        "solid x\nvertex a b c\nendsolid x\n",
        "solid\nbogus\nendsolid\n",
    ] {
        assert!(read(bad.as_bytes(), Some("a.stl"), &mut NoResources, &limits()).is_err(), "{bad:?}");
    }
}

// ---- detection and the in-process mutation fuzz gate (U1-A2) ----------------------------------------

#[test]
fn detection() {
    assert_eq!(detect(b"glTF\x02\0\0\0", None), Some(Format::Gltf));
    assert_eq!(detect(b"  {\"asset\":{}}", None), Some(Format::Gltf));
    assert_eq!(detect(OBJ.as_bytes(), None), Some(Format::Obj));
    assert_eq!(detect(b"solid x\nendsolid x\n", None), Some(Format::Stl));
    assert_eq!(detect(b"\0\0\0", Some("a.OBJ")), Some(Format::Obj));
    assert_eq!(detect(b"\x01\x02\x03", None), None);
    assert!(read(b"\x01\x02\x03", None, &mut NoResources, &limits()).is_err());
}

pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub(crate) fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

pub(crate) fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = seed.to_vec();
    for _ in 0..1 + rng.below(4) {
        match rng.below(5) {
            0 if !v.is_empty() => v.truncate(rng.below(v.len())),
            1 if !v.is_empty() => {
                let i = rng.below(v.len());
                v[i] ^= 1 << rng.below(8);
            }
            2 if v.len() >= 4 => {
                let i = rng.below(v.len() - 3);
                let big = [0u32, 1, 0x7fff_ffff, 0xffff_ffff, 0x00ff_ffff][rng.below(5)];
                v[i..i + 4].copy_from_slice(&big.to_le_bytes());
            }
            3 if !v.is_empty() => {
                let i = rng.below(v.len());
                v[i] = [0, 0xff, b'-', b'9', b'"', b'/'][rng.below(6)];
            }
            _ => v.push(rng.next() as u8),
        }
    }
    v
}

#[test]
fn mutation_fuzz_never_panics_and_stays_within_the_budget() {
    let bin = triangle_bin();
    let seeds: Vec<Vec<u8>> = vec![
        glb(&triangle_json(None), &bin),
        triangle_json(Some(&format!("data:application/octet-stream;base64,{}", b64(&bin)))).into_bytes(),
        OBJ.as_bytes().to_vec(),
        binary_stl(&[TRI, TRI], 0),
        b"solid s\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid s\n".to_vec(),
    ];
    let lim = Limits { max_memory_bytes: 8 << 20, max_input_bytes: 1 << 20, ..Limits::default() };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for (si, seed) in seeds.iter().enumerate() {
        for it in 0..3000 {
            let data = mutate(seed, &mut rng);
            let r = catch_unwind(AssertUnwindSafe(|| {
                let mut res = |_: &str| Some(bin.clone());
                let _ = read(&data, None, &mut res, &lim);
                for fmt in [Format::Gltf, Format::Obj, Format::Stl] {
                    let _ = read_as(fmt, &data, &mut res, &lim);
                }
            }));
            assert!(r.is_ok(), "panic on seed {si} iteration {it}: {data:?}");
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(42);
    let lim = limits();
    for _ in 0..2000 {
        let n = rng.below(300);
        let data: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        let r = catch_unwind(AssertUnwindSafe(|| {
            for fmt in [Format::Gltf, Format::Obj, Format::Stl] {
                let _ = read_as(fmt, &data, &mut NoResources, &lim);
            }
        }));
        assert!(r.is_ok());
    }
}
