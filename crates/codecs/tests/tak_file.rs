//! A `.tak` file read through the media source: the same samples as `cfa4_tak::decode_file`,
//! read front to back and from the middle.

/// A mono 16-bit 44.1 kHz TAK file of `samples` in 4096-sample frames, each channel coded as
/// first differences with one plain residual code (a minimal writer; `crates/tak/tests/oracle.rs`
/// has the full one).
fn tak(samples: &[i16]) -> Vec<u8> {
    #[derive(Default)]
    struct W(Vec<u8>, usize);
    impl W {
        fn put(&mut self, v: u64, n: u32) {
            for i in 0..n {
                if self.1.is_multiple_of(8) {
                    self.0.push(0);
                }
                if (v >> i) & 1 == 1 {
                    let l = self.0.len() - 1;
                    self.0[l] |= 1 << (self.1 % 8);
                }
                self.1 += 1;
            }
        }
        fn crc(&mut self, from: usize) {
            self.1 = self.0.len() * 8;
            let c = cfa4_tak::crc24(&self.0[from..]);
            self.0.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
            self.1 += 24;
        }
    }
    let info = |w: &mut W| {
        for (v, n) in [(2, 6), (0, 4), (4, 4), (samples.len() as u64, 35), (0, 3), (44100 - 6000, 18), (8, 5), (0, 4), (0, 1)] {
            w.put(v, n);
        }
    };
    let mut si = W::default();
    info(&mut si);
    si.crc(0);
    let mut out = b"tBaK".to_vec();
    out.extend_from_slice(&[1, si.0.len() as u8, 0, 0]);
    out.extend_from_slice(&si.0);
    out.extend_from_slice(&[0; 4]);
    let chunks: Vec<&[i16]> = samples.chunks(4096).collect();
    for (k, c) in chunks.iter().enumerate() {
        let last = k + 1 == chunks.len();
        let mut w = W::default();
        w.put(0xA0FF, 16);
        w.put(u64::from(last) | 2, 3);
        w.put(k as u64, 21);
        if last {
            w.put(c.len() as u64 - 1, 14);
            w.put(0, 2);
        }
        info(&mut w);
        w.put(0, 6);
        w.crc(0);
        let h = w.0.len();
        // first sample, channel LPC mode 1, one subframe, no LPC, simple code, mode 35 (19 bits)
        for (v, n) in [(0, 1), (c[0] as u16 as u64, 16), (1, 2), (0, 3), (0, 1), (0, 1), (35, 6)] {
            w.put(v, n);
        }
        for p in c.windows(2) {
            let d = i32::from(p[1]) - i32::from(p[0]);
            let x = ((d << 1) ^ (d >> 31)) as u32;
            w.put(u64::from(x), 19);
            if x >= 0x58000 {
                w.put(0, 1);
            }
        }
        w.crc(h);
        out.extend_from_slice(&w.0);
    }
    out
}

#[test]
fn tak_file_reads_through_the_media_source() {
    let pcm: Vec<i16> = (0..20_000).map(|i| ((f64::from(i) * 0.031).sin() * 12000.0 + f64::from(i % 97) * 30.0) as i16).collect();
    let data = tak(&pcm);
    let (info, planes) = cfa4_tak::decode_file(&data).unwrap();
    assert_eq!(planes[0], pcm.iter().map(|&s| i32::from(s)).collect::<Vec<_>>());
    let want: Vec<f32> = planes[0].iter().map(|&v| info.to_f32(v)).collect();
    let s = cfa4_codecs::open_bytes("music.tak", data.into()).unwrap();
    let a = s.info().audio().unwrap();
    assert_eq!((a.sample_rate, a.channels, a.codec.as_str()), (44100, 1, "TAK"));
    let read = |from: usize, n: usize| s.audio(from as i64, n, 44100).unwrap().channels[0].clone();
    assert_eq!(read(0, want.len()), want);
    assert_eq!(read(12_345, 4000), want[12_345..16_345]);
}
