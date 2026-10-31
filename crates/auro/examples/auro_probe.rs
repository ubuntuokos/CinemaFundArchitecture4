// SPDX-License-Identifier: GPL-3.0-or-later
//! Probe and decode an Auro-3D carrier given as raw interleaved little-endian 32-bit PCM (24-bit
//! samples left-aligned, as `ffmpeg -f s32le` writes them), in WAV channel order:
//!
//! `cargo run -p cfa4-auro --example auro_probe -- carrier.s32 <channels> <rate> [out.s32]`
//!
//! Prints the detection, each carrier's block statistics and the level of every restored stream
//! per second; with `out.s32` it also writes the restored channels there in the same format.

use std::io::Write;

use cfa4_auro::cfa4::{unfold, wav_carrier_ids};
use cfa4_auro::decode::StreamId;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, channels, rate, rest @ ..] = args.as_slice() else {
        return Err("usage: auro_probe carrier.s32 <channels> <rate> [out.s32]".into());
    };
    let ch: usize = channels.parse().map_err(|e| format!("channels: {e}"))?;
    let rate: usize = rate.parse().map_err(|e| format!("rate: {e}"))?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if ch == 0 || wav_carrier_ids(ch).is_none() {
        return Err(format!("no Auro carrier has {ch} channels"));
    }
    let frames = bytes.len() / (4 * ch);
    let mut planes = vec![Vec::with_capacity(frames); ch];
    for (i, w) in bytes.chunks_exact(4).enumerate().take(frames * ch) {
        planes[i % ch].push(i32::from_le_bytes([w[0], w[1], w[2], w[3]]) >> 8);
    }
    let refs: Vec<&[i32]> = planes.iter().map(Vec::as_slice).collect();
    let mut so = std::io::stdout().lock();
    let Some(u) = unfold(&refs) else {
        let _ = writeln!(so, "no Auro-Codec carrier");
        return Ok(());
    };
    let d = u.detection;
    let mut lines = vec![
        format!(
            "config {} : {} -> {}, block {} samples, decode errors {}",
            d.config.0,
            d.carrier.name().unwrap_or("?"),
            d.original.name().unwrap_or("?"),
            d.block_size,
            u.decode_errors
        ),
        format!("outputs: {}", u.speakers.iter().map(|s| StreamId::name(*s)).collect::<Vec<_>>().join(" ")),
    ];
    let rate = rate.max(1);
    for s in 0..frames / rate {
        let mut line = format!("{s:4}s");
        for c in &u.channels {
            let e: f64 = c.iter().skip(s * rate).take(rate).map(|&v| (f64::from(v) / 8_388_608.0).powi(2)).sum();
            line += &format!(" {:6.1}", 10.0 * (e / rate as f64 + 1e-20).log10());
        }
        lines.push(line);
    }
    // a closed pipe (`| head`) ends the listing, not the program
    for line in lines {
        if writeln!(so, "{line}").is_err() {
            break;
        }
    }
    if let [out, ..] = rest {
        let mut b = Vec::with_capacity(frames * u.channels.len() * 4);
        for f in 0..frames {
            for c in &u.channels {
                b.extend_from_slice(&(c.get(f).copied().unwrap_or(0) << 8).to_le_bytes());
            }
        }
        std::fs::write(out, b).map_err(|e| format!("{out}: {e}"))?;
    }
    Ok(())
}
