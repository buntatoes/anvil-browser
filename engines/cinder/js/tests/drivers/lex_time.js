// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Timing driver for `bundle lexdiff --time`: lexes the sources of
// ./lex_input.js (written by the tool) with the rescans replayed, and
// prints the token count and how the tokens are stored. With LEX false
// it only turns the sources into code units, so the tool can subtract
// that from the step count.
// needs: host

import { SOURCES, MODULE_GOAL, LEX } from "./lex_input.js";
import { createLexer } from "../../src/syntax/lexer.js";
import { lexWithReplay, unitsOf } from "../../src/syntax/token_dump.js";
import { lexErrorMessage } from "../../src/syntax/lex_messages.js";

let k = 0;
while (k < SOURCES.length) {
  const units = unitsOf(SOURCES[k]);
  if (LEX) {
    const lx = createLexer(units, MODULE_GOAL[k]);
    const n = lexWithReplay(lx, null);
    if (n < 0) {
      console.log(`ERROR ${lexErrorMessage(lx)}`);
    } else {
      const t = lx.tokens;
      console.log(
        `${units.length} units, ${n} tokens, ${t.values.length} side values, ${t.numbers.length} numbers, ${lx.starts.length} lines`,
      );
    }
  } else {
    console.log(`${units.length} units`);
  }
  k += 1;
}
