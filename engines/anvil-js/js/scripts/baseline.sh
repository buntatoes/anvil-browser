#!/bin/sh
# SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
# Copyright (c) 2026 Buntato
#
# Runs the anvil-jstest conformance suite (DOM tests left out) against the
# stand-in and prints the summary. The suite is at ANVIL_JSTEST_DIR
# (default ../../anvil-jstest, next to this repository). Nothing is
# written there: its sources are copied to a temporary directory to build
# `jstest`, which goes into this tree's target/jstest. Pass --json FILE
# to keep the per-test results.
set -eu

cd "$(dirname "$0")/.."
ROOT=$(pwd)
SUITE=${ANVIL_JSTEST_DIR:-../../anvil-jstest}
SUITE=$(cd "$SUITE" && pwd)

if [ ! -x "$ROOT/target/msrv/release/standin" ]; then
    (cd tools && CARGO_TARGET_DIR="$ROOT/target/msrv" cargo +1.99.1 build --release --quiet -p standin)
fi

if [ ! -x "$ROOT/target/jstest/release/jstest" ]; then
    tmp=$(mktemp -d)
    cp -R "$SUITE/Cargo.toml" "$SUITE/src" "$tmp/"
    (cd "$tmp" && CARGO_TARGET_DIR="$ROOT/target/jstest" cargo build --release --quiet --bin jstest)
    rm -rf "$tmp"
fi

cd "$SUITE"
"$ROOT/target/jstest/release/jstest" --engine "$ROOT/target/msrv/release/standin" \
    --skip-feature dom --allow-failures "$@"
