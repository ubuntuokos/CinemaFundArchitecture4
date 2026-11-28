//! Tests of the MPEG-TS export profile (CFA4-SCE-001 P2a.1).

use crate::mpegts_profile::{MpegTsProfile, PROFILE_VERSION, ProfileError, TsMuxMode, TsOutput, TsSystem, TsTimestamps};
use cfa4_mpegts::{ServiceType, SiError, TsParams};

#[test]
fn json_round_trips() {
    let p = MpegTsProfile {
        output: TsOutput::M2ts192,
        mux: TsMuxMode::Cbr { mux_rate: 20_000_000 },
        system: TsSystem::SystemB,
        service_name: "Műsor".into(),
        service_id: 7,
        pcr_pid: Some(0x1FF),
        timestamps: TsTimestamps::Preserve,
        tables_version: 3,
        ..MpegTsProfile::default()
    };
    let json = p.to_json().unwrap();
    assert!(json.contains(r#""mux":{"mode":"cbr","muxRate":20000000}"#), "{json}");
    assert!(json.contains(r#""output":"m2ts192""#), "{json}");
    assert_eq!(MpegTsProfile::from_json(&json).unwrap(), p);
}

#[test]
fn old_and_new_profiles_load() {
    // an empty or version-less profile takes every default
    assert_eq!(MpegTsProfile::from_json("{}").unwrap(), MpegTsProfile::default());
    let partial = MpegTsProfile::from_json(r#"{"serviceName":"News","psiPeriodMs":200}"#).unwrap();
    assert_eq!(partial, MpegTsProfile { service_name: "News".into(), psi_period_ms: 200, ..MpegTsProfile::default() });
    // a field a later CFA4 adds is ignored
    let newer = MpegTsProfile::from_json(r#"{"version":1,"futureOption":true}"#).unwrap();
    assert_eq!(newer, MpegTsProfile::default());
    // a newer format version is refused by validation, not misread
    let v2 = MpegTsProfile::from_json(r#"{"version":2}"#).unwrap();
    assert_eq!(v2.validate(1), Err(ProfileError::UnknownVersion(2)));
    assert_eq!(MpegTsProfile { version: 0, ..MpegTsProfile::default() }.effective().version, PROFILE_VERSION);
}

#[test]
fn defaults_give_todays_muxer_parameters_with_an_sdt() {
    let p = MpegTsProfile::default();
    assert_eq!(p.validate(2), Ok(()));
    let params = p.to_params(2, true).unwrap();
    assert_eq!(params, TsParams { sdt: true, ..TsParams::default() });
    assert_eq!(p.to_params(1, false).unwrap().service_type, ServiceType::DigitalRadio);
    let atsc = MpegTsProfile { system: TsSystem::SystemB, ..MpegTsProfile::default() };
    assert!(!atsc.to_params(1, true).unwrap().sdt);
}

#[test]
fn ids_and_pids_reach_the_muxer_parameters() {
    let p = MpegTsProfile {
        service_id: 0x22,
        transport_stream_id: 0x33,
        original_network_id: 0x44,
        pmt_pid: 0x0500,
        first_es_pid: 0x0600,
        pcr_pid: Some(0x0700),
        service_provider: "Prov".into(),
        tables_version: 9,
        ..MpegTsProfile::default()
    };
    let t = p.to_params(3, true).unwrap();
    assert_eq!((t.program_number, t.transport_stream_id, t.original_network_id), (0x22, 0x33, 0x44));
    assert_eq!((t.pmt_pid, t.first_es_pid, t.pcr_pid), (0x0500, 0x0600, Some(0x0700)));
    assert_eq!((t.service_provider.as_str(), t.tables_version), ("Prov", 9));
}

#[test]
fn out_of_range_values_are_refused() {
    let d = MpegTsProfile::default;
    let field = |p: MpegTsProfile| match p.validate(1) {
        Err(ProfileError::OutOfRange { field, .. }) => field,
        other => panic!("{other:?}"),
    };
    assert_eq!(field(MpegTsProfile { pcr_period_ms: 19, ..d() }), "pcrPeriodMs");
    assert_eq!(field(MpegTsProfile { pcr_period_ms: 101, ..d() }), "pcrPeriodMs");
    assert_eq!(field(MpegTsProfile { psi_period_ms: 501, ..d() }), "psiPeriodMs");
    assert_eq!(field(MpegTsProfile { sdt_period_ms: 2001, ..d() }), "sdtPeriodMs");
    assert_eq!(field(MpegTsProfile { mux: TsMuxMode::Cbr { mux_rate: 999_999 }, ..d() }), "muxRate");
    assert_eq!(field(MpegTsProfile { mux: TsMuxMode::Cbr { mux_rate: 200_000_001 }, ..d() }), "muxRate");
    assert_eq!(MpegTsProfile { mux: TsMuxMode::Cbr { mux_rate: 200_000_000 }, ..d() }.validate(1), Ok(()));
    assert_eq!(MpegTsProfile { service_id: 0, ..d() }.validate(1), Err(ProfileError::Params(SiError::ProgramNumberZero)));
    assert!(matches!(MpegTsProfile { pmt_pid: 0x11, ..d() }.validate(1), Err(ProfileError::Params(SiError::PidOutOfRange { .. }))));
    assert!(matches!(MpegTsProfile { pmt_pid: 0x100, ..d() }.validate(1), Err(ProfileError::Params(SiError::PidCollision { .. }))));
    // the message names the field and the range
    let msg = MpegTsProfile { pcr_period_ms: 5, ..d() }.validate(1).unwrap_err().to_string();
    assert_eq!(msg, "pcrPeriodMs is 5; allowed: 20-100");
}

#[test]
fn cbr_and_kept_timestamps_are_refused_until_the_muxer_has_them() {
    let cbr = MpegTsProfile { mux: TsMuxMode::Cbr { mux_rate: 10_000_000 }, ..MpegTsProfile::default() };
    assert_eq!(cbr.validate(1), Ok(()));
    assert!(matches!(cbr.to_params(1, true), Err(ProfileError::NotYetSupported(_))));
    let keep = MpegTsProfile { timestamps: TsTimestamps::Preserve, ..MpegTsProfile::default() };
    assert!(matches!(keep.to_params(1, true), Err(ProfileError::NotYetSupported(_))));
}

#[test]
fn garbage_json_never_panics() {
    let inputs = [
        "",
        "null",
        "[]",
        r#"{"version":-1}"#,
        r#"{"pmtPid":70000}"#,
        r#"{"mux":{"mode":"cbr"}}"#,
        r#"{"mux":{"mode":"xyz","muxRate":1}}"#,
        r#"{"output":"ts204"}"#,
        r#"{"serviceName":7}"#,
        r#"{"tablesVersion":255}"#,
        "{\"serviceName\":\"\\u0000\"}",
    ];
    for s in inputs {
        if let Ok(p) = MpegTsProfile::from_json(s) {
            let _ = p.validate(1);
            let _ = p.to_params(1, true);
        }
    }
    assert!(MpegTsProfile::from_json(r#"{"tablesVersion":255}"#).unwrap().validate(1).is_err());
    assert!(MpegTsProfile::from_json("{\"serviceName\":\"\\u0000\"}").unwrap().validate(1).is_err());
}
