// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

//! AES-128, AES-256, AES-128-CTR, AES-256-GCM, SHA-256, HMAC-SHA256, and HKDF-SHA256.
//!
//! Keys, blocks, counters, and tags are raw bytes. Nothing here frames a
//! message, adds padding, or chooses a nonce. `aes128_encrypt_block` takes a
//! 16-byte key and a 16-byte block and returns the 16-byte ciphertext.
//! `aes256_encrypt_block` does the same with a 32-byte key. There is no
//! inverse-block function. `aes128_ctr` takes a 16-byte key, a 16-byte initial
//! counter, and a byte string, and returns a buffer of the same length. That
//! call both encrypts and decrypts. `aes256_gcm_seal` takes a 32-byte key, a
//! 12-byte nonce, associated data, and plaintext, and returns the ciphertext
//! plus a 16-byte tag. `aes256_gcm_open` returns the plaintext, or
//! `CipherError::TagMismatch` and no plaintext, when the tag does not match.
//!
//! anvil-browser seals Linux `profile.anvil` with `aes256_gcm_seal` and
//! `aes256_gcm_open`. anvil-video encrypts MP4 samples with `aes128_ctr` from
//! `cenc_encrypt_sample` and `cenc_decrypt_sample`. `anvil-net` derives TLS 1.3
//! hybrid handshake secrets with `sha256`, `hkdf_extract`, and `hkdf_expand_label`.
//! This is not a full TLS stack. It has no AES-192, no CBC, and no nonce
//! generator. The GCM nonce is 12 bytes only, and the tag is always 16 bytes.
//! The single-block helpers are not called by those crates directly.
//! There is no input-length cap. The 32-bit GCM counter wraps without an error.
//! SubBytes indexes a 256-byte table, so block encryption is not constant-time.
//! The tag compare in `aes256_gcm_open` does read all 16 bytes before it
//! accepts or rejects.

// FIPS-197 SubBytes table. Index `b` replaces byte `b`. The lookup is data-dependent, not constant-time.
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

// Key-schedule round constants. Index 0 is unused. Later entries are successive powers of x in GF(2^8).
const RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// Tag failure from `aes256_gcm_open`. There is no other error variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CipherError {
    /// Computed tag and supplied tag differ. The open call returns no plaintext.
    TagMismatch,
}

impl std::fmt::Display for CipherError {
    /// Writes `authentication tag does not match`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TagMismatch => write!(f, "authentication tag does not match"),
        }
    }
}

impl std::error::Error for CipherError {}

/// Encrypts one 16-byte block under a 16-byte AES-128 key.
///
/// The return value is the ciphertext block. The schedule is 10 rounds
/// (`Nk` = 4). This does not decrypt, and no sibling crate calls it.
/// anvil-video uses `aes128_ctr` instead. The S-box lookup is not constant-time.
pub fn aes128_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let round_keys = expand_key(key, 4, 10);
    encrypt_block(block, &round_keys, 10)
}

/// Encrypts one 16-byte block under a 32-byte AES-256 key.
///
/// The return value is the ciphertext block. The schedule is 14 rounds
/// (`Nk` = 8). GCM uses this same forward cipher through `encrypt_block`,
/// not through this wrapper. There is no matching decrypt function, and no
/// sibling crate calls this wrapper.
pub fn aes256_encrypt_block(key: &[u8; 32], block: &[u8; 16]) -> [u8; 16] {
    let round_keys = expand_key(key, 8, 14);
    encrypt_block(block, &round_keys, 14)
}

/// Encrypts or decrypts `input` with AES-128-CTR.
///
/// `key` is 16 bytes. `iv` is the initial 16-byte counter, not a 12-byte GCM
/// nonce. After each keystream block, including a short final block, the
/// counter increases by one as a big-endian 128-bit integer. Each output byte
/// is the input byte XORed with the AES-128 encryption of the current counter.
/// Encrypt and decrypt are the same call. The output length equals the input
/// length. Wrap of the 128-bit counter is silent. anvil-video calls this from
/// `cenc_encrypt_sample` and `cenc_decrypt_sample` to encrypt MP4 samples.
/// This path has no tag.
pub fn aes128_ctr(key: &[u8; 16], iv: &[u8; 16], input: &[u8]) -> Vec<u8> {
    let round_keys = expand_key(key, 4, 10);
    let mut counter = *iv;
    let mut out = vec![0u8; input.len()];
    let mut offset = 0;
    while offset < input.len() {
        let stream = encrypt_block(&counter, &round_keys, 10);
        let n = (input.len() - offset).min(16);
        for i in 0..n {
            out[offset + i] = input[offset + i] ^ stream[i];
        }
        offset += n;
        // 128-bit big-endian add. Carry may walk from byte 15 back through byte 0.
        inc_128(&mut counter);
    }
    out
}

/// Seals `plaintext` with AES-256-GCM.
///
/// `key` is 32 bytes. `iv` is 12 bytes. Other nonce lengths are not
/// implemented. `aad` is authenticated and is not written into the ciphertext.
/// The vector is the ciphertext and has the same length as `plaintext`. The
/// array is the 16-byte tag. The first counter block is `iv` followed by the
/// big-endian word 1. Keystream starts one 32-bit step after that. anvil-browser
/// uses this to seal Linux `profile.anvil`. No other crate calls it.
pub fn aes256_gcm_seal(
    key: &[u8; 32],
    iv: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> (Vec<u8>, [u8; 16]) {
    let round_keys = expand_key(key, 8, 14);
    let h = encrypt_block(&[0u8; 16], &round_keys, 14);
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;
    // J0 is the 12-byte nonce plus a 32-bit counter of 1. Keystream starts at J0 + 1.
    let mut counter = j0;
    inc_32(&mut counter);
    let mut body = vec![0u8; plaintext.len()];
    let mut offset = 0;
    while offset < plaintext.len() {
        let stream = encrypt_block(&counter, &round_keys, 14);
        let n = (plaintext.len() - offset).min(16);
        for i in 0..n {
            body[offset + i] = plaintext[offset + i] ^ stream[i];
        }
        offset += n;
        inc_32(&mut counter);
    }
    let tag = finish_tag(&round_keys, &h, &j0, aad, &body);
    (body, tag)
}

/// Opens an AES-256-GCM ciphertext.
///
/// `iv` is 12 bytes, the same layout as `aes256_gcm_seal`. `aad` must be the
/// bytes that were sealed, and `tag` must be the 16-byte tag. The tag is
/// checked before any keystream byte is produced. A mismatch returns
/// `CipherError::TagMismatch` and no plaintext. The check reads all 16 tag
/// bytes and does not stop at the first difference. Block encryption around
/// that check is still not constant-time. anvil-browser uses this to open
/// Linux `profile.anvil`.
pub fn aes256_gcm_open(
    key: &[u8; 32],
    iv: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    tag: &[u8; 16],
) -> Result<Vec<u8>, CipherError> {
    let round_keys = expand_key(key, 8, 14);
    let h = encrypt_block(&[0u8; 16], &round_keys, 14);
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;
    let expect = finish_tag(&round_keys, &h, &j0, aad, ciphertext);
    // Reject before the keystream loop. tags_equal does not return early.
    if !tags_equal(&expect, tag) {
        return Err(CipherError::TagMismatch);
    }
    let mut counter = j0;
    inc_32(&mut counter);
    let mut plain = vec![0u8; ciphertext.len()];
    let mut offset = 0;
    while offset < ciphertext.len() {
        let stream = encrypt_block(&counter, &round_keys, 14);
        let n = (ciphertext.len() - offset).min(16);
        for i in 0..n {
            plain[offset + i] = ciphertext[offset + i] ^ stream[i];
        }
        offset += n;
        inc_32(&mut counter);
    }
    Ok(plain)
}

/// Builds one GCM tag: GHASH(H, AAD, ciphertext) XORed with AES-256(K, J0).
///
/// `h` is the hash subkey, the encryption of a 16-byte zero block under the
/// same round keys. `j0` is the 12-byte nonce followed by a 32-bit counter of
/// 1. `body` is ciphertext on seal and on open. The round count passed to the
/// block cipher is fixed at 14, because only AES-256-GCM calls this. The
/// result is 16 bytes. AAD is covered by GHASH and does not appear in `body`.
fn finish_tag(round_keys: &[u8], h: &[u8; 16], j0: &[u8; 16], aad: &[u8], body: &[u8]) -> [u8; 16] {
    let s = ghash(h, aad, body);
    let ek = encrypt_block(j0, round_keys, 14);
    let mut tag = [0u8; 16];
    for i in 0..16 {
        tag[i] = s[i] ^ ek[i];
    }
    tag
}

/// Compares two 16-byte tags without stopping at the first mismatch.
///
/// Each pair of bytes is XORed and ORed into one accumulator. The function
/// returns true only when that accumulator is zero, so a match and a mismatch
/// both read all 16 bytes. This compare does not short-circuit. It is the only
/// compare in this file written that way. The AES rounds that produce a tag
/// still index `SBOX` and are not constant-time.
fn tags_equal(a: &[u8; 16], b: &[u8; 16]) -> bool {
    // Fixed 16-byte pass. Do not return on the first differing byte.
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// GHASH over associated data, then ciphertext, then the length block.
///
/// `h` is the 16-byte hash subkey. `y` starts at zero. Blocks of `aad` are
/// absorbed first, then blocks of `body`. An empty slice adds no block. The
/// last block is two big-endian `u64` values: the bit length of `aad`, then
/// the bit length of `body`. That block is multiplied by `h` in GF(2^128).
/// The returned 16 bytes are the GHASH output before the tag XOR with E(K, J0).
fn ghash(h: &[u8; 16], aad: &[u8], body: &[u8]) -> [u8; 16] {
    let mut y = [0u8; 16];
    ghash_blocks(&mut y, h, aad);
    ghash_blocks(&mut y, h, body);
    let mut len_block = [0u8; 16];
    // Bit lengths, not byte lengths. Each half is a big-endian u64.
    len_block[..8].copy_from_slice(&(aad.len() as u64 * 8).to_be_bytes());
    len_block[8..].copy_from_slice(&(body.len() as u64 * 8).to_be_bytes());
    xor_block(&mut y, &len_block);
    gmul(&y, h)
}

/// Folds `data` into the GHASH accumulator `y`, 16 bytes at a time.
///
/// Each step XORs one block into `y` and multiplies the result by `h`. A
/// short final block is copied into a zeroed 16-byte buffer, so the unused
/// bytes stay zero. Those pad bytes are not added to the bit length; `ghash`
/// writes the real lengths later. An empty `data` leaves `y` unchanged.
/// Bytes are taken from the start of `data` with no length prefix in the block.
fn ghash_blocks(y: &mut [u8; 16], h: &[u8; 16], data: &[u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let mut block = [0u8; 16];
        let n = (data.len() - offset).min(16);
        // Tail shorter than 16 bytes stays zero-padded on the right.
        block[..n].copy_from_slice(&data[offset..offset + n]);
        xor_block(y, &block);
        *y = gmul(y, h);
        offset += n;
    }
}

/// Multiplies `x` by `y` in GF(2^128) and returns the 16-byte product.
///
/// Both inputs are big-endian in bit order: the high bit of byte 0 is tested
/// first. `v` starts as `y` and shifts right once per bit of `x`. When the
/// bit of `x` is set, `v` is XORed into the product. When the bit that shifts
/// out of `v` is 1, the reduction polynomial x^128 + x^7 + x^2 + x + 1 is
/// folded in as the byte `0xe1` on the high end. The inner test branches on
/// the data bits, so this multiply is not constant-time.
fn gmul(x: &[u8; 16], y: &[u8; 16]) -> [u8; 16] {
    let mut z = [0u8; 16];
    let mut v = *y;
    for byte_index in 0..16 {
        for bit in 0..8 {
            if (x[byte_index] >> (7 - bit)) & 1 == 1 {
                xor_block(&mut z, &v);
            }
            let lsb = v[15] & 1;
            shift_right(&mut v);
            // Bit shifted out is 1: XOR the GHASH reduction constant 0xe1.
            if lsb == 1 {
                v[0] ^= 0xe1;
            }
        }
    }
    z
}

/// Shifts a 16-byte block right by one bit, toward byte 15.
///
/// The low bit of byte `i` becomes the high bit of byte `i + 1`. The high bit
/// of byte 0 becomes zero. The bit that leaves byte 15 is not returned. `gmul`
/// reads `v[15] & 1` before calling this, and that bit decides the reduction.
fn shift_right(v: &mut [u8; 16]) {
    let mut carry = 0u8;
    for byte in v.iter_mut() {
        let next = *byte & 1;
        *byte = (*byte >> 1) | (carry << 7);
        carry = next;
    }
}

/// XORs 16-byte `b` into `a` in place.
///
/// Byte `i` mixes with byte `i`. There is no endian swap. GHASH and the final
/// tag XOR use this. AddRoundKey has its own loop and does not call it.
fn xor_block(a: &mut [u8; 16], b: &[u8; 16]) {
    for i in 0..16 {
        a[i] ^= b[i];
    }
}

/// Adds one to bytes 12..16 of `counter`, as a big-endian `u32`.
///
/// Carry stops at byte 12. Bytes 0 through 11, the GCM nonce, are not modified.
/// Overflow wraps inside those four bytes and does not return an error.
/// GCM keystream blocks use this width. CTR mode uses `inc_128` instead.
fn inc_32(counter: &mut [u8; 16]) {
    // Indices 15, 14, 13, then 12. Byte 11 is never written.
    for i in (12..16).rev() {
        let (sum, overflow) = counter[i].overflowing_add(1);
        counter[i] = sum;
        if !overflow {
            break;
        }
    }
}

/// Adds one to the whole block as a big-endian 128-bit integer.
///
/// Carry may propagate from byte 15 through byte 0. Overflow wraps to all
/// zeros and does not return an error. `aes128_ctr` calls this once per
/// keystream block, after the XOR, including when the last chunk is shorter
/// than 16 bytes. GCM does not call this.
fn inc_128(counter: &mut [u8; 16]) {
    // All 16 bytes, low byte first. Wider than the GCM counter in `inc_32`.
    for i in (0..16).rev() {
        let (sum, overflow) = counter[i].overflowing_add(1);
        counter[i] = sum;
        if !overflow {
            break;
        }
    }
}

/// Expands `key` into concatenated AES round keys.
///
/// `nk` is the key length in 32-bit words: 4 for AES-128, 8 for AES-256.
/// `nr` is the round count: 10 or 14. The buffer is `16 * (nr + 1)` bytes,
/// round 0 first, 16 bytes per round. The input key is copied to the front.
/// Each new 4-byte word is the previous word XORed with the word `nk` words
/// back. When the new word starts a key-sized group, the previous word is
/// RotWord, then SubWord, then XORed with `RCON` on byte 0. For AES-256, a
/// word that lands 16 bytes into the 32-byte group also gets SubWord alone.
/// A bad `nk` or `nr` is not checked. Callers pass the pairs above.
fn expand_key(key: &[u8], nk: usize, nr: usize) -> Vec<u8> {
    let total = 16 * (nr + 1);
    let mut words = vec![0u8; total];
    words[..key.len()].copy_from_slice(key);
    let mut generated = key.len();
    let mut rcon_i = 1usize;
    let mut prev = [0u8; 4];
    prev.copy_from_slice(&words[generated - 4..generated]);
    while generated < total {
        if generated % (nk * 4) == 0 {
            rot_word(&mut prev);
            sub_word(&mut prev);
            prev[0] ^= RCON[rcon_i];
            rcon_i += 1;
        } else if nk > 6 && generated % (nk * 4) == 16 {
            // AES-256 only. Nk is 8, so this is word 4 of the key. AES-128 has Nk 4 and skips this.
            sub_word(&mut prev);
        }
        for j in 0..4 {
            prev[j] ^= words[generated - nk * 4 + j];
        }
        words[generated..generated + 4].copy_from_slice(&prev);
        generated += 4;
    }
    words
}

/// Rotates one 4-byte schedule word left by one byte.
///
/// Byte 0 moves to byte 3. The other bytes move down one index. This is
/// RotWord. It does not use `SBOX`. `expand_key` calls `sub_word` next, then
/// XORs `RCON` into byte 0.
fn rot_word(word: &mut [u8; 4]) {
    let first = word[0];
    word[0] = word[1];
    word[1] = word[2];
    word[2] = word[3];
    word[3] = first;
}

/// Replaces each byte of a 4-byte word with `SBOX[byte]`.
///
/// This is SubWord in the key schedule. The word is updated in place. The
/// table index is the data byte, so the step is not constant-time. The cipher
/// round uses `sub_bytes` on the full state instead of this helper.
fn sub_word(word: &mut [u8; 4]) {
    for b in word.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

/// Runs AES over one 16-byte block and returns the ciphertext state.
///
/// `round_keys` is the buffer from `expand_key`. `nr` is 10 for AES-128 and
/// 14 for AES-256. State bytes stay in block order. Column `c` is the four
/// bytes at `4 * c`. Round 0 is AddRoundKey only. Each middle round is
/// SubBytes, ShiftRows, MixColumns, then AddRoundKey. The last round is
/// SubBytes, ShiftRows, and AddRoundKey, with no MixColumns. No padding is
/// added. The same forward function supplies CTR and GCM keystream.
fn encrypt_block(block: &[u8; 16], round_keys: &[u8], nr: usize) -> [u8; 16] {
    let mut state = *block;
    add_round_key(&mut state, &round_keys[0..16]);
    for round in 1..nr {
        sub_bytes(&mut state);
        shift_rows(&mut state);
        mix_columns(&mut state);
        let start = round * 16;
        add_round_key(&mut state, &round_keys[start..start + 16]);
    }
    // Last round. MixColumns is intentionally not called.
    sub_bytes(&mut state);
    shift_rows(&mut state);
    let start = nr * 16;
    add_round_key(&mut state, &round_keys[start..start + 16]);
    state
}

/// XORs one 16-byte round key into `state`.
///
/// State byte `i` mixes with key byte `i`. There is no rotation and no endian
/// conversion. `key` is the 16-byte slice for this round, taken from the
/// expanded schedule at `round * 16`.
fn add_round_key(state: &mut [u8; 16], key: &[u8]) {
    for i in 0..16 {
        state[i] ^= key[i];
    }
}

/// Replaces every state byte with `SBOX[byte]`.
///
/// This is the SubBytes round step. The state is updated in place. Indexing
/// the 256-byte table by the data is not constant-time. The key schedule uses
/// `sub_word` for the same substitution on four bytes.
fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

/// Rotates the AES rows of `state` to the left.
///
/// Bytes are stored column by column. Row 0, indices 0, 4, 8, and 12, is left
/// as it is. Row 1 (1, 5, 9, 13) moves one byte. Row 2 (2, 6, 10, 14) moves
/// two bytes. Row 3 (3, 7, 11, 15) moves three bytes. No byte is XORed here.
/// A copy of the state is taken first so the rotates do not read their own writes.
fn shift_rows(state: &mut [u8; 16]) {
    let s = *state;
    state[1] = s[5];
    state[5] = s[9];
    state[9] = s[13];
    state[13] = s[1];
    state[2] = s[10];
    state[6] = s[14];
    state[10] = s[2];
    state[14] = s[6];
    state[3] = s[15];
    state[7] = s[3];
    state[11] = s[7];
    state[15] = s[11];
}

/// Mixes each 4-byte column as a polynomial over GF(2^8).
///
/// Column `c` starts at index `4 * c`. The factors are 02 and 03 from the AES
/// MixColumns matrix. Multiply-by-02 is `xtime`. Multiply-by-03 is `xtime(b)
/// XOR b`. The modulus is x^8 + x^4 + x^3 + x + 1. Columns do not mix with
/// each other. `encrypt_block` skips this on the final round.
fn mix_columns(state: &mut [u8; 16]) {
    for col in 0..4 {
        let i = col * 4;
        let a0 = state[i];
        let a1 = state[i + 1];
        let a2 = state[i + 2];
        let a3 = state[i + 3];
        state[i] = xtime(a0) ^ (xtime(a1) ^ a1) ^ a2 ^ a3;
        state[i + 1] = a0 ^ xtime(a1) ^ (xtime(a2) ^ a2) ^ a3;
        state[i + 2] = a0 ^ a1 ^ xtime(a2) ^ (xtime(a3) ^ a3);
        state[i + 3] = (xtime(a0) ^ a0) ^ a1 ^ a2 ^ xtime(a3);
    }
}

/// Multiplies `a` by x in GF(2^8) and reduces when bit 7 was set.
///
/// A clear high bit is a left shift by one. A set high bit XORs `0x1b` after
/// that shift. `0x1b` is the low byte of x^8 + x^4 + x^3 + x + 1. `mix_columns`
/// uses this for the matrix factors 2 and 3. The branch on bit 7 follows the data.
fn xtime(a: u8) -> u8 {
    let shifted = a << 1;
    // Reduce only when the bit that shifted out is 1.
    if a & 0x80 == 0 {
        shifted
    } else {
        shifted ^ 0x1b
    }
}

/// FIPS 180-4 SHA-256 round constants.
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        let j = i * 4;
        w[i] = u32::from_be_bytes([block[j], block[j + 1], block[j + 2], block[j + 3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Computes the 32-byte SHA-256 digest of `data` (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut chunks = data.chunks_exact(64);
    for chunk in &mut chunks {
        let block: &[u8; 64] = chunk.try_into().unwrap();
        sha256_compress(&mut state, block);
    }
    let rem = chunks.remainder();
    let mut tail = [0u8; 64];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    if rem.len() >= 56 {
        sha256_compress(&mut state, &tail);
        tail = [0u8; 64];
    }
    let bit_len = (data.len() as u64).wrapping_mul(8);
    tail[56..64].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut state, &tail);
    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Computes the 32-byte HMAC-SHA256 authentication code of `data` under `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Vec::with_capacity(64 + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let inner_hash = sha256(&inner);
    let mut outer = [0u8; 96];
    outer[..64].copy_from_slice(&opad);
    outer[64..96].copy_from_slice(&inner_hash);
    sha256(&outer)
}

/// Extracts a 32-byte pseudorandom key from `salt` and input key material `ikm`
/// using HKDF-Extract with SHA-256 (RFC 5869 Section 2.2).
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    if salt.is_empty() {
        hmac_sha256(&[0u8; 32], ikm)
    } else {
        hmac_sha256(salt, ikm)
    }
}

/// Expands a 32-byte pseudorandom key `prk` with context `info` to `len` bytes
/// (capped at `255 * 32`) using HKDF-Expand with SHA-256 (RFC 5869 Section 2.3).
pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], len: usize) -> Vec<u8> {
    let target = len.min(255 * 32);
    let mut out = Vec::with_capacity(target);
    let mut prev = [0u8; 32];
    let mut prev_len = 0usize;
    let blocks = target.div_ceil(32);
    for i in 1..=blocks {
        let mut msg = Vec::with_capacity(prev_len + info.len() + 1);
        msg.extend_from_slice(&prev[..prev_len]);
        msg.extend_from_slice(info);
        msg.push(i as u8);
        prev = hmac_sha256(prk, &msg);
        prev_len = 32;
        let take = (target - out.len()).min(32);
        out.extend_from_slice(&prev[..take]);
    }
    out
}

/// TLS 1.3 `HKDF-Expand-Label` with SHA-256 (RFC 8446 Section 7.1).
pub fn hkdf_expand_label(secret: &[u8; 32], label: &[u8], context: &[u8], len: u16) -> Vec<u8> {
    let full_label_len = (6 + label.len()).min(255);
    let ctx_len = context.len().min(255);
    let mut info = Vec::with_capacity(2 + 1 + full_label_len + 1 + ctx_len);
    info.extend_from_slice(&len.to_be_bytes());
    info.push(full_label_len as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(&label[..full_label_len - 6]);
    info.push(ctx_len as u8);
    info.extend_from_slice(&context[..ctx_len]);
    hkdf_expand(secret, &info, usize::from(len))
}

#[cfg(test)]
mod _noop_mod {
    #[allow(dead_code)]
    fn _noop() {}
}

#[cfg(test)]
mod tests {
    // Locks the FIPS-197 AES-128 and AES-256 block vectors, the NIST CTR
    // sample and decrypt-by-the-same-call, GCM round trip with a rejected
    // tag, and the all-zero empty AES-256-GCM tag.
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn aes128_block_matches_fips_197() {
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let block: [u8; 16] = hex("00112233445566778899aabbccddeeff").try_into().unwrap();
        let out = aes128_encrypt_block(&key, &block);
        assert_eq!(hex::encode_local(&out), "69c4e0d86a7b0430d8cdb78070b4c55a");
    }

    #[test]
    fn aes256_block_matches_fips_197() {
        let key: [u8; 32] = hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
            .try_into()
            .unwrap();
        let block: [u8; 16] = hex("00112233445566778899aabbccddeeff").try_into().unwrap();
        let out = aes256_encrypt_block(&key, &block);
        assert_eq!(hex::encode_local(&out), "8ea2b7ca516745bfeafc49904b496089");
    }

    #[test]
    fn aes128_ctr_matches_nist_sp800_38a() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff").try_into().unwrap();
        let plain = hex("6bc1bee22e409f96e93d7e117393172a");
        let ct = aes128_ctr(&key, &iv, &plain);
        assert_eq!(hex::encode_local(&ct), "874d6191b620e3261bef6864990db6ce");
        assert_eq!(aes128_ctr(&key, &iv, &ct), plain);
    }

    #[test]
    fn aes256_gcm_round_trip_and_rejects_bad_tag() {
        let key = [0x11u8; 32];
        let iv = [0x22u8; 12];
        let aad = b"profile";
        let plain = b"bookmarks and cookies";
        let (body, tag) = aes256_gcm_seal(&key, &iv, aad, plain);
        let opened = aes256_gcm_open(&key, &iv, aad, &body, &tag).unwrap();
        assert_eq!(opened, plain);

        let mut bad = tag;
        bad[0] ^= 1;
        assert_eq!(
            aes256_gcm_open(&key, &iv, aad, &body, &bad),
            Err(CipherError::TagMismatch)
        );

        let empty = aes256_gcm_seal(&key, &iv, b"", b"");
        assert!(empty.0.is_empty());
        assert!(
            aes256_gcm_open(&key, &iv, b"", b"", &empty.1)
                .unwrap()
                .is_empty()
        );
        let mut bad_empty = empty.1;
        bad_empty[15] ^= 0xff;
        assert_eq!(
            aes256_gcm_open(&key, &iv, b"", b"", &bad_empty),
            Err(CipherError::TagMismatch)
        );
    }

    #[test]
    fn aes256_gcm_empty_matches_nist() {
        let (body, tag) = aes256_gcm_seal(&[0u8; 32], &[0u8; 12], b"", b"");
        assert!(body.is_empty());
        assert_eq!(hex::encode_local(&tag), "530f8afbc74536b9a963b4f1c4cb738b");
    }

    #[test]
    fn sha256_and_hkdf_match_nist_and_rfc_5869_vectors() {
        assert_eq!(
            hex::encode_local(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // RFC 5869 Test Case 1
        let ikm = [0x0bu8; 22];
        let salt = hex("000102030405060708090a0b0c");
        let info = hex("f0f1f2f3f4f5f6f7f8f9");
        let prk = hkdf_extract(&salt, &ikm);
        assert_eq!(
            hex::encode_local(&prk),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let okm = hkdf_expand(&prk, &info, 42);
        assert_eq!(
            hex::encode_local(&okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
        let label_out = hkdf_expand_label(&prk, b"derived", &sha256(b""), 32);
        assert_eq!(label_out.len(), 32);
    }

    mod hex {
        pub fn encode_local(bytes: &[u8]) -> String {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut out = String::with_capacity(bytes.len() * 2);
            for &b in bytes {
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
            out
        }
    }
}
