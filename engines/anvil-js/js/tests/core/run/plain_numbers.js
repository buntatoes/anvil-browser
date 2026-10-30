// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Numbers are doubles: %, -0, int32 and uint32 operators. Plain code
// does not use the names NaN and Infinity (they come with the syntax
// level).
// needs: plain
// expect: 0.30000000000000004
// expect: -2
// expect: 2
// expect: true
// expect: true
// expect: -2147483648
// expect: 4294967295
// expect: -6
// expect: 1
// expect: 0
// expect: 9007199254740992
export function main() {
  return [
    0.1 + 0.2,
    -5 % 3,
    5 % -3,
    1 / (-0 % 5) < 0,
    -0 === 0,
    1 << 31,
    -1 >>> 0,
    ~5,
    3 & 5,
    4294967296 | 0,
    9007199254740993,
  ];
}
