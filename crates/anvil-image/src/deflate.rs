// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust RFC 1951 DEFLATE, RFC 1950 zlib (Adler-32), and RFC 1952 CRC-32.

use crate::ImageError;

const MAX_BITS: usize = 15;

const LENGTH_BASES: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const DIST_BASES: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            if (c & 1) != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Compute ISO 3309 / RFC 1952 CRC-32 checksum.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc as u8) ^ b) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

/// Compute RFC 1950 Adler-32 rolling checksum.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= MOD_ADLER;
        s2 %= MOD_ADLER;
    }
    (s2 << 16) | s1
}

/// Decompress an RFC 1950 zlib stream bounded to `max_output_bytes`.
pub fn zlib_decompress_bounded(
    input: &[u8],
    max_output_bytes: usize,
) -> Result<Vec<u8>, ImageError> {
    if input.len() < 6 {
        return Err(ImageError::UnexpectedEof);
    }
    let cmf = input[0];
    let flg = input[1];
    let cm = cmf & 0x0F;
    let cinfo = cmf >> 4;
    if cm != 8 || cinfo > 7 || (((u16::from(cmf) << 8) | u16::from(flg)) % 31) != 0 {
        return Err(ImageError::InvalidHeader("invalid zlib header"));
    }
    if (flg & 0x20) != 0 {
        return Err(ImageError::UnsupportedFormat(
            "zlib preset dictionary not supported",
        ));
    }
    let deflate_slice = &input[2..input.len() - 4];
    let out = deflate_decode(deflate_slice, max_output_bytes)?;
    let expected = u32::from_be_bytes(
        input[input.len() - 4..]
            .try_into()
            .map_err(|_| ImageError::UnexpectedEof)?,
    );
    if adler32(&out) != expected {
        return Err(ImageError::InvalidHeader("zlib Adler-32 checksum mismatch"));
    }
    Ok(out)
}

/// Compress `input` into an RFC 1950 zlib stream using DEFLATE fixed Huffman or stored blocks.
pub fn zlib_compress(input: &[u8]) -> Vec<u8> {
    let deflated = deflate_compress(input);
    let mut out = Vec::with_capacity(deflated.len() + 6);
    out.push(0x78);
    out.push(0x9C);
    out.extend_from_slice(&deflated);
    out.extend_from_slice(&adler32(input).to_be_bytes());
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u64,
    bit_cnt: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn ensure_bits(&mut self, n: u8) -> Result<(), ImageError> {
        while self.bit_cnt < n {
            if self.byte_pos >= self.data.len() {
                return Err(ImageError::UnexpectedEof);
            }
            self.bit_buf |= u64::from(self.data[self.byte_pos]) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u8) -> Result<u32, ImageError> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let mask = (1u64 << n) - 1;
        let val = (self.bit_buf & mask) as u32;
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(val)
    }

    fn align_to_byte(&mut self) {
        let drop = self.bit_cnt & 7;
        self.bit_buf >>= drop;
        self.bit_cnt -= drop;
    }

    fn read_u16_le(&mut self) -> Result<u16, ImageError> {
        let lo = self.read_bits(8)? as u16;
        let hi = self.read_bits(8)? as u16;
        Ok(lo | (hi << 8))
    }

    fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, ImageError> {
        self.align_to_byte();
        let mut out = vec![0u8; count];
        let mut idx = 0usize;
        while self.bit_cnt >= 8 && idx < count {
            out[idx] = (self.bit_buf & 0xFF) as u8;
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
            idx += 1;
        }
        let rem = count - idx;
        if self.byte_pos + rem > self.data.len() {
            return Err(ImageError::UnexpectedEof);
        }
        out[idx..].copy_from_slice(&self.data[self.byte_pos..self.byte_pos + rem]);
        self.byte_pos += rem;
        Ok(out)
    }
}

struct HuffmanDecoder {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
    max_len: u8,
}

impl HuffmanDecoder {
    fn build(lengths: &[u8]) -> Result<Self, ImageError> {
        let mut counts = [0u16; MAX_BITS + 1];
        let mut max_len = 0u8;
        for &l in lengths {
            if usize::from(l) > MAX_BITS {
                return Err(ImageError::InvalidHeader("DEFLATE code length > 15"));
            }
            if l > 0 {
                counts[usize::from(l)] += 1;
                if l > max_len {
                    max_len = l;
                }
            }
        }
        let mut offsets = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for bits in 1..=MAX_BITS {
            offsets[bits] = total;
            total = total.saturating_add(counts[bits]);
        }
        let mut symbols = vec![0u16; usize::from(total)];
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                let pos = usize::from(offsets[usize::from(l)]);
                if pos < symbols.len() {
                    symbols[pos] = sym as u16;
                    offsets[usize::from(l)] += 1;
                }
            }
        }
        Ok(Self {
            counts,
            symbols,
            max_len,
        })
    }

    fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, ImageError> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for length in 1..=self.max_len {
            let bit = br.read_bits(1)? as i32;
            code |= bit;
            let count = i32::from(self.counts[usize::from(length)]);
            if code - count < first {
                let idx = (index + (code - first)) as usize;
                return self
                    .symbols
                    .get(idx)
                    .copied()
                    .ok_or(ImageError::InvalidHeader("bad DEFLATE Huffman code"));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(ImageError::InvalidHeader("invalid DEFLATE Huffman symbol"))
    }
}

fn fixed_huffman_trees() -> (HuffmanDecoder, HuffmanDecoder) {
    let mut lit_lengths = [0u8; 288];
    lit_lengths[0..=143].fill(8);
    lit_lengths[144..=255].fill(9);
    lit_lengths[256..=279].fill(7);
    lit_lengths[280..=287].fill(8);
    let dist_lengths = [5u8; 32];
    (
        HuffmanDecoder::build(&lit_lengths).unwrap_or(HuffmanDecoder {
            counts: [0; MAX_BITS + 1],
            symbols: Vec::new(),
            max_len: 0,
        }),
        HuffmanDecoder::build(&dist_lengths).unwrap_or(HuffmanDecoder {
            counts: [0; MAX_BITS + 1],
            symbols: Vec::new(),
            max_len: 0,
        }),
    )
}

fn read_dynamic_trees(
    br: &mut BitReader<'_>,
) -> Result<(HuffmanDecoder, HuffmanDecoder), ImageError> {
    let hlit = br.read_bits(5)? as usize + 257;
    let hdist = br.read_bits(5)? as usize + 1;
    let hclen = br.read_bits(4)? as usize + 4;
    if hlit > 286 || hdist > 32 {
        return Err(ImageError::InvalidHeader("DEFLATE HLIT/HDIST out of range"));
    }
    let mut cl_lengths = [0u8; 19];
    for &idx in CL_ORDER.iter().take(hclen) {
        cl_lengths[idx] = br.read_bits(3)? as u8;
    }
    let cl_tree = HuffmanDecoder::build(&cl_lengths)?;
    let total_codes = hlit + hdist;
    let mut lengths = Vec::with_capacity(total_codes);
    while lengths.len() < total_codes {
        let sym = cl_tree.decode(br)?;
        match sym {
            0..=15 => lengths.push(sym as u8),
            16 => {
                let prev = *lengths
                    .last()
                    .ok_or(ImageError::InvalidHeader("bad DEFLATE repeat 16"))?;
                let repeat = br.read_bits(2)? as usize + 3;
                lengths.resize(lengths.len() + repeat, prev);
            }
            17 => {
                let repeat = br.read_bits(3)? as usize + 3;
                lengths.resize(lengths.len() + repeat, 0);
            }
            18 => {
                let repeat = br.read_bits(7)? as usize + 11;
                lengths.resize(lengths.len() + repeat, 0);
            }
            _ => return Err(ImageError::InvalidHeader("unknown DEFLATE code length")),
        }
    }
    if lengths.len() > total_codes {
        return Err(ImageError::InvalidHeader("DEFLATE code lengths overflow"));
    }
    let lit = HuffmanDecoder::build(&lengths[..hlit])?;
    let dist = HuffmanDecoder::build(&lengths[hlit..])?;
    Ok((lit, dist))
}

fn decode_huffman_block(
    br: &mut BitReader<'_>,
    lit_tree: &HuffmanDecoder,
    dist_tree: &HuffmanDecoder,
    out: &mut Vec<u8>,
    max_output_bytes: usize,
) -> Result<(), ImageError> {
    loop {
        let sym = lit_tree.decode(br)?;
        match sym {
            0..=255 => {
                if out.len() >= max_output_bytes {
                    return Err(ImageError::DimensionsTooLarge {
                        width: 0,
                        height: 0,
                    });
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let len_idx = usize::from(sym - 257);
                let extra = br.read_bits(LENGTH_EXTRA[len_idx])? as usize;
                let length = usize::from(LENGTH_BASES[len_idx]) + extra;

                let dist_sym = usize::from(dist_tree.decode(br)?);
                if dist_sym >= DIST_BASES.len() {
                    return Err(ImageError::InvalidHeader("DEFLATE distance code invalid"));
                }
                let dist_extra = br.read_bits(DIST_EXTRA[dist_sym])? as usize;
                let distance = usize::from(DIST_BASES[dist_sym]) + dist_extra;
                if distance == 0 || distance > out.len() {
                    return Err(ImageError::InvalidHeader(
                        "DEFLATE back-reference distance out of range",
                    ));
                }
                if out.len().saturating_add(length) > max_output_bytes {
                    return Err(ImageError::DimensionsTooLarge {
                        width: 0,
                        height: 0,
                    });
                }
                let start = out.len() - distance;
                for i in 0..length {
                    let b = out[start + i];
                    out.push(b);
                }
            }
            _ => return Err(ImageError::InvalidHeader("invalid DEFLATE literal symbol")),
        }
    }
}

fn deflate_decode(input: &[u8], max_output_bytes: usize) -> Result<Vec<u8>, ImageError> {
    let mut br = BitReader::new(input);
    let mut out = Vec::with_capacity((input.len().saturating_mul(2)).min(max_output_bytes));
    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                br.align_to_byte();
                let block_len = br.read_u16_le()?;
                let nlen = br.read_u16_le()?;
                if block_len != !nlen {
                    return Err(ImageError::InvalidHeader("DEFLATE LEN/NLEN mismatch"));
                }
                let len = usize::from(block_len);
                if out.len().saturating_add(len) > max_output_bytes {
                    return Err(ImageError::DimensionsTooLarge {
                        width: 0,
                        height: 0,
                    });
                }
                let slice = br.read_bytes(len)?;
                out.extend_from_slice(&slice);
            }
            1 => {
                let (lit_tree, dist_tree) = fixed_huffman_trees();
                decode_huffman_block(&mut br, &lit_tree, &dist_tree, &mut out, max_output_bytes)?;
            }
            2 => {
                let (lit_tree, dist_tree) = read_dynamic_trees(&mut br)?;
                decode_huffman_block(&mut br, &lit_tree, &dist_tree, &mut out, max_output_bytes)?;
            }
            _ => return Err(ImageError::InvalidHeader("invalid DEFLATE block type")),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok(out)
}

struct BitWriter {
    out: Vec<u8>,
    bit_buf: u64,
    bit_cnt: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn write_bits(&mut self, val: u32, n: u8) {
        if n == 0 {
            return;
        }
        self.bit_buf |= (u64::from(val) & ((1u64 << n) - 1)) << self.bit_cnt;
        self.bit_cnt += n;
        while self.bit_cnt >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
        }
    }

    fn write_huffman_msb(&mut self, code: u16, length: u8) {
        let mut rev = 0u32;
        for i in 0..length {
            let bit = (code >> (length - 1 - i)) & 1;
            rev |= u32::from(bit) << i;
        }
        self.write_bits(rev, length);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_cnt > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
        }
        self.out
    }
}

fn write_fixed_literal(bw: &mut BitWriter, sym: u16) {
    match sym {
        0..=143 => bw.write_huffman_msb(0x30 + sym, 8),
        144..=255 => bw.write_huffman_msb(0x190 + (sym - 144), 9),
        256..=279 => bw.write_huffman_msb(sym - 256, 7),
        _ => bw.write_huffman_msb(0xC0 + (sym - 280), 8),
    }
}

fn write_fixed_length(bw: &mut BitWriter, length: usize) {
    let mut idx = LENGTH_BASES.len() - 1;
    for (i, &base) in LENGTH_BASES.iter().enumerate() {
        let next_base = LENGTH_BASES.get(i + 1).copied().unwrap_or(259) as usize;
        if length >= usize::from(base) && length < next_base {
            idx = i;
            break;
        }
    }
    write_fixed_literal(bw, 257 + idx as u16);
    let extra = LENGTH_EXTRA[idx];
    if extra > 0 {
        bw.write_bits((length - usize::from(LENGTH_BASES[idx])) as u32, extra);
    }
}

fn write_fixed_distance(bw: &mut BitWriter, dist: usize) {
    let mut idx = DIST_BASES.len() - 1;
    for (i, &base) in DIST_BASES.iter().enumerate() {
        let next_base = DIST_BASES.get(i + 1).copied().unwrap_or(32769) as usize;
        if dist >= usize::from(base) && dist < next_base {
            idx = i;
            break;
        }
    }
    bw.write_huffman_msb(idx as u16, 5);
    let extra = DIST_EXTRA[idx];
    if extra > 0 {
        bw.write_bits((dist - usize::from(DIST_BASES[idx])) as u32, extra);
    }
}

fn deflate_compress_stored(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + 5 * (input.len() / 65535 + 1));
    let mut offset = 0usize;
    loop {
        let chunk_len = (input.len() - offset).min(65535);
        let is_final = offset + chunk_len >= input.len();
        out.push(if is_final { 0x01 } else { 0x00 });
        let l = chunk_len as u16;
        let nl = !l;
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&nl.to_le_bytes());
        out.extend_from_slice(&input[offset..offset + chunk_len]);
        offset += chunk_len;
        if is_final {
            break;
        }
    }
    out
}

fn deflate_compress(input: &[u8]) -> Vec<u8> {
    if input.is_empty() {
        return vec![0x01, 0x00, 0x00, 0xFF, 0xFF];
    }
    const WINDOW_SIZE: usize = 32768;
    const HASH_BITS: usize = 15;
    const HASH_SIZE: usize = 1 << HASH_BITS;
    const HASH_MASK: usize = HASH_SIZE - 1;
    const MAX_CHAIN: usize = 32;

    let mut bw = BitWriter::new();
    bw.write_bits(1, 1);
    bw.write_bits(1, 2);

    let mut head = vec![-1isize; HASH_SIZE];
    let mut prev = vec![-1isize; input.len()];
    let hash3 = |p: usize| -> usize {
        let v =
            (u32::from(input[p]) << 16) | (u32::from(input[p + 1]) << 8) | u32::from(input[p + 2]);
        (((v.wrapping_mul(0x1E35_A7BD)) >> (32 - HASH_BITS)) as usize) & HASH_MASK
    };

    let mut pos = 0usize;
    while pos < input.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        if pos + 3 <= input.len() {
            let h = hash3(pos);
            let mut cand = head[h];
            prev[pos] = cand;
            head[h] = pos as isize;
            let mut chain = 0usize;
            let max_len = 258usize.min(input.len() - pos);
            while cand >= 0 && chain < MAX_CHAIN {
                let cpos = cand as usize;
                let dist = pos - cpos;
                if dist > WINDOW_SIZE {
                    break;
                }
                if input[cpos + best_len] == input[pos + best_len] {
                    let mut l = 0usize;
                    while l < max_len && input[cpos + l] == input[pos + l] {
                        l += 1;
                    }
                    if l >= 3 && l > best_len {
                        best_len = l;
                        best_dist = dist;
                        if l == max_len {
                            break;
                        }
                    }
                }
                cand = prev[cpos];
                chain += 1;
            }
        }
        if best_len >= 3 {
            write_fixed_length(&mut bw, best_len);
            write_fixed_distance(&mut bw, best_dist);
            for step in 1..best_len {
                let p = pos + step;
                if p + 3 <= input.len() {
                    let h = hash3(p);
                    prev[p] = head[h];
                    head[h] = p as isize;
                }
            }
            pos += best_len;
        } else {
            write_fixed_literal(&mut bw, u16::from(input[pos]));
            pos += 1;
        }
    }
    write_fixed_literal(&mut bw, 256);
    let compressed = bw.finish();
    let stored_overhead = 5 * input.len().div_ceil(65535);
    if compressed.len() <= input.len().saturating_add(stored_overhead) {
        compressed
    } else {
        deflate_compress_stored(input)
    }
}
