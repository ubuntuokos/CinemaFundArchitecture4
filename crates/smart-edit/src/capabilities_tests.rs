use crate::capabilities::*;

const STRICT: Policy = Policy { allow_experimental: false };
const LAX: Policy = Policy { allow_experimental: true };

fn h264_high_420_8() -> StreamFormat {
    StreamFormat { profile: "high".into(), chroma: "4:2:0".into(), bit_depth: 8 }
}

#[test]
fn builtin_table_has_only_valid_keys_and_no_duplicates() {
    let r = Registry::native();
    let mut n = 0;
    for e in r.entries() {
        assert!(e.key.is_valid(), "{:?}", e.key);
        n += 1;
    }
    // every built-in row survived insertion: none was rejected or replaced by a later duplicate
    assert_eq!(crate::capabilities::native_entries().len(), n);
    assert!(n > 50, "{n}");
}

#[test]
fn invalid_keys_are_rejected() {
    let mut r = Registry::empty();
    let bad = Key { operation: Operation::Decode, codec: Codec::H264, container: Some(Container::Mp4) };
    let e = Entry { key: bad, backend: Backend::Native, level: SupportLevel::Supported, evidence: String::new(), limits: None };
    assert_eq!(r.insert(e), Err(RegistryError::InvalidKey(bad)));
    let missing = Key { operation: Operation::Mux, codec: Codec::H264, container: None };
    let p = Probe { key: missing, backend: Backend::Platform, outcome: ProbeOutcome::Available { detail: "x".into() }, limits: None };
    assert_eq!(r.apply_probes([p]), Err(RegistryError::InvalidKey(missing)));
    let d = r.decide(bad, STRICT);
    assert!(!d.allowed);
    assert_eq!(d.level, SupportLevel::Unsupported);
}

#[test]
fn unknown_capabilities_are_unsupported_with_a_reason() {
    let r = Registry::empty();
    let d = r.decide(Key::codec(Operation::Decode, Codec::Av1), STRICT);
    assert!(!d.allowed);
    assert_eq!(d.backend, None);
    assert_eq!(d.level, SupportLevel::Unsupported);
    assert_eq!(d.reasons, vec!["decoding AV1: no backend provides it".to_string()]);
}

#[test]
fn baseline_decode_and_demux() {
    let r = Registry::native();
    for codec in Codec::VIDEO {
        let d = r.decide(Key::codec(Operation::Decode, codec), STRICT);
        assert!(d.allowed, "{codec:?}");
        assert_eq!(d.backend, Some(Backend::Native));
    }
    assert!(r.decide(Key::in_container(Operation::Demux, Codec::Mpeg2Video, Container::MpegPs), STRICT).allowed);
    assert!(r.decide(Key::in_container(Operation::Demux, Codec::H264, Container::Mp4), STRICT).allowed);
    // implemented but without an oracle test: only for a lax policy
    let mkv = Key::in_container(Operation::Demux, Codec::H264, Container::Matroska);
    assert!(!r.decide(mkv, STRICT).allowed);
    assert_eq!(r.decide(mkv, STRICT).level, SupportLevel::Experimental);
    assert!(r.decide(mkv, LAX).allowed);
}

#[test]
fn no_smart_cut_is_allowed_before_it_is_implemented() {
    // WP-03 gate: an unproven cut must never count as lossless
    let r = Registry::native();
    for codec in Codec::VIDEO {
        for source in Container::ALL {
            for output in Container::ALL {
                for policy in [STRICT, LAX] {
                    let d = r.smart_cut(codec, source, output, policy);
                    assert!(!d.allowed, "{codec:?} {source:?} → {output:?}");
                    assert!(!d.blockers().is_empty());
                }
            }
        }
    }
    let d = r.smart_cut(Codec::H264, Container::Mp4, Container::Mp4, STRICT);
    let blocked: Vec<Operation> = d.steps.iter().filter(|s| !s.allowed).map(|s| s.key.operation).collect();
    assert_eq!(blocked, vec![Operation::GopIndex, Operation::PacketCopy]);
}

#[test]
fn smart_cut_needs_every_step() {
    let mut r = Registry::native();
    let index = Key::codec(Operation::GopIndex, Codec::H264);
    let copy = Key::in_container(Operation::PacketCopy, Codec::H264, Container::Mp4);
    let ok = |key| Entry { key, backend: Backend::Native, level: SupportLevel::Supported, evidence: "test".into(), limits: None };
    r.insert(ok(index)).unwrap();
    assert!(!r.smart_cut(Codec::H264, Container::Mp4, Container::Mp4, STRICT).allowed);
    r.insert(ok(copy)).unwrap();
    assert!(r.smart_cut(Codec::H264, Container::Mp4, Container::Mp4, STRICT).allowed);
    // MPEG-2 in a program stream cannot be written back: no muxer
    let mut m = Registry::native();
    m.insert(ok(Key::codec(Operation::GopIndex, Codec::Mpeg2Video))).unwrap();
    m.insert(ok(Key::in_container(Operation::PacketCopy, Codec::Mpeg2Video, Container::MpegPs))).unwrap();
    let d = m.smart_cut(Codec::Mpeg2Video, Container::MpegPs, Container::MpegPs, STRICT);
    assert!(!d.allowed);
    assert!(d.blockers().iter().any(|b| b.contains("no muxer")), "{:?}", d.blockers());
}

#[test]
fn smart_render_checks_encoder_limits() {
    let mut r = Registry::native();
    for key in [
        Key::codec(Operation::GopIndex, Codec::H264),
        Key::in_container(Operation::PacketCopy, Codec::H264, Container::Mp4),
        Key::in_container(Operation::BoundaryReencode, Codec::H264, Container::Mp4),
    ] {
        r.insert(Entry { key, backend: Backend::Native, level: SupportLevel::Supported, evidence: "test".into(), limits: None }).unwrap();
    }
    assert!(r.smart_render(Codec::H264, Container::Mp4, Container::Mp4, &h264_high_420_8(), STRICT).allowed);
    let ten_bit_422 = StreamFormat { profile: "high-422".into(), chroma: "4:2:2".into(), bit_depth: 10 };
    let d = r.smart_render(Codec::H264, Container::Mp4, Container::Mp4, &ten_bit_422, STRICT);
    assert!(!d.allowed);
    assert_eq!(d.conditions.len(), 3, "{:?}", d.conditions);
    // only the encoder is refused: no allowed encoder fits the stream
    let refused: Vec<Operation> = d.steps.iter().filter(|s| !s.allowed).map(|s| s.key.operation).collect();
    assert_eq!(refused, vec![Operation::Encode]);
}

#[test]
fn smart_render_without_known_limits_is_refused() {
    let r = Registry::native();
    let d = r.smart_render(Codec::Mpeg2Video, Container::MpegPs, Container::MpegPs, &h264_high_420_8(), LAX);
    assert!(!d.allowed);
    assert!(d.conditions.iter().any(|c| c.contains("cannot be shown")));
    assert!(d.blockers().iter().any(|b| b.contains("no MPEG-2 video encoder")));
}

#[test]
fn smart_render_picks_the_encoder_that_fits_the_stream() {
    let mut r = Registry::native();
    let hevc = Key::codec(Operation::Encode, Codec::Hevc);
    for key in [
        Key::codec(Operation::GopIndex, Codec::Hevc),
        Key::in_container(Operation::PacketCopy, Codec::Hevc, Container::Mp4),
        Key::in_container(Operation::BoundaryReencode, Codec::Hevc, Container::Mp4),
    ] {
        r.insert(Entry { key, backend: Backend::Native, level: SupportLevel::Supported, evidence: "test".into(), limits: None }).unwrap();
    }
    let lim = |depth: u8, profile: &str| Some(EncoderLimits { profiles: vec![profile.into()], chroma: vec!["4:2:0".into()], bit_depths: vec![depth] });
    r.insert(Entry { key: hevc, backend: Backend::Platform, level: SupportLevel::Supported, evidence: "test".into(), limits: lim(8, "main") }).unwrap();
    r.insert(Entry { key: hevc, backend: Backend::Gstreamer, level: SupportLevel::Supported, evidence: "test".into(), limits: lim(10, "main10") }).unwrap();
    let main10 = StreamFormat { profile: "main10".into(), chroma: "4:2:0".into(), bit_depth: 10 };
    let d = r.smart_render(Codec::Hevc, Container::Mp4, Container::Mp4, &main10, STRICT);
    assert!(d.allowed, "{:?}", d.blockers());
    let enc = d.steps.iter().find(|s| s.key == hevc).unwrap();
    assert_eq!(enc.backend, Some(Backend::Gstreamer));
    let main = StreamFormat { profile: "main".into(), chroma: "4:2:0".into(), bit_depth: 8 };
    let d = r.smart_render(Codec::Hevc, Container::Mp4, Container::Mp4, &main, STRICT);
    assert_eq!(d.steps.iter().find(|s| s.key == hevc).unwrap().backend, Some(Backend::Platform));
    // nothing fits 4:2:2: refused, with each backend's reason
    let yuv422 = StreamFormat { profile: "main422-10".into(), chroma: "4:2:2".into(), bit_depth: 10 };
    let d = r.smart_render(Codec::Hevc, Container::Mp4, Container::Mp4, &yuv422, STRICT);
    assert!(!d.allowed);
    assert!(!d.steps.iter().find(|s| s.key == hevc).unwrap().allowed);
    assert!(
        d.conditions.iter().any(|c| c.starts_with("Platform encoder")) && d.conditions.iter().any(|c| c.starts_with("Gstreamer encoder")),
        "{:?}",
        d.conditions
    );
}

#[test]
fn baseline_av1_has_no_encoder_and_mpeg2_demuxes_from_iso_and_matroska() {
    let r = Registry::native();
    let d = r.decide(Key::codec(Operation::Encode, Codec::Av1), LAX);
    assert!(!d.allowed);
    assert_eq!(d.level, SupportLevel::Unsupported);
    for c in [Container::Mp4, Container::Mov, Container::Matroska, Container::MpegPs, Container::MpegTs, Container::Mxf] {
        assert!(r.decide(Key::in_container(Operation::Demux, Codec::Mpeg2Video, c), STRICT).allowed, "{c:?}");
    }
}

#[test]
fn hardware_encoders_wait_for_a_probe() {
    let mut r = Registry::native();
    let hevc = Key::codec(Operation::Encode, Codec::Hevc);
    let d = r.decide(hevc, STRICT);
    assert!(!d.allowed);
    assert_eq!(d.level, SupportLevel::NeedsProbe);
    assert_eq!(d.reasons.len(), 2, "native and platform are both listed: {:?}", d.reasons);

    let limits = EncoderLimits { profiles: vec!["main".into()], chroma: vec!["4:2:0".into()], bit_depths: vec![8] };
    r.apply_probes([Probe {
        key: hevc,
        backend: Backend::Platform,
        outcome: ProbeOutcome::Available { detail: "NVENC".into() },
        limits: Some(limits.clone()),
    }])
    .unwrap();
    let d = r.decide(hevc, STRICT);
    assert!(d.allowed);
    assert_eq!(d.backend, Some(Backend::Platform));
    assert_eq!(r.encoder_limits(Codec::Hevc, STRICT), Some(&limits));
}

#[test]
fn probes_are_order_independent_and_unavailable_wins() {
    let key = Key::codec(Operation::Encode, Codec::Av1);
    let yes = Probe { key, backend: Backend::Platform, outcome: ProbeOutcome::Available { detail: "found".into() }, limits: None };
    let no = Probe { key, backend: Backend::Platform, outcome: ProbeOutcome::Unavailable { reason: "driver too old".into() }, limits: None };
    let other = Probe {
        key: Key::codec(Operation::Encode, Codec::Hevc),
        backend: Backend::Platform,
        outcome: ProbeOutcome::Available { detail: "found".into() },
        limits: None,
    };

    let mut a = Registry::native();
    a.apply_probes([yes.clone(), no.clone(), other.clone()]).unwrap();
    let mut b = Registry::native();
    b.apply_probes([other.clone(), no.clone(), yes.clone(), yes.clone()]).unwrap();
    let mut c = Registry::native();
    c.apply_probes([no.clone()]).unwrap();
    c.apply_probes([yes.clone(), other.clone()]).unwrap();

    assert_eq!(a, b);
    assert_eq!(a, c);
    let d = a.decide(key, STRICT);
    assert!(!d.allowed);
    assert!(d.reasons.iter().any(|r| r.contains("driver too old")), "{:?}", d.reasons);
    assert!(a.decide(Key::codec(Operation::Encode, Codec::Hevc), STRICT).allowed);
    assert_eq!(serde_json::to_string(&a.report()).unwrap(), serde_json::to_string(&b.report()).unwrap());
}

#[test]
fn the_native_backend_is_preferred() {
    let mut r = Registry::empty();
    let key = Key::codec(Operation::Encode, Codec::H264);
    for backend in [Backend::Ffmpeg, Backend::Platform, Backend::Native] {
        r.insert(Entry { key, backend, level: SupportLevel::Supported, evidence: format!("{backend:?}"), limits: None }).unwrap();
    }
    let d = r.decide(key, STRICT);
    assert_eq!(d.backend, Some(Backend::Native));
    let order: Vec<&str> = d.reasons.iter().map(|s| s.split(':').next().unwrap_or("")).collect();
    assert_eq!(order, vec!["Native", "Platform", "Ffmpeg"]);
}

#[test]
fn no_ffmpeg_backend_is_registered() {
    // Smart Cut runs on CFA4's own code; ffmpeg is its validation oracle (owner decision 2026-10-27)
    assert!(Registry::native().entries().all(|e| e.backend != Backend::Ffmpeg));
}

#[test]
fn report_round_trips_and_is_stable() {
    let r = Registry::native();
    let json = serde_json::to_string_pretty(&r.report()).unwrap();
    assert_eq!(json, serde_json::to_string_pretty(&Registry::native().report()).unwrap());
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r.report());
    assert_eq!(back.registry_version, REGISTRY_VERSION);
    // the wire names are part of the control-API contract
    assert!(json.contains("\"operation\": \"packet-copy\""));
    assert!(json.contains("\"codec\": \"mpeg2-video\""));
    assert!(json.contains("\"container\": \"mpeg-ps\""));
    assert!(json.contains("\"level\": \"needs-probe\""));
}

#[test]
fn hostile_report_json_does_not_panic() {
    for text in ["", "{}", "[]", "{\"registry_version\": -1}", "{\"registry_version\":1,\"entries\":[{\"key\":{}}],\"probes\":[]}", "\u{0}"] {
        let r: Result<Report, _> = serde_json::from_str(text);
        assert!(r.is_err(), "{text:?}");
    }
}
