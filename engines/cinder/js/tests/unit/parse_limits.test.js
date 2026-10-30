// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: the nesting and source size limits, with small limits (the
// 1000-level limit itself is checked by `progen depth` and `bundle
// astdiff`). The depth error is reported at the token that would go one
// level too deep; the size error at 1:1 before anything is read.

import { createParserWithLimits, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

function check(text, maxBytes, maxDepth) {
  const p = createParserWithLimits(unitsOf(text), false, maxBytes, maxDepth);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(formatError(parseError(p)));
  } else {
    console.log("OK");
  }
}

check("((((x))))", 1000, 6);
check("(((((x)))))", 1000, 6);
check("{{{{{{}}}}}}", 1000, 6);
check("{{{{{{{}}}}}}}", 1000, 6);
check("[[[[[]]]]]", 1000, 6);
check("[[[[[[]]]]]]", 1000, 6);
check("function f() { function g() { function h() { x } } }", 1000, 6);
check("x = `${`${`${`${1}`}`}`}`", 1000, 6);
check("x = 1;", 6, 1000);
check("x = 12;", 6, 1000);
check("x = '\u00e9';", 8, 1000);
check("x = '\u00e9';", 9, 1000);
