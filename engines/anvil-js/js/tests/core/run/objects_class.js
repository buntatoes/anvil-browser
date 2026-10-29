// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// A class with a constructor, a method, a static method, and `this`.
// needs: objects
// expect: 3
// expect: 7
class Point {
  constructor(x, y) {
    this.x = x;
    this.y = y;
  }
  sum() {
    return this.x + this.y;
  }
  static origin() {
    return new Point(0, 0);
  }
}
export function main() {
  const p = new Point(1, 2);
  const o = Point.origin();
  o.x = 7;
  return [p.sum(), o.sum()];
}
