// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"math"
	"os"
	"strconv"

	anvilttf "github.com/buntatoes/anvil-ttf"
)

const (
	version = "anvil-ttf 0.11.0 (go)"
	author  = "Buntato"

	opLoadFont       = 0x01
	opGlyphMetrics   = 0x02
	opRasterizeGlyph = 0x03
	opRasterizeLCD   = 0x04
	opShutdown       = 0xFF
)

func main() {
	args := os.Args[1:]
	if len(args) == 0 {
		printUsage()
		return
	}

	switch args[0] {
	case "--version", "-v", "version":
		fmt.Printf("%s\nAuthor: %s\n", version, author)
	case "--self-test":
		runSelfTest()
	case "--worker":
		runWorker()
	case "--inspect":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "Usage: anvil-ttf --inspect <font.ttf>")
			os.Exit(1)
		}
		inspectFont(args[1])
	case "--render":
		if len(args) < 5 {
			fmt.Fprintln(os.Stderr, "Usage: anvil-ttf --render <font.ttf> <char> <size_px> <out.pgm>")
			os.Exit(1)
		}
		renderGlyph(args[1], args[2], args[3], args[4])
	default:
		printUsage()
	}
}

func printUsage() {
	fmt.Printf("%s\n", version)
	fmt.Println("Usage: anvil-ttf [command] [args...]")
	fmt.Println("Commands:")
	fmt.Println("  --version                         Print version and author")
	fmt.Println("  --self-test                        Run internal validation tests")
	fmt.Println("  --worker                          Run in persistent binary IPC worker mode")
	fmt.Println("  --inspect <font.ttf>              Dump font tables and metrics")
	fmt.Println("  --render <font> <char> <px> <out> Render glyph to Netpbm PGM file")
}

func inspectFont(path string) {
	data, err := os.ReadFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error reading file: %v\n", err)
		os.Exit(1)
	}

	font, err := anvilttf.Parse(data)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error parsing font: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("Font Family:    %s\n", font.FamilyName())
	fmt.Printf("Units Per Em:   %d\n", font.UnitsPerEm)
	fmt.Printf("Ascender:       %d\n", font.Ascender)
	fmt.Printf("Descender:      %d\n", font.Descender)
	fmt.Printf("Line Gap:       %d\n", font.LineGap)
	fmt.Printf("Line Height 16: %.2f px\n", font.LineHeightPx(16.0))

	samples := []rune{'A', 'a', '0', 'W', 'g', '.'}
	for _, ch := range samples {
		gid := font.GlyphID(ch)
		metric := font.GlyphMetric(gid)
		advPx := font.GlyphAdvancePx(ch, 16.0)
		fmt.Printf("  '%c' (gid %5d): adv=%d lsb=%d (%.2f px @ 16px)\n", ch, gid, metric.AdvanceWidth, metric.LeftSideBearing, advPx)
	}
}

func renderGlyph(fontPath, charStr, sizeStr, outPath string) {
	data, err := os.ReadFile(fontPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error reading font file: %v\n", err)
		os.Exit(1)
	}

	font, err := anvilttf.Parse(data)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error parsing font: %v\n", err)
		os.Exit(1)
	}

	size, err := strconv.ParseFloat(sizeStr, 32)
	if err != nil || size <= 0 {
		fmt.Fprintf(os.Stderr, "Invalid font size: %s\n", sizeStr)
		os.Exit(1)
	}

	runes := []rune(charStr)
	if len(runes) == 0 {
		fmt.Fprintln(os.Stderr, "Empty character string")
		os.Exit(1)
	}
	ch := runes[0]

	bmp := font.RasterizeGlyph(ch, float32(size))
	if bmp == nil {
		fmt.Fprintf(os.Stderr, "Glyph '%c' produced no bitmap (empty or missing outline)\n", ch)
		os.Exit(1)
	}

	// Write PGM (P5 binary grayscale Netpbm)
	header := fmt.Sprintf("P5\n%d %d\n255\n", bmp.Width, bmp.Height)
	f, err := os.Create(outPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error creating output file: %v\n", err)
		os.Exit(1)
	}
	defer f.Close()

	if _, err := f.WriteString(header); err != nil {
		fmt.Fprintf(os.Stderr, "Error writing PGM header: %v\n", err)
		os.Exit(1)
	}
	if _, err := f.Write(bmp.Coverage); err != nil {
		fmt.Fprintf(os.Stderr, "Error writing PGM data: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("Rendered '%c' (%dx%d, bearing=(%d,%d), adv=%.2f) to %s\n",
		ch, bmp.Width, bmp.Height, bmp.BearingX, bmp.BearingY, bmp.AdvancePx, outPath)
}

func runSelfTest() {
	fmt.Println("Running internal self-tests...")

	// 1. Synthetic TTF Generation & Parse
	ttfData := buildTestSyntheticTTF()
	font, err := anvilttf.Parse(ttfData)
	if err != nil {
		fmt.Fprintf(os.Stderr, "FAIL: Parse synthetic TTF failed: %v\n", err)
		os.Exit(1)
	}

	if font.UnitsPerEm != 1000 {
		fmt.Fprintf(os.Stderr, "FAIL: UnitsPerEm expected 1000, got %d\n", font.UnitsPerEm)
		os.Exit(1)
	}
	if font.Ascender != 800 || font.Descender != -200 || font.LineGap != 100 {
		fmt.Fprintf(os.Stderr, "FAIL: Metrics mismatch: asc=%d desc=%d gap=%d\n", font.Ascender, font.Descender, font.LineGap)
		os.Exit(1)
	}

	// 2. Glyph Mapping & Metrics
	gidA := font.GlyphID('A')
	if gidA != 1 {
		fmt.Fprintf(os.Stderr, "FAIL: GlyphID('A') expected 1, got %d\n", gidA)
		os.Exit(1)
	}
	mA := font.GlyphMetric(gidA)
	if mA.AdvanceWidth != 700 || mA.LeftSideBearing != 20 {
		fmt.Fprintf(os.Stderr, "FAIL: Metric for 'A' expected 700/20, got %d/%d\n", mA.AdvanceWidth, mA.LeftSideBearing)
		os.Exit(1)
	}

	// 3. Rasterization
	bmp := font.RasterizeGlyph('A', 20.0)
	if bmp == nil || bmp.Width == 0 || bmp.Height == 0 {
		fmt.Fprintln(os.Stderr, "FAIL: RasterizeGlyph('A') produced nil or empty bitmap")
		os.Exit(1)
	}

	solidCount := 0
	for _, c := range bmp.Coverage {
		if c > 0 {
			solidCount++
		}
	}
	if solidCount == 0 {
		fmt.Fprintln(os.Stderr, "FAIL: RasterizeGlyph coverage is all zero")
		os.Exit(1)
	}

	// 4. Subpixel LCD Rasterization
	lcdBmp := font.RasterizeGlyphSubpixelLCD('A', 20.0)
	if lcdBmp == nil || lcdBmp.Width == 0 || lcdBmp.Height == 0 {
		fmt.Fprintln(os.Stderr, "FAIL: RasterizeGlyphSubpixelLCD produced nil or empty bitmap")
		os.Exit(1)
	}
	expectedLcdBytes := int(lcdBmp.Width * lcdBmp.Height * 3)
	if len(lcdBmp.RgbCoverage) != expectedLcdBytes {
		fmt.Fprintf(os.Stderr, "FAIL: Subpixel LCD coverage length expected %d, got %d\n", expectedLcdBytes, len(lcdBmp.RgbCoverage))
		os.Exit(1)
	}

	// 5. Linear Space Blending
	dst := [3]byte{255, 255, 255}
	fg := [3]byte{0, 0, 0}
	blended := anvilttf.BlendGlyphPixelGammaCorrect(dst, fg, 128)
	if blended[0] >= 255 || blended[1] >= 255 || blended[2] >= 255 {
		fmt.Fprintln(os.Stderr, "FAIL: Linear blend failed to darken destination")
		os.Exit(1)
	}

	// 6. Fallback Metrics
	w := anvilttf.MeasureProportionalFallback("Hello World", 16.0)
	if w <= 0 {
		fmt.Fprintln(os.Stderr, "FAIL: MeasureProportionalFallback returned <= 0")
		os.Exit(1)
	}

	fmt.Println("PASS: All internal self-tests succeeded.")
}

func runWorker() {
	serveWorker(os.Stdin, os.Stdout)
}

func serveWorker(stdin io.Reader, stdout io.Writer) {
	var activeFont *anvilttf.Font

	for {
		var opBuf [1]byte
		if _, err := io.ReadFull(stdin, opBuf[:]); err != nil {
			break
		}
		op := opBuf[0]

		var lenBuf [4]byte
		if _, err := io.ReadFull(stdin, lenBuf[:]); err != nil {
			break
		}
		payloadLen := binary.LittleEndian.Uint32(lenBuf[:])

		payload := make([]byte, payloadLen)
		if payloadLen > 0 {
			if _, err := io.ReadFull(stdin, payload); err != nil {
				break
			}
		}

		if op == opShutdown {
			sendWorkerResponse(stdout, 0x00, nil)
			return
		}

		switch op {
		case opLoadFont:
			font, err := anvilttf.Parse(payload)
			if err != nil {
				sendWorkerError(stdout, err.Error())
				continue
			}
			activeFont = font
			resp := make([]byte, 8)
			binary.LittleEndian.PutUint16(resp[0:2], font.UnitsPerEm)
			binary.LittleEndian.PutUint16(resp[2:4], uint16(font.Ascender))
			binary.LittleEndian.PutUint16(resp[4:6], uint16(font.Descender))
			binary.LittleEndian.PutUint16(resp[6:8], uint16(font.LineGap))
			sendWorkerResponse(stdout, 0x00, resp)

		case opGlyphMetrics:
			if activeFont == nil {
				sendWorkerError(stdout, "no font loaded")
				continue
			}
			if len(payload) < 4 {
				sendWorkerError(stdout, "invalid payload for OP_GLYPH_METRICS")
				continue
			}
			code := binary.LittleEndian.Uint32(payload[:4])
			gid := activeFont.GlyphID(rune(code))
			metric := activeFont.GlyphMetric(gid)

			resp := make([]byte, 12)
			binary.LittleEndian.PutUint16(resp[0:2], metric.AdvanceWidth)
			binary.LittleEndian.PutUint16(resp[2:4], uint16(metric.LeftSideBearing))

			outline := activeFont.GlyphOutline(gid)
			if outline != nil {
				binary.LittleEndian.PutUint16(resp[4:6], uint16(outline.XMin))
				binary.LittleEndian.PutUint16(resp[6:8], uint16(outline.YMin))
				binary.LittleEndian.PutUint16(resp[8:10], uint16(outline.XMax))
				binary.LittleEndian.PutUint16(resp[10:12], uint16(outline.YMax))
			}
			sendWorkerResponse(stdout, 0x00, resp)

		case opRasterizeGlyph:
			if activeFont == nil {
				sendWorkerError(stdout, "no font loaded")
				continue
			}
			if len(payload) < 11 {
				sendWorkerError(stdout, "invalid payload for OP_RASTERIZE_GLYPH")
				continue
			}
			ch := rune(binary.LittleEndian.Uint32(payload[0:4]))
			fontSize := math.Float32frombits(binary.LittleEndian.Uint32(payload[4:8]))
			phase := payload[8]
			bold := payload[9] != 0
			italic := payload[10] != 0

			bmp := activeFont.RasterizeGlyphStyled(ch, fontSize, phase, bold, italic)
			if bmp == nil {
				sendWorkerError(stdout, "failed to rasterize glyph")
				continue
			}

			hdrLen := 20
			resp := make([]byte, hdrLen+len(bmp.Coverage))
			binary.LittleEndian.PutUint32(resp[0:4], bmp.Width)
			binary.LittleEndian.PutUint32(resp[4:8], bmp.Height)
			binary.LittleEndian.PutUint32(resp[8:12], uint32(bmp.BearingX))
			binary.LittleEndian.PutUint32(resp[12:16], uint32(bmp.BearingY))
			binary.LittleEndian.PutUint32(resp[16:20], math.Float32bits(bmp.AdvancePx))
			copy(resp[hdrLen:], bmp.Coverage)
			sendWorkerResponse(stdout, 0x00, resp)

		case opRasterizeLCD:
			if activeFont == nil {
				sendWorkerError(stdout, "no font loaded")
				continue
			}
			if len(payload) < 8 {
				sendWorkerError(stdout, "invalid payload for OP_RASTERIZE_LCD")
				continue
			}
			ch := rune(binary.LittleEndian.Uint32(payload[0:4]))
			fontSize := math.Float32frombits(binary.LittleEndian.Uint32(payload[4:8]))

			bmp := activeFont.RasterizeGlyphSubpixelLCD(ch, fontSize)
			if bmp == nil {
				sendWorkerError(stdout, "failed to rasterize subpixel LCD")
				continue
			}

			hdrLen := 20
			resp := make([]byte, hdrLen+len(bmp.RgbCoverage))
			binary.LittleEndian.PutUint32(resp[0:4], bmp.Width)
			binary.LittleEndian.PutUint32(resp[4:8], bmp.Height)
			binary.LittleEndian.PutUint32(resp[8:12], uint32(bmp.BearingX))
			binary.LittleEndian.PutUint32(resp[12:16], uint32(bmp.BearingY))
			binary.LittleEndian.PutUint32(resp[16:20], math.Float32bits(bmp.AdvancePx))
			copy(resp[hdrLen:], bmp.RgbCoverage)
			sendWorkerResponse(stdout, 0x00, resp)

		default:
			sendWorkerError(stdout, fmt.Sprintf("unsupported opcode: 0x%02X", op))
		}
	}
}

func sendWorkerResponse(w io.Writer, status byte, payload []byte) {
	var hdr [5]byte
	hdr[0] = status
	binary.LittleEndian.PutUint32(hdr[1:5], uint32(len(payload)))
	_, _ = w.Write(hdr[:])
	if len(payload) > 0 {
		_, _ = w.Write(payload)
	}
}

func sendWorkerError(w io.Writer, msg string) {
	sendWorkerResponse(w, 0x01, []byte(msg))
}

// buildTestSyntheticTTF generates an in-memory TrueType font for self-testing.
func buildTestSyntheticTTF() []byte {
	head := make([]byte, 54)
	binary.BigEndian.PutUint16(head[18:20], 1000)
	binary.BigEndian.PutUint16(head[50:52], 1) // long loca

	hhea := make([]byte, 36)
	binary.BigEndian.PutUint16(hhea[4:6], 800)
	binary.BigEndian.PutUint16(hhea[6:8], 0xFF38) // -200
	binary.BigEndian.PutUint16(hhea[8:10], 100)
	binary.BigEndian.PutUint16(hhea[34:36], 3)

	maxp := make([]byte, 6)
	binary.BigEndian.PutUint16(maxp[4:6], 3)

	var hmtx []byte
	metrics := [][2]int16{{500, 0}, {700, 20}, {300, 10}}
	for _, m := range metrics {
		advBuf := make([]byte, 2)
		lsbBuf := make([]byte, 2)
		binary.BigEndian.PutUint16(advBuf, uint16(m[0]))
		binary.BigEndian.PutUint16(lsbBuf, uint16(m[1]))
		hmtx = append(hmtx, advBuf...)
		hmtx = append(hmtx, lsbBuf...)
	}

	var glyf []byte
	appendI16 := func(v int16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, uint16(v))
		glyf = append(glyf, buf...)
	}
	appendU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		glyf = append(glyf, buf...)
	}

	// Glyph 1: rectangle for 'A'
	appendI16(1)   // numContours
	appendI16(0)   // xMin
	appendI16(0)   // yMin
	appendI16(600) // xMax
	appendI16(700) // yMax
	appendU16(3)   // endPts[0]
	appendU16(0)   // instruction length
	for i := 0; i < 4; i++ {
		glyf = append(glyf, 0x01) // on-curve
	}
	for _, dx := range []int16{0, 600, 0, -600} {
		appendI16(dx)
	}
	for _, dy := range []int16{0, 0, 700, -700} {
		appendI16(dy)
	}
	glyph1Len := uint32(len(glyf))

	var loca []byte
	appendLocaU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		loca = append(loca, buf...)
	}
	appendLocaU32(0)
	appendLocaU32(0)
	appendLocaU32(glyph1Len)
	appendLocaU32(glyph1Len)

	var cmap []byte
	appendCmapU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		cmap = append(cmap, buf...)
	}
	appendCmapU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		cmap = append(cmap, buf...)
	}

	appendCmapU16(0)  // version
	appendCmapU16(1)  // numSubtables
	appendCmapU16(3)  // platformID = 3
	appendCmapU16(10) // encodingID = 10 (Format 12)
	appendCmapU32(12) // subOffset = 12

	// Format 12 subtable
	appendCmapU16(12) // format
	appendCmapU16(0)  // reserved
	appendCmapU32(40) // length
	appendCmapU32(0)  // language
	appendCmapU32(2)  // numGroups
	// Group 1: 'A' -> gid 1
	appendCmapU32(65)
	appendCmapU32(65)
	appendCmapU32(1)
	// Group 2: 'i' -> gid 2
	appendCmapU32(105)
	appendCmapU32(105)
	appendCmapU32(2)

	nameTbl := []byte{
		0, 0, // format
		0, 1, // count = 1
		0, 18, // stringOffset = 18
		// record 0
		0, 3, // platformID = 3 (Windows)
		0, 1, // encodingID = 1
		0, 0, // languageID = 0
		0, 1, // nameID = 1 (Family)
		0, 8, // length = 8
		0, 0, // offset = 0
		// UTF-16 string "Test"
		0, 'T', 0, 'e', 0, 's', 0, 't',
	}

	type tblEntry struct {
		tag  [4]byte
		data []byte
	}
	tbls := []tblEntry{
		{[4]byte{'c', 'm', 'a', 'p'}, cmap},
		{[4]byte{'g', 'l', 'y', 'f'}, glyf},
		{[4]byte{'h', 'e', 'a', 'd'}, head},
		{[4]byte{'h', 'h', 'e', 'a'}, hhea},
		{[4]byte{'h', 'm', 't', 'x'}, hmtx},
		{[4]byte{'l', 'o', 'c', 'a'}, loca},
		{[4]byte{'m', 'a', 'x', 'p'}, maxp},
		{[4]byte{'n', 'a', 'm', 'e'}, nameTbl},
	}

	var sfnt []byte
	appendSfntU16 := func(v uint16) {
		buf := make([]byte, 2)
		binary.BigEndian.PutUint16(buf, v)
		sfnt = append(sfnt, buf...)
	}
	appendSfntU32 := func(v uint32) {
		buf := make([]byte, 4)
		binary.BigEndian.PutUint32(buf, v)
		sfnt = append(sfnt, buf...)
	}

	appendSfntU32(0x00010000) // TrueType signature
	appendSfntU16(uint16(len(tbls)))
	appendSfntU16(128)
	appendSfntU16(3)
	appendSfntU16(0)

	bodyOffset := 12 + len(tbls)*16
	currentOff := bodyOffset
	for _, t := range tbls {
		sfnt = append(sfnt, t.tag[:]...)
		appendSfntU32(0) // checksum
		appendSfntU32(uint32(currentOff))
		appendSfntU32(uint32(len(t.data)))
		currentOff += (len(t.data) + 3) &^ 3 // 4-byte aligned
	}

	for _, t := range tbls {
		sfnt = append(sfnt, t.data...)
		pad := (4 - (len(t.data) % 4)) % 4
		for p := 0; p < pad; p++ {
			sfnt = append(sfnt, 0)
		}
	}

	return sfnt
}
