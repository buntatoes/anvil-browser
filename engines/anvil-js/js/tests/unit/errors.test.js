// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Engine error objects and their text form.
// expect: SyntaxError
// expect: 2:2: unexpected token
// expect: 4 6
// expect: RangeError: 1:1: too big
// expect: 1:1: source is 70 bytes, over the 64 byte limit
// expect: 3:1: nesting is too deep (limit 1000)

import {
  makeError,
  syntaxError,
  rangeError,
  formatError,
  describeError,
  sourceTooLarge,
  nestingTooDeep,
} from "../../src/core/errors.js";
import { lineStarts } from "../../src/core/positions.js";

// "ab\ncdef\n\n"
const starts = lineStarts([0x61, 0x62, 0x0a, 0x63, 0x64, 0x65, 0x66, 0x0a, 0x0a]);
const e = syntaxError("unexpected token", starts, 4, 6);
console.log(e.name);
console.log(formatError(e));
console.log(`${e.offset} ${e.end}`);
console.log(describeError(rangeError("too big", starts, 0, 1)));
console.log(formatError(sourceTooLarge(70, 64)));
console.log(formatError(nestingTooDeep(1000, starts, 8, 8)));
const m = makeError("SyntaxError", "x", 1, 2, 3, 4);
if (m.line !== 1 || m.column !== 2 || m.offset !== 3 || m.end !== 4) {
  console.log("FAIL makeError fields");
}
