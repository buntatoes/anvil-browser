// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! How a run can end badly, and the stack that comes with it.
//!
//! There are four kinds, and every one carries a plain reason:
//!
//! - [`RunError::Thrown`]: a script exception nobody caught. Script can
//!   catch these; this is what's left when it didn't.
//! - [`RunError::Abort`]: an uncatchable limit (heap bytes, step budget,
//!   host interrupt). Script never sees these, and `finally` doesn't run.
//! - [`RunError::Compile`]: the code couldn't be loaded (for example, it
//!   needs more registers than allowed).
//! - [`RunError::Internal`]: a bug in the engine or a misused API. Never
//!   silent.

use crate::cinder::error::Span;
use crate::cinder::gc::{Abort, HeapError};
use crate::cinder::limits::LimitKind;
use crate::cinder::value::Value;
use std::fmt;

/// One line of a stack: which function, and where in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackLine {
    /// Function name (`anonymous` when it has none).
    pub function: String,
    /// 1-based line, 0 when unknown.
    pub line: u32,
    /// 1-based column, 0 when unknown.
    pub column: u32,
}

impl StackLine {
    /// The line that ends a stack cut to `max_stack_lines`:
    /// `... 9,936 more frames not shown`. It has no line or column.
    pub fn omitted(count: usize) -> StackLine {
        let digits = count.to_string();
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(c);
        }
        let noun = if count == 1 { "frame" } else { "frames" };
        StackLine {
            function: format!("... {grouped} more {noun} not shown"),
            line: 0,
            column: 0,
        }
    }

    /// True for the line [`StackLine::omitted`] makes.
    pub fn is_omitted_marker(&self) -> bool {
        self.line == 0 && self.function.starts_with("... ")
    }
}

impl fmt::Display for StackLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_omitted_marker() {
            write!(f, "{}", self.function)
        } else if self.line == 0 {
            write!(f, "at {}", self.function)
        } else {
            write!(f, "at {} ({}:{})", self.function, self.line, self.column)
        }
    }
}

/// How a run ended, when it didn't end with a value.
#[derive(Clone, Debug)]
pub enum RunError {
    /// An exception nobody caught.
    Thrown {
        /// The thrown value.
        value: Value,
        /// `Name: message` for error objects, the value as text otherwise.
        message: String,
        /// Where it was thrown, innermost first.
        stack: Vec<StackLine>,
    },
    /// An uncatchable limit.
    Abort {
        /// Which limit.
        kind: LimitKind,
        /// Plain description.
        reason: String,
        /// The limit's value.
        limit: u64,
        /// The value that ran into it.
        observed: u64,
        /// Where the script was, innermost first.
        stack: Vec<StackLine>,
    },
    /// The code couldn't be loaded.
    Compile {
        /// Plain description.
        message: String,
        /// Where in the source.
        span: Span,
    },
    /// An engine bug or a misused API.
    Internal {
        /// Plain description.
        reason: String,
        /// Where the script was, innermost first.
        stack: Vec<StackLine>,
    },
}

impl RunError {
    /// The message or reason, whatever the kind.
    pub fn message(&self) -> &str {
        match self {
            RunError::Thrown { message, .. } => message,
            RunError::Abort { reason, .. } => reason,
            RunError::Compile { message, .. } => message,
            RunError::Internal { reason, .. } => reason,
        }
    }

    /// The stack, if the kind has one.
    pub fn stack(&self) -> &[StackLine] {
        match self {
            RunError::Thrown { stack, .. }
            | RunError::Abort { stack, .. }
            | RunError::Internal { stack, .. } => stack,
            RunError::Compile { .. } => &[],
        }
    }

    /// True for script exceptions, the only kind script may catch.
    pub fn is_catchable(&self) -> bool {
        matches!(self, RunError::Thrown { .. })
    }

    /// An internal error with no stack.
    pub fn internal(reason: impl Into<String>) -> RunError {
        RunError::Internal {
            reason: reason.into(),
            stack: Vec::new(),
        }
    }

    /// An abort from a heap limit, with no stack yet.
    pub fn from_abort(a: Abort) -> RunError {
        RunError::Abort {
            kind: a.kind,
            reason: a.reason,
            limit: a.limit,
            observed: a.observed,
            stack: Vec::new(),
        }
    }

    pub(crate) fn with_stack(mut self, lines: Vec<StackLine>) -> RunError {
        match &mut self {
            RunError::Thrown { stack, .. }
            | RunError::Abort { stack, .. }
            | RunError::Internal { stack, .. } => {
                if stack.is_empty() {
                    *stack = lines;
                }
            }
            RunError::Compile { .. } => {}
        }
        self
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Thrown { message, .. } => write!(f, "Uncaught {message}")?,
            RunError::Abort { reason, .. } => write!(f, "Stopped: {reason}")?,
            RunError::Compile { message, span } => {
                return write!(f, "{}:{}: {message}", span.line, span.column);
            }
            RunError::Internal { reason, .. } => write!(f, "Internal error: {reason}")?,
        }
        for line in self.stack() {
            write!(f, "\n    {line}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RunError {}

/// What a heap error turns into. Range errors are thrown by the caller
/// (they need an error object), so they come through as their own variant.
pub(crate) enum Lowered {
    Run(RunError),
    Range(String),
}

pub(crate) fn lower(e: HeapError) -> Lowered {
    match e {
        HeapError::Abort(a) => Lowered::Run(RunError::from_abort(a)),
        HeapError::Range(m) => Lowered::Range(m),
        HeapError::Internal(m) => Lowered::Run(RunError::internal(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_stack() {
        let e = RunError::Thrown {
            value: Value::UNDEFINED,
            message: "TypeError: boom".to_string(),
            stack: vec![
                StackLine {
                    function: "inner".to_string(),
                    line: 3,
                    column: 5,
                },
                StackLine {
                    function: "main".to_string(),
                    line: 0,
                    column: 0,
                },
            ],
        };
        assert_eq!(
            e.to_string(),
            "Uncaught TypeError: boom\n    at inner (3:5)\n    at main"
        );
        assert!(e.is_catchable());
        let a = RunError::from_abort(Abort {
            kind: LimitKind::Steps,
            reason: "step budget used up".to_string(),
            limit: 1,
            observed: 1,
        });
        assert!(!a.is_catchable());
        assert_eq!(a.to_string(), "Stopped: step budget used up");
        assert_eq!(RunError::internal("x").message(), "x");
    }
}
