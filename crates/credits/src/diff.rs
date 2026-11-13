//! What changed in a credits spreadsheet since it was last accepted, row by row, and the table
//! that results from accepting some of the changes.
//!
//! Rows are compared as written (trailing empty cells ignored). Common leading and trailing rows
//! are matched first; the rows in between by longest common subsequence. A run of removed and
//! added rows at the same place is one *change*:
//!
//! - **added**: rows only in the new sheet;
//! - **removed**: rows only in the accepted table;
//! - **changed**: rows replaced by other rows at the same place.
//!
//! [`merge`] applies the accepted changes and keeps the accepted table elsewhere, so nothing in
//! the credits changes without the user saying so.

use serde::Serialize;

/// Above this many cell comparisons the middle of the table is treated as one change instead
/// of running the quadratic matcher (a hostile or completely rewritten sheet).
const MAX_LCS_CELLS: usize = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

/// One change. Rows are 0-based indices; `old_rows` in the accepted table, `new_rows` in the
/// new sheet (an empty range where the other side has none: the place it would go).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: usize,
    pub kind: ChangeKind,
    pub old_rows: (usize, usize),
    pub new_rows: (usize, usize),
    pub before: Vec<Vec<String>>,
    pub after: Vec<Vec<String>>,
}

fn norm(row: &[String]) -> &[String] {
    let n = row.iter().rposition(|c| !c.trim().is_empty()).map_or(0, |i| i + 1);
    row.get(..n).unwrap_or(row)
}

fn same(a: &[String], b: &[String]) -> bool {
    norm(a) == norm(b)
}

/// Pairs of equal rows `(old, new)` in order, for the whole tables.
fn matches(old: &[Vec<String>], new: &[Vec<String>]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let mut pre = 0;
    while pre < old.len() && pre < new.len() && old.get(pre).zip(new.get(pre)).is_some_and(|(a, b)| same(a, b)) {
        pairs.push((pre, pre));
        pre += 1;
    }
    let mut suf = 0;
    while suf < old.len() - pre && suf < new.len() - pre {
        let (a, b) = (old.get(old.len() - 1 - suf), new.get(new.len() - 1 - suf));
        if !a.zip(b).is_some_and(|(a, b)| same(a, b)) {
            break;
        }
        suf += 1;
    }
    let (o, n) = (old.get(pre..old.len() - suf).unwrap_or(&[]), new.get(pre..new.len() - suf).unwrap_or(&[]));
    if !o.is_empty() && !n.is_empty() && o.len().saturating_mul(n.len()) <= MAX_LCS_CELLS {
        // LCS lengths, from the end
        let w = n.len() + 1;
        let mut t = vec![0u32; (o.len() + 1) * w];
        for i in (0..o.len()).rev() {
            for j in (0..n.len()).rev() {
                let v = if o.get(i).zip(n.get(j)).is_some_and(|(a, b)| same(a, b)) {
                    t.get((i + 1) * w + j + 1).copied().unwrap_or(0) + 1
                } else {
                    t.get((i + 1) * w + j).copied().unwrap_or(0).max(t.get(i * w + j + 1).copied().unwrap_or(0))
                };
                if let Some(c) = t.get_mut(i * w + j) {
                    *c = v;
                }
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < o.len() && j < n.len() {
            if o.get(i).zip(n.get(j)).is_some_and(|(a, b)| same(a, b)) {
                pairs.push((pre + i, pre + j));
                i += 1;
                j += 1;
            } else if t.get((i + 1) * w + j).copied().unwrap_or(0) >= t.get(i * w + j + 1).copied().unwrap_or(0) {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    for k in 0..suf {
        pairs.push((old.len() - suf + k, new.len() - suf + k));
    }
    pairs
}

/// The table without its trailing empty rows (they mean nothing; empty rows inside the table
/// are gaps and do count).
fn trimmed(rows: &[Vec<String>]) -> &[Vec<String>] {
    let n = rows.iter().rposition(|r| !norm(r).is_empty()).map_or(0, |i| i + 1);
    rows.get(..n).unwrap_or(rows)
}

/// The changes from `old` (accepted) to `new` (the sheet now).
pub fn diff(old: &[Vec<String>], new: &[Vec<String>]) -> Vec<Change> {
    let (old, new) = (trimmed(old), trimmed(new));
    let mut out = Vec::new();
    let (mut oi, mut ni) = (0usize, 0usize);
    let push = |o0: usize, o1: usize, n0: usize, n1: usize, out: &mut Vec<Change>| {
        if o0 == o1 && n0 == n1 {
            return;
        }
        let kind = match (o1 > o0, n1 > n0) {
            (true, true) => ChangeKind::Changed,
            (true, false) => ChangeKind::Removed,
            _ => ChangeKind::Added,
        };
        out.push(Change {
            id: out.len(),
            kind,
            old_rows: (o0, o1),
            new_rows: (n0, n1),
            before: old.get(o0..o1).unwrap_or(&[]).to_vec(),
            after: new.get(n0..n1).unwrap_or(&[]).to_vec(),
        });
    };
    for (o, n) in matches(old, new) {
        push(oi, o, ni, n, &mut out);
        oi = o + 1;
        ni = n + 1;
    }
    push(oi, old.len(), ni, new.len(), &mut out);
    out
}

/// The accepted table with the changes whose id is in `accept` applied.
pub fn merge(old: &[Vec<String>], new: &[Vec<String>], changes: &[Change], accept: &[usize]) -> Vec<Vec<String>> {
    let (old, _new) = (trimmed(old), trimmed(new));
    let mut out = Vec::with_capacity(old.len().max(new.len()));
    let mut oi = 0usize;
    for c in changes {
        // unchanged rows before this change
        out.extend(old.get(oi..c.old_rows.0).unwrap_or(&[]).iter().cloned());
        if accept.contains(&c.id) {
            out.extend(c.after.iter().cloned());
        } else {
            out.extend(c.before.iter().cloned());
        }
        oi = c.old_rows.1;
    }
    out.extend(old.get(oi..).unwrap_or(&[]).iter().cloned());
    out
}
