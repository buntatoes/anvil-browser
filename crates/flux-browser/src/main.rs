use std::env;
use std::fs;
use std::process::ExitCode;

use flux_browser::{VERSION, Viewport, load_url, render_html};
use flux_paint::DisplayItem;

fn main() -> ExitCode {
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
        Some("render") => render(&args[1..]),
        Some("version") => {
            println!("flux-browser {VERSION}");
            Ok(())
        }
        Some("help") | None => {
            println!("flux-browser {VERSION}");
            println!("flux render [--width N] <file or http URL>");
            Ok(())
        }
        Some(command) => Err(format!("unknown command {command}")),
    }
}

fn render(args: &[String]) -> Result<(), String> {
    let mut width = 960.0;
    let mut target: Option<&str> = None;
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
            other if target.is_none() => {
                target = Some(other);
                index += 1;
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    let target = target.ok_or("render needs a file or an http URL")?;
    let viewport = Viewport {
        width,
        height: 720.0,
    };
    let page = if target.starts_with("http://") || target.starts_with("https://") {
        load_url(target, viewport).map_err(|err| err.to_string())?
    } else {
        let html =
            fs::read_to_string(target).map_err(|err| format!("could not read {target}: {err}"))?;
        render_html(&html, target, viewport)
    };
    println!("title: {}", page.title);
    println!("url: {}", page.url);
    for item in &page.display_list.items {
        match item {
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color,
            } => {
                println!("rect {x} {y} {width} {height} {}", color.hex());
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
