// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// ++ and compound assignment on properties.
// needs: syntax
// expect: 2
// expect: 10
export function main() {
  const o = { n: 1, m: 5 };
  o.n++;
  o.m *= 2;
  return [o.n, o.m];
}
