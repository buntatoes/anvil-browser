// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Line-start tables and offset to line and column, checked by hand and
// against a plain scan on many generated inputs.
// expect: fixed cases ok
// expect: generated cases ok: 200 inputs

import { lineStarts, lineOf, columnOf, position, offsetOf } from "../../src/core/positions.js";

let failures = 0;

function check(what, got, want) {
  if (got !== want) {
    failures++;
    if (failures < 20) {
      console.log(`FAIL ${what}: got ${got}, want ${want}`);
    }
  }
}

// "a\nb\r\nc\rd\u2028e\u2029f"
const src = [0x61, 0x0a, 0x62, 0x0d, 0x0a, 0x63, 0x0d, 0x64, 0x2028, 0x65, 0x2029, 0x66];
const starts = lineStarts(src);
check("line count", starts.length, 6);
check("start 1", starts[1], 2);
check("crlf once", starts[2], 5);
check("cr", starts[3], 7);
check("ls", starts[4], 9);
check("ps", starts[5], 11);
check("a", lineOf(starts, 0), 1);
check("lf itself", lineOf(starts, 1), 1);
check("b", lineOf(starts, 2), 2);
check("cr of crlf", columnOf(starts, 3), 2);
check("lf of crlf", columnOf(starts, 4), 3);
check("lf of crlf line", lineOf(starts, 4), 2);
check("f", lineOf(starts, 11), 6);
check("f col", columnOf(starts, 11), 1);
check("end", lineOf(starts, 12), 6);
const p = position(starts, 5);
check("pos line", p.line, 3);
check("pos col", p.column, 1);
check("offsetOf", offsetOf(starts, 3, 2), 6);
check("offsetOf bad", offsetOf(starts, 7, 1), -1);
check("empty", lineStarts([]).length, 1);
check("only crlf", lineStarts([0x0d, 0x0a]).length, 2);
check("cr cr", lineStarts([0x0d, 0x0d]).length, 3);
if (failures === 0) {
  console.log("fixed cases ok");
}

// xorshift32: the same sequence on every run.
let seed = 2463534242;
function next() {
  seed = seed ^ (seed << 13);
  seed = seed ^ (seed >>> 17);
  seed = seed ^ (seed << 5);
  seed = seed >>> 0;
  return seed;
}

const alphabet = [0x61, 0x62, 0x0a, 0x0d, 0x2028, 0x2029, 0x20, 0xd83d, 0xde00];
let inputs = 0;
for (let round = 0; round < 200; round++) {
  const n = next() % 40;
  const units = [];
  for (let i = 0; i < n; i++) {
    units.push(alphabet[next() % alphabet.length]);
  }
  const table = lineStarts(units);
  // Plain scan: the line and column of every offset, end included.
  let line = 1;
  let col = 1;
  for (let i = 0; i <= n; i++) {
    check(`line r${round} i${i}`, lineOf(table, i), line);
    check(`col r${round} i${i}`, columnOf(table, i), col);
    if (i < n) {
      const c = units[i];
      const crlf = c === 0x0d && i + 1 < n && units[i + 1] === 0x0a;
      if (c === 0x0a || c === 0x2028 || c === 0x2029 || (c === 0x0d && !crlf)) {
        line++;
        col = 1;
      } else {
        col++;
      }
    }
  }
  inputs++;
}
if (failures === 0) {
  console.log(`generated cases ok: ${inputs} inputs`);
}
