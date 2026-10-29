// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The storage behind `Map`, `Set`, `WeakMap` and `WeakSet`.
//!
//! Entries sit in a `Vec` in insertion order. Deleting one leaves a gap
//! (`None`) so that live iterators, which hold a position, keep their place
//! and still see entries added after them. The gaps are squeezed out only
//! while no iterator is open on the collection; the collector counts
//! iterators that died without finishing, so a dropped iterator doesn't
//! hold the gaps forever.
//!
//! Keys are compared with SameValueZero: `-0` and `0` are the same key,
//! `NaN` is equal to itself, strings compare by content and objects by
//! identity. The index keys strings by their code units.

use super::{ObjectRef, SymbolRef, Value};
use std::collections::HashMap;

/// Which collection an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionKind {
    /// `Map`.
    Map,
    /// `Set`.
    Set,
    /// `WeakMap`: entries live only as long as their key.
    WeakMap,
    /// `WeakSet`.
    WeakSet,
}

impl CollectionKind {
    /// True for the weak kinds.
    pub fn is_weak(self) -> bool {
        matches!(self, CollectionKind::WeakMap | CollectionKind::WeakSet)
    }

    /// The constructor name.
    pub fn name(self) -> &'static str {
        match self {
            CollectionKind::Map => "Map",
            CollectionKind::Set => "Set",
            CollectionKind::WeakMap => "WeakMap",
            CollectionKind::WeakSet => "WeakSet",
        }
    }
}

/// A key as the index sees it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MapKey {
    /// `undefined`.
    Undefined,
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number, by its bits after folding `-0` into `0` and every `NaN`
    /// into one.
    Number(u64),
    /// A string, by content.
    String(Box<[u16]>),
    /// A symbol, by identity.
    Symbol(SymbolRef),
    /// An object, by identity.
    Object(ObjectRef),
}

impl MapKey {
    /// The key for a number.
    pub fn number(n: f64) -> MapKey {
        let n = if n == 0.0 { 0.0 } else { n };
        let bits = if n.is_nan() {
            super::CANONICAL_NAN_BITS
        } else {
            n.to_bits()
        };
        MapKey::Number(bits)
    }

    fn bytes(&self) -> usize {
        match self {
            MapKey::String(s) => s.len() * 2,
            _ => 0,
        }
    }
}

/// The entries of one collection.
#[derive(Clone, Debug)]
pub struct MapData {
    /// Which collection.
    pub kind: CollectionKind,
    /// Entries in insertion order; `None` where one was deleted.
    pub entries: Vec<Option<(Value, Value)>>,
    /// Key to position in `entries`.
    pub index: HashMap<MapKey, usize>,
    /// Iterators open on this collection.
    pub open_iterators: u32,
    string_bytes: usize,
}

/// Bytes charged per entry, on top of the vector slot: the index entry and
/// its hashing overhead.
const ENTRY_OVERHEAD: usize = 48;

impl MapData {
    /// An empty collection.
    pub fn new(kind: CollectionKind) -> MapData {
        MapData {
            kind,
            entries: Vec::new(),
            index: HashMap::new(),
            open_iterators: 0,
            string_bytes: 0,
        }
    }

    /// Live entries.
    pub fn size(&self) -> usize {
        self.index.len()
    }

    /// The value for `key`.
    pub fn get(&self, key: &MapKey) -> Option<Value> {
        let i = *self.index.get(key)?;
        self.entries.get(i).and_then(|e| e.map(|(_, v)| v))
    }

    /// True when `key` is present.
    pub fn has(&self, key: &MapKey) -> bool {
        self.index.contains_key(key)
    }

    /// Add or replace. A new key goes at the end.
    pub fn set(&mut self, key: MapKey, k: Value, v: Value) {
        if let Some(&i) = self.index.get(&key) {
            if let Some(Some(e)) = self.entries.get_mut(i) {
                e.1 = v;
            }
            return;
        }
        if self.open_iterators == 0 && self.entries.len() >= 16 {
            let gaps = self.entries.len() - self.index.len();
            if gaps > self.index.len() {
                self.compact();
            }
        }
        self.string_bytes += key.bytes();
        self.index.insert(key, self.entries.len());
        self.entries.push(Some((k, v)));
    }

    /// Remove `key`. True when it was there.
    pub fn delete(&mut self, key: &MapKey) -> bool {
        match self.index.remove(key) {
            Some(i) => {
                self.string_bytes -= key.bytes();
                if let Some(e) = self.entries.get_mut(i) {
                    *e = None;
                }
                true
            }
            None => false,
        }
    }

    /// Remove everything. Open iterators see the end.
    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            *e = None;
        }
        self.index.clear();
        self.string_bytes = 0;
        if self.open_iterators == 0 {
            self.entries.clear();
        }
    }

    /// Squeeze out the gaps. Only safe with no open iterators.
    pub fn compact(&mut self) {
        let old = std::mem::take(&mut self.entries);
        let mut moved = vec![0usize; old.len()];
        for (i, e) in old.into_iter().enumerate() {
            if let Some(e) = e {
                moved[i] = self.entries.len();
                self.entries.push(Some(e));
            }
        }
        for pos in self.index.values_mut() {
            *pos = moved[*pos];
        }
    }

    /// Keep only the entries `keep` says yes to (used by the collector for
    /// weak collections, which have no iterators).
    pub fn retain_keys(&mut self, mut keep: impl FnMut(Value) -> bool) {
        let mut dead = Vec::new();
        for (k, pos) in &self.index {
            if let Some(Some((key, _))) = self.entries.get(*pos)
                && !keep(*key)
            {
                dead.push(k.clone());
            }
        }
        for k in dead {
            self.delete(&k);
        }
        if self.open_iterators == 0 {
            self.compact();
        }
    }

    /// The entry at `pos` or the next live one after it, with its position.
    pub fn next_from(&self, mut pos: usize) -> Option<(usize, Value, Value)> {
        while pos < self.entries.len() {
            if let Some(Some((k, v))) = self.entries.get(pos) {
                return Some((pos, *k, *v));
            }
            pos += 1;
        }
        None
    }

    /// Every value held, for tracing a strong collection.
    pub fn for_each_value(&self, mut f: impl FnMut(Value)) {
        for (k, v) in self.entries.iter().flatten() {
            f(*k);
            f(*v);
        }
    }

    /// Estimated bytes owned.
    pub fn owned_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Option<(Value, Value)>>()
            + self.index.len() * ENTRY_OVERHEAD
            + self.string_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_value_zero() {
        assert_eq!(MapKey::number(0.0), MapKey::number(-0.0));
        assert_eq!(MapKey::number(f64::NAN), MapKey::number(-f64::NAN));
        assert_ne!(MapKey::number(1.0), MapKey::number(2.0));
    }

    #[test]
    fn order_gaps_and_compaction() {
        let mut m = MapData::new(CollectionKind::Map);
        for i in 0..40 {
            m.set(
                MapKey::number(f64::from(i)),
                Value::int32(i),
                Value::int32(i),
            );
        }
        for i in 0..30 {
            assert!(m.delete(&MapKey::number(f64::from(i))));
        }
        assert_eq!(m.size(), 10);
        assert_eq!(m.next_from(0).map(|e| e.0), Some(30));
        // An open iterator blocks compaction.
        m.open_iterators = 1;
        m.set(MapKey::number(100.0), Value::int32(100), Value::UNDEFINED);
        assert_eq!(m.entries.len(), 41);
        m.open_iterators = 0;
        m.set(MapKey::number(101.0), Value::int32(101), Value::UNDEFINED);
        assert_eq!(m.entries.len(), 12);
        assert_eq!(
            m.get(&MapKey::number(35.0)).and_then(Value::as_int32),
            Some(35)
        );
        assert_eq!(m.next_from(0).map(|e| e.1.as_int32()), Some(Some(30)));
    }

    #[test]
    fn clear_keeps_iterator_positions_valid() {
        let mut m = MapData::new(CollectionKind::Set);
        m.set(MapKey::Null, Value::NULL, Value::NULL);
        m.open_iterators = 1;
        m.clear();
        assert_eq!(m.entries.len(), 1);
        assert!(m.next_from(0).is_none());
        m.set(MapKey::Bool(true), Value::TRUE, Value::TRUE);
        assert_eq!(m.next_from(0).map(|e| e.0), Some(1));
    }
}
