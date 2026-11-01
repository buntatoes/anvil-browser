// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

// Package anvilgo implements the in-tree Go engine subsystem (`engines/go`) for
// Anvil Browser (`v.24.5.1`).
//
// In addition to the standalone Go repositories (`anvil-json` v.04,
// `anvil-compress` v.04, `anvil-hpack` v.02, `anvil-pqc` v.03, `anvil-gpu` v.12,
// `anvil-video` v.11, and `anvil-engine` v.24.5.1), this in-tree package provides:
//   - Go subsystem manifest and runtime verification (`VerifyGoSubsystems`,
//     `FormatDiagnosticsJSON`) surfaced on `anvil://version`.
//   - Deterministic profile/diagnostic bundle archiving (`PackBundle`,
//     `UnpackBundle`, `VerifyBundleDigest`) with FNV-1a 64-bit and IEEE CRC-32
//     integrity seals.
//   - HTTP/1.1 & HTTP/2 network trace and HAR 1.2 timeline analysis
//     (`AnalyzeNetworkTrace`, `ExportHAR12`) for inspecting header compression
//     savings, stream multiplexing, and post-quantum TLS 1.3 handshakes.
package anvilgo

import (
	"fmt"
	"runtime"
	"strings"
)

// BrowserVersion is the synchronized Anvil Browser release version.
const BrowserVersion = "0.24.5.1"

// EngineName is the identifier for the in-tree Go engine layer.
const EngineName = "AnvilGoEngine"

// SubsystemDescriptor describes a Go-powered component in the Anvil ecosystem.
type SubsystemDescriptor struct {
	Name        string
	Repository  string
	Version     string
	Spec        string
	MaxCapBytes int
	Features    []string
}

// EngineReport summarizes the health and configuration of all Go subsystems.
type EngineReport struct {
	EngineName    string
	BrowserVer    string
	GoRuntime     string
	GOOS          string
	GOARCH        string
	CgoEnabled    bool
	Subsystems    []SubsystemDescriptor
	SelfTestClean bool
}

// RegisteredSubsystems returns the canonical table of Go engines linked into Anvil Browser.
func RegisteredSubsystems() []SubsystemDescriptor {
	return []SubsystemDescriptor{
		{
			Name:        "anvil-json",
			Repository:  "github.com/buntatoes/anvil-json",
			Version:     "0.4.0",
			Spec:        "RFC 8259 + RFC 6901 JSON Pointer + NDJSON",
			MaxCapBytes: 16 * 1024 * 1024,
			Features: []string{
				"RFC 8259 recursive-descent parser (64-level depth cap)",
				"UTF-16 surrogate pair decoding & lone-surrogate rejection",
				"RFC 6901 JSON Pointer resolution & token escaping",
				"NDJSON batch parsing & lossless binary value wire framing",
			},
		},
		{
			Name:        "anvil-compress",
			Repository:  "github.com/buntatoes/anvil-compress",
			Version:     "0.4.0",
			Spec:        "RFC 1951 DEFLATE + RFC 1950 Zlib + RFC 1952 Gzip",
			MaxCapBytes: 32 * 1024 * 1024,
			Features: []string{
				"32 KiB sliding-window LZ77 + fixed/stored Huffman DEFLATE",
				"Adler-32 (RFC 1950) & IEEE 802.3 CRC-32 (RFC 1952) integrity checks",
				"Concatenated multi-member gzip streams (RFC 1952 Section 2.2)",
				"Automatic stream format detection (DecompressAuto)",
			},
		},
		{
			Name:        "anvil-hpack",
			Repository:  "github.com/buntatoes/anvil-hpack",
			Version:     "0.2.0",
			Spec:        "RFC 7541 HPACK HTTP/2 Header Compression",
			MaxCapBytes: 64 * 1024,
			Features: []string{
				"61-entry static table + FIFO dynamic table with 32-byte RFC 7541 overhead",
				"256-symbol canonical Huffman encoder & strict EOS/padding validator",
				"Never-indexed sensitive header protection for cookie & authorization",
				"Stateful connection-scoped encoder/decoder worker handles",
			},
		},
		{
			Name:        "anvil-pqc",
			Repository:  "github.com/buntatoes/anvil-pqc",
			Version:     "0.3.0",
			Spec:        "NIST FIPS 203 ML-KEM-768 + RFC 7748 X25519 + X25519MLKEM768 + FIPS 202 SHA-3",
			MaxCapBytes: 16 * 1024 * 1024,
			Features: []string{
				"NIST FIPS 203 ML-KEM-768 keygen, encapsulation, and implicit-rejection decapsulation",
				"RFC 7748 X25519 constant-time Montgomery ladder with all-zero output check",
				"TLS 1.3 X25519MLKEM768 (0x11EC) hybrid key exchange",
				"NIST FIPS 202 Keccak-f[1600] SHA3-256, SHA3-512, SHAKE128, and SHAKE256",
			},
		},
		{
			Name:        "anvil-gpu",
			Repository:  "github.com/buntatoes/anvil-gpu",
			Version:     "0.12.0",
			Spec:        "2D SDF Compositor Passes + BT.709 NV12/I420 + 16x16 Tile Device",
			MaxCapBytes: 64 * 1024 * 1024,
			Features: []string{
				"Blurred rounded-box SDF shadows, linear/radial/SDF gradients, and glyph coverage",
				"BT.709 limited-range RGBA <-> NV12 and I420 planar color conversion",
				"Batched GpuSceneBatch execution and 16x16 tile rasterizer",
				"Shelf-packed RGBA8 TextureAtlas with nearest and bilinear sampling",
			},
		},
		{
			Name:        "anvil-video",
			Repository:  "github.com/buntatoes/anvil-video",
			Version:     "0.11.0",
			Spec:        "RFC 6716 Opus + RFC 6386 VP8 + VP9 + AV1 OBU + MP4/fMP4/WebM + CENC",
			MaxCapBytes: 64 * 1024 * 1024,
			Features: []string{
				"RFC 6386 VP8, VP9 Profile 0, AOMedia AV1 OBU, and ADCT/AVID video codecs",
				"RFC 6716 Opus (SILK LPC + CELT MDCT), lossless FLAC, IMA-ADPCM, and PCM16 audio",
				"ISOBMFF MP4, Fragmented MP4 (fMP4), and WebM/EBML container muxing/demuxing",
				"W3C EME ClearKey CENC AES-128-CTR decryption, MSE SourceBuffer, and WebCodecs",
			},
		},
		{
			Name:        "anvil-engine",
			Repository:  "github.com/buntatoes/anvil-engine",
			Version:     BrowserVersion,
			Spec:        "Forge HTML5 DOM + Selectors 4 + CSS Cascade + Block/Flex/Grid Layout + Paint + Raster + Script/WASM",
			MaxCapBytes: 64 * 1024 * 1024,
			Features: []string{
				"HTML5 arena DOM parser with Selectors Level 4 and Shadow DOM",
				"CSS Cascade Layers (@layer), nesting, custom properties, and math functions",
				"Block, inline, Flexbox, CSS Grid, and table layout with incremental caching",
				"DisplayList painter, RGBA8/BGRA8 rasterizer, and budgeted JS + WebAssembly 1.0 VM",
			},
		},
		{
			Name:        "anvil-go-engine",
			Repository:  "github.com/buntatoes/anvil-browser/engines/go",
			Version:     BrowserVersion,
			Spec:        "ANVBv1 Diagnostic Bundle + HAR 1.2 Network Trace Analyzer",
			MaxCapBytes: 64 * 1024 * 1024,
			Features: []string{
				"Deterministic ANVBv1 binary archive packer & unpacker",
				"Dual IEEE CRC-32 + FNV-1a 64-bit entry integrity verification",
				"HTTP/1.1 & HTTP/2 HAR 1.2 network timeline & HPACK savings analyzer",
				"Post-quantum X25519MLKEM768 key-exchange telemetry aggregator",
			},
		},
	}
}

// VerifyGoSubsystems runs an internal self-check across the in-tree Go engine
// primitives (bundle archiving, CRC-32/FNV-1a digests, and HAR 1.2 trace analysis).
func VerifyGoSubsystems() (EngineReport, error) {
	subsystems := RegisteredSubsystems()
	report := EngineReport{
		EngineName:    EngineName,
		BrowserVer:    BrowserVersion,
		GoRuntime:     runtime.Version(),
		GOOS:          runtime.GOOS,
		GOARCH:        runtime.GOARCH,
		CgoEnabled:    false,
		Subsystems:    subsystems,
		SelfTestClean: false,
	}

	// 1. Verify bundle roundtrip
	entries := []BundleEntry{
		{Path: "manifest.txt", Data: []byte("Anvil Browser Go Engine Self-Test")},
		{Path: "config/limits.json", Data: []byte(`{"max_depth":64,"max_decompressed":33554432}`)},
	}
	packed, err := PackBundle(entries)
	if err != nil {
		return report, fmt.Errorf("bundle pack self-test failed: %w", err)
	}
	unpacked, err := UnpackBundle(packed, DefaultMaxBundleBytes)
	if err != nil || len(unpacked) != 2 {
		return report, fmt.Errorf("bundle unpack self-test failed: %v", err)
	}

	// 2. Verify network trace analysis
	events := []TraceSpan{
		{
			StreamID:         1,
			Method:           "GET",
			Scheme:           "https",
			Authority:        "example.org",
			Path:             "/index.html",
			Protocol:         "h2",
			Status:           200,
			RawHeaderBytes:   420,
			HpackWireBytes:   98,
			ResponseBodySize: 16384,
			DurationMicros:   12400,
			KeyExchangeGroup: "X25519MLKEM768",
		},
	}
	summary := AnalyzeNetworkTrace(events)
	if summary.TotalRequests != 1 || summary.PostQuantumCount != 1 || summary.HeaderSavingsRatio <= 0.5 {
		return report, fmt.Errorf("network trace self-test failed: %+v", summary)
	}

	report.SelfTestClean = true
	return report, nil
}

// FormatDiagnosticsJSON serializes an EngineReport into deterministic RFC 8259 JSON.
func FormatDiagnosticsJSON(r EngineReport) string {
	var b strings.Builder
	b.WriteString("{\n")
	fmt.Fprintf(&b, "  \"engine\": %q,\n", r.EngineName)
	fmt.Fprintf(&b, "  \"browser_version\": %q,\n", r.BrowserVer)
	fmt.Fprintf(&b, "  \"go_runtime\": %q,\n", r.GoRuntime)
	fmt.Fprintf(&b, "  \"goos\": %q,\n", r.GOOS)
	fmt.Fprintf(&b, "  \"goarch\": %q,\n", r.GOARCH)
	fmt.Fprintf(&b, "  \"cgo_enabled\": %v,\n", r.CgoEnabled)
	fmt.Fprintf(&b, "  \"self_test_clean\": %v,\n", r.SelfTestClean)
	b.WriteString("  \"subsystems\": [\n")
	for i, sub := range r.Subsystems {
		b.WriteString("    {\n")
		fmt.Fprintf(&b, "      \"name\": %q,\n", sub.Name)
		fmt.Fprintf(&b, "      \"repository\": %q,\n", sub.Repository)
		fmt.Fprintf(&b, "      \"version\": %q,\n", sub.Version)
		fmt.Fprintf(&b, "      \"spec\": %q,\n", sub.Spec)
		fmt.Fprintf(&b, "      \"max_cap_bytes\": %d\n", sub.MaxCapBytes)
		if i+1 < len(r.Subsystems) {
			b.WriteString("    },\n")
		} else {
			b.WriteString("    }\n")
		}
	}
	b.WriteString("  ]\n")
	b.WriteString("}")
	return b.String()
}
