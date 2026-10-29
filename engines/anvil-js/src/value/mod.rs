// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The value word and the handle types.
//!
//! A [`Value`] is one 64-bit word. A plain number is stored as its `f64`
//! bits. Everything else is a quiet NaN carrying a 4-bit tag in bits 47..50
//! and a 47-bit payload below it:
//!
//! ```text
//! bits 63..51   0x7FF8 >> 3 (positive quiet NaN)
//! bits 50..47   tag (1..15, 0 is the one canonical NaN)
//! bits 46..0    payload: an i32, a bool, or a heap handle
//! ```
//!
//! Every NaN that comes in is turned into the one canonical NaN, so no
//! arithmetic result can ever look like a tagged value. Numbers that are
//! whole and fit an `i32` are stored with the `Int32` tag, except `-0`,
//! which has to stay a double.

pub mod atom;
pub mod collection;
pub mod number;
pub mod object;
pub mod shape;
pub mod string;

use crate::anvil_js::gc::arena::RawHandle;
use std::fmt;

/// The kinds a [`Value`] can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A double (including NaN and the infinities).
    Number,
    /// A number stored as a 32-bit integer.
    Int32,
    /// `true` or `false`.
    Bool,
    /// `null`.
    Null,
    /// `undefined`.
    Undefined,
    /// A string on the heap.
    String,
    /// An object on the heap (plain objects, arrays, functions, wrappers).
    Object,
    /// A symbol on the heap.
    Symbol,
    /// A BigInt (reserved, not made yet).
    BigInt,
    /// Internal marker for a `let`/`const` binding not set yet. Never seen by script.
    Uninit,
    /// Internal marker for a gap in array storage. Never seen by script.
    Hole,
}

const TAG_SHIFT: u32 = 47;
const TOP17_PREFIX: u64 = 0xFFF0;
const PAYLOAD_MASK: u64 = (1u64 << TAG_SHIFT) - 1;
/// Bit pattern of the one NaN every NaN is turned into.
pub const CANONICAL_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

const TAG_INT32: u64 = 1;
const TAG_BOOL: u64 = 2;
const TAG_NULL: u64 = 3;
const TAG_UNDEFINED: u64 = 4;
const TAG_STRING: u64 = 5;
const TAG_OBJECT: u64 = 6;
const TAG_SYMBOL: u64 = 7;
const TAG_BIGINT: u64 = 8;
const TAG_UNINIT: u64 = 9;
const TAG_HOLE: u64 = 10;

const fn tagged(tag: u64, payload: u64) -> u64 {
    ((TOP17_PREFIX | tag) << TAG_SHIFT) | (payload & PAYLOAD_MASK)
}

const fn top17(tag: u64) -> u64 {
    TOP17_PREFIX | tag
}

/// One engine value. Copying it is free.
///
/// `Value` has no `PartialEq` on purpose: comparing bits is not the same as
/// any of the language's equality rules (NaN, `0 === -0`, strings by
/// content). Use [`Value::same_bits`] when bit identity is what you want.
#[derive(Clone, Copy)]
pub struct Value(u64);

macro_rules! handle_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub RawHandle);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{} ({})", stringify!($name), self.0)
            }
        }
    };
}

handle_type!(
    /// Handle to an object on the heap.
    ObjectRef
);
handle_type!(
    /// Handle to a string on the heap.
    StringRef
);
handle_type!(
    /// Handle to a symbol on the heap.
    SymbolRef
);
handle_type!(
    /// Handle to a shape on the heap. Shapes are never script values.
    ShapeRef
);

impl Value {
    /// `undefined`.
    pub const UNDEFINED: Value = Value(tagged(TAG_UNDEFINED, 0));
    /// `null`.
    pub const NULL: Value = Value(tagged(TAG_NULL, 0));
    /// `true`.
    pub const TRUE: Value = Value(tagged(TAG_BOOL, 1));
    /// `false`.
    pub const FALSE: Value = Value(tagged(TAG_BOOL, 0));
    /// The canonical NaN.
    pub const NAN: Value = Value(CANONICAL_NAN_BITS);
    /// Internal TDZ marker.
    pub const UNINIT: Value = Value(tagged(TAG_UNINIT, 0));
    /// Internal array gap marker.
    pub const HOLE: Value = Value(tagged(TAG_HOLE, 0));

    /// A number. Whole numbers that fit an `i32` (but not `-0`) use the
    /// `Int32` form; every NaN becomes the canonical NaN.
    pub fn number(n: f64) -> Value {
        if n.is_nan() {
            return Value::NAN;
        }
        let i = n as i32;
        if f64::from(i) == n && !(i == 0 && n.is_sign_negative()) {
            return Value::int32(i);
        }
        Value(n.to_bits())
    }

    /// A number stored as a double, even when it is whole. NaN is still
    /// made canonical.
    pub fn double(n: f64) -> Value {
        if n.is_nan() {
            Value::NAN
        } else {
            Value(n.to_bits())
        }
    }

    /// A 32-bit integer.
    pub const fn int32(i: i32) -> Value {
        Value(tagged(TAG_INT32, i as u32 as u64))
    }

    /// A boolean.
    pub const fn boolean(b: bool) -> Value {
        if b { Value::TRUE } else { Value::FALSE }
    }

    /// A string value.
    pub fn string(r: StringRef) -> Value {
        Value(tagged(TAG_STRING, r.0.to_payload()))
    }

    /// An object value.
    pub fn object(r: ObjectRef) -> Value {
        Value(tagged(TAG_OBJECT, r.0.to_payload()))
    }

    /// A symbol value.
    pub fn symbol(r: SymbolRef) -> Value {
        Value(tagged(TAG_SYMBOL, r.0.to_payload()))
    }

    /// The raw bits, for debugging and tests.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Build a value from raw bits. Bits that are a NaN without a known tag
    /// become the canonical NaN, so this can't make a value with a bad tag.
    pub fn from_bits(bits: u64) -> Value {
        let v = Value(bits);
        match v.kind() {
            Kind::Number => Value::double(f64::from_bits(bits)),
            _ => v,
        }
    }

    /// True when the two values have the same bits.
    pub const fn same_bits(self, other: Value) -> bool {
        self.0 == other.0
    }

    const fn top17(self) -> u64 {
        self.0 >> TAG_SHIFT
    }

    const fn payload(self) -> u64 {
        self.0 & PAYLOAD_MASK
    }

    /// Which kind of value this is.
    pub fn kind(self) -> Kind {
        let top = self.top17();
        if top & 0x1FFF0 != TOP17_PREFIX {
            return Kind::Number;
        }
        match top & 0xF {
            TAG_INT32 => Kind::Int32,
            TAG_BOOL => Kind::Bool,
            TAG_NULL => Kind::Null,
            TAG_UNDEFINED => Kind::Undefined,
            TAG_STRING => Kind::String,
            TAG_OBJECT => Kind::Object,
            TAG_SYMBOL => Kind::Symbol,
            TAG_BIGINT => Kind::BigInt,
            TAG_UNINIT => Kind::Uninit,
            TAG_HOLE => Kind::Hole,
            // Tag 0 is the canonical NaN; unused tags never get made, and
            // reading them as a number is the safe answer.
            _ => Kind::Number,
        }
    }

    /// True for both number forms.
    pub fn is_number(self) -> bool {
        matches!(self.kind(), Kind::Number | Kind::Int32)
    }

    /// True for the `Int32` form. One compare.
    pub const fn is_int32(self) -> bool {
        self.top17() == top17(TAG_INT32)
    }

    /// True for booleans. One compare.
    pub const fn is_bool(self) -> bool {
        self.top17() == top17(TAG_BOOL)
    }

    /// True for `null`. One compare.
    pub const fn is_null(self) -> bool {
        self.top17() == top17(TAG_NULL)
    }

    /// True for `undefined`. One compare.
    pub const fn is_undefined(self) -> bool {
        self.top17() == top17(TAG_UNDEFINED)
    }

    /// True for `null` or `undefined`.
    pub const fn is_nullish(self) -> bool {
        self.is_null() || self.is_undefined()
    }

    /// True for strings. One compare.
    pub const fn is_string(self) -> bool {
        self.top17() == top17(TAG_STRING)
    }

    /// True for objects. One compare.
    pub const fn is_object(self) -> bool {
        self.top17() == top17(TAG_OBJECT)
    }

    /// True for symbols. One compare.
    pub const fn is_symbol(self) -> bool {
        self.top17() == top17(TAG_SYMBOL)
    }

    /// True for the internal TDZ marker. One compare.
    pub const fn is_uninit(self) -> bool {
        self.top17() == top17(TAG_UNINIT)
    }

    /// True for the internal array gap marker. One compare.
    pub const fn is_hole(self) -> bool {
        self.top17() == top17(TAG_HOLE)
    }

    /// The number, for either number form.
    pub fn as_number(self) -> Option<f64> {
        match self.kind() {
            Kind::Number => Some(f64::from_bits(self.0)),
            Kind::Int32 => Some(f64::from(self.payload() as u32 as i32)),
            _ => None,
        }
    }

    /// The integer, for the `Int32` form only.
    pub fn as_int32(self) -> Option<i32> {
        if self.is_int32() {
            Some(self.payload() as u32 as i32)
        } else {
            None
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(self) -> Option<bool> {
        if self.is_bool() {
            Some(self.payload() != 0)
        } else {
            None
        }
    }

    /// The string handle, if this is a string.
    pub fn as_string(self) -> Option<StringRef> {
        if self.is_string() {
            Some(StringRef(RawHandle::from_payload(self.payload())))
        } else {
            None
        }
    }

    /// The object handle, if this is an object.
    pub fn as_object(self) -> Option<ObjectRef> {
        if self.is_object() {
            Some(ObjectRef(RawHandle::from_payload(self.payload())))
        } else {
            None
        }
    }

    /// The symbol handle, if this is a symbol.
    pub fn as_symbol(self) -> Option<SymbolRef> {
        if self.is_symbol() {
            Some(SymbolRef(RawHandle::from_payload(self.payload())))
        } else {
            None
        }
    }

    /// The `typeof` answer for kinds that don't need the heap to tell.
    /// Objects give `None`, because functions answer `"function"`.
    pub fn typeof_primitive(self) -> Option<&'static str> {
        match self.kind() {
            Kind::Number | Kind::Int32 => Some("number"),
            Kind::Bool => Some("boolean"),
            Kind::Null => Some("object"),
            Kind::Undefined | Kind::Uninit | Kind::Hole => Some("undefined"),
            Kind::String => Some("string"),
            Kind::Symbol => Some("symbol"),
            Kind::BigInt => Some("bigint"),
            Kind::Object => None,
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::UNDEFINED
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            Kind::Number | Kind::Int32 => {
                write!(
                    f,
                    "{}",
                    number::number_to_string(self.as_number().unwrap_or(f64::NAN))
                )
            }
            Kind::Bool => write!(f, "{}", self.payload() != 0),
            Kind::Null => f.write_str("null"),
            Kind::Undefined => f.write_str("undefined"),
            Kind::String => write!(f, "string({})", RawHandle::from_payload(self.payload())),
            Kind::Object => write!(f, "object({})", RawHandle::from_payload(self.payload())),
            Kind::Symbol => write!(f, "symbol({})", RawHandle::from_payload(self.payload())),
            Kind::BigInt => f.write_str("bigint"),
            Kind::Uninit => f.write_str("<uninitialized>"),
            Kind::Hole => f.write_str("<hole>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(index: u32, generation: u16) -> RawHandle {
        RawHandle { index, generation }
    }

    #[test]
    fn immediates_round_trip() {
        assert_eq!(Value::UNDEFINED.kind(), Kind::Undefined);
        assert_eq!(Value::NULL.kind(), Kind::Null);
        assert_eq!(Value::TRUE.as_bool(), Some(true));
        assert_eq!(Value::FALSE.as_bool(), Some(false));
        assert_eq!(Value::boolean(true).as_bool(), Some(true));
        for i in [0, 1, -1, i32::MAX, i32::MIN, 12345] {
            let v = Value::int32(i);
            assert_eq!(v.kind(), Kind::Int32);
            assert_eq!(v.as_int32(), Some(i));
            assert_eq!(v.as_number(), Some(f64::from(i)));
        }
        assert_eq!(Value::UNINIT.kind(), Kind::Uninit);
        assert_eq!(Value::HOLE.kind(), Kind::Hole);
        assert_eq!(Value::default().kind(), Kind::Undefined);
    }

    #[test]
    fn numbers_pick_the_right_form() {
        assert!(Value::number(3.0).is_int32());
        assert!(Value::number(-7.0).is_int32());
        assert!(!Value::number(0.5).is_int32());
        assert!(!Value::number(2147483648.0).is_int32());
        assert!(Value::number(-2147483648.0).is_int32());
        let neg_zero = Value::number(-0.0);
        assert_eq!(neg_zero.kind(), Kind::Number);
        assert!(neg_zero.as_number().unwrap().is_sign_negative());
        assert!(Value::number(0.0).is_int32());
        for x in [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN_POSITIVE,
            5e-324,
            -1.5,
        ] {
            let v = Value::number(x);
            assert_eq!(v.kind(), Kind::Number);
            assert_eq!(v.as_number().unwrap().to_bits(), x.to_bits());
        }
        assert!(Value::double(4.0).kind() == Kind::Number);
    }

    #[test]
    fn every_nan_is_canonical() {
        let nans = [
            f64::NAN,
            -f64::NAN,
            f64::from_bits(0x7FF0_0000_0000_0001),
            f64::from_bits(0x7FFF_FFFF_FFFF_FFFF),
            f64::from_bits(0xFFF8_0000_0000_0000),
            f64::from_bits(tagged(TAG_OBJECT, 5)),
            f64::from_bits(tagged(TAG_STRING, 0)),
        ];
        for n in nans {
            let v = Value::number(n);
            assert_eq!(v.bits(), CANONICAL_NAN_BITS);
            assert_eq!(v.kind(), Kind::Number);
            assert!(v.as_number().unwrap().is_nan());
            assert_eq!(Value::double(n).bits(), CANONICAL_NAN_BITS);
        }
    }

    #[test]
    fn handles_round_trip() {
        for handle in [h(0, 0), h(1, 1), h(u32::MAX - 1, 0x7FFF), h(77, 300)] {
            let s = Value::string(StringRef(handle));
            let o = Value::object(ObjectRef(handle));
            let y = Value::symbol(SymbolRef(handle));
            assert_eq!(s.as_string(), Some(StringRef(handle)));
            assert_eq!(o.as_object(), Some(ObjectRef(handle)));
            assert_eq!(y.as_symbol(), Some(SymbolRef(handle)));
            assert_eq!(s.as_object(), None);
            assert_eq!(o.as_string(), None);
            assert_eq!(o.kind(), Kind::Object);
        }
    }

    #[test]
    fn one_compare_tests_match_kind_for_every_top17_pattern() {
        for top in 0u64..(1 << 17) {
            for low in [0u64, 1, 0xFFFF_FFFF, PAYLOAD_MASK] {
                let v = Value((top << TAG_SHIFT) | low);
                let k = v.kind();
                assert_eq!(v.is_int32(), k == Kind::Int32, "top {top:#x}");
                assert_eq!(v.is_bool(), k == Kind::Bool, "top {top:#x}");
                assert_eq!(v.is_null(), k == Kind::Null, "top {top:#x}");
                assert_eq!(v.is_undefined(), k == Kind::Undefined, "top {top:#x}");
                assert_eq!(v.is_string(), k == Kind::String, "top {top:#x}");
                assert_eq!(v.is_object(), k == Kind::Object, "top {top:#x}");
                assert_eq!(v.is_symbol(), k == Kind::Symbol, "top {top:#x}");
                assert_eq!(v.is_uninit(), k == Kind::Uninit, "top {top:#x}");
                assert_eq!(v.is_hole(), k == Kind::Hole, "top {top:#x}");
            }
        }
    }

    #[test]
    fn from_bits_never_makes_unknown_tags() {
        let v = Value::from_bits(tagged(13, 99));
        assert_eq!(v.bits(), CANONICAL_NAN_BITS);
        let o = Value::object(ObjectRef(h(3, 4)));
        assert!(Value::from_bits(o.bits()).same_bits(o));
    }

    #[test]
    fn typeof_for_primitives() {
        assert_eq!(Value::NULL.typeof_primitive(), Some("object"));
        assert_eq!(Value::UNDEFINED.typeof_primitive(), Some("undefined"));
        assert_eq!(Value::number(1.5).typeof_primitive(), Some("number"));
        assert_eq!(Value::int32(1).typeof_primitive(), Some("number"));
        assert_eq!(Value::TRUE.typeof_primitive(), Some("boolean"));
        assert_eq!(
            Value::string(StringRef(h(0, 0))).typeof_primitive(),
            Some("string")
        );
        assert_eq!(Value::object(ObjectRef(h(0, 0))).typeof_primitive(), None);
    }

    #[test]
    fn debug_output() {
        assert_eq!(format!("{:?}", Value::int32(5)), "5");
        assert_eq!(format!("{:?}", Value::NULL), "null");
        assert_eq!(format!("{:?}", Value::number(0.5)), "0.5");
    }
}
