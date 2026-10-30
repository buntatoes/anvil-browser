// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Arrays (length, push, pop, index) and objects with properties in
// insertion order.
// needs: plain
// expect: 3
// expect: 4
// expect: 2
// expect: 1
// expect: two
export function main() {
  const a = [1, 2];
  a.push(3);
  const n = a.length;
  a[3] = 4;
  const last = a.pop();
  const o = { one: 1, two: 2 };
  o.three = 3;
  return [n, last, o.two, o["one"], "two"];
}
