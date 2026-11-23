//! STL reader: binary (80-byte header, `u32` triangle count, 50 bytes per triangle) and ASCII
//! (`solid` … `endsolid`, possibly several solids). STL is a bare triangle soup: one node, one mesh, one
//! primitive per solid; vertices are not welded; no materials, hierarchy, cameras or animation.

use crate::util::{Budget, finite_f32};
use crate::{Asset, Error, Format, Limits, Mesh, Node, Primitive, Result, Scene, Transform, Units, invalid};

const HEADER: usize = 80;
const TRI: usize = 50;

/// A binary STL: the size matches the triangle count exactly (an ASCII file that happens to start with
/// `solid` can never match, and a binary file whose header starts with `solid` still does).
pub(crate) fn is_binary_stl(data: &[u8]) -> bool {
    let Some(count) = data.get(HEADER..HEADER + 4) else { return false };
    let n = u64::from(u32::from_le_bytes([count[0], count[1], count[2], count[3]]));
    n.checked_mul(TRI as u64).and_then(|b| b.checked_add(HEADER as u64 + 4)) == Some(data.len() as u64)
}

pub(crate) fn read(data: &[u8], limits: &Limits) -> Result<Scene> {
    let mut scene = Scene::empty(Format::Stl, Units::Unspecified, limits.max_report_items);
    scene.asset = Asset::default();
    let mut budget = Budget::new(limits.max_memory_bytes);
    if is_binary_stl(data) || (data.len() >= HEADER + 4 && !looks_ascii(data)) {
        read_binary(data, &mut scene, &mut budget)?;
    } else {
        read_ascii(data, limits, &mut scene, &mut budget)?;
    }
    Ok(scene)
}

fn looks_ascii(data: &[u8]) -> bool {
    let head = data.get(..data.len().min(256)).unwrap_or(data);
    let trimmed = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let start = trimmed.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    trimmed.get(start..).is_some_and(|t| t.starts_with(b"solid"))
}

fn f32_at(b: &[u8], off: usize) -> Result<f32> {
    let s = b.get(off..off + 4).ok_or_else(|| Error::Invalid("truncated STL".into()))?;
    let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    if v.is_finite() { Ok(v) } else { invalid("non-finite STL coordinate") }
}

fn read_binary(data: &[u8], scene: &mut Scene, budget: &mut Budget) -> Result<()> {
    let Some(count) = data.get(HEADER..HEADER + 4) else {
        return invalid("truncated STL header");
    };
    let n = u32::from_le_bytes([count[0], count[1], count[2], count[3]]) as usize;
    let body = data.get(HEADER + 4..).unwrap_or(&[]);
    let need = n.checked_mul(TRI).ok_or_else(|| Error::Invalid("STL triangle count overflows".into()))?;
    if body.len() < need {
        return invalid(format!("truncated STL: {n} triangles declared, {} bytes of data", body.len()));
    }
    if body.len() > need {
        scene.report.approximated("trailing bytes", format!("{} bytes after the last triangle ignored", body.len() - need));
    }
    budget.take(n.saturating_mul(3), std::mem::size_of::<[f32; 3]>() * 2 + 4)?;
    let mut prim = Primitive::default();
    prim.positions.reserve_exact(n * 3);
    prim.normals.reserve_exact(n * 3);
    prim.indices.reserve_exact(n * 3);
    let mut attributes = false;
    let mut computed = false;
    for (t, rec) in body.chunks_exact(TRI).take(n).enumerate() {
        let mut nrm = [f32_at(rec, 0)?, f32_at(rec, 4)?, f32_at(rec, 8)?];
        let mut v = [[0.0f32; 3]; 3];
        for (k, vk) in v.iter_mut().enumerate() {
            *vk = [f32_at(rec, 12 + k * 12)?, f32_at(rec, 16 + k * 12)?, f32_at(rec, 20 + k * 12)?];
        }
        if rec.get(48..50).is_some_and(|a| a != [0, 0]) {
            attributes = true;
        }
        if nrm == [0.0; 3] {
            nrm = facet_normal(&v);
            computed = true;
        }
        let base = u32::try_from(t * 3).map_err(|_| Error::Limit("too many STL vertices".into()))?;
        for (k, vk) in v.iter().enumerate() {
            prim.positions.push(*vk);
            prim.normals.push(nrm);
            prim.indices.push(base + k as u32);
        }
    }
    if attributes {
        scene.report.unsupported("attribute bytes", "per-triangle attribute bytes (colour/material in some writers) are not carried");
    }
    if computed {
        scene.report.approximated("normals", "zero facet normals recomputed from the vertices");
    }
    let header = String::from_utf8_lossy(data.get(..HEADER).unwrap_or(&[]));
    let name = header.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    let name = (!name.is_empty() && name.chars().all(|c| !c.is_control())).then(|| budget.string(name)).transpose()?;
    push_solid(scene, name, prim);
    Ok(())
}

fn facet_normal(v: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
    let b = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
    let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    if len.is_finite() && len > 0.0 { [c[0] / len, c[1] / len, c[2] / len] } else { [0.0; 3] }
}

fn push_solid(scene: &mut Scene, name: Option<String>, prim: Primitive) {
    let mesh = scene.meshes.len();
    scene.meshes.push(Mesh { name: name.clone(), primitives: vec![prim] });
    let node = scene.nodes.len();
    scene.nodes.push(Node { name, parent: None, children: Vec::new(), transform: Transform::IDENTITY, mesh: Some(mesh), camera: None });
    scene.roots.push(node);
}

#[derive(Default)]
struct Solid {
    name: Option<String>,
    prim: Primitive,
    /// Vertices of the current facet and its stated normal.
    facet: Vec<[f32; 3]>,
    normal: Option<[f32; 3]>,
}

fn read_ascii(data: &[u8], limits: &Limits, scene: &mut Scene, budget: &mut Budget) -> Result<()> {
    let text = String::from_utf8_lossy(data);
    let mut cur: Option<Solid> = None;
    let mut computed = false;
    for (ln, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let mut tok = line.split_whitespace();
        let Some(kw) = tok.next() else { continue };
        match kw.to_ascii_lowercase().as_str() {
            "solid" => {
                if cur.is_some() {
                    return invalid(format!("line {}: nested solid", ln + 1));
                }
                if scene.nodes.len() >= limits.max_nodes {
                    return Err(Error::Limit("too many STL solids".into()));
                }
                let rest = line.get(kw.len()..).unwrap_or("").trim();
                let name = (!rest.is_empty()).then(|| budget.string(rest)).transpose()?;
                cur = Some(Solid { name, ..Solid::default() });
            }
            "endsolid" => {
                let Some(s) = cur.take() else {
                    return invalid(format!("line {}: endsolid without solid", ln + 1));
                };
                if !s.facet.is_empty() {
                    return invalid(format!("line {}: incomplete facet", ln + 1));
                }
                push_solid(scene, s.name, s.prim);
            }
            "facet" => {
                let Some(s) = cur.as_mut() else {
                    return invalid(format!("line {}: facet outside solid", ln + 1));
                };
                if !s.facet.is_empty() {
                    return invalid(format!("line {}: facet inside facet", ln + 1));
                }
                // `facet normal nx ny nz`
                let mut n = [0.0f32; 3];
                if tok.next().is_some_and(|w| w.eq_ignore_ascii_case("normal")) {
                    for c in &mut n {
                        *c = finite_f32(tok.next().unwrap_or(""))?;
                    }
                    s.normal = Some(n);
                } else {
                    s.normal = None;
                }
            }
            "vertex" => {
                let Some(s) = cur.as_mut() else {
                    return invalid(format!("line {}: vertex outside solid", ln + 1));
                };
                let mut v = [0.0f32; 3];
                for c in &mut v {
                    *c = finite_f32(tok.next().unwrap_or(""))?;
                }
                s.facet.push(v);
                if s.facet.len() == 3 {
                    budget.take(3, std::mem::size_of::<[f32; 3]>() * 2 + 4)?;
                    let tri = [s.facet[0], s.facet[1], s.facet[2]];
                    let nrm = match s.normal {
                        Some(n) if n != [0.0; 3] => n,
                        _ => {
                            computed = true;
                            facet_normal(&tri)
                        }
                    };
                    let base = u32::try_from(s.prim.positions.len()).map_err(|_| Error::Limit("too many STL vertices".into()))?;
                    for (k, p) in tri.iter().enumerate() {
                        s.prim.positions.push(*p);
                        s.prim.normals.push(nrm);
                        s.prim.indices.push(base + k as u32);
                    }
                    s.facet.clear();
                } else if s.facet.len() > 3 {
                    return invalid(format!("line {}: more than three vertices in a facet", ln + 1));
                }
            }
            "outer" | "endloop" | "endfacet" => {}
            other => return invalid(format!("line {}: unexpected STL keyword {other:?}", ln + 1)),
        }
    }
    if cur.is_some() {
        return invalid("truncated STL: missing endsolid");
    }
    if scene.nodes.is_empty() {
        return invalid("no solid in STL file");
    }
    if computed {
        scene.report.approximated("normals", "missing or zero facet normals recomputed from the vertices");
    }
    Ok(())
}
