// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Token dump driver for the token differential (`bundle lexdiff`). The
// tool writes ./lex_input.js on the fly with the sources to lex; this
// prints `FILE k` and then the dump of source k, in the format of
// `refdump --tokens`, with byte offsets.
// needs: host

import { SOURCES, MODULE_GOAL } from "./lex_input.js";
import { createLexer } from "../../src/syntax/lexer.js";
import { dumpTokens, byteOffsets, unitsOf } from "../../src/syntax/token_dump.js";

let k = 0;
while (k < SOURCES.length) {
  console.log(`FILE ${k}`);
  const units = unitsOf(SOURCES[k]);
  const lx = createLexer(units, MODULE_GOAL[k]);
  dumpTokens(lx, byteOffsets(units), (line) => console.log(line));
  k += 1;
}
