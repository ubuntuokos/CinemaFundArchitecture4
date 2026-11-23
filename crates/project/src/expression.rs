//! Saved CFX expressions (plan CFA4-AX-INTERPRETER-001, X3).
//!
//! An [`Expression`] is the source text of a CFX program (`docs/expr-language.md`) plus what a run needs besides the
//! volume: the grid it runs over, the attributes it creates, its parameter values and an optional fuel budget. The
//! project keeps the *text and its hash* only. Nothing here compiles or runs anything, and **whether this machine
//! trusts the text is not stored in the project**: the engine keeps that per machine, keyed by [`Expression::hash`]
//! (plan §5.1, N3-7′), so a project that arrives from elsewhere never runs an expression until its user approves it.
//!
//! Everything read from a file is hostile (AGENTS.md §0.4): [`Expression`] validates on deserialization, every size is
//! bounded, and the stored hash must match the text, so a hash can never be used to claim an approval for other text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Longest accepted source, in bytes (the same limit as `cfa4_expr::MAX_SOURCE_BYTES`).
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// Most outputs, and most parameters, one expression may name.
pub const MAX_NAMES: usize = 64;
/// Longest name (expression, grid, output or parameter) in bytes.
pub const MAX_NAME: usize = 256;
/// Most expressions a project may keep.
pub const MAX_EXPRESSIONS: usize = 4096;

/// Stable id of a saved expression within its project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExprId(pub u64);

/// Why an expression value was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExprError {
    #[error("the source is larger than {MAX_SOURCE_BYTES} bytes")]
    SourceTooLarge,
    #[error("`{what}` is empty or longer than {MAX_NAME} bytes")]
    BadName { what: &'static str },
    #[error("too many {what} (at most {MAX_NAMES})")]
    TooMany { what: &'static str },
    #[error("output `{0}`: the type is `float` or `vec3`")]
    BadOutputType(String),
    #[error("parameter `{0}`: a finite number, a boolean or [x, y, z]")]
    BadParam(String),
    #[error("the stored hash `{0}` does not belong to the source text")]
    HashMismatch(String),
    #[error("fuel must be at least 1")]
    BadFuel,
}

/// SHA-256 of the source text, as 64 lowercase hex characters: the identity of an expression for approvals.
#[must_use]
pub fn source_hash(source: &str) -> String {
    Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME
}

fn param_ok(v: &Value) -> bool {
    match v {
        Value::Bool(_) => true,
        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
        Value::Array(a) => a.len() == 3 && a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)),
        _ => false,
    }
}

/// One saved expression: text, hash and run settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ExpressionRaw")]
pub struct Expression {
    pub id: ExprId,
    /// Shown in lists.
    pub name: String,
    /// The CFX program text.
    pub source: String,
    /// [`source_hash`] of `source`.
    pub hash: String,
    /// The grid the expression runs over.
    pub domain: String,
    /// Attributes the expression creates: `(name, "float" | "vec3")`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<(String, String)>,
    /// Parameter values by name (numbers, booleans or `[x, y, z]`); the others take their declared defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    /// Instruction budget per voxel; none = the language default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel: Option<u32>,
}

#[derive(Deserialize)]
struct ExpressionRaw {
    id: ExprId,
    name: String,
    source: String,
    hash: String,
    domain: String,
    #[serde(default)]
    outputs: Vec<(String, String)>,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    fuel: Option<u32>,
}

impl TryFrom<ExpressionRaw> for Expression {
    type Error = ExprError;
    fn try_from(r: ExpressionRaw) -> Result<Self, ExprError> {
        let e = Expression { id: r.id, name: r.name, source: r.source, hash: r.hash, domain: r.domain, outputs: r.outputs, params: r.params, fuel: r.fuel };
        e.validate()?;
        Ok(e)
    }
}

impl Expression {
    /// A checked new expression (the hash is computed from `source`).
    ///
    /// # Errors
    /// [`ExprError`] when a size, name, type or number is out of range.
    pub fn new(
        id: ExprId,
        name: impl Into<String>,
        source: impl Into<String>,
        domain: impl Into<String>,
        outputs: Vec<(String, String)>,
        params: BTreeMap<String, Value>,
        fuel: Option<u32>,
    ) -> Result<Self, ExprError> {
        let source = source.into();
        let hash = source_hash(&source);
        let e = Expression { id, name: name.into(), source, hash, domain: domain.into(), outputs, params, fuel };
        e.validate()?;
        Ok(e)
    }

    /// Checks every bound; also run on every value read from a file.
    ///
    /// # Errors
    /// [`ExprError`] for the first problem found.
    pub fn validate(&self) -> Result<(), ExprError> {
        if self.source.len() > MAX_SOURCE_BYTES {
            return Err(ExprError::SourceTooLarge);
        }
        if !name_ok(&self.name) {
            return Err(ExprError::BadName { what: "name" });
        }
        if !name_ok(&self.domain) {
            return Err(ExprError::BadName { what: "domain" });
        }
        if self.outputs.len() > MAX_NAMES {
            return Err(ExprError::TooMany { what: "outputs" });
        }
        for (n, t) in &self.outputs {
            if !name_ok(n) {
                return Err(ExprError::BadName { what: "output" });
            }
            if t != "float" && t != "vec3" {
                return Err(ExprError::BadOutputType(n.clone()));
            }
        }
        if self.params.len() > MAX_NAMES {
            return Err(ExprError::TooMany { what: "parameters" });
        }
        for (n, v) in &self.params {
            if !name_ok(n) || !param_ok(v) {
                return Err(ExprError::BadParam(n.chars().take(64).collect()));
            }
        }
        if self.fuel == Some(0) {
            return Err(ExprError::BadFuel);
        }
        if self.hash != source_hash(&self.source) {
            return Err(ExprError::HashMismatch(self.hash.chars().take(64).collect()));
        }
        Ok(())
    }
}
