// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Arrow functions through the cover grammar.
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
  ["() => {}", "s"],
  ["(a, b) => a", "s"],
  ["a => b => c", "s"],
  ["(a = 1, {b}, [c], ...d) => 0", "s"],
  ["async a => a", "s"],
  ["async (a) => await a", "s"],
  ["async(a)", "s"],
  ["async\n(a) => a", "s"],
  ["(a, b)", "s"],
  ["(a, ...b)", "s"],
  ["() => {} ()", "s"],
  ["(a)\n=> a", "s"],
  ["({a: 1}) => a", "s"],
  ["([a.b]) => 1", "s"],
  ["(a, a) => 1", "s"],
  ["(...a,) => 1", "s"],
  ["async (...a, b) => 1", "s"],
  ["function* g() { (x = yield) => 1; }", "s"],
  ["async (x = await 1) => 1", "s"],
  ["(a = 1) => { 'use strict'; }", "s"],
  ["(eval) => { 'use strict' }", "s"],
  ["()", "s"],
  ["(a,) => 1", "s"],
  ["(a,)", "s"],
  ["async () => { var await; }", "s"],
  ["async await => 1", "s"],
  ["(a) => {}\n(b)", "s"],
  ["x = a => a, b", "s"],
  ["a ? b => c : d => e", "s"],
  ["(a.b) => 1", "s"],
  ["((a)) => 1", "s"],
  ["({a = 1}) => a", "s"],
  ["({a = 1})", "s"],
  ["async ({a = 1})", "s"],
  ["(yield) => 1", "s"],
  ["'use strict'; (yield) => 1", "s"],
]);
