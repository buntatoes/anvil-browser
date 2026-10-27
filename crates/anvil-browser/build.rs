// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Build script for the `anvil-browser` package.
//!
//! Cargo runs this before `src/main.rs` and `src/bin/anvil-setup.rs` are linked.
//! On Windows with the MSVC toolchain it writes an application manifest and
//! passes that file to the linker for every binary in the package. It does not
//! compile other languages, and it does not embed page or asset bytes. Those
//! `include_str!` and `include_bytes!` calls live in the crate sources.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Write the Windows manifest when the target is MSVC, then emit linker arguments.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // `/MANIFEST:EMBED` is an MSVC link flag. Other targets skip this block.
    if target_os == "windows"
        && target_env == "msvc"
        && let Ok(out_dir) = env::var("OUT_DIR")
    {
        let manifest_path = PathBuf::from(out_dir).join("anvil.manifest");
        let manifest_template = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity
    version="@ASSEMBLY_VERSION@"
    processorArchitecture="amd64"
    name="Buntato.AnvilBrowser"
    type="win32"
  />
  <description>Anvil Browser by Buntato</description>
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
        let manifest_xml = manifest_template.replace("@ASSEMBLY_VERSION@", &assembly_version());
        // The manifest asks for asInvoker (no elevation) and lists Windows
        // compatibility identifiers. A failed write leaves the binaries unmanifested.
        if fs::write(&manifest_path, manifest_xml).is_ok() {
            println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
                manifest_path.display()
            );
        }
    }
}

/// The package version as a four-part Windows assembly version.
///
/// `0.22.0` becomes `0.22.0.0`. A pre-release or build suffix is dropped, and
/// a part that is not a plain number becomes 0.
fn assembly_version() -> String {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut parts: Vec<u16> = core
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .take(3)
        .collect();
    parts.resize(4, 0);
    parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(".")
}
