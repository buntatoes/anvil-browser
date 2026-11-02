// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
	"testing"
)

func TestCurvesFlattening(t *testing.T) {
	// 1. Quadratic Bézier
	var quadPoly [][2]float32
	p0 := [2]float32{0, 0}
	c := [2]float32{50, 100}
	p2 := [2]float32{100, 0}
	FlattenQuadBezier(p0, c, p2, 6, &quadPoly)

	if len(quadPoly) != 6 {
		t.Fatalf("FlattenQuadBezier expected 6 vertices, got %d", len(quadPoly))
	}
	if quadPoly[0] != p0 {
		t.Errorf("FlattenQuadBezier[0] expected %v, got %v", p0, quadPoly[0])
	}

	// 2. Cubic Bézier
	var cubicPoly [][2]float32
	c1 := [2]float32{30, 90}
	c2 := [2]float32{70, 90}
	p3 := [2]float32{100, 0}
	FlattenCubicBezier(p0, c1, c2, p3, 8, &cubicPoly)

	if len(cubicPoly) != 8 {
		t.Fatalf("FlattenCubicBezier expected 8 vertices, got %d", len(cubicPoly))
	}
	if cubicPoly[0] != p0 {
		t.Errorf("FlattenCubicBezier[0] expected %v, got %v", p0, cubicPoly[0])
	}

	// 3. Mixed Cubic Contour
	pts := []CubicPoint{
		{X: 0, Y: 0, Order: CurveOrderOnCurve},
		{X: 30, Y: 90, Order: CurveOrderCubicOffCurve},
		{X: 70, Y: 90, Order: CurveOrderCubicOffCurve},
		{X: 100, Y: 0, Order: CurveOrderOnCurve},
		{X: 50, Y: -50, Order: CurveOrderQuadraticOffCurve},
	}
	mixedPoly := FlattenCubicContour(pts)
	if len(mixedPoly) < 10 {
		t.Errorf("FlattenCubicContour expected >= 10 vertices for mixed contour, got %d", len(mixedPoly))
	}
}

func TestGammaCorrectLUTs(t *testing.T) {
	if lin0 := SRGBToLinearU12(0); lin0 != 0 {
		t.Errorf("SRGBToLinearU12(0) expected 0, got %d", lin0)
	}
	if lin255 := SRGBToLinearU12(255); lin255 != 4095 {
		t.Errorf("SRGBToLinearU12(255) expected 4095, got %d", lin255)
	}

	if s0 := LinearU12ToSRGB(0); s0 != 0 {
		t.Errorf("LinearU12ToSRGB(0) expected 0, got %d", s0)
	}
	if s4095 := LinearU12ToSRGB(4095); s4095 != 255 {
		t.Errorf("LinearU12ToSRGB(4095) expected 255, got %d", s4095)
	}

	// Test monotonic round-trip
	for i := 0; i < 256; i++ {
		lin := SRGBToLinearU12(byte(i))
		s := int(LinearU12ToSRGB(lin))
		diff := s - i
		if diff < 0 {
			diff = -diff
		}
		if diff > 1 {
			t.Errorf("Round-trip error at %d: lin=%d s=%d diff=%d", i, lin, s, diff)
		}
	}
}

func TestStemContrastCurve(t *testing.T) {
	if c0 := EnhanceStemContrast(0, 0); c0 != 0 {
		t.Errorf("EnhanceStemContrast(0, 0) expected 0, got %d", c0)
	}
	if c255 := EnhanceStemContrast(255, 0); c255 != 255 {
		t.Errorf("EnhanceStemContrast(255, 0) expected 255, got %d", c255)
	}

	// Midtone (128) on dark text (fgLuma = 0) should be boosted
	cMidDark := EnhanceStemContrast(128, 0)
	if cMidDark <= 128 {
		t.Errorf("EnhanceStemContrast(128, 0) expected > 128, got %d", cMidDark)
	}

	// Midtone (128) on light text (fgLuma = 255) should have smaller boost
	cMidLight := EnhanceStemContrast(128, 255)
	if cMidDark <= cMidLight {
		t.Errorf("Dark text boost should be higher than light text boost (%d <= %d)", cMidDark, cMidLight)
	}
}

func TestLinearAlphaBlending(t *testing.T) {
	dst := [3]byte{255, 255, 255} // White
	fg := [3]byte{0, 0, 0}        // Black

	// Coverage 0 -> untouched destination
	out0 := BlendGlyphPixelGammaCorrect(dst, fg, 0)
	if out0 != dst {
		t.Errorf("Blend with cov=0 expected %v, got %v", dst, out0)
	}

	// Coverage 255 -> full foreground
	out255 := BlendGlyphPixelGammaCorrect(dst, fg, 255)
	if out255 != fg {
		t.Errorf("Blend with cov=255 expected %v, got %v", fg, out255)
	}

	// Coverage 128 -> darkened gray
	out128 := BlendGlyphPixelGammaCorrect(dst, fg, 128)
	if out128[0] >= 255 || out128[1] >= 255 || out128[2] >= 255 {
		t.Errorf("Blend with cov=128 should produce darker gray, got %v", out128)
	}

	// LCD subpixel blend
	covLcd := [3]byte{255, 128, 0}
	outLcd := BlendSubpixelLCDGammaCorrect(dst, fg, covLcd)
	if outLcd[0] != 0 {
		t.Errorf("Red channel expected 0 for cov=255, got %d", outLcd[0])
	}
	if outLcd[2] != 255 {
		t.Errorf("Blue channel expected 255 for cov=0, got %d", outLcd[2])
	}
}

func TestCFFType2Charstrings(t *testing.T) {
	// Program for a 100x100 square:
	// 50 hmoveto (dx=50)
	// 100 100 rlineto (dx=100, dy=100)
	// -100 hlineto (dx=-100)
	// endchar
	prog := []byte{
		byte(50 + 139), // 50
		22,             // hmoveto
		byte(100 + 139),
		byte(100 + 139), // 100 100
		5,               // rlineto
		byte(139 - 100), // -100
		6,               // hlineto
		14,              // endchar
	}

	outline, err := InterpretType2CharString(prog)
	if err != nil || outline == nil {
		t.Fatalf("InterpretType2CharString failed: %v", err)
	}

	if len(outline.Contours) != 1 {
		t.Fatalf("Expected 1 contour, got %d", len(outline.Contours))
	}
	if outline.XMin > outline.XMax || outline.YMin > outline.YMax {
		t.Errorf("Invalid outline bounds: [%d, %d, %d, %d]", outline.XMin, outline.YMin, outline.XMax, outline.YMax)
	}
}

func TestCFFSubroutines(t *testing.T) {
	// Local subr 0: 50 50 rlineto, return
	localSub0 := []byte{
		byte(50 + 139),
		byte(50 + 139),
		5,  // rlineto
		11, // return
	}

	// Global subr 0: -50 -50 rlineto, return
	globalSub0 := []byte{
		byte(139 - 50),
		byte(139 - 50),
		5,  // rlineto
		11, // return
	}

	// Main program:
	// 0 0 rmoveto
	// callsubr (index 0 with bias 107 -> operand 0 - 107 = -107)
	// callgsubr (index 0 with bias 107 -> operand 0 - 107 = -107)
	// endchar
	// To push -107, 28 byte followed by int16(-107) = 0xFF95
	var mainProg []byte
	mainProg = append(mainProg, 139, 139, 21) // 0 0 rmoveto
	// callsubr with operand -107
	mainProg = append(mainProg, 28, 0xFF, 0x95, 10)
	// callgsubr with operand -107
	mainProg = append(mainProg, 28, 0xFF, 0x95, 29)
	mainProg = append(mainProg, 14) // endchar

	outline, err := InterpretType2CharStringWithSubrs(mainProg, [][]byte{localSub0}, [][]byte{globalSub0})
	if err != nil || outline == nil {
		t.Fatalf("InterpretType2CharStringWithSubrs failed: %v", err)
	}

	if len(outline.Contours) != 1 {
		t.Fatalf("Expected 1 contour after subroutines, got %d", len(outline.Contours))
	}
}

func TestGPOSPairPos(t *testing.T) {
	// Synthetic GPOS table with PairPosFormat1
	// Header (10 bytes): major=1, minor=0, scriptList=10, featureList=10, lookupList=10
	var gpos []byte
	appendU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		gpos = append(gpos, buf...)
	}

	appendU16(1)  // major
	appendU16(0)  // minor
	appendU16(10) // scriptListOff
	appendU16(10) // featureListOff
	appendU16(10) // lookupListOff = 10

	// LookupList (starts at 10)
	_ = len(gpos) // llBase
	appendU16(1)  // lookupCount = 1
	appendU16(4)  // lookup[0] offset relative to llBase = 4

	// Lookup table (starts at llBase + 4 = 14)
	_ = len(gpos) // lBase
	appendU16(2)  // lookupType = 2 (PairPos)
	appendU16(0)  // lookupFlag = 0
	appendU16(1)  // subTableCount = 1
	appendU16(8)  // subTable[0] offset relative to lBase = 8

	// PairPosFormat1 subtable (starts at lBase + 6 = 20)
	_ = len(gpos)     // subBase
	appendU16(1)      // posFormat = 1
	appendU16(12)     // coverageOffset relative to subBase = 12
	appendU16(0x0004) // valueFormat1 = XAdvance (2 bytes)
	appendU16(0x0000) // valueFormat2 = 0
	appendU16(1)      // pairSetCount = 1
	appendU16(18)     // pairSet[0] offset relative to subBase = 18

	// Coverage table format 1 (starts at subBase + 12 = 32)
	appendU16(1) // format = 1
	appendU16(1) // count = 1
	appendU16(1) // glyph[0] = 1 (leftGID = 1)

	// PairSet table (starts at subBase + 18 = 38)
	appendU16(1)      // pairValueCount = 1
	appendU16(2)      // secondGlyph = 2 (rightGID = 2)
	appendU16(0xFFCE) // xAdvance = -50

	pairs := make(map[[2]uint16]int16)
	parseGpos(gpos, pairs)

	val, ok := pairs[[2]uint16{1, 2}]
	if !ok {
		t.Fatalf("GPOS pair (1, 2) not found")
	}
	if val != -50 {
		t.Errorf("GPOS pair (1, 2) expected -50, got %d", val)
	}
}
