// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// A __proto__ key in an object literal is left out.
// expect-lint: __proto__
export const o = { __proto__: null };
