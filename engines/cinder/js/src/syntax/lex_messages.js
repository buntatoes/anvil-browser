// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Text of the lexer's errors, and the error object for a failed lexer.
// The messages are the reference front end's, word for word; see
// tests/DIFFERENCES.md for where the place of an error differs.

import { syntaxError, sourceTooLarge } from "../core/errors.js";
import { highSurrogateOf, lowSurrogateOf } from "../core/text.js";
import { E_NONE, E_CHAR, E_FLAG_UNKNOWN, E_FLAG_DUPLICATE, E_SOURCE_TOO_LARGE } from "./lexer.js";

// By error code (see lexer.js).
const MESSAGES = [
  "",
  "unterminated comment",
  "`#` must be followed by a name",
  "unexpected NUL character",
  "unexpected character U+",
  "expected `u` after `\\` in a name",
  "escaped character is not allowed in a name",
  "escaped surrogate is not allowed in a name",
  "expected a name",
  "code point is past U+10FFFF",
  "malformed `\\u{...}` escape",
  "empty `\\u{}` escape",
  "`\\u` needs four hex digits",
  "`_` must sit between two digits",
  "missing digits after the radix prefix",
  "`_` is not allowed in a legacy octal literal",
  "BigInt literals can't start with 0",
  "`_` can't follow a leading 0",
  "a BigInt literal must be an integer",
  "a number can't be followed directly by a name or digit",
  "`_` can't follow the decimal point",
  "missing exponent digits",
  "unterminated string literal",
  "unterminated literal",
  "`\\x` needs two hex digits",
  "unterminated template literal",
  "unterminated regular expression",
  "escapes are not allowed in regex flags",
  "unknown regex flag ",
  "duplicate regex flag ",
  "regex flags `u` and `v` can't be used together",
  "",
];

const HEX = "0123456789ABCDEF";

// Upper-case hex of cp, at least four digits.
export function hex4(cp) {
  let s = "";
  let v = cp;
  while (v > 0 || s.length < 4) {
    s = HEX.slice(v % 16, v % 16 + 1) + s;
    v = Math.floor(v / 16);
  }
  return s;
}

// The string of one code point.
export function charOf(cp) {
  if (cp > 0xffff) {
    return String.fromCharCode(highSurrogateOf(cp), lowSurrogateOf(cp));
  }
  return String.fromCharCode(cp);
}

// The message for the lexer's error (empty when there is none).
export function lexErrorMessage(lx) {
  const code = lx.error;
  if (code === E_NONE) {
    return "";
  }
  if (code === E_CHAR) {
    return MESSAGES[code] + hex4(lx.errorArg);
  }
  if (code === E_FLAG_UNKNOWN || code === E_FLAG_DUPLICATE) {
    return MESSAGES[code] + "`" + charOf(lx.errorArg) + "`";
  }
  if (code === E_SOURCE_TOO_LARGE) {
    return sourceTooLarge(lx.errorArg, lx.maxBytes).message;
  }
  return MESSAGES[code];
}

// The lexer's error as an engine error object (see core/errors.js), or
// null when there is none.
export function lexError(lx) {
  if (lx.error === E_NONE) {
    return null;
  }
  if (lx.error === E_SOURCE_TOO_LARGE) {
    return sourceTooLarge(lx.errorArg, lx.maxBytes);
  }
  return syntaxError(lexErrorMessage(lx), lx.starts, lx.errorStart, lx.errorEnd);
}

// Number of messages: one per error code, with an empty one for E_NONE.
export const MESSAGE_COUNT = MESSAGES.length;
