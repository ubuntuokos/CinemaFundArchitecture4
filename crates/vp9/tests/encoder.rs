//! The encoder's streams decoded by ffmpeg: the pictures must equal ours sample for sample (so
//! the stream means the same to every decoder) and stay close to the source.

mod common;

use cfa4_vp9::encode::{Config, Encoder, Input, Rate};

/// `frames` pictures of ffmpeg's moving test pattern, as 8-bit 4:2:0.
fn source(w: usize, h: usize, frames: usize) -> Option<Vec<Vec<u8>>> {
    let ff = common::ffmpeg()?;
    let path = common::fixtures_dir().join(format!("enc_src_{w}x{h}_{frames}.yuv"));
    if !path.exists() {
        let tmp = path.with_extension(format!("{}.yuv", common::unique()));
        let mut c = std::process::Command::new(ff);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"]);
        c.arg(format!("testsrc2=size={w}x{h}:rate=25"));
        c.args(["-frames:v", &frames.to_string(), "-pix_fmt", "yuv420p", "-f", "rawvideo"]).arg(&tmp);
        if !common::run(&mut c) {
            return None;
        }
        let _ = std::fs::rename(&tmp, &path);
    }
    let d = std::fs::read(&path).ok()?;
    let size = w * h + 2 * w.div_ceil(2) * h.div_ceil(2);
    Some(d.chunks_exact(size).map(<[u8]>::to_vec).collect())
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    let mse = se / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

fn round_trip(name: &str, cfg: Config, frames: usize, min_psnr: f64) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let Some(src) = source(w, h, frames) else { return };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for f in &src {
        let (y, rest) = f.split_at(w * h);
        let (u, v) = rest.split_at(cw * ch);
        let input = Input { y, u, v, y_stride: w, uv_stride: cw };
        packets.push(enc.encode(&input, false).unwrap().data);
    }
    let ivf = common::fixtures_dir().join(format!("{name}.{}.ivf", common::unique()));
    common::write_ivf(&ivf, w as u16, h as u16, &packets);
    let yuv = ivf.with_extension("yuv");
    assert!(common::reference_decode(&ivf, &yuv, "yuv420p"), "ffmpeg failed on {name}");
    let reference = std::fs::read(&yuv).unwrap();
    let _ = std::fs::remove_file(&yuv);
    let checked = common::check_files(name, &ivf, &reference, "yuv420p");
    let _ = std::fs::remove_file(&ivf);
    checked.unwrap();
    let size = w * h + 2 * cw * ch;
    let bytes: usize = packets.iter().map(Vec::len).sum();
    let mut total = 0.0;
    for (i, (s, r)) in src.iter().zip(reference.chunks_exact(size)).enumerate() {
        let q = psnr(&s[..w * h], &r[..w * h]);
        assert!(q > min_psnr, "{name} frame {i}: {q:.2} dB ({bytes} bytes in all)");
        total += q;
    }
    eprintln!("{name}: {bytes} bytes, mean luma PSNR {:.2} dB", total / src.len().max(1) as f64);
}

#[test]
fn ffmpeg_decodes_key_and_inter_frames_like_we_do() {
    let mut cfg = Config::new(176, 144);
    cfg.keyframe_interval = 8;
    round_trip("enc_cif_q60", cfg, 12, 34.0);
}

#[test]
fn ffmpeg_decodes_odd_sizes_like_we_do() {
    let mut cfg = Config::new(99, 71);
    cfg.rate = Rate::Quantizer(120);
    cfg.speed = 0;
    round_trip("enc_odd_q120", cfg, 5, 28.0);
}

#[test]
fn ffmpeg_decodes_fine_quantizers_like_we_do() {
    let mut cfg = Config::new(128, 72);
    cfg.rate = Rate::Quantizer(4);
    cfg.speed = 2;
    round_trip("enc_q4", cfg, 4, 45.0);
}

#[test]
fn ffmpeg_decodes_rate_controlled_streams_like_we_do() {
    let mut cfg = Config::new(320, 180);
    cfg.rate = Rate::Bitrate { bits_per_second: 400_000, fps_num: 25, fps_den: 1 };
    round_trip("enc_abr", cfg, 15, 26.0);
}
