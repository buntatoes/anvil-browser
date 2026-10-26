// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! End-to-end checks for the renderer launcher, with a real child process.
//!
//! This file has its own `main` (see `harness = false` in Cargo.toml). Started
//! normally, it runs the checks. Started with `ANVIL_STAND_IN` set, it acts
//! as a stand-in renderer instead: it speaks the wire format on stdin and
//! stdout, and a word in the document makes it misbehave on purpose, the way
//! a page that trips a parser bug might.

use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use anvil_sandbox::launcher::{
    RelaunchBudget, RenderOutcome, RendererCommand, RendererFailure, RendererLimits,
    RendererProcess,
};
use anvil_sandbox::wire::{
    self, Direction, FailureCode, FetchKind, FetchReply, FetchRequest, Message, RenderFailure,
    RenderRequest, RenderedDocument, Role, WIRE_VERSION, WireError,
};

fn main() {
    if let Ok(mode) = std::env::var("ANVIL_STAND_IN") {
        stand_in(&mode);
        return;
    }
    let options = TestOptions::from_args();
    let checks: &[(&str, fn())] = &[
        (
            "renders_a_document_and_answers_pings",
            renders_a_document_and_answers_pings,
        ),
        (
            "starts_with_a_clean_environment",
            starts_with_a_clean_environment,
        ),
        (
            "fetches_go_through_the_browser",
            fetches_go_through_the_browser,
        ),
        (
            "too_many_fetches_kill_the_renderer",
            too_many_fetches_kill_the_renderer,
        ),
        (
            "slow_fetches_hit_the_page_time_limit",
            slow_fetches_hit_the_page_time_limit,
        ),
        (
            "a_refused_page_keeps_the_renderer",
            a_refused_page_keeps_the_renderer,
        ),
        (
            "a_hung_renderer_is_killed_at_the_deadline",
            a_hung_renderer_is_killed_at_the_deadline,
        ),
        (
            "a_crash_reports_its_exit_code",
            a_crash_reports_its_exit_code,
        ),
        ("an_abort_is_contained", an_abort_is_contained),
        ("garbage_kills_the_renderer", garbage_kills_the_renderer),
        (
            "an_oversize_frame_is_refused_early",
            an_oversize_frame_is_refused_early,
        ),
        (
            "a_browser_message_from_the_renderer_is_refused",
            a_browser_message_from_the_renderer_is_refused,
        ),
        (
            "a_page_for_another_tab_is_refused",
            a_page_for_another_tab_is_refused,
        ),
        (
            "a_page_for_an_old_navigation_is_refused",
            a_page_for_an_old_navigation_is_refused,
        ),
        (
            "a_silent_renderer_fails_the_handshake",
            a_silent_renderer_fails_the_handshake,
        ),
        (
            "another_protocol_version_is_refused",
            another_protocol_version_is_refused,
        ),
        (
            "a_renderer_that_exits_at_once_is_reported",
            a_renderer_that_exits_at_once_is_reported,
        ),
        (
            "a_program_that_is_not_there_fails_to_start",
            a_program_that_is_not_there_fails_to_start,
        ),
        (
            "a_stderr_flood_neither_blocks_nor_grows",
            a_stderr_flood_neither_blocks_nor_grows,
        ),
        (
            "memory_over_the_cap_kills_the_renderer",
            memory_over_the_cap_kills_the_renderer,
        ),
        (
            "dropping_the_handle_ends_the_process",
            dropping_the_handle_ends_the_process,
        ),
        (
            "shutdown_lets_the_renderer_exit",
            shutdown_lets_the_renderer_exit,
        ),
        (
            "a_crash_loop_stops_relaunching",
            a_crash_loop_stops_relaunching,
        ),
        (
            "an_idle_renderer_passes_its_heartbeat",
            an_idle_renderer_passes_its_heartbeat,
        ),
        (
            "a_renderer_busy_while_idle_is_killed",
            a_renderer_busy_while_idle_is_killed,
        ),
        (
            "a_renderer_that_stops_reading_fails_its_heartbeat",
            a_renderer_that_stops_reading_fails_its_heartbeat,
        ),
    ];
    if options.list {
        for (name, _) in checks {
            println!("{name}: test");
        }
        return;
    }
    let mut passed = 0;
    let mut failed = Vec::new();
    let mut filtered = 0;
    println!();
    for (name, check) in checks {
        if !options.wants(name) {
            filtered += 1;
            continue;
        }
        match catch_unwind(AssertUnwindSafe(check)) {
            Ok(()) => {
                println!("test {name} ... ok");
                passed += 1;
            }
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    let result = if failed.is_empty() { "ok" } else { "FAILED" };
    println!(
        "\ntest result: {result}. {passed} passed; {} failed; 0 ignored; 0 measured; {filtered} filtered out\n",
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}

/// The few test-runner options that matter here. Others are accepted and
/// ignored, so `cargo test -- --test-threads 2` still works.
struct TestOptions {
    filters: Vec<String>,
    exact: bool,
    list: bool,
    ignored_only: bool,
}

impl TestOptions {
    fn from_args() -> Self {
        let mut options = Self {
            filters: Vec::new(),
            exact: false,
            list: false,
            ignored_only: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--exact" => options.exact = true,
                "--list" => options.list = true,
                "--ignored" => options.ignored_only = true,
                "--test-threads" | "--skip" | "--format" | "--color" | "--logfile" | "-Z" => {
                    let _ = args.next();
                }
                flag if flag.starts_with('-') => {}
                filter => options.filters.push(filter.to_string()),
            }
        }
        options
    }

    fn wants(&self, name: &str) -> bool {
        if self.ignored_only {
            return false;
        }
        self.filters.is_empty()
            || self.filters.iter().any(|f| {
                if self.exact {
                    f == name
                } else {
                    name.contains(f.as_str())
                }
            })
    }
}

// ---------------------------------------------------------------------------
// The stand-in renderer

fn stand_in(mode: &str) {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    match mode {
        "silent" => {
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        "exit" => std::process::exit(9),
        _ => {}
    }
    let Ok((seq, Message::Hello { .. })) = wire::read_message(&mut input, Direction::ToRenderer)
    else {
        std::process::exit(2);
    };
    let version = if mode == "old" {
        WIRE_VERSION + 1
    } else {
        WIRE_VERSION
    };
    let hello = Message::Hello {
        role: Role::Renderer,
        version,
    };
    if wire::write_message(&mut output, seq, &hello).is_err() {
        return;
    }
    loop {
        let Ok((seq, message)) = wire::read_message(&mut input, Direction::ToRenderer) else {
            return;
        };
        match message {
            Message::Ping { nonce } => {
                let _ = wire::write_message(&mut output, seq, &Message::Pong { nonce });
            }
            Message::Render(req) => serve(&mut input, &mut output, seq, &req),
            Message::Shutdown => std::process::exit(0),
            _ => std::process::exit(3),
        }
    }
}

fn serve(input: &mut impl io::Read, output: &mut impl Write, seq: u32, req: &RenderRequest) {
    let html = req.html.as_str();
    let mut doc = RenderedDocument {
        tab_id: req.tab_id,
        nav_id: req.nav_id,
        title: format!("rendered {}", req.url),
        dom_nodes: 3,
        blocked_resources: 0,
        html: html.to_uppercase(),
    };
    if html.contains("HANG") {
        std::thread::sleep(Duration::from_secs(30));
    }
    if html.contains("CRASH") {
        std::process::exit(7);
    }
    if html.contains("ABORT") {
        std::process::abort();
    }
    if html.contains("GARBAGE") {
        let _ = output.write_all(b"this is not a frame at all, just noise");
        let _ = output.flush();
        std::thread::sleep(Duration::from_secs(30));
    }
    if html.contains("OVERSIZE") {
        let mut head = wire::encode(seq, &Message::Rendered(doc.clone())).unwrap();
        head[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let _ = output.write_all(&head[..wire::HEADER_BYTES]);
        let _ = output.flush();
        std::thread::sleep(Duration::from_secs(30));
    }
    if html.contains("BROWSERMSG") {
        // Pretend to be the browser. The frame is valid, but not from here.
        let frame = wire::encode(seq, &Message::Ping { nonce: 1 }).unwrap();
        let _ = output.write_all(&frame);
        let _ = output.flush();
        std::thread::sleep(Duration::from_secs(30));
    }
    if html.contains("OTHERTAB") {
        doc.tab_id += 1;
    }
    if html.contains("OLDNAV") {
        doc.nav_id = doc.nav_id.wrapping_sub(1);
    }
    if html.contains("ENV") {
        let leaked: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| k != "ANVIL_STAND_IN")
            .collect();
        let dir = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        doc.title = format!("env=[{}] dir={dir}", leaked.join(","));
    }
    if html.contains("STDERR") {
        let chunk = vec![b'e'; 64 * 1024];
        let mut err = io::stderr().lock();
        for _ in 0..32 {
            let _ = err.write_all(&chunk);
        }
    }
    if html.contains("HOG") {
        let mut keep: Vec<Vec<u8>> = Vec::new();
        for _ in 0..64 {
            keep.push(vec![1u8; 8 * 1024 * 1024]);
        }
        std::thread::sleep(Duration::from_secs(30));
        drop(keep);
    }
    if html.contains("REFUSE") {
        let failure = RenderFailure {
            tab_id: req.tab_id,
            nav_id: req.nav_id,
            code: FailureCode::Limit,
            detail: "script budget".to_string(),
        };
        let _ = wire::write_message(output, seq, &Message::Failed(failure));
        return;
    }
    if html.contains("SPIN") {
        // Answer, but keep a thread busy in the background, the way a
        // renderer that has been taken over might mine or scan.
        std::thread::spawn(|| {
            let mut n = 0u64;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        });
    }
    if html.contains("DEAF") {
        // Answer, then stop reading.
        let _ = wire::write_message(output, seq, &Message::Rendered(doc));
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    let fetches = if html.contains("FLOOD") {
        10_000
    } else if html.contains("TRICKLE") {
        400
    } else if html.contains("FETCH") {
        2
    } else {
        0
    };
    // A trickle is a page script calling fetch() over and over.
    let kind = if html.contains("TRICKLE") {
        FetchKind::ScriptFetch
    } else {
        FetchKind::Stylesheet
    };
    let mut got = Vec::new();
    for n in 0..fetches {
        let want = FetchRequest {
            nav_id: req.nav_id,
            kind,
            url: format!("https://example.test/{n}.css"),
        };
        if wire::write_message(output, seq, &Message::Fetch(want)).is_err() {
            return;
        }
        match wire::read_message(input, Direction::ToRenderer) {
            Ok((_, Message::FetchReply(reply))) => {
                got.push(format!(
                    "{}:{}",
                    reply.status,
                    String::from_utf8_lossy(&reply.body)
                ));
            }
            _ => return,
        }
    }
    if !got.is_empty() {
        doc.title = got.join(" ");
    }
    let _ = wire::write_message(output, seq, &Message::Rendered(doc));
}

// ---------------------------------------------------------------------------
// Helpers

fn stand_in_command(mode: &str) -> RendererCommand {
    let me = std::env::current_exe().expect("test binary path");
    RendererCommand::new(me).env("ANVIL_STAND_IN", mode)
}

fn quick_limits() -> RendererLimits {
    RendererLimits {
        startup: Duration::from_secs(10),
        reply: Duration::from_secs(10),
        ..RendererLimits::default()
    }
}

fn launch(mode: &str, limits: RendererLimits) -> RendererProcess {
    RendererProcess::launch(&stand_in_command(mode), limits).expect("stand-in starts")
}

fn request(html: &str) -> RenderRequest {
    RenderRequest {
        tab_id: 4,
        nav_id: 11,
        url: "https://example.test/".to_string(),
        html: html.to_string(),
        csp_header: None,
        scripts_enabled: true,
        viewport_width: 800,
        viewport_height: 600,
        https_only: false,
        http_allowed_hosts: Vec::new(),
    }
}

fn no_fetch(_: &FetchRequest) -> FetchReply {
    panic!("this page should not fetch anything");
}

/// Render `html` and expect the renderer to be killed or to die.
fn render_failure(process: &mut RendererProcess, html: &str) -> RendererFailure {
    match process.render(&request(html), &mut no_fetch) {
        Ok(outcome) => panic!("expected a failure, got {outcome:?}"),
        Err(failure) => failure,
    }
}

#[cfg(target_os = "linux")]
fn gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        // A reaped process has no /proc entry. A zombie still has one, so
        // this also checks that the launcher waited for it.
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn gone(_pid: u32) -> bool {
    true
}

/// After a failure the handle is dead and says so.
fn assert_dead(process: &mut RendererProcess) {
    assert!(!process.is_alive());
    assert_eq!(process.ping(), Err(RendererFailure::Closed));
    assert!(gone(process.pid()), "renderer process is still there");
}

// ---------------------------------------------------------------------------
// Checks

fn renders_a_document_and_answers_pings() {
    let mut process = launch("good", quick_limits());
    assert!(process.is_alive());
    process.ping().unwrap();
    for nav in 0..3 {
        let mut req = request("<p>hello</p>");
        req.nav_id = nav;
        match process.render(&req, &mut no_fetch).unwrap() {
            RenderOutcome::Done(doc) => {
                assert_eq!(doc.html, "<P>HELLO</P>");
                assert_eq!(doc.title, "rendered https://example.test/");
                assert_eq!(doc.nav_id, nav);
            }
            other => panic!("{other:?}"),
        }
    }
    process.ping().unwrap();
    assert!(process.is_alive());
}

fn starts_with_a_clean_environment() {
    assert!(std::env::var_os("PATH").is_some(), "test needs PATH set");
    let mut process = launch("good", quick_limits());
    let RenderOutcome::Done(doc) = process.render(&request("ENV"), &mut no_fetch).unwrap() else {
        panic!("refused");
    };
    if cfg!(windows) {
        let names = doc.title["env=[".len()..]
            .split(']')
            .next()
            .unwrap_or_default();
        for name in names.split(',').filter(|n| !n.is_empty()) {
            let name = name.to_ascii_lowercase();
            assert!(
                name == "systemroot" || name == "windir",
                "{name} was passed on"
            );
        }
    } else {
        assert_eq!(doc.title, "env=[] dir=/");
    }
}

fn fetches_go_through_the_browser() {
    let mut process = launch("good", quick_limits());
    let mut asked = Vec::new();
    let mut fetch = |want: &FetchRequest| {
        asked.push(want.url.clone());
        FetchReply {
            nav_id: 999, // The launcher puts the right one back.
            status: 200,
            content_type: "text/css".to_string(),
            body: format!("body{}", asked.len()).into_bytes(),
        }
    };
    let RenderOutcome::Done(doc) = process.render(&request("FETCH"), &mut fetch).unwrap() else {
        panic!("refused");
    };
    assert_eq!(
        asked,
        ["https://example.test/0.css", "https://example.test/1.css"]
    );
    assert_eq!(doc.title, "200:body1 200:body2");

    // A reply too big for the wire goes back as a failed fetch, and the
    // renderer keeps working.
    let mut huge = |_: &FetchRequest| FetchReply {
        nav_id: 0,
        status: 200,
        content_type: String::new(),
        body: vec![0u8; wire::MAX_FETCH_BODY_BYTES + 1],
    };
    let RenderOutcome::Done(doc) = process.render(&request("FETCH"), &mut huge).unwrap() else {
        panic!("refused");
    };
    assert_eq!(doc.title, "0: 0:");
    process.ping().unwrap();
}

fn too_many_fetches_kill_the_renderer() {
    let limits = RendererLimits {
        max_fetches_per_render: 5,
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let mut count = 0;
    let mut fetch = |_: &FetchRequest| {
        count += 1;
        FetchReply {
            nav_id: 0,
            status: 0,
            content_type: String::new(),
            body: Vec::new(),
        }
    };
    let failure = process.render(&request("FLOOD"), &mut fetch).unwrap_err();
    assert_eq!(failure, RendererFailure::Unexpected("too many fetches"));
    assert_eq!(count, 5);
    assert_dead(&mut process);
}

fn slow_fetches_hit_the_page_time_limit() {
    // Every answer comes well within the reply time, but the page as a
    // whole takes too long.
    let limits = RendererLimits {
        reply: Duration::from_secs(5),
        page_total: Duration::from_millis(800),
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let mut kinds = Vec::new();
    let mut fetch = |want: &FetchRequest| {
        kinds.push(want.kind);
        std::thread::sleep(Duration::from_millis(50));
        FetchReply {
            nav_id: 0,
            status: 200,
            content_type: String::new(),
            body: b"{}".to_vec(),
        }
    };
    let started = Instant::now();
    let failure = process.render(&request("TRICKLE"), &mut fetch).unwrap_err();
    assert_eq!(failure, RendererFailure::PageTooSlow);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(kinds.len() > 2 && kinds.len() < 400, "{}", kinds.len());
    assert!(kinds.iter().all(|k| *k == FetchKind::ScriptFetch));
    assert_dead(&mut process);
}

fn a_refused_page_keeps_the_renderer() {
    let mut process = launch("good", quick_limits());
    match process.render(&request("REFUSE"), &mut no_fetch).unwrap() {
        RenderOutcome::Refused(f) => assert_eq!(f.code, FailureCode::Limit),
        other => panic!("{other:?}"),
    }
    process.ping().unwrap();
}

fn a_hung_renderer_is_killed_at_the_deadline() {
    let limits = RendererLimits {
        reply: Duration::from_millis(300),
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let started = Instant::now();
    assert_eq!(
        render_failure(&mut process, "HANG"),
        RendererFailure::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_dead(&mut process);
}

fn a_crash_reports_its_exit_code() {
    let mut process = launch("good", quick_limits());
    let failure = render_failure(&mut process, "<p>CRASH</p>");
    assert_eq!(
        failure,
        RendererFailure::Exited {
            code: Some(7),
            signal: None
        }
    );
    assert_eq!(failure.to_string(), "the renderer stopped (exit code 7)");
    assert!(!failure.to_string().contains("<p>"));
    assert_dead(&mut process);
}

fn an_abort_is_contained() {
    let mut process = launch("good", quick_limits());
    let failure = render_failure(&mut process, "ABORT");
    #[cfg(unix)]
    assert_eq!(
        failure,
        RendererFailure::Exited {
            code: None,
            signal: Some(6)
        }
    );
    #[cfg(not(unix))]
    assert!(matches!(failure, RendererFailure::Exited { .. }));
    assert_dead(&mut process);
    // The browser side carries on and can start a new renderer.
    launch("good", quick_limits()).ping().unwrap();
}

fn garbage_kills_the_renderer() {
    let mut process = launch("good", quick_limits());
    let failure = render_failure(&mut process, "GARBAGE");
    assert_eq!(failure, RendererFailure::BadFrame(WireError::BadMagic));
    assert_dead(&mut process);
}

fn an_oversize_frame_is_refused_early() {
    let mut process = launch("good", quick_limits());
    let started = Instant::now();
    let failure = render_failure(&mut process, "OVERSIZE");
    assert!(
        matches!(
            failure,
            RendererFailure::BadFrame(WireError::TooLarge { .. })
        ),
        "{failure:?}"
    );
    // Refused from the header alone, without waiting for a body.
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_dead(&mut process);
}

fn a_browser_message_from_the_renderer_is_refused() {
    let mut process = launch("good", quick_limits());
    let failure = render_failure(&mut process, "BROWSERMSG");
    assert!(
        matches!(
            failure,
            RendererFailure::BadFrame(WireError::WrongDirection(_))
        ),
        "{failure:?}"
    );
    assert_dead(&mut process);
}

fn a_page_for_another_tab_is_refused() {
    let mut process = launch("good", quick_limits());
    assert_eq!(
        render_failure(&mut process, "OTHERTAB"),
        RendererFailure::Unexpected("a page for another tab")
    );
    assert_dead(&mut process);
}

fn a_page_for_an_old_navigation_is_refused() {
    let mut process = launch("good", quick_limits());
    assert_eq!(
        render_failure(&mut process, "OLDNAV"),
        RendererFailure::Unexpected("a page for another navigation")
    );
    assert_dead(&mut process);
}

fn a_silent_renderer_fails_the_handshake() {
    let limits = RendererLimits {
        startup: Duration::from_millis(300),
        ..quick_limits()
    };
    let started = Instant::now();
    let failure = RendererProcess::launch(&stand_in_command("silent"), limits).unwrap_err();
    assert_eq!(failure, RendererFailure::Handshake("no hello in time"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

fn another_protocol_version_is_refused() {
    let failure = RendererProcess::launch(&stand_in_command("old"), quick_limits()).unwrap_err();
    assert_eq!(
        failure,
        RendererFailure::Handshake("another protocol version")
    );
}

fn a_renderer_that_exits_at_once_is_reported() {
    let failure = RendererProcess::launch(&stand_in_command("exit"), quick_limits()).unwrap_err();
    assert_eq!(
        failure,
        RendererFailure::Exited {
            code: Some(9),
            signal: None
        }
    );
}

fn a_program_that_is_not_there_fails_to_start() {
    let command = RendererCommand::new("/this/renderer/does/not/exist");
    let failure = RendererProcess::launch(&command, quick_limits()).unwrap_err();
    assert!(matches!(failure, RendererFailure::Start(_)));
    assert!(!failure.is_crash());
}

fn a_stderr_flood_neither_blocks_nor_grows() {
    let limits = RendererLimits {
        stderr_cap_bytes: 1000,
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    // 2 MiB of stderr, far more than a pipe holds. If nobody drained it the
    // renderer would block and this would time out.
    let RenderOutcome::Done(_) = process.render(&request("STDERR"), &mut no_fetch).unwrap() else {
        panic!("refused");
    };
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(process.stderr_text().len(), 1000);
}

fn memory_over_the_cap_kills_the_renderer() {
    if !cfg!(target_os = "linux") {
        // Only Linux has the memory watch. Windows uses the job object.
        return;
    }
    let limits = RendererLimits {
        memory_cap_bytes: 64 * 1024 * 1024,
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let failure = render_failure(&mut process, "HOG");
    match failure {
        RendererFailure::OverMemory { resident_bytes } => {
            assert!(resident_bytes > 64 * 1024 * 1024);
        }
        other => panic!("{other:?}"),
    }
    assert_dead(&mut process);
}

fn dropping_the_handle_ends_the_process() {
    let mut process = launch("good", quick_limits());
    process.ping().unwrap();
    let pid = process.pid();
    drop(process);
    assert!(gone(pid));

    // Even one that is stuck in the middle of a page.
    let limits = RendererLimits {
        reply: Duration::from_millis(100),
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let pid = process.pid();
    let _ = process.render(&request("HANG"), &mut no_fetch);
    drop(process);
    assert!(gone(pid));
}

fn shutdown_lets_the_renderer_exit() {
    let process = launch("good", quick_limits());
    let pid = process.pid();
    let started = Instant::now();
    process.shutdown();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(gone(pid));
}

fn a_crash_loop_stops_relaunching() {
    let mut budget = RelaunchBudget::new(2, Duration::from_secs(60));
    let mut launches = 0;
    while budget.may_launch(Instant::now()) {
        launches += 1;
        let mut process = launch("good", quick_limits());
        let failure = render_failure(&mut process, "CRASH");
        assert!(failure.is_crash());
        budget.record_crash(Instant::now());
        assert!(launches <= 2);
    }
    assert_eq!(launches, 2);
}

fn an_idle_renderer_passes_its_heartbeat() {
    let mut process = launch("good", quick_limits());
    process.heartbeat().unwrap();
    let RenderOutcome::Done(_) = process
        .render(&request("<p>hi</p>"), &mut no_fetch)
        .unwrap()
    else {
        panic!("refused");
    };
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(100));
        process.heartbeat().unwrap();
    }
    assert!(process.is_alive());
}

fn a_renderer_busy_while_idle_is_killed() {
    let limits = RendererLimits {
        idle_cpu: Duration::from_millis(200),
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let pid = process.pid();
    let RenderOutcome::Done(_) = process.render(&request("SPIN"), &mut no_fetch).unwrap() else {
        panic!("refused");
    };
    if cfg!(target_os = "linux") {
        // Wait until the spinning thread has clearly used its allowance.
        let start = anvil_sandbox::launcher::cpu_time(pid).expect("cpu time");
        let deadline = Instant::now() + Duration::from_secs(10);
        while anvil_sandbox::launcher::cpu_time(pid)
            .is_some_and(|t| t < start + Duration::from_millis(400))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        match process.heartbeat() {
            Err(RendererFailure::BusyWhileIdle { cpu_ms }) => assert!(cpu_ms > 200, "{cpu_ms}"),
            other => panic!("{other:?}"),
        }
        assert!(!process.is_alive());
        assert!(gone(pid));
        assert_eq!(process.heartbeat(), Err(RendererFailure::Closed));
    } else {
        // No CPU check here yet: the heartbeat only pings.
        process.heartbeat().unwrap();
    }
}

fn a_renderer_that_stops_reading_fails_its_heartbeat() {
    let limits = RendererLimits {
        reply: Duration::from_millis(500),
        ..quick_limits()
    };
    let mut process = launch("good", limits);
    let pid = process.pid();
    let RenderOutcome::Done(_) = process.render(&request("DEAF"), &mut no_fetch).unwrap() else {
        panic!("refused");
    };
    let started = Instant::now();
    assert_eq!(process.heartbeat(), Err(RendererFailure::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(gone(pid));
}
