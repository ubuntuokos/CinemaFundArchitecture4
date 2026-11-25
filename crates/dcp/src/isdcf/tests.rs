use super::*;

#[test]
fn a_full_name() {
    let n = parse("MyFilm_FTR-2-25-3D-Temp_F-133_EN-fr-CCAP_US-13_71-HI-VI-ATMOS_4K_ST_20261114_FAC_SMPTE-3D_VF").unwrap();
    assert_eq!((n.title.as_str(), n.content_type.as_str(), n.content_kind()), ("MyFilm", "FTR", Some("feature")));
    assert_eq!((n.version, n.frame_rate, n.stereoscopic), (Some(2), Some(25), Some(true)));
    assert_eq!(n.modifiers, ["Temp"]);
    assert_eq!(n.aspect, Some(Aspect { container: 'F', image: Some(133) }));
    assert_eq!(n.audio_language.as_deref(), Some("EN"));
    assert_eq!(n.subtitles, Some(Subtitles::BurnedIn("fr".into())));
    assert!(n.closed_captions);
    assert_eq!(n.territory.as_deref(), Some("US-13"));
    assert_eq!((n.audio.as_deref(), n.audio_channels(), n.immersive()), (Some("71"), Some(8), true));
    assert_eq!(n.audio_modifiers, ["HI", "VI", "ATMOS"]);
    assert_eq!(n.resolution.as_deref(), Some("4K"));
    assert_eq!((n.studio.as_deref(), n.date.as_deref(), n.facility.as_deref()), (Some("ST"), Some("20261114"), Some("FAC")));
    assert!(!n.bad_date);
    assert_eq!((n.standard, n.package), (Some(Standard::Smpte), Some(PackageType::Vf)));
    assert!(n.unknown.is_empty() && n.missing.is_empty(), "{n:?}");
}

#[test]
fn the_cfa4_export_name() {
    // cfa4-export's dcp_isdcf_name: no territory, studio or facility
    let n = parse("RoundTrip_FTR-1_F_EN-XX_51_2K_20261114_SMPTE_OV").unwrap();
    assert_eq!((n.version, n.frame_rate, n.stereoscopic), (Some(1), None, None));
    assert_eq!((n.audio_language.as_deref(), n.subtitles.clone()), (Some("EN"), Some(Subtitles::None)));
    assert_eq!((n.territory.clone(), n.studio.clone(), n.facility.clone()), (None, None, None));
    assert_eq!((n.audio_channels(), n.immersive()), (Some(6), false));
    assert_eq!((n.standard, n.package), (Some(Standard::Smpte), Some(PackageType::Ov)));
    assert!(n.unknown.is_empty() && n.missing.is_empty(), "{n:?}");
    let n = parse("RoundTrip_TLR-1_S_XX-DE_IAB_4K_20261114_IOP_OV").unwrap();
    assert_eq!((n.content_kind(), n.audio_language.clone()), (Some("trailer"), None));
    assert_eq!(n.subtitles, Some(Subtitles::Separate("DE".into())));
    assert_eq!((n.audio_channels(), n.immersive(), n.standard), (None, true, Some(Standard::Interop)));
}

#[test]
fn version_and_frame_rate() {
    assert_eq!(parse("A_FTR-25_F_EN-XX_51_2K_20261114_SMPTE_OV").map(|n| (n.version, n.frame_rate)), Some((None, Some(25))));
    assert_eq!(parse("A_FTR-3-48_F_EN-XX_51_2K_20261114_SMPTE_OV").map(|n| (n.version, n.frame_rate)), Some((Some(3), Some(48))));
    assert_eq!(parse("A_TST-2D_F_EN-XX_MOS_2K_20261114_SMPTE_OV").map(|n| (n.stereoscopic, n.audio_channels())), Some((Some(false), Some(0))));
}

#[test]
fn missing_unknown_and_bad_fields() {
    let n = parse("A_FTR-1_F_EN-XX_2K_20261399_WHAT?_SMPTE").unwrap();
    assert!(n.bad_date);
    assert_eq!(n.missing, ["audio", "package type"]);
    assert_eq!(n.facility.as_deref(), Some("WHAT?"));
    let n = parse("A_FTR_EN-XX_F_51_2K_20261114_SMPTE_OV").unwrap();
    // the aspect ratio after the language is out of order
    assert_eq!(n.unknown, ["F"]);
    assert_eq!(n.missing, ["aspect ratio"]);
}

#[test]
fn not_a_name() {
    assert_eq!(parse("Round Trip"), None);
    assert_eq!(parse("My_Film_FTR_F"), None);
    assert_eq!(parse("A_XYZ-1_F_EN-XX_51_2K_20261114_SMPTE_OV"), None);
    assert_eq!(parse(""), None);
}
