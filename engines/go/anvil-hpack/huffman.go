// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

import "sync"

const (
	huffmanEOS  uint16 = 256
	huffmanLeaf uint32 = 1 << 31
)

// HuffmanEncodedLen returns the number of bytes required to Huffman-encode input.
func HuffmanEncodedLen(input []byte) int {
	bits := 0
	for _, b := range input {
		bits += int(huffmanCodes[b].bits)
	}
	return (bits + 7) / 8
}

// HuffmanEncode appends the RFC 7541 canonical Huffman encoding of input to out,
// padding the final partial byte with all-ones (high bits of EOS).
func HuffmanEncode(input []byte, out []byte) []byte {
	var acc uint64
	var bits uint32
	for _, b := range input {
		entry := huffmanCodes[b]
		acc = (acc << entry.bits) | uint64(entry.code)
		bits += uint32(entry.bits)
		for bits >= 8 {
			bits -= 8
			out = append(out, byte(acc>>bits))
		}
		acc &= (uint64(1) << bits) - 1
	}
	if bits > 0 {
		pad := 8 - bits
		out = append(out, byte((acc<<pad)|((uint64(1)<<pad)-1)))
	}
	return out
}

var (
	decodeTreeOnce sync.Once
	decodeTreeData [][2]uint32
)

func getDecodeTree() [][2]uint32 {
	decodeTreeOnce.Do(func() {
		tree := [][2]uint32{{0, 0}}
		for sym, entry := range huffmanCodes {
			node := 0
			for i := int(entry.bits) - 1; i >= 0; i-- {
				bit := (entry.code >> uint(i)) & 1
				if i == 0 {
					tree[node][bit] = huffmanLeaf | uint32(sym)
				} else {
					next := tree[node][bit]
					if next == 0 {
						tree = append(tree, [2]uint32{0, 0})
						idx := len(tree) - 1
						tree[node][bit] = uint32(idx)
						node = idx
					} else {
						node = int(next)
					}
				}
			}
		}
		decodeTreeData = tree
	})
	return decodeTreeData
}

// HuffmanDecode decodes an RFC 7541 Huffman-encoded byte slice up to maxLen output bytes.
// Padding must be <= 7 all-ones bits, and the EOS symbol (256) must not appear.
func HuffmanDecode(input []byte, maxLen int) ([]byte, error) {
	tree := getDecodeTree()
	capHint := len(input) * 8 / 5
	if capHint > maxLen {
		capHint = maxLen
	}
	out := make([]byte, 0, capHint)
	node := 0
	var pendingBits uint32
	pendingAllOnes := true

	for _, b := range input {
		for shift := 7; shift >= 0; shift-- {
			bit := (b >> uint(shift)) & 1
			pendingBits++
			if bit != 1 {
				pendingAllOnes = false
			}
			next := tree[node][bit]
			if next == 0 {
				return nil, &HpackError{Kind: ErrInvalidHuffman}
			}
			if next&huffmanLeaf != 0 {
				symbol := uint16(next & ^huffmanLeaf)
				if symbol == huffmanEOS {
					return nil, &HpackError{Kind: ErrInvalidHuffman}
				}
				if len(out) >= maxLen {
					return nil, &HpackError{Kind: ErrStringTooLong, Value: len(out) + 1}
				}
				out = append(out, byte(symbol))
				node = 0
				pendingBits = 0
				pendingAllOnes = true
			} else {
				node = int(next)
			}
		}
	}
	if pendingBits > 7 || !pendingAllOnes {
		return nil, &HpackError{Kind: ErrInvalidHuffman}
	}
	return out, nil
}
