//! CFX: the CFA4 restricted spatial expression language (plan `CFA4-AX-INTERPRETER-001`, X1).
//!
//! A tiny typed, deterministic language that gives a value to one voxel or one point. The language
//! has no I/O at all (no files, network, processes, clock, environment), no recursion, only loops
//! with compile-time bounds, and every run is limited by a fuel budget, so no input can crash, hang
//! or escape. Pure Rust, no `unsafe`, no dependencies. Spec: `docs/expr-language.md`.
//!
//! ```
//! use cfa4_expr::{compile, Context, Env, Schema, Ty, Value};
//! let schema = Schema { context: Context::Volume, attrs: vec![("density".into(), Ty::Float), ("mask".into(), Ty::Float)] };
//! let prog = compile("reads @density; writes @mask; @mask = @density > 0.5 ? 1.0 : 0.0;", &schema).unwrap();
//! let mut vm = prog.new_vm();
//! let inputs = [Value::Float(0.7), Value::Float(0.0)];
//! let mut outputs = [Value::Float(0.0), Value::Float(0.0)];
//! prog.run(&mut vm, 10_000, &Env::default(), &[], &inputs, &mut outputs).unwrap();
//! assert_eq!(outputs[1], Value::Float(1.0));
//! ```
//!
//! Determinism: the result of a run depends only on its inputs, never on thread count or timing.
//! Floating-point library functions (`sin`, `exp`, …) come from the platform's math library, so
//! results are bit-identical on one build and platform; across platforms they are not guaranteed.

#![forbid(unsafe_code)]

mod ast;
mod compile;
mod error;
mod lex;
mod parse;
mod types;
mod vm;

#[cfg(test)]
mod tests;

pub use compile::Program;
pub use error::{Diagnostic, Result, Span};
pub use types::{Context, Env, ParamDecl, RunStats, Schema, Ty, Value};
pub use vm::{DEFAULT_FUEL, MAX_FUEL, Vm};

/// Largest accepted source, in bytes.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;

/// Parse, check and compile one expression for the given attribute schema. Stops at the first error.
pub fn compile(source: &str, schema: &Schema) -> Result<Program> {
    if source.len() > MAX_SOURCE_BYTES {
        return error::err("E-LIMIT-1", Span::default(), "a forrás túl nagy (legfeljebb 64 KiB)", "source too large (at most 64 KiB)");
    }
    let toks = lex::lex(source)?;
    let parsed = parse::Parser::new(toks).parse_program()?;
    compile::compile_program(parsed, schema)
}
