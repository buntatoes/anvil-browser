// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Serialization of tokens and component values back to CSS text.
//!
//! Written from CSS Syntax Level 3 section 9 ("Serialization") and the
//! CSSOM rules for serializing identifiers and strings. Where two tokens
//! would merge into one when re-read (an ident next to a number, `/` next
//! to `*`), an empty comment `/**/` goes between them, so tokenizing the
//! output gives the same tokens back. Bad-string and bad-url tokens are
//! written in a form that reads back as the same bad token.

use std::fmt::Write;

use crate::prism::parser::{BlockKind, ComponentValue};
use crate::prism::tokenizer::{Number, Token};

/// Write an identifier, escaping what would not read back as the same
/// identifier.
pub fn serialize_identifier(s: &str, out: &mut String) {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 && chars[0] == '-' {
        out.push_str("\\-");
        return;
    }
    for (i, &c) in chars.iter().enumerate() {
        let leading_digit = c.is_ascii_digit() && (i == 0 || (i == 1 && chars[0] == '-'));
        if c == '\0' {
            out.push('\u{FFFD}');
        } else if ('\u{1}'..='\u{1F}').contains(&c) || c == '\u{7F}' || leading_digit {
            let _ = write!(out, "\\{:x} ", c as u32);
        } else if !c.is_ascii() || c == '-' || c == '_' || c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
}

/// Write the name part of a hash or unit, which may start with a digit.
fn serialize_name(s: &str, out: &mut String) {
    for c in s.chars() {
        if c == '\0' {
            out.push('\u{FFFD}');
        } else if ('\u{1}'..='\u{1F}').contains(&c) || c == '\u{7F}' {
            let _ = write!(out, "\\{:x} ", c as u32);
        } else if !c.is_ascii() || c == '-' || c == '_' || c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
}

/// Write a double-quoted string.
pub fn serialize_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if ('\u{1}'..='\u{1F}').contains(&c) || c == '\u{7F}' => {
                let _ = write!(out, "\\{:x} ", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn serialize_url_body(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\\' | '"' | '\'' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_whitespace() || ('\u{1}'..='\u{1F}').contains(&c) || c == '\u{7F}' => {
                let _ = write!(out, "\\{:x} ", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn serialize_number(n: &Number, out: &mut String) {
    let v = n.value;
    if v.is_infinite() {
        // Only reachable from literals too large for f64; this reads back
        // as the same infinity.
        out.push_str(if v > 0.0 { "1e999" } else { "-1e999" });
        return;
    }
    if n.has_sign && !v.is_sign_negative() {
        out.push('+');
    }
    if v == 0.0 && v.is_sign_negative() {
        out.push('-');
    }
    if n.is_integer && v.abs() < 1e15 {
        let _ = write!(out, "{}", v.abs() as i64 * if v < 0.0 { -1 } else { 1 });
        return;
    }
    let text = format!("{}", if v == 0.0 { 0.0 } else { v });
    out.push_str(&text);
    if !n.is_integer && !text.contains(['.', 'e', 'E']) {
        out.push_str(".0");
    }
}

/// Write a dimension's unit, escaping a leading `e` that would read back
/// as an exponent.
fn serialize_unit(unit: &str, out: &mut String) {
    let mut chars = unit.chars();
    let first = chars.next();
    let second = chars.next();
    let third = chars.next();
    let looks_like_exponent = matches!(first, Some('e' | 'E'))
        && (matches!(second, Some(c) if c.is_ascii_digit())
            || (matches!(second, Some('+' | '-'))
                && matches!(third, Some(c) if c.is_ascii_digit())));
    if looks_like_exponent {
        let _ = write!(out, "\\{:x} ", first.map(|c| c as u32).unwrap_or(0x65));
        serialize_name(&unit[1..], out);
    } else if unit.starts_with('-') || unit.starts_with(|c: char| c.is_ascii_digit()) {
        serialize_identifier(unit, out);
    } else {
        serialize_name(unit, out);
    }
}

/// Write one token.
pub fn serialize_token(t: &Token, out: &mut String) {
    match t {
        Token::Ident(s) => serialize_identifier(s, out),
        Token::Function(s) => {
            serialize_identifier(s, out);
            out.push('(');
        }
        Token::AtKeyword(s) => {
            out.push('@');
            serialize_identifier(s, out);
        }
        Token::Hash { value, is_id } => {
            out.push('#');
            if *is_id {
                serialize_identifier(value, out);
            } else {
                serialize_name(value, out);
            }
        }
        Token::String(s) => serialize_string(s, out),
        Token::BadString => out.push_str("\"\n"),
        Token::Url(u) => {
            out.push_str("url(");
            serialize_url_body(u, out);
            out.push(')');
        }
        Token::BadUrl => out.push_str("url(()"),
        Token::Delim('\\') => out.push_str("\\\n"),
        Token::Delim(c) => out.push(*c),
        Token::Number(n) => serialize_number(n, out),
        Token::Percentage(n) => {
            serialize_number(n, out);
            out.push('%');
        }
        Token::Dimension { value, unit } => {
            serialize_number(value, out);
            serialize_unit(unit, out);
        }
        Token::UnicodeRange { start, end } => {
            if start == end {
                let _ = write!(out, "U+{start:X}");
            } else {
                let _ = write!(out, "U+{start:X}-{end:X}");
            }
        }
        Token::Whitespace => out.push(' '),
        Token::Cdo => out.push_str("<!--"),
        Token::Cdc => out.push_str("-->"),
        Token::Colon => out.push(':'),
        Token::Semicolon => out.push(';'),
        Token::Comma => out.push(','),
        Token::LeftBracket => out.push('['),
        Token::RightBracket => out.push(']'),
        Token::LeftParen => out.push('('),
        Token::RightParen => out.push(')'),
        Token::LeftBrace => out.push('{'),
        Token::RightBrace => out.push('}'),
    }
}

/// The kind of token at the start or end of a component value, for deciding
/// whether a comment must separate two neighbors.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Ident,
    /// The identifier `--`, which reads as `-->` when a `>` follows.
    DashDash,
    Function,
    Url,
    BadUrl,
    AtKeyword,
    Hash,
    Number,
    Percentage,
    Dimension,
    Cdc,
    Delim(char),
    Open,
    Other,
}

fn boundary(t: &Token) -> Boundary {
    match t {
        Token::Ident(s) if s == "--" => Boundary::DashDash,
        Token::Ident(_) => Boundary::Ident,
        Token::Function(_) => Boundary::Function,
        Token::Url(_) => Boundary::Url,
        Token::BadUrl => Boundary::BadUrl,
        Token::AtKeyword(_) => Boundary::AtKeyword,
        Token::Hash { .. } => Boundary::Hash,
        Token::Number(_) => Boundary::Number,
        Token::Percentage(_) => Boundary::Percentage,
        Token::Dimension { .. } => Boundary::Dimension,
        Token::Cdc => Boundary::Cdc,
        Token::Delim(c) => Boundary::Delim(*c),
        Token::LeftParen => Boundary::Open,
        _ => Boundary::Other,
    }
}

/// True when writing `a` then `b` directly would read back differently.
fn needs_comment(a: Boundary, b: Boundary) -> bool {
    use Boundary::*;
    let word = |e: Boundary| {
        matches!(
            e,
            Ident
                | DashDash
                | Function
                | Url
                | BadUrl
                | Delim('-')
                | Number
                | Percentage
                | Dimension
        )
    };
    match a {
        Ident => word(b) || b == Cdc || b == Open,
        DashDash => word(b) || b == Cdc || b == Open || b == Delim('>'),
        AtKeyword | Hash | Dimension => word(b) || b == Cdc,
        Delim('#') | Delim('-') => word(b),
        Number => word(b) || b == Delim('%'),
        Delim('@') => matches!(
            b,
            Ident | DashDash | Function | Url | BadUrl | Delim('-') | Cdc
        ),
        Delim('.') | Delim('+') => matches!(b, Number | Percentage | Dimension),
        Delim('/') => b == Delim('*'),
        Delim('<') => b == Delim('!'),
        _ => false,
    }
}

fn first_boundary(v: &ComponentValue) -> Boundary {
    match v {
        ComponentValue::Token(t) => boundary(&t.token),
        ComponentValue::Function(_) => Boundary::Function,
        ComponentValue::Block(b) => match b.kind {
            BlockKind::Paren => Boundary::Open,
            _ => Boundary::Other,
        },
    }
}

fn last_boundary(v: &ComponentValue) -> Boundary {
    match v {
        ComponentValue::Token(t) => boundary(&t.token),
        _ => Boundary::Other,
    }
}

enum Work<'a> {
    List(&'a [ComponentValue]),
    Text(&'static str),
}

/// Write component values as CSS text. Uses a work list, not recursion.
pub fn to_css(values: &[ComponentValue]) -> String {
    let mut out = String::new();
    let mut prev = Boundary::Other;
    let mut stack: Vec<Work<'_>> = vec![Work::List(values)];
    while let Some(w) = stack.pop() {
        match w {
            Work::Text(t) => {
                out.push_str(t);
                prev = Boundary::Other;
            }
            Work::List(list) => {
                let Some((first, rest)) = list.split_first() else {
                    continue;
                };
                if !rest.is_empty() {
                    stack.push(Work::List(rest));
                }
                if needs_comment(prev, first_boundary(first)) {
                    out.push_str("/**/");
                }
                match first {
                    ComponentValue::Token(t) => {
                        serialize_token(&t.token, &mut out);
                        prev = last_boundary(first);
                    }
                    ComponentValue::Function(f) => {
                        serialize_identifier(&f.name, &mut out);
                        out.push('(');
                        stack.push(Work::Text(")"));
                        stack.push(Work::List(&f.arguments));
                        prev = Boundary::Other;
                    }
                    ComponentValue::Block(b) => {
                        let (open, close) = match b.kind {
                            BlockKind::Paren => ("(", ")"),
                            BlockKind::Bracket => ("[", "]"),
                            BlockKind::Brace => ("{", "}"),
                        };
                        out.push_str(open);
                        stack.push(Work::Text(close));
                        stack.push(Work::List(&b.contents));
                        prev = Boundary::Other;
                    }
                }
            }
        }
    }
    out
}
