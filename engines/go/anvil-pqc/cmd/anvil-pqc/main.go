// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package main

import (
	"bufio"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"

	anvilpqc "github.com/buntatoes/anvil-pqc"
)

const (
	opFillRandom                byte = 1
	opSHA3_256                  byte = 2
	opSHA3_512                  byte = 3
	opSHAKE128                  byte = 4
	opSHAKE256                  byte = 5
	opX25519                    byte = 6
	opX25519PublicKey           byte = 7
	opX25519SharedSecret        byte = 8
	opMLKEMKeyGenFromSeed       byte = 9
	opMLKEMKeyGen               byte = 10
	opMLKEMCheckEK              byte = 11
	opMLKEMEncapsWith           byte = 12
	opMLKEMEncaps               byte = 13
	opMLKEMCheckDK              byte = 14
	opMLKEMDecaps               byte = 15
	opHybridClientShareFromSeed byte = 16
	opHybridClientShare         byte = 17
	opHybridServerRespondWith   byte = 18
	opHybridServerRespond       byte = 19
	opHybridClientFinish        byte = 20

	maxPartsCount         = 1024
	maxPartBytes          = 160 * 1024 * 1024
	maxTotalPartBytes     = 160 * 1024 * 1024
	maxRandomOrShakeBytes = 16 * 1024 * 1024
	maxKeyOrShareBytes    = 64 * 1024
)

func zeroBytes(buf []byte) {
	for i := range buf {
		buf[i] = 0
	}
}

func errorCode(err error) byte {
	var pe *anvilpqc.Error
	if errors.As(err, &pe) {
		return byte(pe.Code)
	}
	return byte(anvilpqc.ErrCodeBadLength)
}

func readParts(br *bufio.Reader, hdr []byte) ([][]byte, error) {
	if _, err := io.ReadFull(br, hdr[:4]); err != nil {
		return nil, err
	}
	countU32 := binary.LittleEndian.Uint32(hdr[:4])
	if countU32 > maxPartsCount {
		return nil, anvilpqc.ErrBadLength
	}
	count := int(countU32)
	parts := make([][]byte, count)
	total := 0
	for i := 0; i < count; i++ {
		if _, err := io.ReadFull(br, hdr[:4]); err != nil {
			return nil, err
		}
		nU32 := binary.LittleEndian.Uint32(hdr[:4])
		if nU32 > maxPartBytes || int(nU32) > maxTotalPartBytes-total {
			return nil, anvilpqc.ErrBadLength
		}
		n := int(nU32)
		total += n
		buf := make([]byte, n)
		if _, err := io.ReadFull(br, buf); err != nil {
			return nil, err
		}
		parts[i] = buf
	}
	return parts, nil
}

func runWorker(r io.Reader, w io.Writer) error {
	br := bufio.NewReaderSize(r, 64*1024)
	bw := bufio.NewWriterSize(w, 64*1024)
	var hdr [16]byte

	for {
		op, err := br.ReadByte()
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}

		switch op {
		case opFillRandom:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			nU32 := binary.LittleEndian.Uint32(hdr[:4])
			if nU32 > maxRandomOrShakeBytes {
				return anvilpqc.ErrBadLength
			}
			buf := make([]byte, int(nU32))
			if err := anvilpqc.FillRandom(buf); err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(buf)
			}
			zeroBytes(buf)

		case opSHA3_256:
			parts, err := readParts(br, hdr[:])
			if err != nil {
				return err
			}
			sum := anvilpqc.SHA3_256(parts...)
			_ = bw.WriteByte(0)
			_, _ = bw.Write(sum[:])

		case opSHA3_512:
			parts, err := readParts(br, hdr[:])
			if err != nil {
				return err
			}
			sum := anvilpqc.SHA3_512(parts...)
			_ = bw.WriteByte(0)
			_, _ = bw.Write(sum[:])

		case opSHAKE128:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			outLenU32 := binary.LittleEndian.Uint32(hdr[:4])
			if outLenU32 > maxRandomOrShakeBytes {
				return anvilpqc.ErrBadLength
			}
			parts, err := readParts(br, hdr[:])
			if err != nil {
				return err
			}
			out := anvilpqc.SHAKE128Bytes(int(outLenU32), parts...)
			_ = bw.WriteByte(0)
			_, _ = bw.Write(out)

		case opSHAKE256:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			outLenU32 := binary.LittleEndian.Uint32(hdr[:4])
			if outLenU32 > maxRandomOrShakeBytes {
				return anvilpqc.ErrBadLength
			}
			parts, err := readParts(br, hdr[:])
			if err != nil {
				return err
			}
			out := anvilpqc.SHAKE256Bytes(int(outLenU32), parts...)
			_ = bw.WriteByte(0)
			_, _ = bw.Write(out)

		case opX25519:
			var scalar, u [32]byte
			if _, err := io.ReadFull(br, scalar[:]); err != nil {
				return err
			}
			if _, err := io.ReadFull(br, u[:]); err != nil {
				zeroBytes(scalar[:])
				return err
			}
			out := anvilpqc.X25519(&scalar, &u)
			zeroBytes(scalar[:])
			_ = bw.WriteByte(0)
			_, _ = bw.Write(out[:])
			zeroBytes(out[:])

		case opX25519PublicKey:
			var secret [32]byte
			if _, err := io.ReadFull(br, secret[:]); err != nil {
				return err
			}
			pub := anvilpqc.X25519PublicKey(&secret)
			zeroBytes(secret[:])
			_ = bw.WriteByte(0)
			_, _ = bw.Write(pub[:])

		case opX25519SharedSecret:
			var secret, peer [32]byte
			if _, err := io.ReadFull(br, secret[:]); err != nil {
				return err
			}
			if _, err := io.ReadFull(br, peer[:]); err != nil {
				zeroBytes(secret[:])
				return err
			}
			ss, err := anvilpqc.X25519SharedSecret(&secret, &peer)
			zeroBytes(secret[:])
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(ss[:])
				zeroBytes(ss[:])
			}

		case opMLKEMKeyGenFromSeed:
			var seed [anvilpqc.SeedLen]byte
			if _, err := io.ReadFull(br, seed[:]); err != nil {
				return err
			}
			ek, dk := anvilpqc.KeyGenFromSeed(&seed)
			zeroBytes(seed[:])
			_ = bw.WriteByte(0)
			_, _ = bw.Write(ek.Bytes())
			_, _ = bw.Write(dk.Bytes())
			dk.Clear()

		case opMLKEMKeyGen:
			ek, dk, err := anvilpqc.KeyGen()
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(ek.Bytes())
				_, _ = bw.Write(dk.Bytes())
				dk.Clear()
			}

		case opMLKEMCheckEK:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			nU32 := binary.LittleEndian.Uint32(hdr[:4])
			if nU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			buf := make([]byte, int(nU32))
			if _, err := io.ReadFull(br, buf); err != nil {
				return err
			}
			if _, err := anvilpqc.EncapsulationKeyFromBytes(buf); err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
			}

		case opMLKEMEncapsWith:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			nU32 := binary.LittleEndian.Uint32(hdr[:4])
			if nU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			ekBuf := make([]byte, int(nU32))
			if _, err := io.ReadFull(br, ekBuf); err != nil {
				return err
			}
			var m [32]byte
			if _, err := io.ReadFull(br, m[:]); err != nil {
				return err
			}
			ek, err := anvilpqc.EncapsulationKeyFromBytes(ekBuf)
			if err != nil {
				zeroBytes(m[:])
				_ = bw.WriteByte(errorCode(err))
			} else {
				ct, ss := ek.EncapsulateWith(&m)
				zeroBytes(m[:])
				_ = bw.WriteByte(0)
				_, _ = bw.Write(ct)
				_, _ = bw.Write(ss[:])
				zeroBytes(ss[:])
			}

		case opMLKEMEncaps:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			nU32 := binary.LittleEndian.Uint32(hdr[:4])
			if nU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			ekBuf := make([]byte, int(nU32))
			if _, err := io.ReadFull(br, ekBuf); err != nil {
				return err
			}
			ek, err := anvilpqc.EncapsulationKeyFromBytes(ekBuf)
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				ct, ss, err := ek.Encapsulate()
				if err != nil {
					_ = bw.WriteByte(errorCode(err))
				} else {
					_ = bw.WriteByte(0)
					_, _ = bw.Write(ct)
					_, _ = bw.Write(ss[:])
					zeroBytes(ss[:])
				}
			}

		case opMLKEMCheckDK:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			nU32 := binary.LittleEndian.Uint32(hdr[:4])
			if nU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			buf := make([]byte, int(nU32))
			if _, err := io.ReadFull(br, buf); err != nil {
				zeroBytes(buf)
				return err
			}
			dk, err := anvilpqc.DecapsulationKeyFromBytes(buf)
			zeroBytes(buf)
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				dk.Clear()
			}

		case opMLKEMDecaps:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			dkLenU32 := binary.LittleEndian.Uint32(hdr[0:4])
			ctLenU32 := binary.LittleEndian.Uint32(hdr[4:8])
			if dkLenU32 > maxKeyOrShareBytes || ctLenU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			dkBuf := make([]byte, int(dkLenU32))
			ctBuf := make([]byte, int(ctLenU32))
			if _, err := io.ReadFull(br, dkBuf); err != nil {
				zeroBytes(dkBuf)
				return err
			}
			if _, err := io.ReadFull(br, ctBuf); err != nil {
				zeroBytes(dkBuf)
				return err
			}
			dk, err := anvilpqc.DecapsulationKeyFromBytes(dkBuf)
			zeroBytes(dkBuf)
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				ss, err := dk.Decapsulate(ctBuf)
				dk.Clear()
				if err != nil {
					_ = bw.WriteByte(errorCode(err))
				} else {
					_ = bw.WriteByte(0)
					_, _ = bw.Write(ss[:])
					zeroBytes(ss[:])
				}
			}

		case opHybridClientShareFromSeed:
			var seed [anvilpqc.SeedLen]byte
			var secret [32]byte
			if _, err := io.ReadFull(br, seed[:]); err != nil {
				return err
			}
			if _, err := io.ReadFull(br, secret[:]); err != nil {
				zeroBytes(seed[:])
				return err
			}
			state, share := anvilpqc.ClientShareFromSeed(&seed, &secret)
			zeroBytes(seed[:])
			zeroBytes(secret[:])
			_ = bw.WriteByte(0)
			_, _ = bw.Write(state.DecapsulationKeyBytes())
			xsec := state.X25519SecretBytes()
			_, _ = bw.Write(xsec[:])
			zeroBytes(xsec[:])
			_, _ = bw.Write(share)
			state.Clear()

		case opHybridClientShare:
			state, share, err := anvilpqc.ClientShare()
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(state.DecapsulationKeyBytes())
				xsec := state.X25519SecretBytes()
				_, _ = bw.Write(xsec[:])
				zeroBytes(xsec[:])
				_, _ = bw.Write(share)
				state.Clear()
			}

		case opHybridServerRespondWith:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			csLenU32 := binary.LittleEndian.Uint32(hdr[:4])
			if csLenU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			clientShare := make([]byte, int(csLenU32))
			if _, err := io.ReadFull(br, clientShare); err != nil {
				return err
			}
			var m, secret [32]byte
			if _, err := io.ReadFull(br, m[:]); err != nil {
				return err
			}
			if _, err := io.ReadFull(br, secret[:]); err != nil {
				zeroBytes(m[:])
				return err
			}
			serverShare, ss, err := anvilpqc.ServerRespondWith(clientShare, &m, &secret)
			zeroBytes(m[:])
			zeroBytes(secret[:])
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(serverShare)
				_, _ = bw.Write(ss[:])
				zeroBytes(ss[:])
			}

		case opHybridServerRespond:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			csLenU32 := binary.LittleEndian.Uint32(hdr[:4])
			if csLenU32 > maxKeyOrShareBytes {
				return anvilpqc.ErrBadLength
			}
			clientShare := make([]byte, int(csLenU32))
			if _, err := io.ReadFull(br, clientShare); err != nil {
				return err
			}
			serverShare, ss, err := anvilpqc.ServerRespond(clientShare)
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				_ = bw.WriteByte(0)
				_, _ = bw.Write(serverShare)
				_, _ = bw.Write(ss[:])
				zeroBytes(ss[:])
			}

		case opHybridClientFinish:
			var dkBuf [anvilpqc.DecapsulationKeyLen]byte
			var secret [32]byte
			if _, err := io.ReadFull(br, dkBuf[:]); err != nil {
				return err
			}
			if _, err := io.ReadFull(br, secret[:]); err != nil {
				zeroBytes(dkBuf[:])
				return err
			}
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				zeroBytes(dkBuf[:])
				zeroBytes(secret[:])
				return err
			}
			ssLenU32 := binary.LittleEndian.Uint32(hdr[:4])
			if ssLenU32 > maxKeyOrShareBytes {
				zeroBytes(dkBuf[:])
				zeroBytes(secret[:])
				return anvilpqc.ErrBadLength
			}
			serverShare := make([]byte, int(ssLenU32))
			if _, err := io.ReadFull(br, serverShare); err != nil {
				zeroBytes(dkBuf[:])
				zeroBytes(secret[:])
				return err
			}
			state, err := anvilpqc.NewClientStateFromParts(dkBuf[:], &secret)
			zeroBytes(dkBuf[:])
			zeroBytes(secret[:])
			if err != nil {
				_ = bw.WriteByte(errorCode(err))
			} else {
				ss, err := state.Finish(serverShare)
				state.Clear()
				if err != nil {
					_ = bw.WriteByte(errorCode(err))
				} else {
					_ = bw.WriteByte(0)
					_, _ = bw.Write(ss[:])
					zeroBytes(ss[:])
				}
			}

		default:
			return fmt.Errorf("unknown anvil-pqc worker opcode %d", op)
		}

		if err := bw.Flush(); err != nil {
			return err
		}
	}
}

func runSelfTest() error {
	var seed [anvilpqc.SeedLen]byte
	for i := range seed {
		seed[i] = byte(i + 1)
	}
	var cSecret, sM, sSecret [32]byte
	for i := 0; i < 32; i++ {
		cSecret[i] = byte(0x21 + i)
		sM[i] = byte(0x41 + i)
		sSecret[i] = byte(0x61 + i)
	}
	state, cShare := anvilpqc.ClientShareFromSeed(&seed, &cSecret)
	defer state.Clear()
	sShare, sSS, err := anvilpqc.ServerRespondWith(cShare, &sM, &sSecret)
	if err != nil {
		return err
	}
	cSS, err := state.Finish(sShare)
	if err != nil {
		return err
	}
	if cSS != sSS {
		return errors.New("hybrid self-test shared secret mismatch")
	}
	return nil
}

func main() {
	if len(os.Args) > 1 && os.Args[1] == "--worker" {
		if err := runWorker(os.Stdin, os.Stdout); err != nil {
			os.Exit(1)
		}
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "--self-test" {
		if err := runSelfTest(); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		fmt.Println("anvil-pqc v.03 OK")
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "sha3-256" {
		data, err := io.ReadAll(io.LimitReader(os.Stdin, maxTotalPartBytes))
		if err != nil {
			os.Exit(1)
		}
		sum := anvilpqc.SHA3_256(data)
		fmt.Println(hex.EncodeToString(sum[:]))
		return
	}
	if err := runSelfTest(); err != nil {
		os.Exit(1)
	}
	fmt.Println("anvil-pqc v.03")
}
