// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Arbitrary-precision signed integer arithmetic for ECMAScript `BigInt`.

use std::cmp::Ordering;

/// Maximum exponent allowed for `BigInt ** exp` before failing cleanly.
const MAX_BIGINT_POW_BITS: u64 = 1 << 20;

/// Arbitrary-precision signed integer stored in little-endian base-`2^32` limbs.
/// Zero is canonically represented with `neg = false` and an empty `digits` vector.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JsBigInt {
    /// True when strictly negative. Always false for zero.
    pub neg: bool,
    /// Magnitude limbs in little-endian order; no trailing zero limbs.
    pub digits: Vec<u32>,
}

impl JsBigInt {
    /// Zero (`0n`).
    pub const ZERO: JsBigInt = JsBigInt {
        neg: false,
        digits: Vec::new(),
    };

    /// Wrap limbs and sign into canonical normalized form.
    pub fn from_parts(neg: bool, mut digits: Vec<u32>) -> Self {
        while digits.last() == Some(&0) {
            digits.pop();
        }
        JsBigInt {
            neg: if digits.is_empty() { false } else { neg },
            digits,
        }
    }

    /// Construct from an `i64`.
    pub fn from_i64(n: i64) -> Self {
        if n == 0 {
            return Self::ZERO;
        }
        let neg = n < 0;
        let mag = n.unsigned_abs();
        Self::from_u64_signed(neg, mag)
    }

    /// Construct from an unsigned `u64`.
    pub fn from_u64(n: u64) -> Self {
        Self::from_u64_signed(false, n)
    }

    /// Extract low 64 bits in two's-complement wrapping representation (`ToBigUint64` / `ToBigInt64`).
    pub fn to_u64_wrapping(&self) -> u64 {
        let u = self.as_uint_n(64);
        let lo = u64::from(*u.digits.first().unwrap_or(&0));
        let hi = u64::from(*u.digits.get(1).unwrap_or(&0));
        lo | (hi << 32)
    }

    /// Construct from an `i128`.
    pub fn from_i128(n: i128) -> Self {
        if n == 0 {
            return Self::ZERO;
        }
        let neg = n < 0;
        let mag = n.unsigned_abs();
        let mut digits = Vec::new();
        let mut rem = mag;
        while rem != 0 {
            digits.push((rem & 0xFFFF_FFFF) as u32);
            rem >>= 32;
        }
        Self::from_parts(neg, digits)
    }

    fn from_u64_signed(neg: bool, mag: u64) -> Self {
        if mag == 0 {
            return Self::ZERO;
        }
        let lo = (mag & 0xFFFF_FFFF) as u32;
        let hi = (mag >> 32) as u32;
        if hi == 0 {
            Self::from_parts(neg, vec![lo])
        } else {
            Self::from_parts(neg, vec![lo, hi])
        }
    }

    /// True if `self == 0n`.
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Owned heap allocation estimate for GC accounting.
    pub fn owned_bytes(&self) -> usize {
        self.digits.capacity() * std::mem::size_of::<u32>()
    }

    /// Fit into a 47-bit signed inline BigInt payload if possible.
    pub fn to_inline_i64(&self) -> Option<i64> {
        let i = self.to_i64_exact()?;
        let min = -(1i64 << 46);
        let max = (1i64 << 46) - 1;
        (min..=max).contains(&i).then_some(i)
    }

    /// Exact conversion to `i64` if within range.
    pub fn to_i64_exact(&self) -> Option<i64> {
        if self.digits.is_empty() {
            return Some(0);
        }
        if self.digits.len() > 2 {
            return None;
        }
        let lo = u64::from(self.digits[0]);
        let hi = u64::from(*self.digits.get(1).unwrap_or(&0));
        let mag = lo | (hi << 32);
        if self.neg {
            if mag == (1u64 << 63) {
                Some(i64::MIN)
            } else if mag < (1u64 << 63) {
                Some(-(mag as i64))
            } else {
                None
            }
        } else {
            i64::try_from(mag).ok()
        }
    }

    /// Convert to `f64` using binary magnitude accumulation (saturating at infinity).
    pub fn to_f64(&self) -> f64 {
        if self.digits.is_empty() {
            return 0.0;
        }
        let mut acc = 0.0f64;
        for &limb in self.digits.iter().rev() {
            acc = acc * 4_294_967_296.0 + f64::from(limb);
        }
        if self.neg { -acc } else { acc }
    }

    /// Exact integer `f64` -> `BigInt` conversion if `f` is a finite integer.
    pub fn from_f64_exact(f: f64) -> Option<Self> {
        if !f.is_finite() || f.fract() != 0.0 {
            return None;
        }
        if f == 0.0 {
            return Some(Self::ZERO);
        }
        let bits = f.to_bits();
        let neg = (bits >> 63) != 0;
        let exp = ((bits >> 52) & 0x7FF) as i32 - 1023;
        let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        if exp <= 52 {
            let val = mantissa >> (52 - exp) as u32;
            Some(Self::from_u64_signed(neg, val))
        } else {
            let base = Self::from_u64_signed(neg, mantissa);
            Some(base.shl_usize((exp - 52) as usize))
        }
    }

    /// Parse ECMAScript `StringToBigInt` / integer literal syntax (`0x`/`0o`/`0b`/decimal).
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.is_empty() {
            return Some(Self::ZERO);
        }
        let (neg, body) = if let Some(rest) = s.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = s.strip_prefix('+') {
            (false, rest)
        } else {
            (false, s)
        };
        if body.is_empty() {
            return None;
        }
        let (radix, digits_str) = if body.starts_with("0x") || body.starts_with("0X") {
            if s.starts_with('-') || s.starts_with('+') {
                return None;
            }
            (16u32, &body[2..])
        } else if body.starts_with("0o") || body.starts_with("0O") {
            if s.starts_with('-') || s.starts_with('+') {
                return None;
            }
            (8u32, &body[2..])
        } else if body.starts_with("0b") || body.starts_with("0B") {
            if s.starts_with('-') || s.starts_with('+') {
                return None;
            }
            (2u32, &body[2..])
        } else {
            (10u32, body)
        };
        if digits_str.is_empty() {
            return None;
        }
        let mut mag: Vec<u32> = Vec::new();
        for ch in digits_str.chars() {
            let d = ch.to_digit(radix)?;
            mul_add_small(&mut mag, radix, d);
        }
        Some(Self::from_parts(neg, mag))
    }

    /// Format into base `radix` (`2..=36`).
    pub fn to_radix_string(&self, radix: u32) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        if let Some(i) = self.to_i64_exact()
            && radix == 10
        {
            return i.to_string();
        }
        let mut work = self.digits.clone();
        let mut out = Vec::new();
        while !work.is_empty() {
            let rem = div_rem_small(&mut work, radix);
            let ch = char::from_digit(rem, radix).unwrap_or('0');
            out.push(ch);
        }
        if self.neg {
            out.push('-');
        }
        out.reverse();
        out.into_iter().collect()
    }

    /// Logical negation (`-x`).
    pub fn neg(&self) -> Self {
        if self.is_zero() {
            Self::ZERO
        } else {
            JsBigInt {
                neg: !self.neg,
                digits: self.digits.clone(),
            }
        }
    }

    /// Exact comparison between two `JsBigInt`s.
    pub fn cmp_val(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => cmp_mag(&self.digits, &other.digits),
            (true, true) => cmp_mag(&other.digits, &self.digits),
        }
    }

    /// Compare `self == f` with exact mathematical semantics (`NaN`/non-integer is `false`).
    pub fn eq_f64(&self, f: f64) -> bool {
        Self::from_f64_exact(f).is_some_and(|rhs| self.cmp_val(&rhs) == Ordering::Equal)
    }

    /// Exact mathematical comparison against `f64` (`None` if `f.is_nan()`).
    pub fn cmp_f64(&self, f: f64) -> Option<Ordering> {
        if f.is_nan() {
            return None;
        }
        if f == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if f == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let trunc = f.trunc();
        let trunc_bi = Self::from_f64_exact(trunc)?;
        match self.cmp_val(&trunc_bi) {
            Ordering::Equal => {
                let frac = f - trunc;
                if frac > 0.0 {
                    Some(Ordering::Less)
                } else if frac < 0.0 {
                    Some(Ordering::Greater)
                } else {
                    Some(Ordering::Equal)
                }
            }
            ord => Some(ord),
        }
    }

    /// Signed addition `self + other`.
    pub fn add(&self, other: &Self) -> Self {
        if self.neg == other.neg {
            return Self::from_parts(self.neg, add_mag(&self.digits, &other.digits));
        }
        match cmp_mag(&self.digits, &other.digits) {
            Ordering::Equal => Self::ZERO,
            Ordering::Greater => Self::from_parts(self.neg, sub_mag(&self.digits, &other.digits)),
            Ordering::Less => Self::from_parts(other.neg, sub_mag(&other.digits, &self.digits)),
        }
    }

    /// Signed subtraction `self - other`.
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    /// Signed multiplication `self * other`.
    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::ZERO;
        }
        let neg = self.neg ^ other.neg;
        let mut out = vec![0u32; self.digits.len() + other.digits.len()];
        for (i, &a) in self.digits.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.digits.iter().enumerate() {
                let prod = u64::from(a) * u64::from(b) + u64::from(out[i + j]) + carry;
                out[i + j] = (prod & 0xFFFF_FFFF) as u32;
                carry = prod >> 32;
            }
            out[i + other.digits.len()] = carry as u32;
        }
        Self::from_parts(neg, out)
    }

    /// Signed truncating division & remainder `(q, r)` where `q` truncates toward zero.
    pub fn div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() {
            return None;
        }
        if self.is_zero() {
            return Some((Self::ZERO, Self::ZERO));
        }
        match cmp_mag(&self.digits, &other.digits) {
            Ordering::Less => Some((Self::ZERO, self.clone())),
            Ordering::Equal => {
                let q_neg = self.neg ^ other.neg;
                Some((Self::from_parts(q_neg, vec![1]), Self::ZERO))
            }
            Ordering::Greater => {
                let (q_mag, r_mag) = div_rem_mag(&self.digits, &other.digits);
                let q = Self::from_parts(self.neg ^ other.neg, q_mag);
                let r = Self::from_parts(self.neg, r_mag);
                Some((q, r))
            }
        }
    }

    /// Exponentiation `self ** exp` returning `None` on overflow above `MAX_BIGINT_POW_BITS`.
    pub fn pow_checked(&self, exp: &Self) -> Result<Self, &'static str> {
        if exp.neg {
            return Err("Exponent must be positive");
        }
        if exp.is_zero() {
            return Ok(Self::from_i64(1));
        }
        if self.is_zero() {
            return Ok(Self::ZERO);
        }
        if self.digits == [1] {
            let is_odd = (self.neg) && (exp.digits[0] & 1 == 1);
            return Ok(Self::from_parts(is_odd, vec![1]));
        }
        let Some(e) = exp.to_i64_exact() else {
            return Err("BigInt exponent too large");
        };
        let e = e as u64;
        let bit_len = self.bit_len() as u64;
        if bit_len.saturating_mul(e) > MAX_BIGINT_POW_BITS {
            return Err("BigInt exponent too large");
        }
        let mut base = self.clone();
        let mut res = Self::from_i64(1);
        let mut k = e;
        while k > 0 {
            if k & 1 == 1 {
                res = res.mul(&base);
            }
            k >>= 1;
            if k > 0 {
                base = base.mul(&base);
            }
        }
        Ok(res)
    }

    /// Bitwise NOT (`~x == -(x + 1)`).
    pub fn bit_not(&self) -> Self {
        self.add(&Self::from_i64(1)).neg()
    }

    /// Two's-complement bitwise AND (`&`).
    pub fn bit_and(&self, other: &Self) -> Self {
        self.bitwise_twos(other, |a, b| a & b)
    }

    /// Two's-complement bitwise OR (`|`).
    pub fn bit_or(&self, other: &Self) -> Self {
        self.bitwise_twos(other, |a, b| a | b)
    }

    /// Two's-complement bitwise XOR (`^`).
    pub fn bit_xor(&self, other: &Self) -> Self {
        self.bitwise_twos(other, |a, b| a ^ b)
    }

    /// Left shift (`self << shift`). Supports negative shifts per ECMAScript spec.
    pub fn shl(&self, shift: &Self) -> Result<Self, &'static str> {
        let Some(s) = shift.to_i64_exact() else {
            if shift.neg && !self.neg {
                return Ok(Self::ZERO);
            }
            if shift.neg && self.neg {
                return Ok(Self::from_i64(-1));
            }
            return Err("BigInt shift count too large");
        };
        if s < 0 {
            return Ok(self.shr_u64(s.unsigned_abs()));
        }
        if s as u64 > MAX_BIGINT_POW_BITS {
            return Err("BigInt shift count too large");
        }
        Ok(self.shl_usize(s as usize))
    }

    /// Signed right shift (`self >> shift`). Supports negative shifts per ECMAScript spec.
    pub fn shr(&self, shift: &Self) -> Result<Self, &'static str> {
        let Some(s) = shift.to_i64_exact() else {
            if !shift.neg && !self.neg {
                return Ok(Self::ZERO);
            }
            if !shift.neg && self.neg {
                return Ok(Self::from_i64(-1));
            }
            return Err("BigInt shift count too large");
        };
        if s < 0 {
            let cnt = s.unsigned_abs();
            if cnt > MAX_BIGINT_POW_BITS {
                return Err("BigInt shift count too large");
            }
            return Ok(self.shl_usize(cnt as usize));
        }
        Ok(self.shr_u64(s as u64))
    }

    /// ECMAScript `BigInt.asUintN(bits, self)`.
    pub fn as_uint_n(&self, bits: u64) -> Self {
        if bits == 0 || self.is_zero() {
            return Self::ZERO;
        }
        if bits > MAX_BIGINT_POW_BITS {
            if !self.neg {
                return self.clone();
            }
        }
        let n_bits = bits.min(MAX_BIGINT_POW_BITS) as usize;
        let limbs_needed = n_bits.div_ceil(32);
        let (twos, sign_ext) = self.to_twos_limbs(limbs_needed);
        let _ = sign_ext;
        let mut out = twos;
        let rem = n_bits % 32;
        if rem != 0 {
            let mask = (1u32 << rem) - 1;
            if let Some(last) = out.last_mut() {
                *last &= mask;
            }
        }
        Self::from_parts(false, out)
    }

    /// ECMAScript `BigInt.asIntN(bits, self)`.
    pub fn as_int_n(&self, bits: u64) -> Self {
        if bits == 0 {
            return Self::ZERO;
        }
        let u = self.as_uint_n(bits);
        let n_bits = bits.min(MAX_BIGINT_POW_BITS) as usize;
        let top_limb = (n_bits - 1) / 32;
        let top_bit = (n_bits - 1) % 32;
        let negative = u
            .digits
            .get(top_limb)
            .is_some_and(|&w| (w & (1u32 << top_bit)) != 0);
        if !negative {
            return u;
        }
        let mut pow2 = vec![0u32; n_bits / 32 + 1];
        pow2[n_bits / 32] = 1u32 << (n_bits % 32);
        let pow2_bi = Self::from_parts(false, pow2);
        u.sub(&pow2_bi)
    }

    fn shl_usize(&self, bits: usize) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let word_shift = bits / 32;
        let bit_shift = (bits % 32) as u32;
        let mut out = vec![0u32; word_shift + self.digits.len() + 1];
        if bit_shift == 0 {
            out[word_shift..word_shift + self.digits.len()].copy_from_slice(&self.digits);
        } else {
            let mut carry = 0u32;
            for (i, &w) in self.digits.iter().enumerate() {
                out[word_shift + i] = (w << bit_shift) | carry;
                carry = w >> (32 - bit_shift);
            }
            out[word_shift + self.digits.len()] = carry;
        }
        Self::from_parts(self.neg, out)
    }

    fn shr_u64(&self, bits: u64) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let max_bits = (self.digits.len() as u64) * 32;
        if bits >= max_bits {
            return if self.neg {
                Self::from_i64(-1)
            } else {
                Self::ZERO
            };
        }
        let b = bits as usize;
        let word_shift = b / 32;
        let bit_shift = (b % 32) as u32;
        let mut mag = vec![0u32; self.digits.len() - word_shift];
        let mut rem_nonzero = self.digits[..word_shift].iter().any(|&w| w != 0);
        if bit_shift == 0 {
            mag.copy_from_slice(&self.digits[word_shift..]);
        } else {
            for i in 0..mag.len() {
                let lo = self.digits[word_shift + i] >> bit_shift;
                let hi = self
                    .digits
                    .get(word_shift + i + 1)
                    .map(|&w| w << (32 - bit_shift))
                    .unwrap_or(0);
                mag[i] = lo | hi;
            }
            if (self.digits[word_shift] & ((1u32 << bit_shift) - 1)) != 0 {
                rem_nonzero = true;
            }
        }
        let res = Self::from_parts(self.neg, mag);
        if self.neg && rem_nonzero {
            res.sub(&Self::from_i64(1))
        } else {
            res
        }
    }

    fn bit_len(&self) -> usize {
        let Some(&last) = self.digits.last() else {
            return 0;
        };
        (self.digits.len() - 1) * 32 + (32 - last.leading_zeros() as usize)
    }

    fn to_twos_limbs(&self, len: usize) -> (Vec<u32>, u32) {
        if !self.neg {
            let mut out = Vec::with_capacity(len);
            for i in 0..len {
                out.push(*self.digits.get(i).unwrap_or(&0));
            }
            return (out, 0);
        }
        let mut out = Vec::with_capacity(len);
        let mut carry = 1u64;
        for i in 0..len {
            let inv = !(*self.digits.get(i).unwrap_or(&0));
            let sum = u64::from(inv) + carry;
            out.push((sum & 0xFFFF_FFFF) as u32);
            carry = sum >> 32;
        }
        (out, u32::MAX)
    }

    fn bitwise_twos(&self, other: &Self, op: fn(u32, u32) -> u32) -> Self {
        let len = self.digits.len().max(other.digits.len()).max(1);
        let (a_limbs, a_ext) = self.to_twos_limbs(len);
        let (b_limbs, b_ext) = other.to_twos_limbs(len);
        let mut raw = Vec::with_capacity(len);
        for i in 0..len {
            raw.push(op(a_limbs[i], b_limbs[i]));
        }
        let neg = (op(a_ext, b_ext) & 1) != 0;
        if !neg {
            return Self::from_parts(false, raw);
        }
        let mut mag = Vec::with_capacity(len);
        let mut carry = 1u64;
        for w in raw {
            let sum = u64::from(!w) + carry;
            mag.push((sum & 0xFFFF_FFFF) as u32);
            carry = sum >> 32;
        }
        if carry > 0 {
            mag.push(carry as u32);
        }
        Self::from_parts(true, mag)
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for (&x, &y) in a.iter().rev().zip(b.iter().rev()) {
        if x != y {
            return x.cmp(&y);
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let n = a.len().max(b.len());
    let mut out = Vec::with_capacity(n + 1);
    let mut carry = 0u64;
    for i in 0..n {
        let sum = u64::from(*a.get(i).unwrap_or(&0)) + u64::from(*b.get(i).unwrap_or(&0)) + carry;
        out.push((sum & 0xFFFF_FFFF) as u32);
        carry = sum >> 32;
    }
    if carry > 0 {
        out.push(carry as u32);
    }
    out
}

fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &av) in a.iter().enumerate() {
        let diff = i64::from(av) - i64::from(*b.get(i).unwrap_or(&0)) - borrow;
        if diff < 0 {
            out.push((diff + (1i64 << 32)) as u32);
            borrow = 1;
        } else {
            out.push(diff as u32);
            borrow = 0;
        }
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

fn mul_add_small(digits: &mut Vec<u32>, mul: u32, add: u32) {
    let mut carry = u64::from(add);
    for d in digits.iter_mut() {
        let prod = u64::from(*d) * u64::from(mul) + carry;
        *d = (prod & 0xFFFF_FFFF) as u32;
        carry = prod >> 32;
    }
    if carry > 0 {
        digits.push(carry as u32);
    }
}

fn div_rem_small(digits: &mut Vec<u32>, div: u32) -> u32 {
    let mut rem = 0u64;
    let d = u64::from(div);
    for w in digits.iter_mut().rev() {
        let cur = (rem << 32) | u64::from(*w);
        *w = (cur / d) as u32;
        rem = cur % d;
    }
    while digits.last() == Some(&0) {
        digits.pop();
    }
    rem as u32
}

fn div_rem_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if b.len() == 1 {
        let mut q = a.to_vec();
        let rem = div_rem_small(&mut q, b[0]);
        let r = if rem == 0 { Vec::new() } else { vec![rem] };
        return (q, r);
    }
    let total_bits = a.len() * 32;
    let mut q = vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new();
    for bit_idx in (0..total_bits).rev() {
        let word_idx = bit_idx / 32;
        let bit = (a[word_idx] >> (bit_idx % 32)) & 1;
        let mut carry = bit;
        for w in &mut r {
            let next_carry = *w >> 31;
            *w = (*w << 1) | carry;
            carry = next_carry;
        }
        if carry > 0 {
            r.push(carry);
        }
        if cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            q[word_idx] |= 1u32 << (bit_idx % 32);
        }
    }
    while q.last() == Some(&0) {
        q.pop();
    }
    (q, r)
}
