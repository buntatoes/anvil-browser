// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Math.min and the like are extra built-ins, at any level.
// needs: host
// expect-lint: `Math.min` is one of the extra built-ins
// expect-lint: `Number.isInteger` is one of the extra built-ins
// expect-lint: `isNaN` is one of the extra built-ins
export const a = Math.min(1, 2);
export const b = Number.isInteger(3);
export const c = isNaN(4);
export const d = Math.floor(1.5);
