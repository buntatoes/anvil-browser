// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! HTTPS-only, per-site permissions, and cookie partitions, seen from the session.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anvil_browser::permissions::{ScriptRule, action_key};
use anvil_browser::{Session, Viewport};

fn session() -> Session {
    let mut session = Session::new(Viewport {
        width: 640.0,
        height: 480.0,
    });
    session.set_tab_worker_command(
        &PathBuf::from(env!("CARGO_BIN_EXE_anvil")),
        &["--tab-worker".to_string()],
        Duration::from_secs(10),
    );
    session
}

fn active_text(session: &Session) -> String {
    let page = session.active().current().expect("page");
    page.document.text_content(page.document.body())
}

fn out_text(session: &Session) -> String {
    let page = session.active().current().expect("page");
    let out = page.document.get_element_by_id("out").expect("out");
    page.document.text_content(out)
}

fn active_title(session: &Session) -> String {
    session.active().current().expect("page").title.clone()
}

/// Plain HTTP server on loopback. It records the first bytes of each
/// connection. `/page` runs a script, `/cookie` sets a cookie.
fn server() -> (String, Arc<Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = buf[..n].to_vec();
            log.lock().unwrap().push(head.clone());
            let text = String::from_utf8_lossy(&head);
            let (extra, body) = if text.starts_with("GET /cookie") {
                (
                    "Set-Cookie: visit=1; Path=/\r\n",
                    "<html><head><title>cookie</title></head><body>set</body></html>".to_string(),
                )
            } else {
                (
                    "",
                    "<html><head><title>plain page</title></head><body>\
                     <p id=\"out\">before</p>\
                     <script>document.getElementById('out').textContent='script ran'</script>\
                     </body></html>"
                        .to_string(),
                )
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (base, seen)
}

fn plain_requests(seen: &Arc<Mutex<Vec<Vec<u8>>>>) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|head| head.starts_with(b"GET ") || head.starts_with(b"POST "))
        .count()
}

fn link(query: &str) -> String {
    format!("anvil://settings/permissions?{query}&key={}", action_key())
}

#[test]
fn https_only_shows_a_warning_and_never_falls_back_on_its_own() {
    let (base, seen) = server();
    let url = format!("{base}/page");
    let mut session = session();
    session
        .open_target("anvil://settings?toggle=https_only")
        .unwrap();
    assert!(session.settings().https_only_mode);

    session.open_target(&url).unwrap();
    assert_eq!(active_title(&session), "Secure connection not available");
    assert!(active_text(&session).contains("Nothing was sent over plain HTTP"));
    assert_eq!(
        plain_requests(&seen),
        0,
        "no plain request reached the server"
    );
    let html = session.active().current().unwrap().document.to_html();
    assert!(html.contains("allow_http=127.0.0.1"));
    assert!(html.contains(action_key()));

    // The page's own link allows the host and opens the http address.
    let continue_link = format!(
        "anvil://settings/permissions?allow_http=127.0.0.1&open={}&key={}",
        anvil_browser::frontend::urlencode_component(&url),
        action_key()
    );
    session.open_target(&continue_link).unwrap();
    assert_eq!(active_title(&session), "plain page");
    assert_eq!(plain_requests(&seen), 1);
    assert!(
        session
            .site_permissions()
            .get("127.0.0.1")
            .unwrap()
            .allow_http
    );

    // Requiring HTTPS again brings the warning back.
    session
        .open_target(&link("site=127.0.0.1&http=require"))
        .unwrap();
    session.open_target(&url).unwrap();
    assert_eq!(active_title(&session), "Secure connection not available");
    assert_eq!(plain_requests(&seen), 1);
}

#[test]
fn permission_changes_need_the_page_key() {
    let mut session = session();
    session
        .open_target("anvil://settings/permissions?site=evil.test&http=allow")
        .unwrap();
    assert!(session.site_permissions().is_empty());
    assert!(active_text(&session).contains("nothing was changed"));
    session
        .open_target("anvil://settings/permissions?add=evil.test&key=wrong")
        .unwrap();
    assert!(session.site_permissions().is_empty());

    session
        .open_target(&link("add=https://Shop.Test/cart"))
        .unwrap();
    assert!(session.site_permissions().get("shop.test").is_some());
    assert!(active_text(&session).contains("shop.test"));
    assert_eq!(
        session.active().current().unwrap().url,
        "anvil://settings/permissions"
    );
    session
        .open_target(&link("site=shop.test&remove=1"))
        .unwrap();
    assert!(session.site_permissions().is_empty());
}

#[test]
fn a_site_can_block_scripts_and_cookies() {
    let (base, _seen) = server();
    let mut session = session();

    session.open_target(&format!("{base}/page")).unwrap();
    assert_eq!(out_text(&session), "script ran");

    session
        .open_target(&link("site=127.0.0.1&scripts=block"))
        .unwrap();
    assert_eq!(
        session.site_permissions().get("127.0.0.1").unwrap().scripts,
        ScriptRule::Block
    );
    session.open_target(&format!("{base}/page")).unwrap();
    assert_eq!(out_text(&session), "before");

    session.open_target(&format!("{base}/cookie")).unwrap();
    assert_eq!(session.cookie_jar().len(), 1);
    session
        .open_target(&link("site=127.0.0.1&cookies=block"))
        .unwrap();
    assert_eq!(session.cookie_jar().len(), 0, "stored cookies are removed");
    session.open_target(&format!("{base}/cookie")).unwrap();
    assert_eq!(session.cookie_jar().len(), 0, "new cookies are refused");

    session
        .open_target(&link("site=127.0.0.1&cookies=allow"))
        .unwrap();
    session.open_target(&format!("{base}/cookie")).unwrap();
    assert_eq!(session.cookie_jar().len(), 1);
    // The cookie lives in the partition of the page it was set under.
    assert_eq!(session.cookie_jar().cookies()[0].partition, "127.0.0.1");
    session
        .open_target(&link("site=127.0.0.1&clear_cookies=1"))
        .unwrap();
    assert_eq!(session.cookie_jar().len(), 0);
}

#[test]
fn site_permissions_survive_the_profile() {
    let mut session = session();
    session.update_site_permissions(|p| {
        p.set_scripts("example.test", ScriptRule::Allow);
        p.set_block_cookies("ads.test", true);
    });
    let snapshot = session.to_profile_snapshot();
    let mut other = self::session();
    other.apply_profile_snapshot(snapshot);
    assert_eq!(other.site_permissions(), session.site_permissions());
    assert!(other.cookie_jar().is_host_blocked("x.ads.test"));
    assert!(other.scripts_enabled_for("https://www.example.test/"));
}
