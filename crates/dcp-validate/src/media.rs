//! DCP-06 (plan F06, F07, F08): the track files a composition plays, read with `cfa4-mxf`, and the
//! picture codestreams in them, read with `cfa4-j2k`.
//!
//! - `DCP.MXF.001`: each track file is an MXF OP-Atom file (ST 429-3) with one essence track of
//!   the kind its reel asset declares, the reel asset's edit rate and intrinsic duration, its file
//!   package UMID carries the asset's UUID (ST 429-3 9.3.2), its labels are of the composition's
//!   standard (the OP-Atom label of SMPTE files is version 2, that of MXF Interop files 1), and it
//!   is encrypted exactly when the reel asset has a `KeyId`, with that key (ST 429-6).
//! - `DCP.J2K.001`: every frame of a picture track file follows the DCI JPEG 2000 profile (see
//!   [`crate::codestream`]) and stays within the DCI bit rate: 250 Mbit/s for the frame and
//!   200 Mbit/s for a component at the image rate (both eyes count for stereoscopic pictures).
//! - `DCP.PIC.001`: the picture's size is one of the DCI containers, its `ScreenAspectRatio`
//!   matches it, its `FrameRate` is the edit rate (twice it for stereoscopic pictures, ST 429-10),
//!   and the rate is one cinema servers play: 24 or 48 (2K) and 24 (4K) everywhere, the other
//!   ST 428-21 rates as a compatibility warning, anything else a failure.

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::FileExt as _;
use std::path::Path;

use cfa4_dcp::{AssetKind, Composition, Rational, ReelAsset, ScreenAspectRatio, Standard, Uuid};
use cfa4_mxf::{ByteSource, Codec, TrackKind};

use crate::codestream;
use crate::integrity::{Files, asset_element, at};
use crate::rules::*;
use crate::{Finding, Location, Status, ValidationControl};

/// A file read in place (no mapping, no copy).
pub(crate) struct FileSource {
    file: File,
    len: u64,
}

impl FileSource {
    pub(crate) fn open(path: &Path) -> std::io::Result<FileSource> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(FileSource { file, len })
    }

    fn bytes(&self, at: u64, n: usize) -> std::io::Result<Vec<u8>> {
        let n = n.min(usize::try_from(self.len.saturating_sub(at)).unwrap_or(usize::MAX));
        let mut b = vec![0; n];
        self.file.read_exact_at(&mut b, at)?;
        Ok(b)
    }
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.file.read_exact_at(buf, offset)
    }
}

/// What a track file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Essence {
    Picture,
    Sound,
    TimedText,
    Other,
}

/// A track file as read.
struct Track {
    essence: Essence,
    /// Essence tracks in the file.
    tracks: usize,
    op_atom: bool,
    /// The standard of its labels (`None` when the label is not one of the two).
    standard: Option<Standard>,
    edit_rate: Rational,
    duration: Option<i64>,
    /// Material number of the file package UMID.
    asset: Option<[u8; 16]>,
    /// Cryptographic key id, when encrypted.
    key: Option<[u8; 16]>,
    warnings: Vec<String>,
    /// Picture size (stored) and frames as (offset, size).
    size: Option<(u32, u32)>,
    frames: Vec<(u64, u32)>,
}

fn rational(r: cfa4_mxf::Rational) -> Rational {
    Rational { num: i64::from(r.num), den: i64::from(r.den) }
}

/// Reads the track file at `path`; `Ok(None)` when it is not MXF at all.
fn read_track(path: &Path) -> Result<Option<Track>, String> {
    let src = FileSource::open(path).map_err(|e| format!("cannot be opened: {e}"))?;
    let head = src.bytes(0, 65_536 + 32).map_err(|e| format!("cannot be read: {e}"))?;
    if !cfa4_mxf::sniff(&head) {
        return Ok(None);
    }
    let peek = cfa4_mxf::peek(&src).map_err(|e| format!("is not a readable MXF file: {e}"))?;
    let asset = peek.file_packages.first().and_then(|u| u[16..].try_into().ok());
    let op_atom = peek.operational_pattern == cfa4_mxf::OperationalPattern::Atom;
    if cfa4_mxf::is_timed_text(&head) {
        let t = cfa4_mxf::open_timed_text(&src).map_err(|e| format!("is not a readable timed text track file: {e}"))?;
        return Ok(Some(Track {
            essence: Essence::TimedText,
            tracks: 1,
            op_atom,
            standard: None,
            edit_rate: rational(t.edit_rate),
            duration: t.duration,
            asset,
            key: None,
            warnings: t.warnings,
            size: None,
            frames: Vec::new(),
        }));
    }
    let f = cfa4_mxf::open(&src).map_err(|e| format!("is not a readable MXF track file: {e}"))?;
    // ST 377-1 OP-Atom label 06 0E 2B 34 04 01 01 vv 0D 01 02 01 10 …: SMPTE writes version 2,
    // MXF Interop version 1
    let label = f.operational_pattern_label.0;
    let standard = match (label[..4] == [0x06, 0x0E, 0x2B, 0x34]).then_some(label[7]) {
        Some(2) => Some(Standard::Smpte),
        Some(1) => Some(Standard::Interop),
        _ => None,
    };
    let Some(t) = f.tracks.first() else {
        return Ok(Some(Track {
            essence: Essence::Other,
            tracks: 0,
            op_atom,
            standard,
            edit_rate: Rational { num: 0, den: 1 },
            duration: None,
            asset,
            key: None,
            warnings: f.warnings,
            size: None,
            frames: Vec::new(),
        }));
    };
    let essence = match (t.kind, t.codec) {
        (TrackKind::Picture, Codec::Jpeg2000) => Essence::Picture,
        (TrackKind::Sound, Codec::Pcm) => Essence::Sound,
        _ => Essence::Other,
    };
    Ok(Some(Track {
        essence,
        tracks: f.tracks.len(),
        op_atom,
        standard,
        edit_rate: rational(t.edit_rate),
        duration: t.duration,
        asset,
        key: t.encryption.as_ref().map(|e| e.key_id),
        warnings: f.warnings.clone(),
        size: t.picture.as_ref().map(|p| (p.stored_width, p.stored_height)),
        frames: t.samples.iter().map(|s| (s.offset, s.size)).collect(),
    }))
}

/// One reel asset that names a track file of the package.
struct Use<'a> {
    cpl: &'a str,
    standard: Standard,
    reel: usize,
    asset: &'a ReelAsset,
}

impl Use<'_> {
    fn at(&self) -> Location {
        at(self.cpl, asset_element(self.reel, self.asset), Some(self.asset.pos), self.asset.id)
    }
}

fn expected(kind: AssetKind) -> Option<Essence> {
    match kind {
        AssetKind::MainPicture | AssetKind::MainStereoscopicPicture => Some(Essence::Picture),
        AssetKind::MainSound => Some(Essence::Sound),
        AssetKind::MainSubtitle | AssetKind::MainCaption | AssetKind::MainClosedCaption => Some(Essence::TimedText),
        // immersive audio and other data tracks: their essence is DCP-07's
        AssetKind::AuxData => None,
        _ => None,
    }
}

fn essence_name(e: Essence) -> &'static str {
    match e {
        Essence::Picture => "JPEG 2000 pictures",
        Essence::Sound => "PCM sound",
        Essence::TimedText => "timed text",
        Essence::Other => "other essence",
    }
}

fn hex(b: &[u8; 16]) -> String {
    Uuid(*b).to_string()
}

/// The results of the three rules; `None` for `DCP.J2K.001` when it was not asked for.
pub(crate) struct Media {
    pub mxf: (Vec<Finding>, String),
    pub j2k: Option<(Vec<Finding>, String)>,
    pub pic: (Vec<Finding>, String),
}

/// A picture track file to check frame by frame.
struct Picture<'a> {
    path: &'a str,
    file: &'a Path,
    asset: Uuid,
    stereo: bool,
    rate: Rational,
    size: Option<(u32, u32)>,
    frames: Vec<(u64, u32)>,
}

/// Checks the track files of `cpls`; `None` when cancelled.
pub(crate) fn check(cpls: &[(String, Composition)], files: &Files, codestreams: bool, control: &ValidationControl) -> Option<Media> {
    let mut mxf = Vec::new();
    let mut pic = Vec::new();
    let mut read: BTreeMap<Uuid, Result<Option<Track>, String>> = BTreeMap::new();
    let mut pictures: Vec<Picture<'_>> = Vec::new();
    let mut seen_picture: BTreeMap<Uuid, bool> = BTreeMap::new();
    let mut unreadable = Vec::new();
    let mut uses = Vec::new();
    for (path, c) in cpls {
        for (r, reel) in c.reels.iter().enumerate() {
            for a in &reel.assets {
                uses.push(Use { cpl: path, standard: c.standard, reel: r, asset: a });
            }
        }
    }
    let mut checked = 0usize;
    let total = uses.len() as u64;
    for (k, u) in uses.iter().enumerate() {
        control.report("media", k as u64, total);
        if control.is_cancelled() {
            return None;
        }
        let Some(want) = expected(u.asset.kind) else { continue };
        // a reel asset outside the package (a version file) is DCP.CPL.003's
        let Some(id) = u.asset.id else { continue };
        let Some((file_path, file)) = files.get(&id) else { continue };
        let track = read.entry(id).or_insert_with(|| read_track(file));
        let loc = || Location { file: Some(file_path.clone()), asset: Some(id), ..Location::default() };
        let picture = matches!(u.asset.kind, AssetKind::MainPicture | AssetKind::MainStereoscopicPicture);
        let t = match track {
            Ok(Some(t)) if !picture || t.essence == Essence::Picture => t,
            // Interop subtitles are XML (DCP-08)
            Ok(None) if want == Essence::TimedText && u.standard == Standard::Interop => continue,
            other => {
                let m = match other {
                    Err(e) => format!("'{file_path}' {e}"),
                    Ok(None) => format!("'{file_path}' is not an MXF file"),
                    Ok(Some(t)) => format!("'{file_path}' holds {}, but the reel asset is a {:?}", essence_name(t.essence), u.asset.kind),
                };
                mxf.push(Finding::new(MXF_TRACK_FILES, Status::Fail, m).at(loc()));
                if picture {
                    // without pictures there are no codestreams or picture format to check
                    let why = format!("'{file_path}' is not a readable picture track file (see DCP.MXF.001)");
                    unreadable.push(Finding::new(J2K_CODESTREAM, Status::Unchecked, why.clone()).at(loc()));
                    pic.push(Finding::new(PIC_FORMAT, Status::Unchecked, why).at(u.at()));
                }
                continue;
            }
        };
        checked += 1;
        let mut fail = |m: String| mxf.push(Finding::new(MXF_TRACK_FILES, Status::Fail, m).at(u.at()));
        if !t.op_atom {
            fail(format!("'{file_path}' is not OP-Atom (ST 429-3)"));
        }
        if t.tracks != 1 {
            fail(format!("'{file_path}' has {} essence tracks; a track file has one", t.tracks));
        }
        if t.essence != want {
            fail(format!("'{file_path}' holds {}, but the reel asset is a {:?}", essence_name(t.essence), u.asset.kind));
        }
        if let Some(s) = t.standard
            && s != u.standard
        {
            fail(format!("'{file_path}' has {} labels in a {:?} composition", if s == Standard::Smpte { "SMPTE" } else { "MXF Interop" }, u.standard));
        }
        if let Some(rate) = u.asset.edit_rate
            && !rate.same_value(t.edit_rate)
        {
            fail(format!("'{file_path}' has the edit rate {}, the reel asset {rate}", t.edit_rate));
        }
        match (u.asset.intrinsic_duration, t.duration) {
            (Some(d), Some(fd)) if i64::try_from(d).ok() != Some(fd) => fail(format!("'{file_path}' is {fd} edit units long, the IntrinsicDuration is {d}")),
            (Some(_), None) => fail(format!("'{file_path}' does not say how long it is")),
            _ => {}
        }
        match t.asset {
            Some(a) if a != id.0 => fail(format!("'{file_path}' is the track file {}, but the reel asset names {id}", hex(&a))),
            None => fail(format!("'{file_path}' has no file package")),
            _ => {}
        }
        match (u.asset.key_id, t.key) {
            (Some(k), None) => fail(format!("the reel asset has the KeyId {k}, but '{file_path}' is not encrypted")),
            (None, Some(k)) => fail(format!("'{file_path}' is encrypted with the key {}, but the reel asset has no KeyId", hex(&k))),
            (Some(k), Some(fk)) if k.0 != fk => fail(format!("'{file_path}' is encrypted with the key {}, the reel asset names {k}", hex(&fk))),
            _ => {}
        }
        for w in &t.warnings {
            mxf.push(Finding::new(MXF_TRACK_FILES, Status::Warning, format!("'{file_path}': {w}")).at(loc()));
        }
        if t.essence != Essence::Picture || !matches!(u.asset.kind, AssetKind::MainPicture | AssetKind::MainStereoscopicPicture) {
            continue;
        }
        // ST 429-10: a stereoscopic file has two frames (left, right) per edit unit
        let stereo = u.asset.kind == AssetKind::MainStereoscopicPicture;
        let per_unit = if stereo { 2 } else { 1 };
        if let Some(d) = t.duration
            && !t.frames.is_empty()
            && u64::try_from(d).ok().map(|d| d * per_unit) != Some(t.frames.len() as u64)
        {
            let what = if t.frames.len() as u64 == u64::try_from(d).unwrap_or(0) * 2 { "a stereoscopic file" } else { "frames that do not match its duration" };
            mxf.push(
                Finding::new(
                    MXF_TRACK_FILES,
                    Status::Fail,
                    format!("'{file_path}' has {} frames for {d} edit units ({what}) in a {:?}", t.frames.len(), u.asset.kind),
                )
                .at(u.at()),
            );
        }
        pic.extend(picture_format(u, file_path, t));
        if seen_picture.insert(id, stereo).is_none() {
            if t.key.is_some() {
                continue;
            }
            pictures.push(Picture { path: file_path, file, asset: id, stereo, rate: t.edit_rate, size: t.size, frames: t.frames.clone() });
        }
    }
    let encrypted: Vec<&String> = read
        .iter()
        .filter(|(id, t)| seen_picture.contains_key(id) && matches!(t, Ok(Some(t)) if t.key.is_some()))
        .filter_map(|(id, _)| files.get(id).map(|(p, _)| p))
        .collect();
    let j2k = if codestreams {
        let mut out = unreadable;
        for p in &encrypted {
            out.push(
                Finding::new(J2K_CODESTREAM, Status::Unchecked, format!("'{p}' is encrypted; its codestreams need the content key"))
                    .at(Location::file(p.as_str())),
            );
        }
        let mut frames = 0u64;
        let total: u64 = pictures.iter().map(|p| p.frames.len() as u64).sum();
        for p in &pictures {
            let (f, n) = frames_of(p, frames, total, control)?;
            out.extend(f);
            frames += n;
        }
        Some((out, format!("{frames} frames in {} picture track files follow the DCI JPEG 2000 profile", pictures.len())))
    } else {
        None
    };
    let pictures_seen = seen_picture.len();
    Some(Media {
        mxf: (mxf, format!("{checked} track files of the declared kind, rate and duration")),
        j2k,
        pic: (pic, format!("{pictures_seen} picture track files in a cinema format")),
    })
}

/// `DCP.PIC.001` for one picture reel asset.
fn picture_format(u: &Use<'_>, file_path: &str, t: &Track) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |status: Status, m: String| out.push(Finding::new(PIC_FORMAT, status, m).at(u.at()));
    let stereo = u.asset.kind == AssetKind::MainStereoscopicPicture;
    let mut four_k = false;
    if let Some((w, h)) = t.size {
        four_k = w > 2048 || h > 1080;
        const STANDARD: &[(u32, u32)] = &[(2048, 1080), (1998, 1080), (2048, 858), (4096, 2160), (3996, 2160), (4096, 1716)];
        if w > 4096 || h > 2160 {
            push(Status::Fail, format!("'{file_path}' is {w} × {h}, larger than the 4K container (4096 × 2160)"));
        } else if !STANDARD.contains(&(w, h)) {
            push(Status::Warning, format!("'{file_path}' is {w} × {h}, not a DCI image size (flat, scope or full container)"));
        }
        match &u.asset.screen_aspect_ratio {
            Some(ScreenAspectRatio::Ratio(r)) if i128::from(r.num) * i128::from(h) != i128::from(r.den) * i128::from(w) => {
                push(Status::Warning, format!("the ScreenAspectRatio {r} is not the {w} × {h} of '{file_path}'"))
            }
            Some(ScreenAspectRatio::Decimal(d)) => match d.trim().parse::<f64>() {
                Ok(v) if (v - f64::from(w) / f64::from(h)).abs() > 0.01 => {
                    push(Status::Warning, format!("the ScreenAspectRatio {d} is not the {w} × {h} of '{file_path}'"))
                }
                Ok(_) => {}
                Err(_) => push(Status::Warning, format!("the ScreenAspectRatio '{d}' is not a number")),
            },
            _ => {}
        }
        if stereo && four_k {
            push(Status::Warning, format!("'{file_path}' is 4K stereoscopic, which the DCI specification does not define"));
        }
    }
    let Some(rate) = u.asset.edit_rate else { return out };
    if let Some(fr) = u.asset.frame_rate {
        let want = if stereo { Rational { num: rate.num * 2, den: rate.den } } else { rate };
        if !fr.same_value(want) {
            let why = if stereo { "twice the edit rate for stereoscopic pictures" } else { "the edit rate" };
            push(Status::Fail, format!("the FrameRate {fr} is not {why} ({rate})"));
        }
    }
    let fps = (rate.den > 0 && rate.num % rate.den == 0).then(|| rate.num / rate.den);
    let (core, optional): (&[i64], &[i64]) = match (u.standard, four_k, stereo) {
        (Standard::Smpte, false, false) => (&[24, 48], &[25, 30, 50, 60, 96, 100, 120]),
        (Standard::Smpte, true, false) => (&[24], &[25, 30, 48, 50, 60]),
        (Standard::Smpte, _, true) => (&[24], &[25, 30, 48, 50, 60]),
        (Standard::Interop, false, false) => (&[24, 48], &[25]),
        (Standard::Interop, _, _) => (&[24], &[25, 48]),
    };
    let what = format!("{}{}", if four_k { "4K" } else { "2K" }, if stereo { " stereoscopic" } else { "" });
    match fps {
        Some(f) if core.contains(&f) => {}
        Some(f) if optional.contains(&f) => {
            push(Status::Warning, format!("{f} fps {what} is in the {:?} standard but not every cinema server plays it", u.standard))
        }
        _ => push(Status::Fail, format!("the edit rate {rate} is not a {:?} cinema frame rate for {what}", u.standard)),
    }
    out
}

/// Repeated problems are reported once per file and message, with the first frame and a count.
#[derive(Default)]
struct Tally {
    order: Vec<(Status, String)>,
    seen: BTreeMap<(Status, String), (u64, u64)>,
}

impl Tally {
    fn add(&mut self, status: Status, message: String, frame: u64) {
        let e = self.seen.entry((status, message.clone())).or_insert_with(|| {
            self.order.push((status, message));
            (frame, 0)
        });
        e.1 += 1;
    }

    fn findings(self, p: &Picture<'_>) -> Vec<Finding> {
        let loc = Location { file: Some(p.path.to_string()), asset: Some(p.asset), ..Location::default() };
        self.order
            .into_iter()
            .map(|k| {
                let (first, n) = self.seen[&k];
                let more = if n > 1 { format!(" ({n} frames)") } else { String::new() };
                Finding::new(J2K_CODESTREAM, k.0, format!("'{}' frame {first}: {}{more}", p.path, k.1)).at(loc.clone())
            })
            .collect()
    }
}

/// The DCI limits in bytes, (frame, component), at the edit rate `rate`: 250 Mbit/s for a frame
/// and 200 Mbit/s for a component at the image rate, which is twice the edit rate for stereoscopic
/// pictures. `None` for a rate that is not positive.
pub(crate) fn dci_budget(rate: Rational, stereo: bool) -> Option<(i64, i64)> {
    let images = rate.num.checked_mul(if stereo { 2 } else { 1 }).filter(|i| *i > 0 && rate.den > 0)?;
    let at = |bits_per_second: i64| i64::try_from(i128::from(bits_per_second / 8) * i128::from(rate.den) / i128::from(images)).ok();
    Some((at(250_000_000)?, at(200_000_000)?))
}

/// `DCP.J2K.001` for every frame of one picture track file; `None` when cancelled. Returns the
/// findings and the frames checked.
fn frames_of(p: &Picture<'_>, done: u64, total: u64, control: &ValidationControl) -> Option<(Vec<Finding>, u64)> {
    let mut tally = Tally::default();
    let src = match FileSource::open(p.file) {
        Ok(s) => s,
        Err(e) => {
            let f = Finding::new(J2K_CODESTREAM, Status::Fail, format!("'{}' cannot be opened: {e}", p.path)).at(Location::file(p.path));
            return Some((vec![f], 0));
        }
    };
    let (max_frame, max_component) = dci_budget(p.rate, p.stereo).unzip();
    // above 48 images a second the DCI limits are not the only ones in use (HFR)
    let images = (if p.stereo { 2 } else { 1 }) * p.rate.num;
    let over = if images <= 48 * p.rate.den { Status::Fail } else { Status::Warning };
    let mut first_size = None;
    for (k, &(offset, size)) in p.frames.iter().enumerate() {
        let k = k as u64;
        if k.is_multiple_of(256) {
            control.report("media", done + k, total);
            if control.is_cancelled() {
                return None;
            }
        }
        if let Some(max) = max_frame
            && i64::from(size) > max
        {
            tally.add(over, format!("{size} bytes, over the DCI {max} bytes a frame at this rate"), k);
        }
        let read = |at: u64, n: usize| src.bytes(offset + at, n.min(usize::try_from(u64::from(size).saturating_sub(at)).unwrap_or(0)));
        let mut head = match read(0, codestream::HEAD) {
            Ok(b) => b,
            Err(e) => {
                tally.add(Status::Fail, format!("cannot be read: {e}"), k);
                continue;
            }
        };
        let mut main = codestream::main_header(&head);
        if matches!(main, Ok(None)) && size as usize > head.len() {
            head = read(0, codestream::MAX_HEAD).unwrap_or_default();
            main = codestream::main_header(&head);
        }
        let main = match main {
            Ok(Some(m)) => m,
            Ok(None) => {
                tally.add(Status::Fail, "the main header is cut short".into(), k);
                continue;
            }
            Err(e) => {
                tally.add(Status::Fail, e, k);
                continue;
            }
        };
        let header = match cfa4_j2k::parse_header(&head[..main.end + 2]) {
            Ok(h) => h,
            Err(e) => {
                tally.add(Status::Fail, format!("the main header cannot be read: {e}"), k);
                continue;
            }
        };
        for m in codestream::profile_problems(&header, &main) {
            tally.add(Status::Fail, m, k);
        }
        for m in codestream::header_remarks(&header, &main) {
            tally.add(Status::Warning, m, k);
        }
        let dims = (header.width(), header.height());
        if first_size.is_none() {
            first_size = Some(dims);
        }
        if let Some((w, h)) = p.size
            && dims != (w, h)
        {
            tally.add(Status::Fail, format!("the codestream is {} × {}, the MXF picture descriptor {w} × {h}", dims.0, dims.1), k);
        } else if first_size != Some(dims) {
            tally.add(Status::Fail, "the image size changes within the file".into(), k);
        }
        let parts = match codestream::tile_parts(u64::from(size), main.end as u64, read) {
            Ok(t) => t,
            Err(e) => {
                tally.add(Status::Fail, format!("cannot be read: {e}"), k);
                continue;
            }
        };
        for m in parts.problems {
            tally.add(Status::Fail, m, k);
        }
        if parts.no_eoc {
            tally.add(Status::Warning, "the codestream does not end with EOC after the last tile-part".into(), k);
        }
        let four_k = codestream::four_k(&header);
        let want = if four_k { 6 } else { 3 };
        if parts.lengths.len() != want {
            tally.add(
                Status::Fail,
                format!(
                    "{} tile-parts; the {} profile has {want}, one per component{}",
                    parts.lengths.len(),
                    if four_k { "4K" } else { "2K" },
                    if four_k { " and resolution group" } else { "" }
                ),
                k,
            );
        } else if let (Some(c), Some(max)) = (codestream::component_bytes(&parts.lengths, four_k), max_component) {
            for (i, b) in c.iter().enumerate() {
                if i64::try_from(*b).unwrap_or(i64::MAX) > max {
                    tally.add(over, format!("component {i} is {b} bytes, over the DCI {max} bytes at this rate"), k);
                }
            }
        }
    }
    Some((tally.findings(p), p.frames.len() as u64))
}
