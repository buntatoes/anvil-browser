// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Map and Set: SameValueZero keys, insertion order, size.
// needs: collections
// expect: 2
// expect: 2
// expect: 2
// expect: a1b2
export function main() {
  const m = new Map();
  m.set(NaN, 1);
  m.set(NaN, 2);
  m.set(0, 3);
  const s = new Set();
  s.add(-0);
  s.add(0);
  s.add("0");
  const order = new Map();
  order.set("a", 1);
  order.set("b", 2);
  let text = "";
  order.forEach((v, k) => {
    text = text + k + v;
  });
  return [m.size, m.get(NaN), s.size, text];
}
