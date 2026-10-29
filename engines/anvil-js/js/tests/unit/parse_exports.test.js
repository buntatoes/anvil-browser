// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Module export lists: duplicate exports, exports of undeclared names, import bindings.
// Each case prints `index OK` or `index line:col: message`; the
// .expected file is what the reference parser says for the same sources.

import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { unitsOf } from "../../src/syntax/token_dump.js";

// Each case is [source, goal]: "s" for a script, "m" for a module.
function checkAll(cases) {
  let i = 0;
  while (i < cases.length) {
    const c = cases[i];
    const p = createParser(unitsOf(c[0]), c[1] === "m");
    const prog = parseProgram(p);
    if (prog === null) {
      console.log(`${i} ${formatError(parseError(p))}`);
    } else {
      console.log(`${i} OK`);
    }
    i += 1;
  }
}

checkAll([
  ["export { \"\\uD800\" as a } from \"x\"", "s"],
  ["export { \"\\uD800\" as a } from \"x\"", "m"],
  ["export { a as \"\\uD800\" } from \"x\"", "m"],
  ["import { \"\\uD800\" as a } from \"x\"", "m"],
  ["export { x as y, z as y }; var x, z;", "m"],
  ["export { a }; ", "m"],
  ["export { a }; { let a; }", "m"],
  ["export default 1; export default 2;", "m"],
  ["export default function () {} export { a as default }; var a;", "m"],
  ["import { a, a } from \"x\";", "m"],
  ["import { a } from \"x\"; let a;", "m"],
  ["import a, * as a from \"x\";", "m"],
  ["function f(){} function f(){}", "m"],
  ["function f(){} var f;", "m"],
  ["export let a; export function a() {}", "m"],
  ["export * as a from \"x\"; export * as a from \"y\";", "m"],
  ["export { a as \"b\" }; export { c as \"b\" }; var a, c;", "m"],
  ["export { \"b\" } from \"x\"; export { b }; var b;", "m"],
  ["export var [a, a] = [];", "m"],
  ["export { a }; { var a; }", "m"],
  ["var f; var f;", "m"],
  ["export { a }; import { a } from \"x\";", "m"],
  ["export * from \"x\"; export * from \"y\";", "m"],
  ["export { arguments }; var arguments;", "m"],
  ["export { eval } from \"x\";", "m"],
  ["export { if } from \"x\";", "m"],
  ["export { if };", "m"],
  ["let a; export { a as if };", "m"],
  ["import.meta", "s"],
  ["import.meta", "m"],
  ["import(a) = 1", "s"],
  ["await: ;", "m"],
  ["await 1;", "m"],
  ["var await;", "m"],
  ["export { x as y }; export let y = 1; var x;", "m"],
  ["export default class {} export default 1;", "m"],
  ["export { default } from \"x\"; export default 1;", "m"],
  ["let x; export { x as \"y\\u0000\" }; export { x as \"y\\0\" };", "m"],
]);
