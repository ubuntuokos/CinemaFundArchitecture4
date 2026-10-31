//! Encode time of the DCI profiles on a synthetic 12-bit frame:
//! `cargo run --release -p cfa4-j2k --example j2k_encode_bench [-- 2k|4k [OUT.j2c]]`.

use cfa4_j2k::{EncodeOptions, Image, Plane};

fn main() {
    let mut args = std::env::args().skip(1);
    let four_k = args.next().as_deref() == Some("4k");
    let (w, h) = if four_k { (4096u32, 2160u32) } else { (2048, 1080) };
    let mut seed = 1u32;
    let components = (0..3)
        .map(|c| {
            let data = (0..w * h)
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
                .collect();
            Plane { width: w, height: h, precision: 12, signed: false, dx: 1, dy: 1, data }
        })
        .collect();
    let img = Image { width: w, height: h, components, mct: false };
    let opts = if four_k { EncodeOptions::cinema_4k(24) } else { EncodeOptions::cinema_2k(24) };
    let start = std::time::Instant::now();
    match cfa4_j2k::encode(&img, &opts) {
        Ok(cs) => {
            println!("{w}x{h}: {} bytes in {:.0} ms", cs.len(), start.elapsed().as_secs_f64() * 1000.0);
            if let Some(out) = args.next()
                && std::fs::write(&out, &cs).is_err()
            {
                eprintln!("{out}: not written");
            }
        }
        Err(e) => eprintln!("{e}"),
    }
}
