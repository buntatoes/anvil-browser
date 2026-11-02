// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// A plain file can not use closures.
// expect-lint: needs level closures
// needs: plain
export function f() {
  return () => 1;
}
