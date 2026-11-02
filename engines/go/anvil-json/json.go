// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

// Package anviljson implements a zero-dependency, clean-room RFC 8259 JSON
// parser, value tree, compact/pretty serializer, RFC 6901 JSON Pointer
// resolver, and NDJSON batch processor.
//
// Security and resource limits:
//   - MaxDepth (64): caps nested arrays and objects.
//   - MaxInputBytes (16 MiB): caps input payload size before parsing.
package anviljson

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Version is the semantic version of anvil-json.
const Version = "0.4.0"

// MaxDepth is the default maximum nesting depth of arrays and objects.
const MaxDepth = 64

// MaxInputBytes is the default maximum input length in bytes (16 MiB).
const MaxInputBytes = 16 * 1024 * 1024

// Kind identifies the JSON value variant held by a Value.
type Kind uint8

const (
	KindNull Kind = iota
	KindBool
	KindNumber
	KindString
	KindArray
	KindObject
)

// String returns the human-readable name of the JSON value kind.
func (k Kind) String() string {
	switch k {
	case KindNull:
		return "null"
	case KindBool:
		return "boolean"
	case KindNumber:
		return "number"
	case KindString:
		return "string"
	case KindArray:
		return "array"
	case KindObject:
		return "object"
	default:
		return "unknown"
	}
}

// Member represents a single key-value pair in a JSON object.
// Object members are stored in a slice to preserve insertion order and
// duplicate keys.
type Member struct {
	Key   string
	Value Value
}

// Value represents any RFC 8259 JSON value.
type Value struct {
	kind    Kind
	boolean bool
	number  float64
	str     string
	array   []Value
	object  []Member
}

// Null constructs a JSON null value.
func Null() Value {
	return Value{kind: KindNull}
}

// Bool constructs a JSON boolean value.
func Bool(b bool) Value {
	return Value{kind: KindBool, boolean: b}
}

// Number constructs a JSON number value.
func Number(n float64) Value {
	return Value{kind: KindNumber, number: n}
}

// StringVal constructs a JSON string value.
func StringVal(s string) Value {
	return Value{kind: KindString, str: s}
}

// Array constructs a JSON array value from the given elements.
func Array(items ...Value) Value {
	copied := make([]Value, len(items))
	copy(copied, items)
	return Value{kind: KindArray, array: copied}
}

// Object constructs a JSON object value from the given ordered members.
func Object(members ...Member) Value {
	copied := make([]Member, len(members))
	copy(copied, members)
	return Value{kind: KindObject, object: copied}
}

// EmptyObject constructs an empty JSON object.
func EmptyObject() Value {
	return Value{kind: KindObject, object: nil}
}

// EmptyArray constructs an empty JSON array.
func EmptyArray() Value {
	return Value{kind: KindArray, array: nil}
}

// KindOf returns the Kind of this JSON value.
func (v Value) KindOf() Kind {
	return v.kind
}

// IsNull reports whether v is a JSON null value.
func (v Value) IsNull() bool {
	return v.kind == KindNull
}

// AsBool returns the boolean value if v is KindBool.
func (v Value) AsBool() (bool, bool) {
	if v.kind == KindBool {
		return v.boolean, true
	}
	return false, false
}

// AsF64 returns the float64 value if v is KindNumber.
func (v Value) AsF64() (float64, bool) {
	if v.kind == KindNumber {
		return v.number, true
	}
	return 0, false
}

// AsI64 converts a finite integral JSON number within int64 range.
func (v Value) AsI64() (int64, bool) {
	if v.kind != KindNumber || math.IsNaN(v.number) || math.IsInf(v.number, 0) {
		return 0, false
	}
	if math.Trunc(v.number) != v.number {
		return 0, false
	}
	if v.number < -9223372036854775808.0 || v.number >= 9223372036854775808.0 {
		return 0, false
	}
	return int64(v.number), true
}

// AsU64 converts a finite non-negative integral JSON number within uint64 range.
func (v Value) AsU64() (uint64, bool) {
	if v.kind != KindNumber || math.IsNaN(v.number) || math.IsInf(v.number, 0) {
		return 0, false
	}
	if v.number < 0.0 || math.Trunc(v.number) != v.number {
		return 0, false
	}
	if v.number >= 18446744073709551616.0 {
		return 0, false
	}
	return uint64(v.number), true
}

// AsStr returns the string contents if v is KindString.
func (v Value) AsStr() (string, bool) {
	if v.kind == KindString {
		return v.str, true
	}
	return "", false
}

// AsArray returns the slice of elements if v is KindArray.
func (v Value) AsArray() ([]Value, bool) {
	if v.kind == KindArray {
		return v.array, true
	}
	return nil, false
}

// AsMembers returns the slice of ordered object members if v is KindObject.
func (v Value) AsMembers() ([]Member, bool) {
	if v.kind == KindObject {
		return v.object, true
	}
	return nil, false
}

// Get looks up key on an object. When duplicate keys exist, the last matching
// member wins (matching RFC 8259 / ECMAScript JSON.parse behavior).
func (v *Value) Get(key string) (*Value, bool) {
	if v == nil || v.kind != KindObject {
		return nil, false
	}
	for i := len(v.object) - 1; i >= 0; i-- {
		if v.object[i].Key == key {
			return &v.object[i].Value, true
		}
	}
	return nil, false
}

// GetPath walks a chain of object keys.
func (v *Value) GetPath(path []string) (*Value, bool) {
	cur := v
	for _, k := range path {
		next, ok := cur.Get(k)
		if !ok {
			return nil, false
		}
		cur = next
	}
	return cur, true
}

// Index returns the i-th element of an array.
func (v *Value) Index(i int) (*Value, bool) {
	if v == nil || v.kind != KindArray || i < 0 || i >= len(v.array) {
		return nil, false
	}
	return &v.array[i], true
}

// Len returns the number of elements in an array or members in an object, or 0 otherwise.
func (v Value) Len() int {
	switch v.kind {
	case KindArray:
		return len(v.array)
	case KindObject:
		return len(v.object)
	default:
		return 0
	}
}

// IsEmpty reports whether an array or object has zero elements/members.
func (v Value) IsEmpty() bool {
	return v.Len() == 0
}

// Set inserts or replaces key on an object. If duplicate entries with key
// exist, the first matching position is updated and later duplicates are removed.
func (v *Value) Set(key string, val Value) bool {
	if v == nil || v.kind != KindObject {
		return false
	}
	firstIdx := -1
	for i := range v.object {
		if v.object[i].Key == key {
			firstIdx = i
			break
		}
	}
	if firstIdx >= 0 {
		v.object[firstIdx].Value = val
		out := v.object[:firstIdx+1]
		for i := firstIdx + 1; i < len(v.object); i++ {
			if v.object[i].Key != key {
				out = append(out, v.object[i])
			}
		}
		v.object = out
	} else {
		v.object = append(v.object, Member{Key: key, Value: val})
	}
	return true
}

// Push appends val to an array.
func (v *Value) Push(val Value) bool {
	if v == nil || v.kind != KindArray {
		return false
	}
	v.array = append(v.array, val)
	return true
}

// Equal performs deep structural equality comparison between two JSON values.
func (v Value) Equal(other Value) bool {
	if v.kind != other.kind {
		return false
	}
	switch v.kind {
	case KindNull:
		return true
	case KindBool:
		return v.boolean == other.boolean
	case KindNumber:
		return v.number == other.number
	case KindString:
		return v.str == other.str
	case KindArray:
		if len(v.array) != len(other.array) {
			return false
		}
		for i := range v.array {
			if !v.array[i].Equal(other.array[i]) {
				return false
			}
		}
		return true
	case KindObject:
		if len(v.object) != len(other.object) {
			return false
		}
		for i := range v.object {
			if v.object[i].Key != other.object[i].Key || !v.object[i].Value.Equal(other.object[i].Value) {
				return false
			}
		}
		return true
	default:
		return false
	}
}

// Clone returns a deep copy of v.
func (v Value) Clone() Value {
	switch v.kind {
	case KindArray:
		arr := make([]Value, len(v.array))
		for i := range v.array {
			arr[i] = v.array[i].Clone()
		}
		return Value{kind: KindArray, array: arr}
	case KindObject:
		obj := make([]Member, len(v.object))
		for i := range v.object {
			obj[i] = Member{Key: v.object[i].Key, Value: v.object[i].Value.Clone()}
		}
		return Value{kind: KindObject, object: obj}
	default:
		return v
	}
}

// ParseError describes a syntax or limit failure with its zero-based byte offset.
type ParseError struct {
	Message string
	Offset  int
}

func (e *ParseError) Error() string {
	return fmt.Sprintf("%s at byte %d", e.Message, e.Offset)
}

// Parse parses an RFC 8259 JSON string using MaxDepth (64) and MaxInputBytes (16 MiB).
func Parse(input string) (Value, error) {
	return ParseBytesWithLimits([]byte(input), MaxDepth, MaxInputBytes)
}

// ParseBytes parses a UTF-8 RFC 8259 JSON byte slice using default limits.
func ParseBytes(input []byte) (Value, error) {
	return ParseBytesWithLimits(input, MaxDepth, MaxInputBytes)
}

// ParseWithLimits parses an RFC 8259 JSON string with custom depth and byte caps.
func ParseWithLimits(input string, maxDepth int, maxBytes int) (Value, error) {
	return ParseBytesWithLimits([]byte(input), maxDepth, maxBytes)
}

// ParseBytesWithLimits parses a UTF-8 RFC 8259 JSON byte slice with custom caps.
func ParseBytesWithLimits(input []byte, maxDepth int, maxBytes int) (Value, error) {
	if len(input) > maxBytes {
		return Null(), &ParseError{Message: "input exceeds byte cap", Offset: maxBytes}
	}
	if !utf8.Valid(input) {
		return Null(), &ParseError{Message: "input is not valid UTF-8", Offset: 0}
	}
	p := parser{
		src:      input,
		pos:      0,
		depth:    0,
		maxDepth: maxDepth,
	}
	p.skipWS()
	val, err := p.parseValue()
	if err != nil {
		return Null(), err
	}
	p.skipWS()
	if p.pos != len(p.src) {
		return Null(), p.err("trailing characters after JSON value")
	}
	return val, nil
}

// Stringify serializes v into compact RFC 8259 JSON text.
func Stringify(v Value) string {
	var b strings.Builder
	writeCompact(&b, v)
	return b.String()
}

// StringifyPretty serializes v into multi-line indented RFC 8259 JSON text.
func StringifyPretty(v Value, indent int) string {
	if indent < 0 {
		indent = 0
	}
	var b strings.Builder
	writePretty(&b, v, indent, 0)
	return b.String()
}

// EscapeString formats a raw UTF-8 string as a quoted and escaped JSON string literal.
func EscapeString(s string) string {
	var b strings.Builder
	writeString(&b, s)
	return b.String()
}

type parser struct {
	src      []byte
	pos      int
	depth    int
	maxDepth int
}

func (p *parser) err(msg string) error {
	return &ParseError{Message: msg, Offset: p.pos}
}

func (p *parser) errAt(msg string, offset int) error {
	return &ParseError{Message: msg, Offset: offset}
}

func (p *parser) peek() (byte, bool) {
	if p.pos < len(p.src) {
		return p.src[p.pos], true
	}
	return 0, false
}

func (p *parser) skipWS() {
	for p.pos < len(p.src) {
		switch p.src[p.pos] {
		case ' ', '\t', '\r', '\n':
			p.pos++
		default:
			return
		}
	}
}

func (p *parser) parseValue() (Value, error) {
	p.skipWS()
	b, ok := p.peek()
	if !ok {
		return Null(), p.err("unexpected end of JSON input")
	}
	switch b {
	case 'n':
		return p.parseNull()
	case 't':
		return p.parseTrue()
	case 'f':
		return p.parseFalse()
	case '"':
		s, err := p.parseString()
		if err != nil {
			return Null(), err
		}
		return StringVal(s), nil
	case '[':
		return p.parseArray()
	case '{':
		return p.parseObject()
	case '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
		return p.parseNumber()
	default:
		return Null(), p.err("unexpected token")
	}
}

func (p *parser) parseNull() (Value, error) {
	if p.startsWith("null") {
		p.pos += 4
		return Null(), nil
	}
	return Null(), p.err("expected 'null'")
}

func (p *parser) parseTrue() (Value, error) {
	if p.startsWith("true") {
		p.pos += 4
		return Bool(true), nil
	}
	return Null(), p.err("expected 'true'")
}

func (p *parser) parseFalse() (Value, error) {
	if p.startsWith("false") {
		p.pos += 5
		return Bool(false), nil
	}
	return Null(), p.err("expected 'false'")
}

func (p *parser) startsWith(kw string) bool {
	end := p.pos + len(kw)
	if end > len(p.src) {
		return false
	}
	return string(p.src[p.pos:end]) == kw
}

func (p *parser) parseString() (string, error) {
	if b, ok := p.peek(); !ok || b != '"' {
		return "", p.err("expected '\"'")
	}
	p.pos++
	var out strings.Builder
	for {
		if p.pos >= len(p.src) {
			return "", p.err("unterminated string literal")
		}
		b := p.src[p.pos]
		if b == '"' {
			p.pos++
			return out.String(), nil
		}
		if b == '\\' {
			p.pos++
			if p.pos >= len(p.src) {
				return "", p.err("unterminated escape sequence")
			}
			esc := p.src[p.pos]
			p.pos++
			switch esc {
			case '"':
				out.WriteByte('"')
			case '\\':
				out.WriteByte('\\')
			case '/':
				out.WriteByte('/')
			case 'b':
				out.WriteByte('\b')
			case 'f':
				out.WriteByte('\f')
			case 'n':
				out.WriteByte('\n')
			case 'r':
				out.WriteByte('\r')
			case 't':
				out.WriteByte('\t')
			case 'u':
				u, err := p.readHex4()
				if err != nil {
					return "", err
				}
				if u >= 0xD800 && u <= 0xDBFF {
					if p.pos+2 > len(p.src) || p.src[p.pos] != '\\' || p.src[p.pos+1] != 'u' {
						return "", p.err("unpaired high UTF-16 surrogate")
					}
					p.pos += 2
					low, err := p.readHex4()
					if err != nil {
						return "", err
					}
					if low < 0xDC00 || low > 0xDFFF {
						return "", p.err("invalid low UTF-16 surrogate")
					}
					cp := 0x10000 + ((rune(u) - 0xD800) << 10) + (rune(low) - 0xDC00)
					if !utf8.ValidRune(cp) {
						return "", p.err("invalid supplementary code point")
					}
					out.WriteRune(cp)
				} else if u >= 0xDC00 && u <= 0xDFFF {
					return "", p.err("unpaired low UTF-16 surrogate")
				} else {
					out.WriteRune(rune(u))
				}
			default:
				return "", p.err("invalid string escape")
			}
			continue
		}
		if b < 0x20 {
			return "", p.err("unescaped control character in string")
		}
		r, width := utf8.DecodeRune(p.src[p.pos:])
		if r == utf8.RuneError && width == 1 {
			return "", p.err("invalid UTF-8 sequence in string")
		}
		out.WriteRune(r)
		p.pos += width
	}
}

func (p *parser) readHex4() (uint16, error) {
	if p.pos+4 > len(p.src) {
		return 0, p.err("truncated \\uXXXX escape")
	}
	var val uint16
	for i := 0; i < 4; i++ {
		b := p.src[p.pos+i]
		var digit uint16
		switch {
		case b >= '0' && b <= '9':
			digit = uint16(b - '0')
		case b >= 'a' && b <= 'f':
			digit = uint16(b-'a') + 10
		case b >= 'A' && b <= 'F':
			digit = uint16(b-'A') + 10
		default:
			return 0, p.errAt("non-hex digit in \\uXXXX escape", p.pos+i)
		}
		val = (val << 4) | digit
	}
	p.pos += 4
	return val, nil
}

func (p *parser) parseNumber() (Value, error) {
	start := p.pos
	if b, ok := p.peek(); ok && b == '-' {
		p.pos++
	}
	b, ok := p.peek()
	if !ok {
		return Null(), p.err("expected digit after '-'")
	}
	if b == '0' {
		p.pos++
		if next, ok := p.peek(); ok && next >= '0' && next <= '9' {
			return Null(), p.err("leading zeros are not allowed")
		}
	} else if b >= '1' && b <= '9' {
		p.pos++
		for {
			next, ok := p.peek()
			if !ok || next < '0' || next > '9' {
				break
			}
			p.pos++
		}
	} else {
		return Null(), p.err("expected digit")
	}

	if next, ok := p.peek(); ok && next == '.' {
		p.pos++
		fracFirst, ok := p.peek()
		if !ok || fracFirst < '0' || fracFirst > '9' {
			return Null(), p.err("expected digit after decimal point")
		}
		for {
			d, ok := p.peek()
			if !ok || d < '0' || d > '9' {
				break
			}
			p.pos++
		}
	}

	if next, ok := p.peek(); ok && (next == 'e' || next == 'E') {
		p.pos++
		if sign, ok := p.peek(); ok && (sign == '+' || sign == '-') {
			p.pos++
		}
		expFirst, ok := p.peek()
		if !ok || expFirst < '0' || expFirst > '9' {
			return Null(), p.err("expected digit in exponent")
		}
		for {
			d, ok := p.peek()
			if !ok || d < '0' || d > '9' {
				break
			}
			p.pos++
		}
	}

	slice := string(p.src[start:p.pos])
	n, err := strconv.ParseFloat(slice, 64)
	if err != nil || math.IsNaN(n) || math.IsInf(n, 0) {
		return Null(), p.errAt("number out of f64 range", start)
	}
	return Number(n), nil
}

func (p *parser) parseArray() (Value, error) {
	if p.depth >= p.maxDepth {
		return Null(), p.err("nesting exceeds depth cap")
	}
	p.pos++ // '['
	p.depth++
	p.skipWS()
	if b, ok := p.peek(); ok && b == ']' {
		p.pos++
		p.depth--
		return EmptyArray(), nil
	}
	var items []Value
	for {
		val, err := p.parseValue()
		if err != nil {
			p.depth--
			return Null(), err
		}
		items = append(items, val)
		p.skipWS()
		b, ok := p.peek()
		if !ok {
			p.depth--
			return Null(), p.err("unterminated array")
		}
		if b == ',' {
			p.pos++
			p.skipWS()
			if next, ok := p.peek(); ok && next == ']' {
				p.depth--
				return Null(), p.err("trailing comma in array")
			}
		} else if b == ']' {
			p.pos++
			p.depth--
			return Value{kind: KindArray, array: items}, nil
		} else {
			p.depth--
			return Null(), p.err("expected ',' or ']' in array")
		}
	}
}

func (p *parser) parseObject() (Value, error) {
	if p.depth >= p.maxDepth {
		return Null(), p.err("nesting exceeds depth cap")
	}
	p.pos++ // '{'
	p.depth++
	p.skipWS()
	if b, ok := p.peek(); ok && b == '}' {
		p.pos++
		p.depth--
		return EmptyObject(), nil
	}
	var members []Member
	for {
		p.skipWS()
		b, ok := p.peek()
		if !ok || b != '"' {
			p.depth--
			return Null(), p.err("expected object key string")
		}
		key, err := p.parseString()
		if err != nil {
			p.depth--
			return Null(), err
		}
		p.skipWS()
		colon, ok := p.peek()
		if !ok || colon != ':' {
			p.depth--
			return Null(), p.err("expected ':' after object key")
		}
		p.pos++
		val, err := p.parseValue()
		if err != nil {
			p.depth--
			return Null(), err
		}
		members = append(members, Member{Key: key, Value: val})
		p.skipWS()
		sep, ok := p.peek()
		if !ok {
			p.depth--
			return Null(), p.err("unterminated object")
		}
		if sep == ',' {
			p.pos++
			p.skipWS()
			if next, ok := p.peek(); ok && next == '}' {
				p.depth--
				return Null(), p.err("trailing comma in object")
			}
		} else if sep == '}' {
			p.pos++
			p.depth--
			return Value{kind: KindObject, object: members}, nil
		} else {
			p.depth--
			return Null(), p.err("expected ',' or '}' in object")
		}
	}
}

func writeCompact(out *strings.Builder, v Value) {
	switch v.kind {
	case KindNull:
		out.WriteString("null")
	case KindBool:
		if v.boolean {
			out.WriteString("true")
		} else {
			out.WriteString("false")
		}
	case KindNumber:
		writeNumber(out, v.number)
	case KindString:
		writeString(out, v.str)
	case KindArray:
		out.WriteByte('[')
		for i, item := range v.array {
			if i > 0 {
				out.WriteByte(',')
			}
			writeCompact(out, item)
		}
		out.WriteByte(']')
	case KindObject:
		out.WriteByte('{')
		for i, m := range v.object {
			if i > 0 {
				out.WriteByte(',')
			}
			writeString(out, m.Key)
			out.WriteByte(':')
			writeCompact(out, m.Value)
		}
		out.WriteByte('}')
	}
}

func writePretty(out *strings.Builder, v Value, indent int, level int) {
	switch v.kind {
	case KindArray:
		if len(v.array) == 0 {
			out.WriteString("[]")
			return
		}
		out.WriteString("[\n")
		for i, item := range v.array {
			pushIndent(out, indent*(level+1))
			writePretty(out, item, indent, level+1)
			if i+1 < len(v.array) {
				out.WriteByte(',')
			}
			out.WriteByte('\n')
		}
		pushIndent(out, indent*level)
		out.WriteByte(']')
	case KindObject:
		if len(v.object) == 0 {
			out.WriteString("{}")
			return
		}
		out.WriteString("{\n")
		for i, m := range v.object {
			pushIndent(out, indent*(level+1))
			writeString(out, m.Key)
			out.WriteString(": ")
			writePretty(out, m.Value, indent, level+1)
			if i+1 < len(v.object) {
				out.WriteByte(',')
			}
			out.WriteByte('\n')
		}
		pushIndent(out, indent*level)
		out.WriteByte('}')
	default:
		writeCompact(out, v)
	}
}

func pushIndent(out *strings.Builder, spaces int) {
	for i := 0; i < spaces; i++ {
		out.WriteByte(' ')
	}
}

func writeNumber(out *strings.Builder, n float64) {
	if math.IsNaN(n) || math.IsInf(n, 0) {
		out.WriteString("null")
		return
	}
	if n == 0.0 {
		out.WriteByte('0')
		return
	}
	if math.Trunc(n) == n && math.Abs(n) < 1e15 {
		out.WriteString(strconv.FormatInt(int64(n), 10))
	} else {
		out.WriteString(strconv.FormatFloat(n, 'g', -1, 64))
	}
}

func writeString(out *strings.Builder, s string) {
	out.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			out.WriteString("\\\"")
		case '\\':
			out.WriteString("\\\\")
		case '\b':
			out.WriteString("\\b")
		case '\f':
			out.WriteString("\\f")
		case '\n':
			out.WriteString("\\n")
		case '\r':
			out.WriteString("\\r")
		case '\t':
			out.WriteString("\\t")
		default:
			if r < 0x20 {
				const hexDigits = "0123456789abcdef"
				out.WriteString("\\u00")
				out.WriteByte(hexDigits[(r>>4)&0x0F])
				out.WriteByte(hexDigits[r&0x0F])
			} else {
				out.WriteRune(r)
			}
		}
	}
	out.WriteByte('"')
}
