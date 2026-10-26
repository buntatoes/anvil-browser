// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Time-budgeted fuzz runs for the HTTP/2 frame reader, frame parser, and
//! client exchange.
//!
//! Each test runs seeded cases until its budget is spent (1.5 s by default,
//! `ANVIL_ROBUSTNESS_MS` to change it). Every exchange runs on its own thread
//! and must finish within 10 s, so a hang fails the test instead of stalling
//! it. A failing case prints its seed.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anvil_hpack::{Encoder, Header};
use anvil_net::NetError;
use anvil_net::h2::{
    self, Frame, FrameReader, MAX_FRAME_SIZE, MAX_HEADER_BLOCK, Parsed, Request, Transport, flag,
    kind, parse_frame,
};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let len = self.below(max + 1);
        (0..len).map(|_| self.byte()).collect()
    }
}

fn budget() -> Duration {
    let ms = std::env::var("ANVIL_ROBUSTNESS_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    Duration::from_millis(ms)
}

/// Run `case(seed)` for fresh seeds until the budget is spent.
fn for_seeds(base: u64, mut case: impl FnMut(u64)) -> usize {
    let start = Instant::now();
    let mut n = 0u64;
    while start.elapsed() < budget() && n < 200_000 {
        case(base ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        n += 1;
    }
    n as usize
}

/// Server bytes handed back in pieces of random size.
struct Script {
    input: Vec<u8>,
    pos: usize,
    piece: usize,
    sent: usize,
}

impl Transport for Script {
    fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.sent += data.len();
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        let n = (self.input.len() - self.pos).min(buf.len()).min(self.piece);
        buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn request(body: Vec<u8>) -> Request {
    Request {
        method: if body.is_empty() { "GET" } else { "POST" }.into(),
        scheme: "https".into(),
        authority: "example.test".into(),
        path: "/".into(),
        headers: vec![("accept".into(), "*/*".into())],
        body,
    }
}

type Outcome = Result<(u16, Vec<(String, String)>, Vec<u8>), String>;

/// One exchange against `input` on its own thread, with a hang limit.
fn exchange(seed: u64, input: Vec<u8>, piece: usize, body: Vec<u8>) -> Outcome {
    let (tx, rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(move || {
            let mut io = Script {
                input,
                pos: 0,
                piece: piece.max(1),
                sent: 0,
            };
            let out = h2::exchange_raw(&mut io, &request(body), 64 * 1024)
                .map_err(|err| format!("{err:?}"));
            let _ = tx.send(out);
        })
        .unwrap();
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(out) => {
            let _ = worker.join();
            out
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("seed {seed:#x}: exchange hung"),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("seed {seed:#x}: exchange panicked")
        }
    }
}

fn header_block(headers: &[Header]) -> Vec<u8> {
    let mut enc = Encoder::new();
    let mut out = Vec::new();
    enc.encode(headers, &mut out);
    out
}

/// A correct reply: SETTINGS, HEADERS (maybe split into CONTINUATIONs and
/// padded), and DATA frames that carry `body`.
fn good_reply(rng: &mut Rng, body: &[u8]) -> Vec<u8> {
    let mut out = Frame::new(kind::SETTINGS, 0, 0, vec![0, 3, 0, 0, 0, 100]).to_bytes();
    if rng.chance(2) {
        out.extend(Frame::new(kind::SETTINGS, flag::ACK, 0, Vec::new()).to_bytes());
    }
    if rng.chance(3) {
        out.extend(Frame::new(kind::PING, 0, 0, vec![7; 8]).to_bytes());
    }
    let block = header_block(&[
        Header::new(":status", "200"),
        Header::new("content-type", "text/plain"),
        Header::new("content-length", &body.len().to_string()),
    ]);
    let split = rng.below(block.len() + 1);
    let (first, rest) = block.split_at(split);
    let mut payload = first.to_vec();
    let mut flags = 0;
    if rng.chance(2) {
        let pad = rng.below(20);
        let mut padded = vec![pad as u8];
        padded.extend_from_slice(&payload);
        padded.extend(std::iter::repeat_n(0, pad));
        payload = padded;
        flags |= flag::PADDED;
    }
    if rest.is_empty() {
        flags |= flag::END_HEADERS;
    }
    out.extend(Frame::new(kind::HEADERS, flags, 1, payload).to_bytes());
    if !rest.is_empty() {
        let pieces: Vec<&[u8]> = rest.chunks(1 + rng.below(8)).collect();
        for (i, chunk) in pieces.iter().enumerate() {
            let last = i + 1 == pieces.len();
            let f = if last { flag::END_HEADERS } else { 0 };
            out.extend(Frame::new(kind::CONTINUATION, f, 1, chunk.to_vec()).to_bytes());
        }
    }
    if body.is_empty() {
        out.extend(Frame::new(kind::DATA, flag::END_STREAM, 1, Vec::new()).to_bytes());
    } else {
        let chunks: Vec<&[u8]> = body.chunks(1 + rng.below(MAX_FRAME_SIZE)).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let f = if i + 1 == chunks.len() {
                flag::END_STREAM
            } else {
                0
            };
            out.extend(Frame::new(kind::DATA, f, 1, chunk.to_vec()).to_bytes());
            if rng.chance(4) {
                out.extend(Frame::new(kind::WINDOW_UPDATE, 0, 0, vec![0, 0, 1, 0]).to_bytes());
            }
        }
    }
    out
}

/// A random frame of any type, often with bad fields.
fn random_frame(rng: &mut Rng) -> Frame {
    let kind = if rng.chance(8) {
        rng.byte()
    } else {
        rng.below(10) as u8
    };
    let stream = match rng.below(5) {
        0 => 0,
        1 => 1,
        2 => 3,
        3 => 0x7fff_ffff,
        _ => rng.next() as u32,
    };
    let len = match rng.below(6) {
        0 => 0,
        1 => 4,
        2 => 5,
        3 => 6,
        4 => 8,
        _ => rng.below(64),
    };
    let mut payload: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
    if kind == kind::SETTINGS && rng.chance(2) {
        payload.clear();
        for _ in 0..rng.below(6) {
            payload.extend_from_slice(&(rng.below(8) as u16).to_be_bytes());
            let value = match rng.below(4) {
                0 => 0,
                1 => u32::MAX,
                2 => 1 << 31,
                _ => rng.next() as u32,
            };
            payload.extend_from_slice(&value.to_be_bytes());
        }
    }
    Frame::new(kind, rng.byte(), stream, payload)
}

#[test]
fn every_frame_type_parses_or_errors_with_consistent_fields() {
    let n = for_seeds(0x4832_5052_5345_0001, |seed| {
        let mut rng = Rng::new(seed);
        let frame = random_frame(&mut rng);
        let Ok(parsed) = parse_frame(&frame) else {
            return;
        };
        match parsed {
            Parsed::Data {
                stream_id,
                data,
                flow_len,
                ..
            } => {
                assert_ne!(stream_id, 0, "seed {seed:#x}: DATA on stream 0");
                assert_eq!(flow_len as usize, frame.payload.len(), "seed {seed:#x}");
                assert!(data.len() <= frame.payload.len(), "seed {seed:#x}");
            }
            Parsed::Headers {
                stream_id, block, ..
            }
            | Parsed::Continuation {
                stream_id, block, ..
            } => {
                assert_ne!(stream_id, 0, "seed {seed:#x}: header block on stream 0");
                assert!(block.len() <= frame.payload.len(), "seed {seed:#x}");
            }
            Parsed::Settings { ack, values } => {
                assert_eq!(frame.stream_id, 0, "seed {seed:#x}");
                assert_eq!(frame.payload.len() % 6, 0, "seed {seed:#x}");
                assert_eq!(values.len(), frame.payload.len() / 6, "seed {seed:#x}");
                if ack {
                    assert!(
                        values.is_empty(),
                        "seed {seed:#x}: SETTINGS ACK with values"
                    );
                }
            }
            Parsed::Ping { .. } => {
                assert_eq!(frame.payload.len(), 8, "seed {seed:#x}");
                assert_eq!(frame.stream_id, 0, "seed {seed:#x}");
            }
            Parsed::WindowUpdate { increment, .. } => {
                assert_eq!(frame.payload.len(), 4, "seed {seed:#x}");
                assert!(increment <= 0x7fff_ffff, "seed {seed:#x}");
            }
            Parsed::RstStream { stream_id, .. } => {
                assert_ne!(stream_id, 0, "seed {seed:#x}");
                assert_eq!(frame.payload.len(), 4, "seed {seed:#x}");
            }
            Parsed::GoAway { .. } => {
                assert_eq!(frame.stream_id, 0, "seed {seed:#x}");
                assert!(frame.payload.len() >= 8, "seed {seed:#x}");
            }
            Parsed::Priority | Parsed::PushPromise | Parsed::Unknown => {}
        }
    });
    assert!(n > 100);
}

#[test]
fn padding_never_reads_past_the_payload() {
    for_seeds(0x4832_5041_4444_0002, |seed| {
        let mut rng = Rng::new(seed);
        let kind = if rng.chance(2) {
            kind::DATA
        } else {
            kind::HEADERS
        };
        let len = rng.below(12);
        let mut payload: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        if let Some(first) = payload.first_mut() {
            *first = rng.below(16) as u8;
        }
        let mut flags = flag::PADDED;
        if kind == kind::HEADERS && rng.chance(2) {
            flags |= flag::PRIORITY;
        }
        let frame = Frame::new(kind, flags, 1, payload.clone());
        let result = parse_frame(&frame);
        let pad = payload.first().copied().map(usize::from);
        let prio = if flags & flag::PRIORITY != 0 { 5 } else { 0 };
        match pad {
            None => assert!(result.is_err(), "seed {seed:#x}: empty padded frame"),
            Some(pad) if pad + prio + 1 > payload.len() => {
                assert!(result.is_err(), "seed {seed:#x}: pad {pad} past {len}")
            }
            Some(_) => assert!(result.is_ok(), "seed {seed:#x}: good padding refused"),
        }
    });
}

#[test]
fn frame_reader_reassembles_any_split() {
    for_seeds(0x4832_5245_4144_0003, |seed| {
        let mut rng = Rng::new(seed);
        let frames: Vec<Frame> = (0..1 + rng.below(6))
            .map(|_| {
                let len = rng.below(300);
                Frame::new(
                    rng.below(10) as u8,
                    rng.byte(),
                    rng.next() as u32 & 0x7fff_ffff,
                    (0..len).map(|_| rng.byte()).collect(),
                )
            })
            .collect();
        let wire: Vec<u8> = frames.iter().flat_map(Frame::to_bytes).collect();
        let mut reader = FrameReader::new();
        let mut got = Vec::new();
        let mut pos = 0;
        while pos < wire.len() {
            let step = (1 + rng.below(40)).min(wire.len() - pos);
            reader.push(&wire[pos..pos + step]);
            pos += step;
            while let Some(frame) = reader.next_frame().expect("valid frames") {
                got.push(frame);
            }
        }
        assert_eq!(got.len(), frames.len(), "seed {seed:#x}");
        for (a, b) in got.iter().zip(&frames) {
            assert_eq!(a.to_bytes(), b.to_bytes(), "seed {seed:#x}");
        }
    });
}

#[test]
fn oversize_frame_headers_are_refused_before_buffering() {
    for_seeds(0x4832_4f56_4552_0004, |seed| {
        let mut rng = Rng::new(seed);
        let len = MAX_FRAME_SIZE + 1 + rng.below((1 << 24) - MAX_FRAME_SIZE - 1);
        let mut head = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8];
        head.push(rng.below(10) as u8);
        head.push(rng.byte());
        head.extend_from_slice(&1u32.to_be_bytes());
        let mut reader = FrameReader::new();
        reader.push(&head);
        assert!(
            reader.next_frame().is_err(),
            "seed {seed:#x}: {len}-byte frame accepted"
        );
    });
}

#[test]
fn good_replies_in_any_split_give_the_exact_body() {
    for_seeds(0x4832_474f_4f44_0005, |seed| {
        let mut rng = Rng::new(seed);
        let max = if rng.chance(8) { 40_000 } else { 600 };
        let body = rng.bytes(max);
        let reply = good_reply(&mut rng, &body);
        let piece = 1 + rng.below(3000);
        let req_body = if rng.chance(3) {
            rng.bytes(2000)
        } else {
            Vec::new()
        };
        let (status, headers, got) = exchange(seed, reply, piece, req_body)
            .unwrap_or_else(|err| panic!("seed {seed:#x}: good reply refused: {err}"));
        assert_eq!(status, 200, "seed {seed:#x}");
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "content-type" && v == "text/plain")
        );
        assert_eq!(got, body, "seed {seed:#x}");
    });
}

#[test]
fn damaged_and_random_streams_end_without_panic_or_hang() {
    for_seeds(0x4832_4d55_5441_0006, |seed| {
        let mut rng = Rng::new(seed);
        let body = rng.bytes(300);
        let mut wire = match rng.below(3) {
            0 => good_reply(&mut rng, &body),
            1 => {
                let count = rng.below(12);
                let mut w = Vec::new();
                for _ in 0..count {
                    w.extend(random_frame(&mut rng).to_bytes());
                }
                w
            }
            _ => {
                let mut w = good_reply(&mut rng, &body);
                let at = rng.below(w.len());
                let extra: Vec<u8> = random_frame(&mut rng).to_bytes();
                w.splice(at..at, extra);
                w
            }
        };
        for _ in 0..rng.below(6) {
            if wire.is_empty() {
                break;
            }
            let at = rng.below(wire.len());
            match rng.below(4) {
                0 => wire[at] ^= 1 << rng.below(8),
                1 => wire[at] = rng.byte(),
                2 => wire.truncate(at),
                _ => {
                    wire.remove(at);
                }
            }
        }
        let piece = 1 + rng.below(100);
        if let Ok((status, _, got)) = exchange(seed, wire, piece, Vec::new()) {
            assert!(
                (100..1000).contains(&status),
                "seed {seed:#x}: status {status}"
            );
            assert!(got.len() <= 64 * 1024, "seed {seed:#x}: body past the cap");
        }
    });
}

#[test]
fn continuation_floods_and_bad_hpack_are_refused() {
    let cases = for_seeds(0x4832_434f_4e54_0007, |seed| {
        let mut rng = Rng::new(seed);
        let mut wire = Frame::new(kind::SETTINGS, 0, 0, Vec::new()).to_bytes();
        match rng.below(3) {
            0 => {
                // A header block that never ends, past MAX_HEADER_BLOCK.
                wire.extend(Frame::new(kind::HEADERS, 0, 1, vec![0x88]).to_bytes());
                let mut total = 1;
                while total <= MAX_HEADER_BLOCK + MAX_FRAME_SIZE {
                    let n = 1 + rng.below(MAX_FRAME_SIZE);
                    wire.extend(Frame::new(kind::CONTINUATION, 0, 1, vec![0x40; n]).to_bytes());
                    total += n;
                }
            }
            1 => {
                // CONTINUATION on another stream, or with nothing open.
                if rng.chance(2) {
                    wire.extend(Frame::new(kind::HEADERS, 0, 1, vec![0x88]).to_bytes());
                }
                let sid = if rng.chance(2) { 3 } else { 0 };
                wire.extend(
                    Frame::new(kind::CONTINUATION, flag::END_HEADERS, sid, vec![0x88]).to_bytes(),
                );
            }
            _ => {
                // A complete header block whose HPACK bytes are damaged.
                let mut block = header_block(&[
                    Header::new(":status", "200"),
                    Header::new("x-a", &"z".repeat(rng.below(80))),
                ]);
                let at = rng.below(block.len());
                block[at] = match rng.below(3) {
                    0 => 0xff,
                    1 => 0x3f,
                    _ => rng.byte(),
                };
                block.extend((0..rng.below(8)).map(|_| 0xff));
                wire.extend(
                    Frame::new(
                        kind::HEADERS,
                        flag::END_HEADERS | flag::END_STREAM,
                        1,
                        block,
                    )
                    .to_bytes(),
                );
            }
        }
        if let Ok((status, headers, _)) = exchange(seed, wire, 4096, Vec::new()) {
            // Only damage that still decodes to a valid response may pass.
            assert!((100..1000).contains(&status), "seed {seed:#x}");
            for (name, value) in headers {
                assert!(
                    !name
                        .bytes()
                        .chain(value.bytes())
                        .any(|b| b == b'\r' || b == b'\n'),
                    "seed {seed:#x}: CR or LF in a header field"
                );
            }
        }
    });
    assert!(cases > 3);
}

#[test]
fn settings_extremes_never_break_the_exchange() {
    for_seeds(0x4832_5345_5454_0008, |seed| {
        let mut rng = Rng::new(seed);
        let mut payload = Vec::new();
        for _ in 0..1 + rng.below(8) {
            let id = rng.below(8) as u16;
            let value = match rng.below(5) {
                0 => 0,
                1 => u32::MAX,
                2 => 0x7fff_ffff,
                3 => 0x8000_0000,
                _ => rng.next() as u32,
            };
            payload.extend_from_slice(&id.to_be_bytes());
            payload.extend_from_slice(&value.to_be_bytes());
        }
        let body = rng.bytes(200);
        let mut wire = Frame::new(kind::SETTINGS, 0, 0, payload).to_bytes();
        let rest = good_reply(&mut rng, &body);
        wire.extend(rest);
        let upload = rng.bytes(70_000);
        let _ = exchange(seed, wire, 1 + rng.below(512), upload);
    });
}

#[test]
fn header_fields_with_cr_lf_or_nul_are_malformed() {
    // RFC 9113 section 8.2.1: a field value with CR, LF, or NUL, or a field
    // name with uppercase or those bytes, makes the response malformed.
    for_seeds(0x4832_4352_4c46_0009, |seed| {
        let mut rng = Rng::new(seed);
        let bad = [b'\r', b'\n', 0u8][rng.below(3)];
        let mut text = b"ok".to_vec();
        text.insert(rng.below(3), bad);
        let text = String::from_utf8(text).unwrap();
        let header = if rng.chance(2) {
            Header::new("x-a", &text)
        } else {
            Header::new(&format!("x{text}"), "v")
        };
        let block = header_block(&[Header::new(":status", "200"), header]);
        let mut wire = Frame::new(kind::SETTINGS, 0, 0, Vec::new()).to_bytes();
        wire.extend(
            Frame::new(
                kind::HEADERS,
                flag::END_HEADERS | flag::END_STREAM,
                1,
                block,
            )
            .to_bytes(),
        );
        assert!(
            exchange(seed, wire, 4096, Vec::new()).is_err(),
            "seed {seed:#x}: header with {bad:#x} accepted"
        );
    });
}

/// One HEADERS reply that ends the stream, after an empty SETTINGS.
fn reply_with(field: Header) -> Vec<u8> {
    let block = header_block(&[Header::new(":status", "200"), field]);
    let mut wire = Frame::new(kind::SETTINGS, 0, 0, Vec::new()).to_bytes();
    wire.extend(
        Frame::new(
            kind::HEADERS,
            flag::END_HEADERS | flag::END_STREAM,
            1,
            block,
        )
        .to_bytes(),
    );
    wire
}

#[test]
fn a_space_in_a_name_is_malformed_but_spaces_in_values_are_fine() {
    for name in ["x a", " x", "x "] {
        assert!(
            exchange(1, reply_with(Header::new(name, "v")), 4096, Vec::new()).is_err(),
            "name {name:?} accepted"
        );
    }
    for value in ["\r", "a\nb", "a\0", "\r\n"] {
        assert!(
            exchange(2, reply_with(Header::new("x-a", value)), 4096, Vec::new()).is_err(),
            "value {value:?} accepted"
        );
    }
    let (status, headers, _) = exchange(
        3,
        reply_with(Header::new("x-a", "two words\tand a tab")),
        4096,
        Vec::new(),
    )
    .expect("a plain value is fine");
    assert_eq!(status, 200);
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "x-a" && v == "two words\tand a tab")
    );
}
