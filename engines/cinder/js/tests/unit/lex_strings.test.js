// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// String literals: every escape, line continuations, legacy octal
// escapes (flagged), raw LS and PS, and the errors.

import { createLexer, nextToken } from "../../src/syntax/lexer.js";
import { stringValue, F_LEGACY_OCTAL } from "../../src/syntax/tokens.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text.length} units`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

show("'' \"\" 'a' \"it's\" 'say \"hi\"'");
show("'\\n\\t\\r\\b\\f\\v\\0\\q\\'\\\"\\\\'");
show("'\\x41\\x7e\\u0041\\u{1F600}\\u{10FFFF}\\uD800\\u{0000041}'");
show("'\\00' '\\101' '\\377' '\\400' '\\8\\9' '\\08' '\\7a'");
show("'a\\\nb' 'a\\\r\nb' 'a\\\rb' 'a\\\u2028b' '\u2028\u2029'");
show("'\u00e9\ud83d\ude00' '\\\u00e9' '\\\ud83d\ude00'");
show("'abc");
show("'ab\ncd'");
show("'a\r'");
show("'\\x4'");
show("'\\xZZ'");
show("'\\u12'");
show("'\\u{}'");
show("'\\u{110000}'");
show("'\\u{12'");
show("'abc\\");

// Values without escapes are read from the source; with escapes they are
// kept as code unit arrays.
const lx = createLexer(unitsOf("'plain' 'e\\x73c' '\\1'"), false);
const a = nextToken(lx);
const b = nextToken(lx);
const c = nextToken(lx);
const t = lx.tokens;
console.log(`aux ${t.aux[a]} ${t.aux[b]} ${t.aux[c]}, values ${t.values.length}`);
console.log(`${stringValue(t, lx.src, b).length} units, legacy ${(t.flags[c] & F_LEGACY_OCTAL) !== 0}`);
