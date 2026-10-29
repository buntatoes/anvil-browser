// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Size and depth limits. Every limit must come back as an error with a
//! message, never as a crash, a hang, or a silent cutoff.

#![forbid(unsafe_code)]

use anvil_js::ast::{Expr, ExprKind, UnaryOp};
use anvil_js::{DEFAULT_MAX_DEPTH, ParseOptions, Span, parse_module, parse_script};

/// Nesting shapes that each push the parser one level deeper per repeat.
fn shape(name: &str, n: usize) -> String {
    match name {
        "paren" => format!("{}a{}", "(".repeat(n), ")".repeat(n)),
        "array" => format!("{}{}", "[".repeat(n), "]".repeat(n)),
        "object" => format!("{}1{}", "({a:".repeat(n), "})".repeat(n)),
        "block" => format!("{}{}", "{".repeat(n), "}".repeat(n)),
        "if" => format!("{}x", "if(a)".repeat(n)),
        "while" => format!("{}x", "while(a)".repeat(n)),
        "unary" => format!("{}a", "!".repeat(n)),
        "assign" => format!("{}1", "a=".repeat(n)),
        "cond" => format!("{}1", "a?b:".repeat(n)),
        "arrow" => format!("{}1", "()=>".repeat(n)),
        "function" => format!("{}{}", "function f(){".repeat(n), "}".repeat(n)),
        "template" => format!("{}x{}", "`${".repeat(n), "}`".repeat(n)),
        "class" => format!("{}{}", "(class{m(){return ".repeat(n), "}})".repeat(n)),
        "new" => format!("{}A", "new ".repeat(n)),
        "exponent" => format!("{}2", "2**".repeat(n)),
        "binding" => format!("let {}a{} = x", "[".repeat(n), "]".repeat(n)),
        "member_chain" => format!("a{}", ".b".repeat(n)),
        "call_chain" => format!("a{}", "()".repeat(n)),
        "addition" => format!("1{}", "+1".repeat(n)),
        "comma_free_seq" => "x;".repeat(n),
        _ => panic!("unknown shape {name}"),
    }
}

const SHAPES: &[&str] = &[
    "paren",
    "array",
    "object",
    "block",
    "if",
    "while",
    "unary",
    "assign",
    "cond",
    "arrow",
    "function",
    "template",
    "class",
    "new",
    "exponent",
    "binding",
    "member_chain",
    "call_chain",
    "addition",
];

fn depth_error(src: &str) -> bool {
    match parse_script(src, &ParseOptions::default()) {
        Err(e) => e.message.contains("nesting is too deep"),
        Ok(_) => false,
    }
}

#[test]
fn every_shape_stops_at_the_depth_limit() {
    for name in SHAPES {
        let src = shape(name, DEFAULT_MAX_DEPTH as usize + 50);
        assert!(depth_error(&src), "{name}: expected a depth error");
    }
}

#[test]
fn every_shape_far_past_the_limit_still_errors() {
    for name in SHAPES {
        let src = shape(name, 20_000);
        assert!(depth_error(&src), "{name}: expected a depth error");
    }
}

#[test]
fn moderate_nesting_parses() {
    for name in SHAPES {
        let src = shape(name, 60);
        if let Err(e) = parse_script(&src, &ParseOptions::default()) {
            panic!("{name}: {e}");
        }
    }
}

#[test]
fn flat_statement_lists_are_not_limited() {
    let src = shape("comma_free_seq", 50_000);
    let p = parse_script(&src, &ParseOptions::default()).expect("flat list");
    assert_eq!(p.body.len(), 50_000);
}

#[test]
fn limit_is_configurable() {
    let opts = ParseOptions {
        max_depth: 20,
        ..ParseOptions::default()
    };
    assert!(parse_script(&shape("paren", 10), &opts).is_ok());
    let e = parse_script(&shape("paren", 30), &opts).unwrap_err();
    assert!(e.message.contains("limit 20"), "{e}");
}

#[test]
fn works_from_a_tiny_caller_stack() {
    // The parser runs on its own thread, so the caller's stack size
    // doesn't matter.
    let handle = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| depth_error(&shape("paren", 15_000)))
        .expect("spawn");
    assert!(handle.join().expect("join"));
}

#[test]
fn source_size_limit() {
    let opts = ParseOptions {
        max_source_bytes: 10,
        ..ParseOptions::default()
    };
    let e = parse_script("var abcdefgh = 1;", &opts).unwrap_err();
    assert!(e.message.contains("byte limit"), "{e}");
    assert!(parse_script("var a=1;", &opts).is_ok());
    assert!(parse_module("var abcdefgh = 1;", &opts).is_err());
}

#[test]
fn default_limits_are_documented_values() {
    let d = ParseOptions::default();
    assert_eq!(d.max_depth, DEFAULT_MAX_DEPTH);
    assert_eq!(d.max_source_bytes, anvil_js::DEFAULT_MAX_SOURCE_BYTES);
}

#[test]
fn dropping_a_very_deep_tree_does_not_recurse() {
    let mut e = Expr {
        kind: ExprKind::Ident("x".into()),
        span: Span::default(),
    };
    for _ in 0..1_000_000 {
        e = Expr {
            kind: ExprKind::Unary {
                op: UnaryOp::Not,
                arg: Box::new(e),
            },
            span: Span::default(),
        };
    }
    drop(e);
}

#[test]
fn deep_chain_under_the_limit_parses_and_drops() {
    let src = shape("member_chain", DEFAULT_MAX_DEPTH as usize - 10);
    let p = parse_script(&src, &ParseOptions::default()).expect("chain");
    drop(p);
}
