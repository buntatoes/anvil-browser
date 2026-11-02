// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

// Package anvilhpack implements clean-room RFC 7541 HPACK header compression
// and decompression for HTTP/2, including N-bit prefixed integers, the 61-entry
// static table, FIFO dynamic table eviction, canonical Huffman coding, and
// hostile-input caps.
package anvilhpack

import (
	"bytes"
	"fmt"
)

// Version is the semantic version of anvil-hpack.
const Version = "0.2.0"

// DefaultTableSize is the RFC 7541 §6.5.2 initial dynamic table size (4,096 bytes).
const DefaultTableSize = 4096

// DefaultMaxHeaderListSize is the default 256 KiB cap on decoded header list size.
const DefaultMaxHeaderListSize = 256 * 1024

// MaxStringLen is the 64 KiB cap on a single header name or value string literal.
const MaxStringLen = 64 * 1024

// MaxInteger is the maximum prefixed integer accepted by DecodeInteger (u32::MAX).
const MaxInteger = int(^uint32(0))

// Header represents one HTTP/2 header field in a block.
type Header struct {
	Name         []byte
	Value        []byte
	NeverIndexed bool
}

// NewHeader creates a standard indexable header field.
func NewHeader(name, value string) Header {
	return Header{
		Name:         []byte(name),
		Value:        []byte(value),
		NeverIndexed: false,
	}
}

// NewHeaderBytes creates a standard indexable header field from byte slices.
func NewHeaderBytes(name, value []byte) Header {
	n := make([]byte, len(name))
	v := make([]byte, len(value))
	copy(n, name)
	copy(v, value)
	return Header{Name: n, Value: v, NeverIndexed: false}
}

// NewSensitiveHeader creates a never-indexed header field (RFC 7541 §6.2.3).
func NewSensitiveHeader(name, value string) Header {
	return Header{
		Name:         []byte(name),
		Value:        []byte(value),
		NeverIndexed: true,
	}
}

// NameStr returns the header name as a string.
func (h Header) NameStr() string {
	return string(h.Name)
}

// ValueStr returns the header value as a string.
func (h Header) ValueStr() string {
	return string(h.Value)
}

// Equal reports whether h and other have identical name, value, and never-indexed flag.
func (h Header) Equal(other Header) bool {
	return h.NeverIndexed == other.NeverIndexed &&
		bytes.Equal(h.Name, other.Name) &&
		bytes.Equal(h.Value, other.Value)
}

// ErrorKind identifies the specific RFC 7541 decoding failure.
type ErrorKind uint8

const (
	ErrTruncated ErrorKind = iota
	ErrIntegerOverflow
	ErrInvalidIndex
	ErrInvalidHuffman
	ErrStringTooLong
	ErrTableSizeTooLarge
	ErrLateTableSizeUpdate
	ErrHeaderListTooLarge
)

// HpackError describes a header block decoding failure.
type HpackError struct {
	Kind  ErrorKind
	Value int
}

func (e *HpackError) Error() string {
	switch e.Kind {
	case ErrTruncated:
		return "hpack block ended mid-field"
	case ErrIntegerOverflow:
		return "hpack integer is too large"
	case ErrInvalidIndex:
		return fmt.Sprintf("hpack table index %d is out of range", e.Value)
	case ErrInvalidHuffman:
		return "invalid hpack huffman string"
	case ErrStringTooLong:
		return fmt.Sprintf("hpack string of %d bytes is over the limit", e.Value)
	case ErrTableSizeTooLarge:
		return fmt.Sprintf("hpack table size update of %d bytes is over the limit", e.Value)
	case ErrLateTableSizeUpdate:
		return "hpack table size update after a header field"
	case ErrHeaderListTooLarge:
		return fmt.Sprintf("hpack header list of %d bytes is over the limit", e.Value)
	default:
		return "hpack error"
	}
}
