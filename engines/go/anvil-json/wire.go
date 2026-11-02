// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anviljson

import (
	"encoding/binary"
	"errors"
	"io"
	"math"
)

// Binary wire tags for transferring parsed JSON Value trees across the Go
// worker pipe without loss of precision, key order, or duplicate keys.
const (
	wireTagNull   byte = 0
	wireTagBool   byte = 1
	wireTagNumber byte = 2
	wireTagString byte = 3
	wireTagArray  byte = 4
	wireTagObject byte = 5
)

// EncodeBinaryValue appends a compact binary representation of v to dst.
func EncodeBinaryValue(dst []byte, v Value) []byte {
	switch v.kind {
	case KindNull:
		return append(dst, wireTagNull)
	case KindBool:
		b := byte(0)
		if v.boolean {
			b = 1
		}
		return append(dst, wireTagBool, b)
	case KindNumber:
		var buf [9]byte
		buf[0] = wireTagNumber
		binary.LittleEndian.PutUint64(buf[1:9], math.Float64bits(v.number))
		return append(dst, buf[:]...)
	case KindString:
		var hdr [5]byte
		hdr[0] = wireTagString
		binary.LittleEndian.PutUint32(hdr[1:5], uint32(len(v.str)))
		dst = append(dst, hdr[:]...)
		return append(dst, v.str...)
	case KindArray:
		var hdr [5]byte
		hdr[0] = wireTagArray
		binary.LittleEndian.PutUint32(hdr[1:5], uint32(len(v.array)))
		dst = append(dst, hdr[:]...)
		for _, elem := range v.array {
			dst = EncodeBinaryValue(dst, elem)
		}
		return dst
	case KindObject:
		var hdr [5]byte
		hdr[0] = wireTagObject
		binary.LittleEndian.PutUint32(hdr[1:5], uint32(len(v.object)))
		dst = append(dst, hdr[:]...)
		for _, m := range v.object {
			var klen [4]byte
			binary.LittleEndian.PutUint32(klen[:], uint32(len(m.Key)))
			dst = append(dst, klen[:]...)
			dst = append(dst, m.Key...)
			dst = EncodeBinaryValue(dst, m.Value)
		}
		return dst
	default:
		return append(dst, wireTagNull)
	}
}

// DecodeBinaryValue decodes a Value written by EncodeBinaryValue from src[pos:].
func DecodeBinaryValue(src []byte, pos *int) (Value, error) {
	if *pos >= len(src) {
		return Null(), io.ErrUnexpectedEOF
	}
	tag := src[*pos]
	*pos++
	switch tag {
	case wireTagNull:
		return Null(), nil
	case wireTagBool:
		if *pos >= len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		b := src[*pos] != 0
		*pos++
		return Bool(b), nil
	case wireTagNumber:
		if *pos+8 > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		bits := binary.LittleEndian.Uint64(src[*pos : *pos+8])
		*pos += 8
		return Number(math.Float64frombits(bits)), nil
	case wireTagString:
		if *pos+4 > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		n := int(binary.LittleEndian.Uint32(src[*pos : *pos+4]))
		*pos += 4
		if n < 0 || *pos+n > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		s := string(src[*pos : *pos+n])
		*pos += n
		return StringVal(s), nil
	case wireTagArray:
		if *pos+4 > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		count := int(binary.LittleEndian.Uint32(src[*pos : *pos+4]))
		*pos += 4
		if count < 0 || count > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		items := make([]Value, count)
		for i := 0; i < count; i++ {
			elem, err := DecodeBinaryValue(src, pos)
			if err != nil {
				return Null(), err
			}
			items[i] = elem
		}
		return Value{kind: KindArray, array: items}, nil
	case wireTagObject:
		if *pos+4 > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		count := int(binary.LittleEndian.Uint32(src[*pos : *pos+4]))
		*pos += 4
		if count < 0 || count > len(src) {
			return Null(), io.ErrUnexpectedEOF
		}
		members := make([]Member, count)
		for i := 0; i < count; i++ {
			if *pos+4 > len(src) {
				return Null(), io.ErrUnexpectedEOF
			}
			klen := int(binary.LittleEndian.Uint32(src[*pos : *pos+4]))
			*pos += 4
			if klen < 0 || *pos+klen > len(src) {
				return Null(), io.ErrUnexpectedEOF
			}
			key := string(src[*pos : *pos+klen])
			*pos += klen
			val, err := DecodeBinaryValue(src, pos)
			if err != nil {
				return Null(), err
			}
			members[i] = Member{Key: key, Value: val}
		}
		return Value{kind: KindObject, object: members}, nil
	default:
		return Null(), errors.New("unknown binary JSON value tag")
	}
}
