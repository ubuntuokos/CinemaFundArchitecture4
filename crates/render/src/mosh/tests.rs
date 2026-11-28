//! DM0: the "foreign IDR + B's P pictures" stream decodes with our own decoder, and the result
//! starts out as A, not B.

use super::*;

/// A soft diagonal gradient with a grid of bright dots, shifted `dx` pixels right: plenty of
/// texture for the motion search to lock on to.
fn scene(w: usize, h: usize, dx: usize, hue: [f32; 3]) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let sx = x + w - dx % w;
            let dot = if (sx % 8 < 3) && (y % 8 < 3) { 0.6 } else { 0.0 };
            let g = (x + y) as f32 / (w + h) as f32 * 0.4;
            let c = hue.map(|k| dec([(k * 0.5 + g + dot).min(1.0); 3])[0]);
            let i = (y * w + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

/// Mean absolute difference of the display-encoded RGB.
fn diff(a: &Image, b: &Image) -> f32 {
    let n = (a.w * a.h) as f32;
    a.px.chunks_exact(4)
        .zip(b.px.chunks_exact(4))
        .map(|(p, q)| {
            let (e, f) = (enc([p[0], p[1], p[2]]), enc([q[0], q[1], q[2]]));
            (0..3).map(|k| (e[k] - f[k]).abs()).sum::<f32>() / 3.0
        })
        .sum::<f32>()
        / n
}

#[test]
fn foreign_idr_then_p_pictures_decodes_and_starts_from_a() {
    let (w, h) = (64, 48);
    let a = scene(w, h, 0, [0.9, 0.1, 0.1]);
    let b: Vec<Image> = (0..6).map(|i| scene(w, h, i * 2, [0.1, 0.2, 0.9])).collect();
    let out = mosh(&a, &b, 30).expect("mosh");
    assert_eq!(out.len(), b.len() - 1);
    for o in &out {
        assert_eq!((o.w, o.h), (w, h));
        assert!(o.px.iter().all(|v| v.is_finite()));
    }
    // the first moshed picture is A moved by B's motion: far closer to A than to B
    let (to_a, to_b) = (diff(&out[0], &a), diff(&out[0], &b[1]));
    eprintln!("first moshed picture: mean |Δ| to A {to_a:.4}, to B {to_b:.4}");
    assert!(to_a < to_b * 0.5, "to A {to_a}, to B {to_b}");
    // a higher QP lets less of B's detail in
    let coarse = mosh(&a, &b, 45).expect("mosh qp 45");
    assert!(diff(&coarse[4], &b[5]) >= diff(&out[4], &b[5]) * 0.9);
    // the same input gives the same pictures
    assert_eq!(mosh(&a, &b, 30).expect("again")[2].px, out[2].px);
}

/// Coarse random blocks (4 px): new content the motion search cannot find in the picture before.
fn blocks(w: usize, h: usize, seed: u32) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut s = (((x / 4) * 7919 + (y / 4) * 104_729) as u32) ^ seed.wrapping_mul(0x9e37_79b9);
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let c = [(s & 255) as f32 / 255.0, ((s >> 8) & 255) as f32 / 255.0, ((s >> 16) & 255) as f32 / 255.0];
            let i = (y * w + x) * 4;
            img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    img
}

#[test]
fn without_clean_blocks_more_of_a_survives() {
    // B cuts to new content after two pictures: with Clean Blocks the encoder refreshes the
    // poorly predicted blocks with intra blocks of B, so B shows through cleanly; without them
    // every block is moved from the moshed picture before and keeps more of A.
    let (w, h) = (64, 48);
    let a = scene(w, h, 0, [0.9, 0.1, 0.1]);
    let b: Vec<Image> = (0..6).map(|i| if i < 2 { scene(w, h, i * 2, [0.1, 0.2, 0.9]) } else { blocks(w, h, 7) }).collect();
    let run = |clean: bool| {
        let mut s = Session::new(&a, &b[0], 30, clean).expect("session");
        b[1..].iter().map(|f| s.push(f).expect("push")).collect::<Vec<_>>()
    };
    let (clean, dirty) = (run(true), run(false));
    let (c, d) = (diff(&clean[1], &b[2]), diff(&dirty[1], &b[2]));
    eprintln!("picture after the cut: mean |Δ| to B with clean blocks {c:.4}, without {d:.4}");
    assert!(d > c * 1.2, "with clean blocks {c}, without {d}");
    // the bloom takes the switch too: the motion into new content repeats differently
    let r = scene(w, h, 0, [0.9, 0.1, 0.1]);
    let step = |clean: bool| Bloom::new(&b[1], &b[2], 30, clean).expect("bloom").step(&r).expect("step");
    let (sc, sd) = (step(true), step(false));
    assert!(sd.px.iter().all(|v| v.is_finite()));
    assert_ne!(sc.px, sd.px);
    assert_eq!(step(false).px, sd.px);
}

#[test]
fn odd_tiny_and_bad_inputs_never_panic() {
    for (w, h) in [(1, 1), (3, 5), (17, 9), (2, 2)] {
        let a = scene(w, h, 0, [0.5; 3]);
        let b: Vec<Image> = (0..3).map(|i| scene(w, h, i, [0.2; 3])).collect();
        if let Ok(out) = mosh(&a, &b, 26) {
            assert_eq!(out.len(), 2);
            assert!(out.iter().all(|o| (o.w, o.h) == (w, h) && o.px.iter().all(|v| v.is_finite())));
        }
    }
    let a = scene(16, 16, 0, [0.5; 3]);
    assert_eq!(mosh(&a, &[], 26), Err(MoshError::BadInput));
    assert_eq!(mosh(&a, &[scene(16, 16, 0, [0.5; 3])], 26), Err(MoshError::BadInput));
    assert_eq!(mosh(&a, &[scene(16, 8, 0, [0.5; 3]), scene(16, 8, 0, [0.5; 3])], 26), Err(MoshError::BadInput));
    assert_eq!(mosh(&Image::new(0, 0), &[Image::new(0, 0), Image::new(0, 0)], 26), Err(MoshError::BadInput));
    // NaN pixels and an out-of-range QP
    let mut bad = scene(16, 16, 0, [0.5; 3]);
    bad.px.iter_mut().step_by(3).for_each(|v| *v = f32::NAN);
    let out = mosh(&bad, &[bad.clone(), bad.clone()], 255).expect("NaN input still moshes");
    assert!(out[0].px.iter().all(|v| v.is_finite()));
}

/// DM4: frame times of the mosh at 1080p (full, ½ and ¼ encode resolution). Not a gate: run on
/// the reference machine with
/// `cargo test -p cfa4-render --release --lib mosh::tests::frame_times_1080p -- --ignored --nocapture`.
#[test]
#[ignore]
fn frame_times_1080p() {
    use std::time::Instant;
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    for d in [1, 2, 4] {
        let (w, h) = (1920 / d, 1080 / d);
        let a = scene(w, h, 0, [0.9, 0.1, 0.1]);
        let b: Vec<Image> = (0..25).map(|i| scene(w, h, i * 4 / d, [0.1, 0.2, 0.9])).collect();
        let t = Instant::now();
        let mut s = Session::new(&a, &b[0], 30, true).expect("session");
        let start = ms(t);
        let t = Instant::now();
        for f in &b[1..] {
            s.push(f).expect("push");
        }
        let push = ms(t) / 24.0;
        let t = Instant::now();
        let bloom = Bloom::new(&b[0], &b[1], 30, true).expect("bloom");
        let bloom_new = ms(t);
        let mut r = b[1].clone();
        let t = Instant::now();
        for _ in 0..12 {
            r = bloom.step(&r).expect("step");
        }
        let step = ms(t) / 12.0;
        eprintln!("{w}x{h}: transition start {start:.1} ms, next frame {push:.1} ms; bloom start {bloom_new:.1} ms, next repeat {step:.1} ms");
    }
}
