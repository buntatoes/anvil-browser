// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Each loop iteration gets its own `let` binding.
// needs: closures
// expect: 0
// expect: 2
export function main() {
  const fs = [];
  for (let i = 0; i < 3; i++) {
    fs.push(() => i);
  }
  return [fs[0](), fs[2]()];
}
