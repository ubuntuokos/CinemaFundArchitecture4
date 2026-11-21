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

/// The guard bits of the QCD marker segment.
fn qcd_guard(cs: &[u8]) -> u8 {
    let at = cs.windows(2).position(|w| w == [0xFF, 0x5C]).expect("QCD");
    cs[at + 4] >> 5
}

#[test]
fn cinema_2k_saturates_with_one_guard_bit() {
    // 12-bit X'Y'Z' pictures: grey, coloured, and the extreme of full-range steps (0 ↔ 4095)
    let (w, h) = (256u32, 136u32);
    let grey = {
        let mut img = picture(w, h, &[(12, false, 1, 1); 3], 5);
        let first = img.components[0].data.clone();
        for p in &mut img.components {
            p.data.clone_from(&first);
        }
        img
    };
    let colour = picture(w, h, &[(12, false, 1, 1); 3], 77);
    let extreme = {
        let mut img = picture(w, h, &[(12, false, 1, 1); 3], 9);
        for (c, p) in img.components.iter_mut().enumerate() {
            for (k, v) in p.data.iter_mut().enumerate() {
                let (x, y) = (k as u32 % w, k as u32 / w);
                // single-pixel lines, checkerboards and blocks, each component shifted
                let on = match (x / 64 + c as u32) % 3 {
                    0 => x % 2 == 0,
                    1 => (x + y) % 2 == 0,
                    _ => (x / 8 + y / 8) % 2 == 0,
                };
                *v = if on { 4095 } else { 0 };
            }
        }
        img
    };
    for (name, img) in [("grey", &grey), ("colour", &colour), ("extreme", &extreme)] {
        let (cs, stats) = encode_with_stats(img, &EncodeOptions::cinema_2k(24)).unwrap();
        assert_eq!(qcd_guard(&cs), 1, "{name}");
        let back = decode(&cs).unwrap();
        let q = psnr(img, &back);
        assert!(q >= 50.0, "{name}: {q:.2} dB, {} saturated", stats.saturated);
        // saturation is for rare coefficients beyond the nominal range only
        assert!(stats.saturated <= u64::from(w * h) * 3 / 1000, "{name}: {} saturated", stats.saturated);
    }
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
    assert_eq!((h.levels, h.layers, h.reversible, h.progression, h.mct), (5, 1, false, 4, true));
    assert_eq!(qcd_guard(&cs), 1, "2K Digital Cinema: 1 guard bit");
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 3);
    assert_eq!(tlm(&cs), parts);
    let back = decode(&cs).unwrap();
    assert!(psnr(&img, &back) > 50.0, "{}", psnr(&img, &back));
    // without the component transform the profile still holds (CFA4 DCPs before 2026-11)
    let no_mct = EncodeOptions { mct: false, ..EncodeOptions::cinema_2k(24) };
    let cs0 = encode(&img, &no_mct).unwrap();
    assert!(!crate::parse_header(&cs0).unwrap().mct);
    assert!(psnr(&img, &decode(&cs0).unwrap()) > 50.0);
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
    assert_eq!(qcd_guard(&cs), 2, "4K Digital Cinema: 2 guard bits");
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 6);
    assert_eq!(tlm(&cs), parts);
    assert!(psnr(&img, &decode(&cs).unwrap()) > 50.0);
}

#[test]
fn imf_profile_selection_follows_annex_e() {
    // ST 2067-21 Annex E example: UHD 4:2:2 at 23.976 is 397.7 MSamples/s (417.6 with the 5 %
    // headroom): mainlevel 5; 4K class because 3840 > 2048
    let p = ImfProfile::select(3840, 2160, 2, 24000, 1001, true, 0).unwrap();
    assert_eq!((p.size, p.mainlevel, p.sublevel), (ImfSize::FourK, 5, 0));
    assert_eq!(p.rsiz(), 0x0805);
    // HD 4:4:4 at 24 lossy, 250 Mbit/s: 149 MSamples/s (156.7) is mainlevel 3; 262.5 Mbit/s needs
    // sublevel 2, but mainlevel 3 stops at sublevel 1
    let p = ImfProfile::select(1920, 1080, 3, 24, 1, false, 250_000_000).unwrap();
    assert_eq!((p.size, p.mainlevel, p.sublevel), (ImfSize::TwoK, 3, 1));
    assert_eq!(p.rsiz(), 0x0413);
    // UHD 4:4:4 at 60 lossy 800 Mbit/s: 1492.9 MSamples/s → mainlevel 7, sublevel 4 (1600)
    let p = ImfProfile::select(3840, 2160, 3, 60, 1, false, 800_000_000).unwrap();
    assert_eq!((p.size, p.mainlevel, p.sublevel), (ImfSize::FourK, 7, 4));
    assert_eq!(ImfProfile::max_sublevel(4), 2);
    assert_eq!(ImfProfile::sublevel_bitrate(3), 800_000_000);
    assert!(ImfProfile::select(8193, 100, 3, 24, 1, true, 0).is_none());
    assert!(ImfProfile::select(8192, 6224, 3, 1000, 1, true, 0).is_none(), "beyond mainlevel 10");
    assert!(ImfProfile::select(64, 64, 3, 0, 0, true, 0).is_some(), "zero rates never divide by zero");
}

#[test]
fn imf_codestreams_follow_the_profile() {
    // reversible 4:4:4 10-bit: lossless, Rsiz with the levels, CPRL, 32 × 32, one tile-part per
    // component with a TLM
    let img = picture(256, 144, &[(10, false, 1, 1); 3], 41);
    let p = ImfProfile::select(256, 144, 3, 24, 1, true, 0).unwrap();
    let opts = EncodeOptions::imf(p, 256, 144, 24, 1);
    assert_eq!((opts.levels, opts.max_bytes), (5, None));
    let cs = encode(&img, &opts).unwrap();
    assert_eq!(&cs[6..8], p.rsiz().to_be_bytes(), "Rsiz");
    assert_eq!(p.rsiz() >> 8, 0x07);
    let h = crate::parse_header(&cs).unwrap();
    assert_eq!((h.levels, h.layers, h.reversible, h.progression), (5, 1, true, 4));
    let parts = tile_parts(&cs);
    assert_eq!(parts.len(), 3);
    assert_eq!(tlm(&cs), parts);
    assert_eq!(decode(&cs).unwrap().components.iter().map(|c| &c.data).collect::<Vec<_>>(), img.components.iter().map(|c| &c.data).collect::<Vec<_>>());
    // lossy 4:2:2 12-bit: 9/7 under the sublevel's byte limit per frame
    let img = picture(320, 180, &[(12, false, 1, 1), (12, false, 2, 1), (12, false, 2, 1)], 43);
    let p = ImfProfile::select(320, 180, 2, 25, 1, false, 100_000_000).unwrap();
    assert_eq!((p.mainlevel, p.sublevel), (1, 1));
    let opts = EncodeOptions::imf(p, 320, 180, 25, 1);
    assert_eq!(opts.max_bytes, Some(1_000_000));
    let cs = encode(&img, &opts).unwrap();
    assert_eq!(&cs[6..8], [0x04, 0x11]);
    assert!(psnr(&img, &decode(&cs).unwrap()) > 45.0);
    let tight = EncodeOptions { max_bytes: Some(4_000), ..opts.clone() };
    assert!(encode(&img, &tight).unwrap().len() <= 4_000);
    // what the profile does not allow
    let bad = |img: &Image, o: &EncodeOptions| encode(img, o).is_err();
    assert!(bad(&picture(64, 64, &[(12, true, 1, 1); 3], 1), &EncodeOptions::imf(p, 64, 64, 25, 1)), "signed");
    assert!(bad(&picture(64, 64, &[(12, false, 1, 1); 4], 1), &EncodeOptions::imf(p, 64, 64, 25, 1)), "4 components");
    assert!(bad(&picture(64, 64, &[(12, false, 1, 2); 3], 1), &EncodeOptions::imf(p, 64, 64, 25, 1)), "vertical sub-sampling");
    assert!(bad(&picture(2050, 16, &[(12, false, 1, 1); 3], 1), &EncodeOptions::imf(p, 2050, 16, 25, 1)), "wider than 2K");
    let img = picture(64, 64, &[(12, false, 1, 1); 3], 1);
    assert!(bad(&img, &EncodeOptions { code_block: (6, 6), ..EncodeOptions::imf(p, 64, 64, 25, 1) }));
    assert!(bad(&img, &EncodeOptions { progression: Progression::Lrcp, ..EncodeOptions::imf(p, 64, 64, 25, 1) }));
    assert!(bad(&img, &EncodeOptions { reversible: true, ..EncodeOptions::imf(p, 64, 64, 25, 1) }));
    assert!(bad(&img, &EncodeOptions::imf(ImfProfile { sublevel: 2, ..p }, 64, 64, 25, 1)), "sublevel above the mainlevel's");
    assert!(bad(&img, &EncodeOptions::imf(ImfProfile { sublevel: 0, ..p }, 64, 64, 25, 1)), "lossy sublevel 0");
    assert!(!bad(&img, &EncodeOptions::imf(p, 64, 64, 25, 1)));
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
            ht: r(3) == 0,
        };
        let Ok(cs) = encode(&img, &opts) else { continue };
        let back = decode(&cs).unwrap();
        if reversible && opts.max_bytes.is_none() && opts.max_component_bytes.is_none() {
            assert_eq!(back.components, img.components, "{opts:?}");
        }
        if let Some(m) = opts.max_bytes {
            assert!(cs.len() <= m);
        }
    }
}

/// Code-blocks that stop coding below the predicted threshold give the codestream full coding
/// gives, within its byte limits, at the same quality.
#[test]
fn early_termination_keeps_the_rate_control_result() {
    let mut noisy = picture(512, 272, &[(12, false, 1, 1); 3], 77);
    let mut seed = 7u32;
    for v in &mut noisy.components[1].data {
        *v = (*v + (rnd(&mut seed) % 512) as i32).min(4095);
    }
    let smooth = picture(512, 272, &[(12, false, 1, 1); 3], 78);
    for img in [&noisy, &smooth] {
        for (bytes, comp) in [(30_000, None), (60_000, Some(25_000)), (150_000, Some(40_000)), (2_000_000, None)] {
            let opts = EncodeOptions { max_bytes: Some(bytes), max_component_bytes: comp, ..EncodeOptions::cinema_2k(24) };
            let fast = encode_with(img, &opts, true).unwrap();
            let full = encode_with(img, &opts, false).unwrap();
            assert!(fast.len() <= bytes && tile_parts(&fast).iter().all(|&l| comp.is_none_or(|c| l <= c)));
            let (qf, qs) = (psnr(img, &decode(&fast).unwrap()), psnr(img, &decode(&full).unwrap()));
            assert!((qf - qs).abs() < 0.02, "{bytes} {comp:?}: {qf} dB, {qs} dB in full");
        }
    }
}

/// The galloping search finds what the plain bisection finds.
#[test]
fn refine_agrees_with_search() {
    let mut seed = 3u32;
    for _ in 0..500 {
        let n = (rnd(&mut seed) % 300) as usize;
        let cut = (rnd(&mut seed) % (n as u32 + 2)) as usize; // ok(Some(k)) for k < cut
        let ok = |k: Option<usize>| Ok(k.is_none_or(|k| k < cut));
        let guess = match rnd(&mut seed) % 3 {
            0 => None,
            _ => Some((rnd(&mut seed) % (n as u32 + 5)) as usize),
        };
        assert_eq!(refine(n, guess, ok).unwrap(), search(n, ok).unwrap(), "n {n} cut {cut} guess {guess:?}");
    }
}

#[test]
fn htj2k_round_trips_and_matches_the_part_1_coder() {
    let shapes: [&[(u8, bool, u8, u8)]; 5] =
        [&[(8, false, 1, 1); 3], &[(1, false, 1, 1)], &[(16, false, 1, 1), (16, false, 2, 1), (16, false, 2, 1)], &[(12, true, 1, 1)], &[(10, false, 1, 2); 4]];
    for (k, comps) in shapes.iter().enumerate() {
        let img = picture(77, 45, comps, 7 + k as u32);
        for cb in [(6, 6), (2, 2), (10, 2), (3, 7)] {
            let ht = EncodeOptions { ht: true, code_block: cb, ..EncodeOptions::lossless() };
            let back = decode(&encode(&img, &ht).unwrap()).unwrap();
            assert_eq!(back.components, img.components, "{comps:?} {cb:?}");
        }
        // with every bit-plane coded, both block coders give the same reconstruction
        let lossy = EncodeOptions::lossy(1.5);
        let part1 = decode(&encode(&img, &lossy).unwrap()).unwrap();
        let ht = decode(&encode(&img, &EncodeOptions { ht: true, ..lossy }).unwrap()).unwrap();
        assert_eq!(ht.components, part1.components, "{comps:?} lossy");
    }
}

#[test]
fn htj2k_header_and_limits() {
    let img = picture(64, 32, &[(8, false, 1, 1); 3], 3);
    let cs = encode(&img, &EncodeOptions { ht: true, ..EncodeOptions::lossless() }).unwrap();
    // SIZ Rsiz with bit 14, then CAP: Pcap with Part 15, Ccap15 reversible with Bp for B = 11
    assert_eq!(&cs[0..4], &[0xFF, 0x4F, 0xFF, 0x51]);
    assert_eq!(u16::from_be_bytes([cs[6], cs[7]]), 0x4000);
    let cap = 4 + usize::from(u16::from_be_bytes([cs[4], cs[5]]));
    assert_eq!(&cs[cap..cap + 10], &[0xFF, 0x50, 0, 8, 0, 2, 0, 0, 0, 3]);
    // COD code-block style: HT
    assert_eq!(&cs[cap + 10..cap + 12], &[0xFF, 0x52]);
    assert_eq!(cs[cap + 10 + 12], 0x40);
    let lossy = encode(&img, &EncodeOptions { ht: true, ..EncodeOptions::lossy(1.0) }).unwrap();
    assert_eq!(lossy[cap + 9] & 0x20, 0x20, "irreversible flag in Ccap15");
    let dci = EncodeOptions { ht: true, ..EncodeOptions::cinema_2k(24) };
    assert!(matches!(encode(&img, &dci), Err(Error::Unsupported(_))));
}

#[test]
fn htj2k_meets_byte_limits_close_to_the_part_1_coder() {
    let img = picture(160, 96, &[(10, false, 1, 1); 3], 11);
    let full = encode(&img, &EncodeOptions { ht: true, ..EncodeOptions::lossless() }).unwrap().len();
    let mut last = 0.0;
    for limit in [full / 40, full / 12, full / 4, full / 2] {
        for opts in [EncodeOptions::lossy(0.25), EncodeOptions::lossless()] {
            let ht = EncodeOptions { ht: true, max_bytes: Some(limit), ..opts.clone() };
            let cs = encode(&img, &ht).unwrap();
            assert!(cs.len() <= limit, "{} > {limit}", cs.len());
            let q_ht = psnr(&img, &decode(&cs).unwrap());
            let q_p1 = psnr(&img, &decode(&encode(&img, &EncodeOptions { max_bytes: Some(limit), ..opts.clone() }).unwrap()).unwrap());
            // whole bit-planes only (no fractional passes), so a little below Part 1
            assert!(q_ht > q_p1 - 3.0, "limit {limit} reversible {}: HT {q_ht:.2} dB, Part 1 {q_p1:.2} dB", opts.reversible);
            if !opts.reversible {
                assert!(q_ht >= last, "{q_ht} after {last}");
                last = q_ht;
            }
        }
    }
    // per-component limits too
    let opts = EncodeOptions { ht: true, max_component_bytes: Some(full / 10), ..EncodeOptions::lossy(0.25) };
    assert!(psnr(&img, &decode(&encode(&img, &opts).unwrap()).unwrap()) > 25.0);
    // a limit above the lossless size changes nothing
    let roomy = encode(&img, &EncodeOptions { ht: true, max_bytes: Some(full * 2), ..EncodeOptions::lossless() }).unwrap();
    assert_eq!(decode(&roomy).unwrap().components, img.components);
}
