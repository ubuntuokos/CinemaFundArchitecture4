//! Timed text track files (SMPTE ST 429-5): the XML document clip-wrapped in the body partition,
//! each ancillary resource (font, PNG subpicture) alone in a generic stream partition (ST 410),
//! the header metadata closed and complete in the header partition and the single index table
//! segment in the footer (ST 429-5 7.2, 11.6; ST 429-3).
//!
//! Layout written:
//!
//! ```text
//! header partition pack (closed, complete) · primer · preface · identification · content
//!   storage · essence container data · material package · file package · TimedTextDescriptor ·
//!   TimedTextResourceSubDescriptors
//! body partition pack (BodySID 1) · timed text element (clip-wrapped XML)
//! generic stream partition pack (BodySID 2) · resource 1      ...one per resource
//! footer partition pack (IndexSID 129) · index table segment (one entry)
//! random index pack (every partition, generic streams included; 6.3)
//! ```

use std::io;

use super::labels::*;
use super::{PackageIds, Set, Timestamp, Umid, batch, hash128, klv, utf16};
use crate::Rational;

/// The XML document's essence stream.
const BODY_SID: u32 = 1;
/// The index table (distinct from every BodySID; the generic streams count up from 2).
const INDEX_SID: u32 = 129;
/// Ancillary resources a track file may hold (the generic stream BodySIDs stay below the
/// IndexSID).
pub const MAX_TIMED_TEXT_RESOURCES: usize = 120;

/// MIME media type of an OpenType / TrueType font resource, as D-Cinema servers expect it.
pub const MIME_OPENTYPE: &str = "application/x-font-opentype";
/// MIME media type of a PNG subpicture.
pub const MIME_PNG: &str = "image/png";

/// An ancillary resource the document refers to by its UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedTextResource {
    /// The UUID the XML document uses (e.g. `LoadFont` / `Image` `urn:uuid:`).
    pub id: [u8; 16],
    /// RFC 2046 media type ([`MIME_OPENTYPE`], [`MIME_PNG`]).
    pub mime: String,
    pub data: Vec<u8>,
}

/// What a timed text track file describes.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedTextConfig {
    /// The composition's edit rate.
    pub edit_rate: Rational,
    /// Duration in edit units (the reel's).
    pub duration: i64,
    /// The document's own UUID (ST 428-7 `Id`), repeated in the descriptor (11.4).
    pub resource_id: [u8; 16],
    /// Namespace of the document's root element (ST 428-7:
    /// `http://www.smpte-ra.org/schemas/428-7/2010/DCST`).
    pub namespace: String,
    /// Package UMIDs and names (the file package UMID is the CPL asset's UUID).
    pub ids: PackageIds,
    pub company: String,
    pub product: String,
    pub version: String,
    pub modified: Timestamp,
}

impl TimedTextConfig {
    pub fn new(edit_rate: Rational, duration: i64, resource_id: [u8; 16], namespace: &str, ids: PackageIds) -> Self {
        TimedTextConfig {
            edit_rate,
            duration,
            resource_id,
            namespace: namespace.to_string(),
            ids,
            company: "CFA4".into(),
            product: "CFA4".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            modified: Timestamp::default(),
        }
    }
}

fn invalid(m: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, m.to_string())
}

/// Instance UID `n` of the file (derived from the file package UMID, version 4 variant bits).
fn uid(file: &Umid, n: u32) -> [u8; 16] {
    let mut u = file.material_number();
    u[6] = (u[6] & 0x0F) | 0x40;
    u[8] = (u[8] & 0x3F) | 0x80;
    u[12..].copy_from_slice(&n.to_be_bytes());
    u
}

/// A partition pack (ST 377-1 Table 5) of an OP-Atom timed text track file, KAG 1.
#[allow(clippy::too_many_arguments)]
fn partition(kind: u8, status: u8, this: u64, prev: u64, footer: u64, hbc: u64, ibc: u64, index_sid: u32, body_sid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(112);
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    for x in [this, prev, footer, hbc, ibc] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&index_sid.to_be_bytes());
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&body_sid.to_be_bytes());
    v.extend_from_slice(&OP_ATOM);
    // a generic stream partition carries no essence container of the generic container
    let containers: &[&[u8]] = if status == 0x11 { &[] } else { &[&EC_TIMED_TEXT] };
    v.extend_from_slice(&batch(containers, 16));
    let mut out = Vec::new();
    klv(&mut out, &partition_key(kind, status), &v);
    out
}

/// A track, its sequence and its one component (source clip or timecode).
#[allow(clippy::too_many_arguments)]
fn track(out: &mut Vec<u8>, file: &Umid, uid0: u32, id: u32, number: u32, rate: Rational, def: [u8; 16], duration: i64, component: impl FnOnce(&mut Set)) {
    let (tu, su, cu) = (uid(file, uid0), uid(file, uid0 + 1), uid(file, uid0 + 2));
    let mut t = Set::new(0x3B, tu);
    t.u32(0x4801, id).u32(0x4804, number).rat(0x4B01, rate).i64(0x4B02, 0).p(0x4803, &su);
    t.write(out);
    let mut s = Set::new(0x0F, su);
    s.p(0x0201, &def).i64(0x0202, duration).refs(0x1001, &[cu]);
    s.write(out);
    let mut c = Set::new(if def == DEF_TIMECODE { 0x14 } else { 0x11 }, cu);
    c.p(0x0201, &def).i64(0x0202, duration);
    component(&mut c);
    c.write(out);
}

/// The header metadata (ST 429-5 7.4, 7.5): one data track in each package, the timed text
/// descriptor and one resource sub-descriptor per ancillary resource.
fn metadata(cfg: &TimedTextConfig, resources: &[TimedTextResource]) -> Vec<u8> {
    let file = &cfg.ids.file;
    let u = |n| uid(file, n);
    let mut out = Vec::new();
    let primer: Vec<Vec<u8>> = PRIMER
        .iter()
        .chain(TIMED_TEXT_PRIMER)
        .map(|(t, ul)| {
            let mut e = t.to_be_bytes().to_vec();
            e.extend_from_slice(ul);
            e
        })
        .collect();
    let items: Vec<&[u8]> = primer.iter().map(Vec::as_slice).collect();
    klv(&mut out, &PRIMER_KEY, &batch(&items, 18));
    let ts = cfg.modified.bytes();
    // Preface: the file package is the primary package (ST 429-3)
    let mut s = Set::new(0x2F, u(1));
    s.p(0x3B02, &ts).u16(0x3B05, 0x0103).u32(0x3B07, 1).refs(0x3B06, &[u(2)]).p(0x3B03, &u(3)).p(0x3B09, &OP_ATOM).p(0x3B08, &u(2000));
    s.p(0x3B0A, &batch(&[&EC_TIMED_TEXT], 16)).p(0x3B0B, &batch(&[], 16));
    s.write(&mut out);
    let mut s = Set::new(0x30, u(2));
    s.p(0x3C09, &u(100))
        .p(0x3C01, &utf16(&cfg.company))
        .p(0x3C02, &utf16(&cfg.product))
        .p(0x3C04, &utf16(&cfg.version))
        .p(0x3C05, &hash128(b"CFA4 MXF writer"))
        .p(0x3C06, &ts);
    s.write(&mut out);
    let mut s = Set::new(0x18, u(3));
    s.refs(0x1901, &[u(1000), u(2000)]).refs(0x1902, &[u(4)]);
    s.write(&mut out);
    let mut s = Set::new(0x23, u(4));
    s.p(0x2701, &file.0).u32(0x3F06, INDEX_SID).u32(0x3F07, BODY_SID);
    s.write(&mut out);
    let rate = cfg.edit_rate;
    let dur = cfg.duration;
    for (material, base) in [(true, 1000u32), (false, 2000)] {
        let mut s = Set::new(if material { 0x36 } else { 0x37 }, u(base));
        let (umid, name) = if material { (cfg.ids.material, &cfg.ids.material_name) } else { (cfg.ids.file, &cfg.ids.file_name) };
        s.p(0x4401, &umid.0);
        if !name.is_empty() {
            s.p(0x4402, &utf16(name));
        }
        s.p(0x4405, &ts).p(0x4404, &ts).refs(0x4403, &[u(base + 1), u(base + 4)]);
        if !material {
            s.p(0x4701, &u(3000));
        }
        s.write(&mut out);
        // timecode track 1 (ST 429-3), the data track 2
        track(&mut out, file, base + 1, 1, 0, rate, DEF_TIMECODE, dur, |s| {
            let base = u16::try_from((rate.num.max(1) as i64 + rate.den.max(1) as i64 / 2) / rate.den.max(1) as i64).unwrap_or(24);
            s.u16(0x1502, base).i64(0x1501, 0).u8(0x1503, 0);
        });
        let number = if material { 0 } else { u32::from_be_bytes([0x17, 0x01, 0x0B, 0x01]) };
        let (src, src_track) = if material { (cfg.ids.file, 2) } else { (Umid([0; 32]), 0) };
        track(&mut out, file, base + 4, 2, number, rate, DEF_DATA, dur, |s| {
            s.i64(0x1201, 0).p(0x1101, &src.0).u32(0x1102, src_track);
        });
    }
    // TimedTextDescriptor (Table A.3: the generic data essence descriptor's items, then the
    // timed text items) and its sub-descriptors (Table A.6)
    let subs: Vec<[u8; 16]> = (0..resources.len()).map(|i| u(3100 + i as u32)).collect();
    let mut s = Set::new(0x64, u(3000));
    s.u32(0x3006, 2).rat(0x3001, rate).i64(0x3002, dur).p(0x3004, &EC_TIMED_TEXT);
    s.p(TAG_TIMED_TEXT, &cfg.resource_id).p(TAG_TIMED_TEXT + 1, &utf16("UTF-8")).p(TAG_TIMED_TEXT + 2, &utf16(&cfg.namespace));
    if !subs.is_empty() {
        s.refs(TAG_SUB_DESCRIPTORS, &subs);
    }
    s.write(&mut out);
    for (k, (r, sub)) in resources.iter().zip(&subs).enumerate() {
        let mut s = Set::new(0x65, *sub);
        s.p(TAG_TIMED_TEXT + 3, &r.id).p(TAG_TIMED_TEXT + 4, &utf16(&r.mime)).u32(TAG_TIMED_TEXT + 5, BODY_SID + 1 + k as u32);
        s.write(&mut out);
    }
    out
}

/// A complete timed text track file in memory: `document` (the XML) and its `resources`.
pub fn write_timed_text(cfg: &TimedTextConfig, document: &[u8], resources: &[TimedTextResource]) -> io::Result<Vec<u8>> {
    if !cfg.edit_rate.is_valid() {
        return Err(invalid("timed text edit rate must be positive"));
    }
    if cfg.duration < 0 {
        return Err(invalid("timed text duration is negative"));
    }
    if resources.len() > MAX_TIMED_TEXT_RESOURCES {
        return Err(invalid("too many timed text ancillary resources"));
    }
    let limit = crate::MAX_RESOURCE_BYTES as usize;
    if document.len() > limit || resources.iter().any(|r| r.data.len() > limit) {
        return Err(invalid("timed text resource over 64 MiB"));
    }
    for (i, r) in resources.iter().enumerate() {
        if resources.iter().take(i).any(|o| o.id == r.id) || r.id == cfg.resource_id {
            return Err(invalid("timed text resources need distinct ids"));
        }
    }
    let meta = metadata(cfg, resources);
    let header_len = partition(2, 4, 0, 0, 0, 0, 0, 0, 0).len();
    let body_at = (header_len + meta.len()) as u64;
    let mut body = partition(3, 4, body_at, 0, 0, 0, 0, 0, BODY_SID);
    klv(&mut body, &TIMED_TEXT_ELEMENT, document);
    // the generic stream partitions: the pack, then the resource KLV, nothing between (6.1)
    let mut streams = Vec::new();
    let mut offsets = Vec::new();
    let mut prev = body_at;
    let mut at = body_at + body.len() as u64;
    for (k, r) in resources.iter().enumerate() {
        let mut p = partition(3, 0x11, at, 0, 0, 0, 0, 0, BODY_SID + 1 + k as u32);
        klv(&mut p, &GENERIC_STREAM_ELEMENT, &r.data);
        offsets.push((BODY_SID + 1 + k as u32, at));
        prev = at;
        at += p.len() as u64;
        streams.push(p);
    }
    let footer = at;
    // ST 429-5 7.2: one index entry at the start of the element, no delta entries, no byte count
    let mut index = Vec::new();
    let mut s = Set { key: INDEX_KEY, v: Vec::new() };
    s.p(0x3C0A, &uid(&cfg.ids.file, 4000)).rat(0x3F0B, cfg.edit_rate).i64(0x3F0C, 0).i64(0x3F0D, 1).u32(0x3F05, 0);
    s.u32(0x3F06, INDEX_SID).u32(0x3F07, BODY_SID).u8(0x3F08, 0).u8(0x3F0E, 0);
    let entry: [u8; 11] = [0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0];
    s.p(0x3F0A, &batch(&[&entry], 11));
    s.write(&mut index);
    // rewrite the packs that need the footer offset
    let mut out = partition(2, 4, 0, 0, footer, meta.len() as u64, 0, 0, 0);
    out.extend_from_slice(&meta);
    let body_pack_len = partition(3, 4, body_at, 0, 0, 0, 0, 0, BODY_SID).len();
    out.extend_from_slice(&partition(3, 4, body_at, 0, footer, 0, 0, 0, BODY_SID));
    out.extend_from_slice(body.get(body_pack_len..).unwrap_or_default());
    for (p, &(sid, this)) in streams.iter().zip(&offsets) {
        let pack_len = partition(3, 0x11, this, 0, 0, 0, 0, 0, sid).len();
        let before = offsets.iter().take_while(|o| o.1 < this).last().map_or(body_at, |o| o.1);
        out.extend_from_slice(&partition(3, 0x11, this, before, footer, 0, 0, 0, sid));
        out.extend_from_slice(p.get(pack_len..).unwrap_or_default());
    }
    if out.len() as u64 != footer {
        return Err(io::Error::other("internal: timed text partition sizes changed"));
    }
    out.extend_from_slice(&partition(4, 4, footer, prev, footer, 0, index.len() as u64, INDEX_SID, 0));
    out.extend_from_slice(&index);
    let mut rip = Vec::new();
    let mut parts = vec![(0u32, 0u64), (BODY_SID, body_at)];
    parts.extend(offsets.iter().copied());
    parts.push((0, footer));
    for (sid, off) in parts {
        rip.extend_from_slice(&sid.to_be_bytes());
        rip.extend_from_slice(&off.to_be_bytes());
    }
    rip.extend_from_slice(&((16 + 4 + rip.len() + 4) as u32).to_be_bytes());
    klv(&mut out, &RIP_KEY, &rip);
    Ok(out)
}
