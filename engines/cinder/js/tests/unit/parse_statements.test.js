// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Loops, labels, switch, if, with and try.
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
  ["for (var i = 0; i < 1; i++);", "s"],
  ["for (let i in o);", "s"],
  ["for (const x of y);", "s"],
  ["for (x of y);", "s"],
  ["for (let of x);", "s"],
  ["for (let in x);", "s"],
  ["for (async of x);", "s"],
  ["for (async of => 1;;);", "s"],
  ["for (var x = 1 in y);", "s"],
  ["'use strict'; for (var x = 1 in y);", "s"],
  ["for (var x = 1 of y);", "s"],
  ["for (let x = 1 of y);", "s"],
  ["for (x in y, z);", "s"],
  ["for (x of y, z);", "s"],
  ["for ((a in b);;);", "s"],
  ["for (a in b in c);", "s"],
  ["for (let [a] = b;;);", "s"],
  ["L: for(;;) { continue L; }", "s"],
  ["L: { continue L; }", "s"],
  ["L: L: ;", "s"],
  ["L: { break L; }", "s"],
  ["break L;", "s"],
  ["while (1) { function f() { break; } }", "s"],
  ["a: b: while(1) continue a;", "s"],
  ["switch (a) { case 1: default: default: }", "s"],
  ["switch (a) { case 1: let x; case 2: let y; }", "s"],
  ["if (1) function f() {}", "s"],
  ["'use strict'; if (1) function f() {}", "s"],
  ["while (1) function f() {}", "s"],
  ["L: function f() {}", "s"],
  ["'use strict'; L: function f() {}", "s"],
  // The reference accepts this; the engine follows the spec (tests/DIFFERENCES.md, labelled function as a statement body).
  ["if (1) L: function f() {}", "s"],
  ["with (a) b;", "s"],
  ["'use strict'; with (a) b;", "s"],
  ["for await (x of y);", "s"],
  ["for await (x of y);", "m"],
  ["try {}", "s"],
  ["try {} catch {} finally {}", "s"],
  ["try {} catch (e) {}", "s"],
  ["if (1) let x;", "s"],
  ["if (1) let\nx;", "s"],
  ["while (1) const x = 1;", "s"],
  ["label: let\nx", "s"],
  ["do function f() {} while (0)", "s"],
  ["if (1) class A {}", "s"],
  ["for (let let of x);", "s"],
  ["for (let.x of y);", "s"],
  ["for (let.x in y);", "s"],
  ["for (let\nof x);", "s"],
  ["debugger", "s"],
  ["debugger 1", "s"],
  ["switch (a) {}", "s"],
  ["continue", "s"],
  ["for (;;) { continue; }", "s"],
]);
