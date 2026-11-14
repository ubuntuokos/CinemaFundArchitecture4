//! AMWA AS-11 descriptive metadata: the AS-11 Core framework, the AS-11 Segmentation framework
//! (programme parts on a timeline DM track) and the UK DPP framework, as DMS frameworks of the
//! material package (#96).
//!
//! The data model (set and item labels, value types, which items are required) follows the BSD
//! licensed bmx library of the BBC (`include/bmx/as11/as11_extensions_data_model.h`,
//! `uk_dpp_extensions_data_model.h`, `src/as11/AS11WriterHelper.cpp`; reimplemented in Rust, see
//! `docs/licensing.md`). Item names are bmx's short names (`SeriesTitle`, `ShimVersion`, …) and
//! values are text as in bmx's framework files: booleans `true` / `false`, versions `1.1`,
//! rationals `16/9`, dates `2011-08-01`, positions and durations as a time code
//! (`09:58:00:00`, measured from the file's start time code) or a frame count.

use crate::klv::{int_of, rational_of, utf16_of};

/// What a framework item holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemType {
    Text,
    U8,
    U16,
    Bool,
    /// `VersionType`: major and minor byte.
    Version,
    Rational,
    /// A frame position in the material package.
    Position,
    /// A length in frames.
    Length,
    /// An MXF timestamp (the date part is what AS-11 uses).
    Timestamp,
}

/// One framework item.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub name: &'static str,
    pub ul: [u8; 16],
    pub kind: ItemType,
    pub required: bool,
}

const fn core(n: u8, name: &'static str, kind: ItemType, required: bool) -> Item {
    Item { name, ul: [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x01, 0x01, n], kind, required }
}

const fn dpp(n: u8, name: &'static str, kind: ItemType, required: bool) -> Item {
    Item { name, ul: [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x0D, 0x0C, 0x01, 0x01, 0x01, 0x01, n, 0x00], kind, required }
}

const fn seg(n: u8, name: &'static str) -> Item {
    Item { name, ul: [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x02, 0x01, n], kind: ItemType::U16, required: true }
}

use ItemType::*;

/// The AS-11 Core framework items.
pub const CORE_ITEMS: &[Item] = &[
    core(0x01, "SeriesTitle", Text, true),
    core(0x02, "ProgrammeTitle", Text, true),
    core(0x03, "EpisodeTitleNumber", Text, true),
    core(0x04, "ShimName", Text, true),
    core(0x05, "AudioTrackLayout", U8, true),
    core(0x06, "PrimaryAudioLanguage", Text, true),
    core(0x07, "ClosedCaptionsPresent", Bool, true),
    core(0x08, "ClosedCaptionsType", U8, false),
    core(0x09, "ClosedCaptionsLanguage", Text, false),
    core(0x0A, "ShimVersion", Version, true),
];

/// The AS-11 Segmentation framework items.
pub const SEGMENTATION_ITEMS: &[Item] = &[seg(0x01, "PartNumber"), seg(0x02, "PartTotal")];

/// The UK DPP framework items.
pub const UKDPP_ITEMS: &[Item] = &[
    dpp(0x01, "ProductionNumber", Text, true),
    dpp(0x02, "Synopsis", Text, true),
    dpp(0x03, "Originator", Text, true),
    dpp(0x04, "CopyrightYear", U16, true),
    dpp(0x05, "OtherIdentifier", Text, false),
    dpp(0x06, "OtherIdentifierType", Text, false),
    dpp(0x07, "Genre", Text, false),
    dpp(0x08, "Distributor", Text, false),
    dpp(0x09, "PictureRatio", Rational, false),
    dpp(0x0A, "ThreeD", Bool, true),
    dpp(0x0B, "ThreeDType", U8, false),
    dpp(0x0C, "ProductPlacement", Bool, false),
    dpp(0x0D, "PSEPass", U8, true),
    dpp(0x0E, "PSEManufacturer", Text, false),
    dpp(0x0F, "PSEVersion", Text, false),
    dpp(0x10, "VideoComments", Text, false),
    dpp(0x11, "SecondaryAudioLanguage", Text, true),
    dpp(0x12, "TertiaryAudioLanguage", Text, true),
    dpp(0x13, "AudioLoudnessStandard", U8, true),
    dpp(0x14, "AudioComments", Text, false),
    dpp(0x15, "LineUpStart", Position, true),
    dpp(0x16, "IdentClockStart", Position, true),
    dpp(0x17, "TotalNumberOfParts", U16, true),
    dpp(0x18, "TotalProgrammeDuration", Length, true),
    dpp(0x19, "AudioDescriptionPresent", Bool, true),
    dpp(0x1A, "AudioDescriptionType", U8, false),
    dpp(0x1B, "OpenCaptionsPresent", Bool, true),
    dpp(0x1C, "OpenCaptionsType", U8, false),
    dpp(0x1D, "OpenCaptionsLanguage", Text, false),
    dpp(0x1E, "SigningPresent", U8, true),
    dpp(0x1F, "SignLanguage", U8, false),
    dpp(0x20, "CompletionDate", Timestamp, true),
    dpp(0x21, "TextlessElementsExist", Bool, false),
    dpp(0x22, "ProgrammeHasText", Bool, false),
    dpp(0x23, "ProgrammeTextLanguage", Text, false),
    dpp(0x24, "ContactEmail", Text, true),
    dpp(0x25, "ContactTelephoneNumber", Text, true),
];

/// Set keys of the frameworks.
pub const CORE_FRAMEWORK_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x01, 0x01, 0x00];
pub const SEGMENTATION_FRAMEWORK_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x02, 0x01, 0x00];
pub const UKDPP_FRAMEWORK_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x0C, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00];
/// Descriptive metadata scheme labels (the preface's DMSchemes).
pub const CORE_SCHEME: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x01, 0x00, 0x00];
pub const SEGMENTATION_SCHEME: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x0B, 0x02, 0x00, 0x00];
pub const UKDPP_SCHEME: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x0C, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00];
/// The descriptive metadata data definition.
pub const DEF_DESCRIPTIVE: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x01, 0x10, 0x00, 0x00, 0x00];

/// Track names and ids bmx gives the DM tracks.
pub const CORE_TRACK: (&str, u32) = ("AS_11_Core", 5001);
pub const SEGMENTATION_TRACK: (&str, u32) = ("AS_11_Segmentation", 5002);
pub const UKDPP_TRACK: (&str, u32) = ("AS_11_UKDPP", 5101);

/// One programme part: `start` and `duration` in frames of the material package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub start: i64,
    pub duration: i64,
    pub part_number: u16,
    pub part_total: u16,
}

/// AS-11 metadata of a file: name / value pairs of each framework (empty: the framework is
/// absent) and the programme parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct As11 {
    pub core: Vec<(String, String)>,
    pub segments: Vec<Segment>,
    pub ukdpp: Vec<(String, String)>,
}

/// Frame counting for positions and lengths: the material package's start time code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// Frame count of the first frame.
    pub start: i64,
    /// Frames per second of the time code (rounded).
    pub base: u16,
}

impl Clock {
    fn frames(&self, s: &str) -> Option<i64> {
        let s = s.trim();
        if let Ok(n) = s.parse::<i64>() {
            return Some(n);
        }
        let parts: Vec<&str> = s.split([':', ';', '.']).collect();
        let [h, m, sec, f] = parts.as_slice() else { return None };
        let n = |x: &str, max: i64| x.parse::<i64>().ok().filter(|v| (0..=max).contains(v));
        let base = i64::from(self.base.max(1));
        let (h, m, sec, f) = (n(h, 23)?, n(m, 59)?, n(sec, 59)?, n(f, base - 1)?);
        Some(((h * 60 + m) * 60 + sec) * base + f)
    }

    fn format(&self, frames: i64) -> String {
        let base = i64::from(self.base.max(1));
        let f = frames.rem_euclid(24 * 3600 * base);
        format!("{:02}:{:02}:{:02}:{:02}", f / (base * 3600), f / (base * 60) % 60, f / base % 60, f % base)
    }
}

/// The 8 bytes of a timestamp from `2011-08-01` or `2011-08-01T12:30:00`.
fn timestamp(s: &str) -> Option<[u8; 8]> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let t = match time {
        Some(t) => chrono::NaiveTime::parse_from_str(t.trim_end_matches('Z'), "%H:%M:%S").ok()?,
        None => chrono::NaiveTime::MIN,
    };
    use chrono::{Datelike, Timelike};
    let year = u16::try_from(d.year()).ok()?;
    let y = year.to_be_bytes();
    Some([y[0], y[1], d.month() as u8, d.day() as u8, t.hour() as u8, t.minute() as u8, t.second() as u8, 0])
}

fn boolean(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// The value of an item as stored, from its text.
pub fn encode(item: &Item, value: &str, clock: Clock) -> Result<Vec<u8>, String> {
    let bad = || format!("{}: \"{value}\" is not a valid {:?}", item.name, item.kind);
    let v = value.trim();
    Ok(match item.kind {
        Text => {
            let units: Vec<u16> = value.encode_utf16().collect();
            // a local set item holds at most 65 535 bytes
            if units.len() > 0x7FFF {
                return Err(format!("{}: longer than {} characters", item.name, 0x7FFF));
            }
            units.iter().flat_map(|u| u.to_be_bytes()).collect()
        }
        U8 => vec![v.parse::<u8>().map_err(|_| bad())?],
        U16 => v.parse::<u16>().map_err(|_| bad())?.to_be_bytes().to_vec(),
        Bool => vec![u8::from(boolean(v).ok_or_else(bad)?)],
        Version => {
            let (a, b) = v.split_once('.').unwrap_or((v, "0"));
            vec![a.parse::<u8>().map_err(|_| bad())?, b.parse::<u8>().map_err(|_| bad())?]
        }
        Rational => {
            let (a, b) = v.split_once(['/', ':']).ok_or_else(bad)?;
            let (a, b) = (a.trim().parse::<i32>().map_err(|_| bad())?, b.trim().parse::<i32>().map_err(|_| bad())?);
            if b == 0 {
                return Err(bad());
            }
            [a.to_be_bytes(), b.to_be_bytes()].concat()
        }
        Position => {
            let f = clock.frames(v).ok_or_else(bad)?;
            // a time code is measured from the start time code; a plain number is a position
            let pos = if v.parse::<i64>().is_ok() { f } else { f.saturating_sub(clock.start) };
            pos.to_be_bytes().to_vec()
        }
        Length => clock.frames(v).ok_or_else(bad)?.max(0).to_be_bytes().to_vec(),
        Timestamp => timestamp(v).ok_or_else(bad)?.to_vec(),
    })
}

/// The text of a stored item value.
pub fn decode(item: &Item, v: &[u8], clock: Clock) -> Option<String> {
    Some(match item.kind {
        Text => utf16_of(v),
        U8 | U16 => int_of(v, false)?.to_string(),
        Bool => (*v.first()? != 0).to_string(),
        Version => format!("{}.{}", v.first()?, v.get(1)?),
        Rational => {
            let r = rational_of(v)?;
            format!("{}/{}", r.num, r.den)
        }
        Position => clock.format(clock.start.saturating_add(int_of(v, true)?)),
        Length => int_of(v, true)?.to_string(),
        Timestamp => {
            let y = u16::from_be_bytes([*v.first()?, *v.get(1)?]);
            let (mo, d, h, mi, s) = (*v.get(2)?, *v.get(3)?, *v.get(4)?, *v.get(5)?, *v.get(6)?);
            if (h, mi, s) == (0, 0, 0) { format!("{y:04}-{mo:02}-{d:02}") } else { format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}") }
        }
    })
}

/// The framework's items as stored, checked: every name known, every required item given.
pub fn encode_all(items: &'static [Item], values: &[(String, String)], clock: Clock) -> Result<Vec<(&'static Item, Vec<u8>)>, String> {
    let mut out: Vec<(&'static Item, Vec<u8>)> = Vec::new();
    for (name, value) in values {
        let item = items.iter().find(|i| i.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("unknown AS-11 item \"{name}\""))?;
        if out.iter().any(|(i, _)| i.name == item.name) {
            return Err(format!("{}: given twice", item.name));
        }
        out.push((item, encode(item, value, clock)?));
    }
    if let Some(missing) = items.iter().find(|i| i.required && !out.iter().any(|(o, _)| o.name == i.name)) {
        return Err(format!("{} is required", missing.name));
    }
    out.sort_by_key(|(i, _)| i.ul);
    Ok(out)
}

/// The UK DPP items the writer fills in when they are not given: the number of parts and the
/// programme duration from the segmentation (bmx does the same).
pub fn complete_ukdpp(values: &[(String, String)], segments: &[Segment]) -> Vec<(String, String)> {
    let mut v = values.to_vec();
    let has = |v: &[(String, String)], n: &str| v.iter().any(|(k, _)| k.trim().eq_ignore_ascii_case(n));
    if !has(&v, "TotalNumberOfParts") {
        v.push(("TotalNumberOfParts".into(), segments.len().min(usize::from(u16::MAX)).to_string()));
    }
    if !has(&v, "TotalProgrammeDuration") {
        let total = segments.iter().fold(0i64, |a, s| a.saturating_add(s.duration.max(0)));
        v.push(("TotalProgrammeDuration".into(), total.to_string()));
    }
    v
}

/// Check programme parts against a `duration`-frame package: in order, not overlapping, inside.
pub fn check_segments(segments: &[Segment], duration: Option<i64>) -> Result<(), String> {
    let mut next = 0i64;
    for (k, s) in segments.iter().enumerate() {
        if s.duration <= 0 || s.start < next {
            return Err(format!("AS-11 part {} (frame {}, {} frames) is empty or starts before the previous part ends", k + 1, s.start, s.duration));
        }
        next = s.start.saturating_add(s.duration);
    }
    if let Some(d) = duration
        && next > d
    {
        return Err(format!("the AS-11 parts end at frame {next}, after the programme ({d} frames)"));
    }
    Ok(())
}

/// Read the AS-11 frameworks of a file's header metadata.
pub(crate) fn read(md: &crate::meta::Metadata, clock: Clock) -> Option<As11> {
    use crate::klv::Ul;
    let values = |key: [u8; 16], items: &'static [Item]| -> Vec<(String, String)> {
        let Some(s) = md.sets.iter().find(|s| s.key.0 == key) else { return Vec::new() };
        items.iter().filter_map(|i| s.dynamic.get(&Ul(i.ul)).and_then(|v| decode(i, v, clock)).map(|v| (i.name.to_string(), v))).collect()
    };
    let core = values(CORE_FRAMEWORK_KEY, CORE_ITEMS);
    let ukdpp = values(UKDPP_FRAMEWORK_KEY, UKDPP_ITEMS);
    // segmentation: the DM segments of the sequence whose segments hold segmentation frameworks
    let mut segments = Vec::new();
    for seq in md.of_kind(0x0F) {
        let comps = seq.references(0x1001);
        let is_seg = comps
            .iter()
            .filter_map(|c| md.get(c))
            .any(|c| c.kind() == Some(0x41) && c.reference(0x6101).and_then(|f| md.get(&f)).is_some_and(|f| f.key.0 == SEGMENTATION_FRAMEWORK_KEY));
        if !is_seg {
            continue;
        }
        let mut pos = 0i64;
        for c in comps.iter().filter_map(|c| md.get(c)).take(10_000) {
            let d = c.i(0x0202).unwrap_or(0).max(0);
            if c.kind() == Some(0x41)
                && let Some(f) = c.reference(0x6101).and_then(|f| md.get(&f))
            {
                let get = |n: usize| {
                    SEGMENTATION_ITEMS.get(n).and_then(|i| f.dynamic.get(&Ul(i.ul))).and_then(|v| int_of(v, false)).and_then(|v| u16::try_from(v).ok())
                };
                segments.push(Segment { start: pos, duration: d, part_number: get(0).unwrap_or(0), part_total: get(1).unwrap_or(0) });
            }
            pos = pos.saturating_add(d);
        }
        break;
    }
    (!core.is_empty() || !ukdpp.is_empty() || !segments.is_empty()).then_some(As11 { core, segments, ukdpp })
}

/// First dynamic local tag of the AS-11 items (in the order Core, Segmentation, UK DPP).
const FIRST_TAG: u16 = 0x8060;

fn all_items() -> impl Iterator<Item = &'static Item> {
    CORE_ITEMS.iter().chain(SEGMENTATION_ITEMS).chain(UKDPP_ITEMS)
}

/// The primer entries of every AS-11 item.
pub(crate) fn primer() -> Vec<(u16, [u8; 16])> {
    all_items().zip(FIRST_TAG..).map(|(i, t)| (t, i.ul)).collect()
}

/// The dynamic local tag of an item.
pub(crate) fn tag(item: &Item) -> u16 {
    all_items().zip(FIRST_TAG..).find(|(i, _)| i.ul == item.ul).map_or(FIRST_TAG, |(_, t)| t)
}

/// The frameworks of an [`As11`] as stored, checked once when a writer starts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Encoded {
    pub core: Vec<(&'static Item, Vec<u8>)>,
    pub ukdpp: Vec<(&'static Item, Vec<u8>)>,
    pub segments: Vec<Segment>,
}

impl Encoded {
    pub(crate) fn new(a: &As11, clock: Clock) -> Result<Encoded, String> {
        if a.core.is_empty() {
            return Err("AS-11 needs the AS-11 Core framework".into());
        }
        check_segments(&a.segments, None)?;
        if a.segments.len() > 10_000 {
            return Err("more than 10 000 AS-11 parts".into());
        }
        let core = encode_all(CORE_ITEMS, &a.core, clock)?;
        let ukdpp = if a.ukdpp.is_empty() { Vec::new() } else { encode_all(UKDPP_ITEMS, &complete_ukdpp(&a.ukdpp, &a.segments), clock)? };
        Ok(Encoded { core, ukdpp, segments: a.segments.clone() })
    }

    /// The preface's DMSchemes.
    pub(crate) fn schemes(&self) -> Vec<[u8; 16]> {
        let mut v = vec![CORE_SCHEME];
        if !self.segments.is_empty() {
            v.push(SEGMENTATION_SCHEME);
        }
        if !self.ukdpp.is_empty() {
            v.push(UKDPP_SCHEME);
        }
        v
    }
}

/// A framework in bmx's text form: one `Name: value` per line (blank lines and `#` comments are
/// skipped).
pub fn parse_framework_text(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once(':').ok_or_else(|| format!("line {}: expected \"Name: value\"", n + 1))?;
        out.push((k.trim().to_string(), v.trim().to_string()));
        if out.len() > 1_000 {
            return Err("more than 1000 framework lines".into());
        }
    }
    Ok(out)
}

/// Programme parts in bmx's segmentation text form: one `part/total start duration` per line, the
/// start a time code of the file's clock and the duration a time code or a frame count
/// (`1/2 09:58:00:12 00:00:00:05`).
pub fn parse_segments_text(text: &str, clock: Clock) -> Result<Vec<Segment>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || format!("line {}: expected \"part/total start duration\"", n + 1);
        let mut it = line.split_whitespace();
        let (Some(parts), Some(start), Some(dur), None) = (it.next(), it.next(), it.next(), it.next()) else { return Err(bad()) };
        let (p, t) = parts.split_once('/').ok_or_else(bad)?;
        let part_number = p.parse::<u16>().map_err(|_| bad())?;
        let part_total = t.parse::<u16>().map_err(|_| bad())?;
        let start = clock.frames(start).ok_or_else(bad)?;
        let start = if start < clock.start { start.saturating_add(24 * 3600 * i64::from(clock.base.max(1))) } else { start } - clock.start;
        let duration = clock.frames(dur).ok_or_else(bad)?;
        out.push(Segment { start, duration, part_number, part_total });
        if out.len() > 10_000 {
            return Err("more than 10 000 AS-11 parts".into());
        }
    }
    Ok(out)
}

/// Check framework text against its item list (names, values, required items) without a file:
/// positions are checked for syntax only.
pub fn check_framework_text(text: &str, ukdpp: bool) -> Result<(), String> {
    let values = parse_framework_text(text)?;
    // any frame number of any rate passes the syntax check
    let clock = Clock { start: 0, base: 1000 };
    if ukdpp { encode_all(UKDPP_ITEMS, &complete_ukdpp(&values, &[]), clock).map(|_| ()) } else { encode_all(CORE_ITEMS, &values, clock).map(|_| ()) }
}
