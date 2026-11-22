//! An Interplay ACM file read through the media source: the same samples as
//! `cfa4_acm::decode_file`, read front to back and from the middle.

/// A mono 22.05 kHz ACM file of `blocks` blocks of 8 columns × 40 rows; every column is coded
/// as 4-bit linear values drawn from a small generator (a minimal writer; `crates/acm/tests/
/// oracle.rs` has the full one).
fn acm(blocks: usize) -> Vec<u8> {
    let (level, rows) = (3u32, 40usize);
    let total = (rows << level) * blocks;
    let mut out = vec![0x97, 0x28, 0x03, 0x01];
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&22050u16.to_le_bytes());
    out.extend_from_slice(&(level as u16 | (rows as u16) << 4).to_le_bytes());
    let (mut bytes, mut nbits) = (Vec::<u8>::new(), 0usize);
    let mut put = |v: u64, n: u32| {
        for i in 0..n {
            if nbits.is_multiple_of(8) {
                bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let l = bytes.len() - 1;
                bytes[l] |= 1 << (nbits % 8);
            }
            nbits += 1;
        }
    };
    let mut x = 12345u64;
    for _ in 0..blocks {
        put(4, 4); // 16 amplitudes
        put(300, 16);
        for _ in 0..1 << level {
            put(4, 5); // linear, 4 bits
            for _ in 0..rows {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                put(x >> 60, 4);
            }
        }
    }
    out.extend_from_slice(&bytes);
    out
}

#[test]
fn acm_file_reads_through_the_media_source() {
    let data = acm(30);
    let (h, pcm) = cfa4_acm::decode_file(&data).unwrap();
    assert_eq!(pcm.len() as u64, h.frames);
    assert!(pcm.iter().any(|&v| v != 0));
    let want: Vec<f32> = pcm.iter().map(|&v| f32::from(v) / 32768.0).collect();
    let s = cfa4_codecs::open_bytes("voice.acm", data.into()).unwrap();
    let a = s.info().audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (22050, 1, "Interplay ACM"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 22050).unwrap().channels[0].clone();
    assert_eq!(read(0, want.len()), want);
    assert_eq!(read(4_321, 3000), want[4_321..7_321]);
}
