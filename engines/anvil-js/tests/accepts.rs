// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Source texts the parser must accept, grouped by area. One test per case,
//! so a failure names the construct.

#![forbid(unsafe_code)]

mod common;
use common::{module, script};

macro_rules! scripts {
    ($($name:ident: $src:expr;)*) => {
        $(#[test] fn $name() { script($src); })*
    };
}

macro_rules! modules {
    ($($name:ident: $src:expr;)*) => {
        $(#[test] fn $name() { module($src); })*
    };
}

scripts! {
    // declarations
    var_simple: "var a = 1, b, c = a;";
    let_const: "let a = 1; const b = 2;";
    let_as_identifier_sloppy: "var let = 1; let = 2; let.x;";
    let_newline_decl: "let\nx = 1";
    let_in_for_in_sloppy: "for (let in {}) ;";
    destructure_array_decl: "let [a, , b = 2, ...c] = arr;";
    destructure_object_decl: "const {a, b: {c}, d = 1, ...e} = obj;";
    destructure_nested_defaults: "var [{x = 1} = {}, [y] = []] = z;";
    function_decl: "function f(a, b) { return a + b; }";
    function_defaults_rest: "function f(a, b = a, ...c) {}";
    function_pattern_params: "function f({a, b}, [c, d]) {}";
    function_trailing_comma: "function f(a, b,) {} f(1, 2,);";
    generator_decl: "function* g() { yield 1; yield* other(); const x = yield; }";
    async_decl: "async function f() { await x; for await (const y of z) {} }";
    async_generator: "async function* g() { yield await 1; }";
    class_decl: "class A extends B { constructor() { super(); } m() { return super.m(); } }";
    class_fields: "class A { x = 1; #y = 2; static z; static #w = 3; ['k'] = 4; 'q'; 5; }";
    class_private_methods: "class A { #m() {} get #g() { return 1; } set #g(v) {} static #s() {} t() { return this.#m() + (#g in this); } }";
    class_accessors: "class A { get x() { return 1; } set x(v) {} static get y() {} }";
    class_static_block: "class A { static x; static { this.x = 1; var y = 2; } }";
    class_async_gen_methods: "class A { async m() { await 1; } *g() { yield 1; } async *ag() {} }";
    class_keywordish_names: "class A { static() {} get() {} set() {} async() {} static static() {} 'constructor'() {} }";
    class_field_asi: "class A { a\n b = 1\n ['c']\n d() {} }";
    class_expr: "var C = class Named extends (a, b) {};";
    class_method_named_get_newline: "class A { get\n x() {} }";
    nested_class_private_access: "class A { #x; m() { class B { n(o) { return o.#x; } } } }";
    // statements
    if_else: "if (a) b; else if (c) d; else { e; }";
    loops: "for (;;) break; for (var i = 0; i < 1; i++) continue; while (x) x--; do x++; while (x < 3)";
    for_in_of: "for (var k in o) ; for (const v of a) ; for (let [k, v] of m) ; for (x.y of z) ; for ([a, b] of c) ;";
    for_in_var_init_sloppy: "for (var a = 1 in {}) ;";
    for_of_async_arrow: "for (async of => {};;) break;";
    for_await_of: "async function f() { for await (x of y) ; }";
    for_let_of_of: "for (let of of []) ;";
    for_in_with_in_in_parens: "for (var x = (a in b) ? 1 : 2; ;) break;";
    labeled: "outer: for (;;) { inner: for (;;) { continue outer; } break outer; }";
    labeled_block_break: "lbl: { break lbl; }";
    labeled_nested_loop: "a: b: while (1) continue a;";
    labeled_function_sloppy: "l: function f() {}";
    switch_stmt: "switch (x) { case 1: a(); case 2: break; default: b(); }";
    switch_lexical: "switch (x) { case 0: let y = 1; break; case 1: { let y = 2; } }";
    try_catch: "try { a(); } catch (e) { b(e); } finally { c(); }";
    try_optional_binding: "try {} catch {}";
    try_catch_pattern: "try {} catch ({message}) {} try {} catch ([a, b]) {}";
    catch_var_same_name: "try {} catch (e) { var e = 1; }";
    with_sloppy: "with (obj) { x; }";
    debugger_stmt: "debugger;";
    throw_stmt: "function f() { throw new Error('x'); }";
    empty_statements: ";;; if (a) ; else ;";
    return_asi: "function f() { return\n1 }";
    if_function_annex_b: "if (a) function f() {}";
    block_functions_sloppy_dupes: "{ function f() {} function f() {} }";
    // expressions
    arith: "a + b * c - d / e % f ** g ** h;";
    unary_ops: "!a; ~b; -c; +d; typeof e; void f; delete g.h;";
    update_ops: "a++; b--; ++c; --d; e.f++; ++g[h];";
    comparisons: "a < b; a > b; a <= b; a >= b; a == b; a != b; a === b; a !== b; a instanceof B; 'x' in o;";
    bitwise: "a & b | c ^ d << e >> f >>> g;";
    logical: "a && b || c; d ?? e; (a || b) ?? c; a ?? (b && c);";
    conditional_chain: "a ? b : c ? d : e;";
    assignment_ops: "a = 1; a += 1; a -= 1; a *= 1; a /= 1; a %= 1; a **= 1; a <<= 1; a >>= 1; a >>>= 1; a &= 1; a |= 1; a ^= 1; a &&= 1; a ||= 1; a ??= 1;";
    destructuring_assignment: "[a, b] = [b, a]; ({a, b: c.d, ...e} = f); [x.y, ...z[0]] = w;";
    destructuring_assign_defaults: "({a = 1, b: {c} = {}} = d); [e = 2] = f;";
    destructuring_assign_parens: "[(a), (b.c)] = d; ({e: (f)} = g);";
    comma_expr: "a, b, c;";
    optional_chaining: "a?.b; a?.[b]; a?.(b); a?.b.c(d)[e]; a?.b?.();";
    nullish_assign_member: "a.b ??= c;";
    exponent_paren_unary: "(-a) ** b; a ** -b;";
    spread_everywhere: "f(...a, ...b); [...a, ...b]; ({...a, ...b}); new F(...a);";
    template_literals: "`plain`; `a${b}c${d}e`; `${`nested ${deep}`}`;";
    tagged_templates: "tag`x${y}z`; a.b`c`; f()`g`; tag`\\unicode`;";
    template_with_object_in_sub: "`${{a: 1}.a}`;";
    new_forms: "new A; new A(); new A.B(); new new A()(); new (f())(); new a.b.c;";
    new_target: "function f() { return new.target; }";
    arrows: "x => x; (x) => x; (a, b) => a + b; () => {}; ([a], {b}) => a; (...r) => r; (a = 1) => a;";
    arrow_body_object: "() => ({a: 1});";
    async_arrows: "async x => x; async (a, b) => a; async () => { await 1; };";
    async_as_name: "var async = 1; async(1); async\n(1); async => async;";
    arrow_newline_body: "x =>\n x";
    iife_forms: "(function () {})(); (() => {})(); !function () {}();";
    object_literal_forms: "({a: 1, 'b': 2, 3: 3, [d]: 4, e, f() {}, get g() { return 1; }, set g(v) {}, async h() {}, *i() {}, async *j() {}});";
    object_keyword_keys: "({if: 1, class: 2, new: 3, get: 4, set: 5, async: 6, await: 7});";
    object_get_set_as_names: "({get, set, async});";
    object_proto_once: "({__proto__: a, ['__proto__']: b, __proto__() {}});";
    object_proto_dupe_pattern_ok: "({__proto__: a, __proto__: b} = c);";
    regex_literals: "/a/; /[/]/g; x = /b\\/c/i; f(/d/);";
    regex_after_keywords: "return_ = 1; typeof /x/; void /y/; if (/z/.test(s)) ;";
    regex_vs_division: "a / b / c; (a) / b; a[0] / 2; x++ / 2; 1 / 2 / 3;";
    regex_after_brace_statement: "{}\n/foo/";
    division_after_paren_expr: "(1) / 2;";
    regex_in_template: "`${/x/.source}`;";
    numbers: "0; 1.5; .5; 5.; 1e10; 0x1F; 0o17; 0b11; 1_000; 10n; 0xFFn;";
    legacy_octal_sloppy: "017; 08; '\\07';";
    strings: "'a'; \"b\"; '\\n\\t\\u0041\\u{1F600}\\x41';";
    unicode_identifiers: "var café = 1, π = 3.14, ǅ = 2, ᾈ = 4, 변수 = 5, $ = 0, _ = 1;";
    unicode_escape_identifier: "var \\u0061 = 1; \\u{62} = a;";
    this_and_super_member_in_object: "({ m() { return super.x; } });";
    getter_in_object_super: "({ get x() { return super.x; } });";
    import_dynamic_in_script: "import('x').then(f); import(a, {with: {type: 'json'}});";
    yield_as_name_sloppy: "var yield = 1; yield;";
    await_as_name_script: "var await = 1; await;";
    in_operator_in_for_body: "for (;;) { a in b; break; }";
    private_in: "class A { #x; static has(o) { return #x in o; } }";
    html_comment_script: "x = 1 <!-- comment\n--> also a comment\n;";
    hashbang: "#!/usr/bin/env anvil\nvar x;";
    directive_prologue: "'use strict'; var x = 1;";
    use_strict_function: "function f() { 'use strict'; return 1; }";
    asi_basic: "a = 1\nb = 2\nc()";
    asi_postfix_restriction: "a\n++b";
    asi_before_close_brace: "{ a }";
    asi_do_while: "do {} while (0) x";
    asi_continue_newline: "for (;;) { continue\nfoo }";
    asi_break_newline: "for (;;) { break\nfoo }";
    asi_arrow_block_then_paren: "var f = () => {}\n(1)";
    paren_call_not_asi: "a\n(b)";
    sequence_in_for: "for (i = 0, j = 1; i < j; i++, j--) ;";
    complex_bundle_like: "!function(e){var t={};function n(r){if(t[r])return t[r].exports;var o=t[r]={i:r,l:!1,exports:{}};return e[r].call(o.exports,o,o.exports,n),o.l=!0,o.exports}n.m=e}([function(e,t){e.exports=1}]);";
    getter_setter_numeric_keys: "({ get 1() {}, set 'two'(v) {}, get [3]() {} });";
    class_extends_call: "class A extends mixin(B, C) {}";
    nested_functions_scopes: "function a() { var x; function b() { let x; { const x = 1; } } }";
    var_redeclare: "var a; var a; function f() {} var f;";
    eval_arguments_sloppy: "var eval, arguments; eval = 1;";
    super_in_arrow_in_method: "({ m() { return () => super.x; } });";
    super_call_in_arrow_ctor: "class A extends B { constructor() { (() => super())(); } }";
    new_target_in_arrow_in_fn: "function f() { return () => new.target; }";
    deep_ok_nesting: "((((((((((a))))))))));";
    comments_everywhere: "/* a */ var /* b */ x /* c */ = // d\n 1 /* e */;";
    bigint_keys: "({ 1n: 1 });";
    labeled_continue_in_do: "l: do { continue l; } while (0)";
    yield_in_generator_params_default_name: "function* g(x = 1) {}";
    async_method_named_get: "({ async get() {} });";
    generator_method_computed: "({ *[Symbol.iterator]() {} });";
}

modules! {
    module_imports: "import a from 'a'; import * as b from 'b'; import {c, d as e} from 'c'; import f, {g} from 'f'; import h, * as i from 'h'; import 'side';";
    module_import_string_name: "import {'a-b' as ab} from 'x';";
    module_import_attributes: "import data from './d.json' with { type: 'json' };";
    module_exports: "export var a = 1; export let b; export const c = 3; export function d() {} export class E {} export async function f() {}";
    module_export_list: "var a, b; export {a, b as c}; export {a as 'string name'};";
    module_export_from: "export {x} from 'x'; export {y as z} from 'y'; export * from 'w'; export * as ns from 'v'; export {'s' as t} from 'u';";
    module_export_default_expr: "export default 1 + 2;";
    module_export_default_fn: "export default function () {}";
    module_export_default_named_fn: "export default function named() {} named();";
    module_export_default_class: "export default class {}";
    module_export_default_async: "export default async function () {}";
    module_top_level_await: "await x; for await (const y of z) ;";
    module_import_meta: "import.meta.url;";
    module_strict_by_default: "var x = 1;";
    module_export_default_arrow: "export default () => 1;";
    module_export_hoisted_function: "export { f }; function f() {}";
    module_dynamic_import: "import('./x.js');";
    module_default_and_named_export: "export default 1; export const x = 2;";
}
