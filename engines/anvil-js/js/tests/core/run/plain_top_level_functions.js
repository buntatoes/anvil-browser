// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Top-level functions calling each other, recursion, and top-level state.
// needs: plain
// expect: 55
// expect: 3
let calls = 0;
function fib(n) {
  calls++;
  if (n < 2) {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}
function count() {
  return calls;
}
export function main() {
  const f = fib(10);
  calls = 0;
  fib(3);
  return [f, count() - 2];
}
