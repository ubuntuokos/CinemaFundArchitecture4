use crate::error::Span;
use crate::types::{Ty, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
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
    And,
    Or,
}

#[derive(Debug, Clone)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Height of the tree below and including this node (bounded by the parser).
    pub depth: u32,
}

#[derive(Debug, Clone)]
pub(crate) enum ExprKind {
    Int(i32),
    Float(f32),
    Bool(bool),
    Var(String),
    Attr(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Member(Box<Expr>, u8),
}

#[derive(Debug, Clone)]
pub(crate) enum Target {
    Var(String),
    Attr(String),
}

#[derive(Debug, Clone)]
pub(crate) enum Stmt {
    Decl { ty: Ty, name: String, init: Expr, span: Span },
    Assign { target: Target, value: Expr, span: Span },
    If { cond: Expr, then: Vec<Stmt>, els: Vec<Stmt> },
    For { var: String, start: i32, end: i32, body: Vec<Stmt>, span: Span },
    Block(Vec<Stmt>),
}

#[derive(Debug, Clone)]
pub(crate) struct ParamHeader {
    pub name: String,
    pub ty: Ty,
    pub default: Value,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Parsed {
    pub reads: Vec<(String, Span)>,
    pub writes: Vec<(String, Span)>,
    pub params: Vec<ParamHeader>,
    pub body: Vec<Stmt>,
}
