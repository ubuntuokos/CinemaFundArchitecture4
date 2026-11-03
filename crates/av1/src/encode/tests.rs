use super::*;

/// A synthetic test picture: gradients, edges, texture and a moving square.
fn picture(w: usize, h: usize, t: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut y = vec![0u8; w * h];
    let mut seed = 0x1234_5678u32;
    for r in 0..h {
        for c in 0..w {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = ((seed >> 16) & 7) as i32 - 4;
            let mut v = (c * 255 / w.max(1)) as i32 / 2 + (r * 255 / h.max(1)) as i32 / 3;
            if (c / 16 + r / 16) % 2 == 0 && r > h / 2 {
                v += 60;
            }
            let (sx, sy) = (10 + 3 * t, 12 + 2 * t);
            if c >= sx && c < sx + 24 && r >= sy && r < sy + 20 {
                v = 220 - ((c - sx) * 3) as i32;
            }
            y[r * w + c] = (v + noise).clamp(0, 255) as u8;
        }
    }
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    for r in 0..ch {
        for c in 0..cw {
            u[r * cw + c] = (128 + (c as i32 - cw as i32 / 2) / 2) as u8;
            v[r * cw + c] = (128 + (r as i32 - ch as i32 / 2) / 3 + t as i32) as u8;
        }
    }
    (y, u, v)
}

fn psnr(a: &[u16], b: &[u8]) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

fn encode_clip(cfg: EncoderConfig, frames: usize) -> (Vec<Vec<u8>>, Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut out = Vec::new();
    let mut pics = Vec::new();
    for t in 0..frames {
        let p = picture(w, h, t);
        let f = YuvFrame::U8 { y: &p.0, u: &p.1, v: &p.2, y_stride: w, uv_stride: w.div_ceil(2) };
        for pk in enc.encode(&f, t as i64).expect("encode") {
            out.push(pk.data);
        }
        pics.push(p);
    }
    (out, pics)
}

#[test]
fn intra_frame_round_trips() {
    let mut cfg = EncoderConfig::new(96, 64, 25, 1);
    cfg.rate = RateControl::QIndex(60);
    cfg.threads = 1;
    let (packets, pics) = encode_clip(cfg, 1);
    let mut dec = crate::Decoder::with_threads(1);
    let pics_out = dec.decode(&packets[0]).expect("decode");
    let got = pics_out.first().expect("one picture");
    let p = psnr(&got.planes[0], &pics[0].0);
    eprintln!("intra: {} bytes, psnr {p:.2}", packets[0].len());
    assert!(p > 35.0, "psnr {p}");
}

#[test]
fn inter_frames_round_trip() {
    let mut cfg = EncoderConfig::new(96, 64, 25, 1);
    cfg.rate = RateControl::QIndex(60);
    cfg.threads = 1;
    let (packets, pics) = encode_clip(cfg, 4);
    let mut dec = crate::Decoder::with_threads(1);
    let mut got = Vec::new();
    for p in &packets {
        got.extend(dec.decode(p).expect("decode"));
    }
    got.extend(dec.flush());
    assert_eq!(got.len(), 4);
    for (i, (g, s)) in got.iter().zip(&pics).enumerate() {
        let p = psnr(&g.planes[0], &s.0);
        eprintln!("frame {i}: {} bytes, psnr {p:.2}", packets[i].len());
        assert!(p > 35.0, "frame {i} psnr {p}");
    }
}

fn decode_packets(packets: &[Vec<u8>]) -> Vec<crate::Picture> {
    let mut dec = crate::Decoder::with_threads(2);
    let mut got = Vec::new();
    for p in packets {
        got.extend(dec.decode(p).expect("decode"));
    }
    got.extend(dec.flush());
    got
}

/// Sizes that are not multiples of 8 (partial blocks, forced splits), several tiles coded in
/// parallel, every preset.
#[test]
fn odd_sizes_tiles_and_presets_round_trip() {
    for (preset, w, h) in [(Preset::Speed, 130, 70), (Preset::Balanced, 203, 117), (Preset::Quality, 300, 140)] {
        let mut cfg = EncoderConfig::new(w, h, 25, 1);
        cfg.rate = RateControl::QIndex(80);
        cfg.preset = preset;
        cfg.tiles = 4;
        cfg.threads = 2;
        let (packets, pics) = encode_clip(cfg, 3);
        let got = decode_packets(&packets);
        assert_eq!(got.len(), 3);
        for (i, (g, s)) in got.iter().zip(&pics).enumerate() {
            let p = psnr(&g.planes[0], &s.0);
            assert!(p > 33.0, "{preset:?} {w}x{h} frame {i}: psnr {p}");
        }
    }
}

#[test]
fn ten_bit_round_trips() {
    let (w, h) = (96usize, 64usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.bit_depth = 10;
    cfg.rate = RateControl::QIndex(60);
    cfg.threads = 1;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    let mut srcs = Vec::new();
    for t in 0..3 {
        let (y, u, v) = picture(w, h, t);
        let up = |p: &[u8]| -> Vec<u16> { p.iter().map(|&s| ((s as u16) << 2) | (s as u16 >> 6)).collect() };
        let (y, u, v) = (up(&y), up(&u), up(&v));
        let f = YuvFrame::U16 { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 };
        for pk in enc.encode(&f, t as i64).expect("encode") {
            packets.push(pk.data);
        }
        srcs.push(y);
    }
    let got = decode_packets(&packets);
    assert_eq!(got.len(), 3);
    for (g, s) in got.iter().zip(&srcs) {
        let mse: f64 = g.planes[0].iter().zip(s).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum::<f64>() / s.len() as f64;
        let p = 10.0 * (1023.0f64 * 1023.0 / mse.max(1e-9)).log10();
        assert!(p > 35.0, "psnr {p}");
    }
}

/// A higher CRF spends fewer bits.
#[test]
fn crf_controls_the_rate() {
    let size = |crf: u8| -> usize {
        let mut cfg = EncoderConfig::new(96, 64, 25, 1);
        cfg.rate = RateControl::Crf(crf as f32);
        cfg.threads = 1;
        encode_clip(cfg, 3).0.iter().map(|p| p.len()).sum()
    };
    let (lo, hi) = (size(18), size(38));
    assert!(hi < lo, "crf 38: {hi} bytes, crf 18: {lo} bytes");
}

#[test]
fn bad_configurations_are_errors() {
    assert!(Encoder::new(EncoderConfig::new(0, 64, 25, 1)).is_err());
    assert!(Encoder::new(EncoderConfig::new(64, 64, 0, 1)).is_err());
    let mut cfg = EncoderConfig::new(64, 64, 25, 1);
    cfg.bit_depth = 12;
    assert!(Encoder::new(cfg).is_err());
    // a frame too small for the configured size
    let mut enc = Encoder::new(EncoderConfig::new(64, 64, 25, 1)).expect("encoder");
    let y = vec![0u8; 10];
    let f = YuvFrame::U8 { y: &y, u: &y, v: &y, y_stride: 64, uv_stride: 32 };
    assert!(enc.encode(&f, 0).is_err());
}
