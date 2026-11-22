//! AMWA AS-03 (MXF Program Delivery, rev 1.0, 2010): the shims of the specification (the three PBS
//! NGIS annexes) and the check of a file's essence against the named shim.
//!
//! An AS-03 file is OP1a with the DMS-AS-03 descriptive metadata framework
//! ([`crate::as11::AS03_ITEMS`]) whose `ShimName` names the constraints the file keeps. Annex A
//! and B (PBS NGIS HD Playout / Distribution) are 1080i 16:9 with MPEG-2 at about 25 Mbit/s or
//! H.264 at about 18 Mbit/s; Annex C (PBS NGIS SD) is 480i 4:3 MPEG-2 at 8 to 15 Mbit/s with
//! 16-bit PCM. The general provisions (5.1.6, 7.1) allow 5 to 50 Mbit/s and 48 kHz PCM with up to
//! 16 channels. An unknown shim name is a shim of the user's own, so nothing is checked for it.

/// The video coding of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Video {
    #[default]
    Other,
    Mpeg2,
    H264,
}

/// What an export writes, to compare with a shim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub video: Video,
    pub width: u32,
    pub height: u32,
    /// Video bit rate in bits per second (0 when not known).
    pub bit_rate: u32,
    pub audio_channels: u32,
    pub audio_bits: u32,
    pub audio_sample_rate: u32,
}

/// One shim of the annexes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shim {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    /// Allowed (video coding, nominal Mbit/s, lowest, highest) combinations.
    pub rates: &'static [(Video, f64, f64, f64)],
    /// PCM bits required (0: any).
    pub audio_bits: u32,
    /// Most audio channels the shim lists (surround plus pairs, or one to three tracks).
    pub max_channels: u32,
}

const PBS_HD_RATES: &[(Video, f64, f64, f64)] = &[(Video::Mpeg2, 25.0, 22.5, 27.5), (Video::H264, 18.0, 16.2, 19.8)];

/// The shims of the specification.
pub const SHIMS: &[Shim] = &[
    Shim { name: "PBS NGIS HD Playout", width: 1920, height: 1080, rates: PBS_HD_RATES, audio_bits: 0, max_channels: 14 },
    Shim { name: "PBS NGIS HD Distribution", width: 1920, height: 1080, rates: PBS_HD_RATES, audio_bits: 0, max_channels: 14 },
    Shim { name: "PBS NGIS SD", width: 720, height: 480, rates: &[(Video::Mpeg2, 11.5, 8.0, 15.0)], audio_bits: 16, max_channels: 6 },
];

/// The shim named `name` (case-insensitive).
pub fn shim(name: &str) -> Option<&'static Shim> {
    SHIMS.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim()))
}

fn video_name(v: Video) -> &'static str {
    match v {
        Video::Mpeg2 => "MPEG-2",
        Video::H264 => "H.264",
        Video::Other => "another video coding",
    }
}

/// The general provisions of 5.1.5 and 5.1.6: MPEG-2 or H.264 at 5 to 50 Mbit/s, 48 kHz PCM, up to
/// 16 channels. Checked for every file, whatever its shim.
pub fn check_general(facts: &Facts) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    if facts.video == Video::Other {
        problems.push("AS-03 video is MPEG-2 or H.264".into());
    }
    if facts.bit_rate != 0 && !(5_000_000..=50_000_000).contains(&facts.bit_rate) {
        problems.push(format!("the video bit rate is {:.1} Mbit/s, AS-03 allows 5 to 50 Mbit/s", f64::from(facts.bit_rate) / 1e6));
    }
    if facts.audio_sample_rate != 0 && facts.audio_sample_rate != 48_000 {
        problems.push(format!("audio is {} Hz, AS-03 needs 48 kHz PCM", facts.audio_sample_rate));
    }
    if facts.audio_channels > 16 {
        problems.push(format!("audio has {} channels, AS-03 allows up to 16", facts.audio_channels));
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
}

/// Check `facts` against the shim called `name` (and the general provisions). An unknown shim name
/// passes the shim part.
pub fn check_shim(name: &str, facts: &Facts) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    if let Err(e) = check_general(facts) {
        problems.push(e);
    }
    let Some(s) = shim(name) else {
        return if problems.is_empty() { Ok(()) } else { Err(format!("AS-03: {}", problems.join("; "))) };
    };
    if (facts.width, facts.height) != (s.width, s.height) {
        problems.push(format!("the frame is {}x{}, the shim needs {}x{}", facts.width, facts.height, s.width, s.height));
    }
    match s.rates.iter().find(|r| r.0 == facts.video) {
        None => {
            let want: Vec<&str> = s.rates.iter().map(|r| video_name(r.0)).collect();
            problems.push(format!("video is {}, the shim needs {}", video_name(facts.video), want.join(" or ")));
        }
        Some(&(_, nominal, lo, hi)) => {
            let mbps = f64::from(facts.bit_rate) / 1e6;
            if facts.bit_rate != 0 && !(lo..=hi).contains(&mbps) {
                problems.push(format!("the {} bit rate is {mbps:.1} Mbit/s, the shim needs about {nominal} Mbit/s ({lo} to {hi})", video_name(facts.video)));
            }
        }
    }
    if s.audio_bits != 0 && facts.audio_bits != 0 && facts.audio_bits != s.audio_bits {
        problems.push(format!("audio is {} bit, the shim needs {} bit PCM", facts.audio_bits, s.audio_bits));
    }
    if facts.audio_channels > s.max_channels {
        problems.push(format!("audio has {} channels, the shim lists up to {}", facts.audio_channels, s.max_channels));
    }
    if problems.is_empty() { Ok(()) } else { Err(format!("shim {}: {}", s.name, problems.join("; "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hd() -> Facts {
        Facts { video: Video::Mpeg2, width: 1920, height: 1080, bit_rate: 25_000_000, audio_channels: 8, audio_bits: 24, audio_sample_rate: 48_000 }
    }

    #[test]
    fn shims_check_the_essence() {
        assert_eq!(check_shim("PBS NGIS HD Playout", &hd()), Ok(()));
        assert_eq!(check_shim(" pbs ngis hd distribution ", &hd()), Ok(()));
        assert_eq!(check_shim("PBS NGIS HD Playout", &Facts { video: Video::H264, bit_rate: 18_000_000, ..hd() }), Ok(()));
        let e = check_shim("PBS NGIS HD Playout", &Facts { video: Video::H264, ..hd() }).unwrap_err();
        assert!(e.contains("H.264 bit rate is 25.0"), "{e}");
        let e = check_shim("PBS NGIS HD Playout", &Facts { width: 1280, height: 720, ..hd() }).unwrap_err();
        assert!(e.contains("1280x720"), "{e}");
        let sd = Facts { width: 720, height: 480, bit_rate: 10_000_000, audio_channels: 2, audio_bits: 16, ..hd() };
        assert_eq!(check_shim("PBS NGIS SD", &sd), Ok(()));
        let e = check_shim("PBS NGIS SD", &Facts { audio_bits: 24, video: Video::H264, ..sd }).unwrap_err();
        assert!(e.contains("24 bit") && e.contains("MPEG-2"), "{e}");
        assert!(check_shim("PBS NGIS SD", &Facts { audio_channels: 8, ..sd }).is_err());
    }

    #[test]
    fn the_general_provisions_apply_to_every_shim() {
        assert_eq!(check_shim("MY SHIM", &hd()), Ok(()), "a shim of the user's own keeps the general provisions only");
        for bad in [
            Facts { video: Video::Other, ..hd() },
            Facts { bit_rate: 60_000_000, ..hd() },
            Facts { bit_rate: 2_000_000, ..hd() },
            Facts { audio_sample_rate: 44_100, ..hd() },
            Facts { audio_channels: 17, ..hd() },
        ] {
            assert!(check_shim("MY SHIM", &bad).unwrap_err().starts_with("AS-03:"), "{bad:?}");
        }
        assert_eq!(SHIMS.len(), 3);
        assert!(SHIMS.iter().all(|s| shim(s.name) == Some(s)));
    }
}
