//! IMF virtual track fingerprint (CineCert / HPA draft memo "IMF Virtual Track Fingerprint",
//! `README.md`): a SHA-1 over the canonical timeline of one virtual track of an IMF composition
//! playlist (SMPTE ST 2067-3).
//!
//! The `Resource` elements of the track, in playlist order across all segments, become timeline
//! items (`TrackFileId`, `EntryPoint`, `SourceDuration`, `RepeatCount`). An item that is
//! *congruent* with the previous one (same track file, entry point and duration) adds its repeat
//! count to it; an item that *continues* the previous one (same track file, both played once, the
//! next edit unit) adds its duration to it; any other item starts a new entry. Each entry is then
//! digested as the 16 bytes of the track file UUID and the entry point, duration and repeat count
//! as 64-bit big-endian integers. A stereoscopic track (`StereoImageTrackFileResourceType`)
//! digests duration and repeat count first, then track file and entry point of the left and the
//! right eye.

use std::fmt;

use roxmltree::{Document, Node, ParsingOptions};
use sha1::{Digest, Sha1};

use crate::{Error, Result};

/// URN prefix of a virtual track fingerprint.
pub const URN_PREFIX: &str = "urn:smpte:imf-vtfp:";

/// Most nodes a composition playlist may have (DTDs are refused as well).
const MAX_NODES: u32 = 2_000_000;

/// Shortest abbreviated fingerprint the memo allows, in hexadecimal digits.
pub const MIN_DIGITS: usize = 4;

/// A virtual track fingerprint: the 20 bytes of a SHA-1.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Vtfp(pub [u8; 20]);

impl Vtfp {
    /// The 40 lower-case hexadecimal digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The URN form, `urn:smpte:imf-vtfp:<40 hex digits>`.
    pub fn to_urn(&self) -> String {
        format!("{URN_PREFIX}{}", self.to_hex())
    }
}

impl fmt::Display for Vtfp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_urn())
    }
}

impl fmt::Debug for Vtfp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vtfp({})", self.to_hex())
    }
}

/// The hexadecimal digits of a fingerprint URN, full or abbreviated (4 to 40 lower-case digits).
pub fn urn_digits(urn: &str) -> Result<&str> {
    let digits = urn.strip_prefix(URN_PREFIX).ok_or(Error::Urn("imf-vtfp"))?;
    let ok = (MIN_DIGITS..=40).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok { Ok(digits) } else { Err(Error::Urn("imf-vtfp")) }
}

/// Whether two fingerprint URNs match: exact and case-sensitive over the length of the shorter
/// one, so an abbreviated value matches the full value it was cut from.
pub fn urns_match(a: &str, b: &str) -> Result<bool> {
    let (a, b) = (urn_digits(a)?, urn_digits(b)?);
    let n = a.len().min(b.len());
    Ok(a.get(..n) == b.get(..n))
}

/// A virtual track of a composition playlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualTrack {
    /// `TrackId` as written, without the `urn:uuid:` prefix, lower case.
    pub track_id: String,
    /// Local name of the sequence element (`MainImageSequence`, `MainAudioSequence`, …).
    pub kind: String,
    /// Number of `Resource` elements of the track over all segments.
    pub resources: usize,
}

fn parse(xml: &str) -> Result<Document<'_>> {
    let opt = ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES, ..ParsingOptions::default() };
    Document::parse_with_options(xml, opt).map_err(|e| Error::Xml(e.to_string()))
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.text()).map(str::trim)
}

/// The UUID text of a `urn:uuid:` value, lower case.
fn uuid_text(v: &str) -> Option<String> {
    let s = v.trim();
    let s = s.strip_prefix("urn:uuid:").unwrap_or(s);
    parse_uuid(s).map(|_| s.to_ascii_lowercase())
}

/// The 16 bytes of a UUID in its 8-4-4-4-12 hexadecimal form (RFC 4122).
fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let s = s.strip_prefix("urn:uuid:").unwrap_or(s);
    let b = s.as_bytes();
    if b.len() != 36 || [8, 13, 18, 23].iter().any(|&i| b.get(i) != Some(&b'-')) {
        return None;
    }
    let hex: Vec<u8> = b.iter().copied().filter(|&c| c != b'-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in hex.chunks_exact(2).enumerate() {
        let hi = char::from(*pair.first()?).to_digit(16)?;
        let lo = char::from(*pair.get(1)?).to_digit(16)?;
        *out.get_mut(i)? = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Every sequence element of every segment, in playlist order.
fn sequences<'a, 'i>(doc: &'a Document<'i>) -> Result<Vec<Node<'a, 'i>>> {
    let root = doc.root_element();
    if root.tag_name().name() != "CompositionPlaylist" {
        return Err(Error::Cpl(format!("root element is {}, not CompositionPlaylist", root.tag_name().name())));
    }
    let segments = child(root, "SegmentList").ok_or_else(|| Error::Cpl("no SegmentList".into()))?;
    let mut out = Vec::new();
    for seg in segments.children().filter(|c| c.is_element() && c.tag_name().name() == "Segment") {
        let Some(list) = child(seg, "SequenceList") else { continue };
        out.extend(list.children().filter(|c| c.is_element() && child(*c, "TrackId").is_some()));
    }
    Ok(out)
}

fn resources<'a, 'i>(seq: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    child(seq, "ResourceList").into_iter().flat_map(|l| l.children()).filter(|c| c.is_element() && c.tag_name().name() == "Resource")
}

/// The virtual tracks of a composition playlist, in order of first appearance.
pub fn virtual_tracks(xml: &str) -> Result<Vec<VirtualTrack>> {
    let doc = parse(xml)?;
    let mut out: Vec<VirtualTrack> = Vec::new();
    for seq in sequences(&doc)? {
        let id = text(seq, "TrackId").and_then(uuid_text).ok_or_else(|| Error::Cpl(format!("{}: TrackId is not a UUID", seq.tag_name().name())))?;
        let count = resources(seq).count();
        match out.iter_mut().find(|t| t.track_id == id) {
            Some(t) => t.resources = t.resources.saturating_add(count),
            None => out.push(VirtualTrack { track_id: id, kind: seq.tag_name().name().to_string(), resources: count }),
        }
    }
    Ok(out)
}

/// One eye or one mono resource: where it plays from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Clip {
    track_file: [u8; 16],
    entry: u64,
    duration: u64,
}

/// A canonical timeline item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Mono { clip: Clip, repeat: u64 },
    Stereo { left: Clip, right: Clip, duration: u64, repeat: u64 },
}

fn number(n: Node<'_, '_>, name: &str) -> Result<Option<u64>> {
    match text(n, name) {
        None => Ok(None),
        Some(t) => t.parse::<u64>().map(Some).map_err(|_| Error::Cpl(format!("{name} is not a non-negative integer: {t}"))),
    }
}

/// `TrackFileId`, `EntryPoint` and `SourceDuration` of a resource (or one eye), with the ST 2067-3
/// defaults: entry point 0, duration from the entry point to the end of the intrinsic duration.
fn clip(n: Node<'_, '_>) -> Result<Clip> {
    let track_file = text(n, "TrackFileId").and_then(parse_uuid).ok_or_else(|| Error::Cpl("Resource without a TrackFileId UUID".into()))?;
    let entry = number(n, "EntryPoint")?.unwrap_or(0);
    let duration = match number(n, "SourceDuration")? {
        Some(d) => d,
        None => number(n, "IntrinsicDuration")?
            .and_then(|i| i.checked_sub(entry))
            .ok_or_else(|| Error::Cpl("Resource without SourceDuration or a usable IntrinsicDuration".into()))?,
    };
    Ok(Clip { track_file, entry, duration })
}

/// Duration of a stereo resource: both eyes play the same number of edit units, and the resource's
/// own `SourceDuration`, when present, must equal it (ST 2067-2 stereoscopic image resources).
fn stereo_duration(n: Node<'_, '_>, left: &Clip, right: &Clip) -> Result<u64> {
    if left.duration != right.duration {
        return Err(Error::Cpl("stereo Resource: LeftEye and RightEye SourceDuration differ".into()));
    }
    match number(n, "SourceDuration")? {
        Some(d) if d != left.duration => Err(Error::Cpl("stereo Resource: SourceDuration differs from its eyes'".into())),
        _ => Ok(left.duration),
    }
}

fn item(n: Node<'_, '_>) -> Result<Item> {
    let repeat = number(n, "RepeatCount")?.unwrap_or(1);
    match (child(n, "LeftEye"), child(n, "RightEye")) {
        (Some(l), Some(r)) => {
            let left = clip(l)?;
            let right = clip(r)?;
            Ok(Item::Stereo { left, right, duration: stereo_duration(n, &left, &right)?, repeat })
        }
        (None, None) => Ok(Item::Mono { clip: clip(n)?, repeat }),
        _ => Err(Error::Cpl("stereo Resource needs both LeftEye and RightEye".into())),
    }
}

/// `next` plays the edit unit right after `prev` in the same track file.
fn continues(prev: &Clip, next: &Clip) -> bool {
    prev.track_file == next.track_file && prev.entry.checked_add(prev.duration) == Some(next.entry)
}

/// Fold `next` into `prev` if it is congruent with it or continues it; `false` if it is a new item.
fn merge(prev: &mut Item, next: &Item) -> bool {
    match (prev, next) {
        (Item::Mono { clip: p, repeat: pr }, Item::Mono { clip: n, repeat: nr }) => {
            if p == n {
                *pr = pr.saturating_add(*nr);
                true
            } else if *pr == 1 && *nr == 1 && continues(p, n) {
                p.duration = p.duration.saturating_add(n.duration);
                true
            } else {
                false
            }
        }
        (Item::Stereo { left: pl, right: prr, duration: pd, repeat: pr }, Item::Stereo { left: nl, right: nrr, duration: nd, repeat: nr }) => {
            if pl == nl && prr == nrr && pd == nd {
                *pr = pr.saturating_add(*nr);
                true
            } else if *pr == 1 && *nr == 1 && continues(pl, nl) && continues(prr, nrr) {
                *pd = pd.saturating_add(*nd);
                pl.duration = pl.duration.saturating_add(nl.duration);
                prr.duration = prr.duration.saturating_add(nrr.duration);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// The canonical timeline of the virtual track `track_id` (with or without `urn:uuid:`).
fn timeline(xml: &str, track_id: &str) -> Result<Vec<Item>> {
    let want = uuid_text(track_id).ok_or_else(|| Error::Cpl(format!("track id is not a UUID: {track_id}")))?;
    let doc = parse(xml)?;
    let mut list: Vec<Item> = Vec::new();
    let mut found = false;
    for seq in sequences(&doc)? {
        if text(seq, "TrackId").and_then(uuid_text).as_deref() != Some(want.as_str()) {
            continue;
        }
        found = true;
        for res in resources(seq) {
            let next = item(res)?;
            let merged = list.last_mut().is_some_and(|prev| merge(prev, &next));
            if !merged {
                list.push(next);
            }
        }
    }
    if !found {
        return Err(Error::Cpl(format!("no virtual track {want}")));
    }
    let stereo = list.iter().filter(|i| matches!(i, Item::Stereo { .. })).count();
    if stereo != 0 && stereo != list.len() {
        return Err(Error::Cpl("virtual track mixes stereo and mono resources".into()));
    }
    Ok(list)
}

/// The virtual track fingerprint of the track `track_id` of the composition playlist `xml`.
pub fn fingerprint(xml: &str, track_id: &str) -> Result<Vtfp> {
    let mut sha = Sha1::new();
    for item in timeline(xml, track_id)? {
        match item {
            Item::Mono { clip, repeat } => {
                sha.update(clip.track_file);
                sha.update(clip.entry.to_be_bytes());
                sha.update(clip.duration.to_be_bytes());
                sha.update(repeat.to_be_bytes());
            }
            Item::Stereo { left, right, duration, repeat } => {
                sha.update(duration.to_be_bytes());
                sha.update(repeat.to_be_bytes());
                for eye in [left, right] {
                    sha.update(eye.track_file);
                    sha.update(eye.entry.to_be_bytes());
                }
            }
        }
    }
    Ok(Vtfp(sha.finalize().into()))
}
