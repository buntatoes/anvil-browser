// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// finally runs on the way out of a return.
// needs: errors
// expect: 1
// expect: 2
let log = 0;
function f() {
  try {
    return 1;
  } finally {
    log = 2;
  }
}
export function main() {
  const r = f();
  return [r, log];
}
