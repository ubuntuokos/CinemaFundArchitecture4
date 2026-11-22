//! Region and level-of-detail selection for large meshes (CFA4-OPENUSD-3D-001 U1-A3).
//!
//! The rule is a *pure function* `(scene, frame) → plan`: deterministic, no I/O, no clock. The frame is
//! the resource budget the caller took from the ACS (memory, optionally a triangle cap). Every decision
//! carries its reasons (which frame forced it, the ARF-A3 / ACS `route_decision` principle), and a level
//! that changes the result (anything but [`Level::Full`]) is flagged `needs_approval`: quality is never
//! reduced silently (ACS conflict rule 3). [`apply`] performs the plan only when the caller says it was
//! approved.
//!
//! This module defines no descriptor schema: the ARF layer (CFA4-ARF-001) owns that. The plan is plain
//! numbers over a [`Scene`], so it can be mapped onto an ARF descriptor when that code exists.
//!
//! Positions are in each mesh's own space (node transforms are not applied): bounds and regions are
//! therefore a property of the asset's geometry, which is what a load-time decision needs.

use std::collections::HashMap;

use crate::{Scene, Severity};

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    /// Whether `p` lies inside the box (edges included).
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }

    /// A box is usable when every coordinate is finite and `min <= max`.
    pub fn is_valid(&self) -> bool {
        (0..3).all(|i| self.min[i].is_finite() && self.max[i].is_finite() && self.min[i] <= self.max[i])
    }
}

impl Scene {
    /// The box around every mesh vertex, or `None` for a scene with no geometry.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut b: Option<Bounds> = None;
        for p in self.meshes.iter().flat_map(|m| &m.primitives).flat_map(|p| &p.positions) {
            match &mut b {
                None => b = Some(Bounds { min: *p, max: *p }),
                Some(b) => {
                    for i in 0..3 {
                        b.min[i] = b.min[i].min(p[i]);
                        b.max[i] = b.max[i].max(p[i]);
                    }
                }
            }
        }
        b
    }

    /// Bytes the mesh data occupies once loaded (positions, normals, texture coordinates, indices).
    pub fn mesh_bytes(&self) -> usize {
        self.meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .map(|p| {
                p.positions.len().saturating_mul(12)
                    + p.normals.len().saturating_mul(12)
                    + p.texcoords.len().saturating_mul(8)
                    + p.indices.len().saturating_mul(4)
            })
            .fold(0usize, usize::saturating_add)
    }
}

/// The resource frame a decision is made against (from the ACS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Memory the mesh data may occupy.
    pub memory_bytes: usize,
    /// Most triangles the consumer can draw, if limited.
    pub max_triangles: Option<usize>,
}

/// A level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The data as read.
    Full,
    /// Vertex clustering on a `grid`³ lattice over the scene bounds.
    Clustered { grid: u32 },
}

/// A candidate level with its estimated cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelEstimate {
    pub level: Level,
    pub triangles: usize,
    pub bytes: usize,
}

/// The outcome of [`plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Every level considered, finest first.
    pub levels: Vec<LevelEstimate>,
    /// Index into `levels` of the chosen one.
    pub chosen: usize,
    /// The chosen level fits the frame. `false` only when even the coarsest level does not.
    pub fits: bool,
    /// The result differs from the data as read.
    pub changes_result: bool,
    /// Applying the plan reduces quality: the caller must obtain approval first.
    pub needs_approval: bool,
    /// Why this level, naming the frame that forced it.
    pub reasons: Vec<String>,
}

/// Lattice sizes tried after [`Level::Full`], fine to coarse.
const GRIDS: [u32; 6] = [256, 128, 64, 32, 16, 8];

/// Choose the finest level that fits `frame`. The estimate for a clustered level is an upper bound:
/// a surface through a `g`³ lattice touches at most about `4·g²` triangles' worth of cells, and the data
/// shrinks in proportion to the triangles kept.
pub fn plan(scene: &Scene, frame: Frame) -> Plan {
    let tris = scene.triangle_count();
    let bytes = scene.mesh_bytes();
    let mut levels = vec![LevelEstimate { level: Level::Full, triangles: tris, bytes }];
    for g in GRIDS {
        let cap = (g as usize).saturating_mul(g as usize).saturating_mul(4);
        let t = tris.min(cap);
        let b = if tris == 0 { 0 } else { (bytes as u128 * t as u128 / tris as u128) as usize };
        levels.push(LevelEstimate { level: Level::Clustered { grid: g }, triangles: t, bytes: b });
    }
    let fits_frame = |l: &LevelEstimate| l.bytes <= frame.memory_bytes && frame.max_triangles.is_none_or(|m| l.triangles <= m);
    let first_fit = levels.iter().position(fits_frame);
    let (chosen, fits) = match first_fit {
        Some(i) => (i, true),
        None => (levels.len() - 1, false),
    };
    let mut reasons = Vec::new();
    if let Some(full) = levels.first() {
        if chosen == 0 {
            reasons.push(format!(
                "full detail fits the frame ({} bytes of {}, {} triangles{})",
                full.bytes,
                frame.memory_bytes,
                full.triangles,
                frame.max_triangles.map_or(String::new(), |m| format!(" of {m}"))
            ));
        } else {
            if full.bytes > frame.memory_bytes {
                reasons.push(format!("full detail needs {} bytes, the memory frame allows {}", full.bytes, frame.memory_bytes));
            }
            if let Some(m) = frame.max_triangles.filter(|m| full.triangles > *m) {
                reasons.push(format!("full detail has {} triangles, the triangle frame allows {m}", full.triangles));
            }
            if let Some(c) = levels.get(chosen) {
                if fits {
                    reasons.push(format!("the finest level that fits is {:?}: about {} triangles, {} bytes", c.level, c.triangles, c.bytes));
                } else {
                    reasons.push(format!("not even the coarsest level fits ({:?}: about {} triangles, {} bytes)", c.level, c.triangles, c.bytes));
                }
            }
        }
    }
    Plan { levels, chosen, fits, changes_result: chosen != 0, needs_approval: chosen != 0, reasons }
}

/// Keep only the triangles whose centroid lies in `region`; vertices no triangle uses are dropped.
/// The scene is otherwise unchanged and the report says what was cut.
pub fn extract_region(scene: &Scene, region: Bounds) -> Scene {
    let mut out = scene.clone();
    let (mut kept, mut total) = (0usize, 0usize);
    for prim in out.meshes.iter_mut().flat_map(|m| m.primitives.iter_mut()) {
        let src = prim.clone();
        total += src.indices.len() / 3;
        let mut remap: HashMap<u32, u32> = HashMap::new();
        *prim = crate::Primitive { material: src.material, ..Default::default() };
        for tri in src.indices.chunks_exact(3) {
            let pts: Option<Vec<[f32; 3]>> = tri.iter().map(|&i| src.positions.get(i as usize).copied()).collect();
            let Some(pts) = pts else { continue };
            let c = [0, 1, 2].map(|k| (pts[0][k] + pts[1][k] + pts[2][k]) / 3.0);
            if !region.contains(c) {
                continue;
            }
            kept += 1;
            for &i in tri {
                let next = u32::try_from(remap.len()).unwrap_or(u32::MAX);
                let new = *remap.entry(i).or_insert_with(|| {
                    let idx = i as usize;
                    if let Some(p) = src.positions.get(idx) {
                        prim.positions.push(*p);
                    }
                    if let Some(n) = src.normals.get(idx) {
                        prim.normals.push(*n);
                    }
                    if let Some(t) = src.texcoords.get(idx) {
                        prim.texcoords.push(*t);
                    }
                    next
                });
                prim.indices.push(new);
            }
        }
    }
    out.report.add(Severity::Approximated, "region", format!("region crop kept {kept} of {total} triangles"));
    out
}

/// Vertex clustering on a `grid`³ lattice over the scene bounds: vertices in one cell merge (positions
/// and normals averaged), triangles that collapse are dropped.
pub fn cluster(scene: &Scene, grid: u32) -> Scene {
    let mut out = scene.clone();
    let grid = grid.max(1);
    let Some(b) = scene.bounds() else { return out };
    let cell = |p: &[f32; 3]| -> [u32; 3] {
        [0, 1, 2].map(|k| {
            let span = b.max[k] - b.min[k];
            if span <= 0.0 || !span.is_finite() {
                0
            } else {
                let t = ((p[k] - b.min[k]) / span * grid as f32).floor();
                (t.max(0.0) as u32).min(grid - 1)
            }
        })
    };
    let before = scene.triangle_count();
    for prim in out.meshes.iter_mut().flat_map(|m| m.primitives.iter_mut()) {
        let src = prim.clone();
        let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
        // per cluster: position sum, normal sum, count, first texcoord
        let mut acc: Vec<([f32; 3], [f32; 3], u32, Option<[f32; 2]>)> = Vec::new();
        let mut map: Vec<u32> = Vec::with_capacity(src.positions.len());
        for (i, p) in src.positions.iter().enumerate() {
            let key = cell(p);
            let next = u32::try_from(acc.len()).unwrap_or(u32::MAX);
            let id = *ids.entry(key).or_insert_with(|| {
                acc.push(([0.0; 3], [0.0; 3], 0, src.texcoords.get(i).copied()));
                next
            });
            if let Some(a) = acc.get_mut(id as usize) {
                for k in 0..3 {
                    a.0[k] += p[k];
                    if let Some(n) = src.normals.get(i) {
                        a.1[k] += n[k];
                    }
                }
                a.2 += 1;
            }
            map.push(id);
        }
        let has_n = src.normals.len() == src.positions.len() && !src.normals.is_empty();
        let has_t = src.texcoords.len() == src.positions.len() && !src.texcoords.is_empty();
        prim.positions = acc.iter().map(|a| a.0.map(|v| v / a.2.max(1) as f32)).collect();
        prim.normals = if has_n {
            acc.iter()
                .map(|a| {
                    let l = (a.1[0] * a.1[0] + a.1[1] * a.1[1] + a.1[2] * a.1[2]).sqrt();
                    if l > 0.0 { a.1.map(|v| v / l) } else { [0.0, 0.0, 1.0] }
                })
                .collect()
        } else {
            Vec::new()
        };
        prim.texcoords = if has_t { acc.iter().map(|a| a.3.unwrap_or([0.0; 2])).collect() } else { Vec::new() };
        prim.indices = Vec::with_capacity(src.indices.len());
        for tri in src.indices.chunks_exact(3) {
            let m: Option<Vec<u32>> = tri.iter().map(|&i| map.get(i as usize).copied()).collect();
            if let Some(m) = m
                && m[0] != m[1]
                && m[1] != m[2]
                && m[0] != m[2]
            {
                prim.indices.extend_from_slice(&m);
            }
        }
    }
    let after = out.triangle_count();
    out.report.add(Severity::Approximated, "lod", format!("clustered on a {grid}^3 lattice: {before} -> {after} triangles"));
    out
}

/// Perform `plan` on `scene`. A plan that changes the result is refused (`None`) unless `approved`.
pub fn apply(scene: &Scene, plan: &Plan, approved: bool) -> Option<Scene> {
    let level = plan.levels.get(plan.chosen)?.level;
    match level {
        Level::Full => Some(scene.clone()),
        Level::Clustered { grid } => approved.then(|| cluster(scene, grid)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, Mesh, Primitive, Units};

    /// An `n`×`n` quad grid in the XY plane, two triangles per quad.
    fn grid_scene(n: u32) -> Scene {
        let mut s = Scene::empty(Format::Obj, Units::Unspecified, 100);
        let mut p = Primitive::default();
        for y in 0..=n {
            for x in 0..=n {
                p.positions.push([x as f32, y as f32, 0.0]);
                p.normals.push([0.0, 0.0, 1.0]);
            }
        }
        let w = n + 1;
        for y in 0..n {
            for x in 0..n {
                let a = y * w + x;
                p.indices.extend_from_slice(&[a, a + 1, a + w, a + 1, a + w + 1, a + w]);
            }
        }
        s.meshes.push(Mesh { name: None, primitives: vec![p] });
        s
    }

    #[test]
    fn a_scene_that_fits_keeps_full_detail_without_approval() {
        let s = grid_scene(8);
        let plan = plan(&s, Frame { memory_bytes: 1 << 20, max_triangles: None });
        assert_eq!((plan.chosen, plan.fits, plan.changes_result, plan.needs_approval), (0, true, false, false));
        assert!(plan.reasons[0].contains("fits"));
        assert!(apply(&s, &plan, false).is_some_and(|x| x == s));
    }

    #[test]
    fn a_tight_frame_picks_the_finest_fitting_level_with_reasons_and_needs_approval() {
        let s = grid_scene(100); // 20 000 triangles
        let frame = Frame { memory_bytes: usize::MAX, max_triangles: Some(5_000) };
        let a = plan(&s, frame);
        assert_eq!(a, plan(&s, frame), "deterministic");
        assert!(a.fits && a.changes_result && a.needs_approval);
        let chosen = a.levels[a.chosen];
        assert!(chosen.triangles <= 5_000);
        assert!(a.levels[..a.chosen].iter().all(|l| l.triangles > 5_000), "the finest fitting level");
        assert!(a.reasons.iter().any(|r| r.contains("triangle frame")), "{:?}", a.reasons);
        assert!(apply(&s, &a, false).is_none(), "quality is not reduced without approval");
        let out = apply(&s, &a, true).expect("approved");
        assert!(out.triangle_count() < s.triangle_count());
        assert!(out.report.items.iter().any(|i| i.path == "lod"), "the cut is in the report");
    }

    #[test]
    fn nothing_fits_is_said_so() {
        let s = grid_scene(50);
        let p = plan(&s, Frame { memory_bytes: 10, max_triangles: None });
        assert!(!p.fits);
        assert_eq!(p.chosen, p.levels.len() - 1);
        assert!(p.reasons.iter().any(|r| r.contains("not even")));
    }

    #[test]
    fn clustering_is_valid_and_keeps_bounds() {
        let s = grid_scene(40);
        let c = cluster(&s, 8);
        assert!(c.triangle_count() < s.triangle_count());
        let prim = &c.meshes[0].primitives[0];
        assert!(prim.indices.iter().all(|&i| (i as usize) < prim.positions.len()));
        assert_eq!(prim.normals.len(), prim.positions.len());
        let (b0, b1) = (s.bounds().unwrap(), c.bounds().unwrap());
        assert!(b1.min[0] >= b0.min[0] && b1.max[0] <= b0.max[0]);
    }

    #[test]
    fn region_keeps_only_the_triangles_inside() {
        let s = grid_scene(10);
        let r = extract_region(&s, Bounds { min: [0.0, 0.0, -1.0], max: [5.0, 5.0, 1.0] });
        assert_eq!(r.triangle_count(), 50);
        let p = &r.meshes[0].primitives[0];
        assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
        assert!(p.positions.len() < s.meshes[0].primitives[0].positions.len(), "unused vertices are dropped");
        assert!(r.report.items.iter().any(|i| i.path == "region"));
        assert_eq!(extract_region(&s, Bounds { min: [50.0; 3], max: [60.0; 3] }).triangle_count(), 0);
    }

    #[test]
    fn hostile_indices_and_empty_scenes_do_not_panic() {
        let mut s = grid_scene(2);
        s.meshes[0].primitives[0].indices.push(9999);
        s.meshes[0].primitives[0].indices.extend_from_slice(&[0, 1]);
        let _ = cluster(&s, 4);
        let _ = extract_region(&s, Bounds { min: [f32::NAN; 3], max: [1.0; 3] });
        let empty = Scene::empty(Format::Stl, Units::Unspecified, 10);
        let p = plan(&empty, Frame { memory_bytes: 0, max_triangles: Some(0) });
        assert!(p.fits && !p.needs_approval);
        assert_eq!(cluster(&empty, 4).triangle_count(), 0);
    }
}
