//! Synthetic files: a small OP1a writer (enough of ST 377-1 to exercise the demuxer without
//! ffmpeg) and robustness checks.

use crate::testing::*;
use crate::*;

/// An OP1a file: `n` frame-wrapped content packages, each a picture element of `pic(i)` and a
/// 16-bit stereo sound element of 1920 frames (sample k of the file = k, -k on the channels);
/// a footer index with temporal offsets `to` and flags `flags`.
fn build(n: usize, pic: impl Fn(usize) -> Vec<u8>, to: &[i8], flags: &[u8]) -> Vec<u8> {
    let mut md = Vec::new();
    // primer: no dynamic tags
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &OP1A);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut cs = LocalSet::new(2);
    cs.p(0x1901, &batch(&[&uuid(3), &uuid(4)]));
    klv(&mut md, set_key(0x18), &cs.0);
    // material package: timecode, picture, sound
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4402, &[0, b'M', 0, b'P']).p(0x4403, &batch(&[&uuid(10), &uuid(11), &uuid(12)]));
    klv(&mut md, set_key(0x36), &mp.0);
    let d = n as i64;
    klv(&mut md, set_key(0x3B), &track(10, 1, 0, 20));
    klv(&mut md, set_key(0x0F), &sequence(20, TC_DEF, d, 30));
    let mut tc = LocalSet::new(30);
    tc.p(0x0201, &TC_DEF).p(0x0202, &d.to_be_bytes()).p(0x1501, &90_000i64.to_be_bytes()).p(0x1502, &25u16.to_be_bytes()).p(0x1503, &[0]);
    klv(&mut md, set_key(0x14), &tc.0);
    klv(&mut md, set_key(0x3B), &track(11, 2, 0, 21));
    klv(&mut md, set_key(0x0F), &sequence(21, PICTURE_DEF, d, 31));
    klv(&mut md, set_key(0x11), &source_clip(31, PICTURE_DEF, d, umid(2), 2));
    klv(&mut md, set_key(0x3B), &track(12, 3, 0, 22));
    klv(&mut md, set_key(0x0F), &sequence(22, SOUND_DEF, d, 32));
    klv(&mut md, set_key(0x11), &source_clip(32, SOUND_DEF, d, umid(2), 3));
    // file package
    let mut fp = LocalSet::new(4);
    fp.p(0x4401, &umid(2)).p(0x4403, &batch(&[&uuid(13), &uuid(14)])).p(0x4701, &uuid(40));
    klv(&mut md, set_key(0x37), &fp.0);
    klv(&mut md, set_key(0x3B), &track(13, 2, 0x1501_0500, 23));
    klv(&mut md, set_key(0x0F), &sequence(23, PICTURE_DEF, d, 33));
    klv(&mut md, set_key(0x11), &source_clip(33, PICTURE_DEF, d, [0; 32], 0));
    klv(&mut md, set_key(0x3B), &track(14, 3, 0x1601_0300, 24));
    klv(&mut md, set_key(0x0F), &sequence(24, SOUND_DEF, d, 34));
    klv(&mut md, set_key(0x11), &source_clip(34, SOUND_DEF, d, [0; 32], 0));
    let mut mdesc = LocalSet::new(40);
    mdesc.p(0x3001, &[0, 0, 0, 25, 0, 0, 0, 1]).p(0x3F01, &batch(&[&uuid(41), &uuid(42)]));
    klv(&mut md, set_key(0x44), &mdesc.0);
    let mut cdci = LocalSet::new(41);
    cdci.p(0x3006, &2u32.to_be_bytes())
        .p(0x3203, &64u32.to_be_bytes())
        .p(0x3202, &32u32.to_be_bytes())
        .p(0x320E, &[0, 0, 0, 2, 0, 0, 0, 1])
        .p(0x3301, &8u32.to_be_bytes())
        .p(0x3201, &ul(&[0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x0A, 0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03, 0x00]));
    klv(&mut md, set_key(0x28), &cdci.0);
    let mut wave = LocalSet::new(42);
    wave.p(0x3006, &3u32.to_be_bytes())
        .p(0x3D03, &[0, 0, 0xBB, 0x80, 0, 0, 0, 1])
        .p(0x3D07, &2u32.to_be_bytes())
        .p(0x3D01, &16u32.to_be_bytes())
        .p(0x3D0A, &4u16.to_be_bytes());
    klv(&mut md, set_key(0x48), &wave.0);
    let mut ecd = LocalSet::new(5);
    ecd.p(0x2701, &umid(2)).p(0x3F07, &1u32.to_be_bytes()).p(0x3F06, &2u32.to_be_bytes());
    klv(&mut md, set_key(0x23), &ecd.0);

    let mut f = partition(2, 0, 0, 0, md.len() as u64);
    f.extend_from_slice(&md);
    let body_at = f.len() as u64;
    f.extend_from_slice(&partition(3, body_at, 1, 0, 0));
    let mut offsets = Vec::new();
    let mut cp_base = None;
    for i in 0..n {
        let at = f.len() as u64;
        let base = *cp_base.get_or_insert(at);
        offsets.push(at - base);
        klv(&mut f, PIC_KEY, &pic(i));
        let mut snd = Vec::new();
        for k in 0..1920 {
            let s = ((i * 1920 + k) % 30000) as i16;
            snd.extend_from_slice(&s.to_le_bytes());
            snd.extend_from_slice(&(-s).to_le_bytes());
        }
        klv(&mut f, SND_KEY, &snd);
    }
    let footer_at = f.len() as u64;
    f.extend_from_slice(&partition(4, footer_at, 0, 2, 0));
    let mut ix = LocalSet::new(6);
    let mut entries = Vec::new();
    for i in 0..n {
        let mut e = vec![to.get(i).copied().unwrap_or(0) as u8, 0, flags.get(i).copied().unwrap_or(0)];
        e.extend_from_slice(&offsets[i].to_be_bytes());
        entries.push(e);
    }
    let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
    ix.p(0x3F0B, &[0, 0, 0, 25, 0, 0, 0, 1])
        .p(0x3F0C, &0i64.to_be_bytes())
        .p(0x3F0D, &(n as i64).to_be_bytes())
        .p(0x3F06, &2u32.to_be_bytes())
        .p(0x3F07, &1u32.to_be_bytes())
        .p(0x3F0A, &batch(&refs));
    klv(&mut f, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00]), &ix.0);
    f
}

/// Stored order I0 P3 B1 B2 P6 B4 B5: display d is stored at d + TO[d].
fn ipbb() -> (Vec<i8>, Vec<u8>) {
    (vec![0, 1, 1, -2, 1, 1, -2], vec![0xC0, 0x22, 0x33, 0x33, 0x22, 0x33, 0x33])
}

fn prores_like(i: usize) -> Vec<u8> {
    let mut v = vec![0, 0, 0, 40];
    v.extend_from_slice(b"icpf");
    v.extend(std::iter::repeat_n(i as u8, 32));
    v
}

#[test]
fn opens_synthetic_op1a() {
    let (to, flags) = ipbb();
    let f = build(7, |i| vec![i as u8; 100 + i], &to, &flags);
    let m = open(&f).unwrap();
    assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
    assert_eq!(m.operational_pattern.name(), "OP1a");
    assert_eq!(m.material_package_name.as_deref(), Some("MP"));
    let tc = m.timecode.unwrap();
    assert_eq!((tc.start, tc.rounded_base, tc.drop_frame), (90_000, 25, false));
    assert_eq!(tc.format(), "01:00:00:00");
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.track_number, 0x1501_0500);
    assert_eq!(t.edit_rate, Rational::new(25, 1));
    assert_eq!(t.duration, Some(7));
    assert_eq!(t.samples.len(), 7);
    let p = t.picture.as_ref().unwrap();
    assert_eq!((p.frame_width(), p.frame_height()), (64, 32));
    assert_eq!(t.codec, Codec::ProRes { profile: Some(3) }, "coding label wins (the bytes do not sniff as anything)");
    // presentation order from the temporal offsets
    let pts: Vec<i64> = t.samples.iter().map(|s| s.pts).collect();
    assert_eq!(pts, vec![0, 3, 1, 2, 6, 4, 5]);
    assert_eq!(t.display_order, vec![0, 2, 3, 1, 5, 6, 4]);
    assert!(t.temporal_offsets);
    assert_eq!(t.sample_at(3), Some(1));
    // the index marks only the first picture random-access (ProRes would be intra; this label says ProRes so all are key)
    assert!(t.samples.iter().all(|s| s.key));
    // sample bytes
    for i in 0..7 {
        assert_eq!(m.read_sample(&f, v, i).unwrap(), vec![i as u8; 100 + i]);
    }
    // sound: 7 × 1920 frames, sample k = k on the left, -k on the right
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    let s = &m.tracks[a];
    assert_eq!(s.codec, Codec::Pcm);
    assert_eq!(s.stored_sample_frames(), 7 * 1920);
    assert_eq!(s.samples_per_edit_unit(), (48000, 25));
    let pcm = m.read_pcm(&f, a, 1900, 50).unwrap();
    for k in 0..50 {
        assert_eq!(pcm[0][k], (1900 + k) as f32 / 32768.0);
        assert_eq!(pcm[1][k], -((1900 + k) as f32) / 32768.0);
    }
    // past the end: zeros
    let tail = m.read_pcm(&f, a, 7 * 1920 - 2, 4).unwrap();
    assert_eq!(tail[0][2..], [0.0, 0.0]);
}

#[test]
fn key_frames_from_index_flags_for_long_gop() {
    let (to, flags) = ipbb();
    // AVC-looking essence (start code + AUD) and no coding label: sniffed as AVC
    let f = build(7, |i| vec![0, 0, 0, 1, 9, 0x10, i as u8], &to, &flags);
    // replace the ProRes coding label so the codec comes from the bytes
    let mut f2 = f.clone();
    let lbl = [0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03, 0x00];
    let at = f2.windows(8).position(|w| w == lbl).unwrap();
    f2[at..at + 8].copy_from_slice(&[0x04, 0x01, 0x02, 0x02, 0x01, 0x31, 0x40, 0x01]);
    let m = open(&f2).unwrap();
    let t = &m.tracks[m.track_of_kind(TrackKind::Picture).unwrap()];
    assert_eq!(t.codec, Codec::Avc { intra: false });
    let keys: Vec<bool> = t.samples.iter().map(|s| s.key).collect();
    assert_eq!(keys, vec![true, false, false, false, false, false, false]);
    assert_eq!(t.sync_before(5), 0);
    assert!(t.indexed);
    let b: Vec<bool> = t.samples.iter().map(|s| s.b_picture).collect();
    assert_eq!(b, vec![false, false, true, true, false, true, true]);
}

#[test]
fn missing_temporal_offsets_with_b_pictures_ask_for_reordering() {
    let (_, flags) = ipbb();
    let m = open(&build(7, prores_like, &[], &flags)).unwrap();
    let t = &m.tracks[0];
    assert!(!t.temporal_offsets);
    // ProRes is intra-only: the B flags are ignored for key frames, reordering still reported
    assert!(t.samples.iter().all(|s| s.key));
}

#[test]
fn not_mxf_and_empty() {
    assert!(matches!(open(&b"RIFF....WAVE".to_vec()), Err(Error::NotMxf(_))));
    assert!(open(&Vec::new()).is_err());
    assert!(!sniff(b"\x1aE\xdf\xa3"));
}

#[test]
fn truncation_never_panics_and_keeps_whole_frames() {
    let (to, flags) = ipbb();
    let f = build(7, prores_like, &to, &flags);
    let mut opened = 0;
    for cut in (0..f.len()).step_by(37) {
        let part = f[..cut].to_vec();
        if let Ok(m) = open(&part) {
            opened += 1;
            for (ti, t) in m.tracks.iter().enumerate() {
                for i in 0..t.samples.len() {
                    // every listed sample is fully inside the file
                    assert!(m.read_sample(&part, ti, i).is_ok(), "cut {cut} track {ti} sample {i}");
                }
                if t.sound.is_some() {
                    let _ = m.read_pcm(&part, ti, 0, 5000);
                }
            }
        }
    }
    assert!(opened > 10, "truncated files with complete metadata still open ({opened})");
}

#[test]
fn mutation_never_panics() {
    let (to, flags) = ipbb();
    let f = build(5, prores_like, &to, &flags);
    let mut seed = 0x1234_5678u64;
    for _ in 0..600 {
        let mut g = f.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % g.len();
            g[at] = (seed >> 32) as u8;
        }
        if let Ok(m) = open(&g) {
            for (ti, t) in m.tracks.iter().enumerate() {
                for i in 0..t.samples.len().min(8) {
                    let _ = m.read_sample(&g, ti, i);
                }
                let _ = m.read_pcm(&g, ti, 0, 100);
            }
        }
    }
}

#[test]
fn drop_frame_timecode_format() {
    let tc = Timecode { start: 107_892, rounded_base: 30, drop_frame: true };
    assert_eq!(tc.format(), "01:00:00;00");
    let tc = Timecode { start: 1800, rounded_base: 30, drop_frame: true };
    assert_eq!(tc.format(), "00:01:00;02");
    let tc = Timecode { start: 90_000 + 25 * 61 + 3, rounded_base: 25, drop_frame: false };
    assert_eq!(tc.format(), "01:01:01:03");
}

#[test]
fn hostile_pcm_requests_fail_before_allocation() {
    let (to, flags) = ipbb();
    let bytes = build(5, prores_like, &to, &flags);
    let mut file = open(&bytes).unwrap();
    let track = file.track_of_kind(TrackKind::Sound).unwrap();
    file.tracks[track].sound.as_mut().unwrap().channels = u32::MAX;
    assert!(file.read_pcm(&bytes, track, 0, 100).is_err());
    file.tracks[track].sound.as_mut().unwrap().channels = 2;
    file.tracks[track].sound.as_mut().unwrap().block_align = u32::MAX;
    assert!(file.read_pcm(&bytes, track, 0, 100).is_err());
    assert!(file.read_pcm(&bytes, track, 0, usize::MAX).is_err());
    assert!(crate::decode_pcm(&[], SoundFormat::Pcm, usize::MAX, 16, 4).is_err());
}

#[test]
fn a_corrupt_index_position_cannot_allocate_a_giant_sparse_table() {
    let (to, flags) = ipbb();
    let mut bytes = build(5, prores_like, &to, &flags);
    let prefix = [0x3f, 0x0c, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0];
    let at = bytes.windows(prefix.len()).position(|w| w == prefix).unwrap();
    bytes[at + 4..at + 12].copy_from_slice(&49_999_999i64.to_be_bytes());
    let file = open(&bytes).unwrap();
    assert_eq!(file.tracks[file.track_of_kind(TrackKind::Picture).unwrap()].samples.len(), 5);
}

const IAB_KEY: [u8; 16] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x02, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x16, 0x01, 0x0D, 0x01];

/// A fake IABitstream frame of `body` bytes (the demuxer only reads the segment framing).
fn iab_frame(i: usize, body: usize) -> Vec<u8> {
    let mut f = vec![0x01, 0, 0, 0, 2, 0xAB, i as u8, 0x02];
    f.extend_from_slice(&(body as u32).to_be_bytes());
    f.extend(std::iter::repeat_n(i as u8, body));
    f
}

#[derive(Clone, Copy, PartialEq)]
enum IabWrap {
    Frame,
    ClipIndexed,
    ClipBare,
}

/// An OP1a file with one IAB sound track of `frames` at 24 fps; `iab_descriptor` false writes a
/// generic sound descriptor (the essence bytes identify it) and a plain sound element key.
fn build_iab(frames: &[Vec<u8>], wrap: IabWrap, iab_descriptor: bool) -> Vec<u8> {
    let key = if iab_descriptor { IAB_KEY } else { SND_KEY };
    let number = u32::from_be_bytes([key[12], key[13], key[14], key[15]]);
    let mut md = Vec::new();
    klv(&mut md, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00]), &batch(&[]));
    let mut pre = LocalSet::new(1);
    pre.p(0x3B03, &uuid(2)).p(0x3B09, &OP1A);
    klv(&mut md, set_key(0x2F), &pre.0);
    let mut cs = LocalSet::new(2);
    cs.p(0x1901, &batch(&[&uuid(3), &uuid(4)]));
    klv(&mut md, set_key(0x18), &cs.0);
    let d = frames.len() as i64;
    let mut mp = LocalSet::new(3);
    mp.p(0x4401, &umid(1)).p(0x4403, &batch(&[&uuid(12)]));
    klv(&mut md, set_key(0x36), &mp.0);
    klv(&mut md, set_key(0x3B), &track(12, 3, 0, 22));
    klv(&mut md, set_key(0x0F), &sequence(22, SOUND_DEF, d, 32));
    klv(&mut md, set_key(0x11), &source_clip(32, SOUND_DEF, d, umid(2), 3));
    let mut fp = LocalSet::new(4);
    fp.p(0x4401, &umid(2)).p(0x4403, &batch(&[&uuid(14)])).p(0x4701, &uuid(42));
    klv(&mut md, set_key(0x37), &fp.0);
    klv(&mut md, set_key(0x3B), &track(14, 3, number, 24));
    klv(&mut md, set_key(0x0F), &sequence(24, SOUND_DEF, d, 34));
    klv(&mut md, set_key(0x11), &source_clip(34, SOUND_DEF, d, [0; 32], 0));
    let mut desc = LocalSet::new(42);
    desc.p(0x3006, &3u32.to_be_bytes()).p(0x3D03, &[0, 0, 0xBB, 0x80, 0, 0, 0, 1]).p(0x3D07, &0u32.to_be_bytes()).p(0x3D01, &24u32.to_be_bytes());
    klv(&mut md, set_key(if iab_descriptor { 0x7B } else { 0x42 }), &desc.0);
    let mut ecd = LocalSet::new(5);
    ecd.p(0x2701, &umid(2)).p(0x3F07, &1u32.to_be_bytes()).p(0x3F06, &2u32.to_be_bytes());
    klv(&mut md, set_key(0x23), &ecd.0);

    let mut f = partition(2, 0, 0, 0, md.len() as u64);
    f.extend_from_slice(&md);
    let body_at = f.len() as u64;
    f.extend_from_slice(&partition(3, body_at, 1, 0, 0));
    let mut offsets = Vec::new();
    if wrap == IabWrap::Frame {
        let base = f.len() as u64;
        for fr in frames {
            offsets.push(f.len() as u64 - base);
            klv(&mut f, key, fr);
        }
    } else {
        let mut all = Vec::new();
        for fr in frames {
            offsets.push(all.len() as u64);
            all.extend_from_slice(fr);
        }
        klv(&mut f, key, &all);
    }
    let footer_at = f.len() as u64;
    f.extend_from_slice(&partition(4, footer_at, 0, 2, 0));
    if wrap != IabWrap::ClipBare {
        let mut ix = LocalSet::new(6);
        let entries: Vec<Vec<u8>> = offsets
            .iter()
            .map(|o| {
                let mut e = vec![0, 0, 0x80];
                e.extend_from_slice(&o.to_be_bytes());
                e
            })
            .collect();
        let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
        ix.p(0x3F0B, &[0, 0, 0, 24, 0, 0, 0, 1])
            .p(0x3F0C, &0i64.to_be_bytes())
            .p(0x3F0D, &d.to_be_bytes())
            .p(0x3F06, &2u32.to_be_bytes())
            .p(0x3F07, &1u32.to_be_bytes())
            .p(0x3F0A, &batch(&refs));
        klv(&mut f, ul(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00]), &ix.0);
    }
    f
}

#[test]
fn iab_tracks_give_one_sample_per_frame_however_wrapped() {
    let frames: Vec<Vec<u8>> = (0..6).map(|i| iab_frame(i, 30 + 7 * i)).collect();
    for wrap in [IabWrap::Frame, IabWrap::ClipIndexed, IabWrap::ClipBare] {
        for desc in [true, false] {
            let f = build_iab(&frames, wrap, desc);
            let m = open(&f).unwrap();
            let a = m.track_of_kind(TrackKind::Sound).unwrap();
            let t = &m.tracks[a];
            assert_eq!(t.codec, Codec::Iab, "wrap {} descriptor {desc}", wrap as u8);
            assert_eq!(t.sound_format, SoundFormat::Iab);
            assert_eq!(t.samples.len(), 6, "wrap {} descriptor {desc}", wrap as u8);
            assert_eq!(t.wrapping, if wrap == IabWrap::Frame { Wrapping::Frame } else { Wrapping::Clip });
            for (i, fr) in frames.iter().enumerate() {
                assert_eq!(&m.read_sample(&f, a, i).unwrap(), fr);
            }
            assert!(m.read_pcm(&f, a, 0, 10).is_err(), "IAB is not PCM");
        }
    }
    // a damaged frame length ends the split there, never past the element
    let mut f = build_iab(&frames, IabWrap::ClipBare, true);
    let at = f.windows(8).position(|w| w == [0x01, 0, 0, 0, 2, 0xAB, 3, 0x02]).unwrap();
    f[at + 8] = 0x7F;
    let m = open(&f).unwrap();
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    assert_eq!(m.tracks[a].samples.len(), 3);
}

#[test]
fn pcm_that_happens_to_look_like_iab_stays_pcm() {
    // a Broadcast Wave track whose first bytes read 01 00 00 00 00 02
    let f = build(2, |i| vec![i as u8; 10], &[], &[]);
    let m = open(&f).unwrap();
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    assert_eq!(m.tracks[a].codec, Codec::Pcm);
}

const DCST: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST"><Id>urn:uuid:11111111-1111-1111-1111-111111111111</Id></SubtitleReel>"#;

#[test]
fn timed_text_track_file_gives_the_document_and_its_resources() {
    let font = b"OTTO fake font bytes".as_slice();
    let png = b"\x89PNG fake".as_slice();
    let f = build_timed_text(DCST, &[("application/x-font-opentype", font), ("image/png", png)]);
    assert!(is_timed_text(&f));
    assert!(sniff(&f));
    let tt = open_timed_text(&f).unwrap();
    assert_eq!(tt.edit_rate, Rational::new(24, 1));
    assert_eq!(tt.duration, Some(240));
    assert_eq!(tt.resource_id, Some([0x11; 16]));
    assert_eq!(tt.ucs_encoding.as_deref(), Some("UTF-8"));
    assert_eq!(tt.namespace.as_deref(), Some("http://www.smpte-ra.org/schemas/428-7/2010/DCST"));
    assert_eq!(tt.read_document(&f).unwrap(), DCST);
    assert_eq!(tt.resources.len(), 2);
    assert_eq!(tt.resources[0].id, [0x20; 16]);
    assert_eq!(tt.resources[0].mime.as_deref(), Some("application/x-font-opentype"));
    assert_eq!(tt.resources[1].stream_id, 3);
    assert_eq!(tt.read_resource(&f, 0).unwrap(), font);
    assert_eq!(tt.read_resource(&f, 1).unwrap(), png);
    assert!(tt.read_resource(&f, 2).is_err());
    assert!(tt.warnings.is_empty(), "{:?}", tt.warnings);
    // the picture/sound demuxer says what it is instead of failing obscurely
    assert!(matches!(open(&f), Err(Error::Unsupported(m)) if m.contains("timed text")));
}

#[test]
fn ordinary_files_are_not_timed_text() {
    let (to, flags) = ipbb();
    let f = build(3, prores_like, &to, &flags);
    assert!(!is_timed_text(&f));
    assert!(open_timed_text(&f).is_err());
    assert!(!is_timed_text(b""));
    assert!(!is_timed_text(&f[..40]));
}

#[test]
fn a_missing_resource_partition_is_a_warning_not_a_failure() {
    let f = build_timed_text(DCST, &[("image/png", b"png")]);
    // drop the generic stream partition and its KLV: cut from the GS partition to the footer
    let gs = f.windows(16).position(|w| w == GS_KEY).unwrap();
    let gs_partition = f[..gs].windows(16).rposition(|w| w[..13] == crate::klv::PARTITION_PREFIX).unwrap();
    let footer = f.windows(16).rposition(|w| w[..13] == crate::klv::PARTITION_PREFIX).unwrap();
    let mut g = f[..gs_partition].to_vec();
    g.extend_from_slice(&f[footer..]);
    let tt = open_timed_text(&g).unwrap();
    assert_eq!(tt.read_document(&g).unwrap(), DCST);
    assert!(tt.resources[0].data.is_none());
    assert!(tt.read_resource(&g, 0).is_err());
    assert_eq!(tt.warnings.len(), 1);
}

#[test]
fn hostile_timed_text_never_panics() {
    let f = build_timed_text(DCST, &[("image/png", b"png data here")]);
    for cut in (0..f.len()).step_by(11) {
        let part = f[..cut].to_vec();
        let _ = is_timed_text(&part);
        if let Ok(tt) = open_timed_text(&part) {
            let _ = tt.read_document(&part);
            let _ = tt.read_resource(&part, 0);
        }
    }
    let mut seed = 0x9E37_79B9u64;
    for _ in 0..400 {
        let mut g = f.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % g.len();
            g[at] = (seed >> 32) as u8;
        }
        let _ = is_timed_text(&g);
        if let Ok(tt) = open_timed_text(&g) {
            let _ = tt.read_document(&g);
            for i in 0..tt.resources.len() {
                let _ = tt.read_resource(&g, i);
            }
        }
    }
}

fn tagged(p: u8, i: usize) -> Vec<u8> {
    let mut v = prores_like(i);
    v.push(p);
    v
}

#[test]
fn op3a_edit_list_plays_the_clips_back_to_back() {
    let comps = [Comp::Clip { pkg: 0, start: 2, dur: 3 }, Comp::Clip { pkg: 1, start: 0, dur: 2 }, Comp::Clip { pkg: 0, start: 0, dur: 1 }];
    let f = build_edit_list(&comps, [5, 4], tagged, &[], &[]);
    let m = open(&f).unwrap();
    assert_eq!(m.operational_pattern.name(), "OP3a");
    assert_eq!(m.tracks.len(), 2, "{:?}", m.warnings);
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!((t.duration, t.first_edit_unit()), (Some(6), 0));
    assert_eq!(t.display_order, (0..6).collect::<Vec<_>>());
    // A2 A3 A4 B0 B1 A0
    let want = [(0, 2), (0, 3), (0, 4), (1, 0), (1, 1), (0, 0)];
    for (d, (p, i)) in want.into_iter().enumerate() {
        assert_eq!(m.read_sample(&f, v, t.sample_at(d as i64).unwrap()).unwrap(), tagged(p, i), "display {d}");
    }
    // sound follows the same edit list, 1920 frames per edit unit
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    let s = &m.tracks[a];
    assert_eq!((s.duration, s.stored_sample_frames()), (Some(6), 6 * 1920));
    let left = |at: u64| m.read_pcm(&f, a, at, 1).unwrap()[0][0];
    let value = |p: usize, k: usize| (p * 10000 + k) as f32 / 32768.0;
    assert_eq!(left(0), value(0, 2 * 1920));
    assert_eq!(left(3 * 1920 - 1), value(0, 5 * 1920 - 1));
    assert_eq!(left(3 * 1920), value(1, 0));
    assert_eq!(left(5 * 1920 - 1), value(1, 2 * 1920 - 1));
    assert_eq!(left(5 * 1920), value(0, 0));
    // a read across a clip boundary
    let pcm = m.read_pcm(&f, a, 3 * 1920 - 2, 4).unwrap();
    assert_eq!(pcm[1], vec![-value(0, 5 * 1920 - 2), -value(0, 5 * 1920 - 1), -value(1, 0), -value(1, 1)]);
}

#[test]
fn fillers_in_an_edit_list_play_as_blank() {
    let comps = [Comp::Filler(1), Comp::Clip { pkg: 1, start: 1, dur: 2 }, Comp::Filler(3), Comp::Clip { pkg: 0, start: 0, dur: 2 }, Comp::Filler(2)];
    let f = build_edit_list(&comps, [2, 4], tagged, &[], &[]);
    let m = open(&f).unwrap();
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.duration, Some(4));
    assert_eq!(t.gaps, vec![(0, 1), (3, 3), (8, 2)]);
    assert_eq!(t.gap_edit_units(), 6);
    // presentation: blank, B1, B2, blank x3, A0, A1, blank x2
    let shown: Vec<Option<Vec<u8>>> = (0..10).map(|p| t.essence_position(p).map(|e| m.read_sample(&f, v, t.sample_at(e).unwrap()).unwrap())).collect();
    let want = [None, Some((1, 1)), Some((1, 2)), None, None, None, Some((0, 0)), Some((0, 1)), None, None];
    for (p, w) in want.into_iter().enumerate() {
        assert_eq!(shown[p], w.map(|(k, i)| tagged(k, i)), "presentation {p}");
    }
    // sound: the same gaps in sample frames (1920 per edit unit)
    let a = m.track_of_kind(TrackKind::Sound).unwrap();
    let s = &m.tracks[a];
    assert_eq!(s.gaps, t.gaps);
    assert_eq!(s.stored_sample_frames(), 4 * 1920);
    assert_eq!(s.sound_runs(0, 10 * 1920), vec![(1920, 0, 2 * 1920), (6 * 1920, 2 * 1920, 2 * 1920)]);
    assert_eq!(s.sound_runs(1000, 7000), vec![(1920, 0, 2 * 1920)]);
    assert_eq!(s.sound_runs(3 * 1920 + 5, 7 * 1920), vec![(6 * 1920, 2 * 1920, 1920)]);
    assert_eq!(s.sound_runs(8 * 1920, 20 * 1920), vec![]);
    let left = |at: u64| m.read_pcm(&f, a, at, 1).unwrap()[0][0];
    assert_eq!(left(0), (10000 + 1920) as f32 / 32768.0);
    assert_eq!(left(2 * 1920), 0.0);
}

/// A clip whose package is not in the file plays as blank for its duration.
#[test]
fn a_clip_of_another_file_plays_as_blank() {
    let comps = [Comp::Clip { pkg: 0, start: 0, dur: 2 }, Comp::Clip { pkg: 7, start: 0, dur: 4 }, Comp::Clip { pkg: 1, start: 0, dur: 1 }];
    let f = build_edit_list(&comps, [2, 2], tagged, &[], &[]);
    let m = open(&f).unwrap();
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    assert_eq!(m.tracks[v].gaps, vec![(2, 4)]);
    assert_eq!(m.tracks[v].essence_position(6), Some(2));
    assert!(m.warnings.iter().any(|w| w.contains("not in this file")), "{:?}", m.warnings);
}

#[test]
fn long_gop_clips_keep_their_decoding_references_hidden() {
    // stored I0 P3 B1 B2 P6 B4 B5 in each package; package A from display 4 needs its GOP from
    // I0 (stored 0..=6 covers display 4 and 5): the pictures before it decode but never show
    let (to, flags) = ipbb();
    let avc = |p: u8, i: usize| vec![0, 0, 0, 1, 9, 0x10, p, i as u8];
    let comps = [Comp::Clip { pkg: 0, start: 4, dur: 2 }, Comp::Clip { pkg: 1, start: 0, dur: 3 }];
    let f = build_edit_list(&comps, [7, 7], avc, &to, &flags);
    let mut f2 = f.clone();
    let lbl = [0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03, 0x00];
    while let Some(at) = f2.windows(8).position(|w| w == lbl) {
        f2[at..at + 8].copy_from_slice(&[0x04, 0x01, 0x02, 0x02, 0x01, 0x31, 0x40, 0x01]);
    }
    let m = open(&f2).unwrap();
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert_eq!(t.codec, Codec::Avc { intra: false });
    assert_eq!(t.duration, Some(5));
    assert_eq!(t.display_order.len(), 5);
    // displays 4 and 5 of A are stored at 5 and 6; B's displays 0..3 at stored 0, 2, 3 (from I0 to P3)
    let shown: Vec<Vec<u8>> = (0..5).map(|d| m.read_sample(&f2, v, t.sample_at(d).unwrap()).unwrap()).collect();
    let want = [(0, 5), (0, 6), (1, 0), (1, 2), (1, 3)];
    for (d, (p, i)) in want.into_iter().enumerate() {
        assert_eq!(shown[d], avc(p, i), "display {d}");
    }
    // every presented picture's random-access point is a stored key picture of its own clip
    for d in 0..5 {
        let s = t.sample_at(d).unwrap();
        assert_eq!(t.samples[s].pts, d);
        let k = t.sync_before(s);
        assert!(t.samples[k].key);
        assert_eq!(m.read_sample(&f2, v, k).unwrap()[6], want[d as usize].0);
    }
    // the hidden references have pts no presented picture uses
    let hidden = t.samples.iter().filter(|s| !(0..5).contains(&s.pts)).count();
    assert_eq!(hidden, t.samples.len() - 5);
    let mut all: Vec<i64> = t.samples.iter().map(|s| s.pts).collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), t.samples.len());
}

#[test]
fn long_gop_clips_without_temporal_offsets_take_the_bitstream_order() {
    // #1008: the same stored I0 P3 B1 B2 P6 B4 B5 packages as above, but the index marks the B
    // pictures without temporal offsets; the presentation order comes from the bitstream (here
    // from the picture's stored number, as the caller's codec parser would give it)
    let (_, flags) = ipbb();
    let avc = |p: u8, i: usize| vec![0, 0, 0, 1, 9, 0x10, p, i as u8];
    let comps = [Comp::Clip { pkg: 0, start: 4, dur: 2 }, Comp::Clip { pkg: 1, start: 0, dur: 3 }];
    let mut f = build_edit_list(&comps, [7, 7], avc, &[], &flags);
    let lbl = [0x04, 0x01, 0x02, 0x02, 0x03, 0x06, 0x03, 0x00];
    while let Some(at) = f.windows(8).position(|w| w == lbl) {
        f[at..at + 8].copy_from_slice(&[0x04, 0x01, 0x02, 0x02, 0x01, 0x31, 0x40, 0x01]);
    }
    const DISPLAY_OF_STORED: [i64; 7] = [0, 3, 1, 2, 6, 4, 5];
    let asked = std::cell::Cell::new(0);
    let order = |codec: Codec, heads: &[Vec<u8>]| {
        assert_eq!(codec, Codec::Avc { intra: false });
        asked.set(asked.get() + 1);
        heads.iter().map(|h| h.get(7).and_then(|&i| DISPLAY_OF_STORED.get(i as usize).copied())).collect::<Option<Vec<i64>>>()
    };
    let m = open_with_order(&f, &order).unwrap();
    assert_eq!(asked.get(), 2, "once per clip");
    assert!(m.warnings.iter().all(|w| !w.contains("stored order")), "{:?}", m.warnings);
    let v = m.track_of_kind(TrackKind::Picture).unwrap();
    let t = &m.tracks[v];
    assert!(!t.needs_reorder);
    assert_eq!(t.duration, Some(5));
    let shown: Vec<Vec<u8>> = (0..5).map(|d| m.read_sample(&f, v, t.sample_at(d).unwrap()).unwrap()).collect();
    let want = [(0, 5), (0, 6), (1, 0), (1, 2), (1, 3)];
    for (d, (p, i)) in want.into_iter().enumerate() {
        assert_eq!(shown[d], avc(p, i), "display {d}");
    }
    // without a bitstream order the stored order stays, with a warning
    let m = open(&f).unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("stored order")), "{:?}", m.warnings);
    // an order that is not one position per picture is not used
    let m = open_with_order(&f, &|_, heads: &[Vec<u8>]| Some(vec![0; heads.len()])).unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("stored order")), "{:?}", m.warnings);
    let m = open_with_order(&f, &|_, _: &[Vec<u8>]| Some(vec![i64::MAX, -1])).unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("stored order")), "{:?}", m.warnings);
}

#[test]
fn hostile_edit_lists_never_panic() {
    for comps in [
        vec![Comp::Clip { pkg: 0, start: i64::MAX, dur: 3 }, Comp::Clip { pkg: 1, start: 0, dur: i64::MAX }],
        vec![Comp::Clip { pkg: 0, start: -5, dur: -1 }, Comp::Clip { pkg: 1, start: 9, dur: 2 }],
        vec![Comp::Filler(i64::MAX), Comp::Clip { pkg: 1, start: 0, dur: 1 }],
        vec![Comp::Clip { pkg: 1, start: 0, dur: 1 }, Comp::Filler(i64::MAX), Comp::Clip { pkg: 0, start: 0, dur: 1 }, Comp::Filler(i64::MAX)],
        vec![Comp::Filler(-4), Comp::Clip { pkg: 9, start: 0, dur: i64::MAX }, Comp::Clip { pkg: 0, start: 0, dur: 1 }],
    ] {
        let f = build_edit_list(&comps, [3, 2], tagged, &[], &[]);
        if let Ok(m) = open(&f) {
            for (i, t) in m.tracks.iter().enumerate() {
                for d in 0..t.display_order.len() as i64 {
                    let _ = t.sample_at(d).map(|s| m.read_sample(&f, i, s));
                }
                if t.kind == TrackKind::Sound {
                    let _ = m.read_pcm(&f, i, 0, 64);
                    let _ = t.sound_runs(0, u64::MAX);
                }
                let _ = (t.essence_position(i64::MAX), t.essence_position(i64::MIN), t.gap_edit_units());
            }
        }
    }
}
