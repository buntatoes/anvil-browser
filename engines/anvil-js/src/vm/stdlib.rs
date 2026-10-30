// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Built-in ECMAScript (ES2022-ES2024) standard library for `Vm`:
//! `Object`, `Function`, `Array`, `String`, `Number`, `Boolean`, `Symbol`,
//! `Math`, `JSON`, `Map`, `Set`, `Promise`, `Date`, `performance`,
//! `structuredClone`, `atob`, `btoa`, `encodeURIComponent`,
//! `decodeURIComponent`, `parseInt`, `parseFloat`, `isNaN`, `isFinite`,
//! and `queueMicrotask`.

use super::host::{NativeCx, NativeFn};
use super::{ErrorKind, Exit, RunError, Vm, heap_to_run};
use crate::anvil_js::value::atom::PropertyKey;
use crate::anvil_js::value::collection::CollectionKind;
use crate::anvil_js::value::number::{number_to_string, to_int32, to_uint32};
use crate::anvil_js::value::object::{BoundFunction, IterKind, ObjectClass, Property, Slot};
use crate::anvil_js::value::shape::Attrs;
use crate::anvil_js::value::{ObjectRef, Value};
use std::collections::HashMap;

const HIDDEN: Attrs = Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE);
const FIXED: Attrs = Attrs(0);

impl Vm {
    /// Install the built-in ECMAScript standard library onto the global object
    /// and intrinsic prototypes.
    pub fn install_stdlib(&mut self) -> Result<(), RunError> {
        self.install_global_functions()?;
        self.install_object_builtins()?;
        self.install_function_builtins()?;
        self.install_array_builtins()?;
        self.install_string_builtins()?;
        self.install_number_and_boolean_builtins()?;
        self.install_symbol_builtins()?;
        self.install_math_builtins()?;
        self.install_json_builtins()?;
        self.install_collection_builtins()?;
        self.install_promise_builtins()?;
        self.install_regexp_builtins()?;
        self.install_web_and_timer_builtins()?;
        Ok(())
    }

    pub(crate) fn def_method(
        &mut self,
        target: ObjectRef,
        name: &str,
        f: NativeFn,
    ) -> Result<Value, RunError> {
        let fv = self.register_native(name, f)?;
        self.define_hidden(target, name, fv, HIDDEN)?;
        Ok(fv)
    }

    /// Define a native method on `target` (`writable`, `configurable`).
    pub fn define_method(
        &mut self,
        target: ObjectRef,
        name: &str,
        f: NativeFn,
    ) -> Result<Value, RunError> {
        self.def_method(target, name, f)
    }

    /// Define a data property on `target` (`writable`, `enumerable`, `configurable`).
    pub fn define_data(
        &mut self,
        target: ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RunError> {
        self.define_hidden(target, name, value, Attrs::DEFAULT)
    }

    /// Define an accessor property (`getter` and/or `setter`) on `target`.
    pub fn define_accessor(
        &mut self,
        target: ObjectRef,
        name: &str,
        getter: Option<NativeFn>,
        setter: Option<NativeFn>,
    ) -> Result<(), RunError> {
        let get = match getter {
            Some(f) => self.register_native(&format!("get {name}"), f)?,
            None => Value::UNDEFINED,
        };
        let set = match setter {
            Some(f) => self.register_native(&format!("set {name}"), f)?,
            None => Value::UNDEFINED,
        };
        let key = self.heap.key_from_str(name).map_err(heap_to_run)?;
        if self
            .heap
            .get_own(target, key)
            .map_err(heap_to_run)?
            .is_some()
        {
            self.heap.delete_own(target, key).map_err(heap_to_run)?;
        }
        self.heap
            .define_own(
                target,
                key,
                Property {
                    attrs: Attrs(Attrs::ENUMERABLE | Attrs::CONFIGURABLE),
                    slot: Slot::Accessor { get, set },
                },
            )
            .map_err(heap_to_run)?;
        Ok(())
    }

    /// Allocate a plain JavaScript object inheriting from `Object.prototype`.
    pub fn new_plain_object(&mut self) -> Result<ObjectRef, RunError> {
        self.heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)
    }

    /// Allocate a host wrapper object with prototype `proto` and tag `data`.
    pub fn new_host_object(
        &mut self,
        proto: Option<ObjectRef>,
        data: crate::anvil_js::value::object::HostData,
    ) -> Result<ObjectRef, RunError> {
        self.heap
            .alloc_object(proto, ObjectClass::Host(data))
            .map_err(heap_to_run)
    }

    /// Allocate a dense JavaScript `Array` populated with `items`.
    pub fn new_array_from(&mut self, items: &[Value]) -> Result<Value, RunError> {
        let arr = self
            .heap
            .new_array(Some(self.intr.array_prototype))
            .map_err(heap_to_run)?;
        for (i, item) in items.iter().enumerate() {
            self.heap
                .define_own(arr, PropertyKey::Index(i as u32), Property::data(*item))
                .map_err(heap_to_run)?;
        }
        Ok(Value::object(arr))
    }

    fn install_global_functions(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        for (name, f) in [
            ("parseInt", global_parse_int as NativeFn),
            ("parseFloat", global_parse_float),
            ("isNaN", global_is_nan),
            ("isFinite", global_is_finite),
            ("encodeURIComponent", global_encode_uri_component),
            ("decodeURIComponent", global_decode_uri_component),
            ("encodeURI", global_encode_uri_component),
            ("decodeURI", global_decode_uri_component),
            ("atob", global_atob),
            ("btoa", global_btoa),
            ("structuredClone", global_structured_clone),
            ("queueMicrotask", global_queue_microtask),
        ] {
            self.def_method(g, name, f)?;
        }
        Ok(())
    }

    fn install_object_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let op = self.intr.object_prototype;
        let ctor = self.register_native("Object", object_ctor)?;
        let co = ctor.as_object().unwrap();
        self.define_hidden(co, "prototype", Value::object(op), FIXED)?;
        self.define_hidden(op, "constructor", ctor, HIDDEN)?;
        for (name, f) in [
            ("keys", object_keys as NativeFn),
            ("values", object_values),
            ("entries", object_entries),
            ("fromEntries", object_from_entries),
            ("assign", object_assign),
            ("create", object_create),
            ("getPrototypeOf", object_get_prototype_of),
            ("setPrototypeOf", object_set_prototype_of),
            ("hasOwn", object_has_own),
            ("is", object_is),
            ("defineProperty", object_define_property),
            (
                "getOwnPropertyDescriptor",
                object_get_own_property_descriptor,
            ),
            ("getOwnPropertyNames", object_get_own_property_names),
            ("freeze", object_freeze),
            ("seal", object_freeze),
            ("preventExtensions", object_prevent_extensions),
            ("isFrozen", object_is_frozen),
            ("isSealed", object_is_frozen),
            ("isExtensible", object_is_extensible),
            ("groupBy", object_group_by),
            (
                "getOwnPropertyDescriptors",
                object_get_own_property_descriptors,
            ),
            ("defineProperties", object_define_properties),
            ("getOwnPropertySymbols", object_get_own_property_symbols),
        ] {
            self.def_method(co, name, f)?;
        }
        for (name, f) in [
            ("hasOwnProperty", object_proto_has_own_property as NativeFn),
            ("isPrototypeOf", object_proto_is_prototype_of),
            ("propertyIsEnumerable", object_proto_property_is_enumerable),
            ("toString", object_proto_to_string),
            ("valueOf", object_proto_value_of),
        ] {
            self.def_method(op, name, f)?;
        }
        self.define_hidden(g, "Object", ctor, HIDDEN)?;
        Ok(())
    }

    fn install_function_builtins(&mut self) -> Result<(), RunError> {
        let fp = self.intr.function_prototype;
        self.def_method(fp, "bind", function_bind)?;
        self.def_method(fp, "toString", function_to_string)?;
        Ok(())
    }

    fn install_array_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let ap = self.intr.array_prototype;
        let ctor = self.register_native("Array", array_ctor)?;
        let co = ctor.as_object().unwrap();
        self.define_hidden(co, "prototype", Value::object(ap), FIXED)?;
        self.define_hidden(ap, "constructor", ctor, HIDDEN)?;
        for (name, f) in [
            ("isArray", array_is_array as NativeFn),
            ("from", array_from),
            ("of", array_of),
        ] {
            self.def_method(co, name, f)?;
        }
        for (name, f) in [
            ("pop", array_pop as NativeFn),
            ("shift", array_shift),
            ("unshift", array_unshift),
            ("slice", array_slice),
            ("splice", array_splice),
            ("concat", array_concat),
            ("join", array_join),
            ("indexOf", array_index_of),
            ("lastIndexOf", array_last_index_of),
            ("includes", array_includes),
            ("forEach", array_for_each),
            ("map", array_map),
            ("filter", array_filter),
            ("reduce", array_reduce),
            ("reduceRight", array_reduce_right),
            ("some", array_some),
            ("every", array_every),
            ("find", array_find),
            ("findIndex", array_find_index),
            ("findLast", array_find_last),
            ("findLastIndex", array_find_last_index),
            ("flat", array_flat),
            ("flatMap", array_flat_map),
            ("at", array_at),
            ("reverse", array_reverse),
            ("sort", array_sort),
            ("fill", array_fill),
            ("toReversed", array_to_reversed),
            ("toSorted", array_to_sorted),
            ("toSpliced", array_to_spliced),
            ("with", array_with),
            ("keys", array_keys),
            ("values", array_values),
            ("entries", array_entries),
            ("toString", array_to_string),
        ] {
            self.def_method(ap, name, f)?;
        }
        self.define_hidden(g, "Array", ctor, HIDDEN)?;
        Ok(())
    }

    fn install_string_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let sp = self.intr.string_prototype;
        let ctor = self.register_native("String", string_ctor)?;
        let co = ctor.as_object().unwrap();
        self.define_hidden(co, "prototype", Value::object(sp), FIXED)?;
        self.define_hidden(sp, "constructor", ctor, HIDDEN)?;
        self.def_method(co, "fromCharCode", string_from_char_code)?;
        self.def_method(co, "fromCodePoint", string_from_code_point)?;
        for (name, f) in [
            ("charAt", string_char_at as NativeFn),
            ("charCodeAt", string_char_code_at),
            ("codePointAt", string_code_point_at),
            ("at", string_at),
            ("slice", string_slice),
            ("substring", string_substring),
            ("substr", string_substr),
            ("concat", string_concat),
            ("indexOf", string_index_of),
            ("lastIndexOf", string_last_index_of),
            ("includes", string_includes),
            ("startsWith", string_starts_with),
            ("endsWith", string_ends_with),
            ("trim", string_trim),
            ("trimStart", string_trim_start),
            ("trimLeft", string_trim_start),
            ("trimEnd", string_trim_end),
            ("trimRight", string_trim_end),
            ("padStart", string_pad_start),
            ("padEnd", string_pad_end),
            ("repeat", string_repeat),
            ("split", string_split),
            ("replace", string_replace),
            ("replaceAll", string_replace_all),
            ("match", string_match),
            ("matchAll", string_match_all),
            ("search", string_search),
            ("toLowerCase", string_to_lower_case),
            ("toUpperCase", string_to_upper_case),
            ("toString", string_to_string),
            ("valueOf", string_to_string),
        ] {
            self.def_method(sp, name, f)?;
        }
        self.define_hidden(g, "String", ctor, HIDDEN)?;
        Ok(())
    }
}

// ---- helpers --------------------------------------------------------------

fn to_num(cx: &mut NativeCx<'_>, v: Value) -> Result<f64, RunError> {
    cx.vm.conv_number(v).map_err(|e| cx.vm.exit_to_run(e))
}

fn to_str_rust(cx: &mut NativeCx<'_>, v: Value) -> Result<String, RunError> {
    let s = cx.vm.conv_string(v).map_err(|e| cx.vm.exit_to_run(e))?;
    cx.vm.heap.string_to_rust(s).map_err(heap_to_run)
}

fn to_int_or_inf(cx: &mut NativeCx<'_>, v: Value) -> Result<f64, RunError> {
    let n = to_num(cx, v)?;
    if n.is_nan() || n == 0.0 {
        Ok(0.0)
    } else if n.is_infinite() {
        Ok(n)
    } else {
        Ok(n.trunc())
    }
}

fn make_array_from_slice(cx: &mut NativeCx<'_>, items: &[Value]) -> Result<Value, RunError> {
    let arr = cx
        .vm
        .heap
        .new_array(Some(cx.vm.intr.array_prototype))
        .map_err(heap_to_run)?;
    for (i, item) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(arr, PropertyKey::Index(i as u32), Property::data(*item))
            .map_err(heap_to_run)?;
    }
    Ok(Value::object(arr))
}

fn read_array_items(cx: &mut NativeCx<'_>, recv: Value) -> Result<Vec<Value>, RunError> {
    if recv.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let len_v = cx
        .vm
        .read(cx.host, recv, cx.vm.intr.key_length)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let len_n = to_int_or_inf(cx, len_v)?;
    let len = if len_n <= 0.0 {
        0
    } else {
        len_n.min(super::extra::MAX_SPREAD as f64) as u32
    };
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let v = cx
            .vm
            .read(cx.host, recv, PropertyKey::Index(i))
            .map_err(|e| cx.vm.exit_to_run(e))?;
        out.push(v);
    }
    Ok(out)
}

fn same_value_zero(cx: &mut NativeCx<'_>, a: Value, b: Value) -> Result<bool, RunError> {
    if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
        if x.is_nan() && y.is_nan() {
            return Ok(true);
        }
        return Ok(x == y);
    }
    cx.vm.strict_eq(a, b).map_err(|e| cx.vm.exit_to_run(e))
}

// ---- Object builtins ------------------------------------------------------

fn object_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    if v.is_nullish() {
        let o = cx
            .vm
            .heap
            .new_object(Some(cx.vm.intr.object_prototype))
            .map_err(heap_to_run)?;
        return Ok(Value::object(o));
    }
    if v.is_object() {
        return Ok(v);
    }
    let proto = if v.is_string() {
        cx.vm.intr.string_prototype
    } else if v.is_number() {
        cx.vm.intr.number_prototype
    } else if v.is_bool() {
        cx.vm.intr.boolean_prototype
    } else {
        cx.vm.intr.symbol_prototype
    };
    let o = cx
        .vm
        .heap
        .alloc_object(Some(proto), ObjectClass::Primitive(v))
        .map_err(heap_to_run)?;
    Ok(Value::object(o))
}

fn object_keys(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if target.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let pairs = cx
        .vm
        .own_enumerable(cx.host, target)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut out = Vec::with_capacity(pairs.len());
    for (k, _) in pairs {
        if k.is_symbol() {
            continue;
        }
        let text = cx.vm.key_text(k);
        out.push(cx.vm.string(&text)?);
    }
    make_array_from_slice(cx, &out)
}

fn object_values(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if target.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let pairs = cx
        .vm
        .own_enumerable(cx.host, target)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut out = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        if !k.is_symbol() {
            out.push(v);
        }
    }
    make_array_from_slice(cx, &out)
}

fn object_entries(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if target.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let pairs = cx
        .vm
        .own_enumerable(cx.host, target)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut out = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        if k.is_symbol() {
            continue;
        }
        let ks = cx.vm.string(&cx.vm.key_text(k))?;
        let pair = make_array_from_slice(cx, &[ks, v])?;
        out.push(pair);
    }
    make_array_from_slice(cx, &out)
}

fn object_from_entries(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let iter = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, iter)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let out = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.keep(Value::object(out));
    for item in items {
        let Some(po) = item.as_object() else {
            return Err(cx.type_error("Iterator value is not an entry object"));
        };
        let k = cx
            .vm
            .read(cx.host, Value::object(po), PropertyKey::Index(0))
            .map_err(|e| cx.vm.exit_to_run(e))?;
        let v = cx
            .vm
            .read(cx.host, Value::object(po), PropertyKey::Index(1))
            .map_err(|e| cx.vm.exit_to_run(e))?;
        let pk = cx.vm.conv_key(k).map_err(|e| cx.vm.exit_to_run(e))?;
        cx.vm
            .heap
            .define_own(out, pk, Property::data(v))
            .map_err(heap_to_run)?;
    }
    Ok(Value::object(out))
}

fn object_assign(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(to) = target.as_object() else {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    };
    for src in args.iter().skip(1) {
        if src.is_nullish() {
            continue;
        }
        let pairs = cx
            .vm
            .own_enumerable(cx.host, *src)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for (k, v) in pairs {
            cx.vm
                .set_op(cx.host, Value::object(to), k, v, None)
                .map_err(|e| cx.vm.exit_to_run(e))?;
        }
    }
    Ok(Value::object(to))
}

fn object_create(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let proto_arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let proto = if proto_arg.is_null() {
        None
    } else if let Some(po) = proto_arg.as_object() {
        Some(po)
    } else {
        return Err(cx.type_error("Object prototype may only be an Object or null"));
    };
    let obj = cx.vm.heap.new_object(proto).map_err(heap_to_run)?;
    Ok(Value::object(obj))
}

fn object_get_prototype_of(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    if v.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let Some(o) = v.as_object() else {
        return Ok(Value::object(cx.vm.intr.object_prototype));
    };
    match cx.vm.heap.get_prototype(o).map_err(heap_to_run)? {
        Some(p) => Ok(Value::object(p)),
        None => Ok(Value::NULL),
    }
}

fn object_set_prototype_of(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let proto_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    if v.is_nullish() {
        return Err(cx.type_error("Object.setPrototypeOf called on null or undefined"));
    }
    let proto = if proto_arg.is_null() {
        None
    } else if let Some(po) = proto_arg.as_object() {
        Some(po)
    } else {
        return Err(cx.type_error("Object prototype may only be an Object or null"));
    };
    if let Some(o) = v.as_object() {
        if !cx.vm.heap.set_prototype(o, proto).map_err(heap_to_run)? {
            return Err(cx.type_error("Cyclic __proto__ value"));
        }
    }
    Ok(v)
}

fn object_has_own(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let k = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = v.as_object() else {
        if v.is_nullish() {
            return Err(cx.type_error("Cannot convert undefined or null to object"));
        }
        return Ok(Value::FALSE);
    };
    let pk = cx.vm.conv_key(k).map_err(|e| cx.vm.exit_to_run(e))?;
    if pk == cx.vm.intr.key_length && cx.vm.heap.object(o).map_err(heap_to_run)?.is_array() {
        return Ok(Value::TRUE);
    }
    let has = cx.vm.heap.get_own(o, pk).map_err(heap_to_run)?.is_some();
    Ok(Value::boolean(has))
}

fn object_proto_has_own_property(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let k = args.first().copied().unwrap_or(Value::UNDEFINED);
    object_has_own(cx, Value::UNDEFINED, &[this, k])
}

fn object_proto_to_string(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let tag = if this.is_undefined() {
        "[object Undefined]"
    } else if this.is_null() {
        "[object Null]"
    } else if this.is_string() {
        "[object String]"
    } else if this.is_number() {
        "[object Number]"
    } else if this.is_bool() {
        "[object Boolean]"
    } else if let Some(o) = this.as_object() {
        let obj = cx.vm.heap.object(o).map_err(heap_to_run)?;
        if obj.is_array() {
            "[object Array]"
        } else if obj.is_callable() {
            "[object Function]"
        } else if matches!(obj.class, ObjectClass::Error) {
            "[object Error]"
        } else {
            "[object Object]"
        }
    } else {
        "[object Object]"
    };
    cx.vm.string(tag)
}

fn object_proto_value_of(
    _cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    Ok(this)
}

fn object_is(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let a = args.first().copied().unwrap_or(Value::UNDEFINED);
    let b = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
        if x.is_nan() && y.is_nan() {
            return Ok(Value::TRUE);
        }
        if x == 0.0 && y == 0.0 {
            return Ok(Value::boolean(x.is_sign_positive() == y.is_sign_positive()));
        }
        return Ok(Value::boolean(x == y));
    }
    let eq = cx.vm.strict_eq(a, b).map_err(|e| cx.vm.exit_to_run(e))?;
    Ok(Value::boolean(eq))
}

fn object_define_property(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let key_v = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let desc_v = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Err(cx.type_error("Object.defineProperty called on non-object"));
    };
    let Some(desc) = desc_v.as_object() else {
        return Err(cx.type_error("Property description must be an object"));
    };
    let pk = cx.vm.conv_key(key_v).map_err(|e| cx.vm.exit_to_run(e))?;
    let read_prop = |cx: &mut NativeCx<'_>, name: &str| -> Result<Option<Value>, RunError> {
        let k = cx.vm.heap.key_from_str(name).map_err(heap_to_run)?;
        match cx.vm.heap.lookup(desc, k).map_err(heap_to_run)? {
            crate::anvil_js::gc::Lookup::Found { .. } => {
                let v = cx
                    .vm
                    .read(cx.host, Value::object(desc), k)
                    .map_err(|e| cx.vm.exit_to_run(e))?;
                Ok(Some(v))
            }
            _ => Ok(None),
        }
    };
    let mut bits = 0u8;
    if let Some(w) = read_prop(cx, "writable")?
        && cx.vm.truthy(w).map_err(|e| cx.vm.exit_to_run(e))?
    {
        bits |= Attrs::WRITABLE;
    }
    if let Some(e) = read_prop(cx, "enumerable")?
        && cx.vm.truthy(e).map_err(|e| cx.vm.exit_to_run(e))?
    {
        bits |= Attrs::ENUMERABLE;
    }
    if let Some(c) = read_prop(cx, "configurable")?
        && cx.vm.truthy(c).map_err(|e| cx.vm.exit_to_run(e))?
    {
        bits |= Attrs::CONFIGURABLE;
    }
    let get = read_prop(cx, "get")?;
    let set = read_prop(cx, "set")?;
    let slot = if get.is_some() || set.is_some() {
        Slot::Accessor {
            get: get.unwrap_or(Value::UNDEFINED),
            set: set.unwrap_or(Value::UNDEFINED),
        }
    } else {
        let val = read_prop(cx, "value")?.unwrap_or(Value::UNDEFINED);
        Slot::Data(val)
    };
    if cx.vm.heap.get_own(o, pk).map_err(heap_to_run)?.is_some() {
        cx.vm.heap.delete_own(o, pk).map_err(heap_to_run)?;
    }
    cx.vm
        .heap
        .define_own(
            o,
            pk,
            Property {
                attrs: Attrs(bits),
                slot,
            },
        )
        .map_err(heap_to_run)?;
    Ok(Value::object(o))
}

fn object_get_own_property_descriptor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let key_v = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Ok(Value::UNDEFINED);
    };
    let pk = cx.vm.conv_key(key_v).map_err(|e| cx.vm.exit_to_run(e))?;
    let Some(prop) = cx.vm.heap.get_own(o, pk).map_err(heap_to_run)? else {
        return Ok(Value::UNDEFINED);
    };
    let d = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    match prop.slot {
        Slot::Data(v) => {
            cx.vm.define_hidden(d, "value", v, Attrs::DEFAULT)?;
            cx.vm.define_hidden(
                d,
                "writable",
                Value::boolean(prop.attrs.writable()),
                Attrs::DEFAULT,
            )?;
        }
        Slot::Accessor { get, set } => {
            cx.vm.define_hidden(d, "get", get, Attrs::DEFAULT)?;
            cx.vm.define_hidden(d, "set", set, Attrs::DEFAULT)?;
        }
    }
    cx.vm.define_hidden(
        d,
        "enumerable",
        Value::boolean(prop.attrs.enumerable()),
        Attrs::DEFAULT,
    )?;
    cx.vm.define_hidden(
        d,
        "configurable",
        Value::boolean(prop.attrs.configurable()),
        Attrs::DEFAULT,
    )?;
    Ok(Value::object(d))
}

fn object_get_own_property_names(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    };
    let keys = cx.vm.heap.own_keys(o).map_err(heap_to_run)?;
    let mut out = Vec::new();
    for k in keys {
        if !k.is_symbol() {
            let s = cx.vm.string(&cx.vm.key_text(k))?;
            out.push(s);
        }
    }
    make_array_from_slice(cx, &out)
}

fn object_freeze(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if let Some(o) = target.as_object() {
        cx.vm.heap.prevent_extensions(o).map_err(heap_to_run)?;
        let keys = cx.vm.heap.own_keys(o).map_err(heap_to_run)?;
        for k in keys {
            if let Some(mut p) = cx.vm.heap.get_own(o, k).map_err(heap_to_run)? {
                p.attrs = Attrs(p.attrs.0 & !(Attrs::WRITABLE | Attrs::CONFIGURABLE));
                cx.vm.heap.define_own(o, k, p).map_err(heap_to_run)?;
            }
        }
    }
    Ok(target)
}

fn object_prevent_extensions(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if let Some(o) = target.as_object() {
        cx.vm.heap.prevent_extensions(o).map_err(heap_to_run)?;
    }
    Ok(target)
}

fn object_is_frozen(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Ok(Value::TRUE);
    };
    let ext = cx.vm.heap.object(o).map_err(heap_to_run)?.extensible;
    Ok(Value::boolean(!ext))
}

fn object_is_extensible(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Ok(Value::FALSE);
    };
    let ext = cx.vm.heap.object(o).map_err(heap_to_run)?.extensible;
    Ok(Value::boolean(ext))
}

/// ES2024 `Object.groupBy(items, callbackFn)`
fn object_group_by(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let items_v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let cb = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, items_v)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    let out = cx.vm.heap.new_object(None).map_err(heap_to_run)?;
    cx.keep(Value::object(out));
    for (idx, item) in items.iter().enumerate() {
        let key_v = cx.call(cb, Value::UNDEFINED, &[*item, Value::number(idx as f64)])?;
        let pk = cx.vm.conv_key(key_v).map_err(|e| cx.vm.exit_to_run(e))?;
        let bucket = match cx.vm.heap.get_own(out, pk).map_err(heap_to_run)? {
            Some(Property {
                slot: Slot::Data(v),
                ..
            }) if v.as_object().is_some() => v.as_object().unwrap(),
            _ => {
                let arr = cx
                    .vm
                    .heap
                    .new_array(Some(cx.vm.intr.array_prototype))
                    .map_err(heap_to_run)?;
                cx.vm
                    .heap
                    .define_own(out, pk, Property::data(Value::object(arr)))
                    .map_err(heap_to_run)?;
                arr
            }
        };
        cx.vm
            .array_push(bucket, *item)
            .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(out))
}

// ---- Function builtins ----------------------------------------------------

fn function_bind(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(fo) = this.as_object() else {
        return Err(cx.type_error("Function.prototype.bind called on non-callable"));
    };
    if !cx.vm.heap.object(fo).map_err(heap_to_run)?.is_callable() {
        return Err(cx.type_error("Function.prototype.bind called on non-callable"));
    }
    let bound_this = args.first().copied().unwrap_or(Value::UNDEFINED);
    let bound_args: Vec<Value> = if args.is_empty() {
        Vec::new()
    } else {
        args[1..].to_vec()
    };
    let orig_name = cx.vm.function_name(fo).unwrap_or_default();
    let orig_len = cx.vm.function_length(fo).unwrap_or(0);
    let length = orig_len.saturating_sub(bound_args.len() as u32);
    let bound = cx
        .vm
        .heap
        .alloc_object(
            Some(cx.vm.intr.function_prototype),
            ObjectClass::Bound(Box::new(BoundFunction {
                target: this,
                this: bound_this,
                args: bound_args,
                name: format!("bound {orig_name}"),
                length,
            })),
        )
        .map_err(heap_to_run)?;
    Ok(Value::object(bound))
}

fn function_to_string(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Function.prototype.toString requires a Function"));
    };
    let name = cx.vm.function_name(o).unwrap_or_default();
    cx.vm
        .string(&format!("function {name}() {{ [native code] }}"))
}
// ---- Array builtins -------------------------------------------------------

fn array_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    if args.len() == 1
        && let Some(n) = args[0].as_number()
    {
        let len = to_uint32(n);
        if f64::from(len) != n {
            return Err(cx.range_error("Invalid array length"));
        }
        let arr = cx
            .vm
            .heap
            .new_array(Some(cx.vm.intr.array_prototype))
            .map_err(heap_to_run)?;
        cx.vm.heap.set_array_length(arr, len).map_err(heap_to_run)?;
        return Ok(Value::object(arr));
    }
    make_array_from_slice(cx, args)
}

fn array_is_array(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let is_arr = v
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_array()));
    Ok(Value::boolean(is_arr))
}

fn array_from(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let src = args.first().copied().unwrap_or(Value::UNDEFINED);
    let map_fn = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    if src.is_nullish() {
        return Err(cx.type_error("Array.from requires an array-like or iterable object"));
    }
    let items = match cx.vm.spread_values(cx.host, src) {
        Ok(v) => v,
        Err(_) => read_array_items(cx, src)?,
    };
    if map_fn.is_undefined() {
        return make_array_from_slice(cx, &items);
    }
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(map_fn);
    cx.keep(this_arg);
    let mut mapped = Vec::with_capacity(items.len());
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(map_fn, this_arg, &[*v, Value::number(i as f64)])?;
        cx.keep(r);
        mapped.push(r);
    }
    make_array_from_slice(cx, &mapped)
}

fn array_of(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    make_array_from_slice(cx, args)
}

fn array_pop(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(Value::UNDEFINED);
    };
    let len = cx.vm.heap.array_length(o).map_err(heap_to_run)?;
    if len == 0 {
        return Ok(Value::UNDEFINED);
    }
    let idx = len - 1;
    let val = cx
        .vm
        .read(cx.host, this, PropertyKey::Index(idx))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    cx.vm.heap.set_array_length(o, idx).map_err(heap_to_run)?;
    Ok(val)
}

fn array_shift(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(Value::UNDEFINED);
    };
    let items = read_array_items(cx, this)?;
    if items.is_empty() {
        return Ok(Value::UNDEFINED);
    }
    let first = items[0];
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.into_iter().skip(1).enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(v))
            .map_err(heap_to_run)?;
    }
    Ok(first)
}

fn array_unshift(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(Value::number(0.0));
    };
    let mut items = args.to_vec();
    let existing = read_array_items(cx, this)?;
    items.extend(existing);
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    Ok(Value::number(items.len() as f64))
}

fn clamp_index(idx: f64, len: usize) -> usize {
    let l = len as f64;
    if idx < 0.0 {
        (l + idx).max(0.0) as usize
    } else {
        idx.min(l) as usize
    }
}

fn array_slice(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let items = read_array_items(cx, this)?;
    let len = items.len();
    let start = match args.first() {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => 0,
    };
    let end = match args.get(1) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => len,
    };
    let slice = if start < end {
        &items[start..end]
    } else {
        &[][..]
    };
    make_array_from_slice(cx, slice)
}

fn array_splice(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return make_array_from_slice(cx, &[]);
    };
    let mut items = read_array_items(cx, this)?;
    let len = items.len();
    if args.is_empty() {
        return make_array_from_slice(cx, &[]);
    }
    let start = clamp_index(to_int_or_inf(cx, args[0])?, len);
    let del_count = if args.len() == 1 {
        len - start
    } else {
        let d = to_int_or_inf(cx, args[1])?.max(0.0) as usize;
        d.min(len - start)
    };
    let insert = if args.len() > 2 { &args[2..] } else { &[][..] };
    let removed: Vec<Value> = items
        .splice(start..start + del_count, insert.iter().copied())
        .collect();
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    make_array_from_slice(cx, &removed)
}

fn array_concat(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut out = if this
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_array()))
    {
        read_array_items(cx, this)?
    } else {
        vec![this]
    };
    for a in args {
        if a.as_object()
            .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_array()))
        {
            out.extend(read_array_items(cx, *a)?);
        } else {
            out.push(*a);
        }
    }
    make_array_from_slice(cx, &out)
}

fn array_join(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let sep = match args.first() {
        Some(v) if !v.is_undefined() => to_str_rust(cx, *v)?,
        _ => ",".to_string(),
    };
    let items = read_array_items(cx, this)?;
    let mut parts = Vec::with_capacity(items.len());
    for v in items {
        if v.is_nullish() {
            parts.push(String::new());
        } else {
            parts.push(to_str_rust(cx, v)?);
        }
    }
    cx.vm.string(&parts.join(&sep))
}

fn array_to_string(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    array_join(cx, this, &[])
}

fn array_index_of(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let needle = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    let start = match args.get(1) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, items.len()),
        _ => 0,
    };
    for (i, item) in items.iter().enumerate().skip(start) {
        if cx
            .vm
            .strict_eq(*item, needle)
            .map_err(|e| cx.vm.exit_to_run(e))?
        {
            return Ok(Value::number(i as f64));
        }
    }
    Ok(Value::number(-1.0))
}

fn array_last_index_of(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let needle = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    if items.is_empty() {
        return Ok(Value::number(-1.0));
    }
    let from = match args.get(1) {
        Some(v) => {
            let n = to_int_or_inf(cx, *v)?;
            if n < 0.0 {
                let k = (items.len() as f64) + n;
                if k < 0.0 {
                    return Ok(Value::number(-1.0));
                }
                k as usize
            } else {
                (n as usize).min(items.len() - 1)
            }
        }
        None => items.len() - 1,
    };
    for i in (0..=from).rev() {
        if cx
            .vm
            .strict_eq(items[i], needle)
            .map_err(|e| cx.vm.exit_to_run(e))?
        {
            return Ok(Value::number(i as f64));
        }
    }
    Ok(Value::number(-1.0))
}

fn array_includes(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let needle = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    let start = match args.get(1) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, items.len()),
        _ => 0,
    };
    for item in items.iter().skip(start) {
        if same_value_zero(cx, *item, needle)? {
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

fn array_for_each(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate() {
        cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
    }
    Ok(Value::UNDEFINED)
}

fn array_map(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    let mut out = Vec::with_capacity(items.len());
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        cx.keep(r);
        out.push(r);
    }
    make_array_from_slice(cx, &out)
}

fn array_filter(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    let mut out = Vec::new();
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            out.push(*v);
        }
    }
    make_array_from_slice(cx, &out)
}

fn array_reduce(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    let (mut acc, start) = if args.len() >= 2 {
        (args[1], 0)
    } else if !items.is_empty() {
        (items[0], 1)
    } else {
        return Err(cx.type_error("Reduce of empty array with no initial value"));
    };
    for (i, v) in items.iter().enumerate().skip(start) {
        cx.keep(acc);
        acc = cx.call(
            cb,
            Value::UNDEFINED,
            &[acc, *v, Value::number(i as f64), this],
        )?;
    }
    Ok(acc)
}

fn array_reduce_right(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    let (mut acc, count) = if args.len() >= 2 {
        (args[1], items.len())
    } else if !items.is_empty() {
        (items[items.len() - 1], items.len() - 1)
    } else {
        return Err(cx.type_error("Reduce of empty array with no initial value"));
    };
    for i in (0..count).rev() {
        cx.keep(acc);
        acc = cx.call(
            cb,
            Value::UNDEFINED,
            &[acc, items[i], Value::number(i as f64), this],
        )?;
    }
    Ok(acc)
}

fn array_some(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

fn array_every(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if !cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::FALSE);
        }
    }
    Ok(Value::TRUE)
}

fn array_find(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(*v);
        }
    }
    Ok(Value::UNDEFINED)
}

fn array_find_index(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::number(i as f64));
        }
    }
    Ok(Value::number(-1.0))
}

fn array_find_last(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate().rev() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(*v);
        }
    }
    Ok(Value::UNDEFINED)
}

fn array_find_last_index(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = read_array_items(cx, this)?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    cx.keep(this_arg);
    for (i, v) in items.iter().enumerate().rev() {
        let r = cx.call(cb, this_arg, &[*v, Value::number(i as f64), this])?;
        if cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::number(i as f64));
        }
    }
    Ok(Value::number(-1.0))
}

fn flatten_into(
    cx: &mut NativeCx<'_>,
    items: &[Value],
    depth: usize,
    out: &mut Vec<Value>,
) -> Result<(), RunError> {
    for v in items {
        if depth > 0
            && v.as_object()
                .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_array()))
        {
            let nested = read_array_items(cx, *v)?;
            flatten_into(cx, &nested, depth - 1, out)?;
        } else {
            out.push(*v);
        }
    }
    Ok(())
}

fn array_flat(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let depth = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0).min(1000.0) as usize,
        _ => 1,
    };
    let items = read_array_items(cx, this)?;
    let mut out = Vec::new();
    flatten_into(cx, &items, depth, &mut out)?;
    make_array_from_slice(cx, &out)
}

fn array_flat_map(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mapped = array_map(cx, this, args)?;
    let items = read_array_items(cx, mapped)?;
    let mut out = Vec::new();
    flatten_into(cx, &items, 1, &mut out)?;
    make_array_from_slice(cx, &out)
}

fn array_at(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let items = read_array_items(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let k = if idx < 0.0 {
        (items.len() as isize) + (idx as isize)
    } else {
        idx as isize
    };
    if k < 0 || (k as usize) >= items.len() {
        Ok(Value::UNDEFINED)
    } else {
        Ok(items[k as usize])
    }
}

fn array_reverse(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(this);
    };
    let mut items = read_array_items(cx, this)?;
    items.reverse();
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    Ok(this)
}

fn sort_vec(
    cx: &mut NativeCx<'_>,
    items: &mut [Value],
    cmp: Option<Value>,
) -> Result<(), RunError> {
    for v in items.iter() {
        cx.keep(*v);
    }
    if let Some(f) = cmp {
        cx.keep(f);
        // Insertion sort so script comparator calls can return Result cleanly.
        for i in 1..items.len() {
            let mut j = i;
            while j > 0 {
                let a = items[j - 1];
                let b = items[j];
                if a.is_undefined() {
                    items.swap(j - 1, j);
                    j -= 1;
                    continue;
                }
                if b.is_undefined() {
                    break;
                }
                let r = cx.call(f, Value::UNDEFINED, &[a, b])?;
                let n = to_num(cx, r)?;
                if n > 0.0 {
                    items.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
    } else {
        let mut keyed: Vec<(bool, String, Value)> = Vec::with_capacity(items.len());
        for v in items.iter() {
            if v.is_undefined() {
                keyed.push((true, String::new(), *v));
            } else {
                keyed.push((false, to_str_rust(cx, *v)?, *v));
            }
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        for (dst, (_, _, v)) in items.iter_mut().zip(keyed) {
            *dst = v;
        }
    }
    Ok(())
}

fn array_sort(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(this);
    };
    let cmp = args.first().copied().filter(|v| !v.is_undefined());
    let mut items = read_array_items(cx, this)?;
    sort_vec(cx, &mut items, cmp)?;
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    Ok(this)
}

fn array_fill(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this
        .as_object()
        .filter(|o| cx.vm.heap.object(*o).is_ok_and(|x| x.is_array()))
    else {
        return Ok(this);
    };
    let fill_val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mut items = read_array_items(cx, this)?;
    let len = items.len();
    let start = match args.get(1) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => 0,
    };
    let end = match args.get(2) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => len,
    };
    for slot in items.iter_mut().take(end.max(start)).skip(start) {
        *slot = fill_val;
    }
    cx.vm.heap.set_array_length(o, 0).map_err(heap_to_run)?;
    for (i, v) in items.iter().enumerate() {
        cx.vm
            .heap
            .define_own(o, PropertyKey::Index(i as u32), Property::data(*v))
            .map_err(heap_to_run)?;
    }
    Ok(this)
}

/// ES2023 `Array.prototype.toReversed()`
fn array_to_reversed(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let mut items = read_array_items(cx, this)?;
    items.reverse();
    make_array_from_slice(cx, &items)
}

/// ES2023 `Array.prototype.toSorted(compareFn)`
fn array_to_sorted(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cmp = args.first().copied().filter(|v| !v.is_undefined());
    let mut items = read_array_items(cx, this)?;
    sort_vec(cx, &mut items, cmp)?;
    make_array_from_slice(cx, &items)
}

/// ES2023 `Array.prototype.toSpliced(start, deleteCount, ...items)`
fn array_to_spliced(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut items = read_array_items(cx, this)?;
    let len = items.len();
    if args.is_empty() {
        return make_array_from_slice(cx, &items);
    }
    let start = clamp_index(to_int_or_inf(cx, args[0])?, len);
    let del_count = if args.len() == 1 {
        len - start
    } else {
        let d = to_int_or_inf(cx, args[1])?.max(0.0) as usize;
        d.min(len - start)
    };
    let insert = if args.len() > 2 { &args[2..] } else { &[][..] };
    let _ = items.splice(start..start + del_count, insert.iter().copied());
    make_array_from_slice(cx, &items)
}

/// ES2023 `Array.prototype.with(index, value)`
fn array_with(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut items = read_array_items(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let val = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let k = if idx < 0.0 {
        (items.len() as isize) + (idx as isize)
    } else {
        idx as isize
    };
    if k < 0 || (k as usize) >= items.len() {
        return Err(cx.range_error("Invalid index for Array.prototype.with"));
    }
    items[k as usize] = val;
    make_array_from_slice(cx, &items)
}

fn make_list_iterator(
    cx: &mut NativeCx<'_>,
    target: Value,
    kind: IterKind,
) -> Result<Value, RunError> {
    let iter = cx
        .vm
        .heap
        .alloc_object(
            Some(cx.vm.intr.object_prototype),
            ObjectClass::ListIterator {
                target,
                index: 0,
                kind,
            },
        )
        .map_err(heap_to_run)?;
    cx.vm.def_method(iter, "next", list_iterator_next)?;
    Ok(Value::object(iter))
}

fn list_iterator_next(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("ListIterator.prototype.next called on incompatible receiver"));
    };
    let (target, index, kind) = match cx.vm.heap.object(o).map_err(heap_to_run)?.class {
        ObjectClass::ListIterator {
            target,
            index,
            kind,
        } => (target, index, kind),
        _ => {
            return Err(
                cx.type_error("ListIterator.prototype.next called on incompatible receiver")
            );
        }
    };
    if target.is_undefined() {
        return make_iter_result(cx, Value::UNDEFINED, true);
    }
    let len_v = cx
        .vm
        .read(cx.host, target, cx.vm.intr.key_length)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let len = to_int_or_inf(cx, len_v)?.max(0.0) as u32;
    if index >= len {
        if let ObjectClass::ListIterator { target: t, .. } =
            cx.vm.heap.class_mut(o).map_err(heap_to_run)?
        {
            *t = Value::UNDEFINED;
        }
        return make_iter_result(cx, Value::UNDEFINED, true);
    }
    if let ObjectClass::ListIterator { index: idx_mut, .. } =
        cx.vm.heap.class_mut(o).map_err(heap_to_run)?
    {
        *idx_mut = index + 1;
    }
    let item = match kind {
        IterKind::Keys => Value::number(f64::from(index)),
        IterKind::Values => cx
            .vm
            .read(cx.host, target, PropertyKey::Index(index))
            .map_err(|e| cx.vm.exit_to_run(e))?,
        IterKind::Entries => {
            let v = cx
                .vm
                .read(cx.host, target, PropertyKey::Index(index))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            make_array_from_slice(cx, &[Value::number(f64::from(index)), v])?
        }
    };
    make_iter_result(cx, item, false)
}

fn make_iter_result(cx: &mut NativeCx<'_>, value: Value, done: bool) -> Result<Value, RunError> {
    let res = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.vm.define_hidden(res, "value", value, Attrs::DEFAULT)?;
    cx.vm
        .define_hidden(res, "done", Value::boolean(done), Attrs::DEFAULT)?;
    Ok(Value::object(res))
}

fn array_keys(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_list_iterator(cx, this, IterKind::Keys)
}

fn array_values(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_list_iterator(cx, this, IterKind::Values)
}

fn array_entries(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_list_iterator(cx, this, IterKind::Entries)
}

// ---- String builtins ------------------------------------------------------

fn string_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(v) = args.first() else {
        return cx.vm.string("");
    };
    if let Some(sym) = v.as_symbol() {
        let d = cx
            .vm
            .heap
            .symbol_description(sym)
            .map_err(heap_to_run)?
            .and_then(|s| cx.vm.heap.string_to_rust(s).ok())
            .unwrap_or_default();
        return cx.vm.string(&format!("Symbol({d})"));
    }
    let s = cx.vm.conv_string(*v).map_err(|e| cx.vm.exit_to_run(e))?;
    Ok(Value::string(s))
}

fn string_from_char_code(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let mut units = Vec::with_capacity(args.len());
    for a in args {
        let n = to_num(cx, *a)?;
        units.push(to_uint32(n) as u16);
    }
    let s = cx.vm.heap.alloc_string_units(units).map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_from_code_point(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let mut out = String::new();
    for a in args {
        let n = to_num(cx, *a)?;
        if !n.is_finite() || n.fract() != 0.0 || !(0.0..=0x10FFFF as f64).contains(&n) {
            return Err(cx.range_error("Invalid code point"));
        }
        let Some(ch) = char::from_u32(n as u32) else {
            return Err(cx.range_error("Invalid code point"));
        };
        out.push(ch);
    }
    cx.vm.string(&out)
}

fn recv_string_units(cx: &mut NativeCx<'_>, this: Value) -> Result<Vec<u16>, RunError> {
    if this.is_nullish() {
        return Err(cx.type_error("String.prototype method called on null or undefined"));
    }
    let s = cx.vm.conv_string(this).map_err(|e| cx.vm.exit_to_run(e))?;
    Ok(cx.vm.heap.string_units(s).map_err(heap_to_run)?.to_vec())
}

fn string_char_at(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    if idx < 0.0 || (idx as usize) >= units.len() {
        return cx.vm.string("");
    }
    let s = cx
        .vm
        .heap
        .alloc_string_units(vec![units[idx as usize]])
        .map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_char_code_at(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    if idx < 0.0 || (idx as usize) >= units.len() {
        return Ok(Value::number(f64::NAN));
    }
    Ok(Value::number(f64::from(units[idx as usize])))
}

fn string_code_point_at(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    if idx < 0.0 || (idx as usize) >= units.len() {
        return Ok(Value::UNDEFINED);
    }
    let i = idx as usize;
    let first = u32::from(units[i]);
    if (0xD800..=0xDBFF).contains(&first)
        && i + 1 < units.len()
        && (0xDC00..=0xDFFF).contains(&u32::from(units[i + 1]))
    {
        let second = u32::from(units[i + 1]);
        let cp = ((first - 0xD800) << 10) + (second - 0xDC00) + 0x10000;
        return Ok(Value::number(f64::from(cp)));
    }
    Ok(Value::number(f64::from(first)))
}

fn string_at(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let idx = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    let k = if idx < 0.0 {
        (units.len() as isize) + (idx as isize)
    } else {
        idx as isize
    };
    if k < 0 || (k as usize) >= units.len() {
        return Ok(Value::UNDEFINED);
    }
    let s = cx
        .vm
        .heap
        .alloc_string_units(vec![units[k as usize]])
        .map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_slice(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let len = units.len();
    let start = match args.first() {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => 0,
    };
    let end = match args.get(1) {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => len,
    };
    let sub = if start < end {
        units[start..end].to_vec()
    } else {
        Vec::new()
    };
    let s = cx.vm.heap.alloc_string_units(sub).map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_substring(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let len = units.len();
    let a = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0).min(len as f64) as usize,
        _ => 0,
    };
    let b = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0).min(len as f64) as usize,
        _ => len,
    };
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let s = cx
        .vm
        .heap
        .alloc_string_units(units[start..end].to_vec())
        .map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_substr(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, this)?;
    let len = units.len();
    let start = match args.first() {
        Some(v) if !v.is_undefined() => clamp_index(to_int_or_inf(cx, *v)?, len),
        _ => 0,
    };
    let count = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0) as usize,
        _ => len - start,
    };
    let end = (start + count).min(len);
    let s = cx
        .vm
        .heap
        .alloc_string_units(units[start..end].to_vec())
        .map_err(heap_to_run)?;
    Ok(Value::string(s))
}

fn string_concat(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut out = to_str_rust(cx, this)?;
    for a in args {
        out.push_str(&to_str_rust(cx, *a)?);
    }
    cx.vm.string(&out)
}

fn string_index_of(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let hay = to_str_rust(cx, this)?;
    let needle = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let pos = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0) as usize,
        _ => 0,
    };
    if pos > hay.len() {
        return Ok(if needle.is_empty() {
            Value::number(hay.len() as f64)
        } else {
            Value::number(-1.0)
        });
    }
    match hay.get(pos..).and_then(|s| s.find(&needle)) {
        Some(off) => Ok(Value::number((pos + off) as f64)),
        None => Ok(Value::number(-1.0)),
    }
}

fn string_last_index_of(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let hay = to_str_rust(cx, this)?;
    let needle = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    match hay.rfind(&needle) {
        Some(idx) => Ok(Value::number(idx as f64)),
        None => Ok(Value::number(-1.0)),
    }
}

fn string_includes(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let hay = to_str_rust(cx, this)?;
    let needle = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let pos = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0) as usize,
        _ => 0,
    };
    let ok = hay.get(pos..).is_some_and(|s| s.contains(&needle));
    Ok(Value::boolean(ok))
}

fn string_starts_with(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let hay = to_str_rust(cx, this)?;
    let needle = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let pos = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?.max(0.0) as usize,
        _ => 0,
    };
    let ok = hay.get(pos..).is_some_and(|s| s.starts_with(&needle));
    Ok(Value::boolean(ok))
}

fn string_ends_with(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let hay = to_str_rust(cx, this)?;
    let needle = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let end = match args.get(1) {
        Some(v) if !v.is_undefined() => {
            to_int_or_inf(cx, *v)?.max(0.0).min(hay.len() as f64) as usize
        }
        _ => hay.len(),
    };
    let ok = hay.get(..end).is_some_and(|s| s.ends_with(&needle));
    Ok(Value::boolean(ok))
}

fn string_trim(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    cx.vm.string(s.trim())
}

fn string_trim_start(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    cx.vm.string(s.trim_start())
}

fn string_trim_end(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    cx.vm.string(s.trim_end())
}

fn string_pad_start(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let max_len =
        to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?.max(0.0) as usize;
    if max_len <= s.len() {
        return cx.vm.string(&s);
    }
    let fill = match args.get(1) {
        Some(v) if !v.is_undefined() => to_str_rust(cx, *v)?,
        _ => " ".to_string(),
    };
    if fill.is_empty() {
        return cx.vm.string(&s);
    }
    let needed = max_len - s.len();
    let pad: String = fill.chars().cycle().take(needed).collect();
    cx.vm.string(&format!("{pad}{s}"))
}

fn string_pad_end(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let max_len =
        to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?.max(0.0) as usize;
    if max_len <= s.len() {
        return cx.vm.string(&s);
    }
    let fill = match args.get(1) {
        Some(v) if !v.is_undefined() => to_str_rust(cx, *v)?,
        _ => " ".to_string(),
    };
    if fill.is_empty() {
        return cx.vm.string(&s);
    }
    let needed = max_len - s.len();
    let pad: String = fill.chars().cycle().take(needed).collect();
    cx.vm.string(&format!("{s}{pad}"))
}

fn string_repeat(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let count = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    if count < 0.0 || count.is_infinite() {
        return Err(cx.range_error("Invalid count value"));
    }
    let n = count as usize;
    if s.len().saturating_mul(n) > cx.vm.heap.limits().max_string_length {
        return Err(cx.range_error("Invalid string length"));
    }
    cx.vm.string(&s.repeat(n))
}

fn string_split(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let limit = match args.get(1) {
        Some(v) if !v.is_undefined() => to_uint32(to_num(cx, *v)?) as usize,
        _ => usize::MAX,
    };
    if limit == 0 {
        return make_array_from_slice(cx, &[]);
    }
    let Some(sep_v) = args.first() else {
        let val = cx.vm.string(&s)?;
        return make_array_from_slice(cx, &[val]);
    };
    if sep_v.is_undefined() {
        let val = cx.vm.string(&s)?;
        return make_array_from_slice(cx, &[val]);
    }
    if let Some((pattern, flags)) = extract_regexp_pair(cx, *sep_v)? {
        let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
            .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
        let chars: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut last_end = 0usize;
        let mut search_pos = 0usize;
        while search_pos <= chars.len() {
            let Some(m) = compiled.exec_at(&chars, search_pos) else {
                break;
            };
            if m.end_char == last_end && m.start_char == m.end_char {
                search_pos += 1;
                continue;
            }
            let piece: String = chars[last_end..m.start_char].iter().collect();
            out.push(cx.vm.string(&piece)?);
            if out.len() >= limit {
                return make_array_from_slice(cx, &out);
            }
            for g in &m.groups {
                let gv = match g {
                    Some((gs, ge)) => {
                        let gs_str: String = chars[*gs..*ge].iter().collect();
                        cx.vm.string(&gs_str)?
                    }
                    None => Value::UNDEFINED,
                };
                out.push(gv);
                if out.len() >= limit {
                    return make_array_from_slice(cx, &out);
                }
            }
            last_end = m.end_char;
            search_pos = if m.end_char == m.start_char {
                m.end_char + 1
            } else {
                m.end_char
            };
        }
        let tail: String = chars[last_end..].iter().collect();
        out.push(cx.vm.string(&tail)?);
        if out.len() > limit {
            out.truncate(limit);
        }
        return make_array_from_slice(cx, &out);
    }
    let sep = to_str_rust(cx, *sep_v)?;
    let parts: Vec<String> = if sep.is_empty() {
        s.chars().take(limit).map(|c| c.to_string()).collect()
    } else {
        s.split(&sep).take(limit).map(|p| p.to_string()).collect()
    };
    let mut vals = Vec::with_capacity(parts.len());
    for p in parts {
        vals.push(cx.vm.string(&p)?);
    }
    make_array_from_slice(cx, &vals)
}

fn string_replace(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    string_replace_impl(cx, this, args, false)
}

fn string_replace_all(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    string_replace_impl(cx, this, args, true)
}

fn string_to_lower_case(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    cx.vm.string(&s.to_lowercase())
}

fn string_to_upper_case(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    cx.vm.string(&s.to_uppercase())
}

fn string_to_string(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    if let Some(s) = this.as_string() {
        return Ok(Value::string(s));
    }
    if let Some(o) = this.as_object()
        && let ObjectClass::Primitive(p) = cx.vm.heap.object(o).map_err(heap_to_run)?.class
        && p.is_string()
    {
        return Ok(p);
    }
    Err(cx.type_error("String.prototype.toString requires a String"))
}
impl Vm {
    fn install_number_and_boolean_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let np = self.intr.number_prototype;
        let n_ctor = self.register_native("Number", number_ctor)?;
        let nco = n_ctor.as_object().unwrap();
        self.define_hidden(nco, "prototype", Value::object(np), FIXED)?;
        self.define_hidden(np, "constructor", n_ctor, HIDDEN)?;
        for (name, val) in [
            ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
            ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
            ("EPSILON", f64::EPSILON),
            ("MAX_VALUE", f64::MAX),
            ("MIN_VALUE", f64::MIN_POSITIVE),
            ("NaN", f64::NAN),
            ("POSITIVE_INFINITY", f64::INFINITY),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ] {
            self.define_hidden(nco, name, Value::number(val), FIXED)?;
        }
        for (name, f) in [
            ("isNaN", number_is_nan as NativeFn),
            ("isFinite", number_is_finite),
            ("isInteger", number_is_integer),
            ("isSafeInteger", number_is_safe_integer),
            ("parseInt", global_parse_int),
            ("parseFloat", global_parse_float),
        ] {
            self.def_method(nco, name, f)?;
        }
        self.def_method(np, "toString", number_to_string_method)?;
        self.def_method(np, "toFixed", number_to_fixed)?;
        self.def_method(np, "valueOf", number_value_of)?;
        self.define_hidden(g, "Number", n_ctor, HIDDEN)?;

        let bp = self.intr.boolean_prototype;
        let b_ctor = self.register_native("Boolean", boolean_ctor)?;
        let bco = b_ctor.as_object().unwrap();
        self.define_hidden(bco, "prototype", Value::object(bp), FIXED)?;
        self.define_hidden(bp, "constructor", b_ctor, HIDDEN)?;
        self.def_method(bp, "toString", boolean_to_string_method)?;
        self.def_method(bp, "valueOf", boolean_value_of)?;
        self.define_hidden(g, "Boolean", b_ctor, HIDDEN)?;
        Ok(())
    }

    fn install_symbol_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let syp = self.intr.symbol_prototype;
        let ctor = self.register_native("Symbol", symbol_ctor)?;
        let co = ctor.as_object().unwrap();
        self.define_hidden(co, "prototype", Value::object(syp), FIXED)?;
        self.define_hidden(syp, "constructor", ctor, HIDDEN)?;
        for well_known in [
            "iterator",
            "asyncIterator",
            "toStringTag",
            "toPrimitive",
            "hasInstance",
            "species",
            "isConcatSpreadable",
            "match",
            "replace",
            "search",
            "split",
        ] {
            let desc = self
                .heap
                .alloc_string(&format!("Symbol.{well_known}"))
                .map_err(heap_to_run)?;
            let sym = self.heap.alloc_symbol(Some(desc)).map_err(heap_to_run)?;
            self.define_hidden(co, well_known, Value::symbol(sym), FIXED)?;
        }
        let reg_obj = self.heap.new_object(None).map_err(heap_to_run)?;
        self.define_hidden(co, "__symbol_registry", Value::object(reg_obj), HIDDEN)?;
        self.def_method(co, "for", symbol_for_static)?;
        self.def_method(co, "keyFor", symbol_key_for_static)?;
        self.def_method(syp, "toString", symbol_to_string_method)?;
        self.define_accessor(syp, "description", Some(symbol_description_getter), None)?;
        self.define_hidden(g, "Symbol", ctor, HIDDEN)?;
        Ok(())
    }

    fn install_math_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let math = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        for (name, val) in [
            ("PI", std::f64::consts::PI),
            ("E", std::f64::consts::E),
            ("LN2", std::f64::consts::LN_2),
            ("LN10", std::f64::consts::LN_10),
            ("LOG2E", std::f64::consts::LOG2_E),
            ("LOG10E", std::f64::consts::LOG10_E),
            ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
            ("SQRT2", std::f64::consts::SQRT_2),
        ] {
            self.define_hidden(math, name, Value::number(val), FIXED)?;
        }
        for (name, f) in [
            ("abs", math_abs as NativeFn),
            ("floor", math_floor),
            ("ceil", math_ceil),
            ("round", math_round),
            ("trunc", math_trunc),
            ("sign", math_sign),
            ("min", math_min),
            ("max", math_max),
            ("pow", math_pow),
            ("sqrt", math_sqrt),
            ("cbrt", math_cbrt),
            ("hypot", math_hypot),
            ("sin", math_sin),
            ("cos", math_cos),
            ("tan", math_tan),
            ("asin", math_asin),
            ("acos", math_acos),
            ("atan", math_atan),
            ("atan2", math_atan2),
            ("log", math_log),
            ("log2", math_log2),
            ("log10", math_log10),
            ("exp", math_exp),
            ("clz32", math_clz32),
            ("imul", math_imul),
            ("fround", math_fround),
            ("random", math_random),
        ] {
            self.def_method(math, name, f)?;
        }
        self.define_hidden(g, "Math", Value::object(math), HIDDEN)?;
        Ok(())
    }

    fn install_json_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let json = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.def_method(json, "parse", json_parse)?;
        self.def_method(json, "stringify", json_stringify)?;
        self.define_hidden(g, "JSON", Value::object(json), HIDDEN)?;
        Ok(())
    }
}

// ---- Number, Boolean, Symbol & Global numeric helpers ---------------------

fn number_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(v) = args.first() else {
        return Ok(Value::number(0.0));
    };
    Ok(Value::number(to_num(cx, *v)?))
}

fn number_is_nan(_cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let is = args
        .first()
        .and_then(|v| v.as_number())
        .is_some_and(|n| n.is_nan());
    Ok(Value::boolean(is))
}

fn number_is_finite(
    _cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let is = args
        .first()
        .and_then(|v| v.as_number())
        .is_some_and(|n| n.is_finite());
    Ok(Value::boolean(is))
}

fn number_is_integer(
    _cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let is = args
        .first()
        .and_then(|v| v.as_number())
        .is_some_and(|n| n.is_finite() && n.trunc() == n);
    Ok(Value::boolean(is))
}

fn number_is_safe_integer(
    _cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let is = args
        .first()
        .and_then(|v| v.as_number())
        .is_some_and(|n| n.is_finite() && n.trunc() == n && n.abs() <= 9_007_199_254_740_991.0);
    Ok(Value::boolean(is))
}

fn recv_number(cx: &mut NativeCx<'_>, this: Value) -> Result<f64, RunError> {
    if let Some(n) = this.as_number() {
        return Ok(n);
    }
    if let Some(bi) = this.as_bigint() {
        return Ok(bi as f64);
    }
    if let Some(o) = this.as_object()
        && let ObjectClass::Primitive(p) = cx.vm.heap.object(o).map_err(heap_to_run)?.class
    {
        if let Some(n) = p.as_number() {
            return Ok(n);
        }
        if let Some(bi) = p.as_bigint() {
            return Ok(bi as f64);
        }
    }
    Err(cx.type_error("Number.prototype method requires a Number"))
}

fn number_to_string_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let n = recv_number(cx, this)?;
    let radix = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)? as i32,
        _ => 10,
    };
    if !(2..=36).contains(&radix) {
        return Err(cx.range_error("toString() radix argument must be between 2 and 36"));
    }
    if radix == 10 || !n.is_finite() {
        return cx.vm.string(&number_to_string(n));
    }
    let neg = n < 0.0;
    let mut val = n.abs().trunc() as u128;
    if val == 0 {
        return cx.vm.string("0");
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut chars = Vec::new();
    while val > 0 {
        chars.push(digits[(val % (radix as u128)) as usize] as char);
        val /= radix as u128;
    }
    if neg {
        chars.push('-');
    }
    let out: String = chars.into_iter().rev().collect();
    cx.vm.string(&out)
}

fn number_to_fixed(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = recv_number(cx, this)?;
    let digits = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    if !(0.0..=100.0).contains(&digits) {
        return Err(cx.range_error("toFixed() digits argument must be between 0 and 100"));
    }
    if !n.is_finite() {
        return cx.vm.string(&number_to_string(n));
    }
    cx.vm.string(&format!("{:.*}", digits as usize, n))
}

fn number_value_of(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(recv_number(cx, this)?))
}

fn boolean_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let b = cx.vm.truthy(v).map_err(|e| cx.vm.exit_to_run(e))?;
    Ok(Value::boolean(b))
}

fn recv_bool(cx: &mut NativeCx<'_>, this: Value) -> Result<bool, RunError> {
    if let Some(b) = this.as_bool() {
        return Ok(b);
    }
    if let Some(o) = this.as_object()
        && let ObjectClass::Primitive(p) = cx.vm.heap.object(o).map_err(heap_to_run)?.class
        && let Some(b) = p.as_bool()
    {
        return Ok(b);
    }
    Err(cx.type_error("Boolean.prototype method requires a Boolean"))
}

fn boolean_to_string_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let b = recv_bool(cx, this)?;
    cx.vm.string(if b { "true" } else { "false" })
}

fn boolean_value_of(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    Ok(Value::boolean(recv_bool(cx, this)?))
}

fn symbol_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let desc = match args.first() {
        Some(v) if !v.is_undefined() => {
            Some(cx.vm.conv_string(*v).map_err(|e| cx.vm.exit_to_run(e))?)
        }
        _ => None,
    };
    let sym = cx.vm.heap.alloc_symbol(desc).map_err(heap_to_run)?;
    Ok(Value::symbol(sym))
}

fn symbol_to_string_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let sym = if let Some(s) = this.as_symbol() {
        s
    } else if let Some(o) = this.as_object()
        && let ObjectClass::Primitive(p) = cx.vm.heap.object(o).map_err(heap_to_run)?.class
        && let Some(s) = p.as_symbol()
    {
        s
    } else {
        return Err(cx.type_error("Symbol.prototype.toString requires a Symbol"));
    };
    let d = cx
        .vm
        .heap
        .symbol_description(sym)
        .map_err(heap_to_run)?
        .and_then(|s| cx.vm.heap.string_to_rust(s).ok())
        .unwrap_or_default();
    cx.vm.string(&format!("Symbol({d})"))
}

fn global_is_nan(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    Ok(Value::boolean(to_num(cx, v)?.is_nan()))
}

fn global_is_finite(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    Ok(Value::boolean(to_num(cx, v)?.is_finite()))
}

fn global_parse_float(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let trimmed = s.trim_start();
    if trimmed.starts_with("Infinity") || trimmed.starts_with("+Infinity") {
        return Ok(Value::number(f64::INFINITY));
    }
    if trimmed.starts_with("-Infinity") {
        return Ok(Value::number(f64::NEG_INFINITY));
    }
    let mut end = 0;
    let bytes = trimmed.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        saw_digit = true;
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            saw_digit = true;
            end += 1;
        }
    }
    if !saw_digit {
        return Ok(Value::number(f64::NAN));
    }
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exp_end = end + 1;
        if exp_end < bytes.len() && (bytes[exp_end] == b'+' || bytes[exp_end] == b'-') {
            exp_end += 1;
        }
        let mut exp_digits = false;
        while exp_end < bytes.len() && bytes[exp_end].is_ascii_digit() {
            exp_digits = true;
            exp_end += 1;
        }
        if exp_digits {
            end = exp_end;
        }
    }
    let parsed = trimmed[..end].parse::<f64>().unwrap_or(f64::NAN);
    Ok(Value::number(parsed))
}

fn global_parse_int(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let mut trimmed = s.trim_start();
    let mut sign = 1.0f64;
    if let Some(rest) = trimmed.strip_prefix('-') {
        sign = -1.0;
        trimmed = rest;
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        trimmed = rest;
    }
    let mut radix = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int32(to_num(cx, *v)?),
        _ => 0,
    };
    if radix == 0 {
        if let Some(rest) = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
        {
            radix = 16;
            trimmed = rest;
        } else {
            radix = 10;
        }
    } else if radix == 16
        && let Some(rest) = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
    {
        trimmed = rest;
    }
    if !(2..=36).contains(&radix) {
        return Ok(Value::number(f64::NAN));
    }
    let mut acc = 0.0f64;
    let mut count = 0usize;
    for ch in trimmed.chars() {
        let Some(d) = ch.to_digit(radix as u32) else {
            break;
        };
        acc = acc * (radix as f64) + f64::from(d);
        count += 1;
    }
    if count == 0 {
        Ok(Value::number(f64::NAN))
    } else {
        Ok(Value::number(sign * acc))
    }
}

// ---- Math builtins --------------------------------------------------------

fn math_abs(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.abs()))
}

fn math_floor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.floor()))
}

fn math_ceil(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.ceil()))
}

fn math_round(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    if !n.is_finite() || n == 0.0 {
        return Ok(Value::number(n));
    }
    Ok(Value::number((n + 0.5).floor()))
}

fn math_trunc(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.trunc()))
}

fn math_sign(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let out = if n.is_nan() || n == 0.0 {
        n
    } else if n > 0.0 {
        1.0
    } else {
        -1.0
    };
    Ok(Value::number(out))
}

fn math_min(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut best = f64::INFINITY;
    for a in args {
        let n = to_num(cx, *a)?;
        if n.is_nan() {
            return Ok(Value::number(f64::NAN));
        }
        if n < best || (n == 0.0 && best == 0.0 && n.is_sign_negative()) {
            best = n;
        }
    }
    Ok(Value::number(best))
}

fn math_max(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut best = f64::NEG_INFINITY;
    for a in args {
        let n = to_num(cx, *a)?;
        if n.is_nan() {
            return Ok(Value::number(f64::NAN));
        }
        if n > best || (n == 0.0 && best == 0.0 && n.is_sign_positive()) {
            best = n;
        }
    }
    Ok(Value::number(best))
}

fn math_pow(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let x = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let y = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(super::interp::js_pow(x, y)))
}

fn math_sqrt(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.sqrt()))
}

fn math_cbrt(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(n.cbrt()))
}

fn math_hypot(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut sum = 0.0f64;
    let mut saw_inf = false;
    let mut saw_nan = false;
    for a in args {
        let n = to_num(cx, *a)?;
        if n.is_infinite() {
            saw_inf = true;
        } else if n.is_nan() {
            saw_nan = true;
        } else {
            sum += n * n;
        }
    }
    if saw_inf {
        Ok(Value::number(f64::INFINITY))
    } else if saw_nan {
        Ok(Value::number(f64::NAN))
    } else {
        Ok(Value::number(sum.sqrt()))
    }
}

fn math_sin(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.sin(),
    ))
}

fn math_cos(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.cos(),
    ))
}

fn math_tan(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.tan(),
    ))
}

fn math_asin(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.asin(),
    ))
}

fn math_acos(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.acos(),
    ))
}

fn math_atan(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.atan(),
    ))
}

fn math_atan2(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let y = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let x = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(y.atan2(x)))
}

fn math_log(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.ln(),
    ))
}

fn math_log2(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.log2(),
    ))
}

fn math_log10(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.log10(),
    ))
}

fn math_exp(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(
        to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?.exp(),
    ))
}

fn math_clz32(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_uint32(to_num(
        cx,
        args.first().copied().unwrap_or(Value::UNDEFINED),
    )?);
    Ok(Value::number(f64::from(n.leading_zeros())))
}

fn math_imul(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let a = to_int32(to_num(
        cx,
        args.first().copied().unwrap_or(Value::UNDEFINED),
    )?);
    let b = to_int32(to_num(
        cx,
        args.get(1).copied().unwrap_or(Value::UNDEFINED),
    )?);
    Ok(Value::number(f64::from(a.wrapping_mul(b))))
}

fn math_fround(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    Ok(Value::number(f64::from(n as f32)))
}

fn math_random(cx: &mut NativeCx<'_>, _this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let step_seed = cx
        .vm
        .total_steps
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0xBF58_476D_1CE4_E5B9);
    let frac = ((step_seed >> 11) as f64) / ((1u64 << 53) as f64);
    Ok(Value::number(frac))
}

// ---- JSON builtins --------------------------------------------------------

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r' | b'\n')
        {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self, cx: &mut NativeCx<'_>) -> Result<Value, String> {
        self.skip_ws();
        let Some(&b) = self.bytes.get(self.pos) else {
            return Err("Unexpected end of JSON input".to_string());
        };
        match b {
            b'n' => {
                if self.bytes[self.pos..].starts_with(b"null") {
                    self.pos += 4;
                    Ok(Value::NULL)
                } else {
                    Err("Unexpected token in JSON".to_string())
                }
            }
            b't' => {
                if self.bytes[self.pos..].starts_with(b"true") {
                    self.pos += 4;
                    Ok(Value::TRUE)
                } else {
                    Err("Unexpected token in JSON".to_string())
                }
            }
            b'f' => {
                if self.bytes[self.pos..].starts_with(b"false") {
                    self.pos += 5;
                    Ok(Value::FALSE)
                } else {
                    Err("Unexpected token in JSON".to_string())
                }
            }
            b'"' => {
                let s = self.parse_string()?;
                cx.vm.string(&s).map_err(|e| e.message().to_string())
            }
            b'[' => self.parse_array(cx),
            b'{' => self.parse_object(cx),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err("Unexpected token in JSON".to_string()),
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err("Expected string in JSON".to_string());
        }
        self.pos += 1;
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if b == b'"' {
                return Ok(out);
            }
            if b < 0x20 {
                return Err("Unescaped control character in JSON string".to_string());
            }
            if b == b'\\' {
                let Some(&esc) = self.bytes.get(self.pos) else {
                    return Err("Unexpected end of JSON string".to_string());
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if self.pos + 4 > self.bytes.len() {
                            return Err("Invalid unicode escape in JSON".to_string());
                        }
                        let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                            .map_err(|_| "Invalid unicode escape in JSON".to_string())?;
                        let code = u32::from_str_radix(hex, 16)
                            .map_err(|_| "Invalid unicode escape in JSON".to_string())?;
                        self.pos += 4;
                        let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                        out.push(ch);
                    }
                    _ => return Err("Invalid escape in JSON string".to_string()),
                }
            } else {
                // Re-decode UTF-8 scalar starting at `self.pos - 1`.
                let rest = std::str::from_utf8(&self.bytes[self.pos - 1..])
                    .map_err(|_| "Invalid UTF-8 in JSON".to_string())?;
                let ch = rest.chars().next().unwrap();
                self.pos += ch.len_utf8() - 1;
                out.push(ch);
            }
        }
        Err("Unterminated string in JSON".to_string())
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() || !self.bytes[self.pos].is_ascii_digit() {
            return Err("Invalid number in JSON".to_string());
        }
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if self.pos >= self.bytes.len() || !self.bytes[self.pos].is_ascii_digit() {
                return Err("Invalid number in JSON".to_string());
            }
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() || !self.bytes[self.pos].is_ascii_digit() {
                return Err("Invalid exponent in JSON".to_string());
            }
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "Invalid number in JSON".to_string())?;
        let n = text
            .parse::<f64>()
            .map_err(|_| "Invalid number in JSON".to_string())?;
        Ok(Value::number(n))
    }

    fn parse_array(&mut self, cx: &mut NativeCx<'_>) -> Result<Value, String> {
        self.pos += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return make_array_from_slice(cx, &items).map_err(|e| e.message().to_string());
        }
        loop {
            let v = self.parse_value(cx)?;
            cx.keep(v);
            items.push(v);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err("Expected ',' or ']' in JSON array".to_string()),
            }
        }
        make_array_from_slice(cx, &items).map_err(|e| e.message().to_string())
    }

    fn parse_object(&mut self, cx: &mut NativeCx<'_>) -> Result<Value, String> {
        self.pos += 1;
        self.skip_ws();
        let obj = cx
            .vm
            .heap
            .new_object(Some(cx.vm.intr.object_prototype))
            .map_err(|e| heap_to_run(e).message().to_string())?;
        cx.keep(Value::object(obj));
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Value::object(obj));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err("Expected ':' in JSON object".to_string());
            }
            self.pos += 1;
            let val = self.parse_value(cx)?;
            cx.keep(val);
            let pk = cx
                .vm
                .heap
                .key_from_str(&key)
                .map_err(|e| heap_to_run(e).message().to_string())?;
            cx.vm
                .heap
                .define_own(obj, pk, Property::data(val))
                .map_err(|e| heap_to_run(e).message().to_string())?;
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err("Expected ',' or '}' in JSON object".to_string()),
            }
        }
        Ok(Value::object(obj))
    }
}

fn json_parse(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let text = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let mut parser = JsonParser::new(&text);
    let val = match parser.parse_value(cx) {
        Ok(v) => v,
        Err(msg) => return Err(cx.vm.make_thrown(ErrorKind::Syntax, &msg)),
    };
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(cx
            .vm
            .make_thrown(ErrorKind::Syntax, "Unexpected trailing token in JSON"));
    }
    Ok(val)
}

fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn stringify_val(
    cx: &mut NativeCx<'_>,
    v: Value,
    seen: &mut Vec<ObjectRef>,
) -> Result<Option<String>, RunError> {
    if v.is_undefined() || v.is_symbol() {
        return Ok(None);
    }
    if v.is_null() {
        return Ok(Some("null".to_string()));
    }
    if let Some(b) = v.as_bool() {
        return Ok(Some(if b { "true" } else { "false" }.to_string()));
    }
    if let Some(n) = v.as_number() {
        if !n.is_finite() {
            return Ok(Some("null".to_string()));
        }
        return Ok(Some(number_to_string(n)));
    }
    if let Some(s) = v.as_string() {
        let text = cx.vm.heap.string_to_rust(s).map_err(heap_to_run)?;
        return Ok(Some(escape_json_str(&text)));
    }
    let Some(o) = v.as_object() else {
        return Ok(None);
    };
    let obj = cx.vm.heap.object(o).map_err(heap_to_run)?.clone();
    if obj.is_callable() {
        return Ok(None);
    }
    if let ObjectClass::Primitive(p) = obj.class {
        return stringify_val(cx, p, seen);
    }
    if seen.contains(&o) {
        return Err(cx.type_error("Converting circular structure to JSON"));
    }
    seen.push(o);
    let out = if obj.is_array() {
        let items = read_array_items(cx, v)?;
        let mut parts = Vec::with_capacity(items.len());
        for item in items {
            let s = stringify_val(cx, item, seen)?.unwrap_or_else(|| "null".to_string());
            parts.push(s);
        }
        format!("[{}]", parts.join(","))
    } else {
        let pairs = cx
            .vm
            .own_enumerable(cx.host, v)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        let mut parts = Vec::new();
        for (k, val) in pairs {
            if k.is_symbol() {
                continue;
            }
            if let Some(sv) = stringify_val(cx, val, seen)? {
                let key_str = escape_json_str(&cx.vm.key_text(k));
                parts.push(format!("{key_str}:{sv}"));
            }
        }
        format!("{{{}}}", parts.join(","))
    };
    seen.pop();
    Ok(Some(out))
}

fn json_stringify(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mut seen = Vec::new();
    match stringify_val(cx, v, &mut seen)? {
        Some(s) => cx.vm.string(&s),
        None => Ok(Value::UNDEFINED),
    }
}
impl Vm {
    fn install_collection_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let map_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let map_ctor = self.register_native("Map", map_ctor_fn)?;
        let mco = map_ctor.as_object().unwrap();
        self.define_hidden(mco, "prototype", Value::object(map_proto), FIXED)?;
        self.define_hidden(map_proto, "constructor", map_ctor, HIDDEN)?;
        self.def_method(mco, "groupBy", map_group_by)?;
        for (name, f) in [
            ("get", map_get as NativeFn),
            ("set", map_set),
            ("has", map_has),
            ("delete", map_delete),
            ("clear", map_clear),
            ("forEach", map_for_each),
            ("keys", map_keys),
            ("values", map_values),
            ("entries", map_entries),
        ] {
            self.def_method(map_proto, name, f)?;
        }
        let size_get = self.register_native("get size", collection_size_getter)?;
        let key_size = self.heap.key_from_str("size").map_err(heap_to_run)?;
        self.heap
            .define_own(
                map_proto,
                key_size,
                Property {
                    attrs: Attrs(Attrs::CONFIGURABLE),
                    slot: Slot::Accessor {
                        get: size_get,
                        set: Value::UNDEFINED,
                    },
                },
            )
            .map_err(heap_to_run)?;
        self.define_hidden(g, "Map", map_ctor, HIDDEN)?;

        let set_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let set_ctor = self.register_native("Set", set_ctor_fn)?;
        let sco = set_ctor.as_object().unwrap();
        self.define_hidden(sco, "prototype", Value::object(set_proto), FIXED)?;
        self.define_hidden(set_proto, "constructor", set_ctor, HIDDEN)?;
        for (name, f) in [
            ("add", set_add as NativeFn),
            ("has", map_has),
            ("delete", map_delete),
            ("clear", map_clear),
            ("forEach", set_for_each),
            ("keys", set_values),
            ("values", set_values),
            ("entries", set_entries),
        ] {
            self.def_method(set_proto, name, f)?;
        }
        self.heap
            .define_own(
                set_proto,
                key_size,
                Property {
                    attrs: Attrs(Attrs::CONFIGURABLE),
                    slot: Slot::Accessor {
                        get: size_get,
                        set: Value::UNDEFINED,
                    },
                },
            )
            .map_err(heap_to_run)?;
        self.define_hidden(g, "Set", set_ctor, HIDDEN)?;

        let wm_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let wm_ctor = self.register_native("WeakMap", weak_map_ctor_fn)?;
        let wmco = wm_ctor.as_object().unwrap();
        self.define_hidden(wmco, "prototype", Value::object(wm_proto), FIXED)?;
        self.define_hidden(wm_proto, "constructor", wm_ctor, HIDDEN)?;
        for (name, f) in [
            ("get", map_get as NativeFn),
            ("set", weak_map_set),
            ("has", map_has),
            ("delete", map_delete),
        ] {
            self.def_method(wm_proto, name, f)?;
        }
        self.define_hidden(g, "WeakMap", wm_ctor, HIDDEN)?;

        let ws_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let ws_ctor = self.register_native("WeakSet", weak_set_ctor_fn)?;
        let wsco = ws_ctor.as_object().unwrap();
        self.define_hidden(wsco, "prototype", Value::object(ws_proto), FIXED)?;
        self.define_hidden(ws_proto, "constructor", ws_ctor, HIDDEN)?;
        for (name, f) in [
            ("add", weak_set_add as NativeFn),
            ("has", map_has),
            ("delete", map_delete),
        ] {
            self.def_method(ws_proto, name, f)?;
        }
        self.define_hidden(g, "WeakSet", ws_ctor, HIDDEN)?;

        let wr_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let wr_ctor = self.register_native("WeakRef", weak_ref_ctor_fn)?;
        let wrco = wr_ctor.as_object().unwrap();
        self.define_hidden(wrco, "prototype", Value::object(wr_proto), FIXED)?;
        self.define_hidden(wr_proto, "constructor", wr_ctor, HIDDEN)?;
        self.def_method(wr_proto, "deref", weak_ref_deref)?;
        self.define_hidden(g, "WeakRef", wr_ctor, HIDDEN)?;

        let fr_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let fr_ctor = self.register_native("FinalizationRegistry", finalization_registry_ctor)?;
        let frco = fr_ctor.as_object().unwrap();
        self.define_hidden(frco, "prototype", Value::object(fr_proto), FIXED)?;
        self.define_hidden(fr_proto, "constructor", fr_ctor, HIDDEN)?;
        self.def_method(fr_proto, "register", finalization_registry_register)?;
        self.def_method(fr_proto, "unregister", finalization_registry_unregister)?;
        self.define_hidden(g, "FinalizationRegistry", fr_ctor, HIDDEN)?;
        Ok(())
    }

    fn install_promise_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let p_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let p_ctor = self.register_native("Promise", promise_ctor)?;
        let pco = p_ctor.as_object().unwrap();
        self.define_hidden(pco, "prototype", Value::object(p_proto), FIXED)?;
        self.define_hidden(p_proto, "constructor", p_ctor, HIDDEN)?;
        for (name, f) in [
            ("resolve", promise_resolve_static as NativeFn),
            ("reject", promise_reject_static),
            ("all", promise_all),
            ("race", promise_race),
            ("allSettled", promise_all_settled),
            ("any", promise_any),
            ("withResolvers", promise_with_resolvers),
            ("__unwrap", promise_unwrap),
            ("__unwrap_all", promise_unwrap_all),
        ] {
            self.def_method(pco, name, f)?;
        }
        for (name, f) in [
            ("then", promise_then as NativeFn),
            ("catch", promise_catch),
            ("finally", promise_finally),
        ] {
            self.def_method(p_proto, name, f)?;
        }
        self.define_hidden(g, "Promise", p_ctor, HIDDEN)?;
        Ok(())
    }

    fn install_web_and_timer_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let date_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let date_ctor = self.register_native("Date", date_ctor_fn)?;
        let dco = date_ctor.as_object().unwrap();
        self.define_hidden(dco, "prototype", Value::object(date_proto), FIXED)?;
        self.define_hidden(date_proto, "constructor", date_ctor, HIDDEN)?;
        self.def_method(dco, "now", date_now)?;
        self.def_method(date_proto, "getTime", date_get_time)?;
        self.def_method(date_proto, "valueOf", date_get_time)?;
        self.def_method(date_proto, "toISOString", date_to_iso_string)?;
        self.define_hidden(g, "Date", date_ctor, HIDDEN)?;

        let perf = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.def_method(perf, "now", date_now)?;
        self.define_hidden(g, "performance", Value::object(perf), HIDDEN)?;

        self.install_bigint_proxy_reflect_intl_and_generators()?;
        Ok(())
    }
}

// ---- Map and Set ----------------------------------------------------------

fn lookup_global_proto(cx: &mut NativeCx<'_>, global_name: &str) -> Option<ObjectRef> {
    let ctor = cx.vm.get_global(global_name)?.as_object()?;
    let p = cx
        .vm
        .heap
        .get_own(ctor, cx.vm.intr.key_prototype)
        .ok()??
        .slot
        .data()?;
    p.as_object()
}

fn map_ctor_fn(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let proto = lookup_global_proto(cx, "Map").or(Some(cx.vm.intr.object_prototype));
    let obj = cx
        .vm
        .heap
        .new_collection(proto, CollectionKind::Map)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(obj));
    if let Some(iter) = args.first().copied()
        && !iter.is_nullish()
    {
        let entries = cx
            .vm
            .spread_values(cx.host, iter)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for entry in entries {
            let Some(eo) = entry.as_object() else {
                return Err(cx.type_error("Iterator value is not an entry object"));
            };
            let k = cx
                .vm
                .read(cx.host, Value::object(eo), PropertyKey::Index(0))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            let v = cx
                .vm
                .read(cx.host, Value::object(eo), PropertyKey::Index(1))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            let mk = cx.vm.heap.map_key(k).map_err(heap_to_run)?;
            cx.vm
                .heap
                .with_collection(obj, |d| d.set(mk, k, v))
                .map_err(heap_to_run)?;
        }
    }
    Ok(Value::object(obj))
}

fn set_ctor_fn(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let proto = lookup_global_proto(cx, "Set").or(Some(cx.vm.intr.object_prototype));
    let obj = cx
        .vm
        .heap
        .new_collection(proto, CollectionKind::Set)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(obj));
    if let Some(iter) = args.first().copied()
        && !iter.is_nullish()
    {
        let items = cx
            .vm
            .spread_values(cx.host, iter)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for item in items {
            let mk = cx.vm.heap.map_key(item).map_err(heap_to_run)?;
            cx.vm
                .heap
                .with_collection(obj, |d| d.set(mk, item, item))
                .map_err(heap_to_run)?;
        }
    }
    Ok(Value::object(obj))
}

fn collection_size_getter(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("size getter called on incompatible receiver"));
    };
    let Some(m) = cx.vm.heap.collection(o).map_err(heap_to_run)? else {
        return Err(cx.type_error("size getter called on incompatible receiver"));
    };
    Ok(Value::number(m.size() as f64))
}

fn map_get(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Map.prototype.get called on incompatible receiver"));
    };
    let key = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mk = cx.vm.heap.map_key(key).map_err(heap_to_run)?;
    let Some(m) = cx.vm.heap.collection(o).map_err(heap_to_run)? else {
        return Err(cx.type_error("Map.prototype.get called on incompatible receiver"));
    };
    Ok(m.get(&mk).unwrap_or(Value::UNDEFINED))
}

fn map_set(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Map.prototype.set called on incompatible receiver"));
    };
    let key = args.first().copied().unwrap_or(Value::UNDEFINED);
    let val = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let mk = cx.vm.heap.map_key(key).map_err(heap_to_run)?;
    let ok = cx
        .vm
        .heap
        .with_collection(o, |d| d.set(mk, key, val))
        .map_err(heap_to_run)?;
    if ok.is_none() {
        return Err(cx.type_error("Map.prototype.set called on incompatible receiver"));
    }
    Ok(this)
}

fn set_add(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Set.prototype.add called on incompatible receiver"));
    };
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mk = cx.vm.heap.map_key(val).map_err(heap_to_run)?;
    let ok = cx
        .vm
        .heap
        .with_collection(o, |d| d.set(mk, val, val))
        .map_err(heap_to_run)?;
    if ok.is_none() {
        return Err(cx.type_error("Set.prototype.add called on incompatible receiver"));
    }
    Ok(this)
}

fn map_has(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Collection.prototype.has called on incompatible receiver"));
    };
    let key = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mk = cx.vm.heap.map_key(key).map_err(heap_to_run)?;
    let Some(m) = cx.vm.heap.collection(o).map_err(heap_to_run)? else {
        return Err(cx.type_error("Collection.prototype.has called on incompatible receiver"));
    };
    Ok(Value::boolean(m.has(&mk)))
}

fn map_delete(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Collection.prototype.delete called on incompatible receiver"));
    };
    let key = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mk = cx.vm.heap.map_key(key).map_err(heap_to_run)?;
    let Some(removed) = cx
        .vm
        .heap
        .with_collection(o, |d| d.delete(&mk))
        .map_err(heap_to_run)?
    else {
        return Err(cx.type_error("Collection.prototype.delete called on incompatible receiver"));
    };
    Ok(Value::boolean(removed))
}

fn map_clear(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Collection.prototype.clear called on incompatible receiver"));
    };
    let ok = cx
        .vm
        .heap
        .with_collection(o, |d| d.clear())
        .map_err(heap_to_run)?;
    if ok.is_none() {
        return Err(cx.type_error("Collection.prototype.clear called on incompatible receiver"));
    }
    Ok(Value::UNDEFINED)
}

fn map_for_each(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Map.prototype.forEach called on incompatible receiver"));
    };
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(cb);
    cx.keep(this_arg);
    let mut pos = 0usize;
    loop {
        let next = cx
            .vm
            .heap
            .collection(o)
            .map_err(heap_to_run)?
            .and_then(|m| m.next_from(pos));
        let Some((idx, k, v)) = next else {
            break;
        };
        cx.keep(k);
        cx.keep(v);
        cx.call(cb, this_arg, &[v, k, this])?;
        pos = idx + 1;
    }
    Ok(Value::UNDEFINED)
}

fn set_for_each(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Set.prototype.forEach called on incompatible receiver"));
    };
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(cb);
    cx.keep(this_arg);
    let mut pos = 0usize;
    loop {
        let next = cx
            .vm
            .heap
            .collection(o)
            .map_err(heap_to_run)?
            .and_then(|m| m.next_from(pos));
        let Some((idx, k, _)) = next else {
            break;
        };
        cx.keep(k);
        cx.call(cb, this_arg, &[k, k, this])?;
        pos = idx + 1;
    }
    Ok(Value::UNDEFINED)
}

fn make_collection_iterator(
    cx: &mut NativeCx<'_>,
    this: Value,
    kind: IterKind,
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Collection iterator called on incompatible receiver"));
    };
    if cx
        .vm
        .heap
        .with_collection(o, |m| m.open_iterators = m.open_iterators.saturating_add(1))
        .map_err(heap_to_run)?
        .is_none()
    {
        return Err(cx.type_error("Collection iterator called on incompatible receiver"));
    };
    let iter = cx
        .vm
        .heap
        .alloc_object(
            Some(cx.vm.intr.object_prototype),
            ObjectClass::CollectionIterator {
                map: Some(o),
                pos: 0,
                kind,
            },
        )
        .map_err(heap_to_run)?;
    cx.vm.def_method(iter, "next", collection_iterator_next)?;
    Ok(Value::object(iter))
}

fn collection_iterator_next(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("CollectionIterator.next called on incompatible receiver"));
    };
    let (map, pos, kind) = match cx.vm.heap.object(o).map_err(heap_to_run)?.class {
        ObjectClass::CollectionIterator { map, pos, kind } => (map, pos, kind),
        _ => return Err(cx.type_error("CollectionIterator.next called on incompatible receiver")),
    };
    let Some(map_obj) = map else {
        return make_iter_result(cx, Value::UNDEFINED, true);
    };
    let next = cx
        .vm
        .heap
        .collection(map_obj)
        .map_err(heap_to_run)?
        .and_then(|m| m.next_from(pos as usize));
    let Some((idx, k, v)) = next else {
        let _ = cx.vm.heap.with_collection(map_obj, |m| {
            m.open_iterators = m.open_iterators.saturating_sub(1);
        });
        if let ObjectClass::CollectionIterator { map: m_ref, .. } =
            cx.vm.heap.class_mut(o).map_err(heap_to_run)?
        {
            *m_ref = None;
        }
        return make_iter_result(cx, Value::UNDEFINED, true);
    };
    if let ObjectClass::CollectionIterator { pos: p_ref, .. } =
        cx.vm.heap.class_mut(o).map_err(heap_to_run)?
    {
        *p_ref = (idx + 1) as u32;
    }
    let item = match kind {
        IterKind::Keys => k,
        IterKind::Values => v,
        IterKind::Entries => make_array_from_slice(cx, &[k, v])?,
    };
    make_iter_result(cx, item, false)
}

fn map_keys(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_collection_iterator(cx, this, IterKind::Keys)
}

fn map_values(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_collection_iterator(cx, this, IterKind::Values)
}

fn map_entries(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_collection_iterator(cx, this, IterKind::Entries)
}

fn set_values(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_collection_iterator(cx, this, IterKind::Values)
}

fn set_entries(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    make_collection_iterator(cx, this, IterKind::Entries)
}

/// ES2024 `Map.groupBy(items, callbackFn)`
fn map_group_by(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let items_v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let cb = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, items_v)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    for v in &items {
        cx.keep(*v);
    }
    cx.keep(cb);
    let proto = lookup_global_proto(cx, "Map").or(Some(cx.vm.intr.object_prototype));
    let out = cx
        .vm
        .heap
        .new_collection(proto, CollectionKind::Map)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(out));
    for (idx, item) in items.iter().enumerate() {
        let key_v = cx.call(cb, Value::UNDEFINED, &[*item, Value::number(idx as f64)])?;
        cx.keep(key_v);
        let mk = cx.vm.heap.map_key(key_v).map_err(heap_to_run)?;
        let existing = cx
            .vm
            .heap
            .collection(out)
            .map_err(heap_to_run)?
            .and_then(|m| m.get(&mk))
            .and_then(|v| v.as_object());
        let bucket = match existing {
            Some(b) => b,
            None => {
                let arr = cx
                    .vm
                    .heap
                    .new_array(Some(cx.vm.intr.array_prototype))
                    .map_err(heap_to_run)?;
                cx.vm
                    .heap
                    .with_collection(out, |m| m.set(mk, key_v, Value::object(arr)))
                    .map_err(heap_to_run)?;
                arr
            }
        };
        cx.vm
            .array_push(bucket, *item)
            .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(out))
}

// ---- Promise and Microtask Queue ------------------------------------------

const P_STATE: &str = "__p_state";
const P_VALUE: &str = "__p_val";
const P_FULFILL: &str = "__p_fulfill";
const P_REJECT: &str = "__p_reject";

fn new_pending_promise(cx: &mut NativeCx<'_>) -> Result<ObjectRef, RunError> {
    let proto = lookup_global_proto(cx, "Promise").or(Some(cx.vm.intr.object_prototype));
    let p = cx.vm.heap.new_object(proto).map_err(heap_to_run)?;
    cx.keep(Value::object(p));
    let f_arr = cx
        .vm
        .heap
        .new_array(Some(cx.vm.intr.array_prototype))
        .map_err(heap_to_run)?;
    let r_arr = cx
        .vm
        .heap
        .new_array(Some(cx.vm.intr.array_prototype))
        .map_err(heap_to_run)?;
    cx.vm
        .define_hidden(p, P_STATE, Value::number(0.0), HIDDEN)?;
    cx.vm.define_hidden(p, P_VALUE, Value::UNDEFINED, HIDDEN)?;
    cx.vm
        .define_hidden(p, P_FULFILL, Value::object(f_arr), HIDDEN)?;
    cx.vm
        .define_hidden(p, P_REJECT, Value::object(r_arr), HIDDEN)?;
    Ok(p)
}

fn make_promise_resolvers(
    cx: &mut NativeCx<'_>,
    promise: ObjectRef,
) -> Result<(Value, Value), RunError> {
    let res_raw = cx.vm.register_native("resolve", promise_resolve_bound)?;
    let rej_raw = cx.vm.register_native("reject", promise_reject_bound)?;
    let p_val = Value::object(promise);
    let resolve_fn = function_bind(cx, res_raw, &[p_val])?;
    let reject_fn = function_bind(cx, rej_raw, &[p_val])?;
    Ok((resolve_fn, reject_fn))
}

fn settle_promise(
    cx: &mut NativeCx<'_>,
    promise: ObjectRef,
    state: i32,
    val: Value,
) -> Result<(), RunError> {
    let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
    let cur = cx
        .vm
        .heap
        .get_own(promise, k_state)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_int32())
        .unwrap_or(0);
    if cur != 0 {
        return Ok(());
    }
    cx.vm
        .define_hidden(promise, P_STATE, Value::number(f64::from(state)), HIDDEN)?;
    cx.vm.define_hidden(promise, P_VALUE, val, HIDDEN)?;
    let queue_key = if state == 1 { P_FULFILL } else { P_REJECT };
    let k_q = cx.vm.heap.key_from_str(queue_key).map_err(heap_to_run)?;
    if let Some(q_val) = cx
        .vm
        .heap
        .get_own(promise, k_q)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
    {
        let handlers = read_array_items(cx, q_val)?;
        for h in handlers {
            if h.as_object().is_some() {
                cx.enqueue_job(h, Value::UNDEFINED, &[val])?;
            }
        }
    }
    Ok(())
}

fn promise_resolve_bound(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(p) = this.as_object() else {
        return Ok(Value::UNDEFINED);
    };
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    if let Some(vo) = val.as_object() {
        let k_then = cx.vm.heap.key_from_str("then").map_err(heap_to_run)?;
        if let crate::anvil_js::gc::Lookup::Found {
            prop:
                Property {
                    slot: Slot::Data(then_fn),
                    ..
                },
            ..
        } = cx.vm.heap.lookup(vo, k_then).map_err(heap_to_run)?
            && then_fn
                .as_object()
                .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
        {
            let (res, rej) = make_promise_resolvers(cx, p)?;
            cx.keep(res);
            cx.keep(rej);
            cx.call(then_fn, val, &[res, rej])?;
            return Ok(Value::UNDEFINED);
        }
    }
    settle_promise(cx, p, 1, val)?;
    Ok(Value::UNDEFINED)
}

fn promise_reject_bound(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(p) = this.as_object() else {
        return Ok(Value::UNDEFINED);
    };
    let reason = args.first().copied().unwrap_or(Value::UNDEFINED);
    settle_promise(cx, p, 2, reason)?;
    Ok(Value::UNDEFINED)
}

fn promise_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let exec = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !exec
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
    {
        return Err(cx.type_error("Promise resolver is not a function"));
    }
    let p = new_pending_promise(cx)?;
    let (res, rej) = make_promise_resolvers(cx, p)?;
    cx.keep(res);
    cx.keep(rej);
    if let Err(RunError::Thrown { value, .. }) = cx.call(exec, Value::UNDEFINED, &[res, rej]) {
        settle_promise(cx, p, 2, value)?;
    }
    Ok(Value::object(p))
}

fn promise_resolve_static(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let p = new_pending_promise(cx)?;
    promise_resolve_bound(cx, Value::object(p), &[val])?;
    Ok(Value::object(p))
}

fn promise_reject_static(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let reason = args.first().copied().unwrap_or(Value::UNDEFINED);
    let p = new_pending_promise(cx)?;
    settle_promise(cx, p, 2, reason)?;
    Ok(Value::object(p))
}

/// ES2024 `Promise.withResolvers()`
fn promise_with_resolvers(
    cx: &mut NativeCx<'_>,
    _this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let p = new_pending_promise(cx)?;
    let (res, rej) = make_promise_resolvers(cx, p)?;
    let out = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.vm
        .define_hidden(out, "promise", Value::object(p), Attrs::DEFAULT)?;
    cx.vm.define_hidden(out, "resolve", res, Attrs::DEFAULT)?;
    cx.vm.define_hidden(out, "reject", rej, Attrs::DEFAULT)?;
    Ok(Value::object(out))
}

fn promise_reaction_job(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let handler = args.first().copied().unwrap_or(Value::UNDEFINED);
    let res = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let rej = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    let is_reject = args.get(3).and_then(|v| v.as_bool()).unwrap_or(false);
    let val = args.get(4).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(res);
    cx.keep(rej);
    if handler
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
    {
        match cx.call(handler, Value::UNDEFINED, &[val]) {
            Ok(out) => {
                cx.keep(out);
                cx.call(res, Value::UNDEFINED, &[out])?;
            }
            Err(RunError::Thrown { value, .. }) => {
                cx.keep(value);
                cx.call(rej, Value::UNDEFINED, &[value])?;
            }
            Err(other) => return Err(other),
        }
    } else if is_reject {
        cx.call(rej, Value::UNDEFINED, &[val])?;
    } else {
        cx.call(res, Value::UNDEFINED, &[val])?;
    }
    Ok(Value::UNDEFINED)
}

fn promise_then(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(po) = this.as_object() else {
        return Err(cx.type_error("Promise.prototype.then called on non-object"));
    };
    let on_fulfill = args.first().copied().unwrap_or(Value::UNDEFINED);
    let on_reject = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let child = new_pending_promise(cx)?;
    let (res, rej) = make_promise_resolvers(cx, child)?;
    cx.keep(res);
    cx.keep(rej);
    let raw_job = cx.vm.register_native("reaction", promise_reaction_job)?;
    let f_job = function_bind(
        cx,
        raw_job,
        &[Value::UNDEFINED, on_fulfill, res, rej, Value::FALSE],
    )?;
    let r_job = function_bind(
        cx,
        raw_job,
        &[Value::UNDEFINED, on_reject, res, rej, Value::TRUE],
    )?;
    let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
    let state = cx
        .vm
        .heap
        .get_own(po, k_state)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_int32())
        .unwrap_or(0);
    let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
    let val = cx
        .vm
        .heap
        .get_own(po, k_val)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .unwrap_or(Value::UNDEFINED);
    match state {
        1 => cx.enqueue_job(f_job, Value::UNDEFINED, &[val])?,
        2 => cx.enqueue_job(r_job, Value::UNDEFINED, &[val])?,
        _ => {
            let kf = cx.vm.heap.key_from_str(P_FULFILL).map_err(heap_to_run)?;
            if let Some(fo) = cx
                .vm
                .heap
                .get_own(po, kf)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_object())
            {
                cx.vm
                    .array_push(fo, f_job)
                    .map_err(|e| cx.vm.exit_to_run(e))?;
            }
            let kr = cx.vm.heap.key_from_str(P_REJECT).map_err(heap_to_run)?;
            if let Some(ro) = cx
                .vm
                .heap
                .get_own(po, kr)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_object())
            {
                cx.vm
                    .array_push(ro, r_job)
                    .map_err(|e| cx.vm.exit_to_run(e))?;
            }
        }
    }
    Ok(Value::object(child))
}

fn promise_catch(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let on_reject = args.first().copied().unwrap_or(Value::UNDEFINED);
    promise_then(cx, this, &[Value::UNDEFINED, on_reject])
}

fn finally_passthru(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = this;
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    cx.keep(val);
    if cb
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
    {
        cx.call(cb, Value::UNDEFINED, &[])?;
    }
    Ok(val)
}

fn finally_rethrow(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = this;
    let reason = args.first().copied().unwrap_or(Value::UNDEFINED);
    cx.keep(reason);
    if cb
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
    {
        cx.call(cb, Value::UNDEFINED, &[])?;
    }
    Err(cx.vm.exit_to_run(Exit::Throw {
        value: reason,
        stack: None,
    }))
}

fn promise_finally(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    let pass_raw = cx.vm.register_native("finally_pass", finally_passthru)?;
    let throw_raw = cx.vm.register_native("finally_throw", finally_rethrow)?;
    let on_f = function_bind(cx, pass_raw, &[cb])?;
    let on_r = function_bind(cx, throw_raw, &[cb])?;
    promise_then(cx, this, &[on_f, on_r])
}

fn promise_all(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let iter = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, iter)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        if let Some(o) = item.as_object() {
            let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
            let state = cx
                .vm
                .heap
                .get_own(o, k_state)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_int32());
            let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
            let val = cx
                .vm
                .heap
                .get_own(o, k_val)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .unwrap_or(Value::UNDEFINED);
            if state == Some(2) {
                return promise_reject_static(cx, Value::UNDEFINED, &[val]);
            } else if state == Some(1) {
                results.push(val);
                continue;
            }
        }
        results.push(item);
    }
    let arr = make_array_from_slice(cx, &results)?;
    promise_resolve_static(cx, Value::UNDEFINED, &[arr])
}

fn promise_race(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let iter = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, iter)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let Some(first) = items.first().copied() else {
        let p = new_pending_promise(cx)?;
        return Ok(Value::object(p));
    };
    if let Some(o) = first.as_object() {
        let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
        if cx
            .vm
            .heap
            .get_own(o, k_state)
            .map_err(heap_to_run)?
            .is_some()
        {
            return Ok(first);
        }
    }
    promise_resolve_static(cx, Value::UNDEFINED, &[first])
}

fn promise_all_settled(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let iter = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, iter)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let entry = cx
            .vm
            .heap
            .new_object(Some(cx.vm.intr.object_prototype))
            .map_err(heap_to_run)?;
        cx.keep(Value::object(entry));
        let mut status = "fulfilled";
        let mut payload_key = "value";
        let mut payload = item;
        if let Some(o) = item.as_object() {
            let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
            let state = cx
                .vm
                .heap
                .get_own(o, k_state)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_int32());
            let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
            if let Some(st) = state {
                payload = cx
                    .vm
                    .heap
                    .get_own(o, k_val)
                    .map_err(heap_to_run)?
                    .and_then(|p| p.slot.data())
                    .unwrap_or(Value::UNDEFINED);
                if st == 2 {
                    status = "rejected";
                    payload_key = "reason";
                }
            }
        }
        let st_v = cx.vm.string(status)?;
        cx.vm.define_hidden(entry, "status", st_v, Attrs::DEFAULT)?;
        cx.vm
            .define_hidden(entry, payload_key, payload, Attrs::DEFAULT)?;
        out.push(Value::object(entry));
    }
    let arr = make_array_from_slice(cx, &out)?;
    promise_resolve_static(cx, Value::UNDEFINED, &[arr])
}

fn promise_any(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let iter = args.first().copied().unwrap_or(Value::UNDEFINED);
    let items = cx
        .vm
        .spread_values(cx.host, iter)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let mut rejected = Vec::new();
    for item in items {
        if let Some(o) = item.as_object() {
            let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
            let state = cx
                .vm
                .heap
                .get_own(o, k_state)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_int32());
            if state == Some(2) {
                let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
                let val = cx
                    .vm
                    .heap
                    .get_own(o, k_val)
                    .map_err(heap_to_run)?
                    .and_then(|p| p.slot.data())
                    .unwrap_or(Value::UNDEFINED);
                rejected.push(val);
                continue;
            }
            if state == Some(1) {
                let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
                let val = cx
                    .vm
                    .heap
                    .get_own(o, k_val)
                    .map_err(heap_to_run)?
                    .and_then(|p| p.slot.data())
                    .unwrap_or(Value::UNDEFINED);
                return promise_resolve_static(cx, Value::UNDEFINED, &[val]);
            }
        }
        return promise_resolve_static(cx, Value::UNDEFINED, &[item]);
    }
    let errs_arr = make_array_from_slice(cx, &rejected)?;
    let msg = cx.vm.string("All promises were rejected")?;
    let err = aggregate_error_ctor(cx, Value::UNDEFINED, &[errs_arr, msg])?;
    promise_reject_static(cx, Value::UNDEFINED, &[err])
}

// ---- Web, Timer, URI, Base64 & structuredClone ----------------------------

fn global_queue_microtask(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !cb
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()))
    {
        return Err(cx.type_error("queueMicrotask requires a callback function"));
    }
    cx.enqueue_job(cb, Value::UNDEFINED, &[])?;
    Ok(Value::UNDEFINED)
}

fn clone_recursive(
    cx: &mut NativeCx<'_>,
    v: Value,
    seen: &mut HashMap<ObjectRef, ObjectRef>,
) -> Result<Value, RunError> {
    let Some(o) = v.as_object() else {
        if v.is_symbol() {
            return Err(cx.type_error("Symbol cannot be cloned"));
        }
        return Ok(v);
    };
    if let Some(&existing) = seen.get(&o) {
        return Ok(Value::object(existing));
    }
    let obj = cx.vm.heap.object(o).map_err(heap_to_run)?.clone();
    if obj.is_callable() {
        return Err(cx.type_error("Function object cannot be cloned"));
    }
    if obj.is_array() {
        let new_arr = cx
            .vm
            .heap
            .new_array(Some(cx.vm.intr.array_prototype))
            .map_err(heap_to_run)?;
        cx.keep(Value::object(new_arr));
        seen.insert(o, new_arr);
        let items = read_array_items(cx, v)?;
        for (i, item) in items.into_iter().enumerate() {
            let cloned = clone_recursive(cx, item, seen)?;
            cx.vm
                .heap
                .define_own(
                    new_arr,
                    PropertyKey::Index(i as u32),
                    Property::data(cloned),
                )
                .map_err(heap_to_run)?;
        }
        return Ok(Value::object(new_arr));
    }
    if let ObjectClass::Collection(m) = &obj.class
        && !m.kind.is_weak()
    {
        let proto = lookup_global_proto(cx, m.kind.name()).or(Some(cx.vm.intr.object_prototype));
        let new_col = cx
            .vm
            .heap
            .new_collection(proto, m.kind)
            .map_err(heap_to_run)?;
        cx.keep(Value::object(new_col));
        seen.insert(o, new_col);
        let entries: Vec<(Value, Value)> = m.entries.iter().flatten().copied().collect();
        for (k, val) in entries {
            let ck = clone_recursive(cx, k, seen)?;
            let cv = if m.kind == CollectionKind::Set {
                ck
            } else {
                clone_recursive(cx, val, seen)?
            };
            let mk = cx.vm.heap.map_key(ck).map_err(heap_to_run)?;
            cx.vm
                .heap
                .with_collection(new_col, |d| d.set(mk, ck, cv))
                .map_err(heap_to_run)?;
        }
        return Ok(Value::object(new_col));
    }
    let new_obj = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.keep(Value::object(new_obj));
    seen.insert(o, new_obj);
    let pairs = cx
        .vm
        .own_enumerable(cx.host, v)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    for (k, val) in pairs {
        if k.is_symbol() {
            continue;
        }
        let cloned = clone_recursive(cx, val, seen)?;
        cx.vm
            .heap
            .define_own(new_obj, k, Property::data(cloned))
            .map_err(heap_to_run)?;
    }
    Ok(Value::object(new_obj))
}

/// ES2022 `structuredClone(value)`
fn global_structured_clone(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let mut seen = HashMap::new();
    clone_recursive(cx, v, &mut seen)
}

fn global_encode_uri_component(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                *b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    cx.vm.string(&out)
}

fn global_decode_uri_component(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(cx.vm.make_thrown(ErrorKind::Error, "URI malformed"));
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .map_err(|_| cx.vm.make_thrown(ErrorKind::Error, "URI malformed"))?;
            let val = u8::from_str_radix(hex, 16)
                .map_err(|_| cx.vm.make_thrown(ErrorKind::Error, "URI malformed"))?;
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded =
        String::from_utf8(out).map_err(|_| cx.vm.make_thrown(ErrorKind::Error, "URI malformed"))?;
    cx.vm.string(&decoded)
}

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn global_btoa(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let units = recv_string_units(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let mut bytes = Vec::with_capacity(units.len());
    for u in units {
        if u > 0xFF {
            return Err(cx.vm.make_thrown(
                ErrorKind::Error,
                "InvalidCharacterError: btoa input contains characters outside Latin1",
            ));
        }
        bytes.push(u as u8);
    }
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(B64_TABLE[(b0 >> 2) as usize] as char);
        out.push(B64_TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_TABLE[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    cx.vm.string(&out)
}

fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn global_atob(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .collect();
    let mut units = Vec::new();
    let mut i = 0;
    while i < clean.len() {
        let c0 = clean[i];
        let c1 = *clean
            .get(i + 1)
            .ok_or_else(|| cx.vm.make_thrown(ErrorKind::Error, "Invalid base64"))?;
        let c2 = clean.get(i + 2).copied().unwrap_or(b'=');
        let c3 = clean.get(i + 3).copied().unwrap_or(b'=');
        let v0 =
            b64_val(c0).ok_or_else(|| cx.vm.make_thrown(ErrorKind::Error, "Invalid base64"))?;
        let v1 =
            b64_val(c1).ok_or_else(|| cx.vm.make_thrown(ErrorKind::Error, "Invalid base64"))?;
        units.push(u16::from((v0 << 2) | (v1 >> 4)));
        if c2 != b'=' {
            let v2 =
                b64_val(c2).ok_or_else(|| cx.vm.make_thrown(ErrorKind::Error, "Invalid base64"))?;
            units.push(u16::from(((v1 & 0x0F) << 4) | (v2 >> 2)));
            if c3 != b'=' {
                let v3 = b64_val(c3)
                    .ok_or_else(|| cx.vm.make_thrown(ErrorKind::Error, "Invalid base64"))?;
                units.push(u16::from(((v2 & 0x03) << 6) | v3));
            }
        }
        i += 4;
    }
    let out = cx.vm.heap.alloc_string_units(units).map_err(heap_to_run)?;
    Ok(Value::string(out))
}

fn date_now(cx: &mut NativeCx<'_>, _this: Value, _args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::number(cx.host.now_ms()))
}

fn date_ctor_fn(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let ms = match args.first() {
        Some(v) => to_num(cx, *v)?,
        None => cx.host.now_ms(),
    };
    if let Some(o) = this.as_object()
        && o != cx.vm.intr.global
    {
        cx.vm
            .define_hidden(o, "__date_ms", Value::number(ms), HIDDEN)?;
        return Ok(this);
    }
    cx.vm.string("1970-01-01T00:00:00.000Z")
}

fn date_get_time(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Date.prototype.getTime called on incompatible receiver"));
    };
    let k = cx.vm.heap.key_from_str("__date_ms").map_err(heap_to_run)?;
    let ms = cx
        .vm
        .heap
        .get_own(o, k)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_number())
        .unwrap_or(f64::NAN);
    Ok(Value::number(ms))
}

fn date_to_iso_string(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let ms = date_get_time(cx, this, &[])?.as_number().unwrap_or(0.0);
    cx.vm.string(&format!(
        "1970-01-01T00:00:00.{:03}Z",
        (ms.abs() as u64) % 1000
    ))
}

// ---- Promise.__unwrap / Promise.__unwrap_all & RegExp ---------------------

fn promise_unwrap(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let mut cur = args.first().copied().unwrap_or(Value::UNDEFINED);
    cx.keep(cur);
    cx.vm.drain_jobs(cx.host)?;
    let k_state = cx.vm.heap.key_from_str(P_STATE).map_err(heap_to_run)?;
    let k_val = cx.vm.heap.key_from_str(P_VALUE).map_err(heap_to_run)?;
    let k_then = cx.vm.heap.key_from_str("then").map_err(heap_to_run)?;
    for _ in 0..16 {
        let Some(o) = cur.as_object() else {
            break;
        };
        let mut state = cx
            .vm
            .heap
            .get_own(o, k_state)
            .map_err(heap_to_run)?
            .and_then(|p| p.slot.data())
            .and_then(|v| v.as_int32());
        if state == Some(0) {
            cx.vm.drain_jobs(cx.host)?;
            state = cx
                .vm
                .heap
                .get_own(o, k_state)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_int32());
        }
        if let Some(st) = state {
            let val = cx
                .vm
                .heap
                .get_own(o, k_val)
                .map_err(heap_to_run)?
                .and_then(|p| p.slot.data())
                .unwrap_or(Value::UNDEFINED);
            if st == 1 {
                cur = val;
                cx.keep(cur);
                continue;
            } else if st == 2 {
                return Err(cx.vm.exit_to_run(Exit::Throw {
                    value: val,
                    stack: None,
                }));
            }
            break;
        }
        if let crate::anvil_js::gc::Lookup::Found {
            prop:
                Property {
                    slot: Slot::Data(then_fn),
                    ..
                },
            ..
        } = cx.vm.heap.lookup(o, k_then).map_err(heap_to_run)?
            && then_fn
                .as_object()
                .is_some_and(|to| cx.vm.heap.object(to).is_ok_and(|x| x.is_callable()))
        {
            cur = promise_resolve_static(cx, Value::UNDEFINED, &[cur])?;
            cx.keep(cur);
            cx.vm.drain_jobs(cx.host)?;
            continue;
        }
        break;
    }
    Ok(cur)
}

fn promise_unwrap_all(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let raw = args.first().copied().unwrap_or(Value::UNDEFINED);
    let resolved = promise_unwrap(cx, Value::UNDEFINED, &[raw])?;
    cx.keep(resolved);
    let items = cx
        .vm
        .spread_values(cx.host, resolved)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    for v in &items {
        cx.keep(*v);
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let u = promise_unwrap(cx, Value::UNDEFINED, &[item])?;
        cx.keep(u);
        out.push(u);
    }
    make_array_from_slice(cx, &out)
}

const RE_SOURCE: &str = "__re_source";
const RE_FLAGS: &str = "__re_flags";

impl Vm {
    fn install_regexp_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;
        let re_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        let re_ctor = self.register_native("RegExp", regexp_ctor)?;
        let rco = re_ctor.as_object().unwrap();
        self.define_hidden(rco, "prototype", Value::object(re_proto), FIXED)?;
        self.define_hidden(re_proto, "constructor", re_ctor, HIDDEN)?;
        for (name, f) in [
            ("exec", regexp_exec as NativeFn),
            ("test", regexp_test),
            ("toString", regexp_to_string),
        ] {
            self.def_method(re_proto, name, f)?;
        }
        self.define_hidden(g, "RegExp", re_ctor, HIDDEN)?;
        Ok(())
    }
}

fn extract_regexp_pair(
    cx: &mut NativeCx<'_>,
    val: Value,
) -> Result<Option<(String, String)>, RunError> {
    let Some(o) = val.as_object() else {
        return Ok(None);
    };
    let ks = cx.vm.heap.key_from_str(RE_SOURCE).map_err(heap_to_run)?;
    let Some(src_v) = cx
        .vm
        .heap
        .get_own(o, ks)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
    else {
        return Ok(None);
    };
    let kf = cx.vm.heap.key_from_str(RE_FLAGS).map_err(heap_to_run)?;
    let flags_v = cx
        .vm
        .heap
        .get_own(o, kf)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .unwrap_or(Value::UNDEFINED);
    let src = to_str_rust(cx, src_v)?;
    let flags = if flags_v.is_undefined() {
        String::new()
    } else {
        to_str_rust(cx, flags_v)?
    };
    Ok(Some((src, flags)))
}

fn build_regexp_object(
    cx: &mut NativeCx<'_>,
    target: Option<ObjectRef>,
    pattern: &str,
    flags_str: &str,
) -> Result<Value, RunError> {
    let (compiled, canonical_flags) = super::regex::CompiledRegex::compile(pattern, flags_str)
        .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
    let obj = match target {
        Some(o) if o != cx.vm.intr.global => o,
        _ => {
            let proto = lookup_global_proto(cx, "RegExp").or(Some(cx.vm.intr.object_prototype));
            cx.vm.heap.new_object(proto).map_err(heap_to_run)?
        }
    };
    cx.keep(Value::object(obj));
    let src_text = if pattern.is_empty() { "(?:)" } else { pattern };
    let src_v = cx.vm.string(src_text)?;
    let raw_v = cx.vm.string(pattern)?;
    let flags_v = cx.vm.string(&canonical_flags)?;
    cx.vm.define_hidden(obj, RE_SOURCE, raw_v, HIDDEN)?;
    cx.vm.define_hidden(obj, RE_FLAGS, flags_v, HIDDEN)?;
    cx.vm.define_hidden(obj, "source", src_v, FIXED)?;
    cx.vm.define_hidden(obj, "flags", flags_v, FIXED)?;
    cx.vm
        .define_hidden(obj, "global", Value::boolean(compiled.flags.global), FIXED)?;
    cx.vm.define_hidden(
        obj,
        "ignoreCase",
        Value::boolean(compiled.flags.ignore_case),
        FIXED,
    )?;
    cx.vm.define_hidden(
        obj,
        "multiline",
        Value::boolean(compiled.flags.multiline),
        FIXED,
    )?;
    cx.vm
        .define_hidden(obj, "dotAll", Value::boolean(compiled.flags.dot_all), FIXED)?;
    cx.vm.define_hidden(
        obj,
        "unicode",
        Value::boolean(compiled.flags.unicode),
        FIXED,
    )?;
    cx.vm
        .define_hidden(obj, "sticky", Value::boolean(compiled.flags.sticky), FIXED)?;
    cx.vm
        .define_hidden(obj, "lastIndex", Value::number(0.0), Attrs::DEFAULT)?;
    Ok(Value::object(obj))
}

fn regexp_ctor(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let pat_arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let flags_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    if let Some((existing_pat, existing_flags)) = extract_regexp_pair(cx, pat_arg)? {
        let flags = if flags_arg.is_undefined() {
            existing_flags
        } else {
            to_str_rust(cx, flags_arg)?
        };
        return build_regexp_object(cx, this.as_object(), &existing_pat, &flags);
    }
    let pattern = if pat_arg.is_undefined() {
        String::new()
    } else {
        to_str_rust(cx, pat_arg)?
    };
    let flags = if flags_arg.is_undefined() {
        String::new()
    } else {
        to_str_rust(cx, flags_arg)?
    };
    build_regexp_object(cx, this.as_object(), &pattern, &flags)
}

fn make_exec_match_array(
    cx: &mut NativeCx<'_>,
    chars: &[char],
    m: &super::regex::ReMatch,
    input_val: Value,
) -> Result<Value, RunError> {
    let mut items = Vec::with_capacity(1 + m.groups.len());
    let full: String = chars[m.start_char..m.end_char].iter().collect();
    items.push(cx.vm.string(&full)?);
    for g in &m.groups {
        match g {
            Some((gs, ge)) => {
                let s: String = chars[*gs..*ge].iter().collect();
                items.push(cx.vm.string(&s)?);
            }
            None => items.push(Value::UNDEFINED),
        }
    }
    let arr_val = make_array_from_slice(cx, &items)?;
    if let Some(arr_obj) = arr_val.as_object() {
        cx.vm.define_hidden(
            arr_obj,
            "index",
            Value::number(m.start_char as f64),
            Attrs::DEFAULT,
        )?;
        cx.vm
            .define_hidden(arr_obj, "input", input_val, Attrs::DEFAULT)?;
    }
    Ok(arr_val)
}

fn regexp_exec(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(re_obj) = this.as_object() else {
        return Err(cx.type_error("RegExp.prototype.exec called on non-object"));
    };
    let Some((pattern, flags)) = extract_regexp_pair(cx, this)? else {
        return Err(cx.type_error("RegExp.prototype.exec called on incompatible receiver"));
    };
    let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
        .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
    let input_arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let input_str = to_str_rust(cx, input_arg)?;
    let input_val = cx.vm.string(&input_str)?;
    cx.keep(input_val);
    let chars: Vec<char> = input_str.chars().collect();
    let stateful = compiled.flags.global || compiled.flags.sticky;
    let k_last = cx.vm.heap.key_from_str("lastIndex").map_err(heap_to_run)?;
    let start_pos = if stateful {
        let li = cx
            .vm
            .read(cx.host, this, k_last)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        to_int_or_inf(cx, li)?.max(0.0) as usize
    } else {
        0
    };
    let Some(m) = compiled.exec_at(&chars, start_pos) else {
        if stateful {
            cx.vm
                .define_hidden(re_obj, "lastIndex", Value::number(0.0), Attrs::DEFAULT)?;
        }
        return Ok(Value::NULL);
    };
    if stateful {
        cx.vm.define_hidden(
            re_obj,
            "lastIndex",
            Value::number(m.end_char as f64),
            Attrs::DEFAULT,
        )?;
    }
    make_exec_match_array(cx, &chars, &m, input_val)
}

fn regexp_test(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let res = regexp_exec(cx, this, args)?;
    Ok(Value::boolean(!res.is_null()))
}

fn regexp_to_string(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some((pattern, flags)) = extract_regexp_pair(cx, this)? else {
        return Err(cx.type_error("RegExp.prototype.toString called on incompatible receiver"));
    };
    let src = if pattern.is_empty() { "(?:)" } else { &pattern };
    cx.vm.string(&format!("/{src}/{flags}"))
}

fn ensure_regexp_pair(cx: &mut NativeCx<'_>, arg: Value) -> Result<(String, String), RunError> {
    if let Some(pair) = extract_regexp_pair(cx, arg)? {
        return Ok(pair);
    }
    let pat = if arg.is_undefined() {
        String::new()
    } else {
        to_str_rust(cx, arg)?
    };
    Ok((pat, String::new()))
}

fn string_match(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let input_val = cx.vm.string(&s)?;
    cx.keep(input_val);
    let arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let (pattern, flags) = ensure_regexp_pair(cx, arg)?;
    let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
        .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
    let chars: Vec<char> = s.chars().collect();
    if !compiled.flags.global {
        let Some(m) = compiled.exec_at(&chars, 0) else {
            return Ok(Value::NULL);
        };
        return make_exec_match_array(cx, &chars, &m, input_val);
    }
    let mut matches = Vec::new();
    let mut pos = 0usize;
    while pos <= chars.len() {
        let Some(m) = compiled.exec_at(&chars, pos) else {
            break;
        };
        let text: String = chars[m.start_char..m.end_char].iter().collect();
        matches.push(cx.vm.string(&text)?);
        pos = if m.end_char == m.start_char {
            m.end_char + 1
        } else {
            m.end_char
        };
    }
    if matches.is_empty() {
        Ok(Value::NULL)
    } else {
        make_array_from_slice(cx, &matches)
    }
}

fn string_match_all(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let input_val = cx.vm.string(&s)?;
    cx.keep(input_val);
    let arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let (pattern, flags) = ensure_regexp_pair(cx, arg)?;
    let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
        .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
    let chars: Vec<char> = s.chars().collect();
    let mut results = Vec::new();
    let mut pos = 0usize;
    while pos <= chars.len() {
        let Some(m) = compiled.exec_at(&chars, pos) else {
            break;
        };
        let arr = make_exec_match_array(cx, &chars, &m, input_val)?;
        cx.keep(arr);
        results.push(arr);
        pos = if m.end_char == m.start_char {
            m.end_char + 1
        } else {
            m.end_char
        };
    }
    make_array_from_slice(cx, &results)
}

fn string_search(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let (pattern, flags) = ensure_regexp_pair(cx, arg)?;
    let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
        .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
    let chars: Vec<char> = s.chars().collect();
    match compiled.exec_at(&chars, 0) {
        Some(m) => Ok(Value::number(m.start_char as f64)),
        None => Ok(Value::number(-1.0)),
    }
}

fn expand_replacement_template(
    template: &str,
    chars: &[char],
    m: &super::regex::ReMatch,
) -> String {
    let mut out = String::with_capacity(template.len());
    let tchars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < tchars.len() {
        if tchars[i] == '$' && i + 1 < tchars.len() {
            match tchars[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                }
                '&' => {
                    out.extend(&chars[m.start_char..m.end_char]);
                    i += 2;
                }
                '`' => {
                    out.extend(&chars[..m.start_char]);
                    i += 2;
                }
                '\'' => {
                    out.extend(&chars[m.end_char..]);
                    i += 2;
                }
                d @ '1'..='9' => {
                    let idx = (d as u8 - b'1') as usize;
                    if let Some(Some((gs, ge))) = m.groups.get(idx) {
                        out.extend(&chars[*gs..*ge]);
                    }
                    i += 2;
                }
                _ => {
                    out.push('$');
                    i += 1;
                }
            }
        } else {
            out.push(tchars[i]);
            i += 1;
        }
    }
    out
}

fn string_replace_impl(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
    replace_all: bool,
) -> Result<Value, RunError> {
    let s = to_str_rust(cx, this)?;
    let search_v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let repl_v = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let repl_is_fn = repl_v
        .as_object()
        .is_some_and(|o| cx.vm.heap.object(o).is_ok_and(|x| x.is_callable()));

    if let Some((pattern, flags)) = extract_regexp_pair(cx, search_v)? {
        let (compiled, _) = super::regex::CompiledRegex::compile(&pattern, &flags)
            .map_err(|msg| cx.vm.make_thrown(ErrorKind::Syntax, &msg))?;
        let all = replace_all || compiled.flags.global;
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut last_end = 0usize;
        let mut pos = 0usize;
        let repl_str = if !repl_is_fn {
            Some(to_str_rust(cx, repl_v)?)
        } else {
            None
        };
        let s_val = cx.vm.string(&s)?;
        cx.keep(s_val);
        while pos <= chars.len() {
            let Some(m) = compiled.exec_at(&chars, pos) else {
                break;
            };
            out.extend(&chars[last_end..m.start_char]);
            if repl_is_fn {
                let full_str: String = chars[m.start_char..m.end_char].iter().collect();
                let mut call_args = Vec::with_capacity(3 + m.groups.len());
                call_args.push(cx.vm.string(&full_str)?);
                for g in &m.groups {
                    match g {
                        Some((gs, ge)) => {
                            let gs_str: String = chars[*gs..*ge].iter().collect();
                            call_args.push(cx.vm.string(&gs_str)?);
                        }
                        None => call_args.push(Value::UNDEFINED),
                    }
                }
                call_args.push(Value::number(m.start_char as f64));
                call_args.push(s_val);
                let r = cx.call(repl_v, Value::UNDEFINED, &call_args)?;
                out.push_str(&to_str_rust(cx, r)?);
            } else if let Some(tpl) = &repl_str {
                out.push_str(&expand_replacement_template(tpl, &chars, &m));
            }
            last_end = m.end_char;
            if !all {
                break;
            }
            pos = if m.end_char == m.start_char {
                m.end_char + 1
            } else {
                m.end_char
            };
        }
        out.extend(&chars[last_end..]);
        return cx.vm.string(&out);
    }

    let from = to_str_rust(cx, search_v)?;
    if !repl_is_fn {
        let to = to_str_rust(cx, repl_v)?;
        return if replace_all {
            cx.vm.string(&s.replace(&from, &to))
        } else {
            cx.vm.string(&s.replacen(&from, &to, 1))
        };
    }
    let s_val = cx.vm.string(&s)?;
    cx.keep(s_val);
    let from_val = cx.vm.string(&from)?;
    cx.keep(from_val);
    if !replace_all {
        if let Some(byte_idx) = s.find(&from) {
            let char_idx = s[..byte_idx].chars().count();
            let r = cx.call(
                repl_v,
                Value::UNDEFINED,
                &[from_val, Value::number(char_idx as f64), s_val],
            )?;
            let rep = to_str_rust(cx, r)?;
            let mut out = String::with_capacity(s.len() + rep.len());
            out.push_str(&s[..byte_idx]);
            out.push_str(&rep);
            out.push_str(&s[byte_idx + from.len()..]);
            return cx.vm.string(&out);
        }
        return cx.vm.string(&s);
    }
    cx.vm.string(&s)
}

// ---- ES2022-ES2024 Extensions: BigInt, Proxy, Reflect, AggregateError, Intl, Generators & Modules ----

impl Vm {
    fn install_bigint_proxy_reflect_intl_and_generators(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;

        // BigInt
        let bi_ctor = self.register_native("BigInt", bigint_ctor_fn)?;
        let bico = bi_ctor.as_object().unwrap();
        self.define_hidden(
            bico,
            "prototype",
            Value::object(self.intr.number_prototype),
            FIXED,
        )?;
        self.def_method(bico, "asIntN", bigint_as_int_n)?;
        self.def_method(bico, "asUintN", bigint_as_uint_n)?;
        self.define_hidden(g, "BigInt", bi_ctor, HIDDEN)?;

        // AggregateError
        let ae_proto = self
            .heap
            .new_object(Some(self.intr.error_prototypes[0]))
            .map_err(heap_to_run)?;
        let ae_name = self.string("AggregateError")?;
        self.define_hidden(ae_proto, "name", ae_name, HIDDEN)?;
        let ae_ctor = self.register_native("AggregateError", aggregate_error_ctor)?;
        let aeco = ae_ctor.as_object().unwrap();
        self.define_hidden(aeco, "prototype", Value::object(ae_proto), FIXED)?;
        self.define_hidden(ae_proto, "constructor", ae_ctor, HIDDEN)?;
        self.define_hidden(g, "AggregateError", ae_ctor, HIDDEN)?;

        // Proxy
        let proxy_ctor = self.register_native("Proxy", proxy_ctor_fn)?;
        let pxco = proxy_ctor.as_object().unwrap();
        self.def_method(pxco, "revocable", proxy_revocable_fn)?;
        self.define_hidden(g, "Proxy", proxy_ctor, HIDDEN)?;

        // Reflect
        let reflect = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        for (name, f) in [
            ("get", reflect_get as NativeFn),
            ("set", reflect_set),
            ("has", reflect_has),
            ("deleteProperty", reflect_delete_property),
            ("ownKeys", reflect_own_keys),
            ("apply", reflect_apply),
            ("construct", reflect_construct),
            ("getPrototypeOf", object_get_prototype_of),
            ("setPrototypeOf", reflect_set_prototype_of),
            ("defineProperty", reflect_define_property),
            (
                "getOwnPropertyDescriptor",
                object_get_own_property_descriptor,
            ),
            ("isExtensible", object_is_extensible),
            ("preventExtensions", reflect_prevent_extensions),
        ] {
            self.def_method(reflect, name, f)?;
        }
        self.define_hidden(g, "Reflect", Value::object(reflect), HIDDEN)?;

        // Intl
        let intl = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        for (name, f) in [
            ("DateTimeFormat", intl_date_time_format_ctor as NativeFn),
            ("NumberFormat", intl_number_format_ctor),
            ("Collator", intl_collator_ctor),
            ("PluralRules", intl_plural_rules_ctor),
        ] {
            self.def_method(intl, name, f)?;
        }
        self.define_hidden(g, "Intl", Value::object(intl), HIDDEN)?;

        // Generator & ES Module helpers
        self.def_method(g, "__anvil_yield", anvil_yield_helper)?;
        self.def_method(g, "__anvil_make_generator", anvil_make_generator_helper)?;
        self.def_method(g, "__anvil_import_module", anvil_import_module_helper)?;
        let mod_reg = self.heap.new_object(None).map_err(heap_to_run)?;
        self.define_hidden(g, "__anvil_modules", Value::object(mod_reg), HIDDEN)?;
        let mod_exp = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.define_hidden(g, "__anvil_exports", Value::object(mod_exp), HIDDEN)?;

        Ok(())
    }
}

fn object_get_own_property_descriptors(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    };
    let out = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.keep(Value::object(out));
    let keys = cx.vm.heap.own_keys(o).map_err(heap_to_run)?;
    for k in keys {
        let kv = cx.vm.prop_key_value(k).map_err(|e| cx.vm.exit_to_run(e))?;
        let desc = object_get_own_property_descriptor(cx, Value::UNDEFINED, &[target, kv])?;
        if !desc.is_undefined() {
            cx.vm
                .heap
                .define_own(out, k, Property::data(desc))
                .map_err(heap_to_run)?;
        }
    }
    Ok(Value::object(out))
}

fn object_define_properties(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let props = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    if target.as_object().is_none() {
        return Err(cx.type_error("Object.defineProperties called on non-object"));
    }
    let pairs = cx
        .vm
        .own_enumerable(cx.host, props)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    for (k, desc) in pairs {
        let kv = cx.vm.prop_key_value(k).map_err(|e| cx.vm.exit_to_run(e))?;
        object_define_property(cx, Value::UNDEFINED, &[target, kv, desc])?;
    }
    Ok(target)
}

fn object_get_own_property_symbols(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    };
    let keys = cx.vm.heap.own_keys(o).map_err(heap_to_run)?;
    let mut out = Vec::new();
    for k in keys {
        if let PropertyKey::Symbol(s) = k {
            out.push(Value::symbol(s));
        }
    }
    make_array_from_slice(cx, &out)
}

fn object_proto_is_prototype_of(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(proto) = this.as_object() else {
        return Ok(Value::FALSE);
    };
    let mut cur = args.first().and_then(|v| v.as_object());
    let mut hops = 0usize;
    while let Some(o) = cur {
        let p = cx.vm.heap.get_prototype(o).map_err(heap_to_run)?;
        if p == Some(proto) {
            return Ok(Value::TRUE);
        }
        cur = p;
        hops += 1;
        if hops > 1024 {
            break;
        }
    }
    Ok(Value::FALSE)
}

fn object_proto_property_is_enumerable(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Ok(Value::FALSE);
    };
    let k = cx
        .vm
        .conv_key(args.first().copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let is_enum = cx
        .vm
        .heap
        .get_own(o, k)
        .map_err(heap_to_run)?
        .is_some_and(|p| p.attrs.enumerable());
    Ok(Value::boolean(is_enum))
}

fn symbol_for_static(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let key_s = cx
        .vm
        .conv_string(args.first().copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let pk = cx.vm.heap.key_from_string(key_s).map_err(heap_to_run)?;
    let sym_ctor = cx
        .vm
        .get_global("Symbol")
        .and_then(|v| v.as_object())
        .ok_or_else(|| cx.type_error("Symbol global missing"))?;
    let k_reg = cx
        .vm
        .heap
        .key_from_str("__symbol_registry")
        .map_err(heap_to_run)?;
    let reg_obj = cx
        .vm
        .heap
        .get_own(sym_ctor, k_reg)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_object())
        .ok_or_else(|| cx.type_error("Symbol registry missing"))?;
    if let Some(existing) = cx
        .vm
        .heap
        .get_own(reg_obj, pk)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        && existing.is_symbol()
    {
        return Ok(existing);
    }
    let sym = cx.vm.heap.alloc_symbol(Some(key_s)).map_err(heap_to_run)?;
    let sv = Value::symbol(sym);
    cx.vm
        .heap
        .define_own(reg_obj, pk, Property::data(sv))
        .map_err(heap_to_run)?;
    Ok(sv)
}

fn symbol_key_for_static(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(target_sym) = arg.as_symbol() else {
        return Err(cx.type_error("Symbol.keyFor requires a Symbol argument"));
    };
    let sym_ctor = cx
        .vm
        .get_global("Symbol")
        .and_then(|v| v.as_object())
        .ok_or_else(|| cx.type_error("Symbol global missing"))?;
    let k_reg = cx
        .vm
        .heap
        .key_from_str("__symbol_registry")
        .map_err(heap_to_run)?;
    let Some(reg_obj) = cx
        .vm
        .heap
        .get_own(sym_ctor, k_reg)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_object())
    else {
        return Ok(Value::UNDEFINED);
    };
    for k in cx.vm.heap.own_keys(reg_obj).map_err(heap_to_run)? {
        if let Some(v) = cx
            .vm
            .heap
            .get_own(reg_obj, k)
            .map_err(heap_to_run)?
            .and_then(|p| p.slot.data())
            && v.as_symbol() == Some(target_sym)
        {
            return cx.vm.string(&cx.vm.key_text(k));
        }
    }
    Ok(Value::UNDEFINED)
}

fn symbol_description_getter(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let sym = if let Some(s) = this.as_symbol() {
        s
    } else if let Some(o) = this.as_object()
        && let ObjectClass::Primitive(p) = cx.vm.heap.object(o).map_err(heap_to_run)?.class
        && let Some(s) = p.as_symbol()
    {
        s
    } else {
        return Err(cx.type_error("Symbol.prototype.description requires a Symbol"));
    };
    match cx.vm.heap.symbol_description(sym).map_err(heap_to_run)? {
        Some(s) => Ok(Value::string(s)),
        None => Ok(Value::UNDEFINED),
    }
}

fn weak_map_ctor_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let proto = lookup_global_proto(cx, "WeakMap").or(Some(cx.vm.intr.object_prototype));
    let obj = cx
        .vm
        .heap
        .new_collection(proto, CollectionKind::WeakMap)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(obj));
    if let Some(iter) = args.first().copied()
        && !iter.is_nullish()
    {
        let entries = cx
            .vm
            .spread_values(cx.host, iter)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for entry in entries {
            let Some(eo) = entry.as_object() else {
                return Err(cx.type_error("Iterator value is not an entry object"));
            };
            let k = cx
                .vm
                .read(cx.host, Value::object(eo), PropertyKey::Index(0))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            let v = cx
                .vm
                .read(cx.host, Value::object(eo), PropertyKey::Index(1))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            weak_map_set(cx, Value::object(obj), &[k, v])?;
        }
    }
    Ok(Value::object(obj))
}

fn weak_map_set(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let key = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !key.is_object() && !key.is_symbol() {
        return Err(cx.type_error("Invalid value used as weak map key"));
    }
    map_set(cx, this, args)
}

fn weak_set_ctor_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let proto = lookup_global_proto(cx, "WeakSet").or(Some(cx.vm.intr.object_prototype));
    let obj = cx
        .vm
        .heap
        .new_collection(proto, CollectionKind::WeakSet)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(obj));
    if let Some(iter) = args.first().copied()
        && !iter.is_nullish()
    {
        let items = cx
            .vm
            .spread_values(cx.host, iter)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for item in items {
            weak_set_add(cx, Value::object(obj), &[item])?;
        }
    }
    Ok(Value::object(obj))
}

fn weak_set_add(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !val.is_object() && !val.is_symbol() {
        return Err(cx.type_error("Invalid value used in weak set"));
    }
    set_add(cx, this, args)
}

fn weak_ref_ctor_fn(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !target.is_object() && !target.is_symbol() {
        return Err(cx.type_error("WeakRef target must be an object or symbol"));
    }
    let proto = lookup_global_proto(cx, "WeakRef").or(Some(cx.vm.intr.object_prototype));
    let o = match this.as_object() {
        Some(o) if o != cx.vm.intr.global => o,
        _ => cx.vm.heap.new_object(proto).map_err(heap_to_run)?,
    };
    cx.vm.define_hidden(o, "__weak_target", target, HIDDEN)?;
    Ok(Value::object(o))
}

fn weak_ref_deref(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("WeakRef.prototype.deref called on non-object"));
    };
    let k = cx
        .vm
        .heap
        .key_from_str("__weak_target")
        .map_err(heap_to_run)?;
    Ok(cx
        .vm
        .heap
        .get_own(o, k)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .unwrap_or(Value::UNDEFINED))
}

fn finalization_registry_ctor(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let cb = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !cb
        .as_object()
        .is_some_and(|o| cx.vm.is_callable_obj(o).unwrap_or(false))
    {
        return Err(cx.type_error("FinalizationRegistry requires a callable cleanup callback"));
    }
    let proto =
        lookup_global_proto(cx, "FinalizationRegistry").or(Some(cx.vm.intr.object_prototype));
    let o = match this.as_object() {
        Some(o) if o != cx.vm.intr.global => o,
        _ => cx.vm.heap.new_object(proto).map_err(heap_to_run)?,
    };
    cx.vm.define_hidden(o, "__fr_cb", cb, HIDDEN)?;
    Ok(Value::object(o))
}

fn finalization_registry_register(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !target.is_object() && !target.is_symbol() {
        return Err(
            cx.type_error("FinalizationRegistry.prototype.register target must be an object")
        );
    }
    Ok(Value::UNDEFINED)
}

fn finalization_registry_unregister(
    _cx: &mut NativeCx<'_>,
    _this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    Ok(Value::FALSE)
}

fn parse_bigint_str(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    let clean: String = trimmed.chars().filter(|&c| c != '_').collect();
    let clean = clean.strip_suffix('n').unwrap_or(&clean);
    if let Some(hex) = clean
        .strip_prefix("0x")
        .or_else(|| clean.strip_prefix("0X"))
    {
        return i128::from_str_radix(hex, 16).ok().map(|v| v as i64);
    }
    if let Some(bin) = clean
        .strip_prefix("0b")
        .or_else(|| clean.strip_prefix("0B"))
    {
        return i128::from_str_radix(bin, 2).ok().map(|v| v as i64);
    }
    if let Some(oct) = clean
        .strip_prefix("0o")
        .or_else(|| clean.strip_prefix("0O"))
    {
        return i128::from_str_radix(oct, 8).ok().map(|v| v as i64);
    }
    clean.parse::<i128>().ok().map(|v| v as i64)
}

fn bigint_ctor_fn(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    if let Some(bi) = v.as_bigint() {
        return Ok(Value::bigint(bi));
    }
    if let Some(b) = v.as_bool() {
        return Ok(Value::bigint(if b { 1 } else { 0 }));
    }
    if let Some(n) = v.as_number() {
        if !n.is_finite() || n.fract() != 0.0 {
            return Err(cx.range_error(
                "The number cannot be converted to a BigInt because it is not an integer",
            ));
        }
        return Ok(Value::bigint(n as i64));
    }
    let s = to_str_rust(cx, v)?;
    let Some(i) = parse_bigint_str(&s) else {
        return Err(cx.vm.make_thrown(
            ErrorKind::Syntax,
            &format!("Cannot convert {s} to a BigInt"),
        ));
    };
    Ok(Value::bigint(i))
}

fn bigint_as_int_n(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let bits = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::UNDEFINED))? as u32;
    let bi = bigint_ctor_fn(cx, Value::UNDEFINED, &args[1..])?
        .as_bigint()
        .unwrap_or(0);
    if bits == 0 {
        return Ok(Value::bigint(0));
    }
    if bits >= 64 {
        return Ok(Value::bigint(bi));
    }
    let shift = 64 - bits;
    Ok(Value::bigint((bi << shift) >> shift))
}

fn bigint_as_uint_n(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let bits = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::UNDEFINED))? as u32;
    let bi = bigint_ctor_fn(cx, Value::UNDEFINED, &args[1..])?
        .as_bigint()
        .unwrap_or(0);
    if bits == 0 {
        return Ok(Value::bigint(0));
    }
    if bits >= 64 {
        return Ok(Value::bigint(bi));
    }
    let mask = (1u64 << bits) - 1;
    Ok(Value::bigint(((bi as u64) & mask) as i64))
}

fn aggregate_error_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let errors_arg = args.first().copied().unwrap_or(Value::UNDEFINED);
    let msg_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let opts_arg = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    let list = cx.vm.spread_values(cx.host, errors_arg).unwrap_or_default();
    let errs_arr = make_array_from_slice(cx, &list)?;
    cx.keep(errs_arr);
    let msg_str = if msg_arg.is_undefined() {
        String::new()
    } else {
        to_str_rust(cx, msg_arg)?
    };
    let e = cx.vm.make_error(ErrorKind::Error, &msg_str)?;
    if let Some(o) = e.as_object() {
        let name_v = cx.vm.string("AggregateError")?;
        cx.vm.define_hidden(o, "name", name_v, HIDDEN)?;
        cx.vm.define_hidden(o, "errors", errs_arr, HIDDEN)?;
        if let Some(opt_obj) = opts_arg.as_object() {
            let k_cause = cx.vm.heap.key_from_str("cause").map_err(heap_to_run)?;
            if let crate::anvil_js::gc::Lookup::Found { .. } =
                cx.vm.heap.lookup(opt_obj, k_cause).map_err(heap_to_run)?
            {
                let c = cx
                    .vm
                    .read(cx.host, opts_arg, k_cause)
                    .map_err(|err| cx.vm.exit_to_run(err))?;
                cx.vm.define_hidden(o, "cause", c, HIDDEN)?;
            }
        }
    }
    Ok(e)
}

fn proxy_ctor_fn(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let Some(target) = args.first().and_then(|v| v.as_object()) else {
        return Err(cx.type_error("Cannot create proxy with a non-object as target or handler"));
    };
    let Some(handler) = args.get(1).and_then(|v| v.as_object()) else {
        return Err(cx.type_error("Cannot create proxy with a non-object as target or handler"));
    };
    let proxy = cx
        .vm
        .heap
        .alloc_object(
            Some(cx.vm.intr.object_prototype),
            ObjectClass::Proxy {
                target: Some(target),
                handler: Some(handler),
            },
        )
        .map_err(heap_to_run)?;
    Ok(Value::object(proxy))
}

fn proxy_revoke_bound(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    if let Some(po) = this.as_object()
        && let Ok(ObjectClass::Proxy { target, handler }) = cx.vm.heap.class_mut(po)
    {
        *target = None;
        *handler = None;
    }
    Ok(Value::UNDEFINED)
}

fn proxy_revocable_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let proxy = proxy_ctor_fn(cx, Value::UNDEFINED, args)?;
    cx.keep(proxy);
    let rev_raw = cx.vm.register_native("revoke", proxy_revoke_bound)?;
    let revoke = function_bind(cx, rev_raw, &[proxy])?;
    cx.keep(revoke);
    let out = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.vm.define_hidden(out, "proxy", proxy, Attrs::DEFAULT)?;
    cx.vm.define_hidden(out, "revoke", revoke, Attrs::DEFAULT)?;
    Ok(Value::object(out))
}

fn reflect_get(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !target.is_object() {
        return Err(cx.type_error("Reflect.get called on non-object"));
    }
    let key = cx
        .vm
        .conv_key(args.get(1).copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    cx.vm
        .read(cx.host, target, key)
        .map_err(|e| cx.vm.exit_to_run(e))
}

fn reflect_set(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !target.is_object() {
        return Err(cx.type_error("Reflect.set called on non-object"));
    }
    let key = cx
        .vm
        .conv_key(args.get(1).copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let val = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    match cx.vm.set_op(cx.host, target, key, val, None) {
        Ok(()) => Ok(Value::TRUE),
        Err(_) => Ok(Value::FALSE),
    }
}

fn reflect_has(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(mut o) = target.as_object() else {
        return Err(cx.type_error("Reflect.has called on non-object"));
    };
    let key_v = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let key = cx.vm.conv_key(key_v).map_err(|e| cx.vm.exit_to_run(e))?;
    if let Some((t, h)) = cx
        .vm
        .proxy_parts(o, "has")
        .map_err(|e| cx.vm.exit_to_run(e))?
    {
        if let Some(trap) = cx
            .vm
            .proxy_trap(h, "has")
            .map_err(|e| cx.vm.exit_to_run(e))?
        {
            let kv = cx
                .vm
                .prop_key_value(key)
                .map_err(|e| cx.vm.exit_to_run(e))?;
            let r = cx.call(trap, Value::object(h), &[Value::object(t), kv])?;
            return Ok(Value::boolean(
                cx.vm.truthy(r).map_err(|e| cx.vm.exit_to_run(e))?,
            ));
        }
        o = t;
    }
    if key == cx.vm.intr.key_length && cx.vm.heap.object(o).map_err(heap_to_run)?.is_array() {
        return Ok(Value::TRUE);
    }
    let found = matches!(
        cx.vm.heap.lookup(o, key).map_err(heap_to_run)?,
        crate::anvil_js::gc::Lookup::Found { .. }
    );
    Ok(Value::boolean(found))
}

fn reflect_delete_property(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if !target.is_object() {
        return Err(cx.type_error("Reflect.deleteProperty called on non-object"));
    }
    let key = cx
        .vm
        .conv_key(args.get(1).copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    match cx.vm.delete_op(cx.host, target, key) {
        Ok(b) => Ok(Value::boolean(b)),
        Err(_) => Ok(Value::FALSE),
    }
}

fn reflect_own_keys(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(mut o) = target.as_object() else {
        return Err(cx.type_error("Reflect.ownKeys called on non-object"));
    };
    if let Some((t, h)) = cx
        .vm
        .proxy_parts(o, "ownKeys")
        .map_err(|e| cx.vm.exit_to_run(e))?
    {
        if let Some(trap) = cx
            .vm
            .proxy_trap(h, "ownKeys")
            .map_err(|e| cx.vm.exit_to_run(e))?
        {
            return cx.call(trap, Value::object(h), &[Value::object(t)]);
        }
        o = t;
    }
    let keys = cx.vm.heap.own_keys(o).map_err(heap_to_run)?;
    let mut out = Vec::with_capacity(keys.len());
    for k in keys {
        out.push(cx.vm.prop_key_value(k).map_err(|e| cx.vm.exit_to_run(e))?);
    }
    make_array_from_slice(cx, &out)
}

fn reflect_apply(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let args_list = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    super::builtins::function_apply(cx, target, &[this_arg, args_list])
}

fn reflect_construct(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let args_list = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let new_target = args.get(2).copied().unwrap_or(target);
    let Some(to) = target.as_object() else {
        return Err(cx.type_error("Reflect.construct target is not a constructor"));
    };
    let argv = if args_list.is_nullish() {
        Vec::new()
    } else {
        cx.vm
            .spread_values(cx.host, args_list)
            .map_err(|e| cx.vm.exit_to_run(e))?
    };
    for v in &argv {
        cx.keep(*v);
    }
    let proto_holder = new_target.as_object().unwrap_or(to);
    let proto = cx
        .vm
        .function_prototype_property(proto_holder)
        .map_err(|e| cx.vm.exit_to_run(e))?
        .as_object()
        .unwrap_or(cx.vm.intr.object_prototype);
    let instance = cx.vm.heap.new_object(Some(proto)).map_err(heap_to_run)?;
    cx.keep(Value::object(instance));
    let res = cx.call(target, Value::object(instance), &argv)?;
    Ok(if res.is_object() {
        res
    } else {
        Value::object(instance)
    })
}

fn reflect_set_prototype_of(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    match object_set_prototype_of(cx, Value::UNDEFINED, args) {
        Ok(_) => Ok(Value::TRUE),
        Err(_) => Ok(Value::FALSE),
    }
}

fn reflect_define_property(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    match object_define_property(cx, Value::UNDEFINED, args) {
        Ok(_) => Ok(Value::TRUE),
        Err(_) => Ok(Value::FALSE),
    }
}

fn reflect_prevent_extensions(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let Some(o) = target.as_object() else {
        return Err(cx.type_error("Reflect.preventExtensions called on non-object"));
    };
    cx.vm.heap.prevent_extensions(o).map_err(heap_to_run)?;
    Ok(Value::TRUE)
}

fn intl_resolved_options(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let out = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    let locale = if let Some(o) = this.as_object() {
        let k = cx.vm.heap.key_from_str("__locale").map_err(heap_to_run)?;
        cx.vm
            .heap
            .get_own(o, k)
            .map_err(heap_to_run)?
            .and_then(|p| p.slot.data())
    } else {
        None
    };
    let loc_v = match locale {
        Some(v) if v.is_string() => v,
        _ => cx.vm.string("en-US")?,
    };
    cx.vm.define_hidden(out, "locale", loc_v, Attrs::DEFAULT)?;
    let tz = cx.vm.string("UTC")?;
    cx.vm.define_hidden(out, "timeZone", tz, Attrs::DEFAULT)?;
    Ok(Value::object(out))
}

fn intl_extract_locale(cx: &mut NativeCx<'_>, args: &[Value]) -> Result<Value, RunError> {
    if let Some(first) = args.first().copied()
        && !first.is_undefined()
    {
        if first.is_string() {
            return Ok(first);
        }
        if let Some(o) = first.as_object()
            && cx.vm.heap.object(o).map_err(heap_to_run)?.is_array()
            && let Ok(v) = cx.vm.read(cx.host, first, PropertyKey::Index(0))
            && v.is_string()
        {
            return Ok(v);
        }
    }
    cx.vm.string("en-US")
}

fn intl_date_format_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    let ms = if let Some(o) = v.as_object() {
        date_get_time(cx, Value::object(o), &[])?
            .as_number()
            .unwrap_or(0.0)
    } else if let Some(n) = v.as_number() {
        n
    } else {
        cx.host.now_ms()
    };
    let days = (ms / 86_400_000.0).floor() as i64;
    let year = 1970 + (days / 365);
    cx.vm.string(&format!("1/1/{year}"))
}

fn intl_date_time_format_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let o = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    let loc = intl_extract_locale(cx, args)?;
    cx.vm.define_hidden(o, "__locale", loc, HIDDEN)?;
    cx.vm.def_method(o, "format", intl_date_format_fn)?;
    cx.vm
        .def_method(o, "resolvedOptions", intl_resolved_options)?;
    Ok(Value::object(o))
}

fn intl_number_format_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    if !n.is_finite() {
        return cx.vm.string(&number_to_string(n));
    }
    let sign = if n < 0.0 { "-" } else { "" };
    let abs = n.abs();
    let int_part = abs.trunc() as u128;
    let frac = abs.fract();
    let grouped = crate::anvil_js::limits::group_digits(int_part as u64);
    if frac == 0.0 {
        cx.vm.string(&format!("{sign}{grouped}"))
    } else {
        let frac_s = format!("{frac:.3}");
        let trimmed = frac_s
            .trim_start_matches('0')
            .trim_end_matches('0')
            .trim_end_matches('.');
        cx.vm.string(&format!("{sign}{grouped}{trimmed}"))
    }
}

fn intl_number_format_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let o = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    let loc = intl_extract_locale(cx, args)?;
    cx.vm.define_hidden(o, "__locale", loc, HIDDEN)?;
    cx.vm.def_method(o, "format", intl_number_format_fn)?;
    cx.vm
        .def_method(o, "resolvedOptions", intl_resolved_options)?;
    Ok(Value::object(o))
}

fn intl_collator_compare_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let a = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let b = to_str_rust(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let ord = match a.cmp(&b) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    };
    Ok(Value::number(ord))
}

fn intl_collator_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let o = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    let loc = intl_extract_locale(cx, args)?;
    cx.vm.define_hidden(o, "__locale", loc, HIDDEN)?;
    cx.vm.def_method(o, "compare", intl_collator_compare_fn)?;
    cx.vm
        .def_method(o, "resolvedOptions", intl_resolved_options)?;
    Ok(Value::object(o))
}

fn intl_plural_select_fn(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let n = to_num(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    cx.vm.string(if (n - 1.0).abs() < f64::EPSILON {
        "one"
    } else {
        "other"
    })
}

fn intl_plural_rules_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let o = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    let loc = intl_extract_locale(cx, args)?;
    cx.vm.define_hidden(o, "__locale", loc, HIDDEN)?;
    cx.vm.def_method(o, "select", intl_plural_select_fn)?;
    cx.vm
        .def_method(o, "resolvedOptions", intl_resolved_options)?;
    Ok(Value::object(o))
}

fn anvil_yield_helper(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let delegate = args.get(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let yields_arr = args
        .get(2)
        .and_then(|v| v.as_object())
        .ok_or_else(|| cx.type_error("invalid generator state"))?;
    let cur_len = cx.vm.heap.array_length(yields_arr).map_err(heap_to_run)?;
    if cur_len >= 1024 {
        let stop = cx.vm.string("__ANVIL_GEN_LIMIT__")?;
        return Err(cx.vm.exit_to_run(Exit::Throw {
            value: stop,
            stack: None,
        }));
    }
    if delegate {
        let items = cx
            .vm
            .spread_values(cx.host, val)
            .map_err(|e| cx.vm.exit_to_run(e))?;
        for item in items {
            cx.vm
                .array_push(yields_arr, item)
                .map_err(|e| cx.vm.exit_to_run(e))?;
        }
        return Ok(Value::UNDEFINED);
    }
    cx.vm
        .array_push(yields_arr, val)
        .map_err(|e| cx.vm.exit_to_run(e))?;
    Ok(val)
}

fn generator_next_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Generator.prototype.next called on non-object"));
    };
    let k_idx = cx.vm.heap.key_from_str("__gen_idx").map_err(heap_to_run)?;
    let k_done = cx.vm.heap.key_from_str("__gen_done").map_err(heap_to_run)?;
    let k_items = cx
        .vm
        .heap
        .key_from_str("__gen_items")
        .map_err(heap_to_run)?;
    let k_ret = cx.vm.heap.key_from_str("__gen_ret").map_err(heap_to_run)?;
    let is_done = cx
        .vm
        .heap
        .get_own(o, k_done)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_done {
        return make_iter_result(cx, Value::UNDEFINED, true);
    }
    let idx = cx
        .vm
        .heap
        .get_own(o, k_idx)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_int32())
        .unwrap_or(0) as u32;
    let items_obj = cx
        .vm
        .heap
        .get_own(o, k_items)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_object());
    if let Some(arr) = items_obj {
        let len = cx.vm.heap.array_length(arr).map_err(heap_to_run)?;
        if idx < len {
            let item = cx
                .vm
                .read(cx.host, Value::object(arr), PropertyKey::Index(idx))
                .map_err(|e| cx.vm.exit_to_run(e))?;
            cx.vm
                .define_hidden(o, "__gen_idx", Value::int32((idx + 1) as i32), HIDDEN)?;
            return make_iter_result(cx, item, false);
        }
    }
    cx.vm.define_hidden(o, "__gen_done", Value::TRUE, HIDDEN)?;
    let ret = cx
        .vm
        .heap
        .get_own(o, k_ret)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .unwrap_or(Value::UNDEFINED);
    make_iter_result(cx, ret, true)
}

fn generator_return_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    if let Some(o) = this.as_object() {
        cx.vm.define_hidden(o, "__gen_done", Value::TRUE, HIDDEN)?;
    }
    let v = args.first().copied().unwrap_or(Value::UNDEFINED);
    make_iter_result(cx, v, true)
}

fn generator_throw_method(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    if let Some(o) = this.as_object() {
        cx.vm.define_hidden(o, "__gen_done", Value::TRUE, HIDDEN)?;
    }
    let err = args.first().copied().unwrap_or(Value::UNDEFINED);
    Err(cx.vm.exit_to_run(Exit::Throw {
        value: err,
        stack: None,
    }))
}

fn generator_self_iterator(
    _cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    Ok(this)
}

fn anvil_make_generator_helper(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let body_fn = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let yields_arr = cx
        .vm
        .heap
        .new_array(Some(cx.vm.intr.array_prototype))
        .map_err(heap_to_run)?;
    let yields_val = Value::object(yields_s_ref(yields_arr));
    cx.keep(yields_val);
    let ret_val = match cx.call(body_fn, this_arg, &[yields_val]) {
        Ok(v) => v,
        Err(RunError::Thrown { value, .. }) => {
            if let Some(s) = value.as_string()
                && cx.vm.heap.string_to_rust(s).ok().as_deref() == Some("__ANVIL_GEN_LIMIT__")
            {
                Value::UNDEFINED
            } else {
                return Err(cx.vm.exit_to_run(Exit::Throw { value, stack: None }));
            }
        }
        Err(other) => return Err(other),
    };
    cx.keep(ret_val);
    let gen_obj = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    cx.keep(Value::object(gen_obj));
    cx.vm
        .define_hidden(gen_obj, "__gen_items", yields_val, HIDDEN)?;
    cx.vm.define_hidden(gen_obj, "__gen_ret", ret_val, HIDDEN)?;
    cx.vm
        .define_hidden(gen_obj, "__gen_idx", Value::int32(0), HIDDEN)?;
    cx.vm
        .define_hidden(gen_obj, "__gen_done", Value::FALSE, HIDDEN)?;
    cx.vm.def_method(gen_obj, "next", generator_next_method)?;
    cx.vm
        .def_method(gen_obj, "return", generator_return_method)?;
    cx.vm.def_method(gen_obj, "throw", generator_throw_method)?;
    if let Some(k_sym) = cx.vm.symbol_iterator_key() {
        let self_fn = cx
            .vm
            .register_native("[Symbol.iterator]", generator_self_iterator)?;
        cx.vm
            .heap
            .define_own(
                gen_obj,
                k_sym,
                Property {
                    attrs: HIDDEN,
                    slot: Slot::Data(self_fn),
                },
            )
            .map_err(heap_to_run)?;
    }
    Ok(Value::object(gen_obj))
}

#[inline]
fn yields_s_ref(r: ObjectRef) -> ObjectRef {
    r
}

fn anvil_import_module_helper(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let spec = to_str_rust(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let k_mods = cx
        .vm
        .heap
        .key_from_str("__anvil_modules")
        .map_err(heap_to_run)?;
    if let Some(mods_obj) = cx
        .vm
        .heap
        .get_own(cx.vm.intr.global, k_mods)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_object())
    {
        let pk = cx.vm.heap.key_from_str(&spec).map_err(heap_to_run)?;
        if let Some(ns) = cx
            .vm
            .heap
            .get_own(mods_obj, pk)
            .map_err(heap_to_run)?
            .and_then(|p| p.slot.data())
            && ns.is_object()
        {
            return Ok(ns);
        }
    }
    let k_exp = cx
        .vm
        .heap
        .key_from_str("__anvil_exports")
        .map_err(heap_to_run)?;
    if let Some(exp) = cx
        .vm
        .heap
        .get_own(cx.vm.intr.global, k_exp)
        .map_err(heap_to_run)?
        .and_then(|p| p.slot.data())
        && exp.is_object()
    {
        return Ok(exp);
    }
    let empty = cx
        .vm
        .heap
        .new_object(Some(cx.vm.intr.object_prototype))
        .map_err(heap_to_run)?;
    Ok(Value::object(empty))
}
