// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Command-line entrypoint and process-role dispatcher for the `anvil` binary.
//!
//! # Process Architecture Overview
//!
//! A single `anvil` executable plays every role in the browser's multi-process
//! architecture depending on its command-line arguments:
//!
//! 1. **Main Browser / UI Shell Process (`anvil`, `anvil open [target]`)**:
//!    - Applies mandatory OS sandbox limits (`enforce_mandatory_sandbox_or_abort`)
//!      before parsing any user input or opening any network socket.
//!    - Creates the native desktop window (`shell::open` via Wayland `wl_shm` on
//!      Linux or Direct2D + Win32 on Windows).
//!    - Owns the network stack (`anvil-net`), persistent state (`anvil-storage`),
//!      and shields (`adblock-rust`).
//!    - Spawns one isolated `--renderer` child process per open tab for `http://`,
//!      `https://`, and local file pages. Built-in `anvil://` pages stay inside
//!      the main browser process.
//!    - When the user closes the window cleanly, the session records
//!      `ERROR ID-1 (User Closed the Browser)` as a normal exit status.
//!    - If an unhandled panic occurs in the main browser process, the installed
//!      panic hook logs `ERROR ID-2 (Browser Process Crashed)` to `stderr`.
//!
//! 2. **Confined Per-Tab Renderer Worker (`anvil --renderer`)**:
//!    - Spawned by `anvil-sandbox` (wrapped in `bwrap`/`unshare` + `prlimit` on
//!      Linux when available) with an empty environment.
//!    - Immediately locks down worker OS limits (`lock_current_worker_process_or_abort`)
//!      and disables all in-process networking (`anvil_net::turn_off_network`).
//!    - Speaks length-prefixed binary frames (`anvil_sandbox::wire`) over `stdin`
//!      and `stdout`.
//!    - Whenever HTML parsing, CSS layout, or Cinder script execution requires
//!      an external script, image, or `fetch()` resource, the renderer sends a
//!      `Message::Fetch` request back over `stdout` to the parent browser and
//!      waits for `Message::FetchReply` on `stdin`.
//!
//! 3. **Headless Inspection & Packaging Commands (`render`, `raster`, `install`, `uninstall`)**:
//!    - `anvil render <target>`: Runs the HTML/CSS/layout/paint pipeline and
//!      prints the resulting `anvil-paint` display list commands as text.
//!    - `anvil raster <target> <out.ppm>`: Rasterizes a page into an uncompressed
//!      PPM image file via `anvil-raster`.
//!    - `anvil pdf <target> <out.pdf>`: Saves the whole page as a PDF, one
//!      8.5 by 11 sheet at a time, the same as `Ctrl+P` in the window.
//!    - `anvil install` / `anvil uninstall`: Installs or removes the per-user
//!      application binary and desktop shortcuts.

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anvil_browser::installer::{install_default, uninstall_default};
use anvil_browser::{
    BrowserFetch, ErrorId, VERSION, Viewport, classify_error_reason,
    enforce_mandatory_sandbox_or_abort, load_target, lock_current_worker_process_or_abort,
    page_frame_owned, render_with_browser_fetches,
};
use anvil_paint::DisplayItem;
use anvil_raster::rasterize;
use anvil_sandbox::wire::{
    self, Direction, FailureCode, FetchKind, FetchReply, FetchRequest, MAX_DOCUMENT_BYTES,
    MAX_FETCHES_PER_RENDER, MAX_REPORTED_NODES, MAX_TITLE_BYTES, Message, RenderFailure,
    RenderRequest, RenderedDocument, RenderedFrame, Role, WIRE_VERSION,
};

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    /// Attach to a console. `main` passes the parent-process id.
    fn AttachConsole(dw_process_id: u32) -> i32;
}

/// Entrypoint of the `anvil` executable.
///
/// Execution proceeds in four strict phases:
/// 1. **Mandatory Sandbox Enforcement**: Calls [`enforce_mandatory_sandbox_or_abort`]
///    immediately. If the operating system refuses to apply process hardening
///    (such as `PR_SET_NO_NEW_PRIVS` on Linux or Job/Mitigation policies on
///    Windows), the process aborts immediately (`ERROR ID-3`).
/// 2. **Crash Diagnostics Hook**: Installs a panic hook on the main browser
///    process so that any unhandled fatal panic is tagged with
///    `ERROR ID-2 (Browser Process Crashed)` in diagnostic logs.
/// 3. **Console Attachment (Windows)**: Attaches to the parent terminal if
///    launched from `cmd.exe` or PowerShell so CLI output remains visible.
/// 4. **Command Dispatch**: Delegates to [`run`] to execute the requested
///    subcommand or open the graphical browser window.
fn main() -> ExitCode {
    // Phase 1: Quit immediately if mandatory OS sandbox limits cannot be set.
    let _mandatory_sandbox = enforce_mandatory_sandbox_or_abort();

    // Phase 2: Tag unhandled main-process panics with ERROR ID-2 so users and
    // diagnostic logs can immediately identify a browser-level crash.
    let is_renderer = env::args().skip(1).any(|arg| arg == "--renderer");
    if !is_renderer {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let crash_id = ErrorId::BrowserCrashed;
            eprintln!("[{}] {} — {}", crash_id.code(), crash_id.title(), info);
            default_hook(info);
        }));
    }

    // Phase 3: On Windows GUI builds (`windows_subsystem = "windows"`), attach
    // to the parent console when launched from a terminal session.
    #[cfg(windows)]
    unsafe {
        let _ = AttachConsole(0xFFFF_FFFF);
    }

    // Phase 4: Run the requested CLI command or graphical window loop.
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let error_id = classify_error_reason(&message);
            eprintln!("[{}] {}: {message}", error_id.code(), error_id.title());
            ExitCode::FAILURE
        }
    }
}

/// Inspect `std::env::args()` and route execution to the matching subsystem.
///
/// The first token after the binary path determines the mode unless the user
/// passed a bare URL or HTML file path directly (in which case the graphical
/// window opens directly to that target).
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args()
        .skip(1)
        .filter(|a| !a.trim().is_empty())
        .collect();
    match args.first().map(String::as_str) {
        // No arguments: if invoked under a Setup/Uninstall filename, perform that action; then open the window.
        None => {
            if let Ok(exe) = env::current_exe()
                && let Some(name) = exe.file_name().and_then(|s| s.to_str())
            {
                let lower = name.to_ascii_lowercase();
                if lower.contains("uninstall") {
                    let dir = uninstall_default()?;
                    println!("uninstalled Anvil Browser {VERSION} from {}", dir.display());
                    return Ok(());
                }
                if lower.contains("setup")
                    && let Ok(report) = install_default()
                {
                    println!(
                        "installed Anvil Browser {VERSION} to {}",
                        report.installed_binary.display()
                    );
                }
            }
            open_window(&[])
        }
        // Wire-format frames in on stdin and out on stdout. The window starts one per tab.
        Some("--renderer") => run_renderer(),
        // Print display-list lines. No window and no PPM file.
        Some("render") => render(&args[1..]),
        // Headless PPM. No window.
        Some("raster") => raster(&args[1..]),
        // Headless PDF. No window.
        Some("pdf") => save_pdf(&args[1..]),
        // Window, with an optional file or URL after the flags.
        Some("open" | "view") => open_window(&args[1..]),
        // Copy into the per-user install directory via `installer::install_default`.
        Some("install" | "--install") => {
            let report = install_default()?;
            println!(
                "installed Anvil Browser {VERSION} to {}",
                report.installed_binary.display()
            );
            for sc in &report.shortcut_paths {
                println!("shortcut: {}", sc.display());
            }
            Ok(())
        }
        // Remove that install directory and its shortcuts via `installer::uninstall_default`.
        Some("uninstall" | "--uninstall") => {
            let dir = uninstall_default()?;
            println!("uninstalled Anvil Browser {VERSION} from {}", dir.display());
            Ok(())
        }
        // Print `anvil-browser` and the `VERSION` constant.
        Some("version" | "--version" | "-v") => {
            println!("anvil-browser {VERSION}");
            Ok(())
        }
        // Print the command list. This path does not open a window.
        Some("help" | "--help" | "-h") => {
            println!("anvil-browser {VERSION}");
            println!("anvil [file or http/https/anvil URL]");
            println!("anvil open [--width N] [--height N] [file or http/https/anvil URL]");
            println!("anvil render [--width N] <file or http/https/anvil URL>");
            println!(
                "anvil raster [--width N] [--height N] <file or http/https/anvil URL> <output.ppm>"
            );
            println!("anvil pdf [--width N] <file or http/https/anvil URL> <output.pdf>");
            println!("anvil install");
            println!("anvil uninstall");
            Ok(())
        }
        // A bare http(s) URL, anvil:// URL, file:// URL, or local file path opens a window on that target.
        Some(target)
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("anvil://")
                || target.starts_with("file://")
                || target.starts_with("view-source:")
                || target.ends_with(".html")
                || target.ends_with(".htm")
                || std::path::Path::new(target).is_file() =>
        {
            open_window(&args)
        }
        Some(command) => Err(format!("unknown command {command}")),
    }
}

/// Serve `anvil_sandbox::wire` protocol requests inside a confined `--renderer` worker process.
///
/// # Security & Lifecycle Steps
/// 1. **Lock Worker OS Limits**: Calls [`lock_current_worker_process_or_abort`] to
///    enforce the strict renderer sandbox profile (no child process creation,
///    zero file-creation budget, and hardened memory/privilege flags).
/// 2. **Disable In-Process Networking**: Calls [`anvil_net::turn_off_network`] so
///    that any direct socket attempt inside the worker fails immediately. All
///    external bytes must be requested from the parent browser over the wire pipe.
/// 3. **Handshake (`Message::Hello`)**: Reads the initial `Hello` frame from
///    the parent browser on `stdin`, verifies `WIRE_VERSION`, and replies with
///    `Role::Renderer` on `stdout`.
/// 4. **Message Loop**: Processes one request at a time from the browser:
///    - `Message::Ping { nonce }` -> Replies `Message::Pong { nonce }` for
///      liveness monitoring.
///    - `Message::StatusQuery` -> Inspects `/proc/self` namespaces and visible
///      PIDs via `anvil_sandbox::confine::read_own_status()` so the parent can
///      verify namespace confinement.
///    - `Message::Render(req)` -> Runs the HTML5/CSS/Layout/Paint/Script pipeline
///      via [`render_for_browser`], streaming back both the binary `RenderedFrame`
///      display list and the final `RenderedDocument`.
///    - `Message::Shutdown` or `UnexpectedEof` -> Exits cleanly (`Ok(())`).
fn run_renderer() -> Result<(), String> {
    // Step 1: Lock OS worker sandbox limits before touching untrusted bytes.
    lock_current_worker_process_or_abort();
    // Step 2: Permanently disable direct networking in this process.
    anvil_net::turn_off_network();
    let stdin = io::stdin();
    let stdout = io::stdout();
    // Step 3: Complete the initial protocol version and role handshake.
    let (seq, hello) =
        wire::read_message(&mut stdin.lock(), Direction::ToRenderer).map_err(|e| e.to_string())?;
    match hello {
        Message::Hello {
            role: Role::Browser,
            version: WIRE_VERSION,
        } => {}
        _ => return Err("renderer: expected the browser's hello".to_string()),
    }
    let reply = Message::Hello {
        role: Role::Renderer,
        version: WIRE_VERSION,
    };
    wire::write_message(&mut stdout.lock(), seq, &reply).map_err(|e| e.to_string())?;
    // Step 4: Enter the synchronous request/response loop over stdin/stdout.
    loop {
        let (seq, message) = match wire::read_message(&mut stdin.lock(), Direction::ToRenderer) {
            Ok(frame) => frame,
            // The browser closed the pipe. That is a normal way to stop.
            Err(wire::WireError::Io(io::ErrorKind::UnexpectedEof)) => return Ok(()),
            Err(err) => return Err(format!("renderer: {err}")),
        };
        let answer = match message {
            Message::Ping { nonce } => Message::Pong { nonce },
            Message::StatusQuery => Message::Status(anvil_sandbox::confine::read_own_status()),
            Message::Render(req) => render_for_browser(&req, seq)?,
            Message::Shutdown => return Ok(()),
            _ => return Err("renderer: unexpected message from the browser".to_string()),
        };
        wire::write_message(&mut stdout.lock(), seq, &answer).map_err(|e| e.to_string())?;
    }
}

/// Synchronous RPC bridge used by a confined `--renderer` worker to ask the
/// parent browser process for external subresources during a single `Render`.
///
/// Because the renderer worker has no network access and no filesystem write
/// budget, every external `<script src>`, `<img src>`, or JavaScript `fetch()`
/// call invokes [`BrowserLink::fetch`]:
/// - Sends a `Message::Fetch(FetchRequest)` frame on `stdout` tagged with the
///   active `seq` and `nav_id`.
/// - Waits on `stdin` for the matching `Message::FetchReply`.
/// - Enforces `MAX_FETCHES_PER_RENDER` (`ERROR ID-7`) so a runaway page cannot
///   flood the IPC pipe.
/// - Marks `broken = true` if the pipe fails or returns an out-of-order frame,
///   short-circuiting all subsequent subresource requests for this render.
struct BrowserLink {
    /// Wire sequence number of the enclosing `Message::Render` request.
    seq: u32,
    /// Navigation identifier assigned by the parent tab controller.
    nav_id: u64,
    /// Number of subresource fetches requested so far during this render pass.
    asked: AtomicUsize,
    /// Set to `true` if the IPC pipe encounters an unexpected message or I/O error.
    broken: AtomicBool,
}

impl BrowserLink {
    /// Request a subresource (`kind`, `url`) from the parent browser over the IPC pipe.
    fn fetch(&self, kind: FetchKind, url: &str) -> Option<FetchReply> {
        if self.broken.load(Ordering::SeqCst) {
            return None;
        }
        // The browser kills a renderer that asks for more than MAX_FETCHES_PER_RENDER (ERROR ID-7).
        if self.asked.fetch_add(1, Ordering::SeqCst) >= MAX_FETCHES_PER_RENDER {
            return None;
        }
        let want = Message::Fetch(FetchRequest {
            nav_id: self.nav_id,
            kind,
            url: url.to_string(),
        });
        // A URL the wire won't carry, such as one over 8 KiB, is treated as a
        // local failed fetch without poisoning the wire pipe.
        let frame = wire::encode(self.seq, &want).ok()?;
        let answer = {
            let mut out = io::stdout().lock();
            io::Write::write_all(&mut out, &frame)
                .and_then(|()| io::Write::flush(&mut out))
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    wire::read_message(&mut io::stdin().lock(), Direction::ToRenderer)
                        .map_err(|e| e.to_string())
                })
        };
        match answer {
            Ok((seq, Message::FetchReply(reply)))
                if seq == self.seq && reply.nav_id == self.nav_id =>
            {
                Some(reply)
            }
            _ => {
                self.broken.store(true, Ordering::SeqCst);
                None
            }
        }
    }
}

/// Execute a full HTML -> CSS -> Layout -> Paint -> Script render pass inside the
/// confined renderer worker and package the result into wire messages.
///
/// # Wire Protocol Ordering
/// 1. Subresource fetches (`Message::Fetch` / `Message::FetchReply`) occur inline
///    while [`render_with_browser_fetches`] parses the document, evaluates scripts,
///    and decodes images.
/// 2. Once the page finishes rendering, the serialized `anvil-paint` display list
///    is sent first as an intermediate `Message::Frame(RenderedFrame)` carrying the
///    same `seq` number.
/// 3. Finally, `Ok(Message::Rendered(RenderedDocument))` (or `Message::Failed` with
///    `FailureCode::TooLarge`, corresponding to `ERROR ID-9`) is returned to [`run_renderer`]
///    to conclude the request.
fn render_for_browser(req: &RenderRequest, seq: u32) -> Result<Message, String> {
    let link = Arc::new(BrowserLink {
        seq,
        nav_id: req.nav_id,
        asked: AtomicUsize::new(0),
        broken: AtomicBool::new(false),
    });
    let ask: BrowserFetch = {
        let link = Arc::clone(&link);
        Arc::new(move |kind, url| link.fetch(kind, url))
    };
    let viewport = Viewport {
        width: req.viewport_width.max(1) as f32,
        height: req.viewport_height.max(1) as f32,
    };
    let page = render_with_browser_fetches(
        &req.html,
        &req.url,
        viewport,
        req.csp_header.as_deref(),
        req.scripts_enabled,
        ask,
    );
    if link.broken.load(Ordering::SeqCst) {
        return Err("renderer: the browser broke off a fetch".to_string());
    }
    let html = page.document.to_html();
    if html.len() > MAX_DOCUMENT_BYTES {
        return Ok(Message::Failed(RenderFailure {
            tab_id: req.tab_id,
            nav_id: req.nav_id,
            code: FailureCode::TooLarge,
            detail: "the finished page is too large".to_string(),
        }));
    }
    let title = clip(&page.title, MAX_TITLE_BYTES).to_string();
    let dom_nodes = u32::try_from(page.document.node_count())
        .unwrap_or(u32::MAX)
        .min(MAX_REPORTED_NODES);
    let blocked_resources = u32::try_from(page.blocked_csp_count).unwrap_or(u32::MAX);
    // The painted page goes first, with the same sequence number. A frame
    // that can't be encoded is left out; the window then has only the HTML.
    // The page is given up first, so its images aren't held twice.
    let frame = page_frame_owned(page, viewport);
    let encoded = anvil_browser::frame::encode(&frame);
    drop(frame);
    if let Ok(display_list) = encoded {
        let painted = Message::Frame(RenderedFrame {
            tab_id: req.tab_id,
            nav_id: req.nav_id,
            title: title.clone(),
            dom_nodes,
            blocked_resources,
            display_list,
        });
        wire::write_message(&mut io::stdout().lock(), seq, &painted).map_err(|e| e.to_string())?;
    }
    Ok(Message::Rendered(RenderedDocument {
        tab_id: req.tab_id,
        nav_id: req.nav_id,
        title,
        dom_nodes,
        blocked_resources,
        html,
    }))
}

/// The longest start of `text` that fits in `max` bytes without cutting a character.
fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Load one target and print each `anvil-paint` display item as text.
fn render(args: &[String]) -> Result<(), String> {
    let (viewport, positional) = parse_viewport_args(args, 960.0, 720.0)?;
    let [target] = positional.as_slice() else {
        return Err("render needs a file or an http/https URL".to_string());
    };
    let page = load_target(target, viewport)?;
    println!("title: {}", page.title);
    println!("url: {}", page.url);
    for item in &page.display_list.items {
        match item {
            DisplayItem::PushClip {
                x,
                y,
                width,
                height,
            } => {
                println!("push-clip {x} {y} {width} {height}");
            }
            DisplayItem::PopClip => {
                println!("pop-clip");
            }
            DisplayItem::PushFixed => {
                println!("push-fixed");
            }
            DisplayItem::PopFixed => {
                println!("pop-fixed");
            }
            DisplayItem::BoxShadow {
                x,
                y,
                width,
                height,
                blur_radius,
                color,
                ..
            } => {
                println!(
                    "box-shadow {x} {y} {width} {height} blur={blur_radius} {}",
                    color.hex()
                );
            }
            DisplayItem::LinearGradientRect {
                x,
                y,
                width,
                height,
                angle_deg,
                start_color,
                end_color,
                ..
            } => {
                println!(
                    "linear-gradient {x} {y} {width} {height} angle={angle_deg} {} -> {}",
                    start_color.hex(),
                    end_color.hex()
                );
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                println!("rect {x} {y} {width} {height} {}", color.hex());
            }
            DisplayItem::RoundedRect {
                x,
                y,
                width,
                height,
                radius,
                fill_color,
                ..
            } => {
                println!(
                    "rounded-rect {x} {y} {width} {height} r={radius} {}",
                    fill_color.hex()
                );
            }
            DisplayItem::Image {
                x,
                y,
                width,
                height,
                image,
                ..
            } => {
                println!(
                    "image {x} {y} {width} {height} ({}x{} px)",
                    image.width, image.height
                );
            }
            DisplayItem::Text {
                x,
                y,
                text,
                color,
                font_size,
            } => {
                println!("text {x} {y} {font_size} {} {text}", color.hex());
            }
        }
    }
    Ok(())
}

/// Load one target, rasterize it with `anvil-raster`, and write a PPM file.
fn raster(args: &[String]) -> Result<(), String> {
    let (viewport, positional) = parse_viewport_args(args, 960.0, 720.0)?;
    let [target, output] = positional.as_slice() else {
        return Err("raster needs a source and an output .ppm path".to_string());
    };
    let page = load_target(target, viewport)?;
    let fb = rasterize(
        &page.display_list,
        (viewport.width.round() as u32).max(1),
        (viewport.height.round() as u32).max(1),
        0.0,
    );
    fs::write(output, fb.to_ppm()).map_err(|err| format!("could not write {output}: {err}"))?;
    println!("wrote {}x{} frame to {output}", fb.width(), fb.height());
    Ok(())
}

/// Load one target and write the whole page as a PDF file.
fn save_pdf(args: &[String]) -> Result<(), String> {
    let (viewport, positional) = parse_viewport_args(args, 816.0, 1056.0)?;
    let [target, output] = positional.as_slice() else {
        return Err("pdf needs a source and an output .pdf path".to_string());
    };
    let page = load_target(target, viewport)?;
    let width = (viewport.width.round() as u32).max(1);
    let bytes =
        anvil_browser::pdf::display_list_to_pdf(page.shown_list(), width, page.shown_height());
    fs::write(output, &bytes).map_err(|err| format!("could not write {output}: {err}"))?;
    println!("wrote {} bytes to {output}", bytes.len());
    Ok(())
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetSystemMetrics(n_index: i32) -> i32;
}

/// Detect the default window viewport size based on primary monitor dimensions (1080p, 1440p, 4K).
#[cfg(any(windows, target_os = "linux"))]
fn default_window_dimensions() -> (f32, f32) {
    #[cfg(windows)]
    unsafe {
        let screen_w = GetSystemMetrics(0);
        let screen_h = GetSystemMetrics(1);
        if screen_w >= 1680 && screen_h >= 940 {
            return (1600.0, 920.0);
        }
        if screen_w >= 1280 && screen_h >= 760 {
            return (1280.0, 800.0);
        }
    }
    #[cfg(target_os = "linux")]
    if let Some((screen_w, screen_h, scale_pct)) =
        anvil_browser::wayland::query_wayland_monitor_info()
    {
        let scale = scale_pct.max(100);
        let logical_w = (screen_w as u64 * 100 / scale as u64) as u32;
        let logical_h = (screen_h as u64 * 100 / scale as u64) as u32;
        if logical_w >= 1680 && logical_h >= 940 {
            return (1600.0, 920.0);
        }
        if logical_w >= 1280 && logical_h >= 760 {
            return (1280.0, 800.0);
        }
    }
    (1280.0, 800.0)
}

/// Open the graphical desktop shell (`shell::open`).
///
/// - No positional argument opens the default home page (`anvil://home`).
/// - One positional argument navigates the initial tab to that URL or local file.
/// - When the user closes the window cleanly, the session completes with
///   [`ErrorId::UserClosedBrowser`] (`ERROR ID-1`). Setting `ANVIL_LOG_EXIT=1`
///   prints `[ERROR ID-1] User Closed the Browser` to `stderr` on exit.
#[cfg(any(windows, target_os = "linux"))]
fn open_window(args: &[String]) -> Result<(), String> {
    let (def_w, def_h) = default_window_dimensions();
    let (viewport, positional) = parse_viewport_args(args, def_w, def_h)?;
    let result = match positional.as_slice() {
        [] => anvil_browser::shell::open(None, viewport),
        [target] => anvil_browser::shell::open(Some(target), viewport),
        _ => return Err("open takes at most one file or URL".to_string()),
    };
    if result.is_ok() && env::var_os("ANVIL_LOG_EXIT").is_some() {
        let closed = ErrorId::UserClosedBrowser;
        eprintln!("[{}] {}", closed.code(), closed.title());
    }
    result
}

/// The window shell is compiled only for Windows and Linux.
#[cfg(not(any(windows, target_os = "linux")))]
fn open_window(_args: &[String]) -> Result<(), String> {
    Err("the window shell is only built for Windows and Linux".to_string())
}

/// Pull `--width` and `--height` out of `args`. Everything else stays positional.
fn parse_viewport_args(
    args: &[String],
    default_width: f32,
    default_height: f32,
) -> Result<(Viewport, Vec<&str>), String> {
    let mut width = default_width;
    let mut height = default_height;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--width" => {
                let value = args.get(index + 1).ok_or("width needs a number")?;
                width = value.parse::<f32>().map_err(|_| "width needs a number")?;
                if width <= 0.0 {
                    return Err("width needs a number".to_string());
                }
                index += 2;
            }
            "--height" => {
                let value = args.get(index + 1).ok_or("height needs a number")?;
                height = value.parse::<f32>().map_err(|_| "height needs a number")?;
                if height <= 0.0 {
                    return Err("height needs a number".to_string());
                }
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(format!("unexpected argument {other}"));
            }
            other => {
                positional.push(other);
                index += 1;
            }
        }
    }
    Ok((Viewport { width, height }, positional))
}
