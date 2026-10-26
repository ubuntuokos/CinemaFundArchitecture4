use super::*;
use crate::{FrameRequest, MediaKind, MediaSource, open_bytes};
use cfa4_time::{TICKS_PER_SECOND, Tick};

/// A lossless WebP of `w`×`h` RGBA `px`, as its `VP8L` chunk payload.
fn vp8l(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(px, w, h, image::ExtendedColorType::Rgba8).unwrap();
    let (fourcc, payload) = chunks(&out[12..]).next().unwrap();
    assert_eq!(&fourcc, b"VP8L");
    payload.to_vec()
}

fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

struct F {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    ms: u32,
    blend: bool,
    dispose: bool,
    px: Vec<u8>,
}

fn u24(v: u32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// An animated WebP with a `cw`×`ch` canvas, a magenta background colour hint and `frames`.
fn animation(cw: u32, ch: u32, frames: &[F]) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    let mut vp8x = vec![0x12, 0, 0, 0];
    vp8x.extend(u24(cw - 1));
    vp8x.extend(u24(ch - 1));
    chunk(&mut body, b"VP8X", &vp8x);
    chunk(&mut body, b"ANIM", &[255, 0, 255, 255, 0, 0]);
    for f in frames {
        let mut anmf = Vec::new();
        anmf.extend(u24(f.x / 2));
        anmf.extend(u24(f.y / 2));
        anmf.extend(u24(f.w - 1));
        anmf.extend(u24(f.h - 1));
        anmf.extend(u24(f.ms));
        anmf.push(u8::from(!f.blend) << 1 | u8::from(f.dispose));
        chunk(&mut anmf, b"VP8L", &vp8l(f.w, f.h, &f.px));
        chunk(&mut body, b"ANMF", &anmf);
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn solid(w: u32, h: u32, p: [u8; 4]) -> Vec<u8> {
    p.repeat((w * h) as usize)
}

fn ms(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND / 1000)
}

fn at(s: &dyn MediaSource, t: Tick, x: u32, y: u32, w: u32) -> [u8; 4] {
    let f = s.video_frame(FrameRequest::full(t)).unwrap();
    let cfa4_frame::PixelData::Rgba8(p) = &f.data else { panic!("RGBA8") };
    let i = ((y * w + x) * 4) as usize;
    p[i..i + 4].try_into().unwrap()
}

fn sample() -> Vec<u8> {
    animation(
        8,
        6,
        &[
            F { x: 0, y: 0, w: 8, h: 6, ms: 100, blend: false, dispose: false, px: solid(8, 6, [10, 20, 30, 255]) },
            F { x: 2, y: 2, w: 4, h: 2, ms: 50, blend: true, dispose: true, px: solid(4, 2, [200, 0, 0, 128]) },
            F { x: 4, y: 2, w: 4, h: 4, ms: 0, blend: true, dispose: false, px: solid(4, 4, [0, 250, 0, 255]) },
        ],
    )
}

#[test]
fn frames_are_composed_on_the_canvas_with_their_timing() {
    let s = open_bytes("anim.webp", sample().into(), &[]).unwrap();
    let info = s.info();
    assert_eq!(info.kind, MediaKind::Movie);
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (8, 6, "WebP"));
    assert_eq!(info.duration, ms(250), "100 + 50 + a zero delay played as 100 ms");
    assert!(v.has_alpha, "the disposed rectangle leaves a transparent hole");

    assert_eq!(at(&*s, Tick::ZERO, 3, 3, 8), [10, 20, 30, 255]);
    // half-transparent red over the opaque canvas: libwebp's integer blend
    assert_eq!(at(&*s, ms(100), 3, 3, 8), blend_pixel([200, 0, 0, 128], [10, 20, 30, 255]));
    assert_eq!(at(&*s, ms(100), 0, 0, 8), [10, 20, 30, 255], "outside the frame the canvas stays");
    // the red frame was disposed: its rectangle is transparent (not the magenta hint) under the
    // green frame, which covers only part of it
    assert_eq!(at(&*s, ms(150), 2, 2, 8), [0, 0, 0, 0]);
    assert_eq!(at(&*s, ms(150), 5, 3, 8), [0, 250, 0, 255]);
    assert_eq!(at(&*s, ms(150), 0, 0, 8), [10, 20, 30, 255]);
}

#[test]
fn a_disposed_area_shows_through_as_transparency() {
    let s = open_bytes("anim.webp", sample().into(), &[]).unwrap();
    let bytes = sample();
    let anim = parse(&bytes).unwrap();
    let pictures: Vec<_> = anim.frames().map(|f| f.unwrap().into_buffer()).collect();
    assert_eq!(pictures.len(), 3);
    assert_eq!(pictures[2].get_pixel(3, 3).0, [0, 0, 0, 0]);
    assert_eq!(at(&*s, ms(200), 3, 3, 8), [0, 0, 0, 0]);
}

#[test]
fn blending_follows_libwebp() {
    assert_eq!(blend_pixel([1, 2, 3, 255], [9, 9, 9, 9]), [1, 2, 3, 255], "opaque replaces exactly");
    assert_eq!(blend_pixel([1, 2, 3, 0], [9, 8, 7, 6]), [9, 8, 7, 6], "transparent keeps the canvas");
    // over a transparent canvas the colour keeps libwebp's rounding
    let p = blend_pixel([200, 100, 50, 128], [0, 0, 0, 0]);
    assert_eq!(p[3], 128);
    for a in 1..=255u8 {
        for d in [0u8, 1, 128, 255] {
            let p = blend_pixel([255, 0, 128, a], [0, 255, 7, d]);
            assert!(p[3] >= a, "blended alpha never drops below the frame's");
        }
    }
}

#[test]
fn a_huge_canvas_is_refused_before_allocating() {
    let mut bytes = sample();
    // canvas 2^24 × 2^24
    bytes[24..30].copy_from_slice(&[0xFF; 6]);
    let anim = parse(&bytes).unwrap();
    assert_eq!((anim.width, anim.height), (1 << 24, 1 << 24));
    assert!(!anim.fits());
    assert!(open_bytes("huge.webp", bytes.into(), &[]).is_err());
}

#[test]
fn a_frame_outside_the_canvas_ends_the_animation() {
    let bytes = animation(
        8,
        6,
        &[
            F { x: 0, y: 0, w: 8, h: 6, ms: 40, blend: false, dispose: false, px: solid(8, 6, [1, 1, 1, 255]) },
            F { x: 0, y: 0, w: 8, h: 6, ms: 40, blend: false, dispose: false, px: solid(8, 6, [2, 2, 2, 255]) },
            F { x: 6, y: 0, w: 4, h: 2, ms: 40, blend: false, dispose: false, px: solid(4, 2, [3, 3, 3, 255]) },
            F { x: 0, y: 0, w: 8, h: 6, ms: 40, blend: false, dispose: false, px: solid(8, 6, [4, 4, 4, 255]) },
        ],
    );
    let s = open_bytes("bad.webp", bytes.into(), &[]).unwrap();
    assert_eq!(s.info().duration, ms(80));
    assert_eq!(at(&*s, ms(79), 0, 0, 8), [2, 2, 2, 255]);
}

#[test]
fn stills_and_other_files_are_not_animations() {
    let mut still = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut still).encode(&solid(3, 2, [5, 6, 7, 255]), 3, 2, image::ExtendedColorType::Rgba8).unwrap();
    assert!(parse(&still).is_none());
    assert_eq!(open_bytes("still.webp", still.into(), &[]).unwrap().info().kind, MediaKind::Still);
    assert!(parse(b"RIFF\x04\0\0\0WAVE").is_none());
    assert!(parse(b"RIFF").is_none());
}

#[test]
fn damaged_files_never_panic() {
    let whole = sample();
    for cut in 0..whole.len() {
        let _ = open_bytes("cut.webp", whole[..cut].to_vec().into(), &[]);
    }
    let mut s = 0x9E37_79B9u32;
    for round in 0..400 {
        let mut b = whole.clone();
        for _ in 0..1 + round % 6 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let i = 12 + (s >> 8) as usize % (b.len() - 12);
            b[i] = (s >> 24) as u8;
        }
        let _ = open_bytes("flipped.webp", b.into(), &[]);
    }
}
