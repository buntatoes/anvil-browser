// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Renderers started through a confinement tool, and without one.
//!
//! These checks run the real `anvil --renderer`. Where the machine has
//! `bwrap` or `unshare` they check that a renderer started through it is in
//! its own namespaces, can't see this process, and can't read its memory,
//! while one started plainly can. Where the machine has neither tool they
//! check that renderers start plainly and say why.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anvil_browser::{Session, Viewport};
use anvil_sandbox::confine::{self, ConfineMode, Helper, ToolKind};
use anvil_sandbox::launcher::{
    RenderOutcome, RendererCommand, RendererFailure, RendererLimits, RendererProcess,
};
use anvil_sandbox::wire::{FetchReply, FetchRequest, RenderRequest};

fn plain() -> RendererCommand {
    RendererCommand::new(env!("CARGO_BIN_EXE_anvil")).arg("--renderer")
}

fn limits() -> RendererLimits {
    RendererLimits {
        startup: Duration::from_secs(20),
        reply: Duration::from_secs(20),
        ..RendererLimits::default()
    }
}

fn helper(kind: ToolKind) -> Option<Helper> {
    confine::find_helpers()
        .into_iter()
        .find(|helper| helper.kind == kind)
}

fn page(nav_id: u64, html: &str) -> RenderRequest {
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

fn refuse(want: &FetchRequest) -> FetchReply {
    FetchReply {
        nav_id: want.nav_id,
        status: 0,
        content_type: String::new(),
        body: Vec::new(),
    }
}

/// The first readable mapping of this process, as a page number.
fn readable_page_of_this_process() -> u64 {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let line = maps
        .lines()
        .find(|line| {
            line.split_whitespace()
                .nth(1)
                .is_some_and(|p| p.starts_with('r'))
        })
        .unwrap();
    let start = line.split('-').next().unwrap();
    u64::from_str_radix(start, 16).unwrap() / 4096
}

/// Run a small shell check the way `command` would start a renderer, and
/// return what it printed. It looks for this process in `/proc` and tries
/// to read one page of its memory.
fn probe_through(command: &RendererCommand) -> String {
    let me = std::process::id();
    let page = readable_page_of_this_process();
    let script = format!(
        "if [ -e /proc/{me}/status ]; then echo seen; else echo hidden; fi; \
         n=$(dd if=/proc/{me}/mem bs=4096 skip={page} count=1 status=none 2>/dev/null | wc -c); \
         echo read=$n"
    );
    let probe = command
        .clone()
        .with_program("/usr/bin/sh", ["-c", script.as_str()]);
    let out = probe
        .to_command()
        .stdin(Stdio::null())
        .output()
        .expect("probe runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn wait_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn the_decision_matches_what_this_machine_has() {
    let decision = confine::decide(ConfineMode::Auto, &plain(), &limits());
    let found = confine::find_helpers();
    match &decision.in_use {
        Some(helper) => {
            assert!(found.contains(helper));
            let status = decision.self_test.as_ref().expect("self-test status");
            let ours = confine::own_namespaces();
            for (theirs, mine) in status.namespaces.iter().zip(ours).take(6) {
                assert_ne!(*theirs, mine, "{status:?}");
            }
            assert!(status.visible_processes <= 4, "{status:?}");
            assert!(decision.command_for(&plain()).is_wrapped());
            eprintln!("confined by {}: {status:?}", helper.kind.name());
        }
        None => {
            // No tool, or none passed: renderers start plainly, with a reason.
            assert!(!decision.command_for(&plain()).is_wrapped());
            assert!(!decision.notes.is_empty());
            eprintln!("not confined: {:?}", decision.notes);
        }
    }
}

#[test]
fn a_tool_that_fails_its_self_test_falls_back_and_says_so() {
    // A "bwrap" that is really a program that exits at once.
    let broken = Helper {
        kind: ToolKind::Bwrap,
        path: PathBuf::from("/usr/bin/false"),
        prlimit: None,
    };
    let decision = confine::decide_with(ConfineMode::Auto, &plain(), &limits(), vec![broken]);
    assert!(decision.in_use.is_none());
    assert!(!decision.command_for(&plain()).is_wrapped());
    assert_eq!(decision.notes.len(), 1);
    assert!(
        decision.notes[0].starts_with("bwrap was found but failed its self-test"),
        "{:?}",
        decision.notes
    );
    // With a working tool after it, that one is used instead.
    if let Some(unshare) = helper(ToolKind::Unshare) {
        let broken = Helper {
            kind: ToolKind::Bwrap,
            path: PathBuf::from("/usr/bin/false"),
            prlimit: None,
        };
        let decision = confine::decide_with(
            ConfineMode::Auto,
            &plain(),
            &limits(),
            vec![broken, unshare],
        );
        assert_eq!(
            decision.in_use.as_ref().map(|h| h.kind),
            Some(ToolKind::Unshare),
            "{:?}",
            decision.notes
        );
        assert!(decision.notes[0].contains("bwrap"));
    }
}

#[test]
fn with_no_tool_at_all_renderers_start_as_before_and_it_says_so() {
    let decision = confine::decide_with(ConfineMode::Auto, &plain(), &limits(), Vec::new());
    assert!(decision.in_use.is_none());
    assert!(decision.self_test.is_none());
    assert_eq!(decision.notes.len(), 1);
    if cfg!(target_os = "linux") {
        assert!(
            decision.notes[0].starts_with("neither bwrap nor unshare was found"),
            "{:?}",
            decision.notes
        );
    }
    let command = decision.command_for(&plain());
    assert!(!command.is_wrapped());
    assert_eq!(command.program(), plain().program());
    let mut process = RendererProcess::launch(&command, limits()).expect("starts");
    assert_eq!(process.pid(), process.started_pid());
    let outcome = process
        .render(
            &page(1, "<p>plain</p><script>document.title='plain'</script>"),
            &mut refuse,
        )
        .expect("renders");
    match outcome {
        RenderOutcome::Done(doc, _) => assert_eq!(doc.title, "plain"),
        other => panic!("{other:?}"),
    }
    process.shutdown();
}

#[test]
fn without_a_tool_the_renderer_can_see_and_read_this_process() {
    let out = probe_through(&plain());
    assert!(out.contains("seen"), "{out}");
    // Plain processes of the same user can read each other's memory unless
    // the kernel's ptrace restrictions say otherwise.
    let restricted = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .map(|v| v.trim() != "0")
        .unwrap_or(false);
    if !restricted {
        assert!(out.contains("read=4096"), "{out}");
    }
}

#[test]
fn through_each_tool_the_renderer_can_neither_see_nor_read_this_process() {
    for kind in [ToolKind::Bwrap, ToolKind::Unshare] {
        let Some(helper) = helper(kind) else {
            eprintln!("{} is not on this machine", kind.name());
            continue;
        };
        let wrapped = helper.wrap(&plain()).unwrap();
        let out = probe_through(&wrapped);
        assert!(out.contains("hidden"), "{}: {out}", kind.name());
        assert!(out.contains("read=0"), "{}: {out}", kind.name());
    }
}

#[test]
fn a_confined_renderer_renders_is_watched_and_ends_with_its_wrapper() {
    for kind in [ToolKind::Bwrap, ToolKind::Unshare] {
        let Some(helper) = helper(kind) else {
            continue;
        };
        let command = helper.wrap(&plain()).unwrap();
        let mut process = RendererProcess::launch(&command, limits()).expect("starts");
        let renderer = process.pid();
        let wrapper = process.started_pid();
        assert_ne!(renderer, wrapper, "{}", kind.name());
        // The pid watched for memory and CPU is the renderer itself.
        let comm = std::fs::read_to_string(format!("/proc/{renderer}/comm")).unwrap();
        assert_eq!(comm.trim(), "anvil", "{}", kind.name());
        let status = process.status().unwrap();
        assert_eq!(status.limits[0], Some(256 << 20));
        let outcome = process
            .render(
                &page(1, "<p id=x>confined</p><script>document.title='t'</script>"),
                &mut refuse,
            )
            .unwrap();
        match outcome {
            RenderOutcome::Done(doc, _) => assert_eq!(doc.title, "t"),
            other => panic!("{}: {other:?}", kind.name()),
        }
        process.heartbeat().unwrap();
        drop(process);
        assert!(
            wait_gone(renderer),
            "{} left its renderer running",
            kind.name()
        );
    }
}

#[test]
fn a_confined_renderer_killed_by_a_signal_is_reported_as_a_crash() {
    let Some(helper) = helper(ToolKind::Bwrap) else {
        return;
    };
    let command = helper.wrap(&plain()).unwrap();
    let mut process = RendererProcess::launch(&command, limits()).expect("starts");
    let renderer = process.pid();
    let killed = std::process::Command::new("/usr/bin/kill")
        .args(["-9", &renderer.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let failure = process
        .render(&page(1, "<p>x</p>"), &mut refuse)
        .unwrap_err();
    assert!(
        matches!(
            failure,
            RendererFailure::Exited {
                signal: Some(9),
                ..
            }
        ),
        "{failure:?}"
    );
    assert!(failure.is_crash());
}

#[test]
fn tab_renderers_follow_the_setting() {
    let file = std::env::temp_dir().join(format!("anvil-confine-tab-{}.html", std::process::id()));
    std::fs::write(&file, "<title>tab</title><p>tab page</p>").unwrap();
    let ours = confine::own_namespaces();
    for mode in [ConfineMode::Off, ConfineMode::Auto] {
        let mut session = Session::new(Viewport {
            width: 640.0,
            height: 480.0,
        });
        session.set_tab_worker_command(
            std::path::Path::new(env!("CARGO_BIN_EXE_anvil")),
            &["--renderer".to_string()],
            Duration::from_secs(20),
        );
        session.set_renderer_confinement(mode);
        session.open_target(file.to_str().unwrap()).unwrap();
        let pid = session.tab_worker_pid(0).expect("tab renderer");
        let link = std::fs::read_link(format!("/proc/{pid}/ns/pid")).unwrap();
        let id = confine::parse_namespace_link(&link.to_string_lossy()).unwrap();
        let decision = session.tab_workers().confinement().unwrap();
        if mode == ConfineMode::Auto && decision.in_use.is_some() {
            assert_ne!(
                id, ours[0],
                "a confined tab renderer shares the pid namespace"
            );
        } else {
            assert_eq!(id, ours[0], "{mode:?}: {:?}", decision.notes);
        }
    }
    let _ = std::fs::remove_file(&file);
}

/// The window starts a tab's renderer on the thread that loads the page,
/// and that thread ends when the page is done. The renderer has to outlive
/// it.
#[test]
fn a_confined_renderer_outlives_the_thread_that_started_it() {
    for kind in [ToolKind::Bwrap, ToolKind::Unshare] {
        let Some(helper) = helper(kind) else {
            continue;
        };
        let command = helper.wrap(&plain()).unwrap();
        let mut process = std::thread::spawn(move || {
            RendererProcess::launch(&command, limits()).expect("starts")
        })
        .join()
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            process.is_alive(),
            "{} renderer ended with its thread",
            kind.name()
        );
        process
            .ping()
            .unwrap_or_else(|err| panic!("{}: {err}", kind.name()));
        process.shutdown();
    }
}
fn page_text(session: &Session) -> String {
    let page = session.active().current().expect("page");
    page.document.text_content(page.document.body())
}

/// The tab renderer's process id. `tab_worker_pid` is `None` while another
/// thread holds that renderer, which a heartbeat does for a moment every
/// interval, so a missing id is read again for up to a second before it
/// counts as no renderer.
fn live_pid(session: &Session) -> Option<u32> {
    for _ in 0..100 {
        if let Some(pid) = session.tab_worker_pid(0) {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn session_with_real_renderer() -> Session {
    let mut session = Session::new(Viewport {
        width: 900.0,
        height: 700.0,
    });
    session.set_tab_worker_command(
        std::path::Path::new(env!("CARGO_BIN_EXE_anvil")),
        &["--renderer".to_string()],
        Duration::from_secs(20),
    );
    session
}

/// The line for `name` in the confinement table, as page text.
fn table_line<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(name)
        .unwrap_or_else(|| panic!("no {name} in {text}"));
    let rest = &text[start + name.len()..];
    let end = ["Active", "Limited", "Not active", "Unknown"]
        .iter()
        .filter_map(|state| rest.find(state).map(|at| at + state.len()))
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn the_security_page_shows_what_the_renderer_reads_both_ways() {
    let mut session = session_with_real_renderer();
    session.set_renderer_confinement(ConfineMode::Auto);
    session.open_target("anvil://security").unwrap();
    let text = page_text(&session);
    assert!(text.contains("Renderer confinement"), "{text}");
    assert!(text.contains("Confinement: automatic"), "{text}");
    let confined = session
        .tab_workers()
        .confinement()
        .unwrap()
        .in_use
        .is_some();
    let expect = if confined { "Active" } else { "Not active" };
    for name in [
        "Own user namespace",
        "Own pid namespace",
        "Own network namespace",
        "Can't see the window's process",
    ] {
        assert!(table_line(&text, name).ends_with(expect), "{name}: {text}");
    }
    // There is no system call filter either way, and the page says so.
    assert!(table_line(&text, "System call filter (seccomp)").ends_with("Not active"));
    // A file size limit of 0 never reads as stopping every write.
    assert!(!table_line(&text, "File size limit").ends_with("Active"));

    // Turned off, the same lines read back as not active.
    session.set_renderer_confinement(ConfineMode::Off);
    session.open_target("anvil://security").unwrap();
    let text = page_text(&session);
    assert!(text.contains("Confinement: off"), "{text}");
    assert!(text.contains("turned off in the settings"), "{text}");
    for name in [
        "Own user namespace",
        "Own pid namespace",
        "Own network namespace",
        "Host files read-only",
        "File size limit",
    ] {
        assert!(
            table_line(&text, name).ends_with("Not active"),
            "{name}: {text}"
        );
    }
}

/// The two lines about writing files, as a real renderer reads them back
/// when started through `decision`.
fn file_write_rows(decision: &confine::Confinement) -> (confine::State, confine::State, String) {
    let command = decision.command_for(&plain());
    let mut process = RendererProcess::launch(&command, limits()).expect("starts");
    let status = process.status().expect("status");
    process.shutdown();
    let rows = confine::protections(decision, Ok(&status), &confine::own_namespaces());
    let state = |name: &str| {
        rows.iter()
            .find(|row| row.name == name)
            .map(|row| row.state)
            .unwrap_or_else(|| panic!("no row {name}: {rows:?}"))
    };
    (
        state("Host files read-only"),
        state("File size limit"),
        status.writable_mounts,
    )
}

#[test]
fn only_bwrap_makes_host_files_read_only() {
    use confine::State;
    // Through unshare the renderer still sees the host's files, so writing
    // must not read as stopped, whatever the file size limit is.
    if let Some(unshare) = helper(ToolKind::Unshare) {
        let with_prlimit = unshare.prlimit.is_some();
        let decision = confine::decide_with(ConfineMode::Auto, &plain(), &limits(), vec![unshare]);
        assert_eq!(
            decision.in_use.as_ref().map(|h| h.kind),
            Some(ToolKind::Unshare)
        );
        let (read_only, size, mounts) = file_write_rows(&decision);
        assert_eq!(read_only, State::NotActive, "{mounts}");
        assert_ne!(size, State::Active);
        if with_prlimit {
            assert_eq!(size, State::Limited);
        }
    }
    // Through bwrap only private mounts are writable.
    if let Some(bwrap) = helper(ToolKind::Bwrap) {
        let decision = confine::decide_with(ConfineMode::Auto, &plain(), &limits(), vec![bwrap]);
        assert_eq!(
            decision.in_use.as_ref().map(|h| h.kind),
            Some(ToolKind::Bwrap)
        );
        let (read_only, size, mounts) = file_write_rows(&decision);
        assert_eq!(read_only, State::Active, "{mounts}");
        assert_ne!(size, State::Active);
    }
    // Turned off, neither is in place.
    let decision = confine::decide(ConfineMode::Off, &plain(), &limits());
    let (read_only, size, mounts) = file_write_rows(&decision);
    assert_eq!(read_only, State::NotActive, "{mounts}");
    assert_eq!(size, State::NotActive);
}

#[test]
fn only_the_built_in_page_can_change_the_setting() {
    let mut session = session_with_real_renderer();
    session.set_renderer_confinement(ConfineMode::Auto);
    // Without this run's key, as from a link on a web page, nothing changes.
    session
        .open_target("anvil://security?toggle=confinement")
        .unwrap();
    assert_eq!(session.settings().renderer_confinement, ConfineMode::Auto);
    let key = anvil_browser::permissions::action_key();
    session
        .open_target(&format!("anvil://security?key={key}&toggle=confinement"))
        .unwrap();
    assert_eq!(session.settings().renderer_confinement, ConfineMode::Off);
    assert!(page_text(&session).contains("Confinement: off"));
    session
        .open_target(&format!("anvil://security?key={key}&toggle=confinement"))
        .unwrap();
    assert_eq!(session.settings().renderer_confinement, ConfineMode::Auto);
}

#[test]
fn running_tab_renderers_are_listed_with_how_they_were_started() {
    let file = std::env::temp_dir().join(format!("anvil-confine-list-{}.html", std::process::id()));
    std::fs::write(&file, "<p>listed</p>").unwrap();
    let mut session = session_with_real_renderer();
    session.set_renderer_confinement(ConfineMode::Auto);
    session.open_target(file.to_str().unwrap()).unwrap();
    let pid = session.tab_worker_pid(0).expect("renderer");
    let tool = session
        .tab_workers()
        .confinement()
        .unwrap()
        .in_use
        .as_ref()
        .map(|helper| helper.kind.name());
    session.new_tab();
    session.open_target("anvil://security").unwrap();
    let text = page_text(&session);
    let how = match tool {
        Some(name) => format!("process {pid}, started through {name}"),
        None => format!("process {pid}, started without a confinement tool"),
    };
    assert!(text.contains(&how), "{how} missing from {text}");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn an_idle_confined_renderer_passes_its_heartbeats_and_keeps_its_process() {
    let file = std::env::temp_dir().join(format!("anvil-confine-idle-{}.html", std::process::id()));
    std::fs::write(&file, "<p>idle</p>").unwrap();
    let mut session = session_with_real_renderer();
    session.set_renderer_confinement(ConfineMode::Auto);
    session.set_tab_heartbeat_interval(Duration::from_millis(100));
    session.open_target(file.to_str().unwrap()).unwrap();
    let pid = live_pid(&session).expect("renderer");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        live_pid(&session),
        Some(pid),
        "the idle renderer was replaced or ended"
    );
    let _ = std::fs::remove_file(&file);
}

/// A renderer whose program is under /tmp still starts through `bwrap`,
/// though `bwrap` gives it an empty /tmp. Through `unshare`, and with no
/// tool, the same copy starts too.
#[test]
fn a_renderer_kept_under_tmp_still_starts_through_each_tool() {
    let dir = PathBuf::from(format!("/tmp/anvil-confine-copy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("anvil");
    std::fs::copy(env!("CARGO_BIN_EXE_anvil"), &copy).unwrap();
    let command = RendererCommand::new(copy.to_str().unwrap()).arg("--renderer");
    let mut tried = Vec::new();
    for kind in [ToolKind::Bwrap, ToolKind::Unshare] {
        let Some(tool) = helper(kind) else {
            continue;
        };
        tried.push(kind);
        let decision = confine::decide_with(ConfineMode::Auto, &command, &limits(), vec![tool]);
        assert_eq!(
            decision.in_use.as_ref().map(|h| h.kind),
            Some(kind),
            "{:?}",
            decision.notes
        );
        let mut process =
            RendererProcess::launch(&decision.command_for(&command), limits()).unwrap();
        match process
            .render(&page(1, "<p>from tmp</p>"), &mut refuse)
            .unwrap()
        {
            RenderOutcome::Done(doc, _) => assert!(doc.html.contains("from tmp")),
            other => panic!("{other:?}"),
        }
    }
    let decision = confine::decide_with(ConfineMode::Auto, &command, &limits(), Vec::new());
    assert!(decision.in_use.is_none());
    let mut process = RendererProcess::launch(&decision.command_for(&command), limits()).unwrap();
    assert!(matches!(
        process
            .render(&page(1, "<p>plain</p>"), &mut refuse)
            .unwrap(),
        RenderOutcome::Done(..)
    ));
    drop(process);
    eprintln!("tools tried: {tried:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
