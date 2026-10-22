//! Win32 window shell that presents the software framebuffer and dispatches input.

use std::ffi::c_void;
use std::fs;
use std::ptr::{null, null_mut};

use flux_raster::Framebuffer;

use crate::{Session, Viewport};

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
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_MOUSEWHEEL: u32 = 0x020A;

const VK_BACK: usize = 0x08;
const VK_ESCAPE: usize = 0x1B;
const VK_PRIOR: usize = 0x21;
const VK_NEXT: usize = 0x22;
const VK_UP: usize = 0x26;
const VK_DOWN: usize = 0x28;

const IDC_ARROW: *const u16 = 32512 as *const u16;
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
    framebuffer: Framebuffer,
}

impl Host {
    fn navigate(&mut self, target: &str) -> Result<(), String> {
        if target.starts_with("http://") || target.starts_with("https://") {
            self.session
                .open_url(target)
                .map_err(|err| err.to_string())?;
        } else {
            let html = fs::read_to_string(target)
                .map_err(|err| format!("could not read {target}: {err}"))?;
            self.session.open_html(&html, target);
        }
        self.framebuffer = self.session.rasterize();
        Ok(())
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

pub fn open(target: &str, viewport: Viewport) -> Result<(), String> {
    let session = Session::new(viewport);
    let framebuffer = session.rasterize();
    let mut host = Box::new(Host {
        session,
        framebuffer,
    });
    host.navigate(target)?;

    let class_name = wide("FluxWindow");
    let title = wide(&host.window_title());
    let width = (viewport.width.round() as i32).max(320);
    let height = (viewport.height.round() as i32).max(240);

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
            width,
            height,
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
        WM_SIZE => {
            if let Some(host) = unsafe { host_mut(hwnd) } {
                let mut rect = WinRect::default();
                unsafe {
                    GetClientRect(hwnd, &mut rect);
                }
                let width = (rect.right - rect.left).max(0) as f32;
                let height = (rect.bottom - rect.top).max(0) as f32;
                let current = host.session.viewport();
                if width > 0.0
                    && height > 0.0
                    && (width != current.width || height != current.height)
                {
                    host.session.set_viewport(Viewport { width, height });
                    host.framebuffer = host.session.rasterize();
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
                let cursor = if host.session.hit_link(x, y).is_some() {
                    IDC_HAND
                } else {
                    IDC_ARROW
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
                if let Some(target) = host.session.hit_link(x, y) {
                    if host.navigate(&target).is_ok() {
                        let title = wide(&host.window_title());
                        unsafe {
                            SetWindowTextW(hwnd, title.as_ptr());
                            InvalidateRect(hwnd, null(), 0);
                        }
                    }
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
                    host.framebuffer = host.session.rasterize();
                    unsafe {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
            }
            0
        }
        WM_KEYDOWN => {
            if w_param == VK_ESCAPE {
                unsafe {
                    DestroyWindow(hwnd);
                }
                return 0;
            }
            if let Some(host) = unsafe { host_mut(hwnd) } {
                let changed = match w_param {
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
                    host.framebuffer = host.session.rasterize();
                    let title = wide(&host.window_title());
                    unsafe {
                        SetWindowTextW(hwnd, title.as_ptr());
                        InvalidateRect(hwnd, null(), 0);
                    }
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
