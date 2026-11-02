// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Builds and links the native C++20 Desktop UI Chrome & Window Layer (`ui/anvil_ui_d2d.cpp`,
// `ui/anvil_ui_window.cpp`) into `anvil-browser` (`anvil_ui_shell`), linking Direct2D (`d2d1`),
// DirectWrite (`dwrite`), and Desktop Window Manager (`dwmapi`) on Windows.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const UI_DIR: &str = "../../ui";
const UI_UNITS: &[(&str, &str)] = &[
    ("../../ui/anvil_ui_d2d.cpp", "anvil_ui_d2d"),
    ("../../ui/anvil_ui_window.cpp", "anvil_ui_window"),
];

fn find_msvc_tool(tool_name: &str) -> Option<PathBuf> {
    let roots = [
        r"C:\Program Files\Microsoft Visual Studio",
        r"C:\Program Files (x86)\Microsoft Visual Studio",
    ];
    for root in roots {
        let root_path = Path::new(root);
        if !root_path.exists() {
            continue;
        }
        if let Some(found) = walk_for_host_x64_tool(root_path, tool_name, 0) {
            return Some(found);
        }
    }
    None
}

fn walk_for_host_x64_tool(dir: &Path, tool_name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 12 {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walk_for_host_x64_tool(&path, tool_name, depth + 1) {
                return Some(found);
            }
        } else if path
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(tool_name))
        {
            let full = path.to_string_lossy().replace('/', "\\");
            if full.contains("Hostx64\\x64\\") {
                return Some(path);
            }
        }
    }
    None
}

fn find_msvc_and_sdk_includes(cl_exe: &Path) -> Vec<String> {
    let mut args = Vec::new();
    // <VC\Tools\MSVC\<ver>>\bin\Hostx64\x64\cl.exe -> <VC\Tools\MSVC\<ver>>\include
    if let Some(msvc_root) = cl_exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        let msvc_inc = msvc_root.join("include");
        if msvc_inc.exists() {
            args.push(format!("/I{}", msvc_inc.display()));
        }
    }

    let kits_roots = [
        r"C:\Program Files (x86)\Windows Kits\10\Include",
        r"C:\Program Files\Windows Kits\10\Include",
    ];
    for root in kits_roots {
        let root_path = Path::new(root);
        if !root_path.exists() {
            continue;
        }
        let Ok(entries) = fs::read_dir(root_path) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("um").join("windows.h").exists())
            .collect();
        versions.sort();
        if let Some(best) = versions.pop() {
            for sub in ["ucrt", "um", "shared", "winrt"] {
                let sub_dir = best.join(sub);
                if sub_dir.exists() {
                    args.push(format!("/I{}", sub_dir.display()));
                }
            }
            break;
        }
    }
    args
}

fn find_go() -> String {
    let win_go = Path::new(r"C:\Program Files\Go\bin\go.exe");
    if win_go.exists() {
        return win_go.to_string_lossy().into_owned();
    }
    "go".to_string()
}

fn build_go_tool(
    go_bin: &str,
    repo_dir: &Path,
    cmd_pkg: &str,
    out_bin: &Path,
    gocache: &Path,
    gotmp: &Path,
    env_var: &str,
) {
    let status = Command::new(go_bin)
        .current_dir(repo_dir)
        .env("CGO_ENABLED", "0")
        .env("GOCACHE", gocache)
        .env("GOTMPDIR", gotmp)
        .args([
            "build",
            "-trimpath",
            "-ldflags=-s -w",
            "-buildvcs=false",
            "-o",
            out_bin.to_str().unwrap(),
            cmd_pkg,
        ])
        .status()
        .unwrap_or_else(|e| panic!("failed to execute go build in {}: {e}", repo_dir.display()));
    assert!(
        status.success() && out_bin.exists(),
        "go build failed for {}",
        repo_dir.display()
    );
    println!("cargo:rustc-env={}={}", env_var, out_bin.display());
}

fn build_go_engine(out_dir: &Path) {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let go_engine_dir = manifest_dir.join("../../engines/go");
    let root_dir = manifest_dir.join("../../engines/go");
    if !go_engine_dir.exists() {
        return;
    }
    for f in [
        "go.mod",
        "manifest.go",
        "archive.go",
        "nettrace.go",
        "cmd/anvil-go-engine/main.go",
    ] {
        println!("cargo:rerun-if-changed={}", go_engine_dir.join(f).display());
    }
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let go_bin = find_go();
    let gocache = out_dir.join("gocache");
    let gotmp = out_dir.join("gotmp");
    let _ = fs::create_dir_all(&gocache);
    let _ = fs::create_dir_all(&gotmp);

    build_go_tool(
        &go_bin,
        &go_engine_dir,
        "./cmd/anvil-go-engine",
        &out_dir.join(format!("anvil-go-engine{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_GO_ENGINE_BIN",
    );

    let gpu_dir = root_dir.join("anvil-gpu");
    println!(
        "cargo:rerun-if-changed={}",
        gpu_dir.join("cmd/anvil-gpu/main.go").display()
    );
    build_go_tool(
        &go_bin,
        &gpu_dir,
        "./cmd/anvil-gpu",
        &out_dir.join(format!("anvil-gpu-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_GPU_GO_BIN",
    );

    let video_dir = root_dir.join("anvil-video");
    println!(
        "cargo:rerun-if-changed={}",
        video_dir.join("cmd/anvil-video/main.go").display()
    );
    build_go_tool(
        &go_bin,
        &video_dir,
        "./cmd/anvil-video",
        &out_dir.join(format!("anvil-video-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_VIDEO_GO_BIN",
    );

    let engine_dir = root_dir.join("anvil-engine");
    println!(
        "cargo:rerun-if-changed={}",
        engine_dir.join("cmd/anvil-engine/main.go").display()
    );
    build_go_tool(
        &go_bin,
        &engine_dir,
        "./cmd/anvil-engine",
        &out_dir.join(format!("anvil-engine-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_ENGINE_GO_BIN",
    );

    let ttf_dir = root_dir.join("anvil-ttf");
    println!(
        "cargo:rerun-if-changed={}",
        ttf_dir.join("cmd/anvil-ttf/main.go").display()
    );
    build_go_tool(
        &go_bin,
        &ttf_dir,
        "./cmd/anvil-ttf",
        &out_dir.join(format!("anvil-ttf-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_TTF_GO_BIN",
    );

    let img_dir = root_dir.join("anvil-image");
    println!(
        "cargo:rerun-if-changed={}",
        img_dir.join("cmd/anvil-image/main.go").display()
    );
    build_go_tool(
        &go_bin,
        &img_dir,
        "./cmd/anvil-image",
        &out_dir.join(format!("anvil-image-go{ext}")),
        &gocache,
        &gotmp,
        "ANVIL_IMAGE_GO_BIN",
    );
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    build_go_engine(&out_dir);

    println!("cargo:rerun-if-changed={UI_DIR}/anvil_core_abi.h");
    for (src, _) in UI_UNITS {
        println!("cargo:rerun-if-changed={src}");
        if !Path::new(src).exists() {
            return;
        }
    }
    println!("cargo:rustc-check-cfg=cfg(anvil_cpp_ui)");

    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-arg=/STACK:33554432");
        if let (Some(cl_exe), Some(lib_exe)) = (find_msvc_tool("cl.exe"), find_msvc_tool("lib.exe"))
        {
            let inc_args = find_msvc_and_sdk_includes(&cl_exe);
            let mut obj_paths = Vec::with_capacity(UI_UNITS.len());
            for (src, stem) in UI_UNITS {
                let obj_path = out_dir.join(format!("{stem}.obj"));
                let mut cmd = Command::new(&cl_exe);
                cmd.args([
                    "/nologo",
                    "/c",
                    "/O2",
                    "/fp:precise",
                    "/std:c++20",
                    "/EHsc-",
                    "/GR-",
                    &format!("/I{UI_DIR}"),
                ]);
                for inc in &inc_args {
                    cmd.arg(inc);
                }
                cmd.arg(src).arg(format!("/Fo{}", obj_path.display()));
                if !cmd.status().is_ok_and(|s| s.success()) || !obj_path.exists() {
                    return;
                }
                obj_paths.push(obj_path);
            }

            let lib_path = out_dir.join("anvil_ui_shell.lib");
            let mut lib_cmd = Command::new(&lib_exe);
            lib_cmd
                .arg("/NOLOGO")
                .arg(format!("/OUT:{}", lib_path.display()));
            for obj in &obj_paths {
                lib_cmd.arg(obj.to_str().unwrap());
            }
            if lib_cmd.status().is_ok_and(|s| s.success()) && lib_path.exists() {
                println!("cargo:rustc-link-search=native={}", out_dir.display());
                println!("cargo:rustc-link-lib=static=anvil_ui_shell");
                println!("cargo:rustc-link-lib=dylib=d2d1");
                println!("cargo:rustc-link-lib=dylib=dwrite");
                println!("cargo:rustc-link-lib=dylib=dwmapi");
                println!("cargo:rustc-link-lib=dylib=gdi32");
                println!("cargo:rustc-link-lib=dylib=user32");
                println!("cargo:rustc-link-lib=dylib=ole32");
                println!("cargo:rustc-cfg=anvil_cpp_ui");
            }
        }
    } else {
        println!("cargo:rustc-link-arg=-Wl,-z,stack-size=33554432");
        let mut obj_paths = Vec::with_capacity(UI_UNITS.len());
        for (src, stem) in UI_UNITS {
            let obj_path = out_dir.join(format!("{stem}.o"));
            let gpp_ok = Command::new("g++")
                .args([
                    "-c",
                    "-O3",
                    "-std=c++20",
                    "-fno-exceptions",
                    "-fno-rtti",
                    &format!("-I{UI_DIR}"),
                    src,
                    "-o",
                    obj_path.to_str().unwrap(),
                ])
                .status()
                .is_ok_and(|s| s.success());
            if !gpp_ok || !obj_path.exists() {
                return;
            }
            obj_paths.push(obj_path);
        }

        let lib_path = out_dir.join("libanvil_ui_shell.a");
        let mut ar_cmd = Command::new("ar");
        ar_cmd.arg("rcs").arg(lib_path.to_str().unwrap());
        for obj in &obj_paths {
            ar_cmd.arg(obj.to_str().unwrap());
        }
        if ar_cmd.status().is_ok_and(|s| s.success()) && lib_path.exists() {
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=anvil_ui_shell");
            println!("cargo:rustc-cfg=anvil_cpp_ui");
        }
    }
}
