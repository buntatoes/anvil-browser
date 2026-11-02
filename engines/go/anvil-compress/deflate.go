// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilcompress

import "fmt"

// ErrorKind categorizes decompression and framing failures.
type ErrorKind uint8

const (
	ErrUnexpectedEof ErrorKind = iota
	ErrInvalidHeader
	ErrInvalidBlockType
	ErrStoredLengthMismatch
	ErrInvalidHuffmanTree
	ErrInvalidCode
	ErrInvalidDistance
	ErrChecksumMismatch
	ErrLengthMismatch
	ErrOutputLimitExceeded
)

// CompressError describes a framing, bitstream, checksum, or output-cap failure.
type CompressError struct {
	Kind     ErrorKind
	Detail   string
	Expected uint32
	Actual   uint32
	Limit    int
}

func (e *CompressError) Error() string {
	switch e.Kind {
	case ErrUnexpectedEof:
		return "unexpected end of compressed stream"
	case ErrInvalidHeader:
		return fmt.Sprintf("invalid header: %s", e.Detail)
	case ErrInvalidBlockType:
		return fmt.Sprintf("invalid DEFLATE block type %d", e.Actual)
	case ErrStoredLengthMismatch:
		return "stored DEFLATE block LEN/NLEN mismatch"
	case ErrInvalidHuffmanTree:
		return fmt.Sprintf("invalid Huffman code tree: %s", e.Detail)
	case ErrInvalidCode:
		return "invalid Huffman symbol in bitstream"
	case ErrInvalidDistance:
		return fmt.Sprintf("invalid LZ77 back-reference distance %d", e.Actual)
	case ErrChecksumMismatch:
		return fmt.Sprintf("checksum mismatch (expected 0x%08x, got 0x%08x)", e.Expected, e.Actual)
	case ErrLengthMismatch:
		return fmt.Sprintf("uncompressed length mismatch (expected %d, got %d)", e.Expected, e.Actual)
	case ErrOutputLimitExceeded:
		return fmt.Sprintf("decompressed output exceeds %d byte cap", e.Limit)
	default:
		return "compression error"
	}
}

const maxBits = 15

var lengthBases = [29]uint16{
	3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
	35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
}

var lengthExtra = [29]uint8{
	0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
	3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
}

var distBases = [30]uint16{
	1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
	257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
}

var distExtra = [30]uint8{
	0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
	7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
}

var clOrder = [19]int{
	16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
}

// DeflateDecompress decompresses a raw RFC 1951 DEFLATE stream using the default 32 MiB cap.
func DeflateDecompress(input []byte) ([]byte, error) {
	return DeflateDecompressBounded(input, DefaultMaxDecompressedBytes)
}

// DeflateDecompressBounded decompresses a raw RFC 1951 DEFLATE stream up to maxOutputBytes.
func DeflateDecompressBounded(input []byte, maxOutputBytes int) ([]byte, error) {
	out, _, err := deflateDecodeStream(input, 0, maxOutputBytes)
	return out, err
}

func deflateDecodeStream(input []byte, startByte int, maxOutputBytes int) ([]byte, int, error) {
	br := bitReader{data: input, bytePos: startByte}
	out := make([]byte, 0, minInt(len(input)*2, maxOutputBytes))

	for {
		bfinal, err := br.readBits(1)
		if err != nil {
			return nil, br.bytePos, err
		}
		btype, err := br.readBits(2)
		if err != nil {
			return nil, br.bytePos, err
		}

		switch btype {
		case 0:
			br.alignToByte()
			blockLen, err := br.readU16LE()
			if err != nil {
				return nil, br.bytePos, err
			}
			nlen, err := br.readU16LE()
			if err != nil {
				return nil, br.bytePos, err
			}
			if blockLen != ^nlen {
				return nil, br.bytePos, &CompressError{Kind: ErrStoredLengthMismatch}
			}
			if len(out)+int(blockLen) > maxOutputBytes {
				return nil, br.bytePos, &CompressError{Kind: ErrOutputLimitExceeded, Limit: maxOutputBytes}
			}
			slice, err := br.readBytes(int(blockLen))
			if err != nil {
				return nil, br.bytePos, err
			}
			out = append(out, slice...)
		case 1:
			litTree, distTree := fixedHuffmanTrees()
			var err error
			out, err = decodeHuffmanBlock(&br, &litTree, &distTree, out, maxOutputBytes)
			if err != nil {
				return nil, br.bytePos, err
			}
		case 2:
			litTree, distTree, err := readDynamicHuffmanTrees(&br)
			if err != nil {
				return nil, br.bytePos, err
			}
			out, err = decodeHuffmanBlock(&br, &litTree, &distTree, out, maxOutputBytes)
			if err != nil {
				return nil, br.bytePos, err
			}
		default:
			return nil, br.bytePos, &CompressError{Kind: ErrInvalidBlockType, Actual: btype}
		}

		if bfinal == 1 {
			break
		}
	}
	br.alignToByte()
	return out, br.bytePos, nil
}

type bitReader struct {
	data    []byte
	bytePos int
	bitBuf  uint64
	bitCnt  uint8
}

func (br *bitReader) ensureBits(n uint8) error {
	for br.bitCnt < n {
		if br.bytePos >= len(br.data) {
			return &CompressError{Kind: ErrUnexpectedEof}
		}
		br.bitBuf |= uint64(br.data[br.bytePos]) << br.bitCnt
		br.bytePos++
		br.bitCnt += 8
	}
	return nil
}

func (br *bitReader) readBits(n uint8) (uint32, error) {
	if n == 0 {
		return 0, nil
	}
	if err := br.ensureBits(n); err != nil {
		return 0, err
	}
	mask := (uint64(1) << n) - 1
	val := uint32(br.bitBuf & mask)
	br.bitBuf >>= n
	br.bitCnt -= n
	return val, nil
}

func (br *bitReader) alignToByte() {
	drop := br.bitCnt & 7
	br.bitBuf >>= drop
	br.bitCnt -= drop
}

func (br *bitReader) readU16LE() (uint16, error) {
	lo, err := br.readBits(8)
	if err != nil {
		return 0, err
	}
	hi, err := br.readBits(8)
	if err != nil {
		return 0, err
	}
	return uint16(lo) | (uint16(hi) << 8), nil
}

func (br *bitReader) readBytes(count int) ([]byte, error) {
	br.alignToByte()
	out := make([]byte, count)
	idx := 0
	for br.bitCnt >= 8 && idx < count {
		out[idx] = byte(br.bitBuf & 0xFF)
		br.bitBuf >>= 8
		br.bitCnt -= 8
		idx++
	}
	rem := count - idx
	if br.bytePos+rem > len(br.data) {
		return nil, &CompressError{Kind: ErrUnexpectedEof}
	}
	copy(out[idx:], br.data[br.bytePos:br.bytePos+rem])
	br.bytePos += rem
	return out, nil
}

type huffmanDecoder struct {
	counts  [maxBits + 1]uint16
	symbols []uint16
	maxLen  uint8
}

func buildHuffmanDecoder(lengths []uint8) (huffmanDecoder, error) {
	var counts [maxBits + 1]uint16
	var maxLen uint8
	for _, l := range lengths {
		if l > maxBits {
			return huffmanDecoder{}, &CompressError{Kind: ErrInvalidHuffmanTree, Detail: "code length > 15"}
		}
		if l > 0 {
			counts[l]++
			if l > maxLen {
				maxLen = l
			}
		}
	}

	var offsets [maxBits + 1]uint16
	var total uint16
	for bits := 1; bits <= maxBits; bits++ {
		offsets[bits] = total
		total += counts[bits]
	}

	symbols := make([]uint16, total)
	for sym, l := range lengths {
		if l > 0 {
			pos := offsets[l]
			symbols[pos] = uint16(sym)
			offsets[l]++
		}
	}

	return huffmanDecoder{
		counts:  counts,
		symbols: symbols,
		maxLen:  maxLen,
	}, nil
}

func (hd *huffmanDecoder) decode(br *bitReader) (uint16, error) {
	var code int32
	var first int32
	var index int32
	for length := uint8(1); length <= hd.maxLen; length++ {
		bit, err := br.readBits(1)
		if err != nil {
			return 0, err
		}
		code |= int32(bit)
		count := int32(hd.counts[length])
		if code-count < first {
			return hd.symbols[index+(code-first)], nil
		}
		index += count
		first += count
		first <<= 1
		code <<= 1
	}
	return 0, &CompressError{Kind: ErrInvalidCode}
}

func fixedHuffmanTrees() (huffmanDecoder, huffmanDecoder) {
	var litLengths [288]uint8
	for i := 0; i <= 143; i++ {
		litLengths[i] = 8
	}
	for i := 144; i <= 255; i++ {
		litLengths[i] = 9
	}
	for i := 256; i <= 279; i++ {
		litLengths[i] = 7
	}
	for i := 280; i <= 287; i++ {
		litLengths[i] = 8
	}
	var distLengths [32]uint8
	for i := 0; i < 32; i++ {
		distLengths[i] = 5
	}
	lit, _ := buildHuffmanDecoder(litLengths[:])
	dist, _ := buildHuffmanDecoder(distLengths[:])
	return lit, dist
}

func readDynamicHuffmanTrees(br *bitReader) (huffmanDecoder, huffmanDecoder, error) {
	hlitBits, err := br.readBits(5)
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}
	hdistBits, err := br.readBits(5)
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}
	hclenBits, err := br.readBits(4)
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}
	hlit := int(hlitBits) + 257
	hdist := int(hdistBits) + 1
	hclen := int(hclenBits) + 4
	if hlit > 286 || hdist > 32 {
		return huffmanDecoder{}, huffmanDecoder{}, &CompressError{Kind: ErrInvalidHuffmanTree, Detail: "HLIT/HDIST out of range"}
	}

	var clLengths [19]uint8
	for i := 0; i < hclen; i++ {
		v, err := br.readBits(3)
		if err != nil {
			return huffmanDecoder{}, huffmanDecoder{}, err
		}
		clLengths[clOrder[i]] = uint8(v)
	}
	clTree, err := buildHuffmanDecoder(clLengths[:])
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}

	totalCodes := hlit + hdist
	lengths := make([]uint8, 0, totalCodes)
	for len(lengths) < totalCodes {
		sym, err := clTree.decode(br)
		if err != nil {
			return huffmanDecoder{}, huffmanDecoder{}, err
		}
		switch {
		case sym <= 15:
			lengths = append(lengths, uint8(sym))
		case sym == 16:
			if len(lengths) == 0 {
				return huffmanDecoder{}, huffmanDecoder{}, &CompressError{Kind: ErrInvalidHuffmanTree, Detail: "repeat code 16 with no previous length"}
			}
			prev := lengths[len(lengths)-1]
			extra, err := br.readBits(2)
			if err != nil {
				return huffmanDecoder{}, huffmanDecoder{}, err
			}
			repeat := int(extra) + 3
			for r := 0; r < repeat; r++ {
				lengths = append(lengths, prev)
			}
		case sym == 17:
			extra, err := br.readBits(3)
			if err != nil {
				return huffmanDecoder{}, huffmanDecoder{}, err
			}
			repeat := int(extra) + 3
			for r := 0; r < repeat; r++ {
				lengths = append(lengths, 0)
			}
		case sym == 18:
			extra, err := br.readBits(7)
			if err != nil {
				return huffmanDecoder{}, huffmanDecoder{}, err
			}
			repeat := int(extra) + 11
			for r := 0; r < repeat; r++ {
				lengths = append(lengths, 0)
			}
		default:
			return huffmanDecoder{}, huffmanDecoder{}, &CompressError{Kind: ErrInvalidHuffmanTree, Detail: "unknown code length symbol"}
		}
	}
	if len(lengths) > totalCodes {
		return huffmanDecoder{}, huffmanDecoder{}, &CompressError{Kind: ErrInvalidHuffmanTree, Detail: "code lengths overflow HLIT+HDIST"}
	}

	litTree, err := buildHuffmanDecoder(lengths[:hlit])
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}
	distTree, err := buildHuffmanDecoder(lengths[hlit:])
	if err != nil {
		return huffmanDecoder{}, huffmanDecoder{}, err
	}
	return litTree, distTree, nil
}

func decodeHuffmanBlock(br *bitReader, litTree *huffmanDecoder, distTree *huffmanDecoder, out []byte, maxOutputBytes int) ([]byte, error) {
	for {
		sym, err := litTree.decode(br)
		if err != nil {
			return out, err
		}
		switch {
		case sym < 256:
			if len(out) >= maxOutputBytes {
				return out, &CompressError{Kind: ErrOutputLimitExceeded, Limit: maxOutputBytes}
			}
			out = append(out, byte(sym))
		case sym == 256:
			return out, nil
		case sym <= 285:
			lenIdx := int(sym - 257)
			extraBits := lengthExtra[lenIdx]
			extra, err := br.readBits(extraBits)
			if err != nil {
				return out, err
			}
			length := int(lengthBases[lenIdx]) + int(extra)

			distSym, err := distTree.decode(br)
			if err != nil {
				return out, err
			}
			if distSym >= 30 {
				return out, &CompressError{Kind: ErrInvalidDistance, Actual: uint32(distSym)}
			}
			distEx := distExtra[distSym]
			distExtraVal, err := br.readBits(distEx)
			if err != nil {
				return out, err
			}
			distance := int(distBases[distSym]) + int(distExtraVal)
			if distance == 0 || distance > len(out) {
				return out, &CompressError{Kind: ErrInvalidDistance, Actual: uint32(distance)}
			}
			if len(out)+length > maxOutputBytes {
				return out, &CompressError{Kind: ErrOutputLimitExceeded, Limit: maxOutputBytes}
			}
			start := len(out) - distance
			for i := 0; i < length; i++ {
				out = append(out, out[start+i])
			}
		default:
			return out, &CompressError{Kind: ErrInvalidCode}
		}
	}
}

// DeflateCompress compresses input into an RFC 1951 DEFLATE bitstream using
// 32 KiB LZ77 hash-chain matching and fixed Huffman coding (or stored blocks
// when smaller).
func DeflateCompress(input []byte) []byte {
	if len(input) == 0 {
		return []byte{0x01, 0x00, 0x00, 0xFF, 0xFF}
	}

	bw := bitWriter{}
	bw.writeBits(1, 1) // BFINAL = 1
	bw.writeBits(1, 2) // BTYPE = 01 (fixed Huffman)

	const windowSize = 32768
	const hashBits = 15
	const hashSize = 1 << hashBits
	const hashMask = hashSize - 1
	const maxChain = 32

	head := make([]int, hashSize)
	for i := range head {
		head[i] = -1
	}
	prev := make([]int, len(input))
	for i := range prev {
		prev[i] = -1
	}

	hash3 := func(pos int) int {
		v := (uint32(input[pos]) << 16) | (uint32(input[pos+1]) << 8) | uint32(input[pos+2])
		return int(((v * 0x1e35a7bd) >> (32 - hashBits)) & hashMask)
	}

	pos := 0
	for pos < len(input) {
		bestLen := 0
		bestDist := 0

		if pos+3 <= len(input) {
			h := hash3(pos)
			cand := head[h]
			prev[pos] = cand
			head[h] = pos

			chain := 0
			maxLen := minInt(258, len(input)-pos)
			for cand >= 0 && chain < maxChain {
				dist := pos - cand
				if dist > windowSize {
					break
				}
				if input[cand+bestLen] == input[pos+bestLen] {
					l := 0
					for l < maxLen && input[cand+l] == input[pos+l] {
						l++
					}
					if l >= 3 && l > bestLen {
						bestLen = l
						bestDist = dist
						if l == maxLen {
							break
						}
					}
				}
				cand = prev[cand]
				chain++
			}
		}

		if bestLen >= 3 {
			writeFixedLength(&bw, bestLen)
			writeFixedDistance(&bw, bestDist)
			for step := 1; step < bestLen; step++ {
				p := pos + step
				if p+3 <= len(input) {
					h := hash3(p)
					prev[p] = head[h]
					head[h] = p
				}
			}
			pos += bestLen
		} else {
			writeFixedLiteral(&bw, uint16(input[pos]))
			pos++
		}
	}

	writeFixedLiteral(&bw, 256) // end-of-block
	huffmanBytes := bw.finish()

	storedOverhead := 5 * ((len(input) + 65534) / 65535)
	if len(huffmanBytes) <= len(input)+storedOverhead {
		return huffmanBytes
	}
	return deflateCompressStored(input)
}

func deflateCompressStored(input []byte) []byte {
	out := make([]byte, 0, len(input)+5*(len(input)/65535+1))
	offset := 0
	for {
		rem := len(input) - offset
		chunkLen := minInt(rem, 65535)
		isFinal := offset+chunkLen >= len(input)
		var bfinal byte
		if isFinal {
			bfinal = 0x01
		}
		out = append(out, bfinal)
		l := uint16(chunkLen)
		nl := ^l
		out = append(out, byte(l), byte(l>>8), byte(nl), byte(nl>>8))
		out = append(out, input[offset:offset+chunkLen]...)
		offset += chunkLen
		if isFinal {
			break
		}
	}
	return out
}

type bitWriter struct {
	out    []byte
	bitBuf uint64
	bitCnt uint8
}

func (bw *bitWriter) writeBits(val uint32, n uint8) {
	bw.bitBuf |= (uint64(val) & ((uint64(1) << n) - 1)) << bw.bitCnt
	bw.bitCnt += n
	for bw.bitCnt >= 8 {
		bw.out = append(bw.out, byte(bw.bitBuf&0xFF))
		bw.bitBuf >>= 8
		bw.bitCnt -= 8
	}
}

func (bw *bitWriter) writeHuffmanMSB(code uint16, length uint8) {
	var rev uint32
	for i := uint8(0); i < length; i++ {
		bit := (code >> (length - 1 - i)) & 1
		rev |= uint32(bit) << i
	}
	bw.writeBits(rev, length)
}

func (bw *bitWriter) finish() []byte {
	if bw.bitCnt > 0 {
		bw.out = append(bw.out, byte(bw.bitBuf&0xFF))
		bw.bitBuf = 0
		bw.bitCnt = 0
	}
	return bw.out
}

func writeFixedLiteral(bw *bitWriter, sym uint16) {
	switch {
	case sym <= 143:
		bw.writeHuffmanMSB(0x30+sym, 8)
	case sym <= 255:
		bw.writeHuffmanMSB(0x190+(sym-144), 9)
	case sym <= 279:
		bw.writeHuffmanMSB(sym-256, 7)
	default:
		bw.writeHuffmanMSB(0xC0+(sym-280), 8)
	}
}

func writeFixedLength(bw *bitWriter, length int) {
	idx := len(lengthBases) - 1
	for i := 0; i < len(lengthBases); i++ {
		nextBase := 259
		if i+1 < len(lengthBases) {
			nextBase = int(lengthBases[i+1])
		}
		if length >= int(lengthBases[i]) && length < nextBase {
			idx = i
			break
		}
	}
	sym := uint16(257 + idx)
	writeFixedLiteral(bw, sym)
	extra := lengthExtra[idx]
	if extra > 0 {
		bw.writeBits(uint32(length-int(lengthBases[idx])), extra)
	}
}

func writeFixedDistance(bw *bitWriter, dist int) {
	idx := len(distBases) - 1
	for i := 0; i < len(distBases); i++ {
		nextBase := 32769
		if i+1 < len(distBases) {
			nextBase = int(distBases[i+1])
		}
		if dist >= int(distBases[i]) && dist < nextBase {
			idx = i
			break
		}
	}
	bw.writeHuffmanMSB(uint16(idx), 5)
	extra := distExtra[idx]
	if extra > 0 {
		bw.writeBits(uint32(dist-int(distBases[idx])), extra)
	}
}

func minInt(a, b int) int {
	if a < b {
		return a
	}
	return b
}
