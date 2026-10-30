// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// === and !== on numbers, objects and mixed types compare without
// conversion. (On strings they need the strings level, on functions closures.)
// needs: plain
// expect: false
// expect: true
// expect: true
// expect: false
// expect: true
export function main() {
  const o = {};
  const p = {};
  return [1 === true, 0 === -0, o === o, o === p, null !== undefined];
}
