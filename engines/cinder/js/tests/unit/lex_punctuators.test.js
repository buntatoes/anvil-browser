// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Every punctuator alone, longest-match runs, and \`?.\` before a digit.

import { createLexer, nextToken } from "../../src/syntax/lexer.js";
import { PUNCT_TEXT, PUNCT_COUNT, TK_PUNCT, TK_EOF } from "../../src/syntax/tokens.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text.length} units`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

let ok = 0;
let p = 0;
while (p < PUNCT_COUNT) {
  const lx = createLexer(PUNCT_TEXT[p], false);
  const i = nextToken(lx);
  const j = nextToken(lx);
  if (lx.tokens.kind[i] === TK_PUNCT && lx.tokens.aux[i] === p && lx.tokens.kind[j] === TK_EOF) {
    ok += 1;
  } else {
    console.log(`FAIL punctuator ${p}`);
  }
  p += 1;
}
console.log(`${ok} of ${PUNCT_COUNT} punctuators lex alone`);
show("a?.b a?.[0] a?.5:1 a ?. b ...x ... . .. x>>>=1 >>= >>> >> <<= ** **= ??= ?? ||= &&= =>");
show("a+++b a---b a====b a!===b <=>");
show("@");
show("\u0000");
show("x = 1 \u2190 2");
show("\ud83d\ude00");
