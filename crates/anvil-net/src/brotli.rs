// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Clean-room RFC 7932 Brotli (`br`) compressed data format decoder and framing encoder.
//!
//! Supports RFC 7932 Section 9.1 window size headers (`WBITS`), Section 9.2 meta-block
//! headers (`ISLAST`, `ISLASTEMPTY`, `MNIBBLES`, `MLEN`, `ISUNCOMPRESSED`, metadata blocks),
//! uncompressed meta-blocks, simple and complex canonical Huffman prefix code trees
//! (Sections 3.4–3.5), context-mode literal decoding (`LSB6`, `MSB6`, `UTF8`, `Signed`),
//! context map RLE + inverse move-to-front (`IMTF`, Section 7.3), insert-and-copy command
//! decoding (Section 5), distance ring buffer short codes and postfix/extra-bit codes
//! (Section 4), and bounded output enforcement ([`CompressError::OutputTooLarge`]).

use crate::anvil_compress::{CompressError, DEFAULT_MAX_DECOMPRESSED_BYTES};

/// Order in which code-length code lengths are read in complex prefix trees (RFC 7932 Section 3.5).
const CODE_LENGTH_CODE_ORDER: [usize; 18] =
    [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Base insert lengths for insert length codes `0..24` (RFC 7932 Section 5, Table 1).
const INSERT_LENGTH_OFFSET: [u32; 24] = [
    0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26, 34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210,
    22594,
];

/// Extra bits for insert length codes `0..24` (RFC 7932 Section 5, Table 1).
const INSERT_LENGTH_NBITS: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24,
];

/// Base copy lengths for copy length codes `0..24` (RFC 7932 Section 5, Table 2).
const COPY_LENGTH_OFFSET: [u32; 24] = [
    2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118,
];

/// Extra bits for copy length codes `0..24` (RFC 7932 Section 5, Table 2).
const COPY_LENGTH_NBITS: [u8; 24] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24,
];

/// Insert range index (`0..3`, multiplied by 8) for each of the 11 command ranges (RFC 7932 Section 5, Table 3).
const INSERT_RANGE_LUT: [u8; 11] = [0, 0, 0, 0, 8, 8, 0, 16, 8, 16, 16];

/// Copy range index (`0..3`, multiplied by 8) for each of the 11 command ranges (RFC 7932 Section 5, Table 3).
const COPY_RANGE_LUT: [u8; 11] = [0, 8, 0, 8, 0, 8, 16, 0, 16, 8, 16];

/// Block count base offsets for block length codes `0..26` (RFC 7932 Section 6).
const BLOCK_LEN_OFFSET: [u32; 26] = [
    1, 5, 9, 13, 17, 25, 33, 41, 49, 65, 81, 97, 113, 145, 177, 209, 241, 305, 369, 497, 753, 1265,
    2289, 4337, 8433, 16625,
];

/// Extra bits for block length codes `0..26` (RFC 7932 Section 6).
const BLOCK_LEN_NBITS: [u8; 26] = [
    2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 24,
];

/// Distance short code index offsets (`0..16`, RFC 7932 Section 4, Table 1).
const DIST_SHORT_INDEX: [usize; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];

/// Distance short code value deltas (`0..16`, RFC 7932 Section 4, Table 1).
const DIST_SHORT_DELTA: [isize; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];

/// Context lookup tables for `UTF8` and `Signed` modes (RFC 7932 Section 7.1.2).
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

/// LSB-first bitstream reader (RFC 7932 Section 1.3).
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

    fn read_bits(&mut self, n: u8) -> Result<u32, CompressError> {
        if n == 0 {
            return Ok(0);
        }
        let mut val = 0u32;
        let mut shift = 0u8;
        let mut rem = n;
        while rem > 0 {
            if self.byte_pos >= self.data.len() {
                return Err(CompressError::UnexpectedEof);
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
        Ok(val)
    }

    fn align_to_byte(&mut self) -> Result<(), CompressError> {
        if self.bit_offset != 0 {
            let pad = self.read_bits(8 - self.bit_offset)?;
            if pad != 0 {
                return Err(CompressError::InvalidHeader(
                    "non-zero padding bits in byte alignment",
                ));
            }
        }
        Ok(())
    }

    fn read_aligned_bytes(&mut self, len: usize) -> Result<&'a [u8], CompressError> {
        self.align_to_byte()?;
        if self.byte_pos.saturating_add(len) > self.data.len() {
            return Err(CompressError::UnexpectedEof);
        }
        let slice = &self.data[self.byte_pos..self.byte_pos + len];
        self.byte_pos += len;
        Ok(slice)
    }
}

/// Canonical prefix tree decoder (RFC 7932 Section 3).
#[derive(Clone, Debug)]
struct PrefixTree {
    /// `(code, len, symbol)` entries sorted by code length, or single-symbol shortcut.
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

    fn from_code_lengths(lengths: &[u8]) -> Result<Self, CompressError> {
        let mut non_zero = Vec::new();
        let mut max_len = 0u8;
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                if len > 15 {
                    return Err(CompressError::InvalidHuffmanTree(
                        "prefix code length exceeds 15",
                    ));
                }
                non_zero.push((sym as u16, len));
                if len > max_len {
                    max_len = len;
                }
            }
        }
        if non_zero.is_empty() {
            return Err(CompressError::InvalidHuffmanTree("empty prefix code tree"));
        }
        if non_zero.len() == 1 {
            return Ok(Self::single(non_zero[0].0));
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
            // Reverse bits for LSB-first bitstream matching
            let rev = reverse_bits(c, len);
            entries.push((rev, len, sym));
        }
        entries.sort_by_key(|&(_, len, sym)| (len, sym));
        Ok(Self { entries, max_len })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, CompressError> {
        if self.max_len == 0 {
            return Ok(self.entries[0].2);
        }
        let mut acc = 0u16;
        for len in 1..=self.max_len {
            let bit = br.read_bits(1)? as u16;
            acc |= bit << (len - 1);
            for &(code, elen, sym) in &self.entries {
                if elen == len && code == acc {
                    return Ok(sym);
                }
                if elen > len {
                    break;
                }
            }
        }
        Err(CompressError::InvalidCode)
    }
}

fn reverse_bits(mut val: u16, len: u8) -> u16 {
    let mut out = 0u16;
    for _ in 0..len {
        out = (out << 1) | (val & 1);
        val >>= 1;
    }
    out
}

fn alphabet_bit_width(alphabet_size: usize) -> u8 {
    if alphabet_size <= 1 {
        return 0;
    }
    let max_sym = (alphabet_size - 1) as u32;
    (32 - max_sym.leading_zeros()) as u8
}

/// Decode a simple or complex prefix code tree for `alphabet_size` symbols (RFC 7932 Sections 3.4–3.5).
fn decode_prefix_tree(
    br: &mut BitReader<'_>,
    alphabet_size: usize,
) -> Result<PrefixTree, CompressError> {
    let hskip = br.read_bits(2)?;
    if hskip == 1 {
        // Simple prefix code (RFC 7932 Section 3.4)
        let nsym = (br.read_bits(2)? as usize) + 1;
        let width = alphabet_bit_width(alphabet_size);
        let mut syms = [0u16; 4];
        for i in 0..nsym {
            let s = br.read_bits(width)? as usize;
            if s >= alphabet_size {
                return Err(CompressError::InvalidHuffmanTree(
                    "simple prefix symbol out of range",
                ));
            }
            for &prev in &syms[..i] {
                if prev == s as u16 {
                    return Err(CompressError::InvalidHuffmanTree(
                        "duplicate symbol in simple prefix code",
                    ));
                }
            }
            syms[i] = s as u16;
        }
        let mut lengths = vec![0u8; alphabet_size];
        match nsym {
            1 => return Ok(PrefixTree::single(syms[0])),
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
                let tree_select = br.read_bits(1)?;
                if tree_select == 0 {
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
            _ => unreachable!(),
        }
        return PrefixTree::from_code_lengths(&lengths);
    }

    // Complex prefix code (RFC 7932 Section 3.5)
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
        return Err(CompressError::InvalidHuffmanTree(
            "invalid code-length prefix tree space",
        ));
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
                return Err(CompressError::InvalidHuffmanTree(
                    "repeat count exceeds alphabet size",
                ));
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
        return Err(CompressError::InvalidHuffmanTree(
            "incomplete symbol prefix tree",
        ));
    }
    PrefixTree::from_code_lengths(&sym_lengths)
}

/// Decode one code-length code length using the fixed prefix table in RFC 7932 Section 3.5.
fn decode_cl_code_length(br: &mut BitReader<'_>) -> Result<u8, CompressError> {
    let b0 = br.read_bits(1)?;
    let b1 = br.read_bits(1)?;
    match (b0, b1) {
        (0, 0) => Ok(0),
        (1, 0) => Ok(4),
        (0, 1) => {
            let b2 = br.read_bits(1)?;
            Ok(if b2 == 0 { 3 } else { 2 })
        }
        (1, 1) => {
            let b2 = br.read_bits(1)?;
            if b2 == 0 {
                Ok(1)
            } else {
                let b3 = br.read_bits(1)?;
                Ok(if b3 == 0 { 5 } else { 0 })
            }
        }
        _ => unreachable!(),
    }
}

/// Decode RFC 7932 variable-length block-type/tree count (`1..=256`).
fn decode_varlen_uint8(br: &mut BitReader<'_>) -> Result<usize, CompressError> {
    if br.read_bits(1)? == 0 {
        return Ok(1);
    }
    let n = br.read_bits(3)? as u8;
    if n == 0 {
        return Ok(2);
    }
    let extra = br.read_bits(n)? as usize;
    Ok((1usize << n) + extra + 1)
}

fn decode_block_len(br: &mut BitReader<'_>, tree: &PrefixTree) -> Result<u32, CompressError> {
    let code = tree.decode(br)? as usize;
    if code >= 26 {
        return Err(CompressError::InvalidCode);
    }
    let extra = br.read_bits(BLOCK_LEN_NBITS[code])?;
    Ok(BLOCK_LEN_OFFSET[code] + extra)
}

/// Decode RFC 7932 Section 7.3 context map.
fn decode_context_map(
    br: &mut BitReader<'_>,
    num_contexts: usize,
) -> Result<(usize, Vec<u8>), CompressError> {
    let ntrees = decode_varlen_uint8(br)?;
    if ntrees == 1 {
        return Ok((1, vec![0u8; num_contexts]));
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
                return Err(CompressError::InvalidHeader(
                    "context map RLE run exceeds size",
                ));
            }
            map.resize(map.len() + reps, 0);
        } else {
            map.push((code - rlemax) as u8);
        }
    }
    if br.read_bits(1)? != 0 {
        // Inverse move-to-front transform (RFC 7932 Section 7.3)
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
    Ok((ntrees, map))
}

fn literal_context_id(mode: u8, p1: u8, p2: u8) -> usize {
    match mode & 3 {
        0 => (p1 & 0x3F) as usize, // LSB6
        1 => (p1 >> 2) as usize,   // MSB6
        2 => (UTF8_CONTEXT_LUT0[p1 as usize] | UTF8_CONTEXT_LUT1[p2 as usize]) as usize, // UTF8
        _ => ((SIGNED_CONTEXT_LUT[p1 as usize] << 3) | SIGNED_CONTEXT_LUT[p2 as usize]) as usize, // Signed
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
    fn decode(br: &mut BitReader<'_>) -> Result<Self, CompressError> {
        let num_types = decode_varlen_uint8(br)?;
        if num_types == 1 {
            return Ok(Self {
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
        Ok(Self {
            num_types,
            type_tree: Some(type_tree),
            len_tree: Some(len_tree),
            curr_type: 0,
            prev_type: 1,
            block_len,
        })
    }

    fn next_command(&mut self, br: &mut BitReader<'_>) -> Result<usize, CompressError> {
        if self.num_types > 1 {
            if self.block_len == 0 {
                let code = self.type_tree.as_ref().unwrap().decode(br)? as usize;
                let next_type = match code {
                    0 => self.prev_type,
                    1 => (self.curr_type + 1) % self.num_types,
                    c => c - 2,
                };
                self.prev_type = self.curr_type;
                self.curr_type = next_type;
                self.block_len = decode_block_len(br, self.len_tree.as_ref().unwrap())?;
            }
            self.block_len -= 1;
        }
        Ok(self.curr_type)
    }
}

/// Decode an insert-and-copy command symbol `0..704` (RFC 7932 Section 5).
fn decode_insert_and_copy(
    br: &mut BitReader<'_>,
    cmd_code: u16,
) -> Result<(usize, usize, bool), CompressError> {
    if cmd_code >= 704 {
        return Err(CompressError::InvalidCode);
    }
    let range_idx = (cmd_code >> 6) as usize;
    let insert_code = (INSERT_RANGE_LUT[range_idx] as usize) + (((cmd_code >> 3) & 7) as usize);
    let copy_code = (COPY_RANGE_LUT[range_idx] as usize) + ((cmd_code & 7) as usize);
    let insert_len = (INSERT_LENGTH_OFFSET[insert_code]
        + br.read_bits(INSERT_LENGTH_NBITS[insert_code])?) as usize;
    let copy_len =
        (COPY_LENGTH_OFFSET[copy_code] + br.read_bits(COPY_LENGTH_NBITS[copy_code])?) as usize;
    let dist_code_zero = range_idx < 2;
    Ok((insert_len, copy_len, dist_code_zero))
}

/// Decompress an RFC 7932 Brotli (`br`) stream with a default 32 MiB output cap.
pub fn decompress_brotli(input: &[u8]) -> Result<Vec<u8>, CompressError> {
    decompress_brotli_bounded(input, DEFAULT_MAX_DECOMPRESSED_BYTES)
}

/// Decompress an RFC 7932 Brotli (`br`) stream, returning [`CompressError::OutputTooLarge`]
/// if the decompressed payload exceeds `max_output` bytes.
pub fn decompress_brotli_bounded(
    input: &[u8],
    max_output: usize,
) -> Result<Vec<u8>, CompressError> {
    let mut br = BitReader::new(input);

    // 1. Decode window bits WBITS (RFC 7932 Section 9.1)
    let wbits = if br.read_bits(1)? == 0 {
        16u8
    } else {
        let n = br.read_bits(3)? as u8;
        if n != 0 {
            17 + n
        } else {
            let m = br.read_bits(3)? as u8;
            if m == 1 {
                return Err(CompressError::InvalidHeader(
                    "reserved WBITS encoding in Brotli header",
                ));
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
        // 2. Meta-block header (RFC 7932 Section 9.2)
        let is_last = br.read_bits(1)? != 0;
        if is_last && br.read_bits(1)? != 0 {
            // ISLASTEMPTY = 1
            break;
        }
        let mnibbles_code = br.read_bits(2)? as u8;
        if mnibbles_code == 3 {
            // Metadata meta-block (MLEN = 0)
            if br.read_bits(1)? != 0 {
                return Err(CompressError::InvalidHeader(
                    "non-zero reserved bit in Brotli metadata block",
                ));
            }
            let mbytes_code = br.read_bits(2)? as u8;
            if mbytes_code == 0 {
                br.align_to_byte()?;
            } else {
                let mut mlen_meta = 0usize;
                for i in 0..mbytes_code {
                    let b = br.read_bits(8)? as usize;
                    if i + 1 == mbytes_code && mbytes_code > 1 && b == 0 {
                        return Err(CompressError::InvalidHeader(
                            "non-minimal metadata length in Brotli block",
                        ));
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
                return Err(CompressError::InvalidHeader(
                    "non-minimal MNIBBLES in Brotli meta-block header",
                ));
            }
            mlen_raw |= nib << (i * 4);
        }
        let mlen = (mlen_raw as usize) + 1;
        if out.len().saturating_add(mlen) > max_output {
            return Err(CompressError::OutputTooLarge { limit: max_output });
        }

        if !is_last {
            let is_uncompressed = br.read_bits(1)? != 0;
            if is_uncompressed {
                let raw = br.read_aligned_bytes(mlen)?;
                out.extend_from_slice(raw);
                continue;
            }
        }

        // 3. Compressed meta-block (RFC 7932 Section 9.2)
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

        // 4. Main insert-and-copy command loop
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
                    return Err(CompressError::InvalidDistance(0));
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
            if distance == 0 || distance > max_allowed_dist {
                return Err(CompressError::InvalidDistance(
                    distance.min(u16::MAX as usize) as u16,
                ));
            }
            if out.len().saturating_add(copy_len) > max_output {
                return Err(CompressError::OutputTooLarge { limit: max_output });
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

    Ok(out)
}

/// Encode `input` into a valid RFC 7932 Brotli (`br`) bitstream.
///
/// Emits a minimal empty last meta-block for empty inputs, or non-last uncompressed
/// meta-blocks followed by a final empty last meta-block (`ISLAST=1, ISLASTEMPTY=1`).
pub fn compress_brotli(input: &[u8]) -> Vec<u8> {
    if input.is_empty() {
        // WBITS=16 (0), ISLAST=1 (1), ISLASTEMPTY=1 (1) -> 0b0000_0110 = 0x06
        return vec![0x06];
    }

    struct BitWriter {
        buf: Vec<u8>,
        cur: u8,
        bit_pos: u8,
    }
    impl BitWriter {
        fn new() -> Self {
            Self {
                buf: Vec::new(),
                cur: 0,
                bit_pos: 0,
            }
        }
        fn write_bits(&mut self, mut val: u32, mut n: u8) {
            while n > 0 {
                let avail = 8 - self.bit_pos;
                let take = n.min(avail);
                let mask = ((1u32 << take) - 1) as u8;
                self.cur |= (val as u8 & mask) << self.bit_pos;
                val >>= take;
                n -= take;
                self.bit_pos += take;
                if self.bit_pos == 8 {
                    self.buf.push(self.cur);
                    self.cur = 0;
                    self.bit_pos = 0;
                }
            }
        }
        fn align_and_write_bytes(&mut self, bytes: &[u8]) {
            if self.bit_pos != 0 {
                self.buf.push(self.cur);
                self.cur = 0;
                self.bit_pos = 0;
            }
            self.buf.extend_from_slice(bytes);
        }
        fn finish(mut self) -> Vec<u8> {
            if self.bit_pos != 0 {
                self.buf.push(self.cur);
            }
            self.buf
        }
    }

    let mut bw = BitWriter::new();
    // WBITS = 16 -> 1 bit (0)
    bw.write_bits(0, 1);

    for chunk in input.chunks(65_536) {
        let mlen_minus_1 = (chunk.len() - 1) as u32;
        // ISLAST = 0
        bw.write_bits(0, 1);
        // MNIBBLES = 4 (code 0) for up to 65536 bytes
        bw.write_bits(0, 2);
        bw.write_bits(mlen_minus_1, 16);
        // ISUNCOMPRESSED = 1
        bw.write_bits(1, 1);
        bw.align_and_write_bytes(chunk);
    }

    // Final empty meta-block: ISLAST = 1, ISLASTEMPTY = 1
    bw.write_bits(1, 1);
    bw.write_bits(1, 1);
    bw.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brotli_empty_and_uncompressed_round_trip() {
        let empty = compress_brotli(b"");
        assert_eq!(decompress_brotli(&empty).unwrap(), b"");

        let sample = b"<!DOCTYPE html><html><body><h1>Anvil Brotli Test</h1></body></html>";
        let encoded = compress_brotli(sample);
        let decoded = decompress_brotli(&encoded).unwrap();
        assert_eq!(decoded, sample);
    }

    #[test]
    fn brotli_prefix_coded_literals_and_lz77_copy_decode_correctly() {
        // Construct a single-symbol compressed meta-block repeating 'A' (0x41) 10 times:
        // WBITS=16 (0), ISLAST=1 (1), ISLASTEMPTY=0 (0), MNIBBLES=4 (00), MLEN-1=9 (0x0009, 16 bits),
        // NBLTYPESL=1 (0), NBLTYPESI=1 (0), NBLTYPESD=1 (0),
        // NPOSTFIX=0 (00), NDIRECT=0 (0000), context_mode[0]=0 (00),
        // NTREESL=1 (0), NTREESD=1 (0),
        // HTREEL[0]: simple prefix code (hskip=1: 01), nsym=1 (00), sym='A' (0x41, 8 bits),
        // HTREEI[0]: simple prefix code (hskip=1: 01), nsym=1 (00), sym=19 (insert 2, copy 8, dist_code_zero=true, 10 bits).
        let mut bits: Vec<(u32, u8)> = vec![
            (0, 1),  // WBITS = 16
            (1, 1),  // ISLAST = 1
            (0, 1),  // ISLASTEMPTY = 0
            (0, 2),  // MNIBBLES = 4
            (9, 16), // MLEN - 1 = 9 (10 bytes total)
            (0, 1),  // NBLTYPESL = 1
            (0, 1),  // NBLTYPESI = 1
            (0, 1),  // NBLTYPESD = 1
            (0, 2),  // NPOSTFIX = 0
            (0, 4),  // NDIRECT = 0
            (0, 2),  // context_mode[0] = LSB6
            (0, 1),  // NTREESL = 1
            (0, 1),  // NTREESD = 1
            // HTREEL[0] (alphabet 256 -> 8 bits): simple, 1 sym = 0x41 ('A')
            (1, 2),
            (0, 2),
            (0x41, 8),
            // HTREEI[0] (alphabet 704 -> 10 bits): simple, 1 sym = 14 (insert_code=1 -> len 1, copy_code=6 -> len 8, range=0 -> dist_code_zero=true)
            // Wait: 1 + 8 = 9, or insert_code=2 (len 2) + copy_code=6 (len 8) = 10 -> sym = (2 << 3) | 6 = 22
            (1, 2),
            (0, 2),
            (22, 10),
            // HTREED[0] (alphabet 64 -> 6 bits): simple, 1 sym = 4 (last_distance - 1 = 4 - 1? No, dist_code_zero uses dist_ring[0] Wait: dist_code=0 is last_distances[0]=4, or sym=0 in HTREED)
            (1, 2),
            (0, 2),
            (0, 6),
        ];
        // Wait: if dist_code_zero=true, dist_code=0 -> dist_ring[0] = 4, which requires 4 literals inserted first!
        // So let's use insert_code=4 (len 4) + copy_code=4 (len 6) = 10 bytes!
        // sym = (4 << 3) | 4 = 36.
        bits[18] = (36, 10);

        let mut raw = Vec::new();
        let mut cur = 0u8;
        let mut pos = 0u8;
        for (mut v, mut n) in bits {
            while n > 0 {
                let take = n.min(8 - pos);
                let mask = ((1u32 << take) - 1) as u8;
                cur |= (v as u8 & mask) << pos;
                v >>= take;
                n -= take;
                pos += take;
                if pos == 8 {
                    raw.push(cur);
                    cur = 0;
                    pos = 0;
                }
            }
        }
        if pos > 0 {
            raw.push(cur);
        }

        let out = decompress_brotli(&raw).unwrap();
        assert_eq!(out, b"AAAAAAAAAA");
    }

    #[test]
    fn brotli_enforces_bounded_output_limit() {
        let payload = vec![b'X'; 1024];
        let encoded = compress_brotli(&payload);
        assert!(matches!(
            decompress_brotli_bounded(&encoded, 512),
            Err(CompressError::OutputTooLarge { limit: 512 })
        ));
    }
}
