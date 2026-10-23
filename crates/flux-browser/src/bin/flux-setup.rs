//! Standalone Flux Browser Setup & Uninstaller utility (`flux-setup`).

use std::env;
use std::process::ExitCode;

use flux_browser::VERSION;
use flux_browser::installer::{install_default, uninstall_default};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--uninstall" | "uninstall" | "/uninstall") => match uninstall_default() {
            Ok(dir) => {
                println!(
                    "Flux Browser {VERSION} has been uninstalled from {}",
                    dir.display()
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Uninstall failed: {err}");
                ExitCode::FAILURE
            }
        },
        Some("--help" | "-h" | "help") => {
            println!("Flux Browser Setup {VERSION} (Author: Buntos)");
            println!("  flux-setup              Install Flux Browser and desktop/menu shortcuts");
            println!("  flux-setup --uninstall  Remove Flux Browser and shortcuts");
            ExitCode::SUCCESS
        }
        _ => match install_default() {
            Ok(report) => {
                println!("Flux Browser {VERSION} installed successfully!");
                println!("  Location : {}", report.installed_binary.display());
                for sc in &report.shortcut_paths {
                    println!("  Shortcut : {}", sc.display());
                }
                if report.registered_os_entry {
                    println!("  OS Entry : Registered per-user application entry");
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Installation failed: {err}");
                ExitCode::FAILURE
            }
        },
    }
}
