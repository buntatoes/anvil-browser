// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilpqc

import (
	"encoding/binary"
	"math/bits"
)

const feMask uint64 = (1 << 51) - 1

type fe [5]uint64

// Basepoint is the u-coordinate of the RFC 7748 X25519 base point (9).
var Basepoint = [32]byte{9}

func weakCarry(h fe) fe {
	for pass := 0; pass < 2; pass++ {
		for i := 0; i < 4; i++ {
			h[i+1] += h[i] >> 51
			h[i] &= feMask
		}
		h[0] += 19 * (h[4] >> 51)
		h[4] &= feMask
	}
	return h
}

func decodeU(b *[32]byte) fe {
	w0 := binary.LittleEndian.Uint64(b[0:8])
	w1 := binary.LittleEndian.Uint64(b[8:16])
	w2 := binary.LittleEndian.Uint64(b[16:24])
	w3 := binary.LittleEndian.Uint64(b[24:32]) & 0x7fffffffffffffff
	return fe{
		w0 & feMask,
		((w0 >> 51) | (w1 << 13)) & feMask,
		((w1 >> 38) | (w2 << 26)) & feMask,
		((w2 >> 25) | (w3 << 39)) & feMask,
		(w3 >> 12) & feMask,
	}
}

func encodeFe(h fe) [32]byte {
	h = weakCarry(h)
	t := h
	t[0] += 19
	for i := 0; i < 4; i++ {
		t[i+1] += t[i] >> 51
		t[i] &= feMask
	}
	geP := t[4] >> 51
	t[4] &= feMask
	mask := uint64(0) - geP
	var r fe
	for i := 0; i < 5; i++ {
		r[i] = (t[i] & mask) | (h[i] & ^mask)
	}
	words := [4]uint64{
		r[0] | (r[1] << 51),
		(r[1] >> 13) | (r[2] << 38),
		(r[2] >> 26) | (r[3] << 25),
		(r[3] >> 39) | (r[4] << 12),
	}
	var out [32]byte
	for i, w := range words {
		binary.LittleEndian.PutUint64(out[8*i:8*i+8], w)
	}
	return out
}

func feAdd(a, b fe) fe {
	return weakCarry(fe{
		a[0] + b[0],
		a[1] + b[1],
		a[2] + b[2],
		a[3] + b[3],
		a[4] + b[4],
	})
}

func feSub(a, b fe) fe {
	const twoP0 uint64 = 2 * ((1 << 51) - 19)
	const twoP uint64 = 2 * ((1 << 51) - 1)
	return weakCarry(fe{
		a[0] + twoP0 - b[0],
		a[1] + twoP - b[1],
		a[2] + twoP - b[2],
		a[3] + twoP - b[3],
		a[4] + twoP - b[4],
	})
}

type u128 struct {
	hi, lo uint64
}

func mul64(x, y uint64) u128 {
	hi, lo := bits.Mul64(x, y)
	return u128{hi: hi, lo: lo}
}

func add128(a, b u128) u128 {
	lo, carry := bits.Add64(a.lo, b.lo, 0)
	hi := a.hi + b.hi + carry
	return u128{hi: hi, lo: lo}
}

func shr51(a u128) uint64 {
	return (a.lo >> 51) | (a.hi << 13)
}

func carryWide(r [5]u128) fe {
	for i := 0; i < 4; i++ {
		c := shr51(r[i])
		r[i].lo &= feMask
		r[i].hi = 0
		r[i+1] = add128(r[i+1], u128{lo: c})
	}
	c := shr51(r[4])
	r[4].lo &= feMask
	r[4].hi = 0
	r0 := r[0].lo + 19*c
	return weakCarry(fe{
		r0,
		r[1].lo,
		r[2].lo,
		r[3].lo,
		r[4].lo,
	})
}

func feMul(a, b fe) fe {
	b19 := [5]uint64{b[0], 19 * b[1], 19 * b[2], 19 * b[3], 19 * b[4]}
	r0 := add128(add128(add128(add128(mul64(a[0], b[0]), mul64(a[1], b19[4])), mul64(a[2], b19[3])), mul64(a[3], b19[2])), mul64(a[4], b19[1]))
	r1 := add128(add128(add128(add128(mul64(a[0], b[1]), mul64(a[1], b[0])), mul64(a[2], b19[4])), mul64(a[3], b19[3])), mul64(a[4], b19[2]))
	r2 := add128(add128(add128(add128(mul64(a[0], b[2]), mul64(a[1], b[1])), mul64(a[2], b[0])), mul64(a[3], b19[4])), mul64(a[4], b19[3]))
	r3 := add128(add128(add128(add128(mul64(a[0], b[3]), mul64(a[1], b[2])), mul64(a[2], b[1])), mul64(a[3], b[0])), mul64(a[4], b19[4]))
	r4 := add128(add128(add128(add128(mul64(a[0], b[4]), mul64(a[1], b[3])), mul64(a[2], b[2])), mul64(a[3], b[1])), mul64(a[4], b[0]))
	return carryWide([5]u128{r0, r1, r2, r3, r4})
}

func feSquare(a fe) fe {
	return feMul(a, a)
}

func feMulSmall(a fe, k uint64) fe {
	return carryWide([5]u128{
		mul64(a[0], k),
		mul64(a[1], k),
		mul64(a[2], k),
		mul64(a[3], k),
		mul64(a[4], k),
	})
}

func feInvert(z fe) fe {
	var exp [32]byte
	for i := range exp {
		exp[i] = 0xff
	}
	exp[0] = 0xeb
	exp[31] = 0x7f
	result := fe{1, 0, 0, 0, 0}
	for bit := 254; bit >= 0; bit-- {
		result = feSquare(result)
		if ((exp[bit/8] >> uint(bit%8)) & 1) == 1 {
			result = feMul(result, z)
		}
	}
	return result
}

func cswap(swap uint64, a, b *fe) {
	mask := uint64(0) - swap
	for i := 0; i < 5; i++ {
		t := mask & (a[i] ^ b[i])
		a[i] ^= t
		b[i] ^= t
	}
}

// X25519 computes the RFC 7748 §5 scalar multiplication of u by the clamped scalar.
func X25519(scalar, u *[32]byte) [32]byte {
	k := *scalar
	k[0] &= 248
	k[31] &= 127
	k[31] |= 64
	x1 := decodeU(u)
	x2 := fe{1, 0, 0, 0, 0}
	var z2 fe
	x3 := x1
	z3 := fe{1, 0, 0, 0, 0}
	var swap uint64
	for t := 254; t >= 0; t-- {
		kt := uint64((k[t/8] >> uint(t%8)) & 1)
		swap ^= kt
		cswap(swap, &x2, &x3)
		cswap(swap, &z2, &z3)
		swap = kt

		a := feAdd(x2, z2)
		aa := feSquare(a)
		b := feSub(x2, z2)
		bb := feSquare(b)
		e := feSub(aa, bb)
		c := feAdd(x3, z3)
		d := feSub(x3, z3)
		da := feMul(d, a)
		cb := feMul(c, b)
		x3 = feSquare(feAdd(da, cb))
		z3 = feMul(x1, feSquare(feSub(da, cb)))
		x2 = feMul(aa, bb)
		z2 = feMul(e, feAdd(aa, feMulSmall(e, 121665)))
	}
	cswap(swap, &x2, &x3)
	cswap(swap, &z2, &z3)
	ctZero(k[:])
	return encodeFe(feMul(x2, feInvert(z2)))
}

// X25519PublicKey computes the 32-byte X25519 public key for secret: X25519(secret, 9).
func X25519PublicKey(secret *[32]byte) [32]byte {
	return X25519(secret, &Basepoint)
}

// X25519SharedSecret computes X25519(secret, peer) and rejects the all-zero small-order output.
func X25519SharedSecret(secret, peer *[32]byte) ([32]byte, error) {
	k := X25519(secret, peer)
	var acc byte
	for _, b := range k {
		acc |= b
	}
	if acc == 0 {
		return [32]byte{}, ErrZeroSharedSecret
	}
	return k, nil
}
