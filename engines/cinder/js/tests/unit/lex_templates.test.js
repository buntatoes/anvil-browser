// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Templates: pieces, nesting, continuation rescans, cooked and raw
// values, CR and CRLF, escapes only tagged templates allow, errors.

import { createLexer, nextToken, rescanTemplate } from "../../src/syntax/lexer.js";
import { TK_TEMPLATE, F_TAIL, templateRaw } from "../../src/syntax/tokens.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text.length} units`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

const BT = "`";
const D = "${";

show(`${BT}${BT} ${BT}a${BT} ${BT}a${D}b}c${BT} ${BT}${D}a}${D}b}${BT}`);
show(`${BT}a${D}${BT}b${D}c}d${BT}}e${BT} ${BT}${D} {} }${BT} ${BT}${D} {a: {}} }x${BT}`);
show(`${BT}\\n\\u0041\\x41\\u{1F600}${BT} ${BT}\\'\\"\\\\${BT}`);
show(`${BT}\\unicode${BT} ${BT}\\u{${BT} ${BT}\\x${BT} ${BT}\\01${BT} ${BT}\\8${BT} ${BT}\\0${BT} ${BT}\\u{110000}${BT}`);
show(`${BT}a\r\nb${BT} ${BT}a\rb${BT} ${BT}a\\\r\nb${BT} ${BT}a\\\u2028b${BT}`);
show(`${BT}$${BT} ${BT}$$${D}a}$${BT} ${BT}}${BT} ${BT}\\${BT}${BT} ${BT}\\${D}a}${BT}`);
show(`${BT}a${D}b /* } */}c${BT} ${BT}${D}a + ${BT}${D}b}${BT}}${BT}`);
show(`${BT}abc`);
show(`${BT}a${D}b}c`);
show(`${BT}\\`);

// The parser drives continuations: a `}` is a punctuator until it asks.
const lx = createLexer(unitsOf(`${BT}x${D}1}y${BT}`), false);
const head = nextToken(lx);
nextToken(lx);
const brace = nextToken(lx);
console.log(`head tail: ${(lx.tokens.flags[head] & F_TAIL) !== 0}, brace kind: ${lx.tokens.kind[brace]}`);
const piece = rescanTemplate(lx, brace);
const raw = templateRaw(lx.tokens, lx.src, piece);
console.log(
  `after rescan: template ${lx.tokens.kind[piece] === TK_TEMPLATE}, tail ${(lx.tokens.flags[piece] & F_TAIL) !== 0}, raw ${raw.length} unit, ${lx.tokens.kind.length} tokens`,
);
