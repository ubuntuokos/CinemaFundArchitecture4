//! `.adx` and `.hca` files read through the media source: the same samples as the crates'
//! `decode_file`, read front to back and from the middle.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 24) as u8
    }
}

/// A stereo ADX file of `groups` block groups with a modest random signal.
fn adx(groups: usize) -> Vec<u8> {
    let mut rng = Rng(5);
    let mut f = vec![0u8; 0x20];
    f[0] = 0x80;
    f[2..4].copy_from_slice(&0x1Cu16.to_be_bytes());
    f[4..8].copy_from_slice(&[3, 18, 4, 2]);
    f[8..12].copy_from_slice(&32000u32.to_be_bytes());
    f[16..18].copy_from_slice(&500u16.to_be_bytes());
    f[0x1A..].copy_from_slice(b"(c)CRI");
    for _ in 0..groups * 2 {
        f.extend_from_slice(&(u16::from(rng.next()) << 3).to_be_bytes());
        f.extend((0..16).map(|_| rng.next()));
    }
    f
}

/// A mono HCA file (version 2.0, `comp` chunk, 96 bands) of `blocks` random blocks.
fn hca(blocks: u32) -> Vec<u8> {
    let mut rng = Rng(9);
    let size = 1800u16;
    let mut h = b"HCA\0".to_vec();
    h.extend_from_slice(&[2, 0, 0, 0]);
    h.extend_from_slice(b"fmt\0\x01");
    h.extend_from_slice(&44100u32.to_be_bytes()[1..]);
    h.extend_from_slice(&blocks.to_be_bytes());
    h.extend_from_slice(&[0, 128, 0, 64]);
    h.extend_from_slice(b"comp");
    h.extend_from_slice(&size.to_be_bytes());
    h.extend_from_slice(&[1, 15, 1, 0, 96, 96, 0, 0, 0, 0]);
    h.extend_from_slice(b"pad\0\0\0");
    let len = h.len() as u16;
    h[6..8].copy_from_slice(&len.to_be_bytes());
    for _ in 0..blocks {
        h.extend_from_slice(&[0xFF, 0xFF]);
        h.extend((0..usize::from(size) - 2).map(|_| rng.next() & rng.next()));
    }
    h
}

#[test]
fn adx_file_reads_through_the_media_source() {
    let data = adx(300);
    let (info, planes) = cfa4_adx::decode_file(&data).unwrap();
    let want: Vec<Vec<f32>> = planes.iter().map(|p| p.iter().map(|&v| cfa4_adx::Header::to_f32(v)).collect()).collect();
    let s = cfa4_codecs::open_bytes("voice.adx", data.into()).unwrap();
    let a = s.info().audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (info.sample_rate, 2, "CRI ADX"));
    let read = |from: usize, n: usize, ch: usize| s.audio(from as i64, n, 32000).unwrap().channels[ch].clone();
    assert_eq!(read(0, want[0].len(), 0), want[0]);
    assert_eq!(read(4321, 2000, 1), want[1][4321..6321]);
}

#[test]
fn hca_file_reads_through_the_media_source() {
    let data = hca(6);
    let (info, planes, damaged) = cfa4_hca::decode_file(&data, None).unwrap();
    assert_eq!(damaged, 0);
    let s = cfa4_codecs::open_bytes("bgm.hca", data.into()).unwrap();
    let a = s.info().audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (info.sample_rate, 1, "CRI HCA"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 44100).unwrap().channels[0].clone();
    assert_eq!(read(0, planes[0].len()), planes[0]);
    assert_eq!(read(3000, 1500), planes[0][3000..4500]);
}

#[test]
fn encrypted_hca_is_refused_without_a_key() {
    let mut data = hca(2);
    // turn the pad chunk into a ciph chunk of type 56
    let at = data.windows(4).position(|w| w == b"pad\0").unwrap();
    data[at..at + 6].copy_from_slice(b"ciph\0\x38");
    assert!(cfa4_codecs::open_bytes("bgm.hca", data.into()).is_err());
}
