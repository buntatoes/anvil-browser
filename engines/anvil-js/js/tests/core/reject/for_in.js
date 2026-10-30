// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// for-in is left out.
// expect-lint: `for...in`
const o = {};
for (const k in o) {}
