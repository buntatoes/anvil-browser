// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The lexer: source code units in, tokens out (spec clause 12 and annex
// B.1). The source is an array of UTF-16 code units; the state is one
// record, passed to every function; tokens go to the parallel arrays of
// tokens.js. Nothing here allocates per token except for values that
// differ from the source text (names with escapes, strings with escapes,
// big numbers).
//
// Two choices depend on where the parser is, so the parser drives them,
// as in the reference: a `/` or `/=` token at the start of an expression
// is read again as a regular expression with rescanRegex, and a `}` that
// closes a template substitution is read again as the next template piece
// with rescanTemplate.
//
// Errors: a function that fails records an error code, its start and end
// offsets and an argument in the state and returns -1 (or false); the
// first error stops the lexer. lex_messages.js turns the record into a
// message. Error places match the reference: most errors sit at the start
// of the token, escape errors at the backslash.

import { MAX_SOURCE_BYTES } from "../core/limits.js";
import {
  CP_LF,
  CP_CR,
  CP_LS,
  CP_PS,
  highSurrogateOf,
  lowSurrogateOf,
  codePointAt,
  codeUnitCount,
  utf8Offset,
  isWhitespace,
  isIdStart,
  isIdContinue,
} from "../core/text.js";
import { lineStarts, lineIndex } from "../core/positions.js";
import {
  TK_EOF,
  TK_IDENT,
  TK_PRIVATE,
  TK_PUNCT,
  TK_NUMBER,
  TK_BIGINT,
  TK_STRING,
  TK_TEMPLATE,
  TK_REGEX,
  F_NL,
  F_ESCAPED,
  F_LEGACY_OCTAL,
  F_TAIL,
  F_BAD_ESCAPE,
  F_RADIX_2,
  F_RADIX_8,
  F_RADIX_16,
  P_LBRACE,
  P_RBRACE,
  P_LPAREN,
  P_RPAREN,
  P_LBRACKET,
  P_RBRACKET,
  P_DOT,
  P_ELLIPSIS,
  P_SEMICOLON,
  P_COMMA,
  P_LT,
  P_GT,
  P_LT_EQ,
  P_GT_EQ,
  P_EQ_EQ,
  P_NOT_EQ,
  P_EQ_EQ_EQ,
  P_NOT_EQ_EQ,
  P_PLUS,
  P_MINUS,
  P_STAR,
  P_PERCENT,
  P_STAR_STAR,
  P_PLUS_PLUS,
  P_MINUS_MINUS,
  P_SHL,
  P_SHR,
  P_USHR,
  P_AMP,
  P_PIPE,
  P_CARET,
  P_BANG,
  P_TILDE,
  P_AMP_AMP,
  P_PIPE_PIPE,
  P_QUESTION_QUESTION,
  P_QUESTION,
  P_QUESTION_DOT,
  P_COLON,
  P_EQ,
  P_PLUS_EQ,
  P_MINUS_EQ,
  P_STAR_EQ,
  P_PERCENT_EQ,
  P_STAR_STAR_EQ,
  P_SHL_EQ,
  P_SHR_EQ,
  P_USHR_EQ,
  P_AMP_EQ,
  P_PIPE_EQ,
  P_CARET_EQ,
  P_AMP_AMP_EQ,
  P_PIPE_PIPE_EQ,
  P_QUESTION_QUESTION_EQ,
  P_ARROW,
  P_SLASH,
  P_SLASH_EQ,
  newTokens,
  pushToken,
  truncateTokens,
  pushValue,
  pushNumber,
  sliceUnits,
} from "./tokens.js";
import { decimalValue, radixValue } from "./numeric.js";

// Error codes. lex_messages.js has the text of each.
export const E_NONE = 0;
export const E_UNTERMINATED_COMMENT = 1;
export const E_HASH = 2;
export const E_NUL = 3;
export const E_CHAR = 4;
export const E_ESCAPE_NOT_U = 5;
export const E_ESCAPE_NOT_ID = 6;
export const E_ESCAPE_SURROGATE = 7;
export const E_EXPECTED_NAME = 8;
export const E_CODE_POINT_RANGE = 9;
export const E_BRACE_ESCAPE = 10;
export const E_EMPTY_BRACE_ESCAPE = 11;
export const E_U_FOUR_DIGITS = 12;
export const E_SEPARATOR = 13;
export const E_RADIX_DIGITS = 14;
export const E_LEGACY_SEPARATOR = 15;
export const E_BIGINT_LEADING_ZERO = 16;
export const E_LEADING_ZERO_SEPARATOR = 17;
export const E_BIGINT_NOT_INTEGER = 18;
export const E_NUMBER_TAIL = 19;
export const E_DOT_SEPARATOR = 20;
export const E_EXPONENT_DIGITS = 21;
export const E_UNTERMINATED_STRING = 22;
export const E_UNTERMINATED_LITERAL = 23;
export const E_X_TWO_DIGITS = 24;
export const E_UNTERMINATED_TEMPLATE = 25;
export const E_UNTERMINATED_REGEX = 26;
export const E_FLAG_ESCAPE = 27;
export const E_FLAG_UNKNOWN = 28;
export const E_FLAG_DUPLICATE = 29;
export const E_FLAGS_U_AND_V = 30;
export const E_SOURCE_TOO_LARGE = 31;

// How readEscape ended.
const ESC_FAILED = -1;
const ESC_NORMAL = 0;
const ESC_CONTINUATION = 1;
const ESC_LEGACY = 2;
const ESC_INVALID = 3;

// A lexer over `src` (code units). `isModule` picks the Module goal, where
// HTML-like comments are not comments.
export function createLexer(src, isModule) {
  return createLexerWithLimit(src, isModule, MAX_SOURCE_BYTES);
}

// The same with another source size limit, in UTF-8 bytes (for tests).
export function createLexerWithLimit(src, isModule, maxBytes) {
  const lx = {
    src: src,
    len: src.length,
    isModule: isModule,
    pos: 0,
    starts: lineStarts(src),
    lineCursor: 0,
    tokens: newTokens(),
    maxBytes: maxBytes,
    error: E_NONE,
    errorStart: 0,
    errorEnd: 0,
    errorArg: 0,
    // Set by skipTrivia: whether a line terminator was crossed.
    newline: false,
    // Results of readIdentName.
    nameAux: -1,
    nameEscaped: false,
  };
  // Every code unit takes at least one byte, so only a long source needs
  // the exact count.
  if (src.length > maxBytes / 3) {
    const bytes = utf8Offset(src, src.length);
    if (bytes > maxBytes) {
      lx.error = E_SOURCE_TOO_LARGE;
      lx.errorArg = bytes;
    }
  }
  return lx;
}

// Record an error; returns -1.
function fail(lx, code, start, arg) {
  if (lx.error === E_NONE) {
    lx.error = code;
    lx.errorStart = start;
    lx.errorEnd = lx.pos;
    lx.errorArg = arg;
  }
  return -1;
}

// 0-based line index of offset. Tokens mostly move forward, so this walks
// from the last answer and falls back to a binary search when going back.
function lineAt(lx, offset) {
  const starts = lx.starts;
  let i = lx.lineCursor;
  if (starts[i] > offset) {
    i = lineIndex(starts, offset);
  } else {
    while (i + 1 < starts.length && starts[i + 1] <= offset) {
      i = i + 1;
    }
  }
  lx.lineCursor = i;
  return i;
}

function emit(lx, kind, start, end, flags, aux) {
  const li = lineAt(lx, start);
  return pushToken(lx.tokens, kind, start, end, li + 1, start - lx.starts[li] + 1, flags, aux);
}

// The unit at offset i, or -1 past the end.
function unitAt(lx, i) {
  if (i < lx.len) {
    return lx.src[i];
  }
  return -1;
}

function isDigit(c) {
  return c >= 0x30 && c <= 0x39;
}

function isHexDigit(c) {
  return (c >= 0x30 && c <= 0x39) || (c >= 0x61 && c <= 0x66) || (c >= 0x41 && c <= 0x46);
}

function hexValue(c) {
  if (c <= 0x39) {
    return c - 0x30;
  }
  if (c >= 0x61) {
    return c - 0x61 + 10;
  }
  return c - 0x41 + 10;
}

function isRadixDigit(c, radix) {
  if (radix === 10) {
    return c >= 0x30 && c <= 0x39;
  }
  if (radix === 16) {
    return isHexDigit(c);
  }
  if (radix === 8) {
    return c >= 0x30 && c <= 0x37;
  }
  return c === 0x30 || c === 0x31;
}

function isAsciiIdStart(c) {
  return (c >= 0x61 && c <= 0x7a) || (c >= 0x41 && c <= 0x5a) || c === 0x24 || c === 0x5f;
}

function isAsciiIdContinue(c) {
  return (
    (c >= 0x61 && c <= 0x7a) || (c >= 0x41 && c <= 0x5a) || (c >= 0x30 && c <= 0x39) || c === 0x24 || c === 0x5f
  );
}

// Append the units of code point cp. A lone surrogate value stays one
// unit.
function pushCodePoint(out, cp) {
  if (cp > 0xffff) {
    out.push(highSurrogateOf(cp));
    out.push(lowSurrogateOf(cp));
  } else {
    out.push(cp);
  }
}

function appendRange(out, src, start, end) {
  let i = start;
  while (i < end) {
    out.push(src[i]);
    i = i + 1;
  }
}

// ---- trivia ----

// Skip to the next line terminator (not past it).
function skipLine(lx) {
  const src = lx.src;
  const n = lx.len;
  let i = lx.pos;
  while (i < n) {
    const c = src[i];
    if (c === CP_LF || c === CP_CR || c === CP_LS || c === CP_PS) {
      break;
    }
    i = i + 1;
  }
  lx.pos = i;
}

function startsWith3(lx, a, b, c) {
  const i = lx.pos;
  return i + 2 < lx.len && lx.src[i] === a && lx.src[i + 1] === b && lx.src[i + 2] === c;
}

// Skip whitespace, line terminators and comments. Sets lx.newline.
// Returns 0, or -1 for an unterminated comment.
function skipTrivia(lx) {
  const src = lx.src;
  const n = lx.len;
  const begin = lx.pos;
  let newline = false;
  if (lx.pos === 0 && n >= 2 && src[0] === 0x23 && src[1] === 0x21) {
    // Hashbang comment.
    skipLine(lx);
  }
  while (lx.pos < n) {
    const c = src[lx.pos];
    if (c === 0x20 || c === 0x09) {
      lx.pos = lx.pos + 1;
    } else if (c === CP_LF || c === CP_CR) {
      lx.pos = lx.pos + 1;
      newline = true;
    } else if (c === 0x2f) {
      const d = unitAt(lx, lx.pos + 1);
      if (d === 0x2f) {
        skipLine(lx);
      } else if (d === 0x2a) {
        const start = lx.pos;
        let i = lx.pos + 2;
        let closed = false;
        while (i < n) {
          const e = src[i];
          if (e === 0x2a && i + 1 < n && src[i + 1] === 0x2f) {
            i = i + 2;
            closed = true;
            break;
          }
          if (e === CP_LF || e === CP_CR || e === CP_LS || e === CP_PS) {
            newline = true;
          }
          i = i + 1;
        }
        lx.pos = i;
        if (!closed) {
          return fail(lx, E_UNTERMINATED_COMMENT, start, 0);
        }
      } else {
        break;
      }
    } else if (c < 0x80) {
      if (c === 0x0b || c === 0x0c) {
        lx.pos = lx.pos + 1;
      } else if (!lx.isModule && c === 0x3c && startsWith3(lx, 0x3c, 0x21, 0x2d) && unitAt(lx, lx.pos + 3) === 0x2d) {
        // `<!--` (annex B.1.1).
        skipLine(lx);
      } else if (!lx.isModule && c === 0x2d && (newline || begin === 0) && startsWith3(lx, 0x2d, 0x2d, 0x3e)) {
        // `-->` at the start of a line, or at the start of the input
        // after nothing but whitespace and comments on one line.
        skipLine(lx);
      } else {
        break;
      }
    } else {
      const cp = codePointAt(src, lx.pos);
      if (isWhitespace(cp)) {
        lx.pos = lx.pos + codeUnitCount(cp);
      } else if (cp === CP_LS || cp === CP_PS) {
        lx.pos = lx.pos + 1;
        newline = true;
      } else {
        break;
      }
    }
  }
  lx.newline = newline;
  return 0;
}

// ---- tokens ----

// Read the next token. Returns its index in lx.tokens, or -1 on error.
export function nextToken(lx) {
  if (lx.error !== E_NONE) {
    return -1;
  }
  if (skipTrivia(lx) < 0) {
    return -1;
  }
  let flags = 0;
  if (lx.newline) {
    flags = F_NL;
  }
  const start = lx.pos;
  if (start >= lx.len) {
    return emit(lx, TK_EOF, start, start, flags, -1);
  }
  const c = lx.src[start];
  if (c < 0x80) {
    if (isAsciiIdStart(c) || c === 0x5c) {
      return readIdentToken(lx, start, flags);
    }
    if (isDigit(c) || (c === 0x2e && isDigit(unitAt(lx, start + 1)))) {
      return readNumber(lx, start, flags);
    }
    if (c === 0x22 || c === 0x27) {
      return readString(lx, start, flags, c);
    }
    if (c === 0x60) {
      lx.pos = start + 1;
      return readTemplate(lx, start, flags);
    }
    if (c === 0x23) {
      return readPrivateName(lx, start, flags);
    }
    return readPunct(lx, start, flags);
  }
  const cp = codePointAt(lx.src, start);
  if (isIdStart(cp)) {
    return readIdentToken(lx, start, flags);
  }
  lx.pos = start + codeUnitCount(cp);
  return fail(lx, E_CHAR, start, cp);
}

function readIdentToken(lx, start, flags) {
  if (!readIdentName(lx, start)) {
    return -1;
  }
  let f = flags;
  if (lx.nameEscaped) {
    f = f | F_ESCAPED;
  }
  return emit(lx, TK_IDENT, start, lx.pos, f, lx.nameAux);
}

function readPrivateName(lx, start, flags) {
  lx.pos = start + 1;
  const n = unitAt(lx, lx.pos);
  if (n >= 0 && (n === 0x5c || isIdStart(codePointAt(lx.src, lx.pos)))) {
    if (!readIdentName(lx, lx.pos)) {
      return -1;
    }
    let f = flags;
    if (lx.nameEscaped) {
      f = f | F_ESCAPED;
    }
    return emit(lx, TK_PRIVATE, start, lx.pos, f, lx.nameAux);
  }
  return fail(lx, E_HASH, start, 0);
}

// Read an IdentifierName at lx.pos. On success sets lx.nameAux (-1 when
// the name is the source text, else the index of the decoded name) and
// lx.nameEscaped.
function readIdentName(lx, start) {
  const src = lx.src;
  const n = lx.len;
  const nameStart = lx.pos;
  let first = true;
  let name = null;
  while (lx.pos < n) {
    const c = src[lx.pos];
    if (c === 0x5c) {
      const esc = lx.pos;
      lx.pos = lx.pos + 1;
      if (unitAt(lx, lx.pos) !== 0x75) {
        fail(lx, E_ESCAPE_NOT_U, esc, 0);
        return false;
      }
      lx.pos = lx.pos + 1;
      const cp = readUnicodeEscape(lx, esc, false);
      if (cp < 0) {
        return false;
      }
      if (cp >= 0xd800 && cp <= 0xdfff) {
        fail(lx, E_ESCAPE_SURROGATE, esc, 0);
        return false;
      }
      let ok = false;
      if (first) {
        ok = isIdStart(cp);
      } else {
        ok = isIdContinue(cp);
      }
      if (!ok) {
        fail(lx, E_ESCAPE_NOT_ID, esc, 0);
        return false;
      }
      if (name === null) {
        name = sliceUnits(src, nameStart, esc);
      }
      pushCodePoint(name, cp);
    } else if (c < 0x80) {
      // Letters, `$` and `_`, and digits after the first (inline: this is
      // the hottest loop).
      if (
        !(
          (c >= 0x61 && c <= 0x7a) ||
          (c >= 0x41 && c <= 0x5a) ||
          c === 0x5f ||
          c === 0x24 ||
          (!first && c >= 0x30 && c <= 0x39)
        )
      ) {
        break;
      }
      lx.pos = lx.pos + 1;
      if (name !== null) {
        name.push(c);
      }
    } else {
      const cp = codePointAt(src, lx.pos);
      let ok = false;
      if (first) {
        ok = isIdStart(cp);
      } else {
        ok = isIdContinue(cp);
      }
      if (!ok) {
        break;
      }
      const w = codeUnitCount(cp);
      if (name !== null) {
        appendRange(name, src, lx.pos, lx.pos + w);
      }
      lx.pos = lx.pos + w;
    }
    first = false;
  }
  if (lx.pos === nameStart) {
    fail(lx, E_EXPECTED_NAME, start, 0);
    return false;
  }
  if (name === null) {
    lx.nameAux = -1;
    lx.nameEscaped = false;
  } else {
    lx.nameAux = pushValue(lx.tokens, name);
    lx.nameEscaped = true;
  }
  return true;
}

// After `\u`: four hex digits or `{hex}`. Returns the code point, or -1.
// With `quiet` (templates) a bad escape records no error.
function readUnicodeEscape(lx, esc, quiet) {
  if (unitAt(lx, lx.pos) === 0x7b) {
    lx.pos = lx.pos + 1;
    let value = 0;
    let digits = 0;
    while (true) {
      const c = unitAt(lx, lx.pos);
      if (c === 0x7d) {
        lx.pos = lx.pos + 1;
        break;
      }
      if (c >= 0 && isHexDigit(c)) {
        lx.pos = lx.pos + 1;
        value = value * 16 + hexValue(c);
        digits = digits + 1;
        if (value > 0x10ffff) {
          return escapeFail(lx, quiet, E_CODE_POINT_RANGE, esc);
        }
      } else {
        return escapeFail(lx, quiet, E_BRACE_ESCAPE, esc);
      }
    }
    if (digits === 0) {
      return escapeFail(lx, quiet, E_EMPTY_BRACE_ESCAPE, esc);
    }
    return value;
  }
  let value = 0;
  let k = 0;
  while (k < 4) {
    const c = unitAt(lx, lx.pos);
    if (c >= 0 && isHexDigit(c)) {
      lx.pos = lx.pos + 1;
      value = value * 16 + hexValue(c);
    } else {
      return escapeFail(lx, quiet, E_U_FOUR_DIGITS, esc);
    }
    k = k + 1;
  }
  return value;
}

function escapeFail(lx, quiet, code, esc) {
  if (!quiet) {
    fail(lx, code, esc, 0);
  }
  return -1;
}

// ---- numbers ----

// Digits in `radix` with optional `_` between two digits. Returns how
// many digits were read, or -1.
function readDigits(lx, radix, allowSeparator, start) {
  const src = lx.src;
  const n = lx.len;
  let count = 0;
  while (lx.pos < n) {
    const c = src[lx.pos];
    if ((c >= 0x30 && c <= 0x39 && radix === 10) || (radix !== 10 && isRadixDigit(c, radix))) {
      lx.pos = lx.pos + 1;
      count = count + 1;
    } else if (c === 0x5f && allowSeparator) {
      const next = unitAt(lx, lx.pos + 1);
      if (count === 0 || next < 0 || !isRadixDigit(next, radix)) {
        lx.pos = lx.pos + 1;
        return fail(lx, E_SEPARATOR, start, 0);
      }
      lx.pos = lx.pos + 1;
    } else {
      break;
    }
  }
  return count;
}

// Optional `.digits` and exponent. Returns false on error.
function readDecimalTail(lx, start) {
  if (unitAt(lx, lx.pos) === 0x2e) {
    lx.pos = lx.pos + 1;
    if (unitAt(lx, lx.pos) === 0x5f) {
      fail(lx, E_DOT_SEPARATOR, start, 0);
      return false;
    }
    if (readDigits(lx, 10, true, start) < 0) {
      return false;
    }
  }
  const e = unitAt(lx, lx.pos);
  if (e === 0x65 || e === 0x45) {
    lx.pos = lx.pos + 1;
    const s = unitAt(lx, lx.pos);
    if (s === 0x2b || s === 0x2d) {
      lx.pos = lx.pos + 1;
    }
    const count = readDigits(lx, 10, true, start);
    if (count < 0) {
      return false;
    }
    if (count === 0) {
      fail(lx, E_EXPONENT_DIGITS, start, 0);
      return false;
    }
  }
  return true;
}

// BigInt digits from src[start, end): `_` dropped, lowercase, leading
// zeros dropped, "0" for zero.
function bigintDigitUnits(src, start, end) {
  const out = [];
  let i = start;
  while (i < end) {
    let c = src[i];
    if (c !== 0x5f && !(out.length === 0 && c === 0x30)) {
      if (c >= 0x41 && c <= 0x46) {
        c = c + 0x20;
      }
      out.push(c);
    }
    i = i + 1;
  }
  if (out.length === 0) {
    out.push(0x30);
  }
  return out;
}

function readNumber(lx, start, flags) {
  const src = lx.src;
  const first = src[start];
  const second = unitAt(lx, start + 1);
  let kind = TK_NUMBER;
  let aux = -1;
  let f = flags;
  if (
    first === 0x30 &&
    (second === 0x78 || second === 0x58 || second === 0x6f || second === 0x4f || second === 0x62 || second === 0x42)
  ) {
    let radix = 2;
    let radixFlag = F_RADIX_2;
    if (second === 0x78 || second === 0x58) {
      radix = 16;
      radixFlag = F_RADIX_16;
    } else if (second === 0x6f || second === 0x4f) {
      radix = 8;
      radixFlag = F_RADIX_8;
    }
    lx.pos = start + 2;
    const digitsStart = lx.pos;
    const count = readDigits(lx, radix, true, start);
    if (count < 0) {
      return -1;
    }
    if (count === 0) {
      return fail(lx, E_RADIX_DIGITS, start, 0);
    }
    const digitsEnd = lx.pos;
    if (unitAt(lx, lx.pos) === 0x6e) {
      lx.pos = lx.pos + 1;
      kind = TK_BIGINT;
      f = f | radixFlag;
      aux = pushValue(lx.tokens, bigintDigitUnits(src, digitsStart, digitsEnd));
    } else {
      aux = pushNumber(lx.tokens, radixValue(src, digitsStart, digitsEnd, radix));
    }
  } else if (first === 0x30 && isDigit(second)) {
    // Legacy octal (017) or a decimal with a leading zero (089).
    f = f | F_LEGACY_OCTAL;
    lx.pos = start + 1;
    const digitsStart = lx.pos;
    readDigits(lx, 10, false, start);
    const digitsEnd = lx.pos;
    const next = unitAt(lx, lx.pos);
    if (next === 0x5f) {
      return fail(lx, E_LEGACY_SEPARATOR, start, 0);
    }
    if (next === 0x6e) {
      return fail(lx, E_BIGINT_LEADING_ZERO, start, 0);
    }
    let octal = true;
    let i = digitsStart;
    while (i < digitsEnd) {
      if (src[i] >= 0x38) {
        octal = false;
      }
      i = i + 1;
    }
    if (octal) {
      aux = pushNumber(lx.tokens, radixValue(src, digitsStart, digitsEnd, 8));
    } else {
      if (!readDecimalTail(lx, start)) {
        return -1;
      }
      aux = pushNumber(lx.tokens, decimalValue(src, digitsStart, lx.pos));
    }
  } else {
    let isInt = true;
    if (first === 0x30) {
      lx.pos = start + 1;
      if (unitAt(lx, lx.pos) === 0x5f) {
        return fail(lx, E_LEADING_ZERO_SEPARATOR, start, 0);
      }
    } else if (first !== 0x2e) {
      lx.pos = start;
      if (readDigits(lx, 10, true, start) < 0) {
        return -1;
      }
    } else {
      lx.pos = start;
    }
    const intEnd = lx.pos;
    const t = unitAt(lx, lx.pos);
    if (t === 0x2e || t === 0x65 || t === 0x45) {
      isInt = false;
    }
    if (!readDecimalTail(lx, start)) {
      return -1;
    }
    if (unitAt(lx, lx.pos) === 0x6e) {
      if (!isInt) {
        return fail(lx, E_BIGINT_NOT_INTEGER, start, 0);
      }
      lx.pos = lx.pos + 1;
      kind = TK_BIGINT;
      aux = pushValue(lx.tokens, bigintDigitUnits(src, start, intEnd));
    } else {
      aux = pushNumber(lx.tokens, decimalValue(src, start, lx.pos));
    }
  }
  // The source character right after a numeric literal must not be an
  // IdentifierStart or a DecimalDigit (spec 12.9.3).
  if (lx.pos < lx.len) {
    const c = src[lx.pos];
    let bad = false;
    if (c < 0x80) {
      bad = isAsciiIdStart(c) || isDigit(c) || c === 0x5c;
    } else {
      bad = isIdStart(codePointAt(src, lx.pos));
    }
    if (bad) {
      truncateSide(lx, kind, aux);
      return fail(lx, E_NUMBER_TAIL, start, 0);
    }
  }
  return emit(lx, kind, start, lx.pos, f, aux);
}

// Drop the side-table entry made for a token that failed after all.
function truncateSide(lx, kind, aux) {
  if (kind === TK_NUMBER) {
    lx.tokens.numbers.pop();
  } else if (aux >= 0) {
    lx.tokens.values.pop();
  }
}

// ---- strings and templates ----

// Read one escape at the backslash into `out`. In a string a bad escape
// is an error right away; in a template it comes back as ESC_INVALID.
function readEscape(lx, out, template, start) {
  const esc = lx.pos;
  lx.pos = lx.pos + 1;
  if (lx.pos >= lx.len) {
    fail(lx, E_UNTERMINATED_LITERAL, start, 0);
    return ESC_FAILED;
  }
  const c = lx.src[lx.pos];
  lx.pos = lx.pos + 1;
  if (c === 0x62) {
    out.push(0x08);
  } else if (c === 0x74) {
    out.push(0x09);
  } else if (c === 0x6e) {
    out.push(0x0a);
  } else if (c === 0x76) {
    out.push(0x0b);
  } else if (c === 0x66) {
    out.push(0x0c);
  } else if (c === 0x72) {
    out.push(0x0d);
  } else if (c === CP_CR) {
    if (unitAt(lx, lx.pos) === CP_LF) {
      lx.pos = lx.pos + 1;
    }
    return ESC_CONTINUATION;
  } else if (c === CP_LF || c === CP_LS || c === CP_PS) {
    return ESC_CONTINUATION;
  } else if (c === 0x30 && !isDigit(unitAt(lx, lx.pos))) {
    out.push(0);
  } else if (c >= 0x30 && c <= 0x37) {
    if (template) {
      return ESC_INVALID;
    }
    // Up to three octal digits, value at most 0o377.
    let value = c - 0x30;
    let max = 2;
    if (c <= 0x33) {
      max = 3;
    }
    let k = 1;
    while (k < max) {
      const d = unitAt(lx, lx.pos);
      if (d >= 0x30 && d <= 0x37) {
        lx.pos = lx.pos + 1;
        value = value * 8 + (d - 0x30);
        k = k + 1;
      } else {
        break;
      }
    }
    out.push(value);
    return ESC_LEGACY;
  } else if (c === 0x38 || c === 0x39) {
    if (template) {
      return ESC_INVALID;
    }
    out.push(c);
    return ESC_LEGACY;
  } else if (c === 0x78) {
    let value = 0;
    let k = 0;
    while (k < 2) {
      const d = unitAt(lx, lx.pos);
      if (d >= 0 && isHexDigit(d)) {
        lx.pos = lx.pos + 1;
        value = value * 16 + hexValue(d);
      } else {
        if (template) {
          return ESC_INVALID;
        }
        fail(lx, E_X_TWO_DIGITS, esc, 0);
        return ESC_FAILED;
      }
      k = k + 1;
    }
    out.push(value);
  } else if (c === 0x75) {
    const saved = lx.pos;
    const cp = readUnicodeEscape(lx, esc, template);
    if (cp < 0) {
      if (template) {
        lx.pos = saved;
        return ESC_INVALID;
      }
      return ESC_FAILED;
    }
    pushCodePoint(out, cp);
  } else {
    // Any other character stands for itself (a surrogate pair is copied
    // one unit at a time).
    out.push(c);
  }
  return ESC_NORMAL;
}

function readString(lx, start, flags, quote) {
  const src = lx.src;
  const n = lx.len;
  let f = flags;
  let cooked = null;
  lx.pos = start + 1;
  let segment = lx.pos;
  while (true) {
    if (lx.pos >= n) {
      return fail(lx, E_UNTERMINATED_STRING, start, 0);
    }
    const c = src[lx.pos];
    if (c === quote) {
      if (cooked !== null) {
        appendRange(cooked, src, segment, lx.pos);
      }
      lx.pos = lx.pos + 1;
      break;
    }
    if (c === CP_LF || c === CP_CR) {
      return fail(lx, E_UNTERMINATED_STRING, start, 0);
    }
    if (c === 0x5c) {
      if (cooked === null) {
        cooked = [];
      }
      appendRange(cooked, src, segment, lx.pos);
      const k = readEscape(lx, cooked, false, start);
      if (k === ESC_FAILED) {
        return -1;
      }
      if (k === ESC_LEGACY) {
        f = f | F_LEGACY_OCTAL;
      }
      segment = lx.pos;
    } else {
      lx.pos = lx.pos + 1;
    }
  }
  let aux = -1;
  if (cooked !== null) {
    aux = pushValue(lx.tokens, cooked);
  }
  return emit(lx, TK_STRING, start, lx.pos, f, aux);
}

// Raw template text of src[start, end): CRLF and CR read as LF.
function appendRaw(out, src, start, end) {
  let i = start;
  while (i < end) {
    const c = src[i];
    if (c === CP_CR) {
      if (i + 1 < end && src[i + 1] === CP_LF) {
        i = i + 1;
      }
      out.push(CP_LF);
    } else {
      out.push(c);
    }
    i = i + 1;
  }
}

// Template characters from lx.pos (after a backtick or `}`) up to `${` or
// the closing backtick. `start` is where the token starts.
function readTemplate(lx, start, flags) {
  const src = lx.src;
  const n = lx.len;
  let f = flags;
  let cooked = null;
  let raw = null;
  let segment = lx.pos;
  let badAt = -1;
  while (true) {
    if (lx.pos >= n) {
      return fail(lx, E_UNTERMINATED_TEMPLATE, start, 0);
    }
    const c = src[lx.pos];
    if (c === 0x60) {
      if (cooked !== null) {
        appendRange(cooked, src, segment, lx.pos);
        appendRange(raw, src, segment, lx.pos);
      }
      lx.pos = lx.pos + 1;
      f = f | F_TAIL;
      break;
    }
    if (c === 0x24 && unitAt(lx, lx.pos + 1) === 0x7b) {
      if (cooked !== null) {
        appendRange(cooked, src, segment, lx.pos);
        appendRange(raw, src, segment, lx.pos);
      }
      lx.pos = lx.pos + 2;
      break;
    }
    if (c === 0x5c || c === CP_CR) {
      if (cooked === null) {
        cooked = [];
        raw = [];
      }
      appendRange(cooked, src, segment, lx.pos);
      appendRange(raw, src, segment, lx.pos);
      const piece = lx.pos;
      if (c === CP_CR) {
        lx.pos = lx.pos + 1;
        if (unitAt(lx, lx.pos) === CP_LF) {
          lx.pos = lx.pos + 1;
        }
        cooked.push(CP_LF);
        raw.push(CP_LF);
      } else {
        const k = readEscape(lx, cooked, true, start);
        if (k === ESC_FAILED) {
          return -1;
        }
        if (k === ESC_INVALID && badAt < 0) {
          f = f | F_BAD_ESCAPE;
          badAt = piece;
        }
        appendRaw(raw, src, piece, lx.pos);
      }
      segment = lx.pos;
    } else {
      lx.pos = lx.pos + 1;
    }
  }
  let aux = -1;
  if (cooked !== null) {
    if ((f & F_BAD_ESCAPE) !== 0) {
      aux = pushValue(lx.tokens, null);
    } else {
      aux = pushValue(lx.tokens, cooked);
    }
    pushValue(lx.tokens, raw);
    if (badAt >= 0) {
      pushValue(lx.tokens, [badAt]);
    }
  }
  return emit(lx, TK_TEMPLATE, start, lx.pos, f, aux);
}

// ---- punctuators ----

// The unit after the current one equals c: consume it.
function eat(lx, c) {
  if (lx.pos < lx.len && lx.src[lx.pos] === c) {
    lx.pos = lx.pos + 1;
    return true;
  }
  return false;
}

// Punctuators that are always one character, by code unit (-1: not one).
const SINGLE = [];
let single = 0;
while (single < 128) {
  SINGLE.push(-1);
  single = single + 1;
}
SINGLE[0x7b] = P_LBRACE;
SINGLE[0x7d] = P_RBRACE;
SINGLE[0x28] = P_LPAREN;
SINGLE[0x29] = P_RPAREN;
SINGLE[0x5b] = P_LBRACKET;
SINGLE[0x5d] = P_RBRACKET;
SINGLE[0x3b] = P_SEMICOLON;
SINGLE[0x2c] = P_COMMA;
SINGLE[0x3a] = P_COLON;
SINGLE[0x7e] = P_TILDE;

function readPunct(lx, start, flags) {
  const c = lx.src[start];
  lx.pos = start + 1;
  let p = SINGLE[c];
  if (p >= 0) {
    return emit(lx, TK_PUNCT, start, lx.pos, flags, p);
  }
  if (c === 0x2e) {
    if (unitAt(lx, lx.pos) === 0x2e && unitAt(lx, lx.pos + 1) === 0x2e) {
      lx.pos = lx.pos + 2;
      p = P_ELLIPSIS;
    } else {
      p = P_DOT;
    }
  } else if (c === 0x3c) {
    if (eat(lx, 0x3c)) {
      p = P_SHL;
      if (eat(lx, 0x3d)) {
        p = P_SHL_EQ;
      }
    } else if (eat(lx, 0x3d)) {
      p = P_LT_EQ;
    } else {
      p = P_LT;
    }
  } else if (c === 0x3e) {
    if (eat(lx, 0x3e)) {
      if (eat(lx, 0x3e)) {
        p = P_USHR;
        if (eat(lx, 0x3d)) {
          p = P_USHR_EQ;
        }
      } else if (eat(lx, 0x3d)) {
        p = P_SHR_EQ;
      } else {
        p = P_SHR;
      }
    } else if (eat(lx, 0x3d)) {
      p = P_GT_EQ;
    } else {
      p = P_GT;
    }
  } else if (c === 0x3d) {
    if (eat(lx, 0x3d)) {
      p = P_EQ_EQ;
      if (eat(lx, 0x3d)) {
        p = P_EQ_EQ_EQ;
      }
    } else if (eat(lx, 0x3e)) {
      p = P_ARROW;
    } else {
      p = P_EQ;
    }
  } else if (c === 0x21) {
    if (eat(lx, 0x3d)) {
      p = P_NOT_EQ;
      if (eat(lx, 0x3d)) {
        p = P_NOT_EQ_EQ;
      }
    } else {
      p = P_BANG;
    }
  } else if (c === 0x2b) {
    if (eat(lx, 0x2b)) {
      p = P_PLUS_PLUS;
    } else if (eat(lx, 0x3d)) {
      p = P_PLUS_EQ;
    } else {
      p = P_PLUS;
    }
  } else if (c === 0x2d) {
    if (eat(lx, 0x2d)) {
      p = P_MINUS_MINUS;
    } else if (eat(lx, 0x3d)) {
      p = P_MINUS_EQ;
    } else {
      p = P_MINUS;
    }
  } else if (c === 0x2a) {
    if (eat(lx, 0x2a)) {
      p = P_STAR_STAR;
      if (eat(lx, 0x3d)) {
        p = P_STAR_STAR_EQ;
      }
    } else if (eat(lx, 0x3d)) {
      p = P_STAR_EQ;
    } else {
      p = P_STAR;
    }
  } else if (c === 0x25) {
    p = P_PERCENT;
    if (eat(lx, 0x3d)) {
      p = P_PERCENT_EQ;
    }
  } else if (c === 0x26) {
    if (eat(lx, 0x26)) {
      p = P_AMP_AMP;
      if (eat(lx, 0x3d)) {
        p = P_AMP_AMP_EQ;
      }
    } else if (eat(lx, 0x3d)) {
      p = P_AMP_EQ;
    } else {
      p = P_AMP;
    }
  } else if (c === 0x7c) {
    if (eat(lx, 0x7c)) {
      p = P_PIPE_PIPE;
      if (eat(lx, 0x3d)) {
        p = P_PIPE_PIPE_EQ;
      }
    } else if (eat(lx, 0x3d)) {
      p = P_PIPE_EQ;
    } else {
      p = P_PIPE;
    }
  } else if (c === 0x5e) {
    p = P_CARET;
    if (eat(lx, 0x3d)) {
      p = P_CARET_EQ;
    }
  } else if (c === 0x3f) {
    if (eat(lx, 0x3f)) {
      p = P_QUESTION_QUESTION;
      if (eat(lx, 0x3d)) {
        p = P_QUESTION_QUESTION_EQ;
      }
    } else if (unitAt(lx, lx.pos) === 0x2e && !isDigit(unitAt(lx, lx.pos + 1))) {
      // `?.` but not `? .5` (spec 12.8: OptionalChainingPunctuator).
      lx.pos = lx.pos + 1;
      p = P_QUESTION_DOT;
    } else {
      p = P_QUESTION;
    }
  } else if (c === 0x2f) {
    p = P_SLASH;
    if (eat(lx, 0x3d)) {
      p = P_SLASH_EQ;
    }
  }
  if (p < 0) {
    if (c === 0) {
      return fail(lx, E_NUL, start, 0);
    }
    return fail(lx, E_CHAR, start, c);
  }
  return emit(lx, TK_PUNCT, start, lx.pos, flags, p);
}

// ---- rescans ----

// Token i (the last one, a `/` or `/=`) read again as a regular
// expression literal. Returns the new token's index, or -1.
export function rescanRegex(lx, i) {
  const t = lx.tokens;
  const start = t.start[i];
  const flags = t.flags[i] & F_NL;
  truncateTokens(t, i);
  const src = lx.src;
  const n = lx.len;
  lx.pos = start + 1;
  let inClass = false;
  while (true) {
    if (lx.pos >= n) {
      return fail(lx, E_UNTERMINATED_REGEX, start, 0);
    }
    const c = src[lx.pos];
    if (c === CP_LF || c === CP_CR || c === CP_LS || c === CP_PS) {
      return fail(lx, E_UNTERMINATED_REGEX, start, 0);
    }
    if (c === 0x5c) {
      lx.pos = lx.pos + 1;
      const d = unitAt(lx, lx.pos);
      if (d < 0 || d === CP_LF || d === CP_CR || d === CP_LS || d === CP_PS) {
        return fail(lx, E_UNTERMINATED_REGEX, start, 0);
      }
      lx.pos = lx.pos + 1;
    } else if (c === 0x5b) {
      inClass = true;
      lx.pos = lx.pos + 1;
    } else if (c === 0x5d) {
      inClass = false;
      lx.pos = lx.pos + 1;
    } else if (c === 0x2f && !inClass) {
      lx.pos = lx.pos + 1;
      break;
    } else {
      lx.pos = lx.pos + 1;
    }
  }
  const flagsStart = lx.pos;
  // Flags: IdentifierPartChars; only d g i m s u v y, once each, and not
  // u with v (spec 22.2.3.1 early errors).
  let seen = 0;
  while (lx.pos < n) {
    const cp = codePointAt(src, lx.pos);
    if (cp === 0x5c) {
      return fail(lx, E_FLAG_ESCAPE, start, 0);
    }
    if (!isIdContinue(cp)) {
      break;
    }
    lx.pos = lx.pos + codeUnitCount(cp);
    const bit = flagBit(cp);
    if (bit === 0) {
      return fail(lx, E_FLAG_UNKNOWN, start, cp);
    }
    if ((seen & bit) !== 0) {
      return fail(lx, E_FLAG_DUPLICATE, start, cp);
    }
    seen = seen | bit;
  }
  if ((seen & 32) !== 0 && (seen & 64) !== 0) {
    return fail(lx, E_FLAGS_U_AND_V, start, 0);
  }
  return emit(lx, TK_REGEX, start, lx.pos, flags, flagsStart);
}

function flagBit(cp) {
  if (cp === 0x64) {
    return 1;
  }
  if (cp === 0x67) {
    return 2;
  }
  if (cp === 0x69) {
    return 4;
  }
  if (cp === 0x6d) {
    return 8;
  }
  if (cp === 0x73) {
    return 16;
  }
  if (cp === 0x75) {
    return 32;
  }
  if (cp === 0x76) {
    return 64;
  }
  if (cp === 0x79) {
    return 128;
  }
  return 0;
}

// Token i (the last one, a `}`) read again as the next piece of a
// template. Returns the new token's index, or -1.
export function rescanTemplate(lx, i) {
  const t = lx.tokens;
  const start = t.start[i];
  const flags = t.flags[i] & F_NL;
  truncateTokens(t, i);
  lx.pos = start + 1;
  return readTemplate(lx, start, flags);
}

// Go back to just after token count-1, dropping later tokens, so lexing
// can resume from there (for parser lookahead).
export function rewind(lx, count) {
  truncateTokens(lx.tokens, count);
  if (count === 0) {
    lx.pos = 0;
  } else {
    lx.pos = lx.tokens.end[count - 1];
  }
}

// Lex everything with no parser: no regex rescans, no template
// continuations. Returns the token count, or -1 on error. For tests and
// timing; the parser calls nextToken itself.
export function lexAll(lx) {
  while (true) {
    const i = nextToken(lx);
    if (i < 0) {
      return -1;
    }
    if (lx.tokens.kind[i] === TK_EOF) {
      return i + 1;
    }
  }
}
