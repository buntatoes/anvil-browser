// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Fuzz-style checks for the renderer wire format.
//!
//! A seeded random generator builds valid messages, breaks them in many small
//! ways, and feeds the results to the decoder. The decoder must never panic,
//! and anything it accepts must encode back to exactly the same bytes.
//!
//! The runs are short so `cargo test` stays quick. Set `ANVIL_WIRE_FUZZ_ROUNDS`
//! to run longer, and `ANVIL_WIRE_FUZZ_SEED` to replay one seed.

use std::io::Cursor;

use anvil_sandbox::wire::{
    self, Direction, FailureCode, FetchKind, FetchReply, FetchRequest, HEADER_BYTES, InteractOp,
    InteractOutcome, InteractRequest, Message, MessageKind, RenderFailure, RenderRequest,
    RenderedDocument, RenderedFrame, RendererStatus, ResizeRequest, Role, WIRE_VERSION, WireError,
    WsCommandOp, WsReplyBody, WsWireCommand, WsWireEvent, WsWireReply,
};

/// Small xorshift generator, so a failing seed can be replayed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
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
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    fn text(&mut self, max: usize) -> String {
        const PIECES: &[&str] = &[
            "a",
            "Z",
            "0",
            " ",
            "<",
            ">",
            "&",
            "\"",
            "é",
            "漢",
            "\u{10000}",
            "\n",
            "\0",
            "-",
            ".",
        ];
        let len = self.below(max + 1);
        let mut out = String::new();
        while out.len() < len {
            out.push_str(PIECES[self.below(PIECES.len())]);
        }
        out
    }

    fn host(&mut self) -> String {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789-.";
        let len = 1 + self.below(30);
        (0..len)
            .map(|_| CHARS[self.below(CHARS.len())] as char)
            .collect()
    }

    fn url(&mut self) -> String {
        const CHARS: &[u8] = b"abcxyz0189/?=&%#.-_~";
        let scheme = if self.chance(2) {
            "https://"
        } else {
            "http://"
        };
        let len = self.below(60);
        let tail: String = (0..len)
            .map(|_| CHARS[self.below(CHARS.len())] as char)
            .collect();
        format!("{scheme}{}{tail}", self.host())
    }
}

fn seed() -> u64 {
    std::env::var("ANVIL_WIRE_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_a11e_0000_0001)
}

fn rounds(default: usize) -> usize {
    std::env::var("ANVIL_WIRE_FUZZ_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// A random valid message and the direction it travels in.
fn random_message(rng: &mut Rng) -> (Direction, Message) {
    match rng.below(18) {
        0 => (
            Direction::ToRenderer,
            Message::Hello {
                role: Role::Browser,
                version: WIRE_VERSION,
            },
        ),
        1 => (
            Direction::ToBrowser,
            Message::Hello {
                role: Role::Renderer,
                version: rng.next() as u16,
            },
        ),
        2 => (Direction::ToRenderer, Message::Ping { nonce: rng.next() }),
        3 => (Direction::ToBrowser, Message::Pong { nonce: rng.next() }),
        4 => {
            let hosts = (0..rng.below(5)).map(|_| rng.host()).collect();
            (
                Direction::ToRenderer,
                Message::Render(RenderRequest {
                    tab_id: rng.next(),
                    nav_id: rng.next(),
                    url: rng.url(),
                    html: rng.text(400),
                    csp_header: if rng.chance(2) {
                        Some(rng.text(40))
                    } else {
                        None
                    },
                    scripts_enabled: rng.chance(2),
                    viewport_width: 1 + rng.below(16_384) as u32,
                    viewport_height: 1 + rng.below(16_384) as u32,
                    https_only: rng.chance(2),
                    http_allowed_hosts: hosts,
                    extended_script_reach: rng.chance(2),
                }),
            )
        }
        5 => {
            let has_next_timer = rng.chance(2);
            (
                Direction::ToBrowser,
                Message::Rendered(RenderedDocument {
                    tab_id: rng.next(),
                    nav_id: rng.next(),
                    title: rng.text(50),
                    dom_nodes: rng.below(1_000_000) as u32,
                    blocked_resources: rng.next() as u32,
                    html: rng.text(400),
                    final_url: if rng.chance(2) {
                        rng.url()
                    } else {
                        String::new()
                    },
                    animating: rng.chance(2),
                    has_next_timer,
                    next_timer_ms_bits: if has_next_timer { rng.next() } else { 0 },
                    spa_back_depth: rng.below(64) as u32,
                    spa_forward_depth: rng.below(64) as u32,
                }),
            )
        }
        6 => (
            Direction::ToBrowser,
            Message::Failed(RenderFailure {
                tab_id: rng.next(),
                nav_id: rng.next(),
                code: [
                    FailureCode::Limit,
                    FailureCode::TooLarge,
                    FailureCode::Internal,
                ][rng.below(3)],
                detail: rng.text(60),
            }),
        ),
        7 => (
            Direction::ToBrowser,
            Message::Fetch(FetchRequest {
                nav_id: rng.next(),
                kind: [
                    FetchKind::Stylesheet,
                    FetchKind::Script,
                    FetchKind::Image,
                    FetchKind::Font,
                    FetchKind::ScriptFetch,
                ][rng.below(5)],
                url: rng.url(),
            }),
        ),
        8 => (
            Direction::ToRenderer,
            Message::FetchReply(FetchReply {
                nav_id: rng.next(),
                status: if rng.chance(4) {
                    0
                } else {
                    100 + rng.below(500) as u16
                },
                content_type: rng.text(20),
                body: (0..rng.below(300)).map(|_| rng.next() as u8).collect(),
            }),
        ),
        9 => (Direction::ToRenderer, Message::StatusQuery),
        10 => {
            let mut namespaces = [0u64; 7];
            for id in &mut namespaces {
                *id = rng.next();
            }
            let mut limits = [None; 5];
            for limit in &mut limits {
                if rng.chance(2) {
                    *limit = Some(rng.next());
                }
            }
            (
                Direction::ToBrowser,
                Message::Status(RendererStatus {
                    no_new_privs: [Some(false), Some(true), None][rng.below(3)],
                    seccomp: [Some(0), Some(1), Some(2), None][rng.below(4)],
                    namespaces,
                    uid_map: rng.text(30),
                    pid: rng.next() as u32,
                    visible_processes: rng.next() as u32,
                    root_entries: rng.text(80),
                    writable_mounts: rng.text(80),
                    limits,
                }),
            )
        }
        11 => (
            Direction::ToBrowser,
            Message::Frame(RenderedFrame {
                tab_id: rng.next(),
                nav_id: rng.next(),
                title: rng.text(50),
                dom_nodes: rng.below(1_000_000) as u32,
                blocked_resources: rng.next() as u32,
                display_list: (0..rng.below(300)).map(|_| rng.next() as u8).collect(),
            }),
        ),
        12 => (
            Direction::ToRenderer,
            Message::Resize(ResizeRequest {
                tab_id: rng.next(),
                nav_id: rng.next(),
                viewport_width: 1 + rng.below(16_384) as u32,
                viewport_height: 1 + rng.below(16_384) as u32,
            }),
        ),
        13 => (
            Direction::ToRenderer,
            Message::Interact(InteractRequest {
                tab_id: rng.next(),
                nav_id: rng.next(),
                op: [
                    InteractOp::Click,
                    InteractOp::Scroll,
                    InteractOp::InputChar,
                    InteractOp::KeyDown,
                    InteractOp::TimerStep,
                    InteractOp::FocusBlur,
                    InteractOp::KeyText,
                    InteractOp::HistoryGo,
                    InteractOp::ViewportScroll,
                    InteractOp::InputAction,
                    InteractOp::ResizeEvent,
                ][rng.below(11)],
                x_bits: rng.next() as u32,
                y_bits: rng.next() as u32,
                delta_bits: rng.next() as u32,
                now_ms_bits: rng.next(),
                viewport_width: 1 + rng.below(16_384) as u32,
                viewport_height: 1 + rng.below(16_384) as u32,
                modifiers: rng.next() as u8,
                text: rng.text(40),
            }),
        ),
        14 => {
            let has_scroll_to = rng.chance(2);
            let has_next_timer = rng.chance(2);
            (
                Direction::ToBrowser,
                Message::InteractResult(InteractOutcome {
                    tab_id: rng.next(),
                    nav_id: rng.next(),
                    changed: rng.chance(2),
                    handled_by_script: rng.chance(2),
                    animating: rng.chance(2),
                    focus_input: rng.chance(2),
                    title: rng.text(30),
                    navigate_url: if rng.chance(3) { Some(rng.url()) } else { None },
                    post_url: if rng.chance(3) { Some(rng.url()) } else { None },
                    post_content_type: rng.text(20),
                    post_body: (0..rng.below(60)).map(|_| rng.next() as u8).collect(),
                    scroll_to_bits: if has_scroll_to { rng.next() as u32 } else { 0 },
                    has_scroll_to,
                    next_timer_ms_bits: if has_next_timer { rng.next() } else { 0 },
                    has_next_timer,
                    current_url: if rng.chance(2) {
                        rng.url()
                    } else {
                        String::new()
                    },
                    spa_back_depth: rng.below(32) as u32,
                    spa_forward_depth: rng.below(32) as u32,
                    copied_text: if rng.chance(3) {
                        Some(rng.text(30))
                    } else {
                        None
                    },
                }),
            )
        }
        15 => {
            let op = match rng.below(5) {
                0 => WsCommandOp::Connect {
                    url: format!("wss://{}/ws", rng.host()),
                    protocols: (0..rng.below(3)).map(|_| rng.host()).collect(),
                },
                1 => WsCommandOp::SendText {
                    handle: rng.next(),
                    text: rng.text(60),
                },
                2 => WsCommandOp::SendBinary {
                    handle: rng.next(),
                    data: (0..rng.below(60)).map(|_| rng.next() as u8).collect(),
                },
                3 => WsCommandOp::Close {
                    handle: rng.next(),
                    code: 1000,
                    reason: rng.text(20),
                },
                _ => WsCommandOp::Poll { handle: rng.next() },
            };
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: rng.next(),
                    op,
                }),
            )
        }
        16 => {
            let body = match rng.below(3) {
                0 => WsReplyBody::Connected {
                    ok: rng.chance(2),
                    handle: rng.next(),
                    protocol: rng.host(),
                    resolved_url: format!("wss://{}/ws", rng.host()),
                    error: rng.text(20),
                },
                1 => WsReplyBody::Ack {
                    ok: rng.chance(2),
                    error: rng.text(20),
                },
                _ => WsReplyBody::Polled {
                    ready_state: rng.below(4) as u8,
                    events: (0..rng.below(4))
                        .map(|_| match rng.below(4) {
                            0 => WsWireEvent::Text(rng.text(20)),
                            1 => WsWireEvent::Binary(vec![1, 2, 3]),
                            2 => WsWireEvent::Closed {
                                code: 1000,
                                reason: rng.text(10),
                            },
                            _ => WsWireEvent::Error(rng.text(10)),
                        })
                        .collect(),
                },
            };
            (
                Direction::ToRenderer,
                Message::WsReply(WsWireReply {
                    nav_id: rng.next(),
                    body,
                }),
            )
        }
        _ => (Direction::ToRenderer, Message::Shutdown),
    }
}

/// Break `frame` in one of several ways.
fn mutate(rng: &mut Rng, frame: &mut Vec<u8>) {
    match rng.below(9) {
        // Flip a few bits anywhere.
        0 => {
            for _ in 0..1 + rng.below(4) {
                if !frame.is_empty() {
                    let at = rng.below(frame.len());
                    frame[at] ^= 1 << rng.below(8);
                }
            }
        }
        // Overwrite a byte with a value that tends to matter.
        1 => {
            if !frame.is_empty() {
                let at = rng.below(frame.len());
                frame[at] = [0x00, 0x01, 0x02, 0x7f, 0x80, 0xfe, 0xff][rng.below(7)];
            }
        }
        // Cut it short.
        2 => {
            let keep = rng.below(frame.len() + 1);
            frame.truncate(keep);
        }
        // Add bytes at the end.
        3 => {
            for _ in 0..1 + rng.below(8) {
                frame.push(rng.next() as u8);
            }
        }
        // Change the declared body length.
        4 => {
            if frame.len() >= HEADER_BYTES {
                let len = match rng.below(4) {
                    0 => u32::MAX,
                    1 => rng.next() as u32,
                    2 => (frame.len() - HEADER_BYTES) as u32 + 1,
                    _ => (frame.len() - HEADER_BYTES).saturating_sub(1) as u32,
                };
                frame[12..16].copy_from_slice(&len.to_le_bytes());
            }
        }
        // Change a length field inside the body: any four bytes after the
        // header, read as a little-endian length.
        5 => {
            if frame.len() >= HEADER_BYTES + 4 {
                let at = HEADER_BYTES + rng.below(frame.len() - HEADER_BYTES - 3);
                let len = [0u32, 1, 255, 0x7fff_ffff, u32::MAX, rng.next() as u32][rng.below(6)];
                frame[at..at + 4].copy_from_slice(&len.to_le_bytes());
            }
        }
        // Change the kind byte.
        6 => {
            if frame.len() > 6 {
                frame[6] = rng.next() as u8;
            }
        }
        // Drop a byte from the middle.
        7 => {
            if !frame.is_empty() {
                let at = rng.below(frame.len());
                frame.remove(at);
            }
        }
        // Random bytes behind a real header.
        _ => {
            frame.truncate(HEADER_BYTES.min(frame.len()));
            for _ in 0..rng.below(64) {
                frame.push(rng.next() as u8);
            }
        }
    }
}

/// Decode in both directions. Whatever is accepted must encode back to the
/// same bytes, and the reader must agree with the slice decoder.
fn check(frame: &[u8]) -> usize {
    let mut accepted = 0;
    for direction in [Direction::ToRenderer, Direction::ToBrowser] {
        let decoded = wire::decode(frame, direction);
        if let Ok((seq, message)) = &decoded {
            accepted += 1;
            assert!(message.kind().allowed(direction));
            let again = wire::encode(*seq, message).expect("an accepted message encodes");
            assert_eq!(again, frame, "decode then encode changed the bytes");
        }
        let mut reader = Cursor::new(frame.to_vec());
        let read = wire::read_message(&mut reader, direction);
        match (&decoded, &read) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a, b);
                assert_eq!(reader.position() as usize, frame.len());
            }
            // The reader stops at the end of its frame, so spare bytes after
            // it are not its business.
            (Err(WireError::TrailingBytes), _) => {}
            (Ok(_), Err(err)) => panic!("reader refused what decode accepted: {err}"),
            (Err(_), _) => {}
        }
    }
    accepted
}

#[test]
fn valid_messages_round_trip() {
    let mut rng = Rng::new(seed());
    for _ in 0..rounds(5_000) {
        let (direction, message) = random_message(&mut rng);
        let seq = rng.next() as u32;
        let frame = wire::encode(seq, &message).expect("valid message encodes");
        let (back_seq, back) = wire::decode(&frame, direction).expect("valid frame decodes");
        assert_eq!(back_seq, seq);
        assert_eq!(back, message);
    }
}

#[test]
fn broken_frames_never_panic_and_never_decode_loosely() {
    let mut rng = Rng::new(seed() ^ 0x9e37_79b9_7f4a_7c15);
    let mut accepted = 0usize;
    let total = rounds(40_000);
    for _ in 0..total {
        let (_, message) = random_message(&mut rng);
        let mut frame = wire::encode(rng.next() as u32, &message).unwrap();
        for _ in 0..1 + rng.below(3) {
            mutate(&mut rng, &mut frame);
        }
        accepted += check(&frame);
    }
    // Most broken frames must be refused. Some mutations land on free
    // values, like a byte inside a string, so a few still decode.
    assert!(
        accepted < total,
        "{accepted} of {total} broken frames decoded"
    );
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng::new(seed() ^ 0x0123_4567_89ab_cdef);
    for _ in 0..rounds(20_000) {
        let len = rng.below(80);
        let mut frame: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Half the time give it a real header so the body decoders get a turn.
        if rng.chance(2) && frame.len() >= HEADER_BYTES {
            frame[0..4].copy_from_slice(b"ANVR");
            frame[4..6].copy_from_slice(&WIRE_VERSION.to_le_bytes());
            frame[6] = 1 + rng.below(13) as u8;
            frame[7] = 0;
            let body = (frame.len() - HEADER_BYTES) as u32;
            frame[12..16].copy_from_slice(&body.to_le_bytes());
        }
        check(&frame);
    }
}

#[test]
fn a_stream_of_frames_reads_back_in_order_and_stops_at_the_first_bad_one() {
    let mut rng = Rng::new(seed() ^ 0x00ff_00ff_00ff_00ff);
    for _ in 0..rounds(500) {
        let mut stream = Vec::new();
        let mut sent = Vec::new();
        for seq in 0..1 + rng.below(6) as u32 {
            let message = loop {
                let (direction, message) = random_message(&mut rng);
                if direction == Direction::ToBrowser {
                    break message;
                }
            };
            stream.extend_from_slice(&wire::encode(seq, &message).unwrap());
            sent.push((seq, message));
        }
        let mut reader = Cursor::new(stream.clone());
        for expected in &sent {
            let got = wire::read_message(&mut reader, Direction::ToBrowser).unwrap();
            assert_eq!(&got, expected);
        }
        assert!(matches!(
            wire::read_message(&mut reader, Direction::ToBrowser),
            Err(WireError::Io(_))
        ));

        // One stray byte in the middle: every frame that ends before it
        // reads back unchanged, and nothing after it panics.
        let mut ends = Vec::new();
        let mut at = 0;
        for (seq, message) in &sent {
            at += wire::encode(*seq, message).unwrap().len();
            ends.push(at);
        }
        let cut = rng.below(stream.len());
        stream.insert(cut, 0xA5);
        let mut reader = Cursor::new(stream);
        let intact = ends.iter().filter(|end| **end <= cut).count();
        for expected in &sent[..intact] {
            let got = wire::read_message(&mut reader, Direction::ToBrowser).unwrap();
            assert_eq!(&got, expected);
        }
        while wire::read_message(&mut reader, Direction::ToBrowser).is_ok() {}
    }
}

#[test]
fn every_kind_has_a_small_header_cap_where_it_should() {
    for (kind, cap) in [
        (MessageKind::Hello, 3),
        (MessageKind::Ping, 8),
        (MessageKind::Pong, 8),
        (MessageKind::Shutdown, 0),
        (MessageKind::Resize, 24),
    ] {
        assert_eq!(kind.max_body(), cap);
    }
    assert!(MessageKind::Fetch.max_body() < 16 * 1024);
    assert!(MessageKind::Failed.max_body() < 2 * 1024);
}
