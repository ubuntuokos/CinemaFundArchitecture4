//! AMWA AS-02 version files (AS-02 sections 4.3.5, 5.2, 5.4): an OP1b / OP2b / OP3b file with
//! external essence. Its header metadata holds one material package (the version's timeline, one
//! source clip per piece of a component file, fillers for gaps) and a copy of the file package of
//! every essence component file, each with a network locator that names the component file
//! relative to the version file. The file holds no essence, no Essence Container Data and no index
//! table; the footer partition only closes the file.
//!
//! The file package copies here carry a minimal file descriptor (edit rate and the track id the
//! package describes), not a copy of the component's full descriptor.
//!
//! ```text
//! header partition pack (closed, complete) · primer · preface · identification · content
//!   storage · material package (+ tracks) · file packages (+ tracks) · file descriptors ·
//!   network locators
//! footer partition pack (closed, complete) · random index pack
//! ```

use std::io;

use super::labels::*;
use super::{Set, Timestamp, Umid, batch, hash128, klv, utf16};
use crate::Rational;

/// Tracks, clips and components a version file may hold (the metadata must fit the 16-bit local
/// set lengths and a sensible file size).
pub const MAX_VERSION_TRACKS: usize = 256;
pub const MAX_VERSION_CLIPS: usize = 10_000;
pub const MAX_VERSION_COMPONENTS: usize = 256;

/// Locators property of a generic descriptor and the URL string of a network locator (ST 377-1
/// Annex B), for the primer pack.
const LOCATOR_PRIMER: &[(u16, [u8; 16])] =
    &[(0x2F01, ul(&[0x01, 0x01, 0x01, 0x02, 0x06, 0x01, 0x01, 0x04, 0x06, 0x03])), (0x4001, ul(&[0x01, 0x01, 0x01, 0x01, 0x01, 0x02, 0x01, 0x01, 0x01]))];

/// The kind of a version track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionTrackKind {
    Picture,
    Sound,
}

/// One piece of a track: a range of a component file's track, or a filler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionClipSpec {
    /// The component's file package UMID; `None` for a filler.
    pub package: Option<Umid>,
    /// Track id inside the component's file package.
    pub source_track: u32,
    /// First edit unit used (at the version track's edit rate).
    pub start: i64,
    pub duration: i64,
}

/// A track of the version's material package.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionTrackSpec {
    pub kind: VersionTrackKind,
    pub track_id: u32,
    /// MXF track number (non-zero and unique, AS-02 4.3.5).
    pub track_number: u32,
    pub name: String,
    pub edit_rate: Rational,
    pub clips: Vec<VersionClipSpec>,
}

/// An essence component file the version uses.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentSpec {
    /// UMID of the component's top-level file package.
    pub umid: Umid,
    pub kind: VersionTrackKind,
    pub edit_rate: Rational,
    /// The component's duration in edit units.
    pub duration: i64,
    /// The component's track id (the one the clips use as `source_track`).
    pub track_id: u32,
    /// The path of the component file relative to the version file, `/` separated.
    pub locator: String,
}

/// What a version file says.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionConfig {
    /// The material package UMID: the version's file identifier (AS-02 9.2.1).
    pub material: Umid,
    pub name: String,
    pub tracks: Vec<VersionTrackSpec>,
    pub components: Vec<ComponentSpec>,
    pub company: String,
    pub product: String,
    pub version: String,
    pub modified: Timestamp,
}

impl VersionConfig {
    pub fn new(material: Umid, name: &str) -> Self {
        VersionConfig {
            material,
            name: name.to_string(),
            tracks: Vec::new(),
            components: Vec::new(),
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

/// Instance UID `n` of the file (derived from the material package UMID).
fn uid(material: &Umid, n: u32) -> [u8; 16] {
    let mut u = material.material_number();
    u[6] = (u[6] & 0x0F) | 0x40;
    u[8] = (u[8] & 0x3F) | 0x80;
    u[12..].copy_from_slice(&n.to_be_bytes());
    u
}

/// OP1b for one track, OP2b / OP3b are not written: a multi-track OP1b file (ST 391) has the
/// multi-track qualifier. Qualifier bits (byte 15): bit 0 always set, 0x02 external essence,
/// 0x08 multi-track.
fn op_label(tracks: usize) -> [u8; 16] {
    [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x02, 0x03 | if tracks > 1 { 0x08 } else { 0x00 }, 0x00]
}

fn partition(op: &[u8; 16], kind: u8, this: u64, prev: u64, footer: u64, hbc: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(112);
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    for x in [this, prev, footer, hbc, 0] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&0u32.to_be_bytes()); // IndexSID
    v.extend_from_slice(&0u64.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes()); // BodySID
    v.extend_from_slice(op);
    v.extend_from_slice(&batch(&[], 16));
    let mut out = Vec::new();
    klv(&mut out, &partition_key(kind, 4), &v);
    out
}

fn def(kind: VersionTrackKind) -> [u8; 16] {
    match kind {
        VersionTrackKind::Picture => DEF_PICTURE,
        VersionTrackKind::Sound => DEF_SOUND,
    }
}

/// A track, its sequence and the components of the sequence.
#[allow(clippy::too_many_arguments)]
fn track(
    out: &mut Vec<u8>,
    m: &Umid,
    uid0: u32,
    id: u32,
    number: u32,
    name: Option<&str>,
    rate: Rational,
    kind: [u8; 16],
    duration: i64,
    comps: &[(u32, Set)],
) {
    let (tu, su) = (uid(m, uid0), uid(m, uid0 + 1));
    let mut t = Set::new(0x3B, tu);
    t.u32(0x4801, id).u32(0x4804, number).rat(0x4B01, rate).i64(0x4B02, 0).p(0x4803, &su);
    if let Some(n) = name.filter(|n| !n.is_empty()) {
        t.p(0x4802, &utf16(n));
    }
    t.write(out);
    let cu: Vec<[u8; 16]> = comps.iter().map(|(n, _)| uid(m, *n)).collect();
    let mut s = Set::new(0x0F, su);
    s.p(0x0201, &kind).i64(0x0202, duration).refs(0x1001, &cu);
    s.write(out);
    for (_, c) in comps {
        c.write(out);
    }
}

fn clip_set(m: &Umid, n: u32, kind: [u8; 16], c: &VersionClipSpec) -> Set {
    let mut s = Set::new(if c.package.is_some() { 0x11 } else { 0x09 }, uid(m, n));
    s.p(0x0201, &kind).i64(0x0202, c.duration);
    if let Some(p) = c.package {
        s.i64(0x1201, c.start).p(0x1101, &p.0).u32(0x1102, c.source_track);
    }
    s
}

fn metadata(cfg: &VersionConfig) -> Vec<u8> {
    let m = &cfg.material;
    let u = |n| uid(m, n);
    let mut out = Vec::new();
    let primer: Vec<Vec<u8>> = PRIMER
        .iter()
        .chain(LOCATOR_PRIMER)
        .map(|(t, ul)| {
            let mut e = t.to_be_bytes().to_vec();
            e.extend_from_slice(ul);
            e
        })
        .collect();
    let items: Vec<&[u8]> = primer.iter().map(Vec::as_slice).collect();
    klv(&mut out, &PRIMER_KEY, &batch(&items, 18));
    let ts = cfg.modified.bytes();
    // Preface: the material package is the primary package (OP1b)
    let mut s = Set::new(0x2F, u(1));
    s.p(0x3B02, &ts).u16(0x3B05, 0x0103).u32(0x3B07, 1).refs(0x3B06, &[u(2)]).p(0x3B03, &u(3)).p(0x3B09, &op_label(cfg.tracks.len())).p(0x3B08, &u(10));
    s.p(0x3B0A, &batch(&[], 16)).p(0x3B0B, &batch(&[], 16));
    s.write(&mut out);
    let mut s = Set::new(0x30, u(2));
    s.p(0x3C09, &u(100))
        .p(0x3C01, &utf16(&cfg.company))
        .p(0x3C02, &utf16(&cfg.product))
        .p(0x3C04, &utf16(&cfg.version))
        .p(0x3C05, &hash128(b"CFA4 MXF writer"))
        .p(0x3C06, &ts);
    s.write(&mut out);
    // content storage: the packages, no Essence Container Data (4.3.5)
    let comp_uids: Vec<[u8; 16]> = (0..cfg.components.len()).map(|i| u(1000 + 100 * i as u32)).collect();
    let mut pkgs = vec![u(10)];
    pkgs.extend(&comp_uids);
    let mut s = Set::new(0x18, u(3));
    s.refs(0x1901, &pkgs).refs(0x1902, &[]);
    s.write(&mut out);

    // the material package: one track per version track
    let mut uid_next = 100_000u32;
    let mut track_sets: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut track_uids = Vec::new();
    for t in &cfg.tracks {
        let first = uid_next;
        uid_next += 1; // the track takes `first`, its sequence `first + 1`
        let mut comps: Vec<(u32, Set)> = Vec::new();
        let mut total = 0i64;
        let k = def(t.kind);
        for c in &t.clips {
            uid_next += 1;
            comps.push((uid_next, clip_set(m, uid_next, k, c)));
            total = total.saturating_add(c.duration);
        }
        uid_next += 1;
        let mut buf = Vec::new();
        track(&mut buf, m, first, t.track_id, t.track_number, Some(&t.name), t.edit_rate, k, total, &comps);
        track_uids.push(u(first));
        track_sets.push((first, buf));
    }
    let mut s = Set::new(0x36, u(10));
    s.p(0x4401, &m.0);
    if !cfg.name.is_empty() {
        s.p(0x4402, &utf16(&cfg.name));
    }
    s.p(0x4405, &ts).p(0x4404, &ts).refs(0x4403, &track_uids);
    s.write(&mut out);
    for (_, b) in &track_sets {
        out.extend_from_slice(b);
    }

    // the file package copies: one track, a descriptor with one network locator
    for (i, c) in cfg.components.iter().enumerate() {
        let base = 1000 + 100 * i as u32;
        let (tu, loc, desc) = (u(base + 1), u(base + 5), u(base + 4));
        let mut s = Set::new(0x37, u(base));
        s.p(0x4401, &c.umid.0).p(0x4405, &ts).p(0x4404, &ts).refs(0x4403, &[tu]).p(0x4701, &desc);
        s.write(&mut out);
        let number = if c.kind == VersionTrackKind::Picture { 0x1501_0500u32 } else { 0x1601_0300 };
        // a file package's source clip points to nothing (the essence itself)
        let mut comp = Set::new(0x11, u(base + 3));
        comp.p(0x0201, &def(c.kind)).i64(0x0202, c.duration).i64(0x1201, 0).p(0x1101, &[0u8; 32]).u32(0x1102, 0);
        track(&mut out, m, base + 1, c.track_id, number, None, c.edit_rate, def(c.kind), c.duration, &[(base + 3, comp)]);
        let mut d = Set::new(0x25, desc);
        d.u32(0x3006, c.track_id).rat(0x3001, c.edit_rate).i64(0x3002, c.duration).refs(0x2F01, &[loc]);
        d.write(&mut out);
        let mut l = Set::new(0x32, loc);
        l.p(0x4001, &utf16(&c.locator));
        l.write(&mut out);
    }
    out
}

fn check(cfg: &VersionConfig) -> io::Result<()> {
    if cfg.tracks.is_empty() || cfg.tracks.len() > MAX_VERSION_TRACKS {
        return Err(invalid("a version file needs 1 to 256 tracks"));
    }
    if cfg.components.len() > MAX_VERSION_COMPONENTS {
        return Err(invalid("too many essence components in a version file"));
    }
    let mut numbers: Vec<u32> = cfg.tracks.iter().map(|t| t.track_number).collect();
    if numbers.contains(&0) {
        return Err(invalid("a version file's track numbers are non-zero"));
    }
    numbers.sort_unstable();
    if numbers.windows(2).any(|p| p[0] == p[1]) {
        return Err(invalid("a version file's track numbers are unique"));
    }
    let mut clips = 0usize;
    for t in &cfg.tracks {
        if !t.edit_rate.is_valid() {
            return Err(invalid("a version track needs a positive edit rate"));
        }
        clips = clips.saturating_add(t.clips.len());
        for c in &t.clips {
            if c.duration < 0 || c.start < 0 {
                return Err(invalid("a version clip has a negative start or duration"));
            }
            if let Some(p) = c.package
                && !cfg.components.iter().any(|x| x.umid == p)
            {
                return Err(invalid("a version clip names a package that is not a listed component"));
            }
        }
    }
    if clips > MAX_VERSION_CLIPS {
        return Err(invalid("too many clips in a version file"));
    }
    for c in &cfg.components {
        if !c.edit_rate.is_valid() || c.duration < 0 {
            return Err(invalid("a component needs a positive edit rate and a non-negative duration"));
        }
        if c.locator.is_empty() || c.locator.len() > 4096 || c.locator.starts_with('/') || c.locator.contains('\\') || c.locator.split('/').any(|p| p == "..") {
            return Err(invalid("a component locator is a relative path inside the bundle"));
        }
    }
    Ok(())
}

/// A complete version file in memory.
pub fn write_version(cfg: &VersionConfig) -> io::Result<Vec<u8>> {
    check(cfg)?;
    let op = op_label(cfg.tracks.len());
    let meta = metadata(cfg);
    let header_len = partition(&op, 2, 0, 0, 0, 0).len();
    let footer = (header_len + meta.len()) as u64;
    let mut out = partition(&op, 2, 0, 0, footer, meta.len() as u64);
    out.extend_from_slice(&meta);
    out.extend_from_slice(&partition(&op, 4, footer, 0, footer, 0));
    let mut rip = Vec::new();
    for (sid, off) in [(0u32, 0u64), (0, footer)] {
        rip.extend_from_slice(&sid.to_be_bytes());
        rip.extend_from_slice(&off.to_be_bytes());
    }
    rip.extend_from_slice(&((16 + 4 + rip.len() + 4) as u32).to_be_bytes());
    klv(&mut out, &RIP_KEY, &rip);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::as02_version::read_version;
    use crate::file::TrackKind;

    fn config() -> VersionConfig {
        let pic = Umid::from_seed(b"pic");
        let snd = Umid::from_seed(b"snd");
        let r25 = Rational::new(25, 1);
        let mut c = VersionConfig::new(Umid::from_seed(b"version"), "Version A");
        c.components = vec![
            ComponentSpec { umid: pic, kind: VersionTrackKind::Picture, edit_rate: r25, duration: 500, track_id: 2, locator: "media/pic.mxf".into() },
            ComponentSpec {
                umid: snd,
                kind: VersionTrackKind::Sound,
                edit_rate: Rational::new(48000, 1),
                duration: 960_000,
                track_id: 3,
                locator: "media/snd_1.mxf".into(),
            },
        ];
        c.tracks = vec![
            VersionTrackSpec {
                kind: VersionTrackKind::Picture,
                track_id: 1,
                track_number: 0x1501_0500,
                name: "V1".into(),
                edit_rate: r25,
                clips: vec![
                    VersionClipSpec { package: Some(pic), source_track: 2, start: 10, duration: 100 },
                    VersionClipSpec { package: None, source_track: 0, start: 0, duration: 25 },
                    VersionClipSpec { package: Some(pic), source_track: 2, start: 200, duration: 50 },
                ],
            },
            VersionTrackSpec {
                kind: VersionTrackKind::Sound,
                track_id: 2,
                track_number: 0x1601_0300,
                name: "A1".into(),
                edit_rate: Rational::new(48000, 1),
                clips: vec![VersionClipSpec { package: Some(snd), source_track: 3, start: 0, duration: 336_000 }],
            },
        ];
        c
    }

    #[test]
    fn a_version_file_reads_back_as_written() {
        let cfg = config();
        let bytes = write_version(&cfg).unwrap();
        let v = read_version(&bytes).unwrap();
        assert!(v.problems.is_empty(), "{:?}", v.problems);
        assert_eq!(v.operational_pattern, "OP1b");
        assert_eq!(v.material_umid, cfg.material.0);
        assert_eq!(v.name.as_deref(), Some("Version A"));
        assert_eq!(v.tracks.len(), 2);
        assert_eq!((v.tracks[0].kind, v.tracks[0].track_number, v.tracks[0].name.as_deref()), (TrackKind::Picture, 0x1501_0500, Some("V1")));
        let clips: Vec<(Option<[u8; 32]>, i64, i64)> = v.tracks[0].clips.iter().map(|c| (c.package, c.start, c.duration)).collect();
        assert_eq!(clips, vec![(Some(cfg.components[0].umid.0), 10, 100), (None, 0, 25), (Some(cfg.components[0].umid.0), 200, 50)]);
        assert_eq!(v.tracks[0].clips[0].source_track, 2);
        assert_eq!((v.tracks[1].kind, v.tracks[1].edit_rate.num), (TrackKind::Sound, 48000));
        assert_eq!(v.components.len(), 2);
        assert_eq!(v.component(&cfg.components[1].umid.0).and_then(|c| c.locator.as_deref()), Some("media/snd_1.mxf"));
    }

    #[test]
    fn a_version_file_is_refused_when_it_breaks_the_rules() {
        let bad = |f: &dyn Fn(&mut VersionConfig)| {
            let mut c = config();
            f(&mut c);
            write_version(&c).is_err()
        };
        assert!(bad(&|c| c.tracks.clear()));
        assert!(bad(&|c| c.tracks[1].track_number = c.tracks[0].track_number));
        assert!(bad(&|c| c.tracks[0].track_number = 0));
        assert!(bad(&|c| c.components[0].locator = "../x.mxf".into()));
        assert!(bad(&|c| c.components[0].locator = "/abs.mxf".into()));
        assert!(bad(&|c| c.tracks[0].clips[0].package = Some(Umid::from_seed(b"nobody"))));
        assert!(bad(&|c| c.tracks[0].clips[0].duration = -1));
    }
}
