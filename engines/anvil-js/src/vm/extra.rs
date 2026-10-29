// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Ops the compiler emits for whole-language features: global
//! declarations, object and array literals, spread, for-in and for-of,
//! `arguments` mapping and direct `eval`.
//!
//! They sit outside the main loop like the other rare ops.

use super::frame::FrameKind;
use super::interp::Got;
use super::{ErrorKind, Exit, Host, Vm, internal_err, range_err, type_err};
use crate::anvil_js::bytecode::{Op, Reg};
use crate::anvil_js::gc::Lookup;
use crate::anvil_js::value::atom::PropertyKey;
use crate::anvil_js::value::object::{ObjectClass, Property, Slot};
use crate::anvil_js::value::shape::Attrs;
use crate::anvil_js::value::{ObjectRef, Value};
use std::collections::HashSet;

/// Most elements one spread or for-of preparation copies. Sparse arrays
/// can claim a length of four billion; copying that would run far past the
/// step budget without counting a step.
pub(crate) const MAX_SPREAD: u32 = 1 << 24;

impl Vm {
    /// Run one of the ops in this file. Anything else is an internal error.
    #[inline(never)]
    pub(crate) fn extra_op(
        &mut self,
        host: &mut dyn Host,
        op: Op,
        fi: usize,
        code: usize,
        b: usize,
    ) -> Result<(), Exit> {
        let r = |x: Reg| b + x as usize;
        match op {
            Op::LoadGlobalObject { dst } => self.regs[r(dst)] = Value::object(self.intr.global),
            Op::LoadCallee { dst } => self.regs[r(dst)] = self.frames[fi].callee,
            Op::TypeOfGlobal { dst, key } => {
                let k = self.codes[code].keys[key as usize];
                let v = match self.lexical_get(k)? {
                    Some(v) => Some(v),
                    None => match self.heap.lookup(self.intr.global, k)? {
                        Lookup::NotFound => None,
                        Lookup::Found { prop, .. } => Some(match prop.slot {
                            Slot::Data(v) => v,
                            Slot::Accessor { get, .. } => {
                                if get.is_undefined() {
                                    Value::UNDEFINED
                                } else {
                                    let g = Value::object(self.intr.global);
                                    self.call_reentrant(host, get, g, &[])?
                                }
                            }
                        }),
                        Lookup::Exotic(_) => {
                            return Err(type_err("proxy objects are not supported yet"));
                        }
                    },
                };
                let s = self.type_of(v.unwrap_or(Value::UNDEFINED))?;
                self.regs[r(dst)] = Value::string(s);
            }
            Op::DeclareVar { key } => {
                let k = self.codes[code].keys[key as usize];
                if self.heap.get_own(self.intr.lexicals, k)?.is_some() {
                    return Err(already_declared(self, k));
                }
                if self.heap.get_own(self.intr.global, k)?.is_none() {
                    let ok = self.heap.define_own(
                        self.intr.global,
                        k,
                        Property {
                            attrs: Attrs(Attrs::WRITABLE | Attrs::ENUMERABLE),
                            slot: Slot::Data(Value::UNDEFINED),
                        },
                    )?;
                    if !ok {
                        return Err(type_err(format!(
                            "Cannot declare global variable '{}'",
                            self.key_text(k)
                        )));
                    }
                }
            }
            Op::DeclareLex { key, konst } => {
                let k = self.codes[code].keys[key as usize];
                if self.heap.get_own(self.intr.lexicals, k)?.is_some() {
                    return Err(already_declared(self, k));
                }
                if let Some(p) = self.heap.get_own(self.intr.global, k)?
                    && !p.attrs.configurable()
                {
                    return Err(already_declared(self, k));
                }
                let attrs = if konst {
                    Attrs(Attrs::CONFIGURABLE)
                } else {
                    Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE)
                };
                self.heap.define_own(
                    self.intr.lexicals,
                    k,
                    Property {
                        attrs,
                        slot: Slot::Data(Value::UNINIT),
                    },
                )?;
            }
            Op::InitLex { src, key } => {
                let k = self.codes[code].keys[key as usize];
                let v = self.regs[r(src)];
                let Some(p) = self.heap.get_own(self.intr.lexicals, k)? else {
                    return Err(internal_err(format!(
                        "InitLex of '{}', which was never declared",
                        self.key_text(k)
                    )));
                };
                self.heap.define_own(
                    self.intr.lexicals,
                    k,
                    Property {
                        attrs: p.attrs,
                        slot: Slot::Data(v),
                    },
                )?;
            }
            Op::ThrowError { kind, msg } => {
                let kind = ErrorKind::from_index(kind)
                    .ok_or_else(|| internal_err(format!("error kind {kind}")))?;
                let text = self.codes[code]
                    .block
                    .strings
                    .get(msg as usize)
                    .map(|u| String::from_utf16_lossy(u))
                    .unwrap_or_default();
                return Err(Exit::Pending(kind, text));
            }
            Op::DefineField { obj, key, src } => {
                let o = self.object_reg(self.regs[r(obj)])?;
                let k = self.conv_key(self.regs[r(key)])?;
                let v = self.regs[r(src)];
                self.define_field(o, k, v)?;
            }
            Op::DefineAccessor {
                obj,
                key,
                func,
                setter,
            } => {
                let o = self.object_reg(self.regs[r(obj)])?;
                let k = self.conv_key(self.regs[r(key)])?;
                let f = self.regs[r(func)];
                let (mut get, mut set) = (Value::UNDEFINED, Value::UNDEFINED);
                if let Some(Property {
                    slot: Slot::Accessor { get: g, set: s },
                    ..
                }) = self.heap.get_own(o, k)?
                {
                    get = g;
                    set = s;
                }
                if setter {
                    set = f;
                } else {
                    get = f;
                }
                if self.heap.get_own(o, k)?.is_some() {
                    self.heap.delete_own(o, k)?;
                }
                self.heap.define_own(
                    o,
                    k,
                    Property {
                        attrs: Attrs(Attrs::ENUMERABLE | Attrs::CONFIGURABLE),
                        slot: Slot::Accessor { get, set },
                    },
                )?;
            }
            Op::CopyProps { obj, src } => {
                let o = self.object_reg(self.regs[r(obj)])?;
                let s = self.regs[r(src)];
                for (k, v) in self.own_enumerable(host, s)? {
                    self.define_field(o, k, v)?;
                }
            }
            Op::SetProto { obj, src } => {
                let o = self.object_reg(self.regs[r(obj)])?;
                let p = self.regs[r(src)];
                if let Some(po) = p.as_object() {
                    if !self.heap.set_prototype(o, Some(po))? {
                        return Err(type_err("Cyclic __proto__ value"));
                    }
                } else if p.is_null() {
                    self.heap.set_prototype(o, None)?;
                }
            }
            Op::ArrayPush { arr, src } => {
                let a = self.object_reg(self.regs[r(arr)])?;
                let v = self.regs[r(src)];
                self.array_push(a, v)?;
            }
            Op::ArraySpread { arr, src } => {
                let a = self.object_reg(self.regs[r(arr)])?;
                let s = self.regs[r(src)];
                for v in self.spread_values(host, s)? {
                    self.array_push(a, v)?;
                }
            }
            Op::CallSpread {
                dst,
                callee,
                this,
                arr,
            } => {
                let f = self.regs[r(callee)];
                let t = self.regs[r(this)];
                let args = self.spread_values(host, self.regs[r(arr)])?;
                self.invoke(host, f, t, &args, u32::from(dst), FrameKind::Normal)?;
            }
            Op::NewSpread { dst, callee, arr } => {
                let f = self.regs[r(callee)];
                let args = self.spread_values(host, self.regs[r(arr)])?;
                self.construct(host, f, &args, u32::from(dst))?;
            }
            Op::ForInKeys { dst, obj } => {
                let v = self.regs[r(obj)];
                let keys = self.for_in_keys(v)?;
                let a = self.heap.new_array(Some(self.intr.array_prototype))?;
                for k in keys {
                    let s = self.key_string(k)?;
                    self.array_push(a, s)?;
                }
                self.regs[r(dst)] = Value::object(a);
            }
            Op::IterPrepare { dst, src } => {
                let v = self.regs[r(src)];
                let out = if self.is_list_like(v)? {
                    v
                } else if v.is_string() || self.is_iterable_collection(v)? {
                    let a = self.heap.new_array(Some(self.intr.array_prototype))?;
                    for c in self.spread_values(host, v)? {
                        self.array_push(a, c)?;
                    }
                    Value::object(a)
                } else {
                    return Err(not_iterable(self, v));
                };
                self.regs[r(dst)] = out;
            }
            Op::MapArguments { src } => {
                let a = self.object_reg(self.regs[r(src)])?;
                let env = self.frames[fi].env;
                if let ObjectClass::Arguments { env: e, mapped } = self.heap.class_mut(a)? {
                    *e = env;
                    for m in mapped.iter_mut() {
                        *m = true;
                    }
                } else {
                    return Err(internal_err(
                        "MapArguments on something that isn't an arguments object",
                    ));
                }
            }
            Op::DirectEval { dst, args, argc } => {
                let start = r(args);
                let argv: Vec<Value> = self.regs[start..start + argc as usize].to_vec();
                let k = self.intr.key_eval;
                let f = match self.lexical_get(k)? {
                    Some(v) => v,
                    None => match self.heap.lookup(self.intr.global, k)? {
                        Lookup::Found {
                            prop:
                                Property {
                                    slot: Slot::Data(v),
                                    ..
                                },
                            ..
                        } => v,
                        Lookup::NotFound => {
                            return Err(Exit::Pending(
                                ErrorKind::Reference,
                                "eval is not defined".to_string(),
                            ));
                        }
                        _ => {
                            let g = Value::object(self.intr.global);
                            match self.lookup_get(g, k)?.0 {
                                Got::Value(v) => v,
                                Got::Getter(get) => self.call_reentrant(host, get, g, &[])?,
                            }
                        }
                    },
                };
                let builtin = self.eval_fn.is_some_and(|e| e.same_bits(f));
                match (builtin, self.eval_hook) {
                    (true, Some(hook)) => {
                        let src = argv.first().copied().unwrap_or(Value::UNDEFINED);
                        let Some(s) = src.as_string() else {
                            self.regs[r(dst)] = src;
                            return Ok(());
                        };
                        let units = self.heap.string_units(s)?.to_vec();
                        let strict = self.codes[code].block.strict;
                        let v = self.run_eval(host, hook, &units, strict, true)?;
                        self.regs[r(dst)] = v;
                    }
                    _ => self.invoke(
                        host,
                        f,
                        Value::UNDEFINED,
                        &argv,
                        u32::from(dst),
                        FrameKind::Normal,
                    )?,
                }
            }
            _ => return Err(internal_err(format!("op {op:?} reached the slow path"))),
        }
        Ok(())
    }

    /// Run eval code through the compiler's hook, counted like a native
    /// callback since it nests the dispatch loop on the Rust stack.
    pub(crate) fn run_eval(
        &mut self,
        host: &mut dyn Host,
        hook: super::EvalHook,
        units: &[u16],
        strict: bool,
        direct: bool,
    ) -> Result<Value, Exit> {
        let max = self.heap.limits().max_native_reentry;
        if self.native_depth >= max {
            return Err(range_err(format!(
                "Too many nested builtin callbacks ({})",
                crate::anvil_js::limits::group_digits(u64::from(max))
            )));
        }
        self.native_depth += 1;
        let r = hook(self, host, units, strict, direct);
        self.native_depth -= 1;
        Ok(r?)
    }

    /// A global lexical binding's value. `None` when there isn't one;
    /// uninitialized ones throw.
    pub(crate) fn lexical_get(&mut self, k: PropertyKey) -> Result<Option<Value>, Exit> {
        match self.heap.get_own(self.intr.lexicals, k)? {
            None => Ok(None),
            Some(Property {
                slot: Slot::Data(v),
                ..
            }) => {
                if v.is_uninit() {
                    Err(tdz(self, k))
                } else {
                    Ok(Some(v))
                }
            }
            Some(_) => Err(internal_err("accessor in the global lexicals")),
        }
    }

    /// Assign a global lexical binding. False when there isn't one.
    pub(crate) fn lexical_set(&mut self, k: PropertyKey, v: Value) -> Result<bool, Exit> {
        match self.heap.get_own(self.intr.lexicals, k)? {
            None => Ok(false),
            Some(Property {
                slot: Slot::Data(old),
                attrs,
            }) => {
                if old.is_uninit() {
                    return Err(tdz(self, k));
                }
                if !attrs.writable() {
                    return Err(type_err(format!(
                        "Assignment to constant variable '{}'.",
                        self.key_text(k)
                    )));
                }
                self.heap.set_own_value(self.intr.lexicals, k, v)?;
                Ok(true)
            }
            Some(_) => Err(internal_err("accessor in the global lexicals")),
        }
    }

    fn object_reg(&self, v: Value) -> Result<ObjectRef, Exit> {
        v.as_object()
            .ok_or_else(|| internal_err("literal op on something that isn't an object"))
    }

    fn define_field(&mut self, o: ObjectRef, k: PropertyKey, v: Value) -> Result<(), Exit> {
        if let PropertyKey::Index(u32::MAX) = k {
            return Err(internal_err("index 4294967295 is not an array index"));
        }
        if self.heap.get_own(o, k)?.is_some() {
            if self.heap.object(o)?.is_array() && k == self.intr.key_length {
                return Err(type_err("Cannot redefine property: length"));
            }
            self.heap.delete_own(o, k)?;
        }
        if !self.heap.define_own(o, k, Property::data(v))? {
            return Err(type_err(format!(
                "Cannot define property {}, object is not extensible",
                self.key_text(k)
            )));
        }
        Ok(())
    }

    pub(crate) fn array_push(&mut self, a: ObjectRef, v: Value) -> Result<(), Exit> {
        let len = self.heap.array_length(a)?;
        if len == u32::MAX {
            return Err(range_err("Invalid array length"));
        }
        self.heap
            .define_own(a, PropertyKey::Index(len), Property::data(v))?;
        Ok(())
    }

    /// Arrays and arguments objects, which for-of and spread walk by index.
    fn is_list_like(&self, v: Value) -> Result<bool, Exit> {
        Ok(match v.as_object() {
            Some(o) => matches!(
                self.heap.object(o)?.class,
                ObjectClass::Array { .. } | ObjectClass::Arguments { .. }
            ),
            None => false,
        })
    }

    pub(crate) fn read(
        &mut self,
        host: &mut dyn Host,
        recv: Value,
        k: PropertyKey,
    ) -> Result<Value, Exit> {
        match self.lookup_get(recv, k)?.0 {
            Got::Value(v) => Ok(v),
            Got::Getter(g) => Ok(self.call_reentrant(host, g, recv, &[])?),
        }
    }

    /// The values `...v` spreads: array and arguments elements (holes read
    /// as `undefined`), or a string's code points.
    pub(crate) fn spread_values(
        &mut self,
        host: &mut dyn Host,
        v: Value,
    ) -> Result<Vec<Value>, Exit> {
        if let Some(s) = v.as_string() {
            let units = self.heap.string_units(s)?.to_vec();
            let mut out = Vec::new();
            let mut i = 0;
            while i < units.len() {
                let u = units[i];
                let pair = (0xD800..0xDC00).contains(&u)
                    && i + 1 < units.len()
                    && (0xDC00..0xE000).contains(&units[i + 1]);
                let n = if pair { 2 } else { 1 };
                let c = self.heap.alloc_string_units(units[i..i + n].to_vec())?;
                out.push(Value::string(c));
                i += n;
            }
            return Ok(out);
        }
        if let Some(items) = self.spread_collection_or_iter(host, v)? {
            return Ok(items);
        }
        if !self.is_list_like(v)? {
            return Err(not_iterable(self, v));
        }
        let len_v = self.read(host, v, self.intr.key_length)?;
        let len = self.conv_number(len_v)?;
        let len = if len.is_finite() && len > 0.0 {
            len.min(u32::MAX as f64) as u32
        } else {
            0
        };
        if len > MAX_SPREAD {
            return Err(range_err(format!(
                "Cannot spread {} elements (at most {})",
                crate::anvil_js::limits::group_digits(u64::from(len)),
                crate::anvil_js::limits::group_digits(u64::from(MAX_SPREAD))
            )));
        }
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            out.push(self.read(host, v, PropertyKey::Index(i))?);
        }
        Ok(out)
    }

    pub(crate) fn own_enumerable(
        &mut self,
        host: &mut dyn Host,
        v: Value,
    ) -> Result<Vec<(PropertyKey, Value)>, Exit> {
        let mut out = Vec::new();
        if let Some(s) = v.as_string() {
            let n = self.heap.string_len(s)?;
            for i in 0..n {
                let k = PropertyKey::Index(i as u32);
                out.push((k, self.read(host, v, k)?));
            }
            return Ok(out);
        }
        let Some(o) = v.as_object() else {
            return Ok(out);
        };
        for k in self.heap.own_keys(o)? {
            let Some(p) = self.heap.get_own(o, k)? else {
                continue;
            };
            if !p.attrs.enumerable() {
                continue;
            }
            let val = match p.slot {
                Slot::Data(x) => match self.mapped_value(o, k)? {
                    Some(m) => m,
                    None => x,
                },
                Slot::Accessor { get, .. } => {
                    if get.is_undefined() {
                        Value::UNDEFINED
                    } else {
                        self.call_reentrant(host, get, v, &[])?
                    }
                }
            };
            out.push((k, val));
        }
        Ok(out)
    }

    fn mapped_value(&mut self, o: ObjectRef, k: PropertyKey) -> Result<Option<Value>, Exit> {
        match self.lookup_get(Value::object(o), k)?.0 {
            Got::Value(v) => Ok(Some(v)),
            Got::Getter(_) => Ok(None),
        }
    }

    /// The string keys a for-in loop visits: own enumerable keys, then the
    /// prototype's, skipping ones already seen.
    fn for_in_keys(&mut self, v: Value) -> Result<Vec<PropertyKey>, Exit> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let start = if let Some(s) = v.as_string() {
            for i in 0..self.heap.string_len(s)? {
                let k = PropertyKey::Index(i as u32);
                seen.insert(k);
                out.push(k);
            }
            Some(self.intr.string_prototype)
        } else if let Some(o) = v.as_object() {
            Some(o)
        } else if v.is_nullish() {
            None
        } else if v.is_number() {
            Some(self.intr.number_prototype)
        } else if v.is_bool() {
            Some(self.intr.boolean_prototype)
        } else {
            Some(self.intr.symbol_prototype)
        };
        let mut cur = start;
        let mut hops = 0u32;
        while let Some(o) = cur {
            if self.heap.object(o)?.class.is_exotic() {
                return Err(type_err("proxy objects are not supported yet"));
            }
            for k in self.heap.own_keys(o)? {
                if k.is_symbol() || !seen.insert(k) {
                    continue;
                }
                if let Some(p) = self.heap.get_own(o, k)?
                    && p.attrs.enumerable()
                {
                    out.push(k);
                }
            }
            cur = self.heap.get_prototype(o)?;
            hops += 1;
            if hops > 100_000 {
                return Err(internal_err("prototype chain too long"));
            }
        }
        Ok(out)
    }

    fn key_string(&mut self, k: PropertyKey) -> Result<Value, Exit> {
        let text = self.key_text(k);
        Ok(Value::string(self.heap.alloc_string(&text)?))
    }

    pub(crate) fn is_iterable_collection(&self, v: Value) -> Result<bool, Exit> {
        let Some(o) = v.as_object() else {
            return Ok(false);
        };
        Ok(match &self.heap.object(o)?.class {
            ObjectClass::Collection(m) => !m.kind.is_weak(),
            ObjectClass::CollectionIterator { .. } | ObjectClass::ListIterator { .. } => true,
            _ => false,
        })
    }

    pub(crate) fn spread_collection_or_iter(
        &mut self,
        host: &mut dyn Host,
        v: Value,
    ) -> Result<Option<Vec<Value>>, Exit> {
        use crate::anvil_js::value::collection::CollectionKind;
        use crate::anvil_js::value::object::IterKind;
        let Some(o) = v.as_object() else {
            return Ok(None);
        };
        let class = self.heap.object(o)?.class.clone();
        match class {
            ObjectClass::Collection(m) if !m.kind.is_weak() => {
                let mut out = Vec::with_capacity(m.size());
                let mut pos = 0;
                while let Some((idx, k, val)) = m.next_from(pos) {
                    if m.kind == CollectionKind::Set {
                        out.push(k);
                    } else {
                        let pair = self.heap.new_array(Some(self.intr.array_prototype))?;
                        self.array_push(pair, k)?;
                        self.array_push(pair, val)?;
                        out.push(Value::object(pair));
                    }
                    pos = idx + 1;
                }
                Ok(Some(out))
            }
            ObjectClass::CollectionIterator {
                map: Some(map_obj),
                pos,
                kind,
            } => {
                let entries = match &self.heap.object(map_obj)?.class {
                    ObjectClass::Collection(m) => {
                        let mut list = Vec::new();
                        let mut cur = pos as usize;
                        while let Some((idx, k, val)) = m.next_from(cur) {
                            list.push((k, val));
                            cur = idx + 1;
                        }
                        list
                    }
                    _ => Vec::new(),
                };
                let _ = self.heap.with_collection(map_obj, |m| {
                    m.open_iterators = m.open_iterators.saturating_sub(1);
                });
                if let ObjectClass::CollectionIterator { map, .. } = self.heap.class_mut(o)? {
                    *map = None;
                }
                let mut out = Vec::with_capacity(entries.len());
                for (k, val) in entries {
                    let item = match kind {
                        IterKind::Keys => k,
                        IterKind::Values => val,
                        IterKind::Entries => {
                            let pair = self.heap.new_array(Some(self.intr.array_prototype))?;
                            self.array_push(pair, k)?;
                            self.array_push(pair, val)?;
                            Value::object(pair)
                        }
                    };
                    out.push(item);
                }
                Ok(Some(out))
            }
            ObjectClass::CollectionIterator { map: None, .. } => Ok(Some(Vec::new())),
            ObjectClass::ListIterator {
                target,
                index,
                kind,
            } => {
                if target.is_undefined() {
                    return Ok(Some(Vec::new()));
                }
                let len_v = self.read(host, target, self.intr.key_length)?;
                let len = self.conv_number(len_v)?;
                let len = if len.is_finite() && len > 0.0 {
                    len.min(u32::MAX as f64) as u32
                } else {
                    0
                };
                if let ObjectClass::ListIterator { target: t, .. } = self.heap.class_mut(o)? {
                    *t = Value::UNDEFINED;
                }
                let mut out = Vec::new();
                for i in index..len {
                    let val = self.read(host, target, PropertyKey::Index(i))?;
                    let item = match kind {
                        IterKind::Keys => Value::number(f64::from(i)),
                        IterKind::Values => val,
                        IterKind::Entries => {
                            let pair = self.heap.new_array(Some(self.intr.array_prototype))?;
                            self.array_push(pair, Value::number(f64::from(i)))?;
                            self.array_push(pair, val)?;
                            Value::object(pair)
                        }
                    };
                    out.push(item);
                }
                Ok(Some(out))
            }
            _ => Ok(None),
        }
    }

    #[allow(dead_code)]
    fn key_string_legacy(&mut self, k: PropertyKey) -> Result<Value, Exit> {
        let text = self.key_text(k);
        Ok(Value::string(self.heap.alloc_string(&text)?))
    }
}

fn already_declared(vm: &Vm, k: PropertyKey) -> Exit {
    Exit::Pending(
        ErrorKind::Syntax,
        format!("Identifier '{}' has already been declared", vm.key_text(k)),
    )
}

fn tdz(vm: &Vm, k: PropertyKey) -> Exit {
    Exit::Pending(
        ErrorKind::Reference,
        format!("Cannot access '{}' before initialization", vm.key_text(k)),
    )
}

fn not_iterable(vm: &mut Vm, v: Value) -> Exit {
    let what = vm.callee_text(v);
    type_err(format!(
        "{what} is not iterable (only arrays, strings and arguments objects can be iterated so far)"
    ))
}
