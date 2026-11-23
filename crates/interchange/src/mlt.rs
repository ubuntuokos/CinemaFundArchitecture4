//! MLT XML import: Kdenlive projects (`.kdenlive`) and, on the same core, Shotcut projects
//! (`.mlt`, SC-5). Import only: there is no export (owner decision 2026-10-25, 2026-11-22), and no
//! MLT library is involved; `melt` is a test oracle only.
//!
//! The core reads the dialect-independent part of the format: profile, clock-time values,
//! producers and chains, playlists (`entry` and `blank`), tractors and their tracks. The
//! dialect parts (`kdenlive:*`, `shotcut:*` properties) only decide how tracks are typed. What
//! the importer does not map yet (effects, compositions, nested sequences, unknown generators)
//! is listed in the [`Report`]; it never makes the import fail.
//!
//! Facts about the format are those confirmed in
//! `docs/plans/CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-0-REPORT.md`:
//! - a clock value `HH:MM:SS.mmm` is read as `floor(fps·h·3600) + floor(fps·min·60) +
//!   lrint(fps·s)` frames (T3), a plain number is a frame count;
//! - `in` / `out` of an `entry` are frame indices and `out` is the last frame included, so the
//!   length is `out − in + 1`; the length of a `blank` is a frame count (T4);
//! - the importer understands both `<producer>` and `<chain>` (T14).
//!
//! The XML is parsed with the shared safe parser ([`crate::xml::parse`]): no internal DTD
//! subsets, no external entities.

use std::collections::HashMap;

use cfa4_media::Generator;
use cfa4_project::{ItemId, Sequence, TrackKind};
use cfa4_time::{FrameRate, Tick};
use roxmltree::Node;

use crate::common::{Builder, MediaSpec, file_name, resolve_path, settings_for};
use crate::xml::{child, children, elements, parse, text};
use crate::{Error, Format, ImportOptions, Imported, Report, Result};

/// Most playlist entries read from one document.
const MAX_ENTRIES: usize = 200_000;
/// Deepest producer / tractor nesting followed.
const MAX_DEPTH: usize = 16;

/// Whether `text` is an MLT XML document (root element `mlt`).
pub(crate) fn sniff(head: &str) -> bool {
    let mut rest = head;
    // skip the XML declaration, comments and a doctype, then look at the root element
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            match after.find("?>") {
                Some(i) => rest = &after[i + 2..],
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix("<!--") {
            match after.find("-->") {
                Some(i) => rest = &after[i + 3..],
                None => return false,
            }
        } else if rest.starts_with("<!") {
            match rest.find('>') {
                Some(i) => rest = &rest[i + 1..],
                None => return false,
            }
        } else {
            break;
        }
    }
    rest.strip_prefix("<mlt").is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/'))
}

/// Which program wrote the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    Kdenlive,
    Shotcut,
    Plain,
}

/// A property by name: a `<property name=…>` child, else an attribute of the same name.
fn prop<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    children(n, "property").find(|p| p.attribute("name") == Some(name)).map(text).or_else(|| n.attribute(name))
}

/// Whether any element below `n` has a property whose name starts with `prefix`.
fn has_prefixed_property(n: Node<'_, '_>, prefix: &str) -> bool {
    n.descendants().any(|d| d.is_element() && d.tag_name().name() == "property" && d.attribute("name").is_some_and(|k| k.starts_with(prefix)))
}

struct Imp<'a, 'd, 'r> {
    b: Builder,
    opts: &'r ImportOptions,
    report: &'r mut Report,
    rate: FrameRate,
    fps: f64,
    dialect: Dialect,
    /// The `root` attribute of `<mlt>`: the directory relative resources are stored against.
    root: Option<String>,
    /// Producers and chains, playlists and tractors by `id`.
    ids: HashMap<&'a str, Node<'a, 'd>>,
    /// Media items already made, by producer id.
    media: HashMap<String, Option<(ItemId, f64)>>,
    entries: usize,
}

pub(crate) fn import(text: &str, opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    let doc = parse(text).map_err(|e| Error::parse(Format::Mlt, e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "mlt" {
        return Err(Error::parse(Format::Mlt, "the root element is not <mlt>"));
    }
    let profile = child(root, "profile").ok_or_else(|| Error::parse(Format::Mlt, "no <profile> element"))?;
    let num: i64 = profile.attribute("frame_rate_num").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let den: i64 = profile.attribute("frame_rate_den").and_then(|v| v.trim().parse().ok()).unwrap_or(1);
    if num <= 0 || den <= 0 {
        return Err(Error::parse(Format::Mlt, "the profile has no valid frame rate"));
    }
    let rate = FrameRate::new(num, den);
    let size = |name: &str| profile.attribute(name).and_then(|v| v.trim().parse::<u32>().ok()).filter(|v| *v > 0);
    let (width, height) = (size("width").unwrap_or(1920), size("height").unwrap_or(1080));

    let dialect = if has_prefixed_property(root, "kdenlive:") {
        Dialect::Kdenlive
    } else if has_prefixed_property(root, "shotcut") {
        Dialect::Shotcut
    } else {
        Dialect::Plain
    };
    let mut ids = HashMap::new();
    for n in elements(root) {
        if matches!(n.tag_name().name(), "producer" | "chain" | "playlist" | "tractor")
            && let Some(id) = n.attribute("id")
        {
            ids.insert(id, n);
        }
    }
    let name = root
        .attribute("title")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| opts.name.clone())
        .unwrap_or_else(|| "Imported MLT".into());
    let mut imp = Imp {
        b: Builder::new(&name),
        opts,
        report,
        rate,
        fps: rate.as_f64(),
        dialect,
        root: root.attribute("root").map(str::trim).filter(|r| !r.is_empty()).map(str::to_string),
        ids,
        media: HashMap::new(),
        entries: 0,
    };
    imp.report.info(match dialect {
        Dialect::Kdenlive => format!("Kdenlive project (MLT {})", root.attribute("version").unwrap_or("?")),
        Dialect::Shotcut => format!("Shotcut project (MLT {})", root.attribute("version").unwrap_or("?")),
        Dialect::Plain => format!("MLT project (MLT {})", root.attribute("version").unwrap_or("?")),
    });

    // Sequences: tractors no other tractor uses as a track and no playlist places as a clip.
    let mut used: Vec<&str> = Vec::new();
    for n in elements(root) {
        match n.tag_name().name() {
            "tractor" => used.extend(children(n, "track").filter_map(|t| t.attribute("producer"))),
            "playlist" if n.attribute("id") != Some("main_bin") => used.extend(children(n, "entry").filter_map(|e| e.attribute("producer"))),
            _ => {}
        }
    }
    let roots: Vec<Node<'_, '_>> =
        elements(root).filter(|n| n.tag_name().name() == "tractor" && n.attribute("id").is_none_or(|id| !used.contains(&id))).collect();
    if roots.is_empty() {
        return Err(Error::Empty);
    }
    // Kdenlive 22.12 and later wrap the timeline in a project tractor whose only track is the sequence
    let roots: Vec<Node<'_, '_>> = roots.into_iter().map(|r| imp.unwrap_project_tractor(r)).collect();
    for (i, tractor) in roots.iter().enumerate() {
        let seq_name = prop(*tractor, "kdenlive:sequenceproperties.sequencename")
            .or_else(|| prop(*tractor, "shotcut:name"))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| if i == 0 { name.clone() } else { format!("{name} {}", i + 1) });
        let id = imp.sequence(*tractor, &seq_name, width, height);
        imp.b.top.push(id);
    }
    Ok(imp.b.finish())
}

impl<'a, 'd> Imp<'a, 'd, '_> {
    /// Frames of a clock value or frame count (T3).
    fn frames(&mut self, v: &str) -> Option<i64> {
        let v = v.trim();
        if v.is_empty() {
            return None;
        }
        if let Some((hms, ms)) = v.split_once('.').or(Some((v, ""))).filter(|_| v.contains(':')) {
            let mut parts = hms.split(':');
            let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() {
                // HH:MM:SS:FF timecode
                let f: i64 = hms.rsplit(':').next()?.parse().ok()?;
                let (h, m, s): (i64, i64, i64) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
                return Some(((self.fps * (h * 3600) as f64).floor() + (self.fps * (m * 60) as f64).floor() + (self.fps * s as f64).round()) as i64 + f);
            }
            let (h, m): (f64, f64) = (h.parse().ok()?, m.parse().ok()?);
            let secs: f64 = format!("{s}.{}", if ms.is_empty() { "0" } else { ms }).parse().ok()?;
            let total = (self.fps * h * 3600.0).floor() + (self.fps * m * 60.0).floor() + (self.fps * secs).round();
            return (total.is_finite() && total.abs() < 1e12).then_some(total as i64);
        }
        v.parse::<i64>().ok().or_else(|| v.parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f.round() as i64))
    }

    fn frames_attr(&mut self, n: Node<'a, 'd>, name: &str) -> Option<i64> {
        let v = n.attribute(name)?;
        let f = self.frames(v);
        if f.is_none() {
            self.report.warn(format!("a time value {v:?} could not be read"));
        }
        f
    }

    /// The sequence inside a project tractor: a tractor with one track, itself a tractor of tracks
    /// that are tractors (Kdenlive's track containers), is only a wrapper.
    fn unwrap_project_tractor(&self, mut tractor: Node<'a, 'd>) -> Node<'a, 'd> {
        for _ in 0..MAX_DEPTH {
            let mut tracks = children(tractor, "track");
            let (Some(only), None) = (tracks.next(), tracks.next()) else { break };
            let inner = only.attribute("producer").and_then(|p| self.ids.get(p)).copied().filter(|n| n.tag_name().name() == "tractor");
            match inner {
                Some(i)
                    if children(i, "track")
                        .any(|t| t.attribute("producer").and_then(|p| self.ids.get(p)).is_some_and(|n| n.tag_name().name() == "tractor")) =>
                {
                    tractor = i
                }
                _ => break,
            }
        }
        tractor
    }

    fn sequence(&mut self, tractor: Node<'a, 'd>, name: &str, width: u32, height: u32) -> ItemId {
        let id = self.b.reserve_sequence(name, settings_for(self.rate, width, height, false), None);
        let mut seq = crate::common::empty_sequence(settings_for(self.rate, width, height, false));
        let mut index = [0usize; 2]; // video, audio
        for t in children(tractor, "track") {
            let Some(pid) = t.attribute("producer") else { continue };
            let Some(&node) = self.ids.get(pid) else {
                self.report.warn(format!("a track refers to the missing producer {pid:?}"));
                continue;
            };
            match node.tag_name().name() {
                // a Kdenlive track: a tractor of two playlists (the second holds the partner of a mix)
                "tractor" => {
                    let kind = if prop(node, "kdenlive:audio_track") == Some("1") { TrackKind::Audio } else { TrackKind::Video };
                    let track_name = prop(node, "kdenlive:track_name").filter(|n| !n.is_empty()).map(str::to_string);
                    let at = self.next_track(&mut seq, kind, &mut index, track_name);
                    for inner in children(node, "track") {
                        let Some(inner_id) = inner.attribute("producer") else { continue };
                        match self.ids.get(inner_id) {
                            Some(&p) if p.tag_name().name() == "playlist" => self.playlist(p, &mut seq, kind, at),
                            _ => self.report.warn(format!("a track refers to the missing playlist {inner_id:?}")),
                        }
                    }
                    // a mix between two clips of one track lives in the track's own tractor
                    self.transitions(node);
                }
                // a Shotcut track, or any plain MLT track: one playlist
                "playlist" => {
                    if pid == "main_bin" {
                        continue;
                    }
                    let hidden_picture = t.attribute("hide").is_some_and(|h| h == "video" || h == "both") && prop(node, "shotcut:video") != Some("1");
                    let kind = if prop(node, "shotcut:audio") == Some("1") || prop(node, "kdenlive:audio_track") == Some("1") || hidden_picture {
                        TrackKind::Audio
                    } else {
                        TrackKind::Video
                    };
                    let track_name = prop(node, "shotcut:name").filter(|n| !n.is_empty()).map(str::to_string);
                    let at = self.next_track(&mut seq, kind, &mut index, track_name);
                    self.playlist(node, &mut seq, kind, at);
                }
                // the black background track of both programs
                _ => {}
            }
        }
        self.transitions(tractor);
        self.b.put_sequence(id, seq);
        id
    }

    fn next_track(&mut self, seq: &mut Sequence, kind: TrackKind, index: &mut [usize; 2], name: Option<String>) -> usize {
        let slot = if kind == TrackKind::Video { 0 } else { 1 };
        let at = *index.get(slot).unwrap_or(&0);
        if let Some(c) = index.get_mut(slot) {
            *c += 1;
        }
        if !self.b.ensure_tracks(seq, kind, at + 1) {
            self.report.warn("tracks past the supported track count were not imported");
        }
        if let (Some(name), Some(t)) = (name, seq.tracks_mut(kind).get_mut(at)) {
            t.name = name;
        }
        at
    }

    /// Lay the entries of a playlist on track `at` of `kind`.
    fn playlist(&mut self, playlist: Node<'a, 'd>, seq: &mut Sequence, kind: TrackKind, at: usize) {
        let mut cursor: i64 = 0;
        for e in elements(playlist) {
            match e.tag_name().name() {
                "blank" => {
                    let len = e.attribute("length").and_then(|v| self.frames(v)).unwrap_or(0);
                    cursor = cursor.saturating_add(len.max(0));
                }
                "entry" => {
                    self.entries += 1;
                    if self.entries > MAX_ENTRIES {
                        self.report.warn(format!("more than {MAX_ENTRIES} playlist entries: the rest was not imported"));
                        return;
                    }
                    let Some(pid) = e.attribute("producer") else { continue };
                    let Some(&prod) = self.ids.get(pid) else {
                        self.report.warn(format!("an entry refers to the missing producer {pid:?}"));
                        continue;
                    };
                    let first = self.frames_attr(e, "in").unwrap_or(0).max(0);
                    let last = match self.frames_attr(e, "out") {
                        Some(o) => Some(o),
                        None => self.frames_attr(prod, "out"),
                    };
                    let Some(last) = last.filter(|l| *l >= first) else {
                        self.report.warn(format!("the entry of {pid:?} has no usable in/out range"));
                        continue;
                    };
                    let frames = last - first + 1;
                    let start = cursor;
                    cursor = cursor.saturating_add(frames);
                    let Some((item, speed)) = self.media_of(pid, prod, kind, 0) else { continue };
                    let name = self.clip_name(prod, item);
                    let mut ti = self.b.clip(item, kind, &name, self.rate.tick_of(start), self.rate.tick_of(frames), Tick::ZERO);
                    ti.source_in = crate::common::scaled(self.rate.tick_of(first), speed);
                    if (speed - 1.0).abs() > 1e-9 {
                        ti.speed = speed;
                    }
                    self.effects(e, prod);
                    match self.b.place(seq, kind, at, ti) {
                        None => self.report.warn("a clip past the supported track count was not imported"),
                        Some(placed) if placed != at => self.report.warn("clips that overlap on one track (a mix) were placed on an extra track"),
                        Some(_) => {}
                    }
                }
                // nested playlists are not mixes, only the entry and blank elements place clips
                _ => {}
            }
        }
    }

    fn clip_name(&self, prod: Node<'a, 'd>, item: ItemId) -> String {
        prop(prod, "kdenlive:clipname")
            .or_else(|| prop(prod, "shotcut:caption"))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .or_else(|| self.b.p.item(item).map(|i| i.name.clone()))
            .unwrap_or_else(|| "Clip".into())
    }

    /// The media item behind a producer, and its playback speed (a `timewarp` producer).
    fn media_of(&mut self, pid: &str, prod: Node<'a, 'd>, kind: TrackKind, depth: usize) -> Option<(ItemId, f64)> {
        if let Some(found) = self.media.get(pid) {
            return *found;
        }
        let made = self.make_media(pid, prod, kind, depth);
        self.media.insert(pid.to_string(), made);
        made
    }

    fn make_media(&mut self, pid: &str, prod: Node<'a, 'd>, kind: TrackKind, depth: usize) -> Option<(ItemId, f64)> {
        if depth > MAX_DEPTH {
            self.report.warn("producers nested too deeply: a clip was not imported");
            return None;
        }
        let service = prop(prod, "mlt_service").unwrap_or("").to_string();
        let resource = prop(prod, "resource").unwrap_or("").to_string();
        let length = self.frames_attr(prod, "out").map(|o| o + 1).or_else(|| prop(prod, "length").and_then(|l| l.parse::<i64>().ok())).unwrap_or(0).max(0);
        let spec = MediaSpec { duration: Some(self.rate.tick_of(length)), ..Default::default() };
        match service.as_str() {
            "avformat" | "avformat-novalidate" | "qimage" | "pixbuf" | "" if !resource.is_empty() => {
                let path = self.path(&resource);
                let name = file_name(&path).to_string();
                Some((self.b.file_media(&path, &name, &path, &spec, None), 1.0))
            }
            "timewarp" => {
                let speed = prop(prod, "warp_speed").and_then(|s| s.parse::<f64>().ok()).filter(|s| s.is_finite() && s.abs() > 1e-6);
                let source =
                    prop(prod, "warp_resource").map(str::to_string).or_else(|| resource.split_once(':').map(|(_, r)| r.to_string())).filter(|r| !r.is_empty());
                match (speed, source) {
                    (Some(speed), Some(source)) => {
                        let path = self.path(&source);
                        let name = file_name(&path).to_string();
                        let full = MediaSpec { duration: Some(self.rate.tick_of((length as f64 * speed.abs()).round() as i64)), ..Default::default() };
                        Some((self.b.file_media(&path, &name, &path, &full, None), speed.abs()))
                    }
                    _ => {
                        self.report.warn(format!("the speed clip {pid:?} has no usable speed or source"));
                        None
                    }
                }
            }
            "color" | "colour" => {
                let g = color_generator(&resource);
                let name = g.label();
                Some((self.b.generator_media(&format!("color:{resource}"), &name, g, &spec, None), 1.0))
            }
            other => {
                let label = if other.is_empty() { "an unknown producer".to_string() } else { format!("the producer service {other:?}") };
                if kind == TrackKind::Audio {
                    self.report.warn(format!("{label} is not supported: its clip was left out"));
                    return None;
                }
                self.report.warn(format!("{label} is not supported: a transparent placeholder was used"));
                let g = Generator::TransparentVideo;
                let name = format!("{} (unsupported)", if other.is_empty() { "Producer" } else { other });
                Some((self.b.generator_media(&format!("unsupported:{pid}"), &name, g, &spec, None), 1.0))
            }
        }
    }

    /// A media path as the program stores it: absolute, or relative to the `root` attribute or the
    /// document's directory.
    fn path(&self, resource: &str) -> String {
        let base = self.root.as_deref().filter(|r| crate::common::is_absolute(r)).or(self.opts.base_dir.as_deref());
        resolve_path(resource, base)
    }

    /// Report the effects of a clip: none are mapped yet.
    fn effects(&mut self, entry: Node<'a, 'd>, prod: Node<'a, 'd>) {
        for f in children(entry, "filter").chain(children(prod, "filter")) {
            if prop(f, "disable") == Some("1") || prop(f, "internal_added").is_some() {
                continue;
            }
            let service = prop(f, "kdenlive_id").or_else(|| prop(f, "shotcut:filter")).or_else(|| prop(f, "mlt_service")).unwrap_or("?");
            self.report.warn(format!("the effect {service:?} is not imported yet"));
        }
    }

    /// Report the compositions and transitions of a sequence: none are mapped yet.
    fn transitions(&mut self, tractor: Node<'a, 'd>) {
        for t in children(tractor, "transition") {
            if prop(t, "internal_added").is_some() || prop(t, "disable") == Some("1") {
                continue;
            }
            let service = prop(t, "kdenlive_id").or_else(|| prop(t, "mlt_service")).unwrap_or("?");
            if matches!(service, "mix" | "qtblend" | "frei0r.cairoblend" | "movit.overlay" | "composite") && self.dialect != Dialect::Kdenlive {
                continue;
            }
            self.report.warn(format!("the transition {service:?} is not imported yet"));
        }
    }
}

/// A colour producer's resource (`#rrggbb`, `#aarrggbb`, `0xrrggbbaa`, a few names) as a generator.
fn color_generator(resource: &str) -> Generator {
    let r = resource.trim();
    let hex = r.strip_prefix('#').or_else(|| r.strip_prefix("0x")).or_else(|| r.strip_prefix("0X"));
    let named = match r.to_ascii_lowercase().as_str() {
        "black" => Some([0.0, 0.0, 0.0, 1.0]),
        "white" => Some([1.0, 1.0, 1.0, 1.0]),
        "red" => Some([1.0, 0.0, 0.0, 1.0]),
        "green" => Some([0.0, 0.5, 0.0, 1.0]),
        "blue" => Some([0.0, 0.0, 1.0, 1.0]),
        _ => None,
    };
    if let Some(c) = named {
        return if c == [0.0, 0.0, 0.0, 1.0] { Generator::BlackVideo } else { Generator::ColorMatte { color: c } };
    }
    let Some(hex) = hex else { return Generator::BlackVideo };
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| f32::from(v) / 255.0);
    // 0xRRGGBBAA (MLT) or #RRGGBB / #AARRGGBB (Kdenlive, Qt)
    let color = match hex.len() {
        6 => (|| Some([byte(0)?, byte(2)?, byte(4)?, 1.0]))(),
        8 if r.starts_with('#') => (|| Some([byte(2)?, byte(4)?, byte(6)?, byte(0)?]))(),
        8 => (|| Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]))(),
        _ => None,
    };
    match color {
        Some(color) => Generator::ColorMatte { color },
        None => Generator::BlackVideo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_the_root_element() {
        assert!(sniff("<?xml version='1.0' encoding='utf-8'?>\n<mlt LC_NUMERIC=\"C\">"));
        assert!(sniff("<mlt>"));
        assert!(!sniff("<mlt-other>"));
        assert!(!sniff("<xmeml version=\"4\"><mlt>"));
        assert!(!sniff("<?xml version='1.0'"));
    }

    fn imp_with_fps(num: i64, den: i64) -> (FrameRate, f64) {
        let r = FrameRate::new(num, den);
        (r, r.as_f64())
    }

    #[test]
    fn clock_values_follow_the_mlt_formula() {
        // T3: floor(fps·h·3600) + floor(fps·min·60) + lrint(fps·s)
        let (rate, fps) = imp_with_fps(30000, 1001);
        let mut report = Report::default();
        let opts = ImportOptions::default();
        let mut imp = Imp {
            b: Builder::new("t"),
            opts: &opts,
            report: &mut report,
            rate,
            fps,
            dialect: Dialect::Plain,
            root: None,
            ids: HashMap::new(),
            media: HashMap::new(),
            entries: 0,
        };
        assert_eq!(imp.frames("00:00:01.000"), Some(30));
        assert_eq!(imp.frames("00:01:00.000"), Some(1798));
        assert_eq!(imp.frames("25"), Some(25));
        assert_eq!(imp.frames("  "), None);
        assert_eq!(imp.frames("nonsense"), None);
        assert_eq!(imp.frames("1:2"), None);
    }

    #[test]
    fn color_resources() {
        assert!(matches!(color_generator("black"), Generator::BlackVideo));
        assert!(matches!(color_generator("#ff0000"), Generator::ColorMatte { color } if color == [1.0, 0.0, 0.0, 1.0]));
        assert!(matches!(color_generator("0x00ff00ff"), Generator::ColorMatte { color } if color == [0.0, 1.0, 0.0, 1.0]));
        assert!(matches!(color_generator("gibberish"), Generator::BlackVideo));
    }
}
