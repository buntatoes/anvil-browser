// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Code inside a generator is reported as outside the subset, not as a
// syntax error.
// expect-lint: `yield` is outside Core JS
export function* g() {
  yield 1;
}
