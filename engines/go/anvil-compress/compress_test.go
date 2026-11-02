// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilcompress

import (
	"bytes"
	"testing"
)

func TestAdler32AndCrc32StandardVectors(t *testing.T) {
	if got := Adler32([]byte("Wikipedia")); got != 0x11E60398 {
		t.Fatalf("expected Adler32 0x11E60398, got 0x%08x", got)
	}
	if got := Crc32([]byte("123456789")); got != 0xCBF43926 {
		t.Fatalf("expected CRC32 0xCBF43926, got 0x%08x", got)
	}
}

func TestStoredBlockRoundtrip(t *testing.T) {
	payload := []byte("Anvil Browser clean-room compression test payload!")
	compressed := DeflateCompress(payload)
	decompressed, err := DeflateDecompress(compressed)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decompressed, payload) {
		t.Fatalf("mismatch: %q vs %q", decompressed, payload)
	}
}

func TestLz77RepetitiveRoundtrip(t *testing.T) {
	var payload []byte
	for i := 0; i < 64; i++ {
		payload = append(payload, []byte("<html><body><h1>Anvil Browser</h1><p>Fast HTTP/2 + TLS 1.3</p></body></html>")...)
	}
	compressed := DeflateCompress(payload)
	if len(compressed) >= len(payload)/4 {
		t.Fatalf("expected strong LZ77 compression, got %d from %d", len(compressed), len(payload))
	}
	decompressed, err := DeflateDecompress(compressed)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decompressed, payload) {
		t.Fatal("repetitive LZ77 roundtrip mismatch")
	}
}

func TestZlibAndGzipRoundtripAndIntegrity(t *testing.T) {
	data := []byte("The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog.")
	z := ZlibCompress(data)
	dz, err := ZlibDecompress(z)
	if err != nil || !bytes.Equal(dz, data) {
		t.Fatalf("zlib roundtrip failed: %v", err)
	}

	CorruptZ := append([]byte(nil), z...)
	CorruptZ[len(CorruptZ)-1] ^= 0x01
	if _, err := ZlibDecompress(CorruptZ); err == nil {
		t.Fatal("expected zlib checksum error")
	}

	g := GzipCompress(data)
	dg, err := GzipDecompress(g)
	if err != nil || !bytes.Equal(dg, data) {
		t.Fatalf("gzip roundtrip failed: %v", err)
	}

	CorruptG := append([]byte(nil), g...)
	CorruptG[len(CorruptG)-5] ^= 0x01
	if _, err := GzipDecompress(CorruptG); err == nil {
		t.Fatal("expected gzip CRC error")
	}
}

func TestMultiMemberGzipAndDecompressAuto(t *testing.T) {
	part1 := []byte("First gzip member. ")
	part2 := []byte("Second gzip member.")
	concat := append(GzipCompress(part1), GzipCompress(part2)...)
	out, err := GzipDecompress(concat)
	if err != nil {
		t.Fatal(err)
	}
	if string(out) != "First gzip member. Second gzip member." {
		t.Fatalf("unexpected multi-member output %q", string(out))
	}

	for _, blob := range [][]byte{
		GzipCompress(part1),
		ZlibCompress(part1),
		DeflateCompress(part1),
	} {
		autoOut, err := DecompressAuto(blob, DefaultMaxDecompressedBytes)
		if err != nil || !bytes.Equal(autoOut, part1) {
			t.Fatalf("DecompressAuto failed: %v", err)
		}
	}
}

func TestBoundedDecompressionRejectsOverflow(t *testing.T) {
	payload := bytes.Repeat([]byte{0x41}, 1024)
	compressed := ZlibCompress(payload)
	if _, err := ZlibDecompressBounded(compressed, 512); err == nil {
		t.Fatal("expected output limit exceeded error")
	}
}
