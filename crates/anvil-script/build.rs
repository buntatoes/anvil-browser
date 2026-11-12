// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
// Builds and links the native C++20 Cinder engine (`CinderVM`: NaN-boxed heap,
// hidden classes, Inline Caches, and register VM) from `../../engines/cinder/cpp/`
// directly into `anvil-script`. That directory is the only copy of these sources.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ENGINE_CPP_DIR: &str = "../../engines/cinder/cpp";

const CPP_STEMS: &[&str] = &[
    "cinder_vm_heap",
    "cinder_vm_ic",
    "cinder_vm_compiler",
    "cinder_jit_accel",
    "cinder_jit_native",
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

fn find_mingw_tool(tool_name: &str) -> PathBuf {
    let sys_path = Path::new("/usr/share/mingw-w64/bin").join(tool_name);
    if sys_path.exists() {
        return sys_path;
    }
    PathBuf::from(tool_name)
}

fn build_go_anvil_json(out_dir: &Path) {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let go_repo = manifest_dir.join("../../engines/go/anvil-json");
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("go.mod").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("json.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("pointer.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("wire.go").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        go_repo.join("cmd/anvil-json/main.go").display()
    );

    let target_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let bin_name = if target_windows {
        "anvil-json-go.exe"
    } else {
        "anvil-json-go"
    };
    let bin_path = out_dir.join(bin_name);
    let gocache = out_dir.join("gocache");
    let gotmp = out_dir.join("gotmp");
    let _ = fs::create_dir_all(&gocache);
    let _ = fs::create_dir_all(&gotmp);

    let go = find_go_binary();
    let mut cmd = Command::new(&go);
    cmd.current_dir(&go_repo)
        .env("CGO_ENABLED", "0")
        .env("GOCACHE", &gocache)
        .env("GOTMPDIR", &gotmp);
    if target_windows {
        cmd.env("GOOS", "windows").env("GOARCH", "amd64");
    }
    let status = cmd
        .arg("build")
        .arg("-trimpath")
        .arg("-ldflags=-s -w")
        .arg("-buildvcs=false")
        .arg("-o")
        .arg(&bin_path)
        .arg("./cmd/anvil-json")
        .status()
        .expect("failed to invoke Go compiler for ../../../anvil-json");

    assert!(
        status.success() && bin_path.exists(),
        "go build failed for ../../../anvil-json"
    );

    println!(
        "cargo:rustc-env=ANVIL_JSON_GO_BIN={}",
        bin_path.to_string_lossy()
    );
}

fn compiler_present(program: &Path) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Run a C++ tool. A missing compiler is handled by the caller before this.
/// A compiler that starts and then fails stops the build.
fn run_checked(cmd: &mut Command, what: &str) {
    match cmd.output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            panic!("the numeric VM was not built ({what}):\n{stdout}{stderr}");
        }
        Err(err) => {
            panic!("could not run the C++ tool for the numeric VM ({what}): {err}");
        }
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    build_go_anvil_json(&out_dir);
    println!("cargo:rerun-if-changed=../../engines/cinder/js/src/runtime/dom_web_bootstrap.js");

    let cpp_dir = ENGINE_CPP_DIR;
    println!("cargo:rerun-if-changed={cpp_dir}/cinder_vm.hpp");
    let mut units = Vec::with_capacity(CPP_STEMS.len());
    for stem in CPP_STEMS {
        let src = format!("{cpp_dir}/{stem}.cpp");
        println!("cargo:rerun-if-changed={src}");
        if !Path::new(&src).exists() {
            panic!("missing C++ source {src} (the numeric VM sources live in engines/cinder/cpp)");
        }
        units.push((src, *stem));
    }
    println!("cargo:rustc-check-cfg=cfg(cinder_cpp_jit)");
    println!("cargo:rustc-check-cfg=cfg(cinder_cpp_compiler)");

    let target_windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");

    if target_windows {
        if let (Some(cl_exe), Some(lib_exe)) = (find_msvc_tool("cl.exe"), find_msvc_tool("lib.exe"))
        {
            println!("cargo:rustc-cfg=cinder_cpp_compiler");
            let mut obj_paths = Vec::with_capacity(units.len());
            for (src, stem) in &units {
                let obj_path = out_dir.join(format!("{stem}.obj"));
                run_checked(
                    Command::new(&cl_exe).args([
                        "/nologo",
                        "/c",
                        "/O2",
                        "/fp:precise",
                        "/std:c++20",
                        "/Gs1048576",
                        "/Zl",
                        &format!("/I{cpp_dir}"),
                        src.as_str(),
                        &format!("/Fo{}", obj_path.display()),
                    ]),
                    src,
                );
                if !obj_path.exists() {
                    panic!(
                        "the numeric VM compile did not write {}",
                        obj_path.display()
                    );
                }
                obj_paths.push(obj_path);
            }

            let lib_path = out_dir.join("cinder_jit_accel.lib");
            let mut lib_cmd = Command::new(&lib_exe);
            lib_cmd
                .arg("/NOLOGO")
                .arg(format!("/OUT:{}", lib_path.display()));
            for obj in &obj_paths {
                lib_cmd.arg(obj.to_str().unwrap());
            }
            run_checked(&mut lib_cmd, "lib.exe");
            if !lib_path.exists() {
                panic!("the numeric VM library was not written");
            }
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=cinder_jit_accel");
            println!("cargo:rustc-link-lib=dylib=kernel32");
            println!("cargo:rustc-cfg=cinder_cpp_jit");
        } else {
            let gpp = find_mingw_tool("x86_64-w64-mingw32-g++");
            let ar = find_mingw_tool("x86_64-w64-mingw32-ar");
            if !compiler_present(&gpp) {
                println!("cargo:warning=no C++ compiler found; the numeric VM was not linked");
                return;
            }
            println!("cargo:rustc-cfg=cinder_cpp_compiler");
            let mut obj_paths = Vec::with_capacity(units.len());
            for (src, stem) in &units {
                let obj_path = out_dir.join(format!("{stem}.o"));
                run_checked(
                    Command::new(&gpp).args([
                        "-c",
                        "-O3",
                        "-std=c++20",
                        "-fno-exceptions",
                        "-fno-rtti",
                        &format!("-I{cpp_dir}"),
                        src.as_str(),
                        "-o",
                        obj_path.to_str().unwrap(),
                    ]),
                    src,
                );
                if !obj_path.exists() {
                    panic!(
                        "the numeric VM compile did not write {}",
                        obj_path.display()
                    );
                }
                obj_paths.push(obj_path);
            }

            let lib_path = out_dir.join("libcinder_jit_accel.a");
            let mut ar_cmd = Command::new(&ar);
            ar_cmd.arg("rcs").arg(lib_path.to_str().unwrap());
            for obj in &obj_paths {
                ar_cmd.arg(obj.to_str().unwrap());
            }
            run_checked(&mut ar_cmd, "ar");
            if !lib_path.exists() {
                panic!("the numeric VM library was not written");
            }
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=cinder_jit_accel");
            println!("cargo:rustc-link-lib=dylib=kernel32");
            println!("cargo:rustc-cfg=cinder_cpp_jit");
        }
    } else if compiler_present(Path::new("g++")) {
        println!("cargo:rustc-cfg=cinder_cpp_compiler");
        let mut obj_paths = Vec::with_capacity(units.len());
        for (src, stem) in &units {
            let obj_path = out_dir.join(format!("{stem}.o"));
            run_checked(
                Command::new("g++").args([
                    "-c",
                    "-O3",
                    "-std=c++20",
                    "-fno-exceptions",
                    "-fno-rtti",
                    &format!("-I{cpp_dir}"),
                    src.as_str(),
                    "-o",
                    obj_path.to_str().unwrap(),
                ]),
                src,
            );
            if !obj_path.exists() {
                panic!(
                    "the numeric VM compile did not write {}",
                    obj_path.display()
                );
            }
            obj_paths.push(obj_path);
        }

        let lib_path = out_dir.join("libcinder_jit_accel.a");
        let mut ar_cmd = Command::new("ar");
        ar_cmd.arg("rcs").arg(lib_path.to_str().unwrap());
        for obj in &obj_paths {
            ar_cmd.arg(obj.to_str().unwrap());
        }
        run_checked(&mut ar_cmd, "ar");
        if !lib_path.exists() {
            panic!("the numeric VM library was not written");
        }
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=cinder_jit_accel");
        println!("cargo:rustc-cfg=cinder_cpp_jit");
    } else {
        println!("cargo:warning=no C++ compiler found; the numeric VM was not linked");
    }
}
