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

	anvilhpack "github.com/buntatoes/anvil-hpack"
)

const (
	opNewEncoder               byte = 1
	opEncoderSetHuffman        byte = 2
	opEncoderSetMaxTableSize   byte = 3
	opEncoderEncode            byte = 4
	opNewDecoder               byte = 5
	opDecoderSetMaxAllowedSize byte = 6
	opDecoderDecode            byte = 7
	opFreeEncoder              byte = 8
	opFreeDecoder              byte = 9
)

func runWorker(r io.Reader, w io.Writer) error {
	br := bufio.NewReaderSize(r, 64*1024)
	bw := bufio.NewWriterSize(w, 64*1024)
	encoders := make(map[uint32]*anvilhpack.Encoder)
	decoders := make(map[uint32]*anvilhpack.Decoder)
	var nextID uint32 = 1
	var hdr [12]byte

	for {
		op, err := br.ReadByte()
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}

		switch op {
		case opNewEncoder:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			tableSize := int(binary.LittleEndian.Uint32(hdr[0:4]))
			id := nextID
			nextID++
			encoders[id] = anvilhpack.NewEncoderWithTableSize(tableSize)
			_ = bw.WriteByte(0)
			binary.LittleEndian.PutUint32(hdr[0:4], id)
			_, _ = bw.Write(hdr[0:4])
		case opEncoderSetHuffman:
			if _, err := io.ReadFull(br, hdr[:5]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			on := hdr[4] != 0
			if enc, ok := encoders[id]; ok {
				enc.SetHuffman(on)
			}
			_ = bw.WriteByte(0)
		case opEncoderSetMaxTableSize:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			sz := int(binary.LittleEndian.Uint32(hdr[4:8]))
			if enc, ok := encoders[id]; ok {
				enc.SetMaxTableSize(sz)
			}
			_ = bw.WriteByte(0)
		case opEncoderEncode:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			count := int(binary.LittleEndian.Uint32(hdr[4:8]))
			fields := make([]anvilhpack.Header, count)
			for i := 0; i < count; i++ {
				if _, err := io.ReadFull(br, hdr[:9]); err != nil {
					return err
				}
				nLen := int(binary.LittleEndian.Uint32(hdr[0:4]))
				vLen := int(binary.LittleEndian.Uint32(hdr[4:8]))
				never := hdr[8] != 0
				name := make([]byte, nLen)
				val := make([]byte, vLen)
				if _, err := io.ReadFull(br, name); err != nil {
					return err
				}
				if _, err := io.ReadFull(br, val); err != nil {
					return err
				}
				fields[i] = anvilhpack.Header{Name: name, Value: val, NeverIndexed: never}
			}
			enc, ok := encoders[id]
			if !ok {
				enc = anvilhpack.NewEncoder()
				encoders[id] = enc
			}
			out := enc.Encode(fields, nil)
			_ = bw.WriteByte(0)
			binary.LittleEndian.PutUint32(hdr[0:4], uint32(enc.Table().Size()))
			binary.LittleEndian.PutUint32(hdr[4:8], uint32(len(out)))
			_, _ = bw.Write(hdr[0:8])
			_, _ = bw.Write(out)
		case opNewDecoder:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			maxTable := int(binary.LittleEndian.Uint32(hdr[0:4]))
			maxList := int(binary.LittleEndian.Uint32(hdr[4:8]))
			id := nextID
			nextID++
			decoders[id] = anvilhpack.NewDecoderWithLimits(maxTable, maxList)
			_ = bw.WriteByte(0)
			binary.LittleEndian.PutUint32(hdr[0:4], id)
			_, _ = bw.Write(hdr[0:4])
		case opDecoderSetMaxAllowedSize:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			sz := int(binary.LittleEndian.Uint32(hdr[4:8]))
			if dec, ok := decoders[id]; ok {
				dec.SetMaxAllowedTableSize(sz)
			}
			_ = bw.WriteByte(0)
		case opDecoderDecode:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			n := int(binary.LittleEndian.Uint32(hdr[4:8]))
			block := make([]byte, n)
			if _, err := io.ReadFull(br, block); err != nil {
				return err
			}
			dec, ok := decoders[id]
			if !ok {
				dec = anvilhpack.NewDecoder()
				decoders[id] = dec
			}
			fields, decErr := dec.Decode(block)
			if decErr != nil {
				var he *anvilhpack.HpackError
				_ = bw.WriteByte(1)
				if errors.As(decErr, &he) {
					hdr[0] = byte(he.Kind)
					binary.LittleEndian.PutUint32(hdr[1:5], uint32(he.Value))
				} else {
					hdr[0] = byte(anvilhpack.ErrTruncated)
					binary.LittleEndian.PutUint32(hdr[1:5], 0)
				}
				_, _ = bw.Write(hdr[0:5])
			} else {
				_ = bw.WriteByte(0)
				binary.LittleEndian.PutUint32(hdr[0:4], uint32(dec.Table().Size()))
				binary.LittleEndian.PutUint32(hdr[4:8], uint32(dec.Table().MaxSize()))
				binary.LittleEndian.PutUint32(hdr[8:12], uint32(len(fields)))
				_, _ = bw.Write(hdr[0:12])
				for _, f := range fields {
					binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(f.Name)))
					binary.LittleEndian.PutUint32(hdr[4:8], uint32(len(f.Value)))
					if f.NeverIndexed {
						hdr[8] = 1
					} else {
						hdr[8] = 0
					}
					_, _ = bw.Write(hdr[0:9])
					_, _ = bw.Write(f.Name)
					_, _ = bw.Write(f.Value)
				}
			}
		case opFreeEncoder:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			delete(encoders, id)
		case opFreeDecoder:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			id := binary.LittleEndian.Uint32(hdr[0:4])
			delete(decoders, id)
		default:
			return fmt.Errorf("unknown worker opcode %d", op)
		}

		if op != opFreeEncoder && op != opFreeDecoder {
			if err := bw.Flush(); err != nil {
				return err
			}
		}
	}
}

func main() {
	args := os.Args[1:]
	if len(args) > 0 && args[0] == "--version" {
		fmt.Printf("anvil-hpack %s (go)\n", anvilhpack.Version)
		return
	}
	if len(args) > 0 && args[0] == "--worker" {
		if err := runWorker(os.Stdin, os.Stdout); err != nil {
			os.Exit(1)
		}
		return
	}
	if len(args) > 0 && args[0] == "--self-test" {
		enc := anvilhpack.NewEncoder()
		dec := anvilhpack.NewDecoder()
		in := []anvilhpack.Header{
			anvilhpack.NewHeader(":method", "GET"),
			anvilhpack.NewHeader(":path", "/"),
			anvilhpack.NewSensitiveHeader("authorization", "Bearer token"),
		}
		block := enc.Encode(in, nil)
		out, err := dec.Decode(block)
		if err != nil || len(out) != len(in) {
			fmt.Fprintf(os.Stderr, "self-test failed: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("ok: block_len=%d table_size=%d\n", len(block), dec.Table().Size())
		return
	}
}
