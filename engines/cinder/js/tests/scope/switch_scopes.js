// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
function f(x) {
  switch (x) {
    case 1: let a = 1; return () => a;
    default: { const b = 2; }
  }
}
