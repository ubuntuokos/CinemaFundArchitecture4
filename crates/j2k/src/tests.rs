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

/// `data` rewritten with its packet headers packed into PPM (main header) or PPT (tile-part
/// headers) markers, one tile-part per tile.
fn packed_headers(data: &[u8], ppm: bool) -> Vec<u8> {
    let spans = crate::decode::packet_spans(data).unwrap_or_else(|e| panic!("{e}"));
    // the main header ends at the first SOT
    let mut at = 2;
    while at + 4 <= data.len() && u16::from_be_bytes([data[at], data[at + 1]]) != 0xFF90 {
        at += 2 + usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
    }
    let mut out = data[..at].to_vec();
    let segment = |out: &mut Vec<u8>, marker: u8, z: u8, body: &[u8]| {
        out.extend([0xFF, marker]);
        out.extend(((body.len() + 3) as u16).to_be_bytes());
        out.push(z);
        out.extend_from_slice(body);
    };
    let (mut heads, mut bodies) = (Vec::new(), Vec::new());
    for (_, tile, packets) in &spans {
        let (mut h, mut b) = (Vec::new(), Vec::new());
        for &(hs, he, be) in packets {
            h.extend_from_slice(&tile[hs..he]);
            b.extend_from_slice(&tile[he..be]);
        }
        heads.push(h);
        bodies.push(b);
    }
    if ppm {
        let mut stream = Vec::new();
        for h in &heads {
            stream.extend((h.len() as u32).to_be_bytes());
            stream.extend_from_slice(h);
        }
        for (z, chunk) in stream.chunks(1000).enumerate() {
            segment(&mut out, 0x60, z as u8, chunk);
        }
    }
    for (((index, _, _), h), b) in spans.iter().zip(&heads).zip(&bodies) {
        let mut tp = Vec::new();
        if !ppm {
            for (z, chunk) in h.chunks(700).enumerate() {
                segment(&mut tp, 0x61, z as u8, chunk);
            }
        }
        let psot = 12 + tp.len() + 2 + b.len();
        out.extend([0xFF, 0x90, 0, 10]);
        out.extend((*index as u16).to_be_bytes());
        out.extend((psot as u32).to_be_bytes());
        out.extend([0, 1]);
        out.extend(tp);
        out.extend([0xFF, 0x93]);
        out.extend_from_slice(b);
    }
    out.extend([0xFF, 0xD9]);
    out
}

/// Packet headers packed into PPM or PPT markers decode to the same picture as in the packets
/// (ffmpeg writes no packed headers; the streams are rewritten from its output).
#[test]
fn packed_packet_headers_decode_like_unpacked_ones() {
    let Some(ff) = cfa4_testkit::ffmpeg() else { return };
    let dir = cfa4_testkit::fixtures_dir("j2k");
    let _ = std::fs::create_dir_all(&dir);
    let f = dir.join("packed_src.j2k");
    let st = std::process::Command::new(ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=150x100", "-frames:v", "1", "-pix_fmt", "rgb24"])
        .args(["-c:v", "jpeg2000", "-pred", "dwt53", "-tile_width", "64", "-tile_height", "64", "-layer_rates", "40,10,1", "-format", "j2k"])
        .arg(&f)
        .stdin(std::process::Stdio::null())
        .status();
    assert!(st.is_ok_and(|s| s.success()), "ffmpeg");
    let data = std::fs::read(&f).unwrap_or_default();
    let want = decode(&data).unwrap_or_else(|e| panic!("{e}"));
    for ppm in [false, true] {
        let packed = packed_headers(&data, ppm);
        assert_ne!(packed, data);
        let got = decode(&packed).unwrap_or_else(|e| panic!("ppm {ppm}: {e}"));
        assert!(got == want, "ppm {ppm}: decodes differently");
        // damaged packed headers are errors or pictures, never a panic
        for cut in (2..packed.len()).step_by(97) {
            let _ = decode(&packed[..cut]);
            let mut bad = packed.clone();
            if let Some(b) = bad.get_mut(cut) {
                *b ^= 0x5A;
            }
            let _ = decode(&bad);
        }
    }
}
