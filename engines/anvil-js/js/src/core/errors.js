// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Errors the engine reports about source text: what went wrong and where.
// An error is a plain object { name, message, line, column, offset, end }
// with 1-based line and column (code units) and code-unit offsets. Its
// text form is `line:column: message`, the same as the reference.

import { position } from "./positions.js";

export const SYNTAX_ERROR = "SyntaxError";
export const RANGE_ERROR = "RangeError";

// An error of kind `name` at an already known place.
export function makeError(name, message, line, column, offset, end) {
  return {
    name: name,
    message: message,
    line: line,
    column: column,
    offset: offset,
    end: end,
  };
}

// An error covering offsets [start, end), placed with a line-start table.
export function errorAt(name, message, starts, start, end) {
  const p = position(starts, start);
  return makeError(name, message, p.line, p.column, start, end);
}

export function syntaxError(message, starts, start, end) {
  return errorAt(SYNTAX_ERROR, message, starts, start, end);
}

export function rangeError(message, starts, start, end) {
  return errorAt(RANGE_ERROR, message, starts, start, end);
}

// `line:column: message`.
export function formatError(e) {
  return `${e.line}:${e.column}: ${e.message}`;
}

// `Name: line:column: message`, for reports that mix kinds.
export function describeError(e) {
  return `${e.name}: ${formatError(e)}`;
}

// The reference's message for a source over the size limit, at 1:1.
export function sourceTooLarge(bytes, limit) {
  return makeError(
    SYNTAX_ERROR,
    `source is ${bytes} bytes, over the ${limit} byte limit`,
    1,
    1,
    0,
    0,
  );
}

// The reference's message for nesting past the parser's depth limit.
export function nestingTooDeep(limit, starts, start, end) {
  return syntaxError(`nesting is too deep (limit ${limit})`, starts, start, end);
}
