//! Flux window interface: tab strip, navigation bar, security badge, address input, and status bar.

use std::path::Path;

use flux_layout::Rect;
use flux_raster::{Color, Framebuffer};

use crate::{Session, Viewport};

pub const TAB_STRIP_HEIGHT: f32 = 36.0;
pub const NAV_BAR_HEIGHT: f32 = 40.0;
pub const BOOKMARKS_BAR_HEIGHT: f32 = 28.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;
pub const SCROLLBAR_WIDTH: f32 = 12.0;
pub const TOP_BAR_HEIGHT: f32 = TAB_STRIP_HEIGHT + NAV_BAR_HEIGHT;

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
    ToggleBookmark,
    Bookmark(usize),
    Scrollbar { ratio: f32 },
    Page { x: f32, y: f32 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiState {
    editing_address: bool,
    loading: bool,
    input: String,
    hovered_link: Option<String>,
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
        self.input = current_url.to_string();
    }

    pub fn blur_address(&mut self) {
        self.editing_address = false;
    }

    pub fn set_input(&mut self, text: &str) {
        self.input = text.to_string();
    }

    pub fn push_char(&mut self, ch: char) {
        if self.editing_address && (' '..='~').contains(&ch) {
            self.input.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.editing_address {
            self.input.pop();
        }
    }

    pub fn submit_address(&mut self) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        self.editing_address = false;
        normalize_address_input(&self.input)
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
    if x < 0.0 || y < 0.0 || x >= window_width {
        return None;
    }
    if y < TAB_STRIP_HEIGHT {
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
        if star_rect(window_width).contains(x, y) {
            return Some(UiHit::ToggleBookmark);
        }
        if address_rect(window_width).contains(x, y) {
            return Some(UiHit::AddressField);
        }
        return None;
    }
    let header_h = top_header_height(session);
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
            "Encrypted HTTPS (Strong Crypto) | SOP & Sandbox Active",
        )
    } else if trimmed.starts_with("flux://") {
        (
            "SYS",
            BADGE_SYS_BG,
            ACCENT,
            "Flux Internal Dashboard | Hardened Sandbox Active",
        )
    } else if trimmed.starts_with("http://") {
        (
            "HTTP",
            BADGE_HTTP_BG,
            BADGE_HTTP_FG,
            "Unencrypted HTTP | Remote-to-Local Isolation Active",
        )
    } else if trimmed.is_empty() {
        (
            "IDLE",
            CONTROL_BG,
            INK_MUTED,
            "Ready — Sandbox & Same-Origin Policy Active",
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

    // Tab strip
    fb.fill_rect(0.0, 0.0, width_f, TAB_STRIP_HEIGHT, STRIP_BG);
    let count = session.len().max(1);
    let tab_w = tab_width(width_f, count);
    let can_close = count > 1;
    for (index, tab) in session.tabs().iter().enumerate() {
        let rect = tab_rect(index, tab_w);
        let is_active = index == session.active_index();
        let bg = if is_active { NAV_BG } else { TAB_IDLE_BG };
        let fg = if is_active { INK_PRIMARY } else { INK_MUTED };
        let border_col = if is_active { ACCENT } else { DIVIDER };
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
        if is_active {
            fb.fill_rect(
                rect.x + 4.0,
                rect.y,
                (rect.width - 8.0).max(0.0),
                2.0,
                ACCENT,
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
            fb.draw_text(close.x + 4.0, close.y + 1.0, "x", 14.0, INK_MUTED);
        }
    }

    let plus = new_tab_rect(count, tab_w);
    fb.fill_rounded_rect(
        plus.x,
        plus.y,
        plus.width,
        plus.height,
        6.0,
        1.0,
        TAB_IDLE_BG,
        DIVIDER,
    );
    fb.draw_text(plus.x + 9.0, plus.y + 4.0, "+", 16.0, ACCENT);

    // Navigation bar
    fb.fill_rect(0.0, TAB_STRIP_HEIGHT, width_f, NAV_BAR_HEIGHT, NAV_BG);
    fb.fill_rect(0.0, TOP_BAR_HEIGHT - 1.0, width_f, 1.0, DIVIDER);
    if ui.is_loading() {
        fb.fill_rect(0.0, TOP_BAR_HEIGHT - 2.0, width_f, 2.0, ACCENT);
    }

    let active_tab = session.active();
    draw_control(&mut fb, back_rect(), "<", active_tab.can_go_back());
    draw_control(&mut fb, forward_rect(), ">", active_tab.can_go_forward());
    draw_control(&mut fb, reload_rect(), "R", active_tab.current().is_some());
    draw_control(&mut fb, home_rect(), "H", true);

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

    // Bookmark star control
    let star = star_rect(width_f);
    fb.fill_rounded_rect(
        star.x,
        star.y,
        star.width,
        star.height,
        6.0,
        1.0,
        CONTROL_BG,
        DIVIDER,
    );
    let star_fg = if session.is_active_bookmarked() {
        ACCENT
    } else if active_tab.current().is_some() {
        INK_MUTED
    } else {
        INK_DIM
    };
    fb.draw_text(star.x + 9.0, star.y + 4.0, "*", 16.0, star_fg);

    // Address field
    let addr = address_rect(width_f);
    let border = if ui.is_editing_address() {
        ACCENT
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
        format!("{}|", ui.input())
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
        "Enter an https://, flux://, or file path"
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
            fb.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                5.0,
                1.0,
                CONTROL_BG,
                DIVIDER,
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

    // Bottom status bar
    let status_y = (height.saturating_sub(status_h_u32)) as f32;
    fb.fill_rect(0.0, status_y, width_f, STATUS_BAR_HEIGHT, STRIP_BG);
    fb.fill_rect(0.0, status_y, width_f, 1.0, DIVIDER);

    let status_left = if ui.is_loading() {
        "Loading resource over sandboxed transport...".to_string()
    } else if let Some(link) = ui.hovered_link() {
        format!("Link -> {link}")
    } else {
        sec_summary.to_string()
    };
    let max_status_chars = (((width_f - 180.0).max(24.0) / 6.5).floor() as usize).max(1);
    let left_trimmed = truncate(&status_left, max_status_chars);
    fb.draw_text(8.0, status_y + 4.0, &left_trimmed, 12.0, INK_MUTED);

    let scroll_pct = if max_scroll > 0.0 {
        ((session.scroll() / max_scroll) * 100.0).round() as u32
    } else {
        0
    };
    let right_label = format!("Shields: ON | {scroll_pct}%");
    let right_x = (width_f - 140.0).max(8.0);
    fb.draw_text(right_x, status_y + 4.0, &right_label, 12.0, ACCENT);

    fb
}

fn draw_control(fb: &mut Framebuffer, rect: Rect, glyph: &str, enabled: bool) {
    fb.fill_rounded_rect(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        6.0,
        1.0,
        CONTROL_BG,
        DIVIDER,
    );
    let fg = if enabled { INK_PRIMARY } else { INK_DIM };
    fb.draw_text(rect.x + 9.0, rect.y + 4.0, glyph, 16.0, fg);
}

fn tab_width(window_width: f32, count: usize) -> f32 {
    let available = (window_width - 52.0).max(72.0);
    (available / (count as f32)).clamp(72.0, 180.0)
}

fn tab_rect(index: usize, tab_w: f32) -> Rect {
    Rect {
        x: 8.0 + (index as f32) * (tab_w + 4.0),
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
        x: 8.0 + (count as f32) * (tab_w + 4.0),
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
        width: (window_width - 224.0).max(60.0),
        height: 28.0,
    }
}

fn star_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 36.0).max(248.0),
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
    fn hit_tests_tabs_controls_and_page_area() {
        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.open_html("<title>First</title><p>One</p>", "first.html");
        session.new_tab();
        session.open_html("<title>Second</title><p>Two</p>", "second.html");

        assert_eq!(
            hit_test_ui(&session, 800.0, 20.0, 16.0),
            Some(UiHit::Tab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 180.0, 16.0),
            Some(UiHit::CloseTab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 384.0, 16.0),
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

        session.add_bookmark("Home", "examples/hello.html");
        assert_eq!(
            hit_test_ui(&session, 800.0, 20.0, TOP_BAR_HEIGHT + 10.0),
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
