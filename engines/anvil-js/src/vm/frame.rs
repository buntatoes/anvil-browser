// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Call frames.
//!
//! Frames live in a `Vec` on the VM, and every frame's registers are a
//! window of one shared, growable register stack. A call from script to
//! script pushes a frame and a window; a return pops them. The Rust stack
//! doesn't grow, so call depth is a setting (`max_call_depth`), not a crash.

use crate::anvil_js::value::{ObjectRef, Value};

/// Where a frame's result goes when it doesn't go into a register.
pub(crate) const DISCARD: u32 = u32::MAX;

/// How a frame was entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameKind {
    /// A plain call.
    Normal,
    /// A `new` call: a non-object result is replaced by `this`.
    Construct,
}

/// One activation of a script function.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    /// Loaded code index.
    pub code: u32,
    /// Next op to run. While an op runs, the op itself is at `pc - 1`.
    pub pc: u32,
    /// First register of this frame's window.
    pub base: usize,
    /// Register in the caller that gets the result, or [`DISCARD`].
    pub dst: u32,
    /// `this`.
    pub this: Value,
    /// The function object (undefined for top-level code).
    pub callee: Value,
    /// Innermost environment.
    pub env: Option<ObjectRef>,
    /// Environments pushed by this frame (not counting the closure's).
    pub env_depth: u16,
    /// How it was entered.
    pub kind: FrameKind,
}
