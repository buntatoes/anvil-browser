// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! `anvil --renderer` speaks the binary wire format from `anvil-sandbox`.
//!
//! These checks start the real binary through the renderer launcher, so the
//! engine, the wire format and the launcher are tested together. The window
//! does not use this mode yet.

use std::time::Duration;

use anvil_sandbox::launcher::{
    RenderOutcome, RendererCommand, RendererFailure, RendererLimits, RendererProcess,
};
use anvil_sandbox::wire::{FetchReply, FetchRequest, RenderRequest};

fn renderer() -> RendererProcess {
    let command = RendererCommand::new(env!("CARGO_BIN_EXE_anvil")).arg("--renderer");
    let limits = RendererLimits {
        startup: Duration::from_secs(20),
        reply: Duration::from_secs(20),
        ..RendererLimits::default()
    };
    RendererProcess::launch(&command, limits).expect("anvil --renderer starts")
}

fn request(nav_id: u64, html: &str) -> RenderRequest {
    RenderRequest {
        tab_id: 1,
        nav_id,
        url: "http://127.0.0.1:9/page.html".to_string(),
        html: html.to_string(),
        csp_header: None,
        scripts_enabled: true,
        viewport_width: 640,
        viewport_height: 480,
        https_only: false,
        http_allowed_hosts: Vec::new(),
    }
}

fn no_fetch(_: &FetchRequest) -> FetchReply {
    panic!("anvil --renderer does not ask the browser for fetches yet");
}

#[test]
fn a_page_with_a_script_renders_in_its_own_process() {
    let mut process = renderer();
    assert_ne!(process.pid(), std::process::id());
    let html = "<html><head><title>first</title></head><body><p id=\"out\">before</p>\
                <script>document.getElementById('out').textContent='after-script'</script>\
                </body></html>";
    match process.render(&request(1, html), &mut no_fetch).unwrap() {
        RenderOutcome::Done(doc) => {
            assert_eq!(doc.title, "first");
            assert!(doc.html.contains("after-script"), "{}", doc.html);
            assert!(doc.dom_nodes > 3);
            assert_eq!(doc.nav_id, 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn one_renderer_serves_several_pages_and_pings() {
    let mut process = renderer();
    for nav in 1..=4 {
        process.ping().unwrap();
        let html = format!("<title>page {nav}</title><p>{nav}</p>");
        let RenderOutcome::Done(doc) = process.render(&request(nav, &html), &mut no_fetch).unwrap()
        else {
            panic!("refused");
        };
        assert_eq!(doc.title, format!("page {nav}"));
    }
    let pid = process.pid();
    process.shutdown();
    #[cfg(target_os = "linux")]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    let _ = pid;
}

#[test]
fn a_script_that_never_ends_is_stopped_and_the_browser_carries_on() {
    // The script budget should end this long before the deadline. If it
    // doesn't, the launcher kills the renderer. Either way the caller gets
    // an answer in bounded time and can start a new renderer.
    let mut process = renderer();
    let html = "<p id=\"x\">x</p><script>while (true) { }</script>";
    match process.render(&request(1, html), &mut no_fetch) {
        Ok(RenderOutcome::Done(_) | RenderOutcome::Refused(_)) => process.ping().unwrap(),
        Err(RendererFailure::TimedOut) => {}
        Err(other) => panic!("{other:?}"),
    }
    let mut fresh = renderer();
    fresh.ping().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn the_renderer_runs_under_the_worker_limits() {
    let mut process = renderer();
    process.ping().unwrap();
    let pid = process.pid();
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    assert!(
        status
            .lines()
            .any(|l| l.split_whitespace().eq(["NoNewPrivs:", "1"])),
        "no_new_privs is not set"
    );
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let address = limits
        .lines()
        .find(|l| l.starts_with("Max address space"))
        .unwrap();
    let soft: u64 = address.split_whitespace().nth(3).unwrap().parse().unwrap();
    assert!(soft <= 256 * 1024 * 1024, "{address}");
    let core = limits
        .lines()
        .find(|l| l.starts_with("Max core file size"))
        .unwrap();
    assert_eq!(core.split_whitespace().nth(4), Some("0"), "{core}");
    // The environment is empty and the working folder is the root.
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    assert!(environ.is_empty(), "the renderer inherited an environment");
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap();
    assert_eq!(cwd, std::path::Path::new("/"));
}
