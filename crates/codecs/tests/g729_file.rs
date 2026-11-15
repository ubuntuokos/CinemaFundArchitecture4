//! A raw `.g729` file read through the media source: the same samples as `cfa4_g729::decode_file`,
//! read front to back and from the middle.

#[test]
fn raw_g729_file_reads_through_the_media_source() {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let data: Vec<u8> = (0..10 * 400)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let want: Vec<f32> = cfa4_g729::decode_file(&data).unwrap().into_iter().map(|s| f32::from(s) / 32768.0).collect();
    let s = cfa4_codecs::open_bytes("speech.g729", data.into()).unwrap();
    let info = s.info().audio().unwrap();
    assert_eq!((info.sample_rate, info.channels, info.codec.as_str()), (8000, 1, "G.729"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 8000).unwrap().channels[0].clone();
    assert_eq!(read(0, want.len()), want);
    assert_eq!(read(12_345, 4000), want[12_345..16_345]);
}
