// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Template literals.
// needs: strings
// expect: a1b2
export function main() {
  const x = 1;
  return [`a${x}b${x + 1}`];
}
