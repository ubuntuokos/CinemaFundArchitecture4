//! The fuel-limited virtual machine and the built-in functions.

use crate::compile::{Func, Op, Program};
use crate::error::{Diagnostic, Result, Span, diag};
use crate::types::{Env, RunStats, Value};

/// Hard ceiling for the per-run fuel a host may ask for.
pub const MAX_FUEL: u32 = 1_000_000;
/// Default fuel for one element (voxel or point).
pub const DEFAULT_FUEL: u32 = 10_000;

/// Reusable machine state (no allocation per element after `Program::new_vm`).
#[derive(Debug, Clone)]
pub struct Vm {
    stack: Vec<Value>,
    slots: Vec<Value>,
}

impl Program {
    pub fn new_vm(&self) -> Vm {
        Vm { stack: Vec::with_capacity(self.max_stack), slots: vec![Value::Int(0); self.nslots] }
    }

    /// Run the expression once. `inputs` and `outputs` are indexed like the schema; reads always see
    /// `inputs` (the value before the run), writes go to `outputs`. Missing `params` take their defaults.
    pub fn run(&self, vm: &mut Vm, fuel: u32, env: &Env, params: &[Value], inputs: &[Value], outputs: &mut [Value]) -> Result<RunStats> {
        if inputs.len() != self.n_attrs || outputs.len() != self.n_attrs {
            return Err(rt("R-INTERNAL", (0, 0), "a bemenet/kimenet mérete nem egyezik a sémával", "input/output length does not match the schema"));
        }
        let fuel = fuel.min(MAX_FUEL);
        vm.stack.clear();
        vm.slots.clear();
        vm.slots.resize(self.nslots, Value::Int(0));
        for (i, p) in self.params.iter().enumerate() {
            let v = params.get(i).copied().unwrap_or(p.default);
            if v.ty() != p.ty {
                return Err(rt("R-PARAM", (0, 0), format!("a(z) {} paraméter típusa hibás", p.name), format!("parameter {} has the wrong type", p.name)));
            }
            vm.slots[i] = v;
        }
        let mut stats = RunStats::default();
        let mut pc = 0usize;
        let mut left = fuel;
        while pc < self.code.len() {
            if left == 0 {
                return Err(rt("R-FUEL", self.spans[pc], "az utasításkorlát kimerült", "instruction budget exhausted"));
            }
            left -= 1;
            let op = self.code[pc];
            let at = self.spans[pc];
            pc += 1;
            macro_rules! pop {
                () => {
                    match vm.stack.pop() {
                        Some(v) => v,
                        None => return Err(internal(at)),
                    }
                };
            }
            macro_rules! push {
                ($v:expr) => {{
                    let v = $v;
                    if vm.stack.len() >= self.max_stack {
                        return Err(internal(at));
                    }
                    vm.stack.push(v);
                }};
            }
            match op {
                Op::PushF(f) => push!(Value::Float(f)),
                Op::PushI(i) => push!(Value::Int(i)),
                Op::PushB(b) => push!(Value::Bool(b)),
                Op::Load(s) => push!(*vm.slots.get(usize::from(s)).ok_or_else(|| internal(at))?),
                Op::Store(s) => {
                    let v = pop!();
                    *vm.slots.get_mut(usize::from(s)).ok_or_else(|| internal(at))? = v;
                }
                Op::LoadAttr(i) => push!(*inputs.get(usize::from(i)).ok_or_else(|| internal(at))?),
                Op::StoreAttr(i) => {
                    let v = canonical(pop!(), &mut stats.wrote_nan);
                    *outputs.get_mut(usize::from(i)).ok_or_else(|| internal(at))? = v;
                }
                Op::LoadP => push!(Value::Vec3(env.p)),
                Op::LoadIjk => push!(Value::Vec3([env.ijk[0] as f32, env.ijk[1] as f32, env.ijk[2] as f32])),
                Op::LoadVoxelSize => push!(Value::Float(env.voxel_size)),
                Op::LoadId => push!(Value::Int(env.id)),
                Op::Neg => {
                    let v = pop!();
                    push!(match v {
                        Value::Float(f) => Value::Float(-f),
                        Value::Int(i) => Value::Int(i.checked_neg().ok_or_else(|| int_err(at))?),
                        Value::Vec3(a) => Value::Vec3([-a[0], -a[1], -a[2]]),
                        Value::Bool(_) => return Err(internal(at)),
                    });
                }
                Op::Not => match pop!() {
                    Value::Bool(b) => push!(Value::Bool(!b)),
                    _ => return Err(internal(at)),
                },
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne => {
                    let b = pop!();
                    let a = pop!();
                    push!(binop(op, a, b, at)?);
                }
                Op::Jump(t) => pc = t as usize,
                Op::JumpIfFalse(t) => match pop!() {
                    Value::Bool(true) => {}
                    Value::Bool(false) => pc = t as usize,
                    _ => return Err(internal(at)),
                },
                Op::Call(f, n) => {
                    let n = usize::from(n);
                    let mut args = [Value::Int(0); 8];
                    if n > args.len() {
                        return Err(internal(at));
                    }
                    for k in (0..n).rev() {
                        args[k] = pop!();
                    }
                    push!(call(f, &args[..n]).map_err(|c| if c == "int" { int_err(at) } else { internal(at) })?);
                }
                Op::Vec3Ctor(n) => {
                    if n == 1 {
                        match pop!() {
                            Value::Float(f) => push!(Value::Vec3([f; 3])),
                            _ => return Err(internal(at)),
                        }
                    } else {
                        let (c, b, a) = (pop!(), pop!(), pop!());
                        match (a, b, c) {
                            (Value::Float(x), Value::Float(y), Value::Float(z)) => push!(Value::Vec3([x, y, z])),
                            _ => return Err(internal(at)),
                        }
                    }
                }
                Op::Member(i) => match pop!() {
                    Value::Vec3(v) => push!(Value::Float(v[usize::from(i).min(2)])),
                    _ => return Err(internal(at)),
                },
                Op::ToFloat => match pop!() {
                    Value::Float(f) => push!(Value::Float(f)),
                    Value::Int(i) => push!(Value::Float(i as f32)),
                    _ => return Err(internal(at)),
                },
                Op::ToInt => match pop!() {
                    Value::Float(f) => push!(Value::Int(f as i32)),
                    Value::Int(i) => push!(Value::Int(i)),
                    _ => return Err(internal(at)),
                },
            }
        }
        stats.fuel_used = fuel - left;
        Ok(stats)
    }
}

fn rt(code: &'static str, at: (u32, u32), hu: impl Into<String>, en: impl Into<String>) -> Diagnostic {
    diag(code, Span { line: at.0, col: at.1, len: 0 }, hu, en)
}

fn internal(at: (u32, u32)) -> Diagnostic {
    rt("R-INTERNAL", at, "belső hiba (érvénytelen bájtkód)", "internal error (invalid bytecode)")
}

fn int_err(at: (u32, u32)) -> Diagnostic {
    rt("R-INT", at, "egész túlcsordulás vagy osztás nullával", "integer overflow or division by zero")
}

fn canonical(v: Value, wrote_nan: &mut bool) -> Value {
    let fix = |f: f32, flag: &mut bool| {
        if f.is_nan() {
            *flag = true;
            f32::NAN
        } else {
            f
        }
    };
    match v {
        Value::Float(f) => Value::Float(fix(f, wrote_nan)),
        Value::Vec3(a) => Value::Vec3([fix(a[0], wrote_nan), fix(a[1], wrote_nan), fix(a[2], wrote_nan)]),
        other => other,
    }
}

fn binop(op: Op, a: Value, b: Value, at: (u32, u32)) -> Result<Value> {
    use Value::{Bool, Float, Int, Vec3};
    let v = match (op, a, b) {
        (Op::Add, Float(x), Float(y)) => Float(x + y),
        (Op::Sub, Float(x), Float(y)) => Float(x - y),
        (Op::Mul, Float(x), Float(y)) => Float(x * y),
        (Op::Div, Float(x), Float(y)) => Float(x / y),
        (Op::Rem, Float(x), Float(y)) => Float(x % y),
        (Op::Add, Int(x), Int(y)) => Int(x.checked_add(y).ok_or_else(|| int_err(at))?),
        (Op::Sub, Int(x), Int(y)) => Int(x.checked_sub(y).ok_or_else(|| int_err(at))?),
        (Op::Mul, Int(x), Int(y)) => Int(x.checked_mul(y).ok_or_else(|| int_err(at))?),
        (Op::Div, Int(x), Int(y)) => Int(x.checked_div(y).ok_or_else(|| int_err(at))?),
        (Op::Rem, Int(x), Int(y)) => Int(x.checked_rem(y).ok_or_else(|| int_err(at))?),
        (Op::Add, Vec3(x), Vec3(y)) => Vec3([x[0] + y[0], x[1] + y[1], x[2] + y[2]]),
        (Op::Sub, Vec3(x), Vec3(y)) => Vec3([x[0] - y[0], x[1] - y[1], x[2] - y[2]]),
        (Op::Mul, Vec3(x), Vec3(y)) => Vec3([x[0] * y[0], x[1] * y[1], x[2] * y[2]]),
        (Op::Mul, Vec3(x), Float(y)) | (Op::Mul, Float(y), Vec3(x)) => Vec3([x[0] * y, x[1] * y, x[2] * y]),
        (Op::Div, Vec3(x), Float(y)) => Vec3([x[0] / y, x[1] / y, x[2] / y]),
        (Op::Lt, Float(x), Float(y)) => Bool(x < y),
        (Op::Le, Float(x), Float(y)) => Bool(x <= y),
        (Op::Gt, Float(x), Float(y)) => Bool(x > y),
        (Op::Ge, Float(x), Float(y)) => Bool(x >= y),
        (Op::Lt, Int(x), Int(y)) => Bool(x < y),
        (Op::Le, Int(x), Int(y)) => Bool(x <= y),
        (Op::Gt, Int(x), Int(y)) => Bool(x > y),
        (Op::Ge, Int(x), Int(y)) => Bool(x >= y),
        (Op::Eq, x, y) if x.ty() == y.ty() => Bool(x == y),
        (Op::Ne, x, y) if x.ty() == y.ty() => Bool(x != y),
        _ => return Err(internal(at)),
    };
    Ok(v)
}

fn map1(v: Value, f: impl Fn(f32) -> f32) -> Option<Value> {
    match v {
        Value::Float(x) => Some(Value::Float(f(x))),
        Value::Vec3(a) => Some(Value::Vec3([f(a[0]), f(a[1]), f(a[2])])),
        _ => None,
    }
}

fn map2(a: Value, b: Value, f: impl Fn(f32, f32) -> f32) -> Option<Value> {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => Some(Value::Float(f(x, y))),
        (Value::Vec3(x), Value::Vec3(y)) => Some(Value::Vec3([f(x[0], y[0]), f(x[1], y[1]), f(x[2], y[2])])),
        _ => None,
    }
}

fn clamp_f(x: f32, lo: f32, hi: f32) -> f32 {
    x.max(lo).min(hi)
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    if e0 == e1 {
        return if x >= e0 { 1.0 } else { 0.0 };
    }
    let t = clamp_f((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fit(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if a == b {
        return c;
    }
    let t = clamp_f((x - a) / (b - a), 0.0, 1.0);
    c + t * (d - c)
}

fn vlen(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// `Err("int")` for an integer error, `Err("internal")` for a type mismatch (cannot happen after checking).
fn call(f: Func, a: &[Value]) -> std::result::Result<Value, &'static str> {
    use Value::{Float, Int, Vec3};
    let r = match (f, a) {
        (Func::Abs, [Int(i)]) => return i.checked_abs().map(Int).ok_or("int"),
        (Func::Abs, [v]) => map1(*v, f32::abs),
        (Func::Floor, [v]) => map1(*v, f32::floor),
        (Func::Ceil, [v]) => map1(*v, f32::ceil),
        (Func::Round, [v]) => map1(*v, f32::round),
        (Func::Sqrt, [v]) => map1(*v, f32::sqrt),
        (Func::Exp, [v]) => map1(*v, f32::exp),
        (Func::Log, [v]) => map1(*v, |x| if x <= 0.0 { f32::NAN } else { x.ln() }),
        (Func::Sin, [v]) => map1(*v, f32::sin),
        (Func::Cos, [v]) => map1(*v, f32::cos),
        (Func::Tan, [v]) => map1(*v, f32::tan),
        (Func::Min, [Int(x), Int(y)]) => Some(Int(*x.min(y))),
        (Func::Max, [Int(x), Int(y)]) => Some(Int(*x.max(y))),
        (Func::Min, [x, y]) => map2(*x, *y, f32::min),
        (Func::Max, [x, y]) => map2(*x, *y, f32::max),
        (Func::Pow, [x, y]) => map2(*x, *y, f32::powf),
        (Func::Atan2, [x, y]) => map2(*x, *y, f32::atan2),
        (Func::Clamp, [Int(x), Int(lo), Int(hi)]) => Some(Int(if x < lo {
            *lo
        } else if x > hi {
            *hi
        } else {
            *x
        })),
        (Func::Clamp, [Float(x), Float(lo), Float(hi)]) => Some(Float(clamp_f(*x, *lo, *hi))),
        (Func::Clamp, [Vec3(x), Vec3(lo), Vec3(hi)]) => Some(Vec3([clamp_f(x[0], lo[0], hi[0]), clamp_f(x[1], lo[1], hi[1]), clamp_f(x[2], lo[2], hi[2])])),
        (Func::Clamp, [Vec3(x), Float(lo), Float(hi)]) => Some(Vec3([clamp_f(x[0], *lo, *hi), clamp_f(x[1], *lo, *hi), clamp_f(x[2], *lo, *hi)])),
        (Func::Lerp, [Float(x), Float(y), Float(t)]) => Some(Float(x + (y - x) * t)),
        (Func::Lerp, [Vec3(x), Vec3(y), Float(t)]) => Some(Vec3([x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t])),
        (Func::Lerp, [Vec3(x), Vec3(y), Vec3(t)]) => Some(Vec3([x[0] + (y[0] - x[0]) * t[0], x[1] + (y[1] - x[1]) * t[1], x[2] + (y[2] - x[2]) * t[2]])),
        (Func::Smoothstep, [Float(e0), Float(e1), x]) => map1(*x, |x| smooth(*e0, *e1, x)),
        (Func::Step, [Float(e), x]) => map1(*x, |x| if x >= *e { 1.0 } else { 0.0 }),
        (Func::Fit, [Float(x), Float(a), Float(b), Float(c), Float(d)]) => Some(Float(fit(*x, *a, *b, *c, *d))),
        (Func::Length, [Vec3(v)]) => Some(Float(vlen(*v))),
        (Func::Dot, [Vec3(x), Vec3(y)]) => Some(Float(x[0] * y[0] + x[1] * y[1] + x[2] * y[2])),
        (Func::Cross, [Vec3(x), Vec3(y)]) => Some(Vec3([x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]])),
        (Func::Normalize, [Vec3(v)]) => {
            let l = vlen(*v);
            Some(Vec3(if l == 0.0 { [0.0; 3] } else { [v[0] / l, v[1] / l, v[2] / l] }))
        }
        (Func::Noise, [Vec3(v)]) => Some(Float(noise3(*v))),
        (Func::Hash, xs) if (1..=4).contains(&xs.len()) => {
            let mut h = 0x811c_9dc5u32;
            for x in xs {
                let Int(i) = x else { return Err("internal") };
                h = hash32(h ^ (*i as u32));
            }
            Some(Int(h as i32))
        }
        (Func::Rand, [Int(seed), Int(id)]) => {
            let h = hash32(hash32(*seed as u32) ^ (*id as u32).wrapping_mul(0x9e37_79b9));
            Some(Float((h >> 8) as f32 / 16_777_216.0))
        }
        _ => None,
    };
    r.ok_or("internal")
}

/// 32-bit integer mixer.
pub(crate) fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn lattice(ix: i32, iy: i32, iz: i32) -> [f32; 3] {
    let h = hash32((ix as u32) ^ hash32((iy as u32) ^ hash32((iz as u32) ^ 0x9e37_79b9)));
    let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 127.5 - 1.0;
    [c(0), c(8), c(16)]
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Deterministic gradient noise in [-1, 1]: hashed lattice gradients, smooth interpolation.
/// A non-finite input gives 0.
pub(crate) fn noise3(p: [f32; 3]) -> f32 {
    if !p.iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    let fl = [p[0].floor(), p[1].floor(), p[2].floor()];
    let f = [p[0] - fl[0], p[1] - fl[1], p[2] - fl[2]];
    let i = [fl[0] as i32, fl[1] as i32, fl[2] as i32];
    let u = [fade(f[0]), fade(f[1]), fade(f[2])];
    let corner = |dx: i32, dy: i32, dz: i32| {
        let g = lattice(i[0].wrapping_add(dx), i[1].wrapping_add(dy), i[2].wrapping_add(dz));
        g[0] * (f[0] - dx as f32) + g[1] * (f[1] - dy as f32) + g[2] * (f[2] - dz as f32)
    };
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(corner(0, 0, 0), corner(1, 0, 0), u[0]);
    let x10 = l(corner(0, 1, 0), corner(1, 1, 0), u[0]);
    let x01 = l(corner(0, 0, 1), corner(1, 0, 1), u[0]);
    let x11 = l(corner(0, 1, 1), corner(1, 1, 1), u[0]);
    let y0 = l(x00, x10, u[1]);
    let y1 = l(x01, x11, u[1]);
    clamp_f(l(y0, y1, u[2]), -1.0, 1.0)
}
