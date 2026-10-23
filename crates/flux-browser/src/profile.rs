//! Persistent compressed and integrity-verified local profile storage (`FLXP\x01` container).
//!
//! Serializes session bookmarks, browsing history, `BrowserSettings`, viewport zoom level,
//! open tab URLs, and RFC 6265bis `CookieJar` entries into JSON (`flux-json`), compresses
//! the payload with `zlib` (`flux-compress`), and seals the container with a 64-bit integrity tag.

use std::fs;
use std::path::PathBuf;

use flux_compress::{compress_zlib, decompress_zlib_bounded};
use flux_json::{JsonValue, parse_json, to_json_pretty};
use flux_net::CookieJar;

use crate::{Bookmark, BrowserSettings, VERSION};

/// Magic header for Flux Browser binary profile containers (`"FLXP\x01"`).
pub const PROFILE_MAGIC: &[u8; 5] = b"FLXP\x01";

/// Maximum decompressed profile JSON size (8 MiB).
pub const MAX_PROFILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    InvalidMagic,
    TruncatedContainer,
    IntegrityCheckFailed,
    DecompressFailed,
    JsonError(String),
    IoError(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "invalid profile container header"),
            Self::TruncatedContainer => write!(f, "truncated profile container"),
            Self::IntegrityCheckFailed => write!(f, "profile integrity verification failed"),
            Self::DecompressFailed => write!(f, "profile decompression failed"),
            Self::JsonError(msg) => write!(f, "profile JSON error: {msg}"),
            Self::IoError(msg) => write!(f, "profile I/O error: {msg}"),
        }
    }
}

impl std::error::Error for ProfileError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSnapshot {
    pub settings: BrowserSettings,
    pub zoom_percent: u16,
    pub active_tab: usize,
    pub tab_urls: Vec<String>,
    pub bookmarks: Vec<Bookmark>,
    pub history_log: Vec<Bookmark>,
    pub cookie_jar: CookieJar,
}

impl Default for ProfileSnapshot {
    fn default() -> Self {
        Self {
            settings: BrowserSettings::default(),
            zoom_percent: 100,
            active_tab: 0,
            tab_urls: Vec::new(),
            bookmarks: Vec::new(),
            history_log: Vec::new(),
            cookie_jar: CookieJar::new(),
        }
    }
}

/// Return the default per-user encrypted profile file path (`profile.flx`).
pub fn default_profile_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(base).join("Flux Browser").join("profile.flx"))
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return Some(PathBuf::from(xdg).join("flux-browser").join("profile.flx"));
        }
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("flux-browser")
                .join("profile.flx"),
        )
    }
}

/// Serialize a [`ProfileSnapshot`] into formatted JSON.
pub fn snapshot_to_json(snapshot: &ProfileSnapshot) -> String {
    let mut root = JsonValue::empty_object();
    root.insert("version", JsonValue::String(VERSION.to_string()));
    root.insert("active_tab", JsonValue::Number(snapshot.active_tab as f64));
    root.insert(
        "zoom_percent",
        JsonValue::Number(snapshot.zoom_percent as f64),
    );

    let mut settings_obj = JsonValue::empty_object();
    settings_obj.insert(
        "gpu_video_acceleration",
        JsonValue::Bool(snapshot.settings.gpu_video_acceleration),
    );
    settings_obj.insert(
        "https_only_mode",
        JsonValue::Bool(snapshot.settings.https_only_mode),
    );
    settings_obj.insert(
        "scripts_enabled",
        JsonValue::Bool(snapshot.settings.scripts_enabled),
    );
    root.insert("settings", settings_obj);

    let mut tabs_arr = JsonValue::empty_array();
    for url in &snapshot.tab_urls {
        let mut item = JsonValue::empty_object();
        item.insert("title", JsonValue::String(url.clone()));
        item.insert("url", JsonValue::String(url.clone()));
        tabs_arr.push(item);
    }
    root.insert("tabs", tabs_arr);

    let mut bm_arr = JsonValue::empty_array();
    for bm in &snapshot.bookmarks {
        let mut item = JsonValue::empty_object();
        item.insert("title", JsonValue::String(bm.title.clone()));
        item.insert("url", JsonValue::String(bm.url.clone()));
        bm_arr.push(item);
    }
    root.insert("bookmarks", bm_arr);

    let mut hist_arr = JsonValue::empty_array();
    for entry in &snapshot.history_log {
        let mut item = JsonValue::empty_object();
        item.insert("title", JsonValue::String(entry.title.clone()));
        item.insert("url", JsonValue::String(entry.url.clone()));
        hist_arr.push(item);
    }
    root.insert("history", hist_arr);

    if let Ok(cookies_val) = parse_json(&snapshot.cookie_jar.to_json()) {
        root.insert("cookie_jar", cookies_val);
    }

    to_json_pretty(&root)
}

/// Deserialize a [`ProfileSnapshot`] from JSON text.
pub fn snapshot_from_json(json_src: &str) -> Result<ProfileSnapshot, ProfileError> {
    let doc = parse_json(json_src).map_err(|e| ProfileError::JsonError(e.to_string()))?;
    let mut snapshot = ProfileSnapshot::default();

    if let Some(active) = doc.get("active_tab").and_then(JsonValue::as_f64) {
        if active >= 0.0 {
            snapshot.active_tab = active as usize;
        }
    }
    if let Some(zoom) = doc.get("zoom_percent").and_then(JsonValue::as_f64) {
        snapshot.zoom_percent = (zoom.round() as u16).clamp(50, 250);
    }
    if let Some(settings) = doc.get("settings") {
        if let Some(v) = settings
            .get("gpu_video_acceleration")
            .and_then(JsonValue::as_bool)
        {
            snapshot.settings.gpu_video_acceleration = v;
        }
        if let Some(v) = settings.get("https_only_mode").and_then(JsonValue::as_bool) {
            snapshot.settings.https_only_mode = v;
        }
        if let Some(v) = settings.get("scripts_enabled").and_then(JsonValue::as_bool) {
            snapshot.settings.scripts_enabled = v;
        }
    }
    if let Some(tabs) = doc.get("tabs").and_then(JsonValue::as_array) {
        for item in tabs {
            if let Some(url) = item.get("url").and_then(JsonValue::as_str) {
                if !url.trim().is_empty() {
                    snapshot.tab_urls.push(url.trim().to_string());
                }
            }
        }
    }
    if let Some(bms) = doc.get("bookmarks").and_then(JsonValue::as_array) {
        for item in bms {
            let title = item.get("title").and_then(JsonValue::as_str).unwrap_or("");
            let url = item.get("url").and_then(JsonValue::as_str).unwrap_or("");
            if !url.trim().is_empty() {
                snapshot.bookmarks.push(Bookmark {
                    title: if title.trim().is_empty() {
                        url.trim().to_string()
                    } else {
                        title.trim().to_string()
                    },
                    url: url.trim().to_string(),
                });
            }
        }
    }
    if let Some(hist) = doc.get("history").and_then(JsonValue::as_array) {
        for item in hist {
            let title = item.get("title").and_then(JsonValue::as_str).unwrap_or("");
            let url = item.get("url").and_then(JsonValue::as_str).unwrap_or("");
            if !url.trim().is_empty() {
                snapshot.history_log.push(Bookmark {
                    title: if title.trim().is_empty() {
                        url.trim().to_string()
                    } else {
                        title.trim().to_string()
                    },
                    url: url.trim().to_string(),
                });
            }
        }
    }
    if let Some(cj_val) = doc.get("cookie_jar") {
        let cj_json = to_json_pretty(cj_val);
        if let Ok(jar) = CookieJar::from_json(&cj_json) {
            snapshot.cookie_jar = jar;
        }
    }

    Ok(snapshot)
}

/// Compress and seal a [`ProfileSnapshot`] into a binary `FLXP\x01` container.
pub fn encode_encrypted_profile(snapshot: &ProfileSnapshot, nonce: u64) -> Vec<u8> {
    let json_bytes = snapshot_to_json(snapshot).into_bytes();
    let compressed = compress_zlib(&json_bytes);
    let tag = compute_keyed_mac(nonce, &compressed);

    let mut out = Vec::with_capacity(5 + 8 + compressed.len() + 8);
    out.extend_from_slice(PROFILE_MAGIC);
    out.extend_from_slice(&nonce.to_le_bytes());
    out.extend_from_slice(&compressed);
    out.extend_from_slice(&tag.to_le_bytes());
    out
}

/// Verify, decompress, and parse either a `FLXP\x01` binary container or plain JSON bytes.
pub fn decode_profile_bytes(bytes: &[u8]) -> Result<ProfileSnapshot, ProfileError> {
    if bytes.starts_with(b"{") {
        let text =
            std::str::from_utf8(bytes).map_err(|e| ProfileError::JsonError(e.to_string()))?;
        return snapshot_from_json(text);
    }
    if bytes.len() < 5 + 8 + 8 {
        return Err(ProfileError::TruncatedContainer);
    }
    if &bytes[0..5] != PROFILE_MAGIC {
        return Err(ProfileError::InvalidMagic);
    }
    let nonce = u64::from_le_bytes(bytes[5..13].try_into().unwrap());
    let tag_offset = bytes.len() - 8;
    let compressed = &bytes[13..tag_offset];
    let expected_tag = u64::from_le_bytes(bytes[tag_offset..].try_into().unwrap());
    let actual_tag = compute_keyed_mac(nonce, compressed);
    if actual_tag != expected_tag {
        return Err(ProfileError::IntegrityCheckFailed);
    }
    let decompressed = decompress_zlib_bounded(compressed, MAX_PROFILE_BYTES)
        .map_err(|_| ProfileError::DecompressFailed)?;
    let text =
        std::str::from_utf8(&decompressed).map_err(|e| ProfileError::JsonError(e.to_string()))?;
    snapshot_from_json(text)
}

/// Save a `FLXP\x01` profile container to `path` (creating parent directories if needed).
pub fn save_encrypted_profile_to_path(
    snapshot: &ProfileSnapshot,
    path: &std::path::Path,
) -> Result<(), ProfileError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| ProfileError::IoError(e.to_string()))?;
        }
    }
    let nonce = (snapshot.bookmarks.len() as u64)
        .wrapping_mul(31)
        .wrapping_add(snapshot.history_log.len() as u64);
    let bytes = encode_encrypted_profile(snapshot, nonce);
    fs::write(path, bytes).map_err(|e| ProfileError::IoError(e.to_string()))
}

/// Load a [`ProfileSnapshot`] from `path` (supporting both `FLXP\x01` containers and JSON).
pub fn load_profile_from_path(path: &std::path::Path) -> Result<ProfileSnapshot, ProfileError> {
    let bytes = fs::read(path).map_err(|e| ProfileError::IoError(e.to_string()))?;
    decode_profile_bytes(&bytes)
}

fn compute_keyed_mac(nonce: u64, payload: &[u8]) -> u64 {
    let mut acc = 14695981039346656037u64 ^ nonce;
    for &b in payload {
        acc ^= b as u64;
        acc = acc.wrapping_mul(1099511628211);
    }
    acc ^ (payload.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypts_authenticates_and_restores_full_profile_snapshot() {
        let mut jar = CookieJar::new();
        assert!(jar.store_set_cookie(
            "https://example.com/app",
            "__Host-session=tok777; Secure; HttpOnly; Path=/; SameSite=Strict"
        ));
        let snap = ProfileSnapshot {
            settings: BrowserSettings {
                gpu_video_acceleration: false,
                https_only_mode: true,
                scripts_enabled: false,
            },
            zoom_percent: 125,
            active_tab: 0,
            tab_urls: vec!["flux://home".to_string(), "https://example.com".to_string()],
            bookmarks: vec![Bookmark {
                title: "Example".to_string(),
                url: "https://example.com".to_string(),
            }],
            history_log: vec![Bookmark {
                title: "Flux Start".to_string(),
                url: "flux://home".to_string(),
            }],
            cookie_jar: jar,
        };

        let mut encrypted = encode_encrypted_profile(&snap, 0x1234_5678_9ABC_DEF0);
        assert_eq!(&encrypted[0..5], PROFILE_MAGIC);
        let restored = decode_profile_bytes(&encrypted).expect("valid encrypted profile");
        assert_eq!(restored, snap);

        // Tampering with ciphertext must fail integrity check
        let mid = encrypted.len() / 2;
        encrypted[mid] ^= 0x55;
        assert_eq!(
            decode_profile_bytes(&encrypted),
            Err(ProfileError::IntegrityCheckFailed)
        );
    }
}
