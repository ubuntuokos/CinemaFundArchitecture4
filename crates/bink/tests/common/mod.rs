//! Shared test helpers: a little-endian bit writer, a random generator and a `.bik` writer.
#![allow(dead_code)]

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
}

#[derive(Default)]
pub struct BitW {
    pub bytes: Vec<u8>,
    pub n: usize,
}

impl BitW {
    pub fn put(&mut self, bits: u32, v: u32) {
        for i in 0..bits {
            if self.n.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 != 0 {
                *self.bytes.last_mut().unwrap() |= 1 << (self.n % 8);
            }
            self.n += 1;
        }
    }
    pub fn align32(&mut self) {
        while !self.n.is_multiple_of(32) {
            self.put(1, 0);
        }
    }
}

pub struct Track {
    pub rate: u16,
    pub flags: u16,
    pub packets: Vec<Vec<u8>>,
}

/// A `.bik` file of `video` packets (and per-frame audio packets of each track).
pub fn bik(tag: &[u8; 4], w: u32, h: u32, flags: u32, video: &[Vec<u8>], tracks: &[Track]) -> Vec<u8> {
    bik_keys(tag, w, h, flags, video, tracks, &[0])
}

/// [`bik`] with the frames in `keys` flagged as key frames in the index.
pub fn bik_keys(tag: &[u8; 4], w: u32, h: u32, flags: u32, video: &[Vec<u8>], tracks: &[Track], keys: &[usize]) -> Vec<u8> {
    let frames = video.len();
    let mut body: Vec<Vec<u8>> = Vec::new();
    for (i, v) in video.iter().enumerate() {
        let mut f = Vec::new();
        for t in tracks {
            let p = t.packets.get(i).cloned().unwrap_or_default();
            f.extend_from_slice(&(p.len() as u32).to_le_bytes());
            f.extend_from_slice(&p);
        }
        f.extend_from_slice(v);
        // keep frames 4-aligned like the real files
        while !f.len().is_multiple_of(4) {
            f.push(0);
        }
        body.push(f);
    }
    let mut head = Vec::new();
    let w32 = |h: &mut Vec<u8>, v: u32| h.extend_from_slice(&v.to_le_bytes());
    head.extend_from_slice(tag);
    w32(&mut head, 0); // size, patched
    w32(&mut head, frames as u32);
    w32(&mut head, body.iter().map(Vec::len).max().unwrap_or(0) as u32);
    w32(&mut head, frames as u32);
    w32(&mut head, w);
    w32(&mut head, h);
    w32(&mut head, 30);
    w32(&mut head, 1);
    w32(&mut head, flags);
    w32(&mut head, tracks.len() as u32);
    if tag[3] == b'k' {
        w32(&mut head, 0);
    }
    if !tracks.is_empty() {
        for _ in tracks {
            w32(&mut head, 1 << 16);
        }
        for t in tracks {
            head.extend_from_slice(&t.rate.to_le_bytes());
            head.extend_from_slice(&t.flags.to_le_bytes());
        }
        for i in 0..tracks.len() {
            w32(&mut head, i as u32);
        }
    }
    let index_at = head.len();
    head.resize(index_at + 4 * frames, 0);
    let mut pos = head.len();
    for (i, b) in body.iter().enumerate() {
        let key = u32::from(keys.contains(&i));
        head[index_at + 4 * i..index_at + 4 * i + 4].copy_from_slice(&((pos as u32) | key).to_le_bytes());
        pos += b.len();
    }
    let total = pos;
    head[4..8].copy_from_slice(&((total - 8) as u32).to_le_bytes());
    for b in body {
        head.extend_from_slice(&b);
    }
    head
}

// ---------------------------------------------------------------- video streams

pub fn log2(v: u32) -> u32 {
    31 - v.leading_zeros()
}

/// One plane of a first frame: every block FILL (6), PATTERN (8) or RAW (9).
pub fn first_plane(rng: &mut Rng, w: &mut BitW, ver: u8, width: u32, height: u32, chroma: bool) {
    let pw = width >> chroma as u32;
    let (bw, bh) = if chroma { ((width + 15) >> 4, (height + 15) >> 4) } else { ((width + 7) >> 3, (height + 7) >> 3) };
    if ver == b'k' {
        w.put(1, 0);
    }
    let wa = pw.max(8).div_ceil(8) * 8;
    let l_types = log2((wa >> 3) + 511) + 1;
    let l_sub = log2((wa >> 4) + 511) + 1;
    let l_col = log2(bw * 64 + 511) + 1;
    let l_pat = log2((bw << 3) + 511) + 1;
    let l_run = log2(bw * 48 + 511) + 1;
    for i in 0..9 {
        if i == 2 {
            for _ in 0..16 {
                w.put(4, 0);
            }
        }
        if i != 6 && i != 7 {
            w.put(4, 0);
        }
    }
    for row in 0..bh {
        // a bundle whose count is 0 is closed for the rest of the plane, so every row keeps
        // patterns (and colours) flowing, and the empty bundles are announced once
        let mut types: Vec<u32> = (0..bw).map(|_| [6, 8, 9][rng.below(3) as usize]).collect();
        types[0] = 8;
        let n_col: u32 = types.iter().map(|&t| [0, 0, 0, 0, 0, 0, 1, 0, 2, 64][t as usize]).sum();
        let n_pat: u32 = types.iter().filter(|&&t| t == 8).count() as u32 * 8;
        let count = if ver == b'k' { bw ^ 0xBB } else { bw };
        w.put(l_types, count);
        w.put(1, 0);
        for &t in &types {
            w.put(4, t);
        }
        if row == 0 {
            w.put(l_sub, 0);
        }
        w.put(l_col, n_col);
        w.put(1, 0);
        for _ in 0..n_col {
            let b = rng.below(256);
            w.put(4, b >> 4);
            w.put(4, b & 15);
        }
        w.put(l_pat, n_pat);
        for _ in 0..n_pat {
            let b = rng.below(256);
            w.put(4, b & 15);
            w.put(4, b >> 4);
        }
        if row == 0 {
            for _ in 0..4 {
                w.put(l_types, 0); // x offsets, y offsets, intra dc, inter dc
            }
            w.put(l_run, 0);
        }
    }
    w.align32();
}

/// One plane of a frame of the old format (`b`). Block types: 0 skip, 2 intra DCT, 3 motion with
/// residue, 4 motion with DCT, 5 fill, 6 pattern, 7 motion, 8 raw. A bundle whose count is 0 is
/// closed for the rest of the plane, so the kinds a plane uses are forced into every row. The
/// inline bits of the DCT and residue blocks are small fixed snippets of known length.
pub fn plane_b(rng: &mut Rng, w: &mut BitW, width: u32, height: u32, chroma: bool, inter: bool) {
    let (bw, bh) = if chroma { ((width + 15) >> 4, (height + 15) >> 4) } else { ((width + 7) >> 3, (height + 7) >> 3) };
    let (pw, ph) = (((width + chroma as u32) >> chroma as u32) as i32, ((height + chroma as u32) >> chroma as u32) as i32);
    let motion = inter && bw > 1 && pw >= 8 && ph >= 8;
    // the kinds in use: slot i of every row is forced to forced[i]
    let mut forced: Vec<u32> = vec![6];
    if motion {
        forced.push(7);
    }
    if bw >= 3 {
        forced.push(2);
    }
    if motion && bw >= 4 {
        forced.push(4);
    }
    if motion && bw >= 5 {
        forced.push(3);
    }
    let mut kinds: Vec<u32> = vec![5, 6, 8, 2];
    if inter {
        kinds.push(0);
    }
    if motion {
        kinds.extend([7, 4, 3]);
    }
    kinds.retain(|k| forced.contains(k) || matches!(k, 5 | 8 | 0));
    for row in 0..bh {
        let mut types: Vec<u32> = (0..bw).map(|_| kinds[rng.below(kinds.len() as u32) as usize]).collect();
        for (i, &f) in forced.iter().enumerate() {
            types[i] = f;
        }
        let count = |f: &dyn Fn(u32) -> bool| types.iter().filter(|&&t| f(t)).count() as u32;
        let n_col: u32 = types.iter().map(|&t| [0, 0, 0, 0, 0, 1, 2, 0, 64][t as usize]).sum();
        let n_pat = count(&|t| t == 6) * 8;
        let n_mov = count(&|t| matches!(t, 3 | 4 | 7));
        let n_intra = count(&|t| t == 2);
        let n_inter = count(&|t| t == 4);
        let n_res = count(&|t| t == 3);
        // bundles 0..=9 in order
        w.put(13, bw);
        for &t in &types {
            w.put(4, t);
        }
        w.put(13, n_col);
        for _ in 0..n_col {
            w.put(8, rng.below(256));
        }
        w.put(13, n_pat);
        for _ in 0..n_pat {
            w.put(8, rng.below(256));
        }
        for axis in 0..2 {
            // offsets that keep the reference block inside the visible picture (ffmpeg does not
            // keep the padding around it from one frame to the next in this old format)
            if n_mov > 0 || row == 0 {
                w.put(13, n_mov);
            }
            for (bx, _) in types.iter().enumerate().filter(|(_, t)| matches!(**t, 3 | 4 | 7)) {
                let (pos, n) = if axis == 0 { (bx as i32 * 8, pw) } else { (row as i32 * 8, ph) };
                let lo = (-pos).max(-16);
                let hi = (n - 8 - pos).min(15);
                let v = if hi < lo { 0 } else { lo + rng.below((hi - lo + 1) as u32) as i32 };
                w.put(5, (v + 16) as u32);
            }
        }
        let bundle = |w: &mut BitW, n: u32, bits: u32, rng: &mut Rng, lo: u32| {
            if n > 0 || row == 0 {
                w.put(13, n);
            }
            for _ in 0..n {
                w.put(bits, lo + rng.below((1 << bits) - lo));
            }
        };
        bundle(w, n_intra, 11, rng, 0); // 5: intra dc
        bundle(w, n_inter, 11, rng, 0); // 6: inter dc
        bundle(w, n_intra, 4, rng, 0); // 7: intra quantizer
        bundle(w, n_inter, 4, rng, 0); // 8: inter quantizer
        bundle(w, n_res, 7, rng, 2); // 9: residue masks
        // inline bits, in block order
        for &t in &types {
            match t {
                2 | 4 => {
                    if rng.below(2) == 0 {
                        w.put(4, 0);
                    } else {
                        w.put(4, 1);
                        w.put(3, 0);
                        w.put(1, 1);
                        w.put(1, rng.below(2));
                        w.put(1, 0);
                        w.put(1, 1);
                        w.put(1, rng.below(2));
                    }
                }
                3 => {
                    w.put(3, 0);
                    w.put(3, 0);
                    w.put(1, 1);
                    w.put(1, 0);
                    w.put(1, rng.below(2));
                    w.put(1, 1);
                    w.put(1, 0);
                    w.put(1, rng.below(2));
                    w.put(1, 1);
                }
                _ => {}
            }
        }
    }
    w.align32();
}

/// A whole frame of the old format.
pub fn frame_b(rng: &mut Rng, w: u32, h: u32, inter: bool) -> Vec<u8> {
    let mut b = BitW::default();
    for p in 0..3 {
        plane_b(rng, &mut b, w, h, p != 0, inter);
    }
    b.bytes
}

/// The first frame of a stream: all planes, with the alpha plane first when there is one.
pub fn first_frame(rng: &mut Rng, ver: u8, w: u32, h: u32, alpha: bool) -> Vec<u8> {
    let mut b = BitW::default();
    if ver == b'b' {
        return frame_b(rng, w, h, false);
    }
    if alpha {
        if ver >= b'i' {
            b.put(32, 0);
        }
        first_plane(rng, &mut b, ver, w, h, false);
    }
    if ver >= b'i' {
        b.put(32, 0);
    }
    for p in 0..3 {
        first_plane(rng, &mut b, ver, w, h, p != 0);
    }
    b.bytes
}

// ---------------------------------------------------------------- audio streams

pub const CRITICAL_FREQS: [u32; 25] =
    [100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700, 4400, 5300, 6400, 7700, 9500, 12000, 15500, 24500];

pub struct Params {
    pub rate: u32,
    pub channels: u32,
    pub dct: bool,
    pub revision: u8,
}

impl Params {
    pub fn frame_len(&self) -> usize {
        let mut bits = if self.rate < 22050 {
            9
        } else if self.rate < 44100 {
            10
        } else {
            11
        };
        if !self.dct && self.revision != b'b' {
            bits += self.channels.ilog2();
        }
        1 << bits
    }
    pub fn num_bands(&self) -> usize {
        let rate = if self.dct { self.rate as u64 } else { self.rate as u64 * self.channels as u64 };
        let half = rate.div_ceil(2) as u32;
        let mut n = 1;
        while n < 25 && half > CRITICAL_FREQS[n - 1] {
            n += 1;
        }
        n
    }
    /// Channels per transform and the number of blocks per frame.
    pub fn tx_channels(&self) -> usize {
        if self.dct { self.channels as usize } else { 1 }
    }
}

pub fn put_float(w: &mut BitW, rng: &mut Rng) {
    w.put(5, 10 + rng.below(12));
    w.put(23, rng.below(1 << 23));
    w.put(1, rng.below(2));
}

pub fn write_block(w: &mut BitW, rng: &mut Rng, p: &Params, channels: usize) {
    let fl = p.frame_len();
    if p.dct {
        w.put(2, 0);
    }
    for _ in 0..channels {
        if p.revision == b'b' {
            // two raw 32-bit floats of moderate size
            for _ in 0..2 {
                let v = (rng.below(2000) as f32 - 1000.0) * 8.0;
                w.put(32, v.to_bits());
            }
        } else {
            put_float(w, rng);
            put_float(w, rng);
        }
        for _ in 0..p.num_bands() {
            w.put(8, 20 + rng.below(40));
        }
        let mut i = 2;
        while i < fl {
            let j = if p.revision == b'b' {
                i + 16
            } else if rng.below(3) == 0 {
                let v = rng.below(16);
                w.put(1, 1);
                w.put(4, v);
                i + [2usize, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64][v as usize] * 8
            } else {
                w.put(1, 0);
                i + 8
            };
            let j = j.min(fl);
            let width = if rng.below(4) == 0 { 0 } else { 1 + rng.below(7) };
            w.put(4, width);
            if width != 0 {
                for _ in i..j {
                    let c = rng.below(1 << width);
                    w.put(width, c);
                    if c != 0 {
                        w.put(1, rng.below(2));
                    }
                }
            }
            i = j;
        }
        let _ = channels;
    }
    w.align32();
}

pub fn packet(rng: &mut Rng, p: &Params, blocks: usize) -> Vec<u8> {
    let mut w = BitW::default();
    // the decoded size in bytes of 16-bit samples: timestamps (and our timeline) follow it
    let fl = p.frame_len();
    let per_block = if p.dct { (fl - fl / 16) * p.channels as usize } else { fl - fl / 16 };
    w.put(32, (blocks * per_block * 2) as u32);
    for _ in 0..blocks {
        let mut done = 0;
        while done < p.tx_channels() {
            let n = (p.tx_channels() - done).min(2);
            write_block(&mut w, rng, p, n);
            done += 2;
        }
    }
    w.bytes
}

/// A frame of the current format (not `b`) in which every block is skipped: a copy of the
/// previous frame.
pub fn skip_frame(ver: u8, width: u32, height: u32) -> Vec<u8> {
    let mut w = BitW::default();
    if ver >= b'i' {
        w.put(32, 0);
    }
    for p in 0..3 {
        let chroma = p != 0;
        let pw = width >> chroma as u32;
        let (bw, bh) = if chroma { ((width + 15) >> 4, (height + 15) >> 4) } else { ((width + 7) >> 3, (height + 7) >> 3) };
        if ver == b'k' {
            w.put(1, 0);
        }
        let wa = pw.max(8).div_ceil(8) * 8;
        let l_types = log2((wa >> 3) + 511) + 1;
        let l_sub = log2((wa >> 4) + 511) + 1;
        let l_col = log2(bw * 64 + 511) + 1;
        let l_pat = log2((bw << 3) + 511) + 1;
        let l_run = log2(bw * 48 + 511) + 1;
        for i in 0..9 {
            if i == 2 {
                for _ in 0..16 {
                    w.put(4, 0);
                }
            }
            if i != 6 && i != 7 {
                w.put(4, 0);
            }
        }
        for row in 0..bh {
            w.put(l_types, if ver == b'k' { bw ^ 0xBB } else { bw });
            w.put(1, 1); // one value for the whole row
            w.put(4, 0); // skip
            if row == 0 {
                w.put(l_sub, 0);
                w.put(l_col, 0);
                w.put(l_pat, 0);
                for _ in 0..4 {
                    w.put(l_types, 0);
                }
                w.put(l_run, 0);
            }
        }
        w.align32();
    }
    w.bytes
}
