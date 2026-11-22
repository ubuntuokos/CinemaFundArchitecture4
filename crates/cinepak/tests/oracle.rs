//! The Cinepak decoder against ffmpeg's `cinepak` decoder (the external oracle).
//!
//! Streams come from two sources: a small random-stream writer here (every chunk id, partial
//! codebooks, 4- and 6-element vectors, skipped blocks, several strips with absolute and relative
//! y, palette video, sizes that are no multiple of 4) and ffmpeg's own Cinepak encoder. The
//! pictures must equal ffmpeg's byte for byte. Damaged frames must never panic.

use std::path::Path;
use std::process::Command;

use cfa4_cinepak::{Decoder, Format};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n.max(1)
    }
    fn byte(&mut self) -> u8 {
        self.below(256) as u8
    }
}

/// Writes the lazily loaded 32-bit flag words of a chunk, interleaved with its data.
struct Flags {
    slot: usize,
    word: u32,
    mask: u32,
}

impl Flags {
    fn new() -> Self {
        Self { slot: 0, word: 0, mask: 0 }
    }
    /// One flag bit (mirrors `!(mask >>= 1)` of the decoder).
    fn bit(&mut self, out: &mut Vec<u8>, b: bool) {
        self.mask >>= 1;
        if self.mask == 0 {
            self.slot = out.len();
            out.extend_from_slice(&[0; 4]);
            self.mask = 0x8000_0000;
            self.word = 0;
        }
        if b {
            self.word |= self.mask;
        }
        out[self.slot..self.slot + 4].copy_from_slice(&self.word.to_be_bytes());
    }
}

fn chunk(id: u8, body: &[u8]) -> Vec<u8> {
    let n = body.len() as u32 + 4;
    [vec![id, (n >> 16) as u8, (n >> 8) as u8, n as u8], body.to_vec()].concat()
}

/// A codebook chunk: id 0x20/0x22 (full) or 0x21/0x23 (partial), 0x24-0x27 the 4-element forms.
fn codebook(rng: &mut Rng, id: u8, entries: usize) -> Vec<u8> {
    let n = if id & 4 != 0 { 4 } else { 6 };
    let mut out = Vec::new();
    let mut fl = Flags::new();
    for _ in 0..entries {
        let present = id & 1 == 0 || rng.below(3) != 0;
        if id & 1 != 0 {
            fl.bit(&mut out, present);
        }
        if present {
            for _ in 0..n {
                out.push(rng.byte());
            }
        }
    }
    chunk(id, &out)
}

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Skip,
    V1,
    V4,
}

/// A vector chunk of kind `id` (0x30 intra, 0x31 inter, 0x32 all V1) for `blocks` blocks.
fn vectors(rng: &mut Rng, id: u8, blocks: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut fl = Flags::new();
    for _ in 0..blocks {
        let kind = match id {
            0x30 => {
                if rng.below(2) == 0 {
                    Block::V1
                } else {
                    Block::V4
                }
            }
            0x31 => [Block::Skip, Block::V1, Block::V4][rng.below(3) as usize],
            _ => Block::V1,
        };
        if id & 1 != 0 {
            fl.bit(&mut out, kind != Block::Skip);
        }
        if kind != Block::Skip {
            if id & 2 == 0 {
                fl.bit(&mut out, kind == Block::V4);
            }
            if kind == Block::V1 {
                out.push(rng.byte());
            } else {
                (0..4).for_each(|_| out.push(rng.byte()));
            }
        }
    }
    chunk(id, &out)
}

/// One frame of random strips. `first`: every block is coded and every codebook complete.
fn frame(rng: &mut Rng, w: usize, h: usize, first: bool) -> Vec<u8> {
    let (cw, ch) = ((w + 3) & !3, (h + 3) & !3);
    let nstrips = 1 + rng.below(3) as usize;
    let rows = ch / 4;
    let per = rows.div_ceil(nstrips);
    let flags = if rng.below(2) == 0 { 1u8 } else { 0 };
    let mut strips = Vec::new();
    let mut y = 0;
    let mut count = 0;
    while y < ch {
        let y2 = (y + per * 4).min(ch);
        let blocks = (cw / 4) * ((y2 - y) / 4);
        let mut body = Vec::new();
        // codebooks: the first strip (or any strip when flags says so) carries them; the first
        // frame needs complete ones for every strip it reads from
        let own = first || flags & 1 != 0 || count == 0;
        if own || rng.below(2) == 0 {
            let id4 = if first { [0x20, 0x24][rng.below(2) as usize] } else { [0x20, 0x21, 0x24, 0x25][rng.below(4) as usize] };
            let id1 = if first { [0x22, 0x26][rng.below(2) as usize] } else { [0x22, 0x23, 0x26, 0x27][rng.below(4) as usize] };
            body.extend(codebook(rng, id4, 256));
            body.extend(codebook(rng, id1, 256));
        }
        let vid = if first { [0x30, 0x32][rng.below(2) as usize] } else { [0x30, 0x31, 0x32][rng.below(3) as usize] };
        body.extend(vectors(rng, vid, blocks));
        // some strips with y1 = 0 meaning "after the previous strip" (the field then holds the height)
        let rel = count > 0 && rng.below(2) == 0;
        let (y1f, y2f) = if rel { (0, y2 - y) } else { (y, y2) };
        let (y1f, y2f) = if count == 0 { (0, y2 - y) } else { (y1f, y2f) };
        let size = body.len() + 12;
        let mut s = vec![if first || count == 0 { 0x10 } else { 0x11 }, (size >> 16) as u8, (size >> 8) as u8, size as u8];
        for v in [y1f, 0, y2f, cw] {
            s.extend_from_slice(&(v as u16).to_be_bytes());
        }
        s.extend(body);
        strips.push(s);
        y = y2;
        count += 1;
    }
    let payload: Vec<u8> = strips.concat();
    let total = 10 + payload.len();
    let mut f = vec![flags, (total >> 16) as u8, (total >> 8) as u8, total as u8];
    f.extend_from_slice(&(w as u16).to_be_bytes());
    f.extend_from_slice(&(h as u16).to_be_bytes());
    f.extend_from_slice(&(count as u16).to_be_bytes());
    f.extend(payload);
    f
}

fn avi(w: u32, h: u32, bit_count: u16, palette: &[u8], frames: &[Vec<u8>]) -> Vec<u8> {
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = kind.to_vec();
        b.extend_from_slice(body);
        chunk(b"LIST", &b)
    }
    let le = |v: u32| v.to_le_bytes();
    let n = frames.len() as u32;
    let maxsize = frames.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let mut avih = Vec::new();
    for v in [40_000, 0, 0, 0x10, n, 0, 1, maxsize, w, h, 0, 0, 0, 0] {
        avih.extend_from_slice(&le(v));
    }
    let mut strh = b"vidscvid".to_vec();
    for v in [0, 0, 0, 1, 25, 0, n, maxsize, u32::MAX, 0] {
        strh.extend_from_slice(&le(v));
    }
    strh.extend_from_slice(&[0u8; 8]);
    let mut strf = Vec::new();
    for v in [40 + palette.len() as u32, w, h] {
        strf.extend_from_slice(&le(v));
    }
    strf.extend_from_slice(&1u16.to_le_bytes());
    strf.extend_from_slice(&bit_count.to_le_bytes());
    strf.extend_from_slice(b"cvid");
    for v in [w * h * 3, 0, 0, (palette.len() / 4) as u32, 0] {
        strf.extend_from_slice(&le(v));
    }
    strf.extend_from_slice(palette);
    let strl = list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat());
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), strl].concat());
    let (mut movi_body, mut idx) = (Vec::new(), Vec::new());
    for f in frames {
        idx.extend_from_slice(&[b"00dc".as_slice(), &le(0x10), &le(4 + movi_body.len() as u32), &le(f.len() as u32)].concat());
        movi_body.extend_from_slice(&chunk(b"00dc", f));
    }
    chunk(b"RIFF", &[b"AVI ".to_vec(), hdrl, list(b"movi", &movi_body), chunk(b"idx1", &idx)].concat())
}

/// The `00dc` chunks of an AVI file, in order.
fn video_chunks(file: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= file.len() {
        let size = u32::from_le_bytes(file[i + 4..i + 8].try_into().unwrap()) as usize;
        if &file[i..i + 4] == b"LIST" {
            i += 12;
            continue;
        }
        if &file[i + 2..i + 4] == b"dc" {
            out.push(file[i + 8..i + 8 + size].to_vec());
        }
        i += 8 + size + (size & 1);
    }
    out
}

fn ffmpeg_decode(ff: &Path, file: &[u8], tag: &str) -> Vec<u8> {
    let dir = cfa4_testkit::fixtures::fixtures_dir("cinepak");
    let path = dir.join(format!("{tag}.avi"));
    std::fs::write(&path, file).expect("write avi");
    let out = Command::new(ff)
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(&path)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("run ffmpeg");
    out.stdout
}

fn rgb(p: &cfa4_cinepak::Picture) -> Vec<u8> {
    p.to_rgba().chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect()
}

fn check(ff: &Path, tag: &str, w: u32, h: u32, bit_count: u16, palette: &[u8], frames: &[Vec<u8>]) {
    let mut dec = Decoder::new(w, h, bit_count).expect("decoder");
    dec.set_palette_bgr0(palette);
    let mut ours = Vec::new();
    for (k, f) in frames.iter().enumerate() {
        let p = dec.decode(f).unwrap_or_else(|e| panic!("{tag} frame {k}: {e}"));
        assert!(!p.damaged, "{tag} frame {k} is damaged");
        assert_eq!(p.format, if bit_count == 8 { Format::Pal8 } else { Format::Rgb24 });
        ours.extend(rgb(&p));
    }
    let theirs = ffmpeg_decode(ff, &avi(w, h, bit_count, palette, frames), tag);
    assert!(!theirs.is_empty(), "{tag}: ffmpeg decoded nothing");
    assert_eq!(theirs.len(), ours.len(), "{tag}: ffmpeg decoded {} bytes, we {}", theirs.len(), ours.len());
    if let Some(i) = theirs.iter().zip(&ours).position(|(a, b)| a != b) {
        panic!("{tag}: differs from ffmpeg at byte {i} (frame {})", i / (w as usize * h as usize * 3));
    }
}

#[test]
fn random_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cinepak oracle") else { return };
    let mut rng = Rng(0xc1_4e_9a_cc);
    for (w, h) in [(64usize, 48usize), (37, 23), (16, 16), (30, 30), (5, 3)] {
        for round in 0..3 {
            let frames: Vec<Vec<u8>> = (0..8).map(|k| frame(&mut rng, w, h, k == 0)).collect();
            check(&ff, &format!("rgb_{w}x{h}_{round}"), w as u32, h as u32, 24, &[], &frames);
        }
    }
}

#[test]
fn palette_streams_match_ffmpeg() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cinepak palette oracle") else { return };
    let mut rng = Rng(0x9a1e_77e5);
    let palette: Vec<u8> = (0..256u32).flat_map(|_| [rng.byte(), rng.byte(), rng.byte(), 0]).collect();
    for (w, h) in [(48usize, 32usize), (26, 14)] {
        let frames: Vec<Vec<u8>> = (0..6).map(|k| frame(&mut rng, w, h, k == 0)).collect();
        check(&ff, &format!("pal_{w}x{h}"), w as u32, h as u32, 8, &palette, &frames);
    }
}

/// Streams written by ffmpeg's own Cinepak encoder (several strips, intra and inter frames).
#[test]
fn ffmpeg_encoded_streams_decode_the_same() {
    let Some(ff) = cfa4_testkit::oracle::ffmpeg_or_skip("cinepak encoder oracle") else { return };
    let dir = cfa4_testkit::fixtures::fixtures_dir("cinepak");
    for (name, size, extra) in
        [("enc_a", "96x64", vec!["-g", "5"]), ("enc_b", "60x36", vec!["-g", "3", "-q:v", "9"]), ("enc_c", "128x96", vec!["-strips", "4"])]
    {
        let path = dir.join(format!("{name}.avi"));
        let st = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size={size}:rate=10:duration=2"))
            .args(["-c:v", "cinepak"])
            .args(&extra)
            .arg(&path)
            .status()
            .expect("run ffmpeg");
        if !st.success() {
            eprintln!("ffmpeg has no usable cinepak encoder, skipping {name}");
            return;
        }
        let file = std::fs::read(&path).unwrap();
        let (w, h) = size.split_once('x').map(|(a, b)| (a.parse::<u32>().unwrap(), b.parse::<u32>().unwrap())).unwrap();
        let frames = video_chunks(&file);
        assert!(frames.len() >= 10, "{name}: {} packets", frames.len());
        let mut dec = Decoder::new(w, h, 24).unwrap();
        let ours: Vec<u8> = frames.iter().flat_map(|f| rgb(&dec.decode(f).unwrap())).collect();
        let theirs = ffmpeg_decode(&ff, &file, name);
        assert_eq!(theirs.len(), ours.len(), "{name}");
        assert!(theirs == ours, "{name}: differs from ffmpeg");
    }
}

#[test]
fn damaged_frames_never_panic() {
    let mut rng = Rng(0xdead_cafe);
    for (w, h, bits) in [(20usize, 12usize, 24u16), (21, 13, 8)] {
        let frames: Vec<Vec<u8>> = (0..4).map(|k| frame(&mut rng, w, h, k == 0)).collect();
        for good in &frames {
            for _ in 0..400 {
                let mut bad = good.clone();
                match rng.below(3) {
                    0 => bad.truncate(rng.below(bad.len() as u32) as usize),
                    1 => {
                        for _ in 0..1 + rng.below(4) {
                            let i = rng.below(bad.len() as u32) as usize;
                            bad[i] = rng.byte();
                        }
                    }
                    _ => {
                        let i = rng.below(24.min(bad.len() as u32)) as usize;
                        bad[i] = rng.byte();
                    }
                }
                let mut dec = Decoder::new(w as u32, h as u32, bits).unwrap();
                let _ = dec.decode(&bad);
                let _ = dec.decode(good);
            }
        }
    }
}
