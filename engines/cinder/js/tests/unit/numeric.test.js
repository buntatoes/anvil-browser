// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Numeric values: decimal and radix literals rounded to the nearest
// double (ties to even), checked against literals the host already read,
// and BigInt digits in decimal.

import { decimalValue, radixValue, decimalDigitsOf } from "../../src/syntax/numeric.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

let failures = 0;
let checks = 0;

function dec(text, want) {
  const u = unitsOf(text);
  const got = decimalValue(u, 0, u.length);
  checks += 1;
  if (got !== want || 1 / got !== 1 / want) {
    failures += 1;
    console.log(`FAIL ${text}: got ${got}, want ${want}`);
  }
}

function rad(text, radix, want) {
  const u = unitsOf(text);
  const got = radixValue(u, 0, u.length, radix);
  checks += 1;
  if (got !== want) {
    failures += 1;
    console.log(`FAIL ${text} in ${radix}: got ${got}, want ${want}`);
  }
}

dec("0", 0);
dec("0.0", 0);
dec("1", 1);
dec("1.5", 1.5);
dec(".5", 0.5);
dec("5.", 5);
dec("1_000", 1000);
dec("1e3", 1000);
dec("1E-3", 0.001);
dec("0.1", 0.1);
dec("0.3", 0.3);
dec("4.35", 4.35);
dec("123456789012345678901234567890", 123456789012345678901234567890);
dec("9007199254740993", 9007199254740992);
dec("9007199254740995", 9007199254740996);
dec("18014398509481985", 18014398509481984);
dec("1e400", 1e400);
dec("1e308", 1e308);
dec("1.7976931348623157e308", 1.7976931348623157e308);
dec("1.7976931348623158e308", 1.7976931348623157e308);
dec("1.7976931348623159e308", 1e400);
dec("5e-324", 5e-324);
dec("2e-324", 0);
dec("3e-324", 5e-324);
dec("2.4703282292062327e-324", 0);
dec("2.4703282292062328e-324", 5e-324);
dec("2.2250738585072011e-308", 2.2250738585072011e-308);
dec("2.2250738585072014e-308", 2.2250738585072014e-308);
dec("8.98846567431158e307", 8.98846567431158e307);
dec("0.0000000000000000000000000001", 1e-28);
dec("1e-400", 0);
dec("123.456e-200", 1.23456e-198);
dec("7.2057594037927933e16", 72057594037927936);
dec("2.2204460492503131e-16", 2.220446049250313e-16);
dec("3.14159265358979323846264338327950288", 3.141592653589793);
dec("1_2_3.4_5e1_0", 1.2345e12);
dec("0.000000000000000000000000000000000000000000001e45", 1);
dec("100000000000000000000000000000000000000000000000000000000000000000000000000000000001", 1e83);
rad("ff", 16, 255);
rad("1_0", 2, 2);
rad("777", 8, 511);
rad("20000000000001", 16, 9007199254740992);
rad("20000000000003", 16, 9007199254740996);
rad("1fffffffffffff", 16, 9007199254740991);
rad("ffffffffffffffffffff", 16, 1208925819614629174706176);
rad("777777777777777777777", 8, 9223372036854775808);
console.log(`${checks - failures} of ${checks} values exact`);

function digits(text, radix) {
  let s = "";
  for (const d of decimalDigitsOf(unitsOf(text), radix)) {
    s += `${d}`;
  }
  return s;
}
console.log(digits("0", 10));
console.log(digits("ff", 16));
console.log(digits("ffffffffffffffffffffffffffffffff", 16));
console.log(digits("777", 8));
console.log(digits("1010", 2));
console.log(digits("12345678901234567890123456789", 10));
