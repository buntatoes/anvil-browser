// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
{
  async function a() {}
}
function f() {
  if (true) {
    async function b() {}
  }
}
