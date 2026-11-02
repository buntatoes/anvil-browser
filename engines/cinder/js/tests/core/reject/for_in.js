// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// for-in is left out.
// expect-lint: `for...in`
const o = {};
for (const k in o) {}
