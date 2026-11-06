//! WebP stills and animations against libwebp (an external test oracle, #87): `cwebp` and
//! `img2webp` write the files, `dwebp` and `anim_dump` decode them, and our pictures must be the
//! ones libwebp gives.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_frame::PixelData;
use cfa4_media::{FrameRequest, MediaKind, MediaSource};
use cfa4_time::{TICKS_PER_SECOND, Tick};

const W: u32 = 61; // odd sizes exercise the chroma edge and the lossless transforms' last block
const H: u32 = 37;

fn tool(name: &str) -> Option<PathBuf> {
    let found = cfa4_testkit::oracle::find_tool(name);
    if found.is_none() {
        cfa4_testkit::skip(&format!("webp_oracle: {name}"), &format!("{name} not found (Ubuntu package `webp`)"));
    }
    found
}

/// A picture with smooth gradients, hard edges, noise and (when `alpha`) a soft transparent hole.
fn picture(seed: u32, alpha: bool) -> Vec<u8> {
    picture_sized(W, H, seed, alpha)
}

fn picture_sized(w: u32, h: u32, seed: u32, alpha: bool) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (s >> 27) as u8;
            let r = (x * 255 / w) as u8 ^ (seed as u8).wrapping_mul(37);
            let g = if (x / 8 + y / 8) % 2 == 0 { 30 } else { 220 };
            let b = (y * 255 / h) as u8;
            let d = (x as i32 - w as i32 / 2).pow(2) + (y as i32 - h as i32 / 2).pow(2);
            let a = if alpha { (d * 2).clamp(0, 255) as u8 } else { 255 };
            px.extend_from_slice(&[r.wrapping_add(noise), g, b.wrapping_sub(noise), a]);
        }
    }
    px
}

fn write_png(path: &Path, rgba: &[u8]) {
    write_png_sized(path, W, H, rgba);
}

fn write_png_sized(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap().save(path).unwrap();
}

/// `px` with the colour of fully transparent pixels zeroed (anim_dump does this before writing).
fn clean(mut px: Vec<u8>) -> Vec<u8> {
    for p in px.as_chunks_mut::<4>().0 {
        if p[3] == 0 {
            *p = [0; 4];
        }
    }
    px
}

/// Every picture anim_dump wrote for `webp` (removed afterwards), cleaned.
fn anim_dump_pictures(anim_dump: &Path, dir: &Path, webp: &Path, prefix: &str) -> Vec<(u32, u32, Vec<u8>)> {
    run(Command::new(anim_dump).arg("-pam").arg("-folder").arg(dir).arg("-prefix").arg(prefix).arg(webp));
    let mut out = Vec::new();
    for n in 0.. {
        let path = dir.join(format!("{prefix}{n:04}.pam"));
        if !path.exists() {
            break;
        }
        let (w, h, px) = read_pam(&path);
        std::fs::remove_file(&path).unwrap();
        out.push((w, h, clean(px)));
    }
    out
}

/// A PAM file's RGBA pixels (dwebp and anim_dump write `TUPLTYPE RGB_ALPHA`).
fn read_pam(path: &Path) -> (u32, u32, Vec<u8>) {
    let b = std::fs::read(path).unwrap();
    let end = b.windows(7).position(|w| w == b"ENDHDR\n").expect("PAM header") + 7;
    let head = String::from_utf8_lossy(&b[..end]);
    let field = |k: &str| -> u32 { head.lines().find_map(|l| l.strip_prefix(k)).unwrap().trim().parse().unwrap() };
    let (w, h, depth) = (field("WIDTH "), field("HEIGHT "), field("DEPTH "));
    assert_eq!(depth, 4, "{}: RGBA PAM", path.display());
    (w, h, b[end..].to_vec())
}

fn rgba_at(s: &dyn MediaSource, t: Tick) -> Vec<u8> {
    match &s.video_frame(FrameRequest::full(t)).unwrap().data {
        PixelData::Rgba8(p) => p.to_vec(),
        _ => panic!("WebP frames are RGBA8"),
    }
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn stills_decode_like_dwebp() {
    let (Some(cwebp), Some(dwebp)) = (tool("cwebp"), tool("dwebp")) else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("media/webp");
    let cases: [(&str, bool, &[&str]); 5] = [
        ("lossy.webp", false, &["-q", "75"]),
        ("lossy_alpha.webp", true, &["-q", "60", "-alpha_q", "100"]),
        ("lossy_alpha_filtered.webp", true, &["-q", "90", "-alpha_filter", "best", "-alpha_q", "50"]),
        ("lossless.webp", false, &["-lossless", "-z", "9"]),
        ("lossless_alpha.webp", true, &["-lossless", "-exact"]),
    ];
    for (i, (name, alpha, args)) in cases.into_iter().enumerate() {
        let png = dir.join(format!("{name}.png"));
        let webp = dir.join(name);
        let pam = dir.join(format!("{name}.pam"));
        let src_px = picture(i as u32, alpha);
        write_png(&png, &src_px);
        run(Command::new(&cwebp).arg("-quiet").args(args).arg(&png).arg("-o").arg(&webp));
        run(Command::new(&dwebp).arg("-quiet").arg(&webp).arg("-pam").arg("-o").arg(&pam));
        let (w, h, want) = read_pam(&pam);

        let s = cfa4_media::open_bytes(name, std::fs::read(&webp).unwrap().into(), &[]).unwrap();
        let info = s.info();
        assert_eq!(info.kind, MediaKind::Still, "{name}");
        let v = info.video.as_ref().unwrap();
        assert_eq!((v.width, v.height), (w, h), "{name}");
        assert_eq!(v.has_alpha, alpha, "{name}");
        let got = rgba_at(&*s, Tick::ZERO);
        assert_eq!(got.len(), want.len(), "{name}");
        let worst = got.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(worst, 0, "{name}: largest difference from dwebp");
        if args.contains(&"-lossless") {
            assert_eq!(got, src_px, "{name}: lossless is the source picture");
        }
    }
}

#[test]
fn animations_play_like_anim_dump() {
    let (Some(img2webp), Some(anim_dump)) = (tool("img2webp"), tool("anim_dump")) else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("media/webp_anim");
    // Frames that change only in part, so img2webp writes sub-rectangles that blend or dispose,
    // and a repeated frame that it merges into a longer one.
    let base = picture(7, true);
    let mut frames = vec![base.clone()];
    for k in 1..6u32 {
        let mut f = frames.last().unwrap().clone();
        for y in (k * 4)..(k * 4 + 9).min(H) {
            for x in (k * 7)..(k * 7 + 13).min(W) {
                let i = ((y * W + x) * 4) as usize;
                f[i..i + 4].copy_from_slice(&[(k * 40) as u8, 255 - (k * 40) as u8, 90, if k % 2 == 0 { 0 } else { 255 }]);
            }
        }
        frames.push(f);
    }
    frames.push(frames.last().unwrap().clone());
    let pngs: Vec<PathBuf> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let p = dir.join(format!("f{i}.png"));
            write_png(&p, f);
            p
        })
        .collect();
    let delays = [40u32, 80, 40, 120, 40, 40, 40];

    for (name, mode) in [("lossless.webp", "-lossless"), ("lossy.webp", "-lossy"), ("mixed.webp", "-mixed")] {
        let webp = dir.join(name);
        let mut cmd = Command::new(&img2webp);
        if mode == "-mixed" {
            cmd.arg("-mixed");
        }
        for (p, d) in pngs.iter().zip(delays) {
            cmd.args(["-d", &d.to_string()]);
            if mode != "-mixed" {
                cmd.arg(mode);
            }
            cmd.arg(p);
        }
        run(cmd.arg("-o").arg(&webp));
        let prefix = format!("{}_", name.trim_end_matches(".webp"));
        let mut dumped: Vec<Vec<u8>> = Vec::new();
        for (w, h, px) in anim_dump_pictures(&anim_dump, &dir, &webp, &prefix) {
            assert_eq!((w, h), (W, H));
            if dumped.last() != Some(&px) {
                dumped.push(px);
            }
        }

        let s = cfa4_media::open_bytes(name, std::fs::read(&webp).unwrap().into(), &[]).unwrap();
        let info = s.info();
        assert_eq!(info.kind, MediaKind::Movie, "{name}: an animated WebP is a clip");
        let v = info.video.as_ref().unwrap();
        assert_eq!((v.width, v.height, v.codec.as_str()), (W, H, "WebP"), "{name}");
        assert!(v.has_alpha, "{name}");
        let total: u32 = delays.iter().sum();
        assert_eq!(info.duration, Tick(i64::from(total) * TICKS_PER_SECOND / 1000), "{name}");

        let mut ours: Vec<Vec<u8>> = Vec::new();
        let mut ms = 0u32;
        for d in delays {
            let px = clean(rgba_at(&*s, Tick(i64::from(ms) * TICKS_PER_SECOND / 1000)));
            if ours.last() != Some(&px) {
                ours.push(px);
            }
            ms += d;
        }
        let n = dumped.len();
        assert_eq!(ours.len(), n, "{name}: distinct pictures");
        for (k, (a, b)) in ours.iter().zip(&dumped).enumerate() {
            let worst = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            assert_eq!(worst, 0, "{name}: picture {k} differs from anim_dump");
        }
        assert!(n >= 5, "{name}: anim_dump wrote {n} frames");
    }
}

#[test]
fn offsets_disposal_and_blending_compose_like_anim_dump() {
    let (Some(cwebp), Some(webpmux), Some(anim_dump)) = (tool("cwebp"), tool("webpmux"), tool("anim_dump")) else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("media/webp_mux");
    // (size, alpha, cwebp options, webpmux frame options: +duration+x+y+dispose and blend)
    let frames: [((u32, u32), bool, &[&str], &str); 7] = [
        ((W, H), true, &["-lossless", "-exact"], "+50+0+0+0+b"),
        ((20, 14), true, &["-q", "80", "-alpha_q", "100"], "+60+6+4+1+b"),
        ((24, 10), true, &["-lossless", "-exact"], "+70+10+8+0+b"),
        ((15, 15), false, &["-q", "70"], "+40+30+12+1-b"),
        ((W, H), true, &["-q", "85", "-alpha_q", "60", "-alpha_filter", "best"], "+80+0+0+1+b"),
        ((30, 20), true, &["-lossless", "-exact"], "+90+16+6+0+b"),
        ((12, 8), true, &["-lossless", "-exact"], "+20+40+2+0-b"),
    ];
    let mut cmd = Command::new(&webpmux);
    let mut durations = Vec::new();
    for (i, ((w, h), alpha, args, opts)) in frames.into_iter().enumerate() {
        let png = dir.join(format!("m{i}.png"));
        let webp = dir.join(format!("m{i}.webp"));
        write_png_sized(&png, w, h, &picture_sized(w, h, 11 + i as u32, alpha));
        run(Command::new(&cwebp).arg("-quiet").args(args).arg(&png).arg("-o").arg(&webp));
        cmd.arg("-frame").arg(&webp).arg(opts);
        durations.push(opts.split('+').nth(1).unwrap().parse::<i64>().unwrap());
    }
    let anim = dir.join("composed.webp");
    run(cmd.args(["-loop", "0", "-bgcolor", "255,0,255,255"]).arg("-o").arg(&anim));
    let want = anim_dump_pictures(&anim_dump, &dir, &anim, "composed_");
    assert_eq!(want.len(), frames.len());

    let s = cfa4_media::open_bytes("composed.webp", std::fs::read(&anim).unwrap().into(), &[]).unwrap();
    let mut t = 0i64;
    for (k, ((_, _, want), d)) in want.iter().zip(&durations).enumerate() {
        let got = clean(rgba_at(&*s, Tick(t * TICKS_PER_SECOND / 1000)));
        let worst = got.iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert_eq!(worst, 0, "frame {k} differs from anim_dump");
        t += d;
    }
    assert_eq!(s.info().duration, Tick(t * TICKS_PER_SECOND / 1000));
}
