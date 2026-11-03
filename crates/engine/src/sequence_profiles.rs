//! Sequence profiles: named frame formats (size, display and pixel aspect, frame rate, scan, 360°)
//! that a sequence can be set to, and the user's own ones, edited in Sequence ▸ Sequence
//! Profiles…. CFA4's own implementation of the capability other open editors call "profiles";
//! no code or profile files from another project are used.
//!
//! - **Built-in profiles** are authored in code ([`builtin_profiles`]): common delivery formats,
//!   chosen by us. They cannot be changed or deleted; saving one under a new description makes a
//!   user profile.
//! - **User profiles** are one JSON file each in `<data dir>/sequence-profiles/`:
//!   `{"format": "cfa4.sequence-profile", "version": 1, "description": …, "width": …, …}`.
//!   The file name comes from the description ([`file_stem`]); a name already taken by another
//!   profile gets `_2`, `_3`, … ([`free_file`]).
//! - **Descriptions are unique** (case- and space-insensitive) across built-in and user profiles;
//!   a file whose description is already taken is left out of the list.
//! - **Aspects:** the display aspect is always `width × pixel aspect : height`, in lowest terms.
//!   [`display_aspect_for`] and [`pixel_aspect_for`] derive one from the other, so an editor can
//!   change either.
//! - **Saving is all-or-nothing:** the new file is written atomically before an edited profile's
//!   old file is removed, so a failed save leaves the profile as it was.
//!
//! Commands: `sequenceProfiles.list`, `sequenceProfiles.save`, `sequenceProfiles.duplicate`,
//! `sequenceProfiles.delete`, `sequenceProfiles.apply` (undoable; sets the active sequence's frame
//! size, frame rate and pixel aspect).

use std::path::{Path, PathBuf};

use cfa4_project::{checked_par, validate_frame_size};
use cfa4_time::{FrameRate, TICKS_PER_SECOND, Tick};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, has_seq, str_p};
use crate::{EngineError, Result, Session};

pub const FILE_FORMAT: &str = "cfa4.sequence-profile";
const FILE_VERSION: u32 = 1;
const PROFILES_DIR: &str = "sequence-profiles";
/// Longest description, in characters.
pub const DESCRIPTION_MAX: usize = 128;
/// Longest file stem made from a description, in bytes (ASCII only).
const STEM_MAX: usize = 64;
/// Most user profile files read from the folder.
const PROFILES_MAX: usize = 2048;
/// Largest profile file read, in bytes.
const FILE_MAX: u64 = 64 * 1024;
/// Most numbered names tried for a file or a duplicate's description.
const NUMBERED_MAX: u32 = 1000;
/// Highest frame rate a profile may have (as [`cfa4_project::SequenceSettings::validate`]).
const FPS_MAX: f64 = 1000.0;

/// A named frame format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceProfile {
    pub description: String,
    pub width: u32,
    pub height: u32,
    /// Picture aspect, `width × pixel aspect : height` in lowest terms.
    pub display_aspect: (u32, u32),
    /// Pixel width : height, in lowest terms.
    pub pixel_aspect: (u32, u32),
    /// Frames per second as numerator : denominator (`[24000, 1001]` is 23.976).
    pub fps: (u32, u32),
    /// Interlaced scan (CFA4 sequences are progressive; applying keeps the size and rate).
    #[serde(default)]
    pub interlaced: bool,
    /// 360° equirectangular (spherical) video.
    #[serde(default)]
    pub spherical: bool,
}

#[derive(Serialize, Deserialize)]
struct ProfileFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    profile: SequenceProfile,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// `(a, b)` in lowest terms, or `None` when a term is zero or does not fit `u32`.
fn reduced(a: u64, b: u64) -> Option<(u32, u32)> {
    if a == 0 || b == 0 {
        return None;
    }
    let g = gcd(a, b);
    Some((u32::try_from(a / g).ok()?, u32::try_from(b / g).ok()?))
}

/// The display aspect of a `width × height` frame with `par` pixels, in lowest terms.
pub fn display_aspect_for(width: u32, height: u32, par: (u32, u32)) -> Option<(u32, u32)> {
    reduced(u64::from(width).checked_mul(u64::from(par.0))?, u64::from(height).checked_mul(u64::from(par.1))?)
}

/// The pixel aspect that shows a `width × height` frame at display aspect `dar`, in lowest terms.
pub fn pixel_aspect_for(width: u32, height: u32, dar: (u32, u32)) -> Option<(u32, u32)> {
    reduced(u64::from(dar.0).checked_mul(u64::from(height))?, u64::from(dar.1).checked_mul(u64::from(width))?)
}

/// Descriptions compare trimmed, with runs of white space as one space, ignoring case.
pub fn description_key(description: &str) -> String {
    description.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The file stem for a description: ASCII letters and digits in lower case, every other run of
/// characters as one `_`, at most 64 bytes; `profile` when nothing is left.
pub fn file_stem(description: &str) -> String {
    let mut out = String::new();
    for c in description.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= STEM_MAX {
            break;
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() { "profile".into() } else { out.into() }
}

/// A file in `dir` for `description` that no other profile uses: `<stem>.json`, else
/// `<stem>_2.json`, `<stem>_3.json`, …. `own` (the edited profile's current file) counts as free.
pub fn free_file(dir: &Path, description: &str, own: Option<&Path>) -> std::result::Result<PathBuf, String> {
    let stem = file_stem(description);
    for n in 1..=NUMBERED_MAX {
        let name = if n == 1 { format!("{stem}.json") } else { format!("{stem}_{n}.json") };
        let path = dir.join(name);
        if own.is_some_and(|o| o == path) || !path.exists() {
            return Ok(path);
        }
    }
    Err(format!("there is no free file name for “{description}” in {}", dir.display()))
}

impl SequenceProfile {
    fn new(description: &str, width: u32, height: u32, par: (u32, u32), fps: (u32, u32)) -> Self {
        let display_aspect = display_aspect_for(width, height, par).unwrap_or((width.max(1), height.max(1)));
        Self { description: description.into(), width, height, display_aspect, pixel_aspect: par, fps, interlaced: false, spherical: false }
    }

    pub fn frame_rate(&self) -> FrameRate {
        FrameRate::new(i64::from(self.fps.0), i64::from(self.fps.1))
    }

    /// The profile tidied (description trimmed, aspects and rate in lowest terms) or why it can't
    /// be used. Profiles come from files and commands, so every value is checked.
    pub fn validated(&self) -> std::result::Result<Self, String> {
        let description = self.description.split_whitespace().collect::<Vec<_>>().join(" ");
        if description.is_empty() {
            return Err("the description must not be empty".into());
        }
        if description.chars().count() > DESCRIPTION_MAX {
            return Err(format!("the description must be at most {DESCRIPTION_MAX} characters"));
        }
        if description.chars().any(char::is_control) {
            return Err("the description must not contain control characters".into());
        }
        validate_frame_size(self.width, self.height)?;
        let pixel_aspect = checked_par(self.pixel_aspect).ok_or("the pixel aspect ratio must be positive and at most 8:1 either way")?;
        let display_aspect = display_aspect_for(self.width, self.height, pixel_aspect).ok_or("the display aspect ratio does not fit")?;
        if reduced(u64::from(self.display_aspect.0), u64::from(self.display_aspect.1)) != Some(display_aspect) {
            return Err(format!(
                "the display aspect ratio {}:{} does not match {} x {} with {}:{} pixels ({}:{})",
                self.display_aspect.0, self.display_aspect.1, self.width, self.height, pixel_aspect.0, pixel_aspect.1, display_aspect.0, display_aspect.1
            ));
        }
        let fps = reduced(u64::from(self.fps.0), u64::from(self.fps.1)).ok_or("the frame rate must be positive")?;
        let rate = f64::from(fps.0) / f64::from(fps.1);
        // one frame must last at least one tick
        if rate > FPS_MAX || i128::from(TICKS_PER_SECOND) * i128::from(fps.1) / i128::from(fps.0) < 1 {
            return Err(format!("the frame rate must be at most {FPS_MAX} fps"));
        }
        if i128::from(TICKS_PER_SECOND) * i128::from(fps.1) / i128::from(fps.0) > i128::from(Tick::MAX.0) {
            return Err("the frame rate is too low".into());
        }
        Ok(Self { description, display_aspect, pixel_aspect, fps, ..self.clone() })
    }

    /// Parse a profile file (any problem is an error, never a panic).
    pub fn parse(bytes: &[u8]) -> std::result::Result<Self, String> {
        let file: ProfileFile = serde_json::from_slice(bytes).map_err(|e| format!("not a sequence profile: {e}"))?;
        if file.format != FILE_FORMAT {
            return Err(format!("not a sequence profile (format `{}`)", file.format));
        }
        if file.version == 0 || file.version > FILE_VERSION {
            return Err(format!("sequence profile version {} is not supported", file.version));
        }
        file.profile.validated()
    }

    pub fn read(path: &Path) -> std::result::Result<Self, String> {
        let len = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
        if len > FILE_MAX {
            return Err(format!("{}: too large for a sequence profile", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn to_bytes(&self) -> std::result::Result<Vec<u8>, String> {
        let file = ProfileFile { format: FILE_FORMAT.into(), version: FILE_VERSION, profile: self.clone() };
        serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())
    }

    fn json(&self, file: Option<&Path>) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.insert("builtin".into(), json!(file.is_none()));
            o.insert("file".into(), json!(file.map(|f| f.to_string_lossy().into_owned())));
        }
        v
    }
}

/// CFA4's built-in profiles (our own list of common delivery formats).
pub fn builtin_profiles() -> Vec<SequenceProfile> {
    const SQ: (u32, u32) = (1, 1);
    const RATES: [(&str, (u32, u32)); 8] = [
        ("23.976", (24_000, 1001)),
        ("24", (24, 1)),
        ("25", (25, 1)),
        ("29.97", (30_000, 1001)),
        ("30", (30, 1)),
        ("50", (50, 1)),
        ("59.94", (60_000, 1001)),
        ("60", (60, 1)),
    ];
    let mut v = Vec::new();
    for (name, w, h) in [("HD 1080p", 1920, 1080), ("UHD 2160p", 3840, 2160)] {
        for (label, fps) in RATES {
            v.push(SequenceProfile::new(&format!("{name} {label} fps"), w, h, SQ, fps));
        }
    }
    for (label, fps) in [("25", (25, 1)), ("29.97", (30_000, 1001)), ("50", (50, 1)), ("59.94", (60_000, 1001))] {
        v.push(SequenceProfile::new(&format!("HD 720p {label} fps"), 1280, 720, SQ, fps));
    }
    v.push(SequenceProfile::new("DCI 2K Flat 24 fps", 1998, 1080, SQ, (24, 1)));
    v.push(SequenceProfile::new("DCI 2K Scope 24 fps", 2048, 858, SQ, (24, 1)));
    v.push(SequenceProfile::new("DCI 4K Full 24 fps", 4096, 2160, SQ, (24, 1)));
    v.push(SequenceProfile::new("Vertical HD 1080x1920 30 fps", 1080, 1920, SQ, (30, 1)));
    v.push(SequenceProfile::new("Square 1080x1080 30 fps", 1080, 1080, SQ, (30, 1)));
    let mut pal = SequenceProfile::new("SD PAL 16:9 interlaced 25 fps", 720, 576, (64, 45), (25, 1));
    pal.interlaced = true;
    v.push(pal);
    let mut ntsc = SequenceProfile::new("SD NTSC 16:9 interlaced 29.97 fps", 720, 480, (32, 27), (30_000, 1001));
    ntsc.interlaced = true;
    v.push(ntsc);
    let mut vr = SequenceProfile::new("360° 4K equirectangular 30 fps", 3840, 1920, SQ, (30, 1));
    vr.spherical = true;
    v.push(vr);
    let mut vr8 = SequenceProfile::new("360° 8K equirectangular 30 fps", 7680, 3840, SQ, (30, 1));
    vr8.spherical = true;
    v.push(vr8);
    v
}

impl Session {
    /// The user's sequence profiles folder (None without a data directory, e.g. tests).
    pub fn sequence_profiles_dir(&self) -> Option<PathBuf> {
        self.prefs_path.as_deref().and_then(Path::parent).map(|d| d.join(PROFILES_DIR))
    }

    /// The built-in profiles, then the user's by description with the file each is in. Files that
    /// fail [`SequenceProfile::read`] and descriptions already taken are left out.
    pub fn sequence_profiles(&self) -> Vec<(SequenceProfile, Option<PathBuf>)> {
        let mut found: Vec<(SequenceProfile, PathBuf)> = self
            .sequence_profiles_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
            .take(PROFILES_MAX)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
            .filter_map(|p| SequenceProfile::read(&p).ok().map(|profile| (profile, p)))
            .collect();
        // the same order on every system (read_dir's is not)
        found.sort_by(|a, b| description_key(&a.0.description).cmp(&description_key(&b.0.description)).then_with(|| a.1.cmp(&b.1)));
        let mut out: Vec<(SequenceProfile, Option<PathBuf>)> = builtin_profiles().into_iter().map(|p| (p, None)).collect();
        for (p, file) in found {
            let k = description_key(&p.description);
            if !out.iter().any(|(o, _)| description_key(&o.description) == k) {
                out.push((p, Some(file)));
            }
        }
        out
    }

    fn sequence_profile(&self, description: &str) -> Option<(SequenceProfile, Option<PathBuf>)> {
        let k = description_key(description);
        self.sequence_profiles().into_iter().find(|(p, _)| description_key(&p.description) == k)
    }

    /// Save `profile` and return it (tidied) with its file. `original` names the profile being
    /// edited: a user profile is replaced (its file renamed when the description changes); a
    /// built-in one is kept and `profile` becomes a new user profile. The description must not be
    /// any other profile's.
    pub fn save_sequence_profile(&self, profile: &SequenceProfile, original: Option<&str>) -> std::result::Result<(SequenceProfile, PathBuf), String> {
        let p = profile.validated()?;
        let dir = self.sequence_profiles_dir().ok_or("there is no profiles folder in this session")?;
        let own: Option<PathBuf> = match original {
            Some(o) => match self.sequence_profile(o) {
                Some((_, file)) => file,
                None => return Err(format!("there is no sequence profile “{o}”")),
            },
            None => None,
        };
        let key = description_key(&p.description);
        let taken = self.sequence_profiles().into_iter().any(|(q, file)| description_key(&q.description) == key && (file.is_none() || file != own));
        if taken {
            return Err(format!("a sequence profile is already called “{}”", p.description));
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = free_file(&dir, &p.description, own.as_deref())?;
        // the new file first: if it can't be written, the edited profile stays as it was
        cfa4_format::atomic_write(&path, &p.to_bytes()?).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(old) = own.filter(|o| *o != path)
            && let Err(e) = std::fs::remove_file(&old)
        {
            // put things back: the old file stays the profile, the new one goes
            let _ = std::fs::remove_file(&path);
            return Err(format!("{}: {e}", old.display()));
        }
        Ok((p, path))
    }

    /// Save a copy of the profile called `description` as “<description> copy” (“… copy 2”, …
    /// when taken).
    pub fn duplicate_sequence_profile(&self, description: &str) -> std::result::Result<(SequenceProfile, PathBuf), String> {
        let (src, _) = self.sequence_profile(description).ok_or_else(|| format!("there is no sequence profile “{description}”"))?;
        let base: String = src.description.chars().take(DESCRIPTION_MAX.saturating_sub(10)).collect();
        let names: Vec<String> = self.sequence_profiles().iter().map(|(p, _)| description_key(&p.description)).collect();
        for n in 1..=NUMBERED_MAX {
            let name = if n == 1 { format!("{base} copy") } else { format!("{base} copy {n}") };
            if !names.contains(&description_key(&name)) {
                return self.save_sequence_profile(&SequenceProfile { description: name, ..src }, None);
            }
        }
        Err(format!("there is no free name for a copy of “{description}”"))
    }

    /// Delete a user profile (built-in profiles can't be deleted).
    pub fn delete_sequence_profile(&self, description: &str) -> std::result::Result<(), String> {
        let (_, file) = self.sequence_profile(description).ok_or_else(|| format!("there is no sequence profile “{description}”"))?;
        let file = file.ok_or("built-in sequence profiles can't be deleted")?;
        std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))
    }
}

// ---------------------------------------------------------------- commands

type Run = fn(&mut Session, &Value) -> Result<Value>;

fn spec(
    id: &'static str,
    label: &'static str,
    params: &'static str,
    enabled: fn(&Session) -> std::result::Result<(), String>,
    run: Run,
    journal: bool,
) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal }
}

fn list_json(s: &Session) -> Value {
    let profiles: Vec<Value> = s.sequence_profiles().iter().map(|(p, file)| p.json(file.as_deref())).collect();
    json!({"dir": s.sequence_profiles_dir(), "profiles": profiles})
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(list_json(s))
}

fn need<'a>(p: &'a Value, cmd: &str, key: &str) -> Result<&'a str> {
    str_p(p, key).ok_or_else(|| bad(cmd, format!("need `{key}`")))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequenceProfiles.save";
    let raw = p.get("profile").cloned().ok_or_else(|| bad(CMD, "need `profile`"))?;
    let profile: SequenceProfile = serde_json::from_value(raw).map_err(|e| bad(CMD, format!("`profile`: {e}")))?;
    let original = match p.get("original").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_str().ok_or_else(|| bad(CMD, "`original` must be a string"))?),
        None => None,
    };
    let (saved, file) = s.save_sequence_profile(&profile, original).map_err(|e| bad(CMD, e))?;
    Ok(json!({"profile": saved.json(Some(&file)), "profiles": list_json(s)}))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequenceProfiles.duplicate";
    let (saved, file) = s.duplicate_sequence_profile(need(p, CMD, "description")?).map_err(|e| bad(CMD, e))?;
    Ok(json!({"profile": saved.json(Some(&file)), "profiles": list_json(s)}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequenceProfiles.delete";
    s.delete_sequence_profile(need(p, CMD, "description")?).map_err(|e| bad(CMD, e))?;
    Ok(list_json(s))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "sequenceProfiles.apply";
    let description = need(p, CMD, "description")?;
    let (profile, _) = s.sequence_profile(description).ok_or_else(|| bad(CMD, format!("there is no sequence profile “{description}”")))?;
    let id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let scale_motion = crate::commands::bool_p(p, "scaleMotion").unwrap_or(false);
    let rate = profile.frame_rate();
    let scaled = s.edit("Apply Sequence Profile", |pr, _| {
        let q = pr.sequence_mut(id).ok_or(EngineError::NoSequence)?;
        let old_size = (q.settings.width, q.settings.height);
        q.settings.width = profile.width;
        q.settings.height = profile.height;
        q.settings.par = profile.pixel_aspect;
        q.settings.frame_rate = rate;
        if !rate.supports_drop_frame() {
            q.settings.drop_frame = false;
        }
        q.settings.preset = profile.description.clone();
        q.settings.validate().map_err(|e| bad(CMD, e))?;
        Ok(if scale_motion { cfa4_edit::frame_size::scale_motion(q, old_size, (profile.width, profile.height)) } else { 0 })
    })?;
    // what CFA4 sequences can't be yet: said, not silently dropped
    let mut not_applied = Vec::new();
    if profile.interlaced {
        not_applied.push("interlaced");
    }
    if profile.spherical {
        not_applied.push("spherical");
    }
    Ok(json!({"profile": profile.description, "scaledClips": scaled, "notApplied": not_applied}))
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("sequenceProfiles.list", "List Sequence Profiles", "{}", always, list, false),
        spec(
            "sequenceProfiles.save",
            "Save Sequence Profile",
            r#"{"profile":{"description":str,"width":u32,"height":u32,"displayAspect":[u32,u32],"pixelAspect":[u32,u32],"fps":[u32,u32],"interlaced":bool?,"spherical":bool?},"original":str?}"#,
            always,
            save,
            true,
        ),
        spec("sequenceProfiles.duplicate", "Duplicate Sequence Profile", r#"{"description":str}"#, always, duplicate, true),
        spec("sequenceProfiles.delete", "Delete Sequence Profile", r#"{"description":str}"#, always, delete, true),
        spec("sequenceProfiles.apply", "Apply Sequence Profile", r#"{"description":str,"scaleMotion":bool=false}"#, has_seq, apply, true),
    ]
}
