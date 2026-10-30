// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Atoms and property keys.
//!
//! An [`Atom`] is an interned string used as a property name, so comparing
//! two keys is comparing two integers. Keys that are array indices
//! (`"0"` to `"4294967294"`, written the canonical way) become
//! [`PropertyKey::Index`] instead, and go to element storage.
//!
//! Atoms live for as long as the heap does. Their bytes count toward the
//! heap budget.

use super::SymbolRef;
use std::collections::HashMap;

/// An interned property name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Atom(pub u32);

/// The largest array index. `2^32 - 1` itself is not an index, it is the
/// largest possible length.
pub const MAX_ARRAY_INDEX: u32 = u32::MAX - 1;

/// A property key after it has been sorted into its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    /// An array index.
    Index(u32),
    /// Any other string key.
    Atom(Atom),
    /// A symbol key.
    Symbol(SymbolRef),
}

impl PropertyKey {
    /// True for symbol keys.
    pub fn is_symbol(self) -> bool {
        matches!(self, PropertyKey::Symbol(_))
    }
}

/// If `units` is the canonical spelling of an array index, return it.
/// `"0"` is an index; `"00"`, `"+1"`, `"1.0"` and `"4294967295"` are not.
pub fn array_index(units: &[u16]) -> Option<u32> {
    if units.is_empty() || units.len() > 10 {
        return None;
    }
    if units[0] == u16::from(b'0') {
        return if units.len() == 1 { Some(0) } else { None };
    }
    let mut n: u64 = 0;
    for &u in units {
        if !(u16::from(b'0')..=u16::from(b'9')).contains(&u) {
            return None;
        }
        n = n * 10 + u64::from(u - u16::from(b'0'));
    }
    if n <= u64::from(MAX_ARRAY_INDEX) {
        Some(n as u32)
    } else {
        None
    }
}

/// Bytes one atom costs on top of its code units (table entry and map entry).
const ATOM_OVERHEAD: usize = 2 * std::mem::size_of::<Vec<u16>>() + 16;

/// The table of interned names.
#[derive(Debug, Default)]
pub struct AtomTable {
    map: HashMap<Vec<u16>, Atom>,
    names: Vec<Vec<u16>>,
    bytes: usize,
}

impl AtomTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of atoms.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True when there are no atoms.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Bytes the table uses, as counted toward the heap budget.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The atom for `units`, if it was interned already.
    pub fn find(&self, units: &[u16]) -> Option<Atom> {
        self.map.get(units).copied()
    }

    /// What interning `units` would add to [`AtomTable::bytes`]. Zero when
    /// it is already there.
    pub fn cost(&self, units: &[u16]) -> usize {
        if self.map.contains_key(units) {
            0
        } else {
            ATOM_OVERHEAD + units.len() * 4
        }
    }

    /// Intern `units`. Fails only when all `u32` atom numbers are used.
    pub fn intern(&mut self, units: &[u16]) -> Result<Atom, String> {
        if let Some(a) = self.map.get(units) {
            return Ok(*a);
        }
        let id = u32::try_from(self.names.len())
            .map_err(|_| "all 4,294,967,296 atom numbers are in use".to_string())?;
        let atom = Atom(id);
        self.map.insert(units.to_vec(), atom);
        self.names.push(units.to_vec());
        self.bytes += ATOM_OVERHEAD + units.len() * 4;
        Ok(atom)
    }

    /// The code units of an atom.
    pub fn units(&self, atom: Atom) -> Option<&[u16]> {
        self.names.get(atom.0 as usize).map(Vec::as_slice)
    }

    /// The atom as a Rust string (lone surrogates become U+FFFD).
    pub fn to_rust_string(&self, atom: Atom) -> Option<String> {
        self.units(atom).map(String::from_utf16_lossy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn index_keys_are_canonical_only() {
        assert_eq!(array_index(&u("0")), Some(0));
        assert_eq!(array_index(&u("7")), Some(7));
        assert_eq!(array_index(&u("4294967294")), Some(4294967294));
        for bad in [
            "",
            "00",
            "01",
            "+1",
            "-1",
            "1.0",
            "4294967295",
            "99999999999",
            "1e3",
            "a",
        ] {
            assert_eq!(array_index(&u(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn interning_is_stable() {
        let mut t = AtomTable::new();
        assert!(t.is_empty());
        let a = t.intern(&u("x")).unwrap();
        let b = t.intern(&u("y")).unwrap();
        let a2 = t.intern(&u("x")).unwrap();
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(t.len(), 2);
        assert_eq!(t.find(&u("y")), Some(b));
        assert_eq!(t.find(&u("z")), None);
        assert_eq!(t.to_rust_string(b).as_deref(), Some("y"));
        assert_eq!(t.cost(&u("x")), 0);
        let before = t.bytes();
        let cost = t.cost(&u("longer name"));
        t.intern(&u("longer name")).unwrap();
        assert_eq!(t.bytes(), before + cost);
    }

    #[test]
    fn atoms_keep_utf16_exactly() {
        let mut t = AtomTable::new();
        let lone = [0xD800u16];
        let a = t.intern(&lone).unwrap();
        assert_eq!(t.units(a), Some(&lone[..]));
        assert_eq!(t.to_rust_string(a).as_deref(), Some("\u{FFFD}"));
    }
}
