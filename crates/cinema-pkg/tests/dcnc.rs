//! ISDCF Digital Cinema Naming Convention: the registry pages' own examples, round trips and
//! hostile input.

use std::panic::catch_unwind;

use cfa4_cinema_pkg::isdcf::dcnc::{Name, check};
use cfa4_cinema_pkg::isdcf::{Lookup, Registry};

fn codes(name: &str) -> Vec<&'static str> {
    check(name, Registry::builtin().unwrap()).iter().map(|f| f.code).collect()
}

#[test]
fn clean_names_have_no_findings() {
    for name in [
        "ThankYouService_FTR-4_S_EN-XX-CCAP_US-R-INT-TD_51-HI-VI_2K_DI_20170928_DLB_SMPTE_OV",
        "MovieTitle_FTR-1_F_EN-EN-CCAP_US-G_51_2K_DI_20130515_DLB_IOP_VF",
        "Demo_TST-1_F_EN-XX_INT-TL_51_2K_NULL_20261031_NUL_SMPTE_OV",
        "Film_FTR-2-Temp-3D-48_S-266_EN-fr_US-13_IAB-HI-VI-DBOX_4K_DI_20261115_DLB_SMPTE-3D_VF-1",
    ] {
        let f = check(name, Registry::builtin().unwrap());
        assert!(f.is_empty(), "{name}: {f:?}");
    }
}

#[test]
fn round_trip() {
    for name in [
        "Film_FTR-2-Temp-3D-48_S-266_EN-fr-OCAP_US-13_IAB-HI-VI-DBOX_4K_DI_20261115_DLB_SMPTE-3D_VF-1",
        "AlitaBattleAngel_FTR-3D-6FL-XBrite_S_EN-EN-CCAP_US_ATMOS-DBOX-HI-VI_2K_DI_20190126_DLB_SMPTE-3D_OV",
        "A_TLR_F_XX_US_MOS_2K_NULL_20260101_NUL_SMPTE_OV",
    ] {
        assert_eq!(Name::parse(name).unwrap().to_string(), name);
    }
    let n = Name::parse("Film_FTR-2-Temp-3D_S-266_EN-FR-CCAP_US-13_51-HI_4K_DI_20261115_DLB_SMPTE-3D_VF-1").unwrap();
    assert_eq!((n.content.kind.as_str(), n.content.version, n.content.modifiers.len()), ("FTR", Some(2), 2));
    assert_eq!(n.aspect.image.as_deref(), Some("266"));
    assert_eq!((n.language.subtitles.as_slice(), n.language.captions.as_deref()), (&["FR".to_string()][..], Some("CCAP")));
    assert!(n.standard.stereo);
    assert_eq!(n.package.version, Some(1));
}

#[test]
fn each_rule_fires() {
    let ok = "Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV";
    assert!(codes(ok).is_empty(), "{:?}", codes(ok));
    let cases = [
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE", "DCNC.FIELDS"),
        ("Film__F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.EMPTY"),
        ("My-Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.TITLE"),
        ("Film_ZZZ-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.CONTENT_TYPE"),
        ("Film_FTR-12345_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.CONTENT_TYPE"),
        ("Film_FTR-1-3D-Temp_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE-3D_OV", "DCNC.MODIFIER_ORDER"),
        ("Film_FTR-1_X_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.ASPECT"),
        ("Film_TLR-1_F-239_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.ASPECT"),
        ("Film_FTR-1_F_QQQQ-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.LANGUAGE"),
        ("Film_FTR-1_F_EN_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.LANGUAGE"),
        ("Film_FTR-1_F_ENG-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.OBSOLETE"),
        ("Film_FTR-1_F_EN-XX_XX_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.TERRITORY"),
        ("Film_FTR-1_F_EN-XX_US-XX_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.TERRITORY"),
        ("Film_FTR-1_F_EN-XX_us-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.TERRITORY"),
        ("Film_FTR-1_F_EN-XX_US-13_52_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.AUDIO"),
        ("Film_FTR-1_F_EN-XX_US-13_ATMOS_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.OBSOLETE"),
        ("Film_FTR-1_F_EN-XX_US-13_51_8K_DI_20261115_DLB_SMPTE_OV", "DCNC.RESOLUTION"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_ZZZZQ_20261115_DLB_SMPTE_OV", "DCNC.STUDIO"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20260230_DLB_SMPTE_OV", "DCNC.DATE"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_2026-11-15_DLB_SMPTE_OV", "DCNC.DATE"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_ZZZZQ_SMPTE_OV", "DCNC.FACILITY"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_ST_OV", "DCNC.STANDARD"),
        ("Film_FTR-1-3D_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV", "DCNC.3D"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE-3D_OV", "DCNC.3D"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_XV", "DCNC.PACKAGE"),
        ("Film_FTR-1_F_EN-XX_US-13_51_2K_DI_20261115_DLB_SMPTE_OV-1", "DCNC.PACKAGE"),
    ];
    for (name, code) in cases {
        let got = codes(name);
        assert!(got.contains(&code), "{name}: want {code}, got {got:?}");
    }
}

#[test]
fn registry_lookups() {
    let reg = Registry::builtin().unwrap();
    assert!(matches!(Registry::lookup(&reg.languages, "en"), Lookup::Current(e) if e.code == "EN"));
    assert!(matches!(Registry::lookup(&reg.languages, "FRE"), Lookup::Obsolete { use_instead: Some(c), .. } if c == "FR"));
    assert!(matches!(Registry::lookup(&reg.territories, "INT"), Lookup::Current(_)));
    assert_eq!(Registry::lookup(&reg.audio, "52"), Lookup::Unknown);
    assert!(reg.studios.len() > 500 && reg.facilities.len() > 1000);
}

#[test]
fn hostile_names_never_panic() {
    let reg = Registry::builtin().unwrap();
    let base = "Film_FTR-2-Temp-3D-48_S-266_EN-fr-OCAP_US-13_IAB-HI-VI-DBOX_4K_DI_20261115_DLB_SMPTE-3D_VF-1";
    let mut inputs: Vec<String> = (0..=base.len()).filter(|&i| base.is_char_boundary(i)).map(|i| base[..i].to_string()).collect();
    inputs.extend(["", "_", "___________", "-_-_-_-_-_-_-_-_-_-_-_-", "Á_ő_€_-_--_x_y_z_w_v_u_t", &"_".repeat(10_000), &"-".repeat(10_000)].map(String::from));
    inputs.push(format!("{}_FTR-{}_F_EN-XX_US_51_2K_DI_20261115_DLB_SMPTE_OV", "x".repeat(100_000), "9".repeat(40)));
    for s in &inputs {
        assert!(catch_unwind(|| check(s, reg)).is_ok(), "{s:.80}");
    }
}
