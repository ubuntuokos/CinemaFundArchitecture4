//! Scene writers (CFA4-OPENUSD-3D-EXPORT-001, phase U3a): glTF 2.0 (`.glb`, or `.gltf` + `.bin`),
//! Wavefront OBJ (+ MTL) and STL (binary or ASCII), from the [`Scene`] model the readers produce.
//!
//! Like the readers, the writers do no file I/O: [`export`] returns the bytes of every file
//! ([`ExportFile`]) and a *loss report* ([`Export::report`]) that names everything the target format
//! cannot carry (export rule 1: no silent loss; the caller shows it before anything is written). The
//! output is deterministic (same scene and options, same bytes: no clock, no random numbers, sorted
//! JSON keys; rule 4). The scene is validated first: an inconsistent [`Scene`] (an index out of range,
//! a cycle, a NaN) is an [`Error::Invalid`], never a panic (U3a-A3). The output size is estimated against
//! [`ExportOptions::max_memory_bytes`] before anything is allocated (U3a-A9).
//!
//! Source asset metadata survives: the glTF `asset.copyright` is kept (a comment line in OBJ), and the
//! `generator` is the one the caller names (rule 3). Units and axes are never converted: the numbers are
//! written as they are, and the report says so where the source and target conventions differ (rule 5).

use crate::util::Budget;
use crate::{Error, Format, Report, Resolver, Result, Scene, Transform, Units, invalid};

/// What to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// One self-contained binary glTF (`.glb`): the default.
    Glb,
    /// glTF JSON plus an external `.bin` buffer.
    Gltf,
    /// Wavefront OBJ, with an MTL file when the scene has materials.
    Obj,
    /// Binary STL.
    StlBinary,
    /// ASCII STL.
    StlAscii,
}

impl Target {
    /// The main file's extension.
    pub fn extension(self) -> &'static str {
        match self {
            Target::Glb => "glb",
            Target::Gltf => "gltf",
            Target::Obj => "obj",
            Target::StlBinary | Target::StlAscii => "stl",
        }
    }

    /// The file format family.
    pub fn format(self) -> Format {
        match self {
            Target::Glb | Target::Gltf => Format::Gltf,
            Target::Obj => Format::Obj,
            Target::StlBinary | Target::StlAscii => Format::Stl,
        }
    }

    /// `glb`, `gltf`, `obj`, `stl` (binary) or `stl-ascii`, case-insensitive.
    pub fn from_name(name: &str) -> Option<Target> {
        match name.trim().to_ascii_lowercase().as_str() {
            "glb" => Some(Target::Glb),
            "gltf" => Some(Target::Gltf),
            "obj" => Some(Target::Obj),
            "stl" | "stl-binary" => Some(Target::StlBinary),
            "stl-ascii" => Some(Target::StlAscii),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Target::Glb => "glb",
            Target::Gltf => "gltf",
            Target::Obj => "obj",
            Target::StlBinary => "stl",
            Target::StlAscii => "stl-ascii",
        }
    }
}

/// Options of one export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub target: Target,
    /// File name without extension; made safe by [`sanitize_stem`].
    pub stem: String,
    /// Written into the file (glTF `asset.generator`, an OBJ comment): the program's name and version.
    pub generator: String,
    /// `.glb` only: put the texture images into the file (needs the [`Resolver`] to supply them);
    /// otherwise the images stay references next to the file (export decision E-5).
    pub embed_textures: bool,
    /// Budget for the bytes the export builds (checked against an estimate before allocating).
    pub max_memory_bytes: usize,
}

impl ExportOptions {
    pub fn new(target: Target, stem: &str) -> ExportOptions {
        ExportOptions {
            target,
            stem: stem.to_string(),
            generator: format!("CFA4 cfa4-model3d {}", env!("CARGO_PKG_VERSION")),
            embed_textures: false,
            max_memory_bytes: 1 << 30,
        }
    }
}

/// One file of the export, named relative to the output folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// The result of [`export`].
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    /// The main file first, then the `.bin` or `.mtl`.
    pub files: Vec<ExportFile>,
    /// What the target format cannot carry (the loss report; empty items = nothing lost).
    pub report: Report,
    /// Relative paths of texture images the files reference but do not contain; the caller puts them
    /// next to the output (or reports them missing).
    pub textures: Vec<String>,
}

/// Turns `stem` into a file name stem: letters, digits, `-`, `_`, `.` and spaces survive, everything else
/// (path separators, control characters, `:`) becomes `_`; no leading dot or space, at most 100
/// characters, never empty (`scene`).
pub fn sanitize_stem(stem: &str) -> String {
    let s: String = stem.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' }).take(100).collect();
    let s = s.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if s.is_empty() { "scene".into() } else { s }
}

/// Writes `scene` in the target format. `resolver` is asked for texture images only when
/// `opts.embed_textures` is set on a `.glb`.
pub fn export(scene: &Scene, opts: &ExportOptions, resolver: &mut dyn Resolver) -> Result<Export> {
    validate(scene)?;
    let stem = sanitize_stem(&opts.stem);
    let mut report = Report::with_cap(1_000);
    if !scene.report.is_clean() {
        report.approximated(
            "source",
            format!(
                "the import reported {} element(s) the reader did not carry; they are not in the scene and so not in the exported file",
                scene.report.items.len() + scene.report.dropped
            ),
        );
    }
    if opts.embed_textures && opts.target != Target::Glb {
        report.approximated("embed_textures", "texture images are embedded only in .glb; here they stay references next to the file");
    }
    let mut budget = Budget::new(opts.max_memory_bytes);
    let mut textures = Vec::new();
    let files = match opts.target {
        Target::Glb | Target::Gltf => crate::write_gltf::write(scene, opts, &stem, resolver, &mut budget, &mut report, &mut textures)?,
        Target::Obj => crate::write_obj::write(scene, opts, &stem, &mut budget, &mut report, &mut textures)?,
        Target::StlBinary | Target::StlAscii => crate::write_stl::write(scene, opts, &stem, &mut budget, &mut report)?,
    };
    textures.sort();
    textures.dedup();
    Ok(Export { files, report, textures })
}

// ------------------------------------------------------------------------------------ validation

fn finite(v: &[f32]) -> bool {
    v.iter().all(|f| f.is_finite())
}

/// Checks the scene is consistent enough to write: indices in range, no cycle, finite numbers.
fn validate(scene: &Scene) -> Result<()> {
    let n = scene.nodes.len();
    let mut parents = vec![0u32; n];
    for (i, node) in scene.nodes.iter().enumerate() {
        if node.mesh.is_some_and(|m| m >= scene.meshes.len()) {
            return invalid(format!("node {i}: mesh index out of range"));
        }
        if node.camera.is_some_and(|c| c >= scene.cameras.len()) {
            return invalid(format!("node {i}: camera index out of range"));
        }
        match node.transform {
            Transform::Trs { translation, rotation, scale } => {
                if !(finite(&translation) && finite(&rotation) && finite(&scale)) {
                    return invalid(format!("node {i}: non-finite transform"));
                }
            }
            Transform::Matrix(m) => {
                if !finite(&m) {
                    return invalid(format!("node {i}: non-finite matrix"));
                }
            }
        }
        for &c in &node.children {
            let Some(p) = parents.get_mut(c) else { return invalid(format!("node {i}: child {c} out of range")) };
            *p += 1;
            if *p > 1 || c == i {
                return invalid(format!("node {c} has more than one parent or is its own child"));
            }
        }
    }
    if scene.roots.iter().any(|&r| r >= n) {
        return invalid("a scene root is out of range");
    }
    // acyclic: every node must be reachable from a parentless one
    let mut seen = 0usize;
    let mut stack: Vec<usize> = parents.iter().enumerate().filter(|(_, p)| **p == 0).map(|(i, _)| i).collect();
    while let Some(i) = stack.pop() {
        seen += 1;
        if let Some(node) = scene.nodes.get(i) {
            stack.extend(node.children.iter().copied());
        }
    }
    if seen != n {
        return invalid("the node graph has a cycle");
    }
    for (mi, mesh) in scene.meshes.iter().enumerate() {
        for (pi, p) in mesh.primitives.iter().enumerate() {
            let at = format!("meshes[{mi}].primitives[{pi}]");
            let count = p.positions.len();
            if !p.positions.iter().all(|v| finite(v)) || !p.normals.iter().all(|v| finite(v)) || !p.texcoords.iter().all(|v| finite(v)) {
                return invalid(format!("{at}: non-finite vertex data"));
            }
            if (!p.normals.is_empty() && p.normals.len() != count) || (!p.texcoords.is_empty() && p.texcoords.len() != count) {
                return invalid(format!("{at}: normals or texcoords do not match the positions"));
            }
            if p.indices.len() % 3 != 0 || p.indices.iter().any(|&i| i as usize >= count) {
                return invalid(format!("{at}: bad triangle indices"));
            }
            if p.material.is_some_and(|m| m >= scene.materials.len()) {
                return invalid(format!("{at}: material index out of range"));
            }
        }
    }
    for (i, m) in scene.materials.iter().enumerate() {
        if !(finite(&m.base_color) && finite(&m.emissive) && m.metallic.is_finite() && m.roughness.is_finite()) {
            return invalid(format!("materials[{i}]: non-finite factor"));
        }
    }
    for (i, c) in scene.cameras.iter().enumerate() {
        let ok = match c.projection {
            crate::Projection::Perspective { yfov, aspect_ratio, znear, zfar } => {
                yfov.is_finite() && znear.is_finite() && aspect_ratio.is_none_or(f32::is_finite) && zfar.is_none_or(f32::is_finite)
            }
            crate::Projection::Orthographic { xmag, ymag, znear, zfar } => finite(&[xmag, ymag, znear, zfar]),
        };
        if !ok {
            return invalid(format!("cameras[{i}]: non-finite projection"));
        }
    }
    for (ai, a) in scene.animations.iter().enumerate() {
        for (ci, ch) in a.channels.iter().enumerate() {
            let at = format!("animations[{ai}].channels[{ci}]");
            if ch.node >= n {
                return invalid(format!("{at}: node out of range"));
            }
            let comps = if ch.target == crate::TargetPath::Rotation { 4 } else { 3 };
            let per_key = if ch.interpolation == crate::Interpolation::CubicSpline { 3 * comps } else { comps };
            if ch.values.len() != ch.times.len().saturating_mul(per_key) {
                return invalid(format!("{at}: {} keys but {} values", ch.times.len(), ch.values.len()));
            }
            if !finite(&ch.times) || !finite(&ch.values) || ch.times.windows(2).any(|w| w[1] < w[0]) {
                return invalid(format!("{at}: bad keyframes"));
            }
        }
    }
    Ok(())
}

/// Charges an estimated output size against the budget (before anything is built).
pub(crate) fn charge(budget: &mut Budget, bytes: usize) -> Result<()> {
    budget.take(bytes, 1).map_err(|_| Error::Limit(format!("the export would need about {bytes} bytes, more than the memory budget")))
}

// ------------------------------------------------------------------------------------- transforms

/// Column-major 4×4, as glTF stores it.
pub(crate) type Mat = [f32; 16];

pub(crate) const IDENTITY: Mat = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut o = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0f32;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            o[c * 4 + r] = s;
        }
    }
    o
}

pub(crate) fn local_matrix(t: &Transform) -> Mat {
    match *t {
        Transform::Matrix(m) => m,
        Transform::Trs { translation, rotation, scale } => {
            let [x, y, z, w] = rotation;
            let len = (x * x + y * y + z * z + w * w).sqrt();
            let (x, y, z, w) = if len.is_finite() && len > 0.0 { (x / len, y / len, z / len, w / len) } else { (0.0, 0.0, 0.0, 1.0) };
            let r = [
                [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
                [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
                [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
            ];
            // columns: R's columns scaled, then the translation
            let mut m = IDENTITY;
            for (c, s) in scale.iter().enumerate() {
                for (row, rr) in r.iter().enumerate() {
                    m[c * 4 + row] = rr[c] * s;
                }
            }
            m[12] = translation[0];
            m[13] = translation[1];
            m[14] = translation[2];
            m
        }
    }
}

/// World matrix of every node (parent chain multiplied down from the parentless nodes).
pub(crate) fn world_matrices(scene: &Scene) -> Vec<Mat> {
    let n = scene.nodes.len();
    let mut world = vec![IDENTITY; n];
    let mut has_parent = vec![false; n];
    for node in &scene.nodes {
        for &c in &node.children {
            if let Some(h) = has_parent.get_mut(c) {
                *h = true;
            }
        }
    }
    let mut stack: Vec<usize> = (0..n).filter(|&i| !has_parent.get(i).copied().unwrap_or(false)).collect();
    for &i in &stack {
        if let (Some(w), Some(node)) = (world.get_mut(i), scene.nodes.get(i)) {
            *w = local_matrix(&node.transform);
        }
    }
    while let Some(i) = stack.pop() {
        let (Some(node), Some(&parent)) = (scene.nodes.get(i), world.get(i)) else { continue };
        for &c in &node.children {
            if let (Some(child), Some(w)) = (scene.nodes.get(c), world.get_mut(c)) {
                *w = mul(&parent, &local_matrix(&child.transform));
                stack.push(c);
            }
        }
    }
    world
}

pub(crate) fn is_identity(m: &Mat) -> bool {
    m.iter().zip(IDENTITY.iter()).all(|(a, b)| a == b)
}

pub(crate) fn transform_point(m: &Mat, p: [f32; 3]) -> [f32; 3] {
    [m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12], m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13], m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]]
}

/// Determinant of the upper-left 3×3 (its sign says whether the transform mirrors).
pub(crate) fn det3(m: &Mat) -> f32 {
    m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2]) + m[8] * (m[1] * m[6] - m[5] * m[2])
}

/// A normal under `m`: the cofactor matrix of the 3×3 (the inverse transpose up to a positive factor),
/// normalised; a degenerate result keeps the input.
pub(crate) fn transform_normal(m: &Mat, n: [f32; 3]) -> [f32; 3] {
    let c = [
        [m[5] * m[10] - m[9] * m[6], m[8] * m[6] - m[4] * m[10], m[4] * m[9] - m[8] * m[5]],
        [m[9] * m[2] - m[1] * m[10], m[0] * m[10] - m[8] * m[2], m[8] * m[1] - m[0] * m[9]],
        [m[1] * m[6] - m[5] * m[2], m[4] * m[2] - m[0] * m[6], m[0] * m[5] - m[4] * m[1]],
    ];
    // c[col][row]: the cofactor of element (row, col) is stored so that out = Σ c[k][row] * n[k]
    let o =
        [c[0][0] * n[0] + c[1][0] * n[1] + c[2][0] * n[2], c[0][1] * n[0] + c[1][1] * n[1] + c[2][1] * n[2], c[0][2] * n[0] + c[1][2] * n[1] + c[2][2] * n[2]];
    let sign = if det3(m) < 0.0 { -1.0 } else { 1.0 };
    let o = [o[0] * sign, o[1] * sign, o[2] * sign];
    let len = (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt();
    if len.is_finite() && len > 0.0 { [o[0] / len, o[1] / len, o[2] / len] } else { n }
}

/// One mesh instance of the scene: the node that places it, its mesh and its world matrix.
pub(crate) struct Instance {
    pub node: usize,
    pub mesh: usize,
    pub world: Mat,
}

pub(crate) fn instances(scene: &Scene) -> Vec<Instance> {
    let world = world_matrices(scene);
    scene.nodes.iter().enumerate().filter_map(|(i, n)| Some(Instance { node: i, mesh: n.mesh?, world: world.get(i).copied().unwrap_or(IDENTITY) })).collect()
}

/// Items of the loss report that every flat target (OBJ, STL) shares: hierarchy, cameras, animation,
/// nodes without a mesh, baked transforms, and the units note. `what` names the format.
pub(crate) fn flat_target_report(scene: &Scene, what: &str, inst: &[Instance], report: &mut Report) {
    if scene.nodes.iter().any(|n| !n.children.is_empty()) {
        report.unsupported("nodes", format!("{what} has no node hierarchy: the nodes are written flat, with their world transforms applied to the vertices"));
    }
    let baked = inst.iter().filter(|i| !is_identity(&i.world)).count();
    if baked > 0 {
        report.approximated("nodes", format!("{baked} mesh instance(s) have a transform that is baked into the vertices ({what} has no node transforms)"));
    }
    let empty = scene.nodes.iter().filter(|n| n.mesh.is_none() && n.camera.is_none()).count();
    if empty > 0 {
        report.unsupported("nodes", format!("{empty} node(s) carry no mesh and are not written (names and transforms of groups are lost)"));
    }
    if !scene.cameras.is_empty() {
        report.unsupported("cameras", format!("{} camera(s) are not written ({what} has no cameras)", scene.cameras.len()));
    }
    if scene.animations.iter().any(|a| !a.channels.is_empty()) {
        report.unsupported("animations", format!("{what} has no animation: the keyframes are not written"));
    }
    if scene.units == Units::Meters {
        report.approximated("units", format!("the scene is in metres, Y up; {what} states no unit or axis: the numbers are written unchanged"));
    }
}
