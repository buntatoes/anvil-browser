// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// for await is left out.
// expect-lint: await
async function f(a) {
  for await (const x of a) {}
}
