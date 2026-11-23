//! Wavefront OBJ (+ MTL) writer.
//!
//! One `o` object per mesh-carrying node, in node order, with the node's world transform applied to the
//! vertices (OBJ has no node transforms; an identity transform leaves the numbers untouched). Vertices,
//! texture coordinates and normals are written per primitive, faces by index, `usemtl` per primitive.
//! Numbers use the shortest decimal text of the `f32`, so reading the file back gives the same bits.
//! A material library (`<stem>.mtl`) is written when the scene has materials.

use std::fmt::Write as _;

use crate::util::Budget;
use crate::write::{ExportFile, ExportOptions, Instance, charge, det3, flat_target_report, instances, is_identity, transform_normal, transform_point};
use crate::{Format, Material, Report, Result, Scene};

/// Text that is safe on one OBJ/MTL line: no control characters, no `#` (a comment), trimmed.
fn line_text(s: &str) -> String {
    s.chars().map(|c| if c.is_control() || c == '#' { '_' } else { c }).collect::<String>().trim().to_string()
}

/// Unique, non-empty, single-line material names.
fn material_names(materials: &[Material]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(materials.len());
    for (i, m) in materials.iter().enumerate() {
        let base = m.name.as_deref().map(line_text).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("material_{i}"));
        let mut name = base.clone();
        let mut k = 1usize;
        while out.contains(&name) {
            name = format!("{base}_{k}");
            k += 1;
        }
        out.push(name);
    }
    out
}

fn mtl_slot(slot: &str) -> Option<(&'static str, bool)> {
    match slot {
        "baseColor" | "map_Kd" => Some(("map_Kd", false)),
        "emissive" | "map_Ke" => Some(("map_Ke", false)),
        "normal" => Some(("map_Bump", true)),
        "bump" | "map_Bump" | "map_bump" => Some(("map_Bump", false)),
        _ => None,
    }
}

fn mtl(scene: &Scene, names: &[String], report: &mut Report, textures: &mut Vec<String>) -> String {
    let mut s = String::from("# CFA4 material library\n");
    let mut pbr_noted = false;
    let mut sided_noted = false;
    for (i, (m, name)) in scene.materials.iter().zip(names).enumerate() {
        let _ = writeln!(s, "newmtl {name}");
        let _ = writeln!(s, "Kd {} {} {}", m.base_color[0], m.base_color[1], m.base_color[2]);
        if m.base_color[3] != 1.0 {
            let _ = writeln!(s, "d {}", m.base_color[3]);
        }
        if m.emissive != [0.0; 3] {
            let _ = writeln!(s, "Ke {} {} {}", m.emissive[0], m.emissive[1], m.emissive[2]);
        }
        if scene.format != Format::Obj && !pbr_noted && (m.metallic, m.roughness) != (0.0, 1.0) {
            pbr_noted = true;
            report.approximated(
                "materials",
                "the metallic and roughness factors have no MTL equivalent: only the base colour (Kd), opacity (d) and emission (Ke) are written",
            );
        }
        if m.double_sided && !sided_noted {
            sided_noted = true;
            report.unsupported("materials", "double-sided materials are not carried by MTL");
        }
        for t in &m.textures {
            let path = format!("materials[{i}].{}", t.slot);
            let Some(uri) = t.uri.as_deref() else {
                report.unsupported(path, "an embedded texture image is not part of the scene model, so it cannot be written");
                continue;
            };
            let Some((key, approx)) = mtl_slot(&t.slot) else {
                report.unsupported(path, format!("texture slot {:?} has no MTL map statement and is dropped", t.slot));
                continue;
            };
            if approx {
                report.approximated(path, "the normal map is written as `map_Bump` (MTL has no separate normal-map statement)");
            }
            let _ = writeln!(s, "{key} {uri}");
            textures.push(uri.to_string());
        }
        s.push('\n');
    }
    s
}

fn push_v3(out: &mut String, key: &str, v: [f32; 3]) {
    let _ = writeln!(out, "{key} {} {} {}", v[0], v[1], v[2]);
}

pub(crate) fn write(
    scene: &Scene,
    opts: &ExportOptions,
    stem: &str,
    budget: &mut Budget,
    report: &mut Report,
    textures: &mut Vec<String>,
) -> Result<Vec<ExportFile>> {
    let inst = instances(scene);
    // about 100 bytes per vertex (three attribute lines) and 80 per triangle, per instance
    let (mut verts, mut tris) = (0usize, 0usize);
    for i in &inst {
        for p in scene.meshes.get(i.mesh).map(|m| m.primitives.as_slice()).unwrap_or(&[]) {
            verts = verts.saturating_add(p.positions.len());
            tris = tris.saturating_add(p.indices.len() / 3);
        }
    }
    charge(
        budget,
        verts.saturating_mul(100).saturating_add(tris.saturating_mul(80)).saturating_add(scene.materials.len().saturating_mul(512)).saturating_add(1024),
    )?;
    flat_target_report(scene, "OBJ", &inst, report);

    let names = material_names(&scene.materials);
    let mut out = String::new();
    let _ = writeln!(out, "# {}", line_text(&opts.generator));
    if let Some(c) = scene.asset.copyright.as_deref().map(line_text).filter(|c| !c.is_empty()) {
        let _ = writeln!(out, "# Copyright: {c}");
    }
    let has_mtl = !scene.materials.is_empty();
    if has_mtl {
        let _ = writeln!(out, "mtllib {stem}.mtl");
    }
    let mut base = 0usize; // vertices written so far (OBJ indices are file-global, from 1)
    let mut current: Option<usize> = None; // material set by the last usemtl
    let mut none_noted = false;
    for Instance { node, mesh, world } in &inst {
        let Some(m) = scene.meshes.get(*mesh) else { continue };
        let name = scene.nodes.get(*node).and_then(|n| n.name.as_deref()).map(line_text).unwrap_or_default();
        let _ = writeln!(out, "o {name}");
        let ident = is_identity(world);
        let flip = !ident && det3(world) < 0.0;
        for p in &m.primitives {
            if p.indices.is_empty() || p.positions.is_empty() {
                continue;
            }
            match (p.material, current) {
                (Some(mi), cur) if cur != Some(mi) => {
                    let _ = writeln!(out, "usemtl {}", names.get(mi).map_or("none", String::as_str));
                    current = Some(mi);
                }
                (None, Some(_)) => {
                    // OBJ cannot unset a material
                    if !none_noted {
                        none_noted = true;
                        report.approximated(
                            "materials",
                            "a primitive without a material follows one with a material: it is written with `usemtl none` (OBJ cannot unset a material)",
                        );
                    }
                    let _ = writeln!(out, "usemtl none");
                    current = None;
                }
                _ => {}
            }
            for v in &p.positions {
                push_v3(&mut out, "v", if ident { *v } else { transform_point(world, *v) });
            }
            for t in &p.texcoords {
                let _ = writeln!(out, "vt {} {}", t[0], t[1]);
            }
            for n in &p.normals {
                push_v3(&mut out, "vn", if ident { *n } else { transform_normal(world, *n) });
            }
            let (has_uv, has_n) = (!p.texcoords.is_empty(), !p.normals.is_empty());
            for tri in p.indices.chunks_exact(3) {
                let mut t = [tri[0], tri[1], tri[2]];
                if flip {
                    t.swap(1, 2);
                }
                out.push('f');
                for i in t {
                    let a = base + i as usize + 1;
                    let _ = match (has_uv, has_n) {
                        (true, true) => write!(out, " {a}/{a}/{a}"),
                        (true, false) => write!(out, " {a}/{a}"),
                        (false, true) => write!(out, " {a}//{a}"),
                        (false, false) => write!(out, " {a}"),
                    };
                }
                out.push('\n');
            }
            base += p.positions.len();
        }
    }
    let mut files = vec![ExportFile { name: format!("{stem}.obj"), data: out.into_bytes() }];
    if has_mtl {
        files.push(ExportFile { name: format!("{stem}.mtl"), data: mtl(scene, &names, report, textures).into_bytes() });
    }
    Ok(files)
}
