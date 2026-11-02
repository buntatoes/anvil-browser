// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package anvilhpack

import (
	"bytes"
	"encoding/hex"
	"testing"
)

type rfcCase struct {
	headers   [][2]string
	blockHex  string
	tableSize int
}

func mustHex(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

func makeHeaders(pairs [][2]string) []Header {
	out := make([]Header, len(pairs))
	for i, p := range pairs {
		out[i] = NewHeader(p[0], p[1])
	}
	return out
}

func runRFCGroup(t *testing.T, cases []rfcCase, tableSize int, huffman bool) {
	t.Helper()
	decoder := NewDecoderWithLimits(tableSize, 1<<20)
	encoder := NewEncoderWithTableSize(tableSize)
	encoder.SetHuffman(huffman)

	for i, c := range cases {
		block := mustHex(c.blockHex)
		decoded, err := decoder.Decode(block)
		if err != nil {
			t.Fatalf("block %d decode failed: %v", i, err)
		}
		expected := makeHeaders(c.headers)
		if len(decoded) != len(expected) {
			t.Fatalf("block %d header count mismatch: %d vs %d", i, len(decoded), len(expected))
		}
		for j := range expected {
			if !decoded[j].Equal(expected[j]) {
				t.Fatalf("block %d field %d mismatch: %+v vs %+v", i, j, decoded[j], expected[j])
			}
		}
		if decoder.Table().Size() != c.tableSize {
			t.Fatalf("block %d decoder table size %d != %d", i, decoder.Table().Size(), c.tableSize)
		}

		encoded := encoder.Encode(expected, nil)
		if !bytes.Equal(encoded, block) {
			t.Fatalf("block %d encoded bytes mismatch:\n got: %x\nwant: %x", i, encoded, block)
		}
		if encoder.Table().Size() != c.tableSize {
			t.Fatalf("block %d encoder table size %d != %d", i, encoder.Table().Size(), c.tableSize)
		}
	}
}

func TestRFC7541RequestsWithoutHuffman(t *testing.T) {
	cases := []rfcCase{
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "http"},
				{":path", "/"},
				{":authority", "www.example.com"},
			},
			blockHex:  "828684410f7777772e6578616d706c652e636f6d",
			tableSize: 57,
		},
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "http"},
				{":path", "/"},
				{":authority", "www.example.com"},
				{"cache-control", "no-cache"},
			},
			blockHex:  "828684be58086e6f2d6361636865",
			tableSize: 110,
		},
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "https"},
				{":path", "/index.html"},
				{":authority", "www.example.com"},
				{"custom-key", "custom-value"},
			},
			blockHex:  "828785bf400a637573746f6d2d6b65790c637573746f6d2d76616c7565",
			tableSize: 164,
		},
	}
	runRFCGroup(t, cases, 4096, false)
}

func TestRFC7541RequestsWithHuffman(t *testing.T) {
	cases := []rfcCase{
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "http"},
				{":path", "/"},
				{":authority", "www.example.com"},
			},
			blockHex:  "828684418cf1e3c2e5f23a6ba0ab90f4ff",
			tableSize: 57,
		},
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "http"},
				{":path", "/"},
				{":authority", "www.example.com"},
				{"cache-control", "no-cache"},
			},
			blockHex:  "828684be5886a8eb10649cbf",
			tableSize: 110,
		},
		{
			headers: [][2]string{
				{":method", "GET"},
				{":scheme", "https"},
				{":path", "/index.html"},
				{":authority", "www.example.com"},
				{"custom-key", "custom-value"},
			},
			blockHex:  "828785bf408825a849e95ba97d7f8925a849e95bb8e8b4bf",
			tableSize: 164,
		},
	}
	runRFCGroup(t, cases, 4096, true)
}

func TestRFC7541ResponsesWithoutHuffman(t *testing.T) {
	cases := []rfcCase{
		{
			headers: [][2]string{
				{":status", "302"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:21 GMT"},
				{"location", "https://www.example.com"},
			},
			blockHex:  "4803333032580770726976617465611d4d6f6e2c203231204f637420323031332032303a31333a323120474d546e1768747470733a2f2f7777772e6578616d706c652e636f6d",
			tableSize: 222,
		},
		{
			headers: [][2]string{
				{":status", "307"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:21 GMT"},
				{"location", "https://www.example.com"},
			},
			blockHex:  "4803333037c1c0bf",
			tableSize: 222,
		},
		{
			headers: [][2]string{
				{":status", "200"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:22 GMT"},
				{"location", "https://www.example.com"},
				{"content-encoding", "gzip"},
				{"set-cookie", "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1"},
			},
			blockHex:  "88c1611d4d6f6e2c203231204f637420323031332032303a31333a323220474d54c05a04677a69707738666f6f3d4153444a4b48514b425a584f5157454f50495541585157454f49553b206d61782d6167653d333630303b2076657273696f6e3d31",
			tableSize: 215,
		},
	}
	runRFCGroup(t, cases, 256, false)
}

func TestRFC7541ResponsesWithHuffman(t *testing.T) {
	cases := []rfcCase{
		{
			headers: [][2]string{
				{":status", "302"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:21 GMT"},
				{"location", "https://www.example.com"},
			},
			blockHex:  "488264025885aec3771a4b6196d07abe941054d444a8200595040b8166e082a62d1bff6e919d29ad171863c78f0b97c8e9ae82ae43d3",
			tableSize: 222,
		},
		{
			headers: [][2]string{
				{":status", "307"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:21 GMT"},
				{"location", "https://www.example.com"},
			},
			blockHex:  "4883640effc1c0bf",
			tableSize: 222,
		},
		{
			headers: [][2]string{
				{":status", "200"},
				{"cache-control", "private"},
				{"date", "Mon, 21 Oct 2013 20:13:22 GMT"},
				{"location", "https://www.example.com"},
				{"content-encoding", "gzip"},
				{"set-cookie", "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1"},
			},
			blockHex:  "88c16196d07abe941054d444a8200595040b8166e084a62d1bffc05a839bd9ab77ad94e7821dd7f2e6c7b335dfdfcd5b3960d5af27087f3672c1ab270fb5291f9587316065c003ed4ee5b1063d5007",
			tableSize: 215,
		},
	}
	runRFCGroup(t, cases, 256, true)
}
