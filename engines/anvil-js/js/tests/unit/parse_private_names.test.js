// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Private names: duplicates, getter and setter pairs, undeclared names, nested classes, `#x in`.
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
  ["class A { #a; #a; }", "s"],
  ["class A { get #a() {} static set #a(v) {} }", "s"],
  ["class A { get #a() {} get #a() {} }", "s"],
  ["class A { m() { #b in this; } }", "s"],
  ["class A { m() { class B { #x; } this.#x; } }", "s"],
  ["this.#a;", "s"],
  ["class A { static #a; #a; }", "s"],
  ["class A { #a() {} #a; }", "s"],
  ["class A { get #a() {} set #a(v) {} }", "s"],
  ["class A { #a; m() { #a in this; } }", "s"],
  ["class A { #x; m() { class B { y = this.#x; } } }", "s"],
  ["class A { m() { class B { y = this.#x; } } #x; }", "s"],
  ["class A { [this.#a] = 1; #a; }", "s"],
  ["class A { #constructor() {} }", "s"],
  ["class C { #x; m() { delete this.#x; } }", "s"],
  ["class C { #x; m() { delete (this.#x); } }", "s"],
  ["class C { #x; m() { delete this?.#x; } }", "s"],
  ["class A { static async *#m() {} static #m; }", "s"],
  ["class A { get #a() {} set #a(v) {} #a; }", "s"],
  ["class A { static set #a(v) {} static get #a() {} }", "s"],
  ["class A { m() { #x in #x in this; } #x; }", "s"],
  ["class A { #x; m() { (#x) in this; } }", "s"],
  ["class A { #x; m() { #x in this in that; } }", "s"],
]);
