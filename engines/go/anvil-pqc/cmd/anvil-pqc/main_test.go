// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package main

import (
	"bytes"
	"encoding/binary"
	"errors"
	"testing"

	anvilpqc "github.com/buntatoes/anvil-pqc"
)

func TestWorkerRejectsOversizedFramesAndUnknownOpcodes(t *testing.T) {
	// Oversized opFillRandom (0xFFFFFFFF bytes) must fail with ErrBadLength without allocating.
	var req bytes.Buffer
	req.WriteByte(opFillRandom)
	var hdr [4]byte
	binary.LittleEndian.PutUint32(hdr[:], 0xFFFFFFFF)
	req.Write(hdr[:])
	var resp bytes.Buffer
	if err := runWorker(&req, &resp); !errors.Is(err, anvilpqc.ErrBadLength) {
		t.Fatalf("expected ErrBadLength on oversized opFillRandom, got %v", err)
	}

	// Oversized readParts count (0xFFFFFFFF parts) in opSHA3_256 must fail with ErrBadLength.
	req.Reset()
	resp.Reset()
	req.WriteByte(opSHA3_256)
	binary.LittleEndian.PutUint32(hdr[:], 0xFFFFFFFF)
	req.Write(hdr[:])
	if err := runWorker(&req, &resp); !errors.Is(err, anvilpqc.ErrBadLength) {
		t.Fatalf("expected ErrBadLength on oversized readParts count, got %v", err)
	}

	// Oversized opMLKEMCheckEK length (> maxKeyOrShareBytes) must fail with ErrBadLength.
	req.Reset()
	resp.Reset()
	req.WriteByte(opMLKEMCheckEK)
	binary.LittleEndian.PutUint32(hdr[:], maxKeyOrShareBytes+1)
	req.Write(hdr[:])
	if err := runWorker(&req, &resp); !errors.Is(err, anvilpqc.ErrBadLength) {
		t.Fatalf("expected ErrBadLength on oversized opMLKEMCheckEK, got %v", err)
	}

	// Unknown opcode must return an error.
	req.Reset()
	resp.Reset()
	req.WriteByte(0xFE)
	if err := runWorker(&req, &resp); err == nil {
		t.Fatal("expected error on unknown worker opcode")
	}
}
