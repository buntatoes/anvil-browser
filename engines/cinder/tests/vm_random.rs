// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
#![forbid(unsafe_code)]
//! Seeded random programs.
//!
//! 1. Random valid programs (they pass `validate`) must never panic and
//!    must end with a value, a script exception, or a limit abort. Never an
//!    internal error.
//! 2. Random property programs must give the same result with inline
//!    caches on and off.

mod vm_support;

use cinder::bytecode::{BinOp, CodeBlock, Handler, Op, UnOp};
use cinder::limits::{LimitKind, Limits};
use cinder::value::atom::PropertyKey;
use cinder::value::object::{ObjectClass, Slot};
use cinder::value::{ObjectRef, Value};
use cinder::vm::{BasicHost, RunError, Vm};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use vm_support::*;

/// A small seeded generator (splitmix64).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn reg(&mut self, regs: u16) -> u16 {
        self.below(regs as usize) as u16
    }
}

const KEYS: [&str; 6] = ["a", "b", "c", "length", "0", "prototype"];

fn random_block(rng: &mut Rng, name: &str, params: u16, nested: bool) -> CodeBlock {
    let regs = 8u16;
    let mut b = CodeBlock::new(name, params, u32::from(regs));
    for k in KEYS {
        b.add_key(k);
    }
    for s in ["", "x", "10", "-0"] {
        b.add_string(s);
    }
    for n in [0.5, -0.0, f64::NAN, f64::INFINITY, 1e21, 4_294_967_296.0] {
        b.add_number(n);
    }
    b.loops = 2;
    if nested {
        b.functions.push(random_block(rng, "inner", 2, false));
    }
    let len = 8 + rng.below(40);
    for _ in 0..len {
        let r = rng.reg(regs);
        let s = rng.reg(regs);
        let t = rng.reg(regs);
        let key = rng.below(KEYS.len()) as u32;
        let op = match rng.below(26) {
            0 => Op::LoadInt {
                dst: r,
                value: [0, 1, -1, 2, 7, i32::MAX, i32::MIN][rng.below(7)],
            },
            1 => Op::LoadNumber {
                dst: r,
                idx: rng.below(b.numbers.len()) as u32,
            },
            2 => Op::LoadString {
                dst: r,
                idx: rng.below(b.strings.len()) as u32,
            },
            3 => [
                Op::LoadUndefined { dst: r },
                Op::LoadNull { dst: r },
                Op::LoadTrue { dst: r },
                Op::LoadFalse { dst: r },
                Op::LoadThis { dst: r },
            ][rng.below(5)],
            4 => Op::Move { dst: r, src: s },
            5..=8 => Op::Binary {
                op: BinOp::ALL[rng.below(BinOp::ALL.len())],
                dst: r,
                a: s,
                b: t,
            },
            9 => Op::Unary {
                op: UnOp::ALL[rng.below(UnOp::ALL.len())],
                dst: r,
                src: s,
            },
            10 | 11 => {
                // A jump anywhere in the block; patched once the length is known.
                let target = rng.below(len) as i32;
                let off = target - (b.ops.len() as i32 + 1);
                match rng.below(4) {
                    0 => Op::Jump { offset: off },
                    1 => Op::JumpIfTrue {
                        cond: r,
                        offset: off,
                    },
                    2 => Op::JumpIfFalse {
                        cond: r,
                        offset: off,
                    },
                    _ => Op::JumpIfNullish {
                        cond: r,
                        offset: off,
                    },
                }
            }
            12 => Op::LoopHeader {
                id: rng.below(2) as u32,
            },
            13 => Op::NewObject { dst: r },
            14 => Op::NewArray { dst: r },
            15 | 16 => {
                let ic = b.add_ic();
                Op::GetProp {
                    dst: r,
                    obj: s,
                    key,
                    ic,
                }
            }
            17 => {
                let ic = b.add_ic();
                Op::SetProp {
                    obj: r,
                    src: s,
                    key,
                    ic,
                }
            }
            18 => Op::GetElem {
                dst: r,
                obj: s,
                key: t,
            },
            19 => Op::SetElem {
                obj: r,
                key: s,
                src: t,
            },
            20 => Op::DeleteElem {
                dst: r,
                obj: s,
                key: t,
            },
            21 if nested => Op::Closure { dst: r, func: 0 },
            21 | 22 => {
                let argc = rng.below(3) as u16;
                let args = rng.below((regs - argc) as usize + 1) as u16;
                Op::Call {
                    dst: r,
                    callee: s,
                    this: t,
                    args,
                    argc,
                }
            }
            23 => {
                let argc = rng.below(3) as u16;
                let args = rng.below((regs - argc) as usize + 1) as u16;
                Op::New {
                    dst: r,
                    callee: s,
                    args,
                    argc,
                }
            }
            24 => {
                let ic = b.add_ic();
                Op::GetGlobal { dst: r, key, ic }
            }
            _ => Op::Throw { src: r },
        };
        b.emit(op);
    }
    // Jumps were aimed at 0..len; the block has exactly `len` ops so far.
    b.emit(Op::Return { src: rng.reg(regs) });
    for _ in 0..rng.below(3) {
        let start = rng.below(len) as u32;
        let end = start + rng.below(len - start as usize + 1) as u32;
        b.handlers.push(Handler {
            start,
            end,
            target: rng.below(len + 1) as u32,
            catch_reg: rng.reg(regs),
            env_depth: 0,
        });
    }
    b
}

fn random_limits() -> Limits {
    Limits {
        max_steps: Some(5_000),
        max_call_depth: 200,
        max_heap_bytes: 4 << 20,
        initial_gc_trigger: 64 << 10,
        max_string_length: 10_000,
        ..Limits::default()
    }
}

#[test]
fn random_programs_never_panic_or_fail_internally() {
    let mut counts: HashMap<&'static str, u32> = HashMap::new();
    for seed in 0..3_000u64 {
        let mut rng = Rng::new(seed);
        let block = random_block(&mut rng, "main", 0, true);
        if let Err(e) = block.validate() {
            panic!("seed {seed}: the generator made an invalid block: {e}");
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut vm = Vm::new(random_limits()).expect("limits");
            let r = run_in(&mut vm, block);
            // Thrown strings and arrays can print as nothing.
            let may_be_empty = match &r {
                Err(RunError::Thrown { value, .. }) => {
                    value.is_string()
                        || value
                            .as_object()
                            .is_some_and(|o| vm.heap().object(o).is_ok_and(|o| o.is_array()))
                }
                _ => false,
            };
            (r, vm.frame_count(), may_be_empty)
        }));
        let (r, frames, may_be_empty) = match outcome {
            Ok(x) => x,
            Err(_) => panic!("seed {seed}: the VM panicked"),
        };
        assert_eq!(frames, 0, "seed {seed}: frames left behind");
        let kind = match &r {
            Ok(_) => "value",
            Err(RunError::Thrown { message, value, .. }) => {
                assert!(
                    !message.is_empty() || may_be_empty,
                    "seed {seed}: empty message for {value:?}"
                );
                "thrown"
            }
            Err(RunError::Abort { kind, reason, .. }) => {
                assert!(
                    matches!(kind, LimitKind::Steps | LimitKind::HeapBytes),
                    "seed {seed}: unexpected abort {kind:?}: {reason}"
                );
                "abort"
            }
            Err(e @ RunError::Internal { .. }) | Err(e @ RunError::Compile { .. }) => {
                panic!("seed {seed}: {e}")
            }
        };
        *counts.entry(kind).or_default() += 1;
    }
    eprintln!("random program outcomes: {counts:?}");
    for k in ["value", "thrown", "abort"] {
        assert!(
            counts.get(k).copied().unwrap_or(0) > 0,
            "no '{k}' outcomes: {counts:?}"
        );
    }
}

const PKEYS: [&str; 5] = ["a", "b", "c", "d", "e"];

/// A property program. Registers:
/// r0 log array, r1..r4 objects, r5 value, r6 temp, r7 loop counter,
/// r8 a constructor, r9 its prototype.
fn property_program(rng: &mut Rng) -> CodeBlock {
    let mut b = CodeBlock::new("props", 0, 10);
    b.loops = 1;
    for k in PKEYS {
        b.add_key(k);
    }
    let klen = b.add_key("length");
    let kproto = b.add_key("prototype");
    for k in PKEYS {
        b.add_string(k);
    }
    let mut ctor = CodeBlock::new("Thing", 0, 1);
    ctor.emit(Op::LoadUndefined { dst: 0 });
    ctor.emit(Op::Return { src: 0 });
    b.functions.push(ctor);

    let log = |b: &mut CodeBlock, src: u16| {
        let ic = b.add_ic();
        b.emit(Op::GetProp {
            dst: 6,
            obj: 0,
            key: klen,
            ic,
        });
        b.emit(Op::SetElem {
            obj: 0,
            key: 6,
            src,
        });
    };
    b.emit(Op::NewArray { dst: 0 });
    b.emit(Op::Closure { dst: 8, func: 0 });
    let ic = b.add_ic();
    b.emit(Op::GetProp {
        dst: 9,
        obj: 8,
        key: kproto,
        ic,
    });
    log(&mut b, 9);
    for r in 1..=4 {
        if rng.below(2) == 0 {
            b.emit(Op::NewObject { dst: r });
        } else {
            b.emit(Op::New {
                dst: r,
                callee: 8,
                args: 0,
                argc: 0,
            });
        }
        log(&mut b, r);
    }
    b.emit(Op::LoadInt { dst: 7, value: 0 });
    let head = b.emit(Op::LoopHeader { id: 0 });
    for _ in 0..(5 + rng.below(25)) {
        let o = 1 + rng.below(4) as u16;
        let key = rng.below(PKEYS.len()) as u32;
        match rng.below(9) {
            0 | 1 => {
                let ic = b.add_ic();
                b.emit(Op::SetProp {
                    obj: o,
                    src: 5,
                    key,
                    ic,
                });
            }
            2 | 3 => {
                let ic = b.add_ic();
                b.emit(Op::GetProp {
                    dst: 5,
                    obj: o,
                    key,
                    ic,
                });
                log(&mut b, 5);
            }
            4 => {
                b.emit(Op::LoadInt {
                    dst: 5,
                    value: rng.below(100) as i32,
                });
            }
            5 => {
                b.emit(Op::Move {
                    dst: 5,
                    src: 1 + rng.below(4) as u16,
                });
            }
            6 => {
                if rng.below(2) == 0 {
                    b.emit(Op::NewObject { dst: o });
                } else {
                    b.emit(Op::New {
                        dst: o,
                        callee: 8,
                        args: 0,
                        argc: 0,
                    });
                }
                log(&mut b, o);
            }
            7 => {
                b.emit(Op::LoadString { dst: 6, idx: key });
                b.emit(Op::DeleteElem {
                    dst: 6,
                    obj: o,
                    key: 6,
                });
            }
            _ => {
                // Change the shared prototype, which moves the epoch.
                let ic = b.add_ic();
                b.emit(Op::SetProp {
                    obj: 9,
                    src: 5,
                    key,
                    ic,
                });
            }
        }
    }
    b.emit(Op::Unary {
        op: UnOp::Inc,
        dst: 7,
        src: 7,
    });
    b.emit(Op::LoadInt {
        dst: 6,
        value: 2 + rng.below(6) as i32,
    });
    b.emit(Op::Binary {
        op: BinOp::Lt,
        dst: 6,
        a: 7,
        b: 6,
    });
    let off = back_to(&b, head);
    b.emit(Op::JumpIfTrue {
        cond: 6,
        offset: off,
    });
    b.emit(Op::Return { src: 0 });
    b
}

/// Everything reachable from `root`, as text. Objects get numbers in the
/// order they are first seen, so two heaps with the same structure give
/// the same text. Walks with an explicit list, never recursion.
fn deep_dump(vm: &mut Vm, root: Value) -> String {
    let mut ids: HashMap<ObjectRef, usize> = HashMap::new();
    let mut order: Vec<ObjectRef> = Vec::new();
    let mut out = String::new();
    let fixed: HashMap<ObjectRef, &str> = [
        (vm.object_prototype(), "Object.prototype"),
        (vm.function_prototype(), "Function.prototype"),
        (vm.array_prototype(), "Array.prototype"),
    ]
    .into_iter()
    .collect();
    let name =
        |o: ObjectRef, ids: &mut HashMap<ObjectRef, usize>, order: &mut Vec<ObjectRef>| -> String {
            if let Some(n) = fixed.get(&o) {
                return (*n).to_string();
            }
            let next = ids.len();
            let id = *ids.entry(o).or_insert_with(|| {
                order.push(o);
                next
            });
            format!("#{id}")
        };
    let root_text = match root.as_object() {
        Some(o) => name(o, &mut ids, &mut order),
        None => text(vm, root),
    };
    out.push_str(&format!("root {root_text}\n"));
    let mut i = 0;
    while i < order.len() {
        let o = order[i];
        i += 1;
        let class = match vm.heap().object(o).unwrap().class {
            ObjectClass::Array { length } => format!("array({length})"),
            ObjectClass::Function { .. } => "function".to_string(),
            ObjectClass::Ordinary => "object".to_string(),
            ref c => format!("{c:?}"),
        };
        let proto = match vm.heap().get_prototype(o).unwrap() {
            Some(p) => name(p, &mut ids, &mut order),
            None => "null".to_string(),
        };
        out.push_str(&format!("#{} {class} proto {proto}:", ids[&o]));
        for key in vm.heap().own_keys(o).unwrap() {
            let prop = vm.heap().get_own(o, key).unwrap().unwrap();
            let k = match key {
                PropertyKey::Index(n) => n.to_string(),
                PropertyKey::Atom(a) => vm.heap().atoms().to_rust_string(a).unwrap(),
                PropertyKey::Symbol(_) => "symbol".to_string(),
            };
            let v = match prop.slot {
                Slot::Data(v) => v,
                Slot::Accessor { .. } => Value::UNDEFINED,
            };
            let vt = match v.as_object() {
                Some(c) => name(c, &mut ids, &mut order),
                None => text(vm, v),
            };
            out.push_str(&format!(" {k}={vt}/{}", prop.attrs.0));
        }
        out.push('\n');
    }
    out
}

fn run_props(block: CodeBlock, ics: bool) -> (Result<String, String>, u64) {
    let mut vm = vm();
    vm.set_ic_enabled(ics);
    let r = run_in(&mut vm, block);
    let hits = vm.ic_stats().hits;
    match r {
        Ok(v) => (Ok(deep_dump(&mut vm, v)), hits),
        Err(e) => (Err(e.to_string()), hits),
    }
}

#[test]
fn property_programs_match_with_and_without_inline_caches() {
    let mut total_hits = 0;
    for seed in 0..500u64 {
        let mut rng = Rng::new(1_000 + seed);
        let block = property_program(&mut rng);
        let (on, hits_on) = run_props(block.clone(), true);
        let (off, hits_off) = run_props(block, false);
        assert_eq!(
            on, off,
            "seed {seed}: results differ with caches on and off"
        );
        assert_eq!(hits_off, 0, "seed {seed}: caches were used while off");
        assert!(on.is_ok(), "seed {seed}: {on:?}");
        total_hits += hits_on;
    }
    eprintln!("property programs: {total_hits} cache hits with caches on");
    assert!(total_hits > 10_000, "caches barely used: {total_hits} hits");
}

#[test]
fn random_programs_with_ics_on_and_off_agree() {
    // The general random programs too: same outcome text either way.
    for seed in 0..500u64 {
        let mk = || random_block(&mut Rng::new(50_000 + seed), "main", 0, true);
        let go = |ics: bool| {
            let mut vm = Vm::new(random_limits()).unwrap();
            vm.set_ic_enabled(ics);
            match run_in(&mut vm, mk()) {
                Ok(v) => Ok(deep_dump(&mut vm, v)),
                Err(e) => Err(e.to_string()),
            }
        };
        assert_eq!(go(true), go(false), "seed {seed}");
    }
    let _ = BasicHost::default();
}
