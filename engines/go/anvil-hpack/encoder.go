// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilhpack

// Encoder encodes HTTP/2 header blocks in order for one connection.
type Encoder struct {
	table             *DynamicTable
	hasPendingSize    bool
	pendingSizeUpdate int
	hasPendingMin     bool
	pendingMinSize    int
	useHuffman        bool
}

// NewEncoder creates an Encoder with the default 4,096-byte dynamic table and Huffman enabled.
func NewEncoder() *Encoder {
	return &Encoder{
		table:      NewDynamicTable(DefaultTableSize),
		useHuffman: true,
	}
}

// NewEncoderWithTableSize creates an Encoder whose dynamic table starts at size bytes.
func NewEncoderWithTableSize(size int) *Encoder {
	return &Encoder{
		table:      NewDynamicTable(size),
		useHuffman: true,
	}
}

// SetHuffman enables or disables Huffman string literal coding.
func (e *Encoder) SetHuffman(on bool) {
	e.useHuffman = on
}

// Table returns the Encoder's dynamic table.
func (e *Encoder) Table() *DynamicTable {
	return e.table
}

// SetMaxTableSize updates the dynamic table capacity and records a pending
// RFC 7541 §4.2 table size update for the next encoded block.
func (e *Encoder) SetMaxTableSize(size int) {
	e.table.SetMaxSize(size)
	e.hasPendingSize = true
	e.pendingSizeUpdate = size
	if !e.hasPendingMin || size < e.pendingMinSize {
		e.hasPendingMin = true
		e.pendingMinSize = size
	}
}

// Encode appends the HPACK representation of headers to out.
func (e *Encoder) Encode(headers []Header, out []byte) []byte {
	if e.hasPendingSize {
		size := e.pendingSizeUpdate
		e.hasPendingSize = false
		if e.hasPendingMin {
			minSz := e.pendingMinSize
			e.hasPendingMin = false
			if minSz < size {
				out = EncodeInteger(minSz, 5, 0x20, out)
			}
		}
		out = EncodeInteger(size, 5, 0x20, out)
	}
	for _, h := range headers {
		out = e.encodeField(h, out)
	}
	return out
}

func (e *Encoder) encodeField(header Header, out []byte) []byte {
	idx, exact, found := e.table.Find(header.Name, header.Value)
	if header.NeverIndexed {
		nameIdx := 0
		if found {
			nameIdx = idx
		}
		out = EncodeInteger(nameIdx, 4, 0x10, out)
		if nameIdx == 0 {
			out = e.writeString(header.Name, out)
		}
		return e.writeString(header.Value, out)
	}
	if found && exact {
		return EncodeInteger(idx, 7, 0x80, out)
	}
	nameIdx := 0
	if found {
		nameIdx = idx
	}
	worthIndexing := EntrySize(header.Name, header.Value) <= e.table.MaxSize()/2
	if worthIndexing {
		out = EncodeInteger(nameIdx, 6, 0x40, out)
	} else {
		out = EncodeInteger(nameIdx, 4, 0x00, out)
	}
	if nameIdx == 0 {
		out = e.writeString(header.Name, out)
	}
	out = e.writeString(header.Value, out)
	if worthIndexing {
		e.table.Insert(header.Name, header.Value)
	}
	return out
}

func (e *Encoder) writeString(data []byte, out []byte) []byte {
	huffLen := HuffmanEncodedLen(data)
	if e.useHuffman && huffLen <= len(data) {
		out = EncodeInteger(huffLen, 7, 0x80, out)
		return HuffmanEncode(data, out)
	}
	out = EncodeInteger(len(data), 7, 0x00, out)
	return append(out, data...)
}
