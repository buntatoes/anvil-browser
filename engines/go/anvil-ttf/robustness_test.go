// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
	"math"
	"math/rand"
	"testing"
)

func TestMalformedFontHeader(t *testing.T) {
	// Empty slice
	if _, err := Parse([]byte{}); err == nil {
		t.Error("Expected error for empty slice, got nil")
	}

	// Truncated header
	if _, err := Parse([]byte{0, 1, 0}); err == nil {
		t.Error("Expected error for 3-byte slice, got nil")
	}

	// Invalid signature
	badSig := []byte{0xDE, 0xAD, 0xBE, 0xEF, 0, 1, 0, 0, 0, 0, 0, 0}
	if _, err := Parse(badSig); err == nil {
		t.Error("Expected error for bad signature, got nil")
	}

	// Excessive table count (> MaxSfntTables = 1280)
	tooManyTables := make([]byte, 12)
	binary.BigEndian.PutUint32(tooManyTables[0:4], 0x00010000)
	binary.BigEndian.PutUint16(tooManyTables[4:6], 2000)
	if _, err := Parse(tooManyTables); err == nil {
		t.Error("Expected error for >1280 tables, got nil")
	}
}

func TestMalformedTables(t *testing.T) {
	// Missing required tables
	minimal := buildMinimalSyntheticTTF()

	// Truncate minimal font to middle of directory
	if _, err := Parse(minimal[:40]); err == nil {
		t.Error("Expected error for truncated directory, got nil")
	}

	// Parse with zero unitsPerEm in head table
	corruptedHead := make([]byte, len(minimal))
	copy(corruptedHead, minimal)
	// Find 'head' table in minimal and zero out unitsPerEm (bytes 18..20 of head)
	// In buildMinimalSyntheticTTF, head is at offset 12 + 8*16 = 140
	// Let's verify Parse handles zero unitsPerEm
	headZero := make([]byte, 54)
	binary.BigEndian.PutUint16(headZero[18:20], 0)
	if _, _, err := parseHead(headZero); err == nil {
		t.Error("parseHead should fail when unitsPerEm is 0")
	}

	// Truncated hhea
	if _, _, _, _, err := parseHhea([]byte{0, 1, 2}); err == nil {
		t.Error("parseHhea should fail on truncated slice")
	}

	// Truncated maxp
	if _, err := parseMaxp([]byte{0, 1}); err == nil {
		t.Error("parseMaxp should fail on truncated slice")
	}
}

func TestExtremeFontSizes(t *testing.T) {
	bytes := buildMinimalSyntheticTTF()
	font, err := Parse(bytes)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	extremes := []float32{
		0.0,
		-1.0,
		-100.0,
		float32(math.NaN()),
		float32(math.Inf(1)),
		float32(math.Inf(-1)),
		10000.0, // Exceeds MaxGlyphBitmapSide = 512.0
		1e9,
	}

	for _, size := range extremes {
		bmp := font.RasterizeGlyph('A', size)
		if bmp != nil {
			t.Errorf("RasterizeGlyph('A', %f) should return nil, got %dx%d", size, bmp.Width, bmp.Height)
		}

		lcdBmp := font.RasterizeGlyphSubpixelLCD('A', size)
		if lcdBmp != nil {
			t.Errorf("RasterizeGlyphSubpixelLCD('A', %f) should return nil", size)
		}

		styledBmp := font.RasterizeGlyphStyled('A', size, 0, true, true)
		if styledBmp != nil {
			t.Errorf("RasterizeGlyphStyled('A', %f) should return nil", size)
		}
	}
}

func TestCompoundGlyphDepthLimit(t *testing.T) {
	// Self-referential composite glyph: glyph 0 references glyph 0
	var glyf []byte
	appendI16 := func(v int16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, uint16(v))
		glyf = append(glyf, buf...)
	}
	appendU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		glyf = append(glyf, buf...)
	}

	// Composite glyph header
	appendI16(-1) // numberOfContours = -1 (composite)
	appendI16(0)
	appendI16(0)
	appendI16(100)
	appendI16(100)

	// Component 0: references glyph 0 with MORE_COMPONENTS flag
	appendU16(0x0002 | 0x0020) // ARGS_ARE_XY_VALUES | MORE_COMPONENTS
	appendU16(0)               // compGid = 0 (self-reference)
	appendI16(10)              // dx
	appendI16(10)              // dy

	offsets := []int{0, len(glyf)}
	// parseSingleGlyph should hit MaxCompositeGlyphDepth (64) and safely return without infinite loop
	_, err := parseSingleGlyph(glyf, glyf, offsets, 0)
	if err != nil {
		t.Logf("Self-referential composite terminated cleanly with: %v", err)
	}
}

func TestCFFCharstringFuzzing(t *testing.T) {
	r := rand.New(rand.NewSource(42))
	for iter := 0; iter < 200; iter++ {
		length := r.Intn(128) + 1
		randomProg := make([]byte, length)
		r.Read(randomProg)
		// Ensure it ends or has random bytes - should never panic
		_, _ = InterpretType2CharString(randomProg)
	}
}
