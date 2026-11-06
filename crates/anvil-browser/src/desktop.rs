// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! What the desktop has actually said about light/dark and window buttons.
//!
//! A missing file, a failed read, or a value this module does not understand
//! stays "not reported". Nothing here guesses a compositor feature.

#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::Command;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// One caption button the desktop named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

/// Which side of the title strip a button sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionSide {
    Left,
    Right,
}

/// Window buttons, in the order the desktop listed them.
///
/// `reported` is false when no desktop file or setting named the buttons.
/// The window then keeps the usual right-side minimize, maximize, and close.
/// `server_side` is true only after Wayland's decoration object says the
/// compositor is drawing the frame (`mode` 2). Those buttons are hidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionLayout {
    pub reported: bool,
    pub server_side: bool,
    pub buttons: Vec<(CaptionButton, CaptionSide)>,
}

impl CaptionLayout {
    /// Right-side minimize, maximize, close. Used when the desktop has not said.
    pub fn classic() -> Self {
        Self {
            reported: false,
            server_side: false,
            buttons: vec![
                (CaptionButton::Minimize, CaptionSide::Right),
                (CaptionButton::Maximize, CaptionSide::Right),
                (CaptionButton::Close, CaptionSide::Right),
            ],
        }
    }
}

#[cfg(not(test))]
static LAYOUT: OnceLock<CaptionLayout> = OnceLock::new();
static SERVER_SIDE: AtomicBool = AtomicBool::new(false);

/// Layout used for hit testing and drawing.
///
/// Tests always get the classic layout unless a test sets one. The window
/// process reads the desktop once.
pub fn caption_layout() -> CaptionLayout {
    caption_layout_now()
}

#[cfg(test)]
fn caption_layout_now() -> CaptionLayout {
    TEST_LAYOUT.with(|slot| slot.borrow().clone().unwrap_or_else(CaptionLayout::classic))
}

#[cfg(not(test))]
fn caption_layout_now() -> CaptionLayout {
    let mut layout = LAYOUT.get_or_init(read_caption_layout).clone();
    if SERVER_SIDE.load(Ordering::Relaxed) {
        layout.server_side = true;
        layout.buttons.clear();
    }
    layout
}

/// Remember a Wayland decoration configure. `true` means mode 2, server side.
pub fn set_server_side_decorations(server_side: bool) {
    SERVER_SIDE.store(server_side, Ordering::Relaxed);
}

/// Last value stored by [`set_server_side_decorations`].
pub fn server_side_decorations() -> bool {
    SERVER_SIDE.load(Ordering::Relaxed)
}

#[cfg(test)]
thread_local! {
    static TEST_LAYOUT: std::cell::RefCell<Option<CaptionLayout>> = const { std::cell::RefCell::new(None) };
    static TEST_LIGHT: std::cell::RefCell<Option<Option<bool>>> = const { std::cell::RefCell::new(None) };
}

/// Tests pick a layout. `None` restores the classic buttons.
#[cfg(test)]
pub fn set_caption_layout_for_test(layout: Option<CaptionLayout>) {
    TEST_LAYOUT.with(|slot| *slot.borrow_mut() = layout);
}

/// `Some(true)` means the desktop asked for a light theme.
/// `Some(false)` means it asked for a dark theme.
/// `None` means it did not say.
///
/// Tests get `None` unless they set a value, so a developer machine cannot
/// change the result.
pub fn reported_light() -> Option<bool> {
    reported_light_now()
}

#[cfg(test)]
fn reported_light_now() -> Option<bool> {
    TEST_LIGHT.with(|slot| slot.borrow().unwrap_or(None))
}

#[cfg(not(test))]
fn reported_light_now() -> Option<bool> {
    *LIGHT.get_or_init(read_reported_light)
}

#[cfg(not(test))]
static LIGHT: OnceLock<Option<bool>> = OnceLock::new();

/// Tests pick a reported scheme. `None` means "desktop did not say".
#[cfg(test)]
pub fn set_reported_light_for_test(value: Option<bool>) {
    TEST_LIGHT.with(|slot| *slot.borrow_mut() = Some(value));
}

#[cfg(not(test))]
fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config");
    }
    PathBuf::from(".config")
}

#[cfg(not(test))]
fn desktop_names() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Read buttons from a GNOME `button-layout` value such as `close,minimize:`.
///
/// Tokens this browser does not draw (`appmenu`, `icon`, `menu`) are skipped.
/// Returns `None` when the string names no minimize, maximize, or close button,
/// so an empty or unknown value is not treated as "hide every button".
pub fn parse_gnome_button_layout(spec: &str) -> Option<CaptionLayout> {
    let spec = spec.trim().trim_matches('\'').trim_matches('"');
    let (left, right) = spec.split_once(':').unwrap_or((spec, ""));
    let mut buttons = Vec::new();
    push_gnome_side(&mut buttons, left, CaptionSide::Left);
    push_gnome_side(&mut buttons, right, CaptionSide::Right);
    if buttons.is_empty() {
        return None;
    }
    Some(CaptionLayout {
        reported: true,
        server_side: false,
        buttons,
    })
}

fn push_gnome_side(out: &mut Vec<(CaptionButton, CaptionSide)>, text: &str, side: CaptionSide) {
    for token in text.split(',') {
        let button = match token.trim().to_ascii_lowercase().as_str() {
            "minimize" => CaptionButton::Minimize,
            "maximize" => CaptionButton::Maximize,
            "close" => CaptionButton::Close,
            _ => continue,
        };
        out.push((button, side));
    }
}

/// Read KWin `ButtonsOnLeft` / `ButtonsOnRight`.
///
/// `I` is minimize, `A` is maximize, `X` is close. Other letters are skipped.
/// The section existing is a report even when both sides are empty: that
/// means the desktop asked for no buttons.
pub fn parse_kwin_buttons(left: &str, right: &str) -> CaptionLayout {
    let mut buttons = Vec::new();
    push_kwin_side(&mut buttons, left, CaptionSide::Left);
    push_kwin_side(&mut buttons, right, CaptionSide::Right);
    CaptionLayout {
        reported: true,
        server_side: false,
        buttons,
    }
}

fn push_kwin_side(out: &mut Vec<(CaptionButton, CaptionSide)>, text: &str, side: CaptionSide) {
    for ch in text.chars() {
        let button = match ch.to_ascii_uppercase() {
            'I' => CaptionButton::Minimize,
            'A' => CaptionButton::Maximize,
            'X' => CaptionButton::Close,
            _ => continue,
        };
        out.push((button, side));
    }
}

/// `true` when the text asks for a light theme, `false` for dark.
///
/// GNOME `color-scheme=default` is not a light or dark answer.
pub fn parse_color_scheme_value(text: &str) -> Option<bool> {
    let value = text
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_ascii_lowercase();
    if value == "prefer-dark" || value == "dark" {
        return Some(false);
    }
    if value == "prefer-light" || value == "light" {
        return Some(true);
    }
    None
}

/// Read `gtk-application-prefer-dark-theme` and `gtk-theme-name` from a settings.ini.
pub fn parse_gtk_settings(text: &str) -> Option<bool> {
    let mut prefer_dark: Option<bool> = None;
    let mut theme_dark: Option<bool> = None;
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        if key.eq_ignore_ascii_case("gtk-application-prefer-dark-theme") {
            prefer_dark = match value {
                "1" | "true" | "True" => Some(false),
                "0" | "false" | "False" => Some(true),
                _ => None,
            };
        } else if key.eq_ignore_ascii_case("gtk-theme-name")
            && value.to_ascii_lowercase().contains("dark")
        {
            theme_dark = Some(false);
        }
    }
    prefer_dark.or(theme_dark)
}

/// Read KDE `ColorScheme`. A name containing "dark" is dark. Any other
/// non-empty name is light. A missing key is not a report.
pub fn parse_kde_color_scheme(text: &str) -> Option<bool> {
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("ColorScheme") {
            let value = value.trim().to_ascii_lowercase();
            if value.contains("dark") {
                return Some(false);
            }
            if value.contains("light") {
                return Some(true);
            }
            return None;
        }
    }
    None
}

#[cfg(not(test))]
fn read_caption_layout() -> CaptionLayout {
    let names = desktop_names();
    let kde = names.contains("kde") || names.contains("kwin");
    let gnome = names.contains("gnome") || names.contains("unity");
    if kde && let Some(layout) = read_kwin_layout() {
        return layout;
    }
    if gnome && let Some(layout) = read_gnome_layout() {
        return layout;
    }
    if !kde && let Some(layout) = read_kwin_layout() {
        return layout;
    }
    if !gnome && let Some(layout) = read_gnome_layout() {
        return layout;
    }
    CaptionLayout::classic()
}

#[cfg(not(test))]
fn read_kwin_layout() -> Option<CaptionLayout> {
    let text = fs::read_to_string(config_dir().join("kwinrc")).ok()?;
    let mut in_section = false;
    let mut saw = false;
    let mut left = String::new();
    let mut right = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed.eq_ignore_ascii_case("[org.kde.kdecoration2]");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("ButtonsOnLeft") {
            left = value.trim().to_string();
            saw = true;
        } else if key.trim().eq_ignore_ascii_case("ButtonsOnRight") {
            right = value.trim().to_string();
            saw = true;
        }
    }
    if !saw {
        return None;
    }
    Some(parse_kwin_buttons(&left, &right))
}

#[cfg(not(test))]
fn read_gnome_layout() -> Option<CaptionLayout> {
    let text = gsettings_get("org.gnome.desktop.wm.preferences", "button-layout")?;
    parse_gnome_button_layout(&text)
}

#[cfg(not(test))]
fn read_reported_light() -> Option<bool> {
    if let Some(text) = gsettings_get("org.gnome.desktop.interface", "color-scheme")
        && let Some(light) = parse_color_scheme_value(&text)
    {
        return Some(light);
    }
    let dir = config_dir();
    for name in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Some(light) = read_file_parse(dir.join(name), parse_gtk_settings) {
            return Some(light);
        }
    }
    if let Some(light) = read_file_parse(dir.join("kdeglobals"), parse_kde_color_scheme) {
        return Some(light);
    }
    #[cfg(windows)]
    {
        if let Some(light) = windows_apps_use_light() {
            return Some(light);
        }
    }
    None
}

#[cfg(not(test))]
fn read_file_parse(path: PathBuf, parse: fn(&str) -> Option<bool>) -> Option<bool> {
    let text = fs::read_to_string(path).ok()?;
    parse(&text)
}

/// Run `gsettings get` when `/usr/bin/gsettings` exists.
///
/// A missing tool, a non-zero status, an empty answer, or no exit within
/// 400 milliseconds is not a report. The process is stopped if it waits.
#[cfg(not(test))]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let exe = Path::new("/usr/bin/gsettings");
    if !exe.exists() {
        return None;
    }
    let mut child = Command::new(exe)
        .args(["get", schema, key])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < std::time::Duration::from_millis(400) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut text = String::new();
    let mut stdout = child.stdout.take()?;
    std::io::Read::read_to_string(&mut stdout, &mut text).ok()?;
    let text = text.trim().trim_matches('\'').trim_matches('"').trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Sign-extended `HKEY_CURRENT_USER`. Win32 defines it as `(LONG)0x80000001`.
pub fn hkey_current_user() -> isize {
    0x8000_0001u32 as i32 as isize
}

/// `AppsUseLightTheme` in the current user hive. 1 is light, 0 is dark.
/// Any other result is not a report.
#[cfg(windows)]
fn windows_apps_use_light() -> Option<bool> {
    // SAFETY: `hkey` is the predefined HKEY_CURRENT_USER value, sign-extended
    // the way Win32 defines that handle. The subkey and value are fixed
    // NUL-terminated UTF-16 literals. `data` points at a local `u32` and
    // `size` is its byte length, so RegGetValueW writes at most four bytes
    // into memory this function owns. `kind` is null because the type is not
    // needed. A non-zero status, or a size other than four, is not a report.
    unsafe {
        let subkey: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();
        let value: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut data: u32 = 0;
        let mut size: u32 = 4;
        // RRF_RT_DWORD: accept a REG_DWORD and reject other types.
        let status = RegGetValueW(
            hkey_current_user() as *mut core::ffi::c_void,
            subkey.as_ptr(),
            value.as_ptr(),
            0x0000_0018,
            core::ptr::null_mut(),
            &mut data as *mut u32 as *mut core::ffi::c_void,
            &mut size,
        );
        if status != 0 || size != 4 {
            return None;
        }
        match data {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }
    }
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegGetValueW(
        hkey: *mut core::ffi::c_void,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut core::ffi::c_void,
        size: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_layout_keeps_side_and_skips_unknown_tokens() {
        let layout = parse_gnome_button_layout("close,appmenu:minimize,maximize").unwrap();
        assert!(layout.reported);
        assert_eq!(
            layout.buttons,
            vec![
                (CaptionButton::Close, CaptionSide::Left),
                (CaptionButton::Minimize, CaptionSide::Right),
                (CaptionButton::Maximize, CaptionSide::Right),
            ]
        );
        assert!(parse_gnome_button_layout("appmenu:").is_none());
        assert!(parse_gnome_button_layout("").is_none());
    }

    #[test]
    fn kwin_letters_map_only_the_three_buttons() {
        let layout = parse_kwin_buttons("X", "IA");
        assert!(layout.reported);
        assert_eq!(
            layout.buttons,
            vec![
                (CaptionButton::Close, CaptionSide::Left),
                (CaptionButton::Minimize, CaptionSide::Right),
                (CaptionButton::Maximize, CaptionSide::Right),
            ]
        );
        let none = parse_kwin_buttons("M", "");
        assert!(none.reported);
        assert!(none.buttons.is_empty());
    }

    #[test]
    fn color_scheme_text_does_not_treat_default_as_light_or_dark() {
        assert_eq!(parse_color_scheme_value("prefer-dark"), Some(false));
        assert_eq!(parse_color_scheme_value("'prefer-light'"), Some(true));
        assert_eq!(parse_color_scheme_value("default"), None);
        assert_eq!(
            parse_gtk_settings("gtk-application-prefer-dark-theme=1\n"),
            Some(false)
        );
        assert_eq!(
            parse_gtk_settings("gtk-application-prefer-dark-theme=0\n"),
            Some(true)
        );
        assert_eq!(parse_gtk_settings("gtk-theme-name=Breeze\n"), None);
        assert_eq!(
            parse_gtk_settings("gtk-theme-name=Adwaita-dark\n"),
            Some(false)
        );
        assert_eq!(
            parse_kde_color_scheme("ColorScheme=BreezeDark\n"),
            Some(false)
        );
        assert_eq!(
            parse_kde_color_scheme("ColorScheme=BreezeLight\n"),
            Some(true)
        );
        assert_eq!(parse_kde_color_scheme("ColorScheme=Breeze\n"), None);
        assert_eq!(
            hkey_current_user() as usize,
            0x8000_0001u32 as i32 as isize as usize
        );
        assert_eq!(parse_kde_color_scheme("[General]\n"), None);
    }
}
