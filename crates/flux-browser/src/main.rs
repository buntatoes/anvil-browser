#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::process::ExitCode;

use flux_browser::installer::{install_default, uninstall_default};
use flux_browser::{
    TabWorkerRequest, TabWorkerResponse, VERSION, Viewport, enforce_mandatory_sandbox_or_abort,
    load_target, lock_current_worker_process_or_abort, render_html_with_resources_and_csp,
};
use flux_paint::DisplayItem;
use flux_raster::rasterize;
use flux_sandbox::{read_ipc_frame, write_ipc_frame};

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn AttachConsole(dw_process_id: u32) -> i32;
}

fn main() -> ExitCode {
    // Hardcoded fail-closed security gate: Flux refuses to launch if the OS sandbox is unavailable.
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

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => open_window(&[]),
        Some("--tab-worker") => run_tab_worker(),
        Some("render") => render(&args[1..]),
        Some("raster") => raster(&args[1..]),
        Some("open" | "view") => open_window(&args[1..]),
        Some("install" | "--install") => {
            let report = install_default()?;
            println!(
                "installed Flux Browser {VERSION} to {}",
                report.installed_binary.display()
            );
            for sc in &report.shortcut_paths {
                println!("shortcut: {}", sc.display());
            }
            Ok(())
        }
        Some("uninstall" | "--uninstall") => {
            let dir = uninstall_default()?;
            println!("uninstalled Flux Browser {VERSION} from {}", dir.display());
            Ok(())
        }
        Some("version" | "--version" | "-v") => {
            println!("flux-browser {VERSION}");
            Ok(())
        }
        Some("help" | "--help" | "-h") => {
            println!("flux-browser {VERSION}");
            println!("flux [file or http/https/flux URL]");
            println!("flux open [--width N] [--height N] [file or http/https/flux URL]");
            println!("flux render [--width N] <file or http/https/flux URL>");
            println!(
                "flux raster [--width N] [--height N] <file or http/https/flux URL> <output.ppm>"
            );
            println!("flux install");
            println!("flux uninstall");
            Ok(())
        }
        Some(target)
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("flux://")
                || target.ends_with(".html")
                || target.ends_with(".htm") =>
        {
            open_window(&args)
        }
        Some(command) => Err(format!("unknown command {command}")),
    }
}

fn run_tab_worker() -> Result<(), String> {
    lock_current_worker_process_or_abort();
    let mut stdin = io::stdin().lock();
    let req_json = read_ipc_frame(&mut stdin).map_err(|e| e.to_string())?;
    let req = TabWorkerRequest::from_json(&req_json).map_err(|e| e.to_string())?;
    let page = render_html_with_resources_and_csp(
        &req.html,
        "",
        HashMap::new(),
        &req.url,
        Viewport {
            width: 960.0,
            height: 720.0,
        },
        req.csp_header.as_deref(),
        0,
    );
    let resp = TabWorkerResponse {
        tab_id: req.tab_id,
        ok: true,
        title: page.title,
        dom_nodes: page.document.node_count(),
        blocked_csp_resources: page.blocked_csp_count,
        error: None,
    };
    let mut stdout = io::stdout().lock();
    write_ipc_frame(&mut stdout, &resp.to_json()).map_err(|e| e.to_string())
}

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

#[cfg(any(windows, target_os = "linux"))]
fn open_window(args: &[String]) -> Result<(), String> {
    let (viewport, positional) = parse_viewport_args(args, 960.0, 720.0)?;
    match positional.as_slice() {
        [] => flux_browser::shell::open(None, viewport),
        [target] => flux_browser::shell::open(Some(target), viewport),
        _ => Err("open takes at most one file or URL".to_string()),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_window(_args: &[String]) -> Result<(), String> {
    Err("the window shell is only built for Windows and Linux".to_string())
}

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
