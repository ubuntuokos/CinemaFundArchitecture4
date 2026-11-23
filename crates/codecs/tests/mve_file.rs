//! An Interplay MVE file read through the media source: every picture equals what
//! `cfa4_mve::Decoder` gives playing from the start, whichever order the pictures are asked for
//! in (a seek restarts from a kept decoder state), and the sound reads from the middle.

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_time::Tick;
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

fn opcode(kind: u8, version: u8, body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u16).to_le_bytes().to_vec();
    v.extend([kind, version]);
    v.extend_from_slice(body);
    v
}

fn chunk(kind: u16, ops: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = ops.concat();
    let mut v = (body.len() as u16).to_le_bytes().to_vec();
    v.extend_from_slice(&kind.to_le_bytes());
    v.extend(body);
    v
}

/// A 16 × 16 8-bit file of `frames` packets (one in five not shown) with mono 16-bit sound: each
/// of the four 8×8 blocks is raw or a copy of the picture before.
fn mve(frames: usize) -> Vec<u8> {
    let mut rng = Rng(0x5eed_f00d);
    let mut out = b"Interplay MVE File\x1A\0".to_vec();
    out.extend_from_slice(&[0x1A, 0, 0, 1, 0x33, 0x11]);
    out.extend(chunk(2, &[opcode(0x05, 2, &[2, 0, 2, 0, 0, 0, 0, 0]), opcode(0x02, 0, &[0x40, 0x41, 0, 0, 8, 0])]));
    out.extend(chunk(0, &[opcode(0x03, 0, &[0, 0, 2, 0, 0x22, 0x56])]));
    for f in 0..frames {
        let mut ops = Vec::new();
        let mut audio = vec![0, 0, 0xFF, 0xFF, 0, 0];
        (0..800).for_each(|_| audio.push(rng.below(256) as u8));
        ops.push(opcode(0x08, 0, &audio));
        if f == 0 {
            let mut body = vec![0, 0, 0, 1];
            (0..768).for_each(|_| body.push(rng.below(64) as u8));
            ops.push(opcode(0x0C, 0, &body));
        }
        let mut stream: Vec<u8> = (0..14).map(|_| rng.below(256) as u8).collect();
        let mut nibbles = Vec::new();
        for _ in 0..4 {
            if f == 0 || rng.below(2) == 0 {
                nibbles.push(0xB);
                (0..64).for_each(|_| stream.push(rng.below(256) as u8));
            } else {
                nibbles.push(0x0);
            }
        }
        let dmap: Vec<u8> = nibbles.chunks(2).map(|p| p[0] | p[1] << 4).collect();
        ops.push(opcode(0x0F, 0, &dmap));
        ops.push(opcode(0x11, 0, &stream));
        if f % 5 != 3 {
            ops.push(opcode(0x07, 0, &[]));
        }
        out.extend(chunk(3, &ops));
    }
    out.extend(chunk(5, &[]));
    out
}

#[test]
fn mve_file_reads_in_any_order() {
    let frames = 200;
    let data = mve(frames);
    let m = cfa4_mve::parse(&data).unwrap();
    assert_eq!((m.width, m.height, m.video.len(), m.audio_packets.len()), (16, 16, frames, frames));
    let mut dec = cfa4_mve::Decoder::new(16, 16, false).unwrap();
    let mut want = Vec::new();
    for p in &m.video {
        let input = cfa4_mve::Input {
            format: p.format,
            send_buffer: p.send_buffer,
            video: p.video.of(&data),
            decode_map: p.decode_map.of(&data),
            skip_map: p.skip_map.of(&data),
            palette: p.palette.as_deref(),
        };
        if let Some(f) = dec.decode(&input).unwrap() {
            want.push(f.to_rgba());
        }
    }
    assert_eq!(want.len(), frames - frames / 5);
    assert!(want[3] != want[4]);

    let src = cfa4_codecs::open_bytes("intro.mve", Arc::from(data.clone())).unwrap();
    let info = src.info();
    assert_eq!(info.container, "Interplay MVE");
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (16, 16, "Interplay MVE"));
    let a = info.audio().unwrap();
    assert_eq!((a.sample_rate, a.channels), (22050, 1));

    let rate = v.frame_rate;
    let at = |k: usize| FrameRequest::full(rate.tick_of(k as i64) + Tick::from_rational(1, 1, 1000));
    let n = want.len();
    for k in [0, 1, 2, 3, n - 1, 5, 70, 66, 63, 64, 130, 0, 128, 127, 150, 149, 148, 100, 101] {
        let f = src.video_frame(at(k)).unwrap();
        let PixelData::Rgba8(px) = &f.data else { panic!("not RGBA") };
        let d: Vec<usize> = (0..px.len()).filter(|&i| px[i] != want[k][i]).collect();
        assert!(d.is_empty(), "frame {k} (n={n}) differs at {} places, first {:?}", d.len(), &d[..d.len().min(8)]);
    }

    let mut pcm = Vec::new();
    for s in &m.audio_packets {
        pcm.extend(cfa4_mve::decode_audio(m.audio.as_ref().unwrap(), s.of(&data)).unwrap());
    }
    let got = src.audio(1_000, 2_000, 22050).unwrap();
    let exp: Vec<f32> = pcm[1_000..3_000].iter().map(|&v| f32::from(v) / 32768.0).collect();
    assert_eq!(got.channels[0], exp);
}
