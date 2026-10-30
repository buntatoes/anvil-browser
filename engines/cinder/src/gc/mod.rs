// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The heap and the garbage collector.
//!
//! Every heap cell lives in a typed [`arena::Arena`] and is reached through a
//! generation-checked handle. Collection is precise, non-moving mark-sweep:
//!
//! - **Mark** starts from the roots the caller passes in plus the host's
//!   [`roots::Persistent`] roots, and follows children with an explicit work
//!   list, so a long chain of objects never touches the Rust stack.
//! - **Sweep** frees every unmarked cell. Its slot's generation goes up, so
//!   any old handle to it gives an error instead of the new occupant.
//!
//! The heap is budgeted in bytes, never in object counts. An allocation that
//! pushes the heap past the trigger only asks for a collection; the VM runs
//! it at its next safepoint ([`Heap::safepoint`]), so Rust code holding
//! values in locals never sees a collection in the middle of its work. An
//! allocation that would go past `max_heap_bytes` stops right there with
//! [`HeapError::Abort`], and so does a safepoint that can't get back under
//! the limit. Aborts can't be caught by script.

pub mod arena;
mod collections;
#[cfg(test)]
pub mod prng;
pub mod roots;
#[cfg(test)]
mod stress;

use crate::cinder::limits::{LimitKind, Limits, group_digits};
use crate::cinder::value::atom::{AtomTable, PropertyKey, array_index};
use crate::cinder::value::object::{
    Dictionary, Elements, HostData, Object, ObjectClass, Property, Props, Slot,
};
use crate::cinder::value::shape::{self, Attrs, Shape};
use crate::cinder::value::string::{JsString, collect_units, unit_at, units_to_string};
use crate::cinder::value::{ObjectRef, ShapeRef, StringRef, SymbolRef, Value};
use arena::{Arena, ArenaError, RawHandle};
use roots::{Persistent, RootTable, WrapperTable};
use std::collections::HashMap;
use std::fmt;

/// A limit that stops the script. Script can't catch it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Abort {
    /// Which limit.
    pub kind: LimitKind,
    /// Plain description, ready to show.
    pub reason: String,
    /// The limit's value.
    pub limit: u64,
    /// The value that ran into it.
    pub observed: u64,
}

impl fmt::Display for Abort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

/// Everything that can go wrong in a heap operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    /// An uncatchable limit was hit.
    Abort(Abort),
    /// A catchable `RangeError` (for example, a string that's too long).
    Range(String),
    /// A bug or a misuse of the API, such as a stale handle. Never silent.
    Internal(String),
}

impl fmt::Display for HeapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeapError::Abort(a) => write!(f, "{a}"),
            HeapError::Range(m) => write!(f, "RangeError: {m}"),
            HeapError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for HeapError {}

fn internal(what: &str, err: ArenaError) -> HeapError {
    match err {
        ArenaError::Full => HeapError::Abort(Abort {
            kind: LimitKind::HandleSpace,
            reason: format!("no more {what} slots: {err}"),
            limit: u64::from(u32::MAX),
            observed: u64::from(u32::MAX),
        }),
        other => HeapError::Internal(format!("{what}: {other}")),
    }
}

/// A symbol cell.
#[derive(Debug, Clone)]
pub struct Symbol {
    /// The description, if any.
    pub description: Option<StringRef>,
}

/// What one collection did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CollectStats {
    /// Objects freed.
    pub freed_objects: usize,
    /// Strings freed.
    pub freed_strings: usize,
    /// Symbols freed.
    pub freed_symbols: usize,
    /// Shapes freed.
    pub freed_shapes: usize,
    /// Heap bytes before.
    pub bytes_before: usize,
    /// Heap bytes after.
    pub bytes_after: usize,
    /// Host data of wrappers that died, so the host can drop its side.
    pub finalized: Vec<HostData>,
}

/// Running counters, for telemetry.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HeapStats {
    /// Collections run.
    pub collections: u64,
    /// Objects ever made.
    pub objects_allocated: u64,
    /// Strings ever made.
    pub strings_allocated: u64,
    /// Shapes ever made.
    pub shapes_created: u64,
    /// Objects switched to dictionary mode.
    pub dictionary_conversions: u64,
    /// Ropes flattened.
    pub ropes_flattened: u64,
}

/// Result of a lookup along the prototype chain.
#[derive(Debug, Clone, Copy)]
pub enum Lookup {
    /// Found on `holder`.
    Found {
        /// The object that has the property.
        holder: ObjectRef,
        /// The property.
        prop: Property,
    },
    /// Not on any object in the chain.
    NotFound,
    /// The walk reached an exotic object (a proxy). The VM has to carry on
    /// through that object's own internal methods.
    Exotic(ObjectRef),
}

#[derive(Clone, Copy)]
enum Cell {
    Object(ObjectRef),
    String(StringRef),
    Symbol(SymbolRef),
    Shape(ShapeRef),
}

fn push_value(work: &mut Vec<Cell>, v: Value) {
    if let Some(o) = v.as_object() {
        work.push(Cell::Object(o));
    } else if let Some(s) = v.as_string() {
        work.push(Cell::String(s));
    } else if let Some(y) = v.as_symbol() {
        work.push(Cell::Symbol(y));
    }
}

fn push_key(work: &mut Vec<Cell>, k: PropertyKey) {
    if let PropertyKey::Symbol(s) = k {
        work.push(Cell::Symbol(s));
    }
}

/// True when a property may be replaced by `new` under the language's rules
/// for non-configurable properties.
fn can_redefine(old: Property, new: Property) -> bool {
    if old.attrs.configurable() {
        return true;
    }
    if new.attrs.configurable() || new.attrs.enumerable() != old.attrs.enumerable() {
        return false;
    }
    match (old.slot, new.slot) {
        (Slot::Data(ov), Slot::Data(nv)) => {
            old.attrs.writable() || (!new.attrs.writable() && ov.same_bits(nv))
        }
        (Slot::Accessor { get: og, set: os }, Slot::Accessor { get: ng, set: ns }) => {
            og.same_bits(ng) && os.same_bits(ns)
        }
        _ => false,
    }
}

/// The engine heap.
#[derive(Debug)]
pub struct Heap {
    limits: Limits,
    objects: Arena<Object>,
    strings: Arena<JsString>,
    symbols: Arena<Symbol>,
    shapes: Arena<Shape>,
    atoms: AtomTable,
    root_shapes: HashMap<Option<ObjectRef>, ShapeRef>,
    roots: RootTable,
    wrappers: WrapperTable,
    bytes: usize,
    next_trigger: usize,
    collect_requested: bool,
    prototype_epoch: u64,
    stats: HeapStats,
}

impl Heap {
    /// A heap with the given limits. Fails with a reason if the limits
    /// don't make sense.
    pub fn new(limits: Limits) -> Result<Heap, String> {
        limits.validate()?;
        Ok(Heap {
            next_trigger: limits.initial_gc_trigger,
            limits,
            objects: Arena::new(),
            strings: Arena::new(),
            symbols: Arena::new(),
            shapes: Arena::new(),
            atoms: AtomTable::new(),
            root_shapes: HashMap::new(),
            roots: RootTable::new(),
            wrappers: WrapperTable::new(),
            bytes: 0,
            collect_requested: false,
            prototype_epoch: 0,
            stats: HeapStats::default(),
        })
    }

    /// The limits in force.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Heap bytes in use, as counted toward `max_heap_bytes`.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Counters.
    pub fn stats(&self) -> &HeapStats {
        &self.stats
    }

    /// True when an allocation passed the trigger and a collection should
    /// run at the next safepoint.
    pub fn collect_requested(&self) -> bool {
        self.collect_requested
    }

    /// Goes up whenever a key is added to or removed from an object that is
    /// some object's prototype.
    pub fn prototype_epoch(&self) -> u64 {
        self.prototype_epoch
    }

    /// Number of live objects.
    pub fn live_objects(&self) -> usize {
        self.objects.live()
    }

    /// Number of live strings.
    pub fn live_strings(&self) -> usize {
        self.strings.live()
    }

    /// Number of live shapes.
    pub fn live_shapes(&self) -> usize {
        self.shapes.live()
    }

    /// The atom table.
    pub fn atoms(&self) -> &AtomTable {
        &self.atoms
    }

    // ---- byte accounting ------------------------------------------------

    fn limit_abort(&self, needed: usize) -> HeapError {
        let total = self.bytes.saturating_add(needed);
        HeapError::Abort(Abort {
            kind: LimitKind::HeapBytes,
            reason: format!(
                "heap limit reached: {} more bytes needed with {} bytes in use, limit {} bytes",
                group_digits(needed as u64),
                group_digits(self.bytes as u64),
                group_digits(self.limits.max_heap_bytes as u64)
            ),
            limit: self.limits.max_heap_bytes as u64,
            observed: total as u64,
        })
    }

    fn charge(&mut self, n: usize) -> Result<(), HeapError> {
        if self.bytes.saturating_add(n) > self.limits.max_heap_bytes {
            return Err(self.limit_abort(n));
        }
        self.bytes += n;
        if self.bytes > self.next_trigger {
            self.collect_requested = true;
        }
        Ok(())
    }

    /// Account for a cell that went from `before` to `after` owned bytes.
    /// The change has already happened; if it went past the limit, the
    /// script stops.
    fn adjust(&mut self, before: usize, after: usize) -> Result<(), HeapError> {
        if after >= before {
            let grow = after - before;
            if self.bytes.saturating_add(grow) > self.limits.max_heap_bytes {
                let err = self.limit_abort(grow);
                self.bytes = self.bytes.saturating_add(grow);
                return Err(err);
            }
            self.bytes += grow;
            if self.bytes > self.next_trigger {
                self.collect_requested = true;
            }
        } else {
            self.bytes = self.bytes.saturating_sub(before - after);
        }
        Ok(())
    }

    /// Count every live cell from scratch. Tests compare this with the
    /// running count.
    pub fn recount_bytes(&self) -> usize {
        let objects: usize = self
            .objects
            .iter()
            .map(|(_, o)| Arena::<Object>::slot_bytes() + o.owned_bytes())
            .sum();
        let strings: usize = self
            .strings
            .iter()
            .map(|(_, s)| Arena::<JsString>::slot_bytes() + s.owned_bytes())
            .sum();
        let symbols = self.symbols.live() * Arena::<Symbol>::slot_bytes();
        let shapes: usize = self
            .shapes
            .iter()
            .map(|(_, s)| Arena::<Shape>::slot_bytes() + s.owned_bytes())
            .sum();
        objects + strings + symbols + shapes + self.atoms.bytes()
    }

    // ---- strings ----------------------------------------------------------

    fn check_string_length(&self, len: usize) -> Result<(), HeapError> {
        if len > self.limits.max_string_length {
            return Err(HeapError::Range(format!(
                "Invalid string length: {} code units, limit {}",
                group_digits(len as u64),
                group_digits(self.limits.max_string_length as u64)
            )));
        }
        Ok(())
    }

    /// Make a flat string from code units.
    pub fn alloc_string_units(&mut self, units: Vec<u16>) -> Result<StringRef, HeapError> {
        self.check_string_length(units.len())?;
        let cell = JsString::Flat(units);
        self.charge(Arena::<JsString>::slot_bytes() + cell.owned_bytes())?;
        let h = self
            .strings
            .insert(cell)
            .map_err(|e| internal("string", e))?;
        self.stats.strings_allocated += 1;
        Ok(StringRef(h))
    }

    /// Make a string from Rust text.
    pub fn alloc_string(&mut self, s: &str) -> Result<StringRef, HeapError> {
        self.alloc_string_units(s.encode_utf16().collect())
    }

    /// Length in code units.
    pub fn string_len(&self, s: StringRef) -> Result<usize, HeapError> {
        Ok(self
            .strings
            .get(s.0)
            .map_err(|e| internal("string", e))?
            .len())
    }

    /// Join two strings. Short results are copied; long ones make a rope.
    pub fn concat(&mut self, a: StringRef, b: StringRef) -> Result<StringRef, HeapError> {
        let (la, da) = {
            let s = self.strings.get(a.0).map_err(|e| internal("string", e))?;
            (s.len(), s.depth())
        };
        let (lb, db) = {
            let s = self.strings.get(b.0).map_err(|e| internal("string", e))?;
            (s.len(), s.depth())
        };
        let total = la.saturating_add(lb);
        self.check_string_length(total)?;
        if total < self.limits.rope_min_length {
            let mut units = collect_units(&self.strings, a).map_err(|e| internal("string", e))?;
            units.extend(collect_units(&self.strings, b).map_err(|e| internal("string", e))?);
            return self.alloc_string_units(units);
        }
        let cell = JsString::Rope {
            left: a,
            right: b,
            len: total,
            depth: da.max(db).saturating_add(1),
        };
        self.charge(Arena::<JsString>::slot_bytes())?;
        let h = self
            .strings
            .insert(cell)
            .map_err(|e| internal("string", e))?;
        self.stats.strings_allocated += 1;
        Ok(StringRef(h))
    }

    /// Flatten a rope in place. The handle stays the same.
    pub fn flatten(&mut self, s: StringRef) -> Result<(), HeapError> {
        let cell = self.strings.get(s.0).map_err(|e| internal("string", e))?;
        if cell.flat_units().is_some() {
            return Ok(());
        }
        let units = collect_units(&self.strings, s).map_err(|e| internal("string", e))?;
        let flat = JsString::Flat(units);
        let after = flat.owned_bytes();
        let slot = self
            .strings
            .get_mut(s.0)
            .map_err(|e| internal("string", e))?;
        *slot = flat;
        self.stats.ropes_flattened += 1;
        self.adjust(0, after)
    }

    /// The code units of a string, flattening it first if needed.
    pub fn string_units(&mut self, s: StringRef) -> Result<&[u16], HeapError> {
        self.flatten(s)?;
        match self.strings.get(s.0).map_err(|e| internal("string", e))? {
            JsString::Flat(u) => Ok(u),
            JsString::Rope { .. } => Err(HeapError::Internal(
                "string is still a rope right after flattening".to_string(),
            )),
        }
    }

    /// True when the string is a rope (not flattened yet).
    pub fn is_rope(&self, s: StringRef) -> Result<bool, HeapError> {
        Ok(self
            .strings
            .get(s.0)
            .map_err(|e| internal("string", e))?
            .flat_units()
            .is_none())
    }

    /// The code unit at `index` (`charCodeAt`), without flattening.
    pub fn char_code_at(&self, s: StringRef, index: usize) -> Result<Option<u16>, HeapError> {
        unit_at(&self.strings, s, index).map_err(|e| internal("string", e))
    }

    /// The string as Rust text, for the host. Lone surrogates become U+FFFD.
    pub fn string_to_rust(&self, s: StringRef) -> Result<String, HeapError> {
        let units = collect_units(&self.strings, s).map_err(|e| internal("string", e))?;
        Ok(units_to_string(&units))
    }

    // ---- keys and atoms ---------------------------------------------------

    /// The property key for some code units: an index or an atom.
    pub fn key_from_units(&mut self, units: &[u16]) -> Result<PropertyKey, HeapError> {
        if let Some(i) = array_index(units) {
            return Ok(PropertyKey::Index(i));
        }
        let cost = self.atoms.cost(units);
        self.charge(cost)?;
        let atom = self.atoms.intern(units).map_err(|m| {
            HeapError::Abort(Abort {
                kind: LimitKind::HandleSpace,
                reason: m,
                limit: u64::from(u32::MAX),
                observed: u64::from(u32::MAX),
            })
        })?;
        Ok(PropertyKey::Atom(atom))
    }

    /// The property key for Rust text.
    pub fn key_from_str(&mut self, s: &str) -> Result<PropertyKey, HeapError> {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.key_from_units(&units)
    }

    /// The property key for a string value.
    pub fn key_from_string(&mut self, s: StringRef) -> Result<PropertyKey, HeapError> {
        let units = self.string_units(s)?.to_vec();
        self.key_from_units(&units)
    }

    // ---- symbols ----------------------------------------------------------

    /// Make a new symbol.
    pub fn alloc_symbol(&mut self, description: Option<StringRef>) -> Result<SymbolRef, HeapError> {
        self.charge(Arena::<Symbol>::slot_bytes())?;
        let h = self
            .symbols
            .insert(Symbol { description })
            .map_err(|e| internal("symbol", e))?;
        Ok(SymbolRef(h))
    }

    /// A symbol's description.
    pub fn symbol_description(&self, s: SymbolRef) -> Result<Option<StringRef>, HeapError> {
        Ok(self
            .symbols
            .get(s.0)
            .map_err(|e| internal("symbol", e))?
            .description)
    }

    // ---- shapes -----------------------------------------------------------

    fn alloc_shape(&mut self, shape: Shape) -> Result<ShapeRef, HeapError> {
        self.charge(Arena::<Shape>::slot_bytes() + shape.owned_bytes())?;
        let h = self
            .shapes
            .insert(shape)
            .map_err(|e| internal("shape", e))?;
        self.stats.shapes_created += 1;
        Ok(ShapeRef(h))
    }

    fn root_shape(&mut self, proto: Option<ObjectRef>) -> Result<ShapeRef, HeapError> {
        if let Some(r) = self.root_shapes.get(&proto)
            && self.shapes.contains(r.0)
        {
            return Ok(*r);
        }
        let r = self.alloc_shape(Shape::root(proto))?;
        self.root_shapes.insert(proto, r);
        Ok(r)
    }

    /// The shape of an object, or `None` in dictionary mode.
    pub fn shape_of(&self, obj: ObjectRef) -> Result<Option<ShapeRef>, HeapError> {
        Ok(self.object(obj)?.shape())
    }

    /// Look at a shape.
    pub fn shape(&self, s: ShapeRef) -> Result<&Shape, HeapError> {
        self.shapes.get(s.0).map_err(|e| internal("shape", e))
    }

    /// The child of `from` for `(key, attrs)`, making it if needed. `None`
    /// means `from` has too many children and the object should go to
    /// dictionary mode instead.
    fn transition(
        &mut self,
        from: ShapeRef,
        key: PropertyKey,
        attrs: Attrs,
    ) -> Result<Option<ShapeRef>, HeapError> {
        let parent = self.shapes.get(from.0).map_err(|e| internal("shape", e))?;
        if let Some(c) = parent.find_transition(&self.shapes, key, attrs) {
            return Ok(Some(c));
        }
        if parent.transitions.len() >= self.limits.max_transitions_per_shape as usize {
            return Ok(None);
        }
        let child = Shape::child(from, parent, key, attrs);
        let child = self.alloc_shape(child)?;
        let parent = self
            .shapes
            .get_mut(from.0)
            .map_err(|e| internal("shape", e))?;
        let before = parent.owned_bytes();
        parent.transitions.retain(|(_, _, c)| c.0 != child.0);
        parent.transitions.push((key, attrs, child));
        let after = parent.owned_bytes();
        self.adjust(before, after)?;
        Ok(Some(child))
    }

    // ---- objects ----------------------------------------------------------

    /// Look at an object.
    pub fn object(&self, obj: ObjectRef) -> Result<&Object, HeapError> {
        self.objects.get(obj.0).map_err(|e| internal("object", e))
    }

    fn object_mut(&mut self, obj: ObjectRef) -> Result<&mut Object, HeapError> {
        self.objects
            .get_mut(obj.0)
            .map_err(|e| internal("object", e))
    }

    /// True when the handle points at a live object.
    pub fn is_live(&self, obj: ObjectRef) -> bool {
        self.objects.contains(obj.0)
    }

    /// Make an object with prototype `proto` and class `class`.
    pub fn alloc_object(
        &mut self,
        proto: Option<ObjectRef>,
        class: ObjectClass,
    ) -> Result<ObjectRef, HeapError> {
        if let Some(p) = proto {
            self.object_mut(p)?.is_prototype = true;
        }
        let shape = self.root_shape(proto)?;
        let obj = Object {
            props: Props::Shaped {
                shape,
                slots: Vec::new(),
            },
            elements: Elements::default(),
            class,
            extensible: true,
            is_prototype: false,
        };
        self.charge(Arena::<Object>::slot_bytes() + obj.owned_bytes())?;
        let h = self
            .objects
            .insert(obj)
            .map_err(|e| internal("object", e))?;
        self.stats.objects_allocated += 1;
        Ok(ObjectRef(h))
    }

    /// Make a plain object.
    pub fn new_object(&mut self, proto: Option<ObjectRef>) -> Result<ObjectRef, HeapError> {
        self.alloc_object(proto, ObjectClass::Ordinary)
    }

    /// Make an empty array.
    pub fn new_array(&mut self, proto: Option<ObjectRef>) -> Result<ObjectRef, HeapError> {
        self.alloc_object(proto, ObjectClass::Array { length: 0 })
    }

    fn touched(&mut self, obj: ObjectRef) -> Result<(), HeapError> {
        if self.object(obj)?.is_prototype {
            self.prototype_epoch += 1;
        }
        Ok(())
    }

    /// The class of an object, to change its internal data (the VM uses it
    /// for `arguments` mapping).
    pub fn class_mut(&mut self, obj: ObjectRef) -> Result<&mut ObjectClass, HeapError> {
        Ok(&mut self.object_mut(obj)?.class)
    }

    /// The prototype of an object.
    pub fn get_prototype(&self, obj: ObjectRef) -> Result<Option<ObjectRef>, HeapError> {
        match &self.object(obj)?.props {
            Props::Shaped { shape, .. } => Ok(self.shape(*shape)?.proto),
            Props::Dictionary(d) => Ok(d.proto),
        }
    }

    /// An own property, if present. This is the ordinary internal method;
    /// the VM sends exotic objects through their own methods first.
    pub fn get_own(&self, obj: ObjectRef, key: PropertyKey) -> Result<Option<Property>, HeapError> {
        let o = self.object(obj)?;
        if let PropertyKey::Index(i) = key {
            return Ok(o.elements.get(i));
        }
        match &o.props {
            Props::Shaped { shape, slots } => {
                let found =
                    shape::lookup(&self.shapes, *shape, key).map_err(|e| internal("shape", e))?;
                match found {
                    Some((slot, attrs)) => match slots.get(slot as usize) {
                        Some(s) => Ok(Some(Property { attrs, slot: *s })),
                        None => Err(HeapError::Internal(format!(
                            "shape slot {slot} is past the object's {} slots",
                            slots.len()
                        ))),
                    },
                    None => Ok(None),
                }
            }
            Props::Dictionary(d) => Ok(d.get(key)),
        }
    }

    /// Find a property on the object or its prototype chain.
    pub fn lookup(&self, obj: ObjectRef, key: PropertyKey) -> Result<Lookup, HeapError> {
        let mut cur = obj;
        let mut hops = 0usize;
        loop {
            if self.object(cur)?.class.is_exotic() {
                return Ok(Lookup::Exotic(cur));
            }
            if let Some(prop) = self.get_own(cur, key)? {
                return Ok(Lookup::Found { holder: cur, prop });
            }
            match self.get_prototype(cur)? {
                Some(p) => cur = p,
                None => return Ok(Lookup::NotFound),
            }
            hops += 1;
            if hops > self.objects.live() {
                return Err(HeapError::Internal(format!(
                    "prototype chain is longer than the {} live objects, so it has a cycle",
                    self.objects.live()
                )));
            }
        }
    }

    /// Switch an object to dictionary mode.
    pub fn make_dictionary(&mut self, obj: ObjectRef) -> Result<(), HeapError> {
        let before = self.object(obj)?.owned_bytes();
        self.make_dictionary_inner(obj)?;
        let after = self.object(obj)?.owned_bytes();
        self.adjust(before, after)
    }

    /// The switch itself, without byte accounting (the caller does it).
    fn make_dictionary_inner(&mut self, obj: ObjectRef) -> Result<(), HeapError> {
        let (shape, slots) = match &self.object(obj)?.props {
            Props::Dictionary(_) => return Ok(()),
            Props::Shaped { shape, slots } => (*shape, slots.clone()),
        };
        let proto = self.shape(shape)?.proto;
        let keys = shape::keys_in_order(&self.shapes, shape).map_err(|e| internal("shape", e))?;
        let mut dict = Dictionary::new(proto);
        for (key, slot, attrs) in keys {
            let s = slots.get(slot as usize).copied().ok_or_else(|| {
                HeapError::Internal(format!("shape slot {slot} is past the object's slots"))
            })?;
            dict.insert(key, Property { attrs, slot: s });
        }
        self.object_mut(obj)?.props = Props::Dictionary(dict);
        self.stats.dictionary_conversions += 1;
        Ok(())
    }

    /// Define (or redefine) an own property. Returns false when the language
    /// says no: the object isn't extensible, or the existing property can't
    /// be changed that way.
    pub fn define_own(
        &mut self,
        obj: ObjectRef,
        key: PropertyKey,
        prop: Property,
    ) -> Result<bool, HeapError> {
        let before = self.object(obj)?.owned_bytes();
        let result = self.define_own_inner(obj, key, prop);
        let after = self.object(obj)?.owned_bytes();
        self.adjust(before, after)?;
        result
    }

    fn define_own_inner(
        &mut self,
        obj: ObjectRef,
        key: PropertyKey,
        prop: Property,
    ) -> Result<bool, HeapError> {
        let existing = self.get_own(obj, key)?;
        if let Some(old) = existing {
            if !can_redefine(old, prop) {
                return Ok(false);
            }
        } else if !self.object(obj)?.extensible {
            return Ok(false);
        }

        if let PropertyKey::Index(i) = key {
            let o = self.object_mut(obj)?;
            o.elements.define(i, prop);
            if let ObjectClass::Array { length } = &mut o.class
                && i >= *length
            {
                *length = i + 1;
            }
            if existing.is_none() {
                self.touched(obj)?;
            }
            return Ok(true);
        }

        let shape = match &self.object(obj)?.props {
            Props::Dictionary(_) => None,
            Props::Shaped { shape, .. } => Some(*shape),
        };
        let Some(shape) = shape else {
            if let Props::Dictionary(d) = &mut self.object_mut(obj)?.props {
                d.insert(key, prop);
            }
            if existing.is_none() {
                self.touched(obj)?;
            }
            return Ok(true);
        };

        if let Some(old) = existing {
            if old.attrs == prop.attrs {
                let (slot, _) = shape::lookup(&self.shapes, shape, key)
                    .map_err(|e| internal("shape", e))?
                    .ok_or_else(|| {
                        HeapError::Internal("property vanished from its shape".to_string())
                    })?;
                if let Props::Shaped { slots, .. } = &mut self.object_mut(obj)?.props
                    && let Some(s) = slots.get_mut(slot as usize)
                {
                    *s = prop.slot;
                }
                return Ok(true);
            }
            // An attribute change on an existing key: go to dictionary mode.
            self.make_dictionary_inner(obj)?;
            if let Props::Dictionary(d) = &mut self.object_mut(obj)?.props {
                d.insert(key, prop);
            }
            return Ok(true);
        }

        let count = self.shape(shape)?.count;
        let next = if count >= self.limits.dictionary_threshold {
            None
        } else {
            self.transition(shape, key, prop.attrs)?
        };
        match next {
            Some(next) => {
                if let Props::Shaped { shape, slots } = &mut self.object_mut(obj)?.props {
                    *shape = next;
                    slots.push(prop.slot);
                }
            }
            None => {
                self.make_dictionary_inner(obj)?;
                if let Props::Dictionary(d) = &mut self.object_mut(obj)?.props {
                    d.insert(key, prop);
                }
            }
        }
        self.touched(obj)?;
        Ok(true)
    }

    /// Write a data value to an own property, making it (with the usual
    /// attributes) if it isn't there. Returns false for accessors,
    /// read-only properties and non-extensible objects; the VM decides
    /// whether that throws and calls setters itself.
    pub fn set_own_value(
        &mut self,
        obj: ObjectRef,
        key: PropertyKey,
        value: Value,
    ) -> Result<bool, HeapError> {
        match self.get_own(obj, key)? {
            Some(p) => match p.slot {
                Slot::Data(_) if p.attrs.writable() => self.define_own(
                    obj,
                    key,
                    Property {
                        attrs: p.attrs,
                        slot: Slot::Data(value),
                    },
                ),
                _ => Ok(false),
            },
            None => self.define_own(obj, key, Property::data(value)),
        }
    }

    /// A data value along the chain: `Some(value)` when found as data,
    /// `None` when missing. Accessors and exotic objects give an internal
    /// error, since running them is the VM's job.
    pub fn get_value(&self, obj: ObjectRef, key: PropertyKey) -> Result<Option<Value>, HeapError> {
        match self.lookup(obj, key)? {
            Lookup::Found { prop, .. } => match prop.slot {
                Slot::Data(v) => Ok(Some(v)),
                Slot::Accessor { .. } => Err(HeapError::Internal(
                    "get_value reached an accessor; the VM must call it".to_string(),
                )),
            },
            Lookup::NotFound => Ok(None),
            Lookup::Exotic(_) => Err(HeapError::Internal(
                "get_value reached a proxy; the VM must use its traps".to_string(),
            )),
        }
    }

    /// Delete an own property. Returns false when it isn't configurable.
    pub fn delete_own(&mut self, obj: ObjectRef, key: PropertyKey) -> Result<bool, HeapError> {
        let before = self.object(obj)?.owned_bytes();
        let result = self.delete_own_inner(obj, key);
        let after = self.object(obj)?.owned_bytes();
        self.adjust(before, after)?;
        result
    }

    fn delete_own_inner(&mut self, obj: ObjectRef, key: PropertyKey) -> Result<bool, HeapError> {
        let Some(old) = self.get_own(obj, key)? else {
            return Ok(true);
        };
        if !old.attrs.configurable() {
            return Ok(false);
        }
        if let PropertyKey::Index(i) = key {
            let ok = self.object_mut(obj)?.elements.delete(i);
            self.touched(obj)?;
            return Ok(ok);
        }
        let shape = self.object(obj)?.shape();
        if let Some(shape) = shape {
            let (last_key, parent) = {
                let s = self.shape(shape)?;
                (s.key, s.parent)
            };
            if last_key == Some(key)
                && let Some(parent) = parent
            {
                if let Props::Shaped { shape, slots } = &mut self.object_mut(obj)?.props {
                    *shape = parent;
                    slots.pop();
                }
                self.touched(obj)?;
                return Ok(true);
            }
            self.make_dictionary_inner(obj)?;
        }
        if let Props::Dictionary(d) = &mut self.object_mut(obj)?.props {
            d.remove(key);
        }
        self.touched(obj)?;
        Ok(true)
    }

    /// Own keys in the language's order: indices ascending, then string
    /// keys in the order they were added, then symbols in the order they
    /// were added.
    pub fn own_keys(&self, obj: ObjectRef) -> Result<Vec<PropertyKey>, HeapError> {
        let o = self.object(obj)?;
        let mut out: Vec<PropertyKey> = o
            .elements
            .indices()
            .into_iter()
            .map(PropertyKey::Index)
            .collect();
        let named: Vec<PropertyKey> = match &o.props {
            Props::Shaped { shape, .. } => shape::keys_in_order(&self.shapes, *shape)
                .map_err(|e| internal("shape", e))?
                .into_iter()
                .map(|(k, _, _)| k)
                .collect(),
            Props::Dictionary(d) => d.iter().map(|(k, _)| k).collect(),
        };
        out.extend(named.iter().filter(|k| !k.is_symbol()));
        out.extend(named.iter().filter(|k| k.is_symbol()));
        Ok(out)
    }

    /// Change the prototype. Returns false when the object isn't extensible
    /// or the change would make a cycle.
    pub fn set_prototype(
        &mut self,
        obj: ObjectRef,
        proto: Option<ObjectRef>,
    ) -> Result<bool, HeapError> {
        if self.get_prototype(obj)? == proto {
            return Ok(true);
        }
        if !self.object(obj)?.extensible {
            return Ok(false);
        }
        let mut cur = proto;
        let mut hops = 0usize;
        while let Some(p) = cur {
            if p == obj {
                return Ok(false);
            }
            if self.object(p)?.class.is_exotic() {
                break;
            }
            cur = self.get_prototype(p)?;
            hops += 1;
            if hops > self.objects.live() {
                return Err(HeapError::Internal(
                    "prototype chain has a cycle".to_string(),
                ));
            }
        }
        if let Some(p) = proto {
            self.object_mut(p)?.is_prototype = true;
        }
        let before = self.object(obj)?.owned_bytes();
        let shape = self.object(obj)?.shape();
        match shape {
            None => {
                if let Props::Dictionary(d) = &mut self.object_mut(obj)?.props {
                    d.proto = proto;
                }
            }
            Some(shape) => {
                let keys =
                    shape::keys_in_order(&self.shapes, shape).map_err(|e| internal("shape", e))?;
                let old_slots = match &self.object(obj)?.props {
                    Props::Shaped { slots, .. } => slots.clone(),
                    Props::Dictionary(_) => Vec::new(),
                };
                let root = self.root_shape(proto)?;
                self.object_mut(obj)?.props = Props::Shaped {
                    shape: root,
                    slots: Vec::new(),
                };
                for (key, slot, attrs) in keys {
                    let s = old_slots.get(slot as usize).copied().ok_or_else(|| {
                        HeapError::Internal(format!("shape slot {slot} is past the object's slots"))
                    })?;
                    self.define_own_inner(obj, key, Property { attrs, slot: s })?;
                }
            }
        }
        let after = self.object(obj)?.owned_bytes();
        self.adjust(before, after)?;
        self.touched(obj)?;
        Ok(true)
    }

    /// `Object.preventExtensions`.
    pub fn prevent_extensions(&mut self, obj: ObjectRef) -> Result<(), HeapError> {
        self.object_mut(obj)?.extensible = false;
        Ok(())
    }

    /// The length of an array.
    pub fn array_length(&self, obj: ObjectRef) -> Result<u32, HeapError> {
        match self.object(obj)?.class {
            ObjectClass::Array { length } => Ok(length),
            _ => Err(HeapError::Internal(
                "array_length on an object that isn't an array".to_string(),
            )),
        }
    }

    /// Set an array's length, deleting elements past it. Returns false when
    /// a non-configurable element stops the shrink; the length then ends
    /// just past that element.
    pub fn set_array_length(&mut self, obj: ObjectRef, new_len: u32) -> Result<bool, HeapError> {
        let old_len = self.array_length(obj)?;
        let before = self.object(obj)?.owned_bytes();
        let mut final_len = new_len;
        let mut ok = true;
        if new_len < old_len {
            let o = self.object_mut(obj)?;
            match &mut o.elements {
                Elements::Packed(v) => v.truncate(new_len as usize),
                Elements::Sparse(_) => {
                    let doomed: Vec<u32> = o
                        .elements
                        .indices()
                        .into_iter()
                        .filter(|i| *i >= new_len)
                        .rev()
                        .collect();
                    for i in doomed {
                        if !o.elements.delete(i) {
                            final_len = i + 1;
                            ok = false;
                            break;
                        }
                    }
                }
            }
        }
        if let ObjectClass::Array { length } = &mut self.object_mut(obj)?.class {
            *length = final_len;
        }
        let after = self.object(obj)?.owned_bytes();
        self.adjust(before, after)?;
        Ok(ok)
    }

    // ---- fast paths for inline caches and environments --------------------

    /// Where an own named property sits in a shaped object: its slot and
    /// attributes. `None` for dictionary objects and missing keys.
    pub fn own_slot(
        &self,
        obj: ObjectRef,
        key: PropertyKey,
    ) -> Result<Option<(u32, Attrs)>, HeapError> {
        match &self.object(obj)?.props {
            Props::Shaped { shape, .. } => {
                shape::lookup(&self.shapes, *shape, key).map_err(|e| internal("shape", e))
            }
            Props::Dictionary(_) => Ok(None),
        }
    }

    /// The slot at `slot` of a shaped object.
    pub fn read_slot(&self, obj: ObjectRef, slot: u32) -> Result<Slot, HeapError> {
        match &self.object(obj)?.props {
            Props::Shaped { slots, .. } => slots.get(slot as usize).copied().ok_or_else(|| {
                HeapError::Internal(format!(
                    "slot {slot} is past the object's {} slots",
                    slots.len()
                ))
            }),
            Props::Dictionary(_) => Err(HeapError::Internal(
                "read_slot on a dictionary object".to_string(),
            )),
        }
    }

    /// Overwrite the data slot at `slot` of a shaped object. The caller has
    /// checked the shape, so the slot is a writable data property.
    pub fn write_slot(&mut self, obj: ObjectRef, slot: u32, value: Value) -> Result<(), HeapError> {
        match &mut self.object_mut(obj)?.props {
            Props::Shaped { slots, .. } => match slots.get_mut(slot as usize) {
                Some(s) => {
                    *s = Slot::Data(value);
                    Ok(())
                }
                None => Err(HeapError::Internal(format!(
                    "slot {slot} is past the object's slots"
                ))),
            },
            Props::Dictionary(_) => Err(HeapError::Internal(
                "write_slot on a dictionary object".to_string(),
            )),
        }
    }

    /// Add a property along a transition an inline cache saw before: if the
    /// object still has shape `from` and `to` is still the live child of
    /// `from` for the next slot, move the object to `to` and push `value`.
    /// Returns false (and changes nothing) when any of that no longer holds.
    pub fn apply_add_transition(
        &mut self,
        obj: ObjectRef,
        from: ShapeRef,
        to: ShapeRef,
        value: Value,
    ) -> Result<bool, HeapError> {
        let Ok(to_shape) = self.shapes.get(to.0) else {
            return Ok(false);
        };
        if to_shape.parent != Some(from) {
            return Ok(false);
        }
        let new_slot = to_shape.slot as usize;
        let before = self.object(obj)?.owned_bytes();
        let o = self.object_mut(obj)?;
        if !o.extensible {
            return Ok(false);
        }
        let added = match &mut o.props {
            Props::Shaped { shape, slots } if *shape == from && slots.len() == new_slot => {
                *shape = to;
                slots.push(Slot::Data(value));
                true
            }
            _ => false,
        };
        if added {
            let after = self.object(obj)?.owned_bytes();
            self.adjust(before, after)?;
            self.touched(obj)?;
        }
        Ok(added)
    }

    /// Make an environment object with `size` slots, all `undefined`.
    pub fn alloc_env(
        &mut self,
        parent: Option<ObjectRef>,
        size: u32,
    ) -> Result<ObjectRef, HeapError> {
        let env = self.alloc_object(None, ObjectClass::Environment { parent })?;
        let before = self.object(env)?.owned_bytes();
        self.object_mut(env)?.elements = Elements::Packed(vec![Value::UNDEFINED; size as usize]);
        let after = self.object(env)?.owned_bytes();
        self.adjust(before, after)?;
        Ok(env)
    }

    /// The enclosing environment.
    pub fn env_parent(&self, env: ObjectRef) -> Result<Option<ObjectRef>, HeapError> {
        match self.object(env)?.class {
            ObjectClass::Environment { parent } => Ok(parent),
            _ => Err(HeapError::Internal(
                "env_parent on an object that isn't an environment".to_string(),
            )),
        }
    }

    /// Read slot `slot` of an environment.
    pub fn env_get(&self, env: ObjectRef, slot: u32) -> Result<Value, HeapError> {
        match &self.object(env)?.elements {
            Elements::Packed(v) => v.get(slot as usize).copied().ok_or_else(|| {
                HeapError::Internal(format!(
                    "environment slot {slot} is past its {} slots",
                    v.len()
                ))
            }),
            Elements::Sparse(_) => Err(HeapError::Internal(
                "environment with sparse slots".to_string(),
            )),
        }
    }

    /// Write slot `slot` of an environment.
    pub fn env_set(&mut self, env: ObjectRef, slot: u32, value: Value) -> Result<(), HeapError> {
        match &mut self.object_mut(env)?.elements {
            Elements::Packed(v) => match v.get_mut(slot as usize) {
                Some(s) => {
                    *s = value;
                    Ok(())
                }
                None => Err(HeapError::Internal(format!(
                    "environment slot {slot} is past its slots"
                ))),
            },
            Elements::Sparse(_) => Err(HeapError::Internal(
                "environment with sparse slots".to_string(),
            )),
        }
    }

    // ---- roots and wrappers -----------------------------------------------

    /// Keep `value` alive until [`Heap::release`].
    pub fn persist(&mut self, value: Value) -> Result<Persistent, HeapError> {
        self.roots.add(value).map_err(HeapError::Internal)
    }

    /// The value behind a persistent root.
    pub fn persistent(&self, p: Persistent) -> Result<Value, HeapError> {
        self.roots.get(p).map_err(HeapError::Internal)
    }

    /// Stop keeping a persistent root's value alive.
    pub fn release(&mut self, p: Persistent) -> Result<(), HeapError> {
        self.roots.release(p).map_err(HeapError::Internal)
    }

    /// Record `obj` as the one wrapper for `data`. The record is weak.
    pub fn set_wrapper(&mut self, data: HostData, obj: ObjectRef) {
        self.wrappers.insert(data, obj);
    }

    /// The live wrapper for `data`, if there is one.
    pub fn wrapper(&self, data: HostData) -> Option<ObjectRef> {
        self.wrappers
            .get(data)
            .filter(|o| self.objects.contains(o.0))
    }

    // ---- collection -------------------------------------------------------

    /// Run a collection now if one was asked for, then check the limit.
    /// The VM calls this at safepoints, passing every live register.
    pub fn safepoint(&mut self, roots: &[Value]) -> Result<Option<CollectStats>, HeapError> {
        let stats = if self.collect_requested {
            Some(self.collect(roots))
        } else {
            None
        };
        if self.bytes > self.limits.max_heap_bytes {
            return Err(HeapError::Abort(Abort {
                kind: LimitKind::HeapBytes,
                reason: format!(
                    "heap limit reached: {} bytes live after collection, limit {} bytes",
                    group_digits(self.bytes as u64),
                    group_digits(self.limits.max_heap_bytes as u64)
                ),
                limit: self.limits.max_heap_bytes as u64,
                observed: self.bytes as u64,
            }));
        }
        Ok(stats)
    }

    /// Collect garbage now. `roots` are the caller's live values; the host's
    /// persistent roots are added on top.
    pub fn collect(&mut self, roots: &[Value]) -> CollectStats {
        let bytes_before = self.bytes;
        let mut work: Vec<Cell> = Vec::new();
        for v in roots {
            push_value(&mut work, *v);
        }

        for v in self.roots.values() {
            push_value(&mut work, v);
        }
        let mut weak: Vec<ObjectRef> = Vec::new();
        loop {
            self.mark_all(&mut work, &mut weak);
            // Ephemerons: a weak entry's value lives when its key does.
            // Marking values can make more keys live, so go round until
            // nothing new turns up.
            for w in &weak {
                if let Ok(Object {
                    class: ObjectClass::Collection(m),
                    ..
                }) = self.objects.get(w.0)
                {
                    for (k, v) in m.entries.iter().flatten() {
                        if let Some(ko) = k.as_object()
                            && self.objects.is_marked(ko.0)
                            && !self.value_marked(*v)
                        {
                            push_value(&mut work, *v);
                        }
                    }
                }
            }
            if work.is_empty() {
                break;
            }
        }
        // Drop weak entries whose keys are about to be swept.
        let mut weak_freed = 0usize;
        for w in &weak {
            let objects = &self.objects;
            let marked: Vec<bool> = match objects.get(w.0) {
                Ok(Object {
                    class: ObjectClass::Collection(m),
                    ..
                }) => m
                    .entries
                    .iter()
                    .flatten()
                    .map(|(k, _)| k.as_object().is_some_and(|o| objects.is_marked(o.0)))
                    .collect(),
                _ => continue,
            };
            if let Ok(Object {
                class: ObjectClass::Collection(m),
                ..
            }) = self.objects.get_mut(w.0)
            {
                let before = m.owned_bytes();
                let mut it = marked.into_iter();
                m.retain_keys(|_| it.next().unwrap_or(false));
                weak_freed += before.saturating_sub(m.owned_bytes());
            }
        }
        self.bytes = self.bytes.saturating_sub(weak_freed);

        let objects = &self.objects;
        let finalized = self.wrappers.sweep(|o| objects.is_marked(o.0));

        let mut freed_bytes = 0usize;
        let mut closed: Vec<ObjectRef> = Vec::new();
        let freed_objects = self.objects.sweep(|_, o| {
            freed_bytes += Arena::<Object>::slot_bytes() + o.owned_bytes();
            if let ObjectClass::CollectionIterator { map: Some(m), .. } = o.class {
                closed.push(m);
            }
        });
        // Iterators that died open no longer hold their collection's gaps.
        for m in closed {
            if let Ok(Object {
                class: ObjectClass::Collection(d),
                ..
            }) = self.objects.get_mut(m.0)
            {
                d.open_iterators = d.open_iterators.saturating_sub(1);
            }
        }
        self.finish_collect(bytes_before, freed_bytes, freed_objects, finalized)
    }

    fn value_marked(&self, v: Value) -> bool {
        if let Some(o) = v.as_object() {
            self.objects.is_marked(o.0)
        } else if let Some(s) = v.as_string() {
            self.strings.is_marked(s.0)
        } else if let Some(y) = v.as_symbol() {
            self.symbols.is_marked(y.0)
        } else {
            true
        }
    }

    fn mark_all(&mut self, work: &mut Vec<Cell>, weak: &mut Vec<ObjectRef>) {
        while let Some(cell) = work.pop() {
            match cell {
                Cell::Object(r) => {
                    if !self.objects.mark(r.0) {
                        continue;
                    }
                    let Ok(o) = self.objects.get(r.0) else {
                        continue;
                    };
                    if let ObjectClass::Collection(m) = &o.class
                        && m.kind.is_weak()
                    {
                        weak.push(r);
                    }
                    match &o.props {
                        Props::Shaped { shape, slots } => {
                            work.push(Cell::Shape(*shape));
                            for s in slots {
                                for v in s.values() {
                                    push_value(work, v);
                                }
                            }
                        }
                        Props::Dictionary(d) => {
                            if let Some(p) = d.proto {
                                work.push(Cell::Object(p));
                            }
                            for (k, p) in d.iter() {
                                push_key(work, k);
                                for v in p.slot.values() {
                                    push_value(work, v);
                                }
                            }
                        }
                    }
                    o.elements.for_each_value(|v| push_value(work, v));
                    o.class.for_each_value(|v| push_value(work, v));
                }
                Cell::String(r) => {
                    if !self.strings.mark(r.0) {
                        continue;
                    }
                    if let Ok(JsString::Rope { left, right, .. }) = self.strings.get(r.0) {
                        work.push(Cell::String(*left));
                        work.push(Cell::String(*right));
                    }
                }
                Cell::Symbol(r) => {
                    if !self.symbols.mark(r.0) {
                        continue;
                    }
                    if let Ok(Symbol {
                        description: Some(d),
                    }) = self.symbols.get(r.0)
                    {
                        work.push(Cell::String(*d));
                    }
                }
                Cell::Shape(r) => {
                    if !self.shapes.mark(r.0) {
                        continue;
                    }
                    if let Ok(s) = self.shapes.get(r.0) {
                        if let Some(p) = s.parent {
                            work.push(Cell::Shape(p));
                        }
                        if let Some(p) = s.proto {
                            work.push(Cell::Object(p));
                        }
                        if let Some(k) = s.key {
                            push_key(work, k);
                        }
                    }
                }
            }
        }
    }

    fn finish_collect(
        &mut self,
        bytes_before: usize,
        mut freed_bytes: usize,
        freed_objects: usize,
        finalized: Vec<HostData>,
    ) -> CollectStats {
        let freed_strings = self.strings.sweep(|_, s| {
            freed_bytes += Arena::<JsString>::slot_bytes() + s.owned_bytes();
        });
        let freed_symbols = self.symbols.sweep(|_, _| {
            freed_bytes += Arena::<Symbol>::slot_bytes();
        });
        let freed_shapes = self.shapes.sweep(|_, s| {
            freed_bytes += Arena::<Shape>::slot_bytes() + s.owned_bytes();
        });

        // Drop edges and root entries that point at shapes that just died.
        // Capacity stays the same, so the byte count does too.
        let live_shapes: std::collections::HashSet<RawHandle> =
            self.shapes.iter().map(|(h, _)| h).collect();
        for (_, s) in self.shapes.iter_mut() {
            s.transitions.retain(|(_, _, c)| live_shapes.contains(&c.0));
        }
        self.root_shapes.retain(|_, r| live_shapes.contains(&r.0));

        self.bytes = self.bytes.saturating_sub(freed_bytes);
        let growth = self.bytes / 100 * self.limits.gc_growth_percent as usize;
        self.next_trigger = growth
            .max(self.limits.initial_gc_trigger)
            .min(self.limits.max_heap_bytes);
        self.collect_requested = false;
        self.stats.collections += 1;
        CollectStats {
            freed_objects,
            freed_strings,
            freed_symbols,
            freed_shapes,
            bytes_before,
            bytes_after: self.bytes,
            finalized,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap() -> Heap {
        Heap::new(Limits::default()).unwrap()
    }

    fn data(heap: &Heap, obj: ObjectRef, key: PropertyKey) -> Option<Value> {
        heap.get_own(obj, key).unwrap().and_then(|p| p.slot.data())
    }

    #[test]
    fn objects_have_identity() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let a = h.new_object(None).unwrap();
        let b = a; // same handle, same object
        h.set_own_value(b, x, Value::int32(2)).unwrap();
        assert_eq!(data(&h, a, x).and_then(Value::as_int32), Some(2));
        let c = h.new_object(None).unwrap();
        assert_ne!(a, c);
        assert!(data(&h, c, x).is_none());
    }

    #[test]
    fn same_keys_same_order_share_a_shape() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let y = h.key_from_str("y").unwrap();
        let a = h.new_object(None).unwrap();
        let b = h.new_object(None).unwrap();
        let c = h.new_object(None).unwrap();
        for o in [a, b] {
            h.set_own_value(o, x, Value::int32(1)).unwrap();
            h.set_own_value(o, y, Value::int32(2)).unwrap();
        }
        h.set_own_value(c, y, Value::int32(1)).unwrap();
        h.set_own_value(c, x, Value::int32(2)).unwrap();
        assert_eq!(h.shape_of(a).unwrap(), h.shape_of(b).unwrap());
        assert_ne!(h.shape_of(a).unwrap(), h.shape_of(c).unwrap());
        let created = h.stats().shapes_created;
        let d = h.new_object(None).unwrap();
        h.set_own_value(d, x, Value::int32(1)).unwrap();
        h.set_own_value(d, y, Value::int32(1)).unwrap();
        assert_eq!(
            h.stats().shapes_created,
            created,
            "existing transitions reused"
        );
        // A different prototype means a different root shape.
        let e = h.new_object(Some(a)).unwrap();
        h.set_own_value(e, x, Value::int32(1)).unwrap();
        assert_ne!(h.shape_of(e).unwrap(), h.shape_of(d).unwrap());
    }

    #[test]
    fn dictionary_mode_past_threshold() {
        let limits = Limits {
            dictionary_threshold: 8,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let o = h.new_object(None).unwrap();
        let keys: Vec<PropertyKey> = (0..20)
            .map(|i| h.key_from_str(&format!("k{i}")).unwrap())
            .collect();
        for (i, k) in keys.iter().enumerate() {
            h.set_own_value(o, *k, Value::int32(i as i32)).unwrap();
            assert_eq!(
                h.object(o).unwrap().is_dictionary(),
                i >= 8,
                "after {} keys",
                i + 1
            );
        }
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(data(&h, o, *k).and_then(Value::as_int32), Some(i as i32));
        }
        assert_eq!(h.own_keys(o).unwrap(), keys, "order survives the switch");
        assert_eq!(h.stats().dictionary_conversions, 1);
    }

    #[test]
    fn dictionary_mode_on_delete_and_attribute_change() {
        let mut h = heap();
        let ks: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|s| h.key_from_str(s).unwrap())
            .collect();
        let o = h.new_object(None).unwrap();
        for k in &ks {
            h.set_own_value(o, *k, Value::TRUE).unwrap();
        }
        let shape_ab = {
            let p = h.new_object(None).unwrap();
            h.set_own_value(p, ks[0], Value::TRUE).unwrap();
            h.set_own_value(p, ks[1], Value::TRUE).unwrap();
            h.shape_of(p).unwrap()
        };
        // Deleting the last key goes back to the parent shape.
        assert!(h.delete_own(o, ks[2]).unwrap());
        assert_eq!(h.shape_of(o).unwrap(), shape_ab);
        // Deleting a middle key goes to dictionary mode.
        assert!(h.delete_own(o, ks[0]).unwrap());
        assert!(h.object(o).unwrap().is_dictionary());
        assert_eq!(h.own_keys(o).unwrap(), vec![ks[1]]);

        let q = h.new_object(None).unwrap();
        h.set_own_value(q, ks[0], Value::int32(1)).unwrap();
        let frozen = Property {
            attrs: Attrs::NONE,
            slot: Slot::Data(Value::int32(1)),
        };
        assert!(h.define_own(q, ks[0], frozen).unwrap());
        assert!(h.object(q).unwrap().is_dictionary());
        assert!(
            !h.set_own_value(q, ks[0], Value::int32(5)).unwrap(),
            "read-only"
        );
        assert!(!h.delete_own(q, ks[0]).unwrap(), "not configurable");
        let loosen = Property::data(Value::int32(1));
        assert!(
            !h.define_own(q, ks[0], loosen).unwrap(),
            "can't make it configurable again"
        );
    }

    #[test]
    fn too_many_transitions_go_to_dictionary() {
        let limits = Limits {
            max_transitions_per_shape: 4,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let mut objs = Vec::new();
        for i in 0..6 {
            let k = h.key_from_str(&format!("only{i}")).unwrap();
            let o = h.new_object(None).unwrap();
            h.set_own_value(o, k, Value::NULL).unwrap();
            objs.push(o);
        }
        let dicts: Vec<bool> = objs
            .iter()
            .map(|o| h.object(*o).unwrap().is_dictionary())
            .collect();
        assert_eq!(dicts, vec![false, false, false, false, true, true]);
    }

    #[test]
    fn prototype_chain_lookup_and_epoch() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let y = h.key_from_str("y").unwrap();
        let base = h.new_object(None).unwrap();
        h.set_own_value(base, x, Value::int32(1)).unwrap();
        let mid = h.new_object(Some(base)).unwrap();
        let leaf = h.new_object(Some(mid)).unwrap();
        match h.lookup(leaf, x).unwrap() {
            Lookup::Found { holder, .. } => assert_eq!(holder, base),
            other => panic!("expected found, got {other:?}"),
        }
        assert!(matches!(h.lookup(leaf, y).unwrap(), Lookup::NotFound));
        assert_eq!(
            h.get_value(leaf, x).unwrap().and_then(Value::as_int32),
            Some(1)
        );
        let epoch = h.prototype_epoch();
        h.set_own_value(mid, x, Value::int32(2)).unwrap();
        assert!(
            h.prototype_epoch() > epoch,
            "adding a key to a prototype bumps the epoch"
        );
        assert_eq!(
            h.get_value(leaf, x).unwrap().and_then(Value::as_int32),
            Some(2)
        );
        let epoch = h.prototype_epoch();
        h.set_own_value(leaf, y, Value::int32(3)).unwrap();
        assert_eq!(h.prototype_epoch(), epoch, "leaf objects don't bump it");
    }

    #[test]
    fn deep_prototype_chain_has_no_hop_cap() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let root = h.new_object(None).unwrap();
        h.set_own_value(root, x, Value::TRUE).unwrap();
        let mut cur = root;
        for _ in 0..1000 {
            cur = h.new_object(Some(cur)).unwrap();
        }
        assert!(h.get_value(cur, x).unwrap().is_some());
    }

    #[test]
    fn set_prototype_rules() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let z = h.key_from_str("z").unwrap();
        let a = h.new_object(None).unwrap();
        let b = h.new_object(Some(a)).unwrap();
        assert!(!h.set_prototype(a, Some(b)).unwrap(), "cycle refused");
        h.set_own_value(b, x, Value::int32(7)).unwrap();
        let c = h.new_object(None).unwrap();
        assert!(h.set_prototype(b, Some(c)).unwrap());
        assert_eq!(h.get_prototype(b).unwrap(), Some(c));
        assert_eq!(
            data(&h, b, x).and_then(Value::as_int32),
            Some(7),
            "own keys survive"
        );
        h.prevent_extensions(b).unwrap();
        assert!(!h.set_prototype(b, None).unwrap());
        assert!(
            !h.set_own_value(b, z, Value::NULL).unwrap(),
            "can't add keys now"
        );
    }

    #[test]
    fn proxies_stop_the_ordinary_walk() {
        let mut h = heap();
        let x = h.key_from_str("x").unwrap();
        let p = h
            .alloc_object(
                None,
                ObjectClass::Proxy {
                    target: None,
                    handler: None,
                },
            )
            .unwrap();
        let o = h.new_object(Some(p)).unwrap();
        assert!(matches!(h.lookup(o, x).unwrap(), Lookup::Exotic(e) if e == p));
        assert!(h.get_value(o, x).is_err());
    }

    #[test]
    fn arrays_track_length_and_order_keys() {
        let mut h = heap();
        let arr = h.new_array(None).unwrap();
        let name = h.key_from_str("name").unwrap();
        let sym = h.alloc_symbol(None).unwrap();
        h.set_own_value(arr, PropertyKey::Symbol(sym), Value::NULL)
            .unwrap();
        h.set_own_value(arr, name, Value::NULL).unwrap();
        for i in [2u32, 0, 1] {
            h.set_own_value(arr, PropertyKey::Index(i), Value::int32(i as i32))
                .unwrap();
        }
        assert_eq!(h.array_length(arr).unwrap(), 3);
        assert_eq!(
            h.own_keys(arr).unwrap(),
            vec![
                PropertyKey::Index(0),
                PropertyKey::Index(1),
                PropertyKey::Index(2),
                name,
                PropertyKey::Symbol(sym)
            ]
        );
        h.set_own_value(arr, PropertyKey::Index(4_000_000_000), Value::TRUE)
            .unwrap();
        assert_eq!(h.array_length(arr).unwrap(), 4_000_000_001);
        assert!(h.set_array_length(arr, 1).unwrap());
        assert_eq!(h.own_keys(arr).unwrap().len(), 3);
        // A non-configurable element stops the shrink.
        let fixed = Property {
            attrs: Attrs(Attrs::WRITABLE | Attrs::ENUMERABLE),
            slot: Slot::Data(Value::TRUE),
        };
        h.define_own(arr, PropertyKey::Index(5), fixed).unwrap();
        assert!(!h.set_array_length(arr, 0).unwrap());
        assert_eq!(h.array_length(arr).unwrap(), 6);
        assert_eq!(h.key_from_str("7").unwrap(), PropertyKey::Index(7));
        assert!(matches!(
            h.key_from_str("07").unwrap(),
            PropertyKey::Atom(_)
        ));
        let plain = h.new_object(None).unwrap();
        assert!(h.array_length(plain).is_err());
    }

    #[test]
    fn strings_concat_and_flatten() {
        let limits = Limits {
            rope_min_length: 4,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let a = h.alloc_string("ab").unwrap();
        let b = h.alloc_string("c").unwrap();
        let short = h.concat(a, b).unwrap();
        assert!(!h.is_rope(short).unwrap());
        let mut s = h.alloc_string("\u{1F600}").unwrap();
        for _ in 0..5 {
            s = h.concat(s, a).unwrap();
        }
        assert!(h.is_rope(s).unwrap());
        assert_eq!(h.string_len(s).unwrap(), 12);
        assert_eq!(h.char_code_at(s, 0).unwrap(), Some(0xD83D));
        assert_eq!(h.char_code_at(s, 1).unwrap(), Some(0xDE00));
        assert_eq!(h.char_code_at(s, 2).unwrap(), Some(u16::from(b'a')));
        assert_eq!(h.string_to_rust(s).unwrap(), "\u{1F600}ababababab");
        assert_eq!(h.string_units(s).unwrap().len(), 12);
        assert!(!h.is_rope(s).unwrap(), "flattened in place");
        assert_eq!(h.stats().ropes_flattened, 1);
        assert_eq!(h.bytes(), h.recount_bytes());
        let k = h.key_from_string(s).unwrap();
        assert!(matches!(k, PropertyKey::Atom(_)));
    }

    #[test]
    fn string_length_limit_is_a_range_error() {
        let limits = Limits {
            max_string_length: 10,
            rope_min_length: 2,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let a = h.alloc_string("123456").unwrap();
        let err = h.concat(a, a).unwrap_err();
        match err {
            HeapError::Range(m) => {
                assert_eq!(m, "Invalid string length: 12 code units, limit 10");
            }
            other => panic!("expected RangeError, got {other:?}"),
        }
        assert!(matches!(
            h.alloc_string("12345678901"),
            Err(HeapError::Range(_))
        ));
    }

    #[test]
    fn symbols_and_descriptions_survive_collection() {
        let mut h = heap();
        let d = h.alloc_string("tag").unwrap();
        let s = h.alloc_symbol(Some(d)).unwrap();
        h.collect(&[Value::symbol(s)]);
        assert_eq!(h.symbol_description(s).unwrap(), Some(d));
        assert_eq!(h.string_to_rust(d).unwrap(), "tag");
        h.collect(&[]);
        assert!(h.symbol_description(s).is_err());
        assert!(h.string_to_rust(d).is_err());
    }

    #[test]
    fn accessors_are_kept_and_traced() {
        let mut h = heap();
        let k = h.key_from_str("v").unwrap();
        let o = h.new_object(None).unwrap();
        let getter = h.alloc_object(None, ObjectClass::Native { id: 1 }).unwrap();
        let prop = Property {
            attrs: Attrs(Attrs::CONFIGURABLE),
            slot: Slot::Accessor {
                get: Value::object(getter),
                set: Value::UNDEFINED,
            },
        };
        assert!(h.define_own(o, k, prop).unwrap());
        assert!(
            !h.set_own_value(o, k, Value::NULL).unwrap(),
            "plain write can't hit an accessor"
        );
        assert!(h.get_value(o, k).is_err(), "the VM has to call getters");
        h.collect(&[Value::object(o)]);
        assert!(h.is_live(getter));
        assert!(h.object(getter).unwrap().is_callable());
    }

    #[test]
    fn persistent_roots_and_wrappers() {
        let mut h = heap();
        let kept = h.new_object(None).unwrap();
        let wrapper = h
            .alloc_object(None, ObjectClass::Host(HostData(5)))
            .unwrap();
        h.set_wrapper(HostData(5), wrapper);
        let p = h.persist(Value::object(kept)).unwrap();
        let stats = h.collect(&[]);
        assert!(h.is_live(kept));
        assert!(!h.is_live(wrapper));
        assert_eq!(stats.finalized, vec![HostData(5)]);
        assert_eq!(h.wrapper(HostData(5)), None);
        assert!(h.persistent(p).unwrap().as_object() == Some(kept));
        h.release(p).unwrap();
        h.collect(&[]);
        assert!(!h.is_live(kept));
    }

    #[test]
    fn wrapper_identity_while_alive() {
        let mut h = heap();
        let w = h
            .alloc_object(None, ObjectClass::Host(HostData(9)))
            .unwrap();
        h.set_wrapper(HostData(9), w);
        h.collect(&[Value::object(w)]);
        assert_eq!(h.wrapper(HostData(9)), Some(w));
    }

    #[test]
    fn heap_limit_aborts_with_reason() {
        let limits = Limits {
            max_heap_bytes: 64 * 1024,
            initial_gc_trigger: 16 * 1024,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let mut keep = Vec::new();
        let err = loop {
            match h.new_object(None) {
                Ok(o) => keep.push(Value::object(o)),
                Err(e) => break e,
            }
            if keep.len() > 1_000_000 {
                panic!("limit never hit");
            }
        };
        match err {
            HeapError::Abort(a) => {
                assert_eq!(a.kind, LimitKind::HeapBytes);
                assert!(a.reason.starts_with("heap limit reached:"), "{}", a.reason);
                assert!(a.reason.contains("limit 65,536 bytes"), "{}", a.reason);
                assert_eq!(a.limit, 65_536);
                assert!(a.observed > a.limit);
            }
            other => panic!("expected an abort, got {other:?}"),
        }
        assert!(h.collect_requested());
        // Everything is still rooted, so the safepoint collects and still
        // can't make room; but it is under the limit so it does not abort.
        assert!(h.safepoint(&keep).is_ok());
        assert_eq!(h.live_objects(), keep.len());
        // Drop the roots and there is room again.
        h.collect(&[]);
        assert!(h.new_object(None).is_ok());
    }

    #[test]
    fn safepoint_collects_only_when_asked() {
        let limits = Limits {
            initial_gc_trigger: 4096,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        assert_eq!(h.safepoint(&[]).unwrap(), None);
        while !h.collect_requested() {
            h.new_object(None).unwrap();
        }
        let stats = h.safepoint(&[]).unwrap().expect("a collection ran");
        assert!(stats.freed_objects > 0);
        assert_eq!(h.live_objects(), 0);
        assert!(!h.collect_requested());
    }

    #[test]
    fn stale_object_handle_is_an_internal_error() {
        let mut h = heap();
        let o = h.new_object(None).unwrap();
        h.collect(&[]);
        let fresh = h.new_object(None).unwrap();
        assert_eq!(fresh.0.index, o.0.index, "slot reused");
        let err = h.object(o).unwrap_err();
        match err {
            HeapError::Internal(m) => assert!(m.contains("stale handle"), "{m}"),
            other => panic!("expected internal error, got {other:?}"),
        }
        let x = h.key_from_str("x").unwrap();
        assert!(h.set_own_value(o, x, Value::NULL).is_err());
        assert!(h.set_own_value(fresh, x, Value::NULL).unwrap());
    }

    #[test]
    fn bad_limits_are_refused() {
        let limits = Limits {
            gc_growth_percent: 10,
            ..Limits::default()
        };
        assert!(Heap::new(limits).is_err());
    }

    #[test]
    fn errors_display_plainly() {
        let e = HeapError::Range("Invalid string length".to_string());
        assert_eq!(e.to_string(), "RangeError: Invalid string length");
        let e = HeapError::Internal("x".to_string());
        assert_eq!(e.to_string(), "internal error: x");
    }
}
