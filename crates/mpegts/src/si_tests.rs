//! Tests of the transport stream identifiers and the DVB SDT (CFA4-SCE-001 P2a.1).

use crate::si::{MAX_NAME_BYTES, NULL_PID, SDT_PID, ServiceType, SiError, TsParams, sdt_packets, sdt_section};
use crate::ts::crc32_mpeg;

fn dvb() -> TsParams {
    TsParams { sdt: true, ..TsParams::default() }
}

#[test]
fn default_matches_what_the_muxer_writes_today() {
    let p = TsParams::default();
    assert_eq!((p.transport_stream_id, p.program_number, p.pmt_pid, p.first_es_pid), (1, 1, 0x1000, 0x100));
    assert_eq!(p.pcr_pid, None);
    assert!(!p.sdt);
    assert_eq!(p.tables_version, 0);
    assert_eq!(p.validate(16), Ok(()));
}

#[test]
fn sdt_section_matches_en_300_468_byte_for_byte() {
    let sec = sdt_section(&dvb(), 2).unwrap();
    #[rustfmt::skip]
    let want: &[u8] = &[
        0x42, 0xF0, 0x1E,       // table_id, section_syntax_indicator, section_length 30
        0x00, 0x01,             // transport_stream_id
        0xC1, 0x00, 0x00,       // version 0, current; section 0 of 0
        0xFF, 0x01,             // original_network_id
        0xFF,                   // reserved_future_use
        0x00, 0x01,             // service_id = program_number
        0xFC,                   // no EIT schedule, no EIT p/f
        0x80, 0x0D,             // running, not scrambled, descriptors_loop_length 13
        0x48, 0x0B, 0x01,       // service_descriptor, 11 bytes, digital television
        0x04, b'C', b'F', b'A', b'4',
        0x04, b'C', b'F', b'A', b'4',
    ];
    assert_eq!(&sec[..sec.len() - 4], want);
    // a section with its CRC divides to zero (H.222.0 annex A)
    assert_eq!(crc32_mpeg(&sec), 0);
    assert_eq!(usize::from(sec[1] & 0x0F) << 8 | usize::from(sec[2]), sec.len() - 3);
}

#[test]
fn sdt_carries_ids_version_type_and_utf8_names() {
    let p = TsParams {
        transport_stream_id: 0x1234,
        program_number: 0x0042,
        original_network_id: 0x2001,
        tables_version: 7,
        service_type: ServiceType::DigitalRadio,
        service_name: "Műsor".into(),
        service_provider: "CFA4 stúdió".into(),
        ..dvb()
    };
    let sec = sdt_section(&p, 1).unwrap();
    assert_eq!(&sec[3..5], &[0x12, 0x34]);
    assert_eq!(sec[5], 0xC1 | 7 << 1);
    assert_eq!(&sec[8..10], &[0x20, 0x01]);
    assert_eq!(&sec[11..13], &[0x00, 0x42]);
    assert_eq!(sec[18], 0x02);
    let provider = "CFA4 stúdió".as_bytes();
    assert_eq!(usize::from(sec[19]), provider.len() + 1);
    assert_eq!(sec[20], 0x15, "UTF-8 selector");
    assert_eq!(&sec[21..21 + provider.len()], provider);
    assert_eq!(crc32_mpeg(&sec), 0);
}

#[test]
fn packets_carry_the_section_on_pid_0x11_with_continuity() {
    let p = TsParams { service_name: "N".repeat(150), service_provider: "P".repeat(100), ..dvb() };
    let sec = sdt_section(&p, 1).unwrap();
    let mut cc = 14;
    let pk = sdt_packets(&p, 1, &mut cc).unwrap();
    assert_eq!(pk.len(), 2, "{} section bytes need two packets", sec.len());
    assert_eq!(cc, 0);
    for (k, packet) in pk.iter().enumerate() {
        assert_eq!(packet[0], 0x47);
        assert_eq!(u16::from(packet[1] & 0x1F) << 8 | u16::from(packet[2]), SDT_PID);
        assert_eq!(packet[1] & 0x40 != 0, k == 0, "payload_unit_start only on the first");
        assert_eq!(packet[3], 0x10 | ((14 + k as u8) & 0x0F));
    }
    let mut payload: Vec<u8> = pk.iter().flat_map(|p| p[4..].iter().copied()).collect();
    assert_eq!(payload.remove(0), 0, "pointer_field");
    assert_eq!(&payload[..sec.len()], &sec[..]);
    assert!(payload[sec.len()..].iter().all(|&b| b == 0xFF));
}

#[test]
fn invalid_parameters_are_refused() {
    let bad = |p: TsParams, streams: usize| p.validate(streams).unwrap_err();
    assert_eq!(bad(dvb(), 0), SiError::StreamCount(0));
    assert_eq!(bad(dvb(), 17), SiError::StreamCount(17));
    assert_eq!(bad(TsParams { program_number: 0, ..dvb() }, 1), SiError::ProgramNumberZero);
    assert_eq!(bad(TsParams { tables_version: 32, ..dvb() }, 1), SiError::Version(32));
    for pid in [0x0000, SDT_PID, 0x001F, 0x1FFB, NULL_PID] {
        assert!(matches!(bad(TsParams { pmt_pid: pid, ..dvb() }, 1), SiError::PidOutOfRange { what: "PMT", .. }), "PMT 0x{pid:04X}");
        assert!(matches!(bad(TsParams { pcr_pid: Some(pid), ..dvb() }, 1), SiError::PidOutOfRange { what: "PCR", .. }), "PCR 0x{pid:04X}");
    }
    // the elementary streams must all fit below the reserved PIDs
    assert!(matches!(bad(TsParams { first_es_pid: 0x1FF8, ..dvb() }, 4), SiError::PidOutOfRange { .. }));
    assert_eq!(TsParams { first_es_pid: 0x1FF8, ..dvb() }.validate(3), Ok(()));
    assert!(matches!(bad(TsParams { pmt_pid: 0x0102, ..dvb() }, 3), SiError::PidCollision { what: "PMT", .. }));
    assert!(matches!(bad(TsParams { pcr_pid: Some(0x1000), ..dvb() }, 1), SiError::PidCollision { what: "PCR", .. }));
    // a PCR on an elementary stream's PID, or on a PID of its own, is fine
    assert_eq!(TsParams { pcr_pid: Some(0x0101), ..dvb() }.validate(2), Ok(()));
    assert_eq!(TsParams { pcr_pid: Some(0x01FF), ..dvb() }.validate(2), Ok(()));
    assert_eq!(bad(TsParams { service_name: "a\nb".into(), ..dvb() }, 1), SiError::NameControl { what: "service name" });
    let long = TsParams { service_name: "n".repeat(MAX_NAME_BYTES - 3), service_provider: "pppp".into(), ..dvb() };
    assert_eq!(bad(long, 1), SiError::NamesTooLong(MAX_NAME_BYTES + 1));
    let full = TsParams { service_name: "n".repeat(MAX_NAME_BYTES - 4), service_provider: "pppp".into(), ..dvb() };
    let sec = sdt_section(&full, 1).unwrap();
    assert_eq!(sec[17], 0xFF, "the descriptor fills its 8-bit length");
    assert!(sdt_section(&TsParams { program_number: 0, ..dvb() }, 1).is_err());
}

#[test]
fn no_parameters_make_it_panic() {
    // a deterministic sweep over the edges of every field
    let pids = [0u16, 0x1F, 0x20, 0x100, 0x1000, 0x1FFA, 0x1FFB, 0x1FFF, u16::MAX];
    let names = ["", "x", "é", "\u{7f}", "\u{0}"];
    let mut seen = 0;
    for &pmt in &pids {
        for &es in &pids {
            for pcr in [None, Some(0x20), Some(es), Some(u16::MAX)] {
                for (k, name) in names.iter().enumerate() {
                    for streams in [0usize, 1, 16, 17, usize::MAX] {
                        let p = TsParams {
                            pmt_pid: pmt,
                            first_es_pid: es,
                            pcr_pid: pcr,
                            service_name: name.repeat(k * 60),
                            tables_version: (k * 8) as u8,
                            ..dvb()
                        };
                        let mut cc = 0;
                        if sdt_packets(&p, streams, &mut cc).is_ok() {
                            seen += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(seen > 0);
}

/// PAT, PMT (one H.264 stream on 0x100) and SDT, ten times, then null packets: ffprobe reads the
/// service name and provider back from our SDT.
#[test]
fn ffprobe_reads_the_service_from_our_sdt() {
    let Some(ffprobe) = cfa4_testkit::ffprobe_or_skip("SDT service name") else { return };
    let p = TsParams { service_name: "Próba adás".into(), service_provider: "CFA4".into(), ..dvb() };
    let section_packet = |pid: u16, cc: u8, mut sec: Vec<u8>| {
        let len = sec.len() - 3 + 4;
        sec[1] = 0xB0 | (len >> 8) as u8;
        sec[2] = len as u8;
        let crc = crc32_mpeg(&sec);
        sec.extend_from_slice(&crc.to_be_bytes());
        let mut pk = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10 | cc, 0];
        pk.extend_from_slice(&sec);
        pk.resize(188, 0xFF);
        pk
    };
    let pat = vec![0x00, 0, 0, 0x00, 0x01, 0xC1, 0, 0, 0x00, 0x01, 0xF0, 0x00];
    let pmt = vec![0x02, 0, 0, 0x00, 0x01, 0xC1, 0, 0, 0xE1, 0x00, 0xF0, 0, 0x1B, 0xE1, 0x00, 0xF0, 0];
    let mut ts = Vec::new();
    let mut sdt_cc = 0;
    for k in 0..10u8 {
        ts.extend(section_packet(0, k & 15, pat.clone()));
        ts.extend(section_packet(0x1000, k & 15, pmt.clone()));
        for pk in sdt_packets(&p, 1, &mut sdt_cc).unwrap() {
            ts.extend_from_slice(&pk);
        }
        for _ in 0..20 {
            let mut null = [0xFFu8; 188];
            null[..4].copy_from_slice(&[0x47, 0x1F, 0xFF, 0x10]);
            ts.extend_from_slice(&null);
        }
    }
    let dir = std::env::temp_dir().join(format!("cfa4-sdt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sdt.ts");
    std::fs::write(&path, &ts).unwrap();
    let out = std::process::Command::new(ffprobe)
        .args(["-v", "error", "-show_entries", "program_tags=service_name,service_provider", "-of", "default=nw=1"])
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("TAG:service_name=Próba adás"), "ffprobe: {text}");
    assert!(text.contains("TAG:service_provider=CFA4"), "ffprobe: {text}");
}
