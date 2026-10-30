// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The canonical scope dump (src/syntax/scope_dump.js, format in
// docs/SCOPE.md) of a few small programs; the expected text was checked
// by hand against the spec's declaration instantiation.

import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { analyzeProgram } from "../../src/syntax/scope.js";
import { dumpScopes } from "../../src/syntax/scope_dump.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

function print(line) {
  console.log(line);
}

function dump(src, isModule) {
  const prog = parseProgram(createParser(unitsOf(src), isModule));
  dumpScopes(analyzeProgram(prog), print);
}

dump("var n = 0;\nfunction f(a, b = a) {\n  let c = () => a + n++ + this;\n  { function g() {} }\n  return c;\n}\n", false);
dump("for (let i = 0; i < 2; i++) {\n  setTimeout(() => i);\n}\nwith (o) { x; }\n", false);
dump("import { v } from 'm';\nexport default class {\n  #p = v;\n  get() { return this.#p; }\n}\n", true);
