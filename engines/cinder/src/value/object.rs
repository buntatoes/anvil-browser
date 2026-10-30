// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Objects: named properties, elements, and the object class.
//!
//! Named properties live either in shape slots ([`Props::Shaped`]) or, for
//! big or heavily edited objects, in a per-object table
//! ([`Props::Dictionary`]). Indexed properties live in [`Elements`], packed
//! in a `Vec` while they are dense and in a sorted map once they get gappy
//! or need attributes other than the usual ones.
//!
//! The functions that need shapes or atoms (getting, defining, deleting)
//! are on the heap in `gc`, since they touch more than one arena. This file
//! has the parts that only need the object itself.

use super::atom::PropertyKey;
use super::collection::MapData;
use super::shape::Attrs;
use super::{ObjectRef, ShapeRef, Value};
use std::collections::{BTreeMap, HashMap};

/// What a property slot holds.
#[derive(Clone, Copy, Debug)]
pub enum Slot {
    /// A data property.
    Data(Value),
    /// An accessor property. `undefined` means no getter or setter.
    Accessor {
        /// The getter function or `undefined`.
        get: Value,
        /// The setter function or `undefined`.
        set: Value,
    },
}

impl Slot {
    /// The data value, if this is a data slot.
    pub fn data(self) -> Option<Value> {
        match self {
            Slot::Data(v) => Some(v),
            Slot::Accessor { .. } => None,
        }
    }

    /// True for accessor slots.
    pub fn is_accessor(self) -> bool {
        matches!(self, Slot::Accessor { .. })
    }

    /// Every value this slot holds, for tracing.
    pub fn values(self) -> [Value; 2] {
        match self {
            Slot::Data(v) => [v, Value::UNDEFINED],
            Slot::Accessor { get, set } => [get, set],
        }
    }
}

/// One property: its attributes and its slot.
#[derive(Clone, Copy, Debug)]
pub struct Property {
    /// Attributes (the writable bit is ignored for accessors).
    pub attrs: Attrs,
    /// The value or accessor pair.
    pub slot: Slot,
}

impl Property {
    /// A data property with the usual attributes.
    pub fn data(v: Value) -> Property {
        Property {
            attrs: Attrs::DEFAULT,
            slot: Slot::Data(v),
        }
    }
}

/// Named properties in dictionary mode. Keeps insertion order.
#[derive(Debug, Default, Clone)]
pub struct Dictionary {
    /// The prototype (in shaped mode, this lives in the shape).
    pub proto: Option<ObjectRef>,
    index: HashMap<PropertyKey, usize>,
    entries: Vec<Option<(PropertyKey, Property)>>,
}

impl Dictionary {
    /// An empty dictionary with prototype `proto`.
    pub fn new(proto: Option<ObjectRef>) -> Self {
        Dictionary {
            proto,
            index: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Number of properties.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when there are no properties.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Look up a property.
    pub fn get(&self, key: PropertyKey) -> Option<Property> {
        let i = *self.index.get(&key)?;
        self.entries
            .get(i)
            .and_then(|e| e.as_ref())
            .map(|(_, p)| *p)
    }

    /// Add or replace a property. A new key goes at the end of the order.
    pub fn insert(&mut self, key: PropertyKey, prop: Property) {
        if let Some(&i) = self.index.get(&key)
            && let Some(Some((_, p))) = self.entries.get_mut(i)
        {
            *p = prop;
            return;
        }
        self.index.insert(key, self.entries.len());
        self.entries.push(Some((key, prop)));
    }

    /// Remove a property. Returns it if it was there.
    pub fn remove(&mut self, key: PropertyKey) -> Option<Property> {
        let i = self.index.remove(&key)?;
        let old = self
            .entries
            .get_mut(i)
            .and_then(Option::take)
            .map(|(_, p)| p);
        if self.entries.len() > 16 && self.index.len() * 2 < self.entries.len() {
            self.compact();
        }
        old
    }

    fn compact(&mut self) {
        self.entries.retain(Option::is_some);
        self.index.clear();
        for (i, e) in self.entries.iter().enumerate() {
            if let Some((k, _)) = e {
                self.index.insert(*k, i);
            }
        }
    }

    /// Properties in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (PropertyKey, Property)> + '_ {
        self.entries.iter().filter_map(|e| *e)
    }

    /// Estimated bytes owned.
    pub fn owned_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Option<(PropertyKey, Property)>>()
            + self.index.capacity() * (std::mem::size_of::<(PropertyKey, usize)>() + 8)
    }
}

/// Where the named properties are.
#[derive(Debug, Clone)]
pub enum Props {
    /// Keys in a shared shape, values in `slots` by shape slot.
    Shaped {
        /// The shape.
        shape: ShapeRef,
        /// One entry per key of the shape.
        slots: Vec<Slot>,
    },
    /// Keys and values in a per-object table.
    Dictionary(Dictionary),
}

/// How far past the end a store may land before elements go sparse.
pub const PACKED_GAP: u32 = 1024;

/// Indexed properties.
#[derive(Debug, Clone)]
pub enum Elements {
    /// Dense values. [`Value::HOLE`] marks a missing index. Every present
    /// element has the usual attributes.
    Packed(Vec<Value>),
    /// Gappy or attribute-carrying elements, sorted by index.
    Sparse(BTreeMap<u32, Property>),
}

impl Default for Elements {
    fn default() -> Self {
        Elements::Packed(Vec::new())
    }
}

impl Elements {
    /// The property at `index`, if present.
    pub fn get(&self, index: u32) -> Option<Property> {
        match self {
            Elements::Packed(v) => v
                .get(index as usize)
                .filter(|x| !x.is_hole())
                .map(|x| Property::data(*x)),
            Elements::Sparse(m) => m.get(&index).copied(),
        }
    }

    /// Number of present elements.
    pub fn count(&self) -> usize {
        match self {
            Elements::Packed(v) => v.iter().filter(|x| !x.is_hole()).count(),
            Elements::Sparse(m) => m.len(),
        }
    }

    /// Switch to sparse storage.
    pub fn make_sparse(&mut self) {
        if let Elements::Packed(v) = self {
            let mut m = BTreeMap::new();
            for (i, x) in v.iter().enumerate() {
                if !x.is_hole() {
                    m.insert(i as u32, Property::data(*x));
                }
            }
            *self = Elements::Sparse(m);
        }
    }

    /// Define or replace the element at `index`. Usual-attribute data goes
    /// in packed storage when it fits; anything else makes storage sparse.
    pub fn define(&mut self, index: u32, prop: Property) {
        let plain = prop.attrs == Attrs::DEFAULT && !prop.slot.is_accessor();
        if let (true, Elements::Packed(v), Slot::Data(x)) = (plain, &mut *self, prop.slot) {
            let i = index as usize;
            if i < v.len() {
                v[i] = x;
                return;
            }
            if index as u64 <= v.len() as u64 + u64::from(PACKED_GAP).max(v.len() as u64) {
                v.resize(i, Value::HOLE);
                v.push(x);
                return;
            }
        }
        self.make_sparse();
        if let Elements::Sparse(m) = self {
            m.insert(index, prop);
        }
    }

    /// Remove the element at `index`. Returns false when it is not configurable.
    pub fn delete(&mut self, index: u32) -> bool {
        match self {
            Elements::Packed(v) => {
                let i = index as usize;
                if i < v.len() {
                    v[i] = Value::HOLE;
                    while v.last().is_some_and(|x| x.is_hole()) {
                        v.pop();
                    }
                }
                true
            }
            Elements::Sparse(m) => match m.get(&index) {
                Some(p) if !p.attrs.configurable() => false,
                _ => {
                    m.remove(&index);
                    true
                }
            },
        }
    }

    /// Indices of present elements, ascending.
    pub fn indices(&self) -> Vec<u32> {
        match self {
            Elements::Packed(v) => v
                .iter()
                .enumerate()
                .filter(|(_, x)| !x.is_hole())
                .map(|(i, _)| i as u32)
                .collect(),
            Elements::Sparse(m) => m.keys().copied().collect(),
        }
    }

    /// The highest present index, if any.
    pub fn last_index(&self) -> Option<u32> {
        match self {
            Elements::Packed(v) => v.iter().rposition(|x| !x.is_hole()).map(|i| i as u32),
            Elements::Sparse(m) => m.keys().next_back().copied(),
        }
    }

    /// Every value held, for tracing.
    pub fn for_each_value(&self, mut f: impl FnMut(Value)) {
        match self {
            Elements::Packed(v) => v.iter().for_each(|x| f(*x)),
            Elements::Sparse(m) => m
                .values()
                .for_each(|p| p.slot.values().into_iter().for_each(&mut f)),
        }
    }

    /// Estimated bytes owned.
    pub fn owned_bytes(&self) -> usize {
        match self {
            Elements::Packed(v) => v.capacity() * std::mem::size_of::<Value>(),
            Elements::Sparse(m) => m.len() * (std::mem::size_of::<(u32, Property)>() + 16),
        }
    }
}

/// Data a host attaches to its wrapper objects (for example a node id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostData(pub u64);

/// What kind of object this is, and the internal data that goes with it.
#[derive(Debug, Clone)]
pub enum ObjectClass {
    /// A plain object.
    Ordinary,
    /// An array. `length` is kept here, not in a slot.
    Array {
        /// The array length.
        length: u32,
    },
    /// A function compiled from script. `code` names its code block, `env`
    /// is the closure environment.
    Function {
        /// Index of the compiled code block.
        code: u32,
        /// Captured environment object.
        env: Option<ObjectRef>,
    },
    /// A builtin or host function.
    Native {
        /// Which native function.
        id: u32,
    },
    /// An environment record for captured variables. Its slots are in
    /// `elements`, by index.
    Environment {
        /// The enclosing environment.
        parent: Option<ObjectRef>,
    },
    /// An error object.
    Error,
    /// A boxed primitive (`new Number(1)` and so on).
    Primitive(Value),
    /// A host wrapper (DOM node and so on).
    Host(HostData),
    /// A proxy. Both are `None` once revoked. Every operation on a proxy
    /// goes through its handler, so it never gets an inline cache entry.
    Proxy {
        /// The target object.
        target: Option<ObjectRef>,
        /// The handler object.
        handler: Option<ObjectRef>,
    },
    /// An `arguments` object. Its elements hold the arguments. In a sloppy
    /// function with simple parameters, index `i` is tied to parameter `i`
    /// (slot `i` of `env`) while `mapped[i]` is set.
    Arguments {
        /// The function's environment, once mapped.
        env: Option<ObjectRef>,
        /// Which indices are still tied to their parameter.
        mapped: Vec<bool>,
    },
    /// A `Map`, `Set`, `WeakMap` or `WeakSet`.
    Collection(Box<MapData>),
    /// An iterator over an array-like object or a string: keys, values or
    /// entries by index. `target` is `undefined` once it has finished.
    ListIterator {
        /// The object or string being walked.
        target: Value,
        /// Next index.
        index: u32,
        /// What it yields.
        kind: IterKind,
    },
    /// An iterator over a collection. `map` is `None` once finished.
    CollectionIterator {
        /// The collection.
        map: Option<ObjectRef>,
        /// Next entry position.
        pos: u32,
        /// What it yields.
        kind: IterKind,
    },
    /// A function made by `bind`.
    Bound(Box<BoundFunction>),
}

/// What an iterator yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IterKind {
    /// Keys (indices for lists).
    Keys,
    /// Values.
    Values,
    /// `[key, value]` pairs.
    Entries,
}

/// The parts of a bound function.
#[derive(Clone, Debug)]
pub struct BoundFunction {
    /// The function it calls.
    pub target: Value,
    /// The `this` it passes.
    pub this: Value,
    /// Arguments put before the call's own.
    pub args: Vec<Value>,
    /// Its `name` (`bound f`).
    pub name: String,
    /// Its `length`.
    pub length: u32,
}

impl ObjectClass {
    /// True for objects whose property operations can't use the ordinary
    /// shape fast paths (proxies, and later typed arrays and the like).
    pub fn is_exotic(&self) -> bool {
        matches!(self, ObjectClass::Proxy { .. })
    }

    /// Every value this class holds, for tracing.
    pub fn for_each_value(&self, mut f: impl FnMut(Value)) {
        match self {
            ObjectClass::Function { env: Some(e), .. } => f(Value::object(*e)),
            ObjectClass::Environment { parent: Some(p) } => f(Value::object(*p)),
            ObjectClass::Primitive(v) => f(*v),
            ObjectClass::Arguments { env: Some(e), .. } => f(Value::object(*e)),
            ObjectClass::Collection(m) if !m.kind.is_weak() => m.for_each_value(f),
            ObjectClass::ListIterator { target, .. } => f(*target),
            ObjectClass::CollectionIterator { map: Some(m), .. } => f(Value::object(*m)),
            ObjectClass::Bound(b) => {
                f(b.target);
                f(b.this);
                b.args.iter().for_each(|v| f(*v));
            }
            ObjectClass::Proxy { target, handler } => {
                if let Some(t) = target {
                    f(Value::object(*t));
                }
                if let Some(h) = handler {
                    f(Value::object(*h));
                }
            }
            _ => {}
        }
    }
}

/// An object cell on the heap.
#[derive(Debug, Clone)]
pub struct Object {
    /// Named properties.
    pub props: Props,
    /// Indexed properties.
    pub elements: Elements,
    /// What kind of object it is.
    pub class: ObjectClass,
    /// `[[Extensible]]`.
    pub extensible: bool,
    /// Set once this object is some object's prototype. Adding or removing
    /// keys on it then bumps the heap's prototype epoch, which inline caches
    /// for prototype hits check.
    pub is_prototype: bool,
}

impl Object {
    /// Estimated bytes owned beyond the arena slot.
    pub fn owned_bytes(&self) -> usize {
        let props = match &self.props {
            Props::Shaped { slots, .. } => slots.capacity() * std::mem::size_of::<Slot>(),
            Props::Dictionary(d) => d.owned_bytes(),
        };
        let class = match &self.class {
            ObjectClass::Collection(m) => m.owned_bytes(),
            ObjectClass::Bound(b) => {
                std::mem::size_of::<BoundFunction>()
                    + b.args.capacity() * std::mem::size_of::<Value>()
                    + b.name.capacity()
            }
            ObjectClass::Arguments { mapped, .. } => mapped.capacity(),
            _ => 0,
        };
        props + self.elements.owned_bytes() + class
    }

    /// True for arrays.
    pub fn is_array(&self) -> bool {
        matches!(self.class, ObjectClass::Array { .. })
    }

    /// True for callable objects.
    pub fn is_callable(&self) -> bool {
        matches!(
            self.class,
            ObjectClass::Function { .. } | ObjectClass::Native { .. } | ObjectClass::Bound(_)
        )
    }

    /// The shape, in shaped mode.
    pub fn shape(&self) -> Option<ShapeRef> {
        match &self.props {
            Props::Shaped { shape, .. } => Some(*shape),
            Props::Dictionary(_) => None,
        }
    }

    /// True in dictionary mode.
    pub fn is_dictionary(&self) -> bool {
        matches!(self.props, Props::Dictionary(_))
    }
}

#[cfg(test)]
mod tests {
    use super::super::atom::Atom;
    use super::*;

    #[test]
    fn packed_elements_grow_and_go_sparse() {
        let mut e = Elements::default();
        e.define(0, Property::data(Value::int32(1)));
        e.define(2, Property::data(Value::int32(3)));
        assert!(matches!(e, Elements::Packed(_)));
        assert!(e.get(1).is_none(), "gap reads as missing");
        assert_eq!(
            e.get(2)
                .and_then(|p| p.slot.data())
                .and_then(Value::as_int32),
            Some(3)
        );
        assert_eq!(e.count(), 2);
        assert_eq!(e.indices(), vec![0, 2]);
        e.define(1_000_000, Property::data(Value::TRUE));
        assert!(matches!(e, Elements::Sparse(_)), "a far store goes sparse");
        assert_eq!(e.indices(), vec![0, 2, 1_000_000]);
        assert_eq!(e.last_index(), Some(1_000_000));
    }

    #[test]
    fn element_attributes_force_sparse() {
        let mut e = Elements::default();
        e.define(0, Property::data(Value::int32(1)));
        e.define(
            1,
            Property {
                attrs: Attrs(Attrs::ENUMERABLE),
                slot: Slot::Data(Value::int32(2)),
            },
        );
        assert!(matches!(e, Elements::Sparse(_)));
        assert!(!e.delete(1), "not configurable");
        assert!(e.delete(0));
        assert_eq!(e.indices(), vec![1]);
    }

    #[test]
    fn packed_delete_trims_trailing_holes() {
        let mut e = Elements::default();
        for i in 0..4 {
            e.define(i, Property::data(Value::int32(i as i32)));
        }
        assert!(e.delete(3));
        assert!(e.delete(1));
        if let Elements::Packed(v) = &e {
            assert_eq!(v.len(), 3);
        }
        assert!(e.delete(2));
        if let Elements::Packed(v) = &e {
            assert_eq!(v.len(), 1);
        }
    }

    #[test]
    fn dictionary_keeps_order_and_compacts() {
        let mut d = Dictionary::new(None);
        for i in 0..40 {
            d.insert(
                PropertyKey::Atom(Atom(i)),
                Property::data(Value::int32(i as i32)),
            );
        }
        for i in (0..40).filter(|i| i % 3 != 0) {
            assert!(d.remove(PropertyKey::Atom(Atom(i))).is_some());
        }
        assert!(d.remove(PropertyKey::Atom(Atom(1))).is_none());
        let keys: Vec<u32> = d
            .iter()
            .map(|(k, _)| match k {
                PropertyKey::Atom(a) => a.0,
                _ => u32::MAX,
            })
            .collect();
        assert_eq!(keys, (0..40).filter(|i| i % 3 == 0).collect::<Vec<_>>());
        for i in (0..40).filter(|i| i % 3 == 0) {
            let p = d.get(PropertyKey::Atom(Atom(i))).unwrap();
            assert_eq!(p.slot.data().and_then(Value::as_int32), Some(i as i32));
        }
        d.insert(PropertyKey::Atom(Atom(0)), Property::data(Value::NULL));
        assert_eq!(
            d.iter().next().map(|(k, _)| k),
            Some(PropertyKey::Atom(Atom(0))),
            "replace keeps place"
        );
        assert_eq!(d.len(), 14);
    }

    #[test]
    fn proxies_are_exotic() {
        assert!(
            ObjectClass::Proxy {
                target: None,
                handler: None
            }
            .is_exotic()
        );
        assert!(!ObjectClass::Ordinary.is_exotic());
    }
}
