// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
{
  function a() {}
}
function f() {
  { function b() { return b; } }
  let c;
  { function c() {} }
  return a;
}
