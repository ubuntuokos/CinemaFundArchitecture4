//! KDN-20: the Kdenlive id → CFA4 effect table stays consistent with the live effect registry.

use std::collections::HashSet;

const CSV: &str = include_str!("../data/kdenlive-effect-map.csv");

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

#[test]
fn every_mapped_effect_exists_in_the_registry() {
    let mut lines = CSV.lines();
    let header = fields(lines.next().unwrap_or(""));
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(id), Some(kind), Some(file), Some(cfa), Some(matc)) = (col("kdenlive_id"), col("kind"), col("source_file"), col("cfa4_id"), col("match")) else {
        panic!("the CSV header lost a required column: {header:?}");
    };
    let mut seen = HashSet::new();
    let mut rows = 0;
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let f = fields(line);
        assert_eq!(f.len(), header.len(), "wrong column count in: {line}");
        rows += 1;
        assert!(!f[id].is_empty(), "empty kdenlive_id in: {line}");
        assert!(seen.insert((f[kind].clone(), f[id].clone(), f[file].clone())), "duplicate row: {line}");
        match f[matc].as_str() {
            "SAME" | "CLOSE" => {
                assert!(!f[cfa].is_empty(), "{} is {} but has no cfa4_id", f[id], f[matc]);
                assert!(cfa4_project::effect::find_effect(&f[cfa]).is_some(), "{} maps to unknown CFA4 effect {}", f[id], f[cfa]);
            }
            "BLEND" | "NA" | "NONE" => assert!(f[cfa].is_empty(), "{} is {} but names a cfa4_id", f[id], f[matc]),
            other => panic!("unknown match value {other:?} for {}", f[id]),
        }
    }
    assert!(rows >= 500, "the table lost rows: {rows}");
}
