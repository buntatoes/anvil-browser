//! Win32 window shell that presents the software framebuffer and dispatches input.

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use std::thread;

use flux_raster::Framebuffer;

use crate::ui::{UiHit, UiState, compose_frame, content_viewport, hit_test_ui};
use crate::{ClickOutcome, Page, Session, Viewport, load_target};

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
const VK_ESCAPE: usize = 0x1B;
const VK_PRIOR: usize = 0x21;
const VK_NEXT: usize = 0x22;
const VK_UP: usize = 0x26;
const VK_DOWN: usize = 0x28;
const VK_F5: usize = 0x74;
const VK_F6: usize = 0x75;

const IDC_ARROW: *const u16 = 32512 as *const u16;
const IDC_IBEAM: *const u16 = 32513 as *const u16;
const IDC_HAND: *const u16 = 32649 as *const u16;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;

type PendingLoad = Arc<Mutex<Option<(usize, Result<Page, String>)>>>;

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

    fn start_navigation(&mut self, hwnd: Hwnd, target: &str) {
        if target.starts_with("http://") || target.starts_with("https://") {
            let url = target.to_string();
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
}

pub fn open(target: Option<&str>, window_size: Viewport) -> Result<(), String> {
    let win_w = (window_size.width.round() as u32).max(480);
    let win_h = (window_size.height.round() as u32).max(320);
    let mut session = Session::new(content_viewport(win_w as f32, win_h as f32));
    let mut ui = UiState::new();

    if let Some(target) = target {
        session.open_target(target)?;
    } else {
        ui.focus_address("");
    }

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
                let cursor = match hit_test_ui(&host.session, host.window_width as f32, x, y) {
                    Some(UiHit::AddressField) => IDC_IBEAM,
                    Some(
                        UiHit::Tab(_)
                        | UiHit::CloseTab(_)
                        | UiHit::NewTab
                        | UiHit::Back
                        | UiHit::Forward
                        | UiHit::Reload,
                    ) => IDC_HAND,
                    Some(UiHit::Page { x: px, y: py })
                        if host.session.hit_link(px, py).is_some() =>
                    {
                        IDC_HAND
                    }
                    _ => IDC_ARROW,
                };
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
                        changed = host.session.select(index);
                    }
                    Some(UiHit::CloseTab(index)) => {
                        host.ui.blur_address();
                        changed = host.session.close_tab(index);
                    }
                    Some(UiHit::NewTab) => {
                        host.session.new_tab();
                        host.ui.focus_address("");
                        changed = true;
                    }
                    Some(UiHit::Back) => {
                        host.ui.blur_address();
                        changed = host.session.back();
                    }
                    Some(UiHit::Forward) => {
                        host.ui.blur_address();
                        changed = host.session.forward();
                    }
                    Some(UiHit::Reload) => {
                        host.ui.blur_address();
                        let url = host.current_url();
                        if !url.is_empty() {
                            host.start_navigation(hwnd, &url);
                            changed = true;
                        }
                    }
                    Some(UiHit::AddressField) => {
                        let url = host.current_url();
                        host.ui.focus_address(&url);
                        changed = true;
                    }
                    Some(UiHit::Page { x: px, y: py }) => {
                        if host.ui.is_editing_address() {
                            host.ui.blur_address();
                            changed = true;
                        }
                        match host.session.click_at(px, py) {
                            ClickOutcome::ScriptHandled => {
                                changed = true;
                            }
                            ClickOutcome::Navigate(target) => {
                                host.start_navigation(hwnd, &target);
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
                }
            }
            0
        }
        WM_KEYDOWN => {
            if let Some(host) = unsafe { host_mut(hwnd) } {
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
