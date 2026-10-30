// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The dispatch loop, calls, exception unwinding and property access.
//!
//! One loop runs every script frame. A call from script to script pushes a
//! frame and keeps going in the same loop, so the Rust stack only grows when
//! a native function calls back into script (counted by
//! `max_native_reentry`).
//!
//! Each op:
//!
//! 1. counts a step (the step budget is an uncatchable abort),
//! 2. every `interrupt_check_interval` steps, asks the host whether to stop,
//! 3. runs a collection if one was asked for (the safepoint: no Rust local
//!    holds an unrooted value here),
//! 4. runs.
//!
//! Errors in an op come back as [`Exit`]. Script exceptions unwind through
//! the handler tables; aborts and internal errors go straight out, without
//! running any handler.

use super::frame::{DISCARD, Frame, FrameKind};
use super::ic::IcEntry;
use super::{
    ErrorKind, Exit, Host, NativeCx, RunError, Vm, internal_err, range_err, steps_abort, type_err,
};
use crate::anvil_js::bytecode::{BinOp, Op, Reg, UnOp};
use crate::anvil_js::gc::Lookup;
use crate::anvil_js::limits::{LimitKind, group_digits};
use crate::anvil_js::value::atom::PropertyKey;
use crate::anvil_js::value::number::{number_to_string, to_uint32};
use crate::anvil_js::value::object::{ObjectClass, Property, Slot};
use crate::anvil_js::value::shape::Attrs;
use crate::anvil_js::value::{ObjectRef, Value};

/// What a property read found.
pub(crate) enum Got {
    Value(Value),
    Getter(Value),
}

/// What a property write did.
enum Put {
    Done,
    Setter(Value),
}

impl Vm {
    // ---- frames and calls -------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_frame(
        &mut self,
        code: u32,
        callee: Value,
        env: Option<ObjectRef>,
        this: Value,
        args: &[Value],
        dst: u32,
        kind: FrameKind,
    ) -> Result<(), Exit> {
        let max = self.heap.limits().max_call_depth as usize;
        let lc = self
            .codes
            .get(code as usize)
            .ok_or_else(|| internal_err(format!("no loaded code {code}")))?;
        if self.frames.len() >= max {
            return Err(range_err(format!(
                "Maximum call depth exceeded ({} frames) in function '{}'",
                group_digits(max as u64),
                lc.block.display_name()
            )));
        }
        let n = lc.block.register_count as usize;
        let params = lc.block.params as usize;
        let strict = lc.block.strict;
        let arguments_reg = lc.block.arguments_reg;
        let rest_reg = lc.block.rest_reg;
        let this = if strict || this.is_object() {
            this
        } else {
            self.coerce_this(this)?
        };
        let base = self.regs.len();
        self.regs.resize(base + n, Value::UNDEFINED);
        let take = params.min(args.len());
        self.regs[base..base + take].copy_from_slice(&args[..take]);
        if let Some(reg) = arguments_reg {
            let a = self.make_arguments(callee, args, params, strict)?;
            self.regs[base + reg as usize] = a;
        }
        if let Some(reg) = rest_reg {
            let arr = self.heap.new_array(Some(self.intr.array_prototype))?;
            for (i, v) in args.iter().skip(params).enumerate() {
                self.heap
                    .define_own(arr, PropertyKey::Index(i as u32), Property::data(*v))?;
            }
            self.regs[base + reg as usize] = Value::object(arr);
        }
        self.frames.push(Frame {
            code,
            pc: 0,
            base,
            dst,
            this,
            callee,
            env,
            env_depth: 0,
            kind,
        });
        Ok(())
    }

    /// Sloppy-mode `this`: `undefined` and `null` become the global object,
    /// other primitives get wrapped.
    fn coerce_this(&mut self, this: Value) -> Result<Value, Exit> {
        if this.is_nullish() || this.is_uninit() || this.is_hole() {
            return Ok(Value::object(self.intr.global));
        }
        let proto = if this.is_string() {
            self.intr.string_prototype
        } else if this.is_number() {
            self.intr.number_prototype
        } else if this.is_bool() {
            self.intr.boolean_prototype
        } else if this.is_symbol() {
            self.intr.symbol_prototype
        } else {
            self.intr.object_prototype
        };
        let o = self
            .heap
            .alloc_object(Some(proto), ObjectClass::Primitive(this))?;
        Ok(Value::object(o))
    }

    fn make_arguments(
        &mut self,
        callee: Value,
        args: &[Value],
        params: usize,
        strict: bool,
    ) -> Result<Value, Exit> {
        let mapped = if strict {
            Vec::new()
        } else {
            vec![false; params.min(args.len())]
        };
        let o = self.heap.alloc_object(
            Some(self.intr.object_prototype),
            ObjectClass::Arguments { env: None, mapped },
        )?;
        for (i, v) in args.iter().enumerate() {
            self.heap
                .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))?;
        }
        let hidden = Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE);
        self.heap.define_own(
            o,
            self.intr.key_length,
            Property {
                attrs: hidden,
                slot: Slot::Data(Value::number(args.len() as f64)),
            },
        )?;
        if !strict {
            self.heap.define_own(
                o,
                self.intr.key_callee,
                Property {
                    attrs: hidden,
                    slot: Slot::Data(callee),
                },
            )?;
        }
        Ok(Value::object(o))
    }

    /// Strict mode of the code running now (natives count as strict).
    pub(crate) fn strict_now(&self) -> bool {
        self.frames
            .last()
            .map(|f| self.codes[f.code as usize].block.strict)
            .unwrap_or(true)
    }

    /// A failed write or delete: strict code throws, sloppy code goes on.
    pub(crate) fn quiet_fail<T>(&self, done: T, err: Exit) -> Result<T, Exit> {
        if self.strict_now() {
            Err(err)
        } else {
            Ok(done)
        }
    }

    /// The parameter slot an `arguments` index is tied to, if any.
    pub(crate) fn mapped_slot(
        &self,
        o: ObjectRef,
        key: PropertyKey,
    ) -> Result<Option<(ObjectRef, u32)>, Exit> {
        let PropertyKey::Index(i) = key else {
            return Ok(None);
        };
        if let ObjectClass::Arguments {
            env: Some(env),
            mapped,
        } = &self.heap.object(o)?.class
            && mapped.get(i as usize).copied().unwrap_or(false)
        {
            return Ok(Some((*env, i)));
        }
        Ok(None)
    }

    /// Run frames from `base` until the frame at `base` returns.
    pub(crate) fn execute(&mut self, host: &mut dyn Host, base: usize) -> Result<Value, RunError> {
        loop {
            let err = match self.dispatch(host, base) {
                Ok(v) => return Ok(v),
                Err(e) => e,
            };
            let (value, stack) = match err {
                Exit::Throw { value, stack } => (value, stack),
                Exit::Pending(kind, msg) => match self.make_error_value(kind, &msg) {
                    Ok(v) => (v, None),
                    Err(e) => return Err(self.fail(e, base)),
                },
                Exit::Fatal(e) => {
                    let e = e.with_stack(self.stack_lines());
                    self.cleanup(base);
                    return Err(e);
                }
            };
            let stack = match stack {
                Some(s) => s,
                None => self.stack_lines(),
            };
            match self.unwind(value, base) {
                Ok(true) => continue,
                Ok(false) => {
                    let message = self
                        .to_display_string(value)
                        .unwrap_or_else(|_| "<unprintable value>".to_string());
                    self.cleanup(base);
                    return Err(RunError::Thrown {
                        value,
                        message,
                        stack,
                    });
                }
                Err(e) => return Err(self.fail(e, base)),
            }
        }
    }

    fn fail(&mut self, e: Exit, base: usize) -> RunError {
        let r = self.exit_to_run(e);
        self.cleanup(base);
        r
    }

    fn cleanup(&mut self, base: usize) {
        if self.frames.len() > base {
            let rb = self.frames[base].base;
            self.frames.truncate(base);
            self.regs.truncate(rb);
        }
    }

    /// Find a handler for `value`, popping frames down to `base`. True when
    /// one was found (the loop carries on there).
    fn unwind(&mut self, value: Value, base: usize) -> Result<bool, Exit> {
        loop {
            let fi = self.frames.len() - 1;
            let f = self.frames[fi];
            let pc = f.pc.saturating_sub(1);
            let handler = self.codes[f.code as usize]
                .block
                .handlers
                .iter()
                .find(|h| h.start <= pc && pc < h.end)
                .copied();
            if let Some(h) = handler {
                let mut env = f.env;
                let mut depth = f.env_depth;
                if depth < h.env_depth {
                    return Err(internal_err(format!(
                        "handler expects {} environments but the frame has {depth}",
                        h.env_depth
                    )));
                }
                while depth > h.env_depth {
                    let e = env.ok_or_else(|| internal_err("environment chain ended early"))?;
                    env = self.heap.env_parent(e)?;
                    depth -= 1;
                }
                let fr = &mut self.frames[fi];
                fr.env = env;
                fr.env_depth = depth;
                fr.pc = h.target;
                self.regs[f.base + h.catch_reg as usize] = value;
                return Ok(true);
            }
            if fi <= base {
                return Ok(false);
            }
            self.frames.pop();
            self.regs.truncate(f.base);
        }
    }

    /// Call any function value and wait for the result.
    pub(crate) fn call_value(
        &mut self,
        host: &mut dyn Host,
        f: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let class = match f.as_object() {
            Some(o) => self
                .heap
                .object(o)
                .map(|o| o.class.clone())
                .map_err(super::heap_to_run)?,
            None => ObjectClass::Ordinary,
        };
        match class {
            ObjectClass::Function { code, env } => {
                let base = self.frames.len();
                if let Err(e) =
                    self.push_frame(code, f, env, this, args, DISCARD, FrameKind::Normal)
                {
                    return Err(self.exit_to_run(e));
                }
                self.execute(host, base)
            }
            ObjectClass::Native { id } => self.call_native(host, id, this, args),
            ObjectClass::Bound(b) => {
                let mut combined = Vec::with_capacity(b.args.len() + args.len());
                combined.extend_from_slice(&b.args);
                combined.extend_from_slice(args);
                self.call_value(host, b.target, b.this, &combined)
            }
            ObjectClass::Proxy {
                target: Some(t),
                handler: Some(h),
            } => {
                if let Ok(Some(trap)) = self.proxy_trap(h, "apply") {
                    let arr = self
                        .heap
                        .new_array(Some(self.intr.array_prototype))
                        .map_err(super::heap_to_run)?;
                    for (i, v) in args.iter().enumerate() {
                        self.heap
                            .define_own(arr, PropertyKey::Index(i as u32), Property::data(*v))
                            .map_err(super::heap_to_run)?;
                    }
                    self.call_value(
                        host,
                        trap,
                        Value::object(h),
                        &[Value::object(t), this, Value::object(arr)],
                    )
                } else {
                    self.call_value(host, Value::object(t), this, args)
                }
            }
            ObjectClass::Proxy { .. } => Err(self.make_thrown(
                ErrorKind::Type,
                "Cannot perform 'apply' on a proxy that has been revoked",
            )),
            _ if false => {
                unreachable!()
            }
            _ => {
                let what = self.callee_text(f);
                Err(self.make_thrown(ErrorKind::Type, &format!("{what} is not a function")))
            }
        }
    }

    /// A call from a native function back into script.
    pub(crate) fn call_reentrant(
        &mut self,
        host: &mut dyn Host,
        f: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let max = self.heap.limits().max_native_reentry;
        if self.native_depth >= max {
            return Err(self.make_thrown(
                ErrorKind::Range,
                &format!(
                    "Too many nested builtin callbacks ({})",
                    group_digits(u64::from(max))
                ),
            ));
        }
        self.native_depth += 1;
        let r = self.call_value(host, f, this, args);
        self.native_depth -= 1;
        r
    }

    #[inline(never)]
    pub(crate) fn call_native(
        &mut self,
        host: &mut dyn Host,
        id: u32,
        this: Value,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let f = match self.natives.get(id as usize) {
            Some(n) => n.f,
            None => return Err(RunError::internal(format!("no native function {id}"))),
        };
        let mark = self.native_roots.len();
        self.native_roots.push(this);
        self.native_roots.extend_from_slice(args);
        let r = {
            let mut cx = NativeCx { vm: self, host };
            f(&mut cx, this, args)
        };
        self.native_roots.truncate(mark);
        r
    }

    pub(crate) fn callee_text(&mut self, v: Value) -> String {
        if v.is_object() {
            return "object".to_string();
        }
        if v.is_string() {
            return match self.to_display_string(v) {
                Ok(s) => format!("\"{s}\""),
                Err(_) => "string".to_string(),
            };
        }
        self.to_display_string(v)
            .unwrap_or_else(|_| "value".to_string())
    }

    /// Call `f` from inside the loop: script functions get a frame, native
    /// ones run now and their result goes to `dst`.
    #[inline(never)]
    pub(crate) fn invoke(
        &mut self,
        host: &mut dyn Host,
        f: Value,
        this: Value,
        args: &[Value],
        dst: u32,
        kind: FrameKind,
    ) -> Result<(), Exit> {
        let class = match f.as_object() {
            Some(o) => self.heap.object(o)?.class.clone(),
            None => ObjectClass::Ordinary,
        };
        match class {
            ObjectClass::Function { code, env } => {
                self.push_frame(code, f, env, this, args, dst, kind)
            }
            ObjectClass::Bound(b) => {
                let mut combined = Vec::with_capacity(b.args.len() + args.len());
                combined.extend_from_slice(&b.args);
                combined.extend_from_slice(args);
                let bound_this = if kind == FrameKind::Construct {
                    this
                } else {
                    b.this
                };
                self.invoke(host, b.target, bound_this, &combined, dst, kind)
            }
            ObjectClass::Proxy {
                target: Some(t),
                handler: Some(h),
            } => {
                if kind == FrameKind::Construct {
                    if let Some(trap) = self.proxy_trap(h, "construct")? {
                        let arr = self.heap.new_array(Some(self.intr.array_prototype))?;
                        for (i, v) in args.iter().enumerate() {
                            self.heap.define_own(
                                arr,
                                PropertyKey::Index(i as u32),
                                Property::data(*v),
                            )?;
                        }
                        return self.invoke(
                            host,
                            trap,
                            Value::object(h),
                            &[Value::object(t), Value::object(arr), f],
                            dst,
                            FrameKind::Normal,
                        );
                    }
                    return self.invoke(host, Value::object(t), this, args, dst, kind);
                }
                if let Some(trap) = self.proxy_trap(h, "apply")? {
                    let arr = self.heap.new_array(Some(self.intr.array_prototype))?;
                    for (i, v) in args.iter().enumerate() {
                        self.heap.define_own(
                            arr,
                            PropertyKey::Index(i as u32),
                            Property::data(*v),
                        )?;
                    }
                    return self.invoke(
                        host,
                        trap,
                        Value::object(h),
                        &[Value::object(t), this, Value::object(arr)],
                        dst,
                        FrameKind::Normal,
                    );
                }
                self.invoke(host, Value::object(t), this, args, dst, kind)
            }
            ObjectClass::Proxy { .. } => Err(type_err(
                "Cannot perform 'apply' on a proxy that has been revoked",
            )),
            ObjectClass::Native { id } => {
                let v = self.call_native(host, id, this, args)?;
                let v = if kind == FrameKind::Construct && !v.is_object() {
                    this
                } else {
                    v
                };
                self.store_caller(dst, v);
                Ok(())
            }
            _ => {
                let what = self.callee_text(f);
                Err(type_err(format!("{what} is not a function")))
            }
        }
    }

    pub(crate) fn store_caller(&mut self, dst: u32, v: Value) {
        if dst != DISCARD
            && let Some(f) = self.frames.last()
        {
            let i = f.base + dst as usize;
            self.regs[i] = v;
        }
    }

    // ---- the loop ---------------------------------------------------------

    #[inline(never)]
    fn tick(&mut self, host: &mut dyn Host) -> Result<(), Exit> {
        if let Some(max) = self.heap.limits().max_steps
            && self.steps > max
        {
            self.steps = max;
            return Err(Exit::Fatal(steps_abort(max)));
        }
        let interval = u64::from(self.heap.limits().interrupt_check_interval);
        if self.steps.is_multiple_of(interval)
            && let Some(reason) = host.interrupt()
        {
            return Err(Exit::Fatal(RunError::Abort {
                kind: LimitKind::Interrupt,
                reason: format!("stopped by host: {reason}"),
                limit: interval,
                observed: self.steps,
                stack: Vec::new(),
            }));
        }
        if self.heap.collect_requested() {
            let roots = self.gather_roots(host);
            let r = self.heap.safepoint(&roots);
            self.root_buf = roots;
            match r {
                Ok(Some(stats)) => {
                    for d in stats.finalized {
                        host.finalized(d);
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn dispatch(&mut self, host: &mut dyn Host, base: usize) -> Result<Value, Exit> {
        let interval = u64::from(self.heap.limits().interrupt_check_interval);
        let max_steps = self.heap.limits().max_steps.unwrap_or(u64::MAX);
        loop {
            self.steps += 1;
            self.total_steps += 1;
            if self.steps > max_steps
                || self.steps.is_multiple_of(interval)
                || self.heap.collect_requested()
            {
                self.tick(host)?;
            }
            let fi = self.frames.len() - 1;
            let (code, pc, b) = {
                let f = &self.frames[fi];
                (f.code as usize, f.pc as usize, f.base)
            };
            let op = match self.codes[code].block.ops.get(pc) {
                Some(op) => *op,
                None => return Err(internal_err(format!("pc {pc} is past the end"))),
            };
            self.frames[fi].pc = pc as u32 + 1;
            let r = |x: Reg| b + x as usize;
            match op {
                Op::Nop => {}
                Op::LoadNumber { dst, idx } => {
                    let n = self.codes[code].block.numbers[idx as usize];
                    self.regs[r(dst)] = Value::number(n);
                }
                Op::LoadInt { dst, value } => self.regs[r(dst)] = Value::number(f64::from(value)),
                Op::LoadString { dst, idx } => {
                    self.regs[r(dst)] = Value::string(self.codes[code].strings[idx as usize])
                }
                Op::LoadUndefined { dst } => self.regs[r(dst)] = Value::UNDEFINED,
                Op::LoadNull { dst } => self.regs[r(dst)] = Value::NULL,
                Op::LoadTrue { dst } => self.regs[r(dst)] = Value::TRUE,
                Op::LoadFalse { dst } => self.regs[r(dst)] = Value::FALSE,
                Op::LoadUninit { dst } => self.regs[r(dst)] = Value::UNINIT,
                Op::LoadThis { dst } => self.regs[r(dst)] = self.frames[fi].this,
                Op::Move { dst, src } => self.regs[r(dst)] = self.regs[r(src)],
                Op::Binary { op, dst, a, b: bb } => {
                    let (x, y) = (self.regs[r(a)], self.regs[r(bb)]);
                    let v = match (op, x.as_int32(), y.as_int32()) {
                        (BinOp::Add, Some(i), Some(j)) => {
                            Value::number(f64::from(i) + f64::from(j))
                        }
                        (BinOp::Sub, Some(i), Some(j)) => {
                            Value::number(f64::from(i) - f64::from(j))
                        }
                        (BinOp::Lt, Some(i), Some(j)) => Value::boolean(i < j),
                        _ => self.binary(host, op, x, y)?,
                    };
                    self.regs[r(dst)] = v;
                }
                Op::Unary { op, dst, src } => {
                    let v = self.unary(op, self.regs[r(src)])?;
                    self.regs[r(dst)] = v;
                }
                Op::Jump { offset } => self.jump(fi, pc, offset),
                Op::JumpIfTrue { cond, offset } => {
                    if self.truthy(self.regs[r(cond)])? {
                        self.jump(fi, pc, offset);
                    }
                }
                Op::JumpIfFalse { cond, offset } => {
                    if !self.truthy(self.regs[r(cond)])? {
                        self.jump(fi, pc, offset);
                    }
                }
                Op::JumpIfNullish { cond, offset } => {
                    let v = self.regs[r(cond)];
                    if v.is_nullish() || v.is_uninit() || v.is_hole() {
                        self.jump(fi, pc, offset);
                    }
                }
                Op::LoopHeader { id } => {
                    if let Some(t) = self.codes[code].loop_trips.get_mut(id as usize) {
                        *t += 1;
                    }
                }
                Op::Call {
                    dst,
                    callee,
                    this,
                    args,
                    argc,
                } => {
                    self.op_call(host, b, dst, Some(this), callee, args, argc)?;
                }
                Op::New {
                    dst,
                    callee,
                    args,
                    argc,
                } => {
                    self.op_call(host, b, dst, None, callee, args, argc)?;
                }
                Op::Return { src } => {
                    let v = self.regs[r(src)];
                    let f = self.frames.pop().ok_or_else(|| internal_err("no frame"))?;
                    self.regs.truncate(f.base);
                    let v = if f.kind == FrameKind::Construct && !v.is_object() {
                        f.this
                    } else {
                        v
                    };
                    if self.frames.len() <= base {
                        return Ok(v);
                    }
                    self.store_caller(f.dst, v);
                }
                Op::Throw { src } => {
                    return Err(Exit::Throw {
                        value: self.regs[r(src)],
                        stack: None,
                    });
                }
                _ => self.other_op(host, op, fi, code, b)?,
            }
        }
    }

    /// The less common ops, kept out of the main loop so its stack frame
    /// stays small (native re-entry nests the loop on the Rust stack).
    #[inline(never)]
    fn other_op(
        &mut self,
        host: &mut dyn Host,
        op: Op,
        fi: usize,
        code: usize,
        b: usize,
    ) -> Result<(), Exit> {
        let r = |x: Reg| b + x as usize;
        match op {
            Op::CheckTdz { src, name } => {
                if self.regs[r(src)].is_uninit() {
                    let key = self.codes[code].keys[name as usize];
                    return Err(Exit::Pending(
                        ErrorKind::Reference,
                        format!(
                            "Cannot access '{}' before initialization",
                            self.key_text(key)
                        ),
                    ));
                }
            }
            Op::GetGlobal { dst, key, ic } => {
                let v = self.get_global_op(host, code, key, ic, dst)?;
                if let Some(v) = v {
                    self.regs[r(dst)] = v;
                }
            }
            Op::SetGlobal { src, key, ic } => {
                let v = self.regs[r(src)];
                self.set_global_op(host, code, key, ic, v)?;
            }
            Op::PushEnv { size } => {
                let env = self.heap.alloc_env(self.frames[fi].env, size)?;
                let f = &mut self.frames[fi];
                if f.env_depth == u16::MAX {
                    return Err(internal_err("more than 65,535 environments in one frame"));
                }
                f.env = Some(env);
                f.env_depth += 1;
            }
            Op::PopEnv => {
                let f = self.frames[fi];
                if f.env_depth == 0 {
                    return Err(internal_err(
                        "PopEnv with no environment pushed in this function",
                    ));
                }
                let e = f.env.ok_or_else(|| internal_err("no environment to pop"))?;
                let parent = self.heap.env_parent(e)?;
                let f = &mut self.frames[fi];
                f.env = parent;
                f.env_depth -= 1;
            }
            Op::GetEnv { dst, depth, slot } => {
                let e = self.env_at(fi, depth)?;
                self.regs[r(dst)] = self.heap.env_get(e, slot)?;
            }
            Op::SetEnv { src, depth, slot } => {
                let e = self.env_at(fi, depth)?;
                self.heap.env_set(e, slot, self.regs[r(src)])?;
            }
            Op::NewObject { dst } => {
                let o = self.heap.new_object(Some(self.intr.object_prototype))?;
                self.regs[r(dst)] = Value::object(o);
            }
            Op::NewArray { dst } => {
                let o = self.heap.new_array(Some(self.intr.array_prototype))?;
                self.regs[r(dst)] = Value::object(o);
            }
            Op::GetProp { dst, obj, key, ic } => {
                let recv = self.regs[r(obj)];
                if let Some(v) = self.ic_get(code, ic, recv)? {
                    self.regs[r(dst)] = v;
                } else {
                    let k = self.codes[code].keys[key as usize];
                    self.get_op(host, recv, k, Some((code, ic)), u32::from(dst))?;
                }
            }
            Op::SetProp { obj, src, key, ic } => {
                let recv = self.regs[r(obj)];
                let v = self.regs[r(src)];
                if !self.ic_set(code, ic, recv, v)? {
                    let k = self.codes[code].keys[key as usize];
                    self.set_op(host, recv, k, v, Some((code, ic)))?;
                }
            }
            Op::GetElem { dst, obj, key } => {
                let recv = self.regs[r(obj)];
                let k = self.regs[r(key)];
                let k = self.conv_key(k)?;
                self.get_op(host, recv, k, None, u32::from(dst))?;
            }
            Op::SetElem { obj, key, src } => {
                let recv = self.regs[r(obj)];
                let k = self.regs[r(key)];
                let v = self.regs[r(src)];
                let k = self.conv_key(k)?;
                self.set_op(host, recv, k, v, None)?;
            }
            Op::DeleteElem { dst, obj, key } => {
                let recv = self.regs[r(obj)];
                let k = self.regs[r(key)];
                let k = self.conv_key(k)?;
                let v = self.delete_op(host, recv, k)?;
                self.regs[r(dst)] = Value::boolean(v);
            }
            Op::Closure { dst, func } => {
                let child = self.codes[code].children[func as usize];
                let env = self.frames[fi].env;
                let o = self.heap.alloc_object(
                    Some(self.intr.function_prototype),
                    ObjectClass::Function { code: child, env },
                )?;
                self.regs[r(dst)] = Value::object(o);
            }
            _ => return self.extra_op(host, op, fi, code, b),
        }
        Ok(())
    }

    /// `Call` (with `this`) and `New` (without).
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn op_call(
        &mut self,
        host: &mut dyn Host,
        b: usize,
        dst: Reg,
        this: Option<Reg>,
        callee: Reg,
        args: Reg,
        argc: u16,
    ) -> Result<(), Exit> {
        let f = self.regs[b + callee as usize];
        let start = b + args as usize;
        let argv: Vec<Value> = self.regs[start..start + argc as usize].to_vec();
        match this {
            Some(t) => {
                let t = self.regs[b + t as usize];
                self.invoke(host, f, t, &argv, u32::from(dst), FrameKind::Normal)
            }
            None => self.construct(host, f, &argv, u32::from(dst)),
        }
    }

    fn jump(&mut self, fi: usize, pc: usize, offset: i32) {
        let target = (pc as i64 + 1 + i64::from(offset)) as u32;
        self.frames[fi].pc = target;
    }

    fn env_at(&self, fi: usize, depth: u16) -> Result<ObjectRef, Exit> {
        let mut e = self.frames[fi].env;
        for _ in 0..depth {
            e = match e {
                Some(x) => self.heap.env_parent(x)?,
                None => None,
            };
        }
        e.ok_or_else(|| internal_err(format!("environment depth {depth} is past the chain")))
    }

    // ---- operators --------------------------------------------------------

    #[inline(never)]
    fn binary(
        &mut self,
        host: &mut dyn Host,
        op: BinOp,
        a: Value,
        b: Value,
    ) -> Result<Value, Exit> {
        if let (Some(x), Some(y)) = (a.as_bigint(), b.as_bigint()) {
            return Ok(match op {
                BinOp::Add => Value::bigint(x.wrapping_add(y)),
                BinOp::Sub => Value::bigint(x.wrapping_sub(y)),
                BinOp::Mul => Value::bigint(x.wrapping_mul(y)),
                BinOp::Div => {
                    if y == 0 {
                        return Err(range_err("Division by zero"));
                    }
                    Value::bigint(x.wrapping_div(y))
                }
                BinOp::Mod => {
                    if y == 0 {
                        return Err(range_err("Division by zero"));
                    }
                    Value::bigint(x.wrapping_rem(y))
                }
                BinOp::Exp => {
                    if y < 0 {
                        return Err(range_err("Exponent must be positive"));
                    }
                    Value::bigint(x.wrapping_pow(y as u32))
                }
                BinOp::BitAnd => Value::bigint(x & y),
                BinOp::BitOr => Value::bigint(x | y),
                BinOp::BitXor => Value::bigint(x ^ y),
                BinOp::Shl => Value::bigint(x.wrapping_shl(y as u32)),
                BinOp::Shr => Value::bigint(x.wrapping_shr(y as u32)),
                BinOp::UShr => return Err(type_err("BigInts have no unsigned right shift")),
                BinOp::StrictEq => Value::boolean(x == y),
                BinOp::StrictNe => Value::boolean(x != y),
                BinOp::LooseEq => Value::boolean(x == y),
                BinOp::LooseNe => Value::boolean(x != y),
                BinOp::Lt => Value::boolean(x < y),
                BinOp::Gt => Value::boolean(x > y),
                BinOp::Le => Value::boolean(x <= y),
                BinOp::Ge => Value::boolean(x >= y),
                BinOp::InstanceOf => Value::boolean(self.instance_of(a, b)?),
                BinOp::In => Value::boolean(self.has_property(host, a, b)?),
            });
        }
        Ok(match op {
            BinOp::Add => {
                if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
                    return Ok(Value::number(x + y));
                }
                let pa = self.conv_primitive(a)?;
                let pb = self.conv_primitive(b)?;
                if pa.is_string() || pb.is_string() {
                    let sa = self.conv_string(pa)?;
                    let sb = self.conv_string(pb)?;
                    Value::string(self.heap.concat(sa, sb)?)
                } else {
                    Value::number(self.conv_number(pa)? + self.conv_number(pb)?)
                }
            }
            BinOp::Sub => Value::number(self.conv_number(a)? - self.conv_number(b)?),
            BinOp::Mul => Value::number(self.conv_number(a)? * self.conv_number(b)?),
            BinOp::Div => Value::number(self.conv_number(a)? / self.conv_number(b)?),
            BinOp::Mod => Value::number(self.conv_number(a)? % self.conv_number(b)?),
            BinOp::Exp => {
                let x = self.conv_number(a)?;
                let y = self.conv_number(b)?;
                Value::number(js_pow(x, y))
            }
            BinOp::BitAnd => Value::number(f64::from(self.conv_int32(a)? & self.conv_int32(b)?)),
            BinOp::BitOr => Value::number(f64::from(self.conv_int32(a)? | self.conv_int32(b)?)),
            BinOp::BitXor => Value::number(f64::from(self.conv_int32(a)? ^ self.conv_int32(b)?)),
            BinOp::Shl => {
                let x = self.conv_int32(a)?;
                let y = to_uint32(self.conv_number(b)?) & 31;
                Value::number(f64::from(x.wrapping_shl(y)))
            }
            BinOp::Shr => {
                let x = self.conv_int32(a)?;
                let y = to_uint32(self.conv_number(b)?) & 31;
                Value::number(f64::from(x >> y))
            }
            BinOp::UShr => {
                let x = to_uint32(self.conv_number(a)?);
                let y = to_uint32(self.conv_number(b)?) & 31;
                Value::number(f64::from(x >> y))
            }
            BinOp::StrictEq => Value::boolean(self.strict_eq(a, b)?),
            BinOp::StrictNe => Value::boolean(!self.strict_eq(a, b)?),
            BinOp::LooseEq => Value::boolean(self.loose_eq(a, b)?),
            BinOp::LooseNe => Value::boolean(!self.loose_eq(a, b)?),
            BinOp::Lt => Value::boolean(self.less_than(a, b)? == Some(true)),
            BinOp::Gt => Value::boolean(self.less_than(b, a)? == Some(true)),
            BinOp::Le => Value::boolean(self.less_than(b, a)? == Some(false)),
            BinOp::Ge => Value::boolean(self.less_than(a, b)? == Some(false)),
            BinOp::InstanceOf => Value::boolean(self.instance_of(a, b)?),
            BinOp::In => Value::boolean(self.has_property(host, a, b)?),
        })
    }

    #[inline(never)]
    fn unary(&mut self, op: UnOp, v: Value) -> Result<Value, Exit> {
        if let Some(x) = v.as_bigint() {
            return Ok(match op {
                UnOp::Neg => Value::bigint(x.wrapping_neg()),
                UnOp::Plus => return Err(type_err("Cannot convert a BigInt value to a number")),
                UnOp::Not => Value::boolean(x == 0),
                UnOp::BitNot => Value::bigint(!x),
                UnOp::TypeOf => Value::string(self.type_of(v)?),
                UnOp::Inc => Value::bigint(x.wrapping_add(1)),
                UnOp::Dec => Value::bigint(x.wrapping_sub(1)),
            });
        }
        Ok(match op {
            UnOp::Neg => Value::number(-self.conv_number(v)?),
            UnOp::Plus => Value::number(self.conv_number(v)?),
            UnOp::Not => Value::boolean(!self.truthy(v)?),
            UnOp::BitNot => Value::number(f64::from(!self.conv_int32(v)?)),
            UnOp::TypeOf => Value::string(self.type_of(v)?),
            UnOp::Inc => Value::number(self.conv_number(v)? + 1.0),
            UnOp::Dec => Value::number(self.conv_number(v)? - 1.0),
        })
    }

    fn instance_of(&mut self, a: Value, b: Value) -> Result<bool, Exit> {
        let callable = match b.as_object() {
            Some(o) => self.heap.object(o)?.is_callable(),
            None => false,
        };
        let Some(bo) = b.as_object().filter(|_| callable) else {
            return Err(type_err("Right-hand side of 'instanceof' is not callable"));
        };
        let proto = self.function_prototype_property(bo)?;
        let Some(proto) = proto.as_object() else {
            let t = self.display_units(proto)?;
            return Err(type_err(format!(
                "Function has non-object prototype '{}' in instanceof check",
                String::from_utf16_lossy(&t)
            )));
        };
        let Some(mut cur) = a.as_object() else {
            return Ok(false);
        };
        let mut hops = 0usize;
        loop {
            if self.heap.object(cur)?.class.is_exotic() {
                return Err(type_err("instanceof through a proxy is not supported yet"));
            }
            match self.heap.get_prototype(cur)? {
                Some(p) if p == proto => return Ok(true),
                Some(p) => cur = p,
                None => return Ok(false),
            }
            hops += 1;
            if hops > self.heap.live_objects() {
                return Err(internal_err("prototype chain has a cycle"));
            }
        }
    }

    fn has_property(&mut self, host: &mut dyn Host, key: Value, obj: Value) -> Result<bool, Exit> {
        let k = self.conv_key(key)?;
        if let Some(o) = obj.as_object()
            && let Some((target, handler)) = self.proxy_parts(o, "has")?
        {
            if let Some(trap) = self.proxy_trap(handler, "has")? {
                let kv = self.prop_key_value(k)?;
                let res = self.call_reentrant(
                    host,
                    trap,
                    Value::object(handler),
                    &[Value::object(target), kv],
                )?;
                return Ok(self.truthy(res)?);
            }
            return self.has_property(host, key, Value::object(target));
        }
        let k = self.conv_key(key)?;
        let Some(o) = obj.as_object() else {
            let t = self.display_units(obj)?;
            return Err(type_err(format!(
                "Cannot use 'in' operator to search for '{}' in {}",
                self.key_text(k),
                String::from_utf16_lossy(&t)
            )));
        };
        if k == self.intr.key_length && self.heap.object(o)?.is_array() {
            return Ok(true);
        }
        match self.heap.lookup(o, k)? {
            Lookup::Found { .. } => Ok(true),
            Lookup::NotFound => Ok(false),
            Lookup::Exotic(_) => Err(proxy_err()),
        }
    }

    /// `f.prototype` for `new` and `instanceof`, made on first use for
    /// script functions.
    pub(crate) fn function_prototype_property(&mut self, f: ObjectRef) -> Result<Value, Exit> {
        let key = self.intr.key_prototype;
        match self.heap.get_own(f, key)? {
            Some(Property {
                slot: Slot::Data(v),
                ..
            }) => Ok(v),
            Some(_) => Ok(Value::UNDEFINED),
            None => {
                if matches!(self.heap.object(f)?.class, ObjectClass::Function { .. }) {
                    let p = self.heap.new_object(Some(self.intr.object_prototype))?;
                    self.heap.define_own(
                        f,
                        key,
                        Property {
                            attrs: Attrs(Attrs::WRITABLE),
                            slot: Slot::Data(Value::object(p)),
                        },
                    )?;
                    self.heap.define_own(
                        p,
                        self.intr.key_constructor,
                        Property {
                            attrs: Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
                            slot: Slot::Data(Value::object(f)),
                        },
                    )?;
                    Ok(Value::object(p))
                } else {
                    Ok(Value::UNDEFINED)
                }
            }
        }
    }

    #[inline(never)]
    pub(crate) fn construct(
        &mut self,
        host: &mut dyn Host,
        f: Value,
        args: &[Value],
        dst: u32,
    ) -> Result<(), Exit> {
        let callable = match f.as_object() {
            Some(o) => self.is_callable_obj(o)?,
            None => false,
        };
        let Some(fo) = f.as_object().filter(|_| callable) else {
            let what = self.callee_text(f);
            return Err(type_err(format!("{what} is not a constructor")));
        };
        let mut target_obj = fo;
        while let ObjectClass::Bound(b) = &self.heap.object(target_obj)?.class {
            if let Some(to) = b.target.as_object() {
                target_obj = to;
            } else {
                break;
            }
        }
        let proto = self.function_prototype_property(target_obj)?;
        let proto = proto.as_object().unwrap_or(self.intr.object_prototype);
        let instance = self.heap.new_object(Some(proto))?;
        self.invoke(
            host,
            f,
            Value::object(instance),
            args,
            dst,
            FrameKind::Construct,
        )
    }

    // ---- property access --------------------------------------------------

    /// Inline cache read. `None` is a miss.
    #[inline]
    fn ic_get(&mut self, code: usize, site: u32, recv: Value) -> Result<Option<Value>, Exit> {
        if !self.ic_enabled {
            return Ok(None);
        }
        let epoch = self.heap.prototype_epoch();
        let lc = &self.codes[code];
        let s = &lc.ic[site as usize];
        if s.entries.is_empty() {
            return Ok(None);
        }
        let mut found = None;
        if let Some(o) = recv.as_object() {
            let obj = self.heap.object(o)?;
            if let Some(shape) = obj.shape() {
                for e in &s.entries {
                    match *e {
                        IcEntry::Own { shape: s2, slot } if s2 == shape => {
                            found = self.heap.read_slot(o, slot)?.data();
                            break;
                        }
                        IcEntry::Proto {
                            shape: s2,
                            holder,
                            holder_shape,
                            slot,
                            epoch: e2,
                        } if s2 == shape && e2 == epoch => {
                            if self.heap.is_live(holder)
                                && self.heap.object(holder)?.shape() == Some(holder_shape)
                            {
                                found = self.heap.read_slot(holder, slot)?.data();
                            }
                            break;
                        }
                        IcEntry::ArrayLength => {
                            if let ObjectClass::Array { length } = obj.class {
                                found = Some(Value::number(f64::from(length)));
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else if let Some(st) = recv.as_string()
            && s.entries.contains(&IcEntry::StringLength)
        {
            found = Some(Value::number(self.heap.string_len(st)? as f64));
        }
        let s = &mut self.codes[code].ic[site as usize];
        if found.is_some() {
            s.hits += 1;
        }
        Ok(found)
    }

    /// Inline cache write. False is a miss.
    #[inline]
    fn ic_set(&mut self, code: usize, site: u32, recv: Value, v: Value) -> Result<bool, Exit> {
        if !self.ic_enabled {
            return Ok(false);
        }
        let Some(o) = recv.as_object() else {
            return Ok(false);
        };
        let epoch = self.heap.prototype_epoch();
        let s = &self.codes[code].ic[site as usize];
        if s.entries.is_empty() {
            return Ok(false);
        }
        let Some(shape) = self.heap.object(o)?.shape() else {
            return Ok(false);
        };
        let mut action = None;
        for e in &s.entries {
            match *e {
                IcEntry::Own { shape: s2, slot } if s2 == shape => {
                    action = Some(*e);
                    let _ = slot;
                    break;
                }
                IcEntry::AddProp {
                    from, epoch: e2, ..
                } if from == shape && e2 == epoch => {
                    action = Some(*e);
                    break;
                }
                _ => {}
            }
        }
        let done = match action {
            Some(IcEntry::Own { slot, .. }) => {
                self.heap.write_slot(o, slot, v)?;
                true
            }
            Some(IcEntry::AddProp { from, to, .. }) => {
                self.heap.apply_add_transition(o, from, to, v)?
            }
            _ => false,
        };
        if done {
            self.codes[code].ic[site as usize].hits += 1;
        }
        Ok(done)
    }

    fn record(&mut self, site: Option<(usize, u32)>, entry: Option<IcEntry>) {
        if let Some((code, ic)) = site {
            let s = &mut self.codes[code].ic[ic as usize];
            if self.ic_enabled {
                s.misses += 1;
                if let Some(e) = entry {
                    s.record(e);
                }
            }
        }
    }

    /// Generic read. The result (or a getter's frame) goes to `dst`.
    #[inline(never)]
    pub(crate) fn get_op(
        &mut self,
        host: &mut dyn Host,
        recv: Value,
        key: PropertyKey,
        site: Option<(usize, u32)>,
        dst: u32,
    ) -> Result<(), Exit> {
        if let Some(o) = recv.as_object()
            && let Some((target, handler)) = self.proxy_parts(o, "get")?
        {
            if let Some(trap) = self.proxy_trap(handler, "get")? {
                let kv = self.prop_key_value(key)?;
                return self.invoke(
                    host,
                    trap,
                    Value::object(handler),
                    &[Value::object(target), kv, recv],
                    dst,
                    FrameKind::Normal,
                );
            }
            return self.get_op(host, Value::object(target), key, None, dst);
        }
        let (got, entry) = self.lookup_get(recv, key)?;
        self.record(site, entry);
        match got {
            Got::Value(v) => {
                self.store_caller(dst, v);
                Ok(())
            }
            Got::Getter(g) => self.invoke(host, g, recv, &[], dst, FrameKind::Normal),
        }
    }

    pub(crate) fn lookup_get(
        &mut self,
        recv: Value,
        key: PropertyKey,
    ) -> Result<(Got, Option<IcEntry>), Exit> {
        let start = if let Some(o) = recv.as_object() {
            let obj = self.heap.object(o)?;
            if key == self.intr.key_length
                && let ObjectClass::Array { length } = obj.class
            {
                let entry = obj.shape().map(|_| IcEntry::ArrayLength);
                return Ok((Got::Value(Value::number(f64::from(length))), entry));
            }
            if let Some((env, slot)) = self.mapped_slot(o, key)? {
                return Ok((Got::Value(self.heap.env_get(env, slot)?), None));
            }
            o
        } else if let Some(s) = recv.as_string() {
            if key == self.intr.key_length {
                let n = self.heap.string_len(s)?;
                return Ok((
                    Got::Value(Value::number(n as f64)),
                    Some(IcEntry::StringLength),
                ));
            }
            if let PropertyKey::Index(i) = key
                && let Some(u) = self.heap.char_code_at(s, i as usize)?
            {
                let c = self.heap.alloc_string_units(vec![u])?;
                return Ok((Got::Value(Value::string(c)), None));
            }
            self.intr.string_prototype
        } else if recv.is_number() || recv.is_bigint() {
            self.intr.number_prototype
        } else if recv.is_bool() {
            self.intr.boolean_prototype
        } else if recv.is_symbol() {
            self.intr.symbol_prototype
        } else {
            let what = if recv.is_null() { "null" } else { "undefined" };
            return Err(type_err(format!(
                "Cannot read properties of {what} (reading '{}')",
                self.key_text(key)
            )));
        };
        match self.heap.lookup(start, key)? {
            Lookup::Found { holder, prop } => match prop.slot {
                Slot::Data(v) => {
                    let entry = self.entry_for_get(recv, holder, key)?;
                    Ok((Got::Value(v), entry))
                }
                Slot::Accessor { get, .. } => {
                    if get.is_undefined() {
                        Ok((Got::Value(Value::UNDEFINED), None))
                    } else {
                        Ok((Got::Getter(get), None))
                    }
                }
            },
            Lookup::NotFound => {
                if let Some(o) = recv.as_object() {
                    if key == self.intr.key_name {
                        if let Some(n) = self.function_name(o) {
                            let s = self.heap.alloc_string(&n)?;
                            return Ok((Got::Value(Value::string(s)), None));
                        }
                    } else if key == self.intr.key_length
                        && let Some(n) = self.function_length(o)
                    {
                        return Ok((Got::Value(Value::number(f64::from(n))), None));
                    }
                }
                if key == self.intr.key_prototype
                    && let Some(o) = recv.as_object()
                    && matches!(self.heap.object(o)?.class, ObjectClass::Function { .. })
                {
                    let p = self.function_prototype_property(o)?;
                    return Ok((Got::Value(p), None));
                }
                Ok((Got::Value(Value::UNDEFINED), None))
            }
            Lookup::Exotic(_) => Err(proxy_err()),
        }
    }

    fn entry_for_get(
        &self,
        recv: Value,
        holder: ObjectRef,
        key: PropertyKey,
    ) -> Result<Option<IcEntry>, Exit> {
        if matches!(key, PropertyKey::Index(_)) {
            return Ok(None);
        }
        let Some(o) = recv.as_object() else {
            return Ok(None);
        };
        let Some(shape) = self.heap.object(o)?.shape() else {
            return Ok(None);
        };
        let Some(holder_shape) = self.heap.object(holder)?.shape() else {
            return Ok(None);
        };
        let Some((slot, _)) = self.heap.own_slot(holder, key)? else {
            return Ok(None);
        };
        if holder == o {
            Ok(Some(IcEntry::Own { shape, slot }))
        } else {
            Ok(Some(IcEntry::Proto {
                shape,
                holder,
                holder_shape,
                slot,
                epoch: self.heap.prototype_epoch(),
            }))
        }
    }

    /// Generic write.
    #[inline(never)]
    pub(crate) fn set_op(
        &mut self,
        host: &mut dyn Host,
        recv: Value,
        key: PropertyKey,
        v: Value,
        site: Option<(usize, u32)>,
    ) -> Result<(), Exit> {
        if let Some(o) = recv.as_object()
            && let Some((target, handler)) = self.proxy_parts(o, "set")?
        {
            if let Some(trap) = self.proxy_trap(handler, "set")? {
                let kv = self.prop_key_value(key)?;
                let ok = self.call_reentrant(
                    host,
                    trap,
                    Value::object(handler),
                    &[Value::object(target), kv, v, recv],
                )?;
                if !self.truthy(ok)? {
                    return self.quiet_fail(
                        (),
                        type_err(format!(
                            "'set' on proxy: trap returned falsish for property '{}'",
                            self.key_text(key)
                        )),
                    );
                }
                return Ok(());
            }
            return self.set_op(host, Value::object(target), key, v, None);
        }
        let (put, entry) = self.do_set(recv, key, v)?;
        self.record(site, entry);
        match put {
            Put::Done => Ok(()),
            Put::Setter(f) => self.invoke(host, f, recv, &[v], DISCARD, FrameKind::Normal),
        }
    }

    fn do_set(
        &mut self,
        recv: Value,
        key: PropertyKey,
        v: Value,
    ) -> Result<(Put, Option<IcEntry>), Exit> {
        let Some(o) = recv.as_object() else {
            if recv.is_nullish() || recv.is_uninit() || recv.is_hole() {
                let what = if recv.is_null() { "null" } else { "undefined" };
                return Err(type_err(format!(
                    "Cannot set properties of {what} (setting '{}')",
                    self.key_text(key)
                )));
            }
            let proto = if recv.is_string() {
                self.intr.string_prototype
            } else if recv.is_number() || recv.is_bigint() {
                self.intr.number_prototype
            } else if recv.is_bool() {
                self.intr.boolean_prototype
            } else {
                self.intr.symbol_prototype
            };
            if let Lookup::Found {
                prop:
                    Property {
                        slot: Slot::Accessor { set, .. },
                        ..
                    },
                ..
            } = self.heap.lookup(proto, key)?
                && !set.is_undefined()
            {
                return Ok((Put::Setter(set), None));
            }
            let t = self.display_units(recv)?;
            let ty = recv.typeof_primitive().unwrap_or("value");
            let e = type_err(format!(
                "Cannot create property '{}' on {ty} '{}'",
                self.key_text(key),
                String::from_utf16_lossy(&t)
            ));
            return self.quiet_fail((Put::Done, None), e);
        };
        if let Some((env, slot)) = self.mapped_slot(o, key)? {
            self.heap.env_set(env, slot, v)?;
        }
        let is_array = self.heap.object(o)?.is_array();
        if is_array && key == self.intr.key_length {
            let n = self.conv_number(v)?;
            let len = to_uint32(n);
            if f64::from(len) != n {
                return Err(range_err(format!(
                    "Invalid array length: {}",
                    number_to_string(n)
                )));
            }
            if !self.heap.set_array_length(o, len)? {
                let e = type_err(
                    "Cannot delete a non-configurable array element while setting 'length'",
                );
                return self.quiet_fail((Put::Done, None), e);
            }
            return Ok((Put::Done, None));
        }
        let name = || key;
        match self.heap.lookup(o, key)? {
            Lookup::Found { holder, prop } => match prop.slot {
                Slot::Data(_) if !prop.attrs.writable() => {
                    let e = type_err(format!(
                        "Cannot assign to read only property '{}' of object",
                        self.key_text(name())
                    ));
                    self.quiet_fail((Put::Done, None), e)
                }
                Slot::Data(_) if holder == o => {
                    let slot = if matches!(key, PropertyKey::Index(_)) {
                        None
                    } else {
                        self.heap.own_slot(o, key)?
                    };
                    let shape = self.heap.object(o)?.shape();
                    if !self.heap.set_own_value(o, key, v)? {
                        let e = type_err(format!(
                            "Cannot assign to read only property '{}' of object",
                            self.key_text(key)
                        ));
                        return self.quiet_fail((Put::Done, None), e);
                    }
                    let entry = match (shape, slot) {
                        (Some(shape), Some((slot, _))) => Some(IcEntry::Own { shape, slot }),
                        _ => None,
                    };
                    Ok((Put::Done, entry))
                }
                Slot::Data(_) => self.add_own(o, key, v),
                Slot::Accessor { set, .. } => {
                    if set.is_undefined() {
                        let e = type_err(format!(
                            "Cannot set property {} of object which has only a getter",
                            self.key_text(key)
                        ));
                        self.quiet_fail((Put::Done, None), e)
                    } else {
                        Ok((Put::Setter(set), None))
                    }
                }
            },
            Lookup::NotFound => self.add_own(o, key, v),
            Lookup::Exotic(_) => Err(proxy_err()),
        }
    }

    fn add_own(
        &mut self,
        o: ObjectRef,
        key: PropertyKey,
        v: Value,
    ) -> Result<(Put, Option<IcEntry>), Exit> {
        if !self.heap.object(o)?.extensible {
            let e = type_err(format!(
                "Cannot add property {}, object is not extensible",
                self.key_text(key)
            ));
            return self.quiet_fail((Put::Done, None), e);
        }
        let before = self.heap.object(o)?.shape();
        if let PropertyKey::Index(i) = key
            && i == u32::MAX
        {
            return Err(internal_err("index 4294967295 is not an array index"));
        }
        if !self.heap.define_own(o, key, Property::data(v))? {
            let e = type_err(format!(
                "Cannot add property {}, object is not extensible",
                self.key_text(key)
            ));
            return self.quiet_fail((Put::Done, None), e);
        }
        let after = self.heap.object(o)?.shape();
        let entry = match (before, after, key) {
            (_, _, PropertyKey::Index(_)) => None,
            (Some(from), Some(to), _) if self.heap.shape(to)?.parent == Some(from) => {
                Some(IcEntry::AddProp {
                    from,
                    to,
                    epoch: self.heap.prototype_epoch(),
                })
            }
            _ => None,
        };
        Ok((Put::Done, entry))
    }

    pub(crate) fn delete_op(
        &mut self,
        host: &mut dyn Host,
        recv: Value,
        key: PropertyKey,
    ) -> Result<bool, Exit> {
        if let Some(o) = recv.as_object()
            && let Some((target, handler)) = self.proxy_parts(o, "deleteProperty")?
        {
            if let Some(trap) = self.proxy_trap(handler, "deleteProperty")? {
                let kv = self.prop_key_value(key)?;
                let ok = self.call_reentrant(
                    host,
                    trap,
                    Value::object(handler),
                    &[Value::object(target), kv],
                )?;
                if !self.truthy(ok)? {
                    return self.quiet_fail(
                        false,
                        type_err(format!(
                            "'deleteProperty' on proxy: trap returned falsish for property '{}'",
                            self.key_text(key)
                        )),
                    );
                }
                return Ok(true);
            }
            return self.delete_op(host, Value::object(target), key);
        }
        let Some(o) = recv.as_object() else {
            if recv.is_nullish() {
                return Err(type_err("Cannot convert undefined or null to object"));
            }
            return Ok(true);
        };
        let obj = self.heap.object(o)?;
        if obj.class.is_exotic() {
            return Err(proxy_err());
        }
        if obj.is_array() && key == self.intr.key_length {
            let e = type_err("Cannot delete property 'length' of [object Array]");
            return self.quiet_fail(false, e);
        }
        if self.heap.delete_own(o, key)? {
            if let PropertyKey::Index(i) = key
                && let ObjectClass::Arguments { mapped, .. } = self.heap.class_mut(o)?
                && let Some(m) = mapped.get_mut(i as usize)
            {
                *m = false;
            }
            Ok(true)
        } else {
            let e = type_err(format!(
                "Cannot delete property '{}' of object",
                self.key_text(key)
            ));
            self.quiet_fail(false, e)
        }
    }

    #[inline(never)]
    fn get_global_op(
        &mut self,
        host: &mut dyn Host,
        code: usize,
        key: u32,
        ic: u32,
        dst: Reg,
    ) -> Result<Option<Value>, Exit> {
        let k = self.codes[code].keys[key as usize];
        if let Some(v) = self.lexical_get(k)? {
            return Ok(Some(v));
        }
        let g = Value::object(self.intr.global);
        if let Some(v) = self.ic_get(code, ic, g)? {
            return Ok(Some(v));
        }
        if let Lookup::NotFound = self.heap.lookup(self.intr.global, k)? {
            self.record(Some((code, ic)), None);
            return Err(Exit::Pending(
                ErrorKind::Reference,
                format!("{} is not defined", self.key_text(k)),
            ));
        }
        self.get_op(host, g, k, Some((code, ic)), u32::from(dst))?;
        Ok(None)
    }

    #[inline(never)]
    fn set_global_op(
        &mut self,
        host: &mut dyn Host,
        code: usize,
        key: u32,
        ic: u32,
        v: Value,
    ) -> Result<(), Exit> {
        let k = self.codes[code].keys[key as usize];
        if self.lexical_set(k, v)? {
            return Ok(());
        }
        let g = Value::object(self.intr.global);
        if self.ic_set(code, ic, g, v)? {
            return Ok(());
        }
        if let Lookup::NotFound = self.heap.lookup(self.intr.global, k)? {
            self.record(Some((code, ic)), None);
            if !self.codes[code].block.strict {
                return self.set_op(host, g, k, v, None);
            }
            return Err(Exit::Pending(
                ErrorKind::Reference,
                format!("{} is not defined", self.key_text(k)),
            ));
        }
        self.set_op(host, g, k, v, Some((code, ic)))
    }

    pub(crate) fn is_callable_obj(&self, mut o: ObjectRef) -> Result<bool, Exit> {
        let mut hops = 0usize;
        loop {
            match self.heap.object(o)?.class {
                ObjectClass::Proxy {
                    target: Some(t), ..
                } => {
                    o = t;
                    hops += 1;
                    if hops > 64 {
                        return Ok(false);
                    }
                }
                ObjectClass::Proxy { target: None, .. } => return Ok(false),
                _ => return Ok(self.heap.object(o)?.is_callable()),
            }
        }
    }

    pub(crate) fn proxy_parts(
        &self,
        o: ObjectRef,
        trap_name: &str,
    ) -> Result<Option<(ObjectRef, ObjectRef)>, Exit> {
        match self.heap.object(o)?.class {
            ObjectClass::Proxy {
                target: Some(t),
                handler: Some(h),
            } => Ok(Some((t, h))),
            ObjectClass::Proxy { .. } => Err(type_err(format!(
                "Cannot perform '{trap_name}' on a proxy that has been revoked"
            ))),
            _ => Ok(None),
        }
    }

    pub(crate) fn proxy_trap(
        &mut self,
        handler: ObjectRef,
        name: &str,
    ) -> Result<Option<Value>, Exit> {
        let k = self.heap.key_from_str(name)?;
        match self.heap.lookup(handler, k)? {
            Lookup::Found {
                prop:
                    Property {
                        slot: Slot::Data(f),
                        ..
                    },
                ..
            } if !f.is_nullish() => Ok(Some(f)),
            _ => Ok(None),
        }
    }

    pub(crate) fn prop_key_value(&mut self, key: PropertyKey) -> Result<Value, Exit> {
        match key {
            PropertyKey::Symbol(s) => Ok(Value::symbol(s)),
            _ => {
                let text = self.key_text(key);
                Ok(Value::string(self.heap.alloc_string(&text)?))
            }
        }
    }
}

#[allow(dead_code)]
fn proxy_err() -> Exit {
    type_err("proxy objects are not supported yet")
}

/// `x ** y`, with the language's special cases where it differs from
/// `powf`.
pub(crate) fn js_pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return 1.0;
    }
    if (x == 1.0 || x == -1.0) && y.is_infinite() {
        return f64::NAN;
    }
    x.powf(y)
}

#[cfg(test)]
mod tests {
    use super::js_pow;

    #[test]
    fn pow_special_cases() {
        assert!(js_pow(1.0, f64::NAN).is_nan());
        assert!(js_pow(-1.0, f64::INFINITY).is_nan());
        assert!(js_pow(1.0, f64::NEG_INFINITY).is_nan());
        assert_eq!(js_pow(f64::NAN, 0.0), 1.0);
        assert_eq!(js_pow(2.0, 10.0), 1024.0);
        assert!(js_pow(-8.0, 1.0 / 3.0).is_nan());
    }
}
