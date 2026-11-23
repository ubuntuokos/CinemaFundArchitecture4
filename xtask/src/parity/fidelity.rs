//! Fidelity tiers (docs/plans/CFA4-PARITY-G7, R4–R6): how faithful a present item is.
//!
//! A label match says an item *exists*; it says nothing about whether it behaves like the reference
//! (that is gap G1, behaviour tests). The tier is a separate, committed annotation, keyed by our own
//! item id in `xtask/parity/fidelity.json`:
//!
//! ```json
//! {"schema": 1, "items": [
//!   {"area": "effects", "id": "warp-stabilizer", "tier": "approximate", "note": "2-D only (docs/gaps.md)"},
//!   {"area": "commands", "id": "edit.undo", "tier": "exact", "evidence": "crates/engine tests::undo_roundtrip"}
//! ]}
//! ```
//!
//! An item with no annotation is *unrated*, never `exact` (VISION.md 2.5: evidence, not a claim):
//! `exact` needs `evidence`, `approximate` and `stub` need a `note`. Panels that are only a
//! placeholder dock, or only listed in the Window menu, are automatic stubs. The file is read as
//! hostile input, like the reference.

use super::*;

/// The committed fidelity file, from the repository root.
pub(super) const FIDELITY_FILE: &str = "xtask/parity/fidelity.json";
/// Longest note or evidence text (bytes).
const MAX_TEXT_BYTES: usize = 1024;
/// The area names an entry may carry.
const AREAS: [Area; 6] = [Area::Menus, Area::MenusEgui, Area::Commands, Area::Panels, Area::Effects, Area::Transitions];

/// How faithful a present item is. `Unrated` is the default: no evidence, no claim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tier {
    #[default]
    Unrated,
    Exact,
    Approximate,
    Stub,
}

impl Tier {
    pub(super) const ALL: [Tier; 4] = [Tier::Exact, Tier::Approximate, Tier::Stub, Tier::Unrated];

    pub(super) fn name(self) -> &'static str {
        match self {
            Tier::Unrated => "unrated",
            Tier::Exact => "exact",
            Tier::Approximate => "approximate",
            Tier::Stub => "stub",
        }
    }

    pub(super) fn index(self) -> usize {
        Tier::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// A tier an entry may state; `unrated` is the absence of an entry, not a value.
    fn parse(s: &str) -> Option<Tier> {
        match s {
            "exact" => Some(Tier::Exact),
            "approximate" => Some(Tier::Approximate),
            "stub" => Some(Tier::Stub),
            _ => None,
        }
    }
}

/// One annotation: area, our item id, its tier and why.
#[derive(Debug, PartialEq)]
pub(super) struct Entry {
    pub(super) area: Area,
    pub(super) id: String,
    pub(super) tier: Tier,
    pub(super) evidence: String,
    pub(super) note: String,
}

/// The parsed fidelity file.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Fidelity {
    pub(super) entries: Vec<Entry>,
}

fn text_field(what: &str, o: &serde_json::Map<String, Value>, key: &str) -> Result<String, String> {
    match o.get(key) {
        None => Ok(String::new()),
        Some(Value::String(s)) => {
            if s.len() > MAX_TEXT_BYTES {
                return Err(format!("{what}: {key} is longer than {MAX_TEXT_BYTES} bytes"));
            }
            plain_text(&format!("{what}: {key}"), s)?;
            Ok(s.trim().to_string())
        }
        Some(_) => Err(format!("{what}: {key} is not a string")),
    }
}

/// Parses a fidelity file; any deviation from the fixed schema is an error.
pub(super) fn parse(v: &Value) -> Result<Fidelity, String> {
    let o = v.as_object().ok_or("fidelity: not an object")?;
    if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "schema" | "items")) {
        return Err(format!("fidelity: unknown field {k:?}"));
    }
    if o.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err("fidelity: schema must be the number 1".into());
    }
    let list = o.get("items").and_then(Value::as_array).ok_or("fidelity: items is missing or not an array")?;
    if list.len() > MAX_ITEMS {
        return Err(format!("fidelity: more than {MAX_ITEMS} items"));
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(list.len());
    for (i, e) in list.iter().enumerate() {
        let what = format!("fidelity items[{i}]");
        let it = e.as_object().ok_or_else(|| format!("{what}: not an object"))?;
        if let Some(k) = it.keys().find(|k| !matches!(k.as_str(), "area" | "id" | "tier" | "evidence" | "note")) {
            return Err(format!("{what}: unknown field {k:?}"));
        }
        let str_of = |key: &str| match it.get(key) {
            Some(Value::String(s)) => Ok(s.as_str()),
            Some(_) => Err(format!("{what}: {key} is not a string")),
            None => Err(format!("{what}: {key} is missing")),
        };
        let area_name = str_of("area")?;
        let area = AREAS.iter().copied().find(|a| a.name() == area_name).ok_or_else(|| format!("{what}: unknown area {area_name:?}"))?;
        let id = str_of("id")?.trim().to_string();
        if id.is_empty() || id.len() > MAX_PATH_BYTES {
            return Err(format!("{what}: id is empty or too long"));
        }
        plain_text(&format!("{what}: id"), &id)?;
        let tier_name = str_of("tier")?;
        let tier =
            Tier::parse(tier_name).ok_or_else(|| format!("{what}: tier {tier_name:?} is not exact, approximate or stub (an item with no entry is unrated)"))?;
        let evidence = text_field(&what, it, "evidence")?;
        let note = text_field(&what, it, "note")?;
        if tier == Tier::Exact && evidence.is_empty() {
            return Err(format!("{what}: {id} is exact but has no evidence (a test name or a document)"));
        }
        if tier != Tier::Exact && note.is_empty() {
            return Err(format!("{what}: {id} is {} but has no note", tier.name()));
        }
        if !seen.insert((area.name(), id.clone())) {
            return Err(format!("{what}: {} {id} is annotated twice", area.name()));
        }
        entries.push(Entry { area, id, tier, evidence, note });
    }
    Ok(Fidelity { entries })
}

impl Fidelity {
    /// Sets the tier of our items of `area` from the annotations. An id that is none of our items
    /// is an error (a stale annotation must not pass silently), and so is an annotation that
    /// contradicts an automatic stub.
    pub(super) fn apply(&self, area: Area, ours: &mut [OurItem]) -> Result<(), String> {
        for e in self.entries.iter().filter(|e| e.area == area) {
            let mut found = false;
            for it in ours.iter_mut().filter(|it| it.id == e.id) {
                found = true;
                if it.tier == Tier::Stub && e.tier != Tier::Stub {
                    return Err(format!("fidelity: {} {} is {} but {} is an automatic stub ({})", area.name(), e.id, e.tier.name(), show(&it.path), it.note));
                }
                it.tier = e.tier;
                it.evidence = e.evidence.clone();
                if !e.note.is_empty() {
                    it.note = e.note.clone();
                }
            }
            if !found {
                return Err(format!("fidelity: {} id {:?} is not one of our items (stale annotation?)", area.name(), e.id));
            }
        }
        Ok(())
    }
}

/// Loads the fidelity annotations: `explicit` (`--fidelity FILE`) or the committed file, if any.
pub(super) fn load(root: &Path, explicit: Option<&Path>) -> Result<Fidelity, String> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => {
            let p = root.join(FIDELITY_FILE);
            if !p.exists() {
                return Ok(Fidelity::default());
            }
            p
        }
    };
    let text = read_bounded(&path)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&v).map_err(|e| format!("{}: {e}", path.display()))
}
