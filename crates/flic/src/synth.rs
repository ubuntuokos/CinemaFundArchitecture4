//! Synthetic FLIC streams for tests (feature `synth`): random but valid frames with every chunk
//! type (256- and 64-level palettes, byte run, line compressed, delta, black, copy, mini). FFmpeg
//! has no FLIC encoder, so this is the generator the oracle test decodes with ffmpeg and with this
//! crate.

pub struct Rng(pub u64);

impl Rng {
    pub fn rand(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn below(&mut self, n: u32) -> u32 {
        self.rand() % n.max(1)
    }
}

fn chunk(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut body = body.to_vec();
    if body.len() % 2 == 1 {
        body.push(0);
    }
    let mut c = ((body.len() + 6) as u32).to_le_bytes().to_vec();
    c.extend_from_slice(&kind.to_le_bytes());
    c.extend(body);
    c
}

fn frame(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut f = ((body.len() + 16) as u32).to_le_bytes().to_vec();
    f.extend_from_slice(&0xF1FAu16.to_le_bytes());
    f.extend_from_slice(&(chunks.len() as u16).to_le_bytes());
    f.extend_from_slice(&[0; 8]);
    f.extend(body);
    f
}

fn palette(rng: &mut Rng, six_bit: bool) -> Vec<u8> {
    let max = if six_bit { 64 } else { 256 };
    let mut packets = Vec::new();
    let mut skip_total = 0u32;
    let n = 1 + rng.below(3);
    for _ in 0..n {
        let skip = rng.below(40).min(255 - skip_total.min(255));
        skip_total += skip;
        let count = (1 + rng.below(60)).min(256 - skip_total.min(255));
        packets.push(skip as u8);
        packets.push(count as u8);
        for _ in 0..count * 3 {
            packets.push(rng.below(max) as u8);
        }
        skip_total += count;
    }
    let mut b = (n as u16).to_le_bytes().to_vec();
    b.extend(packets);
    b
}

fn full_palette() -> Vec<u8> {
    let mut b = 1u16.to_le_bytes().to_vec();
    b.extend_from_slice(&[0, 0]);
    for i in 0..256u32 {
        b.extend_from_slice(&[(i * 3 % 256) as u8, (i * 5 % 256) as u8, (255 - i) as u8]);
    }
    b
}

fn brun(rng: &mut Rng, w: usize, h: usize) -> Vec<u8> {
    let mut b = Vec::new();
    for _ in 0..h {
        b.extend_from_slice(&[0]);
        let mut x = 0;
        while x < w {
            let n = (1 + rng.below(40) as usize).min(w - x).min(127);
            if rng.below(2) == 0 {
                b.push(n as u8);
                b.push(rng.below(256) as u8);
            } else {
                b.push((-(n as i32)) as i8 as u8);
                for _ in 0..n {
                    b.push(rng.below(256) as u8);
                }
            }
            x += n;
        }
    }
    b
}

fn lc(rng: &mut Rng, w: usize, h: usize) -> Vec<u8> {
    let start = rng.below(h as u32) as usize;
    let n = 1 + rng.below((h - start) as u32) as usize;
    let mut b = (start as u16).to_le_bytes().to_vec();
    b.extend_from_slice(&(n as u16).to_le_bytes());
    for _ in 0..n {
        let packets = rng.below(5);
        b.push(packets as u8);
        let mut x = 0;
        for _ in 0..packets {
            let skip = rng.below(30) as usize;
            if x + skip >= w {
                // no room: a zero-length packet
                b.push(0);
                b.push(0);
                continue;
            }
            x += skip;
            let run = (1 + rng.below(40) as usize).min(w - x).min(127);
            b.push(skip as u8);
            if rng.below(2) == 0 {
                b.push(run as u8);
                for _ in 0..run {
                    b.push(rng.below(256) as u8);
                }
            } else {
                b.push((-(run as i32)) as i8 as u8);
                b.push(rng.below(256) as u8);
            }
            x += run;
        }
    }
    b
}

fn delta(rng: &mut Rng, w: usize, h: usize) -> Vec<u8> {
    // ffmpeg writes the "last byte" opcode at the end of its padded row: only a width that is a
    // multiple of its row alignment gives the same picture
    let last_byte = w.is_multiple_of(64);
    let mut lines = Vec::new();
    let mut count = 0u16;
    let mut y = 0usize;
    while y < h {
        if rng.below(3) == 0 {
            let k = (1 + rng.below(4) as usize).min(h - y);
            lines.extend_from_slice(&(-(k as i32) as i16).to_le_bytes());
            y += k;
            if y >= h {
                break;
            }
        }
        if last_byte && rng.below(4) == 0 {
            lines.extend_from_slice(&(0x8000u16 | rng.below(256) as u16).to_le_bytes());
        }
        let packets = rng.below(4);
        lines.extend_from_slice(&(packets as u16).to_le_bytes());
        let mut x = 0;
        for _ in 0..packets {
            let skip = rng.below(30) as usize;
            if x + skip + 2 > w {
                lines.push(0);
                lines.push(0);
                continue;
            }
            x += skip;
            let words = (1 + rng.below(20) as usize).min((w - x) / 2).min(127);
            lines.push(skip as u8);
            if rng.below(2) == 0 {
                lines.push(words as u8);
                for _ in 0..words * 2 {
                    lines.push(rng.below(256) as u8);
                }
            } else {
                lines.push((-(words as i32)) as i8 as u8);
                lines.push(rng.below(256) as u8);
                lines.push(rng.below(256) as u8);
            }
            x += words * 2;
        }
        count += 1;
        y += 1;
    }
    let mut b = count.to_le_bytes().to_vec();
    b.extend(lines);
    b
}

fn copy(rng: &mut Rng, w: usize, h: usize) -> Vec<u8> {
    let stride = (w + 3) & !3;
    (0..stride * h).map(|_| rng.below(256) as u8).collect()
}

fn header(magic: u16, w: usize, h: usize, frames: usize, speed: u32, depth: u16) -> Vec<u8> {
    let mut hd = vec![0u8; 128];
    hd[4..6].copy_from_slice(&magic.to_le_bytes());
    hd[6..8].copy_from_slice(&(frames as u16).to_le_bytes());
    hd[8..10].copy_from_slice(&(w as u16).to_le_bytes());
    hd[10..12].copy_from_slice(&(h as u16).to_le_bytes());
    hd[12..14].copy_from_slice(&depth.to_le_bytes());
    hd[16..20].copy_from_slice(&speed.to_le_bytes());
    hd
}

/// A random valid stream of `frames` frames of `w`x`h` pixels.
pub fn stream(seed: u64, w: usize, h: usize, frames: usize, magic: u16) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = header(magic, w, h, frames, 5, 8);
    let mut data = Vec::new();
    for f in 0..frames {
        let mut chunks = Vec::new();
        if f == 0 {
            chunks.push(chunk(11, &full_palette()));
            chunks.push(chunk(15, &brun(&mut rng, w, h)));
        } else {
            match rng.below(9) {
                0 => chunks.push(chunk(13, &[])),
                1 => chunks.push(chunk(16, &copy(&mut rng, w, h))),
                2 => chunks.push(chunk(12, &lc(&mut rng, w, h))),
                3 | 4 => chunks.push(chunk(7, &delta(&mut rng, w, h))),
                5 => chunks.push(chunk(15, &brun(&mut rng, w, h))),
                _ => {
                    chunks.push(chunk(if rng.below(2) == 0 { 4 } else { 11 }, &palette(&mut rng, true)));
                    if rng.below(2) == 0 {
                        chunks.push(chunk(12, &lc(&mut rng, w, h)));
                    } else {
                        chunks.push(chunk(7, &delta(&mut rng, w, h)));
                    }
                }
            }
            if rng.below(6) == 0 {
                chunks.push(chunk(18, &[1, 2, 3, 4]));
            }
        }
        data.extend(frame(&chunks));
    }
    out.extend(data);
    out
}

/// The frames of a stream as the demuxer packets them.
pub fn packets(file: &[u8], h: &crate::Header) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut pos = h.first_chunk;
    while pos + 6 <= file.len() {
        let size = u32::from_le_bytes([file[pos], file[pos + 1], file[pos + 2], file[pos + 3]]) as usize;
        let magic = u16::from_le_bytes([file[pos + 4], file[pos + 5]]);
        if (magic == 0xF1FA || magic == 0xF5FA) && size > 6 {
            let end = (pos + size).min(file.len());
            v.push(file[pos..end].to_vec());
            pos += size;
        } else {
            pos += size.max(6);
        }
    }
    v
}
