// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// for-of over an array.
// needs: syntax
// expect: 6
export function main() {
  let s = 0;
  for (const x of [1, 2, 3]) {
    s += x;
  }
  return [s];
}
