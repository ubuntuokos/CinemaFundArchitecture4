use super::*;

const ALL: [UncompressedLayout; 9] = [
    UncompressedLayout::CbYCr444x8,
    UncompressedLayout::Uyvy8,
    UncompressedLayout::Yuy2x8,
    UncompressedLayout::Planar422x8,
    UncompressedLayout::V210,
    UncompressedLayout::Planar422x10,
    UncompressedLayout::CbYCrY12,
    UncompressedLayout::CbYCrY16,
    UncompressedLayout::Planar420x8,
];

fn info(layout: UncompressedLayout, w: u32, h: u32) -> PictureInfo {
    PictureInfo { stored_width: w, stored_height: h, picture_coding: Some(Ul(layout.label())), ..Default::default() }
}

/// Deterministic test planes of `depth` bits.
fn planes(layout: UncompressedLayout, w: u32, h: u32) -> [Vec<u16>; 3] {
    let (sx, sy) = layout.subsampling();
    let (w, h) = (w as usize, h as usize);
    let n = [w * h, w.div_ceil(sx as usize) * h.div_ceil(sy as usize), w.div_ceil(sx as usize) * h.div_ceil(sy as usize)];
    let mask = ((1u32 << layout.depth()) - 1) as u16;
    let mut seed = 0x1234_5678u32;
    n.map(|n| {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 13) as u16 & mask
            })
            .collect()
    })
}

#[test]
fn labels_round_trip() {
    for l in ALL {
        assert_eq!(UncompressedLayout::from_label(Ul(l.label())), Some(l));
    }
    // version byte ignored; other tails and compressed codings are not layouts
    let mut b = UncompressedLayout::V210.label();
    b[7] = 0x01;
    assert_eq!(UncompressedLayout::from_label(Ul(b)), Some(UncompressedLayout::V210));
    b[15] = 0x09;
    assert_eq!(UncompressedLayout::from_label(Ul(b)), None);
}

#[test]
fn every_layout_round_trips_through_pack_and_decode() {
    for l in ALL {
        for (w, h) in [(48, 4), (96, 6), (12, 2)] {
            let p = planes(l, w, h);
            let bytes = l.pack(w, h, [&p[0], &p[1], &p[2]]);
            assert_eq!(bytes.len() as u64, l.picture_bytes(w, h), "{l:?} {w}x{h}");
            let pic = decode(&Layout::YCbCr(l), &info(l, w, h), &bytes).unwrap();
            assert_eq!(pic.planes, p, "{l:?} {w}x{h}");
            assert_eq!((pic.depth, pic.subsampling, pic.rgb), (l.depth(), l.subsampling(), false));
            assert_eq!(pic.display, (0, 0, w, h));
        }
    }
}

#[test]
fn byte_order_matches_the_layout_tables() {
    // UYVY: Cb Y0 Cr Y1
    let pic = decode(&Layout::YCbCr(UncompressedLayout::Uyvy8), &info(UncompressedLayout::Uyvy8, 2, 1), &[10, 20, 30, 40]).unwrap();
    assert_eq!(pic.planes, [vec![20, 40], vec![10], vec![30]]);
    // YUY2: Y0 Cb Y1 Cr
    let pic = decode(&Layout::YCbCr(UncompressedLayout::Yuy2x8), &info(UncompressedLayout::Yuy2x8, 2, 1), &[10, 20, 30, 40]).unwrap();
    assert_eq!(pic.planes, [vec![10, 30], vec![20], vec![40]]);
    // 12-bit Cb Y Cr Y: 0x123 0x456 0x789 0xABC as big-endian bit-fields
    let pic = decode(&Layout::YCbCr(UncompressedLayout::CbYCrY12), &info(UncompressedLayout::CbYCrY12, 2, 1), &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]).unwrap();
    assert_eq!(pic.planes, [vec![0x456, 0xABC], vec![0x123], vec![0x789]]);
    // 16-bit: big-endian words
    let pic = decode(&Layout::YCbCr(UncompressedLayout::CbYCrY16), &info(UncompressedLayout::CbYCrY16, 2, 1), &[0, 1, 0, 2, 0, 3, 0, 4]).unwrap();
    assert_eq!(pic.planes, [vec![2, 4], vec![1], vec![3]]);
    // v210: little-endian words, Cb0 in the low bits of the first, Y0 next, Cr0 on top
    let mut v = Vec::new();
    let words: [[u32; 3]; 4] = [[64, 940, 512], [100, 200, 101], [300, 102, 400], [103, 500, 104]];
    for w in words {
        v.extend_from_slice(&(w[0] | w[1] << 10 | w[2] << 20).to_le_bytes());
    }
    v.resize(128, 0);
    let pic = decode(&Layout::YCbCr(UncompressedLayout::V210), &info(UncompressedLayout::V210, 6, 1), &v).unwrap();
    assert_eq!(pic.planes, [vec![940, 100, 101, 102, 103, 104], vec![64, 200, 400], vec![512, 300, 500]]);
}

#[test]
fn descriptor_properties_give_the_layout_without_a_label() {
    let cdci = |d, h, v, reversed| PictureInfo {
        component_depth: d,
        horizontal_subsampling: h,
        vertical_subsampling: v,
        reversed_byte_order: reversed,
        ..Default::default()
    };
    assert_eq!(Layout::of(&cdci(8, 2, 1, false)).unwrap(), Layout::Interleaved { pattern: CB_Y_CR_Y, pixels: 2, depth: 8, padding: 0 });
    assert_eq!(Layout::of(&cdci(8, 2, 1, true)).unwrap(), Layout::Interleaved { pattern: Y_CB_Y_CR, pixels: 2, depth: 8, padding: 0 });
    assert_eq!(Layout::of(&cdci(10, 2, 1, false)).unwrap(), Layout::YCbCr(UncompressedLayout::V210));
    assert_eq!(Layout::of(&cdci(8, 2, 2, false)).unwrap(), Layout::YCbCr(UncompressedLayout::Planar420x8));
    assert_eq!(Layout::of(&cdci(10, 1, 1, false)).unwrap().name(), "Uncompressed 4:4:4 10-bit");
    assert!(Layout::of(&cdci(0, 2, 1, false)).is_err());
    assert!(Layout::of(&cdci(10, 4, 1, false)).is_err());
    // the label wins over conflicting properties
    let mut p = cdci(8, 1, 1, false);
    p.picture_coding = Some(Ul(UncompressedLayout::V210.label()));
    assert_eq!(Layout::of(&p).unwrap(), Layout::YCbCr(UncompressedLayout::V210));
    // 10-bit 4:4:4 with 2 padding bits per pixel (32 bits): Cb Y Cr then pad
    let mut p = cdci(10, 1, 1, false);
    p.padding_bits = 2;
    p.stored_width = 1;
    p.stored_height = 1;
    let l = Layout::of(&p).unwrap();
    let word: u32 = 0x100 << 22 | 0x200 << 12 | 0x300 << 2;
    let pic = decode(&l, &p, &word.to_be_bytes()).unwrap();
    assert_eq!(pic.planes, [vec![0x200], vec![0x100], vec![0x300]]);
}

#[test]
fn rgba_pixel_layouts() {
    let rgba = |layout: &[(u8, u8)], w, h| PictureInfo { rgba: true, stored_width: w, stored_height: h, pixel_layout: layout.to_vec(), ..Default::default() };
    // 8-bit B G R A (byte order of a common little-endian ARGB word)
    let p = rgba(&[(b'B', 8), (b'G', 8), (b'R', 8), (b'A', 8)], 2, 1);
    let l = Layout::of(&p).unwrap();
    assert_eq!(l.name(), "Uncompressed RGBA 8-bit");
    let pic = decode(&l, &p, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    assert!(pic.rgb);
    assert_eq!(pic.planes, [vec![3, 7], vec![2, 6], vec![1, 5]]);
    assert_eq!(pic.alpha, Some(vec![4, 8]));
    // 10-bit R G B and 2 fill bits (DPX-style), video-range codes rescaled to full range
    let mut p = rgba(&[(b'R', 10), (b'G', 10), (b'B', 10), (b'F', 2)], 1, 1);
    p.component_min_ref = Some(64);
    p.component_max_ref = Some(940);
    let l = Layout::of(&p).unwrap();
    let word: u32 = 940 << 22 | 64 << 12 | 502 << 2;
    let pic = decode(&l, &p, &word.to_be_bytes()).unwrap();
    assert_eq!(pic.planes, [vec![1023], vec![0], vec![512]]);
    assert_eq!(pic.alpha, None);
    // X'Y'Z', palettes, mixed depths and floats are refused
    for bad in [
        vec![(0xD8, 12), (0xD9, 12), (0xDA, 12)],
        vec![(b'P', 8)],
        vec![(b'R', 8), (b'G', 10), (b'B', 8)],
        vec![(b'R', 254), (b'G', 254), (b'B', 254)],
        vec![(b'R', 8), (b'G', 8)],
    ] {
        assert!(Layout::of(&rgba(&bad, 1, 1)).is_err(), "{bad:?}");
    }
}

#[test]
fn separate_fields_are_woven_and_fills_skipped() {
    let l = UncompressedLayout::Uyvy8;
    let mut p = info(l, 2, 1);
    p.frame_layout = 1;
    p.image_start_offset = 2;
    p.image_end_offset = 1;
    // field 1: Y 1 2, field 2: Y 3 4
    let data = [0xEE, 0xEE, 0, 1, 0, 2, 0xEE, 0xEE, 0xEE, 0, 3, 0, 4, 0xEE];
    assert_eq!(Layout::YCbCr(l).edit_unit_bytes(&p), 14);
    let pic = decode(&Layout::YCbCr(l), &p, &data).unwrap();
    assert_eq!((pic.width, pic.height), (2, 2));
    assert_eq!(pic.planes[0], vec![1, 2, 3, 4]);
}

#[test]
fn display_rectangle_comes_from_the_offsets() {
    let l = UncompressedLayout::CbYCr444x8;
    let mut p = info(l, 8, 6);
    p.display_width = 4;
    p.display_height = 2;
    p.display_x_offset = 2;
    p.sampled_y_offset = 1;
    let bytes = vec![0; l.picture_bytes(8, 6) as usize];
    assert_eq!(decode(&Layout::YCbCr(l), &p, &bytes).unwrap().display, (2, 1, 4, 2));
    // hostile offsets stay inside the picture
    p.display_x_offset = i32::MAX;
    p.display_y_offset = i32::MIN;
    p.display_width = 100;
    let d = decode(&Layout::YCbCr(l), &p, &bytes).unwrap().display;
    assert!(d.0 + d.2 <= 8 && d.1 + d.3 <= 6, "{d:?}");
}

#[test]
fn hostile_sizes_are_errors() {
    let l = UncompressedLayout::Uyvy8;
    // short data
    assert!(decode(&Layout::YCbCr(l), &info(l, 4, 4), &[0; 31]).is_err());
    // no dimensions
    assert!(decode(&Layout::YCbCr(l), &info(l, 0, 4), &[0; 64]).is_err());
    // giant dimensions are refused before any allocation
    assert!(decode(&Layout::YCbCr(l), &info(l, u32::MAX, u32::MAX), &[0; 64]).is_err());
    let mut p = info(l, 2, 1);
    p.image_start_offset = u32::MAX;
    p.frame_layout = 1;
    assert!(decode(&Layout::YCbCr(l), &p, &[0; 64]).is_err());
}
