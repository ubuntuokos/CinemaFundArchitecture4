//! The performance condition of the VVC plan (section 3.4, ADR 0013, VVC-TEST-021): decoding
//! through the VVdeC helper must not be slower than FFmpeg's own VVC decoder on the same stream,
//! neither for playback (pictures per second) nor for a seek (the time from "start decoding at a
//! random-access unit" to the first picture), with the same threads on the same cores.
//!
//! Timing tests are noisy, so this one runs only with `CFA4_VVC_PERF=1` (otherwise SKIPPED); the
//! numbers go to stderr and the verdict is asserted with a tolerance. Needs the helper
//! (`scripts/dev/vvc-decoder.py build`), the corpus (`scripts/dev/vvc-conformance.py fetch`) and an
//! FFmpeg with the native `vvc` decoder (7.1 or later; `CFA4_FFMPEG`, else `/opt/cfa4/ffmpeg-<CFA4_FFMPEG_VERSION>`,
//! else `ffmpeg`). Two kinds of stream: the realistic-bitrate VVenC fixtures
//! (`scripts/dev/vvc-perf-fixture.py encode`, `target/fixtures/vvc/perf-*.266`), which carry the
//! verdict, and the JVET conformance streams `LFNST_D_3` (1080p) and `TRANS_A_2` (720p), stress
//! streams at extreme bitrates, which are only reported. `CFA4_VVC_PERF_STREAMS` replaces the
//! first list, `CFA4_VVC_PERF_INFO_STREAMS` the second (comma-separated; a name is a corpus
//! stream, a path is used as given). `CFA4_VVC_PERF_THREADS` the threads of both
//! decoders (default 4), `CFA4_VVC_PERF_RUNS` the runs of each measurement (default 3; the best
//! counts). Pin the cores with `taskset` and record `lscpu` in the report
//! (docs/development/reference-machine.md).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cfa4_codecs::video::VideoDecoder;
use cfa4_codecs::vvc_external::{self, ExternalVvcDecoder};

fn helper() -> Option<PathBuf> {
    if let Some(p) = vvc_external::configured_helper() {
        return Some(p);
    }
    let root = cfa4_testkit::workspace_root();
    let pin: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("packaging/vvc-decode/vvdec.json")).ok()?).ok()?;
    let prefix = root.join("target/vvc-decoder");
    let stamp = std::fs::read_to_string(prefix.join("VVDEC_COMMIT")).ok()?;
    let exe = prefix.join("bin/cfa4-vvc-decode");
    (stamp.trim() == pin.get("commit")?.as_str()? && exe.is_file()).then_some(exe)
}

fn has_native_vvc(p: &Path) -> bool {
    Command::new(p)
        .args(["-hide_banner", "-decoders"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some("vvc")))
}

/// The CFA4 FFmpeg build, `/opt/cfa4/ffmpeg-VERSION/bin/ffmpeg`, with the pinned version of
/// `packaging/ffmpeg-version.env` (the one place it is set), if that file can be read.
fn pinned_ffmpeg() -> Option<PathBuf> {
    let env = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/ffmpeg-version.env")).ok()?;
    let version = env.lines().find_map(|l| l.trim().strip_prefix("CFA4_FFMPEG_VERSION="))?.trim().trim_matches('"');
    Some(PathBuf::from(format!("/opt/cfa4/ffmpeg-{version}/bin/ffmpeg")))
}

/// `CFA4_FFMPEG` when set (and then only that: a wrong path is an error, not a quiet switch to
/// another FFmpeg), else the CFA4 build, else `ffmpeg` on the PATH.
fn ffmpeg() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("CFA4_FFMPEG").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        return if has_native_vvc(&p) { Ok(p) } else { Err(format!("CFA4_FFMPEG={} has no native vvc decoder", p.display())) };
    }
    pinned_ffmpeg().into_iter().chain([PathBuf::from("ffmpeg")]).find(|p| has_native_vvc(p)).ok_or_else(|| "no FFmpeg with the native vvc decoder".to_string())
}

fn best<T>(runs: usize, mut f: impl FnMut() -> (Duration, T)) -> (Duration, T) {
    let mut best: Option<(Duration, T)> = None;
    for _ in 0..runs {
        let r = f();
        if best.as_ref().is_none_or(|b| r.0 < b.0) {
            best = Some(r);
        }
    }
    best.unwrap()
}

/// FFmpeg decoding `path` completely, the pictures written as raw video to the null sink.
fn ffmpeg_decode(ff: &Path, path: &Path, threads: usize, frames: Option<usize>) -> Duration {
    let mut cmd = Command::new(ff);
    cmd.args(["-v", "error", "-threads", &threads.to_string(), "-i"]).arg(path);
    if let Some(n) = frames {
        cmd.args(["-frames:v", &n.to_string()]);
    }
    cmd.args(["-f", "rawvideo", "-y", "/dev/null"]).stdin(Stdio::null());
    let t = Instant::now();
    assert!(cmd.status().unwrap().success(), "ffmpeg failed on {}", path.display());
    t.elapsed()
}

fn helper_decode(helper: &Path, threads: usize, units: &[&[u8]], want_first: bool) -> (Duration, usize) {
    let t = Instant::now();
    let mut dec = ExternalVvcDecoder::new(helper, threads).unwrap();
    dec.set_request_timeout(Duration::from_secs(120));
    let mut pictures = 0;
    for (i, u) in units.iter().enumerate() {
        pictures += dec.decode(u, i as i64).unwrap().len();
        if want_first && pictures > 0 {
            return (t.elapsed(), pictures);
        }
    }
    pictures += dec.flush().len();
    (t.elapsed(), pictures)
}

#[test]
fn the_helper_is_not_slower_than_ffmpeg() {
    let ctx = "vvc_perf::the_helper_is_not_slower_than_ffmpeg";
    if std::env::var_os("CFA4_VVC_PERF").is_none() {
        return eprintln!("SKIPPED ({ctx}): timing test, set CFA4_VVC_PERF=1");
    }
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let ff = match ffmpeg() {
        Ok(ff) => ff,
        // an explicit wrong path is a failure; no FFmpeg at all is a skip
        Err(e) if std::env::var_os("CFA4_FFMPEG").is_some_and(|v| !v.is_empty()) => panic!("{e}"),
        Err(e) => return eprintln!("SKIPPED ({ctx}): {e}"),
    };
    let corpus = std::env::var_os("CFA4_VVC_CONFORMANCE_DIR").map_or_else(|| cfa4_testkit::fixtures_dir("vvc-conformance"), PathBuf::from);
    let threads: usize = std::env::var("CFA4_VVC_PERF_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let runs: usize = std::env::var("CFA4_VVC_PERF_RUNS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let fixtures = cfa4_testkit::fixtures_dir("vvc");
    let verdict_streams = std::env::var("CFA4_VVC_PERF_STREAMS").unwrap_or_else(|_| {
        ["perf-1080p-faster.266", "perf-1080p-medium.266", "perf-2160p-faster.266"].map(|n| fixtures.join(n).display().to_string()).join(",")
    });
    let info_streams = std::env::var("CFA4_VVC_PERF_INFO_STREAMS").unwrap_or_else(|_| "LFNST_D_3,TRANS_A_2".into());
    let streams: Vec<(String, bool)> = verdict_streams
        .split(',')
        .map(|n| (n.to_string(), true))
        .chain(info_streams.split(',').filter(|n| !n.is_empty()).map(|n| (n.to_string(), false)))
        .collect();
    let scratch = cfa4_testkit::fixtures_dir("vvc/perf");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut measured: Vec<usize> = Vec::new();
    let mut verdicts = Vec::new();

    for (index, (name, counts)) in streams.iter().enumerate() {
        let bit = if name.contains('/') { PathBuf::from(name) } else { corpus.join(name).join(format!("{name}.bit")) };
        let name = bit.file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
        let name = name.as_str();
        let Ok(data) = std::fs::read(&bit) else {
            eprintln!("vvc_perf: {name} not in the corpus, skipped");
            continue;
        };
        let aus = cfa4_vvc::annexb_access_units(&data);
        let units: Vec<&[u8]> = aus.iter().map(|&(o, l)| &data[o..o + l]).collect();

        // playback: the whole stream
        let (t_helper, pictures) = best(runs, || helper_decode(&helper, threads, &units, false));
        let t_ff = best(runs, || (ffmpeg_decode(&ff, &bit, threads, None), ())).0;
        let (fps_h, fps_f) = (pictures as f64 / t_helper.as_secs_f64(), pictures as f64 / t_ff.as_secs_f64());
        eprintln!("vvc_perf: {name} playback ({threads} threads): helper {fps_h:.1} fps ({t_helper:?}), ffmpeg {fps_f:.1} fps ({t_ff:?}), {pictures} pictures");
        verdicts.push((format!("{name} playback"), !counts || fps_h >= 0.85 * fps_f, format!("helper {fps_h:.1} fps vs ffmpeg {fps_f:.1} fps")));

        // seek: from a random-access unit in the middle to its first picture
        let rau: Vec<usize> = (0..units.len()).filter(|&i| vvc_external::is_random_access_unit(units[i])).collect();
        let Some(&at) = rau.iter().find(|&&i| i >= units.len() * 2 / 5) else {
            eprintln!("vvc_perf: {name} has no random-access unit after 40%, seek not measured");
            continue;
        };
        // the parameter sets come with the units of an IRAP in a conformance stream; a stream
        // that does not repeat them is not usable for a mid-stream seek
        let tail = &data[aus[at].0..];
        let cut = scratch.join(format!("{name}-seek.bit"));
        std::fs::write(&cut, tail).unwrap();
        let (s_helper, first) = best(runs, || helper_decode(&helper, threads, &units[at..], true));
        let s_ff = best(runs, || (ffmpeg_decode(&ff, &cut, threads, Some(1)), ())).0;
        eprintln!("vvc_perf: {name} seek to unit {at} (first picture): helper {s_helper:?} ({first} pictures), ffmpeg {s_ff:?}");
        verdicts.push((
            format!("{name} seek"),
            !counts || s_helper <= s_ff.mul_f64(1.25) + Duration::from_millis(150),
            format!("helper {s_helper:?} vs ffmpeg {s_ff:?}"),
        ));
        let _ = std::fs::remove_file(&cut);
        if *counts {
            measured.push(index);
        }
    }

    // start-up of the helper: the hello, once, when the first VVC file is opened (nothing at CFA4 start)
    let (start, _) = best(runs, || {
        let t = Instant::now();
        let d = ExternalVvcDecoder::new(&helper, threads).unwrap();
        let e = t.elapsed();
        drop(d);
        (e, ())
    });
    eprintln!("vvc_perf: helper start to hello: {start:?}");

    // every verdict stream must have both its playback and its seek result; only the
    // informational ones may be missing
    // (identified by the position in the configured list, not by the file stem)
    let missing: Vec<&str> = streams.iter().enumerate().filter(|(i, (_, counts))| *counts && !measured.contains(i)).map(|(_, (n, _))| n.as_str()).collect();
    assert!(missing.is_empty() && !measured.is_empty(), "verdict streams not measured: {missing:?} (scripts/dev/vvc-perf-fixture.py encode)");
    let failed: Vec<_> = verdicts.iter().filter(|v| !v.1).collect();
    assert!(failed.is_empty(), "the helper is slower than FFmpeg: {failed:?}");
}
