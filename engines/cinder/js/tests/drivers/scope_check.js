// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Scope self-check driver for `bundle scopecheck`: the tool writes
// ./scope_input.js with the sources; this prints `FILE k` and then
// `OK places bindings references functions scopes` when the analysis of
// source k passes src/syntax/scope_check.js, `PROBLEM text` lines when it
// does not, or `SKIP line:col: message` when the source does not parse.
// needs: host

import { SOURCES, MODULE_GOAL } from "./scope_input.js";
import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { analyzeProgram } from "../../src/syntax/scope.js";
import { checkScopes, identifierPlaces } from "../../src/syntax/scope_check.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

let k = 0;
while (k < SOURCES.length) {
  console.log(`FILE ${k}`);
  const p = createParser(unitsOf(SOURCES[k]), MODULE_GOAL[k]);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(`SKIP ${formatError(parseError(p))}`);
  } else {
    const a = analyzeProgram(prog);
    const problems = checkScopes(prog, a);
    if (problems.length === 0) {
      console.log(
        `OK ${identifierPlaces(prog)} ${a.bindings.length} ${a.refs.length} ${a.funcs.length} ${a.scopes.length}`,
      );
    } else {
      for (const t of problems) {
        console.log(`PROBLEM ${t}`);
      }
    }
  }
  k += 1;
}
