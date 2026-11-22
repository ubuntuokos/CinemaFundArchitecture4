//! AMWA AS-11 shims `UK DPP HD` and `UK DPP SD` (version 1.1): the checks a file must pass to
//! carry that `ShimName`.
//!
//! The rules come from the published, Apache-2.0 licensed specifications (AS-11 UK DPP HD / SD,
//! github.com/amwa-tv): the Core and UK DPP framework constraints (the `dm_extras.py` conditions,
//! the enumerations, the text length limits, the picture ratios, the audio track layouts) and the
//! essence constraints this crate can see (picture coding, channels, sample size). No code was
//! copied. Any other shim name is a shim of the user's own, so nothing is checked for it.

/// The picture coding of the file, as far as the shims care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Video {
    /// H.264 intra with `profile_idc` (110 = High 10 Intra).
    AvcIntra {
        profile_idc: u8,
    },
    /// SMPTE ST 386 D-10 at `mbps` Mbit/s with `lines` (625 or 525).
    D10 {
        mbps: u8,
        lines: u16,
    },
    Other,
}

/// What an export writes, to compare with a shim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub video: Video,
    pub width: u32,
    pub height: u32,
    /// Frame rate numerator and denominator (25/1 for the shims).
    pub rate: (u32, u32),
    pub interlaced: bool,
    pub audio_channels: u32,
    pub audio_bits: u32,
    pub audio_rate: u32,
}

/// The two shims of this module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shim {
    UkDppHd,
    UkDppSd,
}

impl Shim {
    /// The shim named `name` (`UK DPP HD`, `UK DPP SD`; case and spaces are not significant).
    pub fn named(name: &str) -> Option<Shim> {
        let key: String = name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
        match key.as_str() {
            "UKDPPHD" => Some(Shim::UkDppHd),
            "UKDPPSD" => Some(Shim::UkDppSd),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shim::UkDppHd => "UK DPP HD",
            Shim::UkDppSd => "UK DPP SD",
        }
    }

    /// `AudioTrackLayout` values the shim allows (AS-11 Core enumeration): HD EBU R 48 2a,
    /// R 123 4b, 4c, 16c, 16d, 16f; SD the first three.
    fn layouts(self) -> &'static [u8] {
        match self {
            Shim::UkDppHd => &[3, 28, 29, 49, 50, 52],
            Shim::UkDppSd => &[3, 28, 29],
        }
    }
}

fn get<'a>(items: &'a [(String, String)], name: &str) -> Option<&'a str> {
    items.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

fn flag(items: &[(String, String)], name: &str) -> Option<bool> {
    get(items, name).map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
}

fn number(items: &[(String, String)], name: &str) -> Option<u32> {
    get(items, name).and_then(|v| v.parse().ok())
}

/// Maximum code points of the UK DPP text items (the registers' "Restrictions").
const TEXT_LIMITS: &[(&str, usize)] = &[
    ("SeriesTitle", 127),
    ("ProgrammeTitle", 127),
    ("EpisodeTitleNumber", 127),
    ("ProductionNumber", 127),
    ("Synopsis", 250),
    ("Originator", 127),
    ("OtherIdentifier", 127),
    ("OtherIdentifierType", 127),
    ("Genre", 127),
    ("Distributor", 127),
    ("PSEManufacturer", 127),
    ("PSEVersion", 127),
    ("VideoComments", 127),
    ("AudioComments", 127),
    ("ContactEmail", 127),
    ("ContactTelephoneNumber", 127),
];

/// Language items: an ISO 639-2 three-letter code.
const LANGUAGES: &[&str] =
    &["PrimaryAudioLanguage", "ClosedCaptionsLanguage", "SecondaryAudioLanguage", "TertiaryAudioLanguage", "OpenCaptionsLanguage", "ProgrammeTextLanguage"];

/// Enumerations: item and the number of values (0-based).
const ENUMS: &[(&str, u8)] = &[
    ("ClosedCaptionsType", 2),
    ("OpenCaptionsType", 2),
    ("ThreeDType", 4),
    ("PSEPass", 3),
    ("SigningPresent", 3),
    ("SignLanguage", 2),
    ("AudioLoudnessStandard", 2),
    ("AudioDescriptionType", 2),
];

/// The picture ratios the shims allow.
const RATIOS: &[(i32, i32)] = &[(4, 3), (14, 9), (15, 9), (16, 9), (37, 20), (21, 9), (12, 5)];

/// Check the Core and UK DPP framework text of an export against `shim`.
pub fn check_metadata(shim: Shim, core: &[(String, String)], ukdpp: &[(String, String)]) -> Result<(), String> {
    let mut p: Vec<String> = Vec::new();
    let all = || core.iter().chain(ukdpp.iter());
    // shim identity
    if let Some(v) = get(core, "ShimVersion") {
        let (a, b) = v.split_once('.').unwrap_or((v, "0"));
        if (a.trim(), b.trim()) != ("1", "1") {
            p.push(format!("ShimVersion is {v}, {} needs 1.1", shim.name()));
        }
    }
    // text limits, languages, enumerations
    for (name, max) in TEXT_LIMITS {
        if let Some(v) = all().find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v)
            && v.chars().count() > *max
        {
            p.push(format!("{name} has more than {max} characters"));
        }
    }
    for name in LANGUAGES {
        if let Some(v) = get(core, name).or_else(|| get(ukdpp, name))
            && !(v.len() == 3 && v.bytes().all(|b| b.is_ascii_alphabetic()))
        {
            p.push(format!("{name} \"{v}\" is not an ISO 639-2 code (three letters)"));
        }
    }
    for (name, count) in ENUMS {
        if let Some(v) = get(core, name).or_else(|| get(ukdpp, name))
            && v.parse::<u8>().map_or(true, |n| n >= *count)
        {
            p.push(format!("{name} \"{v}\" is not a value of its enumeration (0..{})", count.saturating_sub(1)));
        }
    }
    // Core: layout, captions
    if let Some(v) = get(core, "AudioTrackLayout") {
        match v.parse::<u8>() {
            Ok(n) if shim.layouts().contains(&n) => {}
            _ => p.push(format!(
                "AudioTrackLayout {v} is not one {} allows (EBU R 48 2a, R 123 4b, 4c{})",
                shim.name(),
                if shim == Shim::UkDppHd { ", 16c, 16d, 16f" } else { "" }
            )),
        }
    }
    if flag(core, "ClosedCaptionsPresent") == Some(true) {
        p.push(format!("{} files carry no closed captions (ClosedCaptionsPresent must be false)", shim.name()));
    }
    // UK DPP conditions (dm_extras.py)
    let need = |p: &mut Vec<String>, cond: &str, names: &[&str]| {
        for n in names {
            if get(ukdpp, n).is_none() {
                p.push(format!("{n} is required when {cond}"));
            }
        }
    };
    if get(ukdpp, "OtherIdentifier").is_some() {
        need(&mut p, "OtherIdentifier is given", &["OtherIdentifierType"]);
    }
    if flag(ukdpp, "ThreeD") == Some(true) {
        need(&mut p, "ThreeD is true", &["ThreeDType"]);
    }
    if number(ukdpp, "PSEPass").is_some_and(|v| v != 2) {
        need(&mut p, "PSEPass is not 2 (Not tested)", &["PSEManufacturer", "PSEVersion"]);
    }
    if flag(ukdpp, "AudioDescriptionPresent") == Some(true) {
        need(&mut p, "AudioDescriptionPresent is true", &["AudioDescriptionType"]);
    }
    if flag(ukdpp, "OpenCaptionsPresent") == Some(true) {
        need(&mut p, "OpenCaptionsPresent is true", &["OpenCaptionsType", "OpenCaptionsLanguage"]);
    }
    if number(ukdpp, "SigningPresent").is_some_and(|v| v == 0 || v == 2) {
        need(&mut p, "SigningPresent is Yes or Signer only", &["SignLanguage"]);
    }
    if flag(ukdpp, "ProgrammeHasText") == Some(true) {
        need(&mut p, "ProgrammeHasText is true", &["ProgrammeTextLanguage"]);
    }
    if let Some(v) = get(ukdpp, "PictureRatio") {
        let ok = v
            .split_once(['/', ':'])
            .and_then(|(a, b)| Some((a.trim().parse::<i32>().ok()?, b.trim().parse::<i32>().ok()?)))
            .is_some_and(|r| RATIOS.contains(&r));
        if !ok {
            p.push(format!("PictureRatio {v} is not 4/3, 14/9, 15/9, 16/9, 37/20, 21/9 or 12/5"));
        }
    }
    if let Some(v) = get(ukdpp, "CompletionDate")
        && v.len() > 10
        && !v.ends_with("T00:00:00")
    {
        p.push("CompletionDate has a time part; the shim takes a date only".into());
    }
    if p.is_empty() { Ok(()) } else { Err(format!("{} (version 1.1): {}", shim.name(), p.join("; "))) }
}

/// Check the picture and sound of an export against `shim`.
pub fn check_essence(shim: Shim, f: &Facts) -> Result<(), String> {
    let mut p: Vec<String> = Vec::new();
    match shim {
        Shim::UkDppHd => {
            if f.video != (Video::AvcIntra { profile_idc: 110 }) {
                p.push("video must be AVC High 10 Intra (RP 2027 class 100)".into());
            }
            if (f.width, f.height) != (1920, 1080) || f.rate != (25, 1) || !f.interlaced {
                p.push(format!(
                    "picture must be 1920x1080 at 25 fps interlaced (1080i25), not {}x{} {}/{}{}",
                    f.width,
                    f.height,
                    f.rate.0,
                    f.rate.1,
                    if f.interlaced { "i" } else { "p" }
                ));
            }
            if f.audio_channels != 4 && f.audio_channels != 16 {
                p.push(format!("the file needs exactly 4 or 16 audio channels, not {}", f.audio_channels));
            }
        }
        Shim::UkDppSd => {
            if f.video != (Video::D10 { mbps: 50, lines: 625 }) {
                p.push("video must be D-10 50 Mbit/s 625/50 (ST 386)".into());
            }
            if f.audio_channels != 4 {
                p.push(format!("the file needs 4 audio channels, not {}", f.audio_channels));
            }
        }
    }
    if f.audio_rate != 48_000 || f.audio_bits != 24 {
        p.push(format!("audio must be uncompressed 24-bit 48 kHz PCM, not {}-bit {} Hz", f.audio_bits, f.audio_rate));
    }
    if p.is_empty() { Ok(()) } else { Err(format!("{}: {}", shim.name(), p.join("; "))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(s: &str) -> Vec<(String, String)> {
        s.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
    }

    fn hd_core() -> Vec<(String, String)> {
        kv("ShimName: UK DPP HD\nShimVersion: 1.1\nAudioTrackLayout: 3\nPrimaryAudioLanguage: eng\nClosedCaptionsPresent: false")
    }

    #[test]
    fn shim_names() {
        assert_eq!(Shim::named("uk dpp hd"), Some(Shim::UkDppHd));
        assert_eq!(Shim::named("UKDPP SD"), Some(Shim::UkDppSd));
        assert_eq!(Shim::named("my own"), None);
    }

    #[test]
    fn good_metadata_passes() {
        let dpp = kv("PictureRatio: 16/9\nPSEPass: 2\nOpenCaptionsPresent: false\nSigningPresent: 1");
        assert_eq!(check_metadata(Shim::UkDppHd, &hd_core(), &dpp), Ok(()));
    }

    #[test]
    fn conditions_and_limits_are_checked() {
        let mut core = hd_core();
        core.retain(|(k, _)| k != "ClosedCaptionsPresent");
        core.push(("ClosedCaptionsPresent".into(), "true".into()));
        let dpp = kv("PSEPass: 0\nOpenCaptionsPresent: true\nSigningPresent: 0\nPictureRatio: 5/4\nOtherIdentifier: x\nSecondaryAudioLanguage: en");
        let e = check_metadata(Shim::UkDppHd, &core, &dpp).unwrap_err();
        for want in [
            "no closed captions",
            "PSEManufacturer",
            "OpenCaptionsType",
            "SignLanguage",
            "PictureRatio 5/4",
            "OtherIdentifierType",
            "SecondaryAudioLanguage \"en\"",
        ] {
            assert!(e.contains(want), "{want} missing in {e}");
        }
        let long = vec![("Synopsis".to_string(), "x".repeat(251))];
        assert!(check_metadata(Shim::UkDppSd, &hd_core(), &long).unwrap_err().contains("Synopsis"));
    }

    #[test]
    fn layouts_differ_between_hd_and_sd() {
        let mut core = hd_core();
        core.retain(|(k, _)| k != "AudioTrackLayout");
        core.push(("AudioTrackLayout".into(), "49".into()));
        assert_eq!(check_metadata(Shim::UkDppHd, &core, &[]), Ok(()));
        assert!(check_metadata(Shim::UkDppSd, &core, &[]).unwrap_err().contains("AudioTrackLayout 49"));
        core.retain(|(k, _)| k != "ShimVersion");
        core.push(("ShimVersion".into(), "2.0".into()));
        assert!(check_metadata(Shim::UkDppHd, &core, &[]).unwrap_err().contains("needs 1.1"));
    }

    fn hd_facts() -> Facts {
        Facts {
            video: Video::AvcIntra { profile_idc: 110 },
            width: 1920,
            height: 1080,
            rate: (25, 1),
            interlaced: true,
            audio_channels: 4,
            audio_bits: 24,
            audio_rate: 48_000,
        }
    }

    #[test]
    fn essence_is_checked() {
        assert_eq!(check_essence(Shim::UkDppHd, &hd_facts()), Ok(()));
        let bad = Facts { audio_channels: 2, audio_bits: 16, interlaced: false, video: Video::Other, ..hd_facts() };
        let e = check_essence(Shim::UkDppHd, &bad).unwrap_err();
        for want in ["High 10 Intra", "1080i25", "4 or 16", "24-bit"] {
            assert!(e.contains(want), "{want} missing in {e}");
        }
        let sd = Facts { video: Video::D10 { mbps: 50, lines: 625 }, width: 720, height: 608, audio_channels: 4, ..hd_facts() };
        assert_eq!(check_essence(Shim::UkDppSd, &sd), Ok(()));
        assert!(check_essence(Shim::UkDppSd, &hd_facts()).unwrap_err().contains("D-10"));
    }
}
