// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Functions, generators and async functions.
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
  ["function* g() { yield; yield* a; yield\na }", "s"],
  ["function* g() { var yield; }", "s"],
  ["function* g(a = yield) {}", "s"],
  ["async function f() { await a; for await (x of y); }", "s"],
  ["async function f() { var await; }", "s"],
  ["function f() { for await (x of y); }", "s"],
  ["async function* f() { yield await a; }", "s"],
  ["function f() { new.target; }", "s"],
  ["new.target", "s"],
  ["() => new.target", "s"],
  ["function f() { () => new.target; }", "s"],
  ["function f(a, a) {}", "s"],
  ["'use strict'; function f(a, a) {}", "s"],
  ["function f() { 'use strict'; with (a) {} }", "s"],
  ["function f(a = 1) { 'use strict' }", "s"],
  ["function eval() { 'use strict' }", "s"],
  ["return", "s"],
  ["function* f() { function yield() {} }", "s"],
  ["async function f() { function await() {} }", "s"],
  ["function f() {} function f() {}", "s"],
  ["function* g() { yield\n* a }", "s"],
  ["function* g() { yield * a }", "s"],
  ["function* g() { x = yield in y }", "s"],
  ["function* g() { yield ? 1 : 2 }", "s"],
  ["async function f() { await\n1 }", "s"],
  ["function f(...a) {}", "s"],
  ["function f(...a = 1) {}", "s"],
  ["function f(a,) {}", "s"],
  ["function f(,) {}", "s"],
  ["function f() { return /a/ }", "s"],
  ["async function* f() { for await (const x of y) {} }", "s"],
  ["(function* yield() {})", "s"],
  ["(async function await() {})", "s"],
  ["function* g() { yield => 1 }", "s"],
  ["async function f() { async () => await 1 }", "s"],
  ["function f() { await: 1 }", "s"],
]);
