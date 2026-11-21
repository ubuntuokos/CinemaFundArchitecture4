// SPDX-License-Identifier: GPL-3.0-or-later
//! Decodes a raw AVS3 elementary stream into planar YUV 4:2:0 (8-bit samples, or 16-bit little endian above
//! 8 bits): `avs3dec in.avs3 out.yuv`.

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: avs3dec in.avs3 out.yuv");
        std::process::exit(2);
    };
    let data = std::fs::read(input)?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    let mut dec = cfa4_avs3::Decoder::new();
    let mut write = |f: cfa4_avs3::Frame| -> std::io::Result<()> {
        for p in &f.planes {
            if f.bit_depth > 8 {
                for &s in p {
                    out.write_all(&s.to_le_bytes())?;
                }
            } else {
                out.write_all(&p.iter().map(|&s| s as u8).collect::<Vec<u8>>())?;
            }
        }
        Ok(())
    };
    for (n, packet) in cfa4_avs3::split_packets(&data).into_iter().enumerate() {
        for f in dec.decode(packet, n as i64)? {
            write(f)?;
        }
    }
    for f in dec.flush() {
        write(f)?;
    }
    Ok(())
}
