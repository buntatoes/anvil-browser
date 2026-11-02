// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Spread in array literals is left out.
// expect-lint: spread
const a = [1];
export const b = [...a];
