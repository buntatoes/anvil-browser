// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
function f() {
  return () => () => this;
}
function g() {
  return () => { return function () { return this; }; };
}
