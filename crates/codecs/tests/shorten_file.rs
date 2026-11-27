//! A `.shn` file read through the media source: the same samples as `cfa4_shorten::decode_file`,
//! read front to back and from the middle.

/// A version 2 mono 16-bit Shorten stream of `samples`, every block coded with the `DIFF1`
/// predictor (a minimal writer; `crates/shorten/tests/oracle.rs` has the full one).
fn shn(samples: &[i16]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let put = |bits: &mut Vec<bool>, v: u32, n: u32| bits.extend((0..n).rev().map(|i| (v >> i) & 1 == 1));
    let ur = |bits: &mut Vec<bool>, k: u32, v: u32| {
        bits.extend(std::iter::repeat_n(false, (v >> k) as usize));
        bits.push(true);
        put(bits, v & ((1u32 << k) - 1), k);
    };
    let uint = |bits: &mut Vec<bool>, v: u32| {
        let k = (32 - v.leading_zeros()).saturating_sub(2);
        ur(bits, 2, k);
        ur(bits, k, v);
    };
    for b in *b"ajkg" {
        put(&mut bits, u32::from(b), 8);
    }
    put(&mut bits, 2, 8);
    for v in [5, 1, 256, 0, 0, 0] {
        uint(&mut bits, v); // type s16le, channels, block size, LPC order, means, skip
    }
    let mut header = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0".to_vec();
    header.extend_from_slice(&44100u32.to_le_bytes());
    header.extend_from_slice(&88200u32.to_le_bytes());
    header.extend_from_slice(b"\x02\0\x10\0data\0\0\0\0");
    ur(&mut bits, 2, 9);
    ur(&mut bits, 5, header.len() as u32);
    for b in header {
        ur(&mut bits, 8, u32::from(b));
    }
    let mut prev = 0i32;
    for block in samples.chunks(256) {
        if block.len() < 256 {
            ur(&mut bits, 2, 5);
            uint(&mut bits, block.len() as u32);
        }
        ur(&mut bits, 2, 1); // DIFF1
        ur(&mut bits, 3, 12);
        for &s in block {
            let r = i32::from(s) - prev;
            prev = i32::from(s);
            ur(&mut bits, 13, ((r << 1) ^ (r >> 31)) as u32);
        }
    }
    ur(&mut bits, 2, 4);
    bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (u8::from(b) << (7 - i)))).collect()
}

#[test]
fn shn_file_reads_through_the_media_source() {
    let pcm: Vec<i16> = (0..20_000).map(|i| ((f64::from(i) * 0.031).sin() * 12000.0 + f64::from(i % 97) * 30.0) as i16).collect();
    let data = shn(&pcm);
    let (info, planes) = cfa4_shorten::decode_file(&data).unwrap();
    assert_eq!(planes[0], pcm.iter().map(|&s| i32::from(s)).collect::<Vec<_>>());
    let want: Vec<f32> = planes[0].iter().map(|&v| info.to_f32(v)).collect();
    let s = cfa4_codecs::open_bytes("music.shn", data.into()).unwrap();
    let a = s.info().audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (44100, 1, "Shorten"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 44100).unwrap().channels[0].clone();
    assert_eq!(read(0, want.len()), want);
    assert_eq!(read(12_345, 4000), want[12_345..16_345]);
}
