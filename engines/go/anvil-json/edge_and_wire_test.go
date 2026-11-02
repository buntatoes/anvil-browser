// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anviljson

import (
	"strings"
	"testing"
)

func TestAllEscapesAndControlCharacters(t *testing.T) {
	raw := "tab\tnewline\ncarriage\rform\fback\bquote\"slash\\ctrl\x00\x01\x1f"
	escaped := EscapeString(raw)
	if !strings.Contains(escaped, `\t`) || !strings.Contains(escaped, `\n`) ||
		!strings.Contains(escaped, `\r`) || !strings.Contains(escaped, `\f`) ||
		!strings.Contains(escaped, `\b`) || !strings.Contains(escaped, `\u0000`) ||
		!strings.Contains(escaped, `\u001f`) {
		t.Fatalf("unexpected escaped string: %s", escaped)
	}
	parsed, err := Parse(escaped)
	if err != nil {
		t.Fatalf("failed to parse escaped string: %v", err)
	}
	if s, ok := parsed.AsStr(); !ok || s != raw {
		t.Fatalf("roundtrip mismatch: got %q, want %q", s, raw)
	}

	slashDoc, err := Parse(`"a\/b"`)
	if err != nil {
		t.Fatal(err)
	}
	if s, _ := slashDoc.AsStr(); s != "a/b" {
		t.Fatalf("expected a/b, got %q", s)
	}
}

func TestNumericEdgeCasesAndAccessors(t *testing.T) {
	cases := []struct {
		src   string
		want  float64
		i64Ok bool
		u64Ok bool
	}{
		{"0", 0, true, true},
		{"-0", 0, true, true},
		{"1000000", 1000000, true, true},
		{"-99999", -99999, true, false},
		{"3.14159265", 3.14159265, false, false},
		{"1.5e-4", 0.00015, false, false},
		{"2E+3", 2000, true, true},
	}
	for _, tc := range cases {
		v, err := Parse(tc.src)
		if err != nil {
			t.Fatalf("Parse(%q) error: %v", tc.src, err)
		}
		got, ok := v.AsF64()
		if !ok || got != tc.want {
			t.Fatalf("Parse(%q) = %v, want %v", tc.src, got, tc.want)
		}
		if _, ok := v.AsI64(); ok != tc.i64Ok {
			t.Fatalf("Parse(%q).AsI64() ok=%v, want %v", tc.src, ok, tc.i64Ok)
		}
		if _, ok := v.AsU64(); ok != tc.u64Ok {
			t.Fatalf("Parse(%q).AsU64() ok=%v, want %v", tc.src, ok, tc.u64Ok)
		}
	}

	invalidNumbers := []string{
		"-",
		"+1",
		"00",
		"-01",
		"1.e5",
		"1e+",
		"1e-",
		".5",
	}
	for _, bad := range invalidNumbers {
		if _, err := Parse(bad); err == nil {
			t.Fatalf("expected error for invalid number %q", bad)
		}
	}
}

func TestObjectAndArrayMutationHelpers(t *testing.T) {
	obj := EmptyObject()
	if !obj.IsEmpty() || obj.Len() != 0 {
		t.Fatal("expected empty object")
	}
	if !obj.Set("title", StringVal("Anvil")) {
		t.Fatal("Set failed")
	}
	arr := EmptyArray()
	if !arr.Push(Number(10)) || !arr.Push(Bool(true)) {
		t.Fatal("Push failed")
	}
	obj.Set("items", arr)
	if obj.Len() != 2 || obj.IsEmpty() {
		t.Fatalf("expected len 2, got %d", obj.Len())
	}

	cloned := obj.Clone()
	if !cloned.Equal(obj) {
		t.Fatal("Clone should equal original")
	}
	if itemsMut, ok := cloned.Get("items"); ok && itemsMut != nil {
		itemsMut.Push(StringVal("mutated"))
	}
	if cloned.Equal(obj) {
		t.Fatal("mutated clone should not equal original")
	}
}

func TestRFC6901PointerEdgeCasesAndErrors(t *testing.T) {
	doc, err := Parse(`{"items":["zero","one","two"],"nested":{"key":"val"}}`)
	if err != nil {
		t.Fatal(err)
	}
	// Valid nested pointer
	v, err := doc.ResolvePointer("/nested/key")
	if err != nil {
		t.Fatal(err)
	}
	if s, _ := v.AsStr(); s != "val" {
		t.Fatalf("expected val, got %q", s)
	}

	// Invalid pointers
	badPointers := []string{
		"no-leading-slash",
		"/items/01",
		"/items/99",
		"/items/notanumber",
		"/nested/missing",
		"/items/0/sub",
		"/~2",
		"/~",
	}
	for _, ptr := range badPointers {
		if _, err := doc.ResolvePointer(ptr); err == nil {
			t.Fatalf("expected error for pointer %q", ptr)
		}
	}
}

func TestBinaryWireRejectsMalformedPayloads(t *testing.T) {
	pos := 0
	if _, err := DecodeBinaryValue(nil, &pos); err == nil {
		t.Fatal("expected error on empty binary wire")
	}
	// Unknown tag
	pos = 0
	if _, err := DecodeBinaryValue([]byte{99}, &pos); err == nil {
		t.Fatal("expected error on unknown wire tag")
	}
	// Truncated number
	pos = 0
	if _, err := DecodeBinaryValue([]byte{wireTagNumber, 1, 2, 3}, &pos); err == nil {
		t.Fatal("expected error on truncated number wire")
	}
	// Truncated string
	pos = 0
	if _, err := DecodeBinaryValue([]byte{wireTagString, 10, 0, 0, 0, 'a', 'b'}, &pos); err == nil {
		t.Fatal("expected error on truncated string wire")
	}
}

func TestParseBytesRejectsInvalidUTF8AndOversizedInput(t *testing.T) {
	if _, err := ParseBytes([]byte{0xff, 0xfe, 0xfd}); err == nil {
		t.Fatal("expected UTF-8 error")
	}
	if _, err := ParseWithLimits(`{"a":[1,2,3]}`, 1, 1024); err == nil {
		t.Fatal("expected depth limit error when maxDepth=1")
	}
	if _, err := ParseWithLimits(`{"a":123456789}`, 64, 5); err == nil {
		t.Fatal("expected byte limit error when maxBytes=5")
	}
}
