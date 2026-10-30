// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Syntax tree dump driver for the AST differential (`bundle astdiff`).
// The tool writes ./parse_input.js on the fly with the sources to parse;
// this prints `FILE k` and then the dump of source k, in the format of
// `refdump --ast` (with spans in bytes when SPANS is set), or
// `ERROR line:col: message` when the parse fails. With PARSE_ONLY only
// the verdict is printed (`OK` or the error line).
// needs: host

import { SOURCES, MODULE_GOAL, SPANS, PARSE_ONLY } from "./parse_input.js";
import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { dumpProgram } from "../../src/syntax/ast_dump.js";
import { byteOffsets, unitsOf } from "../../src/syntax/token_dump.js";

function print(line) {
  console.log(line);
}

let k = 0;
while (k < SOURCES.length) {
  console.log(`FILE ${k}`);
  const units = unitsOf(SOURCES[k]);
  const p = createParser(units, MODULE_GOAL[k]);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(`ERROR ${formatError(parseError(p))}`);
  } else if (PARSE_ONLY) {
    console.log("OK");
  } else {
    let bytes = null;
    if (SPANS) {
      bytes = byteOffsets(units);
    }
    dumpProgram(prog, SPANS, bytes, print);
  }
  k += 1;
}
