use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "linux" {
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let link_dir = out_dir.join("openssl-link");
    let _ = fs::create_dir_all(&link_dir);
    for stem in ["ssl", "crypto"] {
        let dst = link_dir.join(format!("lib{stem}.so"));
        if let Some(src) = find_shared_lib(stem) {
            if dst.exists() {
                let _ = fs::remove_file(&dst);
            }
            if std::os::unix::fs::symlink(&src, &dst).is_err() {
                println!("cargo:warning=could not link {src:?} for HTTPS");
            }
        } else {
            println!("cargo:warning=could not find lib{stem} for HTTPS");
        }
    }
    println!("cargo:rustc-link-search=native={}", link_dir.display());
}

fn find_shared_lib(stem: &str) -> Option<PathBuf> {
    let names = [format!("lib{stem}.so.3"), format!("lib{stem}.so")];
    if let Some(path) = from_ldconfig(&names) {
        return Some(path);
    }
    let mut dirs = Vec::new();
    if let Ok(out) = Command::new("gcc").args(["-print-multiarch"]).output() {
        if out.status.success() {
            let arch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !arch.is_empty() {
                dirs.push(format!("/usr/lib/{arch}"));
                dirs.push(format!("/lib/{arch}"));
            }
        }
    }
    dirs.extend([
        "/usr/lib64".into(),
        "/lib64".into(),
        "/usr/lib".into(),
        "/lib".into(),
    ]);
    for dir in dirs {
        for name in &names {
            let path = Path::new(&dir).join(name);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

fn from_ldconfig(names: &[String]) -> Option<PathBuf> {
    let out = Command::new("ldconfig").arg("-p").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for name in names {
        for line in text.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix(name.as_str()) else {
                continue;
            };
            if !rest.starts_with(' ') && !rest.starts_with('\t') {
                continue;
            }
            let Some(path) = line.split("=>").nth(1) else {
                continue;
            };
            let path = PathBuf::from(path.trim());
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}
