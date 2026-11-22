//! The project's 3D-asset section (CFA4-OPENUSD-3D-001 §4, phase U1).
//!
//! There is no global 3D mode. A project gets a `scene3d` section only when 3D content is imported into
//! it, and loses it when the last asset goes: a project without 3D content saves exactly the keys it did
//! before (P-A1, P-A2). The section holds *references*, never the geometry: a file path plus the
//! SHA-256 of the file as imported, the importer that read it, the chosen route, the unsupported report
//! and the capabilities switched on per content type. Geometry is read again from the file on demand.
//!
//! Routes (§2 Dönt-2): `A` hands the file to an external backend that returns EXR/AOV (the default; CFA4
//! has no 3D interpreter on this route); `B` is the native import through `cfa4-model3d`. Switching a
//! route never touches the reference or the report (P-A3).
//!
//! Everything here is read from project files, so nothing is trusted: [`Scene3d::repair`] drops what
//! cannot be used and clips what is too long. This section is independent of the VFX-2 `VfxGraph` and
//! ARF sections.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Most assets in a project.
pub const MAX_ASSETS: usize = 1_000;
/// Longest asset name (characters).
pub const MAX_NAME_CHARS: usize = 200;
/// Longest file path (bytes).
pub const MAX_PATH_BYTES: usize = 4096;
/// Most report entries kept per asset.
pub const MAX_REPORT_ITEMS: usize = 1_000;
/// Longest report path or message (characters).
pub const MAX_REPORT_CHARS: usize = 500;

/// Source file format of an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format3d {
    Gltf,
    Obj,
    Stl,
    /// `.usd`, `.usda`, `.usdc`, `.usdz`: route A only until U2 (no native USD).
    Usd,
}

impl Format3d {
    pub fn name(self) -> &'static str {
        match self {
            Format3d::Gltf => "gltf",
            Format3d::Obj => "obj",
            Format3d::Stl => "stl",
            Format3d::Usd => "usd",
        }
    }

    /// The format a file extension stands for.
    pub fn from_extension(ext: &str) -> Option<Format3d> {
        match ext.to_ascii_lowercase().as_str() {
            "gltf" | "glb" => Some(Format3d::Gltf),
            "obj" => Some(Format3d::Obj),
            "stl" => Some(Format3d::Stl),
            "usd" | "usda" | "usdc" | "usdz" => Some(Format3d::Usd),
            _ => None,
        }
    }
}

/// How an asset is brought into the video project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    /// External EXR replacement (default).
    #[default]
    A,
    /// Native import (`cfa4-model3d`).
    B,
}

impl Route {
    /// Whether the native route can read `format` in this build.
    pub fn native_supported(format: Format3d) -> bool {
        !matches!(format, Format3d::Usd)
    }
}

/// A capability switched on for a content type by importing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// The project may reference assets of this type.
    Reference,
    /// A preview of the asset may be shown.
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportSeverity {
    /// The element is not carried.
    Unsupported,
    /// The element is carried approximated or converted.
    Approximated,
}

/// One line of the unsupported report (no silent loss).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEntry {
    pub severity: ReportSeverity,
    pub path: String,
    pub message: String,
}

/// What the importer found, for the asset panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetStats {
    pub nodes: u64,
    pub meshes: u64,
    pub vertices: u64,
    pub triangles: u64,
    pub cameras: u64,
    pub materials: u64,
    pub animations: u64,
}

/// One referenced 3D file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset3d {
    /// Project-wide id (from `Project::alloc_id`).
    pub id: u64,
    pub name: String,
    /// The source file (a reference; the project never embeds it).
    pub path: String,
    /// Lower-case hex SHA-256 of the file as imported.
    pub sha256: String,
    pub format: Format3d,
    #[serde(default)]
    pub route: Route,
    /// The importer that produced `stats` and `report` (`cfa4-model3d 1`); absent for route-A-only formats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<AssetStats>,
    /// The unsupported report, kept when the route changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub report: Vec<ReportEntry>,
    /// Report entries beyond the importer's cap, counted only.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dropped: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// The project's 3D section.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene3d {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset3d>,
    /// Capabilities per content type, switched on by importing that type and off with its last asset.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<Format3d, Vec<Capability>>,
}

fn clip(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max).collect();
    }
}

/// A SHA-256 as written here: 64 lower-case hex digits.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Scene3d {
    /// No 3D content: the section is not written.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty() && self.capabilities.is_empty()
    }

    pub fn asset(&self, id: u64) -> Option<&Asset3d> {
        self.assets.iter().find(|a| a.id == id)
    }

    pub fn asset_mut(&mut self, id: u64) -> Option<&mut Asset3d> {
        self.assets.iter_mut().find(|a| a.id == id)
    }

    /// The ids in use (they count for `Project::max_id`).
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.assets.iter().map(|a| a.id)
    }

    /// Switch capabilities on for the types present and off for the others (P-A2: only imported content
    /// turns 3D on). Call after every change to `assets`.
    pub fn refresh_capabilities(&mut self) {
        let present: Vec<Format3d> = self.assets.iter().map(|a| a.format).collect();
        self.capabilities.retain(|f, _| present.contains(f));
        for f in present {
            self.capabilities.entry(f).or_insert_with(|| vec![Capability::Reference, Capability::Preview]);
        }
    }

    /// Brings a section read from a file into range. Returns how many assets were dropped (unusable
    /// reference, duplicate id, over the cap).
    pub fn repair(&mut self) -> usize {
        let before = self.assets.len();
        let mut seen = std::collections::BTreeSet::new();
        self.assets.retain_mut(|a| {
            clip(&mut a.name, MAX_NAME_CHARS);
            a.sha256.make_ascii_lowercase();
            if a.path.trim().is_empty() || a.path.len() > MAX_PATH_BYTES || a.path.contains('\0') || !is_sha256_hex(&a.sha256) {
                return false;
            }
            if a.route == Route::B && !Route::native_supported(a.format) {
                a.route = Route::A;
            }
            a.report.truncate(MAX_REPORT_ITEMS);
            for r in &mut a.report {
                clip(&mut r.path, MAX_REPORT_CHARS);
                clip(&mut r.message, MAX_REPORT_CHARS);
            }
            seen.insert(a.id)
        });
        self.assets.truncate(MAX_ASSETS);
        self.refresh_capabilities();
        before - self.assets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: u64, format: Format3d) -> Asset3d {
        Asset3d {
            id,
            name: format!("a{id}"),
            path: format!("/x/{id}.{}", format.name()),
            sha256: "ab".repeat(32),
            format,
            route: Route::A,
            importer: None,
            stats: None,
            report: Vec::new(),
            dropped: 0,
        }
    }

    #[test]
    fn an_empty_section_writes_nothing_and_a_default_route_is_a() {
        let s = Scene3d::default();
        assert!(s.is_empty());
        assert_eq!(serde_json::to_string(&s).unwrap(), "{}");
        let a: Asset3d = serde_json::from_str(&serde_json::to_string(&asset(1, Format3d::Obj)).unwrap().replace("\"route\":\"A\",", "")).unwrap();
        assert_eq!(a.route, Route::A);
    }

    #[test]
    fn capabilities_follow_the_content() {
        let mut s = Scene3d { assets: vec![asset(1, Format3d::Gltf), asset(2, Format3d::Stl)], ..Default::default() };
        s.refresh_capabilities();
        assert_eq!(s.capabilities.len(), 2);
        s.assets.remove(0);
        s.refresh_capabilities();
        assert_eq!(s.capabilities.keys().copied().collect::<Vec<_>>(), [Format3d::Stl]);
        s.assets.clear();
        s.refresh_capabilities();
        assert!(s.is_empty(), "the last asset takes the section away");
    }

    #[test]
    fn hostile_sections_are_repaired() {
        let mut bad_hash = asset(2, Format3d::Obj);
        bad_hash.sha256 = "xyz".into();
        let mut usd_b = asset(3, Format3d::Usd);
        usd_b.route = Route::B;
        let mut long = asset(4, Format3d::Gltf);
        long.name = "n".repeat(10_000);
        long.sha256 = "AB".repeat(32);
        long.report =
            (0..MAX_REPORT_ITEMS + 5).map(|_| ReportEntry { severity: ReportSeverity::Unsupported, path: "p".repeat(5000), message: "m".into() }).collect();
        let mut s = Scene3d { assets: vec![asset(1, Format3d::Gltf), asset(1, Format3d::Obj), bad_hash, usd_b, long], ..Default::default() };
        assert_eq!(s.repair(), 2, "duplicate id and bad hash");
        assert_eq!(s.assets.len(), 3);
        assert_eq!(s.asset(3).unwrap().route, Route::A, "no native USD");
        let l = s.asset(4).unwrap();
        assert_eq!((l.name.chars().count(), l.sha256.as_str(), l.report.len()), (MAX_NAME_CHARS, "ab".repeat(32).as_str(), MAX_REPORT_ITEMS));
        assert!(l.report[0].path.chars().count() <= MAX_REPORT_CHARS);
    }
}
