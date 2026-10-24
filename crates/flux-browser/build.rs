use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn link_openssl_soname() {
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

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    link_openssl_soname();

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        if let Ok(out_dir) = env::var("OUT_DIR") {
            let manifest_path = PathBuf::from(out_dir).join("flux.manifest");
            let manifest_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity
    version="0.16.0.0"
    processorArchitecture="amd64"
    name="Buntos.FluxBrowser"
    type="win32"
  />
  <description>Flux Browser by Buntos</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}" />
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}" />
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}" />
    </application>
  </compatibility>
</assembly>
"#;
            if fs::write(&manifest_path, manifest_xml).is_ok() {
                println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
                println!(
                    "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
                    manifest_path.display()
                );
            }
        }
    }
}
