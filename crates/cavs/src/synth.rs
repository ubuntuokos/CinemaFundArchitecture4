//! Synthetic AVS1 streams for tests (feature `synth`): random but syntactically valid I / P / B
//! pictures (every macroblock type, sub-partition, intra mode, motion vector, residual code and
//! escape, skip runs, slices, quantiser and deblocking settings) written from the decoder's own
//! tables. The oracle test decodes them here and with ffmpeg's `cavs` decoder (an external
//! oracle; skipped when ffmpeg is missing). FFmpeg has no AVS1 encoder, so this is the generator.

use crate::tables::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 33) as u32 % n.max(1)
    }

    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    acc: u64,
    n: u32,
    /// Byte offsets of the start codes written on purpose.
    codes: Vec<usize>,
}

impl Writer {
    fn put(&mut self, bits: u32, v: u64) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.bytes.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    fn ue(&mut self, v: u32) {
        let x = u64::from(v) + 1;
        let len = 64 - x.leading_zeros();
        self.put(len - 1, 0);
        self.put(len, x);
    }

    fn se(&mut self, v: i32) {
        self.ue(if v > 0 { (2 * v - 1) as u32 } else { (-2 * v) as u32 });
    }

    fn ue_k(&mut self, v: u32, order: u32) {
        self.ue(v >> order);
        if order > 0 {
            self.put(order, u64::from(v & ((1 << order) - 1)));
        }
    }

    /// Stuffing (`1` then `0`s) to the byte boundary.
    fn align(&mut self) {
        if self.n != 0 {
            self.put(1, 1);
            while self.n != 0 {
                self.put(1, 0);
            }
        }
    }

    fn bits(&self) -> usize {
        self.bytes.len() * 8 + self.n as usize
    }

    fn start_code(&mut self, code: u8) {
        self.align();
        self.codes.push(self.bytes.len());
        self.bytes.extend_from_slice(&[0, 0, 1, code]);
    }
}

/// What to generate.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub width: u32,
    pub height: u32,
    /// No B pictures, output in decoding order.
    pub low_delay: bool,
    pub seed: u64,
    pub pictures: usize,
}

struct Gen {
    w: Writer,
    rng: Rng,
    mb_w: usize,
    mb_h: usize,
    qp_fixed: bool,
    /// Luma intra modes as the decoder predicts them: the bottom blocks of the macroblocks
    /// above, the right blocks of the one on the left, and whether the row above counts.
    top_modes: Vec<[i32; 2]>,
    left_modes: [i32; 2],
    top_avail: bool,
}

impl Gen {
    fn residual(&mut self, tabs: &[Vlc2d], esc_order: u32) {
        let mut r = 0usize;
        let last = tabs.len() - 1;
        let mut runs = 0u32;
        let coeffs = self.rng.below(7);
        for _ in 0..coeffs {
            let t = &tabs[r.min(last)];
            if self.rng.chance(20) {
                // escape: run 1..=max, level from the escape code
                let run = self.rng.range(1, 12) as u32;
                if runs + run > 64 {
                    break;
                }
                let sign = self.rng.below(2);
                self.w.ue_k(59 + 2 * (run - 1) + sign, t.golomb_order);
                let cap = if self.rng.chance(10) { 2000 } else { 24 };
                let esc = self.rng.below(cap);
                self.w.ue_k(esc, esc_order);
                let add = if run > t.max_run as u32 { 1 } else { i32::from(t.level_add[run as usize]) };
                let l = esc as i32 + add;
                while l > tabs[r.min(last)].inc_limit && r < last {
                    r += 1;
                }
                runs += run;
            } else {
                let codes: Vec<u32> = (0..59).filter(|&c| t.rltab[c as usize][0] != 0 && runs + t.rltab[c as usize][1] as u32 <= 64).collect();
                let Some(&c) = codes.get(self.rng.below(codes.len() as u32) as usize) else { break };
                let e = t.rltab[c as usize];
                self.w.ue_k(c, t.golomb_order);
                runs += e[1] as u32;
                r += e[2] as usize;
            }
        }
        // end of block
        let t = &tabs[r.min(last)];
        let eob = (0..59u32).find(|&c| t.rltab[c as usize][0] == 0).unwrap_or(0);
        self.w.ue_k(eob, t.golomb_order);
    }

    fn qp_delta(&mut self, cbp: u8) {
        if cbp != 0 && !self.qp_fixed {
            let d = self.rng.range(-4, 4);
            self.w.se(d);
        }
    }

    fn residual_intra(&mut self, cbp: u8) {
        self.qp_delta(cbp);
        for block in 0..4 {
            if cbp & (1 << block) != 0 {
                self.residual(&INTRA_DEC, 1);
            }
        }
        self.chroma(cbp);
    }

    fn chroma(&mut self, cbp: u8) {
        for block in 4..6 {
            if cbp & (1 << block) != 0 {
                self.residual(&CHROMA_DEC, 0);
            }
        }
    }

    fn residual_inter(&mut self) {
        let code = self.rng.below(64);
        self.w.ue(code);
        let cbp = CBP_TAB[code as usize][1];
        self.qp_delta(cbp);
        for block in 0..4 {
            if cbp & (1 << block) != 0 {
                self.residual(&INTER_DEC, 0);
            }
        }
        self.chroma(cbp);
    }

    /// Intra prediction modes: only modes whose neighbouring samples exist (an illegal mode
    /// makes the decoder predict from what the previous picture left in its border lines, so a
    /// picture would decode differently after a seek), coded against the predicted mode.
    fn intra_modes(&mut self, mbx: usize) {
        let left_avail = mbx > 0;
        let top = if self.top_avail { self.top_modes.get(mbx).copied().unwrap_or([-1, -1]) } else { [-1, -1] };
        let mut m = [0i32; 4];
        for block in 0..4 {
            let (l, t) = match block {
                0 => (self.left_modes[0], top[0]),
                1 => (m[0], top[1]),
                2 => (self.left_modes[1], m[0]),
                _ => (m[2], m[1]),
            };
            let has_left = block % 2 == 1 || left_avail;
            let has_top = block >= 2 || self.top_avail;
            let legal: &[i32] = match (has_left, has_top) {
                (true, true) => &[0, 1, 2, 3, 4],
                (false, true) => &[0, 2],
                (true, false) => &[1, 2],
                (false, false) => &[2],
            };
            let mode = legal[self.rng.below(legal.len() as u32) as usize];
            let pred = if l.min(t) == -1 { 2 } else { l.min(t) };
            if mode == pred {
                self.w.put(1, 1);
            } else {
                self.w.put(1, 0);
                self.w.put(2, (if mode < pred { mode } else { mode - 1 }) as u64);
            }
            m[block] = mode;
        }
        self.left_modes = [m[1], m[3]];
        if let Some(t) = self.top_modes.get_mut(mbx) {
            *t = [m[2], m[3]];
        }
        let legal_uv: &[u32] = match (left_avail, self.top_avail) {
            (true, true) => &[0, 1, 2, 3],
            (false, true) => &[0, 2],
            (true, false) => &[0, 1],
            (false, false) => &[0],
        };
        let uv = legal_uv[self.rng.below(legal_uv.len() as u32) as usize];
        self.w.ue(uv);
    }

    /// A macroblock without intra modes: its neighbours predict from "not available".
    fn not_intra(&mut self, mbx: usize) {
        self.left_modes = [-1, -1];
        if let Some(t) = self.top_modes.get_mut(mbx) {
            *t = [-1, -1];
        }
    }

    fn mvd(&mut self) {
        let big = self.rng.chance(10);
        for _ in 0..2 {
            let v = if big { self.rng.range(-300, 300) } else { self.rng.range(-12, 12) };
            self.w.se(v);
        }
    }

    /// Slice start codes at some macroblock rows (row 0 included at times). A slice only follows
    /// a row that took two bytes or more: the decoder looks for a start code at the byte boundary
    /// at each row start, so after a row of skipped macroblocks it would take this slice for the
    /// one of that row (FFmpeg then leaves the last row unwritten).
    fn maybe_slice(&mut self, mby: usize, pic_qp_fixed: bool, inter: bool, prev_row: Option<usize>) -> bool {
        if prev_row.is_some_and(|p| self.w.bits().div_ceil(8) < p.div_ceil(8) + 2) || !self.rng.chance(40) {
            return false;
        }
        self.w.start_code(mby as u8);
        if !pic_qp_fixed {
            self.qp_fixed = self.rng.chance(50);
            self.w.put(1, u64::from(self.qp_fixed));
            self.w.put(6, u64::from(self.rng.range(16, 50) as u32));
        }
        if inter {
            self.w.put(1, 0); // no weighting
        }
        true
    }

    /// Writes a picture. `kind` 1 = I, 2 = P, 3 = B. `two_refs` allows P reference index 1.
    fn picture(&mut self, kind: u32, distance: u32, low_delay: bool, two_refs: bool) {
        self.w.start_code(if kind == 1 { 0xb3 } else { 0xb6 });
        self.w.put(16, 0xffff); // bbv_delay
        if kind == 1 {
            self.w.put(1, 0); // no time code
            self.w.put(1, 1); // marker
        } else {
            self.w.put(2, u64::from(kind - 1));
        }
        self.w.put(8, u64::from(distance));
        if low_delay {
            self.w.ue(0);
        }
        self.w.put(1, 1); // progressive_frame
        self.w.put(1, 0); // top_field_first
        self.w.put(1, 0); // repeat_first_field
        let pic_qp_fixed = self.rng.chance(50);
        self.qp_fixed = pic_qp_fixed;
        self.w.put(1, u64::from(pic_qp_fixed));
        self.w.put(6, u64::from(self.rng.range(12, 52) as u32));
        let mut ref_flag = true;
        let mut skip_mode = false;
        if kind == 1 {
            self.w.put(4, 0);
        } else {
            if kind == 2 {
                ref_flag = !two_refs || self.rng.chance(30);
                self.w.put(1, u64::from(ref_flag));
            }
            self.w.put(4, 0);
            skip_mode = self.rng.chance(60);
            self.w.put(1, u64::from(skip_mode));
        }
        let lf_disable = self.rng.chance(20);
        self.w.put(1, u64::from(lf_disable));
        if !lf_disable {
            let offsets = self.rng.chance(50);
            self.w.put(1, u64::from(offsets));
            if offsets {
                let (a, b) = (self.rng.range(-8, 8), self.rng.range(-8, 8));
                self.w.se(a);
                self.w.se(b);
            }
        }

        let mut skip_count: i64 = -1;
        let mut prev_row = None;
        for mby in 0..self.mb_h {
            for mbx in 0..self.mb_w {
                if mbx == 0 {
                    let slice = self.maybe_slice(mby, pic_qp_fixed, kind != 1, prev_row);
                    if slice {
                        skip_count = -1;
                    }
                    prev_row = Some(self.w.bits());
                    self.left_modes = [-1, -1];
                    self.top_avail = mby > 0 && !slice;
                }
                if kind == 1 {
                    self.intra_modes(mbx);
                    let code = self.rng.below(64);
                    self.w.ue(code);
                    self.residual_intra(CBP_TAB[code as usize][0]);
                    continue;
                }
                if skip_mode && skip_count < 0 {
                    let n = self.rng.below(4);
                    self.w.ue(n);
                    skip_count = i64::from(n);
                }
                let skip = skip_mode && {
                    let c = skip_count;
                    skip_count -= 1;
                    c != 0
                };
                if skip {
                    self.not_intra(mbx);
                    continue;
                }
                let base = if kind == 2 { 1 } else { 6 } + u32::from(skip_mode);
                let last = if kind == 2 { 5 } else { 29 };
                let mb_type = if self.rng.chance(12) {
                    last + 1 + self.rng.below(64) // intra, with the coded block pattern
                } else {
                    base + self.rng.below(last + 1 - base)
                };
                self.w.ue(mb_type - base);
                if mb_type > last {
                    self.intra_modes(mbx);
                    self.residual_intra(CBP_TAB[(mb_type - last - 1) as usize][0]);
                    continue;
                }
                self.not_intra(mbx);
                if kind == 2 {
                    let parts = match mb_type {
                        1 => 0,
                        2 => 1,
                        3 | 4 => 2,
                        _ => 4,
                    };
                    if !ref_flag {
                        for _ in 0..parts {
                            self.w.put(1, u64::from(self.rng.below(2)));
                        }
                    }
                    for _ in 0..parts {
                        self.mvd();
                    }
                    if mb_type != 1 {
                        self.residual_inter();
                    }
                } else {
                    let mvds = match mb_type {
                        6 | 7 => 0,
                        8..=10 => 1,
                        29 => {
                            let subs: Vec<u32> = (0..4).map(|_| self.rng.below(4)).collect();
                            for &s in &subs {
                                self.w.put(2, u64::from(s));
                            }
                            subs.iter().filter(|&&s| s != 0).count()
                        }
                        t => {
                            let f = PARTITION_FLAGS[t as usize];
                            [FWD0, FWD1, BWD0, BWD1].iter().filter(|&&m| f & m != 0).count()
                        }
                    };
                    for _ in 0..mvds {
                        self.mvd();
                    }
                    if mb_type != 6 {
                        self.residual_inter();
                    }
                }
            }
        }
    }
}

/// A stream: sequence header, then I P B B P B B … (or I P P … with `low_delay`).
fn stream(p: Params) -> (Vec<u8>, Vec<usize>) {
    let mut g = Gen {
        w: Writer::default(),
        rng: Rng(p.seed | 1),
        mb_w: p.width.div_ceil(16) as usize,
        mb_h: p.height.div_ceil(16) as usize,
        qp_fixed: true,
        top_modes: vec![[-1, -1]; p.width.div_ceil(16) as usize],
        left_modes: [-1, -1],
        top_avail: false,
    };
    g.w.start_code(0xb0);
    g.w.put(8, 0x20); // Jizhun
    g.w.put(8, 0x40); // level 6.0
    g.w.put(1, 1); // progressive_sequence
    g.w.put(14, u64::from(p.width));
    g.w.put(14, u64::from(p.height));
    g.w.put(2, 1); // 4:2:0
    g.w.put(3, 1); // 8 bits
    g.w.put(4, 1); // square samples
    g.w.put(4, 3); // 25 Hz
    g.w.put(18, 1000);
    g.w.put(1, 1);
    g.w.put(12, 0);
    g.w.put(1, u64::from(p.low_delay));
    g.w.put(1, 1);
    g.w.put(18, 100);
    g.w.put(3, 0);

    let mut refs = 0;
    let mut distance = 0u32;
    g.picture(1, 0, p.low_delay, false);
    refs += 1;
    let mut written = 1;
    while written < p.pictures {
        let i_again = g.rng.chance(10);
        // a P picture refers to two pictures only when both follow the last I picture (closed
        // GOPs for P pictures: decoding can start at any I picture, as FFmpeg's seeking does)
        if p.low_delay {
            distance += 2;
            g.picture(if i_again { 1 } else { 2 }, distance & 255, true, refs >= 2);
            refs = if i_again { 1 } else { refs + 1 };
            written += 1;
            continue;
        }
        // a reference picture three steps on, then the B pictures before it
        let nb = g.rng.below(3);
        let ref_distance = distance + 2 * (nb + 1);
        g.picture(if i_again { 1 } else { 2 }, ref_distance & 255, false, refs >= 2);
        refs = if i_again { 1 } else { refs + 1 };
        written += 1;
        for k in 0..nb {
            g.picture(3, (distance + 2 * (k + 1)) & 255, false, true);
            written += 1;
        }
        distance = ref_distance;
    }
    g.w.start_code(0xb1); // sequence end
    (g.w.bytes, g.w.codes)
}

/// True when the stream holds no start code it did not write on purpose.
fn clean(data: &[u8], codes: &[usize]) -> bool {
    data.windows(3).enumerate().all(|(i, w)| w != [0, 0, 1] || codes.contains(&i))
}

/// A stream for `p`, generated again with derived seeds while random data imitates a start code.
/// Returns the stream and the seed that made it.
pub fn clean_stream(p: Params) -> Option<(Vec<u8>, u64)> {
    (0..64u64).map(|k| p.seed.wrapping_mul(1_000_003).wrapping_add(k)).find_map(|seed| {
        let (data, codes) = stream(Params { seed, ..p });
        clean(&data, &codes).then_some((data, seed))
    })
}

#[cfg(test)]
mod oracle {
    use std::process::{Command, Stdio};

    use super::{Params, clean_stream};
    use crate::{Decoder, Frame, split_packets};

    fn have_ffmpeg() -> bool {
        Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn decode_all(data: &[u8]) -> Vec<Frame> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for (i, p) in split_packets(data).into_iter().enumerate() {
            if let Ok(Some(f)) = d.decode_tagged(p, i as i64) {
                out.push(f);
            }
        }
        out.extend(d.flush());
        out
    }

    #[test]
    fn synthetic_streams_match_ffmpeg() {
        if !have_ffmpeg() {
            eprintln!("SKIPPED (AVS1 synthetic oracle): ffmpeg is not installed");
            return;
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/cavs");
        std::fs::create_dir_all(&dir).unwrap();
        let sizes = [(64, 48), (72, 40), (50, 34), (176, 144), (32, 96), (120, 66), (16, 16), (200, 18)];
        // CAVS_SYNTH_CASES=n runs more streams locally
        let n = std::env::var("CAVS_SYNTH_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(24u64);
        let cases = (0..n).map(|k| {
            let (width, height) = sizes[k as usize % sizes.len()];
            Params { width, height, low_delay: k % 3 == 2, seed: k + 1, pictures: 8 + (k as usize % 4) * 3 }
        });
        for c in cases {
            let (data, seed) = clean_stream(c).expect("no clean stream");
            let name = format!("synth_{}x{}_{seed}.avs", c.width, c.height);
            let path = dir.join(&name);
            std::fs::write(&path, &data).unwrap();
            let out = Command::new("ffmpeg")
                .args(["-v", "quiet", "-cpuflags", "0", "-f", "cavsvideo", "-i"])
                .arg(&path)
                .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
                .output()
                .unwrap();
            let want = out.stdout;
            let got = decode_all(&data);
            let size = (c.width * c.height + 2 * c.width.div_ceil(2) * c.height.div_ceil(2)) as usize;
            assert_eq!(want.len() % size, 0, "{name}");
            assert!(!got.is_empty(), "{name}: no pictures");
            for (i, (f, w)) in got.iter().zip(want.chunks(size)).enumerate() {
                let g = f.planes.concat();
                if g != w {
                    let pos = g.iter().zip(w).position(|(a, b)| a != b).unwrap_or(0);
                    let ny = (c.width * c.height) as usize;
                    let (plane, at, wd) = if pos < ny { ("Y", pos, c.width as usize) } else { ("C", pos - ny, c.width.div_ceil(2) as usize) };
                    let mbs: Vec<(usize, usize)> = (0..c.height.div_ceil(16) as usize)
                        .flat_map(|my| (0..c.width.div_ceil(16) as usize).map(move |mx| (mx, my)))
                        .filter(|&(mx, my)| {
                            (0..16).any(|y| {
                                (0..16).any(|x| {
                                    let (px, py) = (mx * 16 + x, my * 16 + y);
                                    px < c.width as usize && py < c.height as usize && g[py * c.width as usize + px] != w[py * c.width as usize + px]
                                })
                            })
                        })
                        .collect();
                    eprintln!("luma MBs differing: {mbs:?}");
                    let _ = std::fs::write(dir.join(format!("{name}.got")), &g);
                    let _ = std::fs::write(dir.join(format!("{name}.want")), w);
                    panic!("{name}: picture {i} (packet {}) differs, first at {plane} x={} y={} ({} vs {})", f.tag, at % wd, at / wd, g[pos], w[pos]);
                }
            }
            assert_eq!(got.len(), want.len() / size, "{name}: picture count");
        }
    }
}
