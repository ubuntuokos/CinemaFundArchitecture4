//! Resource and responsiveness metrics of the render validation harness
//! (CFA4-RENDER-INTEGRATION-PLAN-001, F0V; AC-07 memory growth, AC-09 main-thread pauses).
//!
//! Root-free: process memory comes from `/proc/<pid>/status`, never from a privileged tool. The
//! parsers take text or a path, so tests run on synthetic input; a missing or malformed file gives
//! `None`, never zero in place of "unknown" and never a panic. The analysis functions work on
//! integer samples (milliseconds, KiB), so no floating-point seconds enter a comparison.
//!
//! The system-wide readers (`/proc/meminfo`, pressure, cgroup limits) already exist in
//! `cfa4-tasks`; this module adds the per-process figures they do not cover.

use std::fs;
use std::io::Read;
use std::path::Path;

/// At most this many bytes are read from a status file (a real one is about 1.5 KiB).
const MAX_STATUS_BYTES: u64 = 64 * 1024;

/// Memory figures of one process, in KiB, from `/proc/<pid>/status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessMemory {
    /// `VmRSS`: resident set size now.
    pub rss_kib: u64,
    /// `VmHWM`: peak resident set size so far.
    pub peak_rss_kib: u64,
}

/// Parse the text of a `/proc/<pid>/status` file. `None` if `VmRSS` or `VmHWM` is missing or
/// not a number followed by `kB`.
pub fn parse_status(text: &str) -> Option<ProcessMemory> {
    fn field(text: &str, key: &str) -> Option<u64> {
        let line = text.lines().find_map(|l| l.strip_prefix(key))?;
        let mut parts = line.trim_start_matches(':').split_whitespace();
        let value = parts.next()?.parse::<u64>().ok()?;
        (parts.next()? == "kB").then_some(value)
    }
    Some(ProcessMemory { rss_kib: field(text, "VmRSS")?, peak_rss_kib: field(text, "VmHWM")? })
}

/// Read and parse a status file (bounded read).
pub fn read_status(path: &Path) -> Option<ProcessMemory> {
    let mut text = String::new();
    fs::File::open(path).ok()?.take(MAX_STATUS_BYTES).read_to_string(&mut text).ok()?;
    parse_status(&text)
}

/// Memory of the calling process (`/proc/self/status`); `None` off Linux or without `/proc`.
pub fn current_process_memory() -> Option<ProcessMemory> {
    read_status(Path::new("/proc/self/status"))
}

/// One timed memory reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemSample {
    /// Milliseconds since the start of the run.
    pub t_ms: u64,
    /// Resident set size, KiB.
    pub rss_kib: u64,
}

/// Memory growth after a warm-up period (AC-07: growth after the first ten minutes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Growth {
    /// Samples at or after the warm-up end.
    pub samples: usize,
    /// Last minus first resident size after the warm-up, KiB (negative when memory fell).
    pub delta_kib: i64,
    /// Least-squares slope, KiB per minute, rounded to the nearest integer.
    pub slope_kib_per_min: i64,
    /// Largest resident size seen in the whole run, KiB.
    pub peak_kib: u64,
}

/// Growth of resident memory after `warmup_ms`. `None` if fewer than two samples fall after the
/// warm-up, the times are not strictly increasing, or they span no time: a run too short to judge
/// is "not measured", never a pass.
pub fn growth_after(samples: &[MemSample], warmup_ms: u64) -> Option<Growth> {
    if samples.windows(2).any(|w| w[1].t_ms <= w[0].t_ms) {
        return None;
    }
    let late: Vec<&MemSample> = samples.iter().filter(|s| s.t_ms >= warmup_ms).collect();
    let (first, last) = (late.first()?, late.last()?);
    if late.len() < 2 || last.t_ms == first.t_ms {
        return None;
    }
    // Least squares on (t in minutes, rss) in i128 integer arithmetic over milliseconds.
    let n = late.len() as i128;
    let t0 = i128::from(first.t_ms);
    let (mut st, mut sy, mut stt, mut sty) = (0i128, 0i128, 0i128, 0i128);
    for s in &late {
        let t = i128::from(s.t_ms) - t0;
        let y = i128::from(s.rss_kib);
        st += t;
        sy += y;
        stt += t * t;
        sty += t * y;
    }
    let denom = n * stt - st * st;
    if denom == 0 {
        return None;
    }
    // slope in KiB per millisecond = (n·Σty − Σt·Σy) / denom; scale to a minute, rounding.
    let numer = (n * sty - st * sy) * 60_000;
    let slope = if numer >= 0 { (numer + denom / 2) / denom } else { (numer - denom / 2) / denom };
    Some(Growth {
        samples: late.len(),
        delta_kib: i64::try_from(i128::from(last.rss_kib) - i128::from(first.rss_kib)).ok()?,
        slope_kib_per_min: i64::try_from(slope).ok()?,
        peak_kib: samples.iter().map(|s| s.rss_kib).max()?,
    })
}

/// Longest pause between consecutive heartbeats of a thread's event loop, in milliseconds
/// (AC-09: no pause longer than the threshold while rendering, previewing or cancelling).
///
/// `heartbeats_ms` are the times at which the loop ran its probe; they must increase strictly. A
/// run is also judged on the gap from the last heartbeat to `end_ms`, because a frozen loop stops
/// producing heartbeats. `None` for an empty series, non-increasing times, or `end_ms` before the
/// last heartbeat.
pub fn max_pause_ms(heartbeats_ms: &[u64], end_ms: u64) -> Option<u64> {
    let last = *heartbeats_ms.last()?;
    if end_ms < last || heartbeats_ms.windows(2).any(|w| w[1] <= w[0]) {
        return None;
    }
    let inner = heartbeats_ms.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
    Some(inner.max(end_ms - last))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tcfa4\nVmPeak:\t  900000 kB\nVmHWM:\t  123456 kB\nVmRSS:\t  100000 kB\nThreads:\t4\n";

    fn s(t_ms: u64, rss_kib: u64) -> MemSample {
        MemSample { t_ms, rss_kib }
    }

    #[test]
    fn parses_a_real_status_layout() {
        assert_eq!(parse_status(STATUS), Some(ProcessMemory { rss_kib: 100_000, peak_rss_kib: 123_456 }));
    }

    #[test]
    fn missing_or_malformed_fields_are_unknown_not_zero() {
        assert_eq!(parse_status("Name:\tx\n"), None);
        assert_eq!(parse_status("VmRSS:\t12 kB\n"), None);
        assert_eq!(parse_status("VmRSS:\tabc kB\nVmHWM:\t1 kB\n"), None);
        assert_eq!(parse_status("VmRSS:\t12 MB\nVmHWM:\t1 kB\n"), None);
        assert_eq!(read_status(Path::new("/nonexistent/status")), None);
    }

    #[test]
    fn the_calling_process_has_memory_on_linux() {
        if cfg!(target_os = "linux") && Path::new("/proc/self/status").exists() {
            let m = current_process_memory().expect("own status is readable");
            assert!(m.rss_kib > 0 && m.peak_rss_kib >= m.rss_kib);
        }
    }

    #[test]
    fn flat_memory_after_warmup_has_zero_growth() {
        let v: Vec<MemSample> = (0..20).map(|i| s(i * 60_000, 500_000)).collect();
        let g = growth_after(&v, 10 * 60_000).unwrap();
        assert_eq!((g.delta_kib, g.slope_kib_per_min, g.samples), (0, 0, 10));
    }

    #[test]
    fn a_leak_shows_as_slope_and_the_warmup_ramp_is_ignored() {
        // fast ramp during warm-up, then 1000 KiB/min of leak
        let mut v: Vec<MemSample> = (0..10).map(|i| s(i * 60_000, 100_000 + i * 50_000)).collect();
        v.extend((10..20).map(|i| s(i * 60_000, 550_000 + (i - 10) * 1000)));
        let g = growth_after(&v, 10 * 60_000).unwrap();
        assert_eq!(g.slope_kib_per_min, 1000);
        assert_eq!(g.delta_kib, 9000);
        assert_eq!(g.peak_kib, 559_000);
    }

    #[test]
    fn falling_memory_gives_a_negative_slope() {
        let v = [s(0, 2000), s(60_000, 1000), s(120_000, 0)];
        assert_eq!(growth_after(&v, 0).unwrap().slope_kib_per_min, -1000);
    }

    #[test]
    fn a_run_too_short_to_judge_is_not_measured() {
        assert_eq!(growth_after(&[], 0), None);
        assert_eq!(growth_after(&[s(0, 1)], 0), None);
        assert_eq!(growth_after(&[s(0, 1), s(10, 2)], 100), None);
        assert_eq!(growth_after(&[s(5, 1), s(5, 2)], 0), None);
        assert_eq!(growth_after(&[s(0, 1), s(10, 2), s(9, 3)], 0), None);
    }

    #[test]
    fn the_longest_pause_includes_the_tail() {
        assert_eq!(max_pause_ms(&[0, 16, 32, 48], 50), Some(16));
        assert_eq!(max_pause_ms(&[0, 16, 400, 416], 420), Some(384));
        // the loop froze after its last heartbeat
        assert_eq!(max_pause_ms(&[0, 16, 32], 2032), Some(2000));
    }

    #[test]
    fn bad_heartbeat_series_are_not_measured() {
        assert_eq!(max_pause_ms(&[], 10), None);
        assert_eq!(max_pause_ms(&[5, 5], 10), None);
        assert_eq!(max_pause_ms(&[0, 10], 9), None);
        assert_eq!(max_pause_ms(&[7], 7), Some(0));
    }
}
