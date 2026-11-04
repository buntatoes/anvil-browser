// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! IPC bridge to the standalone Go `anvil-json` (`v.04`) engine binary
//! (`github.com/buntatoes/anvil-json/cmd/anvil-json --worker`).

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Default maximum nesting depth for arrays and objects.
pub const MAX_DEPTH: usize = 64;

/// Default maximum input length in bytes (16 MiB).
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

pub use MAX_DEPTH as MAX_JSON_DEPTH;
pub use MAX_INPUT_BYTES as MAX_JSON_BYTES;

const OP_PARSE: u8 = 1;
const OP_STRINGIFY: u8 = 2;
const OP_STRINGIFY_PRETTY: u8 = 3;
const OP_ESCAPE_STRING: u8 = 4;
const OP_RESOLVE_POINTER: u8 = 5;

const WIRE_TAG_NULL: u8 = 0;
const WIRE_TAG_BOOL: u8 = 1;
const WIRE_TAG_NUMBER: u8 = 2;
const WIRE_TAG_STRING: u8 = 3;
const WIRE_TAG_ARRAY: u8 = 4;
const WIRE_TAG_OBJECT: u8 = 5;

static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_JSON_GO_BIN"));
static WORKER_DISABLED: AtomicBool = AtomicBool::new(false);
static WORKER_EVER_SUCCEEDED: AtomicBool = AtomicBool::new(false);

/// Parsed RFC 8259 JSON value produced or consumed by the Go `anvil-json` engine.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn empty_object() -> Self {
        Self::Object(Vec::new())
    }

    pub fn empty_array() -> Self {
        Self::Array(Vec::new())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        let n = self.as_f64()?;
        if n.is_finite() && n.fract() == 0.0 && n >= (i64::MIN as f64) && n < (i64::MAX as f64) {
            Some(n as i64)
        } else {
            None
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        let n = self.as_f64()?;
        if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n < (u64::MAX as f64) {
            Some(n as u64)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<JsonValue>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_members(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            Self::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        self.as_members()
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, JsonValue)>> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        let entries = self.as_members()?;
        entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut JsonValue> {
        let entries = self.as_object_mut()?;
        entries
            .iter_mut()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key)?.as_u64()
    }

    pub fn get_array(&self, key: &str) -> Option<&[JsonValue]> {
        self.get(key)?.as_array()
    }

    pub fn get_object(&self, key: &str) -> Option<&[(String, JsonValue)]> {
        self.get(key)?.as_object()
    }

    pub fn get_path(&self, path: &[&str]) -> Option<&JsonValue> {
        let mut cur = self;
        for segment in path {
            cur = cur.get(segment)?;
        }
        Some(cur)
    }

    pub fn index(&self, idx: usize) -> Option<&JsonValue> {
        self.as_array()?.get(idx)
    }

    pub fn get_index(&self, idx: usize) -> Option<&JsonValue> {
        self.index(idx)
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Array(items) => items.len(),
            Self::Object(entries) => entries.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn set(&mut self, key: impl Into<String>, value: JsonValue) -> bool {
        let Some(entries) = self.as_object_mut() else {
            return false;
        };
        let key = key.into();
        if let Some(first_idx) = entries.iter().position(|(k, _)| *k == key) {
            entries[first_idx].1 = value;
            let mut i = first_idx + 1;
            while i < entries.len() {
                if entries[i].0 == key {
                    entries.remove(i);
                } else {
                    i += 1;
                }
            }
        } else {
            entries.push((key, value));
        }
        true
    }

    pub fn insert(&mut self, key: impl Into<String>, value: JsonValue) -> bool {
        self.set(key, value)
    }

    pub fn push(&mut self, value: JsonValue) -> bool {
        if let Self::Array(items) = self {
            items.push(value);
            true
        } else {
            false
        }
    }

    pub fn to_json_string(&self) -> String {
        stringify(self)
    }

    pub fn to_json_pretty(&self, indent: usize) -> String {
        stringify_pretty(self, indent)
    }

    pub fn resolve_pointer(&self, pointer: &str) -> Result<Option<JsonValue>, ParseError> {
        resolve_pointer(self, pointer)
    }
}

/// Parse failure with byte offset returned by the Go `anvil-json` engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

pub type JsonError = ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

fn encode_binary_value(dst: &mut Vec<u8>, v: &JsonValue) {
    match v {
        JsonValue::Null => dst.push(WIRE_TAG_NULL),
        JsonValue::Bool(b) => {
            dst.push(WIRE_TAG_BOOL);
            dst.push(u8::from(*b));
        }
        JsonValue::Number(n) => {
            dst.push(WIRE_TAG_NUMBER);
            dst.extend_from_slice(&n.to_bits().to_le_bytes());
        }
        JsonValue::String(s) => {
            dst.push(WIRE_TAG_STRING);
            dst.extend_from_slice(&(s.len() as u32).to_le_bytes());
            dst.extend_from_slice(s.as_bytes());
        }
        JsonValue::Array(items) => {
            dst.push(WIRE_TAG_ARRAY);
            dst.extend_from_slice(&(items.len() as u32).to_le_bytes());
            for elem in items {
                encode_binary_value(dst, elem);
            }
        }
        JsonValue::Object(members) => {
            dst.push(WIRE_TAG_OBJECT);
            dst.extend_from_slice(&(members.len() as u32).to_le_bytes());
            for (k, val) in members {
                dst.extend_from_slice(&(k.len() as u32).to_le_bytes());
                dst.extend_from_slice(k.as_bytes());
                encode_binary_value(dst, val);
            }
        }
    }
}

fn wire_err(offset: usize) -> ParseError {
    ParseError {
        message: "truncated binary JSON wire value".to_string(),
        offset,
    }
}

fn decode_binary_value(src: &[u8], pos: &mut usize) -> Result<JsonValue, ParseError> {
    let tag = *src.get(*pos).ok_or_else(|| wire_err(*pos))?;
    *pos += 1;
    match tag {
        WIRE_TAG_NULL => Ok(JsonValue::Null),
        WIRE_TAG_BOOL => {
            let b = *src.get(*pos).ok_or_else(|| wire_err(*pos))? != 0;
            *pos += 1;
            Ok(JsonValue::Bool(b))
        }
        WIRE_TAG_NUMBER => {
            let bytes = src.get(*pos..*pos + 8).ok_or_else(|| wire_err(*pos))?;
            *pos += 8;
            let bits = u64::from_le_bytes(bytes.try_into().unwrap());
            Ok(JsonValue::Number(f64::from_bits(bits)))
        }
        WIRE_TAG_STRING => {
            let len_bytes = src.get(*pos..*pos + 4).ok_or_else(|| wire_err(*pos))?;
            *pos += 4;
            let n = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
            let s_bytes = src.get(*pos..*pos + n).ok_or_else(|| wire_err(*pos))?;
            *pos += n;
            let s = String::from_utf8(s_bytes.to_vec()).map_err(|_| wire_err(*pos))?;
            Ok(JsonValue::String(s))
        }
        WIRE_TAG_ARRAY => {
            let len_bytes = src.get(*pos..*pos + 4).ok_or_else(|| wire_err(*pos))?;
            *pos += 4;
            let count = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
            let mut items = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                items.push(decode_binary_value(src, pos)?);
            }
            Ok(JsonValue::Array(items))
        }
        WIRE_TAG_OBJECT => {
            let len_bytes = src.get(*pos..*pos + 4).ok_or_else(|| wire_err(*pos))?;
            *pos += 4;
            let count = u32::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
            let mut members = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let klen_bytes = src.get(*pos..*pos + 4).ok_or_else(|| wire_err(*pos))?;
                *pos += 4;
                let klen = u32::from_le_bytes(klen_bytes.try_into().unwrap()) as usize;
                let k_bytes = src.get(*pos..*pos + klen).ok_or_else(|| wire_err(*pos))?;
                *pos += klen;
                let key = String::from_utf8(k_bytes.to_vec()).map_err(|_| wire_err(*pos))?;
                let val = decode_binary_value(src, pos)?;
                members.push((key, val));
            }
            Ok(JsonValue::Object(members))
        }
        _ => Err(wire_err(*pos)),
    }
}

struct GoJsonWorker {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

fn resolve_go_binary_path() -> Result<PathBuf, ParseError> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-json-go-v04-{}{}", std::process::id(), ext));
    let needs_write = match std::fs::metadata(&tmp_path) {
        Ok(meta) => meta.len() == 0,
        Err(_) => true,
    };
    if needs_write {
        if std::fs::write(&tmp_path, EMBEDDED_GO_BIN).is_err() {
            let _ = std::fs::remove_file(&tmp_path);
            WORKER_DISABLED.store(true, Ordering::Relaxed);
            return Err(ParseError {
                message: "failed to write Go anvil-json worker binary".to_string(),
                offset: 0,
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(tmp_path)
}

fn spawn_worker() -> Result<GoJsonWorker, ParseError> {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return Err(ParseError {
            message: "Go anvil-json worker disabled in confined process".to_string(),
            offset: 0,
        });
    }
    let bin = resolve_go_binary_path()?;
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
            match retry.spawn() {
                Ok(c) => c,
                Err(_) => {
                    if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
                        WORKER_DISABLED.store(true, Ordering::Relaxed);
                    }
                    return Err(ParseError {
                        message: "failed to spawn Go anvil-json worker".to_string(),
                        offset: 0,
                    });
                }
            }
        }
    };
    let stdin = child.stdin.take().ok_or_else(|| ParseError {
        message: "missing Go worker stdin".to_string(),
        offset: 0,
    })?;
    let stdout = child.stdout.take().ok_or_else(|| ParseError {
        message: "missing Go worker stdout".to_string(),
        offset: 0,
    })?;
    Ok(GoJsonWorker {
        _child: child,
        stdin,
        stdout,
    })
}

fn worker_slot() -> &'static Mutex<Option<GoJsonWorker>> {
    static WORKER: OnceLock<Mutex<Option<GoJsonWorker>>> = OnceLock::new();
    WORKER.get_or_init(|| Mutex::new(None))
}

/// Pre-spawns the persistent Go `anvil-json` worker process before sandbox lock-down.
pub fn warmup_worker() {
    if let Ok(mut guard) = worker_slot().lock()
        && guard.is_none()
    {
        *guard = spawn_worker().ok();
    }
}

pub fn parse(input: &str) -> Result<JsonValue, ParseError> {
    parse_with_limits(input, MAX_DEPTH, MAX_INPUT_BYTES)
}

pub fn parse_bytes(input: &[u8]) -> Result<JsonValue, ParseError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ParseError {
            message: "input byte length exceeds cap".to_string(),
            offset: MAX_INPUT_BYTES,
        });
    }
    let text = std::str::from_utf8(input).map_err(|e| ParseError {
        message: "input is not valid UTF-8".to_string(),
        offset: e.valid_up_to(),
    })?;
    parse_with_limits(text, MAX_DEPTH, MAX_INPUT_BYTES)
}

pub fn parse_with_limits(
    input: &str,
    max_depth: usize,
    max_bytes: usize,
) -> Result<JsonValue, ParseError> {
    let bytes = input.as_bytes();
    if bytes.len() > max_bytes {
        return Err(ParseError {
            message: "input byte length exceeds cap".to_string(),
            offset: max_bytes,
        });
    }
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return parse_with_limits_rust(input, max_depth, max_bytes);
    }
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        match spawn_worker() {
            Ok(w) => *guard = Some(w),
            Err(_) => return parse_with_limits_rust(input, max_depth, max_bytes),
        }
    }
    let w = guard.as_mut().unwrap();
    let mut hdr = [0u8; 13];
    hdr[0] = OP_PARSE;
    hdr[1..5].copy_from_slice(&(max_depth.min(u32::MAX as usize) as u32).to_le_bytes());
    hdr[5..9].copy_from_slice(&(max_bytes.min(u32::MAX as usize) as u32).to_le_bytes());
    hdr[9..13].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    if w.stdin.write_all(&hdr).is_err()
        || w.stdin.write_all(bytes).is_err()
        || w.stdin.flush().is_err()
    {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return parse_with_limits_rust(input, max_depth, max_bytes);
    }
    let mut status = [0u8; 1];
    if w.stdout.read_exact(&mut status).is_err() {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return parse_with_limits_rust(input, max_depth, max_bytes);
    }
    WORKER_EVER_SUCCEEDED.store(true, Ordering::Relaxed);
    if status[0] == 0 {
        let mut len_buf = [0u8; 4];
        w.stdout.read_exact(&mut len_buf).map_err(|_| ParseError {
            message: "Go anvil-json worker short read".to_string(),
            offset: 0,
        })?;
        let n = u32::from_le_bytes(len_buf) as usize;
        let mut wire = vec![0u8; n];
        w.stdout.read_exact(&mut wire).map_err(|_| ParseError {
            message: "Go anvil-json worker payload read failed".to_string(),
            offset: 0,
        })?;
        let mut pos = 0;
        decode_binary_value(&wire, &mut pos)
    } else {
        let mut err_hdr = [0u8; 8];
        if w.stdout.read_exact(&mut err_hdr).is_err() {
            *guard = None;
            return Err(ParseError {
                message: "JSON parse error".to_string(),
                offset: 0,
            });
        }
        let offset = u32::from_le_bytes([err_hdr[0], err_hdr[1], err_hdr[2], err_hdr[3]]) as usize;
        let msg_len = u32::from_le_bytes([err_hdr[4], err_hdr[5], err_hdr[6], err_hdr[7]]) as usize;
        let mut msg_bytes = vec![0u8; msg_len];
        let _ = w.stdout.read_exact(&mut msg_bytes);
        Err(ParseError {
            message: String::from_utf8_lossy(&msg_bytes).into_owned(),
            offset,
        })
    }
}

pub fn stringify(value: &JsonValue) -> String {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return stringify_rust(value);
    }
    let mut wire = Vec::new();
    encode_binary_value(&mut wire, value);
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = spawn_worker().ok();
    }
    let Some(w) = guard.as_mut() else {
        return stringify_rust(value);
    };
    let mut hdr = [0u8; 5];
    hdr[0] = OP_STRINGIFY;
    hdr[1..5].copy_from_slice(&(wire.len() as u32).to_le_bytes());
    if w.stdin.write_all(&hdr).is_err()
        || w.stdin.write_all(&wire).is_err()
        || w.stdin.flush().is_err()
    {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return stringify_rust(value);
    }
    if let Some(s) = read_worker_string(&mut w.stdout) {
        WORKER_EVER_SUCCEEDED.store(true, Ordering::Relaxed);
        s
    } else {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        stringify_rust(value)
    }
}

pub fn stringify_pretty(value: &JsonValue, indent: usize) -> String {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return stringify_pretty_rust(value, indent);
    }
    let mut wire = Vec::new();
    encode_binary_value(&mut wire, value);
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = spawn_worker().ok();
    }
    let Some(w) = guard.as_mut() else {
        return stringify_pretty_rust(value, indent);
    };
    let mut hdr = [0u8; 9];
    hdr[0] = OP_STRINGIFY_PRETTY;
    hdr[1..5].copy_from_slice(&(indent as u32).to_le_bytes());
    hdr[5..9].copy_from_slice(&(wire.len() as u32).to_le_bytes());
    if w.stdin.write_all(&hdr).is_err()
        || w.stdin.write_all(&wire).is_err()
        || w.stdin.flush().is_err()
    {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return stringify_pretty_rust(value, indent);
    }
    if let Some(s) = read_worker_string(&mut w.stdout) {
        WORKER_EVER_SUCCEEDED.store(true, Ordering::Relaxed);
        s
    } else {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        stringify_pretty_rust(value, indent)
    }
}

pub fn escape_string(s: &str) -> String {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return escape_string_rust(s);
    }
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = spawn_worker().ok();
    }
    let Some(w) = guard.as_mut() else {
        return escape_string_rust(s);
    };
    let bytes = s.as_bytes();
    let mut hdr = [0u8; 5];
    hdr[0] = OP_ESCAPE_STRING;
    hdr[1..5].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    if w.stdin.write_all(&hdr).is_err()
        || w.stdin.write_all(bytes).is_err()
        || w.stdin.flush().is_err()
    {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return escape_string_rust(s);
    }
    if let Some(out) = read_worker_string(&mut w.stdout) {
        WORKER_EVER_SUCCEEDED.store(true, Ordering::Relaxed);
        out
    } else {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        escape_string_rust(s)
    }
}

fn read_worker_string(stdout: &mut ChildStdout) -> Option<String> {
    let mut status = [0u8; 1];
    stdout.read_exact(&mut status).ok()?;
    let mut len_buf = [0u8; 4];
    stdout.read_exact(&mut len_buf).ok()?;
    let n = u32::from_le_bytes(len_buf) as usize;
    let mut out = vec![0u8; n];
    stdout.read_exact(&mut out).ok()?;
    if status[0] == 0 {
        String::from_utf8(out).ok()
    } else {
        None
    }
}

pub use escape_string as escape_json_string;
pub use parse as parse_json;
pub use parse_bytes as parse_json_bytes;
pub use stringify as to_json_string;

pub fn parse_json_bounded(
    input: &str,
    max_depth: usize,
    max_bytes: usize,
) -> Result<JsonValue, ParseError> {
    parse_with_limits(input, max_depth, max_bytes)
}

pub fn to_json_pretty(value: &JsonValue) -> String {
    stringify_pretty(value, 2)
}

/// Evaluate an RFC 6901 JSON Pointer (`""` or `"/a/b/0"`) against `value`
/// using the Go `anvil-json` (`v.04`) worker.
pub fn resolve_pointer(value: &JsonValue, pointer: &str) -> Result<Option<JsonValue>, ParseError> {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return resolve_pointer_rust(value, pointer);
    }
    let mut wire = Vec::new();
    encode_binary_value(&mut wire, value);
    let mut guard = worker_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        match spawn_worker() {
            Ok(w) => *guard = Some(w),
            Err(_) => return resolve_pointer_rust(value, pointer),
        }
    }
    let w = guard.as_mut().unwrap();
    let ptr_bytes = pointer.as_bytes();
    let mut hdr = [0u8; 9];
    hdr[0] = OP_RESOLVE_POINTER;
    hdr[1..5].copy_from_slice(&(ptr_bytes.len() as u32).to_le_bytes());
    hdr[5..9].copy_from_slice(&(wire.len() as u32).to_le_bytes());
    if w.stdin.write_all(&hdr).is_err()
        || w.stdin.write_all(ptr_bytes).is_err()
        || w.stdin.write_all(&wire).is_err()
        || w.stdin.flush().is_err()
    {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return resolve_pointer_rust(value, pointer);
    }
    let mut status = [0u8; 1];
    if w.stdout.read_exact(&mut status).is_err() {
        *guard = None;
        if !WORKER_EVER_SUCCEEDED.load(Ordering::Relaxed) {
            WORKER_DISABLED.store(true, Ordering::Relaxed);
        }
        return resolve_pointer_rust(value, pointer);
    }
    WORKER_EVER_SUCCEEDED.store(true, Ordering::Relaxed);
    match status[0] {
        0 => {
            let mut len_buf = [0u8; 4];
            w.stdout.read_exact(&mut len_buf).map_err(|_| wire_err(0))?;
            let n = u32::from_le_bytes(len_buf) as usize;
            let mut out = vec![0u8; n];
            w.stdout.read_exact(&mut out).map_err(|_| wire_err(0))?;
            let mut pos = 0;
            Ok(Some(decode_binary_value(&out, &mut pos)?))
        }
        2 => {
            let mut len_buf = [0u8; 4];
            let _ = w.stdout.read_exact(&mut len_buf);
            Ok(None)
        }
        _ => {
            let mut len_buf = [0u8; 4];
            let _ = w.stdout.read_exact(&mut len_buf);
            let n = u32::from_le_bytes(len_buf) as usize;
            let mut msg_bytes = vec![0u8; n];
            let _ = w.stdout.read_exact(&mut msg_bytes);
            Err(ParseError {
                message: String::from_utf8_lossy(&msg_bytes).into_owned(),
                offset: 0,
            })
        }
    }
}

struct RustJsonParser<'a> {
    src: &'a [u8],
    pos: usize,
    depth: usize,
    max_depth: usize,
}

impl<'a> RustJsonParser<'a> {
    fn err(&self, msg: &str) -> ParseError {
        ParseError {
            message: msg.to_string(),
            offset: self.pos,
        }
    }

    fn err_at(&self, msg: &str, offset: usize) -> ParseError {
        ParseError {
            message: msg.to_string(),
            offset,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn starts_with(&self, kw: &[u8]) -> bool {
        self.src.get(self.pos..self.pos + kw.len()) == Some(kw)
    }

    fn parse_value(&mut self) -> Result<JsonValue, ParseError> {
        self.skip_ws();
        let b = self
            .peek()
            .ok_or_else(|| self.err("unexpected end of JSON input"))?;
        match b {
            b'n' => {
                if self.starts_with(b"null") {
                    self.pos += 4;
                    Ok(JsonValue::Null)
                } else {
                    Err(self.err("expected 'null'"))
                }
            }
            b't' => {
                if self.starts_with(b"true") {
                    self.pos += 4;
                    Ok(JsonValue::Bool(true))
                } else {
                    Err(self.err("expected 'true'"))
                }
            }
            b'f' => {
                if self.starts_with(b"false") {
                    self.pos += 5;
                    Ok(JsonValue::Bool(false))
                } else {
                    Err(self.err("expected 'false'"))
                }
            }
            b'"' => self.parse_string().map(JsonValue::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(self.err("unexpected token")),
        }
    }

    fn read_hex4(&mut self) -> Result<u16, ParseError> {
        if self.pos + 4 > self.src.len() {
            return Err(self.err("truncated \\uXXXX escape"));
        }
        let mut val = 0u16;
        for i in 0..4 {
            let b = self.src[self.pos + i];
            let digit = match b {
                b'0'..=b'9' => u16::from(b - b'0'),
                b'a'..=b'f' => u16::from(b - b'a') + 10,
                b'A'..=b'F' => u16::from(b - b'A') + 10,
                _ => return Err(self.err_at("non-hex digit in \\uXXXX escape", self.pos + i)),
            };
            val = (val << 4) | digit;
        }
        self.pos += 4;
        Ok(val)
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        if self.peek() != Some(b'"') {
            return Err(self.err("expected '\"'"));
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(self.err("unterminated string literal"));
            }
            let b = self.src[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.src.len() {
                    return Err(self.err("unterminated escape sequence"));
                }
                let esc = self.src[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let u = self.read_hex4()?;
                        if (0xD800..=0xDBFF).contains(&u) {
                            if self.src.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                return Err(self.err("unpaired high UTF-16 surrogate"));
                            }
                            self.pos += 2;
                            let low = self.read_hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(self.err("invalid low UTF-16 surrogate"));
                            }
                            let cp = 0x10000
                                + ((u32::from(u) - 0xD800) << 10)
                                + (u32::from(low) - 0xDC00);
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| self.err("invalid supplementary code point"))?;
                            out.push(ch);
                        } else if (0xDC00..=0xDFFF).contains(&u) {
                            return Err(self.err("unpaired low UTF-16 surrogate"));
                        } else if let Some(ch) = char::from_u32(u32::from(u)) {
                            out.push(ch);
                        }
                    }
                    _ => return Err(self.err("invalid string escape")),
                }
                continue;
            }
            if b < 0x20 {
                return Err(self.err("unescaped control character in string"));
            }
            let rest = std::str::from_utf8(&self.src[self.pos..])
                .map_err(|_| self.err("invalid UTF-8 sequence in string"))?;
            let ch = rest
                .chars()
                .next()
                .ok_or_else(|| self.err("unterminated string literal"))?;
            out.push(ch);
            self.pos += ch.len_utf8();
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let b = self
            .peek()
            .ok_or_else(|| self.err("expected digit after '-'"))?;
        if b == b'0' {
            self.pos += 1;
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("leading zeros are not allowed"));
            }
        } else if b.is_ascii_digit() {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        } else {
            return Err(self.err("expected digit"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected digit after decimal point"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("expected digit in exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let slice = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.err_at("invalid number", start))?;
        let n: f64 = slice
            .parse()
            .map_err(|_| self.err_at("number out of f64 range", start))?;
        if !n.is_finite() {
            return Err(self.err_at("number out of f64 range", start));
        }
        Ok(JsonValue::Number(n))
    }

    fn parse_array(&mut self) -> Result<JsonValue, ParseError> {
        if self.depth >= self.max_depth {
            return Err(self.err("nesting exceeds depth cap"));
        }
        self.pos += 1;
        self.depth += 1;
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(JsonValue::Array(Vec::new()));
        }
        let mut items = Vec::new();
        loop {
            let val = match self.parse_value() {
                Ok(v) => v,
                Err(e) => {
                    self.depth -= 1;
                    return Err(e);
                }
            };
            items.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.depth -= 1;
                        return Err(self.err("trailing comma in array"));
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(JsonValue::Array(items));
                }
                None => {
                    self.depth -= 1;
                    return Err(self.err("unterminated array"));
                }
                _ => {
                    self.depth -= 1;
                    return Err(self.err("expected ',' or ']' in array"));
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, ParseError> {
        if self.depth >= self.max_depth {
            return Err(self.err("nesting exceeds depth cap"));
        }
        self.pos += 1;
        self.depth += 1;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(JsonValue::Object(Vec::new()));
        }
        let mut members = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                self.depth -= 1;
                return Err(self.err("expected object key string"));
            }
            let key = match self.parse_string() {
                Ok(k) => k,
                Err(e) => {
                    self.depth -= 1;
                    return Err(e);
                }
            };
            self.skip_ws();
            if self.peek() != Some(b':') {
                self.depth -= 1;
                return Err(self.err("expected ':' after object key"));
            }
            self.pos += 1;
            let val = match self.parse_value() {
                Ok(v) => v,
                Err(e) => {
                    self.depth -= 1;
                    return Err(e);
                }
            };
            members.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.depth -= 1;
                        return Err(self.err("trailing comma in object"));
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(JsonValue::Object(members));
                }
                None => {
                    self.depth -= 1;
                    return Err(self.err("unterminated object"));
                }
                _ => {
                    self.depth -= 1;
                    return Err(self.err("expected ',' or '}' in object"));
                }
            }
        }
    }
}

fn parse_with_limits_rust(
    input: &str,
    max_depth: usize,
    max_bytes: usize,
) -> Result<JsonValue, ParseError> {
    let bytes = input.as_bytes();
    if bytes.len() > max_bytes {
        return Err(ParseError {
            message: "input byte length exceeds cap".to_string(),
            offset: max_bytes,
        });
    }
    let mut p = RustJsonParser {
        src: bytes,
        pos: 0,
        depth: 0,
        max_depth,
    };
    p.skip_ws();
    let val = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.src.len() {
        return Err(p.err("trailing characters after JSON value"));
    }
    Ok(val)
}

fn write_string_rust(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let b = c as u8;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push_str("\\u00");
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0F) as usize] as char);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number_rust(out: &mut String, n: f64) {
    if !n.is_finite() {
        out.push_str("null");
        return;
    }
    if n == 0.0 {
        out.push('0');
        return;
    }
    if n.trunc() == n && n.abs() < 1e15 {
        out.push_str(&(n as i64).to_string());
    } else {
        out.push_str(&n.to_string());
    }
}

fn write_compact_rust(out: &mut String, v: &JsonValue) {
    match v {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => write_number_rust(out, *n),
        JsonValue::String(s) => write_string_rust(out, s),
        JsonValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact_rust(out, item);
            }
            out.push(']');
        }
        JsonValue::Object(members) => {
            out.push('{');
            for (i, (k, val)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string_rust(out, k);
                out.push(':');
                write_compact_rust(out, val);
            }
            out.push('}');
        }
    }
}

fn write_pretty_rust(out: &mut String, v: &JsonValue, indent: usize, level: usize) {
    match v {
        JsonValue::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                for _ in 0..indent.saturating_mul(level + 1) {
                    out.push(' ');
                }
                write_pretty_rust(out, item, indent, level + 1);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            for _ in 0..indent.saturating_mul(level) {
                out.push(' ');
            }
            out.push(']');
        }
        JsonValue::Object(members) => {
            if members.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, val)) in members.iter().enumerate() {
                for _ in 0..indent.saturating_mul(level + 1) {
                    out.push(' ');
                }
                write_string_rust(out, k);
                out.push_str(": ");
                write_pretty_rust(out, val, indent, level + 1);
                if i + 1 < members.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            for _ in 0..indent.saturating_mul(level) {
                out.push(' ');
            }
            out.push('}');
        }
        _ => write_compact_rust(out, v),
    }
}

fn stringify_rust(v: &JsonValue) -> String {
    let mut out = String::new();
    write_compact_rust(&mut out, v);
    out
}

fn stringify_pretty_rust(v: &JsonValue, indent: usize) -> String {
    let mut out = String::new();
    write_pretty_rust(&mut out, v, indent, 0);
    out
}

fn escape_string_rust(s: &str) -> String {
    let mut out = String::new();
    write_string_rust(&mut out, s);
    out
}

fn unescape_pointer_token(token: &str) -> Result<String, ParseError> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => {
                    return Err(ParseError {
                        message: "invalid '~' escape in JSON Pointer".to_string(),
                        offset: 0,
                    });
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn resolve_pointer_rust(value: &JsonValue, pointer: &str) -> Result<Option<JsonValue>, ParseError> {
    if pointer.is_empty() {
        return Ok(Some(value.clone()));
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(ParseError {
            message: "JSON Pointer must be empty or start with '/'".to_string(),
            offset: 0,
        });
    };
    let mut cur = value;
    for raw_token in rest.split('/') {
        let token = unescape_pointer_token(raw_token)?;
        match cur {
            JsonValue::Object(members) => {
                if let Some((_, next)) = members.iter().rev().find(|(k, _)| k == &token) {
                    cur = next;
                } else {
                    return Ok(None);
                }
            }
            JsonValue::Array(items) => {
                if token.len() > 1 && token.starts_with('0') {
                    return Ok(None);
                }
                let Ok(idx) = token.parse::<usize>() else {
                    return Ok(None);
                };
                if let Some(next) = items.get(idx) {
                    cur = next;
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(cur.clone()))
}
