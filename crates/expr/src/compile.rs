//! Type checker and bytecode generator (one pass over the AST).

use crate::ast::*;
use crate::error::{Diagnostic, Result, Span, diag, err};
use crate::types::{Context, ParamDecl, Schema, Ty, Value};

pub(crate) const MAX_CODE: usize = 65_536;
pub(crate) const MAX_SLOTS: usize = 256;
const MAX_ITER_PRODUCT: u64 = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Func {
    Abs,
    Min,
    Max,
    Clamp,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Pow,
    Exp,
    Log,
    Sin,
    Cos,
    Tan,
    Atan2,
    Lerp,
    Smoothstep,
    Step,
    Fit,
    Length,
    Dot,
    Cross,
    Normalize,
    Noise,
    Hash,
    Rand,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    PushF(f32),
    PushI(i32),
    PushB(bool),
    Load(u16),
    Store(u16),
    LoadAttr(u16),
    StoreAttr(u16),
    LoadP,
    LoadIjk,
    LoadVoxelSize,
    LoadId,
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Jump(u32),
    JumpIfFalse(u32),
    Call(Func, u8),
    Vec3Ctor(u8),
    Member(u8),
    ToFloat,
    ToInt,
}

/// A compiled expression.
#[derive(Debug, Clone)]
pub struct Program {
    pub(crate) code: Vec<Op>,
    pub(crate) spans: Vec<(u32, u32)>,
    pub(crate) nslots: usize,
    pub(crate) max_stack: usize,
    pub(crate) params: Vec<ParamDecl>,
    pub(crate) reads: Vec<usize>,
    pub(crate) writes: Vec<usize>,
    pub(crate) n_attrs: usize,
    pub(crate) warnings: Vec<Diagnostic>,
}

impl Program {
    /// Schema indices of the attributes the expression reads.
    pub fn reads(&self) -> &[usize] {
        &self.reads
    }
    /// Schema indices of the attributes it writes.
    pub fn writes(&self) -> &[usize] {
        &self.writes
    }
    pub fn params(&self) -> &[ParamDecl] {
        &self.params
    }
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
    /// Number of bytecode instructions.
    pub fn len(&self) -> usize {
        self.code.len()
    }
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }
}

#[derive(Clone)]
struct Var {
    name: String,
    slot: u16,
    ty: Ty,
    readonly: bool,
}

struct Compiler<'a> {
    schema: &'a Schema,
    reads: Vec<usize>,
    writes: Vec<usize>,
    scopes: Vec<Vec<Var>>,
    nslots: usize,
    code: Vec<Op>,
    spans: Vec<(u32, u32)>,
    cur: Span,
    depth: i32,
    max_depth: i32,
    warnings: Vec<Diagnostic>,
    iter_product: u64,
}

type Typed = (Ty, Option<usize>);

pub(crate) fn compile_program(parsed: Parsed, schema: &Schema) -> Result<Program> {
    let mut c = Compiler {
        schema,
        reads: Vec::new(),
        writes: Vec::new(),
        scopes: vec![Vec::new()],
        nslots: 0,
        code: Vec::new(),
        spans: Vec::new(),
        cur: Span::default(),
        depth: 0,
        max_depth: 0,
        warnings: Vec::new(),
        iter_product: 1,
    };
    for (list, out, what) in [(&parsed.reads, 0, "reads"), (&parsed.writes, 1, "writes")] {
        for (name, span) in list {
            let Some(i) = schema.index(name) else {
                return err("E-ATTR-1", *span, format!("ismeretlen attribútum: @{name}"), format!("unknown attribute @{name} in {what}"));
            };
            let v = if out == 0 { &mut c.reads } else { &mut c.writes };
            if !v.contains(&i) {
                v.push(i);
            }
        }
    }
    let mut params = Vec::new();
    for p in &parsed.params {
        if is_builtin(&p.name, schema.context) || c.lookup(&p.name).is_some() {
            return err("E-TYPE-5", p.span, format!("a név már foglalt: {}", p.name), format!("name already in use: {}", p.name));
        }
        let default = match (p.ty, p.default) {
            (Ty::Float, Value::Int(i)) => Value::Float(i as f32),
            (_, d) => d,
        };
        let slot = c.new_slot(p.span)?;
        c.scopes[0].push(Var { name: p.name.clone(), slot, ty: p.ty, readonly: true });
        params.push(ParamDecl { name: p.name.clone(), ty: p.ty, default });
    }
    c.scopes.push(Vec::new());
    for s in &parsed.body {
        c.stmt(s)?;
    }
    Ok(Program {
        code: c.code,
        spans: c.spans,
        nslots: c.nslots,
        max_stack: c.max_depth.max(0) as usize + 1,
        params,
        reads: c.reads,
        writes: c.writes,
        n_attrs: schema.attrs.len(),
        warnings: c.warnings,
    })
}

fn is_builtin(name: &str, ctx: Context) -> bool {
    match ctx {
        Context::Volume => matches!(name, "P" | "ijk" | "voxelsize"),
        Context::Points => matches!(name, "P" | "id"),
    }
}

fn type_name(t: Ty) -> &'static str {
    match t {
        Ty::Float => "float",
        Ty::Int => "int",
        Ty::Bool => "bool",
        Ty::Vec3 => "vec3",
    }
}

fn delta(op: &Op) -> i32 {
    match op {
        Op::PushF(_) | Op::PushI(_) | Op::PushB(_) | Op::Load(_) | Op::LoadAttr(_) | Op::LoadP | Op::LoadIjk | Op::LoadVoxelSize | Op::LoadId => 1,
        Op::Store(_) | Op::StoreAttr(_) | Op::JumpIfFalse(_) => -1,
        Op::Neg | Op::Not | Op::Member(_) | Op::ToFloat | Op::ToInt | Op::Jump(_) => 0,
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::Lt | Op::Le | Op::Gt | Op::Ge | Op::Eq | Op::Ne => -1,
        Op::Call(_, n) | Op::Vec3Ctor(n) => 1 - i32::from(*n),
    }
}

impl Compiler<'_> {
    fn lookup(&self, name: &str) -> Option<Var> {
        self.scopes.iter().rev().find_map(|s| s.iter().rev().find(|v| v.name == name)).cloned()
    }

    fn new_slot(&mut self, span: Span) -> Result<u16> {
        if self.nslots >= MAX_SLOTS {
            return err("E-LIMIT-3", span, "túl sok helyi változó (legfeljebb 256)", "too many local variables (at most 256)");
        }
        self.nslots += 1;
        Ok((self.nslots - 1) as u16)
    }

    fn emit(&mut self, op: Op) -> Result<usize> {
        if self.code.len() >= MAX_CODE {
            return err("E-LIMIT-4", self.cur, "a program túl hosszú", "program too long");
        }
        self.depth += delta(&op);
        self.max_depth = self.max_depth.max(self.depth);
        self.code.push(op);
        self.spans.push((self.cur.line, self.cur.col));
        Ok(self.code.len() - 1)
    }

    fn patch_jump(&mut self, at: usize) {
        let target = self.code.len() as u32;
        match &mut self.code[at] {
            Op::Jump(t) | Op::JumpIfFalse(t) => *t = target,
            _ => {}
        }
    }

    fn warn_literal(&mut self, span: Span) {
        self.warnings.push(diag("W-TYPE-1", span, "egész jegyű literál float helyén (float-ra alakítva)", "integer literal used as float (converted)"));
    }

    /// Make `got` have type `want`: only an integer literal may become a float.
    fn coerce(&mut self, got: Typed, want: Ty, span: Span) -> Result<()> {
        if got.0 == want {
            return Ok(());
        }
        if got.0 == Ty::Int
            && want == Ty::Float
            && let Some(at) = got.1
            && let Op::PushI(v) = self.code[at]
        {
            self.code[at] = Op::PushF(v as f32);
            self.warn_literal(span);
            return Ok(());
        }
        err(
            "E-TYPE-1",
            span,
            format!("típuseltérés: {} helyett {}", type_name(want), type_name(got.0)),
            format!("type mismatch: expected {}, found {}", type_name(want), type_name(got.0)),
        )
    }

    fn stmt(&mut self, s: &Stmt) -> Result<()> {
        match s {
            Stmt::Decl { ty, name, init, span } => {
                self.cur = *span;
                let got = self.expr(init)?;
                self.coerce(got, *ty, init.span)?;
                if is_builtin(name, self.schema.context)
                    || self.scopes.last().is_some_and(|sc| sc.iter().any(|v| v.name == *name))
                    || self.scopes[0].iter().any(|v| v.name == *name)
                {
                    return err("E-TYPE-5", *span, format!("a név már foglalt: {name}"), format!("name already declared: {name}"));
                }
                let slot = self.new_slot(*span)?;
                self.cur = *span;
                self.emit(Op::Store(slot))?;
                if let Some(sc) = self.scopes.last_mut() {
                    sc.push(Var { name: name.clone(), slot, ty: *ty, readonly: false });
                }
            }
            Stmt::Assign { target, value, span } => {
                self.cur = *span;
                match target {
                    Target::Var(n) => {
                        let Some(v) = self.lookup(n) else {
                            return err("E-TYPE-2", *span, format!("ismeretlen név: {n}"), format!("unknown name: {n}"));
                        };
                        if v.readonly {
                            return err("E-TYPE-6", *span, format!("nem írható: {n}"), format!("{n} is read-only"));
                        }
                        let got = self.expr(value)?;
                        self.coerce(got, v.ty, value.span)?;
                        self.cur = *span;
                        self.emit(Op::Store(v.slot))?;
                    }
                    Target::Attr(n) => {
                        let Some(i) = self.schema.index(n) else {
                            return err("E-ATTR-1", *span, format!("ismeretlen attribútum: @{n}"), format!("unknown attribute @{n}"));
                        };
                        if !self.writes.contains(&i) {
                            return err("E-ATTR-3", *span, format!("@{n} nincs a writes listában"), format!("@{n} is not in the writes list"));
                        }
                        let want = self.schema.attrs[i].1;
                        let got = self.expr(value)?;
                        self.coerce(got, want, value.span)?;
                        self.cur = *span;
                        self.emit(Op::StoreAttr(i as u16))?;
                    }
                }
            }
            Stmt::If { cond, then, els } => {
                let got = self.expr(cond)?;
                self.coerce(got, Ty::Bool, cond.span)?;
                self.cur = cond.span;
                let jf = self.emit(Op::JumpIfFalse(0))?;
                self.block(then)?;
                if els.is_empty() {
                    self.patch_jump(jf);
                } else {
                    let j = self.emit(Op::Jump(0))?;
                    self.patch_jump(jf);
                    self.block(els)?;
                    self.patch_jump(j);
                }
            }
            Stmt::For { var, start, end, body, span } => {
                let count = (i64::from(*end) - i64::from(*start)) as u64;
                let saved = self.iter_product;
                self.iter_product = self.iter_product.saturating_mul(count);
                if self.iter_product > MAX_ITER_PRODUCT {
                    return err("E-LOOP-3", *span, "a beágyazott ciklusok iterációszorzata túl nagy", "nested loops run too many iterations in total");
                }
                if is_builtin(var, self.schema.context) || self.lookup(var).is_some() {
                    return err("E-TYPE-5", *span, format!("a név már foglalt: {var}"), format!("name already declared: {var}"));
                }
                self.scopes.push(Vec::new());
                let slot = self.new_slot(*span)?;
                if let Some(sc) = self.scopes.last_mut() {
                    sc.push(Var { name: var.clone(), slot, ty: Ty::Int, readonly: true });
                }
                self.cur = *span;
                self.emit(Op::PushI(*start))?;
                self.emit(Op::Store(slot))?;
                let top = self.code.len() as u32;
                self.emit(Op::Load(slot))?;
                self.emit(Op::PushI(*end))?;
                self.emit(Op::Lt)?;
                let jf = self.emit(Op::JumpIfFalse(0))?;
                for s in body {
                    self.stmt(s)?;
                }
                self.cur = *span;
                self.emit(Op::Load(slot))?;
                self.emit(Op::PushI(1))?;
                self.emit(Op::Add)?;
                self.emit(Op::Store(slot))?;
                self.emit(Op::Jump(top))?;
                self.patch_jump(jf);
                self.scopes.pop();
                self.iter_product = saved;
            }
            Stmt::Block(b) => self.block(b)?,
        }
        Ok(())
    }

    fn block(&mut self, b: &[Stmt]) -> Result<()> {
        self.scopes.push(Vec::new());
        for s in b {
            self.stmt(s)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> Result<Typed> {
        match &e.kind {
            ExprKind::Int(i) => {
                self.cur = e.span;
                let at = self.emit(Op::PushI(*i))?;
                Ok((Ty::Int, Some(at)))
            }
            ExprKind::Float(f) => {
                self.cur = e.span;
                self.emit(Op::PushF(*f))?;
                Ok((Ty::Float, None))
            }
            ExprKind::Bool(b) => {
                self.cur = e.span;
                self.emit(Op::PushB(*b))?;
                Ok((Ty::Bool, None))
            }
            ExprKind::Var(n) => {
                self.cur = e.span;
                if let Some(v) = self.lookup(n) {
                    self.emit(Op::Load(v.slot))?;
                    return Ok((v.ty, None));
                }
                if is_builtin(n, self.schema.context) {
                    return Ok(match n.as_str() {
                        "P" => {
                            self.emit(Op::LoadP)?;
                            (Ty::Vec3, None)
                        }
                        "ijk" => {
                            self.emit(Op::LoadIjk)?;
                            (Ty::Vec3, None)
                        }
                        "voxelsize" => {
                            self.emit(Op::LoadVoxelSize)?;
                            (Ty::Float, None)
                        }
                        _ => {
                            self.emit(Op::LoadId)?;
                            (Ty::Int, None)
                        }
                    });
                }
                if matches!(n.as_str(), "P" | "ijk" | "voxelsize" | "id") {
                    return err("E-ATTR-4", e.span, format!("a(z) {n} nincs ebben a kontextusban"), format!("{n} does not exist in this context"));
                }
                err("E-TYPE-2", e.span, format!("ismeretlen név: {n}"), format!("unknown name: {n}"))
            }
            ExprKind::Attr(n) => {
                let Some(i) = self.schema.index(n) else {
                    return err("E-ATTR-1", e.span, format!("ismeretlen attribútum: @{n}"), format!("unknown attribute @{n}"));
                };
                if !self.reads.contains(&i) {
                    return err("E-ATTR-2", e.span, format!("@{n} nincs a reads listában"), format!("@{n} is not in the reads list"));
                }
                self.cur = e.span;
                self.emit(Op::LoadAttr(i as u16))?;
                Ok((self.schema.attrs[i].1, None))
            }
            ExprKind::Unary(UnOp::Neg, a) if matches!(a.kind, ExprKind::Int(_)) => {
                // `-3` is one literal (so it can still become a float).
                let ExprKind::Int(i) = a.kind else { return err("E-TYPE-1", e.span, "belső hiba", "internal error") };
                self.cur = e.span;
                let at = self.emit(Op::PushI(-i))?;
                Ok((Ty::Int, Some(at)))
            }
            ExprKind::Unary(op, a) => {
                let t = self.expr(a)?;
                self.cur = e.span;
                match (op, t.0) {
                    (UnOp::Neg, Ty::Float | Ty::Int | Ty::Vec3) => {
                        self.emit(Op::Neg)?;
                        Ok((t.0, None))
                    }
                    (UnOp::Not, Ty::Bool) => {
                        self.emit(Op::Not)?;
                        Ok((Ty::Bool, None))
                    }
                    _ => err("E-TYPE-1", e.span, "az operátor nem alkalmazható erre a típusra", format!("operator not applicable to {}", type_name(t.0))),
                }
            }
            ExprKind::Binary(op, a, b) => self.binary(e, *op, a, b),
            ExprKind::Ternary(c, a, b) => {
                let ct = self.expr(c)?;
                self.coerce(ct, Ty::Bool, c.span)?;
                self.cur = e.span;
                let jf = self.emit(Op::JumpIfFalse(0))?;
                let base = self.depth;
                let ta = self.expr(a)?;
                self.cur = e.span;
                let j = self.emit(Op::Jump(0))?;
                self.patch_jump(jf);
                self.depth = base;
                let tb = self.expr(b)?;
                self.patch_jump(j);
                self.unify(ta, tb, e.span)
            }
            ExprKind::Call(name, args) => self.call(e, name, args),
            ExprKind::Member(a, idx) => {
                let t = self.expr(a)?;
                if t.0 != Ty::Vec3 {
                    return err("E-TYPE-1", e.span, "csak vec3-nak van x/y/z tagja", "only vec3 has x/y/z members");
                }
                self.cur = e.span;
                self.emit(Op::Member(*idx))?;
                Ok((Ty::Float, None))
            }
        }
    }

    /// Two operands of one type; an integer literal may adapt to a float on the other side.
    fn unify(&mut self, a: Typed, b: Typed, span: Span) -> Result<Typed> {
        if a.0 == b.0 {
            return Ok((a.0, None));
        }
        if a.0 == Ty::Int && b.0 == Ty::Float && a.1.is_some() {
            self.coerce(a, Ty::Float, span)?;
            return Ok((Ty::Float, None));
        }
        if b.0 == Ty::Int && a.0 == Ty::Float && b.1.is_some() {
            self.coerce(b, Ty::Float, span)?;
            return Ok((Ty::Float, None));
        }
        err(
            "E-TYPE-1",
            span,
            format!("típuseltérés: {} és {}", type_name(a.0), type_name(b.0)),
            format!("type mismatch: {} and {}", type_name(a.0), type_name(b.0)),
        )
    }

    fn binary(&mut self, e: &Expr, op: BinOp, a: &Expr, b: &Expr) -> Result<Typed> {
        if matches!(op, BinOp::And | BinOp::Or) {
            let ta = self.expr(a)?;
            self.coerce(ta, Ty::Bool, a.span)?;
            self.cur = e.span;
            let base = self.depth;
            if op == BinOp::And {
                let jf = self.emit(Op::JumpIfFalse(0))?;
                let tb = self.expr(b)?;
                self.coerce(tb, Ty::Bool, b.span)?;
                self.cur = e.span;
                let j = self.emit(Op::Jump(0))?;
                self.patch_jump(jf);
                self.depth = base - 1;
                self.emit(Op::PushB(false))?;
                self.patch_jump(j);
            } else {
                // a || b  ==  a ? true : b
                let jf = self.emit(Op::JumpIfFalse(0))?;
                self.emit(Op::PushB(true))?;
                let j = self.emit(Op::Jump(0))?;
                self.patch_jump(jf);
                self.depth = base - 1;
                let tb = self.expr(b)?;
                self.coerce(tb, Ty::Bool, b.span)?;
                self.patch_jump(j);
            }
            return Ok((Ty::Bool, None));
        }
        let ta = self.expr(a)?;
        let tb = self.expr(b)?;
        let (ra, rb) = (ta.0, tb.0);
        // Mixed vec3/float scaling is the only legal mixed-type form.
        let scale = matches!(op, BinOp::Mul) && ((ra == Ty::Vec3 && rb == Ty::Float) || (ra == Ty::Float && rb == Ty::Vec3));
        let vdiv = matches!(op, BinOp::Div) && ra == Ty::Vec3 && rb == Ty::Float;
        let vscale_lit = matches!(op, BinOp::Mul | BinOp::Div) && ra == Ty::Vec3 && rb == Ty::Int && tb.1.is_some();
        let lit_scale = matches!(op, BinOp::Mul) && ra == Ty::Int && rb == Ty::Vec3 && ta.1.is_some();
        let (operand, result_vec) = if scale || vdiv {
            (Ty::Vec3, true)
        } else if vscale_lit {
            self.coerce(tb, Ty::Float, e.span)?;
            (Ty::Vec3, true)
        } else if lit_scale {
            self.coerce(ta, Ty::Float, e.span)?;
            (Ty::Vec3, true)
        } else {
            (self.unify(ta, tb, e.span)?.0, false)
        };
        self.cur = e.span;
        let (opc, result) = match op {
            BinOp::Add | BinOp::Sub => match operand {
                Ty::Float | Ty::Int | Ty::Vec3 => (if op == BinOp::Add { Op::Add } else { Op::Sub }, operand),
                Ty::Bool => return err("E-TYPE-1", e.span, "bool-on nincs aritmetika", "no arithmetic on bool"),
            },
            BinOp::Mul => (Op::Mul, if result_vec { Ty::Vec3 } else { arith(operand, e.span)? }),
            BinOp::Div => (Op::Div, if result_vec { Ty::Vec3 } else { arith_novec(operand, e.span)? }),
            BinOp::Rem => (Op::Rem, arith_novec(operand, e.span)?),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if !matches!(operand, Ty::Float | Ty::Int) {
                    return err("E-TYPE-1", e.span, "a rendezés float vagy int típuson értelmezett", "ordering needs float or int");
                }
                (
                    match op {
                        BinOp::Lt => Op::Lt,
                        BinOp::Le => Op::Le,
                        BinOp::Gt => Op::Gt,
                        _ => Op::Ge,
                    },
                    Ty::Bool,
                )
            }
            BinOp::Eq => (Op::Eq, Ty::Bool),
            BinOp::Ne => (Op::Ne, Ty::Bool),
            BinOp::And | BinOp::Or => return err("E-TYPE-1", e.span, "belső hiba", "internal error"),
        };
        self.emit(opc)?;
        Ok((result, None))
    }

    fn call(&mut self, e: &Expr, name: &str, args: &[Expr]) -> Result<Typed> {
        let mut typed = Vec::with_capacity(args.len());
        for a in args {
            typed.push(self.expr(a)?);
        }
        self.cur = e.span;
        let tys: Vec<Ty> = typed.iter().map(|t| t.0).collect();
        // Conversions and the vec3 constructor have their own rules.
        match name {
            "float" | "int" => {
                if tys.len() != 1 {
                    return err("E-TYPE-4", e.span, "egy argumentum kell", "exactly one argument is expected");
                }
                let (to, op) = if name == "float" { (Ty::Float, Op::ToFloat) } else { (Ty::Int, Op::ToInt) };
                if !matches!(tys[0], Ty::Float | Ty::Int) {
                    return err("E-TYPE-1", e.span, "csak float és int alakítható", "only float and int convert");
                }
                self.emit(op)?;
                return Ok((to, None));
            }
            "vec3" => {
                if tys.len() != 1 && tys.len() != 3 {
                    return err("E-TYPE-4", e.span, "a vec3 egy vagy három argumentumot vár", "vec3 takes one or three arguments");
                }
                for t in &typed {
                    self.coerce(*t, Ty::Float, e.span)?;
                }
                self.emit(Op::Vec3Ctor(tys.len() as u8))?;
                return Ok((Ty::Vec3, None));
            }
            _ => {}
        }
        let mut hit = resolve(name, &tys);
        // Integer literals may stand where a float is expected.
        if hit.is_none() {
            let as_float: Vec<Ty> = typed.iter().map(|t| if t.0 == Ty::Int && t.1.is_some() { Ty::Float } else { t.0 }).collect();
            if as_float != tys
                && let Some(h) = resolve(name, &as_float)
            {
                for t in &typed {
                    if t.0 == Ty::Int && t.1.is_some() {
                        self.coerce(*t, Ty::Float, e.span)?;
                    }
                }
                hit = Some(h);
            }
        }
        let Some((func, ret)) = hit else {
            if !known_function(name) {
                return err("E-TYPE-3", e.span, format!("ismeretlen függvény: {name}"), format!("unknown function: {name}"));
            }
            return err(
                "E-TYPE-4",
                e.span,
                format!("a(z) {name} nem hívható ezekkel az argumentumtípusokkal"),
                format!("{name} cannot be called with these argument types"),
            );
        };
        self.emit(Op::Call(func, tys.len() as u8))?;
        Ok((ret, None))
    }
}

fn arith(t: Ty, span: Span) -> Result<Ty> {
    match t {
        Ty::Float | Ty::Int | Ty::Vec3 => Ok(t),
        Ty::Bool => err("E-TYPE-1", span, "bool-on nincs aritmetika", "no arithmetic on bool"),
    }
}

fn arith_novec(t: Ty, span: Span) -> Result<Ty> {
    match t {
        Ty::Float | Ty::Int => Ok(t),
        _ => err("E-TYPE-1", span, "az operátor float vagy int típuson értelmezett", "operator needs float or int"),
    }
}

const FUNCS: &[(&str, Func)] = &[
    ("abs", Func::Abs),
    ("min", Func::Min),
    ("max", Func::Max),
    ("clamp", Func::Clamp),
    ("floor", Func::Floor),
    ("ceil", Func::Ceil),
    ("round", Func::Round),
    ("sqrt", Func::Sqrt),
    ("pow", Func::Pow),
    ("exp", Func::Exp),
    ("log", Func::Log),
    ("sin", Func::Sin),
    ("cos", Func::Cos),
    ("tan", Func::Tan),
    ("atan2", Func::Atan2),
    ("lerp", Func::Lerp),
    ("smoothstep", Func::Smoothstep),
    ("step", Func::Step),
    ("fit", Func::Fit),
    ("length", Func::Length),
    ("dot", Func::Dot),
    ("cross", Func::Cross),
    ("normalize", Func::Normalize),
    ("noise", Func::Noise),
    ("hash", Func::Hash),
    ("rand", Func::Rand),
];

fn known_function(name: &str) -> bool {
    FUNCS.iter().any(|(n, _)| *n == name)
}

fn resolve(name: &str, a: &[Ty]) -> Option<(Func, Ty)> {
    use Ty::{Float as F, Int as I, Vec3 as V};
    let f = FUNCS.iter().find(|(n, _)| *n == name)?.1;
    let ret = match (f, a) {
        (Func::Abs, [I]) => I,
        (Func::Abs | Func::Floor | Func::Ceil | Func::Round | Func::Sqrt | Func::Exp | Func::Log | Func::Sin | Func::Cos | Func::Tan, [F]) => F,
        (Func::Abs | Func::Floor | Func::Ceil | Func::Round | Func::Sqrt | Func::Exp | Func::Log | Func::Sin | Func::Cos | Func::Tan, [V]) => V,
        (Func::Min | Func::Max, [I, I]) => I,
        (Func::Min | Func::Max | Func::Pow | Func::Atan2, [F, F]) => F,
        (Func::Min | Func::Max | Func::Pow | Func::Atan2, [V, V]) => V,
        (Func::Clamp, [I, I, I]) => I,
        (Func::Clamp, [F, F, F]) => F,
        (Func::Clamp, [V, V, V] | [V, F, F]) => V,
        (Func::Lerp, [F, F, F]) => F,
        (Func::Lerp, [V, V, F] | [V, V, V]) => V,
        (Func::Smoothstep, [F, F, F]) => F,
        (Func::Smoothstep, [F, F, V]) => V,
        (Func::Step, [F, F]) => F,
        (Func::Step, [F, V]) => V,
        (Func::Fit, [F, F, F, F, F]) => F,
        (Func::Length | Func::Dot | Func::Noise, [V] | [V, V]) if (f == Func::Dot) == (a.len() == 2) => F,
        (Func::Cross, [V, V]) => V,
        (Func::Normalize, [V]) => V,
        (Func::Hash, xs) if (1..=4).contains(&xs.len()) && xs.iter().all(|t| *t == I) => I,
        (Func::Rand, [I, I]) => F,
        _ => return None,
    };
    Some((f, ret))
}
