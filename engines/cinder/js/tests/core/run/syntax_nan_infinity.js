// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The names NaN and Infinity, and NaN never equal to itself.
// needs: syntax
// expect: true
// expect: false
// expect: true
// expect: true
export function main() {
  const x = NaN;
  return [x !== x, NaN === NaN, 1 / 0 === Infinity, -Infinity < -1e308];
}
