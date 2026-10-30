#!/bin/sh
# SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
# Copyright (c) 2026 Buntato
#
# Everything that has to pass, in order, one step at a time. Run from
# anywhere. Needs Rust 1.99.1 (the minimum) and a stable toolchain, both
# installed with rustup. SKIP_STABLE=1 leaves out the stable pass.
#
# The token differential runs on every corpus file, on the hand-written
# cases in tests/lex, on a sample of the generated stress cases (every
# 5th of 3000) and on all generated malformed inputs, with the identifier
# scan on every 101st code point and a 256 KB timing run. FULL=1 runs
# all 3000 stress cases, the scan on every code point, and the 1 MB
# file (token differential and timing).
#
# The syntax tree differential runs on the corpus, tests/parse, tests/lex
# and the generated lexer inputs, on the nesting depth files, and on a
# sample of 20,000 generated programs (every 10th); FULL=1 runs all of
# them. The parser timing run is 256 KB, or 1 MB with FULL=1.
#
# The scope checks: the parser unit tests against unitgen --check, the
# scope self-check and the scope differential on the corpus, tests/parse,
# tests/scope and every 10th generated program (FULL=1: all of them).
#
# The Rust reference is the cinder crate at the root of this repository
# (the path refdump's Cargo.toml uses). It is only read, never written.
# The corpus also reads ../../anvil-jstest, checked out next to this
# repository (see tests/corpus/CORPUS.txt); a corpus directory that is
# not there is left out with a note.
set -eu

cd "$(dirname "$0")/.."
ROOT=$(pwd)

# The corpus directories that exist, one per line.
corpus_dirs() {
    grep -v '^#' tests/corpus/CORPUS.txt | grep -v '^$' | while read -r d; do
        if [ -d "$d" ]; then
            echo "$d"
        else
            echo "note: corpus directory $d is not there, left out" >&2
        fi
    done
}

step() {
    printf '\n== %s\n' "$*"
}

# Run every Rust and JS check with one toolchain. $1 names it, $2 is the
# target directory, the rest is the cargo command to use.
pass() {
    name=$1
    target=$2
    shift 2
    step "rustfmt ($name)"
    (cd tools && "$@" fmt --all -- --check)

    step "clippy -D warnings ($name)"
    (cd tools && CARGO_TARGET_DIR="$ROOT/$target" "$@" clippy --all-targets --quiet -- -D warnings)

    step "cargo test ($name)"
    (cd tools && CARGO_TARGET_DIR="$ROOT/$target" "$@" test --quiet)

    step "release build of the tools ($name)"
    (cd tools && CARGO_TARGET_DIR="$ROOT/$target" "$@" build --release --quiet)

    bin="$ROOT/$target/release"
    js_checks "$name" "$bin"
}

js_checks() {
    name=$1
    bin=$2
    tmp=$(mktemp -d)

    # dist/ is a build output and is not checked in.
    step "bundle lint and build ($name)"
    mkdir -p dist
    "$bin/bundle" build --root . --out dist/cinder.bundle.js

    step "bundle runs on the stand-in ($name)"
    "$bin/standin" dist/cinder.bundle.js

    step "unit tests on the stand-in ($name)"
    "$bin/bundle" unit --root . --standin "$bin/standin" tests/unit

    step "Core JS probes ($name)"
    "$bin/bundle" probes --root . --standin "$bin/standin" tests/core

    step "limits match the reference ($name)"
    "$bin/refdump" --limits > "$tmp/limits.txt"
    diff -u tests/unit/limits_table.test.expected "$tmp/limits.txt"

    # The reference takes its identifier classes from the Rust standard
    # library, whose Unicode version moves with the toolchain. The tables
    # are the ones the reference gave when built with Rust 1.85.1; newer
    # toolchains have newer Unicode data, so a difference is reported, not
    # failed, unless STRICT_TABLES=1.
    step "identifier tables match the reference ($name)"
    "$bin/refdump" --id-tables > "$tmp/unicode_tables.js"
    same_tables=1
    if [ "$strict_tables" = 1 ]; then
        diff -u src/core/unicode_tables.js "$tmp/unicode_tables.js"
    elif ! cmp -s src/core/unicode_tables.js "$tmp/unicode_tables.js"; then
        same_tables=0
        echo "note: the reference built with this toolchain has different identifier tables"
        echo "      ($(diff src/core/unicode_tables.js "$tmp/unicode_tables.js" | grep -c '^[<>]') changed lines; newer Unicode data)"
    fi

    lexer_checks "$name" "$bin" "$tmp" "$same_tables"
    parser_checks "$name" "$bin" "$tmp"
    scope_checks "$name" "$bin" "$tmp"

    rm -rf "$tmp"
}

# The parser against the reference: hand-built trees, corpus,
# hand-written and generated sources, nesting depth, and timing. Uses the
# lexer inputs lexer_checks left in $tmp.
parser_checks() {
    name=$1
    bin=$2
    tmp=$3
    diff_args="--root . --standin $bin/standin --refdump $bin/refdump"

    step "hand-built trees match the reference dump ($name)"
    for f in tests/parse/trees/*; do
        printf -- '-- %s\n' "$(basename "$f")"
        case $f in
        *.mjs) "$bin/refdump" --ast "$f" --module ;;
        *) "$bin/refdump" --ast "$f" ;;
        esac
    done > "$tmp/trees.txt"
    diff -u tests/unit/ast_dump.test.expected "$tmp/trees.txt"

    step "syntax tree differential: corpus, tests/parse, tests/lex ($name)"
    corpus=$(corpus_dirs)
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args --known tests/parse/KNOWN.txt tests/parse tests/lex "$tmp/starts" $corpus

    step "syntax tree differential: generated lexer inputs ($name)"
    if [ "${FULL:-0}" = 1 ]; then
        sample=1
    else
        sample=5
    fi
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args --sample "$sample" "$tmp/cases"
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args "$tmp/errors"

    step "syntax tree differential: nesting depth ($name)"
    "$bin/progen" depth --out "$tmp/depth"
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args "$tmp/depth"

    step "syntax tree differential: generated programs ($name)"
    if [ "${FULL:-0}" = 1 ]; then
        sample=1
    else
        sample=10
    fi
    "$bin/progen" programs --seed 1 --count 20000 --out "$tmp/programs.batch"
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args --sample "$sample" "$tmp/programs.batch"

    step "parser timing on the stand-in ($name)"
    if [ "${FULL:-0}" = 1 ]; then
        bytes=1048576
    else
        bytes=262144
    fi
    "$bin/progen" big --seed 3 --bytes "$bytes" --out "$tmp/program.js"
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args "$tmp/program.js"
    # shellcheck disable=SC2086
    "$bin/bundle" astdiff $diff_args --time "$tmp/program.js"
}

# Scope analysis: the parser unit tests are up to date, the self-check,
# and the differential with the reference's scope pass. Uses the
# generated programs parser_checks left in $tmp.
scope_checks() {
    name=$1
    bin=$2
    tmp=$3
    diff_args="--root . --standin $bin/standin --refdump $bin/refdump"
    if [ "${FULL:-0}" = 1 ]; then
        sample=1
    else
        sample=10
    fi

    step "parser unit tests match unitgen ($name)"
    "$bin/unitgen" --root . --refdump "$bin/refdump" --check

    step "scope self-check: corpus, tests/parse, tests/scope ($name)"
    corpus=$(corpus_dirs)
    # shellcheck disable=SC2086
    "$bin/bundle" scopecheck --root . --standin "$bin/standin" tests/parse tests/scope tests/lex $corpus

    step "scope self-check: generated programs ($name)"
    "$bin/bundle" scopecheck --root . --standin "$bin/standin" --sample "$sample" "$tmp/programs.batch"

    step "scope differential: corpus, tests/parse, tests/scope ($name)"
    # shellcheck disable=SC2086
    "$bin/bundle" scopediff $diff_args --known tests/scope/KNOWN.txt tests/parse tests/scope $corpus

    step "scope differential: generated programs ($name)"
    # shellcheck disable=SC2086
    "$bin/bundle" scopediff $diff_args --known tests/scope/KNOWN.txt --sample "$sample" "$tmp/programs.batch"
}

# The lexer against the reference: corpus, hand-written and generated
# inputs, the identifier scan, and timing.
lexer_checks() {
    name=$1
    bin=$2
    tmp=$3
    same_tables=$4
    diff_args="--root . --standin $bin/standin --refdump $bin/refdump"

    step "token differential: corpus and tests/lex ($name)"
    corpus=$(corpus_dirs)
    "$bin/lexgen" starts --out "$tmp/starts"
    # shellcheck disable=SC2086
    "$bin/bundle" lexdiff $diff_args --known tests/lex/KNOWN.txt tests/lex "$tmp/starts" $corpus

    step "token differential: generated cases ($name)"
    "$bin/lexgen" cases --seed 1 --count 3000 --out "$tmp/cases"
    "$bin/lexgen" errors --seed 2 --out "$tmp/errors"
    if [ "${FULL:-0}" = 1 ]; then
        sample=1
    else
        sample=5
    fi
    # shellcheck disable=SC2086
    "$bin/bundle" lexdiff $diff_args --sample "$sample" "$tmp/cases"
    # shellcheck disable=SC2086
    "$bin/bundle" lexdiff $diff_args "$tmp/errors"

    step "identifier scan against the reference ($name)"
    if [ "${FULL:-0}" = 1 ]; then
        scan=1
    else
        scan=101
    fi
    if [ "$same_tables" = 1 ]; then
        # shellcheck disable=SC2086
        "$bin/bundle" lexdiff $diff_args --id-scan "$scan"
    else
        # The engine keeps the Rust 1.85.1 classes for now, so the scan
        # would only show the newer Unicode data.
        echo "note: skipped, the identifier tables differ (see above)"
    fi

    step "lexer timing on the stand-in ($name)"
    if [ "${FULL:-0}" = 1 ]; then
        bytes=1048576
    else
        bytes=262144
    fi
    "$bin/lexgen" big --seed 3 --bytes "$bytes" --out "$tmp/big.js"
    if [ "${FULL:-0}" = 1 ]; then
        # shellcheck disable=SC2086
        "$bin/bundle" lexdiff $diff_args "$tmp/big.js"
    fi
    # shellcheck disable=SC2086
    "$bin/bundle" lexdiff $diff_args --time "$tmp/big.js"
}

strict_tables=${STRICT_TABLES:-0}
pass "1.99.1: $(cargo +1.99.1 --version)" target/msrv cargo +1.99.1

if [ "${SKIP_STABLE:-0}" != "1" ]; then
    # The stable toolchain's own bin directory goes first on PATH, so its
    # rustc, rustdoc and clippy are the ones used, not another found earlier.
    RUSTUP=${RUSTUP:-rustup}
    STABLE_BIN=${STABLE_BIN:-$(dirname "$("$RUSTUP" which --toolchain stable cargo)")}
    PATH="$STABLE_BIN:$PATH"
    export PATH
    pass "stable: $("$STABLE_BIN/cargo" --version)" target/stable "$STABLE_BIN/cargo"
fi

step "house rules: headers, wording, paths, no un-safe Rust"
rm -f tools/Cargo.lock
"$ROOT/target/msrv/release/bundle" house --root .

printf '\nall checks passed\n'
