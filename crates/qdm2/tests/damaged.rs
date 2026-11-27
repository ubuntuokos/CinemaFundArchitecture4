//! Damaged input: the decoder returns an error or samples, never panics.

use cfa4_qdm2::Decoder;

fn extradata(channels: u32, fft_size: u32, block: u32) -> Vec<u8> {
    let mut e = Vec::new();
    e.extend_from_slice(&12u32.to_be_bytes());
    e.extend_from_slice(b"frmaQDM2");
    e.extend_from_slice(&44u32.to_be_bytes());
    e.extend_from_slice(b"QDCA");
    for v in [1, channels, 44100, 48000, fft_size * 16, fft_size, block] {
        e.extend_from_slice(&v.to_be_bytes());
    }
    e
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_blocks_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for (channels, fft, block) in [(1, 128, 512), (2, 256, 1024), (2, 512, 2048), (1, 256, 300)] {
        let Ok(mut dec) = Decoder::new(&extradata(channels, fft, block)) else { continue };
        for round in 0..200 {
            let len = if round % 7 == 0 { rng.next() as usize % 64 } else { block as usize };
            let mut packet: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            if round % 3 == 0 {
                packet.iter_mut().for_each(|b| *b &= 0x1f);
            }
            let _ = dec.decode(&packet);
        }
    }
}

#[test]
fn bad_configurations_are_refused() {
    assert!(Decoder::new(&[]).is_err());
    assert!(Decoder::new(&[0u8; 64]).is_err());
    assert!(Decoder::new(&extradata(0, 128, 512)).is_err());
    assert!(Decoder::new(&extradata(3, 128, 512)).is_err());
    assert!(Decoder::new(&extradata(1, 100, 512)).is_err());
    let mut cut = extradata(1, 128, 512);
    cut.truncate(30);
    assert!(Decoder::new(&cut).is_err());
}
