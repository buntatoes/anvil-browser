// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Classes: fields, static blocks, private names, accessors, computed keys.
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
  ["class A {}", "s"],
  ["class A extends B { constructor() { super(); } }", "s"],
  ["class A { constructor() { super(); } }", "s"],
  ["class A { static x = 1; #y; static #z = 2; [k] = 3; 'q' = 4; 5 = 6 }", "s"],
  ["class A { static { var a; } }", "s"],
  ["class A { static { await } }", "s"],
  ["class A { #a; m() { return #a in this; } }", "s"],
  ["class A { get #a() {} set #a(v) {} }", "s"],
  ["class A { constructor() {} constructor() {} }", "s"],
  ["class A { get constructor() {} }", "s"],
  ["class A { static prototype() {} }", "s"],
  ["class A { static prototype = 1 }", "s"],
  ["class A { constructor = 1 }", "s"],
  ["class A { #constructor }", "s"],
  ["class A { async *gen() { yield await 1; } }", "s"],
  ["class A { static async *[x]() {} }", "s"],
  ["class A { 'constructor'() {} }", "s"],
  ["class A { static constructor() {} }", "s"],
  ["class A { a = arguments }", "s"],
  ["class A { m() { super.x; } }", "s"],
  ["class A { m() { super(); } }", "s"],
  ["class A extends B { x = super.y; }", "s"],
  ["(class extends (a, b) {})", "s"],
  ["class A { set a() {} }", "s"],
  ["class A { get a(b) {} }", "s"],
  ["class extends B {}", "s"],
  ["class A { #a; static m() { delete this.#a } }", "s"],
  ["class A { x = () => this.#y; #y }", "s"],
  ["class let {}", "s"],
  ["class A { static { return; } }", "s"],
  ["class A { static { arguments; } }", "s"],
  ["class A { static async *#m() {} }", "s"],
  ["class A { 'use strict'; }", "s"],
  ["class A { a; b }", "s"],
  ["class A { static = 1; get = 2; set; async }", "s"],
  ["class A extends B { constructor() { (() => super())(); } }", "s"],
  ["class A { m() { with (a) {} } }", "s"],
  ["class A { [super.x] = 1 }", "s"],
  ["class A { #a = 1; m(o) { return o?.#a; } }", "s"],
  ["class A { static { var await; } }", "s"],
  ["class A { x = await }", "s"],
  ["class A { static { class B { [await] = 1 } } }", "s"],
]);
