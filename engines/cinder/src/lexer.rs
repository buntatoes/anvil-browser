// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! ECMAScript lexer.
//!
//! The lexer turns source text into [`Token`]s, one at a time. It never
//! panics: anything it can't read comes back as a [`SyntaxError`] with a
//! span.
//!
//! Two spots in the grammar depend on where the parser is, so the parser
//! drives them:
//!
//! - A `/` or `/=` token at the start of an expression is a regular
//!   expression literal. The parser calls [`Lexer::rescan_regex`] with that
//!   token, and the lexer reads it again as a regex.
//! - A `}` that closes a template substitution continues the template. The
//!   parser calls [`Lexer::rescan_template_continuation`] with that token.
//!
//! Every token carries `newline_before`, which the parser uses for automatic
//! semicolon insertion and the "no line break here" rules.
//!
//! Identifier characters: ASCII is exact. Outside ASCII, `char::is_alphabetic`
//! stands in for ID_Start and `char::is_alphanumeric` (plus ZWNJ and ZWJ) for
//! ID_Continue. That's close but not exact. Exact tables will come later
//! from an in-house generator that reads the Unicode data files.

use crate::cinder::error::{Span, SyntaxError};

/// Which grammar goal the source is read with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    /// A classic script. HTML-like comments are allowed.
    Script,
    /// A module. Always strict, and HTML-like comments are not allowed.
    Module,
}

/// Every punctuator in the language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Ellipsis,
    Semicolon,
    Comma,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Plus,
    Minus,
    Star,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    Shl,
    Shr,
    UShr,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    QuestionQuestion,
    Question,
    QuestionDot,
    Colon,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    PercentEq,
    StarStarEq,
    ShlEq,
    ShrEq,
    UShrEq,
    AmpEq,
    PipeEq,
    CaretEq,
    AmpAmpEq,
    PipePipeEq,
    QuestionQuestionEq,
    Arrow,
    Slash,
    SlashEq,
}

impl Punct {
    /// Source text of the punctuator.
    pub fn as_str(self) -> &'static str {
        use Punct::*;
        match self {
            LBrace => "{",
            RBrace => "}",
            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            Dot => ".",
            Ellipsis => "...",
            Semicolon => ";",
            Comma => ",",
            Lt => "<",
            Gt => ">",
            LtEq => "<=",
            GtEq => ">=",
            EqEq => "==",
            NotEq => "!=",
            EqEqEq => "===",
            NotEqEq => "!==",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Percent => "%",
            StarStar => "**",
            PlusPlus => "++",
            MinusMinus => "--",
            Shl => "<<",
            Shr => ">>",
            UShr => ">>>",
            Amp => "&",
            Pipe => "|",
            Caret => "^",
            Bang => "!",
            Tilde => "~",
            AmpAmp => "&&",
            PipePipe => "||",
            QuestionQuestion => "??",
            Question => "?",
            QuestionDot => "?.",
            Colon => ":",
            Eq => "=",
            PlusEq => "+=",
            MinusEq => "-=",
            StarEq => "*=",
            PercentEq => "%=",
            StarStarEq => "**=",
            ShlEq => "<<=",
            ShrEq => ">>=",
            UShrEq => ">>>=",
            AmpEq => "&=",
            PipeEq => "|=",
            CaretEq => "^=",
            AmpAmpEq => "&&=",
            PipePipeEq => "||=",
            QuestionQuestionEq => "??=",
            Arrow => "=>",
            Slash => "/",
            SlashEq => "/=",
        }
    }
}

/// A BigInt literal, kept as digits so no precision is lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigIntLit {
    /// 2, 8, 10, or 16.
    pub radix: u32,
    /// Digits in that radix, lowercase, without prefix, separators, or `n`.
    pub digits: String,
}

/// One piece of a template literal.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplatePart {
    /// The cooked value, or `None` when the piece has an escape that is only
    /// allowed in tagged templates.
    pub cooked: Option<Vec<u16>>,
    /// Where that bad escape is, when `cooked` is `None`.
    pub bad_escape: Option<Span>,
    /// The raw text, with CR and CRLF turned into LF.
    pub raw: String,
    /// True when this piece ends the template (closing backtick).
    pub tail: bool,
}

/// What a token is.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// An identifier name. Reserved words come through here too; the parser
    /// decides what they mean. Unicode escapes are already decoded.
    Ident(String),
    /// `#name`, without the `#`.
    PrivateName(String),
    /// A punctuator.
    Punct(Punct),
    /// A numeric literal.
    Number(f64),
    /// A BigInt literal.
    BigInt(BigIntLit),
    /// A string literal's value, in UTF-16 code units.
    String(Vec<u16>),
    /// A template piece. A piece read after a backtick is a head (or a whole
    /// template when `tail` is set); one read after `}` is a middle or tail.
    Template(TemplatePart),
    /// A regular expression literal.
    Regex {
        /// Text between the slashes, unchanged.
        pattern: String,
        /// Flags after the closing slash.
        flags: String,
    },
    /// End of input.
    Eof,
}

/// One token with its position and flags.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// Where it is.
    pub span: Span,
    /// A line terminator came between the previous token and this one.
    pub newline_before: bool,
    /// An identifier written with at least one `\u` escape. Such a name never
    /// acts as a keyword.
    pub escaped: bool,
    /// A legacy octal number (`017`), a decimal with a leading zero (`08`),
    /// or a string with an octal or `\8` `\9` escape. Strict code rejects it.
    pub legacy_octal: bool,
}

impl Token {
    /// True for an identifier token spelled exactly `word` with no escapes.
    pub fn is_word(&self, word: &str) -> bool {
        !self.escaped && matches!(&self.kind, TokenKind::Ident(n) if n == word)
    }

    /// True for the punctuator `p`.
    pub fn is_punct(&self, p: Punct) -> bool {
        self.kind == TokenKind::Punct(p)
    }
}

/// Words that can never be identifiers.
pub const RESERVED_WORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Words that are reserved only in strict code.
pub const STRICT_RESERVED_WORDS: &[&str] = &[
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
];

/// True for a word that is reserved in all code. `await` and `yield` are
/// left out here because they depend on context; the parser checks them.
pub fn is_always_reserved(word: &str) -> bool {
    word != "await" && word != "yield" && RESERVED_WORDS.contains(&word)
}

/// True for a word reserved in strict code only (including `yield`).
pub fn is_strict_reserved(word: &str) -> bool {
    STRICT_RESERVED_WORDS.contains(&word)
}

/// Approximate ID_Start. Exact for ASCII.
pub fn is_id_start(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic() || c == '$' || c == '_'
    } else {
        c.is_alphabetic()
    }
}

/// Approximate ID_Continue. Exact for ASCII.
pub fn is_id_continue(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '$' || c == '_'
    } else {
        c.is_alphanumeric() || c == '\u{200C}' || c == '\u{200D}'
    }
}

/// LF, CR, LS, PS.
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

/// Saved lexer position, used by the parser to look ahead and back up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LexState {
    pos: usize,
    line: u32,
    col: u32,
}

/// The lexer.
#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: u32,
    col: u32,
    goal: Goal,
}

type LexResult<T> = Result<T, SyntaxError>;

enum EscapeKind {
    /// Code units were pushed.
    Normal,
    /// Backslash plus line terminator: nothing is pushed.
    LineContinuation,
    /// A legacy octal or `\8` `\9` escape (strings only).
    LegacyOctal,
    /// An escape only allowed in tagged templates.
    Invalid(Span),
}

impl<'a> Lexer<'a> {
    /// A lexer over `src`.
    pub fn new(src: &'a str, goal: Goal) -> Self {
        Lexer {
            src,
            pos: 0,
            line: 1,
            col: 1,
            goal,
        }
    }

    /// The source text.
    pub fn source(&self) -> &'a str {
        self.src
    }

    /// Current position, for [`Lexer::restore`].
    pub fn save(&self) -> LexState {
        LexState {
            pos: self.pos,
            line: self.line,
            col: self.col,
        }
    }

    /// Go back to a saved position.
    pub fn restore(&mut self, state: LexState) {
        self.pos = state.pos;
        self.line = state.line;
        self.col = state.col;
    }

    fn reset_to(&mut self, span: Span) {
        self.pos = span.start;
        self.line = span.line;
        self.col = span.column;
    }

    fn peek(&self) -> Option<char> {
        let b = *self.src.as_bytes().get(self.pos)?;
        if b < 0x80 {
            Some(b as char)
        } else {
            self.src[self.pos..].chars().next()
        }
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
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
                    // The LF that follows ends the line.
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

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn here(&self) -> Span {
        Span {
            start: self.pos,
            end: self.pos,
            line: self.line,
            column: self.col,
        }
    }

    fn span_from(&self, start: Span) -> Span {
        Span {
            end: self.pos,
            ..start
        }
    }

    fn err<T>(&self, msg: impl Into<String>, start: Span) -> LexResult<T> {
        Err(SyntaxError::new(msg, self.span_from(start)))
    }

    /// Skip whitespace and comments. Returns whether a line terminator was
    /// crossed.
    fn skip_trivia(&mut self) -> LexResult<bool> {
        let mut newline = false;
        if self.pos == 0 && self.starts_with("#!") {
            self.skip_line();
        }
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                let bytes = self.src.as_bytes();
                let mut scan = self.pos + 1;
                while scan < bytes.len() {
                    // SAFETY: `scan < bytes.len()` is checked by the loop condition.
                    let b = unsafe { *bytes.get_unchecked(scan) };
                    if b == b' ' || b == b'\t' {
                        scan += 1;
                    } else {
                        break;
                    }
                }
                let delta = (scan - self.pos) as u32;
                self.pos = scan;
                self.col += delta;
            } else if is_whitespace(c) {
                self.bump();
            } else if is_line_terminator(c) {
                self.bump();
                newline = true;
            } else if c == '/' {
                match self.peek_at(1) {
                    Some('/') => self.skip_line(),
                    Some('*') => {
                        let start = self.here();
                        self.bump();
                        self.bump();
                        loop {
                            match self.bump() {
                                None => return self.err("unterminated comment", start),
                                Some('*') if self.peek() == Some('/') => {
                                    self.bump();
                                    break;
                                }
                                Some(c) if is_line_terminator(c) => newline = true,
                                Some(_) => {}
                            }
                        }
                    }
                    _ => break,
                }
            } else if self.goal == Goal::Script
                && ((c == '<' && self.starts_with("<!--"))
                    || (c == '-' && (newline || self.pos == 0) && self.starts_with("-->")))
            {
                self.skip_line();
            } else {
                break;
            }
        }
        Ok(newline)
    }

    fn skip_line(&mut self) {
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            // SAFETY: `self.pos < bytes.len()` is checked by the loop condition.
            let b = unsafe { *bytes.get_unchecked(self.pos) };
            if b < 0x80 && b != b'\n' && b != b'\r' {
                self.pos += 1;
                self.col += 1;
            } else {
                break;
            }
        }
        while let Some(c) = self.peek() {
            if is_line_terminator(c) {
                break;
            }
            self.bump();
        }
    }

    /// Read the next token.
    pub fn next_token(&mut self) -> LexResult<Token> {
        let newline_before = self.skip_trivia()?;
        let start = self.here();
        let mut tok = Token {
            kind: TokenKind::Eof,
            span: start,
            newline_before,
            escaped: false,
            legacy_octal: false,
        };
        let Some(c) = self.peek() else {
            return Ok(tok);
        };
        tok.kind = if is_id_start(c) || c == '\\' {
            let (name, escaped) = self.read_ident_name(start)?;
            tok.escaped = escaped;
            TokenKind::Ident(name)
        } else if c.is_ascii_digit()
            || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()))
        {
            let (kind, legacy) = self.read_number(start)?;
            tok.legacy_octal = legacy;
            kind
        } else if c == '"' || c == '\'' {
            let (value, legacy) = self.read_string(c, start)?;
            tok.legacy_octal = legacy;
            TokenKind::String(value)
        } else if c == '`' {
            self.bump();
            TokenKind::Template(self.read_template_chars(start)?)
        } else if c == '#' {
            self.bump();
            match self.peek() {
                Some(n) if is_id_start(n) || n == '\\' => {
                    let (name, escaped) = self.read_ident_name(self.here())?;
                    tok.escaped = escaped;
                    TokenKind::PrivateName(name)
                }
                _ => return self.err("`#` must be followed by a name", start),
            }
        } else {
            TokenKind::Punct(self.read_punct(start)?)
        };
        tok.span = self.span_from(start);
        Ok(tok)
    }

    fn read_punct(&mut self, start: Span) -> LexResult<Punct> {
        use Punct::*;
        let c = self.bump().unwrap_or('\0');
        let p = match c {
            '{' => LBrace,
            '}' => RBrace,
            '(' => LParen,
            ')' => RParen,
            '[' => LBracket,
            ']' => RBracket,
            ';' => Semicolon,
            ',' => Comma,
            ':' => Colon,
            '~' => Tilde,
            '.' => {
                if self.starts_with("..") {
                    self.bump();
                    self.bump();
                    Ellipsis
                } else {
                    Dot
                }
            }
            '<' => {
                if self.eat('<') {
                    if self.eat('=') { ShlEq } else { Shl }
                } else if self.eat('=') {
                    LtEq
                } else {
                    Lt
                }
            }
            '>' => {
                if self.eat('>') {
                    if self.eat('>') {
                        if self.eat('=') { UShrEq } else { UShr }
                    } else if self.eat('=') {
                        ShrEq
                    } else {
                        Shr
                    }
                } else if self.eat('=') {
                    GtEq
                } else {
                    Gt
                }
            }
            '=' => {
                if self.eat('=') {
                    if self.eat('=') { EqEqEq } else { EqEq }
                } else if self.eat('>') {
                    Arrow
                } else {
                    Eq
                }
            }
            '!' => {
                if self.eat('=') {
                    if self.eat('=') { NotEqEq } else { NotEq }
                } else {
                    Bang
                }
            }
            '+' => {
                if self.eat('+') {
                    PlusPlus
                } else if self.eat('=') {
                    PlusEq
                } else {
                    Plus
                }
            }
            '-' => {
                if self.eat('-') {
                    MinusMinus
                } else if self.eat('=') {
                    MinusEq
                } else {
                    Minus
                }
            }
            '*' => {
                if self.eat('*') {
                    if self.eat('=') { StarStarEq } else { StarStar }
                } else if self.eat('=') {
                    StarEq
                } else {
                    Star
                }
            }
            '%' => {
                if self.eat('=') {
                    PercentEq
                } else {
                    Percent
                }
            }
            '&' => {
                if self.eat('&') {
                    if self.eat('=') { AmpAmpEq } else { AmpAmp }
                } else if self.eat('=') {
                    AmpEq
                } else {
                    Amp
                }
            }
            '|' => {
                if self.eat('|') {
                    if self.eat('=') { PipePipeEq } else { PipePipe }
                } else if self.eat('=') {
                    PipeEq
                } else {
                    Pipe
                }
            }
            '^' => {
                if self.eat('=') {
                    CaretEq
                } else {
                    Caret
                }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.bump();
                    if self.eat('=') {
                        QuestionQuestionEq
                    } else {
                        QuestionQuestion
                    }
                } else if self.peek() == Some('.')
                    && !self.peek_at(1).is_some_and(|d| d.is_ascii_digit())
                {
                    self.bump();
                    QuestionDot
                } else {
                    Question
                }
            }
            '/' => {
                if self.eat('=') {
                    SlashEq
                } else {
                    Slash
                }
            }
            other => {
                let what = if other == '\0' {
                    "unexpected NUL character".to_string()
                } else {
                    format!("unexpected character U+{:04X}", other as u32)
                };
                return self.err(what, start);
            }
        };
        Ok(p)
    }

    /// Read an IdentifierName. Returns the decoded name and whether any
    /// escape was used.
    fn read_ident_name(&mut self, start: Span) -> LexResult<(String, bool)> {
        let bytes = self.src.as_bytes();
        if self.pos < bytes.len() {
            // SAFETY: `self.pos < bytes.len()` is verified above.
            let b0 = unsafe { *bytes.get_unchecked(self.pos) };
            if b0.is_ascii_alphabetic() || b0 == b'$' || b0 == b'_' {
                let run_start = self.pos;
                let mut scan = self.pos + 1;
                while scan < bytes.len() {
                    // SAFETY: `scan < bytes.len()` is checked by the loop condition.
                    let b = unsafe { *bytes.get_unchecked(scan) };
                    if b.is_ascii_alphanumeric() || b == b'$' || b == b'_' {
                        scan += 1;
                    } else {
                        break;
                    }
                }
                let stop_is_non_ident_ascii = if scan >= bytes.len() {
                    true
                } else {
                    // SAFETY: `scan < bytes.len()` holds in the `else` branch.
                    let next_b = unsafe { *bytes.get_unchecked(scan) };
                    next_b < 0x80 && next_b != b'\\'
                };
                if stop_is_non_ident_ascii {
                    let delta = (scan - run_start) as u32;
                    self.pos = scan;
                    self.col += delta;
                    // SAFETY: Every byte in `run_start..scan` was validated as ASCII.
                    let slice = unsafe {
                        std::str::from_utf8_unchecked(bytes.get_unchecked(run_start..scan))
                    };
                    return Ok((slice.to_string(), false));
                }
            }
        }
        let mut name = String::new();
        let mut escaped = false;
        let mut first = true;
        while let Some(c) = self.peek() {
            let ch = if c == '\\' {
                let esc_start = self.here();
                self.bump();
                if !self.eat('u') {
                    return self.err("expected `u` after `\\` in a name", esc_start);
                }
                escaped = true;
                let cp = self.read_unicode_escape_body(esc_start)?;
                match char::from_u32(cp) {
                    Some(ch) => {
                        let ok = if first {
                            is_id_start(ch)
                        } else {
                            is_id_continue(ch)
                        };
                        if !ok {
                            return self
                                .err("escaped character is not allowed in a name", esc_start);
                        }
                        ch
                    }
                    None => {
                        return self.err("escaped surrogate is not allowed in a name", esc_start);
                    }
                }
            } else {
                let ok = if first {
                    is_id_start(c)
                } else {
                    is_id_continue(c)
                };
                if !ok {
                    break;
                }
                self.bump();
                c
            };
            name.push(ch);
            first = false;
        }
        if name.is_empty() {
            return self.err("expected a name", start);
        }
        Ok((name, escaped))
    }

    /// After `\u`: four hex digits or `{hex}`. Returns the code point.
    fn read_unicode_escape_body(&mut self, esc_start: Span) -> LexResult<u32> {
        if self.eat('{') {
            let mut value: u32 = 0;
            let mut digits = 0;
            loop {
                match self.peek() {
                    Some('}') => {
                        self.bump();
                        break;
                    }
                    Some(c) if c.is_ascii_hexdigit() => {
                        self.bump();
                        value = value
                            .saturating_mul(16)
                            .saturating_add(c.to_digit(16).unwrap_or(0));
                        digits += 1;
                        if value > 0x10FFFF {
                            return self.err("code point is past U+10FFFF", esc_start);
                        }
                    }
                    _ => return self.err("malformed `\\u{...}` escape", esc_start),
                }
            }
            if digits == 0 {
                return self.err("empty `\\u{}` escape", esc_start);
            }
            Ok(value)
        } else {
            let mut value = 0;
            for _ in 0..4 {
                match self.peek() {
                    Some(c) if c.is_ascii_hexdigit() => {
                        self.bump();
                        value = value * 16 + c.to_digit(16).unwrap_or(0);
                    }
                    _ => return self.err("`\\u` needs four hex digits", esc_start),
                }
            }
            Ok(value)
        }
    }

    /// Digits in `radix` with optional `_` separators between digits.
    fn read_digits(&mut self, radix: u32, allow_sep: bool, start: Span) -> LexResult<String> {
        if radix == 10 {
            let bytes = self.src.as_bytes();
            let run_start = self.pos;
            let mut scan = self.pos;
            while scan < bytes.len() {
                // SAFETY: `scan < bytes.len()` is checked by the loop condition.
                let b = unsafe { *bytes.get_unchecked(scan) };
                if b.is_ascii_digit() {
                    scan += 1;
                } else {
                    break;
                }
            }
            let next_is_underscore =
                scan < bytes.len() && unsafe { *bytes.get_unchecked(scan) } == b'_';
            if scan > run_start && !next_is_underscore {
                let delta = (scan - run_start) as u32;
                self.pos = scan;
                self.col += delta;
                // SAFETY: Every byte in `run_start..scan` is an ASCII decimal digit.
                let slice =
                    unsafe { std::str::from_utf8_unchecked(bytes.get_unchecked(run_start..scan)) };
                return Ok(slice.to_string());
            }
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if c.is_digit(radix) => {
                    self.bump();
                    out.push(c.to_ascii_lowercase());
                }
                Some('_') if allow_sep => {
                    let next_ok = self.peek_at(1).is_some_and(|d| d.is_digit(radix));
                    if out.is_empty() || !next_ok {
                        self.bump();
                        return self.err("`_` must sit between two digits", start);
                    }
                    self.bump();
                }
                _ => break,
            }
        }
        Ok(out)
    }

    fn read_number(&mut self, start: Span) -> LexResult<(TokenKind, bool)> {
        let mut legacy = false;
        let first = self.peek().unwrap_or('0');
        let second = self.peek_at(1);
        let kind = if first == '0' && matches!(second, Some('x' | 'X' | 'o' | 'O' | 'b' | 'B')) {
            self.bump();
            let radix = match self.bump() {
                Some('x' | 'X') => 16,
                Some('o' | 'O') => 8,
                _ => 2,
            };
            let digits = self.read_digits(radix, true, start)?;
            if digits.is_empty() {
                return self.err("missing digits after the radix prefix", start);
            }
            if self.eat('n') {
                TokenKind::BigInt(BigIntLit {
                    radix,
                    digits: trim_zeros(&digits),
                })
            } else {
                TokenKind::Number(radix_to_f64(&digits, radix))
            }
        } else if first == '0' && second.is_some_and(|d| d.is_ascii_digit()) {
            // Legacy octal (017) or a decimal with a leading zero (089).
            legacy = true;
            self.bump();
            let digits = self.read_digits(10, false, start)?;
            if self.peek() == Some('_') {
                return self.err("`_` is not allowed in a legacy octal literal", start);
            }
            if self.peek() == Some('n') {
                return self.err("BigInt literals can't start with 0", start);
            }
            if digits.bytes().all(|b| b < b'8') {
                TokenKind::Number(radix_to_f64(&digits, 8))
            } else {
                let mut text = digits;
                self.read_decimal_tail(&mut text, start)?;
                TokenKind::Number(parse_decimal(&text))
            }
        } else {
            let mut text = String::new();
            let mut is_int = true;
            if first == '0' {
                self.bump();
                text.push('0');
                if self.peek() == Some('_') {
                    return self.err("`_` can't follow a leading 0", start);
                }
            } else if first != '.' {
                text = self.read_digits(10, true, start)?;
            }
            if self.peek() == Some('.') || self.peek().is_some_and(|c| c == 'e' || c == 'E') {
                is_int = false;
            }
            self.read_decimal_tail(&mut text, start)?;
            if self.peek() == Some('n') {
                if !is_int {
                    return self.err("a BigInt literal must be an integer", start);
                }
                self.bump();
                TokenKind::BigInt(BigIntLit {
                    radix: 10,
                    digits: trim_zeros(&text),
                })
            } else {
                TokenKind::Number(parse_decimal(&text))
            }
        };
        if let Some(c) = self.peek()
            && (is_id_start(c) || c.is_ascii_digit() || c == '\\')
        {
            return self.err(
                "a number can't be followed directly by a name or digit",
                start,
            );
        }
        Ok((kind, legacy))
    }

    /// Optional `.digits` and exponent after the integer part.
    fn read_decimal_tail(&mut self, text: &mut String, start: Span) -> LexResult<()> {
        if self.eat('.') {
            text.push('.');
            if self.peek() == Some('_') {
                return self.err("`_` can't follow the decimal point", start);
            }
            text.push_str(&self.read_digits(10, true, start)?);
        }
        if let Some(e @ ('e' | 'E')) = self.peek() {
            self.bump();
            text.push(e);
            if let Some(sign @ ('+' | '-')) = self.peek() {
                self.bump();
                text.push(sign);
            }
            let exp = self.read_digits(10, true, start)?;
            if exp.is_empty() {
                return self.err("missing exponent digits", start);
            }
            text.push_str(&exp);
        }
        Ok(())
    }

    fn read_string(&mut self, quote: char, start: Span) -> LexResult<(Vec<u16>, bool)> {
        self.bump();
        let mut out = Vec::new();
        let mut legacy = false;
        loop {
            match self.peek() {
                None => return self.err("unterminated string literal", start),
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some('\n' | '\r') => return self.err("unterminated string literal", start),
                Some('\\') => match self.read_escape(&mut out, false, start)? {
                    EscapeKind::LegacyOctal => legacy = true,
                    EscapeKind::Invalid(span) => {
                        return Err(SyntaxError::new("invalid escape sequence", span));
                    }
                    EscapeKind::Normal | EscapeKind::LineContinuation => {}
                },
                Some(c) => {
                    self.bump();
                    push_char(&mut out, c);
                }
            }
        }
        Ok((out, legacy))
    }

    /// Read one escape starting at the backslash. In strings, a bad escape is
    /// an error right away. In templates it comes back as `Invalid`.
    fn read_escape(
        &mut self,
        out: &mut Vec<u16>,
        template: bool,
        start: Span,
    ) -> LexResult<EscapeKind> {
        let esc_start = self.here();
        self.bump();
        let Some(c) = self.bump() else {
            return self.err("unterminated literal", start);
        };
        let unit = match c {
            'b' => 0x08,
            't' => 0x09,
            'n' => 0x0A,
            'v' => 0x0B,
            'f' => 0x0C,
            'r' => 0x0D,
            '\r' => {
                self.eat('\n');
                return Ok(EscapeKind::LineContinuation);
            }
            '\n' | '\u{2028}' | '\u{2029}' => return Ok(EscapeKind::LineContinuation),
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => 0,
            '0'..='7' => {
                if template {
                    return Ok(EscapeKind::Invalid(self.span_from(esc_start)));
                }
                // Up to three octal digits, value at most 0o377.
                let mut value = c as u32 - '0' as u32;
                let max_digits = if c <= '3' { 3 } else { 2 };
                let mut n = 1;
                while n < max_digits {
                    match self.peek() {
                        Some(d @ '0'..='7') => {
                            self.bump();
                            value = value * 8 + (d as u32 - '0' as u32);
                            n += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value as u16);
                return Ok(EscapeKind::LegacyOctal);
            }
            '8' | '9' => {
                if template {
                    return Ok(EscapeKind::Invalid(self.span_from(esc_start)));
                }
                out.push(c as u16);
                return Ok(EscapeKind::LegacyOctal);
            }
            'x' => {
                let mut value = 0u32;
                for _ in 0..2 {
                    match self.peek() {
                        Some(d) if d.is_ascii_hexdigit() => {
                            self.bump();
                            value = value * 16 + d.to_digit(16).unwrap_or(0);
                        }
                        _ => {
                            if template {
                                return Ok(EscapeKind::Invalid(self.span_from(esc_start)));
                            }
                            return self.err("`\\x` needs two hex digits", esc_start);
                        }
                    }
                }
                value as u16
            }
            'u' => {
                let saved = self.save();
                match self.read_unicode_escape_body(esc_start) {
                    Ok(cp) => {
                        push_code_point(out, cp);
                        return Ok(EscapeKind::Normal);
                    }
                    Err(e) => {
                        if template {
                            self.restore(saved);
                            return Ok(EscapeKind::Invalid(self.span_from(esc_start)));
                        }
                        return Err(e);
                    }
                }
            }
            other => {
                push_char(out, other);
                return Ok(EscapeKind::Normal);
            }
        };
        out.push(unit);
        Ok(EscapeKind::Normal)
    }

    /// Read template characters after a backtick or `}` up to `${` or the
    /// closing backtick.
    fn read_template_chars(&mut self, start: Span) -> LexResult<TemplatePart> {
        let mut cooked = Vec::new();
        let mut bad_escape = None;
        let mut raw = String::new();
        let tail;
        loop {
            let piece_start = self.pos;
            match self.peek() {
                None => return self.err("unterminated template literal", start),
                Some('`') => {
                    self.bump();
                    tail = true;
                    break;
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    self.bump();
                    self.bump();
                    tail = false;
                    break;
                }
                Some('\\') => {
                    match self.read_escape(&mut cooked, true, start)? {
                        EscapeKind::Invalid(span) => {
                            if bad_escape.is_none() {
                                bad_escape = Some(span);
                            }
                        }
                        EscapeKind::Normal
                        | EscapeKind::LineContinuation
                        | EscapeKind::LegacyOctal => {}
                    }
                    push_raw(&mut raw, &self.src[piece_start..self.pos]);
                }
                Some('\r') => {
                    self.bump();
                    self.eat('\n');
                    cooked.push(0x0A);
                    raw.push('\n');
                }
                Some(c) => {
                    self.bump();
                    push_char(&mut cooked, c);
                    raw.push(c);
                }
            }
        }
        Ok(TemplatePart {
            cooked: if bad_escape.is_some() {
                None
            } else {
                Some(cooked)
            },
            bad_escape,
            raw,
            tail,
        })
    }

    /// Read `tok` (a `/` or `/=` punctuator) again as a regex literal.
    pub fn rescan_regex(&mut self, tok: &Token) -> LexResult<Token> {
        self.reset_to(tok.span);
        let start = self.here();
        self.bump();
        let mut in_class = false;
        loop {
            match self.peek() {
                None => return self.err("unterminated regular expression", start),
                Some(c) if is_line_terminator(c) => {
                    return self.err("unterminated regular expression", start);
                }
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        Some(c) if !is_line_terminator(c) => {
                            self.bump();
                        }
                        _ => return self.err("unterminated regular expression", start),
                    }
                }
                Some('[') => {
                    in_class = true;
                    self.bump();
                }
                Some(']') => {
                    in_class = false;
                    self.bump();
                }
                Some('/') if !in_class => {
                    self.bump();
                    break;
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
        let pattern = self.src[start.start + 1..self.pos - 1].to_string();
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c == '\\' {
                return self.err("escapes are not allowed in regex flags", start);
            }
            if !is_id_continue(c) {
                break;
            }
            self.bump();
            if !"dgimsuvy".contains(c) {
                return self.err(format!("unknown regex flag `{c}`"), start);
            }
            if flags.contains(c) {
                return self.err(format!("duplicate regex flag `{c}`"), start);
            }
            flags.push(c);
        }
        if flags.contains('u') && flags.contains('v') {
            return self.err("regex flags `u` and `v` can't be used together", start);
        }
        Ok(Token {
            kind: TokenKind::Regex { pattern, flags },
            span: self.span_from(start),
            newline_before: tok.newline_before,
            escaped: false,
            legacy_octal: false,
        })
    }

    /// Read `tok` (a `}` punctuator) again as the next template piece.
    pub fn rescan_template_continuation(&mut self, tok: &Token) -> LexResult<Token> {
        self.reset_to(tok.span);
        let start = self.here();
        self.bump();
        let part = self.read_template_chars(start)?;
        Ok(Token {
            kind: TokenKind::Template(part),
            span: self.span_from(start),
            newline_before: tok.newline_before,
            escaped: false,
            legacy_octal: false,
        })
    }
}

fn push_char(out: &mut Vec<u16>, c: char) {
    let mut buf = [0u16; 2];
    out.extend_from_slice(c.encode_utf16(&mut buf));
}

fn push_code_point(out: &mut Vec<u16>, cp: u32) {
    match char::from_u32(cp) {
        Some(c) => push_char(out, c),
        // A lone surrogate from `\uD800` and friends.
        None => out.push(cp as u16),
    }
}

fn push_raw(raw: &mut String, text: &str) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            raw.push('\n');
        } else {
            raw.push(c);
        }
    }
}

fn trim_zeros(digits: &str) -> String {
    let t = digits.trim_start_matches('0');
    if t.is_empty() {
        "0".to_string()
    } else {
        t.to_string()
    }
}

/// Decimal text (digits, optional fraction, optional exponent) to the
/// nearest `f64`.
fn parse_decimal(text: &str) -> f64 {
    let mut t = text.to_string();
    if t.starts_with('.') {
        t.insert(0, '0');
    }
    if t.ends_with('.') {
        t.push('0');
    }
    if let Some(i) = t.find(['e', 'E'])
        && t[..i].ends_with('.')
    {
        t.insert(i, '0');
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// Digits in a power-of-two radix to the nearest `f64`, rounding half to
/// even like the spec asks.
pub(crate) fn radix_to_f64(digits: &str, radix: u32) -> f64 {
    let bits_per = match radix {
        2 => 1,
        8 => 3,
        _ => 4,
    };
    let mut bits: Vec<bool> = Vec::with_capacity(digits.len() * bits_per);
    for c in digits.chars() {
        let v = c.to_digit(radix).unwrap_or(0);
        for i in (0..bits_per).rev() {
            bits.push((v >> i) & 1 == 1);
        }
    }
    let first_one = bits.iter().position(|&b| b).unwrap_or(bits.len());
    let bits = &bits[first_one..];
    if bits.len() <= 64 {
        let mut v: u64 = 0;
        for &b in bits {
            v = (v << 1) | b as u64;
        }
        return v as f64;
    }
    let mut top: u64 = 0;
    for &b in &bits[..64] {
        top = (top << 1) | b as u64;
    }
    if bits[64..].iter().any(|&b| b) {
        // Sticky bit: well below the 53-bit mantissa, so the u64 to f64
        // conversion still rounds the right way.
        top |= 1;
    }
    let shift = bits.len() - 64;
    let mut value = top as f64;
    for _ in 0..shift {
        value *= 2.0;
        if value.is_infinite() {
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let mut lx = Lexer::new(src, Goal::Script);
        let mut out = Vec::new();
        loop {
            let t = lx.next_token().expect("lex");
            if t.kind == TokenKind::Eof {
                break;
            }
            out.push(t.kind);
        }
        out
    }

    fn one(src: &str) -> TokenKind {
        let k = kinds(src);
        assert_eq!(k.len(), 1, "{src}: {k:?}");
        k.into_iter().next().unwrap_or(TokenKind::Eof)
    }

    fn lex_err(src: &str) -> SyntaxError {
        let mut lx = Lexer::new(src, Goal::Script);
        loop {
            match lx.next_token() {
                Ok(t) if t.kind == TokenKind::Eof => panic!("expected an error for {src:?}"),
                Ok(_) => {}
                Err(e) => return e,
            }
        }
    }

    fn num(src: &str) -> f64 {
        match one(src) {
            TokenKind::Number(n) => n,
            other => panic!("{src}: {other:?}"),
        }
    }

    fn s(src: &str) -> String {
        match one(src) {
            TokenKind::String(v) => String::from_utf16_lossy(&v),
            other => panic!("{src}: {other:?}"),
        }
    }

    #[test]
    fn decimal_numbers() {
        assert_eq!(num("0"), 0.0);
        assert_eq!(num("42"), 42.0);
        assert_eq!(num("1.5"), 1.5);
        assert_eq!(num(".5"), 0.5);
        assert_eq!(num("5."), 5.0);
        assert_eq!(num("1e3"), 1000.0);
        assert_eq!(num("1E-2"), 0.01);
        assert_eq!(num("2.e+2"), 200.0);
        assert_eq!(num("1_000_000"), 1_000_000.0);
        assert_eq!(num("1e400"), f64::INFINITY);
        assert_eq!(num("0.1"), 0.1);
    }

    #[test]
    fn radix_numbers() {
        assert_eq!(num("0xff"), 255.0);
        assert_eq!(num("0XAB_CD"), 43981.0);
        assert_eq!(num("0o17"), 15.0);
        assert_eq!(num("0b1010"), 10.0);
        assert_eq!(num("0x1fffffffffffff"), 9007199254740991.0);
        // 2^53 + 1 rounds to even (2^53); 2^53 + 3 rounds up.
        assert_eq!(num("0x20000000000001"), 9007199254740992.0);
        assert_eq!(num("0x20000000000003"), 9007199254740996.0);
        assert_eq!(num("0x10000000000000000"), 18446744073709551616.0);
    }

    #[test]
    fn huge_hex_rounds_correctly() {
        // 2^124 + 2^71 + 1: exactly above halfway between two doubles, so it
        // rounds up to 2^124 + 2^72.
        let v = radix_to_f64("10000000000000800000000000000001", 16);
        let expected = 2f64.powi(124) + 2f64.powi(72);
        assert_eq!(v, expected);
        // Exactly halfway with an even mantissa rounds down.
        let v = radix_to_f64("10000000000000800000000000000000", 16);
        assert_eq!(v, 2f64.powi(124));
    }

    #[test]
    fn legacy_octal_flags() {
        let mut lx = Lexer::new("017 089 0", Goal::Script);
        let a = lx.next_token().unwrap();
        assert_eq!(a.kind, TokenKind::Number(15.0));
        assert!(a.legacy_octal);
        let b = lx.next_token().unwrap();
        assert_eq!(b.kind, TokenKind::Number(89.0));
        assert!(b.legacy_octal);
        let c = lx.next_token().unwrap();
        assert!(!c.legacy_octal);
    }

    #[test]
    fn bigint_literals() {
        assert_eq!(
            one("123n"),
            TokenKind::BigInt(BigIntLit {
                radix: 10,
                digits: "123".into()
            })
        );
        assert_eq!(
            one("0xFFn"),
            TokenKind::BigInt(BigIntLit {
                radix: 16,
                digits: "ff".into()
            })
        );
        assert_eq!(
            one("0n"),
            TokenKind::BigInt(BigIntLit {
                radix: 10,
                digits: "0".into()
            })
        );
    }

    #[test]
    fn bad_numbers() {
        for src in [
            "1__0", "1_", "0_1", "0x", "0b2", "1.5n", "1e5n", "3in", "07n", "1e", "0x_1", "08_1",
            "1._5",
        ] {
            lex_err(src);
        }
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(s("'abc'"), "abc");
        assert_eq!(s(r#""a\nb""#), "a\nb");
        assert_eq!(s(r"'\x41\u0042\u{43}'"), "ABC");
        assert_eq!(s(r"'\u{1F600}'"), "\u{1F600}");
        assert_eq!(s("'a\\\nb'"), "ab");
        assert_eq!(s("'a\\\r\nb'"), "ab");
        assert_eq!(s(r"'\0'"), "\0");
        assert_eq!(s(r"'\q'"), "q");
        assert_eq!(s("'\u{2028}'"), "\u{2028}");
    }

    #[test]
    fn string_utf16_units() {
        match one(r"'\uD800'") {
            TokenKind::String(v) => assert_eq!(v, vec![0xD800]),
            other => panic!("{other:?}"),
        }
        match one("'\u{1F600}'") {
            TokenKind::String(v) => assert_eq!(v.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn octal_escapes() {
        let mut lx = Lexer::new(r"'\101' '\8' '\0'", Goal::Script);
        let a = lx.next_token().unwrap();
        assert_eq!(a.kind, TokenKind::String(vec![65]));
        assert!(a.legacy_octal);
        assert!(lx.next_token().unwrap().legacy_octal);
        assert!(!lx.next_token().unwrap().legacy_octal);
    }

    #[test]
    fn bad_strings() {
        for src in [
            "'abc",
            "'a\nb'",
            r"'\x4'",
            r"'\u12'",
            r"'\u{110000}'",
            r"'\u{}'",
        ] {
            lex_err(src);
        }
    }

    #[test]
    fn punctuators_longest_match() {
        let k = kinds(">>>= >>> >>= >> >= ** **= ?? ??= ?. ... => &&= ||=");
        let want = [
            Punct::UShrEq,
            Punct::UShr,
            Punct::ShrEq,
            Punct::Shr,
            Punct::GtEq,
            Punct::StarStar,
            Punct::StarStarEq,
            Punct::QuestionQuestion,
            Punct::QuestionQuestionEq,
            Punct::QuestionDot,
            Punct::Ellipsis,
            Punct::Arrow,
            Punct::AmpAmpEq,
            Punct::PipePipeEq,
        ];
        assert_eq!(
            k,
            want.iter()
                .map(|p| TokenKind::Punct(*p))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn question_dot_before_digit_is_conditional() {
        let k = kinds("a?.5:1");
        assert_eq!(k[1], TokenKind::Punct(Punct::Question));
        assert_eq!(k[2], TokenKind::Number(0.5));
    }

    #[test]
    fn identifiers_and_escapes() {
        assert_eq!(one("café"), TokenKind::Ident("café".into()));
        assert_eq!(one(r"\u0061bc"), TokenKind::Ident("abc".into()));
        assert_eq!(one(r"a\u{62}"), TokenKind::Ident("ab".into()));
        assert_eq!(one("$_x9"), TokenKind::Ident("$_x9".into()));
        let mut lx = Lexer::new(r"\u0069f", Goal::Script);
        let t = lx.next_token().unwrap();
        assert!(t.escaped);
        assert!(!t.is_word("if"));
        lex_err(r"\u0031a");
        lex_err(r"a\u002Db");
        lex_err(r"\x61");
    }

    #[test]
    fn private_names() {
        assert_eq!(one("#x"), TokenKind::PrivateName("x".into()));
        lex_err("# x");
    }

    #[test]
    fn comments_and_newlines() {
        let mut lx = Lexer::new("a // c\n b /* x */ c /* \n */ d", Goal::Script);
        assert!(!lx.next_token().unwrap().newline_before);
        assert!(lx.next_token().unwrap().newline_before);
        assert!(!lx.next_token().unwrap().newline_before);
        assert!(lx.next_token().unwrap().newline_before);
        lex_err("/* never closed");
    }

    #[test]
    fn hashbang_only_at_start() {
        assert_eq!(
            kinds("#!/usr/bin/env anvil\nx"),
            vec![TokenKind::Ident("x".into())]
        );
        lex_err(" #!x");
    }

    #[test]
    fn html_comments_in_scripts_only() {
        assert_eq!(
            kinds("x <!-- y\nz"),
            vec![TokenKind::Ident("x".into()), TokenKind::Ident("z".into())]
        );
        assert_eq!(
            kinds("x\n--> y\nz"),
            vec![TokenKind::Ident("x".into()), TokenKind::Ident("z".into())]
        );
        let mut lx = Lexer::new("x <!-- y", Goal::Module);
        lx.next_token().unwrap();
        assert_eq!(lx.next_token().unwrap().kind, TokenKind::Punct(Punct::Lt));
    }

    #[test]
    fn line_and_column() {
        let mut lx = Lexer::new("a\r\n  b\u{2028}c\r d", Goal::Script);
        let a = lx.next_token().unwrap();
        assert_eq!((a.span.line, a.span.column), (1, 1));
        let b = lx.next_token().unwrap();
        assert_eq!((b.span.line, b.span.column), (2, 3));
        let c = lx.next_token().unwrap();
        assert_eq!((c.span.line, c.span.column), (3, 1));
        let d = lx.next_token().unwrap();
        assert_eq!((d.span.line, d.span.column), (4, 2));
    }

    #[test]
    fn columns_count_utf16() {
        let mut lx = Lexer::new("'\u{1F600}' x", Goal::Script);
        lx.next_token().unwrap();
        let x = lx.next_token().unwrap();
        assert_eq!(x.span.column, 6);
    }

    #[test]
    fn regex_rescan() {
        let mut lx = Lexer::new("/a[/]b\\/c/gi;", Goal::Script);
        let slash = lx.next_token().unwrap();
        assert!(slash.is_punct(Punct::Slash));
        let re = lx.rescan_regex(&slash).unwrap();
        assert_eq!(
            re.kind,
            TokenKind::Regex {
                pattern: "a[/]b\\/c".into(),
                flags: "gi".into()
            }
        );
        assert!(lx.next_token().unwrap().is_punct(Punct::Semicolon));
    }

    #[test]
    fn regex_errors() {
        for src in ["/abc", "/a\nb/", "/a/gg", "/a/x", "/a/uv", "/a/\\u0067"] {
            let mut lx = Lexer::new(src, Goal::Script);
            let t = lx.next_token().unwrap();
            assert!(lx.rescan_regex(&t).is_err(), "{src}");
        }
    }

    #[test]
    fn template_pieces() {
        let mut lx = Lexer::new("`a${x}b${y}c`", Goal::Script);
        let head = lx.next_token().unwrap();
        match &head.kind {
            TokenKind::Template(p) => {
                assert_eq!(p.raw, "a");
                assert!(!p.tail);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(lx.next_token().unwrap().kind, TokenKind::Ident("x".into()));
        let close = lx.next_token().unwrap();
        let mid = lx.rescan_template_continuation(&close).unwrap();
        match &mid.kind {
            TokenKind::Template(p) => {
                assert_eq!(p.raw, "b");
                assert!(!p.tail);
            }
            other => panic!("{other:?}"),
        }
        lx.next_token().unwrap();
        let close = lx.next_token().unwrap();
        let tail = lx.rescan_template_continuation(&close).unwrap();
        match &tail.kind {
            TokenKind::Template(p) => {
                assert_eq!(p.raw, "c");
                assert!(p.tail);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn template_raw_and_cooked() {
        match one("`a\r\nb\\u0041\\\n`") {
            TokenKind::Template(p) => {
                assert_eq!(p.raw, "a\nb\\u0041\\\n");
                assert_eq!(
                    String::from_utf16_lossy(p.cooked.as_ref().unwrap()),
                    "a\nbA"
                );
            }
            other => panic!("{other:?}"),
        }
        match one(r"`\unicode and \01`") {
            TokenKind::Template(p) => {
                assert!(p.cooked.is_none());
                assert!(p.bad_escape.is_some());
                assert_eq!(p.raw, r"\unicode and \01");
            }
            other => panic!("{other:?}"),
        }
        lex_err("`abc");
    }

    #[test]
    fn never_panics_on_odd_bytes() {
        for src in [
            "\u{0}", "\\", "\\u", "\\u{", "'\\", "`\\", "`${", "0x", ".", "..", "@", "\u{FFFF}",
        ] {
            let mut lx = Lexer::new(src, Goal::Script);
            for _ in 0..8 {
                match lx.next_token() {
                    Ok(t) if t.kind == TokenKind::Eof => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    }

    #[test]
    fn whitespace_set() {
        assert_eq!(
            kinds("\u{FEFF}\u{00A0}\u{3000}x\t\u{000B}\u{000C}"),
            vec![TokenKind::Ident("x".into())]
        );
        // NEL is not whitespace in JavaScript.
        lex_err("\u{0085}");
    }
}
