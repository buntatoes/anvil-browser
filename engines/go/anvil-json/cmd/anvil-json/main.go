// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

package main

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"

	anviljson "github.com/buntatoes/anvil-json"
)

const (
	opParse           byte = 1
	opStringify       byte = 2
	opStringifyPretty byte = 3
	opEscapeString    byte = 4
	opResolvePointer  byte = 5
)

func runWorker(r io.Reader, w io.Writer) error {
	br := bufio.NewReaderSize(r, 64*1024)
	bw := bufio.NewWriterSize(w, 64*1024)
	var hdr [13]byte

	for {
		op, err := br.ReadByte()
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}

		switch op {
		case opParse:
			if _, err := io.ReadFull(br, hdr[:12]); err != nil {
				return err
			}
			maxDepth := int(binary.LittleEndian.Uint32(hdr[0:4]))
			maxBytes := int(binary.LittleEndian.Uint32(hdr[4:8]))
			n := int(binary.LittleEndian.Uint32(hdr[8:12]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			val, parseErr := anviljson.ParseBytesWithLimits(payload, maxDepth, maxBytes)
			if parseErr != nil {
				offset := 0
				msg := parseErr.Error()
				var pe *anviljson.ParseError
				if errors.As(parseErr, &pe) {
					offset = pe.Offset
					msg = pe.Message
				}
				_ = bw.WriteByte(1)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(offset))
				binary.LittleEndian.PutUint32(hdr[4:8], uint32(len(msg)))
				_, _ = bw.Write(hdr[0:8])
				_, _ = bw.WriteString(msg)
			} else {
				encoded := anviljson.EncodeBinaryValue(nil, val)
				_ = bw.WriteByte(0)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(encoded)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.Write(encoded)
			}
		case opStringify:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			n := int(binary.LittleEndian.Uint32(hdr[0:4]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			pos := 0
			val, decErr := anviljson.DecodeBinaryValue(payload, &pos)
			if decErr != nil {
				msg := decErr.Error()
				_ = bw.WriteByte(1)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(msg)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.WriteString(msg)
			} else {
				out := anviljson.Stringify(val)
				_ = bw.WriteByte(0)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(out)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.WriteString(out)
			}
		case opStringifyPretty:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			indent := int(binary.LittleEndian.Uint32(hdr[0:4]))
			n := int(binary.LittleEndian.Uint32(hdr[4:8]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			pos := 0
			val, decErr := anviljson.DecodeBinaryValue(payload, &pos)
			if decErr != nil {
				msg := decErr.Error()
				_ = bw.WriteByte(1)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(msg)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.WriteString(msg)
			} else {
				out := anviljson.StringifyPretty(val, indent)
				_ = bw.WriteByte(0)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(out)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.WriteString(out)
			}
		case opEscapeString:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			n := int(binary.LittleEndian.Uint32(hdr[0:4]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			out := anviljson.EscapeString(string(payload))
			_ = bw.WriteByte(0)
			binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(out)))
			_, _ = bw.Write(hdr[0:4])
			_, _ = bw.WriteString(out)
		case opResolvePointer:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			docLen := int(binary.LittleEndian.Uint32(hdr[0:4]))
			ptrLen := int(binary.LittleEndian.Uint32(hdr[4:8]))
			docBytes := make([]byte, docLen)
			if _, err := io.ReadFull(br, docBytes); err != nil {
				return err
			}
			ptrBytes := make([]byte, ptrLen)
			if _, err := io.ReadFull(br, ptrBytes); err != nil {
				return err
			}
			pos := 0
			doc, decErr := anviljson.DecodeBinaryValue(docBytes, &pos)
			if decErr != nil {
				msg := decErr.Error()
				_ = bw.WriteByte(1)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(msg)))
				_, _ = bw.Write(hdr[0:4])
				_, _ = bw.WriteString(msg)
			} else {
				resolved, ptrErr := anviljson.ResolvePointer(&doc, string(ptrBytes))
				if ptrErr != nil {
					msg := ptrErr.Error()
					_ = bw.WriteByte(1)
					binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(msg)))
					_, _ = bw.Write(hdr[0:4])
					_, _ = bw.WriteString(msg)
				} else {
					encoded := anviljson.EncodeBinaryValue(nil, *resolved)
					_ = bw.WriteByte(0)
					binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(encoded)))
					_, _ = bw.Write(hdr[0:4])
					_, _ = bw.Write(encoded)
				}
			}
		default:
			return fmt.Errorf("unknown worker opcode %d", op)
		}

		if err := bw.Flush(); err != nil {
			return err
		}
	}
}

func main() {
	args := os.Args[1:]
	if len(args) > 0 && args[0] == "--version" {
		fmt.Printf("anvil-json %s (go)\n", anviljson.Version)
		return
	}
	if len(args) > 0 && args[0] == "--worker" {
		if err := runWorker(os.Stdin, os.Stdout); err != nil {
			os.Exit(1)
		}
		return
	}
	if len(args) > 0 && args[0] == "--self-test" {
		v, err := anviljson.Parse(`{"engine":"go","version":4,"items":[true,null,"ok"]}`)
		if err != nil {
			fmt.Fprintf(os.Stderr, "self-test parse failed: %v\n", err)
			os.Exit(1)
		}
		ptrVal, err := anviljson.ResolvePointer(&v, "/items/2")
		if err != nil {
			fmt.Fprintf(os.Stderr, "self-test pointer failed: %v\n", err)
			os.Exit(1)
		}
		if s, ok := ptrVal.AsStr(); !ok || s != "ok" {
			fmt.Fprintf(os.Stderr, "self-test unexpected pointer value\n")
			os.Exit(1)
		}
		fmt.Println(anviljson.Stringify(v))
		return
	}

	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		fmt.Fprintf(os.Stderr, "read error: %v\n", err)
		os.Exit(1)
	}
	doc, err := anviljson.ParseBytes(input)
	if err != nil {
		fmt.Fprintf(os.Stderr, "%v\n", err)
		os.Exit(1)
	}
	if len(args) >= 2 && args[0] == "--pointer" {
		target, err := anviljson.ResolvePointer(&doc, args[1])
		if err != nil {
			fmt.Fprintf(os.Stderr, "%v\n", err)
			os.Exit(1)
		}
		fmt.Print(anviljson.Stringify(*target))
		return
	}
	if len(args) > 0 && args[0] == "--pretty" {
		fmt.Print(anviljson.StringifyPretty(doc, 2))
		return
	}
	fmt.Print(anviljson.Stringify(doc))
}
