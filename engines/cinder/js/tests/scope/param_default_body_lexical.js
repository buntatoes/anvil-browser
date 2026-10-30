// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
const v = "outer";
function f(x = v) {
  const v = "inner";
  return x;
}
