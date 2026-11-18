// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Clean-room RFC 7932 Section 8 / Appendix A / Appendix B Brotli static dictionary
//! and word transformation engine.

/// Raw RFC 7932 Appendix A static dictionary data (`122,784` bytes).
pub const STATIC_DICT: &[u8; 122_784] = include_bytes!("rfc7932_dict.bin");

/// Bit-depth `NDBITS[length]` for dictionary word lengths `0..=24` (RFC 7932 Appendix A).
pub const NDBITS: [u8; 25] = [
    0, 0, 0, 0, 10, 10, 11, 11, 10, 10, 10, 10, 10, 9, 9, 8, 7, 7, 8, 7, 7, 6, 6, 5, 5,
];

/// Precomputed `DOFFSET[length]` table derived from `NDBITS` (RFC 7932 Section 8).
pub const DOFFSET: [usize; 25] = {
    let mut offsets = [0usize; 25];
    let mut len = 0usize;
    while len < 24 {
        let nwords = if len < 4 { 0 } else { 1usize << NDBITS[len] };
        offsets[len + 1] = offsets[len] + len * nwords;
        len += 1;
    }
    offsets
};

/// Elementary word transformation operation (`0..=20`, RFC 7932 Section 8 & Appendix B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WordOp {
    Identity = 0,
    FermentFirst = 1,
    FermentAll = 2,
    OmitFirst1 = 3,
    OmitFirst2 = 4,
    OmitFirst3 = 5,
    OmitFirst4 = 6,
    OmitFirst5 = 7,
    OmitFirst6 = 8,
    OmitFirst7 = 9,
    OmitFirst8 = 10,
    OmitFirst9 = 11,
    OmitLast1 = 12,
    OmitLast2 = 13,
    OmitLast3 = 14,
    OmitLast4 = 15,
    OmitLast5 = 16,
    OmitLast6 = 17,
    OmitLast7 = 18,
    OmitLast8 = 19,
    OmitLast9 = 20,
}

/// One static-dictionary word transform entry from RFC 7932 Appendix B.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub prefix: &'static [u8],
    pub op: WordOp,
    pub suffix: &'static [u8],
}

impl Transform {
    const fn new(prefix: &'static [u8], op: WordOp, suffix: &'static [u8]) -> Self {
        Self { prefix, op, suffix }
    }
}

/// Complete list of `121` RFC 7932 Appendix B word transformations.
pub const TRANSFORMS: [Transform; 121] = [
    Transform::new(b"", WordOp::Identity, b""),          // 0
    Transform::new(b"", WordOp::Identity, b" "),         // 1
    Transform::new(b" ", WordOp::Identity, b" "),        // 2
    Transform::new(b"", WordOp::OmitFirst1, b""),        // 3
    Transform::new(b"", WordOp::FermentFirst, b" "),     // 4
    Transform::new(b"", WordOp::Identity, b" the "),     // 5
    Transform::new(b" ", WordOp::Identity, b""),         // 6
    Transform::new(b"s ", WordOp::Identity, b" "),       // 7
    Transform::new(b"", WordOp::Identity, b" of "),      // 8
    Transform::new(b"", WordOp::FermentFirst, b""),      // 9
    Transform::new(b"", WordOp::Identity, b" and "),     // 10
    Transform::new(b"", WordOp::OmitFirst2, b""),        // 11
    Transform::new(b"", WordOp::OmitLast1, b""),         // 12
    Transform::new(b", ", WordOp::Identity, b" "),       // 13
    Transform::new(b"", WordOp::Identity, b", "),        // 14
    Transform::new(b" ", WordOp::FermentFirst, b" "),    // 15
    Transform::new(b"", WordOp::Identity, b" in "),      // 16
    Transform::new(b"", WordOp::Identity, b" to "),      // 17
    Transform::new(b"e ", WordOp::Identity, b" "),       // 18
    Transform::new(b"", WordOp::Identity, b"\""),        // 19
    Transform::new(b"", WordOp::Identity, b"."),         // 20
    Transform::new(b"", WordOp::Identity, b"\">"),       // 21
    Transform::new(b"", WordOp::Identity, b"\n"),        // 22
    Transform::new(b"", WordOp::OmitLast3, b""),         // 23
    Transform::new(b"", WordOp::Identity, b"]"),         // 24
    Transform::new(b"", WordOp::Identity, b" for "),     // 25
    Transform::new(b"", WordOp::OmitFirst3, b""),        // 26
    Transform::new(b"", WordOp::OmitLast2, b""),         // 27
    Transform::new(b"", WordOp::Identity, b" a "),       // 28
    Transform::new(b"", WordOp::Identity, b" that "),    // 29
    Transform::new(b" ", WordOp::FermentFirst, b""),     // 30
    Transform::new(b"", WordOp::Identity, b". "),        // 31
    Transform::new(b".", WordOp::Identity, b""),         // 32
    Transform::new(b" ", WordOp::Identity, b", "),       // 33
    Transform::new(b"", WordOp::OmitFirst4, b""),        // 34
    Transform::new(b"", WordOp::Identity, b" with "),    // 35
    Transform::new(b"", WordOp::Identity, b"'"),         // 36
    Transform::new(b"", WordOp::Identity, b" from "),    // 37
    Transform::new(b"", WordOp::Identity, b" by "),      // 38
    Transform::new(b"", WordOp::OmitFirst5, b""),        // 39
    Transform::new(b"", WordOp::OmitFirst6, b""),        // 40
    Transform::new(b" the ", WordOp::Identity, b""),     // 41
    Transform::new(b"", WordOp::OmitLast4, b""),         // 42
    Transform::new(b"", WordOp::Identity, b". The "),    // 43
    Transform::new(b"", WordOp::FermentAll, b""),        // 44
    Transform::new(b"", WordOp::Identity, b" on "),      // 45
    Transform::new(b"", WordOp::Identity, b" as "),      // 46
    Transform::new(b"", WordOp::Identity, b" is "),      // 47
    Transform::new(b"", WordOp::OmitLast7, b""),         // 48
    Transform::new(b"", WordOp::OmitLast1, b"ing "),     // 49
    Transform::new(b"", WordOp::Identity, b"\n\t"),      // 50
    Transform::new(b"", WordOp::Identity, b":"),         // 51
    Transform::new(b" ", WordOp::Identity, b". "),       // 52
    Transform::new(b"", WordOp::Identity, b"ed "),       // 53
    Transform::new(b"", WordOp::OmitFirst9, b""),        // 54
    Transform::new(b"", WordOp::OmitFirst7, b""),        // 55
    Transform::new(b"", WordOp::OmitLast6, b""),         // 56
    Transform::new(b"", WordOp::Identity, b"("),         // 57
    Transform::new(b"", WordOp::FermentFirst, b", "),    // 58
    Transform::new(b"", WordOp::OmitLast8, b""),         // 59
    Transform::new(b"", WordOp::Identity, b" at "),      // 60
    Transform::new(b"", WordOp::Identity, b"ly "),       // 61
    Transform::new(b" the ", WordOp::Identity, b" of "), // 62
    Transform::new(b"", WordOp::OmitLast5, b""),         // 63
    Transform::new(b"", WordOp::OmitLast9, b""),         // 64
    Transform::new(b" ", WordOp::FermentFirst, b", "),   // 65
    Transform::new(b"", WordOp::FermentFirst, b"\""),    // 66
    Transform::new(b".", WordOp::Identity, b"("),        // 67
    Transform::new(b"", WordOp::FermentAll, b" "),       // 68
    Transform::new(b"", WordOp::FermentFirst, b"\">"),   // 69
    Transform::new(b"", WordOp::Identity, b"=\""),       // 70
    Transform::new(b" ", WordOp::Identity, b"."),        // 71
    Transform::new(b".com/", WordOp::Identity, b""),     // 72
    Transform::new(b" the ", WordOp::Identity, b" of the "), // 73
    Transform::new(b"", WordOp::FermentFirst, b"'"),     // 74
    Transform::new(b"", WordOp::Identity, b". This "),   // 75
    Transform::new(b"", WordOp::Identity, b","),         // 76
    Transform::new(b".", WordOp::Identity, b" "),        // 77
    Transform::new(b"", WordOp::FermentFirst, b"("),     // 78
    Transform::new(b"", WordOp::FermentFirst, b"."),     // 79
    Transform::new(b"", WordOp::Identity, b" not "),     // 80
    Transform::new(b" ", WordOp::Identity, b"=\""),      // 81
    Transform::new(b"", WordOp::Identity, b"er "),       // 82
    Transform::new(b" ", WordOp::FermentAll, b" "),      // 83
    Transform::new(b"", WordOp::Identity, b"al "),       // 84
    Transform::new(b" ", WordOp::FermentAll, b""),       // 85
    Transform::new(b"", WordOp::Identity, b"='"),        // 86
    Transform::new(b"", WordOp::FermentAll, b"\""),      // 87
    Transform::new(b"", WordOp::FermentFirst, b". "),    // 88
    Transform::new(b" ", WordOp::Identity, b"("),        // 89
    Transform::new(b"", WordOp::Identity, b"ful "),      // 90
    Transform::new(b" ", WordOp::FermentFirst, b". "),   // 91
    Transform::new(b"", WordOp::Identity, b"ive "),      // 92
    Transform::new(b"", WordOp::Identity, b"less "),     // 93
    Transform::new(b"", WordOp::FermentAll, b"'"),       // 94
    Transform::new(b"", WordOp::Identity, b"est "),      // 95
    Transform::new(b" ", WordOp::FermentFirst, b"."),    // 96
    Transform::new(b"", WordOp::FermentAll, b"\">"),     // 97
    Transform::new(b" ", WordOp::Identity, b"='"),       // 98
    Transform::new(b"", WordOp::FermentFirst, b","),     // 99
    Transform::new(b"", WordOp::Identity, b"ize "),      // 100
    Transform::new(b"", WordOp::FermentAll, b"."),       // 101
    Transform::new(b"\xc2\xa0", WordOp::Identity, b""),  // 102
    Transform::new(b" ", WordOp::Identity, b","),        // 103
    Transform::new(b"", WordOp::FermentFirst, b"=\""),   // 104
    Transform::new(b"", WordOp::FermentAll, b"=\""),     // 105
    Transform::new(b"", WordOp::Identity, b"ous "),      // 106
    Transform::new(b"", WordOp::FermentAll, b", "),      // 107
    Transform::new(b"", WordOp::FermentFirst, b"='"),    // 108
    Transform::new(b" ", WordOp::FermentFirst, b","),    // 109
    Transform::new(b" ", WordOp::FermentAll, b"=\""),    // 110
    Transform::new(b" ", WordOp::FermentAll, b", "),     // 111
    Transform::new(b"", WordOp::FermentAll, b","),       // 112
    Transform::new(b"", WordOp::FermentAll, b"("),       // 113
    Transform::new(b"", WordOp::FermentAll, b". "),      // 114
    Transform::new(b" ", WordOp::FermentAll, b"."),      // 115
    Transform::new(b"", WordOp::FermentAll, b"='"),      // 116
    Transform::new(b" ", WordOp::FermentAll, b". "),     // 117
    Transform::new(b" ", WordOp::FermentFirst, b"=\""),  // 118
    Transform::new(b" ", WordOp::FermentAll, b"='"),     // 119
    Transform::new(b" ", WordOp::FermentFirst, b"='"),   // 120
];

fn ferment_step(buf: &mut [u8], pos: usize) -> usize {
    if buf[pos] < 192 {
        if (97..=122).contains(&buf[pos]) {
            buf[pos] ^= 32;
        }
        1
    } else if buf[pos] < 224 {
        if pos + 1 < buf.len() {
            buf[pos + 1] ^= 32;
        }
        2
    } else {
        if pos + 2 < buf.len() {
            buf[pos + 2] ^= 5;
        }
        3
    }
}

/// Decodes and transforms a static dictionary word per RFC 7932 Section 8.
///
/// Appends the resulting bytes into `out` and returns the number of emitted
/// bytes, or `None` if `copy_len` is outside `4..=24` or `transform_id >= 121`.
pub fn emit_dictionary_word(
    copy_len: usize,
    word_id: usize,
    max_emit: usize,
    out: &mut Vec<u8>,
) -> Option<usize> {
    if !(4..=24).contains(&copy_len) {
        return None;
    }
    let ndbits = NDBITS[copy_len];
    let nwords = 1usize << ndbits;
    let index = word_id & (nwords - 1);
    let transform_id = word_id >> ndbits;
    if transform_id >= TRANSFORMS.len() {
        return None;
    }

    let start = DOFFSET[copy_len] + index * copy_len;
    let base = &STATIC_DICT[start..start + copy_len];
    let transform = TRANSFORMS[transform_id];

    let mut word = [0u8; 24];
    word[..copy_len].copy_from_slice(base);
    let body = match transform.op {
        WordOp::Identity => &word[..copy_len],
        WordOp::FermentFirst => {
            if copy_len > 0 {
                let _ = ferment_step(&mut word[..copy_len], 0);
            }
            &word[..copy_len]
        }
        WordOp::FermentAll => {
            let mut i = 0;
            while i < copy_len {
                i += ferment_step(&mut word[..copy_len], i);
            }
            &word[..copy_len]
        }
        WordOp::OmitFirst1
        | WordOp::OmitFirst2
        | WordOp::OmitFirst3
        | WordOp::OmitFirst4
        | WordOp::OmitFirst5
        | WordOp::OmitFirst6
        | WordOp::OmitFirst7
        | WordOp::OmitFirst8
        | WordOp::OmitFirst9 => {
            let k = (transform.op as usize) - (WordOp::OmitFirst1 as usize) + 1;
            if copy_len > k {
                &word[k..copy_len]
            } else {
                &[]
            }
        }
        WordOp::OmitLast1
        | WordOp::OmitLast2
        | WordOp::OmitLast3
        | WordOp::OmitLast4
        | WordOp::OmitLast5
        | WordOp::OmitLast6
        | WordOp::OmitLast7
        | WordOp::OmitLast8
        | WordOp::OmitLast9 => {
            let k = (transform.op as usize) - (WordOp::OmitLast1 as usize) + 1;
            if copy_len > k {
                &word[..copy_len - k]
            } else {
                &[]
            }
        }
    };

    let total = transform.prefix.len() + body.len() + transform.suffix.len();
    if total > max_emit {
        return None;
    }
    out.extend_from_slice(transform.prefix);
    out.extend_from_slice(body);
    out.extend_from_slice(transform.suffix);
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32_ieee(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    #[test]
    fn static_dictionary_matches_rfc7932_appendix_a_and_b_checksums() {
        assert_eq!(STATIC_DICT.len(), 122_784);
        assert_eq!(DOFFSET[24] + 24 * (1usize << NDBITS[24]), 122_784);
        assert_eq!(crc32_ieee(STATIC_DICT), 0x5136_cb04);

        let mut encoded = Vec::with_capacity(648);
        for tr in TRANSFORMS {
            encoded.extend_from_slice(tr.prefix);
            encoded.push(0);
            encoded.push(tr.op as u8);
            encoded.extend_from_slice(tr.suffix);
            encoded.push(0);
        }
        assert_eq!(encoded.len(), 648);
        assert_eq!(crc32_ieee(&encoded), 0x3d96_5f81);
    }

    #[test]
    fn emits_transformed_word_from_static_dictionary() {
        let mut out = Vec::new();
        // Word 0 of length 4 is b"time" in Appendix A; transform 4 is FermentFirst + " ".
        let nwords_4 = 1usize << NDBITS[4];
        let word_id = 4 * nwords_4;
        let emitted = emit_dictionary_word(4, word_id, 32, &mut out).unwrap();
        assert_eq!(emitted, 5);
        assert_eq!(&out, b"Time ");
    }
}
