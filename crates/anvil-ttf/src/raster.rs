// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust TrueType/OpenType outline decoding (`glyf`, `CFF `), kerning (`kern`, `GPOS`),
//! vertical stem grid fitting, 4x4 coverage rasterizer, and 3x LCD subpixel rasterizer.

use std::collections::HashMap;

use crate::{
    GlyphBitmap, GlyphOutline, MAX_COMPOSITE_GLYPH_DEPTH, MAX_CONTOUR_POINTS, MAX_GLYPH_CONTOURS,
    OutlinePoint, SubpixelGlyphBitmap, TtfError, interpret_type2_charstring_with_subrs,
};

const MAX_GLYPH_BITMAP_SIDE: f32 = 256.0;
const MAX_GLYPH_OFFSET: f32 = 1024.0;

#[inline]
fn read_u16_be(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

#[inline]
fn read_i16_be(data: &[u8], off: usize) -> Option<i16> {
    let s = data.get(off..off + 2)?;
    Some(i16::from_be_bytes([s[0], s[1]]))
}

#[inline]
fn read_u32_be(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Pre-indexed outline storage allowing fast lazy per-glyph decoding.
#[derive(Debug)]
pub(crate) enum OutlineSource {
    Glyf {
        glyf_range: (usize, usize),
        offsets: Vec<usize>,
    },
    Cff {
        charstrings: Vec<(usize, usize)>,
        local_subrs: Vec<(usize, usize)>,
        global_subrs: Vec<(usize, usize)>,
    },
    None,
}

impl OutlineSource {
    pub fn decode_glyph(&self, font_bytes: &[u8], gid: u16) -> Option<GlyphOutline> {
        let idx = usize::from(gid);
        match self {
            Self::Glyf {
                glyf_range,
                offsets,
            } => {
                let glyf = font_bytes.get(glyf_range.0..glyf_range.1)?;
                let start = *offsets.get(idx)?;
                let end = *offsets.get(idx + 1)?;
                if start >= end || end > glyf.len() {
                    return None;
                }
                parse_single_glyf(&glyf[start..end], glyf, offsets, 0)
            }
            Self::Cff {
                charstrings,
                local_subrs,
                global_subrs,
            } => {
                let &(cs_s, cs_e) = charstrings.get(idx)?;
                let prog = font_bytes.get(cs_s..cs_e)?;
                let l_slices: Vec<&[u8]> = local_subrs
                    .iter()
                    .filter_map(|&(s, e)| font_bytes.get(s..e))
                    .collect();
                let g_slices: Vec<&[u8]> = global_subrs
                    .iter()
                    .filter_map(|&(s, e)| font_bytes.get(s..e))
                    .collect();
                interpret_type2_charstring_with_subrs(prog, &l_slices, &g_slices).ok()
            }
            Self::None => None,
        }
    }
}

pub(crate) fn parse_glyf_offsets(
    loca: &[u8],
    num_glyphs: usize,
    index_to_loc_format: i16,
) -> Result<Vec<usize>, TtfError> {
    let mut offsets = vec![0usize; num_glyphs + 1];
    if index_to_loc_format == 0 {
        if loca.len() < (num_glyphs + 1) * 2 {
            return Err(TtfError::InvalidTable("loca"));
        }
        for (i, slot) in offsets.iter_mut().enumerate() {
            let word = read_u16_be(loca, i * 2).ok_or(TtfError::InvalidTable("loca"))?;
            *slot = usize::from(word) * 2;
        }
    } else {
        if loca.len() < (num_glyphs + 1) * 4 {
            return Err(TtfError::InvalidTable("loca"));
        }
        for (i, slot) in offsets.iter_mut().enumerate() {
            let byte_off = read_u32_be(loca, i * 4).ok_or(TtfError::InvalidTable("loca"))?;
            *slot = byte_off as usize;
        }
    }
    Ok(offsets)
}

fn parse_single_glyf(
    slice: &[u8],
    glyf: &[u8],
    offsets: &[usize],
    depth: u8,
) -> Option<GlyphOutline> {
    if depth > MAX_COMPOSITE_GLYPH_DEPTH || slice.len() < 10 {
        return None;
    }
    let num_contours = read_i16_be(slice, 0)?;
    let x_min = read_i16_be(slice, 2)?;
    let y_min = read_i16_be(slice, 4)?;
    let x_max = read_i16_be(slice, 6)?;
    let y_max = read_i16_be(slice, 8)?;
    if num_contours == 0 {
        return None;
    }
    if num_contours > 0 {
        if (num_contours as usize) > MAX_GLYPH_CONTOURS {
            return None;
        }
        let contours = parse_simple_contours(&slice[10..], num_contours as usize)?;
        Some(GlyphOutline {
            x_min,
            y_min,
            x_max,
            y_max,
            contours,
        })
    } else {
        let contours = parse_compound_contours(&slice[10..], glyf, offsets, depth + 1);
        Some(GlyphOutline {
            x_min,
            y_min,
            x_max,
            y_max,
            contours,
        })
    }
}

fn parse_simple_contours(data: &[u8], num_contours: usize) -> Option<Vec<Vec<OutlinePoint>>> {
    if num_contours > MAX_GLYPH_CONTOURS || data.len() < num_contours * 2 + 2 {
        return None;
    }
    let mut end_pts = Vec::with_capacity(num_contours);
    for i in 0..num_contours {
        end_pts.push(usize::from(read_u16_be(data, i * 2)?));
    }
    let total_pts = end_pts.last().copied()?.checked_add(1)?;
    if total_pts == 0 || total_pts > MAX_CONTOUR_POINTS {
        return None;
    }
    let insn_len = usize::from(read_u16_be(data, num_contours * 2)?);
    let mut pos = num_contours * 2 + 2 + insn_len;
    if pos > data.len() {
        return None;
    }

    let mut flags = Vec::with_capacity(total_pts);
    while flags.len() < total_pts {
        let f = *data.get(pos)?;
        pos += 1;
        flags.push(f);
        if (f & 0x08) != 0 {
            let repeat = usize::from(*data.get(pos)?);
            pos += 1;
            for _ in 0..repeat {
                if flags.len() >= total_pts {
                    break;
                }
                flags.push(f);
            }
        }
    }

    let mut xs = Vec::with_capacity(total_pts);
    let mut cur_x = 0i16;
    for &f in &flags {
        let short = (f & 0x02) != 0;
        let same_or_pos = (f & 0x10) != 0;
        let dx = if short {
            let b = i16::from(*data.get(pos)?);
            pos += 1;
            if same_or_pos { b } else { -b }
        } else if same_or_pos {
            0
        } else {
            let v = read_i16_be(data, pos)?;
            pos += 2;
            v
        };
        cur_x = cur_x.wrapping_add(dx);
        xs.push(cur_x);
    }

    let mut ys = Vec::with_capacity(total_pts);
    let mut cur_y = 0i16;
    for &f in &flags {
        let short = (f & 0x04) != 0;
        let same_or_pos = (f & 0x20) != 0;
        let dy = if short {
            let b = i16::from(*data.get(pos)?);
            pos += 1;
            if same_or_pos { b } else { -b }
        } else if same_or_pos {
            0
        } else {
            let v = read_i16_be(data, pos)?;
            pos += 2;
            v
        };
        cur_y = cur_y.wrapping_add(dy);
        ys.push(cur_y);
    }

    let mut contours = Vec::with_capacity(num_contours);
    let mut start = 0usize;
    for end in end_pts {
        if end >= total_pts || end < start {
            break;
        }
        let mut contour = Vec::with_capacity(end - start + 1);
        for idx in start..=end {
            contour.push(OutlinePoint {
                x: xs[idx],
                y: ys[idx],
                on_curve: (flags[idx] & 0x01) != 0,
            });
        }
        contours.push(contour);
        start = end + 1;
    }
    Some(contours)
}

fn parse_compound_contours(
    data: &[u8],
    glyf: &[u8],
    offsets: &[usize],
    depth: u8,
) -> Vec<Vec<OutlinePoint>> {
    let mut pos = 0usize;
    let mut all_contours = Vec::new();
    while pos + 4 <= data.len() {
        let (Some(flags), Some(comp_gid_raw)) =
            (read_u16_be(data, pos), read_u16_be(data, pos + 2))
        else {
            break;
        };
        pos += 4;
        let comp_gid = usize::from(comp_gid_raw);
        let args_words = (flags & 0x0001) != 0;
        let args_xy = (flags & 0x0002) != 0;
        let mut dx = 0i16;
        let mut dy = 0i16;
        if args_words {
            if pos + 4 > data.len() {
                break;
            }
            if let (Some(a1), Some(a2)) = (read_i16_be(data, pos), read_i16_be(data, pos + 2))
                && args_xy
            {
                dx = a1;
                dy = a2;
            }
            pos += 4;
        } else {
            if pos + 2 > data.len() {
                break;
            }
            if args_xy {
                dx = i16::from(data[pos] as i8);
                dy = i16::from(data[pos + 1] as i8);
            }
            pos += 2;
        }

        let (mut a, mut b, mut c, mut d) = (1.0f64, 0.0f64, 0.0f64, 1.0f64);
        if (flags & 0x0008) != 0 {
            if let Some(s_raw) = read_i16_be(data, pos) {
                let s = f64::from(s_raw) / 16384.0;
                a = s;
                d = s;
            }
            pos += 2;
        } else if (flags & 0x0040) != 0 {
            if let (Some(sx), Some(sy)) = (read_i16_be(data, pos), read_i16_be(data, pos + 2)) {
                a = f64::from(sx) / 16384.0;
                d = f64::from(sy) / 16384.0;
            }
            pos += 4;
        } else if (flags & 0x0080) != 0 {
            if let (Some(ar), Some(br), Some(cr), Some(dr)) = (
                read_i16_be(data, pos),
                read_i16_be(data, pos + 2),
                read_i16_be(data, pos + 4),
                read_i16_be(data, pos + 6),
            ) {
                a = f64::from(ar) / 16384.0;
                b = f64::from(br) / 16384.0;
                c = f64::from(cr) / 16384.0;
                d = f64::from(dr) / 16384.0;
            }
            pos += 8;
        }

        if comp_gid + 1 < offsets.len() {
            let cs = offsets[comp_gid];
            let ce = offsets[comp_gid + 1];
            if cs < ce
                && ce <= glyf.len()
                && let Some(sub) = parse_single_glyf(&glyf[cs..ce], glyf, offsets, depth)
            {
                for contour in sub.contours {
                    let transformed: Vec<OutlinePoint> = contour
                        .into_iter()
                        .map(|pt| {
                            let px = f64::from(pt.x);
                            let py = f64::from(pt.y);
                            let tx = a * px + c * py + f64::from(dx);
                            let ty = b * px + d * py + f64::from(dy);
                            OutlinePoint {
                                x: tx.round().clamp(-32768.0, 32767.0) as i16,
                                y: ty.round().clamp(-32768.0, 32767.0) as i16,
                                on_curve: pt.on_curve,
                            }
                        })
                        .collect();
                    all_contours.push(transformed);
                }
            }
        }
        if (flags & 0x0020) == 0 {
            break;
        }
    }
    all_contours
}

// ---------- CFF 1.0 Index & DICT Indexing ----------

fn read_cff_offset(cff: &[u8], start: usize, off_size: usize) -> Option<usize> {
    let slice = cff.get(start..start + off_size)?;
    match off_size {
        1 => Some(usize::from(slice[0])),
        2 => Some(usize::from(u16::from_be_bytes([slice[0], slice[1]]))),
        3 => Some(
            (usize::from(slice[0]) << 16) | (usize::from(slice[1]) << 8) | usize::from(slice[2]),
        ),
        4 => Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as usize),
        _ => None,
    }
}

fn parse_cff_index_ranges(
    cff: &[u8],
    base_off: usize,
    start: usize,
) -> Option<(Vec<(usize, usize)>, usize)> {
    let count = usize::from(read_u16_be(cff, start)?);
    if count == 0 {
        return Some((Vec::new(), start + 2));
    }
    let off_size = usize::from(*cff.get(start + 2)?);
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let off_array_start = start + 3;
    let off_array_len = (count + 1).checked_mul(off_size)?;
    let data_base = off_array_start + off_array_len - 1;
    if off_array_start + off_array_len > cff.len() {
        return None;
    }
    let mut rel_offs = Vec::with_capacity(count + 1);
    for idx in 0..=count {
        let off = read_cff_offset(cff, off_array_start + idx * off_size, off_size)?;
        if off == 0 {
            return None;
        }
        rel_offs.push(off);
    }
    let end_pos = data_base + *rel_offs.last()?;
    if end_pos > cff.len() {
        return None;
    }
    let max_items = count.min(8192);
    let mut ranges = Vec::with_capacity(max_items);
    for idx in 0..max_items {
        let s = base_off + data_base + rel_offs[idx];
        let e = base_off + data_base + rel_offs[idx + 1];
        ranges.push((s, e));
    }
    Some((ranges, end_pos))
}

fn parse_cff_dict_operands(dict: &[u8], target_op: u8) -> Option<Vec<i32>> {
    let mut stack = Vec::new();
    let mut i = 0usize;
    while i < dict.len() {
        let b0 = dict[i];
        i += 1;
        match b0 {
            28 => {
                if i + 2 > dict.len() {
                    break;
                }
                let v = i32::from(i16::from_be_bytes([dict[i], dict[i + 1]]));
                i += 2;
                stack.push(v);
            }
            29 => {
                if i + 4 > dict.len() {
                    break;
                }
                let v = i32::from_be_bytes([dict[i], dict[i + 1], dict[i + 2], dict[i + 3]]);
                i += 4;
                stack.push(v);
            }
            30 => {
                while i < dict.len() {
                    let nib = dict[i];
                    i += 1;
                    if (nib & 0xF0) == 0xF0 || (nib & 0x0F) == 0x0F {
                        break;
                    }
                }
                stack.push(0);
            }
            32..=246 => stack.push(i32::from(b0) - 139),
            247..=250 => {
                if i >= dict.len() {
                    break;
                }
                let b1 = dict[i];
                i += 1;
                stack.push((i32::from(b0) - 247) * 256 + i32::from(b1) + 108);
            }
            251..=254 => {
                if i >= dict.len() {
                    break;
                }
                let b1 = dict[i];
                i += 1;
                stack.push(-((i32::from(b0) - 251) * 256 + i32::from(b1) + 108));
            }
            12 => {
                if i < dict.len() {
                    i += 1;
                }
                stack.clear();
            }
            op if op == target_op => {
                if !stack.is_empty() {
                    return Some(stack);
                }
                stack.clear();
            }
            _ => stack.clear(),
        }
    }
    None
}

pub(crate) fn parse_cff_source(cff: &[u8], base_off: usize) -> Option<OutlineSource> {
    if cff.len() < 4 || cff[0] != 1 {
        return None;
    }
    let hdr_size = usize::from(cff[2]);
    let (_, after_name) = parse_cff_index_ranges(cff, base_off, hdr_size)?;
    let (top_ranges, after_top) = parse_cff_index_ranges(cff, base_off, after_name)?;
    let (td_s, td_e) = *top_ranges.first()?;
    let top_dict = cff.get((td_s - base_off)..(td_e - base_off))?;

    let cs_ops = parse_cff_dict_operands(top_dict, 17)?;
    let cs_off = *cs_ops.last()?;
    if cs_off <= 0 {
        return None;
    }
    let (charstrings, _) = parse_cff_index_ranges(cff, base_off, cs_off as usize)?;
    let global_subrs = parse_cff_index_ranges(cff, base_off, after_top)
        .and_then(|(_, after_str)| parse_cff_index_ranges(cff, base_off, after_str))
        .map(|(g, _)| g)
        .unwrap_or_default();

    let mut local_subrs = Vec::new();
    if let Some(priv_ops) = parse_cff_dict_operands(top_dict, 18)
        && priv_ops.len() >= 2
    {
        let priv_size = priv_ops[priv_ops.len() - 2];
        let priv_off = priv_ops[priv_ops.len() - 1];
        if priv_size > 0
            && priv_off > 0
            && let Some(priv_slice) = cff.get(priv_off as usize..(priv_off + priv_size) as usize)
            && let Some(sub_ops) = parse_cff_dict_operands(priv_slice, 19)
            && let Some(&sub_rel) = sub_ops.last()
            && sub_rel > 0
            && let Some((ls, _)) =
                parse_cff_index_ranges(cff, base_off, (priv_off + sub_rel) as usize)
        {
            local_subrs = ls;
        }
    }

    Some(OutlineSource::Cff {
        charstrings,
        local_subrs,
        global_subrs,
    })
}

// ---------- `kern` + `GPOS` Kerning Parser ----------

pub(crate) fn parse_kerning_pairs(
    kern: Option<&[u8]>,
    gpos: Option<&[u8]>,
) -> HashMap<(u16, u16), i16> {
    let mut pairs = HashMap::new();
    if let Some(k) = kern {
        parse_kern_table(k, &mut pairs);
    }
    if let Some(g) = gpos {
        parse_gpos_table(g, &mut pairs);
    }
    pairs
}

fn parse_kern_table(kern: &[u8], out: &mut HashMap<(u16, u16), i16>) {
    if kern.len() < 4 || read_u16_be(kern, 0) != Some(0) {
        return;
    }
    let Some(n_tables) = read_u16_be(kern, 2).map(usize::from) else {
        return;
    };
    let mut pos = 4usize;
    for _ in 0..n_tables {
        if pos + 6 > kern.len() {
            break;
        }
        let Some(sub_len) = read_u16_be(kern, pos + 2).map(usize::from) else {
            break;
        };
        let Some(coverage) = read_u16_be(kern, pos + 4) else {
            break;
        };
        if (coverage >> 8) == 0
            && pos + 14 <= kern.len()
            && let Some(n_pairs) = read_u16_be(kern, pos + 6).map(usize::from)
        {
            let mut p = pos + 14;
            for _ in 0..n_pairs {
                if p + 6 > kern.len() {
                    break;
                }
                if let (Some(l), Some(r), Some(v)) = (
                    read_u16_be(kern, p),
                    read_u16_be(kern, p + 2),
                    read_i16_be(kern, p + 4),
                ) {
                    out.insert((l, r), v);
                }
                p += 6;
            }
        }
        if sub_len == 0 {
            break;
        }
        pos += sub_len;
    }
}

#[inline]
fn value_record_byte_len(format: u16) -> usize {
    (format.count_ones() as usize) * 2
}

#[inline]
fn x_advance_offset_in_record(format: u16) -> Option<usize> {
    if (format & 0x0004) == 0 {
        return None;
    }
    Some(((format & 0x0003).count_ones() as usize) * 2)
}

fn parse_gpos_coverage(cov: &[u8]) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let Some(format) = read_u16_be(cov, 0) else {
        return out;
    };
    if format == 1 {
        let n = read_u16_be(cov, 2).map(usize::from).unwrap_or(0).min(4096);
        if 4 + n * 2 <= cov.len() {
            for i in 0..n {
                if let Some(gid) = read_u16_be(cov, 4 + i * 2) {
                    out.push((i as u16, gid));
                }
            }
        }
    } else if format == 2 {
        let n = read_u16_be(cov, 2).map(usize::from).unwrap_or(0).min(1024);
        if 4 + n * 6 <= cov.len() {
            for i in 0..n {
                let rec = 4 + i * 6;
                if let (Some(sg), Some(eg), Some(sc)) = (
                    read_u16_be(cov, rec),
                    read_u16_be(cov, rec + 2),
                    read_u16_be(cov, rec + 4),
                ) && eg >= sg
                    && (eg - sg) <= 2048
                {
                    for gid in sg..=eg {
                        out.push((sc + (gid - sg), gid));
                    }
                }
            }
        }
    }
    out
}

fn parse_gpos_class_def(cls: &[u8]) -> HashMap<u16, u16> {
    let mut m = HashMap::new();
    let Some(format) = read_u16_be(cls, 0) else {
        return m;
    };
    if format == 1 {
        if let (Some(sg), Some(cnt)) = (read_u16_be(cls, 2), read_u16_be(cls, 4)) {
            let n = usize::from(cnt).min(4096);
            if 6 + n * 2 <= cls.len() {
                for i in 0..n {
                    if let Some(c) = read_u16_be(cls, 6 + i * 2)
                        && c != 0
                    {
                        m.insert(sg + i as u16, c);
                    }
                }
            }
        }
    } else if format == 2 {
        let n = read_u16_be(cls, 2).map(usize::from).unwrap_or(0).min(1024);
        if 4 + n * 6 <= cls.len() {
            for i in 0..n {
                let rec = 4 + i * 6;
                if let (Some(sg), Some(eg), Some(c)) = (
                    read_u16_be(cls, rec),
                    read_u16_be(cls, rec + 2),
                    read_u16_be(cls, rec + 4),
                ) && c != 0
                    && eg >= sg
                    && (eg - sg) <= 2048
                {
                    for gid in sg..=eg {
                        m.insert(gid, c);
                    }
                }
            }
        }
    }
    m
}

fn parse_gpos_pair_pos_subtable(sub: &[u8], out: &mut HashMap<(u16, u16), i16>) {
    let Some(pos_format) = read_u16_be(sub, 0) else {
        return;
    };
    if pos_format == 1 {
        let (Some(cov_off), Some(vf1), Some(vf2), Some(ps_count)) = (
            read_u16_be(sub, 2).map(usize::from),
            read_u16_be(sub, 4),
            read_u16_be(sub, 6),
            read_u16_be(sub, 8).map(|n| usize::from(n).min(4096)),
        ) else {
            return;
        };
        let Some(x_adv_off) = x_advance_offset_in_record(vf1) else {
            return;
        };
        if cov_off >= sub.len() || 10 + ps_count * 2 > sub.len() {
            return;
        }
        let rec_len = 2 + value_record_byte_len(vf1) + value_record_byte_len(vf2);
        for (cov_idx, left_gid) in parse_gpos_coverage(&sub[cov_off..]) {
            let ci = usize::from(cov_idx);
            if ci >= ps_count {
                continue;
            }
            let Some(ps_off) = read_u16_be(sub, 10 + ci * 2).map(usize::from) else {
                continue;
            };
            let Some(pv_count) = read_u16_be(sub, ps_off).map(|n| usize::from(n).min(2048)) else {
                continue;
            };
            let mut p = ps_off + 2;
            for _ in 0..pv_count {
                if p + rec_len > sub.len() {
                    break;
                }
                if let (Some(right_gid), Some(x_adv)) =
                    (read_u16_be(sub, p), read_i16_be(sub, p + 2 + x_adv_off))
                    && x_adv != 0
                {
                    out.insert((left_gid, right_gid), x_adv);
                }
                p += rec_len;
            }
        }
    } else if pos_format == 2 && sub.len() >= 16 {
        let (
            Some(cov_off),
            Some(vf1),
            Some(vf2),
            Some(cd1_off),
            Some(cd2_off),
            Some(c1_cnt),
            Some(c2_cnt),
        ) = (
            read_u16_be(sub, 2).map(usize::from),
            read_u16_be(sub, 4),
            read_u16_be(sub, 6),
            read_u16_be(sub, 8).map(usize::from),
            read_u16_be(sub, 10).map(usize::from),
            read_u16_be(sub, 12).map(|n| usize::from(n).min(512)),
            read_u16_be(sub, 14).map(|n| usize::from(n).min(512)),
        )
        else {
            return;
        };
        let Some(x_adv_off) = x_advance_offset_in_record(vf1) else {
            return;
        };
        if cov_off >= sub.len() || cd1_off >= sub.len() || cd2_off >= sub.len() {
            return;
        }
        let cell_len = value_record_byte_len(vf1) + value_record_byte_len(vf2);
        if cell_len == 0 || 16 + c1_cnt * c2_cnt * cell_len > sub.len() {
            return;
        }
        let coverage = parse_gpos_coverage(&sub[cov_off..]);
        let class1 = parse_gpos_class_def(&sub[cd1_off..]);
        let class2 = parse_gpos_class_def(&sub[cd2_off..]);
        let mut class2_by_id: HashMap<u16, Vec<u16>> = HashMap::new();
        for (gid, cid) in class2 {
            class2_by_id.entry(cid).or_default().push(gid);
        }
        for (_, left_gid) in coverage {
            let c1 = usize::from(class1.get(&left_gid).copied().unwrap_or(0));
            if c1 >= c1_cnt {
                continue;
            }
            for c2 in 1..c2_cnt {
                let cell_off = 16 + (c1 * c2_cnt + c2) * cell_len;
                if let Some(x_adv) = read_i16_be(sub, cell_off + x_adv_off)
                    && x_adv != 0
                    && let Some(right_gids) = class2_by_id.get(&(c2 as u16))
                {
                    for &right_gid in right_gids {
                        out.insert((left_gid, right_gid), x_adv);
                    }
                }
            }
        }
    }
}

fn parse_gpos_table(gpos: &[u8], out: &mut HashMap<(u16, u16), i16>) {
    if gpos.len() < 10 {
        return;
    }
    let (Some(major), Some(minor), Some(ll_off)) = (
        read_u16_be(gpos, 0),
        read_u16_be(gpos, 2),
        read_u16_be(gpos, 8).map(usize::from),
    ) else {
        return;
    };
    if major != 1 || (minor != 0 && minor != 1) || ll_off + 2 > gpos.len() {
        return;
    }
    let n_lookups = read_u16_be(gpos, ll_off)
        .map(|n| usize::from(n).min(256))
        .unwrap_or(0);
    if ll_off + 2 + n_lookups * 2 > gpos.len() {
        return;
    }
    for i in 0..n_lookups {
        let Some(l_rel) = read_u16_be(gpos, ll_off + 2 + i * 2).map(usize::from) else {
            continue;
        };
        let l_off = ll_off + l_rel;
        if l_off + 6 > gpos.len() {
            continue;
        }
        let (Some(lookup_type), Some(n_subs)) = (
            read_u16_be(gpos, l_off),
            read_u16_be(gpos, l_off + 4).map(|n| usize::from(n).min(128)),
        ) else {
            continue;
        };
        if l_off + 6 + n_subs * 2 > gpos.len() {
            continue;
        }
        for s in 0..n_subs {
            let Some(sub_rel) = read_u16_be(gpos, l_off + 6 + s * 2).map(usize::from) else {
                continue;
            };
            let sub_off = l_off + sub_rel;
            if sub_off >= gpos.len() {
                continue;
            }
            if lookup_type == 2 {
                parse_gpos_pair_pos_subtable(&gpos[sub_off..], out);
            } else if lookup_type == 9
                && sub_off + 8 <= gpos.len()
                && read_u16_be(gpos, sub_off) == Some(1)
                && read_u16_be(gpos, sub_off + 2) == Some(2)
                && let Some(ext_off) = read_u32_be(gpos, sub_off + 4).map(|v| v as usize)
                && sub_off + ext_off < gpos.len()
            {
                parse_gpos_pair_pos_subtable(&gpos[sub_off + ext_off..], out);
            }
        }
    }
}

// ---------- Hinting & Coverage Rasterizers ----------

fn snap_vertical_grid_y(
    cur: OutlinePoint,
    prev: OutlinePoint,
    next: OutlinePoint,
    scale: f32,
    grid_fit: bool,
) -> f32 {
    let raw_y = -f32::from(cur.y) * scale;
    if !grid_fit {
        return raw_y;
    }
    let is_baseline = cur.y == 0;
    let dy_prev = (i32::from(cur.y) - i32::from(prev.y)).abs();
    let dx_prev = (i32::from(cur.x) - i32::from(prev.x)).abs();
    let dy_next = (i32::from(cur.y) - i32::from(next.y)).abs();
    let dx_next = (i32::from(cur.x) - i32::from(next.x)).abs();
    let is_horiz_stem = cur.on_curve
        && ((prev.on_curve && dy_prev <= 2 && dx_prev >= 8)
            || (next.on_curve && dy_next <= 2 && dx_next >= 8));
    if is_baseline || is_horiz_stem {
        raw_y.round()
    } else {
        raw_y
    }
}

fn flatten_contour_opts(
    points: &[OutlinePoint],
    scale: f32,
    x_offset_px: f32,
    grid_fit: bool,
) -> Vec<[f32; 2]> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }
    let mut expanded: Vec<(f32, f32, bool)> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let cur = points[i];
        let next = points[(i + 1) % n];
        let prev = points[(i + n - 1) % n];
        let cx = f32::from(cur.x) * scale + x_offset_px;
        let cy = snap_vertical_grid_y(cur, prev, next, scale, grid_fit);
        expanded.push((cx, cy, cur.on_curve));
        if !cur.on_curve && !next.on_curve {
            let after = points[(i + 2) % n];
            let nx = f32::from(next.x) * scale + x_offset_px;
            let ny = snap_vertical_grid_y(next, cur, after, scale, grid_fit);
            expanded.push(((cx + nx) * 0.5, (cy + ny) * 0.5, true));
        }
    }

    let first_on = expanded.iter().position(|p| p.2).unwrap_or(0);
    let m = expanded.len();
    let mut poly = Vec::with_capacity(m * 2);
    let mut idx = 0usize;
    while idx < m {
        let p0 = expanded[(first_on + idx) % m];
        let p1 = expanded[(first_on + idx + 1) % m];
        if p1.2 {
            poly.push([p0.0, p0.1]);
            idx += 1;
        } else {
            let p2 = expanded[(first_on + idx + 2) % m];
            const STEPS: usize = 6;
            for s in 0..STEPS {
                let t = (s as f32) / (STEPS as f32);
                let inv = 1.0 - t;
                let x = inv * inv * p0.0 + 2.0 * inv * t * p1.0 + t * t * p2.0;
                let y = inv * inv * p0.1 + 2.0 * inv * t * p1.1 + t * t * p2.1;
                poly.push([x, y]);
            }
            idx += 2;
        }
    }
    poly
}

fn pixel_bounds(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Option<(i32, i32, u32, u32)> {
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    let f_left = min_x.floor();
    let f_top = min_y.floor();
    let f_right = max_x.ceil();
    let f_bottom = max_y.ceil();
    let w = f_right - f_left;
    let h = f_bottom - f_top;
    if w < 0.0
        || h < 0.0
        || w > MAX_GLYPH_BITMAP_SIDE
        || h > MAX_GLYPH_BITMAP_SIDE
        || f_left.abs() > MAX_GLYPH_OFFSET
        || f_top.abs() > MAX_GLYPH_OFFSET
    {
        return None;
    }
    Some((
        f_left as i32,
        f_top as i32,
        (w as u32).max(1),
        (h as u32).max(1),
    ))
}

type SegmentBounds = (Vec<([f32; 2], [f32; 2])>, i32, i32, u32, u32);

fn collect_segments(
    outline: &GlyphOutline,
    scale: f32,
    x_offset_px: f32,
    grid_fit: bool,
) -> Option<SegmentBounds> {
    let mut segments = Vec::new();
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for contour in &outline.contours {
        let poly = flatten_contour_opts(contour, scale, x_offset_px, grid_fit);
        if poly.len() < 2 {
            continue;
        }
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            min_x = min_x.min(a[0]).min(b[0]);
            min_y = min_y.min(a[1]).min(b[1]);
            max_x = max_x.max(a[0]).max(b[0]);
            max_y = max_y.max(a[1]).max(b[1]);
            if (a[1] - b[1]).abs() > 1e-5 {
                segments.push((a, b));
            }
        }
    }
    if segments.is_empty() {
        return None;
    }
    let (x0, y0, w, h) = pixel_bounds(min_x, min_y, max_x, max_y)?;
    Some((segments, x0, y0, w, h))
}

pub(crate) fn rasterize_outline_styled(
    outline: &GlyphOutline,
    scale: f32,
    advance_px: f32,
    subpixel_x_phase: u8,
    synthetic_bold: bool,
    synthetic_italic: bool,
    font_size_px: f32,
) -> Option<GlyphBitmap> {
    if outline.contours.is_empty() {
        return None;
    }
    let sheared_outline;
    let eff = if synthetic_italic {
        sheared_outline = GlyphOutline {
            x_min: outline.x_min,
            y_min: outline.y_min,
            x_max: outline.x_max,
            y_max: outline.y_max,
            contours: outline
                .contours
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|pt| OutlinePoint {
                            x: pt.x.saturating_add((f32::from(pt.y) * 0.20).round() as i16),
                            y: pt.y,
                            on_curve: pt.on_curve,
                        })
                        .collect()
                })
                .collect(),
        };
        &sheared_outline
    } else {
        outline
    };

    let phase_off = f32::from(subpixel_x_phase & 3) * 0.25;
    let (segments, x0, y0, mut width, height) = collect_segments(eff, scale, phase_off, true)?;
    let mut coverage = vec![0u8; (width as usize) * (height as usize)];
    const SUB: usize = 4;

    let mut crossings = Vec::new();
    for py in 0..height {
        let mut row_hits = vec![0u8; width as usize];
        for sy in 0..SUB {
            let sample_y = (y0 as f32) + (py as f32) + ((sy as f32) + 0.5) / (SUB as f32);
            crossings.clear();
            for &(a, b) in &segments {
                let (y_min, y_max, dir) = if a[1] < b[1] {
                    (a[1], b[1], 1i32)
                } else {
                    (b[1], a[1], -1i32)
                };
                if sample_y >= y_min && sample_y < y_max {
                    let t = (sample_y - a[1]) / (b[1] - a[1]);
                    crossings.push((a[0] + t * (b[0] - a[0]), dir));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (px, hits) in row_hits.iter_mut().enumerate() {
                for sx in 0..SUB {
                    let sample_x = (x0 as f32) + (px as f32) + ((sx as f32) + 0.5) / (SUB as f32);
                    let mut winding = 0i32;
                    for &(cx, dir) in &crossings {
                        if cx <= sample_x {
                            winding += dir;
                        } else {
                            break;
                        }
                    }
                    if winding != 0 {
                        *hits += 1;
                    }
                }
            }
        }
        let row_base = (py as usize) * (width as usize);
        for (px, &hits) in row_hits.iter().enumerate() {
            coverage[row_base + px] = ((u32::from(hits) * 255 + 8) / 16).min(255) as u8;
        }
    }

    let mut final_adv = advance_px;
    if synthetic_bold && width > 0 && height > 0 && width < 511 {
        let old_w = width as usize;
        let new_w = old_w + 1;
        let h = height as usize;
        let mut bold_cov = vec![0u8; new_w * h];
        for row in 0..h {
            for col in 0..new_w {
                let c0 = if col < old_w {
                    coverage[row * old_w + col]
                } else {
                    0
                };
                let c1 = if col > 0 {
                    ((u16::from(coverage[row * old_w + col - 1]) * 210) / 255) as u8
                } else {
                    0
                };
                bold_cov[row * new_w + col] = c0.max(c1);
            }
        }
        width = new_w as u32;
        final_adv += (font_size_px * 0.04).max(0.5);
        coverage = bold_cov;
    }

    Some(GlyphBitmap {
        width,
        height,
        bearing_x: x0,
        bearing_y: -y0,
        advance_px: final_adv,
        coverage,
    })
}

pub(crate) fn rasterize_outline_subpixel_lcd(
    outline: &GlyphOutline,
    scale: f32,
    advance_px: f32,
) -> Option<SubpixelGlyphBitmap> {
    if outline.contours.is_empty() {
        return None;
    }
    let (segments, x0, y0, width, height) = collect_segments(outline, scale, 0.0, true)?;
    let sub_cols = (width as usize) * 3;
    let mut rgb_coverage = vec![0u8; sub_cols * (height as usize)];
    const SUB_X: usize = 4;
    const SUB_Y: usize = 4;

    let mut crossings = Vec::new();
    for py in 0..height {
        let mut raw_sub = vec![0u8; sub_cols];
        for sy in 0..SUB_Y {
            let sample_y = (y0 as f32) + (py as f32) + ((sy as f32) + 0.5) / (SUB_Y as f32);
            crossings.clear();
            for &(a, b) in &segments {
                let (y_min, y_max, dir) = if a[1] < b[1] {
                    (a[1], b[1], 1i32)
                } else {
                    (b[1], a[1], -1i32)
                };
                if sample_y >= y_min && sample_y < y_max {
                    let t = (sample_y - a[1]) / (b[1] - a[1]);
                    crossings.push((a[0] + t * (b[0] - a[0]), dir));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (col, hits) in raw_sub.iter_mut().enumerate() {
                for sx in 0..SUB_X {
                    let sub_idx = col * SUB_X + sx;
                    let sample_x = (x0 as f32) + ((sub_idx as f32) + 0.5) / ((3 * SUB_X) as f32);
                    let mut winding = 0i32;
                    for &(cx, dir) in &crossings {
                        if cx <= sample_x {
                            winding += dir;
                        } else {
                            break;
                        }
                    }
                    if winding != 0 {
                        *hits += 1;
                    }
                }
            }
        }
        let unfiltered: Vec<u32> = raw_sub
            .iter()
            .map(|&h| ((u32::from(h) * 255 + 8) / 16).min(255))
            .collect();
        let row_base = (py as usize) * sub_cols;
        for col in 0..sub_cols {
            let left = unfiltered[col.saturating_sub(1)];
            let center = unfiltered[col];
            let right = unfiltered[(col + 1).min(sub_cols - 1)];
            rgb_coverage[row_base + col] = ((left + center * 2 + right + 2) / 4).min(255) as u8;
        }
    }

    Some(SubpixelGlyphBitmap {
        width,
        height,
        bearing_x: x0,
        bearing_y: -y0,
        advance_px,
        rgb_coverage,
    })
}
