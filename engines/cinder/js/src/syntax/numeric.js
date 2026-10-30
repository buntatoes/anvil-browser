// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Numeric literal values, rounded to the nearest double with ties to
// even, as the language asks (spec 12.9.3.1 and 6.1.6.1).
//
// The common case (at most 15 significant digits and a decimal exponent
// within 22) is exact in plain double arithmetic. Everything else goes
// through a small unsigned big-integer type: an array of 24-bit limbs,
// least significant first. Products and carries stay below 2^53, so the
// arithmetic is exact. Source text is read as code units and `_`
// separators are skipped; the lexer has already checked the syntax.

const LIMB = 16777216;
const LIMB_BITS = 24;
const POSITIVE_INFINITY = 1 / 0;

// 10^0 to 10^22, all exact doubles.
const POW10 = [
  1, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17,
  1e18, 1e19, 1e20, 1e21, 1e22,
];

// Value of one digit code unit (0-9, a-z, A-Z), or 99.
export function digitValue(c) {
  if (c >= 0x30 && c <= 0x39) {
    return c - 0x30;
  }
  if (c >= 0x61 && c <= 0x7a) {
    return c - 0x61 + 10;
  }
  if (c >= 0x41 && c <= 0x5a) {
    return c - 0x41 + 10;
  }
  return 99;
}

// ---- big integers ----

// A big integer from a small non-negative integer.
export function bigFromSmall(x) {
  const a = [];
  let v = x;
  while (v > 0) {
    a.push(v % LIMB);
    v = Math.floor(v / LIMB);
  }
  return a;
}

function bigTrim(a) {
  while (a.length > 0 && a[a.length - 1] === 0) {
    a.pop();
  }
}

// a = a * m + add, in place, for 0 <= m, add < 2^24.
export function bigMulAdd(a, m, add) {
  let carry = add;
  let i = 0;
  while (i < a.length) {
    const t = a[i] * m + carry;
    const hi = Math.floor(t / LIMB);
    a[i] = t - hi * LIMB;
    carry = hi;
    i = i + 1;
  }
  while (carry > 0) {
    const hi = Math.floor(carry / LIMB);
    a.push(carry - hi * LIMB);
    carry = hi;
  }
  bigTrim(a);
}

// a = a * 10^k, in place.
export function bigMulPow10(a, k) {
  let left = k;
  while (left >= 7) {
    bigMulAdd(a, 10000000, 0);
    left = left - 7;
  }
  if (left > 0) {
    bigMulAdd(a, POW10[left], 0);
  }
}

function bitsOf(x) {
  let n = 0;
  let v = x;
  while (v > 0) {
    v = v >>> 1;
    n = n + 1;
  }
  return n;
}

// Number of bits in a (0 for zero).
export function bigBitLength(a) {
  if (a.length === 0) {
    return 0;
  }
  return (a.length - 1) * LIMB_BITS + bitsOf(a[a.length - 1]);
}

// Bit i of a.
export function bigBit(a, i) {
  const limb = Math.floor(i / LIMB_BITS);
  if (limb >= a.length) {
    return 0;
  }
  return (a[limb] >>> (i - limb * LIMB_BITS)) & 1;
}

// Whether any of bits 0 to n-1 of a is set.
function bigAnyBelow(a, n) {
  let i = 0;
  const full = Math.floor(n / LIMB_BITS);
  while (i < full && i < a.length) {
    if (a[i] !== 0) {
      return true;
    }
    i = i + 1;
  }
  const rest = n - full * LIMB_BITS;
  if (rest > 0 && full < a.length) {
    const mask = (1 << rest) - 1;
    if ((a[full] & mask) !== 0) {
      return true;
    }
  }
  return false;
}

// a * 2^s as a new big integer.
export function bigShiftLeft(a, s) {
  const out = [];
  const limbs = Math.floor(s / LIMB_BITS);
  const bits = s - limbs * LIMB_BITS;
  let i = 0;
  while (i < limbs) {
    out.push(0);
    i = i + 1;
  }
  let carry = 0;
  i = 0;
  while (i < a.length) {
    const v = a[i] * (1 << bits) + carry;
    const hi = Math.floor(v / LIMB);
    out.push(v - hi * LIMB);
    carry = hi;
    i = i + 1;
  }
  if (carry > 0) {
    out.push(carry);
  }
  bigTrim(out);
  return out;
}

// -1, 0 or 1 as a < b, a = b, a > b.
export function bigCompare(a, b) {
  if (a.length !== b.length) {
    if (a.length < b.length) {
      return -1;
    }
    return 1;
  }
  let i = a.length - 1;
  while (i >= 0) {
    if (a[i] !== b[i]) {
      if (a[i] < b[i]) {
        return -1;
      }
      return 1;
    }
    i = i - 1;
  }
  return 0;
}

// a = a - b in place, for a >= b.
export function bigSub(a, b) {
  let borrow = 0;
  let i = 0;
  while (i < a.length) {
    let v = a[i] - borrow;
    if (i < b.length) {
      v = v - b[i];
    }
    if (v < 0) {
      v = v + LIMB;
      borrow = 1;
    } else {
      borrow = 0;
    }
    a[i] = v;
    i = i + 1;
  }
  bigTrim(a);
}

// floor(n / d); n becomes the remainder. d must not be zero.
export function bigDivMod(n, d) {
  const q = [];
  let shift = bigBitLength(n) - bigBitLength(d);
  while (shift >= 0) {
    const t = bigShiftLeft(d, shift);
    if (bigCompare(n, t) >= 0) {
      bigSub(n, t);
      const limb = Math.floor(shift / LIMB_BITS);
      while (q.length <= limb) {
        q.push(0);
      }
      q[limb] = q[limb] + (1 << (shift - limb * LIMB_BITS));
    }
    shift = shift - 1;
  }
  bigTrim(q);
  return q;
}

// 2^k as a double, for -1074 <= k <= 1023. Exact: halving and doubling
// a power of two loses nothing in that range.
function pow2(k) {
  let x = 1;
  let i = 0;
  if (k >= 0) {
    while (i < k) {
      x = x * 2;
      i = i + 1;
    }
  } else {
    while (i < -k) {
      x = x / 2;
      i = i + 1;
    }
  }
  return x;
}

// The double nearest to q * 2^scale (plus a little more when `sticky`,
// meaning bits were cut off below q), ties to even.
export function bigToDouble(q, sticky, scale) {
  const len = bigBitLength(q);
  if (len === 0) {
    return 0;
  }
  const top = len - 1 + scale;
  if (top > 1023) {
    return POSITIVE_INFINITY;
  }
  let precision = 53;
  if (top < -1022) {
    precision = 53 - (-1022 - top);
  }
  const drop = len - precision;
  if (drop <= 0) {
    let m = 0;
    let i = len - 1;
    while (i >= 0) {
      m = m * 2 + bigBit(q, i);
      i = i - 1;
    }
    return m * pow2(scale);
  }
  let m = 0;
  let i = len - 1;
  while (i >= drop) {
    m = m * 2 + bigBit(q, i);
    i = i - 1;
  }
  const round = bigBit(q, drop - 1);
  const rest = sticky || bigAnyBelow(q, drop - 1);
  if (round === 1 && (rest || m % 2 === 1)) {
    m = m + 1;
  }
  if (m === 0) {
    return 0;
  }
  return m * pow2(scale + drop);
}

// ---- decimal literals ----

// Value of digits * 10^exp10, with digits a list of decimal digit values
// without leading zeros.
function decimalSlow(digits, exp10) {
  let n = digits.length;
  while (n > 0 && digits[n - 1] === 0) {
    n = n - 1;
  }
  if (n === 0) {
    return 0;
  }
  const exp = exp10 + (digits.length - n);
  // The value is in [10^(n-1+exp), 10^(n+exp)).
  if (n + exp > 310) {
    return POSITIVE_INFINITY;
  }
  if (n + exp < -330) {
    return 0;
  }
  const d = [];
  let i = 0;
  while (i < n) {
    bigMulAdd(d, 10, digits[i]);
    i = i + 1;
  }
  if (exp >= 0) {
    bigMulPow10(d, exp);
    return bigToDouble(d, false, 0);
  }
  const s = bigFromSmall(1);
  bigMulPow10(s, -exp);
  // Scale so the quotient has 55 or 56 bits: enough for 53 bits, a
  // rounding bit and one to spare; the remainder is the sticky bit.
  const shift = 55 + bigBitLength(s) - bigBitLength(d);
  let num = d;
  let den = s;
  if (shift > 0) {
    num = bigShiftLeft(d, shift);
  } else if (shift < 0) {
    den = bigShiftLeft(s, -shift);
  }
  const q = bigDivMod(num, den);
  return bigToDouble(q, num.length > 0, -shift);
}

// Value of a decimal literal in src[start, end): digits, an optional
// fraction and an optional exponent, maybe with `_` separators.
export function decimalValue(src, start, end) {
  // First pass: count significant digits and read the exponent, with no
  // allocation, so the common case stays cheap.
  let i = start;
  let mantissa = 0;
  let sig = 0;
  let scale = 0;
  let seenDot = false;
  let leading = true;
  while (i < end) {
    const c = src[i];
    if (c === 0x2e) {
      seenDot = true;
    } else if (c === 0x65 || c === 0x45) {
      break;
    } else if (c !== 0x5f) {
      const v = c - 0x30;
      if (leading && v === 0) {
        if (seenDot) {
          scale = scale - 1;
        }
      } else {
        leading = false;
        sig = sig + 1;
        if (sig <= 15) {
          mantissa = mantissa * 10 + v;
        }
        if (seenDot) {
          scale = scale - 1;
        }
      }
    }
    i = i + 1;
  }
  let exp = 0;
  if (i < end) {
    i = i + 1;
    let negative = false;
    if (src[i] === 0x2b) {
      i = i + 1;
    } else if (src[i] === 0x2d) {
      negative = true;
      i = i + 1;
    }
    while (i < end) {
      const c = src[i];
      if (c !== 0x5f && exp < 100000000) {
        exp = exp * 10 + (c - 0x30);
      }
      i = i + 1;
    }
    if (negative) {
      exp = -exp;
    }
  }
  if (sig === 0) {
    return 0;
  }
  if (sig <= 15) {
    const e = scale + exp;
    if (e === 0) {
      return mantissa;
    }
    if (e > 0 && e <= 22) {
      return mantissa * POW10[e];
    }
    if (e < 0 && e >= -22) {
      return mantissa / POW10[-e];
    }
  }
  // Second pass: every significant digit.
  const digits = [];
  let fraction = 0;
  seenDot = false;
  leading = true;
  i = start;
  while (i < end) {
    const c = src[i];
    if (c === 0x2e) {
      seenDot = true;
    } else if (c === 0x65 || c === 0x45) {
      break;
    } else if (c !== 0x5f) {
      const v = c - 0x30;
      if (!(leading && v === 0)) {
        leading = false;
        digits.push(v);
      }
      if (seenDot) {
        fraction = fraction + 1;
      }
    }
    i = i + 1;
  }
  return decimalSlow(digits, exp - fraction);
}

// Value of the digits in src[start, end) in radix 2, 8 or 16 (or a
// legacy octal literal), maybe with `_` separators.
export function radixValue(src, start, end, radix) {
  let v = 0;
  let i = start;
  let exact = true;
  while (i < end) {
    const c = src[i];
    if (c !== 0x5f) {
      v = v * radix + digitValue(c);
      if (v >= 9007199254740992) {
        exact = false;
      }
    }
    i = i + 1;
  }
  if (exact) {
    return v;
  }
  const a = [];
  i = start;
  while (i < end) {
    const c = src[i];
    if (c !== 0x5f) {
      bigMulAdd(a, radix, digitValue(c));
    }
    i = i + 1;
  }
  return bigToDouble(a, false, 0);
}

// Decimal digits (values 0-9, most significant first) of the integer
// written with `digits` (code units) in `radix`. For BigInt literals.
export function decimalDigitsOf(digits, radix) {
  // Limbs of 10^7, least significant first.
  const limbs = [];
  let i = 0;
  while (i < digits.length) {
    let carry = digitValue(digits[i]);
    let j = 0;
    while (j < limbs.length) {
      const t = limbs[j] * radix + carry;
      carry = Math.floor(t / 10000000);
      limbs[j] = t - carry * 10000000;
      j = j + 1;
    }
    while (carry > 0) {
      const hi = Math.floor(carry / 10000000);
      limbs.push(carry - hi * 10000000);
      carry = hi;
    }
    i = i + 1;
  }
  const out = [];
  if (limbs.length === 0) {
    out.push(0);
    return out;
  }
  let k = limbs.length - 1;
  while (k >= 0) {
    let v = limbs[k];
    const part = [];
    let n = 0;
    while (n < 7) {
      part.push(v % 10);
      v = Math.floor(v / 10);
      n = n + 1;
    }
    let m = 6;
    while (m >= 0) {
      if (out.length > 0 || part[m] !== 0) {
        out.push(part[m]);
      }
      m = m - 1;
    }
    k = k - 1;
  }
  if (out.length === 0) {
    out.push(0);
  }
  return out;
}
