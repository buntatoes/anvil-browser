// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Command line for the `anvil` binary.
//!
//! `main` asks `anvil-sandbox` to apply process limits, then `run` reads argv.
//! A window goes through `shell::open` on Windows and Linux. `render` prints
//! a `anvil-paint` display list. `raster` writes a PPM file with `anvil-raster`.
//! `--renderer` answers `anvil-sandbox` wire-format frames in this process
//! until the browser says to stop or closes stdin. The window keeps one such
//! process per tab for http(s) pages and local files, and paints a document
//! only when the renderer says it finished. `anvil://` pages stay in the
//! window. `--tab-worker` is the older JSON frame form of the same work. The
//! window no longer starts it.

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::process::ExitCode;

use anvil_browser::installer::{install_default, uninstall_default};
use anvil_browser::{
    TabWorkerRequest, TabWorkerResponse, VERSION, Viewport, enforce_mandatory_sandbox_or_abort,
    load_target, lock_current_worker_process_or_abort, render_html_with_resources_csp_and_storage,
};
use anvil_paint::DisplayItem;
use anvil_raster::rasterize;
use anvil_sandbox::encode_ipc_frame;
use anvil_sandbox::wire::{
    self, Direction, FailureCode, MAX_DOCUMENT_BYTES, MAX_REPORTED_NODES, MAX_SHORT_TEXT_BYTES,
    MAX_TITLE_BYTES, Message, RenderFailure, RenderRequest, RenderedDocument, Role, WIRE_VERSION,
};
use anvil_sandbox::{read_ipc_frame, write_ipc_frame};

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    /// Attach to a console. `main` passes the parent-process id.
    fn AttachConsole(dw_process_id: u32) -> i32;
}

/// Apply sandbox limits, attach the parent console on Windows, then dispatch argv.
fn main() -> ExitCode {
    // Quit if the sandbox limits cannot be set.
    let _mandatory_sandbox = enforce_mandatory_sandbox_or_abort();

    #[cfg(windows)]
    unsafe {
        let _ = AttachConsole(0xFFFF_FFFF);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Choose one argv path. The first token is the command unless it is a bare target.
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // No arguments: empty window.
        None => open_window(&[]),
        // JSON frames in on stdin, frames out on stdout. The window no longer uses it.
        Some("--tab-worker") => run_tab_worker(),
        // Wire-format frames in on stdin and out on stdout. The window starts one per tab.
        Some("--renderer") => run_renderer(),
        // Print display-list lines. No window and no PPM file.
        Some("render") => render(&args[1..]),
        // Headless PPM. No window.
        Some("raster") => raster(&args[1..]),
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
            println!("anvil install");
            println!("anvil uninstall");
            Ok(())
        }
        // A bare http(s) URL, anvil:// URL, or .html/.htm path opens a window on that target.
        Some(target)
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("anvil://")
                || target.ends_with(".html")
                || target.ends_with(".htm") =>
        {
            open_window(&args)
        }
        Some(command) => Err(format!("unknown command {command}")),
    }
}

/// Serve worker requests until the window closes stdin.
///
/// `lock_current_worker_process_or_abort` applies the worker limits from
/// `anvil-sandbox`. Each request is one document. Scripts run only when the
/// request asks for them. The response HTML is the document after that step.
/// This process owns its script runtime and does not receive the window's
/// cookie jar. A response that does not fit in one `ANVILIPC` frame is
/// `ok: false` with an empty document. Closing stdin ends the loop. This
/// function does not open a window.
fn run_tab_worker() -> Result<(), String> {
    lock_current_worker_process_or_abort();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut served = 0u64;
    loop {
        let req_json = match read_ipc_frame(&mut stdin) {
            Ok(json) => json,
            // The window closed the pipe. That is the normal way to stop.
            Err(_) if served > 0 => return Ok(()),
            Err(err) => return Err(err.to_string()),
        };
        let req = TabWorkerRequest::from_json(&req_json).map_err(|e| e.to_string())?;
        let resp = serve_tab_request(&req);
        write_ipc_frame(&mut stdout, &resp.to_json()).map_err(|e| e.to_string())?;
        served += 1;
    }
}

/// Serve wire-format requests until the browser says to stop or closes stdin.
///
/// The process takes the worker limits from `anvil-sandbox`. It answers the
/// browser's hello, pings, and one `Render` at a time. Anything else from the
/// browser ends the process, because only a broken browser would send it.
/// A panic while rendering ends the process too, and the browser sees that
/// as a crash. The window keeps one of these processes per tab, started with
/// an empty environment. This mode still loads its own scripts and images.
/// Moving those fetches to the browser is the next step.
fn run_renderer() -> Result<(), String> {
    lock_current_worker_process_or_abort();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let (seq, hello) =
        wire::read_message(&mut stdin, Direction::ToRenderer).map_err(|e| e.to_string())?;
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
    wire::write_message(&mut stdout, seq, &reply).map_err(|e| e.to_string())?;
    loop {
        let (seq, message) = match wire::read_message(&mut stdin, Direction::ToRenderer) {
            Ok(frame) => frame,
            // The browser closed the pipe. That is a normal way to stop.
            Err(wire::WireError::Io(io::ErrorKind::UnexpectedEof)) => return Ok(()),
            Err(err) => return Err(format!("renderer: {err}")),
        };
        let answer = match message {
            Message::Ping { nonce } => Message::Pong { nonce },
            Message::Render(req) => render_for_browser(&req),
            Message::Shutdown => return Ok(()),
            _ => return Err("renderer: unexpected message from the browser".to_string()),
        };
        wire::write_message(&mut stdout, seq, &answer).map_err(|e| e.to_string())?;
    }
}

/// Run one wire-format request through the same steps as the tab worker.
fn render_for_browser(req: &RenderRequest) -> Message {
    let worker_req = TabWorkerRequest {
        tab_id: req.tab_id,
        url: req.url.clone(),
        html: req.html.clone(),
        scripts_enabled: req.scripts_enabled,
        csp_header: req.csp_header.clone(),
        viewport_width: req.viewport_width,
        viewport_height: req.viewport_height,
        https_only: req.https_only,
        http_allowed_hosts: req.http_allowed_hosts.clone(),
    };
    let resp = serve_tab_request(&worker_req);
    let failed = |code, detail: &str| {
        Message::Failed(RenderFailure {
            tab_id: req.tab_id,
            nav_id: req.nav_id,
            code,
            detail: clip(detail, MAX_SHORT_TEXT_BYTES).to_string(),
        })
    };
    if !resp.ok {
        let detail = resp
            .error
            .as_deref()
            .unwrap_or("the page could not be rendered");
        return failed(FailureCode::Internal, detail);
    }
    if resp.html.len() > MAX_DOCUMENT_BYTES {
        return failed(FailureCode::TooLarge, "the finished page is too large");
    }
    Message::Rendered(RenderedDocument {
        tab_id: req.tab_id,
        nav_id: req.nav_id,
        title: clip(&resp.title, MAX_TITLE_BYTES).to_string(),
        dom_nodes: u32::try_from(resp.dom_nodes)
            .unwrap_or(u32::MAX)
            .min(MAX_REPORTED_NODES),
        blocked_resources: u32::try_from(resp.blocked_csp_resources).unwrap_or(u32::MAX),
        html: resp.html,
    })
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

/// Render one worker request and build its response.
fn serve_tab_request(req: &TabWorkerRequest) -> TabWorkerResponse {
    // Subresource requests from this page follow the window's HTTPS-only rules.
    let _https_only = anvil_net::enter_https_only(anvil_net::HttpsOnlyPolicy::new(
        req.https_only,
        &req.http_allowed_hosts,
    ));
    let page = render_html_with_resources_csp_and_storage(
        &req.html,
        "",
        HashMap::new(),
        &req.url,
        Viewport {
            width: req.viewport_width.max(1) as f32,
            height: req.viewport_height.max(1) as f32,
        },
        req.csp_header.as_deref(),
        0,
        &[],
        None,
        req.scripts_enabled,
    );
    let resp = TabWorkerResponse {
        tab_id: req.tab_id,
        ok: true,
        title: page.title.clone(),
        dom_nodes: page.document.node_count(),
        blocked_csp_resources: page.blocked_csp_count,
        error: None,
        html: page.document.to_html(),
    };
    if encode_ipc_frame(&resp.to_json()).is_err() {
        return TabWorkerResponse {
            tab_id: req.tab_id,
            ok: false,
            title: String::new(),
            dom_nodes: 0,
            blocked_csp_resources: 0,
            error: Some("worker response exceeds the IPC cap".to_string()),
            html: String::new(),
        };
    }
    resp
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

/// Open `shell::open`. No positional argument starts blank. One argument is the first target.
#[cfg(any(windows, target_os = "linux"))]
fn open_window(args: &[String]) -> Result<(), String> {
    let (viewport, positional) = parse_viewport_args(args, 960.0, 720.0)?;
    match positional.as_slice() {
        [] => anvil_browser::shell::open(None, viewport),
        [target] => anvil_browser::shell::open(Some(target), viewport),
        _ => Err("open takes at most one file or URL".to_string()),
    }
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
