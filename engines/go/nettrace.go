// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilgo

import (
	"fmt"
	"strings"
)

// TraceSpan records one HTTP/1.1 or HTTP/2 request/response exchange in an
// Anvil Browser session for timeline and HAR 1.2 analysis.
type TraceSpan struct {
	StreamID         uint32
	Method           string
	Scheme           string
	Authority        string
	Path             string
	Protocol         string
	Status           int
	RawHeaderBytes   int
	HpackWireBytes   int
	ResponseBodySize int
	DurationMicros   int64
	KeyExchangeGroup string
	BlockedByShield  bool
}

// TraceAnalysis summarizes aggregate HTTP/2 multiplexing, HPACK header
// compression savings, post-quantum TLS 1.3 coverage, and Shields blocking.
type TraceAnalysis struct {
	TotalRequests      int
	Http2Count         int
	Http11Count        int
	BlockedCount       int
	PostQuantumCount   int
	TotalRawHeaderB    int
	TotalHpackWireB    int
	TotalResponseBodyB int
	TotalDurationUs    int64
	HeaderSavingsRatio float64
}

// IsPostQuantumGroup reports whether a TLS 1.3 group name includes ML-KEM
// hybrid key exchange (such as X25519MLKEM768 or SecP256r1MLKEM768).
func IsPostQuantumGroup(group string) bool {
	upper := strings.ToUpper(strings.TrimSpace(group))
	return strings.Contains(upper, "MLKEM") || strings.Contains(upper, "ML-KEM")
}

// AnalyzeNetworkTrace aggregates a slice of TraceSpan exchanges into a TraceAnalysis.
func AnalyzeNetworkTrace(spans []TraceSpan) TraceAnalysis {
	var out TraceAnalysis
	out.TotalRequests = len(spans)
	for _, s := range spans {
		if s.BlockedByShield {
			out.BlockedCount++
			continue
		}
		proto := strings.ToLower(strings.TrimSpace(s.Protocol))
		if proto == "h2" || proto == "http/2" || proto == "http/2.0" {
			out.Http2Count++
		} else {
			out.Http11Count++
		}
		if IsPostQuantumGroup(s.KeyExchangeGroup) {
			out.PostQuantumCount++
		}
		if s.RawHeaderBytes > 0 {
			out.TotalRawHeaderB += s.RawHeaderBytes
		}
		if s.HpackWireBytes > 0 {
			out.TotalHpackWireB += s.HpackWireBytes
		}
		if s.ResponseBodySize > 0 {
			out.TotalResponseBodyB += s.ResponseBodySize
		}
		if s.DurationMicros > 0 {
			out.TotalDurationUs += s.DurationMicros
		}
	}
	if out.TotalRawHeaderB > 0 && out.TotalHpackWireB <= out.TotalRawHeaderB {
		saved := float64(out.TotalRawHeaderB - out.TotalHpackWireB)
		out.HeaderSavingsRatio = saved / float64(out.TotalRawHeaderB)
	}
	return out
}

// ExportHAR12 formats a slice of TraceSpan exchanges as an RFC 8259 JSON
// HTTP Archive (HAR 1.2) document suitable for inspection or archival.
func ExportHAR12(spans []TraceSpan) string {
	var b strings.Builder
	b.WriteString("{\n")
	b.WriteString("  \"log\": {\n")
	b.WriteString("    \"version\": \"1.2\",\n")
	fmt.Fprintf(&b, "    \"creator\": {\"name\": \"Anvil Browser\", \"version\": %q},\n", BrowserVersion)
	b.WriteString("    \"entries\": [\n")
	for i, s := range spans {
		url := fmt.Sprintf("%s://%s%s", s.Scheme, s.Authority, s.Path)
		ms := float64(s.DurationMicros) / 1000.0
		b.WriteString("      {\n")
		fmt.Fprintf(&b, "        \"streamId\": %d,\n", s.StreamID)
		fmt.Fprintf(&b, "        \"time\": %.3f,\n", ms)
		fmt.Fprintf(&b, "        \"_keyExchange\": %q,\n", s.KeyExchangeGroup)
		fmt.Fprintf(&b, "        \"_blockedByShield\": %v,\n", s.BlockedByShield)
		b.WriteString("        \"request\": {\n")
		fmt.Fprintf(&b, "          \"method\": %q,\n", s.Method)
		fmt.Fprintf(&b, "          \"url\": %q,\n", url)
		fmt.Fprintf(&b, "          \"httpVersion\": %q,\n", s.Protocol)
		fmt.Fprintf(&b, "          \"headersSize\": %d\n", s.HpackWireBytes)
		b.WriteString("        },\n")
		b.WriteString("        \"response\": {\n")
		fmt.Fprintf(&b, "          \"status\": %d,\n", s.Status)
		fmt.Fprintf(&b, "          \"httpVersion\": %q,\n", s.Protocol)
		fmt.Fprintf(&b, "          \"headersSize\": %d,\n", s.RawHeaderBytes)
		fmt.Fprintf(&b, "          \"bodySize\": %d\n", s.ResponseBodySize)
		b.WriteString("        }\n")
		if i+1 < len(spans) {
			b.WriteString("      },\n")
		} else {
			b.WriteString("      }\n")
		}
	}
	b.WriteString("    ]\n")
	b.WriteString("  }\n")
	b.WriteString("}")
	return b.String()
}
