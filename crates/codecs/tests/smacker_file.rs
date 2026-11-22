//! Smacker files read through the media source: pictures and audio equal what `cfa4-smacker`
//! decodes front to back, in any order, from the middle, and across tracks.

use cfa4_codecs::open_bytes;
use cfa4_media::FrameRequest;
use cfa4_smacker::{Decoder, Smk, TrackKind, decode_audio};
use cfa4_testkit::smk::{Rng, random_spec, smk_file};
use cfa4_time::FrameRate;

/// The pictures a file shows, front to back: the picture after each frame.
fn pictures(data: &[u8]) -> Vec<Vec<u8>> {
    let smk = Smk::parse(data).unwrap();
    let mut dec = Decoder::new(&smk).unwrap();
    (0..smk.frames)
        .map(|i| {
            let parts = smk.split(data, i);
            match (parts.video, parts.palette) {
                (Some(v), _) => {
                    let _ = dec.decode(parts.palette, v);
                }
                (None, Some(p)) => {
                    let _ = dec.update_palette(p);
                }
                (None, None) => {}
            }
            dec.rgba()
        })
        .collect()
}

/// A track's samples as the source lays them out: each chunk cut or padded to the samples its
/// declared length stands for, planar.
fn track_planes(data: &[u8], slot: usize) -> Vec<Vec<f32>> {
    let smk = Smk::parse(data).unwrap();
    let t = smk.tracks[slot].unwrap();
    let ch = usize::from(t.channels);
    let mut planes = vec![Vec::new(); ch];
    for i in 0..smk.frames {
        let Some(chunk) = smk.split(data, i).audio[slot] else { continue };
        let n = cfa4_smacker::chunk_samples(&t, chunk);
        let mut s = decode_audio(&t, chunk).unwrap_or_default();
        s.resize(n * ch, 0);
        for (k, v) in s.iter().enumerate() {
            planes[k % ch].push(t.to_f32(*v));
        }
    }
    planes
}

fn frame_at(rate: FrameRate, n: i64) -> FrameRequest {
    FrameRequest::full(rate.tick_of(n))
}

#[test]
fn pictures_match_in_every_order() {
    let mut rng = Rng(0x51_0a);
    for n in 0..8 {
        let spec = random_spec(&mut rng);
        let data = smk_file(&mut rng, &spec);
        let want = pictures(&data);
        let s = open_bytes("clip.smk", data.clone().into()).unwrap();
        let v = s.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (spec.w, spec.h), "file {n}");
        assert_eq!(v.par, (1, 1 + u32::from(spec.interlace)));
        let smk = Smk::parse(&data).unwrap();
        let (num, den) = smk.frame_rate();
        assert_eq!((v.frame_rate.num, v.frame_rate.den), (num, den));
        let rate = v.frame_rate;
        let rgba = |i: usize| -> Vec<u8> {
            let f = s.video_frame(frame_at(rate, i as i64)).unwrap();
            match &f.data {
                cfa4_frame::PixelData::Rgba8(p) => p.to_vec(),
                _ => panic!("RGBA expected"),
            }
        };
        // front to back
        for (i, w) in want.iter().enumerate() {
            assert_eq!(&rgba(i), w, "file {n} frame {i} in order");
        }
        // jumps: backwards, from the middle, the last frame, a random order
        let last = want.len() - 1;
        let mut order = vec![last, 0, last / 2, 1.min(last), last, last / 3];
        order.extend((0..20).map(|_| rng.below(want.len() as u64) as usize));
        for i in order {
            assert_eq!(&rgba(i), &want[i], "file {n} frame {i} after a jump");
        }
        // past the end shows the last frame
        assert_eq!(&rgba(want.len() + 5), &want[last]);
    }
}

#[test]
fn a_long_file_restarts_from_snapshots() {
    // 700 frames: more than the interval between snapshots, so jumps restore one and decode forward
    let mut rng = Rng(0x10_09);
    let mut spec = random_spec(&mut rng);
    spec.frames = 700;
    spec.ring = false;
    spec.w = 16;
    spec.h = 16;
    let data = smk_file(&mut rng, &spec);
    let want = pictures(&data);
    let s = open_bytes("long.smk", data.into()).unwrap();
    let rate = s.info().video.as_ref().unwrap().frame_rate;
    for i in [650usize, 12, 400, 399, 401, 699, 0, 333, 129, 128, 127] {
        let f = s.video_frame(frame_at(rate, i as i64)).unwrap();
        let cfa4_frame::PixelData::Rgba8(p) = &f.data else { panic!("RGBA expected") };
        assert_eq!(p.as_slice(), want[i].as_slice(), "frame {i}");
    }
}

#[test]
fn audio_tracks_match() {
    let mut rng = Rng(0xa0d10);
    let mut checked = 0;
    for n in 0..24 {
        let spec = random_spec(&mut rng);
        let data = smk_file(&mut rng, &spec);
        let smk = Smk::parse(&data).unwrap();
        let slots: Vec<usize> = smk.tracks.iter().enumerate().filter_map(|(i, t)| t.filter(|t| t.kind != TrackKind::Bink).map(|_| i)).collect();
        let s = open_bytes("clip.smk", data.clone().into()).unwrap();
        let info = s.info().clone();
        let with_samples: Vec<usize> = slots.iter().copied().filter(|&sl| !track_planes(&data, sl)[0].is_empty()).collect();
        assert_eq!(info.audio_streams.len(), with_samples.len(), "file {n}");
        for (stream, &slot) in with_samples.iter().enumerate() {
            let want = track_planes(&data, slot);
            let t = smk.tracks[slot].unwrap();
            let a = &info.audio_streams[stream];
            assert_eq!((a.sample_rate, a.channels as usize, a.bits_per_sample), (t.rate, want.len(), Some(u32::from(t.bits))));
            let total = want[0].len();
            // the whole track, then windows from the middle
            let got = s.audio_stream(stream, 0, total + 10, t.rate).unwrap();
            for (c, w) in want.iter().enumerate() {
                assert_eq!(&got.channels[c][..total], w.as_slice(), "file {n} track {slot} channel {c}");
                assert!(got.channels[c][total..].iter().all(|&v| v == 0.0));
            }
            for _ in 0..6 {
                let from = rng.below(total as u64) as usize;
                let len = 1 + rng.below(300) as usize;
                let got = s.audio_stream(stream, from as i64, len, t.rate).unwrap();
                for (c, w) in want.iter().enumerate() {
                    let end = (from + len).min(total);
                    assert_eq!(&got.channels[c][..end - from], &w[from..end], "file {n} track {slot} window {from}+{len}");
                }
            }
            checked += 1;
        }
        assert!(s.audio_stream(with_samples.len(), 0, 10, 44100).is_err());
        let dur = info.duration;
        assert!(dur >= FrameRate::new(smk.frame_rate().0, smk.frame_rate().1).tick_of(smk.frames as i64));
    }
    assert!(checked > 5, "the random files held {checked} audio tracks");
}

/// Opens a damaged file and asks for pictures and audio; the result does not matter, a panic does.
fn exercise(m: Vec<u8>) {
    let Ok(s) = open_bytes("bad.smk", m.into()) else { return };
    let rate = s.info().video.as_ref().map_or(FrameRate::new(10, 1), |v| v.frame_rate);
    for i in [0i64, 3, 1, 9] {
        let _ = s.video_frame(frame_at(rate, i));
    }
    for k in 0..s.info().audio_streams.len() {
        let _ = s.audio_stream(k, 0, 4000, 44100);
    }
}

#[test]
fn damaged_files_open_or_fail_cleanly() {
    let mut rng = Rng(0xda3a);
    let mut panicked = Vec::new();
    for case in 0..4 {
        let spec = random_spec(&mut rng);
        let data = smk_file(&mut rng, &spec);
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        // random bytes and truncation
        for n in 0..120 {
            let mut m = data.clone();
            for _ in 0..1 + rng.below(6) {
                let at = if rng.chance(50) { rng.below(m.len().min(200) as u64) } else { rng.below(m.len() as u64) } as usize;
                m[at] = rng.byte();
            }
            if rng.chance(30) {
                m.truncate(rng.below(m.len() as u64) as usize);
            }
            files.push((format!("case {case} bytes {n}"), m));
        }
        // single bit flips
        for n in 0..120 {
            let mut m = data.clone();
            let at = if rng.chance(60) { rng.below(m.len().min(400) as u64) } else { rng.below(m.len() as u64) } as usize;
            m[at] ^= 1 << rng.below(8);
            files.push((format!("case {case} flip {n}"), m));
        }
        // corrupt size fields: the header's sizes, the frame sizes, the first audio chunk sizes
        for at in (4..104).step_by(4).chain((104..160.min(data.len())).step_by(4)) {
            for v in [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x0100_0000] {
                let mut m = data.clone();
                if let Some(w) = m.get_mut(at..at + 4) {
                    w.copy_from_slice(&v.to_le_bytes());
                }
                files.push((format!("case {case} size at {at} = {v:#x}"), m));
            }
        }
        for (name, m) in files {
            if std::panic::catch_unwind(|| exercise(m)).is_err() {
                panicked.push(name);
            }
        }
    }
    assert!(panicked.is_empty(), "panics on damaged files: {panicked:?}");
}
