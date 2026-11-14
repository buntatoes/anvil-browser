// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Clean-room W3C WOFF 2.0 (`wOF2`, `0x774F4632`) webfont container unpacker and
//! RFC 7932 Brotli stream decompressor for `anvil-ttf`.

use crate::{MAX_FONT_BYTES, MAX_SFNT_TABLES, TtfError};

/// W3C WOFF 2.0 Section 3.2 Known Table Tags (`0..62`).
const KNOWN_TABLE_TAGS: [[u8; 4]; 63] = [
    *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name", *b"OS/2", *b"post", *b"cvt ",
    *b"fpgm", *b"glyf", *b"loca", *b"prep", *b"CFF ", *b"VORG", *b"EBDT", *b"EBLC", *b"gasp",
    *b"hdmx", *b"kern", *b"LTSH", *b"PCLT", *b"VDMX", *b"vhea", *b"vmtx", *b"BASE", *b"GDEF",
    *b"GPOS", *b"GSUB", *b"EBSC", *b"JSTF", *b"MATH", *b"CBDT", *b"CBLC", *b"COLR", *b"CPAL",
    *b"SVG ", *b"sbix", *b"acnt", *b"avar", *b"bdat", *b"bloc", *b"bsln", *b"cvar", *b"fdsc",
    *b"feat", *b"fmtx", *b"fvar", *b"gvar", *b"hsty", *b"just", *b"lcar", *b"mort", *b"morx",
    *b"opbd", *b"prop", *b"trak", *b"Zapf", *b"Silf", *b"Glat", *b"Gloc", *b"Feat", *b"Sill",
];

const CODE_LENGTH_CODE_ORDER: [usize; 18] =
    [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];

const INSERT_LENGTH_OFFSET: [u32; 24] = [
    0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26, 34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210,
    22594,
];

const INSERT_LENGTH_NBITS: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24,
];

const COPY_LENGTH_OFFSET: [u32; 24] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118,
];

const COPY_LENGTH_NBITS: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24,
];

const INSERT_RANGE_LUT: [u8; 11] = [0, 0, 0, 0, 8, 8, 0, 16, 8, 16, 16];
const COPY_RANGE_LUT: [u8; 11] = [0, 8, 0, 8, 0, 8, 16, 0, 16, 8, 16];

const BLOCK_LEN_OFFSET: [u32; 26] = [
    1, 5, 9, 13, 17, 25, 33, 41, 49, 65, 81, 97, 113, 145, 177, 209, 241, 305, 369, 497, 753, 1265,
    2289, 4337, 8433, 16625,
];

const BLOCK_LEN_NBITS: [u8; 26] = [
    2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 24,
];

const DIST_SHORT_INDEX: [usize; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
const DIST_SHORT_DELTA: [isize; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];

const UTF8_CONTEXT_LUT0: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    8, 12, 16, 12, 12, 20, 12, 16, 24, 28, 12, 12, 32, 12, 36, 12, 44, 44, 44, 44, 44, 44, 44, 44,
    44, 44, 32, 32, 24, 40, 28, 12, 12, 48, 52, 52, 52, 48, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48,
    52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 24, 12, 28, 12, 12, 12, 56, 60, 60, 60, 56, 60, 60,
    60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 24, 12, 28, 12, 0,
    0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
    0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
    2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3,
    2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
];

const UTF8_CONTEXT_LUT1: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1,
    1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1,
    1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
];

const SIGNED_CONTEXT_LUT: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7,
];

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_offset: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, n: u8) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        let mut val = 0u32;
        let mut shift = 0u8;
        let mut rem = n;
        while rem > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }
            let avail = 8 - self.bit_offset;
            let take = rem.min(avail);
            let mask = ((1u16 << take) - 1) as u8;
            let bits = (self.data[self.byte_pos] >> self.bit_offset) & mask;
            val |= (bits as u32) << shift;
            shift += take;
            rem -= take;
            self.bit_offset += take;
            if self.bit_offset == 8 {
                self.bit_offset = 0;
                self.byte_pos += 1;
            }
        }
        Some(val)
    }

    fn align_to_byte(&mut self) -> Option<()> {
        if self.bit_offset != 0 {
            let pad = self.read_bits(8 - self.bit_offset)?;
            if pad != 0 {
                return None;
            }
        }
        Some(())
    }

    fn read_aligned_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        self.align_to_byte()?;
        if self.byte_pos.saturating_add(len) > self.data.len() {
            return None;
        }
        let slice = &self.data[self.byte_pos..self.byte_pos + len];
        self.byte_pos += len;
        Some(slice)
    }
}

#[derive(Clone)]
struct PrefixTree {
    entries: Vec<(u16, u8, u16)>,
    max_len: u8,
}

impl PrefixTree {
    fn single(sym: u16) -> Self {
        Self {
            entries: vec![(0, 0, sym)],
            max_len: 0,
        }
    }

    fn from_code_lengths(lengths: &[u8]) -> Option<Self> {
        let mut non_zero = Vec::new();
        let mut max_len = 0u8;
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                if len > 15 {
                    return None;
                }
                non_zero.push((sym as u16, len));
                if len > max_len {
                    max_len = len;
                }
            }
        }
        if non_zero.is_empty() {
            return None;
        }
        if non_zero.len() == 1 {
            return Some(Self::single(non_zero[0].0));
        }

        let mut bl_count = [0u32; 16];
        for &(_, len) in &non_zero {
            bl_count[len as usize] += 1;
        }
        let mut next_code = [0u16; 16];
        let mut code = 0u32;
        for bits in 1..=15 {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code as u16;
        }

        let mut entries = Vec::with_capacity(non_zero.len());
        for (sym, len) in non_zero {
            let c = next_code[len as usize];
            next_code[len as usize] = c.wrapping_add(1);
            let mut rev = 0u16;
            let mut v = c;
            for _ in 0..len {
                rev = (rev << 1) | (v & 1);
                v >>= 1;
            }
            entries.push((rev, len, sym));
        }
        entries.sort_by_key(|&(_, len, sym)| (len, sym));
        Some(Self { entries, max_len })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Option<u16> {
        if self.max_len == 0 {
            return Some(self.entries[0].2);
        }
        let mut acc = 0u16;
        for len in 1..=self.max_len {
            let bit = br.read_bits(1)? as u16;
            acc |= bit << (len - 1);
            for &(code, elen, sym) in &self.entries {
                if elen == len && code == acc {
                    return Some(sym);
                }
                if elen > len {
                    break;
                }
            }
        }
        None
    }
}

fn alphabet_bit_width(alphabet_size: usize) -> u8 {
    if alphabet_size <= 1 {
        return 0;
    }
    let max_sym = (alphabet_size - 1) as u32;
    (32 - max_sym.leading_zeros()) as u8
}

fn decode_cl_code_length(br: &mut BitReader<'_>) -> Option<u8> {
    let b0 = br.read_bits(1)?;
    let b1 = br.read_bits(1)?;
    match (b0, b1) {
        (0, 0) => Some(0),
        (1, 0) => Some(4),
        (0, 1) => Some(if br.read_bits(1)? == 0 { 3 } else { 2 }),
        (1, 1) => {
            if br.read_bits(1)? == 0 {
                Some(1)
            } else {
                Some(if br.read_bits(1)? == 0 { 5 } else { 0 })
            }
        }
        _ => None,
    }
}

fn decode_prefix_tree(br: &mut BitReader<'_>, alphabet_size: usize) -> Option<PrefixTree> {
    let hskip = br.read_bits(2)?;
    if hskip == 1 {
        let nsym = (br.read_bits(2)? as usize) + 1;
        let width = alphabet_bit_width(alphabet_size);
        let mut syms = [0u16; 4];
        for i in 0..nsym {
            let s = br.read_bits(width)? as usize;
            if s >= alphabet_size || syms[..i].contains(&(s as u16)) {
                return None;
            }
            syms[i] = s as u16;
        }
        let mut lengths = vec![0u8; alphabet_size];
        match nsym {
            1 => return Some(PrefixTree::single(syms[0])),
            2 => {
                lengths[syms[0] as usize] = 1;
                lengths[syms[1] as usize] = 1;
            }
            3 => {
                lengths[syms[0] as usize] = 1;
                lengths[syms[1] as usize] = 2;
                lengths[syms[2] as usize] = 2;
            }
            4 => {
                if br.read_bits(1)? == 0 {
                    for &s in &syms[..4] {
                        lengths[s as usize] = 2;
                    }
                } else {
                    lengths[syms[0] as usize] = 1;
                    lengths[syms[1] as usize] = 2;
                    lengths[syms[2] as usize] = 3;
                    lengths[syms[3] as usize] = 3;
                }
            }
            _ => return None,
        }
        return PrefixTree::from_code_lengths(&lengths);
    }

    let mut cl_lengths = [0u8; 18];
    let mut space = 32i32;
    let mut num_codes = 0usize;
    for &idx in &CODE_LENGTH_CODE_ORDER[(hskip as usize)..] {
        if space <= 0 {
            break;
        }
        let v = decode_cl_code_length(br)?;
        cl_lengths[idx] = v;
        if v != 0 {
            space -= 32 >> v;
            num_codes += 1;
        }
    }
    if num_codes != 1 && space != 0 {
        return None;
    }
    let cl_tree = PrefixTree::from_code_lengths(&cl_lengths)?;

    let mut sym_lengths = vec![0u8; alphabet_size];
    let mut symbol = 0usize;
    let mut prev_code_len = 8u8;
    let mut repeat = 0u32;
    let mut repeat_code_len = 0u8;
    let mut sym_space = 32768i32;

    while symbol < alphabet_size && sym_space > 0 {
        let code_len = cl_tree.decode(br)? as u8;
        if code_len < 16 {
            repeat = 0;
            sym_lengths[symbol] = code_len;
            symbol += 1;
            if code_len != 0 {
                prev_code_len = code_len;
                sym_space -= 32768 >> code_len;
            }
        } else {
            let extra_bits = if code_len == 16 { 2 } else { 3 };
            let new_len = if code_len == 16 { prev_code_len } else { 0 };
            if repeat_code_len != new_len {
                repeat = 0;
                repeat_code_len = new_len;
            }
            let old_repeat = repeat;
            if repeat > 0 {
                repeat = (repeat - 2) << extra_bits;
            }
            repeat += br.read_bits(extra_bits)? + 3;
            let repeat_delta = (repeat - old_repeat) as usize;
            if symbol + repeat_delta > alphabet_size {
                return None;
            }
            for slot in &mut sym_lengths[symbol..symbol + repeat_delta] {
                *slot = repeat_code_len;
            }
            symbol += repeat_delta;
            if repeat_code_len != 0 {
                sym_space -= (repeat_delta as i32) << (15 - repeat_code_len);
            }
        }
    }
    if sym_space != 0 {
        return None;
    }
    PrefixTree::from_code_lengths(&sym_lengths)
}

fn decode_varlen_uint8(br: &mut BitReader<'_>) -> Option<usize> {
    if br.read_bits(1)? == 0 {
        return Some(1);
    }
    let n = br.read_bits(3)? as u8;
    if n == 0 {
        return Some(2);
    }
    let extra = br.read_bits(n)? as usize;
    Some((1usize << n) + extra + 1)
}

fn decode_block_len(br: &mut BitReader<'_>, tree: &PrefixTree) -> Option<u32> {
    let code = tree.decode(br)? as usize;
    if code >= 26 {
        return None;
    }
    let extra = br.read_bits(BLOCK_LEN_NBITS[code])?;
    Some(BLOCK_LEN_OFFSET[code] + extra)
}

fn decode_context_map(br: &mut BitReader<'_>, num_contexts: usize) -> Option<(usize, Vec<u8>)> {
    let ntrees = decode_varlen_uint8(br)?;
    if ntrees == 1 {
        return Some((1, vec![0u8; num_contexts]));
    }
    let use_rle = br.read_bits(1)? != 0;
    let rlemax = if use_rle {
        (br.read_bits(4)? as usize) + 1
    } else {
        0
    };
    let tree = decode_prefix_tree(br, ntrees + rlemax)?;
    let mut map = Vec::with_capacity(num_contexts);
    while map.len() < num_contexts {
        let code = tree.decode(br)? as usize;
        if code == 0 {
            map.push(0);
        } else if code <= rlemax {
            let reps = (1usize << code) + (br.read_bits(code as u8)? as usize);
            if map.len() + reps > num_contexts {
                return None;
            }
            map.resize(map.len() + reps, 0);
        } else {
            map.push((code - rlemax) as u8);
        }
    }
    if br.read_bits(1)? != 0 {
        let mut mtf: [u8; 256] = std::array::from_fn(|i| i as u8);
        for v in &mut map {
            let idx = *v as usize;
            let val = mtf[idx];
            *v = val;
            for j in (1..=idx).rev() {
                mtf[j] = mtf[j - 1];
            }
            mtf[0] = val;
        }
    }
    Some((ntrees, map))
}

fn literal_context_id(mode: u8, p1: u8, p2: u8) -> usize {
    match mode & 3 {
        0 => (p1 & 0x3F) as usize,
        1 => (p1 >> 2) as usize,
        2 => (UTF8_CONTEXT_LUT0[p1 as usize] | UTF8_CONTEXT_LUT1[p2 as usize]) as usize,
        _ => ((SIGNED_CONTEXT_LUT[p1 as usize] << 3) | SIGNED_CONTEXT_LUT[p2 as usize]) as usize,
    }
}

struct BlockSplitState {
    num_types: usize,
    type_tree: Option<PrefixTree>,
    len_tree: Option<PrefixTree>,
    curr_type: usize,
    prev_type: usize,
    block_len: u32,
}

impl BlockSplitState {
    fn decode(br: &mut BitReader<'_>) -> Option<Self> {
        let num_types = decode_varlen_uint8(br)?;
        if num_types == 1 {
            return Some(Self {
                num_types: 1,
                type_tree: None,
                len_tree: None,
                curr_type: 0,
                prev_type: 1,
                block_len: u32::MAX,
            });
        }
        let type_tree = decode_prefix_tree(br, num_types + 2)?;
        let len_tree = decode_prefix_tree(br, 26)?;
        let block_len = decode_block_len(br, &len_tree)?;
        Some(Self {
            num_types,
            type_tree: Some(type_tree),
            len_tree: Some(len_tree),
            curr_type: 0,
            prev_type: 1,
            block_len,
        })
    }

    fn next_command(&mut self, br: &mut BitReader<'_>) -> Option<usize> {
        if self.num_types > 1 {
            if self.block_len == 0 {
                let code = self.type_tree.as_ref()?.decode(br)? as usize;
                let next_type = match code {
                    0 => self.prev_type,
                    1 => (self.curr_type + 1) % self.num_types,
                    c => c - 2,
                };
                self.prev_type = self.curr_type;
                self.curr_type = next_type;
                self.block_len = decode_block_len(br, self.len_tree.as_ref()?)?;
            }
            self.block_len -= 1;
        }
        Some(self.curr_type)
    }
}

fn decode_insert_and_copy(br: &mut BitReader<'_>, cmd_code: u16) -> Option<(usize, usize, bool)> {
    if cmd_code >= 704 {
        return None;
    }
    let range_idx = (cmd_code >> 6) as usize;
    let insert_code = (INSERT_RANGE_LUT[range_idx] as usize) + (((cmd_code >> 3) & 7) as usize);
    let copy_code = (COPY_RANGE_LUT[range_idx] as usize) + ((cmd_code & 7) as usize);
    let insert_len = (INSERT_LENGTH_OFFSET[insert_code]
        + br.read_bits(INSERT_LENGTH_NBITS[insert_code])?) as usize;
    let copy_len =
        (COPY_LENGTH_OFFSET[copy_code] + br.read_bits(COPY_LENGTH_NBITS[copy_code])?) as usize;
    let dist_code_zero = range_idx < 2;
    Some((insert_len, copy_len, dist_code_zero))
}

/// Decompress an RFC 7932 Brotli stream bounded by `max_output` bytes.
pub fn decompress_brotli_woff2(input: &[u8], max_output: usize) -> Option<Vec<u8>> {
    let mut br = BitReader::new(input);
    let wbits = if br.read_bits(1)? == 0 {
        16u8
    } else {
        let n = br.read_bits(3)? as u8;
        if n != 0 {
            17 + n
        } else {
            let m = br.read_bits(3)? as u8;
            if m == 1 {
                return None;
            } else if m != 0 {
                8 + m
            } else {
                17
            }
        }
    };
    let max_window_distance = (1usize << wbits).saturating_sub(16);

    let mut out = Vec::new();
    let mut dist_ring: [usize; 4] = [4, 11, 15, 16];

    loop {
        let is_last = br.read_bits(1)? != 0;
        if is_last && br.read_bits(1)? != 0 {
            break;
        }
        let mnibbles_code = br.read_bits(2)? as u8;
        if mnibbles_code == 3 {
            if br.read_bits(1)? != 0 {
                return None;
            }
            let mbytes_code = br.read_bits(2)? as u8;
            if mbytes_code == 0 {
                br.align_to_byte()?;
            } else {
                let mut mlen_meta = 0usize;
                for i in 0..mbytes_code {
                    let b = br.read_bits(8)? as usize;
                    if i + 1 == mbytes_code && mbytes_code > 1 && b == 0 {
                        return None;
                    }
                    mlen_meta |= b << (i * 8);
                }
                mlen_meta += 1;
                let _ = br.read_aligned_bytes(mlen_meta)?;
            }
            if is_last {
                break;
            }
            continue;
        }

        let mnibbles = mnibbles_code + 4;
        let mut mlen_raw = 0u32;
        for i in 0..mnibbles {
            let nib = br.read_bits(4)?;
            if i + 1 == mnibbles && mnibbles > 4 && nib == 0 {
                return None;
            }
            mlen_raw |= nib << (i * 4);
        }
        let mlen = (mlen_raw as usize) + 1;
        if out.len().saturating_add(mlen) > max_output {
            return None;
        }

        if !is_last && br.read_bits(1)? != 0 {
            let raw = br.read_aligned_bytes(mlen)?;
            out.extend_from_slice(raw);
            continue;
        }

        let mut lit_split = BlockSplitState::decode(&mut br)?;
        let mut cmd_split = BlockSplitState::decode(&mut br)?;
        let mut dist_split = BlockSplitState::decode(&mut br)?;

        let npostfix = br.read_bits(2)? as u8;
        let ndirect = (br.read_bits(4)? as usize) << npostfix;
        let postfix_mask = (1usize << npostfix) - 1;

        let mut context_modes = Vec::with_capacity(lit_split.num_types);
        for _ in 0..lit_split.num_types {
            context_modes.push(br.read_bits(2)? as u8);
        }

        let (ntrees_l, context_map_l) = decode_context_map(&mut br, lit_split.num_types * 64)?;
        let (ntrees_d, context_map_d) = decode_context_map(&mut br, dist_split.num_types * 4)?;

        let mut htrees_l = Vec::with_capacity(ntrees_l);
        for _ in 0..ntrees_l {
            htrees_l.push(decode_prefix_tree(&mut br, 256)?);
        }
        let mut htrees_i = Vec::with_capacity(cmd_split.num_types);
        for _ in 0..cmd_split.num_types {
            htrees_i.push(decode_prefix_tree(&mut br, 704)?);
        }
        let dist_alphabet_size = 16 + ndirect + (48usize << npostfix);
        let mut htrees_d = Vec::with_capacity(ntrees_d);
        for _ in 0..ntrees_d {
            htrees_d.push(decode_prefix_tree(&mut br, dist_alphabet_size)?);
        }

        let meta_end = out.len() + mlen;
        while out.len() < meta_end {
            let btype_i = cmd_split.next_command(&mut br)?;
            let cmd_sym = htrees_i[btype_i].decode(&mut br)?;
            let (insert_len, copy_len, dist_code_zero) = decode_insert_and_copy(&mut br, cmd_sym)?;

            for _ in 0..insert_len {
                if out.len() >= meta_end {
                    break;
                }
                let btype_l = lit_split.next_command(&mut br)?;
                let p1 = out.last().copied().unwrap_or(0);
                let p2 = if out.len() >= 2 {
                    out[out.len() - 2]
                } else {
                    0
                };
                let ctx = literal_context_id(context_modes[btype_l], p1, p2);
                let tree_idx = context_map_l[(btype_l << 6) | ctx] as usize;
                let lit_byte = htrees_l[tree_idx].decode(&mut br)? as u8;
                out.push(lit_byte);
            }

            if out.len() >= meta_end {
                break;
            }

            let dist_code = if dist_code_zero {
                0usize
            } else {
                let btype_d = dist_split.next_command(&mut br)?;
                let copy_ctx = match copy_len {
                    0..=2 => 0,
                    3 => 1,
                    4 => 2,
                    _ => 3,
                };
                let tree_idx = context_map_d[(btype_d << 2) | copy_ctx] as usize;
                htrees_d[tree_idx].decode(&mut br)? as usize
            };

            let distance = if dist_code < 16 {
                let base = dist_ring[DIST_SHORT_INDEX[dist_code]] as isize;
                let d = base + DIST_SHORT_DELTA[dist_code];
                if d <= 0 {
                    return None;
                }
                d as usize
            } else if dist_code < 16 + ndirect {
                dist_code - 15
            } else {
                let dcode = dist_code - 16 - ndirect;
                let postfix = dcode & postfix_mask;
                let hcode = dcode >> npostfix;
                let ndistbits = ((hcode >> 1) + 1) as u8;
                let offset = (((2 + (hcode & 1)) << ndistbits) - 4) << npostfix;
                let extra = br.read_bits(ndistbits)? as usize;
                offset + (extra << npostfix) + postfix + ndirect + 1
            };

            if dist_code != 0 {
                dist_ring[3] = dist_ring[2];
                dist_ring[2] = dist_ring[1];
                dist_ring[1] = dist_ring[0];
                dist_ring[0] = distance;
            }

            let max_allowed_dist = out.len().min(max_window_distance);
            if distance == 0
                || distance > max_allowed_dist
                || out.len().saturating_add(copy_len) > max_output
            {
                return None;
            }

            for _ in 0..copy_len {
                if out.len() >= meta_end {
                    break;
                }
                let b = out[out.len() - distance];
                out.push(b);
            }
        }

        if is_last {
            break;
        }
    }

    Some(out)
}

/// Read a W3C WOFF 2.0 `UIntBase128` variable-length integer.
fn read_uint_base128(data: &[u8], pos: &mut usize) -> Option<u32> {
    let mut accum = 0u32;
    for i in 0..5 {
        if *pos >= data.len() {
            return None;
        }
        let b = data[*pos];
        *pos += 1;
        if i == 0 && b == 0x80 {
            return None;
        }
        if (accum & 0xFE00_0000) != 0 {
            return None;
        }
        accum = (accum << 7) | u32::from(b & 0x7F);
        if (b & 0x80) == 0 {
            return Some(accum);
        }
    }
    None
}

/// Read a W3C WOFF 2.0 `255UShort` variable-length unsigned 16-bit integer.
fn read_255_ushort(data: &[u8], pos: &mut usize) -> Option<u16> {
    let code = *data.get(*pos)?;
    *pos += 1;
    match code {
        253 => {
            let hi = *data.get(*pos)?;
            let lo = *data.get(*pos + 1)?;
            *pos += 2;
            Some(u16::from_be_bytes([hi, lo]))
        }
        254 => {
            let v = *data.get(*pos)?;
            *pos += 1;
            Some(253 * 2 + u16::from(v))
        }
        255 => {
            let v = *data.get(*pos)?;
            *pos += 1;
            Some(253 + u16::from(v))
        }
        c => Some(u16::from(c)),
    }
}

/// Decode one `(dx, dy, on_curve)` coordinate triplet from WOFF 2.0 Section 5.1 Table 1.
fn decode_glyf_triplet(flag: u8, glyph_stream: &[u8], pos: &mut usize) -> Option<(i32, i32, bool)> {
    let on_curve = (flag & 0x80) == 0;
    let f = flag & 0x7F;
    if f < 10 {
        let b = i32::from(*glyph_stream.get(*pos)?);
        *pos += 1;
        let dy = ((i32::from(f) & 14) << 7) + b;
        let dy = if (f & 1) == 0 { -dy } else { dy };
        Some((0, dy, on_curve))
    } else if f < 20 {
        let b = i32::from(*glyph_stream.get(*pos)?);
        *pos += 1;
        let idx = i32::from(f) - 10;
        let dx = ((idx & 14) << 7) + b;
        let dx = if (idx & 1) == 0 { -dx } else { dx };
        Some((dx, 0, on_curve))
    } else if f < 84 {
        let b = i32::from(*glyph_stream.get(*pos)?);
        *pos += 1;
        let idx = i32::from(f) - 20;
        let b0 = (idx & 0x30) + 1 + (b >> 4);
        let b1 = ((idx & 0x0C) << 2) + 1 + (b & 0x0F);
        let dx = if (idx & 2) == 0 { -b0 } else { b0 };
        let dy = if (idx & 1) == 0 { -b1 } else { b1 };
        Some((dx, dy, on_curve))
    } else if f < 120 {
        let b0 = i32::from(*glyph_stream.get(*pos)?);
        let b1 = i32::from(*glyph_stream.get(*pos + 1)?);
        *pos += 2;
        let idx = i32::from(f) - 84;
        let mag_x = ((idx / 12) << 8) + b0 + 1;
        let mag_y = (((idx % 12) >> 2) << 8) + b1 + 1;
        let dx = if (idx & 2) == 0 { -mag_x } else { mag_x };
        let dy = if (idx & 1) == 0 { -mag_y } else { mag_y };
        Some((dx, dy, on_curve))
    } else if f < 124 {
        let b0 = i32::from(*glyph_stream.get(*pos)?);
        let b1 = i32::from(*glyph_stream.get(*pos + 1)?);
        let b2 = i32::from(*glyph_stream.get(*pos + 2)?);
        *pos += 3;
        let idx = i32::from(f) - 120;
        let mag_x = (b0 << 4) + (b1 >> 4) + 1;
        let mag_y = ((b1 & 0x0F) << 8) + b2 + 1;
        let dx = if (idx & 2) == 0 { -mag_x } else { mag_x };
        let dy = if (idx & 1) == 0 { -mag_y } else { mag_y };
        Some((dx, dy, on_curve))
    } else {
        let b0 = i32::from(*glyph_stream.get(*pos)?);
        let b1 = i32::from(*glyph_stream.get(*pos + 1)?);
        let b2 = i32::from(*glyph_stream.get(*pos + 2)?);
        let b3 = i32::from(*glyph_stream.get(*pos + 3)?);
        *pos += 4;
        let idx = i32::from(f) - 124;
        let mag_x = (b0 << 8) + b1 + 1;
        let mag_y = (b2 << 8) + b3 + 1;
        let dx = if (idx & 2) == 0 { -mag_x } else { mag_x };
        let dy = if (idx & 1) == 0 { -mag_y } else { mag_y };
        Some((dx, dy, on_curve))
    }
}

/// Reconstruct TrueType `glyf` and `loca` tables (plus per-glyph `xMin` array) from a WOFF 2.0 Section 5.1 transformed `glyf` stream.
pub(crate) fn reconstruct_woff2_glyf_loca(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<i16>)> {
    if data.len() < 36 {
        return None;
    }
    let option_flags = u16::from_be_bytes([data[2], data[3]]);
    let num_glyphs = u16::from_be_bytes([data[4], data[5]]) as usize;
    let index_format = u16::from_be_bytes([data[6], data[7]]);
    let n_contour_size = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let n_points_size = u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as usize;
    let flag_size = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
    let glyph_size = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;
    let composite_size = u32::from_be_bytes([data[24], data[25], data[26], data[27]]) as usize;
    let bbox_size = u32::from_be_bytes([data[28], data[29], data[30], data[31]]) as usize;
    let instr_size = u32::from_be_bytes([data[32], data[33], data[34], data[35]]) as usize;

    let mut off = 36usize;
    let n_contour_stream = data.get(off..off.checked_add(n_contour_size)?)?;
    off += n_contour_size;
    let n_points_stream = data.get(off..off.checked_add(n_points_size)?)?;
    off += n_points_size;
    let flag_stream = data.get(off..off.checked_add(flag_size)?)?;
    off += flag_size;
    let glyph_stream = data.get(off..off.checked_add(glyph_size)?)?;
    off += glyph_size;
    let composite_stream = data.get(off..off.checked_add(composite_size)?)?;
    off += composite_size;
    let bbox_stream = data.get(off..off.checked_add(bbox_size)?)?;
    off += bbox_size;
    let instr_stream = data.get(off..off.checked_add(instr_size)?)?;
    off += instr_size;

    let overlap_bitmap_len = if (option_flags & 1) != 0 {
        (num_glyphs + 7) >> 3
    } else {
        0
    };
    let overlap_bitmap = if overlap_bitmap_len > 0 {
        data.get(off..off.checked_add(overlap_bitmap_len)?)?
    } else {
        &[][..]
    };

    let bbox_bitmap_len = ((num_glyphs + 31) >> 5) << 2;
    if bbox_stream.len() < bbox_bitmap_len {
        return None;
    }
    let bbox_bitmap = &bbox_stream[..bbox_bitmap_len];
    let mut bbox_pos = bbox_bitmap_len;

    let mut n_contour_pos = 0usize;
    let mut n_points_pos = 0usize;
    let mut flag_pos = 0usize;
    let mut glyph_pos = 0usize;
    let mut comp_pos = 0usize;
    let mut instr_pos = 0usize;

    let mut glyf_out = Vec::new();
    let mut loca_offsets = Vec::with_capacity(num_glyphs + 1);
    let mut x_mins = Vec::with_capacity(num_glyphs);

    for gid in 0..num_glyphs {
        loca_offsets.push(glyf_out.len());
        let nc_hi = *n_contour_stream.get(n_contour_pos)?;
        let nc_lo = *n_contour_stream.get(n_contour_pos + 1)?;
        n_contour_pos += 2;
        let n_contours = i16::from_be_bytes([nc_hi, nc_lo]);
        let has_explicit_bbox = (bbox_bitmap[gid >> 3] & (0x80 >> (gid & 7))) != 0;

        if n_contours == 0 {
            if has_explicit_bbox {
                return None;
            }
            x_mins.push(0);
            continue;
        }

        if n_contours < 0 {
            // Composite glyph
            if !has_explicit_bbox || bbox_pos + 8 > bbox_stream.len() {
                return None;
            }
            let x_min = i16::from_be_bytes([bbox_stream[bbox_pos], bbox_stream[bbox_pos + 1]]);
            let bbox_bytes = &bbox_stream[bbox_pos..bbox_pos + 8];
            bbox_pos += 8;
            x_mins.push(x_min);

            let comp_start = comp_pos;
            let mut have_instructions = false;
            loop {
                let flags = u16::from_be_bytes([
                    *composite_stream.get(comp_pos)?,
                    *composite_stream.get(comp_pos + 1)?,
                ]);
                comp_pos += 2;
                comp_pos = comp_pos.checked_add(2)?; // component glyph index
                let arg_bytes = if (flags & 0x0001) != 0 { 4 } else { 2 };
                comp_pos = comp_pos.checked_add(arg_bytes)?;
                if (flags & 0x0008) != 0 {
                    comp_pos = comp_pos.checked_add(2)?;
                } else if (flags & 0x0040) != 0 {
                    comp_pos = comp_pos.checked_add(4)?;
                } else if (flags & 0x0080) != 0 {
                    comp_pos = comp_pos.checked_add(8)?;
                }
                if (flags & 0x0100) != 0 {
                    have_instructions = true;
                }
                if (flags & 0x0020) == 0 {
                    break;
                }
            }
            let comp_slice = composite_stream.get(comp_start..comp_pos)?;
            let instr_len = if have_instructions {
                read_255_ushort(glyph_stream, &mut glyph_pos)? as usize
            } else {
                0
            };
            let instr_slice = instr_stream.get(instr_pos..instr_pos.checked_add(instr_len)?)?;
            instr_pos += instr_len;

            glyf_out.extend_from_slice(&(-1i16).to_be_bytes());
            glyf_out.extend_from_slice(bbox_bytes);
            glyf_out.extend_from_slice(comp_slice);
            if have_instructions {
                glyf_out.extend_from_slice(&(instr_len as u16).to_be_bytes());
                glyf_out.extend_from_slice(instr_slice);
            }
            while (glyf_out.len() & 3) != 0 {
                glyf_out.push(0);
            }
            continue;
        }

        // Simple glyph (n_contours > 0)
        let nc = n_contours as usize;
        let mut end_pts = Vec::with_capacity(nc);
        let mut total_pts = 0usize;
        for _ in 0..nc {
            let pts = read_255_ushort(n_points_stream, &mut n_points_pos)? as usize;
            total_pts = total_pts.checked_add(pts)?;
            if total_pts == 0 {
                return None;
            }
            end_pts.push((total_pts - 1) as u16);
        }

        let mut xs = Vec::with_capacity(total_pts);
        let mut ys = Vec::with_capacity(total_pts);
        let mut on_curves = Vec::with_capacity(total_pts);
        let mut cur_x = 0i32;
        let mut cur_y = 0i32;
        let mut min_x = i16::MAX;
        let mut min_y = i16::MAX;
        let mut max_x = i16::MIN;
        let mut max_y = i16::MIN;

        for _ in 0..total_pts {
            let f = *flag_stream.get(flag_pos)?;
            flag_pos += 1;
            let (dx, dy, on_curve) = decode_glyf_triplet(f, glyph_stream, &mut glyph_pos)?;
            cur_x = cur_x.wrapping_add(dx);
            cur_y = cur_y.wrapping_add(dy);
            let px = cur_x as i16;
            let py = cur_y as i16;
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
            xs.push(dx as i16);
            ys.push(dy as i16);
            on_curves.push(on_curve);
        }

        if total_pts == 0 {
            min_x = 0;
            min_y = 0;
            max_x = 0;
            max_y = 0;
        }
        let (x_min, y_min, x_max, y_max) = if has_explicit_bbox {
            if bbox_pos + 8 > bbox_stream.len() {
                return None;
            }
            let bx0 = i16::from_be_bytes([bbox_stream[bbox_pos], bbox_stream[bbox_pos + 1]]);
            let by0 = i16::from_be_bytes([bbox_stream[bbox_pos + 2], bbox_stream[bbox_pos + 3]]);
            let bx1 = i16::from_be_bytes([bbox_stream[bbox_pos + 4], bbox_stream[bbox_pos + 5]]);
            let by1 = i16::from_be_bytes([bbox_stream[bbox_pos + 6], bbox_stream[bbox_pos + 7]]);
            bbox_pos += 8;
            (bx0, by0, bx1, by1)
        } else {
            (min_x, min_y, max_x, max_y)
        };
        x_mins.push(x_min);

        let instr_len = read_255_ushort(glyph_stream, &mut glyph_pos)? as usize;
        let instr_slice = instr_stream.get(instr_pos..instr_pos.checked_add(instr_len)?)?;
        instr_pos += instr_len;

        glyf_out.extend_from_slice(&n_contours.to_be_bytes());
        glyf_out.extend_from_slice(&x_min.to_be_bytes());
        glyf_out.extend_from_slice(&y_min.to_be_bytes());
        glyf_out.extend_from_slice(&x_max.to_be_bytes());
        glyf_out.extend_from_slice(&y_max.to_be_bytes());
        for &ep in &end_pts {
            glyf_out.extend_from_slice(&ep.to_be_bytes());
        }
        glyf_out.extend_from_slice(&(instr_len as u16).to_be_bytes());
        glyf_out.extend_from_slice(instr_slice);

        let overlap_simple =
            !overlap_bitmap.is_empty() && (overlap_bitmap[gid >> 3] & (0x80 >> (gid & 7))) != 0;

        // Write uncompressed TrueType point flags (2-byte signed deltas for all coordinates)
        for (i, &is_on_curve) in on_curves.iter().enumerate().take(total_pts) {
            let mut tf = if is_on_curve { 0x01u8 } else { 0x00u8 };
            if i == 0 && overlap_simple {
                tf |= 0x40;
            }
            glyf_out.push(tf);
        }
        for &dx in &xs {
            glyf_out.extend_from_slice(&dx.to_be_bytes());
        }
        for &dy in &ys {
            glyf_out.extend_from_slice(&dy.to_be_bytes());
        }
        while (glyf_out.len() & 3) != 0 {
            glyf_out.push(0);
        }
    }
    loca_offsets.push(glyf_out.len());

    let mut loca_out =
        Vec::with_capacity(loca_offsets.len() * if index_format == 0 { 2 } else { 4 });
    for &off_val in &loca_offsets {
        if index_format == 0 {
            loca_out.extend_from_slice(&((off_val / 2) as u16).to_be_bytes());
        } else {
            loca_out.extend_from_slice(&(off_val as u32).to_be_bytes());
        }
    }

    Some((glyf_out, loca_out, x_mins))
}

/// Reconstruct a WOFF 2.0 Section 5.4 transformed `hmtx` table into a standard SFNT `hmtx` table.
fn reconstruct_woff2_hmtx(
    data: &[u8],
    num_glyphs: usize,
    num_h_metrics: usize,
    x_mins: &[i16],
) -> Option<Vec<u8>> {
    if data.is_empty() || num_h_metrics == 0 || num_h_metrics > num_glyphs {
        return None;
    }
    let flags = data[0];
    if (flags & 0xFC) != 0 {
        return None;
    }
    let has_prop_lsb = (flags & 1) == 0;
    let has_mono_lsb = (flags & 2) == 0;
    let mut pos = 1usize;

    let mut advances = Vec::with_capacity(num_h_metrics);
    for _ in 0..num_h_metrics {
        let hi = *data.get(pos)?;
        let lo = *data.get(pos + 1)?;
        pos += 2;
        advances.push(u16::from_be_bytes([hi, lo]));
    }

    let mut lsbs = Vec::with_capacity(num_glyphs);
    for i in 0..num_h_metrics {
        let lsb = if has_prop_lsb {
            let hi = *data.get(pos)?;
            let lo = *data.get(pos + 1)?;
            pos += 2;
            i16::from_be_bytes([hi, lo])
        } else {
            *x_mins.get(i).unwrap_or(&0)
        };
        lsbs.push(lsb);
    }
    for i in num_h_metrics..num_glyphs {
        let lsb = if has_mono_lsb {
            let hi = *data.get(pos)?;
            let lo = *data.get(pos + 1)?;
            pos += 2;
            i16::from_be_bytes([hi, lo])
        } else {
            *x_mins.get(i).unwrap_or(&0)
        };
        lsbs.push(lsb);
    }

    let mut out = Vec::with_capacity(num_h_metrics * 4 + (num_glyphs - num_h_metrics) * 2);
    for i in 0..num_h_metrics {
        out.extend_from_slice(&advances[i].to_be_bytes());
        out.extend_from_slice(&lsbs[i].to_be_bytes());
    }
    for &lsb in &lsbs[num_h_metrics..] {
        out.extend_from_slice(&lsb.to_be_bytes());
    }
    Some(out)
}

/// Unpack a W3C WOFF 2.0 (`wOF2`, `0x774F4632`) webfont container into a canonical SFNT byte stream.
pub fn unpack_woff2(data: &[u8]) -> Result<Vec<u8>, TtfError> {
    if data.len() < 48 {
        return Err(TtfError::UnexpectedEof);
    }
    let flavor = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if flavor != 0x0001_0000 && flavor != 0x7472_7565 && flavor != 0x4F54_544F {
        return Err(TtfError::InvalidSfntSignature);
    }
    let num_tables = u16::from_be_bytes([data[12], data[13]]) as usize;
    if num_tables == 0 || num_tables > MAX_SFNT_TABLES {
        return Err(TtfError::TooManyTables { count: num_tables });
    }
    let total_comp_size = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;

    struct Woff2DirEntry {
        tag: [u8; 4],
        flags: u8,
        transform_len: usize,
        payload: Vec<u8>,
    }

    let mut pos = 48usize;
    let mut entries = Vec::with_capacity(num_tables);
    let mut total_uncompressed = 0usize;

    for _ in 0..num_tables {
        let flags = *data.get(pos).ok_or(TtfError::UnexpectedEof)?;
        pos += 1;
        let tag_idx = (flags & 0x3F) as usize;
        let tag = if tag_idx < 63 {
            KNOWN_TABLE_TAGS[tag_idx]
        } else {
            let s = data.get(pos..pos + 4).ok_or(TtfError::UnexpectedEof)?;
            pos += 4;
            [s[0], s[1], s[2], s[3]]
        };
        let xform_ver = (flags >> 6) & 0x03;
        let orig_len = read_uint_base128(data, &mut pos).ok_or(TtfError::UnexpectedEof)? as usize;
        let is_glyf_or_loca = &tag == b"glyf" || &tag == b"loca";
        let is_xformed = if is_glyf_or_loca {
            xform_ver == 0
        } else if &tag == b"hmtx" {
            xform_ver == 1
        } else {
            xform_ver != 0
        };
        let transform_len = if is_xformed {
            read_uint_base128(data, &mut pos).ok_or(TtfError::UnexpectedEof)? as usize
        } else {
            orig_len
        };
        if &tag == b"loca" && is_xformed && transform_len != 0 {
            return Err(TtfError::InvalidTable("loca"));
        }
        total_uncompressed =
            total_uncompressed
                .checked_add(transform_len)
                .ok_or(TtfError::FontTooLarge {
                    size: MAX_FONT_BYTES,
                })?;
        if total_uncompressed > MAX_FONT_BYTES {
            return Err(TtfError::FontTooLarge {
                size: total_uncompressed,
            });
        }
        entries.push(Woff2DirEntry {
            tag,
            flags,
            transform_len,
            payload: Vec::new(),
        });
    }

    let comp_end = pos
        .checked_add(total_comp_size)
        .ok_or(TtfError::UnexpectedEof)?;
    let comp_slice = data.get(pos..comp_end).ok_or(TtfError::UnexpectedEof)?;
    let uncompressed = decompress_brotli_woff2(comp_slice, total_uncompressed.max(64))
        .ok_or(TtfError::InvalidSfntSignature)?;
    if uncompressed.len() < total_uncompressed {
        return Err(TtfError::UnexpectedEof);
    }

    let mut u_pos = 0usize;
    for entry in &mut entries {
        let slice = &uncompressed[u_pos..u_pos + entry.transform_len];
        u_pos += entry.transform_len;
        entry.payload = slice.to_vec();
    }

    // Reconstruct transformed glyf + loca if present
    let mut x_mins: Vec<i16> = Vec::new();
    let glyf_idx = entries.iter().position(|e| &e.tag == b"glyf");
    let loca_idx = entries.iter().position(|e| &e.tag == b"loca");
    if let Some(gi) = glyf_idx {
        let xform_ver = (entries[gi].flags >> 6) & 0x03;
        if xform_ver == 0 {
            let (glyf_bytes, loca_bytes, decoded_xmins) =
                reconstruct_woff2_glyf_loca(&entries[gi].payload)
                    .ok_or(TtfError::InvalidTable("glyf"))?;
            entries[gi].payload = glyf_bytes;
            if let Some(li) = loca_idx {
                entries[li].payload = loca_bytes;
            }
            x_mins = decoded_xmins;
        }
    }

    // Reconstruct transformed hmtx if present
    if let Some(hi) = entries.iter().position(|e| &e.tag == b"hmtx") {
        let xform_ver = (entries[hi].flags >> 6) & 0x03;
        if xform_ver == 1 {
            let num_glyphs = entries
                .iter()
                .find(|e| &e.tag == b"maxp" && e.payload.len() >= 6)
                .map(|e| u16::from_be_bytes([e.payload[4], e.payload[5]]) as usize)
                .unwrap_or(x_mins.len());
            let num_h_metrics = entries
                .iter()
                .find(|e| &e.tag == b"hhea" && e.payload.len() >= 36)
                .map(|e| u16::from_be_bytes([e.payload[34], e.payload[35]]) as usize)
                .unwrap_or(1);
            entries[hi].payload =
                reconstruct_woff2_hmtx(&entries[hi].payload, num_glyphs, num_h_metrics, &x_mins)
                    .ok_or(TtfError::InvalidTable("hmtx"))?;
        }
    }

    let mut sfnt_size = 12usize + num_tables * 16;
    for entry in &entries {
        sfnt_size = sfnt_size
            .checked_add((entry.payload.len() + 3) & !3)
            .ok_or(TtfError::FontTooLarge {
                size: MAX_FONT_BYTES,
            })?;
        if sfnt_size > MAX_FONT_BYTES {
            return Err(TtfError::FontTooLarge { size: sfnt_size });
        }
    }

    let mut out = vec![0u8; sfnt_size];
    out[0..4].copy_from_slice(&flavor.to_be_bytes());
    out[4..6].copy_from_slice(&(num_tables as u16).to_be_bytes());
    let mut cur_off = 12 + num_tables * 16;
    for (i, entry) in entries.iter().enumerate() {
        let dir_pos = 12 + i * 16;
        let len = entry.payload.len();
        out[dir_pos..dir_pos + 4].copy_from_slice(&entry.tag);
        out[dir_pos + 4..dir_pos + 8].copy_from_slice(&0u32.to_be_bytes());
        out[dir_pos + 8..dir_pos + 12].copy_from_slice(&(cur_off as u32).to_be_bytes());
        out[dir_pos + 12..dir_pos + 16].copy_from_slice(&(len as u32).to_be_bytes());
        out[cur_off..cur_off + len].copy_from_slice(&entry.payload);
        cur_off += (len + 3) & !3;
    }
    Ok(out)
}
