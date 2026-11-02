// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
{
  function* g() {}
}
function f() {
  switch (1) {
    case 1:
      function* h() {}
  }
}
