// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! `anvil --renderer` speaks the binary wire format from `anvil-sandbox`.
//!
//! These checks start the real binary through the renderer launcher, so the
//! engine, the wire format and the launcher are tested together. The
//! renderer has no network. It asks the browser for every linked script and
//! every script `fetch()`, and these checks answer for the browser.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anvil_browser::answer_renderer_fetch;
use anvil_net::CookieJar;
use anvil_sandbox::launcher::{
    RenderOutcome, RendererCommand, RendererFailure, RendererLimits, RendererProcess,
};
use anvil_sandbox::wire::{
    self, Direction, FetchKind, FetchReply, FetchRequest, Message, RenderRequest, Role,
    WIRE_VERSION,
};

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
    request_at(nav_id, "http://127.0.0.1:9/page.html", html)
}

fn request_at(nav_id: u64, url: &str, html: &str) -> RenderRequest {
    RenderRequest {
        tab_id: 1,
        nav_id,
        url: url.to_string(),
        html: html.to_string(),
        csp_header: None,
        scripts_enabled: true,
        viewport_width: 640,
        viewport_height: 480,
        https_only: false,
        http_allowed_hosts: Vec::new(),
    }
}

/// For pages that load nothing.
fn no_fetch(want: &FetchRequest) -> FetchReply {
    panic!("this page should not fetch anything: {want:?}");
}

fn reply(want: &FetchRequest, status: u16, content_type: &str, body: &str) -> FetchReply {
    FetchReply {
        nav_id: want.nav_id,
        status,
        content_type: content_type.to_string(),
        body: body.as_bytes().to_vec(),
    }
}

/// The sockets among the open files of process `pid`.
#[cfg(target_os = "linux")]
fn sockets_of(pid: u32) -> Vec<String> {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| target.starts_with("socket:"))
        .collect()
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

#[test]
fn linked_scripts_and_script_fetch_are_asked_of_the_browser() {
    let mut process = renderer();
    let pid = process.pid();
    let html = "<p id=\"s\">none</p><p id=\"f\">none</p><img src=\"/photo.png\">\
                <script src=\"/app.js\"></script>\
                <script>let r = fetch('/api'); \
                document.getElementById('f').textContent = r.body + ':' + r.status;</script>";
    let mut asked = Vec::new();
    let mut fetch = |want: &FetchRequest| {
        asked.push((want.kind, want.url.clone(), want.nav_id));
        // While the renderer waits for this answer it holds no socket.
        #[cfg(target_os = "linux")]
        assert!(sockets_of(pid).is_empty(), "{:?}", sockets_of(pid));
        match want.url.as_str() {
            "http://127.0.0.1:9/app.js" => reply(
                want,
                200,
                "text/javascript",
                "document.getElementById('s').textContent = 'linked';",
            ),
            "http://127.0.0.1:9/api" => reply(want, 200, "", "pong"),
            _ => reply(want, 0, "", ""),
        }
    };
    let RenderOutcome::Done(doc) = process.render(&request(5, html), &mut fetch).unwrap() else {
        panic!("refused");
    };
    assert!(doc.html.contains(">linked<"), "{}", doc.html);
    assert!(doc.html.contains(">pong:200<"), "{}", doc.html);
    // The image is asked for after scripts have run.
    assert_eq!(
        asked,
        vec![
            (
                FetchKind::Script,
                "http://127.0.0.1:9/app.js".to_string(),
                5
            ),
            (
                FetchKind::ScriptFetch,
                "http://127.0.0.1:9/api".to_string(),
                5
            ),
            (
                FetchKind::Image,
                "http://127.0.0.1:9/photo.png".to_string(),
                5
            ),
        ]
    );
    let _ = pid;
}

#[test]
fn the_renderer_opens_no_connection_of_its_own() {
    // Images, video and map tiles on a real port: the renderer must not
    // touch any of them, and a refused script stays refused.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let html = format!(
        "<img src=\"{base}/a.png\"><video src=\"{base}/v.mp4\"></video>\
         <img src=\"anvil-map://2/0/0\">\
         <script src=\"{base}/refused.js\"></script>\
         <script>document.title = 'ran';</script>"
    );
    let mut process = renderer();
    let mut fetch = |want: &FetchRequest| reply(want, 0, "", "");
    let RenderOutcome::Done(doc) = process
        .render(
            &request_at(1, &format!("{base}/index.html"), &html),
            &mut fetch,
        )
        .unwrap()
    else {
        panic!("refused");
    };
    assert_eq!(doc.title, "ran");
    std::thread::sleep(Duration::from_millis(100));
    assert!(listener.accept().is_err(), "the renderer connected");
}

/// A local server that answers by path and keeps each request head.
fn route_server(routes: Vec<(&'static str, String)>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut sock) = stream else {
                continue;
            };
            let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let path = head.split(' ').nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(head);
            let answer = routes
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                });
            let _ = sock.write_all(answer.as_bytes());
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, seen)
}

fn ok_response(body: &str, extra: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

#[test]
fn the_browser_keeps_the_cookies_for_a_renderer_page() {
    let (port, seen) = route_server(vec![
        (
            "/app.js",
            ok_response("document.getElementById('s').textContent = 'linked';", ""),
        ),
        (
            "/api",
            ok_response("from-api", "Set-Cookie: seen=yes; Path=/\r\n"),
        ),
    ]);
    let page = format!("http://127.0.0.1:{port}/index.html");
    let jar = Arc::new(Mutex::new(CookieJar::new()));
    assert!(
        jar.lock()
            .unwrap()
            .store_set_cookie_under(&page, &page, "sid=abc; Path=/")
    );
    let html = format!(
        "<p id=\"s\"></p><p id=\"f\"></p><p id=\"x\"></p>\
         <script src=\"/app.js\"></script>\
         <script>let r = fetch('/api'); \
         let other = fetch('http://localhost:{port}/api'); \
         document.getElementById('f').textContent = r.body; \
         document.getElementById('x').textContent = 'cross:' + other.status; \
         document.getElementById('s').setAttribute('data-cookie', document.cookie);</script>"
    );
    let mut process = renderer();
    let mut fetch = |want: &FetchRequest| answer_renderer_fetch(want, &page, &jar);
    let RenderOutcome::Done(doc) = process
        .render(&request_at(2, &page, &html), &mut fetch)
        .unwrap()
    else {
        panic!("refused");
    };
    assert!(doc.html.contains(">linked<"), "{}", doc.html);
    assert!(doc.html.contains(">from-api<"), "{}", doc.html);
    assert!(doc.html.contains(">cross:403<"), "{}", doc.html);
    // The renderer never saw a cookie.
    assert!(!doc.html.contains("sid=abc"), "{}", doc.html);
    assert!(!doc.html.contains("seen=yes"), "{}", doc.html);
    // The browser's jar sent one and kept the new one.
    assert!(
        jar.lock()
            .unwrap()
            .cookies()
            .iter()
            .any(|c| c.name == "seen" && c.value == "yes")
    );
    let seen = seen.lock().unwrap();
    let api: Vec<_> = seen.iter().filter(|h| h.starts_with("GET /api ")).collect();
    assert_eq!(api.len(), 1, "{api:?}");
    assert!(api[0].contains("sid=abc"), "{}", api[0]);
    let app: Vec<_> = seen
        .iter()
        .filter(|h| h.starts_with("GET /app.js "))
        .collect();
    assert_eq!(app.len(), 1, "{app:?}");
    assert!(
        !app[0].to_ascii_lowercase().contains("cookie:"),
        "{}",
        app[0]
    );
}

#[test]
fn a_wrong_answer_to_a_fetch_ends_the_renderer() {
    // Talk to the renderer by hand, as a broken browser would.
    let mut child = Command::new(env!("CARGO_BIN_EXE_anvil"))
        .arg("--renderer")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut to = child.stdin.take().unwrap();
    let mut from = child.stdout.take().unwrap();
    let hello = Message::Hello {
        role: Role::Browser,
        version: WIRE_VERSION,
    };
    wire::write_message(&mut to, 1, &hello).unwrap();
    let (_, answer) = wire::read_message(&mut from, Direction::ToBrowser).unwrap();
    assert!(matches!(answer, Message::Hello { .. }));
    let html = "<script src=\"/app.js\"></script><p>x</p>";
    wire::write_message(&mut to, 2, &Message::Render(request(9, html))).unwrap();
    let (seq, asked) = wire::read_message(&mut from, Direction::ToBrowser).unwrap();
    assert_eq!(seq, 2);
    assert!(
        matches!(asked, Message::Fetch(FetchRequest { nav_id: 9, .. })),
        "{asked:?}"
    );
    // A reply for another navigation.
    let wrong = FetchReply {
        nav_id: 10,
        status: 200,
        content_type: String::new(),
        body: Vec::new(),
    };
    wire::write_message(&mut to, 2, &Message::FetchReply(wrong)).unwrap();
    // The renderer sends nothing more and exits with a failure.
    let mut rest = Vec::new();
    from.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "{} more bytes", rest.len());
    let status = child.wait().unwrap();
    assert!(!status.success());
}

/// A fresh renderer reports what the OS has put it under. Started plainly,
/// it shares every namespace with this process and carries the worker
/// limits it set on itself.
#[cfg(target_os = "linux")]
#[test]
fn a_renderer_reports_its_own_os_status() {
    let mut process = renderer();
    let status = process.status().expect("status answer");
    let ours = anvil_sandbox::confine::own_namespaces();
    assert_eq!(
        status.namespaces, ours,
        "a plain renderer shares namespaces"
    );
    assert_eq!(status.no_new_privs, Some(true));
    assert_eq!(status.pid, process.pid());
    assert!(status.visible_processes > 1, "{status:?}");
    // The worker limits: 256 MiB of address space and no core files.
    assert_eq!(status.limits[0], Some(256 << 20));
    assert_eq!(status.limits[1], Some(0));
    // It still renders after answering.
    let outcome = process
        .render(&request(1, "<p>after status</p>"), &mut no_fetch)
        .expect("render");
    assert!(matches!(outcome, RenderOutcome::Done(_)));
    process.shutdown();
}
