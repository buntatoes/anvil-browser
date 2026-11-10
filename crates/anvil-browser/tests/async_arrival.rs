// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Async scripts run in fetch-completion order. Defer stays in document order.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use anvil_browser::script_order::run_document_scripts;
use anvil_dom::{ScriptSlot, parse_html};
use anvil_script::ScriptRuntime;

fn serve(body: &'static str, delay: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        thread::sleep(delay);
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf);
        let payload = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(payload.as_bytes());
    });
    port
}

fn read_body(port: u16) -> Result<Vec<u8>, String> {
    use std::net::TcpStream;
    let mut sock = TcpStream::connect(("127.0.0.1", port)).map_err(|err| err.to_string())?;
    let _ = sock.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    let mut buf = Vec::new();
    sock.read_to_end(&mut buf).map_err(|err| err.to_string())?;
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no header".to_string())?;
    Ok(buf[split + 4..].to_vec())
}

#[test]
fn async_runs_in_arrival_order_and_defer_stays_in_document_order() {
    let slow = serve(
        "document.getElementById('out').textContent += 'A';",
        Duration::from_millis(200),
    );
    let fast = serve(
        "document.getElementById('out').textContent += 'B';",
        Duration::from_millis(10),
    );
    let html = format!(
        "<p id=\"out\">init</p>\
         <script src=\"http://127.0.0.1:{slow}/a.js\" async></script>\
         <script src=\"http://127.0.0.1:{fast}/b.js\" async></script>\
         <script src=\"http://127.0.0.1:{fast}/d.js\" defer></script>"
    );
    let mut doc = parse_html(&html);
    let mut runtime = ScriptRuntime::new();
    let fetch = move |slot: &ScriptSlot| {
        let src = slot.src.clone().unwrap_or_default();
        let port = if src.contains(&format!(":{slow}/")) {
            slow
        } else {
            fast
        };
        if src.contains("d.js") {
            return Ok(b"document.getElementById('out').textContent += 'D';".to_vec());
        }
        read_body(port)
    };
    let log = run_document_scripts(
        &mut doc,
        &mut runtime,
        true,
        &fetch,
        |_runtime, _state| {},
        |_runtime| None,
        false,
    );
    let out = doc.text_content(doc.get_element_by_id("out").unwrap());
    assert!(
        out.contains("BA") || out.ends_with("BAD") || out.contains("initBA"),
        "arrival order lost: {out} {:?}",
        log.order
    );
    assert!(out.contains('D'), "{out}");
    let b = out.find('B').unwrap();
    let a = out.find('A').unwrap();
    let d = out.find('D').unwrap();
    assert!(b < a && a < d, "expected B then A then D, got {out}");
    let defer_at = log
        .order
        .iter()
        .position(|entry| entry.starts_with("defer:"))
        .expect("defer");
    let async_positions: Vec<_> = log
        .order
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.starts_with("async:"))
        .map(|(i, _)| i)
        .collect();
    assert!(
        async_positions.iter().all(|i| *i < defer_at),
        "defer ran before async finished: {:?}",
        log.order
    );
    let dcl = log
        .events
        .iter()
        .position(|event| event == "DOMContentLoaded")
        .expect("dcl");
    assert!(
        log.events[..dcl]
            .iter()
            .filter(|e| *e == "readystatechange")
            .count()
            >= 2,
        "defer should be after interactive: {:?}",
        log.events
    );
}
