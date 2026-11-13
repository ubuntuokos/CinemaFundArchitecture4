//! OpenEXR PIZ compression (compression code 4, 32 scan lines per chunk): the 16-bit words of a
//! chunk are remapped through a table of the values that occur, transformed with a 2D Haar-like
//! wavelet per channel and Huffman coded with a canonical code whose lengths are stored run-length
//! packed. Lossless for every pixel type (a 32-bit sample is two interleaved 16-bit words).
//!
//! Ported from OpenEXR 3.1.13 (`src/lib/OpenEXRCore/internal_piz.c`, `internal_huf.c`, the classic
//! table decoder, not the fast one), BSD-3-Clause, Copyright Contributors to the OpenEXR Project;
//! see `docs/licensing.md` §5. The byte layout of a chunk:
//!
//! ```text
//! u16 minNonZero, u16 maxNonZero         first and last non-zero byte of the value bitmap
//! bitmap[minNonZero..=maxNonZero]        bit v set = value v occurs (0 is implied)
//! u32 huffman length
//! u32 im, u32 iM, u32 table length, u32 bit count, u32 0
//! packed code lengths of symbols im..=iM (6 bits each; 59–62 = 2–5 zeros; 63 + 8 bits = 6–261)
//! code bits, most significant first   (symbol iM is the run code: the next 8 bits repeat the
//!                                      previous word that many times)
//! ```
//!
//! All integers are little-endian. `decompress` and `compress` take and give the uncompressed chunk
//! in the file's scan-line layout: for each line, each channel's samples of that line.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// One channel of a chunk: samples per line, lines, and bytes per sample (2 for half, 4 for
/// float and uint).
#[derive(Clone, Copy, Debug)]
pub struct PizChannel {
    pub width: usize,
    pub lines: usize,
    pub bytes: usize,
}

const USHORT_RANGE: usize = 1 << 16;
const BITMAP_SIZE: usize = USHORT_RANGE >> 3;
const HUF_ENCSIZE: usize = (1 << 16) + 1;
const HUF_DECBITS: u32 = 14;
const HUF_DECSIZE: usize = 1 << HUF_DECBITS;
const HUF_DECMASK: u64 = (HUF_DECSIZE - 1) as u64;
const SHORT_ZEROCODE_RUN: u64 = 59;
const LONG_ZEROCODE_RUN: u64 = 63;
const SHORTEST_LONG_RUN: u64 = 2 + LONG_ZEROCODE_RUN - SHORT_ZEROCODE_RUN;
const LONGEST_LONG_RUN: u64 = 255 + SHORTEST_LONG_RUN;
/// Longest Huffman code the format allows.
const MAX_CODE_LEN: u64 = 58;

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The size of the uncompressed chunk, in bytes.
fn raw_len(channels: &[PizChannel]) -> Option<usize> {
    channels.iter().try_fold(0usize, |t, c| t.checked_add(c.width.checked_mul(c.lines)?.checked_mul(c.bytes)?))
}

/// Decompress one PIZ chunk into `out_len` bytes of scan-line data.
pub fn decompress(data: &[u8], channels: &[PizChannel], out_len: usize) -> Result<Vec<u8>, String> {
    if channels.iter().any(|c| c.bytes % 2 != 0) || raw_len(channels) != Some(out_len) {
        return Err("PIZ: channel layout does not match the chunk size".into());
    }
    let corrupt = |what: &str| format!("PIZ: {what}");
    let min_nz = usize::from(le16(data, 0).ok_or_else(|| corrupt("truncated range"))?);
    let max_nz = usize::from(le16(data, 2).ok_or_else(|| corrupt("truncated range"))?);
    if max_nz >= BITMAP_SIZE {
        return Err(corrupt("bad value range"));
    }
    let mut bitmap = vec![0u8; BITMAP_SIZE];
    let mut pos = 4;
    if min_nz <= max_nz {
        let n = max_nz - min_nz + 1;
        let src = data.get(pos..pos + n).ok_or_else(|| corrupt("truncated bitmap"))?;
        bitmap[min_nz..=max_nz].copy_from_slice(src);
        pos += n;
    }
    let (lut, max_value) = reverse_lut(&bitmap);
    let huf_len = le32(data, pos).ok_or_else(|| corrupt("truncated length"))? as usize;
    pos += 4;
    let huf = data.get(pos..pos.checked_add(huf_len).ok_or_else(|| corrupt("bad length"))?).ok_or_else(|| corrupt("truncated Huffman data"))?;
    let mut words = vec![0u16; out_len / 2];
    huf_decompress(huf, &mut words).map_err(corrupt)?;

    let mut base = 0;
    for c in channels {
        let wcount = c.bytes / 2;
        let n = c.width * c.lines * wcount;
        let plane = &mut words[base..base + n];
        for j in 0..wcount {
            wav_2d_decode(plane, j, c.width, wcount, c.lines, wcount * c.width, max_value);
        }
        base += n;
    }
    for w in &mut words {
        *w = lut[usize::from(*w)];
    }

    // planar per channel → scan-line layout
    let mut out = vec![0u8; out_len];
    let lines = channels.iter().map(|c| c.lines).max().unwrap_or(0);
    let mut o = 0;
    for y in 0..lines {
        let mut base = 0;
        for c in channels {
            let per_line = c.width * c.bytes / 2;
            if y < c.lines {
                for &w in &words[base + y * per_line..base + (y + 1) * per_line] {
                    out[o..o + 2].copy_from_slice(&w.to_le_bytes());
                    o += 2;
                }
            }
            base += per_line * c.lines;
        }
    }
    Ok(out)
}

/// Compress one chunk of scan-line data (the layout `decompress` returns). The caller stores the
/// chunk uncompressed when the result is not smaller.
pub fn compress(raw: &[u8], channels: &[PizChannel]) -> Result<Vec<u8>, String> {
    if channels.iter().any(|c| c.bytes % 2 != 0) || raw_len(channels) != Some(raw.len()) {
        return Err("PIZ: channel layout does not match the chunk size".into());
    }
    // scan-line layout → planar per channel, native 16-bit words
    let mut words = vec![0u16; raw.len() / 2];
    let lines = channels.iter().map(|c| c.lines).max().unwrap_or(0);
    let mut i = 0;
    for y in 0..lines {
        let mut base = 0;
        for c in channels {
            let per_line = c.width * c.bytes / 2;
            if y < c.lines {
                for w in &mut words[base + y * per_line..base + (y + 1) * per_line] {
                    *w = u16::from_le_bytes([raw[i], raw[i + 1]]);
                    i += 2;
                }
            }
            base += per_line * c.lines;
        }
    }

    let mut bitmap = vec![0u8; BITMAP_SIZE];
    for &w in &words {
        bitmap[usize::from(w >> 3)] |= 1 << (w & 7);
    }
    bitmap[0] &= !1; // zero is implied
    let min_nz = bitmap.iter().position(|&b| b != 0).unwrap_or(BITMAP_SIZE - 1);
    let max_nz = bitmap.iter().rposition(|&b| b != 0).unwrap_or(0);
    let (lut, max_value) = forward_lut(&bitmap);
    for w in &mut words {
        *w = lut[usize::from(*w)];
    }

    let mut out = Vec::with_capacity(raw.len() / 2 + 64);
    out.extend_from_slice(&(min_nz as u16).to_le_bytes());
    out.extend_from_slice(&(max_nz as u16).to_le_bytes());
    if min_nz <= max_nz {
        out.extend_from_slice(&bitmap[min_nz..=max_nz]);
    }

    let mut base = 0;
    for c in channels {
        let wcount = c.bytes / 2;
        let n = c.width * c.lines * wcount;
        let plane = &mut words[base..base + n];
        for j in 0..wcount {
            wav_2d_encode(plane, j, c.width, wcount, c.lines, wcount * c.width, max_value);
        }
        base += n;
    }

    let huf = huf_compress(&words)?;
    let len = u32::try_from(huf.len()).map_err(|_| "PIZ: chunk too large".to_string())?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&huf);
    Ok(out)
}

/// value → index among the values that occur (0 always occurs); returns the table and the
/// largest index.
fn forward_lut(bitmap: &[u8]) -> (Vec<u16>, u16) {
    let mut lut = vec![0u16; USHORT_RANGE];
    let mut k = 0u32;
    for (i, v) in lut.iter_mut().enumerate() {
        if i == 0 || bitmap[i >> 3] & (1 << (i & 7)) != 0 {
            *v = k as u16;
            k += 1;
        }
    }
    (lut, (k - 1) as u16)
}

/// index → value, the inverse of `forward_lut`; returns the table and the largest index.
fn reverse_lut(bitmap: &[u8]) -> (Vec<u16>, u16) {
    let mut lut = vec![0u16; USHORT_RANGE];
    let mut k = 0usize;
    for i in 0..USHORT_RANGE {
        if i == 0 || bitmap[i >> 3] & (1 << (i & 7)) != 0 {
            lut[k] = i as u16;
            k += 1;
        }
    }
    (lut, (k - 1) as u16)
}

// ---- wavelet ----------------------------------------------------------------------------------

/// The 14-bit basis (no modulo arithmetic), used when every value is below 1 << 14.
fn wenc14(a: u16, b: u16) -> (u16, u16) {
    let (a, b) = (i32::from(a as i16), i32::from(b as i16));
    (((a + b) >> 1) as u16, (a - b) as u16)
}

fn wdec14(l: u16, h: u16) -> (u16, u16) {
    let (l, h) = (i32::from(l as i16), i32::from(h as i16));
    let a = l + (h & 1) + (h >> 1);
    (a as i16 as u16, (a - h) as i16 as u16)
}

const A_OFFSET: i32 = 1 << 15;
const MOD_MASK: i32 = (1 << 16) - 1;

/// The 16-bit basis (modulo arithmetic) for larger values.
fn wenc16(a: u16, b: u16) -> (u16, u16) {
    let ao = (i32::from(a) + A_OFFSET) & MOD_MASK;
    let mut m = (ao + i32::from(b)) >> 1;
    let d = ao - i32::from(b);
    if d < 0 {
        m = (m + A_OFFSET) & MOD_MASK;
    }
    (m as u16, (d & MOD_MASK) as u16)
}

fn wdec16(l: u16, h: u16) -> (u16, u16) {
    let (m, d) = (i32::from(l), i32::from(h));
    let b = (m - (d >> 1)) & MOD_MASK;
    let a = (d + b - A_OFFSET) & MOD_MASK;
    (a as u16, b as u16)
}

/// Forward transform of the `nx` × `ny` samples at `buf[start..]` (x step `ox`, y step `oy`).
fn wav_2d_encode(buf: &mut [u16], start: usize, nx: usize, ox: usize, ny: usize, oy: usize, mx: u16) {
    let enc = if mx < 1 << 14 { wenc14 } else { wenc16 };
    let n = nx.min(ny);
    let (mut p, mut p2) = (1usize, 2usize);
    while p2 <= n {
        let (oy1, oy2, ox1, ox2) = (oy * p, oy * p2, ox * p, ox * p2);
        let ey = start + oy * (ny - p2);
        let mut py = start;
        while py <= ey {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let (p01, p10) = (px + ox1, px + oy1);
                let p11 = p10 + ox1;
                let (i00, i01) = enc(buf[px], buf[p01]);
                let (i10, i11) = enc(buf[p10], buf[p11]);
                (buf[px], buf[p10]) = enc(i00, i10);
                (buf[p01], buf[p11]) = enc(i01, i11);
                px += ox2;
            }
            if nx & p != 0 {
                let p10 = px + oy1;
                (buf[px], buf[p10]) = enc(buf[px], buf[p10]);
            }
            py += oy2;
        }
        if ny & p != 0 {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let p01 = px + ox1;
                (buf[px], buf[p01]) = enc(buf[px], buf[p01]);
                px += ox2;
            }
        }
        p = p2;
        p2 <<= 1;
    }
}

/// Inverse of `wav_2d_encode`.
fn wav_2d_decode(buf: &mut [u16], start: usize, nx: usize, ox: usize, ny: usize, oy: usize, mx: u16) {
    let dec = if mx < 1 << 14 { wdec14 } else { wdec16 };
    let n = nx.min(ny);
    let mut p = 1usize;
    while p <= n {
        p <<= 1;
    }
    p >>= 1;
    let mut p2 = p;
    p >>= 1;
    while p >= 1 {
        let (oy1, oy2, ox1, ox2) = (oy * p, oy * p2, ox * p, ox * p2);
        let ey = start + oy * (ny - p2);
        let mut py = start;
        while py <= ey {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let (p01, p10) = (px + ox1, px + oy1);
                let p11 = p10 + ox1;
                let (i00, i10) = dec(buf[px], buf[p10]);
                let (i01, i11) = dec(buf[p01], buf[p11]);
                (buf[px], buf[p01]) = dec(i00, i01);
                (buf[p10], buf[p11]) = dec(i10, i11);
                px += ox2;
            }
            if nx & p != 0 {
                let p10 = px + oy1;
                (buf[px], buf[p10]) = dec(buf[px], buf[p10]);
            }
            py += oy2;
        }
        if ny & p != 0 {
            let ex = py + ox * (nx - p2);
            let mut px = py;
            while px <= ex {
                let p01 = px + ox1;
                (buf[px], buf[p01]) = dec(buf[px], buf[p01]);
                px += ox2;
            }
        }
        p2 = p;
        p >>= 1;
    }
}

// ---- Huffman ----------------------------------------------------------------------------------

/// A code table entry: `code << 6 | length`.
fn hlen(code: u64) -> u32 {
    (code & 63) as u32
}

fn hcode_of(code: u64) -> u64 {
    code >> 6
}

/// Turn code lengths into canonical codes: shorter codes are numerically higher, codes of one
/// length increase with the symbol.
fn canonical_codes(hcode: &mut [u64]) {
    let mut n = [0u64; 59];
    for &l in hcode.iter() {
        n[l as usize] += 1;
    }
    let mut c = 0u64;
    for i in (1..=58).rev() {
        let nc = (c + n[i]) >> 1;
        n[i] = c;
        c = nc;
    }
    for h in hcode.iter_mut() {
        let l = *h;
        if l > 0 {
            *h = l | (n[l as usize] << 6);
            n[l as usize] += 1;
        }
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    c: u64,
    lc: u32,
}

impl BitReader<'_> {
    fn bits(&mut self, n: u32) -> Option<u64> {
        while self.lc < n {
            self.c = (self.c << 8) | u64::from(*self.data.get(self.pos)?);
            self.pos += 1;
            self.lc += 8;
        }
        self.lc -= n;
        Some((self.c >> self.lc) & ((1 << n) - 1))
    }
}

/// Read the packed code lengths of symbols `im..=i_max` and make the canonical code table.
fn unpack_table(r: &mut BitReader, im: usize, i_max: usize) -> Result<Vec<u64>, &'static str> {
    let mut hcode = vec![0u64; HUF_ENCSIZE];
    let mut i = im;
    while i <= i_max {
        let l = r.bits(6).ok_or("truncated code table")?;
        if l == LONG_ZEROCODE_RUN {
            let run = (r.bits(8).ok_or("truncated code table")? + SHORTEST_LONG_RUN) as usize;
            if i + run > i_max + 1 {
                return Err("bad code table");
            }
            i += run;
        } else if l >= SHORT_ZEROCODE_RUN {
            let run = (l - SHORT_ZEROCODE_RUN + 2) as usize;
            if i + run > i_max + 1 {
                return Err("bad code table");
            }
            i += run;
        } else {
            hcode[i] = l;
            i += 1;
        }
    }
    canonical_codes(&mut hcode);
    Ok(hcode)
}

#[derive(Clone, Default)]
struct Dec {
    /// Length of the short code filling this slot (0 = none).
    len: u32,
    lit: u32,
    /// Symbols of the long codes whose first 14 bits are this slot.
    long: Vec<u32>,
}

fn build_dec_table(hcode: &[u64], im: usize, i_max: usize) -> Result<Vec<Dec>, &'static str> {
    let mut t = vec![Dec::default(); HUF_DECSIZE];
    for (sym, &h) in hcode.iter().enumerate().take(i_max + 1).skip(im) {
        let (c, l) = (hcode_of(h), hlen(h));
        if l == 0 {
            continue;
        }
        if c >> l != 0 {
            return Err("bad code");
        }
        if l > HUF_DECBITS {
            let d = &mut t[(c >> (l - HUF_DECBITS)) as usize];
            if d.len != 0 {
                return Err("bad code table");
            }
            d.long.push(sym as u32);
        } else {
            let first = (c << (HUF_DECBITS - l)) as usize;
            for d in &mut t[first..first + (1 << (HUF_DECBITS - l))] {
                if d.len != 0 || !d.long.is_empty() {
                    return Err("bad code table");
                }
                d.len = l;
                d.lit = sym as u32;
            }
        }
    }
    Ok(t)
}

/// Emit a decoded symbol: the run code repeats the previous word.
fn put(sym: u32, rlc: u32, r: &mut BitReader, out: &mut [u16], o: &mut usize) -> Result<(), &'static str> {
    if sym == rlc {
        let run = r.bits(8).ok_or("truncated run")? as usize;
        if *o == 0 {
            return Err("run with nothing before it");
        }
        if *o + run > out.len() {
            return Err("too much data");
        }
        let prev = out[*o - 1];
        out[*o..*o + run].fill(prev);
        *o += run;
    } else {
        *out.get_mut(*o).ok_or("too much data")? = sym as u16;
        *o += 1;
    }
    Ok(())
}

/// Decode the Huffman block of a PIZ chunk into exactly `out.len()` words.
fn huf_decompress(data: &[u8], out: &mut [u16]) -> Result<(), &'static str> {
    if data.len() < 20 {
        return if out.is_empty() { Ok(()) } else { Err("truncated Huffman header") };
    }
    let rd = |at| le32(data, at).map(|v| v as usize).ok_or("truncated Huffman header");
    let (im, i_max, n_bits) = (rd(0)?, rd(4)?, rd(12)?);
    if im >= HUF_ENCSIZE || i_max >= HUF_ENCSIZE || im > i_max {
        return Err("bad symbol range");
    }
    let body = &data[20..];
    let mut r = BitReader { data: body, pos: 0, c: 0, lc: 0 };
    let hcode = unpack_table(&mut r, im, i_max)?;
    let table = build_dec_table(&hcode, im, i_max)?;
    let start = r.pos;
    let n_bytes = n_bits.div_ceil(8);
    let code = body.get(start..start.checked_add(n_bytes).ok_or("bad bit count")?).ok_or("truncated code bits")?;
    let rlc = i_max as u32;
    let mut r = BitReader { data: code, pos: 0, c: 0, lc: 0 };
    let mut o = 0usize;
    while r.pos < code.len() {
        r.c = (r.c << 8) | u64::from(code[r.pos]);
        r.pos += 1;
        r.lc += 8;
        while r.lc >= HUF_DECBITS {
            let d = &table[((r.c >> (r.lc - HUF_DECBITS)) & HUF_DECMASK) as usize];
            if d.len != 0 {
                r.lc -= d.len;
                let sym = d.lit;
                put(sym, rlc, &mut r, out, &mut o)?;
            } else {
                if d.long.is_empty() {
                    return Err("bad code");
                }
                let mut found = None;
                for &sym in &d.long {
                    let h = hcode[sym as usize];
                    let l = hlen(h);
                    while r.lc < l && r.pos < code.len() && r.lc <= 56 {
                        r.c = (r.c << 8) | u64::from(code[r.pos]);
                        r.pos += 1;
                        r.lc += 8;
                    }
                    if r.lc >= l && hcode_of(h) == (r.c >> (r.lc - l)) & ((1u64 << l) - 1) {
                        r.lc -= l;
                        found = Some(sym);
                        break;
                    }
                }
                let sym = found.ok_or("bad long code")?;
                put(sym, rlc, &mut r, out, &mut o)?;
            }
        }
    }
    // the last byte's padding bits, then the remaining short codes
    let pad = ((8 - n_bits % 8) % 8) as u32;
    if r.lc < pad {
        return Err("bad bit count");
    }
    r.c >>= pad;
    r.lc -= pad;
    while r.lc > 0 {
        let d = &table[((r.c << (HUF_DECBITS - r.lc)) & HUF_DECMASK) as usize];
        if d.len == 0 || d.len > r.lc {
            return Err("bad code at the end");
        }
        r.lc -= d.len;
        let sym = d.lit;
        put(sym, rlc, &mut r, out, &mut o)?;
    }
    if o != out.len() { Err("wrong amount of data") } else { Ok(()) }
}

/// Code lengths of a Huffman code for `freq` (symbols with frequency 0 get none).
fn code_lengths(freq: &[u64]) -> Vec<u64> {
    let mut frq = freq.to_vec();
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = frq.iter().enumerate().filter(|(_, f)| **f > 0).map(|(i, &f)| Reverse((f, i))).collect();
    let mut link: Vec<usize> = (0..frq.len()).collect();
    let mut len = vec![0u64; frq.len()];
    while heap.len() > 1 {
        let Some(Reverse((_, mm))) = heap.pop() else { break };
        let Some(Reverse((_, m))) = heap.pop() else { break };
        frq[m] += frq[mm];
        heap.push(Reverse((frq[m], m)));
        // lengthen every code of both subtrees; join the lists
        let mut j = m;
        loop {
            len[j] += 1;
            if link[j] == j {
                link[j] = mm;
                break;
            }
            j = link[j];
        }
        let mut j = mm;
        loop {
            len[j] += 1;
            if link[j] == j {
                break;
            }
            j = link[j];
        }
    }
    len
}

struct BitWriter {
    out: Vec<u8>,
    c: u64,
    lc: u32,
    bits: u64,
}

impl BitWriter {
    fn put(&mut self, n: u32, v: u64) {
        self.bits += u64::from(n);
        // at most 58 bits per call, so split long codes to keep the accumulator in range
        if n > 32 {
            self.put_raw(n - 32, v >> 32);
            self.put_raw(32, v & 0xFFFF_FFFF);
        } else {
            self.put_raw(n, v);
        }
    }
    fn put_raw(&mut self, n: u32, v: u64) {
        self.c = (self.c << n) | v;
        self.lc += n;
        while self.lc >= 8 {
            self.lc -= 8;
            self.out.push((self.c >> self.lc) as u8);
        }
    }
    fn code(&mut self, h: u64) {
        self.put(hlen(h), hcode_of(h));
    }
    fn finish(&mut self) {
        if self.lc > 0 {
            self.out.push((self.c << (8 - self.lc)) as u8);
            self.lc = 0;
        }
    }
}

/// Huffman-code `words` (the Huffman block of a PIZ chunk, with its 20-byte header).
fn huf_compress(words: &[u16]) -> Result<Vec<u8>, String> {
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let mut freq = vec![0u64; HUF_ENCSIZE];
    for &w in words {
        freq[usize::from(w)] += 1;
    }
    let im = freq.iter().position(|&f| f > 0).unwrap_or(0);
    // the run-length pseudo-symbol follows the largest symbol
    let rlc = freq.iter().rposition(|&f| f > 0).unwrap_or(0) + 1;
    freq[rlc] = 1;
    let mut hc = code_lengths(&freq);
    if hc.iter().any(|&l| l > MAX_CODE_LEN) {
        return Err("PIZ: Huffman code too long".into());
    }
    canonical_codes(&mut hc);

    let mut w = BitWriter { out: Vec::with_capacity(words.len()), c: 0, lc: 0, bits: 0 };
    // packed table
    let mut i = im;
    while i <= rlc {
        let l = u64::from(hlen(hc[i]));
        if l == 0 {
            let mut run = 1u64;
            while i < rlc && run < LONGEST_LONG_RUN && hlen(hc[i + 1]) == 0 {
                i += 1;
                run += 1;
            }
            if run >= 2 {
                if run >= SHORTEST_LONG_RUN {
                    w.put(6, LONG_ZEROCODE_RUN);
                    w.put(8, run - SHORTEST_LONG_RUN);
                } else {
                    w.put(6, SHORT_ZEROCODE_RUN + run - 2);
                }
                i += 1;
                continue;
            }
        }
        w.put(6, l);
        i += 1;
    }
    w.finish();
    let table = std::mem::take(&mut w.out);
    w.bits = 0;

    // code bits: a run of 1 + n equal words is the word, the run code and n (8 bits) when shorter
    let rl = hc[rlc];
    let send = |w: &mut BitWriter, s: u64, run: u64| {
        if u64::from(hlen(s)) + u64::from(hlen(rl)) + 8 < u64::from(hlen(s)) * run {
            w.code(s);
            w.code(rl);
            w.put(8, run);
        } else {
            for _ in 0..=run {
                w.code(s);
            }
        }
    };
    let mut s = words[0];
    let mut run = 0u64;
    for &v in &words[1..] {
        if v == s && run < 255 {
            run += 1;
        } else {
            send(&mut w, hc[usize::from(s)], run);
            run = 0;
        }
        s = v;
    }
    send(&mut w, hc[usize::from(s)], run);
    let n_bits = w.bits;
    w.finish();
    let n_bits = u32::try_from(n_bits).map_err(|_| "PIZ: chunk too large".to_string())?;

    let mut out = Vec::with_capacity(20 + table.len() + w.out.len());
    for v in [im as u32, rlc as u32, table.len() as u32, n_bits, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&table);
    out.extend_from_slice(&w.out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(raw: &[u8], channels: &[PizChannel]) -> Vec<u8> {
        let packed = compress(raw, channels).unwrap();
        decompress(&packed, channels, raw.len()).unwrap()
    }

    fn layout(raw_planes: &[Vec<u16>], width: usize, lines: usize, bytes: usize) -> Vec<u8> {
        // planes are given per channel, line after line, as 16-bit words
        let mut out = Vec::new();
        let per_line = width * bytes / 2;
        for y in 0..lines {
            for p in raw_planes {
                for w in &p[y * per_line..(y + 1) * per_line] {
                    out.extend_from_slice(&w.to_le_bytes());
                }
            }
        }
        out
    }

    #[test]
    fn wavelets_invert() {
        for a in [0u16, 1, 2, 77, 8191, 16383] {
            for b in [0u16, 1, 3, 500, 16383] {
                let (l, h) = wenc14(a, b);
                assert_eq!(wdec14(l, h), (a, b));
            }
        }
        for a in [0u16, 1, 40000, 65535, 32768] {
            for b in [0u16, 2, 65535, 12345] {
                let (l, h) = wenc16(a, b);
                assert_eq!(wdec16(l, h), (a, b));
            }
        }
    }

    #[test]
    fn half_and_float_chunks_survive() {
        // odd sizes exercise the odd column and odd line paths; the float channel's two words
        // are interleaved; values above 1 << 14 after the lookup switch to the 16-bit wavelet
        for (w, lines) in [(1, 1), (7, 5), (33, 32), (64, 3), (5, 32)] {
            let n = w * lines;
            let ramp: Vec<u16> = (0..n).map(|i| (i * 37 % 1000) as u16 + 0x3C00).collect();
            let flat = vec![0x3C00u16; n];
            let float: Vec<u16> = (0..n * 2).map(|i| (i as u32).wrapping_mul(2654435761) as u16).collect();
            let chans = [PizChannel { width: w, lines, bytes: 2 }, PizChannel { width: w, lines, bytes: 2 }, PizChannel { width: w, lines, bytes: 4 }];
            let raw = layout_mixed(&[&ramp, &flat], &float, w, lines);
            assert_eq!(roundtrip(&raw, &chans), raw, "{w}×{lines}");
        }
        let zeros = vec![0u8; 3 * 2 * 16 * 4];
        let chans = [PizChannel { width: 16, lines: 4, bytes: 2 }; 3];
        assert_eq!(roundtrip(&zeros, &chans), zeros);
        let p: Vec<u16> = (0..64).map(|i| i * 1000).collect();
        let raw = layout(&[p], 16, 4, 2);
        assert_eq!(roundtrip(&raw, &[PizChannel { width: 16, lines: 4, bytes: 2 }]), raw);
    }

    fn layout_mixed(halves: &[&Vec<u16>], float: &[u16], w: usize, lines: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..lines {
            for p in halves {
                for v in &p[y * w..(y + 1) * w] {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            for v in &float[y * w * 2..(y + 1) * w * 2] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn long_codes_and_runs_decode() {
        // many distinct values with very skewed counts give codes longer than 14 bits
        let mut p = Vec::new();
        for i in 0..4000u16 {
            p.push(i);
        }
        p.extend(std::iter::repeat_n(7u16, 60_000));
        let w = 256;
        let lines = p.len() / w;
        p.truncate(w * lines);
        let raw = layout(&[p], w, lines, 2);
        let chans = [PizChannel { width: w, lines, bytes: 2 }];
        let packed = compress(&raw, &chans).unwrap();
        assert!(packed.len() < raw.len(), "{} of {}", packed.len(), raw.len());
        assert_eq!(decompress(&packed, &chans, raw.len()).unwrap(), raw);
    }

    #[test]
    fn hostile_chunks_are_errors() {
        let chans = [PizChannel { width: 8, lines: 2, bytes: 2 }];
        let raw: Vec<u8> = (0..32u8).collect();
        let good = compress(&raw, &chans).unwrap();
        for cut in 0..good.len() {
            assert!(decompress(&good[..cut], &chans, raw.len()).is_err(), "cut {cut}");
        }
        let mut bad = good.clone();
        bad[2..4].copy_from_slice(&9000u16.to_le_bytes());
        assert!(decompress(&bad, &chans, raw.len()).is_err());
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0x5A;
            let _ = decompress(&b, &chans, raw.len()); // never panics
        }
        assert!(decompress(&good, &chans, raw.len() + 2).is_err());
    }
}
