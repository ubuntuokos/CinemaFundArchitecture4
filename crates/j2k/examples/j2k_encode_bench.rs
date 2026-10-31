//! Encode time of the DCI profiles:
//! `cargo run --release -p cfa4-j2k --example j2k_encode_bench [-- 2k|4k [OUT.j2c] [--raw FRAME.rgb48]]`.
//!
//! Without `--raw` the frame is synthetic (gradients, a texture and grain). `--raw` reads a
//! 4096 × 2160 `rgb48le` frame (`ffmpeg -i IN -frames:v 1 -vf scale=4096:2160,format=rgb48le -f
//! rawvideo FRAME.rgb48`), taken to 12 bits; 2K uses every other sample of it. The PSNR of the
//! decoded codestream is printed too.

use cfa4_j2k::{EncodeOptions, Image, Plane};

fn synthetic(w: u32, h: u32) -> Vec<Vec<i32>> {
    let mut seed = 1u32;
    (0..3)
        .map(|c| {
            (0..w * h)
                .map(|k| {
                    let (x, y) = (k % w, k / w);
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    // gradients, a texture and grain
                    let base = (x * 3 + y * 2 + c * 500) % 4096;
                    let tex = if (x / 37 + y / 23) % 2 == 0 { 300 } else { 0 };
                    ((base + tex + seed % 64) % 4096) as i32
                })
                .collect()
        })
        .collect()
}

fn raw(path: &str, w: u32, h: u32) -> Option<Vec<Vec<i32>>> {
    let bytes = std::fs::read(path).ok()?;
    let (sw, sh) = (4096usize, 2160usize);
    if bytes.len() < sw * sh * 6 {
        return None;
    }
    let step = sw / w as usize;
    let mut planes: Vec<Vec<i32>> = (0..3).map(|_| Vec::with_capacity((w * h) as usize)).collect();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = ((y * step) * sw + x * step) * 6;
            for (c, p) in planes.iter_mut().enumerate() {
                let v = u16::from_le_bytes([*bytes.get(at + 2 * c)?, *bytes.get(at + 2 * c + 1)?]);
                p.push(i32::from(v >> 4));
            }
        }
    }
    Some(planes)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let four_k = args.first().map(String::as_str) == Some("4k");
    let (w, h) = if four_k { (4096u32, 2160u32) } else { (2048, 1080) };
    let raw_path = args.iter().position(|a| a == "--raw").and_then(|i| args.get(i + 1));
    let out = args.get(1).filter(|a| !a.starts_with("--"));
    let data = match raw_path {
        Some(p) => match raw(p, w, h) {
            Some(d) => d,
            None => {
                eprintln!("{p}: not a 4096x2160 rgb48le frame");
                return;
            }
        },
        None => synthetic(w, h),
    };
    let components = data.into_iter().map(|data| Plane { width: w, height: h, precision: 12, signed: false, dx: 1, dy: 1, data }).collect();
    let img = Image { width: w, height: h, components, mct: false };
    let opts = if four_k { EncodeOptions::cinema_4k(24) } else { EncodeOptions::cinema_2k(24) };
    let start = std::time::Instant::now();
    match cfa4_j2k::encode(&img, &opts) {
        Ok(cs) => {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let psnr = match cfa4_j2k::decode(&cs) {
                Ok(dec) => {
                    let (mut se, mut n) = (0f64, 0f64);
                    for (a, b) in img.components.iter().zip(&dec.components) {
                        for (&x, &y) in a.data.iter().zip(&b.data) {
                            se += f64::from(x - y).powi(2);
                            n += 1.0;
                        }
                    }
                    10.0 * (4095f64.powi(2) / (se / n.max(1.0)).max(1e-12)).log10()
                }
                Err(_) => f64::NAN,
            };
            println!("{w}x{h}: {} bytes in {ms:.0} ms, PSNR {psnr:.3} dB", cs.len());
            if let Some(out) = out
                && std::fs::write(out, &cs).is_err()
            {
                eprintln!("{out}: not written");
            }
        }
        Err(e) => eprintln!("{e}"),
    }
}
