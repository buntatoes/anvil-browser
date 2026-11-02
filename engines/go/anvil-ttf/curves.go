// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

// FlattenQuadBezier subdivides a quadratic Bezier segment (p0, c, p2) into steps linear vertices
// (t = 0 included, t = 1 omitted).
func FlattenQuadBezier(p0, c, p2 [2]float32, steps int, out *[][2]float32) {
	if steps < 1 {
		steps = 1
	}
	fSteps := float32(steps)
	for s := 0; s < steps; s++ {
		t := float32(s) / fSteps
		inv := 1.0 - t
		x := inv*inv*p0[0] + 2.0*inv*t*c[0] + t*t*p2[0]
		y := inv*inv*p0[1] + 2.0*inv*t*c[1] + t*t*p2[1]
		*out = append(*out, [2]float32{x, y})
	}
}

// FlattenCubicBezier subdivides a cubic Bezier segment (p0, c1, c2, p3) into steps linear vertices
// (t = 0 included, t = 1 omitted).
func FlattenCubicBezier(p0, c1, c2, p3 [2]float32, steps int, out *[][2]float32) {
	if steps < 1 {
		steps = 1
	}
	fSteps := float32(steps)
	for s := 0; s < steps; s++ {
		t := float32(s) / fSteps
		inv := 1.0 - t
		b0 := inv * inv * inv
		b1 := 3.0 * inv * inv * t
		b2 := 3.0 * inv * t * t
		b3 := t * t * t
		x := b0*p0[0] + b1*c1[0] + b2*c2[0] + b3*p3[0]
		y := b0*p0[1] + b1*c1[1] + b2*c2[1] + b3*p3[1]
		*out = append(*out, [2]float32{x, y})
	}
}

// FlattenCubicContour flattens a mixed contour containing OnCurve, QuadraticOffCurve,
// and CubicOffCurve control points into linear polygon vertices.
func FlattenCubicContour(points []CubicPoint) [][2]float32 {
	if len(points) < 2 {
		return nil
	}

	firstOn := 0
	for i, p := range points {
		if p.Order == CurveOrderOnCurve {
			firstOn = i
			break
		}
	}

	n := len(points)
	var poly [][2]float32
	idx := 0

	for idx < n {
		p0 := points[(firstOn+idx)%n]
		p1 := points[(firstOn+idx+1)%n]

		switch p1.Order {
		case CurveOrderOnCurve:
			poly = append(poly, [2]float32{p0.X, p0.Y})
			idx++
		case CurveOrderQuadraticOffCurve:
			p2 := points[(firstOn+idx+2)%n]
			FlattenQuadBezier([2]float32{p0.X, p0.Y}, [2]float32{p1.X, p1.Y}, [2]float32{p2.X, p2.Y}, 6, &poly)
			idx += 2
		case CurveOrderCubicOffCurve:
			c2 := points[(firstOn+idx+2)%n]
			p3 := points[(firstOn+idx+3)%n]
			FlattenCubicBezier(
				[2]float32{p0.X, p0.Y},
				[2]float32{p1.X, p1.Y},
				[2]float32{c2.X, c2.Y},
				[2]float32{p3.X, p3.Y},
				8,
				&poly,
			)
			idx += 3
		}
	}

	return poly
}
