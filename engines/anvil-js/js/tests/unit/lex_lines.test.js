// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Line terminators (LF, CR, CRLF, LS, PS), whitespace of every kind,
// lines and columns (columns in code units, so a pair counts two), and
// the source size limit.

import { createLexer, createLexerWithLimit, nextToken } from "../../src/syntax/lexer.js";
import { lexError } from "../../src/syntax/lex_messages.js";
import { formatError } from "../../src/core/errors.js";
import { dumpTokens, byteOffsets, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text.length} units`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

show("a\nb\r\nc\rd\u2028e\u2029f");
show("a\tb\u000bc\u000cd\u00a0e\ufefff\u2003g\u3000h\u1680i\u202fj");
show("\ud835\udc65 b\n\ud835\udc65\ud835\udc65 c");
show("x\n\n\n  'unterminated");

// Byte offsets, as the reference counts them.
const units = unitsOf("a\u00e9\u4e2d\ud83d\ude00b");
const table = byteOffsets(units);
let line = "bytes";
for (const b of table) {
  line += ` ${b}`;
}
console.log(line);
dumpTokens(createLexer(units, false), byteOffsets(units), (l) => console.log(l));

// The size limit, in UTF-8 bytes, with the limit and the size named.
const big = createLexerWithLimit(unitsOf("abc\u00e9"), false, 4);
console.log(`next ${nextToken(big)}: ${formatError(lexError(big))}`);
const fits = createLexerWithLimit(unitsOf("abc\u00e9"), false, 5);
console.log(`next ${nextToken(fits)}, error ${lexError(fits)}`);
