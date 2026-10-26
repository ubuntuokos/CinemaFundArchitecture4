use super::*;
use cfa4_heif::CleanAperture;
use cfa4_heif::writer::{self, Item};

/// A 3×2 picture whose pixels are numbered 0..6 (in the red channel).
fn numbered() -> Rgba {
    Rgba { width: 3, height: 2, px: (0..6u8).flat_map(|i| [i, 0, 0, 255]).collect() }
}

fn reds(p: &Rgba) -> Vec<u8> {
    p.px.as_chunks::<4>().0.iter().map(|q| q[0]).collect()
}

#[test]
fn transforms_move_pixels_where_libheif_does() {
    // 0 1 2
    // 3 4 5
    let t = |tr| {
        let p = transform(numbered(), tr).unwrap();
        ((p.width, p.height), reds(&p))
    };
    assert_eq!(t(Transform::Rotate(1)), ((2, 3), vec![2, 5, 1, 4, 0, 3]), "a quarter turn anticlockwise");
    assert_eq!(t(Transform::Rotate(2)), ((3, 2), vec![5, 4, 3, 2, 1, 0]));
    assert_eq!(t(Transform::Rotate(3)), ((2, 3), vec![3, 0, 4, 1, 5, 2]));
    assert_eq!(t(Transform::Mirror { left_right: true }), ((3, 2), vec![2, 1, 0, 5, 4, 3]));
    assert_eq!(t(Transform::Mirror { left_right: false }), ((3, 2), vec![3, 4, 5, 0, 1, 2]));
    let crop = CleanAperture { width: (2, 1), height: (1, 1), horizontal_offset: (1, 2), vertical_offset: (1, 2) };
    assert_eq!(t(Transform::Crop(crop)), ((2, 1), vec![4, 5]));
}

fn hevc_file(data: &[u8], extra: Vec<Vec<u8>>) -> Vec<u8> {
    let mut properties =
        vec![writer::hvcc_box(&[1, 1, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF0, 0, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F, 0]), writer::ispe_box(16, 16)];
    properties.extend(extra);
    writer::build(1, &[Item { id: 1, kind: *b"hvc1", data: data.to_vec(), properties, ..Item::default() }])
}

#[test]
fn what_we_do_not_decode_is_unsupported_so_libheif_can_try() {
    let av1 = writer::build(1, &[Item { id: 1, kind: *b"av01", data: vec![0; 8], ..Item::default() }]);
    assert!(matches!(decode("a.avif", &av1), Err(MediaError::Unsupported(_))));
}

#[test]
fn damaged_files_are_errors_not_panics() {
    assert!(decode("x.heic", b"not HEIF").is_err());
    // an HEVC item whose bitstream is garbage
    let mut s = 0x1234_5678u32;
    for n in [0usize, 1, 7, 64, 500] {
        let data: Vec<u8> = (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 24) as u8
            })
            .collect();
        assert!(decode("garbage.heic", &hevc_file(&data, vec![])).is_err());
    }
    // a grid far larger than the decoded-size limit
    let grid =
        Item { id: 10, kind: *b"grid", data: writer::grid_data(1, 1, 0xFFFF_FFFF, 0xFFFF_FFFF), references: vec![(*b"dimg", vec![1])], ..Item::default() };
    let tile = Item { id: 1, kind: *b"hvc1", data: vec![0; 4], properties: vec![writer::hvcc_box(&[1; 23])], ..Item::default() };
    assert!(decode("huge.heic", &writer::build(10, &[grid, tile])).is_err());
}

#[test]
fn colour_follows_colr_then_the_vui_then_libheif_defaults() {
    let pic = |matrix: u8, primaries: u8, range_signalled: bool| cfa4_hevc::Picture {
        width: 2,
        height: 2,
        chroma_width: 1,
        chroma_height: 1,
        bit_depth: 8,
        y: cfa4_hevc::Plane::U8(vec![0; 4]),
        u: cfa4_hevc::Plane::U8(vec![0]),
        v: cfa4_hevc::Plane::U8(vec![0]),
        y_stride: 2,
        uv_stride: 1,
        pts: 0,
        poc: 0,
        key: true,
        color: cfa4_hevc::ColorInfo { full_range: false, range_signalled, primaries, transfer: 1, matrix },
        sar: (0, 0),
        draft: false,
    };
    let nclx = Colour::Nclx { primaries: 12, transfer: 13, matrix: 1, full_range: true };
    let c = colour(Some(&nclx), &pic(9, 9, true));
    assert_eq!((c.matrix, c.primaries, c.transfer, c.range), (Matrix::Bt709, Primaries::P3D65, Transfer::Srgb, Range::Full));
    let c = colour(None, &pic(9, 9, true));
    assert_eq!((c.matrix, c.primaries, c.range), (Matrix::Bt2020Ncl, Primaries::Bt2020, Range::Limited));
    let c = colour(None, &pic(2, 2, false));
    assert_eq!((c.matrix, c.transfer, c.range), (Matrix::Bt601, Transfer::Srgb, Range::Full));
    // a VUI that signals limited range without colour codes keeps its range
    let c = colour(None, &pic(2, 2, true));
    assert_eq!((c.matrix, c.transfer, c.range), (Matrix::Bt601, Transfer::Srgb, Range::Limited));
}

/// Nested grids keep each parent canvas alive while their tiles decode: eight nested 1×1 grids
/// of 8192×16384 (512 MiB each) must stop at the live limit, not allocate 4 GiB.
#[test]
fn nested_grid_canvases_are_charged_together() {
    let leaf = Image { item: 9, coding: Coding::Other(*b"jpeg"), size: None, colour: vec![], transforms: vec![], alpha: None };
    let image = (0..8).fold(leaf, |tile, i| Image {
        item: i,
        coding: Coding::Grid { rows: 1, columns: 1, width: 8192, height: 16384, tiles: vec![tile] },
        size: Some((8192, 16384)),
        colour: vec![],
        transforms: vec![],
        alpha: None,
    });
    let mut live = Live::default();
    let Err(e) = decode_image("nested.heic", &image, None, Samples::Colour, &mut live) else { panic!("decoded") };
    assert!(e.to_string().contains("decoding limit"), "{e}");
    // a single grid of that size still gets as far as its tile
    let one = Image {
        item: 1,
        coding: Coding::Grid {
            rows: 1,
            columns: 1,
            width: 8192,
            height: 16384,
            tiles: vec![Image { item: 2, coding: Coding::Other(*b"jpeg"), size: None, colour: vec![], transforms: vec![], alpha: None }],
        },
        size: None,
        colour: vec![],
        transforms: vec![],
        alpha: None,
    };
    let Err(e) = decode_image("one.heic", &one, None, Samples::Colour, &mut Live::default()) else { panic!("decoded") };
    assert!(e.to_string().contains("'jpeg'"), "{e}");
}
