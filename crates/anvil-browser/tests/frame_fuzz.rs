// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Fuzz-style checks for display list frames.
//!
//! A seeded generator paints random display lists, some with numbers no page
//! should have, and builds frames from them. Every built frame must pass the
//! checker and round-trip exactly. Then the encoded bytes are broken in many
//! small ways: the decoder must never panic, and anything it accepts must
//! encode back to the same bytes and pass the checker.
//!
//! The runs are short so `cargo test` stays quick. Set
//! `ANVIL_FRAME_FUZZ_ROUNDS` to run longer, and `ANVIL_FRAME_FUZZ_SEED` to
//! replay one seed.

use anvil_browser::frame::{self, Frame, LinkMove, LinkRegion, MAX_PAGE_HEIGHT};
use anvil_image::Image;
use anvil_layout::{Color, Edge};
use anvil_paint::{
    DisplayItem, DisplayList, Layer, LayerEffect, Rect, StickyConstraint, Transform2D,
};

/// Small xorshift generator, so a failing seed can be replayed.
struct Rng(u64);

impl Rng {
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

    /// Mostly ordinary numbers, sometimes ones the checker must catch.
    fn num(&mut self, scale: f32) -> f32 {
        match self.below(40) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -scale * 1e6,
            3 => scale * 1e6,
            4 => -0.0,
            _ => (self.below(20_000) as f32 / 10_000.0 - 0.2) * scale,
        }
    }

    fn color(&mut self) -> Color {
        let v = self.next().to_le_bytes();
        Color {
            r: v[0],
            g: v[1],
            b: v[2],
            a: v[3],
        }
    }

    fn text(&mut self) -> String {
        const PIECES: &[&str] = &["a", "Z", " ", "<", "é", "漢", "\u{1F600}", "\n", "\0"];
        (0..self.below(12))
            .map(|_| PIECES[self.below(PIECES.len())])
            .collect()
    }

    fn rect(&mut self) -> Rect {
        Rect {
            x: self.num(1000.0),
            y: self.num(3000.0),
            width: self.num(800.0),
            height: self.num(400.0),
        }
    }

    fn item(&mut self) -> DisplayItem {
        let (x, y, width, height) = (
            self.num(1000.0),
            self.num(3000.0),
            self.num(800.0),
            self.num(400.0),
        );
        match self.below(10) {
            0 => DisplayItem::PushClip {
                x,
                y,
                width,
                height,
            },
            1 => DisplayItem::PopClip,
            2 => DisplayItem::PushFixed,
            3 => DisplayItem::PopFixed,
            4 => DisplayItem::BoxShadow {
                x,
                y,
                width,
                height,
                offset_x: self.num(20.0),
                offset_y: self.num(20.0),
                blur_radius: self.num(40.0),
                spread_radius: self.num(10.0),
                radius: self.num(30.0),
                color: self.color(),
            },
            5 => DisplayItem::LinearGradientRect {
                x,
                y,
                width,
                height,
                radius: self.num(30.0),
                border_width: self.num(5.0),
                angle_deg: self.num(360.0),
                start_color: self.color(),
                end_color: self.color(),
                border_color: self.color(),
            },
            6 => DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color: self.color(),
            },
            7 => DisplayItem::RoundedRect {
                x,
                y,
                width,
                height,
                radius: self.num(30.0),
                border_width: self.num(5.0),
                fill_color: self.color(),
                border_color: self.color(),
            },
            8 => {
                let (w, h) = (1 + self.below(6) as u32, 1 + self.below(6) as u32);
                let mut image = Image::new(w, h);
                if self.chance(20) {
                    image.pixels.pop();
                }
                for p in &mut image.pixels {
                    *p = self.next() as u8;
                }
                DisplayItem::Image {
                    x,
                    y,
                    width,
                    height,
                    border_radius: self.num(10.0),
                    image,
                }
            }
            _ => DisplayItem::Text {
                x,
                y,
                text: self.text(),
                color: self.color(),
                font_size: self.num(40.0),
            },
        }
    }

    fn effect(&mut self) -> LayerEffect {
        match self.below(3) {
            0 => LayerEffect::Sticky(StickyConstraint {
                rect: self.rect(),
                margin: Edge {
                    top: self.num(10.0),
                    right: self.num(10.0),
                    bottom: self.num(10.0),
                    left: self.num(10.0),
                },
                containing: self.rect(),
                top: self.chance(2).then(|| self.num(50.0)),
                right: self.chance(2).then(|| self.num(50.0)),
                bottom: self.chance(2).then(|| self.num(50.0)),
                left: self.chance(2).then(|| self.num(50.0)),
            }),
            1 => LayerEffect::Transform(Transform2D {
                a: self.num(3.0),
                b: self.num(1.0),
                c: self.num(1.0),
                d: self.num(3.0),
                e: self.num(500.0),
                f: self.num(500.0),
            }),
            _ => LayerEffect::Opacity(self.num(1.5)),
        }
    }

    fn list(&mut self) -> DisplayList {
        let items: Vec<_> = (0..self.below(40)).map(|_| self.item()).collect();
        let mut layers = Vec::new();
        let mut start = 0;
        for _ in 0..self.below(6) {
            start += self.below(4);
            let end = start + self.below(8);
            layers.push(Layer {
                start,
                end,
                effect: self.effect(),
            });
        }
        DisplayList { items, layers }
    }

    fn links(&mut self) -> Vec<LinkRegion> {
        const URLS: &[&str] = &[
            "https://a.test/",
            "http://b.test/x?y=1",
            "anvil://settings",
            "file:///etc/hosts",
            "https://a b.test/",
        ];
        (0..self.below(5))
            .map(|_| LinkRegion {
                rect: self.rect(),
                url: URLS[self.below(URLS.len())].to_string(),
                moves: (0..self.below(4)).map(|_| self.link_move()).collect(),
            })
            .collect()
    }

    fn link_move(&mut self) -> LinkMove {
        match self.effect() {
            LayerEffect::Sticky(c) => LinkMove::Sticky(c),
            LayerEffect::Transform(m) => LinkMove::Transform(m),
            LayerEffect::Opacity(_) => LinkMove::Fixed,
        }
    }
}

fn seed() -> u64 {
    std::env::var("ANVIL_FRAME_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_f4a3_0000_0001)
}

fn rounds(default: usize) -> usize {
    std::env::var("ANVIL_FRAME_FUZZ_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn random_frame(rng: &mut Rng) -> Frame {
    let list = rng.list();
    let height = if rng.chance(10) {
        MAX_PAGE_HEIGHT * 2.0
    } else {
        rng.num(5000.0)
    };
    Frame::build(&list, rng.num(1200.0), height, &rng.links())
}

#[test]
fn built_frames_pass_and_round_trip() {
    let base = seed();
    for round in 0..rounds(2000) {
        let seed = base.wrapping_add(round as u64).max(1);
        let mut rng = Rng(seed);
        let frame = random_frame(&mut rng);
        assert_eq!(frame::check(&frame), Ok(()), "seed {seed}");
        let bytes = frame::encode(&frame).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let back = frame::decode(&bytes).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(back, frame, "seed {seed}");
        assert_eq!(frame::encode(&back).unwrap(), bytes, "seed {seed}");
    }
}

#[test]
fn broken_frames_never_panic_and_never_change_form() {
    let base = seed() ^ 0x0bad;
    for round in 0..rounds(3000) {
        let seed = base.wrapping_add(round as u64).max(1);
        let mut rng = Rng(seed);
        let mut bytes = frame::encode(&random_frame(&mut rng)).unwrap();
        for _ in 0..1 + rng.below(4) {
            match rng.below(6) {
                0 if !bytes.is_empty() => {
                    let at = rng.below(bytes.len());
                    bytes[at] ^= 1 << rng.below(8);
                }
                1 if !bytes.is_empty() => {
                    let at = rng.below(bytes.len());
                    bytes[at] = rng.next() as u8;
                }
                2 => {
                    let at = rng.below(bytes.len() + 1);
                    bytes.truncate(at);
                }
                3 => {
                    let at = rng.below(bytes.len() + 1);
                    bytes.insert(at, rng.next() as u8);
                }
                4 if bytes.len() >= 4 => {
                    // Overwrite four bytes, often a count or a length.
                    let at = rng.below(bytes.len() - 3);
                    let v = match rng.below(3) {
                        0 => u32::MAX,
                        1 => rng.below(1 << 20) as u32,
                        _ => f32::NAN.to_bits(),
                    };
                    bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
                _ => {
                    let at = rng.below(bytes.len() + 1);
                    bytes.splice(at..at, [0u8; 4]);
                }
            }
        }
        if let Ok(frame) = frame::decode(&bytes) {
            assert_eq!(frame::check(&frame), Ok(()), "seed {seed}");
            assert_eq!(frame::encode(&frame).unwrap(), bytes, "seed {seed}");
        }
    }
}
