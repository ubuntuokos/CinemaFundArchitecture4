//! A THP writer that picks every syntax element at random (shared by the oracle test here and
//! the media-source test in `cfa4-codecs`).
#![allow(dead_code)]

pub struct Rng(pub u64);

impl Rng {
    pub fn step(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.step() % n.max(1)
    }
}

pub struct Writer {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Writer {
    fn new() -> Self {
        Writer { out: Vec::new(), acc: 0, n: 0 }
    }
    fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | u64::from((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn align(&mut self, rng: &mut Rng) {
        while self.n != 0 {
            self.put(rng.below(2) as u32, 1);
        }
    }
}

/// DC: 12 symbols of 4 bits (code = symbol). AC: the 162 symbols (run 0..15 × size 1..10, EOB,
/// ZRL) of 8 bits in the order of `AC_SYMBOLS`.
pub fn ac_symbols() -> Vec<u8> {
    let mut v = vec![0x00, 0xF0];
    for run in 0..16u8 {
        for size in 1..=10u8 {
            v.push(run << 4 | size);
        }
    }
    v
}

pub fn put_amp(w: &mut Writer, level: i32, size: u32) {
    let v = if level >= 0 { level } else { level + (1 << size) - 1 };
    w.put(v as u32, size);
}

pub fn block(w: &mut Writer, rng: &mut Rng, syms: &[u8]) {
    // DC difference
    let size = if rng.below(4) == 0 { rng.below(7) as u32 } else { rng.below(5) as u32 };
    w.put(size, 4);
    if size > 0 {
        let lim = 1i32 << size;
        let mag = (lim >> 1) + rng.below((lim >> 1) as u64) as i32;
        put_amp(w, if rng.below(2) == 0 { mag } else { -mag }, size);
    }
    // AC coefficients
    let mut i = 0usize;
    let density = rng.below(4);
    loop {
        if i >= 63 || rng.below(8) < (6 - density).min(5) && i > 0 {
            if i < 63 {
                emit(w, syms, 0x00);
            }
            break;
        }
        let run = if density == 0 { rng.below(3) } else { rng.below(1 + 2 * density) } as usize;
        if i + run + 1 > 63 {
            emit(w, syms, 0x00);
            break;
        }
        let size = 1 + if rng.below(6) == 0 { rng.below(5) } else { rng.below(3) } as u32;
        let lim = 1i32 << size;
        let mag = (lim >> 1) + rng.below((lim >> 1) as u64) as i32;
        emit(w, syms, (run as u8) << 4 | size as u8);
        put_amp(w, if rng.below(2) == 0 { mag } else { -mag }, size);
        i += run + 1;
    }
}

pub fn emit(w: &mut Writer, syms: &[u8], sym: u8) {
    let idx = syms.iter().position(|&s| s == sym).expect("symbol");
    w.put(idx as u32, 8);
}

pub fn seg(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// A THP picture: `samp` is the sampling of the luma plane ((2,2), (2,1), (1,2), (1,1)) or
/// `None` for a grey picture.
pub fn picture(w: usize, h: usize, samp: Option<(usize, usize)>, restart: usize, rng: &mut Rng) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    let mut dqt = vec![0u8];
    for _ in 0..64 {
        dqt.push(1 + rng.below(24) as u8);
    }
    dqt.push(1);
    for _ in 0..64 {
        dqt.push(1 + rng.below(24) as u8);
    }
    seg(&mut out, 0xDB, &dqt);
    let ncomp = if samp.is_some() { 3 } else { 1 };
    let (hs, vs) = samp.unwrap_or((1, 1));
    let mut sof = vec![8, (h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8, ncomp as u8];
    for c in 0..ncomp {
        sof.extend_from_slice(&[c as u8 + 1, if c == 0 { (hs << 4 | vs) as u8 } else { 0x11 }, u8::from(c > 0)]);
    }
    seg(&mut out, 0xC0, &sof);
    let syms = ac_symbols();
    let mut dht = vec![0x00];
    let mut bits = [0u8; 16];
    bits[3] = 12;
    dht.extend_from_slice(&bits);
    dht.extend(0..12u8);
    dht.push(0x10);
    let mut bits = [0u8; 16];
    bits[7] = 162;
    dht.extend_from_slice(&bits);
    dht.extend_from_slice(&syms);
    seg(&mut out, 0xC4, &dht);
    if restart > 0 {
        seg(&mut out, 0xDD, &(restart as u16).to_be_bytes());
    }
    let mut sos = vec![ncomp as u8];
    for c in 0..ncomp {
        sos.extend_from_slice(&[c as u8 + 1, 0x00]);
    }
    sos.extend_from_slice(&[0, 63, 0]);
    seg(&mut out, 0xDA, &sos);
    let (mw, mh) = (w.div_ceil(hs * 8), h.div_ceil(vs * 8));
    let mut bw = Writer::new();
    let mut count = 0;
    for _ in 0..mw * mh {
        if restart > 0 && count == 0 {
            count = restart;
        }
        let blocks = if samp.is_some() { hs * vs + 2 } else { 1 };
        for _ in 0..blocks {
            block(&mut bw, rng, &syms);
        }
        if restart > 0 {
            count -= 1;
            if count == 0 {
                bw.align(rng);
            }
        }
    }
    bw.align(rng);
    out.extend(bw.out);
    out
}

/// A sound packet of `n` samples per channel.
pub fn audio_packet(ch: usize, n: usize, rng: &mut Rng) -> Vec<u8> {
    let mut p = vec![0u8; 4];
    p.extend_from_slice(&(n as u32).to_be_bytes());
    for _ in 0..ch {
        for _ in 0..16 {
            p.extend_from_slice(&((rng.below(4096) as i32 - 2048) as i16).to_be_bytes());
        }
    }
    for _ in 0..ch {
        p.extend_from_slice(&((rng.below(20000) as i32 - 10000) as i16).to_be_bytes());
        p.extend_from_slice(&((rng.below(20000) as i32 - 10000) as i16).to_be_bytes());
    }
    for _ in 0..ch {
        for i in 0..n.div_ceil(14) {
            p.push((rng.below(8) << 4 | rng.below(12)) as u8);
            let samples = (n - i * 14).min(14);
            for _ in 0..samples.div_ceil(2) {
                p.push(rng.below(256) as u8);
            }
        }
    }
    let size = p.len() as u32;
    p[..4].copy_from_slice(&size.to_be_bytes());
    p
}

pub fn thp_file(pics: &[Vec<u8>], audio: Option<(u32, Vec<Vec<u8>>)>, w: usize, h: usize) -> Vec<u8> {
    let mut f = Vec::new();
    let push = |f: &mut Vec<u8>, v: u32| f.extend_from_slice(&v.to_be_bytes());
    f.extend_from_slice(b"THP\0");
    push(&mut f, 0x11000);
    push(&mut f, 0);
    push(&mut f, 0);
    push(&mut f, 29.97f32.to_bits());
    push(&mut f, pics.len() as u32);
    let sizes: Vec<usize> =
        pics.iter().enumerate().map(|(i, p)| if audio.is_some() { 16 } else { 12 } + p.len() + audio.as_ref().map_or(0, |a| a.1[i].len())).collect();
    push(&mut f, sizes[0] as u32);
    push(&mut f, 0);
    push(&mut f, 48);
    push(&mut f, 0);
    let comp_len = 4 + 16 + 12 + if audio.is_some() { 12 } else { 0 };
    push(&mut f, (48 + comp_len) as u32);
    push(&mut f, 0);
    push(&mut f, if audio.is_some() { 2 } else { 1 });
    let mut types = [0u8; 16];
    types[0] = 0;
    types[1] = 1;
    f.extend_from_slice(&types);
    push(&mut f, w as u32);
    push(&mut f, h as u32);
    push(&mut f, 0);
    if let Some((ch, _)) = &audio {
        push(&mut f, *ch);
        push(&mut f, 32000);
        push(&mut f, 0);
    }
    for (i, p) in pics.iter().enumerate() {
        push(&mut f, sizes.get(i + 1).copied().unwrap_or(0) as u32);
        push(&mut f, if i > 0 { sizes[i - 1] as u32 } else { 0 });
        push(&mut f, p.len() as u32);
        if let Some((_, a)) = &audio {
            push(&mut f, a[i].len() as u32);
        }
        f.extend_from_slice(p);
        if let Some((_, a)) = &audio {
            f.extend_from_slice(&a[i]);
        }
    }
    f
}
