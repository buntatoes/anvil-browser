// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! TLS 1.3 `X25519MLKEM768` key shares and key schedule built on `anvil-pqc` and `anvil-cipher`.
//!
//! `anvil-pqc` implements ML-KEM-768 (FIPS 203), X25519 (RFC 7748), the
//! `X25519MLKEM768` hybrid group (`draft-ietf-tls-ecdhe-mlkem-03`), and SHA-3
//! (FIPS 202). `anvil-cipher` implements SHA-256, HMAC-SHA256, HKDF-SHA256
//! (RFC 5869), TLS 1.3 `HKDF-Expand-Label` (RFC 8446 Section 7.1), and
//! AES-256-GCM.
//!
//! This module ties those two crates together so the browser stack can:
//! - Build and parse TLS 1.3 `key_share` extensions offering `X25519MLKEM768`
//!   (`0x11EC`) alongside classical `X25519` (`0x001D`).
//! - Complete the 64-byte `X25519MLKEM768` hybrid key exchange or the 32-byte
//!   classical `X25519` key exchange.
//! - Run the RFC 8446 Section 7.1 handshake key schedule to derive
//!   `handshake_secret`, `client_handshake_traffic_secret`,
//!   `server_handshake_traffic_secret`, Finished `verify_data`, and AES-256-GCM
//!   record keys and per-record nonces.
//! - Run a deterministic self-test (`verify_pqc_hybrid_self_test`) before TLS
//!   sessions open.

use std::io::Error;
use std::sync::OnceLock;

use anvil_cipher::{
    aes256_gcm_open, aes256_gcm_seal, hkdf_expand_label, hkdf_extract, hmac_sha256, sha256,
};
use anvil_pqc::hybrid::{self, CLIENT_SHARE_LEN, ClientState, SERVER_SHARE_LEN, SHARED_SECRET_LEN};
use anvil_pqc::keccak::sha3_256;
use anvil_pqc::{fill_random, mlkem, x25519};

use crate::NetError;

/// TLS 1.3 `NamedGroup` codepoint for `X25519MLKEM768` (`0x11EC`).
pub const CODEPOINT_X25519_MLKEM768: u16 = hybrid::CODEPOINT;

/// TLS 1.3 `NamedGroup` codepoint for `X25519` (`0x001D`).
pub const CODEPOINT_X25519: u16 = 0x001d;

/// Client-side state and public key shares for a TLS 1.3 `ClientHello` that
/// offers `X25519MLKEM768` first and classical `X25519` second.
pub struct ClientKeyShares {
    hybrid_state: ClientState,
    hybrid_share: Vec<u8>,
    classical_x25519_secret: [u8; 32],
    classical_x25519_share: [u8; 32],
}

impl Drop for ClientKeyShares {
    fn drop(&mut self) {
        self.classical_x25519_secret = [0u8; 32];
        std::hint::black_box(&self.classical_x25519_secret);
    }
}

impl ClientKeyShares {
    /// Generate fresh `X25519MLKEM768` and classical `X25519` key shares from
    /// the operating system CSPRNG (`anvil_pqc::fill_random`).
    pub fn generate() -> Result<Self, NetError> {
        let (hybrid_state, hybrid_share) = hybrid::client_share()
            .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc keygen failed: {e}"))))?;
        let mut classical_x25519_secret = [0u8; 32];
        fill_random(&mut classical_x25519_secret)
            .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc rng failed: {e}"))))?;
        let classical_x25519_share = x25519::public_key(&classical_x25519_secret);
        Ok(Self {
            hybrid_state,
            hybrid_share,
            classical_x25519_secret,
            classical_x25519_share,
        })
    }

    /// Build `X25519MLKEM768` and classical `X25519` key shares deterministically
    /// from caller-supplied seeds.
    pub fn from_seeds(
        mlkem_seed: &[u8; mlkem::SEED_LEN],
        hybrid_x25519_secret: &[u8; 32],
        classical_x25519_secret: &[u8; 32],
    ) -> Self {
        let (hybrid_state, hybrid_share) =
            hybrid::client_share_from_seed(mlkem_seed, hybrid_x25519_secret);
        let classical_x25519_share = x25519::public_key(classical_x25519_secret);
        Self {
            hybrid_state,
            hybrid_share,
            classical_x25519_secret: *classical_x25519_secret,
            classical_x25519_share,
        }
    }

    /// The 1,216-byte `X25519MLKEM768` client share (`mlkem768_ek || x25519_pub`).
    pub fn hybrid_share(&self) -> &[u8] {
        &self.hybrid_share
    }

    /// The 32-byte classical `X25519` client share.
    pub fn classical_x25519_share(&self) -> &[u8; 32] {
        &self.classical_x25519_share
    }

    /// Encode the RFC 8446 Section 4.2.8 `KeyShareClientHello` extension body
    /// offering `X25519MLKEM768` (`0x11EC`, 1,216 bytes) followed by `X25519`
    /// (`0x001D`, 32 bytes).
    pub fn encode_tls13_key_share_extension(&self) -> Vec<u8> {
        let entries_len = (2 + 2 + self.hybrid_share.len()) + (2 + 2 + 32);
        let mut out = Vec::with_capacity(2 + entries_len);
        out.extend_from_slice(&(entries_len as u16).to_be_bytes());
        out.extend_from_slice(&CODEPOINT_X25519_MLKEM768.to_be_bytes());
        out.extend_from_slice(&(self.hybrid_share.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.hybrid_share);
        out.extend_from_slice(&CODEPOINT_X25519.to_be_bytes());
        out.extend_from_slice(&32u16.to_be_bytes());
        out.extend_from_slice(&self.classical_x25519_share);
        out
    }

    /// Finish the `X25519MLKEM768` exchange against the 1,120-byte server share
    /// and return the 64-byte combined shared secret.
    pub fn finish_hybrid(&self, server_share: &[u8]) -> Result<[u8; SHARED_SECRET_LEN], NetError> {
        self.hybrid_state
            .finish(server_share)
            .map_err(|e| NetError::Io(Error::other(format!("hybrid key exchange failed: {e}"))))
    }

    /// Finish a classical `X25519` exchange when the server selects `0x001D`.
    pub fn finish_classical_x25519(&self, server_share: &[u8; 32]) -> Result<[u8; 32], NetError> {
        x25519::shared_secret(&self.classical_x25519_secret, server_share)
            .map_err(|e| NetError::Io(Error::other(format!("X25519 key exchange failed: {e}"))))
    }
}

/// Parse a TLS 1.3 `KeyShareClientHello` body built by
/// [`ClientKeyShares::encode_tls13_key_share_extension`] into `(group, share)` pairs.
pub fn parse_tls13_key_share_extension(buf: &[u8]) -> Result<Vec<(u16, Vec<u8>)>, NetError> {
    if buf.len() < 2 {
        return Err(NetError::InvalidResponse);
    }
    let list_len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if buf.len() != 2 + list_len {
        return Err(NetError::InvalidResponse);
    }
    let mut pos = 2usize;
    let mut entries = Vec::new();
    while pos < buf.len() {
        if pos + 4 > buf.len() {
            return Err(NetError::InvalidResponse);
        }
        let group = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let kex_len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
        pos += 4;
        if pos + kex_len > buf.len() {
            return Err(NetError::InvalidResponse);
        }
        entries.push((group, buf[pos..pos + kex_len].to_vec()));
        pos += kex_len;
    }
    Ok(entries)
}

/// Respond to a 1,216-byte `X25519MLKEM768` client share using system randomness.
pub fn server_accept_hybrid(
    client_share: &[u8],
) -> Result<(Vec<u8>, [u8; SHARED_SECRET_LEN]), NetError> {
    hybrid::server_respond(client_share)
        .map_err(|e| NetError::Io(Error::other(format!("hybrid server respond failed: {e}"))))
}

/// Respond to a 1,216-byte `X25519MLKEM768` client share using caller-supplied
/// randomness (`m` for ML-KEM-768 encapsulation and `x25519_secret` for X25519).
pub fn server_accept_hybrid_with(
    client_share: &[u8],
    m: &[u8; 32],
    x25519_secret: &[u8; 32],
) -> Result<(Vec<u8>, [u8; SHARED_SECRET_LEN]), NetError> {
    hybrid::server_respond_with(client_share, m, x25519_secret)
        .map_err(|e| NetError::Io(Error::other(format!("hybrid server respond failed: {e}"))))
}

/// TLS 1.3 handshake secrets, AEAD traffic keys, and transcript digests derived
/// from a shared secret (`X25519MLKEM768` or `X25519`) and the
/// `ClientHello || ServerHello` transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tls13HandshakeKeys {
    /// SHA-256 digest of the `ClientHello || ServerHello` transcript.
    pub transcript_sha256: [u8; 32],
    /// SHA3-256 digest of the `ClientHello || ServerHello` transcript (`anvil-pqc`).
    pub transcript_sha3_256: [u8; 32],
    /// RFC 8446 Section 7.1 `Handshake Secret`.
    pub handshake_secret: [u8; 32],
    /// RFC 8446 Section 7.1 `client_handshake_traffic_secret`.
    pub client_handshake_traffic_secret: [u8; 32],
    /// RFC 8446 Section 7.1 `server_handshake_traffic_secret`.
    pub server_handshake_traffic_secret: [u8; 32],
    /// 32-byte AES-256-GCM key for client handshake records.
    pub client_write_key: [u8; 32],
    /// 32-byte AES-256-GCM key for server handshake records.
    pub server_write_key: [u8; 32],
    /// 12-byte base IV for client handshake records.
    pub client_write_iv: [u8; 12],
    /// 12-byte base IV for server handshake records.
    pub server_write_iv: [u8; 12],
}

impl Tls13HandshakeKeys {
    /// Run the RFC 8446 Section 7.1 handshake key schedule over `shared_secret`
    /// and `transcript_bytes` (`ClientHello || ServerHello`).
    pub fn derive(shared_secret: &[u8], transcript_bytes: &[u8]) -> Result<Self, NetError> {
        let transcript_sha256 = sha256(transcript_bytes);
        let transcript_sha3_256 = sha3_256(&[transcript_bytes]);
        let zero_ikm = [0u8; 32];
        let early_secret = hkdf_extract(&zero_ikm, &zero_ikm);
        let empty_hash = sha256(b"");
        let derived: [u8; 32] = hkdf_expand_label(&early_secret, b"derived", &empty_hash, 32)
            .try_into()
            .unwrap();
        let handshake_secret = hkdf_extract(&derived, shared_secret);
        let client_handshake_traffic_secret: [u8; 32] =
            hkdf_expand_label(&handshake_secret, b"c hs traffic", &transcript_sha256, 32)
                .try_into()
                .unwrap();
        let server_handshake_traffic_secret: [u8; 32] =
            hkdf_expand_label(&handshake_secret, b"s hs traffic", &transcript_sha256, 32)
                .try_into()
                .unwrap();
        let client_write_key: [u8; 32] =
            hkdf_expand_label(&client_handshake_traffic_secret, b"key", b"", 32)
                .try_into()
                .unwrap();
        let server_write_key: [u8; 32] =
            hkdf_expand_label(&server_handshake_traffic_secret, b"key", b"", 32)
                .try_into()
                .unwrap();
        let client_write_iv: [u8; 12] =
            hkdf_expand_label(&client_handshake_traffic_secret, b"iv", b"", 12)
                .try_into()
                .unwrap();
        let server_write_iv: [u8; 12] =
            hkdf_expand_label(&server_handshake_traffic_secret, b"iv", b"", 12)
                .try_into()
                .unwrap();

        Ok(Self {
            transcript_sha256,
            transcript_sha3_256,
            handshake_secret,
            client_handshake_traffic_secret,
            server_handshake_traffic_secret,
            client_write_key,
            server_write_key,
            client_write_iv,
            server_write_iv,
        })
    }

    /// Compute the RFC 8446 Section 4.4.4 `Finished` `verify_data` MAC for the
    /// client handshake traffic secret.
    pub fn client_finished_verify_data(
        &self,
        transcript_hash: &[u8; 32],
    ) -> Result<[u8; 32], NetError> {
        let finished_key =
            hkdf_expand_label(&self.client_handshake_traffic_secret, b"finished", b"", 32);
        Ok(hmac_sha256(&finished_key, transcript_hash))
    }

    /// Compute the RFC 8446 Section 4.4.4 `Finished` `verify_data` MAC for the
    /// server handshake traffic secret.
    pub fn server_finished_verify_data(
        &self,
        transcript_hash: &[u8; 32],
    ) -> Result<[u8; 32], NetError> {
        let finished_key =
            hkdf_expand_label(&self.server_handshake_traffic_secret, b"finished", b"", 32);
        Ok(hmac_sha256(&finished_key, transcript_hash))
    }

    /// Encrypt a client handshake record with AES-256-GCM using the RFC 8446
    /// Section 5.3 per-record nonce (`client_write_iv XOR seq`).
    pub fn seal_client_record(
        &self,
        seq: u64,
        aad: &[u8],
        plaintext: &[u8],
    ) -> (Vec<u8>, [u8; 16]) {
        let nonce = record_nonce(&self.client_write_iv, seq);
        aes256_gcm_seal(&self.client_write_key, &nonce, aad, plaintext)
    }

    /// Decrypt and authenticate a client handshake record with AES-256-GCM.
    pub fn open_client_record(
        &self,
        seq: u64,
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, NetError> {
        let nonce = record_nonce(&self.client_write_iv, seq);
        aes256_gcm_open(&self.client_write_key, &nonce, aad, ciphertext, tag)
            .map_err(|e| NetError::Io(Error::other(format!("TLS record auth failed: {e}"))))
    }

    /// Encrypt a server handshake record with AES-256-GCM using `server_write_iv XOR seq`.
    pub fn seal_server_record(
        &self,
        seq: u64,
        aad: &[u8],
        plaintext: &[u8],
    ) -> (Vec<u8>, [u8; 16]) {
        let nonce = record_nonce(&self.server_write_iv, seq);
        aes256_gcm_seal(&self.server_write_key, &nonce, aad, plaintext)
    }

    /// Decrypt and authenticate a server handshake record with AES-256-GCM.
    pub fn open_server_record(
        &self,
        seq: u64,
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>, NetError> {
        let nonce = record_nonce(&self.server_write_iv, seq);
        aes256_gcm_open(&self.server_write_key, &nonce, aad, ciphertext, tag)
            .map_err(|e| NetError::Io(Error::other(format!("TLS record auth failed: {e}"))))
    }

    /// Derive the RFC 8446 Section 7.1 `master_secret` and application traffic keys
    /// over `ClientHello .. ServerFinished` (`full_handshake_transcript`).
    pub fn derive_application_keys(
        &self,
        full_handshake_transcript: &[u8],
    ) -> Result<Tls13ApplicationKeys, NetError> {
        let th = sha256(full_handshake_transcript);
        let empty_hash = sha256(b"");
        let derived_master: [u8; 32] =
            hkdf_expand_label(&self.handshake_secret, b"derived", &empty_hash, 32)
                .try_into()
                .unwrap();
        let zero_ikm = [0u8; 32];
        let master_secret = hkdf_extract(&derived_master, &zero_ikm);
        let client_app_traffic_secret: [u8; 32] =
            hkdf_expand_label(&master_secret, b"c ap traffic", &th, 32)
                .try_into()
                .unwrap();
        let server_app_traffic_secret: [u8; 32] =
            hkdf_expand_label(&master_secret, b"s ap traffic", &th, 32)
                .try_into()
                .unwrap();
        let client_write_key: [u8; 32] =
            hkdf_expand_label(&client_app_traffic_secret, b"key", b"", 32)
                .try_into()
                .unwrap();
        let server_write_key: [u8; 32] =
            hkdf_expand_label(&server_app_traffic_secret, b"key", b"", 32)
                .try_into()
                .unwrap();
        let client_write_iv: [u8; 12] =
            hkdf_expand_label(&client_app_traffic_secret, b"iv", b"", 12)
                .try_into()
                .unwrap();
        let server_write_iv: [u8; 12] =
            hkdf_expand_label(&server_app_traffic_secret, b"iv", b"", 12)
                .try_into()
                .unwrap();
        Ok(Tls13ApplicationKeys {
            master_secret,
            client_app_traffic_secret,
            server_app_traffic_secret,
            client_write_key,
            server_write_key,
            client_write_iv,
            server_write_iv,
        })
    }
}

/// TLS 1.3 application-data traffic secrets and AES-256-GCM record keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tls13ApplicationKeys {
    pub master_secret: [u8; 32],
    pub client_app_traffic_secret: [u8; 32],
    pub server_app_traffic_secret: [u8; 32],
    pub client_write_key: [u8; 32],
    pub server_write_key: [u8; 32],
    pub client_write_iv: [u8; 12],
    pub server_write_iv: [u8; 12],
}

/// TLS record content types (RFC 8446 Section 5.1).
pub const RECORD_CHANGE_CIPHER_SPEC: u8 = 20;
pub const RECORD_ALERT: u8 = 21;
pub const RECORD_HANDSHAKE: u8 = 22;
pub const RECORD_APPLICATION_DATA: u8 = 23;

/// TLS 1.3 handshake message types (RFC 8446 Section 4).
pub const HANDSHAKE_CLIENT_HELLO: u8 = 1;
pub const HANDSHAKE_SERVER_HELLO: u8 = 2;
pub const HANDSHAKE_ENCRYPTED_EXTENSIONS: u8 = 8;
pub const HANDSHAKE_FINISHED: u8 = 20;

/// TLS 1.3 cipher suite codepoints (RFC 8446 Appendix B.4).
pub const CIPHER_TLS_AES_256_GCM_SHA384: u16 = 0x1302;
pub const CIPHER_TLS_AES_128_GCM_SHA256: u16 = 0x1301;

const EXT_SERVER_NAME: u16 = 0x0000;
const EXT_SUPPORTED_GROUPS: u16 = 0x000a;
const EXT_SIGNATURE_ALGORITHMS: u16 = 0x000d;
const EXT_ALPN: u16 = 0x0010;
const EXT_SUPPORTED_VERSIONS: u16 = 0x002b;
const EXT_KEY_SHARE: u16 = 0x0033;

/// Frame a plaintext TLS record (`[content_type, 0x03, 0x03, len_hi, len_lo, payload]`).
pub fn encode_plaintext_record(content_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(content_type);
    out.push(0x03);
    out.push(0x03);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Read one outer TLS record from `reader`.
pub fn read_tls_record<R: std::io::Read>(reader: &mut R) -> Result<([u8; 5], Vec<u8>), NetError> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header)?;
    let len = u16::from_be_bytes([header[3], header[4]]) as usize;
    if len > 18432 {
        return Err(NetError::InvalidResponse);
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok((header, body))
}

/// Seal an RFC 8446 Section 5.2 `TLSCiphertext` record with AES-256-GCM.
pub fn seal_tls13_record(
    key: &[u8; 32],
    base_iv: &[u8; 12],
    seq: u64,
    inner_content_type: u8,
    content: &[u8],
) -> Vec<u8> {
    let mut inner = Vec::with_capacity(content.len() + 1);
    inner.extend_from_slice(content);
    inner.push(inner_content_type);
    let cipher_len = (inner.len() + 16) as u16;
    let header = [
        RECORD_APPLICATION_DATA,
        0x03,
        0x03,
        (cipher_len >> 8) as u8,
        cipher_len as u8,
    ];
    let nonce = record_nonce(base_iv, seq);
    let (ct, tag) = aes256_gcm_seal(key, &nonce, &header, &inner);
    let mut out = Vec::with_capacity(5 + ct.len() + 16);
    out.extend_from_slice(&header);
    out.extend_from_slice(&ct);
    out.extend_from_slice(&tag);
    out
}

/// Open and authenticate an RFC 8446 Section 5.2 `TLSCiphertext` record.
pub fn open_tls13_record(
    key: &[u8; 32],
    base_iv: &[u8; 12],
    seq: u64,
    header: &[u8; 5],
    encrypted_body: &[u8],
) -> Result<(u8, Vec<u8>), NetError> {
    if header[0] != RECORD_APPLICATION_DATA || encrypted_body.len() < 17 {
        return Err(NetError::InvalidResponse);
    }
    let ct_len = encrypted_body.len() - 16;
    let ct = &encrypted_body[..ct_len];
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&encrypted_body[ct_len..]);
    let nonce = record_nonce(base_iv, seq);
    let mut inner = aes256_gcm_open(key, &nonce, header, ct, &tag)
        .map_err(|e| NetError::Io(Error::other(format!("TLS 1.3 record auth failed: {e}"))))?;
    while inner.last() == Some(&0) {
        inner.pop();
    }
    let Some(content_type) = inner.pop() else {
        return Err(NetError::InvalidResponse);
    };
    Ok((content_type, inner))
}

fn wrap_handshake_msg(msg_type: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len();
    let mut out = Vec::with_capacity(4 + len);
    out.push(msg_type);
    out.push(((len >> 16) & 0xff) as u8);
    out.push(((len >> 8) & 0xff) as u8);
    out.push((len & 0xff) as u8);
    out.extend_from_slice(body);
    out
}

fn unwrap_handshake_msg(msg: &[u8], expected_type: u8) -> Result<&[u8], NetError> {
    if msg.len() < 4 || msg[0] != expected_type {
        return Err(NetError::InvalidResponse);
    }
    let len = ((msg[1] as usize) << 16) | ((msg[2] as usize) << 8) | (msg[3] as usize);
    if msg.len() != 4 + len {
        return Err(NetError::InvalidResponse);
    }
    Ok(&msg[4..])
}

/// Build an RFC 8446 Section 4.1.2 `ClientHello` handshake message offering
/// `X25519MLKEM768` (`0x11EC`) and `X25519` (`0x001D`), SNI, and ALPN.
pub fn encode_client_hello(
    sni_host: &str,
    alpn_protocols: &[&str],
    client_random: &[u8; 32],
    session_id: &[u8],
    shares: &ClientKeyShares,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(1400);
    body.extend_from_slice(&0x0303u16.to_be_bytes());
    body.extend_from_slice(client_random);
    let sid_len = session_id.len().min(32);
    body.push(sid_len as u8);
    body.extend_from_slice(&session_id[..sid_len]);
    // CipherSuites: TLS_AES_256_GCM_SHA384 (0x1302), TLS_AES_128_GCM_SHA256 (0x1301)
    body.extend_from_slice(&4u16.to_be_bytes());
    body.extend_from_slice(&CIPHER_TLS_AES_256_GCM_SHA384.to_be_bytes());
    body.extend_from_slice(&CIPHER_TLS_AES_128_GCM_SHA256.to_be_bytes());
    // legacy_compression_methods: [0x00]
    body.push(1);
    body.push(0);

    let mut exts = Vec::with_capacity(1320);
    // server_name (0x0000)
    if !sni_host.is_empty() {
        let host_bytes = sni_host.as_bytes();
        let entry_len = 1 + 2 + host_bytes.len();
        let ext_len = 2 + entry_len;
        exts.extend_from_slice(&EXT_SERVER_NAME.to_be_bytes());
        exts.extend_from_slice(&(ext_len as u16).to_be_bytes());
        exts.extend_from_slice(&(entry_len as u16).to_be_bytes());
        exts.push(0);
        exts.extend_from_slice(&(host_bytes.len() as u16).to_be_bytes());
        exts.extend_from_slice(host_bytes);
    }
    // supported_versions (0x002b) -> [0x0304]
    exts.extend_from_slice(&EXT_SUPPORTED_VERSIONS.to_be_bytes());
    exts.extend_from_slice(&3u16.to_be_bytes());
    exts.push(2);
    exts.extend_from_slice(&0x0304u16.to_be_bytes());
    // supported_groups (0x000a) -> [0x11EC, 0x001D]
    exts.extend_from_slice(&EXT_SUPPORTED_GROUPS.to_be_bytes());
    exts.extend_from_slice(&6u16.to_be_bytes());
    exts.extend_from_slice(&4u16.to_be_bytes());
    exts.extend_from_slice(&CODEPOINT_X25519_MLKEM768.to_be_bytes());
    exts.extend_from_slice(&CODEPOINT_X25519.to_be_bytes());
    // signature_algorithms (0x000d)
    exts.extend_from_slice(&EXT_SIGNATURE_ALGORITHMS.to_be_bytes());
    exts.extend_from_slice(&6u16.to_be_bytes());
    exts.extend_from_slice(&4u16.to_be_bytes());
    exts.extend_from_slice(&0x0403u16.to_be_bytes());
    exts.extend_from_slice(&0x0804u16.to_be_bytes());
    // ALPN (0x0010)
    if !alpn_protocols.is_empty() {
        let mut alpn_list = Vec::new();
        for proto in alpn_protocols {
            let pb = proto.as_bytes();
            if !pb.is_empty() && pb.len() <= 255 {
                alpn_list.push(pb.len() as u8);
                alpn_list.extend_from_slice(pb);
            }
        }
        let ext_len = 2 + alpn_list.len();
        exts.extend_from_slice(&EXT_ALPN.to_be_bytes());
        exts.extend_from_slice(&(ext_len as u16).to_be_bytes());
        exts.extend_from_slice(&(alpn_list.len() as u16).to_be_bytes());
        exts.extend_from_slice(&alpn_list);
    }
    // key_share (0x0033)
    let ks_body = shares.encode_tls13_key_share_extension();
    exts.extend_from_slice(&EXT_KEY_SHARE.to_be_bytes());
    exts.extend_from_slice(&(ks_body.len() as u16).to_be_bytes());
    exts.extend_from_slice(&ks_body);

    body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
    body.extend_from_slice(&exts);
    wrap_handshake_msg(HANDSHAKE_CLIENT_HELLO, &body)
}

/// Parsed fields of a TLS 1.3 `ClientHello` handshake message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedClientHello {
    pub client_random: [u8; 32],
    pub session_id: Vec<u8>,
    pub sni_host: Option<String>,
    pub alpn_protocols: Vec<String>,
    pub key_shares: Vec<(u16, Vec<u8>)>,
}

/// Parse an RFC 8446 Section 4.1.2 `ClientHello` handshake message.
pub fn parse_client_hello(handshake_msg: &[u8]) -> Result<ParsedClientHello, NetError> {
    let body = unwrap_handshake_msg(handshake_msg, HANDSHAKE_CLIENT_HELLO)?;
    if body.len() < 38 {
        return Err(NetError::InvalidResponse);
    }
    let mut client_random = [0u8; 32];
    client_random.copy_from_slice(&body[2..34]);
    let mut pos = 34usize;
    let sid_len = body[pos] as usize;
    pos += 1;
    if pos + sid_len + 2 > body.len() {
        return Err(NetError::InvalidResponse);
    }
    let session_id = body[pos..pos + sid_len].to_vec();
    pos += sid_len;
    let cs_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2 + cs_len;
    if pos >= body.len() {
        return Err(NetError::InvalidResponse);
    }
    let comp_len = body[pos] as usize;
    pos += 1 + comp_len;
    if pos + 2 > body.len() {
        return Err(NetError::InvalidResponse);
    }
    let exts_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2;
    if pos + exts_len != body.len() {
        return Err(NetError::InvalidResponse);
    }
    let mut sni_host = None;
    let mut alpn_protocols = Vec::new();
    let mut key_shares = Vec::new();
    while pos < body.len() {
        if pos + 4 > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_type = u16::from_be_bytes([body[pos], body[pos + 1]]);
        let ext_len = u16::from_be_bytes([body[pos + 2], body[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_data = &body[pos..pos + ext_len];
        pos += ext_len;
        match ext_type {
            EXT_SERVER_NAME => {
                if ext_data.len() >= 5 && ext_data[2] == 0 {
                    let nlen = u16::from_be_bytes([ext_data[3], ext_data[4]]) as usize;
                    if 5 + nlen <= ext_data.len() {
                        sni_host = String::from_utf8(ext_data[5..5 + nlen].to_vec()).ok();
                    }
                }
            }
            EXT_ALPN => {
                if ext_data.len() >= 2 {
                    let mut apos = 2usize;
                    while apos < ext_data.len() {
                        let plen = ext_data[apos] as usize;
                        apos += 1;
                        if apos + plen <= ext_data.len()
                            && let Ok(s) = std::str::from_utf8(&ext_data[apos..apos + plen])
                        {
                            alpn_protocols.push(s.to_string());
                        }
                        apos += plen;
                    }
                }
            }
            EXT_KEY_SHARE => {
                key_shares = parse_tls13_key_share_extension(ext_data)?;
            }
            _ => {}
        }
    }
    Ok(ParsedClientHello {
        client_random,
        session_id,
        sni_host,
        alpn_protocols,
        key_shares,
    })
}

/// Build an RFC 8446 Section 4.1.3 `ServerHello` handshake message.
pub fn encode_server_hello(
    server_random: &[u8; 32],
    session_id: &[u8],
    cipher_suite: u16,
    selected_group: u16,
    server_share: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(80 + server_share.len());
    body.extend_from_slice(&0x0303u16.to_be_bytes());
    body.extend_from_slice(server_random);
    let sid_len = session_id.len().min(32);
    body.push(sid_len as u8);
    body.extend_from_slice(&session_id[..sid_len]);
    body.extend_from_slice(&cipher_suite.to_be_bytes());
    body.push(0);

    let mut exts = Vec::with_capacity(16 + server_share.len());
    // supported_versions (0x002b) -> 0x0304
    exts.extend_from_slice(&EXT_SUPPORTED_VERSIONS.to_be_bytes());
    exts.extend_from_slice(&2u16.to_be_bytes());
    exts.extend_from_slice(&0x0304u16.to_be_bytes());
    // key_share (0x0033) -> KeyShareEntry { group, key_exchange }
    let ks_len = 2 + 2 + server_share.len();
    exts.extend_from_slice(&EXT_KEY_SHARE.to_be_bytes());
    exts.extend_from_slice(&(ks_len as u16).to_be_bytes());
    exts.extend_from_slice(&selected_group.to_be_bytes());
    exts.extend_from_slice(&(server_share.len() as u16).to_be_bytes());
    exts.extend_from_slice(server_share);

    body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
    body.extend_from_slice(&exts);
    wrap_handshake_msg(HANDSHAKE_SERVER_HELLO, &body)
}

/// Parsed fields of a TLS 1.3 `ServerHello` handshake message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedServerHello {
    pub server_random: [u8; 32],
    pub session_id: Vec<u8>,
    pub cipher_suite: u16,
    pub selected_group: u16,
    pub server_share: Vec<u8>,
}

/// Parse an RFC 8446 Section 4.1.3 `ServerHello` handshake message.
pub fn parse_server_hello(handshake_msg: &[u8]) -> Result<ParsedServerHello, NetError> {
    let body = unwrap_handshake_msg(handshake_msg, HANDSHAKE_SERVER_HELLO)?;
    if body.len() < 38 {
        return Err(NetError::InvalidResponse);
    }
    let mut server_random = [0u8; 32];
    server_random.copy_from_slice(&body[2..34]);
    let mut pos = 34usize;
    let sid_len = body[pos] as usize;
    pos += 1;
    if pos + sid_len + 3 + 2 > body.len() {
        return Err(NetError::InvalidResponse);
    }
    let session_id = body[pos..pos + sid_len].to_vec();
    pos += sid_len;
    let cipher_suite = u16::from_be_bytes([body[pos], body[pos + 1]]);
    pos += 3;
    let exts_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2;
    if pos + exts_len != body.len() {
        return Err(NetError::InvalidResponse);
    }
    let mut selected_group = 0u16;
    let mut server_share = Vec::new();
    while pos < body.len() {
        if pos + 4 > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_type = u16::from_be_bytes([body[pos], body[pos + 1]]);
        let ext_len = u16::from_be_bytes([body[pos + 2], body[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_data = &body[pos..pos + ext_len];
        pos += ext_len;
        if ext_type == EXT_KEY_SHARE && ext_data.len() >= 4 {
            selected_group = u16::from_be_bytes([ext_data[0], ext_data[1]]);
            let klen = u16::from_be_bytes([ext_data[2], ext_data[3]]) as usize;
            if 4 + klen == ext_data.len() {
                server_share = ext_data[4..].to_vec();
            }
        }
    }
    if server_share.is_empty() {
        return Err(NetError::InvalidResponse);
    }
    Ok(ParsedServerHello {
        server_random,
        session_id,
        cipher_suite,
        selected_group,
        server_share,
    })
}

/// Build an RFC 8446 Section 4.3.1 `EncryptedExtensions` handshake message.
pub fn encode_encrypted_extensions(selected_alpn: Option<&str>) -> Vec<u8> {
    let mut exts = Vec::new();
    if let Some(proto) = selected_alpn {
        let pb = proto.as_bytes();
        let list_len = 1 + pb.len();
        let ext_len = 2 + list_len;
        exts.extend_from_slice(&EXT_ALPN.to_be_bytes());
        exts.extend_from_slice(&(ext_len as u16).to_be_bytes());
        exts.extend_from_slice(&(list_len as u16).to_be_bytes());
        exts.push(pb.len() as u8);
        exts.extend_from_slice(pb);
    }
    let mut body = Vec::with_capacity(2 + exts.len());
    body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
    body.extend_from_slice(&exts);
    wrap_handshake_msg(HANDSHAKE_ENCRYPTED_EXTENSIONS, &body)
}

/// Parse an `EncryptedExtensions` handshake message and return the negotiated ALPN protocol.
pub fn parse_encrypted_extensions(handshake_msg: &[u8]) -> Result<Option<String>, NetError> {
    let body = unwrap_handshake_msg(handshake_msg, HANDSHAKE_ENCRYPTED_EXTENSIONS)?;
    if body.len() < 2 {
        return Err(NetError::InvalidResponse);
    }
    let exts_len = u16::from_be_bytes([body[0], body[1]]) as usize;
    if 2 + exts_len != body.len() {
        return Err(NetError::InvalidResponse);
    }
    let mut pos = 2usize;
    let mut selected_alpn = None;
    while pos < body.len() {
        if pos + 4 > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_type = u16::from_be_bytes([body[pos], body[pos + 1]]);
        let ext_len = u16::from_be_bytes([body[pos + 2], body[pos + 3]]) as usize;
        pos += 4;
        if pos + ext_len > body.len() {
            return Err(NetError::InvalidResponse);
        }
        let ext_data = &body[pos..pos + ext_len];
        pos += ext_len;
        if ext_type == EXT_ALPN && ext_data.len() >= 3 {
            let plen = ext_data[2] as usize;
            if 3 + plen <= ext_data.len() {
                selected_alpn = String::from_utf8(ext_data[3..3 + plen].to_vec()).ok();
            }
        }
    }
    Ok(selected_alpn)
}

/// Build an RFC 8446 Section 4.4.4 `Finished` handshake message.
pub fn encode_finished(verify_data: &[u8; 32]) -> Vec<u8> {
    wrap_handshake_msg(HANDSHAKE_FINISHED, verify_data)
}

/// Parse an RFC 8446 Section 4.4.4 `Finished` handshake message.
pub fn parse_finished(handshake_msg: &[u8]) -> Result<[u8; 32], NetError> {
    let body = unwrap_handshake_msg(handshake_msg, HANDSHAKE_FINISHED)?;
    if body.len() != 32 {
        return Err(NetError::InvalidResponse);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(body);
    Ok(out)
}

fn ct_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Established TLS 1.3 session state with negotiated group, ALPN, and application traffic keys.
#[derive(Clone, Debug)]
pub struct Tls13Session {
    pub is_client: bool,
    pub negotiated_group: u16,
    pub negotiated_alpn: Option<String>,
    pub cipher_suite: u16,
    pub app_keys: Tls13ApplicationKeys,
    pub write_seq: u64,
    pub read_seq: u64,
}

impl Tls13Session {
    /// Name of the negotiated key-exchange group (`"X25519MLKEM768"` or `"X25519"`).
    pub fn group_name(&self) -> &'static str {
        match self.negotiated_group {
            CODEPOINT_X25519_MLKEM768 => "X25519MLKEM768",
            CODEPOINT_X25519 => "X25519",
            _ => "unknown",
        }
    }

    /// Seal one application-data record and advance `write_seq`.
    pub fn seal_application_data(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let (key, iv) = if self.is_client {
            (
                &self.app_keys.client_write_key,
                &self.app_keys.client_write_iv,
            )
        } else {
            (
                &self.app_keys.server_write_key,
                &self.app_keys.server_write_iv,
            )
        };
        let seq = self.write_seq;
        self.write_seq = self.write_seq.saturating_add(1);
        seal_tls13_record(key, iv, seq, RECORD_APPLICATION_DATA, plaintext)
    }

    /// Open and authenticate one incoming application-data record and advance `read_seq`.
    pub fn open_application_data(
        &mut self,
        header: &[u8; 5],
        encrypted_body: &[u8],
    ) -> Result<Vec<u8>, NetError> {
        let (key, iv) = if self.is_client {
            (
                &self.app_keys.server_write_key,
                &self.app_keys.server_write_iv,
            )
        } else {
            (
                &self.app_keys.client_write_key,
                &self.app_keys.client_write_iv,
            )
        };
        let seq = self.read_seq;
        let (ct_type, plain) = open_tls13_record(key, iv, seq, header, encrypted_body)?;
        self.read_seq = self.read_seq.saturating_add(1);
        if ct_type != RECORD_APPLICATION_DATA {
            return Err(NetError::InvalidResponse);
        }
        Ok(plain)
    }
}

/// Client-side TLS 1.3 handshake state machine supporting `X25519MLKEM768` (`0x11EC`) and `X25519` (`0x001D`).
pub struct Tls13ClientHandshaker {
    shares: ClientKeyShares,
    client_hello_msg: Vec<u8>,
}

impl Tls13ClientHandshaker {
    /// Start a client handshake and return `(handshaker, client_hello_record_bytes)`.
    pub fn start(sni_host: &str, alpn_protocols: &[&str]) -> Result<(Self, Vec<u8>), NetError> {
        let shares = ClientKeyShares::generate()?;
        let mut client_random = [0u8; 32];
        fill_random(&mut client_random)
            .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc rng failed: {e}"))))?;
        let mut session_id = [0u8; 32];
        fill_random(&mut session_id)
            .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc rng failed: {e}"))))?;
        let client_hello_msg = encode_client_hello(
            sni_host,
            alpn_protocols,
            &client_random,
            &session_id,
            &shares,
        );
        let record = encode_plaintext_record(RECORD_HANDSHAKE, &client_hello_msg);
        Ok((
            Self {
                shares,
                client_hello_msg,
            },
            record,
        ))
    }

    /// Process the server flight (`ServerHello` + encrypted `EncryptedExtensions` & `Finished` records)
    /// and return `(established_session, client_finished_record)`.
    pub fn complete_server_flight(
        self,
        server_hello_msg: &[u8],
        encrypted_records: &[([u8; 5], Vec<u8>)],
    ) -> Result<(Tls13Session, Vec<u8>), NetError> {
        if encrypted_records.len() < 2 {
            return Err(NetError::InvalidResponse);
        }
        let parsed_sh = parse_server_hello(server_hello_msg)?;
        let shared_secret: Vec<u8> = match parsed_sh.selected_group {
            CODEPOINT_X25519_MLKEM768 => {
                self.shares.finish_hybrid(&parsed_sh.server_share)?.to_vec()
            }
            CODEPOINT_X25519 => {
                let peer: [u8; 32] = parsed_sh
                    .server_share
                    .as_slice()
                    .try_into()
                    .map_err(|_| NetError::InvalidResponse)?;
                self.shares.finish_classical_x25519(&peer)?.to_vec()
            }
            _ => return Err(NetError::InvalidResponse),
        };

        let mut transcript =
            Vec::with_capacity(self.client_hello_msg.len() + server_hello_msg.len() + 128);
        transcript.extend_from_slice(&self.client_hello_msg);
        transcript.extend_from_slice(server_hello_msg);
        let hs_keys = Tls13HandshakeKeys::derive(&shared_secret, &transcript)?;

        let (ct0, ee_msg) = open_tls13_record(
            &hs_keys.server_write_key,
            &hs_keys.server_write_iv,
            0,
            &encrypted_records[0].0,
            &encrypted_records[0].1,
        )?;
        if ct0 != RECORD_HANDSHAKE {
            return Err(NetError::InvalidResponse);
        }
        let negotiated_alpn = parse_encrypted_extensions(&ee_msg)?;
        transcript.extend_from_slice(&ee_msg);

        let (ct1, fin_msg) = open_tls13_record(
            &hs_keys.server_write_key,
            &hs_keys.server_write_iv,
            1,
            &encrypted_records[1].0,
            &encrypted_records[1].1,
        )?;
        if ct1 != RECORD_HANDSHAKE {
            return Err(NetError::InvalidResponse);
        }
        let server_fin = parse_finished(&fin_msg)?;
        let th_before_sfin = sha256(&transcript);
        let expected_sfin = hs_keys.server_finished_verify_data(&th_before_sfin)?;
        if !ct_eq_32(&server_fin, &expected_sfin) {
            return Err(NetError::Io(Error::other(
                "TLS 1.3 ServerFinished verify_data mismatch",
            )));
        }
        transcript.extend_from_slice(&fin_msg);

        let app_keys = hs_keys.derive_application_keys(&transcript)?;
        let th_for_cfin = sha256(&transcript);
        let cfin_verify = hs_keys.client_finished_verify_data(&th_for_cfin)?;
        let cfin_msg = encode_finished(&cfin_verify);
        let cfin_record = seal_tls13_record(
            &hs_keys.client_write_key,
            &hs_keys.client_write_iv,
            0,
            RECORD_HANDSHAKE,
            &cfin_msg,
        );

        let session = Tls13Session {
            is_client: true,
            negotiated_group: parsed_sh.selected_group,
            negotiated_alpn,
            cipher_suite: parsed_sh.cipher_suite,
            app_keys,
            write_seq: 0,
            read_seq: 0,
        };
        Ok((session, cfin_record))
    }
}

/// Server-side TLS 1.3 handshake state machine supporting `X25519MLKEM768` (`0x11EC`) and `X25519` (`0x001D`).
pub struct Tls13ServerHandshaker {
    negotiated_group: u16,
    negotiated_alpn: Option<String>,
    cipher_suite: u16,
    hs_keys: Tls13HandshakeKeys,
    app_keys: Tls13ApplicationKeys,
    expected_client_finished: [u8; 32],
}

impl Tls13ServerHandshaker {
    /// Accept a `ClientHello` handshake message and return `(handshaker, server_flight_records)`.
    pub fn accept_client_hello(
        client_hello_msg: &[u8],
        preferred_alpn: Option<&str>,
    ) -> Result<(Self, Vec<Vec<u8>>), NetError> {
        let parsed_ch = parse_client_hello(client_hello_msg)?;
        let (selected_group, server_share, shared_secret) = if let Some((_, c_share)) = parsed_ch
            .key_shares
            .iter()
            .find(|(g, _)| *g == CODEPOINT_X25519_MLKEM768)
        {
            let (s_share, ss) = server_accept_hybrid(c_share)?;
            (CODEPOINT_X25519_MLKEM768, s_share, ss.to_vec())
        } else if let Some((_, c_share)) = parsed_ch
            .key_shares
            .iter()
            .find(|(g, _)| *g == CODEPOINT_X25519)
        {
            let peer: [u8; 32] = c_share
                .as_slice()
                .try_into()
                .map_err(|_| NetError::InvalidResponse)?;
            let mut sk = [0u8; 32];
            fill_random(&mut sk)
                .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc rng failed: {e}"))))?;
            let pk = x25519::public_key(&sk);
            let ss = x25519::shared_secret(&sk, &peer)
                .map_err(|e| NetError::Io(Error::other(format!("X25519 failed: {e}"))))?;
            (CODEPOINT_X25519, pk.to_vec(), ss.to_vec())
        } else {
            return Err(NetError::InvalidResponse);
        };

        let mut server_random = [0u8; 32];
        fill_random(&mut server_random)
            .map_err(|e| NetError::Io(Error::other(format!("anvil-pqc rng failed: {e}"))))?;
        let cipher_suite = CIPHER_TLS_AES_256_GCM_SHA384;
        let server_hello_msg = encode_server_hello(
            &server_random,
            &parsed_ch.session_id,
            cipher_suite,
            selected_group,
            &server_share,
        );

        let mut transcript =
            Vec::with_capacity(client_hello_msg.len() + server_hello_msg.len() + 128);
        transcript.extend_from_slice(client_hello_msg);
        transcript.extend_from_slice(&server_hello_msg);
        let hs_keys = Tls13HandshakeKeys::derive(&shared_secret, &transcript)?;

        let negotiated_alpn = preferred_alpn.and_then(|pref| {
            parsed_ch
                .alpn_protocols
                .iter()
                .find(|p| p.as_str() == pref)
                .cloned()
        });
        let ee_msg = encode_encrypted_extensions(negotiated_alpn.as_deref());
        let ee_record = seal_tls13_record(
            &hs_keys.server_write_key,
            &hs_keys.server_write_iv,
            0,
            RECORD_HANDSHAKE,
            &ee_msg,
        );
        transcript.extend_from_slice(&ee_msg);

        let th_before_sfin = sha256(&transcript);
        let sfin_verify = hs_keys.server_finished_verify_data(&th_before_sfin)?;
        let sfin_msg = encode_finished(&sfin_verify);
        let sfin_record = seal_tls13_record(
            &hs_keys.server_write_key,
            &hs_keys.server_write_iv,
            1,
            RECORD_HANDSHAKE,
            &sfin_msg,
        );
        transcript.extend_from_slice(&sfin_msg);

        let app_keys = hs_keys.derive_application_keys(&transcript)?;
        let th_for_cfin = sha256(&transcript);
        let expected_client_finished = hs_keys.client_finished_verify_data(&th_for_cfin)?;

        let sh_record = encode_plaintext_record(RECORD_HANDSHAKE, &server_hello_msg);
        Ok((
            Self {
                negotiated_group: selected_group,
                negotiated_alpn,
                cipher_suite,
                hs_keys,
                app_keys,
                expected_client_finished,
            },
            vec![sh_record, ee_record, sfin_record],
        ))
    }

    /// Verify the client's encrypted `Finished` record and return the established [`Tls13Session`].
    pub fn finish_client(
        self,
        header: &[u8; 5],
        encrypted_body: &[u8],
    ) -> Result<Tls13Session, NetError> {
        let (ct, fin_msg) = open_tls13_record(
            &self.hs_keys.client_write_key,
            &self.hs_keys.client_write_iv,
            0,
            header,
            encrypted_body,
        )?;
        if ct != RECORD_HANDSHAKE {
            return Err(NetError::InvalidResponse);
        }
        let client_fin = parse_finished(&fin_msg)?;
        if !ct_eq_32(&client_fin, &self.expected_client_finished) {
            return Err(NetError::Io(Error::other(
                "TLS 1.3 ClientFinished verify_data mismatch",
            )));
        }
        Ok(Tls13Session {
            is_client: false,
            negotiated_group: self.negotiated_group,
            negotiated_alpn: self.negotiated_alpn,
            cipher_suite: self.cipher_suite,
            app_keys: self.app_keys,
            write_seq: 0,
            read_seq: 0,
        })
    }
}

/// Encrypted TLS 1.3 record stream over any `Read + Write` byte stream.
pub struct Tls13Stream<S> {
    inner: S,
    session: Tls13Session,
    read_buf: Vec<u8>,
    read_pos: usize,
}

impl<S: std::io::Read + std::io::Write> Tls13Stream<S> {
    /// Perform a client-side TLS 1.3 handshake (`X25519MLKEM768` / `X25519`) over `stream`.
    pub fn connect(
        mut stream: S,
        sni_host: &str,
        alpn_protocols: &[&str],
    ) -> Result<Self, NetError> {
        let (handshaker, ch_record) = Tls13ClientHandshaker::start(sni_host, alpn_protocols)?;
        stream.write_all(&ch_record)?;
        stream.flush()?;

        let (sh_hdr, sh_body) = read_tls_record(&mut stream)?;
        if sh_hdr[0] != RECORD_HANDSHAKE {
            return Err(NetError::InvalidResponse);
        }
        let mut encrypted_flight = Vec::with_capacity(2);
        while encrypted_flight.len() < 2 {
            let (hdr, body) = read_tls_record(&mut stream)?;
            if hdr[0] == RECORD_CHANGE_CIPHER_SPEC {
                continue;
            }
            encrypted_flight.push((hdr, body));
        }

        let (session, cfin_record) =
            handshaker.complete_server_flight(&sh_body, &encrypted_flight)?;
        stream.write_all(&cfin_record)?;
        stream.flush()?;
        Ok(Self {
            inner: stream,
            session,
            read_buf: Vec::new(),
            read_pos: 0,
        })
    }

    /// Perform a server-side TLS 1.3 handshake (`X25519MLKEM768` / `X25519`) over `stream`.
    pub fn accept(mut stream: S, preferred_alpn: Option<&str>) -> Result<Self, NetError> {
        let (ch_hdr, ch_body) = read_tls_record(&mut stream)?;
        if ch_hdr[0] != RECORD_HANDSHAKE {
            return Err(NetError::InvalidResponse);
        }
        let (handshaker, flight) =
            Tls13ServerHandshaker::accept_client_hello(&ch_body, preferred_alpn)?;
        for rec in flight {
            stream.write_all(&rec)?;
        }
        stream.flush()?;

        loop {
            let (hdr, body) = read_tls_record(&mut stream)?;
            if hdr[0] == RECORD_CHANGE_CIPHER_SPEC {
                continue;
            }
            let session = handshaker.finish_client(&hdr, &body)?;
            return Ok(Self {
                inner: stream,
                session,
                read_buf: Vec::new(),
                read_pos: 0,
            });
        }
    }

    /// Negotiated TLS 1.3 session metadata and traffic keys.
    pub fn session(&self) -> &Tls13Session {
        &self.session
    }
}

impl<S: std::io::Read + std::io::Write> std::io::Read for Tls13Stream<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.read_pos >= self.read_buf.len() {
            let (hdr, body) = match read_tls_record(&mut self.inner) {
                Ok(pair) => pair,
                Err(NetError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(0);
                }
                Err(NetError::Io(e)) => return Err(e),
                Err(other) => return Err(Error::other(other.to_string())),
            };
            let plain = self
                .session
                .open_application_data(&hdr, &body)
                .map_err(|e| Error::other(e.to_string()))?;
            self.read_buf = plain;
            self.read_pos = 0;
        }
        let avail = &self.read_buf[self.read_pos..];
        let to_copy = avail.len().min(buf.len());
        buf[..to_copy].copy_from_slice(&avail[..to_copy]);
        self.read_pos += to_copy;
        Ok(to_copy)
    }
}

impl<S: std::io::Read + std::io::Write> std::io::Write for Tls13Stream<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let chunk = &buf[..buf.len().min(16384)];
        let record = self.session.seal_application_data(chunk);
        self.inner.write_all(&record)?;
        Ok(chunk.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn record_nonce(base_iv: &[u8; 12], seq: u64) -> [u8; 12] {
    let mut nonce = *base_iv;
    let seq_bytes = seq.to_be_bytes();
    for i in 0..8 {
        nonce[4 + i] ^= seq_bytes[i];
    }
    nonce
}

/// Run a deterministic self-test of `anvil-pqc` (`X25519MLKEM768` + `SHA3-256`)
/// and `anvil-cipher` (`HKDF-SHA256` + `AES-256-GCM`) once per process and
/// return the resulting `handshake_secret`.
pub fn verify_pqc_hybrid_self_test() -> Result<[u8; 32], NetError> {
    static RESULT: OnceLock<Result<[u8; 32], String>> = OnceLock::new();
    let res = RESULT.get_or_init(|| {
        let mlkem_seed = [0x42u8; mlkem::SEED_LEN];
        let client_x25519 = [0x11u8; 32];
        let classical_x25519 = [0x22u8; 32];
        let server_m = [0x33u8; 32];
        let server_x25519 = [0x44u8; 32];

        let client = ClientKeyShares::from_seeds(&mlkem_seed, &client_x25519, &classical_x25519);
        if client.hybrid_share().len() != CLIENT_SHARE_LEN {
            return Err("unexpected client hybrid share length".to_string());
        }
        let ext = client.encode_tls13_key_share_extension();
        let parsed = parse_tls13_key_share_extension(&ext)
            .map_err(|e| format!("key_share parse failed: {e}"))?;
        if parsed.len() != 2 || parsed[0].0 != CODEPOINT_X25519_MLKEM768 {
            return Err("unexpected key_share extension entries".to_string());
        }

        let (server_share, server_secret) =
            server_accept_hybrid_with(&parsed[0].1, &server_m, &server_x25519)
                .map_err(|e| format!("server hybrid respond failed: {e}"))?;
        if server_share.len() != SERVER_SHARE_LEN {
            return Err("unexpected server hybrid share length".to_string());
        }
        let client_secret = client
            .finish_hybrid(&server_share)
            .map_err(|e| format!("client hybrid finish failed: {e}"))?;
        if client_secret != server_secret {
            return Err("X25519MLKEM768 shared secret mismatch".to_string());
        }

        let mut transcript = Vec::with_capacity(ext.len() + server_share.len());
        transcript.extend_from_slice(&ext);
        transcript.extend_from_slice(&server_share);

        let client_keys = Tls13HandshakeKeys::derive(&client_secret, &transcript)
            .map_err(|e| format!("client key schedule failed: {e}"))?;
        let server_keys = Tls13HandshakeKeys::derive(&server_secret, &transcript)
            .map_err(|e| format!("server key schedule failed: {e}"))?;
        if client_keys != server_keys {
            return Err("TLS 1.3 handshake key schedule mismatch".to_string());
        }

        let aad = [0x17, 0x03, 0x03, 0x00, 0x15];
        let msg = b"anvil-pqc-tls13-ready";
        let (ct, tag) = client_keys.seal_client_record(0, &aad, msg);
        let opened = server_keys
            .open_client_record(0, &aad, &ct, &tag)
            .map_err(|e| format!("record open failed: {e}"))?;
        if opened != msg {
            return Err("decrypted record mismatch".to_string());
        }
        Ok(client_keys.handshake_secret)
    });
    match res {
        Ok(secret) => Ok(*secret),
        Err(msg) => Err(NetError::Io(Error::other(msg.clone()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_key_share_extension_and_tls13_key_schedule_round_trip() {
        let secret = verify_pqc_hybrid_self_test().expect("self-test succeeds");
        assert_ne!(secret, [0u8; 32]);

        let client = ClientKeyShares::generate().expect("fresh client shares");
        let ext = client.encode_tls13_key_share_extension();
        let entries = parse_tls13_key_share_extension(&ext).expect("valid extension");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, CODEPOINT_X25519_MLKEM768);
        assert_eq!(entries[0].1.len(), CLIENT_SHARE_LEN);
        assert_eq!(entries[1].0, CODEPOINT_X25519);
        assert_eq!(entries[1].1.len(), 32);

        let (server_share, server_ss) =
            server_accept_hybrid(&entries[0].1).expect("server encapsulates");
        let client_ss = client
            .finish_hybrid(&server_share)
            .expect("client decapsulates");
        assert_eq!(client_ss, server_ss);

        let keys = Tls13HandshakeKeys::derive(&client_ss, b"ClientHello||ServerHello")
            .expect("key schedule");
        let c_fin = keys
            .client_finished_verify_data(&keys.transcript_sha256)
            .expect("client finished");
        let s_fin = keys
            .server_finished_verify_data(&keys.transcript_sha256)
            .expect("server finished");
        assert_ne!(c_fin, s_fin);
    }

    #[test]
    fn tls13_x25519_mlkem768_wire_handshake_and_encrypted_records_round_trip() {
        use std::io::{Cursor, Read, Write};
        use std::net::{TcpListener, TcpStream};

        let (client_hs, ch_record) =
            Tls13ClientHandshaker::start("anvil.example", &["h2", "http/1.1"])
                .expect("client start");
        let mut ch_cursor = Cursor::new(ch_record);
        let (ch_hdr, ch_body) = read_tls_record(&mut ch_cursor).expect("read ch record");
        assert_eq!(ch_hdr[0], RECORD_HANDSHAKE);

        let (server_hs, flight) = Tls13ServerHandshaker::accept_client_hello(&ch_body, Some("h2"))
            .expect("server accept");
        assert_eq!(flight.len(), 3);

        let mut sh_cursor = Cursor::new(&flight[0]);
        let (_, sh_body) = read_tls_record(&mut sh_cursor).expect("read sh record");
        let mut ee_cursor = Cursor::new(&flight[1]);
        let ee_rec = read_tls_record(&mut ee_cursor).expect("read ee record");
        let mut sfin_cursor = Cursor::new(&flight[2]);
        let sfin_rec = read_tls_record(&mut sfin_cursor).expect("read sfin record");

        let (mut client_session, cfin_record) = client_hs
            .complete_server_flight(&sh_body, &[ee_rec, sfin_rec])
            .expect("client complete flight");
        assert_eq!(client_session.negotiated_group, CODEPOINT_X25519_MLKEM768);
        assert_eq!(client_session.group_name(), "X25519MLKEM768");
        assert_eq!(client_session.negotiated_alpn.as_deref(), Some("h2"));

        let mut cfin_cursor = Cursor::new(cfin_record);
        let (cfin_hdr, cfin_body) = read_tls_record(&mut cfin_cursor).expect("read cfin");
        let mut server_session = server_hs
            .finish_client(&cfin_hdr, &cfin_body)
            .expect("server finish");
        assert_eq!(server_session.negotiated_group, CODEPOINT_X25519_MLKEM768);
        assert_eq!(server_session.negotiated_alpn.as_deref(), Some("h2"));

        let app_wire = client_session.seal_application_data(b"GET / HTTP/2.0");
        let mut app_cursor = Cursor::new(app_wire);
        let (app_hdr, app_body) = read_tls_record(&mut app_cursor).expect("read app");
        let opened = server_session
            .open_application_data(&app_hdr, &app_body)
            .expect("server decrypt app");
        assert_eq!(opened, b"GET / HTTP/2.0");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().unwrap();
        let server_thread = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().expect("accept tcp");
            let mut tls = Tls13Stream::accept(tcp, Some("h2")).expect("tls13 accept");
            assert_eq!(tls.session().negotiated_group, CODEPOINT_X25519_MLKEM768);
            let mut req = [0u8; 11];
            tls.read_exact(&mut req).expect("read request");
            assert_eq!(&req, b"hello-anvil");
            tls.write_all(b"pong-mlkem768").expect("write response");
            tls.flush().expect("flush");
        });

        let client_tcp = TcpStream::connect(addr).expect("connect tcp");
        let mut client_tls = Tls13Stream::connect(client_tcp, "localhost", &["h2", "http/1.1"])
            .expect("tls13 connect");
        assert_eq!(
            client_tls.session().negotiated_group,
            CODEPOINT_X25519_MLKEM768
        );
        assert_eq!(client_tls.session().negotiated_alpn.as_deref(), Some("h2"));
        client_tls.write_all(b"hello-anvil").expect("client write");
        client_tls.flush().expect("client flush");
        let mut resp = Vec::new();
        client_tls.read_to_end(&mut resp).expect("client read");
        assert_eq!(resp, b"pong-mlkem768");
        server_thread.join().expect("server thread");
    }
}
