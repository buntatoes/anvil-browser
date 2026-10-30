// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Lines and columns. A line ends at LF, CR, CRLF (counted once), LS or
// PS. Lines and columns are 1-based and columns count UTF-16 code units,
// as in the reference front end. Offsets here are code-unit offsets (the
// reference uses byte offsets; see utf8Offset in text.js).

import { CP_LF, CP_CR, CP_LS, CP_PS } from "./text.js";

// Offsets where each line starts. The first entry is always 0.
export function lineStarts(units) {
  const starts = [0];
  const n = units.length;
  let i = 0;
  while (i < n) {
    const c = units[i];
    if (c === CP_CR) {
      if (i + 1 < n && units[i + 1] === CP_LF) {
        i = i + 1;
      }
      starts.push(i + 1);
    } else if (c === CP_LF || c === CP_LS || c === CP_PS) {
      starts.push(i + 1);
    }
    i = i + 1;
  }
  return starts;
}

// Index of the line holding offset (0-based), by binary search in the
// table from lineStarts.
export function lineIndex(starts, offset) {
  let lo = 0;
  let hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (starts[mid] <= offset) {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  return lo;
}

// 1-based line of offset.
export function lineOf(starts, offset) {
  return lineIndex(starts, offset) + 1;
}

// 1-based column of offset, in code units.
export function columnOf(starts, offset) {
  return offset - starts[lineIndex(starts, offset)] + 1;
}

// { line, column } of offset.
export function position(starts, offset) {
  const i = lineIndex(starts, offset);
  return { line: i + 1, column: offset - starts[i] + 1 };
}

// Offset of a 1-based line and column, or -1 when the line does not exist.
export function offsetOf(starts, line, column) {
  if (line < 1 || line > starts.length) {
    return -1;
  }
  return starts[line - 1] + column - 1;
}
