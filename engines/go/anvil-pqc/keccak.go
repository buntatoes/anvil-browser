// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilpqc

import (
	"encoding/binary"
	"math/bits"
)

// Rho holds the rotation offsets for Keccak-p[1600, 24] step rho (FIPS 202 Algorithm 2),
// computed from the step definition at package initialization.
var Rho = computeRho()

// RoundConstants holds the 24 round constants for Keccak-p[1600, 24] step iota
// (FIPS 202 Algorithms 5 and 6), computed from the LFSR at package initialization.
var RoundConstants = computeRoundConstants()

func computeRho() [25]uint {
	var out [25]uint
	x, y := 1, 0
	for t := 0; t < 24; t++ {
		out[x+5*y] = uint(((t + 1) * (t + 2) / 2) % 64)
		nextY := (2*x + 3*y) % 5
		x = y
		y = nextY
	}
	return out
}

func rcBit(t int) uint64 {
	steps := t % 255
	if steps == 0 {
		return 1
	}
	var r uint16 = 1
	for i := 1; i <= steps; i++ {
		r <<= 1
		r8 := (r >> 8) & 1
		r ^= r8
		r ^= r8 << 4
		r ^= r8 << 5
		r ^= r8 << 6
		r &= 0xff
	}
	return uint64(r & 1)
}

func computeRoundConstants() [24]uint64 {
	var out [24]uint64
	for ir := 0; ir < 24; ir++ {
		var lane uint64
		for j := 0; j <= 6; j++ {
			lane |= rcBit(j+7*ir) << ((uint(1) << uint(j)) - 1)
		}
		out[ir] = lane
	}
	return out
}

// KeccakF1600 applies the 24-round Keccak-p[1600, 24] permutation to a 25-lane state.
func KeccakF1600(a *[25]uint64) {
	for _, rc := range RoundConstants {
		// theta
		var c [5]uint64
		for x := 0; x < 5; x++ {
			c[x] = a[x] ^ a[x+5] ^ a[x+10] ^ a[x+15] ^ a[x+20]
		}
		for x := 0; x < 5; x++ {
			d := c[(x+4)%5] ^ bits.RotateLeft64(c[(x+1)%5], 1)
			for y := 0; y < 5; y++ {
				a[x+5*y] ^= d
			}
		}
		// rho and pi
		var b [25]uint64
		for x := 0; x < 5; x++ {
			for y := 0; y < 5; y++ {
				src := (x+3*y)%5 + 5*x
				b[x+5*y] = bits.RotateLeft64(a[src], int(Rho[src]))
			}
		}
		// chi
		for y := 0; y < 5; y++ {
			for x := 0; x < 5; x++ {
				a[x+5*y] = b[x+5*y] ^ (^b[(x+1)%5+5*y] & b[(x+2)%5+5*y])
			}
		}
		// iota
		a[0] ^= rc
	}
}

// KeccakF1600X4 permutes four independent Keccak-p[1600, 24] states in lockstep.
func KeccakF1600X4(states *[4][25]uint64) {
	for _, rc := range RoundConstants {
		for s := 0; s < 4; s++ {
			a := &states[s]
			var c [5]uint64
			for x := 0; x < 5; x++ {
				c[x] = a[x] ^ a[x+5] ^ a[x+10] ^ a[x+15] ^ a[x+20]
			}
			for x := 0; x < 5; x++ {
				d := c[(x+4)%5] ^ bits.RotateLeft64(c[(x+1)%5], 1)
				for y := 0; y < 5; y++ {
					a[x+5*y] ^= d
				}
			}
			var b [25]uint64
			for x := 0; x < 5; x++ {
				for y := 0; y < 5; y++ {
					src := (x+3*y)%5 + 5*x
					b[x+5*y] = bits.RotateLeft64(a[src], int(Rho[src]))
				}
			}
			for y := 0; y < 5; y++ {
				for x := 0; x < 5; x++ {
					a[x+5*y] = b[x+5*y] ^ (^b[(x+1)%5+5*y] & b[(x+2)%5+5*y])
				}
			}
			a[0] ^= rc
		}
	}
}

// Sponge is an incremental Keccak sponge with a configurable byte rate and domain-separation suffix.
type Sponge struct {
	state     [25]uint64
	rate      int
	suffix    byte
	pos       int
	squeezing bool
}

func newSponge(rate int, suffix byte) *Sponge {
	return &Sponge{
		rate:   rate,
		suffix: suffix,
	}
}

func (s *Sponge) xorByte(i int, b byte) {
	s.state[i/8] ^= uint64(b) << (8 * (i % 8))
}

func (s *Sponge) getByte(i int) byte {
	return byte(s.state[i/8] >> (8 * (i % 8)))
}

// Absorb feeds more input into the sponge before the first Squeeze call.
func (s *Sponge) Absorb(data []byte) {
	if s.squeezing {
		return
	}
	for _, b := range data {
		s.xorByte(s.pos, b)
		s.pos++
		if s.pos == s.rate {
			KeccakF1600(&s.state)
			s.pos = 0
		}
	}
}

func (s *Sponge) finishAbsorb() {
	s.xorByte(s.pos, s.suffix)
	s.xorByte(s.rate-1, 0x80)
	KeccakF1600(&s.state)
	s.pos = 0
	s.squeezing = true
}

// Squeeze writes len(out) output bytes from the sponge.
func (s *Sponge) Squeeze(out []byte) {
	if !s.squeezing {
		s.finishAbsorb()
	}
	for i := range out {
		if s.pos == s.rate {
			KeccakF1600(&s.state)
			s.pos = 0
		}
		out[i] = s.getByte(s.pos)
		s.pos++
	}
}

// Clear zeroes the internal 1600-bit sponge state.
func (s *Sponge) Clear() {
	for i := range s.state {
		s.state[i] = 0
	}
	s.pos = 0
	s.squeezing = false
}

// NewSHAKE128 returns a new SHAKE128 sponge (rate 168, suffix 0x1f).
func NewSHAKE128() *Sponge {
	return newSponge(168, 0x1f)
}

// NewSHAKE256 returns a new SHAKE256 sponge (rate 136, suffix 0x1f).
func NewSHAKE256() *Sponge {
	return newSponge(136, 0x1f)
}

// SHA3_256 returns the 32-byte FIPS 202 SHA3-256 digest of the concatenation of parts.
func SHA3_256(parts ...[]byte) [32]byte {
	s := newSponge(136, 0x06)
	for _, p := range parts {
		s.Absorb(p)
	}
	var out [32]byte
	s.Squeeze(out[:])
	s.Clear()
	return out
}

// SHA3_512 returns the 64-byte FIPS 202 SHA3-512 digest of the concatenation of parts.
func SHA3_512(parts ...[]byte) [64]byte {
	s := newSponge(72, 0x06)
	for _, p := range parts {
		s.Absorb(p)
	}
	var out [64]byte
	s.Squeeze(out[:])
	s.Clear()
	return out
}

// SHAKE128Bytes squeezes n bytes from SHAKE128 over the concatenation of parts.
func SHAKE128Bytes(n int, parts ...[]byte) []byte {
	if n <= 0 {
		return nil
	}
	s := NewSHAKE128()
	for _, p := range parts {
		s.Absorb(p)
	}
	out := make([]byte, n)
	s.Squeeze(out)
	s.Clear()
	return out
}

// SHAKE256Bytes squeezes n bytes from SHAKE256 over the concatenation of parts.
func SHAKE256Bytes(n int, parts ...[]byte) []byte {
	if n <= 0 {
		return nil
	}
	s := NewSHAKE256()
	for _, p := range parts {
		s.Absorb(p)
	}
	out := make([]byte, n)
	s.Squeeze(out)
	s.Clear()
	return out
}

func loadStateWordsLE(state *[25]uint64, block []byte) {
	for i := 0; i < len(block)/8 && i < 25; i++ {
		state[i] = binary.LittleEndian.Uint64(block[8*i : 8*i+8])
	}
}

func storeStateWordsLE(buf []byte, state *[25]uint64) {
	for i := 0; i < len(buf)/8 && i < 25; i++ {
		binary.LittleEndian.PutUint64(buf[8*i:8*i+8], state[i])
	}
}
