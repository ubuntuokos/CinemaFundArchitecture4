//! DCP-09 (plan F13): `DCP.NAME.001`, the ISDCF name in each composition's `ContentTitleText`
//! (read with `cfa4_dcp::isdcf`) against what the package holds. A name is a claim, so every
//! problem is a `WARNING`:
//!
//! - the title does not follow the convention, lacks a required field, has a field out of place or
//!   a date that is not a calendar date;
//! - the content type is not the CPL `ContentKind`;
//! - the frame rate (24 when the name gives none) is not the picture's edit rate;
//! - `3D` is claimed for a 2D picture, or a stereoscopic picture is not named `3D`;
//! - the projector aspect ratio (`F`, `S`, `C`) or the resolution (`2K`, `4K`) is not the
//!   picture's container;
//! - the standard is not the composition's;
//! - the audio field claims more channels than the sound has, sound where there is none (or `MOS`
//!   where there is sound), or immersive audio the package does not have (or the other way round);
//! - the audio or subtitle language is not the CPL's `Language`, subtitles in their own track
//!   file are claimed (upper case) and there are none, or there are and the name says `XX`;
//!   `CCAP` is claimed without a caption track or the other way round;
//! - `OV` is claimed and the composition plays track files from elsewhere, or `VF` and it does not.
//!
//! The picture size, sound channels and immersive audio come from the track files' MXF headers
//! (`cfa4-mxf`); the rest from the composition.

use cfa4_dcp::isdcf::{self, IsdcfName, PackageType, Subtitles};
use cfa4_dcp::{AssetKind, Composition, Rational, Standard};
use cfa4_mxf::{Codec, TrackKind};

use crate::integrity::{Files, at};
use crate::media::FileSource;
use crate::rules::NAME_ISDCF;
use crate::{Finding, Location, Status, ValidationControl};

/// What the package holds, as far as a name speaks of it.
#[derive(Default)]
struct Facts {
    /// Picture: stored size and edit rate (per eye).
    size: Option<(u32, u32)>,
    rate: Option<Rational>,
    stereo: bool,
    /// Most channels of a sound track file; `None` without sound.
    channels: Option<u32>,
    sound_language: Option<String>,
    immersive: bool,
    subtitles: bool,
    subtitle_language: Option<String>,
    captions: bool,
    /// Track files of the composition that are not in the package.
    external: usize,
}

fn facts(c: &Composition, files: &Files) -> Facts {
    let mut f = Facts::default();
    for a in c.reels.iter().flat_map(|r| &r.assets) {
        let file = a.id.and_then(|id| files.get(&id));
        if a.id.is_some() && file.is_none() && !matches!(a.kind, AssetKind::MainMarkers | AssetKind::CompositionMetadata | AssetKind::Other) {
            f.external += 1;
        }
        let mxf = || {
            let src = FileSource::open(&file?.1).ok()?;
            cfa4_mxf::open(&src).ok()
        };
        match a.kind {
            AssetKind::MainPicture | AssetKind::MainStereoscopicPicture => {
                f.stereo |= a.kind == AssetKind::MainStereoscopicPicture;
                f.rate = f.rate.or(a.edit_rate);
                if f.size.is_none() {
                    f.size = mxf()
                        .and_then(|m| m.tracks.into_iter().find_map(|t| t.picture.filter(|_| t.kind == TrackKind::Picture)))
                        .map(|p| (p.stored_width, p.stored_height));
                }
            }
            AssetKind::MainSound => {
                let n = mxf().and_then(|m| m.tracks.into_iter().find_map(|t| t.sound)).map_or(0, |s| s.channels);
                f.channels = Some(f.channels.unwrap_or(0).max(n));
                f.sound_language = f.sound_language.take().or_else(|| a.language.clone());
            }
            AssetKind::AuxData if !f.immersive => f.immersive = mxf().is_some_and(|m| m.tracks.iter().any(|t| t.codec == Codec::Iab)),
            AssetKind::MainSubtitle => {
                f.subtitles = true;
                f.subtitle_language = f.subtitle_language.take().or_else(|| a.language.clone());
            }
            AssetKind::MainCaption | AssetKind::MainClosedCaption => f.captions = true,
            _ => {}
        }
    }
    f
}

/// The DCI containers of each projector aspect ratio, 2K and 4K.
fn container(size: (u32, u32)) -> Option<char> {
    match size {
        (1998, 1080) | (3996, 2160) => Some('F'),
        (2048, 858) | (4096, 1716) => Some('S'),
        (2048, 1080) | (4096, 2160) => Some('C'),
        _ => None,
    }
}

/// The primary language subtag, lower case, when it is a two-letter code.
fn two_letters(tag: &str) -> Option<String> {
    let p = tag.split(['-', '_']).next()?;
    (p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic())).then(|| p.to_ascii_lowercase())
}

fn standard_name(s: Standard) -> &'static str {
    if s == Standard::Interop { "IOP" } else { "SMPTE" }
}

/// The mismatches between the claims of `n` and the package.
fn compare(n: &IsdcfName, c: &Composition, f: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    if !n.missing.is_empty() {
        out.push(format!("lacks the fields {}", n.missing.join(", ")));
    }
    if !n.unknown.is_empty() {
        out.push(format!("has fields out of place or not in the convention: {}", n.unknown.iter().map(|u| format!("'{u}'")).collect::<Vec<_>>().join(", ")));
    }
    if n.bad_date {
        out.push(format!("the date '{}' is not a calendar date YYYYMMDD", n.date.as_deref().unwrap_or("")));
    }
    if let (Some(kind), Some(ck)) = (n.content_kind(), c.content_kind.as_deref())
        && !kind.eq_ignore_ascii_case(ck.trim())
    {
        out.push(format!("the content type {} is a {kind}, the ContentKind is {ck}", n.content_type));
    }
    if let Some(r) = f.rate
        && r.den > 0
        && r.num % r.den == 0
    {
        let claimed = n.frame_rate.unwrap_or(24);
        if i64::from(claimed) != r.num / r.den {
            let how = if n.frame_rate.is_some() { "" } else { " (no frame rate in the name means 24)" };
            out.push(format!("the name claims {claimed} fps{how}, the picture is {} fps", r.num / r.den));
        }
    }
    match (n.stereoscopic, f.stereo) {
        (Some(true), false) => out.push("the name claims 3D, the picture is 2D".into()),
        (Some(false) | None, true) => out.push("the picture is stereoscopic, the name does not say 3D".into()),
        _ => {}
    }
    if let Some(size) = f.size {
        if let (Some(a), Some(have)) = (n.aspect, container(size))
            && a.container != have
        {
            out.push(format!("the aspect ratio {} is not the picture's container {}×{} ({have})", a.container, size.0, size.1));
        }
        let res = if size.0 > 2048 { "4K" } else { "2K" };
        if let Some(r) = n.resolution.as_deref()
            && r != res
        {
            out.push(format!("the name claims {r}, the picture is {res} ({}×{})", size.0, size.1));
        }
    }
    if let Some(s) = n.standard
        && s != c.standard
    {
        out.push(format!("the name claims {}, the composition is {}", standard_name(s), standard_name(c.standard)));
    }
    audio(n, f, &mut out);
    languages(n, f, &mut out);
    match (n.package, f.external) {
        (Some(PackageType::Ov), e) if e > 0 => out.push(format!("the name claims OV, the composition plays {e} track files from another package")),
        (Some(PackageType::Vf), 0) => out.push("the name claims VF, the package holds every track file the composition plays".into()),
        _ => {}
    }
    out
}

fn audio(n: &IsdcfName, f: &Facts, out: &mut Vec<String>) {
    if n.audio.is_none() {
        return;
    }
    match (n.audio_channels(), f.channels) {
        (Some(0), Some(_)) => out.push("the name claims MOS (no sound), the composition has sound".into()),
        (Some(c), None) if c > 0 => out.push(format!("the name claims {} sound, the composition has none", n.audio.as_deref().unwrap_or(""))),
        (Some(want), Some(have)) if want > have => {
            out.push(format!("the name claims {} ({want} channels), the sound has {have}", n.audio.as_deref().unwrap_or("")))
        }
        _ => {}
    }
    match (n.immersive(), f.immersive) {
        (true, false) => out.push("the name claims immersive audio, the composition has no immersive audio track file".into()),
        (false, true) => out.push("the composition has immersive audio (IAB), the name does not say so".into()),
        _ => {}
    }
}

fn languages(n: &IsdcfName, f: &Facts, out: &mut Vec<String>) {
    if let (Some(name), Some(cpl)) = (n.audio_language.as_deref().and_then(two_letters), f.sound_language.as_deref().and_then(two_letters))
        && name != cpl
    {
        out.push(format!("the audio language {} is not the sound's Language {cpl}", name.to_ascii_uppercase()));
    }
    match (&n.subtitles, f.subtitles) {
        (Some(Subtitles::Separate(s)), false) => out.push(format!("the name claims {s} subtitles in their own track file, the composition has none")),
        (Some(Subtitles::None), true) => out.push("the composition has subtitles, the name says XX".into()),
        (Some(Subtitles::Separate(s)), true) => {
            if let (Some(name), Some(cpl)) = (two_letters(s), f.subtitle_language.as_deref().and_then(two_letters))
                && name != cpl
            {
                out.push(format!("the subtitle language {s} is not the subtitles' Language {cpl}"));
            }
        }
        _ => {}
    }
    match (n.closed_captions, f.captions) {
        (true, false) => out.push("the name claims CCAP, the composition has no caption track".into()),
        (false, true) => out.push("the composition has captions, the name does not say CCAP".into()),
        _ => {}
    }
}

/// `DCP.NAME.001`; `None` when cancelled.
pub(crate) fn check(cpls: &[(String, Composition)], files: &Files, control: &ValidationControl) -> Option<(Vec<Finding>, String)> {
    let mut out = Vec::new();
    let mut named = 0usize;
    for (cpl, c) in cpls {
        if control.is_cancelled() {
            return None;
        }
        let loc: Location = at(cpl, Some("CompositionPlaylist/ContentTitleText".into()), None, c.id);
        let warn = |m: String| Finding::new(NAME_ISDCF, Status::Warning, m).at(loc.clone());
        let Some(title) = c.content_title.as_deref() else {
            out.push(warn("the composition has no ContentTitleText".into()));
            continue;
        };
        let Some(n) = isdcf::parse(title) else {
            out.push(warn(format!("'{title}' does not follow the ISDCF naming convention (its second field is not a content type)")));
            continue;
        };
        named += 1;
        for m in compare(&n, c, &facts(c, files)) {
            out.push(warn(format!("'{title}': {m}")));
        }
    }
    Some((out, format!("{named} ISDCF names agree with their packages")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_and_languages() {
        assert_eq!(container((1998, 1080)), Some('F'));
        assert_eq!(container((4096, 1716)), Some('S'));
        assert_eq!(container((2048, 1080)), Some('C'));
        assert_eq!(container((1920, 1080)), None);
        assert_eq!(two_letters("en-US").as_deref(), Some("en"));
        assert_eq!(two_letters("QBP"), None);
    }
}
