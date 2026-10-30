// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// if, while, do-while and for, with let and const.
// needs: plain
// expect: 10
// expect: 5
// expect: 7
export function main() {
  let sum = 0;
  for (let i = 0; i < 5; i++) {
    sum += i;
  }
  let n = 0;
  while (n < 5) {
    n++;
  }
  let k = 10;
  do {
    k -= 3;
  } while (k > 7);
  return [sum, n, k];
}
