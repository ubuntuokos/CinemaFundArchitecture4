//! Render validation harness (CFA4-RENDER-INTEGRATION-PLAN-001, phase F0V).
//!
//! A [`Capture`] is what a render produced: display-encoded 16-bit RGBA frames, one presentation
//! time per frame, and the audio. [`compare`] measures a capture against a reference with the
//! `cfa4-metrics` figures (per-plane difference and PSNR, SSIM, ΔE₀₀, PTS error, dropped frames,
//! audio null test) and gates each one on a [`Thresholds`] value, producing a [`Report`].
//!
//! Three rules keep the harness honest:
//!
//! * **A check that was not measured never passes.** A gating check that is [`Status::Skipped`]
//!   makes the verdict [`Verdict::Incomplete`], not [`Verdict::Pass`].
//! * **Thresholds come from measured noise, not from a chosen number.** [`derive_thresholds`] turns
//!   at least [`MIN_NOISE_RUNS`] runs on an unchanged build into limits by a fixed, documented rule.
//! * **The harness proves it can fail.** [`mutation::self_test`] damages a good capture in six
//!   known ways (dropped frame, BT.601/709 matrix swap, one-frame PTS shift, lost alpha, rounding to
//!   8 bit, one muted audio channel) and every one must be reported as a failure.

use std::fmt::Write as _;

use cfa4_metrics::{MetricError, audio, delta_e_srgb8, diff_u16, ssim_u16, timing};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Schema tag of [`Report`] and [`Thresholds`] JSON.
pub const SCHEMA: &str = "cfa4.render-validation/1";

/// Fewest unchanged-build runs [`derive_thresholds`] accepts (plan F0V: "≥ 10 runs").
pub const MIN_NOISE_RUNS: usize = 10;

/// PSNR floor used when the measured noise is exactly zero: one 16-bit code in 65 535 is 96.3 dB.
pub const PSNR_IDENTICAL_FLOOR_DB: f64 = 96.0;

/// Audio residual limit meaning "silence": far below 24-bit full scale (−144 dBFS).
pub const AUDIO_SILENCE_DBFS: f64 = -200.0;

/// One rendered frame: straight-alpha, display-encoded RGBA, 16 bits per channel, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameCapture {
    pub width: usize,
    pub height: usize,
    pub rgba16: Vec<u16>,
}

impl FrameCapture {
    /// A frame; `None` when `rgba16` is not `width × height × 4` samples.
    pub fn new(width: usize, height: usize, rgba16: Vec<u16>) -> Option<FrameCapture> {
        (width.checked_mul(height)?.checked_mul(4)? == rgba16.len()).then_some(FrameCapture { width, height, rgba16 })
    }

    /// One channel (0 = R … 3 = A) as its own plane.
    pub fn plane(&self, channel: usize) -> Vec<u16> {
        self.rgba16.chunks_exact(4).filter_map(|p| p.get(channel).copied()).collect()
    }

    /// The frame rounded to 8 bits, RGBA interleaved.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.rgba16.iter().map(|&v| ((u32::from(v) * 255 + 32_767) / 65_535) as u8).collect()
    }
}

/// Everything one render produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub frames: Vec<FrameCapture>,
    /// Presentation time of each frame, in `ticks_per_second` units.
    pub pts: Vec<i64>,
    /// Duration of one frame in the same unit.
    pub frame_duration: i64,
    /// Audio, one `Vec` per channel.
    pub audio: Vec<Vec<f32>>,
    pub sample_rate: u32,
}

impl Capture {
    /// SHA-256 of the whole capture (frames, times, audio samples), hex.
    pub fn sha256(&self) -> String {
        let mut h = Sha256::new();
        for f in &self.frames {
            h.update((f.width as u64).to_le_bytes());
            h.update((f.height as u64).to_le_bytes());
            for v in &f.rgba16 {
                h.update(v.to_le_bytes());
            }
        }
        for t in &self.pts {
            h.update(t.to_le_bytes());
        }
        for ch in &self.audio {
            for s in ch {
                h.update(s.to_bits().to_le_bytes());
            }
        }
        hex(&h.finalize())
    }

    /// SHA-256 of each frame's pixels, hex (the per-frame hash of the plan's golden set).
    pub fn frame_hashes(&self) -> Vec<String> {
        self.frames.iter().map(|f| hex(&Sha256::digest(f.rgba16.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()))).collect()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Limits every gating check must respect. All of them are derived by [`derive_thresholds`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub schema: String,
    /// Largest allowed |Δ| on R, G, B, in 16-bit codes.
    pub rgb_max_abs_16: u32,
    /// Smallest allowed PSNR over R, G, B, in dB.
    pub rgb_min_psnr_db: f64,
    /// Largest allowed |Δ| on alpha, in 16-bit codes.
    pub alpha_max_abs_16: u32,
    /// Smallest allowed mean SSIM over R, G, B.
    pub rgb_min_ssim: f64,
    /// Largest allowed ΔE₀₀ on the frames rounded to 8-bit sRGB.
    pub max_delta_e: f64,
    /// Largest allowed |PTS error| in ticks, by position and matched.
    pub pts_max_abs_error: i64,
    /// Most dropped (and, separately, extra) frames allowed.
    pub pts_max_dropped: usize,
    /// Highest allowed audio residual peak in any channel, in dBFS.
    pub audio_max_peak_dbfs: f64,
}

impl Thresholds {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).map(|s| s + "\n").unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Thresholds, String> {
        let t: Thresholds = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if t.schema != SCHEMA {
            return Err(format!("schema {:?}, expected {SCHEMA:?}", t.schema));
        }
        Ok(t)
    }
}

/// Outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Pass,
    Fail,
    /// Not measured; never counts as a pass.
    Skipped,
}

/// One measured figure and the limit it was held to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub status: Status,
    /// What was measured (text, so infinities and counts share one field).
    pub value: String,
    pub limit: String,
    /// `false` for figures reported for information only.
    pub gating: bool,
}

/// Overall result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Fail,
    /// No gating check failed, but at least one was not measured.
    Incomplete,
}

fn check(id: &str, ok: bool, value: impl ToString, limit: impl ToString) -> Check {
    Check { id: id.into(), status: if ok { Status::Pass } else { Status::Fail }, value: value.to_string(), limit: limit.to_string(), gating: true }
}

fn fail_metric(id: &str, e: &MetricError) -> Check {
    Check { id: id.into(), status: Status::Fail, value: format!("not measurable: {e}"), limit: String::new(), gating: true }
}

fn fmt_f(v: f64) -> String {
    if v.is_infinite() { if v > 0.0 { "inf".into() } else { "-inf".into() } } else { format!("{v:.6}") }
}

/// Measure `test` against `reference` and hold each figure to `t`. The A/V drift and the cinema
/// audio parameters are not measured (see [`compare_with`]), so the verdict is at best
/// [`Verdict::Incomplete`].
pub fn compare(reference: &Capture, test: &Capture, t: &Thresholds) -> Vec<Check> {
    compare_with(reference, test, t, &Extras::default())
}

/// Measurements that need a fixture or a declared target, so they are opt-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// The capture is a flash-and-beep render (`sync_flash_beep` fixture): measure A/V drift.
    pub sync: Option<SyncSpec>,
    /// Bit depth the audio was written at (a render's `f32` samples cannot tell); when set, the
    /// audio is held to the D-Cinema parameters of SMPTE ST 428-2 (24 bit, 48 or 96 kHz, 1-16
    /// channels). The package-level rule is `DCP.AUD.001` of `cfa4-dcp-validate`; this is the same
    /// rule applied to what a render produced, before any package exists.
    pub audio_bits: Option<u32>,
}

/// What the flash-and-beep fixture contains and how far the engine may stray from it.
///
/// The fixture shows a white flash on frame 0 of every second and sounds a 1 kHz beep at the same
/// instants (`python/cfa4_devtools/render_fixtures.py`, `sync_flash_beep`).
#[derive(Clone, Debug, PartialEq)]
pub struct SyncSpec {
    /// Unit of `Capture::pts`.
    pub ticks_per_second: i64,
    /// Flashes (and beeps) the fixture holds; any other count is a failure.
    pub expected_events: usize,
    /// Largest allowed |video − audio| offset of the first event, in milliseconds (lip-sync error).
    pub max_abs_offset_ms: f64,
    /// Largest allowed change of that offset over the clip, in milliseconds (drift proper).
    pub max_drift_ms: f64,
}

/// Mean picture level above which a frame counts as a flash, and below which it is dark again.
const FLASH_HIGH: f64 = 0.5;
const FLASH_LOW: f64 = 0.25;
/// Beep detection: a sample above this level (−20 dBFS), after this long a quiet spell.
const BEEP_LEVEL: f32 = 0.1;
const BEEP_QUIET_MS: u32 = 100;

/// Mean of R, G and B of a frame, 0..1.
fn mean_level(f: &FrameCapture) -> f64 {
    let (sum, n) = f.rgba16.chunks_exact(4).fold((0.0f64, 0u64), |(s, n), p| (s + p.iter().take(3).map(|&v| f64::from(v)).sum::<f64>(), n + 3));
    if n == 0 { 0.0 } else { sum / (n as f64 * 65_535.0) }
}

/// The A/V drift check of a flash-and-beep capture.
fn sync_check(c: &Capture, s: &SyncSpec) -> Check {
    let id = "sync.av_drift";
    let levels: Vec<f64> = c.frames.iter().map(mean_level).collect();
    let flashes = timing::rising_edges(&levels, FLASH_LOW, FLASH_HIGH);
    let beeps = match timing::beep_onsets(&c.audio, c.sample_rate, BEEP_LEVEL, BEEP_QUIET_MS) {
        Ok(b) => b,
        Err(e) => return fail_metric(id, &e),
    };
    if flashes.len() != s.expected_events || beeps.len() != s.expected_events {
        return check(id, false, format!("{} flashes, {} beeps", flashes.len(), beeps.len()), format!("{} of each", s.expected_events));
    }
    let Some(video): Option<Vec<i64>> = flashes.iter().map(|&i| c.pts.get(i).copied()).collect() else {
        return check(id, false, "a flash frame has no presentation time", "one time per frame");
    };
    match timing::av_drift(&video, &beeps, s.ticks_per_second, c.sample_rate) {
        Ok(d) => check(
            id,
            d.initial_offset_ms.abs() <= s.max_abs_offset_ms && d.max_drift_ms <= s.max_drift_ms,
            format!("offset {} ms, drift {} ms", fmt_f(d.initial_offset_ms), fmt_f(d.max_drift_ms)),
            format!("offset ≤ {} ms, drift ≤ {} ms", fmt_f(s.max_abs_offset_ms), fmt_f(s.max_drift_ms)),
        ),
        Err(e) => fail_metric(id, &e),
    }
}

/// Sample rates of SMPTE ST 428-2 sound.
pub const ST_428_2_SAMPLE_RATES: [u32; 2] = [48_000, 96_000];
/// Most channels of SMPTE ST 428-2 sound.
pub const ST_428_2_MAX_CHANNELS: usize = 16;

/// The D-Cinema audio parameters of a capture (ST 428-2): 24 bit, 48 or 96 kHz, 1-16 channels of
/// one length, no sample beyond full scale. The usual channel counts (6, 8, 16: ST 429-2) are
/// reported for information only, as `cfa4-dcp-validate` does (a warning there).
fn audio_checks(c: &Capture, bits: u32) -> Vec<Check> {
    let mut out = Vec::new();
    let channels = c.audio.len();
    let equal = c.audio.windows(2).all(|w| w[0].len() == w[1].len());
    let peak = c.audio.iter().flatten().fold(0.0f32, |m, s| m.max(s.abs()));
    out.push(check("audio.st428_2.bits", bits == 24, bits, 24));
    out.push(check("audio.st428_2.sample_rate", ST_428_2_SAMPLE_RATES.contains(&c.sample_rate), c.sample_rate, "48000 or 96000"));
    out.push(check("audio.st428_2.channels", (1..=ST_428_2_MAX_CHANNELS).contains(&channels), channels, "1 to 16"));
    out.push(check("audio.st428_2.channel_length", equal, if equal { "equal" } else { "differs" }, "equal"));
    out.push(check("audio.st428_2.full_scale", peak.is_finite() && peak <= 1.0, format!("{peak:.6}"), "≤ 1.0"));
    out.push(Check {
        id: "audio.st429_2.channel_count".into(),
        status: if matches!(channels, 6 | 8 | 16) { Status::Pass } else { Status::Fail },
        value: channels.to_string(),
        limit: "6, 8 or 16 (informational)".into(),
        gating: false,
    });
    out
}

/// [`compare`] plus the opt-in measurements of `extras`. A requested measurement that cannot be
/// made is a failure; one that was not requested stays [`Status::Skipped`] (never a pass).
pub fn compare_with(reference: &Capture, test: &Capture, t: &Thresholds, extras: &Extras) -> Vec<Check> {
    let mut out = Vec::new();

    // frame count and size first: nothing else is comparable without them
    let same_count = reference.frames.len() == test.frames.len();
    out.push(check("video.frame_count", same_count, test.frames.len(), reference.frames.len()));
    let same_size = same_count && reference.frames.iter().zip(&test.frames).all(|(a, b)| (a.width, a.height) == (b.width, b.height));
    if same_count {
        out.push(check("video.frame_size", same_size, if same_size { "equal" } else { "differs" }, "equal"));
    }

    // timing
    match timing::pts_diff(&reference.pts, &test.pts, reference.frame_duration) {
        Ok(d) => {
            out.push(check("timing.dropped", d.dropped <= t.pts_max_dropped, d.dropped, t.pts_max_dropped));
            out.push(check("timing.extra", d.extra <= t.pts_max_dropped, d.extra, t.pts_max_dropped));
            out.push(check("timing.non_monotonic", d.non_monotonic == 0, d.non_monotonic, 0));
            let worst = d.max_abs_error.max(d.index_max_abs_error);
            out.push(check("timing.pts_error", worst <= t.pts_max_abs_error, worst, t.pts_max_abs_error));
        }
        Err(e) => out.push(fail_metric("timing.pts_error", &e)),
    }

    // pixels, frame by frame, worst figure kept
    if same_size {
        let (mut rgb_max, mut rgb_psnr, mut a_max, mut ssim_min, mut de_max) = (0.0f64, f64::INFINITY, 0.0f64, 1.0f64, 0.0f64);
        let mut error: Option<(&str, MetricError)> = None;
        for (a, b) in reference.frames.iter().zip(&test.frames) {
            for c in 0..3 {
                match diff_u16(&a.plane(c), &b.plane(c), 16) {
                    Ok(d) => {
                        rgb_max = rgb_max.max(d.max_abs);
                        rgb_psnr = rgb_psnr.min(d.psnr);
                    }
                    Err(e) => error = Some(("video.rgb", e)),
                }
                match ssim_u16(&a.plane(c), &b.plane(c), a.width, a.height, 16) {
                    Ok(s) => ssim_min = ssim_min.min(s),
                    Err(e) => error = Some(("video.ssim", e)),
                }
            }
            match diff_u16(&a.plane(3), &b.plane(3), 16) {
                Ok(d) => a_max = a_max.max(d.max_abs),
                Err(e) => error = Some(("video.alpha", e)),
            }
            match delta_e_srgb8(&a.to_rgba8(), &b.to_rgba8(), a.width, a.height, 4) {
                Ok(s) => de_max = de_max.max(s.max),
                Err(e) => error = Some(("video.delta_e", e)),
            }
        }
        if let Some((id, e)) = error {
            out.push(fail_metric(id, &e));
        } else {
            out.push(check("video.rgb.max_abs", rgb_max <= f64::from(t.rgb_max_abs_16), rgb_max, t.rgb_max_abs_16));
            out.push(check("video.rgb.psnr", rgb_psnr >= t.rgb_min_psnr_db, fmt_f(rgb_psnr), fmt_f(t.rgb_min_psnr_db)));
            out.push(check("video.alpha.max_abs", a_max <= f64::from(t.alpha_max_abs_16), a_max, t.alpha_max_abs_16));
            out.push(check("video.rgb.ssim", ssim_min >= t.rgb_min_ssim, fmt_f(ssim_min), fmt_f(t.rgb_min_ssim)));
            out.push(check("video.delta_e", de_max <= t.max_delta_e, fmt_f(de_max), fmt_f(t.max_delta_e)));
        }
    } else {
        for id in ["video.rgb.max_abs", "video.rgb.psnr", "video.alpha.max_abs", "video.rgb.ssim", "video.delta_e"] {
            out.push(Check { id: id.into(), status: Status::Skipped, value: "frames not comparable".into(), limit: String::new(), gating: true });
        }
    }

    // information only: bit-identical frames
    let identical = reference.frame_hashes() == test.frame_hashes();
    out.push(Check {
        id: "video.bit_identical".into(),
        status: if identical { Status::Pass } else { Status::Fail },
        value: identical.to_string(),
        limit: "informational".into(),
        gating: false,
    });

    // audio
    match audio::null_test(&reference.audio, &test.audio) {
        Ok(n) => out.push(check("audio.null_peak_dbfs", n.worst_peak_dbfs <= t.audio_max_peak_dbfs, fmt_f(n.worst_peak_dbfs), fmt_f(t.audio_max_peak_dbfs))),
        Err(e) => out.push(fail_metric("audio.null_peak_dbfs", &e)),
    }

    // A/V drift: measured on a flash-and-beep render, otherwise reported as skipped, never as passed
    match &extras.sync {
        Some(spec) => out.push(sync_check(test, spec)),
        None => out.push(Check {
            id: "sync.av_drift".into(),
            status: Status::Skipped,
            value: "needs a flash-and-beep fixture rendered through the engine (F0V fixtures)".into(),
            limit: String::new(),
            gating: true,
        }),
    }
    if let Some(bits) = extras.audio_bits {
        out.extend(audio_checks(test, bits));
    }
    out
}

/// Verdict of a list of checks: any gating failure fails; else any skipped gating check is
/// incomplete; else pass.
pub fn verdict(checks: &[Check]) -> Verdict {
    let gating = || checks.iter().filter(|c| c.gating);
    if gating().any(|c| c.status == Status::Fail) {
        Verdict::Fail
    } else if gating().any(|c| c.status == Status::Skipped) {
        Verdict::Incomplete
    } else {
        Verdict::Pass
    }
}

/// Like [`verdict`] but ignoring checks the harness version cannot measure yet (`Skipped`), so a
/// run can be judged on what it did measure. Used by the self-test and the noise runs.
pub fn measured_failure(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.gating && c.status == Status::Fail)
}

/// Worst deviation of several runs of the same build from the first run.
#[derive(Clone, Debug, PartialEq)]
pub struct Noise {
    pub runs: usize,
    pub rgb_max_abs_16: u32,
    pub rgb_min_psnr_db: f64,
    pub alpha_max_abs_16: u32,
    pub rgb_min_ssim: f64,
    pub max_delta_e: f64,
    pub pts_max_abs_error: i64,
    pub audio_peak_dbfs: f64,
    /// Every run reproduced the first run's [`Capture::sha256`].
    pub bit_identical: bool,
}

/// Why noise could not be measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoiseError {
    TooFewRuns { got: usize },
    Metric(String),
}

impl std::fmt::Display for NoiseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoiseError::TooFewRuns { got } => write!(f, "{got} runs: at least {MIN_NOISE_RUNS} are needed"),
            NoiseError::Metric(m) => write!(f, "noise not measurable: {m}"),
        }
    }
}

/// Measure the run-to-run noise of `runs` (the same build, same inputs) against `runs[0]`.
pub fn measure_noise(runs: &[Capture]) -> Result<Noise, NoiseError> {
    let (first, rest) = match runs.split_first() {
        Some(x) if runs.len() >= MIN_NOISE_RUNS => x,
        _ => return Err(NoiseError::TooFewRuns { got: runs.len() }),
    };
    let mut n = Noise {
        runs: runs.len(),
        rgb_max_abs_16: 0,
        rgb_min_psnr_db: f64::INFINITY,
        alpha_max_abs_16: 0,
        rgb_min_ssim: 1.0,
        max_delta_e: 0.0,
        pts_max_abs_error: 0,
        audio_peak_dbfs: f64::NEG_INFINITY,
        bit_identical: true,
    };
    let reference_hash = first.sha256();
    for run in rest {
        n.bit_identical &= run.sha256() == reference_hash;
        if run.frames.len() != first.frames.len() {
            return Err(NoiseError::Metric("frame count differs between runs".into()));
        }
        for (a, b) in first.frames.iter().zip(&run.frames) {
            for c in 0..3 {
                let d = diff_u16(&a.plane(c), &b.plane(c), 16).map_err(|e| NoiseError::Metric(e.to_string()))?;
                n.rgb_max_abs_16 = n.rgb_max_abs_16.max(d.max_abs as u32);
                n.rgb_min_psnr_db = n.rgb_min_psnr_db.min(d.psnr);
                let s = ssim_u16(&a.plane(c), &b.plane(c), a.width, a.height, 16).map_err(|e| NoiseError::Metric(e.to_string()))?;
                n.rgb_min_ssim = n.rgb_min_ssim.min(s);
            }
            let d = diff_u16(&a.plane(3), &b.plane(3), 16).map_err(|e| NoiseError::Metric(e.to_string()))?;
            n.alpha_max_abs_16 = n.alpha_max_abs_16.max(d.max_abs as u32);
            let e = delta_e_srgb8(&a.to_rgba8(), &b.to_rgba8(), a.width, a.height, 4).map_err(|e| NoiseError::Metric(e.to_string()))?;
            n.max_delta_e = n.max_delta_e.max(e.max);
        }
        let p = timing::pts_diff(&first.pts, &run.pts, first.frame_duration).map_err(|e| NoiseError::Metric(e.to_string()))?;
        n.pts_max_abs_error = n.pts_max_abs_error.max(p.max_abs_error).max(p.index_max_abs_error);
        let a = audio::null_test(&first.audio, &run.audio).map_err(|e| NoiseError::Metric(e.to_string()))?;
        n.audio_peak_dbfs = n.audio_peak_dbfs.max(a.worst_peak_dbfs);
    }
    Ok(n)
}

/// Safety factor of the threshold rule: a limit is four times the measured noise amplitude
/// (+12 dB for logarithmic figures).
pub const NOISE_FACTOR: f64 = 4.0;

/// Turn measured noise into limits (plan: "Küszöb@F0 = a baseline mérési zajából származtatott,
/// dokumentált érték"). The rule, per figure:
///
/// * deviation figures (16-bit codes, PTS ticks, ΔE): `ceil(4 × worst noise)`; zero noise gives
///   zero, i.e. bit-exact output is required;
/// * PSNR: 12 dB below the worst noise PSNR, or [`PSNR_IDENTICAL_FLOOR_DB`] when the noise is zero;
/// * SSIM: `1 − 4 × (1 − worst noise SSIM)`, never above 0.9999;
/// * audio residual peak: 12 dB above the worst noise peak, or [`AUDIO_SILENCE_DBFS`] when silent.
///
/// Dropped and extra frames are always limited to zero.
pub fn derive_thresholds(noise: &Noise) -> Thresholds {
    let up = |v: f64| (v * NOISE_FACTOR).ceil();
    Thresholds {
        schema: SCHEMA.into(),
        rgb_max_abs_16: up(f64::from(noise.rgb_max_abs_16)) as u32,
        rgb_min_psnr_db: if noise.rgb_min_psnr_db.is_infinite() { PSNR_IDENTICAL_FLOOR_DB } else { noise.rgb_min_psnr_db - 12.0 },
        alpha_max_abs_16: up(f64::from(noise.alpha_max_abs_16)) as u32,
        rgb_min_ssim: (1.0 - NOISE_FACTOR * (1.0 - noise.rgb_min_ssim)).min(0.9999),
        max_delta_e: up(noise.max_delta_e),
        pts_max_abs_error: up(noise.pts_max_abs_error as f64) as i64,
        pts_max_dropped: 0,
        audio_max_peak_dbfs: if noise.audio_peak_dbfs.is_infinite() { AUDIO_SILENCE_DBFS } else { noise.audio_peak_dbfs + 12.0 },
    }
}

/// Reproducibility metadata recorded with every report (plan rule A10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub harness: String,
    pub head: String,
    pub rustc: String,
    pub os: String,
    pub kernel: String,
    pub cpu: String,
    /// `"none (CPU path)"` for every run of this harness version.
    pub gpu: String,
    pub reference_sha256: String,
    pub thresholds_sha256: String,
}

fn command_line(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

impl Metadata {
    /// Collect the host facts (no root, no network) and hash the inputs.
    pub fn collect(reference: &Capture, thresholds: &Thresholds) -> Metadata {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let os = os_release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string()).unwrap_or_else(|| "unknown".into());
        let cpu = std::fs::read_to_string("/proc/cpuinfo")
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()))
            .unwrap_or_else(|| "unknown".into());
        Metadata {
            harness: format!("cfa4-golden {}", env!("CARGO_PKG_VERSION")),
            head: command_line("git", &["rev-parse", "HEAD"]),
            rustc: command_line("rustc", &["--version"]),
            os,
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_else(|_| "unknown".into()),
            cpu,
            gpu: "none (CPU path)".into(),
            reference_sha256: reference.sha256(),
            thresholds_sha256: hex(&Sha256::digest(thresholds.to_json().as_bytes())),
        }
    }
}

/// A validation run: what was compared, how, and what came out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub label: String,
    pub metadata: Metadata,
    pub verdict: Verdict,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn new(label: &str, metadata: Metadata, checks: Vec<Check>) -> Report {
        Report { schema: SCHEMA.into(), label: label.into(), metadata, verdict: verdict(&checks), checks }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).map(|s| s + "\n").unwrap_or_default()
    }

    pub fn to_markdown(&self) -> String {
        let m = &self.metadata;
        let mut s = format!("# Render validation: {}\n\n**Verdict: {:?}**\n\n", self.label, self.verdict);
        let _ = writeln!(s, "| Metadata | |\n|---|---|");
        for (k, v) in [
            ("harness", &m.harness),
            ("HEAD", &m.head),
            ("rustc", &m.rustc),
            ("OS", &m.os),
            ("kernel", &m.kernel),
            ("CPU", &m.cpu),
            ("GPU", &m.gpu),
            ("reference sha256", &m.reference_sha256),
            ("thresholds sha256", &m.thresholds_sha256),
        ] {
            let _ = writeln!(s, "| {k} | `{v}` |");
        }
        let _ = writeln!(s, "\n| Check | Status | Value | Limit | Gating |\n|---|---|---|---|---|");
        for c in &self.checks {
            let _ = writeln!(s, "| `{}` | {:?} | {} | {} | {} |", c.id, c.status, c.value, c.limit, if c.gating { "yes" } else { "no" });
        }
        s
    }
}

/// The mutation self-test: known damage that the harness must catch.
pub mod mutation {
    use super::{Capture, Thresholds, compare, measured_failure};

    /// A way of damaging a capture.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mutation {
        /// One frame in the middle is missing (its time too).
        DropFrame,
        /// Frames encoded with the BT.709 matrix and decoded with BT.601.
        MatrixSwap601709,
        /// Every presentation time one frame late.
        PtsShiftOneFrame,
        /// Alpha forced opaque.
        AlphaLoss,
        /// Samples rounded to the nearest 8-bit level.
        RoundTo8Bit,
        /// The last audio channel silenced.
        MuteChannel,
    }

    impl Mutation {
        pub const ALL: [Mutation; 6] =
            [Mutation::DropFrame, Mutation::MatrixSwap601709, Mutation::PtsShiftOneFrame, Mutation::AlphaLoss, Mutation::RoundTo8Bit, Mutation::MuteChannel];

        pub fn name(self) -> &'static str {
            match self {
                Mutation::DropFrame => "drop_frame",
                Mutation::MatrixSwap601709 => "matrix_swap_601_709",
                Mutation::PtsShiftOneFrame => "pts_shift_one_frame",
                Mutation::AlphaLoss => "alpha_loss",
                Mutation::RoundTo8Bit => "round_to_8_bit",
                Mutation::MuteChannel => "mute_channel",
            }
        }
    }

    fn map_pixels(c: &Capture, f: impl Fn([u16; 4]) -> [u16; 4]) -> Capture {
        let mut out = c.clone();
        for fr in &mut out.frames {
            for p in fr.rgba16.chunks_exact_mut(4) {
                if let Some(&[r, g, b, a]) = p.first_chunk::<4>() {
                    p.copy_from_slice(&f([r, g, b, a]));
                }
            }
        }
        out
    }

    fn to_u16(v: f64) -> u16 {
        v.round().clamp(0.0, 65_535.0) as u16
    }

    /// Apply `m` to a copy of `c`.
    pub fn apply(c: &Capture, m: Mutation) -> Capture {
        match m {
            Mutation::DropFrame => {
                let mut out = c.clone();
                let i = out.frames.len() / 2;
                if i < out.frames.len() && i < out.pts.len() {
                    out.frames.remove(i);
                    out.pts.remove(i);
                }
                out
            }
            Mutation::PtsShiftOneFrame => {
                let mut out = c.clone();
                for t in &mut out.pts {
                    *t += c.frame_duration;
                }
                out
            }
            Mutation::MatrixSwap601709 => map_pixels(c, |[r, g, b, a]| {
                let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
                // encode with BT.709 …
                let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let (cb, cr) = ((b - y) / 1.8556, (r - y) / 1.5748);
                // … decode with BT.601
                let (r2, b2) = (y + 1.402 * cr, y + 1.772 * cb);
                let g2 = (y - 0.299 * r2 - 0.114 * b2) / 0.587;
                [to_u16(r2), to_u16(g2), to_u16(b2), a]
            }),
            Mutation::AlphaLoss => map_pixels(c, |[r, g, b, _]| [r, g, b, u16::MAX]),
            Mutation::RoundTo8Bit => {
                let q = |v: u16| ((u32::from(v) * 255 + 32_767) / 65_535 * 257) as u16;
                map_pixels(c, |[r, g, b, a]| [q(r), q(g), q(b), q(a)])
            }
            Mutation::MuteChannel => {
                let mut out = c.clone();
                if let Some(ch) = out.audio.last_mut() {
                    ch.fill(0.0);
                }
                out
            }
        }
    }

    /// Result of damaging a capture one way.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Outcome {
        pub mutation: Mutation,
        /// The damage changed the capture at all (a no-op cannot prove anything).
        pub changed: bool,
        /// A gating check reported a failure.
        pub caught: bool,
        /// Ids of the checks that failed.
        pub failed_checks: Vec<String>,
    }

    impl Outcome {
        pub fn proved(&self) -> bool {
            self.changed && self.caught
        }
    }

    /// Damage `reference` in every way and report which damage the harness caught.
    pub fn self_test(reference: &Capture, t: &Thresholds) -> Vec<Outcome> {
        Mutation::ALL
            .iter()
            .map(|&m| {
                let damaged = apply(reference, m);
                let checks = compare(reference, &damaged, t);
                Outcome {
                    mutation: m,
                    changed: damaged != *reference,
                    caught: measured_failure(&checks),
                    failed_checks: checks.iter().filter(|c| c.gating && c.status == super::Status::Fail).map(|c| c.id.clone()).collect(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mutation::{Mutation, apply, self_test};

    fn gradient(w: usize, h: usize, seed: u32) -> FrameCapture {
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let v = (x * 65_535 / w.max(1)) as u32;
                px.extend([
                    (v + seed) as u16,
                    ((y * 65_535 / h.max(1)) as u32 ^ seed) as u16,
                    ((x * y) as u32 * 7 + seed) as u16,
                    if x < w / 2 { 40_000 } else { 65_535 },
                ]);
            }
        }
        FrameCapture::new(w, h, px).unwrap()
    }

    fn capture() -> Capture {
        let fd = 1000;
        Capture {
            frames: (0..6).map(|i| gradient(32, 24, i * 13)).collect(),
            pts: (0..6).map(|i| i * fd).collect(),
            frame_duration: fd,
            audio: vec![(0..960).map(|i| (i as f32 * 0.03).sin() * 0.4).collect(), (0..960).map(|i| (i as f32 * 0.05).sin() * 0.3).collect()],
            sample_rate: 48_000,
        }
    }

    fn exact() -> Thresholds {
        let noise = Noise {
            runs: 10,
            rgb_max_abs_16: 0,
            rgb_min_psnr_db: f64::INFINITY,
            alpha_max_abs_16: 0,
            rgb_min_ssim: 1.0,
            max_delta_e: 0.0,
            pts_max_abs_error: 0,
            audio_peak_dbfs: f64::NEG_INFINITY,
            bit_identical: true,
        };
        derive_thresholds(&noise)
    }

    #[test]
    fn identical_capture_passes_what_it_measures_and_is_incomplete_overall() {
        let c = capture();
        let checks = compare(&c, &c.clone(), &exact());
        assert!(!measured_failure(&checks), "{checks:#?}");
        assert_eq!(verdict(&checks), Verdict::Incomplete, "av drift is not measured yet");
    }

    #[test]
    fn every_mutation_is_caught() {
        let c = capture();
        for o in self_test(&c, &exact()) {
            assert!(o.proved(), "{:?} not proved: {o:?}", o.mutation);
        }
    }

    #[test]
    fn the_self_test_is_not_vacuous() {
        // thresholds that allow anything must let the damage through, otherwise a pass proves nothing
        let loose = Thresholds {
            schema: SCHEMA.into(),
            rgb_max_abs_16: u32::MAX,
            rgb_min_psnr_db: 0.0,
            alpha_max_abs_16: u32::MAX,
            rgb_min_ssim: -1.0,
            max_delta_e: f64::MAX,
            pts_max_abs_error: i64::MAX,
            pts_max_dropped: usize::MAX,
            audio_max_peak_dbfs: 0.0,
        };
        let caught: Vec<_> = self_test(&capture(), &loose).into_iter().filter(|o| o.caught).map(|o| o.mutation.name()).collect();
        // a dropped frame still changes the frame count, and a shifted series is still not monotonic
        // by position; nothing else may be reported with limits that wide
        assert_eq!(caught, vec!["drop_frame"], "{caught:?}");
    }

    #[test]
    fn each_mutation_is_caught_by_the_check_that_should_see_it() {
        let c = capture();
        let by_name = |m: Mutation| self_test(&c, &exact()).into_iter().find(|o| o.mutation == m).map(|o| o.failed_checks).unwrap_or_default();
        assert!(by_name(Mutation::DropFrame).contains(&"video.frame_count".to_string()));
        assert!(by_name(Mutation::PtsShiftOneFrame).contains(&"timing.pts_error".to_string()));
        assert!(by_name(Mutation::AlphaLoss).contains(&"video.alpha.max_abs".to_string()));
        assert!(by_name(Mutation::RoundTo8Bit).contains(&"video.rgb.max_abs".to_string()));
        assert!(by_name(Mutation::MuteChannel).contains(&"audio.null_peak_dbfs".to_string()));
        assert!(by_name(Mutation::MatrixSwap601709).contains(&"video.rgb.psnr".to_string()));
    }

    #[test]
    fn the_matrix_swap_is_a_colour_error_not_a_noise_level() {
        let c = capture();
        let damaged = apply(&c, Mutation::MatrixSwap601709);
        let checks = compare(&c, &damaged, &exact());
        let failed: Vec<_> = checks.iter().filter(|k| k.status == Status::Fail).map(|k| k.id.as_str()).collect();
        assert!(failed.contains(&"video.rgb.psnr"), "{failed:?}");
    }

    #[test]
    fn a_missing_measurement_never_passes() {
        let checks = vec![Check { id: "x".into(), status: Status::Skipped, value: String::new(), limit: String::new(), gating: true }];
        assert_eq!(verdict(&checks), Verdict::Incomplete);
        assert_eq!(verdict(&[]), Verdict::Pass);
    }

    #[test]
    fn noise_needs_ten_runs() {
        let c = capture();
        assert_eq!(measure_noise(&vec![c.clone(); 9]), Err(NoiseError::TooFewRuns { got: 9 }));
        let n = measure_noise(&vec![c; 10]).unwrap();
        assert!(n.bit_identical);
        assert_eq!((n.rgb_max_abs_16, n.alpha_max_abs_16, n.pts_max_abs_error), (0, 0, 0));
    }

    #[test]
    fn thresholds_follow_the_documented_rule() {
        let noise = Noise {
            runs: 10,
            rgb_max_abs_16: 2,
            rgb_min_psnr_db: 100.0,
            alpha_max_abs_16: 1,
            rgb_min_ssim: 0.99995,
            max_delta_e: 0.1,
            pts_max_abs_error: 3,
            audio_peak_dbfs: -100.0,
            bit_identical: false,
        };
        let t = derive_thresholds(&noise);
        assert_eq!((t.rgb_max_abs_16, t.alpha_max_abs_16, t.pts_max_abs_error), (8, 4, 12));
        assert_eq!(t.rgb_min_psnr_db, 88.0);
        assert_eq!(t.max_delta_e, 1.0);
        assert!((t.rgb_min_ssim - 0.9998).abs() < 1e-9);
        assert_eq!(t.audio_max_peak_dbfs, -88.0);
        assert_eq!(derive_thresholds(&Noise { rgb_min_psnr_db: f64::INFINITY, ..noise }).rgb_min_psnr_db, PSNR_IDENTICAL_FLOOR_DB);
    }

    #[test]
    fn thresholds_round_trip_through_json() {
        let t = exact();
        assert_eq!(Thresholds::from_json(&t.to_json()).unwrap(), t);
        assert!(Thresholds::from_json("{\"schema\":\"other\"}").is_err());
    }

    #[test]
    fn report_has_json_and_markdown() {
        let c = capture();
        let t = exact();
        let r = Report::new("unit", Metadata::collect(&c, &t), compare(&c, &c.clone(), &t));
        assert!(r.to_json().contains("\"verdict\""));
        let md = r.to_markdown();
        assert!(md.contains("`video.rgb.max_abs`") && md.contains("reference sha256"));
    }

    #[test]
    fn frame_capture_checks_its_size() {
        assert!(FrameCapture::new(2, 2, vec![0; 15]).is_none());
        assert!(FrameCapture::new(2, 2, vec![0; 16]).is_some());
    }

    // ---- A/V drift and cinema audio parameters --------------------------------------------

    const FPS: i64 = 25;
    const TICKS: i64 = 25_000; // one tick per millisecond/25: 1000 ticks per frame
    const RATE: u32 = 48_000;

    /// 4 seconds, a flash on frame 0 of each second, a beep `audio_delay` samples after each flash
    /// plus `creep` samples more per second.
    fn flash_beep(audio_delay: usize, creep: usize) -> Capture {
        let frames: Vec<FrameCapture> = (0..4 * FPS)
            .map(|n| {
                let v: u16 = if n % FPS == 0 { 60_000 } else { 2_000 };
                FrameCapture::new(32, 32, [v, v, v, 65_535].repeat(1024)).unwrap()
            })
            .collect();
        let mut a = vec![0.0f32; 4 * RATE as usize];
        for sec in 0..4usize {
            let start = sec * RATE as usize + audio_delay + creep * sec;
            for k in 0..480 {
                if let Some(s) = a.get_mut(start + k) {
                    *s = (2.0 * std::f32::consts::PI * 1000.0 * k as f32 / RATE as f32).sin() * 0.8;
                }
            }
        }
        Capture { frames, pts: (0..4 * FPS).map(|i| i * (TICKS / FPS)).collect(), frame_duration: TICKS / FPS, audio: vec![a.clone(), a], sample_rate: RATE }
    }

    fn sync_spec() -> SyncSpec {
        SyncSpec { ticks_per_second: TICKS, expected_events: 4, max_abs_offset_ms: 20.0, max_drift_ms: 1.0 }
    }

    fn sync_status(c: &Capture) -> Check {
        let x = Extras { sync: Some(sync_spec()), audio_bits: None };
        compare_with(c, c, &exact(), &x).into_iter().find(|k| k.id == "sync.av_drift").unwrap()
    }

    #[test]
    fn a_synchronised_flash_and_beep_passes_and_completes_the_verdict() {
        let c = flash_beep(0, 0);
        let x = Extras { sync: Some(sync_spec()), audio_bits: Some(24) };
        let checks = compare_with(&c, &c, &exact(), &x);
        assert!(!measured_failure(&checks), "{checks:#?}");
        assert_eq!(verdict(&checks), Verdict::Pass, "{checks:#?}");
    }

    #[test]
    fn a_constant_lip_sync_error_is_caught_by_the_offset() {
        // audio 3 frames (120 ms) late: no drift at all, but far beyond 20 ms
        let k = sync_status(&flash_beep(3 * (RATE as usize / FPS as usize), 0));
        assert_eq!(k.status, Status::Fail, "{k:?}");
    }

    #[test]
    fn a_creeping_drift_is_caught_by_the_drift_limit() {
        // 48 samples (1 ms) more every second: offset stays small, drift reaches 3 ms
        let k = sync_status(&flash_beep(0, 48));
        assert_eq!(k.status, Status::Fail, "{k:?}");
        assert!(k.value.contains("drift"), "{k:?}");
    }

    #[test]
    fn missing_or_extra_events_fail_instead_of_passing_or_skipping() {
        let mut silent = flash_beep(0, 0);
        for ch in &mut silent.audio {
            ch.fill(0.0);
        }
        assert_eq!(sync_status(&silent).status, Status::Fail);
        let mut few = flash_beep(0, 0);
        few.frames.truncate(2 * FPS as usize);
        few.pts.truncate(2 * FPS as usize);
        assert_eq!(sync_status(&few).status, Status::Fail);
    }

    #[test]
    fn without_a_sync_fixture_the_drift_stays_skipped() {
        let c = flash_beep(0, 0);
        let k = compare(&c, &c, &exact()).into_iter().find(|k| k.id == "sync.av_drift").unwrap();
        assert_eq!(k.status, Status::Skipped);
    }

    #[test]
    fn st_428_2_audio_parameters() {
        let ok = |c: &Capture, bits| {
            let x = Extras { sync: None, audio_bits: Some(bits) };
            compare_with(c, c, &exact(), &x).into_iter().filter(|k| k.id.starts_with("audio.st428_2")).all(|k| k.status == Status::Pass)
        };
        let c = capture();
        assert!(ok(&c, 24));
        assert!(!ok(&c, 16), "16 bit is not D-Cinema sound");
        let mut r44 = capture();
        r44.sample_rate = 44_100;
        assert!(!ok(&r44, 24));
        let mut too_many = capture();
        too_many.audio = vec![vec![0.0; 10]; 17];
        assert!(!ok(&too_many, 24));
        let mut hot = capture();
        hot.audio[0][3] = 1.5;
        assert!(!ok(&hot, 24), "a sample beyond full scale clips on the way to 24 bit");
        let mut ragged = capture();
        ragged.audio[1].pop();
        assert!(!ok(&ragged, 24));
        // 2 channels: allowed by ST 428-2, only informational for ST 429-2
        let x = Extras { sync: None, audio_bits: Some(24) };
        let info = compare_with(&c, &c, &exact(), &x).into_iter().find(|k| k.id == "audio.st429_2.channel_count").unwrap();
        assert!(!info.gating);
    }
}
