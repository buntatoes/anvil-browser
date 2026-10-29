// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// this is only allowed in class methods and constructors.
// expect-lint: `this`
export function f() {
  return this;
}
