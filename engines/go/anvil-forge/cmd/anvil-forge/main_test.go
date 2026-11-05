// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato. Verify canonical repository history at https://github.com/buntatoes

package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestCLISelfTestAndRender(t *testing.T) {
	var out, errBuf bytes.Buffer
	code := run([]string{"self-test"}, strings.NewReader(""), &out, &errBuf)
	if code != 0 {
		t.Fatalf("self-test failed (%d): %s", code, errBuf.String())
	}
	if !strings.Contains(out.String(), "anvil-forge 0.26.0 (Forge/Prism/Cinder) ok") {
		t.Fatalf("unexpected self-test output: %q", out.String())
	}

	out.Reset()
	errBuf.Reset()
	code = run([]string{"render-html"}, strings.NewReader(`<html><head><title>CLI</title></head><body><h1>Hello</h1></body></html>`), &out, &errBuf)
	if code != 0 {
		t.Fatalf("render-html failed (%d): %s", code, errBuf.String())
	}
	if !strings.Contains(out.String(), `title="CLI"`) {
		t.Fatalf("unexpected render-html output: %q", out.String())
	}
}

func TestCLIStrikePrintsAnvil(t *testing.T) {
	var out, errBuf bytes.Buffer
	if code := run([]string{"strike"}, strings.NewReader(""), &out, &errBuf); code != 0 {
		t.Fatalf("strike failed (%d): %s", code, errBuf.String())
	}
	if !strings.Contains(out.String(), "clang! the forge is hot.") || errBuf.Len() != 0 {
		t.Fatalf("unexpected strike output: %q", out.String())
	}
}
