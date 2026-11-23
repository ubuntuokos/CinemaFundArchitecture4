use std::panic::{AssertUnwindSafe, catch_unwind};

use super::tests::{MTL, OBJ, Rng, TRI, binary_stl, f32s, glb, limits, triangle_bin, triangle_json};
use super::*;

fn bytes_of(e: &Export, name: &str) -> Vec<u8> {
    e.files
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.data.clone())
        .unwrap_or_else(|| panic!("no file {name}: {:?}", e.files.iter().map(|f| &f.name).collect::<Vec<_>>()))
}

fn triangle_scene() -> Scene {
    read(&glb(&triangle_json(None), &triangle_bin()), None, &mut NoResources, &limits()).unwrap()
}

fn obj_scene() -> Scene {
    let mut res = |u: &str| (u == "m.mtl").then(|| MTL.as_bytes().to_vec());
    read(OBJ.as_bytes(), Some("a.obj"), &mut res, &limits()).unwrap()
}

fn opts(target: Target) -> ExportOptions {
    ExportOptions::new(target, "out")
}

fn run(scene: &Scene, target: Target) -> Export {
    export(scene, &opts(target), &mut NoResources).unwrap()
}

/// What U3a-A1 compares: hierarchy, transforms, cameras, geometry, materials, animation.
fn assert_same_model(a: &Scene, b: &Scene) {
    assert_eq!(a.nodes, b.nodes, "nodes");
    assert_eq!(a.roots, b.roots, "roots");
    assert_eq!(a.meshes, b.meshes, "meshes");
    assert_eq!(a.cameras, b.cameras, "cameras");
    assert_eq!(a.materials, b.materials, "materials");
    assert_eq!(a.animations, b.animations, "animations");
}

fn paths(e: &Export) -> Vec<String> {
    e.report.items.iter().map(|i| format!("{}: {}", i.path, i.message)).collect()
}

fn mentions(e: &Export, needle: &str) -> bool {
    paths(e).iter().any(|p| p.contains(needle))
}

// ------------------------------------------------------------------------------------------ glTF

#[test]
fn glb_round_trip_keeps_hierarchy_transforms_camera_mesh_material_animation() {
    let src = triangle_scene();
    let e = run(&src, Target::Glb);
    assert_eq!(e.files.len(), 1);
    assert_eq!(e.files[0].name, "out.glb");
    assert!(e.report.is_clean(), "glTF to glTF loses nothing: {:?}", paths(&e));
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_same_model(&src, &back);
    assert!(back.report.is_clean(), "{:?}", back.report);
    // the copyright survives, the generator is ours
    assert_eq!(back.asset.copyright.as_deref(), Some("(c) test"));
    assert!(back.asset.generator.as_deref().is_some_and(|g| g.starts_with("CFA4 ")));
    // and a second round is byte-identical (rule 4)
    let again = run(&back, Target::Glb);
    assert_eq!(again.files, e.files);
}

#[test]
fn gltf_with_external_bin_round_trips_and_the_files_are_named_after_the_stem() {
    let src = triangle_scene();
    let e = run(&src, Target::Gltf);
    assert_eq!(e.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["out.gltf", "out.bin"]);
    let bin = bytes_of(&e, "out.bin");
    let mut res = |u: &str| (u == "out.bin").then(|| bin.clone());
    let back = read(&e.files[0].data, Some("out.gltf"), &mut res, &limits()).unwrap();
    assert_same_model(&src, &back);
    assert_eq!(run(&back, Target::Gltf).files, e.files, "deterministic");
}

#[test]
fn gltf_keeps_cubic_spline_step_and_matrix_exactly() {
    let mut s = triangle_scene();
    s.nodes[1].transform = Transform::Matrix([0.1, 0.0, 0.0, 0.0, 0.0, 0.3, 0.0, 0.0, 0.0, 0.0, 0.7, 0.0, 1.5, -2.25, 1e-7, 1.0]);
    s.animations[0].channels.push(Channel {
        node: 1,
        target: TargetPath::Rotation,
        interpolation: Interpolation::CubicSpline,
        times: vec![0.0, 0.1],
        values: (0..24).map(|i| i as f32 * 0.1).collect(),
    });
    s.animations[0].channels.push(Channel {
        node: 0,
        target: TargetPath::Scale,
        interpolation: Interpolation::Step,
        times: vec![0.5],
        values: vec![1.0, 2.0, 3.0],
    });
    let e = run(&s, Target::Glb);
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_same_model(&s, &back);
}

#[test]
fn gltf_embeds_textures_only_when_asked_and_lists_the_references_otherwise() {
    let mut s = triangle_scene();
    s.materials[0].textures.push(TextureRef { slot: "baseColor".into(), uri: Some("tex/red.png".into()), embedded: false, texcoord: 1 });
    let e = run(&s, Target::Glb);
    assert_eq!(e.textures, vec!["tex/red.png".to_string()]);
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_eq!(back.materials[0].textures, s.materials[0].textures, "reference and texCoord survive");

    let mut o = opts(Target::Glb);
    o.embed_textures = true;
    let png = b"\x89PNG not really".to_vec();
    let mut res = |u: &str| (u == "tex/red.png").then(|| png.clone());
    let e = export(&s, &o, &mut res).unwrap();
    assert!(e.textures.is_empty(), "embedded: nothing is left to copy");
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_eq!(back.materials[0].textures.len(), 1);
    assert!(back.materials[0].textures[0].embedded && back.materials[0].textures[0].uri.is_none());
    assert!(e.files[0].data.windows(png.len()).any(|w| w == png.as_slice()), "the image bytes are in the file");

    // an image that is not there stays a reference, and the report says so
    let e = export(&s, &o, &mut NoResources).unwrap();
    assert_eq!(e.textures, vec!["tex/red.png".to_string()]);
    assert!(mentions(&e, "not available to embed"));
    // embedding on .gltf is declined, with a note
    let mut o = opts(Target::Gltf);
    o.embed_textures = true;
    assert!(mentions(&export(&s, &o, &mut res).unwrap(), "only in .glb"));
}

#[test]
fn gltf_reports_what_the_model_cannot_write() {
    let mut s = triangle_scene();
    s.materials[0].textures.push(TextureRef { slot: "baseColor".into(), uri: None, embedded: true, texcoord: 0 });
    s.materials[0].textures.push(TextureRef { slot: "weird".into(), uri: Some("x.png".into()), embedded: false, texcoord: 0 });
    s.materials[0].base_color[3] = 0.5;
    s.meshes.push(Mesh { name: Some("empty".into()), primitives: vec![] });
    s.report.unsupported("skins", "skins and joints are not carried");
    let e = run(&s, Target::Glb);
    assert!(mentions(&e, "embedded texture image"));
    assert!(mentions(&e, "\"weird\""));
    assert!(mentions(&e, "alpha"));
    assert!(mentions(&e, "meshes[1]"));
    assert!(mentions(&e, "the import reported 1 element"));
    // a source without units says how the numbers are read
    let obj = run(&obj_scene(), Target::Glb);
    assert!(mentions(&obj, "no unit"));
}

// ------------------------------------------------------------------------------------------- OBJ

#[test]
fn obj_round_trips_and_a_second_round_is_byte_identical() {
    let src = obj_scene();
    let e = run(&src, Target::Obj);
    assert_eq!(e.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["out.obj", "out.mtl"]);
    let mtl = bytes_of(&e, "out.mtl");
    let mut res = |u: &str| (u == "out.mtl").then(|| mtl.clone());
    let back = read(&e.files[0].data, Some("out.obj"), &mut res, &limits()).unwrap();
    assert_same_model(&src, &back);
    assert_eq!(e.textures, vec!["tex/red.png".to_string()]);
    let again = run(&back, Target::Obj);
    assert_eq!(again.files, e.files, "oda-vissza egyező");
    // OBJ to OBJ states no units, so nothing is lost beyond what the first read dropped
    assert!(!mentions(&e, "units"));
}

#[test]
fn gltf_to_obj_reports_hierarchy_camera_animation_and_bakes_transforms() {
    let src = triangle_scene();
    let e = run(&src, Target::Obj);
    assert!(mentions(&e, "no node hierarchy"));
    assert!(mentions(&e, "camera"));
    assert!(mentions(&e, "no animation"));
    assert!(mentions(&e, "metres"));
    assert!(mentions(&e, "baked"));
    assert!(!mentions(&e, "metallic"), "a non-metal with default roughness loses nothing here");
    let mut metal = src.clone();
    metal.materials[0].metallic = 0.5;
    assert!(mentions(&run(&metal, Target::Obj), "metallic"));
    let mtl = bytes_of(&e, "out.mtl");
    let mut res = |u: &str| (u == "out.mtl").then(|| mtl.clone());
    let back = read(&e.files[0].data, Some("out.obj"), &mut res, &limits()).unwrap();
    // node `tri` sits under root (+1 2 3) with scale 2: the triangle (0,0,0)(1,0,0)(0,1,0) becomes
    let p = &back.meshes[0].primitives[0];
    assert_eq!(p.positions, vec![[1.0, 2.0, 3.0], [3.0, 2.0, 3.0], [1.0, 4.0, 3.0]]);
    assert_eq!(p.indices, vec![0, 1, 2]);
    assert_eq!(back.materials[0].name.as_deref(), Some("red"));
}

#[test]
fn a_mirroring_transform_flips_the_winding() {
    let mut s = triangle_scene();
    s.nodes[1].transform = Transform::Trs { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [-1.0, 1.0, 1.0] };
    s.nodes[0].transform = Transform::IDENTITY;
    for target in [Target::Obj, Target::StlBinary] {
        let e = run(&s, target);
        let mtl = e.files.iter().find(|f| f.name == "out.mtl").map(|f| f.data.clone());
        let mut res = |u: &str| (u == "out.mtl").then(|| mtl.clone()).flatten();
        let back = read(&e.files[0].data, Some(&e.files[0].name), &mut res, &limits()).unwrap();
        let p = &back.meshes[0].primitives[0];
        let tri: Vec<[f32; 3]> = p.indices.iter().map(|&i| p.positions[i as usize]).collect();
        // x is mirrored (−1·x, with a −0 that compares equal to 0); the second and third corner swapped
        assert_eq!(tri, vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]], "{target:?}");
    }
}

#[test]
fn obj_names_are_made_safe_unique_and_a_missing_material_is_noted() {
    let mut s = obj_scene();
    s.materials[0].name = Some("my # mat\nx".into());
    s.materials[1].name = Some("my _ mat_x".into()); // collides after cleaning
    s.nodes[0].name = Some("a\nb".into());
    s.materials.push(Material::named(None));
    s.meshes[1].primitives[0].material = None; // a primitive without material after one with
    let e = run(&s, Target::Obj);
    let text = String::from_utf8(e.files[0].data.clone()).unwrap();
    assert!(!text.contains("a\nb"));
    assert!(text.contains("usemtl none"));
    assert!(mentions(&e, "usemtl none"));
    let mtl = String::from_utf8(bytes_of(&e, "out.mtl")).unwrap();
    assert!(mtl.contains("newmtl material_2"));
    let names: Vec<&str> = mtl.lines().filter_map(|l| l.strip_prefix("newmtl ")).collect();
    let mut dedup = names.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(names.len(), dedup.len(), "unique: {names:?}");
}

// ------------------------------------------------------------------------------------------- STL

#[test]
fn binary_stl_round_trips_and_ascii_matches() {
    let src = read(&binary_stl(&[TRI, [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]]], 0), None, &mut NoResources, &limits()).unwrap();
    let e = run(&src, Target::StlBinary);
    assert_eq!(e.files.len(), 1);
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    // the solid name is the header text: ours is the file stem
    assert_eq!(src.meshes[0].primitives, back.meshes[0].primitives);
    assert_eq!(back.nodes[0].name.as_deref(), Some("out"));
    assert_eq!(run(&back, Target::StlBinary).files, e.files);
    assert!(is_binary(&e.files[0].data));
    let a = run(&src, Target::StlAscii);
    let back_a = read(&a.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_eq!(back_a.meshes[0].primitives[0].positions, src.meshes[0].primitives[0].positions);
    assert_eq!(back_a.meshes[0].primitives[0].normals, src.meshes[0].primitives[0].normals);
    assert!(a.report.is_clean(), "{:?}", paths(&a));
    assert!(e.report.is_clean(), "{:?}", paths(&e));
    assert_eq!(&e.files[0].data[..8], b"out\0\0\0\0\0", "the header carries the stem and no time stamp");
}

fn is_binary(d: &[u8]) -> bool {
    detect(d, None) == Some(Format::Stl) && !d.starts_with(b"solid")
}

#[test]
fn a_stem_that_starts_with_solid_does_not_fool_the_header() {
    let src = obj_scene();
    let mut o = opts(Target::StlBinary);
    o.stem = "Solid model".into();
    let e = export(&src, &o, &mut NoResources).unwrap();
    assert!(!e.files[0].data.starts_with(b"solid") && !e.files[0].data.starts_with(b"Solid"));
    assert!(read(&e.files[0].data, None, &mut NoResources, &limits()).is_ok());
}

#[test]
fn obj_to_stl_reports_everything_stl_cannot_hold() {
    let e = run(&obj_scene(), Target::StlBinary);
    assert!(mentions(&e, "no materials"));
    assert!(mentions(&e, "texture coordinates"));
    assert!(mentions(&e, "merged into one solid"));
    assert!(mentions(&e, "names"));
    let back = read(&e.files[0].data, None, &mut NoResources, &limits()).unwrap();
    assert_eq!(back.triangle_count(), 3, "the quad (2) and the triangle (1)");
    let g = run(&triangle_scene(), Target::StlAscii);
    assert!(mentions(&g, "copyright"));
    assert!(mentions(&g, "no node hierarchy") && mentions(&g, "camera") && mentions(&g, "no animation"));
}

#[test]
fn smooth_normals_are_reduced_and_said_so() {
    let mut s = triangle_scene();
    s.meshes[0].primitives[0].normals = vec![[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    assert!(mentions(&run(&s, Target::StlBinary), "facet normal"));
}

// ---------------------------------------------------------------------- limits, hostility, options

#[test]
fn an_inconsistent_scene_is_an_error_not_a_panic() {
    let good = triangle_scene();
    let mut cases: Vec<(&str, Scene)> = Vec::new();
    let mut s = good.clone();
    s.meshes[0].primitives[0].indices = vec![0, 1, 9];
    cases.push(("index", s));
    let mut s = good.clone();
    s.meshes[0].primitives[0].indices.pop();
    cases.push(("not a triangle list", s));
    let mut s = good.clone();
    s.nodes[1].children = vec![0];
    cases.push(("cycle", s));
    let mut s = good.clone();
    s.nodes[1].children = vec![2];
    cases.push(("two parents", s));
    let mut s = good.clone();
    s.nodes[0].children = vec![42];
    cases.push(("child out of range", s));
    let mut s = good.clone();
    s.nodes[1].mesh = Some(7);
    cases.push(("mesh out of range", s));
    let mut s = good.clone();
    s.meshes[0].primitives[0].positions[0][1] = f32::NAN;
    cases.push(("nan", s));
    let mut s = good.clone();
    s.meshes[0].primitives[0].normals = vec![[0.0; 3]];
    cases.push(("normals mismatch", s));
    let mut s = good.clone();
    s.meshes[0].primitives[0].material = Some(5);
    cases.push(("material out of range", s));
    let mut s = good.clone();
    s.animations[0].channels[0].values.pop();
    cases.push(("keys", s));
    let mut s = good.clone();
    s.nodes[0].transform = Transform::Trs { translation: [f32::INFINITY, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };
    cases.push(("infinite translation", s));
    let mut s = good.clone();
    s.roots = vec![99];
    cases.push(("root", s));
    for target in [Target::Glb, Target::Gltf, Target::Obj, Target::StlBinary, Target::StlAscii] {
        for (name, s) in &cases {
            let r = catch_unwind(AssertUnwindSafe(|| export(s, &opts(target), &mut NoResources)));
            assert!(matches!(r, Ok(Err(Error::Invalid(_)))), "{name} / {target:?}: {r:?}");
        }
    }
}

#[test]
fn the_memory_budget_stops_a_big_export_before_it_is_built() {
    let src = triangle_scene();
    for target in [Target::Glb, Target::Obj, Target::StlBinary, Target::StlAscii] {
        let mut o = opts(target);
        o.max_memory_bytes = 16;
        assert!(matches!(export(&src, &o, &mut NoResources), Err(Error::Limit(_))), "{target:?}");
    }
}

#[test]
fn random_scenes_never_panic_the_writers() {
    // mutate the structure of a real scene field by field
    let good = obj_scene();
    let mut rng = Rng(7);
    for _ in 0..1500 {
        let mut s = if rng.below(2) == 0 { good.clone() } else { triangle_scene() };
        for _ in 0..1 + rng.below(3) {
            match rng.below(8) {
                0 => s.roots.push(rng.below(6)),
                1 => {
                    if let Some(n) = s.nodes.get_mut(rng.below(4)) {
                        n.children.push(rng.below(6));
                    }
                }
                2 => {
                    if let Some(p) = s.meshes.get_mut(0).and_then(|m| m.primitives.get_mut(0)) {
                        p.indices.push(rng.below(9) as u32);
                    }
                }
                3 => {
                    if let Some(p) = s.meshes.get_mut(0).and_then(|m| m.primitives.get_mut(0)) {
                        p.positions.truncate(rng.below(4));
                    }
                }
                4 => s.nodes.truncate(rng.below(4)),
                5 => s.materials.truncate(rng.below(2)),
                6 => s.cameras.clear(),
                _ => s.meshes.truncate(rng.below(2)),
            }
        }
        for target in [Target::Glb, Target::Gltf, Target::Obj, Target::StlBinary, Target::StlAscii] {
            let r = catch_unwind(AssertUnwindSafe(|| {
                let e = export(&s, &opts(target), &mut NoResources);
                if let Ok(e) = e {
                    // whatever was written must read back
                    let mtl = e.files.iter().find(|f| f.name.ends_with(".mtl")).map(|f| f.data.clone());
                    let bin = e.files.iter().find(|f| f.name.ends_with(".bin")).map(|f| f.data.clone());
                    let mut res = |u: &str| if u.ends_with(".mtl") { mtl.clone() } else { bin.clone() };
                    let _ = read(&e.files[0].data, Some(&e.files[0].name), &mut res, &limits());
                }
            }));
            assert!(r.is_ok(), "panic on {target:?}: {s:?}");
        }
    }
}

#[test]
fn stems_targets_and_binary_layout() {
    assert_eq!(sanitize_stem("../etc/passwd"), "_etc_passwd");
    assert_eq!(sanitize_stem(""), "scene");
    assert_eq!(sanitize_stem("  ..  "), "scene");
    assert_eq!(sanitize_stem("a:b\0c"), "a_b_c");
    assert_eq!(sanitize_stem(&"x".repeat(300)).len(), 100);
    assert_eq!(sanitize_stem("Szék modell-1.2"), "Szék modell-1.2");
    for (n, t) in [("GLB", Target::Glb), ("gltf", Target::Gltf), ("obj", Target::Obj), ("stl", Target::StlBinary), ("stl-ascii", Target::StlAscii)] {
        assert_eq!(Target::from_name(n), Some(t));
        assert_eq!(Target::from_name(t.name()), Some(t));
    }
    assert_eq!(Target::from_name("usd"), None);
    // a GLB is 4-byte aligned in both chunks and its length field is the file size
    let e = run(&triangle_scene(), Target::Glb);
    let d = &e.files[0].data;
    assert_eq!(u32::from_le_bytes([d[8], d[9], d[10], d[11]]) as usize, d.len());
    assert_eq!(d.len() % 4, 0);
    let _ = f32s(&[]);
}
