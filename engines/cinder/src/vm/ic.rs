// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Inline caches.
//!
//! Each property site in the bytecode has an [`IcSite`]. It starts empty,
//! holds one entry after the first cacheable access, up to
//! [`MAX_POLY`] entries after that, and gives up (megamorphic) when a fifth
//! kind of receiver shows up. A megamorphic site always takes the generic
//! path.
//!
//! What an entry records, and how it is checked:
//!
//! - **Own:** the receiver's shape. Same shape, same slot.
//! - **Proto:** the receiver's shape, the holder and its shape, and the
//!   heap's prototype epoch. The receiver's shape pins its prototype (each
//!   prototype has its own root shape) and proves the key isn't own. The
//!   epoch goes up whenever any prototype object gains or loses a key or
//!   gets a new prototype, so a matching epoch proves nothing on the chain
//!   changed.
//! - **AddProp:** a store that added a key. It is keyed on the shape
//!   *before* the store and records the shape after, plus the epoch (so a
//!   setter or read-only property that later appears on the chain is
//!   noticed). The heap re-checks that the transition is still live and the
//!   object is still extensible before applying it.
//! - **ArrayLength / StringLength:** `length` of an array or a string.
//!
//! Accessors, read-only properties, dictionary-mode objects and exotic
//! objects (proxies) are never cached.

use crate::cinder::value::{ObjectRef, ShapeRef};

/// Most entries before a site goes megamorphic.
pub const MAX_POLY: usize = 4;

/// One cached case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcEntry {
    /// An own data property.
    Own {
        /// Receiver shape.
        shape: ShapeRef,
        /// Slot in the receiver.
        slot: u32,
    },
    /// A data property on a prototype.
    Proto {
        /// Receiver shape.
        shape: ShapeRef,
        /// The object that has the property.
        holder: ObjectRef,
        /// Its shape.
        holder_shape: ShapeRef,
        /// Slot in the holder.
        slot: u32,
        /// Prototype epoch when recorded.
        epoch: u64,
    },
    /// A store that adds a property.
    AddProp {
        /// Receiver shape before the store.
        from: ShapeRef,
        /// Receiver shape after it.
        to: ShapeRef,
        /// Prototype epoch when recorded.
        epoch: u64,
    },
    /// `array.length`.
    ArrayLength,
    /// `string.length`.
    StringLength,
}

impl IcEntry {
    /// Entries with the same key describe the same kind of receiver; a new
    /// one replaces an old (stale) one.
    fn same_receiver(&self, other: &IcEntry) -> bool {
        match (self, other) {
            (IcEntry::Own { shape: a, .. }, IcEntry::Own { shape: b, .. })
            | (IcEntry::Own { shape: a, .. }, IcEntry::Proto { shape: b, .. })
            | (IcEntry::Proto { shape: a, .. }, IcEntry::Own { shape: b, .. })
            | (IcEntry::Proto { shape: a, .. }, IcEntry::Proto { shape: b, .. })
            | (IcEntry::Own { shape: a, .. }, IcEntry::AddProp { from: b, .. })
            | (IcEntry::AddProp { from: a, .. }, IcEntry::Own { shape: b, .. })
            | (IcEntry::AddProp { from: a, .. }, IcEntry::AddProp { from: b, .. }) => a == b,
            (IcEntry::ArrayLength, IcEntry::ArrayLength)
            | (IcEntry::StringLength, IcEntry::StringLength) => true,
            _ => false,
        }
    }
}

/// A site's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcState {
    /// Nothing cached yet.
    Empty,
    /// One entry.
    Mono,
    /// Two to [`MAX_POLY`] entries.
    Poly,
    /// Gave up; always generic.
    Mega,
}

/// One property site's cache.
#[derive(Clone, Debug, Default)]
pub struct IcSite {
    pub(crate) entries: Vec<IcEntry>,
    pub(crate) mega: bool,
    /// Accesses answered from the cache.
    pub hits: u64,
    /// Accesses that took the generic path.
    pub misses: u64,
}

impl IcSite {
    /// The current state.
    pub fn state(&self) -> IcState {
        if self.mega {
            IcState::Mega
        } else {
            match self.entries.len() {
                0 => IcState::Empty,
                1 => IcState::Mono,
                _ => IcState::Poly,
            }
        }
    }

    /// The cached entries.
    pub fn entries(&self) -> &[IcEntry] {
        &self.entries
    }

    /// Remember `entry`. An entry for the same receiver replaces the old
    /// one; one too many sends the site megamorphic.
    pub(crate) fn record(&mut self, entry: IcEntry) {
        if self.mega {
            return;
        }
        self.entries.retain(|e| !e.same_receiver(&entry));
        if self.entries.len() >= MAX_POLY {
            self.entries.clear();
            self.mega = true;
            return;
        }
        self.entries.push(entry);
    }
}

/// Counters across all sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IcStats {
    /// Cache hits.
    pub hits: u64,
    /// Cache misses.
    pub misses: u64,
    /// Sites with nothing cached.
    pub empty: u32,
    /// Sites with one entry.
    pub mono: u32,
    /// Sites with several entries.
    pub poly: u32,
    /// Sites that gave up.
    pub mega: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cinder::gc::arena::RawHandle;

    fn shape(i: u32) -> ShapeRef {
        ShapeRef(RawHandle {
            index: i,
            generation: 0,
        })
    }

    #[test]
    fn states_go_empty_mono_poly_mega() {
        let mut s = IcSite::default();
        assert_eq!(s.state(), IcState::Empty);
        s.record(IcEntry::Own {
            shape: shape(1),
            slot: 0,
        });
        assert_eq!(s.state(), IcState::Mono);
        // The same receiver shape replaces, it doesn't add.
        s.record(IcEntry::Own {
            shape: shape(1),
            slot: 2,
        });
        assert_eq!(s.entries().len(), 1);
        for i in 2..=4 {
            s.record(IcEntry::Own {
                shape: shape(i),
                slot: 0,
            });
        }
        assert_eq!(s.state(), IcState::Poly);
        assert_eq!(s.entries().len(), 4);
        s.record(IcEntry::Own {
            shape: shape(5),
            slot: 0,
        });
        assert_eq!(s.state(), IcState::Mega);
        assert!(s.entries().is_empty());
        s.record(IcEntry::ArrayLength);
        assert_eq!(s.state(), IcState::Mega);
    }
}
