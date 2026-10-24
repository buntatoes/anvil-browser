// Compiles cpp/flux_raster_accel.cpp when a C++ compiler is on PATH.
// Author: Buntos

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    if depth > 8 {
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

fn main() {
    println!("cargo:rerun-if-changed=cpp/flux_raster_accel.cpp");
    println!("cargo:rustc-check-cfg=cfg(flux_cpp_accel)");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let cpp_src = Path::new("cpp/flux_raster_accel.cpp");
    if !cpp_src.exists() {
        return;
    }

    if cfg!(target_os = "windows") {
        if let (Some(cl_exe), Some(lib_exe)) = (find_msvc_tool("cl.exe"), find_msvc_tool("lib.exe"))
        {
            let obj_path = out_dir.join("flux_raster_accel.obj");
            let lib_path = out_dir.join("flux_raster_accel.lib");

            let cl_status = Command::new(&cl_exe)
                .args([
                    "/nologo",
                    "/c",
                    "/O2",
                    "/fp:fast",
                    "/std:c++20",
                    "/Zl",
                    cpp_src.to_str().unwrap(),
                    &format!("/Fo{}", obj_path.display()),
                ])
                .status();

            if cl_status.is_ok_and(|s| s.success()) && obj_path.exists() {
                let lib_status = Command::new(&lib_exe)
                    .args([
                        "/NOLOGO",
                        &format!("/OUT:{}", lib_path.display()),
                        obj_path.to_str().unwrap(),
                    ])
                    .status();
                if lib_status.is_ok_and(|s| s.success()) && lib_path.exists() {
                    println!("cargo:rustc-link-search=native={}", out_dir.display());
                    println!("cargo:rustc-link-lib=static=flux_raster_accel");
                    println!("cargo:rustc-cfg=flux_cpp_accel");
                }
            }
        }
    } else {
        let obj_path = out_dir.join("flux_raster_accel.o");
        let lib_path = out_dir.join("libflux_raster_accel.a");
        let gpp_ok = Command::new("g++")
            .args([
                "-c",
                "-O3",
                "-std=c++20",
                "-fno-exceptions",
                "-fno-rtti",
                cpp_src.to_str().unwrap(),
                "-o",
                obj_path.to_str().unwrap(),
            ])
            .status()
            .is_ok_and(|s| s.success());
        if gpp_ok && obj_path.exists() {
            let ar_ok = Command::new("ar")
                .args([
                    "rcs",
                    lib_path.to_str().unwrap(),
                    obj_path.to_str().unwrap(),
                ])
                .status()
                .is_ok_and(|s| s.success());
            if ar_ok && lib_path.exists() {
                println!("cargo:rustc-link-search=native={}", out_dir.display());
                println!("cargo:rustc-link-lib=static=flux_raster_accel");
                println!("cargo:rustc-cfg=flux_cpp_accel");
            }
        }
    }
}
