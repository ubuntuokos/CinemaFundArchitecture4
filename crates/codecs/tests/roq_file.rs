//! An id RoQ file read through the media source: every picture equals what `cfa4_roq::Decoder`
//! gives playing from the start, whichever order the pictures are asked for in (a seek restarts
//! from a kept decoder state), and the sound reads from the middle.

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_time::{FrameRate, Tick};
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

fn chunk(kind: u16, size: usize, arg: u16, body: &[u8]) -> Vec<u8> {
    let mut v = kind.to_le_bytes().to_vec();
    v.extend_from_slice(&(size as u32).to_le_bytes());
    v.extend_from_slice(&arg.to_le_bytes());
    v.extend_from_slice(body);
    v
}

/// A 32 × 16 file of `frames` pictures at 30 fps with mono sound after each: every picture has
/// new codebooks (4 and 4 entries) and a mix of the block codes (no sub-blocks).
fn roq(frames: usize) -> Vec<u8> {
    let mut rng = Rng(0x5eed_f00d);
    let mut f = vec![0x84, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 30, 0];
    f.extend(chunk(0x1001, 8, 0, &[32, 0, 16, 0, 8, 0, 4, 0]));
    for k in 0..frames {
        let mut cb = Vec::new();
        for _ in 0..4 {
            (0..6).for_each(|_| cb.push(rng.below(256) as u8));
        }
        for _ in 0..4 {
            (0..4).for_each(|_| cb.push(rng.below(4) as u8));
        }
        f.extend(chunk(0x1002, cb.len(), 0x0404, &cb));
        // two macroblocks (four 8×8 blocks each): the code words come first for each batch
        let mut data = Vec::new();
        let mut codes = Vec::new();
        let mut word = 0u16;
        for blk in 0..8 {
            let c = if k == 0 { 2 } else { rng.below(3) } as u16;
            codes.push(c);
            word |= c << ((7 - blk) * 2);
        }
        data.extend_from_slice(&word.to_le_bytes());
        for &c in &codes {
            match c {
                0 => {}
                1 => data.push(rng.below(256) as u8),
                2 => data.push(rng.below(4) as u8),
                _ => unreachable!(),
            }
        }
        f.extend(chunk(0x1011, data.len(), 0, &data));
        let n = 735;
        let snd: Vec<u8> = (0..n).map(|_| rng.below(256) as u8).collect();
        f.extend(chunk(0x1020, n, (1000 * k) as u16, &snd));
    }
    f
}

#[test]
fn roq_file_reads_in_any_order() {
    let frames = 420;
    let data = roq(frames);
    let s = cfa4_roq::parse(&data).unwrap();
    assert_eq!((s.video.len(), s.audio.len(), s.width, s.height), (frames, frames, 32, 16));
    let mut dec = cfa4_roq::Decoder::new(32, 16).unwrap();
    let want: Vec<[Vec<u8>; 3]> = s.video.iter().map(|v| dec.decode(v.of(&data)).planes).collect();
    assert!(want[3] != want[4]);

    let src = cfa4_codecs::open_bytes("intro.roq", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "id RoQ");
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.frame_rate), (32, 16, FrameRate::new(30, 1)));
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (22050, 1, "id RoQ DPCM"));

    let rate = FrameRate::new(30, 1);
    let at = |k: usize| FrameRequest::full(rate.tick_of(k as i64) + Tick::from_rational(1, 1, 120));
    for k in [0, 1, 2, 3, 419, 5, 130, 129, 127, 128, 260, 0, 258, 257, 400, 399, 398, 300, 301] {
        let f = src.video_frame(at(k)).unwrap();
        let PixelData::Yuv8 { planes, .. } = &f.data else { panic!("not 8-bit YUV") };
        for p in 0..3 {
            assert_eq!(*planes[p], want[k][p], "frame {k}, plane {p}");
        }
    }

    // the sound: chunk k holds samples 735 k ..
    let mut pcm = Vec::new();
    for a in &s.audio {
        pcm.extend(cfa4_roq::decode_audio(a.of(&data), 1).unwrap());
    }
    let got = src.audio(10_000, 2_000, 22050).unwrap();
    let exp: Vec<f32> = pcm[10_000..12_000].iter().map(|&v| f32::from(v) / 32768.0).collect();
    assert_eq!(got.channels[0], exp);
}
