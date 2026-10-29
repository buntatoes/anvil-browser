// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Parser: Module forms: imports, exports, attributes, import.meta, top-level await.
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
  ["import a from 'm'", "m"],
  ["import * as ns from 'm'", "m"],
  ["import {a as b, default as c, 'd e' as f} from 'm'", "m"],
  ["import 'm' with { type: 'json' }", "m"],
  ["import a, * as b from 'm'", "m"],
  ["export * from 'm'", "m"],
  ["export * as ns from 'm'", "m"],
  ["export * as 'x y' from 'm'", "m"],
  ["export { a as 'b' }; let a;", "m"],
  ["export {a}; var a;", "m"],
  ["export { 'a' }", "m"],
  ["export { a } from 'm'", "m"],
  ["export { 'a' as b } from 'm'", "m"],
  ["export default function () {}", "m"],
  ["export default class {}", "m"],
  ["export default async function* () {}", "m"],
  ["export default (1, 2)", "m"],
  ["export default 1, 2", "m"],
  ["export var a, b;", "m"],
  ["export let [a] = b;", "m"],
  ["export const {c} = d;", "m"],
  ["export class A {}", "m"],
  ["export function f() {}", "m"],
  ["export async function f() {}", "m"],
  ["import.meta", "m"],
  ["import.meta.url", "m"],
  ["import(a)", "m"],
  ["await 1", "m"],
  ["function f() { await 1 }", "m"],
  ["export default 1; export default 2", "m"],
  ["export {x}", "m"],
  ["if (1) import a from 'm'", "m"],
  ["<!-- x", "m"],
  ["a\n--> b", "m"],
  ["import {a,} from 'm'", "m"],
  ["import {} from 'm'", "m"],
  ["export {}", "m"],
  ["export {} from 'm'", "m"],
  ["import x from 'm' with {}", "m"],
  ["new.target", "m"],
  ["import.meta", "s"],
  ["let await", "m"],
  ["var yield", "m"],
  ["import a from 'm'", "s"],
  ["export var a", "s"],
  ["import x from 'm' with { type: json }", "m"],
  ["import { a as 'b' } from 'm'", "m"],
  ["export default function f() {} f()", "m"],
  ["export default async\nfunction f() {}", "m"],
  ["import a, {b} from 'm' with { 'type': 'json' }", "m"],
  ["export * as default from 'm'", "m"],
  ["import.meta = 1", "m"],
  ["import.metaa", "m"],
  ["import(", "m"],
  ["import()", "m"],
  ["export { default } from 'm'", "m"],
  ["export { default }", "m"],
  ["export let", "m"],
  ["export async", "m"],
  ["export default class A extends B {}", "m"],
  ["010", "m"],
  ["with (a) {}", "m"],
  ["arguments = 1", "m"],
]);
