// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Local scripts from the `userscripts` folder, seen from the session.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anvil_browser::permissions::{ScriptRule, action_key};
use anvil_browser::{Session, Viewport};

fn session(dir: &Path) -> Session {
    let mut session = Session::new(Viewport {
        width: 640.0,
        height: 480.0,
    });
    session.set_tab_worker_command(
        &PathBuf::from(env!("CARGO_BIN_EXE_anvil")),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session.set_userscripts_dir(dir);
    session
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anvil-us-it-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn text_of(session: &Session, id: &str) -> String {
    let page = session.active().current().expect("page");
    let node = page.document.get_element_by_id(id).expect("element");
    page.document.text_content(node)
}

/// `/page` has an `out` paragraph its own script fills; `/api` answers text.
fn server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let (kind, body) = if head.starts_with("GET /api ") {
                ("text/plain", "from-api".to_string())
            } else {
                (
                    "text/html",
                    "<html><head><title>plain page</title></head><body>\
                     <p id=\"out\">before</p><p id=\"own\"></p>\
                     <script>document.getElementById('own').textContent='page ran'</script>\
                     </body></html>"
                        .to_string(),
                )
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    port
}

const SCRIPT: &str = "// ==UserScript==\n\
// @match http://127.0.0.1/*\n\
// @grant none\n\
// ==/UserScript==\n\
var own = document.getElementById('own').textContent;\n\
var same = fetch('/api');\n\
var other = fetch('http://localhost:' + location.port + '/api');\n\
document.getElementById('out').textContent = 'local ran after ' + own + ', ' + same.body + ', cross:' + other.status;\n";

#[test]
fn a_local_script_runs_on_a_matching_page_after_the_page_scripts() {
    let port = server();
    let dir = folder("runs");
    fs::write(dir.join("hello.js"), SCRIPT).unwrap();
    let mut session = session(&dir);
    let url = format!("http://127.0.0.1:{port}/page");

    // New files start off.
    session.open_target(&url).unwrap();
    assert_eq!(text_of(&session, "out"), "before");

    assert!(session.set_userscript_enabled("hello.js", true));
    session.open_target(&url).unwrap();
    assert_eq!(text_of(&session, "own"), "page ran");
    assert_eq!(
        text_of(&session, "out"),
        "local ran after page ran, from-api, cross:403"
    );

    // A host the file doesn't match.
    session
        .open_target(&format!("http://localhost:{port}/page"))
        .unwrap();
    assert_eq!(text_of(&session, "out"), "before");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn site_and_global_script_switches_stop_local_scripts() {
    let port = server();
    let dir = folder("blocked");
    fs::write(dir.join("hello.js"), SCRIPT).unwrap();
    let mut session = session(&dir);
    session.set_userscript_enabled("hello.js", true);
    let url = format!("http://127.0.0.1:{port}/page");

    session.update_site_permissions(|p| p.set_scripts("127.0.0.1", ScriptRule::Block));
    session.open_target(&url).unwrap();
    assert_eq!(text_of(&session, "out"), "before");
    assert_eq!(text_of(&session, "own"), "");

    session.update_site_permissions(|p| p.set_scripts("127.0.0.1", ScriptRule::Default));
    // Through the profile, which also brings back which files are on.
    let mut snap = session.to_profile_snapshot();
    snap.settings.scripts_enabled = false;
    session.apply_profile_snapshot(snap);
    assert!(session.userscripts().scripts()[0].enabled);
    session.open_target(&url).unwrap();
    assert_eq!(text_of(&session, "out"), "before");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn local_scripts_never_touch_anvil_pages_and_need_the_key_to_change() {
    let dir = folder("page");
    fs::write(
        dir.join("everywhere.js"),
        "// @match *://*/*\ndocument.title = 'changed';\n",
    )
    .unwrap();
    let mut session = session(&dir);

    // Without the key the page only shows.
    session
        .open_target("anvil://settings/scripts?file=everywhere.js&set=on")
        .unwrap();
    assert!(!session.userscripts().scripts()[0].enabled);
    let html = session.active().current().unwrap().document.to_html();
    assert!(html.contains("everywhere.js"));
    assert!(html.contains("didn't come from this page"));
    assert!(html.contains(action_key()));

    session
        .open_target(&format!(
            "anvil://settings/scripts?file=everywhere.js&set=on&key={}",
            action_key()
        ))
        .unwrap();
    assert!(session.userscripts().scripts()[0].enabled);
    assert!(session.userscripts().any_ready());

    for page in [
        "anvil://home",
        "anvil://settings",
        "anvil://settings/scripts",
    ] {
        session.open_target(page).unwrap();
        let current = session.active().current().unwrap();
        assert_ne!(current.title, "changed", "{page}");
        assert!(
            !current
                .document
                .to_html()
                .contains("document.title = 'changed'"),
            "{page}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
