//! A writer of Smacker (`.smk`) test files whose every syntax element is drawn at random within
//! the format's rules: the four header trees (with skipped trees, escape values and
//! most-recent-value slots), palette updates (kept, copied and new runs), SMK2 and SMK4, ring
//! frames, up to seven audio tracks (packed 8- and 16-bit, mono and stereo, and raw PCM), and
//! frame data that is random bytes (every byte string is a valid walk through a prefix code, so
//! the picture blocks come out random, with runs, skips, fills and every full-block mode).
//! Some chunks are damaged on purpose (a wrong channel count, a "no data" audio chunk).
//! `cfa4-smacker` and `cfa4-codecs` check their decoders with these files; ffmpeg has no
//! Smacker encoder.

pub struct Rng(pub u64);

impl Rng {
    pub fn draw(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.draw() % n.max(1)
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    pub fn byte(&mut self) -> u8 {
        self.draw() as u8
    }
}

/// Little-endian bit writer (the first bit written is the lowest of a value).
#[derive(Default)]
struct Bw {
    bytes: Vec<u8>,
    nbits: usize,
}

impl Bw {
    fn put(&mut self, v: u64, n: u32) {
        for i in 0..n {
            if self.nbits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (self.nbits % 8);
            }
            self.nbits += 1;
        }
    }
    fn code(&mut self, bits: &[bool]) {
        for &b in bits {
            self.put(u64::from(b), 1);
        }
    }
}

/// The leaves of a written tree: the value and the bits that reach it.
type Leaves = Vec<(u8, Vec<bool>)>;

/// Writes a random tree of about `leaves` leaves in the file's form (1: node, 0: leaf and its
/// 8 bits), at most 20 deep.
fn write_tree(rng: &mut Rng, bw: &mut Bw, leaves: usize, depth: usize, path: &mut Vec<bool>, out: &mut Leaves) {
    if leaves <= 1 || depth >= 20 {
        bw.put(0, 1);
        let v = rng.byte();
        bw.put(u64::from(v), 8);
        out.push((v, path.clone()));
        return;
    }
    bw.put(1, 1);
    let left = 1 + rng.below(leaves as u64 - 1) as usize;
    path.push(false);
    write_tree(rng, bw, left, depth + 1, path, out);
    path.pop();
    path.push(true);
    write_tree(rng, bw, leaves - left, depth + 1, path, out);
    path.pop();
}

fn small_tree(rng: &mut Rng, bw: &mut Bw) -> Leaves {
    let n = if rng.chance(20) { 1 } else { 2 + rng.below(30) as usize };
    let mut out = Vec::new();
    write_tree(rng, bw, n, 0, &mut Vec::new(), &mut out);
    out
}

/// One header tree (the part after its present bit); returns the declared size.
fn header_tree(rng: &mut Rng, bw: &mut Bw) -> u32 {
    let mut trees: [Option<Leaves>; 2] = [None, None];
    for t in &mut trees {
        if rng.chance(85) {
            bw.put(1, 1);
            *t = Some(small_tree(rng, bw));
            bw.put(0, 1);
        } else {
            bw.put(0, 1);
        }
    }
    let pick = |rng: &mut Rng, t: &Option<Leaves>| -> (u8, Vec<bool>) {
        match t {
            Some(l) => l[rng.below(l.len() as u64) as usize].clone(),
            None => (0, Vec::new()),
        }
    };
    for _ in 0..3 {
        let v = if rng.chance(60) {
            let (lo, _) = pick(rng, &trees[0]);
            let (hi, _) = pick(rng, &trees[1]);
            u64::from(lo) | (u64::from(hi) << 8)
        } else {
            rng.below(65536)
        };
        bw.put(v, 16);
    }
    fn big(rng: &mut Rng, bw: &mut Bw, trees: &[Option<Leaves>; 2], leaves: usize, depth: usize, entries: &mut u32) {
        *entries += 1;
        if leaves <= 1 || depth >= 30 {
            bw.put(0, 1);
            for l in trees.iter().flatten() {
                let (_, code) = &l[rng.below(l.len() as u64) as usize];
                bw.code(code);
            }
            return;
        }
        bw.put(1, 1);
        let left = 1 + rng.below(leaves as u64 - 1) as usize;
        big(rng, bw, trees, left, depth + 1, entries);
        big(rng, bw, trees, leaves - left, depth + 1, entries);
    }
    let mut entries = 0;
    let leaves = 1 + rng.below(40) as usize;
    big(rng, bw, &trees, leaves, 0, &mut entries);
    bw.put(0, 1);
    4 * (entries + rng.below(3) as u32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioKind {
    /// Huffman-packed.
    Packed,
    /// Uncompressed PCM.
    Pcm,
}

#[derive(Clone, Debug)]
pub struct TrackSpec {
    pub rate: u32,
    pub stereo: bool,
    pub b16: bool,
    pub kind: AudioKind,
}

pub struct Spec {
    pub v4: bool,
    pub w: u32,
    pub h: u32,
    pub frames: u32,
    pub ring: bool,
    pub pts_inc: i32,
    pub interlace: bool,
    pub tracks: [Option<TrackSpec>; 7],
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn palette_chunk(rng: &mut Rng, first: bool) -> Vec<u8> {
    let mut body = Vec::new();
    let mut sz = 0usize;
    while sz < 256 {
        let left = 256 - sz;
        match if first { 2 } else { rng.below(3) } {
            0 => {
                let n = 1 + rng.below(left.min(128) as u64) as usize;
                body.push(0x80 | (n - 1) as u8);
                sz += n;
            }
            1 => {
                let n = 1 + rng.below(left.min(64) as u64) as usize;
                let off = rng.below(257 - n as u64) as usize;
                body.push(0x40 | (n - 1) as u8);
                body.push(off as u8);
                sz += n;
            }
            _ => {
                body.push(rng.below(64) as u8);
                body.push(rng.byte());
                body.push(rng.byte());
                sz += 1;
            }
        }
    }
    // the chunk is its size byte and the body, a multiple of 4 bytes long
    while !(body.len() + 1).is_multiple_of(4) {
        body.push(0);
    }
    let mut out = vec![((body.len() + 1) / 4) as u8];
    out.extend(body);
    out
}

fn audio_chunk(rng: &mut Rng, t: &TrackSpec, frame: u32) -> Vec<u8> {
    let ch = if t.stereo { 2usize } else { 1 };
    let bytes = if t.b16 { 2usize } else { 1 };
    let n = 20 + rng.below(300) as usize;
    let mut body = Vec::new();
    match t.kind {
        AudioKind::Pcm if t.b16 => {
            body.extend((0..n * ch * 2).map(|_| rng.byte()));
        }
        AudioKind::Pcm => {
            // ffmpeg keeps the first four bytes (a length) as part of the data
            body.extend(le32((n * ch) as u32));
            body.extend((0..n * ch).map(|_| rng.byte()));
        }
        _ => {
            let unp = (n * ch * bytes) as u32;
            body.extend(le32(unp));
            let mut bw = Bw::default();
            if rng.chance(8) && frame > 0 {
                bw.put(0, 1);
            } else {
                bw.put(1, 1);
                // a wrong channel count or sample size makes ffmpeg refuse the chunk
                let wrong = rng.chance(4);
                bw.put(u64::from(t.stereo != wrong), 1);
                bw.put(u64::from(t.b16), 1);
                for _ in 0..(1usize << (usize::from(t.b16) + usize::from(t.stereo))) {
                    bw.put(0, 1);
                    small_tree(rng, &mut bw);
                    bw.put(0, 1);
                }
                for _ in 0..ch {
                    bw.put(rng.below(1 << (8 * bytes)), 8 * bytes as u32);
                }
            }
            body.extend(bw.bytes);
            body.extend((0..rng.below(400)).map(|_| rng.byte()));
        }
    }
    let mut out = le32(4 + body.len() as u32).to_vec();
    out.extend(body);
    out
}

pub fn smk_file(rng: &mut Rng, spec: &Spec) -> Vec<u8> {
    let n_frames = spec.frames + u32::from(spec.ring);
    // header trees
    let mut tb = Bw::default();
    let mut sizes = [0u32; 4];
    for (i, s) in sizes.iter_mut().enumerate() {
        if i > 0 && rng.chance(12) {
            tb.put(0, 1);
        } else {
            tb.put(1, 1);
            *s = header_tree(rng, &mut tb);
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(if spec.v4 { b"SMK4" } else { b"SMK2" });
    let flags = u32::from(spec.ring) | if spec.interlace { 2 } else { 0 };
    for v in [spec.w, spec.h, spec.frames, spec.pts_inc as u32, flags] {
        out.extend(le32(v));
    }
    for _ in 0..7 {
        out.extend(le32(0));
    }
    out.extend(le32(tb.bytes.len() as u32));
    for s in sizes {
        out.extend(le32(s));
    }
    for t in &spec.tracks {
        match t {
            Some(t) => {
                let mut f = 0u8;
                if t.kind == AudioKind::Packed {
                    f |= 0x80;
                }
                if t.b16 {
                    f |= 0x20;
                }
                if t.stereo {
                    f |= 0x10;
                }
                out.extend_from_slice(&t.rate.to_le_bytes()[..3]);
                out.push(f);
            }
            None => out.extend(le32(0)),
        }
    }
    out.extend(le32(0));
    let mut sizes_b = Vec::new();
    let mut flags_b = Vec::new();
    let mut data = Vec::new();
    for i in 0..n_frames {
        let mut f = Vec::new();
        let mut fl = 0u8;
        if i == 0 || rng.chance(25) {
            fl |= 1;
            f.extend(palette_chunk(rng, i == 0));
        }
        for (k, t) in spec.tracks.iter().enumerate() {
            if let Some(t) = t
                && rng.chance(85)
            {
                fl |= 2 << k;
                f.extend(audio_chunk(rng, t, i));
            }
        }
        let video_len = 1 + rng.below(700) as usize;
        f.extend((0..video_len).map(|_| rng.byte()));
        while !f.len().is_multiple_of(4) {
            f.push(0);
        }
        let key = u32::from(i == 0 || rng.chance(20));
        sizes_b.extend(le32(f.len() as u32 | key));
        flags_b.push(fl);
        data.extend(f);
    }
    out.extend(sizes_b);
    out.extend(flags_b);
    out.extend(tb.bytes);
    out.extend(data);
    out
}

pub fn random_spec(rng: &mut Rng) -> Spec {
    let mut tracks: [Option<TrackSpec>; 7] = Default::default();
    // mostly up to three tracks, now and then four to seven on distinct slots
    let count = if rng.chance(25) { 4 + rng.below(4) as usize } else { rng.below(4) as usize };
    let mut slots = [0usize, 1, 2, 3, 4, 5, 6];
    for n in 0..count {
        let k = n + rng.below((7 - n) as u64) as usize;
        slots.swap(n, k);
        let slot = slots[n];
        let kind = match rng.below(5) {
            0 => AudioKind::Pcm,
            _ => AudioKind::Packed,
        };
        tracks[slot] = Some(TrackSpec { rate: [8000, 11025, 22050, 44100][rng.below(4) as usize], stereo: rng.chance(50), b16: rng.chance(50), kind });
    }
    let sizes = [(16, 16), (64, 48), (96, 80), (32, 24), (20, 12), (36, 28)];
    let (w, h) = sizes[rng.below(sizes.len() as u64) as usize];
    Spec {
        v4: rng.chance(50),
        w,
        h,
        frames: 3 + rng.below(10) as u32,
        ring: rng.chance(40),
        pts_inc: [71, 100, 40, -4000, 33][rng.below(5) as usize],
        interlace: rng.chance(20),
        tracks,
    }
}
