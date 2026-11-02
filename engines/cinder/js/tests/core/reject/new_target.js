// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// new.target is left out.
// expect-lint: new.target
export function F() {
  return new.target;
}
