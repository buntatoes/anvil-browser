// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Unit tests for the self-hosted ES2022+ AST lowering pass, Core JS code
// emitter, and Core JS standard library runtime helpers.

import { compileSource, transpileToCoreJs, parseAndAnalyze } from "../../src/index.js";
import {
  anvilSlice,
  anvilArrayMap,
  anvilArrayFilter,
  anvilArrayReduce,
  anvilArrayFind,
  anvilArrayFindIndex,
  anvilArraySome,
  anvilArrayEvery,
  anvilArrayIncludes,
  anvilArrayJoin,
  anvilStringTrim,
  anvilStringStartsWith,
  anvilStringEndsWith,
  anvilStringIncludes,
  anvilStringSplit,
  anvilStringReplace,
  anvilStringReplaceAll,
  anvilStringToLowerCase,
  anvilStringToUpperCase,
  AnvilPromise,
  anvilPromiseResolve,
  AnvilURL,
  AnvilEventTarget,
  AnvilCustomEvent,
} from "../../src/runtime/builtins.js";

// 1. Destructuring declarations (array, object, defaults, rest)
const d1 = compileSource("const [a, , b = 10, ...rest] = items; const { x, y: z = 5 } = obj;", false, false);
console.log("destructuring ok:", d1.ok, d1.code.indexOf("__anvil_slice") >= 0);

// 2. Default and destructuring parameters + arrow functions
const d2 = compileSource("function add(a = 1, [b, c] = [2, 3]) { return a + b + c; }", false, false);
console.log("params ok:", d2.ok, d2.code.indexOf("function add") >= 0);

// 3. Optional chaining and nullish coalescing assignment
const d3 = compileSource("let v = user?.profile?.name ?? 'anon'; v ??= 'fallback';", false, false);
console.log("optchain ok:", d3.ok, d3.code.indexOf("undefined") >= 0);

// 4. Class fields, static fields, static methods, private fields
const d4 = compileSource(
  "class Counter { #n = 0; static total = 0; inc() { this.#n += 1; return this.#n; } static reset() { Counter.total = 0; } }",
  false,
  false,
);
console.log("class ok:", d4.ok, d4.code.indexOf("__priv_n") >= 0, d4.code.indexOf("Counter.total = 0") >= 0);

// 5. Spread in arrays and objects + computed properties
const d5 = compileSource("const arr = [1, ...mid, 4]; const o = { a: 1, [k]: 2, ...extra };", false, false);
console.log("spread ok:", d5.ok, d5.code.indexOf("__anvil_assign") >= 0);

// 6. For-in, destructuring for-of, parameterless catch
const d6 = compileSource(
  "for (const k in obj) { for (const [a, b] of pairs) { try { work(a, b); } catch { } } }",
  false,
  false,
);
console.log("loops-try ok:", d6.ok, d6.code.indexOf("__anvil_keys") >= 0);

// 7. Syntax error reporting
const bad = compileSource("const x = ;", false, false);
console.log("syntax-err:", !bad.ok, bad.error.length > 0);

// 8. Parse and scope analysis entry point
const pa = parseAndAnalyze("function outer(x) { return () => x + 1; }", false);
console.log("analyze ok:", pa !== null, pa.scope.funcs.length === 3);

// 9. Runtime array helpers
const nums = [1, 2, 3, 4, 5];
const doubled = anvilArrayMap(nums, (n) => n * 2);
const evens = anvilArrayFilter(doubled, (n) => n % 4 === 0);
const sum = anvilArrayReduce(evens, (acc, n) => acc + n, 0);
console.log(
  "array helpers:",
  anvilArrayJoin(anvilSlice(nums, 1, 4), "-"),
  sum,
  anvilArrayFind(nums, (n) => n > 3),
  anvilArrayFindIndex(nums, (n) => n === 3),
  anvilArraySome(nums, (n) => n === 5),
  anvilArrayEvery(nums, (n) => n > 0),
  anvilArrayIncludes(nums, 4),
);

// 10. Runtime string helpers
console.log(
  "string helpers:",
  anvilStringTrim("  hello world \n"),
  anvilStringStartsWith("https://google.com", "https://"),
  anvilStringEndsWith("index.html", ".html"),
  anvilStringIncludes("youtube.com/watch", "watch"),
  anvilArrayJoin(anvilStringSplit("a,b,c", ","), "|"),
  anvilStringReplace("foo bar foo", "foo", "baz"),
  anvilStringReplaceAll("a-b-c", "-", "_"),
  anvilStringToLowerCase("GoOgLe"),
  anvilStringToUpperCase("gmail"),
);

// 11. Runtime Promise, URL, URLSearchParams, EventTarget, CustomEvent
const p = anvilPromiseResolve(21).then((v) => v * 2);
console.log("promise:", p instanceof AnvilPromise, p.value);

const u = new AnvilURL("https://www.youtube.com:443/watch?v=abc123&t=10s#top");
console.log("url:", u.protocol, u.hostname, u.port, u.pathname, u.searchParams.get("v"), u.hash);

const target = new AnvilEventTarget();
let received = "";
target.addEventListener("yt-navigate", (ev) => {
  received = ev.detail;
  ev.preventDefault();
});
const ev = new AnvilCustomEvent("yt-navigate", { detail: "home", cancelable: true });
const notCancelled = target.dispatchEvent(ev);
console.log("events:", received, notCancelled);
console.log("transpile:", transpileToCoreJs("const [p, q] = [1, 2];").indexOf("const p =") >= 0);
