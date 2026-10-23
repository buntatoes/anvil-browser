//! Flux Browser brand logo (`assets/flux-logo.png`) and Windows `.ico` icon generator.
//!
//! Embeds the official Flux Browser neon cyan-violet "f" emblem (`assets/flux-logo.png`),
//! applies an anti-aliased squircle mask, and resamples it at any target resolution for:
//! - The top-left browser window toolbar (`ui.rs`)
//! - Built-in `flux://logo.png` in the top-left of `flux://settings`, `flux://home`, `flux://security`, and `flux://history`
//! - Native Win32 window titlebar & taskbar `HICON` (`shell.rs`)
//! - Windows `.ico` icon file (`flux-logo.ico`) for Desktop/Start Menu shortcuts and Installed Apps (`installer.rs`)

use std::sync::OnceLock;

use flux_gpu::sdf_rounded_box;
use flux_image::{Image, decode_png, encode_png};

/// Embedded 128x128 PNG bytes of the Flux Browser brand logo (`assets/flux-logo.png`).
pub const LOGO_PNG_BYTES: &[u8] = include_bytes!("../../../assets/flux-logo.png");

fn master_logo() -> &'static Image {
    static MASTER: OnceLock<Image> = OnceLock::new();
    MASTER.get_or_init(|| {
        let mut img = decode_png(LOGO_PNG_BYTES).expect("valid embedded assets/flux-logo.png");
        let w = img.width as f32;
        let h = img.height as f32;
        let hw = w * 0.5;
        let hh = h * 0.5;
        let radius = w.min(h) * 0.22;
        for py in 0..img.height {
            for px in 0..img.width {
                let rx = (px as f32 + 0.5) - hw;
                let ry = (py as f32 + 0.5) - hh;
                let d = sdf_rounded_box(rx, ry, hw - 0.5, hh - 0.5, radius);
                let alpha_mask = (0.5 - d).clamp(0.0, 1.0);
                if let Some(mut c) = img.pixel(px, py) {
                    c[3] = ((c[3] as f32) * alpha_mask).round() as u8;
                    img.set_pixel(px, py, c);
                }
            }
        }
        img
    })
}

/// Render the Flux Browser logo as an anti-aliased RGBA [`Image`] of `size x size` pixels.
pub fn render_flux_logo(size: u32) -> Image {
    let dim = size.clamp(12, 512);
    master_logo().resize_bilinear(dim, dim)
}

/// Encode the Flux Browser logo as a clean, metadata-free PNG image (`64x64`).
pub fn encode_flux_png() -> Vec<u8> {
    encode_png(&render_flux_logo(64))
}

/// Encode the Flux Browser logo as a standard Windows `.ico` file containing a PNG-compressed 64x64 icon.
pub fn encode_flux_ico() -> Vec<u8> {
    let icon_img = render_flux_logo(64);
    let png_bytes = encode_png(&icon_img);
    let png_len = png_bytes.len() as u32;

    let mut ico = Vec::with_capacity(22 + png_bytes.len());
    // ICONDIR header (6 bytes): reserved=0, type=1 (ICO), count=1
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    // ICONDIRENTRY (16 bytes)
    ico.push(64); // width
    ico.push(64); // height
    ico.push(0); // color count
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&png_len.to_le_bytes()); // bytes in resource
    ico.extend_from_slice(&22u32.to_le_bytes()); // image offset
    ico.extend_from_slice(&png_bytes);
    ico
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_embedded_flux_logo_and_encodes_valid_windows_ico() {
        let img = render_flux_logo(48);
        assert_eq!(img.width, 48);
        assert_eq!(img.height, 48);
        // Center pixel of the logo should be opaque
        let center = img.pixel(24, 24).unwrap();
        assert_eq!(center[3], 255);
        // Corner pixel outside the squircle should be transparent
        let corner = img.pixel(0, 0).unwrap();
        assert_eq!(corner[3], 0);

        let ico = encode_flux_ico();
        assert!(ico.len() > 22);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        let embedded_png = decode_png(&ico[22..]).expect("valid PNG inside ICO");
        assert_eq!(embedded_png.width, 64);
        assert_eq!(embedded_png.height, 64);
    }
}
