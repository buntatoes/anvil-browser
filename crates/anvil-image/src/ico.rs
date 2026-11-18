// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust Windows ICO and CUR decoder supporting embedded PNG and 1/4/8/24/32-bpp DIBs.

use crate::png::{decode_png, encode_png};
use crate::{Image, ImageError, PNG_SIGNATURE, validate_dimensions};

/// Decode the highest-resolution icon image from an ICO (type 1) or CUR (type 2) container.
pub fn decode_ico(data: &[u8]) -> Result<Image, ImageError> {
    if data.len() < 6 {
        return Err(ImageError::UnexpectedEof);
    }
    let reserved = u16::from_le_bytes([data[0], data[1]]);
    let icon_type = u16::from_le_bytes([data[2], data[3]]);
    let count = usize::from(u16::from_le_bytes([data[4], data[5]]));
    if reserved != 0 || (icon_type != 1 && icon_type != 2) || count == 0 || count > 128 {
        return Err(ImageError::InvalidHeader("invalid ICO directory header"));
    }
    if 6 + count * 16 > data.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let mut best: Option<(u32, u32, usize, usize)> = None;
    for i in 0..count {
        let base = 6 + i * 16;
        let w = if data[base] == 0 {
            256
        } else {
            u32::from(data[base])
        };
        let h = if data[base + 1] == 0 {
            256
        } else {
            u32::from(data[base + 1])
        };
        let size = u32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap()) as usize;
        if size == 0 || offset.checked_add(size).is_none_or(|end| end > data.len()) {
            continue;
        }
        let area = w.saturating_mul(h);
        if best.is_none_or(|(bw, bh, _, _)| area >= bw.saturating_mul(bh)) {
            best = Some((w, h, offset, size));
        }
    }

    let (_, _, offset, size) =
        best.ok_or(ImageError::InvalidHeader("no valid ICO directory entry"))?;
    let payload = &data[offset..offset + size];

    if payload.len() >= 8 && payload[..8] == PNG_SIGNATURE {
        return decode_png(payload);
    }

    if payload.len() < 40 {
        return Err(ImageError::UnexpectedEof);
    }
    let header_size = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    if header_size < 40 || header_size > payload.len() {
        return Err(ImageError::InvalidHeader("invalid ICO DIB header"));
    }

    let width_i32 = i32::from_le_bytes(payload[4..8].try_into().unwrap());
    let raw_height_i32 = i32::from_le_bytes(payload[8..12].try_into().unwrap());
    let width = width_i32.unsigned_abs();
    let raw_height = raw_height_i32.unsigned_abs();
    let height = (raw_height / 2).max(1);
    validate_dimensions(width, height)?;

    let bpp = u16::from_le_bytes([payload[14], payload[15]]);
    let compression = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    if compression != 0 {
        return Err(ImageError::UnsupportedFormat(
            "compressed ICO DIB payload not supported",
        ));
    }

    let colors_used = u32::from_le_bytes(payload[32..36].try_into().unwrap()) as usize;
    let palette_entries = match bpp {
        1 | 4 | 8 => {
            let max_entries = 1usize << bpp;
            if colors_used == 0 || colors_used > max_entries {
                max_entries
            } else {
                colors_used
            }
        }
        24 | 32 => 0,
        _ => {
            return Err(ImageError::UnsupportedFormat(
                "unsupported ICO DIB bit depth",
            ));
        }
    };

    let palette_offset = header_size;
    let palette_bytes = palette_entries * 4;
    if palette_offset + palette_bytes > payload.len() {
        return Err(ImageError::UnexpectedEof);
    }
    let mut palette = Vec::with_capacity(palette_entries);
    for i in 0..palette_entries {
        let p = palette_offset + i * 4;
        palette.push([payload[p + 2], payload[p + 1], payload[p]]);
    }

    let xor_offset = palette_offset + palette_bytes;
    let row_bits = (width as usize).saturating_mul(usize::from(bpp));
    let xor_stride = row_bits.div_ceil(32) * 4;
    let xor_bytes = xor_stride.saturating_mul(height as usize);
    if xor_offset + xor_bytes > payload.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let and_offset = xor_offset + xor_bytes;
    let and_stride = (width as usize).div_ceil(32) * 4;
    let has_and_mask = and_offset + and_stride.saturating_mul(height as usize) <= payload.len();

    let mut img = Image::new(width, height);
    let mut any_nonzero_alpha = false;

    for y in 0..height {
        let src_row = (height - 1 - y) as usize;
        let row_start = xor_offset + src_row * xor_stride;
        for x in 0..width {
            let xu = x as usize;
            let [r, g, b, mut a] = match bpp {
                32 => {
                    let px = row_start + xu * 4;
                    let alpha = payload[px + 3];
                    if alpha != 0 {
                        any_nonzero_alpha = true;
                    }
                    [payload[px + 2], payload[px + 1], payload[px], alpha]
                }
                24 => {
                    let px = row_start + xu * 3;
                    [payload[px + 2], payload[px + 1], payload[px], 255]
                }
                8 => {
                    let idx = usize::from(payload[row_start + xu]);
                    let rgb = palette.get(idx).copied().unwrap_or([0, 0, 0]);
                    [rgb[0], rgb[1], rgb[2], 255]
                }
                4 => {
                    let byte = payload[row_start + (xu >> 1)];
                    let idx = if (xu & 1) == 0 {
                        usize::from(byte >> 4)
                    } else {
                        usize::from(byte & 0x0F)
                    };
                    let rgb = palette.get(idx).copied().unwrap_or([0, 0, 0]);
                    [rgb[0], rgb[1], rgb[2], 255]
                }
                1 => {
                    let byte = payload[row_start + (xu >> 3)];
                    let idx = usize::from((byte >> (7 - (xu & 7))) & 1);
                    let rgb = palette.get(idx).copied().unwrap_or([0, 0, 0]);
                    [rgb[0], rgb[1], rgb[2], 255]
                }
                _ => [0, 0, 0, 0],
            };
            if bpp <= 24 && has_and_mask {
                let mask_byte = payload[and_offset + src_row * and_stride + (xu >> 3)];
                if ((mask_byte >> (7 - (xu & 7))) & 1) != 0 {
                    a = 0;
                }
            }
            img.set_pixel(x, y, [r, g, b, a]);
        }
    }

    if bpp == 32 && !any_nonzero_alpha && has_and_mask {
        for y in 0..height {
            let src_row = (height - 1 - y) as usize;
            for x in 0..width {
                let xu = x as usize;
                let mask_byte = payload[and_offset + src_row * and_stride + (xu >> 3)];
                let a = if ((mask_byte >> (7 - (xu & 7))) & 1) != 0 {
                    0
                } else {
                    255
                };
                if let Some(mut px) = img.pixel(x, y) {
                    px[3] = a;
                    img.set_pixel(x, y, px);
                }
            }
        }
    }

    Ok(img)
}

/// Encode `image` as a single-entry ICO container wrapping a PNG payload.
pub fn encode_ico(image: &Image) -> Vec<u8> {
    let png = encode_png(image);
    let mut out = Vec::with_capacity(22 + png.len());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    let w_byte = if image.width >= 256 {
        0u8
    } else {
        image.width as u8
    };
    let h_byte = if image.height >= 256 {
        0u8
    } else {
        image.height as u8
    };
    out.push(w_byte);
    out.push(h_byte);
    out.push(0);
    out.push(0);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(&png);
    out
}
