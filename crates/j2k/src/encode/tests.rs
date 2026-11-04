use super::*;
use crate::{Plane, decode::decode};

fn rnd(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

/// A test picture: smooth gradients, an edge and some noise, in each component's range.
fn picture(w: u32, h: u32, comps: &[(u8, bool, u8, u8)], seed: u32) -> Image {
    let mut seed = seed | 1;
    let components = comps
        .iter()
        .enumerate()
        .map(|(c, &(precision, signed, dx, dy))| {
            let (pw, ph) = (w.div_ceil(u32::from(dx)), h.div_ceil(u32::from(dy)));
            let max = (1i64 << precision) - 1;
            let data = (0..pw * ph)
                .map(|k| {
                    let (x, y) = (i64::from(k % pw), i64::from(k / pw));
                    let smooth = (x * 3 + y * 5 + c as i64 * 40) * max / i64::from(3 * pw + 5 * ph + 120).max(1);
                    let edge = if x > i64::from(pw) / 2 { max / 4 } else { 0 };
                    let noise = i64::from(rnd(&mut seed) % 9) - 4;
                    let v = (smooth + edge + noise * (max / 255).max(1)).clamp(0, max);
                    (if signed { v - (1 << (precision - 1)) } else { v }) as i32
                })
                .collect();
            Plane { width: pw, height: ph, precision, signed, dx, dy, data }
        })
        .collect();
    Image { width: w, height: h, components, mct: false }
}

/// Peak signal-to-noise ratio of `b` against `a`, over all components.
fn psnr(a: &Image, b: &Image) -> f64 {
    let (mut se, mut n, mut peak) = (0f64, 0f64, 0f64);
    for (p, q) in a.components.iter().zip(&b.components) {
        peak = f64::max(peak, ((1u32 << p.precision) - 1) as f64);
        for (&x, &y) in p.data.iter().zip(&q.data) {
            se += f64::from(x - y).powi(2);
            n += 1.0;
        }
    }
    if se == 0.0 { f64::INFINITY } else { 10.0 * (peak * peak / (se / n)).log10() }
}

#[test]
fn lossless_round_trips_are_exact() {
    let rgb8 = [(8, false, 1, 1); 3];
    let cases: Vec<(u32, u32, Vec<(u8, bool, u8, u8)>, EncodeOptions)> = vec![
        (1, 1, vec![(8, false, 1, 1)], EncodeOptions::lossless()),
        (7, 5, rgb8.to_vec(), EncodeOptions::lossless()),
        (64, 64, rgb8.to_vec(), EncodeOptions { levels: 0, ..EncodeOptions::lossless() }),
        (
            33,
            17,
            vec![(12, false, 1, 1), (12, false, 2, 2), (12, false, 2, 2)],
            EncodeOptions { progression: Progression::Rpcl, precincts: vec![(4, 4), (5, 5)], ..EncodeOptions::lossless() },
        ),
        (
            130,
            70,
            vec![(16, false, 1, 1); 3],
            EncodeOptions { progression: Progression::Cprl, code_block: (4, 5), tile_part_per_component: true, tlm: true, ..EncodeOptions::lossless() },
        ),
        (
            40,
            90,
            vec![(10, true, 1, 1), (1, false, 1, 1)],
            EncodeOptions { progression: Progression::Pcrl, levels: 3, precincts: vec![(3, 3), (4, 3)], ..EncodeOptions::lossless() },
        ),
        (97, 3, vec![(8, false, 1, 1); 4], EncodeOptions { progression: Progression::Rlcp, levels: 7, ..EncodeOptions::lossless() }),
        (50, 50, rgb8.to_vec(), EncodeOptions { mct: false, ..EncodeOptions::lossless() }),
        // components of different precisions: no component transform (it would clip)
        (9, 6, vec![(1, false, 1, 1), (16, false, 1, 1), (16, false, 1, 1)], EncodeOptions { levels: 0, ..EncodeOptions::lossless() }),
        (9, 6, vec![(8, false, 1, 1), (8, true, 1, 1), (8, false, 1, 1)], EncodeOptions::lossless()),
    ];
    for (k, (w, h, comps, opts)) in cases.into_iter().enumerate() {
        let img = picture(w, h, &comps, k as u32 + 3);
        let cs = encode(&img, &opts).unwrap();
        // `mct` records how the stream was coded; the samples are what must round-trip
        assert_eq!(decode(&cs).unwrap().components, img.components, "case {k}: {w}x{h}");
    }
    // the reported case: [0, 65535, 0] in 1-, 16- and 16-bit components
    let mut img = picture(1, 1, &[(1, false, 1, 1), (16, false, 1, 1), (16, false, 1, 1)], 1);
    for (p, v) in img.components.iter_mut().zip([0, 65535, 0]) {
        p.data = vec![v];
    }
    let cs = encode(&img, &EncodeOptions { levels: 0, ..EncodeOptions::lossless() }).unwrap();
    assert_eq!(decode(&cs).unwrap().components, img.components);
}

#[test]
fn lossy_quality_follows_the_step() {
    let img = picture(96, 80, &[(8, false, 1, 1); 3], 11);
    let mut last = f64::INFINITY;
    for step in [0.5f32, 2.0, 8.0, 32.0] {
        let cs = encode(&img, &EncodeOptions::lossy(step)).unwrap();
        let q = psnr(&img, &decode(&cs).unwrap());
        assert!(q < last + 0.5, "step {step}: {q} dB after {last}");
        last = q;
        if step <= 0.5 {
            assert!(q > 45.0, "step {step}: {q} dB");
        }
    }
    assert!(last > 20.0, "{last} dB at step 32");
}

#[test]
fn byte_limits_hold_and_quality_grows_with_them() {
    let img = picture(128, 96, &[(12, false, 1, 1); 3], 5);
    let mut last = 0.0;
    for limit in [2_000usize, 6_000, 20_000] {
        let opts = EncodeOptions { max_bytes: Some(limit), ..EncodeOptions::lossy(0.5) };
        let cs = encode(&img, &opts).unwrap();
        assert!(cs.len() <= limit, "{} > {limit}", cs.len());
        assert!(cs.len() > limit * 8 / 10, "{} far below {limit}", cs.len());
        let q = psnr(&img, &decode(&cs).unwrap());
        assert!(q > last, "{limit}: {q} dB after {last}");
        last = q;
    }
    // a lossless stream truncated to a budget still decodes
    let opts = EncodeOptions { max_bytes: Some(4_000), ..EncodeOptions::lossless() };
    let cs = encode(&img, &opts).unwrap();
    assert!(cs.len() <= 4_000);
    assert!(psnr(&img, &decode(&cs).unwrap()) > 30.0);
    // below the headers there is nothing to do
    let opts = EncodeOptions { max_bytes: Some(50), ..EncodeOptions::lossless() };
    assert!(encode(&img, &opts).is_err());
}

#[test]
fn component_limits_hold_per_component() {
    let mut img = picture(96, 64, &[(12, false, 1, 1); 3], 9);
    // a noisy first component wants many more bytes than the others
    let mut seed = 99u32;
    for v in &mut img.components[0].data {
        *v = (rnd(&mut seed) % 4096) as i32;
    }
    let opts = EncodeOptions { max_component_bytes: Some(3_000), tile_part_per_component: true, progression: Progression::Cprl, ..EncodeOptions::lossy(0.5) };
    let cs = encode(&img, &opts).unwrap();
    // the tile-parts are the components
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|&l| l <= 3_000), "{parts:?}");
    assert!(decode(&cs).is_ok());
}

/// Psot of each tile-part.
fn tile_parts(cs: &[u8]) -> Vec<usize> {
    let mut at = cs.windows(2).position(|w| w == [0xFF, 0x90]).unwrap();
    let mut out = Vec::new();
    while cs[at..].starts_with(&[0xFF, 0x90]) {
        let psot = u32::from_be_bytes(cs[at + 6..at + 10].try_into().unwrap()) as usize;
        out.push(psot);
        at += psot;
    }
    assert_eq!(&cs[at..], [0xFF, 0xD9]);
    out
}

/// The TLM lengths of a codestream.
fn tlm(cs: &[u8]) -> Vec<usize> {
    let at = cs.windows(2).position(|w| w == [0xFF, 0x55]).unwrap();
    let len = usize::from(u16::from_be_bytes([cs[at + 2], cs[at + 3]]));
    assert_eq!(cs[at + 5], 0x50);
    cs[at + 6..at + 2 + len].chunks(5).map(|e| u32::from_be_bytes(e[1..5].try_into().unwrap()) as usize).collect()
}

#[test]
fn cinema_2k_follows_the_profile() {
    let img = picture(256, 136, &[(12, false, 1, 1); 3], 21);
    let opts = EncodeOptions::cinema_2k(24);
    assert_eq!((opts.max_bytes, opts.max_component_bytes), (Some(1_302_083), Some(1_041_666)));
    assert_eq!(EncodeOptions::cinema_2k(48).max_bytes, Some(651_041));
    assert_eq!(EncodeOptions::cinema_4k(48).max_component_bytes, Some(520_833));
    let cs = encode(&img, &opts).unwrap();
    assert_eq!(&cs[..4], [0xFF, 0x4F, 0xFF, 0x51]);
    assert_eq!(&cs[6..8], [0, 3], "Rsiz");
    let h = crate::parse_header(&cs).unwrap();
    assert_eq!((h.levels, h.layers, h.reversible, h.progression), (5, 1, false, 4));
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 3);
    assert_eq!(tlm(&cs), parts);
    let back = decode(&cs).unwrap();
    assert!(psnr(&img, &back) > 50.0, "{}", psnr(&img, &back));
    // a tight budget per frame and per component
    let opts = EncodeOptions { max_bytes: Some(9_000), max_component_bytes: Some(3_500), ..EncodeOptions::cinema_2k(24) };
    let cs = encode(&img, &opts).unwrap();
    assert!(cs.len() <= 9_000);
    assert!(tile_parts(&cs).iter().all(|&l| l <= 3_500));
    // what the profile does not allow
    assert!(encode(&picture(64, 64, &[(8, false, 1, 1); 3], 1), &EncodeOptions::cinema_2k(24)).is_err());
    assert!(encode(&picture(2050, 16, &[(12, false, 1, 1); 3], 1), &EncodeOptions::cinema_2k(24)).is_err());
}

#[test]
fn cinema_4k_puts_the_4k_level_in_its_own_tile_parts() {
    let img = picture(320, 180, &[(12, false, 1, 1); 3], 31);
    let cs = encode(&img, &EncodeOptions::cinema_4k(24)).unwrap();
    assert_eq!(&cs[6..8], [0, 4], "Rsiz");
    assert!(cs.windows(2).any(|w| w == [0xFF, 0x5F]), "POC");
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 6);
    assert_eq!(tlm(&cs), parts);
    assert!(psnr(&img, &decode(&cs).unwrap()) > 50.0);
}

#[test]
fn bad_input_is_an_error() {
    let img = picture(16, 16, &[(8, false, 1, 1)], 1);
    let mut short = img.clone();
    short.components[0].data.pop();
    assert!(encode(&short, &EncodeOptions::lossless()).is_err());
    let mut wrong = img.clone();
    wrong.components[0].width = 15;
    assert!(encode(&wrong, &EncodeOptions::lossless()).is_err());
    assert!(encode(&img, &EncodeOptions { code_block: (11, 1), ..EncodeOptions::lossless() }).is_err());
    assert!(encode(&img, &EncodeOptions { levels: 40, ..EncodeOptions::lossless() }).is_err());
    assert!(encode(&img, &EncodeOptions { precincts: vec![(5, 5), (0, 4)], ..EncodeOptions::lossless() }).is_err());
    assert!(encode(&Image { width: 0, height: 0, components: Vec::new(), mct: false }, &EncodeOptions::lossless()).is_err());
    // out-of-range samples are clamped, not trusted
    let mut hot = img;
    hot.components[0].data[0] = i32::MAX;
    hot.components[0].data[1] = i32::MIN;
    let back = decode(&encode(&hot, &EncodeOptions::lossless()).unwrap()).unwrap();
    assert_eq!(&back.components[0].data[..2], [255, 0]);
}

#[test]
fn step_sizes_round_trip_through_the_header_fields() {
    for (delta, rb) in [(1.0, 8), (0.031_25, 12), (3.7, 14), (1e-12, 8), (1e9, 8)] {
        let (e, mu) = expounded(delta, rb);
        let back = 2f64.powi(rb - i32::from(e)) * (1.0 + f64::from(mu) / 2048.0);
        if (1e-6..1e6).contains(&delta) {
            assert!((back / delta - 1.0).abs() < 1.0 / 2048.0, "{delta}: {back}");
        }
        assert!(e <= 31 && mu < 2048);
    }
}

/// Random images and options: an error or a stream our decoder reads (exactly when lossless and
/// unlimited), never a panic.
#[test]
fn random_options_never_panic() {
    let mut seed = 0x5EED_u32;
    let progressions = [Progression::Lrcp, Progression::Rlcp, Progression::Rpcl, Progression::Pcrl, Progression::Cprl];
    for _ in 0..60 {
        let mut r = |n: u32| rnd(&mut seed) % n.max(1);
        let (w, h) = (1 + r(40), 1 + r(40));
        let comps: Vec<(u8, bool, u8, u8)> = (0..1 + r(4)).map(|_| (1 + r(16) as u8, r(2) == 1, 1 + r(2) as u8, 1 + r(2) as u8)).collect();
        let img = picture(w, h, &comps, r(1000));
        let reversible = r(2) == 0;
        let limited = r(3) == 0;
        let opts = EncodeOptions {
            reversible,
            levels: r(9) as u8,
            progression: progressions[r(5) as usize],
            code_block: (2 + r(5) as u8, 2 + r(5) as u8),
            precincts: (0..r(4)).map(|_| (1 + r(6) as u8, 1 + r(6) as u8)).collect(),
            mct: r(2) == 0,
            step: [0.0, 0.3, 4.0, 1e9][r(4) as usize],
            max_bytes: limited.then(|| 100 + r(3000) as usize),
            max_component_bytes: (r(3) == 0).then(|| 50 + r(1000) as usize),
            tile_part_per_component: r(2) == 0,
            tlm: r(2) == 0,
            profile: Profile::Part1,
        };
        let Ok(cs) = encode(&img, &opts) else { continue };
        let back = decode(&cs).unwrap();
        if reversible && opts.max_bytes.is_none() && opts.max_component_bytes.is_none() {
            assert_eq!(back, img, "{opts:?}");
        }
        if let Some(m) = opts.max_bytes {
            assert!(cs.len() <= m);
        }
    }
}
