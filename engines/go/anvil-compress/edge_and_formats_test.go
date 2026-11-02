// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilcompress

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func TestEmptyPayloadRoundtripAcrossFormats(t *testing.T) {
	empty := []byte{}
	if out, err := DeflateDecompress(DeflateCompress(empty)); err != nil || len(out) != 0 {
		t.Fatalf("empty deflate roundtrip failed: %v, len=%d", err, len(out))
	}
	if out, err := ZlibDecompress(ZlibCompress(empty)); err != nil || len(out) != 0 {
		t.Fatalf("empty zlib roundtrip failed: %v, len=%d", err, len(out))
	}
	if out, err := GzipDecompress(GzipCompress(empty)); err != nil || len(out) != 0 {
		t.Fatalf("empty gzip roundtrip failed: %v, len=%d", err, len(out))
	}
}

func TestMultiBlockStoredBoundaryRoundtrip(t *testing.T) {
	// 140,000 pseudo-random non-repeating bytes to exercise multiple 65,535-byte stored blocks
	buf := make([]byte, 140000)
	state := uint32(0x9E3779B9)
	for i := range buf {
		state ^= state << 13
		state ^= state >> 17
		state ^= state << 5
		buf[i] = byte(state)
	}
	compressed := DeflateCompress(buf)
	decompressed, err := DeflateDecompress(compressed)
	if err != nil {
		t.Fatalf("multi-block stored decompress failed: %v", err)
	}
	if !bytes.Equal(decompressed, buf) {
		t.Fatal("multi-block stored roundtrip mismatch")
	}
}

func TestGzipOptionalHeaderFlagsFextraFnameFcommentFhcrc(t *testing.T) {
	payload := []byte("Gzip header flags FEXTRA + FNAME + FCOMMENT + FHCRC payload")
	deflated := DeflateCompress(payload)

	var stream []byte
	// FLG = 0x02 (FHCRC) | 0x04 (FEXTRA) | 0x08 (FNAME) | 0x10 (FCOMMENT) = 0x1E
	stream = append(stream, 0x1F, 0x8B, 0x08, 0x1E, 0x01, 0x02, 0x03, 0x04, 0x00, 0x03)
	// FEXTRA: XLEN=4, "ABCD"
	stream = append(stream, 0x04, 0x00, 'A', 'B', 'C', 'D')
	// FNAME: "archive.txt\x00"
	stream = append(stream, []byte("archive.txt\x00")...)
	// FCOMMENT: "clean-room comment\x00"
	stream = append(stream, []byte("clean-room comment\x00")...)
	// FHCRC: 2 bytes
	stream = append(stream, 0x12, 0x34)
	// Deflate body
	stream = append(stream, deflated...)
	// Trailer: CRC32 + ISIZE
	var tail [8]byte
	binary.LittleEndian.PutUint32(tail[0:4], Crc32(payload))
	binary.LittleEndian.PutUint32(tail[4:8], uint32(len(payload)))
	stream = append(stream, tail[:]...)

	out, err := GzipDecompress(stream)
	if err != nil {
		t.Fatalf("gzip optional header flags failed: %v", err)
	}
	if !bytes.Equal(out, payload) {
		t.Fatalf("gzip optional header flags payload mismatch: %q", out)
	}
}

func TestMalformedDeflateZlibAndGzipRejections(t *testing.T) {
	// Reserved block type 3 (BFINAL=1, BTYPE=11 -> 0b00000111 = 0x07)
	if _, err := DeflateDecompress([]byte{0x07}); err == nil {
		t.Fatal("expected error for reserved DEFLATE block type 3")
	}

	// Stored block LEN/NLEN mismatch
	badStored := []byte{0x01, 0x04, 0x00, 0x00, 0x00, 't', 'e', 's', 't'}
	if _, err := DeflateDecompress(badStored); err == nil {
		t.Fatal("expected error for stored block LEN/NLEN mismatch")
	}

	// Zlib invalid CM (not 8)
	if _, err := ZlibDecompress([]byte{0x79, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01}); err == nil {
		t.Fatal("expected error for invalid zlib CM")
	}

	// Zlib FDICT set
	if _, err := ZlibDecompress([]byte{0x78, 0xBB, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01}); err == nil {
		t.Fatal("expected error for unsupported zlib FDICT")
	}

	// Gzip bad magic
	if _, err := GzipDecompress(make([]byte, 20)); err == nil {
		t.Fatal("expected error for bad gzip magic")
	}

	// Gzip length mismatch in trailer
	goodGzip := GzipCompress([]byte("length check"))
	goodGzip[len(goodGzip)-1] ^= 0x01
	if _, err := GzipDecompress(goodGzip); err == nil {
		t.Fatal("expected error for gzip ISIZE mismatch")
	}
}
