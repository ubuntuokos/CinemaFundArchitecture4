use super::*;

/// A moving test picture: gradients, a sharp-edged square that moves, and fine texture.
pub(crate) fn picture(w: usize, h: usize, t: usize) -> [Vec<u8>; 3] {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut y = vec![0u8; w * h];
    let mut seed = 0x1234_5678u32;
    for yy in 0..h {
        for xx in 0..w {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 28) as usize;
            let (sx, sy) = (xx + 3 * t, yy + t);
            let mut v = (sx * 2 + sy) % 200 + noise;
            if (20 + 2 * t..52 + 2 * t).contains(&xx) && (10 + t..40 + t).contains(&yy) {
                v = 230 - ((xx / 4 + yy / 4) % 2) * 60;
            }
            y[yy * w + xx] = v.min(255) as u8;
        }
    }
    let u = (0..cw * ch).map(|i| (100 + (i % cw + t) % 50) as u8).collect();
    let v = (0..cw * ch).map(|i| (160 - (i / cw) % 40) as u8).collect();
    [y, u, v]
}

pub(crate) fn input(p: &[Vec<u8>; 3], w: usize) -> Input<'_> {
    Input { y: &p[0], u: &p[1], v: &p[2], y_stride: w, uv_stride: w.div_ceil(2) }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    let mse = se / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

/// The visible luma of a decoded picture.
fn luma(p: &Picture) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| p.y.get(y * p.y_stride + x) as u8).collect()
}

fn encode_all(cfg: Config, frames: usize) -> (Vec<Packet>, Vec<f64>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let mut enc = Encoder::new(cfg).unwrap();
    let mut dec = Decoder::with_threads(1);
    let (mut packets, mut quality) = (Vec::new(), Vec::new());
    for t in 0..frames {
        let pic = picture(w, h, t);
        let pkt = enc.encode(&input(&pic, w), false).unwrap();
        let out = dec.decode(&pkt.data, t as i64).unwrap();
        assert_eq!(out.len(), 1);
        let got = luma(&out[0]);
        assert_eq!(Some(&got), enc.last_picture().map(luma).as_ref(), "frame {t}: decoder and encoder disagree");
        quality.push(psnr(&got, &pic[0]));
        packets.push(pkt);
    }
    (packets, quality)
}

#[test]
fn frames_decode_close_to_the_source() {
    let mut cfg = Config::new(100, 76);
    cfg.keyframe_interval = 4;
    let (packets, quality) = encode_all(cfg, 6);
    let keys: Vec<bool> = packets.iter().map(|p| p.key).collect();
    assert_eq!(keys, [true, false, false, false, true, false]);
    for (t, q) in quality.iter().enumerate() {
        assert!(*q > 33.0, "frame {t}: {q:.2} dB");
    }
    assert!(crate::is_keyframe(&packets[0].data));
    let info = crate::keyframe_info(&packets[0].data).unwrap();
    assert_eq!((info.width, info.height, info.color.color_space), (100, 76, 2));
}

#[test]
fn inter_frames_are_smaller_than_key_frames() {
    let mut cfg = Config::new(128, 64);
    cfg.speed = 2;
    let (packets, _) = encode_all(cfg, 3);
    assert!(packets[1].data.len() * 2 < packets[0].data.len(), "{} vs {}", packets[1].data.len(), packets[0].data.len());
}

#[test]
fn quantizer_trades_size_for_quality() {
    let run = |q| {
        let mut cfg = Config::new(64, 64);
        cfg.rate = Rate::Quantizer(q);
        let (p, qual) = encode_all(cfg, 2);
        (p.iter().map(|p| p.data.len()).sum::<usize>(), qual[1])
    };
    let (small, low) = run(200);
    let (big, high) = run(20);
    assert!(small < big && low < high, "{small} B {low:.1} dB vs {big} B {high:.1} dB");
    assert!(high > 40.0, "{high:.2} dB");
}

#[test]
fn every_speed_produces_a_valid_stream() {
    for speed in 0..3 {
        let mut cfg = Config::new(72, 40);
        cfg.speed = speed;
        let (_, quality) = encode_all(cfg, 3);
        assert!(quality.iter().all(|q| *q > 30.0), "speed {speed}: {quality:?}");
    }
}

#[test]
fn the_bit_rate_is_followed() {
    let mut cfg = Config::new(96, 64);
    cfg.rate = Rate::Bitrate { bits_per_second: 200_000, fps_num: 25, fps_den: 1 };
    cfg.speed = 2;
    let (packets, _) = encode_all(cfg, 30);
    let bits: usize = packets.iter().map(|p| p.data.len() * 8).sum();
    let target = 200_000 * 30 / 25;
    assert!(bits < target * 2 && bits * 2 > target, "{bits} bits for a target of {target}");
}

/// Frequent key frames must not raise the average: their extra allowance is borrowed from the
/// following frames, not added to the budget.
#[test]
fn frequent_key_frames_keep_the_bit_rate() {
    for interval in [1, 4] {
        let mut cfg = Config::new(64, 64);
        cfg.rate = Rate::Bitrate { bits_per_second: 100_000, fps_num: 25, fps_den: 1 };
        cfg.keyframe_interval = interval;
        cfg.speed = 2;
        let (packets, _) = encode_all(cfg, 100);
        // after the controller settles
        let bits: usize = packets.iter().skip(40).map(|p| p.data.len() * 8).sum();
        let rate = bits as f64 * 25.0 / 60.0;
        assert!(rate < 140_000.0 && rate > 60_000.0, "interval {interval}: {rate:.0} bit/s for 100000");
    }
}

#[test]
fn tiny_frames_encode() {
    for (w, h) in [(1, 1), (2, 3), (9, 9), (17, 8)] {
        let (_, quality) = encode_all(Config::new(w, h), 2);
        assert!(quality.iter().all(|q| *q > 25.0), "{w}x{h}: {quality:?}");
    }
}

#[test]
fn wide_frames_use_several_tile_columns() {
    // more than 64 superblocks across need two tile columns
    let mut cfg = Config::new(4160, 16);
    cfg.speed = 2;
    let (_, quality) = encode_all(cfg, 2);
    assert!(quality.iter().all(|q| *q > 30.0), "{quality:?}");
}

#[test]
fn bad_input_is_an_error() {
    assert!(Encoder::new(Config::new(0, 10)).is_err());
    assert!(Encoder::new(Config::new(20000, 10)).is_err());
    let mut cfg = Config::new(16, 16);
    cfg.rate = Rate::Bitrate { bits_per_second: 1000, fps_num: 0, fps_den: 1 };
    assert!(Encoder::new(cfg).is_err());
    let mut enc = Encoder::new(Config::new(16, 16)).unwrap();
    let small = [vec![0u8; 100], vec![0u8; 64], vec![0u8; 64]];
    assert!(enc.encode(&input(&small, 16), false).is_err());
    let narrow = Input { y: &[0u8; 1000], u: &[0u8; 64], v: &[0u8; 64], y_stride: 8, uv_stride: 8 };
    assert!(enc.encode(&narrow, false).is_err());
}
