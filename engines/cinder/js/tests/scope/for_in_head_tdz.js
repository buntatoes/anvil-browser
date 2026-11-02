// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
var x = {};
for (let x in { a: () => x }) {}
for (const { z } in (() => z)) {}
