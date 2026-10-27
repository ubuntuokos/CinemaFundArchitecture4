//! CFA4's own HEIF / HEIC decoder against libheif (an external test oracle, #88). `heif-enc`
//! writes HEVC images; the same bitstreams are rewrapped with `cfa4_heif::writer` as grids and
//! with clean aperture, rotation, mirroring and identity items; libheif's `heif-dec` (or
//! `heif-convert` before libheif 1.18) decodes every file, and ours must give the same picture.

use std::path::PathBuf;
use std::process::Command;

use cfa4_heif::writer::{self, Item};
use cfa4_heif::{Coding, Colour};
use cfa4_media::still::heic;

fn tool(names: &[&str]) -> Option<PathBuf> {
    let found = names.iter().find_map(|n| cfa4_testkit::oracle::find_tool(n));
    if found.is_none() {
        cfa4_testkit::skip(
            "heic_native_oracle",
            &format!("none of {names:?} found (Ubuntu packages libheif-examples, libheif-plugin-x265, libheif-plugin-libde265)"),
        );
    }
    found
}

fn run(cmd: &mut Command) {
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A picture with gradients, edges and an asymmetric mark (so rotations and mirrors show).
fn picture(w: u32, h: u32, seed: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        let mark = x < w / 4 && y < h / 5;
        let r = (x * 255 / w) as u8 ^ (seed as u8).wrapping_mul(29);
        let g = if mark {
            250
        } else if (x / 8 + y / 8) % 2 == 0 {
            40
        } else {
            200
        };
        let b = (y * 255 / h) as u8;
        image::Rgb([r, g, b])
    })
}

struct Oracle {
    enc: PathBuf,
    dec: PathBuf,
    dir: PathBuf,
}

impl Oracle {
    fn new() -> Option<Self> {
        let enc = tool(&["heif-enc"])?;
        let dec = tool(&["heif-dec", "heif-convert"])?;
        Some(Self { enc, dec, dir: cfa4_testkit::fixtures::fixtures_dir("media/heic_native") })
    }

    /// Encode a picture with heif-enc; the file's bytes.
    fn encode(&self, name: &str, pic: &image::RgbImage, args: &[&str]) -> Vec<u8> {
        let png = self.dir.join(format!("{name}.png"));
        let out = self.dir.join(format!("{name}.heic"));
        pic.save(&png).unwrap();
        run(Command::new(&self.enc).args(args).arg("-o").arg(&out).arg(&png));
        std::fs::read(&out).unwrap()
    }

    /// libheif's decode of `bytes` as RGBA8.
    fn decode(&self, name: &str, bytes: &[u8]) -> image::RgbaImage {
        let input = self.dir.join(format!("{name}.in.heic"));
        let output = self.dir.join(format!("{name}.out.png"));
        std::fs::write(&input, bytes).unwrap();
        let _ = std::fs::remove_file(&output);
        run(Command::new(&self.dec).arg(&input).arg(&output));
        image::open(&output).unwrap().to_rgba8()
    }

    /// Our decode must equal libheif's within `tolerance` per channel.
    fn check(&self, name: &str, bytes: &[u8], tolerance: u8) {
        let want = self.decode(name, bytes);
        let got = heic::decode(name, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((got.width, got.height), want.dimensions(), "{name}: size");
        let worst = got.rgba.iter().zip(want.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let mean = got.rgba.iter().zip(want.as_raw()).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / got.rgba.len() as f64;
        eprintln!("{name}: worst {worst}, mean {mean:.3}");
        assert!(worst <= tolerance, "{name}: largest difference from libheif {worst} > {tolerance} (mean {mean:.3})");
        assert!(mean < 0.6, "{name}: mean difference {mean:.3}");
    }
}

/// The HEVC item of a single-image file: (hvcC, data, colour properties).
fn hevc_item(bytes: &[u8]) -> (Vec<u8>, Vec<u8>, Option<Colour>) {
    let p = cfa4_heif::parse(bytes).unwrap().primary;
    let colour = p.nclx().cloned();
    let Coding::Hevc { hvcc, data } = p.coding else { panic!("heif-enc wrote {:?}", p.coding) };
    (hvcc, data, colour)
}

fn nclx_box(c: &Option<Colour>) -> Vec<Vec<u8>> {
    match c {
        Some(Colour::Nclx { primaries, transfer, matrix, full_range }) => vec![writer::nclx_box(*primaries, *transfer, *matrix, *full_range)],
        _ => Vec::new(),
    }
}

fn single(hvcc: &[u8], data: &[u8], colour: &Option<Colour>, w: u32, h: u32, extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut properties = vec![writer::hvcc_box(hvcc), writer::ispe_box(w, h)];
    properties.extend(nclx_box(colour));
    properties.extend(extra);
    writer::build(1, &[Item { id: 1, kind: *b"hvc1", data: data.to_vec(), properties, ..Item::default() }])
}

#[test]
fn heif_enc_images_decode_like_libheif() {
    let Some(o) = Oracle::new() else { return };
    let pic = picture(96, 64, 1);
    for (name, args) in [("q90", &["-q", "90"][..]), ("q40", &["-q", "40"]), ("odd", &["-q", "80"]), ("ten_bit", &["-q", "90", "-b", "10"])] {
        let p = if name == "odd" { picture(75, 41, 3) } else { pic.clone() };
        let bytes = o.encode(name, &p, args);
        // 10-bit goes through the linear-light path and is rounded to 8 bits there
        o.check(name, &bytes, if name == "ten_bit" { 3 } else { 2 });
    }
}

#[test]
fn transforms_grids_and_identity_items_match_libheif() {
    let Some(o) = Oracle::new() else { return };
    let (w, h) = (96u32, 64u32);
    let bytes = o.encode("base", &picture(w, h, 7), &["-q", "92"]);
    let (hvcc, data, colour) = hevc_item(&bytes);

    for (name, extra) in [
        ("rot90", vec![writer::irot_box(1)]),
        ("rot180", vec![writer::irot_box(2)]),
        ("rot270", vec![writer::irot_box(3)]),
        ("mirror_top_bottom", vec![writer::imir_box(0)]),
        ("mirror_left_right", vec![writer::imir_box(1)]),
        // left 14, top 13.75 rounded to 14: an even origin, because libheif upsamples 4:2:0 to
        // 4:4:4 with its own filter before an odd crop (newer releases), which no longer
        // matches its usual YCbCr to RGB path
        ("clap", vec![writer::clap_box((61, 1), (41, 1), (-7, 2), (9, 4))]),
        ("clap_rot_mirror", vec![writer::clap_box((80, 1), (50, 1), (5, 1), (-3, 1)), writer::irot_box(1), writer::imir_box(0)]),
    ] {
        o.check(name, &single(&hvcc, &data, &colour, w, h, extra), 2);
    }

    // an identity item that rotates a hidden coded image
    let mut props = vec![writer::hvcc_box(&hvcc), writer::ispe_box(w, h)];
    props.extend(nclx_box(&colour));
    let iden = writer::build(
        2,
        &[
            Item { id: 1, kind: *b"hvc1", data: data.clone(), properties: props, hidden: true, ..Item::default() },
            Item {
                id: 2,
                kind: *b"iden",
                properties: vec![writer::ispe_box(h, w), writer::irot_box(3)],
                references: vec![(*b"dimg", vec![1])],
                ..Item::default()
            },
        ],
    );
    o.check("iden", &iden, 2);

    // a 2×2 grid of separately coded tiles, cropped by the grid size, then mirrored
    // heif-enc codes sizes that are multiples of 64 without a clean aperture
    let (tw, th) = (64u32, 64u32);
    let mut tiles = Vec::new();
    for i in 0..4u32 {
        let t = o.encode(&format!("tile{i}"), &picture(tw, th, 20 + i), &["-q", "90"]);
        tiles.push(hevc_item(&t));
    }
    let (gw, gh) = (tw * 2 - 10, th * 2 - 6);
    let mut grid_props = vec![writer::ispe_box(gw, gh), writer::imir_box(1)];
    grid_props.extend(nclx_box(&tiles[0].2));
    let mut items = vec![Item {
        id: 10,
        kind: *b"grid",
        data: writer::grid_data(2, 2, gw, gh),
        properties: grid_props,
        references: vec![(*b"dimg", vec![1, 2, 3, 4])],
        ..Item::default()
    }];
    for (i, (hvcc, data, colour)) in tiles.iter().enumerate() {
        let mut properties = vec![writer::hvcc_box(hvcc), writer::ispe_box(tw, th)];
        properties.extend(nclx_box(colour));
        items.push(Item { id: i as u32 + 1, kind: *b"hvc1", data: data.clone(), properties, hidden: true, ..Item::default() });
    }
    o.check("grid", &writer::build(10, &items), 2);
}

#[test]
fn opens_as_a_still_through_the_media_layer() {
    let Some(o) = Oracle::new() else { return };
    let bytes = o.encode("open", &picture(40, 30, 9), &["-q", "90"]);
    let src = cfa4_media::open_bytes("photo.heic", bytes.into(), &[]).unwrap();
    let v = src.info().video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (40, 30));
    assert_eq!(src.info().kind, cfa4_media::MediaKind::Still);
}

/// The alpha plane, a monochrome HEVC auxiliary image (#136), decodes natively like libheif's:
/// plain, and with transforms on the master alone or on both items.
#[test]
fn alpha_images_decode_like_libheif() {
    let Some(o) = Oracle::new() else { return };
    let rgba = image::RgbaImage::from_fn(64, 48, |x, y| image::Rgba([(x * 4) as u8, (y * 5) as u8, 90, if x < 32 { 255 } else { (x * 3 + y) as u8 }]));
    let png = o.dir.join("alpha.png");
    let out = o.dir.join("alpha.heic");
    rgba.save(&png).unwrap();
    run(Command::new(&o.enc).args(["-q", "90"]).arg("-o").arg(&out).arg(&png));
    let bytes = std::fs::read(&out).unwrap();
    assert!(cfa4_heif::parse(&bytes).unwrap().primary.alpha.is_some(), "heif-enc writes the alpha as an auxiliary image");
    let got = heic::decode("alpha.heic", &bytes).unwrap();
    assert!(got.has_alpha);
    o.check("alpha", &bytes, 3);
    // 10-bit colour and alpha
    let out10 = o.dir.join("alpha10.heic");
    run(Command::new(&o.enc).args(["-q", "90", "-b", "10"]).arg("-o").arg(&out10).arg(&png));
    o.check("alpha10", &std::fs::read(&out10).unwrap(), 3);
    // the alpha follows the master's rotation and mirroring
    let p = cfa4_heif::parse(&bytes).unwrap().primary;
    let a = p.alpha.as_ref().unwrap();
    let (Coding::Hevc { hvcc, data }, Coding::Hevc { hvcc: ahvcc, data: adata }) = (&p.coding, &a.coding) else { panic!("HEVC items") };
    let mut colour = vec![writer::hvcc_box(hvcc), writer::ispe_box(64, 48), writer::irot_box(1), writer::imir_box(0)];
    colour.extend(nclx_box(&p.nclx().cloned()));
    let alpha = Item {
        id: 2,
        kind: *b"hvc1",
        data: adata.clone(),
        properties: vec![writer::hvcc_box(ahvcc), writer::ispe_box(64, 48), writer::auxc_box("urn:mpeg:mpegB:cicp:systems:auxiliary:alpha")],
        references: vec![(*b"auxl", vec![1])],
        hidden: true,
    };
    let file = writer::build(1, &[Item { id: 1, kind: *b"hvc1", data: data.clone(), properties: colour.clone(), ..Item::default() }, alpha.clone()]);
    o.check("alpha_master_transforms_only", &file, 3);
    // as an encoder writes it: the alpha item carries the same transforms
    let mut alpha = alpha;
    alpha.properties.extend([writer::irot_box(1), writer::imir_box(0)]);
    let file = writer::build(1, &[Item { id: 1, kind: *b"hvc1", data: data.clone(), properties: colour, ..Item::default() }, alpha]);
    o.check("alpha_rot_mirror", &file, 3);
}
