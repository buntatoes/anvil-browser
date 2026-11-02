// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"math"
)

// Font represents a parsed TrueType or OpenType font.
type Font struct {
	// UnitsPerEm is head.unitsPerEm. Scaling design units to pixels divides by this value.
	UnitsPerEm uint16
	// Ascender is hhea.ascender in font design units (typically positive).
	Ascender int16
	// Descender is hhea.descender in font design units (typically negative).
	Descender int16
	// LineGap is hhea.lineGap in font design units.
	LineGap int16

	familyName string
	cmap       map[rune]uint16
	metrics    []HMetric
	kernPairs  map[[2]uint16]int16
	outlines   []*GlyphOutline
}

// Parse parses an SFNT font from raw binary bytes.
// Supports TrueType (0x00010000, 'true') and OpenType CFF ('OTTO') fonts.
func Parse(bytes []byte) (*Font, error) {
	tables, err := parseTableDirectory(bytes)
	if err != nil {
		return nil, err
	}

	headSlice, err := sliceTable(bytes, tables, "head")
	if err != nil {
		return nil, err
	}
	unitsPerEm, indexToLocFormat, err := parseHead(headSlice)
	if err != nil {
		return nil, err
	}

	hheaSlice, err := sliceTable(bytes, tables, "hhea")
	if err != nil {
		return nil, err
	}
	ascender, descender, lineGap, numHMetrics, err := parseHhea(hheaSlice)
	if err != nil {
		return nil, err
	}

	maxpSlice, err := sliceTable(bytes, tables, "maxp")
	if err != nil {
		return nil, err
	}
	numGlyphs, err := parseMaxp(maxpSlice)
	if err != nil {
		return nil, err
	}

	hmtxSlice, err := sliceTable(bytes, tables, "hmtx")
	if err != nil {
		return nil, err
	}
	metrics, err := parseHmtx(hmtxSlice, numHMetrics, numGlyphs)
	if err != nil {
		return nil, err
	}

	cmapSlice, err := sliceTable(bytes, tables, "cmap")
	if err != nil {
		return nil, err
	}
	cmap, err := parseCmap(cmapSlice)
	if err != nil {
		return nil, err
	}

	// Optional kern table
	kernPairs := make(map[[2]uint16]int16)
	if kernSlice, err := sliceTable(bytes, tables, "kern"); err == nil {
		if kp, err := parseKern(kernSlice); err == nil {
			kernPairs = kp
		}
	}
	// Optional GPOS table
	if gposSlice, err := sliceTable(bytes, tables, "GPOS"); err == nil {
		parseGpos(gposSlice, kernPairs)
	}

	// Optional name table
	familyName := ""
	if nameSlice, err := sliceTable(bytes, tables, "name"); err == nil {
		familyName = parseNameTable(nameSlice)
	}

	// Outlines: loca + glyf or CFF
	var outlines []*GlyphOutline
	locaSlice, locaErr := sliceTable(bytes, tables, "loca")
	glyfSlice, glyfErr := sliceTable(bytes, tables, "glyf")
	if locaErr == nil && glyfErr == nil {
		if decodedOutlines, err := parseGlyfTable(locaSlice, glyfSlice, numGlyphs, indexToLocFormat); err == nil {
			outlines = decodedOutlines
		}
	} else if cffSlice, err := sliceTable(bytes, tables, "CFF "); err == nil {
		if decodedOutlines, err := parseCffTable(cffSlice, numGlyphs); err == nil {
			outlines = decodedOutlines
		}
	}

	if outlines == nil {
		outlines = make([]*GlyphOutline, numGlyphs)
	}

	return &Font{
		UnitsPerEm: unitsPerEm,
		Ascender:   ascender,
		Descender:  descender,
		LineGap:    lineGap,
		familyName: familyName,
		cmap:       cmap,
		metrics:    metrics,
		kernPairs:  kernPairs,
		outlines:   outlines,
	}, nil
}

// FamilyName returns the font family name decoded from the name table.
func (f *Font) FamilyName() string {
	return f.familyName
}

// GlyphID looks up the glyph index for a Unicode rune. Returns 0 if unmapped.
func (f *Font) GlyphID(ch rune) uint16 {
	if gid, ok := f.cmap[ch]; ok {
		return gid
	}
	return 0
}

// GlyphMetric returns the horizontal metrics for a glyph ID.
func (f *Font) GlyphMetric(gid uint16) HMetric {
	idx := int(gid)
	if idx < len(f.metrics) {
		return f.metrics[idx]
	}
	if len(f.metrics) > 0 {
		return f.metrics[0]
	}
	return HMetric{AdvanceWidth: f.UnitsPerEm / 2, LeftSideBearing: 0}
}

// KerningUnits returns the design-unit kerning adjustment between two glyphs.
func (f *Font) KerningUnits(left, right uint16) int16 {
	if delta, ok := f.kernPairs[[2]uint16{left, right}]; ok {
		return delta
	}
	return 0
}

// GlyphAdvancePx computes the horizontal advance for character ch in pixels.
func (f *Font) GlyphAdvancePx(ch rune, fontSizePx float32) float32 {
	if fontSizePx <= 0 || f.UnitsPerEm == 0 {
		return 0
	}
	gid := f.GlyphID(ch)
	scale := fontSizePx / float32(f.UnitsPerEm)
	return float32(f.GlyphMetric(gid).AdvanceWidth) * scale
}

// MeasureText measures the total width of a string at fontSizePx, including pairwise kerning.
func (f *Font) MeasureText(text string, fontSizePx float32) float32 {
	if fontSizePx <= 0 || f.UnitsPerEm == 0 || len(text) == 0 {
		return 0
	}

	scale := fontSizePx / float32(f.UnitsPerEm)
	var totalPx float32
	var prevGid uint16
	first := true

	for _, ch := range text {
		gid := f.GlyphID(ch)
		if !first {
			kern := f.KerningUnits(prevGid, gid)
			totalPx += float32(kern) * scale
		}
		totalPx += float32(f.GlyphMetric(gid).AdvanceWidth) * scale
		prevGid = gid
		first = false
	}

	return totalPx
}

// LineHeightPx calculates the recommended line height at fontSizePx.
func (f *Font) LineHeightPx(fontSizePx float32) float32 {
	if fontSizePx <= 0 || f.UnitsPerEm == 0 {
		return 0
	}
	scale := fontSizePx / float32(f.UnitsPerEm)
	totalUnits := float32(f.Ascender - f.Descender + f.LineGap)
	return totalUnits * scale
}

// GlyphOutline returns the decoded outline for a glyph ID, or nil if empty.
func (f *Font) GlyphOutline(gid uint16) *GlyphOutline {
	idx := int(gid)
	if idx < len(f.outlines) {
		return f.outlines[idx]
	}
	return nil
}

// RasterizeGlyph rasterizes character ch at fontSizePx into an 8-bit coverage bitmap.
func (f *Font) RasterizeGlyph(ch rune, fontSizePx float32) *GlyphBitmap {
	return f.RasterizeGlyphSubpixelPositioned(ch, fontSizePx, 0)
}

// RasterizeGlyphSubpixelPositioned rasterizes ch at fontSizePx with horizontal subpixel phase (0..3).
func (f *Font) RasterizeGlyphSubpixelPositioned(ch rune, fontSizePx float32, subpixelXPhase uint8) *GlyphBitmap {
	if math.IsNaN(float64(fontSizePx)) || math.IsInf(float64(fontSizePx), 0) || fontSizePx <= 0 || f.UnitsPerEm == 0 {
		return nil
	}
	gid := f.GlyphID(ch)
	outline := f.GlyphOutline(gid)
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	scale := fontSizePx / float32(f.UnitsPerEm)
	advancePx := float32(f.GlyphMetric(gid).AdvanceWidth) * scale
	phaseOffset := float32(subpixelXPhase&3) * 0.25
	return rasterizeOutlineWithPhase(outline, scale, advancePx, phaseOffset, true)
}

// RasterizeGlyphSubpixelLCD rasterizes ch at fontSizePx into a 3x horizontal RGB subpixel LCD bitmap.
func (f *Font) RasterizeGlyphSubpixelLCD(ch rune, fontSizePx float32) *SubpixelGlyphBitmap {
	if math.IsNaN(float64(fontSizePx)) || math.IsInf(float64(fontSizePx), 0) || fontSizePx <= 0 || f.UnitsPerEm == 0 {
		return nil
	}
	gid := f.GlyphID(ch)
	outline := f.GlyphOutline(gid)
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	scale := fontSizePx / float32(f.UnitsPerEm)
	advancePx := float32(f.GlyphMetric(gid).AdvanceWidth) * scale
	return rasterizeOutlineSubpixelLCD(outline, scale, advancePx)
}

// RasterizeGlyphStyled rasterizes ch with optional synthetic bold dilation and synthetic italic oblique shear.
func (f *Font) RasterizeGlyphStyled(
	ch rune,
	fontSizePx float32,
	subpixelXPhase uint8,
	syntheticBold bool,
	syntheticItalic bool,
) *GlyphBitmap {
	if math.IsNaN(float64(fontSizePx)) || math.IsInf(float64(fontSizePx), 0) || fontSizePx <= 0 || f.UnitsPerEm == 0 {
		return nil
	}
	gid := f.GlyphID(ch)
	outline := f.GlyphOutline(gid)
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	scale := fontSizePx / float32(f.UnitsPerEm)
	advancePx := float32(f.GlyphMetric(gid).AdvanceWidth) * scale
	return rasterizeGlyphStyled(outline, scale, advancePx, subpixelXPhase, syntheticBold, syntheticItalic, fontSizePx)
}
