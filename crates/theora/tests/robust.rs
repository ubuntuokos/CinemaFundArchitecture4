//! Synthetic streams decode deterministically, and damaged headers and packets are errors, never
//! panics.

use cfa4_theora::synth::{Rng, comment, ident, packet, setup, setup_wide};
use cfa4_theora::{Chroma, Decoder, Error};

fn decoder(w: u32, h: u32, pf: u64, seed: u64) -> (Decoder, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut rng = Rng::new(seed);
    let (i, c, s) = (ident(w, h, pf), comment(), setup(&mut rng));
    (Decoder::new(&i, &c, &s).expect("headers"), i, c, s)
}

#[test]
fn synthetic_stream_decodes_to_the_visible_size() {
    for (w, h, pf, chroma) in [(64, 48, 0, Chroma::C420), (48, 32, 2, Chroma::C422), (48, 32, 3, Chroma::C444)] {
        let (mut d, ..) = decoder(w, h, pf, 7);
        let mut rng = Rng::new(99);
        let mut shown = 0;
        for k in 0..40 {
            if let Ok(p) = d.decode(&packet(&mut rng, k % 8 == 0)) {
                assert_eq!((p.width, p.height, p.chroma), (w, h, chroma));
                assert_eq!(p.planes[0].len(), (w * h) as usize);
                shown += 1;
            }
        }
        assert!(shown >= 5, "{w}x{h}: only {shown} frames decoded");
    }
}

#[test]
fn decoding_is_deterministic_and_reset_needs_a_key_frame() {
    let run = || {
        let (mut d, ..) = decoder(64, 48, 0, 3);
        let mut rng = Rng::new(5);
        (0..30).filter_map(|k| d.decode(&packet(&mut rng, k % 10 == 0)).ok()).map(|p| p.planes).collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
    let (mut d, ..) = decoder(64, 48, 0, 3);
    let mut rng = Rng::new(5);
    assert!(d.decode(&packet(&mut rng, true)).is_ok());
    d.reset();
    // an inter packet without a reference frame
    let mut inter = packet(&mut rng, false);
    inter.truncate(inter.len().min(200));
    assert!(d.decode(&inter).is_err());
    // an empty packet repeats the previous picture, or fails when there is none
    assert!(d.decode(&[]).is_err());
}

#[test]
fn damaged_headers_and_packets_never_panic() {
    let (_, i, c, s) = decoder(64, 48, 0, 11);
    let mut rng = Rng::new(1234);
    // every truncation of the headers, and random single-byte damage
    for n in 0..i.len() {
        assert!(Decoder::new(&i[..n], &c, &s).is_err());
    }
    for n in (0..s.len()).step_by(7) {
        assert!(Decoder::new(&i, &c, &s[..n]).is_err());
    }
    for _ in 0..400 {
        let (mut i2, mut s2) = (i.clone(), s.clone());
        let (a, b) = (rng.below(i2.len() as u64) as usize, rng.below(s2.len() as u64) as usize);
        i2[a] = rng.draw() as u8;
        s2[b] = rng.draw() as u8;
        if let Ok(mut d) = Decoder::new(&i2, &c, &s2) {
            for k in 0..3 {
                let _ = d.decode(&packet(&mut rng, k == 0));
            }
        }
    }
    // damaged and truncated packets of a good stream
    let (mut d, ..) = decoder(64, 48, 0, 11);
    for k in 0..300 {
        let mut p = packet(&mut rng, k % 5 == 0);
        let cut = rng.below(p.len() as u64 + 1) as usize;
        p.truncate(cut);
        for _ in 0..3 {
            let at = rng.below(p.len().max(1) as u64) as usize;
            if let Some(b) = p.get_mut(at) {
                *b = rng.draw() as u8;
            }
        }
        let _ = d.decode(&p);
    }
    // a header packet among the data, and a packet of garbage
    assert!(matches!(d.decode(&[0x80, 1, 2, 3]), Err(Error::Invalid(_))));
    let _ = d.decode(&[0u8; 10]);
}

#[test]
fn extreme_coefficients_never_panic() {
    // every token and large quantizers: the transform inputs overflow 16 bits
    let mut rng = Rng::new(77);
    for pf in [0, 2, 3] {
        let (i, c, s) = (ident(64, 48, pf), comment(), setup_wide(&mut rng));
        let mut d = Decoder::new(&i, &c, &s).expect("headers");
        let mut ok = 0;
        for k in 0..60 {
            if d.decode(&packet(&mut rng, k % 6 == 0)).is_ok() {
                ok += 1;
            }
        }
        assert!(ok > 0);
    }
}
