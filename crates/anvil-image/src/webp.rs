// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Pure-Rust WebP decoder (VP8L lossless, RFC 6386 VP8 lossy keyframe + ALPH, RGBA, first-frame ANMF)
//! and VP8L lossless encoder.

use crate::deflate::zlib_decompress_bounded;
use crate::vp8_tables::{COEFF_UPDATE_PROBS, DEFAULT_COEFF_PROBS, KF_BMODE_PROBS};
use crate::{Image, ImageError, validate_dimensions};

const VP8L_SIGNATURE: u8 = 0x2F;
const LITERAL_ALPHABET: usize = 256;
const LENGTH_PREFIXES: usize = 24;
const DISTANCE_ALPHABET: usize = 40;
const MAX_CODE_LENGTH: usize = 15;
const ROOT_BITS: u32 = 8;

const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

const DISTANCE_MAP: [(i8, i8); 120] = [
    (0, 1),
    (1, 0),
    (1, 1),
    (-1, 1),
    (0, 2),
    (2, 0),
    (1, 2),
    (-1, 2),
    (2, 1),
    (-2, 1),
    (2, 2),
    (-2, 2),
    (0, 3),
    (3, 0),
    (1, 3),
    (-1, 3),
    (3, 1),
    (-3, 1),
    (2, 3),
    (-2, 3),
    (3, 2),
    (-3, 2),
    (0, 4),
    (4, 0),
    (1, 4),
    (-1, 4),
    (4, 1),
    (-4, 1),
    (3, 3),
    (-3, 3),
    (2, 4),
    (-2, 4),
    (4, 2),
    (-4, 2),
    (0, 5),
    (3, 4),
    (-3, 4),
    (4, 3),
    (-4, 3),
    (5, 0),
    (1, 5),
    (-1, 5),
    (5, 1),
    (-5, 1),
    (2, 5),
    (-2, 5),
    (5, 2),
    (-5, 2),
    (4, 4),
    (-4, 4),
    (3, 5),
    (-3, 5),
    (5, 3),
    (-5, 3),
    (0, 6),
    (6, 0),
    (1, 6),
    (-1, 6),
    (6, 1),
    (-6, 1),
    (2, 6),
    (-2, 6),
    (6, 2),
    (-6, 2),
    (4, 5),
    (-4, 5),
    (5, 4),
    (-5, 4),
    (3, 6),
    (-3, 6),
    (6, 3),
    (-6, 3),
    (0, 7),
    (7, 0),
    (1, 7),
    (-1, 7),
    (5, 5),
    (-5, 5),
    (7, 1),
    (-7, 1),
    (4, 6),
    (-4, 6),
    (6, 4),
    (-6, 4),
    (2, 7),
    (-2, 7),
    (7, 2),
    (-7, 2),
    (3, 7),
    (-3, 7),
    (7, 3),
    (-7, 3),
    (5, 6),
    (-5, 6),
    (6, 5),
    (-6, 5),
    (8, 0),
    (4, 7),
    (-4, 7),
    (7, 4),
    (-7, 4),
    (8, 1),
    (8, 2),
    (6, 6),
    (-6, 6),
    (8, 3),
    (5, 7),
    (-5, 7),
    (7, 5),
    (-7, 5),
    (8, 4),
    (6, 7),
    (-6, 7),
    (7, 6),
    (-7, 6),
    (8, 5),
    (7, 7),
    (-7, 7),
    (8, 6),
    (8, 7),
];

#[derive(Default)]
struct WebpChunks<'a> {
    canvas_w: u32,
    canvas_h: u32,
    has_size: bool,
    vp8l: Option<&'a [u8]>,
    vp8: Option<&'a [u8]>,
    alpha: Option<&'a [u8]>,
    rgba: Option<&'a [u8]>,
    anmf: Option<&'a [u8]>,
}

#[inline]
fn read_u24(b: &[u8]) -> u32 {
    u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16)
}

fn scan_subchunks<'a>(data: &'a [u8], chunks: &mut WebpChunks<'a>) -> Result<(), ImageError> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let fourcc = &data[pos..pos + 4];
        let chunk_len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if chunk_len > data.len() - pos {
            return Err(ImageError::UnexpectedEof);
        }
        let payload = &data[pos..pos + chunk_len];
        match fourcc {
            b"VP8X" => {
                if payload.len() < 10 {
                    return Err(ImageError::InvalidHeader("truncated WebP VP8X chunk"));
                }
                let w = read_u24(&payload[4..7]) + 1;
                let h = read_u24(&payload[7..10]) + 1;
                validate_dimensions(w, h)?;
                if !chunks.has_size {
                    chunks.canvas_w = w;
                    chunks.canvas_h = h;
                    chunks.has_size = true;
                }
            }
            b"VP8L" if chunks.vp8l.is_none() => chunks.vp8l = Some(payload),
            b"VP8 " if chunks.vp8.is_none() => chunks.vp8 = Some(payload),
            b"ALPH" if chunks.alpha.is_none() => chunks.alpha = Some(payload),
            b"RGBA" if chunks.rgba.is_none() => chunks.rgba = Some(payload),
            b"ANMF" if chunks.anmf.is_none() => chunks.anmf = Some(payload),
            _ => {}
        }
        pos = pos.saturating_add(chunk_len).saturating_add(chunk_len & 1);
    }
    Ok(())
}

fn walk_webp(data: &[u8]) -> Result<WebpChunks<'_>, ImageError> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err(ImageError::InvalidHeader("missing RIFF WEBP header"));
    }
    let riff_size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let end = riff_size.saturating_add(8).max(12).min(data.len());
    let mut chunks = WebpChunks::default();
    scan_subchunks(&data[12..end], &mut chunks)?;
    if chunks.vp8l.is_none()
        && chunks.vp8.is_none()
        && let Some(anmf) = chunks.anmf
        && anmf.len() > 16
    {
        scan_subchunks(&anmf[16..], &mut chunks)?;
        chunks.has_size = false;
    }
    Ok(chunks)
}

fn argb_to_image(width: u32, height: u32, argb: &[u32]) -> Result<Image, ImageError> {
    let mut pixels = vec![0u8; argb.len() * 4];
    for (i, &p) in argb.iter().enumerate() {
        let o = i * 4;
        pixels[o] = ((p >> 16) & 0xFF) as u8;
        pixels[o + 1] = ((p >> 8) & 0xFF) as u8;
        pixels[o + 2] = (p & 0xFF) as u8;
        pixels[o + 3] = ((p >> 24) & 0xFF) as u8;
    }
    Image::from_rgba(width, height, pixels)
}

fn unfilter_alpha(alpha: &mut [u8], width: usize, height: usize, filter: u8) {
    if filter == 0 || width == 0 {
        return;
    }
    for x in 1..width {
        alpha[x] = alpha[x].wrapping_add(alpha[x - 1]);
    }
    for y in 1..height {
        let row = y * width;
        for x in 0..width {
            let i = row + x;
            let above = alpha[i - width];
            let pred = if x == 0 {
                above
            } else {
                match filter {
                    1 => alpha[i - 1],
                    2 => above,
                    _ => {
                        let left = i32::from(alpha[i - 1]);
                        let corner = i32::from(alpha[i - width - 1]);
                        (left + i32::from(above) - corner).clamp(0, 255) as u8
                    }
                }
            };
            alpha[i] = alpha[i].wrapping_add(pred);
        }
    }
}

fn decode_alpha(payload: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImageError> {
    if payload.is_empty() {
        return Err(ImageError::UnexpectedEof);
    }
    let info = payload[0];
    let data = &payload[1..];
    let count = (width as usize) * (height as usize);
    let method = info & 3;
    let mut alpha = match method {
        0 => {
            if data.len() < count {
                return Err(ImageError::UnexpectedEof);
            }
            data[..count].to_vec()
        }
        1 => {
            let stream = decode_vp8l_headerless(data, width, height)?;
            stream
                .into_iter()
                .map(|p| ((p >> 8) & 0xFF) as u8)
                .collect()
        }
        _ => return Err(ImageError::InvalidHeader("unknown ALPH compression method")),
    };
    let filter = (info >> 2) & 3;
    unfilter_alpha(&mut alpha, width as usize, height as usize, filter);
    Ok(alpha)
}

/// Decode a RIFF WEBP image (`VP8L`, `VP8 ` + optional `ALPH`, `RGBA`, or animated first frame).
pub fn decode_webp(data: &[u8]) -> Result<Image, ImageError> {
    let chunks = walk_webp(data)?;
    if let Some(vp8l) = chunks.vp8l {
        let (w, h, argb) = decode_vp8l(vp8l)?;
        if chunks.has_size && (chunks.canvas_w != w || chunks.canvas_h != h) {
            return Err(ImageError::InvalidHeader(
                "WebP bitstream size differs from VP8X canvas",
            ));
        }
        return argb_to_image(w, h, &argb);
    }
    if let Some(vp8) = chunks.vp8 {
        let frame = decode_vp8(vp8)?;
        if chunks.has_size && (chunks.canvas_w != frame.width || chunks.canvas_h != frame.height) {
            return Err(ImageError::InvalidHeader(
                "WebP bitstream size differs from VP8X canvas",
            ));
        }
        let mut rgba = frame.to_rgba();
        if let Some(alph) = chunks.alpha {
            let alpha = decode_alpha(alph, frame.width, frame.height)?;
            for (i, &a) in alpha.iter().enumerate() {
                if i * 4 + 3 < rgba.len() {
                    rgba[i * 4 + 3] = a;
                }
            }
        }
        return Image::from_rgba(frame.width, frame.height, rgba);
    }
    if let Some(rgba_chunk) = chunks.rgba
        && chunks.has_size
    {
        let expected = (chunks.canvas_w as usize) * (chunks.canvas_h as usize) * 4;
        let pixels = zlib_decompress_bounded(rgba_chunk, expected)?;
        return Image::from_rgba(chunks.canvas_w, chunks.canvas_h, pixels);
    }
    Err(ImageError::UnsupportedFormat(
        "WebP container has no supported image chunk",
    ))
}

/// Encode `image` as a lossless RIFF WEBP container (`VP8L` chunk).
pub fn encode_webp(image: &Image) -> Vec<u8> {
    let payload = encode_vp8l(image.width, image.height, &image.pixels);
    let pad = payload.len() & 1;
    let riff_size = (4 + 8 + payload.len() + pad) as u32;
    let mut out = Vec::with_capacity(8 + riff_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(b"VP8L");
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    if pad == 1 {
        out.push(0);
    }
    out
}

// ---------- VP8L Lossless Codec ----------

struct Vp8lBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> Vp8lBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut br = Self {
            data,
            pos: 0,
            acc: 0,
            nbits: 0,
        };
        br.refill();
        br
    }

    fn refill(&mut self) {
        while self.nbits <= 56 && self.pos < self.data.len() {
            self.acc |= u64::from(self.data[self.pos]) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
    }

    fn peek(&mut self, n: u32) -> u32 {
        if self.nbits < n {
            self.refill();
        }
        (self.acc & ((1u64 << n) - 1)) as u32
    }

    fn consume(&mut self, n: u32) -> Result<(), ImageError> {
        if n > self.nbits {
            self.refill();
            if n > self.nbits {
                return Err(ImageError::UnexpectedEof);
            }
        }
        self.acc >>= n;
        self.nbits -= n;
        Ok(())
    }

    fn read(&mut self, n: u32) -> Result<u32, ImageError> {
        if n == 0 {
            return Ok(0);
        }
        let val = self.peek(n);
        self.consume(n)?;
        Ok(val)
    }

    fn read_flag(&mut self) -> Result<bool, ImageError> {
        Ok(self.read(1)? == 1)
    }
}

#[derive(Clone, Copy, Default)]
struct PrefixEntry {
    bits: u8,
    symbol: u16,
}

struct PrefixCode {
    single_symbol: Option<u16>,
    root: Vec<PrefixEntry>,
    sub_tables: Vec<Vec<PrefixEntry>>,
}

fn reverse_bits(val: u32, n: u8) -> u32 {
    let mut res = 0u32;
    for i in 0..n {
        res = (res << 1) | ((val >> i) & 1);
    }
    res
}

impl PrefixCode {
    fn build(lengths: &[u8]) -> Result<Self, ImageError> {
        let alphabet = lengths.len();
        let mut used_count = 0usize;
        let mut last_used = 0usize;
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                if usize::from(l) > MAX_CODE_LENGTH {
                    return Err(ImageError::InvalidHeader("VP8L code length too long"));
                }
                used_count += 1;
                last_used = sym;
            }
        }
        if used_count == 0 {
            return Err(ImageError::InvalidHeader("empty VP8L prefix code"));
        }
        if used_count == 1 {
            return Ok(Self {
                single_symbol: Some(last_used as u16),
                root: Vec::new(),
                sub_tables: Vec::new(),
            });
        }

        let mut sorted: Vec<(usize, u8)> = lengths
            .iter()
            .enumerate()
            .filter_map(|(s, &l)| (l > 0).then_some((s, l)))
            .collect();
        sorted.sort_by_key(|&(s, l)| (l, s));

        let mut canonical_codes = vec![0u32; alphabet];
        let mut next_code = 0u32;
        let mut prev_len = 0u8;
        let mut total_weight = 0u32;
        for &(s, l) in &sorted {
            next_code <<= l - prev_len;
            prev_len = l;
            canonical_codes[s] = next_code;
            next_code += 1;
            total_weight += 1u32 << (MAX_CODE_LENGTH - usize::from(l));
        }
        if total_weight != (1u32 << MAX_CODE_LENGTH) {
            return Err(ImageError::InvalidHeader("incomplete VP8L prefix code"));
        }

        let root_size = 1usize << ROOT_BITS;
        let mut root = vec![PrefixEntry::default(); root_size];
        let mut sub_tables: Vec<Vec<PrefixEntry>> = Vec::new();

        for (s, l) in sorted {
            let sym = s as u16;
            let rev = reverse_bits(canonical_codes[s], l);
            if u32::from(l) <= ROOT_BITS {
                let step = 1usize << l;
                let mut i = rev as usize;
                while i < root_size {
                    root[i] = PrefixEntry {
                        bits: l,
                        symbol: sym,
                    };
                    i += step;
                }
            } else {
                let root_prefix = (rev & ((1 << ROOT_BITS) - 1)) as usize;
                let sub_bits = l - (ROOT_BITS as u8);
                let sub_code = (rev >> ROOT_BITS) as usize;
                let sub_idx = if root[root_prefix].bits == 0 {
                    let idx = sub_tables.len();
                    sub_tables.push(vec![
                        PrefixEntry::default();
                        1usize << (MAX_CODE_LENGTH - ROOT_BITS as usize)
                    ]);
                    root[root_prefix] = PrefixEntry {
                        bits: (ROOT_BITS as u8) + 1,
                        symbol: idx as u16,
                    };
                    idx
                } else {
                    usize::from(root[root_prefix].symbol)
                };
                let step = 1usize << sub_bits;
                let mut i = sub_code;
                while i < sub_tables[sub_idx].len() {
                    sub_tables[sub_idx][i] = PrefixEntry {
                        bits: l,
                        symbol: sym,
                    };
                    i += step;
                }
            }
        }

        Ok(Self {
            single_symbol: None,
            root,
            sub_tables,
        })
    }

    fn read_symbol(&self, br: &mut Vp8lBitReader<'_>) -> Result<u16, ImageError> {
        if let Some(sym) = self.single_symbol {
            return Ok(sym);
        }
        let peek = br.peek(ROOT_BITS) as usize;
        let entry = self.root[peek];
        if u32::from(entry.bits) <= ROOT_BITS {
            if entry.bits == 0 {
                return Err(ImageError::InvalidHeader("invalid VP8L prefix lookup"));
            }
            br.consume(u32::from(entry.bits))?;
            return Ok(entry.symbol);
        }
        br.consume(ROOT_BITS)?;
        let sub_idx = usize::from(entry.symbol);
        let sub_peek = br.peek(MAX_CODE_LENGTH as u32 - ROOT_BITS) as usize;
        let sub_entry = self
            .sub_tables
            .get(sub_idx)
            .and_then(|t| t.get(sub_peek))
            .copied()
            .ok_or(ImageError::InvalidHeader("invalid VP8L sub-table lookup"))?;
        if sub_entry.bits == 0 {
            return Err(ImageError::InvalidHeader("invalid VP8L sub-table code"));
        }
        br.consume(u32::from(sub_entry.bits) - ROOT_BITS)?;
        Ok(sub_entry.symbol)
    }
}

struct PrefixGroup {
    green: PrefixCode,
    red: PrefixCode,
    blue: PrefixCode,
    alpha: PrefixCode,
    distance: PrefixCode,
}

enum Vp8lTransform {
    Predictor {
        bits: u32,
        data: Vec<u32>,
    },
    CrossColor {
        bits: u32,
        data: Vec<u32>,
    },
    SubtractGreen,
    ColorIndexing {
        packed_width: usize,
        bits: u32,
        palette: Vec<u32>,
    },
}

fn div_round_up(n: usize, bits: u32) -> usize {
    (n + (1 << bits) - 1) >> bits
}

fn read_prefix_code(
    br: &mut Vp8lBitReader<'_>,
    alphabet_size: usize,
) -> Result<PrefixCode, ImageError> {
    if br.read_flag()? {
        let num_symbols = br.read(1)? + 1;
        let first_bits = if br.read(1)? == 1 { 8 } else { 1 };
        let s0 = br.read(first_bits)? as usize;
        if s0 >= alphabet_size {
            return Err(ImageError::InvalidHeader("VP8L simple symbol out of range"));
        }
        let mut lengths = vec![0u8; alphabet_size];
        lengths[s0] = 1;
        if num_symbols == 2 {
            let s1 = br.read(8)? as usize;
            if s1 >= alphabet_size {
                return Err(ImageError::InvalidHeader("VP8L simple symbol out of range"));
            }
            lengths[s1] = 1;
        }
        return PrefixCode::build(&lengths);
    }

    let num_code_lengths = br.read(4)? as usize + 4;
    let mut cl_lengths = [0u8; 19];
    for &order_idx in CODE_LENGTH_ORDER.iter().take(num_code_lengths) {
        cl_lengths[order_idx] = br.read(3)? as u8;
    }
    let cl_code = PrefixCode::build(&cl_lengths)?;

    let mut lengths = vec![0u8; alphabet_size];
    let max_symbols = if br.read_flag()? {
        let len_bits = br.read(3)?;
        let n = br.read(2 + 2 * len_bits)? as usize + 2;
        if n > alphabet_size {
            return Err(ImageError::InvalidHeader("VP8L max symbols overflow"));
        }
        n
    } else {
        alphabet_size
    };

    let mut prev_len = 8u8;
    let mut i = 0usize;
    while i < alphabet_size && i < max_symbols {
        let sym = cl_code.read_symbol(br)?;
        if sym < 16 {
            lengths[i] = sym as u8;
            if sym != 0 {
                prev_len = sym as u8;
            }
            i += 1;
        } else if sym == 16 {
            let count = br.read(2)? as usize + 3;
            for _ in 0..count {
                if i < alphabet_size {
                    lengths[i] = prev_len;
                    i += 1;
                }
            }
        } else if sym == 17 {
            let count = br.read(3)? as usize + 3;
            for _ in 0..count {
                if i < alphabet_size {
                    lengths[i] = 0;
                    i += 1;
                }
            }
        } else {
            let count = br.read(7)? as usize + 11;
            for _ in 0..count {
                if i < alphabet_size {
                    lengths[i] = 0;
                    i += 1;
                }
            }
        }
    }

    PrefixCode::build(&lengths)
}

fn read_prefix_group(
    br: &mut Vp8lBitReader<'_>,
    cache_size: usize,
) -> Result<PrefixGroup, ImageError> {
    Ok(PrefixGroup {
        green: read_prefix_code(br, LITERAL_ALPHABET + LENGTH_PREFIXES + cache_size)?,
        red: read_prefix_code(br, LITERAL_ALPHABET)?,
        blue: read_prefix_code(br, LITERAL_ALPHABET)?,
        alpha: read_prefix_code(br, LITERAL_ALPHABET)?,
        distance: read_prefix_code(br, DISTANCE_ALPHABET)?,
    })
}

fn prefix_value(br: &mut Vp8lBitReader<'_>, prefix: usize) -> Result<usize, ImageError> {
    if prefix < 4 {
        return Ok(prefix + 1);
    }
    let extra_bits = ((prefix - 2) >> 1) as u32;
    let offset = (2 + (prefix & 1)) << extra_bits;
    let extra = br.read(extra_bits)? as usize;
    Ok(offset + extra + 1)
}

fn plane_distance(width: usize, dist_code: usize) -> usize {
    if dist_code <= 120 {
        let (dx, dy) = DISTANCE_MAP[dist_code - 1];
        let d = isize::from(dy) * (width as isize) + isize::from(dx);
        if d < 1 { 1 } else { d as usize }
    } else {
        dist_code - 120
    }
}

#[inline]
fn add_pixels(a: u32, b: u32) -> u32 {
    let ag = (a & 0xFF00_FF00).wrapping_add(b & 0xFF00_FF00);
    let rb = (a & 0x00FF_00FF).wrapping_add(b & 0x00FF_00FF);
    (ag & 0xFF00_FF00) | (rb & 0x00FF_00FF)
}

#[inline]
fn average2(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xFEFE_FEFE) >> 1) + (a & b)
}

#[inline]
fn select_predictor(left: u32, top: u32, top_left: u32) -> u32 {
    let mut to_left = 0i32;
    let mut to_top = 0i32;
    for shift in [24u32, 16, 8, 0] {
        let l = ((left >> shift) & 0xFF) as i32;
        let t = ((top >> shift) & 0xFF) as i32;
        let tl = ((top_left >> shift) & 0xFF) as i32;
        to_left += (t - tl).abs();
        to_top += (l - tl).abs();
    }
    if to_left < to_top { left } else { top }
}

#[inline]
fn clamp_add_sub_full(a: u32, b: u32, c: u32) -> u32 {
    let mut out = 0u32;
    for shift in [24u32, 16, 8, 0] {
        let va = ((a >> shift) & 0xFF) as i32;
        let vb = ((b >> shift) & 0xFF) as i32;
        let vc = ((c >> shift) & 0xFF) as i32;
        let v = (va + vb - vc).clamp(0, 255) as u32;
        out |= v << shift;
    }
    out
}

#[inline]
fn clamp_add_sub_half(a: u32, b: u32) -> u32 {
    let mut out = 0u32;
    for shift in [24u32, 16, 8, 0] {
        let ca = ((a >> shift) & 0xFF) as i32;
        let cb = ((b >> shift) & 0xFF) as i32;
        let v = (ca + (ca - cb) / 2).clamp(0, 255) as u32;
        out |= v << shift;
    }
    out
}

fn vp8l_predict(mode: u32, left: u32, top: u32, top_left: u32, top_right: u32) -> u32 {
    match mode {
        0 => 0xFF00_0000,
        1 => left,
        2 => top,
        3 => top_right,
        4 => top_left,
        5 => average2(average2(left, top_right), top),
        6 => average2(left, top_left),
        7 => average2(left, top),
        8 => average2(top_left, top),
        9 => average2(top, top_right),
        10 => average2(average2(left, top_left), average2(top, top_right)),
        11 => select_predictor(left, top, top_left),
        12 => clamp_add_sub_full(left, top, top_left),
        13 => clamp_add_sub_half(average2(left, top), top_left),
        _ => 0xFF00_0000,
    }
}

fn inverse_predictor(pixels: &mut [u32], width: usize, height: usize, bits: u32, data: &[u32]) {
    if pixels.is_empty() {
        return;
    }
    let blocks_wide = div_round_up(width, bits);
    pixels[0] = add_pixels(pixels[0], 0xFF00_0000);
    for x in 1..width {
        pixels[x] = add_pixels(pixels[x], pixels[x - 1]);
    }
    for y in 1..height {
        let row = y * width;
        pixels[row] = add_pixels(pixels[row], pixels[row - width]);
        let block_row = (y >> bits) * blocks_wide;
        for x in 1..width {
            let mode = (data[block_row + (x >> bits)] >> 8) & 0x0F;
            let i = row + x;
            let left = pixels[i - 1];
            let top = pixels[i - width];
            let top_left = pixels[i - width - 1];
            let top_right = if x + 1 < width {
                pixels[i - width + 1]
            } else {
                pixels[row]
            };
            let pred = vp8l_predict(mode, left, top, top_left, top_right);
            pixels[i] = add_pixels(pixels[i], pred);
        }
    }
}

#[inline]
fn color_delta(t: u8, c: u8) -> i32 {
    (i32::from(t as i8) * i32::from(c as i8)) >> 5
}

fn inverse_cross_color(pixels: &mut [u32], width: usize, bits: u32, data: &[u32]) {
    let blocks_wide = div_round_up(width, bits);
    for (i, px) in pixels.iter_mut().enumerate() {
        let x = i % width;
        let y = i / width;
        let elem = data[(y >> bits) * blocks_wide + (x >> bits)];
        let g2r = elem as u8;
        let g2b = (elem >> 8) as u8;
        let r2b = (elem >> 16) as u8;
        let p = *px;
        let green = (p >> 8) as u8;
        let red = ((p >> 16) & 0xFF) as i32 + color_delta(g2r, green);
        let red_byte = (red & 0xFF) as u8;
        let blue = (p & 0xFF) as i32 + color_delta(g2b, green) + color_delta(r2b, red_byte);
        *px = (p & 0xFF00_FF00) | (u32::from(red_byte) << 16) | ((blue as u32) & 0xFF);
    }
}

fn inverse_color_indexing(
    packed: &[u32],
    packed_width: usize,
    width: usize,
    height: usize,
    bits: u32,
    palette: &[u32],
) -> Vec<u32> {
    let mut out = vec![0u32; width * height];
    let per_byte_bits = 8 >> bits;
    let index_mask = (1u32 << per_byte_bits) - 1;
    let x_mask = (1usize << bits) - 1;
    for y in 0..height {
        for x in 0..width {
            let source = packed[y * packed_width + (x >> bits)];
            let green = (source >> 8) & 0xFF;
            let index = ((green >> (((x & x_mask) as u32) * per_byte_bits)) & index_mask) as usize;
            if let Some(&color) = palette.get(index) {
                out[y * width + x] = color;
            }
        }
    }
    out
}

fn decode_vp8l_image_stream(
    br: &mut Vp8lBitReader<'_>,
    width: usize,
    height: usize,
) -> Result<Vec<u32>, ImageError> {
    let mut transforms = Vec::new();
    let mut cur_w = width;
    let cur_h = height;

    while br.read_flag()? {
        let t_type = br.read(2)?;
        match t_type {
            0 => {
                let bits = br.read(3)? + 2;
                let bw = div_round_up(cur_w, bits);
                let bh = div_round_up(cur_h, bits);
                let data = decode_vp8l_image_stream(br, bw, bh)?;
                transforms.push(Vp8lTransform::Predictor { bits, data });
            }
            1 => {
                let bits = br.read(3)? + 2;
                let bw = div_round_up(cur_w, bits);
                let bh = div_round_up(cur_h, bits);
                let data = decode_vp8l_image_stream(br, bw, bh)?;
                transforms.push(Vp8lTransform::CrossColor { bits, data });
            }
            2 => transforms.push(Vp8lTransform::SubtractGreen),
            3 => {
                let num_colors = br.read(8)? as usize + 1;
                let palette = decode_vp8l_image_stream(br, num_colors, 1)?;
                let bits = if num_colors <= 2 {
                    3
                } else if num_colors <= 4 {
                    2
                } else if num_colors <= 16 {
                    1
                } else {
                    0
                };
                let packed_width = div_round_up(cur_w, bits);
                transforms.push(Vp8lTransform::ColorIndexing {
                    packed_width,
                    bits,
                    palette,
                });
                cur_w = packed_width;
            }
            _ => unreachable!(),
        }
    }

    let (cache_bits, cache_size) = if br.read_flag()? {
        let b = br.read(4)?;
        if !(1..=11).contains(&b) {
            return Err(ImageError::InvalidHeader(
                "VP8L color cache bits out of range",
            ));
        }
        (b, 1usize << b)
    } else {
        (0u32, 0usize)
    };

    let mut meta_bits = 0u32;
    let mut meta_width = 0usize;
    let mut meta_image: Option<Vec<u32>> = None;
    let mut num_groups = 1usize;
    if br.read_flag()? {
        meta_bits = br.read(3)? + 2;
        meta_width = div_round_up(cur_w, meta_bits);
        let meta_height = div_round_up(cur_h, meta_bits);
        let m = decode_vp8l_image_stream(br, meta_width, meta_height)?;
        let max_g = m
            .iter()
            .map(|&p| ((p >> 8) & 0xFFFF) as usize)
            .max()
            .unwrap_or(0);
        num_groups = max_g + 1;
        meta_image = Some(m);
    }

    let mut groups = Vec::with_capacity(num_groups);
    for _ in 0..num_groups {
        groups.push(read_prefix_group(br, cache_size)?);
    }

    let count = cur_w * cur_h;
    let mut pixels = vec![0u32; count];
    let mut color_cache = vec![0u32; cache_size];
    let insert_cache = |cache: &mut [u32], p: u32| {
        if cache_size > 0 {
            let idx = (0x1E35_A7BDu32.wrapping_mul(p) >> (32 - cache_bits)) as usize;
            cache[idx] = p;
        }
    };

    let mut pixel_idx = 0usize;
    while pixel_idx < count {
        let grp = if let Some(ref m) = meta_image {
            let x = pixel_idx % cur_w;
            let y = pixel_idx / cur_w;
            let m_idx = (y >> meta_bits) * meta_width + (x >> meta_bits);
            let g_idx = m
                .get(m_idx)
                .map(|&p| ((p >> 8) & 0xFFFF) as usize)
                .unwrap_or(0);
            groups.get(g_idx).unwrap_or(&groups[0])
        } else {
            &groups[0]
        };

        let sym = usize::from(grp.green.read_symbol(br)?);
        if sym < LITERAL_ALPHABET {
            let green = sym as u32;
            let red = u32::from(grp.red.read_symbol(br)?);
            let blue = u32::from(grp.blue.read_symbol(br)?);
            let alpha = u32::from(grp.alpha.read_symbol(br)?);
            let p = (alpha << 24) | (red << 16) | (green << 8) | blue;
            pixels[pixel_idx] = p;
            insert_cache(&mut color_cache, p);
            pixel_idx += 1;
        } else if sym < LITERAL_ALPHABET + LENGTH_PREFIXES {
            let len = prefix_value(br, sym - LITERAL_ALPHABET)?;
            let dist_sym = usize::from(grp.distance.read_symbol(br)?);
            let dist_val = prefix_value(br, dist_sym)?;
            let dist = plane_distance(cur_w, dist_val);
            if dist < 1 || pixel_idx < dist {
                return Err(ImageError::InvalidHeader(
                    "VP8L backward reference before start",
                ));
            }
            for _ in 0..len {
                if pixel_idx >= count {
                    break;
                }
                let p = pixels[pixel_idx - dist];
                pixels[pixel_idx] = p;
                insert_cache(&mut color_cache, p);
                pixel_idx += 1;
            }
        } else {
            let cache_idx = sym - (LITERAL_ALPHABET + LENGTH_PREFIXES);
            let p = *color_cache
                .get(cache_idx)
                .ok_or(ImageError::InvalidHeader("VP8L color cache out of range"))?;
            pixels[pixel_idx] = p;
            insert_cache(&mut color_cache, p);
            pixel_idx += 1;
        }
    }

    let mut cur_pix_w = cur_w;
    for t in transforms.into_iter().rev() {
        match t {
            Vp8lTransform::Predictor { bits, data } => {
                inverse_predictor(&mut pixels, cur_pix_w, cur_h, bits, &data);
            }
            Vp8lTransform::CrossColor { bits, data } => {
                inverse_cross_color(&mut pixels, cur_pix_w, bits, &data);
            }
            Vp8lTransform::SubtractGreen => {
                for px in &mut pixels {
                    let p = *px;
                    let green = (p >> 8) & 0xFF;
                    let red = ((p >> 16) + green) & 0xFF;
                    let blue = (p + green) & 0xFF;
                    *px = (p & 0xFF00_FF00) | (red << 16) | blue;
                }
            }
            Vp8lTransform::ColorIndexing {
                packed_width,
                bits,
                palette,
            } => {
                pixels =
                    inverse_color_indexing(&pixels, packed_width, width, height, bits, &palette);
                cur_pix_w = width;
            }
        }
    }

    Ok(pixels)
}

fn decode_vp8l(data: &[u8]) -> Result<(u32, u32, Vec<u32>), ImageError> {
    if data.len() < 5 {
        return Err(ImageError::UnexpectedEof);
    }
    if data[0] != VP8L_SIGNATURE {
        return Err(ImageError::InvalidHeader("invalid VP8L signature"));
    }
    let bits_val = u32::from_le_bytes(data[1..5].try_into().unwrap());
    let w = (bits_val & 0x3FFF) + 1;
    let h = ((bits_val >> 14) & 0x3FFF) + 1;
    if (bits_val >> 29) != 0 {
        return Err(ImageError::UnsupportedFormat("unsupported VP8L version"));
    }
    validate_dimensions(w, h)?;
    let mut br = Vp8lBitReader::new(&data[5..]);
    let pixels = decode_vp8l_image_stream(&mut br, w as usize, h as usize)?;
    Ok((w, h, pixels))
}

fn decode_vp8l_headerless(data: &[u8], width: u32, height: u32) -> Result<Vec<u32>, ImageError> {
    validate_dimensions(width, height)?;
    let mut br = Vp8lBitReader::new(data);
    decode_vp8l_image_stream(&mut br, width as usize, height as usize)
}

struct Vp8lBitWriter {
    out: Vec<u8>,
    acc: u64,
    count: u32,
}

impl Vp8lBitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            count: 0,
        }
    }

    fn put(&mut self, value: u32, count: u32) {
        if count > 0 {
            let mask = (1u64 << count) - 1;
            self.acc |= (u64::from(value) & mask) << self.count;
            self.count += count;
            while self.count >= 8 {
                self.out.push((self.acc & 0xFF) as u8);
                self.acc >>= 8;
                self.count -= 8;
            }
        }
    }

    fn put_code(&mut self, code: u32, len: u8) {
        for i in (0..len).rev() {
            self.put((code >> i) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

struct EncodeCode {
    lengths: Vec<u8>,
    codes: Vec<u32>,
}

impl EncodeCode {
    fn new(size: usize) -> Self {
        let mut short = 0u32;
        while (1usize << (short + 1)) <= size {
            short += 1;
        }
        let a = (1usize << (short + 1)) - size;
        let mut lengths = vec![0u8; size];
        for (s, len) in lengths.iter_mut().enumerate() {
            *len = if s < a {
                short as u8
            } else {
                (short + 1) as u8
            };
        }
        let mut order: Vec<(usize, u8)> = lengths.iter().copied().enumerate().collect();
        order.sort_by_key(|&(s, l)| (l, s));
        let mut codes = vec![0u32; size];
        let mut next = 0u32;
        let mut prev_len = 0u8;
        for (s, l) in order {
            next <<= l - prev_len;
            prev_len = l;
            codes[s] = next;
            next += 1;
        }
        Self { lengths, codes }
    }

    fn write_header(&self, bw: &mut Vp8lBitWriter) {
        let short = self.lengths[0];
        let long_used = self.lengths.iter().any(|&l| l != short);
        let (zero, one) = if long_used {
            (short, short + 1)
        } else {
            (0, short)
        };
        bw.put(0, 1);
        bw.put(15, 4);
        for &sym in &CODE_LENGTH_ORDER {
            let used = sym == usize::from(zero) || sym == usize::from(one);
            bw.put(u32::from(used), 3);
        }
        bw.put(0, 1);
        for &l in &self.lengths {
            bw.put(u32::from(l != zero), 1);
        }
    }

    fn put(&self, bw: &mut Vp8lBitWriter, sym: usize) {
        bw.put_code(self.codes[sym], self.lengths[sym]);
    }
}

fn encode_prefix(value: usize) -> (usize, u32, u32) {
    let d = (value - 1) as u32;
    if d < 4 {
        return (d as usize, 0, 0);
    }
    let high = 31 - d.leading_zeros();
    let second = (d >> (high - 1)) & 1;
    let n_bits = high - 1;
    (
        (2 * high + second) as usize,
        n_bits,
        d & ((1 << n_bits) - 1),
    )
}

fn encode_vp8l(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let w = width.clamp(1, 16384);
    let h = height.clamp(1, 16384);
    let count = (w as usize) * (h as usize);
    let mut bw = Vp8lBitWriter::new();
    bw.put(u32::from(VP8L_SIGNATURE), 8);
    bw.put(w - 1, 14);
    bw.put(h - 1, 14);

    let has_alpha = (0..count).any(|i| rgba.get(i * 4 + 3).copied().unwrap_or(255) != 255);
    bw.put(u32::from(has_alpha), 1);
    bw.put(0, 3);
    bw.put(1, 1);
    bw.put(2, 2);
    bw.put(0, 1);

    let mut argb = Vec::with_capacity(count);
    for i in 0..count {
        let base = i * 4;
        let r_raw = rgba.get(base).copied().unwrap_or(0);
        let g = rgba.get(base + 1).copied().unwrap_or(0);
        let b_raw = rgba.get(base + 2).copied().unwrap_or(0);
        let a = rgba.get(base + 3).copied().unwrap_or(255);
        let r = r_raw.wrapping_sub(g);
        let b = b_raw.wrapping_sub(g);
        argb.push((u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b));
    }

    const CACHE_BITS: u32 = 6;
    const CACHE_SIZE: usize = 1 << CACHE_BITS;
    bw.put(1, 1);
    bw.put(CACHE_BITS, 4);
    bw.put(0, 1);

    let green = EncodeCode::new(LITERAL_ALPHABET + LENGTH_PREFIXES + CACHE_SIZE);
    let red = EncodeCode::new(LITERAL_ALPHABET);
    let blue = EncodeCode::new(LITERAL_ALPHABET);
    let alpha = EncodeCode::new(LITERAL_ALPHABET);
    let dist = EncodeCode::new(DISTANCE_ALPHABET);
    green.write_header(&mut bw);
    red.write_header(&mut bw);
    blue.write_header(&mut bw);
    alpha.write_header(&mut bw);
    dist.write_header(&mut bw);

    let mut color_cache = [0u32; CACHE_SIZE];
    let mut cache_used = 0u64;
    let stride = w as usize;
    let mut i = 0usize;

    while i < count {
        let px = argb[i];
        let mut best_len = 0usize;
        let mut best_dist = 1usize;
        for cand_d in [1usize, stride] {
            if cand_d > 0 && i >= cand_d && argb[i - cand_d] == px {
                let mut run = 1usize;
                while i + run < count && run < 4096 && argb[i + run] == argb[i + run - cand_d] {
                    run += 1;
                }
                if run > best_len {
                    best_len = run;
                    best_dist = cand_d;
                }
            }
        }
        if best_len > 0 {
            let (l_sym, l_nb, l_extra) = encode_prefix(best_len);
            green.put(&mut bw, LITERAL_ALPHABET + l_sym);
            bw.put(l_extra, l_nb);
            let (d_sym, d_nb, d_extra) = encode_prefix(best_dist + 120);
            dist.put(&mut bw, d_sym);
            bw.put(d_extra, d_nb);
            for k in 0..best_len {
                let p = argb[i + k];
                let idx = (0x1E35_A7BDu32.wrapping_mul(p) >> (32 - CACHE_BITS)) as usize;
                color_cache[idx] = p;
                cache_used |= 1u64 << idx;
            }
            i += best_len;
            continue;
        }
        let idx = (0x1E35_A7BDu32.wrapping_mul(px) >> (32 - CACHE_BITS)) as usize;
        if (cache_used & (1u64 << idx)) != 0 && color_cache[idx] == px {
            green.put(&mut bw, LITERAL_ALPHABET + LENGTH_PREFIXES + idx);
            i += 1;
            continue;
        }
        green.put(&mut bw, ((px >> 8) & 0xFF) as usize);
        red.put(&mut bw, ((px >> 16) & 0xFF) as usize);
        blue.put(&mut bw, (px & 0xFF) as usize);
        alpha.put(&mut bw, (px >> 24) as usize);
        color_cache[idx] = px;
        cache_used |= 1u64 << idx;
        i += 1;
    }
    bw.finish()
}

// ---------- RFC 6386 VP8 Lossy Keyframe Decoder ----------

const KF_YMODE_TREE: [i8; 8] = [-4, 2, 4, 6, 0, -1, -2, -3];
const KF_YMODE_PROBS: [u8; 4] = [145, 156, 163, 128];

const KF_UV_MODE_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];
const KF_UV_MODE_PROBS: [u8; 3] = [142, 114, 183];

const B_MODE_TREE: [i8; 18] = [
    0, 2, -1, 4, -2, 6, 8, 12, -3, 10, -5, -6, -4, 14, -7, 16, -8, -9,
];

const SEGMENT_TREE: [i8; 6] = [2, 4, 0, -1, -2, -3];

const COEFF_TREE: [i8; 22] = [
    -11, 2, 0, 4, -1, 6, 8, 12, -2, 10, -3, -4, 14, 16, -5, -6, 18, 20, -7, -8, -9, -10,
];

const VP8_ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
const VP8_COEFF_BANDS: [usize; 16] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7];

const DC_QLOOKUP: [i16; 128] = [
    4, 5, 6, 7, 8, 9, 10, 10, 11, 12, 13, 14, 15, 16, 17, 17, 18, 19, 20, 20, 21, 21, 22, 22, 23,
    23, 24, 25, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44,
    45, 46, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67,
    68, 69, 70, 71, 72, 73, 74, 75, 76, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91,
    93, 95, 96, 98, 100, 101, 102, 104, 106, 108, 110, 112, 114, 116, 118, 122, 124, 126, 128, 130,
    132, 134, 136, 138, 140, 143, 145, 148, 151, 154, 157,
];

const AC_QLOOKUP: [i16; 128] = [
    4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
    53, 54, 55, 56, 57, 58, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94,
    96, 98, 100, 102, 104, 106, 108, 110, 112, 114, 116, 119, 122, 125, 128, 131, 134, 137, 140,
    143, 146, 149, 152, 155, 158, 161, 164, 167, 170, 173, 177, 181, 185, 189, 193, 197, 201, 205,
    209, 213, 217, 221, 225, 229, 234, 239, 245, 249, 254, 259, 264, 269, 274, 279, 284,
];

const PCAT1: &[u8] = &[159];
const PCAT2: &[u8] = &[165, 145];
const PCAT3: &[u8] = &[173, 148, 140];
const PCAT4: &[u8] = &[176, 155, 140, 135];
const PCAT5: &[u8] = &[180, 157, 141, 134, 130];
const PCAT6: &[u8] = &[254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129];

struct BoolDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    value: u32,
    range: u32,
    bit_count: i32,
    overread: usize,
}

impl<'a> BoolDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut d = Self {
            data,
            pos: 0,
            value: 0,
            range: 255,
            bit_count: 0,
            overread: 0,
        };
        let b0 = u32::from(d.next_byte());
        let b1 = u32::from(d.next_byte());
        d.value = (b0 << 8) | b1;
        d
    }

    fn next_byte(&mut self) -> u8 {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            b
        } else {
            self.overread += 1;
            0
        }
    }

    fn read_bool(&mut self, prob: u8) -> bool {
        let split = 1 + (((self.range - 1) * u32::from(prob)) >> 8);
        let big_split = split << 8;
        let bit = if self.value >= big_split {
            self.range -= split;
            self.value -= big_split;
            true
        } else {
            self.range = split;
            false
        };
        while self.range < 128 {
            self.value <<= 1;
            self.range <<= 1;
            self.bit_count += 1;
            if self.bit_count == 8 {
                self.bit_count = 0;
                self.value |= u32::from(self.next_byte());
            }
        }
        bit
    }

    fn read_literal(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.read_bool(128));
        }
        v
    }

    fn read_signed(&mut self, n: u32) -> i32 {
        let v = self.read_literal(n) as i32;
        if self.read_bool(128) { -v } else { v }
    }

    fn read_tree(&mut self, tree: &[i8], probs: &[u8], start_node: usize) -> usize {
        let mut i = start_node;
        loop {
            let prob = probs[i >> 1];
            let next = if self.read_bool(prob) {
                tree[i + 1]
            } else {
                tree[i]
            };
            if next <= 0 {
                return (-next) as usize;
            }
            i = next as usize;
        }
    }
}

#[derive(Clone, Copy)]
struct Vp8Quant {
    y_dc: i16,
    y_ac: i16,
    y2_dc: i16,
    y2_ac: i16,
    uv_dc: i16,
    uv_ac: i16,
}

impl Vp8Quant {
    fn from_qi(base_q: i32, y1_dc: i32, y2_dc: i32, y2_ac: i32, uv_dc: i32, uv_ac: i32) -> Self {
        let clamp_q = |q: i32| q.clamp(0, 127) as usize;
        Self {
            y_dc: DC_QLOOKUP[clamp_q(base_q + y1_dc)],
            y_ac: AC_QLOOKUP[clamp_q(base_q)],
            y2_dc: DC_QLOOKUP[clamp_q(base_q + y2_dc)].saturating_mul(2),
            y2_ac: ((i32::from(AC_QLOOKUP[clamp_q(base_q + y2_ac)]) * 155) / 100) as i16,
            uv_dc: DC_QLOOKUP[clamp_q(base_q + uv_dc)],
            uv_ac: AC_QLOOKUP[clamp_q(base_q + uv_ac)],
        }
    }
}

fn vp8_inverse_idct4x4(input: &[i32; 16]) -> [i16; 16] {
    const COSPI8SQRT2MINUS1: i32 = 20091;
    const SINPI8SQRT2: i32 = 35468;
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a1 = input[i] + input[8 + i];
        let b1 = input[i] - input[8 + i];
        let t1 = (input[4 + i] * SINPI8SQRT2) >> 16;
        let t2 = input[12 + i] + ((input[12 + i] * COSPI8SQRT2MINUS1) >> 16);
        let c1 = t1 - t2;
        let t1 = input[4 + i] + ((input[4 + i] * COSPI8SQRT2MINUS1) >> 16);
        let t2 = (input[12 + i] * SINPI8SQRT2) >> 16;
        let d1 = t1 + t2;
        tmp[i * 4] = a1 + d1;
        tmp[i * 4 + 3] = a1 - d1;
        tmp[i * 4 + 1] = b1 + c1;
        tmp[i * 4 + 2] = b1 - c1;
    }
    let mut out = [0i16; 16];
    for i in 0..4 {
        let a1 = tmp[i] + tmp[8 + i];
        let b1 = tmp[i] - tmp[8 + i];
        let t1 = (tmp[4 + i] * SINPI8SQRT2) >> 16;
        let t2 = tmp[12 + i] + ((tmp[12 + i] * COSPI8SQRT2MINUS1) >> 16);
        let c1 = t1 - t2;
        let t1 = tmp[4 + i] + ((tmp[4 + i] * COSPI8SQRT2MINUS1) >> 16);
        let t2 = (tmp[12 + i] * SINPI8SQRT2) >> 16;
        let d1 = t1 + t2;
        out[i * 4] = ((a1 + d1 + 4) >> 3).clamp(-255, 255) as i16;
        out[i * 4 + 3] = ((a1 - d1 + 4) >> 3).clamp(-255, 255) as i16;
        out[i * 4 + 1] = ((b1 + c1 + 4) >> 3).clamp(-255, 255) as i16;
        out[i * 4 + 2] = ((b1 - c1 + 4) >> 3).clamp(-255, 255) as i16;
    }
    out
}

fn vp8_inverse_wht4x4(input: &[i32; 16]) -> [i32; 16] {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a1 = input[i] + input[12 + i];
        let b1 = input[4 + i] + input[8 + i];
        let c1 = input[4 + i] - input[8 + i];
        let d1 = input[i] - input[12 + i];
        tmp[i] = a1 + b1;
        tmp[4 + i] = c1 + d1;
        tmp[8 + i] = a1 - b1;
        tmp[12 + i] = d1 - c1;
    }
    let mut out = [0i32; 16];
    for i in 0..4 {
        let base = i * 4;
        let a1 = tmp[base] + tmp[base + 3];
        let b1 = tmp[base + 1] + tmp[base + 2];
        let c1 = tmp[base + 1] - tmp[base + 2];
        let d1 = tmp[base] - tmp[base + 3];
        out[base] = (a1 + b1 + 3) >> 3;
        out[base + 1] = (c1 + d1 + 3) >> 3;
        out[base + 2] = (a1 - b1 + 3) >> 3;
        out[base + 3] = (d1 - c1 + 3) >> 3;
    }
    out
}

fn decode_vp8_coeffs(
    dec: &mut BoolDecoder<'_>,
    probs: &[[[[u8; 11]; 3]; 8]; 4],
    plane_type: usize,
    first_coeff: usize,
    initial_ctx: usize,
    dc_q: i16,
    ac_q: i16,
) -> ([i32; 16], bool) {
    let mut out = [0i32; 16];
    let mut ctx = initial_ctx.min(2);
    let mut prev_zero = false;
    let mut has_nz = false;
    for i in first_coeff..16 {
        let band = VP8_COEFF_BANDS[i];
        let tbl = &probs[plane_type][band][ctx];
        let start = if prev_zero { 2 } else { 0 };
        let tok = dec.read_tree(&COEFF_TREE, tbl, start);
        if tok == 11 {
            break;
        }
        if tok == 0 {
            ctx = 0;
            prev_zero = true;
            continue;
        }
        prev_zero = false;
        has_nz = true;
        let read_extra = |d: &mut BoolDecoder<'_>, ptbl: &[u8]| -> i32 {
            let mut v = 0i32;
            for &p in ptbl {
                v = (v << 1) | i32::from(d.read_bool(p));
            }
            v
        };
        let abs_v = match tok {
            1..=4 => tok as i32,
            5 => 5 + read_extra(dec, PCAT1),
            6 => 7 + read_extra(dec, PCAT2),
            7 => 11 + read_extra(dec, PCAT3),
            8 => 19 + read_extra(dec, PCAT4),
            9 => 35 + read_extra(dec, PCAT5),
            _ => 67 + read_extra(dec, PCAT6),
        };
        ctx = if abs_v == 1 { 1 } else { 2 };
        let signed = if dec.read_bool(128) { -abs_v } else { abs_v };
        let zz = VP8_ZIGZAG[i];
        let q = i32::from(if zz == 0 { dc_q } else { ac_q });
        out[zz] = signed * q;
    }
    (out, has_nz)
}

fn predict_intra_mb(
    mode: usize,
    n: usize,
    above: Option<&[u8]>,
    left: Option<&[u8]>,
    top_left: u8,
    dst: &mut [u8],
    stride: usize,
) {
    let def_a = [127u8; 16];
    let def_l = [129u8; 16];
    match mode {
        1 => {
            let row = above.unwrap_or(&def_a[..n]);
            for y in 0..n {
                dst[y * stride..y * stride + n].copy_from_slice(&row[..n]);
            }
        }
        2 => {
            let col = left.unwrap_or(&def_l[..n]);
            for y in 0..n {
                let v = col[y];
                dst[y * stride..y * stride + n].fill(v);
            }
        }
        3 => {
            let row = above.unwrap_or(&def_a[..n]);
            let col = left.unwrap_or(&def_l[..n]);
            let p = i32::from(top_left);
            for y in 0..n {
                let l = i32::from(col[y]);
                for x in 0..n {
                    dst[y * stride + x] = (l + i32::from(row[x]) - p).clamp(0, 255) as u8;
                }
            }
        }
        _ => {
            let dc = match (above, left) {
                (Some(a), Some(l)) => {
                    let sum: u32 = a[..n]
                        .iter()
                        .chain(l[..n].iter())
                        .map(|&v| u32::from(v))
                        .sum();
                    ((sum + n as u32) / (2 * n as u32)) as u8
                }
                (Some(a), None) => {
                    let sum: u32 = a[..n].iter().map(|&v| u32::from(v)).sum();
                    ((sum + (n / 2) as u32) / n as u32) as u8
                }
                (None, Some(l)) => {
                    let sum: u32 = l[..n].iter().map(|&v| u32::from(v)).sum();
                    ((sum + (n / 2) as u32) / n as u32) as u8
                }
                (None, None) => 128,
            };
            for y in 0..n {
                dst[y * stride..y * stride + n].fill(dc);
            }
        }
    }
}

fn predict_intra_4x4(mode: usize, above: &[u8; 8], left: &[u8; 4], top_left: u8) -> [u8; 16] {
    let avg2 = |a: u8, b: u8| -> u8 { ((u16::from(a) + u16::from(b) + 1) >> 1) as u8 };
    let avg3 = |a: u8, b: u8, c: u8| -> u8 {
        ((u16::from(a) + 2 * u16::from(b) + u16::from(c) + 2) >> 2) as u8
    };
    let mut out = [128u8; 16];
    let e = [
        left[3], left[2], left[1], left[0], top_left, above[0], above[1], above[2], above[3],
    ];
    match mode {
        0 => {
            let sum: u32 = above[..4]
                .iter()
                .chain(left.iter())
                .map(|&v| u32::from(v))
                .sum();
            out.fill(((sum + 4) >> 3) as u8);
        }
        1 => {
            let p = i32::from(top_left);
            for r in 0..4 {
                for c in 0..4 {
                    out[r * 4 + c] =
                        (i32::from(left[r]) + i32::from(above[c]) - p).clamp(0, 255) as u8;
                }
            }
        }
        2 => {
            let row = [
                avg3(top_left, above[0], above[1]),
                avg3(above[0], above[1], above[2]),
                avg3(above[1], above[2], above[3]),
                avg3(above[2], above[3], above[4]),
            ];
            for r in 0..4 {
                out[r * 4..r * 4 + 4].copy_from_slice(&row);
            }
        }
        3 => {
            let col = [
                avg3(top_left, left[0], left[1]),
                avg3(left[0], left[1], left[2]),
                avg3(left[1], left[2], left[3]),
                avg3(left[2], left[3], left[3]),
            ];
            for r in 0..4 {
                out[r * 4..r * 4 + 4].fill(col[r]);
            }
        }
        4 => {
            for r in 0..4 {
                for c in 0..4 {
                    let i = r + c;
                    out[r * 4 + c] = if i == 6 {
                        avg3(above[6], above[7], above[7])
                    } else {
                        avg3(above[i], above[i + 1], above[i + 2])
                    };
                }
            }
        }
        5 => {
            out[3 * 4] = avg3(e[0], e[1], e[2]);
            out[3 * 4 + 1] = avg3(e[1], e[2], e[3]);
            out[2 * 4] = out[3 * 4 + 1];
            out[3 * 4 + 2] = avg3(e[2], e[3], e[4]);
            out[2 * 4 + 1] = out[3 * 4 + 2];
            out[4] = out[3 * 4 + 2];
            out[3 * 4 + 3] = avg3(e[3], e[4], e[5]);
            out[2 * 4 + 2] = out[3 * 4 + 3];
            out[4 + 1] = out[3 * 4 + 3];
            out[0] = out[3 * 4 + 3];
            out[2 * 4 + 3] = avg3(e[4], e[5], e[6]);
            out[4 + 2] = out[2 * 4 + 3];
            out[1] = out[2 * 4 + 3];
            out[4 + 3] = avg3(e[5], e[6], e[7]);
            out[2] = out[4 + 3];
            out[3] = avg3(e[6], e[7], e[8]);
        }
        6 => {
            out[3 * 4] = avg3(e[1], e[2], e[3]);
            out[2 * 4] = avg3(e[2], e[3], e[4]);
            out[3 * 4 + 1] = avg3(e[3], e[4], e[5]);
            out[4] = out[3 * 4 + 1];
            out[2 * 4 + 1] = avg2(e[4], e[5]);
            out[0] = out[2 * 4 + 1];
            out[3 * 4 + 2] = avg3(e[4], e[5], e[6]);
            out[4 + 1] = out[3 * 4 + 2];
            out[2 * 4 + 2] = avg2(e[5], e[6]);
            out[1] = out[2 * 4 + 2];
            out[3 * 4 + 3] = avg3(e[5], e[6], e[7]);
            out[4 + 2] = out[3 * 4 + 3];
            out[2 * 4 + 3] = avg2(e[6], e[7]);
            out[2] = out[2 * 4 + 3];
            out[4 + 3] = avg3(e[6], e[7], e[8]);
            out[3] = avg2(e[7], e[8]);
        }
        7 => {
            out[0] = avg2(above[0], above[1]);
            out[4] = avg3(above[0], above[1], above[2]);
            out[2 * 4] = avg2(above[1], above[2]);
            out[1] = out[2 * 4];
            out[4 + 1] = avg3(above[1], above[2], above[3]);
            out[3 * 4] = out[4 + 1];
            out[2 * 4 + 1] = avg2(above[2], above[3]);
            out[2] = out[2 * 4 + 1];
            out[3 * 4 + 1] = avg3(above[2], above[3], above[4]);
            out[4 + 2] = out[3 * 4 + 1];
            out[2 * 4 + 2] = avg2(above[3], above[4]);
            out[3] = out[2 * 4 + 2];
            out[3 * 4 + 2] = avg3(above[3], above[4], above[5]);
            out[4 + 3] = out[3 * 4 + 2];
            out[2 * 4 + 3] = avg3(above[4], above[5], above[6]);
            out[3 * 4 + 3] = avg3(above[5], above[6], above[7]);
        }
        8 => {
            out[3 * 4] = avg2(e[0], e[1]);
            out[3 * 4 + 1] = avg3(e[0], e[1], e[2]);
            out[2 * 4] = avg2(e[1], e[2]);
            out[3 * 4 + 2] = out[2 * 4];
            out[2 * 4 + 1] = avg3(e[1], e[2], e[3]);
            out[3 * 4 + 3] = out[2 * 4 + 1];
            out[2 * 4 + 2] = avg2(e[2], e[3]);
            out[4] = out[2 * 4 + 2];
            out[2 * 4 + 3] = avg3(e[2], e[3], e[4]);
            out[4 + 1] = out[2 * 4 + 3];
            out[4 + 2] = avg2(e[3], e[4]);
            out[0] = out[4 + 2];
            out[4 + 3] = avg3(e[3], e[4], e[5]);
            out[1] = out[4 + 3];
            out[2] = avg3(e[4], e[5], e[6]);
            out[3] = avg3(e[5], e[6], e[7]);
        }
        _ => {
            out[0] = avg2(left[0], left[1]);
            out[1] = avg3(left[0], left[1], left[2]);
            out[2] = avg2(left[1], left[2]);
            out[4] = out[2];
            out[3] = avg3(left[1], left[2], left[3]);
            out[4 + 1] = out[3];
            out[4 + 2] = avg2(left[2], left[3]);
            out[2 * 4] = out[4 + 2];
            out[4 + 3] = avg3(left[2], left[3], left[3]);
            out[2 * 4 + 1] = out[4 + 3];
            out[2 * 4 + 2] = left[3];
            out[2 * 4 + 3] = left[3];
            out[3 * 4..3 * 4 + 4].fill(left[3]);
        }
    }
    out
}

fn mb_y_to_bmode(ymode: usize) -> usize {
    match ymode {
        0 => 0,
        1 => 2,
        2 => 3,
        3 => 1,
        _ => 0,
    }
}

struct Vp8Frame {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    y_stride: usize,
    uv_stride: usize,
}

impl Vp8Frame {
    fn to_rgba(&self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            let near_y = y / 2;
            let far_y = if y % 2 == 0 {
                near_y.saturating_sub(1)
            } else {
                (near_y + 1).min(ch - 1)
            };
            for x in 0..w {
                let near_x = x / 2;
                let far_x = if x % 2 == 0 {
                    near_x.saturating_sub(1)
                } else {
                    (near_x + 1).min(cw - 1)
                };
                let sample = |plane: &[u8], stride: usize| -> i32 {
                    let at = |cx: usize, cy: usize| i32::from(plane[cy * stride + cx]);
                    (9 * at(near_x, near_y)
                        + 3 * at(far_x, near_y)
                        + 3 * at(near_x, far_y)
                        + at(far_x, far_y)
                        + 8)
                        >> 4
                };
                let u_val = sample(&self.u, self.uv_stride);
                let v_val = sample(&self.v, self.uv_stride);
                let luma = i32::from(self.y[y * self.y_stride + x]);
                let c = 76309 * (luma - 16);
                let d = u_val - 128;
                let e = v_val - 128;
                let round = 1i32 << 15;
                let o = (y * w + x) * 4;
                out[o] = ((c + 104_597 * e + round) >> 16).clamp(0, 255) as u8;
                out[o + 1] = ((c - 25_675 * d - 53_279 * e + round) >> 16).clamp(0, 255) as u8;
                out[o + 2] = ((c + 132_201 * d + round) >> 16).clamp(0, 255) as u8;
                out[o + 3] = 255;
            }
        }
        out
    }
}

fn decode_vp8(data: &[u8]) -> Result<Vp8Frame, ImageError> {
    if data.len() < 10 {
        return Err(ImageError::UnexpectedEof);
    }
    let tag = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
    if (tag & 1) != 0 {
        return Err(ImageError::UnsupportedFormat(
            "VP8 interframes are not supported",
        ));
    }
    if ((tag >> 1) & 7) > 3 {
        return Err(ImageError::UnsupportedFormat("unknown VP8 version"));
    }
    if data[3] != 0x9D || data[4] != 0x01 || data[5] != 0x2A {
        return Err(ImageError::InvalidHeader("missing VP8 start code"));
    }
    let w = u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3FFF);
    let h = u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3FFF);
    validate_dimensions(w, h)?;

    let first_size = (tag >> 5) as usize;
    let body = &data[10..];
    if first_size > body.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let mut p0 = BoolDecoder::new(&body[..first_size]);
    let _color_space = p0.read_bool(128);
    let _clamping = p0.read_bool(128);

    let seg_enabled = p0.read_bool(128);
    let mut update_map = false;
    let mut update_data = false;
    let mut abs_delta = false;
    let mut seg_quant = [0i8; 4];
    let mut seg_probs = [255u8; 3];
    if seg_enabled {
        update_map = p0.read_bool(128);
        update_data = p0.read_bool(128);
        if update_data {
            abs_delta = p0.read_bool(128);
            for q in &mut seg_quant {
                if p0.read_bool(128) {
                    *q = p0.read_signed(7) as i8;
                }
            }
            for _ in 0..4 {
                if p0.read_bool(128) {
                    let _ = p0.read_signed(6);
                }
            }
        }
        if update_map {
            for p in &mut seg_probs {
                if p0.read_bool(128) {
                    *p = p0.read_literal(8) as u8;
                }
            }
        }
    }

    let _simple_filter = p0.read_bool(128);
    let _filter_level = p0.read_literal(6);
    let _sharpness = p0.read_literal(3);
    if p0.read_bool(128) && p0.read_bool(128) {
        for _ in 0..8 {
            if p0.read_bool(128) {
                let _ = p0.read_signed(6);
            }
        }
    }

    let partitions_log2 = p0.read_literal(2);
    let num_partitions = 1usize << partitions_log2;
    let token_data = &body[first_size..];
    let mut parts: Vec<&[u8]> = Vec::with_capacity(num_partitions);
    if num_partitions > 1 {
        let tbl_bytes = (num_partitions - 1) * 3;
        if token_data.len() < tbl_bytes {
            return Err(ImageError::UnexpectedEof);
        }
        let mut sizes = Vec::with_capacity(num_partitions - 1);
        for i in 0..num_partitions - 1 {
            sizes.push(read_u24(&token_data[i * 3..i * 3 + 3]) as usize);
        }
        let mut cur = tbl_bytes;
        for sz in sizes {
            if cur + sz > token_data.len() {
                return Err(ImageError::UnexpectedEof);
            }
            parts.push(&token_data[cur..cur + sz]);
            cur += sz;
        }
        parts.push(&token_data[cur..]);
    } else {
        parts.push(token_data);
    }

    let base_q = p0.read_literal(7) as i32;
    let read_q_delta = |d: &mut BoolDecoder<'_>| -> i32 {
        if d.read_bool(128) {
            d.read_signed(4)
        } else {
            0
        }
    };
    let y1_dc = read_q_delta(&mut p0);
    let y2_dc = read_q_delta(&mut p0);
    let y2_ac = read_q_delta(&mut p0);
    let uv_dc = read_q_delta(&mut p0);
    let uv_ac = read_q_delta(&mut p0);

    let mut quants = [Vp8Quant::from_qi(base_q, y1_dc, y2_dc, y2_ac, uv_dc, uv_ac); 4];
    for (i, slot) in quants.iter_mut().enumerate() {
        if seg_enabled && update_data {
            let q = if abs_delta {
                i32::from(seg_quant[i])
            } else {
                base_q + i32::from(seg_quant[i])
            };
            *slot = Vp8Quant::from_qi(q, y1_dc, y2_dc, y2_ac, uv_dc, uv_ac);
        }
    }

    let _refresh_entropy = p0.read_bool(128);
    let mut coeff_probs = DEFAULT_COEFF_PROBS;
    for i in 0..4 {
        for j in 0..8 {
            for k in 0..3 {
                for t in 0..11 {
                    if p0.read_bool(COEFF_UPDATE_PROBS[i][j][k][t]) {
                        coeff_probs[i][j][k][t] = p0.read_literal(8) as u8;
                    }
                }
            }
        }
    }

    let mb_no_skip_coeff = p0.read_bool(128);
    let prob_skip_false = if mb_no_skip_coeff {
        p0.read_literal(8) as u8
    } else {
        128
    };
    if p0.overread > 4 {
        return Err(ImageError::UnexpectedEof);
    }

    let mb_cols = (w as usize).div_ceil(16);
    let mb_rows = (h as usize).div_ceil(16);
    let y_stride = mb_cols * 16;
    let uv_stride = mb_cols * 8;
    let mut recon_y = vec![128u8; y_stride * mb_rows * 16];
    let mut recon_u = vec![128u8; uv_stride * mb_rows * 8];
    let mut recon_v = vec![128u8; uv_stride * mb_rows * 8];

    let mut part_decoders: Vec<BoolDecoder<'_>> =
        parts.iter().map(|s| BoolDecoder::new(s)).collect();

    let mut above_bmodes = vec![0usize; mb_cols * 4];
    let mut above_nz_y = vec![0u8; mb_cols * 4];
    let mut above_nz_u = vec![0u8; mb_cols * 2];
    let mut above_nz_v = vec![0u8; mb_cols * 2];
    let mut above_nz_y2 = vec![0u8; mb_cols];

    for mbr in 0..mb_rows {
        let p1 = &mut part_decoders[mbr % parts.len()];
        let mut left_bmodes = [0usize; 4];
        let mut left_nz_y = [0u8; 4];
        let mut left_nz_u = [0u8; 2];
        let mut left_nz_v = [0u8; 2];
        let mut left_nz_y2 = 0u8;

        for mbc in 0..mb_cols {
            let seg_id = if seg_enabled && update_map {
                p0.read_tree(&SEGMENT_TREE, &seg_probs, 0)
            } else {
                0
            };
            let q = quants[seg_id];
            let mb_skip = if mb_no_skip_coeff {
                p0.read_bool(prob_skip_false)
            } else {
                false
            };

            let ymode = p0.read_tree(&KF_YMODE_TREE, &KF_YMODE_PROBS, 0);
            let mut sub_modes = [0usize; 16];
            if ymode == 4 {
                for s in 0..16 {
                    let sr = s / 4;
                    let sc = s % 4;
                    let a = if sr == 0 {
                        above_bmodes[mbc * 4 + sc]
                    } else {
                        sub_modes[(sr - 1) * 4 + sc]
                    };
                    let l = if sc == 0 {
                        left_bmodes[sr]
                    } else {
                        sub_modes[sr * 4 + (sc - 1)]
                    };
                    let mode = p0.read_tree(&B_MODE_TREE, &KF_BMODE_PROBS[a][l], 0);
                    sub_modes[s] = mode;
                }
                for k in 0..4 {
                    above_bmodes[mbc * 4 + k] = sub_modes[12 + k];
                    left_bmodes[k] = sub_modes[k * 4 + 3];
                }
            } else {
                let bm = mb_y_to_bmode(ymode);
                for k in 0..4 {
                    above_bmodes[mbc * 4 + k] = bm;
                    left_bmodes[k] = bm;
                }
            }
            let uv_mode = p0.read_tree(&KF_UV_MODE_TREE, &KF_UV_MODE_PROBS, 0);

            let y_x0 = mbc * 16;
            let y_y0 = mbr * 16;
            let uv_x0 = mbc * 8;
            let uv_y0 = mbr * 8;

            // Decode coefficients
            let mut y2_dc = [0i32; 16];
            let mut y_coeffs = [[0i32; 16]; 16];
            let mut u_coeffs = [[0i32; 16]; 4];
            let mut v_coeffs = [[0i32; 16]; 4];

            if mb_skip {
                if ymode != 4 {
                    above_nz_y2[mbc] = 0;
                    left_nz_y2 = 0;
                }
                for k in 0..4 {
                    above_nz_y[mbc * 4 + k] = 0;
                    left_nz_y[k] = 0;
                }
                for k in 0..2 {
                    above_nz_u[mbc * 2 + k] = 0;
                    left_nz_u[k] = 0;
                    above_nz_v[mbc * 2 + k] = 0;
                    left_nz_v[k] = 0;
                }
            } else {
                if ymode != 4 {
                    let ctx = usize::from(above_nz_y2[mbc]) + usize::from(left_nz_y2);
                    let (raw_y2, nz) =
                        decode_vp8_coeffs(p1, &coeff_probs, 1, 0, ctx, q.y2_dc, q.y2_ac);
                    let flag = u8::from(nz);
                    above_nz_y2[mbc] = flag;
                    left_nz_y2 = flag;
                    y2_dc = vp8_inverse_wht4x4(&raw_y2);
                }
                let (plane_type, first_c) = if ymode == 4 { (3, 0) } else { (0, 1) };
                for s in 0..16 {
                    let sr = s / 4;
                    let sc = s % 4;
                    let ctx = usize::from(above_nz_y[mbc * 4 + sc]) + usize::from(left_nz_y[sr]);
                    let (mut blk, mut nz) = decode_vp8_coeffs(
                        p1,
                        &coeff_probs,
                        plane_type,
                        first_c,
                        ctx,
                        q.y_dc,
                        q.y_ac,
                    );
                    if ymode != 4 {
                        blk[0] = y2_dc[s];
                        if blk[0] != 0 {
                            nz = true;
                        }
                    }
                    let flag = u8::from(nz);
                    above_nz_y[mbc * 4 + sc] = flag;
                    left_nz_y[sr] = flag;
                    y_coeffs[s] = blk;
                }
                for (s, slot) in u_coeffs.iter_mut().enumerate() {
                    let sr = s / 2;
                    let sc = s % 2;
                    let ctx = usize::from(above_nz_u[mbc * 2 + sc]) + usize::from(left_nz_u[sr]);
                    let (blk, nz) =
                        decode_vp8_coeffs(p1, &coeff_probs, 2, 0, ctx, q.uv_dc, q.uv_ac);
                    let flag = u8::from(nz);
                    above_nz_u[mbc * 2 + sc] = flag;
                    left_nz_u[sr] = flag;
                    *slot = blk;
                }
                for (s, slot) in v_coeffs.iter_mut().enumerate() {
                    let sr = s / 2;
                    let sc = s % 2;
                    let ctx = usize::from(above_nz_v[mbc * 2 + sc]) + usize::from(left_nz_v[sr]);
                    let (blk, nz) =
                        decode_vp8_coeffs(p1, &coeff_probs, 2, 0, ctx, q.uv_dc, q.uv_ac);
                    let flag = u8::from(nz);
                    above_nz_v[mbc * 2 + sc] = flag;
                    left_nz_v[sr] = flag;
                    *slot = blk;
                }
            }

            // Predict & reconstruct Y
            if ymode != 4 {
                let mut above_row = [127u8; 16];
                let mut left_col = [129u8; 16];
                let above_opt = (mbr > 0).then(|| {
                    above_row.copy_from_slice(&recon_y[(y_y0 - 1) * y_stride + y_x0..][..16]);
                    above_row.as_slice()
                });
                let left_opt = (mbc > 0).then(|| {
                    for r in 0..16 {
                        left_col[r] = recon_y[(y_y0 + r) * y_stride + (y_x0 - 1)];
                    }
                    left_col.as_slice()
                });
                let top_left = match (mbr > 0, mbc > 0) {
                    (true, true) => recon_y[(y_y0 - 1) * y_stride + (y_x0 - 1)],
                    (true, false) => 129,
                    _ => 127,
                };
                let mut pred = [128u8; 256];
                predict_intra_mb(ymode, 16, above_opt, left_opt, top_left, &mut pred, 16);
                for (s, coeffs) in y_coeffs.iter().enumerate() {
                    let sr = s / 4;
                    let sc = s % 4;
                    let res = vp8_inverse_idct4x4(coeffs);
                    for r in 0..4 {
                        for c in 0..4 {
                            let py = y_y0 + sr * 4 + r;
                            let px = y_x0 + sc * 4 + c;
                            let pv =
                                i16::from(pred[(sr * 4 + r) * 16 + sc * 4 + c]) + res[r * 4 + c];
                            recon_y[py * y_stride + px] = pv.clamp(0, 255) as u8;
                        }
                    }
                }
            } else {
                let mut top_right_row = [127u8; 4];
                if mbr > 0 {
                    if mbc + 1 < mb_cols {
                        top_right_row
                            .copy_from_slice(&recon_y[(y_y0 - 1) * y_stride + y_x0 + 16..][..4]);
                    } else {
                        top_right_row.fill(recon_y[(y_y0 - 1) * y_stride + y_x0 + 15]);
                    }
                }
                for s in 0..16 {
                    let sr = s / 4;
                    let sc = s % 4;
                    let bx = y_x0 + sc * 4;
                    let by = y_y0 + sr * 4;
                    let mut above = [127u8; 8];
                    let mut left = [129u8; 4];
                    if by > 0 {
                        let a_row = (by - 1) * y_stride;
                        above[..4].copy_from_slice(&recon_y[a_row + bx..][..4]);
                        if sc < 3 {
                            above[4..8].copy_from_slice(&recon_y[a_row + bx + 4..][..4]);
                        } else {
                            above[4..8].copy_from_slice(&top_right_row);
                        }
                    }
                    if bx > 0 {
                        for r in 0..4 {
                            left[r] = recon_y[(by + r) * y_stride + (bx - 1)];
                        }
                    }
                    let top_left = match (by > 0, bx > 0) {
                        (true, true) => recon_y[(by - 1) * y_stride + (bx - 1)],
                        (true, false) => 129,
                        _ => 127,
                    };
                    let pred = predict_intra_4x4(sub_modes[s], &above, &left, top_left);
                    let res = vp8_inverse_idct4x4(&y_coeffs[s]);
                    for r in 0..4 {
                        for c in 0..4 {
                            let pv = i16::from(pred[r * 4 + c]) + res[r * 4 + c];
                            recon_y[(by + r) * y_stride + bx + c] = pv.clamp(0, 255) as u8;
                        }
                    }
                }
            }

            // Predict & reconstruct U & V
            let reconstruct_uv = |plane: &mut Vec<u8>, coeffs: &[[i32; 16]; 4]| {
                let mut above_row = [127u8; 8];
                let mut left_col = [129u8; 8];
                let above_opt = (mbr > 0).then(|| {
                    above_row.copy_from_slice(&plane[(uv_y0 - 1) * uv_stride + uv_x0..][..8]);
                    above_row.as_slice()
                });
                let left_opt = (mbc > 0).then(|| {
                    for r in 0..8 {
                        left_col[r] = plane[(uv_y0 + r) * uv_stride + (uv_x0 - 1)];
                    }
                    left_col.as_slice()
                });
                let top_left = match (mbr > 0, mbc > 0) {
                    (true, true) => plane[(uv_y0 - 1) * uv_stride + (uv_x0 - 1)],
                    (true, false) => 129,
                    _ => 127,
                };
                let mut pred = [128u8; 64];
                predict_intra_mb(uv_mode, 8, above_opt, left_opt, top_left, &mut pred, 8);
                for (s, blk) in coeffs.iter().enumerate() {
                    let sr = s / 2;
                    let sc = s % 2;
                    let res = vp8_inverse_idct4x4(blk);
                    for r in 0..4 {
                        for c in 0..4 {
                            let py = uv_y0 + sr * 4 + r;
                            let px = uv_x0 + sc * 4 + c;
                            let pv =
                                i16::from(pred[(sr * 4 + r) * 8 + sc * 4 + c]) + res[r * 4 + c];
                            plane[py * uv_stride + px] = pv.clamp(0, 255) as u8;
                        }
                    }
                }
            };
            reconstruct_uv(&mut recon_u, &u_coeffs);
            reconstruct_uv(&mut recon_v, &v_coeffs);
        }
    }

    Ok(Vp8Frame {
        width: w,
        height: h,
        y: recon_y,
        u: recon_u,
        v: recon_v,
        y_stride,
        uv_stride,
    })
}
