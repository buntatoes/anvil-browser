// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Spans, diagnostics, limits, and the error type.
//!
//! CSS is forgiving: most problems in a style sheet are recovered from by
//! dropping one declaration or one rule, and parsing carries on. Each of those
//! recoveries is recorded as a [`Diagnostic`] so nothing is lost quietly.
//!
//! A few problems stop the whole parse with an [`Error`]: input larger than
//! the byte cap, more tokens than the token cap, and more rules than the rule
//! cap. Nesting deeper than the depth cap drops the outermost rule (or
//! declaration list entry) that contains it, with a
//! [`DiagnosticKind::LimitReached`] diagnostic. All caps live in [`Limits`].

use std::fmt;

/// A region of source text, as byte offsets into the original input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the first byte.
    pub start: usize,
    /// Byte offset just past the last byte.
    pub end: usize,
}

impl Span {
    /// Build a span from two byte offsets.
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// A span covering `self` through the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// True when the span covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Which limit was reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// Input length in bytes.
    InputBytes,
    /// Number of tokens produced by the tokenizer.
    Tokens,
    /// Nesting of blocks, functions, and rules.
    Depth,
    /// Number of rules, nested rules included.
    Rules,
    /// Nesting of selector functions such as `:is()` and `:not()`.
    SelectorDepth,
    /// Compound selectors in one complex selector.
    SelectorLength,
    /// Nesting of `calc()` and other math functions, or of value functions.
    ValueDepth,
    /// Nesting of media, container, and supports conditions.
    ConditionDepth,
}

impl LimitKind {
    /// Short name used in messages.
    pub fn name(self) -> &'static str {
        match self {
            LimitKind::InputBytes => "input bytes",
            LimitKind::Tokens => "tokens",
            LimitKind::Depth => "nesting depth",
            LimitKind::Rules => "rules",
            LimitKind::SelectorDepth => "selector nesting depth",
            LimitKind::SelectorLength => "compound selectors per selector",
            LimitKind::ValueDepth => "value nesting depth",
            LimitKind::ConditionDepth => "condition nesting depth",
        }
    }
}

/// A limit that stopped the parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitError {
    /// Which limit.
    pub kind: LimitKind,
    /// The configured maximum.
    pub limit: usize,
    /// The value that went past it. For counts this is `limit + 1`.
    pub found: usize,
    /// Where in the input it happened.
    pub span: Span,
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} limit reached: limit {}, found {} (bytes {}..{})",
            self.kind.name(),
            self.limit,
            self.found,
            self.span.start,
            self.span.end
        )
    }
}

impl std::error::Error for LimitError {}

/// A failure that stops a whole parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A configured limit was reached.
    Limit(LimitError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Limit(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<LimitError> for Error {
    fn from(e: LimitError) -> Self {
        Error::Limit(e)
    }
}

/// What kind of problem a diagnostic records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// A string ran into a newline. The tokenizer produced a bad-string token.
    BadString,
    /// A `url(` could not be read. The tokenizer produced a bad-url token.
    BadUrl,
    /// A string, comment, block, or function ran to the end of the input.
    UnexpectedEof,
    /// A backslash followed by a newline outside a string.
    InvalidEscape,
    /// A qualified rule's prelude is not a valid selector list. The rule
    /// was dropped as a whole.
    InvalidSelector,
    /// A rule could not be parsed and was dropped.
    InvalidRule,
    /// A declaration could not be parsed and was dropped.
    InvalidDeclaration,
    /// An at-rule whose prelude or position is invalid was dropped.
    InvalidAtRule,
    /// An at-rule this crate does not know. It is kept as
    /// [`crate::anvil_css::parser::Rule::Unknown`].
    UnknownAtRule,
    /// A limit was reached and the containing rule or declaration was
    /// dropped. Parsing carried on after it.
    LimitReached(LimitKind),
}

/// One recovered problem, with where it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The kind of problem.
    pub kind: DiagnosticKind,
    /// Plain description.
    pub message: String,
    /// Where it happened.
    pub span: Span,
}

impl Diagnostic {
    /// Build a diagnostic.
    pub fn new(kind: DiagnosticKind, message: impl Into<String>, span: Span) -> Self {
        Diagnostic {
            kind,
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bytes {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

/// Every cap the parser checks. Build one with `Limits::default()` and change
/// the fields you need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest input, in bytes. Past it: [`Error::Limit`]. Default 320 MiB.
    pub max_input_bytes: usize,
    /// Most tokens in one input. Past it: [`Error::Limit`]. Default 80,000,000.
    pub max_tokens: usize,
    /// Deepest nesting of blocks, functions, and nested rules. Past it, the
    /// containing top-level rule is dropped with a diagnostic. Default 1,280.
    pub max_depth: usize,
    /// Most rules in one style sheet, nested rules included. Past it:
    /// [`Error::Limit`]. Default 10,000,000.
    pub max_rules: usize,
    /// Deepest nesting of selector functions. Past it the selector is
    /// invalid. Default 320.
    pub max_selector_depth: usize,
    /// Most compound selectors in one complex selector. Past it the selector
    /// is invalid. Default 2,560.
    pub max_selector_length: usize,
    /// Deepest nesting of math and value functions. Past it the value is
    /// invalid. Default 320.
    pub max_value_depth: usize,
    /// Deepest nesting of media, container, and supports conditions. Past it
    /// the condition is invalid. Default 320.
    pub max_condition_depth: usize,
}

/// Default for [`Limits::max_input_bytes`].
pub const MAX_INPUT_BYTES: usize = 320 << 20;
/// Default for [`Limits::max_tokens`].
pub const MAX_TOKENS: usize = 80_000_000;
/// Default for [`Limits::max_depth`].
pub const MAX_DEPTH: usize = 1_280;
/// Default for [`Limits::max_rules`].
pub const MAX_RULES: usize = 10_000_000;
/// Default for [`Limits::max_selector_depth`].
pub const MAX_SELECTOR_DEPTH: usize = 320;
/// Default for [`Limits::max_selector_length`].
pub const MAX_SELECTOR_LENGTH: usize = 2_560;
/// Default for [`Limits::max_value_depth`].
pub const MAX_VALUE_DEPTH: usize = 320;
/// Default for [`Limits::max_condition_depth`].
pub const MAX_CONDITION_DEPTH: usize = 320;

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_input_bytes: MAX_INPUT_BYTES,
            max_tokens: MAX_TOKENS,
            max_depth: MAX_DEPTH,
            max_rules: MAX_RULES,
            max_selector_depth: MAX_SELECTOR_DEPTH,
            max_selector_length: MAX_SELECTOR_LENGTH,
            max_value_depth: MAX_VALUE_DEPTH,
            max_condition_depth: MAX_CONDITION_DEPTH,
        }
    }
}
