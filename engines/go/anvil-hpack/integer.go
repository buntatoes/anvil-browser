// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilhpack

// EncodeInteger appends value with an N-bit prefix (RFC 7541 §5.1) to out.
// flags holds the bits above the prefix in the first byte.
func EncodeInteger(value int, prefixBits uint8, flags byte, out []byte) []byte {
	if prefixBits < 1 || prefixBits > 8 {
		prefixBits = 8
	}
	maxPrefix := (1 << prefixBits) - 1
	flagMask := ^byte(maxPrefix)
	if value < maxPrefix {
		return append(out, (flags&flagMask)|byte(value))
	}
	out = append(out, (flags&flagMask)|byte(maxPrefix))
	rest := value - maxPrefix
	for rest >= 128 {
		out = append(out, byte(rest%128)|0x80)
		rest /= 128
	}
	return append(out, byte(rest))
}

// DecodeInteger reads an N-bit prefixed integer starting at input[*pos].
// Values above MaxInteger or encodings with excessive continuation bytes are rejected.
func DecodeInteger(input []byte, pos *int, prefixBits uint8) (int, error) {
	if prefixBits < 1 || prefixBits > 8 {
		return 0, &HpackError{Kind: ErrIntegerOverflow}
	}
	if *pos >= len(input) {
		return 0, &HpackError{Kind: ErrTruncated}
	}
	first := input[*pos]
	maxPrefix := (1 << prefixBits) - 1
	value := int(first) & maxPrefix
	at := *pos + 1
	if value == maxPrefix {
		var shift uint = 0
		for {
			if at >= len(input) {
				return 0, &HpackError{Kind: ErrTruncated}
			}
			b := input[at]
			at++
			if shift > 28 {
				return 0, &HpackError{Kind: ErrIntegerOverflow}
			}
			part := int64(b&0x7F) << shift
			nextVal := int64(value) + part
			if nextVal > int64(MaxInteger) || nextVal < 0 {
				return 0, &HpackError{Kind: ErrIntegerOverflow}
			}
			value = int(nextVal)
			if b&0x80 == 0 {
				break
			}
			shift += 7
		}
	}
	*pos = at
	return value, nil
}
