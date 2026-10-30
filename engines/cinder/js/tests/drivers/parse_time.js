// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Timing driver for `bundle astdiff --time`: parses the sources of
// ./parse_input.js (written by the tool). With PARSE false it only turns
// the sources into code units, so the tool can subtract that from the
// step count.
// needs: host

import { SOURCES, MODULE_GOAL, PARSE } from "./parse_input.js";
import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

let k = 0;
while (k < SOURCES.length) {
  const units = unitsOf(SOURCES[k]);
  if (PARSE) {
    const p = createParser(units, MODULE_GOAL[k]);
    const prog = parseProgram(p);
    if (prog === null) {
      console.log(`ERROR ${formatError(parseError(p))}`);
    } else {
      console.log(`${units.length} units, ${prog.body.length} top-level statements`);
    }
  } else {
    console.log(`${units.length} units`);
  }
  k += 1;
}
