// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Tokenizer for Core JS source, shared by the stand-in host and the bundler.
//!
//! It reads the whole language's token grammar closely enough to name
//! anything outside Core JS precisely (a regex literal, a BigInt, a private
//! name), and reports real lexical errors as syntax errors. Positions are
//! byte offsets plus a 1-based line and a 1-based column counted in UTF-16
//! code units, the same convention as the Rust engine.
//!
//! As in the language grammar, two places depend on the parser: a `/` at the
//! start of an expression is a regex literal ([`Lexer::rescan_regex`]), and a
//! `}` that closes a template substitution continues the template
//! ([`Lexer::rescan_template`]).

use std::rc::Rc;

/// A problem found in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    /// Syntax error (the program is not JavaScript) or subset error (it is,
    /// but it uses something Core JS leaves out).
    pub kind: DiagKind,
    /// Plain description.
    pub message: String,
    /// Byte offset.
    pub start: usize,
    /// 1-based line.
    pub line: u32,
    /// 1-based column in UTF-16 code units.
    pub col: u32,
}

/// Which kind of problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagKind {
    /// Not valid JavaScript (strict mode).
    Syntax,
    /// Valid JavaScript, outside the Core JS subset.
    Subset,
    /// Inside Core JS, but later than the level the file is declared for.
    Level,
}

impl Diag {
    /// Error name used when reporting, such as `SyntaxError`.
    pub fn name(&self) -> &'static str {
        match self.kind {
            DiagKind::Syntax => "SyntaxError",
            DiagKind::Subset => "SubsetError",
            DiagKind::Level => "LevelError",
        }
    }
}

/// Token kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Identifier name, reserved words included.
    Ident(Rc<str>),
    /// `#name`
    PrivateName(Rc<str>),
    /// Punctuator text.
    Punct(&'static str),
    /// Numeric literal.
    Num(f64),
    /// BigInt literal (only reported as outside the subset).
    BigInt,
    /// String literal value in UTF-16 code units.
    Str(Rc<[u16]>),
    /// Template piece: cooked value, and whether it ends the template.
    Template(Rc<[u16]>, bool),
    /// Regex literal (only produced by a rescan).
    Regex,
    /// End of input.
    Eof,
}

/// One token.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// What it is.
    pub tok: Tok,
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset just past the last character.
    pub end: usize,
    /// 1-based line.
    pub line: u32,
    /// 1-based UTF-16 column.
    pub col: u32,
    /// A line terminator came before this token.
    pub nl_before: bool,
    /// An identifier written with a `\u` escape.
    pub escaped: bool,
}

/// Every punctuator, longest first within each leading character.
const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "<<", ">>", "**", "{", "}", "(", ")", "[", "]", ".", ";", ",", "<", ">", "+", "-", "*", "/",
    "%", "&", "|", "^", "!", "~", "?", ":", "=",
];

/// True for LF, CR, LS and PS.
pub fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// WhiteSpace from the spec: TAB, VT, FF, ZWNBSP, and the Zs category.
pub fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{000B}' | '\u{000C}' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Identifier start: exact for ASCII, `char::is_alphabetic` elsewhere
/// (the same approximation the Rust engine uses today).
pub fn is_id_start(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic() || c == '$' || c == '_'
    } else {
        c.is_alphabetic()
    }
}

/// Identifier continue: exact for ASCII, approximate elsewhere.
pub fn is_id_continue(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '$' || c == '_'
    } else {
        c.is_alphanumeric() || c == '\u{200C}' || c == '\u{200D}'
    }
}

/// Saved position.
#[derive(Clone, Copy, Debug)]
pub struct LexState {
    pos: usize,
    line: u32,
    col: u32,
}

/// The tokenizer.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: u32,
    col: u32,
}

type LResult<T> = Result<T, Diag>;

impl<'a> Lexer<'a> {
    /// A lexer over `src`.
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Current position.
    pub fn save(&self) -> LexState {
        LexState {
            pos: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    /// Back to a saved position.
    pub fn restore(&mut self, s: LexState) {
        self.pos = s.pos;
        self.line = s.line;
        self.col = s.col;
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        match c {
            '\n' | '\u{2028}' | '\u{2029}' => {
                self.line += 1;
                self.col = 1;
            }
            '\r' => {
                if self.peek() == Some('\n') {
                    self.col += 1;
                } else {
                    self.line += 1;
                    self.col = 1;
                }
            }
            _ => self.col += c.len_utf16() as u32,
        }
        Some(c)
    }

    /// A syntax error at the current position.
    pub fn err_here(&self, msg: impl Into<String>) -> Diag {
        Diag {
            kind: DiagKind::Syntax,
            message: msg.into(),
            start: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    fn err_at(&self, msg: impl Into<String>, s: LexState) -> Diag {
        Diag {
            kind: DiagKind::Syntax,
            message: msg.into(),
            start: s.pos,
            line: s.line,
            col: s.col,
        }
    }

    fn skip_trivia(&mut self) -> LResult<bool> {
        let mut nl = false;
        if self.pos == 0 && self.src.starts_with("#!") {
            self.skip_line();
        }
        while let Some(c) = self.peek() {
            if is_whitespace(c) {
                self.bump();
            } else if is_line_terminator(c) {
                self.bump();
                nl = true;
            } else if c == '/' && self.peek_at(1) == Some('/') {
                self.skip_line();
            } else if c == '/' && self.peek_at(1) == Some('*') {
                let s = self.save();
                self.bump();
                self.bump();
                loop {
                    match self.bump() {
                        None => return Err(self.err_at("unterminated comment", s)),
                        Some('*') if self.peek() == Some('/') => {
                            self.bump();
                            break;
                        }
                        Some(c) if is_line_terminator(c) => nl = true,
                        Some(_) => {}
                    }
                }
            } else {
                break;
            }
        }
        Ok(nl)
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            if is_line_terminator(c) {
                break;
            }
            self.bump();
        }
    }

    /// The next token.
    pub fn next_token(&mut self) -> LResult<Token> {
        let nl = self.skip_trivia()?;
        let s = self.save();
        let mut t = Token {
            tok: Tok::Eof,
            start: self.pos,
            end: self.pos,
            line: self.line,
            col: self.col,
            nl_before: nl,
            escaped: false,
        };
        let Some(c) = self.peek() else {
            return Ok(t);
        };
        t.tok = if is_id_start(c) || c == '\\' {
            let (name, esc) = self.read_ident()?;
            t.escaped = esc;
            Tok::Ident(name.into())
        } else if c == '#' {
            self.bump();
            match self.peek() {
                Some(d) if is_id_start(d) || d == '\\' => {
                    let (name, _) = self.read_ident()?;
                    Tok::PrivateName(name.into())
                }
                _ => return Err(self.err_at("`#` must start a private name", s)),
            }
        } else if c.is_ascii_digit()
            || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()))
        {
            self.read_number(s)?
        } else if c == '"' || c == '\'' {
            Tok::Str(self.read_string(c, s)?.into())
        } else if c == '`' {
            self.bump();
            let (cooked, tail) = self.read_template_chars(s)?;
            Tok::Template(cooked.into(), tail)
        } else {
            let rest = &self.src[self.pos..];
            let Some(p) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
                return Err(self.err_at(format!("unexpected character `{c}`"), s));
            };
            // `?.` followed by a digit is `?` then a number.
            let p: &'static str =
                if *p == "?." && rest[2..].starts_with(|d: char| d.is_ascii_digit()) {
                    "?"
                } else {
                    p
                };
            for _ in 0..p.len() {
                self.bump();
            }
            Tok::Punct(p)
        };
        t.end = self.pos;
        Ok(t)
    }

    fn read_ident(&mut self) -> LResult<(String, bool)> {
        let mut out = String::new();
        let mut esc = false;
        let mut first = true;
        loop {
            let s = self.save();
            match self.peek() {
                Some('\\') => {
                    self.bump();
                    if self.peek() != Some('u') {
                        return Err(self.err_at("bad escape in identifier", s));
                    }
                    self.bump();
                    let cp = self.read_unicode_escape_body(s)?;
                    let ch = char::from_u32(cp)
                        .ok_or_else(|| self.err_at("bad escape in identifier", s))?;
                    let ok = if first {
                        is_id_start(ch)
                    } else {
                        is_id_continue(ch)
                    };
                    if !ok {
                        return Err(self.err_at("escaped character is not allowed in a name", s));
                    }
                    out.push(ch);
                    esc = true;
                }
                Some(c) if (first && is_id_start(c)) || (!first && is_id_continue(c)) => {
                    self.bump();
                    out.push(c);
                }
                _ => break,
            }
            first = false;
        }
        Ok((out, esc))
    }

    /// After `\u`: either `{hex}` or four hex digits.
    fn read_unicode_escape_body(&mut self, s: LexState) -> LResult<u32> {
        if self.peek() == Some('{') {
            self.bump();
            let mut v: u32 = 0;
            let mut n = 0;
            while let Some(c) = self.peek() {
                if c == '}' {
                    break;
                }
                let d = c
                    .to_digit(16)
                    .ok_or_else(|| self.err_at("bad unicode escape", s))?;
                self.bump();
                v = v.saturating_mul(16).saturating_add(d);
                n += 1;
                if v > 0x10FFFF {
                    return Err(self.err_at("unicode escape is above 0x10FFFF", s));
                }
            }
            if n == 0 || self.peek() != Some('}') {
                return Err(self.err_at("bad unicode escape", s));
            }
            self.bump();
            Ok(v)
        } else {
            let mut v = 0;
            for _ in 0..4 {
                let d = self
                    .peek()
                    .and_then(|c| c.to_digit(16))
                    .ok_or_else(|| self.err_at("bad unicode escape", s))?;
                self.bump();
                v = v * 16 + d;
            }
            Ok(v)
        }
    }

    fn read_digits(&mut self, radix: u32, s: LexState, out: &mut String) -> LResult<()> {
        let mut last_sep = true;
        let mut any = false;
        while let Some(c) = self.peek() {
            if c == '_' {
                if last_sep {
                    return Err(self.err_at("misplaced numeric separator", s));
                }
                self.bump();
                last_sep = true;
                continue;
            }
            if !c.is_digit(radix) {
                break;
            }
            self.bump();
            out.push(c);
            last_sep = false;
            any = true;
        }
        if any && last_sep {
            return Err(self.err_at("misplaced numeric separator", s));
        }
        Ok(())
    }

    fn read_number(&mut self, s: LexState) -> LResult<Tok> {
        let c = self.peek().unwrap_or('0');
        let radix = if c == '0' {
            match self.peek_at(1) {
                Some('x' | 'X') => 16,
                Some('o' | 'O') => 8,
                Some('b' | 'B') => 2,
                _ => 10,
            }
        } else {
            10
        };
        let tok = if radix != 10 {
            self.bump();
            self.bump();
            let mut digits = String::new();
            self.read_digits(radix, s, &mut digits)?;
            if digits.is_empty() {
                return Err(self.err_at("missing digits after the number prefix", s));
            }
            if self.peek() == Some('n') {
                self.bump();
                Tok::BigInt
            } else {
                let mut v = 0f64;
                for d in digits.chars() {
                    v = v * radix as f64 + d.to_digit(radix).unwrap_or(0) as f64;
                }
                // Exact for every value up to 2^53; larger values are
                // rounded once per digit, which Core JS never needs.
                Tok::Num(v)
            }
        } else {
            let mut text = String::new();
            if c == '0'
                && self
                    .peek_at(1)
                    .is_some_and(|d| d.is_ascii_digit() || d == '_')
            {
                return Err(self.err_at(
                    "legacy octal and leading-zero numbers are not allowed in strict code",
                    s,
                ));
            }
            self.read_digits(10, s, &mut text)?;
            let mut int_only = true;
            if self.peek() == Some('.') {
                int_only = false;
                self.bump();
                text.push('.');
                if self.peek() == Some('_') {
                    return Err(self.err_at("misplaced numeric separator", s));
                }
                self.read_digits(10, s, &mut text)?;
            }
            if matches!(self.peek(), Some('e' | 'E')) {
                int_only = false;
                self.bump();
                text.push('e');
                if let Some(sg @ ('+' | '-')) = self.peek() {
                    self.bump();
                    text.push(sg);
                }
                let before = text.len();
                self.read_digits(10, s, &mut text)?;
                if text.len() == before {
                    return Err(self.err_at("missing exponent digits", s));
                }
            }
            if int_only && self.peek() == Some('n') {
                self.bump();
                Tok::BigInt
            } else {
                if text.starts_with('.') {
                    text.insert(0, '0');
                }
                if text.ends_with('.') {
                    text.push('0');
                }
                let v: f64 = text.parse().map_err(|_| self.err_at("bad number", s))?;
                Tok::Num(v)
            }
        };
        if self
            .peek()
            .is_some_and(|c| is_id_start(c) || c.is_ascii_digit() || c == '\\')
        {
            return Err(self.err_at("a number can't be followed directly by a name or digit", s));
        }
        Ok(tok)
    }

    fn push_cp(out: &mut Vec<u16>, cp: u32) {
        if cp >= 0x10000 {
            let v = cp - 0x10000;
            out.push(0xD800 + (v >> 10) as u16);
            out.push(0xDC00 + (v & 0x3FF) as u16);
        } else {
            out.push(cp as u16);
        }
    }

    /// Reads one escape after the backslash. Returns false for a line
    /// continuation (nothing pushed).
    fn read_escape(&mut self, out: &mut Vec<u16>, template: bool) -> LResult<bool> {
        let s = self.save();
        let Some(c) = self.bump() else {
            return Err(self.err_at("unterminated string", s));
        };
        match c {
            'n' => out.push(10),
            't' => out.push(9),
            'r' => out.push(13),
            'b' => out.push(8),
            'f' => out.push(12),
            'v' => out.push(11),
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => out.push(0),
            '1'..='9' | '0' => {
                let what = if template { "templates" } else { "strict code" };
                return Err(self.err_at(
                    format!("octal and \\8 \\9 escapes are not allowed in {what}"),
                    s,
                ));
            }
            'x' => {
                let mut v = 0;
                for _ in 0..2 {
                    let d = self
                        .peek()
                        .and_then(|c| c.to_digit(16))
                        .ok_or_else(|| self.err_at("bad \\x escape", s))?;
                    self.bump();
                    v = v * 16 + d;
                }
                out.push(v as u16);
            }
            'u' => {
                let cp = self.read_unicode_escape_body(s)?;
                Self::push_cp(out, cp);
            }
            '\r' => {
                if self.peek() == Some('\n') {
                    self.bump();
                }
                return Ok(false);
            }
            '\n' | '\u{2028}' | '\u{2029}' => return Ok(false),
            other => {
                let mut buf = [0u16; 2];
                out.extend_from_slice(other.encode_utf16(&mut buf));
            }
        }
        Ok(true)
    }

    fn read_string(&mut self, quote: char, s: LexState) -> LResult<Vec<u16>> {
        self.bump();
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some('\n' | '\r') => return Err(self.err_at("unterminated string", s)),
                Some(c) if c == quote => {
                    self.bump();
                    return Ok(out);
                }
                Some('\\') => {
                    self.bump();
                    self.read_escape(&mut out, false)?;
                }
                Some(c) => {
                    self.bump();
                    let mut buf = [0u16; 2];
                    out.extend_from_slice(c.encode_utf16(&mut buf));
                }
            }
        }
    }

    /// Template characters up to `${` (not tail) or the closing backtick
    /// (tail). The opening backtick or `}` has been consumed.
    fn read_template_chars(&mut self, s: LexState) -> LResult<(Vec<u16>, bool)> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.err_at("unterminated template", s)),
                Some('`') => {
                    self.bump();
                    return Ok((out, true));
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    self.bump();
                    self.bump();
                    return Ok((out, false));
                }
                Some('\\') => {
                    self.bump();
                    self.read_escape(&mut out, true)?;
                }
                Some('\r') => {
                    self.bump();
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                    out.push(10);
                }
                Some(c) => {
                    self.bump();
                    let mut buf = [0u16; 2];
                    out.extend_from_slice(c.encode_utf16(&mut buf));
                }
            }
        }
    }

    /// Read a `/` or `/=` token again as a regex literal. Only used to skip
    /// over it with a precise error.
    pub fn rescan_regex(&mut self, t: &Token) -> LResult<Token> {
        self.pos = t.start;
        self.line = t.line;
        self.col = t.col;
        let s = self.save();
        self.bump();
        let mut class = false;
        loop {
            match self.peek() {
                None => return Err(self.err_at("unterminated regular expression", s)),
                Some(c) if is_line_terminator(c) => {
                    return Err(self.err_at("unterminated regular expression", s));
                }
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        Some(c) if !is_line_terminator(c) => {
                            self.bump();
                        }
                        _ => return Err(self.err_at("unterminated regular expression", s)),
                    }
                }
                Some('[') => {
                    class = true;
                    self.bump();
                }
                Some(']') => {
                    class = false;
                    self.bump();
                }
                Some('/') if !class => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
        while self.peek().is_some_and(is_id_continue) {
            self.bump();
        }
        Ok(Token {
            tok: Tok::Regex,
            end: self.pos,
            ..t.clone()
        })
    }

    /// Read a `}` token again as the next template piece.
    pub fn rescan_template(&mut self, t: &Token) -> LResult<Token> {
        self.pos = t.start;
        self.line = t.line;
        self.col = t.col;
        let s = self.save();
        self.bump();
        let (cooked, tail) = self.read_template_chars(s)?;
        Ok(Token {
            tok: Tok::Template(cooked.into(), tail),
            end: self.pos,
            ..t.clone()
        })
    }
}

#[cfg(test)]
mod lexer_tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let mut lx = Lexer::new(src);
        let mut out = Vec::new();
        loop {
            let t = lx.next_token().unwrap();
            if t.tok == Tok::Eof {
                return out;
            }
            out.push(t.tok);
        }
    }

    #[test]
    fn punctuators_longest_first() {
        assert_eq!(
            toks("a>>>=b ?.c ?.5"),
            vec![
                Tok::Ident("a".into()),
                Tok::Punct(">>>="),
                Tok::Ident("b".into()),
                Tok::Punct("?."),
                Tok::Ident("c".into()),
                Tok::Punct("?"),
                Tok::Num(0.5),
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            toks("0x1F 0b101 0o17 1_000 1.5e3 .25"),
            vec![
                Tok::Num(31.0),
                Tok::Num(5.0),
                Tok::Num(15.0),
                Tok::Num(1000.0),
                Tok::Num(1500.0),
                Tok::Num(0.25)
            ]
        );
        assert!(Lexer::new("017").next_token().is_err());
        assert!(Lexer::new("1__0").next_token().is_err());
        assert!(Lexer::new("3in").next_token().is_err());
        assert_eq!(toks("10n"), vec![Tok::BigInt]);
    }

    #[test]
    fn strings_and_columns() {
        let mut lx = Lexer::new("'\\u{1F600}\\x41\\\n' \u{e9}x");
        let t = lx.next_token().unwrap();
        assert_eq!(t.tok, Tok::Str(vec![0xD83D, 0xDE00, 0x41].into()));
        let t = lx.next_token().unwrap();
        assert_eq!((t.line, t.col), (2, 3));
        assert!(Lexer::new("'\\08'").next_token().is_err());
        assert!(Lexer::new("'abc").next_token().is_err());
    }

    #[test]
    fn line_terminators_and_comments() {
        let mut lx = Lexer::new("a\r\nb\u{2028}c /* x\n */ d");
        let a = lx.next_token().unwrap();
        let b = lx.next_token().unwrap();
        let c = lx.next_token().unwrap();
        let d = lx.next_token().unwrap();
        assert_eq!((a.line, b.line, c.line, d.line), (1, 2, 3, 4));
        assert!(b.nl_before && c.nl_before && d.nl_before);
        assert_eq!(d.col, 5);
    }

    #[test]
    fn templates() {
        let mut lx = Lexer::new("`a${x}b\r\nc`");
        let t = lx.next_token().unwrap();
        assert_eq!(t.tok, Tok::Template(vec![97].into(), false));
        lx.next_token().unwrap();
        let close = lx.next_token().unwrap();
        let t = lx.rescan_template(&close).unwrap();
        assert_eq!(t.tok, Tok::Template(vec![98, 10, 99].into(), true));
    }
}
