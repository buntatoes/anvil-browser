// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The comma operator.
// needs: syntax
// expect: 3
export function main() {
  let a = 0;
  const b = (a++, a++, a + 1);
  return [b];
}
