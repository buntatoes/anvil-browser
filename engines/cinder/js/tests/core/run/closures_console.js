// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// console.log comes with closures, before the rest
// of the host.
// needs: closures
// expect: hello 1
export function main() {
  console.log(1 + 1 === 2 ? "hello" : "", 1);
  return [];
}
