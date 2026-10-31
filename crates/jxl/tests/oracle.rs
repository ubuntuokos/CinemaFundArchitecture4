//! JPEG XL against libjxl (an external test oracle): `cjxl` encodes pictures with many coding
//! tools, `djxl` decodes them, and our decode must give the same samples within a few 16-bit
//! levels; lossless files must give the source exactly.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_jxl::cfa4::{Picture, decode};

fn tool(name: &str) -> Option<PathBuf> {
    let found = cfa4_testkit::oracle::find_tool(name);
    if found.is_none() {
        cfa4_testkit::skip(&format!("jxl oracle: {name}"), &format!("{name} not found (Ubuntu package libjxl-tools)"));
    }
    found
}

/// `args` for this `cjxl`: libjxl 0.8 renamed `--photon_noise ISO<n>` to `--photon_noise_iso=<n>`
/// (Ubuntu 24.04 ships 0.7, newer releases the new spelling).
fn cjxl_args(cjxl: &Path, args: &[&str]) -> Vec<String> {
    let help = Command::new(cjxl)
        .args(["-h", "-v", "-v", "-v", "-v"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
        .unwrap_or_default();
    let renamed = help.contains("--photon_noise_iso");
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (*a, renamed) {
            ("--photon_noise", true) => {
                let iso = it.next().map_or("", |v| v.trim_start_matches("ISO"));
                out.push(format!("--photon_noise_iso={iso}"));
            }
            _ => out.push((*a).to_owned()),
        }
    }
    out
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Gradients, a checkerboard, noise and (with alpha) a soft hole.
fn source(w: u32, h: u32, channels: u32, sixteen: bool) -> image::DynamicImage {
    let mut s = 0x1234_5678u32;
    let mut px = |x: u32, y: u32, c: u32| -> u16 {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (s >> 22) & 0xFF;
        let v = match c {
            0 => x * 65535 / w,
            1 => {
                if (x / 8 + y / 8).is_multiple_of(2) {
                    9000
                } else {
                    52000
                }
            }
            2 => y * 65535 / h,
            _ => {
                let d = (x as i64 - w as i64 / 2).pow(2) + (y as i64 - h as i64 / 2).pow(2);
                (d * 300).min(65535) as u32
            }
        };
        (v + noise * 40).min(65535) as u16
    };
    let n = (w * h * channels) as usize;
    let mut buf = Vec::with_capacity(n);
    for y in 0..h {
        for x in 0..w {
            for c in 0..channels {
                let c = if channels == 2 && c == 1 { 3 } else { c };
                buf.push(px(x, y, c));
            }
        }
    }
    let img = match channels {
        1 => image::DynamicImage::ImageLuma16(image::ImageBuffer::from_raw(w, h, buf).unwrap()),
        2 => image::DynamicImage::ImageLumaA16(image::ImageBuffer::from_raw(w, h, buf).unwrap()),
        3 => image::DynamicImage::ImageRgb16(image::ImageBuffer::from_raw(w, h, buf).unwrap()),
        _ => image::DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(w, h, buf).unwrap()),
    };
    if sixteen {
        img
    } else {
        match channels {
            1 => image::DynamicImage::ImageLuma8(img.to_luma8()),
            2 => image::DynamicImage::ImageLumaA8(img.to_luma_alpha8()),
            3 => image::DynamicImage::ImageRgb8(img.to_rgb8()),
            _ => image::DynamicImage::ImageRgba8(img.to_rgba8()),
        }
    }
}

/// libjxl's decode at the file's bit depth (8 or 16), as 16-bit samples (interleaved, `channels`
/// per pixel), and the bit depth.
fn djxl(djxl: &Path, jxl: &Path, png: &Path) -> (u32, u32, u32, Vec<u16>, u32) {
    run(Command::new(djxl).arg(jxl).arg(png).arg("--quiet"));
    let img = image::open(png).unwrap();
    let bits = if matches!(img.color(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::Rgb8 | image::ColorType::Rgba8) { 8 } else { 16 };
    let (w, h) = (img.width(), img.height());
    let channels = u32::from(img.color().channel_count());
    let samples = match channels {
        1 => img.to_luma16().into_raw(),
        2 => img.to_luma_alpha16().into_raw(),
        3 => img.to_rgb16().into_raw(),
        _ => img.to_rgba16().into_raw(),
    };
    (w, h, channels, samples, bits)
}

/// Our samples quantised to `bits` and widened to 16 bits like the PNG reader widens them.
fn to16(p: &Picture, bits: u32) -> Vec<u16> {
    let max = ((1u32 << bits) - 1) as f32;
    let widen = 65535 / ((1u32 << bits) - 1);
    p.samples.iter().map(|v| ((v.clamp(0.0, 1.0) * max).round() as u32 * widen) as u16).collect()
}

#[test]
fn decodes_like_djxl() {
    let (Some(cjxl), Some(djxl_path)) = (tool("cjxl"), tool("djxl")) else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("jxl/oracle");
    // (name, size, channels, 16-bit source, cjxl options, largest 16-bit difference)
    let cases: &[(&str, (u32, u32), u32, bool, &[&str], u16)] = &[
        ("vardct_d1", (157, 93), 3, false, &["-d", "1"], 257),
        ("vardct_d3_e7", (300, 200), 3, false, &["-d", "3", "-e", "7"], 257),
        ("vardct_d05_e3", (64, 257), 3, false, &["-d", "0.5", "-e", "3"], 257),
        ("vardct_alpha", (130, 70), 4, false, &["-d", "1.5"], 257),
        ("vardct_progressive", (200, 120), 3, false, &["-d", "1", "-p"], 257),
        ("vardct_resampled", (200, 120), 3, false, &["-d", "15"], 257),
        ("vardct_epf3_nogab", (160, 96), 3, false, &["-d", "2", "--epf", "3", "--gaborish", "0"], 257),
        // Photon noise: our synthesis matches jxl-oxide exactly, libjxl 0.7 puts a different
        // grain on a few percent of the pixels (the mean check below still holds).
        ("vardct_noise", (160, 96), 3, false, &["-d", "1", "--photon_noise", "ISO3200"], 3084),
        ("vardct_patches", (256, 160), 3, false, &["-d", "1", "--patches", "1", "-e", "7"], 257),
        ("vardct_16bit", (120, 80), 3, true, &["-d", "1"], 257),
        ("vardct_grey", (90, 61), 1, false, &["-d", "1"], 257),
        ("modular_lossy", (150, 100), 3, false, &["-m", "1", "-d", "1"], 257),
        ("lossless", (157, 93), 3, false, &["-d", "0"], 0),
        ("lossless_e9", (100, 60), 4, false, &["-d", "0", "-e", "9"], 0),
        ("lossless_16bit", (90, 61), 3, true, &["-d", "0"], 0),
        ("lossless_grey_alpha", (90, 61), 2, false, &["-d", "0"], 0),
        ("lossless_e1", (300, 300), 3, false, &["-d", "0", "-e", "1"], 0),
    ];
    let mut failures = Vec::new();
    for (name, (w, h), channels, sixteen, args, tolerance) in cases {
        let png = dir.join(format!("{name}.src.png"));
        let jxl = dir.join(format!("{name}.jxl"));
        let out = dir.join(format!("{name}.djxl.png"));
        let src = source(*w, *h, *channels, *sixteen);
        src.save(&png).unwrap();
        run(Command::new(&cjxl).arg(&png).arg(&jxl).args(cjxl_args(&cjxl, args)).arg("--quiet"));
        let (dw, dh, dc, want, bits) = djxl(&djxl_path, &jxl, &out);
        let got = match decode(&std::fs::read(&jxl).unwrap()) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        assert_eq!((got.width, got.height, got.channels), (dw, dh, dc), "{name}");
        let got16 = to16(&got, bits);
        let worst = got16.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let over = got16.iter().zip(&want).filter(|(a, b)| a.abs_diff(**b) > 257).count();
        let mean = got16.iter().zip(&want).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / want.len().max(1) as f64;
        eprintln!("{name}: {bits}-bit, worst {worst}, mean {mean:.1}, {over} of {} over one 8-bit level", want.len());
        if worst > *tolerance {
            failures.push(format!("{name}: worst {worst} > {tolerance}"));
        }
        if mean > 257.0 {
            failures.push(format!("{name}: mean difference {mean:.1} over one 8-bit level"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
