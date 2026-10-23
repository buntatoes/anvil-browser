//! Cross-platform native window shell (`Win32` on Windows, `X11` on Linux) that
//! presents the framebuffer and dispatches mouse, scroll, and keyboard input.

use std::sync::{Arc, Mutex};

use flux_raster::Framebuffer;

use crate::frontend::{HOME_URL, SECURITY_URL};
use crate::ui::{UiHit, UiState, compose_frame, content_viewport, hit_test_ui};
use crate::{ClickOutcome, Page, Session, Viewport, load_post_target, load_target};

type PendingLoad = Arc<Mutex<Option<(usize, Result<Page, String>)>>>;

struct Host {
    session: Session,
    ui: UiState,
    window_width: u32,
    window_height: u32,
    framebuffer: Framebuffer,
    pending_load: PendingLoad,
}

impl Host {
    fn redraw(&mut self) {
        self.framebuffer = compose_frame(
            &self.session,
            &self.ui,
            self.window_width,
            self.window_height,
        );
    }

    fn current_url(&self) -> String {
        self.session
            .active()
            .current()
            .map(|page| page.url.clone())
            .unwrap_or_default()
    }

    fn window_title(&self) -> String {
        let Some(page) = self.session.active().current() else {
            return "Flux".to_string();
        };
        if page.title.is_empty() {
            format!("Flux - {}", page.url)
        } else {
            format!("{} - Flux", page.title)
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::{
        ClickOutcome, HOME_URL, Host, SECURITY_URL, Session, UiHit, UiState, Viewport,
        compose_frame, content_viewport, hit_test_ui, load_post_target, load_target,
    };

    type Hwnd = *mut c_void;
    type Hdc = *mut c_void;
    type Hinstance = *mut c_void;
    type Hicon = *mut c_void;
    type Hcursor = *mut c_void;
    type Hbrush = *mut c_void;
    type Hmenu = *mut c_void;
    type Lresult = isize;
    type Wparam = usize;
    type Lparam = isize;

    const CS_VREDRAW: u32 = 0x0001;
    const CS_HREDRAW: u32 = 0x0002;
    const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const CW_USEDEFAULT: i32 = -2_147_483_648;
    const GWLP_USERDATA: i32 = -21;

    const WM_DESTROY: u32 = 0x0002;
    const WM_SIZE: u32 = 0x0005;
    const WM_PAINT: u32 = 0x000F;
    const WM_NCCREATE: u32 = 0x0081;
    const WM_NCDESTROY: u32 = 0x0082;
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_CHAR: u32 = 0x0102;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_MOUSEWHEEL: u32 = 0x020A;
    const WM_FLUX_PAGE_READY: u32 = 0x8001;

    const VK_BACK: usize = 0x08;
    const VK_RETURN: usize = 0x0D;
    const VK_CONTROL: i32 = 0x11;
    const VK_MENU: i32 = 0x12;
    const VK_ESCAPE: usize = 0x1B;
    const VK_PRIOR: usize = 0x21;
    const VK_NEXT: usize = 0x22;
    const VK_END: usize = 0x23;
    const VK_HOME: usize = 0x24;
    const VK_LEFT: usize = 0x25;
    const VK_UP: usize = 0x26;
    const VK_RIGHT: usize = 0x27;
    const VK_DOWN: usize = 0x28;
    const VK_KEY_L: usize = 0x4C;
    const VK_KEY_R: usize = 0x52;
    const VK_KEY_T: usize = 0x54;
    const VK_KEY_W: usize = 0x57;
    const VK_F5: usize = 0x74;
    const VK_F6: usize = 0x75;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

    const IDC_ARROW: *const u16 = 32512 as *const u16;
    const IDC_IBEAM: *const u16 = 32513 as *const u16;
    const IDC_HAND: *const u16 = 32649 as *const u16;
    const BI_RGB: u32 = 0;
    const DIB_RGB_COLORS: u32 = 0;

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

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }

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

    #[repr(C)]
    #[derive(Default)]
    struct WinRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct PaintStruct {
        hdc: Hdc,
        f_erase: i32,
        rc_paint: WinRect,
        f_restore: i32,
        f_inc_update: i32,
        rgb_reserved: [u8; 32],
    }

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

    #[repr(C)]
    struct BitmapInfo {
        bmi_header: BitmapInfoHeader,
        bmi_colors: [u32; 1],
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(lp_wnd_class: *const WndClassW) -> u16;
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
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
        fn GetMessageW(
            lp_msg: *mut Msg,
            h_wnd: Hwnd,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> i32;
        fn TranslateMessage(lp_msg: *const Msg) -> i32;
        fn DispatchMessageW(lp_msg: *const Msg) -> Lresult;
        fn PostQuitMessage(n_exit_code: i32);
        fn PostMessageW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> i32;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn BeginPaint(hwnd: Hwnd, lp_paint: *mut PaintStruct) -> Hdc;
        fn EndPaint(hwnd: Hwnd, lp_paint: *const PaintStruct) -> i32;
        fn InvalidateRect(hwnd: Hwnd, lp_rect: *const WinRect, b_erase: i32) -> i32;
        fn GetClientRect(hwnd: Hwnd, lp_rect: *mut WinRect) -> i32;
        fn SetWindowTextW(hwnd: Hwnd, lp_string: *const u16) -> i32;
        fn SetWindowLongPtrW(hwnd: Hwnd, n_index: i32, dw_new_long: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: Hwnd, n_index: i32) -> isize;
        fn LoadCursorW(h_instance: Hinstance, lp_cursor_name: *const u16) -> Hcursor;
        fn SetCursor(h_cursor: Hcursor) -> Hcursor;
        fn GetKeyState(n_virt_key: i32) -> i16;
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: Hwnd,
            dw_attribute: u32,
            pv_attribute: *const c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
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

    impl Host {
        fn start_navigation(&mut self, hwnd: Hwnd, target: &str) {
            if target.starts_with("http://") || target.starts_with("https://") {
                let url =
                    if self.session.settings().https_only_mode && target.starts_with("http://") {
                        format!("https://{}", &target["http://".len()..])
                    } else {
                        target.to_string()
                    };
                let viewport = self.session.viewport();
                let tab_index = self.session.active_index();
                let slot = Arc::clone(&self.pending_load);
                let hwnd_addr = hwnd as usize;
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let result = load_target(&url, viewport);
                    if let Ok(mut guard) = slot.lock() {
                        *guard = Some((tab_index, result));
                    }
                    unsafe {
                        PostMessageW(hwnd_addr as Hwnd, WM_FLUX_PAGE_READY, 0, 0);
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }

        fn start_post_navigation(&mut self, hwnd: Hwnd, target: &str, body: &str) {
            if target.starts_with("http://") || target.starts_with("https://") {
                let url = target.to_string();
                let payload = body.to_string();
                let viewport = self.session.viewport();
                let tab_index = self.session.active_index();
                let slot = Arc::clone(&self.pending_load);
                let hwnd_addr = hwnd as usize;
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let result = load_post_target(&url, &payload, viewport);
                    if let Ok(mut guard) = slot.lock() {
                        *guard = Some((tab_index, result));
                    }
                    unsafe {
                        PostMessageW(hwnd_addr as Hwnd, WM_FLUX_PAGE_READY, 0, 0);
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }
    }

    pub fn open(target: Option<&str>, window_size: Viewport) -> Result<(), String> {
        let win_w = (window_size.width.round() as u32).max(480);
        let win_h = (window_size.height.round() as u32).max(320);
        let mut session = Session::new(content_viewport(win_w as f32, win_h as f32));
        let ui = UiState::new();

        let initial_target = target.unwrap_or(HOME_URL);
        session.open_target(initial_target)?;

        let framebuffer = compose_frame(&session, &ui, win_w, win_h);
        let host = Box::new(Host {
            session,
            ui,
            window_width: win_w,
            window_height: win_h,
            framebuffer,
            pending_load: Arc::new(Mutex::new(None)),
        });

        let class_name = wide("FluxWindow");
        let title = wide(&host.window_title());

        unsafe {
            let wc = WndClassW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfn_wnd_proc: wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: null_mut(),
                h_icon: null_mut(),
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
                drop(Box::from_raw(raw_host));
                return Err("could not create the browser window".to_string());
            }

            let dark_mode: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark_mode).cast::<c_void>(),
                size_of::<i32>() as u32,
            );

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

    unsafe extern "system" fn wnd_proc(
        hwnd: Hwnd,
        msg: u32,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult {
        match msg {
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
            WM_FLUX_PAGE_READY => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let finished = host
                        .pending_load
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    host.ui.set_loading(false);
                    if let Some((tab_idx, Ok(page))) = finished {
                        host.session.open_page_in_tab(tab_idx, page);
                    }
                    refresh_window(hwnd, host);
                }
                0
            }
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
                        host.session
                            .set_viewport(content_viewport(width as f32, height as f32));
                        host.redraw();
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let (x, y) = mouse_coords(l_param);
                    let hit = hit_test_ui(&host.session, host.window_width as f32, x, y);
                    let mut hovered = None;
                    let cursor = match hit {
                        Some(UiHit::AddressField) => IDC_IBEAM,
                        Some(
                            UiHit::Tab(_)
                            | UiHit::CloseTab(_)
                            | UiHit::NewTab
                            | UiHit::Back
                            | UiHit::Forward
                            | UiHit::Reload
                            | UiHit::Home
                            | UiHit::SecurityBadge
                            | UiHit::ToggleBookmark
                            | UiHit::Bookmark(_)
                            | UiHit::Scrollbar { .. },
                        ) => IDC_HAND,
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
                    if host.ui.set_hovered_link(hovered) {
                        host.redraw();
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                    unsafe {
                        SetCursor(LoadCursorW(null_mut(), cursor));
                    }
                }
                0
            }
            WM_LBUTTONDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let (x, y) = mouse_coords(l_param);
                    let hit = hit_test_ui(&host.session, host.window_width as f32, x, y);
                    let mut changed = false;
                    match hit {
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
                            changed = host.session.back();
                        }
                        Some(UiHit::Forward) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            changed = host.session.forward();
                        }
                        Some(UiHit::Reload) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            let url = host.current_url();
                            if !url.is_empty() {
                                host.start_navigation(hwnd, &url);
                                changed = true;
                            }
                        }
                        Some(UiHit::Home) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.start_navigation(hwnd, HOME_URL);
                            changed = true;
                        }
                        Some(UiHit::SecurityBadge) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.start_navigation(hwnd, SECURITY_URL);
                            changed = true;
                        }
                        Some(UiHit::AddressField) => {
                            host.session.blur_page_input();
                            let url = host.current_url();
                            host.ui.focus_address(&url);
                            changed = true;
                        }
                        Some(UiHit::ToggleBookmark) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            host.session.toggle_bookmark_active();
                            changed = true;
                        }
                        Some(UiHit::Bookmark(index)) => {
                            host.ui.blur_address();
                            host.session.blur_page_input();
                            if let Some(bm) = host.session.bookmarks().get(index).cloned() {
                                host.start_navigation(hwnd, &bm.url);
                                changed = true;
                            }
                        }
                        Some(UiHit::Scrollbar { ratio }) => {
                            host.ui.blur_address();
                            let before = host.session.scroll();
                            let after = host.session.scroll_to_ratio(ratio);
                            changed = (after - before).abs() > f32::EPSILON;
                        }
                        Some(UiHit::Page { x: px, y: py }) => {
                            if host.ui.is_editing_address() {
                                host.ui.blur_address();
                                changed = true;
                            }
                            match host.session.click_at(px, py) {
                                ClickOutcome::ScriptHandled
                                | ClickOutcome::InputFocused(_)
                                | ClickOutcome::CheckboxToggled(..)
                                | ClickOutcome::SelectChanged(..) => {
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
                                ClickOutcome::None => {}
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
                        refresh_window(hwnd, host);
                    }
                }
                0
            }
            WM_MOUSEWHEEL => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let delta = ((w_param >> 16) as i16) as f32;
                    let before = host.session.scroll();
                    let after = host.session.scroll_by(-(delta / 120.0) * 48.0);
                    if (after - before).abs() > f32::EPSILON {
                        host.redraw();
                        unsafe {
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
                }
                0
            }
            WM_CHAR => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    if host.ui.is_editing_address() {
                        if let Some(ch) = char::from_u32(w_param as u32) {
                            if (' '..='~').contains(&ch) {
                                host.ui.push_char(ch);
                                refresh_window(hwnd, host);
                            }
                        }
                    } else if host.session.has_focused_input() {
                        if let Some(ch) = char::from_u32(w_param as u32) {
                            if host.session.type_into_focused_input(ch) {
                                refresh_window(hwnd, host);
                            }
                        }
                    }
                }
                0
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if let Some(host) = unsafe { host_mut(hwnd) } {
                    let ctrl_down = unsafe { (GetKeyState(VK_CONTROL) as u16 & 0x8000) != 0 };
                    let alt_down = unsafe { (GetKeyState(VK_MENU) as u16 & 0x8000) != 0 };

                    if ctrl_down {
                        let changed = match w_param {
                            VK_KEY_T => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                host.session.new_tab();
                                let _ = host.session.open_target(HOME_URL);
                                true
                            }
                            VK_KEY_W => {
                                host.ui.blur_address();
                                host.session.blur_page_input();
                                let active = host.session.active_index();
                                host.session.close_tab(active)
                            }
                            VK_KEY_L => {
                                host.session.blur_page_input();
                                let url = host.current_url();
                                host.ui.focus_address(&url);
                                true
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

                    if alt_down {
                        let changed = match w_param {
                            VK_LEFT => host.session.back(),
                            VK_RIGHT => host.session.forward(),
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
                            VK_ESCAPE => {
                                host.ui.blur_address();
                                refresh_window(hwnd, host);
                            }
                            VK_BACK => {
                                host.ui.backspace();
                                refresh_window(hwnd, host);
                            }
                            VK_RETURN => {
                                if let Some(target) = host.ui.submit_address() {
                                    host.start_navigation(hwnd, &target);
                                }
                                refresh_window(hwnd, host);
                            }
                            _ => {}
                        }
                        return 0;
                    }

                    if host.session.has_focused_input() {
                        match w_param {
                            VK_ESCAPE => {
                                if host.session.blur_page_input() {
                                    refresh_window(hwnd, host);
                                }
                            }
                            VK_BACK => {
                                if host.session.backspace_focused_input() {
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

                    if w_param == VK_ESCAPE {
                        unsafe {
                            DestroyWindow(hwnd);
                        }
                        return 0;
                    }

                    let changed = match w_param {
                        VK_F6 => {
                            let url = host.current_url();
                            host.ui.focus_address(&url);
                            true
                        }
                        VK_F5 => {
                            let url = host.current_url();
                            if !url.is_empty() {
                                host.start_navigation(hwnd, &url);
                                true
                            } else {
                                false
                            }
                        }
                        VK_BACK => host.session.back(),
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
                unsafe {
                    EndPaint(hwnd, &ps);
                }
                0
            }
            WM_DESTROY => {
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
        }
    }

    fn refresh_window(hwnd: Hwnd, host: &mut Host) {
        host.redraw();
        let title = wide(&host.window_title());
        unsafe {
            SetWindowTextW(hwnd, title.as_ptr());
            InvalidateRect(hwnd, null(), 0);
        }
    }

    unsafe fn host_mut<'a>(hwnd: Hwnd) -> Option<&'a mut Host> {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Host;
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *ptr })
        }
    }

    fn mouse_coords(l_param: Lparam) -> (f32, f32) {
        let x = (l_param & 0xffff) as i16 as f32;
        let y = ((l_param >> 16) & 0xffff) as i16 as f32;
        (x, y)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{
        ClickOutcome, HOME_URL, Host, SECURITY_URL, Session, UiHit, UiState, Viewport,
        compose_frame, content_viewport, hit_test_ui, load_post_target, load_target,
    };

    type Display = c_void;
    type Window = c_ulong;
    type GC = *mut c_void;
    type Visual = c_void;
    type Atom = c_ulong;
    type KeySym = c_ulong;

    const EXPOSURE_MASK: c_long = 1 << 15;
    const KEY_PRESS_MASK: c_long = 1 << 0;
    const BUTTON_PRESS_MASK: c_long = 1 << 2;
    const POINTER_MOTION_MASK: c_long = 1 << 6;
    const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

    const KEY_PRESS: c_int = 2;
    const BUTTON_PRESS: c_int = 4;
    const MOTION_NOTIFY: c_int = 6;
    const EXPOSE: c_int = 12;
    const CONFIGURE_NOTIFY: c_int = 22;
    const CLIENT_MESSAGE: c_int = 33;

    const Z_PIXMAP: c_int = 2;

    const XK_BACKSPACE: KeySym = 0xff08;
    const XK_RETURN: KeySym = 0xff0d;
    const XK_ESCAPE: KeySym = 0xff1b;
    const XK_HOME: KeySym = 0xff50;
    const XK_LEFT: KeySym = 0xff51;
    const XK_UP: KeySym = 0xff52;
    const XK_RIGHT: KeySym = 0xff53;
    const XK_DOWN: KeySym = 0xff54;
    const XK_PAGE_UP: KeySym = 0xff55;
    const XK_PAGE_DOWN: KeySym = 0xff56;
    const XK_END: KeySym = 0xff57;
    const XK_F5: KeySym = 0xffc2;
    const XK_F6: KeySym = 0xffc3;

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
        fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
        fn XCloseDisplay(display: *mut Display) -> c_int;
        fn XDefaultScreen(display: *mut Display) -> c_int;
        fn XRootWindow(display: *mut Display, screen_number: c_int) -> Window;
        fn XDefaultVisual(display: *mut Display, screen_number: c_int) -> *mut Visual;
        fn XDefaultDepth(display: *mut Display, screen_number: c_int) -> c_int;
        fn XDefaultGC(display: *mut Display, screen_number: c_int) -> GC;
        fn XBlackPixel(display: *mut Display, screen_number: c_int) -> c_ulong;
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
        fn XStoreName(display: *mut Display, w: Window, window_name: *const c_char) -> c_int;
        fn XSelectInput(display: *mut Display, w: Window, event_mask: c_long) -> c_int;
        fn XMapWindow(display: *mut Display, w: Window) -> c_int;
        fn XDestroyWindow(display: *mut Display, w: Window) -> c_int;
        fn XInternAtom(
            display: *mut Display,
            atom_name: *const c_char,
            only_if_exists: c_int,
        ) -> Atom;
        fn XSetWMProtocols(
            display: *mut Display,
            w: Window,
            protocols: *mut Atom,
            count: c_int,
        ) -> c_int;
        fn XPending(display: *mut Display) -> c_int;
        fn XNextEvent(display: *mut Display, event_return: *mut XEvent) -> c_int;
        fn XLookupString(
            event_struct: *mut XKeyEvent,
            buffer_return: *mut c_char,
            bytes_buffer: c_int,
            keysym_return: *mut KeySym,
            status_in_out: *mut c_void,
        ) -> c_int;
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
        fn XFlush(display: *mut Display) -> c_int;
    }

    impl Host {
        fn start_navigation_x11(&mut self, target: &str) {
            if target.starts_with("http://") || target.starts_with("https://") {
                let url =
                    if self.session.settings().https_only_mode && target.starts_with("http://") {
                        format!("https://{}", &target["http://".len()..])
                    } else {
                        target.to_string()
                    };
                let viewport = self.session.viewport();
                let tab_index = self.session.active_index();
                let slot = Arc::clone(&self.pending_load);
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let result = load_target(&url, viewport);
                    if let Ok(mut guard) = slot.lock() {
                        *guard = Some((tab_index, result));
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }

        fn start_post_navigation_x11(&mut self, target: &str, body: &str) {
            if target.starts_with("http://") || target.starts_with("https://") {
                let url = target.to_string();
                let payload = body.to_string();
                let viewport = self.session.viewport();
                let tab_index = self.session.active_index();
                let slot = Arc::clone(&self.pending_load);
                self.ui.set_loading(true);
                thread::spawn(move || {
                    let result = load_post_target(&url, &payload, viewport);
                    if let Ok(mut guard) = slot.lock() {
                        *guard = Some((tab_index, result));
                    }
                });
            } else {
                let _ = self.session.open_target(target);
            }
        }
    }

    pub fn open(target: Option<&str>, window_size: Viewport) -> Result<(), String> {
        let win_w = (window_size.width.round() as u32).max(480);
        let win_h = (window_size.height.round() as u32).max(320);
        let mut session = Session::new(content_viewport(win_w as f32, win_h as f32));
        let ui = UiState::new();

        let initial_target = target.unwrap_or(HOME_URL);
        session.open_target(initial_target)?;

        let framebuffer = compose_frame(&session, &ui, win_w, win_h);
        let mut host = Host {
            session,
            ui,
            window_width: win_w,
            window_height: win_h,
            framebuffer,
            pending_load: Arc::new(Mutex::new(None)),
        };

        unsafe {
            let display = XOpenDisplay(null());
            if display.is_null() {
                return Err("could not connect to X11 display server".to_string());
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
            present_x11(display, screen, window, &host);

            loop {
                if let Some((tab_idx, res)) =
                    host.pending_load.lock().ok().and_then(|mut g| g.take())
                {
                    host.ui.set_loading(false);
                    if let Ok(page) = res {
                        host.session.open_page_in_tab(tab_idx, page);
                    }
                    host.redraw();
                    set_x11_title(display, window, &host.window_title());
                    present_x11(display, screen, window, &host);
                }

                if XPending(display) == 0 {
                    thread::sleep(Duration::from_millis(8));
                    continue;
                }

                let mut ev = XEvent { pad: [0; 24] };
                XNextEvent(display, &mut ev);

                match ev.type_ {
                    EXPOSE => {
                        present_x11(display, screen, window, &host);
                    }
                    CLIENT_MESSAGE => {
                        if ev.client_message.data_l[0] as Atom == wm_delete {
                            break;
                        }
                    }
                    CONFIGURE_NOTIFY => {
                        let w = ev.configure.width.max(1) as u32;
                        let h = ev.configure.height.max(1) as u32;
                        if w != host.window_width || h != host.window_height {
                            host.window_width = w;
                            host.window_height = h;
                            host.session
                                .set_viewport(content_viewport(w as f32, h as f32));
                            host.redraw();
                            present_x11(display, screen, window, &host);
                        }
                    }
                    MOTION_NOTIFY => {
                        let x = ev.motion.x as f32;
                        let y = ev.motion.y as f32;
                        let hit = hit_test_ui(&host.session, host.window_width as f32, x, y);
                        let hovered = match hit {
                            Some(UiHit::Page { x: px, y: py }) => host.session.hit_link(px, py),
                            _ => None,
                        };
                        if host.ui.set_hovered_link(hovered) {
                            host.redraw();
                            present_x11(display, screen, window, &host);
                        }
                    }
                    BUTTON_PRESS => {
                        let btn = ev.button.button;
                        if btn == 4 || btn == 5 {
                            let step = if btn == 4 { -48.0 } else { 48.0 };
                            let before = host.session.scroll();
                            let after = host.session.scroll_by(step);
                            if (after - before).abs() > f32::EPSILON {
                                host.redraw();
                                present_x11(display, screen, window, &host);
                            }
                        } else if btn == 1 {
                            let x = ev.button.x as f32;
                            let y = ev.button.y as f32;
                            let hit = hit_test_ui(&host.session, host.window_width as f32, x, y);
                            let mut changed = false;
                            match hit {
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
                                    changed = host.session.back();
                                }
                                Some(UiHit::Forward) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    changed = host.session.forward();
                                }
                                Some(UiHit::Reload) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    let url = host.current_url();
                                    if !url.is_empty() {
                                        host.start_navigation_x11(&url);
                                        changed = true;
                                    }
                                }
                                Some(UiHit::Home) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(HOME_URL);
                                    changed = true;
                                }
                                Some(UiHit::SecurityBadge) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.start_navigation_x11(SECURITY_URL);
                                    changed = true;
                                }
                                Some(UiHit::AddressField) => {
                                    host.session.blur_page_input();
                                    let url = host.current_url();
                                    host.ui.focus_address(&url);
                                    changed = true;
                                }
                                Some(UiHit::ToggleBookmark) => {
                                    host.ui.blur_address();
                                    host.session.blur_page_input();
                                    host.session.toggle_bookmark_active();
                                    changed = true;
                                }
                                Some(UiHit::Bookmark(index)) => {
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
                                Some(UiHit::Page { x: px, y: py }) => {
                                    if host.ui.is_editing_address() {
                                        host.ui.blur_address();
                                        changed = true;
                                    }
                                    match host.session.click_at(px, py) {
                                        ClickOutcome::ScriptHandled
                                        | ClickOutcome::InputFocused(_)
                                        | ClickOutcome::CheckboxToggled(..)
                                        | ClickOutcome::SelectChanged(..) => {
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
                                        ClickOutcome::None => {}
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
                                present_x11(display, screen, window, &host);
                            }
                        }
                    }
                    KEY_PRESS => {
                        let mut key_ev = ev.key;
                        let mut keysym: KeySym = 0;
                        let mut buf = [0u8; 8];
                        let len = XLookupString(
                            &mut key_ev,
                            buf.as_mut_ptr().cast::<c_char>(),
                            buf.len() as c_int,
                            &mut keysym,
                            null_mut(),
                        );

                        if host.ui.is_editing_address() {
                            match keysym {
                                XK_ESCAPE => {
                                    host.ui.blur_address();
                                }
                                XK_BACKSPACE => {
                                    host.ui.backspace();
                                }
                                XK_RETURN => {
                                    if let Some(target) = host.ui.submit_address() {
                                        host.start_navigation_x11(&target);
                                    }
                                }
                                _ if len > 0 => {
                                    let ch = buf[0] as char;
                                    if (' '..='~').contains(&ch) {
                                        host.ui.push_char(ch);
                                    }
                                }
                                _ => {}
                            }
                            host.redraw();
                            set_x11_title(display, window, &host.window_title());
                            present_x11(display, screen, window, &host);
                            continue;
                        }

                        if host.session.has_focused_input() {
                            match keysym {
                                XK_ESCAPE => {
                                    let _ = host.session.blur_page_input();
                                }
                                XK_BACKSPACE => {
                                    let _ = host.session.backspace_focused_input();
                                }
                                XK_RETURN => match host.session.submit_focused_input() {
                                    ClickOutcome::Navigate(target) => {
                                        host.start_navigation_x11(&target);
                                    }
                                    ClickOutcome::SubmitPost { url, body } => {
                                        host.start_post_navigation_x11(&url, &body);
                                    }
                                    _ => {}
                                },
                                _ if len > 0 => {
                                    let ch = buf[0] as char;
                                    let _ = host.session.type_into_focused_input(ch);
                                }
                                _ => {}
                            }
                            host.redraw();
                            set_x11_title(display, window, &host.window_title());
                            present_x11(display, screen, window, &host);
                            continue;
                        }

                        if keysym == XK_ESCAPE {
                            break;
                        }

                        let changed = match keysym {
                            XK_F6 => {
                                let url = host.current_url();
                                host.ui.focus_address(&url);
                                true
                            }
                            XK_F5 => {
                                let url = host.current_url();
                                if !url.is_empty() {
                                    host.start_navigation_x11(&url);
                                    true
                                } else {
                                    false
                                }
                            }
                            XK_BACKSPACE | XK_LEFT => host.session.back(),
                            XK_RIGHT => host.session.forward(),
                            XK_HOME => {
                                let before = host.session.scroll();
                                (host.session.scroll_to_ratio(0.0) - before).abs() > f32::EPSILON
                            }
                            XK_END => {
                                let before = host.session.scroll();
                                (host.session.scroll_to_ratio(1.0) - before).abs() > f32::EPSILON
                            }
                            XK_UP => {
                                let before = host.session.scroll();
                                (host.session.scroll_by(-40.0) - before).abs() > f32::EPSILON
                            }
                            XK_DOWN => {
                                let before = host.session.scroll();
                                (host.session.scroll_by(40.0) - before).abs() > f32::EPSILON
                            }
                            XK_PAGE_UP => {
                                let step = -host.session.viewport().height * 0.8;
                                let before = host.session.scroll();
                                (host.session.scroll_by(step) - before).abs() > f32::EPSILON
                            }
                            XK_PAGE_DOWN => {
                                let step = host.session.viewport().height * 0.8;
                                let before = host.session.scroll();
                                (host.session.scroll_by(step) - before).abs() > f32::EPSILON
                            }
                            _ => false,
                        };
                        if changed {
                            host.redraw();
                            set_x11_title(display, window, &host.window_title());
                            present_x11(display, screen, window, &host);
                        }
                    }
                    _ => {}
                }
            }

            XDestroyWindow(display, window);
            XCloseDisplay(display);
        }
        Ok(())
    }

    unsafe fn set_x11_title(display: *mut Display, window: Window, title: &str) {
        if let Ok(c_title) = CString::new(title) {
            unsafe {
                XStoreName(display, window, c_title.as_ptr());
            }
        }
    }

    unsafe fn present_x11(display: *mut Display, screen: c_int, window: Window, host: &Host) {
        let width = host.framebuffer.width();
        let height = host.framebuffer.height();
        let mut bgra = host.framebuffer.to_bgra_top_down();
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
                bgra.as_mut_ptr().cast::<c_char>(),
                width,
                height,
                32,
                (width * 4) as c_int,
            );
            if !image.is_null() {
                XPutImage(display, window, gc, image, 0, 0, 0, 0, width, height);
                (*image).data = null_mut();
            }
            XFlush(display);
        }
    }
}

pub use imp::open;
