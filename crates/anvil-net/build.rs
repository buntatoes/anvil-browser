// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Compiles the sibling Go modules `../../../anvil-compress`, `../../../anvil-hpack`,
//! and `../../../anvil-json` into `OUT_DIR` so `anvil-net` and `anvil-browser`
//! execute the Go compression, HPACK, and JSON engines.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_go_binary() -> PathBuf {
    if let Ok(go_bin) = env::var("ANVIL_GO_BIN") {
        let p = PathBuf::from(go_bin);
        if p.exists() {
            return p;
        }
    }
    let candidates = [
        r"C:\Program Files\Go\bin\go.exe",
        "/usr/local/go/bin/go",
        "/usr/bin/go",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("go")
}

fn build_go_tool(
    go: &Path,
    repo_dir: &Path,
    cmd_pkg: &str,
    out_bin: &Path,
    gocache: &Path,
    gotmp: &Path,
    env_var: &str,
) {
    let status = Command::new(go)
        .current_dir(repo_dir)
        .env("CGO_ENABLED", "0")
        .env("GOCACHE", gocache)
        .env("GOTMPDIR", gotmp)
        .arg("build")
        .arg("-trimpath")
        .arg("-ldflags=-s -w")
        .arg("-buildvcs=false")
        .arg("-o")
        .arg(out_bin)
        .arg(cmd_pkg)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "failed to invoke Go compiler in {}: {e}",
                repo_dir.display()
            )
        });

    assert!(
        status.success() && out_bin.exists(),
        "go build failed for {}",
        repo_dir.display()
    );
    println!("cargo:rustc-env={}={}", env_var, out_bin.to_string_lossy());
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.join("..").join("..").join("..");

    let compress_repo = root_dir.join("anvil-compress");
    let hpack_repo = root_dir.join("anvil-hpack");
    let json_repo = root_dir.join("anvil-json");

    for file in [
        compress_repo.join("go.mod"),
        compress_repo.join("checksum.go"),
        compress_repo.join("deflate.go"),
        compress_repo.join("zlib.go"),
        compress_repo.join("gzip.go"),
        compress_repo.join("cmd/anvil-compress/main.go"),
        hpack_repo.join("go.mod"),
        hpack_repo.join("hpack.go"),
        hpack_repo.join("integer.go"),
        hpack_repo.join("tables.go"),
        hpack_repo.join("table.go"),
        hpack_repo.join("huffman.go"),
        hpack_repo.join("encoder.go"),
        hpack_repo.join("decoder.go"),
        hpack_repo.join("cmd/anvil-hpack/main.go"),
        json_repo.join("go.mod"),
        json_repo.join("json.go"),
        json_repo.join("pointer.go"),
        json_repo.join("wire.go"),
        json_repo.join("cmd/anvil-json/main.go"),
    ] {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let gocache = out_dir.join("gocache");
    let gotmp = out_dir.join("gotmp");
    let _ = std::fs::create_dir_all(&gocache);
    let _ = std::fs::create_dir_all(&gotmp);

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let go = find_go_binary();

    build_go_tool(
        &go,
        &compress_repo,
        "./cmd/anvil-compress",
        &out_dir.join(format!("anvil-compress-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_COMPRESS_GO_BIN",
    );
    build_go_tool(
        &go,
        &hpack_repo,
        "./cmd/anvil-hpack",
        &out_dir.join(format!("anvil-hpack-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_HPACK_GO_BIN",
    );
    build_go_tool(
        &go,
        &json_repo,
        "./cmd/anvil-json",
        &out_dir.join(format!("anvil-json-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_JSON_GO_BIN",
    );
}
