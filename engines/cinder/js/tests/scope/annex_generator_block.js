// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
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
