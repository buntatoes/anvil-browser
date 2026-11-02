// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
function outer() {
  L: function inner() { return 1; }
  return inner;
}
M: function top() {}
top();
