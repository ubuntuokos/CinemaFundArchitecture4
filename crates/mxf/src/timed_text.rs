//! Timed text track files: SMPTE ST 429-5 (D-Cinema subtitles, ST 428-7 XML) and the IMF timed
//! text track files of ST 2067-2 (IMSC1 TTML), which use the same mapping.
//!
//! The XML document is one clip-wrapped data element (item type 17h, Table A.2) described by a
//! TimedTextDescriptor (Table A.3). Fonts and subpicture images it references are ancillary
//! resources, each alone in a generic stream partition (ST 410, clause 6) whose BodySID a
//! TimedTextResourceSubDescriptor (Table A.6) names.

use crate::file::{Element, best_metadata, walk};
use crate::klv::{find_header_partition, utf16_of};
use crate::meta::{SUB_DESCRIPTORS_ITEM, Set, tag};
use crate::source::{ByteSource, read_upto};
use crate::{Error, Rational, Result};

/// Set kind of the TimedTextDescriptor (Table A.4).
pub(crate) const TIMED_TEXT_DESCRIPTOR: u8 = 0x64;
/// Set kind of the TimedTextResourceSubDescriptor (Table A.7).
const TIMED_TEXT_RESOURCE_SUB_DESCRIPTOR: u8 = 0x65;

// Item designators (bytes 9-16) of the dynamic properties (Tables A.3, A.6).
const RESOURCE_ID: [u8; 8] = [0x01, 0x01, 0x15, 0x12, 0, 0, 0, 0];
const UCS_ENCODING: [u8; 8] = [0x04, 0x09, 0x05, 0x00, 0, 0, 0, 0];
const NAMESPACE_URI: [u8; 8] = [0x01, 0x02, 0x01, 0x05, 0x01, 0, 0, 0];
const ANCILLARY_RESOURCE_ID: [u8; 8] = [0x01, 0x01, 0x15, 0x13, 0, 0, 0, 0];
const MIME_MEDIA_TYPE: [u8; 8] = [0x04, 0x09, 0x02, 0x01, 0, 0, 0, 0];
const ESSENCE_STREAM_ID: [u8; 8] = [0x01, 0x03, 0x04, 0x04, 0, 0, 0, 0];

/// The documents [`TimedText::read_document`] and [`TimedText::read_resource`] return at most this
/// many bytes (subtitle XML is kilobytes; fonts and PNG subpictures a few megabytes).
pub const MAX_RESOURCE_BYTES: u64 = 64 << 20;

/// A byte range of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub offset: u64,
    pub size: u64,
}

/// A font or subpicture referenced by the timed text document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncillaryResource {
    /// The UUID the document uses to refer to the resource.
    pub id: [u8; 16],
    /// MIME media type, e.g. `application/x-font-opentype` or `image/png`.
    pub mime: Option<String>,
    /// BodySID of the generic stream partition that holds the resource.
    pub stream_id: u32,
    /// The resource data, when its partition is in the file.
    pub data: Option<Region>,
}

/// An opened timed text track file.
#[derive(Clone, Debug)]
pub struct TimedText {
    /// Edit rate of the track (the descriptor's sample rate).
    pub edit_rate: Rational,
    /// Duration in edit units, when known.
    pub duration: Option<i64>,
    /// UUID of the timed text resource.
    pub resource_id: Option<[u8; 16]>,
    /// Character encoding of the XML (`UTF-8` when absent).
    pub ucs_encoding: Option<String>,
    /// Namespace (or profile) of the document's root element, e.g.
    /// `http://www.smpte-ra.org/schemas/428-7/2010/DCST` or `http://www.w3.org/ns/ttml`.
    pub namespace: Option<String>,
    /// The XML document.
    pub document: Region,
    pub resources: Vec<AncillaryResource>,
    /// Problems recovered from.
    pub warnings: Vec<String>,
}

impl TimedText {
    /// The XML document's bytes.
    pub fn read_document(&self, src: &(impl ByteSource + ?Sized)) -> Result<Vec<u8>> {
        read_region(src, self.document)
    }
    /// The bytes of ancillary resource `i`.
    pub fn read_resource(&self, src: &(impl ByteSource + ?Sized), i: usize) -> Result<Vec<u8>> {
        let r = self.resources.get(i).ok_or_else(|| Error::Invalid(format!("no ancillary resource {i}")))?;
        let region = r.data.ok_or_else(|| Error::Invalid(format!("ancillary resource {i} (stream {}) is not in the file", r.stream_id)))?;
        read_region(src, region)
    }
}

fn read_region(src: &(impl ByteSource + ?Sized), r: Region) -> Result<Vec<u8>> {
    if r.size > MAX_RESOURCE_BYTES {
        return Err(Error::Unsupported(format!("timed text resource of {} bytes", r.size)));
    }
    let v = read_upto(src, r.offset, r.size as usize)?;
    if (v.len() as u64) < r.size {
        return Err(Error::Invalid("timed text resource cut short by the end of the file".into()));
    }
    Ok(v)
}

/// Whether `head` (the first bytes of a file) is an MXF file whose header partition lists the
/// timed text essence container (Table A.1: 06 0E 2B 34 04 01 01 0A 0D 01 03 01 02 13 01 01).
pub fn is_timed_text(head: &[u8]) -> bool {
    let Some(at) = find_header_partition(head) else { return false };
    let Some(rest) = head.get(at + 16..) else { return false };
    let Some((len, n)) = crate::klv::ber_length(rest) else { return false };
    let Some(value) = rest.get(n..).map(|v| &v[..v.len().min(len as usize)]) else { return false };
    // 80 bytes of fixed fields, then the essence container batch
    value.get(80..).is_some_and(|b| crate::klv::batch_of(b).into_iter().filter_map(crate::klv::ul_of).any(|ul| is_timed_text_container(&ul)))
}

fn is_timed_text_container(ul: &crate::Ul) -> bool {
    ul.is_smpte() && ul.item_starts_with(&[0x0D, 0x01, 0x03, 0x01, 0x02, 0x13])
}

fn uuid(v: &[u8]) -> Option<[u8; 16]> {
    v.get(..16).and_then(|b| b.try_into().ok())
}

fn text(v: &[u8]) -> Option<String> {
    Some(utf16_of(v)).filter(|s| !s.is_empty())
}

/// Open a timed text track file (ST 429-5 / ST 2067-2).
pub fn open_timed_text(src: &(impl ByteSource + ?Sized)) -> Result<TimedText> {
    let w = walk(src)?;
    let (_, md) = best_metadata(&w)?;
    let mut warnings = w.warnings.clone();
    let desc = md.of_kind(TIMED_TEXT_DESCRIPTOR).next().ok_or_else(|| Error::Invalid("no TimedTextDescriptor in the header metadata".into()))?;
    let element: &Element = w.elements.iter().find(|e| e.key.0[12] == 0x17).ok_or_else(|| Error::Invalid("no timed text essence element".into()))?;
    if element.truncated {
        return Err(Error::Invalid("the timed text document is cut short by the end of the file".into()));
    }
    // sub-descriptors: the descriptor's references, else every resource sub-descriptor
    let referenced: Vec<&Set> = desc
        .dynamic_item(&SUB_DESCRIPTORS_ITEM)
        .map(|v| {
            crate::klv::batch_of(v)
                .into_iter()
                .filter_map(uuid)
                .filter_map(|u| md.get(&u))
                .filter(|s| s.kind() == Some(TIMED_TEXT_RESOURCE_SUB_DESCRIPTOR))
                .collect()
        })
        .unwrap_or_default();
    let subs: Vec<&Set> = if referenced.is_empty() { md.of_kind(TIMED_TEXT_RESOURCE_SUB_DESCRIPTOR).collect() } else { referenced };
    let mut resources = Vec::new();
    for s in subs {
        let Some(id) = s.dynamic_item(&ANCILLARY_RESOURCE_ID).and_then(uuid) else {
            warnings.push("timed text resource sub-descriptor without a resource id".into());
            continue;
        };
        let stream_id = s.dynamic_item(&ESSENCE_STREAM_ID).and_then(|v| crate::klv::int_of(v, false)).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
        let data = w.generic_streams.iter().find(|(sid, _)| *sid == stream_id && stream_id != 0).map(|(_, e)| Region { offset: e.value_offset, size: e.size });
        if data.is_none() {
            warnings.push(format!("ancillary resource stream {stream_id} is not in the file"));
        }
        resources.push(AncillaryResource { id, mime: s.dynamic_item(&MIME_MEDIA_TYPE).and_then(text), stream_id, data });
    }
    Ok(TimedText {
        edit_rate: desc.rational(tag::SAMPLE_RATE).unwrap_or_default(),
        duration: desc.i(tag::CONTAINER_DURATION).filter(|d| *d >= 0),
        resource_id: desc.dynamic_item(&RESOURCE_ID).and_then(uuid),
        ucs_encoding: desc.dynamic_item(&UCS_ENCODING).and_then(text),
        namespace: desc.dynamic_item(&NAMESPACE_URI).and_then(text),
        document: Region { offset: element.value_offset, size: element.size },
        resources,
        warnings,
    })
}
