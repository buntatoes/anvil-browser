// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
function f(a, b = () => a) {
  var a;
  return b;
}
function g(a, b = 1) {
  var c = a;
  return () => c;
}
