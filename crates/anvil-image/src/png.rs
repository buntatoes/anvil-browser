// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust PNG decoder (1/2/4/8/16-bit, color types 0/2/3/4/6, tRNS, Adam7) and encoder.

use crate::deflate::{crc32, crc32_update, zlib_compress, zlib_decompress_bounded};
use crate::{Image, ImageError, PNG_SIGNATURE, validate_dimensions};

const MAX_PNG_IDAT_BYTES: usize = 64 * 1024 * 1024;

const ADAM7_PASSES: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

#[inline]
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let ai = i32::from(a);
    let bi = i32::from(b);
    let ci = i32::from(c);
    let p = ai + bi - ci;
    let pa = (p - ai).abs();
    let pb = (p - bi).abs();
    let pc = (p - ci).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn valid_color_depth(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => false,
    }
}

fn channel_count(color_type: u8) -> usize {
    match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 0,
    }
}

fn scale_sample_to_u8(sample: u16, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => {
            if sample != 0 {
                255
            } else {
                0
            }
        }
        2 => (sample * 85) as u8,
        4 => (sample * 17) as u8,
        8 => sample as u8,
        16 => (sample >> 8) as u8,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_png_pass(
    raw: &[u8],
    raw_offset: &mut usize,
    pass_w: usize,
    pass_h: usize,
    x0: usize,
    y0: usize,
    dx: usize,
    dy: usize,
    bit_depth: u8,
    color_type: u8,
    palette: &[[u8; 3]],
    trns: &[u8],
    out: &mut Image,
) -> Result<(), ImageError> {
    if pass_w == 0 || pass_h == 0 {
        return Ok(());
    }
    let channels = channel_count(color_type);
    let bits_per_pixel = channels * usize::from(bit_depth);
    let stride = (pass_w * bits_per_pixel).div_ceil(8);
    let filter_bpp = bits_per_pixel.div_ceil(8).max(1);

    let needed = pass_h.saturating_mul(1 + stride);
    if *raw_offset + needed > raw.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let mut prev_row = vec![0u8; stride];
    let mut cur_row = vec![0u8; stride];

    for py in 0..pass_h {
        let row_start = *raw_offset;
        *raw_offset += 1 + stride;
        let filter = raw[row_start];
        let encoded = &raw[row_start + 1..row_start + 1 + stride];

        for i in 0..stride {
            let x = encoded[i];
            let a = if i >= filter_bpp {
                cur_row[i - filter_bpp]
            } else {
                0
            };
            let b = prev_row[i];
            let c = if i >= filter_bpp {
                prev_row[i - filter_bpp]
            } else {
                0
            };
            cur_row[i] = match filter {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => x.wrapping_add(paeth_predictor(a, b, c)),
                _ => return Err(ImageError::InvalidFilter(filter)),
            };
        }

        let dst_y = (y0 + py * dy) as u32;
        for px in 0..pass_w {
            let dst_x = (x0 + px * dx) as u32;
            let rgba = match color_type {
                0 => {
                    let sample = read_subbyte_or_word(&cur_row, px, bit_depth);
                    let gray = scale_sample_to_u8(sample, bit_depth);
                    let alpha =
                        if trns.len() >= 2 && u16::from_be_bytes([trns[0], trns[1]]) == sample {
                            0
                        } else {
                            255
                        };
                    [gray, gray, gray, alpha]
                }
                2 => {
                    if bit_depth == 8 {
                        let base = px * 3;
                        let r = cur_row[base];
                        let g = cur_row[base + 1];
                        let b = cur_row[base + 2];
                        let alpha = if trns.len() >= 6
                            && u16::from_be_bytes([trns[0], trns[1]]) == u16::from(r)
                            && u16::from_be_bytes([trns[2], trns[3]]) == u16::from(g)
                            && u16::from_be_bytes([trns[4], trns[5]]) == u16::from(b)
                        {
                            0
                        } else {
                            255
                        };
                        [r, g, b, alpha]
                    } else {
                        let base = px * 6;
                        let r16 = u16::from_be_bytes([cur_row[base], cur_row[base + 1]]);
                        let g16 = u16::from_be_bytes([cur_row[base + 2], cur_row[base + 3]]);
                        let b16 = u16::from_be_bytes([cur_row[base + 4], cur_row[base + 5]]);
                        let alpha = if trns.len() >= 6
                            && u16::from_be_bytes([trns[0], trns[1]]) == r16
                            && u16::from_be_bytes([trns[2], trns[3]]) == g16
                            && u16::from_be_bytes([trns[4], trns[5]]) == b16
                        {
                            0
                        } else {
                            255
                        };
                        [(r16 >> 8) as u8, (g16 >> 8) as u8, (b16 >> 8) as u8, alpha]
                    }
                }
                3 => {
                    let idx = usize::from(read_subbyte_or_word(&cur_row, px, bit_depth));
                    let rgb = palette.get(idx).copied().unwrap_or([0, 0, 0]);
                    let alpha = trns.get(idx).copied().unwrap_or(255);
                    [rgb[0], rgb[1], rgb[2], alpha]
                }
                4 => {
                    if bit_depth == 8 {
                        let base = px * 2;
                        let v = cur_row[base];
                        [v, v, v, cur_row[base + 1]]
                    } else {
                        let base = px * 4;
                        let v = cur_row[base];
                        [v, v, v, cur_row[base + 2]]
                    }
                }
                6 => {
                    if bit_depth == 8 {
                        let base = px * 4;
                        [
                            cur_row[base],
                            cur_row[base + 1],
                            cur_row[base + 2],
                            cur_row[base + 3],
                        ]
                    } else {
                        let base = px * 8;
                        [
                            cur_row[base],
                            cur_row[base + 2],
                            cur_row[base + 4],
                            cur_row[base + 6],
                        ]
                    }
                }
                _ => [0, 0, 0, 255],
            };
            out.set_pixel(dst_x, dst_y, rgba);
        }
        prev_row.copy_from_slice(&cur_row);
    }

    Ok(())
}

#[inline]
fn read_subbyte_or_word(row: &[u8], px: usize, bit_depth: u8) -> u16 {
    match bit_depth {
        1 => u16::from((row[px >> 3] >> (7 - (px & 7))) & 0x01),
        2 => u16::from((row[px >> 2] >> ((3 - (px & 3)) * 2)) & 0x03),
        4 => u16::from((row[px >> 1] >> ((1 - (px & 1)) * 4)) & 0x0F),
        8 => u16::from(row[px]),
        16 => {
            let base = px * 2;
            u16::from_be_bytes([row[base], row[base + 1]])
        }
        _ => 0,
    }
}

/// Decode a PNG stream (1/2/4/8/16-bit, color types 0/2/3/4/6, non-interlaced or Adam7) into RGBA.
pub fn decode_png(data: &[u8]) -> Result<Image, ImageError> {
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(ImageError::InvalidHeader("missing PNG signature"));
    }

    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut saw_ihdr = false;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();

    while pos + 8 <= data.len() {
        let chunk_len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        pos += 8;
        if chunk_len
            .checked_add(4)
            .and_then(|n| pos.checked_add(n))
            .is_none_or(|end| end > data.len())
        {
            return Err(ImageError::UnexpectedEof);
        }
        let chunk_data = &data[pos..pos + chunk_len];
        let expected_crc = u32::from_be_bytes(
            data[pos + chunk_len..pos + chunk_len + 4]
                .try_into()
                .unwrap(),
        );
        let computed_crc = !crc32_update(crc32_update(0xFFFF_FFFFu32, chunk_type), chunk_data);
        if computed_crc != expected_crc {
            return Err(ImageError::InvalidHeader("bad PNG chunk CRC"));
        }
        pos += chunk_len + 4;

        match chunk_type {
            b"IHDR" => {
                if chunk_len < 13 {
                    return Err(ImageError::InvalidHeader("short PNG IHDR chunk"));
                }
                width = u32::from_be_bytes(chunk_data[0..4].try_into().unwrap());
                height = u32::from_be_bytes(chunk_data[4..8].try_into().unwrap());
                bit_depth = chunk_data[8];
                color_type = chunk_data[9];
                let compression = chunk_data[10];
                let filter = chunk_data[11];
                interlace = chunk_data[12];
                validate_dimensions(width, height)?;
                if !valid_color_depth(color_type, bit_depth) {
                    return Err(ImageError::UnsupportedFormat(
                        "unsupported PNG color type or bit depth",
                    ));
                }
                if compression != 0 || filter != 0 || interlace > 1 {
                    return Err(ImageError::UnsupportedFormat(
                        "unsupported PNG compression, filter, or interlace method",
                    ));
                }
                saw_ihdr = true;
            }
            b"PLTE" => {
                palette.reserve(chunk_len / 3);
                for &rgb in chunk_data.as_chunks::<3>().0 {
                    palette.push(rgb);
                }
            }
            b"tRNS" => {
                if trns.len().saturating_add(chunk_len) <= 4096 {
                    trns.extend_from_slice(chunk_data);
                }
            }
            b"IDAT" => {
                if idat.len().saturating_add(chunk_len) > MAX_PNG_IDAT_BYTES {
                    return Err(ImageError::DimensionsTooLarge { width, height });
                }
                idat.extend_from_slice(chunk_data);
            }
            b"IEND" => break,
            _ => {}
        }
    }

    if !saw_ihdr {
        return Err(ImageError::InvalidHeader("missing PNG IHDR chunk"));
    }

    let channels = channel_count(color_type);
    let bits_per_pixel = channels * usize::from(bit_depth);
    let w = width as usize;
    let h = height as usize;

    let expected_raw = if interlace == 0 {
        let stride = (w * bits_per_pixel).div_ceil(8);
        h.saturating_mul(1 + stride)
    } else {
        let mut total = 0usize;
        for (x0, y0, dx, dy) in ADAM7_PASSES {
            if x0 < w && y0 < h {
                let pw = (w - x0).div_ceil(dx);
                let ph = (h - y0).div_ceil(dy);
                let stride = (pw * bits_per_pixel).div_ceil(8);
                total = total.saturating_add(ph.saturating_mul(1 + stride));
            }
        }
        total
    };

    let raw = zlib_decompress_bounded(&idat, expected_raw.saturating_add(65_536))?;
    if raw.len() < expected_raw {
        return Err(ImageError::UnexpectedEof);
    }

    let mut out = Image::new(width, height);
    let mut raw_offset = 0usize;
    if interlace == 0 {
        decode_png_pass(
            &raw,
            &mut raw_offset,
            w,
            h,
            0,
            0,
            1,
            1,
            bit_depth,
            color_type,
            &palette,
            &trns,
            &mut out,
        )?;
    } else {
        for (x0, y0, dx, dy) in ADAM7_PASSES {
            if x0 < w && y0 < h {
                let pw = (w - x0).div_ceil(dx);
                let ph = (h - y0).div_ceil(dy);
                decode_png_pass(
                    &raw,
                    &mut raw_offset,
                    pw,
                    ph,
                    x0,
                    y0,
                    dx,
                    dy,
                    bit_depth,
                    color_type,
                    &palette,
                    &trns,
                    &mut out,
                )?;
            }
        }
    }

    Ok(out)
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let c = crc32(&out[crc_start..]);
    out.extend_from_slice(&c.to_be_bytes());
}

/// Encode `image` as a non-interlaced 8-bit RGBA PNG.
pub fn encode_png(image: &Image) -> Vec<u8> {
    let stride = (image.width as usize).saturating_mul(4);
    let mut raw = Vec::with_capacity((image.height as usize).saturating_mul(1 + stride));
    for y in 0..image.height as usize {
        raw.push(0);
        let start = y * stride;
        let end = (start + stride).min(image.pixels.len());
        raw.extend_from_slice(&image.pixels[start..end]);
        if end - start < stride {
            raw.resize(raw.len() + (stride - (end - start)), 0);
        }
    }

    let compressed = zlib_compress(&raw);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&image.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&image.height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    ihdr[10] = 0;
    ihdr[11] = 0;
    ihdr[12] = 0;

    let mut out = Vec::with_capacity(8 + 25 + 12 + compressed.len() + 12);
    out.extend_from_slice(&PNG_SIGNATURE);
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &compressed);
    write_png_chunk(&mut out, b"IEND", &[]);
    out
}
