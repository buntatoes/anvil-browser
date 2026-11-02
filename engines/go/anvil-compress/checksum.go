// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

// Package anvilcompress implements clean-room RFC 1951 DEFLATE, RFC 1950 zlib
// (Adler-32), and RFC 1952 gzip (CRC-32) compression and bounded decompression
// with zero external or standard-library codec dependencies.
package anvilcompress

// Version is the semantic version of anvil-compress.
const Version = "0.4.0"

// DefaultMaxDecompressedBytes is the default 32 MiB decompression output cap.
const DefaultMaxDecompressedBytes = 32 * 1024 * 1024

// Adler32 computes the RFC 1950 rolling checksum of data modulo 65521.
func Adler32(data []byte) uint32 {
	const modAdler uint32 = 65521
	var s1 uint32 = 1
	var s2 uint32 = 0

	// Process in chunks of up to 5552 bytes before modulo reduction.
	i := 0
	for i < len(data) {
		end := i + 5552
		if end > len(data) {
			end = len(data)
		}
		for ; i < end; i++ {
			s1 += uint32(data[i])
			s2 += s1
		}
		s1 %= modAdler
		s2 %= modAdler
	}
	return (s2 << 16) | s1
}

var crc32Table = func() [256]uint32 {
	var table [256]uint32
	for i := 0; i < 256; i++ {
		c := uint32(i)
		for bit := 0; bit < 8; bit++ {
			if c&1 != 0 {
				c = 0xEDB88320 ^ (c >> 1)
			} else {
				c >>= 1
			}
		}
		table[i] = c
	}
	return table
}()

// Crc32 computes the RFC 1952 / ISO 3309 CRC-32 checksum of data.
func Crc32(data []byte) uint32 {
	crc := uint32(0xFFFFFFFF)
	for _, b := range data {
		idx := byte(crc) ^ b
		crc = (crc >> 8) ^ crc32Table[idx]
	}
	return ^crc
}
