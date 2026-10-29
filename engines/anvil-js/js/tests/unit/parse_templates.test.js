// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Templates, tagged templates and regex or division after the parser's rescans.
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
  ["`a${b}c`", "s"],
  ["tag`\\unicode`", "s"],
  ["`\\unicode`", "s"],
  ["a = /=/g", "s"],
  ["x / y / z", "s"],
  ["a\n/b/g", "s"],
  ["if (a) /b/.test(c)", "s"],
  ["(a) / b", "s"],
  ["a++ / 2", "s"],
  ["`${`${a}`}`", "s"],
  ["`${{a:1}.a}`", "s"],
  ["`${a}`.length", "s"],
  ["a?.`b`", "s"],
  ["a?.b`c`", "s"],
  ["/(/", "s"],
  ["/a/gg", "s"],
  ["new `a`", "s"],
  ["x = `a\r\nb`", "s"],
  ["`${a", "s"],
  ["`a${b}", "s"],
  ["`${}`", "s"],
  ["tag`a${b}\\xg${c}`", "s"],
  ["`\\u{110000}`", "s"],
  ["x = {} / 1", "s"],
  ["{} / 1 /", "s"],
  ["a = b ? /c/ : /d/", "s"],
  ["return /a/", "s"],
  ["x = y => /a/", "s"],
  ["+/a/", "s"],
  ["a[/b/]", "s"],
  ["`${/a/}`", "s"],
  ["x = /[/]/", "s"],
  ["x = /a/\n.b", "s"],
  ["this / 2", "s"],
  ["} /a/", "s"],
  ["x = typeof /a/", "s"],
]);
