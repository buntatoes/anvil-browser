// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The token dump: one line per token in the format of the reference's
// `refdump --tokens` (tools/refdump/FORMAT.md), with the same replay of
// the parser-driven rescans, so the two can be compared line by line.
// Used by the token differential and by the unit tests; not by the
// parser, which drives the rescans itself.
//
// Offsets are written as UTF-8 byte offsets when a byte table (from
// byteOffsets) is given, as the reference does, and as code-unit offsets
// when it is null. Line and column are the same in both.

import { codePointAt, codeUnitCount, utf8Length } from "../core/text.js";
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
  P_LBRACE,
  P_RBRACE,
  P_LPAREN,
  P_RPAREN,
  P_RBRACKET,
  P_SLASH,
  P_SLASH_EQ,
  PUNCT_TEXT,
  tokenName,
  tokenIsWord,
  stringValue,
  templateCooked,
  templateRaw,
  regexPattern,
  regexFlags,
  numberValue,
  bigintRadix,
  bigintDigits,
} from "./tokens.js";
import { nextToken, rescanRegex, rescanTemplate } from "./lexer.js";
import { lexErrorMessage } from "./lex_messages.js";
import { position } from "../core/positions.js";

const HEX = "0123456789abcdef";

// For each code-unit offset 0..n, the UTF-8 byte offset. The low half of
// a surrogate pair gets the offset of the pair's end; no token starts or
// ends there.
export function byteOffsets(units) {
  const out = [];
  let bytes = 0;
  let i = 0;
  while (i < units.length) {
    const cp = codePointAt(units, i);
    const w = codeUnitCount(cp);
    out.push(bytes);
    bytes = bytes + utf8Length(cp);
    if (w === 2) {
      out.push(bytes);
    }
    i = i + w;
  }
  out.push(bytes);
  return out;
}

// Code units quoted like the reference: printable ASCII as is (with `\\`
// and `\"`), `\n`, `\r`, `\t`, everything else `\uXXXX`.
export function quoteUnits(units) {
  let s = '"';
  for (const u of units) {
    if (u === 0x5c) {
      s += "\\\\";
    } else if (u === 0x22) {
      s += '\\"';
    } else if (u === 0x0a) {
      s += "\\n";
    } else if (u === 0x0d) {
      s += "\\r";
    } else if (u === 0x09) {
      s += "\\t";
    } else if (u >= 0x20 && u <= 0x7e) {
      s += String.fromCharCode(u);
    } else {
      s += "\\u";
      let k = 12;
      while (k >= 0) {
        const d = (u >>> k) & 15;
        s += HEX.slice(d, d + 1);
        k -= 4;
      }
    }
  }
  return s + '"';
}

export function numberText(x) {
  if (x === 0 && 1 / x < 0) {
    return "-0";
  }
  return `${x}`;
}

function unitsText(units) {
  let s = "";
  for (const u of units) {
    s += String.fromCharCode(u);
  }
  return s;
}

function offsetText(bytes, i) {
  if (bytes === null) {
    return `${i}`;
  }
  return `${bytes[i]}`;
}

// The dump line of token i.
export function tokenLine(lx, i, bytes) {
  const t = lx.tokens;
  const src = lx.src;
  const k = t.kind[i];
  const f = t.flags[i];
  let s = `${offsetText(bytes, t.start[i])} ${offsetText(bytes, t.end[i])} ${t.line[i]}:${t.column[i]} `;
  if (k === TK_IDENT) {
    s += `ident ${quoteUnits(tokenName(t, src, i))}`;
  } else if (k === TK_PRIVATE) {
    s += `private ${quoteUnits(tokenName(t, src, i))}`;
  } else if (k === TK_PUNCT) {
    s += `punct ${quoteUnits(PUNCT_TEXT[t.aux[i]])}`;
  } else if (k === TK_NUMBER) {
    s += `number ${numberText(numberValue(t, i))}`;
  } else if (k === TK_BIGINT) {
    s += `bigint ${bigintRadix(t, i)}:${unitsText(bigintDigits(t, i))}`;
  } else if (k === TK_STRING) {
    s += `string ${quoteUnits(stringValue(t, src, i))}`;
  } else if (k === TK_TEMPLATE) {
    let cooked = "None";
    if ((f & F_BAD_ESCAPE) === 0) {
      cooked = quoteUnits(templateCooked(t, src, i));
    }
    let tail = "false";
    if ((f & F_TAIL) !== 0) {
      tail = "true";
    }
    s += `template cooked=${cooked} raw=${quoteUnits(templateRaw(t, src, i))} tail=${tail}`;
  } else if (k === TK_REGEX) {
    s += `regex ${quoteUnits(regexPattern(t, src, i))} ${quoteUnits(regexFlags(t, src, i))}`;
  } else {
    s += "eof";
  }
  if ((f & F_NL) !== 0) {
    s += " nl";
  }
  if ((f & F_ESCAPED) !== 0) {
    s += " escaped";
  }
  if ((f & F_LEGACY_OCTAL) !== 0) {
    s += " legacy-octal";
  }
  return s;
}

// Words whose `(` opens a statement head, and words after which `/`
// starts a regex.
const HEAD_WORDS = ["if", "while", "for", "with"];
const REGEX_WORDS = [
  "return",
  "typeof",
  "instanceof",
  "in",
  "of",
  "new",
  "delete",
  "void",
  "throw",
  "case",
  "do",
  "else",
  "yield",
  "await",
];

function wordUnits(w) {
  const out = [];
  let i = 0;
  while (i < w.length) {
    out.push(w.charCodeAt(i));
    i += 1;
  }
  return out;
}

const HEAD_UNITS = [];
for (const w of HEAD_WORDS) {
  HEAD_UNITS.push(wordUnits(w));
}
const REGEX_UNITS = [];
for (const w of REGEX_WORDS) {
  REGEX_UNITS.push(wordUnits(w));
}

function isAnyWord(lx, i, table) {
  for (const w of table) {
    if (tokenIsWord(lx.tokens, lx.src, i, w)) {
      return true;
    }
  }
  return false;
}

// The previous-token rule of FORMAT.md: may a `/` after token prev (-1
// at the start) begin a regex?
function regexAllowed(lx, prev, afterHead) {
  if (prev < 0 || afterHead) {
    return true;
  }
  const t = lx.tokens;
  const k = t.kind[prev];
  if (k === TK_NUMBER || k === TK_BIGINT || k === TK_STRING || k === TK_REGEX || k === TK_PRIVATE) {
    return false;
  }
  if (k === TK_TEMPLATE) {
    return (t.flags[prev] & F_TAIL) === 0;
  }
  if (k === TK_IDENT) {
    return isAnyWord(lx, prev, REGEX_UNITS);
  }
  if (k === TK_PUNCT) {
    const p = t.aux[prev];
    return p !== P_RPAREN && p !== P_RBRACKET && p !== P_RBRACE;
  }
  return true;
}

// Lex all of lx, replaying the rescans like the reference dump: regex
// starts by the previous-token rule of FORMAT.md, template continuations
// exactly with a stack of open braces. Calls visit(i) for each token when
// visit is not null. Returns the token count, or -1 on a lexing error.
export function lexWithReplay(lx, visit) {
  const t = lx.tokens;
  const braces = [];
  const parens = [];
  let prev = -1;
  let afterHead = false;
  let count = 0;
  while (true) {
    let i = nextToken(lx);
    if (i < 0) {
      return -1;
    }
    const k = t.kind[i];
    let p = -1;
    if (k === TK_PUNCT) {
      p = t.aux[i];
    }
    let closesHead = false;
    if (p === P_RPAREN && parens.length > 0) {
      closesHead = parens.pop();
    }
    let closesSubst = false;
    if (p === P_RBRACE && braces.length > 0) {
      closesSubst = braces.pop();
    }
    if ((p === P_SLASH || p === P_SLASH_EQ) && regexAllowed(lx, prev, afterHead)) {
      i = rescanRegex(lx, i);
    } else if (p === P_LPAREN) {
      parens.push(prev >= 0 && isAnyWord(lx, prev, HEAD_UNITS));
    } else if (p === P_LBRACE) {
      braces.push(false);
    } else if (p === P_RBRACE && closesSubst) {
      i = rescanTemplate(lx, i);
    }
    if (i < 0) {
      return -1;
    }
    if (t.kind[i] === TK_TEMPLATE && (t.flags[i] & F_TAIL) === 0) {
      braces.push(true);
    }
    if (visit !== null) {
      visit(i);
    }
    count += 1;
    if (t.kind[i] === TK_EOF) {
      return count;
    }
    afterHead = closesHead;
    prev = i;
  }
}

// The dump: out(line) for each token line, and on a lexing error a last
// line `ERROR line:col: message`. Returns the number of token lines.
export function dumpTokens(lx, bytes, out) {
  const n = lexWithReplay(lx, (i) => out(tokenLine(lx, i, bytes)));
  if (n < 0) {
    const at = position(lx.starts, lx.errorStart);
    out(`ERROR ${at.line}:${at.column}: ${lexErrorMessage(lx)}`);
    return lx.tokens.kind.length;
  }
  return n;
}

// Units of a string.
export function unitsOf(s) {
  const out = [];
  let i = 0;
  while (i < s.length) {
    out.push(s.charCodeAt(i));
    i += 1;
  }
  return out;
}
