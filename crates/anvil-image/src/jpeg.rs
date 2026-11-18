// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust JPEG decoder (baseline SOF0, progressive SOF2, 1/3/4-comp YCbCr/CMYK/RGB) & encoder.

use crate::{Image, ImageError, validate_dimensions};

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn cos_table() -> [[f32; 8]; 8] {
    let mut tbl = [[0.0f32; 8]; 8];
    for (u, row) in tbl.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            let angle = ((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0;
            *cell = angle.cos();
        }
    }
    tbl
}

#[inline]
fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

#[derive(Clone)]
struct HuffTable {
    fast: [u16; 256],
    min_code: [i32; 17],
    max_code: [i32; 17],
    val_ptr: [usize; 17],
    syms: Vec<u8>,
}

impl Default for HuffTable {
    fn default() -> Self {
        Self {
            fast: [0; 256],
            min_code: [0; 17],
            max_code: [-1; 17],
            val_ptr: [0; 17],
            syms: Vec::new(),
        }
    }
}

impl HuffTable {
    fn build(counts: &[u8; 16], syms: &[u8]) -> Self {
        let mut tbl = Self {
            fast: [0; 256],
            min_code: [0; 17],
            max_code: [-1; 17],
            val_ptr: [0; 17],
            syms: syms.to_vec(),
        };
        let mut code = 0i32;
        let mut si = 0usize;
        for bits in 1..=16 {
            let count = usize::from(counts[bits - 1]);
            tbl.val_ptr[bits] = si;
            if count > 0 {
                tbl.min_code[bits] = code;
                tbl.max_code[bits] = code + (count as i32) - 1;
                if bits <= 8 {
                    let shift = 8 - bits;
                    for j in 0..count {
                        let sym = syms.get(si + j).copied().unwrap_or(0);
                        let base = (code + j as i32) << shift;
                        let span = 1i32 << shift;
                        for k in 0..span {
                            let idx = (base + k) as usize;
                            if idx < 256 {
                                tbl.fast[idx] = (u16::from(bits as u8) << 8) | u16::from(sym);
                            }
                        }
                    }
                }
                code += count as i32;
                si += count;
            }
            code <<= 1;
        }
        tbl
    }

    fn is_empty(&self) -> bool {
        self.syms.is_empty()
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self {
            data,
            pos,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    fn next_byte_stuffed(&mut self) -> Result<u8, ImageError> {
        if self.pos >= self.data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        let b = self.data[self.pos];
        self.pos += 1;
        if b == 0xFF {
            if self.pos >= self.data.len() {
                return Err(ImageError::UnexpectedEof);
            }
            let next = self.data[self.pos];
            if next == 0x00 {
                self.pos += 1;
            } else if next == 0xD9 {
                // Reached EOI gracefully inside scan tail.
                self.pos -= 1;
                return Ok(0);
            } else if (0xD0..=0xD7).contains(&next) {
                self.pos += 1;
            } else {
                // Reached next marker segment (e.g. progressive multi-scan end).
                self.pos -= 1;
                return Ok(0);
            }
        }
        Ok(b)
    }

    #[inline]
    fn ensure_bits(&mut self, needed: u8) -> Result<(), ImageError> {
        while self.bit_count < needed {
            let b = self.next_byte_stuffed()?;
            self.bit_buf = (self.bit_buf << 8) | u32::from(b);
            self.bit_count += 8;
        }
        Ok(())
    }

    #[inline]
    fn next_bit(&mut self) -> Result<u8, ImageError> {
        if self.bit_count == 0 {
            let b = self.next_byte_stuffed()?;
            self.bit_buf = (self.bit_buf << 8) | u32::from(b);
            self.bit_count = 8;
        }
        self.bit_count -= 1;
        Ok(((self.bit_buf >> self.bit_count) & 1) as u8)
    }

    fn read_bits(&mut self, count: u8) -> Result<i32, ImageError> {
        if count == 0 {
            return Ok(0);
        }
        self.ensure_bits(count)?;
        self.bit_count -= count;
        let mask = (1u32 << count) - 1;
        Ok(((self.bit_buf >> self.bit_count) & mask) as i32)
    }

    fn decode_huff(&mut self, table: &HuffTable) -> Result<u8, ImageError> {
        if table.is_empty() {
            return Err(ImageError::InvalidHeader("missing JPEG Huffman table"));
        }
        if self.bit_count < 8 {
            while self.bit_count < 16 && self.pos < self.data.len() {
                let b = self.next_byte_stuffed()?;
                self.bit_buf = (self.bit_buf << 8) | u32::from(b);
                self.bit_count += 8;
            }
        }
        if self.bit_count >= 8 {
            let top8 = ((self.bit_buf >> (self.bit_count - 8)) & 0xFF) as usize;
            let fast = table.fast[top8];
            if fast != 0 {
                let bits = (fast >> 8) as u8;
                self.bit_count -= bits;
                return Ok((fast & 0xFF) as u8);
            }
        }
        let mut code = 0i32;
        for bits in 1..=16usize {
            let bit = i32::from(self.next_bit()?);
            code = (code << 1) | bit;
            if code <= table.max_code[bits] {
                let idx = table.val_ptr[bits] + (code - table.min_code[bits]) as usize;
                return table
                    .syms
                    .get(idx)
                    .copied()
                    .ok_or(ImageError::InvalidHeader("bad JPEG Huffman index"));
            }
        }
        Err(ImageError::InvalidHeader("invalid JPEG Huffman code"))
    }

    fn align_and_consume_restart(&mut self) {
        self.bit_count = 0;
        self.bit_buf = 0;
        while self.pos + 1 < self.data.len() && self.data[self.pos] == 0xFF {
            let m = self.data[self.pos + 1];
            if m == 0xFF {
                self.pos += 1;
            } else if (0xD0..=0xD7).contains(&m) {
                self.pos += 2;
                break;
            } else {
                break;
            }
        }
    }
}

#[inline]
fn receive_extend(bits: i32, category: u8) -> i32 {
    if category == 0 {
        return 0;
    }
    let half = 1i32 << (category - 1);
    if bits < half {
        bits - ((1i32 << category) - 1)
    } else {
        bits
    }
}

#[derive(Clone, Copy, Default)]
struct Component {
    id: u8,
    h_factor: u8,
    v_factor: u8,
    quant_id: u8,
    dc_table: u8,
    ac_table: u8,
    blocks_stride: usize,
    active_blocks_x: usize,
    active_blocks_y: usize,
    coeffs_offset: usize,
}

fn idct8x8(coeff: &[f32; 64], cos: &[[f32; 8]; 8]) -> [f32; 64] {
    let mut temp = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0f32;
            for u in 0..8 {
                let cu = if u == 0 { INV_SQRT2 } else { 1.0 };
                sum += cu * coeff[y * 8 + u] * cos[u][x];
            }
            temp[y * 8 + x] = sum * 0.5;
        }
    }
    let mut out = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0f32;
            for v in 0..8 {
                let cv = if v == 0 { INV_SQRT2 } else { 1.0 };
                sum += cv * temp[v * 8 + x] * cos[v][y];
            }
            out[y * 8 + x] = (sum * 0.5 + 128.0).clamp(0.0, 255.0);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn decode_progressive_scan(
    data: &[u8],
    pos: &mut usize,
    components: &mut [Component],
    scan_comps: &[usize],
    ss: usize,
    se: usize,
    ah: u8,
    al: u8,
    mcus_x: usize,
    mcus_y: usize,
    dc_huff: &[HuffTable; 4],
    ac_huff: &[HuffTable; 4],
    restart_interval: u16,
    all_coeffs: &mut [i32],
) -> Result<(), ImageError> {
    let mut reader = BitReader::new(data, *pos);
    let mut prev_dc = [0i32; 4];
    let mut mcu_counter = 0u32;
    let mut eob_run = 0u32;

    if scan_comps.len() == 1 {
        let ci = scan_comps[0];
        let comp = components[ci];
        for by in 0..comp.active_blocks_y {
            for bx in 0..comp.active_blocks_x {
                if restart_interval > 0
                    && mcu_counter > 0
                    && mcu_counter.is_multiple_of(u32::from(restart_interval))
                {
                    reader.align_and_consume_restart();
                    prev_dc = [0; 4];
                    eob_run = 0;
                }
                mcu_counter += 1;
                let blk_start = comp.coeffs_offset + (by * comp.blocks_stride + bx) * 64;
                decode_progressive_block(
                    &mut reader,
                    &comp,
                    ci,
                    ss,
                    se,
                    ah,
                    al,
                    dc_huff,
                    ac_huff,
                    &mut prev_dc,
                    &mut eob_run,
                    &mut all_coeffs[blk_start..blk_start + 64],
                )?;
            }
        }
    } else {
        for my in 0..mcus_y {
            for mx in 0..mcus_x {
                if restart_interval > 0
                    && mcu_counter > 0
                    && mcu_counter.is_multiple_of(u32::from(restart_interval))
                {
                    reader.align_and_consume_restart();
                    prev_dc = [0; 4];
                    eob_run = 0;
                }
                mcu_counter += 1;
                for &ci in scan_comps {
                    let comp = components[ci];
                    for vy in 0..usize::from(comp.v_factor) {
                        for hx in 0..usize::from(comp.h_factor) {
                            let by = my * usize::from(comp.v_factor) + vy;
                            let bx = mx * usize::from(comp.h_factor) + hx;
                            let blk_start =
                                comp.coeffs_offset + (by * comp.blocks_stride + bx) * 64;
                            decode_progressive_block(
                                &mut reader,
                                &comp,
                                ci,
                                ss,
                                se,
                                ah,
                                al,
                                dc_huff,
                                ac_huff,
                                &mut prev_dc,
                                &mut eob_run,
                                &mut all_coeffs[blk_start..blk_start + 64],
                            )?;
                        }
                    }
                }
            }
        }
    }

    *pos = reader.pos;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_progressive_block(
    reader: &mut BitReader<'_>,
    comp: &Component,
    ci: usize,
    ss: usize,
    se: usize,
    ah: u8,
    al: u8,
    dc_huff: &[HuffTable; 4],
    ac_huff: &[HuffTable; 4],
    prev_dc: &mut [i32; 4],
    eob_run: &mut u32,
    zz: &mut [i32],
) -> Result<(), ImageError> {
    if ss == 0 {
        if ah == 0 {
            let cat = reader.decode_huff(&dc_huff[usize::from(comp.dc_table)])?;
            if cat > 15 {
                return Err(ImageError::InvalidHeader("JPEG DC category out of range"));
            }
            let bits = reader.read_bits(cat)?;
            let diff = receive_extend(bits, cat);
            prev_dc[ci] = prev_dc[ci].wrapping_add(diff);
            zz[0] = prev_dc[ci] << al;
        } else {
            let bit = i32::from(reader.next_bit()?);
            zz[0] |= bit << al;
        }
        if se == 0 {
            return Ok(());
        }
    }

    let ac_tbl = &ac_huff[usize::from(comp.ac_table)];
    let mut k = ss.max(1);
    if ah == 0 {
        if *eob_run > 0 {
            *eob_run -= 1;
            return Ok(());
        }
        while k <= se {
            let rs = reader.decode_huff(ac_tbl)?;
            let run = usize::from(rs >> 4);
            let size = rs & 0x0F;
            if size == 0 {
                if run == 15 {
                    k += 16;
                    continue;
                }
                let extra = reader.read_bits(run as u8)? as u32;
                *eob_run = ((1u32 << run) + extra) - 1;
                break;
            }
            k += run;
            if k > se {
                break;
            }
            let bits = reader.read_bits(size)?;
            let val = receive_extend(bits, size);
            zz[k] = val << al;
            k += 1;
        }
    } else {
        let p1 = 1i32 << al;
        let m1 = -p1;
        if *eob_run > 0 {
            while k <= se {
                if zz[k] != 0 && reader.next_bit()? != 0 && (zz[k] & p1) == 0 {
                    zz[k] += if zz[k] >= 0 { p1 } else { m1 };
                }
                k += 1;
            }
            *eob_run -= 1;
            return Ok(());
        }
        while k <= se {
            let rs = reader.decode_huff(ac_tbl)?;
            let mut run = i32::from(rs >> 4);
            let size = rs & 0x0F;
            let new_val = if size == 0 {
                if run < 15 {
                    let extra = reader.read_bits(run as u8)? as u32;
                    *eob_run = (1u32 << run) + extra;
                    break;
                }
                0
            } else if size == 1 {
                if reader.next_bit()? != 0 { p1 } else { m1 }
            } else {
                return Err(ImageError::InvalidHeader(
                    "invalid JPEG progressive refinement symbol",
                ));
            };
            while k <= se {
                if zz[k] != 0 {
                    if reader.next_bit()? != 0 && (zz[k] & p1) == 0 {
                        zz[k] += if zz[k] >= 0 { p1 } else { m1 };
                    }
                } else {
                    if run == 0 {
                        zz[k] = new_val;
                        k += 1;
                        break;
                    }
                    run -= 1;
                }
                k += 1;
            }
        }
        if *eob_run > 0 {
            while k <= se {
                if zz[k] != 0 && reader.next_bit()? != 0 && (zz[k] & p1) == 0 {
                    zz[k] += if zz[k] >= 0 { p1 } else { m1 };
                }
                k += 1;
            }
            *eob_run -= 1;
        }
    }
    Ok(())
}

/// Decode a JPEG (SOF0 baseline or SOF2 progressive, Gray/YCbCr/RGB/CMYK) into RGBA.
pub fn decode_jpeg(data: &[u8]) -> Result<Image, ImageError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(ImageError::InvalidHeader("missing JPEG SOI marker"));
    }

    let mut pos = 2usize;
    let mut qtables = [[1u16; 64]; 4];
    let mut dc_huff: [HuffTable; 4] = Default::default();
    let mut ac_huff: [HuffTable; 4] = Default::default();
    let mut width = 0u32;
    let mut height = 0u32;
    let mut is_progressive = false;
    let mut components: Vec<Component> = Vec::new();
    let mut restart_interval = 0u16;
    let mut adobe_transform: Option<u8> = None;

    let mut mcus_x = 0usize;
    let mut mcus_y = 0usize;
    let mut max_h = 1usize;
    let mut max_v = 1usize;
    let mut prog_coeffs: Vec<i32> = Vec::new();
    let cos = cos_table();

    while pos + 2 <= data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        if pos >= data.len() {
            break;
        }
        let marker = data[pos];
        pos += 1;
        if marker == 0xD9 {
            break;
        }
        if marker == 0x00 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        if pos + 2 > data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        let seg_len = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
        if seg_len < 2 || pos + seg_len > data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        let seg = &data[pos + 2..pos + seg_len];
        pos += seg_len;

        match marker {
            0xDB => {
                let mut sp = 0usize;
                while sp < seg.len() {
                    let info = seg[sp];
                    sp += 1;
                    let pq = info >> 4;
                    let tq = usize::from(info & 0x0F);
                    if tq >= 4 {
                        return Err(ImageError::InvalidHeader("invalid JPEG DQT index"));
                    }
                    if pq == 0 {
                        if sp + 64 > seg.len() {
                            return Err(ImageError::UnexpectedEof);
                        }
                        for i in 0..64 {
                            qtables[tq][i] = u16::from(seg[sp + i]);
                        }
                        sp += 64;
                    } else {
                        if sp + 128 > seg.len() {
                            return Err(ImageError::UnexpectedEof);
                        }
                        for i in 0..64 {
                            qtables[tq][i] =
                                u16::from_be_bytes([seg[sp + i * 2], seg[sp + i * 2 + 1]]);
                        }
                        sp += 128;
                    }
                }
            }
            0xEE => {
                if seg.len() >= 12 && &seg[0..5] == b"Adobe" {
                    adobe_transform = Some(seg[11]);
                }
            }
            0xC0 | 0xC2 => {
                is_progressive = marker == 0xC2;
                if seg.len() < 6 {
                    return Err(ImageError::InvalidHeader("short JPEG SOF segment"));
                }
                let precision = seg[0];
                if precision != 8 {
                    return Err(ImageError::UnsupportedFormat("only 8-bit JPEG supported"));
                }
                height = u32::from(u16::from_be_bytes([seg[1], seg[2]]));
                width = u32::from(u16::from_be_bytes([seg[3], seg[4]]));
                let ncomp = usize::from(seg[5]);
                if width == 0
                    || height == 0
                    || !matches!(ncomp, 1 | 3 | 4)
                    || seg.len() < 6 + ncomp * 3
                {
                    return Err(ImageError::InvalidHeader("invalid JPEG SOF dimensions"));
                }
                validate_dimensions(width, height)?;
                components.clear();
                max_h = 1;
                max_v = 1;
                for i in 0..ncomp {
                    let base = 6 + i * 3;
                    let id = seg[base];
                    let samp = seg[base + 1];
                    let qid = (seg[base + 2]).min(3);
                    let h = (samp >> 4).max(1);
                    let v = (samp & 0x0F).max(1);
                    max_h = max_h.max(usize::from(h));
                    max_v = max_v.max(usize::from(v));
                    components.push(Component {
                        id,
                        h_factor: h,
                        v_factor: v,
                        quant_id: qid,
                        ..Default::default()
                    });
                }
                let mcu_w = max_h * 8;
                let mcu_h = max_v * 8;
                mcus_x = (width as usize).div_ceil(mcu_w);
                mcus_y = (height as usize).div_ceil(mcu_h);
                if is_progressive {
                    let mut offset = 0usize;
                    for comp in &mut components {
                        let stride = mcus_x * usize::from(comp.h_factor);
                        let rows = mcus_y * usize::from(comp.v_factor);
                        comp.blocks_stride = stride;
                        comp.active_blocks_x =
                            ((width as usize) * usize::from(comp.h_factor)).div_ceil(max_h * 8);
                        comp.active_blocks_y =
                            ((height as usize) * usize::from(comp.v_factor)).div_ceil(max_v * 8);
                        comp.coeffs_offset = offset;
                        offset = offset.saturating_add(stride * rows * 64);
                    }
                    prog_coeffs = vec![0i32; offset];
                }
            }
            0xC4 => {
                let mut sp = 0usize;
                while sp + 17 <= seg.len() {
                    let info = seg[sp];
                    sp += 1;
                    let tc = info >> 4;
                    let th = usize::from(info & 0x0F);
                    if th >= 4 {
                        return Err(ImageError::InvalidHeader("invalid JPEG DHT index"));
                    }
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[sp..sp + 16]);
                    sp += 16;
                    let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
                    if sp + total > seg.len() {
                        return Err(ImageError::UnexpectedEof);
                    }
                    let syms = &seg[sp..sp + total];
                    sp += total;
                    let tbl = HuffTable::build(&counts, syms);
                    if tc == 0 {
                        dc_huff[th] = tbl;
                    } else {
                        ac_huff[th] = tbl;
                    }
                }
            }
            0xDD => {
                if seg.len() >= 2 {
                    restart_interval = u16::from_be_bytes([seg[0], seg[1]]);
                }
            }
            0xDA => {
                if components.is_empty() || seg.is_empty() {
                    return Err(ImageError::InvalidHeader("JPEG SOS before SOF"));
                }
                let nscan = usize::from(seg[0]);
                if seg.len() < 1 + nscan * 2 + 3 {
                    return Err(ImageError::InvalidHeader("short JPEG SOS segment"));
                }
                let mut scan_comps = Vec::with_capacity(nscan);
                for i in 0..nscan {
                    let cid = seg[1 + i * 2];
                    let tdta = seg[1 + i * 2 + 1];
                    if let Some((idx, comp)) =
                        components.iter_mut().enumerate().find(|(_, c)| c.id == cid)
                    {
                        comp.dc_table = (tdta >> 4) & 0x03;
                        comp.ac_table = tdta & 0x03;
                        scan_comps.push(idx);
                    }
                }
                let ss = usize::from(seg[1 + nscan * 2]);
                let se = usize::from(seg[1 + nscan * 2 + 1]).min(63);
                let ah_al = seg[1 + nscan * 2 + 2];
                let ah = ah_al >> 4;
                let al = ah_al & 0x0F;

                if !is_progressive {
                    return decode_baseline_scan(
                        data,
                        pos,
                        width,
                        height,
                        &components,
                        &qtables,
                        &dc_huff,
                        &ac_huff,
                        restart_interval,
                        adobe_transform,
                        &cos,
                    );
                }

                decode_progressive_scan(
                    data,
                    &mut pos,
                    &mut components,
                    &scan_comps,
                    ss,
                    se,
                    ah,
                    al,
                    mcus_x,
                    mcus_y,
                    &dc_huff,
                    &ac_huff,
                    restart_interval,
                    &mut prog_coeffs,
                )?;
            }
            _ => {}
        }
    }

    if is_progressive && !components.is_empty() {
        return finish_progressive_image(
            width,
            height,
            &components,
            &qtables,
            mcus_x,
            mcus_y,
            max_h,
            max_v,
            adobe_transform,
            &prog_coeffs,
            &cos,
        );
    }

    Err(ImageError::InvalidHeader("missing JPEG scan data"))
}

#[allow(clippy::too_many_arguments)]
fn decode_baseline_scan(
    data: &[u8],
    scan_pos: usize,
    width: u32,
    height: u32,
    components: &[Component],
    qtables: &[[u16; 64]; 4],
    dc_huff: &[HuffTable; 4],
    ac_huff: &[HuffTable; 4],
    restart_interval: u16,
    adobe_transform: Option<u8>,
    cos: &[[f32; 8]; 8],
) -> Result<Image, ImageError> {
    let mut max_h = 1usize;
    let mut max_v = 1usize;
    for c in components {
        max_h = max_h.max(usize::from(c.h_factor));
        max_v = max_v.max(usize::from(c.v_factor));
    }
    let mcu_w = max_h * 8;
    let mcu_h = max_v * 8;
    let mcus_x = (width as usize).div_ceil(mcu_w);
    let mcus_y = (height as usize).div_ceil(mcu_h);

    let mut reader = BitReader::new(data, scan_pos);
    let mut prev_dc = [0i32; 4];
    let mut mcu_counter = 0u32;
    let mut out = Image::new(width, height);
    let rgb_ids = components.len() == 3
        && (adobe_transform == Some(0)
            || (components[0].id == b'R' && components[1].id == b'G' && components[2].id == b'B'));

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart_interval > 0
                && mcu_counter > 0
                && mcu_counter.is_multiple_of(u32::from(restart_interval))
            {
                reader.align_and_consume_restart();
                prev_dc = [0; 4];
            }
            mcu_counter += 1;

            let mut comp_planes: Vec<Vec<[f32; 64]>> = Vec::with_capacity(components.len());
            for (ci, comp) in components.iter().enumerate() {
                let blocks_count = usize::from(comp.h_factor) * usize::from(comp.v_factor);
                let mut blocks = Vec::with_capacity(blocks_count);
                for _ in 0..blocks_count {
                    let dc_cat = reader.decode_huff(&dc_huff[usize::from(comp.dc_table)])?;
                    if dc_cat > 15 {
                        return Err(ImageError::InvalidHeader("JPEG DC category out of range"));
                    }
                    let dc_bits = reader.read_bits(dc_cat)?;
                    prev_dc[ci] = prev_dc[ci].wrapping_add(receive_extend(dc_bits, dc_cat));

                    let mut zz = [0i32; 64];
                    zz[0] = prev_dc[ci];
                    let mut k = 1usize;
                    while k < 64 {
                        let rs = reader.decode_huff(&ac_huff[usize::from(comp.ac_table)])?;
                        if rs == 0x00 {
                            break;
                        }
                        if rs == 0xF0 {
                            k += 16;
                            continue;
                        }
                        let run = usize::from(rs >> 4);
                        let size = rs & 0x0F;
                        k += run;
                        if k >= 64 {
                            break;
                        }
                        let ac_bits = reader.read_bits(size)?;
                        zz[k] = receive_extend(ac_bits, size);
                        k += 1;
                    }

                    let qtable = &qtables[usize::from(comp.quant_id)];
                    let mut coeff = [0.0f32; 64];
                    for i in 0..64 {
                        coeff[ZIGZAG[i]] = (zz[i] as f32) * f32::from(qtable[i]);
                    }
                    blocks.push(idct8x8(&coeff, cos));
                }
                comp_planes.push(blocks);
            }

            write_mcu_pixels(
                &mut out,
                mx,
                my,
                mcu_w,
                mcu_h,
                max_h,
                max_v,
                components,
                &comp_planes,
                rgb_ids,
                adobe_transform,
            );
        }
    }

    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn finish_progressive_image(
    width: u32,
    height: u32,
    components: &[Component],
    qtables: &[[u16; 64]; 4],
    mcus_x: usize,
    mcus_y: usize,
    max_h: usize,
    max_v: usize,
    adobe_transform: Option<u8>,
    prog_coeffs: &[i32],
    cos: &[[f32; 8]; 8],
) -> Result<Image, ImageError> {
    let mcu_w = max_h * 8;
    let mcu_h = max_v * 8;
    let mut out = Image::new(width, height);
    let rgb_ids = components.len() == 3
        && (adobe_transform == Some(0)
            || (components[0].id == b'R' && components[1].id == b'G' && components[2].id == b'B'));

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            let mut comp_planes: Vec<Vec<[f32; 64]>> = Vec::with_capacity(components.len());
            for comp in components {
                let mut blocks =
                    Vec::with_capacity(usize::from(comp.h_factor) * usize::from(comp.v_factor));
                let qtable = &qtables[usize::from(comp.quant_id)];
                for vy in 0..usize::from(comp.v_factor) {
                    for hx in 0..usize::from(comp.h_factor) {
                        let by = my * usize::from(comp.v_factor) + vy;
                        let bx = mx * usize::from(comp.h_factor) + hx;
                        let start = comp.coeffs_offset + (by * comp.blocks_stride + bx) * 64;
                        let zz = &prog_coeffs[start..start + 64];
                        let mut coeff = [0.0f32; 64];
                        for i in 0..64 {
                            coeff[ZIGZAG[i]] = (zz[i] as f32) * f32::from(qtable[i]);
                        }
                        blocks.push(idct8x8(&coeff, cos));
                    }
                }
                comp_planes.push(blocks);
            }
            write_mcu_pixels(
                &mut out,
                mx,
                my,
                mcu_w,
                mcu_h,
                max_h,
                max_v,
                components,
                &comp_planes,
                rgb_ids,
                adobe_transform,
            );
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn write_mcu_pixels(
    out: &mut Image,
    mx: usize,
    my: usize,
    mcu_w: usize,
    mcu_h: usize,
    max_h: usize,
    max_v: usize,
    components: &[Component],
    comp_planes: &[Vec<[f32; 64]>],
    rgb_ids: bool,
    adobe_transform: Option<u8>,
) {
    for py in 0..mcu_h {
        let img_y = my * mcu_h + py;
        if img_y >= out.height as usize {
            continue;
        }
        for px in 0..mcu_w {
            let img_x = mx * mcu_w + px;
            if img_x >= out.width as usize {
                continue;
            }
            let sample = |ci: usize| -> f32 {
                let comp = &components[ci];
                let cx = (px * usize::from(comp.h_factor)) / max_h;
                let cy = (py * usize::from(comp.v_factor)) / max_v;
                let bx = cx / 8;
                let by = cy / 8;
                let sx = cx % 8;
                let sy = cy % 8;
                let b_idx = by * usize::from(comp.h_factor) + bx;
                comp_planes[ci][b_idx][sy * 8 + sx]
            };

            let rgba = match components.len() {
                1 => {
                    let y = clamp_u8(sample(0));
                    [y, y, y, 255]
                }
                3 if rgb_ids => [
                    clamp_u8(sample(0)),
                    clamp_u8(sample(1)),
                    clamp_u8(sample(2)),
                    255,
                ],
                3 => {
                    let y = sample(0);
                    let cb = sample(1) - 128.0;
                    let cr = sample(2) - 128.0;
                    let r = clamp_u8(y + 1.402 * cr);
                    let g = clamp_u8(y - 0.344_136 * cb - 0.714_136 * cr);
                    let b = clamp_u8(y + 1.772 * cb);
                    [r, g, b, 255]
                }
                4 => {
                    let (c, m, y_ch, k) = if adobe_transform == Some(2) {
                        let yv = sample(0);
                        let cb = sample(1) - 128.0;
                        let cr = sample(2) - 128.0;
                        let c = 255.0 - (yv + 1.402 * cr);
                        let m = 255.0 - (yv - 0.344_136 * cb - 0.714_136 * cr);
                        let y_inv = 255.0 - (yv + 1.772 * cb);
                        (c, m, y_inv, sample(3))
                    } else {
                        (sample(0), sample(1), sample(2), sample(3))
                    };
                    let r = clamp_u8(c * k / 255.0);
                    let g = clamp_u8(m * k / 255.0);
                    let b = clamp_u8(y_ch * k / 255.0);
                    [r, g, b, 255]
                }
                _ => [0, 0, 0, 255],
            };
            out.set_pixel(img_x as u32, img_y as u32, rgba);
        }
    }
}

fn fdct8x8(block: &[f32; 64], cos: &[[f32; 8]; 8]) -> [i32; 64] {
    let mut temp = [0.0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let cu = if u == 0 { INV_SQRT2 } else { 1.0 };
            let mut sum = 0.0f32;
            for x in 0..8 {
                sum += (block[y * 8 + x] - 128.0) * cos[u][x];
            }
            temp[y * 8 + u] = 0.5 * cu * sum;
        }
    }
    let mut out = [0i32; 64];
    for v in 0..8 {
        let cv = if v == 0 { INV_SQRT2 } else { 1.0 };
        for u in 0..8 {
            let mut sum = 0.0f32;
            for y in 0..8 {
                sum += temp[y * 8 + u] * cos[v][y];
            }
            out[v * 8 + u] = (0.5 * cv * sum).round() as i32;
        }
    }
    out
}

/// Encode `image` as a baseline SOF0 4:4:4 YCbCr JPEG.
pub fn encode_jpeg(image: &Image) -> Vec<u8> {
    let w = image.width.clamp(1, 65535) as u16;
    let h = image.height.clamp(1, 65535) as u16;
    let cos = cos_table();
    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]);
    out.extend_from_slice(&[0xFF, 0xDB, 0x00, 67, 0x00]);
    out.extend_from_slice(&[1u8; 64]);

    out.extend_from_slice(&[0xFF, 0xC0, 0x00, 17, 8]);
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);

    let dc_counts: [u8; 16] = [0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let dc_syms: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, (19 + dc_syms.len()) as u8, 0x00]);
    out.extend_from_slice(&dc_counts);
    out.extend_from_slice(&dc_syms);

    let mut ac_syms = Vec::with_capacity(162);
    ac_syms.push(0x00);
    ac_syms.push(0xF0);
    for run in 0..16u8 {
        for size in 1..=10u8 {
            ac_syms.push((run << 4) | size);
        }
    }
    let mut ac_counts = [0u8; 16];
    ac_counts[7] = ac_syms.len() as u8;
    let ac_seg_len = (19 + ac_syms.len()) as u16;
    out.extend_from_slice(&[0xFF, 0xC4]);
    out.extend_from_slice(&ac_seg_len.to_be_bytes());
    out.push(0x10);
    out.extend_from_slice(&ac_counts);
    out.extend_from_slice(&ac_syms);

    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 12, 3, 1, 0x00, 2, 0x00, 3, 0x00, 0, 63, 0]);

    let mut cur_byte = 0u8;
    let mut cur_bits = 0u8;
    fn push_bits(out: &mut Vec<u8>, cur_byte: &mut u8, cur_bits: &mut u8, val: u16, n: u8) {
        for i in (0..n).rev() {
            let b = ((val >> i) & 1) as u8;
            *cur_byte = (*cur_byte << 1) | b;
            *cur_bits += 1;
            if *cur_bits == 8 {
                out.push(*cur_byte);
                if *cur_byte == 0xFF {
                    out.push(0x00);
                }
                *cur_byte = 0;
                *cur_bits = 0;
            }
        }
    }

    let encode_mag = |v: i32| -> (u8, u16) {
        if v == 0 {
            return (0, 0);
        }
        let abs_v = v.unsigned_abs().min(1023) as u16;
        let cat = (16 - abs_v.leading_zeros()) as u8;
        let bits = if v > 0 {
            abs_v
        } else {
            abs_v ^ ((1u16 << cat) - 1)
        };
        (cat, bits)
    };

    let mcus_x = u32::from(w).div_ceil(8);
    let mcus_y = u32::from(h).div_ceil(8);
    let mut prev_dc = [0i32; 3];

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            let mut planes = [[0.0f32; 64]; 3];
            for by in 0..8 {
                for bx in 0..8 {
                    let px = (mx * 8 + bx).min(image.width.saturating_sub(1));
                    let py = (my * 8 + by).min(image.height.saturating_sub(1));
                    let [r, g, b, _] = image.pixel(px, py).unwrap_or([0, 0, 0, 255]);
                    let rf = f32::from(r);
                    let gf = f32::from(g);
                    let bf = f32::from(b);
                    let idx = (by * 8 + bx) as usize;
                    planes[0][idx] = (0.299 * rf + 0.587 * gf + 0.114 * bf).clamp(0.0, 255.0);
                    planes[1][idx] =
                        (-0.168_736 * rf - 0.331_264 * gf + 0.5 * bf + 128.0).clamp(0.0, 255.0);
                    planes[2][idx] =
                        (0.5 * rf - 0.418_688 * gf - 0.081_312 * bf + 128.0).clamp(0.0, 255.0);
                }
            }
            for ci in 0..3 {
                let dct = fdct8x8(&planes[ci], &cos);
                let mut zz = [0i32; 64];
                for i in 0..64 {
                    zz[i] = dct[ZIGZAG[i]];
                }
                let diff = (zz[0] - prev_dc[ci]).clamp(-1023, 1023);
                prev_dc[ci] = zz[0];
                let (dc_cat, dc_bits) = encode_mag(diff);
                push_bits(&mut out, &mut cur_byte, &mut cur_bits, u16::from(dc_cat), 4);
                push_bits(&mut out, &mut cur_byte, &mut cur_bits, dc_bits, dc_cat);

                let mut zero_run = 0u8;
                for &ac_val in &zz[1..] {
                    let clamped = ac_val.clamp(-511, 511);
                    if clamped == 0 {
                        zero_run += 1;
                    } else {
                        while zero_run >= 16 {
                            push_bits(&mut out, &mut cur_byte, &mut cur_bits, 1, 8);
                            zero_run -= 16;
                        }
                        let (ac_cat, ac_bits) = encode_mag(clamped);
                        let sym = (zero_run << 4) | ac_cat;
                        let code = ac_syms.iter().position(|&s| s == sym).unwrap_or(0) as u16;
                        push_bits(&mut out, &mut cur_byte, &mut cur_bits, code, 8);
                        push_bits(&mut out, &mut cur_byte, &mut cur_bits, ac_bits, ac_cat);
                        zero_run = 0;
                    }
                }
                if zero_run > 0 {
                    push_bits(&mut out, &mut cur_byte, &mut cur_bits, 0, 8);
                }
            }
        }
    }

    if cur_bits > 0 {
        let pad = 8 - cur_bits;
        push_bits(
            &mut out,
            &mut cur_byte,
            &mut cur_bits,
            (1u16 << pad) - 1,
            pad,
        );
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}
