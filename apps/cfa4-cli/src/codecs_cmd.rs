//! `cfa4-cli codecs`: what this machine decodes and encodes, for the Qt window's Settings ▸
//! Media & Playback ▸ Decoding & Encoding page (`python/cfa4_devtools/qt_settings.py`).
//!
//! The software column is CFA4's own codecs (docs/codec-parity.md, Linux); the hardware columns
//! come from what `cfa4_platform::register` and the export format probes found on this machine,
//! so they are measured here, not assumed.

use cfa4_export::Format;
use serde_json::{Value, json};

/// How CFA4's own decoder handles a codec.
#[derive(Clone, Copy)]
enum Decode {
    /// Decoded in software (with what is not covered, if anything).
    Software(&'static str),
    /// Not decoded (yet).
    None,
}

/// One row of the table: the codec, our decoder, whether the Linux hardware decoders (NVDEC,
/// VA-API) take it, and the export format that encodes it.
struct Row {
    codec: &'static str,
    kind: &'static str,
    decode: Decode,
    hardware_decode: bool,
    encode: Option<Format>,
}

const fn video(codec: &'static str, decode: Decode, hardware_decode: bool, encode: Option<Format>) -> Row {
    Row { codec, kind: "video", decode, hardware_decode, encode }
}

const fn audio(codec: &'static str, decode: Decode, encode: Option<Format>) -> Row {
    Row { codec, kind: "audio", decode, hardware_decode: false, encode }
}

use Decode::{None as No, Software as Sw};

static ROWS: &[Row] = &[
    video("H.264 / AVC", Sw("4:2:0 / 4:2:2 / 4:4:4, 8-14 bit; no interlaced fields / MBAFF"), true, Some(Format::H264)),
    video("HEVC / H.265", Sw("up to 12-bit 4:4:4; no RExt coding tools"), true, Some(Format::Hevc)),
    video("AV1", Sw("Main 8/10-bit"), false, Some(Format::Av1)),
    video("VP9", Sw("profiles 0-3, 8/10/12-bit"), false, Some(Format::WebM)),
    video("Apple ProRes", Sw("all six profiles, alpha"), false, Some(Format::ProRes)),
    video("Avid DNxHD / DNxHR", Sw("all CIDs, 8/10/12-bit"), false, Some(Format::DnxHr)),
    video("GoPro CineForm", Sw("intra samples"), false, Some(Format::Cineform)),
    video("APV", Sw(""), false, Some(Format::Apv)),
    video("MPEG-2 / MPEG-1", Sw("4:2:2, interlaced"), false, Some(Format::Mpeg2)),
    video("Motion JPEG", Sw(""), false, Some(Format::Mjpeg)),
    video("JPEG 2000", Sw("Part 1 and HTJ2K"), false, Some(Format::Dcp)),
    video("Animated GIF", Sw(""), false, Some(Format::Gif)),
    video("CinemaDNG / DNG", Sw("uncompressed and lossless JPEG raw"), false, None),
    video("DV / DVCPRO", No, false, None),
    video("MPEG-4 Part 2", No, false, None),
    video("WMV / VC-1", No, false, None),
    video("Camera RAW (R3D, ARRIRAW, BRAW…)", No, false, None),
    audio("PCM (WAV, AIFF, BWF)", Sw(""), Some(Format::Wav)),
    audio("AAC", Sw("LC, HE core, LATM"), Some(Format::Aac)),
    audio("AC-3 / E-AC-3", Sw(""), None),
    audio("DTS, DTS-HD MA", Sw("core and lossless up to 7.1"), None),
    audio("Dolby TrueHD", Sw("up to 7.1"), None),
    audio("MP3 / MP2", Sw(""), None),
    audio("Opus", Sw(""), Some(Format::Opus)),
    audio("FLAC", Sw(""), Some(Format::Flac)),
    audio("ALAC, Vorbis", Sw(""), None),
    audio("WMA", No, None),
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
            let (decode, note) = match r.decode {
                Decode::Software(note) => ("software", note),
                Decode::None => ("none", ""),
            };
            let encode = match r.encode {
                Some(f) if builtin(f) => "software",
                Some(f) if available(f) => "hardware",
                _ => "none",
            };
            json!({
                "codec": r.codec,
                "kind": r.kind,
                "decode": decode,
                "decodeNote": note,
                "hardwareDecode": if r.hardware_decode { hw_decoder.clone().map_or(Value::Null, Value::from) } else { Value::Null },
                "encode": encode,
                "exportFormat": r.encode.map(|f| f.label()),
            })
        })
        .collect();
    json!({
        "hardwareDecoder": hardware,
        "gstreamer": gstreamer,
        "codecs": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn without_hardware_only_software_codecs_are_offered() {
        let v = none_found();
        let h264 = row(&v, "H.264 / AVC");
        assert_eq!((h264["decode"].as_str(), h264["encode"].as_str()), (Some("software"), Some("software")));
        assert!(h264["hardwareDecode"].is_null());
        // a format without its own encoder and no hardware one cannot be exported
        assert_eq!(row(&v, "HEVC / H.265")["encode"], "none");
        assert!(row(&v, "CinemaDNG / DNG")["exportFormat"].is_null());
        assert_eq!(row(&v, "DV / DVCPRO")["decode"], "none");
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
    fn codecs_are_listed_once() {
        let mut names: Vec<&str> = ROWS.iter().map(|r| r.codec).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ROWS.len());
    }
}
