// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Names: ASCII, Unicode (tables of src/core/unicode_tables.js), escapes,
// private names, keywords (identifiers to the lexer), the word tables,
// and the errors for bad escapes. Offsets in the dumps are code units.

import { createLexer, nextToken } from "../../src/syntax/lexer.js";
import {
  tokenIsWord,
  tokenName,
  isAlwaysReserved,
  isStrictReserved,
  isContextual,
  RESERVED_WORDS,
  STRICT_RESERVED_WORDS,
  CONTEXTUAL_WORDS,
} from "../../src/syntax/tokens.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text.length} units`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

show("a $b _c d1 $_$ Ab9");
show("\u00e9t\u00e9 \u03c0 \u0416 \u05d0 \u4e2d \ud835\udc65 a\u200cb x\u0660");
show("\\u0061 a\\u0062c \\u{62}\\u{63} \\u{1D465} \\u{0000000061} x\\u200C");
show("#x #\\u0061b #\u00e9");
show("if else return yield await let static of");
show("\\u0069f");
show("\\x61");
show("\\u0030a");
show("a\\u0020");
show("\\uD800");
show("\\u{D800}");
show("a\\u{110000}");
show("a\\u{}");
show("a\\u{61");
show("a\\u006");
show("#");
show("# a");
show("#1");
show("\u00b7");
show("\u0301x");

// Keywords only count written without escapes.
const lx = createLexer(unitsOf("if \\u0069f"), false);
const i = nextToken(lx);
const j = nextToken(lx);
const ifUnits = unitsOf("if");
console.log(`plain if: ${tokenIsWord(lx.tokens, lx.src, i, ifUnits)}, escaped if: ${tokenIsWord(lx.tokens, lx.src, j, ifUnits)}`);
console.log(`escaped name decodes to ${String.fromCharCode(tokenName(lx.tokens, lx.src, j)[0])}`);

function check(word) {
  const u = unitsOf(word);
  return `${word}: ${isAlwaysReserved(u)} ${isStrictReserved(u)} ${isContextual(u)}`;
}
console.log(check("class"));
console.log(check("await"));
console.log(check("yield"));
console.log(check("let"));
console.log(check("of"));
console.log(check("foo"));
console.log(`tables: ${RESERVED_WORDS.length} ${STRICT_RESERVED_WORDS.length} ${CONTEXTUAL_WORDS.length}`);
