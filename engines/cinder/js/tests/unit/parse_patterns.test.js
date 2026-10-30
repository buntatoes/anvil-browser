// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Destructuring in declarations, assignments, parameters and loops.
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
  ["var [a, , b = 1, ...c] = d", "s"],
  ["var {a, b: c, [d]: e = 1, ...f} = g", "s"],
  ["[a, b.c, ...d[0]] = e", "s"],
  ["({a, b: c.d, ...e} = f)", "s"],
  ["({a = 1} = b)", "s"],
  ["[...a, b] = c", "s"],
  ["[...a,] = b", "s"],
  ["({...a.b} = c)", "s"],
  ["var {...[a]} = b", "s"],
  ["({...{}} = a)", "s"],
  ["let [a] = [], {b} = {}", "s"],
  ["for ([a, b] of c);", "s"],
  ["for ({a} in b);", "s"],
  ["try {} catch ({a, b}) {}", "s"],
  ["function f([a] = [], {b} = {}) {}", "s"],
  ["var [a]", "s"],
  ["const a", "s"],
  ["[a = 1] = b", "s"],
  ["[(a)] = b", "s"],
  ["[(a = 1)] = b", "s"],
  ["({a: (b)} = c)", "s"],
  ["({a: (b = 1)} = c)", "s"],
  ["[a, ...b = 1] = c", "s"],
  ["[a.b?.c] = d", "s"],
  ["({a: b()} = c)", "s"],
  ["[...[a, b]] = c", "s"],
  ["({a: [b]} = c)", "s"],
  ["({eval} = a)", "s"],
  ["'use strict'; ({arguments} = a)", "s"],
  ["[x, y] = [y, x]", "s"],
  ["({1: a, 'b': c, [d]: e} = f)", "s"],
  ["let {a: {b}} = c", "s"],
  ["var {a, a} = b", "s"],
  ["[{a = 1}] = b", "s"],
]);
