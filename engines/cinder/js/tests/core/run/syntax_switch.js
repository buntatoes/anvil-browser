// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// switch with fall-through and default.
// needs: syntax
// expect: 23
function pick(n) {
  let r = 0;
  switch (n) {
    case 1:
      r = 1;
      break;
    case 2:
      r = 2;
    case 3:
      r = r * 10 + 3;
      break;
    default:
      r = -1;
  }
  return r;
}
export function main() {
  return [pick(2)];
}
