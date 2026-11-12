//! Writer tests: files read back with our own demuxer; truncation and mutation never panic.

use std::io::Cursor;

use super::*;
use crate::{Codec, OperationalPattern, PartitionKind, TrackKind, Wrapping, open};

fn prores_like(i: usize) -> Vec<u8> {
    let mut v = vec![0, 0, 0, 40];
    v.extend_from_slice(b"icpf");
    v.extend(std::iter::repeat_n(i as u8, 32 + i * 3));
    v
}

fn vc3_like(i: usize) -> Vec<u8> {
    let mut v = vec![0x00, 0x00, 0x02, 0x80, 0x01];
    v.extend(std::iter::repeat_n(i as u8 ^ 0x5A, 500 + 7 * i));
    v
}

/// 16-bit stereo PCM: frame k = (k, -k).
fn pcm16(start: usize, n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for k in start..start + n {
        let s = (k % 30000) as i16;
        v.extend_from_slice(&s.to_le_bytes());
        v.extend_from_slice(&(-s).to_le_bytes());
    }
    v
}

/// 24-bit mono PCM: frame k = 1000 * k (wrapping inside the 24-bit range).
fn pcm24(start: usize, n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for k in start..start + n {
        let s = ((k as i64 * 1000) % 8_000_000) as i32 - 4_000_000;
        v.extend_from_slice(&s.to_le_bytes()[..3]);
    }
    v
}

fn op1a_config(rate: Rational, coding: PictureCoding) -> WriterConfig {
    let mut cfg = WriterConfig::new(Pattern::Op1a, rate, PackageIds::from_seed("test", "Test Clip"));
    cfg.picture = Some(PictureDesc::new(coding, 64, 36));
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 16 }, SoundDesc { sample_rate: 48_000, channels: 1, bits: 24 }];
    cfg.timecode = Some(StartTimecode { frames: 90_000, rate, drop_frame: false });
    cfg
}

#[test]
fn op1a_rejects_an_unbounded_pcm_edit_unit_before_padding() {
    let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(1, 36_000), PackageIds::from_seed("low-rate", "Low Rate"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::ProRes { profile: 3 }, 64, 36));
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 16 }];
    let err = MxfWriter::new(Cursor::new(Vec::new()), cfg).err().unwrap();
    assert!(err.to_string().contains("PCM edit unit"));
}

/// 10 ProRes-like pictures at 25 fps with stereo 16-bit and mono 24-bit sound pushed in uneven chunks.
fn build_op1a() -> Vec<u8> {
    let cfg = op1a_config(Rational::new(25, 1), PictureCoding::ProRes { profile: 3 });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let total = 10 * 1920;
    let (mut a, mut b) = (0, 0);
    for i in 0..10 {
        // sound before and after the picture, in chunks unrelated to the edit units
        let n = 1500.min(total - a);
        w.push_sound(0, &pcm16(a, n)).unwrap();
        a += n;
        w.push_picture(&prores_like(i), FrameInfo::intra()).unwrap();
        let m = 2500.min(total - b);
        w.push_sound(1, &pcm24(b, m)).unwrap();
        b += m;
    }
    w.push_sound(0, &pcm16(a, total - a)).unwrap();
    assert_eq!(w.edit_units(), 10);
    w.finish().unwrap().into_inner()
}

#[test]
fn op1a_reads_back() {
    let f = build_op1a();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
    assert_eq!(m.material_package_name.as_deref(), Some("Test Clip"));
    assert_eq!(m.product_name.as_deref(), Some("CFA4"));
    let tc = m.timecode.unwrap();
    assert_eq!(tc.format(), "01:00:00:00");
    // header rewritten closed and complete; body and footer partitions; footer metadata used
    let kinds: Vec<(PartitionKind, u8)> = m.partitions.iter().map(|p| (p.kind, p.status)).collect();
    assert_eq!(kinds, vec![(PartitionKind::Header, 4), (PartitionKind::Body, 4), (PartitionKind::Footer, 4)]);
    assert!(m.partitions.iter().all(|p| p.footer_partition == m.partitions[2].offset));
    assert_eq!(m.tracks.len(), 3);
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.codec, Codec::ProRes { profile: Some(3) });
    assert_eq!(t.duration, Some(10));
    assert_eq!(t.edit_rate, Rational::new(25, 1));
    assert_eq!(t.wrapping, Wrapping::Frame);
    assert!(t.indexed || t.codec.intra_only());
    let p = t.picture.as_ref().unwrap();
    assert_eq!((p.frame_width(), p.frame_height(), p.component_depth), (64, 36, 10));
    assert_eq!(p.aspect_ratio, Rational::new(16, 9));
    for i in 0..10 {
        assert_eq!(m.read_sample(&f, v, i).unwrap(), prores_like(i));
    }
    let sounds: Vec<usize> = (0..m.tracks.len()).filter(|&i| m.tracks[i].kind == TrackKind::Sound).collect();
    assert_eq!(sounds.len(), 2);
    let s0 = &m.tracks[sounds[0]];
    assert_eq!(s0.codec, Codec::Pcm);
    assert_eq!(s0.stored_sample_frames(), 10 * 1920);
    assert_eq!(s0.chunks.len(), 10);
    let pcm = m.read_pcm(&f, sounds[0], 1900, 100).unwrap();
    for k in 0..100 {
        assert_eq!(pcm[0][k], (1900 + k) as f32 / 32768.0);
        assert_eq!(pcm[1][k], -((1900 + k) as f32) / 32768.0);
    }
    let s1 = &m.tracks[sounds[1]];
    assert_eq!(s1.sound.as_ref().unwrap().bits, 24);
    let pcm = m.read_pcm(&f, sounds[1], 0, 10 * 1920).unwrap();
    for k in (0..10 * 1920).step_by(97) {
        let want = ((k as i64 * 1000) % 8_000_000) as i32 - 4_000_000;
        assert_eq!(pcm[0][k], want as f32 / 8_388_608.0, "sample {k}");
    }
    // index: one entry per content package, two sound slices
    let entries: usize = m.index_segments.iter().map(|s| s.entries.len()).sum();
    assert_eq!(entries, 10);
    assert_eq!(m.index_segments[0].slice_count, 2);
    assert_eq!(m.index_segments[0].delta_entries.len(), 3);
}

#[test]
fn ntsc_sound_follows_the_edit_unit_pattern() {
    let rate = Rational::new(30000, 1001);
    let mut cfg = op1a_config(rate, PictureCoding::Vc3 { cid: 1274 });
    cfg.sound.truncate(1);
    cfg.timecode = Some(StartTimecode { frames: 107_892, rate, drop_frame: true });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let n = 12;
    let total = (n as u64 * 48_000 * 1001 / 30_000) as usize;
    w.push_sound(0, &pcm16(0, total)).unwrap();
    for i in 0..n {
        w.push_picture(&vc3_like(i), FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert_eq!(m.timecode.unwrap().format(), "01:00:00;00");
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    let sizes: Vec<u64> = m.tracks[a].chunks.iter().map(|c| c.samples).collect();
    assert_eq!(sizes[..5], [1601, 1602, 1601, 1602, 1602]);
    assert_eq!(m.tracks[a].stored_sample_frames(), total as u64);
    let pcm = m.read_pcm(&f, a, 0, total).unwrap();
    assert!((0..total).all(|k| pcm[0][k] == (k % 30000) as f32 / 32768.0));
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    assert_eq!(m.tracks[v].codec, Codec::Vc3);
}

#[test]
fn long_gop_index_gives_presentation_order_and_key_frames() {
    let rate = Rational::new(25, 1);
    let mut cfg = op1a_config(rate, PictureCoding::Avc { profile_idc: 100, intra: false });
    cfg.sound.clear();
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    // stored I0 P3 B1 B2 P6 B4 B5 I7(IDR)
    let order = [(0, CodedKind::Intra, true), (3, CodedKind::Predicted, false), (1, CodedKind::Bidirectional, false), (2, CodedKind::Bidirectional, false)];
    let more = [(6, CodedKind::Predicted, false), (4, CodedKind::Bidirectional, false), (5, CodedKind::Bidirectional, false), (7, CodedKind::Intra, true)];
    for (i, (d, kind, key)) in order.iter().chain(more.iter()).enumerate() {
        let au = vec![0, 0, 0, 1, if *key { 0x65 } else { 0x41 }, i as u8];
        w.push_picture(&au, FrameInfo { key: *key, kind: *kind, display: Some(*d) }).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    let t = &m.tracks[0];
    assert_eq!(t.codec, Codec::Avc { intra: false });
    assert!(t.temporal_offsets && t.indexed);
    let pts: Vec<i64> = t.samples.iter().map(|s| s.pts).collect();
    assert_eq!(pts, vec![0, 3, 1, 2, 6, 4, 5, 7]);
    let keys: Vec<bool> = t.samples.iter().map(|s| s.key).collect();
    assert_eq!(keys, vec![true, false, false, false, false, false, false, true]);
    let b: Vec<bool> = t.samples.iter().map(|s| s.b_picture).collect();
    assert_eq!(b, vec![false, false, true, true, false, true, true, false]);
    assert_eq!(t.sample_at(4), Some(5));
}

#[test]
fn op_atom_picture_is_clip_wrapped() {
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(24, 1), PackageIds::from_seed("atom-v", "Atom V"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
    cfg.timecode = Some(StartTimecode { frames: 86_400, rate: Rational::new(24, 1), drop_frame: false });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..9 {
        w.push_picture(&vc3_like(i), FrameInfo::intra()).unwrap();
    }
    assert!(w.push_sound(0, &[0, 0]).is_err());
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Atom);
    assert_eq!(m.timecode.unwrap().format(), "01:00:00:00");
    let t = &m.tracks[0];
    assert_eq!(t.wrapping, Wrapping::Clip);
    assert_eq!(t.codec, Codec::Vc3);
    assert_eq!(t.samples.len(), 9);
    for i in 0..9 {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), vc3_like(i));
    }
}

#[test]
fn op_atom_pcm_helper() {
    let ids = PackageIds::from_seed("atom-a", "Atom A1");
    let samples: Vec<i32> = (0..4801).map(|k| (k * 7 % 65536) - 32768).collect();
    for rate in [None, Some(Rational::new(25, 1))] {
        let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 1, edit_rate: rate, ids: ids.clone(), timecode: None };
        let f = write_opatom_pcm(&opts, &samples).unwrap();
        let m = open(&f).unwrap();
        assert_eq!(m.operational_pattern, OperationalPattern::Atom);
        let t = &m.tracks[0];
        assert_eq!(t.kind, TrackKind::Sound);
        assert_eq!(t.stored_sample_frames(), 4801);
        assert_eq!(t.duration, Some(if rate.is_some() { 3 } else { 4801 }));
        let pcm = m.read_pcm(&f, 0, 0, 4801).unwrap();
        assert!(samples.iter().zip(&pcm[0]).all(|(s, p)| *s as f32 / 32768.0 == *p));
    }
    // 24-bit stereo
    let opts = OpAtomPcm { sample_rate: 48_000, bits: 24, channels: 2, edit_rate: None, ids, timecode: None };
    let f = write_opatom_pcm(&opts, &[8_388_607, -8_388_608, 1, -1]).unwrap();
    let m = open(&f).unwrap();
    let pcm = m.read_pcm(&f, 0, 0, 2).unwrap();
    assert_eq!(pcm[0], vec![8_388_607.0 / 8_388_608.0, 1.0 / 8_388_608.0]);
    assert_eq!(pcm[1], vec![-1.0, -1.0 / 8_388_608.0]);
}

#[test]
fn package_ids_are_written() {
    let f = build_op1a();
    let ids = PackageIds::from_seed("test", "Test Clip");
    assert!(f.windows(32).any(|w| w == ids.material.0));
    assert!(f.windows(32).any(|w| w == ids.file.0));
    assert_ne!(ids.material, ids.file);
    assert_eq!(Umid::from_seed(b"x"), Umid::from_seed(b"x"));
    assert_ne!(Umid::from_seed(b"x"), Umid::from_seed(b"y"));
}

#[test]
fn long_files_split_the_index_into_segments() {
    let mut cfg = op1a_config(Rational::new(25, 1), PictureCoding::ProRes { profile: 4 });
    cfg.sound.clear();
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..12_000 {
        w.push_picture(&[0, 0, 0, 9, b'i', b'c', b'p', b'f', i as u8], FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.index_segments.len() >= 2);
    assert_eq!(m.index_segments.iter().map(|s| s.entries.len()).sum::<usize>(), 12_000);
    assert_eq!(m.tracks[0].samples.len(), 12_000);
    assert_eq!(m.read_sample(&f, 0, 11_999).unwrap()[8], (11_999 % 256) as u8);
}

#[test]
fn rejects_bad_configs() {
    let ids = PackageIds::from_seed("x", "x");
    let cfg = WriterConfig::new(Pattern::Op1a, Rational::new(25, 1), ids.clone());
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(25, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 1, bits: 16 }; 2];
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
}

#[test]
fn timestamps() {
    assert_eq!(Timestamp::from_unix(0), Timestamp { year: 1970, month: 1, day: 1, ..Default::default() });
    let t = Timestamp::from_unix(1_700_000_000);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (2023, 11, 14, 22, 13, 20));
    let t = Timestamp::from_unix(951_782_400); // 2000-02-29
    assert_eq!((t.year, t.month, t.day), (2000, 2, 29));
    // Years far outside chrono's own range are preserved with month/day/time intact and only the
    // year clamped to the MXF `u16`, exactly as the old hand-rolled conversion did (not collapsed
    // to the epoch — reviewer FredApps). `10_000_000_000_000` s → 318857-05-20 17:46:40.
    let t = Timestamp::from_unix(10_000_000_000_000);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (65535, 5, 20, 17, 46, 40));
    // Year 10000 (just past chrono's range) still fits the u16 field.
    let t = Timestamp::from_unix(253_402_300_800);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (10000, 1, 1, 0, 0, 0));
    // A negative year clamps to 0 but keeps its month/day/time.
    let t = Timestamp::from_unix(-10_000_000_000_000);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (0, 8, 13, 6, 13, 20));
    // The `i64` extremes the old code accepted.
    let t = Timestamp::from_unix(i64::MAX);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (65535, 12, 4, 15, 30, 7));
    let t = Timestamp::from_unix(i64::MIN);
    assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (0, 1, 27, 8, 29, 52));
}

#[test]
fn truncated_and_mutated_files_never_panic() {
    let f = build_op1a();
    let mut opened = 0;
    for cut in (0..f.len()).step_by(41) {
        let part = f[..cut].to_vec();
        if let Ok(m) = open(&part) {
            opened += 1;
            for (ti, t) in m.tracks.iter().enumerate() {
                for i in 0..t.samples.len() {
                    assert!(m.read_sample(&part, ti, i).is_ok(), "cut {cut}");
                }
                let _ = m.read_pcm(&part, ti, 0, 3000);
            }
        }
    }
    assert!(opened > 20, "a truncated file with a complete header still opens ({opened})");
    let atom = write_opatom_pcm(
        &OpAtomPcm { sample_rate: 48_000, bits: 24, channels: 1, edit_rate: None, ids: PackageIds::from_seed("m", "m"), timecode: None },
        &(0..3000).collect::<Vec<i32>>(),
    )
    .unwrap();
    let mut seed = 0x9E37_79B9u64;
    for src in [&f, &atom] {
        for cut in (0..src.len()).step_by(53) {
            let _ = open(&src[..cut].to_vec()).map(|m| m.read_pcm(&src[..cut].to_vec(), 0, 0, 100).ok());
        }
        for _ in 0..400 {
            let mut g = src.clone();
            for _ in 0..6 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let at = (seed as usize) % g.len();
                g[at] = (seed >> 40) as u8;
            }
            if let Ok(m) = open(&g) {
                for (ti, t) in m.tracks.iter().enumerate() {
                    for i in 0..t.samples.len().min(12) {
                        let _ = m.read_sample(&g, ti, i);
                    }
                    let _ = m.read_pcm(&g, ti, 0, 500);
                }
            }
        }
    }
}

/// #210 review: a whole-clip read of long stereo PCM (more samples than the old fixed
/// 16 777 216-sample request cap, about 175 s at 48 kHz) must work; only reads far past the stored
/// essence are refused.
#[test]
fn long_pcm_reads_backed_by_the_file_are_not_capped() {
    let frames = 200 * 48_000;
    let samples: Vec<i32> = (0..frames * 2).map(|k| (k % 60_000) as i32 - 30_000).collect();
    let opts = OpAtomPcm { sample_rate: 48_000, bits: 16, channels: 2, edit_rate: None, ids: PackageIds::from_seed("long", "Long"), timecode: None };
    let f = write_opatom_pcm(&opts, &samples).unwrap();
    let m = open(&f).unwrap();
    assert_eq!(m.tracks[0].stored_sample_frames(), frames as u64);
    // a little past the end is zero padding, as before
    let pcm = m.read_pcm(&f, 0, 0, frames + 10).unwrap();
    assert_eq!((pcm.len(), pcm[0].len()), (2, frames + 10));
    for k in [0, 1, frames / 2, frames - 1] {
        assert_eq!(pcm[0][k], samples[2 * k] as f32 / 32768.0, "frame {k}");
        assert_eq!(pcm[1][k], samples[2 * k + 1] as f32 / 32768.0, "frame {k}");
    }
    assert_eq!(pcm[0][frames..], [0.0; 10]);
    // a request that is mostly not backed by data is refused before allocating
    assert!(m.read_pcm(&f, 0, 0, frames + (1 << 24)).is_err());
}

/// A J2K-codestream-shaped frame (SOC, SIZ marker start, filler).
fn j2k_like(i: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F];
    v.extend(std::iter::repeat_n(i as u8 ^ 0x33, 900 + 11 * i));
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

/// #115: a D-Cinema picture track file (ST 429-4): OP-Atom, frame-wrapped J2K, RGBA descriptor
/// with a JPEG 2000 sub-descriptor, closed complete header and no metadata in the footer.
#[test]
fn dcinema_picture_track_file() {
    let uuid = [0x11u8; 16];
    let mut ids = PackageIds::from_seed("dcp-pic", "Reel 1");
    ids.file = Umid::dcinema(uuid);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids.clone());
    cfg.picture = Some(PictureDesc::dcinema(3, 1998, 1080));
    cfg.timecode = Some(StartTimecode { frames: 0, rate: Rational::new(24, 1), drop_frame: false });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..7 {
        w.push_picture(&j2k_like(i), FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Atom);
    let kinds: Vec<(PartitionKind, u8)> = m.partitions.iter().map(|p| (p.kind, p.status)).collect();
    assert_eq!(kinds.first(), Some(&(PartitionKind::Header, 4)));
    let footer = m.partitions.iter().find(|p| p.kind == PartitionKind::Footer).unwrap();
    assert_eq!(footer.header_byte_count, 0, "ST 429-3: header metadata only in the header partition");
    assert_eq!(m.tracks.len(), 1);
    let t = &m.tracks[0];
    assert_eq!(t.codec, Codec::Jpeg2000);
    assert_eq!(t.wrapping, Wrapping::Frame);
    assert_eq!(t.duration, Some(7));
    let p = t.picture.as_ref().unwrap();
    assert!(p.rgba);
    assert_eq!((p.stored_width, p.stored_height), (1998, 1080));
    assert_eq!(p.aspect_ratio, Rational::new(37, 20));
    for i in 0..7 {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), j2k_like(i));
    }
    // the file package UMID carries the asset UUID; the preface names it as primary package
    assert!(f.windows(32).any(|w| w == ids.file.0));
    assert!(f.windows(4).any(|w| w == [0x3B, 0x08, 0x00, 0x10]));
    // the JPEG 2000 sub-descriptor (ST 422) is present: Rsiz 3, Xsiz 1998
    assert!(f.windows(2).any(|w| w == [0x00, 0x03]));
    for cut in (0..f.len()).step_by(97) {
        if let Ok(m) = open(&f[..cut]) {
            for i in 0..m.tracks.first().map_or(0, |t| t.samples.len()) {
                let _ = m.read_sample(&f[..cut], 0, i);
            }
        }
    }
}

/// #115: a D-Cinema sound track file (ST 429-3): 5.1 + HI/VI (8 channels), 24-bit 48 kHz,
/// frame-wrapped, ChannelAssignment config 1.
#[test]
fn dcinema_sound_track_file() {
    const CFG1: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0B, 0x04, 0x02, 0x02, 0x10, 0x03, 0x01, 0x01, 0x00];
    let mut ids = PackageIds::from_seed("dcp-snd", "Reel 1");
    ids.file = Umid::dcinema([0x22; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 6, bits: 24 }];
    cfg.channel_assignment = Some(CFG1);
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let frames = 5 * 2000;
    let mut pcm = Vec::new();
    for k in 0..frames {
        for c in 0..6i32 {
            let s = (k as i32 * 3 + c * 100_000) % 8_000_000;
            pcm.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    w.push_sound(0, &pcm).unwrap();
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Atom);
    let t = &m.tracks[0];
    assert_eq!(t.kind, TrackKind::Sound);
    assert_eq!(t.wrapping, Wrapping::Frame);
    assert_eq!(t.duration, Some(5));
    assert_eq!(t.stored_sample_frames(), frames as u64);
    let got = m.read_pcm(&f, 0, 0, frames).unwrap();
    assert_eq!(got.len(), 6);
    for k in (0..frames).step_by(37) {
        for c in 0..6 {
            let want = (k as i32 * 3 + c as i32 * 100_000) % 8_000_000;
            assert_eq!(got[c][k], want as f32 / 8_388_608.0);
        }
    }
    assert!(f.windows(16).any(|w| w == CFG1));
}

/// #348: MCA labels (ST 377-4) on a D-Cinema sound track file: a soundfield group and two
/// channels, referenced from the Wave descriptor and named in the primer.
#[test]
fn dcinema_sound_mca_labels() {
    let mut ids = PackageIds::from_seed("dcp-mca", "Reel 1");
    ids.file = Umid::dcinema([0x23; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 16, bits: 24 }];
    let group = [0x51; 16];
    let label = |dict: u8, sym: &str, name: &str, channel: Option<u32>, group: Option<[u8; 16]>| McaLabel {
        dictionary: [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x03, 0x02, dict, 0x01, 0, 0, 0, 0],
        link: [channel.unwrap_or(0) as u8 + 0x60; 16],
        symbol: sym.into(),
        name: name.into(),
        channel,
        language: Some("en-US".into()),
        group,
        titles: None,
    };
    cfg.mca = vec![label(2, "sg51", "5.1", None, None), label(1, "chL", "Left", Some(1), Some(group)), label(1, "chC", "Center", Some(3), Some(group))];
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    w.push_sound(0, &vec![0u8; 2000 * 16 * 3]).unwrap();
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.tracks[0].stored_sample_frames(), 2000);
    let key = |b: u8| [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x01, 0x01, 0x01, 0x01, b, 0x00];
    let count = |k: &[u8]| f.windows(k.len()).filter(|w| w == &k).count();
    assert_eq!(count(&key(0x6C)), 1, "one soundfield group label");
    assert_eq!(count(&key(0x6B)), 2, "two audio channel labels");
    // the primer names the MCA items (MCATagSymbol 0e.01.03.07.01.02 at the dynamic tag)
    let mut primer = labels::TAG_MCA.wrapping_add(2).to_be_bytes().to_vec();
    primer.extend_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x07, 0x01, 0x02]);
    assert_eq!(count(&primer), 1);
    // symbols in UTF-16, the channel id, the group link
    let sym: Vec<u8> = "chC".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    assert_eq!(count(&sym), 1);
    let mut chan = (labels::TAG_MCA + 4).to_be_bytes().to_vec();
    chan.extend_from_slice(&[0, 4, 0, 0, 0, 3]);
    assert_eq!(count(&chan), 1);
    assert_eq!(count(&group), 2);
}

#[test]
fn dcinema_needs_one_essence_track() {
    let ids = PackageIds::from_seed("x", "x");
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.picture = Some(PictureDesc::dcinema(3, 2048, 1080));
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 6, bits: 24 }];
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
}

/// A fake IABitstream frame: preamble tag + length, IAFrame tag, then `i`-dependent bytes.
fn iab_like(i: usize) -> Vec<u8> {
    let mut v = vec![0x01, 0, 0, 0, 0, 0x02];
    v.extend(std::iter::repeat_n(i as u8 ^ 0x5A, 300 + 7 * i));
    v
}

/// #343: an immersive audio track file (ST 429-18): frame-wrapped data elements under the class
/// 14 key, the IAB data essence descriptor and sub-descriptor, read back as an IAB sound track.
#[test]
fn dcinema_iab_track_file() {
    let mut ids = PackageIds::from_seed("dcp-iab", "Reel 1");
    ids.file = Umid::dcinema([0x33; 16]);
    let mut cfg = WriterConfig::new(Pattern::DCinema, Rational::new(24, 1), ids);
    cfg.iab = Some(IabDesc { version: 1, max_channels: 10, max_objects: 118, first_frame: 0, sample_rate: 48_000 });
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..9 {
        w.push_data(&iab_like(i)).unwrap();
    }
    assert!(w.push_picture(&[0; 4], FrameInfo::intra()).is_err());
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Atom);
    assert_eq!(m.tracks.len(), 1);
    let t = &m.tracks[0];
    assert_eq!(t.kind, TrackKind::Sound);
    assert_eq!(t.codec, Codec::Iab);
    assert_eq!(t.duration, Some(9));
    assert_eq!(t.edit_rate, Rational::new(24, 1));
    assert_eq!(t.sound.as_ref().unwrap().sample_rate, Rational::new(48_000, 1));
    for i in 0..9 {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), iab_like(i));
    }
    // element key (6.1), essence container (Table 2), coding (Table 4), set keys (Tables 3, 5)
    for ul in [labels::IAB_ELEMENT, labels::EC_IAB, labels::IAB_CODING, labels::IAB_DESCRIPTOR, labels::IAB_SUB_DESCRIPTOR] {
        assert!(f.windows(16).any(|w| w == ul), "{ul:02x?}");
    }
    // clause 10: the primer maps 0x3E01 to the version 5 data essence coding item
    let coding_item = [0x3E, 0x01, 0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x04, 0x03, 0x03, 0x02];
    assert!(f.windows(coding_item.len()).any(|w| w == coding_item));
    // the data track (data definition "data")
    assert!(f.windows(16).any(|w| w == labels::DEF_DATA));
    for cut in (0..f.len()).step_by(89) {
        if let Ok(m) = open(&f[..cut]) {
            for i in 0..m.tracks.first().map_or(0, |t| t.samples.len()) {
                let _ = m.read_sample(&f[..cut], 0, i);
            }
        }
    }
}

#[test]
fn iab_is_only_a_dcinema_track_file() {
    let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(24, 1), PackageIds::from_seed("x", "x"));
    cfg.iab = Some(IabDesc::default());
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg.clone()).is_err());
    cfg.pattern = Pattern::DCinema;
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 6, bits: 24 }];
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
}

/// A picture-like payload: an MPEG-2 sequence header start code then `len` filler bytes.
fn d10_picture(i: usize, len: usize) -> Vec<u8> {
    let mut v = vec![0x00, 0x00, 0x01, 0xB3];
    v.extend(std::iter::repeat_n((i as u8).wrapping_mul(37) | 1, len));
    v
}

/// 24-bit 4-channel PCM: frame k, channel c = (k * 4 + c) * 1000 - 3_000_000.
fn pcm24x4(start: usize, n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for k in start..start + n {
        for c in 0..4 {
            let s = ((k * 4 + c) as i64 * 1000 % 6_000_000) as i32 - 3_000_000;
            v.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    v
}

fn d10_config(mbps: u8, lines: u16, rate: Rational) -> WriterConfig {
    let mut cfg = WriterConfig::new(Pattern::Op1a, rate, PackageIds::from_seed("d10", "D-10 Clip"));
    let mut p = PictureDesc::new(PictureCoding::D10 { mbps, lines }, 720, if lines == 625 { 608 } else { 512 });
    p.aspect = Rational::new(4, 3);
    cfg.picture = Some(p);
    cfg.timecode = Some(StartTimecode { frames: 90_000, rate, drop_frame: false });
    cfg
}

#[test]
fn d10_625_content_packages() {
    let rate = Rational::new(25, 1);
    let mut cfg = d10_config(50, 625, rate);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 4, bits: 24 }];
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let n = 6;
    let total = n * 1920;
    let mut a = 0;
    for i in 0..n {
        let k = 1000.min(total - a);
        w.push_sound(0, &pcm24x4(a, k)).unwrap();
        a += k;
        w.push_picture(&d10_picture(i, 100_000 + 20_000 * i), FrameInfo::intra()).unwrap();
    }
    w.push_sound(0, &pcm24x4(a, total - a)).unwrap();
    let too_big = w.push_picture(&vec![0u8; 250_001], FrameInfo::intra()).unwrap_err();
    assert!(too_big.to_string().contains("250000-byte"), "{too_big}");
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert!(m.partitions.iter().all(|p| p.kag_size == 512));
    let ec = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x01, 0x01, 0x01];
    assert_eq!(m.partitions[0].essence_containers.iter().map(|u| u.0).collect::<Vec<_>>(), vec![ec]);
    let body = &m.partitions[1];
    assert_eq!(body.offset % 512, 0);
    let start = body.essence_start.unwrap() as usize;
    assert_eq!(start % 512, 0);
    // constant content packages: index with the edit unit byte count and three delta entries
    let seg = &m.index_segments[0];
    assert_eq!(seg.duration, n as i64);
    let cp = seg.edit_unit_byte_count as usize;
    assert_eq!(cp, 512 + 250_368 + 61_952);
    assert_eq!(seg.delta_entries.len(), 3);
    assert_eq!(m.partitions[2].offset as usize, start + n * cp);
    for i in 0..n {
        let at = start + i * cp;
        // system item: system metadata pack, user time stamp (01:00:00:00 + i)
        assert_eq!(f[at..at + 16], SYSTEM_METADATA_PACK);
        let pack = &f[at + 20..at + 77];
        assert_eq!(pack[..2], [0x7C, 0x04]);
        assert_eq!(u16::from_be_bytes([pack[5], pack[6]]), i as u16);
        assert_eq!(pack[7..23], ec);
        assert_eq!(pack[40..45], [0x81, i as u8, 0x00, 0x00, 0x01]);
        // picture item on the KAG, padded to 250 000 bytes
        assert_eq!(f[at + 512..at + 528], element_key(0x05, 1, 0x01, 1));
        assert_eq!(f[at + 528..at + 532], [0x83, 0x03, 0xD0, 0x90]);
        let snd = at + 512 + 250_368;
        assert_eq!(f[snd..snd + 16], element_key(0x06, 1, 0x10, 1));
        assert_eq!(f[snd + 20..snd + 24], [0x00, 0x80, 0x07, 0x0F]);
    }
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.codec, Codec::Mpeg2);
    assert_eq!(t.duration, Some(n as i64));
    let p = t.picture.as_ref().unwrap();
    assert_eq!((p.stored_width, p.stored_height, p.frame_layout, p.display_height), (720, 304, 1, 288));
    assert_eq!((p.frame_width(), p.frame_height()), (720, 576));
    for i in 0..n {
        let s = m.read_sample(&f, v, i).unwrap();
        let want = d10_picture(i, 100_000 + 20_000 * i);
        assert_eq!(s.len(), 250_000);
        assert_eq!(s[..want.len()], want[..]);
        assert!(s[want.len()..].iter().all(|&b| b == 0));
    }
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    assert_eq!(m.tracks[a].codec, Codec::Aes3Element);
    assert_eq!(m.tracks[a].sound.as_ref().unwrap().channels, 4);
    let pcm = m.read_pcm(&f, a, 0, total).unwrap();
    for k in (0..total).step_by(53) {
        for (c, ch) in pcm.iter().enumerate().take(4) {
            let want = ((k * 4 + c) as i64 * 1000 % 6_000_000) as i32 - 3_000_000;
            assert_eq!(ch[k], want as f32 / 8_388_608.0, "sample {k} channel {c}");
        }
    }
}

#[test]
fn d10_525_without_sound() {
    let rate = Rational::new(30000, 1001);
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), d10_config(30, 525, rate)).unwrap();
    let n = 7;
    for i in 0..n {
        w.push_picture(&d10_picture(i, 90_000), FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let start = m.partitions[1].essence_start.unwrap() as usize;
    let cp = m.index_segments[0].edit_unit_byte_count as usize;
    assert_eq!(cp, 512 + 125_440 + 51_712);
    let mut counts = Vec::new();
    for i in 0..n {
        let at = start + i * cp;
        assert_eq!(f[at + 21], 0x07, "content package rate 30000/1001");
        let snd = at + 512 + 125_440;
        let h = &f[snd + 20..snd + 24];
        // no valid channels; the 5-sequence count follows the 1601/1602 pattern
        assert_eq!(h[3], 0);
        counts.push((h[0], u16::from_le_bytes([h[1], h[2]])));
    }
    assert_eq!(counts, vec![(2, 1601), (3, 1602), (4, 1601), (5, 1602), (1, 1602), (2, 1601), (3, 1602)]);
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let p = m.tracks[v].picture.as_ref().unwrap();
    assert_eq!((p.stored_height, p.display_height), (256, 243));
    // a silent 4-channel track describes the AES3 element
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    assert_eq!(m.tracks[a].sound.as_ref().unwrap().channels, 4);
}

#[test]
fn d10_rejects_bad_configs() {
    let pal = Rational::new(25, 1);
    let bad = |cfg: WriterConfig| MxfWriter::new(Cursor::new(Vec::new()), cfg).err().unwrap().to_string();
    assert!(bad(d10_config(45, 625, pal)).contains("30, 40 or 50"));
    assert!(bad(d10_config(50, 625, Rational::new(30000, 1001))).contains("25 Hz"));
    let mut cfg = d10_config(50, 625, pal);
    cfg.pattern = Pattern::OpAtom;
    cfg.sound.clear();
    assert!(bad(cfg).contains("OP1a"));
    let mut cfg = d10_config(50, 625, pal);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 16 }; 2];
    assert!(bad(cfg).contains("one sound track"));
    let mut cfg = d10_config(50, 625, pal);
    cfg.sound = vec![SoundDesc { sample_rate: 44_100, channels: 2, bits: 16 }];
    assert!(bad(cfg).contains("48 kHz"));
}

#[test]
fn aes3_words_and_timecode_stamps() {
    // two 16-bit channels: 0x1234 and -2 in the top 16 bits of the 24-bit word
    let pcm = [0x34, 0x12, 0xFE, 0xFF];
    let e = aes3_element(&pcm, 2, 16, 1, 3);
    assert_eq!(e[..4], [3, 1, 0, 0x03]);
    assert_eq!(u32::from_le_bytes(e[4..8].try_into().unwrap()), 0x123400 << 4);
    assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), (0xFFFE00 << 4) | 1);
    assert!((2..8).all(|c| u32::from_le_bytes(e[4 + 4 * c..8 + 4 * c].try_into().unwrap()) == c as u32));
    // truncated PCM gives silence, never a panic
    assert_eq!(aes3_element(&pcm[..3], 2, 24, 2, 0).len(), 4 + 64);
    assert_eq!(timecode_stamp(90_000 + 25 * 61 + 3, 25, false)[..5], [0x81, 0x03, 0x01, 0x01, 0x01]);
    // 29.97 drop frame: frame 1800 is 00:01:00;02
    assert_eq!(timecode_stamp(1800, 30, true)[..5], [0x81, 0x42, 0x00, 0x01, 0x00]);
    assert_eq!(timecode_stamp(17_982, 30, true)[..5], [0x81, 0x40, 0x00, 0x10, 0x00]);
}

/// RDD 9 configuration: 1920x1080 MPEG-2 4:2:2 long GOP at `rate` with `channels` 24-bit channels.
fn rdd9_config(rate: Rational, channels: u32) -> WriterConfig {
    let mut cfg = WriterConfig::new(Pattern::Op1a, rate, PackageIds::from_seed("rdd9", "RDD 9 Clip"));
    let mut p =
        PictureDesc::new(PictureCoding::Mpeg2 { profile_and_level: 0x82, max_gop: 6, b_pictures: 2, closed_gop: true, bit_rate: 50_000_000 }, 1920, 1080);
    p.aspect = Rational::new(16, 9);
    cfg.picture = Some(p);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels, bits: 24 }];
    cfg.timecode = Some(StartTimecode { frames: 90_000, rate, drop_frame: false });
    cfg.rdd9 = true;
    cfg
}

/// Display order of stored picture `i` in closed six-picture GOPs I0 P3 B1 B2 P5 B4.
fn rdd9_frame(i: usize) -> FrameInfo {
    let gop = i / 6 * 6;
    let (d, kind) = match i % 6 {
        0 => (0, CodedKind::Intra),
        1 => (3, CodedKind::Predicted),
        2 => (1, CodedKind::Bidirectional),
        3 => (2, CodedKind::Bidirectional),
        4 => (5, CodedKind::Predicted),
        _ => (4, CodedKind::Bidirectional),
    };
    FrameInfo { key: i.is_multiple_of(6), kind, display: Some((gop + d) as i64) }
}

#[test]
fn rdd9_partitions_system_items_and_channel_elements() {
    let rate = Rational::new(25, 1);
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), rdd9_config(rate, 4)).unwrap();
    // 13 s: a new body partition at the first key frame five seconds into the previous one
    let n = 13 * 25;
    let total = n * 1920;
    let mut a = 0;
    for i in 0..n {
        let k = 1500.min(total - a);
        w.push_sound(0, &pcm24x4(a, k)).unwrap();
        a += k;
        w.push_picture(&d10_picture(i, 3000 + 7 * i), rdd9_frame(i)).unwrap();
    }
    w.push_sound(0, &pcm24x4(a, total - a)).unwrap();
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert!(m.partitions.iter().all(|p| p.kag_size == 512 && p.status == 4));
    // header, body at 0 s, 5.04 s (unit 126) and 10.08 s (unit 252), footer
    let kinds: Vec<PartitionKind> = m.partitions.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PartitionKind::Header, PartitionKind::Body, PartitionKind::Body, PartitionKind::Body, PartitionKind::Footer]);
    let footer = m.partitions[4].offset;
    for (k, p) in m.partitions.iter().enumerate() {
        assert_eq!(p.offset % 512, 0, "partition {k}");
        assert_eq!(p.footer_partition, footer, "partition {k}");
        if k > 0 {
            assert_eq!(p.previous_partition, m.partitions[k - 1].offset, "partition {k}");
        }
    }
    assert_eq!((m.partitions[2].index_sid, m.partitions[3].index_sid), (INDEX_SID, INDEX_SID));
    assert!(m.partitions[2].index_byte_count > 0 && m.partitions[2].index_byte_count % 512 == 0);
    // one index table segment per body partition, in the partition after it (footer: the last)
    let segs: Vec<(i64, i64)> = m.index_segments.iter().map(|s| (s.start_position, s.duration)).collect();
    assert_eq!(segs, vec![(0, 126), (126, 126), (252, n as i64 - 252)]);
    let el = 6_144u32; // 20 + 1920 * 3 bytes on the KAG
    for s in &m.index_segments {
        assert_eq!(s.slice_count, 1);
        assert_eq!(s.delta_entries, vec![(0, 0, 0), (-1, 0, 512), (0, 1, 0), (0, 1, el), (0, 1, 2 * el), (0, 1, 3 * el)]);
    }
    let first = m.partitions[1].essence_start.unwrap() as usize;
    assert_eq!(first % 512, 0);
    let ec = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x03, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x7F, 0x01, 0x00];
    for i in [0, 1, 125, 126, 127, 300] {
        let seg = m.index_segments.iter().find(|s| (s.start_position..s.start_position + s.duration).contains(&(i as i64))).unwrap();
        let e = &seg.entries[i - seg.start_position as usize];
        let body = m.partitions.iter().filter(|p| p.kind == PartitionKind::Body).rev().find(|p| p.body_offset <= e.stream_offset).unwrap();
        let at = body.essence_start.unwrap() as usize + (e.stream_offset - body.body_offset) as usize;
        assert_eq!(at % 512, 0, "content package {i}");
        assert_eq!(f[at..at + 16], SYSTEM_METADATA_PACK);
        let pack = &f[at + 20..at + 77];
        assert_eq!(pack[..2], [0x5C, 0x04]);
        assert_eq!(u16::from_be_bytes([pack[5], pack[6]]), i as u16);
        assert_eq!(pack[7..23], ec);
        assert!(pack[23..40].iter().all(|&b| b == 0), "no creation stamp");
        assert_eq!(pack[40], 0x81);
        assert_eq!(f[at + 512..at + 528], element_key(0x15, 1, 0x05, 0));
        let snd = at + 512 + kag_aligned(20 + 4 + 3000 + 7 * i);
        for c in 0..4 {
            let k = snd + c * el as usize;
            assert_eq!(f[k..k + 16], element_key(0x16, 4, 0x03, c as u8));
            assert_eq!(f[k + 16..k + 20], [0x83, 0x00, 0x16, 0x80]);
        }
    }
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.codec, Codec::Mpeg2);
    assert_eq!(t.duration, Some(n as i64));
    assert!(t.temporal_offsets && t.indexed);
    let pts: Vec<i64> = t.samples.iter().take(12).map(|s| s.pts).collect();
    assert_eq!(pts, vec![0, 3, 1, 2, 5, 4, 6, 9, 7, 8, 11, 10]);
    for i in [0, 125, 126, 251, 252, n - 1] {
        assert_eq!(m.read_sample(&f, v, i).unwrap(), d10_picture(i, 3000 + 7 * i), "picture {i}");
    }
    // four mono AES3 tracks, channel c holding channel c of the pushed PCM
    let sound: Vec<usize> = (0..m.tracks.len()).filter(|&k| m.tracks[k].sound.is_some()).collect();
    assert_eq!(sound.len(), 4);
    for (c, &k) in sound.iter().enumerate() {
        assert_eq!(m.tracks[k].sound.as_ref().unwrap().channels, 1);
        let pcm = m.read_pcm(&f, k, 0, total).unwrap();
        for s in (0..total).step_by(97) {
            let want = ((s * 4 + c) as i64 * 1000 % 6_000_000) as i32 - 3_000_000;
            assert_eq!(pcm[0][s], want as f32 / 8_388_608.0, "sample {s} channel {c}");
        }
    }
}

#[test]
fn rdd9_ntsc_sound_slices_and_silent_channels() {
    // 59.94 Hz: 800 or 801 samples, every element padded to the 801-sample size; one pushed
    // channel goes out as four elements, the others silent
    let rate = Rational::new(60_000, 1001);
    let mut cfg = rdd9_config(rate, 1);
    cfg.picture =
        Some(PictureDesc::new(PictureCoding::Mpeg2 { profile_and_level: 0x82, max_gop: 6, b_pictures: 2, closed_gop: true, bit_rate: 50_000_000 }, 1280, 720));
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let n = 10;
    let total = (n as u64 * 48_000 * 1001 / 60_000) as usize;
    let pcm: Vec<u8> = (0..total).flat_map(|s| ((s as i32) * 256).to_le_bytes()[..3].to_vec()).collect();
    w.push_sound(0, &pcm).unwrap();
    for i in 0..n {
        w.push_picture(&d10_picture(i, 500), rdd9_frame(i)).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let el = (20 + 801 * 3usize).next_multiple_of(512) as u32;
    assert_eq!(m.index_segments.len(), 1);
    assert_eq!(m.index_segments[0].delta_entries[3], (0, 1, el));
    let sound: Vec<usize> = (0..m.tracks.len()).filter(|&k| m.tracks[k].sound.is_some()).collect();
    assert_eq!(sound.len(), 4);
    let a = m.read_pcm(&f, sound[0], 0, total).unwrap();
    let b = m.read_pcm(&f, sound[1], 0, total).unwrap();
    for s in (0..total).step_by(31) {
        assert_eq!(a[0][s], (s as i32 * 256) as f32 / 8_388_608.0, "sample {s}");
        assert_eq!(b[0][s], 0.0);
    }
}

#[test]
fn rdd9_rejects_other_essence() {
    let rate = Rational::new(25, 1);
    let mut cfg = rdd9_config(rate, 2);
    cfg.sound[0].sample_rate = 44_100;
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
    let mut cfg = rdd9_config(rate, 9);
    cfg.sound[0].channels = 9;
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
    let mut cfg = rdd9_config(rate, 2);
    cfg.picture = Some(PictureDesc::new(PictureCoding::ProRes { profile: 3 }, 64, 36));
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
    let mut cfg = rdd9_config(rate, 2);
    cfg.pattern = Pattern::OpAtom;
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
}
/// Three uncompressed pictures of `layout` (32×4), each a different flat colour.
fn build_uncompressed(pattern: Pattern, layout: crate::UncompressedLayout) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut cfg = WriterConfig::new(pattern, Rational::new(25, 1), PackageIds::from_seed("raw", "Raw Clip"));
    cfg.picture = Some(PictureDesc::new(PictureCoding::Uncompressed { layout }, 48, 4));
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let mut pictures = Vec::new();
    for i in 0..3u16 {
        let (sx, sy) = layout.subsampling();
        let n = (48 / sx * 4 / sy) as usize;
        let p = layout.pack(48, 4, [&[16 + i; 192], &vec![100 + i; n], &vec![200 - i; n]]);
        w.push_picture(&p, FrameInfo::intra()).unwrap();
        pictures.push(p);
    }
    (w.finish().unwrap().into_inner(), pictures)
}

#[test]
fn uncompressed_pictures_round_trip() {
    use crate::uncompressed::{Layout, decode};
    for pattern in [Pattern::Op1a, Pattern::OpAtom] {
        for layout in [crate::UncompressedLayout::V210, crate::UncompressedLayout::Uyvy8, crate::UncompressedLayout::Planar420x8] {
            let (bytes, pictures) = build_uncompressed(pattern, layout);
            let f = open(&bytes[..]).unwrap();
            let t = f.tracks.iter().position(|t| t.kind == TrackKind::Picture).unwrap();
            let track = &f.tracks[t];
            assert_eq!(track.codec, Codec::Uncompressed, "{pattern:?} {layout:?}");
            assert_eq!(track.samples.len(), 3);
            assert_eq!(track.wrapping, if pattern == Pattern::OpAtom { Wrapping::Clip } else { Wrapping::Frame });
            let pi = track.picture.as_ref().unwrap();
            let l = Layout::of(pi).unwrap();
            assert_eq!(l, Layout::YCbCr(layout));
            for (i, want) in pictures.iter().enumerate() {
                let s = f.read_sample(&bytes[..], t, i).unwrap();
                assert_eq!(&s, want);
                let pic = decode(&l, pi, &s).unwrap();
                assert_eq!(pic.planes[0][0], 16 + i as u16);
                assert_eq!(pic.planes[2][0], 200 - i as u16);
            }
        }
    }
}

#[test]
fn clip_wrapped_uncompressed_without_an_index_splits_by_picture_size() {
    let (mut bytes, pictures) = build_uncompressed(Pattern::OpAtom, crate::UncompressedLayout::Uyvy8);
    // turn every index table segment into fill
    let index_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00];
    let mut hits = 0;
    for at in 0..bytes.len().saturating_sub(16) {
        if bytes[at..at + 16] == index_key {
            bytes[at..at + 16].copy_from_slice(&KLV_FILL);
            hits += 1;
        }
    }
    assert!(hits > 0);
    let f = open(&bytes[..]).unwrap();
    let track = &f.tracks[0];
    assert_eq!(track.samples.len(), 3);
    assert_eq!(track.wrapping, Wrapping::Clip);
    assert_eq!(f.read_sample(&bytes[..], 0, 2).unwrap(), pictures[2]);
}

/// Rewrites every frame-wrapped picture element of `bytes` as ST 384 line wrapping: a system
/// element (start of stored data / start of line), a fill data element before the first line,
/// then one picture element per stored line of `line` bytes.
fn line_wrap(bytes: &[u8], line: usize) -> Vec<u8> {
    line_wrap_with(bytes, line, &|_| None)
}

/// [`line_wrap`], with a VANC data element (ST 384 7.2, type 09h) of the samples `vanc(picture)`
/// after a "start of line" system element for stored line 9, before each picture's first line.
fn line_wrap_with(bytes: &[u8], line: usize, vanc: &dyn Fn(usize) -> Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut picture = 0;
    let mut at = 0;
    while at + 17 <= bytes.len() {
        let key: [u8; 16] = bytes[at..at + 16].try_into().unwrap();
        let (len, n) = crate::klv::ber_length(&bytes[at + 16..]).unwrap();
        let v = &bytes[at + 16 + n..at + 16 + n + len as usize];
        if key[12] == 0x15 && key[14] == 0x02 {
            for (l, chunk) in v.chunks(line).enumerate() {
                let mut sys = key;
                sys[5] = 0x05;
                sys[12..16].copy_from_slice(&[0x14, 0x01, if l == 0 { 0x01 } else { 0x02 }, 0x00]);
                let mut pack = (l as u32 + 1).to_be_bytes().to_vec();
                pack.extend_from_slice(&[0; 4]);
                crate::testing::klv(&mut out, sys, &pack);
                if l == 0 {
                    let mut data = key;
                    data[12..16].copy_from_slice(&[0x17, 0x01, 0x08, 0x01]);
                    crate::testing::klv(&mut out, data, &[0xEE; 4]);
                    if let Some(samples) = vanc(picture) {
                        let mut sys = key;
                        sys[5] = 0x05;
                        sys[12..16].copy_from_slice(&[0x14, 0x01, 0x02, 0x00]);
                        crate::testing::klv(&mut out, sys, &[0, 0, 0, 9, 0, 0, 0, 0]);
                        let mut data = key;
                        data[12..16].copy_from_slice(&[0x17, 0x02, 0x09, 0x01]);
                        crate::testing::klv(&mut out, data, &samples);
                    }
                }
                let mut pic = key;
                pic[14] = 0x04;
                crate::testing::klv(&mut out, pic, chunk);
            }
            picture += 1;
        } else {
            out.extend_from_slice(&bytes[at..at + 16 + n + len as usize]);
        }
        at += 16 + n + len as usize;
    }
    // the essence container label: line wrapping (ST 384 Table 5)
    for i in 0..out.len().saturating_sub(8) {
        if out[i..i + 6] == [0x0D, 0x01, 0x03, 0x01, 0x02, 0x05] && out[i + 7] == 0x01 {
            out[i + 7] = 0x03;
        }
    }
    out
}

#[test]
fn line_wrapped_uncompressed_pictures_are_gathered_line_by_line() {
    use crate::uncompressed::{Layout, decode};
    let layout = crate::UncompressedLayout::Uyvy8;
    let (bytes, pictures) = build_uncompressed(Pattern::Op1a, layout);
    let lined = line_wrap(&bytes, 96);
    assert!(lined.len() > bytes.len());
    let f = open(&lined[..]).unwrap();
    let t = f.tracks.iter().position(|t| t.kind == TrackKind::Picture).unwrap();
    let track = &f.tracks[t];
    assert_eq!((track.codec, track.wrapping, track.samples.len()), (Codec::Uncompressed, Wrapping::Line, 3), "{:?}", f.warnings);
    let pi = track.picture.as_ref().unwrap();
    let l = Layout::of(pi).unwrap();
    for (i, want) in pictures.iter().enumerate() {
        let s = f.read_sample(&lined[..], t, i).unwrap();
        assert_eq!(&s, want);
        let pic = decode(&l, pi, &s).unwrap();
        assert_eq!((pic.planes[0][0], pic.planes[2][0]), (16 + i as u16, 200 - i as u16));
    }
    // a picture with a line missing is left out; the system elements keep the others in step
    let first_line = lined.windows(4).position(|w| w == [0x15, 0x01, 0x04, 0x01]).unwrap() - 12;
    let mut short = lined[..first_line].to_vec();
    short.extend_from_slice(&lined[first_line + 16 + 4 + 96..]);
    let f = open(&short[..]).unwrap();
    let track = &f.tracks[t];
    assert_eq!(track.samples.len(), 2, "{:?}", f.warnings);
    assert_eq!(f.read_sample(&short[..], t, 0).unwrap(), pictures[1]);
    assert!(f.warnings.iter().any(|w| w.contains("1 line-wrapped picture(s)")), "{:?}", f.warnings);
    // without system elements, the lines are counted
    let mut bare = lined.clone();
    for i in 0..bare.len() - 16 {
        if bare[i..i + 4] == [0x06, 0x0E, 0x2B, 0x34] && bare[i + 12..i + 14] == [0x14, 0x01] {
            bare[i..i + 16].copy_from_slice(&KLV_FILL);
        }
    }
    let f = open(&bare[..]).unwrap();
    assert_eq!(f.tracks[t].samples.len(), 3, "{:?}", f.warnings);
    assert_eq!(f.read_sample(&bare[..], t, 2).unwrap(), pictures[2]);
}

/// 10-bit ST 291 words of a packet: ancillary data flag, DID / SDID / DC and user data words
/// with even parity in b8 and its inverse in b9, then the checksum.
fn st291_words(did: u8, sdid: u8, udw: &[u8]) -> Vec<u16> {
    let par = |b: u8| {
        let p = u16::from(b.count_ones() % 2 == 1);
        u16::from(b) | p << 8 | (1 - p) << 9
    };
    let mut w = vec![0, 0x3FF, 0x3FF, par(did), par(sdid), par(udw.len() as u8)];
    w.extend(udw.iter().map(|&b| par(b)));
    let sum = w[3..].iter().fold(0u16, |a, &x| a.wrapping_add(x & 0x1FF)) & 0x1FF;
    w.push(sum | (!sum >> 8 & 1) << 9);
    w
}

/// #1060: ST 291 packets in the line-wrapped VANC data elements of uncompressed pictures reach
/// `read_anc`: in the luma samples of 8-bit UYVY and of v210, with the stored line of the system
/// element before them and the frame of the start of stored data; a bad 10-bit checksum is not a
/// packet, and a file without VANC packets reads as ANC data with empty frames.
#[test]
fn line_wrapped_vanc_packets_are_read_as_anc() {
    let cdp = |seq: u8| vec![0x96, 0x69, 0x08, 0x4F, 0x43, 0x00, seq, 0x74];
    for (layout, line) in [(crate::UncompressedLayout::Uyvy8, 96), (crate::UncompressedLayout::V210, 128)] {
        let (bytes, _) = build_uncompressed(Pattern::Op1a, layout);
        let samples = |picture: usize, corrupt: bool| -> Option<Vec<u8>> {
            if picture == 1 {
                return None;
            }
            let mut words = st291_words(0x61, 0x01, &cdp(picture as u8));
            if corrupt {
                let last = words.len() - 1;
                words[last] ^= 1;
            }
            // the packet in the luma samples, colour difference samples at black
            let eight = layout == crate::UncompressedLayout::Uyvy8;
            // 8-bit samples carry the low 8 bits of each word
            let black = if eight { 0x80 } else { 0x200 };
            let mut seq: Vec<u16> = words.iter().flat_map(|&y| [black, if eight { y & 0xFF } else { y }]).collect();
            while !seq.len().is_multiple_of(6) {
                seq.push(black);
            }
            Some(match layout {
                crate::UncompressedLayout::Uyvy8 => seq.iter().map(|&w| w as u8).collect(),
                _ => seq.chunks(3).flat_map(|c| (u32::from(c[0]) | u32::from(c[1]) << 10 | u32::from(c[2]) << 20).to_le_bytes()).collect(),
            })
        };
        let lined = line_wrap_with(&bytes, line, &|p| samples(p, false));
        assert!(crate::has_anc(&lined[..lined.len().min(65_536)]), "{layout:?}");
        let a = crate::read_anc(&lined[..]).unwrap();
        assert_eq!(a.edit_rate, Rational::new(25, 1));
        assert_eq!(a.frames.len(), 3, "{layout:?} {:?}", a.warnings);
        assert!(a.frames[1].is_empty());
        for f in [0, 2] {
            let p = &a.frames[f];
            assert_eq!(p.len(), 1, "{layout:?} frame {f}: {p:?}");
            assert_eq!((p[0].line, p[0].wrapping, p[0].coding), (9, 1, if layout == crate::UncompressedLayout::Uyvy8 { 4 } else { 7 }));
            assert!(p[0].is_cdp());
            assert_eq!(p[0].user_data(), &cdp(f as u8)[..]);
        }
        assert_eq!(a.cdps().iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 2]);
        // the pictures still read
        let f = open(&lined[..]).unwrap();
        assert_eq!(f.tracks.iter().find(|t| t.kind == TrackKind::Picture).map(|t| t.samples.len()), Some(3));
        if layout == crate::UncompressedLayout::V210 {
            let bad = crate::read_anc(&line_wrap_with(&bytes, line, &|p| samples(p, true))[..]).unwrap();
            assert!(bad.frames.iter().all(Vec::is_empty), "{:?}", bad.frames);
        }
        let none = crate::read_anc(&line_wrap(&bytes, line)[..]).unwrap();
        assert_eq!(none.frames.len(), 3);
        assert!(none.cdps().is_empty());
    }
    // frame-wrapped uncompressed pictures have no ANC data
    let (bytes, _) = build_uncompressed(Pattern::Op1a, crate::UncompressedLayout::Uyvy8);
    assert!(!crate::has_anc(&bytes[..bytes.len().min(65_536)]));
    assert!(crate::read_anc(&bytes[..]).is_err());
}

#[test]
fn peek_reads_the_packages_of_op_atom_files_without_the_essence() {
    let clip = PackageIds::from_seed("clip", "Clip");
    let mut cfg = WriterConfig::new(Pattern::OpAtom, Rational::new(25, 1), PackageIds { file: Umid::from_seed(b"v1"), ..clip.clone() });
    cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 64, 36));
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..5 {
        w.push_picture(&vc3_like(i), FrameInfo::intra()).unwrap();
    }
    let v = w.finish().unwrap().into_inner();
    let a = write_opatom_pcm(
        &OpAtomPcm {
            sample_rate: 48_000,
            bits: 16,
            channels: 1,
            edit_rate: Some(Rational::new(25, 1)),
            ids: PackageIds { file: Umid::from_seed(b"a1"), ..clip.clone() },
            timecode: None,
        },
        &[0; 9600],
    )
    .unwrap();
    let pv = crate::peek(&v).unwrap();
    let pa = crate::peek(&a).unwrap();
    assert_eq!(pv.operational_pattern, OperationalPattern::Atom);
    assert_eq!(pv.material_package, Some(clip.material.0));
    assert_eq!(pa.material_package, pv.material_package);
    assert_eq!(pv.file_packages, vec![Umid::from_seed(b"v1").0]);
    assert_eq!(pa.file_packages, vec![Umid::from_seed(b"a1").0]);
    // only the header is needed: the file cut right after the header metadata still peeks
    let header_end = v.windows(4).position(|w| w == [0x00, 0x00, 0x02, 0x80]).unwrap();
    assert_eq!(crate::peek(&v[..header_end].to_vec()).unwrap(), pv);
    assert!(crate::peek(&b"not mxf".to_vec()).is_err());
    for cut in (0..v.len().min(4000)).step_by(13) {
        let _ = crate::peek(&v[..cut].to_vec());
    }
}

/// A minimal IMF JPEG 2000 codestream header (SOC, SIZ, COD, QCD) for a 3-component 10-bit
/// `w`×`h` picture, then a fake tile and EOC. `i` varies the payload.
fn imf_j2k(rsiz: u16, w: u32, h: u32, i: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0x4F, 0xFF, 0x51, 0x00, 38 + 9];
    v.extend_from_slice(&rsiz.to_be_bytes());
    for x in [w, h, 0, 0, w, h, 0, 0] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(&3u16.to_be_bytes());
    for _ in 0..3 {
        v.extend_from_slice(&[9, 1, 1]);
    }
    // COD: precincts, CPRL, 1 layer, MCT, 5 levels, 32×32 code-blocks, 9/7, 6 precinct sizes
    v.extend_from_slice(&[0xFF, 0x52, 0x00, 18, 0x01, 0x04, 0x00, 0x01, 0x01, 0x05, 0x03, 0x03, 0x00, 0x00, 0x77, 0x88, 0x88, 0x88, 0x88, 0x88]);
    // QCD: scalar expounded, 2 guard bits, 16 subbands
    v.extend_from_slice(&[0xFF, 0x5C, 0x00, 35, 0x42]);
    v.extend((0..16u16).flat_map(|b| (0x4000 + b).to_be_bytes()));
    v.extend_from_slice(&[0xFF, 0x90]);
    v.extend(std::iter::repeat_n(i as u8 ^ 0x6C, 200 + 5 * (i % 13)));
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

fn imf_picture_config(rate: Rational, rgb: bool) -> (WriterConfig, PackageIds) {
    let mut ids = PackageIds::from_seed("imf-pic", "Main");
    ids.file = Umid::dcinema([0x31; 16]);
    let mut cfg = WriterConfig::new(Pattern::Imf, rate, ids.clone());
    let rsiz = 0x0411; // IMF 2K lossy, mainlevel 1, sublevel 1
    let mut p = PictureDesc::new(PictureCoding::J2k { rsiz }, 1920, 1080);
    p.depth = 10;
    p.rgb = rgb;
    p.j2k = J2kSubDescriptor::from_codestream(&imf_j2k(rsiz, 1920, 1080, 0));
    cfg.picture = Some(p);
    cfg.timecode = Some(StartTimecode { frames: 0, rate, drop_frame: false });
    (cfg, ids)
}

/// #931: an IMF App 2E picture track file (ST 2067-5, ST 2067-21): OP1a, frame-wrapped JPEG 2000,
/// the IMF coding label, the RGBA descriptor with a complete JPEG 2000 sub-descriptor, essence
/// partitions of 60 s, each followed by an index partition, and the RIP listing them all.
#[test]
fn imf_picture_track_file() {
    let rate = Rational::new(1, 1); // 60 frames per essence partition
    let (cfg, ids) = imf_picture_config(rate, true);
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let n = 130;
    for i in 0..n {
        w.push_picture(&imf_j2k(0x0411, 1920, 1080, i), FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
    let t = &m.tracks[0];
    assert_eq!(t.codec, Codec::Jpeg2000);
    assert_eq!(t.wrapping, Wrapping::Frame);
    assert_eq!(t.duration, Some(n as i64));
    assert!(t.picture.as_ref().unwrap().rgba);
    for i in [0, 59, 60, 61, 119, 120, 129] {
        assert_eq!(m.read_sample(&f, 0, i).unwrap(), imf_j2k(0x0411, 1920, 1080, i), "frame {i}");
    }
    // header, then essence (60) / index / essence (60) / index / essence (10) / index, footer
    let layout: Vec<(PartitionKind, u32, u32)> = m.partitions.iter().map(|p| (p.kind, p.body_sid, p.index_sid)).collect();
    use PartitionKind::*;
    assert_eq!(layout, vec![(Header, 0, 0), (Body, 1, 0), (Body, 0, 2), (Body, 1, 0), (Body, 0, 2), (Body, 1, 0), (Body, 0, 2), (Footer, 0, 0)]);
    let footer = m.partitions.last().unwrap();
    assert_eq!((footer.header_byte_count, footer.index_byte_count), (0, 0));
    for p in &m.partitions {
        assert_eq!(p.status, 4, "closed complete partitions");
        assert_eq!(p.kag_size, 1);
        if p.kind != Footer {
            assert_eq!(p.footer_partition, footer.this_partition);
        }
    }
    // essence partitions continue the essence stream: body offsets 0, 60 frames, 120 frames
    let offsets: Vec<u64> = m.partitions.iter().filter(|p| p.body_sid == 1).map(|p| p.body_offset).collect();
    assert_eq!(offsets[0], 0);
    assert!(offsets[1] > 0 && offsets[2] > offsets[1]);
    // ST 2067-5 5.1.1: at least 8 KiB of fill after the header metadata (a fill KLV, BER length 4)
    let fill_key = KLV_FILL;
    let header_end = usize::try_from(m.partitions[1].this_partition).unwrap();
    let at = f[..header_end].windows(16).rposition(|w| w == fill_key).unwrap();
    assert!(header_end - at - 20 >= 8192, "fill {}", header_end - at - 20);
    // the primary package is the file package
    assert!(f.windows(32).any(|w| w == ids.file.0));
    assert!(f.windows(4).any(|w| w == [0x3B, 0x08, 0x00, 0x10]));
    // ST 2067-21 7.1.2: mode P1 frame wrapping (ST 422 Table 2)
    assert!(f.windows(16).any(|w| w == labels::essence_container(0x0D, 0x0C, 0x06, 0x00)));
    // ST 422 Table 3 IMF coding label: 2K lossy (byte 15 = 02), mainlevel 1 sublevel 1 (byte 16 = 03)
    let label = [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x04, 0x01, 0x02, 0x02, 0x03, 0x01, 0x02, 0x03];
    assert!(f.windows(16).any(|w| w == label));
    // the sub-descriptor carries COD, QCD and the R10 G10 B10 layout
    assert!(f.windows(10).any(|w| w == [0x01, 0x04, 0x00, 0x01, 0x01, 0x05, 0x03, 0x03, 0x00, 0x00]));
    assert!(f.windows(8).any(|w| w == [b'R', 10, b'G', 10, b'B', 10, 0, 0]));
    // truncation never panics
    for cut in (0..f.len()).step_by(211) {
        if let Ok(m) = open(&f[..cut]) {
            for i in 0..m.tracks.first().map_or(0, |t| t.samples.len()).min(8) {
                let _ = m.read_sample(&f[..cut], 0, i);
            }
        }
    }
}

/// #931: Y'C'BC'R pictures get a CDCI descriptor with ST 2067-21 narrow-range levels.
#[test]
fn imf_ycbcr_picture_uses_cdci() {
    let rate = Rational::new(24, 1);
    let (mut cfg, _) = imf_picture_config(rate, false);
    if let Some(p) = cfg.picture.as_mut() {
        p.subsampling = (2, 1);
    }
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..3 {
        w.push_picture(&imf_j2k(0x0411, 1920, 1080, i), FrameInfo::intra()).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let p = m.tracks[0].picture.as_ref().unwrap();
    assert!(!p.rgba);
    // one essence partition, one index partition
    assert_eq!(m.partitions.iter().filter(|p| p.kind == PartitionKind::Body).count(), 2);
    // 10-bit: black 64, white 940, colour range 897
    let u32s = |v: u32| v.to_be_bytes();
    assert!(f.windows(4).any(|w| w == u32s(64)));
    assert!(f.windows(4).any(|w| w == u32s(940)));
    assert!(f.windows(4).any(|w| w == u32s(897)));
    assert!(f.windows(8).any(|w| w == [b'Y', 10, b'U', 10, b'V', 10, 0, 0]));
}

/// #931: an IMF sound track file: clip-wrapped 24-bit PCM at the sample rate, the IMF MCA
/// channel assignment, the reference image edit rate and a constant-rate index partition.
#[test]
fn imf_sound_track_file() {
    let mut ids = PackageIds::from_seed("imf-snd", "Main");
    ids.file = Umid::dcinema([0x32; 16]);
    let mut cfg = WriterConfig::new(Pattern::Imf, Rational::new(48_000, 1), ids);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 24 }];
    cfg.reference_image_edit_rate = Some(Rational::new(24, 1));
    cfg.reference_audio_alignment_level = Some(-20);
    let group = [0x41; 16];
    let label = |sym: &str, channel: Option<u32>, g: Option<[u8; 16]>| McaLabel {
        dictionary: [0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0D, 0x03, 0x02, if channel.is_some() { 1 } else { 2 }, 0x01, 0, 0, 0, 0],
        link: if channel.is_none() { group } else { [channel.unwrap_or(0) as u8; 16] },
        symbol: sym.into(),
        name: sym.into(),
        channel,
        language: None,
        group: g,
        titles: None,
    };
    let mut st = label("ST", None, None);
    st.titles = Some(McaTitles { title: "Main".into(), title_version: "1".into(), content_kind: "PRM".into(), element_kind: "FULL".into() });
    cfg.mca = vec![st, label("chL", Some(1), Some(group)), label("chR", Some(2), Some(group))];
    let mut w = MxfWriter::new(Cursor::new(Vec::new()), cfg).unwrap();
    let frames = 48_000 + 123;
    let mut pcm = Vec::new();
    for k in 0..frames {
        for c in 0..2i32 {
            let s = (k as i32 * 5 + c * 1_000_000) % 8_000_000;
            pcm.extend_from_slice(&s.to_le_bytes()[..3]);
        }
    }
    for chunk in pcm.chunks(6 * 1001) {
        w.push_sound(0, chunk).unwrap();
    }
    let f = w.finish().unwrap().into_inner();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
    let t = &m.tracks[0];
    assert_eq!(t.kind, TrackKind::Sound);
    assert_eq!(t.wrapping, Wrapping::Clip);
    assert_eq!(t.duration, Some(frames as i64));
    assert_eq!(t.stored_sample_frames(), frames as u64);
    let got = m.read_pcm(&f, 0, 0, frames).unwrap();
    for k in (0..frames).step_by(101) {
        for c in 0..2 {
            let want = (k as i32 * 5 + c as i32 * 1_000_000) % 8_000_000;
            assert_eq!(got[c][k], want as f32 / 8_388_608.0);
        }
    }
    // header, essence, index partition, footer
    let layout: Vec<(PartitionKind, u32, u32)> = m.partitions.iter().map(|p| (p.kind, p.body_sid, p.index_sid)).collect();
    assert_eq!(layout, vec![(PartitionKind::Header, 0, 0), (PartitionKind::Body, 1, 0), (PartitionKind::Body, 0, 2), (PartitionKind::Footer, 0, 0)]);
    assert!(f.windows(16).any(|w| w == labels::CA_IMF_MCA));
    // the soundfield group's MCA title items (ST 2067-2 Table 7)
    let utf16be = |t: &str| -> Vec<u8> { t.encode_utf16().flat_map(|u| u.to_be_bytes()).collect() };
    for (k, t) in [(7u16, "Main"), (9, "PRM"), (10, "FULL")] {
        let mut item = (labels::TAG_MCA + k).to_be_bytes().to_vec();
        item.extend_from_slice(&(2 * t.len() as u16).to_be_bytes());
        item.extend(utf16be(t));
        assert!(f.windows(item.len()).any(|w| w == item), "MCA item {k}");
    }
    // reference image edit rate 24/1, alignment level -20
    let mut rier = labels::TAG_REF_IMAGE_EDIT_RATE.to_be_bytes().to_vec();
    rier.extend_from_slice(&[0, 8, 0, 0, 0, 24, 0, 0, 0, 1]);
    assert!(f.windows(rier.len()).any(|w| w == rier));
    let mut ral = (labels::TAG_REF_IMAGE_EDIT_RATE + 1).to_be_bytes().to_vec();
    ral.extend_from_slice(&[0, 1, (-20i8) as u8]);
    assert!(f.windows(ral.len()).any(|w| w == ral));
}

/// A real IMF track file (`CFA4_IMF_SAMPLE`, e.g. the track files of the Photon test IMP
/// MERIDIAN_Netflix_Photon_161006) opens cleanly with the partition layout this writer uses:
/// header, essence, index partition (BodySID 1, IndexSID 2), footer, all closed and complete.
#[test]
fn imf_sample_has_the_written_layout() {
    let Ok(path) = std::env::var("CFA4_IMF_SAMPLE") else { return };
    let f = std::fs::read(&path).unwrap();
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
    let layout: Vec<(PartitionKind, u32, u32, u8)> = m.partitions.iter().map(|p| (p.kind, p.body_sid, p.index_sid, p.status)).collect();
    use PartitionKind::*;
    assert_eq!(layout[..3], [(Header, 0, 0, 4), (Body, 1, 0, 4), (Body, 0, 2, 4)]);
    assert_eq!(layout.last(), Some(&(Footer, 0, 0, 4)));
}

#[test]
fn imf_rejects_bad_configs() {
    let ids = PackageIds::from_seed("x", "x");
    let sound = |rate: i32, sd: SoundDesc| {
        let mut cfg = WriterConfig::new(Pattern::Imf, Rational::new(rate, 1), ids.clone());
        cfg.sound = vec![sd];
        MxfWriter::new(Cursor::new(Vec::new()), cfg).err().map(|e| e.to_string())
    };
    assert!(sound(48_000, SoundDesc { sample_rate: 48_000, channels: 2, bits: 16 }).unwrap().contains("24-bit"));
    assert!(sound(48_000, SoundDesc { sample_rate: 44_100, channels: 2, bits: 24 }).is_some());
    assert!(sound(24, SoundDesc { sample_rate: 48_000, channels: 2, bits: 24 }).unwrap().contains("sample rate"));
    assert!(sound(96_000, SoundDesc { sample_rate: 96_000, channels: 2, bits: 24 }).is_none());
    // picture and sound in one file
    let (mut cfg, _) = imf_picture_config(Rational::new(24, 1), true);
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 24 }];
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
    // RDD 9 is not IMF
    let (mut cfg, _) = imf_picture_config(Rational::new(24, 1), true);
    cfg.rdd9 = true;
    assert!(MxfWriter::new(Cursor::new(Vec::new()), cfg).is_err());
}

/// #931: hostile codestream headers give no sub-descriptor instead of a panic.
#[test]
fn j2k_sub_descriptor_from_hostile_codestreams() {
    let good = imf_j2k(0x0411, 64, 32, 0);
    let d = J2kSubDescriptor::from_codestream(&good).unwrap();
    assert_eq!((d.rsiz, d.xsiz, d.ysiz, d.components.len()), (0x0411, 64, 32, 3));
    assert_eq!(d.coding_style.as_deref().map(<[u8]>::len), Some(16));
    assert_eq!(d.quantization.as_deref().map(<[u8]>::len), Some(33));
    for cut in 0..good.len() {
        let _ = J2kSubDescriptor::from_codestream(&good[..cut]);
    }
    for i in 0..good.len().min(120) {
        for b in [0x00, 0xFF, 0x7F] {
            let mut v = good.clone();
            v[i] = b;
            let _ = J2kSubDescriptor::from_codestream(&v);
        }
    }
    assert!(J2kSubDescriptor::from_codestream(b"not j2k").is_none());
}
