// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Engine limits and how they get reported.
//!
//! Every limit the engine enforces lives in [`Limits`], and every default is
//! in its [`Default`] impl, so the host can see and change all of them in one
//! place. When a limit is hit, the engine says which one ([`LimitKind`]),
//! what the limit was, and what value ran into it. Nothing is cut off quietly.

use std::fmt;

/// All the knobs the engine checks. Build one with `Limits::default()` and
/// change the fields you care about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Hard cap on heap bytes. Past this, the script stops (uncatchable).
    pub max_heap_bytes: usize,
    /// Heap size that triggers the first collection.
    pub initial_gc_trigger: usize,
    /// After a collection, the next one triggers at `live * percent / 100`.
    /// Must be at least 100.
    pub gc_growth_percent: u32,
    /// Step budget for one run, as a backstop against scripts stuck in a
    /// loop. `None` turns it off and leaves stopping to the host's
    /// interrupt check. Running out stops the script (uncatchable).
    pub max_steps: Option<u64>,
    /// How many steps go by between two host interrupt checks.
    pub interrupt_check_interval: u32,
    /// Deepest script call stack, in frames. Past it, a `RangeError` is thrown.
    pub max_call_depth: u32,
    /// How deep builtins may call back into script (each level uses the Rust
    /// stack). Past it, a `RangeError` is thrown.
    pub max_native_reentry: u32,
    /// Longest string, in UTF-16 code units. Past it, a `RangeError` is thrown.
    pub max_string_length: usize,
    /// Strings shorter than this are joined right away instead of making a
    /// rope node.
    pub rope_min_length: usize,
    /// An object with more keys than this switches to dictionary mode.
    pub dictionary_threshold: u32,
    /// A shape with this many child shapes sends further additions to
    /// dictionary mode.
    pub max_transitions_per_shape: u32,
    /// Biggest script the compiler accepts, in bytes (scripts and `eval`
    /// code alike). Past it, compiling fails with a message naming this
    /// limit.
    pub max_source_bytes: usize,
    /// Deepest nesting of statements and expressions the compiler walks.
    /// The parser's own depth limit normally stops a program first; this
    /// one guards the compiler's recursion on its own. Past it, compiling
    /// fails with a message naming this limit.
    pub max_compile_depth: u32,
    /// Most registers one compiled function may use.
    pub max_registers_per_function: u32,
    /// Deepest nesting `JSON.parse` and `JSON.stringify` will walk. Past it,
    /// parse throws a `SyntaxError` and stringify a `RangeError`.
    pub max_json_depth: u32,
    /// Deepest nesting of arrays inside arrays that turning an array into a
    /// string (`join`, `toString`, `+` with a string) will walk. Past it, a
    /// `RangeError` is thrown.
    pub max_join_depth: u32,
    /// Most stack lines kept on an error.
    pub max_stack_lines: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_heap_bytes: 1 << 30,
            initial_gc_trigger: 8 << 20,
            gc_growth_percent: 200,
            max_steps: Some(1_000_000_000),
            interrupt_check_interval: 4096,
            max_call_depth: 10_000,
            max_native_reentry: 256,
            max_string_length: (1 << 30) - 1,
            rope_min_length: 64,
            dictionary_threshold: 64,
            max_transitions_per_shape: 32,
            max_source_bytes: 64 << 20,
            max_compile_depth: 2_500,
            max_registers_per_function: 65_535,
            max_json_depth: 10_000,
            max_join_depth: 10_000,
            max_stack_lines: 64,
        }
    }
}

impl Limits {
    /// Check that the settings make sense together. Returns a plain
    /// description of the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        if self.gc_growth_percent < 100 {
            return Err(format!(
                "gc_growth_percent is {}, it must be at least 100",
                self.gc_growth_percent
            ));
        }
        if self.initial_gc_trigger > self.max_heap_bytes {
            return Err(format!(
                "initial_gc_trigger ({} bytes) is above max_heap_bytes ({} bytes)",
                self.initial_gc_trigger, self.max_heap_bytes
            ));
        }
        if self.max_steps == Some(0) {
            return Err(
                "max_steps is 0, so no script could run; use None to turn it off".to_string(),
            );
        }
        if self.interrupt_check_interval == 0 {
            return Err("interrupt_check_interval must be at least 1".to_string());
        }
        if self.max_call_depth == 0 {
            return Err("max_call_depth must be at least 1".to_string());
        }
        if self.max_compile_depth == 0 || self.max_compile_depth > MAX_COMPILE_DEPTH {
            return Err(format!(
                "max_compile_depth is {}, it must be between 1 and {}",
                self.max_compile_depth, MAX_COMPILE_DEPTH
            ));
        }
        if self.max_registers_per_function > 65_535 {
            return Err(format!(
                "max_registers_per_function is {}, registers are 16-bit so the most is 65535",
                self.max_registers_per_function
            ));
        }
        Ok(())
    }
}

/// Highest `max_compile_depth` accepted. The compiler runs on a thread
/// whose stack is sized from the depth, so this caps that stack.
pub const MAX_COMPILE_DEPTH: u32 = 100_000;

/// Which limit was hit.
///
/// Only `RunError::Abort` carries a kind. Limits that end in an exception
/// script can catch (call depth, native re-entry, JSON and join depth) or
/// in a compile error (source bytes, compile depth, registers) say which
/// limit they hit in their message instead. Those kinds are here so every
/// limit has a name a host can match on; `CallDepth`, `NativeReentry`,
/// `SourceBytes`, `JsonDepth`, `JoinDepth` and `CompileDepth` are not
/// raised as an abort today and are kept for hosts that turn those limits
/// into aborts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// `max_heap_bytes`.
    HeapBytes,
    /// `max_steps`.
    Steps,
    /// The host asked the script to stop.
    Interrupt,
    /// `max_call_depth`.
    CallDepth,
    /// `max_native_reentry`.
    NativeReentry,
    /// `max_string_length`.
    StringLength,
    /// The language's array length limit (2^32 - 1).
    ArrayLength,
    /// `max_registers_per_function`.
    Registers,
    /// `max_source_bytes`.
    SourceBytes,
    /// `max_json_depth`.
    JsonDepth,
    /// `max_join_depth`.
    JoinDepth,
    /// `max_compile_depth`.
    CompileDepth,
    /// The heap ran out of slot indices for one kind of cell.
    HandleSpace,
}

impl LimitKind {
    /// The setting name for this limit, as it appears in [`Limits`].
    pub fn name(self) -> &'static str {
        match self {
            LimitKind::HeapBytes => "max_heap_bytes",
            LimitKind::Steps => "max_steps",
            LimitKind::Interrupt => "host interrupt",
            LimitKind::CallDepth => "max_call_depth",
            LimitKind::NativeReentry => "max_native_reentry",
            LimitKind::StringLength => "max_string_length",
            LimitKind::ArrayLength => "array length",
            LimitKind::Registers => "max_registers_per_function",
            LimitKind::SourceBytes => "max_source_bytes",
            LimitKind::JsonDepth => "max_json_depth",
            LimitKind::JoinDepth => "max_join_depth",
            LimitKind::CompileDepth => "max_compile_depth",
            LimitKind::HandleSpace => "handle space",
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Format a count with thousands separators, so messages read easily.
pub fn group_digits(n: u64) -> String {
    let raw = n.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (i, ch) in raw.chars().enumerate() {
        if i > 0 && (raw.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let limits = Limits::default();
        assert_eq!(limits.validate(), Ok(()));
        assert_eq!(limits.max_heap_bytes, 1 << 30);
        assert_eq!(limits.max_call_depth, 10_000);
        assert_eq!(limits.max_native_reentry, 256);
        assert_eq!(limits.max_steps, Some(1_000_000_000));
        assert_eq!(limits.interrupt_check_interval, 4096);
        assert_eq!(limits.max_json_depth, 10_000);
        assert_eq!(limits.max_join_depth, 10_000);
        assert_eq!(limits.max_compile_depth, 2_500);
    }

    #[test]
    fn compile_depth_must_be_in_range() {
        for bad in [0, MAX_COMPILE_DEPTH + 1] {
            let limits = Limits {
                max_compile_depth: bad,
                ..Limits::default()
            };
            let err = limits.validate().unwrap_err();
            assert_eq!(
                err,
                format!("max_compile_depth is {bad}, it must be between 1 and 100000")
            );
        }
    }

    #[test]
    fn bad_settings_say_why() {
        let limits = Limits {
            gc_growth_percent: 50,
            ..Limits::default()
        };
        let err = limits.validate().unwrap_err();
        assert!(err.contains("gc_growth_percent is 50"), "{err}");

        let limits = Limits {
            max_heap_bytes: 1024,
            ..Limits::default()
        };
        let err = limits.validate().unwrap_err();
        assert!(err.contains("above max_heap_bytes"), "{err}");

        let limits = Limits {
            max_registers_per_function: 70_000,
            ..Limits::default()
        };
        assert!(limits.validate().is_err());
    }

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1_073_741_824), "1,073,741,824");
    }
}
