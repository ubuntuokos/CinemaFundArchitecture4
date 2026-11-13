//! PTP state for the live studio (CFA4-LIVE-STUDIO-001 ST 2110 addendum, phase T1).
//!
//! `linuxptp` runs PTP (`ptp4l` with the ST 2059-2 profile, `phc2sys` disciplining the system
//! clock); CFA4 only reads it (plan D9). This module asks `ptp4l` through `pmc` over its local
//! socket, reads the system clock, and reports the lock, the grandmaster, the TAI time and the
//! SDP clock signalling a sender uses now (`ts-refclk` / `mediaclk`). Without `pmc`, without an
//! answer, or without a lock, the report says so and the sender signals its own timebase; nothing
//! here is an error for the caller.
//!
//! The system clock is UTC (`phc2sys` keeps `CLOCK_REALTIME` on UTC); TAI is UTC plus the
//! grandmaster's announced `currentUtcOffset` (37 s when none is valid). No `unsafe` is needed.

use cfa4_st2110::clock::{DEFAULT_LOCK_LIMIT_NS, PtpStatus, PtpTime, sender_clocks};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The ST 2059-2 default PTP domain; `CFA4_PTP_DOMAIN` overrides it (`pmc` only gets answers in
/// the domain `ptp4l` runs in).
pub const ST2059_DEFAULT_DOMAIN: u8 = 127;

/// How long `pmc` may take before it is stopped.
pub const PMC_TIMEOUT: Duration = Duration::from_secs(2);

/// What the live studio knows about PTP right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpReport {
    /// `pmc` ran and `ptp4l` answered.
    pub available: bool,
    /// Why not, when `available` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Locked to a grandmaster (slave port, grandmaster present, offset within 1 µs).
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_state: Option<String>,
    /// The grandmaster's clock identity, `00-11-22-FF-FE-33-44-55`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grandmaster: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_ns: Option<i64>,
    /// TAI − UTC in seconds used for `tai`.
    pub tai_minus_utc: i64,
    /// TAI time since the SMPTE Epoch, seconds and nanoseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tai_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tai_nanos: Option<u32>,
    /// The SDP attributes a sender uses now: `a=ts-refclk:` and `a=mediaclk:` values.
    pub ts_refclk: String,
    pub mediaclk: String,
}

/// Build the report from `pmc`'s answer (or why there is none), the system time and the
/// sender's MAC. Pure, for tests.
pub fn report_from(pmc: Result<String, String>, now: SystemTime, mac: [u8; 6]) -> PtpReport {
    let (status, reason) = match pmc {
        Ok(text) => match PtpStatus::parse_pmc(&text) {
            Ok(s) if s.port_state.is_some() => (Some(s), None),
            Ok(_) => (None, Some("ptp4l did not answer (not running, or another domain)".to_string())),
            Err(e) => (None, Some(format!("unreadable pmc answer: {e}"))),
        },
        Err(e) => (None, Some(e)),
    };
    let (refclk, mediaclk) = sender_clocks(status.as_ref(), mac, DEFAULT_LOCK_LIMIT_NS);
    let tai_minus_utc = status.as_ref().map_or(cfa4_st2110::clock::TAI_UTC_OFFSET_2017, PtpStatus::tai_minus_utc);
    let tai = now.duration_since(UNIX_EPOCH).ok().and_then(|d| PtpTime::from_unix(d.as_secs(), d.subsec_nanos(), tai_minus_utc).ok());
    let s = status.unwrap_or_default();
    PtpReport {
        available: reason.is_none(),
        reason,
        locked: s.locked(DEFAULT_LOCK_LIMIT_NS),
        port_state: s.port_state.map(|p| format!("{p:?}")),
        grandmaster: s.grandmaster.map(|g| g.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("-")),
        domain: s.domain,
        offset_ns: s.master_offset_ns,
        tai_minus_utc,
        tai_seconds: tai.map(|t| t.seconds),
        tai_nanos: tai.map(|t| t.nanos),
        ts_refclk: refclk.to_value(),
        mediaclk: mediaclk.to_value(),
    }
}

/// `pmc` on `PATH` or in `/usr/sbin` (where Ubuntu's `linuxptp` installs it).
pub fn find_pmc() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push(PathBuf::from("/usr/sbin"));
    dirs.into_iter().map(|d| d.join("pmc")).find(|p| p.is_file())
}

/// Ask `ptp4l` through `pmc` (local socket, no network traffic), stopping it after `timeout`.
pub fn run_pmc(pmc: &Path, domain: u8, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(pmc)
        .args(["-u", "-b", "0", "-d", &domain.to_string()])
        .args(["GET TIME_STATUS_NP", "GET DEFAULT_DATA_SET", "GET PORT_DATA_SET", "GET TIME_PROPERTIES_DATA_SET"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", pmc.display()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("pmc did not finish in time".into());
            }
            Err(e) => return Err(format!("pmc: {e}")),
        }
    }
    let mut out = String::new();
    child.stdout.take().ok_or("pmc: no output")?.take(1 << 20).read_to_string(&mut out).map_err(|e| format!("pmc output: {e}"))?;
    Ok(out)
}

/// The MAC of `CFA4_PTP_INTERFACE`, else of the first network interface (by name) that is not
/// loopback and has a non-zero address; `02-00-00-00-00-00` (locally administered) when none.
pub fn local_mac() -> [u8; 6] {
    let read = |name: &str| std::fs::read_to_string(format!("/sys/class/net/{name}/address")).ok().and_then(|t| parse_mac(t.trim()));
    if let Some(m) = std::env::var("CFA4_PTP_INTERFACE").ok().and_then(|i| read(&i)) {
        return m;
    }
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names.iter().filter(|n| n.as_str() != "lo").filter_map(|n| read(n)).find(|m| m.iter().any(|&b| b != 0)).unwrap_or([0x02, 0, 0, 0, 0, 0])
}

fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = s.split(':');
    for b in &mut out {
        *b = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The PTP report now: `pmc` (domain from `CFA4_PTP_DOMAIN`, else 127), the system clock and
/// the local MAC.
pub fn query() -> PtpReport {
    let domain = std::env::var("CFA4_PTP_DOMAIN").ok().and_then(|d| d.parse().ok()).unwrap_or(ST2059_DEFAULT_DOMAIN);
    let answer = match find_pmc() {
        Some(pmc) => run_pmc(&pmc, domain, PMC_TIMEOUT),
        None => Err("pmc not found (install linuxptp)".into()),
    };
    report_from(answer, SystemTime::now(), local_mac())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];

    // Built for these tests in the layout pmc prints (checked against linuxptp 4.0's pmc).
    const LOCKED: &str = "\t\tmaster_offset -40\n\t\tgmPresent true\n\t\tgmIdentity 0c4f5a.fffe.10e2d4\n\t\tdomainNumber 127\n\t\tportState SLAVE\n\t\tcurrentUtcOffset 37\n\t\tcurrentUtcOffsetValid 1\n";

    #[test]
    fn locked_report_signals_the_grandmaster() {
        let now = UNIX_EPOCH + Duration::new(1_762_905_600, 250);
        let r = report_from(Ok(LOCKED.into()), now, MAC);
        assert!(r.available && r.locked);
        assert_eq!(r.ts_refclk, "ptp=IEEE1588-2008:0C-4F-5A-FF-FE-10-E2-D4:127");
        assert_eq!(r.mediaclk, "direct=0");
        assert_eq!((r.tai_seconds, r.tai_nanos), (Some(1_762_905_637), Some(250)));
        assert_eq!(r.port_state.as_deref(), Some("Slave"));
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["tsRefclk"], "ptp=IEEE1588-2008:0C-4F-5A-FF-FE-10-E2-D4:127");
        assert_eq!(j["offsetNs"], -40);
    }

    #[test]
    fn no_ptp_signals_the_local_timebase() {
        let now = UNIX_EPOCH + Duration::from_secs(1_762_905_600);
        for answer in
            [Err("pmc not found (install linuxptp)".to_string()), Ok("sending: GET PORT_DATA_SET\n".to_string()), Ok("\t\tportState SLEEPING\n".to_string())]
        {
            let r = report_from(answer, now, MAC);
            assert!(!r.available && !r.locked);
            assert!(r.reason.is_some());
            assert_eq!((r.ts_refclk.as_str(), r.mediaclk.as_str()), ("localmac=02-11-22-33-44-55", "sender"));
            assert_eq!(r.tai_seconds, Some(1_762_905_637));
        }
        // ptp4l answers but this machine is the master: available, not locked.
        let r = report_from(Ok(LOCKED.replace("SLAVE", "MASTER")), now, MAC);
        assert!(r.available && !r.locked);
        assert_eq!(r.mediaclk, "sender");
    }

    #[test]
    fn macs() {
        assert_eq!(parse_mac("3c:2c:30:a1:b2:c3"), Some([0x3c, 0x2c, 0x30, 0xa1, 0xb2, 0xc3]));
        assert_eq!(parse_mac("3c:2c:30:a1:b2"), None);
        assert_eq!(parse_mac("3c:2c:30:a1:b2:c3:00"), None);
        let m = local_mac();
        assert!(m.iter().any(|&b| b != 0));
    }

    #[test]
    fn query_never_fails() {
        // Whatever this machine has (no pmc, pmc without ptp4l, a running ptp4l), a report comes
        // back with SDP values a sender can use.
        let r = query();
        assert!(!r.ts_refclk.is_empty() && !r.mediaclk.is_empty());
        assert!(r.tai_seconds.is_some());
    }
}
