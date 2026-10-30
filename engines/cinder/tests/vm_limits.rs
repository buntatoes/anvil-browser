// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! One test per limit, each checking the exact message.

mod vm_support;

use cinder::bytecode::{BinOp, CodeBlock, Handler, Op, UnOp};
use cinder::limits::{LimitKind, Limits};
use cinder::value::Value;
use cinder::vm::{BasicHost, NativeCx, RunError, Vm};
use std::sync::atomic::{AtomicUsize, Ordering};
use vm_support::*;

/// function walk(n) { return n === 0 ? 0 : 1 + walk(n - 1) }
/// With n = -1 it never stops.
fn walk() -> CodeBlock {
    let mut f = CodeBlock::new("walk", 1, 6);
    let k = f.add_key("walk");
    let ic = f.add_ic();
    f.emit(Op::LoadInt { dst: 1, value: 0 });
    f.emit(Op::Binary {
        op: BinOp::StrictEq,
        dst: 2,
        a: 0,
        b: 1,
    });
    let j = f.emit(Op::JumpIfFalse { cond: 2, offset: 0 });
    f.emit(Op::Return { src: 1 });
    let rec = f.emit(Op::GetGlobal { dst: 3, key: k, ic });
    patch(&mut f, j, rec);
    f.emit(Op::LoadUndefined { dst: 4 });
    f.emit(Op::Unary {
        op: UnOp::Dec,
        dst: 5,
        src: 0,
    });
    f.emit(Op::Call {
        dst: 2,
        callee: 3,
        this: 4,
        args: 5,
        argc: 1,
    });
    f.emit(Op::LoadInt { dst: 1, value: 1 });
    f.emit(Op::Binary {
        op: BinOp::Add,
        dst: 2,
        a: 1,
        b: 2,
    });
    f.emit(Op::Return { src: 2 });
    f
}

fn walk_vm(limits: Limits) -> (Vm, Value) {
    let mut vm = vm_with(limits);
    let f = closures(&mut vm, vec![walk()])[0];
    vm.define_global("walk", f).unwrap();
    (vm, f)
}

#[test]
fn hundred_thousand_frames_with_a_raised_limit() {
    let (mut vm, f) = walk_vm(Limits {
        max_call_depth: 200_000,
        ..Limits::default()
    });
    let r = vm
        .call(
            &mut BasicHost::default(),
            f,
            Value::UNDEFINED,
            &[Value::number(100_000.0)],
        )
        .unwrap();
    assert_eq!(num(r), 100_000.0);
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn call_depth_default_is_a_catchable_range_error() {
    let (mut vm, f) = walk_vm(Limits::default());
    let err = vm
        .call(
            &mut BasicHost::default(),
            f,
            Value::UNDEFINED,
            &[Value::number(-1.0)],
        )
        .unwrap_err();
    assert_eq!(
        err.message(),
        "RangeError: Maximum call depth exceeded (10,000 frames) in function 'walk'"
    );
    assert!(err.is_catchable());
    assert_eq!(
        err.stack().len(),
        65,
        "64 frames (max_stack_lines) and the marker"
    );
    let last = err.stack().last().unwrap();
    assert!(last.is_omitted_marker());
    assert_eq!(last.to_string(), "... 9,936 more frames not shown");
    assert!(err.stack()[..64].iter().all(|l| !l.is_omitted_marker()));
    assert_eq!(vm.frame_count(), 0);

    // Script can catch it.
    let mut main = CodeBlock::new("main", 0, 4);
    let k = main.add_key("walk");
    let ic = main.add_ic();
    main.emit(Op::GetGlobal { dst: 0, key: k, ic }); // 0
    main.emit(Op::LoadUndefined { dst: 1 }); // 1
    main.emit(Op::LoadInt { dst: 2, value: -1 }); // 2
    main.emit(Op::Call {
        dst: 3,
        callee: 0,
        this: 1,
        args: 2,
        argc: 1,
    }); // 3
    main.emit(Op::Return { src: 3 }); // 4
    main.emit(Op::Return { src: 1 }); // 5 catch
    main.handlers.push(Handler {
        start: 0,
        end: 5,
        target: 5,
        catch_reg: 1,
        env_depth: 0,
    });
    let caught = run_in(&mut vm, main).unwrap();
    assert_eq!(
        text(&mut vm, caught),
        "RangeError: Maximum call depth exceeded (10,000 frames) in function 'walk'"
    );
}

static STACK_LOW: AtomicUsize = AtomicUsize::new(usize::MAX);
static STACK_HIGH: AtomicUsize = AtomicUsize::new(0);
static LEVELS: AtomicUsize = AtomicUsize::new(0);

/// reenter(f): calls f(), which calls reenter(f) again.
fn reenter(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let marker = 0u8;
    let addr = &marker as *const u8 as usize;
    STACK_LOW.fetch_min(addr, Ordering::Relaxed);
    STACK_HIGH.fetch_max(addr, Ordering::Relaxed);
    LEVELS.fetch_add(1, Ordering::Relaxed);
    let f = args.first().copied().unwrap_or(Value::UNDEFINED);
    cx.call(f, Value::UNDEFINED, &[f])
}

#[test]
fn native_reentry_limit() {
    // Each level of native re-entry uses the Rust stack, so run on a thread
    // with a known stack size and report what one level costs.
    let child = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let mut vm = vm();
            let n = vm.register_native("reenter", reenter).unwrap();
            vm.define_global("reenter", n).unwrap();
            // function bounce(f) { return reenter(f) }
            let mut b = CodeBlock::new("bounce", 1, 4);
            let k = b.add_key("reenter");
            let ic = b.add_ic();
            b.emit(Op::GetGlobal { dst: 1, key: k, ic });
            b.emit(Op::LoadUndefined { dst: 2 });
            b.emit(Op::Call {
                dst: 3,
                callee: 1,
                this: 2,
                args: 0,
                argc: 1,
            });
            b.emit(Op::Return { src: 3 });
            let bounce = closures(&mut vm, vec![b])[0];
            let err = vm
                .call(&mut BasicHost::default(), n, Value::UNDEFINED, &[bounce])
                .unwrap_err();
            (err, vm.frame_count())
        })
        .unwrap();
    let (err, frames) = child.join().unwrap();
    assert_eq!(
        err.message(),
        "RangeError: Too many nested builtin callbacks (256)"
    );
    assert!(err.is_catchable());
    assert_eq!(frames, 0);
    let levels = LEVELS.load(Ordering::Relaxed);
    assert_eq!(levels, 257, "the top call plus 256 nested ones");
    let used = STACK_HIGH.load(Ordering::Relaxed) - STACK_LOW.load(Ordering::Relaxed);
    let per_level = used / (levels - 1);
    eprintln!("native re-entry: {used} bytes of Rust stack for 256 levels, {per_level} per level");
    assert!(
        per_level < 64 << 10,
        "{per_level} bytes per level is too much"
    );
}

/// while (true) {}
fn spin() -> CodeBlock {
    let mut b = CodeBlock::new("spin", 0, 1);
    b.loops = 1;
    b.emit(Op::LoopHeader { id: 0 });
    b.emit(Op::Jump { offset: -2 });
    b
}

#[test]
fn step_budget_is_an_uncatchable_abort() {
    let mut vm = vm_with(Limits {
        max_steps: Some(1_000),
        ..Limits::default()
    });
    // try { while (true) {} } catch { return 1 } -- the handler never runs.
    let mut b = spin();
    b.register_count = 2;
    b.emit(Op::LoadInt { dst: 1, value: 1 });
    b.emit(Op::Return { src: 1 });
    b.handlers.push(Handler {
        start: 0,
        end: 2,
        target: 2,
        catch_reg: 0,
        env_depth: 0,
    });
    let err = run_in(&mut vm, b).unwrap_err();
    let RunError::Abort {
        kind,
        reason,
        limit,
        observed,
        stack,
    } = &err
    else {
        panic!("expected an abort, got {err:?}");
    };
    assert_eq!(*kind, LimitKind::Steps);
    assert_eq!(
        reason,
        "step budget used up: 1,000 steps in this run (max_steps)"
    );
    assert_eq!((*limit, *observed), (1_000, 1_000));
    assert_eq!(stack[0].function, "spin");
    assert!(!err.is_catchable());
    assert_eq!(vm.frame_count(), 0);

    // The budget is per run: the next run starts from zero.
    let r = run_in(&mut vm, {
        let mut b = CodeBlock::new("ok", 0, 1);
        b.emit(Op::LoadInt { dst: 0, value: 5 });
        b.emit(Op::Return { src: 0 });
        b
    });
    assert_eq!(num(r.unwrap()), 5.0);
}

#[test]
fn default_step_budget_message() {
    // The default budget is 1,000,000,000 steps; check the message it gives
    // without spending them, by starting a run with the counter near the end
    // is not possible from outside, so use a budget of the same size in a
    // tight loop only when asked. Here: the default value and its wording.
    assert_eq!(Limits::default().max_steps, Some(1_000_000_000));
    let mut vm = vm_with(Limits {
        max_steps: Some(10),
        ..Limits::default()
    });
    let err = run_in(&mut vm, spin()).unwrap_err();
    assert_eq!(
        err.message(),
        "step budget used up: 10 steps in this run (max_steps)"
    );
}

#[test]
fn host_interrupt() {
    let mut vm = vm_with(Limits {
        interrupt_check_interval: 100,
        max_steps: None,
        ..Limits::default()
    });
    let id = load(&mut vm, spin());
    let mut host = BasicHost {
        interrupt_after: Some(3),
        ..BasicHost::default()
    };
    let err = vm.run(&mut host, id).unwrap_err();
    let RunError::Abort {
        kind,
        reason,
        observed,
        ..
    } = &err
    else {
        panic!("expected an abort, got {err:?}");
    };
    assert_eq!(*kind, LimitKind::Interrupt);
    assert_eq!(reason, "stopped by host: interrupted after 3 checks");
    assert_eq!(*observed, 300);
    assert_eq!(host.checks, 3);
}

#[test]
fn heap_limit_is_an_uncatchable_abort() {
    let max = 256 << 10;
    let mut vm = vm_with(Limits {
        max_heap_bytes: max,
        initial_gc_trigger: 64 << 10,
        ..Limits::default()
    });
    // a = []; i = 0; while (true) { a[i] = {}; i++ }   inside a try
    let mut b = CodeBlock::new("fill", 0, 3);
    b.loops = 1;
    b.emit(Op::NewArray { dst: 0 }); // 0
    b.emit(Op::LoadInt { dst: 1, value: 0 }); // 1
    b.emit(Op::LoopHeader { id: 0 }); // 2
    b.emit(Op::NewObject { dst: 2 }); // 3
    b.emit(Op::SetElem {
        obj: 0,
        key: 1,
        src: 2,
    }); // 4
    b.emit(Op::Unary {
        op: UnOp::Inc,
        dst: 1,
        src: 1,
    }); // 5
    b.emit(Op::Jump { offset: -5 }); // 6 -> 2
    b.emit(Op::Return { src: 2 }); // 7 catch
    b.handlers.push(Handler {
        start: 0,
        end: 7,
        target: 7,
        catch_reg: 2,
        env_depth: 0,
    });
    let mut host = BasicHost::default();
    let id = load(&mut vm, b);
    let err = vm.run(&mut host, id).unwrap_err();
    let RunError::Abort {
        kind,
        reason,
        limit,
        observed,
        ..
    } = &err
    else {
        panic!("expected an abort, got {err:?}");
    };
    assert_eq!(*kind, LimitKind::HeapBytes);
    assert_eq!(*limit, max as u64);
    // "heap limit reached: N more bytes needed with M bytes in use, limit
    // 262,144 bytes", where N + M is the observed total.
    let rest = reason
        .strip_prefix("heap limit reached: ")
        .unwrap_or_else(|| panic!("{reason}"));
    let rest = rest
        .strip_suffix(" bytes in use, limit 262,144 bytes")
        .unwrap_or_else(|| panic!("{reason}"));
    let (needed, in_use) = rest.split_once(" more bytes needed with ").unwrap();
    let parse = |s: &str| s.replace(',', "").parse::<u64>().unwrap();
    assert_eq!(parse(needed) + parse(in_use), *observed);
    assert!(*observed > max as u64);
    assert!(
        vm.heap().stats().collections > 0,
        "collections ran before giving up"
    );
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn string_length_limit() {
    let mut vm = vm_with(Limits {
        max_string_length: 100,
        ..Limits::default()
    });
    // s = "aaaaaaaa"; while (true) s = s + s;
    let mut b = CodeBlock::new("grow", 0, 1);
    b.loops = 1;
    let s = b.add_string("aaaaaaaa");
    b.emit(Op::LoadString { dst: 0, idx: s });
    b.emit(Op::LoopHeader { id: 0 });
    b.emit(Op::Binary {
        op: BinOp::Add,
        dst: 0,
        a: 0,
        b: 0,
    });
    b.emit(Op::Jump { offset: -3 });
    let err = run_in(&mut vm, b).unwrap_err();
    assert_eq!(
        err.message(),
        "RangeError: Invalid string length: 128 code units, limit 100"
    );
    assert!(err.is_catchable());
}

#[test]
fn array_length_limit() {
    let mut b = CodeBlock::new("main", 0, 2);
    let len = b.add_key("length");
    let ic = b.add_ic();
    let n = b.add_number(4_294_967_296.0);
    b.emit(Op::NewArray { dst: 0 });
    b.emit(Op::LoadNumber { dst: 1, idx: n });
    b.emit(Op::SetProp {
        obj: 0,
        src: 1,
        key: len,
        ic,
    });
    b.emit(Op::Return { src: 0 });
    let (_, r) = run(b);
    assert_eq!(
        r.unwrap_err().message(),
        "RangeError: Invalid array length: 4294967296"
    );
}

#[test]
fn register_limit_is_a_compile_error() {
    let mut vm = vm();
    let mut big = CodeBlock::new("big", 0, 70_000);
    big.emit(Op::Return { src: 0 });
    let mut main = CodeBlock::new("main", 0, 1);
    main.functions.push(big);
    main.emit(Op::Return { src: 0 });
    let err = vm.load(main).unwrap_err();
    let RunError::Compile { message, .. } = &err else {
        panic!("expected a compile error, got {err:?}");
    };
    assert_eq!(
        message,
        "function 'big' needs 70,000 registers, limit 65,535"
    );
    assert!(
        vm.code(cinder::vm::CodeId(0)).is_none(),
        "nothing was loaded"
    );

    // A smaller setting is honored too.
    let mut vm = vm_with(Limits {
        max_registers_per_function: 16,
        ..Limits::default()
    });
    let mut b = CodeBlock::new("", 0, 17);
    b.emit(Op::Return { src: 0 });
    assert_eq!(
        vm.load(b).unwrap_err().message(),
        "function 'anonymous' needs 17 registers, limit 16"
    );
}

/// a = []; repeat n times: a = [a]; return "" + a
fn nested_arrays(n: i32) -> CodeBlock {
    let mut b = CodeBlock::new("nest", 0, 5);
    b.loops = 1;
    let empty = b.add_string("");
    b.emit(Op::NewArray { dst: 0 }); // 0
    b.emit(Op::LoadInt { dst: 1, value: 0 }); // 1 i
    b.emit(Op::LoadInt { dst: 2, value: n }); // 2
    let head = b.emit(Op::LoopHeader { id: 0 }); // 3
    b.emit(Op::Binary {
        op: BinOp::Lt,
        dst: 3,
        a: 1,
        b: 2,
    }); // 4
    let exit = b.emit(Op::JumpIfFalse { cond: 3, offset: 0 }); // 5
    b.emit(Op::NewArray { dst: 3 });
    b.emit(Op::LoadInt { dst: 4, value: 0 });
    b.emit(Op::SetElem {
        obj: 3,
        key: 4,
        src: 0,
    });
    b.emit(Op::Move { dst: 0, src: 3 });
    b.emit(Op::Unary {
        op: UnOp::Inc,
        dst: 1,
        src: 1,
    });
    let off = back_to(&b, head);
    b.emit(Op::Jump { offset: off });
    let end = b.emit(Op::LoadString { dst: 4, idx: empty });
    patch(&mut b, exit, end);
    b.emit(Op::Binary {
        op: BinOp::Add,
        dst: 4,
        a: 4,
        b: 0,
    });
    b.emit(Op::Return { src: 4 });
    b
}

#[test]
fn join_depth_limit() {
    // 10,000 levels are fine (the outer array plus 9,999 inside it).
    let (_, r) = run(nested_arrays(9_999));
    assert!(r.is_ok(), "{r:?}");
    // One more is not.
    let (_, r) = run(nested_arrays(10_000));
    assert_eq!(
        r.unwrap_err().message(),
        "RangeError: Array nesting too deep to join: more than 10,000 levels (max_join_depth)"
    );
    let mut vm = vm_with(Limits {
        max_join_depth: 10,
        ..Limits::default()
    });
    let r = run_in(&mut vm, nested_arrays(10));
    assert_eq!(
        r.unwrap_err().message(),
        "RangeError: Array nesting too deep to join: more than 10 levels (max_join_depth)"
    );
}

#[test]
fn stack_lines_limit() {
    let (mut vm, f) = walk_vm(Limits {
        max_call_depth: 100,
        max_stack_lines: 5,
        ..Limits::default()
    });
    let err = vm
        .call(
            &mut BasicHost::default(),
            f,
            Value::UNDEFINED,
            &[Value::number(-1.0)],
        )
        .unwrap_err();
    assert_eq!(
        err.message(),
        "RangeError: Maximum call depth exceeded (100 frames) in function 'walk'"
    );
    assert_eq!(err.stack().len(), 6);
    assert!(err.stack()[..5].iter().all(|l| l.function == "walk"));
    assert_eq!(err.stack()[5].to_string(), "... 95 more frames not shown");
    assert_eq!(
        cinder::vm::StackLine::omitted(1).to_string(),
        "... 1 more frame not shown"
    );
    assert_eq!(
        cinder::vm::StackLine::omitted(1234567).to_string(),
        "... 1,234,567 more frames not shown"
    );
}

#[test]
fn bad_limits_are_refused() {
    let err = Vm::new(Limits {
        max_call_depth: 0,
        ..Limits::default()
    })
    .unwrap_err();
    assert!(matches!(err, RunError::Internal { .. }));
    assert!(
        err.message().starts_with("bad limits: "),
        "{}",
        err.message()
    );
}
