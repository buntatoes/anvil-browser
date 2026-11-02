// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! IPC bridge to the standalone Go `anvil-pqc` (`v.03` / `0.3.0`) post-quantum
//! cryptography binary (`github.com/buntatoes/anvil-pqc/cmd/anvil-pqc --worker`).

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

const OP_FILL_RANDOM: u8 = 1;
const OP_SHA3_256: u8 = 2;
const OP_X25519_PUBLIC_KEY: u8 = 7;
const OP_X25519_SHARED_SECRET: u8 = 8;
const OP_HYBRID_CLIENT_SHARE_FROM_SEED: u8 = 16;
const OP_HYBRID_CLIENT_SHARE: u8 = 17;
const OP_HYBRID_SERVER_RESPOND_WITH: u8 = 18;
const OP_HYBRID_SERVER_RESPOND: u8 = 19;
const OP_HYBRID_CLIENT_FINISH: u8 = 20;

static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_PQC_GO_BIN"));

/// Errors returned by the Go `anvil-pqc` engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PqcError {
    BadLength,
    NonCanonicalEncapsulationKey,
    LowOrderPoint,
    RandomFailed,
}

impl fmt::Display for PqcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength => write!(f, "anvil-pqc input length invalid"),
            Self::NonCanonicalEncapsulationKey => {
                write!(f, "ML-KEM-768 encapsulation key failed FIPS 203 check")
            }
            Self::LowOrderPoint => write!(f, "X25519 shared secret is all zeros"),
            Self::RandomFailed => write!(f, "OS CSPRNG or Go worker failed"),
        }
    }
}

impl std::error::Error for PqcError {}

fn map_error_code(code: u8) -> PqcError {
    match code {
        1 => PqcError::BadLength,
        2 => PqcError::NonCanonicalEncapsulationKey,
        3 => PqcError::LowOrderPoint,
        _ => PqcError::RandomFailed,
    }
}

struct GoPqcWorker {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

fn resolve_go_binary_path() -> PathBuf {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-pqc-go-v03-{}{}", std::process::id(), ext));
    if !tmp_path.exists() {
        let _ = std::fs::write(&tmp_path, EMBEDDED_GO_BIN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }
    }
    tmp_path
}

fn spawn_worker() -> Result<GoPqcWorker, PqcError> {
    let bin = resolve_go_binary_path();
    let mut cmd = Command::new(&bin);
    cmd.arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            let mut retry = Command::new(&bin);
            retry
                .arg("--worker")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                retry.creation_flags(CREATE_NO_WINDOW);
            }
            retry.spawn().map_err(|_| PqcError::RandomFailed)?
        }
    };
    let stdin = child.stdin.take().ok_or(PqcError::RandomFailed)?;
    let stdout = child.stdout.take().ok_or(PqcError::RandomFailed)?;
    Ok(GoPqcWorker {
        _child: child,
        stdin,
        stdout,
    })
}

fn worker_slot() -> &'static Mutex<Option<GoPqcWorker>> {
    static WORKER: OnceLock<Mutex<Option<GoPqcWorker>>> = OnceLock::new();
    WORKER.get_or_init(|| Mutex::new(None))
}

fn with_worker<F, T>(mut f: F) -> Result<T, PqcError>
where
    F: FnMut(&mut GoPqcWorker) -> Result<T, PqcError>,
{
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(spawn_worker()?);
    }
    let res = f(guard.as_mut().unwrap());
    if res.is_err() {
        // Re-spawn on next call if I/O desynchronized
    }
    res
}

/// Fill `out` with cryptographically secure random bytes via the Go `anvil-pqc` engine.
pub fn fill_random(out: &mut [u8]) -> Result<(), PqcError> {
    with_worker(|w| {
        let mut req = [0u8; 5];
        req[0] = OP_FILL_RANDOM;
        req[1..5].copy_from_slice(&(out.len() as u32).to_le_bytes());
        w.stdin
            .write_all(&req)
            .map_err(|_| PqcError::RandomFailed)?;
        w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
        let mut status = [0u8; 1];
        w.stdout
            .read_exact(&mut status)
            .map_err(|_| PqcError::RandomFailed)?;
        if status[0] != 0 {
            return Err(map_error_code(status[0]));
        }
        w.stdout
            .read_exact(out)
            .map_err(|_| PqcError::RandomFailed)?;
        Ok(())
    })
}

/// NIST FIPS 203 ML-KEM-768 constants.
pub mod mlkem {
    pub const SEED_LEN: usize = 64;
    pub const ENCAPSULATION_KEY_LEN: usize = 1184;
    pub const DECAPSULATION_KEY_LEN: usize = 2400;
    pub const CIPHERTEXT_LEN: usize = 1088;
    pub const SHARED_SECRET_LEN: usize = 32;
}

/// NIST FIPS 202 SHA-3 (`Keccak-f[1600]`) functions routed to the Go `anvil-pqc` engine.
pub mod keccak {
    use super::*;

    /// Compute SHA3-256 over the concatenation of `parts`.
    pub fn sha3_256(parts: &[&[u8]]) -> [u8; 32] {
        with_worker(|w| {
            let mut wire = Vec::with_capacity(5 + parts.iter().map(|p| 4 + p.len()).sum::<usize>());
            wire.push(OP_SHA3_256);
            wire.extend_from_slice(&(parts.len() as u32).to_le_bytes());
            for p in parts {
                wire.extend_from_slice(&(p.len() as u32).to_le_bytes());
                wire.extend_from_slice(p);
            }
            w.stdin
                .write_all(&wire)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut sum = [0u8; 32];
            w.stdout
                .read_exact(&mut sum)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok(sum)
        })
        .unwrap_or([0u8; 32])
    }
}

/// RFC 7748 X25519 functions routed to the Go `anvil-pqc` engine.
pub mod x25519 {
    use super::*;

    /// Derive the 32-byte X25519 public key for `secret`.
    pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
        with_worker(|w| {
            let mut req = [0u8; 33];
            req[0] = OP_X25519_PUBLIC_KEY;
            req[1..33].copy_from_slice(secret);
            w.stdin
                .write_all(&req)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut pub_key = [0u8; 32];
            w.stdout
                .read_exact(&mut pub_key)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok(pub_key)
        })
        .unwrap_or([0u8; 32])
    }

    /// Compute the 32-byte X25519 shared secret with RFC 7748 §6.1 all-zero check.
    pub fn shared_secret(secret: &[u8; 32], peer_public: &[u8; 32]) -> Result<[u8; 32], PqcError> {
        with_worker(|w| {
            let mut req = [0u8; 65];
            req[0] = OP_X25519_SHARED_SECRET;
            req[1..33].copy_from_slice(secret);
            req[33..65].copy_from_slice(peer_public);
            w.stdin
                .write_all(&req)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut ss = [0u8; 32];
            w.stdout
                .read_exact(&mut ss)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok(ss)
        })
    }
}

/// TLS 1.3 `X25519MLKEM768` (`0x11EC`) hybrid key exchange routed to the Go `anvil-pqc` engine.
pub mod hybrid {
    use super::*;

    pub const CODEPOINT: u16 = 0x11ec;
    pub const CLIENT_SHARE_LEN: usize = 1216;
    pub const SERVER_SHARE_LEN: usize = 1120;
    pub const SHARED_SECRET_LEN: usize = 64;

    /// Client-side state for an in-flight `X25519MLKEM768` handshake.
    pub struct ClientState {
        dk_bytes: Vec<u8>,
        x25519_secret: [u8; 32],
    }

    impl Drop for ClientState {
        fn drop(&mut self) {
            for b in &mut self.dk_bytes {
                *b = 0;
            }
            self.x25519_secret = [0u8; 32];
            std::hint::black_box(&self.dk_bytes);
            std::hint::black_box(&self.x25519_secret);
        }
    }

    impl ClientState {
        /// Complete the client side of the `X25519MLKEM768` exchange against `server_share`.
        pub fn finish(&self, server_share: &[u8]) -> Result<[u8; SHARED_SECRET_LEN], PqcError> {
            if server_share.len() != SERVER_SHARE_LEN {
                return Err(PqcError::BadLength);
            }
            with_worker(|w| {
                let mut wire = Vec::with_capacity(
                    1 + mlkem::DECAPSULATION_KEY_LEN + 32 + 4 + server_share.len(),
                );
                wire.push(OP_HYBRID_CLIENT_FINISH);
                wire.extend_from_slice(&self.dk_bytes);
                wire.extend_from_slice(&self.x25519_secret);
                wire.extend_from_slice(&(server_share.len() as u32).to_le_bytes());
                wire.extend_from_slice(server_share);
                w.stdin
                    .write_all(&wire)
                    .map_err(|_| PqcError::RandomFailed)?;
                w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
                let mut status = [0u8; 1];
                w.stdout
                    .read_exact(&mut status)
                    .map_err(|_| PqcError::RandomFailed)?;
                if status[0] != 0 {
                    return Err(map_error_code(status[0]));
                }
                let mut ss = [0u8; SHARED_SECRET_LEN];
                w.stdout
                    .read_exact(&mut ss)
                    .map_err(|_| PqcError::RandomFailed)?;
                Ok(ss)
            })
        }
    }

    /// Generate a fresh `X25519MLKEM768` client share using OS randomness.
    pub fn client_share() -> Result<(ClientState, Vec<u8>), PqcError> {
        with_worker(|w| {
            w.stdin
                .write_all(&[OP_HYBRID_CLIENT_SHARE])
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut dk_bytes = vec![0u8; mlkem::DECAPSULATION_KEY_LEN];
            let mut x25519_secret = [0u8; 32];
            let mut share = vec![0u8; CLIENT_SHARE_LEN];
            w.stdout
                .read_exact(&mut dk_bytes)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut x25519_secret)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut share)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok((
                ClientState {
                    dk_bytes,
                    x25519_secret,
                },
                share,
            ))
        })
    }

    /// Deterministic `X25519MLKEM768` client share generation from seeds.
    pub fn client_share_from_seed(
        mlkem_seed: &[u8; mlkem::SEED_LEN],
        x25519_secret: &[u8; 32],
    ) -> (ClientState, Vec<u8>) {
        with_worker(|w| {
            let mut req = [0u8; 1 + mlkem::SEED_LEN + 32];
            req[0] = OP_HYBRID_CLIENT_SHARE_FROM_SEED;
            req[1..1 + mlkem::SEED_LEN].copy_from_slice(mlkem_seed);
            req[1 + mlkem::SEED_LEN..].copy_from_slice(x25519_secret);
            w.stdin
                .write_all(&req)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut dk_bytes = vec![0u8; mlkem::DECAPSULATION_KEY_LEN];
            let mut xsec = [0u8; 32];
            let mut share = vec![0u8; CLIENT_SHARE_LEN];
            w.stdout
                .read_exact(&mut dk_bytes)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut xsec)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut share)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok((
                ClientState {
                    dk_bytes,
                    x25519_secret: xsec,
                },
                share,
            ))
        })
        .expect("Go anvil-pqc deterministic client_share_from_seed failed")
    }

    /// Respond to a client share using OS randomness.
    pub fn server_respond(
        client_share: &[u8],
    ) -> Result<(Vec<u8>, [u8; SHARED_SECRET_LEN]), PqcError> {
        if client_share.len() != CLIENT_SHARE_LEN {
            return Err(PqcError::BadLength);
        }
        with_worker(|w| {
            let mut wire = Vec::with_capacity(5 + client_share.len());
            wire.push(OP_HYBRID_SERVER_RESPOND);
            wire.extend_from_slice(&(client_share.len() as u32).to_le_bytes());
            wire.extend_from_slice(client_share);
            w.stdin
                .write_all(&wire)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut server_share = vec![0u8; SERVER_SHARE_LEN];
            let mut ss = [0u8; SHARED_SECRET_LEN];
            w.stdout
                .read_exact(&mut server_share)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut ss)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok((server_share, ss))
        })
    }

    /// Deterministic server response to a client share.
    pub fn server_respond_with(
        client_share: &[u8],
        m: &[u8; 32],
        x25519_secret: &[u8; 32],
    ) -> Result<(Vec<u8>, [u8; SHARED_SECRET_LEN]), PqcError> {
        if client_share.len() != CLIENT_SHARE_LEN {
            return Err(PqcError::BadLength);
        }
        with_worker(|w| {
            let mut wire = Vec::with_capacity(5 + client_share.len() + 64);
            wire.push(OP_HYBRID_SERVER_RESPOND_WITH);
            wire.extend_from_slice(&(client_share.len() as u32).to_le_bytes());
            wire.extend_from_slice(client_share);
            wire.extend_from_slice(m);
            wire.extend_from_slice(x25519_secret);
            w.stdin
                .write_all(&wire)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdin.flush().map_err(|_| PqcError::RandomFailed)?;
            let mut status = [0u8; 1];
            w.stdout
                .read_exact(&mut status)
                .map_err(|_| PqcError::RandomFailed)?;
            if status[0] != 0 {
                return Err(map_error_code(status[0]));
            }
            let mut server_share = vec![0u8; SERVER_SHARE_LEN];
            let mut ss = [0u8; SHARED_SECRET_LEN];
            w.stdout
                .read_exact(&mut server_share)
                .map_err(|_| PqcError::RandomFailed)?;
            w.stdout
                .read_exact(&mut ss)
                .map_err(|_| PqcError::RandomFailed)?;
            Ok((server_share, ss))
        })
    }
}
