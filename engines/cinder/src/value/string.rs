// SPDX-License-Identifier: Apache-2.0
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
use crate::gc::arena::{Arena, ArenaError};

/// Minimum code-unit length for creating a zero-copy [`JsString::Sliced`] view.
pub const SLICE_MIN_LEN: usize = 16;

/// A string cell on the heap.
#[derive(Debug, Clone)]
pub enum JsString {
    /// Compact 1-byte Latin-1 / ASCII code units (`<= 0xFF`).
    OneByte(Vec<u8>),
    /// Full 16-bit UTF-16 code units.
    Flat(Vec<u16>),
    /// Zero-copy substring view into `parent[offset..offset + len]`.
    Sliced {
        /// Backing string handle.
        parent: StringRef,
        /// Starting code-unit offset inside `parent`.
        offset: usize,
        /// Length in code units.
        len: usize,
    },
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
    /// A compact string from a Rust string (`OneByte` when all code units are Latin-1,
    /// `Flat` UTF-16 otherwise).
    pub fn from_str_units(s: &str) -> JsString {
        if s.is_ascii() {
            return JsString::OneByte(s.as_bytes().to_vec());
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        Self::from_units(units)
    }

    /// Build a compact string from UTF-16 code units (`OneByte` if all units `<= 0xFF`).
    pub fn from_units(units: Vec<u16>) -> JsString {
        if units.iter().all(|&u| u <= 0xFF) {
            let bytes: Vec<u8> = units.into_iter().map(|u| u as u8).collect();
            JsString::OneByte(bytes)
        } else {
            JsString::Flat(units)
        }
    }

    /// Length in code units.
    pub fn len(&self) -> usize {
        match self {
            JsString::OneByte(b) => b.len(),
            JsString::Flat(u) => u.len(),
            JsString::Sliced { len, .. } => *len,
            JsString::Rope { len, .. } => *len,
        }
    }

    /// True for the empty string.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rope depth (0 for non-rope strings).
    pub fn depth(&self) -> u32 {
        match self {
            JsString::OneByte(_) | JsString::Flat(_) | JsString::Sliced { .. } => 0,
            JsString::Rope { depth, .. } => *depth,
        }
    }

    /// True when stored in compact 1-byte Latin-1 encoding.
    pub fn is_one_byte(&self) -> bool {
        matches!(self, JsString::OneByte(_))
    }

    /// The Latin-1 bytes, if this string is `OneByte`.
    pub fn one_byte_units(&self) -> Option<&[u8]> {
        match self {
            JsString::OneByte(b) => Some(b),
            _ => None,
        }
    }

    /// The 16-bit units, if this string is `Flat`.
    pub fn flat_units(&self) -> Option<&[u16]> {
        match self {
            JsString::Flat(u) => Some(u),
            _ => None,
        }
    }

    /// True when the string is flat (`OneByte` or `Flat`, not a `Rope` or `Sliced`).
    pub fn is_flat_representation(&self) -> bool {
        matches!(self, JsString::OneByte(_) | JsString::Flat(_))
    }

    /// Estimated bytes this cell owns beyond its arena slot.
    pub fn owned_bytes(&self) -> usize {
        match self {
            JsString::OneByte(b) => b.capacity(),
            JsString::Flat(u) => u.capacity() * 2,
            JsString::Sliced { .. } | JsString::Rope { .. } => 0,
        }
    }
}

/// All units of `root`, walking ropes and slices with an explicit stack.
pub fn collect_units(arena: &Arena<JsString>, root: StringRef) -> Result<Vec<u16>, ArenaError> {
    let total = arena.get(root.0)?.len();
    let mut out = Vec::with_capacity(total);
    let mut stack: Vec<(StringRef, usize, usize)> = vec![(root, 0, total)];
    while let Some((r, offset, len)) = stack.pop() {
        if len == 0 {
            continue;
        }
        match arena.get(r.0)? {
            JsString::OneByte(bytes) => {
                let end = (offset + len).min(bytes.len());
                if offset < end {
                    out.extend(bytes[offset..end].iter().map(|&b| u16::from(b)));
                }
            }
            JsString::Flat(units) => {
                let end = (offset + len).min(units.len());
                if offset < end {
                    out.extend_from_slice(&units[offset..end]);
                }
            }
            JsString::Sliced {
                parent,
                offset: p_off,
                len: p_len,
            } => {
                let take = len.min(p_len.saturating_sub(offset));
                stack.push((*parent, p_off + offset, take));
            }
            JsString::Rope { left, right, .. } => {
                let left_len = arena.get(left.0)?.len();
                let end = offset + len;
                if end > left_len {
                    let right_start = offset.saturating_sub(left_len);
                    let right_end = end - left_len;
                    stack.push((*right, right_start, right_end - right_start));
                }
                if offset < left_len {
                    let left_end = end.min(left_len);
                    stack.push((*left, offset, left_end - offset));
                }
            }
        }
    }
    Ok(out)
}

/// The code unit at `index`, without flattening. Walks down ropes and slices.
pub fn unit_at(
    arena: &Arena<JsString>,
    root: StringRef,
    index: usize,
) -> Result<Option<u16>, ArenaError> {
    let mut r = root;
    let mut i = index;
    loop {
        match arena.get(r.0)? {
            JsString::OneByte(bytes) => return Ok(bytes.get(i).map(|&b| u16::from(b))),
            JsString::Flat(units) => return Ok(units.get(i).copied()),
            JsString::Sliced {
                parent,
                offset,
                len,
            } => {
                if i >= *len {
                    return Ok(None);
                }
                i += *offset;
                r = *parent;
            }
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

    #[test]
    fn one_byte_and_sliced_strings_work() {
        let ascii = JsString::from_str_units("hello world");
        assert!(ascii.is_one_byte());
        assert_eq!(ascii.one_byte_units(), Some(&b"hello world"[..]));
        let latin1 = JsString::from_str_units("café");
        assert!(latin1.is_one_byte());
        assert_eq!(latin1.len(), 4);

        let mut arena = Arena::new();
        let base = flat(&mut arena, "0123456789abcdefghijklmnopqrstuvwxyz");
        let sliced = StringRef(
            arena
                .insert(JsString::Sliced {
                    parent: base,
                    offset: 10,
                    len: 16,
                })
                .unwrap(),
        );
        assert_eq!(arena.get(sliced.0).unwrap().owned_bytes(), 0);
        assert_eq!(
            units_to_string(&collect_units(&arena, sliced).unwrap()),
            "abcdefghijklmnop"
        );
        assert_eq!(unit_at(&arena, sliced, 0).unwrap(), Some(u16::from(b'a')));
        assert_eq!(unit_at(&arena, sliced, 15).unwrap(), Some(u16::from(b'p')));
        assert_eq!(unit_at(&arena, sliced, 16).unwrap(), None);
    }
}
