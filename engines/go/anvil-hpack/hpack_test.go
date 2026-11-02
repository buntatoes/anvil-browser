// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

import (
	"bytes"
	"testing"
)

func TestIntegerRFCExamplesAndRoundtrip(t *testing.T) {
	// C.1.1: 10 with 5-bit prefix
	out := EncodeInteger(10, 5, 0, nil)
	if !bytes.Equal(out, []byte{0x0A}) {
		t.Fatalf("expected [0x0A], got %v", out)
	}
	// C.1.2: 1337 with 5-bit prefix
	out = EncodeInteger(1337, 5, 0, nil)
	if !bytes.Equal(out, []byte{0x1F, 0x9A, 0x0A}) {
		t.Fatalf("expected [0x1F, 0x9A, 0x0A], got %v", out)
	}
	// C.1.3: 42 at octet boundary
	out = EncodeInteger(42, 8, 0, nil)
	if !bytes.Equal(out, []byte{0x2A}) {
		t.Fatalf("expected [0x2A], got %v", out)
	}

	for prefix := uint8(1); prefix <= 8; prefix++ {
		for _, v := range []int{0, 1, 30, 31, 126, 127, 128, 255, 256, 16383, 16384, 1 << 20, MaxInteger} {
			enc := EncodeInteger(v, prefix, 0, nil)
			pos := 0
			dec, err := DecodeInteger(enc, &pos, prefix)
			if err != nil || dec != v || pos != len(enc) {
				t.Fatalf("roundtrip failed for prefix=%d val=%d: %v, %d", prefix, v, err, dec)
			}
		}
	}
}

func TestDynamicTableEvictionAndSnapshot(t *testing.T) {
	dt := NewDynamicTable(100)
	dt.Insert([]byte("a"), []byte("1")) // 34
	dt.Insert([]byte("b"), []byte("2")) // 34
	if dt.Size() != 68 {
		t.Fatalf("expected size 68, got %d", dt.Size())
	}
	dt.Insert([]byte("c"), []byte("3")) // evicts a
	if dt.Len() != 2 {
		t.Fatalf("expected len 2, got %d", dt.Len())
	}
	snap := dt.EntriesSnapshot()
	if len(snap) != 2 || snap[0].NameStr() != "c" || snap[1].NameStr() != "b" {
		t.Fatalf("unexpected snapshot: %+v", snap)
	}
	dt.SetMaxSize(40)
	if dt.Len() != 1 {
		t.Fatalf("expected len 1 after shrink, got %d", dt.Len())
	}
	dt.Insert(bytes.Repeat([]byte("x"), 50), nil)
	if !dt.IsEmpty() || dt.Size() != 0 {
		t.Fatal("expected table cleared by oversized insert")
	}
}

func TestHuffmanRoundtripAndRejections(t *testing.T) {
	all := make([]byte, 256)
	for i := range all {
		all[i] = byte(i)
	}
	enc := HuffmanEncode(all, nil)
	dec, err := HuffmanDecode(enc, 1024)
	if err != nil || !bytes.Equal(dec, all) {
		t.Fatalf("256-byte huffman roundtrip failed: %v", err)
	}

	// Zero-padded 'a' (00011 -> 0x18) is invalid
	if _, err := HuffmanDecode([]byte{0x18}, 10); err == nil {
		t.Fatal("expected error for zero-padded Huffman string")
	}
	// Extra full byte of 1s is invalid
	if _, err := HuffmanDecode([]byte{0x1F, 0xFF}, 10); err == nil {
		t.Fatal("expected error for >7 bits of padding")
	}
	// EOS symbol (30 ones) inside string is invalid
	if _, err := HuffmanDecode([]byte{0xFF, 0xFF, 0xFF, 0xFF}, 10); err == nil {
		t.Fatal("expected error for EOS symbol")
	}
}

func TestDecoderRejectsBadIndexesAndCaps(t *testing.T) {
	dec := NewDecoder()
	if _, err := dec.Decode([]byte{0x80}); err == nil {
		t.Fatal("expected error for index 0")
	}
	if _, err := dec.Decode([]byte{0xBE}); err == nil {
		t.Fatal("expected error for index 62 on empty table")
	}
	if _, err := dec.Decode([]byte{0x82, 0x20}); err == nil {
		t.Fatal("expected error for late table size update")
	}
}
