// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Runs the JavaScript probe programs from the scaling survey when they
//! are present: the directory named by `ANVIL_PROBES`, or else
//! `../js-scale/probes` next to this crate. Each `name.js` should print
//! what `name.expect` holds. Without the directory the test says so and
//! skips. The test prints the pass rate and every failure
//! with its reason; it fails only if a probe makes the engine panic or a
//! probe that passed before stops passing (see `MUST_PASS`).

use cinder::compiler::{install, run_script};
use cinder::limits::Limits;
use cinder::vm::{BasicHost, Vm};
use std::path::PathBuf;

/// Probes known to pass. Losing one is a regression.
const MUST_PASS: &[&str] = &[
    "arguments_obj",
    "asi",
    "big_array",
    "bitwise",
    "block_scope",
    "closure_arrow",
    "closure_counter",
    "comma_ternary",
    "computed_keys",
    "default_params",
    "do_while",
    "equality",
    "error_types",
    "exponent",
    "getter_literal",
    "hoisting",
    "iife",
    "increment",
    "labeled",
    "logical_assign",
    "loop_perf",
    "nested_fn_scope",
    "obj_array_identity",
    "obj_identity",
    "optional_chain",
    "prototype",
    "recursion_depth",
    "regex_in_division",
    "rest_params",
    "shorthand_methods",
    "strict_mode",
    "string_build_perf",
    "switch_fallthrough",
    "tdz",
    "template",
    "typeof_fn",
    "undefined_type",
    "unicode_strings",
    "var_hoist",
];

fn run_probe(src: &str) -> (String, Option<String>) {
    let limits = Limits {
        max_steps: Some(50_000_000),
        ..Limits::default()
    };
    let mut vm = Vm::new(limits).expect("vm");
    install(&mut vm).expect("install");
    let mut host = BasicHost::default();
    let r = run_script(&mut vm, &mut host, src);
    let out: Vec<String> = host.console.iter().map(|(_, t)| t.clone()).collect();
    (out.join("\n"), r.err().map(|e| e.message().to_string()))
}

#[test]
fn probes() {
    let dir = match std::env::var_os("ANVIL_PROBES") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("js-scale")
            .join("probes"),
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!(
            "probe directory {} not found; set ANVIL_PROBES to run the probes. Skipped.",
            dir.display()
        );
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? == "js" {
                Some(p.file_stem()?.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    let mut passed = Vec::new();
    let mut failed = Vec::new();
    for name in &names {
        let src = std::fs::read_to_string(dir.join(format!("{name}.js"))).expect("read probe");
        let expect =
            std::fs::read_to_string(dir.join(format!("{name}.expect"))).unwrap_or_default();
        let expect = expect.trim_end_matches('\n');
        let src2 = src.clone();
        let (out, error) = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || run_probe(&src2))
            .expect("thread")
            .join()
            .unwrap_or_else(|_| panic!("probe {name} panicked"));
        if error.is_none() && out == expect {
            passed.push(name.clone());
        } else {
            let why = match error {
                Some(e) => e,
                None => format!("printed {out:?}, expected {expect:?}"),
            };
            failed.push((name.clone(), why));
        }
    }
    println!(
        "probes: {} of {} pass ({:.1}%)",
        passed.len(),
        names.len(),
        100.0 * passed.len() as f64 / names.len().max(1) as f64
    );
    println!("passing: {}", passed.join(" "));
    for (n, why) in &failed {
        println!("FAIL {n}: {why}");
    }
    for m in MUST_PASS {
        if names.iter().any(|n| n == m) {
            assert!(passed.iter().any(|p| p == m), "probe {m} used to pass");
        }
    }
}
