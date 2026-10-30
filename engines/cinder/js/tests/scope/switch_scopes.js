// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
function f(x) {
  switch (x) {
    case 1: let a = 1; return () => a;
    default: { const b = 2; }
  }
}
