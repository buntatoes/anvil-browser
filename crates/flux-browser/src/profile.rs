//! Local profile file.
//!
//! Bookmarks, history, settings, tabs, and cookies are stored as JSON, then zlib.
//! Windows seals that blob with DPAPI. Linux seals it with AES-256-GCM from libcrypto.
//! The Linux key is a 32-byte file named `profile.key` next to the profile, mode 0600.

use std::fs;
use std::path::PathBuf;

use flux_compress::{compress_zlib, decompress_zlib_bounded};
use flux_json::{JsonValue, parse_json, to_json_pretty};
use flux_net::{CookieJar, HstsStore};

use crate::downloads::DownloadManager;
use crate::{Bookmark, BrowserSettings, SearchEngine, VERSION};

/// Older profile header. Windows still writes this.
pub const PROFILE_MAGIC: &[u8; 5] = b"FLXP\x01";

/// Linux profile header. Payload is AES-256-GCM.
pub const PROFILE_MAGIC_V2: &[u8; 5] = b"FLXP\x02";

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
    pub hsts_store: HstsStore,
    pub downloads: DownloadManager,
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
            hsts_store: HstsStore::new(),
            downloads: DownloadManager::new(),
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
    settings_obj.insert(
        "jit_enabled",
        JsonValue::Bool(snapshot.settings.jit_enabled),
    );
    settings_obj.insert(
        "search_engine",
        JsonValue::String(snapshot.settings.search_engine.slug().to_string()),
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
    if let Ok(hsts_val) = parse_json(&snapshot.hsts_store.to_json()) {
        root.insert("hsts_store", hsts_val);
    }
    root.insert("downloads", snapshot.downloads.to_json_value());

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
        if let Some(v) = settings.get("jit_enabled").and_then(JsonValue::as_bool) {
            snapshot.settings.jit_enabled = v;
        }
        if let Some(eng) = settings.get("search_engine").and_then(JsonValue::as_str) {
            snapshot.settings.search_engine = SearchEngine::from_slug(eng);
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
    if let Some(hsts_val) = doc.get("hsts_store") {
        let hsts_json = to_json_pretty(hsts_val);
        if let Ok(store) = HstsStore::from_json(&hsts_json) {
            snapshot.hsts_store = store;
        }
    }
    if let Some(dl_arr) = doc.get("downloads").and_then(JsonValue::as_array) {
        snapshot.downloads = DownloadManager::from_json_array(dl_arr);
    }

    Ok(snapshot)
}

/// Compress, OS-keystore seal (`DPAPI` on Windows), and MAC-authenticate a [`ProfileSnapshot`]
/// into a binary `FLXP\x01` container.
pub fn encode_encrypted_profile(snapshot: &ProfileSnapshot, nonce: u64) -> Vec<u8> {
    let json_bytes = snapshot_to_json(snapshot).into_bytes();
    let compressed = compress_zlib(&json_bytes);
    #[cfg(not(windows))]
    {
        return seal_linux_profile(&compressed, nonce);
    }
    #[cfg(windows)]
    {
        let sealed = os_keystore_seal(&compressed, nonce);
        let tag = compute_keyed_mac(nonce, &sealed);
        let mut out = Vec::with_capacity(5 + 8 + sealed.len() + 8);
        out.extend_from_slice(PROFILE_MAGIC);
        out.extend_from_slice(&nonce.to_le_bytes());
        out.extend_from_slice(&sealed);
        out.extend_from_slice(&tag.to_le_bytes());
        out
    }
}

/// Verify, unseal (`DPAPI` on Windows), decompress, and parse either a `FLXP\x01` binary container
/// or plain JSON bytes.
pub fn decode_profile_bytes(bytes: &[u8]) -> Result<ProfileSnapshot, ProfileError> {
    if bytes.starts_with(b"{") {
        let text =
            std::str::from_utf8(bytes).map_err(|e| ProfileError::JsonError(e.to_string()))?;
        return snapshot_from_json(text);
    }
    if bytes.len() < 5 + 8 + 8 {
        return Err(ProfileError::TruncatedContainer);
    }
    #[cfg(not(windows))]
    if &bytes[0..5] == PROFILE_MAGIC_V2 {
        return decode_linux_profile(bytes);
    }
    if &bytes[0..5] != PROFILE_MAGIC {
        return Err(ProfileError::InvalidMagic);
    }
    let nonce = u64::from_le_bytes(bytes[5..13].try_into().unwrap());
    let tag_offset = bytes.len() - 8;
    let sealed = &bytes[13..tag_offset];
    let expected_tag = u64::from_le_bytes(bytes[tag_offset..].try_into().unwrap());
    let actual_tag = compute_keyed_mac(nonce, sealed);
    if actual_tag != expected_tag {
        return Err(ProfileError::IntegrityCheckFailed);
    }
    let compressed = os_keystore_unseal(sealed, nonce)?;
    let decompressed = decompress_zlib_bounded(&compressed, MAX_PROFILE_BYTES)
        .map_err(|_| ProfileError::DecompressFailed)?;
    let text =
        std::str::from_utf8(&decompressed).map_err(|e| ProfileError::JsonError(e.to_string()))?;
    snapshot_from_json(text)
}

#[cfg(windows)]
mod win_dpapi {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x01;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    pub fn protect(plain: &[u8], nonce: u64) -> Option<Vec<u8>> {
        if plain.is_empty() {
            return Some(Vec::new());
        }
        let mut entropy_bytes = [0u8; 8];
        entropy_bytes.copy_from_slice(&nonce.to_le_bytes());
        let in_blob = DataBlob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let entropy_blob = DataBlob {
            cb_data: entropy_bytes.len() as u32,
            pb_data: entropy_bytes.as_mut_ptr(),
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };
        unsafe {
            let ok = CryptProtectData(
                &in_blob,
                null(),
                &entropy_blob,
                null_mut(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            );
            if ok == 0 || out_blob.pb_data.is_null() {
                return None;
            }
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let result = slice.to_vec();
            let _ = LocalFree(out_blob.pb_data.cast::<c_void>());
            Some(result)
        }
    }

    pub fn unprotect(cipher: &[u8], nonce: u64) -> Option<Vec<u8>> {
        if cipher.is_empty() {
            return Some(Vec::new());
        }
        let mut entropy_bytes = [0u8; 8];
        entropy_bytes.copy_from_slice(&nonce.to_le_bytes());
        let in_blob = DataBlob {
            cb_data: cipher.len() as u32,
            pb_data: cipher.as_ptr() as *mut u8,
        };
        let entropy_blob = DataBlob {
            cb_data: entropy_bytes.len() as u32,
            pb_data: entropy_bytes.as_mut_ptr(),
        };
        let mut out_blob = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };
        unsafe {
            let ok = CryptUnprotectData(
                &in_blob,
                null_mut(),
                &entropy_blob,
                null_mut(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            );
            if ok == 0 || out_blob.pb_data.is_null() {
                return None;
            }
            let slice = std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize);
            let result = slice.to_vec();
            let _ = LocalFree(out_blob.pb_data.cast::<c_void>());
            Some(result)
        }
    }
}

#[cfg(windows)]
fn os_keystore_seal(compressed: &[u8], nonce: u64) -> Vec<u8> {
    #[cfg(windows)]
    {
        let _ = nonce;
        if let Some(protected) = win_dpapi::protect(compressed, nonce) {
            return protected;
        }
    }
    let _ = nonce;
    compressed.to_vec()
}

fn os_keystore_unseal(sealed: &[u8], nonce: u64) -> Result<Vec<u8>, ProfileError> {
    #[cfg(windows)]
    {
        if let Some(unprotected) = win_dpapi::unprotect(sealed, nonce) {
            return Ok(unprotected);
        }
    }
    let _ = nonce;
    Ok(sealed.to_vec())
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
    #[cfg(not(windows))]
    install_profile_key(path)?;
    let bytes = encode_encrypted_profile(snapshot, nonce);
    fs::write(path, bytes).map_err(|e| ProfileError::IoError(e.to_string()))?;
    #[cfg(not(windows))]
    store_profile_key(path)?;
    Ok(())
}

/// Load a [`ProfileSnapshot`] from `path` (supporting both `FLXP\x01` containers and JSON).
pub fn load_profile_from_path(path: &std::path::Path) -> Result<ProfileSnapshot, ProfileError> {
    #[cfg(not(windows))]
    install_profile_key(path)?;
    let bytes = fs::read(path).map_err(|e| ProfileError::IoError(e.to_string()))?;
    decode_profile_bytes(&bytes)
}

#[cfg(not(windows))]
mod linux_seal {
    use super::ProfileError;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    static KEY: OnceLock<[u8; 32]> = OnceLock::new();

    pub fn install_profile_key(profile_path: &Path) -> Result<(), ProfileError> {
        let path = key_file(profile_path);
        if !path.exists() {
            return Ok(());
        }
        let bytes = fs::read(&path).map_err(|e| ProfileError::IoError(e.to_string()))?;
        if bytes.len() != 32 {
            return Err(ProfileError::IntegrityCheckFailed);
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        let _ = KEY.set(key);
        Ok(())
    }

    pub fn store_profile_key(profile_path: &Path) -> Result<(), ProfileError> {
        let path = key_file(profile_path);
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ProfileError::IoError(e.to_string()))?;
            }
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| ProfileError::IoError(e.to_string()))?;
        file.write_all(key())
            .map_err(|e| ProfileError::IoError(e.to_string()))?;
        Ok(())
    }

    fn key_file(profile_path: &Path) -> PathBuf {
        profile_path.with_file_name("profile.key")
    }

    fn key() -> &'static [u8; 32] {
        KEY.get_or_init(|| {
            let mut fresh = [0u8; 32];
            let mut file = fs::File::open("/dev/urandom").expect("urandom");
            file.read_exact(&mut fresh).expect("urandom");
            fresh
        })
    }

    pub fn seal(compressed: &[u8], nonce: u64) -> Vec<u8> {
        let mut iv = [0u8; 12];
        let mut file = fs::File::open("/dev/urandom").expect("urandom");
        file.read_exact(&mut iv).expect("urandom");
        let aad = nonce.to_le_bytes();
        let (body, tag) = aes_gcm(true, key(), &iv, &aad, compressed, &[0u8; 16]);
        let mut out = Vec::with_capacity(5 + 8 + 12 + body.len() + 16);
        out.extend_from_slice(super::PROFILE_MAGIC_V2);
        out.extend_from_slice(&aad);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&body);
        out.extend_from_slice(&tag);
        out
    }

    pub fn open(bytes: &[u8]) -> Result<Vec<u8>, ProfileError> {
        if bytes.len() < 5 + 8 + 12 + 16 {
            return Err(ProfileError::TruncatedContainer);
        }
        let aad = &bytes[5..13];
        let iv = &bytes[13..25];
        let tag_at = bytes.len() - 16;
        let body = &bytes[25..tag_at];
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&bytes[tag_at..]);
        let (plain, _) = aes_gcm(false, key(), iv, aad, body, &tag);
        if plain.is_empty() && !body.is_empty() {
            return Err(ProfileError::IntegrityCheckFailed);
        }
        Ok(plain)
    }

    fn aes_gcm(
        encrypt: bool,
        key_bytes: &[u8; 32],
        iv: &[u8],
        aad: &[u8],
        input: &[u8],
        tag_in: &[u8; 16],
    ) -> (Vec<u8>, [u8; 16]) {
        use std::ffi::{c_int, c_void};
        use std::ptr::null_mut;
        const SET_IVLEN: c_int = 0x9;
        const GET_TAG: c_int = 0x10;
        const SET_TAG: c_int = 0x11;

        #[link(name = "crypto")]
        unsafe extern "C" {
            fn EVP_CIPHER_CTX_new() -> *mut c_void;
            fn EVP_CIPHER_CTX_free(ctx: *mut c_void);
            fn EVP_aes_256_gcm() -> *const c_void;
            fn EVP_EncryptInit_ex(
                ctx: *mut c_void,
                typ: *const c_void,
                engine: *mut c_void,
                key: *const u8,
                iv: *const u8,
            ) -> c_int;
            fn EVP_DecryptInit_ex(
                ctx: *mut c_void,
                typ: *const c_void,
                engine: *mut c_void,
                key: *const u8,
                iv: *const u8,
            ) -> c_int;
            fn EVP_EncryptUpdate(
                ctx: *mut c_void,
                out: *mut u8,
                outl: *mut c_int,
                input: *const u8,
                inl: c_int,
            ) -> c_int;
            fn EVP_DecryptUpdate(
                ctx: *mut c_void,
                out: *mut u8,
                outl: *mut c_int,
                input: *const u8,
                inl: c_int,
            ) -> c_int;
            fn EVP_EncryptFinal_ex(ctx: *mut c_void, out: *mut u8, outl: *mut c_int) -> c_int;
            fn EVP_DecryptFinal_ex(ctx: *mut c_void, out: *mut u8, outl: *mut c_int) -> c_int;
            fn EVP_CIPHER_CTX_ctrl(ctx: *mut c_void, typ: c_int, arg: c_int, ptr: *mut c_void)
                -> c_int;
        }

        unsafe {
            let ctx = EVP_CIPHER_CTX_new();
            if ctx.is_null() {
                return (Vec::new(), [0u8; 16]);
            }
            let cipher = EVP_aes_256_gcm();
            let started = if encrypt {
                EVP_EncryptInit_ex(ctx, cipher, null_mut(), std::ptr::null(), std::ptr::null())
            } else {
                EVP_DecryptInit_ex(ctx, cipher, null_mut(), std::ptr::null(), std::ptr::null())
            };
            if started != 1
                || EVP_CIPHER_CTX_ctrl(ctx, SET_IVLEN, iv.len() as c_int, null_mut()) != 1
            {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            let keyed = if encrypt {
                EVP_EncryptInit_ex(
                    ctx,
                    std::ptr::null(),
                    null_mut(),
                    key_bytes.as_ptr(),
                    iv.as_ptr(),
                )
            } else {
                EVP_DecryptInit_ex(
                    ctx,
                    std::ptr::null(),
                    null_mut(),
                    key_bytes.as_ptr(),
                    iv.as_ptr(),
                )
            };
            if keyed != 1 {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            let mut ignored = 0;
            let aad_ok = if encrypt {
                EVP_EncryptUpdate(
                    ctx,
                    null_mut(),
                    &mut ignored,
                    aad.as_ptr(),
                    aad.len() as c_int,
                )
            } else {
                EVP_DecryptUpdate(
                    ctx,
                    null_mut(),
                    &mut ignored,
                    aad.as_ptr(),
                    aad.len() as c_int,
                )
            };
            if aad_ok != 1 {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            let mut out = vec![0u8; input.len() + 16];
            let mut out_len = 0;
            let updated = if encrypt {
                EVP_EncryptUpdate(
                    ctx,
                    out.as_mut_ptr(),
                    &mut out_len,
                    input.as_ptr(),
                    input.len() as c_int,
                )
            } else {
                EVP_DecryptUpdate(
                    ctx,
                    out.as_mut_ptr(),
                    &mut out_len,
                    input.as_ptr(),
                    input.len() as c_int,
                )
            };
            if updated != 1 {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            if !encrypt
                && EVP_CIPHER_CTX_ctrl(ctx, SET_TAG, 16, tag_in.as_ptr() as *mut c_void) != 1
            {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            let mut final_len = 0;
            let finished = if encrypt {
                EVP_EncryptFinal_ex(ctx, out.as_mut_ptr().add(out_len as usize), &mut final_len)
            } else {
                EVP_DecryptFinal_ex(ctx, out.as_mut_ptr().add(out_len as usize), &mut final_len)
            };
            if finished != 1 {
                EVP_CIPHER_CTX_free(ctx);
                return (Vec::new(), [0u8; 16]);
            }
            out.truncate(out_len as usize + final_len as usize);
            let mut tag = [0u8; 16];
            if encrypt {
                let _ = EVP_CIPHER_CTX_ctrl(ctx, GET_TAG, 16, tag.as_mut_ptr().cast());
            }
            EVP_CIPHER_CTX_free(ctx);
            (out, tag)
        }
    }
}

#[cfg(not(windows))]
fn install_profile_key(path: &std::path::Path) -> Result<(), ProfileError> {
    linux_seal::install_profile_key(path)
}

#[cfg(not(windows))]
fn store_profile_key(path: &std::path::Path) -> Result<(), ProfileError> {
    linux_seal::store_profile_key(path)
}

#[cfg(not(windows))]
fn seal_linux_profile(compressed: &[u8], nonce: u64) -> Vec<u8> {
    linux_seal::seal(compressed, nonce)
}

#[cfg(not(windows))]
fn decode_linux_profile(bytes: &[u8]) -> Result<ProfileSnapshot, ProfileError> {
    let compressed = linux_seal::open(bytes)?;
    let decompressed = decompress_zlib_bounded(&compressed, MAX_PROFILE_BYTES)
        .map_err(|_| ProfileError::DecompressFailed)?;
    let text =
        std::str::from_utf8(&decompressed).map_err(|e| ProfileError::JsonError(e.to_string()))?;
    snapshot_from_json(text)
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
        let mut hsts = HstsStore::new();
        assert!(hsts.record_hsts_header(
            "https://example.com/app",
            "max-age=63072000; includeSubDomains"
        ));
        let snap = ProfileSnapshot {
            settings: BrowserSettings {
                gpu_video_acceleration: false,
                https_only_mode: true,
                scripts_enabled: false,
                jit_enabled: false,
                search_engine: SearchEngine::DuckDuckGo,
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
            hsts_store: hsts,
            downloads: DownloadManager::new(),
        };

        let mut encrypted = encode_encrypted_profile(&snap, 0x1234_5678_9ABC_DEF0);
        #[cfg(windows)]
        assert_eq!(&encrypted[0..5], PROFILE_MAGIC);
        #[cfg(not(windows))]
        assert_eq!(&encrypted[0..5], PROFILE_MAGIC_V2);
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
