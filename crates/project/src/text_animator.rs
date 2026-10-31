//! Text animators of text layers (Graphics and Titles ▸ Text Animator): the text a layer shows
//! changes over the first seconds of the clip, while the Source Text stays what was typed.
//!
//! - **Counter**: the first number in the text counts up from 0 to its value (an odometer for
//!   stats: "432,014,696 views"), easing out; thousands separators and decimals are kept.
//! - **Typewriter**: characters appear one after another.
//! - **Word by Word**: words appear one after another.
//!
//! An animator runs from `anim_delay` for `anim_duration` seconds of clip time; before it the
//! layer shows the start state, after it the Source Text unchanged. Everything is a pure function
//! of the text and the time, so frames render identically on every machine.

/// Animator names, in the order of the layer's `anim` choice.
pub const ANIMATOR_OPTS: &[&str] = &["None", "Counter", "Typewriter", "Word by Word"];

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
