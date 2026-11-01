// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anvilgo

import (
	"bytes"
	"strings"
	"testing"
)

func TestVerifyGoSubsystemsAndDiagnosticsJSON(t *testing.T) {
	report, err := VerifyGoSubsystems()
	if err != nil {
		t.Fatalf("VerifyGoSubsystems failed: %v", err)
	}
	if !report.SelfTestClean {
		t.Fatal("expected SelfTestClean=true")
	}
	if len(report.Subsystems) != 8 {
		t.Fatalf("expected 8 registered Go subsystems, got %d", len(report.Subsystems))
	}

	jsonOut := FormatDiagnosticsJSON(report)
	for _, needle := range []string{
		`"engine": "AnvilGoEngine"`,
		`"anvil-json"`,
		`"anvil-compress"`,
		`"anvil-hpack"`,
		`"anvil-pqc"`,
		`"anvil-gpu"`,
		`"anvil-video"`,
		`"anvil-engine"`,
		`"anvil-go-engine"`,
		`"self_test_clean": true`,
	} {
		if !strings.Contains(jsonOut, needle) {
			t.Fatalf("missing %q in diagnostics JSON:\n%s", needle, jsonOut)
		}
	}
}

func TestPackAndUnpackBundleRoundtripAndTamperRejection(t *testing.T) {
	entries := []BundleEntry{
		{Path: "profile/settings.json", Data: []byte(`{"https_only_mode":true}`)},
		{Path: "profile/bookmarks.json", Data: []byte(`[{"title":"Anvil","url":"https://example.org"}]`)},
		{Path: "trace/session.har", Data: bytes.Repeat([]byte("HAR-DATA-"), 128)},
	}

	packed, err := PackBundle(entries)
	if err != nil {
		t.Fatalf("PackBundle failed: %v", err)
	}

	unpacked, err := UnpackBundle(packed, DefaultMaxBundleBytes)
	if err != nil {
		t.Fatalf("UnpackBundle failed: %v", err)
	}
	if len(unpacked) != len(entries) {
		t.Fatalf("expected %d entries, got %d", len(entries), len(unpacked))
	}
	for i := range entries {
		if unpacked[i].Path != entries[i].Path || !bytes.Equal(unpacked[i].Data, entries[i].Data) {
			t.Fatalf("entry[%d] mismatch", i)
		}
	}

	// Tamper with payload byte -> trailer or entry checksum must reject
	corrupt := append([]byte(nil), packed...)
	corrupt[len(corrupt)/2] ^= 0xFF
	if _, err := UnpackBundle(corrupt, DefaultMaxBundleBytes); err == nil {
		t.Fatal("expected error on tampered bundle")
	}

	// Enforce byte cap
	if _, err := UnpackBundle(packed, 16); err == nil {
		t.Fatal("expected error when unpacked payload exceeds maxTotalBytes")
	}
}

func TestSanitizeEntryPathRejectsTraversalAndAbsolutePaths(t *testing.T) {
	badPaths := []string{
		"",
		"   ",
		"/etc/passwd",
		"\\windows\\system32",
		"../secret.txt",
		"profile/../../secret.txt",
		"profile/./file.txt",
		"C:/secret.txt",
		"profile//double.txt",
	}
	for _, p := range badPaths {
		if _, err := SanitizeEntryPath(p); err == nil {
			t.Fatalf("expected SanitizeEntryPath(%q) to fail", p)
		}
	}
}

func TestAnalyzeNetworkTraceAndExportHAR12(t *testing.T) {
	spans := []TraceSpan{
		{
			StreamID:         1,
			Method:           "GET",
			Scheme:           "https",
			Authority:        "example.org",
			Path:             "/",
			Protocol:         "h2",
			Status:           200,
			RawHeaderBytes:   500,
			HpackWireBytes:   125,
			ResponseBodySize: 8192,
			DurationMicros:   14500,
			KeyExchangeGroup: "X25519MLKEM768",
		},
		{
			StreamID:         3,
			Method:           "GET",
			Scheme:           "https",
			Authority:        "example.org",
			Path:             "/style.css",
			Protocol:         "h2",
			Status:           200,
			RawHeaderBytes:   300,
			HpackWireBytes:   75,
			ResponseBodySize: 2048,
			DurationMicros:   6200,
			KeyExchangeGroup: "X25519MLKEM768",
		},
		{
			StreamID:        0,
			Method:          "GET",
			Scheme:          "https",
			Authority:       "ads.tracker.example",
			Path:            "/pixel.gif",
			BlockedByShield: true,
		},
	}

	analysis := AnalyzeNetworkTrace(spans)
	if analysis.TotalRequests != 3 || analysis.Http2Count != 2 || analysis.BlockedCount != 1 || analysis.PostQuantumCount != 2 {
		t.Fatalf("unexpected analysis counts: %+v", analysis)
	}
	if analysis.HeaderSavingsRatio != 0.75 {
		t.Fatalf("expected 0.75 HPACK savings ratio, got %v", analysis.HeaderSavingsRatio)
	}

	har := ExportHAR12(spans)
	if !strings.Contains(har, `"version": "1.2"`) || !strings.Contains(har, `"X25519MLKEM768"`) {
		t.Fatalf("unexpected HAR output:\n%s", har)
	}
}
