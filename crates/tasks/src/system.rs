//! Readers for the Linux files ACS-1 needs: `/proc/meminfo`, `/proc/pressure/*`,
//! `/sys/devices/system/cpu/online` and the cgroup v2 limits of this process.
//!
//! Every function takes the filesystem root, so tests run on a synthetic tree. Every read is
//! bounded and every parse is checked: a missing or hostile file gives `None`, never a panic,
//! and never zero in place of "unknown".

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// At most this many bytes are read from one file (`/proc/meminfo` is about 1.5 KiB).
const MAX_FILE_BYTES: u64 = 16 * 1024;
/// At most this many cgroup ancestors are walked when looking for the tightest limit.
const MAX_CGROUP_DEPTH: usize = 32;

pub(crate) fn read_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// The fields of `/proc/meminfo` ACS-1 uses, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemInfo {
    pub total: Option<u64>,
    pub available: Option<u64>,
    pub swap_total: Option<u64>,
    pub swap_free: Option<u64>,
}

pub(crate) fn meminfo(root: &Path) -> MemInfo {
    let Some(text) = read_text(&root.join("proc/meminfo")) else {
        return MemInfo::default();
    };
    parse_meminfo(&text)
}

pub(crate) fn parse_meminfo(text: &str) -> MemInfo {
    let mut info = MemInfo::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        // "MemTotal:       32768000 kB": the unit is kB (KiB) for every field we read
        let mut parts = rest.split_whitespace();
        let value = parts.next().and_then(|v| v.parse::<u64>().ok());
        let bytes = match parts.next() {
            Some("kB") => value.and_then(|v| v.checked_mul(1024)),
            _ => None,
        };
        match key {
            "MemTotal" => info.total = bytes,
            "MemAvailable" => info.available = bytes,
            "SwapTotal" => info.swap_total = bytes,
            "SwapFree" => info.swap_free = bytes,
            _ => {}
        }
    }
    info
}

/// The number of online logical CPUs from `/sys/devices/system/cpu/online` (`0-15`, `0,2-7`).
pub(crate) fn online_cpus(root: &Path) -> Option<u32> {
    parse_cpu_list(&read_text(&root.join("sys/devices/system/cpu/online"))?)
}

pub(crate) fn parse_cpu_list(text: &str) -> Option<u32> {
    let mut count: u32 = 0;
    for part in text.trim().split(',') {
        let n = match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
                b.checked_sub(a)?.checked_add(1)?
            }
            None => {
                part.trim().parse::<u32>().ok()?;
                1
            }
        };
        count = count.checked_add(n)?;
    }
    (count > 0).then_some(count)
}

/// One line of a Linux PSI file, averaged over 10 s, in hundredths of a percent (basis points),
/// so the value stays an integer and comparable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pressure {
    /// Share of time at least one task stalled on the resource (`some avg10`).
    pub some_avg10_bp: u32,
    /// Share of time all non-idle tasks stalled at once (`full avg10`; none for CPU on older
    /// kernels, then zero).
    pub full_avg10_bp: u32,
}

pub(crate) fn pressure(root: &Path, resource: &str) -> Option<Pressure> {
    parse_pressure(&read_text(&root.join("proc/pressure").join(resource))?)
}

pub(crate) fn parse_pressure(text: &str) -> Option<Pressure> {
    let mut out: Option<Pressure> = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let kind = fields.next();
        let Some(avg10) = fields.find_map(|f| f.strip_prefix("avg10=")).and_then(parse_percent_bp) else {
            continue;
        };
        let p = out.get_or_insert_with(Pressure::default);
        match kind {
            Some("some") => p.some_avg10_bp = avg10,
            Some("full") => p.full_avg10_bp = avg10,
            _ => {}
        }
    }
    out
}

/// "12.34" → 1234 basis points; at most 100.00 %, anything else is rejected.
fn parse_percent_bp(text: &str) -> Option<u32> {
    let (int, frac) = text.split_once('.').unwrap_or((text, "0"));
    let int: u32 = int.parse().ok()?;
    let mut frac_digits = frac.bytes().take(2).map(|b| if b.is_ascii_digit() { Some(u32::from(b - b'0')) } else { None });
    let tens = frac_digits.next().flatten().unwrap_or(0);
    let ones = frac_digits.next().flatten().unwrap_or(0);
    let bp = int.checked_mul(100)?.checked_add(tens * 10 + ones)?;
    (bp <= 10_000).then_some(bp)
}

/// This process's cgroup v2 directory (`/sys/fs/cgroup/<path from /proc/self/cgroup>`).
pub(crate) fn own_cgroup(root: &Path) -> Option<PathBuf> {
    let text = read_text(&root.join("proc/self/cgroup"))?;
    // cgroup v2: a single line "0::/user.slice/…"; v1 lines are ignored
    let path = text.lines().find_map(|l| l.strip_prefix("0::"))?.trim();
    let rel = path.trim_start_matches('/');
    if rel.split('/').any(|c| c == "..") {
        return None;
    }
    Some(root.join("sys/fs/cgroup").join(rel))
}

/// One cgroup's `memory.max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryMax {
    /// No file: the memory controller is not enabled there (or the root cgroup), no limit.
    Absent,
    /// `max`.
    Unlimited,
    Bytes(u64),
    /// The file exists but cannot be read or parsed: a limit may exist, its size is unknown.
    Unknown,
}

pub(crate) fn memory_max(dir: &Path) -> MemoryMax {
    let path = dir.join("memory.max");
    match fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MemoryMax::Absent,
        _ => {}
    }
    let Some(text) = read_text(&path) else {
        return MemoryMax::Unknown;
    };
    match text.trim() {
        "max" => MemoryMax::Unlimited,
        t => t.parse().map_or(MemoryMax::Unknown, MemoryMax::Bytes),
    }
}

/// The tightest `memory.max` of the cgroup and its ancestors, in bytes (`None` when unlimited),
/// and whether some `memory.max` on the way could not be read (then the real limit is unknown and
/// may be tighter).
pub(crate) fn cgroup_memory_max(root: &Path, cgroup: &Path) -> (Option<u64>, bool) {
    let unknown = walk_up(root, cgroup).iter().any(|dir| memory_max(dir) == MemoryMax::Unknown);
    let tightest = tightest(root, cgroup, |dir| match memory_max(dir) {
        MemoryMax::Bytes(b) => Some(b),
        _ => None,
    });
    (tightest, unknown)
}

/// The cgroup and its ancestors up to the cgroup root (at most [`MAX_CGROUP_DEPTH`]).
fn walk_up(root: &Path, cgroup: &Path) -> Vec<PathBuf> {
    let top = root.join("sys/fs/cgroup");
    let mut out = Vec::new();
    let mut dir = cgroup.to_path_buf();
    for _ in 0..MAX_CGROUP_DEPTH {
        out.push(dir.clone());
        if dir == top || !dir.starts_with(&top) || !dir.pop() {
            break;
        }
    }
    out
}

/// `memory.current` of one cgroup, in bytes.
pub(crate) fn cgroup_memory_current(cgroup: &Path) -> Option<u64> {
    read_text(&cgroup.join("memory.current"))?.trim().parse().ok()
}

/// Memory still free under the cgroup limits, in bytes: for every cgroup from this one up to the
/// root that has a `memory.max`, its limit minus its own `memory.current` (which also counts its
/// other children, the siblings of this process's cgroup); the smallest wins. `None` when no
/// cgroup is limited, or when a limit or a limited cgroup's usage cannot be read (then the caller
/// must treat the free memory as unknown, not as the limit).
pub(crate) fn cgroup_memory_free(root: &Path, cgroup: &Path) -> Option<u64> {
    let mut best: Option<u64> = None;
    for dir in walk_up(root, cgroup) {
        match memory_max(&dir) {
            MemoryMax::Bytes(max) => {
                let free = max.saturating_sub(cgroup_memory_current(&dir)?);
                best = Some(best.map_or(free, |b| b.min(free)));
            }
            MemoryMax::Unknown => return None,
            MemoryMax::Absent | MemoryMax::Unlimited => {}
        }
    }
    best
}

/// The CPUs this process may run on (`Cpus_allowed_list` in `/proc/self/status`: the affinity
/// mask, which the cgroup `cpuset` also narrows).
pub(crate) fn allowed_cpus(root: &Path) -> Option<u32> {
    let text = read_text(&root.join("proc/self/status"))?;
    let list = text.lines().find_map(|l| l.strip_prefix("Cpus_allowed_list:"))?;
    parse_cpu_list(list)
}

/// The tightest `cpu.max` quota of the cgroup and its ancestors, in thousandths of a CPU
/// (`800000 100000` → 8000); `None` when unlimited or unknown.
pub(crate) fn cgroup_cpu_quota_milli(root: &Path, cgroup: &Path) -> Option<u64> {
    tightest(root, cgroup, |dir| parse_cpu_max(&read_text(&dir.join("cpu.max"))?))
}

pub(crate) fn parse_cpu_max(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let quota: u64 = parts.next()?.parse().ok()?; // "max" fails the parse: unlimited
    let period: u64 = parts.next().map_or(Some(100_000), |p| p.parse().ok())?;
    if period == 0 {
        return None;
    }
    quota.checked_mul(1000)?.checked_div(period)
}

/// The smallest value `read` finds on the way from `cgroup` up to the cgroup root.
fn tightest(root: &Path, cgroup: &Path, read: impl Fn(&Path) -> Option<u64>) -> Option<u64> {
    walk_up(root, cgroup).iter().filter_map(|dir| read(dir)).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_parses_the_kernel_format_and_rejects_garbage() {
        let info = parse_meminfo("MemTotal:       32768000 kB\nMemFree: 1 kB\nMemAvailable:   20000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n");
        assert_eq!(info.total, Some(32_768_000 * 1024));
        assert_eq!(info.available, Some(20_000_000 * 1024));
        assert_eq!((info.swap_total, info.swap_free), (Some(0), Some(0)));
        let hostile = parse_meminfo("MemTotal: 99999999999999999999 kB\nMemAvailable: 18446744073709551615 kB\nSwapTotal: 5 MB\nnonsense\n");
        assert_eq!(hostile, MemInfo::default(), "overflow and unknown units are unknown, not zero");
    }

    #[test]
    fn cpu_lists() {
        assert_eq!(parse_cpu_list("0-15\n"), Some(16));
        assert_eq!(parse_cpu_list("0,2-7"), Some(7));
        assert_eq!(parse_cpu_list("0"), Some(1));
        assert_eq!(parse_cpu_list("7-3"), None);
        assert_eq!(parse_cpu_list(""), None);
        assert_eq!(parse_cpu_list("0-4294967295,0-4294967295"), None);
    }

    #[test]
    fn pressure_lines() {
        let p = parse_pressure("some avg10=12.34 avg60=1.00 avg300=0.00 total=1\nfull avg10=0.50 avg60=0.00 avg300=0.00 total=0\n");
        assert_eq!(p, Some(Pressure { some_avg10_bp: 1234, full_avg10_bp: 50 }));
        assert_eq!(parse_pressure("some avg10=100.00 total=0"), Some(Pressure { some_avg10_bp: 10_000, full_avg10_bp: 0 }));
        assert_eq!(parse_pressure("some avg10=100.01"), None);
        assert_eq!(parse_pressure("some avg10=-1"), None);
        assert_eq!(parse_pressure("garbage"), None);
        assert_eq!(parse_percent_bp("7"), Some(700));
        assert_eq!(parse_percent_bp("0.5"), Some(50));
        assert_eq!(parse_percent_bp("4294967295.00"), None);
    }

    #[test]
    fn cpu_max() {
        assert_eq!(parse_cpu_max("800000 100000\n"), Some(8000));
        assert_eq!(parse_cpu_max("50000 100000"), Some(500));
        assert_eq!(parse_cpu_max("max 100000"), None);
        assert_eq!(parse_cpu_max("100 0"), None);
        assert_eq!(parse_cpu_max("18446744073709551615 1"), None);
    }
}
