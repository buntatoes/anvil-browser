// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Labeled break and continue.
// needs: syntax
// expect: 3
export function main() {
  let hits = 0;
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      if (j === 1) {
        continue outer;
      }
      if (i === 5) {
        break outer;
      }
      hits++;
    }
  }
  return [hits];
}
