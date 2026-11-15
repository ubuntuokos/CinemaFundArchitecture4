//! The ISDCF Digital Cinema Naming Convention (DCNC, ISDCF document 1, version 9): the claims a
//! `ContentTitleText` such as `MyFilm_FTR-1-25-3D_F_EN-fr_US-13_51-HI-VI_4K_ST_20261114_FAC_SMPTE-3D_OV`
//! makes about its package. DCP-09 of the DCP validation plan (F13); `cfa4-dcp-validate` compares the
//! claims with what the package holds (`DCP.NAME.001`).
//!
//! The fields, in order, separated by `_`:
//!
//! | Field | Form | Read as |
//! |---|---|---|
//! | Title | anything (ISDCF: at most 14 characters) | [`IsdcfName::title`] |
//! | Content type | `FTR`, `TLR`, `TSR`, `PRO`, `TST`, `RTG`, `ADV`, `SHR`, `XSN`, `PSA`, `POL`, `CLP`, `EPS`, `HLT`, `EVT` and `-` modifiers: a version number, a frame rate other than 24, `2D`/`3D`, `Temp`, `Pre`, `RedBand`, … | kind, version, frame rate, dimension |
//! | Projector aspect ratio | `F`, `S` or `C`, optionally `-` the image aspect ratio (`F-133`) | [`Aspect`] |
//! | Language | audio `-` subtitles (`-CCAP`): `EN-XX`; subtitles in lower case are burned in, in upper case they are in their own track file; `XX` is none | [`Subtitles`] |
//! | Territory-rating (optional) | `US-13`, `DE`, … | kept as text |
//! | Audio | `10`, `20`, `51`, `71`, `MOS` (no sound), `IAB`, `ATMOS`, with `-` modifiers (`HI`, `VI`, `ATMOS`, …) | channels, immersive, none |
//! | Resolution | `2K` or `4K` | |
//! | Studio (optional) | anything | |
//! | Date | `YYYYMMDD` | checked as a calendar date |
//! | Facility (optional) | anything | |
//! | Standard | `SMPTE` or `IOP`, optionally `-3D` | |
//! | Package type | `OV` or `VF` | |
//!
//! A name is recognised when its second field is a content type. The other fields are matched in
//! order; one that fits no field at its place is kept in [`IsdcfName::unknown`], and a required
//! field that is not found is listed in [`IsdcfName::missing`]. The name is a claim: it proves
//! nothing about the files.

use serde::{Deserialize, Serialize};

use crate::Standard;

/// The content type codes and the ST 429-7 `ContentKind` each stands for.
pub const CONTENT_TYPES: &[(&str, &str)] = &[
    ("FTR", "feature"),
    ("TLR", "trailer"),
    ("TSR", "teaser"),
    ("PRO", "promo"),
    ("TST", "test"),
    ("RTG", "rating"),
    ("ADV", "advertisement"),
    ("SHR", "short"),
    ("XSN", "transitional"),
    ("PSA", "psa"),
    ("POL", "policy"),
    ("CLP", "clip"),
    ("EPS", "episode"),
    ("HLT", "highlights"),
    ("EVT", "event"),
];

/// Frame rates a content type modifier may give (ISDCF: given only when not 24).
const FRAME_RATES: &[u32] = &[24, 25, 30, 48, 50, 60, 96, 100, 120];

/// The projector aspect ratio field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aspect {
    /// `F` (flat, 1.85), `S` (scope, 2.39) or `C` (full container, 1.90).
    pub container: char,
    /// The image aspect ratio in hundredths (`F-133`: 133), when given.
    pub image: Option<u32>,
}

/// The subtitles the language field claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subtitles {
    /// `XX`, or no subtitle part.
    None,
    /// Lower case: burned into the picture.
    BurnedIn(String),
    /// Upper case: in a subtitle track file of their own.
    Separate(String),
}

/// The package type field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageType {
    /// Original version: the package holds every track file it plays.
    Ov,
    /// Version file: it plays track files of another package.
    Vf,
}

/// The claims of a name in the convention.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsdcfName {
    pub title: String,
    /// The content type code (`FTR`).
    pub content_type: String,
    pub version: Option<u32>,
    /// A frame rate modifier; `None` means 24.
    pub frame_rate: Option<u32>,
    /// `Some(true)` for `3D`, `Some(false)` for `2D` (content type or standard field).
    pub stereoscopic: Option<bool>,
    /// The other content type modifiers (`Temp`, `RedBand`, …).
    pub modifiers: Vec<String>,
    pub aspect: Option<Aspect>,
    /// The audio language code (upper case), `None` for `XX`.
    pub audio_language: Option<String>,
    pub subtitles: Option<Subtitles>,
    /// `CCAP` in the language field.
    pub closed_captions: bool,
    pub territory: Option<String>,
    /// The audio field: its first part (`51`, `MOS`, `IAB`) and the modifiers.
    pub audio: Option<String>,
    pub audio_modifiers: Vec<String>,
    /// `2K` or `4K`.
    pub resolution: Option<String>,
    pub studio: Option<String>,
    /// `YYYYMMDD` as written.
    pub date: Option<String>,
    /// The date is not a calendar date.
    pub bad_date: bool,
    pub facility: Option<String>,
    pub standard: Option<Standard>,
    pub package: Option<PackageType>,
    /// Fields that fit no field at their place.
    pub unknown: Vec<String>,
    /// Required fields not found (`aspect ratio`, `language`, `audio`, `resolution`, `date`,
    /// `standard`, `package type`).
    pub missing: Vec<String>,
}

impl IsdcfName {
    /// The ST 429-7 `ContentKind` of the content type.
    pub fn content_kind(&self) -> Option<&'static str> {
        CONTENT_TYPES.iter().find(|(c, _)| *c == self.content_type).map(|(_, k)| *k)
    }

    /// The channels the audio field needs at least (`51`: 6, `71`: 8), `Some(0)` for `MOS`.
    pub fn audio_channels(&self) -> Option<u32> {
        let a = self.audio.as_deref()?;
        if a == "MOS" {
            return Some(0);
        }
        let d: Vec<u32> = a.chars().map(|c| c.to_digit(10)).collect::<Option<_>>()?;
        match d[..] {
            [main, lfe] => Some(main + lfe),
            _ => None,
        }
    }

    /// The audio field claims immersive audio (`IAB`, `ATMOS`, `Atmos`, `DTS-X`, `AURO` …).
    pub fn immersive(&self) -> bool {
        std::iter::once(self.audio.as_deref().unwrap_or("")).chain(self.audio_modifiers.iter().map(String::as_str)).any(|m| {
            let m = m.to_ascii_uppercase();
            matches!(m.as_str(), "IAB" | "ATMOS" | "DTSX" | "DTS:X" | "IMAX" | "AURO" | "AURO3D" | "IMM")
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Aspect,
    Language,
    Territory,
    Audio,
    Resolution,
    Studio,
    Date,
    Facility,
    Standard,
    Package,
}

const SLOTS: [Slot; 10] =
    [Slot::Aspect, Slot::Language, Slot::Territory, Slot::Audio, Slot::Resolution, Slot::Studio, Slot::Date, Slot::Facility, Slot::Standard, Slot::Package];

impl Slot {
    /// Studio, facility and territory take what fits nothing stricter at their place.
    fn free(self) -> bool {
        matches!(self, Slot::Territory | Slot::Studio | Slot::Facility)
    }

    fn name(self) -> &'static str {
        match self {
            Slot::Aspect => "aspect ratio",
            Slot::Language => "language",
            Slot::Territory => "territory",
            Slot::Audio => "audio",
            Slot::Resolution => "resolution",
            Slot::Studio => "studio",
            Slot::Date => "date",
            Slot::Facility => "facility",
            Slot::Standard => "standard",
            Slot::Package => "package type",
        }
    }

    fn fits(self, f: &str) -> bool {
        let parts: Vec<&str> = f.split('-').collect();
        let alpha = |p: &str, n: std::ops::RangeInclusive<usize>| n.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphabetic());
        match self {
            Slot::Aspect => {
                matches!(parts[0], "F" | "S" | "C")
                    && parts.len() <= 2
                    && parts.get(1).is_none_or(|i| (2..=3).contains(&i.len()) && i.bytes().all(|b| b.is_ascii_digit()))
            }
            Slot::Language => {
                let (langs, ccap) = match parts.split_last() {
                    Some((&"CCAP", rest)) => (rest, true),
                    _ => (&parts[..], false),
                };
                (1..=2).contains(&langs.len()) && langs.iter().all(|l| alpha(l, 2..=3)) && (langs.len() == 2 || ccap || langs[0] == "XX")
            }
            Slot::Territory => alpha(parts[0], 2..=2) && parts[0].bytes().all(|b| b.is_ascii_uppercase()),
            Slot::Audio => {
                let a = parts[0];
                (a.len() == 2 && a.bytes().all(|b| b.is_ascii_digit())) || matches!(a.to_ascii_uppercase().as_str(), "MOS" | "IAB" | "ATMOS")
            }
            Slot::Resolution => matches!(f, "2K" | "4K"),
            Slot::Date => f.len() == 8 && f.bytes().all(|b| b.is_ascii_digit()),
            Slot::Standard => matches!(f, "SMPTE" | "IOP" | "SMPTE-3D" | "IOP-3D" | "SMPTE-2D" | "IOP-2D"),
            Slot::Package => matches!(f, "OV" | "VF"),
            Slot::Studio | Slot::Facility => !f.is_empty(),
        }
    }
}

/// The claims of `name`, or `None` when it does not follow the convention (its second field is
/// not a content type).
pub fn parse(name: &str) -> Option<IsdcfName> {
    let fields: Vec<&str> = name.trim().split('_').collect();
    let [title, content, rest @ ..] = &fields[..] else { return None };
    let mut n = IsdcfName { title: (*title).to_string(), ..IsdcfName::default() };
    content_type(content, &mut n)?;
    let mut at = 0usize;
    let mut seen = Vec::new();
    for f in rest {
        // the first slot from here that fits; a free slot only when it is next (no required
        // slot skipped) and no required slot after it fits
        let pick = (at..SLOTS.len()).find(|&i| {
            let s = SLOTS[i];
            s.fits(f) && (!s.free() || (SLOTS[at..i].iter().all(|t| t.free()) && !SLOTS[i + 1..].iter().any(|t| !t.free() && t.fits(f))))
        });
        let Some(i) = pick else {
            n.unknown.push((*f).to_string());
            continue;
        };
        let slot = SLOTS[i];
        seen.push(slot);
        at = i + 1;
        fill(slot, f, &mut n);
    }
    for s in SLOTS {
        if !s.free() && !seen.contains(&s) {
            n.missing.push(s.name().to_string());
        }
    }
    Some(n)
}

fn content_type(f: &str, n: &mut IsdcfName) -> Option<()> {
    let mut parts = f.split('-');
    let code = parts.next()?;
    CONTENT_TYPES.iter().find(|(c, _)| *c == code)?;
    n.content_type = code.to_string();
    for (i, p) in parts.enumerate() {
        let number = p.parse::<u32>().ok().filter(|_| p.bytes().all(|b| b.is_ascii_digit()));
        match (i, number, p) {
            // the first number is the version (FTR-1), a later one a frame rate (FTR-1-25)
            (0, Some(v), _) if n.version.is_none() && v < 24 => n.version = Some(v),
            (_, Some(r), _) if FRAME_RATES.contains(&r) && n.frame_rate.is_none() => n.frame_rate = Some(r),
            (_, Some(v), _) if n.version.is_none() => n.version = Some(v),
            (_, None, "3D") => n.stereoscopic = Some(true),
            (_, None, "2D") => n.stereoscopic = Some(false),
            _ => n.modifiers.push(p.to_string()),
        }
    }
    Some(())
}

fn fill(slot: Slot, f: &str, n: &mut IsdcfName) {
    let parts: Vec<&str> = f.split('-').collect();
    match slot {
        Slot::Aspect => {
            let container = parts[0].chars().next().unwrap_or('F');
            n.aspect = Some(Aspect { container, image: parts.get(1).and_then(|i| i.parse().ok()) });
        }
        Slot::Language => {
            let langs: Vec<&str> = parts.iter().copied().filter(|p| *p != "CCAP").collect();
            n.closed_captions = parts.contains(&"CCAP");
            n.audio_language = langs.first().filter(|l| !l.eq_ignore_ascii_case("XX")).map(|l| l.to_ascii_uppercase());
            n.subtitles = Some(match langs.get(1) {
                None => Subtitles::None,
                Some(s) if s.eq_ignore_ascii_case("XX") => Subtitles::None,
                Some(s) if s.bytes().all(|b| b.is_ascii_lowercase()) => Subtitles::BurnedIn((*s).to_string()),
                Some(s) => Subtitles::Separate((*s).to_string()),
            });
        }
        Slot::Territory => n.territory = Some(f.to_string()),
        Slot::Audio => {
            n.audio = Some(parts[0].to_string());
            n.audio_modifiers = parts[1..].iter().map(|p| (*p).to_string()).collect();
        }
        Slot::Resolution => n.resolution = Some(f.to_string()),
        Slot::Studio => n.studio = Some(f.to_string()),
        Slot::Date => {
            n.date = Some(f.to_string());
            n.bad_date = chrono::NaiveDate::parse_from_str(f, "%Y%m%d").is_err();
        }
        Slot::Facility => n.facility = Some(f.to_string()),
        Slot::Standard => {
            n.standard = Some(if parts[0] == "IOP" { Standard::Interop } else { Standard::Smpte });
            match parts.get(1) {
                Some(&"3D") => n.stereoscopic = Some(true),
                Some(&"2D") if n.stereoscopic.is_none() => n.stereoscopic = Some(false),
                _ => {}
            }
        }
        Slot::Package => n.package = Some(if f == "VF" { PackageType::Vf } else { PackageType::Ov }),
    }
}

#[cfg(test)]
mod tests;
