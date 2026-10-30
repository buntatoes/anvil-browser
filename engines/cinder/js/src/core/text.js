// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Code units and code points: surrogate pairs, line terminators, white
// space and identifier characters, as the language spec defines them.
//
// Source text is handled as a list of UTF-16 code units (an array or a
// typed array of numbers), so this module needs no string operations and
// stays plain.

import { ID_START_RANGES, ID_CONTINUE_EXTRA_RANGES } from "./unicode_tables.js";

export const CP_LF = 0x0a;
export const CP_CR = 0x0d;
export const CP_LS = 0x2028;
export const CP_PS = 0x2029;
export const CP_ZWNJ = 0x200c;
export const CP_ZWJ = 0x200d;
export const MAX_CODE_POINT = 0x10ffff;

export function isHighSurrogate(cu) {
  return cu >= 0xd800 && cu <= 0xdbff;
}

export function isLowSurrogate(cu) {
  return cu >= 0xdc00 && cu <= 0xdfff;
}

export function isSurrogate(cu) {
  return cu >= 0xd800 && cu <= 0xdfff;
}

// The code point of a high and low surrogate pair.
export function combineSurrogates(hi, lo) {
  return (hi - 0xd800) * 0x400 + (lo - 0xdc00) + 0x10000;
}

// High surrogate of a code point above U+FFFF.
export function highSurrogateOf(cp) {
  return Math.floor((cp - 0x10000) / 0x400) + 0xd800;
}

// Low surrogate of a code point above U+FFFF.
export function lowSurrogateOf(cp) {
  return ((cp - 0x10000) % 0x400) + 0xdc00;
}

// The code point starting at index i of units: a pair when a high
// surrogate is followed by a low one, otherwise the code unit itself (a
// lone surrogate stays as it is). -1 past the end.
export function codePointAt(units, i) {
  if (i < 0 || i >= units.length) {
    return -1;
  }
  const a = units[i];
  if (isHighSurrogate(a) && i + 1 < units.length) {
    const b = units[i + 1];
    if (isLowSurrogate(b)) {
      return combineSurrogates(a, b);
    }
  }
  return a;
}

// How many code units a code point takes.
export function codeUnitCount(cp) {
  if (cp > 0xffff) {
    return 2;
  }
  return 1;
}

// How many UTF-8 bytes a code point takes. A lone surrogate counts as 3,
// the size of its replacement character.
export function utf8Length(cp) {
  if (cp < 0x80) {
    return 1;
  }
  if (cp < 0x800) {
    return 2;
  }
  if (cp < 0x10000) {
    return 3;
  }
  return 4;
}

// UTF-8 byte offset of code-unit offset `end` in units. The reference
// front end reports byte offsets; this converts ours for comparison.
export function utf8Offset(units, end) {
  let bytes = 0;
  let i = 0;
  while (i < end && i < units.length) {
    const cp = codePointAt(units, i);
    bytes = bytes + utf8Length(cp);
    i = i + codeUnitCount(cp);
  }
  return bytes;
}

// LineTerminator: LF, CR, LS, PS.
export function isLineTerminator(cp) {
  return cp === CP_LF || cp === CP_CR || cp === CP_LS || cp === CP_PS;
}

// WhiteSpace: TAB, VT, FF, ZWNBSP and every Space_Separator (SP, NBSP,
// U+1680, U+2000 to U+200A, U+202F, U+205F, U+3000).
export function isWhitespace(cp) {
  if (cp === 0x09 || cp === 0x0b || cp === 0x0c || cp === 0x20 || cp === 0xa0) {
    return true;
  }
  if (cp < 0x1680) {
    return false;
  }
  return (
    cp === 0x1680 ||
    (cp >= 0x2000 && cp <= 0x200a) ||
    cp === 0x202f ||
    cp === 0x205f ||
    cp === 0x3000 ||
    cp === 0xfeff
  );
}

// Binary search in a flat list of inclusive [first, last] pairs.
export function inRanges(table, cp) {
  let lo = 0;
  let hi = table.length / 2;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (cp < table[mid * 2]) {
      hi = mid;
    } else if (cp > table[mid * 2 + 1]) {
      lo = mid + 1;
    } else {
      return true;
    }
  }
  return false;
}

function isAsciiLetter(cp) {
  return (cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a);
}

// Can cp start an identifier: `$`, `_`, letters. Above ASCII the table
// follows the reference front end.
// TODO: the tables come from the Unicode letter and digit classes, not the
// exact ID_Start and ID_Continue properties; both front ends share this
// gap, so they agree with each other, not yet with the spec.
export function isIdStart(cp) {
  if (cp < 0x80) {
    return isAsciiLetter(cp) || cp === 0x24 || cp === 0x5f;
  }
  return inRanges(ID_START_RANGES, cp);
}

// Can cp continue an identifier: what can start one, digits, and above
// ASCII also ZWNJ and ZWJ (both in the extra table).
export function isIdContinue(cp) {
  if (cp < 0x80) {
    return isAsciiLetter(cp) || (cp >= 0x30 && cp <= 0x39) || cp === 0x24 || cp === 0x5f;
  }
  return inRanges(ID_START_RANGES, cp) || inRanges(ID_CONTINUE_EXTRA_RANGES, cp);
}
