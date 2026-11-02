// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Destructuring parameters are left out.
// expect-lint: destructuring
export function f([a]) {
  return a;
}
