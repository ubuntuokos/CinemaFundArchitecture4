//! The external VVC decoder (ADR 0013, VVC-02): the `cfa4-vvc-decode` helper (VVdeC) through the
//! Rust adapter must give the same pictures as FFmpeg's own VVC decoder on a VVenC stream, and a
//! helper that is not the VVC helper must be refused.
//!
//! The decoding tests need `scripts/dev/vvc-decoder.py build` (or `CFA4_VVC_DECODER`) and an
//! FFmpeg with `libvvenc` for the fixture; otherwise they report SKIPPED. `libvvenc` and VVdeC
//! are restricted components (`cfa4-restricted-extras`, ADR 0013), so they skip even with
//! `CFA4_REQUIRE_ORACLES=1`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cfa4_codecs::video::{DecodedFrame, VideoDecoder};
use cfa4_codecs::vvc_external::{self, ExternalVvcDecoder};
use cfa4_frame::PixelData;
use cfa4_testkit::fixtures::generate;
use cfa4_testkit::{ffmpeg_has_encoder, ffmpeg_or_skip, fixtures_dir};

const FRAMES: usize = 24;
const WIDTH: u32 = 330;
const HEIGHT: u32 = 202;

/// The helper: `CFA4_VVC_DECODER`, or the default build when its VVdeC stamp matches the pin.
fn helper() -> Option<PathBuf> {
    if let Some(p) = vvc_external::configured_helper() {
        return Some(p);
    }
    let root = cfa4_testkit::workspace_root();
    let pin = std::fs::read_to_string(root.join("packaging/vvc-decode/vvdec.json")).ok()?;
    let pin: serde_json::Value = serde_json::from_str(&pin).ok()?;
    let commit = pin.get("commit")?.as_str()?.to_string();
    let prefix = root.join("target/vvc-decoder");
    let stamp = std::fs::read_to_string(prefix.join("VVDEC_COMMIT")).ok()?;
    let exe = prefix.join("bin/cfa4-vvc-decode");
    (stamp.trim() == commit && exe.is_file()).then_some(exe)
}

/// The VVenC fixture (the same one the `cfa4-vvc` parser tests use) and the FFmpeg that made it.
fn fixture(ctx: &str) -> Option<(PathBuf, PathBuf)> {
    let ff = ffmpeg_or_skip(ctx)?;
    if !ffmpeg_has_encoder(&ff, "libvvenc") {
        eprintln!("SKIPPED ({ctx}): this ffmpeg has no libvvenc (restricted component)");
        return None;
    }
    let out = fixtures_dir("vvc").join(format!("testsrc-{WIDTH}x{HEIGHT}-{FRAMES}f-ip8.266"));
    let path = generate(&out, |tmp| {
        Command::new(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=24"))
            .args(["-frames:v", &FRAMES.to_string(), "-pix_fmt", "yuv420p10le", "-c:v", "libvvenc", "-preset", "faster", "-qp", "32"])
            .args(["-vvenc-params", "IntraPeriod=8", "-f", "vvc"])
            .arg(tmp)
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
    .expect("ffmpeg with libvvenc failed to encode the fixture");
    Some((ff, path))
}

/// FFmpeg's own decode of `path`: 10-bit 4:2:0 frames in output order, planes packed like the
/// helper's (two bytes little-endian per sample).
fn ffmpeg_frames(ff: &Path, path: &Path) -> Vec<Vec<u8>> {
    let out = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-c:v", "vvc", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p10le", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg could not decode the fixture: {}", String::from_utf8_lossy(&out.stderr));
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let size = (w * h + 2 * w.div_ceil(2) * h.div_ceil(2)) * 2;
    assert_eq!(out.stdout.len() % size, 0);
    out.stdout.chunks(size).map(<[u8]>::to_vec).collect()
}

/// The planes of a decoded frame, packed like FFmpeg's raw output.
fn packed(f: &DecodedFrame) -> Vec<u8> {
    match &f.frame.data {
        PixelData::Yuv16 { planes, bits: 10, .. } => planes.iter().flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes())).collect(),
        other => panic!("expected 10-bit frames, got {other:?}"),
    }
}

/// The access units of the fixture, decoded with `pts` = decode index, starting at unit `from`.
fn decode(helper: &Path, data: &[u8], from: usize) -> Vec<DecodedFrame> {
    let mut dec = ExternalVvcDecoder::new(helper, 0).unwrap();
    assert!(dec.name().contains("VVdeC"), "{}", dec.name());
    let mut frames = Vec::new();
    for (i, (off, len)) in cfa4_vvc::annexb_access_units(data).into_iter().enumerate().skip(from) {
        frames.extend(dec.decode(&data[off..off + len], i as i64).unwrap());
    }
    frames.extend(dec.flush());
    frames
}

#[test]
fn helper_matches_ffmpeg() {
    let ctx = "vvc_external::helper_matches_ffmpeg";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some((ff, path)) = fixture(ctx) else { return };
    let data = std::fs::read(&path).unwrap();
    let want = ffmpeg_frames(&ff, &path);
    assert_eq!(want.len(), FRAMES);

    let frames = decode(&helper, &data, 0);
    assert_eq!(frames.len(), FRAMES);
    let mut pts: Vec<i64> = frames.iter().map(|f| f.pts).collect();
    pts.sort_unstable();
    assert_eq!(pts, (0..FRAMES as i64).collect::<Vec<_>>(), "every access unit's picture comes out once");
    for (k, (f, w)) in frames.iter().zip(&want).enumerate() {
        assert_eq!((f.frame.width, f.frame.height), (WIDTH, HEIGHT));
        assert!(packed(f) == *w, "output picture {k} (pts {}) differs from FFmpeg's", f.pts);
    }

    // Decoding the same stream again after a reset gives the same pictures.
    let again = decode(&helper, &data, 0);
    assert!(again.iter().zip(&frames).all(|(a, b)| a.pts == b.pts && packed(a) == packed(b)));
}

/// Decoding that starts at a later random-access unit gives the same pictures from there on.
#[test]
fn decoding_starts_at_a_later_random_access_unit() {
    let ctx = "vvc_external::decoding_starts_at_a_later_random_access_unit";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some((_, path)) = fixture(ctx) else { return };
    let data = std::fs::read(&path).unwrap();
    let units = cfa4_vvc::annexb_access_units(&data);
    let start = units
        .iter()
        .enumerate()
        .skip(1)
        .find(|&(_, &(off, len))| vvc_external::is_random_access_unit(&data[off..off + len]))
        .map(|(i, _)| i)
        .expect("the fixture has a second random-access unit");

    let full = decode(&helper, &data, 0);
    let part = decode(&helper, &data, start);
    assert!(!part.is_empty());
    for f in &part {
        assert!(f.pts >= start as i64, "picture of unit {} before the start unit {start}", f.pts);
        let same = full.iter().find(|g| g.pts == f.pts).expect("the picture is in the full decode");
        assert!(packed(f) == packed(same), "picture of unit {} differs when decoding starts at unit {start}", f.pts);
    }
}

#[test]
fn hostile_units_through_the_real_helper_do_not_crash() {
    let ctx = "vvc_external::hostile_units_through_the_real_helper_do_not_crash";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some((_, path)) = fixture(ctx) else { return };
    let data = std::fs::read(&path).unwrap();
    let units = cfa4_vvc::annexb_access_units(&data);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut dec = ExternalVvcDecoder::new(&helper, 1).unwrap();
    for round in 0..20 {
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
    // the helper still decodes a clean stream afterwards
    dec.reset();
    let mut n = 0;
    for (i, &(off, len)) in units.iter().enumerate() {
        n += dec.decode(&data[off..off + len], i as i64).unwrap().len();
    }
    n += dec.flush().len();
    assert_eq!(n, FRAMES);
}

/// A helper that greets with another codec's tag is refused.
#[test]
fn an_av2_helper_is_not_a_vvc_helper() {
    let dir = fixtures_dir("vvc/fake-helpers");
    let path = dir.join("av2-hello.sh");
    std::fs::write(&path, "#!/bin/sh\nprintf 'CFA4AV2D\\001\\000\\000\\000\\004\\000\\000\\000fake'\nsleep 5\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(ExternalVvcDecoder::new(&path, 0).is_err());
}
