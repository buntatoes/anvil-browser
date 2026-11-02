// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// this is only allowed in class methods and constructors.
// expect-lint: `this`
export function f() {
  return this;
}
