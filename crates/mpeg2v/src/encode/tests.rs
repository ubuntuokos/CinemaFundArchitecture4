use super::*;
use crate::{Picture, access_units};

/// A planar test picture: gradients, a textured square moving `t` samples per frame and a little
/// noise.
struct Clip {
    w: usize,
    cw: usize,
    ch: usize,
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
}

fn clip(w: usize, h: usize, chroma: ChromaFormat, t: usize) -> Clip {
    textured(w, h, chroma, t, 4)
}

/// Like [`clip`] with a texture of ±`amp`.
fn textured(w: usize, h: usize, chroma: ChromaFormat, t: usize, amp: u32) -> Clip {
    let (cw, ch) = (w.div_ceil(2), if chroma == ChromaFormat::Yuv420 { h.div_ceil(2) } else { h });
    // texture hashed from scene coordinates, so it moves with the content
    let noise = |x: usize, y: usize| {
        let mut v = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2C1B_3C6D);
        v ^= v >> 12;
        (v % (2 * amp + 1)) as i32 - amp as i32
    };
    let (sx, sy) = (w / 6 + 3 * t, h / 5 + t);
    let side = w.min(h) / 3;
    let inside = |x: usize, y: usize| x >= sx && x < sx + side && y >= sy && y < sy + side;
    let mut y = vec![0u8; w * h];
    for r in 0..h {
        for c in 0..w {
            let v = if inside(c, r) {
                (60 + ((c - sx) / 4 + (r - sy) / 4) % 2 * 120) as i32 + noise(c - sx, r - sy + 5000)
            } else {
                (40 + (c + t) * 140 / (w + 64) + r * 50 / h) as i32 + noise(c + t, r)
            };
            y[r * w + c] = v.clamp(0, 255) as u8;
        }
    }
    let (fx, fy) = (w / cw.max(1), h / ch.max(1));
    let mut cb = vec![0u8; cw * ch];
    let mut cr = vec![0u8; cw * ch];
    for r in 0..ch {
        for c in 0..cw {
            let sq = inside(c * fx, r * fy);
            cb[r * cw + c] = if sq { 90 } else { (100 + c * 60 / cw) as u8 };
            cr[r * cw + c] = if sq { 170 } else { (150 - r * 40 / ch) as u8 };
        }
    }
    Clip { w, cw, ch, y, cb, cr }
}

fn input(c: &Clip) -> Input<'_> {
    Input { y: &c.y, cb: &c.cb, cr: &c.cr, y_stride: c.w, c_stride: c.cw }
}

fn encode(cfg: Config, frames: usize) -> (Vec<u8>, Vec<Packet>, Vec<Clip>) {
    let mut enc = Encoder::new(cfg.clone()).unwrap();
    let mut es = Vec::new();
    let mut packets = Vec::new();
    let mut clips = Vec::new();
    for t in 0..frames {
        let c = clip(cfg.width as usize, cfg.height as usize, cfg.chroma, t);
        let p = enc.encode(&input(&c)).unwrap();
        es.extend_from_slice(&p.data);
        packets.push(p);
        clips.push(c);
    }
    es.extend_from_slice(&enc.finish());
    (es, packets, clips)
}

fn decode(es: &[u8]) -> Vec<Picture> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for (i, r) in access_units(es).into_iter().enumerate() {
        out.extend(d.decode(&es[r], i as i64).unwrap());
    }
    out.extend(d.flush());
    assert_eq!(d.errors(), 0, "{:?}", d.last_error());
    out
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sse: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
    10.0 * (255f64 * 255.0 * a.len() as f64 / sse.max(1e-9)).log10()
}

#[test]
fn round_trips_through_our_decoder() {
    let mut cfg = Config::new(176, 144, 25, 1);
    cfg.gop = 6;
    cfg.rate = Rate::Quantizer(3);
    let (es, packets, clips) = encode(cfg, 10);
    let pics = decode(&es);
    assert_eq!(pics.len(), 10);
    let info = &pics[0].info;
    assert_eq!((info.width, info.height, info.chroma), (176, 144, ChromaFormat::Yuv420));
    assert_eq!(info.profile_and_level, Some(0x48));
    assert_eq!(info.frame_rate, Some((25, 1)));
    assert!(info.low_delay && info.progressive_sequence);
    for (i, (p, c)) in pics.iter().zip(&clips).enumerate() {
        let want = if i % 6 == 0 { PictureType::I } else { PictureType::P };
        assert_eq!(p.picture_type, want);
        assert_eq!(packets[i].keyframe, want == PictureType::I);
        let (py, pc) = (psnr(&p.y, &c.y), psnr(&p.cb, &c.cb).min(psnr(&p.cr, &c.cr)));
        assert!(py > 38.0 && pc > 38.0, "frame {i}: PSNR Y {py:.2} C {pc:.2}");
    }
}

#[test]
fn motion_compensation_predicts_moving_content() {
    let (w, h) = (256, 160);
    let mut enc = Encoder::new(Config::new(w as u32, h as u32, 25, 1)).unwrap();
    let sizes: Vec<usize> = (0..5).map(|t| enc.encode(&input(&textured(w, h, ChromaFormat::Yuv420, t, 0))).unwrap().data.len()).collect();
    let p = sizes[1..].iter().max().unwrap();
    assert!(p * 2 < sizes[0], "{sizes:?}");
}

#[test]
fn still_pictures_skip_almost_everything() {
    let cfg = Config::new(320, 240, 30000, 1001);
    let c = clip(320, 240, ChromaFormat::Yuv420, 0);
    let mut enc = Encoder::new(cfg).unwrap();
    let i = enc.encode(&input(&c)).unwrap();
    // the first P picture refines the I picture's quantisation error, then nothing is left
    let p1 = enc.encode(&input(&c)).unwrap();
    let p2 = enc.encode(&input(&c)).unwrap();
    assert!(p1.data.len() < i.data.len() && p2.data.len() * 8 < i.data.len(), "I {} P {} P {}", i.data.len(), p1.data.len(), p2.data.len());
}

#[test]
fn four_two_two_profile() {
    let mut cfg = Config::new(192, 112, 50, 1);
    cfg.chroma = ChromaFormat::Yuv422;
    cfg.intra_dc_precision = 11;
    cfg.colour = Some((1, 1, 1));
    cfg.aspect = AspectRatio::Dar16x9;
    cfg.rate = Rate::Quantizer(2);
    let (es, _, clips) = encode(cfg, 5);
    let pics = decode(&es);
    assert_eq!(pics.len(), 5);
    let info = &pics[0].info;
    assert_eq!(info.profile_and_level, Some(0x85));
    assert_eq!(info.chroma, ChromaFormat::Yuv422);
    assert_eq!(info.colour, Some((1, 1, 1)));
    assert_eq!(info.dar, Some((16, 9)));
    for (p, c) in pics.iter().zip(&clips) {
        assert_eq!((p.cb.len(), p.chroma_height()), (c.cw * c.ch, 112));
        assert!(psnr(&p.y, &c.y) > 42.0 && psnr(&p.cr, &c.cr) > 42.0);
    }
}

#[test]
fn odd_sizes_are_padded_and_cropped() {
    for (w, h) in [(17, 9), (101, 77), (33, 130)] {
        let mut cfg = Config::new(w, h, 24, 1);
        cfg.rate = Rate::Quantizer(2);
        let (es, _, clips) = encode(cfg, 3);
        let pics = decode(&es);
        assert_eq!(pics.len(), 3);
        for (p, c) in pics.iter().zip(&clips) {
            assert_eq!((p.width, p.height), (w, h));
            assert!(psnr(&p.y, &c.y) > 40.0, "{w}x{h}");
            assert!(psnr(&p.cb, &c.cb) > 40.0, "{w}x{h}");
        }
    }
}

#[test]
fn bitrate_control_hits_the_target() {
    for (bps, chroma) in
        [(1_500_000u64, ChromaFormat::Yuv420), (2_500_000, ChromaFormat::Yuv420), (2_000_000, ChromaFormat::Yuv422), (400_000, ChromaFormat::Yuv420)]
    {
        let mut cfg = Config::new(352, 288, 25, 1);
        cfg.chroma = chroma;
        cfg.gop = 12;
        cfg.rate = Rate::Bitrate { bits_per_second: bps };
        let frames = 48;
        let (es, _, _) = encode(cfg.clone(), frames);
        let actual = es.len() as f64 * 8.0 * 25.0 / frames as f64;
        let ratio = actual / bps as f64;
        assert!((0.85..1.15).contains(&ratio), "{bps}: {actual:.0} bit/s");
        assert_eq!(Encoder::new(cfg).unwrap().signalled_bit_rate(), bps);
        assert_eq!(decode(&es).len(), frames);
    }
}

#[test]
fn levels_follow_size_and_rate() {
    let pli = |w, h, n, d, rate| {
        let mut c = Config::new(w, h, n, d);
        c.rate = rate;
        Encoder::new(c).unwrap().profile_and_level()
    };
    assert_eq!(pli(720, 576, 25, 1, Rate::Quantizer(4)), 0x48);
    assert_eq!(pli(720, 576, 25, 1, Rate::Bitrate { bits_per_second: 20_000_000 }), 0x46);
    assert_eq!(pli(1440, 1080, 30000, 1001, Rate::Quantizer(4)), 0x46);
    assert_eq!(pli(1920, 1080, 30000, 1001, Rate::Quantizer(4)), 0x44);
    assert_eq!(pli(3840, 2160, 25, 1, Rate::Quantizer(4)), 0x44);
}

#[test]
fn rejects_what_mpeg2_cannot_carry() {
    let bad = |f: &dyn Fn(&mut Config)| {
        let mut c = Config::new(64, 64, 25, 1);
        f(&mut c);
        Encoder::new(c).is_err()
    };
    assert!(bad(&|c| c.fps_num = 15));
    assert!(bad(&|c| c.fps_den = 0));
    assert!(bad(&|c| c.chroma = ChromaFormat::Yuv444));
    assert!(bad(&|c| c.width = 0));
    assert!(bad(&|c| c.height = 4096));
    assert!(bad(&|c| c.intra_dc_precision = 11));
    assert!(bad(&|c| c.rate = Rate::Quantizer(0)));
    assert!(bad(&|c| c.rate = Rate::Quantizer(32)));
    assert!(bad(&|c| c.rate = Rate::Bitrate { bits_per_second: 10 }));
    assert!(bad(&|c| c.gop = 0));
    assert!(!bad(&|c| {
        c.fps_num = 60000;
        c.fps_den = 1001
    }));
    assert!(!bad(&|c| {
        c.fps_num = 48000;
        c.fps_den = 2002
    }));
    // short planes
    let mut enc = Encoder::new(Config::new(64, 64, 25, 1)).unwrap();
    let y = vec![0u8; 64 * 63];
    let c = vec![0u8; 32 * 32];
    assert!(enc.encode(&Input { y: &y, cb: &c, cr: &c, y_stride: 64, c_stride: 32 }).is_err());
}

#[test]
fn extreme_content_stays_decodable() {
    // full-swing noise at the finest quantiser: escapes, large DC differences, every level
    let (w, h) = (64usize, 48usize);
    let mut cfg = Config::new(w as u32, h as u32, 25, 1);
    cfg.rate = Rate::Quantizer(1);
    cfg.intra_dc_precision = 10;
    cfg.gop = 3;
    let mut enc = Encoder::new(cfg).unwrap();
    let mut es = Vec::new();
    let mut s = 1u32;
    let mut clips = Vec::new();
    for t in 0..5 {
        let mut px = |_: usize| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if t % 2 == 0 { (s >> 24) as u8 } else { [0, 255][(s >> 31) as usize] }
        };
        let y: Vec<u8> = (0..w * h).map(&mut px).collect();
        let cb: Vec<u8> = (0..w * h / 4).map(&mut px).collect();
        let cr: Vec<u8> = (0..w * h / 4).map(&mut px).collect();
        es.extend(enc.encode(&Input { y: &y, cb: &cb, cr: &cr, y_stride: w, c_stride: w / 2 }).unwrap().data);
        clips.push(y);
    }
    let pics = decode(&es);
    for (p, y) in pics.iter().zip(&clips) {
        assert!(psnr(&p.y, y) > 40.0);
    }
}

/// ffmpeg decodes our streams to the same pictures as our decoder (up to IDCT rounding).
#[test]
fn ffmpeg_decodes_our_streams() {
    for (name, chroma, rate) in [
        ("enc_420_q", ChromaFormat::Yuv420, Rate::Quantizer(4)),
        ("enc_420_rate", ChromaFormat::Yuv420, Rate::Bitrate { bits_per_second: 800_000 }),
        ("enc_422_q", ChromaFormat::Yuv422, Rate::Quantizer(3)),
    ] {
        let mut cfg = Config::new(160, 96, 25, 1);
        cfg.chroma = chroma;
        cfg.rate = rate;
        cfg.gop = 5;
        let (es, _, _) = encode(cfg, 8);
        let ours = decode(&es);
        let Some(ff) = cfa4_testkit::ffmpeg_or_skip(name) else { return };
        let path = cfa4_testkit::fixtures_dir("mpeg2v").join(format!("{name}.m2v"));
        std::fs::write(&path, &es).unwrap();
        let fmt = if chroma == ChromaFormat::Yuv422 { "yuv422p" } else { "yuv420p" };
        let o = std::process::Command::new(ff)
            .args(["-hide_banner", "-loglevel", "error", "-xerror", "-i"])
            .arg(&path)
            .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", fmt, "-"])
            .output()
            .unwrap();
        assert!(o.status.success(), "{name}: ffmpeg rejects the stream: {}", String::from_utf8_lossy(&o.stderr));
        let fs = ours[0].y.len() + 2 * ours[0].cb.len();
        assert_eq!(o.stdout.len(), fs * ours.len(), "{name}: frame count");
        let mut worst = 0u8;
        for (p, f) in ours.iter().zip(o.stdout.chunks(fs)) {
            for (a, b) in p.y.iter().chain(&p.cb).chain(&p.cr).zip(f) {
                worst = worst.max(a.abs_diff(*b));
            }
        }
        assert!(worst <= 2, "{name}: ffmpeg differs by up to {worst}");
    }
}
