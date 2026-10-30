// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Identifier code point scan for `bundle lexdiff --id-scan`: which code
// points the engine's lexer takes in a name, in the format of `refdump
// --id-scan` (lists start, continue, escaped-start, escaped-continue of
// inclusive hex ranges). The tool writes ./id_scan_input.js with STEP.
// needs: host

import { STEP } from "./id_scan_input.js";
import { createLexer, nextToken } from "../../src/syntax/lexer.js";
import { TK_IDENT, TK_EOF } from "../../src/syntax/tokens.js";
import { highSurrogateOf, lowSurrogateOf } from "../../src/core/text.js";

const HEX = "0123456789abcdef";

function hex(n) {
  if (n === 0) {
    return "0";
  }
  let s = "";
  let v = n;
  while (v > 0) {
    s = HEX[v % 16] + s;
    v = Math.floor(v / 16);
  }
  return s;
}

function oneIdent(units) {
  const lx = createLexer(units, false);
  const i = nextToken(lx);
  if (i < 0 || lx.tokens.kind[i] !== TK_IDENT || lx.tokens.end[i] !== units.length) {
    return false;
  }
  const j = nextToken(lx);
  return j >= 0 && lx.tokens.kind[j] === TK_EOF;
}

function pushCp(out, cp) {
  if (cp > 0xffff) {
    out.push(highSurrogateOf(cp));
    out.push(lowSurrogateOf(cp));
  } else {
    out.push(cp);
  }
}

function pushText(out, s) {
  let i = 0;
  while (i < s.length) {
    out.push(s.charCodeAt(i));
    i += 1;
  }
}

function make(list, cp) {
  const u = [];
  if (list === 1 || list === 3) {
    u.push(0x61);
  }
  if (list < 2) {
    if (cp >= 0xd800 && cp <= 0xdfff) {
      return null;
    }
    pushCp(u, cp);
  } else {
    pushText(u, `\\u{${hex(cp)}}`);
  }
  return u;
}

const NAMES = ["start", "continue", "escaped-start", "escaped-continue"];
let list = 0;
while (list < 4) {
  let first = -1;
  let last = -1;
  let cp = 0;
  while (cp <= 0x10ffff) {
    const u = make(list, cp);
    const ok = u !== null && oneIdent(u);
    if (ok) {
      if (first < 0) {
        first = cp;
      }
      last = cp;
    } else if (first >= 0) {
      console.log(`${NAMES[list]} ${hex(first)} ${hex(last)}`);
      first = -1;
    }
    cp += STEP;
  }
  if (first >= 0) {
    console.log(`${NAMES[list]} ${hex(first)} ${hex(last)}`);
  }
  list += 1;
}
