// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
const v = "outer";
function f(x = v) {
  const v = "inner";
  return x;
}
