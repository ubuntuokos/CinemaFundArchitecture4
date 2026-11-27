//! CFA4 expression engine (CFA4-VFX-001 VFX-4, decision D-2: one expression core).
//!
//! After Effects-style property expressions on the boa JavaScript engine, ported from the
//! EC-donor (#11) expression crate (MIT OR Apache-2.0; `docs/licensing.md` §5) and changed for
//! CFA4. The crate knows no project model: the data an expression reads comes through [`Host`].
//!
//! * **Language**: JavaScript plus AE's legacy array maths (`value + [10, 0]`, `[1, 2] * 2`),
//!   implemented by rewriting `+ - * /` into helper calls ([`rewrite`]). The last evaluated
//!   statement is the result (`var x = …; x`, `if/else` blocks).
//! * **Object model** (`prelude.js`): `time`, `value`, `index`, `frameDuration`, `thisProperty`
//!   and `prop(path)`; property methods `valueAtTime`, `velocityAtTime`, `speedAtTime`, `key`,
//!   `nearestKey`, `numKeys`, `wiggle`, `temporalWiggle`, `smooth`, `loopIn/Out[Duration]`;
//!   global helpers `linear`/`ease`/`easeIn`/`easeOut`, `random`/`gaussRandom`/`seedRandom`/
//!   `noise`, vector maths, `clamp`, time conversions, `posterizeTime`.
//! * **Determinism**: `wiggle` and `random` are seeded by layer index, property id and time (and
//!   `seedRandom`), so frames evaluate identically on every thread, in any order.
//! * **Limits** (CFA4 differs from the donor here: no wall-clock): an evaluation, the expressions
//!   it reads included, runs at most [`MAX_RUNS`] scripts and [`MAX_POLLS`] engine polls of a
//!   fixed number of VM instructions each; loops stop at [`LOOP_LIMIT`] iterations per call;
//!   expressions reading each other nest up to [`MAX_DEPTH`] levels; text nested deeper than the
//!   parser's stack allows is refused ([`rewrite::MAX_NESTING`]). A hostile expression fails with
//!   an error; it never panics, hangs or touches the file system, the network or a clock.
//!
//! Reading other expressions' values: [`Host::value_at`] may call [`evaluate`] again (for a
//! property that has its own expression); cycles and fan-out are cut by the limits above.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod noise;
pub mod rewrite;
mod runtime;

use runtime::{Begin, FRAMES, Frame, Owner, POLLS, RUNS, Req, Resp, Run};

pub use runtime::{LOOP_LIMIT, MAX_POLLS};

/// Maximum nesting of expressions that read other expressions (cycles stop here).
pub const MAX_DEPTH: usize = 16;
/// Maximum script re-runs while answering host requests.
const MAX_PASSES: usize = 2000;
/// Maximum script runs for one outermost evaluation, the expressions it reads included: reads
/// that fan out grow exponentially with [`MAX_DEPTH`] and would hang the frame.
pub const MAX_RUNS: usize = 20_000;

/// The value of a property.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Vec(Vec<f64>),
    Text(String),
}

/// A property's keyframes, in time order (seconds, layer time).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyInfo {
    pub times: Vec<f64>,
    pub values: Vec<Value>,
}

/// What an expression may read. Paths name properties in the host's own scheme.
pub trait Host {
    /// The keyframes of the property at `path`; `None` when there is no such property.
    fn key_info(&self, path: &str) -> Option<KeyInfo>;
    /// The value of the property at `path` at `time` seconds (its own expression included, if
    /// it has one); `None` when there is no such property.
    fn value_at(&self, path: &str, time: f64) -> Option<Value>;
}

/// A host with no properties: expressions can only use their own `value`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHost;

impl Host for NoHost {
    fn key_info(&self, _: &str) -> Option<KeyInfo> {
        None
    }
    fn value_at(&self, _: &str, _: f64) -> Option<Value> {
        None
    }
}

/// One expression evaluation.
pub struct Evaluation<'a> {
    /// The expression text.
    pub text: &'a str,
    /// The time to evaluate at.
    pub time: cfa4_time::Tick,
    /// The property's keyframed value at `time` (what `value` reads).
    pub value: Value,
    /// The property's path (what `thisProperty` reads).
    pub property: &'a str,
    /// The property's stable id (seeds `wiggle` and `random`).
    pub property_uid: u64,
    /// The layer's index (seeds `wiggle` and `random`; what `index` reads).
    pub layer_index: u64,
    /// One frame's duration in seconds (what `frameDuration` reads).
    pub frame_duration: f64,
    /// Where other properties are read from.
    pub host: &'a dyn Host,
}

/// Why an expression failed: a message in AE's style (`Error at line N: …`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError(pub String);

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExprError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError(msg.into()))
}

/// Start the work budget of an outermost evaluation.
fn budget_start() {
    RUNS.with(|r| r.set(0));
    POLLS.with(|p| p.set(0));
}

/// An error once the outermost evaluation has used up its runs or its polls.
fn budget_left() -> Result<(), ExprError> {
    if RUNS.with(|r| r.get()) >= MAX_RUNS {
        return err("Error: expression reads too many other expressions' values (circular or fanning-out references?)");
    }
    if runtime::out_of_budget() {
        return err(runtime::OUT_OF_BUDGET);
    }
    Ok(())
}

/// Pops the evaluation frame on every exit path.
struct FrameGuard;
impl Drop for FrameGuard {
    fn drop(&mut self) {
        FRAMES.with(|f| f.borrow_mut().pop());
    }
}

fn resolve(host: &dyn Host, req: &Req) -> Resp {
    match req {
        Req::Info { path } => match host.key_info(path) {
            Some(i) => Resp::Info(i),
            None => Resp::Missing(path.clone()),
        },
        Req::Value { path, t_bits } => match host.value_at(path, f64::from_bits(*t_bits)) {
            Some(v) => Resp::Value(v),
            None => Resp::Missing(path.clone()),
        },
    }
}

/// Evaluate an expression and return its value.
pub fn evaluate(e: &Evaluation<'_>) -> Result<Value, ExprError> {
    let out = evaluate_out(e)?;
    runtime::to_value(&out).map_err(ExprError)
}

/// Run an expression and return the raw script result.
fn evaluate_out(e: &Evaluation<'_>) -> Result<runtime::Out, ExprError> {
    let t = e.time.seconds();
    let owner = Owner { property: e.property.to_string(), uid: e.property_uid, layer_index: e.layer_index, t_bits: t.to_bits(), text: e.text.to_string() };
    let (depth, cycle) = FRAMES.with(|f| {
        let f = f.borrow();
        (f.len(), f.iter().any(|fr| fr.owner.as_ref() == Some(&owner)))
    });
    if depth == 0 {
        budget_start();
    }
    if cycle {
        // Needs its own result: fails at once instead of nesting to `MAX_DEPTH` once per read.
        return err("Error: circular expression reference (the expression needs its own value)");
    }
    if depth >= MAX_DEPTH {
        return err("Error: expressions reference each other too deeply (circular reference?)");
    }
    budget_left()?;
    let begin = Begin { time: t, value: &e.value, path: e.property, uid: e.property_uid, index: e.layer_index as f64, frame_duration: e.frame_duration };
    FRAMES.with(|f| f.borrow_mut().push(Frame { owner: Some(owner), ..Frame::default() }));
    let _guard = FrameGuard;
    for _ in 0..MAX_PASSES {
        budget_left()?;
        RUNS.with(|r| r.set(r.get().saturating_add(1)));
        match runtime::run(e.text, &begin) {
            Run::Done(r) => return r.map_err(ExprError),
            Run::Pending => {
                let misses = FRAMES.with(|f| f.borrow_mut().get_mut(depth).map(|fr| std::mem::take(&mut fr.misses)).unwrap_or_default());
                for m in misses {
                    // May evaluate other expressions (nested frames above ours).
                    let r = resolve(e.host, &m);
                    FRAMES.with(|f| f.borrow_mut().get_mut(depth).map(|fr| fr.memo.insert(m, r)));
                }
            }
        }
    }
    err("Error: expression needs too many property reads")
}

/// Syntax-check an expression (after the array-maths rewrite) without evaluating it.
pub fn check_syntax(text: &str) -> Result<(), String> {
    rewrite::check_nesting(text)?;
    runtime::check(text)
}

#[cfg(test)]
mod tests;
