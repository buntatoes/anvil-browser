// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Hand-assembled programs: arithmetic, control flow, calls, closures,
//! objects, inline caches, exceptions.

mod vm_support;

use cinder::bytecode::{BinOp, CodeBlock, Handler, Op, UnOp};
use cinder::value::Value;
use cinder::value::atom::PropertyKey;
use cinder::value::object::{Property, Slot};
use cinder::value::shape::Attrs;
use cinder::vm::{BasicHost, IcState, NativeCx, RunError, Vm};
use vm_support::*;

fn bin(op: BinOp, dst: u16, a: u16, b: u16) -> Op {
    Op::Binary { op, dst, a, b }
}

/// Run `a op b` on two loaded constants and return the result as text.
fn eval2(a: Op, b: Op, op: BinOp) -> String {
    let mut blk = CodeBlock::new("main", 0, 3);
    blk.emit(a);
    blk.emit(b);
    blk.emit(bin(op, 2, 0, 1));
    blk.emit(Op::Return { src: 2 });
    let (mut vm, r) = run(blk);
    let v = r.expect("runs");
    text(&mut vm, v)
}

fn int(dst: u16, value: i32) -> Op {
    Op::LoadInt { dst, value }
}

#[test]
fn arithmetic() {
    // (2 + 3) * 4 - 10 / 4
    let mut b = CodeBlock::new("main", 0, 4);
    b.emit(int(0, 2));
    b.emit(int(1, 3));
    b.emit(bin(BinOp::Add, 2, 0, 1));
    b.emit(int(3, 4));
    b.emit(bin(BinOp::Mul, 2, 2, 3));
    b.emit(int(0, 10));
    b.emit(bin(BinOp::Div, 0, 0, 3));
    b.emit(bin(BinOp::Sub, 2, 2, 0));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 17.5);

    assert_eq!(eval2(int(0, 7), int(1, 3), BinOp::Mod), "1");
    assert_eq!(eval2(int(0, -7), int(1, 3), BinOp::Mod), "-1");
    assert_eq!(eval2(int(0, 2), int(1, 10), BinOp::Exp), "1024");
    assert_eq!(eval2(int(0, 6), int(1, 3), BinOp::BitAnd), "2");
    assert_eq!(eval2(int(0, 6), int(1, 3), BinOp::BitOr), "7");
    assert_eq!(eval2(int(0, 6), int(1, 3), BinOp::BitXor), "5");
    assert_eq!(eval2(int(0, 1), int(1, 33), BinOp::Shl), "2");
    assert_eq!(eval2(int(0, -16), int(1, 2), BinOp::Shr), "-4");
    assert_eq!(eval2(int(0, -1), int(1, 0), BinOp::UShr), "4294967295");
    assert_eq!(eval2(int(0, 1), int(1, 0), BinOp::Div), "Infinity");
    assert_eq!(eval2(int(0, 0), int(1, 0), BinOp::Div), "NaN");
    // Int overflow goes to doubles.
    assert_eq!(eval2(int(0, i32::MAX), int(1, 1), BinOp::Add), "2147483648");

    // -0 stays -0: 1 / -0 is -Infinity.
    let mut b = CodeBlock::new("main", 0, 3);
    b.emit(int(0, 0));
    b.emit(Op::Unary {
        op: UnOp::Neg,
        dst: 0,
        src: 0,
    });
    b.emit(int(1, 1));
    b.emit(bin(BinOp::Div, 2, 1, 0));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), f64::NEG_INFINITY);
}

#[test]
fn strings_and_coercion() {
    let mut b = CodeBlock::new("main", 0, 3);
    let s = b.add_string("a");
    b.emit(Op::LoadString { dst: 0, idx: s });
    b.emit(int(1, 1));
    b.emit(bin(BinOp::Add, 2, 0, 1));
    b.emit(Op::LoadTrue { dst: 1 });
    b.emit(bin(BinOp::Add, 2, 2, 1));
    b.emit(Op::LoadNull { dst: 1 });
    b.emit(bin(BinOp::Add, 2, 2, 1));
    b.emit(Op::Return { src: 2 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "a1truenull");

    // "3" * "4" is 12, "5" - 2 is 3, true + 1 is 2.
    let mut b = CodeBlock::new("main", 0, 3);
    let three = b.add_string("3");
    let four = b.add_string(" 4 ");
    b.emit(Op::LoadString { dst: 0, idx: three });
    b.emit(Op::LoadString { dst: 1, idx: four });
    b.emit(bin(BinOp::Mul, 2, 0, 1));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 12.0);

    // typeof
    let mut b = CodeBlock::new("main", 0, 2);
    b.emit(Op::NewObject { dst: 0 });
    b.emit(Op::Unary {
        op: UnOp::TypeOf,
        dst: 1,
        src: 0,
    });
    b.emit(Op::Return { src: 1 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "object");
}

#[test]
fn comparisons() {
    assert_eq!(eval2(int(0, 1), int(1, 2), BinOp::Lt), "true");
    assert_eq!(eval2(int(0, 2), int(1, 2), BinOp::Le), "true");
    assert_eq!(eval2(int(0, 2), int(1, 2), BinOp::Gt), "false");
    assert_eq!(eval2(int(0, 3), int(1, 2), BinOp::Ge), "true");
    assert_eq!(
        eval2(int(0, 1), Op::LoadNull { dst: 1 }, BinOp::LooseEq),
        "false"
    );
    assert_eq!(
        eval2(
            Op::LoadNull { dst: 0 },
            Op::LoadUndefined { dst: 1 },
            BinOp::LooseEq
        ),
        "true"
    );
    assert_eq!(
        eval2(
            Op::LoadNull { dst: 0 },
            Op::LoadUndefined { dst: 1 },
            BinOp::StrictEq
        ),
        "false"
    );
    assert_eq!(
        eval2(Op::LoadTrue { dst: 0 }, int(1, 1), BinOp::LooseEq),
        "true"
    );
    assert_eq!(
        eval2(Op::LoadTrue { dst: 0 }, int(1, 1), BinOp::StrictNe),
        "true"
    );

    // NaN is not equal to itself and compares false both ways.
    let mut b = CodeBlock::new("main", 0, 4);
    let nan = b.add_number(f64::NAN);
    b.emit(Op::LoadNumber { dst: 0, idx: nan });
    b.emit(bin(BinOp::StrictEq, 1, 0, 0));
    b.emit(bin(BinOp::Lt, 2, 0, 0));
    b.emit(bin(BinOp::Ge, 3, 0, 0));
    b.emit(bin(BinOp::BitOr, 1, 1, 2));
    b.emit(bin(BinOp::BitOr, 1, 1, 3));
    b.emit(Op::Return { src: 1 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 0.0);

    // Strings: by content, and "10" < "9".
    let mut b = CodeBlock::new("main", 0, 4);
    let ten = b.add_string("10");
    let nine = b.add_string("9");
    let a = b.add_string("ab");
    b.emit(Op::LoadString { dst: 0, idx: ten });
    b.emit(Op::LoadString { dst: 1, idx: nine });
    b.emit(bin(BinOp::Lt, 2, 0, 1));
    // Build "ab" from "a" + "b" and compare with the constant.
    let sa = b.add_string("a");
    let sb = b.add_string("b");
    b.emit(Op::LoadString { dst: 0, idx: sa });
    b.emit(Op::LoadString { dst: 1, idx: sb });
    b.emit(bin(BinOp::Add, 0, 0, 1));
    b.emit(Op::LoadString { dst: 1, idx: a });
    b.emit(bin(BinOp::StrictEq, 3, 0, 1));
    b.emit(bin(BinOp::BitAnd, 2, 2, 3));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 1.0);

    // "1" == 1 but not ===.
    let mut b = CodeBlock::new("main", 0, 4);
    let one = b.add_string("1");
    b.emit(Op::LoadString { dst: 0, idx: one });
    b.emit(int(1, 1));
    b.emit(bin(BinOp::LooseEq, 2, 0, 1));
    b.emit(bin(BinOp::StrictEq, 3, 0, 1));
    b.emit(bin(BinOp::Sub, 2, 2, 3));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 1.0);
}

/// sum = 0; for (i = 1; i <= 100; i++) sum += i; return sum
fn sum_loop() -> CodeBlock {
    let mut b = CodeBlock::new("main", 0, 4);
    b.loops = 1;
    b.emit(int(0, 0)); // sum
    b.emit(int(1, 1)); // i
    b.emit(int(2, 100));
    let head = b.emit(Op::LoopHeader { id: 0 });
    b.emit(bin(BinOp::Le, 3, 1, 2));
    let exit = b.emit(Op::JumpIfFalse { cond: 3, offset: 0 });
    b.emit(bin(BinOp::Add, 0, 0, 1));
    b.emit(Op::Unary {
        op: UnOp::Inc,
        dst: 1,
        src: 1,
    });
    let off = back_to(&b, head);
    b.emit(Op::Jump { offset: off });
    let end = b.emit(Op::Return { src: 0 });
    patch(&mut b, exit, end);
    b
}

#[test]
fn branching_and_loops() {
    let mut vm = vm();
    let id = load(&mut vm, sum_loop());
    let r = vm.run(&mut BasicHost::default(), id).unwrap();
    assert_eq!(num(r), 5050.0);
    assert_eq!(vm.loop_trips(id), vec![101]);
    assert_eq!(vm.frame_count(), 0);

    // if (x) 1 else 2, with every falsy kind.
    for (load_op, want) in [
        (Op::LoadFalse { dst: 0 }, 2.0),
        (Op::LoadTrue { dst: 0 }, 1.0),
        (int(0, 0), 2.0),
        (Op::LoadNull { dst: 0 }, 2.0),
        (Op::NewObject { dst: 0 }, 1.0),
    ] {
        let mut b = CodeBlock::new("main", 0, 2);
        b.emit(load_op);
        let j = b.emit(Op::JumpIfFalse { cond: 0, offset: 0 });
        b.emit(int(1, 1));
        b.emit(Op::Return { src: 1 });
        let t = b.emit(int(1, 2));
        b.emit(Op::Return { src: 1 });
        patch(&mut b, j, t);
        let (_, r) = run(b);
        assert_eq!(num(r.unwrap()), want);
    }

    // Empty string is falsy, "0" is truthy; nullish jump.
    let mut b = CodeBlock::new("main", 0, 3);
    let e = b.add_string("");
    let z = b.add_string("0");
    b.emit(Op::LoadString { dst: 0, idx: e });
    b.emit(Op::LoadString { dst: 1, idx: z });
    b.emit(Op::Unary {
        op: UnOp::Not,
        dst: 0,
        src: 0,
    });
    b.emit(Op::Unary {
        op: UnOp::Not,
        dst: 1,
        src: 1,
    });
    b.emit(bin(BinOp::StrictNe, 2, 0, 1));
    b.emit(Op::LoadUndefined { dst: 0 });
    let j = b.emit(Op::JumpIfNullish { cond: 0, offset: 0 });
    b.emit(Op::LoadFalse { dst: 2 });
    let t = b.emit(Op::Return { src: 2 });
    patch(&mut b, j, t);
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "true");
}

/// function add(a, b) { return a + b }
fn add_fn() -> CodeBlock {
    let mut f = CodeBlock::new("add", 2, 3);
    f.emit(bin(BinOp::Add, 2, 0, 1));
    f.emit(Op::Return { src: 2 });
    f
}

#[test]
fn calls() {
    // add(40, 2), add(1) is NaN (b is undefined), add(1, 2, 3) ignores 3.
    let mut b = CodeBlock::new("main", 0, 8);
    b.functions.push(add_fn());
    b.emit(Op::Closure { dst: 0, func: 0 });
    b.emit(Op::LoadUndefined { dst: 1 });
    b.emit(int(2, 40));
    b.emit(int(3, 2));
    b.emit(int(4, 3));
    b.emit(Op::Call {
        dst: 5,
        callee: 0,
        this: 1,
        args: 2,
        argc: 2,
    });
    b.emit(Op::Call {
        dst: 6,
        callee: 0,
        this: 1,
        args: 2,
        argc: 1,
    });
    b.emit(Op::Call {
        dst: 7,
        callee: 0,
        this: 1,
        args: 2,
        argc: 3,
    });
    b.emit(Op::NewArray { dst: 0 });
    b.emit(int(1, 0));
    b.emit(Op::SetElem {
        obj: 0,
        key: 1,
        src: 5,
    });
    b.emit(int(1, 1));
    b.emit(Op::SetElem {
        obj: 0,
        key: 1,
        src: 6,
    });
    b.emit(int(1, 2));
    b.emit(Op::SetElem {
        obj: 0,
        key: 1,
        src: 7,
    });
    b.emit(Op::Return { src: 0 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "42,NaN,42");
}

#[test]
fn recursion_uses_frames() {
    // function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) }
    // fib is read through a global so the function can find itself.
    let mut f = CodeBlock::new("fib", 1, 6);
    let k = f.add_key("fib");
    let ic = f.add_ic();
    f.emit(int(1, 2));
    f.emit(bin(BinOp::Lt, 2, 0, 1));
    let j = f.emit(Op::JumpIfFalse { cond: 2, offset: 0 });
    f.emit(Op::Return { src: 0 });
    let rec = f.emit(Op::GetGlobal { dst: 3, key: k, ic });
    patch(&mut f, j, rec);
    f.emit(Op::LoadUndefined { dst: 4 });
    f.emit(int(1, 1));
    f.emit(bin(BinOp::Sub, 5, 0, 1));
    f.emit(Op::Call {
        dst: 2,
        callee: 3,
        this: 4,
        args: 5,
        argc: 1,
    });
    f.emit(int(1, 2));
    f.emit(bin(BinOp::Sub, 5, 0, 1));
    f.emit(Op::Call {
        dst: 5,
        callee: 3,
        this: 4,
        args: 5,
        argc: 1,
    });
    f.emit(bin(BinOp::Add, 2, 2, 5));
    f.emit(Op::Return { src: 2 });

    let mut vm = vm();
    let fns = closures(&mut vm, vec![f]);
    vm.define_global("fib", fns[0]).unwrap();
    let r = vm
        .call(
            &mut BasicHost::default(),
            fns[0],
            Value::UNDEFINED,
            &[Value::number(20.0)],
        )
        .unwrap();
    assert_eq!(num(r), 6765.0);
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn closures_share_captured_state() {
    // function make() {
    //   let n = 0;                          // env slot 0
    //   return [() => ++n, () => n];
    // }
    let mut inc = CodeBlock::new("inc", 0, 1);
    inc.emit(Op::GetEnv {
        dst: 0,
        depth: 0,
        slot: 0,
    });
    inc.emit(Op::Unary {
        op: UnOp::Inc,
        dst: 0,
        src: 0,
    });
    inc.emit(Op::SetEnv {
        src: 0,
        depth: 0,
        slot: 0,
    });
    inc.emit(Op::Return { src: 0 });
    let mut get = CodeBlock::new("get", 0, 1);
    get.emit(Op::GetEnv {
        dst: 0,
        depth: 0,
        slot: 0,
    });
    get.emit(Op::Return { src: 0 });

    let mut make = CodeBlock::new("make", 0, 4);
    make.functions = vec![inc, get];
    make.emit(Op::PushEnv { size: 1 });
    make.emit(int(0, 0));
    make.emit(Op::SetEnv {
        src: 0,
        depth: 0,
        slot: 0,
    });
    make.emit(Op::NewArray { dst: 1 });
    make.emit(Op::Closure { dst: 2, func: 0 });
    make.emit(int(3, 0));
    make.emit(Op::SetElem {
        obj: 1,
        key: 3,
        src: 2,
    });
    make.emit(Op::Closure { dst: 2, func: 1 });
    make.emit(int(3, 1));
    make.emit(Op::SetElem {
        obj: 1,
        key: 3,
        src: 2,
    });
    make.emit(Op::Return { src: 1 });

    let mut vm = vm();
    let fns = closures(&mut vm, vec![make]);
    let mut host = BasicHost::default();
    let pair = |vm: &mut Vm, host: &mut BasicHost| {
        let arr = vm.call(host, fns[0], Value::UNDEFINED, &[]).unwrap();
        vm.heap_mut().persist(arr).unwrap();
        let o = arr.as_object().unwrap();
        let a = vm
            .heap()
            .get_value(o, PropertyKey::Index(0))
            .unwrap()
            .unwrap();
        let b = vm
            .heap()
            .get_value(o, PropertyKey::Index(1))
            .unwrap()
            .unwrap();
        (a, b)
    };
    let (inc1, get1) = pair(&mut vm, &mut host);
    let (inc2, get2) = pair(&mut vm, &mut host);
    for _ in 0..3 {
        vm.call(&mut host, inc1, Value::UNDEFINED, &[]).unwrap();
    }
    vm.call(&mut host, inc2, Value::UNDEFINED, &[]).unwrap();
    vm.collect_garbage(&mut host);
    let g1 = vm.call(&mut host, get1, Value::UNDEFINED, &[]).unwrap();
    let g2 = vm.call(&mut host, get2, Value::UNDEFINED, &[]).unwrap();
    assert_eq!(num(g1), 3.0, "inc1 and get1 share one n");
    assert_eq!(num(g2), 1.0, "a second make() has its own n");
}

#[test]
fn objects_have_reference_identity() {
    // o = {}; p = o; p.x = 5; return [o.x, o === p, {} === {}]
    let mut b = CodeBlock::new("main", 0, 6);
    let x = b.add_key("x");
    let ic1 = b.add_ic();
    let ic2 = b.add_ic();
    b.emit(Op::NewObject { dst: 0 });
    b.emit(Op::Move { dst: 1, src: 0 });
    b.emit(int(2, 5));
    b.emit(Op::SetProp {
        obj: 1,
        src: 2,
        key: x,
        ic: ic1,
    });
    b.emit(Op::GetProp {
        dst: 3,
        obj: 0,
        key: x,
        ic: ic2,
    });
    b.emit(bin(BinOp::StrictEq, 4, 0, 1));
    b.emit(Op::NewObject { dst: 1 });
    b.emit(Op::NewObject { dst: 2 });
    b.emit(bin(BinOp::StrictEq, 5, 1, 2));
    b.emit(Op::NewArray { dst: 0 });
    for (i, r) in [3u16, 4, 5].into_iter().enumerate() {
        b.emit(int(1, i as i32));
        b.emit(Op::SetElem {
            obj: 0,
            key: 1,
            src: r,
        });
    }
    b.emit(Op::Return { src: 0 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "5,true,false");
}

/// function get(o) { return o.x }  (one cache site)
fn getter_fn(key: &str) -> CodeBlock {
    let mut f = CodeBlock::new("get", 1, 2);
    let k = f.add_key(key);
    let ic = f.add_ic();
    f.emit(Op::GetProp {
        dst: 1,
        obj: 0,
        key: k,
        ic,
    });
    f.emit(Op::Return { src: 1 });
    f
}

/// function set(o, v) { o.x = v }
fn setter_fn(key: &str) -> CodeBlock {
    let mut f = CodeBlock::new("set", 2, 2);
    let k = f.add_key(key);
    let ic = f.add_ic();
    f.emit(Op::SetProp {
        obj: 0,
        src: 1,
        key: k,
        ic,
    });
    f.emit(Op::Return { src: 1 });
    f
}

fn obj_with(vm: &mut Vm, proto: Option<cinder::value::ObjectRef>, kv: &[(&str, f64)]) -> Value {
    let proto = proto.or(Some(vm.object_prototype()));
    let o = vm.heap_mut().new_object(proto).unwrap();
    for (k, v) in kv {
        let key = vm.heap_mut().key_from_str(k).unwrap();
        vm.heap_mut()
            .set_own_value(o, key, Value::number(*v))
            .unwrap();
    }
    let v = Value::object(o);
    vm.heap_mut().persist(v).unwrap();
    v
}

#[test]
fn property_ic_states_and_shape_changes() {
    let mut vm = vm();
    let fns = closures(&mut vm, vec![getter_fn("x")]);
    let get = fns[0];
    let get_code = cinder::vm::CodeId(vm.children(cinder::vm::CodeId(0))[0].0);
    let mut host = BasicHost::default();
    let site = |vm: &Vm| vm.ic_site(get_code, 0).unwrap().clone();

    let a = obj_with(&mut vm, None, &[("x", 1.0)]);
    let a2 = obj_with(&mut vm, None, &[("x", 11.0)]);
    let b = obj_with(&mut vm, None, &[("y", 0.0), ("x", 2.0)]);
    let mut call =
        |vm: &mut Vm, o: Value| num(vm.call(&mut host, get, Value::UNDEFINED, &[o]).unwrap());

    assert_eq!(site(&vm).state(), IcState::Empty);
    assert_eq!(call(&mut vm, a), 1.0);
    assert_eq!(site(&vm).state(), IcState::Mono);
    assert_eq!(call(&mut vm, a2), 11.0, "same shape, other object");
    assert_eq!(site(&vm).hits, 1);
    assert_eq!(call(&mut vm, b), 2.0);
    assert_eq!(site(&vm).state(), IcState::Poly);

    // Shape change on a cached object: deleting x moves `a` to a new shape,
    // so the cached slot must not be used.
    let kx = vm.heap_mut().key_from_str("x").unwrap();
    vm.heap_mut()
        .delete_own(a.as_object().unwrap(), kx)
        .unwrap();
    let after = vm
        .call(&mut BasicHost::default(), get, Value::UNDEFINED, &[a])
        .unwrap();
    assert!(after.is_undefined(), "x was deleted, got {after:?}");

    // Adding a different key first gives yet another shape.
    let c = obj_with(&mut vm, None, &[("z", 0.0), ("x", 3.0)]);
    let d = obj_with(&mut vm, None, &[("w", 0.0), ("x", 4.0)]);
    let e = obj_with(&mut vm, None, &[("v", 0.0), ("x", 5.0)]);
    let mut host = BasicHost::default();
    for (o, want) in [(c, 3.0), (d, 4.0), (e, 5.0), (a2, 11.0), (b, 2.0)] {
        let r = vm.call(&mut host, get, Value::UNDEFINED, &[o]).unwrap();
        assert_eq!(num(r), want);
    }
    assert_eq!(site(&vm).state(), IcState::Mega);
    // Mega sites still answer correctly.
    let r = vm.call(&mut host, get, Value::UNDEFINED, &[c]).unwrap();
    assert_eq!(num(r), 3.0);
}

#[test]
fn prototype_hits_are_invalidated() {
    let mut vm = vm();
    let fns = closures(&mut vm, vec![getter_fn("y")]);
    let get = fns[0];
    let code = vm.children(cinder::vm::CodeId(0))[0];
    let mut host = BasicHost::default();

    // o -> m -> p, with y on p.
    let p = obj_with(&mut vm, None, &[("y", 7.0)]);
    let m = obj_with(&mut vm, p.as_object(), &[]);
    let o = obj_with(&mut vm, m.as_object(), &[("own", 1.0)]);
    let mut get_y = |vm: &mut Vm| num(vm.call(&mut host, get, Value::UNDEFINED, &[o]).unwrap());

    assert_eq!(get_y(&mut vm), 7.0);
    assert_eq!(get_y(&mut vm), 7.0);
    let s = vm.ic_site(code, 0).unwrap();
    assert_eq!(s.hits, 1, "second read is a prototype hit");

    // Changing the value on the holder is seen (the slot is read live).
    let ky = vm.heap_mut().key_from_str("y").unwrap();
    vm.heap_mut()
        .set_own_value(p.as_object().unwrap(), ky, Value::number(8.0))
        .unwrap();
    assert_eq!(get_y(&mut vm), 8.0);

    // Adding y on the middle object shadows it: the epoch moved.
    vm.heap_mut()
        .set_own_value(m.as_object().unwrap(), ky, Value::number(100.0))
        .unwrap();
    assert_eq!(get_y(&mut vm), 100.0);

    // Changing o's prototype.
    let q = obj_with(&mut vm, None, &[("y", 9.0)]);
    vm.heap_mut()
        .set_prototype(o.as_object().unwrap(), q.as_object())
        .unwrap();
    assert_eq!(get_y(&mut vm), 9.0);

    // Deleting from the holder.
    vm.heap_mut()
        .delete_own(q.as_object().unwrap(), ky)
        .unwrap();
    let r = vm
        .call(&mut BasicHost::default(), get, Value::UNDEFINED, &[o])
        .unwrap();
    assert!(r.is_undefined());

    // An accessor on the chain is called, never cached.
    let getter = vm
        .register_native("getY", |_cx, _this, _args| Ok(Value::number(55.0)))
        .unwrap();
    vm.heap_mut()
        .define_own(
            q.as_object().unwrap(),
            ky,
            Property {
                attrs: Attrs(Attrs::CONFIGURABLE),
                slot: Slot::Accessor {
                    get: getter,
                    set: Value::UNDEFINED,
                },
            },
        )
        .unwrap();
    for _ in 0..3 {
        let r = vm
            .call(&mut BasicHost::default(), get, Value::UNDEFINED, &[o])
            .unwrap();
        assert_eq!(num(r), 55.0);
    }
}

#[test]
fn store_ic_caches_the_add_transition() {
    let mut vm = vm();
    let fns = closures(&mut vm, vec![setter_fn("x"), getter_fn("x")]);
    let (set, get) = (fns[0], fns[1]);
    let code = vm.children(cinder::vm::CodeId(0))[0];
    let mut host = BasicHost::default();
    let mut last = None;
    for i in 0..5 {
        let o = obj_with(&mut vm, None, &[]);
        vm.call(
            &mut host,
            set,
            Value::UNDEFINED,
            &[o, Value::number(f64::from(i))],
        )
        .unwrap();
        let r = vm.call(&mut host, get, Value::UNDEFINED, &[o]).unwrap();
        assert_eq!(num(r), f64::from(i));
        if let Some(prev) = last {
            let s1 = vm.heap().shape_of(o.as_object().unwrap()).unwrap();
            let s0 = vm.heap().shape_of(prev).unwrap();
            assert_eq!(s0, s1, "fresh objects share the shape");
        }
        last = o.as_object();
    }
    let s = vm.ic_site(code, 0).unwrap();
    assert_eq!(s.misses, 1, "only the first add misses");
    assert_eq!(s.hits, 4);

    // A read-only x on the prototype must stop the cached add.
    let proto = vm.object_prototype();
    let kx = vm.heap_mut().key_from_str("x").unwrap();
    vm.heap_mut()
        .define_own(
            proto,
            kx,
            Property {
                attrs: Attrs(Attrs::CONFIGURABLE),
                slot: Slot::Data(Value::number(0.0)),
            },
        )
        .unwrap();
    let o = obj_with(&mut vm, None, &[]);
    let err = vm
        .call(&mut host, set, Value::UNDEFINED, &[o, Value::number(1.0)])
        .unwrap_err();
    assert_eq!(
        err.message(),
        "TypeError: Cannot assign to read only property 'x' of object"
    );
}

#[test]
fn array_and_string_length() {
    let mut b = CodeBlock::new("main", 0, 5);
    let len = b.add_key("length");
    let ic = b.add_ic();
    let s = b.add_string("héllo");
    b.emit(Op::NewArray { dst: 0 });
    b.emit(int(1, 9));
    b.emit(Op::SetElem {
        obj: 0,
        key: 1,
        src: 1,
    });
    b.emit(Op::GetProp {
        dst: 2,
        obj: 0,
        key: len,
        ic,
    });
    b.emit(Op::LoadString { dst: 3, idx: s });
    let ic2 = b.add_ic();
    b.emit(Op::GetProp {
        dst: 4,
        obj: 3,
        key: len,
        ic: ic2,
    });
    b.emit(bin(BinOp::Mul, 2, 2, 4));
    b.emit(Op::Return { src: 2 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 50.0);

    // Shrinking the length drops elements; arrays join with commas.
    let mut b = CodeBlock::new("main", 0, 3);
    let len = b.add_key("length");
    let ic = b.add_ic();
    b.emit(Op::NewArray { dst: 0 });
    for i in 0..4 {
        b.emit(int(1, i));
        b.emit(Op::SetElem {
            obj: 0,
            key: 1,
            src: 1,
        });
    }
    b.emit(int(2, 2));
    b.emit(Op::SetProp {
        obj: 0,
        src: 2,
        key: len,
        ic,
    });
    b.emit(Op::Return { src: 0 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "0,1");
}

fn catch_handler(start: u32, end: u32, target: u32, reg: u16) -> Handler {
    Handler {
        start,
        end,
        target,
        catch_reg: reg,
        env_depth: 0,
    }
}

#[test]
fn try_catch_finally() {
    // try { throw 1 } catch (e) { r = e + 10 } finally { r = r * 2 }
    let mut b = CodeBlock::new("main", 0, 3);
    b.emit(int(0, 1)); // 0
    b.emit(Op::Throw { src: 0 }); // 1
    b.emit(Op::Jump { offset: 2 }); // 2 -> 5 (finally)
    b.emit(int(2, 10)); // 3 catch: r1 = e + 10
    b.emit(bin(BinOp::Add, 1, 1, 2)); // 4
    b.emit(int(2, 2)); // 5 finally
    b.emit(bin(BinOp::Mul, 1, 1, 2)); // 6
    b.emit(Op::Return { src: 1 }); // 7
    b.handlers.push(catch_handler(0, 3, 3, 1));
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 22.0);

    // try { throw "x" } finally { flag = 1 } -- rethrows after finally.
    let mut b = CodeBlock::new("main", 0, 3);
    let k = b.add_key("flag");
    let ic = b.add_ic();
    let s = b.add_string("boom");
    b.emit(Op::LoadString { dst: 0, idx: s }); // 0
    b.emit(Op::Throw { src: 0 }); // 1
    b.emit(int(2, 1)); // 2 finally (catch-all)
    b.emit(Op::SetGlobal { src: 2, key: k, ic }); // 3
    b.emit(Op::Throw { src: 1 }); // 4 rethrow
    b.handlers.push(catch_handler(0, 2, 2, 1));
    let mut vm = vm();
    vm.define_global("flag", Value::number(0.0)).unwrap();
    let err = run_in(&mut vm, b).unwrap_err();
    assert_eq!(err.message(), "boom");
    assert!(err.is_catchable());
    assert_eq!(vm.get_global("flag").map(num), Some(1.0));
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn throw_across_frames() {
    // function inner(o) { return o.x }       -- o is undefined: TypeError
    // function middle() { return inner() }
    // main: try { middle() } catch (e) { return e }
    let inner = getter_fn("x");
    let mut middle = CodeBlock::new("middle", 0, 3);
    let k = middle.add_key("inner");
    let ic = middle.add_ic();
    middle.emit(Op::GetGlobal { dst: 0, key: k, ic });
    middle.emit(Op::LoadUndefined { dst: 1 });
    middle.emit(Op::Call {
        dst: 2,
        callee: 0,
        this: 1,
        args: 1,
        argc: 0,
    });
    middle.emit(Op::Return { src: 2 });

    let mut vm = vm();
    let fns = closures(&mut vm, vec![inner, middle]);
    vm.define_global("inner", fns[0]).unwrap();
    vm.define_global("middle", fns[1]).unwrap();

    let mut main = CodeBlock::new("main", 0, 3);
    let k = main.add_key("middle");
    let ic = main.add_ic();
    main.emit(Op::GetGlobal { dst: 0, key: k, ic }); // 0
    main.emit(Op::LoadUndefined { dst: 1 }); // 1
    main.emit(Op::Call {
        dst: 2,
        callee: 0,
        this: 1,
        args: 1,
        argc: 0,
    }); // 2
    main.emit(Op::Return { src: 2 }); // 3
    main.emit(Op::Return { src: 1 }); // 4 catch
    main.handlers.push(catch_handler(0, 4, 4, 1));
    let caught = run_in(&mut vm, main.clone()).unwrap();
    assert_eq!(
        text(&mut vm, caught),
        "TypeError: Cannot read properties of undefined (reading 'x')"
    );
    let proto = vm
        .heap()
        .get_prototype(caught.as_object().unwrap())
        .unwrap();
    assert_eq!(
        proto,
        Some(vm.error_prototype(cinder::vm::ErrorKind::Type))
    );

    // Without the handler it comes out with a stack, innermost first.
    main.handlers.clear();
    let err = run_in(&mut vm, main).unwrap_err();
    let RunError::Thrown { message, stack, .. } = &err else {
        panic!("expected a thrown error, got {err:?}");
    };
    assert_eq!(
        message,
        "TypeError: Cannot read properties of undefined (reading 'x')"
    );
    let names: Vec<&str> = stack.iter().map(|l| l.function.as_str()).collect();
    assert_eq!(names, ["get", "middle", "main"]);
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn reference_errors() {
    let mut b = CodeBlock::new("main", 0, 1);
    let k = b.add_key("nope");
    let ic = b.add_ic();
    b.emit(Op::GetGlobal { dst: 0, key: k, ic });
    b.emit(Op::Return { src: 0 });
    let (_, r) = run(b);
    assert_eq!(
        r.unwrap_err().message(),
        "ReferenceError: nope is not defined"
    );

    let mut b = CodeBlock::new("main", 0, 1);
    let k = b.add_key("x");
    b.emit(Op::LoadUninit { dst: 0 });
    b.emit(Op::CheckTdz { src: 0, name: k });
    b.emit(Op::Return { src: 0 });
    let (_, r) = run(b);
    assert_eq!(
        r.unwrap_err().message(),
        "ReferenceError: Cannot access 'x' before initialization"
    );

    let mut b = CodeBlock::new("main", 0, 2);
    b.emit(int(0, 1));
    b.emit(Op::LoadUndefined { dst: 1 });
    b.emit(Op::Call {
        dst: 0,
        callee: 0,
        this: 1,
        args: 0,
        argc: 0,
    });
    b.emit(Op::Return { src: 0 });
    let (_, r) = run(b);
    assert_eq!(r.unwrap_err().message(), "TypeError: 1 is not a function");
}

#[test]
fn constructors_and_instanceof() {
    // function Point(x) { this.x = x }
    let mut point = CodeBlock::new("Point", 1, 2);
    let k = point.add_key("x");
    let ic = point.add_ic();
    point.emit(Op::LoadThis { dst: 1 });
    point.emit(Op::SetProp {
        obj: 1,
        src: 0,
        key: k,
        ic,
    });
    point.emit(Op::LoadUndefined { dst: 1 });
    point.emit(Op::Return { src: 1 });

    let mut b = CodeBlock::new("main", 0, 5);
    b.functions.push(point);
    let kx = b.add_key("x");
    let ic = b.add_ic();
    b.emit(Op::Closure { dst: 0, func: 0 });
    b.emit(int(1, 7));
    b.emit(Op::New {
        dst: 2,
        callee: 0,
        args: 1,
        argc: 1,
    });
    b.emit(Op::GetProp {
        dst: 3,
        obj: 2,
        key: kx,
        ic,
    });
    b.emit(bin(BinOp::InstanceOf, 4, 2, 0));
    b.emit(bin(BinOp::Add, 3, 3, 4)); // 7 + true
    b.emit(Op::Return { src: 3 });
    let (_, r) = run(b);
    assert_eq!(num(r.unwrap()), 8.0);
}

fn twice(_cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = args.first().and_then(|v| v.as_number()).unwrap_or(f64::NAN);
    Ok(Value::number(n * 2.0))
}

fn apply(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let f = args.first().copied().unwrap_or(Value::UNDEFINED);
    let x = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(f);
    // Allocate across the callback to make sure collections are safe.
    let keep = match cx.heap().new_object(None) {
        Ok(o) => Value::object(o),
        Err(_) => return Err(cx.type_error("out of memory")),
    };
    cx.keep(keep);
    let r = cx.call(f, Value::UNDEFINED, &[x])?;
    if cx.heap().object(keep.as_object().unwrap()).is_err() {
        return Err(cx.type_error("a kept object was collected"));
    }
    Ok(r)
}

#[test]
fn natives_and_callbacks() {
    let mut vm = vm();
    let t = vm.register_native("twice", twice).unwrap();
    let a = vm.register_native("apply", apply).unwrap();
    vm.define_global("twice", t).unwrap();
    vm.define_global("apply", a).unwrap();
    let fns = closures(&mut vm, vec![add_fn()]);
    vm.define_global("add", fns[0]).unwrap();

    // apply(twice, 21) + apply(add, 1)  -- add(1) is NaN
    let mut b = CodeBlock::new("main", 0, 6);
    let ka = b.add_key("apply");
    let kt = b.add_key("twice");
    let ic1 = b.add_ic();
    let ic2 = b.add_ic();
    b.emit(Op::GetGlobal {
        dst: 0,
        key: ka,
        ic: ic1,
    });
    b.emit(Op::LoadUndefined { dst: 1 });
    b.emit(Op::GetGlobal {
        dst: 2,
        key: kt,
        ic: ic2,
    });
    b.emit(int(3, 21));
    b.emit(Op::Call {
        dst: 4,
        callee: 0,
        this: 1,
        args: 2,
        argc: 2,
    });
    b.emit(Op::Return { src: 4 });
    let r = run_in(&mut vm, b).unwrap();
    assert_eq!(num(r), 42.0);

    // A native calling a script function that throws: the error passes
    // through the native and can be caught by script outside it.
    let r = vm.call(
        &mut BasicHost::default(),
        a,
        Value::UNDEFINED,
        &[Value::number(1.0), Value::UNDEFINED],
    );
    assert_eq!(r.unwrap_err().message(), "TypeError: 1 is not a function");
}

#[test]
fn script_getters_and_setters_run_as_frames() {
    // A getter written in script: function () { return this.v * 2 }
    let mut g = CodeBlock::new("getter", 0, 3);
    let kv = g.add_key("v");
    let ic = g.add_ic();
    g.emit(Op::LoadThis { dst: 0 });
    g.emit(Op::GetProp {
        dst: 1,
        obj: 0,
        key: kv,
        ic,
    });
    g.emit(int(2, 2));
    g.emit(bin(BinOp::Mul, 1, 1, 2));
    g.emit(Op::Return { src: 1 });
    // A setter: function (n) { this.v = n + 1 }
    let mut s = CodeBlock::new("setter", 1, 3);
    let kv2 = s.add_key("v");
    let ic2 = s.add_ic();
    s.emit(Op::LoadThis { dst: 1 });
    s.emit(int(2, 1));
    s.emit(bin(BinOp::Add, 2, 0, 2));
    s.emit(Op::SetProp {
        obj: 1,
        src: 2,
        key: kv2,
        ic: ic2,
    });
    s.emit(Op::Return { src: 2 });

    let mut vm = vm();
    let fns = closures(&mut vm, vec![g, s, setter_fn("d"), getter_fn("d")]);
    let o = obj_with(&mut vm, None, &[("v", 5.0)]);
    let kd = vm.heap_mut().key_from_str("d").unwrap();
    vm.heap_mut()
        .define_own(
            o.as_object().unwrap(),
            kd,
            Property {
                attrs: Attrs(Attrs::CONFIGURABLE),
                slot: Slot::Accessor {
                    get: fns[0],
                    set: fns[1],
                },
            },
        )
        .unwrap();
    let mut host = BasicHost::default();
    let r = vm.call(&mut host, fns[3], Value::UNDEFINED, &[o]).unwrap();
    assert_eq!(num(r), 10.0);
    vm.call(
        &mut host,
        fns[2],
        Value::UNDEFINED,
        &[o, Value::number(20.0)],
    )
    .unwrap();
    let r = vm.call(&mut host, fns[3], Value::UNDEFINED, &[o]).unwrap();
    assert_eq!(num(r), 42.0);
}

#[test]
fn join_handles_nesting_and_cycles() {
    // a = [1, [2, 3], null]; a[3] = a; "" + a
    let mut b = CodeBlock::new("main", 0, 5);
    let empty = b.add_string("");
    b.emit(Op::NewArray { dst: 0 });
    b.emit(Op::NewArray { dst: 1 });
    b.emit(int(2, 0));
    b.emit(int(3, 2));
    b.emit(Op::SetElem {
        obj: 1,
        key: 2,
        src: 3,
    });
    b.emit(int(2, 1));
    b.emit(int(3, 3));
    b.emit(Op::SetElem {
        obj: 1,
        key: 2,
        src: 3,
    });
    b.emit(int(2, 0));
    b.emit(int(3, 1));
    b.emit(Op::SetElem {
        obj: 0,
        key: 2,
        src: 3,
    });
    b.emit(int(2, 1));
    b.emit(Op::SetElem {
        obj: 0,
        key: 2,
        src: 1,
    });
    b.emit(int(2, 2));
    b.emit(Op::LoadNull { dst: 3 });
    b.emit(Op::SetElem {
        obj: 0,
        key: 2,
        src: 3,
    });
    b.emit(int(2, 3));
    b.emit(Op::SetElem {
        obj: 0,
        key: 2,
        src: 0,
    });
    b.emit(Op::LoadString { dst: 4, idx: empty });
    b.emit(bin(BinOp::Add, 4, 4, 0));
    b.emit(Op::Return { src: 4 });
    let (mut vm, r) = run(b);
    let v = r.unwrap();
    assert_eq!(text(&mut vm, v), "1,2,3,,");
}

#[test]
fn disassembly_of_loaded_code() {
    let mut vm = vm();
    let id = load(&mut vm, sum_loop());
    let text = cinder::bytecode::disasm::disassemble(vm.code(id).unwrap());
    assert!(text.starts_with("function main (params 0, registers 4, ic sites 0)\n"));
    assert!(text.contains("  0003  loop 0\n"));
    assert!(text.contains("  0008  jump 3\n"));
}
