// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The few globals compiled scripts need before the standard library
//! exists: `globalThis`, `undefined`, `NaN`, `Infinity`, `console`, the
//! error constructors, `eval`, `Function.prototype.call` and `apply`, and
//! `Array.prototype.push`.

use super::host::NativeCx;
use super::interp::Got;
use super::{ErrorKind, Exit, RunError, Vm, heap_to_run};
use crate::anvil_js::value::atom::PropertyKey;
use crate::anvil_js::value::number::number_to_string;
use crate::anvil_js::value::object::{ObjectClass, Property, Slot};
use crate::anvil_js::value::shape::Attrs;
use crate::anvil_js::value::{ObjectRef, Value};

/// How deep `console.log` prints nested objects before writing
/// `[Object]` or `[Array]`.
const INSPECT_DEPTH: usize = 2;

/// Most properties `console.log` prints of one object.
const INSPECT_ITEMS: usize = 100;

impl Vm {
    /// Add the basic globals. `eval` is added here too but only works once
    /// the compiler has set the eval hook.
    pub fn install_basics(&mut self) -> Result<(), RunError> {
        let fixed = Attrs(0);
        let g = self.intr.global;
        for (name, v) in [
            ("globalThis", Value::object(g)),
            ("undefined", Value::UNDEFINED),
            ("NaN", Value::number(f64::NAN)),
            ("Infinity", Value::number(f64::INFINITY)),
        ] {
            let attrs = if name == "globalThis" {
                Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE)
            } else {
                fixed
            };
            self.define_hidden(g, name, v, attrs)?;
        }
        let console = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let methods: [(&str, super::NativeFn); 4] = [
            ("log", console_log),
            ("info", console_info),
            ("warn", console_warn),
            ("error", console_error),
        ];
        for (name, f) in methods {
            let fv = self.register_native(name, f)?;
            self.define_hidden(
                console,
                name,
                fv,
                Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE | Attrs::ENUMERABLE),
            )?;
        }
        self.define_hidden(
            g,
            "console",
            Value::object(console),
            Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
        )?;
        let ctors: [super::NativeFn; 5] = [
            error_ctor,
            type_error_ctor,
            range_error_ctor,
            reference_error_ctor,
            syntax_error_ctor,
        ];
        for (kind, f) in ErrorKind::ALL.into_iter().zip(ctors) {
            let c = self.register_native(kind.name(), f)?;
            let co = c
                .as_object()
                .ok_or_else(|| RunError::internal("native function is not an object"))?;
            let proto = self.intr.error_prototypes[kind.index()];
            self.define_hidden(co, "prototype", Value::object(proto), fixed)?;
            self.define_hidden(
                proto,
                "constructor",
                c,
                Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
            )?;
            self.define_hidden(
                g,
                kind.name(),
                c,
                Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
            )?;
        }
        let fp = self.intr.function_prototype;
        let call = self.register_native("call", function_call)?;
        let apply = self.register_native("apply", function_apply)?;
        let hidden = Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE);
        self.define_hidden(fp, "call", call, hidden)?;
        self.define_hidden(fp, "apply", apply, hidden)?;
        let push = self.register_native("push", array_push)?;
        self.define_hidden(self.intr.array_prototype, "push", push, hidden)?;
        let eval = self.register_native("eval", indirect_eval)?;
        self.define_hidden(
            g,
            "eval",
            eval,
            Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
        )?;
        self.eval_fn = Some(eval);
        self.install_stdlib()?;
        Ok(())
    }

    pub(crate) fn define_hidden(
        &mut self,
        obj: ObjectRef,
        name: &str,
        v: Value,
        attrs: Attrs,
    ) -> Result<(), RunError> {
        let key = self.heap.key_from_str(name).map_err(heap_to_run)?;
        if self.heap.get_own(obj, key).map_err(heap_to_run)?.is_some() {
            self.heap.delete_own(obj, key).map_err(heap_to_run)?;
        }
        self.heap
            .define_own(
                obj,
                key,
                Property {
                    attrs,
                    slot: Slot::Data(v),
                },
            )
            .map_err(heap_to_run)?;
        Ok(())
    }

    /// The name of a function object, as `f.name` reads it.
    pub(crate) fn function_name(&self, o: ObjectRef) -> Option<String> {
        match self.heap.object(o).ok()?.class {
            ObjectClass::Function { code, .. } => {
                Some(self.codes.get(code as usize)?.block.name.clone())
            }
            ObjectClass::Native { id } => Some(self.natives.get(id as usize)?.name.clone()),
            ObjectClass::Bound(ref b) => Some(b.name.clone()),
            _ => None,
        }
    }

    /// The parameter count of a function object, as `f.length` reads it.
    pub(crate) fn function_length(&self, o: ObjectRef) -> Option<u32> {
        match self.heap.object(o).ok()?.class {
            ObjectClass::Function { code, .. } => {
                Some(u32::from(self.codes.get(code as usize)?.block.params))
            }
            ObjectClass::Native { .. } => Some(0),
            ObjectClass::Bound(ref b) => Some(b.length),
            _ => None,
        }
    }

    /// `console.log` formatting: strings as they are at the top level,
    /// everything else the way an interactive console shows it.
    pub fn inspect(&mut self, v: Value) -> Result<String, RunError> {
        let mut seen = Vec::new();
        self.inspect_at(v, 0, true, &mut seen)
            .map_err(|e| self.exit_to_run(e))
    }

    fn inspect_at(
        &mut self,
        v: Value,
        depth: usize,
        top: bool,
        seen: &mut Vec<ObjectRef>,
    ) -> Result<String, Exit> {
        if let Some(s) = v.as_string() {
            let text = self.heap.string_to_rust(s)?;
            return Ok(if top { text } else { quote(&text) });
        }
        if let Some(n) = v.as_number() {
            return Ok(if n == 0.0 && n.is_sign_negative() {
                "-0".to_string()
            } else {
                number_to_string(n)
            });
        }
        let Some(o) = v.as_object() else {
            let u = self.display_units(v)?;
            return Ok(String::from_utf16_lossy(&u));
        };
        if seen.contains(&o) {
            return Ok("[Circular]".to_string());
        }
        let class = self.heap.object(o)?.class.clone();
        match class {
            ObjectClass::Function { .. } | ObjectClass::Native { .. } | ObjectClass::Bound(_) => {
                let name = self.function_name(o).unwrap_or_default();
                return Ok(if name.is_empty() {
                    "[Function (anonymous)]".to_string()
                } else {
                    format!("[Function: {name}]")
                });
            }
            ObjectClass::Error => {
                let g = |vm: &mut Vm, k: PropertyKey| -> Result<String, Exit> {
                    match vm.lookup_get(v, k)?.0 {
                        Got::Value(x) if !x.is_undefined() => {
                            let u = vm.display_units(x)?;
                            Ok(String::from_utf16_lossy(&u))
                        }
                        _ => Ok(String::new()),
                    }
                };
                let name = g(self, self.intr.key_name)?;
                let msg = g(self, self.intr.key_message)?;
                let text = if msg.is_empty() {
                    name
                } else if name.is_empty() {
                    msg
                } else {
                    format!("{name}: {msg}")
                };
                return Ok(if top { text } else { format!("[{text}]") });
            }
            ObjectClass::Primitive(p) => {
                let inner = self.inspect_at(p, depth, false, seen)?;
                let kind = if p.is_string() {
                    "String"
                } else if p.is_number() {
                    "Number"
                } else if p.is_bool() {
                    "Boolean"
                } else {
                    "Symbol"
                };
                return Ok(format!("[{kind}: {inner}]"));
            }
            _ => {}
        }
        let is_array = matches!(class, ObjectClass::Array { .. });
        let is_args = matches!(class, ObjectClass::Arguments { .. });
        if depth > INSPECT_DEPTH {
            return Ok(if is_array { "[Array]" } else { "[Object]" }.to_string());
        }
        seen.push(o);
        let mut parts = Vec::new();
        let keys = self.heap.own_keys(o)?;
        if is_array || is_args {
            // Walk the elements that exist and count the gaps between
            // them, so a sparse array with a huge length costs only what
            // it holds. Each element visited counts as a step.
            let len = if is_array {
                self.heap.array_length(o)?
            } else {
                keys.iter()
                    .filter_map(|k| match k {
                        PropertyKey::Index(i) => Some(i.saturating_add(1)),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0)
            };
            let mut idx: Vec<u32> = keys
                .iter()
                .filter_map(|k| match k {
                    PropertyKey::Index(i) if *i < len => Some(*i),
                    _ => None,
                })
                .collect();
            idx.sort_unstable();
            let holes = |n: u32, parts: &mut Vec<String>| {
                if n > 0 {
                    let s = if n == 1 { "" } else { "s" };
                    parts.push(format!("<{n} empty item{s}>"));
                }
            };
            let mut next = 0u32;
            for i in idx {
                if parts.len() >= INSPECT_ITEMS {
                    break;
                }
                self.charge_step()?;
                holes(i - next, &mut parts);
                if parts.len() >= INSPECT_ITEMS {
                    break;
                }
                let x = match self.mapped_slot(o, PropertyKey::Index(i))? {
                    Some((env, slot)) => self.heap.env_get(env, slot)?,
                    None => match self.heap.get_own(o, PropertyKey::Index(i))? {
                        Some(p) => self.slot_value(p.slot),
                        None => Value::UNDEFINED,
                    },
                };
                parts.push(self.inspect_at(x, depth + 1, false, seen)?);
                next = i + 1;
            }
            if next < len {
                if parts.len() >= INSPECT_ITEMS {
                    parts.push(format!("... {} more items", len - next));
                } else {
                    holes(len - next, &mut parts);
                }
            }
        }
        for k in keys {
            if (is_array || is_args) && matches!(k, PropertyKey::Index(_)) {
                continue;
            }
            if is_array && k == self.intr.key_length {
                continue;
            }
            let Some(p) = self.heap.get_own(o, k)? else {
                continue;
            };
            if !p.attrs.enumerable() {
                continue;
            }
            if parts.len() >= INSPECT_ITEMS {
                parts.push("...".to_string());
                break;
            }
            self.charge_step()?;
            let name = match k {
                PropertyKey::Symbol(_) => format!("[{}]", self.key_text(k)),
                _ => {
                    let t = self.key_text(k);
                    if is_identifier(&t) || matches!(k, PropertyKey::Index(_)) {
                        t
                    } else {
                        quote(&t)
                    }
                }
            };
            let text = match p.slot {
                Slot::Data(x) => {
                    let x = match self.mapped_slot(o, k)? {
                        Some((env, slot)) => self.heap.env_get(env, slot)?,
                        None => x,
                    };
                    self.inspect_at(x, depth + 1, false, seen)?
                }
                Slot::Accessor { get, set } => match (get.is_undefined(), set.is_undefined()) {
                    (false, false) => "[Getter/Setter]".to_string(),
                    (false, true) => "[Getter]".to_string(),
                    _ => "[Setter]".to_string(),
                },
            };
            parts.push(format!("{name}: {text}"));
        }
        seen.pop();
        let prefix = if matches!(class, ObjectClass::Arguments { .. }) {
            "[Arguments] "
        } else {
            ""
        };
        Ok(match (is_array || !prefix.is_empty(), parts.is_empty()) {
            (true, true) => format!("{prefix}[]"),
            (true, false) if prefix.is_empty() => format!("[ {} ]", parts.join(", ")),
            (true, false) => format!("{prefix}[ {} ]", parts.join(", ")),
            (false, true) => "{}".to_string(),
            (false, false) => format!("{{ {} }}", parts.join(", ")),
        })
    }

    /// Count one step of formatting work against `max_steps`.
    fn charge_step(&mut self) -> Result<(), Exit> {
        self.steps += 1;
        self.total_steps += 1;
        if let Some(max) = self.heap.limits().max_steps
            && self.steps > max
        {
            self.steps = max;
            return Err(Exit::Fatal(super::steps_abort(max)));
        }
        Ok(())
    }

    fn slot_value(&self, s: Slot) -> Value {
        match s {
            Slot::Data(v) => v,
            Slot::Accessor { .. } => Value::UNDEFINED,
        }
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn quote(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn console_line(cx: &mut NativeCx<'_>, level: &str, args: &[Value]) -> Result<Value, RunError> {
    let mut parts = Vec::with_capacity(args.len());
    for a in args {
        parts.push(cx.vm.inspect(*a)?);
    }
    let text = parts.join(" ");
    cx.host.console(level, &text);
    Ok(Value::UNDEFINED)
}

fn console_log(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    console_line(cx, "log", args)
}

fn console_info(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    console_line(cx, "info", args)
}

fn console_warn(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    console_line(cx, "warn", args)
}

fn console_error(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    console_line(cx, "error", args)
}

fn make_error(cx: &mut NativeCx<'_>, kind: ErrorKind, args: &[Value]) -> Result<Value, RunError> {
    let vm = &mut *cx.vm;
    let msg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let r = (|| -> Result<Value, Exit> {
        let e = vm.make_error_value(kind, "")?;
        let o = e
            .as_object()
            .ok_or_else(|| super::internal_err("error is not an object"))?;
        if msg.is_undefined() {
            vm.heap.delete_own(o, vm.intr.key_message)?;
        } else {
            let s = vm.conv_string(msg)?;
            vm.heap
                .set_own_value(o, vm.intr.key_message, Value::string(s))?;
        }
        Ok(e)
    })();
    r.map_err(|e| vm.exit_to_run(e))
}

fn error_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    make_error(cx, ErrorKind::Error, args)
}

fn type_error_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    make_error(cx, ErrorKind::Type, args)
}

fn range_error_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    make_error(cx, ErrorKind::Range, args)
}

fn reference_error_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    make_error(cx, ErrorKind::Reference, args)
}

fn syntax_error_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    make_error(cx, ErrorKind::Syntax, args)
}

/// `eval` called any way but directly: sloppy global code.
fn indirect_eval(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let src = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(s) = src.as_string() else {
        return Ok(src);
    };
    let Some(hook) = cx.vm.eval_hook else {
        return Err(cx.vm.make_thrown(
            ErrorKind::Error,
            "eval is not available: no compiler installed",
        ));
    };
    let units = cx.vm.heap.string_units(s).map_err(heap_to_run)?.to_vec();
    hook(cx.vm, cx.host, &units, false, false)
}

/// `f.call(thisArg, ...args)`
fn function_call(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let t = args.first().copied().unwrap_or(Value::UNDEFINED);
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    cx.call(this, t, rest)
}

/// `f.apply(thisArg, array)`
fn function_apply(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let t = args.first().copied().unwrap_or(Value::UNDEFINED);
    let list = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let argv = if list.is_nullish() {
        Vec::new()
    } else {
        let vm = &mut *cx.vm;
        match vm.spread_values(cx.host, list) {
            Ok(v) => v,
            Err(e) => return Err(vm.exit_to_run(e)),
        }
    };
    for v in &argv {
        cx.keep(*v);
    }
    cx.call(this, t, &argv)
}

/// `array.push(...items)`, for arrays only so far.
fn array_push(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let vm = &mut *cx.vm;
    let Some(o) = this
        .as_object()
        .filter(|o| vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Err(vm.make_thrown(ErrorKind::Type, "push is only supported on arrays so far"));
    };
    for v in args {
        let len = vm.heap.array_length(o).map_err(heap_to_run)?;
        if len == u32::MAX {
            return Err(vm.make_thrown(ErrorKind::Range, "Invalid array length"));
        }
        vm.heap
            .define_own(o, PropertyKey::Index(len), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    let len = vm.heap.array_length(o).map_err(heap_to_run)?;
    Ok(Value::number(f64::from(len)))
}
