// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

use std::env;
use std::fs;
use std::path::PathBuf;
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

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let go_repo = manifest_dir.join("../../engines/go/anvil-ttf");

    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("go.mod").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("font.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("raster.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("tables.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("cmd/anvil-ttf/main.go").display()
    );

    let bin_name = if cfg!(windows) {
        "anvil-ttf-go.exe"
    } else {
        "anvil-ttf-go"
    };
    let bin_path = out_dir.join(bin_name);
    let gocache = out_dir.join("gocache");
    let gotmp = out_dir.join("gotmp");
    let _ = fs::create_dir_all(&gocache);
    let _ = fs::create_dir_all(&gotmp);

    let go = find_go_binary();
    let status = Command::new(&go)
        .current_dir(&go_repo)
        .env("CGO_ENABLED", "0")
        .env("GOCACHE", &gocache)
        .env("GOTMPDIR", &gotmp)
        .arg("build")
        .arg("-trimpath")
        .arg("-ldflags=-s -w")
        .arg("-buildvcs=false")
        .arg("-o")
        .arg(&bin_path)
        .arg("./cmd/anvil-ttf")
        .status()
        .expect("failed to invoke Go compiler for anvil-ttf");

    assert!(
        status.success() && bin_path.exists(),
        "go build failed for anvil-ttf"
    );

    println!(
        "cargo:rustc-env=ANVIL_TTF_GO_BIN={}",
        bin_path.to_string_lossy()
    );
}
