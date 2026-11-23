//! Diagnostics: one type for compile-time and run-time errors, with a stable code and a position.

use std::fmt;

/// A compile-time error or warning, or a run-time error (`R-*`). `line` and `col` are 1-based and
/// count characters; `len` is the length of the marked token in characters (0 when unknown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable code, e.g. `E-PARSE-1`, `E-LOOP-2`, `R-FUEL` (see `docs/expr-language.md` §9).
    pub code: &'static str,
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub message_hu: String,
    pub message_en: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}:{}: {}", self.code, self.line, self.col, self.message_en)
    }
}

impl std::error::Error for Diagnostic {}

pub type Result<T> = std::result::Result<T, Diagnostic>;

/// Where a node sits in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

pub(crate) fn err<T>(code: &'static str, span: Span, hu: impl Into<String>, en: impl Into<String>) -> Result<T> {
    Err(diag(code, span, hu, en))
}

pub(crate) fn diag(code: &'static str, span: Span, hu: impl Into<String>, en: impl Into<String>) -> Diagnostic {
    Diagnostic { code, line: span.line, col: span.col, len: span.len, message_hu: hu.into(), message_en: en.into() }
}
