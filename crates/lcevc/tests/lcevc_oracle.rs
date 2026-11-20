//! Bit-exactness against V-Nova's LCEVCdec 4.3.1 (BSD-3-Clause-Clear), driven by
//! `tests/oracle/lcevc_oracle.c` (`scripts/ci/install-lcevc-oracle.sh` builds both). Random
//! synthetic access units (prefix-coded and RLE-only chunks, DD and DDS, every scaling and
//! upsampling mode, predicted average, temporal prediction with and without reduced signalling,
//! deblocking, dequantisation offsets, user data) are decoded by both decoders on random base
//! pictures. Skips when the oracle is not installed. `CFA4_LCEVC_SYNTH_ROUNDS=n` sets the number of
//! random streams (default 24).

use cfa4_lcevc::{Chroma, Decoder, Picture};
use std::io::Write;
use std::process::{Command, Stdio};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((v >> i) & 1) as u8;
            if let Some(last) = self.bytes.last_mut() {
                *last |= bit << (7 - self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

fn multibyte(out: &mut Vec<u8>, v: u64) {
    let mut groups = vec![(v & 0x7F) as u8];
    let mut v = v >> 7;
    while v > 0 {
        groups.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(groups.iter().rev());
}

fn block(out: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    if payload.len() <= 5 {
        out.push(kind | ((payload.len() as u8) << 5));
    } else {
        out.push(kind | (7 << 5));
        multibyte(out, payload.len() as u64);
    }
    out.extend_from_slice(payload);
}

/// A canonical prefix code (lengths at most 8 bits) for `symbols`, written as a table header.
struct Code {
    codes: Vec<(u8, u32, u32)>, // symbol, length, code
}

impl Code {
    fn build(rng: &mut Rng, symbols: &[u8], w: &mut BitWriter) -> Code {
        let mut syms: Vec<u8> = symbols.to_vec();
        syms.sort_unstable();
        syms.dedup();
        let n = syms.len();
        if n == 0 {
            w.put(31, 5);
            w.put(31, 5);
            return Code { codes: Vec::new() };
        }
        if n == 1 {
            w.put(0, 5);
            w.put(0, 5);
            w.put(u32::from(syms[0]), 8);
            return Code { codes: vec![(syms[0], 0, 0)] };
        }
        // Shuffle which symbols get the short codes.
        for i in (1..n).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            syms.swap(i, j);
        }
        let mut l = 1u32;
        while (1usize << l) < n {
            l += 1;
        }
        let short = (1usize << l) - n;
        let mut lens: Vec<(u8, u32)> = syms.iter().enumerate().map(|(i, &s)| (s, if i < short { l - 1 } else { l })).collect();
        let min = lens.iter().map(|e| e.1).min().unwrap_or(1);
        let max = lens.iter().map(|e| e.1).max().unwrap_or(1);
        let lb = 32 - (max - min).leading_zeros();
        w.put(min, 5);
        w.put(max, 5);
        let bitmap = n > 31 || rng.chance(1, 3);
        w.put(u32::from(bitmap), 1);
        if bitmap {
            for s in 0..=255u8 {
                if let Some(e) = lens.iter().find(|e| e.0 == s) {
                    w.put(1, 1);
                    w.put(e.1 - min, lb);
                } else {
                    w.put(0, 1);
                }
            }
        } else {
            w.put(n as u32, 5);
            for e in &lens {
                w.put(u32::from(e.0), 8);
                w.put(e.1 - min, lb);
            }
        }
        // Canonical codes as LCEVCdec assigns them.
        lens.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
        let mut codes = Vec::new();
        let mut code = 0u32;
        let mut cur = max;
        for &(s, b) in lens.iter().rev() {
            if b < cur {
                code >>= cur - b;
                cur = b;
            }
            codes.push((s, b, code));
            code += 1;
        }
        Code { codes }
    }

    fn put(&self, w: &mut BitWriter, s: u8) {
        if let Some(&(_, b, c)) = self.codes.iter().find(|e| e.0 == s) {
            w.put(c, b);
        }
    }
}

/// Symbols of one residual layer: (coefficient, zero run after it) events covering `total` TUs.
fn layer_events(rng: &mut Rng, total: u32) -> Vec<(i32, u32)> {
    let mut ev = Vec::new();
    let mut tu = 0;
    let density = rng.range(1, 12) as u64;
    while tu < total {
        let left = total - tu - 1;
        let run = if left == 0 {
            0
        } else if rng.chance(1, 8) {
            left
        } else {
            (rng.below(density * 4) as u32).min(left)
        };
        let v = match rng.below(10) {
            0 => 0,
            1 => rng.range(-8192, 8191) as i32,
            2 | 3 => rng.range(-400, 400) as i32,
            _ => rng.range(-32, 31) as i32,
        };
        ev.push((v, run));
        tu += run + 1;
    }
    ev
}

/// Symbol stream of residual events, with the table (0 LSB, 1 MSB, 2 RL) of each symbol.
fn residual_symbols(ev: &[(i32, u32)]) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    for &(v, run) in ev {
        let rl = run > 0;
        if (-32..=31).contains(&v) {
            let lsb = ((((v << 1) + 0x40) as u8) & 0x7E) | if rl { 0x80 } else { 0 };
            out.push((0, lsb));
        } else {
            let exp = ((v << 1) + 0x4000) as u32;
            out.push((0, ((exp & 0xFE) | 1) as u8));
            out.push((1, (((exp >> 8) & 0x7F) as u8) | if rl { 0x80 } else { 0 }));
        }
        if rl {
            let mut groups = vec![(run & 0x7F) as u8];
            let mut r = run >> 7;
            while r > 0 {
                groups.push((r & 0x7F) as u8);
                r >>= 7;
            }
            groups.reverse();
            let n = groups.len();
            for (i, g) in groups.into_iter().enumerate() {
                out.push((2, if i + 1 < n { g | 0x80 } else { g }));
            }
        }
    }
    out
}

/// Encodes a chunk from (table, symbol) pairs with `tables` prefix-code tables, or as raw bytes.
fn encode_chunk(rng: &mut Rng, syms: &[(usize, u8)], tables: usize, rle: bool, temporal: bool) -> Vec<u8> {
    if rle {
        return syms.iter().map(|s| s.1).collect();
    }
    let mut w = BitWriter::default();
    let mut codes = Vec::new();
    for t in 0..tables {
        let set: Vec<u8> = syms.iter().skip(usize::from(temporal)).filter(|s| s.0 == t).map(|s| s.1).collect();
        codes.push(Code::build(rng, &set, &mut w));
    }
    for (i, &(t, s)) in syms.iter().enumerate() {
        if temporal && i == 0 {
            w.put(u32::from(s), 8);
        } else {
            codes[t].put(&mut w, s);
        }
    }
    w.bytes
}

/// Temporal signal symbols: a raw first byte with the initial state, then runs that toggle the
/// state. The table of each later symbol is the state before it.
fn temporal_symbols(rng: &mut Rng, total: u32) -> Vec<(usize, u8)> {
    let mut state = rng.below(2) as u8;
    let mut out = vec![(0usize, state)];
    let mut tu = 0u32;
    while tu < total {
        let run = if rng.chance(1, 6) { total - tu } else { (rng.range(1, 40) as u32).min(total - tu) };
        let mut groups = vec![(run & 0x7F) as u8];
        let mut r = run >> 7;
        while r > 0 {
            groups.push((r & 0x7F) as u8);
            r >>= 7;
        }
        groups.reverse();
        let n = groups.len();
        for (i, g) in groups.into_iter().enumerate() {
            let last = i + 1 == n;
            out.push((usize::from(state), if last { g } else { g | 0x80 }));
            if last {
                state ^= 1;
            }
        }
        tu += run;
    }
    out
}

struct Stream {
    nals: Vec<Vec<u8>>,
    base: (u32, u32),
    out: (u32, u32),
    chroma: Chroma,
    base_depth: u32,
    enh_depth: u32,
}

fn nal(payload: &[u8], idr: bool) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0x41 | ((if idr { 29 } else { 28 }) << 1), 0xFF];
    let mut zeros = 0;
    for &b in payload {
        if zeros == 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out.push(0x80);
    out
}

fn synth(seed: u64) -> Stream {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..4 {
        rng.next();
    }
    let chroma = [Chroma::Mono, Chroma::C420, Chroma::C422, Chroma::C444][rng.below(4) as usize];
    let chroma_code = match chroma {
        Chroma::Mono => 0,
        Chroma::C420 => 1,
        Chroma::C422 => 2,
        Chroma::C444 => 3,
    };
    let plane_mode = chroma != Chroma::Mono && rng.chance(3, 4);
    let num_planes: usize = if plane_mode { 3 } else { 1 };
    let dds = rng.chance(1, 2);
    let base_code = rng.below(2) as u8;
    let enh_code = base_code.max(rng.below(2) as u8);
    let scaling0 = rng.below(3) as u8;
    let scaling1 = rng.below(3) as u8;
    let upsample = rng.below(5) as u8;
    let pa = rng.chance(1, 2);
    let temporal = rng.chance(1, 2);
    let reduced = temporal && rng.chance(1, 2);
    let deblock_sig = rng.chance(1, 3);
    let sw_mod = rng.chance(1, 3);
    let chroma_sw = rng.chance(1, 3);
    let user_data = if rng.chance(1, 5) { rng.range(1, 2) as u8 } else { 0 };
    let bw = 8 * rng.range(2, 8) as u32;
    let bh = 8 * rng.range(1, 6) as u32;
    let fx = |s: u8| if s != 0 { 2 } else { 1 };
    let fy = |s: u8| if s == 2 { 2 } else { 1 };
    let (w, h) = (bw * fx(scaling0) * fx(scaling1), bh * fy(scaling0) * fy(scaling1));
    let layers = if dds { 16 } else { 4 };

    // Global configuration.
    let mut gc = vec![
        (u8::from(plane_mode) << 7) | (63 << 1) | u8::from(dds),
        (chroma_code << 6) | (base_code << 4) | (enh_code << 2) | (u8::from(sw_mod) << 1) | u8::from(pa),
        (u8::from(reduced) << 7) | (u8::from(temporal) << 6) | (upsample << 3) | (u8::from(deblock_sig) << 2) | scaling1,
        (scaling0 << 6) | (user_data << 2) | u8::from(chroma_sw),
    ];
    if plane_mode {
        gc.push(1 << 4);
    }
    if sw_mod {
        gc.push(rng.below(256) as u8);
    }
    if upsample == 4 {
        for _ in 0..4 {
            let c = rng.range(0, 20000) as u16;
            gc.extend_from_slice(&c.to_be_bytes());
        }
    }
    if deblock_sig {
        gc.push(rng.below(256) as u8);
    }
    gc.extend_from_slice(&(w as u16).to_be_bytes());
    gc.extend_from_slice(&(h as u16).to_be_bytes());
    if chroma_sw {
        gc.push(rng.below(256) as u8);
    }

    let plane_dims = |loq: usize, plane: usize| -> (u32, u32) {
        let (mut pw, mut ph) = (w, h);
        let sc = [scaling0, scaling1];
        for s in sc.iter().take(loq) {
            pw /= fx(*s);
            ph /= fy(*s);
        }
        if plane > 0 {
            let (sx, sy) = chroma.shifts();
            pw >>= sx;
            ph >>= sy;
        }
        (pw, ph)
    };

    let frames = rng.range(2, 5);
    let mut nals = Vec::new();
    for f in 0..frames {
        let idr = f == 0 || rng.chance(1, 8);
        let mut au = Vec::new();
        if idr {
            block(&mut au, 1, &gc);
        }
        // Picture configuration.
        let qm_mode = rng.below(6) as u8;
        let dq = rng.chance(1, 3);
        let refresh = rng.chance(1, 4);
        let sw1 = rng.chance(1, 2);
        let mut pc = vec![(qm_mode << 4) | (u8::from(dq) << 3) | (u8::from(refresh) << 1) | u8::from(sw1)];
        let sw0 = rng.range(1, 6000) as u16;
        pc.extend_from_slice(&(sw0 << 1).to_be_bytes());
        if sw1 {
            let s = rng.range(1, 6000) as u16;
            pc.extend_from_slice(&((s << 1) | u16::from(rng.chance(1, 2))).to_be_bytes());
        }
        if matches!(qm_mode, 2 | 3 | 5) {
            for _ in 0..layers {
                pc.push(rng.below(256) as u8);
            }
        }
        if matches!(qm_mode, 4 | 5) {
            for _ in 0..layers {
                pc.push(rng.below(256) as u8);
            }
        }
        if dq {
            pc.push(rng.below(256) as u8);
        }
        block(&mut au, 2, &pc);

        // Encoded data.
        let tsp = temporal && !refresh;
        let tu_shift = if dds { 2 } else { 1 };
        let mut flags = BitWriter::default();
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        for plane in 0..num_planes {
            for loq in [1usize, 0] {
                let (pw, ph) = plane_dims(loq, plane);
                let total = (pw >> tu_shift) * (ph >> tu_shift);
                for _ in 0..layers {
                    let enabled = rng.chance(3, 4);
                    let rle = rng.chance(1, 3);
                    flags.put(u32::from(enabled), 1);
                    flags.put(u32::from(rle), 1);
                    if enabled {
                        let ev = layer_events(&mut rng, total);
                        let syms = residual_symbols(&ev);
                        chunks.push(encode_chunk(&mut rng, &syms, 3, rle, false));
                    }
                }
            }
            if tsp {
                let enabled = rng.chance(4, 5);
                let rle = rng.chance(1, 3);
                flags.put(u32::from(enabled), 1);
                flags.put(u32::from(rle), 1);
                if enabled {
                    let (pw, ph) = plane_dims(0, plane);
                    let total = (pw >> tu_shift) * (ph >> tu_shift);
                    let syms = temporal_symbols(&mut rng, total);
                    chunks.push(encode_chunk(&mut rng, &syms, 2, rle, true));
                }
            }
        }
        let mut ed = flags.bytes;
        for c in &chunks {
            multibyte(&mut ed, c.len() as u64);
            ed.extend_from_slice(c);
        }
        block(&mut au, 3, &ed);
        nals.push(nal(&au, idr));
    }
    Stream { nals, base: (bw, bh), out: (w, h), chroma, base_depth: 8 + 2 * u32::from(base_code), enh_depth: 8 + 2 * u32::from(enh_code) }
}

fn base_picture(rng: &mut Rng, s: &Stream) -> Picture {
    let mut pic = Picture { width: s.base.0, height: s.base.1, bit_depth: s.base_depth, chroma: s.chroma, planes: Vec::new() };
    let n = if s.chroma == Chroma::Mono { 1 } else { 3 };
    let max = (1u32 << s.base_depth) - 1;
    for p in 0..n {
        let (pw, ph) = pic.plane_size(p);
        let mut plane = Vec::with_capacity((pw * ph) as usize);
        let fx = rng.range(0, 7) as u32;
        let fy = rng.range(0, 7) as u32;
        let noise = rng.range(0, 64) as u32;
        let extreme = rng.chance(1, 5);
        for y in 0..ph {
            for x in 0..pw {
                let v = if extreme {
                    if (x + y) % 2 == 0 { max } else { 0 }
                } else {
                    let base = (x * fx + y * fy) * (max / 128 + 1);
                    (base + rng.below(u64::from(noise) * (max as u64 / 255 + 1) + 1) as u32) % (max + 1)
                };
                plane.push(v as u16);
            }
        }
        pic.planes.push(plane);
    }
    pic
}

fn format(chroma: Chroma, depth: u32) -> u32 {
    let family = match chroma {
        Chroma::Mono => 5000,
        Chroma::C420 => 1000,
        Chroma::C422 => 1200,
        Chroma::C444 => 1400,
    };
    family + if depth == 8 { 1 } else { 2 }
}

fn oracle() -> Option<String> {
    let path = std::env::var("CFA4_LCEVC_ORACLE").unwrap_or_else(|_| "lcevc_oracle".into());
    let ok = Command::new(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
    ok.then_some(path)
}

fn run_oracle(bin: &str, s: &Stream, bases: &[Picture]) -> Result<Vec<u8>, String> {
    let mut input = Vec::new();
    for v in [s.nals.len() as u32, format(s.chroma, s.base_depth), s.base.0, s.base.1, format(s.chroma, s.enh_depth), s.out.0, s.out.1] {
        input.extend_from_slice(&v.to_le_bytes());
    }
    for (n, b) in s.nals.iter().zip(bases) {
        input.extend_from_slice(&(n.len() as u32).to_le_bytes());
        input.extend_from_slice(n);
        for p in &b.planes {
            for &v in p {
                if s.base_depth == 8 {
                    input.push(v as u8);
                } else {
                    input.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    let mut child = Command::new(bin).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(format!("oracle failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(out.stdout)
}

fn bytes_of(p: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for plane in &p.planes {
        for &v in plane {
            if p.bit_depth == 8 {
                out.push(v as u8);
            } else {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}

fn check(bin: &str, seed: u64) -> Result<(), String> {
    let s = synth(seed);
    let mut rng = Rng(seed ^ 0xDEAD_BEEF_1234_5678);
    let bases: Vec<Picture> = s.nals.iter().map(|_| base_picture(&mut rng, &s)).collect();
    let expected = run_oracle(bin, &s, &bases)?;
    let mut dec = Decoder::new();
    let mut got = Vec::new();
    for (i, (n, b)) in s.nals.iter().zip(&bases).enumerate() {
        let d = dec.decode(n, b).map_err(|e| format!("frame {i}: {e}"))?;
        got.extend(bytes_of(&d.picture));
    }
    if got.len() != expected.len() {
        return Err(format!("size {} vs oracle {}", got.len(), expected.len()));
    }
    if let Some(pos) = got.iter().zip(&expected).position(|(a, b)| a != b) {
        let frame_size = got.len() / s.nals.len();
        return Err(format!(
            "first difference at byte {pos} (frame {}, offset {}): ours {} oracle {} (base {:?} out {:?} {:?} {}->{} bits)",
            pos / frame_size,
            pos % frame_size,
            got[pos],
            expected[pos],
            s.base,
            s.out,
            s.chroma,
            s.base_depth,
            s.enh_depth
        ));
    }
    Ok(())
}

#[test]
fn synthetic_streams_match_lcevcdec() {
    let Some(bin) = oracle() else {
        eprintln!("lcevc_oracle not found (scripts/ci/install-lcevc-oracle.sh); skipping");
        return;
    };
    let rounds: u64 = std::env::var("CFA4_LCEVC_SYNTH_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
    let start: u64 = std::env::var("CFA4_LCEVC_SYNTH_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let mut failures = Vec::new();
    for seed in start..start + rounds {
        if let Err(e) = check(&bin, seed) {
            failures.push(format!("#{seed}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{} of {rounds} streams differ:\n{}", failures.len(), failures.join("\n"));
}
