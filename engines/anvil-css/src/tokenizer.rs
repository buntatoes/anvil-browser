// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! CSS Syntax Level 3 tokenizer.
//!
//! Written from section 4 of CSS Syntax Level 3 ("Tokenization"). Input
//! preprocessing (CR LF, CR, and FF become LF; NUL becomes U+FFFD) happens on
//! the fly, so every [`Span`] is a byte range in the caller's original text.
//! Comments are consumed and produce no token, as the specification says.
//!
//! The tokenizer never panics and always makes progress: each call to
//! [`Tokenizer::next_token`] consumes at least one code point or returns
//! `None` at the end of the input. Recoverable problems (an unterminated
//! string, a bad `url(`, an end of file inside a comment) are recorded as
//! [`Diagnostic`]s, and the token the specification asks for is produced.
//!
//! `unicode-range` tokens (`U+0-7F`, `u+4??`) were dropped from the current
//! draft of the specification because they break selectors such as `u+a`.
//! They are produced only when [`Tokenizer::with_unicode_ranges`] is turned
//! on, which the value parser does for the `@font-face` `unicode-range`
//! descriptor.

use crate::anvil_css::error::{Diagnostic, DiagnosticKind, LimitError, LimitKind, Span};

/// The numeric part of a number, percentage, or dimension token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number {
    /// The value. Values too large for `f64` are clamped to `±f64::MAX`.
    pub value: f64,
    /// True when the source had no `.` and no exponent (type "integer").
    pub is_integer: bool,
    /// True when the source started with an explicit `+` or `-`.
    pub has_sign: bool,
}

/// One CSS token.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// `<ident-token>`, with escapes resolved.
    Ident(String),
    /// `<function-token>`: a name directly followed by `(`.
    Function(String),
    /// `<at-keyword-token>`, without the `@`.
    AtKeyword(String),
    /// `<hash-token>`, without the `#`. `is_id` is the "id" type flag: the
    /// rest would start an identifier.
    Hash {
        /// The name after `#`.
        value: String,
        /// True for type flag "id".
        is_id: bool,
    },
    /// `<string-token>`, without quotes and with escapes resolved.
    String(String),
    /// `<bad-string-token>`: a string that ran into a newline.
    BadString,
    /// `<url-token>` from an unquoted `url(...)`.
    Url(String),
    /// `<bad-url-token>`.
    BadUrl,
    /// `<delim-token>`.
    Delim(char),
    /// `<number-token>`.
    Number(Number),
    /// `<percentage-token>`. The value is the number before `%`.
    Percentage(Number),
    /// `<dimension-token>`.
    Dimension {
        /// The numeric part.
        value: Number,
        /// The unit, with escapes resolved and case kept.
        unit: String,
    },
    /// `<unicode-range-token>` (only with unicode ranges turned on).
    UnicodeRange {
        /// First code point.
        start: u32,
        /// Last code point.
        end: u32,
    },
    /// `<whitespace-token>`: one or more whitespace code points.
    Whitespace,
    /// `<CDO-token>`: `<!--`.
    Cdo,
    /// `<CDC-token>`: `-->`.
    Cdc,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `[`
    LeftBracket,
    /// `]`
    RightBracket,
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `{`
    LeftBrace,
    /// `}`
    RightBrace,
}

impl Token {
    /// True for [`Token::Whitespace`].
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Token::Whitespace)
    }

    /// True when this is an ident equal to `name`, ignoring ASCII case.
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(self, Token::Ident(v) if v.eq_ignore_ascii_case(name))
    }

    /// True when this is the delimiter `c`.
    pub fn is_delim(&self, c: char) -> bool {
        matches!(self, Token::Delim(d) if *d == c)
    }
}

/// A token and the source bytes it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct SpannedToken {
    /// The token.
    pub token: Token,
    /// Source bytes, in the original input.
    pub span: Span,
}

/// Whitespace after preprocessing: LF, tab, and space.
pub fn is_whitespace(c: char) -> bool {
    matches!(c, '\n' | '\t' | ' ')
}

/// Ident-start code point: ASCII letter, `_`, or any non-ASCII code point.
pub fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

/// Ident code point: an ident-start code point, a digit, or `-`.
pub fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

fn is_non_printable(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{E}'..='\u{1F}' | '\u{7F}')
}

/// "Check if two code points are a valid escape".
pub fn is_valid_escape(a: Option<char>, b: Option<char>) -> bool {
    a == Some('\\') && b != Some('\n')
}

/// "Check if three code points would start an ident sequence".
pub fn would_start_ident(a: Option<char>, b: Option<char>, c: Option<char>) -> bool {
    match a {
        Some('-') => matches!(b, Some(x) if is_name_start(x) || x == '-') || is_valid_escape(b, c),
        Some('\\') => is_valid_escape(a, b),
        Some(x) => is_name_start(x),
        None => false,
    }
}

/// "Check if three code points would start a number".
pub fn would_start_number(a: Option<char>, b: Option<char>, c: Option<char>) -> bool {
    match a {
        Some('+') | Some('-') => {
            matches!(b, Some(x) if x.is_ascii_digit())
                || (b == Some('.') && matches!(c, Some(x) if x.is_ascii_digit()))
        }
        Some('.') => matches!(b, Some(x) if x.is_ascii_digit()),
        Some(x) => x.is_ascii_digit(),
        None => false,
    }
}

/// Streaming tokenizer over one input string.
#[derive(Debug)]
pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
    unicode_ranges: bool,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Tokenizer<'a> {
    /// Start at the beginning of `src`.
    pub fn new(src: &'a str) -> Self {
        Tokenizer {
            src,
            pos: 0,
            unicode_ranges: false,
            diagnostics: Vec::new(),
        }
    }

    /// Turn `unicode-range` tokens on or off (off by default).
    pub fn with_unicode_ranges(mut self, on: bool) -> Self {
        self.unicode_ranges = on;
        self
    }

    /// Start reading at byte offset `pos`, which must be a character
    /// boundary. Spans stay offsets into the whole text.
    pub(crate) fn starting_at(mut self, pos: usize) -> Self {
        if self.src.is_char_boundary(pos) {
            self.pos = pos;
        }
        self
    }

    /// Current byte offset.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Diagnostics recorded so far.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Take the diagnostics recorded so far.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Decode the preprocessed code point at byte `pos` and its byte length.
    fn char_at(&self, pos: usize) -> Option<(char, usize)> {
        let rest = self.src.get(pos..)?;
        let c = rest.chars().next()?;
        Some(match c {
            '\r' => {
                if rest.as_bytes().get(1) == Some(&b'\n') {
                    ('\n', 2)
                } else {
                    ('\n', 1)
                }
            }
            '\u{C}' => ('\n', 1),
            '\u{0}' => ('\u{FFFD}', 1),
            c => (c, c.len_utf8()),
        })
    }

    /// The code point `n` places after the current one (0 is the next one).
    fn peek(&self, n: usize) -> Option<char> {
        let mut p = self.pos;
        for _ in 0..n {
            let (_, len) = self.char_at(p)?;
            p += len;
        }
        self.char_at(p).map(|(c, _)| c)
    }

    fn bump(&mut self) -> Option<char> {
        let (c, len) = self.char_at(self.pos)?;
        self.pos += len;
        Some(c)
    }

    fn diag(&mut self, kind: DiagnosticKind, message: &str, start: usize) {
        let span = Span::new(start, self.pos.max(start));
        self.diagnostics.push(Diagnostic::new(kind, message, span));
    }

    fn consume_comments(&mut self) {
        while self.src.as_bytes().get(self.pos) == Some(&b'/')
            && self.src.as_bytes().get(self.pos + 1) == Some(&b'*')
        {
            let start = self.pos;
            match self.src[self.pos + 2..].find("*/") {
                Some(i) => self.pos += 2 + i + 2,
                None => {
                    self.pos = self.src.len();
                    self.diag(DiagnosticKind::UnexpectedEof, "unterminated comment", start);
                }
            }
        }
    }

    /// Produce the next token, or `None` at the end of the input.
    pub fn next_token(&mut self) -> Option<SpannedToken> {
        self.consume_comments();
        let start = self.pos;
        let c = self.bump()?;
        let token = match c {
            c if is_whitespace(c) => {
                let bytes = self.src.as_bytes();
                let len = bytes.len();
                while self.pos < len {
                    // SAFETY: `self.pos < len` is checked by the loop condition.
                    let b = unsafe { *bytes.get_unchecked(self.pos) };
                    if matches!(b, b' ' | b'\t' | b'\n') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                while matches!(self.peek(0), Some(x) if is_whitespace(x)) {
                    self.bump();
                }
                Token::Whitespace
            }
            '"' | '\'' => self.consume_string(c, start),
            '#' => {
                let (p0, p1, p2) = (self.peek(0), self.peek(1), self.peek(2));
                if matches!(p0, Some(x) if is_name_char(x)) || is_valid_escape(p0, p1) {
                    let is_id = would_start_ident(p0, p1, p2);
                    let value = self.consume_name();
                    Token::Hash { value, is_id }
                } else {
                    Token::Delim('#')
                }
            }
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            ',' => Token::Comma,
            ':' => Token::Colon,
            ';' => Token::Semicolon,
            '+' => {
                if would_start_number(Some('+'), self.peek(0), self.peek(1)) {
                    self.pos = start;
                    self.consume_numeric()
                } else {
                    Token::Delim('+')
                }
            }
            '-' => {
                let (p0, p1) = (self.peek(0), self.peek(1));
                if would_start_number(Some('-'), p0, p1) {
                    self.pos = start;
                    self.consume_numeric()
                } else if p0 == Some('-') && p1 == Some('>') {
                    self.bump();
                    self.bump();
                    Token::Cdc
                } else if would_start_ident(Some('-'), p0, p1) {
                    self.pos = start;
                    self.consume_ident_like(start)
                } else {
                    Token::Delim('-')
                }
            }
            '.' => {
                if would_start_number(Some('.'), self.peek(0), self.peek(1)) {
                    self.pos = start;
                    self.consume_numeric()
                } else {
                    Token::Delim('.')
                }
            }
            '<' => {
                if self.peek(0) == Some('!')
                    && self.peek(1) == Some('-')
                    && self.peek(2) == Some('-')
                {
                    self.bump();
                    self.bump();
                    self.bump();
                    Token::Cdo
                } else {
                    Token::Delim('<')
                }
            }
            '@' => {
                if would_start_ident(self.peek(0), self.peek(1), self.peek(2)) {
                    Token::AtKeyword(self.consume_name())
                } else {
                    Token::Delim('@')
                }
            }
            '\\' => {
                if is_valid_escape(Some('\\'), self.peek(0)) {
                    self.pos = start;
                    self.consume_ident_like(start)
                } else {
                    self.diag(
                        DiagnosticKind::InvalidEscape,
                        "backslash followed by a newline",
                        start,
                    );
                    Token::Delim('\\')
                }
            }
            c if c.is_ascii_digit() => {
                self.pos = start;
                self.consume_numeric()
            }
            'u' | 'U'
                if self.unicode_ranges
                    && self.peek(0) == Some('+')
                    && matches!(self.peek(1), Some(x) if x.is_ascii_hexdigit() || x == '?') =>
            {
                self.bump();
                self.consume_unicode_range()
            }
            c if is_name_start(c) => {
                self.pos = start;
                self.consume_ident_like(start)
            }
            c => Token::Delim(c),
        };
        Some(SpannedToken {
            token,
            span: Span::new(start, self.pos),
        })
    }

    /// "Consume an escaped code point". The backslash is already consumed.
    fn consume_escape(&mut self) -> char {
        let start = self.pos;
        match self.bump() {
            None => {
                self.diag(
                    DiagnosticKind::UnexpectedEof,
                    "escape at end of input",
                    start,
                );
                '\u{FFFD}'
            }
            Some(c) if c.is_ascii_hexdigit() => {
                let mut value = c.to_digit(16).unwrap_or(0);
                let mut count = 1;
                while count < 6 {
                    match self.peek(0) {
                        Some(x) if x.is_ascii_hexdigit() => {
                            value = value * 16 + x.to_digit(16).unwrap_or(0);
                            self.bump();
                            count += 1;
                        }
                        _ => break,
                    }
                }
                if matches!(self.peek(0), Some(x) if is_whitespace(x)) {
                    self.bump();
                }
                if value == 0 {
                    '\u{FFFD}'
                } else {
                    char::from_u32(value).unwrap_or('\u{FFFD}')
                }
            }
            Some(c) => c,
        }
    }

    /// "Consume an ident sequence".
    fn consume_name(&mut self) -> String {
        let mut out = String::new();
        let bytes = self.src.as_bytes();
        let len = bytes.len();
        loop {
            let run_start = self.pos;
            while self.pos < len {
                // SAFETY: `self.pos < len` is checked by the loop condition.
                let b = unsafe { *bytes.get_unchecked(self.pos) };
                if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos > run_start {
                // SAFETY: every byte in `run_start..self.pos` is verified ASCII.
                let slice = unsafe {
                    std::str::from_utf8_unchecked(bytes.get_unchecked(run_start..self.pos))
                };
                out.push_str(slice);
            }
            match self.peek(0) {
                Some(c) if is_name_char(c) => {
                    self.bump();
                    out.push(c);
                }
                Some('\\') if is_valid_escape(Some('\\'), self.peek(1)) => {
                    self.bump();
                    out.push(self.consume_escape());
                }
                _ => return out,
            }
        }
    }

    /// "Consume a number". Returns the numeric part.
    fn consume_number(&mut self) -> Number {
        let mut repr = String::new();
        let mut is_integer = true;
        let mut has_sign = false;
        if let Some(c @ ('+' | '-')) = self.peek(0) {
            has_sign = true;
            repr.push(c);
            self.bump();
        }
        self.push_digits(&mut repr);
        if self.peek(0) == Some('.') && matches!(self.peek(1), Some(x) if x.is_ascii_digit()) {
            is_integer = false;
            repr.push('.');
            self.bump();
            self.push_digits(&mut repr);
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let p1 = self.peek(1);
            let p2 = self.peek(2);
            let digit = |c: Option<char>| matches!(c, Some(x) if x.is_ascii_digit());
            if digit(p1) || (matches!(p1, Some('+' | '-')) && digit(p2)) {
                is_integer = false;
                repr.push('e');
                self.bump();
                if let Some(s @ ('+' | '-')) = self.peek(0) {
                    repr.push(s);
                    self.bump();
                }
                self.push_digits(&mut repr);
            }
        }
        let mut value: f64 = repr.parse().unwrap_or(0.0);
        if value.is_infinite() {
            value = if value > 0.0 { f64::MAX } else { -f64::MAX };
        }
        Number {
            value,
            is_integer,
            has_sign,
        }
    }

    fn push_digits(&mut self, out: &mut String) {
        let bytes = self.src.as_bytes();
        let len = bytes.len();
        let start = self.pos;
        while self.pos < len {
            // SAFETY: `self.pos < len` is checked by the loop condition.
            let b = unsafe { *bytes.get_unchecked(self.pos) };
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos > start {
            // SAFETY: every byte in `start..self.pos` is an ASCII digit.
            let slice =
                unsafe { std::str::from_utf8_unchecked(bytes.get_unchecked(start..self.pos)) };
            out.push_str(slice);
        }
    }

    /// "Consume a numeric token".
    fn consume_numeric(&mut self) -> Token {
        let value = self.consume_number();
        if would_start_ident(self.peek(0), self.peek(1), self.peek(2)) {
            let unit = self.consume_name();
            Token::Dimension { value, unit }
        } else if self.peek(0) == Some('%') {
            self.bump();
            Token::Percentage(value)
        } else {
            Token::Number(value)
        }
    }

    /// "Consume an ident-like token".
    fn consume_ident_like(&mut self, start: usize) -> Token {
        let name = self.consume_name();
        if name.eq_ignore_ascii_case("url") && self.peek(0) == Some('(') {
            self.bump();
            while matches!(self.peek(0), Some(x) if is_whitespace(x))
                && matches!(self.peek(1), Some(x) if is_whitespace(x))
            {
                self.bump();
            }
            let p0 = self.peek(0);
            let p1 = self.peek(1);
            let quote = |c: Option<char>| matches!(c, Some('"' | '\''));
            if quote(p0) || (matches!(p0, Some(x) if is_whitespace(x)) && quote(p1)) {
                Token::Function(name)
            } else {
                self.consume_url(start)
            }
        } else if self.peek(0) == Some('(') {
            self.bump();
            Token::Function(name)
        } else {
            Token::Ident(name)
        }
    }

    /// "Consume a string token". The opening quote is already consumed.
    fn consume_string(&mut self, ending: char, start: usize) -> Token {
        let mut out = String::new();
        loop {
            let before = self.pos;
            match self.bump() {
                None => {
                    self.diag(DiagnosticKind::UnexpectedEof, "unterminated string", start);
                    return Token::String(out);
                }
                Some(c) if c == ending => return Token::String(out),
                Some('\n') => {
                    self.pos = before;
                    self.diag(DiagnosticKind::BadString, "newline inside a string", start);
                    return Token::BadString;
                }
                Some('\\') => match self.peek(0) {
                    None => {}
                    Some('\n') => {
                        self.bump();
                    }
                    Some(_) => out.push(self.consume_escape()),
                },
                Some(c) => out.push(c),
            }
        }
    }

    /// "Consume a url token". `url(` and leading whitespace are consumed.
    fn consume_url(&mut self, start: usize) -> Token {
        let mut out = String::new();
        while matches!(self.peek(0), Some(x) if is_whitespace(x)) {
            self.bump();
        }
        loop {
            match self.bump() {
                None => {
                    self.diag(DiagnosticKind::UnexpectedEof, "unterminated url(", start);
                    return Token::Url(out);
                }
                Some(')') => return Token::Url(out),
                Some(c) if is_whitespace(c) => {
                    while matches!(self.peek(0), Some(x) if is_whitespace(x)) {
                        self.bump();
                    }
                    match self.peek(0) {
                        Some(')') => {
                            self.bump();
                            return Token::Url(out);
                        }
                        None => {
                            self.diag(DiagnosticKind::UnexpectedEof, "unterminated url(", start);
                            return Token::Url(out);
                        }
                        Some(_) => {
                            self.consume_bad_url_remnants();
                            self.diag(DiagnosticKind::BadUrl, "whitespace inside url(", start);
                            return Token::BadUrl;
                        }
                    }
                }
                Some(c) if c == '"' || c == '\'' || c == '(' || is_non_printable(c) => {
                    self.consume_bad_url_remnants();
                    self.diag(
                        DiagnosticKind::BadUrl,
                        "quote, parenthesis, or control character inside url(",
                        start,
                    );
                    return Token::BadUrl;
                }
                Some('\\') => {
                    if is_valid_escape(Some('\\'), self.peek(0)) {
                        out.push(self.consume_escape());
                    } else {
                        self.consume_bad_url_remnants();
                        self.diag(DiagnosticKind::BadUrl, "invalid escape inside url(", start);
                        return Token::BadUrl;
                    }
                }
                Some(c) => out.push(c),
            }
        }
    }

    fn consume_bad_url_remnants(&mut self) {
        loop {
            match self.bump() {
                None | Some(')') => return,
                Some('\\') if is_valid_escape(Some('\\'), self.peek(0)) => {
                    self.consume_escape();
                }
                Some(_) => {}
            }
        }
    }

    /// Unicode range after `u+`.
    fn consume_unicode_range(&mut self) -> Token {
        let mut digits = String::new();
        while digits.len() < 6 {
            match self.peek(0) {
                Some(c) if c.is_ascii_hexdigit() => {
                    digits.push(c);
                    self.bump();
                }
                _ => break,
            }
        }
        let mut wild = false;
        while digits.len() < 6 && self.peek(0) == Some('?') {
            digits.push('?');
            wild = true;
            self.bump();
        }
        if wild {
            let lo = u32::from_str_radix(&digits.replace('?', "0"), 16).unwrap_or(0);
            let hi = u32::from_str_radix(&digits.replace('?', "F"), 16).unwrap_or(0);
            return Token::UnicodeRange { start: lo, end: hi };
        }
        let start = u32::from_str_radix(&digits, 16).unwrap_or(0);
        if self.peek(0) == Some('-') && matches!(self.peek(1), Some(x) if x.is_ascii_hexdigit()) {
            self.bump();
            let mut end_digits = String::new();
            while end_digits.len() < 6 {
                match self.peek(0) {
                    Some(c) if c.is_ascii_hexdigit() => {
                        end_digits.push(c);
                        self.bump();
                    }
                    _ => break,
                }
            }
            let end = u32::from_str_radix(&end_digits, 16).unwrap_or(0);
            return Token::UnicodeRange { start, end };
        }
        Token::UnicodeRange { start, end: start }
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = SpannedToken;

    fn next(&mut self) -> Option<SpannedToken> {
        self.next_token()
    }
}

/// Tokenized input plus the diagnostics the tokenizer recorded.
#[derive(Clone, Debug, Default)]
pub struct TokenList {
    /// The tokens, in order.
    pub tokens: Vec<SpannedToken>,
    /// Recovered problems.
    pub diagnostics: Vec<Diagnostic>,
}

/// Tokenize all of `src`, checking the input byte cap and the token cap.
pub fn tokenize(
    src: &str,
    max_input_bytes: usize,
    max_tokens: usize,
) -> Result<TokenList, LimitError> {
    tokenize_with(Tokenizer::new(src), src.len(), max_input_bytes, max_tokens)
}

pub(crate) fn tokenize_with(
    mut tokenizer: Tokenizer<'_>,
    len: usize,
    max_input_bytes: usize,
    max_tokens: usize,
) -> Result<TokenList, LimitError> {
    if len > max_input_bytes {
        return Err(LimitError {
            kind: LimitKind::InputBytes,
            limit: max_input_bytes,
            found: len,
            span: Span::new(max_input_bytes, len),
        });
    }
    let mut tokens = Vec::new();
    while let Some(t) = tokenizer.next_token() {
        if tokens.len() >= max_tokens {
            return Err(LimitError {
                kind: LimitKind::Tokens,
                limit: max_tokens,
                found: max_tokens + 1,
                span: t.span,
            });
        }
        tokens.push(t);
    }
    Ok(TokenList {
        tokens,
        diagnostics: tokenizer.take_diagnostics(),
    })
}
