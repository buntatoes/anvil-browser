// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Regular expression literals: read again from a \`/\` or \`/=\` token when
// the parser asks; classes, escapes, flags (syntax only), and errors.

import { createLexer, nextToken, rescanRegex } from "../../src/syntax/lexer.js";
import { TK_REGEX, regexPattern, regexFlags } from "../../src/syntax/tokens.js";
import { lexErrorMessage } from "../../src/syntax/lex_messages.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text}`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

show("x = [/a/, /[/]/, /\\//i, /[\\]/]/m, /a|b/gimsuy, /(?:x)/d]");
show("x = /=/ ; y /= 2 ; z = /=a/g");
show("x = /\\u{1F600}/u, /[a-z]/v, /[[]]/, /\\[/");
show("if (a) /re/.test(s); a = (b) / c / d");
show("return /r/; typeof /t/; a[0] / 2; a++\n/ 2; \`\${a}\` / 2");
show("x = /abc");
show("x = /ab\ncd/");
show("x = /a\\\n/");
show("x = /a/gg");
show("x = /a/z");
show("x = /a/uv");
show("x = /a/\\u0067");
show("x = /[/");
show("x = /a/g\u00e9");

// Directly: the same \`/=\` token as division or as a regex.
const lx = createLexer(unitsOf("/=x/g"), false);
const t = nextToken(lx);
console.log(`first token kind ${lx.tokens.kind[t]} ends at ${lx.tokens.end[t]}`);
const r = rescanRegex(lx, t);
const p = regexPattern(lx.tokens, lx.src, r);
const f = regexFlags(lx.tokens, lx.src, r);
console.log(`regex ${lx.tokens.kind[r] === TK_REGEX}: ${p.length} pattern units, ${f.length} flags, ends at ${lx.tokens.end[r]}`);
const bad = createLexer(unitsOf("/a"), false);
rescanRegex(bad, nextToken(bad));
console.log(lexErrorMessage(bad));
