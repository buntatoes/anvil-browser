// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilpqc

const (
	// HybridCodepoint is the TLS 1.3 NamedGroup codepoint for X25519MLKEM768 (0x11EC = 4588).
	HybridCodepoint uint16 = 0x11ec
	// ClientShareLen is the byte length of the X25519MLKEM768 client key_exchange share (1,216 bytes).
	ClientShareLen = EncapsulationKeyLen + 32
	// ServerShareLen is the byte length of the X25519MLKEM768 server key_exchange share (1,120 bytes).
	ServerShareLen = CiphertextLen + 32
	// HybridSharedSecretLen is the byte length of the combined X25519MLKEM768 shared secret (64 bytes).
	HybridSharedSecretLen = 64
)

// ClientState holds the client's ML-KEM-768 decapsulation key and X25519 private scalar
// between sending the ClientHello share and receiving the ServerHello share.
type ClientState struct {
	mlkem        *DecapsulationKey
	x25519Secret [32]byte
}

// Clear zeroes the secret state held by ClientState.
func (cs *ClientState) Clear() {
	if cs == nil {
		return
	}
	if cs.mlkem != nil {
		cs.mlkem.Clear()
	}
	ctZero(cs.x25519Secret[:])
}

// DecapsulationKeyBytes returns the 2,400-byte ML-KEM-768 decapsulation key for worker serialization.
func (cs *ClientState) DecapsulationKeyBytes() []byte {
	if cs == nil || cs.mlkem == nil {
		return nil
	}
	return cs.mlkem.Bytes()
}

// X25519SecretBytes returns the 32-byte X25519 secret scalar.
func (cs *ClientState) X25519SecretBytes() [32]byte {
	if cs == nil {
		return [32]byte{}
	}
	return cs.x25519Secret
}

// NewClientStateFromParts reconstructs a ClientState from validated ML-KEM-768 decapsulation key bytes
// and a 32-byte X25519 secret scalar.
func NewClientStateFromParts(dkBytes []byte, x25519Secret *[32]byte) (*ClientState, error) {
	dk, err := DecapsulationKeyFromBytes(dkBytes)
	if err != nil {
		return nil, err
	}
	return &ClientState{
		mlkem:        dk,
		x25519Secret: *x25519Secret,
	}, nil
}

// ClientShareFromSeed builds the X25519MLKEM768 client state and 1,216-byte share from a
// 64-byte ML-KEM seed (d || z) and a 32-byte X25519 private scalar.
func ClientShareFromSeed(mlkemSeed *[SeedLen]byte, x25519Secret *[32]byte) (*ClientState, []byte) {
	ek, dk := KeyGenFromSeed(mlkemSeed)
	pub := X25519PublicKey(x25519Secret)
	share := make([]byte, 0, ClientShareLen)
	share = append(share, ek.Bytes()...)
	share = append(share, pub[:]...)
	return &ClientState{
		mlkem:        dk,
		x25519Secret: *x25519Secret,
	}, share
}

// ClientShare generates a fresh X25519MLKEM768 client state and 1,216-byte share from OS randomness.
func ClientShare() (*ClientState, []byte, error) {
	var seed [SeedLen]byte
	var secret [32]byte
	if err := FillRandom(seed[:]); err != nil {
		return nil, nil, err
	}
	if err := FillRandom(secret[:]); err != nil {
		ctZero(seed[:])
		return nil, nil, err
	}
	state, share := ClientShareFromSeed(&seed, &secret)
	ctZero(seed[:])
	ctZero(secret[:])
	return state, share, nil
}

// ServerRespondWith processes a 1,216-byte client share using caller randomness (32-byte m and
// 32-byte X25519 secret), returning the 1,120-byte server share and 64-byte combined shared secret.
func ServerRespondWith(clientShare []byte, m, x25519Secret *[32]byte) ([]byte, [HybridSharedSecretLen]byte, error) {
	if len(clientShare) != ClientShareLen {
		return nil, [HybridSharedSecretLen]byte{}, ErrBadLength
	}
	ek, err := EncapsulationKeyFromBytes(clientShare[:EncapsulationKeyLen])
	if err != nil {
		return nil, [HybridSharedSecretLen]byte{}, err
	}
	var peer [32]byte
	copy(peer[:], clientShare[EncapsulationKeyLen:])
	ecdh, err := X25519SharedSecret(x25519Secret, &peer)
	if err != nil {
		return nil, [HybridSharedSecretLen]byte{}, err
	}
	ct, kem := ek.EncapsulateWith(m)
	pub := X25519PublicKey(x25519Secret)
	share := make([]byte, 0, ServerShareLen)
	share = append(share, ct...)
	share = append(share, pub[:]...)
	return share, combineHybrid(&kem, &ecdh), nil
}

// ServerRespond processes a 1,216-byte client share using OS CSPRNG randomness.
func ServerRespond(clientShare []byte) ([]byte, [HybridSharedSecretLen]byte, error) {
	var m, secret [32]byte
	if err := FillRandom(m[:]); err != nil {
		return nil, [HybridSharedSecretLen]byte{}, err
	}
	if err := FillRandom(secret[:]); err != nil {
		ctZero(m[:])
		return nil, [HybridSharedSecretLen]byte{}, err
	}
	share, ss, err := ServerRespondWith(clientShare, &m, &secret)
	ctZero(m[:])
	ctZero(secret[:])
	return share, ss, err
}

// Finish decapsulates and completes the X25519MLKEM768 key exchange against the 1,120-byte server share.
func (cs *ClientState) Finish(serverShare []byte) ([HybridSharedSecretLen]byte, error) {
	if cs == nil || cs.mlkem == nil {
		return [HybridSharedSecretLen]byte{}, ErrBadDecapsulationKey
	}
	if len(serverShare) != ServerShareLen {
		return [HybridSharedSecretLen]byte{}, ErrBadLength
	}
	ct := serverShare[:CiphertextLen]
	var peer [32]byte
	copy(peer[:], serverShare[CiphertextLen:])
	kem, err := cs.mlkem.Decapsulate(ct)
	if err != nil {
		return [HybridSharedSecretLen]byte{}, err
	}
	ecdh, err := X25519SharedSecret(&cs.x25519Secret, &peer)
	if err != nil {
		ctZero(kem[:])
		return [HybridSharedSecretLen]byte{}, err
	}
	out := combineHybrid(&kem, &ecdh)
	ctZero(kem[:])
	ctZero(ecdh[:])
	return out, nil
}

func combineHybrid(kem, ecdh *[32]byte) [HybridSharedSecretLen]byte {
	var out [HybridSharedSecretLen]byte
	copy(out[:32], kem[:])
	copy(out[32:], ecdh[:])
	return out
}
