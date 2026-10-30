// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// A plain file can not use closures.
// expect-lint: needs level closures
// needs: plain
export function f() {
  return () => 1;
}
