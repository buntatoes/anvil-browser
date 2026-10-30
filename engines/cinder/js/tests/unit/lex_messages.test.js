// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Every error code has its message, and errors come out as engine error
// objects with the line and column of their start.
// expect: 32 messages for 32 codes
// expect: 3:5: unterminated string literal (offset 8)
// expect: SyntaxError
// expect: U+0041 U+1F600 U+10FFFF

import { createLexer, nextToken, E_SOURCE_TOO_LARGE } from "../../src/syntax/lexer.js";
import { lexError, MESSAGE_COUNT, hex4 } from "../../src/syntax/lex_messages.js";
import { formatError } from "../../src/core/errors.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

console.log(`${MESSAGE_COUNT} messages for ${E_SOURCE_TOO_LARGE + 1} codes`);
const lx = createLexer(unitsOf("a\nb\nc + 'x"), false);
while (nextToken(lx) >= 0) {
  // Lex up to the error.
}
const e = lexError(lx);
console.log(`${formatError(e)} (offset ${e.offset})`);
console.log(e.name);
console.log(`U+${hex4(0x41)} U+${hex4(0x1f600)} U+${hex4(0x10ffff)}`);
