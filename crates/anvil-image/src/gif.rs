// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust GIF87a/GIF89a decoder (first frame) and GIF89a encoder.

use crate::{Image, ImageError, validate_dimensions};

fn read_sub_blocks(
    data: &[u8],
    pos: &mut usize,
    out: Option<&mut Vec<u8>>,
) -> Result<(), ImageError> {
    let mut dst = out;
    loop {
        if *pos >= data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        let sub_len = usize::from(data[*pos]);
        *pos += 1;
        if sub_len == 0 {
            return Ok(());
        }
        if *pos + sub_len > data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        if let Some(ref mut buf) = dst {
            if buf.len().saturating_add(sub_len) > 32 * 1024 * 1024 {
                return Err(ImageError::DimensionsTooLarge {
                    width: 0,
                    height: 0,
                });
            }
            buf.extend_from_slice(&data[*pos..*pos + sub_len]);
        }
        *pos += sub_len;
    }
}

fn interlaced_gif_row(row: usize, height: usize) -> usize {
    let mut r = row;
    const PASSES: [(usize, usize); 4] = [(0, 8), (4, 8), (2, 4), (1, 2)];
    for (start, step) in PASSES {
        if start < height {
            let count = (height - 1 - start) / step + 1;
            if r < count {
                return start + r * step;
            }
            r -= count;
        }
    }
    row
}

fn decode_gif_lzw(
    min_code_size: u8,
    stream: &[u8],
    expected_pixels: usize,
) -> Result<Vec<u8>, ImageError> {
    let clear_code = 1u16 << min_code_size;
    let end_code = clear_code + 1;

    let mut code_size = u32::from(min_code_size) + 1;
    let mut dict: Vec<Vec<u8>> = Vec::with_capacity(4096);
    let reset_dict = |d: &mut Vec<Vec<u8>>| {
        d.clear();
        for i in 0..=end_code {
            d.push(vec![(i & 0xFF) as u8]);
        }
    };
    reset_dict(&mut dict);

    let mut bit_pos = 0usize;
    let mut read_code = |size: u32| -> Option<u16> {
        let mut val = 0u32;
        for b in 0..size {
            let total_bit = bit_pos + b as usize;
            let byte_idx = total_bit >> 3;
            if byte_idx >= stream.len() {
                return None;
            }
            let bit = (stream[byte_idx] >> (total_bit & 7)) & 1;
            val |= u32::from(bit) << b;
        }
        bit_pos += size as usize;
        Some(val as u16)
    };

    let mut out = Vec::with_capacity(expected_pixels);
    let mut prev_seq: Option<Vec<u8>> = None;

    while let Some(code) = read_code(code_size) {
        if code == clear_code {
            reset_dict(&mut dict);
            code_size = u32::from(min_code_size) + 1;
            prev_seq = None;
            continue;
        }
        if code == end_code {
            break;
        }
        let idx = usize::from(code);
        let entry = if idx < dict.len() {
            dict[idx].clone()
        } else if let Some(ref prev) = prev_seq {
            if idx == dict.len() && !prev.is_empty() {
                let mut e = prev.clone();
                e.push(prev[0]);
                e
            } else {
                return Err(ImageError::InvalidHeader("invalid GIF LZW code"));
            }
        } else {
            return Err(ImageError::InvalidHeader("invalid initial GIF LZW code"));
        };

        let take = entry.len().min(expected_pixels.saturating_sub(out.len()));
        out.extend_from_slice(&entry[..take]);
        if out.len() >= expected_pixels {
            break;
        }

        if let Some(ref prev) = prev_seq
            && dict.len() < 4096
            && !entry.is_empty()
        {
            let mut next = prev.clone();
            next.push(entry[0]);
            dict.push(next);
            if dict.len() == (1usize << code_size) && code_size < 12 {
                code_size += 1;
            }
        }
        prev_seq = Some(entry);
    }

    Ok(out)
}

/// Decode the first image frame of a GIF87a or GIF89a stream into RGBA.
pub fn decode_gif(data: &[u8]) -> Result<Image, ImageError> {
    if data.len() < 13 || (&data[..6] != b"GIF87a" && &data[..6] != b"GIF89a") {
        return Err(ImageError::InvalidHeader("missing GIF87a/GIF89a signature"));
    }
    let screen_w = u32::from(u16::from_le_bytes([data[6], data[7]]));
    let screen_h = u32::from(u16::from_le_bytes([data[8], data[9]]));
    validate_dimensions(screen_w, screen_h)?;

    let packed = data[10];
    let has_gct = (packed & 0x80) != 0;
    let gct_size = 1usize << ((packed & 0x07) + 1);
    let mut pos = 13usize;

    let mut global_palette = Vec::new();
    if has_gct {
        if pos + gct_size * 3 > data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        global_palette.reserve(gct_size);
        for i in 0..gct_size {
            let base = pos + i * 3;
            global_palette.push([data[base], data[base + 1], data[base + 2]]);
        }
        pos += gct_size * 3;
    }

    let mut transparent_idx: Option<u8> = None;

    while pos < data.len() {
        let marker = data[pos];
        pos += 1;
        match marker {
            0x21 => {
                if pos >= data.len() {
                    return Err(ImageError::UnexpectedEof);
                }
                let ext_label = data[pos];
                pos += 1;
                if ext_label == 0xF9 {
                    if pos >= data.len() {
                        return Err(ImageError::UnexpectedEof);
                    }
                    let block_size = usize::from(data[pos]);
                    pos += 1;
                    if pos + block_size > data.len() {
                        return Err(ImageError::UnexpectedEof);
                    }
                    if block_size >= 4 && (data[pos] & 0x01) != 0 {
                        transparent_idx = Some(data[pos + 3]);
                    } else {
                        transparent_idx = None;
                    }
                    pos += block_size;
                }
                read_sub_blocks(data, &mut pos, None)?;
            }
            0x2C => {
                if pos + 9 > data.len() {
                    return Err(ImageError::UnexpectedEof);
                }
                let left = u32::from(u16::from_le_bytes([data[pos], data[pos + 1]]));
                let top = u32::from(u16::from_le_bytes([data[pos + 2], data[pos + 3]]));
                let img_w = u32::from(u16::from_le_bytes([data[pos + 4], data[pos + 5]]));
                let img_h = u32::from(u16::from_le_bytes([data[pos + 6], data[pos + 7]]));
                validate_dimensions(img_w, img_h)?;
                let img_packed = data[pos + 8];
                pos += 9;

                let has_lct = (img_packed & 0x80) != 0;
                let interlaced = (img_packed & 0x40) != 0;
                let lct_size = 1usize << ((img_packed & 0x07) + 1);

                let local_palette = if has_lct {
                    if pos + lct_size * 3 > data.len() {
                        return Err(ImageError::UnexpectedEof);
                    }
                    let mut p = Vec::with_capacity(lct_size);
                    for i in 0..lct_size {
                        let base = pos + i * 3;
                        p.push([data[base], data[base + 1], data[base + 2]]);
                    }
                    pos += lct_size * 3;
                    Some(p)
                } else {
                    None
                };

                let palette = local_palette.as_deref().unwrap_or(&global_palette);
                if pos >= data.len() {
                    return Err(ImageError::UnexpectedEof);
                }
                let min_code_size = data[pos];
                pos += 1;
                if !(2..=8).contains(&min_code_size) {
                    return Err(ImageError::InvalidHeader("invalid GIF LZW code size"));
                }

                let mut lzw_stream = Vec::new();
                read_sub_blocks(data, &mut pos, Some(&mut lzw_stream))?;

                let pixel_count = (img_w as usize) * (img_h as usize);
                let indices = decode_gif_lzw(min_code_size, &lzw_stream, pixel_count)?;

                let mut out = Image::new(screen_w, screen_h);
                for row in 0..img_h as usize {
                    let dst_row = if interlaced {
                        interlaced_gif_row(row, img_h as usize)
                    } else {
                        row
                    };
                    let py = top + dst_row as u32;
                    if py >= screen_h {
                        continue;
                    }
                    for col in 0..img_w as usize {
                        let px = left + col as u32;
                        if px >= screen_w {
                            continue;
                        }
                        let idx = indices
                            .get(row * img_w as usize + col)
                            .copied()
                            .unwrap_or(0);
                        if transparent_idx == Some(idx) {
                            out.set_pixel(px, py, [0, 0, 0, 0]);
                        } else {
                            let rgb = palette.get(usize::from(idx)).copied().unwrap_or([0, 0, 0]);
                            out.set_pixel(px, py, [rgb[0], rgb[1], rgb[2], 255]);
                        }
                    }
                }
                return Ok(out);
            }
            0x3B => break,
            _ => break,
        }
    }

    Err(ImageError::InvalidHeader("no image frame found in GIF"))
}

/// Encode `image` as a single-frame GIF89a with up to 256 palette entries and index-0 alpha.
pub fn encode_gif(image: &Image) -> Vec<u8> {
    let w = image.width.min(65535) as u16;
    let h = image.height.min(65535) as u16;
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&[0xF7, 0x00, 0x00]);

    let mut palette: Vec<[u8; 3]> = vec![[0, 0, 0]];
    let mut indices = Vec::with_capacity(usize::from(w) * usize::from(h));

    for y in 0..u32::from(h) {
        for x in 0..u32::from(w) {
            let [r, g, b, a] = image.pixel(x, y).unwrap_or([0, 0, 0, 0]);
            if a < 128 {
                indices.push(0);
                continue;
            }
            let rgb = [r, g, b];
            if let Some(pos) = palette.iter().skip(1).position(|&c| c == rgb) {
                indices.push((pos + 1) as u8);
            } else if palette.len() < 256 {
                let idx = palette.len() as u8;
                palette.push(rgb);
                indices.push(idx);
            } else {
                let mut best_idx = 1u8;
                let mut best_dist = u32::MAX;
                for (i, &c) in palette.iter().enumerate().skip(1) {
                    let dr = i32::from(c[0]) - i32::from(r);
                    let dg = i32::from(c[1]) - i32::from(g);
                    let db = i32::from(c[2]) - i32::from(b);
                    let dist = (dr * dr + dg * dg + db * db) as u32;
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = i as u8;
                    }
                }
                indices.push(best_idx);
            }
        }
    }

    palette.resize(256, [0, 0, 0]);
    for rgb in &palette {
        out.extend_from_slice(rgb);
    }

    out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00]);
    out.push(0x2C);
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.push(0x00);
    out.push(8);

    let mut bit_buf = Vec::new();
    let mut cur_byte = 0u8;
    let mut cur_bit = 0u8;
    let mut push_code = |code: u16| {
        for b in 0..9u32 {
            if ((code >> b) & 1) != 0 {
                cur_byte |= 1 << cur_bit;
            }
            cur_bit += 1;
            if cur_bit == 8 {
                bit_buf.push(cur_byte);
                cur_byte = 0;
                cur_bit = 0;
            }
        }
    };

    for (i, &idx) in indices.iter().enumerate() {
        if i % 250 == 0 {
            push_code(256);
        }
        push_code(u16::from(idx));
    }
    push_code(257);
    if cur_bit > 0 {
        bit_buf.push(cur_byte);
    }

    for chunk in bit_buf.chunks(255) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out.push(0x00);
    out.push(0x3B);
    out
}
