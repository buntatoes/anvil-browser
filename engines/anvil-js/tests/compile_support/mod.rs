// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Shared helpers for the compiler tests: run source text with the basic
//! globals and read back the console output or the error.
#![allow(dead_code)]

use anvil_js::compiler::{install, run_script};
use anvil_js::limits::Limits;
use anvil_js::vm::{BasicHost, RunError, Vm};

/// A VM with the basic globals installed.
pub fn vm_with(limits: Limits) -> Vm {
    let mut vm = Vm::new(limits).expect("vm");
    install(&mut vm).expect("install");
    vm
}

/// Run `src`; console lines joined with `|`, or the error message.
pub fn run_with(limits: Limits, src: &str) -> Result<String, String> {
    let mut vm = vm_with(limits);
    let mut host = BasicHost::default();
    let r = run_script(&mut vm, &mut host, src);
    let out: Vec<String> = host.console.iter().map(|(_, t)| t.clone()).collect();
    match r {
        Ok(_) => Ok(out.join("|")),
        Err(e) => Err(e.message().to_string()),
    }
}

/// Run `src` with default limits.
pub fn run(src: &str) -> Result<String, String> {
    run_with(Limits::default(), src)
}

/// Console output of `src`, panicking on an error.
pub fn out(src: &str) -> String {
    match run(src) {
        Ok(s) => s,
        Err(e) => panic!("script failed: {e}\n--- source ---\n{src}"),
    }
}

/// The error message of `src`, panicking if it ran fine.
pub fn fails(src: &str) -> String {
    match run(src) {
        Ok(s) => panic!("script should have failed, printed {s:?}\n--- source ---\n{src}"),
        Err(e) => e,
    }
}

/// The completion value of `src` as display text.
pub fn value(src: &str) -> String {
    let mut vm = vm_with(Limits::default());
    let mut host = BasicHost::default();
    match run_script(&mut vm, &mut host, src) {
        Ok(v) => vm.inspect(v).unwrap_or_else(|e| e.message().to_string()),
        Err(e) => panic!("script failed: {}", e.message()),
    }
}

/// The error of `src`, as the `RunError`.
pub fn error(src: &str) -> RunError {
    let mut vm = vm_with(Limits::default());
    let mut host = BasicHost::default();
    run_script(&mut vm, &mut host, src).expect_err("should fail")
}
