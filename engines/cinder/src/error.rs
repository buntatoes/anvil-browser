// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Source spans and the syntax error type.
//!
//! A [`Span`] holds byte offsets into the source text plus the 1-based line
//! and column of its first character. Columns count UTF-16 code units, the
//! same unit JavaScript strings and source maps use. Lines end at LF, CR,
//! CRLF (counted once), U+2028, and U+2029.

use std::fmt;

/// A region of source text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset just past the last character.
    pub end: usize,
    /// 1-based line of `start`.
    pub line: u32,
    /// 1-based column of `start`, in UTF-16 code units.
    pub column: u32,
}

impl Span {
    /// A span covering `self` through the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start,
            end: other.end.max(self.end),
            line: self.line,
            column: self.column,
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

/// A lexing or parsing failure, with the place it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// Plain description of the problem.
    pub message: String,
    /// Where the problem is.
    pub span: Span,
}

impl SyntaxError {
    /// Build an error at `span`.
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        SyntaxError {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

impl std::error::Error for SyntaxError {}
