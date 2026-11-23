//! Recursive-descent parser with hard nesting limits (no stack overflow on hostile input).

use crate::ast::*;
use crate::error::{Result, Span, err};
use crate::lex::{Tok, Token};
use crate::types::{Ty, Value};

pub(crate) const MAX_EXPR_DEPTH: u32 = 64;
const MAX_BLOCK_DEPTH: u32 = 32;
const MAX_ARGS: usize = 8;

pub(crate) struct Parser {
    toks: Vec<Token>,
    pos: usize,
    nest: u32,
    blocks: u32,
}

fn type_of_kw(k: &str) -> Option<Ty> {
    match k {
        "float" => Some(Ty::Float),
        "int" => Some(Ty::Int),
        "bool" => Some(Ty::Bool),
        "vec3" => Some(Ty::Vec3),
        _ => None,
    }
}

impl Parser {
    pub(crate) fn new(toks: Vec<Token>) -> Self {
        Parser { toks, pos: 0, nest: 0, blocks: 0 }
    }

    fn peek(&self) -> &Token {
        // The lexer always ends with Eof and `pos` never passes it.
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn next(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(&self.peek().tok, Tok::Sym(x) if *x == s)
    }

    fn is_kw(&self, s: &str) -> bool {
        matches!(&self.peek().tok, Tok::Kw(x) if *x == s)
    }

    fn unexpected<T>(&self, want: &str) -> Result<T> {
        let t = self.peek();
        let found = describe(&t.tok);
        err("E-PARSE-1", t.span, format!("váratlan jel: {found}; várt: {want}"), format!("unexpected {found}; expected {want}"))
    }

    fn expect_sym(&mut self, s: &str) -> Result<Span> {
        if self.is_sym(s) { Ok(self.next().span) } else { self.unexpected(&format!("`{s}`")) }
    }

    fn enter(&mut self) -> Result<()> {
        self.nest += 1;
        if self.nest > MAX_EXPR_DEPTH {
            return err("E-LIMIT-2", self.peek().span, "a kifejezés túl mélyen ágyazott", "expression nested too deeply");
        }
        Ok(())
    }

    pub(crate) fn parse_program(&mut self) -> Result<Parsed> {
        let mut p = Parsed::default();
        loop {
            if self.is_kw("reads") || self.is_kw("writes") {
                let reads = self.is_kw("reads");
                self.next();
                loop {
                    let t = self.next();
                    let Tok::Attr(name) = t.tok else {
                        return err("E-PARSE-1", t.span, "attribútum (@név) kell", "an attribute (@name) is expected");
                    };
                    if reads {
                        p.reads.push((name, t.span))
                    } else {
                        p.writes.push((name, t.span))
                    }
                    if self.is_sym(",") {
                        self.next();
                    } else {
                        break;
                    }
                }
                self.expect_sym(";")?;
            } else if self.is_kw("param") {
                self.next();
                let tt = self.next();
                let Some(ty) = (match &tt.tok {
                    Tok::Kw(k) => type_of_kw(k),
                    _ => None,
                }) else {
                    return err("E-PARSE-1", tt.span, "típus kell (float, int, bool, vec3)", "a type is expected (float, int, bool, vec3)");
                };
                let nt = self.next();
                let Tok::Ident(name) = nt.tok else {
                    return err("E-PARSE-1", nt.span, "paraméternév kell", "a parameter name is expected");
                };
                self.expect_sym("=")?;
                let default = self.parse_literal(ty)?;
                self.expect_sym(";")?;
                p.params.push(ParamHeader { name, ty, default, span: nt.span });
            } else {
                break;
            }
        }
        while self.peek().tok != Tok::Eof {
            if self.is_kw("reads") || self.is_kw("writes") || self.is_kw("param") {
                return err(
                    "E-PARSE-2",
                    self.peek().span,
                    "a fejléc (reads/writes/param) az utasítások előtt áll",
                    "header lines (reads/writes/param) must come before statements",
                );
            }
            let s = self.parse_stmt()?;
            p.body.push(s);
        }
        Ok(p)
    }

    fn parse_num(&mut self, float: bool) -> Result<f32> {
        let neg = if self.is_sym("-") {
            self.next();
            true
        } else {
            false
        };
        let t = self.next();
        let v = match t.tok {
            Tok::Float(f) if float => f,
            Tok::Int(i) => i as f32,
            _ => return err("E-PARSE-1", t.span, "szám-literál kell", "a numeric literal is expected"),
        };
        Ok(if neg { -v } else { v })
    }

    fn parse_literal(&mut self, ty: Ty) -> Result<Value> {
        match ty {
            Ty::Float => Ok(Value::Float(self.parse_num(true)?)),
            Ty::Int => {
                let neg = if self.is_sym("-") {
                    self.next();
                    true
                } else {
                    false
                };
                let t = self.next();
                match t.tok {
                    Tok::Int(i) => Ok(Value::Int(if neg { -i } else { i })),
                    _ => err("E-PARSE-1", t.span, "egész literál kell", "an integer literal is expected"),
                }
            }
            Ty::Bool => {
                let t = self.next();
                match t.tok {
                    Tok::Kw("true") => Ok(Value::Bool(true)),
                    Tok::Kw("false") => Ok(Value::Bool(false)),
                    _ => err("E-PARSE-1", t.span, "true vagy false kell", "true or false is expected"),
                }
            }
            Ty::Vec3 => {
                let t = self.next();
                if t.tok != Tok::Kw("vec3") {
                    return err("E-PARSE-1", t.span, "vec3(…) literál kell", "a vec3(…) literal is expected");
                }
                self.expect_sym("(")?;
                let a = self.parse_num(true)?;
                let v = if self.is_sym(",") {
                    self.next();
                    let b = self.parse_num(true)?;
                    self.expect_sym(",")?;
                    let c = self.parse_num(true)?;
                    [a, b, c]
                } else {
                    [a; 3]
                };
                self.expect_sym(")")?;
                Ok(Value::Vec3(v))
            }
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.blocks += 1;
        if self.blocks > MAX_BLOCK_DEPTH {
            return err("E-LIMIT-2", self.peek().span, "a blokk túl mélyen ágyazott", "block nested too deeply");
        }
        self.expect_sym("{")?;
        let mut v = Vec::new();
        while !self.is_sym("}") {
            if self.peek().tok == Tok::Eof {
                return self.unexpected("`}`");
            }
            v.push(self.parse_stmt()?);
        }
        self.next();
        self.blocks -= 1;
        Ok(v)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        let t = self.peek().clone();
        match &t.tok {
            Tok::Kw(k) if type_of_kw(k).is_some() && matches!(self.peek_at(1), Tok::Ident(_)) => {
                let ty = type_of_kw(k).unwrap_or(Ty::Float);
                self.next();
                let nt = self.next();
                let Tok::Ident(name) = nt.tok else { return self.unexpected("name") };
                self.expect_sym("=")?;
                let init = self.parse_expr()?;
                self.expect_sym(";")?;
                Ok(Stmt::Decl { ty, name, init, span: nt.span })
            }
            Tok::Kw("if") => {
                self.next();
                self.expect_sym("(")?;
                let cond = self.parse_expr()?;
                self.expect_sym(")")?;
                let then = self.parse_block()?;
                let mut els = Vec::new();
                if self.is_kw("else") {
                    self.next();
                    if self.is_kw("if") {
                        self.blocks += 1;
                        if self.blocks > MAX_BLOCK_DEPTH {
                            return err("E-LIMIT-2", self.peek().span, "az else-if lánc túl hosszú", "else-if chain too long");
                        }
                        els.push(self.parse_stmt()?);
                        self.blocks -= 1;
                    } else {
                        els = self.parse_block()?;
                    }
                }
                Ok(Stmt::If { cond, then, els })
            }
            Tok::Kw("for") => {
                self.next();
                let nt = self.next();
                let Tok::Ident(var) = nt.tok else {
                    return err("E-PARSE-1", nt.span, "ciklusváltozó-név kell", "a loop variable name is expected");
                };
                if !self.is_kw("in") {
                    return self.unexpected("`in`");
                }
                self.next();
                let a = self.next();
                let Tok::Int(start) = a.tok else {
                    return err(
                        "E-LOOP-1",
                        a.span,
                        "a ciklushatár fordításkor ismert egész konstans kell legyen",
                        "loop bounds must be integer constants known at compile time",
                    );
                };
                self.expect_sym("..")?;
                let b = self.next();
                let Tok::Int(end) = b.tok else {
                    return err(
                        "E-LOOP-1",
                        b.span,
                        "a ciklushatár fordításkor ismert egész konstans kell legyen",
                        "loop bounds must be integer constants known at compile time",
                    );
                };
                let count = i64::from(end) - i64::from(start);
                if !(1..=1024).contains(&count) {
                    return err("E-LOOP-2", a.span, "a ciklus iterációszáma 1 és 1024 között lehet", "a loop must run between 1 and 1024 iterations");
                }
                let body = self.parse_block()?;
                Ok(Stmt::For { var, start, end, body, span: nt.span })
            }
            Tok::Sym("{") => Ok(Stmt::Block(self.parse_block()?)),
            Tok::Ident(n) if matches!(self.peek_at(1), Tok::Sym("=")) => {
                let name = n.clone();
                self.next();
                self.next();
                let value = self.parse_expr()?;
                self.expect_sym(";")?;
                Ok(Stmt::Assign { target: Target::Var(name), value, span: t.span })
            }
            Tok::Attr(n) if matches!(self.peek_at(1), Tok::Sym("=")) => {
                let name = n.clone();
                self.next();
                self.next();
                let value = self.parse_expr()?;
                self.expect_sym(";")?;
                Ok(Stmt::Assign { target: Target::Attr(name), value, span: t.span })
            }
            _ => self.unexpected("a statement"),
        }
    }

    pub(crate) fn parse_expr(&mut self) -> Result<Expr> {
        self.enter()?;
        let c = self.parse_bin(0)?;
        let r = if self.is_sym("?") {
            let q = self.next().span;
            let a = self.parse_expr()?;
            self.expect_sym(":")?;
            let b = self.parse_expr()?;
            let depth = 1 + c.depth.max(a.depth).max(b.depth);
            mk(ExprKind::Ternary(Box::new(c), Box::new(a), Box::new(b)), q, depth)?
        } else {
            c
        };
        self.nest -= 1;
        Ok(r)
    }

    fn parse_bin(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let t = self.peek().clone();
            let Tok::Sym(s) = t.tok else { break };
            let (prec, op) = match s {
                "||" => (0, BinOp::Or),
                "&&" => (1, BinOp::And),
                "==" => (2, BinOp::Eq),
                "!=" => (2, BinOp::Ne),
                "<" => (3, BinOp::Lt),
                "<=" => (3, BinOp::Le),
                ">" => (3, BinOp::Gt),
                ">=" => (3, BinOp::Ge),
                "+" => (4, BinOp::Add),
                "-" => (4, BinOp::Sub),
                "*" => (5, BinOp::Mul),
                "/" => (5, BinOp::Div),
                "%" => (5, BinOp::Rem),
                _ => break,
            };
            if prec < min {
                break;
            }
            self.next();
            let rhs = self.parse_bin(prec + 1)?;
            let depth = 1 + lhs.depth.max(rhs.depth);
            lhs = mk(ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), t.span, depth)?;
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let t = self.peek().clone();
        let op = match t.tok {
            Tok::Sym("-") => UnOp::Neg,
            Tok::Sym("!") => UnOp::Not,
            _ => return self.parse_postfix(),
        };
        self.next();
        self.enter()?;
        let e = self.parse_unary()?;
        self.nest -= 1;
        let depth = 1 + e.depth;
        mk(ExprKind::Unary(op, Box::new(e)), t.span, depth)
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut e = self.parse_primary()?;
        while self.is_sym(".") {
            self.next();
            let t = self.next();
            let idx = match &t.tok {
                Tok::Ident(n) if n == "x" => 0,
                Tok::Ident(n) if n == "y" => 1,
                Tok::Ident(n) if n == "z" => 2,
                _ => return err("E-PARSE-1", t.span, "x, y vagy z kell", "x, y or z is expected"),
            };
            let depth = 1 + e.depth;
            e = mk(ExprKind::Member(Box::new(e), idx), t.span, depth)?;
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let t = self.next();
        match t.tok {
            Tok::Int(i) => mk(ExprKind::Int(i), t.span, 1),
            Tok::Float(f) => mk(ExprKind::Float(f), t.span, 1),
            Tok::Kw("true") => mk(ExprKind::Bool(true), t.span, 1),
            Tok::Kw("false") => mk(ExprKind::Bool(false), t.span, 1),
            Tok::Attr(n) => mk(ExprKind::Attr(n), t.span, 1),
            Tok::Ident(n) => {
                if self.is_sym("(") {
                    self.parse_call(n, t.span)
                } else {
                    mk(ExprKind::Var(n), t.span, 1)
                }
            }
            Tok::Kw(k) if type_of_kw(k).is_some() && self.is_sym("(") => self.parse_call(k.to_string(), t.span),
            Tok::Sym("(") => {
                self.enter()?;
                let e = self.parse_expr()?;
                self.nest -= 1;
                self.expect_sym(")")?;
                Ok(e)
            }
            _ => {
                self.pos = self.pos.saturating_sub(1);
                self.unexpected("an expression")
            }
        }
    }

    fn parse_call(&mut self, name: String, span: Span) -> Result<Expr> {
        self.expect_sym("(")?;
        let mut args = Vec::new();
        if !self.is_sym(")") {
            loop {
                args.push(self.parse_expr()?);
                if args.len() > MAX_ARGS {
                    return err("E-TYPE-4", span, "túl sok argumentum", "too many arguments");
                }
                if self.is_sym(",") {
                    self.next();
                } else {
                    break;
                }
            }
        }
        self.expect_sym(")")?;
        let depth = 1 + args.iter().map(|a| a.depth).max().unwrap_or(0);
        mk(ExprKind::Call(name, args), span, depth)
    }
}

fn mk(kind: ExprKind, span: Span, depth: u32) -> Result<Expr> {
    if depth > MAX_EXPR_DEPTH {
        return err("E-LIMIT-2", span, "a kifejezésfa túl mély", "expression tree too deep");
    }
    Ok(Expr { kind, span, depth })
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Int(i) => format!("`{i}`"),
        Tok::Float(f) => format!("`{f}`"),
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Attr(s) => format!("`@{s}`"),
        Tok::Kw(s) | Tok::Sym(s) => format!("`{s}`"),
        Tok::Eof => "end of input".to_string(),
    }
}
