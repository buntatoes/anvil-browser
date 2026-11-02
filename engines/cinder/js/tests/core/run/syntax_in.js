// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// The `in` operator.
// needs: syntax
// expect: true
// expect: false
export function main() {
  const o = { a: 1 };
  return ["a" in o, "b" in o];
}
