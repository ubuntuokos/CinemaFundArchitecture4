//! AV2 oracle harness (AV2 plan II.3, X/3): the pinned AVM `avmdec` as a bit-exact reference.
//!
//! - [`Corpus`]: the committed manifest (`data/av2-corpus.json`): pinned AVM commits, synthetic
//!   sources, avmenc arguments and, per vector, the bitstream SHA-256 and the per-frame,
//!   per-plane SHA-256 of the AVM v1.0.0 output. Only hashes are committed, never media.
//! - [`avmdec`]: the pinned decoder, from `CFA4_AVMDEC` or the private prefix
//!   (`CFA4_AVM_PREFIX`, default `<workspace>/target/av2-oracle/avm-v1.0.0`) whose `AVM_COMMIT`
//!   stamp must name the pinned commit. `PATH` is never searched: another AVM build is not the
//!   oracle.
//! - [`vector_bitstream`]: a corpus bitstream from `<fixtures>/av2/vectors`, accepted only if
//!   its SHA-256 matches the manifest.
//! - [`y4m_plane_hashes`], [`compare`]: hash a decoder's Y4M output and compare it with the
//!   manifest (geometry, frame count, every plane of every frame).
//!
//! `scripts/dev/av2-oracle.py` builds the oracle and the corpus. A missing oracle or corpus is a
//! `SKIPPED` test, unless `CFA4_REQUIRE_AV2_ORACLE=1` (the AV2 oracle workflow) makes it a
//! failure. The general `CFA4_REQUIRE_ORACLES` does not cover AV2: ordinary CI has no AVM build.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Environment variable that makes a missing AVM oracle or corpus a hard failure.
pub const REQUIRE_ENV: &str = "CFA4_REQUIRE_AV2_ORACLE";

/// The committed corpus manifest.
pub const MANIFEST_JSON: &str = include_str!("../data/av2-corpus.json");

/// Longest a single `avmdec` run may take before it is killed (the 1080p vector takes seconds).
pub const DECODE_TIMEOUT: Duration = Duration::from_secs(300);

/// Largest Y4M output accepted from a decoder (the corpus needs about 10 MB).
pub const MAX_Y4M_BYTES: u64 = 512 << 20;

/// A pinned AVM revision.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Pin {
    #[serde(rename = "ref")]
    pub reference: String,
    pub commit: String,
    pub repo: String,
}

/// A synthetic ffmpeg source.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub lavfi: String,
    pub pix_fmt: String,
    pub frames: usize,
}

/// Decoded picture geometry, normalised so equivalent Y4M colour tags compare equal.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    /// `"420"`, `"422"`, `"444"` or `"400"`.
    pub chroma: String,
    pub bytes_per_sample: u32,
}

/// One corpus vector with its reference hashes.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Vector {
    pub name: String,
    pub source: String,
    pub encoder_args: Vec<String>,
    pub purpose: String,
    pub profile: String,
    pub bitstream_sha256: String,
    pub bitstream_bytes: u64,
    pub geometry: Geometry,
    /// `frames[i][p]`: SHA-256 (hex) of plane `p` of output frame `i`.
    pub frames: Vec<Vec<String>>,
}

/// The corpus manifest.
#[derive(Clone, Debug, Deserialize)]
pub struct Corpus {
    pub schema: u32,
    pub oracle: Pin,
    pub cross_check: Vec<Pin>,
    /// Which picture the frame hashes cover (`"output"`: after film grain synthesis).
    pub hash_stage: String,
    pub sources: std::collections::BTreeMap<String, Source>,
    pub encoder_base_args: Vec<String>,
    pub vectors: Vec<Vector>,
}

impl Corpus {
    /// Parse the committed manifest.
    pub fn load() -> Result<Self, String> {
        Self::parse(MANIFEST_JSON)
    }

    /// Parse and validate a manifest.
    pub fn parse(json: &str) -> Result<Self, String> {
        let corpus: Self = serde_json::from_str(json).map_err(|e| format!("AV2 corpus manifest: {e}"))?;
        corpus.validate()?;
        Ok(corpus)
    }

    /// The vector called `name`.
    pub fn vector(&self, name: &str) -> Option<&Vector> {
        self.vectors.iter().find(|v| v.name == name)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err(format!("unsupported AV2 corpus schema {}", self.schema));
        }
        for pin in std::iter::once(&self.oracle).chain(&self.cross_check) {
            if !is_hex(&pin.commit, 40) {
                return Err(format!("AVM {}: commit `{}` is not a full SHA-1", pin.reference, pin.commit));
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for v in &self.vectors {
            if !names.insert(v.name.as_str()) {
                return Err(format!("duplicate vector {}", v.name));
            }
            let src = self.sources.get(&v.source).ok_or_else(|| format!("{}: unknown source {}", v.name, v.source))?;
            if v.frames.len() != src.frames {
                return Err(format!("{}: {} frame hashes for a {}-frame source", v.name, v.frames.len(), src.frames));
            }
            if !is_hex(&v.bitstream_sha256, 64) {
                return Err(format!("{}: bad bitstream hash", v.name));
            }
            let planes = plane_count(&v.geometry.chroma).ok_or_else(|| format!("{}: unknown chroma {}", v.name, v.geometry.chroma))?;
            for (i, f) in v.frames.iter().enumerate() {
                if f.len() != planes || !f.iter().all(|h| is_hex(h, 64)) {
                    return Err(format!("{}: frame {i} needs {planes} SHA-256 plane hashes", v.name));
                }
            }
        }
        Ok(())
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn plane_count(chroma: &str) -> Option<usize> {
    match chroma {
        "400" => Some(1),
        "420" | "422" | "444" => Some(3),
        _ => None,
    }
}

/// Lowercase hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `CFA4_REQUIRE_AV2_ORACLE` is set to a true value.
pub fn oracle_required() -> bool {
    std::env::var(REQUIRE_ENV).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Report a skipped AV2 oracle test (`SKIPPED (<context>): <reason>`); with
/// `CFA4_REQUIRE_AV2_ORACLE=1` it panics instead, so the oracle workflow cannot pass by skipping.
pub fn skip(context: &str, reason: &str) {
    if oracle_required() {
        panic!("{context}: {reason}, and {REQUIRE_ENV}=1 forbids skipping AV2 oracle tests");
    }
    eprintln!("SKIPPED ({context}): {reason}");
}

/// Default private prefix of a pinned AVM build: `<workspace>/target/av2-oracle/avm-<ref>`.
pub fn default_prefix(reference: &str) -> PathBuf {
    crate::workspace_root().join("target").join("av2-oracle").join(format!("avm-{reference}"))
}

/// Find `avmdec` of `pin` given an explicit binary and a prefix (pure function of its inputs).
///
/// An explicit path must be a file. A prefix counts only when its `AVM_COMMIT` stamp names the
/// pinned commit, so a stale or different build is never used as the oracle.
pub fn find_avmdec_in(pin: &Pin, explicit: Option<&Path>, prefix: &Path) -> Result<Option<PathBuf>, String> {
    if let Some(p) = explicit.filter(|p| !p.as_os_str().is_empty()) {
        return if p.is_file() { Ok(Some(p.to_path_buf())) } else { Err(format!("`{}` is not a file", p.display())) };
    }
    let bin = prefix.join("bin").join("avmdec");
    if !bin.is_file() {
        return Ok(None);
    }
    let stamp = std::fs::read_to_string(prefix.join("AVM_COMMIT")).unwrap_or_default();
    if stamp.trim() != pin.commit {
        return Err(format!("{} is not an AVM {} build (AVM_COMMIT `{}`, expected {})", prefix.display(), pin.reference, stamp.trim(), pin.commit));
    }
    Ok(Some(bin))
}

/// The pinned oracle `avmdec` (v1.0.0): `CFA4_AVMDEC`, else `CFA4_AVM_PREFIX` / the default prefix.
pub fn avmdec(corpus: &Corpus) -> Result<Option<PathBuf>, String> {
    let explicit = std::env::var_os("CFA4_AVMDEC").map(PathBuf::from);
    let prefix = std::env::var_os("CFA4_AVM_PREFIX").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| default_prefix(&corpus.oracle.reference));
    find_avmdec_in(&corpus.oracle, explicit.as_deref(), &prefix).map_err(|e| format!("CFA4_AVMDEC/CFA4_AVM_PREFIX: {e}"))
}

/// The cross-check `avmdec` of `pin` (`av2-normative`): `CFA4_AVMDEC_NORMATIVE`, else its default prefix.
pub fn cross_check_avmdec(pin: &Pin) -> Result<Option<PathBuf>, String> {
    let explicit = std::env::var_os("CFA4_AVMDEC_NORMATIVE").map(PathBuf::from);
    find_avmdec_in(pin, explicit.as_deref(), &default_prefix(&pin.reference)).map_err(|e| format!("CFA4_AVMDEC_NORMATIVE: {e}"))
}

/// Directory of the generated corpus bitstreams: `<fixtures root>/av2/vectors`.
pub fn vectors_dir() -> PathBuf {
    crate::fixtures::fixtures_root().join("av2").join("vectors")
}

/// `<dir>/<name>.obu` if it exists; an error if its SHA-256 differs from the manifest.
pub fn vector_bitstream_in(dir: &Path, v: &Vector) -> Result<Option<PathBuf>, String> {
    let path = dir.join(format!("{}.obu", v.name));
    let Ok(data) = std::fs::read(&path) else { return Ok(None) };
    let got = sha256_hex(&data);
    if got != v.bitstream_sha256 {
        return Err(format!("{}: SHA-256 {got} differs from the manifest {}", path.display(), v.bitstream_sha256));
    }
    Ok(Some(path))
}

/// The corpus bitstream of `v` from [`vectors_dir`] (see [`vector_bitstream_in`]).
pub fn vector_bitstream(v: &Vector) -> Result<Option<PathBuf>, String> {
    vector_bitstream_in(&vectors_dir(), v)
}

/// Geometry and per-frame, per-plane SHA-256 of a Y4M stream.
///
/// Errors on a malformed header, an unknown colour space, a truncated frame or trailing bytes.
pub fn y4m_plane_hashes(data: &[u8]) -> Result<(Geometry, Vec<Vec<String>>), String> {
    let header_end = data.iter().position(|&b| b == b'\n').ok_or("not a Y4M stream: no header line")?;
    let header = data.get(..header_end).ok_or("bad header")?;
    let mut fields = header.split(|&b| b == b' ').filter(|f| !f.is_empty());
    if fields.next() != Some(b"YUV4MPEG2".as_slice()) {
        return Err("not a Y4M stream: missing YUV4MPEG2 signature".into());
    }
    let (mut width, mut height, mut tag) = (0u32, 0u32, "420jpeg".to_string());
    for f in fields {
        let (key, value) = f.split_first().ok_or("empty header field")?;
        let value = std::str::from_utf8(value).map_err(|_| "non-UTF-8 header field")?;
        match key {
            b'W' => width = value.parse().map_err(|_| format!("bad width `{value}`"))?,
            b'H' => height = value.parse().map_err(|_| format!("bad height `{value}`"))?,
            b'C' => tag = value.to_string(),
            _ => {}
        }
    }
    if width == 0 || height == 0 {
        return Err(format!("bad picture size {width}x{height}"));
    }
    let bps: u32 = if ["p10", "p12", "p16"].iter().any(|d| tag.contains(d)) { 2 } else { 1 };
    let (w, h) = (u64::from(width), u64::from(height));
    let luma = w.checked_mul(h).ok_or("picture too large")?;
    let (chroma, planes): (&str, Vec<u64>) = if tag.starts_with("mono") {
        ("400", vec![luma])
    } else if tag.starts_with("444") {
        ("444", vec![luma; 3])
    } else if tag.starts_with("422") {
        let c = w.div_ceil(2) * h;
        ("422", vec![luma, c, c])
    } else if tag.starts_with("420") {
        let c = w.div_ceil(2) * h.div_ceil(2);
        ("420", vec![luma, c, c])
    } else {
        return Err(format!("unsupported Y4M colour space C{tag}"));
    };
    let planes: Vec<usize> =
        planes.iter().map(|&n| n.checked_mul(u64::from(bps)).and_then(|b| usize::try_from(b).ok()).ok_or("picture too large")).collect::<Result<_, _>>()?;
    let frame_size = planes.iter().try_fold(0usize, |acc, &n| acc.checked_add(n)).ok_or("picture too large")?;
    let mut frames = Vec::new();
    let mut pos = header_end + 1;
    while pos < data.len() {
        let rest = data.get(pos..).ok_or("bad offset")?;
        let line_end = rest.iter().position(|&b| b == b'\n').ok_or_else(|| format!("frame {}: no FRAME header", frames.len()))?;
        if !rest.starts_with(b"FRAME") {
            return Err(format!("bad frame header after {} frames", frames.len()));
        }
        pos += line_end + 1;
        let end = pos.checked_add(frame_size).filter(|&e| e <= data.len()).ok_or_else(|| format!("frame {} truncated", frames.len()))?;
        let mut digests = Vec::with_capacity(planes.len());
        let mut p = pos;
        for &size in &planes {
            digests.push(sha256_hex(data.get(p..p + size).ok_or("plane out of range")?));
            p += size;
        }
        frames.push(digests);
        pos = end;
    }
    Ok((Geometry { width, height, chroma: chroma.to_string(), bytes_per_sample: bps }, frames))
}

/// Result of comparing a decoder's output with the reference hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub geometry_match: bool,
    pub reference_frames: usize,
    pub candidate_frames: usize,
    /// Indices of output frames whose planes differ (all of them, in order).
    pub mismatched_frames: Vec<usize>,
}

impl Comparison {
    /// Bit-exact: same geometry, same frame count, every plane of every frame equal.
    pub fn pass(&self) -> bool {
        self.geometry_match && self.reference_frames > 0 && self.reference_frames == self.candidate_frames && self.mismatched_frames.is_empty()
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pass() {
            return write!(f, "PASS ({} frames bit-exact)", self.reference_frames);
        }
        write!(
            f,
            "FAIL (geometry {}, frames {}/{}, mismatched {:?})",
            if self.geometry_match { "ok" } else { "differs" },
            self.candidate_frames,
            self.reference_frames,
            self.mismatched_frames
        )
    }
}

/// Compare a candidate's geometry and plane hashes with vector `v`.
pub fn compare(v: &Vector, geometry: &Geometry, frames: &[Vec<String>]) -> Comparison {
    Comparison {
        geometry_match: &v.geometry == geometry,
        reference_frames: v.frames.len(),
        candidate_frames: frames.len(),
        mismatched_frames: v.frames.iter().zip(frames).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect(),
    }
}

/// Decode `bitstream` with an `avmdec` binary into a temporary Y4M and hash it.
///
/// The run is killed after [`DECODE_TIMEOUT`]; an output over [`MAX_Y4M_BYTES`] is rejected.
pub fn decode_with_avmdec(avmdec: &Path, bitstream: &Path) -> Result<(Geometry, Vec<Vec<String>>), String> {
    let dir = tempfile_dir()?;
    let out = dir.join("out.y4m");
    let log = dir.join("stderr.log");
    let result = (|| {
        let stderr = std::fs::File::create(&log).map_err(|e| format!("create {}: {e}", log.display()))?;
        let mut child = Command::new(avmdec)
            .arg("-o")
            .arg(&out)
            .arg(bitstream)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("launch {}: {e}", avmdec.display()))?;
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().map_err(|e| format!("wait: {e}"))? {
                break s;
            }
            if start.elapsed() > DECODE_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("avmdec timed out after {DECODE_TIMEOUT:?}"));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if !status.success() {
            let err = std::fs::read_to_string(&log).unwrap_or_default();
            let tail: String = err.trim().chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
            return Err(format!("avmdec failed ({status}): {tail}"));
        }
        let len = std::fs::metadata(&out).map_err(|e| format!("avmdec wrote no output: {e}"))?.len();
        if len > MAX_Y4M_BYTES {
            return Err(format!("avmdec output is {len} bytes, over the {MAX_Y4M_BYTES}-byte limit"));
        }
        let data = std::fs::read(&out).map_err(|e| format!("read {}: {e}", out.display()))?;
        y4m_plane_hashes(&data)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn tempfile_dir() -> Result<PathBuf, String> {
    let base = crate::fixtures::fixtures_root().join("av2").join("tmp");
    let dir = crate::fixtures::temp_path(&base.join("decode"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y4m(tag: &str, w: usize, h: usize, frames: &[Vec<u8>]) -> Vec<u8> {
        let mut out = format!("YUV4MPEG2 W{w} H{h} F30:1 Ip A1:1 C{tag}\n").into_bytes();
        for f in frames {
            out.extend_from_slice(b"FRAME\n");
            out.extend_from_slice(f);
        }
        out
    }

    #[test]
    fn committed_manifest_is_consistent() {
        let c = Corpus::load().unwrap();
        assert_eq!(c.oracle.reference, "v1.0.0");
        assert_eq!(c.oracle.commit, "966a7d7cd6fcf60360caf5dc413b2aeeb65e144d");
        assert_eq!(c.cross_check.first().map(|p| p.reference.as_str()), Some("av2-normative"));
        assert_eq!(c.hash_stage, "output");
        assert_eq!(c.vectors.len(), 13);
        assert!(c.vector("422-10bit").is_some_and(|v| v.geometry.chroma == "422" && v.geometry.bytes_per_sample == 2));
        assert!(c.vector("420-8bit-resize").is_some_and(|v| (v.geometry.width, v.geometry.height) == (176, 144)));
        assert!(c.vector("420-10bit-default").is_some_and(|v| v.geometry.bytes_per_sample == 2));
        assert!(c.vector("400-8bit").is_some_and(|v| v.frames.iter().all(|f| f.len() == 1)));
    }

    #[test]
    fn manifest_validation_rejects_inconsistent_entries() {
        let good: serde_json::Value = serde_json::from_str(MANIFEST_JSON).unwrap();
        let broken = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut v = good.clone();
            edit(&mut v);
            Corpus::parse(&v.to_string()).unwrap_err()
        };
        assert!(broken(&|v| v["oracle"]["commit"] = "966a7d7".into()).contains("full SHA-1"));
        assert!(broken(&|v| v["vectors"][0]["frames"].as_array_mut().unwrap().truncate(3)).contains("frame hashes"));
        assert!(broken(&|v| v["vectors"][0]["source"] = "nope".into()).contains("unknown source"));
        assert!(broken(&|v| v["vectors"][7]["frames"][0] = serde_json::json!(["00"])).contains("plane hashes"));
        assert!(broken(&|v| v["vectors"][1]["name"] = "420-8bit-default".into()).contains("duplicate"));
        assert!(Corpus::parse("{").is_err());
    }

    #[test]
    fn y4m_hashes_every_plane_of_every_frame() {
        // 3x3 4:2:0: luma 9 bytes, chroma 2x2 = 4 bytes each.
        let f0: Vec<u8> = (0..17).collect();
        let f1: Vec<u8> = (100..117).collect();
        let (g, frames) = y4m_plane_hashes(&y4m("420jpeg", 3, 3, &[f0.clone(), f1])).unwrap();
        assert_eq!(g, Geometry { width: 3, height: 3, chroma: "420".into(), bytes_per_sample: 1 });
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], vec![sha256_hex(&f0[..9]), sha256_hex(&f0[9..13]), sha256_hex(&f0[13..])]);
        // C420 and C420jpeg are the same layout
        assert_eq!(y4m_plane_hashes(&y4m("420", 3, 3, &[f0])).unwrap().0.chroma, "420");
    }

    #[test]
    fn y4m_layouts_and_bit_depths() {
        let mono = y4m_plane_hashes(&y4m("mono", 4, 2, &[vec![1; 8]])).unwrap();
        assert_eq!((mono.0.chroma.as_str(), mono.1[0].len()), ("400", 1));
        let p10 = y4m_plane_hashes(&y4m("420p10", 2, 2, &[vec![0; 12]])).unwrap();
        assert_eq!(p10.0.bytes_per_sample, 2);
        let c444 = y4m_plane_hashes(&y4m("444", 2, 2, &[vec![0; 12]])).unwrap();
        assert_eq!(c444.0.chroma, "444");
        let c422 = y4m_plane_hashes(&y4m("422", 3, 2, &[vec![0; 6 + 4 + 4]])).unwrap();
        assert_eq!(c422.0.chroma, "422");
    }

    #[test]
    fn y4m_rejects_hostile_input_without_panicking() {
        assert!(y4m_plane_hashes(b"").is_err());
        assert!(y4m_plane_hashes(b"YUV4MPEG2 W2 H2").is_err());
        assert!(y4m_plane_hashes(b"RIFF W2 H2\n").is_err());
        assert!(y4m_plane_hashes(b"YUV4MPEG2 W0 H2\n").is_err());
        assert!(y4m_plane_hashes(b"YUV4MPEG2 Wx H2\n").is_err());
        assert!(y4m_plane_hashes(b"YUV4MPEG2 W99999999999 H2\n").is_err());
        assert!(y4m_plane_hashes(&y4m("411", 2, 2, &[vec![0; 6]])).is_err());
        assert!(y4m_plane_hashes(&y4m("420", 2, 2, &[vec![0; 5]])).is_err(), "truncated frame");
        let mut trailing = y4m("420", 2, 2, &[vec![0; 6]]);
        trailing.extend_from_slice(b"junk");
        assert!(y4m_plane_hashes(&trailing).is_err(), "trailing bytes");
        // A huge declared size with no data is a truncation error, not an allocation.
        assert!(y4m_plane_hashes(b"YUV4MPEG2 W4000000 H4000000 C444\nFRAME\n").is_err());
        // Header only: zero frames is valid Y4M.
        assert_eq!(y4m_plane_hashes(b"YUV4MPEG2 W2 H2 C420\n").unwrap().1.len(), 0);
    }

    #[test]
    fn comparison_reports_every_kind_of_difference() {
        let c = Corpus::load().unwrap();
        let v = c.vector("420-8bit-odd-size").unwrap();
        let same = compare(v, &v.geometry, &v.frames);
        assert!(same.pass(), "{same}");
        let mut frames = v.frames.clone();
        frames[2][1] = "0".repeat(64);
        frames[4][0] = "0".repeat(64);
        let diff = compare(v, &v.geometry, &frames);
        assert_eq!(diff.mismatched_frames, vec![2, 4]);
        assert!(!diff.pass() && diff.to_string().contains("[2, 4]"));
        assert!(!compare(v, &v.geometry, &v.frames[..5]).pass(), "missing frame");
        let mut g = v.geometry.clone();
        g.width += 1;
        assert!(!compare(v, &g, &v.frames).pass(), "geometry");
    }

    #[test]
    fn avmdec_lookup_requires_the_pinned_stamp() {
        let c = Corpus::load().unwrap();
        let d = tempfile::tempdir().unwrap();
        assert_eq!(find_avmdec_in(&c.oracle, None, d.path()).unwrap(), None);
        std::fs::create_dir_all(d.path().join("bin")).unwrap();
        std::fs::write(d.path().join("bin/avmdec"), b"").unwrap();
        assert!(find_avmdec_in(&c.oracle, None, d.path()).is_err(), "no stamp");
        std::fs::write(d.path().join("AVM_COMMIT"), "14f43e4cd8e1dcfcbb709bcf6e08001a5f188845\n").unwrap();
        assert!(find_avmdec_in(&c.oracle, None, d.path()).is_err(), "wrong commit");
        std::fs::write(d.path().join("AVM_COMMIT"), format!("{}\n", c.oracle.commit)).unwrap();
        assert_eq!(find_avmdec_in(&c.oracle, None, d.path()).unwrap(), Some(d.path().join("bin/avmdec")));
        // an explicit binary wins; a wrong explicit path is an error, not a skip
        assert!(find_avmdec_in(&c.oracle, Some(&d.path().join("nope")), d.path()).is_err());
        assert_eq!(find_avmdec_in(&c.oracle, Some(&d.path().join("bin/avmdec")), Path::new("/nonexistent")).unwrap(), Some(d.path().join("bin/avmdec")));
    }

    #[test]
    fn bitstream_must_match_the_manifest() {
        let c = Corpus::load().unwrap();
        let v = c.vector("400-8bit").unwrap();
        let d = tempfile::tempdir().unwrap();
        assert_eq!(vector_bitstream_in(d.path(), v).unwrap(), None);
        std::fs::write(d.path().join("400-8bit.obu"), b"not the vector").unwrap();
        assert!(vector_bitstream_in(d.path(), v).unwrap_err().contains("differs from the manifest"));
    }
}
