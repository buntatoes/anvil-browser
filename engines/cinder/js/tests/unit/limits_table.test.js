// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Every limit by name. The expected output is what `refdump --limits`
// prints for the reference; scripts/check.sh checks the two still agree.

import { LIMITS, limitValue, MAX_CALL_DEPTH } from "../../src/core/limits.js";

for (let i = 0; i < LIMITS.length; i++) {
  console.log(`${LIMITS[i][0]} = ${LIMITS[i][1]}`);
}
if (limitValue("limits.max_call_depth") !== MAX_CALL_DEPTH || limitValue("nope") !== -1) {
  console.log("FAIL limitValue");
}
