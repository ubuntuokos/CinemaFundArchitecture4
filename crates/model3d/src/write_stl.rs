//! STL writer, binary or ASCII: every mesh instance becomes triangles of one solid, with its world
//! transform applied (an identity transform leaves the numbers untouched). STL carries geometry only; the
//! loss report names everything else. The facet normal is the shared vertex normal when the three agree
//! (so a read file writes back unchanged), otherwise it is computed from the triangle.

use std::fmt::Write as _;

use crate::util::Budget;
use crate::write::{ExportFile, ExportOptions, Mat, Target, charge, det3, flat_target_report, instances, is_identity, transform_normal, transform_point};
use crate::{Error, Format, Primitive, Report, Result, Scene};

type Tri = ([[f32; 3]; 3], [f32; 3]);

fn facet_normal(v: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
    let b = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
    let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    if len.is_finite() && len > 0.0 { [c[0] / len, c[1] / len, c[2] / len] } else { [0.0; 3] }
}

/// The triangles of one primitive under `world`; `smooth` is set when a facet's vertex normals differ.
fn triangles(p: &Primitive, world: &Mat, out: &mut Vec<Tri>, smooth: &mut bool) {
    let ident = is_identity(world);
    let flip = !ident && det3(world) < 0.0;
    for tri in p.indices.chunks_exact(3) {
        let mut idx = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        if flip {
            idx.swap(1, 2);
        }
        let mut v = [[0.0f32; 3]; 3];
        for (k, &i) in idx.iter().enumerate() {
            let pos = p.positions.get(i).copied().unwrap_or([0.0; 3]);
            v[k] = if ident { pos } else { transform_point(world, pos) };
        }
        let stated = if p.normals.is_empty() {
            None
        } else {
            let n = |k: usize| {
                let n = p.normals.get(idx[k]).copied().unwrap_or([0.0; 3]);
                if ident { n } else { transform_normal(world, n) }
            };
            let (a, b, c) = (n(0), n(1), n(2));
            if a == b && b == c {
                Some(a)
            } else {
                *smooth = true;
                None
            }
        };
        let normal = stated.filter(|n| *n != [0.0; 3]).unwrap_or_else(|| facet_normal(&v));
        out.push((v, normal));
    }
}

fn header_text(stem: &str) -> Vec<u8> {
    let mut text: String = stem.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' }).collect();
    if text.to_ascii_lowercase().starts_with("solid") {
        text.insert_str(0, "CFA4 ");
    }
    let mut h = text.into_bytes();
    h.truncate(80);
    h.resize(80, 0);
    h
}

pub(crate) fn write(scene: &Scene, opts: &ExportOptions, stem: &str, budget: &mut Budget, report: &mut Report) -> Result<Vec<ExportFile>> {
    let inst = instances(scene);
    let total_tris: usize =
        inst.iter().filter_map(|i| scene.meshes.get(i.mesh)).flat_map(|m| &m.primitives).map(|p| p.indices.len() / 3).fold(0usize, usize::saturating_add);
    let per_tri = if opts.target == Target::StlAscii { 260 } else { 50 };
    // the triangles are collected first (about 48 bytes each), then written
    charge(budget, total_tris.saturating_mul(per_tri + 48).saturating_add(1024))?;
    if total_tris > u32::MAX as usize {
        return Err(Error::Limit("an STL file holds at most 4 294 967 295 triangles".into()));
    }
    flat_target_report(scene, "STL", &inst, report);
    if inst.len() > 1 {
        report.approximated("meshes", format!("{} mesh instances are merged into one solid", inst.len()));
    }
    if !scene.materials.is_empty() {
        report.unsupported("materials", "STL carries no materials or textures");
    }
    if scene.meshes.iter().flat_map(|m| &m.primitives).any(|p| !p.texcoords.is_empty()) {
        report.unsupported("texcoords", "STL carries no texture coordinates");
    }
    if scene.asset.copyright.as_deref().is_some_and(|c| !c.trim().is_empty()) {
        report.unsupported("asset.copyright", "STL has no place for the copyright text; keep it with the project or the source file");
    }
    if scene.format != Format::Stl && scene.nodes.iter().any(|n| n.name.is_some()) {
        report.unsupported("names", "STL carries no node or mesh names (the file name is used)");
    }
    let mut tris: Vec<Tri> = Vec::with_capacity(total_tris);
    let mut smooth = false;
    for i in &inst {
        for p in scene.meshes.get(i.mesh).map(|m| m.primitives.as_slice()).unwrap_or(&[]) {
            triangles(p, &i.world, &mut tris, &mut smooth);
        }
    }
    if smooth {
        report.approximated("normals", "smooth vertex normals are reduced to one facet normal per triangle");
    }
    if opts.target == Target::StlAscii {
        let name: String = stem.chars().map(|c| if c.is_ascii_graphic() { c } else { '_' }).collect();
        let mut s = String::new();
        let _ = writeln!(s, "solid {name}");
        for (v, n) in &tris {
            let _ = writeln!(s, "  facet normal {} {} {}\n    outer loop", n[0], n[1], n[2]);
            for p in v {
                let _ = writeln!(s, "      vertex {} {} {}", p[0], p[1], p[2]);
            }
            s.push_str("    endloop\n  endfacet\n");
        }
        let _ = writeln!(s, "endsolid {name}");
        return Ok(vec![ExportFile { name: format!("{stem}.stl"), data: s.into_bytes() }]);
    }
    let mut data = header_text(stem);
    data.reserve(4 + tris.len() * 50);
    data.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for (v, n) in &tris {
        for c in n.iter().chain(v.iter().flatten()) {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&[0, 0]);
    }
    Ok(vec![ExportFile { name: format!("{stem}.stl"), data }])
}
