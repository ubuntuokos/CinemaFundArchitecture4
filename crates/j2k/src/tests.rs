use super::*;

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(decode(&[]).is_err());
    assert!(decode(&[0xFF, 0x4F, 0xFF, 0x51, 0, 2]).is_err());
    assert!(decode(b"not an image").is_err());
}

/// A minimal one-tile codestream: SIZ with `comps` (precision 8, sub-sampling dx, dy), COD with
/// `levels`, code-block exponent bytes `cb`, `layers` and optional precinct bytes, a reversible
/// QCD, and an empty tile body.
fn codestream(w: u32, h: u32, comps: &[(u8, u8)], levels: u8, cb: (u8, u8), layers: u16, precincts: Option<u8>) -> Vec<u8> {
    let mut d = vec![0xFF, 0x4F];
    let m = |d: &mut Vec<u8>, marker: u8, body: Vec<u8>| {
        d.extend([0xFF, marker]);
        d.extend(((body.len() + 2) as u16).to_be_bytes());
        d.extend(body);
    };
    let mut siz = vec![0, 0];
    for v in [w, h, 0, 0, w, h, 0, 0] {
        siz.extend(v.to_be_bytes());
    }
    siz.extend((comps.len() as u16).to_be_bytes());
    for &(dx, dy) in comps {
        siz.extend([7, dx, dy]);
    }
    m(&mut d, 0x51, siz);
    let mut cod = vec![u8::from(precincts.is_some()), 0];
    cod.extend(layers.to_be_bytes());
    cod.extend([0, levels, cb.0, cb.1, 0, 1]);
    if let Some(p) = precincts {
        cod.extend(std::iter::repeat_n(p, usize::from(levels) + 1));
    }
    m(&mut d, 0x52, cod);
    let mut qcd = vec![0x40];
    qcd.extend(std::iter::repeat_n(8 << 3, 3 * usize::from(levels) + 1));
    m(&mut d, 0x5C, qcd);
    d.extend([0xFF, 0x90, 0, 10, 0, 0, 0, 0, 0, 14, 0, 1, 0xFF, 0x93, 0xFF, 0xD9]);
    d
}

#[test]
fn the_sample_limit_counts_each_components_rounded_size() {
    // 2^32 - 1 columns of a component sub-sampled 1 × 255: one row, 4 294 967 295 samples, which
    // dividing the area by 255 used to count as 16.8 million
    let d = codestream(u32::MAX, 1, &[(1, 255)], 0, (4, 4), 1, None);
    assert_eq!(parse_header(&d).err(), Some(Error::TooLarge));
    assert!(parse_header(&codestream(64, 64, &[(1, 1), (2, 2)], 0, (4, 4), 1, None)).is_ok());
}

#[test]
fn code_block_exponent_bytes_near_255_are_errors() {
    // the exponent byte plus two used to overflow a u8
    for cb in [(255, 4), (4, 254)] {
        assert!(matches!(parse_header(&codestream(64, 64, &[(1, 1)], 0, cb, 1, None)), Err(Error::Invalid(_))));
    }
}

#[test]
fn an_excessive_packet_count_is_refused_before_it_is_built() {
    // 64 × 64 one-sample precincts (4096) × 65 535 layers: 268 million packets, about 8 GiB of
    // packet order for an 80-byte stream
    let d = codestream(64, 64, &[(1, 1)], 0, (4, 4), u16::MAX, Some(0));
    assert_eq!(decode(&d).err(), Some(Error::TooLarge));
    // the same with one layer decodes (to an empty picture)
    assert!(decode(&codestream(64, 64, &[(1, 1)], 0, (4, 4), 1, Some(0))).is_ok());
}
