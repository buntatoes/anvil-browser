// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

// Security and resource limits.
const (
	// MaxFontBytes is the maximum font binary size accepted (320 MiB).
	MaxFontBytes = 320 * 1024 * 1024

	// MaxSfntTables is the maximum number of SFNT directory tables accepted (1,280).
	MaxSfntTables = 1280

	// MaxGlyphContours is the maximum contours per glyph outline (2,560).
	MaxGlyphContours = 2560

	// MaxContourPoints is the maximum control points per glyph outline (40,960).
	MaxContourPoints = 40960

	// MaxCompositeGlyphDepth is the maximum compound glyph recursion depth (64).
	MaxCompositeGlyphDepth = 64

	// MaxGlyphBitmapSide is the maximum width or height of a rasterized glyph bitmap in pixels (512.0).
	MaxGlyphBitmapSide = float32(512.0)

	// MaxGlyphOffset is the maximum pixel distance of a glyph bitmap's corner from the pen (16,777,216.0).
	MaxGlyphOffset = float32(16777216.0)
)

// HMetric represents the horizontal metric for one glyph in font design units.
type HMetric struct {
	// AdvanceWidth is the distance the pen advances horizontally before kerning.
	AdvanceWidth uint16
	// LeftSideBearing is the horizontal distance from the glyph origin to its left edge.
	LeftSideBearing int16
}

// FontFamilySlot identifies standard generic font family and style slots loaded by the browser.
type FontFamilySlot int

const (
	// SansRegular is proportional sans-serif regular (Segoe UI / DejaVu Sans).
	SansRegular FontFamilySlot = iota
	// SansBold is proportional sans-serif bold.
	SansBold
	// SansItalic is proportional sans-serif italic/oblique.
	SansItalic
	// SerifRegular is proportional serif regular (Georgia / Times New Roman).
	SerifRegular
	// SerifBold is proportional serif bold.
	SerifBold
	// MonospaceRegular is fixed-width monospace regular (Consolas / DejaVu Sans Mono).
	MonospaceRegular
	// MonospaceBold is fixed-width monospace bold.
	MonospaceBold
)

// CurveOrder represents the curve classification of an outline vertex.
type CurveOrder int

const (
	// CurveOrderOnCurve is an on-curve vertex.
	CurveOrderOnCurve CurveOrder = iota
	// CurveOrderQuadraticOffCurve is an off-curve quadratic control point (glyf).
	CurveOrderQuadraticOffCurve
	// CurveOrderCubicOffCurve is an off-curve cubic control point (CFF).
	CurveOrderCubicOffCurve
)

// OutlinePoint represents one point on a glyph contour in font design units.
type OutlinePoint struct {
	// X in font units (positive is right of the glyph origin).
	X int16
	// Y in font units (positive is above the baseline).
	Y int16
	// OnCurve is true if this point lies directly on the outline path.
	OnCurve bool
}

// CubicPoint represents a floating-point vertex during CFF CharString flattening.
type CubicPoint struct {
	X     float32
	Y     float32
	Order CurveOrder
}

// GlyphOutline represents a decoded glyph outline in font design units.
type GlyphOutline struct {
	// XMin is the minimum X coordinate from the glyph header.
	XMin int16
	// YMin is the minimum Y coordinate from the glyph header.
	YMin int16
	// XMax is the maximum X coordinate from the glyph header.
	XMax int16
	// YMax is the maximum Y coordinate from the glyph header.
	YMax int16
	// Contours contains the closed loops forming the glyph outline.
	Contours [][]OutlinePoint
}

// GlyphBitmap represents an 8-bit grayscale coverage bitmap for a rasterized glyph.
type GlyphBitmap struct {
	// Width of the bitmap in pixels.
	Width uint32
	// Height of the bitmap in pixels.
	Height uint32
	// BearingX is the left edge relative to the pen in pixels.
	BearingX int32
	// BearingY is the distance from the baseline up to row 0 in pixels.
	BearingY int32
	// AdvancePx is the horizontal advance distance in pixels (excluding kerning).
	AdvancePx float32
	// Coverage holds row-major 8-bit coverage values (0 = empty, 255 = fully covered).
	Coverage []byte
}

// SubpixelGlyphBitmap represents a 3x horizontal RGB subpixel LCD coverage bitmap.
type SubpixelGlyphBitmap struct {
	// Width of the bitmap in whole pixels.
	Width uint32
	// Height of the bitmap in pixels.
	Height uint32
	// BearingX is the left edge relative to the pen in pixels.
	BearingX int32
	// BearingY is the distance from the baseline up to row 0 in pixels.
	BearingY int32
	// AdvancePx is the horizontal advance distance in pixels.
	AdvancePx float32
	// RgbCoverage holds row-major RGB coverage bytes (Width * Height * 3).
	RgbCoverage []byte
}
