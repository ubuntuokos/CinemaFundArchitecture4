//! Reading a reference list: leaf paths from any of the accepted JSON shapes, label
//! normalisation and the out-of-scope rules.

use super::*;

/// Leaf paths (top-level menu or folder first, item label last) of a reference file.
pub(super) fn reference_paths(v: &Value, area: Area) -> Result<Vec<Vec<String>>, String> {
    let mut root = v;
    if let Value::Object(o) = v
        && label_of(o).is_none()
        && let Some(inner) = area.wrapper_keys().iter().find_map(|k| o.get(*k).filter(|c| c.is_array() || c.is_object()))
    {
        root = inner;
    }
    let mut out = Vec::new();
    walk(root, &[], 0, area.min_len(), &mut out)?;
    Ok(out)
}

pub(super) fn label_of(o: &serde_json::Map<String, Value>) -> Option<&str> {
    LABEL_KEYS.iter().find_map(|k| o.get(*k)?.as_str())
}

/// A separator or placeholder rather than a menu item.
pub(super) fn is_separator(label: &str) -> bool {
    let l = label.trim();
    l.is_empty() || l.chars().all(|c| matches!(c, '-' | '—' | '–' | '_')) || l.eq_ignore_ascii_case("missing value") || l.eq_ignore_ascii_case("separator")
}

pub(super) fn push_leaf(prefix: &[String], label: &str, min: usize, out: &mut Vec<Vec<String>>) -> Result<(), String> {
    if is_separator(label) || prefix.iter().any(|p| is_separator(p)) {
        return Ok(());
    }
    if out.len() >= MAX_ITEMS {
        return Err(format!("reference has more than {MAX_ITEMS} items"));
    }
    let bytes = prefix.iter().try_fold(label.trim().len(), |n, part| n.checked_add(part.len())).ok_or("reference path is too large")?;
    if bytes > MAX_PATH_BYTES {
        return Err(format!("reference path has more than {MAX_PATH_BYTES} bytes"));
    }
    let mut path = prefix.to_vec();
    path.push(label.trim().to_string());
    if path.len() >= min.max(1) {
        out.push(path);
    }
    Ok(())
}

pub(super) fn split_path(s: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = s.replace('\u{a0}', " ").split(['>', '▸']).map(|p| p.trim().to_string()).collect();
    (parts.len() >= 2 && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

pub(super) fn walk(v: &Value, prefix: &[String], depth: usize, min: usize, out: &mut Vec<Vec<String>>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("reference is nested deeper than {MAX_DEPTH} levels"));
    }
    let with = |label: &str| {
        let mut p = prefix.to_vec();
        p.push(label.trim().to_string());
        p
    };
    match v {
        Value::String(s) => match split_path(s).filter(|_| prefix.is_empty()) {
            Some(parts) => match parts.split_last() {
                Some((label, menus)) => push_leaf(menus, label, min, out),
                None => Ok(()),
            },
            None => push_leaf(prefix, s, min, out),
        },
        Value::Array(a) => {
            for el in a {
                match el {
                    Value::Array(path) if !path.is_empty() && path.iter().all(Value::is_string) => {
                        let parts: Vec<String> = prefix.iter().cloned().chain(path.iter().filter_map(|p| p.as_str().map(str::to_string))).collect();
                        if let Some((label, menus)) = parts.split_last() {
                            push_leaf(menus, label, min, out)?;
                        }
                    }
                    _ => walk(el, prefix, depth + 1, min, out)?,
                }
            }
            Ok(())
        }
        Value::Object(o) => match label_of(o) {
            Some(label) => {
                let children = CHILD_KEYS.iter().find_map(|k| o.get(*k).filter(|c| has_entries(c)));
                match children {
                    Some(c) if !is_separator(label) => walk(c, &with(label), depth + 1, min, out),
                    Some(_) => Ok(()),
                    None => push_leaf(prefix, label, min, out),
                }
            }
            None => {
                for (k, val) in o {
                    if has_entries(val) {
                        walk(val, &with(k), depth + 1, min, out)?;
                    } else {
                        push_leaf(prefix, k, min, out)?;
                    }
                }
                Ok(())
            }
        },
        _ => Ok(()),
    }
}

pub(super) fn has_entries(v: &Value) -> bool {
    match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => false,
    }
}

/// A label compared case-insensitively, without ellipsis, with `&` as "and" and one space.
pub(super) fn norm(label: &str) -> String {
    let l = label.to_lowercase().replace('…', "").replace("...", "").replace(['’', '‘'], "'").replace('&', " and ");
    l.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why a reference item is not counted, if it isn't.
pub(super) fn out_of_scope(area: Area, path: &[String]) -> Option<&'static str> {
    match area {
        Area::Menus | Area::MenusEgui => menu_out_of_scope(path),
        Area::Commands | Area::Panels => None,
        Area::Effects | Area::Transitions => {
            let top = norm(path.first()?);
            if path.len() >= 2 && (top == "presets" || top == "lumetri presets") {
                return Some("presets");
            }
            let folders = path.get(..path.len().saturating_sub(1)).unwrap_or_default();
            let transition = folders.iter().any(|f| norm(f).ends_with("transitions"));
            match area {
                Area::Effects if transition => Some("transitions (--area transitions)"),
                Area::Transitions if !folders.is_empty() && !transition => Some("effects (--area effects)"),
                _ => None,
            }
        }
    }
}

pub(super) fn menu_out_of_scope(path: &[String]) -> Option<&'static str> {
    let (top, label) = (norm(path.first()?), norm(path.last()?));
    if top == "apple" || top == "help" || top.contains("premiere") {
        return Some("app, Apple and Help menus");
    }
    let parents = path.get(..path.len().saturating_sub(1)).unwrap_or_default();
    if parents.iter().skip(1).any(|p| norm(p).starts_with("open recent") || norm(p).starts_with("recent")) {
        return Some("recent-file lists");
    }
    if ["adobe", "creative cloud", "sign in", "sign out", "log in", "log out"].iter().any(|w| label.contains(w)) {
        return Some("account and cloud items");
    }
    if ["autofill", "start dictation", "emoji and symbols", "writing tools", "services"].iter().any(|w| label.starts_with(w)) {
        return Some("macOS system items");
    }
    None
}
