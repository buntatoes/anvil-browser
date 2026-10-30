// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! A slot arena with generation-checked handles.
//!
//! Each kind of heap cell lives in its own [`Arena`]. A [`RawHandle`] is a
//! slot index plus the generation the slot had when the cell was made. When
//! a slot is freed its generation goes up by one, so an old handle no longer
//! matches and looking it up gives an error instead of whatever lives there
//! now. A slot whose generation would wrap is retired and never handed out
//! again, so an old handle can't match by accident later.

use std::fmt;

/// Generations fit in 15 bits so a handle packs into a value's 47-bit payload.
pub const MAX_GENERATION: u16 = 0x7FFF;

/// Slot index plus generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawHandle {
    /// Slot index in the arena.
    pub index: u32,
    /// Generation of the slot when the handle was made (0 to [`MAX_GENERATION`]).
    pub generation: u16,
}

impl RawHandle {
    /// Pack into 47 bits: index in the low 32, generation above it.
    pub fn to_payload(self) -> u64 {
        u64::from(self.index) | (u64::from(self.generation & MAX_GENERATION) << 32)
    }

    /// Unpack from [`RawHandle::to_payload`].
    pub fn from_payload(payload: u64) -> RawHandle {
        RawHandle {
            index: (payload & 0xFFFF_FFFF) as u32,
            generation: ((payload >> 32) as u16) & MAX_GENERATION,
        }
    }
}

impl fmt::Display for RawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "slot {}, generation {}", self.index, self.generation)
    }
}

/// Why a handle lookup failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// The index is past the end of the arena.
    OutOfRange {
        /// The handle that was looked up.
        handle: RawHandle,
        /// Number of slots in the arena.
        len: usize,
    },
    /// The slot is free (or retired).
    Freed {
        /// The handle that was looked up.
        handle: RawHandle,
    },
    /// The slot holds a newer cell.
    Stale {
        /// The handle that was looked up.
        handle: RawHandle,
        /// The slot's current generation.
        current: u16,
    },
    /// Every slot index is in use.
    Full,
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArenaError::OutOfRange { handle, len } => {
                write!(f, "handle out of range: {handle}, arena has {len} slots")
            }
            ArenaError::Freed { handle } => write!(f, "handle to a freed slot: {handle}"),
            ArenaError::Stale { handle, current } => write!(
                f,
                "stale handle: {handle}, the slot is now at generation {current}"
            ),
            ArenaError::Full => write!(f, "all {} slot indices are in use", u32::MAX),
        }
    }
}

#[derive(Debug)]
struct Entry<T> {
    generation: u16,
    marked: bool,
    retired: bool,
    value: Option<T>,
}

/// A growable set of slots, each either holding a `T` or free.
#[derive(Debug)]
pub struct Arena<T> {
    entries: Vec<Entry<T>>,
    free: Vec<u32>,
    live: usize,
    retired: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    /// An empty arena.
    pub fn new() -> Self {
        Arena {
            entries: Vec::new(),
            free: Vec::new(),
            live: 0,
            retired: 0,
        }
    }

    /// Bytes one slot takes in the arena itself, not counting what `T` owns.
    pub const fn slot_bytes() -> usize {
        std::mem::size_of::<Entry<T>>()
    }

    /// Number of live cells.
    pub fn live(&self) -> usize {
        self.live
    }

    /// Number of slots, live or not.
    pub fn slots(&self) -> usize {
        self.entries.len()
    }

    /// Number of slots retired after their generation ran out.
    pub fn retired(&self) -> usize {
        self.retired
    }

    /// Store `value` and return its handle. Reuses a free slot when there is one.
    pub fn insert(&mut self, value: T) -> Result<RawHandle, ArenaError> {
        if let Some(index) = self.free.pop() {
            let entry = &mut self.entries[index as usize];
            entry.value = Some(value);
            entry.marked = false;
            self.live += 1;
            return Ok(RawHandle {
                index,
                generation: entry.generation,
            });
        }
        let index = u32::try_from(self.entries.len()).map_err(|_| ArenaError::Full)?;
        if index == u32::MAX {
            return Err(ArenaError::Full);
        }
        self.entries.push(Entry {
            generation: 0,
            marked: false,
            retired: false,
            value: Some(value),
        });
        self.live += 1;
        Ok(RawHandle {
            index,
            generation: 0,
        })
    }

    fn entry(&self, handle: RawHandle) -> Result<&Entry<T>, ArenaError> {
        let len = self.entries.len();
        let entry = self
            .entries
            .get(handle.index as usize)
            .ok_or(ArenaError::OutOfRange { handle, len })?;
        if entry.generation != handle.generation {
            return Err(ArenaError::Stale {
                handle,
                current: entry.generation,
            });
        }
        if entry.value.is_none() {
            return Err(ArenaError::Freed { handle });
        }
        Ok(entry)
    }

    fn entry_mut(&mut self, handle: RawHandle) -> Result<&mut Entry<T>, ArenaError> {
        let len = self.entries.len();
        let entry = self
            .entries
            .get_mut(handle.index as usize)
            .ok_or(ArenaError::OutOfRange { handle, len })?;
        if entry.generation != handle.generation {
            return Err(ArenaError::Stale {
                handle,
                current: entry.generation,
            });
        }
        if entry.value.is_none() {
            return Err(ArenaError::Freed { handle });
        }
        Ok(entry)
    }

    /// Look up a live cell.
    pub fn get(&self, handle: RawHandle) -> Result<&T, ArenaError> {
        match &self.entry(handle)?.value {
            Some(v) => Ok(v),
            None => Err(ArenaError::Freed { handle }),
        }
    }

    /// Look up a live cell for changing it.
    pub fn get_mut(&mut self, handle: RawHandle) -> Result<&mut T, ArenaError> {
        match &mut self.entry_mut(handle)?.value {
            Some(v) => Ok(v),
            None => Err(ArenaError::Freed { handle }),
        }
    }

    /// True when `handle` points at a live cell.
    pub fn contains(&self, handle: RawHandle) -> bool {
        self.entry(handle).is_ok()
    }

    /// Free a cell now and hand back its value.
    pub fn remove(&mut self, handle: RawHandle) -> Result<T, ArenaError> {
        self.entry(handle)?;
        self.free_slot(handle.index)
            .ok_or(ArenaError::Freed { handle })
    }

    /// Free the slot at `index`. Gives `None` (and changes nothing) when the
    /// slot is already free.
    fn free_slot(&mut self, index: u32) -> Option<T> {
        let entry = self.entries.get_mut(index as usize)?;
        let value = entry.value.take()?;
        entry.marked = false;
        if entry.generation >= MAX_GENERATION {
            entry.retired = true;
            self.retired += 1;
        } else {
            entry.generation += 1;
            self.free.push(index);
        }
        self.live -= 1;
        Some(value)
    }

    /// Set the mark bit. Returns true when it was not set before, which
    /// means the caller should trace the cell's children.
    pub fn mark(&mut self, handle: RawHandle) -> bool {
        match self.entry_mut(handle) {
            Ok(entry) if !entry.marked => {
                entry.marked = true;
                true
            }
            _ => false,
        }
    }

    /// True when the cell is live and marked.
    pub fn is_marked(&self, handle: RawHandle) -> bool {
        self.entry(handle).map(|e| e.marked).unwrap_or(false)
    }

    /// Free every live cell that is not marked, clear the marks on the rest,
    /// and call `on_free` with each freed handle and value.
    pub fn sweep(&mut self, mut on_free: impl FnMut(RawHandle, T)) -> usize {
        let mut freed = 0;
        for i in 0..self.entries.len() {
            let entry = &mut self.entries[i];
            if entry.value.is_none() {
                continue;
            }
            if entry.marked {
                entry.marked = false;
                continue;
            }
            let handle = RawHandle {
                index: i as u32,
                generation: entry.generation,
            };
            if let Some(value) = self.free_slot(i as u32) {
                on_free(handle, value);
                freed += 1;
            }
        }
        freed
    }

    /// Visit every live cell.
    pub fn iter(&self) -> impl Iterator<Item = (RawHandle, &T)> {
        self.entries.iter().enumerate().filter_map(|(i, e)| {
            e.value.as_ref().map(|v| {
                (
                    RawHandle {
                        index: i as u32,
                        generation: e.generation,
                    },
                    v,
                )
            })
        })
    }

    /// Visit every live cell for changing it.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (RawHandle, &mut T)> {
        self.entries.iter_mut().enumerate().filter_map(|(i, e)| {
            let generation = e.generation;
            e.value.as_mut().map(|v| {
                (
                    RawHandle {
                        index: i as u32,
                        generation,
                    },
                    v,
                )
            })
        })
    }

    /// True when the slot at `index` has been retired.
    pub fn is_retired(&self, index: u32) -> bool {
        self.entries
            .get(index as usize)
            .map(|e| e.retired)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove() {
        let mut arena = Arena::new();
        let a = arena.insert("a").unwrap();
        let b = arena.insert("b").unwrap();
        assert_eq!(arena.get(a), Ok(&"a"));
        assert_eq!(arena.get(b), Ok(&"b"));
        assert_eq!(arena.live(), 2);
        assert_eq!(arena.remove(a), Ok("a"));
        assert_eq!(arena.live(), 1);
        assert_eq!(
            arena.get(a),
            Err(ArenaError::Stale {
                handle: a,
                current: 1
            }),
            "freeing bumps the generation"
        );
    }

    #[test]
    fn reused_slot_rejects_old_handle() {
        let mut arena = Arena::new();
        let a = arena.insert(1).unwrap();
        arena.remove(a).unwrap();
        let c = arena.insert(2).unwrap();
        assert_eq!(c.index, a.index);
        assert_eq!(c.generation, a.generation + 1);
        let err = arena.get(a).unwrap_err();
        assert_eq!(
            err,
            ArenaError::Stale {
                handle: a,
                current: 1
            }
        );
        assert!(err.to_string().contains("stale handle"), "{err}");
        assert_eq!(arena.get(c), Ok(&2));
    }

    #[test]
    fn out_of_range_handle() {
        let arena: Arena<u8> = Arena::new();
        let h = RawHandle {
            index: 5,
            generation: 0,
        };
        assert!(matches!(arena.get(h), Err(ArenaError::OutOfRange { .. })));
    }

    #[test]
    fn generation_wrap_retires_slot() {
        let mut arena = Arena::new();
        let first = arena.insert(0u32).unwrap();
        let mut last = first;
        for i in 0..u32::from(MAX_GENERATION) {
            arena.remove(last).unwrap();
            last = arena.insert(i + 1).unwrap();
            assert_eq!(last.index, first.index);
        }
        assert_eq!(last.generation, MAX_GENERATION);
        arena.remove(last).unwrap();
        assert!(arena.is_retired(first.index));
        assert_eq!(arena.retired(), 1);
        let next = arena.insert(99).unwrap();
        assert_ne!(next.index, first.index, "a retired slot must not come back");
        assert!(arena.get(first).is_err());
        assert!(arena.get(last).is_err());
    }

    #[test]
    fn sweep_frees_unmarked_and_clears_marks() {
        let mut arena = Arena::new();
        let hs: Vec<_> = (0..10).map(|i| arena.insert(i).unwrap()).collect();
        for h in hs.iter().step_by(2) {
            assert!(arena.mark(*h));
            assert!(!arena.mark(*h), "second mark reports already marked");
        }
        let mut freed = Vec::new();
        assert_eq!(arena.sweep(|_, v| freed.push(v)), 5);
        assert_eq!(freed, vec![1, 3, 5, 7, 9]);
        for (i, h) in hs.iter().enumerate() {
            assert_eq!(arena.contains(*h), i % 2 == 0);
            assert!(!arena.is_marked(*h));
        }
        assert_eq!(arena.sweep(|_, _| {}), 5, "marks were cleared");
        assert_eq!(arena.live(), 0);
    }

    #[test]
    fn payload_round_trip() {
        let h = RawHandle {
            index: u32::MAX - 1,
            generation: MAX_GENERATION,
        };
        assert!(h.to_payload() < (1u64 << 47));
        assert_eq!(RawHandle::from_payload(h.to_payload()), h);
    }
}
