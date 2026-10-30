// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Heap operations on `Map`, `Set`, `WeakMap` and `WeakSet` storage. Every
//! change goes through here so the bytes it owns are counted against
//! `max_heap_bytes`.

use super::{Heap, HeapError};
use crate::cinder::value::collection::{CollectionKind, MapData, MapKey};
use crate::cinder::value::object::ObjectClass;
use crate::cinder::value::{ObjectRef, Value};

impl Heap {
    /// Make an empty collection object.
    pub fn new_collection(
        &mut self,
        proto: Option<ObjectRef>,
        kind: CollectionKind,
    ) -> Result<ObjectRef, HeapError> {
        self.alloc_object(proto, ObjectClass::Collection(Box::new(MapData::new(kind))))
    }

    /// The SameValueZero key for `v`.
    pub fn map_key(&mut self, v: Value) -> Result<MapKey, HeapError> {
        Ok(if let Some(n) = v.as_number() {
            MapKey::number(n)
        } else if let Some(s) = v.as_string() {
            MapKey::String(self.string_units(s)?.into())
        } else if let Some(o) = v.as_object() {
            MapKey::Object(o)
        } else if let Some(y) = v.as_symbol() {
            MapKey::Symbol(y)
        } else if let Some(b) = v.as_bool() {
            MapKey::Bool(b)
        } else if v.is_null() {
            MapKey::Null
        } else {
            MapKey::Undefined
        })
    }

    /// The collection data of `obj`, if it is one.
    pub fn collection(&self, obj: ObjectRef) -> Result<Option<&MapData>, HeapError> {
        Ok(match &self.object(obj)?.class {
            ObjectClass::Collection(m) => Some(m),
            _ => None,
        })
    }

    /// The kind of collection `obj` is, if it is one.
    pub fn collection_kind(&self, obj: ObjectRef) -> Result<Option<CollectionKind>, HeapError> {
        Ok(self.collection(obj)?.map(|m| m.kind))
    }

    /// Change a collection, counting the bytes it gains or loses. Returns
    /// `None` when `obj` isn't a collection.
    pub fn with_collection<T>(
        &mut self,
        obj: ObjectRef,
        f: impl FnOnce(&mut MapData) -> T,
    ) -> Result<Option<T>, HeapError> {
        let (before, after, out) = match &mut self.object_mut(obj)?.class {
            ObjectClass::Collection(m) => {
                let before = m.owned_bytes();
                let out = f(m);
                (before, m.owned_bytes(), out)
            }
            _ => return Ok(None),
        };
        self.adjust(before, after)?;
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cinder::limits::Limits;

    #[test]
    fn bytes_follow_the_entries() {
        let mut h = Heap::new(Limits::default()).unwrap();
        let m = h.new_collection(None, CollectionKind::Map).unwrap();
        let start = h.bytes();
        for i in 0..100 {
            let k = h.map_key(Value::int32(i)).unwrap();
            h.with_collection(m, |d| d.set(k, Value::int32(i), Value::NULL))
                .unwrap();
        }
        assert!(h.bytes() > start + 100 * 16);
        assert_eq!(h.bytes(), h.recount_bytes());
        h.with_collection(m, |d| d.clear()).unwrap();
        assert_eq!(h.bytes(), h.recount_bytes());
    }

    #[test]
    fn weak_entries_die_with_their_keys() {
        let mut h = Heap::new(Limits::default()).unwrap();
        let w = h.new_collection(None, CollectionKind::WeakMap).unwrap();
        let k1 = h.new_object(None).unwrap();
        let k2 = h.new_object(None).unwrap();
        let v2 = h.new_object(None).unwrap();
        for (k, v) in [(k1, Value::int32(1)), (k2, Value::object(v2))] {
            let mk = h.map_key(Value::object(k)).unwrap();
            h.with_collection(w, |d| d.set(mk, Value::object(k), v))
                .unwrap();
        }
        // The value of a live key stays alive through the weak map alone.
        h.collect(&[Value::object(w), Value::object(k1)]);
        assert_eq!(h.collection(w).unwrap().unwrap().size(), 1);
        assert!(h.is_live(k1));
        assert!(!h.is_live(k2));
        assert!(!h.is_live(v2));
        assert_eq!(h.bytes(), h.recount_bytes());
    }

    #[test]
    fn a_value_can_keep_another_key_alive() {
        let mut h = Heap::new(Limits::default()).unwrap();
        let w = h.new_collection(None, CollectionKind::WeakMap).unwrap();
        let a = h.new_object(None).unwrap();
        let b = h.new_object(None).unwrap();
        // w[a] = b, w[b] = 7: with only `a` rooted, both entries live.
        for (k, v) in [(a, Value::object(b)), (b, Value::int32(7))] {
            let mk = h.map_key(Value::object(k)).unwrap();
            h.with_collection(w, |d| d.set(mk, Value::object(k), v))
                .unwrap();
        }
        h.collect(&[Value::object(w), Value::object(a)]);
        assert_eq!(h.collection(w).unwrap().unwrap().size(), 2);
        h.collect(&[Value::object(w)]);
        assert_eq!(h.collection(w).unwrap().unwrap().size(), 0);
    }
}
