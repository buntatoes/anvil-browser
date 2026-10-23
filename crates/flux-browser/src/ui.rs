//! Flux window interface: tab strip, navigation bar, security badge, address input,
//! Find-in-Page (`Ctrl+F`) bar, Right-Click Context Menu, and status bar.

use std::path::Path;
use std::sync::OnceLock;

use flux_image::Image;
use flux_layout::Rect;
use flux_raster::{Color, Framebuffer};

use crate::logo::render_flux_logo;
use crate::{Session, Viewport};

pub const TAB_STRIP_HEIGHT: f32 = 36.0;
pub const NAV_BAR_HEIGHT: f32 = 40.0;
pub const BOOKMARKS_BAR_HEIGHT: f32 = 28.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;
pub const SCROLLBAR_WIDTH: f32 = 12.0;
pub const TOP_BAR_HEIGHT: f32 = TAB_STRIP_HEIGHT + NAV_BAR_HEIGHT;

const FIND_BAR_WIDTH: f32 = 300.0;
const FIND_BAR_HEIGHT: f32 = 34.0;
const CTX_MENU_WIDTH: f32 = 196.0;
const CTX_MENU_ROW_H: f32 = 25.0;

pub fn top_header_height(session: &Session) -> f32 {
    if session.bookmarks().is_empty() {
        TOP_BAR_HEIGHT
    } else {
        TOP_BAR_HEIGHT + BOOKMARKS_BAR_HEIGHT
    }
}

const STRIP_BG: Color = Color {
    r: 22,
    g: 20,
    b: 18,
    a: 255,
};
const NAV_BG: Color = Color {
    r: 36,
    g: 32,
    b: 28,
    a: 255,
};
const TAB_IDLE_BG: Color = Color {
    r: 28,
    g: 25,
    b: 22,
    a: 255,
};
const CONTROL_BG: Color = Color {
    r: 48,
    g: 43,
    b: 38,
    a: 255,
};
const CONTROL_HOVER_BG: Color = Color {
    r: 64,
    g: 57,
    b: 49,
    a: 255,
};
const NAV_TOP_BG: Color = Color {
    r: 42,
    g: 37,
    b: 32,
    a: 255,
};
const OVERLAY_SHADOW: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 170,
};
const FIELD_BG: Color = Color {
    r: 20,
    g: 18,
    b: 16,
    a: 255,
};
const DIVIDER: Color = Color {
    r: 68,
    g: 60,
    b: 52,
    a: 255,
};
const ACCENT: Color = Color {
    r: 232,
    g: 195,
    b: 122,
    a: 255,
};
const MATCH_HIGHLIGHT_ACTIVE: Color = Color {
    r: 232,
    g: 195,
    b: 122,
    a: 110,
};
const MATCH_HIGHLIGHT_IDLE: Color = Color {
    r: 136,
    g: 192,
    b: 208,
    a: 75,
};
const INK_PRIMARY: Color = Color {
    r: 242,
    g: 239,
    b: 233,
    a: 255,
};
const INK_MUTED: Color = Color {
    r: 166,
    g: 158,
    b: 146,
    a: 255,
};
const INK_DIM: Color = Color {
    r: 98,
    g: 92,
    b: 84,
    a: 255,
};
const BADGE_TLS_BG: Color = Color {
    r: 24,
    g: 58,
    b: 36,
    a: 255,
};
const BADGE_TLS_FG: Color = Color {
    r: 140,
    g: 224,
    b: 168,
    a: 255,
};
const BADGE_SYS_BG: Color = Color {
    r: 56,
    g: 46,
    b: 24,
    a: 255,
};
const BADGE_HTTP_BG: Color = Color {
    r: 64,
    g: 40,
    b: 20,
    a: 255,
};
const BADGE_HTTP_FG: Color = Color {
    r: 242,
    g: 176,
    b: 94,
    a: 255,
};
const BADGE_FILE_BG: Color = Color {
    r: 34,
    g: 44,
    b: 56,
    a: 255,
};
const BADGE_FILE_FG: Color = Color {
    r: 156,
    g: 198,
    b: 238,
    a: 255,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuAction {
    Back,
    Forward,
    Reload,
    Copy,
    Paste,
    FindInPage,
    ResetZoom,
    SecurityCenter,
}

impl ContextMenuAction {
    pub const ALL: [Self; 8] = [
        Self::Back,
        Self::Forward,
        Self::Reload,
        Self::Copy,
        Self::Paste,
        Self::FindInPage,
        Self::ResetZoom,
        Self::SecurityCenter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Back => "Back              Alt+Left",
            Self::Forward => "Forward          Alt+Right",
            Self::Reload => "Reload Page         Ctrl+R",
            Self::Copy => "Copy                Ctrl+C",
            Self::Paste => "Paste               Ctrl+V",
            Self::FindInPage => "Find in Page...     Ctrl+F",
            Self::ResetZoom => "Reset Zoom (100%)   Ctrl+0",
            Self::SecurityCenter => "Open Security Shield",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    Home,
    SecurityBadge,
    AddressField,
    ZoomPill,
    SettingsButton,
    ToggleBookmark,
    Bookmark(usize),
    FindInput,
    FindPrev,
    FindNext,
    FindClose,
    ContextMenuItem(ContextMenuAction),
    Scrollbar { ratio: f32 },
    Page { x: f32, y: f32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiState {
    editing_address: bool,
    loading: bool,
    input: String,
    hovered_link: Option<String>,
    find_open: bool,
    find_query: String,
    find_active_match: usize,
    context_menu: Option<(f32, f32)>,
    cursor_pos: Option<(f32, f32)>,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_editing_address(&self) -> bool {
        self.editing_address
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn hovered_link(&self) -> Option<&str> {
        self.hovered_link.as_deref()
    }

    pub fn set_hovered_link(&mut self, link: Option<String>) -> bool {
        if self.hovered_link == link {
            return false;
        }
        self.hovered_link = link;
        true
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn focus_address(&mut self, current_url: &str) {
        self.editing_address = true;
        self.find_open = false;
        self.context_menu = None;
        self.input = current_url.to_string();
    }

    pub fn blur_address(&mut self) {
        self.editing_address = false;
    }

    pub fn set_input(&mut self, text: &str) {
        self.input = text.to_string();
    }

    pub fn paste_into_address(&mut self, text: &str) -> bool {
        if !self.editing_address {
            return false;
        }
        let clean: String = text.chars().filter(|c| (' '..='~').contains(c)).collect();
        if clean.is_empty() {
            return false;
        }
        self.input.push_str(&clean);
        true
    }

    pub fn cut_address(&mut self) -> Option<String> {
        if !self.editing_address || self.input.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.input))
    }

    pub fn push_char(&mut self, ch: char) {
        if self.editing_address && (' '..='~').contains(&ch) {
            self.input.push(ch);
        } else if self.find_open && (' '..='~').contains(&ch) {
            self.find_query.push(ch);
            self.find_active_match = 0;
        }
    }

    pub fn backspace(&mut self) {
        if self.editing_address {
            self.input.pop();
        } else if self.find_open {
            self.find_query.pop();
            self.find_active_match = 0;
        }
    }

    pub fn submit_address(&mut self) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        self.editing_address = false;
        normalize_address_input(&self.input)
    }

    pub fn is_find_open(&self) -> bool {
        self.find_open
    }

    pub fn open_find(&mut self) {
        self.find_open = true;
        self.editing_address = false;
        self.context_menu = None;
    }

    pub fn close_find(&mut self) {
        self.find_open = false;
    }

    pub fn find_query(&self) -> &str {
        &self.find_query
    }

    pub fn set_find_query(&mut self, query: &str) {
        self.find_query = query.to_string();
        self.find_active_match = 0;
    }

    pub fn find_active_match(&self) -> usize {
        self.find_active_match
    }

    pub fn find_next(&mut self, total_matches: usize) -> usize {
        if total_matches == 0 {
            self.find_active_match = 0;
        } else {
            self.find_active_match = (self.find_active_match + 1) % total_matches;
        }
        self.find_active_match
    }

    pub fn find_prev(&mut self, total_matches: usize) -> usize {
        if total_matches == 0 {
            self.find_active_match = 0;
        } else if self.find_active_match == 0 {
            self.find_active_match = total_matches - 1;
        } else {
            self.find_active_match -= 1;
        }
        self.find_active_match
    }

    pub fn open_context_menu(&mut self, x: f32, y: f32) {
        self.context_menu = Some((x.max(4.0), y.max(4.0)));
    }

    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
    }

    pub fn context_menu_pos(&self) -> Option<(f32, f32)> {
        self.context_menu
    }

    pub fn cursor_pos(&self) -> Option<(f32, f32)> {
        self.cursor_pos
    }

    pub fn set_cursor_pos(&mut self, x: f32, y: f32) -> bool {
        let next = Some((x, y));
        if self.cursor_pos == next {
            return false;
        }
        self.cursor_pos = next;
        true
    }

    pub fn is_hovering(&self, rect: Rect) -> bool {
        self.cursor_pos
            .is_some_and(|(cx, cy)| rect.contains(cx, cy))
    }

    /// Return the best matching URL suggestion from built-in routes, bookmarks, or history
    /// while the user is typing in the address bar.
    pub fn autocomplete_suggestion(&self, session: &Session) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        let needle = self.input.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        let builtins = [
            "flux://home",
            "flux://settings",
            "flux://downloads",
            "flux://security",
            "flux://history",
        ];
        for route in builtins {
            if route != needle && (route.starts_with(&needle) || route[7..].starts_with(&needle)) {
                return Some(route.to_string());
            }
        }
        for bm in session.bookmarks() {
            let u = bm.url.to_ascii_lowercase();
            if u != needle
                && (u.starts_with(&needle) || bm.title.to_ascii_lowercase().starts_with(&needle))
            {
                return Some(bm.url.clone());
            }
        }
        for entry in session.history_log().iter().rev() {
            let u = entry.url.to_ascii_lowercase();
            if u != needle && u.starts_with(&needle) {
                return Some(entry.url.clone());
            }
        }
        None
    }

    /// Accept the active Omnibox autocomplete suggestion (`Tab` key).
    pub fn accept_autocomplete(&mut self, session: &Session) -> bool {
        if let Some(suggestion) = self.autocomplete_suggestion(session) {
            self.input = suggestion;
            true
        } else {
            false
        }
    }
}

pub fn normalize_address_input(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return None;
    }
    if lower.starts_with("flux://") {
        return Some(lower);
    }
    if lower == "about:home" || lower == "about:blank" {
        return Some("flux://home".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    if Path::new(trimmed).exists()
        || trimmed.ends_with(".html")
        || trimmed.ends_with(".htm")
        || trimmed.contains('\\')
    {
        return Some(trimmed.to_string());
    }
    if trimmed.contains('.') && !trimmed.contains(' ') {
        return Some(format!("https://{trimmed}"));
    }
    Some(trimmed.to_string())
}

pub fn content_viewport(window_width: f32, window_height: f32) -> Viewport {
    Viewport {
        width: window_width.max(1.0),
        height: (window_height - TOP_BAR_HEIGHT - STATUS_BAR_HEIGHT).max(1.0),
    }
}

pub fn hit_test_ui(session: &Session, window_width: f32, x: f32, y: f32) -> Option<UiHit> {
    let default_ui = UiState::new();
    hit_test_ui_with_state(session, &default_ui, window_width, x, y)
}

pub fn hit_test_ui_with_state(
    session: &Session,
    ui: &UiState,
    window_width: f32,
    x: f32,
    y: f32,
) -> Option<UiHit> {
    if x < 0.0 || y < 0.0 || x >= window_width {
        return None;
    }

    // 1. Floating Right-Click Context Menu takes top hit priority when open
    if let Some((mx, my)) = ui.context_menu_pos() {
        let menu_rect = context_menu_rect(mx, my, window_width);
        if menu_rect.contains(x, y) {
            let rel_y = (y - (menu_rect.y + 4.0)).max(0.0);
            let idx = (rel_y / CTX_MENU_ROW_H).floor() as usize;
            if let Some(action) = ContextMenuAction::ALL.get(idx) {
                return Some(UiHit::ContextMenuItem(*action));
            }
        }
    }

    let header_h = top_header_height(session);

    // 2. Floating Find-in-Page (Ctrl+F) bar
    if ui.is_find_open() {
        let fbar = find_bar_rect(window_width, header_h);
        if fbar.contains(x, y) {
            if find_prev_rect(fbar).contains(x, y) {
                return Some(UiHit::FindPrev);
            }
            if find_next_rect(fbar).contains(x, y) {
                return Some(UiHit::FindNext);
            }
            if find_close_rect(fbar).contains(x, y) {
                return Some(UiHit::FindClose);
            }
            return Some(UiHit::FindInput);
        }
    }

    if y < TAB_STRIP_HEIGHT {
        if logo_rect().contains(x, y) {
            return Some(UiHit::Home);
        }
        let count = session.len().max(1);
        let tab_w = tab_width(window_width, count);
        let can_close = count > 1;
        for index in 0..count {
            let rect = tab_rect(index, tab_w);
            if rect.contains(x, y) {
                if can_close && close_rect(rect).contains(x, y) {
                    return Some(UiHit::CloseTab(index));
                }
                return Some(UiHit::Tab(index));
            }
        }
        if new_tab_rect(count, tab_w).contains(x, y) {
            return Some(UiHit::NewTab);
        }
        return None;
    }
    if y < TOP_BAR_HEIGHT {
        if back_rect().contains(x, y) {
            return Some(UiHit::Back);
        }
        if forward_rect().contains(x, y) {
            return Some(UiHit::Forward);
        }
        if reload_rect().contains(x, y) {
            return Some(UiHit::Reload);
        }
        if home_rect().contains(x, y) {
            return Some(UiHit::Home);
        }
        if security_badge_rect().contains(x, y) {
            return Some(UiHit::SecurityBadge);
        }
        if zoom_pill_rect(window_width).contains(x, y) {
            return Some(UiHit::ZoomPill);
        }
        if settings_btn_rect(window_width).contains(x, y) {
            return Some(UiHit::SettingsButton);
        }
        if star_rect(window_width).contains(x, y) {
            return Some(UiHit::ToggleBookmark);
        }
        if address_rect(window_width).contains(x, y) {
            return Some(UiHit::AddressField);
        }
        return None;
    }
    if y < header_h {
        for index in 0..session.bookmarks().len() {
            if bookmark_rect(index).contains(x, y) {
                return Some(UiHit::Bookmark(index));
            }
        }
        return None;
    }
    if session.max_scroll() > 0.0 && x >= (window_width - SCROLLBAR_WIDTH).max(0.0) {
        let page_h = session.viewport().height.max(1.0);
        let ratio = ((y - header_h) / page_h).clamp(0.0, 1.0);
        return Some(UiHit::Scrollbar { ratio });
    }
    Some(UiHit::Page { x, y: y - header_h })
}

fn classify_url_security(url: &str) -> (&'static str, Color, Color, &'static str) {
    let trimmed = url.trim();
    if trimmed.starts_with("https://") {
        (
            "TLS",
            BADGE_TLS_BG,
            BADGE_TLS_FG,
            "Encrypted HTTPS | OS JobSandbox, CSP, & Tracker Shield Active",
        )
    } else if trimmed.starts_with("flux://") {
        (
            "SYS",
            BADGE_SYS_BG,
            ACCENT,
            "Flux Internal Dashboard | Hardcoded Mandatory Sandbox Active",
        )
    } else if trimmed.starts_with("http://") {
        (
            "HTTP",
            BADGE_HTTP_BG,
            BADGE_HTTP_FG,
            "Unencrypted HTTP | OS JobSandbox & Tracker Shield Active",
        )
    } else if trimmed.is_empty() {
        (
            "IDLE",
            CONTROL_BG,
            INK_MUTED,
            "Ready — Hardcoded OS Sandbox & Tracker Shield Active",
        )
    } else {
        (
            "FILE",
            BADGE_FILE_BG,
            BADGE_FILE_FG,
            "Local Document Viewer | Network Isolation Active",
        )
    }
}

pub fn compose_frame(
    session: &Session,
    ui: &UiState,
    window_width: u32,
    window_height: u32,
) -> Framebuffer {
    let width = window_width.max(1);
    let height = window_height.max(1);
    let width_f = width as f32;
    let mut fb = Framebuffer::new(width, height, STRIP_BG);

    // Tab strip + top-left Flux Browser logo emblem
    fb.fill_linear_gradient_rect(
        0.0,
        0.0,
        width_f,
        TAB_STRIP_HEIGHT,
        0.0,
        0.0,
        180.0,
        TAB_IDLE_BG,
        STRIP_BG,
        STRIP_BG,
    );
    let logo = logo_rect();
    fb.draw_image(logo.x, logo.y, logo.width, logo.height, 5.0, top_bar_logo());
    let count = session.len().max(1);
    let tab_w = tab_width(width_f, count);
    let can_close = count > 1;
    for (index, tab) in session.tabs().iter().enumerate() {
        let rect = tab_rect(index, tab_w);
        let is_active = index == session.active_index();
        let is_hovered = ui.is_hovering(rect);
        let bg = if is_active {
            NAV_BG
        } else if is_hovered {
            CONTROL_BG
        } else {
            TAB_IDLE_BG
        };
        let fg = if is_active || is_hovered {
            INK_PRIMARY
        } else {
            INK_MUTED
        };
        let border_col = if is_active {
            ACCENT
        } else if is_hovered {
            INK_MUTED
        } else {
            DIVIDER
        };
        if is_active {
            fb.fill_linear_gradient_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                6.0,
                1.0,
                180.0,
                NAV_TOP_BG,
                NAV_BG,
                border_col,
            );
            fb.fill_rect(
                rect.x + 5.0,
                rect.y,
                (rect.width - 10.0).max(0.0),
                2.0,
                ACCENT,
            );
        } else {
            fb.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                6.0,
                1.0,
                bg,
                border_col,
            );
        }

        let text_space = if can_close {
            (rect.width - 28.0).max(16.0)
        } else {
            (rect.width - 14.0).max(16.0)
        };
        let max_chars = ((text_space / 7.0).floor() as usize).max(1);
        let label = truncate(tab.title(), max_chars);
        fb.draw_text(rect.x + 8.0, rect.y + 6.0, &label, 14.0, fg);

        if can_close {
            let close = close_rect(rect);
            let close_fg = if ui.is_hovering(close) {
                ACCENT
            } else {
                INK_MUTED
            };
            fb.draw_text(close.x + 4.0, close.y + 1.0, "x", 14.0, close_fg);
        }
    }

    let plus = new_tab_rect(count, tab_w);
    let plus_bg = if ui.is_hovering(plus) {
        CONTROL_HOVER_BG
    } else {
        TAB_IDLE_BG
    };
    fb.fill_rounded_rect(
        plus.x,
        plus.y,
        plus.width,
        plus.height,
        6.0,
        1.0,
        plus_bg,
        if ui.is_hovering(plus) {
            ACCENT
        } else {
            DIVIDER
        },
    );
    fb.draw_text(plus.x + 9.0, plus.y + 4.0, "+", 16.0, ACCENT);

    // Navigation bar with subtle vertical studio lighting
    fb.fill_linear_gradient_rect(
        0.0,
        TAB_STRIP_HEIGHT,
        width_f,
        NAV_BAR_HEIGHT,
        0.0,
        0.0,
        180.0,
        NAV_TOP_BG,
        NAV_BG,
        NAV_BG,
    );
    fb.fill_rect(0.0, TOP_BAR_HEIGHT - 1.0, width_f, 1.0, DIVIDER);
    if ui.is_loading() {
        fb.fill_rect(0.0, TOP_BAR_HEIGHT - 2.0, width_f, 2.0, ACCENT);
    }

    let active_tab = session.active();
    draw_control_interactive(&mut fb, ui, back_rect(), "<", active_tab.can_go_back());
    draw_control_interactive(
        &mut fb,
        ui,
        forward_rect(),
        ">",
        active_tab.can_go_forward(),
    );
    draw_control_interactive(
        &mut fb,
        ui,
        reload_rect(),
        "R",
        active_tab.current().is_some(),
    );
    draw_control_interactive(&mut fb, ui, home_rect(), "H", true);

    // Security status badge pill
    let current_page_url = active_tab
        .current()
        .map(|page| page.url.as_str())
        .unwrap_or("");
    let (sec_label, sec_bg, sec_fg, sec_summary) = classify_url_security(current_page_url);
    let badge = security_badge_rect();
    fb.fill_rounded_rect(
        badge.x,
        badge.y,
        badge.width,
        badge.height,
        6.0,
        1.0,
        sec_bg,
        sec_fg,
    );
    fb.draw_text(badge.x + 8.0, badge.y + 6.0, sec_label, 12.0, sec_fg);

    // Zoom Pill quick control
    let zoom_pill = zoom_pill_rect(width_f);
    let zoom_hovered = ui.is_hovering(zoom_pill);
    fb.fill_rounded_rect(
        zoom_pill.x,
        zoom_pill.y,
        zoom_pill.width,
        zoom_pill.height,
        6.0,
        1.0,
        if zoom_hovered {
            CONTROL_HOVER_BG
        } else {
            CONTROL_BG
        },
        if session.zoom_percent() != 100 || zoom_hovered {
            ACCENT
        } else {
            DIVIDER
        },
    );
    let zoom_text = format!("{}%", session.zoom_percent());
    fb.draw_text(
        zoom_pill.x + 6.0,
        zoom_pill.y + 6.0,
        &zoom_text,
        12.0,
        if session.zoom_percent() != 100 {
            ACCENT
        } else {
            INK_PRIMARY
        },
    );

    // Settings quick button ('S')
    draw_control_interactive(&mut fb, ui, settings_btn_rect(width_f), "S", true);

    // Bookmark star control
    let star = star_rect(width_f);
    let star_hovered = ui.is_hovering(star);
    fb.fill_rounded_rect(
        star.x,
        star.y,
        star.width,
        star.height,
        6.0,
        1.0,
        if star_hovered {
            CONTROL_HOVER_BG
        } else {
            CONTROL_BG
        },
        if star_hovered || session.is_active_bookmarked() {
            ACCENT
        } else {
            DIVIDER
        },
    );
    let star_fg = if session.is_active_bookmarked() {
        ACCENT
    } else if active_tab.current().is_some() {
        INK_MUTED
    } else {
        INK_DIM
    };
    fb.draw_text(star.x + 9.0, star.y + 4.0, "*", 16.0, star_fg);

    // Address field (Omnibox with inline autocomplete suggestion)
    let addr = address_rect(width_f);
    let border = if ui.is_editing_address() {
        ACCENT
    } else if ui.is_hovering(addr) {
        INK_MUTED
    } else {
        DIVIDER
    };
    fb.fill_rounded_rect(
        addr.x,
        addr.y,
        addr.width,
        addr.height,
        6.0,
        1.0,
        FIELD_BG,
        border,
    );

    let raw_url = if ui.is_editing_address() {
        if let Some(sugg) = ui.autocomplete_suggestion(session) {
            format!("{}|  -> {} [Tab]", ui.input(), sugg)
        } else {
            format!("{}|", ui.input())
        }
    } else {
        current_page_url.to_string()
    };
    let max_addr_chars = (((addr.width - 16.0).max(8.0) / 7.0).floor() as usize).max(1);
    let shown_url = if ui.is_editing_address() {
        tail(&raw_url, max_addr_chars)
    } else {
        truncate(&raw_url, max_addr_chars)
    };
    let addr_color = if raw_url.is_empty() {
        INK_MUTED
    } else {
        INK_PRIMARY
    };
    let display_text = if shown_url.is_empty() {
        "Search or enter https://, flux://home, flux://settings, flux://security"
    } else {
        shown_url.as_str()
    };
    fb.draw_text(addr.x + 8.0, addr.y + 5.0, display_text, 14.0, addr_color);

    // Optional bookmarks bar
    if !session.bookmarks().is_empty() {
        fb.fill_rect(
            0.0,
            TOP_BAR_HEIGHT,
            width_f,
            BOOKMARKS_BAR_HEIGHT,
            TAB_IDLE_BG,
        );
        fb.fill_rect(
            0.0,
            TOP_BAR_HEIGHT + BOOKMARKS_BAR_HEIGHT - 1.0,
            width_f,
            1.0,
            DIVIDER,
        );
        for (idx, bm) in session.bookmarks().iter().enumerate() {
            let rect = bookmark_rect(idx);
            if rect.x + rect.width > width_f - 8.0 {
                break;
            }
            let hovered = ui.is_hovering(rect);
            fb.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                5.0,
                1.0,
                if hovered {
                    CONTROL_HOVER_BG
                } else {
                    CONTROL_BG
                },
                if hovered { ACCENT } else { DIVIDER },
            );
            let label = truncate(
                if bm.title.is_empty() {
                    &bm.url
                } else {
                    &bm.title
                },
                14,
            );
            fb.draw_text(rect.x + 6.0, rect.y + 3.0, &label, 12.0, INK_PRIMARY);
        }
    }

    // Blit page viewport below top header and above bottom status bar
    let header_h = top_header_height(session);
    let page_fb = session.rasterize();
    let top_y = header_h as u32;
    let status_h_u32 = STATUS_BAR_HEIGHT as u32;
    let avail_h = height.saturating_sub(top_y).saturating_sub(status_h_u32);
    let copy_w = page_fb.width().min(width);
    let copy_h = page_fb.height().min(avail_h);
    for py in 0..copy_h {
        for px in 0..copy_w {
            if let Some(color) = page_fb.pixel(px, py) {
                fb.set_pixel(px as i32, (top_y + py) as i32, color);
            }
        }
    }

    // Find-in-Page (Ctrl+F) highlight overlays & floating search bar
    if ui.is_find_open() {
        let matches = active_tab
            .current()
            .map(|p| p.find_text_matches(ui.find_query()))
            .unwrap_or_default();
        let z = session.zoom_factor();
        let active_idx = ui.find_active_match();
        for (idx, m) in matches.iter().enumerate() {
            let sx = m.x * z;
            let sy = header_h + (m.y * z) - session.scroll();
            let sw = m.width * z;
            let sh = m.height * z;
            if sy + sh > header_h && sy < (height.saturating_sub(status_h_u32) as f32) {
                let (fill, border) = if idx == active_idx {
                    (MATCH_HIGHLIGHT_ACTIVE, ACCENT)
                } else {
                    (MATCH_HIGHLIGHT_IDLE, BADGE_FILE_FG)
                };
                fb.fill_rounded_rect(sx, sy, sw, sh, 3.0, 1.0, fill, border);
            }
        }

        let fbar = find_bar_rect(width_f, header_h);
        fb.fill_box_shadow(
            fbar.x,
            fbar.y,
            fbar.width,
            fbar.height,
            0.0,
            6.0,
            14.0,
            0.0,
            7.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            fbar.x,
            fbar.y,
            fbar.width,
            fbar.height,
            7.0,
            1.0,
            NAV_BG,
            ACCENT,
        );
        let q_shown = truncate(&format!("Find: {}|", ui.find_query()), 19);
        fb.draw_text(fbar.x + 8.0, fbar.y + 8.0, &q_shown, 13.0, INK_PRIMARY);

        let count_label = if matches.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", (active_idx + 1).min(matches.len()), matches.len())
        };
        fb.draw_text(fbar.x + 158.0, fbar.y + 9.0, &count_label, 12.0, ACCENT);

        let prev_btn = find_prev_rect(fbar);
        let next_btn = find_next_rect(fbar);
        let close_btn = find_close_rect(fbar);
        for (btn, glyph) in [(prev_btn, "<"), (next_btn, ">"), (close_btn, "x")] {
            let hovered = ui.is_hovering(btn);
            fb.fill_rounded_rect(
                btn.x,
                btn.y,
                btn.width,
                btn.height,
                4.0,
                1.0,
                if hovered {
                    CONTROL_HOVER_BG
                } else {
                    CONTROL_BG
                },
                if hovered { ACCENT } else { DIVIDER },
            );
            fb.draw_text(btn.x + 6.0, btn.y + 3.0, glyph, 13.0, INK_PRIMARY);
        }
    }

    // Vertical scrollbar
    let max_scroll = session.max_scroll();
    let track_h = avail_h as f32;
    if max_scroll > 0.0 && track_h > 16.0 {
        let track_x = (width_f - SCROLLBAR_WIDTH).max(0.0);
        fb.fill_rect(track_x, header_h, SCROLLBAR_WIDTH, track_h, TAB_IDLE_BG);
        let view_h = session.viewport().height.max(1.0);
        let content_h = (view_h + max_scroll).max(view_h);
        let thumb_h = ((view_h / content_h) * track_h).clamp(24.0, track_h);
        let scroll_ratio = (session.scroll() / max_scroll).clamp(0.0, 1.0);
        let thumb_y = header_h + scroll_ratio * (track_h - thumb_h);
        fb.fill_rounded_rect(
            track_x + 2.0,
            thumb_y,
            (SCROLLBAR_WIDTH - 4.0).max(2.0),
            thumb_h,
            4.0,
            0.0,
            INK_MUTED,
            INK_MUTED,
        );
    }

    // Floating Right-Click Context Menu
    if let Some((mx, my)) = ui.context_menu_pos() {
        let menu_rect = context_menu_rect(mx, my, width_f);
        fb.fill_box_shadow(
            menu_rect.x,
            menu_rect.y,
            menu_rect.width,
            menu_rect.height,
            0.0,
            8.0,
            18.0,
            0.0,
            7.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            menu_rect.x,
            menu_rect.y,
            menu_rect.width,
            menu_rect.height,
            7.0,
            1.0,
            NAV_BG,
            ACCENT,
        );
        for (idx, item) in ContextMenuAction::ALL.iter().enumerate() {
            let ry = menu_rect.y + 4.0 + (idx as f32) * CTX_MENU_ROW_H;
            let row_rect = Rect {
                x: menu_rect.x + 4.0,
                y: ry,
                width: menu_rect.width - 8.0,
                height: CTX_MENU_ROW_H,
            };
            if idx > 0 {
                fb.fill_rect(
                    menu_rect.x + 6.0,
                    ry - 1.0,
                    menu_rect.width - 12.0,
                    1.0,
                    DIVIDER,
                );
            }
            let row_hovered = ui.is_hovering(row_rect);
            if row_hovered {
                fb.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.width,
                    row_rect.height - 2.0,
                    4.0,
                    0.0,
                    CONTROL_HOVER_BG,
                    CONTROL_HOVER_BG,
                );
            }
            fb.draw_text(
                menu_rect.x + 10.0,
                ry + 4.0,
                item.label(),
                12.0,
                if row_hovered { ACCENT } else { INK_PRIMARY },
            );
        }
    }

    // Bottom status bar
    let status_y = (height.saturating_sub(status_h_u32)) as f32;
    fb.fill_linear_gradient_rect(
        0.0,
        status_y,
        width_f,
        STATUS_BAR_HEIGHT,
        0.0,
        0.0,
        180.0,
        TAB_IDLE_BG,
        STRIP_BG,
        STRIP_BG,
    );
    fb.fill_rect(0.0, status_y, width_f, 1.0, DIVIDER);

    let status_left = if ui.is_loading() {
        "Loading resource over sandboxed transport...".to_string()
    } else if let Some(link) = ui.hovered_link() {
        format!("Link -> {link}")
    } else {
        sec_summary.to_string()
    };
    let max_status_chars = (((width_f - 230.0).max(24.0) / 6.5).floor() as usize).max(1);
    let left_trimmed = truncate(&status_left, max_status_chars);
    fb.draw_text(8.0, status_y + 4.0, &left_trimmed, 12.0, INK_MUTED);

    let scroll_pct = if max_scroll > 0.0 {
        ((session.scroll() / max_scroll) * 100.0).round() as u32
    } else {
        0
    };
    let page_blocked = active_tab
        .current()
        .map(|p| p.blocked_csp_count + p.blocked_tracker_count)
        .unwrap_or(0);
    let total_blocked = session.network_shield().blocked_total() as usize + page_blocked;
    let zoom_pct = session.zoom_percent();
    let right_label = format!("Shields: ON ({total_blocked}) | {zoom_pct}% | {scroll_pct}%");
    let right_x = (width_f - 210.0).max(8.0);
    fb.draw_text(right_x, status_y + 4.0, &right_label, 12.0, ACCENT);

    fb
}

fn draw_control_interactive(
    fb: &mut Framebuffer,
    ui: &UiState,
    rect: Rect,
    glyph: &str,
    enabled: bool,
) {
    let hovered = enabled && ui.is_hovering(rect);
    fb.fill_rounded_rect(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        6.0,
        1.0,
        if hovered {
            CONTROL_HOVER_BG
        } else {
            CONTROL_BG
        },
        if hovered { ACCENT } else { DIVIDER },
    );
    let fg = if hovered {
        ACCENT
    } else if enabled {
        INK_PRIMARY
    } else {
        INK_DIM
    };
    fb.draw_text(rect.x + 9.0, rect.y + 4.0, glyph, 16.0, fg);
}

fn top_bar_logo() -> &'static Image {
    static LOGO: OnceLock<Image> = OnceLock::new();
    LOGO.get_or_init(|| render_flux_logo(24))
}

fn logo_rect() -> Rect {
    Rect {
        x: 8.0,
        y: 6.0,
        width: 24.0,
        height: 24.0,
    }
}

fn find_bar_rect(window_width: f32, header_h: f32) -> Rect {
    Rect {
        x: (window_width - FIND_BAR_WIDTH - 20.0).max(8.0),
        y: header_h + 8.0,
        width: FIND_BAR_WIDTH,
        height: FIND_BAR_HEIGHT,
    }
}

fn find_prev_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 84.0,
        y: fbar.y + 5.0,
        width: 22.0,
        height: 24.0,
    }
}

fn find_next_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 58.0,
        y: fbar.y + 5.0,
        width: 22.0,
        height: 24.0,
    }
}

fn find_close_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 32.0,
        y: fbar.y + 5.0,
        width: 22.0,
        height: 24.0,
    }
}

fn context_menu_rect(x: f32, y: f32, window_width: f32) -> Rect {
    let total_h = 8.0 + (ContextMenuAction::ALL.len() as f32) * CTX_MENU_ROW_H;
    let clamped_x = x.clamp(4.0, (window_width - CTX_MENU_WIDTH - 4.0).max(4.0));
    Rect {
        x: clamped_x,
        y: y.max(4.0),
        width: CTX_MENU_WIDTH,
        height: total_h,
    }
}

fn tab_width(window_width: f32, count: usize) -> f32 {
    let available = (window_width - 82.0).max(72.0);
    (available / (count as f32)).clamp(72.0, 180.0)
}

fn tab_rect(index: usize, tab_w: f32) -> Rect {
    Rect {
        x: 38.0 + (index as f32) * (tab_w + 4.0),
        y: 6.0,
        width: tab_w,
        height: TAB_STRIP_HEIGHT - 6.0,
    }
}

fn close_rect(tab: Rect) -> Rect {
    Rect {
        x: tab.x + tab.width - 20.0,
        y: tab.y + 6.0,
        width: 16.0,
        height: 16.0,
    }
}

fn new_tab_rect(count: usize, tab_w: f32) -> Rect {
    Rect {
        x: 38.0 + (count as f32) * (tab_w + 4.0),
        y: 6.0,
        width: 28.0,
        height: 26.0,
    }
}

fn back_rect() -> Rect {
    Rect {
        x: 8.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn forward_rect() -> Rect {
    Rect {
        x: 40.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn reload_rect() -> Rect {
    Rect {
        x: 72.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn home_rect() -> Rect {
    Rect {
        x: 104.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn security_badge_rect() -> Rect {
    Rect {
        x: 136.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 44.0,
        height: 28.0,
    }
}

fn address_rect(window_width: f32) -> Rect {
    Rect {
        x: 184.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: (window_width - 310.0).max(60.0),
        height: 28.0,
    }
}

fn zoom_pill_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 120.0).max(248.0),
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 48.0,
        height: 28.0,
    }
}

fn settings_btn_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 68.0).max(300.0),
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn star_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 36.0).max(332.0),
        y: TAB_STRIP_HEIGHT + 6.0,
        width: 28.0,
        height: 28.0,
    }
}

fn bookmark_rect(index: usize) -> Rect {
    let pill_w = 112.0;
    Rect {
        x: 8.0 + (index as f32) * (pill_w + 6.0),
        y: TOP_BAR_HEIGHT + 4.0,
        width: pill_w,
        height: 20.0,
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return chars.into_iter().take(max_chars).collect();
    }
    let mut out: String = chars.into_iter().take(max_chars - 2).collect();
    out.push_str("..");
    out
}

fn tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_tests_tabs_controls_find_bar_context_menu_and_page_area() {
        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.open_html("<title>First</title><p>One</p>", "first.html");
        session.new_tab();
        session.open_html("<title>Second</title><p>Two</p>", "second.html");

        assert_eq!(hit_test_ui(&session, 800.0, 16.0, 16.0), Some(UiHit::Home));
        assert_eq!(
            hit_test_ui(&session, 800.0, 50.0, 16.0),
            Some(UiHit::Tab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 210.0, 16.0),
            Some(UiHit::CloseTab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 414.0, 16.0),
            Some(UiHit::NewTab)
        );
        assert_eq!(hit_test_ui(&session, 800.0, 16.0, 50.0), Some(UiHit::Back));
        assert_eq!(
            hit_test_ui(&session, 800.0, 50.0, 50.0),
            Some(UiHit::Forward)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 80.0, 50.0),
            Some(UiHit::Reload)
        );
        assert_eq!(hit_test_ui(&session, 800.0, 112.0, 50.0), Some(UiHit::Home));
        assert_eq!(
            hit_test_ui(&session, 800.0, 150.0, 50.0),
            Some(UiHit::SecurityBadge)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 220.0, 50.0),
            Some(UiHit::AddressField)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 770.0, 50.0),
            Some(UiHit::ToggleBookmark)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 40.0, 100.0),
            Some(UiHit::Page {
                x: 40.0,
                y: 100.0 - TOP_BAR_HEIGHT
            })
        );

        let mut ui = UiState::new();
        ui.open_find();
        ui.set_find_query("Two");
        assert_eq!(ui.find_next(3), 1);
        assert_eq!(ui.find_prev(3), 0);
        let fbar = find_bar_rect(800.0, TOP_BAR_HEIGHT);
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, fbar.x + 20.0, fbar.y + 10.0),
            Some(UiHit::FindInput)
        );

        ui.open_context_menu(120.0, 140.0);
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 140.0, 150.0),
            Some(UiHit::ContextMenuItem(ContextMenuAction::Back))
        );

        session.add_bookmark("Home", "examples/hello.html");
        ui.close_context_menu();
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 20.0, TOP_BAR_HEIGHT + 10.0),
            Some(UiHit::Bookmark(0))
        );
    }

    #[test]
    fn edits_and_submits_address_field_input() {
        let mut ui = UiState::new();
        ui.focus_address("examples/hello.html");
        assert!(ui.is_editing_address());
        ui.set_input("example.com");
        ui.push_char('/');
        ui.backspace();
        assert_eq!(ui.submit_address(), Some("https://example.com".to_string()));
        assert!(!ui.is_editing_address());
        assert_eq!(
            normalize_address_input("flux://security"),
            Some("flux://security".to_string())
        );
        assert_eq!(normalize_address_input("javascript:alert(1)"), None);
    }
}
