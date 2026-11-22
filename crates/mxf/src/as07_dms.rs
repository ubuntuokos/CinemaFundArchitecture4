//! AMWA AS-07 (MXF Archive and Preservation) descriptive metadata: the AS_07_Core_DMS framework
//! that every AS-07 file has (specification appendix D.1), with the AS_07_Core_DMS_Device sets
//! (D.2) and AS_07_DMS_Identifier sets (E) it refers to. The framework is one static DM track of
//! the material package and the scheme label is listed in the preface ([`crate::as07::CORE_SCHEME`]).
//!
//! Values are text, as for the AS-11 frameworks ([`crate::as11`]): item name / value pairs. The
//! audio track layout is a layout name of appendix E (`AudioLayout2TrackUndef`, ...) or a UL as 32
//! hex digits. The track name and id are ours: the specification does not fix them.

use crate::as11::{Clock, Item, ItemType::*, decode, encode, encode_all};
use crate::klv::{Ul, batch_of, ul_of};

const fn item(set: u8, n: u8, name: &'static str, kind: crate::as11::ItemType, required: bool) -> Item {
    Item { name, ul: [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x0D, 0x0E, 0x01, 0x01, 0x07, 0x01, set, n], kind, required }
}

/// AS_07_Core_DMS_Framework (the set key) of the core scheme. The specification's tables list the
/// keys with the set kind byte 0x7F ("any encoding"); a file stores them as local sets with 2-byte
/// tags and lengths (0x53, as every header metadata set).
pub const FRAMEWORK_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x0E, 0x01, 0x01, 0x07, 0x01, 0x01, 0x00];
/// AS_07_Core_DMS_Device and AS_07_DMS_Identifier set keys.
pub const DEVICE_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x0E, 0x01, 0x01, 0x07, 0x01, 0x02, 0x00];
pub const IDENTIFIER_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x0E, 0x01, 0x01, 0x07, 0x01, 0x03, 0x00];
/// The DM scheme label of AS_07_Core_DMS (appendix D.1), which every AS-07 file lists in its
/// preface's DMSchemes.
pub const CORE_SCHEME: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x07, 0x01, 0x07, 0x01, 0x00, 0x00];
/// Name and id of the DM track.
pub const TRACK: (&str, u32) = ("AS_07_Core_DMS", 5701);

/// The strong reference batches of the framework: the identifier sets (required, at least one
/// with the role Main) and the device sets.
pub(crate) const IDENTIFIERS: Item = item(0x01, 0x02, "Identifiers", U8, true);
pub(crate) const DEVICES: Item = item(0x01, 0x10, "Devices", U8, false);

/// The AS_07_Core_DMS items that hold a value (appendix D.1).
pub const CORE_ITEMS: &[Item] = &[
    item(0x01, 0x01, "ShimName", Text, true),
    item(0x01, 0x03, "ResponsibleOrganizationName", Text, true),
    item(0x01, 0x05, "ResponsibleOrganizationCode", Text, false),
    item(0x01, 0x06, "NatureOfOrganization", Text, false),
    item(0x01, 0x07, "WorkingTitle", Text, false),
    item(0x01, 0x08, "SecondaryTitle", Text, false),
    item(0x01, 0x09, "PictureFormat", Text, true),
    item(0x01, 0x0A, "IntendedAFD", Text, true),
    item(0x01, 0x0B, "Captions", Text, true),
    item(0x01, 0x0C, "AudioTrackPrimaryLanguage", Text, false),
    item(0x01, 0x0D, "AudioTrackSecondaryLanguage", Text, false),
    item(0x01, 0x0E, "AudioTrackLayout", Auid, true),
    item(0x01, 0x0F, "AudioTrackLayoutComment", Text, false),
];

/// The AS_07_Core_DMS_Device items (appendix D.2); all optional.
pub const DEVICE_ITEMS: &[Item] = &[
    item(0x02, 0x02, "DeviceType", Text, false),
    item(0x02, 0x03, "DeviceManufacturer", Text, false),
    item(0x02, 0x04, "DeviceModel", Utf8, false),
    item(0x02, 0x05, "DeviceSerialNumber", Utf8, false),
    item(0x02, 0x06, "DeviceUsageDescription", Text, false),
];

/// The AS_07_DMS_Identifier items (appendix E).
pub const IDENTIFIER_ITEMS: &[Item] = &[
    item(0x03, 0x02, "Value", Text, true),
    item(0x03, 0x03, "Role", Utf8, true),
    item(0x03, 0x04, "Type", Utf8, true),
    item(0x03, 0x05, "Comment", Text, false),
];

/// The audio track layouts of appendix E (labels 06.0E.2B.34.04.01.01.01.0D.0E.01.01.07.02.04.nn).
pub const LAYOUTS: &[(&str, u8)] = &[
    ("AudioLayoutSilence", 0x01),
    ("AudioLayoutUnknown", 0x02),
    ("AudioLayout1TrackUndef", 0x03),
    ("AudioLayout2TrackUndef", 0x04),
    ("AudioLayout3TrackUndef", 0x05),
    ("AudioLayout4TrackUndef", 0x06),
    ("AudioLayout1TrackAudio", 0x07),
    ("AudioLayout2TracksAudio", 0x08),
    ("AudioLayout1TrackAudio1TrackTime", 0x09),
    ("AudioLayout3TracksAudio", 0x0A),
    ("AudioLayout2TrackAudio1TrackTime", 0x0B),
    ("AudioLayout4TrackAudio", 0x0C),
    ("AudioLayout3TrackAudio1TrackTime", 0x0D),
    ("AudioLayoutEBU48_2a", 0x10),
    ("AudioLayoutEBU123_4b", 0x11),
    ("AudioLayoutEBU123_4c", 0x12),
    ("AudioLayoutEBU123_16c", 0x13),
    ("AudioLayoutEBU123_16d", 0x14),
    ("AudioLayoutEBU123_16f", 0x15),
    ("AudioLayoutST377_4MCA", 0x20),
];

fn layout_ul(n: u8) -> [u8; 16] {
    [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x0E, 0x01, 0x01, 0x07, 0x02, 0x04, n]
}

/// A UL from a layout name (with or without the `AudioLayout` prefix) or 32 hex digits.
pub(crate) fn auid(s: &str) -> Option<[u8; 16]> {
    let t = s.trim();
    let bare = |x: &str| x.strip_prefix("AudioLayout").unwrap_or(x).to_ascii_lowercase();
    if let Some((_, n)) = LAYOUTS.iter().find(|(name, _)| bare(name) == bare(t)) {
        return Some(layout_ul(*n));
    }
    let hex: String = t.trim_start_matches("urn:smpte:ul:").chars().filter(|c| *c != '.' && *c != '-').collect();
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let mut ul = [0u8; 16];
    for (b, c) in ul.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *b = u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()?;
    }
    Some(ul)
}

/// The text of a stored AUID: a layout name, else 32 hex digits.
pub(crate) fn auid_text(v: &[u8]) -> Option<String> {
    let ul: [u8; 16] = v.try_into().ok()?;
    Some(match LAYOUTS.iter().find(|(_, n)| layout_ul(*n) == ul) {
        Some((name, _)) => (*name).to_string(),
        None => ul.iter().map(|b| format!("{b:02x}")).collect(),
    })
}

/// The AS-07 descriptive metadata of a file: name / value pairs of the core framework, and of
/// each identifier and device set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dms07 {
    /// [`CORE_ITEMS`].
    pub core: Vec<(String, String)>,
    /// [`IDENTIFIER_ITEMS`], one entry per set; at least one has the role `Main`.
    pub identifiers: Vec<Vec<(String, String)>>,
    /// [`DEVICE_ITEMS`], one entry per set.
    pub devices: Vec<Vec<(String, String)>>,
}

/// Identifier roles of appendix E.
const ROLES: &[&str] = &["Main", "Additional", "GSP"];
/// Sets a file may hold (a writer limit, so the batches stay small).
const MAX_SETS: usize = 1000;

type Stored = Vec<(&'static Item, Vec<u8>)>;

/// The metadata as stored, checked once when a writer starts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Encoded {
    pub core: Stored,
    pub identifiers: Vec<Stored>,
    pub devices: Vec<Stored>,
}

fn names(items: &'static [Item], values: &[(String, String)], what: &str, clock: Clock) -> Result<Stored, String> {
    encode_all(items, values, clock).map_err(|e| format!("AS-07 {what}: {}", e.replace("AS-11 ", "")))
}

impl Encoded {
    pub(crate) fn new(d: &Dms07, clock: Clock) -> Result<Encoded, String> {
        let core = names(CORE_ITEMS, &d.core, "core", clock)?;
        if d.identifiers.is_empty() {
            return Err("AS-07 core: at least one identifier is required".into());
        }
        if d.identifiers.len() > MAX_SETS || d.devices.len() > MAX_SETS {
            return Err(format!("AS-07 core: more than {MAX_SETS} identifier or device sets"));
        }
        let identifiers = d.identifiers.iter().map(|i| names(IDENTIFIER_ITEMS, i, "identifier", clock)).collect::<Result<Vec<_>, _>>()?;
        let role = |s: &Stored| s.iter().find(|(i, _)| i.name == "Role").map(|(_, v)| String::from_utf8_lossy(v).into_owned()).unwrap_or_default();
        if let Some(r) = identifiers.iter().map(role).find(|r| !ROLES.contains(&r.as_str())) {
            return Err(format!("AS-07 identifier: the role \"{r}\" is not Main, Additional or GSP"));
        }
        if !identifiers.iter().any(|s| role(s) == "Main") {
            return Err("AS-07 core: one identifier needs the role Main".into());
        }
        let devices = d.devices.iter().map(|i| names(DEVICE_ITEMS, i, "device", clock)).collect::<Result<Vec<_>, _>>()?;
        Ok(Encoded { core, identifiers, devices })
    }
}

/// First dynamic local tag of the AS-07 items.
const FIRST_TAG: u16 = 0x8400;

fn all_items() -> impl Iterator<Item = &'static Item> {
    CORE_ITEMS.iter().chain([&IDENTIFIERS, &DEVICES]).chain(DEVICE_ITEMS).chain(IDENTIFIER_ITEMS)
}

/// The primer entries of every AS-07 item.
pub(crate) fn primer() -> Vec<(u16, [u8; 16])> {
    all_items().zip(FIRST_TAG..).map(|(i, t)| (t, i.ul)).collect()
}

/// The dynamic local tag of an item.
pub(crate) fn tag(item: &Item) -> u16 {
    all_items().zip(FIRST_TAG..).find(|(i, _)| i.ul == item.ul).map_or(FIRST_TAG, |(_, t)| t)
}

/// The AS-07 metadata of a file, when it has the core framework.
pub(crate) fn read(md: &crate::meta::Metadata, clock: Clock) -> Option<Dms07> {
    let fw = md.sets.iter().find(|s| s.key.0 == FRAMEWORK_KEY)?;
    let values = |s: &crate::meta::Set, items: &'static [Item]| -> Vec<(String, String)> {
        items.iter().filter_map(|i| s.dynamic.get(&Ul(i.ul)).and_then(|v| decode(i, v, clock)).map(|v| (i.name.to_string(), v))).collect()
    };
    let members = |batch: &Item, items: &'static [Item]| -> Vec<Vec<(String, String)>> {
        let Some(v) = fw.dynamic.get(&Ul(batch.ul)) else { return Vec::new() };
        batch_of(v).into_iter().filter_map(ul_of).filter_map(|u| md.get(&u.0)).take(MAX_SETS).map(|s| values(s, items)).collect()
    };
    Some(Dms07 { core: values(fw, CORE_ITEMS), identifiers: members(&IDENTIFIERS, IDENTIFIER_ITEMS), devices: members(&DEVICES, DEVICE_ITEMS) })
}

/// Checks the values of a [`Dms07`] without a writer (the shim, tools).
pub fn check(d: &Dms07) -> Result<(), String> {
    Encoded::new(d, Clock::default()).map(|_| ())
}

/// Encodes one item value (exposed for tools that show what a value is stored as).
pub fn encode_value(item: &Item, value: &str) -> Result<Vec<u8>, String> {
    encode(item, value, Clock::default())
}
