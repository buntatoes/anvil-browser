// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// new.target is left out.
// expect-lint: new.target
export function F() {
  return new.target;
}
