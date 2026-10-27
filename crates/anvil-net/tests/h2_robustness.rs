// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Seeded robustness runs for the HTTP/2 client: random and damaged server
//! bytes must end in a response or an error, never a panic or a hang.
//!
//! Set `ANVIL_ROBUSTNESS_CASES` for a longer run.

use anvil_hpack::{Encoder, Header};
use anvil_net::NetError;
use anvil_net::h2::{self, Frame, FrameReader, Request, Transport, flag, kind, parse_frame};

/// Small seeded generator so a failing case can be replayed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let len = self.below(max + 1);
        (0..len).map(|_| self.byte()).collect()
    }
}

fn cases() -> usize {
    std::env::var("ANVIL_ROBUSTNESS_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000)
}

struct Script {
    input: Vec<u8>,
    pos: usize,
    piece: usize,
}

impl Transport for Script {
    fn send(&mut self, _data: &[u8]) -> Result<(), NetError> {
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        let n = (self.input.len() - self.pos).min(buf.len()).min(self.piece);
        buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn request(rng: &mut Rng) -> Request {
    Request {
        method: if rng.below(2) == 0 { "GET" } else { "POST" }.into(),
        scheme: "https".into(),
        authority: "example.test".into(),
        path: "/".into(),
        headers: vec![("accept".into(), "*/*".into())],
        body: rng.bytes(3000),
    }
}

/// Run one exchange against `input`. Returns whether a response came back.
fn run(rng: &mut Rng, input: Vec<u8>) -> bool {
    let mut io = Script {
        input,
        pos: 0,
        piece: 1 + rng.below(64),
    };
    let req = request(rng);
    h2::exchange(&mut io, &req, 64 * 1024).is_ok()
}

/// A plausible server reply: SETTINGS, response headers, a few DATA frames.
fn valid_reply(rng: &mut Rng) -> Vec<u8> {
    let mut out = Frame::new(kind::SETTINGS, 0, 0, vec![0, 3, 0, 0, 0, 100]).to_bytes();
    let mut enc = Encoder::new();
    let mut head = Vec::new();
    enc.encode(
        &[
            Header::new(":status", "200"),
            Header::new("content-type", "text/html"),
            Header::new("x-n", &rng.below(1000).to_string()),
        ],
        &mut head,
    );
    out.extend(Frame::new(kind::HEADERS, flag::END_HEADERS, 1, head).to_bytes());
    let frames = rng.below(4);
    for i in 0..=frames {
        let flags = if i == frames { flag::END_STREAM } else { 0 };
        out.extend(Frame::new(kind::DATA, flags, 1, rng.bytes(200)).to_bytes());
    }
    out
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for _ in 0..cases() {
        let input = rng.bytes(400);
        run(&mut rng, input);
    }
}

#[test]
fn random_frames_never_panic() {
    let mut rng = Rng(0x0bad_cafe_f00d_0001);
    for _ in 0..cases() {
        let mut input = Vec::new();
        for _ in 0..rng.below(12) {
            let kind = if rng.below(8) == 0 {
                rng.byte()
            } else {
                rng.below(10) as u8
            };
            let flags = rng.byte();
            let stream = if rng.below(3) == 0 {
                rng.next() as u32
            } else {
                rng.below(4) as u32
            };
            Frame::new(kind, flags, stream, rng.bytes(40)).write_to(&mut input);
        }
        run(&mut rng, input);
    }
}

#[test]
fn damaged_replies_never_panic() {
    let mut rng = Rng(0x5eed_0000_0000_0042);
    let mut completed = 0;
    for _ in 0..cases() {
        let mut input = valid_reply(&mut rng);
        match rng.below(4) {
            0 => {
                let n = rng.below(input.len());
                input.truncate(n);
            }
            1 => {
                for _ in 0..1 + rng.below(4) {
                    let i = rng.below(input.len());
                    input[i] ^= 1 << rng.below(8);
                }
            }
            2 => {
                let i = rng.below(input.len());
                let extra = rng.bytes(20);
                input.splice(i..i, extra);
            }
            _ => {}
        }
        if run(&mut rng, input) {
            completed += 1;
        }
    }
    // The untouched quarter, and some harmless damage, still succeed.
    assert!(
        completed > cases() / 5,
        "only {completed} replies completed"
    );
}

#[test]
fn frame_parsing_never_panics() {
    let mut rng = Rng(0x00f0_0f00_f00f_0ff0);
    for _ in 0..cases() * 5 {
        let frame = Frame::new(
            rng.byte() % 12,
            rng.byte(),
            rng.next() as u32,
            rng.bytes(30),
        );
        let _ = parse_frame(&frame);
        let mut reader = FrameReader::new();
        reader.push(&rng.bytes(60));
        while let Ok(Some(frame)) = reader.next_frame() {
            let _ = parse_frame(&frame);
        }
    }
}
