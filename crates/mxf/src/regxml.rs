//! RegXML (SMPTE ST 2001-1) of a track file's essence descriptor: the XML fragment an IMF
//! Composition Playlist carries in its `EssenceDescriptor` elements (ST 2067-2 6.8).
//!
//! Only the properties this crate's writer puts on picture and sound descriptors are mapped
//! (generic picture, RGBA, CDCI, Wave, the JPEG 2000 sub-descriptor and the MCA label
//! sub-descriptors). Their symbols and value types come from the SMPTE metadata registers
//! (elements ST 335, groups ST 395, types RP 2003-2012); values follow the ST 2001-1 mapping
//! rules: integers in decimal, enumerations by name, rationals as `n/d`, labels as
//! `urn:smpte:ul:` URNs, UUIDs as `urn:uuid:` URNs, byte strings in lowercase hexadecimal, arrays
//! and records as child elements named after their element type and members, and strong
//! references as the referenced set nested in place. Properties the table does not know are left
//! out.

use std::fmt::Write as _;

use crate::klv::{Cur, Ul, batch_of, int_of, rational_of, utf16_of};
use crate::meta::{Primer, set_type as st};
use crate::peek::header_sets;
use crate::source::ByteSource;
use crate::{Error, Result};

/// Group (class) namespace, element (property) namespace, type namespace.
const NS_GROUPS: &str = "http://www.smpte-ra.org/reg/395/2014/13/1/aaf";
const NS_ELEMENTS: &str = "http://www.smpte-ra.org/reg/335/2012";
const NS_TYPES: &str = "http://www.smpte-ra.org/reg/2003/2012";

/// Nesting limit for strong references (sub-descriptors of sub-descriptors).
const MAX_DEPTH: usize = 4;

#[derive(Clone, Copy)]
enum Kind {
    /// Unsigned or signed integer.
    Int {
        signed: bool,
    },
    Rational,
    /// A UL (AUID record, weak reference to a definition, extendible enumeration).
    Ul,
    Uuid,
    Utf16,
    /// ISO 7 (ASCII) string.
    Iso7,
    Enum(&'static [(i64, &'static str)]),
    /// A byte string (DataValue).
    Hex,
    Int32Array,
    /// A batch of strong references to sub-descriptor sets.
    SubDescriptors,
    J2kComponentSizing,
    /// The 8-pair RGBA layout (PixelLayout, J2CLayout).
    RgbaLayout,
}

const BOOLEAN: &[(i64, &str)] = &[(0, "False"), (1, "True")];
const LAYOUT: &[(i64, &str)] = &[(0, "FullFrame"), (1, "SeparateFields"), (2, "OneField"), (3, "MixedFields"), (4, "SegmentedFrame")];
const SCANNING: &[(i64, &str)] = &[
    (0, "ScanningDirection_LeftToRightTopToBottom"),
    (1, "ScanningDirection_RightToLeftTopToBottom"),
    (2, "ScanningDirection_LeftToRightBottomToTop"),
    (3, "ScanningDirection_RightToLeftBottomToTop"),
    (4, "ScanningDirection_TopToBottomLeftToRight"),
    (5, "ScanningDirection_TopToBottomRightToLeft"),
    (6, "ScanningDirection_BottomToTopLeftToRight"),
    (7, "ScanningDirection_BottomToTopRightToLeft"),
];
const SITING: &[(i64, &str)] = &[
    (0, "CoSiting"),
    (1, "HorizontalMidpoint"),
    (2, "ThreeTap"),
    (3, "Quincunx"),
    (4, "Rec601"),
    (5, "LineAlternating"),
    (6, "VerticalMidpoint"),
    (255, "UnknownSiting"),
];
const RGBA_CODES: &[(i64, &str)] = &[
    (0, "CompNull"),
    (48, "CompNone"),
    (65, "CompAlpha"),
    (66, "CompBlue"),
    (70, "CompFill"),
    (71, "CompGreen"),
    (80, "CompPalette"),
    (82, "CompRed"),
    (85, "CompColorDifferenceU"),
    (86, "CompColorDifferenceV"),
    (87, "CompComposite"),
    (88, "CompNonCoSitedLuma"),
    (89, "CompLuma"),
    (90, "CompDepth"),
    (97, "CompAlphaLSBs"),
    (98, "CompBlueLSBs"),
    (103, "CompGreenLSBs"),
    (114, "CompRedLSBs"),
    (117, "CompColorDifferenceULSBs"),
    (118, "CompColorDifferenceVLSBs"),
    (119, "CompCompositeLSBs"),
    (120, "CompNonCoSitedLumaLSBs"),
    (121, "CompLumaLSBs"),
    (122, "CompDepthLSBs"),
    (216, "CompColorX"),
    (217, "CompColorY"),
    (218, "CompColorZ"),
];

/// Static local tags (ST 377-1 Annex B, ST 382): symbol and type.
const STATIC: &[(u16, &str, Kind)] = &[
    (0x3C0A, "InstanceID", Kind::Uuid),
    (0x3006, "LinkedTrackID", Kind::Int { signed: false }),
    (0x3001, "SampleRate", Kind::Rational),
    (0x3002, "EssenceLength", Kind::Int { signed: true }),
    (0x3004, "ContainerFormat", Kind::Ul),
    (0x320C, "FrameLayout", Kind::Enum(LAYOUT)),
    (0x3203, "StoredWidth", Kind::Int { signed: false }),
    (0x3202, "StoredHeight", Kind::Int { signed: false }),
    (0x3209, "DisplayWidth", Kind::Int { signed: false }),
    (0x3208, "DisplayHeight", Kind::Int { signed: false }),
    (0x320A, "DisplayXOffset", Kind::Int { signed: true }),
    (0x320B, "DisplayYOffset", Kind::Int { signed: true }),
    (0x320E, "ImageAspectRatio", Kind::Rational),
    (0x320D, "VideoLineMap", Kind::Int32Array),
    (0x3201, "PictureCompression", Kind::Ul),
    (0x3210, "TransferCharacteristic", Kind::Ul),
    (0x3219, "ColorPrimaries", Kind::Ul),
    (0x321A, "CodingEquations", Kind::Ul),
    (0x3406, "ComponentMaxRef", Kind::Int { signed: false }),
    (0x3407, "ComponentMinRef", Kind::Int { signed: false }),
    (0x3405, "ScanningDirection", Kind::Enum(SCANNING)),
    (0x3401, "PixelLayout", Kind::RgbaLayout),
    (0x3301, "ComponentDepth", Kind::Int { signed: false }),
    (0x3302, "HorizontalSubsampling", Kind::Int { signed: false }),
    (0x3308, "VerticalSubsampling", Kind::Int { signed: false }),
    (0x3303, "ColorSiting", Kind::Enum(SITING)),
    (0x3304, "BlackRefLevel", Kind::Int { signed: false }),
    (0x3305, "WhiteRefLevel", Kind::Int { signed: false }),
    (0x3306, "ColorRange", Kind::Int { signed: false }),
    (0x3D03, "AudioSampleRate", Kind::Rational),
    (0x3D02, "Locked", Kind::Enum(BOOLEAN)),
    (0x3D04, "AudioReferenceLevel", Kind::Int { signed: true }),
    (0x3D06, "SoundCompression", Kind::Ul),
    (0x3D07, "ChannelCount", Kind::Int { signed: false }),
    (0x3D01, "QuantizationBits", Kind::Int { signed: false }),
    (0x3D0A, "BlockAlign", Kind::Int { signed: false }),
    (0x3D09, "AverageBytesPerSecond", Kind::Int { signed: false }),
    (0x3D32, "ChannelAssignment", Kind::Ul),
];

/// Dynamic properties by item designator (UL bytes 9-16, the version byte ignored).
const DYNAMIC: &[(&[u8], &str, Kind)] = &[
    (&[0x06, 0x01, 0x01, 0x04, 0x06, 0x10], "SubDescriptors", Kind::SubDescriptors),
    (&[0x04, 0x01, 0x06, 0x03, 0x01], "Rsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x02], "Xsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x03], "Ysiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x04], "XOsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x05], "YOsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x06], "XTsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x07], "YTsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x08], "XTOsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x09], "YTOsiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x0A], "Csiz", Kind::Int { signed: false }),
    (&[0x04, 0x01, 0x06, 0x03, 0x0B], "PictureComponentSizing", Kind::J2kComponentSizing),
    (&[0x04, 0x01, 0x06, 0x03, 0x0C], "CodingStyleDefault", Kind::Hex),
    (&[0x04, 0x01, 0x06, 0x03, 0x0D], "QuantizationDefault", Kind::Hex),
    (&[0x04, 0x01, 0x06, 0x03, 0x0E], "J2CLayout", Kind::RgbaLayout),
    (&[0x04, 0x02, 0x01, 0x01, 0x06], "ReferenceImageEditRate", Kind::Rational),
    (&[0x04, 0x02, 0x01, 0x01, 0x07], "ReferenceAudioAlignmentLevel", Kind::Int { signed: true }),
    (&[0x01, 0x03, 0x07, 0x01, 0x01], "MCALabelDictionaryID", Kind::Ul),
    (&[0x01, 0x03, 0x07, 0x01, 0x05], "MCALinkID", Kind::Uuid),
    (&[0x01, 0x03, 0x07, 0x01, 0x02], "MCATagSymbol", Kind::Utf16),
    (&[0x01, 0x03, 0x07, 0x01, 0x03], "MCATagName", Kind::Utf16),
    (&[0x01, 0x03, 0x04, 0x0A], "MCAChannelID", Kind::Int { signed: false }),
    (&[0x03, 0x01, 0x01, 0x02, 0x03, 0x15], "RFC5646SpokenLanguage", Kind::Iso7),
    (&[0x01, 0x03, 0x07, 0x01, 0x06], "SoundfieldGroupLinkID", Kind::Uuid),
    (&[0x01, 0x05, 0x10], "MCATitle", Kind::Utf16),
    (&[0x01, 0x05, 0x11], "MCATitleVersion", Kind::Utf16),
    (&[0x03, 0x02, 0x01, 0x02, 0x20], "MCAAudioContentKind", Kind::Utf16),
    (&[0x03, 0x02, 0x01, 0x02, 0x21], "MCAAudioElementKind", Kind::Utf16),
];

/// Group symbols by the set type byte.
fn group_symbol(kind: u8) -> Option<&'static str> {
    Some(match kind {
        st::CDCI_DESCRIPTOR => "CDCIDescriptor",
        st::RGBA_DESCRIPTOR => "RGBADescriptor",
        st::WAVE_DESCRIPTOR => "WAVEPCMDescriptor",
        st::JPEG2000_SUBDESCRIPTOR => "JPEG2000SubDescriptor",
        0x6B => "AudioChannelLabelSubDescriptor",
        0x6C => "SoundfieldGroupLabelSubDescriptor",
        0x6D => "GroupOfSoundfieldGroupsLabelSubDescriptor",
        _ => return None,
    })
}

/// A set's properties in file order: (static tag, dynamic UL when the tag is dynamic, value).
struct RawSet {
    kind: Option<u8>,
    uid: [u8; 16],
    props: Vec<(u16, Option<Ul>, Vec<u8>)>,
}

fn raw_set(key: Ul, v: &[u8], primer: &Primer) -> RawSet {
    let kind = key.item_starts_with(&[0x0D, 0x01, 0x01, 0x01, 0x01, 0x01]).then_some(key.0[14]);
    let mut s = RawSet { kind, uid: [0; 16], props: Vec::new() };
    let mut c = Cur::new(v);
    while c.remaining() >= 4 {
        let (Some(tag), Some(len)) = (c.u16(), c.u16()) else { break };
        let Some(val) = c.bytes(usize::from(len)) else { break };
        if tag == 0x3C0A
            && let Some(u) = val.get(..16).and_then(|b| <[u8; 16]>::try_from(b).ok())
        {
            s.uid = u;
        }
        let ul = if tag >= 0x8000 { primer.0.get(&tag).copied() } else { None };
        s.props.push((tag, ul, val.to_vec()));
    }
    s
}

/// The RegXML fragment of the essence descriptor of the file's top-level file package (the
/// source package with a descriptor), read from the header partition only. The root element
/// declares the three register namespaces (`r0` groups, `r1` elements, `r2` types).
pub fn essence_descriptor(src: &(impl ByteSource + ?Sized)) -> Result<String> {
    let (_, primer, sets) = header_sets(src)?;
    let raw: Vec<RawSet> = sets.iter().map(|(k, v)| raw_set(*k, v, &primer)).collect();
    let descriptor = raw
        .iter()
        .filter(|s| s.kind == Some(st::SOURCE_PACKAGE))
        .find_map(|p| p.props.iter().find(|(t, _, _)| *t == 0x4701).and_then(|(_, _, v)| v.get(..16)).and_then(|b| <[u8; 16]>::try_from(b).ok()))
        .ok_or_else(|| Error::Invalid("no file package with an essence descriptor".into()))?;
    let root = raw.iter().find(|s| s.uid == descriptor).ok_or_else(|| Error::Invalid("the essence descriptor set is missing".into()))?;
    let mut out = String::new();
    write_set(&mut out, &raw, root, 0, true)?;
    Ok(out)
}

fn write_set(out: &mut String, all: &[RawSet], s: &RawSet, depth: usize, root: bool) -> Result<()> {
    let group = s.kind.and_then(group_symbol).ok_or_else(|| Error::Invalid("an essence descriptor set has no RegXML mapping".into()))?;
    let ind = "  ".repeat(depth);
    if root {
        let _ = writeln!(out, "{ind}<r0:{group} xmlns:r0=\"{NS_GROUPS}\" xmlns:r1=\"{NS_ELEMENTS}\" xmlns:r2=\"{NS_TYPES}\">");
    } else {
        let _ = writeln!(out, "{ind}<r0:{group}>");
    }
    for (tag, ul, v) in &s.props {
        let entry = match ul {
            Some(ul) => DYNAMIC
                .iter()
                .find(|(item, _, _)| ul.item_starts_with(item) && ul.0.get(8 + <[u8]>::len(item)..).is_some_and(|rest| rest.iter().all(|&b| b == 0)))
                .map(|&(_, n, k)| (n, k)),
            None => STATIC.iter().find(|(t, _, _)| t == tag).map(|&(_, n, k)| (n, k)),
        };
        let Some((name, kind)) = entry else { continue };
        write_property(out, all, name, kind, v, depth + 1)?;
    }
    let _ = writeln!(out, "{ind}</r0:{group}>");
    Ok(())
}

fn uuid_urn(b: &[u8]) -> Option<String> {
    let hex = |r: std::ops::Range<usize>| -> Option<String> { Some(b.get(r)?.iter().map(|x| format!("{x:02x}")).collect()) };
    Some(format!("urn:uuid:{}-{}-{}-{}-{}", hex(0..4)?, hex(4..6)?, hex(6..8)?, hex(8..10)?, hex(10..16)?))
}

/// A label as an ST 2029 URN: `urn:smpte:ul:060e2b34.0401010d.03020101.00000000`.
pub fn ul_urn(b: &[u8; 16]) -> String {
    let groups: Vec<String> = b.chunks(4).map(|c| c.iter().map(|x| format!("{x:02x}")).collect()).collect();
    format!("urn:smpte:ul:{}", groups.join("."))
}

fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn enum_name(table: &[(i64, &'static str)], v: i64) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == v).map(|(_, n)| *n)
}

fn write_property(out: &mut String, all: &[RawSet], name: &str, kind: Kind, v: &[u8], depth: usize) -> Result<()> {
    let ind = "  ".repeat(depth);
    let simple = |text: Option<String>| -> Option<String> { text.map(|t| format!("{ind}<r1:{name}>{t}</r1:{name}>\n")) };
    let line = match kind {
        Kind::Int { signed } => simple(int_of(v, signed).map(|n| n.to_string())),
        Kind::Rational => simple(rational_of(v).map(|r| format!("{}/{}", r.num, r.den))),
        Kind::Ul => simple(v.get(..16).and_then(|b| <[u8; 16]>::try_from(b).ok()).map(|u| ul_urn(&u))),
        Kind::Uuid => simple(uuid_urn(v)),
        Kind::Utf16 => simple(Some(xml_text(&utf16_of(v)))),
        Kind::Iso7 => simple(Some(xml_text(&String::from_utf8_lossy(v).chars().take_while(|&c| c != '\0').collect::<String>()))),
        Kind::Enum(table) => simple(int_of(v, false).and_then(|n| enum_name(table, n)).map(str::to_string)),
        Kind::Hex => simple(Some(v.iter().map(|b| format!("{b:02x}")).collect())),
        Kind::Int32Array => {
            let mut x = format!("{ind}<r1:{name}>\n");
            for item in batch_of(v) {
                if let Some(n) = int_of(item, true) {
                    let _ = writeln!(x, "{ind}  <r2:Int32>{n}</r2:Int32>");
                }
            }
            let _ = writeln!(x, "{ind}</r1:{name}>");
            Some(x)
        }
        Kind::J2kComponentSizing => {
            let mut x = format!("{ind}<r1:{name}>\n");
            for c in batch_of(v) {
                let [ssiz, xr, yr] = <[u8; 3]>::try_from(c.get(..3).unwrap_or_default()).unwrap_or_default();
                let _ = writeln!(
                    x,
                    "{ind}  <r2:J2KComponentSizing>\n{ind}    <r2:Ssiz>{ssiz}</r2:Ssiz>\n{ind}    <r2:XRSiz>{xr}</r2:XRSiz>\n{ind}    <r2:YRSiz>{yr}</r2:YRSiz>\n{ind}  </r2:J2KComponentSizing>"
                );
            }
            let _ = writeln!(x, "{ind}</r1:{name}>");
            Some(x)
        }
        Kind::RgbaLayout => {
            let mut x = format!("{ind}<r1:{name}>\n");
            for k in 0..8 {
                let code = v.get(2 * k).copied().unwrap_or(0);
                let size = v.get(2 * k + 1).copied().unwrap_or(0);
                let code = enum_name(RGBA_CODES, i64::from(code)).unwrap_or("CompNull");
                let _ = writeln!(
                    x,
                    "{ind}  <r2:RGBAComponent>\n{ind}    <r2:Code>{code}</r2:Code>\n{ind}    <r2:ComponentSize>{size}</r2:ComponentSize>\n{ind}  </r2:RGBAComponent>"
                );
            }
            let _ = writeln!(x, "{ind}</r1:{name}>");
            Some(x)
        }
        Kind::SubDescriptors => {
            if depth > MAX_DEPTH {
                return Err(Error::Invalid("sub-descriptors nested too deeply".into()));
            }
            let mut x = format!("{ind}<r1:{name}>\n");
            for r in batch_of(v) {
                let Some(uid) = r.get(..16).and_then(|b| <[u8; 16]>::try_from(b).ok()) else { continue };
                // unresolved references and sets without a mapping are left out
                if let Some(s) = all.iter().find(|s| s.uid == uid && s.kind.and_then(group_symbol).is_some()) {
                    write_set(&mut x, all, s, depth + 1, false)?;
                }
            }
            let _ = writeln!(x, "{ind}</r1:{name}>");
            Some(x)
        }
    };
    if let Some(l) = line {
        out.push_str(&l);
    }
    Ok(())
}
