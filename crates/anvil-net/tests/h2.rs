// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! HTTP/2 client tests over an in-memory connection.
//!
//! Each test writes the server's side ahead of time, runs one exchange, then
//! reads back what the client sent.

use anvil_hpack::{Decoder, Encoder, Header};
use anvil_net::h2::{
    self, Frame, FrameReader, H2Error, PREFACE, Parsed, Request, Transport, code, flag, kind,
    parse_frame, setting,
};
use anvil_net::{HttpVersion, NetError, Url, build_post_request_with_cookie, parse_url};

/// Server bytes fed to the client in small pieces, and what the client wrote.
struct Script {
    input: Vec<u8>,
    pos: usize,
    piece: usize,
    sent: Vec<u8>,
}

impl Script {
    fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            pos: 0,
            piece: 7,
            sent: Vec::new(),
        }
    }

    /// Frames the client sent after its preface.
    fn client_frames(&self) -> Vec<Frame> {
        assert!(self.sent.starts_with(PREFACE));
        let mut reader = FrameReader::new();
        reader.push(&self.sent[PREFACE.len()..]);
        let mut frames = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            frames.push(frame);
        }
        frames
    }
}

impl Transport for Script {
    fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.sent.extend_from_slice(data);
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        let n = (self.input.len() - self.pos).min(buf.len()).min(self.piece);
        buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn settings(values: &[(u16, u32)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, value) in values {
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&value.to_be_bytes());
    }
    Frame::new(kind::SETTINGS, 0, 0, payload).to_bytes()
}

fn block(encoder: &mut Encoder, fields: &[(&str, &str)]) -> Vec<u8> {
    let list: Vec<Header> = fields.iter().map(|(n, v)| Header::new(n, v)).collect();
    let mut out = Vec::new();
    encoder.encode(&list, &mut out);
    out
}

fn get_request() -> Request {
    Request {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.test".into(),
        path: "/page?x=1".into(),
        headers: vec![
            ("Accept".into(), "text/html".into()),
            ("Connection".into(), "close".into()),
            ("Cookie".into(), "a=1".into()),
        ],
        body: Vec::new(),
    }
}

fn decode_request(frames: &[Frame]) -> Vec<(String, String)> {
    let mut decoder = Decoder::new();
    let mut block = Vec::new();
    for frame in frames {
        if frame.kind == kind::HEADERS || frame.kind == kind::CONTINUATION {
            block.extend_from_slice(&frame.payload);
            if frame.has(flag::END_HEADERS) {
                break;
            }
        }
    }
    decoder
        .decode(&block)
        .unwrap()
        .iter()
        .map(|h| (h.name_str(), h.value_str()))
        .collect()
}

#[test]
fn get_sends_pseudo_fields_and_reads_a_padded_body() {
    let mut enc = Encoder::new();
    let mut input = settings(&[(setting::MAX_CONCURRENT_STREAMS, 100)]);
    let head = block(
        &mut enc,
        &[(":status", "200"), ("content-type", "text/plain")],
    );
    input.extend(Frame::new(kind::HEADERS, flag::END_HEADERS, 1, head).to_bytes());
    let mut padded = vec![3u8];
    padded.extend_from_slice(b"hello ");
    padded.extend_from_slice(&[0, 0, 0]);
    input.extend(Frame::new(kind::DATA, flag::PADDED, 1, padded).to_bytes());
    input.extend(Frame::new(kind::DATA, flag::END_STREAM, 1, b"world".to_vec()).to_bytes());

    let mut io = Script::new(input);
    let response = h2::exchange(&mut io, &get_request(), 1 << 20).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.reason, "");
    assert_eq!(response.version, HttpVersion::Http2);
    assert_eq!(response.header("content-type"), Some("text/plain"));
    assert_eq!(response.body, b"hello world");

    let frames = io.client_frames();
    assert_eq!(frames[0].kind, kind::SETTINGS);
    match parse_frame(&frames[0]).unwrap() {
        Parsed::Settings { ack, values } => {
            assert!(!ack);
            assert!(values.contains(&(setting::ENABLE_PUSH, 0)));
        }
        other => panic!("{other:?}"),
    }
    let fields = decode_request(&frames);
    assert_eq!(
        &fields[..4],
        &[
            (":method".to_string(), "GET".to_string()),
            (":scheme".to_string(), "https".to_string()),
            (":authority".to_string(), "example.test".to_string()),
            (":path".to_string(), "/page?x=1".to_string()),
        ]
    );
    assert!(fields.contains(&("accept".into(), "text/html".into())));
    assert!(fields.contains(&("cookie".into(), "a=1".into())));
    assert!(!fields.iter().any(|(n, _)| n == "connection"));
    let headers = frames.iter().find(|f| f.kind == kind::HEADERS).unwrap();
    assert!(headers.has(flag::END_STREAM));
    // The server's SETTINGS was acknowledged.
    assert!(
        frames
            .iter()
            .any(|f| f.kind == kind::SETTINGS && f.has(flag::ACK))
    );
}

#[test]
fn continuation_interim_responses_ping_and_trailers_are_handled() {
    let mut enc = Encoder::new();
    let mut input = settings(&[]);
    let early = block(&mut enc, &[(":status", "103"), ("link", "</a.css>")]);
    input.extend(Frame::new(kind::HEADERS, flag::END_HEADERS, 1, early).to_bytes());
    let long = "v".repeat(300);
    let head = block(
        &mut enc,
        &[(":status", "404"), ("x-long", &long), ("server", "test")],
    );
    let (a, b) = head.split_at(head.len() / 2);
    input.extend(Frame::new(kind::HEADERS, 0, 1, a.to_vec()).to_bytes());
    input.extend(Frame::new(kind::CONTINUATION, flag::END_HEADERS, 1, b.to_vec()).to_bytes());
    input.extend(Frame::new(kind::PING, 0, 0, b"12345678".to_vec()).to_bytes());
    input.extend(Frame::new(0xfa, 0, 0, b"unknown".to_vec()).to_bytes());
    input.extend(Frame::new(kind::DATA, 0, 1, b"gone".to_vec()).to_bytes());
    let trailers = block(&mut enc, &[("x-check", "ok")]);
    input.extend(
        Frame::new(
            kind::HEADERS,
            flag::END_HEADERS | flag::END_STREAM,
            1,
            trailers,
        )
        .to_bytes(),
    );

    let mut io = Script::new(input);
    let response = h2::exchange(&mut io, &get_request(), 1 << 20).unwrap();
    assert_eq!(response.status, 404);
    assert_eq!(response.header("x-long"), Some(long.as_str()));
    assert_eq!(response.body, b"gone");
    let frames = io.client_frames();
    let pong = frames
        .iter()
        .find(|f| f.kind == kind::PING)
        .expect("ping answered");
    assert!(pong.has(flag::ACK));
    assert_eq!(pong.payload, b"12345678");
    // Received DATA was handed back to the connection window.
    assert!(frames.iter().any(|f| f.kind == kind::WINDOW_UPDATE
        && f.stream_id == 0
        && f.payload == 4u32.to_be_bytes()));
    assert_eq!(frames.last().unwrap().kind, kind::GOAWAY);
}

#[test]
fn a_large_post_body_waits_for_window_updates() {
    let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let mut request = get_request();
    request.method = "POST".into();
    request.body = body.clone();

    let mut enc = Encoder::new();
    let mut input = settings(&[(setting::INITIAL_WINDOW_SIZE, 1000)]);
    for _ in 0..100 {
        input.extend(
            Frame::new(kind::WINDOW_UPDATE, 0, 1, 1000u32.to_be_bytes().to_vec()).to_bytes(),
        );
        input.extend(
            Frame::new(kind::WINDOW_UPDATE, 0, 0, 1000u32.to_be_bytes().to_vec()).to_bytes(),
        );
    }
    let head = block(&mut enc, &[(":status", "201"), ("content-length", "2")]);
    input.extend(Frame::new(kind::HEADERS, flag::END_HEADERS, 1, head).to_bytes());
    input.extend(Frame::new(kind::DATA, flag::END_STREAM, 1, b"ok".to_vec()).to_bytes());

    let mut io = Script::new(input);
    io.piece = 13;
    let response = h2::exchange(&mut io, &request, 1 << 20).unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"ok");

    // Replay the windows to check the client never overran them. The client
    // reads one frame at a time, so credit arrives in the order sent.
    let mut reader = FrameReader::new();
    reader.push(&io.input);
    let mut server = Vec::new();
    while let Some(frame) = reader.next_frame().unwrap() {
        server.push(frame);
    }
    let mut credits = server
        .iter()
        .filter(|f| f.kind == kind::WINDOW_UPDATE || f.kind == kind::SETTINGS);
    let mut stream_window: i64 = 65_535;
    let mut conn_window: i64 = 65_535;
    let mut sent = Vec::new();
    for frame in io.client_frames() {
        if frame.kind != kind::DATA {
            continue;
        }
        let len = frame.payload.len() as i64;
        while stream_window.min(conn_window) < len {
            let credit = credits.next().expect("client sent past its window");
            if credit.kind == kind::SETTINGS {
                stream_window += 1000 - 65_535;
            } else if credit.stream_id == 0 {
                conn_window += 1000;
            } else {
                stream_window += 1000;
            }
        }
        stream_window -= len;
        conn_window -= len;
        sent.extend_from_slice(&frame.payload);
    }
    assert_eq!(sent, body);
}

#[test]
fn http_1_1_required_and_early_goaway_ask_for_fallback() {
    let mut input = settings(&[]);
    input.extend(
        Frame::new(
            kind::RST_STREAM,
            0,
            1,
            code::HTTP_1_1_REQUIRED.to_be_bytes().to_vec(),
        )
        .to_bytes(),
    );
    let mut io = Script::new(input);
    assert!(matches!(
        h2::exchange(&mut io, &get_request(), 1 << 20),
        Err(H2Error::Fallback)
    ));

    let mut payload = 0u32.to_be_bytes().to_vec();
    payload.extend_from_slice(&code::NO_ERROR.to_be_bytes());
    let mut io = Script::new(Frame::new(kind::GOAWAY, 0, 0, payload).to_bytes());
    assert!(matches!(
        h2::exchange(&mut io, &get_request(), 1 << 20),
        Err(H2Error::Fallback)
    ));

    let mut io = Script::new(
        Frame::new(
            kind::RST_STREAM,
            0,
            1,
            code::REFUSED_STREAM.to_be_bytes().to_vec(),
        )
        .to_bytes(),
    );
    assert!(matches!(
        h2::exchange(&mut io, &get_request(), 1 << 20),
        Err(H2Error::Fallback)
    ));
}

fn expect_error(input: Vec<u8>, max_body: usize) -> NetError {
    let mut io = Script::new(input);
    match h2::exchange(&mut io, &get_request(), max_body) {
        Err(H2Error::Net(err)) => err,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn broken_responses_are_errors() {
    let mut enc = Encoder::new();
    let ok_head = block(&mut enc, &[(":status", "200")]);

    // Body over the cap.
    let mut input = Frame::new(kind::HEADERS, flag::END_HEADERS, 1, ok_head.clone()).to_bytes();
    input.extend(Frame::new(kind::DATA, flag::END_STREAM, 1, vec![b'x'; 50]).to_bytes());
    assert!(matches!(expect_error(input, 10), NetError::BodyTooLarge));

    // Server push after push was turned off.
    let input = Frame::new(kind::PUSH_PROMISE, flag::END_HEADERS, 1, vec![0, 0, 0, 2]).to_bytes();
    assert!(matches!(
        expect_error(input, 1000),
        NetError::InvalidResponse
    ));

    // DATA before HEADERS.
    let input = Frame::new(kind::DATA, 0, 1, b"x".to_vec()).to_bytes();
    assert!(matches!(
        expect_error(input, 1000),
        NetError::InvalidResponse
    ));

    // Bad status, upper-case name, pseudo-field after a regular one,
    // a connection field, and a request pseudo-field.
    for fields in [
        vec![Header::new(":status", "2000")],
        vec![Header::new(":status", "200"), Header::new("X-Up", "1")],
        vec![Header::new("x-first", "1"), Header::new(":status", "200")],
        vec![
            Header::new(":status", "200"),
            Header::new("connection", "close"),
        ],
        vec![Header::new(":path", "/")],
    ] {
        let mut out = Vec::new();
        Encoder::new().encode(&fields, &mut out);
        let input =
            Frame::new(kind::HEADERS, flag::END_HEADERS | flag::END_STREAM, 1, out).to_bytes();
        assert!(
            matches!(expect_error(input, 1000), NetError::InvalidResponse),
            "{fields:?}"
        );
    }

    // A different frame inside a header block.
    let mut input = Frame::new(kind::HEADERS, 0, 1, ok_head.clone()).to_bytes();
    input.extend(Frame::new(kind::PING, 0, 0, vec![0; 8]).to_bytes());
    assert!(matches!(
        expect_error(input, 1000),
        NetError::InvalidResponse
    ));

    // Content-Length that does not match the body.
    let mut enc = Encoder::new();
    let head = block(&mut enc, &[(":status", "200"), ("content-length", "9")]);
    let mut input = Frame::new(kind::HEADERS, flag::END_HEADERS, 1, head).to_bytes();
    input.extend(Frame::new(kind::DATA, flag::END_STREAM, 1, b"abc".to_vec()).to_bytes());
    assert!(matches!(
        expect_error(input, 1000),
        NetError::InvalidResponse
    ));

    // A window pushed past its limit.
    let input = Frame::new(
        kind::WINDOW_UPDATE,
        0,
        0,
        0x7fff_ffffu32.to_be_bytes().to_vec(),
    )
    .to_bytes();
    assert!(matches!(
        expect_error(input, 1000),
        NetError::InvalidResponse
    ));

    // A frame longer than the client accepts.
    let input = Frame::new(kind::DATA, 0, 1, vec![0; 20_000]).to_bytes();
    assert!(matches!(
        expect_error(input, 1 << 20),
        NetError::InvalidResponse
    ));

    // The connection ends before the response does.
    let input = Frame::new(kind::HEADERS, flag::END_HEADERS, 1, ok_head).to_bytes();
    assert!(matches!(expect_error(input, 1000), NetError::Io(_)));
}

#[test]
fn frame_checks_follow_the_rfc() {
    let bad = [
        Frame::new(kind::DATA, 0, 0, vec![]),
        Frame::new(kind::DATA, flag::PADDED, 1, vec![5, 1, 2]),
        Frame::new(kind::DATA, flag::PADDED, 1, vec![]),
        Frame::new(kind::HEADERS, flag::PRIORITY, 1, vec![0, 0]),
        Frame::new(kind::PRIORITY, 0, 1, vec![0; 4]),
        Frame::new(kind::RST_STREAM, 0, 0, vec![0; 4]),
        Frame::new(kind::SETTINGS, 0, 1, vec![]),
        Frame::new(kind::SETTINGS, 0, 0, vec![0; 5]),
        Frame::new(kind::SETTINGS, flag::ACK, 0, vec![0; 6]),
        Frame::new(kind::PING, 0, 0, vec![0; 7]),
        Frame::new(kind::PING, 0, 3, vec![0; 8]),
        Frame::new(kind::GOAWAY, 0, 0, vec![0; 7]),
        Frame::new(kind::WINDOW_UPDATE, 0, 0, vec![0; 4]),
        Frame::new(kind::WINDOW_UPDATE, 0, 0, vec![0; 3]),
        Frame::new(kind::CONTINUATION, 0, 0, vec![]),
    ];
    for frame in bad {
        assert!(parse_frame(&frame).is_err(), "{frame:?}");
    }
    let frame = Frame::new(
        kind::DATA,
        flag::PADDED | flag::END_STREAM,
        1,
        vec![2, b'a', 0, 0],
    );
    assert_eq!(
        parse_frame(&frame).unwrap(),
        Parsed::Data {
            stream_id: 1,
            data: b"a".to_vec(),
            flow_len: 4,
            end_stream: true
        }
    );
    // The reserved bit of the stream number is ignored when read.
    let mut raw = Frame::new(kind::PING, 0, 0, vec![0; 8]).to_bytes();
    raw[5] |= 0x80;
    let mut reader = FrameReader::new();
    reader.push(&raw[..10]);
    assert_eq!(reader.next_frame().unwrap(), None);
    reader.push(&raw[10..]);
    assert_eq!(reader.next_frame().unwrap().unwrap().stream_id, 0);
}

#[test]
fn http1_request_bytes_turn_into_an_http2_request() {
    let url: Url = parse_url("https://example.test:8443/form?a=b").unwrap();
    let raw = build_post_request_with_cookie(&url, "text/plain", b"body", Some("s=1"));
    let request = Request::from_http1("https", &raw).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.authority, "example.test:8443");
    assert_eq!(request.path, "/form?a=b");
    assert_eq!(request.body, b"body");
    let list = request.header_list();
    let names: Vec<String> = list.iter().map(|h| h.name_str()).collect();
    assert!(!names.contains(&"host".to_string()));
    assert!(!names.contains(&"connection".to_string()));
    assert!(names.contains(&"content-length".to_string()));
    let cookie = list.iter().find(|h| h.name == b"cookie").unwrap();
    assert!(cookie.never_indexed);
    assert!(Request::from_http1("https", b"GET / HTTP/1.1\r\n\r\n").is_none());
}
