// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Window and input loop.
//!
//! Windows uses Win32. Linux uses X11. Both rasterize through compose_frame
//! into a CPU framebuffer. Windows presents that buffer with Direct3D 11 when
//! the device and `compositor.hlsl` compile succeed, and otherwise copies it
//! with SetDIBitsToDevice. Linux copies it with XPutImage. The page is still
//! rasterized on the CPU. Video frames are still decoded on the CPU.
//!
//! Zoom steps are Session::zoom_in, zoom_out, and reset_zoom in lib.rs.
//! This file only requests them. The settings button opens anvil://settings.
//! There is no pause flag. F9 is not bound on either platform.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anvil_raster::Framebuffer;

use crate::frontend::{
    DOWNLOADS_URL, HISTORY_URL, HOME_URL, INSPECT_URL, SECURITY_URL, SETTINGS_URL,
};
use crate::ui::{
    AppMenuAction, ContextMenuAction, SitePopoverAction, TOP_BAR_HEIGHT, UiHit, UiState,
    compose_frame_with_page_fb, content_viewport, content_viewport_for,
    hit_test_ui_with_state_and_height, top_header_height_for,
};
#[cfg(windows)]
use crate::ui::{cpp_ui_dwm_configure_window, cpp_ui_window_action, scrollbar_ratio_at};
use crate::{
    ClickOutcome, Page, Session, Viewport, load_post_in_tab, load_target_in_tab,
    navigation_error_page,
};

/// Tab id plus a finished page or error, shared with the load thread.
/// A page for a tab that closed while it loaded is dropped.
type PendingLoad = Arc<Mutex<LoadQueue>>;

/// Tickets prevent stopped or superseded loads from replacing a newer page.
#[derive(Default)]
struct LoadQueue {
    next: u64,
    active: std::collections::BTreeMap<u64, u64>,
    ready: std::collections::VecDeque<(u64, Result<Page, String>)>,
}

impl LoadQueue {
    fn begin(&mut self, tab: u64) -> u64 {
        self.cancel(tab);
        self.next = self.next.checked_add(1).expect("load ticket exhausted");
        self.active.insert(tab, self.next);
        self.next
    }
    fn cancel(&mut self, tab: u64) {
        self.active.remove(&tab);
        self.ready.retain(|(id, _)| *id != tab);
    }
    fn finish(&mut self, tab: u64, ticket: u64, page: Result<Page, String>) {
        if self.active.get(&tab) == Some(&ticket) {
            self.active.remove(&tab);
            self.ready.push_back((tab, page));
        }
    }
    fn take(&mut self) -> Option<(u64, Result<Page, String>)> {
        self.ready.pop_front()
    }
    fn is_loading(&self, tab: u64) -> bool {
        self.active.contains_key(&tab)
    }
}

/// Run a page load and turn a panic into the error string page load failed.
/// The worker uses this so a bad document does not abort the window thread.
fn catch_load(work: impl FnOnce() -> Result<Page, String>) -> Result<Page, String> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(_) => Err("page load failed".to_string()),
    }
}

/// Session, toolbar state, window size, CPU framebuffer, and the in-flight load.
struct Host {
    session: Session,
    ui: UiState,
    window_width: u32,
    window_height: u32,
    framebuffer: Framebuffer,
    cached_page_fb: Option<Framebuffer>,
    pending_load: PendingLoad,
    /// Animation frames are being drawn for the active page.
    animating: bool,
    /// Something happened that may have started a transition or animation;
    /// look again before the next idle wait.
    check_animation: bool,
    /// Earliest time for the next animation frame.
    next_animation_frame: Instant,
    /// The Win32 animation timer is running.
    #[cfg(windows)]
    animation_timer: bool,
    /// Optional dirty scanline range `(y0, y1)` for partial upload on chrome-only redraws.
    #[cfg(any(windows, target_os = "linux"))]
    dirty_rows: Option<(u32, u32)>,
    /// Window presentation state (Direct3D 11 on Windows, EGL/OpenGL 3.3 Core + dirty-band SHM on Linux).
    #[cfg(any(windows, target_os = "linux"))]
    present: crate::present::PresentPath,
}

/// Time between animation frames, about 60 a second.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);

impl Host {
    fn history_back(&mut self) -> bool {
        self.stop_loading();
        self.session.back()
    }
    fn history_forward(&mut self) -> bool {
        self.stop_loading();
        self.session.forward()
    }

    fn sync_loading(&mut self) {
        let loading = self
            .pending_load
            .lock()
            .is_ok_and(|q| q.is_loading(self.session.active_tab_id()));
        self.ui.set_loading(loading);
    }

    /// Abandon the active request. Bounded network work may finish, but cannot publish.
    fn stop_loading(&mut self) {
        if let Ok(mut queue) = self.pending_load.lock() {
            queue.cancel(self.session.active_tab_id());
        }
        self.ui.set_loading(false);
    }

    /// Draw the next animation frame when one is due.
    ///
    /// `check_animation` is set after input and page loads; only then is the
    /// page asked whether anything started, so a still page costs nothing.
    /// While something runs, the page clock ticks every [`ANIMATION_FRAME`]
    /// and the page is restyled. Returns true when the framebuffer changed
    /// and should be presented.
    fn animation_step(&mut self) -> bool {
        if self.check_animation {
            self.check_animation = false;
            self.animating = self.session.is_animating();
        }
        if !self.animating {
            return false;
        }
        let now = Instant::now();
        if now < self.next_animation_frame {
            return false;
        }
        self.next_animation_frame = now + ANIMATION_FRAME;
        let changed = self.session.animation_frame();
        self.animating = self.session.is_animating();
        if changed {
            self.redraw();
        }
        changed
    }

    /// Keep the session viewport aligned with the active header, status bar, and inspector dock.
    fn sync_viewport(&mut self) {
        let vp = content_viewport_for(
            &self.session,
            &self.ui,
            self.window_width as f32,
            self.window_height as f32,
        );
        let cur = self.session.viewport();
        if (vp.width - cur.width).abs() > 0.5 || (vp.height - cur.height).abs() > 0.5 {
            self.session.set_viewport(vp);
            self.cached_page_fb = None;
        }
    }

    /// Replace the CPU framebuffer with compose_frame for the current session and toolbar.
    /// Does not copy pixels to the window. WM_PAINT or present_x11 does that.
    fn redraw(&mut self) {
        self.sync_loading();
        self.sync_viewport();
        let page_fb = self.session.rasterize();
        self.framebuffer = compose_frame_with_page_fb(
            &self.session,
            &self.ui,
            self.window_width,
            self.window_height,
            &page_fb,
        );
        self.cached_page_fb = Some(page_fb);
        #[cfg(any(windows, target_os = "linux"))]
        {
            self.dirty_rows = None;
        }
    }

    /// Update the cached page framebuffer in-place for a main-viewport scroll from `prev_scroll`
    /// using scanline shifting + exposed-band rasterization when eligible, falling back to `redraw()`.
    #[cfg(any(windows, target_os = "linux"))]
    fn redraw_after_scroll(&mut self, prev_scroll: f32) {
        self.sync_viewport();
        let cur_scroll = self.session.scroll();
        if (cur_scroll - prev_scroll).abs() > f32::EPSILON
            && let Some(page_fb) = self.cached_page_fb.as_mut()
            && self.session.rasterize_scroll_into(prev_scroll, page_fb)
        {
            self.framebuffer = compose_frame_with_page_fb(
                &self.session,
                &self.ui,
                self.window_width,
                self.window_height,
                page_fb,
            );
            self.dirty_rows = None;
            return;
        }
        self.redraw();
    }

    /// Re-compose only the browser chrome and overlays using the cached page framebuffer.
    fn redraw_chrome_only(&mut self) {
        self.sync_loading();
        let Some(page_fb) = self.cached_page_fb.as_ref() else {
            self.redraw();
            return;
        };
        self.framebuffer = compose_frame_with_page_fb(
            &self.session,
            &self.ui,
            self.window_width,
            self.window_height,
            page_fb,
        );
        #[cfg(any(windows, target_os = "linux"))]
        {
            let has_overlay_or_dock = self.ui.is_inspector_open()
                || self.ui.is_app_menu_open()
                || self.ui.is_site_popover_open()
                || self.ui.is_tab_search_open()
                || self.ui.is_downloads_popover_open()
                || self.ui.is_find_open()
                || self.ui.context_menu_pos().is_some()
                || self.ui.select_dropdown().is_some()
                || self.ui.is_editing_address()
                || self.ui.hovered_link().is_some()
                || self.ui.is_loading()
                || self.ui.cursor_pos().is_some_and(|(_, cy)| cy <= 40.0);
            let dirty_bottom = if has_overlay_or_dock {
                self.window_height
            } else {
                116.min(self.window_height)
            };
            self.dirty_rows = Some((0, dirty_bottom));
        }
    }

    /// URL of the active page, or an empty string when the tab has no page.
    fn current_url(&self) -> String {
        self.session
            .active()
            .current()
            .map(|page| page.url.clone())
            .unwrap_or_default()
    }

    /// Caption text. Uses the page title when it is non-empty, otherwise the URL.
    fn window_title(&self) -> String {
        let Some(page) = self.session.active().current() else {
            return "Anvil".to_string();
        };
        if page.title.is_empty() {
            format!("Anvil - {}", page.url)
        } else {
            format!("{} - Anvil", page.title)
        }
    }

    /// How many times the open find query occurs in the active page.
    fn find_match_count(&self) -> usize {
        let query = self.ui.find_query();
        if query.is_empty() {
            return 0;
        }
        let case_sensitive = self.ui.is_find_case_sensitive();
        self.session
            .active()
            .current()
            .map(|p| p.find_text_matches_with_case(query, case_sensitive).len())
            .unwrap_or(0)
    }

    /// Scroll so the active find match sits about 80 pixels below the top of the page.
    /// Match rectangles are in page space and are multiplied by Session::zoom_factor.
    fn scroll_to_active_find_match(&mut self) {
        let query = self.ui.find_query().to_string();
        if query.is_empty() {
            return;
        }
        let case_sensitive = self.ui.is_find_case_sensitive();
        let Some(rect) = self.session.active().current().and_then(|page| {
            let matches = page.find_text_matches_with_case(&query, case_sensitive);
            if matches.is_empty() {
                None
            } else {
                let idx = self.ui.find_active_match().min(matches.len() - 1);
                Some(matches[idx])
            }
        }) else {
            return;
        };
        // Page-space match y, times zoom, then 80 pixels of margin, becomes the scroll target.
        let scale = self.session.zoom_factor();
        let target_y = (rect.y * scale - 80.0).max(0.0);
        let delta = target_y - self.session.scroll();
        let _ = self.session.scroll_by(delta);
    }
}

// Windows message loop. The CPU framebuffer is presented with Direct3D 11
// when that device is available. SetDIBitsToDevice is the fallback.
#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::{
        AppMenuAction, ClickOutcome, ContextMenuAction, DOWNLOADS_URL, HISTORY_URL, HOME_URL, Host,
        INSPECT_URL, SECURITY_URL, SETTINGS_URL, Session, SitePopoverAction, TOP_BAR_HEIGHT, UiHit,
        UiState, Viewport, compose_frame_with_page_fb, content_viewport, content_viewport_for,
        cpp_ui_dwm_configure_window, cpp_ui_window_action, hit_test_ui_with_state_and_height,
        load_post_in_tab, load_target_in_tab, navigation_error_page, scrollbar_ratio_at,
        top_header_height_for,
    };
    use crate::keys::{Key, KeyEffect, Modifiers, apply_key_command, command_for, focus_of};

    /// Win32 window handle. Stored as a pointer-sized value without a windows crate.
    type Hwnd = *mut c_void;
    /// Device context used while WM_PAINT is copying pixels.
    type Hdc = *mut c_void;
    /// Module instance. This loop passes null and lets Win32 use the current module.
    type Hinstance = *mut c_void;
    /// Icon handle created from the in-memory logo.
    type Hicon = *mut c_void;
    /// Cursor handle loaded for arrow, I-beam, or hand.
    type Hcursor = *mut c_void;
    /// Background brush slot on the window class. Left null so the blit covers the client.
    type Hbrush = *mut c_void;
    /// Menu handle. No Win32 menu bar is attached.
    type Hmenu = *mut c_void;
    /// Value returned from the window procedure.
    type Lresult = isize;
    /// Message word parameter. Key messages carry the virtual key.
    type Wparam = usize;
    /// Message long parameter. Mouse messages pack client x and y here.
    type Lparam = isize;

    /// Class style. The client is redrawn when the height changes.
    const CS_VREDRAW: u32 = 0x0001;
    /// Class style. The client is redrawn when the width changes.
    const CS_HREDRAW: u32 = 0x0002;
    /// Caption, sizing border, system menu, and min and max buttons.
    const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    /// Show the window as soon as it is created.
    const WS_VISIBLE: u32 = 0x1000_0000;
    /// Win32 chooses the initial window position. The size is still win_w and win_h.
    const CW_USEDEFAULT: i32 = -2_147_483_648;
    /// Slot that stores the Host pointer on the HWND.
    const GWLP_USERDATA: i32 = -21;

    /// The window is going away. The handler saves the profile and posts quit.
    const WM_DESTROY: u32 = 0x0002;
    /// The client size changed. The handler updates the session viewport.
    const WM_SIZE: u32 = 0x0005;
    /// The client needs pixels. The handler blits the CPU framebuffer.
    const WM_PAINT: u32 = 0x000F;
    /// Non-client create. The Host pointer is stored before the window is live.
    const WM_NCCREATE: u32 = 0x0081;
    /// Non-client destroy. The Host allocation is dropped here.
    const WM_NCDESTROY: u32 = 0x0082;
    /// A key went down. Alt combinations arrive as WM_SYSKEYDOWN instead.
    const WM_KEYDOWN: u32 = 0x0100;
    /// Translated character from TranslateMessage. Used for typing, not for shortcuts.
    const WM_CHAR: u32 = 0x0102;
    /// The pointer moved in the client area.
    const WM_MOUSEMOVE: u32 = 0x0200;
    /// Left button down in the client area.
    const WM_LBUTTONDOWN: u32 = 0x0201;
    /// Left button up in the client area.
    const WM_LBUTTONUP: u32 = 0x0202;
    /// Right button down in the client area.
    const WM_RBUTTONDOWN: u32 = 0x0204;
    /// Middle button down in the client area.
    const WM_MBUTTONDOWN: u32 = 0x0207;
    /// Left mouse button bit in wParam during WM_MOUSEMOVE.
    const MK_LBUTTON: usize = 0x0001;
    /// Wheel moved. The high word of wParam is the delta, in multiples of 120.
    const WM_MOUSEWHEEL: u32 = 0x020A;
    /// Private message 0x8001. A worker posts it when a remote load finishes.
    const WM_ANVIL_PAGE_READY: u32 = 0x8001;
    /// A SetTimer timer fired. Used for animation frames.
    const WM_TIMER: u32 = 0x0113;
    /// Timer id for animation frames.
    const ANIMATION_TIMER_ID: usize = 1;
    /// Per-Monitor High-DPI V2 awareness context (-4).
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
    /// Window moved to a monitor with a different DPI. lParam points to the suggested WinRect.
    const WM_DPICHANGED: u32 = 0x02E0;
    /// Keep current Z-order in SetWindowPos.
    const SWP_NOZORDER: u32 = 0x0004;
    /// Do not activate window in SetWindowPos.
    const SWP_NOACTIVATE: u32 = 0x0010;

    /// Backspace. In a field it deletes. With no field it goes back.
    const VK_BACK: usize = 0x08;
    /// Tab. In the address bar it accepts the suggestion.
    const VK_TAB: usize = 0x09;
    /// Enter. Submits the address bar, the find query, or the focused page field.
    const VK_RETURN: usize = 0x0D;
    /// Space. Presses the focused toolbar button.
    const VK_SPACE: usize = 0x20;
    /// Shift. Read with GetKeyState.
    const VK_SHIFT: i32 = 0x10;
    /// Control. Read with GetKeyState. It is not a shortcut by itself.
    const VK_CONTROL: i32 = 0x11;
    /// Alt. Read with GetKeyState. Alt key-downs arrive as WM_SYSKEYDOWN.
    const VK_MENU: i32 = 0x12;
    /// Escape. Closes the focused field or a menu, and otherwise destroys the window.
    const VK_ESCAPE: usize = 0x1B;
    /// Page Up. Scrolls up by 80 percent of the page viewport.
    const VK_PRIOR: usize = 0x21;
    /// Page Down. Scrolls down by 80 percent of the page viewport.
    const VK_NEXT: usize = 0x22;
    /// End. Scrolls to the bottom when no field is focused.
    const VK_END: usize = 0x23;
    /// Home. Alt+Home opens the start page. Alone, it scrolls to the top.
    const VK_HOME: usize = 0x24;
    /// Left. Alt+Left goes back. Alone, it is not bound.
    const VK_LEFT: usize = 0x25;
    /// Up. Scrolls up 40 pixels when no field is focused.
    const VK_UP: usize = 0x26;
    /// Right. Alt+Right goes forward. Alone, it is not bound.
    const VK_RIGHT: usize = 0x27;
    /// Down. Scrolls down 40 pixels when no field is focused.
    const VK_DOWN: usize = 0x28;
    /// Delete. Removes the character after the caret in focused fields.
    const VK_DELETE: usize = 0x2E;
    /// Digit 0. With Control, resets zoom to 100 percent.
    const VK_KEY_0: usize = 0x30;
    /// Digit 1. With Control, switches to the first tab.
    const VK_KEY_1: usize = 0x31;
    /// Digit 9. With Control, switches to the last tab.
    const VK_KEY_9: usize = 0x39;
    /// A. With Control in the address bar, selects all text.
    const VK_KEY_A: usize = 0x41;
    /// B. With Control+Shift, toggles the bookmarks bar.
    const VK_KEY_B: usize = 0x42;
    /// C. With Control, copies the address bar or the selection.
    const VK_KEY_C: usize = 0x43;
    /// F. With Control, opens find.
    const VK_KEY_F: usize = 0x46;
    /// H. With Control, opens anvil://history.
    const VK_KEY_H: usize = 0x48;
    /// J. With Control, opens anvil://downloads.
    const VK_KEY_J: usize = 0x4A;
    /// L. With Control, focuses the address bar.
    const VK_KEY_L: usize = 0x4C;
    /// R. With Control, reloads the active URL.
    const VK_KEY_R: usize = 0x52;
    /// S. With Control, saves the active page HTML to the downloads vault.
    const VK_KEY_S: usize = 0x53;
    /// P. With Control, saves the active page as a PDF to the downloads vault.
    const VK_KEY_P: usize = 0x50;
    /// T. With Control, opens a new tab on the start page (or reopens closed tab with Shift).
    const VK_KEY_T: usize = 0x54;
    /// U. With Control, opens view-source for the active page.
    const VK_KEY_U: usize = 0x55;
    /// V. With Control, pastes.
    const VK_KEY_V: usize = 0x56;
    /// W. With Control, closes the active tab.
    const VK_KEY_W: usize = 0x57;
    /// X. With Control, cuts the address bar or the focused field.
    const VK_KEY_X: usize = 0x58;
    /// Numpad 0. With Control, resets zoom to 100 percent.
    const VK_NUMPAD0: usize = 0x60;
    /// Numpad plus. With Control, zooms in one step.
    const VK_ADD: usize = 0x6B;
    /// Numpad minus. With Control, zooms out one step.
    const VK_SUBTRACT: usize = 0x6D;
    /// F5 reloads the active URL. F9 is not declared.
    const VK_F5: usize = 0x74;
    /// F6 focuses the address bar.
    const VK_F6: usize = 0x75;
    /// F10. Puts keyboard focus on the toolbar.
    const VK_F10: usize = 0x79;
    /// F12 toggles the inspector dock. This binding is Windows-only.
    const VK_F12: usize = 0x7B;
    /// Plus key. With Control, zooms in one step.
    const VK_OEM_PLUS: usize = 0xBB;
    /// Minus key. With Control, zooms out one step.
    const VK_OEM_MINUS: usize = 0xBD;
    /// Key down while Alt is held. Handled together with WM_KEYDOWN.
    const WM_SYSKEYDOWN: u32 = 0x0104;
    /// DWM attribute 20. Asks for a dark title bar.
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    /// Clipboard format for UTF-16 text.
    const CF_UNICODETEXT: u32 = 13;
    /// Clipboard allocation that the system may move.
    const GMEM_MOVEABLE: u32 = 0x0002;

    /// Arrow cursor, used when the pointer is over plain page content.
    const IDC_ARROW: *const u16 = 32512 as *const u16;
    /// I-beam, used for the address bar, find field, and page text fields.
    const IDC_IBEAM: *const u16 = 32513 as *const u16;
    /// Hand cursor, used for buttons and links.
    const IDC_HAND: *const u16 = 32649 as *const u16;
    /// Uncompressed DIB. The blit is 32-bit BGRA.
    const BI_RGB: u32 = 0;
    /// SetDIBitsToDevice color use. The table holds color values, not indexes.
    const DIB_RGB_COLORS: u32 = 0;

    /// WNDCLASSW. The procedure is wnd_proc and the class name is AnvilWindow.
    #[repr(C)]
    struct WndClassW {
        style: u32,
        lpfn_wnd_proc: unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: Hinstance,
        h_icon: Hicon,
        h_cursor: Hcursor,
        hbr_background: Hbrush,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    /// POINT. Cursor position carried on MSG, not used for hit testing.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }

    /// MSG. The loop fills this from GetMessageW.
    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        w_param: Wparam,
        l_param: Lparam,
        time: u32,
        pt: Point,
        l_private: u32,
    }

    /// RECT. Client bounds from GetClientRect, and the paint rectangle.
    #[repr(C)]
    #[derive(Default)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    /// PAINTSTRUCT filled by BeginPaint and passed to EndPaint.
    #[repr(C)]
    struct PaintStruct {
        hdc: Hdc,
        f_erase: i32,
        rc_paint: WinRect,
        f_restore: i32,
        f_inc_update: i32,
        rgb_reserved: [u8; 32],
    }

    /// CREATESTRUCTW. lp_create_params is the Host pointer on WM_NCCREATE.
    #[repr(C)]
    struct CreateStructW {
        lp_create_params: *mut c_void,
        h_instance: Hinstance,
        h_menu: Hmenu,
        hwnd_parent: Hwnd,
        cy: i32,
        cx: i32,
        y: i32,
        x: i32,
        style: i32,
        lpsz_name: *const u16,
        lpsz_class: *const u16,
        dw_ex_style: u32,
    }

    /// BITMAPINFOHEADER for a 32-bit top-down-flipped DIB.
    #[repr(C)]
    struct BitmapInfoHeader {
        bi_size: u32,
        bi_width: i32,
        bi_height: i32,
        bi_planes: u16,
        bi_bit_count: u16,
        bi_compression: u32,
        bi_size_image: u32,
        bi_x_pels_per_meter: i32,
        bi_y_pels_per_meter: i32,
        bi_clr_used: u32,
        bi_clr_important: u32,
    }

    /// BITMAPINFO passed to SetDIBitsToDevice. One unused color entry follows the header.
    #[repr(C)]
    struct BitmapInfo {
        bmi_header: BitmapInfoHeader,
        bmi_colors: [u32; 1],
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        /// Register the AnvilWindow class.
        fn RegisterClassW(lp_wnd_class: *const WndClassW) -> u16;
        /// Create the overlapped window and pass the Host pointer as creation data.
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: Hwnd,
            h_menu: Hmenu,
            h_instance: Hinstance,
            lp_param: *mut c_void,
        ) -> Hwnd;
        /// Default handling for messages this loop does not consume.
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
        /// Block until the next message. A zero return ends the loop after WM_DESTROY.
        fn GetMessageW(
            lp_msg: *mut Msg,
            h_wnd: Hwnd,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> i32;
        /// Turn key-downs into WM_CHAR for text input.
        fn TranslateMessage(lp_msg: *const Msg) -> i32;
        /// Call wnd_proc for the message.
        fn DispatchMessageW(lp_msg: *const Msg) -> Lresult;
        /// Make GetMessageW return zero so open can finish.
        fn PostQuitMessage(n_exit_code: i32);
        /// Queue WM_ANVIL_PAGE_READY from the load thread onto this window.
        fn PostMessageW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> i32;
        /// Destroy the HWND. Used when Escape closes the window.
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        /// Start WM_PAINT and return the device context to blit into.
        fn BeginPaint(hwnd: Hwnd, lp_paint: *mut PaintStruct) -> Hdc;
        /// Finish WM_PAINT and validate the client area.
        fn EndPaint(hwnd: Hwnd, lp_paint: *const PaintStruct) -> i32;
        /// Start a repeating WM_TIMER for animation frames.
        fn SetTimer(
            hwnd: Hwnd,
            n_id_event: usize,
            u_elapse: u32,
            lp_timer_func: *const c_void,
        ) -> usize;
        /// Stop the animation timer.
        fn KillTimer(hwnd: Hwnd, u_id_event: usize) -> i32;
        /// Mark the whole client dirty so WM_PAINT runs again.
        fn InvalidateRect(hwnd: Hwnd, lp_rect: *const WinRect, b_erase: i32) -> i32;
        /// Read the client width and height on WM_SIZE.
        fn GetClientRect(hwnd: Hwnd, lp_rect: *mut WinRect) -> i32;
        /// Set the caption from window_title.
        fn SetWindowTextW(hwnd: Hwnd, lp_string: *const u16) -> i32;
        /// Store or clear the Host pointer in GWLP_USERDATA.
        fn SetWindowLongPtrW(hwnd: Hwnd, n_index: i32, dw_new_long: isize) -> isize;
        /// Read the Host pointer from GWLP_USERDATA.
        fn GetWindowLongPtrW(hwnd: Hwnd, n_index: i32) -> isize;
        /// Load a system cursor by id.
        fn LoadCursorW(h_instance: Hinstance, lp_cursor_name: *const u16) -> Hcursor;
        /// Set the cursor for the current hit. Called on every move.
        fn SetCursor(h_cursor: Hcursor) -> Hcursor;
        /// High bit set means the key is down. Used for Control and Alt.
        fn GetKeyState(n_virt_key: i32) -> i16;
        /// Build the window icon from the 32 by 32 logo pixels.
        fn CreateIcon(
            h_instance: Hinstance,
            n_width: i32,
            n_height: i32,
            c_planes: u8,
            c_bits_pixel: u8,
            lpb_and_bits: *const u8,
            lpb_xor_bits: *const u8,
        ) -> Hicon;
        /// Send WM_SETICON for the small and large icons.
        fn SendMessageW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
        /// Take the clipboard. A zero return means the paste or copy is skipped.
        fn OpenClipboard(h_wnd_new_owner: Hwnd) -> i32;
        /// Release the clipboard.
        fn CloseClipboard() -> i32;
        /// Clear the clipboard before writing UTF-16 text.
        fn EmptyClipboard() -> i32;
        /// Read CF_UNICODETEXT. The pointer is locked, not freed, by this loop.
        fn GetClipboardData(u_format: u32) -> *mut c_void;
        /// Give a moveable UTF-16 block to the clipboard. The system owns it after success.
        fn SetClipboardData(u_format: u32, h_mem: *mut c_void) -> *mut c_void;
        /// Enable Per-Monitor High-DPI V2 scaling before window creation.
        fn SetProcessDpiAwarenessContext(value: isize) -> i32;
        /// Reposition and resize the window when WM_DPICHANGED arrives.
        fn SetWindowPos(
            h_wnd: Hwnd,
            h_wnd_insert_after: Hwnd,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            u_flags: u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// Allocate the clipboard block.
        fn GlobalAlloc(u_flags: u32, dw_bytes: usize) -> *mut c_void;
        /// Lock the clipboard block for a copy.
        fn GlobalLock(h_mem: *mut c_void) -> *mut c_void;
        /// Unlock the clipboard block.
        fn GlobalUnlock(h_mem: *mut c_void) -> i32;
        /// Free the clipboard block when SetClipboardData does not take it.
        fn GlobalFree(h_mem: *mut c_void) -> *mut c_void;
        /// Size in bytes of a clipboard block.
        fn GlobalSize(h_mem: *mut c_void) -> usize;
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        /// Request a dark title bar. Failure is ignored.
        fn DwmSetWindowAttribute(
            hwnd: Hwnd,
            dw_attribute: u32,
            pv_attribute: *const c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        /// Copy bottom-up BGRA pixels into the window.
        /// This is the Windows present path. It is not a GPU swapchain.
        fn SetDIBitsToDevice(
            hdc: Hdc,
            x_dest: i32,
            y_dest: i32,
            w: u32,
            h: u32,
            x_src: i32,
            y_src: i32,
            start_scan: u32,
            c_lines: u32,
            lpv_bits: *const c_void,
            lpbmi: *const BitmapInfo,
            color_use: u32,
        ) -> i32;
    }

    /// Write UTF-16 text to the clipboard. Null bytes are not added beyond the wide() nul.
    fn win32_set_clipboard_text(hwnd: Hwnd, text: &str) {
        let utf16 = wide(text);
        let byte_len = utf16.len() * size_of::<u16>();
        unsafe {
            if OpenClipboard(hwnd) == 0 {
                return;
            }
            let _ = EmptyClipboard();
            let h_mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
            if !h_mem.is_null() {
                let dst = GlobalLock(h_mem) as *mut u16;
                if !dst.is_null() {
                    std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
                    let _ = GlobalUnlock(h_mem);
                    if SetClipboardData(CF_UNICODETEXT, h_mem).is_null() {
                        let _ = GlobalFree(h_mem);
                    }
                } else {
                    let _ = GlobalFree(h_mem);
                }
            }
            let _ = CloseClipboard();
        }
    }

    /// Read UTF-16 clipboard text, stopping at 65536 code units, a nul, or the end of the block.
    fn win32_get_clipboard_text(hwnd: Hwnd) -> Option<String> {
        unsafe {
            if OpenClipboard(hwnd) == 0 {
                return None;
            }
            let mut result = None;
            let h_data = GetClipboardData(CF_UNICODETEXT);
            if !h_data.is_null() {
                let src = GlobalLock(h_data) as *const u16;
                if !src.is_null() {
                    // Another program put this block on the clipboard and it
                    // may not end in a nul, so never read past its real size.
                    let max_units = (GlobalSize(h_data) / size_of::<u16>()).min(65_536);
                    let mut len = 0usize;
                    while len < max_units && *src.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(src, len);
                    result = Some(String::from_utf16_lossy(slice));
                    let _ = GlobalUnlock(h_data);
                }
            }
            let _ = CloseClipboard();
            result
        }
    }

    impl Host {
        /// Copy the address text when that field is focused and non-empty.
        /// Otherwise copy the page selection or focused field. Updates the session clipboard and Win32.
        fn copy_to_clipboard(&mut self, hwnd: Hwnd) -> bool {
            if self.ui.is_editing_address() && !self.ui.input().is_empty() {
                let text = self.ui.input().to_string();
                self.session.set_clipboard_text(&text);
                win32_set_clipboard_text(hwnd, &text);
                return true;
            }
            if let Some(text) = self.session.copy_selection_or_focused() {
                win32_set_clipboard_text(hwnd, &text);
                return true;
            }
            false
        }

        /// Cut the address bar, or the focused page field, into the session clipboard and Win32.
        fn cut_to_clipboard(&mut self, hwnd: Hwnd) -> bool {
            if let Some(text) = self.ui.cut_address() {
                self.session.set_clipboard_text(&text);
                win32_set_clipboard_text(hwnd, &text);
                return true;
            }
            if let Some(text) = self.session.cut_focused_input() {
                win32_set_clipboard_text(hwnd, &text);
                return true;
            }
            false
        }

        /// Read Win32 text into the session clipboard, then insert it.
        /// The address bar takes it whole. Find keeps printable ASCII. Otherwise the focused page field takes it.
        fn paste_from_clipboard(&mut self, hwnd: Hwnd) -> bool {
            if let Some(sys_clip) = win32_get_clipboard_text(hwnd)
                && !sys_clip.is_empty()
            {
                self.session.set_clipboard_text(&sys_clip);
            }
            let clip = self.session.clipboard_text().to_string();
            if clip.is_empty() {
                return false;
            }
            if self.ui.is_editing_address() {
                return self.ui.paste_into_address(&clip);
            }
            if self.ui.is_find_open() {
                for ch in clip.chars().filter(|c| (' '..='~').contains(c)) {
                    self.ui.push_char(ch);
                }
                self.scroll_to_active_find_match();
                return true;
            }
            self.session.paste_into_focused_input(&clip)
        }

        /// Open target on the active tab.
        /// anvil://navigate is resolved with the session search engine. HSTS may upgrade the URL.
        /// HTTPS-only is applied by anvil-net on the load thread.
        /// A shield hit records a block and opens the security page instead.
        /// http and https loads run on a worker and post WM_ANVIL_PAGE_READY. Other URLs open on this thread.
        fn start_navigation(&mut self, hwnd: Hwnd, target: &str) {
            let resolved = crate::frontend::resolve_navigation_query_with_engine(
                target,
                self.session.search_engine(),
            )
            .unwrap_or_else(|| target.to_string());
            self.stop_loading();
            if self.session.try_same_page_fragment_navigation(&resolved) {
                return;
            }
            let target_url = self.session.hsts_store().upgrade_url_if_hsts(&resolved);
            if target_url.starts_with("http://") || target_url.starts_with("https://") {
                if self
                    .session
                    .network_shield()
                    .inspect_url(&target_url, None)
                    .is_some()
                {
                    self.session.network_shield_mut().record_block();
                    let _ = self.session.open_target(SECURITY_URL);
                    return;
                }
                let viewport = self.session.viewport();
                let fetch_jar = self.session.cookie_jar_handle();
                let scripts_enabled = self.session.scripts_enabled_for(&target_url);
                let https_only = self.session.https_only_policy();
                let userscripts = self.session.userscript_injection();
                let tab_id = self.session.active_tab_id();
                let workers = self.session.tab_workers();
                let slot = Arc::clone(&self.pending_load);
                let ticket = slot.lock().expect("load queue").begin(tab_id);
                let hwnd_addr = hwnd as usize;
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let _https_only = anvil_net::enter_https_only(https_only);
                    let _userscripts = crate::userscripts::enter(userscripts);
                    let result = super::catch_load(|| {
                        load_target_in_tab(
                            &target_url,
                            viewport,
                            Some(fetch_jar),
                            scripts_enabled,
                            tab_id,
                            &workers,
                        )
                    });
                    if let Ok(mut guard) = slot.lock() {
                        guard.finish(tab_id, ticket, result);
                    }
                    unsafe {
                        PostMessageW(hwnd_addr as Hwnd, WM_ANVIL_PAGE_READY, 0, 0);
                    }
                });
            } else {
                let _ = self.session.open_target(&resolved);
                let _ = self.session.save_default_profile();
            }
        }

        /// Submit a POST body.
        /// http and https run on a worker. A shield hit returns without opening a page.
        /// Any other target is opened with GET-style open_target and the body is dropped.
        fn start_post_navigation(&mut self, hwnd: Hwnd, target: &str, body: &str) {
            self.stop_loading();
            if target.starts_with("http://") || target.starts_with("https://") {
                if self
                    .session
                    .network_shield()
                    .inspect_url(target, None)
                    .is_some()
                {
                    self.session.network_shield_mut().record_block();
                    return;
                }
                let url = target.to_string();
                let payload = body.to_string();
                let viewport = self.session.viewport();
                let fetch_jar = self.session.cookie_jar_handle();
                let scripts_enabled = self.session.scripts_enabled_for(&url);
                let https_only = self.session.https_only_policy();
                let userscripts = self.session.userscript_injection();
                let tab_id = self.session.active_tab_id();
                let workers = self.session.tab_workers();
                let slot = Arc::clone(&self.pending_load);
                let ticket = slot.lock().expect("load queue").begin(tab_id);
                let hwnd_addr = hwnd as usize;
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let _https_only = anvil_net::enter_https_only(https_only);
                    let _userscripts = crate::userscripts::enter(userscripts);
                    let result = super::catch_load(|| {
                        load_post_in_tab(
                            &url,
                            &payload,
                            viewport,
                            Some(fetch_jar),
                            scripts_enabled,
                            tab_id,
                            &workers,
                        )
                    });
                    if let Ok(mut guard) = slot.lock() {
                        guard.finish(tab_id, ticket, result);
                    }
                    unsafe {
                        PostMessageW(hwnd_addr as Hwnd, WM_ANVIL_PAGE_READY, 0, 0);
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }
    }

    /// Create the Win32 window and run GetMessageW until quit.
    /// The initial page is target, or anvil://home. The requested size is at least 480 by 320.
    /// The class is AnvilWindow. A dark title bar is requested and may fail quietly.
    pub fn open(target: Option<&str>, window_size: Viewport) -> Result<(), String> {
        let win_w = (window_size.width.round() as u32).max(480);
        let win_h = (window_size.height.round() as u32).max(320);
        let mut session = Session::new(content_viewport(win_w as f32, win_h as f32));
        if session.load_default_profile().is_err() {
            if win_w >= 3000 {
                let _ = session.zoom_in();
                let _ = session.zoom_in();
            } else if win_w >= 2000 {
                let _ = session.zoom_in();
            }
        }
        session.check_renderer_confinement_in_background();
        let ui = UiState::new();

        session.set_viewport(content_viewport_for(
            &session,
            &ui,
            win_w as f32,
            win_h as f32,
        ));
        let initial_target = target.unwrap_or(HOME_URL);
        session.open_target(initial_target)?;

        let page_fb = session.rasterize();
        let framebuffer = compose_frame_with_page_fb(&session, &ui, win_w, win_h, &page_fb);
        let is_light_theme = session.theme().is_light();
        let host = Box::new(Host {
            session,
            ui,
            window_width: win_w,
            window_height: win_h,
            framebuffer,
            cached_page_fb: Some(page_fb),
            pending_load: Arc::new(Mutex::new(super::LoadQueue::default())),
            animating: false,
            check_animation: true,
            next_animation_frame: std::time::Instant::now(),
            animation_timer: false,
            dirty_rows: None,
            present: crate::present::PresentPath::Untried,
        });

        let class_name = wide("AnvilWindow");
        let title = wide(&host.window_title());

        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let logo_img = crate::logo::render_anvil_logo(32);
            let mut xor_bgra = vec![0u8; 32 * 32 * 4];
            for (dst, src) in xor_bgra
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(logo_img.pixels.as_chunks::<4>().0)
            {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = src[3];
            }
            let and_mask = [0u8; 32 * 4];
            let app_icon = CreateIcon(
                null_mut(),
                32,
                32,
                1,
                32,
                and_mask.as_ptr(),
                xor_bgra.as_ptr(),
            );

            let wc = WndClassW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfn_wnd_proc: wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: null_mut(),
                h_icon: app_icon,
                h_cursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbr_background: null_mut(),
                lpsz_menu_name: null(),
                lpsz_class_name: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            let raw_host = Box::into_raw(host);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                win_w as i32,
                win_h as i32,
                null_mut(),
                null_mut(),
                null_mut(),
                raw_host.cast::<c_void>(),
            );
            if hwnd.is_null() {
                // Don't free the Host here. If creation got as far as
                // WM_NCCREATE, Windows already sent WM_NCDESTROY, and wnd_proc
                // freed it there, so freeing it again would be a double free.
                // If it never got that far, the Host just leaks once on a path
                // that ends the browser anyway.
                return Err("could not create the browser window".to_string());
            }

            if !app_icon.is_null() {
                /// WM_SETICON. wParam 0 sets the small icon and wParam 1 sets the large icon.
                const WM_SETICON: u32 = 0x0080;
                let _ = SendMessageW(hwnd, WM_SETICON, 0, app_icon as Lparam);
                let _ = SendMessageW(hwnd, WM_SETICON, 1, app_icon as Lparam);
            }

            let dark_mode: i32 = i32::from(!is_light_theme);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark_mode).cast::<c_void>(),
                size_of::<i32>() as u32,
            );
            cpp_ui_dwm_configure_window(hwnd as isize, !is_light_theme, false);

            let mut msg = Msg {
                hwnd: null_mut(),
                message: 0,
                w_param: 0,
                l_param: 0,
                time: 0,
                pt: Point { x: 0, y: 0 },
                l_private: 0,
            };
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    /// Win32 window procedure.
    /// Handled messages return 0. The rest go to DefWindowProcW.
    /// Key bindings are on the WM_KEYDOWN arm. F9 is not one of them.
    unsafe extern "system" fn wnd_proc(
        hwnd: Hwnd,
        msg: u32,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult {
        match msg {
            // Store the Host from CREATESTRUCT.lpCreateParams into GWLP_USERDATA, then finish default creation.
            WM_NCCREATE => {
                let create = l_param as *const CreateStructW;
                if !create.is_null() {
                    let ptr = unsafe { (*create).lp_create_params } as isize;
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);
                    }
                }
                unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
            }
            // Drop the Host box and clear GWLP_USERDATA so later messages cannot use it.
            WM_NCDESTROY => {
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
                if !ptr.is_null() {
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                        drop(Box::from_raw(ptr));
                    }
                }
                unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
            }
            // Take the worker result, clear the loading flag, install the page on that tab, and redraw.
            WM_ANVIL_PAGE_READY => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let finished = host
                        .pending_load
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    host.ui.set_loading(false);
                    if let Some((tab_id, result)) = finished {
                        let page = match result {
                            Ok(page) => page,
                            Err(message) => navigation_error_page(
                                "anvil://navigation-error",
                                &message,
                                host.session.viewport(),
                            ),
                        };
                        host.session.open_page_in_tab_id(tab_id, page);
                    }
                    refresh_window(hwnd, host);
                }
                0
            }
            // Read the client rect. On a real size change, store it and call content_viewport.
            // That helper drops the tab strip, navigation bar, and status bar, not the bookmarks row.
            WM_SIZE => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let mut rect = WinRect::default();
                    unsafe {
                        GetClientRect(hwnd, &mut rect);
                    }
                    let width = (rect.right - rect.left).max(0) as u32;
                    let height = (rect.bottom - rect.top).max(0) as u32;
                    if width > 0
                        && height > 0
                        && (width != host.window_width || height != host.window_height)
                    {
                        host.window_width = width;
                        host.window_height = height;
                        host.redraw();
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                }
                0
            }
            WM_DPICHANGED => {
                let suggested = l_param as *const WinRect;
                if !suggested.is_null() {
                    let r = unsafe { &*suggested };
                    let w = (r.right - r.left).max(480);
                    let h = (r.bottom - r.top).max(320);
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            null_mut(),
                            r.left,
                            r.top,
                            w,
                            h,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                0
            }
            // Hit-test the client point and set the cursor.
            // Address and find use the I-beam. Buttons, links, and the scrollbar use the hand.
            // A page text field uses the I-beam. Redraw when the hovered link changes or the pointer is over the toolbar.
            WM_MOUSEMOVE => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    // Window pixels from lParam. Not page pixels.
                    let (x, y) = mouse_coords(l_param);
                    let cursor_moved = host.ui.set_cursor_pos(x, y);
                    if (w_param & MK_LBUTTON) != 0 && host.ui.is_dragging_scrollbar() {
                        let ratio = scrollbar_ratio_at(&host.session, y);
                        let before = host.session.scroll();
                        let after = host.session.scroll_to_ratio(ratio);
                        if (after - before).abs() > f32::EPSILON {
                            host.redraw_after_scroll(before);
                            unsafe {
                                InvalidateRect(hwnd, null(), 0);
                            }
                        }
                        unsafe {
                            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                        }
                        return 0;
                    } else if (w_param & MK_LBUTTON) == 0 && host.ui.is_dragging_scrollbar() {
                        host.ui.stop_scrollbar_drag();
                    }
                    if (w_param & MK_LBUTTON) != 0
                        && let Some((sx, sy)) = host.ui.drag_select_start()
                    {
                        let header_h = top_header_height_for(&host.session, &host.ui);
                        let py = (y - header_h).max(0.0);
                        if (x - sx).hypot(py - sy) >= 4.0 {
                            let _ = host.session.select_text_range(sx, sy, x, py);
                            refresh_chrome_only(hwnd, host);
                        }
                        unsafe {
                            SetCursor(LoadCursorW(null_mut(), IDC_IBEAM));
                        }
                        return 0;
                    } else if (w_param & MK_LBUTTON) == 0 && host.ui.drag_select_start().is_some() {
                        host.ui.stop_drag_select();
                    }
                    let hit = hit_test_ui_with_state_and_height(
                        &host.session,
                        &host.ui,
                        host.window_width as f32,
                        host.window_height as f32,
                        x,
                        y,
                    );
                    let mut hovered = None;
                    let cursor = match hit {
                        Some(
                            UiHit::AddressField
                            | UiHit::FindInput
                            | UiHit::InspectorConsoleInput
                            | UiHit::InspectorStyleInput,
                        ) => IDC_IBEAM,
                        Some(
                            UiHit::Tab(_)
                            | UiHit::CloseTab(_)
                            | UiHit::NewTab
                            | UiHit::TabSearchButton
                            | UiHit::TabSearchItem(_)
                            | UiHit::Back
                            | UiHit::Forward
                            | UiHit::Reload
                            | UiHit::Home
                            | UiHit::SecurityBadge
                            | UiHit::ShieldsButton
                            | UiHit::SitePopoverItem(_)
                            | UiHit::ReaderModeButton
                            | UiHit::WindowMinimize
                            | UiHit::WindowMaximize
                            | UiHit::WindowClose
                            | UiHit::ClearAddress
                            | UiHit::ZoomPill
                            | UiHit::SettingsButton
                            | UiHit::DownloadsButton
                            | UiHit::DownloadsPopoverItem(_)
                            | UiHit::InspectorButton
                            | UiHit::AppMenuButton
                            | UiHit::AppMenuItem(_)
                            | UiHit::InspectorTab(_)
                            | UiHit::InspectorElementRow(_)
                            | UiHit::InspectorFullPage
                            | UiHit::InspectorClose
                            | UiHit::ToggleBookmark
                            | UiHit::Bookmark(_)
                            | UiHit::BookmarkOverflow
                            | UiHit::BookmarkOverflowMore
                            | UiHit::Scrollbar { .. }
                            | UiHit::FindCaseToggle
                            | UiHit::FindPrev
                            | UiHit::FindNext
                            | UiHit::FindClose
                            | UiHit::ContextMenuItem(_)
                            | UiHit::SelectDropdownOption(_)
                            | UiHit::OmniboxSuggestion(_),
                        ) => IDC_HAND,
                        // px and py are page coordinates: hit-test y minus the header. x is not shifted.
                        Some(UiHit::Page { x: px, y: py }) => {
                            if let Some(link) = host.session.hit_link(px, py) {
                                hovered = Some(link);
                                IDC_HAND
                            } else if host.session.hit_text_input(px, py).is_some() {
                                IDC_IBEAM
                            } else {
                                IDC_ARROW
                            }
                        }
                        _ => IDC_ARROW,
                    };
                    let ui_hover_active = y <= TOP_BAR_HEIGHT + 36.0
                        || matches!(
                            hit,
                            Some(UiHit::Scrollbar { .. } | UiHit::OmniboxSuggestion(_))
                        )
                        || (host.ui.is_editing_address()
                            && !host.ui.omnibox_suggestions(&host.session).is_empty())
                        || host.ui.select_dropdown().is_some()
                        || host.ui.context_menu_pos().is_some()
                        || host.ui.is_app_menu_open()
                        || host.ui.is_site_popover_open()
                        || host.ui.is_tab_search_open()
                        || host.ui.is_downloads_popover_open()
                        || host.ui.is_find_open()
                        || host.ui.is_inspector_open();
                    if host.ui.set_hovered_link(hovered) || (cursor_moved && ui_hover_active) {
                        refresh_chrome_only(hwnd, host);
                    }
                    unsafe {
                        SetCursor(LoadCursorW(null_mut(), cursor));
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    host.ui.stop_drag_select();
                    if host.ui.is_dragging_scrollbar() {
                        host.ui.stop_scrollbar_drag();
                        refresh_chrome_only(hwnd, host);
                    }
                }
                0
            }
            WM_MBUTTONDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let (x, y) = mouse_coords(l_param);
                    let hit = hit_test_ui_with_state_and_height(
                        &host.session,
                        &host.ui,
                        host.window_width as f32,
                        host.window_height as f32,
                        x,
                        y,
                    );
                    match hit {
                        Some(UiHit::Tab(index) | UiHit::CloseTab(index)) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            if host.session.close_tab(index) {
                                refresh_window(hwnd, host);
                            }
                        }
                        Some(UiHit::Page { x: px, y: py }) => {
                            if let Some(link) = host.session.hit_link(px, py) {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.session.new_tab();
                                host.start_navigation(hwnd, &link);
                                refresh_window(hwnd, host);
                            }
                        }
                        _ => {}
                    }
                }
                0
            }
            // Open the context menu at the client point. On the page, detect links or select text when nothing is selected yet.
            WM_RBUTTONDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    // Window pixels from lParam. Not page pixels.
                    let (x, y) = mouse_coords(l_param);
                    let mut context_link = None;
                    if let Some(UiHit::Page { x: px, y: py }) = hit_test_ui_with_state_and_height(
                        &host.session,
                        &host.ui,
                        host.window_width as f32,
                        host.window_height as f32,
                        x,
                        y,
                    ) {
                        let _ = host.session.inspect_at(px, py);
                        context_link = host.session.hit_link(px, py);
                        if context_link.is_none() && host.session.selected_text().is_empty() {
                            let _ = host.session.select_text_at(px, py);
                        }
                    }
                    host.ui.open_context_menu_with_link(x, y, context_link);
                    refresh_window(hwnd, host);
                }
                0
            }
            // Act on the hit. A click outside an open menu closes that menu.
            // The zoom pill calls Session::reset_zoom when the percent is not 100, otherwise zoom_in.
            // The settings button navigates to anvil://settings. It does not pause.
            // Page clicks pass page coordinates to Session::click_at.
            WM_LBUTTONDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    // Window pixels from lParam. Not page pixels.
                    let (x, y) = mouse_coords(l_param);
                    let hit = hit_test_ui_with_state_and_height(
                        &host.session,
                        &host.ui,
                        host.window_width as f32,
                        host.window_height as f32,
                        x,
                        y,
                    );
                    let mut changed = false;
                    let mut chrome_only = false;
                    // A click takes keyboard focus off the toolbar.
                    if host.ui.clear_chrome_focus() {
                        changed = true;
                    }
                    if host.ui.is_editing_console()
                        && !matches!(hit, Some(UiHit::InspectorConsoleInput))
                    {
                        host.ui.blur_console();
                        changed = true;
                    }
                    if host.ui.is_editing_style()
                        && !matches!(hit, Some(UiHit::InspectorStyleInput))
                    {
                        host.ui.blur_style();
                        changed = true;
                    }
                    if host.ui.select_dropdown().is_some()
                        && !matches!(hit, Some(UiHit::SelectDropdownOption(_)))
                    {
                        host.ui.close_select_dropdown();
                        changed = true;
                    }
                    if host.ui.is_bookmark_overflow_open()
                        && !matches!(
                            hit,
                            Some(
                                UiHit::Bookmark(_)
                                    | UiHit::BookmarkOverflow
                                    | UiHit::BookmarkOverflowMore
                            )
                        )
                    {
                        host.ui.close_bookmark_overflow();
                        changed = true;
                    }
                    if host.ui.context_menu_pos().is_some()
                        && !matches!(hit, Some(UiHit::ContextMenuItem(_)))
                    {
                        host.ui.close_context_menu();
                        changed = true;
                    }
                    if host.ui.is_app_menu_open()
                        && !matches!(hit, Some(UiHit::AppMenuButton | UiHit::AppMenuItem(_)))
                    {
                        host.ui.close_app_menu();
                        changed = true;
                    }
                    if host.ui.is_site_popover_open()
                        && !matches!(
                            hit,
                            Some(
                                UiHit::SecurityBadge
                                    | UiHit::ShieldsButton
                                    | UiHit::SitePopoverItem(_)
                            )
                        )
                    {
                        host.ui.close_site_popover();
                        changed = true;
                    }
                    if host.ui.is_tab_search_open()
                        && !matches!(hit, Some(UiHit::TabSearchButton | UiHit::TabSearchItem(_)))
                    {
                        host.ui.close_tab_search();
                        changed = true;
                    }
                    if host.ui.is_downloads_popover_open()
                        && !matches!(
                            hit,
                            Some(UiHit::DownloadsButton | UiHit::DownloadsPopoverItem(_))
                        )
                    {
                        host.ui.close_downloads_popover();
                        changed = true;
                    }
                    match hit {
                        Some(UiHit::TabSearchButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.toggle_tab_search();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::TabSearchItem(tab_idx)) => {
                            host.ui.close_tab_search();
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            let _ = host.session.select(tab_idx);
                            changed = true;
                        }
                        Some(UiHit::DownloadsPopoverItem(idx)) => {
                            let save_idx = host.session.downloads().entries().len().clamp(1, 4);
                            host.ui.close_downloads_popover();
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = true;
                            if idx == save_idx {
                                let _ = host.session.save_active_page_to_downloads();
                                let _ = host.session.save_default_profile();
                                chrome_only = true;
                            } else {
                                host.start_navigation(hwnd, DOWNLOADS_URL);
                            }
                        }
                        Some(UiHit::SitePopoverItem(action)) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = true;
                            match action {
                                SitePopoverAction::ToggleSiteShields => {
                                    let _ = host.session.toggle_shields_for_active_site();
                                    let _ = host.session.save_default_profile();
                                    chrome_only = true;
                                }
                                SitePopoverAction::CycleShieldsMode => {
                                    let _ = host.session.cycle_shields_mode();
                                    let _ = host.session.save_default_profile();
                                    chrome_only = true;
                                }
                                SitePopoverAction::ToggleSiteJavaScript => {
                                    let _ = host.session.toggle_scripts_for_active_site();
                                    let _ = host.session.save_default_profile();
                                    chrome_only = true;
                                }
                                SitePopoverAction::ToggleSiteCookies => {
                                    let _ = host.session.toggle_cookies_for_active_site();
                                    let _ = host.session.save_default_profile();
                                    chrome_only = true;
                                }
                                SitePopoverAction::OpenSecurityCenter => {
                                    host.ui.close_site_popover();
                                    host.start_navigation(hwnd, SECURITY_URL);
                                }
                            }
                        }
                        Some(UiHit::ReaderModeButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.session.toggle_reader_mode();
                        }
                        Some(UiHit::OmniboxSuggestion(idx)) => {
                            let target = host
                                .ui
                                .omnibox_suggestions(&host.session)
                                .get(idx)
                                .map(|s| s.url.clone());
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            if let Some(url) = target {
                                host.start_navigation(hwnd, &url);
                            }
                            changed = true;
                        }
                        Some(UiHit::InspectorConsoleInput) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.focus_console();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::InspectorStyleInput) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.focus_style();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::SelectDropdownOption(option_idx)) => {
                            let select_id = host.ui.select_dropdown().map(|d| d.node_id);
                            host.ui.close_select_dropdown();
                            if let Some(id) = select_id {
                                changed =
                                    host.session.choose_select_option(id, option_idx).is_some();
                            } else {
                                changed = true;
                            }
                        }
                        Some(UiHit::ContextMenuItem(action)) => {
                            let context_link = host.ui.context_menu_link().map(str::to_string);
                            host.ui.close_context_menu();
                            changed = true;
                            match action {
                                ContextMenuAction::OpenLinkInNewTab => {
                                    if let Some(link) = context_link {
                                        host.ui.blur_address();
                                        host.session.blur_page_input();
                                        host.session.new_tab();
                                        host.start_navigation(hwnd, &link);
                                    }
                                }
                                ContextMenuAction::CopyLinkAddress => {
                                    if let Some(link) = context_link {
                                        host.session.set_clipboard_text(&link);
                                        win32_set_clipboard_text(hwnd, &link);
                                    }
                                }
                                ContextMenuAction::Back => {
                                    let _ = host.history_back();
                                }
                                ContextMenuAction::Forward => {
                                    let _ = host.history_forward();
                                }
                                ContextMenuAction::Reload => {
                                    let url = host.current_url();
                                    if !url.is_empty() {
                                        host.start_navigation(hwnd, &url);
                                    }
                                }
                                ContextMenuAction::ReopenClosedTab => {
                                    let _ = host.session.reopen_closed_tab();
                                }
                                ContextMenuAction::DuplicateTab => {
                                    let active = host.session.active_index();
                                    let _ = host.session.duplicate_tab(active);
                                }
                                ContextMenuAction::TogglePinTab => {
                                    let active = host.session.active_index();
                                    let _ = host.session.toggle_pin_tab(active);
                                }
                                ContextMenuAction::CloseOtherTabs => {
                                    let active = host.session.active_index();
                                    let _ = host.session.close_other_tabs(active);
                                }
                                ContextMenuAction::Copy => {
                                    let _ = host.copy_to_clipboard(hwnd);
                                }
                                ContextMenuAction::Paste => {
                                    let _ = host.paste_from_clipboard(hwnd);
                                }
                                ContextMenuAction::FindInPage => {
                                    host.session.blur_page_input();
                                    host.ui.open_find();
                                }
                                ContextMenuAction::SavePage => {
                                    if host.session.save_active_page_to_downloads().is_some() {
                                        host.start_navigation(hwnd, DOWNLOADS_URL);
                                    }
                                }
                                ContextMenuAction::SavePdf => {
                                    if host.session.save_active_page_as_pdf().is_some() {
                                        host.start_navigation(hwnd, DOWNLOADS_URL);
                                    }
                                }
                                ContextMenuAction::ViewPageSource => {
                                    let _ = host.session.view_active_page_source();
                                }
                                ContextMenuAction::ToggleReaderMode => {
                                    let _ = host.session.toggle_reader_mode();
                                }
                                ContextMenuAction::ResetZoom => {
                                    let _ = host.session.reset_zoom();
                                }
                                ContextMenuAction::InspectPage => {
                                    host.ui.open_inspector();
                                }
                                ContextMenuAction::SecurityCenter => {
                                    host.start_navigation(hwnd, SECURITY_URL);
                                }
                            }
                        }
                        Some(UiHit::AppMenuButton) => {
                            host.session.blur_page_input();
                            host.ui.toggle_app_menu();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::AppMenuItem(action)) => {
                            host.ui.close_app_menu();
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = true;
                            match action {
                                AppMenuAction::NewTab => {
                                    host.session.new_tab();
                                    let _ = host.session.open_target(HOME_URL);
                                }
                                AppMenuAction::TabSearch => {
                                    host.ui.open_tab_search();
                                    chrome_only = true;
                                }
                                AppMenuAction::HomeDashboard => {
                                    host.start_navigation(hwnd, HOME_URL);
                                }
                                AppMenuAction::Downloads => {
                                    host.start_navigation(hwnd, DOWNLOADS_URL);
                                }
                                AppMenuAction::History => {
                                    host.start_navigation(hwnd, HISTORY_URL);
                                }
                                AppMenuAction::SecurityCenter => {
                                    host.start_navigation(hwnd, SECURITY_URL);
                                }
                                AppMenuAction::ToggleReaderMode => {
                                    let _ = host.session.toggle_reader_mode();
                                }
                                AppMenuAction::ToggleBookmarksBar => {
                                    host.ui.toggle_bookmarks_bar();
                                }
                                AppMenuAction::FindInPage => {
                                    host.ui.open_find();
                                    chrome_only = true;
                                }
                                AppMenuAction::ToggleInspectorDock => {
                                    host.ui.toggle_inspector();
                                }
                                AppMenuAction::OpenInspectorPage => {
                                    host.start_navigation(hwnd, INSPECT_URL);
                                }
                                AppMenuAction::CycleZoom => {
                                    if host.session.zoom_percent() != 100 {
                                        let _ = host.session.reset_zoom();
                                    } else {
                                        let _ = host.session.zoom_in();
                                    }
                                }
                                AppMenuAction::CycleTheme => {
                                    host.session.cycle_theme();
                                    let url = host.current_url();
                                    if url.starts_with("anvil://") {
                                        let _ = host.session.open_target(&url);
                                    }
                                }
                                AppMenuAction::Settings => {
                                    host.start_navigation(hwnd, SETTINGS_URL);
                                }
                            }
                        }
                        Some(UiHit::DownloadsButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.toggle_downloads_popover();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::InspectorButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.toggle_inspector();
                            changed = true;
                        }
                        Some(UiHit::InspectorTab(tab)) => {
                            host.ui.set_inspector_tab(tab);
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::InspectorElementRow(node_id)) => {
                            host.session.select_inspected_node(node_id);
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::InspectorFullPage) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.start_navigation(hwnd, INSPECT_URL);
                            changed = true;
                        }
                        Some(UiHit::InspectorClose) => {
                            host.ui.close_inspector();
                            changed = true;
                        }
                        Some(UiHit::FindInput) => {
                            host.session.blur_page_input();
                            host.ui.open_find();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::FindCaseToggle) => {
                            host.ui.toggle_find_case_sensitive();
                            host.scroll_to_active_find_match();
                            changed = true;
                        }
                        Some(UiHit::FindPrev) => {
                            let count = host.find_match_count();
                            host.ui.find_prev(count);
                            host.scroll_to_active_find_match();
                            changed = true;
                        }
                        Some(UiHit::FindNext) => {
                            let count = host.find_match_count();
                            host.ui.find_next(count);
                            host.scroll_to_active_find_match();
                            changed = true;
                        }
                        Some(UiHit::FindClose) => {
                            host.ui.close_find();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::Tab(index)) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.session.select(index);
                        }
                        Some(UiHit::CloseTab(index)) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.session.close_tab(index);
                        }
                        Some(UiHit::NewTab) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.session.new_tab();
                            let _ = host.session.open_target(HOME_URL);
                            changed = true;
                        }
                        Some(UiHit::Back) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.history_back();
                        }
                        Some(UiHit::Forward) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.history_forward();
                        }
                        Some(UiHit::Reload) => {
                            if host.ui.is_loading() {
                                host.stop_loading();
                                changed = true;
                            } else {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                let url = host.current_url();
                                if !url.is_empty() {
                                    host.start_navigation(hwnd, &url);
                                    changed = true;
                                }
                            }
                        }
                        Some(UiHit::Home) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.start_navigation(hwnd, HOME_URL);
                            changed = true;
                        }
                        Some(UiHit::WindowMinimize) => {
                            cpp_ui_window_action(hwnd as isize, 1);
                        }
                        Some(UiHit::WindowMaximize) => {
                            cpp_ui_window_action(hwnd as isize, 2);
                        }
                        Some(UiHit::WindowClose) => {
                            cpp_ui_window_action(hwnd as isize, 3);
                        }
                        Some(UiHit::TitlebarDrag) => {
                            cpp_ui_window_action(hwnd as isize, 4);
                        }
                        Some(UiHit::SecurityBadge | UiHit::ShieldsButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.ui.toggle_site_popover();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::AddressField) => {
                            host.session.blur_page_input();
                            let url = host.current_url();
                            host.ui.focus_address(&url);
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::ClearAddress) => {
                            host.ui.clear_address();
                            changed = true;
                            chrome_only = true;
                        }
                        Some(UiHit::ToggleBookmark) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.session.toggle_bookmark_active();
                            let _ = host.session.save_default_profile();
                            changed = true;
                        }
                        Some(UiHit::ZoomPill) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            if host.session.zoom_percent() != 100 {
                                let _ = host.session.reset_zoom();
                            } else {
                                let _ = host.session.zoom_in();
                            }
                            changed = true;
                        }
                        Some(UiHit::SettingsButton) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.start_navigation(hwnd, SETTINGS_URL);
                            changed = true;
                        }
                        Some(UiHit::BookmarkOverflow) => {
                            host.ui.toggle_bookmark_overflow();
                            changed = true;
                        }
                        Some(UiHit::BookmarkOverflowMore) => {
                            host.ui.next_bookmark_overflow(
                                host.session.bookmarks().len(),
                                crate::ui::bookmark_bar_visible_count(
                                    &host.session,
                                    host.window_width as f32,
                                ),
                            );
                            changed = true;
                        }
                        Some(UiHit::Bookmark(index)) => {
                            host.ui.close_bookmark_overflow();
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            if let Some(bm) = host.session.bookmarks().get(index).cloned() {
                                host.start_navigation(hwnd, &bm.url);
                                changed = true;
                            }
                        }
                        Some(UiHit::Scrollbar { ratio }) => {
                            host.ui.blur_address();
                            host.ui.start_scrollbar_drag();
                            let before = host.session.scroll();
                            let after = host.session.scroll_to_ratio(ratio);
                            changed = (after - before).abs() > f32::EPSILON;
                        }
                        // Page click. px and py are already in page space, not window space.
                        Some(UiHit::Page { x: px, y: py }) => {
                            if host.ui.is_editing_address() {
                                host.ui.blur_address();
                            }
                            match host.session.click_at(px, py) {
                                ClickOutcome::SelectChanged(node_id, _) => {
                                    if let Some((rect, options)) =
                                        host.session.select_dropdown_info(node_id)
                                        && options.len() > 1
                                    {
                                        host.ui.open_select_dropdown(node_id, rect, options);
                                    }
                                    changed = true;
                                }
                                ClickOutcome::ScriptHandled
                                | ClickOutcome::InputFocused(_)
                                | ClickOutcome::CheckboxToggled(..)
                                | ClickOutcome::VideoToggled(..)
                                | ClickOutcome::VideoSeeked(..) => {
                                    changed = true;
                                }
                                ClickOutcome::Navigate(target) => {
                                    host.start_navigation(hwnd, &target);
                                    changed = true;
                                }
                                ClickOutcome::SubmitPost { url, body } => {
                                    host.start_post_navigation(hwnd, &url, &body);
                                    changed = true;
                                }
                                ClickOutcome::None => {
                                    host.session.clear_selection();
                                    match host.ui.record_page_click(px, py) {
                                        2 => {
                                            let _ = host.session.select_word_at(px, py);
                                        }
                                        3 => {
                                            let _ = host.session.select_line_at(px, py);
                                        }
                                        _ => {
                                            let _ = host.session.select_text_at(px, py);
                                            host.ui.start_drag_select(px, py);
                                        }
                                    }
                                    let _ = host.session.inspect_at(px, py);
                                    changed = true;
                                }
                            }
                        }
                        None => {
                            if host.ui.is_editing_address() {
                                host.ui.blur_address();
                                changed = true;
                                chrome_only = true;
                            }
                            if host.session.blur_page_input() {
                                changed = true;
                                chrome_only = false;
                            }
                        }
                    }
                    if changed {
                        if chrome_only {
                            refresh_chrome_only(hwnd, host);
                        } else {
                            refresh_window(hwnd, host);
                        }
                    }
                }
                0
            }
            // Wheel delta is the high word of wParam. Control zooms. Otherwise the page scrolls 48 pixels per notch.
            WM_MOUSEWHEEL => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let ctrl_down = unsafe { (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0 };
                    let delta = ((w_param >> 16) as i16) as f32;
                    // Control changes the wheel from scroll to zoom. Steps are 25 percent inside Session.
                    if ctrl_down {
                        let before = host.session.zoom_percent();
                        let after = if delta > 0.0 {
                            host.session.zoom_in()
                        } else if delta < 0.0 {
                            host.session.zoom_out()
                        } else {
                            before
                        };
                        if after != before {
                            refresh_window(hwnd, host);
                        }
                    } else {
                        let step = -(delta / 120.0) * 48.0;
                        let before_scroll = host.session.scroll();
                        let scrolled = if let Some((cx, cy)) = host.ui.cursor_pos()
                            && let Some(UiHit::Page { x: px, y: py }) =
                                hit_test_ui_with_state_and_height(
                                    &host.session,
                                    &host.ui,
                                    host.window_width as f32,
                                    host.window_height as f32,
                                    cx,
                                    cy,
                                ) {
                            host.session.scroll_at(px, py, step)
                        } else {
                            let after = host.session.scroll_by(step);
                            (after - before_scroll).abs() > f32::EPSILON
                        };
                        if scrolled {
                            host.redraw_after_scroll(before_scroll);
                            unsafe {
                                InvalidateRect(hwnd, null(), 0);
                            }
                        }
                    }
                }
                0
            }
            // Printable ASCII goes to the address bar, else tab search, else the console/style input, else the find bar, else the focused page field.
            WM_CHAR => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    if host.ui.is_editing_address()
                        || host.ui.is_tab_search_open()
                        || host.ui.is_editing_console()
                        || host.ui.is_editing_style()
                    {
                        if let Some(ch) = char::from_u32(w_param as u32)
                            && (' '..='~').contains(&ch)
                        {
                            host.ui.push_char(ch);
                            refresh_chrome_only(hwnd, host);
                        }
                    } else if host.ui.is_find_open() {
                        if let Some(ch) = char::from_u32(w_param as u32)
                            && (' '..='~').contains(&ch)
                        {
                            host.ui.push_char(ch);
                            host.scroll_to_active_find_match();
                            refresh_window(hwnd, host);
                        }
                    } else if host.session.has_focused_input()
                        && let Some(ch) = char::from_u32(w_param as u32)
                        && host.session.type_into_focused_input(ch)
                    {
                        refresh_window(hwnd, host);
                    }
                }
                0
            }
            // Key bindings. F9 is not bound. There is no pause key.
            // Control+T new tab. Control+W close tab. Control+L or F6 focus the address bar.
            // Control+F find. Control+H history. Control+J downloads. Control+R or F5 reload.
            // Control+C copy. Control+X cut. Control+V paste.
            // Control+Plus, Control+numpad plus zoom in. Control+Minus zooms out. Control+0 resets zoom.
            // Alt+Left back. Alt+Right forward. Alt+Home opens the start page.
            // In the address bar: Escape blurs, Tab accepts the suggestion, Backspace deletes, Enter navigates.
            // In find: Escape closes it, Backspace edits, Enter goes to the next match.
            // In a page field: Escape blurs, Backspace deletes, Enter submits.
            // Escape with no field closes the context menu, the app menu, find, then the inspector.
            // A further Escape destroys the window.
            // F12 toggles the inspector. Home and End jump scroll. Up and Down scroll 40 pixels.
            // Page Up and Page Down scroll 80 percent of the page viewport. Backspace goes back.
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let shift_down = unsafe { (GetKeyState(VK_SHIFT) as u16 & 0x8000) != 0 };
                    let ctrl_down = unsafe { (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0 };
                    let alt_down = unsafe { (GetKeyState(VK_MENU) as u16 & 0x8000) != 0 };

                    // Toolbar focus. F10 or Alt+Shift+T puts it on the toolbar. While it is
                    // there, Tab, the arrows, Home, End, Enter, Space, and Escape go through
                    // crate::keys, the same as on Linux. Control chords still work below.
                    if !ctrl_down {
                        let toolbar_key = match w_param {
                            VK_F10 => Some(Key::F(10)),
                            VK_KEY_T if alt_down && shift_down => Some(Key::Char('t')),
                            _ if host.ui.chrome_focus().is_some() => match w_param {
                                VK_TAB => Some(Key::Tab),
                                VK_LEFT => Some(Key::Left),
                                VK_RIGHT => Some(Key::Right),
                                VK_HOME => Some(Key::Home),
                                VK_END => Some(Key::End),
                                VK_RETURN => Some(Key::Enter),
                                VK_SPACE => Some(Key::Char(' ')),
                                VK_ESCAPE => Some(Key::Escape),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(key) = toolbar_key {
                            let mods = Modifiers {
                                shift: shift_down,
                                ctrl: false,
                                alt: alt_down,
                            };
                            let focus = focus_of(&host.session, &host.ui);
                            let command = command_for(key, mods, focus);
                            match apply_key_command(&mut host.session, &mut host.ui, command) {
                                KeyEffect::Nothing => {}
                                KeyEffect::Redraw | KeyEffect::CloseWindow => {
                                    refresh_window(hwnd, host)
                                }
                                KeyEffect::StopLoading => {
                                    host.stop_loading();
                                    refresh_window(hwnd, host);
                                }
                                KeyEffect::Navigate(target) => {
                                    host.start_navigation(hwnd, &target);
                                    refresh_window(hwnd, host);
                                }
                                KeyEffect::Post { url, body } => {
                                    host.start_post_navigation(hwnd, &url, &body);
                                    refresh_window(hwnd, host);
                                }
                            }
                            return 0;
                        }
                    }

                    // Escape while a page is loading abandons that load (same as the
                    // Stop toolbar button). Skip when an overlay still wants Escape.
                    // Runs after the toolbar-focus block so Escape on the toolbar still
                    // leaves focus through crate::keys first.
                    if w_param == VK_ESCAPE
                        && host.ui.is_loading()
                        && !host.ui.is_find_open()
                        && !host.ui.is_tab_search_open()
                        && !host.ui.is_app_menu_open()
                        && !host.ui.is_bookmark_overflow_open()
                        && !host.ui.is_site_popover_open()
                        && host.ui.context_menu_pos().is_none()
                    {
                        host.stop_loading();
                        host.redraw();
                        // Win32: mark the whole client dirty so WM_PAINT redraws the
                        // nav bar. Reload becomes Stop (and back) only in compose; without
                        // this InvalidateRect the glyph can lag until the next unrelated
                        // paint. hwnd is the same window handle this callback already
                        // owns; lpRect null and bErase 0 match every other InvalidateRect
                        // call in this file (full client, no background erase).
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                        return 0;
                    }

                    // Control chords. Letters without Control fall through to the later match.
                    if ctrl_down {
                        let changed = match w_param {
                            VK_KEY_T => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if shift_down {
                                    host.session.reopen_closed_tab()
                                } else {
                                    host.session.new_tab();
                                    let _ = host.session.open_target(HOME_URL);
                                    true
                                }
                            }
                            VK_KEY_W => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                let active = host.session.active_index();
                                host.session.close_tab(active)
                            }
                            VK_TAB => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if shift_down {
                                    host.session.prev_tab()
                                } else {
                                    host.session.next_tab()
                                }
                            }
                            VK_PRIOR => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if shift_down {
                                    host.session.move_active_tab_left()
                                } else {
                                    host.session.prev_tab()
                                }
                            }
                            VK_NEXT => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if shift_down {
                                    host.session.move_active_tab_right()
                                } else {
                                    host.session.next_tab()
                                }
                            }
                            VK_KEY_1..=VK_KEY_9 => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.session
                                    .select_tab_shortcut((w_param - VK_KEY_1 + 1) as u8)
                            }
                            VK_KEY_U => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.session.view_active_page_source().is_some()
                            }
                            VK_KEY_B if shift_down => {
                                host.ui.toggle_bookmarks_bar();
                                true
                            }
                            VK_KEY_S => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if host.session.save_active_page_to_downloads().is_some() {
                                    host.start_navigation(hwnd, DOWNLOADS_URL);
                                    true
                                } else {
                                    false
                                }
                            }
                            VK_KEY_P => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                if host.session.save_active_page_as_pdf().is_some() {
                                    host.start_navigation(hwnd, DOWNLOADS_URL);
                                    true
                                } else {
                                    false
                                }
                            }
                            VK_KEY_A => {
                                if shift_down {
                                    host.session.blur_page_input();
                                    host.ui.toggle_tab_search();
                                    true
                                } else if host.ui.is_editing_address()
                                    || host.ui.is_editing_console()
                                {
                                    host.ui.select_all_input()
                                } else {
                                    false
                                }
                            }
                            VK_BACK => {
                                if host.ui.is_editing_address()
                                    || host.ui.is_tab_search_open()
                                    || host.ui.is_editing_console()
                                    || host.ui.is_editing_style()
                                {
                                    host.ui.delete_word_back()
                                } else if host.ui.is_find_open()
                                    && !host.session.has_focused_input()
                                {
                                    let deleted = host.ui.delete_word_back();
                                    if deleted {
                                        host.scroll_to_active_find_match();
                                    }
                                    deleted
                                } else if host.session.has_focused_input() {
                                    host.session.delete_word_focused_input()
                                } else {
                                    false
                                }
                            }
                            VK_KEY_L => {
                                host.session.blur_page_input();
                                let url = host.current_url();
                                host.ui.focus_address(&url);
                                true
                            }
                            VK_KEY_F => {
                                host.session.blur_page_input();
                                host.ui.open_find();
                                true
                            }
                            VK_KEY_H => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.start_navigation(hwnd, HISTORY_URL);
                                true
                            }
                            VK_KEY_J => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.start_navigation(hwnd, DOWNLOADS_URL);
                                true
                            }
                            VK_KEY_C => host.copy_to_clipboard(hwnd),
                            VK_KEY_X => host.cut_to_clipboard(hwnd),
                            VK_KEY_V => host.paste_from_clipboard(hwnd),
                            VK_OEM_PLUS | VK_ADD => {
                                let before = host.session.zoom_percent();
                                host.session.zoom_in() != before
                            }
                            VK_OEM_MINUS | VK_SUBTRACT => {
                                let before = host.session.zoom_percent();
                                host.session.zoom_out() != before
                            }
                            VK_KEY_0 | VK_NUMPAD0 => {
                                let before = host.session.zoom_percent();
                                host.session.reset_zoom() != before
                            }
                            VK_KEY_R => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                let url = host.current_url();
                                if !url.is_empty() {
                                    host.start_navigation(hwnd, &url);
                                    true
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        if changed {
                            refresh_window(hwnd, host);
                            return 0;
                        }
                    }

                    // Alt+Left back, Alt+Right forward, Alt+Home start page, Alt+R reader mode. Other Alt keys are ignored here.
                    if alt_down {
                        let changed = match w_param {
                            VK_LEFT => host.history_back(),
                            VK_RIGHT => host.history_forward(),
                            VK_KEY_R => host.session.toggle_reader_mode(),
                            VK_HOME => {
                                host.start_navigation(hwnd, HOME_URL);
                                true
                            }
                            _ => false,
                        };
                        if changed {
                            refresh_window(hwnd, host);
                            return 0;
                        }
                    }

                    if host.ui.is_editing_address() {
                        match w_param {
                            // Escape while the address bar is focused. Blur only. Do not close the window.
                            VK_ESCAPE => {
                                host.ui.blur_address();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_TAB => {
                                if host.ui.accept_autocomplete(&host.session) {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_UP => {
                                if host.ui.move_omnibox_selection_up(&host.session) {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_DOWN => {
                                if host.ui.move_omnibox_selection_down(&host.session) {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_BACK => {
                                host.ui.backspace();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_DELETE => {
                                if host.ui.delete_forward() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_LEFT => {
                                if host.ui.move_cursor_left() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RIGHT => {
                                if host.ui.move_cursor_right() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_HOME => {
                                if host.ui.move_cursor_home() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_END => {
                                if host.ui.move_cursor_end() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RETURN => {
                                if let Some(url) = host.ui.selected_omnibox_url(&host.session) {
                                    host.ui.set_input(&url);
                                }
                                let engine = host.session.search_engine();
                                if let Some(target) = host.ui.submit_address_with_engine(engine) {
                                    host.start_navigation(hwnd, &target);
                                }
                                refresh_window(hwnd, host);
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    if host.ui.is_tab_search_open() {
                        match w_param {
                            VK_ESCAPE => {
                                host.ui.close_tab_search();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_UP => {
                                if host.ui.move_tab_search_up(&host.session) {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_DOWN => {
                                if host.ui.move_tab_search_down(&host.session) {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_BACK => {
                                host.ui.backspace();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_RETURN => {
                                if let Some(idx) = host.ui.selected_tab_search_index(&host.session)
                                {
                                    let _ = host.session.select(idx);
                                }
                                host.ui.close_tab_search();
                                refresh_window(hwnd, host);
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    if host.ui.is_editing_console() || host.ui.is_editing_style() {
                        match w_param {
                            VK_ESCAPE => {
                                host.ui.blur_console();
                                host.ui.blur_style();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_BACK => {
                                host.ui.backspace();
                                refresh_chrome_only(hwnd, host);
                            }
                            VK_DELETE => {
                                if host.ui.delete_forward() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_LEFT => {
                                if host.ui.move_cursor_left() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RIGHT => {
                                if host.ui.move_cursor_right() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_HOME => {
                                if host.ui.move_cursor_home() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_END => {
                                if host.ui.move_cursor_end() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RETURN => {
                                if host.ui.is_editing_console() {
                                    if let Some(expr) = host.ui.take_console_input() {
                                        let _ = host.session.eval_console_js(&expr);
                                        refresh_window(hwnd, host);
                                    }
                                } else if let Some(decl) = host.ui.take_style_input() {
                                    let _ = host.session.apply_inspected_style_override(&decl);
                                    refresh_window(hwnd, host);
                                }
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    if host.ui.is_find_open() && !host.session.has_focused_input() {
                        match w_param {
                            // Escape while find is open. Close find.
                            VK_ESCAPE => {
                                host.ui.close_find();
                                refresh_window(hwnd, host);
                            }
                            VK_BACK => {
                                host.ui.backspace();
                                host.scroll_to_active_find_match();
                                refresh_window(hwnd, host);
                            }
                            VK_DELETE => {
                                if host.ui.delete_forward() {
                                    host.scroll_to_active_find_match();
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_LEFT => {
                                if host.ui.move_cursor_left() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RIGHT => {
                                if host.ui.move_cursor_right() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_HOME => {
                                if host.ui.move_cursor_home() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_END => {
                                if host.ui.move_cursor_end() {
                                    refresh_chrome_only(hwnd, host);
                                }
                            }
                            VK_RETURN => {
                                let count = host.find_match_count();
                                host.ui.find_next(count);
                                host.scroll_to_active_find_match();
                                refresh_window(hwnd, host);
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    if host.session.has_focused_input() {
                        match w_param {
                            // Escape while a page field is focused. Blur that field.
                            VK_ESCAPE => {
                                if host.session.blur_page_input() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_TAB => {
                                if host.session.focus_next_input(shift_down) {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_BACK => {
                                if host.session.backspace_focused_input() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_DELETE => {
                                if host.session.delete_focused_input() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_LEFT => {
                                if host.session.move_focused_input_cursor_left() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_RIGHT => {
                                if host.session.move_focused_input_cursor_right() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_HOME => {
                                if host.session.move_focused_input_cursor_home() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_END => {
                                if host.session.move_focused_input_cursor_end() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_RETURN => {
                                match host.session.submit_focused_input() {
                                    ClickOutcome::Navigate(target) => {
                                        host.start_navigation(hwnd, &target);
                                    }
                                    ClickOutcome::SubmitPost { url, body } => {
                                        host.start_post_navigation(hwnd, &url, &body);
                                    }
                                    _ => {}
                                }
                                refresh_window(hwnd, host);
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    // Escape with nothing focused. Close overlays in order, then destroy the window.
                    if w_param == VK_ESCAPE {
                        if host.ui.select_dropdown().is_some() {
                            host.ui.close_select_dropdown();
                            refresh_chrome_only(hwnd, host);
                            return 0;
                        }
                        if host.ui.context_menu_pos().is_some() {
                            host.ui.close_context_menu();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_tab_search_open() {
                            host.ui.close_tab_search();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_bookmark_overflow_open() {
                            host.ui.close_bookmark_overflow();
                            host.redraw();
                            return 0;
                        }
                        if host.ui.is_downloads_popover_open() {
                            host.ui.close_downloads_popover();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_app_menu_open() {
                            host.ui.close_app_menu();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_site_popover_open() {
                            host.ui.close_site_popover();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_find_open() {
                            host.ui.close_find();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        if host.ui.is_inspector_open() {
                            host.ui.close_inspector();
                            refresh_window(hwnd, host);
                            return 0;
                        }
                        unsafe {
                            DestroyWindow(hwnd);
                        }
                        return 0;
                    }

                    let changed = match w_param {
                        // F12 toggles the inspector dock. Does not navigate.
                        VK_F12 => {
                            host.ui.toggle_inspector();
                            true
                        }
                        // F6 focuses the address bar with the current URL.
                        VK_F6 => {
                            let url = host.current_url();
                            host.ui.focus_address(&url);
                            true
                        }
                        // F5 reloads the active URL through start_navigation.
                        VK_F5 => {
                            let url = host.current_url();
                            if !url.is_empty() {
                                host.start_navigation(hwnd, &url);
                                true
                            } else {
                                false
                            }
                        }
                        VK_BACK => host.history_back(),
                        VK_TAB => host.session.focus_next_input(shift_down),
                        VK_HOME => {
                            let before = host.session.scroll();
                            (host.session.scroll_to_ratio(0.0) - before).abs() > f32::EPSILON
                        }
                        VK_END => {
                            let before = host.session.scroll();
                            (host.session.scroll_to_ratio(1.0) - before).abs() > f32::EPSILON
                        }
                        VK_UP => {
                            let before = host.session.scroll();
                            (host.session.scroll_by(-40.0) - before).abs() > f32::EPSILON
                        }
                        VK_DOWN => {
                            let before = host.session.scroll();
                            (host.session.scroll_by(40.0) - before).abs() > f32::EPSILON
                        }
                        VK_PRIOR => {
                            let step = -host.session.viewport().height * 0.8;
                            let before = host.session.scroll();
                            (host.session.scroll_by(step) - before).abs() > f32::EPSILON
                        }
                        VK_NEXT => {
                            let step = host.session.viewport().height * 0.8;
                            let before = host.session.scroll();
                            (host.session.scroll_by(step) - before).abs() > f32::EPSILON
                        }
                        _ => false,
                    };
                    if changed {
                        refresh_window(hwnd, host);
                    }
                }
                0
            }
            // Present the CPU framebuffer. Direct3D 11 uploads it when the
            // device is available. Otherwise SetDIBitsToDevice copies it.
            WM_PAINT => {
                let mut ps = PaintStruct {
                    hdc: null_mut(),
                    f_erase: 0,
                    rc_paint: WinRect::default(),
                    f_restore: 0,
                    f_inc_update: 0,
                    rgb_reserved: [0; 32],
                };
                let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let width = host.framebuffer.width();
                    let height = host.framebuffer.height();
                    let presented = if let Some((y0, y1)) = host.dirty_rows.take() {
                        host.present.paint_dirty_rows(
                            hwnd,
                            host.framebuffer.pixels(),
                            width,
                            height,
                            y0,
                            y1,
                        )
                    } else {
                        host.present
                            .paint(hwnd, host.framebuffer.pixels(), width, height)
                    };
                    if !presented {
                        let bgra = host.framebuffer.to_bgra_bottom_up();
                        let info = BitmapInfo {
                            bmi_header: BitmapInfoHeader {
                                bi_size: size_of::<BitmapInfoHeader>() as u32,
                                bi_width: width as i32,
                                bi_height: height as i32,
                                bi_planes: 1,
                                bi_bit_count: 32,
                                bi_compression: BI_RGB,
                                bi_size_image: 0,
                                bi_x_pels_per_meter: 0,
                                bi_y_pels_per_meter: 0,
                                bi_clr_used: 0,
                                bi_clr_important: 0,
                            },
                            bmi_colors: [0],
                        };
                        unsafe {
                            SetDIBitsToDevice(
                                hdc,
                                0,
                                0,
                                width,
                                height,
                                0,
                                0,
                                0,
                                height,
                                bgra.as_ptr().cast::<c_void>(),
                                &info,
                                DIB_RGB_COLORS,
                            );
                        }
                    }
                }
                unsafe {
                    EndPaint(hwnd, &ps);
                }
                // Every input and page load ends in a repaint, so this is
                // where a newly started transition or animation is noticed.
                if let Some(host) = unsafe { host_mut(hwnd) }
                    && !host.animation_timer
                    && host.session.is_animating()
                {
                    host.animating = true;
                    // hwnd is this live window and no callback is passed,
                    // so the timer only posts WM_TIMER here.
                    let id = unsafe {
                        SetTimer(
                            hwnd,
                            ANIMATION_TIMER_ID,
                            super::ANIMATION_FRAME.as_millis() as u32,
                            null(),
                        )
                    };
                    host.animation_timer = id != 0;
                }
                0
            }
            // Animation frame: tick the page clock, restyle, and repaint.
            // The timer stops once nothing is running.
            WM_TIMER if w_param == ANIMATION_TIMER_ID => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    if host.animation_step() {
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                    if !host.animating && host.animation_timer {
                        // Same window and id that SetTimer started.
                        unsafe {
                            KillTimer(hwnd, ANIMATION_TIMER_ID);
                        }
                        host.animation_timer = false;
                    }
                }
                0
            }
            // Save the profile and post quit so the GetMessageW loop stops.
            WM_DESTROY => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let _ = host.session.save_default_profile();
                }
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            // Messages not listed above, including most non-client traffic.
            _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
        }
    }

    /// Rasterize, update the caption, sync DWM dark mode, and invalidate so WM_PAINT copies the new buffer.
    fn refresh_window(hwnd: Hwnd, host: &mut Host) {
        host.redraw();
        let dark_mode: i32 = i32::from(!host.session.theme().is_light());
        let title = wide(&host.window_title());
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark_mode).cast::<c_void>(),
                size_of::<i32>() as u32,
            );
            SetWindowTextW(hwnd, title.as_ptr());
            InvalidateRect(hwnd, null(), 0);
        }
    }

    /// Re-compose only the browser chrome and overlays from the cached page framebuffer.
    fn refresh_chrome_only(hwnd: Hwnd, host: &mut Host) {
        host.redraw_chrome_only();
        unsafe {
            InvalidateRect(hwnd, null(), 0);
        }
    }

    /// Borrow the Host stored on the HWND. None before WM_NCCREATE and after WM_NCDESTROY.
    unsafe fn host_mut<'a>(hwnd: Hwnd) -> Option<&'a mut Host> {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *ptr })
        }
    }

    /// Client position packed in lParam.
    /// The low 16 bits are x and the high 16 bits are y. Both are sign-extended.
    /// These are window pixels. The hit test converts y into page space.
    fn mouse_coords(l_param: Lparam) -> (f32, f32) {
        let x = (l_param & 0xffff) as i16 as f32;
        let y = ((l_param >> 16) & 0xffff) as i16 as f32;
        (x, y)
    }

    /// UTF-16 plus a trailing nul for Win32 string arguments.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

// X11 event loop. Pixels reach the window through XPutImage.
// There is no GPU swapchain. F12, F9, and Control chords are not handled here.
#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{
        AppMenuAction, ClickOutcome, ContextMenuAction, DOWNLOADS_URL, HISTORY_URL, HOME_URL, Host,
        INSPECT_URL, SECURITY_URL, SETTINGS_URL, Session, SitePopoverAction, TOP_BAR_HEIGHT, UiHit,
        UiState, Viewport, compose_frame_with_page_fb, content_viewport, content_viewport_for,
        hit_test_ui_with_state_and_height, load_post_in_tab, load_target_in_tab,
        navigation_error_page, top_header_height_for,
    };

    use crate::keys::{
        KeyEffect, Modifiers, apply_key_command, command_for, focus_of, key_from_x11,
    };

    /// Opaque X11 Display. Opened with XOpenDisplay and closed at the end of the loop.
    type Display = c_void;
    /// X11 window id.
    type Window = c_ulong;
    /// X11 graphics context. The default GC of the screen is used for XPutImage.
    type GC = *mut c_void;
    /// X11 visual used to create the ZPixmap.
    type Visual = c_void;
    /// X11 atom. WM_DELETE_WINDOW is interned so the close button can end the loop.
    type Atom = c_ulong;
    /// Keysym from XLookupString. Decoded by crate::keys.
    type KeySym = c_ulong;

    /// Ask X11 for Expose events so damaged regions can be copied again.
    const EXPOSURE_MASK: c_long = 1 << 15;
    /// Ask X11 for key presses.
    const KEY_PRESS_MASK: c_long = 1 << 0;
    /// Ask X11 for mouse button presses, including wheel buttons 4 and 5.
    const BUTTON_PRESS_MASK: c_long = 1 << 2;
    /// Ask X11 for pointer motion.
    const POINTER_MOTION_MASK: c_long = 1 << 6;
    /// Ask X11 for ConfigureNotify so resizes update the viewport.
    const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

    /// XEvent type for a key press.
    const KEY_PRESS: c_int = 2;
    /// XEvent type for a button press. Button 1 is left, 3 is right, 4 and 5 scroll.
    const BUTTON_PRESS: c_int = 4;
    /// XEvent type for pointer motion. x and y are client pixels.
    const MOTION_NOTIFY: c_int = 6;
    /// XEvent type. The handler copies the CPU framebuffer again.
    const EXPOSE: c_int = 12;
    /// XEvent type. The window size changed.
    const CONFIGURE_NOTIFY: c_int = 22;
    /// XEvent type. WM_DELETE_WINDOW arrives this way and ends the loop.
    const CLIENT_MESSAGE: c_int = 33;

    /// XImage format used by the present path. The pixels are the CPU buffer.
    const Z_PIXMAP: c_int = 2;

    /// XImage header. data points at the CPU buffer only until present returns.
    #[repr(C)]
    struct XImage {
        width: c_int,
        height: c_int,
        xoffset: c_int,
        format: c_int,
        data: *mut c_char,
        byte_order: c_int,
        bitmap_unit: c_int,
        bitmap_bit_order: c_int,
        bitmap_pad: c_int,
        depth: c_int,
        bytes_per_line: c_int,
        bits_per_pixel: c_int,
        red_mask: c_ulong,
        green_mask: c_ulong,
        blue_mask: c_ulong,
        obdata: *mut c_char,
        f_pad: [*mut c_void; 6],
    }

    /// XKeyEvent view of an XEvent. state holds the Shift, Control, and Alt bits.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XKeyEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: Window,
        root: Window,
        subwindow: Window,
        time: c_ulong,
        x: c_int,
        y: c_int,
        x_root: c_int,
        y_root: c_int,
        state: c_uint,
        keycode: c_uint,
        same_screen: c_int,
    }

    /// XButtonEvent view of an XEvent. button selects click or wheel.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XButtonEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: Window,
        root: Window,
        subwindow: Window,
        time: c_ulong,
        x: c_int,
        y: c_int,
        x_root: c_int,
        y_root: c_int,
        state: c_uint,
        button: c_uint,
        same_screen: c_int,
    }

    /// XMotionEvent view of an XEvent. x and y are client pixels.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XMotionEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: Window,
        root: Window,
        subwindow: Window,
        time: c_ulong,
        x: c_int,
        y: c_int,
        x_root: c_int,
        y_root: c_int,
        state: c_uint,
        is_hint: c_char,
        same_screen: c_int,
    }

    /// XConfigureEvent view of an XEvent. width and height are the new client size.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XConfigureEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        event: Window,
        window: Window,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
        border_width: c_int,
        above: Window,
        override_redirect: c_int,
    }

    /// XClientMessageEvent view of an XEvent. data_l[0] is the protocol atom.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct XClientMessageEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut Display,
        window: Window,
        message_type: Atom,
        format: c_int,
        data_l: [c_long; 5],
    }

    /// XEvent union. Only the fields this loop reads are declared.
    #[repr(C)]
    union XEvent {
        type_: c_int,
        key: XKeyEvent,
        button: XButtonEvent,
        motion: XMotionEvent,
        configure: XConfigureEvent,
        client_message: XClientMessageEvent,
        pad: [c_long; 24],
    }

    #[link(name = "X11")]
    unsafe extern "C" {
        /// Connect to the X server. A null display ends open with an error.
        fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
        /// Close the display after the window is destroyed.
        fn XCloseDisplay(display: *mut Display) -> c_int;
        /// Screen number used for the root window, visual, and default GC.
        fn XDefaultScreen(display: *mut Display) -> c_int;
        /// Parent window for the browser window.
        fn XRootWindow(display: *mut Display, screen_number: c_int) -> Window;
        /// Visual passed to XCreateImage.
        fn XDefaultVisual(display: *mut Display, screen_number: c_int) -> *mut Visual;
        /// Depth passed to XCreateImage.
        fn XDefaultDepth(display: *mut Display, screen_number: c_int) -> c_int;
        /// GC passed to XPutImage.
        fn XDefaultGC(display: *mut Display, screen_number: c_int) -> GC;
        /// Border and background pixel for the simple window.
        fn XBlackPixel(display: *mut Display, screen_number: c_int) -> c_ulong;
        /// Create the window at the requested size. Position is the origin of the parent.
        fn XCreateSimpleWindow(
            display: *mut Display,
            parent: Window,
            x: c_int,
            y: c_int,
            width: c_uint,
            height: c_uint,
            border_width: c_uint,
            border: c_ulong,
            background: c_ulong,
        ) -> Window;
        /// Set the window title.
        fn XStoreName(display: *mut Display, w: Window, window_name: *const c_char) -> c_int;
        /// Subscribe to expose, key, button, motion, and configure events.
        fn XSelectInput(display: *mut Display, w: Window, event_mask: c_long) -> c_int;
        /// Map the window so it becomes visible.
        fn XMapWindow(display: *mut Display, w: Window) -> c_int;
        /// Destroy the window after the loop stops.
        fn XDestroyWindow(display: *mut Display, w: Window) -> c_int;
        /// Look up WM_DELETE_WINDOW.
        fn XInternAtom(
            display: *mut Display,
            atom_name: *const c_char,
            only_if_exists: c_int,
        ) -> Atom;
        /// Deliver the close button as a ClientMessage instead of destroying immediately.
        fn XSetWMProtocols(
            display: *mut Display,
            w: Window,
            protocols: *mut Atom,
            count: c_int,
        ) -> c_int;
        /// Non-blocking event count. The loop sleeps 8 milliseconds when the queue is empty.
        fn XPending(display: *mut Display) -> c_int;
        /// Read the next X event.
        fn XNextEvent(display: *mut Display, event_return: *mut XEvent) -> c_int;
        /// Translate a key event into a keysym and a Latin-1 byte, if any.
        fn XLookupString(
            event_struct: *mut XKeyEvent,
            buffer_return: *mut c_char,
            bytes_buffer: c_int,
            keysym_return: *mut KeySym,
            status_in_out: *mut c_void,
        ) -> c_int;
        /// Wrap the CPU pixel buffer as a ZPixmap. The buffer stays owned by Rust.
        fn XCreateImage(
            display: *mut Display,
            visual: *mut Visual,
            depth: c_uint,
            format: c_int,
            offset: c_int,
            data: *mut c_char,
            width: c_uint,
            height: c_uint,
            bitmap_pad: c_int,
            bytes_per_line: c_int,
        ) -> *mut XImage;
        /// Copy the ZPixmap into the window. This is the Linux present path, not a GPU present.
        fn XPutImage(
            display: *mut Display,
            d: Window,
            gc: GC,
            image: *mut XImage,
            src_x: c_int,
            src_y: c_int,
            dest_x: c_int,
            dest_y: c_int,
            width: c_uint,
            height: c_uint,
        ) -> c_int;
        /// Send the copy to the server.
        fn XFlush(display: *mut Display) -> c_int;
    }

    impl Host {
        /// Open target on the active tab from the X11 loop.
        /// Search queries, HSTS, and HTTPS-only match the Win32 path.
        /// The network shield is not consulted here.
        /// Remote loads run on a worker. This loop polls pending_load because it does not post an X event.
        fn start_navigation_x11(&mut self, target: &str) {
            let resolved = crate::frontend::resolve_navigation_query_with_engine(
                target,
                self.session.search_engine(),
            )
            .unwrap_or_else(|| target.to_string());
            self.stop_loading();
            if self.session.try_same_page_fragment_navigation(&resolved) {
                return;
            }
            let upgraded = self.session.hsts_store().upgrade_url_if_hsts(&resolved);
            if upgraded.starts_with("http://") || upgraded.starts_with("https://") {
                let url = upgraded;
                let viewport = self.session.viewport();
                let fetch_jar = self.session.cookie_jar_handle();
                let scripts_enabled = self.session.scripts_enabled_for(&url);
                let https_only = self.session.https_only_policy();
                let userscripts = self.session.userscript_injection();
                let tab_id = self.session.active_tab_id();
                let workers = self.session.tab_workers();
                let slot = Arc::clone(&self.pending_load);
                let ticket = slot.lock().expect("load queue").begin(tab_id);
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let _https_only = anvil_net::enter_https_only(https_only);
                    let _userscripts = crate::userscripts::enter(userscripts);
                    let result = super::catch_load(|| {
                        load_target_in_tab(
                            &url,
                            viewport,
                            Some(fetch_jar),
                            scripts_enabled,
                            tab_id,
                            &workers,
                        )
                    });
                    if let Ok(mut guard) = slot.lock() {
                        guard.finish(tab_id, ticket, result);
                    }
                });
            } else {
                let _ = self.session.open_target(&resolved);
                let _ = self.session.save_default_profile();
            }
        }

        /// Submit a POST body on the X11 path.
        /// http and https run on a worker with no shield check. Other targets ignore the body.
        fn start_post_navigation_x11(&mut self, target: &str, body: &str) {
            self.stop_loading();
            if target.starts_with("http://") || target.starts_with("https://") {
                let url = target.to_string();
                let payload = body.to_string();
                let viewport = self.session.viewport();
                let fetch_jar = self.session.cookie_jar_handle();
                let scripts_enabled = self.session.scripts_enabled_for(&url);
                let https_only = self.session.https_only_policy();
                let userscripts = self.session.userscript_injection();
                let tab_id = self.session.active_tab_id();
                let workers = self.session.tab_workers();
                let slot = Arc::clone(&self.pending_load);
                let ticket = slot.lock().expect("load queue").begin(tab_id);
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let _https_only = anvil_net::enter_https_only(https_only);
                    let _userscripts = crate::userscripts::enter(userscripts);
                    let result = super::catch_load(|| {
                        load_post_in_tab(
                            &url,
                            &payload,
                            viewport,
                            Some(fetch_jar),
                            scripts_enabled,
                            tab_id,
                            &workers,
                        )
                    });
                    if let Ok(mut guard) = slot.lock() {
                        guard.finish(tab_id, ticket, result);
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }
    }

    /// Create a native Wayland or X11 window and run until close or Escape.
    /// The initial page is target, or anvil://home. The requested size is at least 480 by 320.
    /// Finished loads are polled at the top of the loop.
    pub fn open(target: Option<&str>, window_size: Viewport) -> Result<(), String> {
        let win_w = (window_size.width.round() as u32).max(480);
        let win_h = (window_size.height.round() as u32).max(320);
        let mut session = Session::new(content_viewport(win_w as f32, win_h as f32));
        if session.load_default_profile().is_err() {
            if let Some((_, _, scale_pct)) = crate::wayland::query_wayland_monitor_info()
                && scale_pct > 100
            {
                let _ = session.set_zoom_percent(scale_pct as u16);
            } else if win_w >= 3000 {
                let _ = session.zoom_in();
                let _ = session.zoom_in();
            } else if win_w >= 2000 {
                let _ = session.zoom_in();
            }
        }
        session.check_renderer_confinement_in_background();
        let ui = UiState::new();

        session.set_viewport(content_viewport_for(
            &session,
            &ui,
            win_w as f32,
            win_h as f32,
        ));
        let initial_target = target.unwrap_or(HOME_URL);
        session.open_target(initial_target)?;

        let page_fb = session.rasterize();
        let framebuffer = compose_frame_with_page_fb(&session, &ui, win_w, win_h, &page_fb);
        let mut host = Host {
            session,
            ui,
            window_width: win_w,
            window_height: win_h,
            framebuffer,
            cached_page_fb: Some(page_fb),
            pending_load: Arc::new(Mutex::new(super::LoadQueue::default())),
            animating: false,
            check_animation: true,
            next_animation_frame: std::time::Instant::now(),
            dirty_rows: None,
            present: crate::present::PresentPath::Untried,
        };

        if crate::wayland::resolve_wayland_socket_path().is_some() {
            match open_wayland(&mut host) {
                Ok(()) => return Ok(()),
                Err(err) => eprintln!("Wayland backend notice (trying X11 fallback): {err}"),
            }
        }

        unsafe {
            let display = XOpenDisplay(null());
            if display.is_null() {
                return Err("could not connect to Wayland or X11 display server".to_string());
            }
            let screen = XDefaultScreen(display);
            let root = XRootWindow(display, screen);
            let black = XBlackPixel(display, screen);
            let window = XCreateSimpleWindow(display, root, 0, 0, win_w, win_h, 1, black, black);

            let mut wm_delete = XInternAtom(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            XSetWMProtocols(display, window, &mut wm_delete, 1);

            XSelectInput(
                display,
                window,
                EXPOSURE_MASK
                    | KEY_PRESS_MASK
                    | BUTTON_PRESS_MASK
                    | POINTER_MOTION_MASK
                    | STRUCTURE_NOTIFY_MASK,
            );
            set_x11_title(display, window, &host.window_title());
            XMapWindow(display, window);
            present_x11(display, screen, window, &mut host);

            loop {
                if let Some((tab_id, res)) =
                    host.pending_load.lock().ok().and_then(|mut g| g.take())
                {
                    host.ui.set_loading(false);
                    let page = match res {
                        Ok(page) => page,
                        Err(message) => navigation_error_page(
                            "anvil://navigation-error",
                            &message,
                            host.session.viewport(),
                        ),
                    };
                    host.session.open_page_in_tab_id(tab_id, page);
                    host.check_animation = true;
                    host.redraw();
                    set_x11_title(display, window, &host.window_title());
                    present_x11(display, screen, window, &mut host);
                }

                // Transitions and animations: draw a frame when one is due.
                if host.animation_step() {
                    present_x11(display, screen, window, &mut host);
                }

                if XPending(display) == 0 {
                    thread::sleep(Duration::from_millis(8));
                    continue;
                }

                let mut ev = XEvent { pad: [0; 24] };
                XNextEvent(display, &mut ev);
                // The event may start a transition; look before the next wait.
                host.check_animation = true;

                match ev.type_ {
                    // Copy the CPU framebuffer again. Expose does not rasterize unless a later event does.
                    EXPOSE => {
                        present_x11(display, screen, window, &mut host);
                    }
                    // WM_DELETE_WINDOW ends the loop. Other client messages are ignored.
                    CLIENT_MESSAGE => {
                        if ev.client_message.data_l[0] as Atom == wm_delete {
                            break;
                        }
                    }
                    // Apply a new client size through content_viewport, then rasterize and present.
                    CONFIGURE_NOTIFY => {
                        let w = ev.configure.width.max(1) as u32;
                        let h = ev.configure.height.max(1) as u32;
                        if w != host.window_width || h != host.window_height {
                            host.window_width = w;
                            host.window_height = h;
                            host.redraw();
                            present_x11(display, screen, window, &mut host);
                        }
                    }
                    // Client x and y. Redraw when the hovered link changes or the pointer is over the toolbar.
                    MOTION_NOTIFY => {
                        // X11 client x. The same space as Win32 mouse_coords. Page y is applied in the hit test.
                        let x = ev.motion.x as f32;
                        let y = ev.motion.y as f32;
                        let cursor_moved = host.ui.set_cursor_pos(x, y);
                        if (ev.motion.state & 256) != 0
                            && let Some((sx, sy)) = host.ui.drag_select_start()
                        {
                            let header_h = top_header_height_for(&host.session, &host.ui);
                            let py = (y - header_h).max(0.0);
                            if (x - sx).hypot(py - sy) >= 4.0 {
                                let _ = host.session.select_text_range(sx, sy, x, py);
                                host.redraw_chrome_only();
                                present_x11(display, screen, window, &mut host);
                            }
                            continue;
                        } else if (ev.motion.state & 256) == 0
                            && host.ui.drag_select_start().is_some()
                        {
                            host.ui.stop_drag_select();
                        }
                        let hit = hit_test_ui_with_state_and_height(
                            &host.session,
                            &host.ui,
                            host.window_width as f32,
                            host.window_height as f32,
                            x,
                            y,
                        );
                        let hovered = match hit {
                            Some(UiHit::Page { x: px, y: py }) => host.session.hit_link(px, py),
                            _ => None,
                        };
                        let in_interactive_chrome = y <= TOP_BAR_HEIGHT + 36.0
                            || matches!(hit, Some(UiHit::Scrollbar { .. }))
                            || host.ui.select_dropdown().is_some()
                            || host.ui.context_menu_pos().is_some()
                            || host.ui.is_app_menu_open()
                            || host.ui.is_site_popover_open()
                            || host.ui.is_tab_search_open()
                            || host.ui.is_downloads_popover_open()
                            || host.ui.is_find_open()
                            || host.ui.is_inspector_open();
                        if host.ui.set_hovered_link(hovered)
                            || (cursor_moved && in_interactive_chrome)
                        {
                            host.redraw_chrome_only();
                            present_x11(display, screen, window, &mut host);
                        }
                    }
                    // Button 4 and 5 scroll 48 pixels (or zoom when Control is held).
                    // Button 3 opens the context menu. Button 1 runs the same hits as the Win32 left click.
                    BUTTON_PRESS => {
                        let btn = ev.button.button;
                        if btn == 4 || btn == 5 {
                            if (ev.button.state & 4) != 0 {
                                let before = host.session.zoom_percent();
                                let after = if btn == 4 {
                                    host.session.zoom_in()
                                } else {
                                    host.session.zoom_out()
                                };
                                if after != before {
                                    host.redraw();
                                    present_x11(display, screen, window, &mut host);
                                }
                            } else {
                                let step = if btn == 4 { -48.0 } else { 48.0 };
                                let x = ev.button.x as f32;
                                let y = ev.button.y as f32;
                                let before_scroll = host.session.scroll();
                                let scrolled = if let Some(UiHit::Page { x: px, y: py }) =
                                    hit_test_ui_with_state_and_height(
                                        &host.session,
                                        &host.ui,
                                        host.window_width as f32,
                                        host.window_height as f32,
                                        x,
                                        y,
                                    ) {
                                    host.session.scroll_at(px, py, step)
                                } else {
                                    let after = host.session.scroll_by(step);
                                    (after - before_scroll).abs() > f32::EPSILON
                                };
                                if scrolled {
                                    host.redraw_after_scroll(before_scroll);
                                    present_x11(display, screen, window, &mut host);
                                }
                            }
                        } else if btn == 3 {
                            // X11 client x for this button event. Page y is applied in the hit test.
                            let x = ev.button.x as f32;
                            let y = ev.button.y as f32;
                            let mut context_link = None;
                            if let Some(UiHit::Page { x: px, y: py }) =
                                hit_test_ui_with_state_and_height(
                                    &host.session,
                                    &host.ui,
                                    host.window_width as f32,
                                    host.window_height as f32,
                                    x,
                                    y,
                                )
                            {
                                let _ = host.session.inspect_at(px, py);
                                context_link = host.session.hit_link(px, py);
                                if context_link.is_none() && host.session.selected_text().is_empty()
                                {
                                    let _ = host.session.select_text_at(px, py);
                                }
                            }
                            host.ui.open_context_menu_with_link(x, y, context_link);
                            host.redraw();
                            present_x11(display, screen, window, &mut host);
                        } else if btn == 2 {
                            let x = ev.button.x as f32;
                            let y = ev.button.y as f32;
                            let hit = hit_test_ui_with_state_and_height(
                                &host.session,
                                &host.ui,
                                host.window_width as f32,
                                host.window_height as f32,
                                x,
                                y,
                            );
                            match hit {
                                Some(UiHit::Tab(index) | UiHit::CloseTab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if host.session.close_tab(index) {
                                        host.redraw();
                                        set_x11_title(display, window, &host.window_title());
                                        present_x11(display, screen, window, &mut host);
                                    }
                                }
                                Some(UiHit::Page { x: px, y: py }) => {
                                    if let Some(link) = host.session.hit_link(px, py) {
                                        host.ui.blur_address();
                                        host.session.blur_page_input();
                                        host.session.new_tab();
                                        host.start_navigation_x11(&link);
                                        host.redraw();
                                        set_x11_title(display, window, &host.window_title());
                                        present_x11(display, screen, window, &mut host);
                                    }
                                }
                                _ => {}
                            }
                        } else if btn == 1 {
                            // X11 client x for this button event. Page y is applied in the hit test.
                            let x = ev.button.x as f32;
                            let y = ev.button.y as f32;
                            let hit = hit_test_ui_with_state_and_height(
                                &host.session,
                                &host.ui,
                                host.window_width as f32,
                                host.window_height as f32,
                                x,
                                y,
                            );
                            let mut changed = false;
                            // A click takes keyboard focus off the toolbar.
                            if host.ui.clear_chrome_focus() {
                                changed = true;
                            }
                            if host.ui.is_editing_console()
                                && !matches!(hit, Some(UiHit::InspectorConsoleInput))
                            {
                                host.ui.blur_console();
                                changed = true;
                            }
                            if host.ui.is_editing_style()
                                && !matches!(hit, Some(UiHit::InspectorStyleInput))
                            {
                                host.ui.blur_style();
                                changed = true;
                            }
                            if host.ui.select_dropdown().is_some()
                                && !matches!(hit, Some(UiHit::SelectDropdownOption(_)))
                            {
                                host.ui.close_select_dropdown();
                                changed = true;
                            }
                            if host.ui.is_bookmark_overflow_open()
                                && !matches!(
                                    hit,
                                    Some(
                                        UiHit::Bookmark(_)
                                            | UiHit::BookmarkOverflow
                                            | UiHit::BookmarkOverflowMore
                                    )
                                )
                            {
                                host.ui.close_bookmark_overflow();
                                changed = true;
                            }
                            if host.ui.context_menu_pos().is_some()
                                && !matches!(hit, Some(UiHit::ContextMenuItem(_)))
                            {
                                host.ui.close_context_menu();
                                changed = true;
                            }
                            if host.ui.is_app_menu_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::AppMenuItem(_)) | Some(UiHit::AppMenuButton)
                                )
                            {
                                host.ui.close_app_menu();
                                changed = true;
                            }
                            if host.ui.is_site_popover_open()
                                && !matches!(
                                    hit,
                                    Some(
                                        UiHit::SecurityBadge
                                            | UiHit::ShieldsButton
                                            | UiHit::SitePopoverItem(_)
                                    )
                                )
                            {
                                host.ui.close_site_popover();
                                changed = true;
                            }
                            if host.ui.is_tab_search_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::TabSearchButton | UiHit::TabSearchItem(_))
                                )
                            {
                                host.ui.close_tab_search();
                                changed = true;
                            }
                            if host.ui.is_downloads_popover_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::DownloadsButton | UiHit::DownloadsPopoverItem(_))
                                )
                            {
                                host.ui.close_downloads_popover();
                                changed = true;
                            }
                            match hit {
                                Some(UiHit::TabSearchButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_tab_search();
                                    changed = true;
                                }
                                Some(UiHit::TabSearchItem(tab_idx)) => {
                                    host.ui.close_tab_search();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    let _ = host.session.select(tab_idx);
                                    changed = true;
                                }
                                Some(UiHit::DownloadsPopoverItem(idx)) => {
                                    let save_idx =
                                        host.session.downloads().entries().len().clamp(1, 4);
                                    host.ui.close_downloads_popover();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    if idx == save_idx {
                                        let _ = host.session.save_active_page_to_downloads();
                                        let _ = host.session.save_default_profile();
                                    } else {
                                        host.start_navigation_x11(DOWNLOADS_URL);
                                    }
                                }
                                Some(UiHit::SitePopoverItem(action)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    match action {
                                        SitePopoverAction::ToggleSiteShields => {
                                            let _ = host.session.toggle_shields_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::CycleShieldsMode => {
                                            let _ = host.session.cycle_shields_mode();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::ToggleSiteJavaScript => {
                                            let _ = host.session.toggle_scripts_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::ToggleSiteCookies => {
                                            let _ = host.session.toggle_cookies_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::OpenSecurityCenter => {
                                            host.ui.close_site_popover();
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                    }
                                }
                                Some(UiHit::ReaderModeButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.toggle_reader_mode();
                                }
                                Some(UiHit::OmniboxSuggestion(idx)) => {
                                    let target = host
                                        .ui
                                        .omnibox_suggestions(&host.session)
                                        .get(idx)
                                        .map(|s| s.url.clone());
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if let Some(url) = target {
                                        host.start_navigation_x11(&url);
                                    }
                                    changed = true;
                                }
                                Some(UiHit::InspectorConsoleInput) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.focus_console();
                                    changed = true;
                                }
                                Some(UiHit::InspectorStyleInput) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.focus_style();
                                    changed = true;
                                }
                                Some(UiHit::SelectDropdownOption(option_idx)) => {
                                    let select_id = host.ui.select_dropdown().map(|d| d.node_id);
                                    host.ui.close_select_dropdown();
                                    if let Some(id) = select_id {
                                        changed = host
                                            .session
                                            .choose_select_option(id, option_idx)
                                            .is_some();
                                    } else {
                                        changed = true;
                                    }
                                }
                                Some(UiHit::AppMenuButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_app_menu();
                                    changed = true;
                                }
                                Some(UiHit::AppMenuItem(action)) => {
                                    host.ui.close_app_menu();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    match action {
                                        AppMenuAction::NewTab => {
                                            host.session.new_tab();
                                            let _ = host.session.open_target(HOME_URL);
                                        }
                                        AppMenuAction::TabSearch => {
                                            host.ui.open_tab_search();
                                        }
                                        AppMenuAction::HomeDashboard => {
                                            host.start_navigation_x11(HOME_URL);
                                        }
                                        AppMenuAction::Downloads => {
                                            host.start_navigation_x11(DOWNLOADS_URL);
                                        }
                                        AppMenuAction::History => {
                                            host.start_navigation_x11(HISTORY_URL);
                                        }
                                        AppMenuAction::SecurityCenter => {
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                        AppMenuAction::ToggleReaderMode => {
                                            let _ = host.session.toggle_reader_mode();
                                        }
                                        AppMenuAction::ToggleBookmarksBar => {
                                            host.ui.toggle_bookmarks_bar();
                                        }
                                        AppMenuAction::FindInPage => {
                                            host.ui.open_find();
                                        }
                                        AppMenuAction::ToggleInspectorDock => {
                                            host.ui.toggle_inspector();
                                        }
                                        AppMenuAction::OpenInspectorPage => {
                                            host.start_navigation_x11(INSPECT_URL);
                                        }
                                        AppMenuAction::CycleZoom => {
                                            if host.session.zoom_percent() != 100 {
                                                let _ = host.session.reset_zoom();
                                            } else {
                                                let _ = host.session.zoom_in();
                                            }
                                        }
                                        AppMenuAction::CycleTheme => {
                                            host.session.cycle_theme();
                                            let url = host.current_url();
                                            if url.starts_with("anvil://") {
                                                let _ = host.session.open_target(&url);
                                            }
                                        }
                                        AppMenuAction::Settings => {
                                            host.start_navigation_x11(SETTINGS_URL);
                                        }
                                    }
                                }
                                Some(UiHit::DownloadsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_downloads_popover();
                                    changed = true;
                                }
                                Some(UiHit::InspectorButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_inspector();
                                    changed = true;
                                }
                                Some(UiHit::InspectorTab(tab)) => {
                                    host.ui.set_inspector_tab(tab);
                                    changed = true;
                                }
                                Some(UiHit::InspectorElementRow(node_id)) => {
                                    host.session.select_inspected_node(node_id);
                                    changed = true;
                                }
                                Some(UiHit::InspectorFullPage) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(INSPECT_URL);
                                    changed = true;
                                }
                                Some(UiHit::InspectorClose) => {
                                    host.ui.close_inspector();
                                    changed = true;
                                }
                                Some(UiHit::ContextMenuItem(action)) => {
                                    let context_link =
                                        host.ui.context_menu_link().map(str::to_string);
                                    host.ui.close_context_menu();
                                    changed = true;
                                    match action {
                                        ContextMenuAction::OpenLinkInNewTab => {
                                            if let Some(link) = context_link {
                                                host.ui.blur_address();
                                                host.session.blur_page_input();
                                                host.session.new_tab();
                                                host.start_navigation_x11(&link);
                                            }
                                        }
                                        ContextMenuAction::CopyLinkAddress => {
                                            if let Some(link) = context_link {
                                                host.session.set_clipboard_text(&link);
                                            }
                                        }
                                        ContextMenuAction::Back => {
                                            let _ = host.history_back();
                                        }
                                        ContextMenuAction::Forward => {
                                            let _ = host.history_forward();
                                        }
                                        ContextMenuAction::Reload => {
                                            let url = host.current_url();
                                            if !url.is_empty() {
                                                host.start_navigation_x11(&url);
                                            }
                                        }
                                        ContextMenuAction::ReopenClosedTab => {
                                            let _ = host.session.reopen_closed_tab();
                                        }
                                        ContextMenuAction::DuplicateTab => {
                                            let active = host.session.active_index();
                                            let _ = host.session.duplicate_tab(active);
                                        }
                                        ContextMenuAction::TogglePinTab => {
                                            let active = host.session.active_index();
                                            let _ = host.session.toggle_pin_tab(active);
                                        }
                                        ContextMenuAction::CloseOtherTabs => {
                                            let active = host.session.active_index();
                                            let _ = host.session.close_other_tabs(active);
                                        }
                                        ContextMenuAction::Copy => {
                                            let _ = host.session.copy_selection_or_focused();
                                        }
                                        ContextMenuAction::Paste => {
                                            let clip = host.session.clipboard_text().to_string();
                                            let _ = host.session.paste_into_focused_input(&clip);
                                        }
                                        ContextMenuAction::FindInPage => {
                                            host.session.blur_page_input();
                                            host.ui.open_find();
                                        }
                                        ContextMenuAction::SavePage => {
                                            if host
                                                .session
                                                .save_active_page_to_downloads()
                                                .is_some()
                                            {
                                                host.start_navigation_x11(DOWNLOADS_URL);
                                            }
                                        }
                                        ContextMenuAction::SavePdf => {
                                            if host.session.save_active_page_as_pdf().is_some() {
                                                host.start_navigation_x11(DOWNLOADS_URL);
                                            }
                                        }
                                        ContextMenuAction::ViewPageSource => {
                                            let _ = host.session.view_active_page_source();
                                        }
                                        ContextMenuAction::ToggleReaderMode => {
                                            let _ = host.session.toggle_reader_mode();
                                        }
                                        ContextMenuAction::ResetZoom => {
                                            let _ = host.session.reset_zoom();
                                        }
                                        ContextMenuAction::InspectPage => {
                                            host.ui.toggle_inspector();
                                        }
                                        ContextMenuAction::SecurityCenter => {
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                    }
                                }
                                Some(UiHit::FindInput) => {
                                    host.session.blur_page_input();
                                    host.ui.open_find();
                                    changed = true;
                                }
                                Some(UiHit::FindCaseToggle) => {
                                    host.ui.toggle_find_case_sensitive();
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindPrev) => {
                                    let count = host.find_match_count();
                                    host.ui.find_prev(count);
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindNext) => {
                                    let count = host.find_match_count();
                                    host.ui.find_next(count);
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindClose) => {
                                    host.ui.close_find();
                                    changed = true;
                                }
                                Some(UiHit::Tab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.select(index);
                                }
                                Some(UiHit::CloseTab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.close_tab(index);
                                }
                                Some(UiHit::NewTab) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.session.new_tab();
                                    let _ = host.session.open_target(HOME_URL);
                                    changed = true;
                                }
                                Some(UiHit::Back) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.history_back();
                                }
                                Some(UiHit::Forward) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.history_forward();
                                }
                                Some(UiHit::Reload) => {
                                    if host.ui.is_loading() {
                                        host.stop_loading();
                                        changed = true;
                                    } else {
                                        host.ui.blur_address();
                                        host.session.blur_page_input();
                                        let url = host.current_url();
                                        if !url.is_empty() {
                                            host.start_navigation_x11(&url);
                                            changed = true;
                                        }
                                    }
                                }
                                Some(UiHit::Home) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(HOME_URL);
                                    changed = true;
                                }
                                Some(UiHit::WindowClose) => {
                                    break;
                                }
                                Some(
                                    UiHit::WindowMinimize
                                    | UiHit::WindowMaximize
                                    | UiHit::TitlebarDrag,
                                ) => {}
                                Some(UiHit::SecurityBadge | UiHit::ShieldsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_site_popover();
                                    changed = true;
                                }
                                Some(UiHit::AddressField) => {
                                    host.session.blur_page_input();
                                    let url = host.current_url();
                                    host.ui.focus_address(&url);
                                    changed = true;
                                }
                                Some(UiHit::ClearAddress) => {
                                    host.ui.clear_address();
                                    changed = true;
                                }
                                Some(UiHit::ToggleBookmark) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.session.toggle_bookmark_active();
                                    let _ = host.session.save_default_profile();
                                    changed = true;
                                }
                                Some(UiHit::ZoomPill) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if host.session.zoom_percent() != 100 {
                                        let _ = host.session.reset_zoom();
                                    } else {
                                        let _ = host.session.zoom_in();
                                    }
                                    changed = true;
                                }
                                Some(UiHit::SettingsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(SETTINGS_URL);
                                    changed = true;
                                }
                                Some(UiHit::BookmarkOverflow) => {
                                    host.ui.toggle_bookmark_overflow();
                                    changed = true;
                                }
                                Some(UiHit::BookmarkOverflowMore) => {
                                    host.ui.next_bookmark_overflow(
                                        host.session.bookmarks().len(),
                                        crate::ui::bookmark_bar_visible_count(
                                            &host.session,
                                            host.window_width as f32,
                                        ),
                                    );
                                    changed = true;
                                }
                                Some(UiHit::Bookmark(index)) => {
                                    host.ui.close_bookmark_overflow();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if let Some(bm) = host.session.bookmarks().get(index).cloned() {
                                        host.start_navigation_x11(&bm.url);
                                        changed = true;
                                    }
                                }
                                Some(UiHit::Scrollbar { ratio }) => {
                                    host.ui.blur_address();
                                    let before = host.session.scroll();
                                    let after = host.session.scroll_to_ratio(ratio);
                                    changed = (after - before).abs() > f32::EPSILON;
                                }
                                // Page click on X11. px and py are already in page space.
                                Some(UiHit::Page { x: px, y: py }) => {
                                    if host.ui.is_editing_address() {
                                        host.ui.blur_address();
                                    }
                                    match host.session.click_at(px, py) {
                                        ClickOutcome::SelectChanged(node_id, _) => {
                                            if let Some((rect, options)) =
                                                host.session.select_dropdown_info(node_id)
                                                && options.len() > 1
                                            {
                                                host.ui
                                                    .open_select_dropdown(node_id, rect, options);
                                            }
                                            changed = true;
                                        }
                                        ClickOutcome::ScriptHandled
                                        | ClickOutcome::InputFocused(_)
                                        | ClickOutcome::CheckboxToggled(..)
                                        | ClickOutcome::VideoToggled(..)
                                        | ClickOutcome::VideoSeeked(..) => {
                                            changed = true;
                                        }
                                        ClickOutcome::Navigate(target) => {
                                            host.start_navigation_x11(&target);
                                            changed = true;
                                        }
                                        ClickOutcome::SubmitPost { url, body } => {
                                            host.start_post_navigation_x11(&url, &body);
                                            changed = true;
                                        }
                                        ClickOutcome::None => {
                                            host.session.clear_selection();
                                            match host.ui.record_page_click(px, py) {
                                                2 => {
                                                    let _ = host.session.select_word_at(px, py);
                                                }
                                                3 => {
                                                    let _ = host.session.select_line_at(px, py);
                                                }
                                                _ => {
                                                    let _ = host.session.select_text_at(px, py);
                                                    host.ui.start_drag_select(px, py);
                                                }
                                            }
                                            let _ = host.session.inspect_at(px, py);
                                            changed = true;
                                        }
                                    }
                                }
                                None => {
                                    if host.ui.is_editing_address() {
                                        host.ui.blur_address();
                                        changed = true;
                                    }
                                    if host.session.blur_page_input() {
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                host.redraw();
                                set_x11_title(display, window, &host.window_title());
                                present_x11(display, screen, window, &mut host);
                            }
                        }
                    }
                    // Keys go through crate::keys: the keysym and the Shift, Control, and Alt bits
                    // become a command for whatever has focus (address bar, find, a page field, or
                    // the page). Typing works in all three fields. Control+T, W, L, F, G, H, J, R,
                    // C, X, V, plus, minus, and 0 match Windows, as do Alt+Left, Alt+Right,
                    // Alt+Home, F3, F5, F6, and F12. Escape leaves a field, then closes the
                    // context menu, app menu, find, and inspector, and then closes the window.
                    // Copy and paste use the session clipboard, not the X selection.
                    KEY_PRESS => {
                        let mut key_ev = ev.key;
                        let mut keysym: KeySym = 0;
                        let mut buf = [0u8; 8];
                        XLookupString(
                            &mut key_ev,
                            buf.as_mut_ptr().cast::<c_char>(),
                            buf.len() as c_int,
                            &mut keysym,
                            null_mut(),
                        );
                        let key = key_from_x11(keysym as u64);
                        let mods = Modifiers::from_x11_state(key_ev.state);
                        let focus = focus_of(&host.session, &host.ui);
                        let command = command_for(key, mods, focus);
                        if matches!(
                            command,
                            crate::keys::KeyCommand::Back | crate::keys::KeyCommand::Forward
                        ) {
                            host.stop_loading();
                        }
                        let effect = apply_key_command(&mut host.session, &mut host.ui, command);
                        match effect {
                            KeyEffect::Nothing => continue,
                            KeyEffect::CloseWindow => break,
                            KeyEffect::Redraw => {}
                            KeyEffect::StopLoading => host.stop_loading(),
                            KeyEffect::Navigate(target) => host.start_navigation_x11(&target),
                            KeyEffect::Post { url, body } => {
                                host.start_post_navigation_x11(&url, &body)
                            }
                        }
                        host.redraw();
                        set_x11_title(display, window, &host.window_title());
                        present_x11(display, screen, window, &mut host);
                    }
                    // Event types this loop does not use. Key and button defaults are handled inside those arms.
                    _ => {}
                }
            }

            let _ = host.session.save_default_profile();
            XDestroyWindow(display, window);
            XCloseDisplay(display);
        }
        Ok(())
    }

    /// Set the caption. A title that contains a nul is left unchanged.
    unsafe fn set_x11_title(display: *mut Display, window: Window, title: &str) {
        if let Ok(c_title) = CString::new(title) {
            unsafe {
                XStoreName(display, window, c_title.as_ptr());
            }
        }
    }

    /// Copy the top-down BGRA framebuffer (or dirty row band) into the window with XPutImage.
    /// image.data is cleared before the XImage goes out of scope so X11 does not free the buffer.
    unsafe fn present_x11(display: *mut Display, screen: c_int, window: Window, host: &mut Host) {
        let width = host.framebuffer.width();
        let height = host.framebuffer.height();
        let dirty = host.dirty_rows.take();
        let (bgra, top, bottom) = host.present.prepare_x11_bgra(&host.framebuffer, dirty);
        let band_h = bottom.saturating_sub(top);
        if width == 0 || height == 0 || band_h == 0 {
            return;
        }
        unsafe {
            let visual = XDefaultVisual(display, screen);
            let depth = XDefaultDepth(display, screen) as c_uint;
            let gc = XDefaultGC(display, screen);
            let image = XCreateImage(
                display,
                visual,
                depth,
                Z_PIXMAP,
                0,
                bgra.as_ptr().cast_mut().cast::<c_char>(),
                width,
                height,
                32,
                (width * 4) as c_int,
            );
            if !image.is_null() {
                XPutImage(
                    display,
                    window,
                    gc,
                    image,
                    0,
                    top as c_int,
                    0,
                    top as c_int,
                    width,
                    band_h,
                );
                (*image).data = null_mut();
            }
            XFlush(display);
        }
    }

    /// Run the native Wayland event loop (`wl_shm` + `xdg_wm_base` + `zxdg_decoration_manager_v1` +
    /// `wl_data_device_manager` + `wp_cursor_shape_manager_v1` + `wp_fractional_scale_manager_v1`).
    fn open_wayland(host: &mut Host) -> Result<(), String> {
        use crate::keys::KeyCommand;
        use crate::wayland::{
            BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, WaylandCursorShape, WaylandEvent, WaylandWindow,
            hit_code_to_cursor_shape, hit_code_to_wayland_edge,
        };

        let mut wl =
            WaylandWindow::connect(&host.window_title(), host.window_width, host.window_height)?;
        let (cfg_w, cfg_h, cfg_max) = wl.configured_size();
        let mut last_maximized = cfg_max;
        crate::ui::cpp_ui_window_action(0, if cfg_max { 101 } else { 100 });
        if cfg_w > 0
            && cfg_h > 0
            && (cfg_w != host.window_width || cfg_h != host.window_height || cfg_max)
        {
            host.window_width = cfg_w.max(320);
            host.window_height = cfg_h.max(240);
            host.redraw();
        }
        let _ = host.dirty_rows.take();
        let _ = host
            .present
            .present_wayland(&mut wl, &host.framebuffer, None);

        'wayland_loop: loop {
            if let Some((tab_id, res)) = host.pending_load.lock().ok().and_then(|mut g| g.take()) {
                host.ui.set_loading(false);
                let page = match res {
                    Ok(page) => page,
                    Err(message) => navigation_error_page(
                        "anvil://navigation-error",
                        &message,
                        host.session.viewport(),
                    ),
                };
                host.session.open_page_in_tab_id(tab_id, page);
                host.check_animation = true;
                host.redraw();
                wl.set_title(&host.window_title());
                let dirty = host.dirty_rows.take();
                let _ = host
                    .present
                    .present_wayland(&mut wl, &host.framebuffer, dirty);
            }

            if host.animation_step() {
                let dirty = host.dirty_rows.take();
                let _ = host
                    .present
                    .present_wayland(&mut wl, &host.framebuffer, dirty);
            }

            let raw_events = wl.poll_events()?;
            if raw_events.is_empty() {
                thread::sleep(Duration::from_millis(8));
                continue;
            }

            let mut events = Vec::with_capacity(raw_events.len());
            for ev in raw_events {
                match ev {
                    WaylandEvent::PointerMotion { .. } => {
                        if matches!(events.last(), Some(WaylandEvent::PointerMotion { .. })) {
                            *events.last_mut().unwrap() = ev;
                        } else {
                            events.push(ev);
                        }
                    }
                    WaylandEvent::PointerScroll {
                        delta_y,
                        x,
                        y,
                        ctrl,
                    } => {
                        if let Some(WaylandEvent::PointerScroll {
                            delta_y: prev_dy,
                            x: prev_x,
                            y: prev_y,
                            ctrl: prev_ctrl,
                        }) = events.last_mut()
                            && *prev_ctrl == ctrl
                        {
                            *prev_dy += delta_y;
                            *prev_x = x;
                            *prev_y = y;
                        } else {
                            events.push(WaylandEvent::PointerScroll {
                                delta_y,
                                x,
                                y,
                                ctrl,
                            });
                        }
                    }
                    WaylandEvent::Configure { .. } => {
                        events.retain(|prev| !matches!(prev, WaylandEvent::Configure { .. }));
                        events.push(ev);
                    }
                    WaylandEvent::FrameDone { .. } => {
                        events.retain(|prev| !matches!(prev, WaylandEvent::FrameDone { .. }));
                        events.push(ev);
                    }
                    _ => events.push(ev),
                }
            }

            for ev in events {
                host.check_animation = true;
                match ev {
                    WaylandEvent::Close => break 'wayland_loop,
                    WaylandEvent::FrameDone { .. } => {
                        if host.animation_step() {
                            let dirty = host.dirty_rows.take();
                            let _ = host
                                .present
                                .present_wayland(&mut wl, &host.framebuffer, dirty);
                        }
                    }
                    WaylandEvent::ScaleChanged { scale_percent } => {
                        let target_zoom = (scale_percent as u16).clamp(50, 250);
                        if host.session.zoom_percent() == 100 && target_zoom != 100 {
                            let _ = host.session.set_zoom_percent(target_zoom);
                            host.redraw();
                            let dirty = host.dirty_rows.take();
                            let _ = host
                                .present
                                .present_wayland(&mut wl, &host.framebuffer, dirty);
                        }
                    }
                    WaylandEvent::Configure {
                        width,
                        height,
                        maximized,
                    } => {
                        crate::ui::cpp_ui_window_action(0, if maximized { 101 } else { 100 });
                        let new_w = if width > 0 { width } else { host.window_width }.max(320);
                        let new_h = if height > 0 {
                            height
                        } else {
                            host.window_height
                        }
                        .max(240);
                        if new_w != host.window_width
                            || new_h != host.window_height
                            || maximized != last_maximized
                        {
                            host.window_width = new_w;
                            host.window_height = new_h;
                            last_maximized = maximized;
                            host.redraw();
                            let _ = host.dirty_rows.take();
                            let _ = host
                                .present
                                .present_wayland(&mut wl, &host.framebuffer, None);
                        }
                    }
                    WaylandEvent::PointerMotion { x, y } => {
                        if host.ui.is_dragging_scrollbar() {
                            let header_h = top_header_height_for(&host.session, &host.ui);
                            let content_h = (host.window_height as f32 - header_h).max(1.0);
                            let page_y = (y - header_h).clamp(0.0, content_h);
                            let ratio = (page_y / content_h).clamp(0.0, 1.0);
                            let before = host.session.scroll();
                            let after = host.session.scroll_to_ratio(ratio);
                            if (after - before).abs() > f32::EPSILON {
                                host.redraw_after_scroll(before);
                                let dirty = host.dirty_rows.take();
                                let _ =
                                    host.present
                                        .present_wayland(&mut wl, &host.framebuffer, dirty);
                            }
                            continue;
                        }
                        if let Some((sx, sy)) = host.ui.drag_select_start() {
                            let header_h = top_header_height_for(&host.session, &host.ui);
                            let py = (y - header_h).max(0.0);
                            if (x - sx).hypot(py - sy) >= 4.0 {
                                let _ = host.session.select_text_range(sx, sy, x, py);
                                host.redraw_chrome_only();
                                let dirty = host.dirty_rows.take();
                                let _ =
                                    host.present
                                        .present_wayland(&mut wl, &host.framebuffer, dirty);
                            }
                            wl.set_cursor_shape(WaylandCursorShape::Text);
                            continue;
                        }
                        let cursor_moved = host.ui.set_cursor_pos(x, y);
                        let packed_hwnd =
                            ((host.window_width as isize) << 16) | (host.window_height as isize);
                        let hit_code = crate::ui::cpp_ui_handle_nchittest(
                            packed_hwnd,
                            x as i32,
                            y as i32,
                            false,
                            false,
                        );
                        let hit = hit_test_ui_with_state_and_height(
                            &host.session,
                            &host.ui,
                            host.window_width as f32,
                            host.window_height as f32,
                            x,
                            y,
                        );
                        let hovered = match hit {
                            Some(UiHit::Page { x: px, y: py }) => host.session.hit_link(px, py),
                            _ => None,
                        };
                        let cursor_shape =
                            if let Some(resize_shape) = hit_code_to_cursor_shape(hit_code) {
                                resize_shape
                            } else if hovered.is_some()
                                || matches!(
                                    hit,
                                    Some(
                                        UiHit::Tab(_)
                                            | UiHit::CloseTab(_)
                                            | UiHit::NewTab
                                            | UiHit::TabSearchButton
                                            | UiHit::TabSearchItem(_)
                                            | UiHit::Back
                                            | UiHit::Forward
                                            | UiHit::Reload
                                            | UiHit::Home
                                            | UiHit::ClearAddress
                                            | UiHit::ToggleBookmark
                                            | UiHit::ZoomPill
                                            | UiHit::DownloadsButton
                                            | UiHit::DownloadsPopoverItem(_)
                                            | UiHit::InspectorButton
                                            | UiHit::InspectorElementRow(_)
                                            | UiHit::SettingsButton
                                            | UiHit::AppMenuButton
                                            | UiHit::AppMenuItem(_)
                                            | UiHit::ContextMenuItem(_)
                                            | UiHit::SelectDropdownOption(_)
                                            | UiHit::OmniboxSuggestion(_)
                                            | UiHit::Bookmark(_)
                                            | UiHit::BookmarkOverflow
                                            | UiHit::BookmarkOverflowMore
                                            | UiHit::SecurityBadge
                                            | UiHit::ShieldsButton
                                            | UiHit::SitePopoverItem(_)
                                            | UiHit::ReaderModeButton
                                            | UiHit::FindCaseToggle
                                            | UiHit::FindPrev
                                            | UiHit::FindNext
                                            | UiHit::FindClose
                                    )
                                )
                            {
                                WaylandCursorShape::Pointer
                            } else if matches!(
                                hit,
                                Some(
                                    UiHit::AddressField
                                        | UiHit::FindInput
                                        | UiHit::InspectorConsoleInput
                                        | UiHit::InspectorStyleInput
                                )
                            ) || matches!(
                                hit,
                                Some(UiHit::Page { x: px, y: py })
                                    if host.session.hit_text_input(px, py).is_some()
                            ) {
                                WaylandCursorShape::Text
                            } else {
                                WaylandCursorShape::Default
                            };
                        wl.set_cursor_shape(cursor_shape);

                        let in_interactive_chrome = y <= TOP_BAR_HEIGHT + 36.0
                            || matches!(hit, Some(UiHit::Scrollbar { .. }))
                            || host.ui.select_dropdown().is_some()
                            || host.ui.context_menu_pos().is_some()
                            || host.ui.is_app_menu_open()
                            || host.ui.is_site_popover_open()
                            || host.ui.is_tab_search_open()
                            || host.ui.is_downloads_popover_open()
                            || host.ui.is_find_open()
                            || host.ui.is_inspector_open();
                        if host.ui.set_hovered_link(hovered)
                            || (cursor_moved && in_interactive_chrome)
                        {
                            host.redraw_chrome_only();
                            let dirty = host.dirty_rows.take();
                            let _ = host
                                .present
                                .present_wayland(&mut wl, &host.framebuffer, dirty);
                        }
                    }
                    WaylandEvent::PointerScroll {
                        delta_y,
                        x,
                        y,
                        ctrl,
                    } => {
                        if ctrl {
                            let before = host.session.zoom_percent();
                            let after = if delta_y < 0.0 {
                                host.session.zoom_in()
                            } else {
                                host.session.zoom_out()
                            };
                            if after != before {
                                host.redraw();
                                let dirty = host.dirty_rows.take();
                                let _ =
                                    host.present
                                        .present_wayland(&mut wl, &host.framebuffer, dirty);
                            }
                        } else {
                            let before_scroll = host.session.scroll();
                            let scrolled = if let Some(UiHit::Page { x: px, y: py }) =
                                hit_test_ui_with_state_and_height(
                                    &host.session,
                                    &host.ui,
                                    host.window_width as f32,
                                    host.window_height as f32,
                                    x,
                                    y,
                                ) {
                                host.session.scroll_at(px, py, delta_y)
                            } else {
                                let after = host.session.scroll_by(delta_y);
                                (after - before_scroll).abs() > f32::EPSILON
                            };
                            if scrolled {
                                host.redraw_after_scroll(before_scroll);
                                let dirty = host.dirty_rows.take();
                                let _ =
                                    host.present
                                        .present_wayland(&mut wl, &host.framebuffer, dirty);
                            }
                        }
                    }
                    WaylandEvent::PointerButton {
                        button,
                        pressed,
                        x,
                        y,
                        ..
                    } => {
                        if button == BTN_LEFT && !pressed {
                            host.ui.stop_scrollbar_drag();
                            host.ui.stop_drag_select();
                            continue;
                        }
                        if !pressed {
                            continue;
                        }
                        if button == BTN_RIGHT {
                            let mut context_link = None;
                            if let Some(UiHit::Page { x: px, y: py }) =
                                hit_test_ui_with_state_and_height(
                                    &host.session,
                                    &host.ui,
                                    host.window_width as f32,
                                    host.window_height as f32,
                                    x,
                                    y,
                                )
                            {
                                let _ = host.session.inspect_at(px, py);
                                context_link = host.session.hit_link(px, py);
                                if context_link.is_none() && host.session.selected_text().is_empty()
                                {
                                    let _ = host.session.select_text_at(px, py);
                                }
                            }
                            host.ui.open_context_menu_with_link(x, y, context_link);
                            host.redraw();
                            let dirty = host.dirty_rows.take();
                            let _ = host
                                .present
                                .present_wayland(&mut wl, &host.framebuffer, dirty);
                        } else if button == BTN_MIDDLE {
                            let hit = hit_test_ui_with_state_and_height(
                                &host.session,
                                &host.ui,
                                host.window_width as f32,
                                host.window_height as f32,
                                x,
                                y,
                            );
                            match hit {
                                Some(UiHit::Tab(index) | UiHit::CloseTab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if host.session.close_tab(index) {
                                        host.redraw();
                                        wl.set_title(&host.window_title());
                                        let dirty = host.dirty_rows.take();
                                        let _ = host.present.present_wayland(
                                            &mut wl,
                                            &host.framebuffer,
                                            dirty,
                                        );
                                    }
                                }
                                Some(UiHit::Page { x: px, y: py }) => {
                                    if let Some(link) = host.session.hit_link(px, py) {
                                        host.ui.blur_address();
                                        host.session.blur_page_input();
                                        host.session.new_tab();
                                        host.start_navigation_x11(&link);
                                        host.redraw();
                                        wl.set_title(&host.window_title());
                                        let dirty = host.dirty_rows.take();
                                        let _ = host.present.present_wayland(
                                            &mut wl,
                                            &host.framebuffer,
                                            dirty,
                                        );
                                    }
                                }
                                _ => {}
                            }
                        } else if button == BTN_LEFT {
                            let packed_hwnd = ((host.window_width as isize) << 16)
                                | (host.window_height as isize);
                            let hit_code = crate::ui::cpp_ui_handle_nchittest(
                                packed_hwnd,
                                x as i32,
                                y as i32,
                                false,
                                false,
                            );
                            if let Some(edge) = hit_code_to_wayland_edge(hit_code) {
                                wl.begin_interactive_resize(edge);
                                continue;
                            }

                            let hit = hit_test_ui_with_state_and_height(
                                &host.session,
                                &host.ui,
                                host.window_width as f32,
                                host.window_height as f32,
                                x,
                                y,
                            );
                            let mut changed = false;
                            // A click takes keyboard focus off the toolbar.
                            if host.ui.clear_chrome_focus() {
                                changed = true;
                            }
                            if host.ui.is_editing_console()
                                && !matches!(hit, Some(UiHit::InspectorConsoleInput))
                            {
                                host.ui.blur_console();
                                changed = true;
                            }
                            if host.ui.is_editing_style()
                                && !matches!(hit, Some(UiHit::InspectorStyleInput))
                            {
                                host.ui.blur_style();
                                changed = true;
                            }
                            if host.ui.select_dropdown().is_some()
                                && !matches!(hit, Some(UiHit::SelectDropdownOption(_)))
                            {
                                host.ui.close_select_dropdown();
                                changed = true;
                            }
                            if host.ui.is_bookmark_overflow_open()
                                && !matches!(
                                    hit,
                                    Some(
                                        UiHit::Bookmark(_)
                                            | UiHit::BookmarkOverflow
                                            | UiHit::BookmarkOverflowMore
                                    )
                                )
                            {
                                host.ui.close_bookmark_overflow();
                                changed = true;
                            }
                            if host.ui.context_menu_pos().is_some()
                                && !matches!(hit, Some(UiHit::ContextMenuItem(_)))
                            {
                                host.ui.close_context_menu();
                                changed = true;
                            }
                            if host.ui.is_app_menu_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::AppMenuItem(_)) | Some(UiHit::AppMenuButton)
                                )
                            {
                                host.ui.close_app_menu();
                                changed = true;
                            }
                            if host.ui.is_site_popover_open()
                                && !matches!(
                                    hit,
                                    Some(
                                        UiHit::SecurityBadge
                                            | UiHit::ShieldsButton
                                            | UiHit::SitePopoverItem(_)
                                    )
                                )
                            {
                                host.ui.close_site_popover();
                                changed = true;
                            }
                            if host.ui.is_tab_search_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::TabSearchButton | UiHit::TabSearchItem(_))
                                )
                            {
                                host.ui.close_tab_search();
                                changed = true;
                            }
                            if host.ui.is_downloads_popover_open()
                                && !matches!(
                                    hit,
                                    Some(UiHit::DownloadsButton | UiHit::DownloadsPopoverItem(_))
                                )
                            {
                                host.ui.close_downloads_popover();
                                changed = true;
                            }
                            match hit {
                                Some(UiHit::TabSearchButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_tab_search();
                                    changed = true;
                                }
                                Some(UiHit::TabSearchItem(tab_idx)) => {
                                    host.ui.close_tab_search();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    let _ = host.session.select(tab_idx);
                                    changed = true;
                                }
                                Some(UiHit::DownloadsPopoverItem(idx)) => {
                                    let save_idx =
                                        host.session.downloads().entries().len().clamp(1, 4);
                                    host.ui.close_downloads_popover();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    if idx == save_idx {
                                        let _ = host.session.save_active_page_to_downloads();
                                        let _ = host.session.save_default_profile();
                                    } else {
                                        host.start_navigation_x11(DOWNLOADS_URL);
                                    }
                                }
                                Some(UiHit::SitePopoverItem(action)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    match action {
                                        SitePopoverAction::ToggleSiteShields => {
                                            let _ = host.session.toggle_shields_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::CycleShieldsMode => {
                                            let _ = host.session.cycle_shields_mode();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::ToggleSiteJavaScript => {
                                            let _ = host.session.toggle_scripts_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::ToggleSiteCookies => {
                                            let _ = host.session.toggle_cookies_for_active_site();
                                            let _ = host.session.save_default_profile();
                                        }
                                        SitePopoverAction::OpenSecurityCenter => {
                                            host.ui.close_site_popover();
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                    }
                                }
                                Some(UiHit::ReaderModeButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.toggle_reader_mode();
                                }
                                Some(UiHit::WindowMinimize) => {
                                    wl.minimize();
                                }
                                Some(UiHit::WindowMaximize) => {
                                    let was_max = crate::ui::cpp_ui_is_maximized(packed_hwnd);
                                    crate::ui::cpp_ui_window_action(packed_hwnd, 2);
                                    wl.set_maximized(!was_max);
                                    changed = true;
                                }
                                Some(UiHit::WindowClose) => {
                                    break 'wayland_loop;
                                }
                                Some(UiHit::TitlebarDrag) => {
                                    wl.begin_interactive_move();
                                }
                                Some(UiHit::OmniboxSuggestion(idx)) => {
                                    let target = host
                                        .ui
                                        .omnibox_suggestions(&host.session)
                                        .get(idx)
                                        .map(|s| s.url.clone());
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if let Some(url) = target {
                                        host.start_navigation_x11(&url);
                                    }
                                    changed = true;
                                }
                                Some(UiHit::InspectorConsoleInput) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.focus_console();
                                    changed = true;
                                }
                                Some(UiHit::InspectorStyleInput) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.focus_style();
                                    changed = true;
                                }
                                Some(UiHit::SelectDropdownOption(option_idx)) => {
                                    let select_id = host.ui.select_dropdown().map(|d| d.node_id);
                                    host.ui.close_select_dropdown();
                                    if let Some(id) = select_id {
                                        changed = host
                                            .session
                                            .choose_select_option(id, option_idx)
                                            .is_some();
                                    } else {
                                        changed = true;
                                    }
                                }
                                Some(UiHit::AppMenuButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_app_menu();
                                    changed = true;
                                }
                                Some(UiHit::AppMenuItem(action)) => {
                                    host.ui.close_app_menu();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = true;
                                    match action {
                                        AppMenuAction::NewTab => {
                                            host.session.new_tab();
                                            let _ = host.session.open_target(HOME_URL);
                                        }
                                        AppMenuAction::TabSearch => {
                                            host.ui.open_tab_search();
                                        }
                                        AppMenuAction::HomeDashboard => {
                                            host.start_navigation_x11(HOME_URL);
                                        }
                                        AppMenuAction::Downloads => {
                                            host.start_navigation_x11(DOWNLOADS_URL);
                                        }
                                        AppMenuAction::History => {
                                            host.start_navigation_x11(HISTORY_URL);
                                        }
                                        AppMenuAction::SecurityCenter => {
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                        AppMenuAction::ToggleReaderMode => {
                                            let _ = host.session.toggle_reader_mode();
                                        }
                                        AppMenuAction::ToggleBookmarksBar => {
                                            host.ui.toggle_bookmarks_bar();
                                        }
                                        AppMenuAction::FindInPage => {
                                            host.ui.open_find();
                                        }
                                        AppMenuAction::ToggleInspectorDock => {
                                            host.ui.toggle_inspector();
                                        }
                                        AppMenuAction::OpenInspectorPage => {
                                            host.start_navigation_x11(INSPECT_URL);
                                        }
                                        AppMenuAction::CycleZoom => {
                                            if host.session.zoom_percent() != 100 {
                                                let _ = host.session.reset_zoom();
                                            } else {
                                                let _ = host.session.zoom_in();
                                            }
                                        }
                                        AppMenuAction::CycleTheme => {
                                            host.session.cycle_theme();
                                            let url = host.current_url();
                                            if url.starts_with("anvil://") {
                                                let _ = host.session.open_target(&url);
                                            }
                                        }
                                        AppMenuAction::Settings => {
                                            host.start_navigation_x11(SETTINGS_URL);
                                        }
                                    }
                                }
                                Some(UiHit::DownloadsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_downloads_popover();
                                    changed = true;
                                }
                                Some(UiHit::InspectorButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_inspector();
                                    changed = true;
                                }
                                Some(UiHit::InspectorTab(tab)) => {
                                    host.ui.set_inspector_tab(tab);
                                    changed = true;
                                }
                                Some(UiHit::InspectorElementRow(node_id)) => {
                                    host.session.select_inspected_node(node_id);
                                    changed = true;
                                }
                                Some(UiHit::InspectorFullPage) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(INSPECT_URL);
                                    changed = true;
                                }
                                Some(UiHit::InspectorClose) => {
                                    host.ui.close_inspector();
                                    changed = true;
                                }
                                Some(UiHit::ContextMenuItem(action)) => {
                                    let context_link =
                                        host.ui.context_menu_link().map(str::to_string);
                                    host.ui.close_context_menu();
                                    changed = true;
                                    match action {
                                        ContextMenuAction::OpenLinkInNewTab => {
                                            if let Some(link) = context_link {
                                                host.ui.blur_address();
                                                host.session.blur_page_input();
                                                host.session.new_tab();
                                                host.start_navigation_x11(&link);
                                            }
                                        }
                                        ContextMenuAction::CopyLinkAddress => {
                                            if let Some(link) = context_link {
                                                host.session.set_clipboard_text(&link);
                                                wl.set_clipboard_text(&link);
                                            }
                                        }
                                        ContextMenuAction::Back => {
                                            let _ = host.history_back();
                                        }
                                        ContextMenuAction::Forward => {
                                            let _ = host.history_forward();
                                        }
                                        ContextMenuAction::Reload => {
                                            let url = host.current_url();
                                            if !url.is_empty() {
                                                host.start_navigation_x11(&url);
                                            }
                                        }
                                        ContextMenuAction::ReopenClosedTab => {
                                            let _ = host.session.reopen_closed_tab();
                                        }
                                        ContextMenuAction::DuplicateTab => {
                                            let active = host.session.active_index();
                                            let _ = host.session.duplicate_tab(active);
                                        }
                                        ContextMenuAction::TogglePinTab => {
                                            let active = host.session.active_index();
                                            let _ = host.session.toggle_pin_tab(active);
                                        }
                                        ContextMenuAction::CloseOtherTabs => {
                                            let active = host.session.active_index();
                                            let _ = host.session.close_other_tabs(active);
                                        }
                                        ContextMenuAction::Copy => {
                                            if let Some(copied) =
                                                host.session.copy_selection_or_focused()
                                            {
                                                wl.set_clipboard_text(&copied);
                                            }
                                        }
                                        ContextMenuAction::Paste => {
                                            if let Some(sys_clip) = wl.read_clipboard_text()
                                                && !sys_clip.is_empty()
                                            {
                                                host.session.set_clipboard_text(&sys_clip);
                                            }
                                            let clip = host.session.clipboard_text().to_string();
                                            let _ = host.session.paste_into_focused_input(&clip);
                                        }
                                        ContextMenuAction::FindInPage => {
                                            host.session.blur_page_input();
                                            host.ui.open_find();
                                        }
                                        ContextMenuAction::SavePage => {
                                            if host
                                                .session
                                                .save_active_page_to_downloads()
                                                .is_some()
                                            {
                                                host.start_navigation_x11(DOWNLOADS_URL);
                                            }
                                        }
                                        ContextMenuAction::SavePdf => {
                                            if host.session.save_active_page_as_pdf().is_some() {
                                                host.start_navigation_x11(DOWNLOADS_URL);
                                            }
                                        }
                                        ContextMenuAction::ViewPageSource => {
                                            let _ = host.session.view_active_page_source();
                                        }
                                        ContextMenuAction::ToggleReaderMode => {
                                            let _ = host.session.toggle_reader_mode();
                                        }
                                        ContextMenuAction::ResetZoom => {
                                            let _ = host.session.reset_zoom();
                                        }
                                        ContextMenuAction::InspectPage => {
                                            host.ui.toggle_inspector();
                                        }
                                        ContextMenuAction::SecurityCenter => {
                                            host.start_navigation_x11(SECURITY_URL);
                                        }
                                    }
                                }
                                Some(UiHit::FindInput) => {
                                    host.session.blur_page_input();
                                    host.ui.open_find();
                                    changed = true;
                                }
                                Some(UiHit::FindCaseToggle) => {
                                    host.ui.toggle_find_case_sensitive();
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindPrev) => {
                                    let count = host.find_match_count();
                                    host.ui.find_prev(count);
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindNext) => {
                                    let count = host.find_match_count();
                                    host.ui.find_next(count);
                                    host.scroll_to_active_find_match();
                                    changed = true;
                                }
                                Some(UiHit::FindClose) => {
                                    host.ui.close_find();
                                    changed = true;
                                }
                                Some(UiHit::Tab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.select(index);
                                }
                                Some(UiHit::CloseTab(index)) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.close_tab(index);
                                }
                                Some(UiHit::NewTab) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.session.new_tab();
                                    let _ = host.session.open_target(HOME_URL);
                                    changed = true;
                                }
                                Some(UiHit::Back) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.history_back();
                                }
                                Some(UiHit::Forward) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.history_forward();
                                }
                                Some(UiHit::Reload) => {
                                    if host.ui.is_loading() {
                                        host.stop_loading();
                                        changed = true;
                                    } else {
                                        host.ui.blur_address();
                                        host.session.blur_page_input();
                                        let url = host.current_url();
                                        if !url.is_empty() {
                                            host.start_navigation_x11(&url);
                                            changed = true;
                                        }
                                    }
                                }
                                Some(UiHit::Home) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(HOME_URL);
                                    changed = true;
                                }
                                Some(UiHit::SecurityBadge | UiHit::ShieldsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.ui.toggle_site_popover();
                                    changed = true;
                                }
                                Some(UiHit::AddressField) => {
                                    host.session.blur_page_input();
                                    let url = host.current_url();
                                    host.ui.focus_address(&url);
                                    changed = true;
                                }
                                Some(UiHit::ClearAddress) => {
                                    host.ui.clear_address();
                                    changed = true;
                                }
                                Some(UiHit::ToggleBookmark) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.session.toggle_bookmark_active();
                                    let _ = host.session.save_default_profile();
                                    changed = true;
                                }
                                Some(UiHit::ZoomPill) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if host.session.zoom_percent() != 100 {
                                        let _ = host.session.reset_zoom();
                                    } else {
                                        let _ = host.session.zoom_in();
                                    }
                                    changed = true;
                                }
                                Some(UiHit::SettingsButton) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(SETTINGS_URL);
                                    changed = true;
                                }
                                Some(UiHit::BookmarkOverflow) => {
                                    host.ui.toggle_bookmark_overflow();
                                    changed = true;
                                }
                                Some(UiHit::BookmarkOverflowMore) => {
                                    host.ui.next_bookmark_overflow(
                                        host.session.bookmarks().len(),
                                        crate::ui::bookmark_bar_visible_count(
                                            &host.session,
                                            host.window_width as f32,
                                        ),
                                    );
                                    changed = true;
                                }
                                Some(UiHit::Bookmark(index)) => {
                                    host.ui.close_bookmark_overflow();
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    if let Some(bm) = host.session.bookmarks().get(index).cloned() {
                                        host.start_navigation_x11(&bm.url);
                                        changed = true;
                                    }
                                }
                                Some(UiHit::Scrollbar { ratio }) => {
                                    host.ui.blur_address();
                                    host.ui.start_scrollbar_drag();
                                    let before = host.session.scroll();
                                    let after = host.session.scroll_to_ratio(ratio);
                                    changed = (after - before).abs() > f32::EPSILON;
                                }
                                Some(UiHit::Page { x: px, y: py }) => {
                                    if host.ui.is_editing_address() {
                                        host.ui.blur_address();
                                    }
                                    match host.session.click_at(px, py) {
                                        ClickOutcome::SelectChanged(node_id, _) => {
                                            if let Some((rect, options)) =
                                                host.session.select_dropdown_info(node_id)
                                                && options.len() > 1
                                            {
                                                host.ui
                                                    .open_select_dropdown(node_id, rect, options);
                                            }
                                            changed = true;
                                        }
                                        ClickOutcome::ScriptHandled
                                        | ClickOutcome::InputFocused(_)
                                        | ClickOutcome::CheckboxToggled(..)
                                        | ClickOutcome::VideoToggled(..)
                                        | ClickOutcome::VideoSeeked(..) => {
                                            changed = true;
                                        }
                                        ClickOutcome::Navigate(target) => {
                                            host.start_navigation_x11(&target);
                                            changed = true;
                                        }
                                        ClickOutcome::SubmitPost { url, body } => {
                                            host.start_post_navigation_x11(&url, &body);
                                            changed = true;
                                        }
                                        ClickOutcome::None => {
                                            host.session.clear_selection();
                                            match host.ui.record_page_click(px, py) {
                                                2 => {
                                                    let _ = host.session.select_word_at(px, py);
                                                }
                                                3 => {
                                                    let _ = host.session.select_line_at(px, py);
                                                }
                                                _ => {
                                                    let _ = host.session.select_text_at(px, py);
                                                    host.ui.start_drag_select(px, py);
                                                }
                                            }
                                            let _ = host.session.inspect_at(px, py);
                                            changed = true;
                                        }
                                    }
                                }
                                None => {
                                    if host.ui.is_editing_address() {
                                        host.ui.blur_address();
                                        changed = true;
                                    }
                                    if host.session.blur_page_input() {
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                host.redraw();
                                wl.set_title(&host.window_title());
                                let dirty = host.dirty_rows.take();
                                let _ =
                                    host.present
                                        .present_wayland(&mut wl, &host.framebuffer, dirty);
                            }
                        }
                    }
                    WaylandEvent::KeyPress { key, mods } => {
                        let focus = focus_of(&host.session, &host.ui);
                        let command = command_for(key, mods, focus);
                        if matches!(command, KeyCommand::Paste)
                            && let Some(sys_clip) = wl.read_clipboard_text()
                            && !sys_clip.is_empty()
                        {
                            host.session.set_clipboard_text(&sys_clip);
                        }
                        let is_copy_or_cut = matches!(command, KeyCommand::Copy | KeyCommand::Cut);
                        if matches!(
                            command,
                            crate::keys::KeyCommand::Back | crate::keys::KeyCommand::Forward
                        ) {
                            host.stop_loading();
                        }
                        let effect = apply_key_command(&mut host.session, &mut host.ui, command);
                        if is_copy_or_cut {
                            let clip = host.session.clipboard_text();
                            if !clip.is_empty() {
                                wl.set_clipboard_text(clip);
                            }
                        }
                        match effect {
                            KeyEffect::Nothing => continue,
                            KeyEffect::CloseWindow => break 'wayland_loop,
                            KeyEffect::Redraw => {}
                            KeyEffect::StopLoading => host.stop_loading(),
                            KeyEffect::Navigate(target) => host.start_navigation_x11(&target),
                            KeyEffect::Post { url, body } => {
                                host.start_post_navigation_x11(&url, &body)
                            }
                        }
                        host.redraw();
                        wl.set_title(&host.window_title());
                        let dirty = host.dirty_rows.take();
                        let _ = host
                            .present
                            .present_wayland(&mut wl, &host.framebuffer, dirty);
                    }
                }
            }
        }

        let _ = host.session.save_default_profile();
        Ok(())
    }
}

/// Apply the process sandbox check, then run the Win32 or X11 loop.
/// The viewport is the window size. content_viewport derives the page size from it.
pub fn open(initial_target: Option<&str>, initial_viewport: Viewport) -> Result<(), String> {
    let _mandatory_sandbox = crate::enforce_mandatory_sandbox_or_abort();
    imp::open(initial_target, initial_viewport)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn host_with(html: &str) -> Host {
        let mut session = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        let ui = UiState::new();
        let framebuffer = crate::ui::compose_frame(&session, &ui, 320, 300);
        session.open_html(html, "https://example.com/");
        Host {
            session,
            ui,
            window_width: 320,
            window_height: 300,
            framebuffer,
            cached_page_fb: None,
            pending_load: Arc::new(Mutex::new(super::LoadQueue::default())),
            animating: false,
            check_animation: true,
            next_animation_frame: Instant::now(),
            dirty_rows: None,
            present: crate::present::PresentPath::Untried,
        }
    }

    #[test]
    fn a_still_page_draws_no_animation_frames() {
        let mut host = host_with("<p>still</p>");
        assert!(!host.animation_step());
        assert!(!host.animating);
        assert!(!host.check_animation);
        // Without a new event nothing asks the page again.
        assert!(!host.animation_step());
    }

    #[test]
    fn a_running_animation_is_drawn_until_it_ends() {
        let mut host = host_with(
            "<style>@keyframes a { from { width: 0px } to { width: 50px } } \
             div { height: 10px; animation-name: a; animation-duration: 1500ms; }</style><div></div>",
        );
        assert!(host.animation_step(), "first frame");
        assert!(host.animating);
        // No frame before the next one is due.
        host.next_animation_frame = Instant::now() + Duration::from_secs(60);
        assert!(!host.animation_step());
        host.next_animation_frame = Instant::now();
        std::thread::sleep(Duration::from_millis(1600));
        assert!(host.animation_step(), "the last frame draws the end state");
        assert!(!host.animating);
        std::thread::sleep(ANIMATION_FRAME);
        assert!(!host.animation_step());
    }
}

#[cfg(test)]
mod load_queue_tests {
    use super::*;
    #[test]
    fn stop_and_new_navigation_reject_stale_results_without_affecting_siblings() {
        let mut q = LoadQueue::default();
        let a = q.begin(1);
        let b = q.begin(2);
        q.cancel(1);
        q.finish(1, a, Err("old".into()));
        assert!(q.take().is_none());
        assert!(q.is_loading(2));
        let newer = q.begin(1);
        q.finish(1, a, Err("old".into()));
        q.finish(2, b, Err("sibling".into()));
        assert_eq!(q.take().unwrap().0, 2);
        q.finish(1, newer, Err("new".into()));
        assert_eq!(q.take().unwrap().0, 1);
        assert!(q.take().is_none());
        let done = q.begin(1);
        q.finish(1, done, Err("queued".into()));
        q.cancel(1);
        assert!(q.take().is_none());
    }
}
