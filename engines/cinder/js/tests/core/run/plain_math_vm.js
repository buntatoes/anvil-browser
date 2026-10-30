// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The math helpers the VM already has, without the extra built-ins.
// needs: plain
// expect: -2
// expect: -1
// expect: 3
// expect: -3
// expect: 0
// expect: 4
// expect: 2.5
// expect: 1.4142135623730951
export function main() {
  return [Math.floor(-1.5), Math.ceil(-1.5), Math.round(2.5), Math.round(-3.5), Math.round(0.49999999999999994), Math.abs(-4), Math.abs(2.5), Math.sqrt(2)];
}
