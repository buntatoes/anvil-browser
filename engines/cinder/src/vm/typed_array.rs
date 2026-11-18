// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Built-in ECMAScript `ArrayBuffer`, TypedArrays (`Int8Array`..`BigUint64Array`),
//! and `DataView`.

use super::host::{NativeCx, NativeFn};
use super::stdlib::{make_list_iterator, read_array_items, to_int_or_inf, to_num, to_str_rust};
use super::{ErrorKind, Exit, RunError, Vm, heap_to_run, type_err};
use crate::value::ObjectRef;
use crate::value::Value;
use crate::value::bigint::JsBigInt;
use crate::value::number::to_int32;
use crate::value::object::{IterKind, ObjectClass, TypedArrayKind};
use crate::value::shape::Attrs;

const HIDDEN: Attrs = Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE);
const FIXED: Attrs = Attrs(0);
const MAX_ARRAY_BUFFER_BYTES: usize = 256 * 1024 * 1024;

const ALL_KINDS: [TypedArrayKind; 11] = [
    TypedArrayKind::Int8,
    TypedArrayKind::Uint8,
    TypedArrayKind::Uint8Clamped,
    TypedArrayKind::Int16,
    TypedArrayKind::Uint16,
    TypedArrayKind::Int32,
    TypedArrayKind::Uint32,
    TypedArrayKind::Float32,
    TypedArrayKind::Float64,
    TypedArrayKind::BigInt64,
    TypedArrayKind::BigUint64,
];

#[derive(Debug, Default)]
pub(crate) struct TypedArrayIntrinsics {
    pub array_buffer_proto: Option<ObjectRef>,
    pub data_view_proto: Option<ObjectRef>,
    pub typed_base_proto: Option<ObjectRef>,
    pub protos: [Option<ObjectRef>; 11],
    pub ctors: [Option<ObjectRef>; 11],
}

impl TypedArrayIntrinsics {
    pub(crate) fn trace(&self, roots: &mut Vec<Value>) {
        if let Some(o) = self.array_buffer_proto {
            roots.push(Value::object(o));
        }
        if let Some(o) = self.data_view_proto {
            roots.push(Value::object(o));
        }
        if let Some(o) = self.typed_base_proto {
            roots.push(Value::object(o));
        }
        for o in self.protos.iter().flatten() {
            roots.push(Value::object(*o));
        }
        for o in self.ctors.iter().flatten() {
            roots.push(Value::object(*o));
        }
    }

    pub(crate) fn proto_for(&self, kind: TypedArrayKind) -> Option<ObjectRef> {
        self.protos[kind_index(kind)]
    }

    pub(crate) fn kind_for_ctor(&self, ctor: ObjectRef) -> Option<TypedArrayKind> {
        for (i, &c) in self.ctors.iter().enumerate() {
            if c == Some(ctor) {
                return Some(ALL_KINDS[i]);
            }
        }
        None
    }
}

const fn kind_index(kind: TypedArrayKind) -> usize {
    match kind {
        TypedArrayKind::Int8 => 0,
        TypedArrayKind::Uint8 => 1,
        TypedArrayKind::Uint8Clamped => 2,
        TypedArrayKind::Int16 => 3,
        TypedArrayKind::Uint16 => 4,
        TypedArrayKind::Int32 => 5,
        TypedArrayKind::Uint32 => 6,
        TypedArrayKind::Float32 => 7,
        TypedArrayKind::Float64 => 8,
        TypedArrayKind::BigInt64 => 9,
        TypedArrayKind::BigUint64 => 10,
    }
}

fn ctor_for_kind(kind: TypedArrayKind) -> NativeFn {
    match kind {
        TypedArrayKind::Int8 => ta_ctor_i8,
        TypedArrayKind::Uint8 => ta_ctor_u8,
        TypedArrayKind::Uint8Clamped => ta_ctor_u8c,
        TypedArrayKind::Int16 => ta_ctor_i16,
        TypedArrayKind::Uint16 => ta_ctor_u16,
        TypedArrayKind::Int32 => ta_ctor_i32,
        TypedArrayKind::Uint32 => ta_ctor_u32,
        TypedArrayKind::Float32 => ta_ctor_f32,
        TypedArrayKind::Float64 => ta_ctor_f64,
        TypedArrayKind::BigInt64 => ta_ctor_bi64,
        TypedArrayKind::BigUint64 => ta_ctor_bu64,
    }
}

fn ta_ctor_i8(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Int8, args)
}
fn ta_ctor_u8(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Uint8, args)
}
fn ta_ctor_u8c(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Uint8Clamped, args)
}
fn ta_ctor_i16(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Int16, args)
}
fn ta_ctor_u16(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Uint16, args)
}
fn ta_ctor_i32(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Int32, args)
}
fn ta_ctor_u32(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Uint32, args)
}
fn ta_ctor_f32(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Float32, args)
}
fn ta_ctor_f64(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::Float64, args)
}
fn ta_ctor_bi64(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::BigInt64, args)
}
fn ta_ctor_bu64(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_construct(cx, TypedArrayKind::BigUint64, args)
}

impl Vm {
    pub(super) fn install_typed_array_builtins(&mut self) -> Result<(), RunError> {
        let g = self.intr.global;

        // ---- ArrayBuffer ----
        let ab_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.typed_arrays.array_buffer_proto = Some(ab_proto);
        let ab_ctor = self.register_native("ArrayBuffer", array_buffer_ctor)?;
        let abco = ab_ctor.as_object().unwrap();
        self.define_hidden(abco, "prototype", Value::object(ab_proto), FIXED)?;
        self.define_hidden(ab_proto, "constructor", ab_ctor, HIDDEN)?;
        self.define_accessor(ab_proto, "byteLength", Some(ab_byte_length_get), None)?;
        self.def_method(ab_proto, "slice", ab_slice)?;
        self.def_method(abco, "isView", ab_is_view)?;
        if let Some(k_tag) = self.symbol_well_known_key("toStringTag") {
            let tag = self.string("ArrayBuffer")?;
            self.heap
                .define_own(
                    ab_proto,
                    k_tag,
                    crate::value::object::Property {
                        attrs: Attrs(Attrs::CONFIGURABLE),
                        slot: crate::value::object::Slot::Data(tag),
                    },
                )
                .map_err(heap_to_run)?;
        }
        self.define_hidden(g, "ArrayBuffer", ab_ctor, HIDDEN)?;

        // ---- %TypedArray%.prototype ----
        let base_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.typed_arrays.typed_base_proto = Some(base_proto);
        self.define_accessor(base_proto, "buffer", Some(ta_buffer_get), None)?;
        self.define_accessor(base_proto, "byteLength", Some(ta_byte_length_get), None)?;
        self.define_accessor(base_proto, "byteOffset", Some(ta_byte_offset_get), None)?;
        self.define_accessor(base_proto, "length", Some(ta_length_get), None)?;
        if let Some(k_tag) = self.symbol_well_known_key("toStringTag") {
            let getter = self.register_native("get [Symbol.toStringTag]", ta_to_string_tag_get)?;
            self.heap
                .define_own(
                    base_proto,
                    k_tag,
                    crate::value::object::Property {
                        attrs: Attrs(Attrs::CONFIGURABLE),
                        slot: crate::value::object::Slot::Accessor {
                            get: getter,
                            set: Value::UNDEFINED,
                        },
                    },
                )
                .map_err(heap_to_run)?;
        }

        for (name, f) in [
            ("subarray", ta_subarray as NativeFn),
            ("slice", ta_slice),
            ("set", ta_set),
            ("fill", ta_fill),
            ("copyWithin", ta_copy_within),
            ("reverse", ta_reverse),
            ("sort", ta_sort),
            ("indexOf", ta_index_of),
            ("lastIndexOf", ta_last_index_of),
            ("includes", ta_includes),
            ("find", ta_find),
            ("findIndex", ta_find_index),
            ("findLast", ta_find_last),
            ("findLastIndex", ta_find_last_index),
            ("every", ta_every),
            ("some", ta_some),
            ("forEach", ta_for_each),
            ("map", ta_map),
            ("filter", ta_filter),
            ("reduce", ta_reduce),
            ("reduceRight", ta_reduce_right),
            ("join", ta_join),
            ("toString", ta_to_string),
            ("at", ta_at),
            ("with", ta_with),
            ("toReversed", ta_to_reversed),
            ("toSorted", ta_to_sorted),
            ("keys", ta_keys),
            ("values", ta_values),
            ("entries", ta_entries),
        ] {
            self.def_method(base_proto, name, f)?;
        }
        if let Some(k_iter) = self.symbol_iterator_key() {
            let values_fn = self.register_native("values", ta_values)?;
            self.heap
                .define_own(
                    base_proto,
                    k_iter,
                    crate::value::object::Property {
                        attrs: HIDDEN,
                        slot: crate::value::object::Slot::Data(values_fn),
                    },
                )
                .map_err(heap_to_run)?;
        }

        // ---- 11 Concrete TypedArray Constructors ----
        for kind in ALL_KINDS {
            let name = kind.ctor_name();
            let proto = self
                .heap
                .new_object(Some(base_proto))
                .map_err(heap_to_run)?;
            let ctor = self.register_native(name, ctor_for_kind(kind))?;
            let co = ctor.as_object().unwrap();
            let bpe = Value::number(kind.elem_size() as f64);
            self.define_hidden(co, "prototype", Value::object(proto), FIXED)?;
            self.define_hidden(proto, "constructor", ctor, HIDDEN)?;
            self.define_hidden(co, "BYTES_PER_ELEMENT", bpe, FIXED)?;
            self.define_hidden(proto, "BYTES_PER_ELEMENT", bpe, FIXED)?;
            self.def_method(co, "from", ta_from)?;
            self.def_method(co, "of", ta_of)?;
            self.define_hidden(g, name, ctor, HIDDEN)?;

            let idx = kind_index(kind);
            self.typed_arrays.protos[idx] = Some(proto);
            self.typed_arrays.ctors[idx] = Some(co);
        }

        // ---- DataView ----
        let dv_proto = self
            .heap
            .new_object(Some(self.intr.object_prototype))
            .map_err(heap_to_run)?;
        self.typed_arrays.data_view_proto = Some(dv_proto);
        let dv_ctor = self.register_native("DataView", data_view_ctor)?;
        let dvco = dv_ctor.as_object().unwrap();
        self.define_hidden(dvco, "prototype", Value::object(dv_proto), FIXED)?;
        self.define_hidden(dv_proto, "constructor", dv_ctor, HIDDEN)?;
        self.define_accessor(dv_proto, "buffer", Some(dv_buffer_get), None)?;
        self.define_accessor(dv_proto, "byteLength", Some(dv_byte_length_get), None)?;
        self.define_accessor(dv_proto, "byteOffset", Some(dv_byte_offset_get), None)?;
        for (name, f) in [
            ("getInt8", dv_get_i8 as NativeFn),
            ("setInt8", dv_set_i8),
            ("getUint8", dv_get_u8),
            ("setUint8", dv_set_u8),
            ("getInt16", dv_get_i16),
            ("setInt16", dv_set_i16),
            ("getUint16", dv_get_u16),
            ("setUint16", dv_set_u16),
            ("getInt32", dv_get_i32),
            ("setInt32", dv_set_i32),
            ("getUint32", dv_get_u32),
            ("setUint32", dv_set_u32),
            ("getFloat32", dv_get_f32),
            ("setFloat32", dv_set_f32),
            ("getFloat64", dv_get_f64),
            ("setFloat64", dv_set_f64),
            ("getBigInt64", dv_get_bi64),
            ("setBigInt64", dv_set_bi64),
            ("getBigUint64", dv_get_bu64),
            ("setBigUint64", dv_set_bu64),
        ] {
            self.def_method(dv_proto, name, f)?;
        }
        if let Some(k_tag) = self.symbol_well_known_key("toStringTag") {
            let tag = self.string("DataView")?;
            self.heap
                .define_own(
                    dv_proto,
                    k_tag,
                    crate::value::object::Property {
                        attrs: Attrs(Attrs::CONFIGURABLE),
                        slot: crate::value::object::Slot::Data(tag),
                    },
                )
                .map_err(heap_to_run)?;
        }
        self.define_hidden(g, "DataView", dv_ctor, HIDDEN)?;
        Ok(())
    }
}

// ---- Low-level Element Read/Write Helpers --------------------------------

fn to_uint8_clamped(n: f64) -> u8 {
    if n.is_nan() || n <= 0.0 {
        return 0;
    }
    if n >= 255.0 {
        return 255;
    }
    let f = n.floor();
    let val = if f + 0.5 < n {
        f + 1.0
    } else if n < f + 0.5 {
        f
    } else if (f as u32) & 1 == 1 {
        f + 1.0
    } else {
        f
    };
    val as u8
}

fn read_buf_bytes(vm: &Vm, buffer: ObjectRef, start: usize, len: usize) -> Result<&[u8], Exit> {
    let ObjectClass::ArrayBuffer(bytes) = &vm.heap.object(buffer)?.class else {
        return Err(type_err("Invalid ArrayBuffer backing store"));
    };
    Ok(bytes.get(start..start + len).unwrap_or(&[]))
}

fn decode_elem_from_slice(vm: &mut Vm, kind: TypedArrayKind, slice: &[u8]) -> Result<Value, Exit> {
    if slice.len() < kind.elem_size() {
        return Ok(Value::UNDEFINED);
    }
    Ok(match kind {
        TypedArrayKind::Int8 => Value::number(f64::from(slice[0] as i8)),
        TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => Value::number(f64::from(slice[0])),
        TypedArrayKind::Int16 => Value::number(f64::from(i16::from_le_bytes([slice[0], slice[1]]))),
        TypedArrayKind::Uint16 => {
            Value::number(f64::from(u16::from_le_bytes([slice[0], slice[1]])))
        }
        TypedArrayKind::Int32 => Value::number(f64::from(i32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ]))),
        TypedArrayKind::Uint32 => Value::number(f64::from(u32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ]))),
        TypedArrayKind::Float32 => Value::number(f64::from(f32::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3],
        ]))),
        TypedArrayKind::Float64 => Value::number(f64::from_le_bytes([
            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
        ])),
        TypedArrayKind::BigInt64 => {
            let v = i64::from_le_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]);
            vm.alloc_bigint_val(JsBigInt::from_i64(v))?
        }
        TypedArrayKind::BigUint64 => {
            let v = u64::from_le_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]);
            vm.alloc_bigint_val(JsBigInt::from_u64(v))?
        }
    })
}

pub(crate) fn read_elem(
    vm: &mut Vm,
    kind: TypedArrayKind,
    buffer: ObjectRef,
    byte_offset: u32,
    index: u32,
) -> Result<Value, Exit> {
    let size = kind.elem_size();
    let pos = byte_offset as usize + (index as usize) * size;
    let slice = read_buf_bytes(vm, buffer, pos, size)?.to_vec();
    decode_elem_from_slice(vm, kind, &slice)
}

pub(crate) fn to_bigint_val(vm: &mut Vm, v: Value) -> Result<JsBigInt, Exit> {
    if v.is_bigint() {
        return vm
            .resolve_bigint(v)?
            .ok_or_else(|| type_err("Cannot convert value to a BigInt"));
    }
    if let Some(b) = v.as_bool() {
        return Ok(JsBigInt::from_i64(if b { 1 } else { 0 }));
    }
    if let Some(s) = v.as_string() {
        let text = vm.heap.string_to_rust(s)?;
        return JsBigInt::parse(&text).ok_or_else(|| {
            Exit::Pending(
                ErrorKind::Syntax,
                format!("Cannot convert {text} to a BigInt"),
            )
        });
    }
    if let Some(o) = v.as_object()
        && let ObjectClass::Primitive(p) = vm.heap.object(o)?.class
    {
        return to_bigint_val(vm, p);
    }
    Err(type_err("Cannot convert value to a BigInt"))
}

pub(crate) fn write_elem(
    vm: &mut Vm,
    kind: TypedArrayKind,
    buffer: ObjectRef,
    byte_offset: u32,
    length: u32,
    index: u32,
    val: Value,
) -> Result<(), Exit> {
    let size = kind.elem_size();
    let bytes: [u8; 8] = if kind.is_bigint() {
        let bi = to_bigint_val(vm, val)?;
        bi.to_u64_wrapping().to_le_bytes()
    } else {
        let n = vm.conv_number(val)?;
        match kind {
            TypedArrayKind::Int8 => {
                let mut b = [0u8; 8];
                b[0] = (to_int32(n) as i8) as u8;
                b
            }
            TypedArrayKind::Uint8 => {
                let mut b = [0u8; 8];
                b[0] = (to_int32(n) as u32) as u8;
                b
            }
            TypedArrayKind::Uint8Clamped => {
                let mut b = [0u8; 8];
                b[0] = to_uint8_clamped(n);
                b
            }
            TypedArrayKind::Int16 => {
                let raw = (to_int32(n) as i16).to_le_bytes();
                let mut b = [0u8; 8];
                b[..2].copy_from_slice(&raw);
                b
            }
            TypedArrayKind::Uint16 => {
                let raw = ((to_int32(n) as u32) as u16).to_le_bytes();
                let mut b = [0u8; 8];
                b[..2].copy_from_slice(&raw);
                b
            }
            TypedArrayKind::Int32 => {
                let raw = to_int32(n).to_le_bytes();
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&raw);
                b
            }
            TypedArrayKind::Uint32 => {
                let raw = crate::value::number::to_uint32(n).to_le_bytes();
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&raw);
                b
            }
            TypedArrayKind::Float32 => {
                let raw = (n as f32).to_le_bytes();
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&raw);
                b
            }
            TypedArrayKind::Float64 => n.to_le_bytes(),
            TypedArrayKind::BigInt64 | TypedArrayKind::BigUint64 => unreachable!(),
        }
    };
    if index >= length {
        return Ok(());
    }
    let pos = byte_offset as usize + (index as usize) * size;
    if let ObjectClass::ArrayBuffer(buf) = vm.heap.class_mut(buffer)?
        && let Some(dst) = buf.get_mut(pos..pos + size)
    {
        dst.copy_from_slice(&bytes[..size]);
    }
    Ok(())
}

// ---- ArrayBuffer Natives --------------------------------------------------

fn array_buffer_ctor(
    cx: &mut NativeCx<'_>,
    _this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    let raw = args.first().copied().unwrap_or(Value::number(0.0));
    let n = to_int_or_inf(cx, raw)?;
    if n < 0.0 || !n.is_finite() || n > MAX_ARRAY_BUFFER_BYTES as f64 {
        return Err(cx.range_error("Invalid array buffer length"));
    }
    let byte_len = n as usize;
    let proto = cx.vm.typed_arrays.array_buffer_proto;
    let ab = cx
        .vm
        .heap
        .new_array_buffer(proto, vec![0u8; byte_len])
        .map_err(heap_to_run)?;
    Ok(Value::object(ab))
}

fn require_array_buffer(cx: &mut NativeCx<'_>, v: Value) -> Result<ObjectRef, RunError> {
    let Some(o) = v.as_object() else {
        return Err(cx.type_error("Method called on incompatible receiver (expected ArrayBuffer)"));
    };
    let obj = cx.vm.heap.object(o).map_err(heap_to_run)?;
    if matches!(obj.class, ObjectClass::ArrayBuffer(_)) {
        Ok(o)
    } else {
        Err(cx.type_error("Method called on incompatible receiver (expected ArrayBuffer)"))
    }
}

fn ab_byte_length_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let o = require_array_buffer(cx, this)?;
    let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(o).map_err(heap_to_run)?.class else {
        unreachable!();
    };
    Ok(Value::number(buf.len() as f64))
}

fn clamp_rel_index(idx: f64, len: usize) -> usize {
    let len_f = len as f64;
    if idx < 0.0 {
        (len_f + idx).max(0.0) as usize
    } else {
        idx.min(len_f) as usize
    }
}

fn ab_slice(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let o = require_array_buffer(cx, this)?;
    let len = {
        let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(o).map_err(heap_to_run)?.class
        else {
            unreachable!();
        };
        buf.len()
    };
    let start_n = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    let end_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => len as f64,
    };
    let first = clamp_rel_index(start_n, len);
    let final_idx = clamp_rel_index(end_n, len);
    let sliced = {
        let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(o).map_err(heap_to_run)?.class
        else {
            unreachable!();
        };
        if final_idx > first {
            buf[first..final_idx].to_vec()
        } else {
            Vec::new()
        }
    };
    let proto = cx.vm.typed_arrays.array_buffer_proto;
    let out = cx
        .vm
        .heap
        .new_array_buffer(proto, sliced)
        .map_err(heap_to_run)?;
    Ok(Value::object(out))
}

fn ab_is_view(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let is_view = args
        .first()
        .copied()
        .and_then(|v| v.as_object())
        .is_some_and(|o| {
            cx.vm.heap.object(o).is_ok_and(|obj| {
                matches!(
                    obj.class,
                    ObjectClass::TypedArray { .. } | ObjectClass::DataView { .. }
                )
            })
        });
    Ok(Value::boolean(is_view))
}

// ---- TypedArray Constructor & Helpers ------------------------------------

fn ta_parts(
    cx: &mut NativeCx<'_>,
    this: Value,
) -> Result<(ObjectRef, TypedArrayKind, ObjectRef, u32, u32), RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Method called on incompatible receiver (expected TypedArray)"));
    };
    match cx.vm.heap.object(o).map_err(heap_to_run)?.class {
        ObjectClass::TypedArray {
            kind,
            buffer,
            byte_offset,
            length,
        } => Ok((o, kind, buffer, byte_offset, length)),
        _ => Err(cx.type_error("Method called on incompatible receiver (expected TypedArray)")),
    }
}

fn alloc_typed_array_zeroed(
    cx: &mut NativeCx<'_>,
    kind: TypedArrayKind,
    length: usize,
) -> Result<(ObjectRef, ObjectRef), RunError> {
    let byte_len = length
        .checked_mul(kind.elem_size())
        .filter(|&b| b <= MAX_ARRAY_BUFFER_BYTES)
        .ok_or_else(|| cx.range_error("Invalid typed array length"))?;
    let ab_proto = cx.vm.typed_arrays.array_buffer_proto;
    let buf = cx
        .vm
        .heap
        .new_array_buffer(ab_proto, vec![0u8; byte_len])
        .map_err(heap_to_run)?;
    cx.keep(Value::object(buf));
    let ta_proto = cx.vm.typed_arrays.proto_for(kind);
    let ta = cx
        .vm
        .heap
        .new_typed_array(ta_proto, kind, buf, 0, length as u32)
        .map_err(heap_to_run)?;
    Ok((ta, buf))
}

fn ta_construct(
    cx: &mut NativeCx<'_>,
    kind: TypedArrayKind,
    args: &[Value],
) -> Result<Value, RunError> {
    let elem_size = kind.elem_size();
    let Some(arg0) = args.first().copied() else {
        let (ta, _) = alloc_typed_array_zeroed(cx, kind, 0)?;
        return Ok(Value::object(ta));
    };

    if let Some(o) = arg0.as_object() {
        let class = cx.vm.heap.object(o).map_err(heap_to_run)?.class.clone();
        match class {
            ObjectClass::ArrayBuffer(buf) => {
                let buf_len = buf.len();
                let off_n = match args.get(1) {
                    Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
                    _ => 0.0,
                };
                if off_n < 0.0 || !off_n.is_finite() || off_n > buf_len as f64 {
                    return Err(cx.range_error("Start offset is outside the bounds of the buffer"));
                }
                let byte_offset = off_n as usize;
                if !byte_offset.is_multiple_of(elem_size) {
                    return Err(cx.range_error(&format!(
                        "start offset of {} should be a multiple of {}",
                        kind.ctor_name(),
                        elem_size
                    )));
                }
                let rem = buf_len - byte_offset;
                let length = match args.get(2) {
                    Some(v) if !v.is_undefined() => {
                        let len_n = to_int_or_inf(cx, *v)?;
                        if len_n < 0.0 || !len_n.is_finite() {
                            return Err(cx.range_error("Invalid typed array length"));
                        }
                        let l = len_n as usize;
                        if l.checked_mul(elem_size).is_none_or(|need| need > rem) {
                            return Err(cx.range_error("Invalid typed array length"));
                        }
                        l
                    }
                    _ => {
                        if !rem.is_multiple_of(elem_size) {
                            return Err(cx.range_error(&format!(
                                "byte length of {} should be a multiple of {}",
                                kind.ctor_name(),
                                elem_size
                            )));
                        }
                        rem / elem_size
                    }
                };
                let proto = cx.vm.typed_arrays.proto_for(kind);
                let ta = cx
                    .vm
                    .heap
                    .new_typed_array(proto, kind, o, byte_offset as u32, length as u32)
                    .map_err(heap_to_run)?;
                return Ok(Value::object(ta));
            }
            ObjectClass::TypedArray {
                kind: src_kind,
                buffer: src_buf,
                byte_offset: src_off,
                length: src_len,
            } => {
                if kind.is_bigint() != src_kind.is_bigint() {
                    return Err(
                        cx.type_error("Cannot convert between BigInt and Number TypedArrays")
                    );
                }
                let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, src_len as usize)?;
                cx.keep(Value::object(ta));
                for i in 0..src_len {
                    let val = read_elem(cx.vm, src_kind, src_buf, src_off, i)
                        .map_err(|e| cx.vm.exit_to_run(e))?;
                    write_elem(cx.vm, kind, dst_buf, 0, src_len, i, val)
                        .map_err(|e| cx.vm.exit_to_run(e))?;
                }
                return Ok(Value::object(ta));
            }
            _ => {
                let items = match cx.vm.spread_values(cx.host, arg0) {
                    Ok(v) => v,
                    Err(_) => read_array_items(cx, arg0)?,
                };
                for v in &items {
                    cx.keep(*v);
                }
                let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, items.len())?;
                cx.keep(Value::object(ta));
                let len = items.len() as u32;
                for (i, &val) in items.iter().enumerate() {
                    write_elem(cx.vm, kind, dst_buf, 0, len, i as u32, val)
                        .map_err(|e| cx.vm.exit_to_run(e))?;
                }
                return Ok(Value::object(ta));
            }
        }
    }

    let len_n = to_int_or_inf(cx, arg0)?;
    if len_n < 0.0 || !len_n.is_finite() || len_n > u32::MAX as f64 {
        return Err(cx.range_error("Invalid typed array length"));
    }
    let (ta, _) = alloc_typed_array_zeroed(cx, kind, len_n as usize)?;
    Ok(Value::object(ta))
}

fn kind_from_static_this(cx: &mut NativeCx<'_>, this: Value) -> Result<TypedArrayKind, RunError> {
    if let Some(o) = this.as_object()
        && let Some(k) = cx.vm.typed_arrays.kind_for_ctor(o)
    {
        return Ok(k);
    }
    Err(cx.type_error("TypedArray static method called on incompatible constructor"))
}

fn ta_from(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let kind = kind_from_static_this(cx, this)?;
    let src = args.first().copied().unwrap_or(Value::UNDEFINED);
    let map_fn = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(2).copied().unwrap_or(Value::UNDEFINED);
    if src.is_nullish() {
        return Err(cx.type_error("Cannot convert undefined or null to object"));
    }
    let mut items = match cx.vm.spread_values(cx.host, src) {
        Ok(v) => v,
        Err(_) => read_array_items(cx, src)?,
    };
    for v in &items {
        cx.keep(*v);
    }
    if !map_fn.is_undefined() {
        cx.keep(map_fn);
        cx.keep(this_arg);
        for (i, item) in items.iter_mut().enumerate() {
            let mapped = cx.call(map_fn, this_arg, &[*item, Value::number(i as f64)])?;
            cx.keep(mapped);
            *item = mapped;
        }
    }
    let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, items.len())?;
    cx.keep(Value::object(ta));
    let len = items.len() as u32;
    for (i, &v) in items.iter().enumerate() {
        write_elem(cx.vm, kind, dst_buf, 0, len, i as u32, v).map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(ta))
}

fn ta_of(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let kind = kind_from_static_this(cx, this)?;
    let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, args.len())?;
    cx.keep(Value::object(ta));
    let len = args.len() as u32;
    for (i, &v) in args.iter().enumerate() {
        write_elem(cx.vm, kind, dst_buf, 0, len, i as u32, v).map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(ta))
}

// ---- TypedArray Prototype Accessors & Methods -----------------------------

fn ta_buffer_get(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let (_, _, buffer, _, _) = ta_parts(cx, this)?;
    Ok(Value::object(buffer))
}

fn ta_byte_length_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let (_, kind, _, _, length) = ta_parts(cx, this)?;
    Ok(Value::number((length as usize * kind.elem_size()) as f64))
}

fn ta_byte_offset_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let (_, _, _, byte_offset, _) = ta_parts(cx, this)?;
    Ok(Value::number(f64::from(byte_offset)))
}

fn ta_length_get(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let (_, _, _, _, length) = ta_parts(cx, this)?;
    Ok(Value::number(f64::from(length)))
}

fn ta_to_string_tag_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let Some(o) = this.as_object() else {
        return Ok(Value::UNDEFINED);
    };
    if let ObjectClass::TypedArray { kind, .. } = cx.vm.heap.object(o).map_err(heap_to_run)?.class {
        cx.vm.string(kind.ctor_name())
    } else {
        Ok(Value::UNDEFINED)
    }
}

fn ta_subarray(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let len = length as usize;
    let begin_n = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    let end_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => len as f64,
    };
    let begin = clamp_rel_index(begin_n, len);
    let end = clamp_rel_index(end_n, len);
    let new_len = end.saturating_sub(begin);
    let new_offset = byte_offset + (begin * kind.elem_size()) as u32;
    let proto = cx.vm.typed_arrays.proto_for(kind);
    let view = cx
        .vm
        .heap
        .new_typed_array(proto, kind, buffer, new_offset, new_len as u32)
        .map_err(heap_to_run)?;
    Ok(Value::object(view))
}

fn ta_slice(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let len = length as usize;
    let begin_n = match args.first() {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    let end_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => len as f64,
    };
    let begin = clamp_rel_index(begin_n, len);
    let end = clamp_rel_index(end_n, len);
    let new_len = end.saturating_sub(begin);
    let elem_size = kind.elem_size();
    let byte_len = new_len * elem_size;
    let copied = {
        let start_b = byte_offset as usize + begin * elem_size;
        let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(buffer).map_err(heap_to_run)?.class
        else {
            return Err(cx.type_error("Invalid backing buffer"));
        };
        buf.get(start_b..start_b + byte_len).unwrap_or(&[]).to_vec()
    };
    let ab_proto = cx.vm.typed_arrays.array_buffer_proto;
    let new_buf = cx
        .vm
        .heap
        .new_array_buffer(ab_proto, copied)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(new_buf));
    let ta_proto = cx.vm.typed_arrays.proto_for(kind);
    let ta = cx
        .vm
        .heap
        .new_typed_array(ta_proto, kind, new_buf, 0, new_len as u32)
        .map_err(heap_to_run)?;
    Ok(Value::object(ta))
}

fn ta_set(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, dst_kind, dst_buf, dst_off, dst_len) = ta_parts(cx, this)?;
    let src = args.first().copied().unwrap_or(Value::UNDEFINED);
    let target_off_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    if target_off_n < 0.0 || !target_off_n.is_finite() || target_off_n > f64::from(dst_len) {
        return Err(cx.range_error("offset is out of bounds"));
    }
    let target_off = target_off_n as u32;

    if let Some(src_obj) = src.as_object()
        && let ObjectClass::TypedArray {
            kind: src_kind,
            buffer: src_buf,
            byte_offset: src_off,
            length: src_len,
        } = cx.vm.heap.object(src_obj).map_err(heap_to_run)?.class
    {
        if target_off.saturating_add(src_len) > dst_len
            || (target_off as usize + src_len as usize) > dst_len as usize
        {
            return Err(cx.range_error("Source is too large"));
        }
        if dst_kind.is_bigint() != src_kind.is_bigint() {
            return Err(cx.type_error("Cannot convert between BigInt and Number TypedArrays"));
        }
        if src_kind == dst_kind {
            let es = dst_kind.elem_size();
            let copy_bytes = src_len as usize * es;
            let s_start = src_off as usize;
            let d_start = dst_off as usize + (target_off as usize) * es;
            let snap = {
                let ObjectClass::ArrayBuffer(b) =
                    &cx.vm.heap.object(src_buf).map_err(heap_to_run)?.class
                else {
                    return Err(cx.type_error("Invalid backing buffer"));
                };
                b.get(s_start..s_start + copy_bytes).unwrap_or(&[]).to_vec()
            };
            if let ObjectClass::ArrayBuffer(b) =
                cx.vm.heap.class_mut(dst_buf).map_err(heap_to_run)?
                && let Some(dst_slice) = b.get_mut(d_start..d_start + snap.len())
            {
                dst_slice.copy_from_slice(&snap);
            }
            return Ok(Value::UNDEFINED);
        }
        let mut vals = Vec::with_capacity(src_len as usize);
        for i in 0..src_len {
            let v = read_elem(cx.vm, src_kind, src_buf, src_off, i)
                .map_err(|e| cx.vm.exit_to_run(e))?;
            vals.push(v);
        }
        for (i, v) in vals.into_iter().enumerate() {
            write_elem(
                cx.vm,
                dst_kind,
                dst_buf,
                dst_off,
                dst_len,
                target_off + i as u32,
                v,
            )
            .map_err(|e| cx.vm.exit_to_run(e))?;
        }
        return Ok(Value::UNDEFINED);
    }

    let items = read_array_items(cx, src)?;
    if target_off as usize + items.len() > dst_len as usize {
        return Err(cx.range_error("offset is out of bounds"));
    }
    for (i, v) in items.into_iter().enumerate() {
        write_elem(
            cx.vm,
            dst_kind,
            dst_buf,
            dst_off,
            dst_len,
            target_off + i as u32,
            v,
        )
        .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::UNDEFINED)
}

fn ta_fill(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (o, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let len = length as usize;
    let start_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    let end_n = match args.get(2) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => len as f64,
    };
    let start = clamp_rel_index(start_n, len) as u32;
    let end = clamp_rel_index(end_n, len) as u32;
    for i in start..end {
        write_elem(cx.vm, kind, buffer, byte_offset, length, i, val)
            .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(o))
}

fn ta_copy_within(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (o, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let len = length as usize;
    let to_n = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    let start_n = to_int_or_inf(cx, args.get(1).copied().unwrap_or(Value::number(0.0)))?;
    let end_n = match args.get(2) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => len as f64,
    };
    let to = clamp_rel_index(to_n, len);
    let start = clamp_rel_index(start_n, len);
    let end = clamp_rel_index(end_n, len);
    let count = end.saturating_sub(start).min(len.saturating_sub(to));
    if count > 0 {
        let es = kind.elem_size();
        let src_b = byte_offset as usize + start * es;
        let dst_b = byte_offset as usize + to * es;
        let byte_count = count * es;
        if let ObjectClass::ArrayBuffer(buf) = cx.vm.heap.class_mut(buffer).map_err(heap_to_run)?
            && src_b + byte_count <= buf.len()
            && dst_b + byte_count <= buf.len()
        {
            buf.copy_within(src_b..src_b + byte_count, dst_b);
        }
    }
    Ok(Value::object(o))
}

fn ta_reverse(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let (o, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let es = kind.elem_size();
    let len = length as usize;
    if let ObjectClass::ArrayBuffer(buf) = cx.vm.heap.class_mut(buffer).map_err(heap_to_run)? {
        for i in 0..(len / 2) {
            let a = byte_offset as usize + i * es;
            let b = byte_offset as usize + (len - 1 - i) * es;
            for k in 0..es {
                buf.swap(a + k, b + k);
            }
        }
    }
    Ok(Value::object(o))
}

fn ta_to_reversed(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let es = kind.elem_size();
    let len = length as usize;
    let byte_len = len * es;
    let mut copied = {
        let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(buffer).map_err(heap_to_run)?.class
        else {
            return Err(cx.type_error("Invalid backing buffer"));
        };
        let s = byte_offset as usize;
        buf.get(s..s + byte_len).unwrap_or(&[]).to_vec()
    };
    for i in 0..(len / 2) {
        let a = i * es;
        let b = (len - 1 - i) * es;
        for k in 0..es {
            copied.swap(a + k, b + k);
        }
    }
    let ab = cx
        .vm
        .heap
        .new_array_buffer(cx.vm.typed_arrays.array_buffer_proto, copied)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(ab));
    let ta = cx
        .vm
        .heap
        .new_typed_array(cx.vm.typed_arrays.proto_for(kind), kind, ab, 0, length)
        .map_err(heap_to_run)?;
    Ok(Value::object(ta))
}

fn collect_ta_vals(
    cx: &mut NativeCx<'_>,
    kind: TypedArrayKind,
    buffer: ObjectRef,
    byte_offset: u32,
    length: u32,
) -> Result<Vec<Value>, RunError> {
    let mut vals = Vec::with_capacity(length as usize);
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        cx.keep(v);
        vals.push(v);
    }
    Ok(vals)
}

fn default_cmp_ta_vals(
    cx: &NativeCx<'_>,
    kind: TypedArrayKind,
    a: Value,
    b: Value,
) -> Result<std::cmp::Ordering, RunError> {
    if kind.is_bigint() {
        let xa = cx
            .vm
            .resolve_bigint(a)
            .ok()
            .flatten()
            .unwrap_or(JsBigInt::ZERO);
        let xb = cx
            .vm
            .resolve_bigint(b)
            .ok()
            .flatten()
            .unwrap_or(JsBigInt::ZERO);
        return Ok(xa.cmp_val(&xb));
    }
    let fa = a.as_number().unwrap_or(0.0);
    let fb = b.as_number().unwrap_or(0.0);
    if fa.is_nan() && fb.is_nan() {
        return Ok(std::cmp::Ordering::Equal);
    }
    if fa.is_nan() {
        return Ok(std::cmp::Ordering::Greater);
    }
    if fb.is_nan() {
        return Ok(std::cmp::Ordering::Less);
    }
    if fa < fb {
        Ok(std::cmp::Ordering::Less)
    } else if fa > fb {
        Ok(std::cmp::Ordering::Greater)
    } else if fa == 0.0 && fb == 0.0 {
        match (fa.is_sign_negative(), fb.is_sign_negative()) {
            (true, false) => Ok(std::cmp::Ordering::Less),
            (false, true) => Ok(std::cmp::Ordering::Greater),
            _ => Ok(std::cmp::Ordering::Equal),
        }
    } else {
        Ok(std::cmp::Ordering::Equal)
    }
}

fn sort_ta_vals(
    cx: &mut NativeCx<'_>,
    kind: TypedArrayKind,
    vals: &mut [Value],
    cmp_fn: Option<Value>,
) -> Result<(), RunError> {
    if let Some(f) = cmp_fn {
        cx.keep(f);
        // Insertion/merge safe for arbitrary JS comparator errors.
        let n = vals.len();
        let mut sorted: Vec<Value> = Vec::with_capacity(n);
        for &v in vals.iter() {
            let mut pos = sorted.len();
            while pos > 0 {
                let res = cx.call(f, Value::UNDEFINED, &[sorted[pos - 1], v])?;
                let ord = to_num(cx, res)?;
                if ord > 0.0 {
                    pos -= 1;
                } else {
                    break;
                }
            }
            sorted.insert(pos, v);
        }
        vals.copy_from_slice(&sorted);
    } else {
        // Native numeric/BigInt sort with zero NaN panics.
        let mut err: Option<RunError> = None;
        vals.sort_by(|&a, &b| {
            if err.is_some() {
                return std::cmp::Ordering::Equal;
            }
            match default_cmp_ta_vals(cx, kind, a, b) {
                Ok(o) => o,
                Err(e) => {
                    err = Some(e);
                    std::cmp::Ordering::Equal
                }
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
    }
    Ok(())
}

fn ta_sort(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (o, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let cmp_fn = args.first().copied().filter(|v| !v.is_undefined());
    let mut vals = collect_ta_vals(cx, kind, buffer, byte_offset, length)?;
    sort_ta_vals(cx, kind, &mut vals, cmp_fn)?;
    for (i, v) in vals.into_iter().enumerate() {
        write_elem(cx.vm, kind, buffer, byte_offset, length, i as u32, v)
            .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(o))
}

fn ta_to_sorted(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let cmp_fn = args.first().copied().filter(|v| !v.is_undefined());
    let mut vals = collect_ta_vals(cx, kind, buffer, byte_offset, length)?;
    sort_ta_vals(cx, kind, &mut vals, cmp_fn)?;
    let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, length as usize)?;
    cx.keep(Value::object(ta));
    for (i, v) in vals.into_iter().enumerate() {
        write_elem(cx.vm, kind, dst_buf, 0, length, i as u32, v)
            .map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(ta))
}

fn same_number_or_bigint_eq(cx: &mut NativeCx<'_>, a: Value, b: Value, zero_mode: bool) -> bool {
    if a.is_number() && b.is_number() {
        let x = a.as_number().unwrap();
        let y = b.as_number().unwrap();
        if zero_mode && x.is_nan() && y.is_nan() {
            return true;
        }
        return x == y;
    }
    if a.is_bigint() && b.is_bigint() {
        return cx.vm.strict_eq(a, b).unwrap_or(false);
    }
    false
}

fn ta_index_of(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if length == 0 {
        return Ok(Value::number(-1.0));
    }
    let from = match args.get(1) {
        Some(v) if !v.is_undefined() => {
            let n = to_int_or_inf(cx, *v)?;
            if n < 0.0 {
                (f64::from(length) + n).max(0.0) as u32
            } else {
                (n as u32).min(length)
            }
        }
        _ => 0,
    };
    for i in from..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        if same_number_or_bigint_eq(cx, v, target, false) {
            return Ok(Value::number(f64::from(i)));
        }
    }
    Ok(Value::number(-1.0))
}

fn ta_last_index_of(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    if length == 0 {
        return Ok(Value::number(-1.0));
    }
    let from = match args.get(1) {
        Some(v) if !v.is_undefined() => {
            let n = to_int_or_inf(cx, *v)?;
            if n < 0.0 {
                let pos = f64::from(length) + n;
                if pos < 0.0 {
                    return Ok(Value::number(-1.0));
                }
                pos as u32
            } else {
                n.min(f64::from(length - 1)) as u32
            }
        }
        _ => length - 1,
    };
    for i in (0..=from).rev() {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        if same_number_or_bigint_eq(cx, v, target, false) {
            return Ok(Value::number(f64::from(i)));
        }
    }
    Ok(Value::number(-1.0))
}

fn ta_includes(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let target = args.first().copied().unwrap_or(Value::UNDEFINED);
    let from = match args.get(1) {
        Some(v) if !v.is_undefined() => {
            clamp_rel_index(to_int_or_inf(cx, *v)?, length as usize) as u32
        }
        _ => 0,
    };
    for i in from..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        if same_number_or_bigint_eq(cx, v, target, true) {
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

fn ta_find_impl(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
    reverse: bool,
    return_index: bool,
) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let pred = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(pred);
    cx.keep(this_arg);
    let indices: Vec<u32> = if reverse {
        (0..length).rev().collect()
    } else {
        (0..length).collect()
    };
    for i in indices {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        cx.keep(v);
        let ok = cx.call(pred, this_arg, &[v, Value::number(f64::from(i)), this])?;
        if cx.vm.truthy(ok).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(if return_index {
                Value::number(f64::from(i))
            } else {
                v
            });
        }
    }
    Ok(if return_index {
        Value::number(-1.0)
    } else {
        Value::UNDEFINED
    })
}

fn ta_find(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_find_impl(cx, this, args, false, false)
}
fn ta_find_index(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_find_impl(cx, this, args, false, true)
}
fn ta_find_last(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_find_impl(cx, this, args, true, false)
}
fn ta_find_last_index(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
) -> Result<Value, RunError> {
    ta_find_impl(cx, this, args, true, true)
}

fn ta_every(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let pred = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(pred);
    cx.keep(this_arg);
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        let ok = cx.call(pred, this_arg, &[v, Value::number(f64::from(i)), this])?;
        if !cx.vm.truthy(ok).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::FALSE);
        }
    }
    Ok(Value::TRUE)
}

fn ta_some(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let pred = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(pred);
    cx.keep(this_arg);
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        let ok = cx.call(pred, this_arg, &[v, Value::number(f64::from(i)), this])?;
        if cx.vm.truthy(ok).map_err(|e| cx.vm.exit_to_run(e))? {
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

fn ta_for_each(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let f = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(f);
    cx.keep(this_arg);
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        cx.call(f, this_arg, &[v, Value::number(f64::from(i)), this])?;
    }
    Ok(Value::UNDEFINED)
}

fn ta_map(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let f = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(f);
    cx.keep(this_arg);
    let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, length as usize)?;
    cx.keep(Value::object(ta));
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        let mapped = cx.call(f, this_arg, &[v, Value::number(f64::from(i)), this])?;
        write_elem(cx.vm, kind, dst_buf, 0, length, i, mapped).map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(ta))
}

fn ta_filter(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let pred = args.first().copied().unwrap_or(Value::UNDEFINED);
    let this_arg = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    cx.keep(this);
    cx.keep(pred);
    cx.keep(this_arg);
    let mut kept = Vec::new();
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        let ok = cx.call(pred, this_arg, &[v, Value::number(f64::from(i)), this])?;
        if cx.vm.truthy(ok).map_err(|e| cx.vm.exit_to_run(e))? {
            cx.keep(v);
            kept.push(v);
        }
    }
    let (ta, dst_buf) = alloc_typed_array_zeroed(cx, kind, kept.len())?;
    cx.keep(Value::object(ta));
    let klen = kept.len() as u32;
    for (i, v) in kept.into_iter().enumerate() {
        write_elem(cx.vm, kind, dst_buf, 0, klen, i as u32, v).map_err(|e| cx.vm.exit_to_run(e))?;
    }
    Ok(Value::object(ta))
}

fn ta_reduce_impl(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
    right: bool,
) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let f = args.first().copied().unwrap_or(Value::UNDEFINED);
    let has_init = args.len() >= 2;
    if length == 0 && !has_init {
        return Err(cx.type_error("Reduce of empty typed array with no initial value"));
    }
    cx.keep(this);
    cx.keep(f);
    let mut acc = if has_init {
        args[1]
    } else {
        let first_idx = if right { length - 1 } else { 0 };
        read_elem(cx.vm, kind, buffer, byte_offset, first_idx).map_err(|e| cx.vm.exit_to_run(e))?
    };
    cx.keep(acc);
    let range: Vec<u32> = if right {
        let end = if has_init { length } else { length - 1 };
        (0..end).rev().collect()
    } else {
        let start = if has_init { 0 } else { 1 };
        (start..length).collect()
    };
    for i in range {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        acc = cx.call(
            f,
            Value::UNDEFINED,
            &[acc, v, Value::number(f64::from(i)), this],
        )?;
        cx.keep(acc);
    }
    Ok(acc)
}

fn ta_reduce(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_reduce_impl(cx, this, args, false)
}
fn ta_reduce_right(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    ta_reduce_impl(cx, this, args, true)
}

fn ta_join(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let sep = match args.first() {
        Some(v) if !v.is_undefined() => to_str_rust(cx, *v)?,
        _ => ",".to_string(),
    };
    let mut parts = Vec::with_capacity(length as usize);
    for i in 0..length {
        let v = read_elem(cx.vm, kind, buffer, byte_offset, i).map_err(|e| cx.vm.exit_to_run(e))?;
        parts.push(to_str_rust(cx, v)?);
    }
    cx.vm.string(&parts.join(&sep))
}

fn ta_to_string(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    ta_join(cx, this, &[])
}

fn ta_at(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let idx_n = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::number(0.0)))?;
    let k = if idx_n < 0.0 {
        f64::from(length) + idx_n
    } else {
        idx_n
    };
    if k < 0.0 || k >= f64::from(length) {
        return Ok(Value::UNDEFINED);
    }
    read_elem(cx.vm, kind, buffer, byte_offset, k as u32).map_err(|e| cx.vm.exit_to_run(e))
}

fn ta_with(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (_, kind, buffer, byte_offset, length) = ta_parts(cx, this)?;
    let idx_n = to_int_or_inf(cx, args.first().copied().unwrap_or(Value::UNDEFINED))?;
    let k = if idx_n < 0.0 {
        f64::from(length) + idx_n
    } else {
        idx_n
    };
    if k < 0.0 || k >= f64::from(length) {
        return Err(cx.range_error("Invalid index"));
    }
    let val = args.get(1).copied().unwrap_or(Value::UNDEFINED);
    let es = kind.elem_size();
    let copied = {
        let ObjectClass::ArrayBuffer(buf) = &cx.vm.heap.object(buffer).map_err(heap_to_run)?.class
        else {
            return Err(cx.type_error("Invalid backing buffer"));
        };
        let s = byte_offset as usize;
        buf.get(s..s + length as usize * es).unwrap_or(&[]).to_vec()
    };
    let ab = cx
        .vm
        .heap
        .new_array_buffer(cx.vm.typed_arrays.array_buffer_proto, copied)
        .map_err(heap_to_run)?;
    cx.keep(Value::object(ab));
    write_elem(cx.vm, kind, ab, 0, length, k as u32, val).map_err(|e| cx.vm.exit_to_run(e))?;
    let ta = cx
        .vm
        .heap
        .new_typed_array(cx.vm.typed_arrays.proto_for(kind), kind, ab, 0, length)
        .map_err(heap_to_run)?;
    Ok(Value::object(ta))
}

fn ta_keys(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let _ = ta_parts(cx, this)?;
    make_list_iterator(cx, this, IterKind::Keys)
}
fn ta_values(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let _ = ta_parts(cx, this)?;
    make_list_iterator(cx, this, IterKind::Values)
}
fn ta_entries(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let _ = ta_parts(cx, this)?;
    make_list_iterator(cx, this, IterKind::Entries)
}

// ---- DataView Constructor & Methods ---------------------------------------

fn data_view_ctor(cx: &mut NativeCx<'_>, _this: Value, args: &[Value]) -> Result<Value, RunError> {
    let buf_val = args.first().copied().unwrap_or(Value::UNDEFINED);
    let buf = require_array_buffer(cx, buf_val)?;
    let buf_len = {
        let ObjectClass::ArrayBuffer(b) = &cx.vm.heap.object(buf).map_err(heap_to_run)?.class
        else {
            unreachable!();
        };
        b.len()
    };
    let off_n = match args.get(1) {
        Some(v) if !v.is_undefined() => to_int_or_inf(cx, *v)?,
        _ => 0.0,
    };
    if off_n < 0.0 || !off_n.is_finite() || off_n > buf_len as f64 {
        return Err(cx.range_error("Start offset is outside the bounds of the buffer"));
    }
    let byte_offset = off_n as usize;
    let rem = buf_len - byte_offset;
    let byte_length = match args.get(2) {
        Some(v) if !v.is_undefined() => {
            let bl_n = to_int_or_inf(cx, *v)?;
            if bl_n < 0.0 || !bl_n.is_finite() || bl_n > rem as f64 {
                return Err(cx.range_error("Invalid DataView length"));
            }
            bl_n as usize
        }
        _ => rem,
    };
    let dv = cx
        .vm
        .heap
        .new_data_view(
            cx.vm.typed_arrays.data_view_proto,
            buf,
            byte_offset as u32,
            byte_length as u32,
        )
        .map_err(heap_to_run)?;
    Ok(Value::object(dv))
}

fn dv_parts(cx: &mut NativeCx<'_>, this: Value) -> Result<(ObjectRef, u32, u32), RunError> {
    let Some(o) = this.as_object() else {
        return Err(cx.type_error("Method called on incompatible receiver (expected DataView)"));
    };
    match cx.vm.heap.object(o).map_err(heap_to_run)?.class {
        ObjectClass::DataView {
            buffer,
            byte_offset,
            byte_length,
        } => Ok((buffer, byte_offset, byte_length)),
        _ => Err(cx.type_error("Method called on incompatible receiver (expected DataView)")),
    }
}

fn dv_buffer_get(cx: &mut NativeCx<'_>, this: Value, _args: &[Value]) -> Result<Value, RunError> {
    let (buf, _, _) = dv_parts(cx, this)?;
    Ok(Value::object(buf))
}
fn dv_byte_length_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let (_, _, byte_length) = dv_parts(cx, this)?;
    Ok(Value::number(f64::from(byte_length)))
}
fn dv_byte_offset_get(
    cx: &mut NativeCx<'_>,
    this: Value,
    _args: &[Value],
) -> Result<Value, RunError> {
    let (_, byte_offset, _) = dv_parts(cx, this)?;
    Ok(Value::number(f64::from(byte_offset)))
}

fn dv_bounds(
    cx: &mut NativeCx<'_>,
    byte_length: u32,
    offset_arg: Value,
    size: usize,
) -> Result<usize, RunError> {
    let n = to_int_or_inf(cx, offset_arg)?;
    if n < 0.0 || !n.is_finite() || n + (size as f64) > f64::from(byte_length) {
        return Err(cx.range_error("Offset is outside the bounds of the DataView"));
    }
    Ok(n as usize)
}

fn dv_little_endian(cx: &mut NativeCx<'_>, args: &[Value], idx: usize) -> Result<bool, RunError> {
    match args.get(idx) {
        Some(v) if !v.is_undefined() => cx.vm.truthy(*v).map_err(|e| cx.vm.exit_to_run(e)),
        _ => Ok(false),
    }
}

fn dv_read_bytes(
    cx: &mut NativeCx<'_>,
    this: Value,
    args: &[Value],
    size: usize,
) -> Result<(Vec<u8>, bool), RunError> {
    let (buffer, byte_offset, byte_length) = dv_parts(cx, this)?;
    let off = dv_bounds(
        cx,
        byte_length,
        args.first().copied().unwrap_or(Value::number(0.0)),
        size,
    )?;
    let le = dv_little_endian(cx, args, 1)?;
    let pos = byte_offset as usize + off;
    let copied = {
        let ObjectClass::ArrayBuffer(b) = &cx.vm.heap.object(buffer).map_err(heap_to_run)?.class
        else {
            return Err(cx.type_error("Invalid backing buffer"));
        };
        b.get(pos..pos + size).map(|s| s.to_vec())
    };
    let Some(slice) = copied else {
        return Err(cx.range_error("Offset out of bounds"));
    };
    Ok((slice, le))
}

fn dv_write_bytes(
    cx: &mut NativeCx<'_>,
    this: Value,
    offset_arg: Value,
    raw: &[u8],
) -> Result<(), RunError> {
    let (buffer, byte_offset, byte_length) = dv_parts(cx, this)?;
    let off = dv_bounds(cx, byte_length, offset_arg, raw.len())?;
    let pos = byte_offset as usize + off;
    if let ObjectClass::ArrayBuffer(b) = cx.vm.heap.class_mut(buffer).map_err(heap_to_run)?
        && let Some(dst) = b.get_mut(pos..pos + raw.len())
    {
        dst.copy_from_slice(raw);
    }
    Ok(())
}

fn dv_get_i8(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, _) = dv_read_bytes(cx, this, args, 1)?;
    Ok(Value::number(f64::from(b[0] as i8)))
}
fn dv_get_u8(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, _) = dv_read_bytes(cx, this, args, 1)?;
    Ok(Value::number(f64::from(b[0])))
}
fn dv_get_i16(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 2)?;
    let a = [b[0], b[1]];
    let v = if le {
        i16::from_le_bytes(a)
    } else {
        i16::from_be_bytes(a)
    };
    Ok(Value::number(f64::from(v)))
}
fn dv_get_u16(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 2)?;
    let a = [b[0], b[1]];
    let v = if le {
        u16::from_le_bytes(a)
    } else {
        u16::from_be_bytes(a)
    };
    Ok(Value::number(f64::from(v)))
}
fn dv_get_i32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 4)?;
    let a = [b[0], b[1], b[2], b[3]];
    let v = if le {
        i32::from_le_bytes(a)
    } else {
        i32::from_be_bytes(a)
    };
    Ok(Value::number(f64::from(v)))
}
fn dv_get_u32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 4)?;
    let a = [b[0], b[1], b[2], b[3]];
    let v = if le {
        u32::from_le_bytes(a)
    } else {
        u32::from_be_bytes(a)
    };
    Ok(Value::number(f64::from(v)))
}
fn dv_get_f32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 4)?;
    let a = [b[0], b[1], b[2], b[3]];
    let v = if le {
        f32::from_le_bytes(a)
    } else {
        f32::from_be_bytes(a)
    };
    Ok(Value::number(f64::from(v)))
}
fn dv_get_f64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 8)?;
    let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
    let v = if le {
        f64::from_le_bytes(a)
    } else {
        f64::from_be_bytes(a)
    };
    Ok(Value::number(v))
}
fn dv_get_bi64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 8)?;
    let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
    let v = if le {
        i64::from_le_bytes(a)
    } else {
        i64::from_be_bytes(a)
    };
    cx.vm
        .alloc_bigint_val(JsBigInt::from_i64(v))
        .map_err(|e| cx.vm.exit_to_run(e))
}
fn dv_get_bu64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let (b, le) = dv_read_bytes(cx, this, args, 8)?;
    let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
    let v = if le {
        u64::from_le_bytes(a)
    } else {
        u64::from_be_bytes(a)
    };
    cx.vm
        .alloc_bigint_val(JsBigInt::from_u64(v))
        .map_err(|e| cx.vm.exit_to_run(e))
}

fn dv_set_i8(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &[(to_int32(n) as i8) as u8],
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_u8(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &[(to_int32(n) as u32) as u8],
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_i16(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = to_int32(n) as i16;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_u16(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = (to_int32(n) as u32) as u16;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_i32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = to_int32(n);
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_u32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = crate::value::number::to_uint32(n);
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_f32(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = n as f32;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_f64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let n = to_num(cx, args.get(1).copied().unwrap_or(Value::UNDEFINED))?;
    let le = dv_little_endian(cx, args, 2)?;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { n.to_le_bytes() } else { n.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_bi64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let bi = to_bigint_val(cx.vm, args.get(1).copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = bi.to_u64_wrapping() as i64;
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
fn dv_set_bu64(cx: &mut NativeCx<'_>, this: Value, args: &[Value]) -> Result<Value, RunError> {
    let bi = to_bigint_val(cx.vm, args.get(1).copied().unwrap_or(Value::UNDEFINED))
        .map_err(|e| cx.vm.exit_to_run(e))?;
    let le = dv_little_endian(cx, args, 2)?;
    let v = bi.to_u64_wrapping();
    dv_write_bytes(
        cx,
        this,
        args.first().copied().unwrap_or(Value::number(0.0)),
        &if le { v.to_le_bytes() } else { v.to_be_bytes() },
    )?;
    Ok(Value::UNDEFINED)
}
