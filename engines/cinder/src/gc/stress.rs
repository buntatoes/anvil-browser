// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Seeded stress tests for the collector.
//!
//! Each test builds random object graphs next to a plain Rust model of the
//! same graph, collects, and checks the heap against the model: everything
//! reachable is intact, everything unreachable is gone, and every handle to
//! a freed object gives an error. Failures print the seed, so a run can be
//! repeated exactly.

use super::prng::Prng;
use super::{Heap, HeapError};
use crate::cinder::limits::{LimitKind, Limits};
use crate::cinder::value::atom::PropertyKey;
use crate::cinder::value::{ObjectRef, Value};
use std::collections::{HashMap, HashSet};

const EDGES: usize = 4;

struct ModelObject {
    handle: ObjectRef,
    id: i32,
    edges: [Option<usize>; EDGES],
    text: Option<String>,
}

struct Keys {
    id: PropertyKey,
    text: PropertyKey,
    edges: [PropertyKey; EDGES],
}

fn keys(h: &mut Heap) -> Keys {
    Keys {
        id: h.key_from_str("id").unwrap(),
        text: h.key_from_str("text").unwrap(),
        edges: [
            h.key_from_str("p0").unwrap(),
            h.key_from_str("p1").unwrap(),
            h.key_from_str("p2").unwrap(),
            h.key_from_str("p3").unwrap(),
        ],
    }
}

fn data(h: &Heap, o: ObjectRef, k: PropertyKey) -> Option<Value> {
    h.get_own(o, k).unwrap().and_then(|p| p.slot.data())
}

fn reachable(model: &HashMap<usize, ModelObject>, roots: &HashSet<usize>) -> HashSet<usize> {
    let mut seen = HashSet::new();
    let mut stack: Vec<usize> = roots.iter().copied().collect();
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        if let Some(m) = model.get(&i) {
            stack.extend(m.edges.iter().flatten().copied());
        }
    }
    seen
}

fn run_seed(seed: u64, steps: usize) {
    let limits = Limits {
        initial_gc_trigger: 64 * 1024,
        rope_min_length: 8,
        ..Limits::default()
    };
    let mut h = Heap::new(limits).unwrap();
    let k = keys(&mut h);
    let mut rng = Prng::new(seed);
    let mut model: HashMap<usize, ModelObject> = HashMap::new();
    let mut roots: HashSet<usize> = HashSet::new();
    let mut dead: Vec<ObjectRef> = Vec::new();
    let mut next_id = 0usize;

    for step in 0..steps {
        let ids: Vec<usize> = {
            let mut v: Vec<usize> = model.keys().copied().collect();
            v.sort_unstable();
            v
        };
        match rng.below(100) {
            0..=34 => {
                let o = h.new_object(None).unwrap();
                h.set_own_value(o, k.id, Value::int32(next_id as i32))
                    .unwrap();
                let mut text = None;
                if rng.chance(30) {
                    let mut s = h.alloc_string("s").unwrap();
                    let mut want = String::from("s");
                    for _ in 0..rng.below(6) {
                        let piece = format!("{}-", rng.below(1000));
                        let p = h.alloc_string(&piece).unwrap();
                        s = h.concat(s, p).unwrap();
                        want.push_str(&piece);
                    }
                    h.set_own_value(o, k.text, Value::string(s)).unwrap();
                    text = Some(want);
                }
                model.insert(
                    next_id,
                    ModelObject {
                        handle: o,
                        id: next_id as i32,
                        edges: [None; EDGES],
                        text,
                    },
                );
                // New objects start rooted, so they survive until dropped.
                roots.insert(next_id);
                next_id += 1;
            }
            35..=64 if !ids.is_empty() => {
                let a = ids[rng.below(ids.len())];
                let b = ids[rng.below(ids.len())];
                let e = rng.below(EDGES);
                let target = model[&b].handle;
                let src = model[&a].handle;
                h.set_own_value(src, k.edges[e], Value::object(target))
                    .unwrap();
                if let Some(m) = model.get_mut(&a) {
                    m.edges[e] = Some(b);
                }
            }
            65..=74 if !ids.is_empty() => {
                let a = ids[rng.below(ids.len())];
                let e = rng.below(EDGES);
                let src = model[&a].handle;
                h.delete_own(src, k.edges[e]).unwrap();
                if let Some(m) = model.get_mut(&a) {
                    m.edges[e] = None;
                }
            }
            75..=89 if !roots.is_empty() => {
                let mut r: Vec<usize> = roots.iter().copied().collect();
                r.sort_unstable();
                let pick = r[rng.below(r.len())];
                roots.remove(&pick);
            }
            _ => {
                let root_values: Vec<Value> = roots
                    .iter()
                    .map(|i| Value::object(model[i].handle))
                    .collect();
                h.collect(&root_values);
                let live = reachable(&model, &roots);
                let all: Vec<usize> = model.keys().copied().collect();
                for i in all {
                    let m = &model[&i];
                    if live.contains(&i) {
                        assert!(
                            h.is_live(m.handle),
                            "seed {seed} step {step}: reachable object {i} was freed"
                        );
                        let id = data(&h, m.handle, k.id).and_then(Value::as_int32);
                        assert_eq!(id, Some(m.id), "seed {seed} step {step}: id of {i}");
                        for (e, want) in m.edges.iter().enumerate() {
                            let got = data(&h, m.handle, k.edges[e]).and_then(Value::as_object);
                            assert_eq!(
                                got,
                                want.map(|w| model[&w].handle),
                                "seed {seed} step {step}: edge {e} of {i}"
                            );
                        }
                        if let Some(want) = &m.text {
                            let s = data(&h, m.handle, k.text)
                                .and_then(Value::as_string)
                                .unwrap();
                            assert_eq!(
                                &h.string_to_rust(s).unwrap(),
                                want,
                                "seed {seed} step {step}: text of {i}"
                            );
                        }
                    } else {
                        assert!(
                            !h.is_live(m.handle),
                            "seed {seed} step {step}: unreachable object {i} survived"
                        );
                        dead.push(m.handle);
                        model.remove(&i);
                    }
                }
                for d in &dead {
                    assert!(
                        matches!(h.object(*d), Err(HeapError::Internal(_))),
                        "seed {seed} step {step}: a dead handle still worked"
                    );
                }
                assert_eq!(
                    h.bytes(),
                    h.recount_bytes(),
                    "seed {seed} step {step}: byte count drifted"
                );
                assert_eq!(
                    h.live_objects(),
                    model.len(),
                    "seed {seed} step {step}: live count"
                );
            }
        }
    }
    // Drop everything: the heap goes back to only the atoms.
    roots.clear();
    h.collect(&[]);
    assert_eq!(h.live_objects(), 0, "seed {seed}");
    assert_eq!(h.live_strings(), 0, "seed {seed}");
    assert_eq!(h.live_shapes(), 0, "seed {seed}");
    assert_eq!(
        h.bytes(),
        h.atoms().bytes(),
        "seed {seed}: bytes back to the atom baseline"
    );
}

#[test]
fn random_graphs_many_seeds() {
    for seed in 1..=24 {
        run_seed(seed, 1500);
    }
}

#[test]
fn random_graph_long_run() {
    run_seed(0xA11F_00D5, 12_000);
}

#[test]
fn cycles_are_collected() {
    let mut h = Heap::new(Limits::default()).unwrap();
    let next = h.key_from_str("next").unwrap();
    let me = h.key_from_str("me").unwrap();
    let ring: Vec<ObjectRef> = (0..1000).map(|_| h.new_object(None).unwrap()).collect();
    for (i, o) in ring.iter().enumerate() {
        let n = ring[(i + 1) % ring.len()];
        h.set_own_value(*o, next, Value::object(n)).unwrap();
        h.set_own_value(*o, me, Value::object(*o)).unwrap();
    }
    h.collect(&[Value::object(ring[500])]);
    assert_eq!(h.live_objects(), 1000, "a rooted ring stays");
    let stats = h.collect(&[]);
    assert_eq!(
        stats.freed_objects, 1000,
        "an unrooted ring goes, cycles and all"
    );
    assert_eq!(h.live_objects(), 0);
}

#[test]
fn deep_chain_marks_without_recursion() {
    let mut h = Heap::new(Limits::default()).unwrap();
    let next = h.key_from_str("next").unwrap();
    let head = h.new_object(None).unwrap();
    let mut cur = head;
    for _ in 0..300_000 {
        let o = h.new_object(None).unwrap();
        h.set_own_value(cur, next, Value::object(o)).unwrap();
        cur = o;
    }
    // Also a deep rope and a deep prototype chain.
    let mut s = h.alloc_string("x").unwrap();
    for _ in 0..100_000 {
        let p = h.alloc_string("y").unwrap();
        s = h.concat(s, p).unwrap();
    }
    let mut proto = h.new_object(None).unwrap();
    for _ in 0..50_000 {
        proto = h.new_object(Some(proto)).unwrap();
    }
    h.collect(&[Value::object(head), Value::string(s), Value::object(proto)]);
    assert_eq!(h.live_objects(), 300_001 + 50_001);
    assert_eq!(h.string_len(s).unwrap(), 100_001);
    assert_eq!(h.bytes(), h.recount_bytes());
    h.collect(&[]);
    assert_eq!(h.live_objects(), 0);
    assert_eq!(h.live_strings(), 0);
}

#[test]
fn generation_wrap_retires_slot_through_the_heap() {
    let mut h = Heap::new(Limits::default()).unwrap();
    let first = h.new_object(None).unwrap();
    let mut old = vec![first];
    let mut cur = first;
    for _ in 0..0x7FFF {
        h.collect(&[]);
        cur = h.new_object(None).unwrap();
        if old.len() < 64 {
            old.push(cur);
        }
    }
    assert_eq!(cur.0.index, first.0.index);
    assert_eq!(cur.0.generation, 0x7FFF);
    h.collect(&[]);
    let after = h.new_object(None).unwrap();
    assert_ne!(
        after.0.index, first.0.index,
        "a worn-out slot is never reused"
    );
    for o in old {
        assert!(h.object(o).is_err(), "old handle {o} must be stale");
    }
}

#[test]
fn stale_handles_never_reach_new_objects() {
    let mut rng = Prng::new(99);
    let mut h = Heap::new(Limits::default()).unwrap();
    let x = h.key_from_str("x").unwrap();
    let mut dead = Vec::new();
    for round in 0..200 {
        let batch: Vec<ObjectRef> = (0..rng.below(50) + 1)
            .map(|_| h.new_object(None).unwrap())
            .collect();
        for o in &batch {
            h.set_own_value(*o, x, Value::int32(round)).unwrap();
        }
        let keep: Vec<Value> = batch
            .iter()
            .filter(|_| rng.chance(50))
            .map(|o| Value::object(*o))
            .collect();
        h.collect(&keep);
        for o in &batch {
            if !keep.iter().any(|v| v.as_object() == Some(*o)) {
                dead.push(*o);
            }
        }
        for d in &dead {
            assert!(
                h.object(*d).is_err(),
                "round {round}: stale handle {d} worked"
            );
            assert!(h.set_own_value(*d, x, Value::NULL).is_err());
        }
    }
}

#[test]
fn byte_budget_under_random_load_aborts_with_reason() {
    for seed in 1..=8u64 {
        let limits = Limits {
            max_heap_bytes: 256 * 1024,
            initial_gc_trigger: 32 * 1024,
            ..Limits::default()
        };
        let mut h = Heap::new(limits).unwrap();
        let mut rng = Prng::new(seed);
        let k = h.key_from_str("v").unwrap();
        let mut kept: Vec<Value> = Vec::new();
        let mut aborted = None;
        for _ in 0..200_000 {
            if let Err(e) = h.safepoint(&kept) {
                aborted = Some(e);
                break;
            }
            let r = if rng.chance(50) {
                h.new_object(None).and_then(|o| {
                    h.set_own_value(o, k, Value::int32(1))?;
                    Ok(Value::object(o))
                })
            } else {
                let len = rng.below(200) + 1;
                h.alloc_string(&"z".repeat(len)).map(Value::string)
            };
            match r {
                Ok(v) => {
                    if rng.chance(40) {
                        kept.push(v);
                    }
                }
                Err(e) => {
                    aborted = Some(e);
                    break;
                }
            }
            assert!(
                h.bytes() <= h.limits().max_heap_bytes,
                "seed {seed}: went past the limit"
            );
        }
        match aborted {
            Some(HeapError::Abort(a)) => {
                assert_eq!(a.kind, LimitKind::HeapBytes, "seed {seed}");
                assert!(
                    a.reason.starts_with("heap limit reached:"),
                    "seed {seed}: {}",
                    a.reason
                );
                assert!(a.reason.contains("262,144"), "seed {seed}: {}", a.reason);
            }
            other => panic!("seed {seed}: expected a heap limit abort, got {other:?}"),
        }
        assert!(
            h.stats().collections > 0,
            "seed {seed}: collections ran before the limit"
        );
        assert_eq!(h.bytes(), h.recount_bytes(), "seed {seed}");
    }
}
