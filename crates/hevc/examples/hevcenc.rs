//! Encode raw 4:2:0 video (yuv420p, or yuv420p10le with HEVC_BITS=10) to an H.265 elementary stream.
//!
//! usage: hevcenc <in.yuv> <width> <height> <out.hevc> [qp=N | crf=N | kbps=N] [preset=speed|balanced|quality] [refs=N]
//!                [keyint=N] [slices=N]

use cfa4_hevc::encode::{Encoder, EncoderConfig, Preset, RateControl, YuvFrame};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: hevcenc <in.yuv> <width> <height> <out.hevc> [qp=N|crf=N|kbps=N] [preset=..] [refs=N] [keyint=N] [slices=N]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read input");
    let w: usize = args[2].parse().expect("width");
    let h: usize = args[3].parse().expect("height");
    let bits: u8 = std::env::var("HEVC_BITS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.bit_depth = bits;
    for a in &args[5..] {
        if let Some(v) = a.strip_prefix("qp=") {
            cfg.rate = RateControl::Qp(v.parse().expect("qp"));
        } else if let Some(v) = a.strip_prefix("crf=") {
            cfg.rate = RateControl::Crf(v.parse().expect("crf"));
        } else if let Some(v) = a.strip_prefix("kbps=") {
            cfg.rate = RateControl::Cbr { kbps: v.parse().expect("kbps") };
        } else if let Some(v) = a.strip_prefix("preset=") {
            cfg.preset = match v {
                "speed" => Preset::Speed,
                "quality" => Preset::Quality,
                _ => Preset::Balanced,
            };
        } else if let Some(v) = a.strip_prefix("refs=") {
            cfg.refs = v.parse().expect("refs");
        } else if let Some(v) = a.strip_prefix("keyint=") {
            cfg.keyint = v.parse().expect("keyint");
        } else if let Some(v) = a.strip_prefix("slices=") {
            cfg.slices = v.parse().expect("slices");
        }
    }
    let bps = if bits > 8 { 2 } else { 1 };
    let fsize = w * h * 3 / 2 * bps;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut out = Vec::new();
    let t0 = std::time::Instant::now();
    let mut n = 0;
    for (i, f) in data.chunks_exact(fsize).enumerate() {
        let pk = if bits > 8 {
            let s: Vec<u16> = f.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let (y, rest) = s.split_at(w * h);
            let (u, v) = rest.split_at(w * h / 4);
            enc.encode(&YuvFrame::U16 { y, u, v, y_stride: w, uv_stride: w / 2 }, i as i64)
        } else {
            let (y, rest) = f.split_at(w * h);
            let (u, v) = rest.split_at(w * h / 4);
            enc.encode(&YuvFrame::U8 { y, u, v, y_stride: w, uv_stride: w / 2 }, i as i64)
        }
        .expect("encode");
        for p in pk {
            out.extend(p.data);
        }
        n += 1;
    }
    let secs = t0.elapsed().as_secs_f64();
    eprintln!(
        "{n} pictures in {secs:.2}s ({:.2} fps), {} bytes ({:.1} kbps at 25 fps)",
        n as f64 / secs,
        out.len(),
        out.len() as f64 * 8.0 * 25.0 / n.max(1) as f64 / 1000.0
    );
    std::fs::write(&args[4], out).expect("write");
}
