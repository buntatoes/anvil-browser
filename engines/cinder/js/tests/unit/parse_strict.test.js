// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Directive prologues, retroactive strict checks and strict mode rules.
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
  ["'use strict'; 010", "s"],
  ["'use strict'; '\\07'", "s"],
  ["'\\07'; 'use strict'", "s"],
  ["function f() { '\\07'; 'use strict'; }", "s"],
  ["'use strict'; var let", "s"],
  ["'use strict'; var static", "s"],
  ["'use strict'; implements", "s"],
  ["'use strict'; eval = 1", "s"],
  ["'use strict'; arguments++", "s"],
  ["'use strict'; delete x", "s"],
  ["'use strict'; ({eval} = x)", "s"],
  ["'use strict'; function f(eval) {}", "s"],
  ["function f(eval) { 'use strict' }", "s"],
  ["'use\\x20strict'; 010", "s"],
  ["('use strict'); 010", "s"],
  ["'a'; 'use strict'; with(a);", "s"],
  ["'use strict'\n;010", "s"],
  ["function f() { 'use strict' } 010", "s"],
  ["'use strict'; 08", "s"],
  ["'use strict'; '\\8'", "s"],
  ["'use strict'; yield = 1", "s"],
  ["'use strict'; var package", "s"],
  ["'use strict' + 1; 010", "s"],
  ["'use strict'\n+ 1; 010", "s"],
  ["function f() { 'a' 'use strict' }", "s"],
  ["function f(a, b = 1) { 'a'; 'use strict' }", "s"],
  ["function f() { 'use strict'; } function g() { with (a); }", "s"],
  ["'use strict'; function f() { 010 }", "s"],
  ["function f() { 'use strict'; return function () { 010 } }", "s"],
  ["'use strict'; delete (x)", "s"],
  ["'use strict'; delete ((x))", "s"],
  ["'use strict'; delete x.y", "s"],
  ["function arguments() { 'use strict' }", "s"],
  ["'use strict'; try {} catch (eval) {}", "s"],
  ["'use strict'; label: eval: ;", "s"],
]);
