// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Each tab renders local files and http(s) pages in its own renderer process.
//!
//! A renderer stays with its tab across navigations and ends when the tab
//! closes. A renderer that hangs or exits shows an error page in that tab,
//! the window session keeps working, and the next load starts a fresh one.
//! Renderers start with an empty environment. `anvil://` pages never start
//! a renderer.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anvil_browser::{Session, Viewport};

fn viewport() -> Viewport {
    Viewport {
        width: 640.0,
        height: 480.0,
    }
}

fn anvil_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_anvil"))
}

/// A scratch folder for this test run.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anvil-tabs-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write_page(dir: &Path, file: &str, marker: &str) -> String {
    let path = dir.join(file);
    let html = format!(
        "<html><head><title>{marker}</title></head><body>\
         <p id=\"out\">before</p>\
         <script>document.getElementById('out').textContent='{marker}-from-script'</script>\
         </body></html>"
    );
    fs::write(&path, html).expect("write page");
    path.to_string_lossy().into_owned()
}

fn active_text(session: &Session) -> String {
    let page = session.active().current().expect("page");
    page.document.text_content(page.document.body())
}

/// True while a process with `pid` still exists.
#[cfg(target_os = "linux")]
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[test]
fn every_tab_gets_its_own_worker_and_closing_the_tab_ends_it() {
    let dir = scratch_dir("own");
    let first = write_page(&dir, "first.html", "alpha");
    let second = write_page(&dir, "second.html", "beta");
    let third = write_page(&dir, "third.html", "gamma");

    let mut session = Session::new(viewport());
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );

    // Built-in pages stay in the window.
    session.open_target("anvil://settings").unwrap();
    assert_eq!(session.tab_worker_pid(0), None);

    session.open_target(&first).unwrap();
    assert!(active_text(&session).contains("alpha-from-script"));
    let pid_a = session.tab_worker_pid(0).expect("worker for tab 0");

    // The same tab keeps the same worker for its next page.
    session.open_target(&third).unwrap();
    assert!(active_text(&session).contains("gamma-from-script"));
    assert_eq!(session.tab_worker_pid(0), Some(pid_a));

    session.new_tab();
    session.open_target(&second).unwrap();
    assert!(active_text(&session).contains("beta-from-script"));
    let pid_b = session.tab_worker_pid(1).expect("worker for tab 1");
    assert_ne!(pid_a, pid_b);
    assert_ne!(pid_b, std::process::id());

    // Closing tab 0 ends its worker and leaves the other tab's worker alone.
    let closed_id = session.tabs()[0].id();
    assert!(session.close_tab(0));
    assert_eq!(session.tab_index_for_id(closed_id), None);
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(3);
        while process_exists(pid_a) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !process_exists(pid_a),
            "worker of the closed tab is still running"
        );
        assert!(process_exists(pid_b));
    }
    assert_eq!(session.tab_worker_pid(0), Some(pid_b));

    // Ending the session ends the remaining worker.
    drop(session);
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(3);
        while process_exists(pid_b) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!process_exists(pid_b));
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_page_for_a_closed_tab_is_dropped() {
    let mut session = Session::new(viewport());
    session.new_tab();
    let gone = session.active_tab_id();
    assert!(session.close_tab(1));
    let page = anvil_browser::render_html("<p>late</p>", "http://127.0.0.1/late", viewport());
    assert!(!session.open_page_in_tab_id(gone, page));
    let kept = session.active_tab_id();
    let page = anvil_browser::render_html("<p>kept</p>", "http://127.0.0.1/kept", viewport());
    assert!(session.open_page_in_tab_id(kept, page));
}

#[test]
fn a_silent_renderer_shows_an_error_page_and_the_window_keeps_going() {
    let dir = scratch_dir("silent");
    let page = write_page(&dir, "page.html", "silent-secret");
    let mut session = Session::new(viewport());
    let (program, args) = sleeping_process();
    session.set_tab_worker_command(Path::new(program), &args, Duration::from_millis(600));

    let started = Instant::now();
    session.open_target(&page).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    let current = session.active().current().expect("error page");
    assert_eq!(current.title, "Navigation failed");
    let text = active_text(&session);
    assert!(text.contains("did not start properly"), "{text}");
    assert!(!text.contains("silent-secret"));
    assert_eq!(session.tab_worker_pid(0), None);

    // Built-in pages still open, and a real renderer works again afterwards.
    session.open_target("anvil://home").unwrap();
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.open_target(&page).unwrap();
    assert!(active_text(&session).contains("silent-secret-from-script"));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_hung_renderer_shows_an_error_page_and_the_window_keeps_going() {
    let dir = scratch_dir("hung");
    let page = write_page(&dir, "page.html", "hung-secret");
    let mut session = Session::new(viewport());
    let (program, args) = hello_then("exec sleep 30");
    session.set_tab_worker_command(Path::new(program), &args, Duration::from_millis(600));

    let started = Instant::now();
    session.open_target(&page).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    let current = session.active().current().expect("error page");
    assert_eq!(current.title, "Navigation failed");
    let text = active_text(&session);
    assert!(text.contains("stopped responding"), "{text}");
    assert!(!text.contains("hung-secret"));
    assert_eq!(session.tab_worker_pid(0), None);

    // Built-in pages still open, and a real renderer works again afterwards.
    session.open_target("anvil://home").unwrap();
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.open_target(&page).unwrap();
    assert!(active_text(&session).contains("hung-secret-from-script"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_renderer_that_exits_shows_an_error_page() {
    let dir = scratch_dir("crash");
    let page = write_page(&dir, "page.html", "crash-secret");
    let mut session = Session::new(viewport());
    let (program, args) = exiting_process();
    session.set_tab_worker_command(Path::new(program), &args, Duration::from_secs(5));

    session.open_target(&page).unwrap();
    let current = session.active().current().expect("error page");
    assert_eq!(current.title, "Navigation failed");
    let text = active_text(&session);
    assert!(text.contains("the renderer stopped"), "{text}");
    assert!(!text.contains("crash-secret"));
    assert_eq!(session.tab_worker_pid(0), None);

    // The tab is still usable.
    session.open_target("anvil://settings").unwrap();
    assert_eq!(
        session.active().current().map(|p| p.url.as_str()),
        Some("anvil://settings")
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A child that reads nothing and stays alive for 30 seconds.
#[cfg(windows)]
fn sleeping_process() -> (&'static str, Vec<String>) {
    (
        "powershell.exe",
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Start-Sleep -Seconds 30".to_string(),
        ],
    )
}

/// A child that reads nothing and stays alive for 30 seconds.
#[cfg(not(windows))]
fn sleeping_process() -> (&'static str, Vec<String>) {
    ("sleep", vec!["30".to_string()])
}

/// A child that exits right away with a failure code.
#[cfg(windows)]
fn exiting_process() -> (&'static str, Vec<String>) {
    ("cmd.exe", vec!["/C".to_string(), "exit 3".to_string()])
}

/// A child that exits right away with a failure code.
#[cfg(not(windows))]
fn exiting_process() -> (&'static str, Vec<String>) {
    ("sh", vec!["-c".to_string(), "exit 3".to_string()])
}

/// A child that sends a valid renderer hello, then runs `then` in the shell.
/// The hello is `ANVR`, version 1, kind 1, no flags, seq 0, a 3-byte body:
/// role 2 (renderer) and version 1.
#[cfg(unix)]
fn hello_then(then: &str) -> (&'static str, Vec<String>) {
    let hello = r"\101\116\126\122\001\000\001\000\000\000\000\000\003\000\000\000\002\001\000";
    (
        "/bin/sh",
        vec!["-c".to_string(), format!("printf '{hello}'; {then}")],
    )
}

#[cfg(unix)]
#[test]
fn a_renderer_that_crashes_mid_page_shows_an_error_page() {
    let dir = scratch_dir("midcrash");
    let page = write_page(&dir, "page.html", "midcrash-secret");
    let mut session = Session::new(viewport());
    let (program, args) = hello_then("sleep 0.2; exit 3");
    session.set_tab_worker_command(Path::new(program), &args, Duration::from_secs(5));
    session.open_target(&page).unwrap();
    let current = session.active().current().expect("error page");
    assert_eq!(current.title, "Navigation failed");
    let text = active_text(&session);
    assert!(
        text.contains("the renderer stopped (exit code 3)"),
        "{text}"
    );
    assert!(!text.contains("midcrash-secret"));
    assert_eq!(session.tab_worker_pid(0), None);
    let _ = fs::remove_dir_all(&dir);
}

/// The renderer of a tab gets none of the window's environment, so on
/// Linux it has no `DISPLAY` or `XAUTHORITY` and can't open the X11 session.
#[cfg(target_os = "linux")]
#[test]
fn a_tab_renderer_starts_with_an_empty_environment() {
    assert!(std::env::var_os("PATH").is_some(), "test needs PATH set");
    let dir = scratch_dir("env");
    let page = write_page(&dir, "page.html", "env");
    let mut session = Session::new(viewport());
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.open_target(&page).unwrap();
    assert!(active_text(&session).contains("env-from-script"));
    let pid = session.tab_worker_pid(0).expect("renderer for tab 0");
    let environ = fs::read(format!("/proc/{pid}/environ")).expect("environ");
    assert!(environ.is_empty(), "the renderer inherited an environment");
    let cwd = fs::read_link(format!("/proc/{pid}/cwd")).expect("cwd");
    assert_eq!(cwd, Path::new("/"));
    let _ = fs::remove_dir_all(&dir);
}

/// Serve `body` as HTML to every request on a loopback port.
fn serve_page(body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().take(8) {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    format!("http://{addr}/page")
}

#[test]
fn http_pages_render_in_the_tab_worker() {
    let url = serve_page(
        "<html><head><title>remote</title></head><body><p id=\"out\">x</p>\
         <script>document.getElementById('out').textContent='remote-from-script'</script></body></html>",
    );
    let mut session = Session::new(viewport());
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.open_target(&url).unwrap();
    assert!(active_text(&session).contains("remote-from-script"));
    assert!(session.tab_worker_pid(0).is_some());
}

/// A page that crashes its renderer every time stops getting new ones after
/// three crashes in a minute. Other tabs are not affected.
#[cfg(unix)]
#[test]
fn a_crash_loop_stops_starting_renderers_for_that_tab() {
    let dir = scratch_dir("loop");
    let page = write_page(&dir, "page.html", "loop-secret");
    let starts = dir.join("starts");
    let mut session = Session::new(viewport());
    let (program, args) = hello_then(&format!("echo start >> '{}'; exit 3", starts.display()));
    session.set_tab_worker_command(Path::new(program), &args, Duration::from_secs(5));
    for _ in 0..3 {
        session.open_target(&page).unwrap();
        assert!(active_text(&session).contains("the renderer stopped"));
    }
    session.open_target(&page).unwrap();
    let current = session.active().current().expect("error page");
    assert_eq!(current.title, "Navigation failed");
    let text = active_text(&session);
    assert!(text.contains("keeps crashing"), "{text}");
    assert!(!text.contains("loop-secret"));
    let count = |path: &Path| fs::read_to_string(path).unwrap_or_default().lines().count();
    assert_eq!(count(&starts), 3, "a fourth renderer was started");

    // Built-in pages still work in that tab, and a new tab gets its own budget.
    session.open_target("anvil://settings").unwrap();
    session.set_tab_worker_command(
        &anvil_exe(),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.open_target(&page).unwrap();
    assert!(active_text(&session).contains("keeps crashing"));
    session.new_tab();
    session.open_target(&page).unwrap();
    assert!(active_text(&session).contains("loop-secret-from-script"));
    let _ = fs::remove_dir_all(&dir);
}
