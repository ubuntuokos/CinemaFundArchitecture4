//! The encoder against ffmpeg's `cfhd` decoder: our samples, muxed into MOV, must decode in
//! ffmpeg exactly as in our decoder, and close to the source. Fixtures go to
//! `target/fixtures/cineform/`.

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use cfa4_cineform::{EncodedFormat, EncoderConfig, Flavor, Quality, decode_frame_as, encode_frame};
use cfa4_isobmff::{Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use cfa4_testkit::fixtures::{fixtures_dir, generate};

fn run(ff: &Path, args: &[&str]) -> bool {
    Command::new(ff).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().is_ok_and(|s| s.success())
}

fn le16(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect()
}

fn psnr(a: &[u16], b: &[u16], bits: u8) -> f64 {
    let max = f64::from((1u32 << bits) - 1);
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    let mse = se / a.len().max(1) as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (max * max / mse).log10() }
}

/// ffmpeg's planar order (Y U V / G B R A) of our planes (Y Cb Cr / R G B A).
fn ffmpeg_order(format: EncodedFormat) -> &'static [usize] {
    match format {
        EncodedFormat::Yuv422 => &[0, 1, 2],
        EncodedFormat::Rgb444 => &[1, 2, 0],
        _ => &[1, 2, 0, 3],
    }
}

fn check(pix: &str, format: EncodedFormat, w: u32, h: u32, quality: Quality, source: &str, min_psnr: f64) {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cineform encoder oracle") else { return };
    let dir = fixtures_dir("cineform");
    let src = format!("{source}=s={w}x{h}");
    let raw = generate(&dir.join(format!("src_{pix}_{w}x{h}_{source}.raw")), |tmp| {
        run(&ff, &["-f", "lavfi", "-i", &src, "-frames:v", "1", "-pix_fmt", pix, "-f", "rawvideo", tmp.to_str().unwrap_or("")])
    })
    .expect("ffmpeg source");
    let raw = le16(&std::fs::read(raw).expect("read source"));
    let cfg = EncoderConfig { width: w, height: h, format, quality };
    // ffmpeg's planes into ours
    let order = ffmpeg_order(format);
    let mut planes = vec![Vec::new(); order.len()];
    let mut off = 0;
    for &i in order {
        let (pw, ph) = cfg.plane_size(i);
        planes[i] = raw[off..off + pw * ph].to_vec();
        off += pw * ph;
    }
    assert_eq!(off, raw.len());
    let refs: Vec<&[u16]> = planes.iter().map(Vec::as_slice).collect();
    let t = std::time::Instant::now();
    let sample = encode_frame(&cfg, &refs).expect("encode");
    eprintln!("{pix} {w}x{h} {quality:?}: {} bytes in {:.1} ms", sample.len(), t.elapsed().as_secs_f64() * 1000.0);
    let ours = decode_frame_as(&sample, Flavor::Ffmpeg).expect("our decode");

    // mux and let ffmpeg decode it
    let mut mov = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("mov");
    let track = mov.add_track(TrackConfig::new(SampleEntry::cineform(w as u16, h as u16), 25)).expect("track");
    mov.write_sample(track, WriteSample { data: &sample, duration: 1, composition_offset: 0, is_sync: true }).expect("sample");
    let mov = mov.finish().expect("finish").into_inner();
    let name = format!("ours_{pix}_{w}x{h}_{quality:?}_{source}");
    let mov_path = dir.join(format!("{name}.mov"));
    std::fs::write(&mov_path, mov).expect("write mov");
    let out = dir.join(format!("{name}.raw"));
    assert!(
        run(&ff, &["-i", mov_path.to_str().unwrap_or(""), "-f", "rawvideo", "-pix_fmt", pix, out.to_str().unwrap_or("")]),
        "ffmpeg cannot decode our {name}"
    );
    let theirs = le16(&std::fs::read(out).expect("read ffmpeg output"));
    let mut off = 0;
    for &i in order {
        let p = &ours.planes[i];
        let n = p.width * p.height;
        let want = &theirs[off..off + n];
        let diff = p.data.iter().zip(want).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "{name} plane {i}: {diff} of {n} samples differ from ffmpeg's decode");
        let q = psnr(&p.data, &planes[i], cfg.bit_depth());
        eprintln!("  plane {i}: {q:.1} dB");
        assert!(q > min_psnr, "{name} plane {i}: {q:.1} dB");
        off += n;
    }
    assert_eq!(off, theirs.len());
}

#[test]
fn yuv422p10_decodes_in_ffmpeg_like_ours() {
    check("yuv422p10le", EncodedFormat::Yuv422, 320, 240, Quality::FilmScan1, "testsrc2", 40.0);
}

#[test]
fn yuv422p10_every_quality() {
    for q in Quality::ALL {
        check("yuv422p10le", EncodedFormat::Yuv422, 256, 144, q, "mandelbrot", 38.0);
    }
}

/// 720 × 486: the chroma lowpass band is 45 wide (odd) and the coded height is 488.
#[test]
fn ntsc_size_decodes_in_ffmpeg_like_ours() {
    check("yuv422p10le", EncodedFormat::Yuv422, 720, 486, Quality::High, "mandelbrot", 38.0);
}

#[test]
fn hd_yuv422p10() {
    check("yuv422p10le", EncodedFormat::Yuv422, 1920, 1080, Quality::FilmScan1, "testsrc2", 40.0);
}

#[test]
fn gbrp12_decodes_in_ffmpeg_like_ours() {
    check("gbrp12le", EncodedFormat::Rgb444, 256, 144, Quality::FilmScan1, "testsrc2", 45.0);
}

#[test]
fn gbrap12_decodes_in_ffmpeg_like_ours() {
    check("gbrap12le", EncodedFormat::Rgba4444, 160, 120, Quality::Medium, "testsrc2", 40.0);
}
