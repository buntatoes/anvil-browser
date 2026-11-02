// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// An arrow function that keeps a local variable alive.
// needs: closures
// expect: 1
// expect: 2
function counter() {
  let n = 0;
  return () => {
    n++;
    return n;
  };
}
export function main() {
  const c = counter();
  const a = c();
  return [a, c()];
}
