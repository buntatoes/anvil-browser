// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
function outer() {
  L: function inner() { return 1; }
  return inner;
}
M: function top() {}
top();
