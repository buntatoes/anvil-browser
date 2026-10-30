// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Names (arrays of UTF-16 code units) compared by value, and hashed maps
// from a name to a value. Plain arrays and records only.

export function sameUnits(a, b) {
  if (a.length !== b.length) {
    return false;
  }
  let i = 0;
  while (i < a.length) {
    if (a[i] !== b[i]) {
      return false;
    }
    i = i + 1;
  }
  return true;
}

export function hashUnits(units) {
  let h = units.length;
  let i = 0;
  while (i < units.length) {
    h = (h * 31 + units[i]) | 0;
    i = i + 1;
  }
  return h & 0x7fffffff;
}

function emptyBuckets(n) {
  const buckets = [];
  let i = 0;
  while (i < n) {
    buckets.push([]);
    i = i + 1;
  }
  return buckets;
}

// A map from names to values. Each bucket holds { name, value } entries.
export function newNameMap() {
  return { buckets: emptyBuckets(8), count: 0 };
}

function entryOf(m, units) {
  const b = m.buckets[hashUnits(units) % m.buckets.length];
  let i = 0;
  while (i < b.length) {
    if (sameUnits(b[i].name, units)) {
      return b[i];
    }
    i = i + 1;
  }
  return null;
}

// The value for a name, or null.
export function nameMapGet(m, units) {
  const e = entryOf(m, units);
  if (e === null) {
    return null;
  }
  return e.value;
}

function grow(m) {
  const old = m.buckets;
  const buckets = emptyBuckets(old.length * 4);
  let i = 0;
  while (i < old.length) {
    const b = old[i];
    let j = 0;
    while (j < b.length) {
      buckets[hashUnits(b[j].name) % buckets.length].push(b[j]);
      j = j + 1;
    }
    i = i + 1;
  }
  m.buckets = buckets;
}

// Set the value for a name (added when new).
export function nameMapSet(m, units, value) {
  const e = entryOf(m, units);
  if (e !== null) {
    e.value = value;
    return;
  }
  if (m.count > m.buckets.length * 2) {
    grow(m);
  }
  m.buckets[hashUnits(units) % m.buckets.length].push({ name: units, value: value });
  m.count = m.count + 1;
}

// Sets of names are maps to `true`.
export function newNameSet() {
  return newNameMap();
}

export function nameSetHas(s, units) {
  return entryOf(s, units) !== null;
}

// Add a name; false when it was already there.
export function nameSetAdd(s, units) {
  if (entryOf(s, units) !== null) {
    return false;
  }
  nameMapSet(s, units, true);
  return true;
}
