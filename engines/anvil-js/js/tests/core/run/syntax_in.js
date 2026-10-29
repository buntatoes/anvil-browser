// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The `in` operator.
// needs: syntax
// expect: true
// expect: false
export function main() {
  const o = { a: 1 };
  return ["a" in o, "b" in o];
}
