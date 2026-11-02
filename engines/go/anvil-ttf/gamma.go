// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"math"
	"sync"
)

var (
	srgbToLinearOnce sync.Once
	srgbToLinearLUT  [256]uint16

	linearToSrgbOnce sync.Once
	linearToSrgbLUT  [4096]byte
)

func initSrgbToLinearLUT() {
	for i := 0; i < 256; i++ {
		s := float64(i) / 255.0
		var lin float64
		if s <= 0.04045 {
			lin = s / 12.92
		} else {
			lin = math.Pow((s+0.055)/1.055, 2.4)
		}
		val := math.Round(lin * 4095.0)
		if val < 0 {
			val = 0
		} else if val > 4095 {
			val = 4095
		}
		srgbToLinearLUT[i] = uint16(val)
	}
}

func initLinearToSrgbLUT() {
	for i := 0; i < 4096; i++ {
		lin := float64(i) / 4095.0
		var s float64
		if lin <= 0.0031308 {
			s = lin * 12.92
		} else {
			s = 1.055*math.Pow(lin, 1.0/2.4) - 0.055
		}
		val := math.Round(s * 255.0)
		if val < 0 {
			val = 0
		} else if val > 255 {
			val = 255
		}
		linearToSrgbLUT[i] = byte(val)
	}
}

// SRGBToLinearU12 converts an 8-bit sRGB channel (0..255) to 12-bit linear light (0..4095).
func SRGBToLinearU12(srgb byte) uint16 {
	srgbToLinearOnce.Do(initSrgbToLinearLUT)
	return srgbToLinearLUT[srgb]
}

// LinearU12ToSRGB converts a 12-bit linear light value (0..4095) back to 8-bit sRGB (0..255).
func LinearU12ToSRGB(linear uint16) byte {
	linearToSrgbOnce.Do(initLinearToSrgbLUT)
	if linear > 4095 {
		linear = 4095
	}
	return linearToSrgbLUT[linear]
}

// EnhanceStemContrast applies a luminance-aware stem contrast curve to coverage bytes.
// Boosts mid-tone coverage on dark-on-light text so thin 1px stems remain crisp and solid.
func EnhanceStemContrast(coverage byte, fgLuma byte) byte {
	if coverage == 0 || coverage == 255 {
		return coverage
	}
	a := uint32(coverage)
	inv := 255 - a
	boost := (a * inv) / 255
	darkWeight := uint32(255 - fgLuma)
	delta := (boost * (darkWeight + 64)) / 512
	out := a + delta
	if out > 255 {
		out = 255
	}
	return byte(out)
}

// BlendGlyphPixelGammaCorrect blends a foreground RGB text pixel over a destination RGB pixel
// in gamma-correct linear light space with luminance-aware stem contrast enhancement.
func BlendGlyphPixelGammaCorrect(dstRGB, fgRGB [3]byte, coverage byte) [3]byte {
	if coverage == 0 {
		return dstRGB
	}
	if coverage == 255 {
		return fgRGB
	}

	fgLuma := byte((uint32(fgRGB[0])*54 + uint32(fgRGB[1])*183 + uint32(fgRGB[2])*19) >> 8)
	alpha := uint32(EnhanceStemContrast(coverage, fgLuma))
	invAlpha := 255 - alpha

	dr := uint32(SRGBToLinearU12(dstRGB[0]))
	dg := uint32(SRGBToLinearU12(dstRGB[1]))
	db := uint32(SRGBToLinearU12(dstRGB[2]))

	sr := uint32(SRGBToLinearU12(fgRGB[0]))
	sg := uint32(SRGBToLinearU12(fgRGB[1]))
	sb := uint32(SRGBToLinearU12(fgRGB[2]))

	or := uint16((sr*alpha + dr*invAlpha + 127) / 255)
	og := uint16((sg*alpha + dg*invAlpha + 127) / 255)
	ob := uint16((sb*alpha + db*invAlpha + 127) / 255)

	return [3]byte{
		LinearU12ToSRGB(or),
		LinearU12ToSRGB(og),
		LinearU12ToSRGB(ob),
	}
}

// BlendSubpixelLCDGammaCorrect blends a 3-channel RGB subpixel LCD coverage triple over dstRGB
// in gamma-correct linear space.
func BlendSubpixelLCDGammaCorrect(dstRGB, fgRGB, covRGB [3]byte) [3]byte {
	if covRGB == [3]byte{0, 0, 0} {
		return dstRGB
	}
	if covRGB == [3]byte{255, 255, 255} {
		return fgRGB
	}

	fgLuma := byte((uint32(fgRGB[0])*54 + uint32(fgRGB[1])*183 + uint32(fgRGB[2])*19) >> 8)
	ar := uint32(EnhanceStemContrast(covRGB[0], fgLuma))
	ag := uint32(EnhanceStemContrast(covRGB[1], fgLuma))
	ab := uint32(EnhanceStemContrast(covRGB[2], fgLuma))

	dr := uint32(SRGBToLinearU12(dstRGB[0]))
	dg := uint32(SRGBToLinearU12(dstRGB[1]))
	db := uint32(SRGBToLinearU12(dstRGB[2]))

	sr := uint32(SRGBToLinearU12(fgRGB[0]))
	sg := uint32(SRGBToLinearU12(fgRGB[1]))
	sb := uint32(SRGBToLinearU12(fgRGB[2]))

	or := uint16((sr*ar + dr*(255-ar) + 127) / 255)
	og := uint16((sg*ag + dg*(255-ag) + 127) / 255)
	ob := uint16((sb*ab + db*(255-ab) + 127) / 255)

	return [3]byte{
		LinearU12ToSRGB(or),
		LinearU12ToSRGB(og),
		LinearU12ToSRGB(ob),
	}
}
