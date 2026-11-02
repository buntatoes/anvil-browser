// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilcompress

import "encoding/binary"

// GzipDecompress decompresses an RFC 1952 gzip stream (including concatenated
// multi-member streams per RFC 1952 §2.2) using the default 32 MiB cap.
func GzipDecompress(input []byte) ([]byte, error) {
	return GzipDecompressBounded(input, DefaultMaxDecompressedBytes)
}

// GzipDecompressBounded decompresses an RFC 1952 gzip stream (single or
// concatenated multi-member) up to maxOutputBytes.
func GzipDecompressBounded(input []byte, maxOutputBytes int) ([]byte, error) {
	if len(input) < 18 {
		return nil, &CompressError{Kind: ErrUnexpectedEof}
	}

	var combined []byte
	pos := 0
	for pos < len(input) {
		if len(input)-pos < 18 {
			return nil, &CompressError{Kind: ErrUnexpectedEof}
		}
		if input[pos] != 0x1F || input[pos+1] != 0x8B {
			return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "gzip magic mismatch"}
		}
		if input[pos+2] != 8 {
			return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "gzip compression method must be 8"}
		}
		flg := input[pos+3]
		pos += 10 // skip ID1, ID2, CM, FLG, MTIME(4), XFL, OS

		// FEXTRA
		if flg&0x04 != 0 {
			if pos+2 > len(input) {
				return nil, &CompressError{Kind: ErrUnexpectedEof}
			}
			xlen := int(binary.LittleEndian.Uint16(input[pos : pos+2]))
			pos += 2
			if pos+xlen > len(input) {
				return nil, &CompressError{Kind: ErrUnexpectedEof}
			}
			pos += xlen
		}

		// FNAME (zero-terminated)
		if flg&0x08 != 0 {
			for pos < len(input) && input[pos] != 0 {
				pos++
			}
			if pos >= len(input) {
				return nil, &CompressError{Kind: ErrUnexpectedEof}
			}
			pos++
		}

		// FCOMMENT (zero-terminated)
		if flg&0x10 != 0 {
			for pos < len(input) && input[pos] != 0 {
				pos++
			}
			if pos >= len(input) {
				return nil, &CompressError{Kind: ErrUnexpectedEof}
			}
			pos++
		}

		// FHCRC (2-byte header CRC16)
		if flg&0x02 != 0 {
			if pos+2 > len(input) {
				return nil, &CompressError{Kind: ErrUnexpectedEof}
			}
			pos += 2
		}

		if pos+8 > len(input) {
			return nil, &CompressError{Kind: ErrUnexpectedEof}
		}

		remainingBudget := maxOutputBytes - len(combined)
		memberOut, nextPos, err := deflateDecodeStream(input, pos, remainingBudget)
		if err != nil {
			return nil, err
		}
		pos = nextPos
		if pos+8 > len(input) {
			return nil, &CompressError{Kind: ErrUnexpectedEof}
		}

		expectedCrc := binary.LittleEndian.Uint32(input[pos : pos+4])
		expectedIsize := binary.LittleEndian.Uint32(input[pos+4 : pos+8])
		pos += 8

		actualCrc := Crc32(memberOut)
		if expectedCrc != actualCrc {
			return nil, &CompressError{
				Kind:     ErrChecksumMismatch,
				Expected: expectedCrc,
				Actual:   actualCrc,
			}
		}
		actualIsize := uint32(len(memberOut))
		if expectedIsize != actualIsize {
			return nil, &CompressError{
				Kind:     ErrLengthMismatch,
				Expected: expectedIsize,
				Actual:   actualIsize,
			}
		}

		combined = append(combined, memberOut...)
	}

	return combined, nil
}

// GzipCompress compresses input into an RFC 1952 gzip stream with CRC-32 and ISIZE trailer.
func GzipCompress(input []byte) []byte {
	deflated := DeflateCompress(input)
	out := make([]byte, 0, len(deflated)+18)
	// ID1=0x1f, ID2=0x8b, CM=8, FLG=0, MTIME=0, XFL=0, OS=255
	out = append(out, 0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF)
	out = append(out, deflated...)
	var tail [8]byte
	binary.LittleEndian.PutUint32(tail[0:4], Crc32(input))
	binary.LittleEndian.PutUint32(tail[4:8], uint32(len(input)))
	out = append(out, tail[:]...)
	return out
}

// DecompressAuto automatically detects gzip (0x1f 0x8b), zlib (RFC 1950 header),
// or raw DEFLATE and decompresses up to maxOutputBytes.
func DecompressAuto(input []byte, maxOutputBytes int) ([]byte, error) {
	if len(input) >= 2 && input[0] == 0x1F && input[1] == 0x8B {
		return GzipDecompressBounded(input, maxOutputBytes)
	}
	if len(input) >= 6 {
		cmf := input[0]
		flg := input[1]
		if (cmf&0x0F) == 8 && (cmf>>4) <= 7 && (flg&0x20) == 0 && ((uint16(cmf)<<8)|uint16(flg))%31 == 0 {
			if out, err := ZlibDecompressBounded(input, maxOutputBytes); err == nil {
				return out, nil
			}
		}
	}
	return DeflateDecompressBounded(input, maxOutputBytes)
}
