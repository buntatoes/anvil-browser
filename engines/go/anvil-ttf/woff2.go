// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
)

var woff2KnownTableTags = [63][4]byte{
	{'c', 'm', 'a', 'p'}, {'h', 'e', 'a', 'd'}, {'h', 'h', 'e', 'a'}, {'h', 'm', 't', 'x'},
	{'m', 'a', 'x', 'p'}, {'n', 'a', 'm', 'e'}, {'O', 'S', '/', '2'}, {'p', 'o', 's', 't'},
	{'c', 'v', 't', ' '}, {'f', 'p', 'g', 'm'}, {'g', 'l', 'y', 'f'}, {'l', 'o', 'c', 'a'},
	{'p', 'r', 'e', 'p'}, {'C', 'F', 'F', ' '}, {'V', 'O', 'R', 'G'}, {'E', 'B', 'D', 'T'},
	{'E', 'B', 'L', 'C'}, {'g', 'a', 's', 'p'}, {'h', 'd', 'm', 'x'}, {'k', 'e', 'r', 'n'},
	{'L', 'T', 'S', 'H'}, {'P', 'C', 'L', 'T'}, {'V', 'D', 'M', 'X'}, {'v', 'h', 'e', 'a'},
	{'v', 'm', 't', 'x'}, {'B', 'A', 'S', 'E'}, {'G', 'D', 'E', 'F'}, {'G', 'P', 'O', 'S'},
	{'G', 'S', 'U', 'B'}, {'E', 'B', 'S', 'C'}, {'J', 'S', 'T', 'F'}, {'M', 'A', 'T', 'H'},
	{'C', 'B', 'D', 'T'}, {'C', 'B', 'L', 'C'}, {'C', 'O', 'L', 'R'}, {'C', 'P', 'A', 'L'},
	{'S', 'V', 'G', ' '}, {'s', 'b', 'i', 'x'}, {'a', 'c', 'n', 't'}, {'a', 'v', 'a', 'r'},
	{'b', 'd', 'a', 't'}, {'b', 'l', 'o', 'c'}, {'b', 's', 'l', 'n'}, {'c', 'v', 'a', 'r'},
	{'f', 'd', 's', 'c'}, {'f', 'e', 'a', 't'}, {'f', 'm', 't', 'x'}, {'f', 'v', 'a', 'r'},
	{'g', 'v', 'a', 'r'}, {'h', 's', 't', 'y'}, {'j', 'u', 's', 't'}, {'l', 'c', 'a', 'r'},
	{'m', 'o', 'r', 't'}, {'m', 'o', 'r', 'x'}, {'o', 'p', 'b', 'd'}, {'p', 'r', 'o', 'p'},
	{'t', 'r', 'a', 'k'}, {'Z', 'a', 'p', 'f'}, {'S', 'i', 'l', 'f'}, {'G', 'l', 'a', 't'},
	{'G', 'l', 'o', 'c'}, {'F', 'e', 'a', 't'}, {'S', 'i', 'l', 'l'},
}

var brotliCodeLengthOrder = [18]int{
	1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15,
}

var brotliInsertLenOffset = [24]uint32{
	0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26, 34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210, 22594,
}

var brotliInsertLenNBits = [24]uint8{
	0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24,
}

var brotliCopyLenOffset = [24]uint32{
	2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118,
}

var brotliCopyLenNBits = [24]uint8{
	0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24,
}

var brotliInsertRangeLUT = [11]uint8{0, 0, 0, 0, 8, 8, 0, 16, 8, 16, 16}
var brotliCopyRangeLUT = [11]uint8{0, 8, 0, 8, 0, 8, 16, 0, 16, 8, 16}

var brotliBlockLenOffset = [26]uint32{
	1, 5, 9, 13, 17, 25, 33, 41, 49, 65, 81, 97, 113, 145, 177, 209, 241, 305, 369, 497, 753, 1265, 2289, 4337, 8433, 16625,
}

var brotliBlockLenNBits = [26]uint8{
	2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 24,
}

var brotliDistShortIndex = [16]int{0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1}
var brotliDistShortDelta = [16]int{0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3}

var brotliUTF8LUT0 = [256]uint8{
	0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	8, 12, 16, 12, 12, 20, 12, 16, 24, 28, 12, 12, 32, 12, 36, 12, 44, 44, 44, 44, 44, 44, 44, 44,
	44, 44, 32, 32, 24, 40, 28, 12, 12, 48, 52, 52, 52, 48, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48,
	52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 24, 12, 28, 12, 12, 12, 56, 60, 60, 60, 56, 60, 60,
	60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 24, 12, 28, 12, 0,
	0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
	0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
	2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3,
	2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
}

var brotliUTF8LUT1 = [256]uint8{
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1,
	1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1,
	1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 0,
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
}

var brotliSignedLUT = [256]uint8{
	0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
	2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
	3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
	3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
	4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
	4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
	5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
	5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7,
}

type brotliBitReader struct {
	data      []byte
	bytePos   int
	bitOffset uint8
}

func (br *brotliBitReader) readBits(n uint8) (uint32, bool) {
	if n == 0 {
		return 0, true
	}
	var val uint32
	var shift uint8
	rem := n
	for rem > 0 {
		if br.bytePos >= len(br.data) {
			return 0, false
		}
		avail := uint8(8) - br.bitOffset
		take := rem
		if avail < take {
			take = avail
		}
		mask := uint8((uint16(1) << take) - 1)
		bits := (br.data[br.bytePos] >> br.bitOffset) & mask
		val |= uint32(bits) << shift
		shift += take
		rem -= take
		br.bitOffset += take
		if br.bitOffset == 8 {
			br.bitOffset = 0
			br.bytePos++
		}
	}
	return val, true
}

func (br *brotliBitReader) alignToByte() bool {
	if br.bitOffset != 0 {
		pad, ok := br.readBits(8 - br.bitOffset)
		if !ok || pad != 0 {
			return false
		}
	}
	return true
}

func (br *brotliBitReader) readAlignedBytes(length int) ([]byte, bool) {
	if !br.alignToByte() {
		return nil, false
	}
	if br.bytePos < 0 || length < 0 || br.bytePos+length > len(br.data) {
		return nil, false
	}
	slice := br.data[br.bytePos : br.bytePos+length]
	br.bytePos += length
	return slice, true
}

type brotliPrefixEntry struct {
	code   uint16
	length uint8
	symbol uint16
}

type brotliPrefixTree struct {
	entries []brotliPrefixEntry
	maxLen  uint8
}

func newSingleBrotliTree(sym uint16) *brotliPrefixTree {
	return &brotliPrefixTree{
		entries: []brotliPrefixEntry{{code: 0, length: 0, symbol: sym}},
		maxLen:  0,
	}
}

func newBrotliPrefixTree(lengths []uint8) (*brotliPrefixTree, bool) {
	type symLen struct {
		sym uint16
		len uint8
	}
	var nonZero []symLen
	var maxLen uint8
	for sym, l := range lengths {
		if l > 0 {
			if l > 15 {
				return nil, false
			}
			nonZero = append(nonZero, symLen{sym: uint16(sym), len: l})
			if l > maxLen {
				maxLen = l
			}
		}
	}
	if len(nonZero) == 0 {
		return nil, false
	}
	if len(nonZero) == 1 {
		return newSingleBrotliTree(nonZero[0].sym), true
	}

	var blCount [16]uint32
	for _, sl := range nonZero {
		blCount[sl.len]++
	}
	var nextCode [16]uint16
	var code uint32
	for bits := 1; bits <= 15; bits++ {
		code = (code + blCount[bits-1]) << 1
		nextCode[bits] = uint16(code)
	}

	entries := make([]brotliPrefixEntry, 0, len(nonZero))
	for _, sl := range nonZero {
		c := nextCode[sl.len]
		nextCode[sl.len] = c + 1
		var rev uint16
		v := c
		for i := uint8(0); i < sl.len; i++ {
			rev = (rev << 1) | (v & 1)
			v >>= 1
		}
		entries = append(entries, brotliPrefixEntry{code: rev, length: sl.len, symbol: sl.sym})
	}
	return &brotliPrefixTree{entries: entries, maxLen: maxLen}, true
}

func (pt *brotliPrefixTree) decode(br *brotliBitReader) (uint16, bool) {
	if pt.maxLen == 0 {
		return pt.entries[0].symbol, true
	}
	var acc uint16
	for l := uint8(1); l <= pt.maxLen; l++ {
		bit, ok := br.readBits(1)
		if !ok {
			return 0, false
		}
		acc |= uint16(bit) << (l - 1)
		for _, e := range pt.entries {
			if e.length == l && e.code == acc {
				return e.symbol, true
			}
		}
	}
	return 0, false
}

func brotliAlphabetBitWidth(alphabetSize int) uint8 {
	if alphabetSize <= 1 {
		return 0
	}
	maxSym := uint32(alphabetSize - 1)
	var bits uint8
	for maxSym > 0 {
		bits++
		maxSym >>= 1
	}
	return bits
}

func decodeBrotliClCodeLength(br *brotliBitReader) (uint8, bool) {
	b0, ok := br.readBits(1)
	if !ok {
		return 0, false
	}
	b1, ok := br.readBits(1)
	if !ok {
		return 0, false
	}
	if b0 == 0 && b1 == 0 {
		return 0, true
	}
	if b0 == 1 && b1 == 0 {
		return 4, true
	}
	if b0 == 0 && b1 == 1 {
		b2, ok := br.readBits(1)
		if !ok {
			return 0, false
		}
		if b2 == 0 {
			return 3, true
		}
		return 2, true
	}
	b2, ok := br.readBits(1)
	if !ok {
		return 0, false
	}
	if b2 == 0 {
		return 1, true
	}
	b3, ok := br.readBits(1)
	if !ok {
		return 0, false
	}
	if b3 == 0 {
		return 5, true
	}
	return 0, true
}

func decodeBrotliPrefixTree(br *brotliBitReader, alphabetSize int) (*brotliPrefixTree, bool) {
	hskip, ok := br.readBits(2)
	if !ok {
		return nil, false
	}
	if hskip == 1 {
		nsymRaw, ok := br.readBits(2)
		if !ok {
			return nil, false
		}
		nsym := int(nsymRaw) + 1
		width := brotliAlphabetBitWidth(alphabetSize)
		var syms [4]uint16
		for i := 0; i < nsym; i++ {
			s, ok := br.readBits(width)
			if !ok || int(s) >= alphabetSize {
				return nil, false
			}
			for j := 0; j < i; j++ {
				if syms[j] == uint16(s) {
					return nil, false
				}
			}
			syms[i] = uint16(s)
		}
		lengths := make([]uint8, alphabetSize)
		switch nsym {
		case 1:
			return newSingleBrotliTree(syms[0]), true
		case 2:
			lengths[syms[0]] = 1
			lengths[syms[1]] = 1
		case 3:
			lengths[syms[0]] = 1
			lengths[syms[1]] = 2
			lengths[syms[2]] = 2
		case 4:
			ts, ok := br.readBits(1)
			if !ok {
				return nil, false
			}
			if ts == 0 {
				for i := 0; i < 4; i++ {
					lengths[syms[i]] = 2
				}
			} else {
				lengths[syms[0]] = 1
				lengths[syms[1]] = 2
				lengths[syms[2]] = 3
				lengths[syms[3]] = 3
			}
		}
		return newBrotliPrefixTree(lengths)
	}

	var clLengths [18]uint8
	space := int32(32)
	numCodes := 0
	for _, idx := range brotliCodeLengthOrder[hskip:] {
		if space <= 0 {
			break
		}
		v, ok := decodeBrotliClCodeLength(br)
		if !ok {
			return nil, false
		}
		clLengths[idx] = v
		if v != 0 {
			space -= int32(32) >> v
			numCodes++
		}
	}
	if numCodes != 1 && space != 0 {
		return nil, false
	}
	clTree, ok := newBrotliPrefixTree(clLengths[:])
	if !ok {
		return nil, false
	}

	symLengths := make([]uint8, alphabetSize)
	symbol := 0
	prevCodeLen := uint8(8)
	repeat := uint32(0)
	repeatCodeLen := uint8(0)
	symSpace := int32(32768)

	for symbol < alphabetSize && symSpace > 0 {
		codeLenRaw, ok := clTree.decode(br)
		if !ok {
			return nil, false
		}
		codeLen := uint8(codeLenRaw)
		if codeLen < 16 {
			repeat = 0
			symLengths[symbol] = codeLen
			symbol++
			if codeLen != 0 {
				prevCodeLen = codeLen
				symSpace -= int32(32768) >> codeLen
			}
		} else {
			extraBits := uint8(3)
			newLen := uint8(0)
			if codeLen == 16 {
				extraBits = 2
				newLen = prevCodeLen
			}
			if repeatCodeLen != newLen {
				repeat = 0
				repeatCodeLen = newLen
			}
			oldRepeat := repeat
			if repeat > 0 {
				repeat = (repeat - 2) << extraBits
			}
			eb, ok := br.readBits(extraBits)
			if !ok {
				return nil, false
			}
			repeat += eb + 3
			repeatDelta := int(repeat - oldRepeat)
			if symbol+repeatDelta > alphabetSize {
				return nil, false
			}
			for i := 0; i < repeatDelta; i++ {
				symLengths[symbol+i] = repeatCodeLen
			}
			symbol += repeatDelta
			if repeatCodeLen != 0 {
				symSpace -= int32(repeatDelta) << (15 - repeatCodeLen)
			}
		}
	}
	if symSpace != 0 {
		return nil, false
	}
	return newBrotliPrefixTree(symLengths)
}

func decodeBrotliVarlenUint8(br *brotliBitReader) (int, bool) {
	b, ok := br.readBits(1)
	if !ok {
		return 0, false
	}
	if b == 0 {
		return 1, true
	}
	nRaw, ok := br.readBits(3)
	if !ok {
		return 0, false
	}
	n := uint8(nRaw)
	if n == 0 {
		return 2, true
	}
	extra, ok := br.readBits(n)
	if !ok {
		return 0, false
	}
	return (1 << n) + int(extra) + 1, true
}

func decodeBrotliBlockLen(br *brotliBitReader, tree *brotliPrefixTree) (uint32, bool) {
	code, ok := tree.decode(br)
	if !ok || code >= 26 {
		return 0, false
	}
	extra, ok := br.readBits(brotliBlockLenNBits[code])
	if !ok {
		return 0, false
	}
	return brotliBlockLenOffset[code] + extra, true
}

func decodeBrotliContextMap(br *brotliBitReader, numContexts int) (int, []uint8, bool) {
	ntrees, ok := decodeBrotliVarlenUint8(br)
	if !ok {
		return 0, nil, false
	}
	if ntrees == 1 {
		return 1, make([]uint8, numContexts), true
	}
	useRleBit, ok := br.readBits(1)
	if !ok {
		return 0, nil, false
	}
	rleMax := 0
	if useRleBit != 0 {
		rm, ok := br.readBits(4)
		if !ok {
			return 0, nil, false
		}
		rleMax = int(rm) + 1
	}
	tree, ok := decodeBrotliPrefixTree(br, ntrees+rleMax)
	if !ok {
		return 0, nil, false
	}
	cmap := make([]uint8, 0, numContexts)
	for len(cmap) < numContexts {
		codeRaw, ok := tree.decode(br)
		if !ok {
			return 0, nil, false
		}
		code := int(codeRaw)
		if code == 0 {
			cmap = append(cmap, 0)
		} else if code <= rleMax {
			extra, ok := br.readBits(uint8(code))
			if !ok {
				return 0, nil, false
			}
			reps := (1 << code) + int(extra)
			if len(cmap)+reps > numContexts {
				return 0, nil, false
			}
			for i := 0; i < reps; i++ {
				cmap = append(cmap, 0)
			}
		} else {
			cmap = append(cmap, uint8(code-rleMax))
		}
	}
	imtfBit, ok := br.readBits(1)
	if !ok {
		return 0, nil, false
	}
	if imtfBit != 0 {
		var mtf [256]uint8
		for i := 0; i < 256; i++ {
			mtf[i] = uint8(i)
		}
		for i, v := range cmap {
			idx := int(v)
			val := mtf[idx]
			cmap[i] = val
			for j := idx; j >= 1; j-- {
				mtf[j] = mtf[j-1]
			}
			mtf[0] = val
		}
	}
	return ntrees, cmap, true
}

func brotliLiteralContextID(mode, p1, p2 uint8) int {
	switch mode & 3 {
	case 0:
		return int(p1 & 0x3F)
	case 1:
		return int(p1 >> 2)
	case 2:
		return int(brotliUTF8LUT0[p1] | brotliUTF8LUT1[p2])
	default:
		return int((brotliSignedLUT[p1] << 3) | brotliSignedLUT[p2])
	}
}

type brotliBlockSplit struct {
	numTypes int
	typeTree *brotliPrefixTree
	lenTree  *brotliPrefixTree
	currType int
	prevType int
	blockLen uint32
}

func decodeBrotliBlockSplit(br *brotliBitReader) (*brotliBlockSplit, bool) {
	numTypes, ok := decodeBrotliVarlenUint8(br)
	if !ok {
		return nil, false
	}
	if numTypes == 1 {
		return &brotliBlockSplit{
			numTypes: 1,
			currType: 0,
			prevType: 1,
			blockLen: 0xFFFFFFFF,
		}, true
	}
	typeTree, ok := decodeBrotliPrefixTree(br, numTypes+2)
	if !ok {
		return nil, false
	}
	lenTree, ok := decodeBrotliPrefixTree(br, 26)
	if !ok {
		return nil, false
	}
	blockLen, ok := decodeBrotliBlockLen(br, lenTree)
	if !ok {
		return nil, false
	}
	return &brotliBlockSplit{
		numTypes: numTypes,
		typeTree: typeTree,
		lenTree:  lenTree,
		currType: 0,
		prevType: 1,
		blockLen: blockLen,
	}, true
}

func (bs *brotliBlockSplit) nextCommand(br *brotliBitReader) (int, bool) {
	if bs.numTypes > 1 {
		if bs.blockLen == 0 {
			codeRaw, ok := bs.typeTree.decode(br)
			if !ok {
				return 0, false
			}
			code := int(codeRaw)
			var nextType int
			switch code {
			case 0:
				nextType = bs.prevType
			case 1:
				nextType = (bs.currType + 1) % bs.numTypes
			default:
				nextType = code - 2
			}
			bs.prevType = bs.currType
			bs.currType = nextType
			bl, ok := decodeBrotliBlockLen(br, bs.lenTree)
			if !ok {
				return 0, false
			}
			bs.blockLen = bl
		}
		bs.blockLen--
	}
	return bs.currType, true
}

func decodeBrotliInsertAndCopy(br *brotliBitReader, cmdCode uint16) (int, int, bool, bool) {
	if cmdCode >= 704 {
		return 0, 0, false, false
	}
	rangeIdx := cmdCode >> 6
	insertCode := int(brotliInsertRangeLUT[rangeIdx]) + int((cmdCode>>3)&7)
	copyCode := int(brotliCopyRangeLUT[rangeIdx]) + int(cmdCode&7)
	insExtra, ok := br.readBits(brotliInsertLenNBits[insertCode])
	if !ok {
		return 0, 0, false, false
	}
	cpExtra, ok := br.readBits(brotliCopyLenNBits[copyCode])
	if !ok {
		return 0, 0, false, false
	}
	insertLen := int(brotliInsertLenOffset[insertCode] + insExtra)
	copyLen := int(brotliCopyLenOffset[copyCode] + cpExtra)
	return insertLen, copyLen, rangeIdx < 2, true
}

func decompressBrotliWoff2(input []byte, maxOutput int) ([]byte, bool) {
	br := &brotliBitReader{data: input}
	b0, ok := br.readBits(1)
	if !ok {
		return nil, false
	}
	wbits := uint8(16)
	if b0 != 0 {
		n, ok := br.readBits(3)
		if !ok {
			return nil, false
		}
		if n != 0 {
			wbits = 17 + uint8(n)
		} else {
			m, ok := br.readBits(3)
			if !ok || m == 1 {
				return nil, false
			}
			if m != 0 {
				wbits = 8 + uint8(m)
			} else {
				wbits = 17
			}
		}
	}
	maxWindowDist := (1 << wbits) - 16
	out := make([]byte, 0, 1024)
	distRing := [4]int{4, 11, 15, 16}

	for {
		isLastBit, ok := br.readBits(1)
		if !ok {
			return nil, false
		}
		isLast := isLastBit != 0
		if isLast {
			isEmpty, ok := br.readBits(1)
			if !ok {
				return nil, false
			}
			if isEmpty != 0 {
				break
			}
		}
		mnibblesCode, ok := br.readBits(2)
		if !ok {
			return nil, false
		}
		if mnibblesCode == 3 {
			resBit, ok := br.readBits(1)
			if !ok || resBit != 0 {
				return nil, false
			}
			mbytesCode, ok := br.readBits(2)
			if !ok {
				return nil, false
			}
			if mbytesCode == 0 {
				if !br.alignToByte() {
					return nil, false
				}
			} else {
				mlenMeta := 0
				for i := uint32(0); i < mbytesCode; i++ {
					b, ok := br.readBits(8)
					if !ok {
						return nil, false
					}
					if i+1 == mbytesCode && mbytesCode > 1 && b == 0 {
						return nil, false
					}
					mlenMeta |= int(b) << (i * 8)
				}
				mlenMeta++
				if _, ok := br.readAlignedBytes(mlenMeta); !ok {
					return nil, false
				}
			}
			if isLast {
				break
			}
			continue
		}

		mnibbles := uint8(mnibblesCode) + 4
		var mlenRaw uint32
		for i := uint8(0); i < mnibbles; i++ {
			nib, ok := br.readBits(4)
			if !ok {
				return nil, false
			}
			if i+1 == mnibbles && mnibbles > 4 && nib == 0 {
				return nil, false
			}
			mlenRaw |= nib << (i * 4)
		}
		mlen := int(mlenRaw) + 1
		if len(out)+mlen > maxOutput {
			return nil, false
		}

		if !isLast {
			isUncomp, ok := br.readBits(1)
			if !ok {
				return nil, false
			}
			if isUncomp != 0 {
				raw, ok := br.readAlignedBytes(mlen)
				if !ok {
					return nil, false
				}
				out = append(out, raw...)
				continue
			}
		}

		litSplit, ok := decodeBrotliBlockSplit(br)
		if !ok {
			return nil, false
		}
		cmdSplit, ok := decodeBrotliBlockSplit(br)
		if !ok {
			return nil, false
		}
		distSplit, ok := decodeBrotliBlockSplit(br)
		if !ok {
			return nil, false
		}

		npostfixRaw, ok := br.readBits(2)
		if !ok {
			return nil, false
		}
		npostfix := uint8(npostfixRaw)
		ndirectRaw, ok := br.readBits(4)
		if !ok {
			return nil, false
		}
		ndirect := int(ndirectRaw) << npostfix
		postfixMask := (1 << npostfix) - 1

		contextModes := make([]uint8, litSplit.numTypes)
		for i := 0; i < litSplit.numTypes; i++ {
			cm, ok := br.readBits(2)
			if !ok {
				return nil, false
			}
			contextModes[i] = uint8(cm)
		}

		ntreesL, contextMapL, ok := decodeBrotliContextMap(br, litSplit.numTypes*64)
		if !ok {
			return nil, false
		}
		ntreesD, contextMapD, ok := decodeBrotliContextMap(br, distSplit.numTypes*4)
		if !ok {
			return nil, false
		}

		htreesL := make([]*brotliPrefixTree, ntreesL)
		for i := 0; i < ntreesL; i++ {
			t, ok := decodeBrotliPrefixTree(br, 256)
			if !ok {
				return nil, false
			}
			htreesL[i] = t
		}
		htreesI := make([]*brotliPrefixTree, cmdSplit.numTypes)
		for i := 0; i < cmdSplit.numTypes; i++ {
			t, ok := decodeBrotliPrefixTree(br, 704)
			if !ok {
				return nil, false
			}
			htreesI[i] = t
		}
		distAlphabetSize := 16 + ndirect + (48 << npostfix)
		htreesD := make([]*brotliPrefixTree, ntreesD)
		for i := 0; i < ntreesD; i++ {
			t, ok := decodeBrotliPrefixTree(br, distAlphabetSize)
			if !ok {
				return nil, false
			}
			htreesD[i] = t
		}

		metaEnd := len(out) + mlen
		for len(out) < metaEnd {
			btypeI, ok := cmdSplit.nextCommand(br)
			if !ok {
				return nil, false
			}
			cmdSym, ok := htreesI[btypeI].decode(br)
			if !ok {
				return nil, false
			}
			insertLen, copyLen, distCodeZero, ok := decodeBrotliInsertAndCopy(br, cmdSym)
			if !ok {
				return nil, false
			}

			for i := 0; i < insertLen && len(out) < metaEnd; i++ {
				btypeL, ok := litSplit.nextCommand(br)
				if !ok {
					return nil, false
				}
				var p1, p2 uint8
				if len(out) >= 1 {
					p1 = out[len(out)-1]
				}
				if len(out) >= 2 {
					p2 = out[len(out)-2]
				}
				ctx := brotliLiteralContextID(contextModes[btypeL], p1, p2)
				treeIdx := contextMapL[(btypeL<<6)|ctx]
				litSym, ok := htreesL[treeIdx].decode(br)
				if !ok {
					return nil, false
				}
				out = append(out, uint8(litSym))
			}

			if len(out) >= metaEnd {
				break
			}

			distCode := 0
			if !distCodeZero {
				btypeD, ok := distSplit.nextCommand(br)
				if !ok {
					return nil, false
				}
				copyCtx := 3
				if copyLen <= 2 {
					copyCtx = 0
				} else if copyLen == 3 {
					copyCtx = 1
				} else if copyLen == 4 {
					copyCtx = 2
				}
				treeIdx := contextMapD[(btypeD<<2)|copyCtx]
				dc, ok := htreesD[treeIdx].decode(br)
				if !ok {
					return nil, false
				}
				distCode = int(dc)
			}

			var distance int
			if distCode < 16 {
				base := distRing[brotliDistShortIndex[distCode]]
				d := base + brotliDistShortDelta[distCode]
				if d <= 0 {
					return nil, false
				}
				distance = d
			} else if distCode < 16+ndirect {
				distance = distCode - 15
			} else {
				dcode := distCode - 16 - ndirect
				postfix := dcode & postfixMask
				hcode := dcode >> npostfix
				ndistbits := uint8((hcode >> 1) + 1)
				offset := (((2 + (hcode & 1)) << ndistbits) - 4) << npostfix
				extra, ok := br.readBits(ndistbits)
				if !ok {
					return nil, false
				}
				distance = offset + (int(extra) << npostfix) + postfix + ndirect + 1
			}

			if distCode != 0 {
				distRing[3] = distRing[2]
				distRing[2] = distRing[1]
				distRing[1] = distRing[0]
				distRing[0] = distance
			}

			maxAllowed := len(out)
			if maxWindowDist < maxAllowed {
				maxAllowed = maxWindowDist
			}
			if distance <= 0 || distance > maxAllowed || len(out)+copyLen > maxOutput {
				return nil, false
			}

			for i := 0; i < copyLen && len(out) < metaEnd; i++ {
				out = append(out, out[len(out)-distance])
			}
		}

		if isLast {
			break
		}
	}

	return out, true
}

func readWoff2UIntBase128(data []byte, pos *int) (uint32, bool) {
	var accum uint32
	for i := 0; i < 5; i++ {
		if *pos >= len(data) {
			return 0, false
		}
		b := data[*pos]
		*pos++
		if i == 0 && b == 0x80 {
			return 0, false
		}
		if (accum & 0xFE000000) != 0 {
			return 0, false
		}
		accum = (accum << 7) | uint32(b&0x7F)
		if (b & 0x80) == 0 {
			return accum, true
		}
	}
	return 0, false
}

func readWoff2UShort255(data []byte, pos *int) (uint16, bool) {
	if *pos >= len(data) {
		return 0, false
	}
	code := data[*pos]
	*pos++
	switch code {
	case 253:
		if *pos+2 > len(data) {
			return 0, false
		}
		v := binary.BigEndian.Uint16(data[*pos : *pos+2])
		*pos += 2
		return v, true
	case 254:
		if *pos >= len(data) {
			return 0, false
		}
		v := data[*pos]
		*pos++
		return 253*2 + uint16(v), true
	case 255:
		if *pos >= len(data) {
			return 0, false
		}
		v := data[*pos]
		*pos++
		return 253 + uint16(v), true
	default:
		return uint16(code), true
	}
}

func decodeWoff2GlyfTriplet(flag uint8, glyphStream []byte, pos *int) (int32, int32, bool, bool) {
	onCurve := (flag & 0x80) == 0
	f := flag & 0x7F
	if f < 10 {
		if *pos >= len(glyphStream) {
			return 0, 0, false, false
		}
		b := int32(glyphStream[*pos])
		*pos++
		dy := ((int32(f) & 14) << 7) + b
		if (f & 1) == 0 {
			dy = -dy
		}
		return 0, dy, onCurve, true
	} else if f < 20 {
		if *pos >= len(glyphStream) {
			return 0, 0, false, false
		}
		b := int32(glyphStream[*pos])
		*pos++
		idx := int32(f) - 10
		dx := ((idx & 14) << 7) + b
		if (idx & 1) == 0 {
			dx = -dx
		}
		return dx, 0, onCurve, true
	} else if f < 84 {
		if *pos >= len(glyphStream) {
			return 0, 0, false, false
		}
		b := int32(glyphStream[*pos])
		*pos++
		idx := int32(f) - 20
		b0 := (idx & 0x30) + 1 + (b >> 4)
		b1 := ((idx & 0x0C) << 2) + 1 + (b & 0x0F)
		dx := b0
		if (idx & 2) == 0 {
			dx = -dx
		}
		dy := b1
		if (idx & 1) == 0 {
			dy = -dy
		}
		return dx, dy, onCurve, true
	} else if f < 120 {
		if *pos+2 > len(glyphStream) {
			return 0, 0, false, false
		}
		b0 := int32(glyphStream[*pos])
		b1 := int32(glyphStream[*pos+1])
		*pos += 2
		idx := int32(f) - 84
		magX := ((idx / 12) << 8) + b0 + 1
		magY := (((idx % 12) >> 2) << 8) + b1 + 1
		dx := magX
		if (idx & 2) == 0 {
			dx = -dx
		}
		dy := magY
		if (idx & 1) == 0 {
			dy = -dy
		}
		return dx, dy, onCurve, true
	} else if f < 124 {
		if *pos+3 > len(glyphStream) {
			return 0, 0, false, false
		}
		b0 := int32(glyphStream[*pos])
		b1 := int32(glyphStream[*pos+1])
		b2 := int32(glyphStream[*pos+2])
		*pos += 3
		idx := int32(f) - 120
		magX := (b0 << 4) + (b1 >> 4) + 1
		magY := ((b1 & 0x0F) << 8) + b2 + 1
		dx := magX
		if (idx & 2) == 0 {
			dx = -dx
		}
		dy := magY
		if (idx & 1) == 0 {
			dy = -dy
		}
		return dx, dy, onCurve, true
	} else {
		if *pos+4 > len(glyphStream) {
			return 0, 0, false, false
		}
		b0 := int32(glyphStream[*pos])
		b1 := int32(glyphStream[*pos+1])
		b2 := int32(glyphStream[*pos+2])
		b3 := int32(glyphStream[*pos+3])
		*pos += 4
		idx := int32(f) - 124
		magX := (b0 << 8) + b1 + 1
		magY := (b2 << 8) + b3 + 1
		dx := magX
		if (idx & 2) == 0 {
			dx = -dx
		}
		dy := magY
		if (idx & 1) == 0 {
			dy = -dy
		}
		return dx, dy, onCurve, true
	}
}

func reconstructWoff2GlyfLoca(data []byte) ([]byte, []byte, []int16, bool) {
	if len(data) < 36 {
		return nil, nil, nil, false
	}
	optionFlags := binary.BigEndian.Uint16(data[2:4])
	numGlyphs := int(binary.BigEndian.Uint16(data[4:6]))
	indexFormat := binary.BigEndian.Uint16(data[6:8])
	nContourSize := int(binary.BigEndian.Uint32(data[8:12]))
	nPointsSize := int(binary.BigEndian.Uint32(data[12:16]))
	flagSize := int(binary.BigEndian.Uint32(data[16:20]))
	glyphSize := int(binary.BigEndian.Uint32(data[20:24]))
	compositeSize := int(binary.BigEndian.Uint32(data[24:28]))
	bboxSize := int(binary.BigEndian.Uint32(data[28:32]))
	instrSize := int(binary.BigEndian.Uint32(data[32:36]))

	off := 36
	if off+nContourSize+nPointsSize+flagSize+glyphSize+compositeSize+bboxSize+instrSize > len(data) {
		return nil, nil, nil, false
	}
	nContourStream := data[off : off+nContourSize]
	off += nContourSize
	nPointsStream := data[off : off+nPointsSize]
	off += nPointsSize
	flagStream := data[off : off+flagSize]
	off += flagSize
	glyphStream := data[off : off+glyphSize]
	off += glyphSize
	compositeStream := data[off : off+compositeSize]
	off += compositeSize
	bboxStream := data[off : off+bboxSize]
	off += bboxSize
	instrStream := data[off : off+instrSize]
	off += instrSize

	var overlapBitmap []byte
	if (optionFlags & 1) != 0 {
		overlapLen := (numGlyphs + 7) >> 3
		if off+overlapLen > len(data) {
			return nil, nil, nil, false
		}
		overlapBitmap = data[off : off+overlapLen]
	}

	bboxBitmapLen := ((numGlyphs + 31) >> 5) << 2
	if len(bboxStream) < bboxBitmapLen {
		return nil, nil, nil, false
	}
	bboxBitmap := bboxStream[:bboxBitmapLen]
	bboxPos := bboxBitmapLen

	var nContourPos, nPointsPos, flagPos, glyphPos, compPos, instrPos int
	var glyfOut []byte
	locaOffsets := make([]int, 0, numGlyphs+1)
	xMins := make([]int16, 0, numGlyphs)

	for gid := 0; gid < numGlyphs; gid++ {
		locaOffsets = append(locaOffsets, len(glyfOut))
		if nContourPos+2 > len(nContourStream) {
			return nil, nil, nil, false
		}
		nContours := int16(binary.BigEndian.Uint16(nContourStream[nContourPos : nContourPos+2]))
		nContourPos += 2
		hasExplicitBbox := (bboxBitmap[gid>>3] & (0x80 >> (gid & 7))) != 0

		if nContours == 0 {
			if hasExplicitBbox {
				return nil, nil, nil, false
			}
			xMins = append(xMins, 0)
			continue
		}

		if nContours < 0 {
			if !hasExplicitBbox || bboxPos+8 > len(bboxStream) {
				return nil, nil, nil, false
			}
			xMin := int16(binary.BigEndian.Uint16(bboxStream[bboxPos : bboxPos+2]))
			bboxBytes := bboxStream[bboxPos : bboxPos+8]
			bboxPos += 8
			xMins = append(xMins, xMin)

			compStart := compPos
			haveInstructions := false
			for {
				if compPos+4 > len(compositeStream) {
					return nil, nil, nil, false
				}
				flags := binary.BigEndian.Uint16(compositeStream[compPos : compPos+2])
				compPos += 4
				if (flags & 0x0001) != 0 {
					compPos += 4
				} else {
					compPos += 2
				}
				if (flags & 0x0008) != 0 {
					compPos += 2
				} else if (flags & 0x0040) != 0 {
					compPos += 4
				} else if (flags & 0x0080) != 0 {
					compPos += 8
				}
				if compPos > len(compositeStream) {
					return nil, nil, nil, false
				}
				if (flags & 0x0100) != 0 {
					haveInstructions = true
				}
				if (flags & 0x0020) == 0 {
					break
				}
			}
			compSlice := compositeStream[compStart:compPos]
			var instrLen int
			if haveInstructions {
				il, ok := readWoff2UShort255(glyphStream, &glyphPos)
				if !ok {
					return nil, nil, nil, false
				}
				instrLen = int(il)
			}
			if instrPos+instrLen > len(instrStream) {
				return nil, nil, nil, false
			}
			instrSlice := instrStream[instrPos : instrPos+instrLen]
			instrPos += instrLen

			var hdr [2]byte
			binary.BigEndian.PutUint16(hdr[:], 0xFFFF)
			glyfOut = append(glyfOut, hdr[:]...)
			glyfOut = append(glyfOut, bboxBytes...)
			glyfOut = append(glyfOut, compSlice...)
			if haveInstructions {
				binary.BigEndian.PutUint16(hdr[:], uint16(instrLen))
				glyfOut = append(glyfOut, hdr[:]...)
				glyfOut = append(glyfOut, instrSlice...)
			}
			for (len(glyfOut) & 3) != 0 {
				glyfOut = append(glyfOut, 0)
			}
			continue
		}

		nc := int(nContours)
		endPts := make([]uint16, nc)
		totalPts := 0
		for i := 0; i < nc; i++ {
			pts, ok := readWoff2UShort255(nPointsStream, &nPointsPos)
			if !ok {
				return nil, nil, nil, false
			}
			totalPts += int(pts)
			if totalPts == 0 {
				return nil, nil, nil, false
			}
			endPts[i] = uint16(totalPts - 1)
		}

		xs := make([]int16, totalPts)
		ys := make([]int16, totalPts)
		onCurves := make([]bool, totalPts)
		var curX, curY int32
		minX, minY := int16(32767), int16(32767)
		maxX, maxY := int16(-32768), int16(-32768)

		for i := 0; i < totalPts; i++ {
			if flagPos >= len(flagStream) {
				return nil, nil, nil, false
			}
			f := flagStream[flagPos]
			flagPos++
			dx, dy, onCurve, ok := decodeWoff2GlyfTriplet(f, glyphStream, &glyphPos)
			if !ok {
				return nil, nil, nil, false
			}
			curX += dx
			curY += dy
			px, py := int16(curX), int16(curY)
			if px < minX {
				minX = px
			}
			if py < minY {
				minY = py
			}
			if px > maxX {
				maxX = px
			}
			if py > maxY {
				maxY = py
			}
			xs[i] = int16(dx)
			ys[i] = int16(dy)
			onCurves[i] = onCurve
		}

		xMin, yMin, xMax, yMax := minX, minY, maxX, maxY
		if hasExplicitBbox {
			if bboxPos+8 > len(bboxStream) {
				return nil, nil, nil, false
			}
			xMin = int16(binary.BigEndian.Uint16(bboxStream[bboxPos : bboxPos+2]))
			yMin = int16(binary.BigEndian.Uint16(bboxStream[bboxPos+2 : bboxPos+4]))
			xMax = int16(binary.BigEndian.Uint16(bboxStream[bboxPos+4 : bboxPos+6]))
			yMax = int16(binary.BigEndian.Uint16(bboxStream[bboxPos+6 : bboxPos+8]))
			bboxPos += 8
		}
		xMins = append(xMins, xMin)

		instrLenRaw, ok := readWoff2UShort255(glyphStream, &glyphPos)
		if !ok {
			return nil, nil, nil, false
		}
		instrLen := int(instrLenRaw)
		if instrPos+instrLen > len(instrStream) {
			return nil, nil, nil, false
		}
		instrSlice := instrStream[instrPos : instrPos+instrLen]
		instrPos += instrLen

		var hdr [10]byte
		binary.BigEndian.PutUint16(hdr[0:2], uint16(nContours))
		binary.BigEndian.PutUint16(hdr[2:4], uint16(xMin))
		binary.BigEndian.PutUint16(hdr[4:6], uint16(yMin))
		binary.BigEndian.PutUint16(hdr[6:8], uint16(xMax))
		binary.BigEndian.PutUint16(hdr[8:10], uint16(yMax))
		glyfOut = append(glyfOut, hdr[:]...)
		var b2 [2]byte
		for _, ep := range endPts {
			binary.BigEndian.PutUint16(b2[:], ep)
			glyfOut = append(glyfOut, b2[:]...)
		}
		binary.BigEndian.PutUint16(b2[:], uint16(instrLen))
		glyfOut = append(glyfOut, b2[:]...)
		glyfOut = append(glyfOut, instrSlice...)

		overlapSimple := len(overlapBitmap) > 0 && (overlapBitmap[gid>>3]&(0x80>>(gid&7))) != 0
		for i := 0; i < totalPts; i++ {
			var tf uint8
			if onCurves[i] {
				tf = 0x01
			}
			if i == 0 && overlapSimple {
				tf |= 0x40
			}
			glyfOut = append(glyfOut, tf)
		}
		for _, dx := range xs {
			binary.BigEndian.PutUint16(b2[:], uint16(dx))
			glyfOut = append(glyfOut, b2[:]...)
		}
		for _, dy := range ys {
			binary.BigEndian.PutUint16(b2[:], uint16(dy))
			glyfOut = append(glyfOut, b2[:]...)
		}
		for (len(glyfOut) & 3) != 0 {
			glyfOut = append(glyfOut, 0)
		}
	}
	locaOffsets = append(locaOffsets, len(glyfOut))

	var locaOut []byte
	if indexFormat == 0 {
		locaOut = make([]byte, len(locaOffsets)*2)
		for i, offVal := range locaOffsets {
			binary.BigEndian.PutUint16(locaOut[i*2:i*2+2], uint16(offVal/2))
		}
	} else {
		locaOut = make([]byte, len(locaOffsets)*4)
		for i, offVal := range locaOffsets {
			binary.BigEndian.PutUint32(locaOut[i*4:i*4+4], uint32(offVal))
		}
	}

	return glyfOut, locaOut, xMins, true
}

func reconstructWoff2Hmtx(data []byte, numGlyphs, numHMetrics int, xMins []int16) ([]byte, bool) {
	if len(data) == 0 || numHMetrics <= 0 || numHMetrics > numGlyphs {
		return nil, false
	}
	flags := data[0]
	if (flags & 0xFC) != 0 {
		return nil, false
	}
	hasPropLsb := (flags & 1) == 0
	hasMonoLsb := (flags & 2) == 0
	pos := 1

	advances := make([]uint16, numHMetrics)
	for i := 0; i < numHMetrics; i++ {
		if pos+2 > len(data) {
			return nil, false
		}
		advances[i] = binary.BigEndian.Uint16(data[pos : pos+2])
		pos += 2
	}

	lsbs := make([]int16, numGlyphs)
	for i := 0; i < numHMetrics; i++ {
		if hasPropLsb {
			if pos+2 > len(data) {
				return nil, false
			}
			lsbs[i] = int16(binary.BigEndian.Uint16(data[pos : pos+2]))
			pos += 2
		} else if i < len(xMins) {
			lsbs[i] = xMins[i]
		}
	}
	for i := numHMetrics; i < numGlyphs; i++ {
		if hasMonoLsb {
			if pos+2 > len(data) {
				return nil, false
			}
			lsbs[i] = int16(binary.BigEndian.Uint16(data[pos : pos+2]))
			pos += 2
		} else if i < len(xMins) {
			lsbs[i] = xMins[i]
		}
	}

	out := make([]byte, numHMetrics*4+(numGlyphs-numHMetrics)*2)
	for i := 0; i < numHMetrics; i++ {
		binary.BigEndian.PutUint16(out[i*4:i*4+2], advances[i])
		binary.BigEndian.PutUint16(out[i*4+2:i*4+4], uint16(lsbs[i]))
	}
	for i := numHMetrics; i < numGlyphs; i++ {
		off := numHMetrics*4 + (i-numHMetrics)*2
		binary.BigEndian.PutUint16(out[off:off+2], uint16(lsbs[i]))
	}
	return out, true
}

// unpackWoff2 converts a W3C WOFF 2.0 ('wOF2', 0x774F4632) font container into a standard SFNT byte slice.
func unpackWoff2(data []byte) ([]byte, error) {
	if len(data) < 48 {
		return nil, errUnexpectedEOF()
	}
	flavor, err := readU32BE(data, 4)
	if err != nil {
		return nil, err
	}
	if flavor != 0x00010000 && flavor != 0x74727565 && flavor != 0x4F54544F {
		return nil, errInvalidSignature()
	}
	numTablesRaw, err := readU16BE(data, 12)
	if err != nil {
		return nil, err
	}
	numTables := int(numTablesRaw)
	if numTables == 0 || numTables > MaxSfntTables {
		return nil, errTooManyTables(numTables)
	}
	totalCompSizeRaw, err := readU32BE(data, 20)
	if err != nil {
		return nil, err
	}
	totalCompSize := int(totalCompSizeRaw)

	type woff2Entry struct {
		tag          [4]byte
		flags        uint8
		transformLen int
		payload      []byte
	}
	pos := 48
	entries := make([]woff2Entry, numTables)
	totalUncompressed := 0

	for i := 0; i < numTables; i++ {
		if pos >= len(data) {
			return nil, errUnexpectedEOF()
		}
		flags := data[pos]
		pos++
		tagIdx := int(flags & 0x3F)
		var tag [4]byte
		if tagIdx < 63 {
			tag = woff2KnownTableTags[tagIdx]
		} else {
			if pos+4 > len(data) {
				return nil, errUnexpectedEOF()
			}
			copy(tag[:], data[pos:pos+4])
			pos += 4
		}
		xformVer := (flags >> 6) & 0x03
		origLen, ok := readWoff2UIntBase128(data, &pos)
		if !ok {
			return nil, errUnexpectedEOF()
		}
		tagStr := string(tag[:])
		isGlyfOrLoca := tagStr == "glyf" || tagStr == "loca"
		var isXformed bool
		if isGlyfOrLoca {
			isXformed = xformVer == 0
		} else if tagStr == "hmtx" {
			isXformed = xformVer == 1
		} else {
			isXformed = xformVer != 0
		}
		transformLen := int(origLen)
		if isXformed {
			tl, ok := readWoff2UIntBase128(data, &pos)
			if !ok {
				return nil, errUnexpectedEOF()
			}
			transformLen = int(tl)
		}
		if tagStr == "loca" && isXformed && transformLen != 0 {
			return nil, errInvalidTable("loca")
		}
		if transformLen < 0 || totalUncompressed+transformLen > MaxFontBytes {
			return nil, errFontTooLarge(totalUncompressed + transformLen)
		}
		totalUncompressed += transformLen
		entries[i] = woff2Entry{
			tag:          tag,
			flags:        flags,
			transformLen: transformLen,
		}
	}

	if totalCompSize < 0 || pos+totalCompSize > len(data) {
		return nil, errUnexpectedEOF()
	}
	maxCap := totalUncompressed
	if maxCap < 64 {
		maxCap = 64
	}
	uncompressed, ok := decompressBrotliWoff2(data[pos:pos+totalCompSize], maxCap)
	if !ok || len(uncompressed) < totalUncompressed {
		return nil, errInvalidSignature()
	}

	uPos := 0
	for i := range entries {
		tl := entries[i].transformLen
		entries[i].payload = uncompressed[uPos : uPos+tl]
		uPos += tl
	}

	var xMins []int16
	glyfIdx := -1
	locaIdx := -1
	for i := range entries {
		switch string(entries[i].tag[:]) {
		case "glyf":
			glyfIdx = i
		case "loca":
			locaIdx = i
		}
	}
	if glyfIdx >= 0 && ((entries[glyfIdx].flags>>6)&0x03) == 0 {
		glyfBytes, locaBytes, decodedXMins, ok := reconstructWoff2GlyfLoca(entries[glyfIdx].payload)
		if !ok {
			return nil, errInvalidTable("glyf")
		}
		entries[glyfIdx].payload = glyfBytes
		if locaIdx >= 0 {
			entries[locaIdx].payload = locaBytes
		}
		xMins = decodedXMins
	}

	for i := range entries {
		if string(entries[i].tag[:]) == "hmtx" && ((entries[i].flags>>6)&0x03) == 1 {
			numGlyphs := len(xMins)
			numHMetrics := 1
			for j := range entries {
				t := string(entries[j].tag[:])
				if t == "maxp" && len(entries[j].payload) >= 6 {
					numGlyphs = int(binary.BigEndian.Uint16(entries[j].payload[4:6]))
				} else if t == "hhea" && len(entries[j].payload) >= 36 {
					numHMetrics = int(binary.BigEndian.Uint16(entries[j].payload[34:36]))
				}
			}
			hmtxBytes, ok := reconstructWoff2Hmtx(entries[i].payload, numGlyphs, numHMetrics, xMins)
			if !ok {
				return nil, errInvalidTable("hmtx")
			}
			entries[i].payload = hmtxBytes
		}
	}

	sfntSize := 12 + numTables*16
	for _, e := range entries {
		sfntSize += (len(e.payload) + 3) & ^3
		if sfntSize > MaxFontBytes {
			return nil, errFontTooLarge(sfntSize)
		}
	}

	out := make([]byte, sfntSize)
	binary.BigEndian.PutUint32(out[0:4], flavor)
	binary.BigEndian.PutUint16(out[4:6], uint16(numTables))
	curOff := 12 + numTables*16
	for i, e := range entries {
		dirPos := 12 + i*16
		copy(out[dirPos:dirPos+4], e.tag[:])
		binary.BigEndian.PutUint32(out[dirPos+8:dirPos+12], uint32(curOff))
		binary.BigEndian.PutUint32(out[dirPos+12:dirPos+16], uint32(len(e.payload)))
		copy(out[curOff:curOff+len(e.payload)], e.payload)
		curOff += (len(e.payload) + 3) & ^3
	}
	return out, nil
}
