// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilcompress

import "encoding/binary"

// ZlibDecompress decompresses an RFC 1950 zlib stream using the default 32 MiB cap.
func ZlibDecompress(input []byte) ([]byte, error) {
	return ZlibDecompressBounded(input, DefaultMaxDecompressedBytes)
}

// ZlibDecompressBounded decompresses an RFC 1950 zlib stream up to maxOutputBytes.
func ZlibDecompressBounded(input []byte, maxOutputBytes int) ([]byte, error) {
	if len(input) < 6 {
		return nil, &CompressError{Kind: ErrUnexpectedEof}
	}
	cmf := input[0]
	flg := input[1]
	cm := cmf & 0x0F
	cinfo := cmf >> 4
	if cm != 8 {
		return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "zlib CM must be 8 (DEFLATE)"}
	}
	if cinfo > 7 {
		return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "zlib CINFO window size > 7"}
	}
	if ((uint16(cmf)<<8)|uint16(flg))%31 != 0 {
		return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "zlib CMF/FLG check bits failed"}
	}
	if flg&0x20 != 0 {
		return nil, &CompressError{Kind: ErrInvalidHeader, Detail: "zlib preset dictionary (FDICT) is not supported"}
	}

	deflateSlice := input[2 : len(input)-4]
	out, err := DeflateDecompressBounded(deflateSlice, maxOutputBytes)
	if err != nil {
		return nil, err
	}

	expected := binary.BigEndian.Uint32(input[len(input)-4:])
	actual := Adler32(out)
	if expected != actual {
		return nil, &CompressError{
			Kind:     ErrChecksumMismatch,
			Expected: expected,
			Actual:   actual,
		}
	}
	return out, nil
}

// ZlibCompress compresses input into an RFC 1950 zlib stream with an Adler-32 trailer.
func ZlibCompress(input []byte) []byte {
	deflated := DeflateCompress(input)
	out := make([]byte, 0, len(deflated)+6)
	out = append(out, 0x78, 0x9C)
	out = append(out, deflated...)
	var tail [4]byte
	binary.BigEndian.PutUint32(tail[:], Adler32(input))
	out = append(out, tail[:]...)
	return out
}
