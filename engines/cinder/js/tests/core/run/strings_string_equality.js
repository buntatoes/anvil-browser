// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// === and !== on strings.
// needs: strings
// expect: true
// expect: false
export function main() {
  const s = "ab";
  return [s === "ab", s !== "ab"];
}
