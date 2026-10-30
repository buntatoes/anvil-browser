// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Numeric literals: every base, separators, legacy octal and leading-zero
// decimals, BigInt, values at the edges of double rounding, and the
// malformed forms with their errors.

import { createLexer } from "../../src/syntax/lexer.js";
import { dumpTokens, unitsOf } from "../../src/syntax/token_dump.js";

function show(text) {
  console.log(`-- ${text}`);
  dumpTokens(createLexer(unitsOf(text), false), null, (l) => console.log(l));
}

show("0 1 42 0.5 .5 5. 5.e3 1e10 1E-7 1e+300 1_000_000 1_0.0_1e1_0 0e0 00");
show("0x0 0xFF 0Xab_cd 0o17 0O7_7 0b1010 0B1_1 0xffffffffffffffffffff 0x20000000000001 0x1fffffffffffff");
show("017 08 09 08.5 09e1 0777 019");
show("0n 10n 0x1Fn 0o7n 0b11n 1_2n 0x00ffn 000x");
show("9007199254740993 18014398509481985 123456789012345678901234567890");
show("1e400 5e-324 2e-324 3e-324 2.4703282292062327e-324 2.4703282292062328e-324");
show("1.7976931348623157e308 1.7976931348623158e308 1.7976931348623159e308 2.2250738585072011e-308");
show("0.1 0.3 4.35 1e21 1e-7 0.0000000000000000000000000001");
show("1..x 1.5.y");
show("1_");
show("1__0");
show("0_1");
show("0x");
show("0x_1");
show("0b2");
show("1e");
show("1e_1");
show("1._5");
show("08n");
show("07_1");
show("1.5n");
show(".5n");
show("3in");
show("5\\u0061");
show("0b12");
