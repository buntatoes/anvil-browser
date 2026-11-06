// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Native Wayland wire-protocol client (`wl_display`, `wl_registry`, `wl_compositor`,
//! `wl_shm`, `wl_output`, `xdg_wm_base`, `xdg_surface`, `xdg_toplevel`,
//! `zxdg_decoration_manager_v1`, `wl_seat` / `wl_pointer` / `wl_keyboard`,
//! `wl_data_device_manager` / `wl_data_device` / `wl_data_source` / `wl_data_offer`,
//! `wp_cursor_shape_manager_v1`, `wp_fractional_scale_manager_v1`, `wp_viewporter`,
//! and `zwp_linux_dmabuf_v1`).
//!
//! Connects directly to `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` over a Unix domain stream
//! socket without linking external Wayland C libraries, passes a `memfd_create` shared
//! memory file descriptor via `SCM_RIGHTS`, and presents top-down `XRGB8888` (`BGRA`)
//! buffers (with dirty-row band damage and zero intermediate allocation) to the
//! Wayland compositor.

use std::collections::VecDeque;
use std::env;
use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::io::{ErrorKind, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::time::Duration;

use crate::keys::{Key, Modifiers, key_from_evdev};

/// Fixed object ID of `wl_display`.
const WL_DISPLAY_ID: u32 = 1;
/// Object ID assigned to `wl_registry`.
const WL_REGISTRY_ID: u32 = 2;

/// `wl_shm` pixel format: 32-bit `ARGB8888` (little-endian `[B, G, R, A]`).
const WL_SHM_FORMAT_ARGB8888: u32 = 0;
/// `wl_shm` pixel format: 32-bit `XRGB8888` (little-endian `[B, G, R, X]`).
const WL_SHM_FORMAT_XRGB8888: u32 = 1;

/// DRM fourcc codes advertised by `zwp_linux_dmabuf_v1`.
const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;
const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;

/// Linux `input-event-codes.h` mouse button codes.
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// Wayland `xdg_toplevel_resize_edge` values.
pub const XDG_RESIZE_NONE: u32 = 0;
pub const XDG_RESIZE_TOP: u32 = 1;
pub const XDG_RESIZE_BOTTOM: u32 = 2;
pub const XDG_RESIZE_LEFT: u32 = 4;
pub const XDG_RESIZE_TOP_LEFT: u32 = 5;
pub const XDG_RESIZE_BOTTOM_LEFT: u32 = 6;
pub const XDG_RESIZE_RIGHT: u32 = 8;
pub const XDG_RESIZE_TOP_RIGHT: u32 = 9;
pub const XDG_RESIZE_BOTTOM_RIGHT: u32 = 10;

/// Pointer cursor shapes supported via `wp_cursor_shape_device_v1` and `wl_pointer.set_cursor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaylandCursorShape {
    /// Standard left-pointing arrow cursor.
    Default,
    /// Interactive link hand pointer cursor.
    Pointer,
    /// Text selection / input I-beam cursor.
    Text,
    /// North edge resize cursor.
    ResizeN,
    /// South edge resize cursor.
    ResizeS,
    /// East edge resize cursor.
    ResizeE,
    /// West edge resize cursor.
    ResizeW,
    /// North-East corner resize cursor.
    ResizeNE,
    /// North-West corner resize cursor.
    ResizeNW,
    /// South-East corner resize cursor.
    ResizeSE,
    /// South-West corner resize cursor.
    ResizeSW,
}

/// Map a [`WaylandCursorShape`] to its `wp_cursor_shape_device_v1.shape` protocol constant.
pub fn wp_cursor_shape_code(shape: WaylandCursorShape) -> u32 {
    match shape {
        WaylandCursorShape::Default => 1,
        WaylandCursorShape::Pointer => 4,
        WaylandCursorShape::Text => 9,
        WaylandCursorShape::ResizeN => 16,
        WaylandCursorShape::ResizeE => 17,
        WaylandCursorShape::ResizeNE => 18,
        WaylandCursorShape::ResizeS => 19,
        WaylandCursorShape::ResizeW => 20,
        WaylandCursorShape::ResizeNW => 21,
        WaylandCursorShape::ResizeSE => 22,
        WaylandCursorShape::ResizeSW => 23,
    }
}

/// Map a C++ hit-test code (`10..=17`) to a [`WaylandCursorShape`] resize cursor.
pub fn hit_code_to_cursor_shape(hit: isize) -> Option<WaylandCursorShape> {
    match hit {
        10 => Some(WaylandCursorShape::ResizeW),
        11 => Some(WaylandCursorShape::ResizeE),
        12 => Some(WaylandCursorShape::ResizeN),
        13 => Some(WaylandCursorShape::ResizeNW),
        14 => Some(WaylandCursorShape::ResizeNE),
        15 => Some(WaylandCursorShape::ResizeS),
        16 => Some(WaylandCursorShape::ResizeSW),
        17 => Some(WaylandCursorShape::ResizeSE),
        _ => None,
    }
}

/// Convert a Wayland `wp_fractional_scale_v1` `preferred_scale` (in 120ths) to a percentage.
pub fn fractional_scale_120_to_percent(scale_120: u32) -> u32 {
    if scale_120 == 0 {
        return 100;
    }
    let raw = (scale_120 * 100 + 60) / 120;
    let snapped = ((raw + 2) / 5) * 5;
    snapped.clamp(50, 400)
}

/// Events emitted by [`WaylandWindow::poll_events`].
#[derive(Clone, Debug, PartialEq)]
pub enum WaylandEvent {
    /// Window size or maximized state changed.
    Configure {
        width: u32,
        height: u32,
        maximized: bool,
    },
    /// Compositor or output scale factor changed (in percent, e.g. `100`, `125`, `150`, `200`).
    ScaleChanged { scale_percent: u32 },
    /// Compositor requested window close.
    Close,
    /// Pointer moved to `(x, y)` in surface-local client coordinates.
    PointerMotion { x: f32, y: f32 },
    /// Mouse button pressed or released (`BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`).
    PointerButton {
        button: u32,
        pressed: bool,
        serial: u32,
        x: f32,
        y: f32,
    },
    /// Vertical scroll wheel delta in pixels (positive scrolls down).
    PointerScroll {
        delta_y: f32,
        x: f32,
        y: f32,
        ctrl: bool,
    },
    /// Key pressed or repeated.
    KeyPress { key: Key, mods: Modifiers },
    /// Compositor frame callback (`wl_surface.frame` / `wl_callback.done`) fired with timestamp in ms.
    FrameDone { timestamp_ms: u32 },
}

unsafe extern "C" {
    fn memfd_create(name: *const c_char, flags: c_uint) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn pipe(pipefd: *mut c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn sendmsg(sockfd: c_int, msg: *const Msghdr, flags: c_int) -> isize;
    fn recvmsg(sockfd: c_int, msg: *mut Msghdr, flags: c_int) -> isize;
}

const MFD_CLOEXEC: c_uint = 0x0001;
const PROT_READ: c_int = 0x1;
const PROT_WRITE: c_int = 0x2;
const MAP_SHARED: c_int = 0x01;
const MAP_FAILED: *mut c_void = !0usize as *mut c_void;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const MSG_DONTWAIT: c_int = 0x40;
const MSG_NOSIGNAL: c_int = 0x4000;

#[repr(C)]
struct Iovec {
    iov_base: *mut c_void,
    iov_len: usize,
}

#[repr(C)]
struct Msghdr {
    msg_name: *mut c_void,
    msg_namelen: u32,
    msg_iov: *mut Iovec,
    msg_iovlen: usize,
    msg_control: *mut c_void,
    msg_controllen: usize,
    msg_flags: c_int,
}

#[repr(C)]
struct Cmsghdr {
    cmsg_len: usize,
    cmsg_level: c_int,
    cmsg_type: c_int,
}

/// Convert `wl_fixed_t` (signed 24.8 fixed-point) to `f32`.
#[inline]
pub fn wl_fixed_to_f32(v: i32) -> f32 {
    (v as f32) / 256.0
}

/// Encode a Wayland wire message (`[object_id, (size << 16) | opcode, payload...]`).
pub fn encode_wl_message(object_id: u32, opcode: u16, payload_words: &[u32]) -> Vec<u8> {
    let size = (8 + payload_words.len() * 4) as u32;
    let header_word_1 = (size << 16) | u32::from(opcode);
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&object_id.to_ne_bytes());
    out.extend_from_slice(&header_word_1.to_ne_bytes());
    for &w in payload_words {
        out.extend_from_slice(&w.to_ne_bytes());
    }
    out
}

/// Append a Wayland string (`len_with_nul: u32` + nul-terminated 4-byte padded UTF-8) to `words`.
pub fn push_wl_string(words: &mut Vec<u32>, s: &str) {
    let bytes = s.as_bytes();
    let len_with_nul = (bytes.len() + 1) as u32;
    words.push(len_with_nul);
    let padded_len = ((len_with_nul as usize) + 3) & !3;
    let mut buf = vec![0u8; padded_len];
    buf[..bytes.len()].copy_from_slice(bytes);
    for &chunk in buf.as_chunks::<4>().0 {
        words.push(u32::from_ne_bytes(chunk));
    }
}

/// Parse a Wayland string starting at byte offset `off` in `payload`.
fn read_wl_string(payload: &[u8], off: &mut usize) -> Option<String> {
    if *off + 4 > payload.len() {
        return None;
    }
    let len = u32::from_ne_bytes(payload[*off..*off + 4].try_into().ok()?) as usize;
    *off += 4;
    if len == 0 {
        return Some(String::new());
    }
    let padded = (len + 3) & !3;
    if *off + padded > payload.len() {
        return None;
    }
    let slice = &payload[*off..*off + len.saturating_sub(1)];
    *off += padded;
    Some(String::from_utf8_lossy(slice).into_owned())
}

/// Read a `u32` at byte offset `off` in `payload`.
fn read_u32(payload: &[u8], off: &mut usize) -> Option<u32> {
    if *off + 4 > payload.len() {
        return None;
    }
    let v = u32::from_ne_bytes(payload[*off..*off + 4].try_into().ok()?);
    *off += 4;
    Some(v)
}

/// Read an `i32` at byte offset `off` in `payload`.
fn read_i32(payload: &[u8], off: &mut usize) -> Option<i32> {
    read_u32(payload, off).map(|v| v as i32)
}

/// Resolve the Wayland socket path (`$WAYLAND_DISPLAY` under `$XDG_RUNTIME_DIR`,
/// with fallback to `wayland-0` / `wayland-1` for GNOME Mutter and KDE KWin).
pub fn resolve_wayland_socket_path() -> Option<PathBuf> {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").ok().map(PathBuf::from);
    if let Ok(display) = env::var("WAYLAND_DISPLAY")
        && !display.is_empty()
    {
        let path = PathBuf::from(&display);
        if path.is_absolute() && path.exists() {
            return Some(path);
        }
        if let Some(ref rt) = runtime_dir {
            let full = rt.join(&display);
            if full.exists() {
                return Some(full);
            }
        }
    }
    if let Some(rt) = runtime_dir {
        for candidate in ["wayland-0", "wayland-1"] {
            let full = rt.join(candidate);
            if full.exists() {
                return Some(full);
            }
        }
    }
    None
}

/// Map a C++ hit-test code (`10..=17`) to Wayland `xdg_toplevel_resize_edge`.
pub fn hit_code_to_wayland_edge(hit: isize) -> Option<u32> {
    match hit {
        10 => Some(XDG_RESIZE_LEFT),
        11 => Some(XDG_RESIZE_RIGHT),
        12 => Some(XDG_RESIZE_TOP),
        13 => Some(XDG_RESIZE_TOP_LEFT),
        14 => Some(XDG_RESIZE_TOP_RIGHT),
        15 => Some(XDG_RESIZE_BOTTOM),
        16 => Some(XDG_RESIZE_BOTTOM_LEFT),
        17 => Some(XDG_RESIZE_BOTTOM_RIGHT),
        _ => None,
    }
}

/// Synthesize a 24x24 `ARGB8888` (`[B, G, R, A]`) software cursor bitmap and hotspot
/// when `wp_cursor_shape_manager_v1` is not advertised by the compositor.
pub fn render_software_cursor_argb(shape: WaylandCursorShape) -> ([u8; 24 * 24 * 4], i32, i32) {
    let mut px = [0u8; 24 * 24 * 4];
    let set_px = |buf: &mut [u8; 24 * 24 * 4], x: i32, y: i32, rgba: [u8; 4]| {
        if (0..24).contains(&x) && (0..24).contains(&y) {
            let idx = ((y as usize) * 24 + (x as usize)) * 4;
            buf[idx] = rgba[2];
            buf[idx + 1] = rgba[1];
            buf[idx + 2] = rgba[0];
            buf[idx + 3] = rgba[3];
        }
    };
    let white = [255, 255, 255, 255];
    let dark = [20, 24, 32, 255];
    match shape {
        WaylandCursorShape::Text => {
            for x in 8..=14 {
                set_px(&mut px, x, 4, dark);
                set_px(&mut px, x, 19, dark);
            }
            for y in 4..=19 {
                set_px(&mut px, 11, y, dark);
                set_px(&mut px, 10, y, white);
            }
            (px, 11, 11)
        }
        WaylandCursorShape::Pointer => {
            for y in 2..18 {
                let half = ((y - 2) / 2).clamp(1, 6);
                for x in 6..(6 + half + 3) {
                    let edge = x == 6 || x == 6 + half + 2 || y == 2 || y == 17;
                    set_px(&mut px, x, y, if edge { dark } else { white });
                }
            }
            (px, 7, 2)
        }
        WaylandCursorShape::ResizeE | WaylandCursorShape::ResizeW => {
            for x in 3..=20 {
                set_px(&mut px, x, 11, dark);
                set_px(&mut px, x, 12, white);
            }
            for d in 0..=3 {
                set_px(&mut px, 3 + d, 11 - d, dark);
                set_px(&mut px, 3 + d, 11 + d, dark);
                set_px(&mut px, 20 - d, 11 - d, dark);
                set_px(&mut px, 20 - d, 11 + d, dark);
            }
            (px, 12, 12)
        }
        WaylandCursorShape::ResizeN | WaylandCursorShape::ResizeS => {
            for y in 3..=20 {
                set_px(&mut px, 11, y, dark);
                set_px(&mut px, 12, y, white);
            }
            for d in 0..=3 {
                set_px(&mut px, 11 - d, 3 + d, dark);
                set_px(&mut px, 11 + d, 3 + d, dark);
                set_px(&mut px, 11 - d, 20 - d, dark);
                set_px(&mut px, 11 + d, 20 - d, dark);
            }
            (px, 12, 12)
        }
        WaylandCursorShape::ResizeNW
        | WaylandCursorShape::ResizeSE
        | WaylandCursorShape::ResizeNE
        | WaylandCursorShape::ResizeSW => {
            for i in 4..=19 {
                let x = if matches!(
                    shape,
                    WaylandCursorShape::ResizeNE | WaylandCursorShape::ResizeSW
                ) {
                    23 - i
                } else {
                    i
                };
                set_px(&mut px, x, i, dark);
                set_px(&mut px, x - 1, i, white);
            }
            (px, 12, 12)
        }
        WaylandCursorShape::Default => {
            for y in 1..17 {
                let max_x = 1 + (y * 3) / 4;
                for x in 1..=max_x {
                    let border = x == 1 || x == max_x || y == 16;
                    set_px(&mut px, x, y, if border { dark } else { white });
                }
            }
            (px, 1, 1)
        }
    }
}

/// Query primary monitor pixel dimensions and scale percentage from the Wayland compositor.
pub fn query_wayland_monitor_info() -> Option<(u32, u32, u32)> {
    let sock_path = resolve_wayland_socket_path()?;
    let stream = UnixStream::connect(&sock_path).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut win = WaylandWindow::new_unconnected(stream);
    win.send_request(WL_DISPLAY_ID, 1, &[WL_REGISTRY_ID]).ok()?;
    win.roundtrip().ok()?;
    if !win.output_ids.is_empty() {
        let _ = win.roundtrip();
    }
    if win.output_width > 0 && win.output_height > 0 {
        let scale_pct = if win.fractional_scale_120 > 0 && win.fractional_scale_120 != 120 {
            fractional_scale_120_to_percent(win.fractional_scale_120)
        } else {
            win.output_scale.max(1) * 100
        };
        Some((win.output_width, win.output_height, scale_pct))
    } else {
        None
    }
}

/// Anonymous shared-memory buffer pool backed by `memfd_create`.
struct ShmSurfacePool {
    fd: RawFd,
    ptr: *mut u8,
    capacity: usize,
    pool_id: u32,
    buffer_id: u32,
    buf_w: u32,
    buf_h: u32,
}

impl Drop for ShmSurfacePool {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() && self.capacity > 0 {
                let _ = munmap(self.ptr.cast::<c_void>(), self.capacity);
            }
            if self.fd >= 0 {
                let _ = close(self.fd);
            }
        }
    }
}

/// Native Wayland window connection and protocol state.
pub struct WaylandWindow {
    stream: UnixStream,
    recv_buf: Vec<u8>,
    received_fds: VecDeque<RawFd>,
    next_id: u32,

    compositor_id: u32,
    compositor_version: u32,
    shm_id: u32,
    xdg_wm_base_id: u32,
    seat_id: u32,
    decoration_mgr_id: u32,
    decoration_id: u32,
    /// `Some(2)` when the compositor said it draws the frame. `Some(1)` is client side.
    decoration_mode: Option<u32>,
    data_device_mgr_id: u32,
    data_device_id: u32,
    data_source_id: u32,
    pending_offer_id: u32,
    pending_offer_has_text: bool,
    selection_offer_id: u32,
    selection_offer_has_text: bool,
    clipboard_text: String,

    cursor_shape_mgr_id: u32,
    cursor_shape_device_id: u32,
    current_cursor_shape: WaylandCursorShape,
    cursor_surface_id: u32,
    cursor_pool: Option<ShmSurfacePool>,

    output_ids: Vec<u32>,
    output_width: u32,
    output_height: u32,
    output_scale: u32,
    viewporter_id: u32,
    fractional_scale_mgr_id: u32,
    fractional_scale_id: u32,
    fractional_scale_120: u32,

    dmabuf_id: u32,
    dmabuf_xrgb_supported: bool,

    surface_id: u32,
    frame_callback_id: u32,
    frame_pending: bool,
    last_frame_timestamp_ms: u32,
    xdg_surface_id: u32,
    xdg_toplevel_id: u32,
    pointer_id: u32,
    keyboard_id: u32,

    shm_pool: Option<ShmSurfacePool>,
    configured: bool,
    pending_width: u32,
    pending_height: u32,
    pending_maximized: bool,

    pointer_x: f32,
    pointer_y: f32,
    pointer_enter_serial: u32,
    last_button_serial: u32,
    last_input_serial: u32,
    mods: Modifiers,
    caps_lock: bool,
}

impl Drop for WaylandWindow {
    fn drop(&mut self) {
        while let Some(fd) = self.received_fds.pop_front() {
            if fd >= 0 {
                unsafe {
                    let _ = close(fd);
                }
            }
        }
    }
}

impl WaylandWindow {
    fn new_unconnected(stream: UnixStream) -> Self {
        Self {
            stream,
            recv_buf: Vec::with_capacity(4096),
            received_fds: VecDeque::new(),
            next_id: 3,
            compositor_id: 0,
            compositor_version: 1,
            shm_id: 0,
            xdg_wm_base_id: 0,
            seat_id: 0,
            decoration_mgr_id: 0,
            decoration_id: 0,
            decoration_mode: None,
            data_device_mgr_id: 0,
            data_device_id: 0,
            data_source_id: 0,
            pending_offer_id: 0,
            pending_offer_has_text: false,
            selection_offer_id: 0,
            selection_offer_has_text: false,
            clipboard_text: String::new(),
            cursor_shape_mgr_id: 0,
            cursor_shape_device_id: 0,
            current_cursor_shape: WaylandCursorShape::Default,
            cursor_surface_id: 0,
            cursor_pool: None,
            output_ids: Vec::new(),
            output_width: 0,
            output_height: 0,
            output_scale: 1,
            viewporter_id: 0,
            fractional_scale_mgr_id: 0,
            fractional_scale_id: 0,
            fractional_scale_120: 120,
            dmabuf_id: 0,
            dmabuf_xrgb_supported: false,
            surface_id: 0,
            frame_callback_id: 0,
            frame_pending: false,
            last_frame_timestamp_ms: 0,
            xdg_surface_id: 0,
            xdg_toplevel_id: 0,
            pointer_id: 0,
            keyboard_id: 0,
            shm_pool: None,
            configured: false,
            pending_width: 0,
            pending_height: 0,
            pending_maximized: false,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_enter_serial: 0,
            last_button_serial: 0,
            last_input_serial: 0,
            mods: Modifiers::default(),
            caps_lock: false,
        }
    }

    /// Connect to the Wayland compositor, bind globals, and create the `xdg_toplevel` window.
    pub fn connect(title: &str, width: u32, height: u32) -> Result<Self, String> {
        let sock_path = resolve_wayland_socket_path()
            .ok_or_else(|| "WAYLAND_DISPLAY socket not found".to_string())?;
        let stream = UnixStream::connect(&sock_path).map_err(|e| {
            format!(
                "could not connect to Wayland socket {}: {e}",
                sock_path.display()
            )
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(2000)));

        let mut win = Self::new_unconnected(stream);
        win.pending_width = width;
        win.pending_height = height;

        // 1. wl_display.get_registry(WL_REGISTRY_ID) + wl_display.sync
        win.send_request(WL_DISPLAY_ID, 1, &[WL_REGISTRY_ID])?;
        win.roundtrip()?;
        // Flush bound globals (wl_seat capabilities, wl_output modes, etc.)
        win.roundtrip()?;

        if win.compositor_id == 0 || win.shm_id == 0 || win.xdg_wm_base_id == 0 {
            return Err(
                "Wayland compositor is missing wl_compositor, wl_shm, or xdg_wm_base".to_string(),
            );
        }

        // Bind wl_data_device for clipboard exchange if both seat and manager exist
        if win.data_device_mgr_id != 0 && win.seat_id != 0 {
            let ddid = win.alloc_id();
            win.data_device_id = ddid;
            win.send_request(win.data_device_mgr_id, 1, &[ddid, win.seat_id])?;
        }

        // 2. Create wl_surface, xdg_surface, and xdg_toplevel
        let surface_id = win.alloc_id();
        win.surface_id = surface_id;
        win.send_request(win.compositor_id, 0, &[surface_id])?;

        // Attach wp_fractional_scale_manager_v1 if available
        if win.fractional_scale_mgr_id != 0 {
            let fs_id = win.alloc_id();
            win.fractional_scale_id = fs_id;
            win.send_request(win.fractional_scale_mgr_id, 1, &[fs_id, surface_id])?;
        }

        let xdg_surface_id = win.alloc_id();
        win.xdg_surface_id = xdg_surface_id;
        win.send_request(win.xdg_wm_base_id, 2, &[xdg_surface_id, surface_id])?;

        let xdg_toplevel_id = win.alloc_id();
        win.xdg_toplevel_id = xdg_toplevel_id;
        win.send_request(xdg_surface_id, 1, &[xdg_toplevel_id])?;

        // Request client-side decorations if the compositor advertises zxdg_decoration_manager_v1
        if win.decoration_mgr_id != 0 {
            let deco_id = win.alloc_id();
            win.decoration_id = deco_id;
            win.send_request(win.decoration_mgr_id, 1, &[deco_id, xdg_toplevel_id])?;
            // mode 1 = client side. The compositor answers with configure.
            win.send_request(deco_id, 1, &[1])?;
        }

        win.set_title(title);
        win.set_app_id("anvil-browser");

        // Initial empty commit so the compositor sends the first xdg_surface.configure
        win.send_request(surface_id, 6, &[])?;
        for _ in 0..4 {
            win.roundtrip()?;
            if win.configured {
                break;
            }
        }

        // Create wp_cursor_shape_device_v1 if pointer and cursor shape manager were bound
        if win.cursor_shape_mgr_id != 0 && win.pointer_id != 0 && win.cursor_shape_device_id == 0 {
            let dev_id = win.alloc_id();
            win.cursor_shape_device_id = dev_id;
            let _ = win.send_request(win.cursor_shape_mgr_id, 1, &[dev_id, win.pointer_id]);
        }

        Ok(win)
    }

    /// True only after `zxdg_toplevel_decoration_v1.configure` reports mode 2.
    pub fn decoration_is_server_side(&self) -> bool {
        self.decoration_mode == Some(2)
    }

    /// Return the latest compositor-configured `(width, height, maximized)` state.
    pub fn configured_size(&self) -> (u32, u32, bool) {
        (
            self.pending_width,
            self.pending_height,
            self.pending_maximized,
        )
    }

    /// Return whether the compositor advertised `zwp_linux_dmabuf_v1` with `XRGB8888`/`ARGB8888`.
    pub fn dmabuf_supported(&self) -> bool {
        self.dmabuf_xrgb_supported
    }

    /// Return whether a `wl_surface.frame` callback is currently awaiting `wl_callback.done`.
    pub fn is_frame_pending(&self) -> bool {
        self.frame_pending
    }

    /// Return the compositor timestamp (in milliseconds) of the most recent `wl_surface.frame` callback.
    pub fn last_frame_timestamp_ms(&self) -> u32 {
        self.last_frame_timestamp_ms
    }

    /// Return the current display scale in percent (`100`, `125`, `150`, `200`, etc.).
    pub fn scale_percent(&self) -> u32 {
        if self.fractional_scale_120 > 0 && self.fractional_scale_120 != 120 {
            fractional_scale_120_to_percent(self.fractional_scale_120)
        } else {
            self.output_scale.max(1) * 100
        }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send_request(&mut self, object_id: u32, opcode: u16, words: &[u32]) -> Result<(), String> {
        let bytes = encode_wl_message(object_id, opcode, words);
        self.stream
            .write_all(&bytes)
            .map_err(|e| format!("Wayland socket write failed: {e}"))
    }

    fn send_request_with_fd(
        &mut self,
        object_id: u32,
        opcode: u16,
        words: &[u32],
        fd: RawFd,
    ) -> Result<(), String> {
        let mut bytes = encode_wl_message(object_id, opcode, words);
        let mut iov = Iovec {
            iov_base: bytes.as_mut_ptr().cast::<c_void>(),
            iov_len: bytes.len(),
        };

        #[repr(C, align(8))]
        struct AlignedCmsg {
            hdr: Cmsghdr,
            fd: c_int,
            pad: c_int,
        }

        let cmsg_space = std::mem::size_of::<Cmsghdr>() + std::mem::size_of::<c_int>();
        let mut cmsg = AlignedCmsg {
            hdr: Cmsghdr {
                cmsg_len: cmsg_space,
                cmsg_level: SOL_SOCKET,
                cmsg_type: SCM_RIGHTS,
            },
            fd,
            pad: 0,
        };

        let hdr = Msghdr {
            msg_name: null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: (&mut cmsg as *mut AlignedCmsg).cast::<c_void>(),
            msg_controllen: std::mem::size_of::<AlignedCmsg>(),
            msg_flags: 0,
        };

        let sent = unsafe { sendmsg(self.stream.as_raw_fd(), &hdr, MSG_NOSIGNAL) };
        if sent < 0 {
            return Err("sendmsg(SCM_RIGHTS) to Wayland socket failed".to_string());
        }
        Ok(())
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        let cb_id = self.alloc_id();
        self.send_request(WL_DISPLAY_ID, 0, &[cb_id])?;
        let mut events = Vec::new();
        for _ in 0..256 {
            self.read_socket(false)?;
            let mut done = false;
            self.drain_messages(&mut events, Some(cb_id), &mut done)?;
            if done {
                return Ok(());
            }
        }
        Ok(())
    }

    fn read_socket(&mut self, nonblocking: bool) -> Result<usize, String> {
        let mut tmp = [0u8; 4096];
        let mut iov = Iovec {
            iov_base: tmp.as_mut_ptr().cast::<c_void>(),
            iov_len: tmp.len(),
        };
        let mut cmsg_buf = [0u64; 8];
        let mut hdr = Msghdr {
            msg_name: null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: cmsg_buf.as_mut_ptr().cast::<c_void>(),
            msg_controllen: std::mem::size_of_val(&cmsg_buf),
            msg_flags: 0,
        };

        let flags = if nonblocking { MSG_DONTWAIT } else { 0 };
        let n = unsafe { recvmsg(self.stream.as_raw_fd(), &mut hdr, flags) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::Interrupted {
                return Ok(0);
            }
            return Err(format!("Wayland socket recvmsg failed: {err}"));
        }
        if n == 0 {
            return Err("Wayland compositor closed connection".to_string());
        }

        // Queue any file descriptors sent via SCM_RIGHTS (wl_keyboard.keymap or wl_data_source.send)
        if hdr.msg_controllen >= std::mem::size_of::<Cmsghdr>() {
            unsafe {
                let cmsg_ptr = hdr.msg_control.cast::<Cmsghdr>();
                let cmsg = &*cmsg_ptr;
                if cmsg.cmsg_level == SOL_SOCKET && cmsg.cmsg_type == SCM_RIGHTS {
                    let data_off = std::mem::size_of::<Cmsghdr>();
                    let fd_bytes = cmsg.cmsg_len.saturating_sub(data_off);
                    let fd_count = fd_bytes / std::mem::size_of::<c_int>();
                    let fds_ptr = hdr.msg_control.cast::<u8>().add(data_off).cast::<c_int>();
                    for i in 0..fd_count {
                        let fd = *fds_ptr.add(i);
                        if fd >= 0 {
                            self.received_fds.push_back(fd);
                        }
                    }
                }
            }
        }

        self.recv_buf.extend_from_slice(&tmp[..n as usize]);
        Ok(n as usize)
    }

    fn drain_messages(
        &mut self,
        out_events: &mut Vec<WaylandEvent>,
        sync_cb: Option<u32>,
        sync_done: &mut bool,
    ) -> Result<(), String> {
        let mut cursor = 0usize;
        while cursor + 8 <= self.recv_buf.len() {
            let obj_id = u32::from_ne_bytes(self.recv_buf[cursor..cursor + 4].try_into().unwrap());
            let size_op =
                u32::from_ne_bytes(self.recv_buf[cursor + 4..cursor + 8].try_into().unwrap());
            let msg_size = (size_op >> 16) as usize;
            let opcode = (size_op & 0xFFFF) as u16;
            if msg_size < 8 {
                return Err("invalid Wayland message size < 8".to_string());
            }
            if cursor + msg_size > self.recv_buf.len() {
                break;
            }
            let payload = self.recv_buf[cursor + 8..cursor + msg_size].to_vec();
            cursor += msg_size;

            if Some(obj_id) == sync_cb && opcode == 0 {
                *sync_done = true;
                continue;
            }
            self.handle_event(obj_id, opcode, &payload, out_events)?;
        }
        if cursor > 0 {
            self.recv_buf.drain(..cursor);
        }
        Ok(())
    }

    fn handle_event(
        &mut self,
        obj_id: u32,
        opcode: u16,
        payload: &[u8],
        out_events: &mut Vec<WaylandEvent>,
    ) -> Result<(), String> {
        if obj_id == WL_DISPLAY_ID {
            if opcode == 0 {
                let mut off = 0;
                let target_id = read_u32(payload, &mut off).unwrap_or(0);
                let code = read_u32(payload, &mut off).unwrap_or(0);
                let msg = read_wl_string(payload, &mut off).unwrap_or_default();
                return Err(format!(
                    "Wayland protocol error on object {target_id} (code {code}): {msg}"
                ));
            }
            return Ok(());
        }

        if self.frame_callback_id != 0 && obj_id == self.frame_callback_id && opcode == 0 {
            let mut off = 0;
            let timestamp_ms = read_u32(payload, &mut off).unwrap_or(0);
            self.frame_callback_id = 0;
            self.frame_pending = false;
            self.last_frame_timestamp_ms = timestamp_ms;
            out_events.push(WaylandEvent::FrameDone { timestamp_ms });
            return Ok(());
        }

        if obj_id == WL_REGISTRY_ID && opcode == 0 {
            let mut off = 0;
            let Some(name) = read_u32(payload, &mut off) else {
                return Ok(());
            };
            let Some(interface) = read_wl_string(payload, &mut off) else {
                return Ok(());
            };
            let Some(version) = read_u32(payload, &mut off) else {
                return Ok(());
            };

            match interface.as_str() {
                "wl_compositor" if self.compositor_id == 0 => {
                    let id = self.alloc_id();
                    let bind_ver = version.min(4);
                    self.compositor_id = id;
                    self.compositor_version = bind_ver;
                    self.bind_global(name, &interface, bind_ver, id)?;
                }
                "wl_shm" if self.shm_id == 0 => {
                    let id = self.alloc_id();
                    self.shm_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                "xdg_wm_base" if self.xdg_wm_base_id == 0 => {
                    let id = self.alloc_id();
                    self.xdg_wm_base_id = id;
                    self.bind_global(name, &interface, version.min(2), id)?;
                }
                "wl_seat" if self.seat_id == 0 => {
                    let id = self.alloc_id();
                    self.seat_id = id;
                    self.bind_global(name, &interface, version.min(5), id)?;
                }
                "zxdg_decoration_manager_v1" if self.decoration_mgr_id == 0 => {
                    let id = self.alloc_id();
                    self.decoration_mgr_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                "wl_data_device_manager" if self.data_device_mgr_id == 0 => {
                    let id = self.alloc_id();
                    self.data_device_mgr_id = id;
                    self.bind_global(name, &interface, version.min(3), id)?;
                }
                "wp_cursor_shape_manager_v1" if self.cursor_shape_mgr_id == 0 => {
                    let id = self.alloc_id();
                    self.cursor_shape_mgr_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                "wl_output" => {
                    let id = self.alloc_id();
                    self.output_ids.push(id);
                    self.bind_global(name, &interface, version.min(3), id)?;
                }
                "wp_viewporter" if self.viewporter_id == 0 => {
                    let id = self.alloc_id();
                    self.viewporter_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                "wp_fractional_scale_manager_v1" if self.fractional_scale_mgr_id == 0 => {
                    let id = self.alloc_id();
                    self.fractional_scale_mgr_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                "zwp_linux_dmabuf_v1" if self.dmabuf_id == 0 => {
                    let id = self.alloc_id();
                    self.dmabuf_id = id;
                    self.bind_global(name, &interface, 1, id)?;
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id == self.dmabuf_id && self.dmabuf_id != 0 {
            if opcode == 0 {
                // zwp_linux_dmabuf_v1.format(format)
                let mut off = 0;
                if let Some(fmt) = read_u32(payload, &mut off)
                    && (fmt == DRM_FORMAT_XRGB8888 || fmt == DRM_FORMAT_ARGB8888)
                {
                    self.dmabuf_xrgb_supported = true;
                }
            }
            return Ok(());
        }

        if self.output_ids.contains(&obj_id) {
            match opcode {
                1 => {
                    // wl_output.mode(flags, width, height, refresh)
                    let mut off = 0;
                    let flags = read_u32(payload, &mut off).unwrap_or(0);
                    let w = read_i32(payload, &mut off).unwrap_or(0);
                    let h = read_i32(payload, &mut off).unwrap_or(0);
                    if (flags & 1) != 0 && w > 0 && h > 0 {
                        self.output_width = w as u32;
                        self.output_height = h as u32;
                    }
                }
                3 => {
                    // wl_output.scale(factor)
                    let mut off = 0;
                    let factor = read_i32(payload, &mut off).unwrap_or(1);
                    if factor > 0 {
                        self.output_scale = factor as u32;
                        if self.fractional_scale_120 == 120 {
                            out_events.push(WaylandEvent::ScaleChanged {
                                scale_percent: (factor as u32) * 100,
                            });
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id == self.fractional_scale_id && self.fractional_scale_id != 0 {
            if opcode == 0 {
                // wp_fractional_scale_v1.preferred_scale(scale)
                let mut off = 0;
                if let Some(scale_120) = read_u32(payload, &mut off)
                    && scale_120 > 0
                {
                    self.fractional_scale_120 = scale_120;
                    out_events.push(WaylandEvent::ScaleChanged {
                        scale_percent: fractional_scale_120_to_percent(scale_120),
                    });
                }
            }
            return Ok(());
        }

        if obj_id == self.xdg_wm_base_id && opcode == 0 {
            // xdg_wm_base.ping(serial) -> xdg_wm_base.pong(serial)
            let mut off = 0;
            if let Some(serial) = read_u32(payload, &mut off) {
                self.send_request(self.xdg_wm_base_id, 3, &[serial])?;
            }
            return Ok(());
        }

        if obj_id == self.seat_id && opcode == 0 {
            // wl_seat.capabilities(caps)
            let mut off = 0;
            if let Some(caps) = read_u32(payload, &mut off) {
                if (caps & 1) != 0 && self.pointer_id == 0 {
                    let pid = self.alloc_id();
                    self.pointer_id = pid;
                    self.send_request(self.seat_id, 0, &[pid])?;
                    if self.cursor_shape_mgr_id != 0 && self.cursor_shape_device_id == 0 {
                        let dev_id = self.alloc_id();
                        self.cursor_shape_device_id = dev_id;
                        let _ = self.send_request(self.cursor_shape_mgr_id, 1, &[dev_id, pid]);
                    }
                }
                if (caps & 2) != 0 && self.keyboard_id == 0 {
                    let kid = self.alloc_id();
                    self.keyboard_id = kid;
                    self.send_request(self.seat_id, 1, &[kid])?;
                }
            }
            return Ok(());
        }

        if obj_id == self.data_device_id && self.data_device_id != 0 {
            match opcode {
                0 => {
                    // wl_data_device.data_offer(new_id)
                    let mut off = 0;
                    if let Some(offer_id) = read_u32(payload, &mut off) {
                        if self.pending_offer_id != 0
                            && self.pending_offer_id != self.selection_offer_id
                        {
                            let _ = self.send_request(self.pending_offer_id, 2, &[]);
                        }
                        self.pending_offer_id = offer_id;
                        self.pending_offer_has_text = false;
                    }
                }
                5 => {
                    // wl_data_device.selection(id)
                    let mut off = 0;
                    let offer_id = read_u32(payload, &mut off).unwrap_or(0);
                    if self.selection_offer_id != 0 && self.selection_offer_id != offer_id {
                        let _ = self.send_request(self.selection_offer_id, 2, &[]);
                    }
                    if offer_id == 0 {
                        self.selection_offer_id = 0;
                        self.selection_offer_has_text = false;
                    } else if offer_id == self.pending_offer_id {
                        self.selection_offer_id = offer_id;
                        self.selection_offer_has_text = self.pending_offer_has_text;
                    } else {
                        self.selection_offer_id = offer_id;
                        self.selection_offer_has_text = true;
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id != 0 && (obj_id == self.pending_offer_id || obj_id == self.selection_offer_id) {
            if opcode == 0 {
                // wl_data_offer.offer(mime_type)
                let mut off = 0;
                if let Some(mime) = read_wl_string(payload, &mut off)
                    && (mime == "text/plain;charset=utf-8"
                        || mime == "text/plain"
                        || mime == "UTF8_STRING")
                {
                    if obj_id == self.pending_offer_id {
                        self.pending_offer_has_text = true;
                    }
                    if obj_id == self.selection_offer_id {
                        self.selection_offer_has_text = true;
                    }
                }
            }
            return Ok(());
        }

        if obj_id == self.data_source_id && self.data_source_id != 0 {
            match opcode {
                1 => {
                    // wl_data_source.send(mime_type, fd)
                    let mut off = 0;
                    let _mime = read_wl_string(payload, &mut off);
                    if let Some(fd) = self.received_fds.pop_front()
                        && fd >= 0
                    {
                        let bytes = self.clipboard_text.as_bytes();
                        unsafe {
                            let _ = write(fd, bytes.as_ptr().cast::<c_void>(), bytes.len());
                            let _ = close(fd);
                        }
                    }
                }
                2 => {
                    // wl_data_source.cancelled()
                    let _ = self.send_request(self.data_source_id, 1, &[]);
                    self.data_source_id = 0;
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id == self.decoration_id && self.decoration_id != 0 {
            if opcode == 0 {
                // configure(mode). 1 = client side, 2 = server side.
                let mut off = 0;
                if let Some(mode) = read_u32(payload, &mut off) {
                    self.decoration_mode = Some(mode);
                }
            }
            return Ok(());
        }

        if obj_id == self.xdg_toplevel_id {
            match opcode {
                0 => {
                    // xdg_toplevel.configure(width, height, states)
                    let mut off = 0;
                    let w = read_i32(payload, &mut off).unwrap_or(0);
                    let h = read_i32(payload, &mut off).unwrap_or(0);
                    let states_len = read_u32(payload, &mut off).unwrap_or(0) as usize;
                    let mut maximized = false;
                    if off + states_len <= payload.len() {
                        for &chunk in payload[off..off + states_len].as_chunks::<4>().0 {
                            let st = u32::from_ne_bytes(chunk);
                            if st == 1 {
                                maximized = true;
                            }
                        }
                    }
                    if w > 0 && h > 0 {
                        self.pending_width = w as u32;
                        self.pending_height = h as u32;
                    }
                    self.pending_maximized = maximized;
                }
                1 => {
                    out_events.push(WaylandEvent::Close);
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id == self.xdg_surface_id && opcode == 0 {
            // xdg_surface.configure(serial) -> ack_configure(serial)
            let mut off = 0;
            if let Some(serial) = read_u32(payload, &mut off) {
                self.send_request(self.xdg_surface_id, 4, &[serial])?;
                self.configured = true;
                out_events.push(WaylandEvent::Configure {
                    width: self.pending_width,
                    height: self.pending_height,
                    maximized: self.pending_maximized,
                });
            }
            return Ok(());
        }

        if obj_id == self.pointer_id && self.pointer_id != 0 {
            match opcode {
                0 => {
                    // enter(serial, surface, sx, sy)
                    let mut off = 0;
                    let serial = read_u32(payload, &mut off).unwrap_or(0);
                    let _surf = read_u32(payload, &mut off);
                    self.pointer_enter_serial = serial;
                    self.apply_cursor_shape(self.current_cursor_shape);
                    if let (Some(sx), Some(sy)) =
                        (read_i32(payload, &mut off), read_i32(payload, &mut off))
                    {
                        self.pointer_x = wl_fixed_to_f32(sx);
                        self.pointer_y = wl_fixed_to_f32(sy);
                        out_events.push(WaylandEvent::PointerMotion {
                            x: self.pointer_x,
                            y: self.pointer_y,
                        });
                    }
                }
                2 => {
                    // motion(time, sx, sy)
                    let mut off = 0;
                    let _time = read_u32(payload, &mut off);
                    if let (Some(sx), Some(sy)) =
                        (read_i32(payload, &mut off), read_i32(payload, &mut off))
                    {
                        self.pointer_x = wl_fixed_to_f32(sx);
                        self.pointer_y = wl_fixed_to_f32(sy);
                        out_events.push(WaylandEvent::PointerMotion {
                            x: self.pointer_x,
                            y: self.pointer_y,
                        });
                    }
                }
                3 => {
                    // button(serial, time, button, state)
                    let mut off = 0;
                    let serial = read_u32(payload, &mut off).unwrap_or(0);
                    let _time = read_u32(payload, &mut off);
                    let button = read_u32(payload, &mut off).unwrap_or(0);
                    let state = read_u32(payload, &mut off).unwrap_or(0);
                    if state == 1 {
                        self.last_button_serial = serial;
                        self.last_input_serial = serial;
                    }
                    out_events.push(WaylandEvent::PointerButton {
                        button,
                        pressed: state == 1,
                        serial,
                        x: self.pointer_x,
                        y: self.pointer_y,
                    });
                }
                4 => {
                    // axis(time, axis, value)
                    let mut off = 0;
                    let _time = read_u32(payload, &mut off);
                    let axis = read_u32(payload, &mut off).unwrap_or(0);
                    let val = read_i32(payload, &mut off).unwrap_or(0);
                    if axis == 0 && val != 0 {
                        let raw = wl_fixed_to_f32(val);
                        let delta_y = if raw.abs() < 15.0 {
                            raw * 4.8
                        } else {
                            raw * 3.2
                        };
                        out_events.push(WaylandEvent::PointerScroll {
                            delta_y,
                            x: self.pointer_x,
                            y: self.pointer_y,
                            ctrl: self.mods.ctrl,
                        });
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if obj_id == self.keyboard_id && self.keyboard_id != 0 {
            match opcode {
                0 => {
                    // wl_keyboard.keymap(format, fd, size) -> close the received keymap fd
                    if let Some(fd) = self.received_fds.pop_front()
                        && fd >= 0
                    {
                        unsafe {
                            let _ = close(fd);
                        }
                    }
                }
                3 => {
                    // key(serial, time, key, state)
                    let mut off = 0;
                    let serial = read_u32(payload, &mut off).unwrap_or(0);
                    let _time = read_u32(payload, &mut off);
                    let scancode = read_u32(payload, &mut off).unwrap_or(0);
                    let state = read_u32(payload, &mut off).unwrap_or(0);
                    if state == 1 || state == 2 {
                        self.last_input_serial = serial;
                        let key = key_from_evdev(scancode, self.mods.shift, self.caps_lock);
                        if key != Key::Other {
                            out_events.push(WaylandEvent::KeyPress {
                                key,
                                mods: self.mods,
                            });
                        }
                    }
                }
                4 => {
                    // modifiers(serial, mods_depressed, mods_latched, mods_locked, group)
                    let mut off = 0;
                    let _serial = read_u32(payload, &mut off);
                    let depressed = read_u32(payload, &mut off).unwrap_or(0);
                    let latched = read_u32(payload, &mut off).unwrap_or(0);
                    let locked = read_u32(payload, &mut off).unwrap_or(0);
                    let active = depressed | latched;
                    self.mods = Modifiers {
                        shift: (active & 1) != 0,
                        ctrl: (active & 4) != 0,
                        alt: (active & 8) != 0,
                    };
                    self.caps_lock = (locked & 2) != 0;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn bind_global(
        &mut self,
        name: u32,
        interface: &str,
        version: u32,
        new_id: u32,
    ) -> Result<(), String> {
        let mut words = Vec::with_capacity(8);
        words.push(name);
        push_wl_string(&mut words, interface);
        words.push(version);
        words.push(new_id);
        self.send_request(WL_REGISTRY_ID, 0, &words)
    }

    /// Update the pointer cursor shape when hovering links, text inputs, or window resize borders.
    pub fn set_cursor_shape(&mut self, shape: WaylandCursorShape) {
        if self.current_cursor_shape == shape {
            return;
        }
        self.current_cursor_shape = shape;
        self.apply_cursor_shape(shape);
    }

    fn apply_cursor_shape(&mut self, shape: WaylandCursorShape) {
        if self.pointer_enter_serial == 0 {
            return;
        }
        if self.cursor_shape_device_id != 0 {
            let code = wp_cursor_shape_code(shape);
            let _ = self.send_request(
                self.cursor_shape_device_id,
                1,
                &[self.pointer_enter_serial, code],
            );
            let _ = self.stream.flush();
            return;
        }
        if self.pointer_id == 0 || self.compositor_id == 0 || self.shm_id == 0 {
            return;
        }
        if self.cursor_surface_id == 0 {
            let sid = self.alloc_id();
            self.cursor_surface_id = sid;
            let _ = self.send_request(self.compositor_id, 0, &[sid]);
        }
        let cursor_bytes = 24 * 24 * 4;
        if self.cursor_pool.is_none() {
            let name = CString::new("anvil-wl-cursor").unwrap();
            let fd = unsafe { memfd_create(name.as_ptr(), MFD_CLOEXEC) };
            if fd >= 0 {
                if unsafe { ftruncate(fd, cursor_bytes as i64) } == 0 {
                    let ptr = unsafe {
                        mmap(
                            null_mut(),
                            cursor_bytes,
                            PROT_READ | PROT_WRITE,
                            MAP_SHARED,
                            fd,
                            0,
                        )
                    };
                    if ptr != MAP_FAILED && !ptr.is_null() {
                        let pool_id = self.alloc_id();
                        let buf_id = self.alloc_id();
                        if self
                            .send_request_with_fd(
                                self.shm_id,
                                0,
                                &[pool_id, cursor_bytes as u32],
                                fd,
                            )
                            .is_ok()
                        {
                            let _ = self.send_request(
                                pool_id,
                                0,
                                &[buf_id, 0, 24, 24, 24 * 4, WL_SHM_FORMAT_ARGB8888],
                            );
                            self.cursor_pool = Some(ShmSurfacePool {
                                fd,
                                ptr: ptr.cast::<u8>(),
                                capacity: cursor_bytes,
                                pool_id,
                                buffer_id: buf_id,
                                buf_w: 24,
                                buf_h: 24,
                            });
                        } else {
                            unsafe {
                                let _ = munmap(ptr, cursor_bytes);
                                let _ = close(fd);
                            }
                        }
                    } else {
                        unsafe {
                            let _ = close(fd);
                        }
                    }
                } else {
                    unsafe {
                        let _ = close(fd);
                    }
                }
            }
        }
        let Some(pool) = self.cursor_pool.as_mut() else {
            return;
        };
        let (argb, hot_x, hot_y) = render_software_cursor_argb(shape);
        unsafe {
            std::ptr::copy_nonoverlapping(argb.as_ptr(), pool.ptr, cursor_bytes);
        }
        let buf_id = pool.buffer_id;
        let _ = self.send_request(self.cursor_surface_id, 1, &[buf_id, 0, 0]);
        let _ = self.send_request(self.cursor_surface_id, 2, &[0, 0, 24, 24]);
        let _ = self.send_request(self.cursor_surface_id, 6, &[]);
        let _ = self.send_request(
            self.pointer_id,
            0,
            &[
                self.pointer_enter_serial,
                self.cursor_surface_id,
                hot_x as u32,
                hot_y as u32,
            ],
        );
        let _ = self.stream.flush();
    }

    /// Publish UTF-8 text to the Wayland clipboard selection (`wl_data_device.set_selection`).
    pub fn set_clipboard_text(&mut self, text: &str) {
        self.clipboard_text = text.to_string();
        if self.data_device_mgr_id == 0 || self.data_device_id == 0 {
            return;
        }
        if self.data_source_id != 0 {
            let _ = self.send_request(self.data_source_id, 1, &[]);
            self.data_source_id = 0;
        }
        let src_id = self.alloc_id();
        self.data_source_id = src_id;
        if self
            .send_request(self.data_device_mgr_id, 0, &[src_id])
            .is_err()
        {
            return;
        }
        for mime in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
            let mut words = Vec::new();
            push_wl_string(&mut words, mime);
            let _ = self.send_request(src_id, 0, &words);
        }
        let serial = if self.last_input_serial != 0 {
            self.last_input_serial
        } else {
            self.pointer_enter_serial
        };
        let _ = self.send_request(self.data_device_id, 1, &[src_id, serial]);
        let _ = self.stream.flush();
    }

    /// Read UTF-8 text from the active Wayland clipboard selection (`wl_data_offer.receive`).
    pub fn read_clipboard_text(&mut self) -> Option<String> {
        if self.selection_offer_id == 0 || !self.selection_offer_has_text {
            return (!self.clipboard_text.is_empty()).then(|| self.clipboard_text.clone());
        }
        let mut fds = [-1 as c_int; 2];
        if unsafe { pipe(fds.as_mut_ptr()) } < 0 {
            return (!self.clipboard_text.is_empty()).then(|| self.clipboard_text.clone());
        }
        let read_fd = fds[0];
        let write_fd = fds[1];

        let mut words = Vec::new();
        push_wl_string(&mut words, "text/plain;charset=utf-8");
        let sent = self.send_request_with_fd(self.selection_offer_id, 1, &words, write_fd);
        unsafe {
            let _ = close(write_fd);
        }
        if sent.is_err() {
            unsafe {
                let _ = close(read_fd);
            }
            return (!self.clipboard_text.is_empty()).then(|| self.clipboard_text.clone());
        }
        let _ = self.stream.flush();
        let _ = self.roundtrip();

        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        for _ in 0..16 {
            let n = unsafe { read(read_fd, buf.as_mut_ptr().cast::<c_void>(), buf.len()) };
            if n <= 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        unsafe {
            let _ = close(read_fd);
        }
        if out.is_empty() {
            (!self.clipboard_text.is_empty()).then(|| self.clipboard_text.clone())
        } else {
            let s = String::from_utf8_lossy(&out).into_owned();
            self.clipboard_text = s.clone();
            Some(s)
        }
    }

    /// Update the Wayland window title (`xdg_toplevel.set_title`).
    pub fn set_title(&mut self, title: &str) {
        let mut words = Vec::new();
        push_wl_string(&mut words, title);
        let _ = self.send_request(self.xdg_toplevel_id, 2, &words);
    }

    /// Set the Wayland application ID (`xdg_toplevel.set_app_id`).
    pub fn set_app_id(&mut self, app_id: &str) {
        let mut words = Vec::new();
        push_wl_string(&mut words, app_id);
        let _ = self.send_request(self.xdg_toplevel_id, 3, &words);
    }

    /// Start an interactive compositor window move (`xdg_toplevel.move`).
    pub fn begin_interactive_move(&mut self) {
        if self.seat_id != 0 && self.last_button_serial != 0 {
            let _ = self.send_request(
                self.xdg_toplevel_id,
                5,
                &[self.seat_id, self.last_button_serial],
            );
        }
    }

    /// Start an interactive compositor window edge resize (`xdg_toplevel.resize`).
    pub fn begin_interactive_resize(&mut self, edges: u32) {
        if self.seat_id != 0 && self.last_button_serial != 0 && edges != XDG_RESIZE_NONE {
            let _ = self.send_request(
                self.xdg_toplevel_id,
                6,
                &[self.seat_id, self.last_button_serial, edges],
            );
        }
    }

    /// Minimize the window (`xdg_toplevel.set_minimized`).
    pub fn minimize(&mut self) {
        let _ = self.send_request(self.xdg_toplevel_id, 13, &[]);
    }

    /// Toggle maximize / restore (`xdg_toplevel.set_maximized` / `unset_maximized`).
    pub fn set_maximized(&mut self, maximize: bool) {
        let opcode = if maximize { 9 } else { 10 };
        let _ = self.send_request(self.xdg_toplevel_id, opcode, &[]);
    }

    /// Poll all pending Wayland events without blocking.
    pub fn poll_events(&mut self) -> Result<Vec<WaylandEvent>, String> {
        let mut events = Vec::new();
        loop {
            let n = self.read_socket(true)?;
            let mut dummy = false;
            self.drain_messages(&mut events, None, &mut dummy)?;
            if n == 0 {
                break;
            }
        }
        Ok(events)
    }

    fn ensure_shm_buffer(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<(*mut u8, usize, u32, bool), String> {
        let needed = (width as usize) * (height as usize) * 4;
        let recreate_pool = self.shm_pool.as_ref().is_none_or(|p| p.capacity < needed);
        let mut buffer_recreated = false;
        if recreate_pool {
            if let Some(old) = self.shm_pool.take() {
                if old.buffer_id != 0 {
                    let _ = self.send_request(old.buffer_id, 0, &[]);
                }
                if old.pool_id != 0 {
                    let _ = self.send_request(old.pool_id, 1, &[]);
                }
            }
            let cap = needed.max(1920 * 1080 * 4);
            let name = CString::new("anvil-wl-shm").unwrap();
            let fd = unsafe { memfd_create(name.as_ptr(), MFD_CLOEXEC) };
            if fd < 0 {
                return Err("memfd_create failed for Wayland shared memory".to_string());
            }
            if unsafe { ftruncate(fd, cap as i64) } < 0 {
                unsafe {
                    let _ = close(fd);
                }
                return Err("ftruncate failed for Wayland shared memory".to_string());
            }
            let ptr = unsafe { mmap(null_mut(), cap, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) };
            if ptr == MAP_FAILED || ptr.is_null() {
                unsafe {
                    let _ = close(fd);
                }
                return Err("mmap failed for Wayland shared memory".to_string());
            }
            let pool_id = self.alloc_id();
            self.send_request_with_fd(self.shm_id, 0, &[pool_id, cap as u32], fd)?;
            self.shm_pool = Some(ShmSurfacePool {
                fd,
                ptr: ptr.cast::<u8>(),
                capacity: cap,
                pool_id,
                buffer_id: 0,
                buf_w: 0,
                buf_h: 0,
            });
            buffer_recreated = true;
        }

        let need_new_buffer = self
            .shm_pool
            .as_ref()
            .is_some_and(|p| p.buffer_id == 0 || p.buf_w != width || p.buf_h != height);
        if need_new_buffer {
            let old_buf_id = self.shm_pool.as_ref().map(|p| p.buffer_id).unwrap_or(0);
            if old_buf_id != 0 {
                let _ = self.send_request(old_buf_id, 0, &[]);
            }
            let buf_id = self.alloc_id();
            let pool_id = self.shm_pool.as_ref().unwrap().pool_id;
            let stride = width * 4;
            self.send_request(
                pool_id,
                0,
                &[buf_id, 0, width, height, stride, WL_SHM_FORMAT_XRGB8888],
            )?;
            if let Some(pool) = self.shm_pool.as_mut() {
                pool.buffer_id = buf_id;
                pool.buf_w = width;
                pool.buf_h = height;
            }
            if self.xdg_surface_id != 0 {
                let _ = self.send_request(self.xdg_surface_id, 3, &[0, 0, width, height]);
            }
            buffer_recreated = true;
        }

        let pool = self.shm_pool.as_mut().unwrap();
        Ok((pool.ptr, needed, pool.buffer_id, buffer_recreated))
    }

    fn commit_damage(
        &mut self,
        buffer_id: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> Result<(), String> {
        self.send_request(self.surface_id, 1, &[buffer_id, 0, 0])?;
        let dmg_op = if self.compositor_version >= 4 { 9 } else { 2 };
        self.send_request(self.surface_id, dmg_op, &[x, y, w, h])?;
        if self.frame_callback_id == 0 && self.surface_id != 0 {
            let cb_id = self.alloc_id();
            self.frame_callback_id = cb_id;
            self.frame_pending = true;
            // wl_surface.frame(callback: new_id<wl_callback>) is opcode 3
            self.send_request(self.surface_id, 3, &[cb_id])?;
        }
        self.send_request(self.surface_id, 6, &[])?;
        let _ = self.stream.flush();
        Ok(())
    }

    /// Present an [`anvil_raster::Framebuffer`] directly into the `mmap`ped `wl_shm` buffer
    /// using SSE2 RGBA-to-BGRA conversion without allocating an intermediate `Vec<u8>`.
    pub fn present_framebuffer(&mut self, fb: &anvil_raster::Framebuffer) -> Result<(), String> {
        let fb_w = fb.width();
        let fb_h = fb.height();
        if !self.configured || fb_w == 0 || fb_h == 0 {
            return Ok(());
        }
        let (ptr, needed, buffer_id, _) = self.ensure_shm_buffer(fb_w, fb_h)?;
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, needed) };
        fb.copy_bgra_rows_top_down(dst, 0, fb_h);
        self.commit_damage(buffer_id, 0, 0, fb_w, fb_h)
    }

    /// Convert and damage only scanlines `y0..y1` of `fb` into the `mmap`ped `wl_shm` buffer.
    /// Automatically falls back to a full-frame update if the surface buffer was just created or resized.
    pub fn present_dirty_rows(
        &mut self,
        fb: &anvil_raster::Framebuffer,
        y0: u32,
        y1: u32,
    ) -> Result<(), String> {
        let fb_w = fb.width();
        let fb_h = fb.height();
        if !self.configured || fb_w == 0 || fb_h == 0 {
            return Ok(());
        }
        let top = y0.min(fb_h);
        let bottom = y1.min(fb_h);
        if top >= bottom {
            return Ok(());
        }
        let (ptr, needed, buffer_id, recreated) = self.ensure_shm_buffer(fb_w, fb_h)?;
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, needed) };
        if recreated {
            fb.copy_bgra_rows_top_down(dst, 0, fb_h);
            self.commit_damage(buffer_id, 0, 0, fb_w, fb_h)
        } else {
            fb.copy_bgra_rows_top_down(dst, top, bottom);
            self.commit_damage(buffer_id, 0, top, fb_w, bottom - top)
        }
    }

    /// Present a top-down `BGRA` (`XRGB8888`) pixel buffer to the Wayland surface.
    pub fn present_bgra(&mut self, bgra: &[u8], width: u32, height: u32) -> Result<(), String> {
        if !self.configured || width == 0 || height == 0 {
            return Ok(());
        }
        let needed = (width as usize) * (height as usize) * 4;
        if bgra.len() < needed {
            return Ok(());
        }
        let (ptr, _, buffer_id, _) = self.ensure_shm_buffer(width, height)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), ptr, needed);
        }
        self.commit_damage(buffer_id, 0, 0, width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_wayland_wire_messages_and_fixed_point() {
        let mut words = Vec::new();
        words.push(7);
        push_wl_string(&mut words, "wl_compositor");
        words.push(4);
        words.push(3);
        let msg = encode_wl_message(WL_REGISTRY_ID, 0, &words);
        assert_eq!(msg.len(), 8 + 4 + 20 + 4 + 4);

        let mut off = 8;
        assert_eq!(read_u32(&msg, &mut off), Some(7));
        assert_eq!(
            read_wl_string(&msg, &mut off).as_deref(),
            Some("wl_compositor")
        );
        assert_eq!(read_u32(&msg, &mut off), Some(4));
        assert_eq!(read_u32(&msg, &mut off), Some(3));
        assert!((wl_fixed_to_f32(256 * 42 + 128) - 42.5).abs() < 1e-4);
        assert_eq!(hit_code_to_wayland_edge(13), Some(XDG_RESIZE_TOP_LEFT));
        assert_eq!(hit_code_to_wayland_edge(17), Some(XDG_RESIZE_BOTTOM_RIGHT));
    }

    #[test]
    fn maps_cursor_shapes_and_fractional_scales() {
        assert_eq!(wp_cursor_shape_code(WaylandCursorShape::Default), 1);
        assert_eq!(wp_cursor_shape_code(WaylandCursorShape::Pointer), 4);
        assert_eq!(wp_cursor_shape_code(WaylandCursorShape::Text), 9);
        assert_eq!(
            hit_code_to_cursor_shape(10),
            Some(WaylandCursorShape::ResizeW)
        );
        assert_eq!(
            hit_code_to_cursor_shape(17),
            Some(WaylandCursorShape::ResizeSE)
        );
        assert_eq!(fractional_scale_120_to_percent(120), 100);
        assert_eq!(fractional_scale_120_to_percent(150), 125);
        assert_eq!(fractional_scale_120_to_percent(180), 150);
        assert_eq!(fractional_scale_120_to_percent(210), 175);
        assert_eq!(fractional_scale_120_to_percent(240), 200);

        let (argb, hx, hy) = render_software_cursor_argb(WaylandCursorShape::Pointer);
        assert_eq!(argb.len(), 24 * 24 * 4);
        assert_eq!((hx, hy), (7, 2));
        assert!(argb.iter().any(|&b| b != 0));
    }

    #[test]
    fn registers_and_completes_wl_surface_frame_callback() {
        let (client_stream, mut server_stream) = UnixStream::pair().unwrap();
        let mut win = WaylandWindow::new_unconnected(client_stream);
        win.surface_id = 10;
        win.compositor_version = 4;
        assert!(!win.is_frame_pending());

        win.commit_damage(20, 0, 32, 640, 128).unwrap();
        assert!(win.is_frame_pending());
        let cb_id = win.frame_callback_id;
        assert!(cb_id >= 3);

        // Simulate compositor firing wl_callback.done(timestamp_ms = 123456)
        let done_msg = encode_wl_message(cb_id, 0, &[123_456]);
        server_stream.write_all(&done_msg).unwrap();
        let events = win.poll_events().unwrap();
        assert_eq!(
            events,
            vec![WaylandEvent::FrameDone {
                timestamp_ms: 123_456
            }]
        );
        assert!(!win.is_frame_pending());
        assert_eq!(win.last_frame_timestamp_ms(), 123_456);
    }
}
