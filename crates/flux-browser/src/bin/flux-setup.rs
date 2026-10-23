#![cfg_attr(not(test), windows_subsystem = "windows")]
//! Standalone Flux Browser Self-Extracting Setup & Uninstaller (`flux-setup`).
//!
//! Supports:
//! - Interactive Windows Setup Wizard (`flux-setup.exe` / `FluxBrowser-Setup-v.07-win64.exe`)
//! - Unattended silent installation (`flux-setup --silent` or `/S`)
//! - Self-extracting single-file bundler (`flux-setup --pack <setup.exe> <flux.exe> <out.exe>`)
//! - Uninstaller (`flux-setup --uninstall`)

use std::env;
use std::fs;
use std::process::{Command, ExitCode};

use flux_browser::VERSION;
use flux_browser::installer::{
    default_install_dir, install_default, pack_sfx_installer, uninstall_default,
};

#[cfg(windows)]
mod win_dialog {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    pub const MB_OK: u32 = 0x0000_0000;
    pub const MB_OKCANCEL: u32 = 0x0000_0001;
    pub const MB_YESNO: u32 = 0x0000_0004;
    pub const MB_ICONERROR: u32 = 0x0000_0010;
    pub const MB_ICONQUESTION: u32 = 0x0000_0020;
    pub const MB_ICONINFORMATION: u32 = 0x0000_0040;
    pub const IDOK: i32 = 1;
    pub const IDYES: i32 = 6;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            h_wnd: *mut c_void,
            lp_text: *const u16,
            lp_caption: *const u16,
            u_type: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn AttachConsole(dw_process_id: u32) -> i32;
    }

    pub fn message_box(title: &str, body: &str, flags: u32) -> i32 {
        let w_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let w_body: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { MessageBoxW(null_mut(), w_body.as_ptr(), w_title.as_ptr(), flags) }
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    unsafe {
        let _ = win_dialog::AttachConsole(0xFFFF_FFFF);
    }
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--pack") => {
            if args.len() != 4 {
                eprintln!("Usage: flux-setup --pack <setup_stub.exe> <flux.exe> <output.exe>");
                return ExitCode::FAILURE;
            }
            let setup_bytes = match fs::read(&args[1]) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Could not read {}: {e}", args[1]);
                    return ExitCode::FAILURE;
                }
            };
            let browser_bytes = match fs::read(&args[2]) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Could not read {}: {e}", args[2]);
                    return ExitCode::FAILURE;
                }
            };
            let packed = pack_sfx_installer(&setup_bytes, &browser_bytes);
            if let Err(e) = fs::write(&args[3], &packed) {
                eprintln!("Could not write {}: {e}", args[3]);
                return ExitCode::FAILURE;
            }
            println!(
                "Packaged self-extracting installer: {} ({} bytes)",
                args[3],
                packed.len()
            );
            ExitCode::SUCCESS
        }
        Some("--uninstall" | "uninstall" | "/uninstall") => match uninstall_default() {
            Ok(dir) => {
                println!(
                    "Flux Browser {VERSION} has been uninstalled from {}",
                    dir.display()
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Uninstall failed: {err}");
                ExitCode::FAILURE
            }
        },
        Some("--help" | "-h" | "help") => {
            println!("Flux Browser Setup {VERSION} (Author: Buntos)");
            println!("  flux-setup              Interactive setup wizard (installs Flux Browser)");
            println!("  flux-setup --silent     Silent unattended installation");
            println!("  flux-setup --uninstall  Remove Flux Browser and shortcuts");
            println!("  flux-setup --pack <s> <f> <o>  Bundle single-file SFX installer");
            ExitCode::SUCCESS
        }
        Some("--silent" | "/S" | "-s") => run_install(true),
        _ => run_install(false),
    }
}

fn run_install(silent: bool) -> ExitCode {
    #[cfg(windows)]
    if !silent {
        let target_dir = default_install_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "%LOCALAPPDATA%\\Programs\\Flux Browser".to_string());
        let title = format!("Flux Browser {VERSION} Setup");
        let prompt = format!(
            "Welcome to the Flux Browser {VERSION} Setup Wizard (by Buntos).\n\n\
             This will install Flux Browser {VERSION} on your computer:\n\
             • Destination: {target_dir}\n\
             • Desktop & Start Menu Shortcuts\n\
             • Windows Installed Apps / Uninstaller Entry\n\n\
             Click OK to install, or Cancel to exit."
        );
        if win_dialog::message_box(
            &title,
            &prompt,
            win_dialog::MB_OKCANCEL | win_dialog::MB_ICONINFORMATION,
        ) != win_dialog::IDOK
        {
            return ExitCode::SUCCESS;
        }
    }

    match install_default() {
        Ok(report) => {
            println!("Flux Browser {VERSION} installed successfully!");
            println!("  Location : {}", report.installed_binary.display());
            for sc in &report.shortcut_paths {
                println!("  Shortcut : {}", sc.display());
            }
            if report.registered_os_entry {
                println!("  OS Entry : Registered per-user application entry");
            }

            #[cfg(windows)]
            if !silent {
                let title = format!("Flux Browser {VERSION} Setup");
                let done_msg = format!(
                    "Flux Browser {VERSION} has been installed successfully!\n\n\
                     Installed executable:\n{}\n\n\
                     Desktop and Start Menu shortcuts have been created.\n\n\
                     Would you like to launch Flux Browser now?",
                    report.installed_binary.display()
                );
                if win_dialog::message_box(
                    &title,
                    &done_msg,
                    win_dialog::MB_YESNO | win_dialog::MB_ICONQUESTION,
                ) == win_dialog::IDYES
                {
                    let _ = Command::new(&report.installed_binary)
                        .current_dir(&report.install_dir)
                        .spawn();
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Installation failed: {err}");
            #[cfg(windows)]
            if !silent {
                let title = format!("Flux Browser {VERSION} Setup Error");
                win_dialog::message_box(
                    &title,
                    &format!("Installation failed:\n{err}"),
                    win_dialog::MB_OK | win_dialog::MB_ICONERROR,
                );
            }
            ExitCode::FAILURE
        }
    }
}
