//! Cross-platform native installer and uninstaller for Windows and Linux.
//!
//! - **Windows**: Installs `flux.exe` (and bundled `examples/` if present) into
//!   `%LOCALAPPDATA%\Programs\Flux Browser`, creates Start Menu and Desktop
//!   shortcuts (`Flux Browser.lnk`), and registers the per-user uninstaller key
//!   under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FluxBrowser`.
//! - **Linux**: Installs `flux` into `~/.local/share/flux-browser/flux` and
//!   `~/.local/bin/flux`, and writes an XDG desktop entry to
//!   `~/.local/share/applications/flux-browser.desktop`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use flux_compress::{compress_zlib, decompress_zlib_bounded};

use crate::VERSION;
use crate::logo::{encode_flux_ico, encode_flux_png};

/// Magic 8-byte trailer footer identifying a self-extracting Flux Setup binary.
pub const SFX_MAGIC: &[u8; 8] = b"FLUXSFX1";

/// Maximum decompressed size of an embedded `flux.exe` binary payload (32 MiB).
pub const MAX_SFX_BINARY_BYTES: usize = 32 * 1024 * 1024;

const EXAMPLE_HELLO_HTML: &str = include_str!("../../../examples/hello.html");
const EXAMPLE_SECOND_HTML: &str = include_str!("../../../examples/second.html");
const EXAMPLE_THEME_CSS: &str = include_str!("../../../examples/theme.css");

/// Combine a compiled `flux-setup` stub executable and a compiled `flux` browser executable
/// into a single self-extracting installer binary (`[setup_exe][zlib(flux_exe)][u64_len]["FLUXSFX1"]`).
pub fn pack_sfx_installer(setup_exe: &[u8], browser_exe: &[u8]) -> Vec<u8> {
    let compressed = compress_zlib(browser_exe);
    let payload_len = (compressed.len() as u64).to_le_bytes();
    let mut out = Vec::with_capacity(setup_exe.len() + compressed.len() + 16);
    out.extend_from_slice(setup_exe);
    out.extend_from_slice(&compressed);
    out.extend_from_slice(&payload_len);
    out.extend_from_slice(SFX_MAGIC);
    out
}

/// Inspect an installer binary's tail for a `FLUXSFX1` trailer and decompress the embedded `flux` binary.
pub fn extract_sfx_payload(installer_bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if installer_bytes.len() < 16 {
        return Ok(None);
    }
    let tail_start = installer_bytes.len() - 8;
    if &installer_bytes[tail_start..] != SFX_MAGIC {
        return Ok(None);
    }
    let len_bytes: [u8; 8] = installer_bytes[tail_start - 8..tail_start]
        .try_into()
        .map_err(|_| "invalid SFX length trailer".to_string())?;
    let payload_len = u64::from_le_bytes(len_bytes) as usize;
    if payload_len == 0 || payload_len + 16 > installer_bytes.len() {
        return Err("corrupted SFX payload length".to_string());
    }
    let payload_start = installer_bytes.len() - 16 - payload_len;
    let compressed = &installer_bytes[payload_start..payload_start + payload_len];
    let decompressed = decompress_zlib_bounded(compressed, MAX_SFX_BINARY_BYTES)
        .map_err(|e| format!("could not decompress embedded browser binary: {e}"))?;
    Ok(Some(decompressed))
}

/// Return the setup stub slice without any appended `FLUXSFX1` trailer.
pub fn strip_sfx_trailer(installer_bytes: &[u8]) -> &[u8] {
    if installer_bytes.len() >= 16 {
        let tail_start = installer_bytes.len() - 8;
        if &installer_bytes[tail_start..] == SFX_MAGIC {
            if let Ok(len_bytes) = installer_bytes[tail_start - 8..tail_start].try_into() {
                let payload_len = u64::from_le_bytes(len_bytes) as usize;
                if payload_len > 0 && payload_len + 16 <= installer_bytes.len() {
                    return &installer_bytes[..installer_bytes.len() - 16 - payload_len];
                }
            }
        }
    }
    installer_bytes
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReport {
    pub install_dir: PathBuf,
    pub installed_binary: PathBuf,
    pub uninstaller_binary: PathBuf,
    pub shortcut_paths: Vec<PathBuf>,
    pub registered_os_entry: bool,
}

/// Return the default per-user installation directory for the current OS.
pub fn default_install_dir() -> Result<PathBuf, String> {
    if cfg!(windows) {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data)
                .join("Programs")
                .join("Flux Browser"));
        }
        if let Some(profile) = env::var_os("USERPROFILE") {
            return Ok(PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("Programs")
                .join("Flux Browser"));
        }
        Err("could not locate LOCALAPPDATA or USERPROFILE environment variable".to_string())
    } else {
        let home = env::var_os("HOME").ok_or("could not locate HOME environment variable")?;
        Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("flux-browser"))
    }
}

/// Install Flux Browser into the default OS per-user directory and register shortcuts/uninstaller.
pub fn install_default() -> Result<InstallReport, String> {
    let dir = default_install_dir()?;
    install_to_dir(&dir, true)
}

/// Uninstall Flux Browser from the default OS per-user directory and remove shortcuts/uninstaller.
pub fn uninstall_default() -> Result<PathBuf, String> {
    let dir = default_install_dir()?;
    uninstall_from_dir(&dir, true)?;
    Ok(dir)
}

/// Install the current Flux binary into `install_dir`.
/// When `integrate_os` is `true`, also creates desktop/menu shortcuts and OS uninstall metadata.
pub fn install_to_dir(install_dir: &Path, integrate_os: bool) -> Result<InstallReport, String> {
    fs::create_dir_all(install_dir)
        .map_err(|e| format!("could not create {}: {e}", install_dir.display()))?;

    let current_exe =
        env::current_exe().map_err(|e| format!("could not locate running executable: {e}"))?;
    let bin_name = if cfg!(windows) { "flux.exe" } else { "flux" };
    let setup_sub_name = if cfg!(windows) {
        "setup.exe"
    } else {
        "flux-setup"
    };
    let installer_subdir = install_dir.join("Installer");
    let _ = fs::create_dir_all(&installer_subdir);
    let installed_binary = install_dir.join(bin_name);
    let uninstaller_binary = installer_subdir.join(setup_sub_name);

    // 1. Check if the running executable embeds a compressed FLUXSFX1 browser payload
    let mut wrote_embedded = false;
    if let Ok(self_bytes) = fs::read(&current_exe) {
        if let Some(browser_bytes) = extract_sfx_payload(&self_bytes)? {
            fs::write(&installed_binary, &browser_bytes).map_err(|e| {
                format!(
                    "could not write extracted browser binary to {}: {e}",
                    installed_binary.display()
                )
            })?;
            let stub_bytes = strip_sfx_trailer(&self_bytes);
            let _ = fs::write(&uninstaller_binary, stub_bytes);
            wrote_embedded = true;
        }
    }

    // 2. Otherwise copy sibling `flux.exe` (or current executable) into `install_dir`
    if !wrote_embedded {
        let sibling_flux = current_exe.parent().map(|p| p.join(bin_name));
        let source_bin = match sibling_flux {
            Some(ref candidate) if candidate.exists() => candidate.clone(),
            _ => current_exe.clone(),
        };
        if source_bin != installed_binary {
            fs::copy(&source_bin, &installed_binary).map_err(|e| {
                format!(
                    "could not copy binary to {}: {e}",
                    installed_binary.display()
                )
            })?;
        }
        let setup_bin_name = if cfg!(windows) {
            "flux-setup.exe"
        } else {
            "flux-setup"
        };
        let sibling_setup = current_exe.parent().map(|p| p.join(setup_bin_name));
        let uninstaller_src = match sibling_setup {
            Some(ref candidate) if candidate.exists() => candidate.clone(),
            _ => current_exe.clone(),
        };
        if uninstaller_src != uninstaller_binary {
            let _ = fs::copy(&uninstaller_src, &uninstaller_binary);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&installed_binary, fs::Permissions::from_mode(0o755));
        let _ = fs::set_permissions(&uninstaller_binary, fs::Permissions::from_mode(0o755));
    }

    // Write bundled brand icon (`flux-logo.ico` and `flux-logo.png`) and `examples/` directory
    let ico_path = install_dir.join("flux-logo.ico");
    let png_path = install_dir.join("flux-logo.png");
    let _ = fs::write(&ico_path, encode_flux_ico());
    let _ = fs::write(&png_path, encode_flux_png());

    let examples_dst = install_dir.join("examples");
    if fs::create_dir_all(&examples_dst).is_ok() {
        let _ = fs::write(examples_dst.join("hello.html"), EXAMPLE_HELLO_HTML);
        let _ = fs::write(examples_dst.join("second.html"), EXAMPLE_SECOND_HTML);
        let _ = fs::write(examples_dst.join("theme.css"), EXAMPLE_THEME_CSS);
    }
    let examples_src = Path::new("examples");
    if examples_src.is_dir() {
        let _ = copy_dir_flat(examples_src, &examples_dst);
    }

    let mut shortcut_paths = Vec::new();
    let mut registered_os_entry = false;

    if integrate_os {
        if cfg!(windows) {
            if let Ok(paths) = create_windows_shortcuts(&installed_binary, &ico_path, install_dir) {
                shortcut_paths.extend(paths);
            }
            registered_os_entry =
                register_windows_uninstaller(&uninstaller_binary, &ico_path, install_dir).is_ok();
        } else if let Ok(paths) = install_linux_desktop_integration(&installed_binary, &png_path) {
            shortcut_paths.extend(paths);
            registered_os_entry = true;
        }
    }

    Ok(InstallReport {
        install_dir: install_dir.to_path_buf(),
        installed_binary,
        uninstaller_binary,
        shortcut_paths,
        registered_os_entry,
    })
}

/// Remove Flux Browser files from `install_dir` and optionally clean up OS shortcuts and registry entries.
pub fn uninstall_from_dir(install_dir: &Path, integrate_os: bool) -> Result<(), String> {
    if integrate_os {
        if cfg!(windows) {
            remove_windows_shortcuts();
            let _ = unregister_windows_uninstaller();
        } else {
            remove_linux_desktop_integration();
        }
    }

    if install_dir.exists() {
        if let Ok(entries) = fs::read_dir(install_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        let _ = fs::remove_dir_all(install_dir);
    }
    Ok(())
}

fn copy_dir_flat(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name() {
                let _ = fs::copy(&path, dst.join(name));
            }
        }
    }
    Ok(())
}

fn create_windows_shortcuts(
    exe_path: &Path,
    ico_path: &Path,
    work_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut created = Vec::new();
    let mut targets: Vec<(PathBuf, &Path, String)> = Vec::new();

    if let Some(appdata) = env::var_os("APPDATA") {
        let start_programs = PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        targets.push((
            start_programs.join("Flux Browser.lnk"),
            exe_path,
            format!("Flux Browser {VERSION}"),
        ));
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let desktop = PathBuf::from(profile).join("Desktop");
        if desktop.exists() {
            targets.push((
                desktop.join("Flux Browser.lnk"),
                exe_path,
                format!("Flux Browser {VERSION}"),
            ));
        }
    }

    let ico_loc = format!("{},0", ico_path.to_string_lossy());
    for (lnk, target_exe, desc) in targets {
        let script = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.WorkingDirectory = '{}'; \
             $s.IconLocation = '{}'; \
             $s.Description = '{}'; \
             $s.Save()",
            escape_ps(&lnk.to_string_lossy()),
            escape_ps(&target_exe.to_string_lossy()),
            escape_ps(&work_dir.to_string_lossy()),
            escape_ps(&ico_loc),
            escape_ps(&desc),
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status();
        if status.is_ok_and(|s| s.success()) {
            created.push(lnk);
        }
    }

    Ok(created)
}

fn remove_windows_shortcuts() {
    if let Some(appdata) = env::var_os("APPDATA") {
        let start_programs = PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        let _ = fs::remove_file(start_programs.join("Flux Browser.lnk"));
        let _ = fs::remove_file(start_programs.join("Uninstall Flux Browser.lnk"));
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let desk_lnk = PathBuf::from(profile)
            .join("Desktop")
            .join("Flux Browser.lnk");
        let _ = fs::remove_file(desk_lnk);
    }
}

fn register_windows_uninstaller(
    uninstaller_path: &Path,
    ico_path: &Path,
    install_dir: &Path,
) -> Result<(), String> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FluxBrowser";
    let ico_str = ico_path.to_string_lossy();
    let uninst_str = uninstaller_path.to_string_lossy();
    let dir_str = install_dir.to_string_lossy();
    let uninstall_cmd = format!("\"{uninst_str}\" --uninstall");
    let quiet_uninstall_cmd = format!("\"{uninst_str}\" --uninstall --silent");

    let entries: &[(&str, &str, &str)] = &[
        ("DisplayName", "REG_SZ", "Flux Browser"),
        ("DisplayVersion", "REG_SZ", VERSION),
        ("Publisher", "REG_SZ", "Buntos"),
        ("InstallLocation", "REG_SZ", &dir_str),
        ("DisplayIcon", "REG_SZ", &ico_str),
        ("UninstallString", "REG_SZ", &uninstall_cmd),
        ("QuietUninstallString", "REG_SZ", &quiet_uninstall_cmd),
        ("NoModify", "REG_DWORD", "1"),
        ("NoRepair", "REG_DWORD", "1"),
    ];

    for &(name, kind, data) in entries {
        let status = Command::new("reg")
            .args(["add", key, "/v", name, "/t", kind, "/d", data, "/f"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("could not write Windows uninstall registry key".to_string());
        }
    }
    Ok(())
}

fn unregister_windows_uninstaller() -> Result<(), String> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FluxBrowser";
    let _ = Command::new("reg").args(["delete", key, "/f"]).status();
    Ok(())
}

fn install_linux_desktop_integration(
    exe_path: &Path,
    png_path: &Path,
) -> Result<Vec<PathBuf>, String> {
    let home = env::var_os("HOME").ok_or("missing HOME")?;
    let home_dir = PathBuf::from(home);
    let mut created = Vec::new();

    let local_bin = home_dir.join(".local").join("bin");
    if fs::create_dir_all(&local_bin).is_ok() {
        let bin_link = local_bin.join("flux");
        if fs::copy(exe_path, &bin_link).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&bin_link, fs::Permissions::from_mode(0o755));
            }
            created.push(bin_link);
        }
    }

    let icons_dir = home_dir
        .join(".local")
        .join("share")
        .join("icons")
        .join("hicolor")
        .join("64x64")
        .join("apps");
    if fs::create_dir_all(&icons_dir).is_ok() {
        let _ = fs::copy(png_path, icons_dir.join("flux-browser.png"));
    }

    let apps_dir = home_dir.join(".local").join("share").join("applications");
    if fs::create_dir_all(&apps_dir).is_ok() {
        let desktop_file = apps_dir.join("flux-browser.desktop");
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Flux Browser\n\
             Comment=Proprietary Web Browser ({VERSION})\n\
             Exec=\"{}\" %u\n\
             Icon={}\n\
             Terminal=false\n\
             Categories=Network;WebBrowser;\n\
             MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n",
            exe_path.display(),
            png_path.display()
        );
        if fs::write(&desktop_file, contents).is_ok() {
            created.push(desktop_file);
        }
    }

    Ok(created)
}

fn remove_linux_desktop_integration() {
    if let Some(home) = env::var_os("HOME") {
        let home_dir = PathBuf::from(home);
        let _ = fs::remove_file(home_dir.join(".local").join("bin").join("flux"));
        let _ = fs::remove_file(
            home_dir
                .join(".local")
                .join("share")
                .join("applications")
                .join("flux-browser.desktop"),
        );
    }
}

fn escape_ps(input: &str) -> String {
    input.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_and_removes_portable_installation_directory() {
        let staging = env::temp_dir().join(format!("flux_installer_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&staging);

        let report = install_to_dir(&staging, false).expect("install to staging directory");
        assert!(report.installed_binary.exists());
        assert!(report.uninstaller_binary.exists());
        assert!(staging.join("examples").join("hello.html").exists());
        assert_eq!(report.install_dir, staging);

        uninstall_from_dir(&staging, false).expect("uninstall from staging directory");
        assert!(!staging.exists());

        let fake_setup = b"MZ_SETUP_STUB";
        let fake_flux = b"MZ_FLUX_BROWSER_PAYLOAD_BINARY_DATA";
        let packed = pack_sfx_installer(fake_setup, fake_flux);
        let extracted = extract_sfx_payload(&packed)
            .expect("valid SFX")
            .expect("has SFX payload");
        assert_eq!(extracted, fake_flux);
        assert_eq!(strip_sfx_trailer(&packed), fake_setup);
    }
}
