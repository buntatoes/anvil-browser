// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! IPC bridge to the standalone Go `anvil-hpack` (`v.02`) engine binary
//! (`github.com/buntatoes/anvil-hpack/cmd/anvil-hpack --worker`).

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// Default dynamic table size in bytes (RFC 7540 §6.5.2 `SETTINGS_HEADER_TABLE_SIZE` = 4,096).
pub const DEFAULT_TABLE_SIZE: usize = 4096;

/// Default cap on the decoded header list size per block (64 KiB).
pub const DEFAULT_MAX_HEADER_LIST_SIZE: usize = 64 * 1024;

const OP_NEW_ENCODER: u8 = 1;
const OP_ENCODER_SET_HUFFMAN: u8 = 2;
const OP_ENCODER_SET_MAX_TABLE_SIZE: u8 = 3;
const OP_ENCODER_ENCODE: u8 = 4;
const OP_NEW_DECODER: u8 = 5;
const OP_DECODER_SET_MAX_ALLOWED_SIZE: u8 = 6;
const OP_DECODER_DECODE: u8 = 7;
const OP_FREE_ENCODER: u8 = 8;
const OP_FREE_DECODER: u8 = 9;

static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_HPACK_GO_BIN"));

/// One HTTP/2 header field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    pub never_indexed: bool,
}

impl Header {
    pub fn new(name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self {
            name: name.as_ref().to_vec(),
            value: value.as_ref().to_vec(),
            never_indexed: false,
        }
    }

    pub fn sensitive(name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self {
            name: name.as_ref().to_vec(),
            value: value.as_ref().to_vec(),
            never_indexed: true,
        }
    }

    pub fn name_str(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    pub fn value_str(&self) -> String {
        String::from_utf8_lossy(&self.value).into_owned()
    }
}

/// Errors produced while decoding an HPACK header block via the Go `anvil-hpack` engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HpackError {
    Truncated,
    IntegerOverflow,
    InvalidIndex(usize),
    InvalidHuffman,
    StringTooLong(usize),
    TableSizeTooLarge(usize),
    LateTableSizeUpdate,
    HeaderListTooLarge(usize),
}

impl fmt::Display for HpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "HPACK block ended mid-field"),
            Self::IntegerOverflow => write!(f, "HPACK integer overflowed"),
            Self::InvalidIndex(idx) => write!(f, "HPACK table index {idx} out of range"),
            Self::InvalidHuffman => write!(f, "invalid HPACK Huffman string"),
            Self::StringTooLong(len) => write!(f, "HPACK string of {len} bytes exceeds limit"),
            Self::TableSizeTooLarge(sz) => {
                write!(f, "HPACK dynamic table size {sz} exceeds allowed limit")
            }
            Self::LateTableSizeUpdate => {
                write!(f, "HPACK dynamic table size update after a header field")
            }
            Self::HeaderListTooLarge(sz) => {
                write!(f, "HPACK decoded header list of {sz} bytes exceeds limit")
            }
        }
    }
}

impl std::error::Error for HpackError {}

struct GoHpackWorker {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

fn resolve_go_binary_path() -> PathBuf {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-hpack-go-v02-{}{}", std::process::id(), ext));
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

fn spawn_worker() -> Result<GoHpackWorker, HpackError> {
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
            retry.spawn().map_err(|_| HpackError::Truncated)?
        }
    };
    let stdin = child.stdin.take().ok_or(HpackError::Truncated)?;
    let stdout = child.stdout.take().ok_or(HpackError::Truncated)?;
    Ok(GoHpackWorker {
        _child: child,
        stdin,
        stdout,
    })
}

fn worker_slot() -> &'static Mutex<Option<GoHpackWorker>> {
    static WORKER: OnceLock<Mutex<Option<GoHpackWorker>>> = OnceLock::new();
    WORKER.get_or_init(|| Mutex::new(None))
}

/// Pre-spawns the persistent Go `anvil-hpack` worker process if not already running.
pub fn warmup_worker() {
    if let Ok(mut guard) = worker_slot().lock()
        && guard.is_none()
    {
        *guard = spawn_worker().ok();
    }
}

/// HPACK header block encoder backed by the Go `anvil-hpack` (`v.02`) engine.
pub struct Encoder {
    id: u32,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self::with_table_size(DEFAULT_TABLE_SIZE)
    }

    pub fn with_table_size(size: usize) -> Self {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = spawn_worker().ok();
        }
        let Some(w) = guard.as_mut() else {
            return Self { id: 0 };
        };
        let mut req = [0u8; 5];
        req[0] = OP_NEW_ENCODER;
        req[1..5].copy_from_slice(&(size as u32).to_le_bytes());
        if w.stdin.write_all(&req).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return Self { id: 0 };
        }
        let mut resp = [0u8; 5];
        if w.stdout.read_exact(&mut resp).is_err() || resp[0] != 0 {
            *guard = None;
            return Self { id: 0 };
        }
        let id = u32::from_le_bytes([resp[1], resp[2], resp[3], resp[4]]);
        Self { id }
    }

    pub fn set_huffman(&mut self, on: bool) {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(w) = guard.as_mut() else {
            return;
        };
        let mut req = [0u8; 6];
        req[0] = OP_ENCODER_SET_HUFFMAN;
        req[1..5].copy_from_slice(&self.id.to_le_bytes());
        req[5] = u8::from(on);
        if w.stdin.write_all(&req).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return;
        }
        let mut status = [0u8; 1];
        if w.stdout.read_exact(&mut status).is_err() {
            *guard = None;
        }
    }

    pub fn set_max_table_size(&mut self, size: usize) {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(w) = guard.as_mut() else {
            return;
        };
        let mut req = [0u8; 9];
        req[0] = OP_ENCODER_SET_MAX_TABLE_SIZE;
        req[1..5].copy_from_slice(&self.id.to_le_bytes());
        req[5..9].copy_from_slice(&(size as u32).to_le_bytes());
        if w.stdin.write_all(&req).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return;
        }
        let mut status = [0u8; 1];
        if w.stdout.read_exact(&mut status).is_err() {
            *guard = None;
        }
    }

    pub fn encode(&mut self, headers: &[Header], out: &mut Vec<u8>) {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = spawn_worker().ok();
        }
        let Some(w) = guard.as_mut() else {
            return;
        };
        let mut wire = Vec::with_capacity(9 + headers.len() * 24);
        wire.push(OP_ENCODER_ENCODE);
        wire.extend_from_slice(&self.id.to_le_bytes());
        wire.extend_from_slice(&(headers.len() as u32).to_le_bytes());
        for h in headers {
            wire.extend_from_slice(&(h.name.len() as u32).to_le_bytes());
            wire.extend_from_slice(&(h.value.len() as u32).to_le_bytes());
            wire.push(u8::from(h.never_indexed));
            wire.extend_from_slice(&h.name);
            wire.extend_from_slice(&h.value);
        }
        if w.stdin.write_all(&wire).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return;
        }
        let mut resp = [0u8; 9];
        if w.stdout.read_exact(&mut resp).is_err() || resp[0] != 0 {
            *guard = None;
            return;
        }
        let block_len = u32::from_le_bytes([resp[5], resp[6], resp[7], resp[8]]) as usize;
        let start = out.len();
        out.resize(start + block_len, 0);
        if w.stdout.read_exact(&mut out[start..]).is_err() {
            out.truncate(start);
            *guard = None;
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if self.id == 0 {
            return;
        }
        if let Ok(mut guard) = worker_slot().lock()
            && let Some(w) = guard.as_mut()
        {
            let mut req = [0u8; 5];
            req[0] = OP_FREE_ENCODER;
            req[1..5].copy_from_slice(&self.id.to_le_bytes());
            let _ = w.stdin.write_all(&req);
        }
    }
}

/// HPACK header block decoder backed by the Go `anvil-hpack` (`v.02`) engine.
pub struct Decoder {
    id: u32,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_TABLE_SIZE, DEFAULT_MAX_HEADER_LIST_SIZE)
    }

    pub fn with_limits(max_table_size: usize, max_header_list_size: usize) -> Self {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = spawn_worker().ok();
        }
        let Some(w) = guard.as_mut() else {
            return Self { id: 0 };
        };
        let mut req = [0u8; 9];
        req[0] = OP_NEW_DECODER;
        req[1..5].copy_from_slice(&(max_table_size as u32).to_le_bytes());
        req[5..9].copy_from_slice(&(max_header_list_size as u32).to_le_bytes());
        if w.stdin.write_all(&req).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return Self { id: 0 };
        }
        let mut resp = [0u8; 5];
        if w.stdout.read_exact(&mut resp).is_err() || resp[0] != 0 {
            *guard = None;
            return Self { id: 0 };
        }
        let id = u32::from_le_bytes([resp[1], resp[2], resp[3], resp[4]]);
        Self { id }
    }

    pub fn set_max_allowed_table_size(&mut self, size: usize) {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(w) = guard.as_mut() else {
            return;
        };
        let mut req = [0u8; 9];
        req[0] = OP_DECODER_SET_MAX_ALLOWED_SIZE;
        req[1..5].copy_from_slice(&self.id.to_le_bytes());
        req[5..9].copy_from_slice(&(size as u32).to_le_bytes());
        if w.stdin.write_all(&req).is_err() || w.stdin.flush().is_err() {
            *guard = None;
            return;
        }
        let mut status = [0u8; 1];
        if w.stdout.read_exact(&mut status).is_err() {
            *guard = None;
        }
    }

    pub fn decode(&mut self, block: &[u8]) -> Result<Vec<Header>, HpackError> {
        let mut guard = worker_slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(spawn_worker()?);
        }
        let w = guard.as_mut().unwrap();
        let mut hdr = [0u8; 9];
        hdr[0] = OP_DECODER_DECODE;
        hdr[1..5].copy_from_slice(&self.id.to_le_bytes());
        hdr[5..9].copy_from_slice(&(block.len() as u32).to_le_bytes());
        if w.stdin.write_all(&hdr).is_err()
            || w.stdin.write_all(block).is_err()
            || w.stdin.flush().is_err()
        {
            *guard = None;
            return Err(HpackError::Truncated);
        }
        let mut status = [0u8; 1];
        if w.stdout.read_exact(&mut status).is_err() {
            *guard = None;
            return Err(HpackError::Truncated);
        }
        if status[0] == 0 {
            let mut meta = [0u8; 12];
            w.stdout
                .read_exact(&mut meta)
                .map_err(|_| HpackError::Truncated)?;
            let count = u32::from_le_bytes([meta[8], meta[9], meta[10], meta[11]]) as usize;
            let mut out = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                let mut fhdr = [0u8; 9];
                w.stdout
                    .read_exact(&mut fhdr)
                    .map_err(|_| HpackError::Truncated)?;
                let nlen = u32::from_le_bytes([fhdr[0], fhdr[1], fhdr[2], fhdr[3]]) as usize;
                let vlen = u32::from_le_bytes([fhdr[4], fhdr[5], fhdr[6], fhdr[7]]) as usize;
                let never = fhdr[8] != 0;
                let mut name = vec![0u8; nlen];
                let mut value = vec![0u8; vlen];
                w.stdout
                    .read_exact(&mut name)
                    .map_err(|_| HpackError::Truncated)?;
                w.stdout
                    .read_exact(&mut value)
                    .map_err(|_| HpackError::Truncated)?;
                out.push(Header {
                    name,
                    value,
                    never_indexed: never,
                });
            }
            Ok(out)
        } else {
            let mut ehdr = [0u8; 5];
            if w.stdout.read_exact(&mut ehdr).is_err() {
                *guard = None;
                return Err(HpackError::Truncated);
            }
            let kind = ehdr[0];
            let val = u32::from_le_bytes([ehdr[1], ehdr[2], ehdr[3], ehdr[4]]) as usize;
            Err(match kind {
                0 => HpackError::Truncated,
                1 => HpackError::IntegerOverflow,
                2 => HpackError::InvalidIndex(val),
                3 => HpackError::InvalidHuffman,
                4 => HpackError::StringTooLong(val),
                5 => HpackError::TableSizeTooLarge(val),
                6 => HpackError::LateTableSizeUpdate,
                7 => HpackError::HeaderListTooLarge(val),
                _ => HpackError::Truncated,
            })
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.id == 0 {
            return;
        }
        if let Ok(mut guard) = worker_slot().lock()
            && let Some(w) = guard.as_mut()
        {
            let mut req = [0u8; 5];
            req[0] = OP_FREE_DECODER;
            req[1..5].copy_from_slice(&self.id.to_le_bytes());
            let _ = w.stdin.write_all(&req);
        }
    }
}
