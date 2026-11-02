// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"encoding/binary"
	"math"
)

func readCffOffset(cff []byte, start, offSize int) (int, bool) {
	if start+offSize > len(cff) {
		return 0, false
	}
	switch offSize {
	case 1:
		return int(cff[start]), true
	case 2:
		return int(binary.BigEndian.Uint16(cff[start : start+2])), true
	case 3:
		return int(cff[start])<<16 | int(cff[start+1])<<8 | int(cff[start+2]), true
	case 4:
		return int(binary.BigEndian.Uint32(cff[start : start+4])), true
	default:
		return 0, false
	}
}

// parseCffIndex parses a CFF 1.0 INDEX structure, returning slices of each element and the offset after the index.
func parseCffIndex(cff []byte, start int) ([][]byte, int, bool) {
	if start+2 > len(cff) {
		return nil, 0, false
	}
	count := int(binary.BigEndian.Uint16(cff[start : start+2]))
	if count == 0 {
		return nil, start + 2, true
	}
	if start+3 > len(cff) || count > 65535 {
		return nil, 0, false
	}

	offSize := int(cff[start+2])
	if offSize < 1 || offSize > 4 {
		return nil, 0, false
	}

	offArrayStart := start + 3
	offArrayLen := (count + 1) * offSize
	dataBase := offArrayStart + offArrayLen - 1
	if offArrayStart+offArrayLen > len(cff) {
		return nil, 0, false
	}

	offsets := make([]int, count+1)
	for idx := 0; idx <= count; idx++ {
		off, ok := readCffOffset(cff, offArrayStart+idx*offSize, offSize)
		if !ok || off == 0 {
			return nil, 0, false
		}
		offsets[idx] = off
	}

	endPos := dataBase + offsets[len(offsets)-1]
	if endPos > len(cff) {
		return nil, 0, false
	}

	maxItems := count
	if maxItems > 8192 {
		maxItems = 8192
	}
	items := make([][]byte, 0, maxItems)
	for idx := 0; idx < maxItems; idx++ {
		s := dataBase + offsets[idx]
		e := dataBase + offsets[idx+1]
		if s > e || e > len(cff) {
			return nil, 0, false
		}
		items = append(items, cff[s:e])
	}

	return items, endPos, true
}

// parseCffDictOperands extracts operand values preceding targetOp in a CFF Top DICT.
func parseCffDictOperands(dict []byte, targetOp byte) []int32 {
	var stack []int32
	i := 0
	for i < len(dict) {
		b0 := dict[i]
		i++
		switch {
		case b0 == 28:
			if i+2 > len(dict) {
				break
			}
			v := int32(int16(binary.BigEndian.Uint16(dict[i : i+2])))
			i += 2
			stack = append(stack, v)
		case b0 == 29:
			if i+4 > len(dict) {
				break
			}
			v := int32(binary.BigEndian.Uint32(dict[i : i+4]))
			i += 4
			stack = append(stack, v)
		case b0 == 30:
			for i < len(dict) {
				nib := dict[i]
				i++
				if (nib&0xF0) == 0xF0 || (nib&0x0F) == 0x0F {
					break
				}
			}
			stack = append(stack, 0)
		case b0 >= 32 && b0 <= 246:
			stack = append(stack, int32(b0)-139)
		case b0 >= 247 && b0 <= 250:
			if i >= len(dict) {
				break
			}
			b1 := dict[i]
			i++
			stack = append(stack, (int32(b0)-247)*256+int32(b1)+108)
		case b0 >= 251 && b0 <= 254:
			if i >= len(dict) {
				break
			}
			b1 := dict[i]
			i++
			stack = append(stack, -((int32(b0)-251)*256 + int32(b1) + 108))
		case b0 == 12:
			if i < len(dict) {
				i++
			}
			stack = nil
		case b0 == targetOp:
			if len(stack) > 0 {
				return stack
			}
			stack = nil
		default:
			stack = nil
		}
	}
	return nil
}

func cffSubrBias(count int) int32 {
	if count < 1240 {
		return 107
	} else if count < 33900 {
		return 1131
	}
	return 32768
}

// parseCffTable decodes OpenType CFF 1.0 outlines for all glyphs.
func parseCffTable(cff []byte, numGlyphs int) ([]*GlyphOutline, error) {
	if len(cff) < 4 || cff[0] != 1 {
		return nil, errInvalidTable("CFF ")
	}
	hdrSize := int(cff[2])
	_, afterNameIdx, ok := parseCffIndex(cff, hdrSize)
	if !ok {
		return nil, errInvalidTable("CFF ")
	}

	topDicts, afterTopIdx, ok := parseCffIndex(cff, afterNameIdx)
	if !ok || len(topDicts) == 0 {
		return nil, errInvalidTable("CFF ")
	}
	topDict := topDicts[0]

	charstringsOps := parseCffDictOperands(topDict, 17)
	if len(charstringsOps) == 0 || charstringsOps[len(charstringsOps)-1] <= 0 {
		return nil, errInvalidTable("CFF ")
	}
	charstringsOff := int(charstringsOps[len(charstringsOps)-1])
	charstrings, _, ok := parseCffIndex(cff, charstringsOff)
	if !ok {
		return nil, errInvalidTable("CFF ")
	}

	var globalSubrs [][]byte
	if _, afterStrIdx, ok := parseCffIndex(cff, afterTopIdx); ok {
		if gsubrs, _, ok := parseCffIndex(cff, afterStrIdx); ok {
			globalSubrs = gsubrs
		}
	}

	var localSubrs [][]byte
	privOps := parseCffDictOperands(topDict, 18)
	if len(privOps) >= 2 {
		privSize := int(privOps[len(privOps)-2])
		privOff := int(privOps[len(privOps)-1])
		if privSize > 0 && privOff > 0 && privOff+privSize <= len(cff) {
			privSlice := cff[privOff : privOff+privSize]
			subrsOps := parseCffDictOperands(privSlice, 19)
			if len(subrsOps) > 0 {
				subrsRel := int(subrsOps[len(subrsOps)-1])
				subrsAbs := privOff + subrsRel
				if lsubrs, _, ok := parseCffIndex(cff, subrsAbs); ok {
					localSubrs = lsubrs
				}
			}
		}
	}

	outlines := make([]*GlyphOutline, numGlyphs)
	for gid := 0; gid < numGlyphs; gid++ {
		if gid < len(charstrings) {
			prog := charstrings[gid]
			outline, _ := InterpretType2CharStringWithSubrs(prog, localSubrs, globalSubrs)
			outlines[gid] = outline
		}
	}

	return outlines, nil
}

// InterpretType2CharString interprets a CFF Type 2 CharString program without subroutines.
func InterpretType2CharString(program []byte) (*GlyphOutline, error) {
	return InterpretType2CharStringWithSubrs(program, nil, nil)
}

func flushCffContour(current *[]CubicPoint, contours *[][]OutlinePoint) {
	if len(*current) >= 2 && len(*contours) < MaxGlyphContours {
		poly := FlattenCubicContour(*current)
		if len(poly) >= 3 {
			maxPts := len(poly)
			if maxPts > MaxContourPoints {
				maxPts = MaxContourPoints
			}
			pts := make([]OutlinePoint, maxPts)
			for i := 0; i < maxPts; i++ {
				x := int16(math.Round(math.Max(-32768, math.Min(32767, float64(poly[i][0])))))
				y := int16(math.Round(math.Max(-32768, math.Min(32767, float64(poly[i][1])))))
				pts[i] = OutlinePoint{X: x, Y: y, OnCurve: true}
			}
			*contours = append(*contours, pts)
		}
	}
	*current = nil
}

// InterpretType2CharStringWithSubrs interprets a CFF Type 2 CharString program with subroutine support.
func InterpretType2CharStringWithSubrs(program []byte, localSubrs, globalSubrs [][]byte) (*GlyphOutline, error) {
	stack := make([]float32, 0, 48)
	var contours [][]OutlinePoint
	var current []CubicPoint

	cx, cy := float32(0.0), float32(0.0)
	stemHints := 0
	i := 0

	type callFrame struct {
		prog []byte
		idx  int
	}
	var callStack []callFrame

	localBias := cffSubrBias(len(localSubrs))
	globalBias := cffSubrBias(len(globalSubrs))

	for {
		if i >= len(program) {
			if len(callStack) > 0 {
				last := callStack[len(callStack)-1]
				callStack = callStack[:len(callStack)-1]
				program = last.prog
				i = last.idx
				continue
			}
			break
		}

		b0 := program[i]
		i++

		switch {
		case b0 == 28:
			if i+2 > len(program) {
				break
			}
			val := float32(int16(binary.BigEndian.Uint16(program[i : i+2])))
			i += 2
			if len(stack) < 48 {
				stack = append(stack, val)
			}
		case b0 >= 32 && b0 <= 246:
			if len(stack) < 48 {
				stack = append(stack, float32(int32(b0)-139))
			}
		case b0 >= 247 && b0 <= 250:
			if i >= len(program) {
				break
			}
			b1 := program[i]
			i++
			val := float32((int32(b0)-247)*256 + int32(b1) + 108)
			if len(stack) < 48 {
				stack = append(stack, val)
			}
		case b0 >= 251 && b0 <= 254:
			if i >= len(program) {
				break
			}
			b1 := program[i]
			i++
			val := -float32((int32(b0)-251)*256 + int32(b1) + 108)
			if len(stack) < 48 {
				stack = append(stack, val)
			}
		case b0 == 255:
			if i+4 > len(program) {
				break
			}
			raw := int32(binary.BigEndian.Uint32(program[i : i+4]))
			i += 4
			if len(stack) < 48 {
				stack = append(stack, float32(raw)/65536.0)
			}
		case b0 == 1 || b0 == 3 || b0 == 18 || b0 == 23:
			stemHints += len(stack) / 2
			stack = stack[:0]
		case b0 == 19 || b0 == 20:
			stemHints += len(stack) / 2
			stack = stack[:0]
			maskBytes := (stemHints + 7) / 8
			i += maskBytes
			if i > len(program) {
				i = len(program)
			}
		case b0 == 21:
			// rmoveto
			flushCffContour(&current, &contours)
			if len(stack) >= 2 {
				dx := stack[len(stack)-2]
				dy := stack[len(stack)-1]
				cx += dx
				cy += dy
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
			}
			stack = stack[:0]
		case b0 == 22:
			// hmoveto
			flushCffContour(&current, &contours)
			if len(stack) >= 1 {
				cx += stack[len(stack)-1]
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
			}
			stack = stack[:0]
		case b0 == 4:
			// vmoveto
			flushCffContour(&current, &contours)
			if len(stack) >= 1 {
				cy += stack[len(stack)-1]
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
			}
			stack = stack[:0]
		case b0 == 5:
			// rlineto
			for p := 0; p+1 < len(stack); p += 2 {
				cx += stack[p]
				cy += stack[p+1]
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
			}
			stack = stack[:0]
		case b0 == 6 || b0 == 7:
			// hlineto (6) or vlineto (7)
			horiz := (b0 == 6)
			for _, delta := range stack {
				if horiz {
					cx += delta
				} else {
					cy += delta
				}
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
				horiz = !horiz
			}
			stack = stack[:0]
		case b0 == 8:
			// rrcurveto
			for seg := 0; seg+5 < len(stack); seg += 6 {
				x1 := cx + stack[seg]
				y1 := cy + stack[seg+1]
				x2 := x1 + stack[seg+2]
				y2 := y1 + stack[seg+3]
				cx = x2 + stack[seg+4]
				cy = y2 + stack[seg+5]
				current = append(current,
					CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
				)
			}
			stack = stack[:0]
		case b0 == 24:
			// rcurveline
			if len(stack) >= 8 {
				curveEnd := len(stack) - 2
				for seg := 0; seg+5 < curveEnd; seg += 6 {
					x1 := cx + stack[seg]
					y1 := cy + stack[seg+1]
					x2 := x1 + stack[seg+2]
					y2 := y1 + stack[seg+3]
					cx = x2 + stack[seg+4]
					cy = y2 + stack[seg+5]
					current = append(current,
						CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
					)
				}
				cx += stack[curveEnd]
				cy += stack[curveEnd+1]
				current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
			}
			stack = stack[:0]
		case b0 == 25:
			// rlinecurve
			if len(stack) >= 8 {
				lineEnd := len(stack) - 6
				for p := 0; p+1 < lineEnd; p += 2 {
					cx += stack[p]
					cy += stack[p+1]
					current = append(current, CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve})
				}
				seg := stack[lineEnd:]
				x1 := cx + seg[0]
				y1 := cy + seg[1]
				x2 := x1 + seg[2]
				y2 := y1 + seg[3]
				cx = x2 + seg[4]
				cy = y2 + seg[5]
				current = append(current,
					CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
				)
			}
			stack = stack[:0]
		case b0 == 26:
			// vvcurveto
			idx := 0
			dx1 := float32(0.0)
			if len(stack)%4 == 1 {
				dx1 = stack[0]
				idx = 1
			}
			for idx+3 < len(stack) {
				x1 := cx + dx1
				y1 := cy + stack[idx]
				x2 := x1 + stack[idx+1]
				y2 := y1 + stack[idx+2]
				cx = x2
				cy = y2 + stack[idx+3]
				current = append(current,
					CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
				)
				dx1 = 0.0
				idx += 4
			}
			stack = stack[:0]
		case b0 == 27:
			// hhcurveto
			idx := 0
			dy1 := float32(0.0)
			if len(stack)%4 == 1 {
				dy1 = stack[0]
				idx = 1
			}
			for idx+3 < len(stack) {
				x1 := cx + stack[idx]
				y1 := cy + dy1
				x2 := x1 + stack[idx+1]
				y2 := y1 + stack[idx+2]
				cx = x2 + stack[idx+3]
				cy = y2
				current = append(current,
					CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
					CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
				)
				dy1 = 0.0
				idx += 4
			}
			stack = stack[:0]
		case b0 == 30 || b0 == 31:
			// vhcurveto (30) or hvcurveto (31)
			vertFirst := (b0 == 30)
			idx := 0
			for idx+3 < len(stack) {
				trailing := float32(0.0)
				if idx+5 == len(stack) {
					trailing = stack[idx+4]
				}
				if vertFirst {
					x1 := cx
					y1 := cy + stack[idx]
					x2 := x1 + stack[idx+1]
					y2 := y1 + stack[idx+2]
					cx = x2 + stack[idx+3]
					cy = y2 + trailing
					current = append(current,
						CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
					)
				} else {
					x1 := cx + stack[idx]
					y1 := cy
					x2 := x1 + stack[idx+1]
					y2 := y1 + stack[idx+2]
					cx = x2 + trailing
					cy = y2 + stack[idx+3]
					current = append(current,
						CubicPoint{X: x1, Y: y1, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: x2, Y: y2, Order: CurveOrderCubicOffCurve},
						CubicPoint{X: cx, Y: cy, Order: CurveOrderOnCurve},
					)
				}
				vertFirst = !vertFirst
				if idx+5 == len(stack) {
					idx += 5
				} else {
					idx += 4
				}
			}
			stack = stack[:0]
		case b0 == 12:
			if i < len(program) {
				i++
			}
			stack = stack[:0]
		case b0 == 10 || b0 == 29:
			// callsubr (10) or callgsubr (29)
			if len(stack) > 0 {
				biased := stack[len(stack)-1]
				stack = stack[:len(stack)-1]

				subrs := localSubrs
				bias := localBias
				if b0 == 29 {
					subrs = globalSubrs
					bias = globalBias
				}

				subrIdx := int32(math.Round(float64(biased))) + bias
				if subrIdx >= 0 && int(subrIdx) < len(subrs) && len(callStack) < 16 {
					callStack = append(callStack, callFrame{prog: program, idx: i})
					program = subrs[int(subrIdx)]
					i = 0
				}
			}
		case b0 == 11:
			// return from subroutine
			i = len(program)
		case b0 == 14:
			// endchar
			flushCffContour(&current, &contours)
			break
		default:
			stack = stack[:0]
		}
	}

	flushCffContour(&current, &contours)
	if len(contours) == 0 {
		return nil, nil
	}

	xMin, yMin := int16(32767), int16(32767)
	xMax, yMax := int16(-32768), int16(-32768)
	for _, contour := range contours {
		for _, pt := range contour {
			if pt.X < xMin {
				xMin = pt.X
			}
			if pt.Y < yMin {
				yMin = pt.Y
			}
			if pt.X > xMax {
				xMax = pt.X
			}
			if pt.Y > yMax {
				yMax = pt.Y
			}
		}
	}

	return &GlyphOutline{
		XMin:     xMin,
		YMin:     yMin,
		XMax:     xMax,
		YMax:     yMax,
		Contours: contours,
	}, nil
}
