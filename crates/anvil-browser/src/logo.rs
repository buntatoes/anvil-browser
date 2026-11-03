// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Brand mark and the plain search badge drawn for settings.
//!
//! `lib.rs` calls `render_anvil_logo` and `render_search_engine_logo` while it
//! resolves `anvil://logo.png` and `anvil://engine-*.png`. `ui.rs` and `shell.rs`
//! call the Anvil mark for the toolbar and the window icon. `installer.rs` calls
//! `encode_anvil_png` and `encode_anvil_ico`.
//!
//! PNG and ICO bytes go through `anvil-image`. The circular mask uses
//! `4x4` SSAA distance sampling. The search badge is drawn here as pixels and is
//! the same for every `SearchEngine`. Search engine names are trademarks of their
//! owners and only show up as text, to say which engine is picked.

use std::sync::OnceLock;

use crate::SearchEngine;
use anvil_image::{Image, decode_png, encode_ico, encode_png};

/// Embedded 512x512 PNG bytes of the Anvil Browser brand mark (`assets/anvil-mark.png`).
pub const LOGO_PNG_BYTES: &[u8] = include_bytes!("../../../assets/anvil-mark.png");

/// Decode the embedded `512x512` `LOGO_PNG_BYTES` master asset once.
fn master_logo() -> &'static Image {
    static MASTER: OnceLock<Image> = OnceLock::new();
    MASTER.get_or_init(|| decode_png(LOGO_PNG_BYTES).expect("valid embedded assets/anvil-mark.png"))
}

/// Render the Anvil Browser logo as a 4x4 subpixel-supersampled (`SSAA`) RGBA [`Image`] of `size x size` pixels.
pub fn render_anvil_logo(size: u32) -> Image {
    let dim = size.clamp(12, 512);
    let mut img = master_logo().resize_bilinear(dim, dim);
    let w = dim as f32;
    let h = dim as f32;
    let hw = w * 0.5;
    let hh = h * 0.5;
    let radius = (w.min(h) * 0.5) - 0.5;
    let offsets = [-0.375_f32, -0.125, 0.125, 0.375];
    for py in 0..img.height {
        for px in 0..img.width {
            let mut cov_sum = 0.0_f32;
            for sy in offsets {
                let ry = (py as f32 + 0.5 + sy) - hh;
                for sx in offsets {
                    let rx = (px as f32 + 0.5 + sx) - hw;
                    let d = (rx * rx + ry * ry).sqrt() - radius;
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    cov_sum += cov;
                }
            }
            let alpha_mask = cov_sum * (1.0 / 16.0);
            if let Some(mut c) = img.pixel(px, py) {
                c[3] = ((c[3] as f32) * alpha_mask).round().clamp(0.0, 255.0) as u8;
                img.set_pixel(px, py, c);
            }
        }
    }
    img
}

/// Encode the Anvil Browser logo as a clean, metadata-free PNG image (`64x64`).
pub fn encode_anvil_png() -> Vec<u8> {
    encode_png(&render_anvil_logo(64))
}

/// Encode the Anvil Browser logo as a standard Windows `.ico` file containing a PNG-compressed 64x64 icon.
pub fn encode_anvil_ico() -> Vec<u8> {
    encode_ico(&render_anvil_logo(64))
}

/// Distance from `(px, py)` to the segment `(ax, ay)`-`(bx, by)`.
fn segment_distance(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq > 0.0 {
        (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (ax + dx * t, ay + dy * t);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// One sample of the plain search badge: an Anvil-orange disc with a white
/// magnifying glass. Every engine gets the same badge, and the page shows the
/// engine's name as text next to it.
fn sample_search_badge_subpixel(nx: f32, ny: f32) -> (f32, f32, f32) {
    const BADGE: (f32, f32, f32) = (255.0, 107.0, 36.0);
    const GLYPH: (f32, f32, f32) = (255.0, 250.0, 245.0);
    let lens_d = ((nx + 0.10).powi(2) + (ny + 0.10).powi(2)).sqrt();
    let in_lens = (lens_d - 0.30).abs() <= 0.08;
    let in_handle = segment_distance(nx, ny, 0.12, 0.12, 0.42, 0.42) <= 0.08;
    if in_lens || in_handle { GLYPH } else { BADGE }
}

/// Render the 4x4 subpixel-supersampled (`SSAA`) search badge for `engine` at `size x size` pixels.
///
/// Every engine gets the same plain Anvil badge. I don't draw other companies'
/// logos; the settings and start pages show each engine's name as text instead.
pub fn render_search_engine_logo(engine: SearchEngine, size: u32) -> Image {
    let _ = engine;
    let dim = size.clamp(16, 256);
    let mut img = Image::new(dim, dim);
    let w = dim as f32;
    let h = dim as f32;
    let cx = w * 0.5;
    let cy = h * 0.5;
    let outer_r = w * 0.46;
    let offsets = [-0.375_f32, -0.125, 0.125, 0.375];

    for py in 0..dim {
        for px in 0..dim {
            let mut r_acc = 0.0_f32;
            let mut g_acc = 0.0_f32;
            let mut b_acc = 0.0_f32;
            let mut a_acc = 0.0_f32;
            for sy in offsets {
                let y = (py as f32 + 0.5 + sy) - cy;
                let ny = y / outer_r;
                for sx in offsets {
                    let x = (px as f32 + 0.5 + sx) - cx;
                    let nx = x / outer_r;
                    let dist = (nx * nx + ny * ny).sqrt();
                    let sub_alpha = ((1.0 - dist) * outer_r).clamp(0.0, 1.0);
                    if sub_alpha > 0.0 {
                        let (sr, sg, sb) = sample_search_badge_subpixel(nx, ny);
                        r_acc += sr * sub_alpha;
                        g_acc += sg * sub_alpha;
                        b_acc += sb * sub_alpha;
                        a_acc += sub_alpha;
                    }
                }
            }
            if a_acc <= 0.0 {
                continue;
            }
            let inv_a = 1.0 / a_acc;
            let badge_alpha = (a_acc * (1.0 / 16.0)).clamp(0.0, 1.0);
            img.set_pixel(
                px,
                py,
                [
                    (r_acc * inv_a).round().clamp(0.0, 255.0) as u8,
                    (g_acc * inv_a).round().clamp(0.0, 255.0) as u8,
                    (b_acc * inv_a).round().clamp(0.0, 255.0) as u8,
                    (badge_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
                ],
            );
        }
    }
    img
}

#[cfg(test)]
mod tests {
    //! Checks the masked Anvil mark, the ICO header, and one opaque center pixel per engine badge.
    use super::*;
    use anvil_image::decode_ico;

    #[test]
    /// The masked mark, the ICO bytes, and one opaque center pixel per engine badge.
    fn renders_embedded_anvil_logo_and_encodes_valid_windows_ico() {
        let img = render_anvil_logo(48);
        assert_eq!(img.width, 48);
        assert_eq!(img.height, 48);
        // Center pixel of the logo should be opaque
        let center = img.pixel(24, 24).unwrap();
        assert_eq!(center[3], 255);
        // Corner pixel outside the circle should be transparent
        let corner = img.pixel(0, 0).unwrap();
        assert_eq!(corner[3], 0);

        let ico = encode_anvil_ico();
        assert!(ico.len() > 22);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        let decoded_ico = decode_ico(&ico).expect("valid ICO decoded by anvil-image");
        assert_eq!(decoded_ico.width, 64);
        assert_eq!(decoded_ico.height, 64);

        for engine in [
            SearchEngine::Google,
            SearchEngine::DuckDuckGo,
            SearchEngine::Bing,
        ] {
            let badge = render_search_engine_logo(engine, 36);
            assert_eq!(badge.width, 36);
            assert_eq!(badge.height, 36);
            assert_eq!(badge.pixel(18, 18).unwrap()[3], 255);
            assert_eq!(badge.pixel(0, 0).unwrap()[3], 0);
        }
    }
}
