//! `cfa4-cli codecs`: what this machine decodes and encodes, for the Qt window's Settings ▸
//! Media & Playback ▸ Decoding & Encoding page (`python/cfa4_devtools/qt_settings.py`).
//!
//! `ROWS` is the single source of CFA4's codec capability catalog (CFA4-PROCODEC-002 work
//! package 0): which crate implements each codec, where the code comes from, whether it is opened
//! on import and whether it can be exported. `docs/codec-parity.md` carries a generated copy of the
//! catalog (`cfa4-cli codecs --markdown`), and the tests below fail when the copy, the crate list
//! or the patent register (`docs/patent-review.md`) drifts from it.
//!
//! The hardware columns come from what `cfa4_platform::register` and the export format probes
//! found on this machine, so they are measured here, not assumed.

use cfa4_export::Format;
use serde_json::{Value, json};

/// How CFA4 reads a codec.
#[derive(Clone, Copy)]
enum Decode {
    /// Decoded in software and opened on import (with what is not covered, if anything).
    Software(&'static str),
    /// Recognised and described on import, but the picture or sound is not decoded.
    Detect(&'static str),
    /// Not decoded (yet).
    None,
}

/// How CFA4 writes a codec.
#[derive(Clone, Copy)]
enum Encode {
    /// The export format that encodes it.
    Format(Format),
    /// An audio codec the export writes as a track inside other formats (the export's audio
    /// codec setting), with where it goes.
    AudioTrack(&'static str),
    /// Not encoded (yet).
    None,
}

/// Where the implementation comes from (`docs/licensing.md` §3 and the reused code register §5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    /// Written for CFA4 from the public specification.
    Own,
    /// Ported from another project; listed in the reused code register.
    Ported,
    /// A third-party Rust crate used as a dependency (symphonia, image).
    Dependency,
    /// Nothing implemented yet.
    Missing,
}

impl Origin {
    fn label(self) -> &'static str {
        match self {
            Origin::Own => "own",
            Origin::Ported => "ported",
            Origin::Dependency => "dependency",
            Origin::Missing => "none",
        }
    }
}

/// One row of the catalog.
struct Row {
    codec: &'static str,
    kind: &'static str,
    /// The CFA4 crates under `crates/` that implement it (each one is in `docs/patent-review.md`).
    crates: &'static [&'static str],
    origin: Origin,
    decode: Decode,
    /// Whether the Linux hardware decoders (NVDEC, VA-API) take it.
    hardware_decode: bool,
    encode: Encode,
}

const fn video(codec: &'static str, crates: &'static [&'static str], origin: Origin, decode: Decode, hardware_decode: bool, encode: Encode) -> Row {
    Row { codec, kind: "video", crates, origin, decode, hardware_decode, encode }
}

const fn image(codec: &'static str, crates: &'static [&'static str], origin: Origin, decode: Decode) -> Row {
    Row { codec, kind: "image", crates, origin, decode, hardware_decode: false, encode: Encode::None }
}

const fn audio(codec: &'static str, crates: &'static [&'static str], origin: Origin, decode: Decode, encode: Encode) -> Row {
    Row { codec, kind: "audio", crates, origin, decode, hardware_decode: false, encode }
}

use Decode::{Detect, None as No, Software as Sw};
use Encode::{AudioTrack, Format as Fmt, None as NoEnc};
use Origin::{Dependency, Missing, Own, Ported};

static ROWS: &[Row] = &[
    video(
        "H.264 / AVC",
        &["h264", "h264enc"],
        Own,
        Sw("4:2:0 / 4:2:2 / 4:4:4, 8-14 bit, MBAFF / PAFF, lossless; encoder 8-bit 4:2:0"),
        true,
        Fmt(Format::H264),
    ),
    video("HEVC / H.265", &["hevc"], Own, Sw("up to 12-bit 4:4:4; no RExt coding tools; encoder Main / Main 10 4:2:0"), true, Fmt(Format::Hevc)),
    video("AV1", &["av1"], Own, Sw("Main 8/10-bit"), false, Fmt(Format::Av1)),
    video("AV2", &["av2"], Own, Detect("bitstream parsing only"), false, NoEnc),
    video("VP8", &["vp8"], Own, Sw("RFC 6386"), false, NoEnc),
    video("VP9", &["vp9"], Own, Sw("profiles 0-3, 8/10/12-bit"), false, Fmt(Format::WebM)),
    video("Apple ProRes", &["prores"], Own, Sw("all six profiles, alpha"), false, Fmt(Format::ProRes)),
    video("Avid DNxHD / DNxHR", &["dnx"], Own, Sw("all CIDs, 8/10/12-bit, interlaced, alpha"), false, Fmt(Format::DnxHr)),
    video("GoPro CineForm", &["cineform"], Ported, Sw("intra samples"), false, Fmt(Format::Cineform)),
    video("APV", &["apv"], Own, Sw(""), false, Fmt(Format::Apv)),
    video("MPEG-2 / MPEG-1", &["mpeg2v"], Own, Sw("4:2:2, interlaced"), false, Fmt(Format::Mpeg2)),
    video("Motion JPEG", &[], Dependency, Sw(""), false, Fmt(Format::Mjpeg)),
    video("JPEG 2000", &["j2k"], Ported, Sw("Part 1 and HTJ2K; DCP and IMF App 2E packages"), false, Fmt(Format::Dcp)),
    video("SMPTE VC-2 (Dirac)", &["dirac"], Ported, Sw("ST 2042-1"), false, NoEnc),
    video("FFV1", &["ffv1"], Own, Sw("versions 0, 1, 3, 8-16 bit"), false, NoEnc),
    video("Hap", &["hap"], Own, Sw(""), false, NoEnc),
    video("Huffyuv", &["huffyuv"], Own, Sw(""), false, NoEnc),
    video("Canopus HQ / HQA / HQX", &["canopus"], Ported, Sw(""), false, NoEnc),
    video("RealVideo 1.0 / 2.0", &["realvideo", "realmedia"], Ported, Sw("in RealMedia (.rm, .rmvb)"), false, NoEnc),
    video("Ut Video", &["utvideo"], Own, Sw(""), false, NoEnc),
    video("DV / DVCPRO", &["dv"], Ported, Sw("DV, DVCAM, DVCPRO 25 / 50, DVCPRO HD"), false, NoEnc),
    video("Animated GIF", &[], Dependency, Sw(""), false, Fmt(Format::Gif)),
    video("CinemaDNG / DNG", &["dng"], Own, Sw("uncompressed and lossless JPEG raw"), false, NoEnc),
    video("LCEVC", &["lcevc"], Own, Detect("enhancement layer detected, not decoded"), false, NoEnc),
    video("MPEG-4 Part 2", &[], Missing, No, false, NoEnc),
    video("WMV / VC-1", &[], Missing, No, false, NoEnc),
    video("Camera RAW (R3D, ARRIRAW, BRAW…)", &[], Missing, No, false, NoEnc),
    image("JPEG XL", &["jxl"], Ported, Sw("")),
    image("HEIF / HEIC", &["heif"], Own, Sw("HEVC image items")),
    audio("PCM (WAV, AIFF, BWF)", &[], Own, Sw(""), Fmt(Format::Wav)),
    audio("AAC", &["aac"], Ported, Sw("LC, HE core, LATM, USAC"), Fmt(Format::Aac)),
    audio("AC-3 / E-AC-3", &["ac3"], Own, Sw(""), AudioTrack("AC-3 audio in video exports")),
    audio("DTS, DTS-HD MA", &["dts"], Ported, Sw("core and lossless up to 7.1"), NoEnc),
    audio("Dolby TrueHD", &["truehd"], Ported, Sw("up to 7.1"), NoEnc),
    audio("MPEG-1 Layer I / II (MP1 / MP2)", &["mpa"], Own, Sw(""), AudioTrack("MP2 audio in MPEG-2 program and transport streams")),
    audio("MP3", &[], Dependency, Sw(""), NoEnc),
    audio("Opus", &["opus"], Ported, Sw(""), Fmt(Format::Opus)),
    audio("FLAC", &["flac"], Own, Sw(""), Fmt(Format::Flac)),
    audio("ALAC, Vorbis", &[], Dependency, Sw(""), NoEnc),
    audio("AMR-NB / AMR-WB", &["amr"], Ported, Sw(""), NoEnc),
    audio("Speex", &["speex"], Ported, Sw("Ogg Speex"), NoEnc),
    audio("TTA (True Audio)", &["tta"], Ported, Sw(""), NoEnc),
    audio("ADPCM, G.711, G.722", &["adpcm"], Own, Sw(""), NoEnc),
    audio("IAB (SMPTE ST 2098-2)", &["iab"], Own, Sw("decode and render"), AudioTrack("IAB track in DCP exports")),
    audio("Auro-3D", &["auro"], Ported, Sw("side channel in 24-bit PCM"), NoEnc),
    audio("WMA", &[], Missing, No, NoEnc),
];

/// The crates under `crates/` that are not codecs: containers, the engine, the UI and tools. Every
/// other crate must be a row's crate (the `every_codec_crate_is_in_the_catalog` test).
#[cfg(test)]
const NOT_CODECS: &[&str] = &[
    "audio-dsp",
    "automation",
    "avi",
    "bitstream",
    "captions",
    "cfb",
    "codecs",
    "color",
    "credits",
    "dcp",
    "dcp-inspect",
    "dcp-validate",
    "edit",
    "engine",
    "export",
    "ffmpeg",
    "format",
    "frame",
    "geom",
    "golden",
    "gpu",
    "gstreamer",
    "interchange",
    "isobmff",
    "live",
    "matroska",
    "media",
    "metrics",
    "mpegts",
    "mxf",
    "obs",
    "ogg",
    "platform",
    "project",
    "qc",
    "render",
    "route",
    "scopes",
    "smart-edit",
    "speech",
    "st2110",
    "tcsync",
    "tcsync-io",
    "testkit",
    "text",
    "time",
    "tts",
    "tts-text",
    "ui-egui",
];

/// The report: the hardware decoder found, the GStreamer backend, and one row per codec.
pub fn codecs() -> Value {
    let hardware = match cfa4_platform::register() {
        cfa4_platform::Availability::Available(name) => json!({ "backend": name, "reason": Value::Null }),
        cfa4_platform::Availability::Unavailable(why) => json!({ "backend": Value::Null, "reason": why }),
    };
    let gstreamer = match cfa4_gstreamer::probe() {
        Ok(status) => json!({ "compiledIn": true, "available": true, "version": status.version, "reason": Value::Null }),
        Err(e) => json!({ "compiledIn": cfa4_gstreamer::compiled_in(), "available": false, "version": Value::Null, "reason": e.to_string() }),
    };
    report(hardware, gstreamer, Format::has_builtin_encoder, cfa4_export::available)
}

/// `builtin`: CFA4 has its own encoder; `available`: a registered (hardware) encoder works here.
fn report(hardware: Value, gstreamer: Value, builtin: impl Fn(Format) -> bool, available: impl Fn(Format) -> bool) -> Value {
    let hw_decoder = hardware["backend"].as_str().map(str::to_string);
    let rows: Vec<Value> = ROWS
        .iter()
        .map(|r| {
            let (decode, state, note) = match r.decode {
                Decode::Software(note) => ("software", "import", note),
                Decode::Detect(note) => ("none", "detect", note),
                Decode::None => ("none", "none", ""),
            };
            let (encode, export_format, audio_track) = match r.encode {
                Encode::Format(f) if builtin(f) => ("software", Some(f.label()), None),
                Encode::Format(f) if available(f) => ("hardware", Some(f.label()), None),
                Encode::Format(_) | Encode::None => ("none", None, None),
                Encode::AudioTrack(place) => ("software", None, Some(place)),
            };
            json!({
                "codec": r.codec,
                "kind": r.kind,
                "crates": r.crates.iter().map(|c| format!("crates/{c}")).collect::<Vec<_>>(),
                "origin": r.origin.label(),
                "decode": decode,
                "decodeState": state,
                "decodeNote": note,
                "hardwareDecode": if r.hardware_decode { hw_decoder.clone().map_or(Value::Null, Value::from) } else { Value::Null },
                "encode": encode,
                "exportFormat": export_format,
                "exportAudioTrack": audio_track,
            })
        })
        .collect();
    json!({
        "hardwareDecoder": hardware,
        "gstreamer": gstreamer,
        "codecs": rows,
    })
}

/// Start and end markers of the generated catalog in `docs/codec-parity.md`.
#[cfg(test)]
const MARK_BEGIN: &str = "<!-- BEGIN GENERATED: cfa4-cli codecs --markdown -->";
#[cfg(test)]
const MARK_END: &str = "<!-- END GENERATED: cfa4-cli codecs --markdown -->";

/// The catalog as the Markdown table `docs/codec-parity.md` carries between `MARK_BEGIN` and
/// `MARK_END`. It shows what CFA4 itself implements, not what this machine's hardware adds.
pub fn markdown() -> String {
    let mut s = String::from("| Codec | Kind | Crates | Origin | Decode (import) | Encode (export) |\n|---|---|---|---|---|---|\n");
    for r in ROWS {
        let crates = if r.crates.is_empty() { "—".to_string() } else { r.crates.iter().map(|c| format!("`crates/{c}`")).collect::<Vec<_>>().join(", ") };
        let with_note = |what: &str, note: &str| if note.is_empty() { what.to_string() } else { format!("{what}: {note}") };
        let decode = match r.decode {
            Decode::Software(note) => with_note("yes", note),
            Decode::Detect(note) => with_note("detect only", note),
            Decode::None => "no".to_string(),
        };
        let encode = match r.encode {
            Encode::Format(f) => format!("yes: {}", f.label()),
            Encode::AudioTrack(place) => format!("yes: {place}"),
            Encode::None => "no".to_string(),
        };
        s.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", r.codec, r.kind, crates, r.origin.label(), decode, encode));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn none_found() -> Value {
        report(
            json!({ "backend": null, "reason": "no driver" }),
            json!({ "compiledIn": false, "available": false, "version": null, "reason": "not built" }),
            |f| f != Format::Hevc,
            |_| false,
        )
    }

    fn row<'a>(v: &'a Value, codec: &str) -> &'a Value {
        v["codecs"].as_array().unwrap().iter().find(|r| r["codec"] == codec).unwrap()
    }

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn without_hardware_only_software_codecs_are_offered() {
        let v = none_found();
        let h264 = row(&v, "H.264 / AVC");
        assert_eq!((h264["decode"].as_str(), h264["encode"].as_str()), (Some("software"), Some("software")));
        assert!(h264["hardwareDecode"].is_null());
        // a format without its own encoder and no hardware one cannot be exported
        assert_eq!(row(&v, "HEVC / H.265")["encode"], "none");
        assert!(row(&v, "HEVC / H.265")["exportFormat"].is_null());
        assert!(row(&v, "CinemaDNG / DNG")["exportFormat"].is_null());
        assert_eq!(row(&v, "WMV / VC-1")["decode"], "none");
        assert_eq!(v["hardwareDecoder"]["reason"], "no driver");
        assert_eq!(v["gstreamer"]["available"], false);
    }

    #[test]
    fn a_hardware_decoder_and_encoder_show_up_where_they_apply() {
        let only_hardware = [Format::Hevc, Format::Av1];
        let v = report(json!({ "backend": "NVDEC", "reason": null }), json!({}), |f| !only_hardware.contains(&f), |f| f == Format::Hevc);
        assert_eq!(row(&v, "H.264 / AVC")["hardwareDecode"], "NVDEC");
        assert_eq!(row(&v, "HEVC / H.265")["hardwareDecode"], "NVDEC");
        assert!(row(&v, "VP9")["hardwareDecode"].is_null());
        assert_eq!(row(&v, "HEVC / H.265")["encode"], "hardware");
        assert_eq!(row(&v, "AV1")["encode"], "none");
    }

    #[test]
    fn catalog_states_origin_and_audio_tracks() {
        let v = none_found();
        let ac3 = row(&v, "AC-3 / E-AC-3");
        assert_eq!((ac3["encode"].as_str(), ac3["exportFormat"].is_null()), (Some("software"), true));
        assert!(ac3["exportAudioTrack"].as_str().unwrap().contains("AC-3"));
        let lcevc = row(&v, "LCEVC");
        assert_eq!((lcevc["decode"].as_str(), lcevc["decodeState"].as_str()), (Some("none"), Some("detect")));
        assert_eq!(row(&v, "DV / DVCPRO")["crates"], json!(["crates/dv"]));
        assert_eq!(row(&v, "DV / DVCPRO")["origin"], "ported");
        assert_eq!(row(&v, "MP3")["origin"], "dependency");
    }

    #[test]
    fn rows_are_consistent() {
        let mut names: Vec<&str> = ROWS.iter().map(|r| r.codec).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ROWS.len(), "a codec is listed twice");
        for r in ROWS {
            let implemented = !matches!(r.decode, Decode::None) || !matches!(r.encode, Encode::None);
            assert_eq!(r.origin == Missing, !implemented, "{}: origin `none` exactly when nothing is implemented", r.codec);
            assert_eq!(r.crates.is_empty(), matches!(r.origin, Dependency | Missing) || r.codec.starts_with("PCM"), "{}: crates and origin disagree", r.codec);
        }
    }

    /// A new codec crate must get a catalog row (or, if it is not a codec, go into `NOT_CODECS`).
    #[test]
    fn every_codec_crate_is_in_the_catalog() {
        let mut listed: Vec<&str> = ROWS.iter().flat_map(|r| r.crates.iter().copied()).collect();
        listed.sort_unstable();
        let before = listed.len();
        listed.dedup();
        assert_eq!(listed.len(), before, "a crate is in two rows");
        let mut unknown = Vec::new();
        for entry in std::fs::read_dir(repo().join("crates")).unwrap() {
            let entry = entry.unwrap();
            if !entry.path().join("Cargo.toml").is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if listed.binary_search(&name.as_str()).is_err() && !NOT_CODECS.contains(&name.as_str()) {
                unknown.push(name);
            }
        }
        unknown.sort();
        assert!(unknown.is_empty(), "crates neither in the codec catalog (apps/cfa4-cli/src/codecs_cmd.rs ROWS) nor in NOT_CODECS: {unknown:?}");
        for c in listed.iter().chain(NOT_CODECS) {
            assert!(repo().join("crates").join(c).join("Cargo.toml").is_file(), "crates/{c} named in codecs_cmd.rs does not exist");
        }
    }

    /// Every codec crate has its row in the patent register (`docs/patent-review.md`).
    #[test]
    fn every_codec_crate_is_in_the_patent_register() {
        let register = std::fs::read_to_string(repo().join("docs/patent-review.md")).unwrap();
        let missing: Vec<&str> = ROWS.iter().flat_map(|r| r.crates.iter().copied()).filter(|c| !register.contains(&format!("`crates/{c}`"))).collect();
        assert!(missing.is_empty(), "codec crates without a row in docs/patent-review.md: {missing:?}");
    }

    /// `docs/codec-parity.md` carries the generated catalog; regenerate it with
    /// `CFA4_BLESS=1 cargo test -p cfa4-cli codecs_cmd`.
    #[test]
    fn parity_doc_carries_the_generated_catalog() {
        let path = repo().join("docs/codec-parity.md");
        let doc = std::fs::read_to_string(&path).unwrap();
        let (Some(b), Some(e)) = (doc.find(MARK_BEGIN), doc.find(MARK_END)) else {
            panic!("docs/codec-parity.md lacks the {MARK_BEGIN} / {MARK_END} markers");
        };
        let start = b + MARK_BEGIN.len();
        let want = format!("\n{}", markdown());
        if std::env::var_os("CFA4_BLESS").is_some() {
            std::fs::write(&path, format!("{}{want}{}", &doc[..start], &doc[e..])).unwrap();
            return;
        }
        assert_eq!(&doc[start..e], want, "docs/codec-parity.md is out of date: run CFA4_BLESS=1 cargo test -p cfa4-cli codecs_cmd");
    }
}
