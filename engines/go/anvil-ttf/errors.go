// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import "fmt"

// ErrorKind identifies the specific category of font error.
type ErrorKind int

const (
	// ErrUnexpectedEOF indicates the read ran past the file or table buffer.
	ErrUnexpectedEOF ErrorKind = iota
	// ErrFontTooLarge indicates the font size exceeds the security limit.
	ErrFontTooLarge
	// ErrTooManyTables indicates the SFNT table count exceeds the security limit.
	ErrTooManyTables
	// ErrInvalidSfntSignature indicates an unrecognized SFNT font signature.
	ErrInvalidSfntSignature
	// ErrMissingTable indicates a required table was absent.
	ErrMissingTable
	// ErrInvalidTable indicates a required or referenced table has corrupted data.
	ErrInvalidTable
)

// TtfError represents an error encountered while parsing or decoding an SFNT font.
type TtfError struct {
	Kind  ErrorKind
	Table string
	Size  int
	Count int
	Msg   string
}

func (e *TtfError) Error() string {
	switch e.Kind {
	case ErrUnexpectedEOF:
		return "unexpected end of font file"
	case ErrFontTooLarge:
		return fmt.Sprintf("font size %d exceeds security limit (%d)", e.Size, MaxFontBytes)
	case ErrTooManyTables:
		return fmt.Sprintf("SFNT table count %d exceeds security limit (%d)", e.Count, MaxSfntTables)
	case ErrInvalidSfntSignature:
		return "unrecognized SFNT font signature"
	case ErrMissingTable:
		return fmt.Sprintf("missing required SFNT table '%s'", e.Table)
	case ErrInvalidTable:
		return fmt.Sprintf("invalid SFNT table '%s'", e.Table)
	default:
		if e.Msg != "" {
			return e.Msg
		}
		return "unknown font error"
	}
}

func errUnexpectedEOF() error {
	return &TtfError{Kind: ErrUnexpectedEOF}
}

func errFontTooLarge(size int) error {
	return &TtfError{Kind: ErrFontTooLarge, Size: size}
}

func errTooManyTables(count int) error {
	return &TtfError{Kind: ErrTooManyTables, Count: count}
}

func errInvalidSignature() error {
	return &TtfError{Kind: ErrInvalidSfntSignature}
}

func errMissingTable(tag string) error {
	return &TtfError{Kind: ErrMissingTable, Table: tag}
}

func errInvalidTable(tag string) error {
	return &TtfError{Kind: ErrInvalidTable, Table: tag}
}
