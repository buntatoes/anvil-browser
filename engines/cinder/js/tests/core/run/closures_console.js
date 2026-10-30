// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// console.log comes with closures, before the rest
// of the host.
// needs: closures
// expect: hello 1
export function main() {
  console.log(1 + 1 === 2 ? "hello" : "", 1);
  return [];
}
