// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// A top-level function passed around as a value.
// needs: closures
// expect: 9
function square(x) {
  return x * x;
}
function apply(f, x) {
  return f(x);
}
export function main() {
  return [apply(square, 3)];
}
