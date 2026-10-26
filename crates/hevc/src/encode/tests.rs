//! Encoder tests: round trips through the decoder (and ffmpeg when present).

use super::*;

/// Deterministic moving test content: gradients, a moving box and texture.
pub(super) fn content(w: usize, h: usize, t: usize) -> [Vec<u8>; 3] {
    let mut y = vec![0u8; w * h];
    let (cw, ch) = (w / 2, h / 2);
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    let mut seed = 0x1234_5678u32 ^ (t as u32);
    for yy in 0..h {
        for xx in 0..w {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 28) as i32 - 8;
            let gx = (xx + 2 * t) as i32;
            let mut val = 40 + (gx * 3 / 2 + yy as i32) % 140 + noise / 2;
            let (bx, by) = (20 + 3 * t, 30 + t);
            if xx >= bx && xx < bx + 40 && yy >= by && yy < by + 30 {
                val = 200 + ((xx ^ yy) & 7) as i32;
            }
            y[yy * w + xx] = val.clamp(0, 255) as u8;
        }
    }
    for yy in 0..ch {
        for xx in 0..cw {
            u[yy * cw + xx] = (100 + (xx + t) % 60) as u8;
            v[yy * cw + xx] = (90 + (yy * 2) % 80) as u8;
        }
    }
    [y, u, v]
}

pub(super) fn psnr(a: &[u16], b: &[u8], shift: u32) -> f64 {
    let mse: f64 = a.iter().zip(b).map(|(&x, &y)| ((x as f64 / (1 << shift) as f64) - y as f64).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

fn encode_clip(cfg: EncoderConfig, frames: usize) -> (Vec<Packet>, Vec<[Vec<u8>; 3]>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    let mut src = Vec::new();
    for t in 0..frames {
        let c = content(w, h, t);
        let f = YuvFrame::U8 { y: &c[0], u: &c[1], v: &c[2], y_stride: w, uv_stride: w / 2 };
        packets.extend(enc.encode(&f, t as i64).expect("encode"));
        src.push(c);
    }
    packets.extend(enc.flush().expect("flush"));
    (packets, src)
}

fn decode(packets: &[Packet]) -> Vec<crate::Picture> {
    let mut dec = Decoder::with_threads(1);
    let mut pics = Vec::new();
    for p in packets {
        pics.extend(dec.decode(&p.data, p.pts).expect("decode"));
    }
    pics.extend(dec.flush());
    assert!(dec.take_error().is_none());
    pics
}

fn plane16(p: &crate::Plane) -> Vec<u16> {
    (0..p.len()).map(|i| p.get(i)).collect()
}

#[test]
fn round_trip_8bit_constant_qp() {
    let mut cfg = EncoderConfig::new(176, 144, 25, 1);
    cfg.rate = RateControl::Qp(24);
    cfg.keyint = 0;
    let (packets, src) = encode_clip(cfg, 6);
    assert_eq!(packets.len(), 6);
    assert!(packets[0].keyframe && !packets[1].keyframe);
    let pics = decode(&packets);
    assert_eq!(pics.len(), 6);
    for (i, (p, s)) in pics.iter().zip(&src).enumerate() {
        assert_eq!((p.width, p.height), (176, 144));
        let py = psnr(&plane16(&p.y), &s[0], 0);
        let pu = psnr(&plane16(&p.u), &s[1], 0);
        assert!(py > 34.0 && pu > 34.0, "picture {i}: PSNR Y {py:.2} U {pu:.2}");
    }
    let bytes: usize = packets.iter().map(|p| p.data.len()).sum();
    assert!(bytes < 6 * 176 * 144 / 4, "{bytes} bytes");
}

fn stream(packets: &[Packet]) -> Vec<u8> {
    packets.iter().flat_map(|p| p.data.iter().copied()).collect()
}

/// Main 10 from 16-bit input: BT.2020 PQ colour description and HDR SEI, several slices.
#[test]
fn round_trip_main10_hdr() {
    let (w, h) = (192usize, 128usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 24, 1);
    cfg.bit_depth = 10;
    cfg.rate = RateControl::Qp(26);
    cfg.slices = 2;
    cfg.color = ColorConfig { primaries: 9, transfer: 16, matrix: 9, full_range: false };
    cfg.mastering_display =
        Some([0x33, 0xc2, 0x86, 0xc4, 0x1d, 0x4c, 0x0b, 0xb8, 0x84, 0xd0, 0x3e, 0x80, 0x3d, 0x13, 0x40, 0x42, 0x00, 0x98, 0x96, 0x80, 0x00, 0x00, 0x00, 0x32]);
    cfg.content_light = Some((1000, 400));
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    let mut src = Vec::new();
    for t in 0..4 {
        let c = content(w, h, t);
        let up: Vec<Vec<u16>> = c.iter().map(|p| p.iter().map(|&s| ((s as u16) << 2) | (t as u16 & 3)).collect()).collect();
        let f = YuvFrame::U16 { y: &up[0], u: &up[1], v: &up[2], y_stride: w, uv_stride: w / 2 };
        packets.extend(enc.encode(&f, t as i64).expect("encode"));
        src.push(c);
    }
    let pics = decode(&packets);
    assert_eq!(pics.len(), 4);
    for (i, (p, s)) in pics.iter().zip(&src).enumerate() {
        assert_eq!(p.bit_depth, 10);
        assert_eq!((p.color.primaries, p.color.transfer, p.color.matrix), (9, 16, 9));
        let py = psnr(&plane16(&p.y), &s[0], 2);
        assert!(py > 36.0, "picture {i}: PSNR Y {py:.2}");
    }
    let annexb = stream(&packets);
    // prefix SEI with mastering display (137) and content light level (144) payloads
    let sei = annexb.windows(6).any(|w| w[..4] == [0, 0, 1, 0x4e] && w[5] == 137);
    assert!(sei, "no HDR SEI");
    check_ffmpeg("main10_hdr", &annexb, &pics, 10);
}

/// Slices, IDR intervals and cropping of a size that is not a multiple of the minimum CU.
#[test]
fn slices_keyint_and_cropping() {
    let (w, h) = (170usize, 138usize);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.rate = RateControl::Crf(24.0);
    cfg.keyint = 3;
    cfg.slices = 3;
    cfg.refs = 3;
    cfg.preset = Preset::Quality;
    let (packets, src) = encode_clip(cfg.clone(), 7);
    let keys: Vec<bool> = packets.iter().map(|p| p.keyframe).collect();
    assert_eq!(keys, [true, false, false, true, false, false, true]);
    let pics = decode(&packets);
    assert_eq!(pics.len(), 7);
    for (i, (p, s)) in pics.iter().zip(&src).enumerate() {
        assert_eq!((p.width, p.height, p.chroma_width), (170, 138, 85));
        let py = psnr(&plane16(&p.y), &s[0], 0);
        assert!(py > 34.0, "picture {i}: PSNR Y {py:.2}");
    }
    // three slice NAL units per picture
    let annexb = stream(&packets[1..2]);
    let slices = annexb.windows(5).filter(|w| w[..4] == [0, 0, 0, 1] && (w[4] >> 1) & 0x3f == 1).count();
    assert_eq!(slices, 3);
    // deterministic for a given slice count
    let (again, _) = encode_clip(cfg, 7);
    assert_eq!(stream(&again), stream(&packets));
    check_ffmpeg("slices_crop", &stream(&packets), &pics, 8);
}

/// Average bitrate: CBR lands near its target and a lower target gives a smaller stream.
#[test]
fn cbr_hits_target() {
    let mut sizes = Vec::new();
    for kbps in [150u32, 400] {
        let mut cfg = EncoderConfig::new(176, 144, 25, 1);
        cfg.rate = RateControl::Cbr { kbps };
        let (packets, _) = encode_clip(cfg, 50);
        let bytes: usize = packets.iter().map(|p| p.data.len()).sum();
        let got = bytes as f64 * 8.0 / 2.0 / 1000.0;
        assert!((got - kbps as f64).abs() < 0.3 * kbps as f64, "target {kbps} kbps, got {got:.1}");
        sizes.push(bytes);
        decode(&packets);
    }
    assert!(sizes[0] < sizes[1]);
}

/// MP4 samples: length-prefixed NAL units without parameter sets, which come from the hvcC.
#[test]
fn length_prefixed_with_hvcc() {
    let mut cfg = EncoderConfig::new(176, 144, 25, 1);
    cfg.rate = RateControl::Vbr { target_kbps: 300, max_kbps: 600 };
    cfg.format = PacketFormat::LengthPrefixed;
    let enc = Encoder::new(cfg.clone()).expect("encoder");
    let hvcc = enc.hvcc();
    assert_eq!(hvcc[0], 1);
    assert_eq!(hvcc[1], 1); // Main
    assert_eq!(hvcc[21] & 3, 3); // 4-byte lengths
    // the arrays: VPS, SPS, PPS
    let mut annexb = Vec::new();
    let mut at = 23;
    for t in [32u8, 33, 34] {
        assert_eq!(hvcc[at] & 0x3f, t);
        let len = u16::from_be_bytes([hvcc[at + 3], hvcc[at + 4]]) as usize;
        annexb.extend([0, 0, 0, 1]);
        annexb.extend_from_slice(&hvcc[at + 5..at + 5 + len]);
        at += 5 + len;
    }
    assert_eq!(at, hvcc.len());
    let (packets, _) = encode_clip(cfg, 4);
    let mut converted = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        let mut d = if i == 0 { annexb.clone() } else { Vec::new() };
        let mut k = 0;
        while k + 4 <= p.data.len() {
            let n = u32::from_be_bytes([p.data[k], p.data[k + 1], p.data[k + 2], p.data[k + 3]]) as usize;
            let nal = &p.data[k + 4..k + 4 + n];
            assert!(!matches!((nal[0] >> 1) & 0x3f, 32..=34), "parameter set in a sample");
            d.extend([0, 0, 0, 1]);
            d.extend_from_slice(nal);
            k += 4 + n;
        }
        assert_eq!(k, p.data.len());
        converted.push(Packet { data: d, ..p.clone() });
    }
    assert_eq!(decode(&converted).len(), 4);
}

/// Our decoder and ffmpeg agree bit-exactly on the encoder's output (skipped without ffmpeg).
fn check_ffmpeg(name: &str, stream: &[u8], pics: &[crate::Picture], bits: u32) {
    let Some(ff) = cfa4_testkit::ffmpeg_or_skip(name) else { return };
    let dir = cfa4_testkit::fixtures_dir("hevc");
    let src = cfa4_testkit::temp_path(&dir.join(format!("enc_{name}.hevc")));
    let yuv = cfa4_testkit::temp_path(&dir.join(format!("enc_{name}.yuv")));
    std::fs::write(&src, stream).expect("write");
    let st = std::process::Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&src)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", if bits > 8 { "yuv420p10le" } else { "yuv420p" }])
        .arg(&yuv)
        .status()
        .expect("ffmpeg");
    assert!(st.success(), "ffmpeg failed on {name}");
    let r = std::fs::read(&yuv).expect("read");
    let mut ours = Vec::new();
    for p in pics {
        for pl in [&p.y, &p.u, &p.v] {
            for i in 0..pl.len() {
                let v = pl.get(i);
                if bits > 8 {
                    ours.extend(v.to_le_bytes());
                } else {
                    ours.push(v as u8);
                }
            }
        }
    }
    assert_eq!(r.len(), ours.len(), "{name}: ffmpeg output size");
    assert!(r == ours, "{name}: ffmpeg decodes differently");
    let _ = std::fs::remove_file(&yuv);
    let _ = std::fs::remove_file(&src);
}
