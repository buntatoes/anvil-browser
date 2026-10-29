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

fn main() {
    println!("cargo:rerun-if-changed={UI_DIR}/anvil_core_abi.h");
    for (src, _) in UI_UNITS {
        println!("cargo:rerun-if-changed={src}");
        if !Path::new(src).exists() {
            return;
        }
    }
    println!("cargo:rustc-check-cfg=cfg(anvil_cpp_ui)");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    if cfg!(target_os = "windows") {
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
