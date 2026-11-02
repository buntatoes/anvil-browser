// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// + and += on strings.
// needs: strings
// expect: ab1
// expect: abc
export function main() {
  let s = "a";
  s += "b";
  const t = s + "c";
  return [s + 1, t];
}
