//! CEA-608 (line 21) character sets and control codes, from the public specification
//! (ANSI/CTA-608-E; 47 CFR §15.119).
//!
//! Bytes travel in pairs with odd parity in bit 7. A pair whose first byte (parity stripped) is
//! 0x10–0x1F is a control code (channel 1: 0x10–0x17, channel 2: 0x18–0x1F); otherwise both bytes
//! are characters from the basic set (0x00 = padding).

/// Add odd parity to a 7-bit value.
pub fn with_parity(b: u8) -> u8 {
    let b = b & 0x7f;
    if b.count_ones().is_multiple_of(2) { b | 0x80 } else { b }
}

/// Basic character set (0x20–0x7F). Differs from ASCII in ten positions.
pub fn basic_char(b: u8) -> Option<char> {
    Some(match b {
        0x2a => 'á',
        0x5c => 'é',
        0x5e => 'í',
        0x5f => 'ó',
        0x60 => 'ú',
        0x7b => 'ç',
        0x7c => '÷',
        0x7d => 'Ñ',
        0x7e => 'ñ',
        0x7f => '█',
        0x20..=0x7e => b as char,
        _ => return None,
    })
}

/// Special characters: control pair `0x11, 0x30 + i`.
pub const SPECIAL: [char; 16] = ['®', '°', '½', '¿', '™', '¢', '£', '♪', 'à', '\u{a0}', 'è', 'â', 'ê', 'î', 'ô', 'û'];

/// Extended Spanish/French/miscellaneous: control pair `0x12, 0x20 + i`.
pub const EXT_12: [char; 32] = [
    'Á', 'É', 'Ó', 'Ú', 'Ü', 'ü', '‘', '¡', '*', '\'', '—', '©', '℠', '•', '“', '”', 'À', 'Â', 'Ç', 'È', 'Ê', 'Ë', 'ë', 'Î', 'Ï', 'ï', 'Ô', 'Ù', 'ù', 'Û', '«',
    '»',
];

/// Extended Portuguese/German/Danish: control pair `0x13, 0x20 + i`.
pub const EXT_13: [char; 32] = [
    'Ã', 'ã', 'Í', 'Ì', 'ì', 'Ò', 'ò', 'Õ', 'õ', '{', '}', '\\', '^', '_', '|', '~', 'Ä', 'ä', 'Ö', 'ö', 'ß', '¥', '¤', '¦', 'Å', 'å', 'Ø', 'ø', '┌', '┐', '└',
    '┘',
];

/// How a character is transmitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoded {
    /// One basic-set byte.
    Basic(u8),
    /// A two-byte special/extended code (channel 1 first byte, second byte). Extended characters
    /// replace the preceding character, so a basic `fallback` is sent first for older decoders.
    Pair { b1: u8, b2: u8, fallback: Option<u8> },
}

fn basic_byte(c: char) -> Option<u8> {
    (0x20u8..=0x7f).find(|&b| basic_char(b) == Some(c))
}

/// Encode one character (`None` if 608 cannot show it).
pub fn encode_char(c: char) -> Option<Encoded> {
    let c = match c {
        '’' | 'ʼ' => '\'',
        '\t' => ' ',
        '–' => '-',
        _ => c,
    };
    if let Some(b) = basic_byte(c) {
        return Some(Encoded::Basic(b));
    }
    if let Some(i) = SPECIAL.iter().position(|&s| s == c) {
        return Some(Encoded::Pair { b1: 0x11, b2: 0x30 + i as u8, fallback: None });
    }
    let fallback = |c: char| -> u8 {
        let f = match c {
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'ü' | 'ù' => 'u',
            'ë' => 'e',
            'ï' | 'ì' => 'i',
            'ã' | 'ä' | 'å' => 'a',
            'ò' | 'õ' | 'ö' | 'ø' => 'o',
            'Ç' => 'C',
            'ß' => 's',
            '‘' | '“' | '”' | '«' | '»' => '"',
            '{' | '┌' | '└' => '[',
            '}' | '┐' | '┘' => ']',
            '¡' => '!',
            '*' | '•' => '.',
            _ => '-',
        };
        basic_byte(f).unwrap_or(0x2d)
    };
    if let Some(i) = EXT_12.iter().position(|&s| s == c) {
        return Some(Encoded::Pair { b1: 0x12, b2: 0x20 + i as u8, fallback: Some(fallback(c)) });
    }
    if let Some(i) = EXT_13.iter().position(|&s| s == c) {
        return Some(Encoded::Pair { b1: 0x13, b2: 0x20 + i as u8, fallback: Some(fallback(c)) });
    }
    None
}

/// Miscellaneous control codes (second byte, first byte 0x14 on channel 1).
pub mod misc {
    pub const RCL: u8 = 0x20; // resume caption loading (pop-on)
    pub const BS: u8 = 0x21; // backspace
    pub const AOF: u8 = 0x22;
    pub const AON: u8 = 0x23;
    pub const DER: u8 = 0x24; // delete to end of row
    pub const RU2: u8 = 0x25; // roll-up 2/3/4 rows
    pub const RU3: u8 = 0x26;
    pub const RU4: u8 = 0x27;
    pub const FON: u8 = 0x28;
    pub const RDC: u8 = 0x29; // resume direct captioning (paint-on)
    pub const TR: u8 = 0x2a;
    pub const RTD: u8 = 0x2b;
    pub const EDM: u8 = 0x2c; // erase displayed memory
    pub const CR: u8 = 0x2d; // carriage return (roll-up)
    pub const ENM: u8 = 0x2e; // erase non-displayed memory
    pub const EOC: u8 = 0x2f; // end of caption (swap memories)
}

/// Row (1–15) and indent column addressed by a preamble address code, if `(b1, b2)` is one
/// (channel bit already removed from `b1`).
pub fn decode_pac(b1: u8, b2: u8) -> Option<(usize, usize)> {
    if !(0x40..=0x7f).contains(&b2) {
        return None;
    }
    let hi = b2 >= 0x60;
    let row = match (b1, hi) {
        (0x11, false) => 1,
        (0x11, true) => 2,
        (0x12, false) => 3,
        (0x12, true) => 4,
        (0x15, false) => 5,
        (0x15, true) => 6,
        (0x16, false) => 7,
        (0x16, true) => 8,
        (0x17, false) => 9,
        (0x17, true) => 10,
        (0x10, false) => 11,
        (0x13, false) => 12,
        (0x13, true) => 13,
        (0x14, false) => 14,
        (0x14, true) => 15,
        _ => return None,
    };
    let a = b2 & 0x1f;
    let indent = if a & 0x10 != 0 { ((a & 0x0e) >> 1) as usize * 4 } else { 0 };
    Some((row, indent))
}

/// Preamble address code for `row` (1–15) with an indent (rounded down to a multiple of 4).
pub fn encode_pac(row: usize, indent: usize) -> (u8, u8) {
    let (b1, hi) = match row {
        1 => (0x11, false),
        2 => (0x11, true),
        3 => (0x12, false),
        4 => (0x12, true),
        5 => (0x15, false),
        6 => (0x15, true),
        7 => (0x16, false),
        8 => (0x16, true),
        9 => (0x17, false),
        10 => (0x17, true),
        11 => (0x10, false),
        12 => (0x13, false),
        13 => (0x13, true),
        14 => (0x14, false),
        _ => (0x14, true),
    };
    let attr = if indent >= 4 { 0x10 | (((indent / 4).min(7) as u8) << 1) } else { 0 };
    (b1, (if hi { 0x60 } else { 0x40 }) | attr)
}

// ---------------------------------------------------------------------------------------------
// Line 21 waveform (CTA-608-E Annex A / 47 CFR §15.119(b))
// ---------------------------------------------------------------------------------------------

/// Bit period of the line 21 signal in luma samples of a 720-sample (13.5 MHz) active line:
/// 13.5 MHz / (32 × fH), fH = 15 734.264 Hz.
const BIT_SAMPLES_720: f64 = 26.8125;

/// The line 21 waveform of the byte pair `(b1, b2)` (parity bits included as given) as `n`
/// 10-bit luma samples of the active line: seven cycles of clock run-in from sample `start`, two
/// logic-zero and one logic-one start bits, then the 16 data bits least significant first, at
/// blanking (64) and 50 IRE (502) with softened edges. Used to test [`decode_line21`] and by
/// writers of digitised vertical interval lines.
pub fn line21_samples(b1: u8, b2: u8, n: usize, start: f64) -> Vec<u16> {
    let w = BIT_SAMPLES_720 * n as f64 / 720.0;
    let (lo, hi) = (64.0, 502.0);
    let mut bits = vec![false, false, true];
    for b in [b1, b2] {
        bits.extend((0..8).map(|k| b >> k & 1 == 1));
    }
    let level = |t: f64| -> f64 {
        let x = (t - start) / w;
        if x < 0.0 {
            return lo;
        }
        if x < 7.0 {
            return lo + (hi - lo) * 0.5 * (1.0 - (2.0 * std::f64::consts::PI * x).cos());
        }
        let k = x - 7.0;
        let at = |i: f64| -> f64 { if i >= 0.0 && bits.get(i as usize).copied().unwrap_or(false) { hi } else { lo } };
        // raised-cosine transitions over a fifth of a bit around each boundary
        let i = k.floor();
        let frac = k - i;
        let (cur, prev, next) = (at(i), at(i - 1.0), at(i + 1.0));
        if frac < 0.1 {
            let m = 0.5 + frac / 0.2;
            prev + (cur - prev) * 0.5 * (1.0 - (std::f64::consts::PI * m).cos())
        } else if frac > 0.9 {
            let m = (frac - 0.9) / 0.2;
            cur + (next - cur) * 0.5 * (1.0 - (std::f64::consts::PI * m).cos())
        } else {
            cur
        }
    };
    (0..n).map(|i| level(i as f64).round().clamp(0.0, 1023.0) as u16).collect()
}

/// The byte pair of a digitised line 21 (CTA-608-E): `samples` are the luma samples of the
/// active line (any count; the bit period scales with it), 10-bit or any other range. The
/// decoder slices at the middle of the line's range, finds the clock run-in (at least five
/// rising edges a bit period apart) and the logic-one start bit after it, and reads the 16 data
/// bits at their centres. Returns the two bytes with their parity bits, or `None` when there is
/// no line 21 signal or a byte fails its odd parity.
pub fn decode_line21(samples: &[u16]) -> Option<(u8, u8)> {
    let n = samples.len();
    if n < 64 {
        return None;
    }
    let w = BIT_SAMPLES_720 * n as f64 / 720.0;
    let lo = *samples.iter().min()?;
    let hi = *samples.iter().max()?;
    if hi.saturating_sub(lo) < 32 {
        return None;
    }
    let thr = (f64::from(lo) + f64::from(hi)) / 2.0;
    // a box filter of about a sixth of a bit against noise
    let r = ((w / 12.0) as usize).max(1);
    let mut sum = vec![0f64; n + 1];
    for (i, &v) in samples.iter().enumerate() {
        sum[i + 1] = sum[i] + f64::from(v);
    }
    let y: Vec<f64> = (0..n).map(|i| (sum[(i + r + 1).min(n)] - sum[i.saturating_sub(r)]) / ((i + r + 1).min(n) - i.saturating_sub(r)) as f64).collect();
    // rising mid-level crossings, interpolated between samples
    let mut rises: Vec<f64> = Vec::new();
    for i in 1..n {
        let (a, b) = (y[i - 1], y[i]);
        if a < thr && b >= thr {
            rises.push(i as f64 - 1.0 + (thr - a) / (b - a));
        }
    }
    let near = |gap: f64, want: f64, tol: f64| (gap - want * w).abs() <= tol * w;
    // the clock run-in: five or more rising edges one bit period apart
    let mut i = 0;
    let last = loop {
        let j = i + rises.get(i..)?.windows(2).take_while(|p| near(p[1] - p[0], 1.0, 0.25)).count();
        if j - i >= 4 {
            break j;
        }
        i = j + 1;
        if i >= rises.len() {
            return None;
        }
    };
    let run_in_end = *rises.get(last)?;
    // the logic-one start bit: about 2.75 bit periods after the last run-in edge
    let start = *rises.iter().find(|&&r| r > run_in_end && near(r - run_in_end, 2.75, 0.6))?;
    let bit = |k: usize| -> Option<bool> {
        let c = start + w * (1.5 + k as f64);
        Some(*y.get(c.round() as usize)? >= thr)
    };
    let mut bytes = [0u8; 2];
    for (b, byte) in bytes.iter_mut().enumerate() {
        for k in 0..8 {
            if bit(b * 8 + k)? {
                *byte |= 1 << k;
            }
        }
    }
    if bytes.iter().any(|b| b.count_ones() % 2 == 0) {
        return None;
    }
    Some((bytes[0], bytes[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity() {
        assert_eq!(with_parity(0x14), 0x94);
        assert_eq!(with_parity(0x2c), 0x2c);
        assert_eq!(with_parity(0x20), 0x20);
        assert_eq!(with_parity(0x2f), 0x2f);
        assert_eq!(with_parity(0x2e), 0xae);
        for b in 0..128u8 {
            assert_eq!(with_parity(b).count_ones() % 2, 1);
        }
    }

    #[test]
    fn pac_roundtrip() {
        for row in 1..=15 {
            for indent in (0..32).step_by(4) {
                let (b1, b2) = encode_pac(row, indent);
                assert_eq!(decode_pac(b1, b2), Some((row, indent)), "row {row} indent {indent}");
            }
        }
    }

    #[test]
    fn chars() {
        assert_eq!(encode_char('A'), Some(Encoded::Basic(0x41)));
        assert_eq!(encode_char('é'), Some(Encoded::Basic(0x5c)));
        assert_eq!(encode_char('♪'), Some(Encoded::Pair { b1: 0x11, b2: 0x37, fallback: None }));
        assert!(matches!(encode_char('{'), Some(Encoded::Pair { b1: 0x13, b2: 0x29, .. })));
        assert!(matches!(encode_char('*'), Some(Encoded::Pair { b1: 0x12, b2: 0x28, .. })));
        assert_eq!(encode_char('漢'), None);
    }
}

#[cfg(test)]
mod line21_tests {
    use super::*;

    /// Waveforms made here decode at every line width and position, with noise, offsets and a
    /// smaller range; lines without the signal, and damaged ones, give `None`, never a panic.
    #[test]
    fn line21_round_trip() {
        let pairs = [(with_parity(0x14), with_parity(0x2c)), (with_parity(b'H'), with_parity(b'i')), (0x80, 0x80), (with_parity(0x7f), with_parity(0x01))];
        for &(a, b) in &pairs {
            // the 26 bit periods end inside the line, as they do in a real one (about 51.6 of 52.7 µs)
            for (n, start) in [(720, 20.0), (720, 12.3), (1440, 30.0), (1920, 40.7), (704, 15.0)] {
                let s = line21_samples(a, b, n, start);
                assert_eq!(decode_line21(&s), Some((a, b)), "{a:02x} {b:02x} n {n} start {start}");
                // noise of ±12 codes and a raised, compressed range (8-bit samples ≪ 2)
                let noisy: Vec<u16> = s.iter().enumerate().map(|(i, &v)| (v / 2 + 200 + ((i * 7919) % 25) as u16).saturating_sub(12)).collect();
                assert_eq!(decode_line21(&noisy), Some((a, b)), "noisy {a:02x} {b:02x} n {n}");
                let eight: Vec<u16> = s.iter().map(|&v| v >> 2 << 2).collect();
                assert_eq!(decode_line21(&eight), Some((a, b)));
            }
        }
        // no signal, a flat line, a short line, a run-in only, a byte with even parity
        assert_eq!(decode_line21(&vec![64; 720]), None);
        assert_eq!(decode_line21(&[]), None);
        assert_eq!(decode_line21(&[64, 940, 64]), None);
        let mut run_in = line21_samples(0x80, 0x80, 720, 20.0);
        for v in run_in.iter_mut().skip(20 + 8 * 27) {
            *v = 64;
        }
        assert_eq!(decode_line21(&run_in), None);
        assert_eq!(decode_line21(&line21_samples(0x14, 0x2c, 720, 20.0)), None, "even parity");
        for cut in 0..200 {
            let s = line21_samples(0x94, 0x2c, 720, 20.0);
            let _ = decode_line21(&s[cut * 3..]);
            let _ = decode_line21(&s[..720 - cut * 3]);
        }
    }
}
