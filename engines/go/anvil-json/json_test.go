// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package anviljson

import (
	"math"
	"strings"
	"testing"
)

func TestParsesPrimitives(t *testing.T) {
	v, err := Parse("null")
	if err != nil || !v.IsNull() {
		t.Fatalf("expected null, got %v, %v", v, err)
	}
	v, err = Parse("true")
	if err != nil {
		t.Fatal(err)
	}
	if b, ok := v.AsBool(); !ok || !b {
		t.Fatalf("expected true, got %v", v)
	}
	v, err = Parse("false")
	if err != nil {
		t.Fatal(err)
	}
	if b, ok := v.AsBool(); !ok || b {
		t.Fatalf("expected false, got %v", v)
	}
	v, err = Parse("-42.5e2")
	if err != nil {
		t.Fatal(err)
	}
	if n, ok := v.AsF64(); !ok || n != -4250.0 {
		t.Fatalf("expected -4250.0, got %v", n)
	}
	v, err = Parse(`"hello\nworld"`)
	if err != nil {
		t.Fatal(err)
	}
	if s, ok := v.AsStr(); !ok || s != "hello\nworld" {
		t.Fatalf("expected hello\\nworld, got %q", s)
	}
}

func TestDecodesSurrogatePairs(t *testing.T) {
	v, err := Parse(`"\uD83D\uDE00"`)
	if err != nil {
		t.Fatal(err)
	}
	s, ok := v.AsStr()
	if !ok || s != "😀" {
		t.Fatalf("expected 😀, got %q", s)
	}
}

func TestRejectsLoneSurrogates(t *testing.T) {
	if _, err := Parse(`"\uD83D"`); err == nil {
		t.Fatal("expected error for lone high surrogate")
	}
	if _, err := Parse(`"\uDE00"`); err == nil {
		t.Fatal("expected error for lone low surrogate")
	}
}

func TestParsesNestedObjectsAndArrays(t *testing.T) {
	doc, err := Parse(`{
		"name": "anvil",
		"version": 6,
		"features": ["tls", "grid", "canvas"],
		"nested": {"ok": true}
	}`)
	if err != nil {
		t.Fatal(err)
	}
	nameVal, ok := doc.Get("name")
	if !ok {
		t.Fatal("missing name")
	}
	if s, _ := nameVal.AsStr(); s != "anvil" {
		t.Fatalf("unexpected name %q", s)
	}
	verVal, _ := doc.Get("version")
	if u, ok := verVal.AsU64(); !ok || u != 6 {
		t.Fatalf("unexpected version %v", u)
	}
	featVal, _ := doc.Get("features")
	elem, ok := featVal.Index(1)
	if !ok {
		t.Fatal("missing index 1")
	}
	if s, _ := elem.AsStr(); s != "grid" {
		t.Fatalf("unexpected feature %q", s)
	}
	okVal, found := doc.GetPath([]string{"nested", "ok"})
	if !found {
		t.Fatal("missing nested.ok")
	}
	if b, _ := okVal.AsBool(); !b {
		t.Fatal("expected nested.ok == true")
	}
}

func TestRoundtripsCompactAndPretty(t *testing.T) {
	val, err := Parse(`{"a":[1,true,null,"x\"y"],"b":{}}`)
	if err != nil {
		t.Fatal(err)
	}
	compact := Stringify(val)
	if compact != `{"a":[1,true,null,"x\"y"],"b":{}}` {
		t.Fatalf("unexpected compact %s", compact)
	}
	pretty := StringifyPretty(val, 2)
	reparsed, err := Parse(pretty)
	if err != nil || !reparsed.Equal(val) {
		t.Fatalf("pretty roundtrip mismatch: %v", err)
	}
}

func TestPreservesDuplicateKeysAndUpdatesInPlace(t *testing.T) {
	val, err := Parse(`{"k":1,"other":2,"k":3}`)
	if err != nil {
		t.Fatal(err)
	}
	if Stringify(val) != `{"k":1,"other":2,"k":3}` {
		t.Fatalf("expected duplicate keys preserved, got %s", Stringify(val))
	}
	kVal, ok := val.Get("k")
	if !ok {
		t.Fatal("missing k")
	}
	if i, _ := kVal.AsI64(); i != 3 {
		t.Fatalf("expected last key 3, got %d", i)
	}
	val.Set("k", Number(9.0))
	if Stringify(val) != `{"k":9,"other":2}` {
		t.Fatalf("expected deduplicated update, got %s", Stringify(val))
	}
}

func TestEnforcesDepthAndMalformedRejections(t *testing.T) {
	deep := strings.Repeat("[", MaxDepth+1) + strings.Repeat("]", MaxDepth+1)
	if _, err := Parse(deep); err == nil {
		t.Fatal("expected depth cap error")
	}
	badInputs := []string{
		"",
		`{"a":1,}`,
		`[1,2,]`,
		`01`,
		`1.`,
		`1e`,
		"\"raw\nnewline\"",
		`true false`,
	}
	for _, sample := range badInputs {
		if _, err := Parse(sample); err == nil {
			t.Fatalf("expected error for %q", sample)
		}
	}
}

func TestNonFiniteNumberSerializesToNull(t *testing.T) {
	if Stringify(Number(math.NaN())) != "null" {
		t.Fatal("expected NaN -> null")
	}
	if Stringify(Number(math.Inf(1))) != "null" {
		t.Fatal("expected +Inf -> null")
	}
}

func TestRFC6901JsonPointer(t *testing.T) {
	doc, err := Parse(`{
		"foo": ["bar", "baz"],
		"": 0,
		"a/b": 1,
		"c%d": 2,
		"e^f": 3,
		"m~n": 8
	}`)
	if err != nil {
		t.Fatal(err)
	}
	root, err := ResolvePointer(&doc, "")
	if err != nil || !root.Equal(doc) {
		t.Fatalf("root pointer failed: %v", err)
	}
	v, err := ResolvePointer(&doc, "/foo/0")
	if err != nil {
		t.Fatal(err)
	}
	if s, _ := v.AsStr(); s != "bar" {
		t.Fatalf("expected bar, got %q", s)
	}
	v, err = ResolvePointer(&doc, "/a~1b")
	if err != nil {
		t.Fatal(err)
	}
	if i, _ := v.AsI64(); i != 1 {
		t.Fatalf("expected 1, got %d", i)
	}
	v, err = ResolvePointer(&doc, "/m~0n")
	if err != nil {
		t.Fatal(err)
	}
	if i, _ := v.AsI64(); i != 8 {
		t.Fatalf("expected 8, got %d", i)
	}
	if FormatPointer([]string{"a/b", "m~n"}) != "/a~1b/m~0n" {
		t.Fatalf("unexpected formatted pointer: %s", FormatPointer([]string{"a/b", "m~n"}))
	}
}

func TestNDJSONLines(t *testing.T) {
	lines, err := ParseLines("{\"id\":1}\n\n{\"id\":2}\n")
	if err != nil || len(lines) != 2 {
		t.Fatalf("unexpected ParseLines result: %v, %v", lines, err)
	}
	encoded := StringifyLines(lines)
	if encoded != "{\"id\":1}\n{\"id\":2}" {
		t.Fatalf("unexpected StringifyLines: %q", encoded)
	}
}

func TestBinaryValueWireRoundtrip(t *testing.T) {
	doc, err := Parse(`{"a":[1,true,false,null,"hello\nworld"],"dup":1,"dup":2}`)
	if err != nil {
		t.Fatal(err)
	}
	wire := EncodeBinaryValue(nil, doc)
	pos := 0
	decoded, err := DecodeBinaryValue(wire, &pos)
	if err != nil {
		t.Fatal(err)
	}
	if pos != len(wire) || !decoded.Equal(doc) {
		t.Fatalf("binary wire roundtrip mismatch: %v vs %v", decoded, doc)
	}
}
