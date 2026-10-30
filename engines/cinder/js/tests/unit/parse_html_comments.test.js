// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: HTML-like comments in scripts and modules (outside the start-of-input case).
// Each case prints `index OK` or `index line:col: message`; the
// .expected file is what the reference parser says for the same sources.

import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

// Each case is [source, goal]: "s" for a script, "m" for a module.
function checkAll(cases) {
  let i = 0;
  while (i < cases.length) {
    const c = cases[i];
    const p = createParser(unitsOf(c[0]), c[1] === "m");
    const prog = parseProgram(p);
    if (prog === null) {
      console.log(`${i} ${formatError(parseError(p))}`);
    } else {
      console.log(`${i} OK`);
    }
    i += 1;
  }
}

checkAll([
  ["<!-- a\nb", "s"],
  ["a <!-- b", "s"],
  ["a\n--> b", "s"],
  ["--> a", "s"],
  ["a --> b", "s"],
  ["x = 1 /*\n*/ --> c", "s"],
  ["<!-- a", "m"],
  ["a\n--> b", "m"],
  ["a /* x */ --> b", "s"],
  ["a\n/* x */ --> b", "s"],
  ["a\n  --> b\nc", "s"],
]);
