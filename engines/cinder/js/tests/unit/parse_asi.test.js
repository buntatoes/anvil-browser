// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Automatic semicolon insertion and the restricted productions.
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
  ["a\nb", "s"],
  ["a = 1\n(b)", "s"],
  ["x\n++y", "s"],
  ["x\n++\ny", "s"],
  ["do x; while (y) z", "s"],
  ["if (a) b\nelse c", "s"],
  ["throw\n1", "s"],
  ["let\nx = 1", "s"],
  ["var a = 1 var b", "s"],
  ["a = b\n/c/g.exec(d)", "s"],
  ["x = y\n`t`", "s"],
  ["for (;;\n) {}", "s"],
  ["{ 1 2 } 3", "s"],
  ["{ 1\n2 } 3", "s"],
  ["a\n?.b", "s"],
  ["L: while (1) { continue\nL; }", "s"],
  ["function f() { return\n1 }", "s"],
  ["function* g() { yield\na }", "s"],
  ["async\nfunction f() {}", "s"],
  ["class A { x\ny }", "s"],
  ["class A { x = 1 y }", "s"],
  ["class A { get\nx() {} }", "s"],
  ["class A { static\nm() {} }", "s"],
  ["var f = function () {}\n(1)", "s"],
  ["a\n=> b", "s"],
  ["x\n--\ny", "s"],
  ["do ; while (0) 1", "s"],
  ["if (a) ; else\n;", "s"],
  ["for (a\nin b) ;", "s"],
  ["a ? b\n: c", "s"],
]);
