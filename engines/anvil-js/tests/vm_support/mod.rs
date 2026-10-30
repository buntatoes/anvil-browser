// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Shared helpers for the VM tests: building blocks by hand and reading
//! results back.

#![allow(dead_code)]

use anvil_js::bytecode::{CodeBlock, Op, Reg};
use anvil_js::limits::Limits;
use anvil_js::value::Value;
use anvil_js::vm::{BasicHost, CodeId, RunError, Vm};

/// A fresh VM with default limits.
pub fn vm() -> Vm {
    Vm::new(Limits::default()).expect("default limits work")
}

/// A fresh VM with some limits changed.
pub fn vm_with(limits: Limits) -> Vm {
    Vm::new(limits).expect("limits work")
}

/// Load and run a block in a fresh VM.
pub fn run(block: CodeBlock) -> (Vm, Result<Value, RunError>) {
    let mut vm = vm();
    let r = run_in(&mut vm, block);
    (vm, r)
}

/// Load and run a block.
pub fn run_in(vm: &mut Vm, block: CodeBlock) -> Result<Value, RunError> {
    let id = vm.load(block)?;
    vm.run(&mut BasicHost::default(), id)
}

/// Load a block.
pub fn load(vm: &mut Vm, block: CodeBlock) -> CodeId {
    vm.load(block).expect("block loads")
}

/// The number in a value.
pub fn num(v: Value) -> f64 {
    v.as_number()
        .unwrap_or_else(|| panic!("not a number: {v:?}"))
}

/// A value as display text.
pub fn text(vm: &mut Vm, v: Value) -> String {
    vm.to_display_string(v).expect("printable")
}

/// Emit a forward or backward jump to be patched later.
pub fn jump_placeholder(b: &mut CodeBlock, op: Op) -> u32 {
    b.emit(op)
}

/// Point the jump at `at` to `target`.
pub fn patch(b: &mut CodeBlock, at: u32, target: u32) {
    let offset = target as i32 - (at as i32 + 1);
    let op = &mut b.ops[at as usize];
    match op {
        Op::Jump { offset: o }
        | Op::JumpIfTrue { offset: o, .. }
        | Op::JumpIfFalse { offset: o, .. }
        | Op::JumpIfNullish { offset: o, .. } => *o = offset,
        other => panic!("not a jump: {other:?}"),
    }
}

/// Offset for a jump emitted next (at `b.ops.len()`) to reach `target`.
pub fn back_to(b: &CodeBlock, target: u32) -> i32 {
    target as i32 - (b.ops.len() as i32 + 1)
}

/// A one-function block that returns the given op's result in r0.
pub fn ret(b: &mut CodeBlock, r: Reg) {
    b.emit(Op::Return { src: r });
}

/// A top-level block whose only job is to make closures of its nested
/// functions and return them in an array, in table order.
pub fn closures(vm: &mut Vm, functions: Vec<CodeBlock>) -> Vec<Value> {
    let n = functions.len();
    let mut top = CodeBlock::new("main", 0, 3);
    top.functions = functions;
    top.emit(Op::NewArray { dst: 0 });
    for i in 0..n {
        top.emit(Op::Closure {
            dst: 1,
            func: i as u32,
        });
        top.emit(Op::LoadInt {
            dst: 2,
            value: i as i32,
        });
        top.emit(Op::SetElem {
            obj: 0,
            key: 2,
            src: 1,
        });
    }
    top.emit(Op::Return { src: 0 });
    let arr = run_in(vm, top).expect("closures made");
    let o = arr.as_object().expect("array");
    let mut out = Vec::with_capacity(n);
    for i in 0..n as u32 {
        let v = vm
            .heap()
            .get_value(o, anvil_js::value::atom::PropertyKey::Index(i))
            .expect("read")
            .expect("present");
        vm.heap_mut().persist(v).expect("rooted");
        out.push(v);
    }
    out
}
