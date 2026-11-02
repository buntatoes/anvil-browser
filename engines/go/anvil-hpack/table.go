// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

import "bytes"

// EntrySize computes the RFC 7541 §4.1 accounting size of a header field:
// len(name) + len(value) + 32 bytes.
func EntrySize(name, value []byte) int {
	return len(name) + len(value) + 32
}

// StaticEntry looks up 1-based index (1..61) in the RFC 7541 static table.
func StaticEntry(index int) (string, string, bool) {
	if index < 1 || index > StaticTableLen {
		return "", "", false
	}
	pair := staticTable[index-1]
	return pair.name, pair.value, true
}

type tableEntry struct {
	name  []byte
	value []byte
}

// DynamicTable holds the FIFO dynamic table for one direction of an HTTP/2 connection.
// Newest entries are at index 0 of entries (corresponding to combined HPACK index 62).
type DynamicTable struct {
	entries []tableEntry
	size    int
	maxSize int
}

// NewDynamicTable creates an empty dynamic table that may grow up to maxSize bytes.
func NewDynamicTable(maxSize int) *DynamicTable {
	return &DynamicTable{
		entries: nil,
		size:    0,
		maxSize: maxSize,
	}
}

// Size returns the current accounted byte size of the dynamic table.
func (dt *DynamicTable) Size() int {
	return dt.size
}

// MaxSize returns the maximum byte capacity of the dynamic table.
func (dt *DynamicTable) MaxSize() int {
	return dt.maxSize
}

// Len returns the number of entries currently in the dynamic table.
func (dt *DynamicTable) Len() int {
	return len(dt.entries)
}

// IsEmpty reports whether the dynamic table has zero entries.
func (dt *DynamicTable) IsEmpty() bool {
	return len(dt.entries) == 0
}

// SetMaxSize updates the dynamic table capacity and evicts oldest entries until it fits.
func (dt *DynamicTable) SetMaxSize(maxSize int) {
	dt.maxSize = maxSize
	dt.evictTo(maxSize)
}

// Insert adds a header entry at the front of the dynamic table after evicting
// older entries to fit. An entry larger than MaxSize clears the table.
func (dt *DynamicTable) Insert(name, value []byte) {
	sz := EntrySize(name, value)
	if sz > dt.maxSize {
		dt.entries = dt.entries[:0]
		dt.size = 0
		return
	}
	dt.evictTo(dt.maxSize - sz)
	nCopy := make([]byte, len(name))
	vCopy := make([]byte, len(value))
	copy(nCopy, name)
	copy(vCopy, value)
	dt.entries = append(dt.entries, tableEntry{})
	copy(dt.entries[1:], dt.entries[:len(dt.entries)-1])
	dt.entries[0] = tableEntry{name: nCopy, value: vCopy}
	dt.size += sz
}

// Get retrieves the entry at combined 1-based HPACK index (1..61 static, 62+ dynamic).
func (dt *DynamicTable) Get(index int) ([]byte, []byte, bool) {
	if index <= 0 {
		return nil, nil, false
	}
	if name, value, ok := StaticEntry(index); ok {
		return []byte(name), []byte(value), true
	}
	dynIdx := index - StaticTableLen - 1
	if dynIdx < 0 || dynIdx >= len(dt.entries) {
		return nil, nil, false
	}
	e := dt.entries[dynIdx]
	return e.name, e.value, true
}

// Find searches the static and dynamic tables for the best combined index for (name, value).
// Returns (index, exactMatch, found).
func (dt *DynamicTable) Find(name, value []byte) (int, bool, bool) {
	nameMatch := 0
	for i, pair := range staticTable {
		if len(pair.name) == len(name) && string(name) == pair.name {
			if len(pair.value) == len(value) && string(value) == pair.value {
				return i + 1, true, true
			}
			if nameMatch == 0 {
				nameMatch = i + 1
			}
		}
	}
	for i, e := range dt.entries {
		if bytes.Equal(e.name, name) {
			idx := StaticTableLen + 1 + i
			if bytes.Equal(e.value, value) {
				return idx, true, true
			}
			if nameMatch == 0 {
				nameMatch = idx
			}
		}
	}
	if nameMatch != 0 {
		return nameMatch, false, true
	}
	return 0, false, false
}

// EntriesSnapshot returns a copy of the current dynamic table entries (newest first).
func (dt *DynamicTable) EntriesSnapshot() []Header {
	out := make([]Header, len(dt.entries))
	for i, e := range dt.entries {
		out[i] = NewHeaderBytes(e.name, e.value)
	}
	return out
}

func (dt *DynamicTable) evictTo(limit int) {
	for dt.size > limit && len(dt.entries) > 0 {
		last := dt.entries[len(dt.entries)-1]
		dt.size -= EntrySize(last.name, last.value)
		dt.entries = dt.entries[:len(dt.entries)-1]
	}
	if len(dt.entries) == 0 {
		dt.size = 0
	}
}
