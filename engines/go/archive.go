// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilgo

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"strings"
)

// BundleMagic identifies an Anvil Go Engine v1 diagnostic/profile archive ("ANVB\x01\x00").
var BundleMagic = [6]byte{'A', 'N', 'V', 'B', 0x01, 0x00}

// DefaultMaxBundleBytes caps unpacked archive bytes at 64 MiB.
const DefaultMaxBundleBytes = 64 * 1024 * 1024

// MaxBundleEntries caps the number of files in one diagnostic archive.
const MaxBundleEntries = 4096

// MaxEntryPathLen caps the UTF-8 relative path length of an archive entry.
const MaxEntryPathLen = 512

// BundleEntry represents a single named file inside an ANVBv1 archive.
type BundleEntry struct {
	Path  string
	Data  []byte
	CRC32 uint32
	FNV1a uint64
}

// ComputeCRC32 calculates the IEEE 802.3 CRC-32 checksum of data.
func ComputeCRC32(data []byte) uint32 {
	crc := uint32(0xFFFFFFFF)
	for _, b := range data {
		crc ^= uint32(b)
		for bit := 0; bit < 8; bit++ {
			if crc&1 != 0 {
				crc = (crc >> 1) ^ 0xEDB88320
			} else {
				crc >>= 1
			}
		}
	}
	return ^crc
}

// ComputeFNV1a64 calculates the 64-bit FNV-1a non-cryptographic hash of data.
func ComputeFNV1a64(data []byte) uint64 {
	const (
		offset64 = 14695981039346656037
		prime64  = 1099511628211
	)
	hash := uint64(offset64)
	for _, b := range data {
		hash ^= uint64(b)
		hash *= prime64
	}
	return hash
}

// SanitizeEntryPath validates that an archive path is relative, UTF-8, free of
// directory traversal (".."), backslashes, or leading slashes.
func SanitizeEntryPath(p string) (string, error) {
	trimmed := strings.TrimSpace(p)
	if trimmed == "" {
		return "", errors.New("bundle entry path is empty")
	}
	if len(trimmed) > MaxEntryPathLen {
		return "", fmt.Errorf("bundle entry path length %d exceeds cap %d", len(trimmed), MaxEntryPathLen)
	}
	if strings.HasPrefix(trimmed, "/") || strings.HasPrefix(trimmed, "\\") || strings.Contains(trimmed, "\\") {
		return "", fmt.Errorf("bundle entry path %q must use relative forward slashes", trimmed)
	}
	if strings.Contains(trimmed, ":") || strings.Contains(trimmed, "\x00") {
		return "", fmt.Errorf("bundle entry path %q contains forbidden characters", trimmed)
	}
	parts := strings.Split(trimmed, "/")
	for _, part := range parts {
		if part == "" || part == "." || part == ".." {
			return "", fmt.Errorf("bundle entry path %q contains invalid segment %q", trimmed, part)
		}
	}
	return trimmed, nil
}

// PackBundle serializes a slice of BundleEntry records into a deterministic
// ANVBv1 binary container with per-entry CRC-32 + FNV-1a digests and a whole-file
// trailer seal.
func PackBundle(entries []BundleEntry) ([]byte, error) {
	if len(entries) > MaxBundleEntries {
		return nil, fmt.Errorf("bundle entry count %d exceeds cap %d", len(entries), MaxBundleEntries)
	}

	var out bytes.Buffer
	out.Write(BundleMagic[:])

	var countBuf [4]byte
	binary.LittleEndian.PutUint32(countBuf[:], uint32(len(entries)))
	out.Write(countBuf[:])

	seen := make(map[string]bool, len(entries))
	for _, entry := range entries {
		cleanPath, err := SanitizeEntryPath(entry.Path)
		if err != nil {
			return nil, err
		}
		if seen[cleanPath] {
			return nil, fmt.Errorf("duplicate bundle entry path %q", cleanPath)
		}
		seen[cleanPath] = true

		crc := ComputeCRC32(entry.Data)
		fnv := ComputeFNV1a64(entry.Data)

		var hdr [18]byte
		binary.LittleEndian.PutUint16(hdr[0:2], uint16(len(cleanPath)))
		binary.LittleEndian.PutUint32(hdr[2:6], uint32(len(entry.Data)))
		binary.LittleEndian.PutUint32(hdr[6:10], crc)
		binary.LittleEndian.PutUint64(hdr[10:18], fnv)

		out.Write(hdr[:])
		out.WriteString(cleanPath)
		out.Write(entry.Data)
	}

	bodyBytes := out.Bytes()
	var trailer [12]byte
	binary.LittleEndian.PutUint32(trailer[0:4], ComputeCRC32(bodyBytes))
	binary.LittleEndian.PutUint64(trailer[4:12], ComputeFNV1a64(bodyBytes))
	out.Write(trailer[:])

	return out.Bytes(), nil
}

// UnpackBundle verifies the whole-archive trailer seal, validates every entry's
// path and dual CRC-32/FNV-1a digests, and enforces maxTotalBytes across the
// unpacked payload.
func UnpackBundle(raw []byte, maxTotalBytes int) ([]BundleEntry, error) {
	if len(raw) < 6+4+12 {
		return nil, errors.New("bundle container is truncated")
	}
	if !bytes.Equal(raw[:6], BundleMagic[:]) {
		return nil, errors.New("bundle magic mismatch (expected ANVBv1)")
	}

	bodyEnd := len(raw) - 12
	body := raw[:bodyEnd]
	trailer := raw[bodyEnd:]

	wantCRC := binary.LittleEndian.Uint32(trailer[0:4])
	wantFNV := binary.LittleEndian.Uint64(trailer[4:12])
	if gotCRC := ComputeCRC32(body); gotCRC != wantCRC {
		return nil, fmt.Errorf("bundle trailer CRC-32 mismatch: got 0x%08x, want 0x%08x", gotCRC, wantCRC)
	}
	if gotFNV := ComputeFNV1a64(body); gotFNV != wantFNV {
		return nil, fmt.Errorf("bundle trailer FNV-1a mismatch: got 0x%016x, want 0x%016x", gotFNV, wantFNV)
	}

	count := int(binary.LittleEndian.Uint32(body[6:10]))
	if count < 0 || count > MaxBundleEntries {
		return nil, fmt.Errorf("bundle entry count %d out of range", count)
	}

	pos := 10
	totalBytes := 0
	entries := make([]BundleEntry, 0, count)
	seen := make(map[string]bool, count)

	for i := 0; i < count; i++ {
		if pos+18 > len(body) {
			return nil, errors.New("bundle truncated inside entry header")
		}
		pathLen := int(binary.LittleEndian.Uint16(body[pos : pos+2]))
		dataLen := int(binary.LittleEndian.Uint32(body[pos+2 : pos+6]))
		entryCRC := binary.LittleEndian.Uint32(body[pos+6 : pos+10])
		entryFNV := binary.LittleEndian.Uint64(body[pos+10 : pos+18])
		pos += 18

		if pathLen <= 0 || pathLen > MaxEntryPathLen || pos+pathLen > len(body) {
			return nil, errors.New("bundle entry path length invalid or truncated")
		}
		pathStr := string(body[pos : pos+pathLen])
		pos += pathLen

		cleanPath, err := SanitizeEntryPath(pathStr)
		if err != nil {
			return nil, err
		}
		if seen[cleanPath] {
			return nil, fmt.Errorf("duplicate bundle entry path %q", cleanPath)
		}
		seen[cleanPath] = true

		if dataLen < 0 || pos+dataLen > len(body) {
			return nil, errors.New("bundle entry payload truncated")
		}
		totalBytes += dataLen
		if totalBytes > maxTotalBytes {
			return nil, fmt.Errorf("bundle unpacked payload %d exceeds cap %d", totalBytes, maxTotalBytes)
		}

		data := make([]byte, dataLen)
		copy(data, body[pos:pos+dataLen])
		pos += dataLen

		if ComputeCRC32(data) != entryCRC || ComputeFNV1a64(data) != entryFNV {
			return nil, fmt.Errorf("bundle entry %q failed integrity check", cleanPath)
		}

		entries = append(entries, BundleEntry{
			Path:  cleanPath,
			Data:  data,
			CRC32: entryCRC,
			FNV1a: entryFNV,
		})
	}

	if pos != len(body) {
		return nil, errors.New("bundle contains trailing unparsed bytes before trailer")
	}
	return entries, nil
}
