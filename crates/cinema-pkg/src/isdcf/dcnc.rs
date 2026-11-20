//! ISDCF Digital Cinema Naming Convention (DCNC): the content title text of a DCP.
//!
//! Twelve fields separated by underscores, each field's parts by hyphens:
//!
//! ```text
//! Title_FTR-2-3D_S-266_EN-FR-CCAP_US-13_51-HI-VI_4K_STU_20261115_FAC_SMPTE-3D_VF-1
//! title type      aspect lang       terr  audio   res studio date  facility standard package
//! ```
//!
//! Field order and rules follow the ISDCF registry pages (<https://www.isdcf.com/registry/>,
//! read 2026-11-15: illustrated guide, general tips, content types and modifiers, aspect ratios,
//! languages, captions, territories, ratings, audio, studios, facilities, standard, package types).
//! [`check`] reports deviations as [`Finding`]s; [`Name`] parses and writes names.

use std::fmt;

use chrono::NaiveDate;

use super::{Lookup, Registry};

/// Number of fields in a name.
pub const FIELDS: usize = 12;

/// Caption markers that end the language field.
pub const CAPTIONS: [&str; 3] = ["CCAP", "OCAP", "OPNCAP"];

/// A deviation from the convention. Always a warning: the convention is not a standard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Stable identifier, `DCNC.<WHAT>`.
    pub code: &'static str,
    /// The field it concerns (1-based), 0 for the whole name.
    pub field: usize,
    /// What is wrong, in English.
    pub message: String,
}

impl Finding {
    fn new(code: &'static str, field: usize, message: impl Into<String>) -> Self {
        Self { code, field, message: message.into() }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field == 0 { write!(f, "{}: {}", self.code, self.message) } else { write!(f, "{} (field {}): {}", self.code, self.field, self.message) }
    }
}

/// Content type field: `FTR-2-Temp-3D-48`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentType {
    /// `FTR`, `TLR`…
    pub kind: String,
    /// Version number, when given.
    pub version: Option<u32>,
    /// Modifiers after the version, as written.
    pub modifiers: Vec<String>,
}

/// Projector aspect ratio field: `F`, `S-266`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aspect {
    /// `F`, `S` or `C`.
    pub container: String,
    /// Image aspect ratio inside the container, three digits (`133`), when given.
    pub image: Option<String>,
}

/// Language field: `EN-FR-CCAP`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    /// Audio language, `XX` for none. Lower case means burned-in subtitles.
    pub audio: String,
    /// Subtitle languages, `XX` for none.
    pub subtitles: Vec<String>,
    /// `CCAP`, `OCAP` or `OPNCAP`.
    pub captions: Option<String>,
}

/// Standard field: `SMPTE`, `IOP-3D`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standard {
    /// `SMPTE` or `IOP`.
    pub family: String,
    /// Stereoscopic package.
    pub stereo: bool,
}

/// Package type field: `OV`, `VF`, `VF-1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// `OV` or `VF`.
    pub kind: String,
    /// For a VF, the feature version whose OV it needs.
    pub version: Option<u32>,
}

/// A name split into its fields. [`Name::parse`] then `to_string` gives the input back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// Film title, words in initial capitals, no separators.
    pub title: String,
    /// Content type, version, modifiers.
    pub content: ContentType,
    /// Projector and image aspect ratio.
    pub aspect: Aspect,
    /// Audio, subtitle and caption languages.
    pub language: Language,
    /// Release territory, then rating and further parts (`US-13`, `US-R-INT-TD`).
    pub territory: Vec<String>,
    /// Audio configuration and modifiers (`51-HI-VI`, `IAB-HI-VI-DBOX`).
    pub audio: Vec<String>,
    /// `2K` or `4K`.
    pub resolution: String,
    /// Studio code.
    pub studio: String,
    /// Creation date as written, `YYYYMMDD`.
    pub date: String,
    /// Facility code.
    pub facility: String,
    /// `SMPTE` / `IOP`, `-3D`.
    pub standard: Standard,
    /// `OV` / `VF`.
    pub package: Package,
}

fn parts(field: &str) -> Vec<String> {
    field.split('-').map(str::to_string).collect()
}

fn number(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

impl Name {
    /// Split `text` into fields. Fails only on the structure: not twelve fields, or an empty one.
    pub fn parse(text: &str) -> Result<Self, Finding> {
        let f: Vec<&str> = text.split('_').collect();
        if f.len() != FIELDS {
            return Err(Finding::new("DCNC.FIELDS", 0, format!("{} underscore-separated fields, the convention has {FIELDS}", f.len())));
        }
        if let Some(i) = f.iter().position(|x| x.is_empty()) {
            return Err(Finding::new("DCNC.EMPTY", i + 1, "empty field; every field needs an entry"));
        }
        let content = {
            let p = parts(f[1]);
            let version = p.get(1).and_then(|v| number(v));
            let skip = if version.is_some() { 2 } else { 1 };
            ContentType { kind: p[0].clone(), version, modifiers: p[skip.min(p.len())..].to_vec() }
        };
        let aspect = {
            let p = parts(f[2]);
            Aspect { container: p[0].clone(), image: (p.len() > 1).then(|| p[1..].join("-")) }
        };
        let language = {
            let mut p = parts(f[3]);
            let captions = p.last().filter(|c| p.len() > 1 && CAPTIONS.iter().any(|k| k.eq_ignore_ascii_case(c))).cloned();
            if captions.is_some() {
                p.pop();
            }
            Language { audio: p[0].clone(), subtitles: p[1..].to_vec(), captions }
        };
        let standard = {
            let p = parts(f[10]);
            Standard { family: p[0].clone(), stereo: p.len() > 1 && p[1..].iter().any(|x| x.eq_ignore_ascii_case("3D")) }
        };
        let package = {
            let p = parts(f[11]);
            Package { kind: p[0].clone(), version: p.get(1).and_then(|v| number(v)) }
        };
        Ok(Self {
            title: f[0].to_string(),
            content,
            aspect,
            language,
            territory: parts(f[4]),
            audio: parts(f[5]),
            resolution: f[6].to_string(),
            studio: f[7].to_string(),
            date: f[8].to_string(),
            facility: f[9].to_string(),
            standard,
            package,
        })
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut content = vec![self.content.kind.clone()];
        content.extend(self.content.version.map(|v| v.to_string()));
        content.extend(self.content.modifiers.iter().cloned());
        let aspect = match &self.aspect.image {
            Some(i) => format!("{}-{i}", self.aspect.container),
            None => self.aspect.container.clone(),
        };
        let mut lang = vec![self.language.audio.clone()];
        lang.extend(self.language.subtitles.iter().cloned());
        lang.extend(self.language.captions.iter().cloned());
        let standard = if self.standard.stereo { format!("{}-3D", self.standard.family) } else { self.standard.family.clone() };
        let package = match self.package.version {
            Some(v) => format!("{}-{v}", self.package.kind),
            None => self.package.kind.clone(),
        };
        let fields = [
            self.title.clone(),
            content.join("-"),
            aspect,
            lang.join("-"),
            self.territory.join("-"),
            self.audio.join("-"),
            self.resolution.clone(),
            self.studio.clone(),
            self.date.clone(),
            self.facility.clone(),
            standard,
            package,
        ];
        f.write_str(&fields.join("_"))
    }
}

/// Recognised modifier: its position in the registry's order, if it is one the registry lists
/// literally or by pattern (`[Luminance]fl`, `[FPS]`). Free chain / theatre names are `None`.
fn modifier_rank(reg: &Registry, m: &str) -> Option<usize> {
    let mods = &reg.content_modifiers;
    if let Some(i) = mods.iter().position(|e| !e.code.starts_with('[') && e.code.eq_ignore_ascii_case(m)) {
        return Some(i);
    }
    let lower = m.to_ascii_lowercase();
    if let Some(n) = lower.strip_suffix("fl")
        && number(n).is_some()
    {
        return mods.iter().position(|e| e.code.eq_ignore_ascii_case("[Luminance]fl"));
    }
    if number(m).is_some() {
        return mods.iter().position(|e| e.code.eq_ignore_ascii_case("[FPS]"));
    }
    None
}

fn obsolete(out: &mut Vec<Finding>, field: usize, what: &str, code: &str, use_instead: Option<String>) {
    let hint = use_instead.map(|c| format!("; use {c}")).unwrap_or_default();
    out.push(Finding::new("DCNC.OBSOLETE", field, format!("{what} {code} is obsolete{hint}")));
}

fn lookup(out: &mut Vec<Finding>, field: usize, what: &str, list: &[super::Entry], code: &str, unknown: &'static str) {
    match Registry::lookup(list, code) {
        Lookup::Current(_) => {}
        Lookup::Obsolete { use_instead, .. } => obsolete(out, field, what, code, use_instead),
        Lookup::Unknown => out.push(Finding::new(unknown, field, format!("{what} {code} is not in the ISDCF registry"))),
    }
}

/// Check `text` against the convention and the registry snapshot. An empty result means no finding.
pub fn check(text: &str, reg: &Registry) -> Vec<Finding> {
    let n = match Name::parse(text) {
        Ok(n) => n,
        Err(f) => return vec![f],
    };
    let mut out = Vec::new();

    // 1 title
    if n.title.contains('-') || n.title.contains(' ') {
        out.push(Finding::new("DCNC.TITLE", 1, "title words are joined in initial capitals, without hyphens or spaces"));
    }
    if !n.title.is_ascii() {
        out.push(Finding::new("DCNC.TITLE", 1, "title has non-ASCII characters; servers and file systems may not show them"));
    }

    // 2 content type
    lookup(&mut out, 2, "content type", &reg.content_types, &n.content.kind, "DCNC.CONTENT_TYPE");
    let head = n.content.kind.len() + n.content.version.map_or(0, |v| 1 + v.to_string().len());
    if head > 7 {
        out.push(Finding::new("DCNC.CONTENT_TYPE", 2, "content type and version are longer than 7 characters"));
    }
    let ranks: Vec<(usize, &String)> = n.content.modifiers.iter().filter_map(|m| modifier_rank(reg, m).map(|r| (r, m))).collect();
    if let Some(w) = ranks.windows(2).find(|w| w[1].0 < w[0].0) {
        out.push(Finding::new("DCNC.MODIFIER_ORDER", 2, format!("modifier {} comes before {} in the registry order", w[1].1, w[0].1)));
    }
    let three_d = n.content.modifiers.iter().any(|m| m.eq_ignore_ascii_case("3D"));

    // 3 aspect ratio
    lookup(&mut out, 3, "projector aspect ratio", &reg.aspect_ratios, &n.aspect.container, "DCNC.ASPECT");
    if let Some(img) = &n.aspect.image {
        if !(img.len() == 3 && img.bytes().all(|b| b.is_ascii_digit())) {
            out.push(Finding::new("DCNC.ASPECT", 3, format!("image aspect ratio {img} is not three digits (133 for 1.33:1)")));
        }
        if ["TLR", "TSR"].iter().any(|k| n.content.kind.eq_ignore_ascii_case(k)) {
            out.push(Finding::new("DCNC.ASPECT", 3, "trailers do not carry an image aspect ratio"));
        }
    }

    // 4 language
    for (i, code) in std::iter::once(&n.language.audio).chain(&n.language.subtitles).enumerate() {
        if code.eq_ignore_ascii_case("XX") {
            continue;
        }
        let what = if i == 0 { "audio language" } else { "subtitle language" };
        lookup(&mut out, 4, what, &reg.languages, code, "DCNC.LANGUAGE");
    }
    if n.language.subtitles.is_empty() {
        out.push(Finding::new("DCNC.LANGUAGE", 4, "no subtitle language; write XX for none"));
    }

    // 5 territory and rating
    let terr = &n.territory[0];
    if terr.eq_ignore_ascii_case("XX") {
        out.push(Finding::new("DCNC.TERRITORY", 5, "XX is not a territory; name the release territory, or INT"));
    } else {
        lookup(&mut out, 5, "territory", &reg.territories, terr, "DCNC.TERRITORY");
        if terr.bytes().any(|b| b.is_ascii_lowercase()) {
            out.push(Finding::new("DCNC.TERRITORY", 5, "territory codes are upper case"));
        }
    }
    if n.territory.iter().any(|p| p.eq_ignore_ascii_case("XX")) && !terr.eq_ignore_ascii_case("XX") {
        out.push(Finding::new("DCNC.TERRITORY", 5, "XX does not mean \"no rating\"; leave the rating out"));
    }

    // 6 audio
    for code in &n.audio {
        match Registry::lookup(&reg.audio, code) {
            Lookup::Current(e) if e.code == "ATMOS" => {
                out.push(Finding::new("DCNC.OBSOLETE", 6, "ATMOS is deprecated; use IAB"));
            }
            Lookup::Current(_) => {}
            Lookup::Obsolete { use_instead, .. } => obsolete(&mut out, 6, "audio code", code, use_instead),
            Lookup::Unknown => out.push(Finding::new("DCNC.AUDIO", 6, format!("audio code {code} is not in the ISDCF registry"))),
        }
    }

    // 7 resolution
    if !["2K", "4K"].contains(&n.resolution.as_str()) {
        out.push(Finding::new("DCNC.RESOLUTION", 7, format!("resolution {} is not 2K or 4K", n.resolution)));
    }

    // 8 studio, 10 facility
    if !n.studio.eq_ignore_ascii_case("NULL") {
        lookup(&mut out, 8, "studio", &reg.studios, &n.studio, "DCNC.STUDIO");
    }
    if !(n.facility.eq_ignore_ascii_case("NUL") || n.facility.eq_ignore_ascii_case("NULL")) {
        lookup(&mut out, 10, "facility", &reg.facilities, &n.facility, "DCNC.FACILITY");
    }

    // 9 date
    let date_ok = n.date.len() == 8 && n.date.bytes().all(|b| b.is_ascii_digit()) && NaiveDate::parse_from_str(&n.date, "%Y%m%d").is_ok();
    if !date_ok {
        out.push(Finding::new("DCNC.DATE", 9, format!("{} is not a date written YYYYMMDD", n.date)));
    }

    // 11 standard
    if !(n.standard.family == "SMPTE" || n.standard.family == "IOP") {
        out.push(Finding::new("DCNC.STANDARD", 11, format!("standard {} is not SMPTE or IOP", n.standard.family)));
    }
    if three_d != n.standard.stereo {
        out.push(Finding::new("DCNC.3D", 11, "3D must appear in both the content type and the standard field (SMPTE-3D, IOP-3D)"));
    }

    // 12 package
    if !(n.package.kind == "OV" || n.package.kind == "VF") {
        out.push(Finding::new("DCNC.PACKAGE", 12, format!("package type {} is not OV or VF", n.package.kind)));
    } else if n.package.kind == "OV" && n.package.version.is_some() {
        out.push(Finding::new("DCNC.PACKAGE", 12, "only a VF names the version of the OV it needs"));
    }
    out
}
