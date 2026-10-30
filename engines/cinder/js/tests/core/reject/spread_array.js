// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Spread in array literals is left out.
// expect-lint: spread
const a = [1];
export const b = [...a];
