//! The external AV2 decoder (ADR 0010, plan X/6): the `cfa4-av2-decode` helper through the
//! Rust adapter must reproduce the AVM v1.0.0 oracle hashes on the committed corpus, and a
//! misbehaving helper (crash, garbage, hang) must be an error, never a crash.
//!
//! The oracle tests need `scripts/dev/av2-decoder.py build` and `scripts/dev/av2-oracle.py
//! corpus`; otherwise they report SKIPPED (a failure under `CFA4_REQUIRE_AV2_ORACLE=1`).
//! The bit-exact requirement covers the `Main_420_10_IP0` scope the plan commits to (G1); other
//! vectors are reported (dav2d's 4:4:4 inter mismatch is known, AV2-00 audit point 4).

use std::path::{Path, PathBuf};
use std::time::Duration;

use cfa4_codecs::av2_external::{self, Av2AnnexBSource, ExternalAv2Decoder};
use cfa4_codecs::video::VideoDecoder;
use cfa4_frame::{PixelData, VideoFrame};
use cfa4_media::MediaSource;
use cfa4_testkit::av2::{self, Corpus, Geometry, Vector};

/// The helper: `CFA4_AV2_DECODER`, or the default build when its dav2d stamp matches the pin.
fn helper() -> Option<PathBuf> {
    if let Some(p) = av2_external::configured_helper() {
        return Some(p);
    }
    let root = cfa4_testkit::workspace_root();
    let pin = std::fs::read_to_string(root.join("packaging/av2-decode/dav2d.json")).ok()?;
    let pin: serde_json::Value = serde_json::from_str(&pin).ok()?;
    let commit = pin.get("commit")?.as_str()?.to_string();
    let prefix = root.join("target/av2-decoder");
    let stamp = std::fs::read_to_string(prefix.join("DAV2D_COMMIT")).ok()?;
    let exe = prefix.join("bin/cfa4-av2-decode");
    (stamp.trim() == commit && exe.is_file()).then_some(exe)
}

/// The packed planes of a frame, hashed like the oracle's Y4M planes.
fn plane_hashes(f: &VideoFrame, mono: bool) -> Vec<String> {
    let mut out: Vec<String> = match &f.data {
        PixelData::Yuv8 { planes, .. } => planes.iter().map(|p| av2::sha256_hex(p)).collect(),
        PixelData::Yuv16 { planes, .. } => planes.iter().map(|p| av2::sha256_hex(&p.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())).collect(),
        _ => panic!("unexpected pixel data"),
    };
    if mono {
        out.truncate(1);
    }
    out
}

fn geometry(f: &VideoFrame, v: &Vector) -> Geometry {
    let bytes = match &f.data {
        PixelData::Yuv16 { .. } => 2,
        _ => 1,
    };
    // the frame type has no 4:0:0, so the chroma label comes from the vector when the planes agree
    Geometry { width: f.width, height: f.height, chroma: v.geometry.chroma.clone(), bytes_per_sample: bytes }
}

fn decode_vector(helper: &Path, v: &Vector, obu: &Path) -> Result<(Geometry, Vec<Vec<String>>), String> {
    let data = std::fs::read(obu).map_err(|e| e.to_string())?;
    let mut dec = ExternalAv2Decoder::new(helper, 0).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    for (i, (off, len)) in av2_external::temporal_units(&data).into_iter().enumerate() {
        frames.extend(dec.decode(&data[off..off + len], i as i64).map_err(|e| e.to_string())?);
    }
    frames.extend(dec.flush());
    let first = frames.first().ok_or("no frames")?;
    let geo = geometry(&first.frame, v);
    let mono = v.geometry.chroma == "400";
    Ok((geo, frames.iter().map(|f| plane_hashes(&f.frame, mono)).collect()))
}

#[test]
fn helper_matches_the_avm_oracle() {
    let ctx = "av2_external::helper_matches_the_avm_oracle";
    let corpus = Corpus::load().unwrap();
    let Some(helper) = helper() else {
        return av2::skip(ctx, "cfa4-av2-decode not built (scripts/dev/av2-decoder.py build)");
    };
    let (mut failures, mut reported) = (Vec::new(), Vec::new());
    for v in &corpus.vectors {
        let Some(obu) = av2::vector_bitstream(v).unwrap() else {
            av2::skip(ctx, &format!("{}: corpus bitstream missing (scripts/dev/av2-oracle.py corpus)", v.name));
            continue;
        };
        let result = decode_vector(&helper, v, &obu).map(|(g, f)| av2::compare(v, &g, &f));
        let line = match &result {
            Ok(cmp) => format!("{}: {cmp}", v.name),
            Err(e) => format!("{}: {e}", v.name),
        };
        eprintln!("{line}");
        if result.as_ref().map(|c| !c.pass()).unwrap_or(true) {
            if v.profile == "Main_420_10_IP0" {
                failures.push(line);
            } else {
                reported.push(line);
            }
        }
    }
    if !reported.is_empty() {
        eprintln!("outside the Main_420_10_IP0 scope (reported, not failing):\n{}", reported.join("\n"));
    }
    assert!(failures.is_empty(), "the external decoder differs from the oracle:\n{}", failures.join("\n"));
}

/// An Annex B file opened as a media source: frames by time, including a seek back.
#[test]
fn annex_b_source_seeks_to_oracle_frames() {
    let ctx = "av2_external::annex_b_source_seeks_to_oracle_frames";
    let corpus = Corpus::load().unwrap();
    let Some(helper) = helper() else {
        return av2::skip(ctx, "cfa4-av2-decode not built (scripts/dev/av2-decoder.py build)");
    };
    let v = corpus.vector("420-10bit-default").unwrap();
    let Some(obu) = av2::vector_bitstream(v).unwrap() else {
        return av2::skip(ctx, "corpus bitstream missing (scripts/dev/av2-oracle.py corpus)");
    };
    let reader: cfa4_media::SharedReader = std::sync::Arc::new(cfa4_media::reader::MemReader(std::fs::read(&obu).unwrap().into()));
    let src = Av2AnnexBSource::open("420-10bit-default.obu", reader, helper).unwrap();
    let info = src.info().video.clone().unwrap();
    assert_eq!((info.width, info.height), (352, 288));
    assert!(info.pixel_format.contains("10-bit"), "{}", info.pixel_format);
    let rate = info.frame_rate;
    for i in [0usize, 9, 4, 5, 1] {
        let f = src.video_frame(cfa4_media::FrameRequest::full(rate.tick_of(i as i64))).unwrap();
        assert_eq!(plane_hashes(&f, false), v.frames[i], "frame {i}");
    }
}

/// A fake helper: a shell script that says hello and then misbehaves.
fn fake_helper(name: &str, body: &str) -> PathBuf {
    let dir = cfa4_testkit::fixtures_dir("av2/fake-helpers");
    let path = dir.join(name);
    let script = format!("#!/bin/sh\nprintf 'CFA4AV2D\\001\\000\\000\\000\\004\\000\\000\\000fake'\n{body}\n");
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

const UNIT: &[u8] = &[0x01, 0x08, 0x02, 0x10, 0x00];

#[test]
fn a_crashing_helper_is_an_error_and_is_restarted() {
    let path = fake_helper("crash.sh", "head -c 8 >/dev/null\nkill -SEGV $$");
    let mut dec = ExternalAv2Decoder::new(&path, 0).unwrap();
    assert!(dec.name().contains("fake"));
    assert!(dec.decode(UNIT, 0).is_err());
    // the next call starts a new helper, which fails the same way
    assert!(dec.decode(UNIT, 1).is_err());
    assert!(dec.flush().is_empty());
    dec.reset();
}

#[test]
fn garbage_from_the_helper_is_an_error() {
    // FRME of 10 bytes (shorter than its header), then DONE
    let path = fake_helper("garbage.sh", "head -c 8 >/dev/null\nprintf 'FRME\\012\\000\\000\\0000123456789DONE\\000\\000\\000\\000'\nsleep 5");
    let mut dec = ExternalAv2Decoder::new(&path, 0).unwrap();
    assert!(dec.decode(UNIT, 0).is_err());
    let path = fake_helper("unknown.sh", "head -c 8 >/dev/null\nprintf 'WHAT\\000\\000\\000\\000'\nsleep 5");
    let mut dec = ExternalAv2Decoder::new(&path, 0).unwrap();
    assert!(dec.decode(UNIT, 0).is_err());
}

#[test]
fn a_hanging_helper_times_out() {
    let path = fake_helper("hang.sh", "sleep 30");
    let mut dec = ExternalAv2Decoder::new(&path, 0).unwrap();
    dec.set_request_timeout(Duration::from_millis(300));
    let t = std::time::Instant::now();
    assert!(dec.decode(UNIT, 0).is_err());
    assert!(t.elapsed() < Duration::from_secs(10));
}

#[test]
fn hostile_units_through_the_real_helper_do_not_crash() {
    let ctx = "av2_external::hostile_units_through_the_real_helper_do_not_crash";
    let corpus = Corpus::load().unwrap();
    let Some(helper) = helper() else {
        return av2::skip(ctx, "cfa4-av2-decode not built (scripts/dev/av2-decoder.py build)");
    };
    let v = corpus.vector("420-8bit-default").unwrap();
    let Some(obu) = av2::vector_bitstream(v).unwrap() else {
        return av2::skip(ctx, "corpus bitstream missing");
    };
    let data = std::fs::read(obu).unwrap();
    let units = av2_external::temporal_units(&data);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut dec = ExternalAv2Decoder::new(&helper, 1).unwrap();
    for round in 0..40 {
        dec.reset();
        for (i, &(off, len)) in units.iter().enumerate() {
            let mut unit = data[off..off + len].to_vec();
            for _ in 0..1 + round % 6 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let k = (seed as usize) % unit.len();
                unit[k] ^= (seed >> 32) as u8 | 1;
            }
            if round % 5 == 4 {
                unit.truncate(unit.len() / 2);
            }
            // errors are fine; a panic or a hang is not
            let _ = dec.decode(&unit, i as i64);
        }
        let _ = dec.flush();
    }
}
