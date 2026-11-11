//! WP-07: the AV1 index finds the temporal units that start with a shown key frame, keeps hidden
//! (forward) key frames and intra-only frames out of the cut points, and tracks sequence header
//! changes.

use crate::av1_index::index_av1;
use crate::capabilities::Codec;
use crate::media_index::{DependencyStatus, IndexError, Packet, PictureType, RandomAccessKind, StreamIndex};
use crate::timestamp::Rational;

/// An OBU of `kind` with a size field.
fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 128);
    let mut out = vec![(kind << 3) | 0x02, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// A sequence header payload: profile 0, `reduced_still_picture_header` as given; `tag` makes
/// two headers differ.
fn sequence_header(reduced: bool, tag: u8) -> Vec<u8> {
    vec![if reduced { 0x08 } else { 0x00 }, tag, 0x80]
}

/// The start of a frame header: `show_existing_frame` 0, `frame_type`, `show_frame`.
fn frame(frame_type: u8, shown: bool) -> Vec<u8> {
    obu(6, &[(frame_type << 5) | (u8::from(shown) << 4), 0xaa, 0x55])
}

fn show_existing() -> Vec<u8> {
    obu(3, &[0x80])
}

const TD: [u8; 2] = [0x12, 0x00];

fn index(units: &[Vec<u8>], config: &[u8]) -> StreamIndex {
    let packets = units.iter().enumerate().map(|(i, d)| Packet { data: d, pts: Some(i as i64), dts: Some(i as i64), keyframe: false });
    index_av1(1, Rational::new(1, 25), config, packets).unwrap()
}

fn unit(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = TD.to_vec();
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

#[test]
fn shown_key_frames_are_clean_access_points() {
    let seq = obu(1, &sequence_header(false, 1));
    // key | inter | hidden alt-ref + inter | show existing | key | intra-only
    let units = [
        unit(&[seq.clone(), frame(0, true)]),
        unit(&[frame(1, true)]),
        unit(&[frame(1, false), frame(1, true)]),
        unit(&[show_existing()]),
        unit(&[frame(0, true)]),
        unit(&[frame(2, true)]),
    ];
    let ix = index(&units, &[]);
    assert_eq!(ix.codec, Codec::Av1);
    let kinds: Vec<RandomAccessKind> = ix.frames.iter().map(|f| f.random_access).collect();
    use RandomAccessKind::{IntraOnly, KeyFrame, None};
    assert_eq!(kinds, [KeyFrame, None, None, None, KeyFrame, IntraOnly]);
    assert!(KeyFrame.is_clean() && !IntraOnly.is_access_point());
    let types: Vec<PictureType> = ix.frames.iter().map(|f| f.picture_type).collect();
    use PictureType::{I, P, Unknown};
    assert_eq!(types, [I, P, P, Unknown, I, I]);
    assert!(ix.frames.iter().skip(1).all(|f| f.dependency_status != DependencyStatus::Independent || f.random_access == KeyFrame));
    assert_eq!(ix.frames[3].dependency_status, DependencyStatus::WithinGop);
    assert!(!ix.frames[3].reference, "show_existing_frame decodes nothing");
    assert_eq!(ix.gops.len(), 2);
    assert!(ix.config_changes.is_empty());
    assert_eq!(ix.frames.iter().map(|f| f.presentation_index).collect::<Vec<_>>(), (0..6).map(Some).collect::<Vec<_>>());
}

#[test]
fn hidden_key_frames_and_missing_sequence_headers_are_not_cut_points() {
    let seq = obu(1, &sequence_header(false, 1));
    // a forward key frame: hidden, shown later
    let ix = index(&[unit(&[frame(0, true)]), unit(&[frame(0, false), frame(1, true)]), unit(&[show_existing()])], &seq);
    assert_eq!(ix.frames[0].random_access, RandomAccessKind::KeyFrame, "the sequence header comes from av1C");
    assert_eq!(ix.frames[1].random_access, RandomAccessKind::IntraOnly);
    assert_eq!(ix.frames[1].dependency_status, DependencyStatus::WithinGop);
    // without a sequence header a key frame cannot start decoding
    let blind = index(&[unit(&[frame(0, true)]), unit(&[frame(1, true)])], &[]);
    assert_eq!(blind.frames[0].random_access, RandomAccessKind::IntraOnly);
    assert_eq!(blind.frames[1].dependency_status, DependencyStatus::Undecodable);
    // inter frames before the first key frame have no references
    let late = index(&[unit(&[seq.clone(), frame(1, true)]), unit(&[frame(0, true)])], &[]);
    assert_eq!(late.frames[0].dependency_status, DependencyStatus::Undecodable);
    assert_eq!(late.frames[1].dependency_status, DependencyStatus::Independent);
    // reduced still-picture headers: every frame is a shown key frame
    let still = obu(1, &sequence_header(true, 1));
    let ix = index(&[unit(&[still, obu(6, &[0x00])])], &[]);
    assert_eq!(ix.frames[0].random_access, RandomAccessKind::KeyFrame);
}

#[test]
fn sequence_header_changes_and_hostile_units() {
    let a = obu(1, &sequence_header(false, 1));
    let b = obu(1, &sequence_header(false, 2));
    let ix = index(&[unit(&[a.clone(), frame(0, true)]), unit(&[a, frame(1, true)]), unit(&[b, frame(0, true)]), unit(&[frame(1, true)])], &[]);
    assert_eq!(ix.config_changes, [2]);
    assert_eq!(ix.frames.iter().map(|f| f.config_id).collect::<Vec<_>>(), [0, 0, 1, 1]);
    // truncated, oversized and forbidden-bit OBUs are skipped without a panic
    let hostile = [vec![0x32, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], vec![0x32, 0x10, 0x00], vec![0x80, 0x00], vec![0x34], Vec::new()];
    let ix = index(&hostile, &[]);
    assert!(ix.frames.iter().all(|f| f.dependency_status == DependencyStatus::Unknown));
    assert!(matches!(index_av1(1, Rational::new(0, 0), &[], std::iter::empty()), Err(IndexError::InvalidTimebase(_))));
}

#[test]
fn the_cfa4_encoder_output_has_a_key_frame_every_gop() {
    use cfa4_av1::encode::{Encoder, EncoderConfig, YuvFrame};
    let mut cfg = EncoderConfig::new(64, 48, 25, 1);
    cfg.keyint = 10;
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut units = Vec::new();
    for i in 0..30usize {
        let y: Vec<u8> = (0..64 * 48).map(|k| ((k + i * 5) % 251) as u8).collect();
        let c = vec![128u8; 32 * 24];
        units.extend(enc.encode(&YuvFrame::U8 { y: &y, u: &c, v: &c, y_stride: 64, uv_stride: 32 }, i as i64).unwrap());
    }
    let av1c = enc.av1c();
    let data: Vec<&[u8]> = units.iter().map(|p| p.data.as_slice()).collect();
    let packets = data.iter().zip(&units).map(|(d, p)| Packet { data: d, pts: Some(p.pts), dts: Some(p.pts), keyframe: p.keyframe });
    let ix = index_av1(1, Rational::new(1, 25), av1c.get(4..).unwrap_or_default(), packets).unwrap();
    assert_eq!(ix.frames.len(), 30);
    for f in &ix.frames {
        let key = f.decode_index % 10 == 0;
        assert_eq!(f.random_access == RandomAccessKind::KeyFrame, key, "{f:?}");
        assert_eq!(f.keyframe, key, "the encoder's key flag agrees: {f:?}");
        assert_eq!(f.dependency_status, if key { DependencyStatus::Independent } else { DependencyStatus::WithinGop }, "{f:?}");
    }
    assert_eq!(ix.gops.len(), 3);
}
