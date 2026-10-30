// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// extends, super(...) and super.method().
// needs: objects
// expect: 12
// expect: true
class Base {
  constructor(v) {
    this.v = v;
  }
  get() {
    return this.v;
  }
}
class Derived extends Base {
  constructor(v) {
    super(v + 1);
  }
  get() {
    return super.get() * 2;
  }
}
export function main() {
  const d = new Derived(5);
  return [d.get(), d instanceof Base];
}
