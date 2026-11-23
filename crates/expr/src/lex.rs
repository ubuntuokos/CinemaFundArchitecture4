//! Lexer. Source is limited to 64 KiB by the caller; identifiers to 64 characters.

use crate::error::{Result, Span, err};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Int(i32),
    Float(f32),
    Ident(String),
    Attr(String),
    Kw(&'static str),
    Sym(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub span: Span,
}

const KEYWORDS: &[&str] = &["reads", "writes", "param", "if", "else", "for", "in", "true", "false", "float", "int", "bool", "vec3"];
const SYMS2: &[&str] = &["&&", "||", "==", "!=", "<=", ">=", ".."];
const SYMS1: &[&str] = &["+", "-", "*", "/", "%", "(", ")", "<", ">", "{", "}", ";", ",", "=", "!", "?", ":", "."];
pub(crate) const MAX_IDENT: usize = 64;

pub(crate) fn lex(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    macro_rules! adv {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            adv!();
            continue;
        }
        let start = (line, col);
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                adv!();
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            adv!();
            adv!();
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    adv!();
                    adv!();
                    closed = true;
                    break;
                }
                adv!();
            }
            if !closed {
                return err("E-LEX-2", Span { line: start.0, col: start.1, len: 2 }, "lezáratlan megjegyzés", "unterminated comment");
            }
            continue;
        }
        if c.is_ascii_digit() {
            let s = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                adv!();
            }
            let mut is_float = false;
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                is_float = true;
                adv!();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    adv!();
                }
            }
            if matches!(chars.get(i), Some('e' | 'E')) {
                let n1 = chars.get(i + 1);
                let n2 = chars.get(i + 2);
                let ok = n1.is_some_and(|d| d.is_ascii_digit()) || (matches!(n1, Some('+' | '-')) && n2.is_some_and(|d| d.is_ascii_digit()));
                if ok {
                    is_float = true;
                    adv!();
                    if matches!(chars.get(i), Some('+' | '-')) {
                        adv!();
                    }
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        adv!();
                    }
                }
            }
            let text: String = chars[s..i].iter().collect();
            let span = Span { line: start.0, col: start.1, len: (i - s) as u32 };
            let tok = if is_float {
                match text.parse::<f32>() {
                    Ok(v) if v.is_finite() => Tok::Float(v),
                    _ => return err("E-LEX-3", span, "érvénytelen szám", "invalid number literal"),
                }
            } else {
                match text.parse::<i32>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => return err("E-LEX-3", span, "az egész szám túl nagy", "integer literal out of range"),
                }
            };
            out.push(Token { tok, span });
            continue;
        }
        if c == '@' || c.is_ascii_alphabetic() || c == '_' {
            let is_attr = c == '@';
            if is_attr {
                adv!();
            }
            let s = i;
            if !chars.get(i).is_some_and(|d| d.is_ascii_alphabetic() || *d == '_') {
                return err("E-LEX-1", Span { line: start.0, col: start.1, len: 1 }, "az @ után azonosító kell", "an identifier must follow @");
            }
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                adv!();
            }
            let text: String = chars[s..i].iter().collect();
            let span = Span { line: start.0, col: start.1, len: (i - s) as u32 + u32::from(is_attr) };
            if text.len() > MAX_IDENT {
                return err("E-LEX-4", span, "túl hosszú azonosító", "identifier too long");
            }
            let tok = if is_attr {
                Tok::Attr(text)
            } else if let Some(k) = KEYWORDS.iter().find(|k| **k == text) {
                Tok::Kw(k)
            } else {
                Tok::Ident(text)
            };
            out.push(Token { tok, span });
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if let Some(s) = SYMS2.iter().find(|s| **s == two) {
            adv!();
            adv!();
            out.push(Token { tok: Tok::Sym(s), span: Span { line: start.0, col: start.1, len: 2 } });
            continue;
        }
        let one = c.to_string();
        if let Some(s) = SYMS1.iter().find(|s| **s == one) {
            adv!();
            out.push(Token { tok: Tok::Sym(s), span: Span { line: start.0, col: start.1, len: 1 } });
            continue;
        }
        return err("E-LEX-1", Span { line: start.0, col: start.1, len: 1 }, format!("ismeretlen karakter: {c:?}"), format!("unknown character: {c:?}"));
    }
    out.push(Token { tok: Tok::Eof, span: Span { line, col, len: 0 } });
    Ok(out)
}
