// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Roots the host holds, and weak wrapper tables.
//!
//! Inside a call, values in registers are roots on their own. A host that
//! wants to keep a value past the call (an event listener, say) takes a
//! [`Persistent`] for it. It stays a root until released.
//!
//! The wrapper table maps host data (a node id, for example) to the one
//! object that stands for it, so the same node always gives the same object.
//! The table is weak: it never keeps a wrapper alive, and a wrapper that
//! dies is reported back so the host can drop its side.

use crate::anvil_js::value::object::HostData;
use crate::anvil_js::value::{ObjectRef, Value};
use std::collections::HashMap;

/// A host-held root. Release it with the heap's `release`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Persistent {
    index: u32,
    generation: u32,
}

#[derive(Debug)]
struct RootSlot {
    generation: u32,
    value: Option<Value>,
}

/// The set of host-held roots.
#[derive(Debug, Default)]
pub struct RootTable {
    slots: Vec<RootSlot>,
    free: Vec<u32>,
    live: usize,
}

impl RootTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live roots.
    pub fn len(&self) -> usize {
        self.live
    }

    /// True when there are no roots.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Keep `value` alive until released.
    pub fn add(&mut self, value: Value) -> Result<Persistent, String> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            self.live += 1;
            return Ok(Persistent {
                index,
                generation: slot.generation,
            });
        }
        let index = u32::try_from(self.slots.len())
            .map_err(|_| "all persistent root slots are in use".to_string())?;
        self.slots.push(RootSlot {
            generation: 0,
            value: Some(value),
        });
        self.live += 1;
        Ok(Persistent {
            index,
            generation: 0,
        })
    }

    /// The value behind a root, or a reason it is gone.
    pub fn get(&self, p: Persistent) -> Result<Value, String> {
        match self.slots.get(p.index as usize) {
            Some(slot) if slot.generation == p.generation => slot
                .value
                .ok_or_else(|| format!("persistent root {} was released", p.index)),
            Some(_) => Err(format!(
                "persistent root {} was released and reused",
                p.index
            )),
            None => Err(format!("persistent root {} does not exist", p.index)),
        }
    }

    /// Change the value behind a root.
    pub fn set(&mut self, p: Persistent, value: Value) -> Result<(), String> {
        self.get(p)?;
        self.slots[p.index as usize].value = Some(value);
        Ok(())
    }

    /// Stop keeping a value alive.
    pub fn release(&mut self, p: Persistent) -> Result<(), String> {
        self.get(p)?;
        let slot = &mut self.slots[p.index as usize];
        slot.value = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(p.index);
        self.live -= 1;
        Ok(())
    }

    /// Every rooted value.
    pub fn values(&self) -> impl Iterator<Item = Value> + '_ {
        self.slots.iter().filter_map(|s| s.value)
    }
}

/// Weak map from host data to its wrapper object.
#[derive(Debug, Default)]
pub struct WrapperTable {
    map: HashMap<HostData, ObjectRef>,
}

impl WrapperTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Record `obj` as the wrapper for `data`.
    pub fn insert(&mut self, data: HostData, obj: ObjectRef) {
        self.map.insert(data, obj);
    }

    /// The wrapper for `data`, if recorded.
    pub fn get(&self, data: HostData) -> Option<ObjectRef> {
        self.map.get(&data).copied()
    }

    /// Drop every entry whose wrapper `is_live` says is dead, and return
    /// their host data, sorted so the order is stable.
    pub fn sweep(&mut self, mut is_live: impl FnMut(ObjectRef) -> bool) -> Vec<HostData> {
        let mut dead: Vec<HostData> = self
            .map
            .iter()
            .filter(|(_, o)| !is_live(**o))
            .map(|(d, _)| *d)
            .collect();
        dead.sort();
        for d in &dead {
            self.map.remove(d);
        }
        dead
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anvil_js::gc::arena::RawHandle;

    #[test]
    fn persistent_roots_add_get_release() {
        let mut t = RootTable::new();
        let a = t.add(Value::int32(1)).unwrap();
        let b = t.add(Value::int32(2)).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(a).unwrap().as_int32(), Some(1));
        t.set(b, Value::int32(3)).unwrap();
        assert_eq!(t.get(b).unwrap().as_int32(), Some(3));
        t.release(a).unwrap();
        assert!(t.get(a).unwrap_err().contains("released"));
        assert!(t.release(a).is_err(), "double release says so");
        let c = t.add(Value::NULL).unwrap();
        assert!(t.get(a).is_err(), "old root does not see the reused slot");
        assert!(t.get(c).unwrap().is_null());
        assert_eq!(t.values().count(), 2);
    }

    #[test]
    fn wrapper_table_is_weak() {
        let mut w = WrapperTable::new();
        let o1 = ObjectRef(RawHandle {
            index: 1,
            generation: 0,
        });
        let o2 = ObjectRef(RawHandle {
            index: 2,
            generation: 0,
        });
        w.insert(HostData(10), o1);
        w.insert(HostData(20), o2);
        assert_eq!(w.get(HostData(10)), Some(o1));
        let dead = w.sweep(|o| o == o1);
        assert_eq!(dead, vec![HostData(20)]);
        assert_eq!(w.len(), 1);
        assert_eq!(w.get(HostData(20)), None);
    }
}
