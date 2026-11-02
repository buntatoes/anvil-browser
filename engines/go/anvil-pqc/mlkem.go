// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilpqc

const (
	mlkemN    = 256
	mlkemQ    = 3329
	mlkemK    = 3
	mlkemEta1 = 2
	mlkemEta2 = 2
	mlkemDU   = 10
	mlkemDV   = 4

	// EncapsulationKeyLen is the byte length of an ML-KEM-768 encapsulation key (1,184 bytes).
	EncapsulationKeyLen = 384*mlkemK + 32
	// DecapsulationKeyLen is the byte length of an ML-KEM-768 decapsulation key (2,400 bytes).
	DecapsulationKeyLen = 768*mlkemK + 96
	// CiphertextLen is the byte length of an ML-KEM-768 ciphertext (1,088 bytes).
	CiphertextLen = 32 * (mlkemDU*mlkemK + mlkemDV)
	// SharedSecretLen is the byte length of an ML-KEM-768 shared secret (32 bytes).
	SharedSecretLen = 32
	// SeedLen is the byte length of an ML-KEM-768 key generation seed d || z (64 bytes).
	SeedLen = 64

	dkPKELen = 384 * mlkemK
	xofBlock = 168
)

// Poly is a 256-coefficient polynomial over Z_3329.
type Poly [mlkemN]uint16

// Zetas holds zeta^BitRev7(i) mod q with zeta = 17 (FIPS 203 Appendix A).
var Zetas = computeZetas()

// Gammas holds zeta^(2*BitRev7(i)+1) mod q (FIPS 203 Appendix A).
var Gammas = computeGammas()

func bitRev7(i int) int {
	r := 0
	for b := 0; b < 7; b++ {
		r |= ((i >> uint(b)) & 1) << uint(6-b)
	}
	return r
}

func powModQ(base uint32, exp int) uint16 {
	var result uint32 = 1
	b := base % mlkemQ
	for exp > 0 {
		if exp&1 == 1 {
			result = (result * b) % mlkemQ
		}
		b = (b * b) % mlkemQ
		exp >>= 1
	}
	return uint16(result)
}

func computeZetas() [128]uint16 {
	var out [128]uint16
	for i := 0; i < 128; i++ {
		out[i] = powModQ(17, bitRev7(i))
	}
	return out
}

func computeGammas() [128]uint16 {
	var out [128]uint16
	for i := 0; i < 128; i++ {
		out[i] = powModQ(17, 2*bitRev7(i)+1)
	}
	return out
}

func mulQ(a, b uint16) uint16 {
	return uint16((uint32(a) * uint32(b)) % mlkemQ)
}

func addQ(a, b uint16) uint16 {
	return uint16((uint32(a) + uint32(b)) % mlkemQ)
}

func subQ(a, b uint16) uint16 {
	return uint16((uint32(a) + mlkemQ - uint32(b)) % mlkemQ)
}

// NTT runs FIPS 203 Algorithm 9 in place on f.
func NTT(f *Poly) {
	i := 1
	for length := 128; length >= 2; length /= 2 {
		for start := 0; start < mlkemN; start += 2 * length {
			zeta := Zetas[i]
			i++
			for j := start; j < start+length; j++ {
				t := mulQ(zeta, f[j+length])
				f[j+length] = subQ(f[j], t)
				f[j] = addQ(f[j], t)
			}
		}
	}
}

// NTTInverse runs FIPS 203 Algorithm 10 in place on f.
func NTTInverse(f *Poly) {
	i := 127
	for length := 2; length <= 128; length *= 2 {
		for start := 0; start < mlkemN; start += 2 * length {
			zeta := Zetas[i]
			i--
			for j := start; j < start+length; j++ {
				t := f[j]
				f[j] = addQ(t, f[j+length])
				f[j+length] = mulQ(zeta, subQ(f[j+length], t))
			}
		}
	}
	for idx := range f {
		f[idx] = mulQ(f[idx], 3303)
	}
}

// MultiplyNTTs runs FIPS 203 Algorithms 11 and 12 on two NTT-domain polynomials.
func MultiplyNTTs(f, g *Poly) Poly {
	var h Poly
	for i := 0; i < 128; i++ {
		a0, a1 := f[2*i], f[2*i+1]
		b0, b1 := g[2*i], g[2*i+1]
		h[2*i] = addQ(mulQ(a0, b0), mulQ(mulQ(a1, b1), Gammas[i]))
		h[2*i+1] = addQ(mulQ(a0, b1), mulQ(a1, b0))
	}
	return h
}

// PolyAdd adds b into a coefficient-wise modulo q.
func PolyAdd(a *Poly, b *Poly) {
	for i := 0; i < mlkemN; i++ {
		a[i] = addQ(a[i], b[i])
	}
}

// PolySub returns a - b coefficient-wise modulo q.
func PolySub(a, b *Poly) Poly {
	var out Poly
	for i := 0; i < mlkemN; i++ {
		out[i] = subQ(a[i], b[i])
	}
	return out
}

func compress(x uint16, d int) uint16 {
	scaled := (uint32(x) << uint(d)) + mlkemQ/2
	return uint16((scaled / mlkemQ) & ((1 << uint(d)) - 1))
}

func decompress(y uint16, d int) uint16 {
	return uint16((mlkemQ*uint32(y) + (1 << uint(d-1))) >> uint(d))
}

func byteEncode(f *Poly, d int, out []byte) []byte {
	var acc uint32
	bitsAcc := 0
	for _, c := range f {
		acc |= uint32(c) << uint(bitsAcc)
		bitsAcc += d
		for bitsAcc >= 8 {
			out = append(out, byte(acc))
			acc >>= 8
			bitsAcc -= 8
		}
	}
	return out
}

func byteDecode(b []byte, d int) Poly {
	var f Poly
	var acc uint32
	bitsAcc := 0
	idx := 0
	mask := (uint32(1) << uint(d)) - 1
	for i := 0; i < mlkemN; i++ {
		for bitsAcc < d {
			acc |= uint32(b[idx]) << uint(bitsAcc)
			idx++
			bitsAcc += 8
		}
		v := acc & mask
		acc >>= uint(d)
		bitsAcc -= d
		if d == 12 {
			f[i] = uint16(v % mlkemQ)
		} else {
			f[i] = uint16(v)
		}
	}
	return f
}

func takeCandidates(buf *[xofBlock]byte, a *Poly, count *int) {
	for off := 0; off+3 <= xofBlock; off += 3 {
		d1 := uint16(buf[off]) | (uint16(buf[off+1]&0x0f) << 8)
		d2 := uint16(buf[off+1]>>4) | (uint16(buf[off+2]) << 4)
		if uint32(d1) < mlkemQ && *count < mlkemN {
			a[*count] = d1
			*count++
		}
		if uint32(d2) < mlkemQ && *count < mlkemN {
			a[*count] = d2
			*count++
		}
	}
}

func sampleNTT(rho *[32]byte, j, i byte) Poly {
	xof := NewSHAKE128()
	xof.Absorb(rho[:])
	xof.Absorb([]byte{j, i})
	var a Poly
	count := 0
	var buf [xofBlock]byte
	for count < mlkemN {
		xof.Squeeze(buf[:])
		takeCandidates(&buf, &a, &count)
	}
	xof.Clear()
	return a
}

func sampleNTTX4(rho *[32]byte, which [4][2]int, a *[mlkemK][mlkemK]Poly) {
	var states [4][25]uint64
	for s := 0; s < 4; s++ {
		i, j := which[s][0], which[s][1]
		var block [xofBlock]byte
		copy(block[:32], rho[:])
		block[32] = byte(j)
		block[33] = byte(i)
		block[34] ^= 0x1f
		block[xofBlock-1] ^= 0x80
		loadStateWordsLE(&states[s], block[:])
	}
	var polys [4]Poly
	var counts [4]int
	for counts[0] < mlkemN || counts[1] < mlkemN || counts[2] < mlkemN || counts[3] < mlkemN {
		KeccakF1600X4(&states)
		for s := 0; s < 4; s++ {
			if counts[s] < mlkemN {
				var buf [xofBlock]byte
				storeStateWordsLE(buf[:], &states[s])
				takeCandidates(&buf, &polys[s], &counts[s])
			}
		}
	}
	for s := 0; s < 4; s++ {
		i, j := which[s][0], which[s][1]
		a[i][j] = polys[s]
	}
}

func samplePolyCBD(b []byte) Poly {
	var f Poly
	for i, by := range b {
		for half := 0; half < 2; half++ {
			nib := uint32(by >> uint(4*half))
			x := (nib & 1) + ((nib >> 1) & 1)
			y := ((nib >> 2) & 1) + ((nib >> 3) & 1)
			f[2*i+half] = uint16((x + mlkemQ - y) % mlkemQ)
		}
	}
	return f
}

func prfEta2(s *[32]byte, b byte) []byte {
	return SHAKE256Bytes(128, s[:], []byte{b})
}

func expandMatrix(rho *[32]byte) [mlkemK][mlkemK]Poly {
	var a [mlkemK][mlkemK]Poly
	var quads [2][4][2]int
	for n := 0; n < 8; n++ {
		quads[n/4][n%4] = [2]int{n / mlkemK, n % mlkemK}
	}
	for q := 0; q < 2; q++ {
		sampleNTTX4(rho, quads[q], &a)
	}
	a[2][2] = sampleNTT(rho, 2, 2)
	return a
}

func kpkeKeygen(d *[32]byte) ([]byte, []byte) {
	g := SHA3_512(d[:], []byte{mlkemK})
	var rho, sigma [32]byte
	copy(rho[:], g[:32])
	copy(sigma[:], g[32:])
	a := expandMatrix(&rho)
	var n byte
	var s, e [mlkemK]Poly
	for i := 0; i < mlkemK; i++ {
		s[i] = samplePolyCBD(prfEta2(&sigma, n))
		n++
	}
	for i := 0; i < mlkemK; i++ {
		e[i] = samplePolyCBD(prfEta2(&sigma, n))
		n++
	}
	for i := 0; i < mlkemK; i++ {
		NTT(&s[i])
		NTT(&e[i])
	}
	ek := make([]byte, 0, EncapsulationKeyLen)
	for i := 0; i < mlkemK; i++ {
		t := e[i]
		for j := 0; j < mlkemK; j++ {
			prod := MultiplyNTTs(&a[i][j], &s[j])
			PolyAdd(&t, &prod)
		}
		ek = byteEncode(&t, 12, ek)
	}
	ek = append(ek, rho[:]...)
	dk := make([]byte, 0, dkPKELen)
	for i := 0; i < mlkemK; i++ {
		dk = byteEncode(&s[i], 12, dk)
	}
	for i := 0; i < mlkemK; i++ {
		s[i] = Poly{}
	}
	ctZero(sigma[:])
	return ek, dk
}

func kpkeEncrypt(ek []byte, m, r *[32]byte) []byte {
	var t [mlkemK]Poly
	for i := 0; i < mlkemK; i++ {
		t[i] = byteDecode(ek[384*i:384*(i+1)], 12)
	}
	var rho [32]byte
	copy(rho[:], ek[384*mlkemK:])
	a := expandMatrix(&rho)
	var n byte
	var y, e1 [mlkemK]Poly
	for i := 0; i < mlkemK; i++ {
		y[i] = samplePolyCBD(prfEta2(r, n))
		n++
	}
	for i := 0; i < mlkemK; i++ {
		e1[i] = samplePolyCBD(prfEta2(r, n))
		n++
	}
	e2 := samplePolyCBD(prfEta2(r, n))
	for i := 0; i < mlkemK; i++ {
		NTT(&y[i])
	}
	c := make([]byte, 0, CiphertextLen)
	for i := 0; i < mlkemK; i++ {
		var u Poly
		for j := 0; j < mlkemK; j++ {
			prod := MultiplyNTTs(&a[j][i], &y[j])
			PolyAdd(&u, &prod)
		}
		NTTInverse(&u)
		PolyAdd(&u, &e1[i])
		var comp Poly
		for idx, x := range u {
			comp[idx] = compress(x, mlkemDU)
		}
		c = byteEncode(&comp, mlkemDU, c)
	}
	var v Poly
	for j := 0; j < mlkemK; j++ {
		prod := MultiplyNTTs(&t[j], &y[j])
		PolyAdd(&v, &prod)
	}
	NTTInverse(&v)
	PolyAdd(&v, &e2)
	mu := byteDecode(m[:], 1)
	for idx, x := range mu {
		mu[idx] = decompress(x, 1)
	}
	PolyAdd(&v, &mu)
	var comp Poly
	for idx, x := range v {
		comp[idx] = compress(x, mlkemDV)
	}
	c = byteEncode(&comp, mlkemDV, c)
	return c
}

func kpkeDecrypt(dkPKE, c []byte) [32]byte {
	c1 := c[:32*mlkemDU*mlkemK]
	c2 := c[32*mlkemDU*mlkemK:]
	var wAcc Poly
	for i := 0; i < mlkemK; i++ {
		u := byteDecode(c1[32*mlkemDU*i:32*mlkemDU*(i+1)], mlkemDU)
		for idx, x := range u {
			u[idx] = decompress(x, mlkemDU)
		}
		NTT(&u)
		s := byteDecode(dkPKE[384*i:384*(i+1)], 12)
		prod := MultiplyNTTs(&s, &u)
		PolyAdd(&wAcc, &prod)
	}
	NTTInverse(&wAcc)
	v := byteDecode(c2, mlkemDV)
	for idx, x := range v {
		v[idx] = decompress(x, mlkemDV)
	}
	w := PolySub(&v, &wAcc)
	var bitsPoly Poly
	for idx, x := range w {
		bitsPoly[idx] = compress(x, 1)
	}
	mSlice := byteEncode(&bitsPoly, 1, make([]byte, 0, 32))
	var m [32]byte
	copy(m[:], mSlice)
	return m
}

// EncapsulationKey is a validated ML-KEM-768 encapsulation key (1,184 bytes).
type EncapsulationKey struct {
	raw []byte
}

// EncapsulationKeyFromBytes validates bytes using the FIPS 203 §7.2 length and modulus checks.
func EncapsulationKeyFromBytes(b []byte) (*EncapsulationKey, error) {
	if len(b) != EncapsulationKeyLen {
		return nil, ErrBadLength
	}
	test := make([]byte, 0, dkPKELen)
	for i := 0; i < mlkemK; i++ {
		dec := byteDecode(b[384*i:384*(i+1)], 12)
		test = byteEncode(&dec, 12, test)
	}
	if !ctEq(test, b[:dkPKELen]) {
		return nil, ErrBadEncapsulationKey
	}
	cp := make([]byte, len(b))
	copy(cp, b)
	return &EncapsulationKey{raw: cp}, nil
}

// Bytes returns the 1,184-byte serialized encapsulation key.
func (ek *EncapsulationKey) Bytes() []byte {
	return ek.raw
}

// EncapsulateWith runs FIPS 203 Algorithm 17 (ML-KEM.Encaps_internal) using caller randomness m.
func (ek *EncapsulationKey) EncapsulateWith(m *[32]byte) ([]byte, [SharedSecretLen]byte) {
	h := SHA3_256(ek.raw)
	g := SHA3_512(m[:], h[:])
	var key, r [32]byte
	copy(key[:], g[:32])
	copy(r[:], g[32:])
	ct := kpkeEncrypt(ek.raw, m, &r)
	ctZero(r[:])
	return ct, key
}

// Encapsulate runs FIPS 203 Algorithm 20 (ML-KEM.Encaps) using OS CSPRNG randomness.
func (ek *EncapsulationKey) Encapsulate() ([]byte, [SharedSecretLen]byte, error) {
	var m [32]byte
	if err := FillRandom(m[:]); err != nil {
		return nil, [SharedSecretLen]byte{}, err
	}
	ct, key := ek.EncapsulateWith(&m)
	ctZero(m[:])
	return ct, key, nil
}

// DecapsulationKey is a validated ML-KEM-768 decapsulation key (2,400 bytes).
type DecapsulationKey struct {
	raw []byte
}

// DecapsulationKeyFromBytes validates bytes using the FIPS 203 §7.3 length and hash checks.
func DecapsulationKeyFromBytes(b []byte) (*DecapsulationKey, error) {
	if len(b) != DecapsulationKeyLen {
		return nil, ErrBadLength
	}
	test := SHA3_256(b[384*mlkemK : 768*mlkemK+32])
	if !ctEq(test[:], b[768*mlkemK+32:768*mlkemK+64]) {
		return nil, ErrBadDecapsulationKey
	}
	cp := make([]byte, len(b))
	copy(cp, b)
	return &DecapsulationKey{raw: cp}, nil
}

// Bytes returns the 2,400-byte serialized decapsulation key.
func (dk *DecapsulationKey) Bytes() []byte {
	return dk.raw
}

// EncapsulationKey extracts the matching 1,184-byte encapsulation key from dk.
func (dk *DecapsulationKey) EncapsulationKey() *EncapsulationKey {
	cp := make([]byte, EncapsulationKeyLen)
	copy(cp, dk.raw[384*mlkemK:768*mlkemK+32])
	return &EncapsulationKey{raw: cp}
}

// Clear zeroes the decapsulation key bytes in memory.
func (dk *DecapsulationKey) Clear() {
	ctZero(dk.raw)
}

// Decapsulate runs FIPS 203 Algorithms 21 and 18 with constant-time implicit rejection.
func (dk *DecapsulationKey) Decapsulate(c []byte) ([SharedSecretLen]byte, error) {
	if dk == nil || len(dk.raw) != DecapsulationKeyLen {
		return [SharedSecretLen]byte{}, ErrBadDecapsulationKey
	}
	var allZero [32]byte
	if ctEq(dk.raw[768*mlkemK+32:768*mlkemK+64], allZero[:]) {
		return [SharedSecretLen]byte{}, ErrBadDecapsulationKey
	}
	if len(c) != CiphertextLen {
		return [SharedSecretLen]byte{}, ErrBadLength
	}
	dkPKE := dk.raw[:dkPKELen]
	ekPKE := dk.raw[384*mlkemK : 768*mlkemK+32]
	h := dk.raw[768*mlkemK+32 : 768*mlkemK+64]
	z := dk.raw[768*mlkemK+64:]
	m := kpkeDecrypt(dkPKE, c)
	g := SHA3_512(m[:], h)
	var key, r [32]byte
	copy(key[:], g[:32])
	copy(r[:], g[32:])
	reject := SHAKE256Bytes(32, z, c)
	c2 := kpkeEncrypt(ekPKE, &m, &r)
	same := ctEq(c, c2)
	ctSelect(key[:], reject, !same)
	ctZero(m[:])
	ctZero(r[:])
	ctZero(reject)
	return key, nil
}

// KeyGenFromSeed runs FIPS 203 Algorithm 16 (ML-KEM.KeyGen_internal) from a 64-byte seed (d || z).
func KeyGenFromSeed(seed *[SeedLen]byte) (*EncapsulationKey, *DecapsulationKey) {
	var d [32]byte
	copy(d[:], seed[:32])
	z := seed[32:]
	ek, dkPKE := kpkeKeygen(&d)
	h := SHA3_256(ek)
	dk := make([]byte, 0, DecapsulationKeyLen)
	dk = append(dk, dkPKE...)
	dk = append(dk, ek...)
	dk = append(dk, h[:]...)
	dk = append(dk, z...)
	ctZero(d[:])
	return &EncapsulationKey{raw: ek}, &DecapsulationKey{raw: dk}
}

// KeyGen runs FIPS 203 Algorithm 19 (ML-KEM.KeyGen) using OS CSPRNG randomness.
func KeyGen() (*EncapsulationKey, *DecapsulationKey, error) {
	var seed [SeedLen]byte
	if err := FillRandom(seed[:]); err != nil {
		return nil, nil, err
	}
	ek, dk := KeyGenFromSeed(&seed)
	ctZero(seed[:])
	return ek, dk, nil
}
