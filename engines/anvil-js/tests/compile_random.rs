// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Seeded random programs.
//!
//! 1. Random expression programs over a few `let` variables, with
//!    closures, arrows, assignments inside expressions, ternaries and
//!    short-circuit operators. Each is run by the engine and by a small
//!    evaluator here, and the printed results must agree.
//! 2. Random token soup: whatever parses must compile and run (or fail
//!    with an error) without the engine panicking.

mod compile_support;
use anvil_js::limits::Limits;
use anvil_js::value::number::{number_to_string, to_int32};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Clone, Copy, Debug)]
enum V {
    Num(f64),
    Bool(bool),
}

impl V {
    fn num(self) -> f64 {
        match self {
            V::Num(n) => n,
            V::Bool(b) => f64::from(u8::from(b)),
        }
    }
    fn truthy(self) -> bool {
        match self {
            V::Num(n) => n != 0.0 && !n.is_nan(),
            V::Bool(b) => b,
        }
    }
    fn show(self) -> String {
        match self {
            V::Bool(b) => b.to_string(),
            V::Num(n) if n == 0.0 && n.is_sign_negative() => "-0".to_string(),
            V::Num(n) => number_to_string(n),
        }
    }
}

const VARS: usize = 4;

/// Build a random expression; evaluate it against `env` as we go, in
/// JavaScript's left-to-right order.
fn build(r: &mut Rng, depth: u32, env: &mut [V; VARS]) -> (String, V) {
    if depth == 0 || r.below(4) == 0 {
        return if r.below(3) == 0 {
            let i = r.below(VARS);
            (format!("v{i}"), env[i])
        } else {
            let n = (r.below(21) as f64) - 5.0;
            (format!("({n})"), V::Num(n))
        };
    }
    match r.below(12) {
        0..=4 => {
            let ops = [
                "+", "-", "*", "/", "%", "<", "===", "&", "|", "^", "<<", ">",
            ];
            let op = ops[r.below(ops.len())];
            let (a, x) = build(r, depth - 1, env);
            let (b, y) = build(r, depth - 1, env);
            let (xn, yn) = (x.num(), y.num());
            let v = match op {
                "+" => V::Num(xn + yn),
                "-" => V::Num(xn - yn),
                "*" => V::Num(xn * yn),
                "/" => V::Num(xn / yn),
                "%" => V::Num(xn % yn),
                "<" => V::Bool(xn < yn),
                ">" => V::Bool(xn > yn),
                "===" => V::Bool(match (x, y) {
                    (V::Num(p), V::Num(q)) => p == q,
                    (V::Bool(p), V::Bool(q)) => p == q,
                    _ => false,
                }),
                "&" => V::Num(f64::from(to_int32(xn) & to_int32(yn))),
                "|" => V::Num(f64::from(to_int32(xn) | to_int32(yn))),
                "^" => V::Num(f64::from(to_int32(xn) ^ to_int32(yn))),
                _ => V::Num(f64::from(
                    to_int32(xn).wrapping_shl((to_int32(yn) as u32) & 31),
                )),
            };
            (format!("({a} {op} {b})"), v)
        }
        5 => {
            let (t, tv) = build(r, depth - 1, env);
            if tv.truthy() {
                let (c, cv) = build(r, depth - 1, env);
                let mut scratch = *env;
                let (a, _) = build(r, depth - 1, &mut scratch);
                (format!("({t} ? {c} : {a})"), cv)
            } else {
                let mut scratch = *env;
                let (c, _) = build(r, depth - 1, &mut scratch);
                let (a, av) = build(r, depth - 1, env);
                (format!("({t} ? {c} : {a})"), av)
            }
        }
        6 => {
            let and = r.below(2) == 0;
            let (a, x) = build(r, depth - 1, env);
            let short = if and { !x.truthy() } else { x.truthy() };
            if short {
                let mut scratch = *env;
                let (b, _) = build(r, depth - 1, &mut scratch);
                (format!("({a} {} {b})", if and { "&&" } else { "||" }), x)
            } else {
                let (b, y) = build(r, depth - 1, env);
                (format!("({a} {} {b})", if and { "&&" } else { "||" }), y)
            }
        }
        7 => {
            let i = r.below(VARS);
            let (e, v) = build(r, depth - 1, env);
            env[i] = v;
            (format!("(v{i} = {e})"), v)
        }
        8 => {
            let (e, v) = build(r, depth - 1, env);
            (format!("(function () {{ return {e}; }})()"), v)
        }
        9 => {
            let (e, v) = build(r, depth - 1, env);
            (format!("(() => {e})()"), v)
        }
        10 => {
            let (e, v) = build(r, depth - 1, env);
            (format!("(-{e})"), V::Num(-v.num()))
        }
        _ => {
            let (e, v) = build(r, depth - 1, env);
            (format!("(!{e})"), V::Bool(!v.truthy()))
        }
    }
}

#[test]
fn random_expressions_match_the_reference() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..300 {
        let mut env = [V::Num(0.0); VARS];
        let mut src = String::new();
        let mut want = Vec::new();
        for (i, slot) in env.iter_mut().enumerate() {
            let n = (r.below(9) as f64) - 2.0;
            *slot = V::Num(n);
            src.push_str(&format!("let v{i} = {n};\n"));
        }
        let wrap = r.below(3);
        if wrap == 1 {
            src.push_str("function body() {\n");
        }
        for _ in 0..4 {
            let (e, v) = build(&mut r, 5, &mut env);
            src.push_str(&format!("console.log({e});\n"));
            want.push(v.show());
        }
        if wrap == 1 {
            src.push_str("}\nbody();\n");
        }
        let got = compile_support::run(&src).unwrap_or_else(|e| panic!("case {case}: {e}\n{src}"));
        assert_eq!(got, want.join("|"), "case {case}\n{src}");
    }
}

const TOKENS: &[&str] = &[
    "a",
    "b",
    "f",
    "x",
    "1",
    "0",
    "'s'",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    "+",
    "-",
    "*",
    "=",
    "==",
    "===",
    "<",
    "?",
    ":",
    ".",
    "=>",
    "let",
    "const",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "break",
    "continue",
    "try",
    "catch",
    "finally",
    "throw",
    "new",
    "this",
    "arguments",
    "typeof",
    "delete",
    "in",
    "of",
    "switch",
    "case",
    "default",
    "eval",
    "null",
    "undefined",
    "...",
    "?.",
    "??",
    "&&",
    "||",
    "!",
    "++",
    "--",
    "`a${",
    "}`",
    "\n",
    "label:",
    "'use strict';",
];

#[test]
fn random_token_soup_never_panics() {
    let limits = Limits {
        max_steps: Some(200_000),
        ..Limits::default()
    };
    let mut r = Rng(0xD1B5_4A32_D192_ED03);
    let mut parsed = 0;
    for _ in 0..4000 {
        let n = 1 + r.below(8);
        let src: Vec<&str> = (0..n).map(|_| TOKENS[r.below(TOKENS.len())]).collect();
        let src = src.join(" ");
        if anvil_js::parse_script(&src, &Default::default()).is_ok() {
            parsed += 1;
        }
        let _ = compile_support::run_with(limits.clone(), &src);
    }
    assert!(parsed > 150, "only {parsed} programs parsed");
}

#[test]
fn random_statement_programs_never_panic() {
    // Bigger, mostly valid programs built from statement templates.
    let parts = [
        "let a = 1;",
        "var b = [1, 2, 3];",
        "function f(x, y) { return x + (y || 0); }",
        "for (let i = 0; i < 3; i++) { if (i == 1) continue; b.push(i); }",
        "for (const k in b) { a += k; }",
        "for (const v of b) { a += v; }",
        "try { throw a; } catch (e) { a = e; } finally { a++; }",
        "switch (a % 3) { case 0: a++; break; case 1: a--; default: a = 0; }",
        "a = f(a, b.length) ? a : -a;",
        "(function () { arguments[0] = 1; })(a);",
        "{ function g() { return a; } a = g(); }",
        "outer: while (true) { do { break outer; } while (false); }",
        "let o = { p: a, get q() { return this.p; }, ['r' + a]: 1 };",
        "a = o?.q ?? a;",
        "eval('a + 1');",
        "(0, eval)('var ev = 1');",
        "console.log(a, b, typeof o);",
        "a = `t${a}`;",
        "delete o.p;",
        "x = 1;",
        "'use strict'; y = 1;",
    ];
    let limits = Limits {
        max_steps: Some(500_000),
        ..Limits::default()
    };
    let mut r = Rng(0x2545_F491_4F6C_DD1D);
    for _ in 0..500 {
        let n = 1 + r.below(12);
        let src: String = (0..n)
            .map(|_| parts[r.below(parts.len())])
            .collect::<Vec<_>>()
            .join("\n");
        let _ = compile_support::run_with(limits.clone(), &src);
        // And a damaged copy: a random slice cut out.
        let chars: Vec<char> = src.chars().collect();
        let at = r.below(chars.len());
        let len = r.below(8).min(chars.len() - at);
        let cut: String = chars[..at].iter().chain(&chars[at + len..]).collect();
        let _ = compile_support::run_with(limits.clone(), &cut);
    }
}
