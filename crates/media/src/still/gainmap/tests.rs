use super::*;

/// A baseline JPEG of an RGB (`channels` = 3) or grey (1) picture.
fn jpeg(w: u32, h: u32, channels: usize, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let color = if channels == 1 { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100).encode(px, w, h, color).unwrap();
    out
}

/// `pic` with the marker segments `segs` (marker, payload) inserted after SOI.
fn with_segments(pic: &[u8], segs: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut v = pic[..2].to_vec();
    for (m, body) in segs {
        v.extend([0xFF, *m]);
        v.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        v.extend(body);
    }
    v.extend(&pic[2..]);
    v
}

/// An ISO 21496-1 block for `p` (separate denominators, three channels).
fn iso_block(p: &GainMapParams) -> Vec<u8> {
    const D: u32 = 1 << 20;
    let q = |v: f32| ((v as f64) * f64::from(D)).round() as i64 as u32;
    let mut v = jpeg::ISO_ID.to_vec();
    v.extend([0, 0, 0, 0, 0x80 | if p.use_base_color_space { 0x40 } else { 0 }]);
    for h in [p.base_headroom, p.alternate_headroom] {
        v.extend(q(h).to_be_bytes());
        v.extend(D.to_be_bytes());
    }
    for c in &p.channels {
        for f in [c.min, c.max, c.gamma, c.base_offset, c.alternate_offset] {
            v.extend(q(f).to_be_bytes());
            v.extend(D.to_be_bytes());
        }
    }
    v
}

fn xmp_segment(xml: &str) -> Vec<u8> {
    let mut v = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    v.extend(xml.as_bytes());
    v
}

/// An MPF file: `primary` (with `primary_segs`) and `others`, each with its MP type code.
fn mpf_file(primary: &[u8], primary_segs: &[(u8, Vec<u8>)], primary_type: u32, others: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let n = 1 + others.len();
    // MPF payload: "MPF\0", big-endian TIFF header, IFD with 3 entries, then the MP entries
    let ifd_len = 2 + 3 * 12 + 4;
    let entries_at = 8 + ifd_len;
    let mpf_len = 4 + entries_at + 16 * n;
    let mut segs = primary_segs.to_vec();
    segs.insert(0, (0xE2, vec![0; mpf_len]));
    let first = with_segments(primary, &segs);
    let mpf_start = 2 + 4 + 4; // SOI, marker + length, "MPF\0"
    let mut mpf = b"MPF\0MM\0*".to_vec();
    mpf.extend(8u32.to_be_bytes());
    mpf.extend(3u16.to_be_bytes());
    mpf.extend([0xB0, 0x00, 0, 7, 0, 0, 0, 4, b'0', b'1', b'0', b'0']);
    mpf.extend([0xB0, 0x01, 0, 4, 0, 0, 0, 1]);
    mpf.extend(u32::try_from(n).unwrap().to_be_bytes());
    mpf.extend([0xB0, 0x02, 0, 7]);
    mpf.extend(u32::try_from(16 * n).unwrap().to_be_bytes());
    mpf.extend(u32::try_from(entries_at).unwrap().to_be_bytes());
    mpf.extend(0u32.to_be_bytes());
    let mut offset = first.len();
    mpf.extend((0x2000_0000 | primary_type).to_be_bytes());
    mpf.extend(u32::try_from(first.len()).unwrap().to_be_bytes());
    mpf.extend([0; 8]);
    for (pic, kind) in others {
        mpf.extend(kind.to_be_bytes());
        mpf.extend(u32::try_from(pic.len()).unwrap().to_be_bytes());
        mpf.extend(u32::try_from(offset - mpf_start).unwrap().to_be_bytes());
        mpf.extend([0; 4]);
        offset += pic.len();
    }
    assert_eq!(mpf.len(), mpf_len);
    let mut file = first;
    file[6..6 + mpf_len].copy_from_slice(&mpf);
    for (pic, _) in others {
        file.extend(pic);
    }
    file
}

fn params(max: [f32; 3], gamma: f32) -> GainMapParams {
    let mut p = GainMapParams::identity();
    p.alternate_headroom = max.iter().copied().fold(0.0, f32::max);
    for (c, m) in p.channels.iter_mut().zip(max) {
        c.max = m;
        c.gamma = gamma;
    }
    p
}

const W: u32 = 16;
const H: u32 = 8;

fn base() -> Vec<u8> {
    (0..W * H).flat_map(|i| [(i * 7 % 256) as u8, (i * 3 % 256) as u8, 200 - (i % 50) as u8]).collect()
}

/// A quarter-size one-channel gain map with a gradient.
fn quarter_map() -> (u32, u32, Vec<u8>) {
    let (w, h) = (W / 4, H / 4);
    (w, h, (0..w * h).map(|i| (i * 255 / (w * h - 1)) as u8).collect())
}

fn decoded(pic: &[u8], gray: bool) -> (usize, usize, Vec<u8>) {
    let img = image::load_from_memory(pic).unwrap();
    let (w, h) = (img.width() as usize, img.height() as usize);
    (w, h, if gray { img.into_luma8().into_raw() } else { img.into_rgba8().into_raw() })
}

/// The HDR picture computed independently, in f64, from the decoded base and map.
fn reference(file_base: &[u8], map_pic: &[u8], p: &GainMapParams) -> Vec<f64> {
    let (w, h, rgba) = decoded(file_base, false);
    let (mw, mh, m) = decoded(map_pic, true);
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let fx = ((x as f64 + 0.5) * mw as f64 / w as f64 - 0.5).clamp(0.0, (mw - 1) as f64);
            let fy = ((y as f64 + 0.5) * mh as f64 / h as f64 - 0.5).clamp(0.0, (mh - 1) as f64);
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(mw - 1), (y0 + 1).min(mh - 1));
            let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
            let at = |x: usize, y: usize| f64::from(m[y * mw + x]);
            let g = ((at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty) / 255.0;
            for (c, ch) in p.channels.iter().enumerate() {
                let g = g.powf(1.0 / f64::from(ch.gamma));
                let boost = f64::from(ch.min) * (1.0 - g) + f64::from(ch.max) * g;
                out.push((lin(rgba[(y * w + x) * 4 + c]) + f64::from(ch.base_offset)) * boost.exp2() - f64::from(ch.alternate_offset));
            }
        }
    }
    out
}

fn hdr_frame(name: &str, file: &[u8]) -> (Vec<f32>, GainMapParams) {
    match probe(name, file).unwrap() {
        Probe::GainMap(d) => {
            let cfa4_frame::PixelData::RgbaF32(px) = &d.frame.data else { panic!("float frame expected") };
            assert_eq!(d.frame.color.transfer, Transfer::Linear);
            (px.to_vec(), d.params)
        }
        _ => panic!("{name}: gain map expected"),
    }
}

fn assert_close(px: &[f32], want: &[f64]) {
    assert_eq!(px.len() / 4 * 3, want.len());
    let mut worst = 0f64;
    for (i, w) in want.iter().enumerate() {
        let got = f64::from(px[i / 3 * 4 + i % 3]);
        worst = worst.max((got - w).abs() / w.abs().max(1e-3));
    }
    assert!(worst < 1e-4, "worst relative difference {worst}");
}

#[test]
fn iso_gain_map_quarter_size_one_channel() {
    let (mw, mh, m) = quarter_map();
    let p = params([2.5; 3], 1.0);
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE2, iso_block(&p))]);
    let base_pic = jpeg(W, H, 3, &base());
    let file = mpf_file(&base_pic, &[(0xE2, b"urn:iso:std:iso:ts:21496:-1\0\0\0\0\0".to_vec())], 0x03_0000, &[(map_pic.clone(), 0)]);
    let (px, got) = hdr_frame("a.jpg", &file);
    assert_eq!(got.origin, Origin::Iso);
    assert!((got.channels[0].max - 2.5).abs() < 1e-6);
    assert_close(&px, &reference(&file, &map_pic, &got));
    assert!(px.chunks(4).any(|q| q[0] > 1.5), "highlights above SDR white");
}

#[test]
fn xmp_gain_map_with_gamma() {
    let (mw, mh, m) = quarter_map();
    let mut p = params([1.5; 3], 2.0);
    p.origin = Origin::Xmp;
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE1, xmp_segment(&xmp::write(&p)))]);
    let primary_xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
        <rdf:Description xmlns:hdrgm=\"http://ns.adobe.com/hdr-gain-map/1.0/\" hdrgm:Version=\"1.0\"/></rdf:RDF></x:xmpmeta>";
    let file = mpf_file(&jpeg(W, H, 3, &base()), &[(0xE1, xmp_segment(primary_xmp))], 0x03_0000, &[(map_pic.clone(), 0)]);
    let (px, got) = hdr_frame("u.jpg", &file);
    assert_eq!(got, p);
    assert_close(&px, &reference(&file, &map_pic, &got));
}

#[test]
fn iso_wins_over_xmp() {
    let (mw, mh, m) = quarter_map();
    let iso = params([3.0; 3], 1.0);
    let xmp_p = params([1.0; 3], 1.0);
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE1, xmp_segment(&xmp::write(&xmp_p))), (0xE2, iso_block(&iso))]);
    let file = mpf_file(&jpeg(W, H, 3, &base()), &[], 0x03_0000, &[(map_pic, 0)]);
    let (_, got) = hdr_frame("b.jpg", &file);
    assert_eq!(got.origin, Origin::Iso);
    assert!((got.channels[1].max - 3.0).abs() < 1e-6);
}

#[test]
fn ultra_hdr_without_mpf_uses_the_directory() {
    let (mw, mh, m) = quarter_map();
    let mut p = params([2.0; 3], 1.0);
    p.origin = Origin::Xmp;
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE1, xmp_segment(&xmp::write(&p)))]);
    let primary_xmp = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         <rdf:Description xmlns:hdrgm=\"http://ns.adobe.com/hdr-gain-map/1.0/\" xmlns:Container=\"http://ns.google.com/photos/1.0/container/\" \
         xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\" hdrgm:Version=\"1.0\"><Container:Directory><rdf:Seq>\
         <rdf:li><Container:Item Item:Semantic=\"Primary\" Item:Mime=\"image/jpeg\"/></rdf:li>\
         <rdf:li><Container:Item Item:Semantic=\"GainMap\" Item:Mime=\"image/jpeg\" Item:Length=\"{}\"/></rdf:li>\
         </rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>",
        map_pic.len()
    );
    let mut file = with_segments(&jpeg(W, H, 3, &base()), &[(0xE1, xmp_segment(&primary_xmp))]);
    file.extend(&map_pic);
    let (px, got) = hdr_frame("v1.jpg", &file);
    assert_eq!(got, p);
    assert_close(&px, &reference(&file, &map_pic, &got));
}

#[test]
fn stereo_mpo_opens_its_left_picture() {
    let left = jpeg(W, H, 3, &base());
    let right = jpeg(W, H, 3, &vec![9; (W * H * 3) as usize]);
    let file = mpf_file(&left, &[], jpeg::MP_TYPE_DISPARITY, &[(right, jpeg::MP_TYPE_DISPARITY)]);
    assert!(matches!(probe("s.mpo", &file).unwrap(), Probe::MultiPicture { stereo: true }));
    let s = StillSource::decode("s.mpo", &file).unwrap();
    let v = s.info.video.as_ref().unwrap();
    assert_eq!(v.codec, "MPO");
    assert!(v.pixel_format.contains("stereo"));
    let (_, _, want) = decoded(&left, false);
    let cfa4_frame::PixelData::Rgba8(px) = &s.frame.data else { panic!("8-bit frame expected") };
    assert_eq!(**px, want);
}

#[test]
fn plain_jpeg_is_left_alone() {
    let pic = jpeg(W, H, 3, &base());
    assert!(matches!(probe("p.jpg", &pic).unwrap(), Probe::Plain));
    assert!(still("p.jpg", &pic).unwrap().is_none());
}

#[test]
fn unreadable_gain_maps_open_the_sdr_base() {
    let (mw, mh, m) = quarter_map();
    let mut bad = iso_block(&params([2.0; 3], 1.0));
    let n = bad.len();
    bad[n - 4..].copy_from_slice(&[0; 4]); // a zero denominator
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE2, bad)]);
    let file = mpf_file(&jpeg(W, H, 3, &base()), &[(0xE2, b"urn:iso:std:iso:ts:21496:-1\0\0\0\0\0".to_vec())], 0x03_0000, &[(map_pic, 0)]);
    let s = StillSource::decode("bad.jpg", &file).unwrap();
    assert!(matches!(s.frame.data, cfa4_frame::PixelData::Rgba8(_)));
}

#[test]
fn damaged_files_do_not_panic() {
    let (mw, mh, m) = quarter_map();
    let p = params([2.0; 3], 1.0);
    let map_pic = with_segments(&jpeg(mw, mh, 1, &m), &[(0xE2, iso_block(&p))]);
    let file = mpf_file(&jpeg(W, H, 3, &base()), &[], 0x03_0000, &[(map_pic, 0)]);
    for n in (0..file.len()).step_by(7) {
        let _ = probe("t.jpg", &file[..n]);
    }
    let mut s = 0x2468_ace1u32;
    for _ in 0..300 {
        let mut f = file.clone();
        for _ in 0..4 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = (s as usize >> 8) % 400.min(f.len());
            f[i] = (s >> 24) as u8;
        }
        let _ = probe("f.jpg", &f);
    }
}
