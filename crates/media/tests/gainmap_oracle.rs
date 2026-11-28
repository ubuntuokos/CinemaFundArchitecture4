//! HDR gain map JPEGs against FFmpeg (an external test oracle and fixture generator; F2g decision D1:
//! no downloaded samples). FFmpeg's `gainmap` filter makes a gain map from an SDR base and a brighter
//! linear alternate picture, and its `jpeg_mpf` muxer writes both as an MPF file with ISO 21496-1
//! metadata. We check that
//! - our MPF reader finds the same pictures, byte for byte, as FFmpeg's `jpeg_mpf` demuxer;
//! - our ISO 21496-1 reader gives the parameters `ffprobe` reports;
//! - our HDR picture matches an independent f64 computation from FFmpeg's decode of both pictures
//!   (relative error and a ΔE00 table on the picture scaled to its peak);
//! - the same file rewritten with XMP `hdrgm` metadata only (FFmpeg writes none) decodes the same.

use std::path::{Path, PathBuf};
use std::process::Command;

use cfa4_frame::PixelData;
use cfa4_media::FrameRequest;
use cfa4_media::still::gainmap::jpeg;

const W: usize = 320;
const H: usize = 240;

/// The filter graph: base = the test pattern tagged sRGB, alternate = three times brighter linear light.
const GRAPH: &str = "[0:v]setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709,format=gbrpf32le,split=3[b][b2][a0];\
    [a0]geq=r=r(X\\,Y)*3:g=g(X\\,Y)*3:b=b(X\\,Y)*3,setparams=color_trc=linear[a];\
    [b][a]gainmap=alt_nits=1000,format=yuvj444p[g];[b2]format=yuvj444p[o]";

fn ffmpeg_can_make_gain_maps(ff: &Path) -> bool {
    let has = |what: &str, name: &str| {
        Command::new(ff).args(["-hide_banner", "-h", &format!("{what}={name}")]).output().is_ok_and(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            o.status.success() && !s.contains("Unknown") && s.contains(name)
        })
    };
    has("filter", "gainmap") && has("muxer", "jpeg_mpf")
}

/// The FFmpeg-made gain map file (generated once into the fixture directory).
fn fixture(ff: &Path) -> Option<PathBuf> {
    let out = cfa4_testkit::fixtures_dir("media/gainmap").join("ffmpeg-iso.jpg");
    cfa4_testkit::fixtures::generate(&out, |tmp| {
        Command::new(ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=s={W}x{H}:d=1"), "-filter_complex", GRAPH])
            .args(["-map", "[o]", "-map", "[g]", "-stream_group", "type=gain_map:st=1:st=0", "-frames:v", "1", "-c:v", "mjpeg", "-q:v", "2", "-f", "jpeg_mpf"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// FFmpeg's packets of stream `k` (the pictures as stored).
fn stream_bytes(ff: &Path, file: &Path, k: usize) -> Vec<u8> {
    let o = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-map", &format!("0:{k}"), "-c", "copy", "-f", "mjpeg", "-"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

/// FFmpeg's decode of stream `k` as interleaved RGB bytes.
fn stream_rgb(ff: &Path, file: &Path, k: usize) -> Vec<u8> {
    let o = Command::new(ff)
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", &format!("0:{k}"), "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o.stdout.len(), W * H * 3, "stream {k} size");
    o.stdout
}

/// One channel's parameters as ffprobe prints them: min, max, gamma, base offset, alternate offset.
#[derive(Debug, Clone, Copy)]
struct Probed {
    base_headroom: f64,
    alternate_headroom: f64,
    channels: [[f64; 5]; 3],
}

/// The gain map parameters from `ffprobe -show_frames` (rationals such as `"12553001/2097152"`).
fn probe_params(fp: &Path, file: &Path) -> Probed {
    let o = Command::new(fp).args(["-v", "error", "-show_frames", "-of", "json"]).arg(file).output().unwrap();
    assert!(o.status.success());
    let text = String::from_utf8_lossy(&o.stdout);
    let values = |key: &str| -> Vec<f64> {
        text.match_indices(&format!("\"{key}\": "))
            .map(|(i, m)| {
                let v = text[i + m.len()..].trim_start_matches('"');
                let v = &v[..v.find(['"', ',', '\n']).unwrap()];
                match v.split_once('/') {
                    Some((n, d)) => n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap(),
                    None => v.parse().unwrap(),
                }
            })
            .collect()
    };
    let (bh, ah) = (values("base_hdr_headroom"), values("alternate_hdr_headroom"));
    let keys = ["gain_map_min", "gain_map_max", "gamma", "base_offset", "alternate_offset"];
    let per: Vec<Vec<f64>> = keys.iter().map(|k| values(k)).collect();
    assert_eq!((bh.len(), ah.len()), (1, 1), "one gain map in {text}");
    let mut channels = [[0.0; 5]; 3];
    for (c, ch) in channels.iter_mut().enumerate() {
        for (f, v) in ch.iter_mut().enumerate() {
            *v = per[f][c.min(per[f].len() - 1)];
        }
    }
    Probed { base_headroom: bh[0], alternate_headroom: ah[0], channels }
}

fn srgb_to_linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb8(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// The HDR picture from FFmpeg's decode of the base and the (same-size) gain map, in f64.
fn reference(base: &[u8], map: &[u8], p: &Probed) -> Vec<f64> {
    let mut out = Vec::with_capacity(W * H * 3);
    for i in 0..W * H {
        for (c, ch) in p.channels.iter().enumerate() {
            let [min, max, gamma, bo, ao] = *ch;
            let g = (f64::from(map[i * 3 + c]) / 255.0).powf(1.0 / gamma);
            out.push((srgb_to_linear(base[i * 3 + c]) + bo) * (min * (1.0 - g) + max * g).exp2() - ao);
        }
    }
    out
}

fn open_hdr(name: &str, bytes: Vec<u8>) -> (Vec<f32>, String) {
    let src = cfa4_media::open_bytes(name, bytes.into(), &[]).unwrap();
    let f = src.video_frame(FrameRequest::full(cfa4_time::Tick::ZERO)).unwrap();
    assert_eq!((f.width as usize, f.height as usize), (W, H));
    let PixelData::RgbaF32(px) = &f.data else { panic!("{name}: a gain map picture opens as floats") };
    (px.to_vec(), src.info().video.as_ref().unwrap().pixel_format.clone())
}

/// Compare `px` with `want`: relative error and ΔE00 of both scaled by `scale` (the peak) into sRGB.
fn compare(name: &str, px: &[f32], want: &[f64], scale: f64) -> (f64, cfa4_metrics::DeltaEStats) {
    let mut rel = 0f64;
    let (mut a, mut b) = (Vec::with_capacity(W * H * 3), Vec::with_capacity(W * H * 3));
    for (i, w) in want.iter().enumerate() {
        let got = f64::from(px[i / 3 * 4 + i % 3]);
        rel += (got - w).abs() / w.abs().max(0.01);
        a.push(linear_to_srgb8(w / scale));
        b.push(linear_to_srgb8(got / scale));
    }
    let mean_rel = rel / want.len() as f64;
    let de = cfa4_metrics::delta_e_srgb8(&a, &b, W, H, 3).unwrap();
    println!("{name}: mean relative error {mean_rel:.5}, ΔE00 mean {:.3} max {:.3}, above 1: {:.2}%", de.mean, de.max, de.above_jnd * 100.0);
    (mean_rel, de)
}

/// A marker segment.
fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    v.extend(body);
    v
}

/// The marker segments of `pic` without the ISO 21496-1 APP2 ones.
fn without_iso(pic: &[u8]) -> Vec<u8> {
    let mut out = pic[..2].to_vec();
    let mut i = 2;
    while pic[i] == 0xFF && pic[i + 1] != 0xDA {
        let len = usize::from(u16::from_be_bytes([pic[i + 2], pic[i + 3]]));
        let seg = &pic[i..i + 2 + len];
        if !(pic[i + 1] == 0xE2 && seg[4..].starts_with(jpeg::ISO_ID)) {
            out.extend(seg);
        }
        i += 2 + len;
    }
    out.extend(&pic[i..]);
    out
}

/// An Ultra HDR file (MPF + XMP `hdrgm`, CIPA DC-007 and the Adobe Gain Map Specification) from a base
/// and a gain map picture.
fn ultra_hdr(base: &[u8], map: &[u8], p: &Probed) -> Vec<u8> {
    let seq = |f: usize| p.channels.iter().map(|c| format!("<rdf:li>{}</rdf:li>", c[f])).collect::<String>();
    let xmp = format!(
        "http://ns.adobe.com/xap/1.0/\0<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         <rdf:Description xmlns:hdrgm=\"http://ns.adobe.com/hdr-gain-map/1.0/\" hdrgm:Version=\"1.0\" hdrgm:HDRCapacityMin=\"{}\" hdrgm:HDRCapacityMax=\"{}\">\
         <hdrgm:GainMapMin><rdf:Seq>{}</rdf:Seq></hdrgm:GainMapMin><hdrgm:GainMapMax><rdf:Seq>{}</rdf:Seq></hdrgm:GainMapMax>\
         <hdrgm:Gamma><rdf:Seq>{}</rdf:Seq></hdrgm:Gamma><hdrgm:OffsetSDR><rdf:Seq>{}</rdf:Seq></hdrgm:OffsetSDR>\
         <hdrgm:OffsetHDR><rdf:Seq>{}</rdf:Seq></hdrgm:OffsetHDR></rdf:Description></rdf:RDF></x:xmpmeta>",
        p.base_headroom,
        p.alternate_headroom,
        seq(0),
        seq(1),
        seq(2),
        seq(3),
        seq(4)
    );
    let map = without_iso(map);
    let map = [&map[..2], &segment(0xE1, xmp.as_bytes()), &map[2..]].concat();
    let primary_xmp = "http://ns.adobe.com/xap/1.0/\0<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
        <rdf:Description xmlns:hdrgm=\"http://ns.adobe.com/hdr-gain-map/1.0/\" hdrgm:Version=\"1.0\"/></rdf:RDF></x:xmpmeta>";
    let base = without_iso(base);
    // strip the old MPF segment of the base, then add ours: header, IFD (3 entries), 2 MP entries
    let mut plain = base[..2].to_vec();
    let mut i = 2;
    while base[i] == 0xFF && base[i + 1] != 0xDA {
        let len = usize::from(u16::from_be_bytes([base[i + 2], base[i + 3]]));
        if !(base[i + 1] == 0xE2 && base[i + 4..].starts_with(b"MPF\0")) {
            plain.extend(&base[i..i + 2 + len]);
        }
        i += 2 + len;
    }
    plain.extend(&base[i..]);
    let mpf_len = 4 + 8 + 2 + 36 + 4 + 32;
    let app1 = segment(0xE1, primary_xmp.as_bytes());
    let first_len = plain.len() + app1.len() + 4 + mpf_len;
    let tiff_at = 2 + app1.len() + 4 + 4;
    let mut mpf = b"MPF\0MM\0*\0\0\0\x08\0\x03".to_vec();
    mpf.extend([0xB0, 0x00, 0, 7, 0, 0, 0, 4, b'0', b'1', b'0', b'0']);
    mpf.extend([0xB0, 0x01, 0, 4, 0, 0, 0, 1, 0, 0, 0, 2]);
    mpf.extend([0xB0, 0x02, 0, 7, 0, 0, 0, 32, 0, 0, 0, 50, 0, 0, 0, 0]);
    mpf.extend(0x2003_0000u32.to_be_bytes());
    mpf.extend(u32::try_from(first_len).unwrap().to_be_bytes());
    mpf.extend([0; 8]);
    mpf.extend(0u32.to_be_bytes());
    mpf.extend(u32::try_from(map.len()).unwrap().to_be_bytes());
    mpf.extend(u32::try_from(first_len - tiff_at).unwrap().to_be_bytes());
    mpf.extend([0; 4]);
    assert_eq!(mpf.len(), mpf_len);
    [&plain[..2], &app1, &segment(0xE2, &mpf), &plain[2..], &map].concat()
}

#[test]
fn ffmpeg_gain_maps() {
    let ff = cfa4_testkit::require_ffmpeg!();
    let fp = cfa4_testkit::require_ffprobe!();
    if !ffmpeg_can_make_gain_maps(&ff) {
        cfa4_testkit::skip("gainmap_oracle", "ffmpeg has no gainmap filter or jpeg_mpf muxer (the release pinned in packaging/ffmpeg-version.env has both)");
        return;
    }
    let Some(file) = fixture(&ff) else {
        cfa4_testkit::skip("gainmap_oracle", "ffmpeg could not write the gain map fixture");
        return;
    };
    let bytes = std::fs::read(&file).unwrap();

    // 1. the pictures, byte for byte, as FFmpeg's demuxer splits them
    let s = jpeg::segments(&bytes);
    let (base_at, mpf) = s.mpf.expect("an MPF segment");
    let entries = jpeg::mp_entries(base_at, mpf, bytes.len()).unwrap();
    assert_eq!(entries.len(), 2);
    let (pic0, pic1) = (stream_bytes(&ff, &file, 0), stream_bytes(&ff, &file, 1));
    assert_eq!(&bytes[entries[0].offset..entries[0].offset + entries[0].size], &pic0[..]);
    assert_eq!(&bytes[entries[1].offset..entries[1].offset + entries[1].size], &pic1[..]);

    // 2. the ISO 21496-1 parameters, as ffprobe reports them
    let p = probe_params(&fp, &file);
    let Ok(cfa4_media::still::gainmap::iso::Block::Params(ours)) = cfa4_media::still::gainmap::iso::parse(jpeg::segments(&pic1).iso.unwrap()) else {
        panic!("ISO parameters expected")
    };
    assert!((f64::from(ours.base_headroom) - p.base_headroom).abs() < 1e-6);
    assert!((f64::from(ours.alternate_headroom) - p.alternate_headroom).abs() < 1e-6);
    for (c, ch) in ours.channels.iter().enumerate() {
        let got = [ch.min, ch.max, ch.gamma, ch.base_offset, ch.alternate_offset].map(f64::from);
        for (a, b) in got.iter().zip(p.channels[c]) {
            assert!((a - b).abs() < 1e-6, "channel {c}: {got:?} vs {:?}", p.channels[c]);
        }
    }
    assert!(p.alternate_headroom > 1.0, "the alternate picture is brighter: {p:?}");

    // 3. the HDR picture against an independent computation from FFmpeg's decode
    let want = reference(&stream_rgb(&ff, &file, 0), &stream_rgb(&ff, &file, 1), &p);
    let (px, desc) = open_hdr("ffmpeg-iso.jpg", bytes.clone());
    assert!(desc.contains("ISO 21496-1"), "{desc}");
    let peak = p.alternate_headroom.exp2();
    let (rel, de) = compare("ISO 21496-1 vs FFmpeg's decode", &px, &want, peak);
    // JPEG decoders need not agree bit for bit (T.81 A.3.3 bounds only the IDCT's accuracy): FFmpeg's
    // and the image crate's differ by a few code values at sharp edges, which the gain map can amplify
    assert!(rel < 0.005 && de.mean < 0.1 && de.above_jnd < 0.02 && de.max < 8.0, "ISO path: relative {rel}, {de:?}");
    // with the same decoder on both sides, only the gain map arithmetic is compared
    let rgb = |pic: &[u8]| image::load_from_memory_with_format(pic, image::ImageFormat::Jpeg).unwrap().into_rgb8().into_raw();
    let (b8, m8) = (rgb(&pic0), rgb(&pic1));
    let ff0 = stream_rgb(&ff, &file, 0);
    let ff1 = stream_rgb(&ff, &file, 1);
    let code = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    println!("JPEG decoders (image crate vs FFmpeg): base differs by at most {} code values, gain map by {}", code(&b8, &ff0), code(&m8, &ff1));
    let (rel, de) = compare("ISO 21496-1 vs the same decode", &px, &reference(&b8, &m8, &p), peak);
    assert!(rel < 1e-4 && de.max < 0.5, "gain map arithmetic: relative {rel}, {de:?}");
    assert!(px.chunks(4).any(|q| q[0] > 1.5 || q[1] > 1.5 || q[2] > 1.5), "highlights above SDR white");

    // 4. the same pictures with XMP hdrgm metadata only
    let uhdr = ultra_hdr(&pic0, &pic1, &p);
    let (px_x, desc_x) = open_hdr("ultra-hdr.jpg", uhdr);
    assert!(desc_x.contains("Ultra HDR"), "{desc_x}");
    let worst = px.iter().zip(&px_x).map(|(a, b)| (a - b).abs() / a.abs().max(0.01)).fold(0f32, f32::max);
    assert!(worst < 1e-4, "XMP and ISO paths differ: worst relative {worst}");
}
