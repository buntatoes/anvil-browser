// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

// Decoder decodes HTTP/2 header blocks in order for one connection.
type Decoder struct {
	table               *DynamicTable
	maxAllowedTableSize int
	maxHeaderListSize   int
}

// NewDecoder creates a Decoder with DefaultTableSize (4,096) and DefaultMaxHeaderListSize (64 KiB).
func NewDecoder() *Decoder {
	return NewDecoderWithLimits(DefaultTableSize, DefaultMaxHeaderListSize)
}

// NewDecoderWithLimits creates a Decoder with custom dynamic table and header list caps.
func NewDecoderWithLimits(maxTableSize, maxHeaderListSize int) *Decoder {
	return &Decoder{
		table:               NewDynamicTable(maxTableSize),
		maxAllowedTableSize: maxTableSize,
		maxHeaderListSize:   maxHeaderListSize,
	}
}

// Table returns the Decoder's dynamic table.
func (d *Decoder) Table() *DynamicTable {
	return d.table
}

// SetMaxAllowedTableSize changes the largest dynamic table size the peer may request.
func (d *Decoder) SetMaxAllowedTableSize(size int) {
	d.maxAllowedTableSize = size
	if d.table.MaxSize() > size {
		d.table.SetMaxSize(size)
	}
}

// Decode decodes one complete HPACK header block.
func (d *Decoder) Decode(block []byte) ([]Header, error) {
	var headers []Header
	listSize := 0
	pos := 0
	seenField := false

	for pos < len(block) {
		first := block[pos]
		var h Header
		var err error

		if first&0x80 != 0 {
			// Indexed Header Field (RFC 7541 §6.1)
			idx, err := DecodeInteger(block, &pos, 7)
			if err != nil {
				return nil, err
			}
			name, value, ok := d.table.Get(idx)
			if !ok {
				return nil, &HpackError{Kind: ErrInvalidIndex, Value: idx}
			}
			h = NewHeaderBytes(name, value)
		} else if first&0xC0 == 0x40 {
			// Literal Header Field with Incremental Indexing (RFC 7541 §6.2.1)
			h, err = d.readLiteral(block, &pos, 6, false)
			if err != nil {
				return nil, err
			}
			d.table.Insert(h.Name, h.Value)
		} else if first&0xE0 == 0x20 {
			// Dynamic Table Size Update (RFC 7541 §6.3)
			if seenField {
				return nil, &HpackError{Kind: ErrLateTableSizeUpdate}
			}
			size, err := DecodeInteger(block, &pos, 5)
			if err != nil {
				return nil, err
			}
			if size > d.maxAllowedTableSize {
				return nil, &HpackError{Kind: ErrTableSizeTooLarge, Value: size}
			}
			d.table.SetMaxSize(size)
			continue
		} else if first&0xF0 == 0x10 {
			// Literal Header Field Never Indexed (RFC 7541 §6.2.3)
			h, err = d.readLiteral(block, &pos, 4, true)
			if err != nil {
				return nil, err
			}
		} else {
			// Literal Header Field without Indexing (RFC 7541 §6.2.2)
			h, err = d.readLiteral(block, &pos, 4, false)
			if err != nil {
				return nil, err
			}
		}

		seenField = true
		listSize += EntrySize(h.Name, h.Value)
		if listSize > d.maxHeaderListSize {
			return nil, &HpackError{Kind: ErrHeaderListTooLarge, Value: listSize}
		}
		headers = append(headers, h)
	}
	return headers, nil
}

func (d *Decoder) readLiteral(block []byte, pos *int, prefixBits uint8, neverIndexed bool) (Header, error) {
	idx, err := DecodeInteger(block, pos, prefixBits)
	if err != nil {
		return Header{}, err
	}
	var name []byte
	if idx == 0 {
		name, err = readString(block, pos)
		if err != nil {
			return Header{}, err
		}
	} else {
		n, _, ok := d.table.Get(idx)
		if !ok {
			return Header{}, &HpackError{Kind: ErrInvalidIndex, Value: idx}
		}
		name = make([]byte, len(n))
		copy(name, n)
	}
	value, err := readString(block, pos)
	if err != nil {
		return Header{}, err
	}
	return Header{
		Name:         name,
		Value:        value,
		NeverIndexed: neverIndexed,
	}, nil
}

func readString(block []byte, pos *int) ([]byte, error) {
	if *pos >= len(block) {
		return nil, &HpackError{Kind: ErrTruncated}
	}
	huffman := block[*pos]&0x80 != 0
	length, err := DecodeInteger(block, pos, 7)
	if err != nil {
		return nil, err
	}
	if length > MaxStringLen {
		return nil, &HpackError{Kind: ErrStringTooLong, Value: length}
	}
	if *pos+length > len(block) {
		return nil, &HpackError{Kind: ErrTruncated}
	}
	raw := block[*pos : *pos+length]
	*pos += length
	if huffman {
		return HuffmanDecode(raw, MaxStringLen)
	}
	out := make([]byte, len(raw))
	copy(out, raw)
	return out, nil
}
