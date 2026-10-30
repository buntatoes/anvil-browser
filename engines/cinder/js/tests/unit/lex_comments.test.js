// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Comments and the line-break flag: single-line, multi-line (a line
// break inside counts), hashbang, and HTML-like comments, which are
// comments in scripts only.

import { createLexer } from "../../src/syntax/lexer.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text, isModule) {
  console.log(`-- ${text.length} units, module ${isModule}`);
  dumpTokens(createLexer(unitsOf(text), isModule), null, (l) => console.log(l));
}

show("a // one\nb /* two */ c /* thr\nee */ d /**/e/* ** / * */f", false);
show("#!hash bang\nx", false);
show("#!hash bang\nx", true);
show(" #!x", false);
show("a <!-- b\nc\n--> d\n  /* x */ --> e\n/*\n*/ --> f\ng --> h", false);
show("a <!-- b\nc\n--> d", true);
show("--> first\nx", false);
show("  --> first\nx", false);
show("/* c */ --> first\nx", false);
show("a /* \u2028 */ b", false);
show("a /* x", false);
show("/* a \n b", false);
