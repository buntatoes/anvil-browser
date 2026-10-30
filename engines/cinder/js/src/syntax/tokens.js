// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Tokens: kinds, flags, punctuator ids, word tables, and the token store.
//
// The store keeps one entry per token in parallel plain arrays (kind,
// start, end, line, column, flags, aux), so lexing makes no object per
// token. Offsets are code-unit offsets into the source array; line and
// column are 1-based, columns in code units. `aux` depends on the kind:
//
//   ident, private  -1 when the name is the source text (for a private
//                   name, after the `#`); else an index into `values`
//                   holding the decoded name (written with escapes)
//   punct           the punctuator id (P_*)
//   number          an index into `numbers`
//   bigint          an index into `values`: the digits in the literal's
//                   radix, lowercase, leading zeros dropped ("0" for zero);
//                   the radix is in the flags
//   string          -1 when the value is the source text between the
//                   quotes; else an index into `values` (cooked units)
//   template        -1 when cooked and raw are both the source text of the
//                   piece; else an index into `values`: cooked units (null
//                   for a piece with an escape only tagged templates
//                   allow) and, at the next index, raw units
//   regex           the offset where the flags start; the pattern is the
//                   source between the slashes
//
// `values` holds arrays of UTF-16 code units, `numbers` holds numbers.
// Words are not told apart by the lexer: keywords come out as `ident`,
// and the tables below are for the parser.

export const TK_EOF = 0;
export const TK_IDENT = 1;
export const TK_PRIVATE = 2;
export const TK_PUNCT = 3;
export const TK_NUMBER = 4;
export const TK_BIGINT = 5;
export const TK_STRING = 6;
export const TK_TEMPLATE = 7;
export const TK_REGEX = 8;

// A line terminator came before the token.
export const F_NL = 1;
// An identifier or private name written with a `\u` escape.
export const F_ESCAPED = 2;
// A number or string written in a legacy octal form (`017`, `08`, `"\1"`,
// `"\8"`); strict code rejects these.
export const F_LEGACY_OCTAL = 4;
// A template piece that ends the template (with a backtick).
export const F_TAIL = 8;
// A template piece with an escape that only tagged templates allow.
export const F_BAD_ESCAPE = 16;
// BigInt radix (none of these: radix 10).
export const F_RADIX_2 = 32;
export const F_RADIX_8 = 64;
export const F_RADIX_16 = 128;

export const P_LBRACE = 0; // {
export const P_RBRACE = 1; // }
export const P_LPAREN = 2; // (
export const P_RPAREN = 3; // )
export const P_LBRACKET = 4; // [
export const P_RBRACKET = 5; // ]
export const P_DOT = 6; // .
export const P_ELLIPSIS = 7; // ...
export const P_SEMICOLON = 8; // ;
export const P_COMMA = 9; // ,
export const P_LT = 10; // <
export const P_GT = 11; // >
export const P_LT_EQ = 12; // <=
export const P_GT_EQ = 13; // >=
export const P_EQ_EQ = 14; // ==
export const P_NOT_EQ = 15; // !=
export const P_EQ_EQ_EQ = 16; // ===
export const P_NOT_EQ_EQ = 17; // !==
export const P_PLUS = 18; // +
export const P_MINUS = 19; // -
export const P_STAR = 20; // *
export const P_PERCENT = 21; // %
export const P_STAR_STAR = 22; // **
export const P_PLUS_PLUS = 23; // ++
export const P_MINUS_MINUS = 24; // --
export const P_SHL = 25; // <<
export const P_SHR = 26; // >>
export const P_USHR = 27; // >>>
export const P_AMP = 28; // &
export const P_PIPE = 29; // |
export const P_CARET = 30; // ^
export const P_BANG = 31; // !
export const P_TILDE = 32; // ~
export const P_AMP_AMP = 33; // &&
export const P_PIPE_PIPE = 34; // ||
export const P_QUESTION_QUESTION = 35; // ??
export const P_QUESTION = 36; // ?
export const P_QUESTION_DOT = 37; // ?.
export const P_COLON = 38; // :
export const P_EQ = 39; // =
export const P_PLUS_EQ = 40; // +=
export const P_MINUS_EQ = 41; // -=
export const P_STAR_EQ = 42; // *=
export const P_PERCENT_EQ = 43; // %=
export const P_STAR_STAR_EQ = 44; // **=
export const P_SHL_EQ = 45; // <<=
export const P_SHR_EQ = 46; // >>=
export const P_USHR_EQ = 47; // >>>=
export const P_AMP_EQ = 48; // &=
export const P_PIPE_EQ = 49; // |=
export const P_CARET_EQ = 50; // ^=
export const P_AMP_AMP_EQ = 51; // &&=
export const P_PIPE_PIPE_EQ = 52; // ||=
export const P_QUESTION_QUESTION_EQ = 53; // ??=
export const P_ARROW = 54; // =>
export const P_SLASH = 55; // /
export const P_SLASH_EQ = 56; // /=

export const PUNCT_COUNT = 57;

// Text of each punctuator, by id.
export const PUNCT_TEXT = [
  [0x7b], // {
  [0x7d], // }
  [0x28], // (
  [0x29], // )
  [0x5b], // [
  [0x5d], // ]
  [0x2e], // .
  [0x2e, 0x2e, 0x2e], // ...
  [0x3b], // ;
  [0x2c], // ,
  [0x3c], // <
  [0x3e], // >
  [0x3c, 0x3d], // <=
  [0x3e, 0x3d], // >=
  [0x3d, 0x3d], // ==
  [0x21, 0x3d], // !=
  [0x3d, 0x3d, 0x3d], // ===
  [0x21, 0x3d, 0x3d], // !==
  [0x2b], // +
  [0x2d], // -
  [0x2a], // *
  [0x25], // %
  [0x2a, 0x2a], // **
  [0x2b, 0x2b], // ++
  [0x2d, 0x2d], // --
  [0x3c, 0x3c], // <<
  [0x3e, 0x3e], // >>
  [0x3e, 0x3e, 0x3e], // >>>
  [0x26], // &
  [0x7c], // |
  [0x5e], // ^
  [0x21], // !
  [0x7e], // ~
  [0x26, 0x26], // &&
  [0x7c, 0x7c], // ||
  [0x3f, 0x3f], // ??
  [0x3f], // ?
  [0x3f, 0x2e], // ?.
  [0x3a], // :
  [0x3d], // =
  [0x2b, 0x3d], // +=
  [0x2d, 0x3d], // -=
  [0x2a, 0x3d], // *=
  [0x25, 0x3d], // %=
  [0x2a, 0x2a, 0x3d], // **=
  [0x3c, 0x3c, 0x3d], // <<=
  [0x3e, 0x3e, 0x3d], // >>=
  [0x3e, 0x3e, 0x3e, 0x3d], // >>>=
  [0x26, 0x3d], // &=
  [0x7c, 0x3d], // |=
  [0x5e, 0x3d], // ^=
  [0x26, 0x26, 0x3d], // &&=
  [0x7c, 0x7c, 0x3d], // ||=
  [0x3f, 0x3f, 0x3d], // ??=
  [0x3d, 0x3e], // =>
  [0x2f], // /
  [0x2f, 0x3d], // /=
];

// Words that are never identifiers. `await` and `yield` depend on
// context (module code, generators, async functions); the parser decides.
export const RESERVED_WORDS = [
  [0x61, 0x77, 0x61, 0x69, 0x74], // await
  [0x62, 0x72, 0x65, 0x61, 0x6b], // break
  [0x63, 0x61, 0x73, 0x65], // case
  [0x63, 0x61, 0x74, 0x63, 0x68], // catch
  [0x63, 0x6c, 0x61, 0x73, 0x73], // class
  [0x63, 0x6f, 0x6e, 0x73, 0x74], // const
  [0x63, 0x6f, 0x6e, 0x74, 0x69, 0x6e, 0x75, 0x65], // continue
  [0x64, 0x65, 0x62, 0x75, 0x67, 0x67, 0x65, 0x72], // debugger
  [0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74], // default
  [0x64, 0x65, 0x6c, 0x65, 0x74, 0x65], // delete
  [0x64, 0x6f], // do
  [0x65, 0x6c, 0x73, 0x65], // else
  [0x65, 0x6e, 0x75, 0x6d], // enum
  [0x65, 0x78, 0x70, 0x6f, 0x72, 0x74], // export
  [0x65, 0x78, 0x74, 0x65, 0x6e, 0x64, 0x73], // extends
  [0x66, 0x61, 0x6c, 0x73, 0x65], // false
  [0x66, 0x69, 0x6e, 0x61, 0x6c, 0x6c, 0x79], // finally
  [0x66, 0x6f, 0x72], // for
  [0x66, 0x75, 0x6e, 0x63, 0x74, 0x69, 0x6f, 0x6e], // function
  [0x69, 0x66], // if
  [0x69, 0x6d, 0x70, 0x6f, 0x72, 0x74], // import
  [0x69, 0x6e], // in
  [0x69, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x6f, 0x66], // instanceof
  [0x6e, 0x65, 0x77], // new
  [0x6e, 0x75, 0x6c, 0x6c], // null
  [0x72, 0x65, 0x74, 0x75, 0x72, 0x6e], // return
  [0x73, 0x75, 0x70, 0x65, 0x72], // super
  [0x73, 0x77, 0x69, 0x74, 0x63, 0x68], // switch
  [0x74, 0x68, 0x69, 0x73], // this
  [0x74, 0x68, 0x72, 0x6f, 0x77], // throw
  [0x74, 0x72, 0x75, 0x65], // true
  [0x74, 0x72, 0x79], // try
  [0x74, 0x79, 0x70, 0x65, 0x6f, 0x66], // typeof
  [0x76, 0x61, 0x72], // var
  [0x76, 0x6f, 0x69, 0x64], // void
  [0x77, 0x68, 0x69, 0x6c, 0x65], // while
  [0x77, 0x69, 0x74, 0x68], // with
  [0x79, 0x69, 0x65, 0x6c, 0x64], // yield
];

// Words reserved in strict code only.
export const STRICT_RESERVED_WORDS = [
  [0x69, 0x6d, 0x70, 0x6c, 0x65, 0x6d, 0x65, 0x6e, 0x74, 0x73], // implements
  [0x69, 0x6e, 0x74, 0x65, 0x72, 0x66, 0x61, 0x63, 0x65], // interface
  [0x6c, 0x65, 0x74], // let
  [0x70, 0x61, 0x63, 0x6b, 0x61, 0x67, 0x65], // package
  [0x70, 0x72, 0x69, 0x76, 0x61, 0x74, 0x65], // private
  [0x70, 0x72, 0x6f, 0x74, 0x65, 0x63, 0x74, 0x65, 0x64], // protected
  [0x70, 0x75, 0x62, 0x6c, 0x69, 0x63], // public
  [0x73, 0x74, 0x61, 0x74, 0x69, 0x63], // static
  [0x79, 0x69, 0x65, 0x6c, 0x64], // yield
];

// Words with a meaning in some places that are plain names elsewhere.
export const CONTEXTUAL_WORDS = [
  [0x61, 0x73], // as
  [0x61, 0x73, 0x79, 0x6e, 0x63], // async
  [0x66, 0x72, 0x6f, 0x6d], // from
  [0x67, 0x65, 0x74], // get
  [0x6d, 0x65, 0x74, 0x61], // meta
  [0x6f, 0x66], // of
  [0x73, 0x65, 0x74], // set
  [0x74, 0x61, 0x72, 0x67, 0x65, 0x74], // target
];

// A new, empty token store.
export function newTokens() {
  return {
    kind: [],
    start: [],
    end: [],
    line: [],
    column: [],
    flags: [],
    aux: [],
    values: [],
    numbers: [],
  };
}

// Number of tokens in the store.
export function tokenCount(t) {
  return t.kind.length;
}

// Append a token; returns its index.
export function pushToken(t, kind, start, end, line, column, flags, aux) {
  t.kind.push(kind);
  t.start.push(start);
  t.end.push(end);
  t.line.push(line);
  t.column.push(column);
  t.flags.push(flags);
  t.aux.push(aux);
  return t.kind.length - 1;
}

// Drop tokens from the end until `n` are left. Side-table entries made
// for them are dropped too.
export function truncateTokens(t, n) {
  let values = t.values.length;
  let numbers = t.numbers.length;
  let i = t.kind.length - 1;
  while (i >= n) {
    const k = t.kind[i];
    const a = t.aux[i];
    if (a >= 0) {
      if (k === TK_NUMBER && a < numbers) {
        numbers = a;
      } else if (
        (k === TK_IDENT || k === TK_PRIVATE || k === TK_STRING || k === TK_BIGINT || k === TK_TEMPLATE) &&
        a < values
      ) {
        values = a;
      }
    }
    t.kind.pop();
    t.start.pop();
    t.end.pop();
    t.line.pop();
    t.column.pop();
    t.flags.pop();
    t.aux.pop();
    i = i - 1;
  }
  while (t.values.length > values) {
    t.values.pop();
  }
  while (t.numbers.length > numbers) {
    t.numbers.pop();
  }
}

// Store a value array; returns its index.
export function pushValue(t, units) {
  t.values.push(units);
  return t.values.length - 1;
}

// Store a number; returns its index.
export function pushNumber(t, x) {
  t.numbers.push(x);
  return t.numbers.length - 1;
}

// Copy of src[start, end).
export function sliceUnits(src, start, end) {
  const out = [];
  let i = start;
  while (i < end) {
    out.push(src[i]);
    i = i + 1;
  }
  return out;
}

// Whether src[start, end) holds exactly the units of `word`.
export function rangeIs(src, start, end, word) {
  if (end - start !== word.length) {
    return false;
  }
  let i = 0;
  while (i < word.length) {
    if (src[start + i] !== word[i]) {
      return false;
    }
    i = i + 1;
  }
  return true;
}

// Whether src[start, end) is one of the words in `table`.
export function rangeInTable(src, start, end, table) {
  let i = 0;
  while (i < table.length) {
    if (rangeIs(src, start, end, table[i])) {
      return true;
    }
    i = i + 1;
  }
  return false;
}

// The name of an ident or private token, decoded, as code units.
export function tokenName(t, src, i) {
  const a = t.aux[i];
  if (a >= 0) {
    return t.values[a];
  }
  if (t.kind[i] === TK_PRIVATE) {
    return sliceUnits(src, t.start[i] + 1, t.end[i]);
  }
  return sliceUnits(src, t.start[i], t.end[i]);
}

// Whether token i is the word `word` written without escapes, the only
// way a keyword counts as one.
export function tokenIsWord(t, src, i, word) {
  if (t.kind[i] !== TK_IDENT || (t.flags[i] & F_ESCAPED) !== 0) {
    return false;
  }
  return rangeIs(src, t.start[i], t.end[i], word);
}

// Whether an ident token names a word that is reserved everywhere.
// `await` and `yield` are left to the parser, as in the reference.
export function isAlwaysReserved(units) {
  if (rangeIs(units, 0, units.length, RESERVED_WORDS[0]) || rangeIs(units, 0, units.length, RESERVED_WORDS[37])) {
    return false;
  }
  return rangeInTable(units, 0, units.length, RESERVED_WORDS);
}

// Whether a name is reserved in strict code only (`yield` included).
export function isStrictReserved(units) {
  return rangeInTable(units, 0, units.length, STRICT_RESERVED_WORDS);
}

// Whether a name is a contextual word.
export function isContextual(units) {
  return rangeInTable(units, 0, units.length, CONTEXTUAL_WORDS);
}

// The punctuator id of token i, or -1 for another kind.
export function tokenPunct(t, i) {
  if (t.kind[i] !== TK_PUNCT) {
    return -1;
  }
  return t.aux[i];
}

// Cooked value of a string token, as code units.
export function stringValue(t, src, i) {
  const a = t.aux[i];
  if (a >= 0) {
    return t.values[a];
  }
  return sliceUnits(src, t.start[i] + 1, t.end[i] - 1);
}

// End of the text of a template piece (before the backtick or `${`).
function templateTextEnd(t, i) {
  if ((t.flags[i] & F_TAIL) !== 0) {
    return t.end[i] - 1;
  }
  return t.end[i] - 2;
}

// Cooked value of a template piece, or null when it has an escape only
// tagged templates allow.
export function templateCooked(t, src, i) {
  const a = t.aux[i];
  if (a >= 0) {
    return t.values[a];
  }
  return sliceUnits(src, t.start[i] + 1, templateTextEnd(t, i));
}

// Raw value of a template piece (CR and CRLF read as LF).
export function templateRaw(t, src, i) {
  const a = t.aux[i];
  if (a >= 0) {
    return t.values[a + 1];
  }
  return sliceUnits(src, t.start[i] + 1, templateTextEnd(t, i));
}

// Offset of the backslash of the first escape that only tagged
// templates allow, in a template piece whose cooked value is null.
export function templateBadEscape(t, i) {
  return t.values[t.aux[i] + 2][0];
}

// Pattern of a regex token, as code units.
export function regexPattern(t, src, i) {
  return sliceUnits(src, t.start[i] + 1, t.aux[i] - 1);
}

// Flags of a regex token, as code units.
export function regexFlags(t, src, i) {
  return sliceUnits(src, t.aux[i], t.end[i]);
}

// Value of a number token.
export function numberValue(t, i) {
  return t.numbers[t.aux[i]];
}

// Radix of a BigInt token.
export function bigintRadix(t, i) {
  const f = t.flags[i];
  if ((f & F_RADIX_2) !== 0) {
    return 2;
  }
  if ((f & F_RADIX_8) !== 0) {
    return 8;
  }
  if ((f & F_RADIX_16) !== 0) {
    return 16;
  }
  return 10;
}

// Digits of a BigInt token in its radix, as code units.
export function bigintDigits(t, i) {
  return t.values[t.aux[i]];
}
