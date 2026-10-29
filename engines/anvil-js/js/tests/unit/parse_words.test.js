// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Contextual words, sloppy-mode `let`, reserved words and escapes.
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
  ["var async, of, get, set, as, from, target, meta;", "s"],
  ["async = 1", "s"],
  ["async()", "s"],
  ["async\n()", "s"],
  ["let = 1", "s"],
  ["let;", "s"],
  ["let\n[a] = b", "s"],
  ["var let", "s"],
  ["let let = 1", "s"],
  ["const let = 1", "s"],
  ["let [let] = 1", "s"],
  ["yield = 1", "s"],
  ["var await", "s"],
  ["await: 1", "s"],
  ["yield: 1", "s"],
  ["function* g() { yield: 1 }", "s"],
  ["async function f() { await: 1 }", "s"],
  ["x = { get, set, async, static }", "s"],
  ["x = { get a() {}, set a(v) {}, async a() {}, async *a() {}, get() {}, set() {}, async() {}, static() {} }", "s"],
  ["x = { if: 1, class: 2, new: 3 }", "s"],
  ["x.if.class.new", "s"],
  ["var enum", "s"],
  ["enum", "s"],
  ["\\u0061sync function f() {}", "s"],
  ["l\\u0065t x = 1", "s"],
  ["var \\u0079ield", "s"],
  ["\\u{76}ar x", "s"],
  ["var x\\u0079 = 1", "s"],
  ["x = { \\u0067et a() {} }", "s"],
  ["n\\u0065w X", "s"],
  ["x = { if }", "s"],
  ["x = { async\na() {} }", "s"],
  ["x = { get\na() {} }", "s"],
  ["let\nlet", "s"],
  ["let.x = 1", "s"],
  ["let()", "s"],
  ["var of; of = of", "s"],
  ["x = async function* () {}", "s"],
  ["async function\nf() {}", "s"],
  ["({ __proto__: 1, __proto__: 2 })", "s"],
  ["({ __proto__: 1, ['__proto__']: 2 })", "s"],
  ["({ __proto__: 1, __proto__ })", "s"],
  ["({ __proto__: 1, __proto__: 2 } = x)", "s"],
  ["x = { a, b, }", "s"],
  ["x = { 'a' }", "s"],
  ["x = { 1 }", "s"],
  ["x = { a: 1, ...b, c }", "s"],
]);
