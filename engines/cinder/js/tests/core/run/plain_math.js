// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The math helpers that are extra built-ins.
// needs: plain
// builtins: yes
// expect: -2
// expect: 3
// expect: -1
// expect: true
// expect: false
export function main() {
  return [Math.floor(-1.5), Math.max(1, 3, 2), Math.min(0, -1), Number.isInteger(4), Number.isInteger(4.5)];
}
