// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

package main

import (
	"fmt"
	"os"

	anvilgo "github.com/buntatoes/anvil-browser/engines/go"
)

func main() {
	report, err := anvilgo.VerifyGoSubsystems()
	if err != nil {
		fmt.Fprintf(os.Stderr, "anvil-go-engine verification error: %v\n", err)
		os.Exit(1)
	}
	if len(os.Args) > 1 && os.Args[1] == "--version" {
		fmt.Printf("%s %s (%s %s/%s)\n", report.EngineName, report.BrowserVer, report.GoRuntime, report.GOOS, report.GOARCH)
		return
	}
	fmt.Println(anvilgo.FormatDiagnosticsJSON(report))
}
