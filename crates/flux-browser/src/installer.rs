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

use crate::VERSION;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallReport {
    pub install_dir: PathBuf,
    pub installed_binary: PathBuf,
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
    let sibling_flux = current_exe.parent().map(|p| p.join(bin_name));
    let source_bin = match sibling_flux {
        Some(ref candidate) if candidate.exists() => candidate.clone(),
        _ => current_exe.clone(),
    };

    let installed_binary = install_dir.join(bin_name);
    if source_bin != installed_binary {
        fs::copy(&source_bin, &installed_binary).map_err(|e| {
            format!(
                "could not copy binary to {}: {e}",
                installed_binary.display()
            )
        })?;
    }

    // Copy bundled examples/ directory if present next to source or working directory
    let examples_dst = install_dir.join("examples");
    let examples_src = Path::new("examples");
    if examples_src.is_dir() {
        let _ = copy_dir_flat(examples_src, &examples_dst);
    }

    let mut shortcut_paths = Vec::new();
    let mut registered_os_entry = false;

    if integrate_os {
        if cfg!(windows) {
            if let Ok(paths) = create_windows_shortcuts(&installed_binary, install_dir) {
                shortcut_paths.extend(paths);
            }
            registered_os_entry =
                register_windows_uninstaller(&installed_binary, install_dir).is_ok();
        } else if let Ok(paths) = install_linux_desktop_integration(&installed_binary) {
            shortcut_paths.extend(paths);
            registered_os_entry = true;
        }
    }

    Ok(InstallReport {
        install_dir: install_dir.to_path_buf(),
        installed_binary,
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

fn create_windows_shortcuts(exe_path: &Path, work_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut created = Vec::new();
    let mut targets = Vec::new();

    if let Some(appdata) = env::var_os("APPDATA") {
        targets.push(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Flux Browser.lnk"),
        );
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let desktop = PathBuf::from(profile).join("Desktop");
        if desktop.exists() {
            targets.push(desktop.join("Flux Browser.lnk"));
        }
    }

    for lnk in targets {
        let script = format!(
            "$ws = New-Object -ComObject WScript.Shell; \
             $s = $ws.CreateShortcut('{}'); \
             $s.TargetPath = '{}'; \
             $s.WorkingDirectory = '{}'; \
             $s.Description = 'Flux Browser {}'; \
             $s.Save()",
            escape_ps(&lnk.to_string_lossy()),
            escape_ps(&exe_path.to_string_lossy()),
            escape_ps(&work_dir.to_string_lossy()),
            VERSION
        );
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &script,
            ])
            .status();
        if status.is_ok_and(|s| s.success()) {
            created.push(lnk);
        }
    }

    Ok(created)
}

fn remove_windows_shortcuts() {
    if let Some(appdata) = env::var_os("APPDATA") {
        let start_lnk = PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Flux Browser.lnk");
        let _ = fs::remove_file(start_lnk);
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let desk_lnk = PathBuf::from(profile)
            .join("Desktop")
            .join("Flux Browser.lnk");
        let _ = fs::remove_file(desk_lnk);
    }
}

fn register_windows_uninstaller(exe_path: &Path, install_dir: &Path) -> Result<(), String> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FluxBrowser";
    let exe_str = exe_path.to_string_lossy();
    let dir_str = install_dir.to_string_lossy();
    let uninstall_cmd = format!("\"{exe_str}\" uninstall");

    let entries: &[(&str, &str, &str)] = &[
        ("DisplayName", "REG_SZ", "Flux Browser"),
        ("DisplayVersion", "REG_SZ", VERSION),
        ("Publisher", "REG_SZ", "Buntos"),
        ("InstallLocation", "REG_SZ", &dir_str),
        ("DisplayIcon", "REG_SZ", &exe_str),
        ("UninstallString", "REG_SZ", &uninstall_cmd),
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

fn install_linux_desktop_integration(exe_path: &Path) -> Result<Vec<PathBuf>, String> {
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

    let apps_dir = home_dir.join(".local").join("share").join("applications");
    if fs::create_dir_all(&apps_dir).is_ok() {
        let desktop_file = apps_dir.join("flux-browser.desktop");
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Flux Browser\n\
             Comment=Proprietary Web Browser ({VERSION})\n\
             Exec=\"{}\" %u\n\
             Terminal=false\n\
             Categories=Network;WebBrowser;\n\
             MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n",
            exe_path.display()
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
        assert_eq!(report.install_dir, staging);

        uninstall_from_dir(&staging, false).expect("uninstall from staging directory");
        assert!(!staging.exists());
    }
}
