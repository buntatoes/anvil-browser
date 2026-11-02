// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

// Package anvilpqc implements post-quantum and hybrid key exchange for Anvil:
//   - ML-KEM-768 from NIST FIPS 203 (August 2024).
//   - X25519 from RFC 7748.
//   - X25519MLKEM768 hybrid key shares and combined shared secret for TLS 1.3
//     (draft-ietf-tls-ecdhe-mlkem-03, NamedGroup 0x11EC).
//   - SHA3-256, SHA3-512, SHAKE128, and SHAKE256 from NIST FIPS 202.
package anvilpqc

import (
	"crypto/rand"
	"crypto/subtle"
)

// ErrorCode identifies the failure class without leaking secret material.
type ErrorCode uint8

const (
	// ErrCodeBadLength indicates an input had the wrong number of bytes.
	ErrCodeBadLength ErrorCode = 1
	// ErrCodeBadEncapsulationKey indicates an encapsulation key failed the FIPS 203 §7.2 modulus check.
	ErrCodeBadEncapsulationKey ErrorCode = 2
	// ErrCodeBadDecapsulationKey indicates a decapsulation key failed the FIPS 203 §7.3 hash check.
	ErrCodeBadDecapsulationKey ErrorCode = 3
	// ErrCodeZeroSharedSecret indicates X25519 produced the all-zero output (small-order peer point).
	ErrCodeZeroSharedSecret ErrorCode = 4
	// ErrCodeNoRandomness indicates the operating system CSPRNG could not be read.
	ErrCodeNoRandomness ErrorCode = 5
)

// Error represents a cryptographic validation or randomness failure in anvil-pqc.
type Error struct {
	Code ErrorCode
}

func (e *Error) Error() string {
	switch e.Code {
	case ErrCodeBadLength:
		return "input has the wrong length"
	case ErrCodeBadEncapsulationKey:
		return "encapsulation key failed the modulus check"
	case ErrCodeBadDecapsulationKey:
		return "decapsulation key failed the hash check"
	case ErrCodeZeroSharedSecret:
		return "X25519 shared secret is all zero"
	case ErrCodeNoRandomness:
		return "could not read the system random source"
	default:
		return "unknown pqc error"
	}
}

var (
	// ErrBadLength is returned when an input slice has an unexpected byte length.
	ErrBadLength = &Error{Code: ErrCodeBadLength}
	// ErrBadEncapsulationKey is returned when an ML-KEM-768 encapsulation key fails the FIPS 203 §7.2 modulus check.
	ErrBadEncapsulationKey = &Error{Code: ErrCodeBadEncapsulationKey}
	// ErrBadDecapsulationKey is returned when an ML-KEM-768 decapsulation key fails the FIPS 203 §7.3 hash check.
	ErrBadDecapsulationKey = &Error{Code: ErrCodeBadDecapsulationKey}
	// ErrZeroSharedSecret is returned when X25519 produces an all-zero shared secret.
	ErrZeroSharedSecret = &Error{Code: ErrCodeZeroSharedSecret}
	// ErrNoRandomness is returned when the system CSPRNG fails.
	ErrNoRandomness = &Error{Code: ErrCodeNoRandomness}
)

// ctEq compares two byte slices in constant time when their lengths match.
func ctEq(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	return subtle.ConstantTimeCompare(a, b) == 1
}

// ctSelect overwrites dst with src when choose is true using a constant-time mask.
func ctSelect(dst, src []byte, choose bool) {
	var mask byte
	if choose {
		mask = 0xff
	}
	notMask := ^mask
	for i := range dst {
		dst[i] = (dst[i] & notMask) | (src[i] & mask)
	}
}

// ctZero zeroes a byte slice so secret material does not linger in buffers.
func ctZero(buf []byte) {
	for i := range buf {
		buf[i] = 0
	}
}

// FillRandom fills buf with cryptographically secure random bytes from the OS CSPRNG.
func FillRandom(buf []byte) error {
	if len(buf) == 0 {
		return nil
	}
	n, err := rand.Read(buf)
	if err != nil || n != len(buf) {
		return ErrNoRandomness
	}
	return nil
}
