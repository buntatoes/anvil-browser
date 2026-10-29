// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// === and !== on strings.
// needs: strings
// expect: true
// expect: false
export function main() {
  const s = "ab";
  return [s === "ab", s !== "ab"];
}
