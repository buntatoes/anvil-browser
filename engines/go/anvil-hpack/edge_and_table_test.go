// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

import (
	"bytes"
	"testing"
)

func TestStaticTableFullLookupBounds(t *testing.T) {
	dt := NewDynamicTable(DefaultTableSize)
	if _, _, ok := dt.Get(0); ok {
		t.Fatal("index 0 must not resolve")
	}
	for idx := 1; idx <= StaticTableLen; idx++ {
		name, _, ok := dt.Get(idx)
		if !ok || len(name) == 0 {
			t.Fatalf("static table entry %d failed lookup", idx)
		}
	}
	if _, _, ok := dt.Get(StaticTableLen + 1); ok {
		t.Fatal("index 62 must not resolve when dynamic table is empty")
	}
}

func TestNeverIndexedSensitiveHeadersAndPlainLiteralMode(t *testing.T) {
	enc := NewEncoder()
	enc.SetHuffman(false)
	dec := NewDecoder()

	headers := []Header{
		NewHeader(":method", "POST"),
		NewHeader(":path", "/session/token"),
		NewSensitiveHeader("authorization", "Bearer secret-token-991"),
		NewSensitiveHeader("cookie", "sid=deadbeefcafebabe"),
		NewSensitiveHeader("x-custom-secret", "top-secret-value"),
	}

	wire := enc.Encode(headers, nil)
	decoded, err := dec.Decode(wire)
	if err != nil {
		t.Fatalf("decode failed: %v", err)
	}
	if len(decoded) != len(headers) {
		t.Fatalf("expected %d headers, got %d", len(headers), len(decoded))
	}
	for i := range headers {
		if !decoded[i].Equal(headers[i]) {
			t.Fatalf("header[%d] mismatch: got %+v, want %+v", i, decoded[i], headers[i])
		}
	}

	// Sensitive headers must never enter the dynamic table
	for _, entry := range dec.Table().EntriesSnapshot() {
		if entry.NameStr() == "authorization" || entry.NameStr() == "cookie" || entry.NameStr() == "x-custom-secret" {
			t.Fatalf("sensitive header %q leaked into dynamic table", entry.NameStr())
		}
	}
}

func TestDynamicTableSizeUpdateMinAndFinalSynchronization(t *testing.T) {
	enc := NewEncoder()
	dec := NewDecoder()

	// Shrink to 0 (evicts everything) then grow to 2048 before next block
	enc.SetMaxTableSize(0)
	enc.SetMaxTableSize(2048)

	block := enc.Encode([]Header{
		NewHeader(":method", "GET"),
		NewHeader("x-anvil-test", "table-resize-sync"),
	}, nil)

	decoded, err := dec.Decode(block)
	if err != nil {
		t.Fatalf("decode after table resize failed: %v", err)
	}
	if len(decoded) != 2 || decoded[1].ValueStr() != "table-resize-sync" {
		t.Fatalf("unexpected decoded headers: %+v", decoded)
	}
	if dec.Table().MaxSize() != 2048 {
		t.Fatalf("expected decoder dynamic table max size 2048, got %d", dec.Table().MaxSize())
	}
}

func TestDecoderRejectsOversizedTableUpdateAndHeaderListCap(t *testing.T) {
	dec := NewDecoderWithLimits(1024, 128)
	dec.SetMaxAllowedTableSize(512)

	// Encode a dynamic table size update of 1024 (> 512 allowed)
	updateBlock := EncodeInteger(1024, 5, 0x20, nil)
	if _, err := dec.Decode(updateBlock); err == nil {
		t.Fatal("expected error when table size update exceeds maxAllowedTableSize")
	}

	// Encode a header list exceeding 128 bytes
	enc := NewEncoder()
	bigBlock := enc.Encode([]Header{
		NewHeader("x-large-header", string(bytes.Repeat([]byte("A"), 200))),
	}, nil)
	if _, err := dec.Decode(bigBlock); err == nil {
		t.Fatal("expected error when decoded header list exceeds maxHeaderListSize")
	}
}
