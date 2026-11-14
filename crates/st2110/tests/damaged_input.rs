//! Damaged-input fuzzing: network data is untrusted (AGENTS.md §0). Truncated, bit-flipped and
//! random datagrams and SDP texts must give errors, never panics, and never an image larger
//! than the format allows.

use cfa4_st2110::sdp::Session;
use cfa4_st2110::video::{Colorimetry, Depth, FrameRate, PackingMode, Sampling};
use cfa4_st2110::{Depacketizer, Packetizer, VideoFormat};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn formats() -> Vec<VideoFormat> {
    let r = FrameRate::new(50, 1).unwrap();
    let mut v = vec![
        VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, 64, 8, r, Colorimetry::Bt709),
        VideoFormat::progressive(Sampling::YCbCr420, Depth::D10, 64, 8, r, Colorimetry::Bt709),
        VideoFormat::progressive(Sampling::Rgb, Depth::D12, 33, 5, r, Colorimetry::Bt709),
    ];
    let mut i = VideoFormat::progressive(Sampling::YCbCr422, Depth::D8, 48, 7, r, Colorimetry::Bt709);
    i.interlace = true;
    v.push(i);
    let mut b = VideoFormat::progressive(Sampling::YCbCr422, Depth::D10, 720, 6, r, Colorimetry::Bt709);
    b.packing = PackingMode::Block;
    v.push(b);
    v
}

#[test]
fn mutated_packets_never_panic() {
    let mut rng = Rng(0x2110_2110_2110_2110);
    for f in formats() {
        let mut tx = Packetizer::new(f.clone(), 100, 1, 0).unwrap();
        let mut packets = Vec::new();
        let img: Vec<u8> = (0..tx.image_len(false)).map(|i| i as u8).collect();
        tx.packetize(&img, 9, false, |p| packets.push(p.to_vec())).unwrap();
        if f.interlace {
            let second: Vec<u8> = vec![7; tx.image_len(true)];
            tx.packetize(&second, 9, true, |p| packets.push(p.to_vec())).unwrap();
        }
        let mut rx = Depacketizer::new(f.clone()).unwrap();
        let max_len = img.len(); // the first field is the larger one
        for round in 0..4000 {
            let mut p = packets[rng.below(packets.len())].clone();
            match round % 4 {
                0 => {
                    let at = rng.below(p.len());
                    p[at] ^= 1 << rng.below(8);
                }
                1 => p.truncate(rng.below(p.len())),
                2 => {
                    for _ in 0..rng.below(6) + 1 {
                        let at = rng.below(p.len());
                        p[at] = rng.next() as u8;
                    }
                }
                _ => p = (0..rng.below(1500)).map(|_| rng.next() as u8).collect(),
            }
            let _ = rx.push(&p);
            while let Some(i) = rx.pop() {
                assert!(i.data.len() <= max_len);
                assert!(i.received <= i.data.len() * 4);
            }
        }
        rx.flush();
    }
}

#[test]
fn mangled_sdp_never_panics() {
    let base = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=x\r\nt=0 0\r\nm=video 5000 RTP/AVP 96\r\nc=IN IP4 239.0.0.1/32\r\na=rtpmap:96 raw/90000\r\na=fmtp:96 sampling=YCbCr-4:2:2; width=1920; height=1080; exactframerate=50; depth=10; colorimetry=BT709; PM=2110GPM; SSN=ST2110-20:2017; TP=2110TPW\r\na=ts-refclk:ptp=IEEE1588-2008:traceable\r\na=mediaclk:direct=0\r\n";
    let mut rng = Rng(42);
    let bytes = base.as_bytes();
    for _ in 0..5000 {
        let mut b = bytes.to_vec();
        for _ in 0..rng.below(4) + 1 {
            let at = rng.below(b.len());
            match rng.below(3) {
                0 => {
                    const CHARS: &[u8] = b" =:;/-0123456789abcxyzAZ\r\n";
                    b[at] = CHARS[rng.below(CHARS.len())];
                }
                1 => {
                    b.remove(at);
                }
                _ => b.truncate(at),
            }
            if b.is_empty() {
                break;
            }
        }
        let Ok(text) = std::str::from_utf8(&b) else { continue };
        if let Ok(s) = Session::parse(text) {
            for m in &s.media {
                let _ = m.rtpmap(96);
                let _ = m.ts_refclk();
                let _ = m.mediaclk();
                if let Some(f) = m.fmtp(96) {
                    let _ = VideoFormat::from_fmtp(&f);
                }
            }
            let _ = Session::parse(&s.write());
        }
    }
}

#[test]
fn mangled_pmc_output_never_panics_or_locks_by_accident() {
    use cfa4_st2110::clock::{DEFAULT_LOCK_LIMIT_NS, PtpStatus};
    let base = "\t\tmaster_offset -87\n\t\tgmPresent true\n\t\tgmIdentity 0c4f5a.fffe.10e2d4\n\t\tdomainNumber 127\n\t\tportState SLAVE\n";
    let mut rng = Rng(7);
    let mut locked = 0;
    for _ in 0..5000 {
        let mut b = base.as_bytes().to_vec();
        for _ in 0..rng.below(3) + 1 {
            let at = rng.below(b.len());
            b[at] = b" .-0123456789abcdefSLAVEtrux\t\n"[rng.below(30)];
        }
        let Ok(text) = std::str::from_utf8(&b) else { continue };
        if let Ok(s) = PtpStatus::parse_pmc(text)
            && s.locked(DEFAULT_LOCK_LIMIT_NS)
        {
            locked += 1;
            assert!(s.grandmaster.is_some() && s.domain.is_some());
        }
    }
    assert!(locked < 5000);
}
