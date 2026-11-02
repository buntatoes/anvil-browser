// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
	"strings"
	"unicode/utf16"
)

// Binary big-endian read helpers.

func readU16BE(data []byte, offset int) (uint16, error) {
	if offset < 0 || offset+2 > len(data) {
		return 0, errUnexpectedEOF()
	}
	return binary.BigEndian.Uint16(data[offset : offset+2]), nil
}

func readI16BE(data []byte, offset int) (int16, error) {
	if offset < 0 || offset+2 > len(data) {
		return 0, errUnexpectedEOF()
	}
	return int16(binary.BigEndian.Uint16(data[offset : offset+2])), nil
}

func readU32BE(data []byte, offset int) (uint32, error) {
	if offset < 0 || offset+4 > len(data) {
		return 0, errUnexpectedEOF()
	}
	return binary.BigEndian.Uint32(data[offset : offset+4]), nil
}

func readI32BE(data []byte, offset int) (int32, error) {
	if offset < 0 || offset+4 > len(data) {
		return 0, errUnexpectedEOF()
	}
	return int32(binary.BigEndian.Uint32(data[offset : offset+4])), nil
}

// TableRecord stores the location and length of an SFNT table.
type TableRecord struct {
	Offset int
	Length int
}

// parseTableDirectory parses the SFNT offset table and 16-byte table directory records.
func parseTableDirectory(bytes []byte) (map[[4]byte]TableRecord, error) {
	if len(bytes) > MaxFontBytes {
		return nil, errFontTooLarge(len(bytes))
	}
	if len(bytes) < 12 {
		return nil, errUnexpectedEOF()
	}

	sig, err := readU32BE(bytes, 0)
	if err != nil {
		return nil, err
	}
	// 0x00010000 (TrueType 1.0), 0x74727565 ('true'), 0x4F54544F ('OTTO' - OpenType CFF)
	if sig != 0x00010000 && sig != 0x74727565 && sig != 0x4F54544F {
		return nil, errInvalidSignature()
	}

	numTablesRaw, err := readU16BE(bytes, 4)
	if err != nil {
		return nil, err
	}
	numTables := int(numTablesRaw)
	if numTables > MaxSfntTables {
		return nil, errTooManyTables(numTables)
	}

	dirEnd := 12 + numTables*16
	if dirEnd > len(bytes) {
		return nil, errUnexpectedEOF()
	}

	tables := make(map[[4]byte]TableRecord, numTables)
	for i := 0; i < numTables; i++ {
		rec := 12 + i*16
		var tag [4]byte
		copy(tag[:], bytes[rec:rec+4])

		off, err := readU32BE(bytes, rec+8)
		if err != nil {
			return nil, err
		}
		length, err := readU32BE(bytes, rec+12)
		if err != nil {
			return nil, err
		}

		offsetInt := int(off)
		lengthInt := int(length)
		if offsetInt < 0 || lengthInt < 0 || offsetInt+lengthInt > len(bytes) {
			return nil, errUnexpectedEOF()
		}

		tables[tag] = TableRecord{Offset: offsetInt, Length: lengthInt}
	}

	return tables, nil
}

func sliceTable(bytes []byte, tables map[[4]byte]TableRecord, tagStr string) ([]byte, error) {
	if len(tagStr) != 4 {
		return nil, errMissingTable(tagStr)
	}
	var tag [4]byte
	copy(tag[:], []byte(tagStr))

	rec, ok := tables[tag]
	if !ok {
		return nil, errMissingTable(tagStr)
	}
	if rec.Offset+rec.Length > len(bytes) {
		return nil, errUnexpectedEOF()
	}
	return bytes[rec.Offset : rec.Offset+rec.Length], nil
}

// parseHead extracts unitsPerEm and indexToLocFormat from the head table.
func parseHead(head []byte) (unitsPerEm uint16, indexToLocFormat int16, err error) {
	if len(head) < 54 {
		return 0, 0, errInvalidTable("head")
	}
	unitsPerEm, err = readU16BE(head, 18)
	if err != nil || unitsPerEm == 0 {
		return 0, 0, errInvalidTable("head")
	}
	indexToLocFormat, err = readI16BE(head, 50)
	if err != nil {
		return 0, 0, errInvalidTable("head")
	}
	return unitsPerEm, indexToLocFormat, nil
}

// parseHhea extracts ascender, descender, lineGap, and numberOfHMetrics from the hhea table.
func parseHhea(hhea []byte) (ascender, descender, lineGap int16, numHMetrics int, err error) {
	if len(hhea) < 36 {
		return 0, 0, 0, 0, errInvalidTable("hhea")
	}
	ascender, err = readI16BE(hhea, 4)
	if err != nil {
		return 0, 0, 0, 0, errInvalidTable("hhea")
	}
	descender, err = readI16BE(hhea, 6)
	if err != nil {
		return 0, 0, 0, 0, errInvalidTable("hhea")
	}
	lineGap, err = readI16BE(hhea, 8)
	if err != nil {
		return 0, 0, 0, 0, errInvalidTable("hhea")
	}
	numHRaw, err := readU16BE(hhea, 34)
	if err != nil || numHRaw == 0 {
		return 0, 0, 0, 0, errInvalidTable("hhea")
	}
	return ascender, descender, lineGap, int(numHRaw), nil
}

// parseMaxp extracts numGlyphs from the maxp table.
func parseMaxp(maxp []byte) (int, error) {
	if len(maxp) < 6 {
		return 0, errInvalidTable("maxp")
	}
	numGlyphs, err := readU16BE(maxp, 4)
	if err != nil {
		return 0, errInvalidTable("maxp")
	}
	return int(numGlyphs), nil
}

// parseHmtx decodes horizontal metrics for all glyphs.
func parseHmtx(hmtx []byte, numHMetrics, numGlyphs int) ([]HMetric, error) {
	if len(hmtx) < numHMetrics*4 {
		return nil, errInvalidTable("hmtx")
	}

	total := numGlyphs
	if numHMetrics > total {
		total = numHMetrics
	}
	metrics := make([]HMetric, total)

	for i := 0; i < numHMetrics; i++ {
		adv, err := readU16BE(hmtx, i*4)
		if err != nil {
			return nil, err
		}
		lsb, err := readI16BE(hmtx, i*4+2)
		if err != nil {
			return nil, err
		}
		metrics[i] = HMetric{AdvanceWidth: adv, LeftSideBearing: lsb}
	}

	lastAdvance := uint16(0)
	if numHMetrics > 0 {
		lastAdvance = metrics[numHMetrics-1].AdvanceWidth
	}

	lsbOffset := numHMetrics * 4
	for i := numHMetrics; i < total; i++ {
		idx := lsbOffset + (i-numHMetrics)*2
		lsb := int16(0)
		if idx+2 <= len(hmtx) {
			if val, err := readI16BE(hmtx, idx); err == nil {
				lsb = val
			}
		}
		metrics[i] = HMetric{AdvanceWidth: lastAdvance, LeftSideBearing: lsb}
	}

	return metrics, nil
}

// parseNameTable decodes the family name (nameID = 1) or full name fallback (nameID = 4).
func parseNameTable(nameTable []byte) string {
	if len(nameTable) < 6 {
		return ""
	}
	countRaw, err := readU16BE(nameTable, 2)
	if err != nil {
		return ""
	}
	stringOffsetRaw, err := readU16BE(nameTable, 4)
	if err != nil {
		return ""
	}

	count := int(countRaw)
	stringOffset := int(stringOffsetRaw)
	if count > 512 || 6+count*12 > len(nameTable) || stringOffset > len(nameTable) {
		return ""
	}

	var fallback string
	for i := 0; i < count; i++ {
		rec := 6 + i*12
		platformID, err := readU16BE(nameTable, rec)
		if err != nil {
			continue
		}
		nameID, err := readU16BE(nameTable, rec+6)
		if err != nil {
			continue
		}
		lengthRaw, err := readU16BE(nameTable, rec+8)
		if err != nil {
			continue
		}
		offsetRaw, err := readU16BE(nameTable, rec+10)
		if err != nil {
			continue
		}

		if nameID != 1 && nameID != 4 {
			continue
		}

		length := int(lengthRaw)
		offset := int(offsetRaw)
		start := stringOffset + offset
		end := start + length
		if length == 0 || length > 256 || start < 0 || end > len(nameTable) {
			continue
		}

		raw := nameTable[start:end]
		var decoded string
		if platformID == 0 || platformID == 3 {
			// Big-endian UTF-16
			u16Count := len(raw) / 2
			u16s := make([]uint16, u16Count)
			for j := 0; j < u16Count; j++ {
				u16s[j] = binary.BigEndian.Uint16(raw[j*2 : j*2+2])
			}
			decoded = string(utf16.Decode(u16s))
		} else {
			// Printable ASCII
			var b strings.Builder
			for _, ch := range raw {
				if ch >= 32 && ch <= 126 {
					b.WriteByte(ch)
				}
			}
			decoded = b.String()
		}

		trimmed := strings.TrimSpace(decoded)
		if trimmed != "" {
			if nameID == 1 {
				return trimmed
			}
			if fallback == "" {
				fallback = trimmed
			}
		}
	}

	return fallback
}
