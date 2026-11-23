//! The per-thread boa runtime: one `Context` with the prelude loaded, a compiled-script cache,
//! the native functions, and the request/miss bookkeeping (see [`crate::Host`]).
//!
//! Ported from the EC-donor (#11) expression crate (MIT OR Apache-2.0) and changed for CFA4:
//! the work budget is a count of engine polls (a fixed number of VM instructions each), not wall
//! clock time, so whether an expression finishes never depends on the machine; the host
//! interface is the small [`crate::Host`] trait instead of the donor's project model.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Script, Source, js_string};

use crate::noise;
use crate::{KeyInfo, Value};

const PRELUDE: &str = include_str!("prelude.js");
/// Message of the error thrown when a request isn't answered yet (never shown to users).
const PENDING: &str = "\u{1}cfa4: pending host request";
const MAX_SCRIPTS: usize = 4096;
/// Loop iterations one script run may make (per function call).
pub const LOOP_LIMIT: u64 = 1_000_000;
/// VM instructions per engine poll (the unit of the work budget).
const POLL_INSTRUCTIONS: u32 = 20_000;
/// Engine polls one outermost evaluation (all its runs and the expressions it reads) may use.
pub const MAX_POLLS: usize = 10_000;
/// The error of an evaluation that ran out of its work budget.
pub const OUT_OF_BUDGET: &str = "Error: expression takes too long to evaluate";
/// The part of boa's stack-size limit error the runtime recovers from (see [`run`]).
const STACK_LIMIT: &str = "maximum stack size";

/// One host request.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Req {
    Info { path: String },
    Value { path: String, t_bits: u64 },
}

/// The answer to a [`Req`].
#[derive(Clone, Debug)]
pub enum Resp {
    Info(KeyInfo),
    Value(Value),
    /// The host does not know the property.
    Missing(String),
}

/// Memoized host answers and pending misses of one expression evaluation.
#[derive(Default)]
pub struct Frame {
    pub memo: HashMap<Req, Resp>,
    pub misses: Vec<Req>,
    /// What this frame evaluates: an evaluation that needs itself is a cycle.
    pub owner: Option<Owner>,
}

/// One expression evaluation: the property, the time and the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub property: String,
    pub uid: u64,
    pub layer_index: u64,
    pub t_bits: u64,
    pub text: String,
}

thread_local! {
    /// Never dropped: boa's own thread-local heap may be torn down first at thread exit, and
    /// dropping GC handles after that aborts the process. The OS reclaims it with the thread.
    static RUNTIME: RefCell<Option<ManuallyDrop<Result<Runtime, String>>>> = const { RefCell::new(None) };
    /// Evaluation stack (nested when one expression reads another's value).
    pub static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// Script runs since the outermost evaluation on this thread began (nested ones included).
    pub static RUNS: Cell<usize> = const { Cell::new(0) };
    /// Engine polls since the outermost evaluation on this thread began.
    pub static POLLS: Cell<usize> = const { Cell::new(0) };
}

struct Runtime {
    ctx: Context,
    scripts: HashMap<String, Result<Script, String>>,
    begin: JsObject,
    finish: JsObject,
}

/// Per-evaluation inputs for `__begin`.
pub struct Begin<'a> {
    pub time: f64,
    pub value: &'a Value,
    pub path: &'a str,
    pub uid: u64,
    pub index: f64,
    pub frame_duration: f64,
}

/// Outcome of one script run.
pub enum Run {
    Done(Result<Out, String>),
    /// The script needs host answers first.
    Pending,
}

/// A script result, detached from the JS heap.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Out>),
    Object(String),
}

fn arg(args: &[JsValue], i: usize) -> JsValue {
    args.get(i).cloned().unwrap_or_default()
}
fn num(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<f64> {
    arg(args, i).to_number(ctx)
}
fn string(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    Ok(arg(args, i).to_string(ctx)?.to_std_string_escaped())
}

fn value_to_js(v: &Value, ctx: &mut Context) -> JsValue {
    match v {
        Value::Num(n) => JsValue::from(*n),
        Value::Vec(items) => JsArray::from_iter(items.iter().map(|x| JsValue::from(*x)), ctx).into(),
        Value::Text(s) => JsValue::from(JsString::from(s.as_str())),
    }
}

fn resp_to_js(r: &Resp, ctx: &mut Context) -> JsResult<JsValue> {
    Ok(match r {
        Resp::Value(v) => value_to_js(v, ctx),
        Resp::Info(info) => {
            let times: JsValue = JsArray::from_iter(info.times.iter().map(|x| JsValue::from(*x)), ctx).into();
            let values: Vec<JsValue> = info.values.iter().map(|v| value_to_js(v, ctx)).collect();
            let values: JsValue = JsArray::from_iter(values, ctx).into();
            JsArray::from_iter([times, values], ctx).into()
        }
        Resp::Missing(path) => return Err(JsNativeError::error().with_message(format!("property \"{path}\" does not exist")).into()),
    })
}

/// `__h(op, path, t)`: answer a host request from the memo, or record a miss and abort.
fn native_host(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let op = num(args, 0, ctx)? as i64;
    let path = string(args, 1, ctx)?;
    let req = match op {
        0 => Req::Info { path },
        1 => Req::Value { path, t_bits: num(args, 2, ctx)?.to_bits() },
        _ => return Err(JsNativeError::typ().with_message("unknown host request").into()),
    };
    let found = FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        let frame = f.last_mut()?;
        match frame.memo.get(&req) {
            Some(r) => Some(r.clone()),
            None => {
                frame.misses.push(req);
                None
            }
        }
    });
    match found {
        Some(r) => resp_to_js(&r, ctx),
        None => Err(JsNativeError::error().with_message(PENDING).into()),
    }
}

/// `__wig(layerIndex, uid, path, dims, freq, amp, octaves, ampMult, t)`.
fn native_wiggle(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let path = string(args, 2, ctx)?;
    let mut a = [0.0; 9];
    for (i, x) in a.iter_mut().enumerate() {
        if i != 2 {
            *x = num(args, i, ctx)?;
        }
    }
    let path_hash = path.bytes().fold(0u64, |h, b| noise::mix(&[h, u64::from(b)]));
    let seed = noise::mix(&[a[0].to_bits(), a[1].to_bits(), path_hash]);
    let dims = (a[3].max(1.0) as usize).min(16);
    let off = noise::wiggle(seed, dims, a[4], a[5], a[6], a[7], a[8]);
    let vals: Vec<JsValue> = off.into_iter().map(JsValue::from).collect();
    Ok(JsArray::from_iter(vals, ctx).into())
}

/// `__noise(x, y, z)`.
fn native_noise(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(noise::noise3(num(args, 0, ctx)?, num(args, 1, ctx)?, num(args, 2, ctx)?)))
}

/// `__rnd(layerIndex, uid, userSeed, timeless, time, counter)`.
fn native_random(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let timeless = arg(args, 3).to_boolean();
    let t = if timeless { 0.0 } else { num(args, 4, ctx)? };
    let parts = [num(args, 0, ctx)?.to_bits(), num(args, 1, ctx)?.to_bits(), num(args, 2, ctx)?.to_bits(), t.to_bits(), num(args, 5, ctx)?.to_bits()];
    Ok(JsValue::from(noise::unit(noise::mix(&parts))))
}

/// `__nest(src)`: throw when source handed to `eval` / `Function` at run time nests deeper than
/// the parser's stack allows (the expression text itself is checked before it is compiled).
fn native_nesting(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let src = string(args, 0, ctx)?;
    crate::rewrite::check_plain_nesting(&src).map_err(|e| JsNativeError::range().with_message(e.trim_start_matches("Error: ").to_string()))?;
    Ok(JsValue::undefined())
}

impl Runtime {
    fn new() -> Result<Runtime, String> {
        let mut ctx = Context::default();
        let limits = ctx.runtime_limits_mut();
        limits.set_loop_iteration_limit(LOOP_LIMIT);
        limits.set_recursion_limit(400);
        let natives: [(JsString, usize, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>); 5] = [
            (js_string!("__h"), 3, native_host),
            (js_string!("__wig"), 9, native_wiggle),
            (js_string!("__noise"), 3, native_noise),
            (js_string!("__rnd"), 6, native_random),
            (js_string!("__nest"), 1, native_nesting),
        ];
        for (name, len, f) in natives {
            ctx.register_global_callable(name, len, NativeFunction::from_fn_ptr(f)).map_err(|e| e.to_string())?;
        }
        ctx.eval(Source::from_bytes(PRELUDE)).map_err(|e| format!("expression prelude: {e}"))?;
        let global = ctx.global_object();
        let func = |name: JsString, ctx: &mut Context| -> Result<JsObject, String> {
            global.get(name, ctx).ok().and_then(|v| v.as_object()).ok_or_else(|| "expression prelude: missing function".to_string())
        };
        let begin = func(js_string!("__begin"), &mut ctx)?;
        let finish = func(js_string!("__finish"), &mut ctx)?;
        Ok(Runtime { ctx, scripts: HashMap::new(), begin, finish })
    }

    fn script(&mut self, text: &str) -> Result<Script, String> {
        if let Some(s) = self.scripts.get(text) {
            return s.clone();
        }
        if self.scripts.len() >= MAX_SCRIPTS {
            self.scripts.clear();
        }
        // A block keeps `let`/`const` local to one evaluation; `{` shares line 1 with the code so
        // error line numbers match the user's text. Errors are rethrown from the script's own
        // frame: boa leaves the stack of the inner frames behind when an exception leaves the
        // script from a nested call (every host request does), and the thread's runtime would
        // fail every expression with "reached the maximum stack size" after a few hundred.
        let src = format!("try{{{}\n}}catch(__e){{throw __e}}", crate::rewrite::rewrite(text));
        let s = Script::parse(Source::from_bytes(src.as_bytes()), None, &mut self.ctx).map_err(|e| format_error(&e.to_string(), line_count(text)));
        self.scripts.insert(text.to_string(), s.clone());
        s
    }

    fn run(&mut self, text: &str, b: &Begin<'_>) -> Run {
        let script = match self.script(text) {
            Ok(s) => s,
            Err(e) => return Run::Done(Err(e)),
        };
        let ctx = &mut self.ctx;
        let args = [
            JsValue::from(b.time),
            value_to_js(b.value, ctx),
            JsValue::from(JsString::from(b.path)),
            JsValue::from(b.uid as f64),
            JsValue::from(b.index),
            JsValue::from(b.frame_duration),
        ];
        if let Err(e) = self.begin.call(&JsValue::undefined(), &args, ctx) {
            return Run::Done(Err(format_error(&e.to_string(), line_count(text))));
        }
        let result = match evaluate_within_budget(&script, ctx) {
            Some(r) => r.and_then(|v| self.finish.call(&JsValue::undefined(), &[v], ctx)),
            // Abandoned mid-run: the context is unusable now (see `run`).
            None => return Run::Done(Err(OUT_OF_BUDGET.into())),
        };
        let pending = FRAMES.with(|f| f.borrow().last().is_some_and(|fr| !fr.misses.is_empty()));
        if pending {
            return Run::Pending;
        }
        Run::Done(match result {
            Ok(v) => detach(&v, ctx, 0),
            Err(e) => Err(format_error(&e.to_string(), line_count(text))),
        })
    }
}

/// Run `script`, giving up (`None`) once the outermost evaluation has used [`MAX_POLLS`] engine
/// polls. boa's budgeted evaluation yields every [`POLL_INSTRUCTIONS`] instructions of the script
/// and the functions it calls (not inside callbacks of native functions such as
/// `Array.prototype.map`, which stay bounded by the loop limit per call). The budget counts VM
/// work, so the same expression is accepted or refused on every machine.
fn evaluate_within_budget(script: &Script, ctx: &mut Context) -> Option<JsResult<JsValue>> {
    let mut run = std::pin::pin!(script.evaluate_async_with_budget(ctx, POLL_INSTRUCTIONS));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(r) = run.as_mut().poll(&mut cx) {
            return Some(r);
        }
        let used = POLLS.with(|p| {
            p.set(p.get().saturating_add(1));
            p.get()
        });
        if used >= MAX_POLLS {
            return None;
        }
    }
}

/// Whether the outermost evaluation on this thread has used up its polls.
pub fn out_of_budget() -> bool {
    POLLS.with(|p| p.get()) >= MAX_POLLS
}

/// Levels of nested arrays a script result keeps.
const MAX_DETACH: usize = 4;

/// Copy a JS value out of the heap.
fn detach(v: &JsValue, ctx: &mut Context, depth: usize) -> Result<Out, String> {
    if v.is_undefined() {
        return Ok(Out::Undefined);
    }
    if v.is_null() {
        return Ok(Out::Null);
    }
    if let Some(b) = v.as_boolean() {
        return Ok(Out::Bool(b));
    }
    if let Some(n) = v.as_number() {
        return Ok(Out::Num(n));
    }
    if let Some(s) = v.as_string() {
        return Ok(Out::Str(s.to_std_string_escaped()));
    }
    let err = |e: boa_engine::JsError| format_error(&e.to_string(), usize::MAX);
    if let Some(o) = v.as_object()
        && o.is_array()
        && depth < MAX_DETACH
    {
        let arr = JsArray::from_object(o).map_err(err)?;
        let n = arr.length(ctx).map_err(err)?.min(1 << 16);
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            out.push(detach(&arr.get(i, ctx).map_err(err)?, ctx, depth + 1)?);
        }
        return Ok(Out::Arr(out));
    }
    // Below the levels detached above, an object's text could be its whole nested content
    // (`String([[[…]]])`), converted by natives that recurse once per level.
    if depth >= MAX_DETACH && v.is_object() {
        return Ok(Out::Object("[object]".into()));
    }
    Ok(Out::Object(v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default()))
}

/// Make boa's messages read like AE's: `Error at line N: message`.
fn format_error(msg: &str, max_line: usize) -> String {
    let msg = msg.trim();
    let first = msg.lines().next().unwrap_or("").trim();
    // The wrapper's closing brace sits one line below the user's text.
    let line = find_line(msg).map(|l| l.min(max_line.max(1)));
    // Drop the position boa appends to the first line (` at line 2, col 5`, ` (unknown at :2:11)`).
    let mut text = first.split(" at line ").next().unwrap_or(first).trim();
    if let Some(i) = text.rfind(" (")
        && text.ends_with(')')
        && text[i..].contains(" at ")
    {
        text = text[..i].trim();
    }
    // `throw new Error("x")` reads better as just the message.
    let text = text.strip_prefix("Error: ").unwrap_or(text);
    match line {
        Some(l) => format!("Error at line {l}: {text}"),
        None => format!("Error: {text}"),
    }
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// Line number from `at line N` (parser errors), else the user script's (`<main>`) backtrace
/// entry, else the first `:line:col` position.
fn find_line(msg: &str) -> Option<usize> {
    if let Some(i) = msg.find("at line ") {
        return msg[i + 8..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok();
    }
    if let Some(l) = msg.lines().find(|l| l.trim_start().starts_with("at <main>")).and_then(position_line) {
        return Some(l);
    }
    position_line(msg)
}

/// First `:line:col` in `msg`.
fn position_line(msg: &str) -> Option<usize> {
    let b = msg.as_bytes();
    for i in 0..b.len() {
        if b[i] != b':' {
            continue;
        }
        let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
        let n = digits(i + 1);
        if n > 0 && b.get(i + 1 + n) == Some(&b':') && digits(i + 2 + n) > 0 {
            return msg[i + 1..i + 1 + n].parse().ok();
        }
    }
    None
}

/// Run `text` once on this thread's runtime.
///
/// boa leaves values on its VM stack when an exception unwinds through call frames, and every
/// pending host request does that (the request throws out of the object-model functions), so a
/// long-lived context eventually fails every run with "maximum stack size". When a run hits that
/// limit the context is replaced with a fresh one and the run retried once: each run starts from
/// `__begin` with memoized answers, so nothing carries over. A script that really overflows the
/// stack fails again on the fresh context and reports the error.
pub fn run(text: &str, b: &Begin<'_>) -> Run {
    RUNTIME.with(|r| {
        let mut r = r.borrow_mut();
        for fresh in [false, true] {
            let out = match &mut **r.get_or_insert_with(|| ManuallyDrop::new(Runtime::new())) {
                Ok(rt) => rt.run(text, b),
                Err(e) => return Run::Done(Err(e.clone())),
            };
            match out {
                Run::Done(Err(e)) if e == OUT_OF_BUDGET => {
                    // Abandoned mid-run: start the next evaluation on a fresh context.
                    if let Some(old) = r.take() {
                        drop(ManuallyDrop::into_inner(old));
                    }
                    return Run::Done(Err(e));
                }
                Run::Done(Err(e)) if !fresh && e.contains(STACK_LIMIT) => {
                    // Dropped here, mid-thread, where boa's heap is alive (unlike at thread exit).
                    if let Some(old) = r.take() {
                        drop(ManuallyDrop::into_inner(old));
                    }
                }
                out => return out,
            }
        }
        Run::Done(Err(format!("Error: {STACK_LIMIT}")))
    })
}

/// Compile `text` (cached) without running it.
pub fn check(text: &str) -> Result<(), String> {
    RUNTIME.with(|r| {
        let mut r = r.borrow_mut();
        match &mut **r.get_or_insert_with(|| ManuallyDrop::new(Runtime::new())) {
            Ok(rt) => rt.script(text).map(|_| ()),
            Err(e) => Err(e.clone()),
        }
    })
}

/// Convert a script result to a property value.
pub fn to_value(out: &Out) -> Result<Value, String> {
    let finite = |x: f64| if x.is_finite() { Ok(x) } else { Err("Error: expression result is not a finite number".to_string()) };
    match out {
        Out::Num(x) => finite(*x).map(Value::Num),
        Out::Bool(b) => Ok(Value::Num(if *b { 1.0 } else { 0.0 })),
        Out::Str(s) => Ok(Value::Text(s.clone())),
        Out::Arr(items) => {
            let mut v = Vec::with_capacity(items.len());
            for i in items {
                match i {
                    Out::Num(x) => v.push(finite(*x)?),
                    _ => return Err("Error: expression result must be a number, an array of numbers or text".into()),
                }
            }
            Ok(Value::Vec(v))
        }
        Out::Undefined | Out::Null | Out::Object(_) => Err("Error: expression result must be a number, an array of numbers or text".into()),
    }
}
