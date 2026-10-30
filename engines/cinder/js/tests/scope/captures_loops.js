// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
function f() {
  var fs = [];
  for (let i = 0; i < 3; i++) { fs.push(() => i); }
  for (const k in { a: 1 }) { fs.push(() => k); }
  for (let j of [1]) { j; }
  let n = 0;
  return () => n++;
}
