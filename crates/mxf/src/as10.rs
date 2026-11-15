//! AMWA AS-10 (MXF for Production, v1.1, 2014): the shims of the specification and the check of
//! a file's essence against the named shim.
//!
//! AS-10 is the Sony XDCAM HD (RDD 9) MXF file with an optional DMS_AS_10 descriptive metadata
//! framework ([`crate::as11::AS10_ITEMS`]) whose `ShimName` names the constraints the file keeps.
//! The shim table is section 8.2 of the specification; an unknown shim name is a shim of the
//! user's own, so nothing is checked for it.

/// What an export writes, to compare with a shim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// MPEG-2 `ProfileAndLevel` byte: `0x44` MP@HL, `0x46` MP@H-14, `0x82` 422P@HL.
    pub profile_and_level: u8,
    /// 4:2:2 chroma (the 4:2:2 Profile).
    pub chroma_422: bool,
    pub width: u32,
    pub height: u32,
    pub audio_channels: u32,
    pub audio_bits: u32,
    /// Audio sample rate in Hz (the shims need 48 kHz PCM).
    pub audio_sample_rate: u32,
    /// Video bit rate in bits per second.
    pub bit_rate: u32,
    /// Constant (`Some(true)`) or variable bit rate; `None` when not known.
    pub cbr: Option<bool>,
}

/// One shim of AS-10 section 8.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shim {
    pub name: &'static str,
    /// Allowed `ProfileAndLevel` bytes.
    pub profiles: &'static [u8],
    pub chroma_422: bool,
    pub width: u32,
    pub height: u32,
    pub audio_channels: u32,
    pub audio_bits: u32,
    /// The video bit rate in Mbit/s the shim names (`35_VBR`: 35).
    pub mbps: &'static [u32],
    /// The shim's rate mode: CBR (`true`) or VBR.
    pub cbr: bool,
}

/// The shims of the specification.
pub const SHIMS: &[Shim] = &[
    Shim { name: "CNN_HD_2012", profiles: &[0x44], chroma_422: false, width: 1440, height: 1080, audio_channels: 4, audio_bits: 16, mbps: &[35], cbr: false },
    Shim { name: "NRK_HD_2012", profiles: &[0x82], chroma_422: true, width: 1920, height: 1080, audio_channels: 8, audio_bits: 16, mbps: &[50], cbr: true },
    Shim { name: "HIGH_HD_2014", profiles: &[0x82], chroma_422: true, width: 1920, height: 1080, audio_channels: 8, audio_bits: 24, mbps: &[50], cbr: true },
    Shim {
        name: "JVC_HD_35_VBR_2012",
        profiles: &[0x44],
        chroma_422: false,
        width: 1440,
        height: 1080,
        audio_channels: 2,
        audio_bits: 16,
        mbps: &[35],
        cbr: false,
    },
    Shim {
        name: "JVC_HD_25_CBR_2012",
        profiles: &[0x46],
        chroma_422: false,
        width: 1440,
        height: 1080,
        audio_channels: 2,
        audio_bits: 16,
        mbps: &[25],
        cbr: true,
    },
];

/// The shim named `name` (case-insensitive).
pub fn shim(name: &str) -> Option<&'static Shim> {
    SHIMS.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim()))
}

fn profile_name(p: u8) -> &'static str {
    match p {
        0x44 => "MP@HL",
        0x46 => "MP@H-14",
        0x82 => "422P@HL",
        _ => "another MPEG-2 profile",
    }
}

/// Check `facts` against the shim called `name`. An unknown shim name passes (a shim of the
/// user's own); a known one must match its video coding, chroma format, frame size, audio and
/// bit rate (with 2 % room for the rate control).
pub fn check_shim(name: &str, facts: &Facts) -> Result<(), String> {
    let Some(s) = shim(name) else { return Ok(()) };
    let mut problems: Vec<String> = Vec::new();
    if !s.profiles.contains(&facts.profile_and_level) {
        let want: Vec<&str> = s.profiles.iter().map(|p| profile_name(*p)).collect();
        problems.push(format!("video is {}, the shim needs {}", profile_name(facts.profile_and_level), want.join(" or ")));
    }
    if facts.chroma_422 != s.chroma_422 {
        problems.push(format!(
            "chroma is {}, the shim needs {}",
            if facts.chroma_422 { "4:2:2" } else { "4:2:0" },
            if s.chroma_422 { "4:2:2" } else { "4:2:0" }
        ));
    }
    if (facts.width, facts.height) != (s.width, s.height) {
        problems.push(format!("the frame is {}x{}, the shim needs {}x{}", facts.width, facts.height, s.width, s.height));
    }
    if facts.audio_channels != s.audio_channels || facts.audio_bits != s.audio_bits {
        problems.push(format!(
            "audio is {} ch / {} bit, the shim needs {} ch / {} bit (48 kHz PCM)",
            facts.audio_channels, facts.audio_bits, s.audio_channels, s.audio_bits
        ));
    }
    if facts.audio_sample_rate != 48_000 {
        problems.push(format!("audio is {} Hz, the shim needs 48 kHz PCM", facts.audio_sample_rate));
    }
    if let Some(cbr) = facts.cbr
        && cbr != s.cbr
    {
        problems.push(format!(
            "the video is {}, the shim needs {}",
            if cbr { "constant bit rate" } else { "variable bit rate" },
            if s.cbr { "constant bit rate" } else { "variable bit rate" }
        ));
    }
    let near = |mbps: u32| {
        let want = u64::from(mbps) * 1_000_000;
        u64::from(facts.bit_rate).abs_diff(want) * 50 <= want
    };
    if facts.bit_rate != 0 && !s.mbps.iter().any(|m| near(*m)) {
        problems.push(format!(
            "the video bit rate is {:.1} Mbit/s, the shim needs {} Mbit/s",
            f64::from(facts.bit_rate) / 1e6,
            s.mbps.iter().map(u32::to_string).collect::<Vec<_>>().join(" or ")
        ));
    }
    if problems.is_empty() { Ok(()) } else { Err(format!("shim {}: {}", s.name, problems.join("; "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nrk() -> Facts {
        Facts {
            profile_and_level: 0x82,
            chroma_422: true,
            width: 1920,
            height: 1080,
            audio_channels: 8,
            audio_bits: 16,
            audio_sample_rate: 48_000,
            bit_rate: 50_000_000,
            cbr: Some(true),
        }
    }

    #[test]
    fn shims_check_the_essence() {
        assert_eq!(check_shim("NRK_HD_2012", &nrk()), Ok(()));
        assert_eq!(check_shim(" nrk_hd_2012 ", &nrk()), Ok(()), "names compare case-insensitively");
        assert_eq!(check_shim("HIGH_HD_2014", &Facts { audio_bits: 24, ..nrk() }), Ok(()));
        // a shim of the user's own is not checked
        assert_eq!(check_shim("MY_SHIM", &Facts::default()), Ok(()));
        let e = check_shim("HIGH_HD_2014", &nrk()).unwrap_err();
        assert!(e.contains("HIGH_HD_2014") && e.contains("16 bit"), "{e}");
        let e = check_shim("NRK_HD_2012", &Facts { width: 1280, height: 720, bit_rate: 35_000_000, ..nrk() }).unwrap_err();
        assert!(e.contains("1280x720") && e.contains("35.0 Mbit/s"), "{e}");
        // the 4:2:0 shims
        let e = check_shim("CNN_HD_2012", &nrk()).unwrap_err();
        assert!(e.contains("MP@HL") && e.contains("4:2:0"), "{e}");
        let jvc = Facts {
            profile_and_level: 0x46,
            chroma_422: false,
            width: 1440,
            height: 1080,
            audio_channels: 2,
            audio_bits: 16,
            audio_sample_rate: 48_000,
            bit_rate: 25_000_000,
            cbr: Some(true),
        };
        assert_eq!(check_shim("JVC_HD_25_CBR_2012", &jvc), Ok(()));
        assert!(check_shim("JVC_HD_35_VBR_2012", &jvc).is_err());
        // 96 kHz audio and VBR video do not fit the CBR shims even at the right numbers
        let e = check_shim("NRK_HD_2012", &Facts { audio_sample_rate: 96_000, ..nrk() }).unwrap_err();
        assert!(e.contains("96000 Hz"), "{e}");
        let e = check_shim("NRK_HD_2012", &Facts { cbr: Some(false), ..nrk() }).unwrap_err();
        assert!(e.contains("variable bit rate"), "{e}");
        assert_eq!(check_shim("NRK_HD_2012", &Facts { cbr: None, ..nrk() }), Ok(()));
    }

    #[test]
    fn the_shims_of_the_specification_are_all_known() {
        assert_eq!(SHIMS.len(), 5);
        assert!(SHIMS.iter().all(|s| shim(s.name) == Some(s)));
    }
}
