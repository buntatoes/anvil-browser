// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
#![cfg_attr(not(test), windows_subsystem = "windows")]
//! Setup and uninstall binary (`anvil-setup`).
//!
//! `main` reads argv and calls `installer::install_default`, `uninstall_default`,
//! or `pack_sfx_installer`. The pack path uses `anvil-compress` inside that
//! installer function. Windows message boxes come from `user32` in this file.
//!
//! Flags this binary actually accepts:
//! - no command: install
//! - `--silent`, `/S`, `-s`, `/quiet`: install or uninstall without message boxes
//! - `--uninstall`, `uninstall`, `/uninstall`: uninstall
//! - a file name containing `uninstall`: uninstall, even with no command flag
//! - `--pack <setup stub> <browser exe> <output exe>`: append a compressed trailer
//! - `--help`, `-h`, `help`: print the command list

use std::env;
use std::fs;
use std::process::{Command, ExitCode};

use anvil_browser::VERSION;
use anvil_browser::installer::{
    default_install_dir, install_default, pack_sfx_installer, uninstall_default,
};

#[cfg(not(windows))]
mod linux_dialog {
    use std::env;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use anvil_browser::keys::Key;
    use anvil_browser::wayland::{
        BTN_LEFT, WaylandCursorShape, WaylandEvent, WaylandWindow, resolve_wayland_socket_path,
    };
    use anvil_browser::{VERSION, Viewport, render_html};
    use anvil_raster::{Framebuffer, rasterize};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum WizardChoice {
        InstallAndLaunch,
        InstallOnly,
        Uninstall,
        Cancel,
    }

    fn has_gui_session() -> bool {
        resolve_wayland_socket_path().is_some()
            || env::var_os("WAYLAND_DISPLAY").is_some()
            || env::var_os("DISPLAY").is_some()
    }

    fn render_wizard_html(
        width: u32,
        height: u32,
        already_installed: bool,
        installing: bool,
        target_dir: &str,
    ) -> Framebuffer {
        let status_badge = if installing {
            "EXTRACTING ENGINE & REGISTERING SHORTCUTS..."
        } else if already_installed {
            "EXISTING INSTALLATION DETECTED"
        } else {
            "READY TO INSTALL"
        };
        let primary_label = if installing {
            "Installing..."
        } else if already_installed {
            "Update & Launch"
        } else {
            "Install & Launch"
        };
        let secondary_label = if already_installed {
            "Uninstall"
        } else {
            "Install Only"
        };
        let secondary_bg = if already_installed {
            "#3b181e"
        } else {
            "#1e2536"
        };
        let secondary_fg = if already_installed {
            "#fca5a5"
        } else {
            "#e2e8f0"
        };

        let html = format!(
            "<!doctype html><html><head><style>\
             body {{ margin: 0; padding: 28px 32px; background: #0f131c; color: #f8fafc; font-family: sans-serif; }}\
             .badge {{ display: inline-block; padding: 4px 10px; border-radius: 6px; background: #261b16; color: #ff8547; font-size: 11px; font-weight: bold; margin-bottom: 10px; }}\
             h1 {{ margin: 0 0 6px 0; font-size: 24px; color: #ffffff; }}\
             .sub {{ margin: 0 0 18px 0; font-size: 13px; color: #94a3b8; }}\
             .card {{ background: #161c28; border: 1px solid #263044; border-radius: 12px; padding: 16px 18px; margin-bottom: 22px; }}\
             .row {{ margin: 6px 0; font-size: 13px; color: #cbd5e1; }}\
             .lbl {{ color: #94a3b8; font-weight: bold; }}\
             .hint {{ font-size: 12px; color: #64748b; margin-bottom: 18px; }}\
             .btns {{ display: flex; flex-direction: row; }}\
             .btn-pri {{ display: inline-block; width: 200px; padding: 12px 16px; border-radius: 10px; background: #ff6b24; color: #ffffff; font-size: 14px; font-weight: bold; text-align: center; margin-right: 14px; }}\
             .btn-sec {{ display: inline-block; width: 150px; padding: 12px 16px; border-radius: 10px; background: {secondary_bg}; color: {secondary_fg}; font-size: 14px; font-weight: bold; text-align: center; margin-right: 14px; }}\
             .btn-can {{ display: inline-block; width: 110px; padding: 12px 16px; border-radius: 10px; background: #181e2b; color: #94a3b8; font-size: 14px; text-align: center; }}\
             </style></head><body>\
             <div class=\"badge\">{status_badge}</div>\
             <h1>Anvil Browser {VERSION} Setup</h1>\
             <p class=\"sub\">Forge Rendering Engine · Cinder JS VM &amp; JIT · Prism CSS</p>\
             <div class=\"card\">\
               <div class=\"row\"><span class=\"lbl\">Install Location:</span> {target_dir}</div>\
               <div class=\"row\"><span class=\"lbl\">CLI Symlink:</span> ~/.local/bin/anvil</div>\
               <div class=\"row\"><span class=\"lbl\">Desktop Integration:</span> Application Menu &amp; Desktop Shortcut</div>\
             </div>\
             <div class=\"hint\">Press Enter to {primary_label}, or click an action below:</div>\
             <div class=\"btns\">\
               <span class=\"btn-pri\">{primary_label}</span>\
               <span class=\"btn-sec\">{secondary_label}</span>\
               <span class=\"btn-can\">Cancel</span>\
             </div>\
             </body></html>"
        );

        let vp = Viewport {
            width: width as f32,
            height: height as f32,
        };
        let page = render_html(&html, "anvil://setup", vp);
        rasterize(&page.display_list, width.max(1), height.max(1), 0.0)
    }

    /// Open a native Wayland Setup Wizard window and run the interactive setup flow.
    pub fn run_wayland_setup_wizard(
        already_installed: bool,
        target_dir: &str,
    ) -> Option<(WizardChoice, Option<WaylandWindow>)> {
        resolve_wayland_socket_path()?;
        let mut win_w = 640u32;
        let mut win_h = 400u32;
        let title = format!("Anvil Browser {VERSION} Setup");
        let mut wl = WaylandWindow::connect(&title, win_w, win_h).ok()?;
        let (cfg_w, cfg_h, _) = wl.configured_size();
        if cfg_w >= 360 && cfg_h >= 240 {
            win_w = cfg_w;
            win_h = cfg_h;
        }
        let mut fb = render_wizard_html(win_w, win_h, already_installed, false, target_dir);
        let _ = wl.present_framebuffer(&fb);

        loop {
            let events = wl.poll_events().ok()?;
            if events.is_empty() {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            for ev in events {
                match ev {
                    WaylandEvent::Close => return Some((WizardChoice::Cancel, None)),
                    WaylandEvent::Configure { width, height, .. } => {
                        if width >= 360 && height >= 240 && (width != win_w || height != win_h) {
                            win_w = width;
                            win_h = height;
                            fb = render_wizard_html(
                                win_w,
                                win_h,
                                already_installed,
                                false,
                                target_dir,
                            );
                        }
                        let _ = wl.present_framebuffer(&fb);
                    }
                    WaylandEvent::PointerMotion { x, y } => {
                        let on_button = (280.0..=385.0).contains(&y)
                            && (24.0..=(win_w as f32 - 24.0)).contains(&x);
                        wl.set_cursor_shape(if on_button {
                            WaylandCursorShape::Pointer
                        } else {
                            WaylandCursorShape::Default
                        });
                    }
                    WaylandEvent::PointerButton {
                        button: BTN_LEFT,
                        pressed: true,
                        x,
                        y,
                        ..
                    } => {
                        if (275.0..=390.0).contains(&y) {
                            if (24.0..=255.0).contains(&x) {
                                let busy_fb = render_wizard_html(
                                    win_w,
                                    win_h,
                                    already_installed,
                                    true,
                                    target_dir,
                                );
                                let _ = wl.present_framebuffer(&busy_fb);
                                return Some((WizardChoice::InstallAndLaunch, Some(wl)));
                            } else if (260.0..=445.0).contains(&x) {
                                if already_installed {
                                    return Some((WizardChoice::Uninstall, Some(wl)));
                                } else {
                                    let busy_fb = render_wizard_html(
                                        win_w,
                                        win_h,
                                        already_installed,
                                        true,
                                        target_dir,
                                    );
                                    let _ = wl.present_framebuffer(&busy_fb);
                                    return Some((WizardChoice::InstallOnly, Some(wl)));
                                }
                            } else if x >= 450.0 {
                                return Some((WizardChoice::Cancel, None));
                            }
                        } else if y < 56.0 && x < 200.0 {
                            wl.begin_interactive_move();
                        }
                    }
                    WaylandEvent::KeyPress { key, .. } => match key {
                        Key::Enter | Key::Char(' ') | Key::Char('y') | Key::Char('Y') => {
                            let busy_fb = render_wizard_html(
                                win_w,
                                win_h,
                                already_installed,
                                true,
                                target_dir,
                            );
                            let _ = wl.present_framebuffer(&busy_fb);
                            return Some((WizardChoice::InstallAndLaunch, Some(wl)));
                        }
                        Key::Char('i') | Key::Char('I') => {
                            let busy_fb = render_wizard_html(
                                win_w,
                                win_h,
                                already_installed,
                                true,
                                target_dir,
                            );
                            let _ = wl.present_framebuffer(&busy_fb);
                            return Some((WizardChoice::InstallOnly, Some(wl)));
                        }
                        Key::Char('u') | Key::Char('U') if already_installed => {
                            return Some((WizardChoice::Uninstall, Some(wl)));
                        }
                        Key::Escape | Key::Char('n') | Key::Char('N') => {
                            return Some((WizardChoice::Cancel, None));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }

    pub fn ask_yes_no(title: &str, body: &str) -> Option<bool> {
        if !has_gui_session() {
            return None;
        }
        if let Ok(out) = Command::new("kdialog")
            .args(["--title", title, "--yesno", body])
            .output()
        {
            match out.status.code() {
                Some(0) => return Some(true),
                Some(1) if out.stderr.is_empty() => return Some(false),
                _ => {}
            }
        }
        if let Ok(out) = Command::new("zenity")
            .args([
                "--question",
                "--title",
                title,
                "--text",
                body,
                "--no-wrap",
                "--no-markup",
            ])
            .output()
        {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.contains("Failed to open display") && !stderr.contains("cannot open display")
            {
                match out.status.code() {
                    Some(0) => return Some(true),
                    Some(1) => return Some(false),
                    _ => {}
                }
            }
        }
        None
    }

    pub fn show_info(title: &str, body: &str) {
        if !has_gui_session() {
            return;
        }
        if Command::new("kdialog")
            .args(["--title", title, "--msgbox", body])
            .status()
            .is_ok_and(|s| s.success())
        {
            return;
        }
        let _ = Command::new("zenity")
            .args([
                "--info",
                "--title",
                title,
                "--text",
                body,
                "--no-wrap",
                "--no-markup",
            ])
            .status();
    }

    pub fn show_error(title: &str, body: &str) {
        if !has_gui_session() {
            return;
        }
        if Command::new("kdialog")
            .args(["--title", title, "--error", body])
            .status()
            .is_ok_and(|s| s.success())
        {
            return;
        }
        let _ = Command::new("zenity")
            .args([
                "--error",
                "--title",
                title,
                "--text",
                body,
                "--no-wrap",
                "--no-markup",
            ])
            .status();
    }
}

/// Windows message-box flags and the `MessageBoxW` call used by the wizard prompts.
#[cfg(windows)]
mod win_dialog {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    /// `MessageBoxW` style: one OK button.
    pub const MB_OK: u32 = 0x0000_0000;
    /// `MessageBoxW` style: OK and Cancel.
    pub const MB_OKCANCEL: u32 = 0x0000_0001;
    /// `MessageBoxW` style: Yes, No, and Cancel.
    pub const MB_YESNOCANCEL: u32 = 0x0000_0003;
    /// `MessageBoxW` style: Yes and No.
    pub const MB_YESNO: u32 = 0x0000_0004;
    /// `MessageBoxW` icon: error.
    pub const MB_ICONERROR: u32 = 0x0000_0010;
    /// `MessageBoxW` icon: question.
    pub const MB_ICONQUESTION: u32 = 0x0000_0020;
    /// `MessageBoxW` icon: information.
    pub const MB_ICONINFORMATION: u32 = 0x0000_0040;
    /// `MessageBoxW` return: OK.
    pub const IDOK: i32 = 1;
    /// `MessageBoxW` return: Yes.
    pub const IDYES: i32 = 6;
    /// `MessageBoxW` return: No.
    pub const IDNO: i32 = 7;

    #[link(name = "user32")]
    unsafe extern "system" {
        /// Win32 message box. `message_box` is the only caller.
        fn MessageBoxW(
            h_wnd: *mut c_void,
            lp_text: *const u16,
            lp_caption: *const u16,
            u_type: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// Attach this process to a console. `main` passes the parent-process id.
        pub fn AttachConsole(dw_process_id: u32) -> i32;
    }

    /// Show a UTF-16 message box and return the button id.
    pub fn message_box(title: &str, body: &str, flags: u32) -> i32 {
        let w_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let w_body: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { MessageBoxW(null_mut(), w_body.as_ptr(), w_title.as_ptr(), flags) }
    }
}

/// Dispatch pack, uninstall, help, or install. Silent flags skip the Windows prompts.
fn main() -> ExitCode {
    #[cfg(windows)]
    unsafe {
        let _ = win_dialog::AttachConsole(0xFFFF_FFFF);
    }
    let args: Vec<String> = env::args().skip(1).collect();
    let silent = args
        .iter()
        .any(|a| matches!(a.as_str(), "--silent" | "/S" | "-s" | "/quiet"));
    let invoked_as_uninstaller = env::current_exe()
        .ok()
        .and_then(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
        })
        .is_some_and(|name| name.contains("uninstall"));

    match args.first().map(String::as_str) {
        // Read two binaries and write `[stub][zlib(browser)][length][ANVILSFX]`.
        Some("--pack") => {
            if args.len() != 4 {
                eprintln!("Usage: anvil-setup --pack <setup_stub.exe> <anvil.exe> <output.exe>");
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
        Some("--uninstall" | "uninstall" | "/uninstall") => run_uninstall(silent),
        // Print the flag list and exit success.
        Some("--help" | "-h" | "help") => {
            println!("Anvil Browser Setup & Uninstaller {VERSION} (Author: Buntato)");
            println!(
                "  anvil-setup              Interactive setup wizard (installs Anvil Browser)"
            );
            println!("  anvil-setup --silent     Silent unattended installation");
            println!("  anvil-setup --uninstall  Remove Anvil Browser and shortcuts");
            println!("  anvil-setup --pack <s> <f> <o>  Bundle single-file SFX installer");
            ExitCode::SUCCESS
        }
        // The executable name itself contains "uninstall".
        _ if invoked_as_uninstaller => run_uninstall(silent),
        // Any other argv, including none, installs.
        _ => run_install(silent),
    }
}

/// Ask before uninstall unless `silent` is set, then call `uninstall_default`.
fn run_uninstall(silent: bool) -> ExitCode {
    #[cfg(not(windows))]
    if !silent {
        let target_dir = default_install_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/.local/share/anvil-browser".to_string());
        let title = format!("Anvil Browser {VERSION} Uninstaller");
        let prompt = format!(
            "Are you sure you want to completely uninstall Anvil Browser {VERSION}?\n\n\
             This will remove:\n\
             - Application files in {target_dir}\n\
             - ~/.local/bin/anvil and desktop application entry"
        );
        if let Some(false) = linux_dialog::ask_yes_no(&title, &prompt) {
            return ExitCode::SUCCESS;
        }
    }
    #[cfg(windows)]
    if !silent {
        let target_dir = default_install_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "%LOCALAPPDATA%\\Programs\\Anvil Browser".to_string());
        let title = format!("Anvil Browser {VERSION} Uninstaller");
        let prompt = format!(
            "Are you sure you want to completely uninstall Anvil Browser {VERSION}?\n\n\
             This will remove:\n\
             • Application files in {target_dir}\n\
             • Desktop and Start Menu shortcuts\n\
             • Windows Installed Apps registry entry\n\n\
             Click Yes to uninstall, or No to cancel."
        );
        if win_dialog::message_box(
            &title,
            &prompt,
            win_dialog::MB_YESNO | win_dialog::MB_ICONQUESTION,
        ) != win_dialog::IDYES
        {
            return ExitCode::SUCCESS;
        }
    }

    match uninstall_default() {
        Ok(dir) => {
            println!(
                "Anvil Browser {VERSION} has been uninstalled from {}",
                dir.display()
            );
            #[cfg(not(windows))]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Uninstaller");
                linux_dialog::show_info(
                    &title,
                    &format!(
                        "Anvil Browser {VERSION} has been completely uninstalled from your computer."
                    ),
                );
            }
            #[cfg(windows)]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Uninstaller");
                win_dialog::message_box(
                    &title,
                    &format!(
                        "Anvil Browser {VERSION} has been completely uninstalled from your computer."
                    ),
                    win_dialog::MB_OK | win_dialog::MB_ICONINFORMATION,
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Uninstall failed: {err}");
            #[cfg(not(windows))]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Uninstaller Error");
                linux_dialog::show_error(&title, &format!("Uninstall failed:\n{err}"));
            }
            #[cfg(windows)]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Uninstaller Error");
                win_dialog::message_box(
                    &title,
                    &format!("Uninstall failed:\n{err}"),
                    win_dialog::MB_OK | win_dialog::MB_ICONERROR,
                );
            }
            ExitCode::FAILURE
        }
    }
}

/// Ask before install unless `silent` is set, then call `install_default`.
///
/// If `anvil` / `anvil.exe` is already in the default directory, Yes reinstalls, No uninstalls,
/// and Cancel returns success without changing files. After a successful install the
/// Yes button on the last prompt spawns the installed binary. Silent mode skips both prompts.
fn run_install(silent: bool) -> ExitCode {
    #[cfg(not(windows))]
    let mut launch_after_install = !silent;
    #[cfg(not(windows))]
    let mut used_native_wizard = false;
    #[cfg(not(windows))]
    let mut active_wizard_window = None;

    #[cfg(not(windows))]
    if !silent {
        let install_dir = default_install_dir().ok();
        let already_installed = install_dir
            .as_ref()
            .is_some_and(|dir| dir.join("anvil").exists());
        let target_dir = install_dir
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.local/share/anvil-browser".to_string());
        if let Some((choice, wl_win)) =
            linux_dialog::run_wayland_setup_wizard(already_installed, &target_dir)
        {
            used_native_wizard = true;
            active_wizard_window = wl_win;
            match choice {
                linux_dialog::WizardChoice::Cancel => return ExitCode::SUCCESS,
                linux_dialog::WizardChoice::Uninstall => {
                    drop(active_wizard_window);
                    return run_uninstall(true);
                }
                linux_dialog::WizardChoice::InstallOnly => {
                    launch_after_install = false;
                }
                linux_dialog::WizardChoice::InstallAndLaunch => {
                    launch_after_install = true;
                }
            }
        } else {
            let title = format!("Anvil Browser {VERSION} Setup");
            let prompt = format!(
                "Welcome to the Anvil Browser {VERSION} Setup Wizard (by Buntato).\n\n\
                 This will install Anvil Browser {VERSION} on your computer:\n\
                 - Destination: {target_dir}\n\
                 - CLI binary: ~/.local/bin/anvil\n\
                 - Desktop Application Menu Entry\n\n\
                 Continue with installation?"
            );
            if let Some(false) = linux_dialog::ask_yes_no(&title, &prompt) {
                return ExitCode::SUCCESS;
            }
        }
    }
    #[cfg(windows)]
    if !silent {
        let install_dir = default_install_dir().ok();
        let already_installed = install_dir
            .as_ref()
            .is_some_and(|dir| dir.join("anvil.exe").exists());
        let target_dir = install_dir
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "%LOCALAPPDATA%\\Programs\\Anvil Browser".to_string());
        let title = format!("Anvil Browser {VERSION} Setup");
        if already_installed {
            let prompt = format!(
                "Anvil Browser is already installed in:\n{target_dir}\n\n\
                 • Click Yes to Update / Reinstall Anvil Browser {VERSION}\n\
                 • Click No to Uninstall Anvil Browser\n\
                 • Click Cancel to exit Setup"
            );
            let choice = win_dialog::message_box(
                &title,
                &prompt,
                win_dialog::MB_YESNOCANCEL | win_dialog::MB_ICONQUESTION,
            );
            if choice == win_dialog::IDNO {
                return run_uninstall(false);
            }
            if choice != win_dialog::IDYES {
                return ExitCode::SUCCESS;
            }
        } else {
            let prompt = format!(
                "Welcome to the Anvil Browser {VERSION} Setup Wizard (by Buntato).\n\n\
                 This will install Anvil Browser {VERSION} on your computer:\n\
                 • Destination: {target_dir}\n\
                 • Desktop & Start Menu Shortcuts\n\
                 • Windows Installed Apps / Control Panel Uninstaller Entry\n\n\
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
    }

    match install_default() {
        Ok(report) => {
            #[cfg(not(windows))]
            drop(active_wizard_window);

            println!("Anvil Browser {VERSION} installed successfully!");
            println!("  Location : {}", report.installed_binary.display());
            for sc in &report.shortcut_paths {
                println!("  Shortcut : {}", sc.display());
            }
            if report.registered_os_entry {
                println!("  OS Entry : Registered per-user application entry");
            }

            #[cfg(not(windows))]
            if !silent {
                let should_launch = if used_native_wizard {
                    launch_after_install
                } else {
                    let title = format!("Anvil Browser {VERSION} Setup");
                    let done_msg = format!(
                        "Anvil Browser {VERSION} has been installed successfully!\n\n\
                         Installed executable:\n{}\n\n\
                         Would you like to launch Anvil Browser now?",
                        report.installed_binary.display()
                    );
                    linux_dialog::ask_yes_no(&title, &done_msg).unwrap_or(true)
                };
                if should_launch {
                    let _ = Command::new(&report.installed_binary)
                        .current_dir(&report.install_dir)
                        .spawn();
                }
            }
            #[cfg(windows)]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Setup");
                let done_msg = format!(
                    "Anvil Browser {VERSION} has been installed successfully!\n\n\
                     Installed executable:\n{}\n\n\
                     Desktop and Start Menu shortcuts have been created.\n\n\
                     Would you like to launch Anvil Browser now?",
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
            #[cfg(not(windows))]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Setup Error");
                linux_dialog::show_error(&title, &format!("Installation failed:\n{err}"));
            }
            #[cfg(windows)]
            if !silent {
                let title = format!("Anvil Browser {VERSION} Setup Error");
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
