// SPDX-License-Identifier: GPL-3.0-or-later
//! Decodes a raw AVS2 stream to planar YUV (8-bit samples as bytes, 10-bit as 16-bit LE).
//! Usage: `avs2dec in.avs2 out.yuv`

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(output)) = (args.get(1), args.get(2)) else {
        return Err("usage: avs2dec in.avs2 out.yuv".into());
    };
    let data = std::fs::read(input)?;
    let mut dec = cfa4_avs2::Decoder::new();
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    let mut write = |f: &cfa4_avs2::Frame| -> std::io::Result<()> {
        for p in &f.planes {
            if f.bit_depth > 8 {
                for &v in p {
                    out.write_all(&v.to_le_bytes())?;
                }
            } else {
                let b: Vec<u8> = p.iter().map(|&v| v as u8).collect();
                out.write_all(&b)?;
            }
        }
        Ok(())
    };
    let mut n = 0;
    for (i, pkt) in cfa4_avs2::split_packets(&data).into_iter().enumerate() {
        match dec.decode(pkt, i as i64) {
            Ok(frames) => {
                for f in &frames {
                    write(f)?;
                    n += 1;
                }
            }
            Err(e) => eprintln!("packet {i}: {e}"),
        }
    }
    for f in &dec.flush() {
        write(f)?;
        n += 1;
    }
    eprintln!("{n} frames");
    Ok(())
}
