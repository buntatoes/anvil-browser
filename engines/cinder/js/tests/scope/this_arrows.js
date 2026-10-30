// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
function f() {
  return () => () => this;
}
function g() {
  return () => { return function () { return this; }; };
}
