// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilttf

import (
	"os"
	"path/filepath"
	"runtime"
)

// LoadSystemFontSlot loads a font matching the requested FontFamilySlot from host OS directories.
func LoadSystemFontSlot(slot FontFamilySlot) *Font {
	candidates := getCandidateFontPaths(slot)
	for _, path := range candidates {
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		font, err := Parse(data)
		if err == nil && font != nil {
			return font
		}
	}
	return nil
}

// LoadSystemFont loads a standard system font based on bold and monospace flags.
func LoadSystemFont(bold, monospace bool) *Font {
	if monospace {
		if bold {
			return LoadSystemFontSlot(MonospaceBold)
		}
		return LoadSystemFontSlot(MonospaceRegular)
	}
	if bold {
		return LoadSystemFontSlot(SansBold)
	}
	return LoadSystemFontSlot(SansRegular)
}

func getCandidateFontPaths(slot FontFamilySlot) []string {
	var candidates []string

	if runtime.GOOS == "windows" {
		winDir := os.Getenv("WINDIR")
		if winDir == "" {
			winDir = `C:\Windows`
		}
		fontsDir := filepath.Join(winDir, "Fonts")

		switch slot {
		case SansRegular:
			candidates = []string{
				filepath.Join(fontsDir, "segoeui.ttf"),
				filepath.Join(fontsDir, "arial.ttf"),
			}
		case SansBold:
			candidates = []string{
				filepath.Join(fontsDir, "segoeuib.ttf"),
				filepath.Join(fontsDir, "arialbd.ttf"),
			}
		case SansItalic:
			candidates = []string{
				filepath.Join(fontsDir, "segoeuii.ttf"),
				filepath.Join(fontsDir, "ariali.ttf"),
			}
		case SerifRegular:
			candidates = []string{
				filepath.Join(fontsDir, "georgia.ttf"),
				filepath.Join(fontsDir, "times.ttf"),
			}
		case SerifBold:
			candidates = []string{
				filepath.Join(fontsDir, "georgiab.ttf"),
				filepath.Join(fontsDir, "timesbd.ttf"),
			}
		case MonospaceRegular:
			candidates = []string{
				filepath.Join(fontsDir, "consola.ttf"),
				filepath.Join(fontsDir, "cour.ttf"),
			}
		case MonospaceBold:
			candidates = []string{
				filepath.Join(fontsDir, "consolab.ttf"),
				filepath.Join(fontsDir, "courbd.ttf"),
			}
		}
	} else {
		// Unix / Linux font candidate paths
		baseDirs := []string{
			"/usr/share/fonts/truetype/dejavu",
			"/usr/share/fonts/TTF",
			"/usr/share/fonts/truetype/liberation",
			"/usr/share/fonts/liberation",
		}

		for _, d := range baseDirs {
			switch slot {
			case SansRegular:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSans.ttf"),
					filepath.Join(d, "LiberationSans-Regular.ttf"),
				)
			case SansBold:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSans-Bold.ttf"),
					filepath.Join(d, "LiberationSans-Bold.ttf"),
				)
			case SansItalic:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSans-Oblique.ttf"),
					filepath.Join(d, "LiberationSans-Italic.ttf"),
				)
			case SerifRegular:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSerif.ttf"),
					filepath.Join(d, "LiberationSerif-Regular.ttf"),
				)
			case SerifBold:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSerif-Bold.ttf"),
					filepath.Join(d, "LiberationSerif-Bold.ttf"),
				)
			case MonospaceRegular:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSansMono.ttf"),
					filepath.Join(d, "LiberationMono-Regular.ttf"),
				)
			case MonospaceBold:
				candidates = append(candidates,
					filepath.Join(d, "DejaVuSansMono-Bold.ttf"),
					filepath.Join(d, "LiberationMono-Bold.ttf"),
				)
			}
		}
	}

	return candidates
}

// MeasureProportionalFallback estimates string width from calibrated typographic ratios
// when no TrueType/OpenType font is loaded.
func MeasureProportionalFallback(text string, fontSizePx float32) float32 {
	if fontSizePx <= 0 {
		return 0
	}
	var total float32
	for _, ch := range text {
		var ratio float32
		switch {
		case ch == ' ':
			ratio = 0.30
		case ch == 'i' || ch == 'l' || ch == '!' || ch == '|' || ch == '.' || ch == ',' || ch == ':' || ch == ';' || ch == '\'':
			ratio = 0.28
		case ch == 'I' || ch == 'j' || ch == 'f' || ch == 't' || ch == 'r' || ch == '(' || ch == ')' || ch == '[' || ch == ']' || ch == '{' || ch == '}':
			ratio = 0.36
		case ch == 'm' || ch == 'w' || ch == 'M' || ch == 'W' || ch == '@' || ch == '%':
			ratio = 0.82
		case ch >= 'A' && ch <= 'Z':
			ratio = 0.66
		case ch >= '0' && ch <= '9':
			ratio = 0.56
		default:
			ratio = 0.54
		}
		total += fontSizePx * ratio
	}
	return total
}
