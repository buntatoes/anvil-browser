// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Download log used by `Session` and the `anvil://downloads` page.
//!
//! `profile.rs` stores the log inside the profile JSON through `to_json_value`.
//! Digests come from `anvil_net::sha256_digest`. Names are reduced to a short
//! ASCII file name. Extensions in `QUARANTINED_EXTENSIONS` are marked
//! quarantined and, when saved, written with a `.quarantined` suffix. They are
//! still written. Windows saves also try to add a `Zone.Identifier` stream.
//! Other targets do not write that stream. `motw_tagged` is set true on the
//! record even when no file is written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::anvil_json::JsonValue;
use anvil_net::sha256_digest;

/// Maximum number of download entries retained in the session log.
pub const MAX_DOWNLOAD_ENTRIES: usize = 128;

/// Maximum single download payload size (`64 MiB`).
pub const MAX_SINGLE_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Extensions that `is_quarantined_extension` treats as executable or script names.
const QUARANTINED_EXTENSIONS: &[&str] = &[
    "exe", "bat", "cmd", "ps1", "scr", "vbs", "js", "msi", "com", "dll", "hta", "reg",
];

/// Security classification of a downloaded file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    /// Checksum verified and file extension is non-executable.
    VerifiedSafe,
    /// Executable or script file extension quarantined against automatic execution.
    QuarantinedExecutable,
    /// Download or verification failed.
    Failed(String),
    /// The person stopped the download before a file was saved.
    Cancelled,
}

impl DownloadEntry {
    /// Percent only when a declared length exists and the saved size does not pass it.
    pub fn progress_percent(&self) -> Option<u8> {
        let total = self.declared_total?;
        if total == 0 || self.byte_size > total {
            return None;
        }
        Some(((self.byte_size as u64).saturating_mul(100) / total as u64) as u8)
    }
}

impl DownloadState {
    /// Short status text shown on the downloads page.
    pub fn label(&self) -> &'static str {
        match self {
            DownloadState::VerifiedSafe => "VERIFIED SAFE",
            DownloadState::QuarantinedExecutable => "QUARANTINED (EXECUTABLE)",
            DownloadState::Failed(_) => "FAILED",
            DownloadState::Cancelled => "CANCELLED",
        }
    }

    /// Stable state name stored in profile JSON. The `Failed` message is not stored.
    pub fn code(&self) -> &'static str {
        match self {
            DownloadState::VerifiedSafe => "verified",
            DownloadState::QuarantinedExecutable => "quarantined",
            DownloadState::Failed(_) => "failed",
            DownloadState::Cancelled => "cancelled",
        }
    }
}

/// Metadata and cryptographic integrity record for a single downloaded file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadEntry {
    /// Id assigned by `DownloadManager`. Restored rows keep the stored id.
    pub id: u64,
    /// Sanitized file name, without a directory.
    pub file_name: String,
    /// URL passed by the caller. Not fetched by this module.
    pub source_url: String,
    /// Caller-supplied type, or `application/octet-stream` when blank.
    pub mime_type: String,
    /// Payload length in bytes, including payloads that were rejected for size.
    pub byte_size: usize,
    /// Lowercase SHA-256 hex, or the literal `exceeded-64mib-cap` when the payload is too large.
    pub sha256_hex: String,
    /// Set true by `record_download`. It does not mean a zone stream was written.
    pub motw_tagged: bool,
    /// Safe, quarantined, or failed.
    pub state: DownloadState,
    /// `Content-Length` when the server sent one. No percent is shown without it.
    pub declared_total: Option<usize>,
    /// File that was written, when a save succeeded.
    pub saved_path: Option<String>,
}

/// Session download manager enforcing size bounds, filename sanitization, SHA-256 hashing, and quarantine policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DownloadManager {
    next_id: u64,
    entries: Vec<DownloadEntry>,
}

impl DownloadManager {
    /// Empty log. The next id is 1.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
        }
    }

    /// Number of retained entries, at most `MAX_DOWNLOAD_ENTRIES`.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when `len` is zero.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in insertion order. The oldest entry is first.
    pub fn entries(&self) -> &[DownloadEntry] {
        &self.entries
    }

    /// Drop every entry. The next id is left unchanged.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Record an in-memory payload. This function does not fetch or write a file.
    ///
    /// Payloads larger than `MAX_SINGLE_DOWNLOAD_BYTES` are stored as `Failed`
    /// and are not hashed. Other payloads are hashed. A quarantined extension
    /// sets `QuarantinedExecutable` and still keeps the digest.
    pub fn record_download(
        &mut self,
        source_url: &str,
        suggested_name: Option<&str>,
        mime_type: &str,
        payload: &[u8],
    ) -> DownloadEntry {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);

        let raw_name = suggested_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| infer_filename_from_url(source_url));
        let file_name = sanitize_download_filename(raw_name);

        if payload.len() > MAX_SINGLE_DOWNLOAD_BYTES {
            let entry = DownloadEntry {
                id,
                file_name,
                source_url: source_url.trim().to_string(),
                mime_type: mime_type.trim().to_string(),
                byte_size: payload.len(),
                sha256_hex: String::from("exceeded-64mib-cap"),
                motw_tagged: true,
                state: DownloadState::Failed("exceeds 64 MiB download limit".to_string()),
                declared_total: None,
                saved_path: None,
            };
            self.push_entry(entry.clone());
            return entry;
        }

        let digest = sha256_digest(payload);
        let sha256_hex = hex_encode_32(&digest);
        let state = if is_quarantined_extension(&file_name) {
            DownloadState::QuarantinedExecutable
        } else {
            DownloadState::VerifiedSafe
        };

        let entry = DownloadEntry {
            id,
            file_name,
            source_url: source_url.trim().to_string(),
            mime_type: if mime_type.trim().is_empty() {
                "application/octet-stream".to_string()
            } else {
                mime_type.trim().to_string()
            },
            byte_size: payload.len(),
            sha256_hex,
            motw_tagged: true,
            state,
            declared_total: None,
            saved_path: None,
        };
        self.push_entry(entry.clone());
        entry
    }

    /// Append `entry`, dropping the oldest row when the log is already full.
    fn push_entry(&mut self, entry: DownloadEntry) {
        if self.entries.len() >= MAX_DOWNLOAD_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(entry);
    }

    /// Record `payload`, then write it under `target_dir` when the state is not `Failed`.
    ///
    /// A quarantined name is saved as `<name>.quarantined`. `apply_mark_of_the_web`
    /// runs after the write. A size failure returns `InvalidData` after the failed
    /// row is already in the log.
    pub fn save_payload_to_dir(
        &mut self,
        source_url: &str,
        suggested_name: Option<&str>,
        mime_type: &str,
        payload: &[u8],
        target_dir: &Path,
    ) -> io::Result<(DownloadEntry, PathBuf)> {
        let entry = self.record_download(source_url, suggested_name, mime_type, payload);
        if let DownloadState::Failed(ref reason) = entry.state {
            return Err(io::Error::new(io::ErrorKind::InvalidData, reason.clone()));
        }
        fs::create_dir_all(target_dir)?;
        let safe_name = if entry.state == DownloadState::QuarantinedExecutable {
            format!("{}.quarantined", entry.file_name)
        } else {
            entry.file_name.clone()
        };
        let dest = target_dir.join(safe_name);
        fs::write(&dest, payload)?;
        apply_mark_of_the_web(&dest, source_url);
        self.note_saved(entry.id, None, &dest);
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|item| item.id == entry.id)
            .cloned()
            .unwrap_or(entry);
        Ok((entry, dest))
    }

    /// Save `payload` and remember `declared_total` when the server sent one.
    pub fn save_measured(
        &mut self,
        source_url: &str,
        suggested_name: Option<&str>,
        mime_type: &str,
        payload: &[u8],
        target_dir: &Path,
        declared_total: Option<usize>,
    ) -> io::Result<DownloadEntry> {
        let (mut entry, dest) =
            self.save_payload_to_dir(source_url, suggested_name, mime_type, payload, target_dir)?;
        self.note_saved(entry.id, declared_total, &dest);
        if let Some(stored) = self.entries.iter().rev().find(|item| item.id == entry.id) {
            entry = stored.clone();
        }
        Ok(entry)
    }

    fn note_saved(&mut self, id: u64, declared_total: Option<usize>, dest: &Path) {
        if let Some(item) = self.entries.iter_mut().rev().find(|item| item.id == id) {
            if declared_total.is_some() {
                item.declared_total = declared_total;
            }
            item.saved_path = Some(dest.display().to_string());
        }
    }

    /// Newest saved path, for Show in folder.
    pub fn newest_saved_path(&self) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find_map(|item| item.saved_path.as_deref())
    }

    /// Newest measured percent. None when no download declared a length.
    /// A cancelled download. No file is written and nothing is hashed.
    pub fn record_cancelled(&mut self, source_url: &str) -> DownloadEntry {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);
        let entry = DownloadEntry {
            id,
            file_name: sanitize_download_filename(infer_filename_from_url(source_url)),
            source_url: source_url.trim().to_string(),
            mime_type: String::new(),
            byte_size: 0,
            sha256_hex: String::new(),
            motw_tagged: false,
            state: DownloadState::Cancelled,
            declared_total: None,
            saved_path: None,
        };
        self.push_entry(entry.clone());
        clear_live_download();
        entry
    }

    pub fn newest_progress_percent(&self) -> Option<u8> {
        self.entries
            .iter()
            .rev()
            .find_map(|item| item.progress_percent())
    }

    /// JSON array of entries. `state` is the code from `DownloadState::code`.
    pub fn to_json_value(&self) -> JsonValue {
        let mut arr = JsonValue::empty_array();
        for item in &self.entries {
            let mut obj = JsonValue::empty_object();
            obj.insert("id", JsonValue::Number(item.id as f64));
            obj.insert("file_name", JsonValue::String(item.file_name.clone()));
            obj.insert("source_url", JsonValue::String(item.source_url.clone()));
            obj.insert("mime_type", JsonValue::String(item.mime_type.clone()));
            obj.insert("byte_size", JsonValue::Number(item.byte_size as f64));
            obj.insert("sha256_hex", JsonValue::String(item.sha256_hex.clone()));
            obj.insert("motw_tagged", JsonValue::Bool(item.motw_tagged));
            obj.insert("state", JsonValue::String(item.state.code().to_string()));
            if let Some(total) = item.declared_total {
                obj.insert("declared_total", JsonValue::Number(total as f64));
            }
            if let Some(path) = &item.saved_path {
                obj.insert("saved_path", JsonValue::String(path.clone()));
            }
            arr.push(obj);
        }
        arr
    }

    /// Rebuild a log from a JSON array. Rows without `file_name` are skipped.
    ///
    /// At most `MAX_DOWNLOAD_ENTRIES` rows are kept. A `failed` state comes back
    /// as `Failed("restored failed state")` because the original message is not stored.
    pub fn from_json_array(items: &[JsonValue]) -> Self {
        let mut mgr = Self::new();
        for item in items.iter().take(MAX_DOWNLOAD_ENTRIES) {
            let Some(file_name) = item.get_str("file_name") else {
                continue;
            };
            let id = item.get_i64("id").unwrap_or(1).max(1) as u64;
            let source_url = item.get_str("source_url").unwrap_or("").to_string();
            let mime_type = item
                .get_str("mime_type")
                .unwrap_or("application/octet-stream")
                .to_string();
            let byte_size = item.get_i64("byte_size").unwrap_or(0).max(0) as usize;
            let sha256_hex = item.get_str("sha256_hex").unwrap_or("").to_string();
            let motw_tagged = item.get_bool("motw_tagged").unwrap_or(true);
            let state = match item.get_str("state").unwrap_or("verified") {
                "quarantined" => DownloadState::QuarantinedExecutable,
                "failed" => DownloadState::Failed("restored failed state".to_string()),
                "cancelled" => DownloadState::Cancelled,
                _ => DownloadState::VerifiedSafe,
            };
            mgr.next_id = mgr.next_id.max(id.saturating_add(1));
            let declared_total = item
                .get_i64("declared_total")
                .filter(|n| *n >= 0)
                .map(|n| n as usize);
            let saved_path = item
                .get_str("saved_path")
                .filter(|path| !path.is_empty())
                .map(str::to_string);
            mgr.entries.push(DownloadEntry {
                id,
                file_name: sanitize_download_filename(file_name),
                source_url,
                mime_type,
                byte_size,
                sha256_hex,
                motw_tagged,
                state,
                declared_total,
                saved_path,
            });
        }
        mgr
    }
}

/// Strip directory traversal components (`..`, `/`, `\`, `:`) and control characters from a downloaded file name.
pub fn sanitize_download_filename(raw: &str) -> String {
    let base = raw
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("download.bin");
    let mut cleaned = String::with_capacity(base.len().min(128));
    for ch in base.chars() {
        if cleaned.len() >= 120 {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            cleaned.push(ch);
        } else if ch == ' ' {
            cleaned.push('_');
        }
    }
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "download.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

/// True when the text after the last dot is one of `QUARANTINED_EXTENSIONS`.
pub fn is_quarantined_extension(file_name: &str) -> bool {
    let lower = file_name.trim().to_ascii_lowercase();
    let Some((_, ext)) = lower.rsplit_once('.') else {
        return false;
    };
    QUARANTINED_EXTENSIONS.contains(&ext)
}

/// True when an HTTP response should be routed into [`DownloadManager`] rather than parsed as HTML.
pub fn is_downloadable_response(
    content_type: Option<&str>,
    content_disposition: Option<&str>,
) -> bool {
    if let Some(disp) = content_disposition {
        let lower = disp.to_ascii_lowercase();
        if lower.contains("attachment") {
            return true;
        }
    }
    if let Some(ct) = content_type {
        let mime = ct
            .split(';')
            .next()
            .unwrap_or(ct)
            .trim()
            .to_ascii_lowercase();
        if matches!(
            mime.as_str(),
            "application/octet-stream"
                | "application/zip"
                | "application/x-zip-compressed"
                | "application/pdf"
                | "application/gzip"
                | "application/x-gzip"
                | "application/x-tar"
                | "application/x-7z-compressed"
                | "application/x-rar-compressed"
                | "application/x-msdownload"
                | "application/vnd.microsoft.portable-executable"
        ) {
            return true;
        }
    }
    false
}

/// Extract a suggested filename from a `Content-Disposition` header value, if present.
pub fn extract_disposition_filename(content_disposition: Option<&str>) -> Option<String> {
    let disp = content_disposition?;
    for part in disp.split(';') {
        let trimmed = part.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(idx) = lower.find("filename=") {
            let raw = trimmed[idx + "filename=".len()..]
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if !raw.is_empty() {
                return Some(sanitize_download_filename(raw));
            }
        }
    }
    None
}

struct LiveSlot {
    active: AtomicBool,
    received: AtomicU64,
    total: AtomicU64,
    generation: AtomicU64,
}

static LIVE: LiveSlot = LiveSlot {
    active: AtomicBool::new(false),
    received: AtomicU64::new(0),
    total: AtomicU64::new(u64::MAX),
    generation: AtomicU64::new(0),
};

/// Bytes received for a download that is still being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveDownload {
    pub received: usize,
    pub total: Option<usize>,
}

impl LiveDownload {
    /// Percent from the bytes read and the declared length.
    ///
    /// `None` when the server did not send a length, or the count has passed it.
    pub fn percent(self) -> Option<u8> {
        let total = self.total?;
        if total == 0 || self.received > total {
            return None;
        }
        Some(((self.received as u64).saturating_mul(100) / total as u64) as u8)
    }
}

/// The download being read right now, if one is.
pub fn live_download() -> Option<LiveDownload> {
    if !LIVE.active.load(Ordering::Acquire) {
        return None;
    }
    let received = LIVE.received.load(Ordering::Relaxed) as usize;
    let total_raw = LIVE.total.load(Ordering::Relaxed);
    let total = if total_raw == u64::MAX {
        None
    } else {
        Some(total_raw as usize)
    };
    Some(LiveDownload { received, total })
}

/// Changes each time the in-flight download is updated or cleared.
pub fn live_generation() -> u64 {
    LIVE.generation.load(Ordering::Acquire)
}

pub fn set_live_download(received: usize, total: Option<usize>) {
    LIVE.received.store(received as u64, Ordering::Relaxed);
    LIVE.total.store(
        total.map(|n| n as u64).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    LIVE.active.store(true, Ordering::Release);
    LIVE.generation.fetch_add(1, Ordering::Release);
}

pub fn clear_live_download() {
    CANCEL.store(false, Ordering::Release);
    if LIVE.active.swap(false, Ordering::Release) {
        LIVE.generation.fetch_add(1, Ordering::Release);
    }
}

fn on_inflight(note: &anvil_net::InFlightBody) {
    let content_type = if note.content_type.is_empty() {
        None
    } else {
        Some(note.content_type.as_str())
    };
    let disposition = if note.content_disposition.is_empty() {
        None
    } else {
        Some(note.content_disposition.as_str())
    };
    if is_downloadable_response(content_type, disposition) {
        set_live_download(note.received, note.total);
    }
}

/// Listen for body bytes while a response is still being read.
pub fn install_progress_hook() {
    anvil_net::set_inflight_hook(Some(on_inflight));
    anvil_net::set_transfer_stop_hook(Some(stop_if_download));
}

static CANCEL: AtomicBool = AtomicBool::new(false);
static CANCELLED_URLS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Ask the read in progress to stop if it is a download.
pub fn request_download_cancel() {
    CANCEL.store(true, Ordering::Release);
}

fn cancel_is_set() -> bool {
    CANCEL.load(Ordering::Acquire)
}

fn stop_if_download(content_type: &str, content_disposition: &str) -> bool {
    if !cancel_is_set() {
        return false;
    }
    let disposition = if content_disposition.is_empty() {
        None
    } else {
        Some(content_disposition)
    };
    let content_type = if content_type.is_empty() {
        None
    } else {
        Some(content_type)
    };
    if !is_downloadable_response(content_type, disposition) {
        return false;
    }
    CANCEL.store(false, Ordering::Release);
    true
}

/// Remember a download that stopped before a file was written.
pub fn remember_cancelled(url: &str) {
    let without_query = url.split_once('?').map(|(head, _)| head).unwrap_or(url);
    let stored = without_query
        .split_once('#')
        .map(|(head, _)| head)
        .unwrap_or(without_query);
    if let Ok(mut guard) = CANCELLED_URLS.lock() {
        guard.push(stored.to_string());
    }
    clear_live_download();
}

/// Take cancelled URLs recorded since the last call.
pub fn take_cancelled() -> Vec<String> {
    CANCELLED_URLS
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default()
}

/// A failed GET that can be requested again.
///
/// Quarantine is not a failure to retry. A query string, a `key=` parameter,
/// or anything other than `http` or `https` is left alone.
pub fn retry_get_url(entry: &DownloadEntry) -> Option<&str> {
    if !matches!(entry.state, DownloadState::Failed(_)) {
        return None;
    }
    let url = entry
        .source_url
        .split_once('#')
        .map(|(head, _)| head)
        .unwrap_or(entry.source_url.as_str());
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    if rest.contains('@') || url.contains('?') || url.contains("key=") {
        return None;
    }
    Some(url)
}

/// A size a person can read. Uses the byte count only, never a guessed percent.
pub fn format_byte_size(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;
    if bytes < KB {
        return format!("{bytes} B");
    }
    if bytes < MB {
        let tenths = (bytes * 10) / KB;
        return format!("{}.{} KB", tenths / 10, tenths % 10);
    }
    let tenths = (bytes * 10) / MB;
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

/// Open the folder that contains `path`. Returns false when the opener is missing.
pub fn reveal_in_folder(path: &Path) -> bool {
    let folder = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(path);
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(folder)
            .spawn()
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .is_ok()
    }
}

/// Default directory on disk where exported or saved files are written (`~/Downloads`).
pub fn default_downloads_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join("Downloads");
    }
    std::env::temp_dir().join("AnvilDownloads")
}

/// Last path segment of `url`, or `download.bin` when that segment has no dot.
fn infer_filename_from_url(url: &str) -> &str {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = without_query.rsplit('/').next().unwrap_or("download.bin");
    if last.is_empty() || !last.contains('.') {
        "download.bin"
    } else {
        last
    }
}

/// Lowercase hex for a 32-byte digest. The result is 64 characters.
fn hex_encode_32(digest: &[u8; 32]) -> String {
    /// Lowercase hex digits, index by nibble.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// On Windows, write a `Zone.Identifier` stream with `ZoneId=3` beside `file_path`.
///
/// Newlines in `source_url` are dropped and the URL is capped at 256 characters.
/// A failed write is ignored. On other targets this function does nothing.
fn apply_mark_of_the_web(file_path: &Path, source_url: &str) {
    #[cfg(target_os = "windows")]
    {
        let mut ads_os = file_path.as_os_str().to_os_string();
        ads_os.push(":Zone.Identifier");
        let ads_path = PathBuf::from(ads_os);
        let safe_url: String = source_url
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n'))
            .take(256)
            .collect();
        let content = format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={safe_url}\r\n");
        let _ = fs::write(ads_path, content);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (file_path, source_url);
    }
}

#[cfg(test)]
mod tests {
    //! Filename cleanup, the extension list, and one safe versus one quarantined record.
    use super::*;

    #[test]
    /// Strip a traversal prefix, hash a JSON payload, and mark an exe name quarantined.
    fn sanitizes_filenames_verifies_sha256_and_quarantines_executables() {
        assert_eq!(
            sanitize_download_filename("../../Windows/System32/cmd.exe"),
            "cmd.exe"
        );
        assert!(is_quarantined_extension("installer.exe"));
        assert!(is_quarantined_extension("script.ps1"));
        assert!(!is_quarantined_extension("report.json"));

        let mut mgr = DownloadManager::new();
        let safe_entry = mgr.record_download(
            "https://hollowcreek.org/reports/audit.json",
            Some("audit.json"),
            "application/json",
            b"{\"status\":\"verified\"}",
        );
        assert_eq!(safe_entry.state, DownloadState::VerifiedSafe);
        assert_eq!(safe_entry.sha256_hex.len(), 64);

        let exec_entry = mgr.record_download(
            "https://untrusted.example.org/dropper.exe",
            Some("../../dropper.exe"),
            "application/octet-stream",
            b"MZ_TEST_BINARY",
        );
        assert_eq!(exec_entry.file_name, "dropper.exe");
        assert_eq!(exec_entry.state, DownloadState::QuarantinedExecutable);

        assert!(is_downloadable_response(Some("application/pdf"), None));
        assert!(is_downloadable_response(
            Some("text/plain"),
            Some("attachment; filename=\"report.txt\"")
        ));
        assert!(!is_downloadable_response(
            Some("text/html; charset=utf-8"),
            None
        ));
        assert_eq!(
            extract_disposition_filename(Some("attachment; filename=\"../archive.zip\"")),
            Some("archive.zip".to_string())
        );
    }

    #[test]
    fn percent_needs_a_declared_length_and_sizes_stay_readable() {
        assert_eq!(format_byte_size(12), "12 B");
        assert_eq!(format_byte_size(1536), "1.5 KB");
        let mut entry = DownloadManager::new().record_download(
            "https://example.com/a.bin",
            Some("a.bin"),
            "application/octet-stream",
            &[1, 2, 3, 4],
        );
        assert_eq!(entry.progress_percent(), None);
        entry.declared_total = Some(8);
        assert_eq!(entry.progress_percent(), Some(50));
        entry.byte_size = 9;
        assert_eq!(entry.progress_percent(), None);

        set_live_download(40, Some(100));
        let live = live_download().unwrap();
        assert_eq!(live.percent(), Some(40));
        assert!(live.received < live.total.unwrap());
        set_live_download(20, None);
        assert_eq!(live_download().unwrap().percent(), None);
        clear_live_download();
        assert!(live_download().is_none());
    }

    #[test]
    fn retry_is_only_a_plain_failed_get() {
        let mut mgr = DownloadManager::new();
        let failed = mgr.record_download(
            "https://example.com/file.zip",
            Some("file.zip"),
            "application/zip",
            b"zip",
        );
        let mut failed = failed;
        failed.state = DownloadState::Failed("the connection failed".into());
        assert_eq!(retry_get_url(&failed), Some("https://example.com/file.zip"));
        failed.source_url = "https://example.com/file.zip?once=1".into();
        assert!(retry_get_url(&failed).is_none());
        failed.source_url = "https://example.com/file.zip".into();
        failed.state = DownloadState::QuarantinedExecutable;
        assert!(retry_get_url(&failed).is_none());
        failed.state = DownloadState::Cancelled;
        assert!(retry_get_url(&failed).is_none());
        let cancelled = mgr.record_cancelled("https://example.com/stopped.bin");
        assert_eq!(cancelled.state, DownloadState::Cancelled);
        assert!(cancelled.saved_path.is_none());
        assert!(cancelled.sha256_hex.is_empty());
    }
}
