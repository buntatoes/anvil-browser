// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Limits hit by compiled programs, each with its exact message.

mod compile_support;
use anvil_js::compiler::{compile_script, run_script};
use anvil_js::limits::{LimitKind, Limits};
use anvil_js::vm::{BasicHost, RunError};
use compile_support::{out, run_with, vm_with};
use std::time::{Duration, Instant};

fn abort(limits: Limits, src: &str) -> (LimitKind, String) {
    let mut vm = vm_with(limits);
    let mut host = BasicHost::default();
    match run_script(&mut vm, &mut host, src) {
        Err(RunError::Abort { kind, reason, .. }) => (kind, reason),
        other => panic!("expected an abort, got {other:?}"),
    }
}

#[test]
fn compile_depth() {
    let lim = Limits {
        max_compile_depth: 50,
        ..Limits::default()
    };
    let src = format!("{}1{}", "(".repeat(60), ")".repeat(60));
    assert_eq!(
        run_with(lim.clone(), &src).unwrap_err(),
        "SyntaxError: nesting too deep to compile: more than 50 levels (max_compile_depth)"
    );
    let src = format!("{}{}", "{".repeat(60), "}".repeat(60));
    let e = compile_script(&src, &lim).unwrap_err();
    assert!(e.message.ends_with("(max_compile_depth)"));
    let bad = Limits {
        max_compile_depth: 0,
        ..Limits::default()
    };
    assert_eq!(
        bad.validate().unwrap_err(),
        "max_compile_depth is 0, it must be between 1 and 100000"
    );
}

#[test]
fn source_bytes() {
    let lim = Limits {
        max_source_bytes: 10,
        ..Limits::default()
    };
    assert_eq!(
        run_with(lim.clone(), "var x = 12345678;").unwrap_err(),
        "SyntaxError: script is 17 bytes, limit 10 bytes (max_source_bytes)"
    );
}

#[test]
fn eval_code_over_the_source_limit_is_a_catchable_syntax_error() {
    let lim = Limits {
        max_source_bytes: 100,
        ..Limits::default()
    };
    let src =
        "var s='xx';for(var i=0;i<6;i++)s+=s;try{eval(s)}catch(e){console.log(e.name,e.message)}";
    assert!(src.len() <= 100);
    assert_eq!(
        run_with(lim, src).unwrap(),
        "SyntaxError script is 128 bytes, limit 100 bytes (max_source_bytes)"
    );
}

#[test]
fn registers() {
    let mut many = String::new();
    for i in 0..70_000 {
        many.push_str(&format!("var v{i} = {i};"));
    }
    let src = format!("function f() {{ {many} }}");
    assert_eq!(
        compile_support::run(&src).unwrap_err(),
        "SyntaxError: function needs more than 65,535 registers (max_registers_per_function)"
    );
}

#[test]
fn step_budget_stops_a_loop() {
    let lim = Limits {
        max_steps: Some(100_000),
        ..Limits::default()
    };
    let (kind, reason) = abort(lim, "while (true) {}");
    assert_eq!(kind, LimitKind::Steps);
    assert!(reason.contains("100,000"), "{reason}");
}

#[test]
fn call_depth() {
    assert_eq!(
        out("try { (function inf() { inf(); })(); } catch (e) { console.log(e.message); }"),
        "Maximum call depth exceeded (10,000 frames) in function 'inf'"
    );
}

#[test]
fn nested_eval_counts_as_native_reentry() {
    // Each eval level nests the dispatch loop on the Rust stack.
    let src = "var n = 0; var code = 'n++; eval(code)'; \
               try { eval(code); } catch (e) { console.log(e.name, e.message, n); }";
    let out = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || compile_support::run(src))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        out.unwrap(),
        "RangeError Too many nested builtin callbacks (256) 256"
    );
}

#[test]
fn heap_limit_aborts() {
    let lim = Limits {
        max_heap_bytes: 16 << 20,
        initial_gc_trigger: 1 << 20,
        ..Limits::default()
    };
    let (kind, _) = abort(lim, "var a = []; while (true) a.push({ x: [1, 2, 3] });");
    assert_eq!(kind, LimitKind::HeapBytes);
}

#[test]
fn console_log_of_a_huge_sparse_array_is_quick() {
    let lim = Limits {
        max_steps: Some(1_000_000),
        ..Limits::default()
    };
    let t = Instant::now();
    let r = run_with(
        lim,
        "var a = []; a.length = 4294967295; console.log(a); a[7] = 1; console.log(a);",
    );
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "took {:?}",
        t.elapsed()
    );
    assert_eq!(
        r.unwrap(),
        "[ <4294967295 empty items> ]|[ <7 empty items>, 1, <4294967287 empty items> ]"
    );
}

#[test]
fn console_log_counts_steps() {
    // Formatting work counts against max_steps like any other work.
    let lim = Limits {
        max_steps: Some(2_000),
        ..Limits::default()
    };
    let src = "var a = []; for (var i = 0; i < 90; i++) a.push(i); \
               for (var j = 0; j < 100; j++) console.log(a);";
    let (kind, reason) = abort(lim, src);
    assert_eq!(kind, LimitKind::Steps);
    assert!(reason.contains("2,000"), "{reason}");
}

#[test]
fn inspect_depth_and_item_limits() {
    // INSPECT_DEPTH: nested deeper than two levels prints as [Array] or
    // [Object]. INSPECT_ITEMS: at most 100 entries, then a count.
    assert_eq!(
        out("console.log([[[[1]]]], { a: { b: { c: { d: 1 } } } })"),
        "[ [ [ [Array] ] ] ] { a: { b: { c: [Object] } } }"
    );
    let s = out("var a = []; for (var i = 0; i < 150; i++) a.push(i); console.log(a)");
    assert!(s.starts_with("[ 0, 1, 2,"), "{s}");
    assert!(s.ends_with("98, 99, ... 50 more items ]"), "{s}");
    let s = out("var o = {}; for (var i = 0; i < 150; i++) o['k' + i] = i; console.log(o)");
    assert!(s.ends_with("k99: 99, ... }"), "{s}");
}

#[test]
fn spread_size_is_capped() {
    // MAX_SPREAD: a sparse array claiming billions of elements can't be
    // spread or walked by for-of.
    assert_eq!(
        out(
            "var a = []; a.length = 4294967295; try { [...a]; } catch (e) { console.log(e.name, e.message); }"
        ),
        "RangeError Cannot spread 4,294,967,295 elements (at most 16,777,216)"
    );
    assert_eq!(
        out(
            "var a = []; a.length = 20000000; function f() {} try { f(...a); } catch (e) { console.log(e.name); }"
        ),
        "RangeError"
    );
}
