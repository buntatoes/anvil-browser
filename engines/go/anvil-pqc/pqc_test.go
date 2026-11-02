// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilpqc

import (
	"bytes"
	"encoding/hex"
	"errors"
	"os"
	"strings"
	"testing"
)

func mustHex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("invalid hex %q: %v", s, err)
	}
	return b
}

func mustHex32(t *testing.T, s string) [32]byte {
	t.Helper()
	b := mustHex(t, s)
	if len(b) != 32 {
		t.Fatalf("expected 32 bytes, got %d", len(b))
	}
	var out [32]byte
	copy(out[:], b)
	return out
}

func TestKeccakDerivedConstantsAndDigests(t *testing.T) {
	if RoundConstants[0] != 1 {
		t.Fatalf("RoundConstants[0] = %d, want 1", RoundConstants[0])
	}
	if Rho[0] != 0 || Rho[1] != 1 {
		t.Fatalf("unexpected Rho[0..1]: %v, %v", Rho[0], Rho[1])
	}
	for i, r := range Rho {
		if r >= 64 {
			t.Fatalf("Rho[%d] = %d >= 64", i, r)
		}
	}

	empty256 := SHA3_256()
	if got := hex.EncodeToString(empty256[:]); got != "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a" {
		t.Fatalf("SHA3_256(empty) = %s", got)
	}
	abc256 := SHA3_256([]byte("abc"))
	if got := hex.EncodeToString(abc256[:]); got != "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532" {
		t.Fatalf("SHA3_256(abc) = %s", got)
	}
	abc512 := SHA3_512([]byte("abc"))
	if got := hex.EncodeToString(abc512[:]); got != "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0" {
		t.Fatalf("SHA3_512(abc) = %s", got)
	}
	sh128 := SHAKE128Bytes(32)
	if got := hex.EncodeToString(sh128); got != "7f9c2ba4e88f827d616045507605853ed73b8093f6efbc88eb1a6eacfa66ef26" {
		t.Fatalf("SHAKE128(empty) = %s", got)
	}
	sh256 := SHAKE256Bytes(32)
	if got := hex.EncodeToString(sh256); got != "46b9dd2b0ba88d13233b3feb743eeb243fcd52ea62b81b82b50c27646ed5762f" {
		t.Fatalf("SHAKE256(empty) = %s", got)
	}
}

func TestKeccakSplitInputAndFourWay(t *testing.T) {
	data := make([]byte, 1000)
	for i := range data {
		data[i] = byte(i * 7)
	}
	whole := SHA3_256(data)
	parts := SHA3_256(data[:135], data[135:137], data[137:])
	if whole != parts {
		t.Fatal("split SHA3_256 did not match one-shot")
	}

	a := NewSHAKE128()
	a.Absorb(data)
	one := make([]byte, 500)
	a.Squeeze(one)

	b := NewSHAKE128()
	b.Absorb(data)
	two := make([]byte, 500)
	b.Squeeze(two[:3])
	b.Squeeze(two[3:200])
	b.Squeeze(two[200:])
	if !bytes.Equal(one, two) {
		t.Fatal("split SHAKE128 squeeze did not match one-shot")
	}

	var states [4][25]uint64
	for s := 0; s < 4; s++ {
		for w := 0; w < 25; w++ {
			states[s][w] = uint64(s*100+w) * 0x0123456789abcdef
		}
	}
	want := states
	for s := 0; s < 4; s++ {
		KeccakF1600(&want[s])
	}
	KeccakF1600X4(&states)
	if states != want {
		t.Fatal("KeccakF1600X4 mismatch against KeccakF1600")
	}
}

func TestX25519RFC7748Vectors(t *testing.T) {
	s1 := mustHex32(t, "a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4")
	u1 := mustHex32(t, "e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
	w1 := mustHex32(t, "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
	if got := X25519(&s1, &u1); got != w1 {
		t.Fatalf("RFC 7748 vector 1 mismatch: %x", got)
	}

	s2 := mustHex32(t, "4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d")
	u2 := mustHex32(t, "e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")
	w2 := mustHex32(t, "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957")
	if got := X25519(&s2, &u2); got != w2 {
		t.Fatalf("RFC 7748 vector 2 mismatch: %x", got)
	}

	iter := func(n int) [32]byte {
		k := Basepoint
		u := Basepoint
		for i := 0; i < n; i++ {
			next := X25519(&k, &u)
			u = k
			k = next
		}
		return k
	}
	if got := iter(1); got != mustHex32(t, "422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079") {
		t.Fatalf("RFC 7748 iter(1) mismatch: %x", got)
	}
	if got := iter(1000); got != mustHex32(t, "684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51") {
		t.Fatalf("RFC 7748 iter(1000) mismatch: %x", got)
	}

	a := mustHex32(t, "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
	b := mustHex32(t, "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb")
	ka := X25519PublicKey(&a)
	kb := X25519PublicKey(&b)
	if ka != mustHex32(t, "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a") {
		t.Fatalf("Alice public key mismatch: %x", ka)
	}
	if kb != mustHex32(t, "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f") {
		t.Fatalf("Bob public key mismatch: %x", kb)
	}
	wantK := mustHex32(t, "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
	ssA, err := X25519SharedSecret(&a, &kb)
	if err != nil || ssA != wantK {
		t.Fatalf("Alice shared secret mismatch: %x, %v", ssA, err)
	}
	ssB, err := X25519SharedSecret(&b, &ka)
	if err != nil || ssB != wantK {
		t.Fatalf("Bob shared secret mismatch: %x, %v", ssB, err)
	}

	var p [32]byte
	for i := range p {
		p[i] = 0xff
	}
	p[0] = 0xed
	p[31] = 0x7f
	p1 := p
	p1[0] = 0xee
	var one [32]byte
	one[0] = 1
	for _, bad := range [][32]byte{{}, one, p, p1} {
		u := bad
		if _, err := X25519SharedSecret(&a, &u); !errors.Is(err, ErrZeroSharedSecret) {
			t.Fatalf("expected ErrZeroSharedSecret for small-order point %x, got %v", u, err)
		}
	}
}

func TestMLKEMAppendixAPolynomialsAndRoundTrips(t *testing.T) {
	appendixAZetas := [128]uint16{
		1, 1729, 2580, 3289, 2642, 630, 1897, 848, 1062, 1919, 193, 797, 2786, 3260, 569, 1746,
		296, 2447, 1339, 1476, 3046, 56, 2240, 1333, 1426, 2094, 535, 2882, 2393, 2879, 1974, 821,
		289, 331, 3253, 1756, 1197, 2304, 2277, 2055, 650, 1977, 2513, 632, 2865, 33, 1320, 1915,
		2319, 1435, 807, 452, 1438, 2868, 1534, 2402, 2647, 2617, 1481, 648, 2474, 3110, 1227, 910,
		17, 2761, 583, 2649, 1637, 723, 2288, 1100, 1409, 2662, 3281, 233, 756, 2156, 3015, 3050,
		1703, 1651, 2789, 1789, 1847, 952, 1461, 2687, 939, 2308, 2437, 2388, 733, 2337, 268, 641,
		1584, 2298, 2037, 3220, 375, 2549, 2090, 1645, 1063, 319, 2773, 757, 2099, 561, 2466, 2594,
		2804, 1092, 403, 1026, 1143, 2150, 2775, 886, 1722, 1212, 1874, 1029, 2110, 2935, 885,
		2154,
	}
	if Zetas != appendixAZetas {
		t.Fatal("Zetas mismatch against FIPS 203 Appendix A")
	}

	var f Poly
	for i := range f {
		f[i] = uint16((i*37 + 11) % mlkemQ)
	}
	orig := f
	NTT(&f)
	NTTInverse(&f)
	if f != orig {
		t.Fatal("NTT -> NTTInverse did not round-trip")
	}

	var a, b Poly
	for i := 0; i < mlkemN; i++ {
		a[i] = uint16((i*7 + 3) % mlkemQ)
		b[i] = uint16((i*i + 5) % mlkemQ)
	}
	var want [mlkemN]int64
	for i := 0; i < mlkemN; i++ {
		for j := 0; j < mlkemN; j++ {
			prod := int64(a[i]) * int64(b[j])
			if i+j < mlkemN {
				want[i+j] += prod
			} else {
				want[i+j-mlkemN] -= prod
			}
		}
	}
	fa, fb := a, b
	NTT(&fa)
	NTT(&fb)
	got := MultiplyNTTs(&fa, &fb)
	NTTInverse(&got)
	for i := 0; i < mlkemN; i++ {
		rem := ((want[i] % mlkemQ) + mlkemQ) % mlkemQ
		if got[i] != uint16(rem) {
			t.Fatalf("MultiplyNTTs coeff %d = %d, want %d", i, got[i], rem)
		}
	}

	for _, d := range []int{1, 4, 10} {
		for y := uint16(0); y < (1 << uint(d)); y++ {
			if compress(decompress(y, d), d) != y {
				t.Fatalf("compress(decompress(%d, %d)) != %d", y, d, y)
			}
		}
		for x := uint16(0); x < mlkemQ; x++ {
			wantC := ((2*(uint64(x)<<uint(d)) + mlkemQ) / (2 * mlkemQ)) % (1 << uint(d))
			if uint64(compress(x, d)) != wantC {
				t.Fatalf("compress(%d, %d) != %d", x, d, wantC)
			}
		}
	}

	for _, d := range []int{1, 4, 10, 12} {
		var p Poly
		mod := 1 << uint(d)
		if d == 12 {
			mod = mlkemQ
		}
		for i := range p {
			p[i] = uint16((i*977 + 13) % mod)
		}
		enc := byteEncode(&p, d, nil)
		if len(enc) != 32*d {
			t.Fatalf("len(byteEncode(d=%d)) = %d", d, len(enc))
		}
		if byteDecode(enc, d) != p {
			t.Fatalf("byteDecode(byteEncode(d=%d)) mismatch", d)
		}
	}
}

func loadACVPCases(t *testing.T) []map[string]string {
	t.Helper()
	paths := []string{"testdata/acvp-mlkem768.txt", "tests/data/acvp-mlkem768.txt"}
	var raw []byte
	var err error
	for _, p := range paths {
		raw, err = os.ReadFile(p)
		if err == nil {
			break
		}
	}
	if err != nil {
		t.Fatalf("failed to read ACVP vectors: %v", err)
	}
	var out []map[string]string
	cur := make(map[string]string)
	lines := append(strings.Split(string(raw), "\n"), "")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "#") {
			continue
		}
		if line == "" {
			if len(cur) > 0 {
				out = append(out, cur)
				cur = make(map[string]string)
			}
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) == 2 {
			cur[parts[0]] = parts[1]
		}
	}
	return out
}

func TestMLKEMACVPVectors(t *testing.T) {
	all := loadACVPCases(t)
	keygenCount, encapCount, decapCount, rejectedCount, ekCheckCount, dkCheckCount := 0, 0, 0, 0, 0, 0

	for _, c := range all {
		switch c["kind"] {
		case "keygen":
			keygenCount++
			var seed [SeedLen]byte
			copy(seed[:32], mustHex(t, c["d"]))
			copy(seed[32:], mustHex(t, c["z"]))
			ek, dk := KeyGenFromSeed(&seed)
			ekHash := SHA3_256(ek.Bytes())
			dkHash := SHA3_256(dk.Bytes())
			if !bytes.Equal(ekHash[:], mustHex(t, c["ek_sha3"])) {
				t.Fatalf("ACVP keygen tc %s ek_sha3 mismatch", c["tc"])
			}
			if !bytes.Equal(dkHash[:], mustHex(t, c["dk_sha3"])) {
				t.Fatalf("ACVP keygen tc %s dk_sha3 mismatch", c["tc"])
			}
			dk.Clear()

		case "encap":
			encapCount++
			ek, err := EncapsulationKeyFromBytes(mustHex(t, c["ek"]))
			if err != nil {
				t.Fatalf("ACVP encap tc %s invalid ek: %v", c["tc"], err)
			}
			m := mustHex32(t, c["m"])
			ct, k := ek.EncapsulateWith(&m)
			if !bytes.Equal(ct, mustHex(t, c["c"])) {
				t.Fatalf("ACVP encap tc %s ciphertext mismatch", c["tc"])
			}
			if !bytes.Equal(k[:], mustHex(t, c["k"])) {
				t.Fatalf("ACVP encap tc %s shared secret mismatch", c["tc"])
			}

		case "decap":
			decapCount++
			if c["reason"] == "modified ciphertext" {
				rejectedCount++
			}
			dk, err := DecapsulationKeyFromBytes(mustHex(t, c["dk"]))
			if err != nil {
				t.Fatalf("ACVP decap tc %s invalid dk: %v", c["tc"], err)
			}
			k, err := dk.Decapsulate(mustHex(t, c["c"]))
			dk.Clear()
			if err != nil {
				t.Fatalf("ACVP decap tc %s failed: %v", c["tc"], err)
			}
			if !bytes.Equal(k[:], mustHex(t, c["k"])) {
				t.Fatalf("ACVP decap tc %s (%s) key mismatch", c["tc"], c["reason"])
			}

		case "ek_check":
			ekCheckCount++
			_, err := EncapsulationKeyFromBytes(mustHex(t, c["ek"]))
			wantOK := c["pass"] == "true"
			if (err == nil) != wantOK {
				t.Fatalf("ACVP ek_check tc %s (%s) got err=%v, wantOK=%v", c["tc"], c["reason"], err, wantOK)
			}

		case "dk_check":
			dkCheckCount++
			_, err := DecapsulationKeyFromBytes(mustHex(t, c["dk"]))
			wantOK := c["pass"] == "true"
			if (err == nil) != wantOK {
				t.Fatalf("ACVP dk_check tc %s (%s) got err=%v, wantOK=%v", c["tc"], c["reason"], err, wantOK)
			}
		}
	}

	if keygenCount != 25 || encapCount == 0 || decapCount == 0 || rejectedCount == 0 || ekCheckCount == 0 || dkCheckCount == 0 {
		t.Fatalf("unexpected ACVP counts: keygen=%d encap=%d decap=%d rejected=%d ek=%d dk=%d",
			keygenCount, encapCount, decapCount, rejectedCount, ekCheckCount, dkCheckCount)
	}
}

func TestHybridX25519MLKEM768RoundTripAndRejection(t *testing.T) {
	state, cShare, err := ClientShare()
	if err != nil {
		t.Fatalf("ClientShare failed: %v", err)
	}
	defer state.Clear()
	if len(cShare) != ClientShareLen {
		t.Fatalf("len(cShare) = %d, want %d", len(cShare), ClientShareLen)
	}

	sShare, sSecret, err := ServerRespond(cShare)
	if err != nil {
		t.Fatalf("ServerRespond failed: %v", err)
	}
	if len(sShare) != ServerShareLen {
		t.Fatalf("len(sShare) = %d, want %d", len(sShare), ServerShareLen)
	}

	cSecret, err := state.Finish(sShare)
	if err != nil {
		t.Fatalf("ClientState.Finish failed: %v", err)
	}
	if cSecret != sSecret {
		t.Fatal("hybrid client and server shared secrets differed")
	}

	// Tampered ML-KEM ciphertext triggers implicit rejection on the first 32 bytes while keeping X25519 second half equal.
	tampered := make([]byte, len(sShare))
	copy(tampered, sShare)
	tampered[0] ^= 0x01
	cTampered, err := state.Finish(tampered)
	if err != nil {
		t.Fatalf("Finish on tampered ciphertext returned error instead of implicit rejection: %v", err)
	}
	if bytes.Equal(cTampered[:32], sSecret[:32]) {
		t.Fatal("implicit rejection produced identical ML-KEM shared secret")
	}
	if !bytes.Equal(cTampered[32:], sSecret[32:]) {
		t.Fatal("X25519 half changed when only ML-KEM ciphertext was tampered")
	}

	// Bad lengths and small-order X25519 peer points are rejected.
	if _, _, err := ServerRespond(cShare[:100]); !errors.Is(err, ErrBadLength) {
		t.Fatalf("expected ErrBadLength, got %v", err)
	}
	if _, err := state.Finish(sShare[:100]); !errors.Is(err, ErrBadLength) {
		t.Fatalf("expected ErrBadLength, got %v", err)
	}
	badPeerShare := make([]byte, len(sShare))
	copy(badPeerShare, sShare[:CiphertextLen])
	if _, err := state.Finish(badPeerShare); !errors.Is(err, ErrZeroSharedSecret) {
		t.Fatalf("expected ErrZeroSharedSecret on zero X25519 peer share, got %v", err)
	}
}

func TestSecurityBoundaryAndAdversarialInputs(t *testing.T) {
	// Negative and zero SHAKE lengths must not panic.
	if got := SHAKE128Bytes(-10, []byte("abc")); got != nil {
		t.Fatalf("expected nil for negative SHAKE128Bytes length, got %v", got)
	}
	if got := SHAKE256Bytes(0, []byte("abc")); got != nil {
		t.Fatalf("expected nil for zero SHAKE256Bytes length, got %v", got)
	}

	// Nil and cleared ClientState / DecapsulationKey must fail cleanly without panicking.
	var nilCS *ClientState
	nilCS.Clear()
	if nilCS.DecapsulationKeyBytes() != nil {
		t.Fatal("expected nil DecapsulationKeyBytes on nil ClientState")
	}
	if _, err := nilCS.Finish(make([]byte, ServerShareLen)); !errors.Is(err, ErrBadDecapsulationKey) {
		t.Fatalf("expected ErrBadDecapsulationKey on nil ClientState.Finish, got %v", err)
	}

	state, cShare, err := ClientShare()
	if err != nil {
		t.Fatalf("ClientShare failed: %v", err)
	}
	sShare, _, err := ServerRespond(cShare)
	if err != nil {
		t.Fatalf("ServerRespond failed: %v", err)
	}
	state.Clear()
	if _, err := state.Finish(sShare); !errors.Is(err, ErrBadDecapsulationKey) {
		t.Fatalf("expected ErrBadDecapsulationKey on cleared ClientState.Finish, got %v", err)
	}

	// Mutated encapsulation key with coefficient >= q (0xFFF = 4095 >= 3329) must be rejected.
	badEK := make([]byte, EncapsulationKeyLen)
	copy(badEK, cShare[:EncapsulationKeyLen])
	badEK[0] = 0xff
	badEK[1] |= 0x0f
	if _, err := EncapsulationKeyFromBytes(badEK); !errors.Is(err, ErrBadEncapsulationKey) {
		t.Fatalf("expected ErrBadEncapsulationKey on out-of-range coefficient, got %v", err)
	}
}
