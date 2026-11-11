//! WP-07: the HEVC index finds the IRAP access points, tells clean CRA / BLA pictures from those
//! with RASL leading pictures, and tracks VPS / SPS / PPS changes.

use cfa4_bitstream::BitWriter;

use crate::capabilities::Codec;
use crate::copy_planner::{CutMode, SmartCutRequest, plan_smart_cut};
use crate::h264_index::NalFormat;
use crate::hevc_index::index_hevc;
use crate::media_index::{DependencyStatus, IndexError, MediaIndex, Packet, PictureType, RandomAccessKind, SourceFingerprint, StreamIndex};
use crate::timestamp::Rational;

/// Annex B access units of our HEVC encoder (64×48, IDR every `keyint`, P pictures).
fn encoded(frames: usize, keyint: u32) -> Vec<(Vec<u8>, i64)> {
    use cfa4_hevc::encode::{Encoder, EncoderConfig, YuvFrame};
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = keyint;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut out = Vec::new();
    for i in 0..frames {
        let y: Vec<u8> = (0..64 * 48).map(|k| ((k + i * 5) % 251) as u8).collect();
        let c = vec![128u8; 32 * 24];
        for p in enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap() {
            out.push((p.data, p.pts));
        }
    }
    for p in enc.flush().unwrap() {
        out.push((p.data, p.pts));
    }
    out
}

fn index(units: &[(Vec<u8>, i64)], format: NalFormat, ps: &[&[u8]]) -> StreamIndex {
    let packets = units.iter().enumerate().map(|(i, (d, pts))| Packet { data: d, pts: Some(*pts), dts: Some(i as i64), keyframe: false });
    index_hevc(1, Rational::new(1, 25), format, ps, packets).unwrap()
}

#[test]
fn encoder_output_has_an_idr_every_gop() {
    let units = encoded(30, 10);
    let ix = index(&units, NalFormat::AnnexB, &[]);
    assert_eq!(ix.codec, Codec::Hevc);
    assert_eq!(ix.frames.len(), 30);
    for f in &ix.frames {
        let idr = f.decode_index % 10 == 0;
        assert_eq!(f.random_access == RandomAccessKind::Idr, idr, "{f:?}");
        assert_eq!(f.picture_type, if idr { PictureType::I } else { PictureType::P }, "{f:?}");
        let dep = if idr { DependencyStatus::Independent } else { DependencyStatus::WithinGop };
        assert_eq!(f.dependency_status, dep, "{f:?}");
        assert_eq!(f.config_id, 0);
    }
    assert_eq!(ix.gops.len(), 3);
    assert!(ix.config_changes.is_empty());
    assert_eq!(ix.frames.iter().map(|f| f.presentation_index).collect::<Vec<_>>(), (0..30).map(Some).collect::<Vec<_>>());
}

#[test]
fn length_prefixed_samples_with_out_of_band_parameter_sets() {
    use cfa4_hevc::encode::{Encoder, EncoderConfig, PacketFormat, YuvFrame};
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 5;
    cfg.format = PacketFormat::LengthPrefixed;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut units = Vec::new();
    for i in 0..10 {
        let y = vec![(i * 20) as u8; 64 * 48];
        let c = vec![128u8; 32 * 24];
        for p in enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i).unwrap() {
            units.push((p.data, p.pts));
        }
    }
    let ps = enc.parameter_sets();
    let ps: Vec<&[u8]> = ps.iter().map(Vec::as_slice).collect();
    let ix = index(&units, NalFormat::LengthPrefixed(4), &ps);
    let idrs: Vec<u64> = ix.frames.iter().filter(|f| f.random_access == RandomAccessKind::Idr).map(|f| f.decode_index).collect();
    assert_eq!(idrs, [0, 5]);
    assert!(ix.frames.iter().all(|f| f.picture_type != PictureType::Unknown), "the PPS gives the slice type");
    // without the parameter sets the slice type stays unknown, but the IDRs are still found
    let blind = index(&units, NalFormat::LengthPrefixed(4), &[]);
    assert_eq!(blind.frames[0].random_access, RandomAccessKind::Idr);
    assert_eq!(blind.frames[1].picture_type, PictureType::Unknown);
    let bad = index_hevc(1, Rational::new(1, 25), NalFormat::LengthPrefixed(5), &[], std::iter::empty());
    assert_eq!(bad.unwrap_err(), IndexError::InvalidNalLengthSize(5));
}

/// A NAL unit: header of type `ty` (layer 0, temporal id 0) and `body`.
fn nal(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![ty << 1, 1];
    out.extend_from_slice(body);
    out
}

/// A PPS with id 0 for SPS 0, no extra slice header bits.
fn pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(0);
    w.write_bits(0, 2);
    w.write_bits(0, 3);
    w.write_bits(0, 8);
    w.rbsp_trailing();
    nal(34, &w.finish())
}

/// The first slice segment of a picture: NAL type `ty`, slice type 0 B, 1 P, 2 I.
fn slice(ty: u8, slice_type: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bit(true);
    if (16..=23).contains(&ty) {
        w.write_bit(false);
    }
    w.write_ue(0);
    w.write_ue(slice_type);
    w.write_bits(0, 16);
    w.rbsp_trailing();
    nal(ty, &w.finish())
}

/// Annex B access units of (NAL type, slice type, pts); the PPS rides with the first.
fn stream(pictures: &[(u8, u32, i64)]) -> StreamIndex {
    let units: Vec<(Vec<u8>, i64)> = pictures
        .iter()
        .enumerate()
        .map(|(i, &(ty, st, pts))| {
            let mut au = Vec::new();
            if i == 0 {
                au.extend_from_slice(&[0, 0, 0, 1]);
                au.extend(pps());
            }
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend(slice(ty, st));
            (au, pts)
        })
        .collect();
    index(&units, NalFormat::AnnexB, &[])
}

const IDR: u8 = 19;
const CRA: u8 = 21;
const BLA: u8 = 16;
const TRAIL_R: u8 = 1;
const RASL_N: u8 = 8;
const RADL_R: u8 = 7;

#[test]
fn cra_pictures_with_rasl_leading_pictures_are_not_clean() {
    // IDR P P | CRA (shown at 5) RASL RASL (shown at 3, 4) P | CRA P (no leading pictures)
    let ix =
        stream(&[(IDR, 2, 0), (TRAIL_R, 1, 1), (TRAIL_R, 1, 2), (CRA, 2, 5), (RASL_N, 0, 3), (RASL_N, 0, 4), (TRAIL_R, 1, 6), (CRA, 2, 7), (TRAIL_R, 1, 8)]);
    let kinds: Vec<RandomAccessKind> = ix.frames.iter().map(|f| f.random_access).collect();
    assert_eq!(kinds[3], RandomAccessKind::Cra { rasl: true });
    assert_eq!(kinds[7], RandomAccessKind::Cra { rasl: false });
    assert!(kinds[0].is_clean() && kinds[7].is_clean() && !kinds[3].is_clean() && kinds[3].is_access_point());
    let deps: Vec<DependencyStatus> = ix.frames.iter().map(|f| f.dependency_status).collect();
    assert_eq!(deps[4], DependencyStatus::CrossesAccessPoint);
    assert_eq!(deps[5], DependencyStatus::CrossesAccessPoint);
    assert_eq!(deps[6], DependencyStatus::WithinGop);
    assert_eq!(ix.frames[4].picture_type, PictureType::B);
    assert!(!ix.frames[4].reference, "RASL_N is a sub-layer non-reference picture");
}

#[test]
fn leading_pictures_without_history_are_undecodable() {
    // the stream starts at a CRA: its RASL pictures have no references
    let ix = stream(&[(CRA, 2, 2), (RASL_N, 0, 0), (RASL_N, 0, 1), (TRAIL_R, 1, 3)]);
    assert_eq!(ix.frames[1].dependency_status, DependencyStatus::Undecodable);
    // a BLA drops its RASL pictures, whatever came before
    let ix = stream(&[(IDR, 2, 0), (TRAIL_R, 1, 1), (BLA, 2, 4), (RASL_N, 0, 2), (RASL_N, 0, 3)]);
    assert_eq!(ix.frames[2].random_access, RandomAccessKind::Bla { rasl: true });
    assert_eq!(ix.frames[3].dependency_status, DependencyStatus::Undecodable);
    // RADL pictures decode from their IRAP picture
    let ix = stream(&[(IDR, 2, 1), (RADL_R, 0, 0), (TRAIL_R, 1, 2)]);
    assert_eq!(ix.frames[1].dependency_status, DependencyStatus::WithinGop);
    assert!(ix.frames[0].random_access.is_clean());
    // a trailing picture before any IRAP picture has no references
    let ix = stream(&[(TRAIL_R, 1, 0), (IDR, 2, 1)]);
    assert_eq!(ix.frames[0].dependency_status, DependencyStatus::Undecodable);
}

#[test]
fn the_copy_planner_cuts_hevc_on_idr_pictures() {
    let units = encoded(30, 10);
    let ix = index(&units, NalFormat::AnnexB, &[]);
    let media = MediaIndex::new("a.hevc", SourceFingerprint { size: 1, modified_ns: None, content_id: None }, vec![ix.clone()]);
    let tick = |f: i64| crate::timestamp::MediaTime::new(f, Rational::new(1, 25)).to_tick().unwrap();
    let req = SmartCutRequest {
        stream_id: 1,
        in_point: tick(10),
        out_point: tick(20),
        mode: CutMode::GopBoundary,
        guarantee: crate::plan::CopyGuarantee::BitExact,
        source_container: crate::capabilities::Container::Mp4,
        output_container: crate::capabilities::Container::Mp4,
        format: None,
    };
    let p = plan_smart_cut(&crate::capabilities::Registry::native(), &media, &req, crate::capabilities::Policy { allow_experimental: false }).unwrap();
    assert!(p.plan.is_lossless(), "{:?}", p.plan.segments);
    assert_eq!(p.plan.copied_frames(), 10);
    assert_eq!(p.plan.codec, Codec::Hevc);
}
