//! Flux window interface: tab strip, navigation bar, and address input.

use std::path::Path;

use flux_layout::Rect;
use flux_raster::{Color, Framebuffer};

use crate::{Session, Viewport};

pub const TAB_STRIP_HEIGHT: f32 = 36.0;
pub const NAV_BAR_HEIGHT: f32 = 40.0;
pub const TOP_BAR_HEIGHT: f32 = TAB_STRIP_HEIGHT + NAV_BAR_HEIGHT;

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

#[derive(Clone, Debug, PartialEq)]
pub enum UiHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    AddressField,
    Page { x: f32, y: f32 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiState {
    editing_address: bool,
    input: String,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_editing_address(&self) -> bool {
        self.editing_address
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
        height: (window_height - TOP_BAR_HEIGHT).max(1.0),
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
        if address_rect(window_width).contains(x, y) {
            return Some(UiHit::AddressField);
        }
        return None;
    }
    Some(UiHit::Page {
        x,
        y: y - TOP_BAR_HEIGHT,
    })
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
        fb.fill_rect(rect.x, rect.y, rect.width, rect.height, bg);
        if is_active {
            fb.fill_rect(rect.x, rect.y, rect.width, 2.0, ACCENT);
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
    fb.fill_rect(plus.x, plus.y, plus.width, plus.height, TAB_IDLE_BG);
    fb.draw_text(plus.x + 9.0, plus.y + 4.0, "+", 16.0, ACCENT);

    // Navigation bar
    fb.fill_rect(0.0, TAB_STRIP_HEIGHT, width_f, NAV_BAR_HEIGHT, NAV_BG);
    fb.fill_rect(0.0, TOP_BAR_HEIGHT - 1.0, width_f, 1.0, DIVIDER);

    let active_tab = session.active();
    draw_control(&mut fb, back_rect(), "<", active_tab.can_go_back());
    draw_control(&mut fb, forward_rect(), ">", active_tab.can_go_forward());
    draw_control(&mut fb, reload_rect(), "R", active_tab.current().is_some());

    // Address field
    let addr = address_rect(width_f);
    let border = if ui.is_editing_address() {
        ACCENT
    } else {
        DIVIDER
    };
    fb.fill_rect(addr.x, addr.y, addr.width, addr.height, border);
    fb.fill_rect(
        addr.x + 1.0,
        addr.y + 1.0,
        (addr.width - 2.0).max(0.0),
        (addr.height - 2.0).max(0.0),
        FIELD_BG,
    );

    let raw_url = if ui.is_editing_address() {
        format!("{}|", ui.input())
    } else {
        active_tab
            .current()
            .map(|page| page.url.clone())
            .unwrap_or_default()
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
        "Enter an http://, https://, or file path"
    } else {
        shown_url.as_str()
    };
    fb.draw_text(addr.x + 8.0, addr.y + 5.0, display_text, 14.0, addr_color);

    // Blit page viewport below top bar
    let page_fb = session.rasterize();
    let top_y = TOP_BAR_HEIGHT as u32;
    let copy_w = page_fb.width().min(width);
    let copy_h = page_fb.height().min(height.saturating_sub(top_y));
    for py in 0..copy_h {
        for px in 0..copy_w {
            if let Some(color) = page_fb.pixel(px, py) {
                fb.set_pixel(px as i32, (top_y + py) as i32, color);
            }
        }
    }

    fb
}

fn draw_control(fb: &mut Framebuffer, rect: Rect, glyph: &str, enabled: bool) {
    fb.fill_rect(rect.x, rect.y, rect.width, rect.height, CONTROL_BG);
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

fn address_rect(window_width: f32) -> Rect {
    Rect {
        x: 108.0,
        y: TAB_STRIP_HEIGHT + 6.0,
        width: (window_width - 116.0).max(60.0),
        height: 28.0,
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
        assert_eq!(
            hit_test_ui(&session, 800.0, 220.0, 50.0),
            Some(UiHit::AddressField)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 40.0, 100.0),
            Some(UiHit::Page {
                x: 40.0,
                y: 100.0 - TOP_BAR_HEIGHT
            })
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
    }
}
