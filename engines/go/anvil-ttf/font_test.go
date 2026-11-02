// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
	"math"
	"testing"
)

// Helper to construct a synthetic TrueType font with head, hhea, maxp, hmtx, loca, glyf, cmap, and name tables.
func buildMinimalSyntheticTTF() []byte {
	head := make([]byte, 54)
	binary.BigEndian.PutUint16(head[18:20], 1000) // unitsPerEm = 1000
	binary.BigEndian.PutUint16(head[50:52], 1)    // long loca format

	hhea := make([]byte, 36)
	binary.BigEndian.PutUint16(hhea[4:6], 800)
	binary.BigEndian.PutUint16(hhea[6:8], 0xFF38) // descender = -200
	binary.BigEndian.PutUint16(hhea[8:10], 100)   // lineGap = 100
	binary.BigEndian.PutUint16(hhea[34:36], 3)    // numberOfHMetrics = 3

	maxp := make([]byte, 6)
	binary.BigEndian.PutUint16(maxp[4:6], 3) // numGlyphs = 3

	var hmtx []byte
	metrics := [][2]int16{{500, 0}, {700, 20}, {300, 10}}
	for _, m := range metrics {
		advBuf := make([]byte, 2)
		lsbBuf := make([]byte, 2)
		binary.BigEndian.PutUint16(advBuf, uint16(m[0]))
		binary.BigEndian.PutUint16(lsbBuf, uint16(m[1]))
		hmtx = append(hmtx, advBuf...)
		hmtx = append(hmtx, lsbBuf...)
	}

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

	// Glyph 1: rectangle for 'A' (x: 0..600, y: 0..700)
	appendI16(1)   // numContours = 1
	appendI16(0)   // xMin
	appendI16(0)   // yMin
	appendI16(600) // xMax
	appendI16(700) // yMax
	appendU16(3)   // endPts[0] = 3
	appendU16(0)   // instruction length = 0
	for i := 0; i < 4; i++ {
		glyf = append(glyf, 0x01) // on-curve point
	}
	for _, dx := range []int16{0, 600, 0, -600} {
		appendI16(dx)
	}
	for _, dy := range []int16{0, 0, 700, -700} {
		appendI16(dy)
	}
	glyph1Len := uint32(len(glyf))

	var loca []byte
	appendLocaU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		loca = append(loca, buf...)
	}
	appendLocaU32(0)
	appendLocaU32(0)
	appendLocaU32(glyph1Len)
	appendLocaU32(glyph1Len)

	var cmap []byte
	appendCmapU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		cmap = append(cmap, buf...)
	}
	appendCmapU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		cmap = append(cmap, buf...)
	}

	appendCmapU16(0)  // version
	appendCmapU16(1)  // numSubtables = 1
	appendCmapU16(3)  // platformID = 3 (Windows)
	appendCmapU16(10) // encodingID = 10 (Format 12)
	appendCmapU32(12) // subOffset = 12

	// Format 12 subtable
	appendCmapU16(12) // format
	appendCmapU16(0)  // reserved
	appendCmapU32(40) // length
	appendCmapU32(0)  // language
	appendCmapU32(2)  // numGroups = 2
	// Group 1: 'A' (65) -> gid 1
	appendCmapU32(65)
	appendCmapU32(65)
	appendCmapU32(1)
	// Group 2: 'i' (105) -> gid 2
	appendCmapU32(105)
	appendCmapU32(105)
	appendCmapU32(2)

	nameTbl := []byte{
		0, 0, // format
		0, 1, // count = 1
		0, 18, // stringOffset = 18
		// record 0
		0, 3, // platformID = 3
		0, 1, // encodingID = 1
		0, 0, // languageID = 0
		0, 1, // nameID = 1 (Family name)
		0, 8, // length = 8
		0, 0, // offset = 0
		0, 'T', 0, 'e', 0, 's', 0, 't',
	}

	type tblEntry struct {
		tag  [4]byte
		data []byte
	}
	tbls := []tblEntry{
		{[4]byte{'c', 'm', 'a', 'p'}, cmap},
		{[4]byte{'g', 'l', 'y', 'f'}, glyf},
		{[4]byte{'h', 'e', 'a', 'd'}, head},
		{[4]byte{'h', 'h', 'e', 'a'}, hhea},
		{[4]byte{'h', 'm', 't', 'x'}, hmtx},
		{[4]byte{'l', 'o', 'c', 'a'}, loca},
		{[4]byte{'m', 'a', 'x', 'p'}, maxp},
		{[4]byte{'n', 'a', 'm', 'e'}, nameTbl},
	}

	var sfnt []byte
	appendSfntU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		sfnt = append(sfnt, buf...)
	}
	appendSfntU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		sfnt = append(sfnt, buf...)
	}

	appendSfntU32(0x00010000) // TrueType signature
	appendSfntU16(uint16(len(tbls)))
	appendSfntU16(128)
	appendSfntU16(3)
	appendSfntU16(0)

	bodyOffset := 12 + len(tbls)*16
	currentOff := bodyOffset
	for _, t := range tbls {
		sfnt = append(sfnt, t.tag[:]...)
		appendSfntU32(0) // checksum
		appendSfntU32(uint32(currentOff))
		appendSfntU32(uint32(len(t.data)))
		currentOff += (len(t.data) + 3) &^ 3
	}

	for _, t := range tbls {
		sfnt = append(sfnt, t.data...)
		pad := (4 - (len(t.data) % 4)) % 4
		for p := 0; p < pad; p++ {
			sfnt = append(sfnt, 0)
		}
	}

	return sfnt
}

func TestMinimalSyntheticTTF(t *testing.T) {
	bytes := buildMinimalSyntheticTTF()
	font, err := Parse(bytes)
	if err != nil {
		t.Fatalf("Parse minimal synthetic TTF failed: %v", err)
	}

	if font.UnitsPerEm != 1000 {
		t.Errorf("UnitsPerEm: expected 1000, got %d", font.UnitsPerEm)
	}
	if font.Ascender != 800 {
		t.Errorf("Ascender: expected 800, got %d", font.Ascender)
	}
	if font.Descender != -200 {
		t.Errorf("Descender: expected -200, got %d", font.Descender)
	}
	if font.LineGap != 100 {
		t.Errorf("LineGap: expected 100, got %d", font.LineGap)
	}
	if font.FamilyName() != "Test" {
		t.Errorf("FamilyName: expected 'Test', got '%s'", font.FamilyName())
	}

	gidA := font.GlyphID('A')
	if gidA != 1 {
		t.Errorf("GlyphID('A'): expected 1, got %d", gidA)
	}
	gidi := font.GlyphID('i')
	if gidi != 2 {
		t.Errorf("GlyphID('i'): expected 2, got %d", gidi)
	}
	gidUnknown := font.GlyphID('Z')
	if gidUnknown != 0 {
		t.Errorf("GlyphID('Z'): expected 0 for unmapped character, got %d", gidUnknown)
	}

	mA := font.GlyphMetric(gidA)
	if mA.AdvanceWidth != 700 || mA.LeftSideBearing != 20 {
		t.Errorf("Metric for 'A': expected 700/20, got %d/%d", mA.AdvanceWidth, mA.LeftSideBearing)
	}

	advPx := font.GlyphAdvancePx('A', 20.0)
	expectedAdvPx := float32(700.0 * 20.0 / 1000.0) // 14.0 px
	if math.Abs(float64(advPx-expectedAdvPx)) > 1e-4 {
		t.Errorf("GlyphAdvancePx('A', 20.0): expected %f, got %f", expectedAdvPx, advPx)
	}

	totalWidth := font.MeasureText("AA", 20.0)
	if math.Abs(float64(totalWidth-expectedAdvPx*2)) > 1e-4 {
		t.Errorf("MeasureText('AA', 20.0): expected %f, got %f", expectedAdvPx*2, totalWidth)
	}

	lineHeight := font.LineHeightPx(20.0)
	expectedLH := float32((800 - (-200) + 100) * 20.0 / 1000.0) // 22.0 px
	if math.Abs(float64(lineHeight-expectedLH)) > 1e-4 {
		t.Errorf("LineHeightPx(20.0): expected %f, got %f", expectedLH, lineHeight)
	}

	outlineA := font.GlyphOutline(gidA)
	if outlineA == nil {
		t.Fatal("GlyphOutline(gidA) returned nil")
	}
	if len(outlineA.Contours) != 1 || len(outlineA.Contours[0]) != 4 {
		t.Fatalf("GlyphOutline contours: expected 1 contour of 4 points, got %d contours", len(outlineA.Contours))
	}
}

func TestRasterizeSyntheticGlyph(t *testing.T) {
	bytes := buildMinimalSyntheticTTF()
	font, err := Parse(bytes)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	bmp := font.RasterizeGlyph('A', 20.0)
	if bmp == nil {
		t.Fatal("RasterizeGlyph('A', 20.0) returned nil")
	}

	if bmp.Width == 0 || bmp.Height == 0 {
		t.Errorf("Bitmap dimensions zero: %dx%d", bmp.Width, bmp.Height)
	}

	solidPixels := 0
	for _, cov := range bmp.Coverage {
		if cov > 0 {
			solidPixels++
		}
	}
	if solidPixels == 0 {
		t.Errorf("Bitmap coverage is all zeros")
	}

	// Phased rasterization
	for phase := uint8(0); phase < 4; phase++ {
		phasedBmp := font.RasterizeGlyphSubpixelPositioned('A', 20.0, phase)
		if phasedBmp == nil {
			t.Errorf("RasterizeGlyphSubpixelPositioned failed for phase %d", phase)
		}
	}
}

func TestSubpixelLCDRasterization(t *testing.T) {
	bytes := buildMinimalSyntheticTTF()
	font, err := Parse(bytes)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	lcdBmp := font.RasterizeGlyphSubpixelLCD('A', 20.0)
	if lcdBmp == nil {
		t.Fatal("RasterizeGlyphSubpixelLCD returned nil")
	}

	if lcdBmp.Width == 0 || lcdBmp.Height == 0 {
		t.Errorf("LCD bitmap dimensions zero: %dx%d", lcdBmp.Width, lcdBmp.Height)
	}

	expectedBytes := int(lcdBmp.Width * lcdBmp.Height * 3)
	if len(lcdBmp.RgbCoverage) != expectedBytes {
		t.Errorf("RgbCoverage length expected %d, got %d", expectedBytes, len(lcdBmp.RgbCoverage))
	}

	nonZero := 0
	for _, b := range lcdBmp.RgbCoverage {
		if b > 0 {
			nonZero++
		}
	}
	if nonZero == 0 {
		t.Errorf("RgbCoverage has zero non-zero subpixels")
	}
}

func TestSyntheticBoldAndItalic(t *testing.T) {
	bytes := buildMinimalSyntheticTTF()
	font, err := Parse(bytes)
	if err != nil {
		t.Fatalf("Parse failed: %v", err)
	}

	normal := font.RasterizeGlyph('A', 20.0)
	bold := font.RasterizeGlyphStyled('A', 20.0, 0, true, false)
	italic := font.RasterizeGlyphStyled('A', 20.0, 0, false, true)

	if normal == nil || bold == nil || italic == nil {
		t.Fatal("Styled rasterization returned nil")
	}

	if bold.Width != normal.Width+1 {
		t.Errorf("Bold width expected %d, got %d", normal.Width+1, bold.Width)
	}
	if bold.AdvancePx <= normal.AdvancePx {
		t.Errorf("Bold advance expected > normal advance (%f <= %f)", bold.AdvancePx, normal.AdvancePx)
	}

	// Italic should produce a non-nil sheared bitmap
	if italic.Width == 0 || italic.Height == 0 {
		t.Errorf("Italic bitmap has zero dimensions: %dx%d", italic.Width, italic.Height)
	}
}

func TestFallbackMetrics(t *testing.T) {
	wEmpty := MeasureProportionalFallback("", 16.0)
	if wEmpty != 0 {
		t.Errorf("Expected 0 for empty string, got %f", wEmpty)
	}

	wHello := MeasureProportionalFallback("Hello World", 16.0)
	if wHello <= 0 {
		t.Errorf("Expected positive width for 'Hello World', got %f", wHello)
	}

	wNarrow := MeasureProportionalFallback("iii", 16.0)
	wWide := MeasureProportionalFallback("WWW", 16.0)
	if wNarrow >= wWide {
		t.Errorf("Expected 'WWW' to be wider than 'iii' (%f >= %f)", wNarrow, wWide)
	}
}
