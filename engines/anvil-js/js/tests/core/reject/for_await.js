// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// for await is left out.
// expect-lint: await
async function f(a) {
  for await (const x of a) {}
}
