// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The rest of the extra built-ins, in the stand-in.
// needs: plain
// builtins: yes
// expect: 1024
// expect: NaN
// expect: -3
// expect: -1
// expect: -5
// expect: true
// expect: false
// expect: false
// expect: true
// expect: true
export function main() {
  return [Math.pow(2, 10), Math.pow(1, 1 / 0), Math.trunc(-3.7), Math.sign(-2), Math.imul(0xffffffff, 5), Number.isNaN(0 / 0), Number.isNaN(1), Number.isFinite(1 / 0), Number.isFinite(1), isNaN(0 / 0) && isFinite(2)];
}
