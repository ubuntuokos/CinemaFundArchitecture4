//! ATRAC9 in WAV (`.at9`, `WAVE_FORMAT_EXTENSIBLE` with the ATRAC9 sub-format) through the media
//! layer against ffmpeg's `wav` demuxer and `atrac9` decoder: synthetic streams whose packets
//! the decoder's strict mode accepts (there is no public sample and no free encoder), read in
//! order and with jumps backwards across decoder checkpoints, plus a short last packet.

mod common;

use std::path::Path;

use common::*;

fn extradata(sri: u32, layout: u32, frame_size: usize, superframe_idx: u32) -> [u8; 12] {
    let cfg = 0xFEu32 << 24 | sri << 20 | layout << 17 | (frame_size as u32 - 1) << 5 | superframe_idx << 3;
    let mut e = [0u8; 12];
    e[0] = 2;
    e[4..8].copy_from_slice(&cfg.to_be_bytes());
    e
}

fn at9(channels: u16, rate: u32, extra: &[u8; 12], ba: usize, data: &[u8]) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(ba as u32 * 100).to_le_bytes());
    fmt.extend_from_slice(&(ba as u16).to_le_bytes());
    fmt.extend_from_slice(&0u16.to_le_bytes());
    fmt.extend_from_slice(&34u16.to_le_bytes());
    fmt.extend_from_slice(&256u16.to_le_bytes());
    fmt.extend_from_slice(&0u32.to_le_bytes());
    fmt.extend_from_slice(&[0xD2, 0x42, 0xE1, 0x47, 0xBA, 0x36, 0x8D, 0x4D, 0x88, 0xFC, 0x61, 0x65, 0x4F, 0x8C, 0x83, 0x6C]);
    fmt.extend_from_slice(extra);
    let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    w.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    w.extend_from_slice(&fmt);
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(data);
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

/// `count` packets of `size` bytes that decode in strict mode one after another, each grown one
/// block at a time from random bits.
fn valid_packets(extra: &[u8; 12], size: usize, blocks: usize, count: usize, seed: u64) -> (Vec<Vec<u8>>, cfa4_realaudio::atrac9::Decoder) {
    let mut rng = Rng(seed);
    let mut d = cfa4_realaudio::atrac9::Decoder::new(extra, size).unwrap();
    d.set_strict(true);
    let out = more_packets(&mut d, size, blocks, count, &mut rng);
    (out, d)
}

/// `count` more packets for the decoder state `d` (advanced past them).
fn more_packets(d: &mut cfa4_realaudio::atrac9::Decoder, size: usize, blocks: usize, count: usize, rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    'packet: while out.len() < count {
        let mut prefix: Vec<u8> = Vec::new();
        for k in 0..blocks {
            let mut found = false;
            for _ in 0..4000 {
                let mut cand = prefix.clone();
                while cand.len() < size {
                    cand.push(rng.below(256) as u8);
                }
                let mut c = d.clone();
                let _ = c.decode(&cand);
                if c.blocks_decoded() > k {
                    cand.truncate(c.bits_used() / 8);
                    prefix = cand;
                    found = true;
                    break;
                }
            }
            if !found {
                continue 'packet;
            }
        }
        while prefix.len() < size {
            prefix.push(rng.below(256) as u8);
        }
        d.decode(&prefix).unwrap();
        out.push(prefix);
    }
    out
}

/// A short last packet of `len` bytes for the state `d`: the start of a valid packet, whose
/// sample count (FFmpeg 9 decodes a short packet's whole frames, then again from the first byte
/// after them) the media layer can tell from the stream's start state, as it does when opening.
fn short_packet(
    d: &cfa4_realaudio::atrac9::Decoder,
    fresh: &cfa4_realaudio::atrac9::Decoder,
    size: usize,
    blocks: usize,
    len: usize,
    rng: &mut Rng,
) -> Vec<u8> {
    for _ in 0..200 {
        let mut c = d.clone();
        let p = more_packets(&mut c, size, blocks, 1, rng).remove(0);
        let tail = &p[..len];
        let mut run = d.clone();
        run.set_strict(false);
        let Ok(o) = run.decode(tail) else { continue };
        if !o[0].is_empty() && fresh.samples_in_packet(tail) == o[0].len() {
            return tail.to_vec();
        }
    }
    panic!("no short packet");
}

/// Our samples through `cfa4_codecs::open_bytes` against ffmpeg's, within 1e-5 of each packet's
/// peak (at least 1).
fn compare(ff: &Path, path: &Path, channels: usize, rate: u32, spp: usize) {
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("?").to_string();
    let src = cfa4_codecs::open_bytes(&file, bytes(path)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let a = src.info().audio_streams.first().cloned().unwrap_or_else(|| panic!("{file}: no audio"));
    assert_eq!((a.codec.as_str(), a.channels, a.sample_rate), ("ATRAC9", channels as u32, rate), "{file}");
    let flat = ffmpeg_audio_f32(ff, path, &[]);
    let want: Vec<Vec<f32>> = (0..channels).map(|c| flat.iter().skip(c).step_by(channels).copied().collect()).collect();
    let n = want[0].len();
    assert_eq!(src.info().duration, cfa4_time::Tick::from_units(n as i64, i64::from(rate)), "{file}: length");
    let peaks: Vec<f32> = (0..n.div_ceil(spp)).map(|k| want.iter().flat_map(|w| &w[k * spp..n.min(k * spp + spp)]).fold(1f32, |m, v| m.max(v.abs()))).collect();
    let check = |start: usize, len: usize| {
        let got = src.audio(start as i64, len, rate).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (c, (g, w)) in got.channels.iter().zip(&want).enumerate() {
            assert_eq!(g.len(), len, "{file}: channel {c} length");
            for (i, (x, y)) in g.iter().zip(&w[start..start + len]).enumerate() {
                let at = start + i;
                assert!((x - y).abs() <= 1e-5 * peaks[at / spp], "{file}: channel {c} sample {at}: ours {x} ffmpeg {y}");
            }
        }
    };
    for s in (0..n).step_by(9_000) {
        check(s, 9_000.min(n - s));
    }
    for s in (0..n).step_by(25_000).rev() {
        check(s, 3000.min(n - s));
    }
}

#[test]
fn synthetic_atrac9_in_wav_matches() {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip("ATRAC9") else { return };
    let dir = cfa4_testkit::fixtures_dir("atrac9");
    // (layout, sample rate index, frame bytes, superframe index, blocks per frame, channels, rate)
    for (layout, sri, fs, sfi, blocks, ch, rate) in [(2u32, 7u32, 300usize, 2u32, 1usize, 2usize, 48_000u32), (3, 6, 700, 0, 4, 6, 44_100)] {
        let extra = extradata(sri, layout, fs, sfi);
        let ba = fs << sfi;
        // 200 packets: past three 64-packet checkpoints
        let (packets, d) = valid_packets(&extra, ba, blocks << sfi, 200, 0x5DEE_CE66_D1CE_4E5B ^ u64::from(layout));
        let mut data = packets.concat();
        // a short last packet of one and a half frames
        if sfi > 0 {
            let fresh = cfa4_realaudio::atrac9::Decoder::new(&extra, ba).unwrap();
            data.extend(short_packet(&d, &fresh, ba, blocks << sfi, fs + fs / 2, &mut Rng(u64::from(layout))));
        }
        let p = dir.join(format!("random_{layout}.at9"));
        std::fs::write(&p, at9(ch as u16, rate, &extra, ba, &data)).unwrap();
        compare(&ff, &p, ch, rate, 256 << sfi);
    }
}
