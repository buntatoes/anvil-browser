use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "linux" {
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let link_dir = out_dir.join("openssl-link");
    let _ = fs::create_dir_all(&link_dir);
    for (stem, file) in [("crypto", "libcrypto.so.3"), ("ssl", "libssl.so.3")] {
        let src = Path::new("/lib/x86_64-linux-gnu").join(file);
        let dst = link_dir.join(format!("lib{stem}.so"));
        if src.exists() && !dst.exists() {
            let _ = std::os::unix::fs::symlink(&src, &dst);
        }
    }
    println!("cargo:rustc-link-search=native={}", link_dir.display());
}
