// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// A function declared inside another one.
// needs: closures
// expect: 6
export function main() {
  function twice(x) {
    return x * 2;
  }
  return [twice(3)];
}
