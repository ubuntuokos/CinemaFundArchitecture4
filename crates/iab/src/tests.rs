use crate::*;

/// A 1 kHz-ish tone, left-justified 24-bit.
fn tone(n: usize, freq: f64, amp: f64, rate: f64) -> Vec<i32> {
    (0..n).map(|i| (((i as f64) * freq * std::f64::consts::TAU / rate).sin() * amp * 8_388_607.0) as i32 * 256).collect()
}

fn pan(x: u16, y: u16, z: u16) -> Pan {
    Pan { gain: Gain::Unity, position: [x, y, z], snap: Snap::Off, zones: None, spread: Spread::None, decorrelation: Decorrelation::None }
}

/// A 7.1 Bed (assets 1-8), one moving Object (asset 9), at `rate`.
fn sample_frame(rate: FrameRate, sample_rate: u32, coding: Coding) -> Frame {
    let n = rate.samples(sample_rate == 96_000, 0);
    let ids = [0x00, 0x04, 0x02, 0x0D, 0x07, 0x08, 0x05, 0x09];
    let channels = ids.iter().enumerate().map(|(k, &id)| BedChannel { id, audio_id: k as u32 + 1, gain: Gain::Unity, decorrelation: None }).collect();
    let mut audio: Vec<AudioData> = (1..=9).map(|id| AudioData { id, coding, samples: tone(n, 100.0 * f64::from(id), 0.25, f64::from(sample_rate)) }).collect();
    audio[3].samples.iter_mut().for_each(|s| *s = (*s / 4) & !0xFF);
    let mut pans = vec![None; rate.sub_blocks()];
    pans[0] = Some(pan(32767, 32767, 0));
    pans[rate.sub_blocks() / 2] = Some(Pan { gain: Gain::Code(64), snap: Snap::On(Some(100)), spread: Spread::OneD(300), ..pan(65535, 65535, 65535) });
    Frame {
        version: 1,
        sample_rate,
        bit_depth: 24,
        frame_rate: rate,
        frame_index: 0,
        max_rendered: 9,
        beds: vec![Bed {
            meta_id: 1,
            use_case: None,
            channels,
            description: Description { bits: 0x81, text: Some("main bed".into()) },
            beds: Vec::new(),
            remaps: Vec::new(),
        }],
        objects: vec![Object {
            meta_id: 2,
            audio_id: 9,
            use_case: None,
            pan: pans,
            description: Description { bits: 0x02, text: None },
            objects: Vec::new(),
            zones19: None,
        }],
        audio,
        authoring_tool: Some("https://example.org/cfa4".into()),
        user_data: vec![UserData { user_id: [7; 16], bytes: vec![1, 2, 3] }],
    }
}

#[test]
fn plex_coding_follows_the_examples() {
    // 5.2 examples 1, 3 and 4: Plex(8) of 0x1234, 0x12345678 and 0xFF
    let mut f = sample_frame(FrameRate::R24, 48_000, Coding::Pcm);
    f.max_rendered = 0x1234;
    let b = write(&f).unwrap();
    // IAFrame element: id 0x08, size Plex(8), version, rate / depth / frame rate, MaxRendered
    let at = 10 + 1;
    let size_len = if b[at] == 0xFF { if b[at + 1] == 0xFF && b[at + 2] == 0xFF { 7 } else { 3 } } else { 1 };
    let mr = at + size_len + 2;
    assert_eq!(b[mr..mr + 3], [0xFF, 0x12, 0x34]);
    for v in [0xFF, 0x1234, 0x1234_5678, 0xFFFF_FFFE] {
        f.max_rendered = v;
        assert_eq!(parse(&write(&f).unwrap()).unwrap().max_rendered, v);
    }
    f.max_rendered = 0xFFFF_FFFF;
    assert!(write(&f).is_err());
}

#[test]
fn pcm_and_dlc_frames_round_trip() {
    for (rate, sr) in [(FrameRate::R24, 48_000), (FrameRate::R25, 96_000), (FrameRate::R48, 48_000), (FrameRate::R100, 96_000)] {
        for coding in [Coding::Pcm, Coding::Dlc] {
            let f = sample_frame(rate, sr, coding);
            let bytes = write(&f).unwrap();
            let g = parse(&bytes).unwrap();
            assert_eq!(g, f, "{rate:?} {sr} {coding:?}");
            assert_eq!(frame_size(&bytes), Some(bytes.len()));
        }
    }
}

#[test]
fn dlc_is_smaller_than_pcm_for_tones() {
    let pcm = write(&sample_frame(FrameRate::R24, 48_000, Coding::Pcm)).unwrap().len();
    let dlc = write(&sample_frame(FrameRate::R24, 48_000, Coding::Dlc)).unwrap().len();
    assert!(dlc * 4 < pcm * 3, "DLC {dlc} bytes, PCM {pcm}");
}

#[test]
fn the_1001_rates_follow_the_frame_sequence() {
    // 29.97: five frames of 1602 1601 1602 1601 1602 samples (8008 = 5 × 48000 × 1001 / 30000)
    let total: usize = (0..5).map(|i| FrameRate::R29_97.samples(false, i)).sum();
    assert_eq!(total, 8008);
    assert_eq!((0..5).map(|i| FrameRate::R59_94.samples(true, i)).sum::<usize>(), 8008);
    for rate in FrameRate::ALL {
        for r96 in [false, true] {
            for i in 0..5 {
                let sizes = rate.sub_block_sizes(r96, i);
                assert_eq!(sizes.len(), rate.sub_blocks());
                assert_eq!(sizes.iter().sum::<usize>(), rate.samples(r96, i), "{rate:?} 96k {r96} index {i}");
            }
        }
    }
    let mut f = sample_frame(FrameRate::R29_97, 48_000, Coding::Pcm);
    for i in 0..5u8 {
        let n = FrameRate::R29_97.samples(false, i);
        f.frame_index = i;
        for a in &mut f.audio {
            a.samples = tone(n, 440.0, 0.1, 48_000.0);
        }
        let g = parse(&write(&f).unwrap()).unwrap();
        assert_eq!(g.frame_index, i);
        assert_eq!(g.samples(), n);
        assert_eq!(g, f);
    }
    // DLC cannot be carried at a 1001 rate
    let mut f = sample_frame(FrameRate::R23_976, 48_000, Coding::Dlc);
    assert!(write(&f).is_err());
    f.audio.iter_mut().for_each(|a| a.coding = Coding::Pcm);
    assert_eq!(parse(&write(&f).unwrap()).unwrap(), f);
}

#[test]
fn split_finds_each_frame_of_a_stream() {
    let a = write(&sample_frame(FrameRate::R48, 48_000, Coding::Dlc)).unwrap();
    let b = write(&sample_frame(FrameRate::R48, 48_000, Coding::Pcm)).unwrap();
    let mut s = a.clone();
    s.extend_from_slice(&b);
    s.extend_from_slice(&a[..20]);
    let frames = split(&s);
    assert_eq!(frames, vec![&a[..], &b[..]]);
    // a preamble with content is skipped
    let mut p = vec![0x01, 0, 0, 0, 3, 9, 9, 9];
    p.extend_from_slice(&a[5..]);
    assert_eq!(parse(&p).unwrap(), parse(&a).unwrap());
    // a bare IAFrame element parses too
    assert_eq!(parse(&a[10..]).unwrap(), parse(&a).unwrap());
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn a_bed_plays_on_its_own_loudspeakers() {
    let mut f = sample_frame(FrameRate::R24, 48_000, Coding::Pcm);
    f.objects.clear();
    let out = render(&f, &Layout::surround_7_1()).unwrap();
    assert_eq!(out.len(), 8);
    // the layout's order is L R C LFE Lrs Rrs Lss Rss, the Bed's L R C LFE Lrs Rrs Lss Rss
    for (ch, o) in out.iter().enumerate() {
        let want: Vec<f32> = f.audio[ch].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
        let err = o.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-6, "channel {ch}: {err}");
    }
    // on 5.1 the side and back surrounds fold into Ls / Rs and the LFE stays
    let out = render(&f, &Layout::surround_5_1()).unwrap();
    assert_eq!(out.len(), 6);
    assert!(rms(&out[4]) > 0.1 && rms(&out[5]) > 0.1);
    assert!((rms(&out[3]) - rms(&f.audio[3].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect::<Vec<_>>())).abs() < 1e-6);
    // stereo has no LFE: it is dropped, everything else lands on L / R
    let out = render(&f, &Layout::stereo()).unwrap();
    assert_eq!(out.len(), 2);
}

#[test]
fn conditional_beds_and_remaps_follow_the_use_case() {
    let n = FrameRate::R24.samples(false, 0);
    let one = |id: u32, audio_id: u32| BedChannel { id, audio_id, gain: Gain::Unity, decorrelation: None };
    let block = RemapBlock { destinations: vec![0x00, 0x04], gains: vec![vec![Gain::Unity, Gain::Silent], vec![Gain::Silent, Gain::Code(64)]] };
    let mut f = sample_frame(FrameRate::R24, 48_000, Coding::Pcm);
    f.objects.clear();
    f.beds = vec![Bed {
        meta_id: 1,
        use_case: None,
        // a "9.1" Bed of two channels for brevity: Lss and Rss
        channels: vec![one(0x05, 1), one(0x09, 2)],
        description: Description::default(),
        beds: vec![Bed {
            meta_id: 2,
            use_case: Some(use_case::SURROUND_5_1),
            channels: vec![one(0x06, 3), one(0x0A, 4)],
            description: Description::default(),
            beds: Vec::new(),
            remaps: Vec::new(),
        }],
        remaps: vec![Remap {
            meta_id: 3,
            use_case: use_case::ITU_A,
            sources: 2,
            destinations: 2,
            blocks: vec![Some(block), None, None, None, None, None, None, None],
        }],
    }];
    f.audio.truncate(4);
    assert_eq!(parse(&write(&f).unwrap()).unwrap(), f);
    let level = |x: &[f32]| rms(x);
    // 7.1: the unconditional Bed, Lss / Rss on their own loudspeakers
    let o = render(&f, &Layout::surround_7_1()).unwrap();
    assert!(level(&o[6]) > 0.1 && level(&o[7]) > 0.1 && level(&o[4]) == 0.0);
    // 5.1: the child Bed replaces it (assets 3 and 4 on Ls / Rs)
    let o = render(&f, &Layout::surround_5_1()).unwrap();
    let a3: Vec<f32> = f.audio[2].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
    assert!(o[4].iter().zip(&a3).all(|(a, b)| (a - b).abs() < 1e-6));
    // stereo: the remap sends Lss to L at unity and Rss to R at -6 dB
    let o = render(&f, &Layout::stereo()).unwrap();
    let a1: Vec<f32> = f.audio[0].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
    let a2: Vec<f32> = f.audio[1].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
    assert!(o[0].iter().zip(&a1).all(|(a, b)| (a - b).abs() < 1e-6));
    assert!(o[1].iter().zip(&a2).all(|(a, b)| (a - b * 0.5).abs() < 1e-6));
    assert_eq!(o[0].len(), n);
}

#[test]
fn objects_pan_by_position_with_snap_and_zones() {
    let n = FrameRate::R48.samples(false, 0);
    let mut f = sample_frame(FrameRate::R48, 48_000, Coding::Pcm);
    f.beds.clear();
    let layout = Layout::surround_7_1_4();
    let at = |p: Pan| {
        let mut f = f.clone();
        f.objects[0].pan = vec![Some(p), None, None, None];
        render(&f, &layout).unwrap().iter().map(|c| rms(c)).collect::<Vec<f32>>()
    };
    // front left corner, snapped: only L
    let g = at(Pan { snap: Snap::On(None), ..pan(32767, 32767, 0) });
    assert!(g[0] > 0.1 && g.iter().skip(1).all(|&v| v == 0.0), "{g:?}");
    // centre of the screen wall: mostly C
    let g = at(pan(49151, 32767, 0));
    let c = g[2];
    assert!(g.iter().enumerate().all(|(i, &v)| i == 2 || v < c * 0.3), "{g:?}");
    // overhead centre: the four tops share it equally, ahead of the floor loudspeakers
    let g = at(pan(49151, 49151, 65535));
    assert!((g[8] - g[11]).abs() < 1e-4 && g[8] > g[0] * 2.0, "{g:?}");
    // constant power
    let total: f32 = g.iter().map(|v| v * v).sum();
    let level = rms(&f.audio[8].samples.iter().map(|&s| s as f32 / 2_147_483_648.0).collect::<Vec<_>>());
    assert!((total.sqrt() - level).abs() < level * 0.01, "{total} {level}");
    // zone gains: the screen-left zone off moves a left-corner object off L
    let mut zones = [ZoneGain::Unity; 9];
    zones[0] = ZoneGain::Zero;
    let g = at(Pan { zones: Some(zones), ..pan(32767, 32767, 0) });
    assert_eq!(g[0], 0.0);
    assert!(g[6] > 0.0);
    // the LFE never carries objects
    assert_eq!(at(pan(49151, 32767, 0))[3], 0.0);
    assert_eq!(n, 1000);
}

#[test]
fn gains_ramp_between_sub_blocks() {
    let mut f = sample_frame(FrameRate::R96, 48_000, Coding::Pcm);
    f.beds.clear();
    // a constant signal, full left in the first sub-block, full right in the second
    f.audio[8].samples = vec![1 << 30; 500];
    f.objects[0].pan = vec![Some(Pan { snap: Snap::On(None), ..pan(32767, 32767, 0) }), Some(Pan { snap: Snap::On(None), ..pan(65535, 32767, 0) })];
    let o = render(&f, &Layout::stereo()).unwrap();
    assert!((o[0][100] - 0.5).abs() < 1e-6 && o[1][100] == 0.0);
    // halfway through the second sub-block the gains have crossed half way
    assert!((o[0][375] - 0.25).abs() < 0.01 && (o[1][375] - 0.25).abs() < 0.01, "{} {}", o[0][375], o[1][375]);
    assert!(o[0][499].abs() < 1e-6 && (o[1][499] - 0.5).abs() < 1e-6);
}

#[test]
fn malformed_frames_are_errors_never_panics() {
    let good = write(&sample_frame(FrameRate::R25, 96_000, Coding::Dlc)).unwrap();
    let pcm = write(&sample_frame(FrameRate::R24, 48_000, Coding::Pcm)).unwrap();
    for data in [&good, &pcm] {
        for cut in (0..data.len()).step_by(37) {
            let _ = parse(&data[..cut]);
        }
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..300 {
            let mut d = data.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed % d.len() as u64) as usize;
                d[i] ^= 1 << (seed >> 61);
            }
            if let Ok(f) = parse(&d) {
                for l in [Layout::stereo(), Layout::surround_7_1_4()] {
                    let _ = render(&f, &l);
                }
            }
        }
    }
    assert_eq!(parse(&[]), Err(Error::Truncated));
    assert_eq!(parse(&[0x40, 0x00]), Err(Error::Invalid("not an IAFrame element")));
    // nesting deeper than any real stream
    let mut b = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    for _ in 0..20 {
        let mut e = vec![0x10, b.len() as u8];
        e.extend_from_slice(&b);
        b = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        b.extend_from_slice(&e);
        if b.len() > 250 {
            break;
        }
    }
    let mut frame = vec![0x01, 0x00, 0x00, 0x08, 0x00, 0x01];
    frame.extend_from_slice(&b);
    let mut el = vec![0x08, frame.len() as u8];
    el.extend_from_slice(&frame);
    let _ = parse(&el);
}
