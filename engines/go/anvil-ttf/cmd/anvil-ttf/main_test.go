// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package main

import (
	"bytes"
	"encoding/binary"
	"io"
	"math"
	"testing"
)

func TestWorkerIPCProtocol(t *testing.T) {
	ttfData := buildTestSyntheticTTF()

	inBuf := new(bytes.Buffer)
	outBuf := new(bytes.Buffer)
	var lenBuf [4]byte

	// 1. Opcode 0x01 (OP_LOAD_FONT)
	inBuf.WriteByte(opLoadFont)
	binary.LittleEndian.PutUint32(lenBuf[:], uint32(len(ttfData)))
	inBuf.Write(lenBuf[:])
	inBuf.Write(ttfData)

	// 2. Opcode 0x02 (OP_GLYPH_METRICS for 'A' = 65)
	inBuf.WriteByte(opGlyphMetrics)
	binary.LittleEndian.PutUint32(lenBuf[:], 4)
	inBuf.Write(lenBuf[:])
	var charBuf [4]byte
	binary.LittleEndian.PutUint32(charBuf[:], 65)
	inBuf.Write(charBuf[:])

	// 3. Opcode 0x03 (OP_RASTERIZE_GLYPH: 'A', size 20.0, phase 0, bold 0, italic 0)
	inBuf.WriteByte(opRasterizeGlyph)
	binary.LittleEndian.PutUint32(lenBuf[:], 11)
	inBuf.Write(lenBuf[:])
	var rastPayload [11]byte
	binary.LittleEndian.PutUint32(rastPayload[0:4], 65)
	binary.LittleEndian.PutUint32(rastPayload[4:8], math.Float32bits(20.0))
	rastPayload[8] = 0  // phase
	rastPayload[9] = 0  // bold
	rastPayload[10] = 0 // italic
	inBuf.Write(rastPayload[:])

	// 4. Opcode 0x04 (OP_RASTERIZE_LCD: 'A', size 20.0)
	inBuf.WriteByte(opRasterizeLCD)
	binary.LittleEndian.PutUint32(lenBuf[:], 8)
	inBuf.Write(lenBuf[:])
	var lcdPayload [8]byte
	binary.LittleEndian.PutUint32(lcdPayload[0:4], 65)
	binary.LittleEndian.PutUint32(lcdPayload[4:8], math.Float32bits(20.0))
	inBuf.Write(lcdPayload[:])

	// 5. Opcode 0xFF (OP_SHUTDOWN)
	inBuf.WriteByte(opShutdown)
	binary.LittleEndian.PutUint32(lenBuf[:], 0)
	inBuf.Write(lenBuf[:])

	// Run genuine IPC server loop
	serveWorker(inBuf, outBuf)

	readResponse := func(r io.Reader) (byte, []byte, error) {
		var hdr [5]byte
		if _, err := io.ReadFull(r, hdr[:]); err != nil {
			return 0, nil, err
		}
		status := hdr[0]
		pLen := binary.LittleEndian.Uint32(hdr[1:5])
		payload := make([]byte, pLen)
		if pLen > 0 {
			if _, err := io.ReadFull(r, payload); err != nil {
				return 0, nil, err
			}
		}
		return status, payload, nil
	}

	// 1. Verify LoadFont response
	status, resp, err := readResponse(outBuf)
	if err != nil {
		t.Fatalf("Failed to read LoadFont response: %v", err)
	}
	if status != 0x00 {
		t.Fatalf("LoadFont expected status 0x00, got 0x%02X: %s", status, string(resp))
	}
	if len(resp) != 8 {
		t.Fatalf("LoadFont expected 8-byte payload, got %d", len(resp))
	}
	unitsPerEm := binary.LittleEndian.Uint16(resp[0:2])
	if unitsPerEm != 1000 {
		t.Errorf("Expected unitsPerEm 1000, got %d", unitsPerEm)
	}

	// 2. Verify GlyphMetrics response
	status, resp, err = readResponse(outBuf)
	if err != nil {
		t.Fatalf("Failed to read GlyphMetrics response: %v", err)
	}
	if status != 0x00 {
		t.Fatalf("GlyphMetrics expected status 0x00, got 0x%02X: %s", status, string(resp))
	}
	if len(resp) != 12 {
		t.Fatalf("GlyphMetrics expected 12-byte payload, got %d", len(resp))
	}
	adv := binary.LittleEndian.Uint16(resp[0:2])
	if adv != 700 {
		t.Errorf("Expected advance 700, got %d", adv)
	}

	// 3. Verify RasterizeGlyph response
	status, resp, err = readResponse(outBuf)
	if err != nil {
		t.Fatalf("Failed to read RasterizeGlyph response: %v", err)
	}
	if status != 0x00 {
		t.Fatalf("RasterizeGlyph expected status 0x00, got 0x%02X: %s", status, string(resp))
	}
	if len(resp) < 20 {
		t.Fatalf("RasterizeGlyph response too short: %d", len(resp))
	}
	w := binary.LittleEndian.Uint32(resp[0:4])
	h := binary.LittleEndian.Uint32(resp[4:8])
	if w == 0 || h == 0 {
		t.Errorf("RasterizeGlyph expected non-zero dimensions, got %dx%d", w, h)
	}
	if len(resp) != int(20+w*h) {
		t.Errorf("Expected payload length %d, got %d", 20+w*h, len(resp))
	}

	// 4. Verify RasterizeLCD response
	status, resp, err = readResponse(outBuf)
	if err != nil {
		t.Fatalf("Failed to read RasterizeLCD response: %v", err)
	}
	if status != 0x00 {
		t.Fatalf("RasterizeLCD expected status 0x00, got 0x%02X: %s", status, string(resp))
	}
	if len(resp) < 20 {
		t.Fatalf("RasterizeLCD response too short: %d", len(resp))
	}
	lw := binary.LittleEndian.Uint32(resp[0:4])
	lh := binary.LittleEndian.Uint32(resp[4:8])
	if lw == 0 || lh == 0 {
		t.Errorf("RasterizeLCD expected non-zero dimensions, got %dx%d", lw, lh)
	}
	if len(resp) != int(20+lw*lh*3) {
		t.Errorf("Expected LCD payload length %d, got %d", 20+lw*lh*3, len(resp))
	}

	// 5. Verify Shutdown response
	status, resp, err = readResponse(outBuf)
	if err != nil {
		t.Fatalf("Failed to read Shutdown response: %v", err)
	}
	if status != 0x00 || len(resp) != 0 {
		t.Errorf("Expected clean shutdown status 0x00 with 0 bytes, got 0x%02X len=%d", status, len(resp))
	}
}

func TestWorkerErrorHandling(t *testing.T) {
	inBuf := new(bytes.Buffer)
	outBuf := new(bytes.Buffer)
	var lenBuf [4]byte

	readResponse := func(r io.Reader) (byte, []byte, error) {
		var hdr [5]byte
		if _, err := io.ReadFull(r, hdr[:]); err != nil {
			return 0, nil, err
		}
		status := hdr[0]
		pLen := binary.LittleEndian.Uint32(hdr[1:5])
		payload := make([]byte, pLen)
		if pLen > 0 {
			if _, err := io.ReadFull(r, payload); err != nil {
				return 0, nil, err
			}
		}
		return status, payload, nil
	}

	// 1. Metrics request before font is loaded
	inBuf.WriteByte(opGlyphMetrics)
	binary.LittleEndian.PutUint32(lenBuf[:], 4)
	inBuf.Write(lenBuf[:])
	var charBuf [4]byte
	binary.LittleEndian.PutUint32(charBuf[:], 65)
	inBuf.Write(charBuf[:])

	// 2. Unsupported opcode
	inBuf.WriteByte(0xEE)
	binary.LittleEndian.PutUint32(lenBuf[:], 0)
	inBuf.Write(lenBuf[:])

	// 3. Shutdown
	inBuf.WriteByte(opShutdown)
	binary.LittleEndian.PutUint32(lenBuf[:], 0)
	inBuf.Write(lenBuf[:])

	serveWorker(inBuf, outBuf)

	// Check 1: no font loaded error
	status, resp, err := readResponse(outBuf)
	if err != nil || status != 0x01 {
		t.Errorf("Expected status 0x01 for uninitialized font, got 0x%02X", status)
	}
	if string(resp) != "no font loaded" {
		t.Errorf("Expected 'no font loaded', got %q", string(resp))
	}

	// Check 2: unsupported opcode error
	status, resp, err = readResponse(outBuf)
	if err != nil || status != 0x01 {
		t.Errorf("Expected status 0x01 for unsupported opcode, got 0x%02X", status)
	}

	// Check 3: clean shutdown
	status, resp, err = readResponse(outBuf)
	if err != nil || status != 0x00 {
		t.Errorf("Expected clean shutdown status 0x00, got 0x%02X", status)
	}
	_ = resp
}
