//! Text animators of text layers (Graphics and Titles ▸ Text Animator): the text a layer shows
//! changes over the first seconds of the clip, while the Source Text stays what was typed.
//!
//! - **Counter**: the first number in the text counts up from 0 to its value (an odometer for
//!   stats: "432,014,696 views"), easing out; thousands separators and decimals are kept.
//! - **Typewriter**: characters appear one after another.
//! - **Word by Word**: words appear one after another.
//! - **Scramble**: the text starts as random symbols that settle into the real characters from
//!   left to right (a "decode" look); spaces and line breaks stay where they are, so the layout
//!   does not jump. The symbols come from a fixed hash of the character position and the
//!   animation step, not from a random generator, so scrubbing shows the same frame every time.
//!
//! An animator runs from `anim_delay` for `anim_duration` seconds of clip time; before it the
//! layer shows the start state, after it the Source Text unchanged. Everything is a pure function
//! of the text and the time, so frames render identically on every machine.

/// Animator names, in the order of the layer's `anim` choice.
pub const ANIMATOR_OPTS: &[&str] = &["None", "Counter", "Typewriter", "Word by Word", "Scramble"];

/// Symbols the Scramble animator shows for characters that have not settled yet (ASCII only,
/// so every font has them).
const SCRAMBLE_GLYPHS: &[u8] = b"!<>-_\\/[]{}=+*^?#%&@$~01";

/// How many times the Scramble symbols change over the animation.
const SCRAMBLE_STEPS: f64 = 48.0;

/// Longest number (characters) the counter reads; longer digit runs are left alone.
const MAX_NUMBER_CHARS: usize = 40;

/// The first number of `text`: (byte range, value, decimals, thousands separator, decimal mark).
fn find_number(text: &str) -> Option<(std::ops::Range<usize>, f64, usize, Option<char>, char)> {
    let b = text.as_bytes();
    let start = b.iter().position(u8::is_ascii_digit)?;
    // the run of digits and separators that ends on a digit
    let mut end = start;
    let mut i = start;
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(0);
        if c.is_ascii_digit() {
            end = i + 1;
        } else if !(matches!(c, b',' | b'.' | b' ' | b'\'') && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            break;
        }
        i += 1;
    }
    let s = text.get(start..end)?;
    if s.len() > MAX_NUMBER_CHARS {
        return None;
    }
    let seps: Vec<(usize, char)> = s.char_indices().filter(|(_, c)| !c.is_ascii_digit()).collect();
    // the decimal mark: the last separator when it is followed by other than exactly 3 digits,
    // or when it differs from the others (1.234,5)
    let (dec, thousands) = match seps.last() {
        None => (None, None),
        Some(&(at, c)) => {
            let after = s.len() - at - 1;
            let others: Vec<char> = seps.iter().rev().skip(1).map(|(_, c)| *c).collect();
            let is_dec = (after != 3 && matches!(c, '.' | ',')) || (!others.is_empty() && others.iter().all(|o| *o != c) && matches!(c, '.' | ','));
            if is_dec { (Some((at, c)), others.first().copied()) } else { (None, Some(c)) }
        }
    };
    let digits: String = s
        .char_indices()
        .filter(|(i, c)| c.is_ascii_digit() || dec.is_some_and(|(d, _)| d == *i))
        .map(|(_, c)| if c.is_ascii_digit() { c } else { '.' })
        .collect();
    let value: f64 = digits.parse().ok()?;
    let decimals = dec.map_or(0, |(at, _)| s.len() - at - 1);
    Some((start..end, value, decimals, thousands, dec.map_or('.', |(_, c)| c)))
}

fn format_number(v: f64, decimals: usize, thousands: Option<char>, mark: char) -> String {
    let s = format!("{:.*}", decimals.min(12), v.max(0.0));
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut out = String::with_capacity(s.len() + int.len() / 3);
    let n = int.len();
    for (i, c) in int.chars().enumerate() {
        if i > 0
            && (n - i) % 3 == 0
            && let Some(t) = thousands
        {
            out.push(t);
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(mark);
        out.push_str(frac);
    }
    out
}

/// SplitMix64 finaliser: a fixed, well-mixed hash of `x`.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// The Scramble animator: the first `round(n·p)` characters are settled, the rest are symbols
/// that change [`SCRAMBLE_STEPS`] times over the animation. Whitespace is never replaced.
fn scramble(text: &str, p: f64) -> String {
    let n = text.chars().count();
    // n ≤ text length, so the product is finite; `as` saturates and is clamped by `n` anyway
    let settled = ((n as f64) * p).floor() as usize;
    let step = (p * SCRAMBLE_STEPS).floor() as u64;
    let glyphs = SCRAMBLE_GLYPHS.len().max(1) as u64;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < settled || c.is_whitespace() {
                c
            } else {
                let h = mix((i as u64) ^ step.wrapping_mul(0x1_0000_0001));
                let g = usize::try_from(h % glyphs).unwrap_or(0);
                SCRAMBLE_GLYPHS.get(g).map_or('#', |b| char::from(*b))
            }
        })
        .collect()
}

/// Ease-out cubic.
fn ease_out(p: f64) -> f64 {
    1.0 - (1.0 - p).powi(3)
}

/// The text an animator shows at `progress` (0 = start, 1 = done; clamped).
pub fn animate(kind: u32, text: &str, progress: f64) -> String {
    let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 1.0 };
    if p >= 1.0 {
        return text.to_string();
    }
    match kind {
        1 => match find_number(text) {
            Some((range, value, decimals, thousands, mark)) => {
                let shown = format_number(value * ease_out(p), decimals, thousands, mark);
                format!("{}{}{}", text.get(..range.start).unwrap_or(""), shown, text.get(range.end..).unwrap_or(""))
            }
            None => text.to_string(),
        },
        2 => {
            let n = text.chars().count();
            let k = ((n as f64) * p).floor() as usize;
            text.chars().take(k).collect()
        }
        3 => {
            // word starts (byte offsets of the first character after whitespace)
            let mut starts = Vec::new();
            let mut prev_space = true;
            for (i, c) in text.char_indices() {
                if !c.is_whitespace() && prev_space {
                    starts.push(i);
                }
                prev_space = c.is_whitespace();
            }
            let k = ((starts.len() as f64) * p).floor() as usize;
            match starts.get(k) {
                Some(&cut) => text.get(..cut).unwrap_or("").trim_end().to_string(),
                None => text.to_string(),
            }
        }
        4 => scramble(text, p),
        _ => text.to_string(),
    }
}

/// Progress of an animator at clip time `seconds`.
pub fn progress(seconds: f64, delay: f64, duration: f64) -> f64 {
    if duration.is_nan() || duration <= 1e-6 {
        return if seconds >= delay { 1.0 } else { 0.0 };
    }
    ((seconds - delay) / duration).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_keeps_the_number_format() {
        assert_eq!(animate(1, "432,014,696 views", 1.0), "432,014,696 views");
        assert_eq!(animate(1, "432,014,696 views", 0.0), "0 views");
        let mid = animate(1, "432,014,696 views", 0.5);
        assert!(mid.ends_with(" views") && mid.contains(','), "{mid}");
        assert_eq!(animate(1, "$1,250.50 saved", 0.0), "$0.00 saved");
        assert_eq!(animate(1, "Up 12.5%", 0.0), "Up 0.0%");
        assert_eq!(animate(1, "1.234.567 Nutzer", 0.0), "0 Nutzer");
        assert!(animate(1, "1.234.567 Nutzer", 0.999).starts_with("1.234."), "{}", animate(1, "1.234.567 Nutzer", 0.999));
        assert_eq!(animate(1, "1 234,5 €", 0.0), "0,0 €");
        assert_eq!(animate(1, "no numbers", 0.3), "no numbers");
        // values count up and never pass the target
        let mut last = -1.0;
        for i in 0..=20 {
            let s = animate(1, "1000", i as f64 / 20.0);
            let v: f64 = s.parse().unwrap();
            assert!(v >= last && v <= 1000.0);
            last = v;
        }
    }

    #[test]
    fn typewriter_and_words() {
        assert_eq!(animate(2, "héllo", 0.0), "");
        assert_eq!(animate(2, "héllo", 0.4), "hé");
        assert_eq!(animate(2, "héllo", 1.0), "héllo");
        assert_eq!(animate(3, "one two  three", 0.0), "");
        assert_eq!(animate(3, "one two  three", 0.34), "one");
        assert_eq!(animate(3, "one two  three", 0.7), "one two");
        assert_eq!(animate(3, "one two  three", 1.0), "one two  three");
        assert_eq!(animate(0, "same", 0.0), "same");
        assert_eq!(animate(9, "same", 0.0), "same");
    }

    #[test]
    fn scramble_settles_left_to_right_and_keeps_the_layout() {
        let text = "Hello, wörld\nline two";
        assert_eq!(animate(4, text, 1.0), text);
        for i in 0..=40 {
            let p = f64::from(i) / 40.0;
            let s = animate(4, text, p);
            // same number of characters, whitespace in the same places
            assert_eq!(s.chars().count(), text.chars().count(), "{p}: {s}");
            for (a, b) in s.chars().zip(text.chars()) {
                assert_eq!(a.is_whitespace(), b.is_whitespace(), "{p}: {s}");
                if b.is_whitespace() {
                    assert_eq!(a, b);
                }
            }
            // the settled prefix is the real text
            let settled = ((text.chars().count() as f64) * p).floor() as usize;
            assert!(s.chars().zip(text.chars()).take(settled).all(|(a, b)| a == b), "{p}: {s}");
        }
        // at the start nothing has settled: no real letter is shown
        let start = animate(4, "ABCDEFGHIJ", 0.0);
        assert!(start.chars().all(|c| u8::try_from(c).is_ok_and(|b| SCRAMBLE_GLYPHS.contains(&b))), "{start}");
    }

    #[test]
    fn scramble_is_deterministic_and_flickers_between_steps() {
        let a = animate(4, "deterministic text", 0.1);
        assert_eq!(a, animate(4, "deterministic text", 0.1));
        // a later step shows other symbols for the characters that are still unsettled
        let b = animate(4, "deterministic text", 0.1 + 1.0 / SCRAMBLE_STEPS);
        assert_ne!(a, b);
    }

    #[test]
    fn hostile_input_never_panics() {
        let long = "9".repeat(10_000);
        for k in 0..5 {
            for p in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.5, 2.0] {
                for t in ["", " ", "1,", ",1", "1,,2", "..5", "99999999999999999999999999", long.as_str(), "日本 1,000 円"] {
                    let _ = animate(k, t, p);
                }
            }
        }
        assert_eq!(progress(0.5, 0.0, 0.0), 1.0);
        assert_eq!(progress(-0.5, 0.0, f64::NAN), 0.0);
        assert_eq!(progress(0.5, 0.0, 1.0), 0.5);
    }
}
