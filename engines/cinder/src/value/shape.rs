// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Shapes (hidden classes).
//!
//! Objects that get the same keys, with the same attributes, in the same
//! order, from the same prototype share one [`Shape`]. A shape knows its
//! parent, the one key it added, that key's slot and attributes, and the
//! child shapes made from it (transitions).
//!
//! Each prototype has its own root shape, so two objects with the same shape
//! always have the same prototype. That lets an inline cache check one shape
//! instead of walking the chain.
//!
//! Looking up a key walks up the parents. Shapes with more than
//! [`TABLE_THRESHOLD`] keys build a hash table the first time they are
//! looked up, so big shapes stay fast.
//!
//! Transition edges don't keep their child shape alive. A child that dies
//! is noticed through its stale handle and the edge is dropped.

use super::atom::PropertyKey;
use super::{ObjectRef, ShapeRef};
use crate::cinder::gc::arena::{Arena, ArenaError};
use std::cell::OnceCell;
use std::collections::HashMap;

/// Property attribute bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Attrs(pub u8);

impl Attrs {
    /// `[[Writable]]` (data properties only).
    pub const WRITABLE: u8 = 1;
    /// `[[Enumerable]]`.
    pub const ENUMERABLE: u8 = 2;
    /// `[[Configurable]]`.
    pub const CONFIGURABLE: u8 = 4;
    /// The usual attributes of a property made by assignment.
    pub const DEFAULT: Attrs = Attrs(Self::WRITABLE | Self::ENUMERABLE | Self::CONFIGURABLE);
    /// No attributes set.
    pub const NONE: Attrs = Attrs(0);

    /// True when writable.
    pub fn writable(self) -> bool {
        self.0 & Self::WRITABLE != 0
    }
    /// True when enumerable.
    pub fn enumerable(self) -> bool {
        self.0 & Self::ENUMERABLE != 0
    }
    /// True when configurable.
    pub fn configurable(self) -> bool {
        self.0 & Self::CONFIGURABLE != 0
    }
}

/// Shapes with more keys than this get a lookup table.
pub const TABLE_THRESHOLD: u32 = 8;

/// One hidden class.
#[derive(Debug)]
pub struct Shape {
    /// The prototype every object with this shape has.
    pub proto: Option<ObjectRef>,
    /// The shape this one was made from (`None` for a root).
    pub parent: Option<ShapeRef>,
    /// The key this shape added (`None` for a root).
    pub key: Option<PropertyKey>,
    /// Attributes of `key`.
    pub attrs: Attrs,
    /// Slot of `key` in the object's slot list.
    pub slot: u32,
    /// Number of keys, counting the parents'.
    pub count: u32,
    /// Child shapes: `(key, attrs) -> child`.
    pub transitions: Vec<(PropertyKey, Attrs, ShapeRef)>,
    table: OnceCell<HashMap<PropertyKey, (u32, Attrs)>>,
}

impl Shape {
    /// A root shape for objects with prototype `proto`.
    pub fn root(proto: Option<ObjectRef>) -> Shape {
        Shape {
            proto,
            parent: None,
            key: None,
            attrs: Attrs::NONE,
            slot: 0,
            count: 0,
            transitions: Vec::new(),
            table: OnceCell::new(),
        }
    }

    /// A child of `parent` (whose cell is `parent_shape`) adding `key`.
    pub fn child(parent: ShapeRef, parent_shape: &Shape, key: PropertyKey, attrs: Attrs) -> Shape {
        Shape {
            proto: parent_shape.proto,
            parent: Some(parent),
            key: Some(key),
            attrs,
            slot: parent_shape.count,
            count: parent_shape.count + 1,
            transitions: Vec::new(),
            table: OnceCell::new(),
        }
    }

    /// Estimated bytes this shape owns beyond its arena slot. The lookup
    /// table is counted up front for big shapes, since it can be built
    /// through a shared reference later.
    pub fn owned_bytes(&self) -> usize {
        let edges =
            self.transitions.capacity() * std::mem::size_of::<(PropertyKey, Attrs, ShapeRef)>();
        let table = if self.count > TABLE_THRESHOLD {
            self.count as usize * (std::mem::size_of::<(PropertyKey, (u32, Attrs))>() + 8)
        } else {
            0
        };
        edges + table
    }

    /// The live child for `(key, attrs)`, if there is one.
    pub fn find_transition(
        &self,
        arena: &Arena<Shape>,
        key: PropertyKey,
        attrs: Attrs,
    ) -> Option<ShapeRef> {
        self.transitions
            .iter()
            .find(|(k, a, child)| *k == key && *a == attrs && arena.contains(child.0))
            .map(|(_, _, child)| *child)
    }
}

/// Find `key` in `shape`: its slot and attributes.
pub fn lookup(
    arena: &Arena<Shape>,
    shape: ShapeRef,
    key: PropertyKey,
) -> Result<Option<(u32, Attrs)>, ArenaError> {
    let s = arena.get(shape.0)?;
    if s.count > TABLE_THRESHOLD {
        if let Some(table) = s.table.get() {
            return Ok(table.get(&key).copied());
        }
        let mut table = HashMap::with_capacity(s.count as usize);
        let mut cur = Some(shape);
        while let Some(r) = cur {
            let c = arena.get(r.0)?;
            if let Some(k) = c.key {
                table.entry(k).or_insert((c.slot, c.attrs));
            }
            cur = c.parent;
        }
        let found = table.get(&key).copied();
        let _ = s.table.set(table);
        return Ok(found);
    }
    let mut cur = Some(shape);
    while let Some(r) = cur {
        let c = arena.get(r.0)?;
        if c.key == Some(key) {
            return Ok(Some((c.slot, c.attrs)));
        }
        cur = c.parent;
    }
    Ok(None)
}

/// All keys of `shape` in the order they were added, with slot and attributes.
pub fn keys_in_order(
    arena: &Arena<Shape>,
    shape: ShapeRef,
) -> Result<Vec<(PropertyKey, u32, Attrs)>, ArenaError> {
    let mut out = Vec::with_capacity(arena.get(shape.0)?.count as usize);
    let mut cur = Some(shape);
    while let Some(r) = cur {
        let c = arena.get(r.0)?;
        if let Some(k) = c.key {
            out.push((k, c.slot, c.attrs));
        }
        cur = c.parent;
    }
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::atom::Atom;
    use super::*;

    fn key(n: u32) -> PropertyKey {
        PropertyKey::Atom(Atom(n))
    }

    fn add(arena: &mut Arena<Shape>, parent: ShapeRef, k: PropertyKey) -> ShapeRef {
        let child = {
            let p = arena.get(parent.0).unwrap();
            Shape::child(parent, p, k, Attrs::DEFAULT)
        };
        let r = ShapeRef(arena.insert(child).unwrap());
        arena
            .get_mut(parent.0)
            .unwrap()
            .transitions
            .push((k, Attrs::DEFAULT, r));
        r
    }

    #[test]
    fn lookup_small_and_big_shapes() {
        let mut arena = Arena::new();
        let root = ShapeRef(arena.insert(Shape::root(None)).unwrap());
        let mut s = root;
        for i in 0..20 {
            s = add(&mut arena, s, key(i));
            let count = arena.get(s.0).unwrap().count;
            assert_eq!(count, i + 1);
            for j in 0..=i {
                assert_eq!(
                    lookup(&arena, s, key(j)).unwrap(),
                    Some((j, Attrs::DEFAULT))
                );
            }
            assert_eq!(lookup(&arena, s, key(99)).unwrap(), None);
        }
        let order: Vec<_> = keys_in_order(&arena, s)
            .unwrap()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        assert_eq!(order, (0..20).map(key).collect::<Vec<_>>());
        assert_eq!(lookup(&arena, root, key(0)).unwrap(), None);
    }

    #[test]
    fn dead_child_edges_are_ignored() {
        let mut arena = Arena::new();
        let root = ShapeRef(arena.insert(Shape::root(None)).unwrap());
        let child = add(&mut arena, root, key(1));
        assert_eq!(
            arena
                .get(root.0)
                .unwrap()
                .find_transition(&arena, key(1), Attrs::DEFAULT),
            Some(child)
        );
        arena.remove(child.0).unwrap();
        assert_eq!(
            arena
                .get(root.0)
                .unwrap()
                .find_transition(&arena, key(1), Attrs::DEFAULT),
            None
        );
    }

    #[test]
    fn attrs_bits() {
        assert!(
            Attrs::DEFAULT.writable()
                && Attrs::DEFAULT.enumerable()
                && Attrs::DEFAULT.configurable()
        );
        assert!(!Attrs::NONE.writable());
        assert!(Attrs(Attrs::ENUMERABLE).enumerable());
    }
}
