// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Independent loopback checks of the platform form route.
//! The marker is synthetic. Nothing here is a private file or an outside host.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anvil_dom::{PatternCheck, check_pattern, parse_html};
use anvil_net::{ContentSecurityPolicy, CookieJar};

use super::{
    ExchangeError, GateReport, Method, NavigationRequest, SelectedFiles, SubmitBlock,
    SubmitDispatch, activation_gate, exchange_form, navigation_from, submission_effect,
};

const MARKER: &str = "INERT-FILE-MARKER";

fn text_of(raw: &str) -> String {
    raw.to_string()
}

fn capture(response: Vec<u8>) -> (std::net::SocketAddr, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match sock.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..split]);
                        let length = head.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        });
                        let need = split + 4 + length.unwrap_or(0);
                        if length.is_none() || buf.len() >= need {
                            buf.truncate(need);
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sock.write_all(&response);
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    (addr, rx)
}

fn request_for(html: &str, page: &str) -> NavigationRequest {
    let doc = parse_html(html);
    let form = doc.get_element_by_id("f").expect("form");
    let effect = submission_effect(&doc, form, None, &SelectedFiles::new(), true).expect("effect");
    navigation_from(&effect, page, &ContentSecurityPolicy::default()).expect("navigation")
}

#[test]
fn loopback_get_has_the_query_and_no_body_or_type() {
    let (addr, rx) =
        capture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec());
    let page = format!("http://127.0.0.1:{}/page", addr.port());
    let html = format!(
        "<form id=\"f\" action=\"http://127.0.0.1:{}/search\" method=\"GET\"><input name=\"q\" value=\"anvil\"></form>",
        addr.port()
    );
    let request = request_for(&html, &page);
    assert_eq!(request.method, Method::Get);
    assert!(request.body.is_empty(), "built body {:?}", request.body);
    assert_eq!(request.content_type, None);
    let mut jar = CookieJar::new();
    let (_final, response) = exchange_form(
        &page,
        &request.url,
        false,
        "",
        &request.body,
        &mut jar,
        &ContentSecurityPolicy::default(),
    )
    .expect("get");
    assert_eq!(response.body, b"ok");
    let raw = rx.recv_timeout(Duration::from_secs(3)).expect("request");
    let raw = text_of(&raw);
    assert!(raw.starts_with("GET /search?q=anvil HTTP/1.1\r\n"), "{raw}");
    let head_end = raw.find("\r\n\r\n").expect("headers");
    let head = &raw[..head_end];
    let has = |name: &str| {
        head.lines()
            .any(|line| line.to_ascii_lowercase().starts_with(&format!("{name}:")))
    };
    assert!(!has("content-length"), "{raw}");
    assert!(!has("content-type"), "{raw}");
    assert_eq!(&raw[head_end + 4..], "");
}

#[test]
fn loopback_post_sends_method_type_and_body() {
    let (addr, rx) =
        capture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec());
    let page = format!("http://127.0.0.1:{}/page", addr.port());
    let html = format!(
        "<form id=\"f\" action=\"http://127.0.0.1:{}/save\" method=\"POST\"><input name=\"q\" value=\"anvil\"></form>",
        addr.port()
    );
    let request = request_for(&html, &page);
    assert_eq!(
        request.url,
        format!("http://127.0.0.1:{}/save", addr.port())
    );
    assert_eq!(
        request.content_type.as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(request.body, b"q=anvil");
    let mut jar = CookieJar::new();
    exchange_form(
        &page,
        &request.url,
        true,
        request.content_type.as_deref().unwrap(),
        &request.body,
        &mut jar,
        &ContentSecurityPolicy::default(),
    )
    .expect("post");
    let raw = rx.recv_timeout(Duration::from_secs(3)).expect("request");
    assert!(raw.starts_with("POST /save HTTP/1.1\r\n"), "{raw}");
    assert!(!raw.contains("POST /save?"), "{raw}");
    assert!(
        raw.contains("Content-Type: application/x-www-form-urlencoded\r\n"),
        "{raw}"
    );
    assert!(raw.ends_with("q=anvil"), "{raw}");
}

#[test]
fn loopback_post_multipart_and_text_plain_keep_crlf_and_type() {
    let (addr, rx) =
        capture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec());
    let page = format!("http://127.0.0.1:{}/page", addr.port());
    let html = format!(
        "<form id=\"f\" action=\"http://127.0.0.1:{}/save\" method=\"POST\" enctype=\"multipart/form-data\"><input name=\"q\" value=\"anvil\"><input type=\"file\" name=\"upload\" filename=\"note.txt\" data-file-content=\"{MARKER}\" value=\"note.txt\"></form>",
        addr.port()
    );
    let request = request_for(&html, &page);
    let content_type = request.content_type.clone().expect("type");
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("boundary")
        .to_string();
    let body = String::from_utf8(request.body.clone()).unwrap();
    assert!(body.contains(&boundary), "{body}");
    assert!(body.contains("\r\n"), "{body}");
    assert!(!body.contains(MARKER), "{body}");
    let mut jar = CookieJar::new();
    exchange_form(
        &page,
        &request.url,
        true,
        &content_type,
        &request.body,
        &mut jar,
        &ContentSecurityPolicy::default(),
    )
    .expect("multipart");
    let raw = rx.recv_timeout(Duration::from_secs(3)).expect("request");
    assert!(
        raw.contains(&format!("Content-Type: {content_type}\r\n")),
        "{raw}"
    );
    assert!(raw.contains(&format!("--{boundary}")), "{raw}");
    assert!(raw.contains("\r\n"), "{raw}");
    assert!(!raw.contains(MARKER), "{raw}");

    let (addr, rx) =
        capture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec());
    let page = format!("http://127.0.0.1:{}/page", addr.port());
    let html = format!(
        "<form id=\"f\" action=\"http://127.0.0.1:{}/plain\" method=\"POST\" enctype=\"text/plain\"><input name=\"q\" value=\"a b\"></form>",
        addr.port()
    );
    let request = request_for(&html, &page);
    assert_eq!(request.body, b"q=a b\r\n");
    assert_eq!(
        request.content_type.as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    let mut jar = CookieJar::new();
    exchange_form(
        &page,
        &request.url,
        true,
        request.content_type.as_deref().unwrap(),
        &request.body,
        &mut jar,
        &ContentSecurityPolicy::default(),
    )
    .expect("plain");
    let raw = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("plain request");
    assert!(
        raw.contains("Content-Type: text/plain;charset=UTF-8\r\n"),
        "{raw}"
    );
    assert!(raw.contains("q=a b\r\n"), "{raw}");
}

#[test]
fn form_action_does_not_use_default_src_and_blocks_before_connect() {
    let allowed = ContentSecurityPolicy::parse("default-src 'none'");
    assert!(
        allowed.allows_form_action("http://127.0.0.1/page", "http://127.0.0.1/save"),
        "absent form-action consulted default-src"
    );
    let (addr, rx) =
        capture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec());
    let page = format!("http://127.0.0.1:{}/page", addr.port());
    let url = format!("http://127.0.0.1:{}/save", addr.port());
    let mut jar = CookieJar::new();
    exchange_form(&page, &url, false, "", b"", &mut jar, &allowed)
        .expect("default-src none still allows");
    let raw = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("allowed get");
    assert!(raw.starts_with("GET /save HTTP/1.1\r\n"), "{raw}");

    let denied = ContentSecurityPolicy::parse("default-src *; form-action 'none'");
    assert!(
        !denied.allows_form_action("http://127.0.0.1/page", "http://127.0.0.1/save"),
        "form-action none was allowed"
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let blocked = format!(
        "http://127.0.0.1:{}/save",
        listener.local_addr().unwrap().port()
    );
    let page = format!(
        "http://127.0.0.1:{}/page",
        listener.local_addr().unwrap().port()
    );
    let mut jar = CookieJar::new();
    let err = exchange_form(
        &page,
        &blocked,
        true,
        "text/plain",
        b"q=1",
        &mut jar,
        &denied,
    );
    assert!(
        matches!(err, Err(ExchangeError::FormAction)),
        "denied hop connected or failed another way: {err:?}"
    );
    assert!(
        listener.accept().is_err(),
        "form-action none still accepted a socket"
    );
}

#[test]
fn a_cross_origin_redirect_does_not_receive_the_first_hops_cookie() {
    let second = TcpListener::bind("127.0.0.1:0").unwrap();
    let second_addr = second.local_addr().unwrap();
    let (tx2, rx2) = mpsc::channel();
    thread::spawn(move || {
        let (mut sock, _) = second.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).unwrap_or(0);
        let _ =
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let _ = tx2.send(String::from_utf8_lossy(&buf[..n]).into_owned());
    });
    let first = TcpListener::bind("127.0.0.1:0").unwrap();
    let first_addr = first.local_addr().unwrap();
    let location = format!("http://127.0.0.1:{}/land", second_addr.port());
    let redirect = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nSet-Cookie: hop=INERT\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let (tx1, rx1) = mpsc::channel();
    thread::spawn(move || {
        let (mut sock, _) = first.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).unwrap_or(0);
        let _ = sock.write_all(redirect.as_bytes());
        let _ = tx1.send(String::from_utf8_lossy(&buf[..n]).into_owned());
    });
    let page = format!("http://127.0.0.1:{}/page", first_addr.port());
    let start = format!("http://127.0.0.1:{}/go", first_addr.port());
    let mut jar = CookieJar::new();
    assert!(
        jar.store_set_cookie(&page, "marker=INERT; Path=/"),
        "cookie was not stored"
    );
    exchange_form(
        &page,
        &start,
        true,
        "application/x-www-form-urlencoded",
        b"q=anvil",
        &mut jar,
        &ContentSecurityPolicy::default(),
    )
    .expect("redirect");
    let first_raw = rx1.recv_timeout(Duration::from_secs(3)).expect("hop 1");
    let second_raw = rx2.recv_timeout(Duration::from_secs(3)).expect("hop 2");
    assert!(
        first_raw.to_ascii_lowercase().contains("cookie:"),
        "same-origin hop omitted the page cookie: {first_raw}"
    );
    assert!(
        first_raw.contains("marker=INERT"),
        "same-origin hop did not send the marker: {first_raw}"
    );
    assert!(
        !second_raw.to_ascii_lowercase().contains("cookie:"),
        "cross-origin hop received credentials: {second_raw}"
    );
    assert!(!second_raw.contains("marker=INERT"), "{second_raw}");
    assert!(!second_raw.contains("hop=INERT"), "{second_raw}");
}

#[test]
fn serializers_reject_page_asserted_file_bytes_and_pattern_is_bounded() {
    let html = format!(
        "<form id=\"f\" action=\"/save\" method=\"POST\" enctype=\"multipart/form-data\"><input type=\"file\" name=\"upload\" filename=\"note.txt\" data-file-content=\"{MARKER}\" value=\"note.txt\"></form>"
    );
    let doc = parse_html(&html);
    let form = doc.get_element_by_id("f").unwrap();
    let (_action, _method, content_type, body) = doc.serialize_form_with_submitter(form, None);
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
    assert!(!body.contains(MARKER), "script-host serializer sent {body}");
    assert!(body.contains("filename=\"\""), "{body}");
    assert!(body.contains("\r\n"), "{body}");

    let effect = submission_effect(&doc, form, None, &SelectedFiles::new(), false).expect("native");
    let native = String::from_utf8(effect.body).unwrap();
    assert!(!native.contains(MARKER), "native serializer sent {native}");

    let plain = parse_html(
        "<form id=\"f\" method=\"POST\" enctype=\"text/plain\"><input name=\"q\" value=\"a b\"></form>",
    );
    let form = plain.get_element_by_id("f").unwrap();
    let (_, _, plain_type, plain_body) = plain.serialize_form_with_submitter(form, None);
    assert_eq!(plain_type, "text/plain;charset=UTF-8");
    assert_eq!(plain_body, "q=a b\r\n");

    assert_eq!(check_pattern("a+", "b"), PatternCheck::Failed);
    assert_eq!(check_pattern("(?=a)", "zzz"), PatternCheck::Skip);
    let bad = parse_html("<input id=\"p\" pattern=\"a+\" value=\"b\">");
    let id = bad.get_element_by_id("p").unwrap();
    assert_eq!(bad.is_control_valid(id), Some(false));
    let skipped = parse_html("<input id=\"p\" pattern=\"(?=a)\" value=\"zzz\">");
    let id = skipped.get_element_by_id("p").unwrap();
    assert_ne!(
        skipped.is_control_valid(id),
        Some(false),
        "unsupported pattern was treated as a constraint"
    );

    let mut doc =
        parse_html("<form id=\"f\"><input id=\"q\" name=\"q\" pattern=\"a+\" value=\"b\"></form>");
    let form = doc.get_element_by_id("f").unwrap();
    let mut report = GateReport::default();
    let interactive = activation_gate(
        &mut doc,
        form,
        None,
        SubmitDispatch::Interactive,
        &mut report,
        |_, _, _, _, _| {},
    );
    assert_eq!(interactive, Err(SubmitBlock::Invalid));
    let mut report = GateReport::default();
    let direct = activation_gate(
        &mut doc,
        form,
        None,
        SubmitDispatch::Direct,
        &mut report,
        |_, _, _, _, _| {},
    );
    assert_eq!(direct, Ok(()));
}
