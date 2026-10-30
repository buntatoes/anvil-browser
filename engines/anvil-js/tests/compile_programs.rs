// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! End-to-end: source text through the real parser, the compiler and the
//! VM, checked by what the program prints.

mod compile_support;
use compile_support::{error, fails, out, value};

#[test]
fn arithmetic_and_operators() {
    assert_eq!(
        out("console.log(1 + 2 * 3, (1 + 2) * 3, 7 % 3, -7 % 3, 2 ** 10, 1 / 0, -1 / 0, 0 / 0)"),
        "7 9 1 -1 1024 Infinity -Infinity NaN"
    );
    assert_eq!(
        out("console.log(5 >>> 1, -5 >> 1, 1 << 31, 5 & 3, 5 | 3, 5 ^ 3, ~5)"),
        "2 -3 -2147483648 1 7 6 -6"
    );
    assert_eq!(
        out(
            "console.log('5' * '2', '5' + 2, 'b' > 'a', null == undefined, null === undefined, NaN == NaN)"
        ),
        "10 52 true true false false"
    );
    assert_eq!(
        out("let i = 1; console.log(i++, i, ++i, i--, --i)"),
        "1 2 3 3 1"
    );
    assert_eq!(
        out("let a = 2; a += 3; a *= 2; a -= 1; a **= 2; a %= 7; console.log(a)"),
        "4"
    );
    assert_eq!(
        out("console.log((1, 2), void 0, typeof null, !0, !!'')"),
        "2 undefined object true false"
    );
    assert_eq!(value("1 + 1; 'last'"), "last");
}

#[test]
fn strings_and_templates() {
    assert_eq!(
        out("console.log(`a${1 + 2}b${'x'}`, `plain`, 'x'.length)"),
        "a3bx plain 1"
    );
    assert_eq!(
        out("let s = ''; for (const c of 'h\u{00e9}llo') s += c + '.'; console.log(s)"),
        "h.\u{00e9}.l.l.o."
    );
    assert_eq!(
        out("console.log('ab'[1], [] + [], [1, 2] + '', ({}) + '')"),
        "b  1,2 [object Object]"
    );
    assert_eq!(
        out("let s = ''; for (let i = 0; i < 5; i++) s += i; console.log(s)"),
        "01234"
    );
}

#[test]
fn objects_keep_their_identity() {
    assert_eq!(
        out("const a = {}; const b = a; b.z = 3; console.log(a.z, a === b, {} === {})"),
        "3 true false"
    );
    assert_eq!(
        out(
            "const m = { x: { y: 1 } }; m.x.y += 5; m.x.y++; ++m.x.y; m['x']['y'] *= 2; console.log(m.x.y)"
        ),
        "16"
    );
    assert_eq!(
        out(
            "const o = { a: 1, get b() { return this.a + 1; }, set b(v) { this.a = v; } }; o.b = 10; console.log(o.b, o.a)"
        ),
        "11 10"
    );
    assert_eq!(
        out(
            "const k = 'dyn'; const o = { [k + 1]: 1, 'q r': 2, 3: 3, f() { return 4; } }; console.log(o.dyn1, o['q r'], o[3], o.f())"
        ),
        "1 2 3 4"
    );
    assert_eq!(
        out(
            "const base = { get g() { return 'G' + this.n; } }; const d = { n: 1, __proto__: base }; console.log(d.g)"
        ),
        "G1"
    );
    assert_eq!(
        out("const o = { p: 1 }; console.log(delete o.p, 'p' in o, o.p)"),
        "true false undefined"
    );
    assert_eq!(
        out("const a = { x: 1 }; const b = { ...a, y: 2 }; console.log(b.x, b.y)"),
        "1 2"
    );
    assert_eq!(
        out("console.log([1, 2], { a: 1, b: 'x' }, [[1, [2, [3, [4]]]]])"),
        "[ 1, 2 ] { a: 1, b: 'x' } [ [ 1, [ 2, [Array] ] ] ]"
    );
}

#[test]
fn arrays_and_spread() {
    assert_eq!(
        out("const a = [1, ...[2, 3], , 5]; console.log(a.length, a[3], a)"),
        "5 undefined [ 1, 2, 3, <1 empty item>, 5 ]"
    );
    assert_eq!(
        out(
            "function f(a, b, c) { return a + b + c; } console.log(f(...[1, 2, 3]), f(1, ...[2, 3]), f(...'abc'))"
        ),
        "6 6 abc"
    );
    assert_eq!(
        out("function P(a, b) { this.s = a + b; } console.log(new P(...[1, 2]).s)"),
        "3"
    );
    assert_eq!(
        out("const a = []; a.push(1, 2); a.push(3); console.log(a.length, a)"),
        "3 [ 1, 2, 3 ]"
    );
}

#[test]
fn functions_and_closures() {
    assert_eq!(
        out("function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); } console.log(fib(20))"),
        "6765"
    );
    assert_eq!(
        out(
            "const c = (function () { let n = 0; return () => ++n; })(); c(); c(); console.log(c())"
        ),
        "3"
    );
    assert_eq!(
        out(
            "function pair() { let v = 0; return [() => ++v, () => v]; } const p = pair(); p[0](); p[0](); console.log(p[1]())"
        ),
        "2"
    );
    assert_eq!(
        out(
            "function a1() { let x = 1; return function () { let y = 2; return function () { return x + y; }; }; } console.log(a1()()())"
        ),
        "3"
    );
    assert_eq!(
        out(
            "const f = function g(n) { return n <= 1 ? 1 : n * g(n - 1); }; console.log(f(10), f.name)"
        ),
        "3628800 g"
    );
    assert_eq!(
        out(
            "const h = () => 1; const o = { m() {} }; console.log(h.name, o.m.name, (function () {}).name === '')"
        ),
        "h m true"
    );
    assert_eq!(
        out(
            "function r(a, ...rest) { return a + ':' + rest.length + ':' + rest[1]; } console.log(r(1, 2, 3))"
        ),
        "1:2:3"
    );
    assert_eq!(
        out("function d(a, b = a + 1) { return a + b; } console.log(d(1), d(1, 5))"),
        "3 6"
    );
    assert_eq!(
        out(
            "function Pt(x) { this.x = x; } Pt.prototype.get = function () { return this.x; }; const p = new Pt(7); console.log(p.get(), p instanceof Pt, p.constructor === Pt)"
        ),
        "7 true true"
    );
    assert_eq!(
        out(
            "function f() { return this.v; } const o = { v: 9 }; console.log(f.call(o), f.apply(o, []))"
        ),
        "9 9"
    );
    assert_eq!(
        out("function depth(n) { return n == 0 ? 0 : 1 + depth(n - 1); } console.log(depth(5000))"),
        "5000"
    );
    assert_eq!(
        out("console.log(typeof hoisted); function hoisted() {}"),
        "function"
    );
}

#[test]
fn loop_bindings_per_iteration() {
    assert_eq!(
        out("var f = []; for (var i = 0; i < 3; i++) f[i] = () => i; console.log(f[0](), f[2]())"),
        "3 3"
    );
    assert_eq!(
        out(
            "var f = []; for (let i = 0; i < 3; i++) { f[i] = () => i; } console.log(f[0](), f[1](), f[2]())"
        ),
        "0 1 2"
    );
    assert_eq!(
        out("var f = []; for (const v of [7, 8]) f.push(() => v); console.log(f[0](), f[1]())"),
        "7 8"
    );
    assert_eq!(
        out(
            "var f = []; for (const k in { a: 1, b: 2 }) f.push(() => k); console.log(f[0](), f[1]())"
        ),
        "a b"
    );
}

#[test]
fn control_flow() {
    assert_eq!(
        out(
            "function sw(x) { let r = ''; switch (x) { case 1: r += '1'; case 2: r += '2'; break; default: r += 'd'; case 3: r += '3'; } return r; } console.log(sw(1), sw(2), sw(3), sw(9))"
        ),
        "12 2 3 d3"
    );
    assert_eq!(
        out(
            "outer: for (var i = 0; i < 3; i++) { for (var j = 0; j < 3; j++) { if (j == 1) continue outer; if (i == 2) break outer; console.log('' + i + j); } }"
        ),
        "00|10"
    );
    assert_eq!(
        out("lbl: { console.log('in'); break lbl; console.log('never'); }"),
        "in"
    );
    assert_eq!(
        out("let d = 0; do { d++; if (d < 3) continue; } while (d < 5); console.log(d)"),
        "5"
    );
    assert_eq!(
        out("let w = 0; while (true) { w++; if (w == 4) break; } console.log(w)"),
        "4"
    );
    assert_eq!(
        out("let s = 0; for (const v of [1, 2, 3]) s += v; console.log(s)"),
        "6"
    );
    assert_eq!(
        out(
            "const ks = []; for (const k in { x: 1, y: 2, 3: 0 }) ks.push(k); console.log(ks.length, ks[0], ks[1], ks[2])"
        ),
        "3 3 x y"
    );
    assert_eq!(
        out("switch (1) { case 1: let sl = 's'; console.log(sl); }"),
        "s"
    );
    assert_eq!(
        out("console.log(null?.x, ({}).q ?? 'dflt', (void 0)?.[1], ({ f() { return 1; } }).f?.())"),
        "undefined dflt undefined 1"
    );
    assert_eq!(
        out("let a = null; a ??= 1; let b = 1; b &&= 2; let c = 0; c ||= 3; console.log(a, b, c)"),
        "1 2 3"
    );
}

#[test]
fn exceptions() {
    assert_eq!(
        out(
            "try { throw new TypeError('boom'); } catch (e) { console.log(e.name, e.message, e instanceof TypeError, e instanceof Error); }"
        ),
        "TypeError boom true true"
    );
    assert_eq!(
        out(
            "function f() { try { return 'try'; } finally { console.log('fin'); } } console.log(f())"
        ),
        "fin|try"
    );
    assert_eq!(
        out(
            "function f() { for (let i = 0; i < 3; i++) { try { if (i == 1) break; } finally { console.log('f' + i); } } return 'done'; } console.log(f())"
        ),
        "f0|f1|done"
    );
    assert_eq!(
        out(
            "function f() { try { throw new Error('x'); } catch (e) { return 'caught ' + e.message; } finally { console.log('fin'); } } console.log(f())"
        ),
        "fin|caught x"
    );
    assert_eq!(
        out(
            "function f() { try { try { throw 1; } finally { console.log('inner'); } } catch (e) { return 'outer ' + e; } } console.log(f())"
        ),
        "inner|outer 1"
    );
    assert_eq!(
        out("function f() { try { return 1; } finally { return 2; } } console.log(f())"),
        "2"
    );
    assert_eq!(
        out(
            "let w = 0; while (true) { try { w++; if (w > 2) break; } finally { w += 10; } } console.log(w)"
        ),
        "22"
    );
    assert_eq!(
        out("let cf; try { throw 'E'; } catch (err) { cf = () => err; } console.log(cf())"),
        "E"
    );
    assert_eq!(
        out("try { null.x; } catch (e) { console.log(e.message); }"),
        "Cannot read properties of null (reading 'x')"
    );
    assert_eq!(
        out("try { (function inf() { inf(); })(); } catch (e) { console.log(e.name); }"),
        "RangeError"
    );
    assert_eq!(fails("throw new RangeError('r')"), "RangeError: r");
    let e = error("function thrower() {\n  throw new Error('deep');\n}\nthrower();");
    assert_eq!(e.message(), "Error: deep");
    assert_eq!(e.stack()[0].function, "thrower");
    assert_eq!(e.stack()[0].line, 2);
}

#[test]
fn tdz() {
    assert_eq!(
        out("try { x; let x = 1; } catch (e) { console.log(e.name, e.message); }"),
        "ReferenceError Cannot access 'x' before initialization"
    );
    assert_eq!(
        out(
            "function f() { try { return y; } catch (e) { return e.message; } let y; } console.log(f())"
        ),
        "Cannot access 'y' before initialization"
    );
    assert_eq!(
        out(
            "function f() { g(); let a = 1; function g() { return a; } } try { f(); } catch (e) { console.log(e.name); }"
        ),
        "ReferenceError"
    );
    assert_eq!(
        fails("let t = t;"),
        "ReferenceError: Cannot access 't' before initialization"
    );
    assert_eq!(
        fails("const c = 1; c = 2;"),
        "TypeError: Assignment to constant variable 'c'."
    );
    assert_eq!(
        fails("function f() { const c = 1; c = 2; } f()"),
        "TypeError: Assignment to constant variable 'c'."
    );
    assert_eq!(
        out(
            "switch (1) { case 0: let q = 1; case 1: try { q; } catch (e) { console.log(e.name); } }"
        ),
        "ReferenceError"
    );
}

#[test]
fn sloppy_and_strict_this() {
    assert_eq!(
        out(
            "function f() { return this; } console.log(f() === globalThis, typeof f.call(5), f.call(null) === globalThis)"
        ),
        "true object true"
    );
    assert_eq!(
        out(
            "function f() { 'use strict'; return this; } console.log(f() === undefined, f.call(5))"
        ),
        "true 5"
    );
    assert_eq!(
        out("const o = { f() { return () => this; } }; console.log(o.f()() === o)"),
        "true"
    );
    assert_eq!(
        out("console.log(this === globalThis, (() => this)() === globalThis)"),
        "true true"
    );
}

#[test]
fn sloppy_and_strict_implicit_globals() {
    assert_eq!(
        out("function mk() { implicitG = 42; } mk(); console.log(implicitG, globalThis.implicitG)"),
        "42 42"
    );
    assert_eq!(
        fails("function f() { 'use strict'; undeclared = 1 } f()"),
        "ReferenceError: undeclared is not defined"
    );
    assert_eq!(
        fails("'use strict'; nope = 1"),
        "ReferenceError: nope is not defined"
    );
    // Failed writes: quiet in sloppy code, a TypeError in strict code.
    assert_eq!(out("NaN = 1; console.log(NaN !== NaN)"), "true");
    assert_eq!(
        fails("'use strict'; NaN = 1"),
        "TypeError: Cannot assign to read only property 'NaN' of object"
    );
    assert_eq!(out("'x'.y = 1; console.log('ok')"), "ok");
    assert_eq!(
        fails("'use strict'; 'x'.y = 1"),
        "TypeError: Cannot create property 'y' on string 'x'"
    );
    assert_eq!(out("console.log(delete [].length)"), "false");
    assert_eq!(
        fails("'use strict'; delete [].length"),
        "TypeError: Cannot delete property 'length' of [object Array]"
    );
}

#[test]
fn sloppy_and_strict_arguments() {
    assert_eq!(
        out(
            "function f(a, b) { arguments[0] = 10; b = 20; return a + ' ' + arguments[1] + ' ' + arguments.length; } console.log(f(1, 2, 3))"
        ),
        "10 20 3"
    );
    assert_eq!(
        out("function f(a) { 'use strict'; arguments[0] = 10; return a; } console.log(f(1))"),
        "1"
    );
    assert_eq!(
        out("function f(a) { 'use strict'; a = 5; return arguments[0]; } console.log(f(1))"),
        "1"
    );
    assert_eq!(
        out("function f(a = 0) { a = 5; return arguments[0]; } console.log(f(1))"),
        "1"
    );
    assert_eq!(
        out("function f(a) { delete arguments[0]; a = 3; return arguments[0]; } console.log(f(1))"),
        "undefined"
    );
    assert_eq!(
        out("function f() { return (() => arguments[0])(); } console.log(f('x'))"),
        "x"
    );
    assert_eq!(
        out("function f() { return arguments.length; } console.log(f(), f(1, 2))"),
        "0 2"
    );
    assert_eq!(
        out("function f() { return arguments.callee === f; } console.log(f())"),
        "true"
    );
    assert_eq!(
        out("function f() { return [...arguments]; } console.log(f(1, 2))"),
        "[ 1, 2 ]"
    );
}

#[test]
fn annex_b_block_functions() {
    assert_eq!(
        out("{ function annex() { return 'annex'; } } console.log(annex())"),
        "annex"
    );
    assert_eq!(
        out(
            "function f() { if (true) { function g() { return 1; } } return typeof g; } console.log(f())"
        ),
        "function"
    );
    assert_eq!(
        out("function f() { 'use strict'; { function g() {} } return typeof g; } console.log(f())"),
        "undefined"
    );
    assert_eq!(
        out("function f() { let g = 1; { function g() {} } return g; } console.log(f())"),
        "1"
    );
    assert_eq!(
        out(
            "function f() { console.log(typeof g); { function g() {} } console.log(typeof g); } f()"
        ),
        "undefined|function"
    );
    assert_eq!(
        out("if (true) function h() { return 2; } console.log(h())"),
        "2"
    );
}

#[test]
fn global_let_const_and_var() {
    assert_eq!(
        out(
            "let gl = 1; const gc = 2; var gv = 3; console.log(typeof gl, gc, globalThis.gl, globalThis.gv)"
        ),
        "number 2 undefined 3"
    );
    assert_eq!(
        out("console.log(typeof notDeclared, typeof alsoNotDeclared)"),
        "undefined undefined"
    );
    assert_eq!(
        fails("notDeclared"),
        "ReferenceError: notDeclared is not defined"
    );
    assert_eq!(
        out("var x1 = 1; { let x1 = 2; console.log(x1); } console.log(x1)"),
        "2|1"
    );
}

#[test]
fn scripts_share_the_global_scope() {
    use anvil_js::compiler::run_script;
    use anvil_js::vm::BasicHost;
    let mut vm = compile_support::vm_with(Default::default());
    let mut host = BasicHost::default();
    run_script(
        &mut vm,
        &mut host,
        "let shared = 1; var v = 2; function f() { return shared + v; }",
    )
    .unwrap();
    run_script(&mut vm, &mut host, "shared += 10; console.log(f())").unwrap();
    let e = run_script(&mut vm, &mut host, "let shared = 3;").unwrap_err();
    assert_eq!(
        e.message(),
        "SyntaxError: Identifier 'shared' has already been declared"
    );
    let e = run_script(&mut vm, &mut host, "let v = 3;").unwrap_err();
    assert_eq!(
        e.message(),
        "SyntaxError: Identifier 'v' has already been declared"
    );
    let e = run_script(&mut vm, &mut host, "var shared;").unwrap_err();
    assert_eq!(
        e.message(),
        "SyntaxError: Identifier 'shared' has already been declared"
    );
    assert_eq!(host.console[0].1, "13");
}

#[test]
fn eval_direct_and_indirect() {
    assert_eq!(
        out("var ev = eval('var evx = 5; evx * 2'); console.log(ev, evx)"),
        "10 5"
    );
    assert_eq!(out("let lx = 1; console.log(eval('lx + 1'))"), "2");
    assert_eq!(
        out("function f() { return (0, eval)('1 + 1'); } console.log(f())"),
        "2"
    );
    assert_eq!(
        out(
            "var g = 'global'; function f() { var g = 'local'; return (0, eval)('g'); } console.log(f())"
        ),
        "global"
    );
    assert_eq!(
        out("console.log(eval(42), eval('\\'use strict\\'; var sv = 1; typeof sv'), typeof sv)"),
        "42 number undefined"
    );
    assert_eq!(
        out("'use strict'; eval('var inStrict = 1'); console.log(typeof inStrict)"),
        "undefined"
    );
    assert_eq!(
        out("eval('let el = 1'); console.log(typeof el)"),
        "undefined"
    );
    assert_eq!(
        out(
            "try { eval('let a = ;'); } catch (e) { console.log(e.name, e instanceof SyntaxError); }"
        ),
        "SyntaxError true"
    );
    assert_eq!(
        out("var e2 = eval; console.log(e2('typeof e2'))"),
        "function"
    );
    assert_eq!(
        out("function f() { var eval = (x) => 'shadow ' + x; return eval(1); } console.log(f())"),
        "shadow 1"
    );
}

#[test]
fn unsupported_features_say_so() {
    let cases = [
        (
            "with ({}) {}",
            "SyntaxError: 'with' statements are not supported",
        ),
        (
            "function* g() {}",
            "SyntaxError: generator functions are not supported yet",
        ),
        ("1n", "SyntaxError: BigInt literals are not supported yet"),
        (
            "for (const x of 5) {}",
            "TypeError: 5 is not iterable (only arrays, strings and arguments objects can be iterated so far)",
        ),
    ];
    for (src, msg) in cases {
        assert_eq!(fails(src), msg, "{src}");
    }
    let direct = "SyntaxError: direct eval is only supported at the top level of a script, outside \
                  functions and blocks with their own declarations; use indirect eval, such as \
                  (0, eval)(code), which runs the code at global scope";
    assert_eq!(fails("function f() { eval('1'); }"), direct);
    assert_eq!(fails("{ let q = 1; eval('q'); }"), direct);
    let e = error("var a;\nwith (a) {}");
    match e {
        anvil_js::vm::RunError::Compile { span, .. } => assert_eq!(span.line, 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn classes_inheritance_private_and_static_blocks() {
    assert_eq!(
        out(r#"
            class Base {
                baseTag = 'B';
                constructor(x) {
                    this.x = x;
                }
                describe() {
                    return this.baseTag + ':' + this.x;
                }
                static kind() {
                    return 'base-kind';
                }
            }
            class Derived extends Base {
                #secret = 100;
                static count = 0;
                static {
                    this.count += 5;
                }
                constructor(x, y) {
                    super(x * 2);
                    this.y = y;
                    this.#secret += y;
                }
                #helper() {
                    return this.#secret;
                }
                get total() {
                    return this.x + this.#helper();
                }
                set total(v) {
                    this.#secret = v;
                }
                describe() {
                    return super.describe() + ':' + this.total;
                }
                static hasSecret(obj) {
                    return #secret in obj && #helper in obj;
                }
                static kind() {
                    return super.kind() + '+derived';
                }
            }
            const d = new Derived(3, 7);
            console.log(d instanceof Derived, d instanceof Base);
            console.log(d.describe(), Derived.count, Derived.kind());
            console.log(Derived.hasSecret(d), Derived.hasSecret({}));
            d.total = 50;
            console.log(d.total);
            function CtorProbe() {
                this.targetName = new.target ? new.target.name : 'none';
            }
            console.log(new CtorProbe().targetName);
        "#),
        "true true|B:6:113 5 base-kind+derived|true false|56|CtorProbe"
    );
}

#[test]
fn destructuring_patterns_everywhere() {
    assert_eq!(
        out(r#"
            const [a, , b = 9, ...rest] = [1, 2, undefined, 4, 5];
            console.log(a, b, rest.join(','));
            const { x, y: renamed = 20, nested: [n1, n2], ...rem } = {
                x: 10,
                nested: [30, 40],
                extra1: 'e1',
                extra2: 'e2'
            };
            console.log(x, renamed, n1, n2, Object.keys(rem).join(','), rem.extra1);
            let p = 1, q = 2;
            [p, q] = [q, p];
            console.log(p, q);
            function calc({ base = 10, mult = 2 } = {}, [first, ...others]) {
                return (base + first + others.length) * mult;
            }
            console.log(calc(undefined, [5, 6, 7]), calc({ base: 3, mult: 4 }, [2]));
            const pairs = [['k1', 1], ['k2', 2]];
            const acc = [];
            for (const [k, v] of pairs) {
                acc.push(k + '=' + v);
            }
            try {
                throw { code: 404, msg: 'missing' };
            } catch ({ code, msg }) {
                acc.push(code + ':' + msg);
            }
            console.log(acc.join('|'));
        "#),
        "1 9 4,5|10 20 30 40 extra1,extra2 e1|2 1|34 20|k1=1|k2=2|404:missing"
    );
}

#[test]
fn regexp_tagged_templates_and_async_await() {
    assert_eq!(
        out(r#"
            const re = /(\w+)-(\d+)/g;
            console.log(re.source, re.flags, re.global, re.test('item-42'));
            re.lastIndex = 0;
            const m = re.exec('a item-42 b ok-7');
            console.log(m[0], m[1], m[2], m.index);
            console.log('a item-42 b ok-7'.match(/(\w+)-(\d+)/g).join(','));
            console.log('a item-42 b ok-7'.replace(/(\w+)-(\d+)/g, '$2:$1'));
            console.log('a,b;c|d'.split(/[,;|]/).join('-'));
            function tag(strings, ...vals) {
                return strings.raw.join('#') + '|' + vals.join(',');
            }
            console.log(tag`hello\n${10}world${20}!`);
        "#),
        "(\\w+)-(\\d+) g true true|item-42 item 42 2|item-42,ok-7|a 42:item b 7:ok|a-b-c-d|hello\\n#world#!|10,20"
    );
    assert_eq!(
        out(r#"
            async function compute(x) {
                const a = await Promise.resolve(x + 1);
                const b = await (a * 3);
                let sum = 0;
                for await (const item of [Promise.resolve(2), 4, Promise.resolve(6)]) {
                    sum += item;
                }
                return b + sum;
            }
            compute(4).then(v => console.log(v));
        "#),
        "27"
    );
}

#[test]
fn console_formatting() {
    assert_eq!(
        out(
            "console.log('s', 1, -0, true, null, undefined, [1, 'a'], { 'a-b': 1, c: { d: [] } }, function nm() {}, () => 1)"
        ),
        "s 1 -0 true null undefined [ 1, 'a' ] { 'a-b': 1, c: { d: [] } } [Function: nm] [Function (anonymous)]"
    );
    assert_eq!(
        out("const o = {}; o.self = o; console.log(o)"),
        "{ self: [Circular] }"
    );
    assert_eq!(
        out("console.log(new Error('m'), [new TypeError('t')])"),
        "Error: m [ [TypeError: t] ]"
    );
    assert_eq!(
        out("function f() { return arguments; } console.log(f(1, 'x'))"),
        "[Arguments] [ 1, 'x' ]"
    );
    assert_eq!(
        out("console.log({ get a() { return 1; }, set b(v) {} })"),
        "{ a: [Getter], b: [Setter] }"
    );
}

#[test]
fn self_hosted_runtime_bundle_contains_web_and_dom_classes() {
    let rt = anvil_js::self_hosted_runtime_bundle();
    anvil_js::corejs::verify("self_hosted_runtime_bundle.js", rt)
        .expect("must verify as valid Core JS");
    assert!(rt.contains("class runtime_builtins$AnvilPromise"));
    assert!(rt.contains("class runtime_builtins$AnvilURL"));
    assert!(rt.contains("class runtime_web_apis$AnvilDOMDocument"));
    assert!(rt.contains("class runtime_web_apis$AnvilDOMParser"));
    assert!(rt.contains("class runtime_web_apis$AnvilCSSStyleSheet"));
    assert!(rt.contains("function runtime_builtins$anvilArrayToSorted("));
    assert!(rt.contains("function runtime_builtins$anvilObjectGroupBy("));
    assert!(rt.contains("function runtime_builtins$anvilPromiseWithResolvers("));
}

#[test]
fn es2022_to_es2024_stdlib_and_builtins() {
    assert_eq!(
        out(r#"
            const obj = { a: 1, b: 2 };
            console.log(Object.keys(obj).join(','), Object.values(obj).join(','));
            const cloned = Object.fromEntries(Object.entries(obj));
            console.log(cloned.a, cloned.b, Object.hasOwn(cloned, 'a'), Object.is(NaN, NaN), Object.is(+0, -0));
            const grouped = Object.groupBy([1, 2, 3, 4, 5], (n) => n % 2 === 0 ? 'even' : 'odd');
            console.log(grouped.even.join(','), grouped.odd.join(','));
        "#),
        "a,b 1,2|1 2 true true false|2,4 1,3,5"
    );
    assert_eq!(
        out(r#"
            const nums = [3, 1, 4, 1, 5];
            console.log(nums.toSorted().join(','), nums.toReversed().join(','), nums.join(','));
            console.log(nums.toSpliced(1, 2, 9, 8).join(','), nums.with(2, 99).join(','));
            console.log([1, [2, [3, 4]]].flat(2).join(','), [1, 2, 3].flatMap(x => [x, x * 10]).join(','));
            console.log(nums.findLast(x => x < 4), nums.findLastIndex(x => x === 1), nums.at(-1));
            console.log(nums.map(x => x * 2).filter(x => x > 4).reduce((a, b) => a + b, 0));
        "#),
        "1,1,3,4,5 5,1,4,1,3 3,1,4,1,5|3,9,8,1,5 3,1,99,1,5|1,2,3,4 1,10,2,20,3,30|1 3 5|24"
    );
    assert_eq!(
        out(r#"
            console.log('  hello world  '.trim().replaceAll('l', 'L'));
            console.log('abc'.padStart(5, '0'), 'abc'.padEnd(5, '.'), 'ha'.repeat(3));
            console.log(Math.trunc(-3.9), Math.sign(-42), Math.hypot(3, 4), Math.clz32(1), Math.imul(6, 7));
            const j = JSON.stringify({ x: [1, true, 'ok'], y: null });
            const p = JSON.parse(j);
            console.log(j, p.x[2], p.x.length);
        "#),
        "heLLo worLd|00abc abc.. hahaha|-3 -1 5 31 42|{\"x\":[1,true,\"ok\"],\"y\":null} ok 3"
    );
    assert_eq!(
        out(r#"
            const m = new Map([['k1', 10], ['k2', 20]]);
            m.set('k3', 30);
            const s = new Set([1, 2, 2, 3]);
            console.log(m.size, m.get('k2'), [...m.keys()].join(','), s.size, [...s].join(','));
            const copy = structuredClone({ nested: [1, { ok: true }] });
            copy.nested[1].ok = false;
            console.log(copy.nested[1].ok, btoa('Anvil'), atob(btoa('Anvil')));
            function add(a, b) { return this.base + a + b; }
            const bound = add.bind({ base: 100 }, 20);
            console.log(bound(3));
        "#),
        "3 20 k1,k2,k3 3 1,2,3|false QW52aWw= Anvil|123"
    );
    assert_eq!(
        out(r#"
            const log = [];
            Promise.resolve(10).then(x => x + 5).then(x => log.push('p:' + x));
            const wr = Promise.withResolvers();
            wr.promise.then(v => log.push('wr:' + v));
            wr.resolve('done');
            queueMicrotask(() => queueMicrotask(() => console.log(log.join(','))));
        "#),
        "wr:done,p:15"
    );
}
