// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! The certificate page and its exceptions, against a local TLS server with
//! a certificate from a throwaway CA. Needs the `openssl` command line tool;
//! without it each test prints a note and returns.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anvil_browser::permissions::action_key;
use anvil_browser::{Session, Viewport};

struct Server {
    child: Child,
    dir: PathBuf,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn openssl(dir: &Path, args: &[&str]) -> bool {
    Command::new("openssl")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `openssl s_server -www` on a self-signed `localhost` certificate.
fn untrusted_server(tag: &str) -> Option<Server> {
    if !openssl(&std::env::temp_dir(), &["version"]) {
        eprintln!("openssl command not found; skipping");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("anvil-cert-page-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(openssl(
        &dir,
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost",
            "-keyout",
            "leaf.key",
            "-out",
            "leaf.pem",
        ],
    ));
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let child = Command::new("openssl")
        .args([
            "s_server", "-quiet", "-www", "-cert", "leaf.pem", "-key", "leaf.key",
        ])
        .args(["-accept", &port.to_string()])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server { child, dir, port };
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "test TLS server did not start");
        thread::sleep(Duration::from_millis(50));
    }
    Some(server)
}

fn session() -> Session {
    let mut session = Session::new(Viewport {
        width: 640.0,
        height: 480.0,
    });
    session.set_tab_worker_command(
        &PathBuf::from(env!("CARGO_BIN_EXE_anvil")),
        &["--renderer".to_string()],
        Duration::from_secs(10),
    );
    session
}

fn title(session: &Session) -> String {
    session.active().current().expect("page").title.clone()
}

fn html(session: &Session) -> String {
    session.active().current().expect("page").document.to_html()
}

#[test]
fn an_untrusted_certificate_gets_its_own_page_and_no_silent_way_past() {
    let Some(server) = untrusted_server("page") else {
        return;
    };
    let url = format!("https://localhost:{}/", server.port);
    let mut session = session();
    session.open_target(&url).unwrap();
    assert_eq!(title(&session), "Certificate problem");
    let page = html(&session);
    assert!(page.contains("Untrusted certificate"), "{page}");
    assert!(page.contains("ERROR ID-17"), "{page}");
    assert!(page.contains("Nothing was sent to the site"), "{page}");
    assert!(page.contains("tls_session=localhost"), "{page}");
    assert!(page.contains("allow_insecure_tls=localhost"), "{page}");
    assert!(page.contains(action_key()));
    assert!(!page.contains("Navigation failed"));

    // Without the key nothing changes.
    let open = format!("https%3A%2F%2Flocalhost%3A{}%2F", server.port);
    session
        .open_target(&format!(
            "anvil://settings/permissions?tls_session=localhost&open={open}"
        ))
        .unwrap();
    assert!(session.session_tls_exceptions().is_empty());
    assert!(html(&session).contains("didn't come from this page"));
    session.open_target(&url).unwrap();
    assert_eq!(title(&session), "Certificate problem");

    // An address on another host is refused even with the key.
    session
        .open_target(&format!(
            "anvil://settings/permissions?tls_session=localhost&open=https%3A%2F%2Fother.test%2F&key={}",
            action_key()
        ))
        .unwrap();
    assert!(session.session_tls_exceptions().is_empty());

    // With the key: allowed for this run only, and the page opens.
    session
        .open_target(&format!(
            "anvil://settings/permissions?tls_session=localhost&open={open}&key={}",
            action_key()
        ))
        .unwrap();
    assert_eq!(session.session_tls_exceptions(), ["localhost".to_string()]);
    assert_ne!(title(&session), "Certificate problem");
    assert_ne!(title(&session), "Navigation failed");
    assert!(html(&session).contains("s_server"), "{}", html(&session));
    assert_eq!(session.active().current().unwrap().url, url);
    assert!(session.site_permissions().insecure_tls_hosts().is_empty());
    assert!(
        session
            .to_profile_snapshot()
            .site_permissions
            .insecure_tls_hosts()
            .is_empty(),
        "a for-now exception is never saved"
    );

    // A new run starts without it.
    let mut fresh = self::session();
    fresh.open_target(&url).unwrap();
    assert_eq!(title(&fresh), "Certificate problem");
}

#[test]
fn other_tls_failures_keep_the_plain_error_page() {
    // A plain TCP server that answers a TLS hello with HTTP text.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        }
    });
    let mut session = session();
    session
        .open_target(&format!("https://localhost:{port}/"))
        .unwrap();
    assert_ne!(title(&session), "Certificate problem");
    assert_eq!(title(&session), "Navigation failed");
}
