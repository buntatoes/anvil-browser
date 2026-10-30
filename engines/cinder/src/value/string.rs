// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! UTF-16 strings and ropes.
//!
//! A string is a sequence of 16-bit code units, as the language defines it:
//! `"\u{1F600}".length` is 2, and each half of a surrogate pair is its own unit.
//!
//! Joining two long strings makes a [`JsString::Rope`] node that points at
//! both halves instead of copying them, so `s += x` in a loop stays linear.
//! The first time someone needs the actual units, the rope is flattened in
//! place (the handle stays the same). Flattening walks the rope with an
//! explicit stack, so a rope a million levels deep can't overflow the Rust
//! stack.

use super::StringRef;
use crate::cinder::gc::arena::{Arena, ArenaError};

/// A string cell on the heap.
#[derive(Debug, Clone)]
pub enum JsString {
    /// The units themselves.
    Flat(Vec<u16>),
    /// `left` followed by `right`, not joined yet.
    Rope {
        /// First part.
        left: StringRef,
        /// Second part.
        right: StringRef,
        /// Total length in code units.
        len: usize,
        /// Longest path down to a flat string.
        depth: u32,
    },
}

impl JsString {
    /// A flat string from a Rust string.
    pub fn from_str_units(s: &str) -> JsString {
        JsString::Flat(s.encode_utf16().collect())
    }

    /// Length in code units.
    pub fn len(&self) -> usize {
        match self {
            JsString::Flat(u) => u.len(),
            JsString::Rope { len, .. } => *len,
        }
    }

    /// True for the empty string.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rope depth (0 for a flat string).
    pub fn depth(&self) -> u32 {
        match self {
            JsString::Flat(_) => 0,
            JsString::Rope { depth, .. } => *depth,
        }
    }

    /// The units, if this string is flat.
    pub fn flat_units(&self) -> Option<&[u16]> {
        match self {
            JsString::Flat(u) => Some(u),
            JsString::Rope { .. } => None,
        }
    }

    /// Estimated bytes this cell owns beyond its arena slot.
    pub fn owned_bytes(&self) -> usize {
        match self {
            JsString::Flat(u) => u.capacity() * 2,
            JsString::Rope { .. } => 0,
        }
    }
}

/// All units of `root`, walking ropes with an explicit stack.
pub fn collect_units(arena: &Arena<JsString>, root: StringRef) -> Result<Vec<u16>, ArenaError> {
    let total = arena.get(root.0)?.len();
    let mut out = Vec::with_capacity(total);
    let mut stack = vec![root];
    while let Some(r) = stack.pop() {
        match arena.get(r.0)? {
            JsString::Flat(units) => out.extend_from_slice(units),
            JsString::Rope { left, right, .. } => {
                stack.push(*right);
                stack.push(*left);
            }
        }
    }
    Ok(out)
}

/// The code unit at `index`, without flattening. Walks down the rope.
pub fn unit_at(
    arena: &Arena<JsString>,
    root: StringRef,
    index: usize,
) -> Result<Option<u16>, ArenaError> {
    let mut r = root;
    let mut i = index;
    loop {
        match arena.get(r.0)? {
            JsString::Flat(units) => return Ok(units.get(i).copied()),
            JsString::Rope {
                left, right, len, ..
            } => {
                if i >= *len {
                    return Ok(None);
                }
                let left_len = arena.get(left.0)?.len();
                if i < left_len {
                    r = *left;
                } else {
                    i -= left_len;
                    r = *right;
                }
            }
        }
    }
}

/// Turn UTF-16 into a Rust string. Lone surrogates become U+FFFD, which only
/// happens on the way out to the host.
pub fn units_to_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// Compare two strings by code units, the way `<` does for strings.
pub fn compare_units(a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(arena: &mut Arena<JsString>, s: &str) -> StringRef {
        StringRef(arena.insert(JsString::from_str_units(s)).unwrap())
    }

    fn rope(arena: &mut Arena<JsString>, l: StringRef, r: StringRef) -> StringRef {
        let ll = arena.get(l.0).unwrap();
        let rr = arena.get(r.0).unwrap();
        let node = JsString::Rope {
            left: l,
            right: r,
            len: ll.len() + rr.len(),
            depth: ll.depth().max(rr.depth()) + 1,
        };
        StringRef(arena.insert(node).unwrap())
    }

    #[test]
    fn length_counts_code_units() {
        assert_eq!(JsString::from_str_units("abc").len(), 3);
        assert_eq!(JsString::from_str_units("\u{1F600}").len(), 2);
        assert_eq!(JsString::from_str_units("é").len(), 1);
        assert!(JsString::from_str_units("").is_empty());
        let s = JsString::from_str_units("\u{1F600}");
        assert_eq!(s.flat_units(), Some(&[0xD83D, 0xDE00][..]));
    }

    #[test]
    fn lone_surrogates_survive_until_output() {
        let units = vec![0x61, 0xD800, 0x62];
        assert_eq!(units_to_string(&units), "a\u{FFFD}b");
        let mut arena = Arena::new();
        let hi = StringRef(arena.insert(JsString::Flat(vec![0xD83D])).unwrap());
        let lo = StringRef(arena.insert(JsString::Flat(vec![0xDE00])).unwrap());
        let both = rope(&mut arena, hi, lo);
        let joined = collect_units(&arena, both).unwrap();
        assert_eq!(units_to_string(&joined), "\u{1F600}");
    }

    #[test]
    fn ropes_read_in_order() {
        let mut arena = Arena::new();
        let a = flat(&mut arena, "ab");
        let b = flat(&mut arena, "cd");
        let c = flat(&mut arena, "e");
        let ab = rope(&mut arena, a, b);
        let abc = rope(&mut arena, ab, c);
        let cab = rope(&mut arena, c, ab);
        assert_eq!(
            units_to_string(&collect_units(&arena, abc).unwrap()),
            "abcde"
        );
        assert_eq!(
            units_to_string(&collect_units(&arena, cab).unwrap()),
            "eabcd"
        );
        for (i, ch) in "abcde".encode_utf16().enumerate() {
            assert_eq!(unit_at(&arena, abc, i).unwrap(), Some(ch));
        }
        assert_eq!(unit_at(&arena, abc, 5).unwrap(), None);
        assert_eq!(arena.get(abc.0).unwrap().depth(), 2);
    }

    #[test]
    fn deep_rope_does_not_use_the_rust_stack() {
        let mut arena = Arena::new();
        let mut s = flat(&mut arena, "x");
        for _ in 0..200_000 {
            let piece = flat(&mut arena, "y");
            s = rope(&mut arena, s, piece);
        }
        let units = collect_units(&arena, s).unwrap();
        assert_eq!(units.len(), 200_001);
        assert_eq!(unit_at(&arena, s, 0).unwrap(), Some(u16::from(b'x')));
    }

    #[test]
    fn compare_is_by_code_unit() {
        let a: Vec<u16> = "a".encode_utf16().collect();
        let b: Vec<u16> = "b".encode_utf16().collect();
        assert_eq!(compare_units(&a, &b), std::cmp::Ordering::Less);
        // U+FF61 sorts after the high surrogate of U+1F600 by code unit.
        let ff: Vec<u16> = "\u{FF61}".encode_utf16().collect();
        let pair: Vec<u16> = "\u{1F600}".encode_utf16().collect();
        assert_eq!(compare_units(&pair, &ff), std::cmp::Ordering::Less);
    }
}
