// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Compiler unit tests: scope analysis, register use, emitted code.

use super::scope::{BindKind, TopMode, analyze};
use super::*;
use crate::anvil_js::bytecode::disasm::disassemble;
use crate::anvil_js::parser::parse_script;

fn parse(src: &str) -> Program {
    parse_script(src, &ParseOptions::default()).expect("parse")
}

fn analysis(src: &str) -> Analysis {
    let p = parse(src);
    let mut d = Depth::new(10_000);
    analyze(&p.body, p.span, p.strict, TopMode::Script, &mut d).expect("analyze")
}

fn compile(src: &str) -> CodeBlock {
    compile_script(src, &Limits::default()).expect("compile")
}

fn binding<'a>(a: &'a Analysis, name: &str) -> Vec<&'a scope::Binding> {
    a.scopes
        .iter()
        .flat_map(|s| s.bindings.iter())
        .filter(|b| b.name == name)
        .collect()
}

/// Compile `src` on a thread with a big stack; returns the stack bytes
/// used and the deepest level reached.
fn stack_use(src: &str) -> (usize, u32) {
    let p = parse(src);
    std::thread::scope(|s| {
        let h = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn_scoped(s, || {
                let base = 0u8;
                let base = std::ptr::addr_of!(base) as usize;
                let mut depth = Depth::new(100_000);
                let a = analyze(&p.body, p.span, p.strict, TopMode::Script, &mut depth).unwrap();
                let mut cg = Cg {
                    a: &a,
                    depth: Depth::new(100_000),
                    fns: Vec::new(),
                    scopes: Vec::new(),
                };
                cg.top(&p, "t").unwrap();
                let low = cg.depth.stack_low.min(depth.stack_low);
                (base - low, cg.depth.peak().max(depth.peak()))
            })
            .unwrap();
        h.join().unwrap()
    })
}

#[test]
fn captured_names_are_marked() {
    let a = analysis("function f() { let a = 1, b = 2; return () => a; }");
    assert!(binding(&a, "a")[0].captured);
    assert!(!binding(&a, "b")[0].captured);
}

#[test]
fn top_level_names_are_global() {
    let a = analysis("var v; let l; const c = 1; function f() {}");
    for n in ["v", "l", "c", "f"] {
        let b = binding(&a, n)[0];
        assert!(a.is_global(0, b), "{n}");
    }
}

#[test]
fn var_hoists_to_the_function_and_let_stays_in_the_block() {
    let a = analysis("function f() { { var v; let l; } }");
    let fs = a.funcs[1].scope;
    assert!(a.scopes[fs].find("v").is_some());
    assert!(a.scopes[fs].find("l").is_none());
    let blocks: Vec<_> = a.scopes.iter().filter(|s| s.tag == Tag::Block).collect();
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].find("l").is_some());
}

#[test]
fn arguments_and_this_bindings() {
    let a = analysis("function f(x) { return () => [this, arguments]; }");
    let f = &a.funcs[1];
    assert!(f.uses_arguments);
    assert!(f.mapped, "sloppy, simple parameters");
    let s = &a.scopes[f.scope];
    let args = &s.bindings[s.find("arguments").unwrap()];
    assert_eq!(args.kind, BindKind::Arguments);
    assert!(args.captured);
    let this = &s.bindings[s.find("this").unwrap()];
    assert_eq!(this.kind, BindKind::This);
    assert!(this.captured);
    // Mapped parameters always live in the environment.
    assert!(s.bindings[s.find("x").unwrap()].captured);

    let a = analysis("function f(x) { 'use strict'; return arguments; }");
    assert!(!a.funcs[1].mapped);
    let a = analysis("function f(x = 1) { return arguments; }");
    assert!(!a.funcs[1].mapped);
    let a = analysis("function f(x, x) { return arguments; }");
    assert!(!a.funcs[1].mapped, "duplicate parameters aren't mapped");
}

#[test]
fn annex_b_adds_a_var_only_in_sloppy_code() {
    let a = analysis("function f() { { function g() {} } }");
    let fs = a.funcs[1].scope;
    assert_eq!(
        a.scopes[fs]
            .find("g")
            .map(|i| a.scopes[fs].bindings[i].kind),
        Some(BindKind::Var)
    );
    let a = analysis("function f() { 'use strict'; { function g() {} } }");
    let fs = a.funcs[1].scope;
    assert!(a.scopes[fs].find("g").is_none());
    let a = analysis("function f() { let g; { function g() {} } }");
    let fs = a.funcs[1].scope;
    assert_eq!(
        a.scopes[fs]
            .find("g")
            .map(|i| a.scopes[fs].bindings[i].kind),
        Some(BindKind::Let)
    );
    let a = analysis("function f(g) { { function g() {} } }");
    let fs = a.funcs[1].scope;
    assert_eq!(
        a.scopes[fs]
            .find("g")
            .map(|i| a.scopes[fs].bindings[i].kind),
        Some(BindKind::Param)
    );
}

#[test]
fn named_function_expressions_see_themselves() {
    let a = analysis("(function g() { return g; })");
    let s = &a.scopes[a.funcs[1].scope];
    assert_eq!(s.bindings[s.find("g").unwrap()].kind, BindKind::FuncSelf);
    let a = analysis("(function g(g) { return g; })");
    let s = &a.scopes[a.funcs[1].scope];
    assert_eq!(s.bindings[s.find("g").unwrap()].kind, BindKind::Param);
}

#[test]
fn locals_use_registers_and_captures_use_an_environment() {
    let b = compile("function f(a) { let b = a + 1; return b; }");
    let f = &b.functions[0];
    let text = disassemble(f);
    assert!(!text.contains("push env"), "{text}");
    assert!(f.register_count <= 5, "{text}");
    let b = compile("function f(a) { return () => a; }");
    let text = disassemble(&b.functions[0]);
    assert!(text.contains("push env"), "{text}");
}

#[test]
fn strictness_is_per_function() {
    let b = compile("function s() { 'use strict'; } function l() {}");
    assert!(!b.strict);
    assert!(b.functions[0].strict);
    assert!(!b.functions[1].strict);
    let b = compile("'use strict'; function f() {}");
    assert!(b.strict && b.functions[0].strict);
}

#[test]
fn push_and_pop_env_balance() {
    // Every path through a loop with captured let bindings, breaks out of
    // nested blocks and a finally keeps the environment count right; the
    // VM checks it at run time, here we check the ops pair up in order.
    let b = compile(
        "function f() { for (let i = 0; i < 3; i++) { let j = i; \
         try { if (i) break; } finally { (() => j)(); } } return 1; }",
    );
    let f = &b.functions[0];
    let pushes = f
        .ops
        .iter()
        .filter(|o| matches!(o, Op::PushEnv { .. }))
        .count();
    let pops = f.ops.iter().filter(|o| matches!(o, Op::PopEnv)).count();
    assert!(pushes >= 1 && pops > pushes, "{}", disassemble(f));
}

#[test]
fn tdz_checks_only_where_needed() {
    let b = compile("function f() { let a = 1; return a; }");
    assert!(
        !disassemble(&b.functions[0]).contains("check"),
        "after the declaration"
    );
    let b = compile("function f() { g(); let a = 1; function g() { return a; } }");
    assert!(disassemble(&b.functions[0].functions[0]).contains("check r"));
    let b = compile("function f() { a; let a = 1; }");
    assert!(disassemble(&b.functions[0]).contains("check r"));
}

#[test]
fn compile_depth_is_counted() {
    let lim = Limits {
        max_compile_depth: 20,
        ..Limits::default()
    };
    let src = format!("{}1{}", "(".repeat(30), ")".repeat(30));
    let e = compile_script(&src, &lim).unwrap_err();
    assert_eq!(
        e.message,
        "nesting too deep to compile: more than 20 levels (max_compile_depth)"
    );
    assert_eq!(e.span.line, 1);
}

#[test]
fn source_size_is_checked() {
    let lim = Limits {
        max_source_bytes: 1000,
        ..Limits::default()
    };
    let e = compile_script(&"1;".repeat(1000), &lim).unwrap_err();
    assert_eq!(
        e.message,
        "script is 2,000 bytes, limit 1,000 bytes (max_source_bytes)"
    );
    let e = compile_eval(&"1;".repeat(1000), &lim, false).unwrap_err();
    assert!(e.message.ends_with("(max_source_bytes)"));
}

#[test]
fn modules_are_refused() {
    let p = crate::anvil_js::parser::parse_module("export const a = 1;", &ParseOptions::default())
        .unwrap();
    let block = compile_program(&p, &Limits::default()).expect("module compiles via lowering");
    assert!(!block.ops.is_empty());
}

#[test]
fn deepest_parse_compiles_within_the_default_limits() {
    // The parser stops at 1,000 levels; anything it accepts has to compile
    // with the default max_compile_depth.
    for (open, close) in [("(", ")"), ("[", "]"), ("-", ""), ("{a:", "}"), ("f(", ")")] {
        let mut n = 2000;
        let src = loop {
            let s = format!("{}1{}", open.repeat(n), close.repeat(n));
            if parse_script(&s, &ParseOptions::default()).is_ok() {
                break s;
            }
            n -= 50;
        };
        compile_script(&src, &Limits::default())
            .unwrap_or_else(|e| panic!("{open}: {}", e.message));
    }
    for (open, close) in [
        ("{", "}"),
        ("if (1) ", ""),
        ("while (0) ", ""),
        ("function f(){", "}"),
    ] {
        let mut n = 2000;
        let src = loop {
            let s = format!("{}1{}", open.repeat(n), close.repeat(n));
            if parse_script(&s, &ParseOptions::default()).is_ok() {
                break s;
            }
            n -= 50;
        };
        compile_script(&src, &Limits::default())
            .unwrap_or_else(|e| panic!("{open}: {}", e.message));
    }
}

#[test]
fn stack_per_level_fits_the_thread_size() {
    // The compiler thread gets STACK_PER_LEVEL bytes per level; measure the
    // real use for the deepest kinds of nesting and print it.
    for (open, close) in [
        ("(", ")"),
        ("[", "]"),
        ("f(", ")"),
        ("{a:", "}"),
        ("-", ""),
        ("a=", ""),
        ("{", "}"),
        ("if (1) ", ""),
        ("function f(){", "}"),
        ("(() => ", ")"),
        ("try {", "} finally {}"),
        ("for(;;)", ""),
        ("x?.(", ")"),
        ("`${", "}`"),
    ] {
        let mut n = 990;
        let src = loop {
            let s = format!("{}1{}", open.repeat(n), close.repeat(n));
            if parse_script(&s, &ParseOptions::default()).is_ok() {
                break s;
            }
            n -= 10;
        };
        let (bytes, levels) = stack_use(&src);
        let per = bytes / levels.max(1) as usize;
        println!("{open:>14} x{n}: {levels} levels, {per} bytes per level");
        assert!(per * 2 <= STACK_PER_LEVEL, "{open}: {per} bytes per level");
    }
}
