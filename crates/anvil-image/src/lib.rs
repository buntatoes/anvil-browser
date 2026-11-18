// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust clean-room PNG, JPEG, GIF, WebP, ICO, BMP, and PPM image codecs and resampling.

mod deflate;
mod gif;
mod ico;
mod jpeg;
mod png;
mod vp8_tables;
mod webp;

use std::fmt;

/// Eight-byte PNG signature.
pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Largest width or height a decoder will accept, in pixels.
pub const MAX_IMAGE_DIMENSION: u32 = 8192;

/// Largest `width * height` a decoder will accept.
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;

/// Error returned on image decoding or validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    UnexpectedEof,
    UnsupportedFormat(&'static str),
    InvalidHeader(&'static str),
    DimensionsTooLarge { width: u32, height: u32 },
    InvalidFilter(u8),
    Worker(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of image file"),
            Self::UnsupportedFormat(msg) => write!(f, "unsupported image format: {msg}"),
            Self::InvalidHeader(msg) => write!(f, "invalid image header: {msg}"),
            Self::DimensionsTooLarge { width, height } => write!(
                f,
                "image dimensions {width}x{height} exceed limits ({MAX_IMAGE_DIMENSION}x{MAX_IMAGE_DIMENSION}, {MAX_IMAGE_PIXELS} pixels)"
            ),
            Self::InvalidFilter(kind) => write!(f, "invalid PNG scanline filter {kind}"),
            Self::Worker(msg) => write!(f, "image worker error: {msg}"),
        }
    }
}

impl std::error::Error for ImageError {}

/// Validate width and height against maximum dimensions and pixel bounds.
pub fn validate_dimensions(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidHeader("zero-sized image"));
    }
    let total_pixels = u64::from(width) * u64::from(height);
    if width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || total_pixels > MAX_IMAGE_PIXELS
    {
        return Err(ImageError::DimensionsTooLarge { width, height });
    }
    Ok(())
}

/// Top-down 8-bit image with tightly packed RGBA pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        validate_dimensions(width, height)?;
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if pixels.len() != expected {
            return Err(ImageError::InvalidHeader("RGBA pixel buffer size mismatch"));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = ((y * self.width + x) as usize) * 4;
        Some([
            self.pixels[idx],
            self.pixels[idx + 1],
            self.pixels[idx + 2],
            self.pixels[idx + 3],
        ])
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y * self.width + x) as usize) * 4;
        self.pixels[idx..idx + 4].copy_from_slice(&rgba);
    }

    pub fn mipmap_halve(&self, halve_x: bool, halve_y: bool) -> Self {
        if self.width == 0 || self.height == 0 {
            return Self::new(0, 0);
        }
        let dst_w = if halve_x && self.width > 1 {
            self.width.div_ceil(2)
        } else {
            self.width
        };
        let dst_h = if halve_y && self.height > 1 {
            self.height.div_ceil(2)
        } else {
            self.height
        };
        let step_x = if dst_w < self.width { 2u32 } else { 1u32 };
        let step_y = if dst_h < self.height { 2u32 } else { 1u32 };
        let mut out = Self::new(dst_w, dst_h);
        for dy in 0..dst_h {
            let sy0 = dy * step_y;
            let sy1 = (sy0 + step_y - 1).min(self.height - 1);
            for dx in 0..dst_w {
                let sx0 = dx * step_x;
                let sx1 = (sx0 + step_x - 1).min(self.width - 1);
                let taps = [
                    self.pixel(sx0, sy0).unwrap_or([0; 4]),
                    self.pixel(sx1, sy0).unwrap_or([0; 4]),
                    self.pixel(sx0, sy1).unwrap_or([0; 4]),
                    self.pixel(sx1, sy1).unwrap_or([0; 4]),
                ];
                let mut sum_r = 0.0f32;
                let mut sum_g = 0.0f32;
                let mut sum_b = 0.0f32;
                let mut sum_a = 0.0f32;
                for c in taps {
                    let a = c[3] as f32;
                    sum_r += (c[0] as f32) * a;
                    sum_g += (c[1] as f32) * a;
                    sum_b += (c[2] as f32) * a;
                    sum_a += a;
                }
                let out_a = (sum_a * 0.25).round().clamp(0.0, 255.0) as u8;
                let rgba = if sum_a > 0.0 {
                    [
                        (sum_r / sum_a).round().clamp(0.0, 255.0) as u8,
                        (sum_g / sum_a).round().clamp(0.0, 255.0) as u8,
                        (sum_b / sum_a).round().clamp(0.0, 255.0) as u8,
                        out_a,
                    ]
                } else {
                    [0, 0, 0, 0]
                };
                out.set_pixel(dx, dy, rgba);
            }
        }
        out
    }

    pub fn resize_bilinear(&self, new_width: u32, new_height: u32) -> Self {
        if new_width == 0 || new_height == 0 || self.width == 0 || self.height == 0 {
            return Self::new(new_width, new_height);
        }
        if new_width == self.width && new_height == self.height {
            return self.clone();
        }
        self.local_resize_bilinear(new_width, new_height)
    }

    fn local_resize_bilinear(&self, new_width: u32, new_height: u32) -> Self {
        let mut src = std::borrow::Cow::Borrowed(self);
        while (src.width > new_width.saturating_mul(2) && src.width > 1)
            || (src.height > new_height.saturating_mul(2) && src.height > 1)
        {
            let hx = src.width > new_width.saturating_mul(2) && src.width > 1;
            let hy = src.height > new_height.saturating_mul(2) && src.height > 1;
            let next = src.mipmap_halve(hx, hy);
            src = std::borrow::Cow::Owned(next);
        }
        if src.width == new_width && src.height == new_height {
            return src.into_owned();
        }

        let mut out = Self::new(new_width, new_height);
        let x_ratio = (src.width as f32) / (new_width as f32);
        let y_ratio = (src.height as f32) / (new_height as f32);

        for ny in 0..new_height {
            let sy = ((ny as f32 + 0.5) * y_ratio - 0.5).clamp(0.0, (src.height - 1) as f32);
            let y0 = sy.floor() as u32;
            let y1 = (y0 + 1).min(src.height - 1);
            let wy = sy - (y0 as f32);

            for nx in 0..new_width {
                let sx = ((nx as f32 + 0.5) * x_ratio - 0.5).clamp(0.0, (src.width - 1) as f32);
                let x0 = sx.floor() as u32;
                let x1 = (x0 + 1).min(src.width - 1);
                let wx = sx - (x0 as f32);

                let c00 = src.pixel(x0, y0).unwrap_or([0; 4]);
                let c10 = src.pixel(x1, y0).unwrap_or([0; 4]);
                let c01 = src.pixel(x0, y1).unwrap_or([0; 4]);
                let c11 = src.pixel(x1, y1).unwrap_or([0; 4]);

                let w00 = (1.0 - wx) * (1.0 - wy);
                let w10 = wx * (1.0 - wy);
                let w01 = (1.0 - wx) * wy;
                let w11 = wx * wy;

                let a00 = (c00[3] as f32) * w00;
                let a10 = (c10[3] as f32) * w10;
                let a01 = (c01[3] as f32) * w01;
                let a11 = (c11[3] as f32) * w11;
                let sum_a = a00 + a10 + a01 + a11;
                let out_a = sum_a.round().clamp(0.0, 255.0) as u8;

                let rgba = if sum_a > 0.0 {
                    let mut rgb = [0u8; 4];
                    for ch in 0..3 {
                        let pm = (c00[ch] as f32) * a00
                            + (c10[ch] as f32) * a10
                            + (c01[ch] as f32) * a01
                            + (c11[ch] as f32) * a11;
                        rgb[ch] = (pm / sum_a).round().clamp(0.0, 255.0) as u8;
                    }
                    rgb[3] = out_a;
                    rgb
                } else {
                    [0, 0, 0, 0]
                };
                out.set_pixel(nx, ny, rgba);
            }
        }
        out
    }
}

/// Detect image container format automatically and decode into top-down RGBA.
pub fn decode_auto(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() >= 8 && bytes[..8] == PNG_SIGNATURE {
        return png::decode_png(bytes);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        return jpeg::decode_jpeg(bytes);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return gif::decode_gif(bytes);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return webp::decode_webp(bytes);
    }
    if bytes.starts_with(&[0, 0, 1, 0]) || bytes.starts_with(&[0, 0, 2, 0]) {
        return ico::decode_ico(bytes);
    }
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    if bytes.starts_with(b"P6") || bytes.starts_with(b"P5") {
        return decode_ppm(bytes);
    }
    Err(ImageError::UnsupportedFormat("unrecognized image format"))
}

pub fn decode_png(bytes: &[u8]) -> Result<Image, ImageError> {
    png::decode_png(bytes)
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<Image, ImageError> {
    jpeg::decode_jpeg(bytes)
}

pub fn decode_gif(bytes: &[u8]) -> Result<Image, ImageError> {
    gif::decode_gif(bytes)
}

pub fn decode_webp(bytes: &[u8]) -> Result<Image, ImageError> {
    webp::decode_webp(bytes)
}

pub fn decode_ico(bytes: &[u8]) -> Result<Image, ImageError> {
    ico::decode_ico(bytes)
}

pub fn decode_bmp(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 54 || !bytes.starts_with(b"BM") {
        return Err(ImageError::InvalidHeader("missing BMP signature"));
    }
    let data_offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    let header_size = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    if header_size < 40 {
        return Err(ImageError::UnsupportedFormat("unsupported BMP header type"));
    }
    let width_i32 = i32::from_le_bytes(bytes[18..22].try_into().unwrap());
    let height_i32 = i32::from_le_bytes(bytes[22..26].try_into().unwrap());
    let planes = u16::from_le_bytes(bytes[26..28].try_into().unwrap());
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    let compression = u32::from_le_bytes(bytes[30..34].try_into().unwrap());
    if planes != 1 {
        return Err(ImageError::InvalidHeader("BMP planes must be 1"));
    }
    if width_i32 <= 0 || height_i32 == 0 {
        return Err(ImageError::InvalidHeader("invalid BMP dimensions"));
    }
    if compression != 0 && compression != 3 {
        return Err(ImageError::UnsupportedFormat(
            "compressed BMP not supported",
        ));
    }
    if !matches!(bpp, 1 | 4 | 8 | 24 | 32) {
        return Err(ImageError::UnsupportedFormat("unsupported BMP bit depth"));
    }
    let width = width_i32 as u32;
    let top_down = height_i32 < 0;
    let height = height_i32.unsigned_abs();
    validate_dimensions(width, height)?;

    let mut palette: Vec<[u8; 4]> = Vec::new();
    if bpp <= 8 {
        let palette_offset = 14 + header_size;
        let num_colors = 1usize << bpp;
        if palette_offset + num_colors * 4 <= bytes.len() {
            palette.reserve(num_colors);
            for i in 0..num_colors {
                let base = palette_offset + i * 4;
                palette.push([bytes[base + 2], bytes[base + 1], bytes[base], 255]);
            }
        }
    }

    let row_stride = (width as usize * bpp as usize).div_ceil(32) * 4;
    if data_offset.saturating_add(row_stride.saturating_mul(height as usize)) > bytes.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let mut out = Image::new(width, height);
    for row in 0..(height as usize) {
        let dst_y = if top_down {
            row as u32
        } else {
            height - 1 - (row as u32)
        };
        let row_base = data_offset + row * row_stride;
        match bpp {
            1 => {
                for x in 0..width {
                    let byte_idx = row_base + (x as usize / 8);
                    let bit_idx = 7 - (x % 8);
                    let color_idx = ((bytes[byte_idx] >> bit_idx) & 1) as usize;
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            4 => {
                for x in 0..width {
                    let byte_idx = row_base + (x as usize / 2);
                    let color_idx = if x % 2 == 0 {
                        ((bytes[byte_idx] >> 4) & 0x0F) as usize
                    } else {
                        (bytes[byte_idx] & 0x0F) as usize
                    };
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            8 => {
                for x in 0..width {
                    let color_idx = bytes[row_base + x as usize] as usize;
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            24 => {
                for x in 0..width {
                    let px_base = row_base + (x as usize) * 3;
                    out.set_pixel(
                        x,
                        dst_y,
                        [bytes[px_base + 2], bytes[px_base + 1], bytes[px_base], 255],
                    );
                }
            }
            32 => {
                for x in 0..width {
                    let px_base = row_base + (x as usize) * 4;
                    out.set_pixel(
                        x,
                        dst_y,
                        [
                            bytes[px_base + 2],
                            bytes[px_base + 1],
                            bytes[px_base],
                            bytes[px_base + 3],
                        ],
                    );
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn read_ppm_token(data: &[u8], pos: &mut usize) -> Result<u32, ImageError> {
    while *pos < data.len() {
        let b = data[*pos];
        if b == b'#' {
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
        } else if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            *pos += 1;
        } else {
            break;
        }
    }
    let start = *pos;
    while *pos < data.len() && data[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if start == *pos {
        return Err(ImageError::InvalidHeader(
            "expected numeric token in PPM header",
        ));
    }
    std::str::from_utf8(&data[start..*pos])
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or(ImageError::InvalidHeader("invalid number in PPM header"))
}

pub fn decode_ppm(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 7 || (!bytes.starts_with(b"P6") && !bytes.starts_with(b"P5")) {
        return Err(ImageError::InvalidHeader("missing Netpbm signature"));
    }
    let is_p6 = bytes.starts_with(b"P6");
    let mut pos = 2usize;
    let width = read_ppm_token(bytes, &mut pos)?;
    let height = read_ppm_token(bytes, &mut pos)?;
    let maxval = read_ppm_token(bytes, &mut pos)?;
    if width == 0 || height == 0 || maxval == 0 || maxval > 255 {
        return Err(ImageError::InvalidHeader("invalid PPM header values"));
    }
    validate_dimensions(width, height)?;
    if pos >= bytes.len() {
        return Err(ImageError::UnexpectedEof);
    }
    pos += 1;
    let mut out = Image::new(width, height);
    if is_p6 {
        let needed = (width as usize) * (height as usize) * 3;
        if pos + needed > bytes.len() {
            return Err(ImageError::UnexpectedEof);
        }
        for y in 0..height {
            for x in 0..width {
                let idx = pos + ((y * width + x) as usize) * 3;
                out.set_pixel(x, y, [bytes[idx], bytes[idx + 1], bytes[idx + 2], 255]);
            }
        }
    } else {
        let needed = (width as usize) * (height as usize);
        if pos + needed > bytes.len() {
            return Err(ImageError::UnexpectedEof);
        }
        for y in 0..height {
            for x in 0..width {
                let g = bytes[pos + ((y * width + x) as usize)];
                out.set_pixel(x, y, [g, g, g, 255]);
            }
        }
    }
    Ok(out)
}

/// Encode `image` as a PNG.
pub fn encode_png(image: &Image) -> Vec<u8> {
    png::encode_png(image)
}

/// Encode `image` as a 32-bit BMP.
pub fn encode_bmp(image: &Image) -> Vec<u8> {
    let pixel_bytes = (image.width as usize)
        .saturating_mul(image.height as usize)
        .saturating_mul(4);
    let file_size = 54 + pixel_bytes;
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(image.width as i32).to_le_bytes());
    out.extend_from_slice(&(-(image.height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for chunk in image.pixels.as_chunks::<4>().0 {
        out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
    }
    out
}

/// Encode `image` as a 1-entry Windows ICO container wrapping a PNG payload.
pub fn encode_ico(image: &Image) -> Vec<u8> {
    ico::encode_ico(image)
}

/// Encode `image` as a Netpbm binary P6 PPM.
pub fn encode_ppm(image: &Image) -> Vec<u8> {
    let header = format!("P6\n{} {}\n255\n", image.width, image.height);
    let mut out =
        Vec::with_capacity(header.len() + (image.width as usize * image.height as usize * 3));
    out.extend_from_slice(header.as_bytes());
    for chunk in image.pixels.as_chunks::<4>().0 {
        out.push(chunk[0]);
        out.push(chunk[1]);
        out.push(chunk[2]);
    }
    out
}

pub fn encode_webp(image: &Image) -> Vec<u8> {
    webp::encode_webp(image)
}

pub fn encode_gif(image: &Image) -> Vec<u8> {
    gif::encode_gif(image)
}

pub fn encode_jpeg(image: &Image) -> Vec<u8> {
    jpeg::encode_jpeg(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_basic() {
        let mut img = Image::new(2, 2);
        img.set_pixel(0, 0, [255, 0, 0, 255]);
        assert_eq!(img.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(1, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn test_dimensions_check() {
        assert!(validate_dimensions(10, 10).is_ok());
        assert!(validate_dimensions(0, 10).is_err());
        assert!(validate_dimensions(9000, 10).is_err());
        assert!(Image::from_rgba(0, 0, vec![]).is_err());
        assert!(Image::from_rgba(0, 10, vec![]).is_err());
        assert!(Image::from_rgba(9000, 10, vec![0; 9000 * 10 * 4]).is_err());
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_ok());
    }

    #[test]
    fn test_pure_rust_codecs_roundtrip() {
        let mut img = Image::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let a = if (x + y) % 5 == 0 { 0 } else { 255 };
                img.set_pixel(x, y, [(x * 16) as u8, (y * 16) as u8, 128, a]);
            }
        }

        let png = encode_png(&img);
        let dec_png = decode_auto(&png).expect("decode PNG in-process");
        assert_eq!(dec_png.width, 16);
        assert_eq!(dec_png.height, 16);
        assert_eq!(dec_png.pixels, img.pixels);

        let bmp = encode_bmp(&img);
        let dec_bmp = decode_auto(&bmp).expect("decode BMP in-process");
        assert_eq!(dec_bmp.pixels, img.pixels);

        let ico = encode_ico(&img);
        let dec_ico = decode_auto(&ico).expect("decode ICO in-process");
        assert_eq!(dec_ico.pixels, img.pixels);

        let gif = encode_gif(&img);
        let dec_gif = decode_auto(&gif).expect("decode GIF in-process");
        assert_eq!(dec_gif.width, 16);
        assert_eq!(dec_gif.pixel(0, 0).unwrap()[3], 0);
        assert_eq!(dec_gif.pixel(1, 0).unwrap()[3], 255);

        let webp = encode_webp(&img);
        let dec_webp = decode_auto(&webp).expect("decode WebP VP8L in-process");
        assert_eq!(dec_webp.pixels, img.pixels);

        let mut opaque = Image::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                opaque.set_pixel(x, y, [200, 80, 40, 255]);
            }
        }
        let jpg = encode_jpeg(&opaque);
        let dec_jpg = decode_auto(&jpg).expect("decode JPEG in-process");
        assert_eq!(dec_jpg.width, 16);
        let px = dec_jpg.pixel(8, 8).unwrap();
        assert!(px[0] > 180 && px[1] < 100 && px[2] < 65 && px[3] == 255);

        let resized = img.resize_bilinear(8, 8);
        assert_eq!(resized.width, 8);
        assert_eq!(resized.height, 8);
    }

    #[test]
    fn test_png_subbyte_adam7_and_ico_8bpp_and_mask() {
        // Construct an 8x8 Adam7-interlaced 1-bit grayscale PNG with tRNS key 0.
        let mut raw = Vec::new();
        // 7 passes for 8x8: sizes 1x1, 1x1, 2x1, 2x2, 4x2, 4x4, 8x4
        for (pw, ph) in [(1, 1), (1, 1), (2, 1), (2, 2), (4, 2), (4, 4), (8, 4)] {
            for _ in 0..ph {
                raw.push(0); // filter None
                let bytes_per_row = (pwusize(pw)).div_ceil(8);
                raw.extend(std::iter::repeat_n(0xFFu8, bytes_per_row));
            }
        }
        fn pwusize(v: usize) -> usize {
            v
        }
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&8u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&8u32.to_be_bytes());
        ihdr[8] = 1; // 1-bit depth
        ihdr[9] = 0; // grayscale
        ihdr[12] = 1; // Adam7 interlace
        let compressed = crate::deflate::zlib_compress(&raw);
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        let append_chunk = |dst: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]| {
            dst.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            let s = dst.len();
            dst.extend_from_slice(tag);
            dst.extend_from_slice(payload);
            let c = crate::deflate::crc32(&dst[s..]);
            dst.extend_from_slice(&c.to_be_bytes());
        };
        append_chunk(&mut png, b"IHDR", &ihdr);
        append_chunk(&mut png, b"tRNS", &[0, 0]);
        append_chunk(&mut png, b"IDAT", &compressed);
        append_chunk(&mut png, b"IEND", &[]);
        let decoded = decode_png(&png).expect("decode 1-bit Adam7 PNG");
        assert_eq!(decoded.pixel(7, 7), Some([255, 255, 255, 255]));

        // Construct a 2x2 8-bpp paletted ICO DIB with 1-bpp AND transparency mask.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&2i32.to_le_bytes()); // biWidth
        dib.extend_from_slice(&4i32.to_le_bytes()); // biHeight (2 * 2)
        dib.extend_from_slice(&1u16.to_le_bytes()); // planes
        dib.extend_from_slice(&8u16.to_le_bytes()); // bpp = 8
        dib.extend_from_slice(&0u32.to_le_bytes()); // compression = BI_RGB
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&2u32.to_le_bytes()); // colorsUsed = 2
        dib.extend_from_slice(&0u32.to_le_bytes());
        // Palette: index 0 = blue BGR (255,0,0), index 1 = red BGR (0,0,255)
        dib.extend_from_slice(&[255, 0, 0, 0, 0, 0, 255, 0]);
        // XOR rows (bottom-up, padded to 4 bytes): row0=[0, 1], row1=[1, 0]
        dib.extend_from_slice(&[0, 1, 0, 0, 1, 0, 0, 0]);
        // AND mask rows (bottom-up, padded to 4 bytes): bottom row (y=1) bit 0 transparent, top row (y=0) opaque
        dib.extend_from_slice(&[0x80, 0, 0, 0, 0x00, 0, 0, 0]);
        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // ICO header, 1 entry
        ico.extend_from_slice(&[2, 2, 2, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&8u16.to_le_bytes());
        ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&dib);
        let decoded_ico = decode_ico(&ico).expect("decode 8bpp paletted ICO");
        assert_eq!(decoded_ico.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(decoded_ico.pixel(0, 1), Some([0, 0, 255, 0]));
    }
}
