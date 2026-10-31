use cfa4_bitstream::BitWriter;
use cfa4_time::{TICKS_PER_SECOND, Tick};

use crate::capabilities::Codec;
use crate::h264_index::{NalFormat, index_h264};
use crate::media_index::*;
use crate::mpeg2_index::index_mpeg2;
use crate::timestamp::{MediaTime, Rational};

// ---------------------------------------------------------------------------------------------
// synthetic MPEG-2 headers (ISO/IEC 13818-2 §6.2.2, §6.2.3)

fn sequence_header(width: u32, height: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x0000_01B3, 32);
    w.write_bits(width, 12);
    w.write_bits(height, 12);
    w.write_bits(1, 4); // aspect ratio
    w.write_bits(3, 4); // 25 fps
    w.write_bits(20_000, 18); // bit rate
    w.write_bit(true);
    w.write_bits(112, 10); // vbv buffer
    w.write_bit(false);
    w.write_bit(false);
    w.write_bit(false);
    w.finish()
}

fn gop_header(closed: bool, broken: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0x0000_01B8, 32);
    w.write_bits(0, 25 - 7); // drop frame, hours, minutes, marker
    w.write_bit(true);
    w.write_bits(0, 6);
    w.write_bit(closed);
    w.write_bit(broken);
    w.write_bits(0, 5);
    w.finish()
}

fn picture(tr: u16, ptype: char) -> Vec<u8> {
    let code = match ptype {
        'I' => 1,
        'P' => 2,
        _ => 3,
    };
    let mut w = BitWriter::new();
    w.write_bits(0x0000_0100, 32);
    w.write_bits(u32::from(tr), 10);
    w.write_bits(code, 3);
    w.write_bits(0xFFFF, 16);
    if code >= 2 {
        w.write_bits(0b0001, 4);
    }
    if code == 3 {
        w.write_bits(0b0001, 4);
    }
    w.write_bit(false);
    w.align_zero();
    // a fake slice so the unit has a body
    w.write_bits(0x0000_0101, 32);
    w.write_bits(0xAA, 8);
    w.finish()
}

/// One GOP in decode order: `I2 B0 B1 P5 B3 B4` (M = 3, N = 6), with a sequence header.
fn gop(closed: bool, broken: bool, seq: Option<(u32, u32)>) -> Vec<Vec<u8>> {
    let mut first = Vec::new();
    if let Some((w, h)) = seq {
        first.extend(sequence_header(w, h));
    }
    first.extend(gop_header(closed, broken));
    first.extend(picture(2, 'I'));
    vec![first, picture(0, 'B'), picture(1, 'B'), picture(5, 'P'), picture(3, 'B'), picture(4, 'B')]
}

fn mpeg2(units: &[Vec<u8>]) -> StreamIndex {
    index_mpeg2(0, Rational::MPEG_90K, units.iter().map(|u| Packet { data: u, pts: None, dts: None, keyframe: false })).unwrap()
}

#[test]
fn mpeg2_open_gops() {
    let mut units = gop(false, false, Some((720, 576)));
    units.extend(gop(false, false, Some((720, 576))));
    let ix = mpeg2(&units);
    assert_eq!(ix.codec, Codec::Mpeg2Video);
    let types: String = ix.frames.iter().map(|f| format!("{:?}", f.picture_type)).collect();
    assert_eq!(types, "IBBPBBIBBPBB");
    let ra: Vec<RandomAccessKind> = ix.frames.iter().map(|f| f.random_access).collect();
    assert_eq!(ra[0], RandomAccessKind::OpenGop { broken_link: false });
    assert_eq!(ra[6], RandomAccessKind::OpenGop { broken_link: false });
    assert!(ra.iter().enumerate().all(|(i, r)| (i == 0 || i == 6) == r.is_access_point()));
    // an open GOP is an access point but not a clean cut point
    assert!(ix.clean_access_points().is_empty());
    assert_eq!(ix.access_points(), vec![0, 6]);

    use DependencyStatus::*;
    let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(
        deps,
        vec![
            Independent,
            Undecodable,
            Undecodable,
            WithinGop,
            WithinGop,
            WithinGop,
            Independent,
            CrossesAccessPoint,
            CrossesAccessPoint,
            WithinGop,
            WithinGop,
            WithinGop
        ]
    );
    let refs: Vec<Vec<u64>> = ix.frames.iter().map(|f| f.references.clone().unwrap()).collect();
    assert_eq!(refs[3], vec![0]);
    assert_eq!(refs[4], vec![0, 3]);
    assert_eq!(refs[7], vec![3, 6], "leading B of the second GOP uses the first GOP's last P");
    assert_eq!(refs[9], vec![6]);
    let reference: Vec<bool> = ix.frames.iter().map(|f| f.reference).collect();
    assert_eq!(reference.iter().filter(|r| **r).count(), 4);

    // presentation order from temporal_reference (no timestamps)
    let shown: Vec<u64> = ix.frames.iter().map(|f| f.presentation_index.unwrap()).collect();
    assert_eq!(shown, vec![2, 0, 1, 5, 3, 4, 8, 6, 7, 11, 9, 10]);
    assert_eq!(ix.by_presentation(6).map(|f| f.decode_index), Some(7));
    assert_eq!(ix.gops, vec![GopInfo { start: 0, count: 6, kind: ra[0] }, GopInfo { start: 6, count: 6, kind: ra[6] }]);
    assert!(ix.config_changes.is_empty());
}

#[test]
fn mpeg2_closed_gop_and_broken_link() {
    let mut units = gop(true, false, Some((720, 576)));
    units.extend(gop(true, false, Some((720, 576))));
    units.extend(gop(false, true, Some((720, 576))));
    let ix = mpeg2(&units);
    assert_eq!(ix.frames[0].random_access, RandomAccessKind::ClosedGop);
    assert_eq!(ix.frames[6].random_access, RandomAccessKind::ClosedGop);
    assert_eq!(ix.clean_access_points(), vec![0, 6]);
    // closed GOP: leading B pictures predict backward only
    assert_eq!(ix.frames[1].dependency_status, DependencyStatus::WithinGop);
    assert_eq!(ix.frames[1].references, Some(vec![0]));
    assert_eq!(ix.frames[7].references, Some(vec![6]));
    // broken link: the leading pictures cannot be decoded
    assert_eq!(ix.frames[12].random_access, RandomAccessKind::OpenGop { broken_link: true });
    assert_eq!(ix.frames[13].dependency_status, DependencyStatus::Undecodable);
    assert_eq!(ix.frames[14].dependency_status, DependencyStatus::Undecodable);
    assert_eq!(ix.frames[15].dependency_status, DependencyStatus::WithinGop);
}

#[test]
fn mpeg2_i_picture_without_sequence_header_is_not_an_access_point() {
    let mut units = gop(true, false, Some((720, 576)));
    units.extend(gop(true, false, None));
    let ix = mpeg2(&units);
    assert_eq!(ix.frames[6].picture_type, PictureType::I);
    assert_eq!(ix.frames[6].random_access, RandomAccessKind::IntraOnly);
    assert_eq!(ix.access_points(), vec![0]);
    assert_eq!(ix.gops.len(), 1);
    assert_eq!(ix.gops[0].count, 12);
}

#[test]
fn mpeg2_sequence_header_change_is_a_configuration_change() {
    let mut units = gop(true, false, Some((720, 576)));
    units.extend(gop(true, false, Some((720, 576))));
    units.extend(gop(true, false, Some((1920, 1088))));
    let ix = mpeg2(&units);
    assert_eq!(ix.config_changes, vec![12]);
    assert_eq!(ix.frames[11].config_id, 0);
    assert_eq!(ix.frames[12].config_id, 1);
}

#[test]
fn mpeg2_timestamps_win_over_temporal_reference() {
    let units = gop(true, false, Some((720, 576)));
    let pts = [3000i64, 0, 1500, 7500, 4500, 6000];
    let ix = index_mpeg2(0, Rational::MPEG_90K, units.iter().zip(pts).map(|(u, p)| Packet { data: u, pts: Some(p), dts: None, keyframe: false })).unwrap();
    let shown: Vec<u64> = ix.frames.iter().map(|f| f.presentation_index.unwrap()).collect();
    assert_eq!(shown, vec![2, 0, 1, 5, 3, 4]);
    assert_eq!(ix.duration(), Some(MediaTime::new(9000, Rational::MPEG_90K)));
}

#[test]
fn stream_starting_mid_gop() {
    let units = gop(true, false, Some((720, 576)));
    // drop the I picture: the stream starts with B pictures and a P picture
    let ix = mpeg2(&units[1..]);
    assert!(ix.frames.iter().all(|f| f.dependency_status == DependencyStatus::Undecodable));
    assert!(ix.gops.is_empty());
}

// ---------------------------------------------------------------------------------------------
// H.264 from CFA4's own encoder

fn encode_h264(frames: usize, keyint: u32, bframes: u8) -> Vec<cfa4_h264enc::Packet> {
    use cfa4_h264enc::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
    let (w, h) = (64usize, 48usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.keyint = keyint;
    cfg.bframes = bframes;
    cfg.rate = RateControl::Qp(30);
    cfg.format = PacketFormat::AnnexB;
    cfg.scenecut = false;
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut out = Vec::new();
    for i in 0..frames {
        let y: Vec<u8> = (0..w * h).map(|k| ((k + i * 7) % 251) as u8).collect();
        let c: Vec<u8> = (0..w * h / 4).map(|k| ((k * 3 + i) % 255) as u8).collect();
        out.extend(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }, i as i64).unwrap());
    }
    out.extend(enc.flush());
    out
}

#[test]
fn h264_index_matches_the_encoder() {
    let packets = encode_h264(30, 12, 2);
    let ix = index_h264(
        1,
        Rational::new(1, 25),
        NalFormat::AnnexB,
        &[],
        packets.iter().map(|p| Packet { data: &p.data, pts: Some(p.pts), dts: Some(p.dts), keyframe: p.keyframe }),
    )
    .unwrap();
    assert_eq!(ix.frames.len(), 30);
    for (f, p) in ix.frames.iter().zip(&packets) {
        let want = match p.frame_type {
            cfa4_h264enc::FrameType::Idr | cfa4_h264enc::FrameType::I => PictureType::I,
            cfa4_h264enc::FrameType::P => PictureType::P,
            cfa4_h264enc::FrameType::B => PictureType::B,
        };
        assert_eq!(f.picture_type, want, "decode index {}", f.decode_index);
        assert_eq!(f.random_access == RandomAccessKind::Idr, p.frame_type == cfa4_h264enc::FrameType::Idr);
        assert_eq!(f.presentation_index, Some(p.pts as u64));
        assert_ne!(f.dependency_status, DependencyStatus::Undecodable);
        assert_ne!(f.dependency_status, DependencyStatus::Unknown);
    }
    let idr: Vec<u64> = packets.iter().enumerate().filter(|(_, p)| p.frame_type == cfa4_h264enc::FrameType::Idr).map(|(i, _)| i as u64).collect();
    assert!(!idr.is_empty());
    assert_eq!(ix.clean_access_points(), idr);
    assert_eq!(ix.gops.iter().map(|g| g.start).collect::<Vec<_>>(), idr);
    assert_eq!(ix.gops.iter().map(|g| g.count).sum::<u64>(), 30);
    assert!(ix.frames.iter().any(|f| f.picture_type == PictureType::B));
    assert!(ix.timestamp_problems.is_empty());
    assert!(ix.config_changes.is_empty(), "repeated identical SPS / PPS are no change");
    assert_eq!(ix.codec, Codec::H264);
}

#[test]
fn h264_length_prefixed_framing() {
    let packets = encode_h264(8, 4, 0);
    let to_avcc = |annexb: &[u8]| -> Vec<u8> {
        cfa4_bitstream::annexb_nals(annexb).into_iter().flat_map(|n| (n.len() as u32).to_be_bytes().into_iter().chain(n.iter().copied())).collect()
    };
    let avcc: Vec<Vec<u8>> = packets.iter().map(|p| to_avcc(&p.data)).collect();
    let a = index_h264(
        0,
        Rational::new(1, 25),
        NalFormat::AnnexB,
        &[],
        packets.iter().map(|p| Packet { data: &p.data, pts: Some(p.pts), dts: Some(p.dts), keyframe: p.keyframe }),
    )
    .unwrap();
    let b = index_h264(
        0,
        Rational::new(1, 25),
        NalFormat::LengthPrefixed(4),
        &[],
        avcc.iter().zip(&packets).map(|(d, p)| Packet { data: d, pts: Some(p.pts), dts: Some(p.dts), keyframe: p.keyframe }),
    )
    .unwrap();
    let strip = |ix: &StreamIndex| ix.frames.iter().map(|f| (f.picture_type, f.random_access, f.dependency_status, f.reference)).collect::<Vec<_>>();
    assert_eq!(strip(&a), strip(&b));
}

/// A NAL unit from a header byte and an RBSP (emulation prevention added).
fn nal(header: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1, header];
    out.extend(cfa4_bitstream::escape_rbsp(rbsp));
    out
}

fn slice(header: u8, slice_type: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // first_mb_in_slice
    w.write_ue(slice_type);
    w.write_ue(0); // pps id
    w.write_bits(0xF0F0, 16);
    w.rbsp_trailing();
    nal(header, &w.finish())
}

fn recovery_sei(frame_cnt: u32, exact: bool, broken: bool) -> Vec<u8> {
    let mut p = BitWriter::new();
    p.write_ue(frame_cnt);
    p.write_bit(exact);
    p.write_bit(broken);
    p.write_bits(0, 2);
    p.align_zero();
    let payload = p.finish();
    let mut rbsp = vec![6, payload.len() as u8];
    rbsp.extend(payload);
    rbsp.push(0x80);
    nal(0x06, &rbsp)
}

fn sps(level: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(100, 8);
    w.write_bits(0, 8);
    w.write_bits(u32::from(level), 8);
    w.write_ue(0);
    w.write_bits(0xABCD, 16);
    w.rbsp_trailing();
    nal(0x67, &w.finish())
}

#[test]
fn h264_recovery_points_and_intra_only_pictures() {
    let i_rp = [recovery_sei(0, true, false), slice(0x61, 7)].concat();
    let i_plain = slice(0x61, 7);
    let p = slice(0x61, 5);
    let b = slice(0x01, 6);
    let idr = [sps(40), slice(0x65, 7)].concat();
    // decode order with pts: IDR(0) P(3) B(1) B(2) | I+RP(6) B(4) B(5) P(9) | I(10) P(11)
    let units: Vec<(Vec<u8>, i64)> = vec![
        (idr, 0),
        (p.clone(), 3),
        (b.clone(), 1),
        (b.clone(), 2),
        (i_rp, 6),
        (b.clone(), 4),
        (b.clone(), 5),
        (p.clone(), 9),
        (i_plain, 10),
        (p.clone(), 11),
    ];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|(d, t)| Packet { data: d, pts: Some(*t), dts: None, keyframe: false }))
            .unwrap();
    use DependencyStatus::*;
    let ra: Vec<RandomAccessKind> = ix.frames.iter().map(|f| f.random_access).collect();
    assert_eq!(ra[0], RandomAccessKind::Idr);
    assert_eq!(ra[4], RandomAccessKind::RecoveryPoint { exact_match: true, broken_link: false });
    assert_eq!(ra[8], RandomAccessKind::IntraOnly, "an I picture alone is no cut point");
    assert_eq!(ix.access_points(), vec![0, 4]);
    assert_eq!(ix.clean_access_points(), vec![0]);
    let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(
        deps,
        vec![Independent, WithinGop, WithinGop, WithinGop, Independent, CrossesAccessPoint, CrossesAccessPoint, WithinGop, Independent, WithinGop]
    );
    assert!(!ix.frames[2].reference);
    assert!(ix.frames[1].reference);
    assert_eq!(ix.gops.len(), 2);
    assert_eq!(ix.gops[1].count, 6);
    assert!(ix.frames.iter().all(|f| f.references.is_none()));
}

#[test]
fn h264_recovery_point_with_a_broken_link_or_a_count_is_not_clean() {
    let units: Vec<(Vec<u8>, i64)> = vec![
        ([sps(40), slice(0x65, 7)].concat(), 0),
        ([recovery_sei(0, false, true), slice(0x61, 7)].concat(), 3),
        (slice(0x01, 6), 2),
        ([recovery_sei(4, true, false), slice(0x61, 7)].concat(), 4),
    ];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|(d, t)| Packet { data: d, pts: Some(*t), dts: None, keyframe: false }))
            .unwrap();
    assert_eq!(ix.frames[1].random_access, RandomAccessKind::RecoveryPoint { exact_match: false, broken_link: true });
    assert_eq!(ix.frames[2].dependency_status, DependencyStatus::Undecodable);
    assert_eq!(ix.frames[3].random_access, RandomAccessKind::IntraOnly);
}

fn sps_with_id(level: u8, id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(100, 8);
    w.write_bits(0, 8);
    w.write_bits(u32::from(level), 8);
    w.write_ue(id);
    w.write_bits(0xABCD, 16);
    w.rbsp_trailing();
    nal(0x67, &w.finish())
}

fn pps(id: u32, sps_id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(id);
    w.write_ue(sps_id);
    w.write_bits(0x5A, 8);
    w.rbsp_trailing();
    nal(0x68, &w.finish())
}

fn slice_with_pps(header: u8, slice_type: u32, pps_id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(slice_type);
    w.write_ue(pps_id);
    w.write_bits(0xF0F0, 16);
    w.rbsp_trailing();
    nal(header, &w.finish())
}

#[test]
fn h264_parameter_set_changes() {
    let units =
        [[sps(40), pps(0, 0), slice(0x65, 7)].concat(), slice(0x61, 5), [sps(40), slice(0x65, 7)].concat(), [sps(41), slice(0x65, 7)].concat(), slice(0x61, 5)];
    let first_sps = sps(40);
    let seed = [&first_sps[4..]];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &seed, units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    assert_eq!(ix.config_changes, vec![3]);
    assert_eq!(ix.frames.iter().map(|f| f.config_id).collect::<Vec<_>>(), vec![0, 0, 0, 1, 1]);
    // without timestamps H.264 has no presentation order here
    assert!(ix.frames.iter().all(|f| f.presentation_index.is_none() && f.pts.is_none()));
}

#[test]
fn h264_switching_parameter_set_ids_is_a_configuration_change() {
    // both configurations come with the codec configuration; the slices switch between them
    let (s0, s1, p0, p1) = (sps_with_id(40, 0), sps_with_id(51, 1), pps(0, 0), pps(1, 1));
    let seed = [&s0[4..], &s1[4..], &p0[4..], &p1[4..]];
    let units = [slice_with_pps(0x65, 7, 0), slice_with_pps(0x61, 5, 0), slice_with_pps(0x65, 7, 1), slice_with_pps(0x61, 5, 1), slice_with_pps(0x65, 7, 0)];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &seed, units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    assert_eq!(ix.config_changes, vec![2, 4]);
    assert_eq!(ix.frames.iter().map(|f| f.config_id).collect::<Vec<_>>(), vec![0, 0, 1, 1, 2]);
    // a new SPS id introduced in-band
    let units = [[sps_with_id(40, 0), pps(0, 0), slice_with_pps(0x65, 7, 0)].concat(), [sps_with_id(51, 3), pps(2, 3), slice_with_pps(0x65, 7, 2)].concat()];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    assert_eq!(ix.config_changes, vec![1]);
}

#[test]
fn h264_stream_starting_at_a_recovery_point_has_undecodable_leading_pictures() {
    let units: Vec<(Vec<u8>, i64)> =
        vec![([sps(40), recovery_sei(0, true, false), slice(0x61, 7)].concat(), 2), (slice(0x01, 6), 0), (slice(0x01, 6), 1), (slice(0x61, 5), 5)];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|(d, t)| Packet { data: d, pts: Some(*t), dts: None, keyframe: false }))
            .unwrap();
    use DependencyStatus::*;
    let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(deps, vec![Independent, Undecodable, Undecodable, WithinGop], "their references were never in the source");
}

#[test]
fn h264_recovery_point_without_timestamps_leaves_dependencies_unknown() {
    let units = [[sps(40), slice(0x65, 7)].concat(), [recovery_sei(0, true, false), slice(0x61, 7)].concat(), slice(0x01, 6), slice(0x61, 5)];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    use DependencyStatus::*;
    let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(deps, vec![Independent, Independent, Unknown, Unknown], "not WithinGop: whether they lead the recovery point is unknown");
}

#[test]
fn h264_truncated_idr_is_no_clean_access_point() {
    let units = [vec![0, 0, 1, 0x65], [sps(40), slice(0x65, 7)].concat()];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: true })).unwrap();
    assert_eq!(ix.frames[0].picture_type, PictureType::Unknown);
    assert_eq!(ix.frames[0].random_access, RandomAccessKind::None);
    assert_eq!(ix.clean_access_points(), vec![1]);
}

#[test]
fn h264_parameter_sets_are_bounded() {
    // many distinct, out-of-range and oversized parameter sets: none may grow the index unbounded
    let mut sets: Vec<Vec<u8>> = (0..5000u32).map(|id| sps_with_id(40, id)[4..].to_vec()).collect();
    sets.extend((0..5000u32).map(|id| pps(id, 0)[4..].to_vec()));
    let mut big = sps_with_id(40, 1)[4..].to_vec();
    big.resize(1 << 20, 0x55);
    sets.push(big);
    let refs: Vec<&[u8]> = sets.iter().map(Vec::as_slice).collect();
    let units = [slice_with_pps(0x65, 7, 0), slice_with_pps(0x65, 7, 300), slice_with_pps(0x65, 7, 0)];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &refs, units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    assert_eq!(ix.frames.len(), 3);
    // PPS 300 is out of range and was not kept: that picture has no known configuration, so no
    // change is claimed around it
    assert!(ix.config_changes.is_empty(), "{:?}", ix.config_changes);
}

#[test]
fn h264_invalid_nal_length_sizes_are_refused() {
    // a zero-byte length prefix never advanced the NAL splitter
    let data = [0u8, 0, 0, 1, 0x65];
    for n in [0u8, 5, 8, 255] {
        let r = index_h264(0, Rational::MPEG_90K, NalFormat::LengthPrefixed(n), &[], [Packet { data: &data, pts: None, dts: None, keyframe: false }]);
        assert_eq!(r, Err(IndexError::InvalidNalLengthSize(n)));
    }
}

#[test]
fn timestamp_problems_are_reported() {
    let p = slice(0x61, 5);
    let i = [sps(40), slice(0x65, 7)].concat();
    let units = [(&i, 0, 0), (&p, 1, 1), (&p, 1, 1), (&p, 3, 0)];
    let ix = index_h264(
        0,
        Rational::new(1, 25),
        NalFormat::AnnexB,
        &[],
        units.iter().map(|(d, t, dts)| Packet { data: d, pts: Some(*t), dts: Some(*dts), keyframe: false }),
    )
    .unwrap();
    assert_eq!(ix.timestamp_problems, vec![2, 3]);
}

#[test]
fn mpeg2_gop_flags_count_without_a_repeated_sequence_header() {
    use DependencyStatus::*;
    for (closed, broken, expected) in [(false, true, [Undecodable, Undecodable]), (true, false, [WithinGop, WithinGop])] {
        let mut units = gop(true, false, Some((720, 576)));
        units.extend(gop(closed, broken, None));
        let ix = mpeg2(&units);
        assert_eq!(ix.frames[6].random_access, RandomAccessKind::IntraOnly);
        let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
        assert_eq!(deps[7..9], expected, "closed {closed} broken {broken}");
        let refs: Vec<Vec<u64>> = ix.frames.iter().map(|f| f.references.clone().unwrap()).collect();
        assert_eq!(refs[7], vec![6], "no reference into the previous GOP");
    }
}

// ---------------------------------------------------------------------------------------------
// shared model

#[test]
fn hostile_packets_do_not_panic() {
    let mut seed = 0x1234_5678u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut good = gop(false, false, Some((720, 576))).concat();
    good.extend(encode_h264(4, 2, 1).into_iter().flat_map(|p| p.data));
    for round in 0..300 {
        let mut d = good.clone();
        match round % 3 {
            0 => d.truncate(rnd() as usize % d.len()),
            1 => {
                for _ in 0..8 {
                    let k = rnd() as usize % d.len();
                    d[k] ^= 1 << (rnd() % 8);
                }
            }
            _ => d = (0..rnd() % 512).map(|_| rnd() as u8).collect(),
        }
        let chunks: Vec<&[u8]> = d.chunks(1 + rnd() as usize % 97).collect();
        let _ = index_mpeg2(0, Rational::MPEG_90K, chunks.iter().map(|c| Packet { data: c, pts: None, dts: None, keyframe: false }));
        let _ = index_h264(
            0,
            Rational::MPEG_90K,
            NalFormat::AnnexB,
            &[],
            chunks.iter().map(|c| Packet { data: c, pts: None, dts: Some(i64::MIN), keyframe: false }),
        );
        for n in [1u8, 2, 4] {
            let _ = index_h264(
                0,
                Rational::MPEG_90K,
                NalFormat::LengthPrefixed(n),
                &chunks,
                chunks.iter().map(|c| Packet { data: c, pts: Some(i64::MAX), dts: None, keyframe: true }),
            );
        }
    }
}

#[test]
fn invalid_timebases_are_refused() {
    let units = gop(true, false, Some((720, 576)));
    for tb in [Rational::new(0, 1), Rational::new(1, 0), Rational::new(-1, 25)] {
        assert_eq!(index_mpeg2(0, tb, units.iter().map(|u| Packet { data: u, pts: None, dts: None, keyframe: false })), Err(IndexError::InvalidTimebase(tb)));
        assert_eq!(index_h264(0, tb, NalFormat::AnnexB, &[], std::iter::empty()), Err(IndexError::InvalidTimebase(tb)));
    }
}

#[test]
fn media_time_converts_exactly_to_ticks() {
    assert_eq!(MediaTime::new(90_000, Rational::MPEG_90K).to_tick(), Some(Tick(TICKS_PER_SECOND)));
    assert_eq!(MediaTime::new(1001, Rational::new(1, 30_000)).to_tick(), Some(Tick(TICKS_PER_SECOND / 30_000 * 1001)));
    assert_eq!(MediaTime::new(-1, Rational::MPEG_90K).to_tick(), Some(Tick(-TICKS_PER_SECOND / 90_000)));
    assert_eq!(MediaTime::new(i64::MAX, Rational::new(1, 1)).to_tick(), None);
    assert_eq!(MediaTime::new(1, Rational::new(1, 0)).to_tick(), None);
}

#[test]
fn cached_index_validity() {
    let fp = SourceFingerprint { size: 1000, modified_ns: Some(5), content_id: Some("abc".into()) };
    let ix = MediaIndex::new("clip-1", fp.clone(), vec![mpeg2(&gop(true, false, Some((720, 576))))]);
    assert_eq!(ix.index_version, INDEX_VERSION);
    assert_eq!(ix.check_valid(&fp), Ok(()));
    assert_eq!(ix.check_valid(&SourceFingerprint { size: 999, ..fp.clone() }), Err(Staleness::SizeChanged { indexed: 1000, now: 999 }));
    assert_eq!(ix.check_valid(&SourceFingerprint { modified_ns: Some(6), ..fp.clone() }), Err(Staleness::Modified));
    assert_eq!(ix.check_valid(&SourceFingerprint { modified_ns: None, ..fp.clone() }), Err(Staleness::Modified));
    assert_eq!(ix.check_valid(&SourceFingerprint { content_id: Some("abd".into()), ..fp.clone() }), Err(Staleness::ContentChanged));
    assert_eq!(ix.check_valid(&SourceFingerprint { content_id: None, ..fp.clone() }), Err(Staleness::ContentUnknown));
    let mut old = ix.clone();
    old.index_version = 0;
    assert_eq!(old.check_valid(&fp), Err(Staleness::OtherVersion { found: 0 }));
    // the index is plain data: it round-trips through JSON (a cache file)
    let json = serde_json::to_string(&ix).unwrap();
    let back: MediaIndex = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ix);
    assert!(json.contains("\"random_access\":\"closed-gop\""));
}

#[test]
fn nal_splitting_matches_the_bitstream_crate() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut samples: Vec<Vec<u8>> = encode_h264(4, 2, 1).into_iter().map(|p| p.data).collect();
    samples.push(vec![0, 0, 1, 0, 0, 0, 1, 0x65, 0, 0, 0, 1, 0, 0, 1]);
    samples.push(vec![0; 64]);
    for _ in 0..200 {
        // few distinct byte values, so start codes and zero runs are common
        samples.push((0..rnd() % 300).map(|_| [0u8, 0, 0, 1, 0x65, 0xFF][rnd() as usize % 6]).collect());
    }
    for d in &samples {
        assert_eq!(crate::h264_index::split_nal_units(d, NalFormat::AnnexB), cfa4_bitstream::annexb_nals(d));
        for n in [1u8, 2, 4] {
            let expected: Vec<&[u8]> =
                cfa4_bitstream::length_prefixed_nals(d, usize::from(n)).unwrap_or_default().into_iter().filter(|nal| !nal.is_empty()).collect();
            assert_eq!(crate::h264_index::split_nal_units(d, NalFormat::LengthPrefixed(n)), expected);
        }
    }
}

#[test]
fn h264_packets_of_empty_nal_units_do_not_amplify_memory() {
    // Each zero byte is an empty length-prefixed NAL unit, each `00 00 01` an empty Annex B one:
    // the splitter must walk them without a descriptor per unit.
    let zeros = vec![0u8; 16 << 20];
    assert!(crate::h264_index::split_nal_units(&zeros, NalFormat::LengthPrefixed(1)).is_empty());
    let start_codes: Vec<u8> = [0u8, 0, 1].repeat(4 << 20);
    assert!(crate::h264_index::split_nal_units(&start_codes, NalFormat::AnnexB).is_empty());
    for (data, format) in [(&zeros, NalFormat::LengthPrefixed(1)), (&start_codes, NalFormat::AnnexB)] {
        let ix = index_h264(0, Rational::MPEG_90K, format, &[], [Packet { data, pts: None, dts: None, keyframe: false }]);
        assert!(ix.is_ok());
    }
}

#[test]
fn h264_pictures_after_an_unsignalled_intra_picture_are_not_called_undecodable() {
    let units: Vec<Vec<u8>> = vec![
        [sps(40), slice(0x41, 5)].concat(), // P before any intra picture: no references
        slice(0x61, 7),                     // I without IDR or recovery point SEI
        slice(0x41, 5),
        slice(0x41, 5),
    ];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|d| Packet { data: d, pts: None, dts: None, keyframe: false })).unwrap();
    let status: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(status, [DependencyStatus::Undecodable, DependencyStatus::Independent, DependencyStatus::Unknown, DependencyStatus::Unknown]);
    assert_eq!(ix.frames[1].random_access, RandomAccessKind::IntraOnly);
    assert!(ix.clean_access_points().is_empty());
}

#[test]
fn mpeg2_header_scan_matches_the_decoder_crate() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut samples: Vec<Vec<u8>> = gop(false, false, Some((720, 576)));
    samples.extend(gop(true, true, None));
    for _ in 0..300 {
        samples.push((0..rnd() % 200).map(|_| [0u8, 0, 0, 1, 0xB3, 0xB8, 0xB5, 0x10, 0xFF][rnd() as usize % 9]).collect());
    }
    for d in &samples {
        let info = cfa4_mpeg2v::scan_access_unit(d);
        let (sequence_header, gop, picture) = crate::mpeg2_index::scan_summary(d);
        assert_eq!(sequence_header, info.sequence_header);
        assert_eq!(gop.map(|g| (g.closed_gop, g.broken_link)), info.gop.map(|g| (g.closed_gop, g.broken_link)));
        assert_eq!(picture.map(|p| (p.coding_type, p.temporal_reference)), info.pictures.first().map(|p| (p.0.coding_type, p.0.temporal_reference)));
    }
}

#[test]
fn mpeg2_packets_of_start_codes_do_not_amplify_memory() {
    // one user-data start code per four bytes: the scanner keeps nothing per code
    let data: Vec<u8> = [0u8, 0, 1, 0xB2].repeat(4 << 20);
    assert_eq!(crate::mpeg2_index::scan_summary(&data), (false, None, None));
    let ix = index_mpeg2(0, Rational::MPEG_90K, [Packet { data: &data, pts: None, dts: None, keyframe: false }]).unwrap();
    assert_eq!(ix.frames[0].picture_type, PictureType::Unknown);
}

#[test]
fn h264_an_unsignalled_intra_picture_counts_as_history_for_a_later_recovery_point() {
    let units: Vec<(Vec<u8>, i64)> = vec![
        ([sps(40), slice(0x61, 7)].concat(), 0), // I without IDR or recovery point SEI
        (slice(0x41, 5), 1),
        ([recovery_sei(0, true, false), slice(0x61, 7)].concat(), 3),
        (slice(0x01, 6), 2), // leading B picture of the recovery point
    ];
    let ix =
        index_h264(0, Rational::new(1, 25), NalFormat::AnnexB, &[], units.iter().map(|(d, t)| Packet { data: d, pts: Some(*t), dts: None, keyframe: false }))
            .unwrap();
    assert_eq!(ix.frames[2].random_access, RandomAccessKind::RecoveryPoint { exact_match: true, broken_link: false });
    assert_eq!(ix.frames[3].dependency_status, DependencyStatus::CrossesAccessPoint);
}
