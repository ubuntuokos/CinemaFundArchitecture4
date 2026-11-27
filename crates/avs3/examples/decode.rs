// SPDX-License-Identifier: GPL-3.0-or-later
//! Decodes an AVS3 elementary stream to raw planar YUV (8-bit samples as bytes, 10-bit as `u16` LE).
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: decode <in.avs3> <out.yuv>");
        return;
    };
    let Ok(data) = std::fs::read(&input) else {
        eprintln!("cannot read {input}");
        return;
    };
    let Ok(mut out) = std::fs::File::create(&output) else {
        eprintln!("cannot write {output}");
        return;
    };
    if std::env::var_os("CFA4_INFO").is_some() {
        eprintln!("{:?}", cfa4_avs3::stream_info(&data));
    }
    let mut dec = cfa4_avs3::Decoder::new();
    let mut put = |f: &cfa4_avs3::Frame| {
        for p in &f.planes {
            let bytes: Vec<u8> = if f.bit_depth > 8 { p.iter().flat_map(|v| v.to_le_bytes()).collect() } else { p.iter().map(|&v| v as u8).collect() };
            let _ = out.write_all(&bytes);
        }
    };
    for (i, pkt) in cfa4_avs3::split_packets(&data).into_iter().enumerate() {
        match dec.decode(pkt, i as i64) {
            Ok(fs) => fs.iter().for_each(&mut put),
            Err(e) => eprintln!("packet {i}: {e}"),
        }
    }
    dec.flush().iter().for_each(&mut put);
}
