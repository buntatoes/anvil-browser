// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import "math"

// parseGlyfTable decodes every glyph outline addressed by loca.
func parseGlyfTable(loca, glyf []byte, numGlyphs int, indexToLocFormat int16) ([]*GlyphOutline, error) {
	offsets := make([]int, numGlyphs+1)
	if indexToLocFormat == 0 {
		// Short format: u16 word offset (byte offset = word offset * 2)
		if len(loca) < (numGlyphs+1)*2 {
			return nil, errInvalidTable("loca")
		}
		for i := 0; i <= numGlyphs; i++ {
			wordOff, err := readU16BE(loca, i*2)
			if err != nil {
				return nil, err
			}
			offsets[i] = int(wordOff) * 2
		}
	} else {
		// Long format: u32 byte offset
		if len(loca) < (numGlyphs+1)*4 {
			return nil, errInvalidTable("loca")
		}
		for i := 0; i <= numGlyphs; i++ {
			byteOff, err := readU32BE(loca, i*4)
			if err != nil {
				return nil, err
			}
			offsets[i] = int(byteOff)
		}
	}

	outlines := make([]*GlyphOutline, numGlyphs)
	for gid := 0; gid < numGlyphs; gid++ {
		start := offsets[gid]
		end := offsets[gid+1]
		if start >= end || end > len(glyf) {
			outlines[gid] = nil
			continue
		}
		slice := glyf[start:end]
		outline, err := parseSingleGlyph(slice, glyf, offsets, 0)
		if err == nil {
			outlines[gid] = outline
		}
	}

	return outlines, nil
}

// parseSingleGlyph decodes a single TrueType glyph record (simple or compound).
func parseSingleGlyph(slice, glyf []byte, offsets []int, depth int) (*GlyphOutline, error) {
	if depth > MaxCompositeGlyphDepth || len(slice) < 10 {
		return nil, nil
	}

	numContours, err := readI16BE(slice, 0)
	if err != nil {
		return nil, err
	}
	xMin, err := readI16BE(slice, 2)
	if err != nil {
		return nil, err
	}
	yMin, err := readI16BE(slice, 4)
	if err != nil {
		return nil, err
	}
	xMax, err := readI16BE(slice, 6)
	if err != nil {
		return nil, err
	}
	yMax, err := readI16BE(slice, 8)
	if err != nil {
		return nil, err
	}

	if numContours == 0 {
		return nil, nil
	}

	if numContours > 0 {
		if int(numContours) > MaxGlyphContours {
			return nil, errInvalidTable("glyf")
		}
		contours, err := parseSimpleContours(slice[10:], int(numContours))
		if err != nil {
			return nil, err
		}
		return &GlyphOutline{
			XMin:     xMin,
			YMin:     yMin,
			XMax:     xMax,
			YMax:     yMax,
			Contours: contours,
		}, nil
	}

	// Compound glyph
	contours, err := parseCompoundContours(slice[10:], glyf, offsets, depth+1)
	if err != nil {
		return nil, err
	}
	return &GlyphOutline{
		XMin:     xMin,
		YMin:     yMin,
		XMax:     xMax,
		YMax:     yMax,
		Contours: contours,
	}, nil
}

func parseSimpleContours(data []byte, numContours int) ([][]OutlinePoint, error) {
	if numContours > MaxGlyphContours || len(data) < numContours*2+2 {
		return nil, errInvalidTable("glyf")
	}

	endPts := make([]int, numContours)
	for i := 0; i < numContours; i++ {
		val, err := readU16BE(data, i*2)
		if err != nil {
			return nil, err
		}
		endPts[i] = int(val)
	}

	totalPts := 0
	if len(endPts) > 0 {
		totalPts = endPts[len(endPts)-1] + 1
	}
	if totalPts == 0 {
		return nil, nil
	}
	if totalPts > MaxContourPoints {
		return nil, errInvalidTable("glyf")
	}

	insnLenRaw, err := readU16BE(data, numContours*2)
	if err != nil {
		return nil, err
	}
	pos := numContours*2 + 2 + int(insnLenRaw)
	if pos > len(data) {
		return nil, errInvalidTable("glyf")
	}

	flags := make([]byte, 0, totalPts)
	for len(flags) < totalPts {
		if pos >= len(data) {
			return nil, errUnexpectedEOF()
		}
		f := data[pos]
		pos++
		flags = append(flags, f)
		if (f & 0x08) != 0 {
			if pos >= len(data) {
				return nil, errUnexpectedEOF()
			}
			repeat := int(data[pos])
			pos++
			for r := 0; r < repeat && len(flags) < totalPts; r++ {
				flags = append(flags, f)
			}
		}
	}

	xs := make([]int16, totalPts)
	curX := int16(0)
	for i, f := range flags {
		short := (f & 0x02) != 0
		sameOrPos := (f & 0x10) != 0
		var dx int16
		if short {
			if pos >= len(data) {
				return nil, errUnexpectedEOF()
			}
			b := int16(data[pos])
			pos++
			if sameOrPos {
				dx = b
			} else {
				dx = -b
			}
		} else if sameOrPos {
			dx = 0
		} else {
			val, err := readI16BE(data, pos)
			if err != nil {
				return nil, err
			}
			pos += 2
			dx = val
		}
		curX += dx
		xs[i] = curX
	}

	ys := make([]int16, totalPts)
	curY := int16(0)
	for i, f := range flags {
		short := (f & 0x04) != 0
		sameOrPos := (f & 0x20) != 0
		var dy int16
		if short {
			if pos >= len(data) {
				return nil, errUnexpectedEOF()
			}
			b := int16(data[pos])
			pos++
			if sameOrPos {
				dy = b
			} else {
				dy = -b
			}
		} else if sameOrPos {
			dy = 0
		} else {
			val, err := readI16BE(data, pos)
			if err != nil {
				return nil, err
			}
			pos += 2
			dy = val
		}
		curY += dy
		ys[i] = curY
	}

	contours := make([][]OutlinePoint, 0, numContours)
	start := 0
	for _, end := range endPts {
		if end >= totalPts || end < start {
			break
		}
		count := end - start + 1
		contour := make([]OutlinePoint, count)
		for idx := start; idx <= end; idx++ {
			contour[idx-start] = OutlinePoint{
				X:       xs[idx],
				Y:       ys[idx],
				OnCurve: (flags[idx] & 0x01) != 0,
			}
		}
		contours = append(contours, contour)
		start = end + 1
	}

	return contours, nil
}

// parseCompoundContours decodes composite glyph components, applying 2x2 matrix transforms and XY offsets.
func parseCompoundContours(data, glyf []byte, offsets []int, depth int) ([][]OutlinePoint, error) {
	pos := 0
	var allContours [][]OutlinePoint

	for {
		if pos+4 > len(data) {
			break
		}
		flags, err1 := readU16BE(data, pos)
		compGidRaw, err2 := readU16BE(data, pos+2)
		if err1 != nil || err2 != nil {
			break
		}
		pos += 4
		compGid := int(compGidRaw)

		argsWords := (flags & 0x0001) != 0
		argsXY := (flags & 0x0002) != 0

		var dx, dy int16
		if argsWords {
			if pos+4 > len(data) {
				break
			}
			a1, errA := readI16BE(data, pos)
			a2, errB := readI16BE(data, pos+2)
			if errA != nil || errB != nil {
				break
			}
			pos += 4
			if argsXY {
				dx, dy = a1, a2
			}
		} else {
			if pos+2 > len(data) {
				break
			}
			a1 := int16(int8(data[pos]))
			a2 := int16(int8(data[pos+1]))
			pos += 2
			if argsXY {
				dx, dy = a1, a2
			}
		}

		// Affine transform matrix: [a c; b d]
		a, b, c, d := float64(1.0), float64(0.0), float64(0.0), float64(1.0)
		if (flags & 0x0008) != 0 {
			// WE_HAVE_A_SCALE: uniform scale (F2.14)
			if pos+2 <= len(data) {
				if sRaw, err := readI16BE(data, pos); err == nil {
					s := float64(sRaw) / 16384.0
					a, d = s, s
				}
				pos += 2
			}
		} else if (flags & 0x0040) != 0 {
			// WE_HAVE_AN_X_AND_Y_SCALE: separate X and Y scales (F2.14)
			if pos+4 <= len(data) {
				if sxRaw, err := readI16BE(data, pos); err == nil {
					a = float64(sxRaw) / 16384.0
				}
				if syRaw, err := readI16BE(data, pos+2); err == nil {
					d = float64(syRaw) / 16384.0
				}
				pos += 4
			}
		} else if (flags & 0x0080) != 0 {
			// WE_HAVE_A_TWO_BY_TWO: 2x2 transformation matrix (F2.14)
			if pos+8 <= len(data) {
				if aRaw, err := readI16BE(data, pos); err == nil {
					a = float64(aRaw) / 16384.0
				}
				if bRaw, err := readI16BE(data, pos+2); err == nil {
					b = float64(bRaw) / 16384.0
				}
				if cRaw, err := readI16BE(data, pos+4); err == nil {
					c = float64(cRaw) / 16384.0
				}
				if dRaw, err := readI16BE(data, pos+6); err == nil {
					d = float64(dRaw) / 16384.0
				}
				pos += 8
			}
		}

		if compGid+1 < len(offsets) {
			cStart := offsets[compGid]
			cEnd := offsets[compGid+1]
			if cStart < cEnd && cEnd <= len(glyf) {
				if sub, err := parseSingleGlyph(glyf[cStart:cEnd], glyf, offsets, depth); err == nil && sub != nil {
					for _, contour := range sub.Contours {
						transformed := make([]OutlinePoint, len(contour))
						for idx, pt := range contour {
							px := float64(pt.X)
							py := float64(pt.Y)
							tx := a*px + c*py + float64(dx)
							ty := b*px + d*py + float64(dy)

							clampX := int16(math.Round(math.Max(-32768, math.Min(32767, tx))))
							clampY := int16(math.Round(math.Max(-32768, math.Min(32767, ty))))
							transformed[idx] = OutlinePoint{
								X:       clampX,
								Y:       clampY,
								OnCurve: pt.OnCurve,
							}
						}
						allContours = append(allContours, transformed)
					}
				}
			}
		}

		// MORE_COMPONENTS: 0x0020
		if (flags & 0x0020) == 0 {
			break
		}
	}

	return allContours, nil
}
