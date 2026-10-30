// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Code points, surrogates, white space, line terminators and identifier
// characters.
// expect: surrogates ok
// expect: codePointAt ok
// expect: utf8 ok
// expect: whitespace ok
// expect: line terminators ok
// expect: identifiers ok

import {
  isHighSurrogate,
  isLowSurrogate,
  isSurrogate,
  combineSurrogates,
  highSurrogateOf,
  lowSurrogateOf,
  codePointAt,
  codeUnitCount,
  utf8Length,
  utf8Offset,
  isLineTerminator,
  isWhitespace,
  isIdStart,
  isIdContinue,
} from "../../src/core/text.js";

let failures = 0;

function check(what, got, want) {
  if (got !== want) {
    failures++;
    console.log(`FAIL ${what}: got ${got}, want ${want}`);
  }
}

function section(name) {
  if (failures === 0) {
    console.log(`${name} ok`);
  }
  failures = 0;
}

check("high", isHighSurrogate(0xd83d), true);
check("high edge", isHighSurrogate(0xdc00), false);
check("low", isLowSurrogate(0xde00), true);
check("any", isSurrogate(0xdfff), true);
check("not", isSurrogate(0xe000), false);
check("combine", combineSurrogates(0xd83d, 0xde00), 0x1f600);
check("hi of", highSurrogateOf(0x1f600), 0xd83d);
check("lo of", lowSurrogateOf(0x1f600), 0xde00);
check("max hi", highSurrogateOf(0x10ffff), 0xdbff);
check("max lo", lowSurrogateOf(0x10ffff), 0xdfff);
for (let cp = 0x10000; cp <= 0x10ffff; cp += 4099) {
  check("round trip", combineSurrogates(highSurrogateOf(cp), lowSurrogateOf(cp)), cp);
}
section("surrogates");

const units = [0x61, 0xd83d, 0xde00, 0xd800, 0x62, 0xdc00];
check("plain", codePointAt(units, 0), 0x61);
check("pair", codePointAt(units, 1), 0x1f600);
check("second half", codePointAt(units, 2), 0xde00);
check("lone high", codePointAt(units, 3), 0xd800);
check("lone low", codePointAt(units, 5), 0xdc00);
check("past end", codePointAt(units, 6), -1);
check("before start", codePointAt(units, -1), -1);
check("units of pair", codeUnitCount(0x1f600), 2);
check("units of bmp", codeUnitCount(0xffff), 1);
const typed = new Uint16Array(2);
typed[0] = 0xdbff;
typed[1] = 0xdfff;
check("typed", codePointAt(typed, 0), 0x10ffff);
section("codePointAt");

check("1 byte", utf8Length(0x7f), 1);
check("2 bytes", utf8Length(0x80), 2);
check("3 bytes", utf8Length(0xffff), 3);
check("4 bytes", utf8Length(0x10000), 4);
check("offset", utf8Offset([0x61, 0xe9, 0xd83d, 0xde00, 0x62], 4), 7);
check("lone", utf8Offset([0xd800, 0x61], 2), 4);
section("utf8");

const spaces = [0x09, 0x0b, 0x0c, 0x20, 0xa0, 0x1680, 0x2000, 0x2005, 0x200a, 0x202f, 0x205f, 0x3000, 0xfeff];
for (const s of spaces) {
  check(`space ${s}`, isWhitespace(s), true);
}
const notSpaces = [0x0a, 0x0d, 0x2028, 0x2029, 0x200b, 0x180e, 0x85, 0x61, 0x1fff, 0x200c];
for (const s of notSpaces) {
  check(`not space ${s}`, isWhitespace(s), false);
}
section("whitespace");

for (const t of [0x0a, 0x0d, 0x2028, 0x2029]) {
  check(`terminator ${t}`, isLineTerminator(t), true);
}
for (const t of [0x85, 0x0b, 0x0c, 0x20]) {
  check(`not terminator ${t}`, isLineTerminator(t), false);
}
section("line terminators");

check("$", isIdStart(0x24), true);
check("_", isIdStart(0x5f), true);
check("A", isIdStart(0x41), true);
check("z", isIdStart(0x7a), true);
check("0 start", isIdStart(0x30), false);
check("0 continue", isIdContinue(0x30), true);
check("-", isIdContinue(0x2d), false);
check("e acute", isIdStart(0xe9), true);
check("greek", isIdStart(0x3bb), true);
check("cjk", isIdStart(0x4e00), true);
check("astral letter", isIdStart(0x1d400), true);
check("zwnj start", isIdStart(0x200c), false);
check("zwnj continue", isIdContinue(0x200c), true);
check("zwj continue", isIdContinue(0x200d), true);
check("arabic digit", isIdContinue(0x663), true);
check("arabic digit start", isIdStart(0x663), false);
check("nbsp", isIdContinue(0xa0), false);
check("ls", isIdContinue(0x2028), false);
check("emoji", isIdStart(0x1f600), false);
section("identifiers");
