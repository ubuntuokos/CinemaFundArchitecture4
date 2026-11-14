//! The device catalogue (§2): one TOML file per family or device, read from a directory, so a new
//! device needs a new file and no code change (§6.2).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{ControlType, FeatureStatus, Identification, Phase, SupportGoal, SystemPath};
use crate::{Error, Problem};

/// The `format` value every catalogue file carries.
pub const CATALOG_FORMAT: &str = "cfa4-surface-catalog";
/// The catalogue schema version this crate reads.
pub const CATALOG_VERSION: u32 = 1;

/// One catalogue file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFile {
    pub format: String,
    pub version: u32,
    /// The family the file describes (`elgato`, `mirabox-family`, …), shown as a filter.
    pub family: String,
    #[serde(default, rename = "model")]
    pub models: Vec<Model>,
}

/// One device model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// Stable id: `<vendor>.<model>` in lower case (`ajazz.akp153e`); reports refer to it.
    pub id: String,
    pub vendor: String,
    pub name: String,
    /// Device kind for the filters of the All devices tab (`lcd_keypad`, `jog_shuttle`, …).
    pub kind: String,
    pub goal: SupportGoal,
    /// The adapter family that drives it. A model whose adapter is not built is PLANNED (§3.2).
    #[serde(default)]
    pub adapter: Option<String>,
    pub phase: Phase,
    #[serde(default)]
    pub system_path: SystemPath,
    /// No route to the device exists at all (§3.2 rule 1: UNAVAILABLE instead of PLANNED).
    #[serde(default)]
    pub unavailable: bool,
    /// Connection kinds (`usb`, `bluetooth`, `network`, `midi`).
    #[serde(default)]
    pub connection: Vec<String>,
    /// Where reusable code comes from and under which licence (§2, §8.2).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub usb: Vec<UsbId>,
    #[serde(default)]
    pub controls: Vec<Control>,
    /// Feature ids (`<control id>.<feature>`) that must be VERIFIED for a VERIFIED model.
    #[serde(default)]
    pub baseline: Vec<String>,
    /// Feature ids a `cfa4-ci` report may verify (protocol-level only, §4.1).
    #[serde(default)]
    pub protocol_features: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<Blocker>,
    /// Known limits shown on the Compatibility tab.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// USB ids of a model or variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsbId {
    pub vid: u16,
    pub pid: u16,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub identification: Identification,
    /// Where the ids were read (adapter source file, a report).
    #[serde(default)]
    pub evidence: Option<String>,
}

/// A group of identical controls, such as the 15 keys of a keypad.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    /// Id within the model (`keys`, `center_wheel`); feature ids start with it.
    pub id: String,
    #[serde(rename = "type")]
    pub control_type: ControlType,
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default)]
    pub rows: Option<u32>,
    #[serde(default)]
    pub columns: Option<u32>,
    /// The adapter's declaration per feature. A feature of the type that is missing here is
    /// UNSUPPORTED; VERIFIED may not be declared (it comes from reports only).
    #[serde(default)]
    pub features: BTreeMap<String, FeatureStatus>,
}

fn one() -> u32 {
    1
}

/// An active blocking entry: the whole model (`scope = "model"`) or one feature id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    pub scope: String,
    pub reason: String,
    #[serde(default)]
    pub reference: Option<String>,
}

impl Model {
    /// Every feature id of the model with its declared status, in a stable order.
    pub fn declared_features(&self) -> BTreeMap<String, FeatureStatus> {
        let mut out = BTreeMap::new();
        for c in &self.controls {
            for f in c.control_type.features() {
                let status = c.features.get(*f).copied().unwrap_or(FeatureStatus::Unsupported);
                out.insert(format!("{}.{}", c.id, f), status);
            }
        }
        for b in &self.blockers {
            if b.scope != "model"
                && let Some(s) = out.get_mut(&b.scope)
            {
                *s = FeatureStatus::Blocked;
            }
        }
        out
    }

    /// Whether a model-wide blocker is active.
    pub fn blocked(&self) -> Option<&Blocker> {
        self.blockers.iter().find(|b| b.scope == "model")
    }

    fn check(&self, problems: &mut Vec<String>) {
        let mut ids = BTreeSet::new();
        for c in &self.controls {
            if !ids.insert(c.id.as_str()) {
                problems.push(format!("{}: control id `{}` used twice", self.id, c.id));
            }
            for (f, s) in &c.features {
                if !c.control_type.features().contains(&f.as_str()) {
                    problems.push(format!("{}: `{}` is not a feature of {}", self.id, f, c.control_type));
                }
                if *s == FeatureStatus::Verified {
                    problems.push(format!("{}: `{}.{}` declares VERIFIED; only a hardware test report can verify a feature", self.id, c.id, f));
                }
            }
        }
        let features = self.declared_features();
        for f in self.baseline.iter().chain(&self.protocol_features) {
            if !features.contains_key(f) {
                problems.push(format!("{}: baseline or protocol feature `{f}` does not exist", self.id));
            }
        }
        for b in &self.blockers {
            if b.scope != "model" && !features.contains_key(&b.scope) {
                problems.push(format!("{}: blocker scope `{}` is neither `model` nor a feature", self.id, b.scope));
            }
        }
        if self.id.is_empty() || self.id.chars().any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || "._-+".contains(c))) {
            problems.push(format!("model id `{}` must be lower-case ascii, digits and `._-+`", self.id));
        }
    }
}

/// The whole catalogue.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// The models, sorted by id.
    pub models: Vec<Model>,
    /// Model id → family, from the file that listed it.
    pub families: BTreeMap<String, String>,
}

impl Catalog {
    /// Parses one file's text. `origin` names it in errors.
    pub fn parse_file(origin: &str, text: &str) -> Result<CatalogFile, Error> {
        let file: CatalogFile = toml::from_str(text).map_err(|e| Error::Parse { origin: origin.to_owned(), message: e.to_string() })?;
        if file.format != CATALOG_FORMAT {
            return Err(Error::Parse { origin: origin.to_owned(), message: format!("format is `{}`, expected `{CATALOG_FORMAT}`", file.format) });
        }
        if file.version > CATALOG_VERSION {
            return Err(Error::Parse { origin: origin.to_owned(), message: format!("schema version {} is newer than {CATALOG_VERSION}", file.version) });
        }
        Ok(file)
    }

    /// Builds a catalogue from parsed files and checks it as a whole.
    pub fn from_files(files: Vec<(String, CatalogFile)>) -> Result<Catalog, Error> {
        let mut problems = Vec::new();
        let mut by_id: BTreeMap<String, Model> = BTreeMap::new();
        let mut families = BTreeMap::new();
        for (origin, file) in files {
            for m in file.models {
                let mut local = Vec::new();
                m.check(&mut local);
                problems.extend(local.into_iter().map(|p| Problem::new(&origin, p)));
                if by_id.contains_key(&m.id) {
                    problems.push(Problem::new(&origin, format!("model id `{}` is listed twice", m.id)));
                    continue;
                }
                families.insert(m.id.clone(), file.family.clone());
                by_id.insert(m.id.clone(), m);
            }
        }
        if !problems.is_empty() {
            return Err(Error::Invalid(problems));
        }
        Ok(Catalog { models: by_id.into_values().collect(), families })
    }

    /// Reads every `*.toml` file of a directory (not recursive), in name order.
    pub fn load_dir(dir: &Path) -> Result<Catalog, Error> {
        let mut paths = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| Error::Io { path: dir.display().to_string(), message: e.to_string() })?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io { path: dir.display().to_string(), message: e.to_string() })?;
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "toml") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut files = Vec::new();
        for path in paths {
            let origin = path.display().to_string();
            let text = std::fs::read_to_string(&path).map_err(|e| Error::Io { path: origin.clone(), message: e.to_string() })?;
            files.push((origin.clone(), Catalog::parse_file(&origin, &text)?));
        }
        Catalog::from_files(files)
    }

    /// The model with this id.
    pub fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The model a connected USB device is, if the catalogue knows its ids.
    pub fn by_usb(&self, vid: u16, pid: u16) -> Option<&Model> {
        self.models.iter().find(|m| m.usb.iter().any(|u| u.vid == vid && u.pid == pid))
    }
}
