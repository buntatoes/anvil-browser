// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// length, charCodeAt, slice, indexOf and String.fromCharCode.
// needs: strings
// expect: 5
// expect: 233
// expect: ll
// expect: 3
// expect: -1
// expect: AB
export function main() {
  const s = "h\u00e9llo";
  return [
    s.length,
    s.charCodeAt(1),
    s.slice(2, 4),
    s.indexOf("l", 3),
    s.indexOf("z"),
    String.fromCharCode(65, 66),
  ];
}
