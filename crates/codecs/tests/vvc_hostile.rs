//! Hostile input through the real VVC helper (ADR 0013, VVC-02, VVC-TEST-017, gate G2).
//!
//! Real JVET conformance streams, damaged in many ways, go through `ExternalVvcDecoder` ->
//! `cfa4-vvc-decode` (VVdeC). A decode error is the right answer to damage; what must never
//! happen is a panic in CFA4, a request that runs past its limit, a picture whose planes do not
//! match its size, or a decoder that cannot decode a clean stream afterwards (the adapter
//! restarts a helper that died). The damage: bit flips, overwritten and inserted bytes, damaged
//! start codes, truncation at every depth, dropped, repeated and swapped access units, empty and
//! oversized units, and garbage that is not VVC at all.
//!
//! The streams come from the unpacked JVET corpus (`scripts/dev/vvc-conformance.py fetch`, or
//! `CFA4_VVC_CONFORMANCE_DIR`), the helper from `scripts/dev/vvc-decoder.py build` (or
//! `CFA4_VVC_DECODER`); otherwise the test reports SKIPPED, also under `CFA4_REQUIRE_ORACLES`
//! (VVdeC and the corpus are restricted components). `CFA4_VVC_HOSTILE_SECONDS` is the time
//! budget of the campaign (default 20); the damage is seeded (`CFA4_VVC_HOSTILE_SEED`), and a
//! failing run prints the stream, round and seed that reproduce it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cfa4_codecs::video::{DecodedFrame, VideoDecoder};
use cfa4_codecs::vvc_external::{self, ExternalVvcDecoder};
use cfa4_frame::PixelData;

/// Streams of the corpus used as victims: small, and covering the tools of Main 10 (the ones
/// the helper refuses outright are not interesting for damage).
const VICTIMS: &[&str] = &[
    "PPS_A", "SLIST_A", "ALF_A", "LMCS_A", "WPP_A", "TILE_A", "SUBPIC_A", "RPL_A", "CAINIT_A", "AMVR_A", "DBLK_A", "MMVD_A", "GPM_A", "BCW_A", "CCLM_A",
    "MIP_A", "TSRC_A", "ISP_A", "SBT_A", "STILL_A",
];

/// Largest victim stream (bytes), so a round stays quick.
const MAX_STREAM: u64 = 300_000;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

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

fn corpus_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("CFA4_VVC_CONFORMANCE_DIR").map_or_else(|| cfa4_testkit::fixtures_dir("vvc-conformance"), PathBuf::from);
    dir.is_dir().then_some(dir)
}

/// The victims present in the corpus: `(name, Annex B bytes, access units)`.
fn victims(corpus: &Path) -> Vec<(String, Vec<u8>, Vec<(usize, usize)>)> {
    let mut out = Vec::new();
    for prefix in VICTIMS {
        // the corpus names carry a revision: PPS_A_1, PPS_A_4 ...
        let Some(name) = std::fs::read_dir(corpus)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.strip_prefix(prefix).is_some_and(|r| r.starts_with('_')))
            .min()
        else {
            continue;
        };
        let bit = corpus.join(&name).join(format!("{name}.bit"));
        let Ok(meta) = std::fs::metadata(&bit) else { continue };
        if meta.len() > MAX_STREAM {
            continue;
        }
        let data = std::fs::read(&bit).unwrap();
        let aus = cfa4_vvc::annexb_access_units(&data);
        if !aus.is_empty() {
            out.push((name, data, aus));
        }
    }
    out
}

/// A picture the adapter hands out must be what it says it is.
fn check_frame(f: &DecodedFrame) {
    let (w, h) = (f.frame.width as usize, f.frame.height as usize);
    assert!(w > 0 && h > 0, "empty picture");
    let (planes, shifts): (Vec<usize>, (u32, u32)) = match &f.frame.data {
        PixelData::Yuv8 { planes, chroma, .. } => (planes.iter().map(|p| p.len()).collect(), chroma.shifts()),
        PixelData::Yuv16 { planes, chroma, .. } => (planes.iter().map(|p| p.len()).collect(), chroma.shifts()),
        _ => panic!("unexpected pixel data"),
    };
    let (cw, ch) = (w.div_ceil(1 << shifts.0), h.div_ceil(1 << shifts.1));
    assert_eq!(planes[0], w * h, "luma plane size");
    for &p in &planes[1..] {
        assert!(p == 0 || p == cw * ch, "chroma plane size {p} for {cw}x{ch}");
    }
}

/// Decode `units` with `dec` (errors allowed); returns the pictures and the longest request.
fn feed(dec: &mut ExternalVvcDecoder, units: &[Vec<u8>]) -> (usize, Duration) {
    let mut pictures = 0;
    let mut longest = Duration::ZERO;
    for (i, u) in units.iter().enumerate() {
        let t = Instant::now();
        match dec.decode(u, i as i64) {
            Ok(frames) => {
                frames.iter().for_each(check_frame);
                pictures += frames.len();
            }
            Err(e) if std::env::var_os("CFA4_VVC_HOSTILE_VERBOSE").is_some() => eprintln!("unit {i}: {e}"),
            Err(_) => {}
        }
        longest = longest.max(t.elapsed());
    }
    let t = Instant::now();
    let frames = dec.flush();
    frames.iter().for_each(check_frame);
    pictures += frames.len();
    (pictures, longest.max(t.elapsed()))
}

fn clean_units(data: &[u8], aus: &[(usize, usize)]) -> Vec<Vec<u8>> {
    aus.iter().map(|&(o, l)| data[o..o + l].to_vec()).collect()
}

/// One damaged copy of the stream's access units. `kind` picks the damage (0..=13).
fn damage(rng: &mut Rng, kind: usize, units: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut u = units.to_vec();
    let n = u.len();
    match kind {
        // bit flips in a few units
        0 | 1 => {
            for _ in 0..1 + rng.below(8) {
                let i = rng.below(u.len());
                if !u[i].is_empty() {
                    let k = rng.below(u[i].len());
                    u[i][k] ^= 1 << rng.below(8);
                }
            }
        }
        // overwritten byte runs
        2 => {
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(u.len());
                let len = u[i].len();
                if len > 0 {
                    let (a, run) = (rng.below(len), 1 + rng.below(16));
                    let fill = [0u8, 0xff, rng.next() as u8][rng.below(3)];
                    for b in u[i].iter_mut().skip(a).take(run) {
                        *b = fill;
                    }
                }
            }
        }
        // inserted random bytes
        3 => {
            let i = rng.below(u.len());
            let at = rng.below(u[i].len() + 1);
            let extra: Vec<u8> = (0..1 + rng.below(64)).map(|_| rng.next() as u8).collect();
            u[i].splice(at..at, extra);
        }
        // damaged start codes and NAL headers
        4 => {
            for _ in 0..1 + rng.below(6) {
                let i = rng.below(u.len());
                let hits: Vec<usize> = u[i].windows(3).enumerate().filter(|(_, w)| *w == [0, 0, 1]).map(|(k, _)| k).collect();
                if let Some(&k) = hits.get(rng.below(hits.len().max(1))) {
                    let at = k + rng.below(6);
                    if at < u[i].len() {
                        u[i][at] = rng.next() as u8;
                    }
                }
            }
        }
        // truncated unit
        5 => {
            let i = rng.below(u.len());
            let keep = rng.below(u[i].len() + 1);
            u[i].truncate(keep);
        }
        // the stream cut short
        6 => u.truncate(rng.below(n + 1)),
        // dropped units
        7 => {
            for _ in 0..1 + rng.below(3) {
                if u.len() > 1 {
                    let i = rng.below(u.len());
                    u.remove(i);
                }
            }
        }
        // repeated units
        8 => {
            let i = rng.below(u.len());
            let copy = u[i].clone();
            u.insert(i, copy);
        }
        // swapped units
        9 => {
            if n > 1 {
                let (a, b) = (rng.below(n), rng.below(n));
                u.swap(a, b);
            }
        }
        // an empty unit and a unit of zeros
        10 => {
            let i = rng.below(u.len());
            u.insert(i, Vec::new());
            u.insert(i, vec![0; 1 + rng.below(4096)]);
        }
        // an oversized unit: a unit followed by megabytes of one byte or noise
        11 => {
            let i = rng.below(u.len());
            let big = 1 + rng.below(6 << 20);
            if rng.below(2) == 0 {
                let len = u[i].len();
                u[i].resize(len + big, 0xa5);
            } else {
                u[i].extend((0..big).map(|_| rng.next() as u8));
            }
        }
        // garbage that is not VVC at all
        12 => {
            let i = rng.below(u.len());
            u[i] = (0..1 + rng.below(100_000)).map(|_| rng.next() as u8).collect();
        }
        // a unit with a start code every few bytes (many tiny NAL units)
        _ => {
            let i = rng.below(u.len());
            let mut v = Vec::new();
            for _ in 0..1 + rng.below(20_000) {
                v.extend_from_slice(&[0, 0, 1, rng.next() as u8, rng.next() as u8, rng.next() as u8]);
            }
            u[i] = v;
        }
    }
    u
}

#[test]
fn damaged_conformance_streams_never_hurt_the_helper_or_cfa4() {
    let ctx = "vvc_hostile::damaged_conformance_streams_never_hurt_the_helper_or_cfa4";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some(corpus) = corpus_dir() else {
        return eprintln!("SKIPPED ({ctx}): no corpus (scripts/dev/vvc-conformance.py fetch, or CFA4_VVC_CONFORMANCE_DIR)");
    };
    let victims = victims(&corpus);
    if victims.is_empty() {
        return eprintln!("SKIPPED ({ctx}): none of the victim streams is in the corpus");
    }
    let budget = Duration::from_secs(std::env::var("CFA4_VVC_HOSTILE_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20));
    let seed: u64 = std::env::var("CFA4_VVC_HOSTILE_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x2545_f491_4f6c_dd1d) | 1;
    let limit = Duration::from_secs(30);

    let mut dec =
        ExternalVvcDecoder::new(&helper, std::env::var("CFA4_VVC_HOSTILE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(0)).expect("start the helper");
    dec.set_request_timeout(limit);
    // what each victim gives clean: the pictures it must give again after all the damage; a
    // stream the helper does not decode (a corpus stream outside Main 10) is no victim
    let mut victims = victims;
    let mut clean = Vec::new();
    victims.retain(|(name, data, aus)| {
        dec.reset();
        let (n, _) = feed(&mut dec, &clean_units(data, aus));
        if n == 0 {
            eprintln!("vvc_hostile: {name} gives no picture clean, left out");
        } else {
            clean.push(n);
        }
        n > 0
    });
    assert!(victims.len() >= 5, "only {} usable victim streams", victims.len());

    let start = Instant::now();
    let (mut rounds, mut pictures, mut longest) = (0usize, 0usize, Duration::ZERO);
    let mut rng = Rng(seed);
    while start.elapsed() < budget {
        let v = rng.below(victims.len());
        let (name, data, aus) = &victims[v];
        let kind = rounds % 14;
        let units = damage(&mut rng, kind, &clean_units(data, aus));
        dec.reset();
        let (n, took) = feed(&mut dec, &units);
        pictures += n;
        longest = longest.max(took);
        assert!(took < limit, "{name}: a request took {took:?} (round {rounds}, damage {kind}, seed {seed})");
        rounds += 1;
        // every few rounds: the decoder still does its job on the clean stream
        if rounds % 25 == 0 {
            dec.reset();
            let (n, _) = feed(&mut dec, &clean_units(data, aus));
            assert_eq!(n, clean[v], "{name}: a clean stream gives {n} pictures after damage (round {rounds}, seed {seed})");
        }
    }
    for (v, (name, data, aus)) in victims.iter().enumerate() {
        dec.reset();
        let (n, _) = feed(&mut dec, &clean_units(data, aus));
        assert_eq!(n, clean[v], "{name}: a clean stream gives {n} pictures after the campaign (seed {seed})");
    }
    eprintln!("vvc_hostile: {rounds} damaged streams from {} victims, {pictures} pictures out, longest request {longest:?}, seed {seed}", victims.len());
    assert!(rounds >= 14, "too few rounds ({rounds}) to cover every kind of damage");
}

#[test]
fn a_helper_that_is_killed_is_replaced() {
    let ctx = "vvc_hostile::a_helper_that_is_killed_is_replaced";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some(corpus) = corpus_dir() else {
        return eprintln!("SKIPPED ({ctx}): no corpus (scripts/dev/vvc-conformance.py fetch, or CFA4_VVC_CONFORMANCE_DIR)");
    };
    let victims = victims(&corpus);
    let Some((name, data, aus)) = victims.first() else {
        return eprintln!("SKIPPED ({ctx}): none of the victim streams is in the corpus");
    };
    let units = clean_units(data, aus);
    // a private copy of the helper, so that the kill cannot reach another test's decoder
    let private = std::env::temp_dir().join(format!("cfa4-vvc-decode-kill-{}", std::process::id()));
    std::fs::copy(&helper, &private).unwrap();
    let helper = private;
    let mut dec = ExternalVvcDecoder::new(&helper, 1).unwrap();
    let (want, _) = feed(&mut dec, &units);
    assert!(want > 0, "{name}");
    // the decoder is the helper's process: kill it from outside between two requests
    let killed = std::process::Command::new("pkill").args(["-KILL", "-f", &helper.to_string_lossy()]).status().is_ok_and(|s| s.success());
    if !killed {
        return eprintln!("SKIPPED ({ctx}): pkill not available");
    }
    dec.reset();
    // the first request after the kill may fail; the next ones must work again
    let _ = dec.decode(&units[0], 0);
    dec.reset();
    let (got, _) = feed(&mut dec, &units);
    let _ = std::fs::remove_file(&helper);
    assert_eq!(got, want, "{name}: pictures after the helper was killed");
}

/// A fake helper: a shell script that says hello (with the VVC tag) and then misbehaves. These
/// tests need no VVdeC: they show what CFA4 does when the helper process itself goes wrong.
fn fake_helper(name: &str, body: &str) -> PathBuf {
    let dir = cfa4_testkit::fixtures_dir("vvc/fake-helpers");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let script = format!("#!/bin/sh\nprintf 'CFA4VVCD\\001\\000\\000\\000\\004\\000\\000\\000fake'\n{body}\n");
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// An IDR_N_LP slice unit with its picture header (a unit the adapter sends on).
const UNIT: &[u8] = &[0, 0, 0, 1, 0, (8 << 3) | 1, 0xc0, 0x80];

#[test]
fn a_crashing_helper_is_an_error_and_is_restarted() {
    let path = fake_helper("crash.sh", "head -c 8 >/dev/null\nkill -SEGV $$");
    let mut dec = ExternalVvcDecoder::new(&path, 0).unwrap();
    assert!(dec.name().contains("fake"));
    assert!(dec.decode(UNIT, 0).is_err());
    // the next call starts a new helper, which fails the same way: an error again, no panic
    assert!(dec.decode(UNIT, 1).is_err());
    assert!(dec.flush().is_empty());
    dec.reset();
}

#[test]
fn garbage_from_the_helper_is_an_error() {
    // FRME of 10 bytes (shorter than its header), then DONE
    let path = fake_helper("garbage.sh", "head -c 8 >/dev/null\nprintf 'FRME\\012\\000\\000\\0000123456789DONE\\000\\000\\000\\000'\nsleep 5");
    let mut dec = ExternalVvcDecoder::new(&path, 0).unwrap();
    assert!(dec.decode(UNIT, 0).is_err());
    let path = fake_helper("unknown.sh", "head -c 8 >/dev/null\nprintf 'WHAT\\000\\000\\000\\000'\nsleep 5");
    let mut dec = ExternalVvcDecoder::new(&path, 0).unwrap();
    assert!(dec.decode(UNIT, 0).is_err());
    // a frame that claims to be 65535 x 65535 pixels but carries no data
    let path = fake_helper(
        "huge.sh",
        "head -c 8 >/dev/null\nprintf 'FRME\\030\\000\\000\\000\\377\\377\\000\\000\\377\\377\\000\\000\\001\\012\\002\\002\\002\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000\\000DONE\\000\\000\\000\\000'\nsleep 5",
    );
    let mut dec = ExternalVvcDecoder::new(&path, 0).unwrap();
    assert!(dec.decode(UNIT, 0).is_err());
}

#[test]
fn a_hanging_helper_times_out_and_the_caller_is_not_held() {
    let path = fake_helper("hang.sh", "sleep 30");
    let mut dec = ExternalVvcDecoder::new(&path, 0).unwrap();
    dec.set_request_timeout(Duration::from_millis(300));
    // the decode runs on a worker, as in playback; the thread that asks (the UI) stays free
    let (tx, rx) = std::sync::mpsc::channel();
    let t = Instant::now();
    let worker = std::thread::spawn(move || {
        let r = dec.decode(UNIT, 0);
        tx.send(r.is_err()).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_secs(10)).expect("the request must end by its timeout"));
    assert!(t.elapsed() < Duration::from_secs(10));
    worker.join().unwrap();
}

#[test]
fn a_memory_cap_turns_huge_pictures_into_an_error() {
    let ctx = "vvc_hostile::a_memory_cap_turns_huge_pictures_into_an_error";
    let Some(helper) = helper() else {
        return eprintln!("SKIPPED ({ctx}): cfa4-vvc-decode not built (scripts/dev/vvc-decoder.py build)");
    };
    let Some(corpus) = corpus_dir() else {
        return eprintln!("SKIPPED ({ctx}): no corpus (scripts/dev/vvc-conformance.py fetch, or CFA4_VVC_CONFORMANCE_DIR)");
    };
    // a 1080p 10-bit stream needs far more than the 48 MiB the wrapper allows
    let bit = corpus.join("LFNST_D_3").join("LFNST_D_3.bit");
    let Ok(data) = std::fs::read(&bit) else {
        return eprintln!("SKIPPED ({ctx}): LFNST_D_3 not in the corpus");
    };
    let aus = cfa4_vvc::annexb_access_units(&data);
    let dir = cfa4_testkit::fixtures_dir("vvc/fake-helpers");
    std::fs::create_dir_all(&dir).unwrap();
    let wrapper = dir.join("capped.sh");
    std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --memory-mb 128\n", helper.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut dec = match ExternalVvcDecoder::new(&wrapper, 1) {
        Ok(d) => d,
        // an older helper without the option refuses it: nothing to test
        Err(e) => return eprintln!("SKIPPED ({ctx}): {e}"),
    };
    dec.set_request_timeout(Duration::from_secs(60));
    let t = Instant::now();
    let mut errors = 0;
    for (i, &(o, l)) in aus.iter().take(12).enumerate() {
        if dec.decode(&data[o..o + l], i as i64).is_err() {
            errors += 1;
        }
    }
    let _ = dec.flush();
    assert!(errors > 0, "48 MiB should not be enough for 1080p 10-bit");
    assert!(t.elapsed() < Duration::from_secs(60));
    // and the same decoder object still works for a small stream afterwards
    if let Some((_, d, units)) = victims(&corpus).first() {
        dec.reset();
        let mut n = 0;
        for (i, &(o, l)) in units.iter().enumerate() {
            n += dec.decode(&d[o..o + l], i as i64).map_or(0, |f| f.len());
        }
        n += dec.flush().len();
        assert!(n > 0, "no picture from a small stream after the cap was hit");
    }
}
