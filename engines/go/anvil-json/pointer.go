// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anviljson

import (
	"fmt"
	"strconv"
	"strings"
)

// PointerError describes an RFC 6901 JSON Pointer syntax or traversal failure.
type PointerError struct {
	Message string
	Token   string
}

func (e *PointerError) Error() string {
	if e.Token != "" {
		return fmt.Sprintf("%s (token %q)", e.Message, e.Token)
	}
	return e.Message
}

// ResolvePointer evaluates an RFC 6901 JSON Pointer against v.
func (v *Value) ResolvePointer(pointer string) (*Value, error) {
	return ResolvePointer(v, pointer)
}

// ResolvePointer evaluates an RFC 6901 JSON Pointer against doc.
//
// An empty pointer ("") resolves to doc itself. Non-empty pointers must begin
// with '/' and use "~1" for '/' and "~0" for '~'.
func ResolvePointer(doc *Value, pointer string) (*Value, error) {
	if doc == nil {
		return nil, &PointerError{Message: "document is nil"}
	}
	if pointer == "" {
		return doc, nil
	}
	if pointer[0] != '/' {
		return nil, &PointerError{Message: "JSON Pointer must be empty or start with '/'", Token: pointer}
	}

	parts := strings.Split(pointer[1:], "/")
	cur := doc
	for _, rawToken := range parts {
		token, err := unescapePointerToken(rawToken)
		if err != nil {
			return nil, err
		}
		switch cur.kind {
		case KindObject:
			next, ok := cur.Get(token)
			if !ok {
				return nil, &PointerError{Message: "object key not found", Token: token}
			}
			cur = next
		case KindArray:
			idx, err := parseArrayIndexToken(token)
			if err != nil {
				return nil, err
			}
			next, ok := cur.Index(idx)
			if !ok {
				return nil, &PointerError{Message: "array index out of bounds", Token: token}
			}
			cur = next
		default:
			return nil, &PointerError{Message: "cannot traverse scalar value", Token: token}
		}
	}
	return cur, nil
}

// FormatPointer encodes a slice of reference tokens into an RFC 6901 JSON Pointer string.
func FormatPointer(tokens []string) string {
	if len(tokens) == 0 {
		return ""
	}
	var b strings.Builder
	for _, tok := range tokens {
		b.WriteByte('/')
		for i := 0; i < len(tok); i++ {
			switch tok[i] {
			case '~':
				b.WriteString("~0")
			case '/':
				b.WriteString("~1")
			default:
				b.WriteByte(tok[i])
			}
		}
	}
	return b.String()
}

func unescapePointerToken(raw string) (string, error) {
	if strings.IndexByte(raw, '~') < 0 {
		return raw, nil
	}
	var b strings.Builder
	b.Grow(len(raw))
	for i := 0; i < len(raw); i++ {
		if raw[i] == '~' {
			if i+1 >= len(raw) {
				return "", &PointerError{Message: "dangling '~' escape in JSON Pointer", Token: raw}
			}
			i++
			switch raw[i] {
			case '0':
				b.WriteByte('~')
			case '1':
				b.WriteByte('/')
			default:
				return "", &PointerError{Message: "invalid '~' escape in JSON Pointer (expected ~0 or ~1)", Token: raw}
			}
		} else {
			b.WriteByte(raw[i])
		}
	}
	return b.String(), nil
}

func parseArrayIndexToken(token string) (int, error) {
	if token == "" || token == "-" {
		return 0, &PointerError{Message: "invalid array index token", Token: token}
	}
	if len(token) > 1 && token[0] == '0' {
		return 0, &PointerError{Message: "leading zero not allowed in JSON Pointer array index", Token: token}
	}
	for i := 0; i < len(token); i++ {
		if token[i] < '0' || token[i] > '9' {
			return 0, &PointerError{Message: "non-digit in JSON Pointer array index", Token: token}
		}
	}
	idx, err := strconv.Atoi(token)
	if err != nil || idx < 0 {
		return 0, &PointerError{Message: "array index overflow", Token: token}
	}
	return idx, nil
}

// ParseLines parses a Newline-Delimited JSON (NDJSON / JSON Lines) stream,
// skipping blank/whitespace-only lines.
func ParseLines(input string) ([]Value, error) {
	lines := strings.Split(input, "\n")
	var out []Value
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		v, err := Parse(trimmed)
		if err != nil {
			return nil, err
		}
		out = append(out, v)
	}
	return out, nil
}

// StringifyLines serializes a slice of JSON values into Newline-Delimited JSON (NDJSON).
func StringifyLines(values []Value) string {
	var b strings.Builder
	for i, v := range values {
		if i > 0 {
			b.WriteByte('\n')
		}
		writeCompact(&b, v)
	}
	return b.String()
}
