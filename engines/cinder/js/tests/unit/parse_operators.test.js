// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Operator precedence, associativity and invalid operator use.
// Each case prints `index OK` or `index line:col: message`; the
// .expected file is what the reference parser says for the same sources.

import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { dumpProgram } from "../../src/syntax/ast_dump.js";
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

// The syntax tree of a script, without spans.
function dumpScript(text) {
  const p = createParser(unitsOf(text), false);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(`ERROR ${formatError(parseError(p))}`);
  } else {
    dumpProgram(prog, false, null, (l) => console.log(l));
  }
}

checkAll([
  ["-a ** b", "s"],
  ["(-a) ** b", "s"],
  ["a ?? b || c", "s"],
  ["(a ?? b) || c", "s"],
  ["a && b ?? c", "s"],
  ["typeof a ** 2", "s"],
  ["1 = 2", "s"],
  ["a++ = 1", "s"],
  ["++a++", "s"],
  ["(a) = 1", "s"],
  ["((a)) = 1", "s"],
  ["(a.b) = 1", "s"],
  ["a?.b = 1", "s"],
  ["a?.b++", "s"],
  ["a ||| b", "s"],
  ["a +", "s"],
  ["a ? b", "s"],
  ["a ? b :", "s"],
  ["a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a + a", "s"],
  ["a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = a = b", "s"],
  ["this = 1", "s"],
  ["new.target = 1", "s"],
  ["a ||= b &&= c ??= d", "s"],
  ["a?.b &&= 1", "s"],
  ["({a} += 1)", "s"],
  ["[a] *= 1", "s"],
  ["a ** b = 1", "s"],
  ["delete a", "s"],
  ["void typeof delete a.b", "s"],
  ["a < b > c <= d >= e", "s"],
  ["a +++ b", "s"],
  ["a ++ + ++ b", "s"],
  ["#x in y", "s"],
  ["x in #y", "s"],
  ["1 + #x in y", "s"],
]);
console.log("-- a + b * c ** d ** e - f");
dumpScript("a + b * c ** d ** e - f");
console.log("-- a ?? b ?? c");
dumpScript("a ?? b ?? c");
console.log("-- a || b && c | d ^ e & f == g < h << i + j * k");
dumpScript("a || b && c | d ^ e & f == g < h << i + j * k");
console.log("-- a = b += c ||= d");
dumpScript("a = b += c ||= d");
console.log("-- a ? b : c ? d : e");
dumpScript("a ? b : c ? d : e");
console.log("-- a, b = c, d");
dumpScript("a, b = c, d");
console.log("-- x in y instanceof z");
dumpScript("x in y instanceof z");
console.log("-- !a?.b.c()");
dumpScript("!a?.b.c()");
console.log("-- a ** -b");
dumpScript("a ** -b");
console.log("-- new a.b(c).d");
dumpScript("new a.b(c).d");
console.log("-- new new a()()");
dumpScript("new new a()()");
console.log("-- a?.b[c](d)?.e");
dumpScript("a?.b[c](d)?.e");
