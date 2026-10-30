// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Scope analysis against binding tables written by hand from the spec.
// Each case parses a source, runs the analysis (and its self-check) and
// compares a compact table of the result with the case's lines:
//
//   B <fn>.<scope> <kind> <name> [flags]   a binding
//   R <name> <kind> -> <fn>.<scope> [flags] | R <name> <kind> -> global [flags]
//                                          a reference
//   F <fn> <kind> [flags]                  a function
//   S <fn>.<scope> [flags]                 a scope with a flag set
//                                          (inits=N: N functions made on entry)
//
// <fn> is the function's name, or its kind when it has none (`script`,
// `module`, `arrow`, `method`, ...); <scope> is the scope kind. Only the
// line types a case lists are compared, so a case can check just its
// bindings or just its references. Lines are compared as a set (sorted),
// so a case lists them in any order.
// needs: host
// expect: 15 cases, 0 failed
// expect: 10 cases, 0 failed
// expect: 8 cases, 0 failed
// expect: 8 cases, 0 failed
// expect: 3 cases, 0 failed
// expect: 5 cases, 0 failed

import { createParser, parseProgram } from "../../src/syntax/parser.js";
import { parseError } from "../../src/syntax/parse_messages.js";
import { formatError } from "../../src/core/errors.js";
import { analyzeProgram, FN_FUNCTION, RES_GLOBAL, RES_CAPTURED, B_PRIVATE } from "../../src/syntax/scope.js";
import { scopeKindName, bindingKindName, funcKindName, privateUnits } from "../../src/syntax/scope_dump.js";
import { unitsOf } from "../../src/syntax/token_dump.js";
import { checkScopes } from "../../src/syntax/scope_check.js";

const REF_KINDS = ["read", "write", "read_write", "export", "private"];

function text(units) {
  let s = "";
  for (const u of units) {
    s += String.fromCharCode(u);
  }
  return s;
}

function funcLabel(a, id) {
  const f = a.funcs[id];
  if (f.kind === FN_FUNCTION && f.node.id !== null) {
    return text(f.node.id.name);
  }
  return funcKindName(f.kind);
}

function scopeLabel(a, s) {
  const sc = a.scopes[s];
  return `${funcLabel(a, sc.func)}.${scopeKindName(sc.kind)}`;
}

function flags(list) {
  let s = "";
  for (const x of list) {
    if (x[0]) {
      s += " " + x[1];
    }
  }
  return s;
}

function bindingName(b) {
  if (b.kind === B_PRIVATE) {
    return text(privateUnits(b.name));
  }
  return text(b.name);
}

function bindingLines(a, out) {
  for (const b of a.bindings) {
    const f = flags([
      [b.tdz, "tdz"],
      [b.captured, "captured"],
      [b.written, "written"],
      [b.mutatedAfterCapture, "mutated"],
      [b.loopFresh, "loop_fresh"],
      [b.exported, "exported"],
      [b.annexVar >= 0, "annex"],
      [b.annexOnly, "annex_only"],
      [b.fromParam >= 0, "from_param"],
      [b.argumentsObject, "arguments_object"],
      [b.isStatic, "static"],
    ]);
    out.push(`B ${scopeLabel(a, b.scope)} ${bindingKindName(b.kind)} ${bindingName(b)}${f}`);
  }
}

function refLines(a, out) {
  // Source order (insertion sort, stable).
  const order = [];
  for (const r of a.refs) {
    let j = order.length - 1;
    order.push(r);
    while (j >= 0 && order[j].node.start > r.node.start) {
      order[j + 1] = order[j];
      j -= 1;
    }
    order[j + 1] = r;
  }
  for (const r of order) {
    let to = "global";
    if (r.resolution !== RES_GLOBAL) {
      to = scopeLabel(a, a.bindings[r.binding].scope);
    }
    const f = flags([
      [r.resolution === RES_CAPTURED, "captured"],
      [r.dynamic, "dynamic"],
      [r.tdzCheck, "tdz_check"],
      [r.typeofRef, "typeof"],
      [r.deleteRef, "delete"],
      [r.declInit, "init"],
    ]);
    let name = text(r.name);
    if (r.kind === 4) {
      name = text(privateUnits(r.name));
    }
    out.push(`R ${name} ${REF_KINDS[r.kind]} -> ${to}${f}`);
  }
}

function funcLines(a, out) {
  for (const f of a.funcs) {
    let caps = "";
    for (const b of f.captures) {
      caps += " " + bindingName(a.bindings[b]);
    }
    const fl = flags([
      [f.strict, "strict"],
      [f.usesThis, "this"],
      [f.thisFromArrows, "this_from_arrows"],
      [f.usesNewTarget, "new_target"],
      [f.usesSuperProperty, "super_property"],
      [f.usesSuperCall, "super_call"],
      [f.argumentsBinding >= 0, "arguments"],
      [f.mapped, "mapped"],
      [f.directEval, "eval"],
      [f.sloppyEval, "sloppy_eval"],
      [!f.simpleParams, "complex_params"],
      [f.paramExpressions, "param_expressions"],
      [caps !== "", `captures=(${caps.substring(1)})`],
    ]);
    out.push(`F ${funcLabel(a, f.id)} ${funcKindName(f.kind)}${fl}`);
  }
}

function scopeLines(a, out) {
  for (const sc of a.scopes) {
    const f = flags([
      [sc.strict, "strict"],
      [sc.grows, "grows"],
      [sc.evalVisible, "eval_visible"],
      [sc.funcInits.length > 0, `inits=${sc.funcInits.length}`],
    ]);
    if (f !== "") {
      out.push(`S ${scopeLabel(a, sc.id)}${f}`);
    }
  }
}

function wants(want, c) {
  for (const l of want) {
    if (l.substring(0, 1) === c) {
      return true;
    }
  }
  return false;
}

function sorted(list) {
  const out = [];
  for (const x of list) {
    let j = out.length - 1;
    out.push(x);
    while (j >= 0 && out[j] > x) {
      out[j + 1] = out[j];
      j -= 1;
    }
    out[j + 1] = x;
  }
  return out;
}

// Runs one case; prints what differs and returns whether it matched.
function checkCase(title, src, isModule, want) {
  const p = createParser(unitsOf(src), isModule);
  const prog = parseProgram(p);
  if (prog === null) {
    console.log(`FAIL ${title}: ${formatError(parseError(p))}`);
    return false;
  }
  const a = analyzeProgram(prog);
  const got = [];
  if (wants(want, "B")) {
    bindingLines(a, got);
  }
  if (wants(want, "R")) {
    refLines(a, got);
  }
  if (wants(want, "F")) {
    funcLines(a, got);
  }
  if (wants(want, "S")) {
    scopeLines(a, got);
  }
  const problems = checkScopes(prog, a);
  if (problems.length > 0) {
    console.log(`FAIL ${title}: self-check: ${problems[0]}`);
    return false;
  }
  const g = sorted(got);
  const w = sorted(want);
  const missing = [];
  const extra = [];
  let i = 0;
  let j = 0;
  while (i < w.length || j < g.length) {
    if (j >= g.length || (i < w.length && w[i] < g[j])) {
      missing.push(w[i]);
      i += 1;
    } else if (i >= w.length || g[j] < w[i]) {
      extra.push(g[j]);
      j += 1;
    } else {
      i += 1;
      j += 1;
    }
  }
  const ok = missing.length === 0 && extra.length === 0;
  if (!ok) {
    console.log(`FAIL ${title}`);
    for (const l of missing) {
      console.log(`  missing: ${l}`);
    }
    for (const l of extra) {
      console.log(`  extra:   ${l}`);
    }
  }
  return ok;
}

// Runs a list of cases ([title, source, isModule, lines]) and prints the
// count.
function runCases(cases) {
  let failed = 0;
  for (const c of cases) {
    if (!checkCase(c[0], c[1], c[2], c[3])) {
      failed += 1;
    }
  }
  console.log(`${cases.length} cases, ${failed} failed`);
}

// Binding kinds, where each declaration lands, and TDZ
// (GlobalDeclarationInstantiation 16.1.7, FunctionDeclarationInstantiation
// 10.2.11, BlockDeclarationInstantiation 14.2.3, catch 14.15.2, for 14.7.4
// and 14.7.5, switch 14.12.4, classes 15.7.14).
runCases([
  [
    "script top level: var and function are var-scoped, let/const/class lexical with TDZ",
    "var a; function f() {} let b; const c = 1; class D {}",
    false,
    [
      "B script.global var a",
      "B script.global function f",
      "B script.global let b tdz",
      "B script.global const c tdz",
      "B script.global class D tdz",
      "B script.class class-name D tdz",
    ],
  ],
  [
    "var in nested blocks goes to the function; let stays in the block",
    "function f() { { var a; let b; { var c; const d = 1; } } }",
    false,
    [
      "B script.global function f",
      "B f.function var a",
      "B f.block let b tdz",
      "B f.function var c",
      "B f.block const d tdz",
    ],
  ],
  [
    "parameters, and a var of a parameter name is the same binding",
    "function f(a, b) { var a; var c; }",
    false,
    ["B script.global function f", "B f.function param a", "B f.function param b", "B f.function var c"],
  ],
  [
    "parameter expressions: parameters have a TDZ, body vars get their own scope, same-named vars copy the parameter",
    "function f(a, b = a) { var a; var c; }",
    false,
    [
      "B script.global function f",
      "B f.function param a tdz",
      "B f.function param b tdz",
      "B f.body var a from_param",
      "B f.body var c",
    ],
  ],
  [
    "catch parameter in its own scope, the catch block apart",
    "try {} catch (e) { let x; var y; }",
    false,
    ["B script.catch catch e", "B script.block let x tdz", "B script.global var y"],
  ],
  [
    "destructuring catch parameter",
    "try {} catch ({ a, b: [c] }) {}",
    false,
    ["B script.catch catch a", "B script.catch catch c"],
  ],
  [
    "for with let: head scope; for with var: function scope (the loop writes it)",
    "for (let i = 0; ; ) {} for (var j = 0; ; ) {} for (const k of []) {} for (var m in {}) {}",
    false,
    ["B script.for let i tdz", "B script.global var j written", "B script.for const k tdz", "B script.global var m written"],
  ],
  [
    "switch cases share one scope (a sloppy function there also gets its annex B var)",
    "switch (0) { case 0: let a; case 1: function g() {} default: const b = 1; }",
    false,
    ["B script.switch let a tdz", "B script.switch function g annex", "B script.global var g annex_only", "B script.switch const b tdz"],
  ],
  [
    "named function expression: its own name scope",
    "(function g() {});",
    false,
    ["B script.function-name function-name g"],
  ],
  [
    "class: outer binding, inner name, private names in the body",
    "class C { #a; #m() {} get #p() {} set #p(v) {} static #s = 1; }",
    false,
    [
      "B script.global class C tdz",
      "B script.class class-name C tdz",
      "B script.class-body private #a",
      "B script.class-body private #m",
      "B script.class-body private #p",
      "B script.class-body private #s static",
      "B setter.function param v",
    ],
  ],
  [
    "class expression: only the inner name, uninitialized until the class is made",
    "(class K {});",
    false,
    ["B script.class class-name K tdz"],
  ],
  [
    "function declarations in a block are block-scoped in strict code",
    '"use strict"; { function f() {} }',
    false,
    ["B script.block function f"],
  ],
  [
    "a let read before its declaration in the same function needs a TDZ check; after it does not",
    "x; let x; x;",
    false,
    ["R x read -> script.global tdz_check", "R x read -> script.global"],
  ],
  [
    "a let read from a nested function always needs a TDZ check; var never does",
    "function f() { return [a, b]; } let a; var b;",
    false,
    ["R a read -> script.global captured tdz_check", "R b read -> script.global captured"],
  ],
  [
    "var initializer: a write through name resolution (init); a let initializer is not a reference",
    "let a = 1; var b = 2; b = 3;",
    false,
    ["R b write -> script.global init", "R b write -> script.global"],
  ],
]);

// Annex B block functions (B.3.2.1 functions, B.3.2.2 scripts, B.3.4
// catch blocks): a sloppy FunctionDeclaration in a block also gets a var
// in the function, unless that var would be an early error.
runCases([
  [
    "a sloppy block function gets a var copy",
    "function f() { { function g() {} } }",
    false,
    ["B script.global function f", "B f.block function g annex", "B f.function var g annex_only"],
  ],
  [
    "the copy joins an existing var of the name",
    "function f() { var g; { function g() {} } }",
    false,
    ["B script.global function f", "B f.function var g", "B f.block function g annex"],
  ],
  [
    "no copy past a let of the name in between",
    "function f() { let g; { function g() {} } }",
    false,
    ["B script.global function f", "B f.function let g tdz", "B f.block function g"],
  ],
  [
    "no copy when a parameter has the name",
    "function f(g) { { function g() {} } }",
    false,
    ["B script.global function f", "B f.function param g", "B f.block function g"],
  ],
  [
    "a simple catch parameter of the name does not block the copy; a destructuring one does",
    "try {} catch (e) { { function e() {} } } try {} catch ([d]) { { function d() {} } }",
    false,
    [
      "B script.catch catch e",
      "B script.block function e annex",
      "B script.global var e annex_only",
      "B script.catch catch d",
      "B script.block function d",
    ],
  ],
  [
    "async functions and generators get no copy",
    "{ async function a() {} function* b() {} }",
    false,
    ["B script.block function a", "B script.block function b"],
  ],
  [
    "strict code gets no copy",
    'function f() { "use strict"; { function g() {} } }',
    false,
    ["B script.global function f", "B f.block function g"],
  ],
  [
    "if clause function (B.3.3) is in its own block and gets a copy",
    "if (1) function g() {}",
    false,
    ["B script.block function g annex", "B script.global var g annex_only"],
  ],
  [
    "a function expression's own name does not block the copy",
    "(function s() { { function s() {} } return s; });",
    false,
    [
      "B script.function-name function-name s",
      "B s.block function s annex",
      "B s.function var s annex_only",
      "R s read -> s.function",
    ],
  ],
  [
    "labelled block functions are declared but get no copy",
    "{ L: function g() {} }",
    false,
    ["B script.block function g"],
  ],
]);

// Resolution: local, captured, global, and dynamic through `with` or a
// sloppy direct eval (ResolveBinding 9.4.2, GetIdentifierReference 9.1.2.1,
// PerformEval 19.2.1.1, EvalDeclarationInstantiation 19.2.1.3).
runCases([
  [
    "unbound names are global",
    "x = 1; typeof y; delete z;",
    false,
    ["R x write -> global", "R y read -> global typeof", "R z read -> global delete"],
  ],
  [
    "captured references (a let read from another function is TDZ-checked), captured and written bindings",
    "function f() { let a = 0; let b = 0; a++; return () => [a, b]; }",
    false,
    [
      "B script.global function f",
      "B f.function let a tdz captured written mutated",
      "B f.function let b tdz captured",
      "R a read_write -> f.function",
      "R a read -> f.function captured tdz_check",
      "R b read -> f.function captured tdz_check",
    ],
  ],
  [
    "a captured var written only by its initializer still counts as written",
    "function f() { var a = 1; return () => a; }",
    false,
    ["B script.global function f", "B f.function var a captured written mutated"],
  ],
  [
    "inside `with`, every reference is dynamic, even to a binding found outside",
    "var o; with (o) { x; o; }",
    false,
    ["R o read -> script.global", "R x read -> global dynamic", "R o read -> script.global dynamic"],
  ],
  [
    "a sloppy direct eval makes the function's names dynamic (`eval` itself too); references outside stay static",
    "var a; function f() { eval(''); a; } a;",
    false,
    [
      "R eval read -> global dynamic",
      "R a read -> script.global captured dynamic",
      "R a read -> script.global",
      "S f.function grows eval_visible",
      "S script.global eval_visible inits=1",
    ],
  ],
  [
    "a strict direct eval does not add names: references stay static",
    '"use strict"; var a; function f() { eval(""); a; }',
    false,
    [
      "R eval read -> global",
      "R a read -> script.global captured",
      "S script.global strict eval_visible inits=1",
      "S f.function strict eval_visible",
    ],
  ],
  [
    "an indirect or optional call of eval is not a direct eval",
    "(0, eval)(''); eval?.('');",
    false,
    ["F script script"],
  ],
  [
    "per-iteration copies only for captured loop lexicals",
    "for (let i = 0; i < 1; i++) { () => i; } for (let j = 0; j < 1; j++) {} for (const k of []) { () => k; }",
    false,
    [
      "B script.for let i tdz captured written mutated loop_fresh",
      "B script.for let j tdz written",
      "B script.for const k tdz captured loop_fresh",
    ],
  ],
]);

// Functions: this, arguments, new.target, super, strictness, parameter
// lists (10.2.11, OrdinaryCallBindThis 10.2.1.2, arrows 15.3, 10.4.4).
runCases([
  [
    "arrows use the enclosing function's this: the flag is on the function that binds it",
    "function f() { return () => () => this; }",
    false,
    ["F script script", "F f function this this_from_arrows", "F arrow arrow", "F arrow arrow"],
  ],
  [
    "arguments object: mapped when sloppy with simple parameters",
    "function m(a) { arguments; } function s(a) { 'use strict'; arguments; } function d(a = 1) { arguments; } function n() {}",
    false,
    [
      "F script script",
      "F m function arguments mapped",
      "F s function strict arguments",
      "F d function arguments complex_params param_expressions",
      "F n function",
    ],
  ],
  [
    "no arguments object when a parameter, function or lexical declaration takes the name",
    "function p(arguments) { arguments; } function q() { function arguments() {} arguments; } function r() { let arguments; arguments; }",
    false,
    ["F script script", "F p function", "F q function", "F arguments function", "F r function"],
  ],
  [
    "var arguments is the arguments object",
    "function n() { var arguments; return arguments; }",
    false,
    ["B script.global function n", "B n.function var arguments arguments_object", "F script script", "F n function arguments mapped"],
  ],
  [
    "arrows take arguments from the enclosing function",
    "function f() { return () => arguments; }",
    false,
    ["F script script", "F f function arguments mapped", "F arrow arrow captures=(arguments)"],
  ],
  [
    "duplicate parameters still map",
    "function f(a, a) { arguments; }",
    false,
    ["F script script", "F f function arguments mapped"],
  ],
  [
    "new.target and super in a constructor; an arrow's super.x uses the constructor's this and home object; class code is strict, the script is not",
    "class A extends Object { constructor() { super(); () => super.x; new.target; } }",
    false,
    ["F script script", "F constructor constructor strict this this_from_arrows new_target super_property super_call", "F arrow arrow strict"],
  ],
  [
    "a use strict directive makes the function and what is inside strict",
    "function f() { 'use strict'; return function g() {}; } function h() {}",
    false,
    ["F script script", "F f function strict", "F g function strict", "F h function"],
  ],
]);

// Modules: imports are immutable bindings with TDZ in the module scope,
// exports mark bindings (16.2.1.6.4 InitializeEnvironment).
runCases([
  [
    "imports (a namespace object has no TDZ) and exports",
    "import d, { a as b } from 'm'; import * as ns from 'n'; export let x; export function f() {} export default function () {} var y; export { y as z };",
    true,
    [
      "B module.module import d tdz",
      "B module.module import b tdz",
      "B module.module import ns",
      "B module.module let x tdz exported",
      "B module.module function f exported",
      "B module.module function *default* exported",
      "B module.module var y exported",
    ],
  ],
  [
    "every read of a named import is TDZ-checked",
    "import { a } from 'm'; a; function f() { a; }",
    true,
    ["R a read -> module.module tdz_check", "R a read -> module.module captured tdz_check"],
  ],
  [
    "module top level is strict and below the global scope",
    "var a;",
    true,
    ["B module.module var a", "S module.global strict", "S module.module strict"],
  ],
]);

// Hoisting, private names and class element functions (15.7.14, 7.3.31
// PrivateElementFind, 15.7.10 ClassFieldDefinitionEvaluation).
runCases([
  [
    "function declarations are made on entry to their scope; the last of a name wins",
    "f(); function f() {} function f() {} { g(); function g() {} }",
    false,
    ["S script.global inits=1", "S script.block inits=1"],
  ],
  [
    "private names resolve to the nearest class body that declares them (a class body belongs to the function around the class)",
    "class A { #x; m(o) { class B { #y; n() { return [o.#x, this.#y, #x in o]; } } } }",
    false,
    [
      "R o read -> method.function captured",
      "R #x private -> script.class-body captured",
      "R #y private -> method.class-body captured",
      "R #x private -> script.class-body captured",
      "R o read -> method.function captured",
    ],
  ],
  [
    "field initializers and static blocks are functions of their own; an implicit constructor makes none",
    "class C { a = this; static { var v; this; } }",
    false,
    ["F script script", "F field field strict this", "F static-block static-block strict this"],
  ],
  [
    "a switch lexical is TDZ-checked everywhere in the switch",
    "switch (0) { case 0: a; case 1: let a; a; }",
    false,
    ["R a read -> script.switch tdz_check", "R a read -> script.switch tdz_check"],
  ],
  [
    "inside a function, `with` makes references dynamic and the eval-free names around stay static",
    "function f(o) { var a; with (o) { a = 1; } return a; }",
    false,
    ["R o read -> f.function", "R a write -> f.function dynamic", "R a read -> f.function"],
  ],
]);
