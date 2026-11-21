//! A QCP (QCELP-13K) file read through the media source: the same samples as `cfa4_qcelp::decode_file`,
//! read front to back and from the middle.

#[test]
fn qcp_file_reads_through_the_media_source() {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x as u8
    };
    let mut data = b"RIFF\0\0\0\0QLCMfmt ".to_vec();
    data.extend_from_slice(&150u32.to_le_bytes());
    data.extend_from_slice(&[1, 0, 0x41, 0x6d, 0x7f, 0x5e, 0x15, 0xb1, 0xd0, 0x11, 0xba, 0x91, 0x00, 0x80, 0x5f, 0xb4, 0xb9, 0x7e, 0, 0]);
    data.extend_from_slice(&[b'Q'; 80]);
    for v in [13000u16, 0, 160, 8000, 16] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&5u32.to_le_bytes());
    for (mode, size) in [1u8, 4, 8, 17, 35].into_iter().enumerate() {
        data.extend_from_slice(&[size, mode as u8]);
    }
    data.resize(170, 0);
    let packets = 300usize;
    data.extend_from_slice(b"vrat");
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(packets as u32).to_le_bytes());
    let mut body = Vec::new();
    for _ in 0..packets {
        // full-rate packets with the rate byte; random bits are mostly concealed, which is fine here
        body.push(4);
        body.push(4);
        body.extend((0..34).map(|_| next()));
    }
    data.extend_from_slice(b"data");
    data.extend_from_slice(&(body.len() as u32).to_le_bytes());
    data.extend_from_slice(&body);

    let want = cfa4_qcelp::decode_file(&data).unwrap();
    assert_eq!(want.len(), packets * 160);
    let s = cfa4_codecs::open_bytes("voice.qcp", data.into()).unwrap();
    let info = s.info().audio().unwrap();
    assert_eq!((info.sample_rate, info.channels, info.codec.as_str()), (8000, 1, "QCELP"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 8000).unwrap().channels[0].clone();
    assert_eq!(read(0, want.len()), want);
    assert_eq!(read(12_345, 4000), want[12_345..16_345]);
}
