// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"math"
	"sort"
)

func pixelBounds(minX, minY, maxX, maxY float32) (left, top int32, width, height uint32, ok bool) {
	if math.IsNaN(float64(minX)) || math.IsNaN(float64(minY)) || math.IsNaN(float64(maxX)) || math.IsNaN(float64(maxY)) ||
		math.IsInf(float64(minX), 0) || math.IsInf(float64(minY), 0) || math.IsInf(float64(maxX), 0) || math.IsInf(float64(maxY), 0) {
		return 0, 0, 0, 0, false
	}

	fLeft := float32(math.Floor(float64(minX)))
	fTop := float32(math.Floor(float64(minY)))
	fRight := float32(math.Ceil(float64(maxX)))
	fBottom := float32(math.Ceil(float64(maxY)))

	w := fRight - fLeft
	h := fBottom - fTop

	if w > MaxGlyphBitmapSide || h > MaxGlyphBitmapSide || w < 0 || h < 0 {
		return 0, 0, 0, 0, false
	}
	if float32(math.Abs(float64(fLeft))) > MaxGlyphOffset || float32(math.Abs(float64(fTop))) > MaxGlyphOffset {
		return 0, 0, 0, 0, false
	}

	uW := uint32(w)
	if uW < 1 {
		uW = 1
	}
	uH := uint32(h)
	if uH < 1 {
		uH = 1
	}

	return int32(fLeft), int32(fTop), uW, uH, true
}

type segment struct {
	a [2]float32
	b [2]float32
}

type crossing struct {
	x   float32
	dir int32
}

// rasterizeOutlineWithPhase fills outline with nonzero winding and 4x4 coverage samples.
func rasterizeOutlineWithPhase(
	outline *GlyphOutline,
	scale float32,
	advancePx float32,
	xOffsetPx float32,
	gridFit bool,
) *GlyphBitmap {
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	var segments []segment
	minX, minY := float32(math.Inf(1)), float32(math.Inf(1))
	maxX, maxY := float32(math.Inf(-1)), float32(math.Inf(-1))

	for _, contour := range outline.Contours {
		poly := flattenContourOpts(contour, scale, xOffsetPx, gridFit)
		if len(poly) < 2 {
			continue
		}
		for i := 0; i < len(poly); i++ {
			a := poly[i]
			b := poly[(i+1)%len(poly)]
			if a[0] < minX {
				minX = a[0]
			}
			if b[0] < minX {
				minX = b[0]
			}
			if a[1] < minY {
				minY = a[1]
			}
			if b[1] < minY {
				minY = b[1]
			}
			if a[0] > maxX {
				maxX = a[0]
			}
			if b[0] > maxX {
				maxX = b[0]
			}
			if a[1] > maxY {
				maxY = a[1]
			}
			if b[1] > maxY {
				maxY = b[1]
			}

			// Drop near-horizontal edges
			dy := a[1] - b[1]
			if dy < 0 {
				dy = -dy
			}
			if dy > 1e-5 {
				segments = append(segments, segment{a: a, b: b})
			}
		}
	}

	if len(segments) == 0 {
		return nil
	}

	x0, y0, width, height, ok := pixelBounds(minX, minY, maxX, maxY)
	if !ok {
		return nil
	}

	coverage := make([]byte, width*height)
	const sub = 4

	for py := uint32(0); py < height; py++ {
		rowHits := make([]uint8, width)
		for sy := 0; sy < sub; sy++ {
			sampleY := float32(y0) + float32(py) + (float32(sy)+0.5)/float32(sub)
			var crossings []crossing

			for _, seg := range segments {
				var ymin, ymax float32
				var dir int32
				if seg.a[1] < seg.b[1] {
					ymin, ymax, dir = seg.a[1], seg.b[1], 1
				} else {
					ymin, ymax, dir = seg.b[1], seg.a[1], -1
				}

				if sampleY >= ymin && sampleY < ymax {
					t := (sampleY - seg.a[1]) / (seg.b[1] - seg.a[1])
					ix := seg.a[0] + t*(seg.b[0]-seg.a[0])
					crossings = append(crossings, crossing{x: ix, dir: dir})
				}
			}

			if len(crossings) == 0 {
				continue
			}

			sort.Slice(crossings, func(i, j int) bool {
				return crossings[i].x < crossings[j].x
			})

			for px := uint32(0); px < width; px++ {
				for sx := 0; sx < sub; sx++ {
					sampleX := float32(x0) + float32(px) + (float32(sx)+0.5)/float32(sub)
					winding := int32(0)
					for _, c := range crossings {
						if c.x <= sampleX {
							winding += c.dir
						} else {
							break
						}
					}
					if winding != 0 {
						rowHits[px]++
					}
				}
			}
		}

		for px := uint32(0); px < width; px++ {
			hits := uint32(rowHits[px])
			cov := (hits*255 + 8) / 16
			if cov > 255 {
				cov = 255
			}
			coverage[py*width+px] = byte(cov)
		}
	}

	return &GlyphBitmap{
		Width:     width,
		Height:    height,
		BearingX:  x0,
		BearingY:  -y0,
		AdvancePx: advancePx,
		Coverage:  coverage,
	}
}

// rasterizeOutlineSubpixelLCD rasterizes an outline into a 3x horizontal RGB subpixel LCD bitmap.
func rasterizeOutlineSubpixelLCD(
	outline *GlyphOutline,
	scale float32,
	advancePx float32,
) *SubpixelGlyphBitmap {
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	xOffsetPx := float32(0.0)
	gridFit := true
	var segments []segment
	minX, minY := float32(math.Inf(1)), float32(math.Inf(1))
	maxX, maxY := float32(math.Inf(-1)), float32(math.Inf(-1))

	for _, contour := range outline.Contours {
		poly := flattenContourOpts(contour, scale, xOffsetPx, gridFit)
		if len(poly) < 2 {
			continue
		}
		for i := 0; i < len(poly); i++ {
			a := poly[i]
			b := poly[(i+1)%len(poly)]
			if a[0] < minX {
				minX = a[0]
			}
			if b[0] < minX {
				minX = b[0]
			}
			if a[1] < minY {
				minY = a[1]
			}
			if b[1] < minY {
				minY = b[1]
			}
			if a[0] > maxX {
				maxX = a[0]
			}
			if b[0] > maxX {
				maxX = b[0]
			}
			if a[1] > maxY {
				maxY = a[1]
			}
			if b[1] > maxY {
				maxY = b[1]
			}

			dy := a[1] - b[1]
			if dy < 0 {
				dy = -dy
			}
			if dy > 1e-5 {
				segments = append(segments, segment{a: a, b: b})
			}
		}
	}

	if len(segments) == 0 {
		return nil
	}

	x0, y0, width, height, ok := pixelBounds(minX, minY, maxX, maxY)
	if !ok {
		return nil
	}

	subCols := int(width) * 3
	rgbCoverage := make([]byte, subCols*int(height))
	const subXPerChannel = 4
	const subY = 4

	for py := uint32(0); py < height; py++ {
		rawSub := make([]uint8, subCols)
		for sy := 0; sy < subY; sy++ {
			sampleY := float32(y0) + float32(py) + (float32(sy)+0.5)/float32(subY)
			var crossings []crossing

			for _, seg := range segments {
				var ymin, ymax float32
				var dir int32
				if seg.a[1] < seg.b[1] {
					ymin, ymax, dir = seg.a[1], seg.b[1], 1
				} else {
					ymin, ymax, dir = seg.b[1], seg.a[1], -1
				}

				if sampleY >= ymin && sampleY < ymax {
					t := (sampleY - seg.a[1]) / (seg.b[1] - seg.a[1])
					ix := seg.a[0] + t*(seg.b[0]-seg.a[0])
					crossings = append(crossings, crossing{x: ix, dir: dir})
				}
			}

			if len(crossings) == 0 {
				continue
			}

			sort.Slice(crossings, func(i, j int) bool {
				return crossings[i].x < crossings[j].x
			})

			for col := 0; col < subCols; col++ {
				for sx := 0; sx < subXPerChannel; sx++ {
					subIdx := col*subXPerChannel + sx
					sampleX := float32(x0) + (float32(subIdx)+0.5)/float32(3*subXPerChannel)
					winding := int32(0)
					for _, c := range crossings {
						if c.x <= sampleX {
							winding += c.dir
						} else {
							break
						}
					}
					if winding != 0 {
						rawSub[col]++
					}
				}
			}
		}

		unfiltered := make([]uint32, subCols)
		for col := 0; col < subCols; col++ {
			hits := uint32(rawSub[col])
			cov := (hits*255 + 8) / 16
			if cov > 255 {
				cov = 255
			}
			unfiltered[col] = cov
		}

		// 3-tap [1, 2, 1] / 4 horizontal FIR filter across subpixel columns.
		rowBase := int(py) * subCols
		for col := 0; col < subCols; col++ {
			left := unfiltered[col]
			if col > 0 {
				left = unfiltered[col-1]
			}
			center := unfiltered[col]
			right := unfiltered[col]
			if col+1 < subCols {
				right = unfiltered[col+1]
			}
			filtered := (left + center*2 + right + 2) / 4
			if filtered > 255 {
				filtered = 255
			}
			rgbCoverage[rowBase+col] = byte(filtered)
		}
	}

	return &SubpixelGlyphBitmap{
		Width:       width,
		Height:      height,
		BearingX:    x0,
		BearingY:    -y0,
		AdvancePx:   advancePx,
		RgbCoverage: rgbCoverage,
	}
}

// rasterizeGlyphStyled rasterizes a glyph with horizontal subpixel phase, synthetic bold dilation,
// and synthetic italic oblique shear.
func rasterizeGlyphStyled(
	outline *GlyphOutline,
	scale float32,
	advancePx float32,
	subpixelXPhase uint8,
	syntheticBold bool,
	syntheticItalic bool,
	fontSizePx float32,
) *GlyphBitmap {
	if outline == nil || len(outline.Contours) == 0 {
		return nil
	}

	effectiveOutline := outline
	if syntheticItalic {
		shearedContours := make([][]OutlinePoint, len(outline.Contours))
		for ci, contour := range outline.Contours {
			sheared := make([]OutlinePoint, len(contour))
			for pi, pt := range contour {
				slant := int16(math.Round(float64(pt.Y) * 0.20))
				sheared[pi] = OutlinePoint{
					X:       pt.X + slant,
					Y:       pt.Y,
					OnCurve: pt.OnCurve,
				}
			}
			shearedContours[ci] = sheared
		}
		effectiveOutline = &GlyphOutline{
			XMin:     outline.XMin,
			YMin:     outline.YMin,
			XMax:     outline.XMax,
			YMax:     outline.YMax,
			Contours: shearedContours,
		}
	}

	phaseOffset := float32(subpixelXPhase&3) * 0.25
	bmp := rasterizeOutlineWithPhase(effectiveOutline, scale, advancePx, phaseOffset, true)
	if bmp == nil {
		return nil
	}

	if syntheticBold && bmp.Width > 0 && bmp.Height > 0 && bmp.Width < 511 {
		oldW := int(bmp.Width)
		newW := oldW + 1
		h := int(bmp.Height)
		boldCov := make([]byte, newW*h)

		for row := 0; row < h; row++ {
			for col := 0; col < newW; col++ {
				c0 := byte(0)
				if col < oldW {
					c0 = bmp.Coverage[row*oldW+col]
				}
				c1 := byte(0)
				if col > 0 {
					c1 = byte((uint16(bmp.Coverage[row*oldW+(col-1)]) * 210) / 255)
				}
				if c1 > c0 {
					boldCov[row*newW+col] = c1
				} else {
					boldCov[row*newW+col] = c0
				}
			}
		}

		bmp.Width = uint32(newW)
		extraAdv := fontSizePx * 0.04
		if extraAdv < 0.5 {
			extraAdv = 0.5
		}
		bmp.AdvancePx += extraAdv
		bmp.Coverage = boldCov
	}

	return bmp
}
