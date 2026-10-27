//! Encode raw 4:2:0 video (yuv420p, or yuv420p10le with AV1_BITS=10) to an IVF AV1 file.
//!
//! usage: av1enc <in.yuv> <width> <height> <out.ivf> [q=N | crf=N | kbps=N] [preset=speed|balanced|quality]
//!               [keyint=N] [tiles=N] [threads=N]

use cfa4_av1::encode::{Encoder, EncoderConfig, Preset, RateControl, YuvFrame};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: av1enc <in.yuv> <width> <height> <out.ivf> [q=N|crf=N|kbps=N] [preset=..] [keyint=N] [tiles=N] [threads=N]");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read input");
    let w: usize = args[2].parse().expect("width");
    let h: usize = args[3].parse().expect("height");
    let bits: u8 = std::env::var("AV1_BITS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.bit_depth = bits;
    for a in &args[5..] {
        if let Some(v) = a.strip_prefix("q=") {
            cfg.rate = RateControl::QIndex(v.parse().expect("q"));
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
        } else if let Some(v) = a.strip_prefix("keyint=") {
            cfg.keyint = v.parse().expect("keyint");
        } else if let Some(v) = a.strip_prefix("tiles=") {
            cfg.tiles = v.parse().expect("tiles");
        } else if let Some(v) = a.strip_prefix("threads=") {
            cfg.threads = v.parse().expect("threads");
        }
    }
    let bps = if bits > 8 { 2 } else { 1 };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let fsize = (w * h + 2 * cw * ch) * bps;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let t0 = std::time::Instant::now();
    for (i, f) in data.chunks_exact(fsize).enumerate() {
        let pk = if bits > 8 {
            let s: Vec<u16> = f.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            let (y, rest) = s.split_at(w * h);
            let (u, v) = rest.split_at(cw * ch);
            enc.encode(&YuvFrame::U16 { y, u, v, y_stride: w, uv_stride: cw }, i as i64)
        } else {
            let (y, rest) = f.split_at(w * h);
            let (u, v) = rest.split_at(cw * ch);
            enc.encode(&YuvFrame::U8 { y, u, v, y_stride: w, uv_stride: cw }, i as i64)
        }
        .expect("encode");
        frames.extend(pk.into_iter().map(|p| p.data));
    }
    let secs = t0.elapsed().as_secs_f64();
    let n = frames.len();
    let total: usize = frames.iter().map(Vec::len).sum();
    eprintln!(
        "{n} frames in {secs:.2}s ({:.2} fps), {total} bytes ({:.1} kbps at 25 fps)",
        n as f64 / secs,
        total as f64 * 8.0 * 25.0 / n.max(1) as f64 / 1000.0
    );
    let mut out = Vec::new();
    out.extend_from_slice(b"DKIF");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"AV01");
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.extend_from_slice(&25u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        out.extend_from_slice(&(f.len() as u32).to_le_bytes());
        out.extend_from_slice(&(i as u64).to_le_bytes());
        out.extend_from_slice(f);
    }
    std::fs::write(&args[4], out).expect("write");
}
