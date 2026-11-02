// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package main

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"

	anvilcompress "github.com/buntatoes/anvil-compress"
)

const (
	opDeflateDecompress byte = 1
	opDeflateCompress   byte = 2
	opZlibDecompress    byte = 3
	opZlibCompress      byte = 4
	opGzipDecompress    byte = 5
	opGzipCompress      byte = 6
	opDecompressAuto    byte = 7
	opAdler32           byte = 8
	opCrc32             byte = 9
)

func writeError(bw *bufio.Writer, err error) {
	var hdr [13]byte
	var ce *anvilcompress.CompressError
	if errors.As(err, &ce) {
		_ = bw.WriteByte(1)
		hdr[0] = byte(ce.Kind)
		binary.LittleEndian.PutUint32(hdr[1:5], ce.Expected)
		binary.LittleEndian.PutUint32(hdr[5:9], ce.Actual)
		binary.LittleEndian.PutUint32(hdr[9:13], uint32(ce.Limit))
		_, _ = bw.Write(hdr[:13])
		binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(ce.Detail)))
		_, _ = bw.Write(hdr[0:4])
		_, _ = bw.WriteString(ce.Detail)
		return
	}
	msg := err.Error()
	_ = bw.WriteByte(1)
	hdr[0] = byte(anvilcompress.ErrInvalidHeader)
	binary.LittleEndian.PutUint32(hdr[1:5], 0)
	binary.LittleEndian.PutUint32(hdr[5:9], 0)
	binary.LittleEndian.PutUint32(hdr[9:13], 0)
	_, _ = bw.Write(hdr[:13])
	binary.LittleEndian.PutUint32(hdr[0:4], uint32(len(msg)))
	_, _ = bw.Write(hdr[0:4])
	_, _ = bw.WriteString(msg)
}

func writeSuccessBytes(bw *bufio.Writer, data []byte) {
	var hdr [4]byte
	_ = bw.WriteByte(0)
	binary.LittleEndian.PutUint32(hdr[:], uint32(len(data)))
	_, _ = bw.Write(hdr[:])
	_, _ = bw.Write(data)
}

func runWorker(r io.Reader, w io.Writer) error {
	br := bufio.NewReaderSize(r, 64*1024)
	bw := bufio.NewWriterSize(w, 64*1024)
	var hdr [8]byte

	for {
		op, err := br.ReadByte()
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}

		switch op {
		case opDeflateDecompress, opZlibDecompress, opGzipDecompress, opDecompressAuto:
			if _, err := io.ReadFull(br, hdr[:8]); err != nil {
				return err
			}
			maxOut := int(binary.LittleEndian.Uint32(hdr[0:4]))
			n := int(binary.LittleEndian.Uint32(hdr[4:8]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			var out []byte
			var decErr error
			switch op {
			case opDeflateDecompress:
				out, decErr = anvilcompress.DeflateDecompressBounded(payload, maxOut)
			case opZlibDecompress:
				out, decErr = anvilcompress.ZlibDecompressBounded(payload, maxOut)
			case opGzipDecompress:
				out, decErr = anvilcompress.GzipDecompressBounded(payload, maxOut)
			case opDecompressAuto:
				out, decErr = anvilcompress.DecompressAuto(payload, maxOut)
			}
			if decErr != nil {
				writeError(bw, decErr)
			} else {
				writeSuccessBytes(bw, out)
			}
		case opDeflateCompress, opZlibCompress, opGzipCompress:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			n := int(binary.LittleEndian.Uint32(hdr[0:4]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			var out []byte
			switch op {
			case opDeflateCompress:
				out = anvilcompress.DeflateCompress(payload)
			case opZlibCompress:
				out = anvilcompress.ZlibCompress(payload)
			case opGzipCompress:
				out = anvilcompress.GzipCompress(payload)
			}
			writeSuccessBytes(bw, out)
		case opAdler32, opCrc32:
			if _, err := io.ReadFull(br, hdr[:4]); err != nil {
				return err
			}
			n := int(binary.LittleEndian.Uint32(hdr[0:4]))
			payload := make([]byte, n)
			if _, err := io.ReadFull(br, payload); err != nil {
				return err
			}
			var sum uint32
			if op == opAdler32 {
				sum = anvilcompress.Adler32(payload)
			} else {
				sum = anvilcompress.Crc32(payload)
			}
			_ = bw.WriteByte(0)
			binary.LittleEndian.PutUint32(hdr[0:4], sum)
			_, _ = bw.Write(hdr[0:4])
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
		fmt.Printf("anvil-compress %s (go)\n", anvilcompress.Version)
		return
	}
	if len(args) > 0 && args[0] == "--worker" {
		if err := runWorker(os.Stdin, os.Stdout); err != nil {
			os.Exit(1)
		}
		return
	}
	if len(args) > 0 && args[0] == "--self-test" {
		msg := []byte("Anvil Compress Go v.04 self-test payload")
		z := anvilcompress.ZlibCompress(msg)
		dz, err := anvilcompress.ZlibDecompress(z)
		if err != nil || !bytes.Equal(dz, msg) {
			fmt.Fprintf(os.Stderr, "zlib self-test failed: %v\n", err)
			os.Exit(1)
		}
		g := anvilcompress.GzipCompress(msg)
		dg, err := anvilcompress.GzipDecompress(g)
		if err != nil || !bytes.Equal(dg, msg) {
			fmt.Fprintf(os.Stderr, "gzip self-test failed: %v\n", err)
			os.Exit(1)
		}
		fmt.Printf("ok: adler32=0x%08x crc32=0x%08x\n", anvilcompress.Adler32(msg), anvilcompress.Crc32(msg))
		return
	}
}
