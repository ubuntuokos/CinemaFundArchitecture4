//! 3D scene readers for the CFA4 project (CFA4-OPENUSD-3D-001, phase U1): glTF 2.0 (`.gltf`, `.glb`),
//! Wavefront OBJ (+ MTL) and STL (binary and ASCII), in pure Rust, with no file I/O and no `unsafe`.
//!
//! Every reader turns untrusted bytes into the same [`Scene`]: node hierarchy, transforms, cameras,
//! triangle meshes, keyframe animation and material references. Nothing is dropped silently: what a
//! reader cannot carry lands in [`Scene::report`] (the *unsupported report*, U1-A1). Hostile, truncated
//! or corrupt input is an [`Error`], never a panic (U1-A2); a [`Limits`] memory budget is charged before
//! every input-sized allocation (U1-A3).
//!
//! The readers never open files. External resources (a glTF `.bin`, an OBJ `.mtl`) are asked of a
//! [`Resolver`] supplied by the caller, which keeps the readers sandbox-friendly and lets the project
//! layer (`scene3d`: path + SHA-256, relink) own file access. Reader-side URI checks reject absolute
//! paths, `..` components and URI schemes before the resolver is called.
//!
//! Units and axes: glTF is metres, Y up; OBJ and STL carry no unit ([`Units::Unspecified`]).

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

mod gltf;
mod obj;
mod stl;
mod util;

#[cfg(test)]
mod tests;

pub use util::safe_relative_uri;

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Malformed, truncated or inconsistent input.
    Invalid(String),
    /// A configured limit (input size, memory budget, node count, depth) was exceeded.
    Limit(String),
    /// A feature the file *requires* that the reader cannot honour (for example a required glTF extension).
    Unsupported(String),
    /// A resource the geometry needs (for example a glTF buffer) was not supplied by the [`Resolver`].
    Missing(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(s) => write!(f, "invalid 3D file: {s}"),
            Error::Limit(s) => write!(f, "3D file exceeds a limit: {s}"),
            Error::Unsupported(s) => write!(f, "unsupported 3D file: {s}"),
            Error::Missing(s) => write!(f, "missing 3D resource: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// Source file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gltf,
    Obj,
    Stl,
}

/// Length unit of a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Meters,
    Unspecified,
}

/// Resource and size limits for one read. The defaults suit a workstation; the project layer passes the
/// ACS memory budget as `max_memory_bytes`.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Largest accepted input (and largest accepted external resource), in bytes.
    pub max_input_bytes: usize,
    /// Budget for everything the reader allocates from input-derived sizes (vertices, indices,
    /// animation keys, decoded buffers, names). Charged before allocating.
    pub max_memory_bytes: usize,
    /// Most nodes in one scene.
    pub max_nodes: usize,
    /// Deepest node nesting.
    pub max_depth: usize,
    /// Most report items kept; further ones are only counted in [`Report::dropped`].
    pub max_report_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_input_bytes: 1 << 30, max_memory_bytes: 1 << 30, max_nodes: 100_000, max_depth: 256, max_report_items: 1_000 }
    }
}

/// Supplies external resources (a glTF `.bin`, an OBJ `.mtl`) by relative URI. Returns `None` when the
/// resource is not available. The reader has already rejected absolute paths, `..` and schemes.
pub trait Resolver {
    fn resolve(&mut self, uri: &str) -> Option<Vec<u8>>;
}

impl<F: FnMut(&str) -> Option<Vec<u8>>> Resolver for F {
    fn resolve(&mut self, uri: &str) -> Option<Vec<u8>> {
        self(uri)
    }
}

/// A resolver that has no resources (self-contained files only).
pub struct NoResources;

impl Resolver for NoResources {
    fn resolve(&mut self, _uri: &str) -> Option<Vec<u8>> {
        None
    }
}

/// Source asset metadata that must survive an export (copyright, generator).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Asset {
    pub generator: Option<String>,
    pub copyright: Option<String>,
    /// The format's own version string (glTF `asset.version`).
    pub version: Option<String>,
}

/// Local transform of a node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    Trs {
        translation: [f32; 3],
        /// Quaternion `x, y, z, w`.
        rotation: [f32; 4],
        scale: [f32; 3],
    },
    /// Column-major 4×4 matrix, as stored by glTF.
    Matrix([f32; 16]),
}

impl Transform {
    pub const IDENTITY: Transform = Transform::Trs { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };
}

/// One node of the hierarchy. Indices refer to the vectors of the owning [`Scene`].
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: Option<String>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub transform: Transform,
    pub mesh: Option<usize>,
    pub camera: Option<usize>,
}

/// A triangle mesh part with one material. Always triangles (strips, fans and polygons are expanded);
/// `normals` and `texcoords` are either empty or have one entry per position.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub texcoords: Vec<[f32; 2]>,
    /// Triangle list, three indices per triangle, all `< positions.len()`.
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub name: Option<String>,
    pub primitives: Vec<Primitive>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Perspective { yfov: f32, aspect_ratio: Option<f32>, znear: f32, zfar: Option<f32> },
    Orthographic { xmag: f32, ymag: f32, znear: f32, zfar: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub name: Option<String>,
    pub projection: Projection,
}

/// A texture reference. The pixels are never read here: the project keeps the reference (path + SHA-256).
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRef {
    /// The material slot (`baseColor`, `normal`, `map_Kd`, …).
    pub slot: String,
    /// Relative URI of an external image; `None` for an embedded one.
    pub uri: Option<String>,
    /// The image is embedded in the file (data URI or glTF buffer view).
    pub embedded: bool,
    pub texcoord: u32,
}

/// A material *reference* with the plain factors the file states.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: Option<String>,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub double_sided: bool,
    pub textures: Vec<TextureRef>,
}

impl Material {
    pub(crate) fn named(name: Option<String>) -> Material {
        Material { name, base_color: [1.0; 4], metallic: 1.0, roughness: 1.0, emissive: [0.0; 3], double_sided: false, textures: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPath {
    Translation,
    Rotation,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    /// Values hold in-tangent, value, out-tangent per key (3× the components).
    CubicSpline,
}

/// Keyframes of one node property. `values` is flat: 3 floats per key for translation and scale, 4 for
/// rotation (times 3 for cubic spline).
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub target: TargetPath,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The element is not carried at all.
    Unsupported,
    /// The element is carried in an approximated or converted form.
    Approximated,
}

/// One entry of the unsupported report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportItem {
    pub severity: Severity,
    /// Where in the source (`nodes[3].skin`, `line 12: curv`, …).
    pub path: String,
    pub message: String,
}

/// What the reader could not carry over (no silent loss, U1-A1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub items: Vec<ReportItem>,
    /// Items beyond [`Limits::max_report_items`], counted but not stored.
    pub dropped: usize,
    cap: usize,
}

impl Report {
    pub(crate) fn with_cap(cap: usize) -> Report {
        Report { items: Vec::new(), dropped: 0, cap }
    }

    pub(crate) fn add(&mut self, severity: Severity, path: impl Into<String>, message: impl Into<String>) {
        if self.items.len() >= self.cap {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.items.push(ReportItem { severity, path: path.into(), message: message.into() });
        }
    }

    pub(crate) fn unsupported(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.add(Severity::Unsupported, path, message);
    }

    pub(crate) fn approximated(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.add(Severity::Approximated, path, message);
    }

    /// Nothing was lost or approximated.
    pub fn is_clean(&self) -> bool {
        self.items.is_empty() && self.dropped == 0
    }
}

/// A scene read from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub format: Format,
    pub units: Units,
    pub asset: Asset,
    pub nodes: Vec<Node>,
    /// Nodes without a parent that belong to the scene.
    pub roots: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub cameras: Vec<Camera>,
    pub materials: Vec<Material>,
    pub animations: Vec<Animation>,
    pub report: Report,
}

impl Scene {
    pub(crate) fn empty(format: Format, units: Units, cap: usize) -> Scene {
        Scene {
            format,
            units,
            asset: Asset::default(),
            nodes: Vec::new(),
            roots: Vec::new(),
            meshes: Vec::new(),
            cameras: Vec::new(),
            materials: Vec::new(),
            animations: Vec::new(),
            report: Report::with_cap(cap),
        }
    }

    /// Total vertices over all meshes.
    pub fn vertex_count(&self) -> usize {
        self.meshes.iter().flat_map(|m| &m.primitives).map(|p| p.positions.len()).sum()
    }

    /// Total triangles over all meshes.
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().flat_map(|m| &m.primitives).map(|p| p.indices.len() / 3).sum()
    }
}

/// Guess the format from the first bytes and, failing that, the file name.
pub fn detect(data: &[u8], file_name: Option<&str>) -> Option<Format> {
    if data.starts_with(b"glTF") {
        return Some(Format::Gltf);
    }
    if stl::is_binary_stl(data) {
        return Some(Format::Stl);
    }
    let ext = file_name.and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("gltf" | "glb") => return Some(Format::Gltf),
        Some("obj") => return Some(Format::Obj),
        Some("stl") => return Some(Format::Stl),
        _ => {}
    }
    let head = data.get(..data.len().min(512)).unwrap_or(data);
    let trimmed = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let first = trimmed.iter().copied().find(|b| !b.is_ascii_whitespace());
    if first == Some(b'{') {
        return Some(Format::Gltf);
    }
    if trimmed.starts_with(b"solid") {
        return Some(Format::Stl);
    }
    let text = String::from_utf8_lossy(trimmed);
    let looks_obj = text.lines().map(str::trim).any(|l| ["v ", "vn ", "vt ", "f ", "o ", "mtllib "].iter().any(|p| l.starts_with(p)));
    looks_obj.then_some(Format::Obj)
}

/// Read a scene in a known format.
pub fn read_as(format: Format, data: &[u8], resolver: &mut dyn Resolver, limits: &Limits) -> Result<Scene> {
    if data.len() > limits.max_input_bytes {
        return Err(Error::Limit(format!("input is {} bytes, the limit is {}", data.len(), limits.max_input_bytes)));
    }
    match format {
        Format::Gltf => gltf::read(data, resolver, limits),
        Format::Obj => obj::read(data, resolver, limits),
        Format::Stl => stl::read(data, limits),
    }
}

/// Detect the format and read the scene.
pub fn read(data: &[u8], file_name: Option<&str>, resolver: &mut dyn Resolver, limits: &Limits) -> Result<Scene> {
    let Some(format) = detect(data, file_name) else {
        return invalid("not a glTF, OBJ or STL file");
    };
    read_as(format, data, resolver, limits)
}
