//! CFA4 common audit trail, layer L1 (CFA4-AUDIT-001, phase AU-1; [ADR 0008](../../../docs/adr/ADR-0008-cfa4-audit.md)).
//!
//! This crate has **no database**. It owns the one thing the Rust and the Python side of the audit
//! trail must agree on, byte for byte: the **canonical form** of an event and its **hash**
//! ([`canonical_bytes`], [`event_hash`]). The first Rust writer (Event Fabric, EF-R1) will add the
//! database driver later; until then nothing here has a caller that is not a test.
//!
//! # Canonical form
//!
//! RFC 8785 (JSON Canonicalization Scheme), narrowed on purpose:
//!
//! - **No floating-point number**: a number must be an integer in the `int64` range, otherwise
//!   [`AuditError::FloatForbidden`] (code `AUDIT_FLOAT_FORBIDDEN`). Money and ratios are decimal
//!   *strings*. This keeps the number serialisation differences of the two languages out of the hash.
//! - Object keys are sorted by their UTF-16 code units (RFC 8785 3.2.3), not by code point.
//! - Strings are escaped minimally: `"` and `\` get a backslash, the control characters below U+0020
//!   become `\b \t \n \f \r` or `\u00xx` (lower-case hex); everything else, including non-ASCII and `/`,
//!   stays as UTF-8.
//! - No whitespace.
//!
//! # Hash
//!
//! `hash = SHA-256("cfa4-audit/v1" || 0x00 || canonical bytes)`. [`event_hash`] hashes every field of
//! the event object except `hash` itself (so `seq` and `prev_hash` are inside the hash).
//!
//! The shared test vectors are `tests/vectors/audit/v1/*.json`; the Python side reads the same files.

use std::cmp::Ordering;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Domain separator in front of the canonical bytes; bumped with the event `schema_ver`.
pub const HASH_DOMAIN_V1: &[u8] = b"cfa4-audit/v1";

/// `prev_hash` of the first event of a chain: 32 zero bytes.
pub const GENESIS_PREV_HASH: [u8; 32] = [0; 32];

/// What can go wrong; every variant has a stable English code (the UI translates it).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditError {
    /// A number that is not an integer in the `int64` range.
    #[error("AUDIT_FLOAT_FORBIDDEN: only integers in the int64 range are allowed in an audit event")]
    FloatForbidden,
    /// The event is not a JSON object.
    #[error("AUDIT_EVENT_NOT_OBJECT: an audit event must be a JSON object")]
    NotObject,
}

impl AuditError {
    /// The stable error code.
    pub fn code(&self) -> &'static str {
        match self {
            AuditError::FloatForbidden => "AUDIT_FLOAT_FORBIDDEN",
            AuditError::NotObject => "AUDIT_EVENT_NOT_OBJECT",
        }
    }
}

/// The canonical bytes of a JSON value (see the crate documentation).
pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>, AuditError> {
    let mut out = Vec::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// The hash of an event object: every field except `hash`.
pub fn event_hash(event: &Value) -> Result<[u8; 32], AuditError> {
    let Value::Object(map) = event else { return Err(AuditError::NotObject) };
    let mut hashed = map.clone();
    hashed.remove("hash");
    hash_canonical(&canonical_bytes(&Value::Object(hashed))?)
}

/// `SHA-256(domain || 0x00 || canonical)` of already canonical bytes.
fn hash_canonical(canonical: &[u8]) -> Result<[u8; 32], AuditError> {
    let mut h = Sha256::new();
    h.update(HASH_DOMAIN_V1);
    h.update([0u8]);
    h.update(canonical);
    Ok(h.finalize().into())
}

/// Lower-case hex of a hash.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        s.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
    }
    s
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<(), AuditError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => match n.as_i64() {
            Some(i) => out.extend_from_slice(i.to_string().as_bytes()),
            None => return Err(AuditError::FloatForbidden),
        },
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| utf16_cmp(a.0, b.0));
            out.push(b'{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_value(item, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// RFC 8785 3.2.3: compare strings by their UTF-16 code units.
fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            '\r' => out.extend_from_slice(b"\\r"),
            c if (c as u32) < 0x20 => out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes()),
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_sort_by_utf16_code_units_not_code_points() {
        // U+FB33 sorts after the emoji U+1F600 (surrogates D83D DE00) in UTF-16, before it by code point.
        let v = json!({"\u{fb33}": 1, "\u{1f600}": 2});
        assert_eq!(String::from_utf8(canonical_bytes(&v).unwrap()).unwrap(), "{\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn floats_are_rejected_even_when_integral() {
        assert_eq!(canonical_bytes(&json!({"a": 1.0})), Err(AuditError::FloatForbidden));
        assert_eq!(canonical_bytes(&json!([1.5])), Err(AuditError::FloatForbidden));
        assert_eq!(canonical_bytes(&json!({"a": u64::MAX})), Err(AuditError::FloatForbidden));
        assert_eq!(AuditError::FloatForbidden.code(), "AUDIT_FLOAT_FORBIDDEN");
    }

    #[test]
    fn int64_extremes_are_kept_exact() {
        let v = json!([i64::MIN, i64::MAX, 0, -1]);
        assert_eq!(canonical_bytes(&v).unwrap(), b"[-9223372036854775808,9223372036854775807,0,-1]");
    }

    #[test]
    fn strings_escape_only_what_the_rfc_requires() {
        let v = json!("a\"b\\c/\u{8}\t\n\u{c}\r\u{1}\u{1f}\u{7f}é");
        assert_eq!(String::from_utf8(canonical_bytes(&v).unwrap()).unwrap(), "\"a\\\"b\\\\c/\\b\\t\\n\\f\\r\\u0001\\u001f\u{7f}é\"");
    }

    #[test]
    fn the_hash_ignores_the_hash_field_and_covers_everything_else() {
        let base = json!({"seq": 1, "prev_hash": "00", "action": "audit.chain.genesis"});
        let h = event_hash(&base).unwrap();
        let mut with_hash = base.clone();
        with_hash["hash"] = json!(to_hex(&h));
        assert_eq!(event_hash(&with_hash).unwrap(), h);
        let mut changed = base.clone();
        changed["seq"] = json!(2);
        assert_ne!(event_hash(&changed).unwrap(), h);
    }

    #[test]
    fn a_non_object_is_not_an_event() {
        assert_eq!(event_hash(&json!([1])), Err(AuditError::NotObject));
    }

    #[test]
    fn hex_is_lower_case_and_fixed_width() {
        assert_eq!(to_hex(&[0, 1, 0xab, 0xff]), "0001abff");
    }
}
