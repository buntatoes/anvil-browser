// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilttf

import "math"

// snapVerticalGridY snaps horizontal stems and the baseline (y == 0) to integer pixel rows.
func snapVerticalGridY(cur, prev, next OutlinePoint, scale float32, gridFit bool) float32 {
	rawY := -float32(cur.Y) * scale
	if !gridFit {
		return rawY
	}

	isBaseline := (cur.Y == 0)
	dyPrev := int32(cur.Y) - int32(prev.Y)
	if dyPrev < 0 {
		dyPrev = -dyPrev
	}
	dxPrev := int32(cur.X) - int32(prev.X)
	if dxPrev < 0 {
		dxPrev = -dxPrev
	}

	dyNext := int32(cur.Y) - int32(next.Y)
	if dyNext < 0 {
		dyNext = -dyNext
	}
	dxNext := int32(cur.X) - int32(next.X)
	if dxNext < 0 {
		dxNext = -dxNext
	}

	isHorizStem := cur.OnCurve &&
		((prev.OnCurve && dyPrev <= 2 && dxPrev >= 8) ||
			(next.OnCurve && dyNext <= 2 && dxNext >= 8))

	if isBaseline || isHorizStem {
		return float32(math.Round(float64(rawY)))
	}
	return rawY
}

// SubpixelXPhaseBin quantizes a fractional X offset into one of 4 subpixel phase bins (0..3).
func SubpixelXPhaseBin(x float32) uint8 {
	frac := float64(x) - math.Floor(float64(x))
	bin := int(math.Floor(frac * 4.0))
	if bin < 0 {
		return 0
	}
	if bin > 3 {
		return 3
	}
	return uint8(bin)
}

// flattenContourOpts turns one closed contour into a polygon in pixel space (y increasing downward).
func flattenContourOpts(points []OutlinePoint, scale float32, xOffsetPx float32, gridFit bool) [][2]float32 {
	n := len(points)
	if n < 2 {
		return nil
	}

	type expPoint struct {
		x       float32
		y       float32
		onCurve bool
	}

	expanded := make([]expPoint, 0, n*2)
	for i := 0; i < n; i++ {
		cur := points[i]
		next := points[(i+1)%n]
		prev := points[(i+n-1)%n]

		cx := float32(cur.X)*scale + xOffsetPx
		cy := snapVerticalGridY(cur, prev, next, scale, gridFit)
		expanded = append(expanded, expPoint{x: cx, y: cy, onCurve: cur.OnCurve})

		// Consecutive off-curve points imply an on-curve point halfway between them.
		if !cur.OnCurve && !next.OnCurve {
			after := points[(i+2)%n]
			nx := float32(next.X)*scale + xOffsetPx
			ny := snapVerticalGridY(next, cur, after, scale, gridFit)
			expanded = append(expanded, expPoint{
				x:       (cx + nx) * 0.5,
				y:       (cy + ny) * 0.5,
				onCurve: true,
			})
		}
	}

	firstOn := 0
	for i, p := range expanded {
		if p.onCurve {
			firstOn = i
			break
		}
	}

	m := len(expanded)
	var poly [][2]float32
	idx := 0

	for idx < m {
		p0 := expanded[(firstOn+idx)%m]
		p1 := expanded[(firstOn+idx+1)%m]

		if p1.onCurve {
			poly = append(poly, [2]float32{p0.x, p0.y})
			idx++
		} else {
			p2 := expanded[(firstOn+idx+2)%m]
			steps := 6
			fSteps := float32(steps)
			for s := 0; s < steps; s++ {
				t := float32(s) / fSteps
				inv := 1.0 - t
				x := inv*inv*p0.x + 2.0*inv*t*p1.x + t*t*p2.x
				y := inv*inv*p0.y + 2.0*inv*t*p1.y + t*t*p2.y
				poly = append(poly, [2]float32{x, y})
			}
			idx += 2
		}
	}

	return poly
}
