//! KDN-20: the Kdenlive effect id → CFA4 effect table (`data/kdenlive-effect-map.csv`, described in
//! `docs/plans/CFA4-KDENLIVE-KDN20-EFFECT-MAP-001-1.0.md`) as the `.kdenlive` importer reads it. The
//! table says which CFA4 effect stands in for a Kdenlive one; it carries no parameter mapping, so
//! every use is a stand-in with default settings.

use std::collections::HashMap;
use std::sync::OnceLock;

const CSV: &str = include_str!("../data/kdenlive-effect-map.csv");

/// How well a CFA4 effect stands in for a Kdenlive one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fit {
    /// The same function; the parameters can be mapped.
    Same,
    /// A similar function; parameters or algorithm differ.
    Close,
    /// No CFA4 effect (`NONE`, `NA`): not an effect there, or not built yet.
    Missing,
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub fit: Fit,
    /// The CFA4 effect id (empty when `fit` is [`Fit::Missing`]).
    pub cfa4: String,
}

/// The table row of a Kdenlive effect id (`kdenlive_id` of a `<filter>`), `None` for an id the table
/// does not list.
pub(crate) fn effect(id: &str) -> Option<&'static Entry> {
    static TABLE: OnceLock<HashMap<String, Entry>> = OnceLock::new();
    TABLE.get_or_init(parse).get(id)
}

/// Splits one CSV record (quote-aware, `""` escapes a quote).
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn parse() -> HashMap<String, Entry> {
    let mut lines = CSV.lines();
    let header = fields(lines.next().unwrap_or(""));
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(id), Some(kind), Some(group), Some(cfa4), Some(fit)) = (col("kdenlive_id"), col("kind"), col("group"), col("cfa4_id"), col("match")) else {
        return HashMap::new();
    };
    let mut table: HashMap<String, (bool, Entry)> = HashMap::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let f = fields(line);
        let get = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        if get(kind) != "effect" || get(id).is_empty() {
            continue;
        }
        let fit = match get(fit) {
            "SAME" => Fit::Same,
            "CLOSE" => Fit::Close,
            _ => Fit::Missing,
        };
        let entry = Entry { fit, cfa4: if fit == Fit::Missing { String::new() } else { get(cfa4).to_string() } };
        // a row of a multi-filter template (group `templates`) only fills in an id no other row has
        let own = get(group) != "templates";
        match table.get(get(id)) {
            Some((true, _)) => {}
            Some((false, _)) if !own => {}
            _ => {
                table.insert(get(id).to_string(), (own, entry));
            }
        }
    }
    table.into_iter().map(|(k, (_, e))| (k, e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_table() {
        assert_eq!(effect("invert").map(|e| (e.fit, e.cfa4.as_str())), Some((Fit::Same, "invert")));
        assert_eq!(effect("boxblur").map(|e| (e.fit, e.cfa4.as_str())), Some((Fit::Close, "gaussian_blur")));
        assert_eq!(effect("shape").map(|e| e.fit), Some(Fit::Missing));
        assert!(effect("no-such-effect").is_none());
        assert!(effect("").is_none());
    }
}
