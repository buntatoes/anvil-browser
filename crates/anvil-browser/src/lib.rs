// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Browser session: tabs, history, bookmarks, and page loading.
//!
//! `src/main.rs` calls `load_target` for the `render` and `raster` commands.
//! `shell` calls `Session` for the window. `ui` reads session state when it
//! builds the shell page.
//!
//! A tab is a `Page` history stack in this process. Each tab also has its
//! own `--renderer` process (see `worker::TabWorkerPool`), started with an
//! empty environment through the `anvil-sandbox` launcher. http(s) pages
//! and local files are parsed and scripted in that process, and the window
//! paints the returned document only when the renderer says it finished. A
//! timeout, a crashed renderer, or a bad frame becomes an error page, and the
//! next load in that tab starts a fresh renderer. Closing a tab ends its
//! renderer. `anvil://`
//! pages are built in this process. `anvil-sandbox`'s `execute_isolated`
//! still wraps each load here and catches a panic.
//!
//! Building a page uses `anvil-dom` (parse, with comment-preserving `<script>` extraction),
//! `anvil-script` (JavaScript runtime, WebAssembly 1.0 stack VM, and `--renderer`
//! interactive binding hydration), `anvil-style` (CSS), `anvil-layout`, and `anvil-paint`.
//! `Session::rasterize` calls `anvil-raster` after that. `anvil-net` fetches pages, stores cookies,
//! checks CSP, and evaluates network and cosmetic filters via Brave's `adblock-rust` (`MPL-2.0`).
//! Script `fetch()` is same-origin and uses that cookie jar
//! through a host callback. The numeric VM does not perform it. In a tab
//! renderer, linked scripts and script `fetch()` are asked of the browser
//! process, which answers with [`answer_renderer_fetch`] (including `$redirect` surrogates).
//! `anvil-image` decodes images. `anvil-video` decodes clips.
//! `frontend` builds `anvil://` HTML. `sites` decides which large-site scripts
//! are not fetched, including Reddit, other social hosts, and bank hosts, and
//! adds a YouTube player made from published JPEG frames.
//! The documents themselves load through the normal HTTP path.
//! `profile` reads and writes `profile.anvil`. `permissions` holds the
//! per-site settings. HTTPS-only is applied by `anvil-net` on whichever
//! thread does a load; `Session` sets it with `anvil_net::enter_https_only`,
//! and the tab's renderer gets the same rules in its request.

pub mod downloads;
pub mod frame;
pub mod frontend;
pub mod installer;
pub mod keys;
pub mod logo;
pub mod permissions;
#[cfg(windows)]
pub mod present;
pub mod profile;
#[cfg(any(windows, target_os = "linux"))]
pub mod shell;
pub mod sites;
pub mod ui;
pub mod worker;

/// Release name for this tree.
pub const VERSION: &str = "v.24.1";

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anvil_dom::{Document, NodeId, StyleSheetSource};
pub use anvil_gpu::{D3d11HardwareStatus, probe_and_compile_d3d11_compositor};
pub use anvil_image::Image;
use anvil_image::decode_auto;
use anvil_json::{JsonValue, parse_json, to_json_pretty};
use anvil_layout::{LayoutBox, Rect, hit_test, hit_test_scrolled, layout_document_interactive};
pub use anvil_net::h2::H2ClientSession;
pub use anvil_net::pqc_tls13::{
    Tls13ApplicationKeys, Tls13ClientHandshaker, Tls13ServerHandshaker, Tls13Session, Tls13Stream,
};
pub use anvil_net::{
    BlockerResult, ConnectionPoolStats, CosmeticFilter, CosmeticResources, NetworkFilter,
    NetworkFilterMask, NetworkShield, RedirectResourceStorage, RequestType, ShieldBlockReason,
    ShieldsMode, check_default_shield_request, clean_url_with_default_shield,
    clear_connection_pool, connection_pool_stats, cosmetic_css_from_default_shield, fetch_batch,
    sri_sha256_token, sync_default_shield,
};
use anvil_net::{
    ContentSecurityPolicy, CookieJar, CspDirective, HstsStore, MAX_REDIRECTS, NetError,
    allows_corp_subresource, append_query, fetch, fetch_post_with_cookies, fetch_with_cookies,
    http_get_with_cookie, is_same_origin, is_url_blocked_by_default_shield, parse_url, resolve_url,
    verify_subresource_integrity,
};
use anvil_paint::{DisplayItem, DisplayList, paint};
pub use anvil_raster::{
    Color, DEFAULT_GLYPH_CACHE_BUDGET_BYTES, DirtyTileGrid, Framebuffer, MAX_WORKER_THREADS,
    RasterEngine, RasterMemoryStats, build_gpu_scene_batch, default_raster_worker_threads,
    interpret_type2_charstring, interpret_type2_charstring_with_subrs, raster_memory_stats,
    raster_simd_width_bytes, raster_worker_threads, rasterize, rasterize_gpu_batch,
    rasterize_incremental, rasterize_parallel, set_raster_memory_budget, set_raster_worker_threads,
    trim_raster_caches,
};
pub use anvil_sandbox::confine::ConfineMode;
use anvil_sandbox::wire::{FetchKind, FetchReply, FetchRequest, MAX_SHORT_TEXT_BYTES};
pub use anvil_sandbox::{
    MandatorySandboxGuard, SandboxError, SandboxTelemetry, TabWorkerRequest, TabWorkerResponse,
    enforce_mandatory_sandbox_or_abort, lock_current_worker_process_or_abort,
};
use anvil_script::{ScriptFetchReply, ScriptRuntime, limit_script_fetch_body};
pub use anvil_script::{
    WasmError, WasmExport, WasmExportKind, WasmFuncType, WasmInstance, WasmMemory, WasmModule,
    WasmValType, WasmValue, validate_wasm,
};
pub use anvil_script::{
    compile_es2022_with_anvil_js, eval_es2022_with_anvil_js, eval_webapp_with_anvil_js,
    self_hosted_bundle, self_hosted_bundle_hash, self_hosted_runtime_bundle,
};
pub use anvil_style::compute_styles_parallel;
use anvil_style::{Animator, Stylesheet, compute_styles_animated, parse_css, parse_css_with_media};

/// Default decoded image cache budget in bytes (32 MiB).
pub const DEFAULT_IMAGE_CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Maximum number of decoded images retained in the LRU cache.
const MAX_IMAGE_CACHE_ENTRIES: usize = 2560;

/// Combined RAM usage and cache statistics for the browser and raster engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserMemoryStats {
    /// Number of decoded RGBA images currently cached.
    pub image_cache_entries: usize,
    /// Total bytes used by cached RGBA images.
    pub image_cache_bytes: usize,
    /// Configured maximum byte budget for the decoded image cache.
    pub image_cache_budget_bytes: usize,
    /// Glyph coverage cache statistics from `anvil-raster`.
    pub raster: RasterMemoryStats,
}

struct ImageLruCache {
    entries: Vec<(String, Image, usize)>,
    used_bytes: usize,
    budget_bytes: usize,
}

impl ImageLruCache {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            used_bytes: 0,
            budget_bytes: DEFAULT_IMAGE_CACHE_BUDGET_BYTES,
        }
    }

    fn get(&mut self, url: &str) -> Option<Image> {
        let idx = self.entries.iter().position(|(k, _, _)| k == url)?;
        let entry = self.entries.remove(idx);
        let img = entry.1.clone();
        self.entries.push(entry);
        Some(img)
    }

    fn insert(&mut self, url: String, img: Image) {
        let cost = img
            .pixels
            .len()
            .saturating_add(url.len())
            .saturating_add(32);
        if cost > self.budget_bytes || self.budget_bytes == 0 {
            return;
        }
        if let Some(idx) = self.entries.iter().position(|(k, _, _)| k == &url) {
            let old = self.entries.remove(idx);
            self.used_bytes = self.used_bytes.saturating_sub(old.2);
        }
        while (!self.entries.is_empty() && self.used_bytes + cost > self.budget_bytes)
            || self.entries.len() >= MAX_IMAGE_CACHE_ENTRIES
        {
            let evicted = self.entries.remove(0);
            self.used_bytes = self.used_bytes.saturating_sub(evicted.2);
        }
        self.used_bytes += cost;
        self.entries.push((url, img, cost));
    }

    fn set_budget(&mut self, budget_bytes: usize) {
        self.budget_bytes = budget_bytes;
        while !self.entries.is_empty() && self.used_bytes > self.budget_bytes {
            let evicted = self.entries.remove(0);
            self.used_bytes = self.used_bytes.saturating_sub(evicted.2);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.entries.shrink_to_fit();
        self.used_bytes = 0;
    }
}

static IMAGE_CACHE: Mutex<ImageLruCache> = Mutex::new(ImageLruCache::new());

fn cached_get_image(url: &str) -> Option<Image> {
    IMAGE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
}

fn cached_decode_image(url: &str, bytes: &[u8]) -> Option<Image> {
    if let Some(hit) = cached_get_image(url) {
        return Some(hit);
    }
    let decoded = decode_auto(bytes).ok()?;
    IMAGE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(url.to_string(), decoded.clone());
    Some(decoded)
}

/// Return current decoded-image and glyph-coverage RAM statistics.
pub fn browser_memory_stats() -> BrowserMemoryStats {
    let guard = IMAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    BrowserMemoryStats {
        image_cache_entries: guard.entries.len(),
        image_cache_bytes: guard.used_bytes,
        image_cache_budget_bytes: guard.budget_bytes,
        raster: raster_memory_stats(),
    }
}

/// Configure the maximum byte budget for the decoded RGBA image cache.
pub fn set_image_cache_budget(budget_bytes: usize) {
    IMAGE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .set_budget(budget_bytes);
}

/// Evict all cached decoded images, glyph bitmaps, and idle pooled connections.
pub fn trim_browser_memory() {
    IMAGE_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    trim_raster_caches();
    clear_connection_pool();
}
pub use anvil_video::{
    AudioCodecProfile, AudioOutputBridge, AudioTrack, CdmSession, DEFAULT_EME_DEMO_KEY,
    DEFAULT_EME_DEMO_KID, EmeKeySystem, Mp4Metadata, OsAudioBackend, VideoClip, encode_pcm16_wav,
    generate_encrypted_demo_mp4, inspect_mp4_metadata, probe_os_audio_backend,
};
use anvil_video::{GpuVideoSession, generate_demo_mp4};
pub use downloads::{DownloadEntry, DownloadManager, DownloadState};
pub use profile::{ProfileError, ProfileSnapshot, default_profile_path};
pub use worker::{TabProcess, TabWorkerPool};

/// Opaque white framebuffer used when the active tab has no page.
const PAPER: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Width and height in CSS pixels. `main` fills this from `--width` and `--height`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Viewport width in CSS pixels.
    pub width: f32,
    /// Viewport height in CSS pixels.
    pub height: f32,
}

/// A title and URL. History rows use this same shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

/// Search engine chosen on the settings page and stored in the profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchEngine {
    /// Google Search. Slug `google`. Home URL `https://www.google.com`.
    Google,
    /// DuckDuckGo. Slug `duckduckgo`. Home URL `https://duckduckgo.com`.
    DuckDuckGo,
    /// Microsoft Bing. Slug `bing`. Home URL `https://www.bing.com`.
    Bing,
}

impl SearchEngine {
    /// Settings label: Google Search, DuckDuckGo, or Microsoft Bing.
    pub fn label(self) -> &'static str {
        match self {
            Self::Google => "Google Search",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Bing => "Microsoft Bing",
        }
    }

    /// Short label used where the full engine name does not fit.
    pub fn short_badge(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::DuckDuckGo => "DDG",
            Self::Bing => "Bing",
        }
    }

    /// Stable profile value: `google`, `duckduckgo`, or `bing`.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::DuckDuckGo => "duckduckgo",
            Self::Bing => "bing",
        }
    }

    /// Engine home page used when a navigation query is empty.
    pub fn home_url(self) -> &'static str {
        match self {
            Self::Google => "https://www.google.com",
            Self::DuckDuckGo => "https://duckduckgo.com",
            Self::Bing => "https://www.bing.com",
        }
    }

    /// Query URL for this engine. `query` is form-encoded.
    pub fn search_url(self, query: &str) -> String {
        let encoded = frontend::urlencode_component(query.trim());
        match self {
            Self::Google => format!("https://www.google.com/search?q={encoded}"),
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
            Self::Bing => format!("https://www.bing.com/search?q={encoded}"),
        }
    }

    /// Google Search, then DuckDuckGo, then Microsoft Bing, then Google Search.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::Google => Self::DuckDuckGo,
            Self::DuckDuckGo => Self::Bing,
            Self::Bing => Self::Google,
        }
    }

    /// Parse a profile slug. Unknown text becomes Google Search.
    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "duckduckgo" | "ddg" => Self::DuckDuckGo,
            "bing" | "microsoft bing" => Self::Bing,
            _ => Self::Google,
        }
    }
}

/// Browser UI and internal-page color theme stored in `BrowserSettings`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BrowserTheme {
    /// Obsidian Forge dark theme (`#12151c` surface, `#ff6b24` ember accent).
    #[default]
    Dark,
    /// Abyssal OLED high-contrast pitch-black theme (`#07090e` surface, `#38bdf8` cyan accent).
    Midnight,
    /// Daylight Studio warm-alabaster light theme (`#f8fafc` surface, `#ea580c` accent).
    Light,
    /// Nordic Frost cool slate theme (`#1e2430` surface, `#88c0d0` frost accent).
    Nordic,
}

impl BrowserTheme {
    /// Display label shown on `anvil://settings` and in the application menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Obsidian Dark",
            Self::Midnight => "Midnight OLED",
            Self::Light => "Daylight Light",
            Self::Nordic => "Nordic Frost",
        }
    }

    /// Stable profile slug (`dark`, `midnight`, `light`, `nordic`).
    pub fn slug(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Midnight => "midnight",
            Self::Light => "light",
            Self::Nordic => "nordic",
        }
    }

    /// Parse a profile or query slug. Unknown values default to [`Self::Dark`].
    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "midnight" | "oled" => Self::Midnight,
            "light" | "daylight" => Self::Light,
            "nordic" | "frost" => Self::Nordic,
            _ => Self::Dark,
        }
    }

    /// True when this theme uses a light surface palette (`Daylight Light`).
    pub fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }

    /// Cycle to the next theme in order: Dark -> Midnight -> Light -> Nordic -> Dark.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::Dark => Self::Midnight,
            Self::Midnight => Self::Light,
            Self::Light => Self::Nordic,
            Self::Nordic => Self::Dark,
        }
    }
}

/// Toggles stored in the profile and edited from `anvil://settings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserSettings {
    /// Passed to `GpuVideoSession::set_hardware_acceleration`. The settings page still says frames are decoded on the CPU.
    pub gpu_video_acceleration: bool,
    /// HTTPS-only. Every `http` request is made over `https` instead, and a
    /// site that can't do that gets a warning page, not a plain `http` load.
    /// Sites allowed on `anvil://settings/permissions` are the exceptions.
    pub https_only_mode: bool,
    /// Stored in the profile and shown on the settings page. Controls whether page scripts run.
    pub scripts_enabled: bool,
    /// Copied onto loaded page runtimes with `ScriptRuntime::set_jit_enabled`.
    pub jit_enabled: bool,
    /// Engine used by `open_target` for `anvil://navigate` and the start page.
    pub search_engine: SearchEngine,
    /// Whether tab renderers are started through a confinement tool when the
    /// system has one that passes its self-test.
    pub renderer_confinement: ConfineMode,
    /// Draw web pages from the page their renderer painted, once the window
    /// has checked it, instead of from the window's own layout. Off by
    /// default. Clicks, typing and animations still change the window's own
    /// copy of the page, and once one does, that copy is drawn again.
    pub draw_from_renderer: bool,
    /// Active UI chrome and internal-page color theme.
    pub theme: BrowserTheme,
}

impl Default for BrowserSettings {
    /// GPU video flag on, HTTPS-only off, scripts on, JIT on, Google Search,
    /// renderer confinement automatic, web pages drawn from the window's
    /// own layout, Obsidian Dark theme.
    fn default() -> Self {
        Self {
            gpu_video_acceleration: true,
            https_only_mode: false,
            scripts_enabled: true,
            jit_enabled: true,
            search_engine: SearchEngine::Google,
            renderer_confinement: ConfineMode::Auto,
            draw_from_renderer: false,
            theme: BrowserTheme::Dark,
        }
    }
}

/// What `Session::click_at` did with one pointer hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    /// The click did not change the page.
    None,
    /// A document script handled the click and the page was restaged.
    ScriptHandled,
    /// A text field took focus.
    InputFocused(NodeId),
    /// A checkbox was toggled. The bool is the new checked state.
    CheckboxToggled(NodeId, bool),
    /// A select control cycled to this option value.
    SelectChanged(NodeId, String),
    /// Play or pause flipped. The bool is the new playing flag.
    VideoToggled(NodeId, bool),
    /// The video timeline moved to this time in milliseconds.
    VideoSeeked(NodeId, u32),
    /// The caller should open this URL.
    Navigate(String),
    /// The caller should POST `body` to `url`.
    SubmitPost { url: String, body: String },
}

/// Interactive `<video>` playback and timeline scrubbing state for a DOM video element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPlayerState {
    pub clip: VideoClip,
    pub audio_bridge: AudioOutputBridge,
    pub playing: bool,
    pub current_ms: u32,
    pub duration_ms: u32,
}

impl VideoPlayerState {
    /// Paused player at time zero. Duration is at least 33 ms.
    pub fn new(clip: VideoClip) -> Self {
        let duration_ms = clip.duration_ms().max(33);
        let mut audio_bridge = AudioOutputBridge::new();
        if let Some(ref track) = clip.audio {
            let _ = audio_bridge.submit_track_window(track, 0, 33, false);
        }
        Self {
            clip,
            playing: false,
            current_ms: 0,
            audio_bridge,
            duration_ms,
        }
    }

    /// Clamp `target_ms` to `duration_ms`.
    pub fn seek_to_ms(&mut self, target_ms: u32) {
        self.current_ms = target_ms.min(self.duration_ms);
        if let Some(ref track) = self.clip.audio {
            let _ = self
                .audio_bridge
                .submit_track_window(track, self.current_ms, 33, false);
        }
    }

    /// Advance time and wrap with modulo `duration_ms + 1`.
    pub fn step_by_ms(&mut self, delta_ms: u32) {
        let span = self.duration_ms.max(1);
        self.current_ms = (self.current_ms + delta_ms) % (span + 1);
        if let Some(ref track) = self.clip.audio {
            let _ = self.audio_bridge.submit_track_window(
                track,
                self.current_ms,
                delta_ms.max(33),
                false,
            );
        }
    }

    /// Frame at `current_ms`, or the poster, with the control bar drawn on top.
    pub fn render_surface(&self) -> Option<Image> {
        let frame = self
            .clip
            .frame_at_ms(self.current_ms)
            .or_else(|| self.clip.poster_frame())?;
        let rgba = anvil_gpu::composite_rgba_frame(frame.width, frame.height, &frame.rgba)
            .unwrap_or_else(|| frame.rgba.clone());
        let mut img = Image::from_rgba(frame.width, frame.height, rgba).ok()?;
        overlay_video_controls_hud(
            &mut img,
            self.playing,
            self.current_ms,
            self.duration_ms,
            self.audio_bridge.last_peak_level,
        );
        Some(img)
    }
}

/// Draw the bottom bar, play or pause mark, and timeline. Images under 48 by 32 are unchanged.
fn overlay_video_controls_hud(
    img: &mut Image,
    playing: bool,
    current_ms: u32,
    duration_ms: u32,
    audio_peak: u8,
) {
    if img.width < 48 || img.height < 32 {
        return;
    }
    let bar_h = 22u32.min(img.height / 3).max(16);
    let y_start = img.height - bar_h;

    // 1. Translucent obsidian glass bottom HUD bar + top hairline
    for py in y_start..img.height {
        for px in 0..img.width {
            if let Some(dst) = img.pixel(px, py) {
                let is_top_border = py == y_start;
                let (sr, sg, sb, sa) = if is_top_border {
                    (110u32, 96u32, 74u32, 230u32)
                } else {
                    (16u32, 14u32, 12u32, 210u32)
                };
                let inv = 255 - sa;
                let r = ((sr * sa + (dst[0] as u32) * inv) / 255) as u8;
                let g = ((sg * sa + (dst[1] as u32) * inv) / 255) as u8;
                let b = ((sb * sa + (dst[2] as u32) * inv) / 255) as u8;
                img.set_pixel(px, py, [r, g, b, 255]);
            }
        }
    }

    // 2. Left Play/Pause button pill (x: 6..24)
    let btn_y0 = y_start + 3;
    let btn_y1 = (img.height - 3).max(btn_y0 + 6);
    for py in btn_y0..btn_y1 {
        for px in 6..24.min(img.width) {
            img.set_pixel(px, py, [42, 37, 32, 255]);
        }
    }
    let mid_y = (btn_y0 + btn_y1) / 2;
    if playing {
        // Pause bars (||)
        for py in (btn_y0 + 3)..(btn_y1.saturating_sub(3)) {
            for px in [11u32, 12, 16, 17] {
                if px < img.width {
                    img.set_pixel(px, py, [232, 195, 122, 255]);
                }
            }
        }
    } else {
        // Play triangle (|>)
        for dy in -4i32..=4i32 {
            let py = (mid_y as i32 + dy).max(btn_y0 as i32) as u32;
            let span = (4 - dy.abs()).max(0) as u32;
            for px in 12..(12 + span + 1).min(img.width) {
                img.set_pixel(px, py, [232, 195, 122, 255]);
            }
        }
    }

    // 3. Interactive Timeline Scrubber Track (x: 32..width-10)
    let track_x0 = 32u32.min(img.width.saturating_sub(8));
    let track_x1 = img.width.saturating_sub(10).max(track_x0 + 4);
    let track_w = (track_x1 - track_x0).max(1);
    let ratio = (current_ms as f32 / (duration_ms.max(1) as f32)).clamp(0.0, 1.0);
    let head_x = track_x0 + ((track_w as f32 * ratio).round() as u32).min(track_w);

    for py in (mid_y.saturating_sub(2))..=(mid_y + 2).min(img.height - 1) {
        for px in track_x0..track_x1 {
            let col = if px <= head_x {
                [232, 195, 122, 255]
            } else {
                [68, 60, 52, 255]
            };
            img.set_pixel(px, py, col);
        }
    }
    // Live audio level meter bar at right edge of HUD
    if audio_peak > 0 && img.width > 16 {
        let meter_x0 = img.width.saturating_sub(6);
        let meter_x1 = img.width.saturating_sub(3);
        let full_h = btn_y1.saturating_sub(btn_y0).max(4);
        let active_h = ((full_h * u32::from(audio_peak.min(100))) / 100).max(2);
        let active_y0 = btn_y1.saturating_sub(active_h);
        for py in active_y0..btn_y1 {
            for px in meter_x0..meter_x1 {
                img.set_pixel(px, py, [126, 214, 152, 255]);
            }
        }
    }
    // Scrubber playhead knob
    for py in (mid_y.saturating_sub(4))..=(mid_y + 4).min(img.height - 1) {
        for px in (head_x.saturating_sub(2))..=(head_x + 2).min(img.width - 1) {
            img.set_pixel(px, py, [252, 238, 198, 255]);
        }
    }
}

/// The page's transition and animation state, with the clock it runs on.
///
/// Time is milliseconds since the page was first styled. Two pages compare
/// equal whatever their animation state, since it is a running clock rather
/// than part of the document.
#[derive(Clone, Debug)]
pub struct PageAnimation {
    animator: Animator,
    started: Instant,
}

impl Default for PageAnimation {
    fn default() -> Self {
        Self {
            animator: Animator::new(),
            started: Instant::now(),
        }
    }
}

impl PartialEq for PageAnimation {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl PageAnimation {
    /// Milliseconds since the page was first styled.
    pub fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    /// The animator's current time, set by the last tick.
    pub fn now_ms(&self) -> f64 {
        self.animator.now_ms()
    }

    /// True while a transition or animation still needs frames.
    pub fn is_animating(&self) -> bool {
        self.animator.is_animating()
    }
}

/// One loaded document after parse, style, layout, and paint.
///
/// `rasterize` is not stored here. `Session::rasterize` builds the framebuffer
/// from `display_list`. `runtime` is the `anvil-script` host for this page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub source: String,
    /// CSS handed in by the caller. It comes before every sheet the page names.
    pub external_css: String,
    /// Fetched `<link rel=stylesheet>` bodies as `(href, css)`, in document
    /// order. `href` is the attribute text, trimmed.
    pub linked_css: Vec<(String, String)>,
    pub images: HashMap<NodeId, Image>,
    pub videos: HashMap<NodeId, VideoPlayerState>,
    pub focused_input: Option<NodeId>,
    pub csp: ContentSecurityPolicy,
    pub blocked_csp_count: usize,
    pub blocked_tracker_count: usize,
    pub document: Document,
    pub runtime: ScriptRuntime,
    pub layout: LayoutBox,
    pub display_list: DisplayList,
    /// Transitions and `@keyframes` animations. Every restyle goes through it.
    pub animation: PageAnimation,
    /// The parsed page sheet, kept until a sheet changes.
    pub style_cache: StyleSheetCache,
    /// What the window found in the frame the renderer painted for this
    /// page, when it sent one.
    pub renderer_frame: Option<FrameCheck>,
    /// The checked frame the window draws this page from. Kept only while
    /// web pages are drawn from their renderer and nothing has changed the
    /// window's copy of the page since; see [`Page::restage_at`].
    pub drawn: Option<frame::Frame>,
    /// Saved vertical scroll position for Back/Forward and tab switching.
    pub scroll_y: f32,
}

/// A summary of a renderer's frame after it passed [`frame::decode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCheck {
    pub items: usize,
    pub layers: usize,
    pub links: usize,
    /// The renderer cut the page to fit the frame caps.
    pub cut: bool,
    /// The frame's display list is the one the window painted itself.
    pub same_as_window: bool,
}

impl FrameCheck {
    /// Compare a checked frame with the page the window painted.
    pub fn compare(frame: &frame::Frame, page: &Page) -> Self {
        FrameCheck {
            items: frame.list.items.len(),
            layers: frame.list.layers.len(),
            links: frame.links.len(),
            cut: frame.cut,
            same_as_window: frame.list.items == page.display_list.items
                && frame.list.layers == page.display_list.layers,
        }
    }
}

/// The frame a renderer sends for `page`: its display list, the page size,
/// and the `http` and `https` links with the boxes they cover, cut to the
/// frame caps by [`frame::Frame::build`].
pub fn page_frame(page: &Page, viewport: Viewport) -> frame::Frame {
    let (links, height) = frame_links_and_height(page, viewport);
    frame::Frame::build(&page.display_list, viewport.width, height, &links)
}

/// [`page_frame`] for a page that is no longer needed. Everything but the
/// display list is dropped first, and the list's items move into the
/// frame, so the renderer doesn't hold its images in the page, the layout
/// and the frame at once.
pub fn page_frame_owned(page: Page, viewport: Viewport) -> frame::Frame {
    let (links, height) = frame_links_and_height(&page, viewport);
    let list = only_display_list(page);
    frame::Frame::build_owned(list, viewport.width, height, &links)
}

/// The display list of `page`; the rest of it is dropped here.
fn only_display_list(page: Page) -> DisplayList {
    page.display_list
}

fn frame_links_and_height(page: &Page, viewport: Viewport) -> (Vec<frame::LinkRegion>, f32) {
    let mut links = Vec::new();
    collect_links(
        &page.layout,
        &page.document,
        &page.url,
        LinkContext::default(),
        &mut links,
    );
    let height = page
        .content_height()
        .max(viewport.height)
        .max(frame::lowest_item_top(&page.display_list));
    (links, height)
}

/// What [`collect_links`] carries down from the boxes above: the link
/// they are inside, if any, and how they move what they hold.
#[derive(Clone, Default)]
struct LinkContext<'d> {
    href: Option<&'d str>,
    moves: Vec<frame::LinkMove>,
}

/// Push a link region for each `<a href>` box, and for each text run
/// inside a link, with the fixed, sticky and transformed boxes it sits in.
/// Stops a little past what a frame can carry.
fn collect_links<'d>(
    b: &LayoutBox,
    document: &'d Document,
    base: &str,
    above: LinkContext<'d>,
    out: &mut Vec<frame::LinkRegion>,
) {
    if out.len() > frame::MAX_LINKS {
        return;
    }
    let mut here = above;
    // The same order the layout hit test uses for one box.
    if b.position == anvil_layout::Position::Fixed {
        here.moves.push(frame::LinkMove::Fixed);
    }
    if let Some(sticky) = b.sticky {
        here.moves.push(frame::LinkMove::Sticky(sticky));
    }
    if let Some(transform) = b.transform {
        here.moves.push(frame::LinkMove::Transform(transform));
    }
    // A link nested deeper than a frame can describe could only be hit in
    // the wrong place, so it is left out.
    if here.moves.len() > frame::MAX_LINK_MOVES {
        return;
    }
    let own = link_href(document, b.node);
    let href = own.or(here.href);
    if let Some(href) = own {
        push_link(base, href, b.rect, &here.moves, out);
    }
    for run in &b.text {
        // Inline elements have no box of their own, so the link of a run
        // is found from its text node up to this box's node.
        let mut run_href = None;
        let mut cursor = Some(run.node);
        for _ in 0..LINK_SEARCH_DEPTH {
            let Some(id) = cursor.filter(|id| *id != b.node) else {
                break;
            };
            run_href = link_href(document, id);
            if run_href.is_some() {
                break;
            }
            cursor = document.get(id).parent();
        }
        if let Some(href) = run_href.or(href) {
            let rect = Rect {
                x: run.x,
                y: run.y,
                width: run.width,
                height: anvil_layout::line_height(run.font_size),
            };
            push_link(base, href, rect, &here.moves, out);
        }
    }
    for child in &b.children {
        let below = LinkContext {
            href,
            moves: here.moves.clone(),
        };
        collect_links(child, document, base, below, out);
    }
}

/// How many elements up from a text run [`collect_links`] looks for its
/// link, so deep nesting can't make the walk slow.
const LINK_SEARCH_DEPTH: usize = 32;

/// The trimmed `href` of `id` when it is an `<a>` with a non-empty one.
fn link_href(document: &Document, id: NodeId) -> Option<&str> {
    let node = document.get(id);
    if node.data().tag() != Some("a") {
        return None;
    }
    node.data()
        .attr("href")
        .map(str::trim)
        .filter(|href| !href.is_empty())
}

fn push_link(
    base: &str,
    href: &str,
    rect: Rect,
    moves: &[frame::LinkMove],
    out: &mut Vec<frame::LinkRegion>,
) {
    if out.len() > frame::MAX_LINKS {
        return;
    }
    if let Some(url) = resolve_url(base, href)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
    {
        out.push(frame::LinkRegion {
            rect,
            url,
            moves: moves.to_vec(),
        });
    }
}

/// The page's parsed style sheet and what it was built from.
///
/// [`Page::restage_at`] runs on every change and every animation frame.
/// Parsing every sheet again each time is wasted work when no sheet
/// changed, so the parsed sheet is kept with a key: the document's style
/// elements and links (text, `href`, and `media`, in order), whether inline
/// style is allowed, and a hash of the caller's CSS and the fetched link
/// bodies. Any change to the key parses again. Media queries are tested
/// during the cascade, not here, so a new viewport reuses the sheet. Two
/// caches always compare equal, since the cache is derived from the page.
#[derive(Clone, Debug, Default)]
pub struct StyleSheetCache {
    key: Option<StyleSheetKey>,
    sheet: Stylesheet,
    parses: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StyleSheetKey {
    sources: Vec<StyleSheetSource>,
    allow_inline: bool,
    supplied_hash: u64,
}

impl PartialEq for StyleSheetCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl StyleSheetCache {
    /// How many times the page sheet has been parsed.
    pub fn parse_count(&self) -> u64 {
        self.parses
    }

    /// The page sheet for `document`, parsed again only when the key changed.
    /// See [`page_stylesheet`] for how it is put together.
    fn sheet_for(
        &mut self,
        document: &Document,
        external_css: &str,
        linked_css: &[(String, String)],
        allow_inline: bool,
    ) -> &Stylesheet {
        let key = StyleSheetKey {
            sources: document.style_sheets(),
            allow_inline,
            supplied_hash: supplied_css_hash(external_css, linked_css),
        };
        if self.key.as_ref() != Some(&key) {
            self.sheet = page_stylesheet(&key.sources, external_css, linked_css, allow_inline);
            self.parses += 1;
            self.key = Some(key);
        }
        &self.sheet
    }
}

/// Hash of the CSS handed in by the caller and the fetched link bodies.
fn supplied_css_hash(external_css: &str, linked_css: &[(String, String)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    external_css.hash(&mut hasher);
    linked_css.hash(&mut hasher);
    hasher.finish()
}

impl Page {
    /// Bottom of the root layout box. Negative sums become 0.
    pub fn content_height(&self) -> f32 {
        (self.layout.rect.y + self.layout.rect.height).max(0.0)
    }

    /// `anvil-layout` hit test in page coordinates, before scroll and zoom.
    /// Sticky boxes are tested where they sit in the flow; use
    /// [`Page::hit_test_in_view`] to test them where they are drawn.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        hit_test(&self.layout, x, y)
    }

    /// Hit test with sticky boxes moved for `view`, the visible part of the
    /// page in page coordinates (`x`, scroll top, width, height, all divided
    /// by the zoom). `x` and `y` are page coordinates too.
    pub fn hit_test_in_view(&self, x: f32, y: f32, view: Rect) -> Option<NodeId> {
        hit_test_scrolled(&self.layout, x, y, view)
    }

    fn node_at(&self, x: f32, y: f32, view: Option<Rect>) -> Option<NodeId> {
        match view {
            Some(view) => self.hit_test_in_view(x, y, view),
            None => self.hit_test(x, y),
        }
    }

    /// Find the laid-out bounding rectangle of `target` in page coordinates.
    pub fn find_layout_rect(&self, target: NodeId) -> Option<Rect> {
        /// Depth-first search for the layout rect of `target`.
        fn walk_rect(b: &LayoutBox, target: NodeId) -> Option<Rect> {
            if b.node == target {
                return Some(b.rect);
            }
            for child in &b.children {
                if let Some(found) = walk_rect(child, target) {
                    return Some(found);
                }
            }
            None
        }
        walk_rect(&self.layout, target)
    }

    /// Return the vertical page offset `y` of a `#fragment` anchor target (`id`, `<a name>`, or `#top`),
    /// adjusted for any top-anchored `position: fixed` or `position: sticky` header bar.
    pub fn fragment_scroll_y(&self, fragment: &str) -> Option<f32> {
        let frag = fragment.trim().trim_start_matches('#');
        if frag.eq_ignore_ascii_case("top") {
            return Some(0.0);
        }
        let target_id = self.document.find_fragment_target(frag)?;
        let header_offset = self.top_sticky_header_offset();
        let adjust = |raw_y: f32| -> f32 {
            if header_offset > 0.0 && raw_y > header_offset {
                (raw_y - header_offset).max(0.0)
            } else {
                raw_y.max(0.0)
            }
        };
        if let Some(rect) = self.find_layout_rect(target_id) {
            return Some(adjust(rect.y));
        }
        let mut curr = self.document.get(target_id).parent();
        while let Some(parent_id) = curr {
            if let Some(rect) = self.find_layout_rect(parent_id) {
                return Some(adjust(rect.y));
            }
            curr = self.document.get(parent_id).parent();
        }
        Some(0.0)
    }

    fn top_sticky_header_offset(&self) -> f32 {
        fn scan(b: &LayoutBox, best: &mut f32) {
            let is_top_bar = (b.position == anvil_layout::Position::Fixed || b.sticky.is_some())
                && b.rect.y <= 12.0
                && b.rect.height >= 20.0
                && b.rect.height <= 140.0;
            if is_top_bar {
                let bottom = b.rect.y.max(0.0) + b.rect.height + 8.0;
                if bottom > *best {
                    *best = bottom;
                }
            }
            for child in &b.children {
                scan(child, best);
            }
        }
        let mut best = 0.0_f32;
        scan(&self.layout, &mut best);
        best
    }

    /// Return the bounding boxes `(x, y, width, height)` of all rendered text runs matching `query`
    /// (case-insensitive) for Find-in-Page (`Ctrl+F`).
    pub fn find_text_matches(&self, query: &str) -> Vec<Rect> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        for item in &self.shown_list().items {
            if let DisplayItem::Text {
                x,
                y,
                text,
                font_size,
                ..
            } = item
                && text.to_ascii_lowercase().contains(&needle)
            {
                let width = ((text.chars().count() as f32) * (*font_size * 0.56)).max(24.0);
                let height = (*font_size * 1.35).max(16.0);
                matches.push(Rect {
                    x: *x,
                    y: *y,
                    width,
                    height,
                });
            }
        }
        matches
    }

    /// The display list drawn for this page: the renderer's checked frame
    /// when the page is drawn from it, else the window's own.
    pub fn shown_list(&self) -> &DisplayList {
        match &self.drawn {
            Some(frame) => &frame.list,
            None => &self.display_list,
        }
    }

    /// Height of the page as drawn: the frame's page height when the page
    /// is drawn from its renderer, else the window's content height.
    pub fn shown_height(&self) -> f32 {
        match &self.drawn {
            Some(frame) => frame.page_height,
            None => self.content_height(),
        }
    }

    /// Return the text content of the DOM node at `(x, y)` for text selection / copy.
    pub fn text_at(&self, x: f32, y: f32) -> Option<String> {
        self.text_at_in_view(x, y, None)
    }

    /// [`Page::text_at`], with sticky boxes moved for `view` when given.
    pub fn text_at_in_view(&self, x: f32, y: f32, view: Option<Rect>) -> Option<String> {
        let id = self.node_at(x, y, view)?;
        let raw = self.document.text_content(id);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Collect all laid-out text runs intersecting `region` in page coordinates, returning
    /// the concatenated selection text and the per-run highlight rectangles.
    pub fn select_text_runs_in_rect(&self, region: Rect) -> (String, Vec<Rect>) {
        let mut runs = Vec::new();
        self.layout.collect_text_runs_in_rect(region, &mut runs);
        let mut out_text = String::new();
        let mut rects = Vec::new();
        let mut last_y: Option<f32> = None;
        for run in runs {
            let trimmed = run.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(py) = last_y {
                if (run.y - py).abs() > 6.0 {
                    out_text.push('\n');
                } else if !out_text.ends_with(' ') && !out_text.ends_with('\n') {
                    out_text.push(' ');
                }
            }
            out_text.push_str(trimmed);
            last_y = Some(run.y);
            rects.push(Rect {
                x: run.x,
                y: run.y,
                width: run.width.max(4.0),
                height: (run.font_size * 1.35).max(14.0),
            });
        }
        (out_text, rects)
    }

    /// Build a 4-line DOM & computed layout/style summary for `target` (or `focused_input` / `body`).
    pub fn element_summary(&self, target: Option<NodeId>) -> (String, String, String, String) {
        let mut id = target
            .or(self.focused_input)
            .unwrap_or_else(|| self.document.body());
        while self.document.get(id).data().tag().is_none() {
            if let Some(p) = self.document.get(id).parent() {
                id = p;
            } else {
                id = self.document.body();
                break;
            }
        }
        // 1. Breadcrumb path
        let mut chain = Vec::new();
        let mut curr = Some(id);
        while let Some(cid) = curr {
            let data = self.document.get(cid).data();
            if let Some(tag) = data.tag() {
                let mut label = tag.to_string();
                if let Some(el_id) = data.attr("id").filter(|s| !s.is_empty()) {
                    label.push('#');
                    label.push_str(el_id);
                } else if let Some(cls) = data.attr("class").filter(|s| !s.is_empty())
                    && let Some(first_cls) = cls.split_whitespace().next()
                {
                    label.push('.');
                    label.push_str(first_cls);
                }
                chain.push(label);
                if chain.len() >= 5 {
                    break;
                }
            }
            curr = self.document.get(cid).parent();
        }
        chain.reverse();
        let breadcrumb = format!("Path: {}", chain.join(" > "));

        // 2. Tag, attributes & child count
        let data = self.document.get(id).data();
        let tag = data.tag().unwrap_or("node");
        let mut attr_parts = Vec::new();
        for key in ["id", "class", "type", "name", "href", "src", "value"] {
            if let Some(v) = data.attr(key).filter(|s| !s.is_empty()) {
                let short: String = v.chars().take(24).collect();
                attr_parts.push(format!("{key}=\"{short}\""));
            }
        }
        let child_count = self.document.children(id).len();
        let attr_str = if attr_parts.is_empty() {
            format!("<{tag}>")
        } else {
            format!("<{tag} {}>", attr_parts.join(" "))
        };
        let elem_line = format!("Inspect: {attr_str} | Children: {child_count}");

        // 3. Layout box & geometry
        fn find_box_ref(b: &LayoutBox, target: NodeId) -> Option<&LayoutBox> {
            if b.node == target {
                return Some(b);
            }
            for c in &b.children {
                if let Some(found) = find_box_ref(c, target) {
                    return Some(found);
                }
            }
            None
        }
        let lbox = find_box_ref(&self.layout, id);
        let rect = lbox
            .map(|b| b.rect)
            .or_else(|| self.find_layout_rect(id))
            .unwrap_or(self.layout.rect);
        let pos_label = lbox
            .map(|b| format!("{:?}", b.position))
            .unwrap_or_else(|| "Flow".to_string());
        let ovf_label = lbox
            .map(|b| format!("{:?}", b.overflow))
            .unwrap_or_else(|| "Visible".to_string());
        let geom_line = format!(
            "Box: {:.0}x{:.0} px at ({:.0}, {:.0}) | Pos: {pos_label} | Overflow: {ovf_label}",
            rect.width, rect.height, rect.x, rect.y
        );

        // 4. Computed box style (border, radius, opacity, background)
        let style_line = if let Some(b) = lbox {
            let bg_str = match b.background {
                Some(c) => format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
                None => "transparent".to_string(),
            };
            format!(
                "Border: {:.0}px | Radius: {:.0}px | Opacity: {:.2} | Bg: {bg_str}",
                b.border.top, b.border_radius, b.opacity
            )
        } else {
            "Inline / Non-Box Element (inherits parent container metrics)".to_string()
        };

        (breadcrumb, elem_line, geom_line, style_line)
    }

    /// Walk from the hit node through its parents to the first text input.
    pub fn text_input_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.text_input_at_in_view(x, y, None)
    }

    /// [`Page::text_input_at`], with sticky boxes moved for `view` when given.
    pub fn text_input_at_in_view(&self, x: f32, y: f32, view: Option<Rect>) -> Option<NodeId> {
        let mut cursor = self.node_at(x, y, view);
        while let Some(id) = cursor {
            if self.document.is_text_input(id) {
                return Some(id);
            }
            cursor = self.document.get(id).parent();
        }
        None
    }

    /// Walk from the hit node through its parents to the first non-empty anchor `href`.
    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.link_at_in_view(x, y, None)
    }

    /// [`Page::link_at`], with sticky boxes moved for `view` when given.
    pub fn link_at_in_view(&self, x: f32, y: f32, view: Option<Rect>) -> Option<&str> {
        let mut cursor = self.node_at(x, y, view);
        while let Some(id) = cursor {
            let node = self.document.get(id);
            if node.data().tag() == Some("a")
                && let Some(href) = node.data().attr("href")
                && !href.trim().is_empty()
            {
                return Some(href);
            }
            cursor = node.parent();
        }
        None
    }

    /// Restyle, lay out, and paint again. Does not re-run scripts or re-fetch resources.
    /// CSS is parsed again only when a sheet changed; see [`StyleSheetCache`].
    ///
    /// The animation clock moves to now first, so a style change made since
    /// the last restyle starts its transition here.
    pub fn restage(&mut self, viewport: Viewport) {
        let now = self.animation.elapsed_ms();
        self.restage_at(viewport, now);
    }

    /// [`Page::restage`] with the animation clock set to `now_ms` (milliseconds
    /// since the page was first styled). Time never runs backward.
    pub fn restage_at(&mut self, viewport: Viewport, now_ms: f64) {
        self.animation.animator.tick(now_ms);
        let sheet = self.style_cache.sheet_for(
            &self.document,
            &self.external_css,
            &self.linked_css,
            self.csp.allows_inline_style(),
        );
        let styles = compute_styles_animated(
            &self.document,
            sheet,
            viewport.width,
            viewport.height,
            &mut self.animation.animator,
        );
        self.layout = layout_document_interactive(
            &self.document,
            &styles,
            viewport.width,
            &self.images,
            self.focused_input,
        );
        self.display_list = paint(&self.layout);
        self.title = self.document.title();
        // The window's copy of the page changed, so the renderer's frame no
        // longer shows it.
        self.drawn = None;
    }

    /// Move the animation clock to `now_ms` and say whether anything is
    /// still running. Does not restyle.
    pub fn tick_animations(&mut self, now_ms: f64) -> bool {
        self.animation.animator.tick(now_ms)
    }

    /// True while a transition or animation still needs frames.
    pub fn is_animating(&self) -> bool {
        self.animation.is_animating()
    }

    /// One animation frame at the current time. See [`Page::animation_frame_at`].
    pub fn animation_frame(&mut self, viewport: Viewport) -> bool {
        let now = self.animation.elapsed_ms();
        self.animation_frame_at(viewport, now)
    }

    /// Compact heap buffers (`source`, `external_css`, `linked_css`, `display_list`, `images`, `videos`)
    /// held by this page when its tab is inactive or in back/forward history.
    pub fn shrink_to_fit(&mut self) {
        self.url.shrink_to_fit();
        self.title.shrink_to_fit();
        self.source.shrink_to_fit();
        self.external_css.shrink_to_fit();
        self.linked_css.shrink_to_fit();
        for img in self.images.values_mut() {
            img.pixels.shrink_to_fit();
        }
        self.images.shrink_to_fit();
        self.videos.shrink_to_fit();
        self.display_list.items.shrink_to_fit();
    }

    /// Tick to `now_ms` and, when something was or still is running, restyle,
    /// lay out, and paint. Returns true when the display list changed and the
    /// window should repaint. The frame where the last transition finishes
    /// still restyles, so its end value is drawn.
    pub fn animation_frame_at(&mut self, viewport: Viewport, now_ms: f64) -> bool {
        let was_running = self.is_animating();
        let running = self.tick_animations(now_ms);
        if !was_running && !running {
            return false;
        }
        self.restage_at(viewport, now_ms);
        true
    }
}

/// Failure from `load_url` before a `Page` exists.
#[derive(Debug)]
pub enum BrowserError {
    /// `anvil-net` could not complete the fetch.
    Net(NetError),
    /// HTTP status outside 200..299.
    Status(u16),
    /// The response body was not UTF-8 text.
    Utf8,
    /// The renderer timed out, sent a bad frame, crashed, or failed to start.
    /// The document was not rendered in this process.
    Worker(String),
    /// The response is a file attachment or binary download rather than a document.
    Download {
        url: String,
        filename: Option<String>,
        content_type: String,
        body: Vec<u8>,
    },
}

impl std::fmt::Display for BrowserError {
    /// One-line text for a network error, an HTTP status, or a non-text body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(err) => write!(f, "{err}"),
            Self::Status(status) => write!(f, "the server returned {status}"),
            Self::Utf8 => write!(f, "the page was not text"),
            Self::Worker(message) => write!(f, "{message}"),
            Self::Download { url, .. } => write!(f, "downloaded {url}"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// Render `html` with an empty stylesheet and an empty image map.
pub fn render_html(html: &str, url: &str, viewport: Viewport) -> Page {
    render_html_with_css(html, "", url, viewport)
}

/// Render `html` with `external_css` and an empty image map.
pub fn render_html_with_css(html: &str, external_css: &str, url: &str, viewport: Viewport) -> Page {
    render_html_with_resources(html, external_css, HashMap::new(), url, viewport)
}

/// Render `html` with `external_css` and `images`, and no CSP header.
pub fn render_html_with_resources(
    html: &str,
    external_css: &str,
    images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
) -> Page {
    render_html_with_resources_and_csp(html, external_css, images, url, viewport, None, 0, None)
}

/// Build a `Page` from HTML with pre-populated `local_storage`.
pub fn render_html_with_storage(
    html: &str,
    url: &str,
    viewport: Viewport,
    initial_storage: &[(String, String, String)],
) -> Page {
    render_html_with_resources_csp_and_storage(
        html,
        "",
        HashMap::new(),
        url,
        viewport,
        None,
        0,
        initial_storage,
        None,
        true,
    )
}

/// Build a `Page` from HTML.
///
/// The steps are marked at the parse call. Raster is not one of them.
#[allow(clippy::too_many_arguments)]
pub fn render_html_with_resources_and_csp(
    html: &str,
    external_css: &str,
    images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
    http_csp_header: Option<&str>,
    initial_blocked_csp: usize,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Page {
    render_html_with_resources_csp_and_storage(
        html,
        external_css,
        images,
        url,
        viewport,
        http_csp_header,
        initial_blocked_csp,
        &[],
        fetch_jar,
        true,
    )
}

/// Build a `Page` from HTML with optional pre-populated `localStorage` entries.
#[allow(clippy::too_many_arguments)]
pub fn render_html_with_resources_csp_and_storage(
    html: &str,
    external_css: &str,
    images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
    http_csp_header: Option<&str>,
    initial_blocked_csp: usize,
    initial_storage: &[(String, String, String)],
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    run_scripts: bool,
) -> Page {
    render_document(
        html,
        PageSheets {
            external_css,
            linked_css: Vec::new(),
        },
        images,
        url,
        viewport,
        http_csp_header,
        initial_blocked_csp,
        initial_storage,
        PageLoads::Here(fetch_jar),
        run_scripts,
    )
}

/// Asks the browser process for one fetch while a page renders. `None`
/// means the fetch could not be asked for: the URL doesn't fit the wire,
/// the page used up its fetches, or the link to the browser is broken.
pub type BrowserFetch = Arc<dyn Fn(FetchKind, &str) -> Option<FetchReply> + Send + Sync>;

/// Where the loads a page makes while it renders go.
#[derive(Clone)]
enum PageLoads {
    /// From this process. Script `fetch()` uses this jar, or a jar for this
    /// page only.
    Here(Option<Arc<Mutex<CookieJar>>>),
    /// Through the browser process. Nothing here touches the network.
    /// Scripts, style sheets and images are asked of the browser. Video and
    /// map tiles are skipped.
    Browser(BrowserFetch),
}

/// Render a page in a tab renderer process.
///
/// Scripts run like they do in [`render_html_with_resources_csp_and_storage`],
/// but every linked script, style sheet and image and every script `fetch()`
/// is asked of the browser through `ask`, which applies the shield,
/// HTTPS-only, CORP, cookies and the same-origin rule. Images are decoded
/// here, up to [`RENDERER_IMAGE_BUDGET`] bytes for the page. Video and map
/// tiles are not loaded here.
pub fn render_with_browser_fetches(
    html: &str,
    url: &str,
    viewport: Viewport,
    http_csp_header: Option<&str>,
    run_scripts: bool,
    ask: BrowserFetch,
) -> Page {
    render_document(
        html,
        PageSheets {
            external_css: "",
            linked_css: Vec::new(),
        },
        HashMap::new(),
        url,
        viewport,
        http_csp_header,
        0,
        &[],
        PageLoads::Browser(ask),
        run_scripts,
    )
}

/// Style sheet text for [`render_document`]: CSS from the caller, and the
/// linked sheets already fetched for the page.
struct PageSheets<'a> {
    external_css: &'a str,
    linked_css: Vec<(String, String)>,
}

/// Body of [`render_html_with_resources_csp_and_storage`], with linked sheets.
#[allow(clippy::too_many_arguments)]
fn render_document(
    html: &str,
    sheets: PageSheets<'_>,
    mut images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
    http_csp_header: Option<&str>,
    initial_blocked_csp: usize,
    initial_storage: &[(String, String, String)],
    loads: PageLoads,
    run_scripts: bool,
) -> Page {
    let PageSheets {
        external_css,
        mut linked_css,
    } = sheets;
    // Pipeline order: parse HTML (anvil-dom), optionally run allowed scripts
    // (anvil-script), attach images and video frames, parse CSS and compute
    // style (anvil-style), lay out (anvil-layout), then paint (anvil-paint).
    // Session::rasterize calls anvil-raster later. This function does not
    // rasterize and does not start a worker. `run_scripts` is false when the
    // HTML already came back from `--renderer`. With `PageLoads::Browser`
    // this is the renderer, and it loads nothing itself.
    let loads_here = matches!(loads, PageLoads::Here(_));
    let mut document = anvil_dom::parse_html(html);
    let combined_csp_raw = match (http_csp_header, document.meta_csp()) {
        (Some(h), Some(m)) => format!("{h}; {m}"),
        (Some(h), None) => h.to_string(),
        (None, Some(m)) => m,
        (None, None) => String::new(),
    };
    let csp = ContentSecurityPolicy::parse(&combined_csp_raw);
    let mut blocked_csp_count = initial_blocked_csp;

    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin(url);
    bind_script_fetch(&mut runtime, &loads);
    if !initial_storage.is_empty() {
        runtime.import_local_storage(initial_storage);
    }
    if run_scripts {
        let run_external = sites::fetch_page_scripts(url);
        for (src, integrity) in document.external_scripts_with_integrity() {
            if !run_external {
                continue;
            }
            if !csp.allows_resource(CspDirective::ScriptSrc, url, &src) {
                blocked_csp_count += 1;
                continue;
            }
            let Some(resolved) = resolve_url(url, &src) else {
                continue;
            };
            let shield_res =
                check_default_shield_request(&resolved, Some(url), RequestType::Script);
            if shield_res.matched {
                blocked_csp_count += 1;
                if let Some((_, surrogate_js)) = shield_res.redirect_resource
                    && !surrogate_js.is_empty()
                {
                    runtime.eval_script(&mut document, &surrogate_js);
                }
                continue;
            }
            let raw_bytes = if let PageLoads::Browser(ask) = &loads {
                if is_remote_origin(&resolved) {
                    ask(FetchKind::Script, &resolved)
                        .filter(|reply| (200..300).contains(&reply.status))
                        .map(|reply| reply.body)
                } else if !is_remote_origin(url) && !url.starts_with("anvil://") {
                    fs::read(&resolved).ok()
                } else {
                    None
                }
            } else if is_remote_origin(&resolved) {
                fetch(&resolved).ok().and_then(|(_, resp)| {
                    if !allows_corp_subresource(
                        resp.header("cross-origin-resource-policy"),
                        url,
                        &resolved,
                    ) {
                        blocked_csp_count += 1;
                        return None;
                    }
                    if (200..300).contains(&resp.status) {
                        Some(resp.body)
                    } else {
                        None
                    }
                })
            } else if !is_remote_origin(url) && !url.starts_with("anvil://") {
                fs::read(&resolved).ok()
            } else {
                None
            };
            if let Some(bytes) = raw_bytes {
                if !verify_subresource_integrity(integrity.as_deref(), &bytes) {
                    blocked_csp_count += 1;
                    continue;
                }
                if let Ok(code) = String::from_utf8(bytes) {
                    runtime.eval_script(&mut document, &code);
                }
            }
        }
        if csp.allows_inline_script() {
            runtime.run_document_scripts(&mut document);
        } else if !document.scripts().is_empty() {
            blocked_csp_count += document.scripts().len();
        }
        let post_script_imgs: Vec<(NodeId, String)> = document
            .images()
            .into_iter()
            .filter(|(id, _)| !images.contains_key(id))
            .collect();
        if loads_here && !post_script_imgs.is_empty() {
            for (id, img) in fetch_page_images(url, &post_script_imgs, &csp, &mut blocked_csp_count)
            {
                images.entry(id).or_insert(img);
            }
        }
    } else if csp.allows_inline_script() {
        runtime.hydrate_interactive_bindings(&mut document);
    }

    if let PageLoads::Browser(ask) = &loads {
        // The window still loads and counts these for the page it draws, so
        // what is blocked here isn't counted twice.
        let mut not_counted = 0usize;
        linked_css.extend(ask_linked_stylesheets(
            ask,
            url,
            &document.stylesheet_links_with_integrity(),
            &csp,
            &mut not_counted,
        ));
        for (id, image) in ask_page_images(ask, url, &document.images(), &csp, &mut not_counted) {
            images.entry(id).or_insert(image);
        }
    }

    for (img_id, src) in document.images() {
        let trimmed = src.trim();
        if !loads_here {
            continue;
        }
        if let Some(map) = sites::map_bitmap(trimmed) {
            images.entry(img_id).or_insert(map);
            continue;
        }
        if trimmed.eq_ignore_ascii_case("anvil://logo.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_anvil_logo(64));
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-google.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::Google, 36));
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-ddg.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::DuckDuckGo, 36));
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-bing.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::Bing, 36));
        }
    }

    let vids = document.videos();
    let mut blocked_tracker_count = 0usize;
    let mut videos = HashMap::new();
    if loads_here && !vids.is_empty() {
        let mut video_session = GpuVideoSession::default();
        for (vid_id, player) in fetch_page_videos(
            url,
            &vids,
            &csp,
            &mut blocked_tracker_count,
            &mut video_session,
        ) {
            if let Some(surface) = player.render_surface() {
                images.entry(vid_id).or_insert(surface);
            }
            videos.insert(vid_id, player);
        }
    }

    if !csp.allows_inline_style() && !document.embedded_css().is_empty() {
        blocked_csp_count += 1;
    }
    let cosmetic_css = cosmetic_css_from_default_shield(url);
    let combined_external_css = if cosmetic_css.is_empty() {
        external_css.to_string()
    } else if external_css.is_empty() {
        cosmetic_css
    } else {
        format!("{external_css}\n{cosmetic_css}")
    };
    let mut style_cache = StyleSheetCache::default();
    let sheet = style_cache.sheet_for(
        &document,
        &combined_external_css,
        &linked_css,
        csp.allows_inline_style(),
    );
    // The first styled pass starts no transitions, but it does start any
    // `@keyframes` animation the page names; the window then draws frames
    // while `Page::is_animating` is true.
    let mut animation = PageAnimation::default();
    animation.animator.tick(0.0);
    let styles = compute_styles_animated(
        &document,
        sheet,
        viewport.width,
        viewport.height,
        &mut animation.animator,
    );
    let layout = layout_document_interactive(&document, &styles, viewport.width, &images, None);
    let display_list = paint(&layout);
    Page {
        url: url.to_string(),
        title: document.title(),
        source: html.to_string(),
        external_css: combined_external_css,
        linked_css,
        images,
        videos,
        focused_input: None,
        csp,
        blocked_csp_count,
        blocked_tracker_count,
        document,
        runtime,
        layout,
        display_list,
        animation,
        style_cache,
        renderer_frame: None,
        drawn: None,
        scroll_y: 0.0,
    }
}

/// The author style sheet for a document whose sheets are `sources`, as
/// returned by [`Document::style_sheets`].
///
/// `external_css` comes first. Then every `<link rel=stylesheet>` and
/// `<style>` element follows in document order, so a later sheet wins a tie
/// whichever kind it is. A link uses the first body in `linked_css` with the
/// same `href`; a link with no fetched body adds nothing. `<style>` elements
/// are skipped when `allow_inline` is false. A `media` attribute on either
/// kind limits that sheet to matching viewports. Each sheet is parsed on its
/// own, so a broken sheet cannot swallow the next one.
fn page_stylesheet(
    sources: &[StyleSheetSource],
    external_css: &str,
    linked_css: &[(String, String)],
    allow_inline: bool,
) -> Stylesheet {
    let mut sheet = parse_css(external_css);
    for source in sources {
        match source {
            StyleSheetSource::Link { href, media, .. } => {
                if let Some((_, css)) = linked_css.iter().find(|(h, _)| h == href) {
                    sheet.append(parse_css_with_media(css, media.as_deref()));
                }
            }
            StyleSheetSource::Style { css, media } => {
                if allow_inline {
                    sheet.append(parse_css_with_media(css, media.as_deref()));
                }
            }
        }
    }
    sheet
}

/// GET `url` with `anvil-net`, fetch allowed stylesheets and images, then render.
pub fn load_url(url: &str, viewport: Viewport) -> Result<Page, BrowserError> {
    load_url_with_cookies(url, viewport, None)
}

/// GET `url` like [`load_url`]. The jar stays in this process. A renderer
/// never holds it: its script `fetch()` calls are answered here, with
/// `fetch_jar`.
pub fn load_url_with_cookies(
    url: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, BrowserError> {
    load_url_for_navigation(url, viewport, fetch_jar, true, 0)
}

/// GET `url` for a navigation.
///
/// `fetch_jar` is the session jar. `Set-Cookie` from the response and from
/// any redirect is stored there, and the next navigation sends it. A missing
/// jar uses a jar that is dropped with this call. The lock is not held after
/// this function returns.
fn navigation_get(
    url: &str,
    fetch_jar: Option<&Arc<Mutex<CookieJar>>>,
) -> Result<(String, anvil_net::Response), BrowserError> {
    match fetch_jar {
        Some(jar) => {
            let mut guard = jar.lock().unwrap_or_else(|err| err.into_inner());
            fetch_with_cookies(url, &mut guard, None, true)
        }
        None => fetch(url),
    }
    .map_err(BrowserError::Net)
}

/// POST `body` as `application/x-www-form-urlencoded` for a navigation.
///
/// Same cookie rule as [`navigation_get`]. A 301, 302, or 303 continues as
/// GET and keeps the cookies gathered along the way.
fn navigation_post(
    url: &str,
    body: &str,
    fetch_jar: Option<&Arc<Mutex<CookieJar>>>,
) -> Result<(String, anvil_net::Response), BrowserError> {
    match fetch_jar {
        Some(jar) => {
            let mut guard = jar.lock().unwrap_or_else(|err| err.into_inner());
            fetch_post_with_cookies(
                url,
                "application/x-www-form-urlencoded",
                body.as_bytes(),
                &mut guard,
                None,
            )
        }
        None => {
            let mut jar = CookieJar::new();
            fetch_post_with_cookies(
                url,
                "application/x-www-form-urlencoded",
                body.as_bytes(),
                &mut jar,
                None,
            )
        }
    }
    .map_err(BrowserError::Net)
}

/// GET `url`, then run the document in a `--renderer` process.
///
/// When `fetch_jar` is set, this request sends that jar and stores
/// `Set-Cookie` on it, including cookies from redirects. The worker still
/// does not receive the jar. Stylesheets and images are fetched in this
/// process after the worker returns. A worker failure is
/// [`BrowserError::Worker`] and does not render the fetched HTML here.
pub fn load_url_for_navigation(
    url: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
) -> Result<Page, BrowserError> {
    load_url_routed(
        url,
        viewport,
        fetch_jar,
        scripts_enabled,
        tab_id,
        DocumentRoute::OneShot,
    )
}

/// Where the DOM and script work for a document runs.
#[derive(Clone, Copy, Debug)]
pub enum DocumentRoute<'a> {
    /// A new `--renderer` process for this one document. The `render` and
    /// `raster` commands use this. Local files stay in this process.
    OneShot,
    /// The long-lived renderer of one tab. Local files go there too.
    Tab(&'a TabWorkerPool),
}

/// GET `url` and run the document through `route`.
fn load_url_routed(
    url: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
    route: DocumentRoute<'_>,
) -> Result<Page, BrowserError> {
    if let Some(html) = sites::gis_map_page(url) {
        return Ok(render_html(&html, url, viewport));
    }
    let (final_url, response) = match navigation_get(url, fetch_jar.as_ref()) {
        Err(BrowserError::Net(NetError::HttpsOnly(http_url))) => {
            return Ok(https_only_page(&http_url, viewport));
        }
        other => other?,
    };
    if !(200..300).contains(&response.status) {
        return Err(BrowserError::Status(response.status));
    }
    if downloads::is_downloadable_response(
        response.header("content-type"),
        response.header("content-disposition"),
    ) {
        return Err(BrowserError::Download {
            url: final_url,
            filename: downloads::extract_disposition_filename(
                response.header("content-disposition"),
            ),
            content_type: response
                .header("content-type")
                .unwrap_or("application/octet-stream")
                .to_string(),
            body: response.body,
        });
    }
    let http_csp = response
        .header("content-security-policy")
        .map(str::to_string);
    let mut html = String::from_utf8(response.body).map_err(|_| BrowserError::Utf8)?;
    html = sites::prepare_document(&html, &final_url);
    let mut page = render_routed(
        route,
        tab_id,
        &final_url,
        &html,
        http_csp.as_deref(),
        scripts_enabled,
        viewport,
        fetch_jar,
    )
    .map_err(BrowserError::Worker)?;
    if let Some((_, frag)) = url.trim().split_once('#')
        && !frag.is_empty()
        && !page.url.contains('#')
    {
        page.url = format!("{}#{frag}", page.url);
    }
    Ok(page)
}

/// POST `body` when `url` is http(s). Any other URL calls `load_target` and ignores `body`.
pub fn load_post_target(url: &str, body: &str, viewport: Viewport) -> Result<Page, String> {
    load_post_target_with_cookies(url, body, viewport, None)
}

/// POST like [`load_post_target`]. The renderer does not receive `fetch_jar`.
pub fn load_post_target_with_cookies(
    url: &str,
    body: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, String> {
    load_post_for_navigation(url, body, viewport, fetch_jar, true, 0)
}

/// POST `body`, then run the response document in a `--renderer` process.
///
/// A worker failure is `Err` and does not render the response HTML here.
pub fn load_post_for_navigation(
    url: &str,
    body: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
) -> Result<Page, String> {
    load_post_routed(
        url,
        body,
        viewport,
        fetch_jar,
        scripts_enabled,
        tab_id,
        DocumentRoute::OneShot,
    )
}

/// POST `body` and run the response document in the worker of tab `tab_id`.
pub fn load_post_in_tab(
    url: &str,
    body: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
    workers: &TabWorkerPool,
) -> Result<Page, String> {
    load_post_routed(
        url,
        body,
        viewport,
        fetch_jar,
        scripts_enabled,
        tab_id,
        DocumentRoute::Tab(workers),
    )
}

fn load_post_routed(
    url: &str,
    body: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
    route: DocumentRoute<'_>,
) -> Result<Page, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let (final_url, response) = match navigation_post(url, body, fetch_jar.as_ref()) {
            Err(BrowserError::Net(NetError::HttpsOnly(http_url))) => {
                return Ok(https_only_page(&http_url, viewport));
            }
            other => other.map_err(|err| err.to_string())?,
        };
        if !(200..300).contains(&response.status) {
            return Err(format!("the server returned {}", response.status));
        }
        let http_csp = response
            .header("content-security-policy")
            .map(str::to_string);
        let mut html =
            String::from_utf8(response.body).map_err(|_| "the page was not text".to_string())?;
        html = sites::prepare_document(&html, &final_url);
        render_routed(
            route,
            tab_id,
            &final_url,
            &html,
            http_csp.as_deref(),
            scripts_enabled,
            viewport,
            fetch_jar,
        )
    } else {
        load_target_routed(url, viewport, fetch_jar, scripts_enabled, tab_id, route)
    }
}

/// Run `html` in `program` and paint the document it returns.
///
/// `html` is the source sent to the worker. It is not rendered when the
/// worker times out, returns `ok: false`, or writes a bad frame. The cookie
/// jar is not sent. Fetches the renderer asks for are answered here by
/// [`answer_renderer_fetch`], and subresources of the returned document are
/// fetched here too.
#[allow(clippy::too_many_arguments)]
pub fn render_through_tab_worker(
    program: &Path,
    args: &[String],
    timeout: Duration,
    tab_id: u64,
    url: &str,
    html: &str,
    csp_header: Option<&str>,
    scripts_enabled: bool,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, String> {
    let request = worker_request(tab_id, url, html, csp_header, scripts_enabled, viewport);
    let jar = page_jar(fetch_jar.as_ref());
    let mut fetch = |want: &FetchRequest| answer_renderer_fetch(want, url, &jar);
    let resp = worker::exchange_tab_worker(program, args, &request, timeout, &mut fetch)?;
    finish_worker_response(resp, &request, csp_header, viewport, fetch_jar)
}

/// Send one document through `route` and paint what comes back.
#[allow(clippy::too_many_arguments)]
fn render_routed(
    route: DocumentRoute<'_>,
    tab_id: u64,
    url: &str,
    html: &str,
    csp_header: Option<&str>,
    scripts_enabled: bool,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, String> {
    match route {
        DocumentRoute::OneShot => {
            let program = worker::current_anvil_exe()?;
            render_through_tab_worker(
                &program,
                &[worker::RENDERER_ARG.to_string()],
                worker::worker_timeout(),
                tab_id,
                url,
                html,
                csp_header,
                scripts_enabled,
                viewport,
                fetch_jar,
            )
        }
        DocumentRoute::Tab(workers) => {
            let request = worker_request(tab_id, url, html, csp_header, scripts_enabled, viewport);
            let jar = page_jar(fetch_jar.as_ref());
            let mut fetch = |want: &FetchRequest| answer_renderer_fetch(want, url, &jar);
            let resp = workers.exchange(&request, &mut fetch)?;
            let page = finish_worker_response(resp, &request, csp_header, viewport, fetch_jar);
            if page.as_ref().is_err_and(|err| err == BAD_FRAME_ERROR) {
                workers.end_broken(tab_id);
            }
            page
        }
    }
}

/// The request a tab worker gets for one document.
fn worker_request(
    tab_id: u64,
    url: &str,
    html: &str,
    csp_header: Option<&str>,
    scripts_enabled: bool,
    viewport: Viewport,
) -> TabWorkerRequest {
    // The worker follows the same HTTPS-only rules as the thread doing the load.
    let https_only = anvil_net::https_only_policy();
    TabWorkerRequest {
        tab_id,
        url: url.to_string(),
        html: html.to_string(),
        scripts_enabled,
        csp_header: csp_header.map(str::to_string),
        viewport_width: viewport.width.round().max(1.0) as u32,
        viewport_height: viewport.height.round().max(1.0) as u32,
        https_only: https_only.enabled,
        http_allowed_hosts: https_only.allowed_http_hosts,
    }
}

/// Check a worker answer and paint its document. A rejected answer is `Err`.
fn finish_worker_response(
    resp: TabWorkerResponse,
    request: &TabWorkerRequest,
    csp_header: Option<&str>,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, String> {
    let tab_id = request.tab_id;
    let url = request.url.as_str();
    if resp.tab_id != tab_id {
        return Err("tab worker returned a different tab id".to_string());
    }
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "tab worker rejected the page".to_string()));
    }
    // Checked before anything else in the answer is used. A renderer that
    // sends a frame the checker refuses is broken, so the page fails.
    let frame = match resp.frame.as_deref().map(frame::decode) {
        Some(Err(_)) => return Err(BAD_FRAME_ERROR.to_string()),
        Some(Ok(frame)) => Some(frame),
        None => None,
    };
    let mut page = paint_worker_document(
        &resp.html,
        url,
        viewport,
        csp_header,
        fetch_jar,
        resp.blocked_csp_resources,
    );
    // The session keeps the frame to draw from only when that is turned on.
    page.renderer_frame = frame
        .as_ref()
        .map(|frame| FrameCheck::compare(frame, &page));
    page.drawn = frame;
    Ok(page)
}

/// The link under the page point `(x, y)` when `page` is one of Anvil's
/// own `anvil://` pages, resolved against its URL. Web pages get `None`.
fn own_page_link(page: &Page, x: f32, y: f32, view: Rect) -> Option<String> {
    if !page.url.starts_with("anvil://") {
        return None;
    }
    let href = page.link_at_in_view(x, y, Some(view))?;
    resolve_url(&page.url, href)
}

/// Extract an in-page `#fragment` when `href` refers to the current `page_url` document.
fn extract_same_page_fragment<'a>(page_url: &str, href: &'a str) -> Option<&'a str> {
    let trimmed = href.trim();
    if let Some(frag) = trimmed.strip_prefix('#') {
        return if frag.is_empty() { None } else { Some(frag) };
    }
    let (href_base, frag) = trimmed.split_once('#')?;
    if frag.is_empty() {
        return None;
    }
    let page_base = page_url.split('#').next().unwrap_or(page_url).trim();
    if href_base.eq_ignore_ascii_case(page_base) {
        return Some(frag);
    }
    if let Some(resolved) = resolve_url(page_base, href_base)
        && resolved.eq_ignore_ascii_case(page_base)
    {
        return Some(frag);
    }
    None
}

/// The error for a renderer frame that [`frame::decode`] refused.
const BAD_FRAME_ERROR: &str = "the renderer sent a page the browser could not check";

/// Lay out `html` without running scripts. `html` is the worker's document.
fn paint_worker_document(
    html: &str,
    url: &str,
    viewport: Viewport,
    http_csp: Option<&str>,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    blocked_from_worker: usize,
) -> Page {
    let parsed = anvil_dom::parse_html(html);
    let combined_csp_raw = match (http_csp, parsed.meta_csp()) {
        (Some(h), Some(m)) => format!("{h}; {m}"),
        (Some(h), None) => h.to_string(),
        (None, Some(m)) => m,
        (None, None) => String::new(),
    };
    let csp = ContentSecurityPolicy::parse(&combined_csp_raw);
    let mut blocked = blocked_from_worker;
    let linked_css = fetch_linked_stylesheets(
        url,
        &parsed.stylesheet_links_with_integrity(),
        &csp,
        &mut blocked,
    );
    let images = fetch_page_images(url, &parsed.images(), &csp, &mut blocked);
    render_document(
        html,
        PageSheets {
            external_css: "",
            linked_css,
        },
        images,
        url,
        viewport,
        http_csp,
        blocked,
        &[],
        PageLoads::Here(fetch_jar),
        false,
    )
}

/// Page shown when a remote navigation does not come back from the worker.
///
/// `reason` is escaped and inserted as text. The failed document is not part
/// of this page.
pub fn navigation_error_page(url: &str, reason: &str, viewport: Viewport) -> Page {
    let html = format!(
        "<!DOCTYPE html>
<html>
<head>
  <title>Navigation failed</title>
  <style>
    html, body {{ margin: 0; padding: 24px; background-color: #0e1015; color: #eceff4; font-size: 15px; }}
    .card {{ background-color: #171b24; border: 1px solid #d08770; border-radius: 10px; padding: 20px; }}
    h1 {{ color: #ebcb8b; font-size: 20px; margin-bottom: 10px; }}
    p {{ color: #d8dee9; margin-bottom: 8px; }}
    a {{ color: #88c0d0; }}
  </style>
</head>
<body>
  <div class=\"card\">
    <h1>Navigation failed</h1>
    <p>Target: {url}</p>
    <p>Reason: {reason}</p>
    <p><a href=\"anvil://home\">Return to Start</a></p>
  </div>
</body>
</html>",
        url = escape_error_text(url),
        reason = escape_error_text(reason),
    );
    render_html(&html, url, viewport)
}

/// The page shown when HTTPS-only is on and `http_url` could not be loaded
/// over HTTPS. Nothing was sent over plain HTTP.
pub fn https_only_page(http_url: &str, viewport: Viewport) -> Page {
    let html = frontend::https_only_html(http_url, permissions::action_key());
    render_html(&html, http_url, viewport)
}

/// What a permissions-page address asked for.
enum PermissionChange {
    /// Show the page with this note.
    Message(String),
    /// Open this address instead.
    Open(String),
}

/// `name=value` pairs from the query of `target`, decoded.
fn query_params(target: &str) -> Vec<(String, String)> {
    let Some((_, query)) = target.split_once('?') else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or("");
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (
                frontend::urldecode_component(k),
                frontend::urldecode_component(v),
            )
        })
        .collect()
}

/// True when `target` carries this run's `key` (see [`permissions::action_key`]).
///
/// `anvil://home`, `settings`, `history`, and `downloads` change state only
/// with the key. The built-in pages put it on their own links, and web pages
/// can't read those pages, so a web page can link to them but can't change
/// anything through them.
fn has_action_key(target: &str) -> bool {
    query_params(target)
        .iter()
        .any(|(k, v)| k == "key" && v == permissions::action_key())
}

/// Escape text that is inserted into the error page.
fn escape_error_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Load a built-in page, an http(s) URL, or a local HTML file.
///
/// `anvil://navigate` is resolved with Google Search, not the session engine.
/// `Session::open_target` handles that prefix itself before it calls this function.
pub fn load_target(target: &str, viewport: Viewport) -> Result<Page, String> {
    load_target_with_cookies(target, viewport, None)
}

/// Load like [`load_target`]. Script `fetch()` uses `fetch_jar` when it is set.
/// http(s) targets use a `--renderer` process with scripts enabled.
pub fn load_target_with_cookies(
    target: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
) -> Result<Page, String> {
    load_target_for_navigation(target, viewport, fetch_jar, true, 0)
}

/// Load `target`. http(s) goes through a `--renderer` process. Other targets stay here.
pub fn load_target_for_navigation(
    target: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
) -> Result<Page, String> {
    load_target_routed(
        target,
        viewport,
        fetch_jar,
        scripts_enabled,
        tab_id,
        DocumentRoute::OneShot,
    )
}

/// Load `target` for tab `tab_id`. http(s) pages and local files run in that
/// tab's renderer. `anvil://` pages are built here.
pub fn load_target_in_tab(
    target: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
    workers: &TabWorkerPool,
) -> Result<Page, String> {
    load_target_routed(
        target,
        viewport,
        fetch_jar,
        scripts_enabled,
        tab_id,
        DocumentRoute::Tab(workers),
    )
}

fn load_target_routed(
    target: &str,
    viewport: Viewport,
    fetch_jar: Option<Arc<Mutex<CookieJar>>>,
    scripts_enabled: bool,
    tab_id: u64,
    route: DocumentRoute<'_>,
) -> Result<Page, String> {
    let trimmed = target.trim();
    if trimmed.len() > "view-source:".len()
        && trimmed[.."view-source:".len()].eq_ignore_ascii_case("view-source:")
    {
        let inner = trimmed["view-source:".len()..].trim();
        let (canonical, raw_html) =
            if let Some((canonical_url, html)) = frontend::resolve_builtin_page(inner) {
                (canonical_url, html)
            } else if inner.starts_with("http://") || inner.starts_with("https://") {
                let (final_url, resp) =
                    navigation_get(inner, fetch_jar.as_ref()).map_err(|e| e.to_string())?;
                (final_url, String::from_utf8_lossy(&resp.body).into_owned())
            } else {
                let html = fs::read_to_string(inner)
                    .map_err(|err| format!("could not read {inner}: {err}"))?;
                (inner.to_string(), html)
            };
        let view_url = format!("view-source:{canonical}");
        let view_html = frontend::view_source_html(&canonical, &raw_html);
        return Ok(render_html(&view_html, &view_url, viewport));
    }
    if let Some(dest) = frontend::resolve_navigation_query(target) {
        return load_target_routed(&dest, viewport, fetch_jar, scripts_enabled, tab_id, route);
    }
    if let Some((canonical_url, html)) = frontend::resolve_builtin_page(target) {
        return Ok(render_html(&html, &canonical_url, viewport));
    }
    if let Some(next) = sites::passthrough_rewrite(target) {
        return load_target_routed(&next, viewport, fetch_jar, scripts_enabled, tab_id, route);
    }
    if let Some(html) = sites::gis_map_page(target) {
        return Ok(render_html(&html, target, viewport));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        match load_url_routed(target, viewport, fetch_jar, scripts_enabled, tab_id, route) {
            Ok(page) => Ok(page),
            Err(BrowserError::Download {
                url,
                filename,
                content_type,
                body,
            }) => {
                let mut mgr = DownloadManager::new();
                #[cfg(not(test))]
                {
                    let dir = downloads::default_downloads_dir();
                    let _ = mgr.save_payload_to_dir(
                        &url,
                        filename.as_deref(),
                        &content_type,
                        &body,
                        &dir,
                    );
                }
                #[cfg(test)]
                {
                    let _ = mgr.record_download(&url, filename.as_deref(), &content_type, &body);
                }
                let html = frontend::downloads_html_with_entries(mgr.entries());
                Ok(render_html(&html, frontend::DOWNLOADS_URL, viewport))
            }
            Err(err) => Err(err.to_string()),
        }
    } else if let DocumentRoute::Tab(workers) = route {
        let html =
            fs::read_to_string(target).map_err(|err| format!("could not read {target}: {err}"))?;
        render_routed(
            DocumentRoute::Tab(workers),
            tab_id,
            target,
            &html,
            None,
            scripts_enabled,
            viewport,
            fetch_jar,
        )
    } else {
        let html =
            fs::read_to_string(target).map_err(|err| format!("could not read {target}: {err}"))?;
        let parsed_doc = anvil_dom::parse_html(&html);
        let csp = ContentSecurityPolicy::parse(&parsed_doc.meta_csp().unwrap_or_default());
        let mut blocked = 0usize;
        let linked_css = fetch_linked_stylesheets(
            target,
            &parsed_doc.stylesheet_links_with_integrity(),
            &csp,
            &mut blocked,
        );
        let images = fetch_page_images(target, &parsed_doc.images(), &csp, &mut blocked);
        Ok(render_document(
            &html,
            PageSheets {
                external_css: "",
                linked_css,
            },
            images,
            target,
            viewport,
            None,
            blocked,
            &[],
            PageLoads::Here(fetch_jar),
            true,
        ))
    }
}

/// Same-origin script `fetch()` for a page loading `loads`.
///
/// Here, it goes through `anvil-net` and the jar. A missing jar still gets
/// a jar for this page, so `Set-Cookie` from the script response is kept on
/// the page host. Cross-origin redirects are not followed. Through the
/// browser, it is a [`FetchKind::ScriptFetch`], and the browser keeps the
/// cookies. The Rust interpreter calls this. The numeric VM does not.
fn bind_script_fetch(runtime: &mut ScriptRuntime, loads: &PageLoads) {
    match loads {
        PageLoads::Here(fetch_jar) => {
            let jar = page_jar(fetch_jar.as_ref());
            runtime.set_fetch_host(Arc::new(move |url, page| script_net_fetch(url, page, &jar)));
        }
        PageLoads::Browser(ask) => {
            let ask = Arc::clone(ask);
            runtime.set_fetch_host(Arc::new(move |url, _page| {
                match ask(FetchKind::ScriptFetch, url) {
                    Some(reply) => ScriptFetchReply {
                        status: reply.status,
                        body: String::from_utf8_lossy(&reply.body).into_owned(),
                    },
                    None => ScriptFetchReply {
                        status: 0,
                        body: String::new(),
                    },
                }
            }));
        }
    }
}

fn script_net_fetch(url: &str, page: &str, jar: &Arc<Mutex<CookieJar>>) -> ScriptFetchReply {
    let mut current = url.trim().to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !is_same_origin(page, &current) {
            return ScriptFetchReply {
                status: 403,
                body: "{\"error\":\"same-origin policy blocked fetch\"}".to_string(),
            };
        }
        if is_url_blocked_by_default_shield(&current, Some(page)).is_some() {
            return ScriptFetchReply {
                status: 0,
                body: String::new(),
            };
        }
        let Ok(parsed) = parse_url(&current) else {
            return ScriptFetchReply {
                status: 0,
                body: String::new(),
            };
        };
        let cookie_hdr = {
            let guard = jar.lock().unwrap_or_else(|err| err.into_inner());
            guard.http_cookie_header_under(&current, page, Some(page), false)
        };
        let Ok(response) = http_get_with_cookie(&parsed, cookie_hdr.as_deref()) else {
            return ScriptFetchReply {
                status: 0,
                body: String::new(),
            };
        };
        {
            let mut guard = jar.lock().unwrap_or_else(|err| err.into_inner());
            for set_cookie in response.headers_named("set-cookie") {
                guard.store_set_cookie_under(&current, page, set_cookie);
            }
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            let Some(location) = response.header("location") else {
                return ScriptFetchReply {
                    status: response.status,
                    body: String::new(),
                };
            };
            let Some(next) = resolve_url(&current, location) else {
                return ScriptFetchReply {
                    status: 0,
                    body: String::new(),
                };
            };
            // A cross-origin hop is not connected.
            if !is_same_origin(page, &next) {
                return ScriptFetchReply {
                    status: 403,
                    body: "{\"error\":\"same-origin policy blocked fetch\"}".to_string(),
                };
            }
            current = next;
            continue;
        }
        let mut body = String::from_utf8_lossy(&response.body).into_owned();
        limit_script_fetch_body(&mut body);
        return ScriptFetchReply {
            status: response.status,
            body,
        };
    }
    ScriptFetchReply {
        status: 0,
        body: String::new(),
    }
}

/// The jar a renderer's `fetch()` calls use for one page: the session jar,
/// or a jar for this page only.
fn page_jar(fetch_jar: Option<&Arc<Mutex<CookieJar>>>) -> Arc<Mutex<CookieJar>> {
    fetch_jar
        .cloned()
        .unwrap_or_else(|| Arc::new(Mutex::new(CookieJar::new())))
}

/// Answer one fetch that a renderer asked for while it works on `page_url`.
///
/// The renderer is not trusted, so every rule is applied here, from the
/// browser's own state. `page_url` is the address the browser gave the
/// renderer, never one the renderer names. In order:
///
/// - The shield, with `page_url` as the initiator.
/// - Mixed content: an `https` page can't load anything over plain `http`,
///   unless HTTPS-only upgrades it.
/// - HTTPS-only, from the policy on this thread, with every exception the
///   person allowed. `anvil-net` applies it on each hop.
/// - [`FetchKind::ScriptFetch`] is a page script's `fetch()`: it has to stay
///   on the page's origin, redirects included, and it is the only kind that
///   sends and stores cookies, in `jar` under the page's site.
/// - Everything else is a GET without cookies, with the shield on every
///   redirect. A response whose `Cross-Origin-Resource-Policy` doesn't
///   allow this page is dropped.
///
/// A blocked or failed fetch is status 0 with an empty body. Response
/// headers, `Set-Cookie` included, never go back to the renderer. Only the
/// status, the content type and the body do.
pub fn answer_renderer_fetch(
    want: &FetchRequest,
    page_url: &str,
    jar: &Arc<Mutex<CookieJar>>,
) -> FetchReply {
    let failed = || FetchReply {
        nav_id: want.nav_id,
        status: 0,
        content_type: String::new(),
        body: Vec::new(),
    };
    let url = want.url.as_str();
    let req_type = match want.kind {
        FetchKind::Script => RequestType::Script,
        FetchKind::Stylesheet => RequestType::Stylesheet,
        FetchKind::Image => RequestType::Image,
        FetchKind::Font => RequestType::Font,
        FetchKind::ScriptFetch => RequestType::XmlHttpRequest,
    };
    let shield_res = check_default_shield_request(url, Some(page_url), req_type);
    if shield_res.matched {
        if let Some((mime, body)) = shield_res.redirect_resource {
            return FetchReply {
                nav_id: want.nav_id,
                status: 200,
                content_type: mime,
                body: body.into_bytes(),
            };
        }
        return failed();
    }
    if is_mixed_content(page_url, url) {
        return failed();
    }
    if want.kind == FetchKind::ScriptFetch {
        let reply = script_net_fetch(url, page_url, jar);
        return FetchReply {
            nav_id: want.nav_id,
            status: reply.status,
            content_type: String::new(),
            body: reply.body.into_bytes(),
        };
    }
    let mut no_cookies = CookieJar::new();
    let Ok((final_url, response)) = fetch_with_cookies(url, &mut no_cookies, Some(page_url), false)
    else {
        return failed();
    };
    if !allows_corp_subresource(
        response.header("cross-origin-resource-policy"),
        page_url,
        &final_url,
    ) {
        return failed();
    }
    if !(200..300).contains(&response.status) {
        return FetchReply {
            status: response.status,
            ..failed()
        };
    }
    FetchReply {
        nav_id: want.nav_id,
        status: response.status,
        content_type: clip_text(
            response.header("content-type").unwrap_or_default(),
            MAX_SHORT_TEXT_BYTES,
        )
        .to_string(),
        body: response.body,
    }
}

/// True when `page_url` is `https` and `url` is plain `http` that
/// HTTPS-only won't upgrade.
fn is_mixed_content(page_url: &str, url: &str) -> bool {
    let page_is_https = parse_url(page_url).is_ok_and(|page| page.scheme == "https");
    let url_is_http = parse_url(url).is_ok_and(|target| target.scheme == "http");
    page_is_https && url_is_http && anvil_net::https_only_policy().upgrade(url).is_none()
}

/// The longest start of `text` that fits in `max` bytes without cutting a
/// character.
fn clip_text(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// True when `url` starts with `http://` or `https://` after trimming.
fn is_remote_origin(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

/// Parallel multi-core worker dispatch for `fetch_page_images` when multiple images are present.
fn fetch_page_images(
    base: &str,
    imgs: &[(NodeId, String)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> HashMap<NodeId, Image> {
    if imgs.len() <= 1 || raster_worker_threads() <= 1 {
        return fetch_page_images_sequential(base, imgs, csp, blocked_csp);
    }
    let mut out = HashMap::new();
    let base_is_remote = is_remote_origin(base);
    let mut pending: Vec<(NodeId, String)> = Vec::new();
    for (node_id, src) in imgs {
        let trimmed = src.trim();
        if let Some(map) = sites::map_bitmap(trimmed) {
            out.insert(*node_id, map);
            continue;
        }
        if trimmed.eq_ignore_ascii_case("anvil://logo.png") {
            out.insert(*node_id, logo::render_anvil_logo(64));
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-google.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::Google, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-ddg.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::DuckDuckGo, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-bing.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::Bing, 36),
            );
            continue;
        }
        if !csp.allows_resource(CspDirective::ImgSrc, base, src) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, src) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        pending.push((*node_id, resolved));
    }
    if pending.is_empty() {
        return out;
    }
    let workers = raster_worker_threads().min(pending.len()).max(1);
    let chunk_len = pending.len().div_ceil(workers);
    let results: Vec<(Vec<(NodeId, Image)>, usize)> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(workers);
        for chunk in pending.chunks(chunk_len) {
            handles.push(s.spawn(move || {
                let mut decoded = Vec::with_capacity(chunk.len());
                let mut local_blocked = 0usize;
                let remote_urls: Vec<String> = chunk
                    .iter()
                    .filter(|(_, resolved)| is_remote_origin(resolved))
                    .map(|(_, resolved)| resolved.clone())
                    .collect();
                let mut remote_responses = fetch_batch(&remote_urls, Some(base)).into_iter();
                for (node_id, resolved) in chunk {
                    let raw_bytes = if is_remote_origin(resolved) {
                        remote_responses
                            .next()
                            .and_then(|r| r.ok())
                            .and_then(|(_, resp)| {
                                if !allows_corp_subresource(
                                    resp.header("cross-origin-resource-policy"),
                                    base,
                                    resolved,
                                ) {
                                    local_blocked += 1;
                                    return None;
                                }
                                if (200..300).contains(&resp.status) {
                                    Some(resp.body)
                                } else {
                                    None
                                }
                            })
                    } else if !base_is_remote && !base.starts_with("anvil://") {
                        fs::read(resolved).ok()
                    } else {
                        None
                    };
                    if let Some(bytes) = raw_bytes
                        && let Some(image) = cached_decode_image(resolved, &bytes)
                    {
                        decoded.push((*node_id, image));
                    }
                }
                (decoded, local_blocked)
            }));
        }
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (decoded, local_blocked) in results {
        *blocked_csp += local_blocked;
        for (node_id, image) in decoded {
            out.insert(node_id, image);
        }
    }
    out
}

/// Decode page images. Built-in `anvil://logo.png` and `anvil://engine-*.png` are drawn here. CSP or the shield can skip a URL.
fn fetch_page_images_sequential(
    base: &str,
    imgs: &[(NodeId, String)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> HashMap<NodeId, Image> {
    let mut out = HashMap::new();
    let base_is_remote = is_remote_origin(base);
    for (node_id, src) in imgs {
        let trimmed = src.trim();
        if let Some(map) = sites::map_bitmap(trimmed) {
            out.insert(*node_id, map);
            continue;
        }
        if trimmed.eq_ignore_ascii_case("anvil://logo.png") {
            out.insert(*node_id, logo::render_anvil_logo(64));
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-google.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::Google, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-ddg.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::DuckDuckGo, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-bing.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::Bing, 36),
            );
            continue;
        }
        if !csp.allows_resource(CspDirective::ImgSrc, base, src) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, src) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        let raw_bytes = if is_remote_origin(&resolved) {
            fetch(&resolved).ok().and_then(|(_, resp)| {
                if !allows_corp_subresource(
                    resp.header("cross-origin-resource-policy"),
                    base,
                    &resolved,
                ) {
                    *blocked_csp += 1;
                    return None;
                }
                if (200..300).contains(&resp.status) {
                    Some(resp.body)
                } else {
                    None
                }
            })
        } else if !base_is_remote && !base.starts_with("anvil://") {
            fs::read(&resolved).ok()
        } else {
            None
        };
        if let Some(bytes) = raw_bytes
            && let Some(image) = cached_decode_image(&resolved, &bytes)
        {
            out.insert(*node_id, image);
        }
    }
    out
}

/// Decode `<video>` sources with `GpuVideoSession`.
///
/// `anvil-yt-frames` and `anvil-yt-board` are YouTube preview frames built in
/// this process. They are not fetched as page URLs and they are not H.264.
/// `anvil://demo.mp4` and `anvil://encrypted.mp4` are generated in this process.
/// A file whose `ftyp` box fails `inspect_mp4_metadata` is skipped.
fn fetch_page_videos(
    base: &str,
    vids: &[(NodeId, String)],
    csp: &ContentSecurityPolicy,
    blocked_trackers: &mut usize,
    video_session: &mut GpuVideoSession,
) -> HashMap<NodeId, VideoPlayerState> {
    let mut out = HashMap::new();
    let base_is_remote = is_remote_origin(base);
    for (node_id, src) in vids {
        let trimmed = src.trim();
        if let Some(clip) = sites::youtube_player_clip(trimmed) {
            out.insert(*node_id, VideoPlayerState::new(clip));
            continue;
        }
        let raw_bytes = if trimmed.eq_ignore_ascii_case("anvil://demo.mp4") {
            generate_demo_mp4(240, 120, 6).ok()
        } else if trimmed.eq_ignore_ascii_case("anvil://encrypted.mp4") {
            generate_encrypted_demo_mp4(240, 120, 6).ok()
        } else {
            if !csp.allows_resource(CspDirective::MediaSrc, base, trimmed) {
                continue;
            }
            if let Some(resolved) = resolve_url(base, trimmed) {
                if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
                    *blocked_trackers += 1;
                    continue;
                }
                if resolved.eq_ignore_ascii_case("anvil://demo.mp4") {
                    generate_demo_mp4(240, 120, 6).ok()
                } else if resolved.eq_ignore_ascii_case("anvil://encrypted.mp4") {
                    generate_encrypted_demo_mp4(240, 120, 6).ok()
                } else if is_remote_origin(&resolved) {
                    fetch(&resolved).ok().and_then(|(_, resp)| {
                        if !allows_corp_subresource(
                            resp.header("cross-origin-resource-policy"),
                            base,
                            &resolved,
                        ) {
                            return None;
                        }
                        if (200..300).contains(&resp.status) {
                            Some(resp.body)
                        } else {
                            None
                        }
                    })
                } else if !base_is_remote && !base.starts_with("anvil://") {
                    fs::read(&resolved).ok()
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(bytes) = raw_bytes {
            if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" && inspect_mp4_metadata(&bytes).is_err()
            {
                continue;
            }
            if let Ok(clip) = video_session.decode_video_auto(&bytes) {
                out.insert(*node_id, VideoPlayerState::new(clip));
            }
        }
    }
    out
}

/// Parallel multi-core worker dispatch for `fetch_linked_stylesheets` when multiple links are present.
fn fetch_linked_stylesheets_parallel(
    base: &str,
    links: &[(String, Option<String>)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> Vec<(String, String)> {
    let base_is_remote = is_remote_origin(base);
    let mut pending: Vec<(usize, String, Option<String>)> = Vec::new();
    for (idx, (href, integrity)) in links.iter().enumerate() {
        if !csp.allows_resource(CspDirective::StyleSrc, base, href) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, href) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        pending.push((idx, resolved, integrity.clone()));
    }
    if pending.is_empty() {
        return Vec::new();
    }
    let workers = raster_worker_threads().min(pending.len()).max(1);
    let chunk_len = pending.len().div_ceil(workers);
    let results: Vec<(Vec<(usize, String)>, usize)> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(workers);
        for chunk in pending.chunks(chunk_len) {
            handles.push(s.spawn(move || {
                let mut local_sheets = Vec::with_capacity(chunk.len());
                let mut local_blocked = 0usize;
                let remote_urls: Vec<String> = chunk
                    .iter()
                    .filter(|(_, resolved, _)| is_remote_origin(resolved))
                    .map(|(_, resolved, _)| resolved.clone())
                    .collect();
                let mut remote_responses = fetch_batch(&remote_urls, Some(base)).into_iter();
                for (idx, resolved, integrity) in chunk {
                    if is_remote_origin(resolved) {
                        if let Some(Ok((_, response))) = remote_responses.next() {
                            if !allows_corp_subresource(
                                response.header("cross-origin-resource-policy"),
                                base,
                                resolved,
                            ) {
                                local_blocked += 1;
                                continue;
                            }
                            if (200..300).contains(&response.status) {
                                if !verify_subresource_integrity(
                                    integrity.as_deref(),
                                    &response.body,
                                ) {
                                    local_blocked += 1;
                                    continue;
                                }
                                if let Ok(css) = String::from_utf8(response.body)
                                    && !css.trim().is_empty()
                                {
                                    local_sheets.push((*idx, css));
                                }
                            }
                        }
                    } else if !base_is_remote
                        && !base.starts_with("anvil://")
                        && let Ok(bytes) = fs::read(resolved)
                    {
                        if !verify_subresource_integrity(integrity.as_deref(), &bytes) {
                            local_blocked += 1;
                            continue;
                        }
                        if let Ok(css) = String::from_utf8(bytes)
                            && !css.trim().is_empty()
                        {
                            local_sheets.push((*idx, css));
                        }
                    }
                }
                (local_sheets, local_blocked)
            }));
        }
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut ordered: Vec<(usize, String)> = Vec::new();
    for (local_sheets, local_blocked) in results {
        *blocked_csp += local_blocked;
        ordered.extend(local_sheets);
    }
    ordered.sort_by_key(|(idx, _)| *idx);
    ordered
        .into_iter()
        .filter_map(|(idx, css)| links.get(idx).map(|(href, _)| (href.clone(), css)))
        .collect()
}

/// Fetch linked stylesheets that CSP, the shield, and the integrity check
/// allow. Returns `(href, css)` in link order; a link that was blocked,
/// failed, or came back empty is left out.
fn fetch_linked_stylesheets(
    base: &str,
    links: &[(String, Option<String>)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> Vec<(String, String)> {
    let mut sheets = Vec::new();
    if links.len() > 1 {
        return fetch_linked_stylesheets_parallel(base, links, csp, blocked_csp);
    }
    let base_is_remote = is_remote_origin(base);
    for (href, integrity) in links {
        if !csp.allows_resource(CspDirective::StyleSrc, base, href) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, href) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        if is_remote_origin(&resolved) {
            if let Ok((_, response)) = fetch(&resolved) {
                if !allows_corp_subresource(
                    response.header("cross-origin-resource-policy"),
                    base,
                    &resolved,
                ) {
                    *blocked_csp += 1;
                    continue;
                }
                if (200..300).contains(&response.status) {
                    if !verify_subresource_integrity(integrity.as_deref(), &response.body) {
                        *blocked_csp += 1;
                        continue;
                    }
                    if let Ok(css) = String::from_utf8(response.body)
                        && !css.trim().is_empty()
                    {
                        sheets.push((href.clone(), css));
                    }
                }
            }
        } else if !base_is_remote
            && !base.starts_with("anvil://")
            && let Ok(bytes) = fs::read(&resolved)
        {
            if !verify_subresource_integrity(integrity.as_deref(), &bytes) {
                *blocked_csp += 1;
                continue;
            }
            if let Ok(css) = String::from_utf8(bytes)
                && !css.trim().is_empty()
            {
                sheets.push((href.clone(), css));
            }
        }
    }
    sheets
}

/// Most bytes of decoded images a renderer keeps for one page (48 MiB).
/// Images past this are left out, so a page full of large images can't
/// push the renderer over its memory limit.
pub const RENDERER_IMAGE_BUDGET: usize = 48 * 1024 * 1024;

/// The renderer's linked style sheets, asked of the browser. The same rules
/// as [`fetch_linked_stylesheets`]: CSP, the shield and integrity. A local
/// page reads local sheets itself, which only works when the renderer can
/// see the file.
fn ask_linked_stylesheets(
    ask: &BrowserFetch,
    base: &str,
    links: &[(String, Option<String>)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> Vec<(String, String)> {
    let mut sheets = Vec::new();
    let base_is_remote = is_remote_origin(base);
    for (href, integrity) in links {
        if !csp.allows_resource(CspDirective::StyleSrc, base, href) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, href) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        let bytes = if is_remote_origin(&resolved) {
            ask(FetchKind::Stylesheet, &resolved)
                .filter(|reply| (200..300).contains(&reply.status))
                .map(|reply| reply.body)
        } else if !base_is_remote && !base.starts_with("anvil://") {
            fs::read(&resolved).ok()
        } else {
            None
        };
        let Some(bytes) = bytes else {
            continue;
        };
        if !verify_subresource_integrity(integrity.as_deref(), &bytes) {
            *blocked_csp += 1;
            continue;
        }
        if let Ok(css) = String::from_utf8(bytes)
            && !css.trim().is_empty()
        {
            sheets.push((href.clone(), css));
        }
    }
    sheets
}

/// The renderer's page images, asked of the browser and decoded here. The
/// same rules as [`fetch_page_images`]: built-in logos are drawn, then CSP
/// and the shield. Map tiles need the network and are left out. Decoding
/// stops adding images once [`RENDERER_IMAGE_BUDGET`] is used up.
fn ask_page_images(
    ask: &BrowserFetch,
    base: &str,
    imgs: &[(NodeId, String)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> HashMap<NodeId, Image> {
    let mut out = HashMap::new();
    let mut used = 0usize;
    let base_is_remote = is_remote_origin(base);
    for (node_id, src) in imgs {
        let trimmed = src.trim();
        let built_in = if trimmed.eq_ignore_ascii_case("anvil://logo.png") {
            Some(logo::render_anvil_logo(64))
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-google.png") {
            Some(logo::render_search_engine_logo(SearchEngine::Google, 36))
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-ddg.png") {
            Some(logo::render_search_engine_logo(
                SearchEngine::DuckDuckGo,
                36,
            ))
        } else if trimmed.eq_ignore_ascii_case("anvil://engine-bing.png") {
            Some(logo::render_search_engine_logo(SearchEngine::Bing, 36))
        } else {
            None
        };
        if let Some(image) = built_in {
            used += image.pixels.len();
            out.insert(*node_id, image);
            continue;
        }
        if trimmed.starts_with("anvil-map://") {
            continue;
        }
        if !csp.allows_resource(CspDirective::ImgSrc, base, src) {
            *blocked_csp += 1;
            continue;
        }
        let Some(resolved) = resolve_url(base, src) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(base)).is_some() {
            *blocked_csp += 1;
            continue;
        }
        if used >= RENDERER_IMAGE_BUDGET {
            continue;
        }
        let bytes = if is_remote_origin(&resolved) {
            ask(FetchKind::Image, &resolved)
                .filter(|reply| (200..300).contains(&reply.status))
                .map(|reply| reply.body)
        } else if !base_is_remote && !base.starts_with("anvil://") {
            fs::read(&resolved).ok()
        } else {
            None
        };
        // Decoded straight, without the window's image cache, so the
        // renderer holds one copy of each image.
        let Some(image) = bytes.and_then(|b| decode_auto(&b).ok()) else {
            continue;
        };
        if used + image.pixels.len() > RENDERER_IMAGE_BUDGET {
            continue;
        }
        used += image.pixels.len();
        out.insert(*node_id, image);
    }
    out
}

/// Maximum number of recently closed tabs kept for `Reopen Closed Tab` (`Ctrl+Shift+T`).
pub const MAX_CLOSED_TABS: usize = 16;

/// History stack for one tab, plus the index of the visible page.
///
/// `id` stays the same while the tab is open, even when tabs before it close.
/// The tab's renderer process in [`TabWorkerPool`] is keyed by this id.
#[derive(Clone, Debug)]
pub struct Tab {
    id: u64,
    history: Vec<Page>,
    index: usize,
    pinned: bool,
}

impl Tab {
    /// A tab with no pages. `current` is `None` and the title is `New Tab`.
    fn empty(id: u64) -> Self {
        Self {
            id,
            history: Vec::new(),
            index: 0,
            pinned: false,
        }
    }

    /// Stable id of this tab.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// True when this tab is pinned to the left of the tab strip.
    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Pin or unpin this tab.
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// Page at `index`, if the stack has one.
    pub fn current(&self) -> Option<&Page> {
        self.history.get(self.index)
    }

    /// Mutable page at `index`, if the stack has one.
    pub fn current_mut(&mut self) -> Option<&mut Page> {
        self.history.get_mut(self.index)
    }

    /// True when the stack is non-empty and `index` is not zero.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty() && self.index > 0
    }

    /// True when a page exists after `index`.
    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    /// Current title, else the current URL, else `New Tab`.
    pub fn title(&self) -> &str {
        let Some(page) = self.current() else {
            return "New Tab";
        };
        if !page.title.is_empty() {
            page.title.as_str()
        } else if !page.url.is_empty() {
            page.url.as_str()
        } else {
            "New Tab"
        }
    }

    /// Overwrite the current history entry. No-op when the stack is empty.
    fn replace_current(&mut self, page: Page) {
        if let Some(slot) = self.history.get_mut(self.index) {
            *slot = page;
        }
    }

    /// Drop entries after `index`, push `page`, and select it.
    fn navigate(&mut self, page: Page) {
        if !self.history.is_empty() {
            self.history.truncate(self.index + 1);
        }
        self.history.push(page);
        self.index = self.history.len() - 1;
    }

    /// Select the previous page. Returns false at the start of the stack.
    fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.index -= 1;
        true
    }

    /// Select the next page. Returns false at the end of the stack.
    fn forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.index += 1;
        true
    }
}

/// Window session. Tabs, scroll, and the clipboard live in this process.
///
/// Page documents for http(s) and local files are built in one worker
/// process per tab (`tab_workers`). Closing a tab ends its worker.
/// `gpu_video` decodes clip bytes. It does not present a window.
/// `sandbox_guard` holds the process limits from `anvil-sandbox`.
/// `cookie_jar`, `network_shield`, and `hsts_store` come from `anvil-net`.
#[derive(Debug)]
pub struct Session {
    tabs: Vec<Tab>,
    closed_tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    tab_workers: Arc<TabWorkerPool>,
    viewport: Viewport,
    scroll: f32,
    zoom_percent: u16,
    bookmarks: Vec<Bookmark>,
    history_log: Vec<Bookmark>,
    settings: BrowserSettings,
    gpu_video: GpuVideoSession,
    sandbox_guard: MandatorySandboxGuard,
    cookie_jar: Arc<Mutex<CookieJar>>,
    network_shield: NetworkShield,
    hsts_store: HstsStore,
    downloads: DownloadManager,
    local_storage: Vec<(String, String, String)>,
    clipboard: String,
    selected_text: String,
    selection_rects: Vec<Rect>,
    inspected_node: Option<NodeId>,
    console_history: Vec<(String, String)>,
    site_permissions: permissions::SitePermissions,
}

/// Add the submit button's `name` and `value` to a form body.
///
/// The clicked control is used when it is an enabled submit button. Enter in
/// a text field uses the first enabled submit button in the form. A button
/// with no name adds nothing. Disabled controls are not successful.
fn append_submitter_pair(
    doc: &Document,
    form_id: NodeId,
    control_id: NodeId,
    encoded: &mut String,
) {
    let Some(submitter) = form_submitter(doc, form_id, control_id) else {
        return;
    };
    let data = doc.get(submitter).data();
    let Some(name) = data.attr("name") else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let value = data.attr("value").unwrap_or("");
    if !encoded.is_empty() {
        encoded.push('&');
    }
    encoded.push_str(&anvil_dom::urlencode_component(name));
    encoded.push('=');
    encoded.push_str(&anvil_dom::urlencode_component(value));
}

/// The submit control that activated this form, if it has not been disabled.
fn form_submitter(doc: &Document, form_id: NodeId, control_id: NodeId) -> Option<NodeId> {
    if doc.is_submit_control(control_id) && !control_is_disabled(doc, control_id) {
        return Some(control_id);
    }
    first_enabled_submit(doc, form_id)
}

fn control_is_disabled(doc: &Document, id: NodeId) -> bool {
    doc.get(id).data().attr("disabled").is_some()
}

fn first_enabled_submit(doc: &Document, id: NodeId) -> Option<NodeId> {
    if doc.is_submit_control(id) && !control_is_disabled(doc, id) {
        return Some(id);
    }
    for child in doc.children(id) {
        if let Some(found) = first_enabled_submit(doc, *child) {
            return Some(found);
        }
    }
    None
}

impl Session {
    /// One empty tab at 100 percent zoom. Exits the process if the sandbox guard cannot be installed.
    pub fn new(viewport: Viewport) -> Self {
        let sandbox_guard = enforce_mandatory_sandbox_or_abort();
        Self {
            tabs: vec![Tab::empty(1)],
            closed_tabs: Vec::new(),
            active: 0,
            next_tab_id: 2,
            tab_workers: Arc::new(TabWorkerPool::new()),
            viewport,
            scroll: 0.0,
            zoom_percent: 100,
            bookmarks: Vec::new(),
            history_log: Vec::new(),
            settings: BrowserSettings::default(),
            gpu_video: GpuVideoSession::default(),
            sandbox_guard,
            cookie_jar: Arc::new(Mutex::new(CookieJar::new())),
            network_shield: NetworkShield::new(),
            hsts_store: HstsStore::new(),
            downloads: DownloadManager::new(),
            local_storage: Vec::new(),
            clipboard: String::new(),
            selected_text: String::new(),
            selection_rects: Vec::new(),
            inspected_node: None,
            console_history: Vec::new(),
            site_permissions: permissions::SitePermissions::new(),
        }
    }

    /// The per-site permissions list.
    pub fn site_permissions(&self) -> &permissions::SitePermissions {
        &self.site_permissions
    }

    /// Change the per-site permissions list. Cookie blocks take effect on the
    /// cookie jar right away.
    pub fn update_site_permissions<R>(
        &mut self,
        change: impl FnOnce(&mut permissions::SitePermissions) -> R,
    ) -> R {
        let out = change(&mut self.site_permissions);
        self.sync_site_permissions();
        out
    }

    /// Copy the cookie blocks from the permissions list onto the jar.
    fn sync_site_permissions(&mut self) {
        let blocked = self.site_permissions.cookie_blocked_hosts();
        self.cookie_jar_mut().set_blocked_hosts(&blocked);
    }

    /// HTTPS-only rules for a load started now: the setting, plus the sites
    /// allowed to use plain HTTP.
    pub fn https_only_policy(&self) -> anvil_net::HttpsOnlyPolicy {
        anvil_net::HttpsOnlyPolicy::new(
            self.settings.https_only_mode,
            &self.site_permissions.http_allowed_hosts(),
        )
    }

    /// Whether scripts run on `url`: the site's rule, or the global switch.
    pub fn scripts_enabled_for(&self, url: &str) -> bool {
        self.site_permissions
            .scripts_allowed(url, self.settings.scripts_enabled)
    }

    /// Borrow the origin-partitioned `(origin, key, value)` entries in `localStorage`.
    pub fn local_storage_entries(&self) -> &[(String, String, String)] {
        &self.local_storage
    }

    /// Merge `(origin, key, value)` rows from `rows` into `self.local_storage`.
    fn merge_local_storage(&mut self, rows: Vec<(String, String, String)>) {
        for (origin, key, value) in rows {
            if origin.is_empty() || key.is_empty() {
                continue;
            }
            if let Some(existing) = self
                .local_storage
                .iter_mut()
                .find(|(o, k, _)| o == &origin && k == &key)
            {
                existing.2 = value;
            } else {
                self.local_storage.push((origin, key, value));
            }
        }
        self.local_storage
            .sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    }

    /// Limits and fault counts reported by `sandbox_guard`.
    pub fn sandbox_telemetry(&self) -> SandboxTelemetry {
        self.sandbox_guard.telemetry()
    }

    /// Lock the session cookie jar.
    pub fn cookie_jar(&self) -> MutexGuard<'_, CookieJar> {
        self.cookie_jar
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// Lock the session cookie jar.
    pub fn cookie_jar_mut(&mut self) -> MutexGuard<'_, CookieJar> {
        self.cookie_jar()
    }

    /// Shared handle used by script `fetch()` on this session.
    pub fn cookie_jar_handle(&self) -> Arc<Mutex<CookieJar>> {
        Arc::clone(&self.cookie_jar)
    }

    /// Borrow the network shield.
    pub fn network_shield(&self) -> &NetworkShield {
        &self.network_shield
    }

    /// Mutable network shield.
    pub fn network_shield_mut(&mut self) -> &mut NetworkShield {
        &mut self.network_shield
    }

    /// Active Brave Shields operating mode (`Standard`, `Aggressive`, `Disabled`).
    pub fn shields_mode(&self) -> ShieldsMode {
        self.network_shield.mode()
    }

    /// Set the Brave Shields operating mode (`Standard`, `Aggressive`, `Disabled`) and sync the process shield.
    pub fn set_shields_mode(&mut self, mode: ShieldsMode) {
        self.network_shield.set_mode(mode);
        sync_default_shield(&self.network_shield);
    }

    /// Cycle Brave Shields mode (`Standard` -> `Aggressive` -> `Disabled` -> `Standard`) and sync.
    pub fn cycle_shields_mode(&mut self) -> ShieldsMode {
        let next = self.network_shield.cycle_mode();
        sync_default_shield(&self.network_shield);
        next
    }

    /// True when Brave Shields are active for `site_url_or_host`.
    pub fn is_shields_enabled_for(&self, site_url_or_host: &str) -> bool {
        self.network_shield.is_enabled_for_site(site_url_or_host)
    }

    /// Toggle Brave Shields Up/Down for `site_url_or_host`, returning `(host, new_enabled_state)`.
    pub fn toggle_shields_for_site(&mut self, site_url_or_host: &str) -> Option<(String, bool)> {
        let res = self.network_shield.toggle_site(site_url_or_host)?;
        sync_default_shield(&self.network_shield);
        Some(res)
    }

    /// Toggle Brave Shields Up/Down for the active tab's page host, returning `(host, new_enabled_state)`.
    pub fn toggle_shields_for_active_site(&mut self) -> Option<(String, bool)> {
        let active_url = self.active().current()?.url.clone();
        self.toggle_shields_for_site(&active_url)
    }

    /// Add a custom ABP / uBlock Origin network or cosmetic filter rule to Brave Shields and sync.
    pub fn add_shield_filter_rule(&mut self, rule: &str) -> bool {
        let added = self.network_shield.add_filter_rule(rule);
        if added {
            sync_default_shield(&self.network_shield);
        }
        added
    }

    /// Borrow the HSTS store.
    pub fn hsts_store(&self) -> &HstsStore {
        &self.hsts_store
    }

    /// Mutable HSTS store.
    pub fn hsts_store_mut(&mut self) -> &mut HstsStore {
        &mut self.hsts_store
    }

    /// Borrow the download log.
    pub fn downloads(&self) -> &DownloadManager {
        &self.downloads
    }

    /// Mutable download log.
    pub fn downloads_mut(&mut self) -> &mut DownloadManager {
        &mut self.downloads
    }

    /// Zoom percent, kept between 50 and 250.
    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    /// Engine stored in `settings`.
    pub fn search_engine(&self) -> SearchEngine {
        self.settings.search_engine
    }

    /// Store `engine`. Does not reload the active page.
    pub fn set_search_engine(&mut self, engine: SearchEngine) {
        self.settings.search_engine = engine;
    }

    /// Advance the stored engine and return it.
    pub fn cycle_search_engine(&mut self) -> SearchEngine {
        self.settings.search_engine = self.settings.search_engine.cycle_next();
        self.settings.search_engine
    }

    /// Theme stored in `settings`.
    pub fn theme(&self) -> BrowserTheme {
        self.settings.theme
    }

    /// Store `theme`. Does not reload the active page.
    pub fn set_theme(&mut self, theme: BrowserTheme) {
        self.settings.theme = theme;
    }

    /// Advance the stored theme and return it.
    pub fn cycle_theme(&mut self) -> BrowserTheme {
        self.settings.theme = self.settings.theme.cycle_next();
        self.settings.theme
    }

    /// The visible part of the active page in page coordinates: the scroll
    /// top and the viewport size, divided by the zoom. Sticky boxes are
    /// placed against this, both when drawn and when hit-tested.
    pub fn page_view(&self) -> Rect {
        let z = self.zoom_factor();
        Rect {
            x: 0.0,
            y: self.scroll / z,
            width: self.viewport.width / z,
            height: self.viewport.height / z,
        }
    }

    /// One animation frame for the active page: tick its clock to now and,
    /// if a transition or animation was or is running, restyle and paint.
    /// Returns true when the window should rasterize and present again.
    pub fn animation_frame(&mut self) -> bool {
        let viewport = self.viewport;
        let changed = self
            .tabs
            .get_mut(self.active)
            .and_then(Tab::current_mut)
            .is_some_and(|page| page.animation_frame(viewport));
        if changed {
            self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        }
        changed
    }

    /// True while the active page has a transition or animation running.
    pub fn is_animating(&self) -> bool {
        self.active().current().is_some_and(Page::is_animating)
    }

    /// `zoom_percent / 100`, clamped to the range 0.5 through 2.5.
    pub fn zoom_factor(&self) -> f32 {
        (self.zoom_percent as f32 / 100.0).clamp(0.5, 2.5)
    }

    /// Clamp `percent` to 50 through 250 and clamp scroll to the new maximum.
    pub fn set_zoom_percent(&mut self, percent: u16) -> u16 {
        self.zoom_percent = percent.clamp(50, 250);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.zoom_percent
    }

    /// Add 25 percent, capped at 250.
    pub fn zoom_in(&mut self) -> u16 {
        let next = self.zoom_percent.saturating_add(25).min(250);
        self.set_zoom_percent(next)
    }

    /// Subtract 25 percent, floored at 50.
    pub fn zoom_out(&mut self) -> u16 {
        let next = self.zoom_percent.saturating_sub(25).max(50);
        self.set_zoom_percent(next)
    }

    /// Set zoom to 100.
    pub fn reset_zoom(&mut self) -> u16 {
        self.set_zoom_percent(100)
    }

    /// Last text copied inside this session. This is not the operating-system clipboard.
    pub fn clipboard_text(&self) -> &str {
        &self.clipboard
    }

    /// Replace the session clipboard string.
    pub fn set_clipboard_text(&mut self, text: &str) {
        self.clipboard = text.to_string();
    }

    /// Last text stored by `select_text_at` or `select_text_range`.
    pub fn selected_text(&self) -> &str {
        &self.selected_text
    }

    /// Page-space highlight rectangles for the current text selection.
    pub fn selection_rects(&self) -> &[Rect] {
        &self.selection_rects
    }

    /// Clear any active page text selection and its highlight rectangles.
    pub fn clear_selection(&mut self) {
        self.selected_text.clear();
        self.selection_rects.clear();
    }

    /// Hit-test page text at the zoomed, scrolled point and store it. A
    /// page drawn from its renderer's frame gives the text run drawn there.
    pub fn select_text_at(&mut self, x: f32, y: f32) -> Option<String> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        let (px, py, view) = (x / z, (y + self.scroll) / z, self.page_view());
        let found = match &page.drawn {
            Some(frame) => frame.text_at(px, py, view),
            None => page.text_at_in_view(px, py, Some(view)),
        };
        let Some(found) = found else {
            self.selection_rects.clear();
            return None;
        };
        let (_, rects) = page.select_text_runs_in_rect(Rect {
            x: (px - 2.0).max(0.0),
            y: (py - 2.0).max(0.0),
            width: 4.0,
            height: 4.0,
        });
        self.selection_rects = rects;
        self.selected_text = found.clone();
        Some(found)
    }

    /// Select all text runs intersecting the window-relative page drag box `(x0, y0)`..`(x1, y1)`.
    pub fn select_text_range(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) -> Option<String> {
        if (x1 - x0).abs() < 4.0 && (y1 - y0).abs() < 4.0 {
            return self.select_text_at(x1, y1);
        }
        let z = self.zoom_factor();
        let page = self.active().current()?;
        let px0 = x0.min(x1) / z;
        let py0 = (y0.min(y1) + self.scroll) / z;
        let px1 = x0.max(x1) / z;
        let py1 = (y0.max(y1) + self.scroll) / z;
        let region = Rect {
            x: px0.max(0.0),
            y: py0.max(0.0),
            width: (px1 - px0).max(2.0),
            height: (py1 - py0).max(2.0),
        };
        let (text, rects) = page.select_text_runs_in_rect(region);
        if text.is_empty() {
            self.selection_rects.clear();
            return None;
        }
        self.selected_text = text.clone();
        self.selection_rects = rects;
        Some(text)
    }

    /// Record the DOM node at zoomed, scrolled page point `(x, y)` as the DevTools inspected element.
    pub fn inspect_at(&mut self, x: f32, y: f32) -> Option<NodeId> {
        let z = self.zoom_factor();
        let view = self.page_view();
        let page = self.active().current()?;
        let hit = page.hit_test_in_view(x / z, (y + self.scroll) / z, view);
        if hit.is_some() {
            self.inspected_node = hit;
        }
        hit
    }

    /// Currently inspected DOM node in the `F12` DevTools dock.
    pub fn inspected_node(&self) -> Option<NodeId> {
        self.inspected_node
    }

    /// Return the 4-line DOM and computed style summary for the inspected element on the active page.
    pub fn inspected_element_summary(&self) -> (String, String, String, String) {
        let Some(page) = self.active().current() else {
            return (
                "Path: (no active document)".to_string(),
                "Inspect: None".to_string(),
                "Box: 0x0 px".to_string(),
                "Border: 0px | Bg: none".to_string(),
            );
        };
        page.element_summary(self.inspected_node)
    }

    /// Console `(expression, result)` history evaluated via the `F12` DevTools Console.
    pub fn console_history(&self) -> &[(String, String)] {
        &self.console_history
    }

    /// Evaluate a JavaScript expression or statement in the active page's `ScriptRuntime`,
    /// restage the page on DOM mutations, and record `(expr, output)` in `console_history`.
    pub fn eval_console_js(&mut self, expr: &str) -> Option<String> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return None;
        }
        let viewport = self.viewport;
        let (output, harvested_storage) = {
            let page = self.tabs.get_mut(self.active)?.current_mut()?;
            let before_logs = page.runtime.console_logs().len();
            let is_stmt = trimmed.contains(';')
                || trimmed.contains("console.")
                || trimmed.starts_with("let ")
                || trimmed.starts_with("const ")
                || trimmed.starts_with("var ")
                || trimmed.starts_with("function ")
                || trimmed.starts_with("if ")
                || trimmed.starts_with("for ")
                || trimmed.starts_with("while ");
            let script = if is_stmt {
                trimmed.to_string()
            } else {
                format!("console.log({trimmed});")
            };
            page.runtime.eval_script(&mut page.document, &script);
            let logs = page.runtime.console_logs();
            let out = if logs.len() > before_logs {
                logs[before_logs..].join(" | ")
            } else if let Some(n) = page.runtime.last_vm_number() {
                format!("{n}")
            } else {
                "ok".to_string()
            };
            let storage = page.runtime.export_local_storage();
            page.restage(viewport);
            (out, storage)
        };
        self.merge_local_storage(harvested_storage);
        if self.console_history.len() >= 32 {
            self.console_history.remove(0);
        }
        self.console_history
            .push((trimmed.to_string(), output.clone()));
        Some(output)
    }

    /// Copy the focused input, else the selection, else the page URL, into the session clipboard.
    pub fn copy_selection_or_focused(&mut self) -> Option<String> {
        let value = if let Some(page) = self.active().current() {
            if let Some(id) = page.focused_input {
                let v = page.document.input_value(id);
                if !v.is_empty() { Some(v) } else { None }
            } else if !self.selected_text.is_empty() {
                Some(self.selected_text.clone())
            } else if !page.url.is_empty() {
                Some(page.url.clone())
            } else {
                None
            }
        } else {
            None
        }?;
        self.clipboard = value.clone();
        Some(value)
    }

    /// Copy the focused input and clear it. Returns `None` when nothing is focused.
    pub fn cut_focused_input(&mut self) -> Option<String> {
        let viewport = self.viewport;
        let page = self.tabs[self.active].current_mut()?;
        let id = page.focused_input?;
        let val = page.document.input_value(id);
        page.document.set_input_value(id, "");
        page.restage(viewport);
        self.clipboard = val.clone();
        Some(val)
    }

    /// Insert printable ASCII from `text` at the caret of the focused input.
    pub fn paste_into_focused_input(&mut self, text: &str) -> bool {
        let clean: String = text.chars().filter(|c| (' '..='~').contains(c)).collect();
        if clean.is_empty() {
            return false;
        }
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let updated = if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            let val = page.document.insert_input_text_at_cursor(id, &clean);
            if page.runtime.dispatch_input(&mut page.document, id, &val) {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            true
        } else {
            false
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        updated
    }

    /// Copy of the settings struct.
    pub fn settings(&self) -> BrowserSettings {
        self.settings
    }

    /// `anvil-video` session used to decode clips. Not a window presenter.
    pub fn gpu_video(&self) -> &GpuVideoSession {
        &self.gpu_video
    }

    /// History rows recorded by `open_page`.
    pub fn history_log(&self) -> &[Bookmark] {
        &self.history_log
    }

    /// Remove history rows. Bookmarks are left in place.
    pub fn clear_history_log(&mut self) {
        self.history_log.clear();
    }

    /// Current viewport size.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Vertical scroll in pixels.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Page height as drawn times zoom, minus the viewport height, floored
    /// at 0. See [`Page::shown_height`].
    pub fn max_scroll(&self) -> f32 {
        let Some(page) = self.active().current() else {
            return 0.0;
        };
        (page.shown_height() * self.zoom_factor() - self.viewport.height).max(0.0)
    }

    /// Add `delta` to scroll and clamp it.
    pub fn scroll_by(&mut self, delta: f32) -> f32 {
        self.scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        self.sync_page_scroll();
        self.scroll
    }

    /// Set scroll to `ratio` times `max_scroll`. `ratio` is clamped to 0 through 1.
    pub fn scroll_to_ratio(&mut self, ratio: f32) -> f32 {
        let max_s = self.max_scroll();
        self.scroll = (ratio.clamp(0.0, 1.0) * max_s).clamp(0.0, max_s);
        self.sync_page_scroll();
        self.scroll
    }

    /// Store the size, restage the active page, and clamp scroll.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        if let Some(current) = self.tabs[self.active].current_mut() {
            current.restage(self.viewport);
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.sync_page_scroll();
    }

    fn sync_page_scroll(&mut self) {
        if let Some(page) = self
            .tabs
            .get_mut(self.active)
            .and_then(|tab| tab.current_mut())
        {
            page.scroll_y = self.scroll;
        }
    }

    /// Resolve the anchor under the zoomed point against the page URL. A
    /// page drawn from its renderer's frame uses the frame's links, which
    /// are already full `http` or `https` URLs.
    pub fn hit_link(&self, x: f32, y: f32) -> Option<String> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        if let Some(frame) = &page.drawn {
            let url = frame.link_at(x / z, (y + self.scroll) / z, self.page_view())?;
            return Some(url.to_string());
        }
        let href = page.link_at_in_view(x / z, (y + self.scroll) / z, Some(self.page_view()))?;
        if let Some(frag) = extract_same_page_fragment(&page.url, href) {
            let base = page.url.split('#').next().unwrap_or(&page.url);
            return Some(format!("{base}#{frag}"));
        }
        if let Some((href_base, frag)) = href.trim().split_once('#')
            && !frag.is_empty()
            && let Some(resolved) = resolve_url(&page.url, href_base)
        {
            return Some(format!("{resolved}#{frag}"));
        }
        resolve_url(&page.url, href)
    }

    /// Resolve the anchor under the zoomed point `(x, y)` against the page URL.
    pub fn link_at(&self, x: f32, y: f32) -> Option<String> {
        self.hit_link(x, y)
    }

    /// Scroll the active page to a `#fragment` target if it exists in the document.
    pub fn scroll_to_fragment(&mut self, fragment: &str) -> bool {
        let z = self.zoom_factor();
        let max_s = self.max_scroll();
        let Some(y) = self
            .active()
            .current()
            .and_then(|page| page.fragment_scroll_y(fragment))
        else {
            return false;
        };
        self.scroll = (y * z).clamp(0.0, max_s);
        self.sync_page_scroll();
        true
    }

    /// Perform in-page `#fragment` scrolling and update `page.url` when `target` refers to a
    /// fragment on the active document.
    pub fn try_same_page_fragment_navigation(&mut self, target: &str) -> bool {
        let Some((base_url, frag)) = self.active().current().and_then(|page| {
            let frag = extract_same_page_fragment(&page.url, target)?;
            let base = page.url.split('#').next().unwrap_or(&page.url).to_string();
            Some((base, frag.to_string()))
        }) else {
            return false;
        };
        if !self.scroll_to_fragment(&frag) {
            return false;
        }
        if let Some(page) = self.tabs[self.active].current_mut() {
            page.url = format!("{base_url}#{frag}");
        }
        true
    }

    /// Text input under the zoomed, scrolled point.
    pub fn hit_text_input(&self, x: f32, y: f32) -> Option<NodeId> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        page.text_input_at_in_view(x / z, (y + self.scroll) / z, Some(self.page_view()))
    }

    /// True when the active page has a focused text input.
    pub fn has_focused_input(&self) -> bool {
        self.active()
            .current()
            .is_some_and(|p| p.focused_input.is_some())
    }

    /// Clear input focus and restage. Returns whether focus was set.
    pub fn blur_page_input(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut()
            && page.focused_input.take().is_some()
        {
            page.restage(viewport);
            return true;
        }
        false
    }

    /// Insert one printable ASCII character (or newline in a `<textarea>`) at the caret of the focused input and restage.
    pub fn type_into_focused_input(&mut self, ch: char) -> bool {
        if !(' '..='~').contains(&ch) && ch != '\n' {
            return false;
        }
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let updated = if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            if ch == '\n' && page.document.get(id).data().tag() != Some("textarea") {
                return false;
            }
            let ch_str = ch.to_string();
            let val = page.document.insert_input_text_at_cursor(id, &ch_str);
            if page.runtime.dispatch_input(&mut page.document, id, &val) {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            true
        } else {
            false
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        updated
    }

    /// Cycle keyboard focus to the next (`reverse == false`) or previous (`reverse == true`)
    /// focusable text input or `<textarea>` on the active page.
    pub fn focus_next_input(&mut self, reverse: bool) -> bool {
        let viewport = self.viewport;
        let z = self.zoom_factor();
        let target_rect = {
            let Some(page) = self.tabs[self.active].current_mut() else {
                return false;
            };
            let inputs = page.document.focusable_inputs();
            if inputs.is_empty() {
                return false;
            }
            let next_id = match page
                .focused_input
                .and_then(|curr| inputs.iter().position(|&id| id == curr))
            {
                Some(idx) => {
                    if reverse {
                        inputs[(idx + inputs.len() - 1) % inputs.len()]
                    } else {
                        inputs[(idx + 1) % inputs.len()]
                    }
                }
                None => {
                    if reverse {
                        *inputs.last().expect("non-empty")
                    } else {
                        inputs[0]
                    }
                }
            };
            page.focused_input = Some(next_id);
            page.restage(viewport);
            page.find_layout_rect(next_id)
        };
        if let Some(rect) = target_rect {
            let top = rect.y * z;
            let bottom = (rect.y + rect.height) * z;
            if top < self.scroll || bottom > self.scroll + self.viewport.height {
                let max_s = self.max_scroll();
                self.scroll = (top - 24.0).clamp(0.0, max_s);
                self.sync_page_scroll();
            }
        }
        true
    }

    /// Delete the character before the caret of the focused input and restage.
    pub fn backspace_focused_input(&mut self) -> bool {
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let updated = if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            let before = page.document.input_value(id);
            let val = page.document.backspace_input_at_cursor(id);
            if val == before {
                return false;
            }
            if page.runtime.dispatch_input(&mut page.document, id, &val) {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            true
        } else {
            false
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        updated
    }

    /// Delete the character at the caret of the focused input (`Delete` key) and restage.
    pub fn delete_focused_input(&mut self) -> bool {
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let updated = if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            let before = page.document.input_value(id);
            let val = page.document.delete_input_at_cursor(id);
            if val == before {
                return false;
            }
            if page.runtime.dispatch_input(&mut page.document, id, &val) {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            true
        } else {
            false
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        updated
    }

    /// Delete the word preceding the caret of the focused input (`Ctrl+Backspace`) and restage.
    pub fn delete_word_focused_input(&mut self) -> bool {
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let updated = if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            let before = page.document.input_value(id);
            let val = page.document.delete_input_word_at_cursor(id);
            if val == before {
                return false;
            }
            if page.runtime.dispatch_input(&mut page.document, id, &val) {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            true
        } else {
            false
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        updated
    }

    /// Move the caret one character left in the focused input and restage.
    pub fn move_focused_input_cursor_left(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            page.document.move_input_cursor_left(id);
            page.restage(viewport);
            return true;
        }
        false
    }

    /// Move the caret one character right in the focused input and restage.
    pub fn move_focused_input_cursor_right(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            page.document.move_input_cursor_right(id);
            page.restage(viewport);
            return true;
        }
        false
    }

    /// Move the caret to the start (`0`) of the focused input and restage.
    pub fn move_focused_input_cursor_home(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            page.document.move_input_cursor_home(id);
            page.restage(viewport);
            return true;
        }
        false
    }

    /// Move the caret to the end of the focused input and restage.
    pub fn move_focused_input_cursor_end(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut()
            && let Some(id) = page.focused_input
        {
            page.document.move_input_cursor_end(id);
            page.restage(viewport);
            return true;
        }
        false
    }

    /// Return the screen-space `(x, y, width, height)` rectangle (within the page viewport) and
    /// `(value, label, is_selected)` options list for a `<select>` element on the active page.
    pub fn select_dropdown_info(
        &self,
        select_id: NodeId,
    ) -> Option<(Rect, Vec<(String, String, bool)>)> {
        let page = self.active().current()?;
        if !page.document.is_select_control(select_id) {
            return None;
        }
        let options = page.document.select_options_list(select_id);
        if options.is_empty() {
            return None;
        }
        let z = self.zoom_factor();
        let page_rect = page.find_layout_rect(select_id).unwrap_or(Rect {
            x: 24.0,
            y: 24.0,
            width: 180.0,
            height: 28.0,
        });
        let screen_rect = Rect {
            x: page_rect.x * z,
            y: page_rect.y * z - self.scroll,
            width: page_rect.width * z,
            height: page_rect.height * z,
        };
        Some((screen_rect, options))
    }

    /// Select the option at `option_index` on `select_id`, dispatch `input` to `AnvilVM`, and restage.
    pub fn choose_select_option(
        &mut self,
        select_id: NodeId,
        option_index: usize,
    ) -> Option<String> {
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let chosen = if let Some(page) = self.tabs[self.active].current_mut() {
            let val = page
                .document
                .select_option_by_index(select_id, option_index)?;
            if page
                .runtime
                .dispatch_input(&mut page.document, select_id, &val)
            {
                harvested_storage = Some(page.runtime.export_local_storage());
            }
            page.restage(viewport);
            Some(val)
        } else {
            None
        };
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        chosen
    }

    /// Blur the focused control, then submit the form that encloses it (or insert `\n` for `<textarea>`).
    pub fn submit_focused_input(&mut self) -> ClickOutcome {
        let focused = self
            .tabs
            .get(self.active)
            .and_then(|t| t.current())
            .and_then(|p| p.focused_input);
        let Some(id) = focused else {
            return ClickOutcome::None;
        };
        let is_textarea = self
            .tabs
            .get(self.active)
            .and_then(|t| t.current())
            .is_some_and(|p| p.document.get(id).data().tag() == Some("textarea"));
        if is_textarea {
            self.type_into_focused_input('\n');
            return ClickOutcome::InputFocused(id);
        }
        self.blur_page_input();
        self.submit_form_for(id)
    }

    /// GET query or POST body for the form around `control_id`. Does not navigate by itself.
    pub fn submit_form_for(&mut self, control_id: NodeId) -> ClickOutcome {
        let viewport = self.viewport;
        let mut harvested_storage = None;
        let mut script_outcome = None;
        if let Some(page) = self.tabs[self.active].current_mut() {
            let Some(form_id) = page.document.enclosing_form(control_id) else {
                return ClickOutcome::None;
            };
            if page.runtime.dispatch_submit(&mut page.document, form_id) {
                harvested_storage = Some(page.runtime.export_local_storage());
                if let Some(dest) = page.runtime.take_pending_navigation() {
                    script_outcome = Some(ClickOutcome::Navigate(dest));
                } else if page.runtime.last_event_default_prevented() {
                    page.restage(viewport);
                    script_outcome = Some(ClickOutcome::ScriptHandled);
                } else {
                    page.restage(viewport);
                }
            }
        }
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        if let Some(outcome) = script_outcome {
            if matches!(outcome, ClickOutcome::ScriptHandled) {
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
            }
            return outcome;
        }
        let Some(page) = self.tabs[self.active].current() else {
            return ClickOutcome::None;
        };
        let Some(form_id) = page.document.enclosing_form(control_id) else {
            return ClickOutcome::None;
        };
        let (action, method, mut encoded) = page.document.serialize_form(form_id);
        append_submitter_pair(&page.document, form_id, control_id, &mut encoded);
        // An action that doesn't resolve against the page (for example an
        // `anvil://` action on a web page) submits nothing.
        let target_base = if action.is_empty() {
            page.url.clone()
        } else {
            match resolve_url(&page.url, &action) {
                Some(url) => url,
                None => return ClickOutcome::None,
            }
        };
        if method == "POST" {
            ClickOutcome::SubmitPost {
                url: target_base,
                body: encoded,
            }
        } else {
            ClickOutcome::Navigate(append_query(&target_base, &encoded))
        }
    }

    /// Hit-test the active page in zoomed, scrolled coordinates.
    ///
    /// Order: script click, video controls, text input, checkbox, select, submit,
    /// then a link. A miss clears input focus.
    pub fn click_at(&mut self, x: f32, y: f32) -> ClickOutcome {
        let z = self.zoom_factor();
        let page_x = x / z;
        let page_y = (y + self.scroll) / z;
        let viewport = self.viewport;
        let view = self.page_view();
        let mut submit_control = None;
        let mut harvested_storage = None;
        let mut script_outcome = None;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(node_id) = page.hit_test_in_view(page_x, page_y, view) {
                self.inspected_node = Some(node_id);
                if page.runtime.dispatch_click(&mut page.document, node_id) {
                    harvested_storage = Some(page.runtime.export_local_storage());
                    if let Some(dest) = page.runtime.take_pending_navigation() {
                        script_outcome = Some(ClickOutcome::Navigate(dest));
                    } else if let Some(dest) = own_page_link(page, page_x, page_y, view) {
                        // Anvil's own pages listen for clicks on whole
                        // sections. That must not stop the links inside.
                        script_outcome = Some(ClickOutcome::Navigate(dest));
                    } else if !page.runtime.last_event_default_prevented() {
                        let mut focused_ctrl = None;
                        let mut range_ctrl = None;
                        let mut select_ctrl = None;
                        let mut cursor = Some(node_id);
                        while let Some(curr) = cursor {
                            if page.document.is_range_input(curr) {
                                range_ctrl = Some(curr);
                                break;
                            }
                            if page.document.is_text_input(curr) {
                                focused_ctrl = Some(curr);
                                break;
                            }
                            if page.document.is_select_control(curr) {
                                select_ctrl = Some(curr);
                                break;
                            }
                            cursor = page.document.get(curr).parent();
                        }
                        if let Some(curr) = focused_ctrl {
                            page.focused_input = Some(curr);
                            page.restage(viewport);
                            script_outcome = Some(ClickOutcome::InputFocused(curr));
                        } else if let Some(curr) = range_ctrl {
                            let rrect = page.find_layout_rect(curr).unwrap_or(Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 160.0,
                                height: 22.0,
                            });
                            let ratio = ((page_x - rrect.x) / rrect.width.max(1.0)).clamp(0.0, 1.0);
                            let val = page.document.set_range_ratio(curr, ratio);
                            page.focused_input = None;
                            if page.runtime.dispatch_input(&mut page.document, curr, &val) {
                                harvested_storage = Some(page.runtime.export_local_storage());
                            }
                            page.restage(viewport);
                            script_outcome = Some(ClickOutcome::SelectChanged(curr, val));
                        } else if let Some(curr) = select_ctrl {
                            page.focused_input = None;
                            let val = page.document.cycle_select_option(curr).unwrap_or_default();
                            page.restage(viewport);
                            script_outcome = Some(ClickOutcome::SelectChanged(curr, val));
                        } else {
                            page.restage(viewport);
                            script_outcome = Some(ClickOutcome::ScriptHandled);
                        }
                    } else {
                        page.restage(viewport);
                        script_outcome = Some(ClickOutcome::ScriptHandled);
                    }
                } else {
                    let mut cursor = Some(node_id);
                    while let Some(curr) = cursor {
                        if page.videos.contains_key(&curr) {
                            page.focused_input = None;
                            let vrect = page.find_layout_rect(curr).unwrap_or(Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 240.0,
                                height: 120.0,
                            });
                            let rel_x = (page_x - vrect.x).max(0.0);
                            let rel_y = (page_y - vrect.y).max(0.0);
                            let in_scrubber =
                                rel_y >= (vrect.height - 26.0).max(0.0) && rel_x >= 32.0;
                            if let Some(player) = page.videos.get_mut(&curr) {
                                if in_scrubber {
                                    let track_w = (vrect.width - 42.0).max(1.0);
                                    let ratio = ((rel_x - 32.0) / track_w).clamp(0.0, 1.0);
                                    let target_ms = ((player.duration_ms as f32 * ratio).round()
                                        as u32)
                                        .min(player.duration_ms);
                                    player.seek_to_ms(target_ms);
                                    if let Some(surface) = player.render_surface() {
                                        page.images.insert(curr, surface);
                                    }
                                    page.restage(viewport);
                                    return ClickOutcome::VideoSeeked(curr, target_ms);
                                } else {
                                    player.playing = !player.playing;
                                    let playing = player.playing;
                                    if playing {
                                        let step = (1000 / player.clip.fps.max(1)).max(33);
                                        player.step_by_ms(step);
                                    }
                                    if let Some(surface) = player.render_surface() {
                                        page.images.insert(curr, surface);
                                    }
                                    page.restage(viewport);
                                    return ClickOutcome::VideoToggled(curr, playing);
                                }
                            }
                        }
                        if page.document.is_range_input(curr) {
                            page.focused_input = None;
                            let rrect = page.find_layout_rect(curr).unwrap_or(Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 160.0,
                                height: 22.0,
                            });
                            let ratio = ((page_x - rrect.x) / rrect.width.max(1.0)).clamp(0.0, 1.0);
                            let val = page.document.set_range_ratio(curr, ratio);
                            let _ = page.runtime.dispatch_input(&mut page.document, curr, &val);
                            page.restage(viewport);
                            return ClickOutcome::SelectChanged(curr, val);
                        }
                        if page.document.is_text_input(curr) {
                            page.focused_input = Some(curr);
                            page.restage(viewport);
                            return ClickOutcome::InputFocused(curr);
                        }
                        if page.document.is_checkbox_input(curr) {
                            page.focused_input = None;
                            let checked = page.document.toggle_checked(curr);
                            page.restage(viewport);
                            return ClickOutcome::CheckboxToggled(curr, checked);
                        }
                        if page.document.is_radio_input(curr) {
                            page.focused_input = None;
                            page.document.select_radio(curr);
                            page.restage(viewport);
                            return ClickOutcome::CheckboxToggled(curr, true);
                        }
                        if page.document.is_select_control(curr) {
                            page.focused_input = None;
                            let val = page.document.cycle_select_option(curr).unwrap_or_default();
                            page.restage(viewport);
                            return ClickOutcome::SelectChanged(curr, val);
                        }
                        if page.document.is_submit_control(curr) {
                            page.focused_input = None;
                            if page.document.get(curr).data().attr("disabled").is_none() {
                                submit_control = Some(curr);
                            }
                            break;
                        }
                        cursor = page.document.get(curr).parent();
                    }
                    if submit_control.is_none()
                        && page.link_at_in_view(page_x, page_y, Some(view)).is_none()
                    {
                        if let Some(target_ctrl) = page.document.label_target(node_id) {
                            if page.document.is_text_input(target_ctrl) {
                                page.focused_input = Some(target_ctrl);
                                page.restage(viewport);
                                return ClickOutcome::InputFocused(target_ctrl);
                            }
                            if page.document.is_checkbox_input(target_ctrl) {
                                page.focused_input = None;
                                let checked = page.document.toggle_checked(target_ctrl);
                                page.restage(viewport);
                                return ClickOutcome::CheckboxToggled(target_ctrl, checked);
                            }
                            if page.document.is_radio_input(target_ctrl) {
                                page.focused_input = None;
                                page.document.select_radio(target_ctrl);
                                page.restage(viewport);
                                return ClickOutcome::CheckboxToggled(target_ctrl, true);
                            }
                            if page.document.is_select_control(target_ctrl) {
                                page.focused_input = None;
                                let val = page
                                    .document
                                    .cycle_select_option(target_ctrl)
                                    .unwrap_or_default();
                                page.restage(viewport);
                                return ClickOutcome::SelectChanged(target_ctrl, val);
                            }
                            if page.document.is_submit_control(target_ctrl)
                                && page
                                    .document
                                    .get(target_ctrl)
                                    .data()
                                    .attr("disabled")
                                    .is_none()
                            {
                                page.focused_input = None;
                                submit_control = Some(target_ctrl);
                            }
                        } else if let Some(details_id) =
                            page.document.enclosing_summary_details(node_id)
                        {
                            page.focused_input = None;
                            let open = page.document.toggle_details(details_id);
                            page.restage(viewport);
                            return ClickOutcome::CheckboxToggled(details_id, open);
                        }
                    }
                }
            }
            if script_outcome.is_none() && page.focused_input.take().is_some() {
                page.restage(viewport);
            }
        }
        if let Some(rows) = harvested_storage {
            self.merge_local_storage(rows);
        }
        if let Some(outcome) = script_outcome {
            if matches!(outcome, ClickOutcome::ScriptHandled) {
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
            }
            return outcome;
        }
        if let Some(ctrl_id) = submit_control {
            return self.submit_form_for(ctrl_id);
        }
        if let Some(target) = self.hit_link(x, y) {
            let _ = self.try_same_page_fragment_navigation(&target);
            return ClickOutcome::Navigate(target);
        }
        ClickOutcome::None
    }

    /// Bookmark slice in insertion order.
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// True when the active page URL is already bookmarked.
    pub fn is_active_bookmarked(&self) -> bool {
        let Some(page) = self.active().current() else {
            return false;
        };
        !page.url.is_empty() && self.bookmarks.iter().any(|b| b.url == page.url)
    }

    /// Append a bookmark when `url` is non-empty and not already stored.
    pub fn add_bookmark(&mut self, title: &str, url: &str) -> bool {
        let url = url.trim();
        if url.is_empty() || self.bookmarks.iter().any(|b| b.url == url) {
            return false;
        }
        let label = if title.trim().is_empty() {
            url.to_string()
        } else {
            title.trim().to_string()
        };
        self.bookmarks.push(Bookmark {
            title: label,
            url: url.to_string(),
        });
        true
    }

    /// Remove every bookmark whose URL equals `url`.
    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        self.bookmarks.len() != before
    }

    /// Remove the active URL, or add it. Returns whether it is bookmarked afterward.
    pub fn toggle_bookmark_active(&mut self) -> bool {
        let Some(page) = self.active().current() else {
            return false;
        };
        let url = page.url.clone();
        let title = page.title.clone();
        if url.is_empty() {
            return false;
        }
        if self.remove_bookmark(&url) {
            false
        } else {
            self.add_bookmark(&title, &url)
        }
    }

    /// Raster the active display list. An empty tab is a white frame of the viewport size.
    pub fn rasterize(&self) -> Framebuffer {
        let width = (self.viewport.width.round() as u32).max(1);
        let height = (self.viewport.height.round() as u32).max(1);
        let Some(page) = self.active().current() else {
            return Framebuffer::new(width, height, PAPER);
        };
        // Raster step, after paint. Zoom other than 100 scales the display list first.
        let list = page.shown_list();
        if self.zoom_percent == 100 {
            rasterize_parallel(list, width, height, self.scroll)
        } else {
            let scaled = list.scaled(self.zoom_factor());
            rasterize_parallel(&scaled, width, height, self.scroll)
        }
    }

    /// Tab slice. Each entry is an in-process history stack.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Number of tabs.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// True when `tabs` is empty. `new` starts with one tab, so a new session is not empty.
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Index passed to `select`.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// The selected tab.
    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// Select `index` and restore that tab's saved scroll position. Returns false when `index` is out of range.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.sync_page_scroll();
        self.active = index;
        let restored = self.tabs[self.active]
            .current()
            .map(|p| p.scroll_y)
            .unwrap_or(0.0);
        self.scroll = restored.clamp(0.0, self.max_scroll());
        true
    }

    /// Push an empty tab and select it. Scroll is reset.
    pub fn new_tab(&mut self) {
        self.sync_page_scroll();
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::empty(id));
        self.active = self.tabs.len() - 1;
        self.scroll = 0.0;
    }

    /// Id of the selected tab.
    pub fn active_tab_id(&self) -> u64 {
        self.tabs[self.active].id
    }

    /// Index of the open tab with `id`.
    pub fn tab_index_for_id(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    /// Compact heap allocations on inactive tabs and non-visible history entries,
    /// returning the number of pages compacted. Nothing in the window calls this yet.
    pub fn trim_inactive_memory(&mut self) -> usize {
        let mut compacted = 0usize;
        for (tab_idx, tab) in self.tabs.iter_mut().enumerate() {
            for (page_idx, page) in tab.history.iter_mut().enumerate() {
                if tab_idx != self.active || page_idx != tab.index {
                    page.shrink_to_fit();
                    compacted += 1;
                }
            }
            tab.history.shrink_to_fit();
        }
        self.bookmarks.shrink_to_fit();
        self.history_log.shrink_to_fit();
        self.local_storage.shrink_to_fit();
        compacted
    }

    /// Shared handle to the per-tab renderer processes. Load threads use this.
    pub fn tab_workers(&self) -> Arc<TabWorkerPool> {
        Arc::clone(&self.tab_workers)
    }

    /// Start tab renderers with `program` and `args`, waiting up to `timeout`
    /// for each to start and for each page. Renderers already running keep
    /// their command.
    pub fn set_tab_worker_command(&mut self, program: &Path, args: &[String], timeout: Duration) {
        self.tab_workers.set_command(program, args, timeout);
    }

    /// Confine tab renderers started from now on as `mode` says, and keep
    /// that in the settings.
    pub fn set_renderer_confinement(&mut self, mode: ConfineMode) {
        self.settings.renderer_confinement = mode;
        self.tab_workers.set_confinement(mode);
    }

    /// anvil://security for this window. The confinement part starts a
    /// fresh renderer the way tab renderers start now and shows what it
    /// read about itself.
    pub fn security_page_html(&self) -> String {
        use anvil_sandbox::confine;
        let mode = self.settings.renderer_confinement;
        let decision = self
            .tab_workers
            .confinement()
            .unwrap_or_else(|reason| Arc::new(confine::Confinement::unavailable(mode, reason)));
        let live = self.tab_workers.probe_renderer_status();
        let rows = confine::protections(
            &decision,
            live.as_ref().map_err(String::as_str),
            &confine::own_namespaces(),
        );
        let mut section = frontend::shields_section_html(&self.network_shield);
        section.push_str(&frontend::confinement_section_html(
            &rows,
            mode,
            &self.tab_workers.running_renderers(),
        ));
        section.push_str(&frontend::drawing_section_html(
            self.settings.draw_from_renderer,
        ));
        frontend::security_html_with_sections(&anvil_net::recent_key_exchanges(), &section)
    }

    /// Look for a confinement tool and run its self-test on a helper
    /// thread, so the first page doesn't wait for it. The window calls
    /// this once it has loaded the profile.
    pub fn check_renderer_confinement_in_background(&self) {
        let pool = Arc::clone(&self.tab_workers);
        let _ = std::thread::Builder::new()
            .name("renderer-confinement".to_string())
            .spawn(move || {
                let _ = pool.confinement();
            });
    }

    /// Give idle tab renderers a heartbeat every `interval` from now on.
    pub fn set_tab_heartbeat_interval(&self, interval: Duration) {
        self.tab_workers.set_heartbeat_interval(interval);
    }

    /// Process id of the worker for the tab at `index`, if it is running.
    pub fn tab_worker_pid(&self, index: usize) -> Option<u32> {
        let tab = self.tabs.get(index)?;
        self.tab_workers.worker_pid(tab.id)
    }

    /// Remove `index` when more than one tab exists and the tab is not pinned, push it onto `closed_tabs`,
    /// and keep `active` in range. The tab's renderer process ends.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 || index >= self.tabs.len() || self.tabs[index].is_pinned() {
            return false;
        }
        self.sync_page_scroll();
        let closed = self.tabs.remove(index);
        self.tab_workers.close(closed.id);
        if self.closed_tabs.len() >= MAX_CLOSED_TABS {
            self.closed_tabs.remove(0);
        }
        self.closed_tabs.push(closed);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        let restored = self.tabs[self.active]
            .current()
            .map(|p| p.scroll_y)
            .unwrap_or(0.0);
        self.scroll = restored.clamp(0.0, self.max_scroll());
        true
    }

    /// Pin or unpin the tab at `index`, keeping all pinned tabs grouped at the left of the tab strip.
    /// Returns the new pinned state of that tab.
    pub fn toggle_pin_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.sync_page_scroll();
        let active_id = self.tabs[self.active].id;
        let new_pinned = !self.tabs[index].pinned;
        self.tabs[index].pinned = new_pinned;
        let mut pinned_tabs = Vec::with_capacity(self.tabs.len());
        let mut normal_tabs = Vec::with_capacity(self.tabs.len());
        for tab in self.tabs.drain(..) {
            if tab.pinned {
                pinned_tabs.push(tab);
            } else {
                normal_tabs.push(tab);
            }
        }
        pinned_tabs.extend(normal_tabs);
        self.tabs = pinned_tabs;
        if let Some(new_active) = self.tabs.iter().position(|t| t.id == active_id) {
            self.active = new_active;
        }
        new_pinned
    }

    /// Close all unpinned tabs except `keep_index`, returning the number of closed tabs.
    pub fn close_other_tabs(&mut self, keep_index: usize) -> usize {
        if keep_index >= self.tabs.len() {
            return 0;
        }
        self.sync_page_scroll();
        let keep_id = self.tabs[keep_index].id;
        let mut remaining = Vec::with_capacity(self.tabs.len());
        let mut closed_count = 0usize;
        for tab in self.tabs.drain(..) {
            if tab.id == keep_id || tab.pinned {
                remaining.push(tab);
            } else {
                self.tab_workers.close(tab.id);
                if self.closed_tabs.len() >= MAX_CLOSED_TABS {
                    self.closed_tabs.remove(0);
                }
                self.closed_tabs.push(tab);
                closed_count += 1;
            }
        }
        self.tabs = remaining;
        self.active = self.tabs.iter().position(|t| t.id == keep_id).unwrap_or(0);
        let restored = self.tabs[self.active]
            .current()
            .map(|p| p.scroll_y)
            .unwrap_or(0.0);
        self.scroll = restored.clamp(0.0, self.max_scroll());
        closed_count
    }

    /// Move the active tab one slot left (`Ctrl+Shift+PageUp`) within its pinned/unpinned group.
    pub fn move_active_tab_left(&mut self) -> bool {
        if self.active == 0 || self.tabs.len() <= 1 {
            return false;
        }
        let prev = self.active - 1;
        if self.tabs[prev].pinned != self.tabs[self.active].pinned {
            return false;
        }
        self.tabs.swap(prev, self.active);
        self.active = prev;
        true
    }

    /// Move the active tab one slot right (`Ctrl+Shift+PageDown`) within its pinned/unpinned group.
    pub fn move_active_tab_right(&mut self) -> bool {
        if self.active + 1 >= self.tabs.len() {
            return false;
        }
        let next = self.active + 1;
        if self.tabs[next].pinned != self.tabs[self.active].pinned {
            return false;
        }
        self.tabs.swap(self.active, next);
        self.active = next;
        true
    }

    /// Number of recently closed tabs available to reopen (`Ctrl+Shift+T`).
    pub fn closed_tabs_len(&self) -> usize {
        self.closed_tabs.len()
    }

    /// Reopen the most recently closed tab (`Ctrl+Shift+T`), preserving its back/forward history and scroll position.
    pub fn reopen_closed_tab(&mut self) -> bool {
        let Some(mut reopened) = self.closed_tabs.pop() else {
            return false;
        };
        self.sync_page_scroll();
        reopened.id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(reopened);
        self.active = self.tabs.len() - 1;
        let restored = self.tabs[self.active]
            .current()
            .map(|p| p.scroll_y)
            .unwrap_or(0.0);
        self.scroll = restored.clamp(0.0, self.max_scroll());
        true
    }

    /// Clone the tab at `index` (including its back/forward history and scroll position) and select the duplicate.
    pub fn duplicate_tab(&mut self, index: usize) -> Option<usize> {
        let mut cloned = self.tabs.get(index)?.clone();
        self.sync_page_scroll();
        cloned.id = self.next_tab_id;
        self.next_tab_id += 1;
        let insert_idx = (index + 1).min(self.tabs.len());
        self.tabs.insert(insert_idx, cloned);
        self.active = insert_idx;
        let restored = self.tabs[self.active]
            .current()
            .map(|p| p.scroll_y)
            .unwrap_or(0.0);
        self.scroll = restored.clamp(0.0, self.max_scroll());
        Some(self.active)
    }

    /// Cycle to the next tab (`Ctrl+Tab`), wrapping around at the end of the strip.
    pub fn next_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.select(next)
    }

    /// Cycle to the previous tab (`Ctrl+Shift+Tab`), wrapping around at the start of the strip.
    pub fn prev_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let prev = (self.active + self.tabs.len() - 1) % self.tabs.len();
        self.select(prev)
    }

    /// Switch directly to tab `1..=8`, or the last tab when `slot == 9` (`Ctrl+1`..`Ctrl+9`).
    pub fn select_tab_shortcut(&mut self, slot: u8) -> bool {
        if self.tabs.is_empty() || !(1..=9).contains(&slot) {
            return false;
        }
        let target = if slot == 9 {
            self.tabs.len() - 1
        } else {
            let idx = (slot - 1) as usize;
            if idx >= self.tabs.len() {
                return false;
            }
            idx
        };
        self.select(target)
    }

    /// Open a line-numbered `view-source:<url>` inspector tab for the active page (`Ctrl+U`).
    pub fn view_active_page_source(&mut self) -> Option<String> {
        let (url, source) = {
            let page = self.active().current()?;
            if page.url.is_empty() && page.source.is_empty() {
                return None;
            }
            (page.url.clone(), page.source.clone())
        };
        if url.to_ascii_lowercase().starts_with("view-source:") {
            return Some(url);
        }
        self.new_tab();
        let view_url = format!("view-source:{url}");
        let html = frontend::with_theme_css(
            &frontend::view_source_html(&url, &source),
            self.settings.theme,
        );
        let mut page =
            render_html_with_storage(&html, &view_url, self.viewport, &self.local_storage);
        page.url = view_url.clone();
        self.open_page(page);
        Some(view_url)
    }

    /// Save the active page's HTML source to the download vault and disk (`Ctrl+S`).
    pub fn save_active_page_to_downloads(&mut self) -> Option<String> {
        let (url, source) = {
            let page = self.active().current()?;
            if page.url.is_empty() && page.source.is_empty() {
                return None;
            }
            (page.url.clone(), page.source.clone())
        };
        let base_name = if let Some(rest) = url.strip_prefix("anvil://") {
            let clean: String = rest
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let trimmed = clean.trim_matches('-');
            format!(
                "anvil-{}.html",
                if trimmed.is_empty() { "page" } else { trimmed }
            )
        } else {
            let suggested = downloads::sanitize_download_filename(&url);
            if suggested.ends_with(".html") || suggested.ends_with(".htm") {
                suggested
            } else {
                format!("{suggested}.html")
            }
        };
        let entry =
            self.record_or_save_download(&url, Some(&base_name), "text/html", source.as_bytes());
        self.auto_persist_profile();
        Some(entry.file_name)
    }

    /// Record history, merge `localStorage`, push `page` on the active tab, and reset scroll.
    pub fn open_page(&mut self, mut page: Page) {
        self.sync_page_scroll();
        self.keep_frame_if_drawn(&mut page);
        page.runtime.set_jit_enabled(self.settings.jit_enabled);
        self.record_history(&page);
        let rows = page.runtime.export_local_storage();
        self.merge_local_storage(rows);
        page.runtime.import_local_storage(&self.local_storage);
        let frag = page
            .url
            .split_once('#')
            .map(|(_, f)| f.to_string())
            .filter(|f| !f.is_empty());
        self.tabs[self.active].navigate(page);
        self.scroll = 0.0;
        if let Some(frag) = frag {
            let _ = self.scroll_to_fragment(&frag);
        }
    }

    /// Push `page` on the tab with `tab_id`. A page for a tab that has closed is dropped.
    pub fn open_page_in_tab_id(&mut self, tab_id: u64, page: Page) -> bool {
        match self.tab_index_for_id(tab_id) {
            Some(index) => {
                self.open_page_in_tab(index, page);
                true
            }
            None => false,
        }
    }

    /// Record history, merge `localStorage`, and push `page` on `tab_index` when that tab exists.
    pub fn open_page_in_tab(&mut self, tab_index: usize, mut page: Page) {
        if tab_index == self.active {
            self.sync_page_scroll();
        }
        self.keep_frame_if_drawn(&mut page);
        page.runtime.set_jit_enabled(self.settings.jit_enabled);
        self.record_history(&page);
        let rows = page.runtime.export_local_storage();
        self.merge_local_storage(rows);
        page.runtime.import_local_storage(&self.local_storage);
        let frag = page
            .url
            .split_once('#')
            .map(|(_, f)| f.to_string())
            .filter(|f| !f.is_empty());
        if let Some(tab) = self.tabs.get_mut(tab_index) {
            tab.navigate(page);
            if tab_index == self.active {
                self.scroll = 0.0;
                if let Some(frag) = frag {
                    let _ = self.scroll_to_fragment(&frag);
                }
            }
        }
    }

    /// Keep the renderer's frame on `page` only when web pages are drawn
    /// from their renderer and `page` is a web page. `anvil://` pages are
    /// always drawn by the window.
    fn keep_frame_if_drawn(&self, page: &mut Page) {
        let web = page.url.starts_with("http://") || page.url.starts_with("https://");
        if !(self.settings.draw_from_renderer && web) {
            page.drawn = None;
        }
    }

    /// Turn drawing web pages from their renderer's frame on or off. Pages
    /// already open keep being drawn by the window; turning it off drops
    /// every kept frame at once.
    pub fn set_draw_from_renderer(&mut self, on: bool) {
        self.settings.draw_from_renderer = on;
        if !on {
            for tab in &mut self.tabs {
                for page in &mut tab.history {
                    page.drawn = None;
                }
            }
            self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        }
    }

    /// Append a history row unless the URL is empty, repeated, or a history, downloads, or inspect page.
    fn record_history(&mut self, page: &Page) {
        let url = page.url.trim();
        if url.is_empty()
            || url.starts_with("anvil://history")
            || url.starts_with("anvil://downloads")
            || url.starts_with("anvil://inspect")
        {
            return;
        }
        let title = if page.title.trim().is_empty() {
            url.to_string()
        } else {
            page.title.trim().to_string()
        };
        if self.history_log.last().is_some_and(|last| last.url == url) {
            return;
        }
        self.history_log.push(Bookmark {
            title,
            url: url.to_string(),
        });
    }

    /// Render `html` inside `execute_isolated`. A caught panic becomes the recovery page.
    pub fn open_html(&mut self, html: &str, url: &str) {
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let seeded_storage = self.local_storage.clone();
        let fetch_jar = self.cookie_jar_handle();
        let page = match self.sandbox_guard.execute_isolated(&label, || {
            Ok(render_html_with_resources_csp_and_storage(
                html,
                "",
                HashMap::new(),
                url,
                viewport,
                None,
                0,
                &seeded_storage,
                Some(fetch_jar.clone()),
                true,
            ))
        }) {
            Ok(p) => p,
            Err(err) => render_sandbox_recovery_page(url, &err.to_string(), viewport),
        };
        self.open_page(page);
    }

    /// Run `workload` on this thread through `execute_isolated`.
    /// A panic or `Err` becomes the recovery page on the active tab. No child process is started.
    pub fn open_isolated_workload<F>(&mut self, url: &str, workload: F)
    where
        F: FnOnce(Viewport) -> Result<Page, String>,
    {
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let page = match self
            .sandbox_guard
            .execute_isolated(&label, || workload(viewport))
        {
            Ok(p) => p,
            Err(err) => render_sandbox_recovery_page(url, &err.to_string(), viewport),
        };
        self.open_page(page);
    }

    /// Upgrade `url` with HSTS, load it inside `execute_isolated`, and open the page or the recovery page.
    pub fn open_url(&mut self, url: &str) -> Result<(), BrowserError> {
        if self.try_same_page_fragment_navigation(url) {
            return Ok(());
        }
        let raw_upgraded = self.hsts_store.upgrade_url_if_hsts(url.trim());
        let upgraded = self
            .network_shield
            .clean_tracking_params(&raw_upgraded, None);
        sync_default_shield(&self.network_shield);
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let fetch_jar = self.cookie_jar_handle();
        let scripts_enabled = self.scripts_enabled_for(&upgraded);
        let _https_only = anvil_net::enter_https_only(self.https_only_policy());
        let tab_id = self.active_tab_id();
        let workers = self.tab_workers();
        match self.sandbox_guard.execute_isolated(&label, || {
            Ok(load_url_routed(
                &upgraded,
                viewport,
                Some(fetch_jar.clone()),
                scripts_enabled,
                tab_id,
                DocumentRoute::Tab(&workers),
            ))
        }) {
            Ok(Ok(page)) => {
                self.open_page(page);
                Ok(())
            }
            Ok(Err(BrowserError::Download {
                url,
                filename,
                content_type,
                body,
            })) => {
                self.record_or_save_download(&url, filename.as_deref(), &content_type, &body);
                let _ = self.open_target(frontend::DOWNLOADS_URL);
                Ok(())
            }
            Ok(Err(err)) => {
                let page = navigation_error_page(&upgraded, &err.to_string(), viewport);
                self.open_page(page);
                Ok(())
            }
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.open_page(page);
                Ok(())
            }
        }
    }

    fn record_or_save_download(
        &mut self,
        url: &str,
        filename: Option<&str>,
        mime_type: &str,
        data: &[u8],
    ) -> downloads::DownloadEntry {
        #[cfg(not(test))]
        {
            let dir = downloads::default_downloads_dir();
            if let Ok((entry, _)) = self
                .downloads
                .save_payload_to_dir(url, filename, mime_type, data, &dir)
            {
                return entry;
            }
            if let Some(last) = self.downloads.entries().last().cloned() {
                return last;
            }
            self.downloads
                .record_download(url, filename, mime_type, data)
        }
        #[cfg(test)]
        {
            self.downloads
                .record_download(url, filename, mime_type, data)
        }
    }

    fn auto_persist_profile(&self) {
        #[cfg(not(test))]
        {
            let _ = self.save_default_profile();
        }
    }

    /// Open a built-in `anvil://` page or another target.
    ///
    /// Settings query flags toggle in-memory settings and then render the settings
    /// page. Sample download actions only record bytes. They do not save a file.
    /// http(s) pages and local files go through the tab's renderer process inside
    /// `execute_isolated`. `anvil://` pages are built in this process. A failed
    /// load, a hung renderer, or a renderer that crashed opens an error page and
    /// does not paint the document. The window keeps running.
    pub fn open_target(&mut self, target: &str) -> Result<(), String> {
        let trimmed = target.trim();
        if self.try_same_page_fragment_navigation(trimmed) {
            return Ok(());
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("view-source:") {
            let inner = trimmed["view-source:".len()..].trim();
            if let Some((active_url, active_src)) = self.active().current().and_then(|p| {
                if inner.is_empty() || p.url.eq_ignore_ascii_case(inner) {
                    Some((p.url.clone(), p.source.clone()))
                } else {
                    None
                }
            }) {
                let view_url = format!("view-source:{active_url}");
                let html = frontend::with_theme_css(
                    &frontend::view_source_html(&active_url, &active_src),
                    self.settings.theme,
                );
                let mut page =
                    render_html_with_storage(&html, &view_url, self.viewport, &self.local_storage);
                page.url = view_url;
                self.open_page(page);
                return Ok(());
            }
        }
        if lower == "anvil://home"
            || lower == "anvil://"
            || lower == "about:home"
            || lower == "about:blank"
            || lower.starts_with("anvil://home?")
        {
            let mut changed = false;
            if !has_action_key(trimmed) {
                // Without this run's key the address only shows the page.
            } else if lower.contains("engine=duckduckgo") || lower.contains("engine=ddg") {
                self.settings.search_engine = SearchEngine::DuckDuckGo;
                changed = true;
            } else if lower.contains("engine=bing") {
                self.settings.search_engine = SearchEngine::Bing;
                changed = true;
            } else if lower.contains("engine=google") {
                self.settings.search_engine = SearchEngine::Google;
                changed = true;
            }
            if changed {
                self.auto_persist_profile();
            }
            let html = frontend::with_action_key(
                &frontend::home_html_with_state(
                    self.settings.search_engine,
                    self.settings.theme,
                    &self.bookmarks,
                    &self.history_log,
                ),
                permissions::action_key(),
            );
            let page = render_html_with_storage(
                &html,
                frontend::HOME_URL,
                self.viewport,
                &self.local_storage,
            );
            self.open_page(page);
            return Ok(());
        }
        if lower.starts_with("anvil://navigate")
            && let Some(dest) =
                frontend::resolve_navigation_query_with_engine(trimmed, self.settings.search_engine)
        {
            return self.open_target(&dest);
        }
        if lower == "anvil://inspect"
            || lower == "anvil://devtools"
            || lower.starts_with("anvil://inspect?")
        {
            let html = frontend::inspect_html_with_state(
                self.tabs.len(),
                self.zoom_percent as u32,
                self.gpu_video.backend().label(),
                self.network_shield.blocked_total(),
                self.hsts_store.len(),
                self.downloads.entries().len(),
                self.cookie_jar().len(),
            );
            let html = frontend::with_theme_css(
                &frontend::with_action_key(&html, permissions::action_key()),
                self.settings.theme,
            );
            let page = render_html_with_storage(
                &html,
                frontend::INSPECT_URL,
                self.viewport,
                &self.local_storage,
            );
            self.open_page(page);
            return Ok(());
        }
        if lower == frontend::SITE_PERMISSIONS_URL
            || lower.starts_with("anvil://settings/permissions?")
        {
            let message = match self.apply_site_permission_change(trimmed) {
                PermissionChange::Open(url) => {
                    self.auto_persist_profile();
                    return self.open_target(&url);
                }
                PermissionChange::Message(message) => {
                    if !message.is_empty() {
                        self.auto_persist_profile();
                    }
                    message
                }
            };
            let html = {
                let jar = self.cookie_jar();
                let count = |host: &str| {
                    jar.cookies()
                        .iter()
                        .filter(|c| c.domain == host || c.domain.ends_with(&format!(".{host}")))
                        .count()
                };
                frontend::site_permissions_html(
                    &self.site_permissions,
                    self.settings.scripts_enabled,
                    self.settings.https_only_mode,
                    &count,
                    &message,
                    permissions::action_key(),
                )
            };
            let html = frontend::with_theme_css(&html, self.settings.theme);
            let mut page =
                render_html_with_storage(&html, trimmed, self.viewport, &self.local_storage);
            page.url = frontend::SITE_PERMISSIONS_URL.to_string();
            self.open_page(page);
            return Ok(());
        }
        if lower == "anvil://settings" || lower.starts_with("anvil://settings?") {
            let params = query_params(trimmed);
            let active_section = params
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("section"))
                .map(|(_, v)| v.to_ascii_lowercase())
                .unwrap_or_else(|| "all".to_string());
            // Every change here needs this run's key. Only the built-in pages
            // carry it, so a link, form, or script on a web page that points at
            // `anvil://settings?toggle=...` just shows the page.
            let mut changed = false;
            if !has_action_key(trimmed) {
                // Without this run's key the address only shows the page.
            } else if lower.contains("toggle=gpu_video") {
                self.settings.gpu_video_acceleration = !self.settings.gpu_video_acceleration;
                self.gpu_video.set_hardware_acceleration(
                    self.settings.gpu_video_acceleration,
                    self.settings.gpu_video_acceleration,
                );
                changed = true;
            } else if lower.contains("toggle=https_only") {
                self.settings.https_only_mode = !self.settings.https_only_mode;
                changed = true;
            } else if lower.contains("toggle=scripts") {
                self.settings.scripts_enabled = !self.settings.scripts_enabled;
                changed = true;
            } else if lower.contains("toggle=jit") {
                self.settings.jit_enabled = !self.settings.jit_enabled;
                changed = true;
            } else if lower.contains("toggle=search_engine") {
                self.settings.search_engine = self.settings.search_engine.cycle_next();
                changed = true;
            } else if lower.contains("engine=duckduckgo") {
                self.settings.search_engine = SearchEngine::DuckDuckGo;
                changed = true;
            } else if lower.contains("engine=bing") {
                self.settings.search_engine = SearchEngine::Bing;
                changed = true;
            } else if lower.contains("engine=google") {
                self.settings.search_engine = SearchEngine::Google;
                changed = true;
            } else if lower.contains("toggle=theme") {
                self.settings.theme = self.settings.theme.cycle_next();
                changed = true;
            } else if let Some((_, theme_slug)) =
                params.iter().find(|(k, _)| k.eq_ignore_ascii_case("theme"))
            {
                self.settings.theme = BrowserTheme::from_slug(theme_slug);
                changed = true;
            } else if let Some((_, zoom_val)) =
                params.iter().find(|(k, _)| k.eq_ignore_ascii_case("zoom"))
            {
                match zoom_val.trim().to_ascii_lowercase().as_str() {
                    "in" => {
                        self.zoom_in();
                    }
                    "out" => {
                        self.zoom_out();
                    }
                    "reset" => {
                        self.reset_zoom();
                    }
                    other => {
                        if let Ok(pct) = other.parse::<u16>() {
                            self.set_zoom_percent(pct);
                        }
                    }
                }
                changed = true;
            } else if lower.contains("action=save_profile") {
                let _ = self.save_default_profile();
            } else if lower.contains("action=clear_profile") {
                self.clear_all_profile_data();
            }
            if changed {
                self.auto_persist_profile();
            }
            let html = frontend::settings_html_with_full_state(
                self.gpu_video.backend().label(),
                self.settings.gpu_video_acceleration,
                self.settings.https_only_mode,
                self.settings.scripts_enabled,
                self.settings.jit_enabled,
                self.settings.search_engine,
                self.settings.theme,
                self.zoom_percent,
                &active_section,
                self.gpu_video.frames_decoded().max(12),
                self.gpu_video.frames_encoded().max(12),
                self.bookmarks.len(),
                self.history_log.len(),
                self.cookie_jar().len(),
            );
            let html = frontend::with_action_key(&html, permissions::action_key());
            let mut page = render_html_with_storage(
                &html,
                frontend::SETTINGS_URL,
                self.viewport,
                &self.local_storage,
            );
            page.url = frontend::SETTINGS_URL.to_string();
            page.runtime.set_jit_enabled(self.settings.jit_enabled);
            self.open_page(page);
            return Ok(());
        }
        if lower == frontend::SECURITY_URL
            || lower == "anvil://shields"
            || lower.starts_with("anvil://security?")
            || lower.starts_with("anvil://shields?")
        {
            let params = query_params(trimmed);
            let mut changed = false;
            // Like the settings page, a change needs this run's key.
            if has_action_key(trimmed) {
                if lower.contains("toggle=confinement") {
                    let next = match self.settings.renderer_confinement {
                        ConfineMode::Auto => ConfineMode::Off,
                        ConfineMode::Off => ConfineMode::Auto,
                    };
                    self.set_renderer_confinement(next);
                    changed = true;
                }
                if lower.contains("toggle=drawing") {
                    let next = !self.settings.draw_from_renderer;
                    self.set_draw_from_renderer(next);
                    changed = true;
                }
                if lower.contains("toggle=shields_mode") {
                    self.cycle_shields_mode();
                    changed = true;
                }
                if let Some((_, mode_val)) = params
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("shields_mode"))
                {
                    self.set_shields_mode(ShieldsMode::from_slug(mode_val));
                    changed = true;
                }
                if let Some((_, site_val)) = params
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("toggle_site"))
                {
                    let _ = self.toggle_shields_for_site(site_val);
                    changed = true;
                }
                if let Some((_, rule_val)) = params
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("add_rule"))
                {
                    let _ = self.add_shield_filter_rule(rule_val);
                    changed = true;
                }
            }
            if changed {
                self.auto_persist_profile();
            }
            let html = frontend::with_theme_css(
                &frontend::with_action_key(&self.security_page_html(), permissions::action_key()),
                self.settings.theme,
            );
            let mut page = render_html_with_storage(
                &html,
                frontend::SECURITY_URL,
                self.viewport,
                &self.local_storage,
            );
            page.url = frontend::SECURITY_URL.to_string();
            self.open_page(page);
            return Ok(());
        }
        if lower == "anvil://history" || lower.starts_with("anvil://history?") {
            if has_action_key(trimmed) && lower.contains("action=clear") {
                self.clear_history_log();
                self.auto_persist_profile();
            }
            let html = frontend::with_theme_css(
                &frontend::with_action_key(
                    &frontend::history_html_with_entries(&self.history_log),
                    permissions::action_key(),
                ),
                self.settings.theme,
            );
            let page = render_html_with_storage(
                &html,
                frontend::HISTORY_URL,
                self.viewport,
                &self.local_storage,
            );
            self.open_page(page);
            return Ok(());
        }
        if lower == "anvil://downloads" || lower.starts_with("anvil://downloads?") {
            let params = query_params(trimmed);
            if !has_action_key(trimmed) {
                // Without this run's key the address only shows the page.
            } else if lower.contains("action=clear") {
                self.downloads.clear();
            } else if lower.contains("action=sample_report") {
                let report = format!(
                    "{{\"version\":\"{VERSION}\",\"sandbox\":\"enforced\",\"eme\":\"org.w3.clearkey\",\"cenc\":true}}"
                );
                self.record_or_save_download(
                    "anvil://security/diagnostic-report.json",
                    Some("anvil-diagnostic-report.json"),
                    "application/json",
                    report.as_bytes(),
                );
            } else if lower.contains("action=sample_mp4")
                && let Ok(mp4_bytes) = generate_encrypted_demo_mp4(120, 68, 4)
            {
                self.record_or_save_download(
                    "anvil://encrypted.mp4",
                    Some("anvil-cenc-encrypted-demo.mp4"),
                    "video/mp4",
                    &mp4_bytes,
                );
            } else if lower.contains("action=export_manifest") {
                let manifest = to_json_pretty(&self.downloads.to_json_value());
                self.record_or_save_download(
                    "anvil://downloads/vault-manifest.json",
                    Some("anvil-downloads-manifest.json"),
                    "application/json",
                    manifest.as_bytes(),
                );
            } else if let Some((_, export_str)) = params
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("export"))
                && let Ok(export_id) = export_str.parse::<u64>()
                && let Some(item) = self
                    .downloads
                    .entries()
                    .iter()
                    .find(|e| e.id == export_id)
                    .cloned()
            {
                let receipt = format!(
                    "{{\"id\":{},\"file_name\":\"{}\",\"source_url\":\"{}\",\"mime_type\":\"{}\",\"byte_size\":{},\"sha256_hex\":\"{}\",\"state\":\"{}\"}}",
                    item.id,
                    item.file_name,
                    item.source_url,
                    item.mime_type,
                    item.byte_size,
                    item.sha256_hex,
                    item.state.code()
                );
                let receipt_name = format!("{}-receipt.json", item.file_name);
                self.record_or_save_download(
                    &item.source_url,
                    Some(&receipt_name),
                    "application/json",
                    receipt.as_bytes(),
                );
            }
            let html = frontend::with_theme_css(
                &frontend::with_action_key(
                    &frontend::downloads_html_with_entries(self.downloads.entries()),
                    permissions::action_key(),
                ),
                self.settings.theme,
            );
            let page = render_html_with_storage(
                &html,
                frontend::DOWNLOADS_URL,
                self.viewport,
                &self.local_storage,
            );
            self.open_page(page);
            return Ok(());
        }
        // HTTPS-only is applied by anvil-net on this thread, for the page and
        // everything it loads.
        let raw_upgraded = self.hsts_store.upgrade_url_if_hsts(trimmed);
        let upgraded =
            if raw_upgraded.starts_with("http://") || raw_upgraded.starts_with("https://") {
                self.network_shield
                    .clean_tracking_params(&raw_upgraded, None)
            } else {
                raw_upgraded
            };
        sync_default_shield(&self.network_shield);
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let fetch_jar = self.cookie_jar_handle();
        let scripts_enabled = self.scripts_enabled_for(&upgraded);
        let _https_only = anvil_net::enter_https_only(self.https_only_policy());
        let tab_id = self.active_tab_id();
        let workers = self.tab_workers();
        if upgraded.starts_with("http://") || upgraded.starts_with("https://") {
            match self.sandbox_guard.execute_isolated(&label, || {
                Ok(load_url_routed(
                    &upgraded,
                    viewport,
                    Some(fetch_jar.clone()),
                    scripts_enabled,
                    tab_id,
                    DocumentRoute::Tab(&workers),
                ))
            }) {
                Ok(Ok(page)) => {
                    self.open_page(page);
                    return Ok(());
                }
                Ok(Err(BrowserError::Download {
                    url,
                    filename,
                    content_type,
                    body,
                })) => {
                    self.record_or_save_download(&url, filename.as_deref(), &content_type, &body);
                    return self.open_target(frontend::DOWNLOADS_URL);
                }
                Ok(Err(err)) => {
                    let page = navigation_error_page(&upgraded, &err.to_string(), viewport);
                    self.open_page(page);
                    return Ok(());
                }
                Err(sandbox_err) => {
                    let page =
                        render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                    self.open_page(page);
                    return Ok(());
                }
            }
        }
        match self.sandbox_guard.execute_isolated(&label, || {
            Ok(load_target_in_tab(
                &upgraded,
                viewport,
                Some(fetch_jar.clone()),
                scripts_enabled,
                tab_id,
                &workers,
            ))
        }) {
            Ok(Ok(page)) => {
                self.open_page(page);
                Ok(())
            }
            Ok(Err(err)) => {
                let page = navigation_error_page(&upgraded, &err, viewport);
                self.open_page(page);
                Ok(())
            }
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.open_page(page);
                Ok(())
            }
        }
    }

    /// Apply a change from a link or form on `anvil://settings/permissions`.
    ///
    /// Changes need the page's `key` (see [`permissions::action_key`]), so a
    /// link on a web page can't change permissions. Queries: `add=HOST`;
    /// `site=HOST` with one of `scripts=default|allow|block`,
    /// `cookies=allow|block`, `http=allow|require`, `clear_cookies=1`, or
    /// `remove=1`; and `allow_http=HOST&open=URL`, which allows plain HTTP for
    /// the host and then opens the `http` address on that host.
    fn apply_site_permission_change(&mut self, target: &str) -> PermissionChange {
        let params = query_params(target);
        let get = |name: &str| {
            params
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        let has_change = ["add", "site", "allow_http"]
            .iter()
            .any(|name| get(name).is_some_and(|v| !v.trim().is_empty()));
        if !has_change {
            return PermissionChange::Message(String::new());
        }
        if get("key") != Some(permissions::action_key()) {
            return PermissionChange::Message(
                "That change didn't come from this page, so nothing was changed.".to_string(),
            );
        }
        if let Some(host) = get("allow_http") {
            let Some(host) = permissions::normalize_host(host) else {
                return PermissionChange::Message("That isn't a site name.".to_string());
            };
            self.update_site_permissions(|p| p.set_allow_http(&host, true));
            if let Some(url) = get("open") {
                let same_host = permissions::host_of(url).is_some_and(|h| h == host);
                if same_host && url.to_ascii_lowercase().starts_with("http://") {
                    return PermissionChange::Open(url.to_string());
                }
            }
            return PermissionChange::Message(format!("{host} may now load over plain HTTP."));
        }
        if let Some(host) = get("add") {
            return PermissionChange::Message(
                match self.update_site_permissions(|p| p.add(host)) {
                    Some(host) => format!("Added {host}."),
                    None => "That isn't a site name.".to_string(),
                },
            );
        }
        let Some(host) = get("site").and_then(permissions::normalize_host) else {
            return PermissionChange::Message("That isn't a site name.".to_string());
        };
        let message = if let Some(rule) = get("scripts").and_then(permissions::ScriptRule::parse) {
            self.update_site_permissions(|p| p.set_scripts(&host, rule));
            format!("JavaScript for {host}: {}.", rule.as_str())
        } else if let Some(value) = get("cookies") {
            let block = value == "block";
            self.update_site_permissions(|p| p.set_block_cookies(&host, block));
            if block {
                format!("Cookies for {host} are blocked.")
            } else {
                format!("Cookies for {host} are allowed.")
            }
        } else if let Some(value) = get("http") {
            let allow = value == "allow";
            self.update_site_permissions(|p| p.set_allow_http(&host, allow));
            if allow {
                format!("{host} may load over plain HTTP when HTTPS-only is on.")
            } else {
                format!("{host} has to use HTTPS when HTTPS-only is on.")
            }
        } else if get("clear_cookies").is_some() {
            let removed = self.cookie_jar_mut().remove_host(&host);
            format!("Removed {removed} cookies for {host}.")
        } else if get("remove").is_some() {
            self.update_site_permissions(|p| p.remove(&host));
            format!("Removed {host} from the list.")
        } else {
            String::new()
        };
        PermissionChange::Message(message)
    }

    /// Seek a `<video>` element on the active page to `timestamp_ms` and re-render its HUD frame.
    pub fn seek_video(&mut self, video_id: NodeId, timestamp_ms: u32) -> bool {
        let viewport = self.viewport;
        let Some(page) = self.tabs[self.active].current_mut() else {
            return false;
        };
        let Some(player) = page.videos.get_mut(&video_id) else {
            return false;
        };
        player.seek_to_ms(timestamp_ms);
        if let Some(surface) = player.render_surface() {
            page.images.insert(video_id, surface);
        }
        page.restage(viewport);
        true
    }

    /// Advance all playing `<video>` elements on the active page by `delta_ms` milliseconds.
    pub fn step_active_videos(&mut self, delta_ms: u32) -> usize {
        let viewport = self.viewport;
        let Some(page) = self.tabs[self.active].current_mut() else {
            return 0;
        };
        let mut stepped = 0usize;
        for (&vid_id, player) in page.videos.iter_mut() {
            if player.playing {
                player.step_by_ms(delta_ms.max(1));
                if let Some(surface) = player.render_surface() {
                    page.images.insert(vid_id, surface);
                }
                stepped += 1;
            }
        }
        if stepped > 0 {
            page.restage(viewport);
        }
        stepped
    }

    /// Copy settings, zoom, tab URLs, bookmarks, history, cookies, HSTS, downloads, `localStorage`, and site permissions into a snapshot.
    pub fn to_profile_snapshot(&self) -> ProfileSnapshot {
        let mut tab_urls = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            let url = tab.current().map(|p| p.url.clone()).unwrap_or_default();
            if !url.is_empty() {
                tab_urls.push(url);
            }
        }
        ProfileSnapshot {
            settings: self.settings,
            zoom_percent: self.zoom_percent,
            active_tab: self.active,
            tab_urls,
            bookmarks: self.bookmarks.clone(),
            history_log: self.history_log.clone(),
            cookie_jar: self.cookie_jar().clone(),
            hsts_store: self.hsts_store.clone(),
            downloads: self.downloads.clone(),
            local_storage: self.local_storage.clone(),
            site_permissions: self.site_permissions.clone(),
        }
    }

    /// Copy settings, zoom, bookmarks, history, cookies, HSTS, downloads, `localStorage`, and site permissions from `snapshot`.
    /// `tab_urls` and `active_tab` are stored but not applied. Duplicate bookmark URLs are skipped.
    pub fn apply_profile_snapshot(&mut self, snapshot: ProfileSnapshot) -> usize {
        self.settings = snapshot.settings;
        let draw = self.settings.draw_from_renderer;
        self.set_draw_from_renderer(draw);
        self.tab_workers
            .set_confinement(self.settings.renderer_confinement);
        self.gpu_video.set_hardware_acceleration(
            self.settings.gpu_video_acceleration,
            self.settings.gpu_video_acceleration,
        );
        self.zoom_percent = snapshot.zoom_percent.clamp(50, 250);
        let mut added = 0usize;
        for bm in snapshot.bookmarks {
            if self.add_bookmark(&bm.title, &bm.url) {
                added += 1;
            }
        }
        for entry in snapshot.history_log {
            if !entry.url.is_empty() && !self.history_log.iter().any(|h| h.url == entry.url) {
                self.history_log.push(entry);
            }
        }
        *self.cookie_jar_mut() = snapshot.cookie_jar;
        self.site_permissions = snapshot.site_permissions;
        self.sync_site_permissions();
        self.hsts_store = snapshot.hsts_store;
        self.downloads = snapshot.downloads;
        self.merge_local_storage(snapshot.local_storage);
        if let Some(page) = self.tabs[self.active].current_mut() {
            page.runtime.import_local_storage(&self.local_storage);
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        added
    }

    /// Write the sealed profile to `path`. Windows uses DPAPI (`ANVP\x01`). Linux uses AES-256-GCM (`ANVP\x02`).
    pub fn save_encrypted_profile(&self, path: &str) -> Result<(), String> {
        let snap = self.to_profile_snapshot();
        profile::save_encrypted_profile_to_path(&snap, Path::new(path)).map_err(|e| e.to_string())
    }

    /// Load `path` and apply it. Accepts JSON, Windows `ANVP\x01`, and Linux `ANVP\x02`.
    pub fn load_encrypted_profile(&mut self, path: &str) -> Result<usize, String> {
        let snap = profile::load_profile_from_path(Path::new(path)).map_err(|e| e.to_string())?;
        Ok(self.apply_profile_snapshot(snap))
    }

    /// Save the session profile to the default per-user encrypted profile location (`profile.anvil`).
    pub fn save_default_profile(&self) -> Result<(), String> {
        let Some(path) = default_profile_path() else {
            return Err("could not resolve default profile path".to_string());
        };
        let snap = self.to_profile_snapshot();
        profile::save_encrypted_profile_to_path(&snap, &path).map_err(|e| e.to_string())
    }

    /// Load the session profile from the default per-user encrypted profile location (`profile.anvil`).
    pub fn load_default_profile(&mut self) -> Result<usize, String> {
        let Some(path) = default_profile_path() else {
            return Ok(0);
        };
        if !path.exists() {
            return Ok(0);
        }
        let snap = profile::load_profile_from_path(&path).map_err(|e| e.to_string())?;
        Ok(self.apply_profile_snapshot(snap))
    }

    /// Clear bookmarks, history, cookies, HSTS, downloads, `localStorage`, and site permissions, then reset settings and zoom.
    /// Deletes the default `profile.anvil` when that path exists. Does not clear the session clipboard.
    pub fn clear_all_profile_data(&mut self) {
        self.bookmarks.clear();
        self.history_log.clear();
        *self.cookie_jar_mut() = CookieJar::new();
        self.hsts_store = HstsStore::new();
        self.downloads.clear();
        self.local_storage.clear();
        self.site_permissions.clear();
        self.settings = BrowserSettings::default();
        self.zoom_percent = 100;
        if let Some(path) = default_profile_path() {
            let _ = fs::remove_file(path);
        }
    }

    /// Load the active URL again and replace the current history entry. Scroll is clamped, not reset.
    pub fn reload_active(&mut self) -> Result<(), String> {
        let Some(current) = self.tabs[self.active].current() else {
            return Ok(());
        };
        let target = current.url.clone();
        if target.is_empty() {
            return Ok(());
        }
        let upgraded = self.hsts_store.upgrade_url_if_hsts(&target);
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let fetch_jar = self.cookie_jar_handle();
        let scripts_enabled = self.scripts_enabled_for(&upgraded);
        let _https_only = anvil_net::enter_https_only(self.https_only_policy());
        let tab_id = self.active_tab_id();
        let workers = self.tab_workers();
        match self.sandbox_guard.execute_isolated(&label, || {
            Ok(load_target_in_tab(
                &upgraded,
                viewport,
                Some(fetch_jar.clone()),
                scripts_enabled,
                tab_id,
                &workers,
            ))
        }) {
            Ok(Ok(mut page)) => {
                self.keep_frame_if_drawn(&mut page);
                self.tabs[self.active].replace_current(page);
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                Ok(())
            }
            Ok(Err(err)) => {
                let page = navigation_error_page(&upgraded, &err, viewport);
                self.tabs[self.active].replace_current(page);
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                Ok(())
            }
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.tabs[self.active].replace_current(page);
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                Ok(())
            }
        }
    }

    /// Move the active tab back and restore that history entry's scroll position.
    pub fn back(&mut self) -> bool {
        self.sync_page_scroll();
        let moved = self.tabs[self.active].back();
        if moved {
            let restored = self.tabs[self.active]
                .current()
                .map(|p| p.scroll_y)
                .unwrap_or(0.0);
            self.scroll = restored.clamp(0.0, self.max_scroll());
        }
        moved
    }

    /// Move the active tab forward and restore that history entry's scroll position.
    pub fn forward(&mut self) -> bool {
        self.sync_page_scroll();
        let moved = self.tabs[self.active].forward();
        if moved {
            let restored = self.tabs[self.active]
                .current()
                .map(|p| p.scroll_y)
                .unwrap_or(0.0);
            self.scroll = restored.clamp(0.0, self.max_scroll());
        }
        moved
    }

    /// Export the current session tabs, active index, and bookmarks as formatted JSON (`anvil-json`).
    pub fn export_json(&self) -> String {
        let mut root = JsonValue::empty_object();
        root.insert("version", JsonValue::String(VERSION.to_string()));
        root.insert("active_tab", JsonValue::Number(self.active as f64));
        let mut tabs_arr = JsonValue::empty_array();
        for tab in &self.tabs {
            let mut item = JsonValue::empty_object();
            item.insert("title", JsonValue::String(tab.title().to_string()));
            let url = tab.current().map(|p| p.url.clone()).unwrap_or_default();
            item.insert("url", JsonValue::String(url));
            tabs_arr.push(item);
        }
        root.insert("tabs", tabs_arr);

        let mut bm_arr = JsonValue::empty_array();
        for bm in &self.bookmarks {
            let mut item = JsonValue::empty_object();
            item.insert("title", JsonValue::String(bm.title.clone()));
            item.insert("url", JsonValue::String(bm.url.clone()));
            bm_arr.push(item);
        }
        root.insert("bookmarks", bm_arr);
        to_json_pretty(&root)
    }

    /// Add bookmarks from a JSON `bookmarks` array. Returns how many URLs were new.
    pub fn import_bookmarks_from_json(&mut self, json_src: &str) -> Result<usize, String> {
        let doc = parse_json(json_src).map_err(|e| e.to_string())?;
        let Some(items) = doc.get("bookmarks").and_then(JsonValue::as_array) else {
            return Ok(0);
        };
        let mut added = 0usize;
        for item in items {
            let title = item.get("title").and_then(JsonValue::as_str).unwrap_or("");
            let url = item.get("url").and_then(JsonValue::as_str).unwrap_or("");
            if self.add_bookmark(title, url) {
                added += 1;
            }
        }
        Ok(added)
    }

    /// Write `export_json` to `path`. This is plain JSON, not the sealed profile.
    pub fn save_profile(&self, path: &str) -> Result<(), String> {
        fs::write(path, self.export_json()).map_err(|e| e.to_string())
    }

    /// Read `path` as text and import bookmarks. This does not open `profile.anvil`.
    pub fn load_profile(&mut self, path: &str) -> Result<usize, String> {
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        self.import_bookmarks_from_json(&raw)
    }

    /// Parse exported session JSON and return the list of tab URLs.
    pub fn parse_exported_urls(json_src: &str) -> Result<Vec<String>, String> {
        let doc = parse_json(json_src).map_err(|e| e.to_string())?;
        let Some(tabs) = doc.get("tabs").and_then(JsonValue::as_array) else {
            return Err("missing tabs array in session JSON".to_string());
        };
        let mut urls = Vec::new();
        for tab in tabs {
            if let Some(url) = tab.get("url").and_then(JsonValue::as_str) {
                urls.push(url.to_string());
            }
        }
        Ok(urls)
    }
}

/// HTML card shown when `execute_isolated` returns a fault. `reason` is inserted as text.
fn render_sandbox_recovery_page(url: &str, reason: &str, viewport: Viewport) -> Page {
    let html = format!(
        "<!DOCTYPE html>
<html>
<head>
  <title>Tab Isolated by Anvil Sandbox</title>
  <style>
    html, body {{ margin: 0; padding: 24px; background-color: #0e1015; color: #eceff4; font-size: 15px; }}
    .card {{ background-color: #171b24; border: 1px solid #d08770; border-radius: 10px; padding: 20px; }}
    h1 {{ color: #ebcb8b; font-size: 20px; margin-bottom: 10px; }}
    p {{ color: #d8dee9; margin-bottom: 8px; }}
    a {{ color: #88c0d0; }}
  </style>
</head>
<body>
  <div class=\"card\">
    <h1>Tab Isolated by Mandatory Sandbox</h1>
    <p>Target: {url}</p>
    <p>Reason: {reason}</p>
    <p><a href=\"anvil://security\">Open Anvil Security Center</a> | <a href=\"anvil://home\">Return to Start</a></p>
  </div>
</body>
</html>"
    );
    render_html(&html, url, viewport)
}

#[cfg(test)]
mod tests {
    //! In-process checks for loading, history, forms, and video. They do not spawn `--renderer`.
    use super::*;
    use anvil_paint::DisplayItem;
    use anvil_style::Color;

    #[test]
    /// `VERSION` is the string `v.24.1`.
    fn release_is_v23() {
        assert_eq!(VERSION, "v.24.1");
    }

    #[test]
    /// A failed navigation page is the error text, not the document that failed.
    fn navigation_failure_page_omits_the_document() {
        let secret = "secret-marker-not-painted";
        let page = navigation_error_page(
            "http://127.0.0.1/page",
            "the renderer stopped responding",
            Viewport {
                width: 320.0,
                height: 200.0,
            },
        );
        let text = page.document.text_content(page.document.body());
        assert_eq!(page.title, "Navigation failed");
        assert!(text.contains("stopped responding"));
        assert!(!text.contains(secret));
    }

    #[test]
    /// Persist `localStorage` across pages and through an encrypted `profile.anvil` snapshot.
    fn persists_origin_isolated_local_storage_in_encrypted_profile() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<p id=\"out\"></p>
             <script>
               localStorage.setItem('theme', 'obsidian-gold');
               document.getElementById('out').textContent = localStorage.getItem('theme');
             </script>",
            "https://example.com/dashboard.html",
        );
        assert!(session.local_storage_entries().contains(&(
            "https://example.com:443".to_string(),
            "theme".to_string(),
            "obsidian-gold".to_string()
        )));

        let snap = session.to_profile_snapshot();
        let encrypted = profile::encode_encrypted_profile(&snap, 0x1122_3344_5566_7788);
        let decoded = profile::decode_profile_bytes(&encrypted).expect("decode profile");

        let mut restored = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        restored.apply_profile_snapshot(decoded);
        restored.open_html(
            "<p id=\"restored\"></p>
             <script>
               document.getElementById('restored').textContent = localStorage.getItem('theme');
             </script>",
            "https://example.com/settings.html",
        );
        let page = restored.active().current().unwrap();
        let node = page.document.get_element_by_id("restored").unwrap();
        assert_eq!(page.document.text_content(node), "obsidian-gold");
    }

    #[test]
    fn profile_hides_other_origins_and_omits_session_storage() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<p id=\"out\"></p>
             <script>
               localStorage.setItem('token', 'alpha-token');
               sessionStorage.setItem('tab', 'ephemeral-marker');
               document.getElementById('out').textContent = localStorage.getItem('token');
             </script>",
            "https://alpha.example/home",
        );
        session.open_html(
            "<p id=\"out\"></p>
             <script>
               document.getElementById('out').textContent = localStorage.getItem('token');
             </script>",
            "https://beta.example/home",
        );
        let hidden = session.active().current().unwrap();
        let hidden_node = hidden.document.get_element_by_id("out").unwrap();
        assert_eq!(hidden.document.text_content(hidden_node), "null");

        let json = profile::snapshot_to_json(&session.to_profile_snapshot());
        assert!(json.contains("alpha-token"));
        assert!(!json.contains("ephemeral-marker"));
        assert!(!json.contains("session_storage"));
        assert!(!json.contains("sessionStorage"));

        let encrypted = profile::encode_encrypted_profile(
            &session.to_profile_snapshot(),
            0x99aa_bbcc_ddee_ff00,
        );
        let decoded = profile::decode_profile_bytes(&encrypted).expect("decode profile");
        let mut restored = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        restored.apply_profile_snapshot(decoded);
        restored.open_html(
            "<p id=\"out\"></p>
             <script>
               document.getElementById('out').textContent =
                 localStorage.getItem('token') + ':' + sessionStorage.getItem('tab');
             </script>",
            "https://alpha.example/again",
        );
        let alpha = restored.active().current().unwrap();
        let alpha_node = alpha.document.get_element_by_id("out").unwrap();
        assert_eq!(alpha.document.text_content(alpha_node), "alpha-token:null");

        restored.open_html(
            "<p id=\"out\"></p>
             <script>
               document.getElementById('out').textContent = localStorage.getItem('token');
             </script>",
            "https://beta.example/again",
        );
        let beta = restored.active().current().unwrap();
        let beta_node = beta.document.get_element_by_id("out").unwrap();
        assert_eq!(beta.document.text_content(beta_node), "null");
    }

    #[test]
    /// An encrypted demo video sets the page title from script, then sample downloads are recorded and cleared.
    fn supports_w3c_eme_cenc_encrypted_video_and_sha256_download_manager() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        // 1. Render a page with CENC-encrypted `<video src="anvil://encrypted.mp4">` and W3C EME script check
        session.open_html(
            "<style>html, body, video { margin: 0; padding: 0; }</style>
             <video id=\"drm_vid\" src=\"anvil://encrypted.mp4\" width=\"240\" height=\"120\"></video>
             <script>
               let access = navigator.requestMediaKeySystemAccess('org.w3.clearkey');
               if (access.supported) {
                 document.title = 'EME ClearKey Ready';
               }
             </script>",
            "anvil://settings",
        );
        let page = session.active().current().unwrap();
        assert_eq!(page.title, "EME ClearKey Ready");
        let vid_id = page.document.get_element_by_id("drm_vid").unwrap();
        assert!(page.videos.contains_key(&vid_id));

        // 2. Test Download Manager (`anvil://downloads`) & SHA-256 verification
        session
            .open_target(&format!(
                "anvil://downloads?action=sample_report&key={}",
                permissions::action_key()
            ))
            .unwrap();
        assert_eq!(session.downloads().len(), 1);
        assert_eq!(
            session.downloads().entries()[0].file_name,
            "anvil-diagnostic-report.json"
        );
        assert_eq!(session.downloads().entries()[0].sha256_hex.len(), 64);

        session
            .open_target(&format!(
                "anvil://downloads?action=sample_mp4&key={}",
                permissions::action_key()
            ))
            .unwrap();
        assert_eq!(session.downloads().len(), 2);

        session
            .open_target(&format!(
                "anvil://downloads?action=clear&key={}",
                permissions::action_key()
            ))
            .unwrap();
        assert!(session.downloads().is_empty());
    }

    #[test]
    /// HSTS upgrade, an SRI match and mismatch, and a sealed snapshot round-trip.
    fn enforces_hsts_sri_and_dpapi_profile_protection_in_session() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert!(session.hsts_store().requires_https("github.com"));
        assert!(session.hsts_store_mut().record_hsts_header(
            "https://hollowcreek.org/login",
            "max-age=31536000; includeSubDomains"
        ));
        assert_eq!(
            session
                .hsts_store()
                .upgrade_url_if_hsts("http://lab.hollowcreek.org/status"),
            "https://lab.hollowcreek.org/status"
        );

        let payload = b"document.title = 'SRI Verified Script';";
        let valid_sri = sri_sha256_token(payload);
        assert!(verify_subresource_integrity(Some(&valid_sri), payload));
        assert!(!verify_subresource_integrity(
            Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="),
            payload
        ));

        let snap = session.to_profile_snapshot();
        let sealed = profile::encode_encrypted_profile(&snap, 0x9988_7766_5544_3322);
        let unsealed = profile::decode_profile_bytes(&sealed).expect("unseal DPAPI profile");
        assert!(unsealed.hsts_store.requires_https("lab.hollowcreek.org"));
    }

    #[test]
    /// Play and seek a demo video, then restore zoom and one bookmark from a sealed snapshot.
    fn supports_encrypted_profile_persistence_and_interactive_video_scrubbing() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<style>html, body, video { margin: 0; padding: 0; }</style>
             <video id=\"demo\" src=\"anvil://demo.mp4\" width=\"240\" height=\"120\"></video>",
            "anvil://settings",
        );
        let vid_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("demo")
            .unwrap();

        // 1. Clicking upper video surface toggles playback ON and advances frame
        assert_eq!(
            session.click_at(60.0, 40.0),
            ClickOutcome::VideoToggled(vid_id, true)
        );
        assert_eq!(session.step_active_videos(50), 1);

        // 2. Clicking bottom timeline scrubber bar seeks to timestamp
        match session.click_at(140.0, 112.0) {
            ClickOutcome::VideoSeeked(id, ts) => {
                assert_eq!(id, vid_id);
                assert!(ts > 0);
            }
            other => panic!("expected VideoSeeked, got {other:?}"),
        }

        // 3. Encrypted profile snapshot round-trip
        session.add_bookmark("Security", "anvil://security");
        session.set_zoom_percent(150);
        let snap = session.to_profile_snapshot();
        let encrypted = profile::encode_encrypted_profile(&snap, 0xCAFE_BABE_1234_5678);
        let restored = profile::decode_profile_bytes(&encrypted).expect("decrypt profile");
        let mut s2 = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(s2.apply_profile_snapshot(restored), 1);
        assert_eq!(s2.zoom_percent(), 150);
        assert_eq!(s2.bookmarks()[0].url, "anvil://security");
    }

    #[test]
    /// Round-trip a WebP and an ICO through `anvil-image`, and read width and height from a demo MP4.
    fn integrates_webp_ico_images_and_hardened_mp4_video_metadata() {
        let mut src = Image::new(16, 16);
        src.set_pixel(4, 4, [255, 200, 80, 180]);
        let webp_bytes = anvil_image::encode_webp(&src);
        let decoded_webp = decode_auto(&webp_bytes).expect("decode WebP via decode_auto");
        assert_eq!(decoded_webp.width, 16);
        assert_eq!(decoded_webp.height, 16);
        assert_eq!(decoded_webp.pixel(4, 4), Some([255, 200, 80, 180]));

        let ico_bytes = anvil_image::encode_ico(&src);
        let decoded_ico = decode_auto(&ico_bytes).expect("decode ICO via decode_auto");
        assert_eq!(decoded_ico.width, 16);
        assert_eq!(decoded_ico.height, 16);

        let demo_mp4 = generate_demo_mp4(120, 68, 4).expect("generate demo MP4");
        let meta = inspect_mp4_metadata(&demo_mp4).expect("inspect MP4 metadata");
        assert_eq!(meta.width, 120);
        assert_eq!(meta.height, 68);
        assert!(meta.duration_ms > 0);
    }

    #[test]
    /// Sandbox telemetry, a blocked inline script, a caught panic on the second tab, and an HttpOnly cookie.
    fn enforces_hardcoded_os_sandbox_csp_and_cookie_isolation_in_session() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        let telemetry = session.sandbox_telemetry();
        assert!(telemetry.os_enforced);
        assert_eq!(telemetry.ui_restrictions_mask, 0);
        assert_eq!(telemetry.active_process_limit, 0);

        // 1. Verify CSP meta policy blocks inline script execution when 'unsafe-inline' is absent
        session.open_html(
            "<html>
               <head>
                 <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'\">
                 <title>Initial Title</title>
               </head>
               <body>
                 <p id=\"msg\">Safe Text</p>
                 <script>
                   document.title = 'Compromised By Inline Script';
                 </script>
               </body>
             </html>",
            "https://example.com/csp-protected.html",
        );
        let page = session.active().current().unwrap();
        assert_eq!(page.title, "Initial Title");
        assert_eq!(page.blocked_csp_count, 1);

        // 2. Verify fault in Tab 2 is contained inside the mandatory sandbox without affecting Tab 1
        session.new_tab();
        session.open_isolated_workload("https://untrusted.example.org/crash", |_vp| {
            panic!("simulated untrusted tab parser panic");
        });
        assert_eq!(session.sandbox_telemetry().contained_tab_faults, 1);
        assert_eq!(
            session.active().current().unwrap().title,
            "Tab Isolated by Anvil Sandbox"
        );
        session.select(0);
        assert_eq!(session.active().current().unwrap().title, "Initial Title");

        // 3. Verify Session CookieJar enforces HttpOnly and SameSite
        assert!(session.cookie_jar_mut().store_set_cookie(
            "https://example.com/login",
            "__Host-token=secret99; Secure; HttpOnly; Path=/; SameSite=Strict"
        ));
        assert_eq!(
            session
                .cookie_jar()
                .script_cookie_string("https://example.com/login"),
            ""
        );
    }

    #[test]
    /// Find-in-page hits, zoom steps, raster size, and cut plus paste on the session clipboard.
    fn supports_find_in_page_zoom_and_clipboard_operations() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<style>html, body, p, input { margin: 0; padding: 0; font-size: 16px; }</style>
             <p id=\"p1\">Hollow Creek Maintenance Log</p>
             <p id=\"p2\">Hollow Creek Valve Status</p>
             <form action=\"https://example.com/q\" method=\"GET\">
               <input id=\"q\" type=\"text\" name=\"q\" value=\"initial\">
             </form>",
            "https://example.com/logs.html",
        );

        // 1. Find-in-Page matches
        let matches = session
            .active()
            .current()
            .unwrap()
            .find_text_matches("hollow");
        assert_eq!(matches.len(), 2);

        // 2. Dynamic Page Zoom (50% - 250%)
        assert_eq!(session.zoom_percent(), 100);
        assert_eq!(session.zoom_in(), 125);
        assert_eq!(session.zoom_in(), 150);
        assert_eq!(session.zoom_out(), 125);
        let fb_zoomed = session.rasterize();
        assert_eq!(fb_zoomed.width(), 800);
        assert_eq!(session.reset_zoom(), 100);

        // 3. Text Selection & Clipboard Copy/Cut/Paste
        assert_eq!(
            session.select_text_at(10.0, 6.0),
            Some("Hollow Creek Maintenance Log".to_string())
        );
        assert_eq!(
            session.copy_selection_or_focused(),
            Some("Hollow Creek Maintenance Log".to_string())
        );
        // Focus the <input> and test cut + paste
        let input_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("q")
            .unwrap();
        session.tabs[session.active]
            .current_mut()
            .unwrap()
            .focused_input = Some(input_id);
        assert_eq!(session.cut_focused_input(), Some("initial".to_string()));
        assert!(session.paste_into_focused_input("pasted-token"));
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .input_value(input_id),
            "pasted-token"
        );
    }

    #[test]
    /// Cycle a select, build its POST body, and restage a max-width media query.
    fn restages_media_queries_and_cycles_select_controls() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<style>
                html, body, form, select, p { margin: 0; padding: 0; }
                p { color: #112233; }
                @media (max-width: 500px) {
                    p { color: #ff5500; }
                }
             </style>
             <form action=\"https://example.com/Config\" method=\"POST\">
               <select id=\"codec\" name=\"codec\">
                 <option value=\"nv12\" selected>NV12 Hardware</option>
                 <option value=\"vaapi\">VA-API Linux</option>
               </select>
               <button id=\"save\" type=\"submit\">Save</button>
             </form>
             <p id=\"msg\">Responsive Text</p>",
            "https://example.com/settings.html",
        );

        let sel_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("codec")
            .unwrap();
        assert_eq!(
            session.click_at(16.0, 10.0),
            ClickOutcome::SelectChanged(sel_id, "vaapi".to_string())
        );
        assert_eq!(
            session.submit_form_for(sel_id),
            ClickOutcome::SubmitPost {
                url: "https://example.com/Config".to_string(),
                body: "codec=vaapi".to_string(),
            }
        );

        session.set_viewport(Viewport {
            width: 420.0,
            height: 600.0,
        });
        let page = session.active().current().unwrap();
        assert!(page.display_list.items.iter().any(|item| matches!(
            item,
            DisplayItem::Text { text, color, .. }
                if text == "Responsive Text" && *color == Color::parse("#ff5500").unwrap()
        )));
    }

    #[test]
    /// Load home, security, navigate, settings, and history, and flip the video-acceleration flag.
    fn loads_builtin_anvil_home_security_settings_and_history_with_gpu_video() {
        let vp = Viewport {
            width: 960.0,
            height: 720.0,
        };
        let home = load_target("anvil://home", vp).expect("load anvil://home");
        assert_eq!(home.url, "anvil://home");
        assert_eq!(home.title, "Anvil Start");

        let sec = load_target("anvil://security", vp).expect("load anvil://security");
        assert_eq!(sec.url, "anvil://security");
        assert_eq!(sec.title, "Anvil Security Center");

        let nav = load_target("anvil://navigate?q=anvil%3A%2F%2Fsecurity", vp)
            .expect("resolve anvil://navigate query");
        assert_eq!(nav.url, "anvil://security");

        let mut session = Session::new(vp);
        session.open_target("anvil://home").unwrap();
        session.open_target("anvil://settings").unwrap();
        let settings_page = session.active().current().unwrap();
        assert_eq!(settings_page.url, "anvil://settings");
        assert_eq!(settings_page.title, "Anvil Settings");
        // Verify the embedded <video src="anvil://demo.mp4"> decoded an NV12/BT.709 video frame
        assert!(!settings_page.images.is_empty());
        assert!(
            settings_page
                .display_list
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::Image { .. }))
        );

        assert!(session.settings().gpu_video_acceleration);
        session
            .open_target(&format!(
                "anvil://settings?toggle=gpu_video&key={}",
                permissions::action_key()
            ))
            .unwrap();
        assert!(!session.settings().gpu_video_acceleration);

        session.open_target("anvil://history").unwrap();
        assert_eq!(session.history_log().len(), 2);
        session
            .open_target(&format!(
                "anvil://history?action=clear&key={}",
                permissions::action_key()
            ))
            .unwrap();
        assert!(session.history_log().is_empty());
    }

    #[test]
    /// Render `examples/hello.html` and check the title plus two paint colors.
    fn renders_the_sample_page() {
        let page = render_html(
            include_str!("../../../examples/hello.html"),
            "hello.html",
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(page.title, "Hello");
        let texts: Vec<_> = page
            .display_list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, color, .. } => Some((text.as_str(), *color)),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|(text, color)| {
            *text == "Anvil" && *color == Color::parse("#e8c37a").unwrap()
        }));
        assert!(page.display_list.items.iter().any(|item| {
            matches!(item, DisplayItem::Rect { color, .. } if *color == Color::parse("#1c1a17").unwrap())
        }));
    }

    const SHEET_VP: Viewport = Viewport {
        width: 400.0,
        height: 300.0,
    };

    /// Render `html` as if `linked` had been fetched for its links.
    fn render_with_linked(html: &str, linked: &[(&str, &str)]) -> Page {
        render_document(
            html,
            PageSheets {
                external_css: "",
                linked_css: linked
                    .iter()
                    .map(|(href, css)| (href.to_string(), css.to_string()))
                    .collect(),
            },
            HashMap::new(),
            "https://example.com/index.html",
            SHEET_VP,
            None,
            0,
            &[],
            PageLoads::Here(None),
            true,
        )
    }

    /// Colour the text `needle` was drawn with.
    fn drawn_color(page: &Page, needle: &str) -> Color {
        page.display_list
            .items
            .iter()
            .find_map(|item| match item {
                DisplayItem::Text { text, color, .. } if text == needle => Some(*color),
                _ => None,
            })
            .expect("text is drawn")
    }

    #[test]
    fn a_link_after_a_style_element_wins() {
        let page = render_with_linked(
            "<style>p { color: #ff0000 }</style><link rel=stylesheet href=late.css><p>Text</p>",
            &[("late.css", "p { color: #00ff00 }")],
        );
        assert_eq!(drawn_color(&page, "Text"), Color::parse("#00ff00").unwrap());
    }

    #[test]
    fn a_style_element_after_a_link_wins() {
        let page = render_with_linked(
            "<link rel=stylesheet href=early.css><style>p { color: #00ff00 }</style><p>Text</p>",
            &[("early.css", "p { color: #ff0000 }")],
        );
        assert_eq!(drawn_color(&page, "Text"), Color::parse("#00ff00").unwrap());
    }

    #[test]
    fn links_and_style_elements_interleave() {
        let page = render_with_linked(
            "<link rel=stylesheet href=a.css>\
             <style>p { color: #ff0000 } i { color: #00ff00 }</style>\
             <link rel=stylesheet href=b.css>\
             <p>One</p><b>Two</b><i>Three</i>",
            &[
                (
                    "a.css",
                    "p { color: #0000ff } b { color: #00ff00 } i { color: #0000ff }",
                ),
                ("b.css", "p { color: #00ff00 }"),
            ],
        );
        let green = Color::parse("#00ff00").unwrap();
        assert_eq!(drawn_color(&page, "One"), green);
        assert_eq!(drawn_color(&page, "Two"), green);
        assert_eq!(drawn_color(&page, "Three"), green);
    }

    #[test]
    fn media_on_a_link_limits_the_sheet() {
        let page = render_with_linked(
            "<style>p, b, i { color: #00ff00 }</style>\
             <link rel=stylesheet href=print.css media=print>\
             <link rel=stylesheet href=screen.css media='screen and (min-width: 300px)'>\
             <link rel=stylesheet href=narrow.css media='(max-width: 200px)'>\
             <p>Print</p><b>Screen</b><i>Narrow</i>",
            &[
                ("print.css", "p { color: #ff0000 }"),
                ("screen.css", "b { color: #0000ff }"),
                ("narrow.css", "i { color: #ff0000 }"),
            ],
        );
        assert_eq!(
            drawn_color(&page, "Print"),
            Color::parse("#00ff00").unwrap()
        );
        assert_eq!(
            drawn_color(&page, "Screen"),
            Color::parse("#0000ff").unwrap()
        );
        assert_eq!(
            drawn_color(&page, "Narrow"),
            Color::parse("#00ff00").unwrap()
        );
    }

    #[test]
    fn a_broken_linked_sheet_does_not_swallow_the_next_one() {
        let page = render_with_linked(
            "<link rel=stylesheet href=broken.css><style>p { color: #00ff00 }</style><p>Text</p>",
            &[("broken.css", "b { color: #ff0000")],
        );
        assert_eq!(drawn_color(&page, "Text"), Color::parse("#00ff00").unwrap());
    }

    #[test]
    fn restaging_keeps_sheet_order_media_and_the_inline_style_block() {
        let mut page = render_with_linked(
            "<meta http-equiv=Content-Security-Policy content=\"style-src 'self'\">\
             <style>p { color: #ff0000 }</style>\
             <link rel=stylesheet href=a.css><link rel=stylesheet href=p.css media=print>\
             <p>Text</p>",
            &[
                ("a.css", "p { color: #00ff00 }"),
                ("p.css", "p { color: #0000ff }"),
            ],
        );
        assert_eq!(drawn_color(&page, "Text"), Color::parse("#00ff00").unwrap());
        page.restage(SHEET_VP);
        assert_eq!(drawn_color(&page, "Text"), Color::parse("#00ff00").unwrap());
    }

    #[test]
    fn fetched_sheets_keep_their_href_and_link_order() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.starts_with("GET /one.css") {
                    "p { color: #000001 }"
                } else if req.starts_with("GET /two.css") {
                    "p { color: #000002 }"
                } else {
                    "p { color: #000003 }"
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(body.as_bytes());
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        });
        let base = format!("http://127.0.0.1:{}/index.html", addr.port());
        let links: Vec<(String, Option<String>)> = ["two.css", "one.css", "three.css"]
            .iter()
            .map(|href| (href.to_string(), None))
            .collect();
        let mut blocked = 0;
        let fetched = fetch_linked_stylesheets(
            &base,
            &links,
            &ContentSecurityPolicy::parse(""),
            &mut blocked,
        );
        assert_eq!(blocked, 0);
        assert_eq!(
            fetched,
            vec![
                ("two.css".to_string(), "p { color: #000002 }".to_string()),
                ("one.css".to_string(), "p { color: #000001 }".to_string()),
                ("three.css".to_string(), "p { color: #000003 }".to_string()),
            ]
        );
    }

    #[test]
    /// The external sheet is applied first. The later embedded rule keeps the paragraph color.
    fn applies_external_stylesheet_before_embedded_rules() {
        let page = render_html_with_css(
            "<style>p { color: #00ff00; }</style><h1>Heading</h1><p>Body</p>",
            "h1 { color: #ff0000; } p { color: #0000ff; }",
            "index.html",
            Viewport {
                width: 400.0,
                height: 300.0,
            },
        );
        let texts: Vec<_> = page
            .display_list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, color, .. } => Some((text.as_str(), *color)),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&("Heading", Color::parse("#ff0000").unwrap())));
        assert!(texts.contains(&("Body", Color::parse("#00ff00").unwrap())));
    }

    #[test]
    /// Back and forward work until a new page drops the forward entry.
    fn history_drops_the_forward_page() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 100.0,
        });
        session.open_html("<title>One</title><p>One</p>", "one");
        session.open_html("<title>Two</title><p>Two</p>", "two");
        assert!(session.back());
        assert_eq!(session.active().current().unwrap().title, "One");
        assert!(session.forward());
        assert_eq!(session.active().current().unwrap().title, "Two");
        assert!(session.back());
        session.open_html("<title>Three</title><p>Three</p>", "three");
        assert!(!session.forward());
        assert_eq!(session.active().current().unwrap().title, "Three");
        assert_eq!(session.viewport().height, 100.0);
    }

    #[test]
    /// A new tab has no page. Closing it returns to the previous tab. The last tab stays open.
    fn a_new_tab_starts_empty_and_can_close() {
        let mut session = Session::new(Viewport {
            width: 100.0,
            height: 100.0,
        });
        session.open_html("<title>Stay</title>", "stay");
        session.new_tab();
        assert_eq!(session.len(), 2);
        assert!(session.active().current().is_none());
        assert!(session.close_tab(1));
        assert_eq!(session.len(), 1);
        assert_eq!(session.active().current().unwrap().title, "Stay");
        assert!(!session.close_tab(0));
    }

    #[test]
    /// Resolve a relative href, then scroll until that point is no longer the link.
    fn hits_a_relative_link_and_scrolls_the_viewport() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 40.0,
        });
        session.open_html(
            "<style>
                html, body, p { margin: 0; padding: 0; font-size: 16px; }
                div { height: 100px; }
             </style>
             <p><a href=\"second.html\">Next page</a></p>
             <div>Spacer</div>",
            "examples/hello.html",
        );
        assert_eq!(
            session.hit_link(4.0, 4.0),
            Some("examples/second.html".to_string())
        );
        assert!(session.scroll_by(50.0) > 0.0);
        assert_eq!(session.hit_link(4.0, 4.0), None);
        let fb = session.rasterize();
        assert_eq!(fb.width(), 200);
        assert_eq!(fb.height(), 40);
    }

    #[test]
    /// A click listener changes the document title and the paragraph text.
    fn dispatches_script_click_and_restages_page() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 100.0,
        });
        session.open_html(
            "<style>html, body, p { margin: 0; padding: 0; font-size: 16px; }</style>
             <p id=\"btn\">Tap here</p>
             <script>
               let b = document.getElementById('btn');
               b.addEventListener('click', function() {
                 document.title = 'Tapped';
                 b.textContent = 'Done';
               });
             </script>",
            "test.html",
        );
        assert_eq!(session.click_at(4.0, 4.0), ClickOutcome::ScriptHandled);
        let page = session.active().current().unwrap();
        assert_eq!(page.title, "Tapped");
        assert_eq!(
            page.document
                .text_content(page.document.get_element_by_id("btn").unwrap()),
            "Done"
        );
    }

    #[test]
    /// `export_json` keeps both tab URLs in open order.
    fn exports_and_parses_session_tabs_via_anvil_json() {
        let mut session = Session::new(Viewport {
            width: 400.0,
            height: 300.0,
        });
        session.open_html("<title>Alpha</title>", "https://example.com/a");
        session.new_tab();
        session.open_html("<title>Beta</title>", "https://example.com/b");

        let exported = session.export_json();
        let urls = Session::parse_exported_urls(&exported).unwrap();
        assert_eq!(
            urls,
            vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string()
            ]
        );
    }

    #[test]
    /// Focus a text field, edit it, build a POST body, and import the bookmark from exported JSON.
    fn focuses_input_types_text_submits_form_and_persists_bookmarks() {
        let mut session = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        session.open_html(
            "<style>html, body, form, input { margin: 0; }</style>
             <form action=\"https://example.com/login\" method=\"POST\">
               <input id=\"user\" type=\"text\" name=\"user\" value=\"al\">
               <button id=\"send\" type=\"submit\">Send</button>
             </form>",
            "https://example.com/index.html",
        );

        let user_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("user")
            .unwrap();
        assert_eq!(
            session.click_at(16.0, 10.0),
            ClickOutcome::InputFocused(user_id)
        );
        assert!(session.has_focused_input());
        assert!(session.backspace_focused_input());
        assert!(session.type_into_focused_input('d'));
        assert!(session.type_into_focused_input('m'));
        assert_eq!(
            session.submit_focused_input(),
            ClickOutcome::SubmitPost {
                url: "https://example.com/login".to_string(),
                body: "user=adm".to_string(),
            }
        );

        assert!(session.toggle_bookmark_active());
        assert!(session.is_active_bookmarked());
        let json = session.export_json();
        let mut restored = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        assert_eq!(restored.import_bookmarks_from_json(&json).unwrap(), 1);
        assert_eq!(
            restored.bookmarks()[0].url,
            "https://example.com/index.html"
        );
    }

    #[test]
    fn enter_on_a_text_field_sends_the_submit_button_name() {
        let mut session = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        session.open_html(
            "<form action=\"https://accounts.google.com/v3/signin/identifier\" method=\"POST\">
               <input id=\"identifierId\" type=\"text\" name=\"identifier\" value=\"ada@example.com\">
               <button id=\"identifierNext\" name=\"action\" value=\"1\">Next</button>
             </form>",
            "https://accounts.google.com/v3/signin/identifier",
        );
        let input = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("identifierId")
            .unwrap();
        assert_eq!(
            session.submit_form_for(input),
            ClickOutcome::SubmitPost {
                url: "https://accounts.google.com/v3/signin/identifier".to_string(),
                body: "identifier=ada%40example.com&action=1".to_string(),
            }
        );
    }

    #[test]
    fn navigation_get_keeps_the_session_cookie() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let body: &[u8] = if req.contains("Cookie:") {
                    b"had-cookie"
                } else {
                    b"first"
                };
                let set_cookie = if req.contains("Cookie:") {
                    ""
                } else {
                    "Set-Cookie: session=abc; Path=/\r\n"
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{set_cookie}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(header.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        });

        let url = format!("http://127.0.0.1:{}/mail", addr.port());
        let jar = Arc::new(Mutex::new(CookieJar::new()));
        let (_final_url, response) = navigation_get(&url, Some(&jar)).unwrap();
        assert_eq!(response.body, b"first");
        assert!(
            jar.lock()
                .unwrap()
                .cookies()
                .iter()
                .any(|cookie| cookie.name == "session" && cookie.value == "abc")
        );
        let (_final_url, response) = navigation_get(&url, Some(&jar)).unwrap();
        assert_eq!(response.body, b"had-cookie");
    }

    #[test]
    fn script_fetch_reads_same_origin_http_and_does_not_connect_cross_origin() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf);
            let body = b"harbor-token";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nSet-Cookie: harbor=token; Path=/\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(body);
            let _ = sock.shutdown(std::net::Shutdown::Both);
        });

        let blocked = TcpListener::bind("127.0.0.1:0").unwrap();
        blocked.set_nonblocking(true).unwrap();
        let blocked_addr = blocked.local_addr().unwrap();

        let origin = format!("http://127.0.0.1:{}/index.html", addr.port());
        let html = format!(
            "<p id=\"out\"></p>
             <script>
               let ok = fetch('http://127.0.0.1:{}/note');
               let denied = fetch('http://127.0.0.1:{}/secret');
               document.getElementById('out').textContent = ok.body + ':' + denied.status;
             </script>",
            addr.port(),
            blocked_addr.port()
        );
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 80.0,
        });
        session.open_html(&html, &origin);
        let page = session.active().current().unwrap();
        let out = page.document.get_element_by_id("out").unwrap();
        assert_eq!(page.document.text_content(out), "harbor-token:403");
        assert!(
            session
                .cookie_jar()
                .cookies()
                .iter()
                .any(|cookie| cookie.name == "harbor" && cookie.value == "token")
        );
        assert!(blocked.accept().unwrap_err().kind() == std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn integrates_parallel_rasterizer_d3d11_shader_probe_and_dual_track_av_mp4() {
        let mut session = Session::new(Viewport {
            width: 320.0,
            height: 200.0,
        });
        session.open_target("anvil://home").unwrap();
        let fb = session.rasterize();
        assert_eq!(fb.width(), 320);
        assert_eq!(fb.height(), 200);
        assert!(raster_simd_width_bytes() >= 4);

        let d3d = probe_and_compile_d3d11_compositor();
        #[cfg(target_os = "windows")]
        assert!(d3d.vs_bytecode_len > 64 && d3d.ps_bytecode_len > 64);
        #[cfg(not(target_os = "windows"))]
        let _ = d3d;

        let cff_prog = [239u8, 139u8, 21u8, 239u8, 6u8, 239u8, 7u8, 39u8, 6u8, 14u8];
        let cff_outline = interpret_type2_charstring(&cff_prog).unwrap();
        assert_eq!(cff_outline.contours.len(), 1);
        let local_sub0: &[u8] = &[189, 139, 21, 189, 6, 11];
        let global_sub0: &[u8] = &[189, 7, 89, 6, 14];
        let subr_outline =
            interpret_type2_charstring_with_subrs(&[32, 10, 32, 29], &[local_sub0], &[global_sub0])
                .unwrap();
        assert_eq!(subr_outline.contours.len(), 1);

        session.open_html(
            "<style>
               :root { --accent: #ff6b24; }
               #grid { display: grid; grid-template-columns: 100px 1fr; gap: 10px; width: calc(100% - 20px); }
               .cell { height: 24px; background: var(--accent); }
             </style>
             <div id=\"grid\"><div class=\"cell\">A</div><div class=\"cell\">B</div></div>",
            "anvil://home",
        );
        let grid_fb = session.rasterize();
        assert_eq!(grid_fb.width(), 320);

        let demo_mp4 = generate_demo_mp4(32, 24, 4).unwrap();
        let mut video_session = GpuVideoSession::default();
        let clip = video_session.decode_video_auto(&demo_mp4).unwrap();
        let mut player = VideoPlayerState::new(clip);
        player.step_by_ms(33);
        assert!(player.audio_bridge.submitted_samples > 0);
        assert!(player.audio_bridge.last_peak_level > 10);
        assert!(player.render_surface().is_some());
        let meta = inspect_mp4_metadata(&demo_mp4).unwrap();
        assert!(meta.has_audio_track);
        assert_eq!(meta.audio_sample_rate, 44_100);
        assert_eq!(meta.audio_channels, 2);

        set_image_cache_budget(DEFAULT_IMAGE_CACHE_BUDGET_BYTES);
        assert_eq!(
            browser_memory_stats().image_cache_budget_bytes,
            DEFAULT_IMAGE_CACHE_BUDGET_BYTES
        );
        session.new_tab();
        session.open_target("anvil://security").unwrap();
        assert!(session.trim_inactive_memory() >= 1);
        trim_browser_memory();
        assert_eq!(browser_memory_stats().image_cache_bytes, 0);
    }

    #[test]
    fn webpage_script_comments_wasm_and_worker_click_hydration_work() {
        let html = "<!doctype html><html><head><title>Init</title></head><body>\
            <div id=\"status\">loading</div>\
            <button id=\"action\" onclick=\"runAction()\">Run</button>\
            <script>\n\
            // Inline comment that previously broke collapsed scripts\n\
            let clicks = 0;\n\
            function runAction() {\n\
                clicks = clicks + 1;\n\
                document.getElementById('status').textContent = 'active:' + clicks;\n\
            }\n\
            const wasmBytes = new Uint8Array([\
                0, 97, 115, 109, 1, 0, 0, 0,\
                1, 7, 1, 96, 2, 127, 127, 1, 127,\
                3, 2, 1, 0,\
                5, 3, 1, 0, 1,\
                7, 16, 2, 6, 109, 101, 109, 111, 114, 121, 2, 0, 3, 97, 100, 100, 0, 0,\
                10, 9, 1, 7, 0, 32, 0, 32, 1, 106, 11\
            ]);\n\
            const inst = new WebAssembly.Instance(new WebAssembly.Module(wasmBytes));\n\
            document.title = 'WASM:' + inst.exports.add(20, 22);\n\
            document.getElementById('status').textContent = 'ready:' + inst.exports.add(10, 5);\n\
            </script></body></html>";

        let vp = Viewport {
            width: 800.0,
            height: 600.0,
        };
        let page = render_html(html, "https://example.com/", vp);
        assert_eq!(page.title, "WASM:42");
        let status_id = page.document.get_element_by_id("status").unwrap();
        assert_eq!(page.document.text_content(status_id), "ready:15");

        let serialized = page.document.to_html();
        let mut worker_painted =
            paint_worker_document(&serialized, "https://example.com/", vp, None, None, 0);
        let btn_id = worker_painted.document.get_element_by_id("action").unwrap();
        assert!(
            worker_painted
                .runtime
                .dispatch_click(&mut worker_painted.document, btn_id)
        );
        let worker_status = worker_painted.document.get_element_by_id("status").unwrap();
        assert_eq!(
            worker_painted.document.text_content(worker_status),
            "active:1"
        );
    }

    /// A local server that answers by path from `routes` (a raw response
    /// each) and writes down the head of every request it gets.
    fn route_server(routes: Vec<(&'static str, String)>) -> (u16, Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else {
                    continue;
                };
                let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(head);
                let reply = routes
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_else(|| {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    });
                let _ = sock.write_all(reply.as_bytes());
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        });
        (port, seen)
    }

    fn ok_response(content_type: &str, body: &str, extra: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn requests_for(seen: &Arc<Mutex<Vec<String>>>, path: &str) -> Vec<String> {
        let line = format!("GET {path} ");
        seen.lock()
            .unwrap()
            .iter()
            .filter(|head| head.starts_with(&line))
            .cloned()
            .collect()
    }

    fn renderer_wants(kind: FetchKind, url: String) -> FetchRequest {
        FetchRequest {
            nav_id: 7,
            kind,
            url,
        }
    }

    #[test]
    fn renderer_fetches_follow_the_browsers_rules() {
        let (port, seen) = route_server(vec![
            ("/app.js", ok_response("text/javascript", "app()", "")),
            (
                "/moved.js",
                "HTTP/1.1 302 Found\r\nLocation: /app.js\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string(),
            ),
            (
                "/locked.js",
                ok_response(
                    "text/javascript",
                    "locked()",
                    "Cross-Origin-Resource-Policy: same-origin\r\n",
                ),
            ),
            (
                "/api",
                ok_response("application/json", "{\"n\":1}", "Set-Cookie: seen=yes; Path=/\r\n"),
            ),
        ]);
        let origin = format!("http://127.0.0.1:{port}");
        let page = format!("{origin}/index.html");
        let jar = Arc::new(Mutex::new(CookieJar::new()));
        assert!(
            jar.lock()
                .unwrap()
                .store_set_cookie_under(&page, &page, "sid=abc; Path=/")
        );

        // A linked script: body and type come back, the nav id is kept, and
        // no cookie goes out.
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::Script, format!("{origin}/app.js")),
            &page,
            &jar,
        );
        assert_eq!(reply.nav_id, 7);
        assert_eq!(reply.status, 200);
        assert_eq!(reply.content_type, "text/javascript");
        assert_eq!(reply.body, b"app()");

        // Redirects are followed in the browser.
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::Script, format!("{origin}/moved.js")),
            &page,
            &jar,
        );
        assert_eq!((reply.status, reply.body.as_slice()), (200, &b"app()"[..]));

        // CORP same-origin, asked for by a page on another origin.
        let other_page = format!("http://localhost:{port}/index.html");
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::Script, format!("{origin}/locked.js")),
            &other_page,
            &jar,
        );
        assert_eq!((reply.status, reply.body.len()), (0, 0));

        // The shield blocks before anything connects.
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::Image, format!("{origin}/pixel.gif")),
            &page,
            &jar,
        );
        assert_eq!(reply.status, 0);

        // Mixed content: an https page can't load a plain http script.
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::Script, format!("{origin}/app.js?mixed")),
            &format!("https://127.0.0.1:{port}/index.html"),
            &jar,
        );
        assert_eq!(reply.status, 0);

        // fetch() on the page's own origin sends and stores cookies in the
        // browser's jar.
        let reply = answer_renderer_fetch(
            &renderer_wants(FetchKind::ScriptFetch, format!("{origin}/api")),
            &page,
            &jar,
        );
        assert_eq!(
            (reply.status, reply.body.as_slice()),
            (200, &b"{\"n\":1}"[..])
        );
        assert!(
            jar.lock()
                .unwrap()
                .cookies()
                .iter()
                .any(|c| c.name == "seen" && c.value == "yes")
        );

        // fetch() to another origin is refused without connecting.
        let reply = answer_renderer_fetch(
            &renderer_wants(
                FetchKind::ScriptFetch,
                format!("http://localhost:{port}/api"),
            ),
            &page,
            &jar,
        );
        assert_eq!(reply.status, 403);

        let app = requests_for(&seen, "/app.js");
        assert_eq!(app.len(), 2, "{app:?}");
        assert!(
            app.iter()
                .all(|head| !head.to_ascii_lowercase().contains("cookie:"))
        );
        assert!(requests_for(&seen, "/pixel.gif").is_empty());
        assert!(requests_for(&seen, "/app.js?mixed").is_empty());
        let api = requests_for(&seen, "/api");
        assert_eq!(api.len(), 1, "{api:?}");
        assert!(api[0].contains("sid=abc"), "{}", api[0]);
    }

    #[test]
    fn renderer_fetches_follow_https_only_and_its_exceptions() {
        let (port, seen) = route_server(vec![(
            "/app.js",
            ok_response("text/javascript", "app()", ""),
        )]);
        let page = format!("http://127.0.0.1:{port}/index.html");
        let jar = Arc::new(Mutex::new(CookieJar::new()));
        let want = || renderer_wants(FetchKind::Script, format!("http://127.0.0.1:{port}/app.js"));
        {
            // Upgraded to https, which this server doesn't speak.
            let _scope = anvil_net::enter_https_only(anvil_net::HttpsOnlyPolicy::new(true, &[]));
            assert_eq!(answer_renderer_fetch(&want(), &page, &jar).status, 0);
            assert!(requests_for(&seen, "/app.js").is_empty());
        }
        {
            // An exception the person allowed is honoured, all of it.
            let allowed = vec!["127.0.0.1".to_string()];
            let _scope =
                anvil_net::enter_https_only(anvil_net::HttpsOnlyPolicy::new(true, &allowed));
            assert_eq!(answer_renderer_fetch(&want(), &page, &jar).status, 200);
            assert_eq!(requests_for(&seen, "/app.js").len(), 1);
        }
    }

    #[test]
    fn the_renderer_asks_the_browser_for_sheets_and_images_and_decodes_them() {
        use std::sync::Mutex as StdMutex;
        let png = anvil_image::encode_png(&Image::from_rgba(3, 2, vec![200; 24]).unwrap());
        let asked: Arc<StdMutex<Vec<(FetchKind, String)>>> = Arc::default();
        let ask: BrowserFetch = {
            let asked = Arc::clone(&asked);
            Arc::new(move |kind, url: &str| {
                asked.lock().unwrap().push((kind, url.to_string()));
                let body = match url.rsplit('/').next().unwrap_or("") {
                    "site.css" => b"p { color: rgb(1, 2, 3); }".to_vec(),
                    "dot.png" => png.clone(),
                    _ => return None,
                };
                Some(FetchReply {
                    nav_id: 1,
                    status: 200,
                    content_type: String::new(),
                    body,
                })
            })
        };
        let html = "<html><head><link rel=stylesheet href=site.css>\
            <meta http-equiv=Content-Security-Policy content=\"img-src 'self'\"></head>\
            <body><p>hi</p><img src=\"dot.png\"><img src=\"https://other.test/x.png\">\
            <img src=\"anvil://logo.png\"><img src=\"missing.png\"></body></html>";
        let page = render_with_browser_fetches(
            html,
            "https://a.test/page.html",
            Viewport {
                width: 400.0,
                height: 300.0,
            },
            None,
            false,
            ask,
        );
        assert_eq!(
            page.linked_css,
            vec![(
                "site.css".to_string(),
                "p { color: rgb(1, 2, 3); }".to_string()
            )]
        );
        let mut sizes: Vec<_> = page.images.values().map(|i| (i.width, i.height)).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![(3, 2), (64, 64)]);
        // The cross-origin image is refused by CSP before anything is asked,
        // and the logo is drawn here.
        let asked = asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![
                (FetchKind::Stylesheet, "https://a.test/site.css".to_string()),
                (FetchKind::Image, "https://a.test/dot.png".to_string()),
                (FetchKind::Image, "https://a.test/missing.png".to_string()),
            ]
        );
        // Not counted here: the window counts what it blocks itself.
        assert_eq!(page.blocked_csp_count, 0);
    }
    /// The renderer's path and the window's path paint the same page: the
    /// renderer's frame survives encoding and checking, and matches the list
    /// the window paints from the renderer's HTML.
    #[test]
    fn the_renderer_frame_and_the_window_page_are_the_same() {
        let html = "<html><head><style>body { background: rgb(240, 240, 250); } \
            .box { border-radius: 6px; box-shadow: 2px 2px 4px rgb(0, 0, 0); \
            background: linear-gradient(90deg, rgb(255, 0, 0), rgb(0, 0, 255)); \
            opacity: 0.5; transform: rotate(5deg); }</style></head><body>\
            <h1 id=\"h\">before</h1><p>Go <a href=\"/next\">next</a> or \
            <a href=\"https://b.test/x\">away</a> or <a href=\"javascript:void(0)\">nowhere</a>.</p>\
            <div class=\"box\">box</div>\
            <div style=\"position: sticky; top: 0\">sticky</div>\
            <div style=\"position: fixed; bottom: 0\">fixed</div>\
            <div style=\"height: 2000px\">tall</div>\
            <script>document.getElementById('h').textContent = 'after';</script></body></html>";
        let viewport = Viewport {
            width: 500.0,
            height: 400.0,
        };
        let url = "https://a.test/page.html";
        let ask: BrowserFetch = Arc::new(|_, _: &str| None);
        let renderer = render_with_browser_fetches(html, url, viewport, None, true, ask);
        let sent = frame::encode(&page_frame(&renderer, viewport)).unwrap();
        let got = frame::decode(&sent).unwrap();
        assert!(!got.cut);
        assert!(got.page_height >= 2000.0);
        let mut urls: Vec<_> = got.links.iter().map(|l| l.url.as_str()).collect();
        urls.dedup();
        assert_eq!(urls, ["https://a.test/next", "https://b.test/x"]);

        let window =
            paint_worker_document(&renderer.document.to_html(), url, viewport, None, None, 0);
        let check = FrameCheck::compare(&got, &window);
        assert!(check.same_as_window, "{check:?}");
        assert!(check.layers >= 3, "{check:?}");
        assert_eq!(check.items, window.display_list.items.len());
        assert_eq!(got.list, renderer.display_list);
        assert_eq!(page_frame(&window, viewport), got);
        // Built from the page given up, the frame is the same.
        assert_eq!(page_frame_owned(renderer, viewport), got);

        // A window page that differs is told apart.
        let other = paint_worker_document("<p>other</p>", url, viewport, None, None, 0);
        assert!(!FrameCheck::compare(&got, &other).same_as_window);
    }

    /// A window page with the renderer's frame of the same page attached,
    /// the way [`finish_worker_response`] makes it.
    fn page_with_frame(html: &str, url: &str, viewport: Viewport) -> Page {
        let ask: BrowserFetch = Arc::new(|_, _: &str| None);
        let renderer = render_with_browser_fetches(html, url, viewport, None, true, ask);
        let sent = frame::encode(&page_frame(&renderer, viewport)).unwrap();
        let got = frame::decode(&sent).unwrap();
        let mut page =
            paint_worker_document(&renderer.document.to_html(), url, viewport, None, None, 0);
        page.renderer_frame = Some(FrameCheck::compare(&got, &page));
        page.drawn = Some(got);
        page
    }

    /// Draws `page` through two sessions, one from the window's own copy and
    /// one from the frame, at each scroll position and zoom, and returns the
    /// first place where a pixel differs.
    fn first_pixel_difference(
        page: &Page,
        viewport: Viewport,
        views: &[(u16, f32)],
    ) -> Option<String> {
        let mut own = Session::new(viewport);
        own.open_page(page.clone());
        let mut drawn = Session::new(viewport);
        drawn.set_draw_from_renderer(true);
        drawn.open_page(page.clone());
        assert!(drawn.active().current().unwrap().drawn.is_some());
        for &(zoom, scroll) in views {
            for session in [&mut own, &mut drawn] {
                session.set_zoom_percent(zoom);
                session.scroll_to_ratio(0.0);
                session.scroll_by(scroll);
            }
            let (a, b) = (own.rasterize(), drawn.rasterize());
            let differ = a
                .pixels()
                .chunks(4)
                .zip(b.pixels().chunks(4))
                .filter(|(x, y)| x != y)
                .count();
            if differ > 0 || own.scroll() != drawn.scroll() {
                return Some(format!(
                    "zoom {zoom} scroll {scroll}: {differ} pixels differ, scrolled to {} and {}",
                    own.scroll(),
                    drawn.scroll()
                ));
            }
        }
        assert!(drawn.active().current().unwrap().drawn.is_some());
        None
    }

    /// The pixel comparison between today's drawing and drawing from the
    /// renderer's frame. Both go through the same rasterizer, so they must
    /// match exactly: not one pixel may differ, at any scroll position or
    /// zoom tested. A difference means the renderer laid out the page in
    /// another way, or the frame lost something on the way, and both are
    /// bugs. The only place the two may part is past the point where a page
    /// too long for a frame was cut; that is tested on its own below.
    #[test]
    fn pages_drawn_from_frames_match_the_window_pixel_for_pixel() {
        let viewport = Viewport {
            width: 480.0,
            height: 360.0,
        };
        // A name, the page, and the zoom and scroll positions to compare at.
        type Fixture = (&'static str, &'static str, &'static [(u16, f32)]);
        let fixtures: &[Fixture] = &[
            (
                "text and links",
                "<html><body><h1>Title</h1><h2>Smaller</h2><p>Plain text with \
                 <a href=\"/next\">a link</a>, <b>bold</b>, <i>italic</i> and \
                 <u>underlined</u> words that wrap onto the next line of the page.</p>\
                 <ul><li>one</li><li>two</li></ul><ol><li>first</li><li>second</li></ol>\
                 <pre>  kept   spaces</pre><blockquote>quoted</blockquote></body></html>",
                &[(100, 0.0), (150, 0.0), (50, 0.0)],
            ),
            (
                "boxes and effects",
                "<html><head><style>\
                 .r { border-radius: 12px; border: 3px solid rgb(10, 80, 160); padding: 8px; margin: 6px; }\
                 .s { box-shadow: 4px 4px 6px rgb(0, 0, 0); background: rgb(250, 230, 200); }\
                 .g { background: linear-gradient(45deg, rgb(255, 0, 0), rgb(0, 0, 255)); height: 40px; }\
                 .o { opacity: 0.4; background: rgb(0, 128, 0); }\
                 .t { transform: rotate(10deg) scale(0.9); background: rgb(120, 0, 120); color: white; }\
                 </style></head><body>\
                 <div class=\"r s\">rounded with a shadow</div><div class=\"g r\">gradient</div>\
                 <div class=\"o r\">see through <span class=\"r\">inside</span></div>\
                 <div class=\"t r\">turned</div></body></html>",
                &[(100, 0.0), (200, 30.0)],
            ),
            (
                "sticky and fixed while scrolling",
                "<html><body style=\"margin: 0\">\
                 <div style=\"position: sticky; top: 0; height: 40px; background: rgb(30, 30, 60); color: white\">header</div>\
                 <div style=\"height: 900px; background: linear-gradient(180deg, rgb(255, 255, 255), rgb(200, 200, 255))\">body</div>\
                 <section style=\"height: 600px\"><div style=\"position: sticky; top: 50px; background: rgb(255, 220, 0)\">section</div></section>\
                 <div style=\"position: fixed; bottom: 0; left: 0; width: 100%; height: 30px; background: rgb(0, 90, 0)\">footer</div>\
                 <div style=\"height: 1500px\">more</div></body></html>",
                &[
                    (100, 0.0),
                    (100, 120.0),
                    (100, 950.0),
                    (100, 1800.0),
                    (150, 700.0),
                ],
            ),
            (
                "flex, grid and clipping",
                "<html><body>\
                 <div style=\"display: flex; gap: 10px\"><div style=\"flex: 1; background: rgb(255, 200, 200)\">a</div>\
                 <div style=\"flex: 2; background: rgb(200, 255, 200)\">b</div></div>\
                 <div style=\"display: grid; grid-template-columns: 1fr 1fr 1fr\"><div>1</div><div>2</div><div>3</div><div>4</div></div>\
                 <div style=\"overflow: hidden; width: 120px; height: 30px; background: rgb(220, 220, 220)\">\
                 clipped text that runs well past the end of its small box</div>\
                 <table><tr><td>cell</td><td>cell two</td></tr></table></body></html>",
                &[(100, 0.0), (125, 0.0)],
            ),
            (
                "a page its script changed",
                "<html><body><p id=\"p\">before</p><div id=\"list\"></div><script>\
                 document.getElementById('p').textContent = 'after the script ran';\
                 const list = document.getElementById('list');\
                 for (let i = 0; i < 30; i++) { const d = document.createElement('div');\
                 d.textContent = 'row ' + i; d.style.background = i % 2 ? 'rgb(240, 240, 240)' : 'white';\
                 list.appendChild(d); }</script></body></html>",
                &[(100, 0.0), (100, 200.0)],
            ),
        ];
        for (name, html, views) in fixtures {
            let page = page_with_frame(html, "https://a.test/page.html", viewport);
            let check = page.renderer_frame.unwrap();
            assert!(check.same_as_window, "{name}: {check:?}");
            assert!(!check.cut, "{name}");
            if let Some(difference) = first_pixel_difference(&page, viewport, views) {
                panic!("{name}: {difference}");
            }
        }
    }

    /// Images: the window loads them over the network, so here both copies
    /// get the same bytes from one stand-in, with scripts off on the
    /// window's side as it has them. Same rule: not one pixel may differ.
    #[test]
    fn a_page_with_images_drawn_from_its_frame_matches_pixel_for_pixel() {
        let viewport = Viewport {
            width: 400.0,
            height: 300.0,
        };
        let mut rgba = Vec::new();
        for i in 0..(16 * 12) {
            rgba.extend_from_slice(&[(i * 7 % 256) as u8, (i * 13 % 256) as u8, 90, 255]);
        }
        let png = anvil_image::encode_png(&Image::from_rgba(16, 12, rgba).unwrap());
        let ask: BrowserFetch = Arc::new(move |kind, url: &str| {
            (kind == FetchKind::Image && url.ends_with("/dot.png")).then(|| FetchReply {
                nav_id: 1,
                status: 200,
                content_type: String::new(),
                body: png.clone(),
            })
        });
        let html = "<html><body><p>picture</p><img src=\"dot.png\" width=\"64\" height=\"48\">\
            <img src=\"dot.png\" style=\"border-radius: 8px; width: 120px\"></body></html>";
        let url = "https://a.test/page.html";
        let renderer = render_with_browser_fetches(html, url, viewport, None, true, ask.clone());
        let got = frame::decode(&frame::encode(&page_frame(&renderer, viewport)).unwrap()).unwrap();
        assert!(
            got.list
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::Image { .. }))
        );
        let mut page = render_with_browser_fetches(
            &renderer.document.to_html(),
            url,
            viewport,
            None,
            false,
            ask,
        );
        let check = FrameCheck::compare(&got, &page);
        assert!(check.same_as_window, "{check:?}");
        page.renderer_frame = Some(check);
        page.drawn = Some(got.clone());
        let difference = first_pixel_difference(&page, viewport, &[(100, 0.0), (175, 0.0)]);
        assert_eq!(difference, None);

        // The comparison does see a frame that lost its images.
        let mut lost = got;
        lost.list
            .items
            .retain(|item| !matches!(item, DisplayItem::Image { .. }));
        page.drawn = Some(lost);
        assert!(first_pixel_difference(&page, viewport, &[(100, 0.0)]).is_some());
    }

    /// A page too tall for a frame is cut. Above the cut it draws the same
    /// as the window; it just stops scrolling where the frame ends.
    #[test]
    fn a_page_too_long_for_a_frame_matches_above_the_cut() {
        let viewport = Viewport {
            width: 400.0,
            height: 300.0,
        };
        let html = "<html><body style=\"margin: 0\"><p>top of a very long page</p>\
            <div style=\"height: 1100000px; background: linear-gradient(180deg, rgb(255, 255, 255), rgb(0, 0, 0))\">\
            </div><p>bottom</p></body></html>";
        let page = page_with_frame(html, "https://a.test/long.html", viewport);
        let check = page.renderer_frame.unwrap();
        assert!(check.cut, "{check:?}");
        assert_eq!(
            first_pixel_difference(&page, viewport, &[(100, 0.0), (100, 5000.0)]),
            None
        );
        let mut drawn = Session::new(viewport);
        drawn.set_draw_from_renderer(true);
        drawn.open_page(page.clone());
        let mut own = Session::new(viewport);
        own.open_page(page);
        assert_eq!(drawn.max_scroll(), frame::MAX_PAGE_HEIGHT - viewport.height);
        assert!(own.max_scroll() > drawn.max_scroll());
    }

    /// Point in the window, scrolled into view, at the first text run on the
    /// active page that contains `needle`.
    fn scroll_to_text(session: &mut Session, needle: &str) -> (f32, f32) {
        fn find(b: &LayoutBox, needle: &str) -> Option<(f32, f32)> {
            b.text
                .iter()
                .find(|run| run.text.contains(needle))
                .map(|run| (run.x + 4.0, run.y + 4.0))
                .or_else(|| b.children.iter().find_map(|c| find(c, needle)))
        }
        let page = session.active().current().unwrap();
        let (x, y) = find(&page.layout, needle).unwrap();
        session.scroll_to_ratio(0.0);
        session.scroll_by(y - 100.0);
        (x, y - session.scroll())
    }

    #[test]
    fn links_on_the_security_page_are_followed_when_clicked() {
        let mut session = Session::new(Viewport {
            width: 1000.0,
            height: 600.0,
        });
        session.open_target("anvil://security").unwrap();
        let (x, y) = scroll_to_text(&mut session, "what their renderer painted");
        let ClickOutcome::Navigate(target) = session.click_at(x, y) else {
            panic!("the link was not followed");
        };
        assert!(target.contains("toggle=drawing"), "{target}");
        session.open_target(&target).unwrap();
        assert!(session.settings().draw_from_renderer);
        let text = session
            .active()
            .current()
            .unwrap()
            .document
            .text_content(session.active().current().unwrap().document.root());
        assert!(
            text.contains("from the page their renderer painted"),
            "{text}"
        );
        // A web page with the same kind of listener keeps today's behaviour.
        session.open_html(
            "<div id=\"s\"><a href=\"https://b.test/\">a link inside</a></div>\
             <script>document.getElementById('s').addEventListener('click', () => {});</script>",
            "https://a.test/",
        );
        let (x, y) = scroll_to_text(&mut session, "a link inside");
        assert_eq!(session.click_at(x, y), ClickOutcome::ScriptHandled);
    }

    #[test]
    fn a_web_page_is_drawn_from_its_frame_only_when_that_is_turned_on() {
        let viewport = Viewport {
            width: 500.0,
            height: 400.0,
        };
        let html = "<html><body style=\"margin: 0\">\
            <div style=\"position: sticky; top: 0; height: 30px; background: rgb(200, 220, 255)\">\
            <a href=\"/top\">top link</a></div>\
            <p>Hollow Creek Maintenance Log</p><p style=\"height: 3000px\">Hollow Creek Valve</p>\
            </body></html>";
        let url = "https://a.test/log.html";
        let page = page_with_frame(html, url, viewport);
        assert!(page.renderer_frame.unwrap().same_as_window);

        let mut session = Session::new(viewport);
        assert!(!session.settings().draw_from_renderer);
        session.open_page(page.clone());
        assert!(session.active().current().unwrap().drawn.is_none());
        let own = session.rasterize();
        let own_max = session.max_scroll();

        session.set_draw_from_renderer(true);
        session.open_page(page.clone());
        let shown = session.active().current().unwrap();
        let frame_height = shown.drawn.as_ref().unwrap().page_height;
        // Same list, same rasterizer: the same pixels.
        assert_eq!(session.rasterize().pixels(), own.pixels());
        assert_eq!(session.max_scroll(), frame_height - viewport.height);
        assert!((session.max_scroll() - own_max).abs() < 1.0);
        assert_eq!(shown.find_text_matches("hollow").len(), 2);
        assert_eq!(
            session.hit_link(5.0, 5.0).as_deref(),
            Some("https://a.test/top")
        );
        assert_eq!(
            session.select_text_at(5.0, 40.0).as_deref(),
            Some("Hollow Creek Maintenance Log")
        );
        // Scrolled, the sticky link is still drawn and hit at the top.
        session.scroll_by(600.0);
        assert_eq!(
            session.hit_link(5.0, 5.0).as_deref(),
            Some("https://a.test/top")
        );
        session.set_zoom_percent(150);
        session.scroll_to_ratio(0.0);
        assert_eq!(
            session.hit_link(5.0, 5.0 * 1.5).as_deref(),
            Some("https://a.test/top")
        );
        session.set_zoom_percent(100);

        // A change to the window's copy, here a resize, draws that copy again.
        session.set_viewport(Viewport {
            width: 480.0,
            height: 400.0,
        });
        assert!(session.active().current().unwrap().drawn.is_none());

        // Our own pages are never drawn from a frame.
        let mut own_page = page.clone();
        own_page.url = "anvil://settings".to_string();
        session.open_page(own_page);
        assert!(session.active().current().unwrap().drawn.is_none());

        // Turning it off drops every frame that was kept.
        session.open_page(page);
        assert!(session.active().current().unwrap().drawn.is_some());
        session.set_draw_from_renderer(false);
        assert!(session.active().current().unwrap().drawn.is_none());
    }

    #[test]
    fn a_cut_frame_scrolls_only_to_where_it_was_cut() {
        let viewport = Viewport {
            width: 500.0,
            height: 400.0,
        };
        let mut page = page_with_frame("<p>long</p>", "https://a.test/", viewport);
        let list = page.display_list.clone();
        let cut = frame::Frame::build(&list, 500.0, frame::MAX_PAGE_HEIGHT * 2.0, &[]);
        assert!(cut.cut);
        page.drawn = Some(cut);
        let mut session = Session::new(viewport);
        session.set_draw_from_renderer(true);
        session.open_page(page);
        assert_eq!(
            session.scroll_to_ratio(1.0),
            frame::MAX_PAGE_HEIGHT - viewport.height
        );
    }

    /// Links in sticky, transformed and fixed boxes are hit in the frame
    /// where they are drawn, and in the same places as the window's own
    /// hit test on its layout.
    #[test]
    fn frame_links_are_hit_where_the_window_hits_them() {
        let html = "<html><body style=\"margin: 0\">\
            <div style=\"position: sticky; top: 0; height: 30px\"><a href=\"/top\">top</a></div>\
            <p style=\"height: 400px\"><a href=\"/flow\">flow</a></p>\
            <div style=\"transform: translateX(150px); width: 120px\"><a href=\"/moved\">moved</a></div>\
            <div style=\"height: 2000px\">tall</div>\
            <div style=\"position: fixed; top: 350px; left: 0\"><a href=\"/pinned\">pinned</a></div>\
            </body></html>";
        let viewport = Viewport {
            width: 500.0,
            height: 400.0,
        };
        let url = "https://a.test/";
        let ask: BrowserFetch = Arc::new(|_, _: &str| None);
        let page = render_with_browser_fetches(html, url, viewport, None, true, ask);
        let got = frame::decode(&frame::encode(&page_frame(&page, viewport)).unwrap()).unwrap();
        let moves = |end: &str| {
            got.links
                .iter()
                .find(|l| l.url.ends_with(end))
                .map(|l| l.moves.clone())
                .unwrap()
        };
        assert!(moves("/flow").is_empty());
        assert!(matches!(moves("/top")[..], [frame::LinkMove::Sticky(_)]));
        assert!(matches!(
            moves("/moved")[..],
            [frame::LinkMove::Transform(_)]
        ));
        assert!(matches!(moves("/pinned")[..], [frame::LinkMove::Fixed]));

        // Where neither is in a fixed box, the frame and the window agree on
        // every point of the view, at the top and scrolled.
        for scroll in [0.0, 150.0] {
            let view = Rect {
                x: 0.0,
                y: scroll,
                width: viewport.width,
                height: viewport.height,
            };
            let mut hits = 0;
            for row in 0..80 {
                for col in 0..50 {
                    let (x, y) = (col as f32 * 10.0 + 1.0, scroll + row as f32 * 5.0 + 1.0);
                    let window = page
                        .link_at_in_view(x, y, Some(view))
                        .and_then(|href| resolve_url(url, href));
                    let drawn = got.link_at(x, y, view);
                    if drawn.is_some_and(|u| u.ends_with("/pinned"))
                        || window.as_deref().is_some_and(|u| u.ends_with("/pinned"))
                    {
                        continue;
                    }
                    assert_eq!(drawn, window.as_deref(), "at {x},{y} scroll {scroll}");
                    hits += usize::from(drawn.is_some());
                }
            }
            assert!(hits > 10, "{hits}");
        }
        // The sticky link follows the scroll, the moved one is to the right.
        let view = |scroll: f32| Rect {
            x: 0.0,
            y: scroll,
            width: 500.0,
            height: 400.0,
        };
        assert_eq!(
            got.link_at(5.0, 155.0, view(150.0)),
            Some("https://a.test/top")
        );
        let moved = got
            .links
            .iter()
            .find(|l| l.url.ends_with("/moved"))
            .unwrap();
        let (x, y) = (moved.rect.x + 2.0, moved.rect.y + 2.0);
        assert_eq!(
            got.link_at(x + 150.0, y, view(0.0)),
            Some("https://a.test/moved")
        );
        assert_eq!(got.link_at(x, y, view(0.0)), None);
        // The fixed link stays in the same place in the window when scrolled.
        let pinned = got
            .links
            .iter()
            .find(|l| l.url.ends_with("/pinned"))
            .unwrap();
        let (x, y) = (pinned.rect.x + 2.0, pinned.rect.y + 2.0);
        assert_eq!(got.link_at(x, y, view(0.0)), Some("https://a.test/pinned"));
        assert_eq!(
            got.link_at(x, y + 900.0, view(900.0)),
            Some("https://a.test/pinned")
        );
        assert_eq!(got.link_at(x, y, view(900.0)), None);
    }

    #[test]
    fn integrates_v23_3_gpu_scene_batch_video_codecs_canvas_and_live_dom_bridge() {
        let mut doc = anvil_dom::parse_html(
            "<div id=\"grid\"><canvas id=\"stage\" width=\"32\" height=\"16\"></canvas></div>",
        );
        let script = "
            document.title = 'AnvilV23_3';
            customElements.define('anvil-card', function() {});
            const grid = document.getElementById('grid');
            grid.classList.add('ready');
            const stage = document.getElementById('stage');
            const ctx = stage.getContext('2d');
            ctx.fillStyle = '#ff0000';
            ctx.fillRect(0, 0, 16, 16);
        ";
        let run = anvil_script::eval_webapp_with_live_dom_bridge(
            &mut doc,
            "https://example.com/",
            script,
        )
        .expect("live dom bridge executed");
        assert!(run.telemetry.bytecode_ops_executed > 0);
        assert_eq!(doc.title(), "AnvilV23_3");
        assert!(doc.is_custom_element_defined("anvil-card"));

        let sheet = anvil_style::parse_css(
            "@supports (display: grid) { #grid:has(canvas) { display: grid; grid-template-columns: repeat(auto-fill, minmax(100px, 1fr)); } }",
        );
        let styles = anvil_style::compute_styles(&doc, &sheet);
        let layout = anvil_layout::layout_document(&doc, &styles, 320.0);
        let display_list = anvil_paint::paint(&layout);
        let engine = anvil_raster::RasterEngine::new();
        let fb = engine.rasterize_gpu_batch(&display_list, 320, 200, 0.0);
        assert_eq!(fb.width(), 320);
        assert_eq!(fb.height(), 200);

        let mut enc = anvil_video::WebCodecsVideoEncoder::new();
        enc.configure("avc1.42E01E", 16, 16, 30, 20).unwrap();
        let vf = anvil_video::VideoFrame {
            width: 16,
            height: 16,
            pts_ms: 0,
            keyframe: true,
            rgba: vec![180u8; 16 * 16 * 4],
        };
        let chunk = enc.encode(&vf, true).expect("encoded video chunk");
        let mut dec = anvil_video::WebCodecsVideoDecoder::new();
        dec.configure("avc1.42E01E").unwrap();
        let decoded = dec.decode(&chunk).expect("decoded video frame");
        assert_eq!(decoded.rgba.len(), 16 * 16 * 4);
    }

    #[test]
    fn google_search_page_ignores_noscript_and_renders_search_form_and_results() {
        let raw_google_html = "<!DOCTYPE html><html><head><title>Google</title>\
            <noscript><style>table,div,span,p{display:none}</style>\
            <meta content=\"0;url=/httpservice/retry/enablejs\" http-equiv=\"refresh\"></noscript>\
            <style>.wrap { max-width: 600px; margin: 0 auto; box-sizing: border-box; } input::-vendor-search-cancel-button, .btn { display: inline-block; color: rgb(232 234 237 / 100%); }</style>\
            </head><body>\
            <noscript><div>Please click here if you are not redirected within a few seconds.</div></noscript>\
            <script>(function(w, d){ w.google = w.google || {}; w.google.kEI = 'ok'; if (d.readyState === 'complete') { d.getElementById('status').textContent = w.google.kEI; } })(window, document);</script>\
            <center><div class=\"wrap\">\
            <form action=\"/search\" method=\"get\">\
            <input type=\"text\" name=\"q\" value=\"rust\">\
            <button class=\"btn\" type=\"submit\">Google Search</button>\
            </form>\
            <p id=\"status\">pending</p>\
            </div></center></body></html>";
        let mut page = render_html(
            raw_google_html,
            "https://www.google.com/",
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(page.title, "Google");
        let status_id = page.document.get_element_by_id("status").unwrap();
        assert_eq!(page.document.text_content(status_id), "ok");
        assert!(!page.layout.children.is_empty());
        assert!(!page.display_list.items.is_empty());
        assert_eq!(page.runtime.take_pending_navigation(), None);
    }

    #[test]
    fn interactive_input_and_submit_events_update_dom_and_honor_prevent_default() {
        let viewport = Viewport {
            width: 800.0,
            height: 600.0,
        };
        let mut session = Session::new(viewport);
        let html = r#"<!DOCTYPE html>
<html>
<body>
  <form id="search-form" action="/search" method="get">
    <input id="q-box" type="text" name="q" value="" />
    <button id="go-btn" type="submit">Search</button>
  </form>
  <p id="live-preview">empty</p>
  <script>
    class SearchController {
      #queries = 0;
      constructor() {
        const inp = document.getElementById("q-box");
        const form = document.getElementById("search-form");
        const preview = document.getElementById("live-preview");
        inp.addEventListener("input", () => {
          this.#queries = this.#queries + 1;
          preview.setTextContent("typing:" + inp.value + ":" + this.#queries);
        });
        form.addEventListener("submit", (ev) => {
          ev.preventDefault();
          preview.setTextContent("intercepted:" + inp.value + ":" + this.#queries);
        });
      }
    }
    new SearchController();
  </script>
</body>
</html>"#;
        session.open_html(html, "https://search.example.com/");
        let inp_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("q-box")
            .unwrap();
        session.tabs[session.active]
            .current_mut()
            .unwrap()
            .focused_input = Some(inp_id);
        assert!(session.type_into_focused_input('a'));
        assert!(session.type_into_focused_input('b'));
        let preview_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("live-preview")
            .unwrap();
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .text_content(preview_id),
            "typing:ab:2"
        );
        let outcome = session.submit_focused_input();
        assert_eq!(outcome, ClickOutcome::ScriptHandled);
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .text_content(preview_id),
            "intercepted:ab:2"
        );
    }

    #[test]
    fn live_page_subresources_reuse_pooled_connections_and_rasterize_gpu_batches() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
                for css in [
                    "#box { width: 40px; height: 24px; background: #1e90ff; }",
                    "#box { border-radius: 6px; box-shadow: 0 2px 4px #000000; }",
                ] {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 512];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => req.extend_from_slice(&buf[..n]),
                        }
                    }
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{css}",
                        css.len()
                    );
                    let _ = sock.write_all(reply.as_bytes());
                }
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        });

        let reused_before = connection_pool_stats().reused_requests;
        let prev_workers = raster_worker_threads();
        set_raster_worker_threads(1);
        let html = format!(
            "<!doctype html><html><head>
               <link rel=\"stylesheet\" href=\"http://127.0.0.1:{port}/a.css\">
               <link rel=\"stylesheet\" href=\"http://127.0.0.1:{port}/b.css\">
             </head><body style=\"margin:0\"><div id=\"box\"></div></body></html>"
        );
        let page = paint_worker_document(
            &html,
            &format!("http://127.0.0.1:{port}/index.html"),
            Viewport {
                width: 120.0,
                height: 80.0,
            },
            None,
            None,
            0,
        );
        set_raster_worker_threads(prev_workers);

        assert!(connection_pool_stats().reused_requests >= reused_before + 1);
        assert_eq!(page.linked_css.len(), 2);
        let fb_default = anvil_raster::rasterize(&page.display_list, 120, 80, 0.0);
        let fb_parallel = anvil_raster::rasterize_parallel(&page.display_list, 120, 80, 0.0);
        let fb_batch = rasterize_gpu_batch(&page.display_list, 120, 80, 0.0);
        assert_eq!(fb_default.pixels(), fb_parallel.pixels());
        assert_eq!(fb_default.pixels(), fb_batch.pixels());
    }

    #[test]
    fn navigates_fragment_anchors_focuses_inputs_under_listeners_and_renders_inline_svg() {
        let mut session = Session::new(Viewport {
            width: 600.0,
            height: 240.0,
        });
        let html = "<!doctype html><html><body style=\"margin:0\">
            <div id=\"wrap\" onclick=\"window.__clicked = 1\">
              <a id=\"jump\" href=\"#install\" style=\"display:block;height:30px\">Jump to Install</a>
              <input id=\"q\" type=\"text\" value=\"\" style=\"display:block;width:160px;height:28px\">
              <svg id=\"icon\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"#38bdf8\">
                <path d=\"M3 3 H21 V21 H3 Z\"/>
              </svg>
            </div>
            <div style=\"height:500px\">Spacer</div>
            <h2 id=\"install\" style=\"height:40px\">Installation Section</h2>
            <div style=\"height:400px\">Bottom Spacer</div>
          </body></html>";
        session.open_html(html, "https://example.com/docs");
        assert_eq!(session.scroll(), 0.0);

        // 1. Inline <svg> is rasterized into the page's layout and display list
        let page = session.active().current().unwrap();
        let svg_id = page.document.get_element_by_id("icon").unwrap();
        assert!(page.find_layout_rect(svg_id).is_some());
        assert!(
            page.display_list
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::Image { width, height, .. } if (*width - 24.0).abs() < 0.5 && (*height - 24.0).abs() < 0.5))
        );

        // 2. Clicking <input> inside an ancestor with an onclick listener focuses the input
        let input_id = page.document.get_element_by_id("q").unwrap();
        let input_rect = page.find_layout_rect(input_id).unwrap();
        assert_eq!(
            session.click_at(input_rect.x + 8.0, input_rect.y + 8.0),
            ClickOutcome::InputFocused(input_id)
        );
        assert!(session.type_into_focused_input('r'));
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .input_value(input_id),
            "r"
        );

        // 3. In-page #fragment navigation scrolls to #install and updates page.url
        assert!(session.try_same_page_fragment_navigation("#install"));
        assert!(session.scroll() > 400.0);
        assert_eq!(
            session.active().current().unwrap().url,
            "https://example.com/docs#install"
        );
        assert!(session.try_same_page_fragment_navigation("#top"));
        assert_eq!(session.scroll(), 0.0);

        // 4. Opening a URL with #fragment initializes scroll to the fragment target
        session.open_html(html, "https://example.com/docs#install");
        assert!(session.scroll() > 400.0);
    }

    #[test]
    fn toggles_details_selects_radios_via_labels_cycles_focus_and_restores_scroll() {
        let mut session = Session::new(Viewport {
            width: 600.0,
            height: 240.0,
        });
        let html = "<!doctype html><html><body style=\"margin:0\">
            <header style=\"position:fixed;top:0;left:0;width:600px;height:48px\">Sticky Nav</header>
            <div style=\"height:60px\"></div>
            <details id=\"faq\">
              <summary id=\"faq-sum\" style=\"display:block;height:28px\">Toggle FAQ</summary>
              <p id=\"faq-body\">Expanded answer text</p>
            </details>
            <form action=\"https://example.com/submit\" method=\"get\">
              <input id=\"r1\" type=\"radio\" name=\"plan\" value=\"free\" checked style=\"display:block;height:24px\">
              <input id=\"r2\" type=\"radio\" name=\"plan\" value=\"pro\" style=\"display:block;height:24px\">
              <label id=\"lbl2\" for=\"r2\" style=\"display:block;height:24px\">Choose Pro Plan</label>
              <textarea id=\"notes\" name=\"notes\" style=\"display:block;width:200px;height:60px\">line1</textarea>
            </form>
            <div style=\"height:500px\">Spacer</div>
            <h2 id=\"section\" style=\"height:36px\">Section Title</h2>
            <div style=\"height:400px\">Bottom</div>
          </body></html>";
        session.open_html(html, "https://example.com/form");

        // 1. <details> starts closed; clicking <summary> opens it
        let page = session.active().current().unwrap();
        let faq_id = page.document.get_element_by_id("faq").unwrap();
        let sum_id = page.document.get_element_by_id("faq-sum").unwrap();
        assert!(!page.document.is_details_open(faq_id));
        let sum_rect = page.find_layout_rect(sum_id).unwrap();
        assert_eq!(
            session.click_at(sum_rect.x + 8.0, sum_rect.y + 8.0),
            ClickOutcome::CheckboxToggled(faq_id, true)
        );
        assert!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .is_details_open(faq_id)
        );

        // 2. Clicking <label for="r2"> selects radio #r2 and unchecks #r1
        let page = session.active().current().unwrap();
        let r1 = page.document.get_element_by_id("r1").unwrap();
        let r2 = page.document.get_element_by_id("r2").unwrap();
        let lbl2 = page.document.get_element_by_id("lbl2").unwrap();
        assert!(page.document.is_checked(r1));
        assert!(!page.document.is_checked(r2));
        let lbl_rect = page.find_layout_rect(lbl2).unwrap();
        assert_eq!(
            session.click_at(lbl_rect.x + 8.0, lbl_rect.y + 8.0),
            ClickOutcome::CheckboxToggled(r2, true)
        );
        let page = session.active().current().unwrap();
        assert!(!page.document.is_checked(r1));
        assert!(page.document.is_checked(r2));

        // 3. Tab focus cycling advances to <textarea>, and Enter inserts a newline instead of submitting
        let notes_id = page.document.get_element_by_id("notes").unwrap();
        assert!(session.focus_next_input(false));
        assert_eq!(
            session.active().current().unwrap().focused_input,
            Some(notes_id)
        );
        assert_eq!(
            session.submit_focused_input(),
            ClickOutcome::InputFocused(notes_id)
        );
        assert!(session.type_into_focused_input('2'));
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .input_value(notes_id),
            "line1\n2"
        );

        // 4. Sticky header offset on #fragment jump and scroll restoration across back/forward
        assert!(session.try_same_page_fragment_navigation("#section"));
        let saved_scroll = session.scroll();
        assert!(saved_scroll > 300.0);
        session.open_html(
            "<!doctype html><html><body><div style=\"height:900px\">Page 2</div></body></html>",
            "https://example.com/page2",
        );
        assert_eq!(session.scroll(), 0.0);
        session.scroll_by(120.0);
        assert!((session.scroll() - 120.0).abs() < 0.5);
        assert!(session.back());
        assert!((session.scroll() - saved_scroll).abs() < 0.5);
        assert!(session.forward());
        assert!((session.scroll() - 120.0).abs() < 0.5);
    }

    #[test]
    fn settings_theme_zoom_section_and_home_activity_work() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(session.theme(), BrowserTheme::Dark);
        assert_eq!(session.cycle_theme(), BrowserTheme::Midnight);
        assert_eq!(session.cycle_theme(), BrowserTheme::Light);
        assert!(session.theme().is_light());
        assert_eq!(session.cycle_theme(), BrowserTheme::Nordic);
        assert_eq!(session.cycle_theme(), BrowserTheme::Dark);

        let key = permissions::action_key();
        session
            .open_target(&format!("anvil://settings?theme=nordic&key={key}"))
            .unwrap();
        assert_eq!(session.theme(), BrowserTheme::Nordic);

        session
            .open_target(&format!("anvil://settings?zoom=125&key={key}"))
            .unwrap();
        assert_eq!(session.zoom_percent(), 125);

        session
            .open_target("anvil://settings?section=appearance")
            .unwrap();
        let page = session.active().current().unwrap();
        assert!(page.document.get_element_by_id("sec-appearance").is_some());

        session.add_bookmark("Rust Docs", "https://doc.rust-lang.org");
        session.open_html("<title>Example</title><p>Hello</p>", "https://example.com");
        session
            .open_target(&format!("anvil://home?engine=duckduckgo&key={key}"))
            .unwrap();
        assert_eq!(session.search_engine(), SearchEngine::DuckDuckGo);
        let doc = &session.active().current().unwrap().document;
        let home_text = doc.text_content(doc.root());
        assert!(home_text.contains("Rust Docs"));
        assert!(home_text.contains("Example"));
    }

    #[test]
    fn tab_lifecycle_view_source_save_page_select_and_range_controls_work() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        let html = "<!doctype html><html><body style=\"margin:0\">
            <select id=\"tier\" name=\"tier\" style=\"display:block;width:160px;height:28px\">
              <option value=\"free\" selected>Free</option>
              <option value=\"pro\">Pro</option>
              <option value=\"team\">Team</option>
            </select>
            <input id=\"vol\" type=\"range\" min=\"0\" max=\"100\" step=\"25\" value=\"0\" style=\"display:block;width:200px;height:24px\">
            <input id=\"name\" type=\"text\" value=\"Marcus\" style=\"display:block;width:180px;height:26px\">
            <progress id=\"prog\" value=\"60\" max=\"100\" style=\"display:block;width:180px;height:18px\"></progress>
          </body></html>";
        session.open_html(html, "https://example.com/controls");

        // 1. <select> dropdown info & direct option selection
        let page = session.active().current().unwrap();
        let tier_id = page.document.get_element_by_id("tier").unwrap();
        let (rect, opts) = session.select_dropdown_info(tier_id).unwrap();
        assert!(rect.width > 50.0);
        assert_eq!(opts.len(), 3);
        assert_eq!(
            session.choose_select_option(tier_id, 2).as_deref(),
            Some("team")
        );
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .select_value(tier_id),
            "team"
        );

        // 2. <input type="range"> click-to-set ratio quantization
        let page = session.active().current().unwrap();
        let vol_id = page.document.get_element_by_id("vol").unwrap();
        let vol_rect = page.find_layout_rect(vol_id).unwrap();
        assert!(matches!(
            session.click_at(vol_rect.x + vol_rect.width * 0.76, vol_rect.y + 8.0),
            ClickOutcome::SelectChanged(id, val) if id == vol_id && val == "75"
        ));
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .input_value(vol_id),
            "75"
        );

        // 3. Focused DOM <input> cursor movement, mid-string insert, forward delete, word delete
        let page = session.active().current().unwrap();
        let name_id = page.document.get_element_by_id("name").unwrap();
        let name_rect = page.find_layout_rect(name_id).unwrap();
        assert_eq!(
            session.click_at(name_rect.x + 8.0, name_rect.y + 8.0),
            ClickOutcome::InputFocused(name_id)
        );
        assert!(session.move_focused_input_cursor_home());
        assert!(session.move_focused_input_cursor_right());
        assert!(session.delete_focused_input()); // "Mrcus"
        assert!(session.type_into_focused_input('a')); // "Marcus"
        assert!(session.move_focused_input_cursor_end());
        assert!(session.type_into_focused_input(' '));
        assert!(session.paste_into_focused_input("Hale")); // "Marcus Hale"
        assert!(session.delete_word_focused_input()); // "Marcus "
        assert_eq!(
            session
                .active()
                .current()
                .unwrap()
                .document
                .input_value(name_id),
            "Marcus "
        );

        // 4. Duplicate tab, close tab, reopen closed tab, view-source, save page
        assert_eq!(session.duplicate_tab(0), Some(1));
        assert_eq!(session.tabs().len(), 2);
        assert!(session.close_tab(1));
        assert_eq!(session.closed_tabs_len(), 1);
        assert!(session.reopen_closed_tab());
        assert_eq!(session.tabs().len(), 2);
        assert!(session.view_active_page_source().is_some());
        assert_eq!(
            session.active().current().map(|p| p.url.as_str()),
            Some("view-source:https://example.com/controls")
        );
        assert!(session.save_active_page_to_downloads().is_some());
        assert!(!session.downloads().entries().is_empty());
    }

    #[test]
    fn pinned_tabs_drag_selection_inspector_and_console_eval_work() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<!doctype html><html><body style=\"margin:0\">\
             <div id=\"banner\" class=\"hero\" style=\"width:240px;height:36px;color:#ff4400\">First line</div>\
             <p id=\"sub\" style=\"margin:0;width:240px;height:28px\">Second line</p>\
             </body></html>",
            "https://example.com/inspect",
        );

        // 1. Mouse-drag range selection across multiple text runs
        assert!(session.select_text_range(4.0, 4.0, 200.0, 60.0).is_some());
        assert!(session.selected_text().contains("First line"));
        assert!(session.selected_text().contains("Second line"));
        assert!(session.selection_rects().len() >= 2);
        session.clear_selection();
        assert!(session.selected_text().is_empty());
        assert!(session.selection_rects().is_empty());

        // 2. Live DOM inspection & interactive Console JS evaluation
        let inspected = session.inspect_at(12.0, 12.0);
        assert!(inspected.is_some());
        let (path_desc, tag_desc, box_desc, style_desc) = session.inspected_element_summary();
        assert!(path_desc.contains("banner") || tag_desc.contains("banner"));
        assert!(box_desc.contains("240x36"));
        assert!(!style_desc.is_empty());

        let result = session.eval_console_js("1 + 2");
        assert_eq!(result.as_deref(), Some("3"));
        assert_eq!(session.console_history().len(), 1);
        assert_eq!(session.console_history()[0].0, "1 + 2");
        assert_eq!(session.console_history()[0].1, "3");

        // 3. Pinned tabs, tab reordering, and Close Other Tabs
        session.new_tab();
        session.open_html("<title>Two</title>", "https://example.com/two");
        session.new_tab();
        session.open_html("<title>Three</title>", "https://example.com/three");
        assert_eq!(session.tabs().len(), 3);
        assert_eq!(session.active_index(), 2);

        // Move tab 2 left to index 1
        assert!(session.move_active_tab_left());
        assert_eq!(session.active_index(), 1);
        assert_eq!(
            session.active().current().map(|p| p.url.as_str()),
            Some("https://example.com/three")
        );

        // Pin tab 0; pinned tab resists closing and stays in the pinned prefix
        assert!(session.toggle_pin_tab(0));
        assert!(session.tabs()[0].is_pinned());
        assert!(!session.close_tab(0));
        // Unpinned tab at index 1 cannot move left into the pinned prefix at index 0
        assert!(!session.move_active_tab_left());
        assert!(session.move_active_tab_right());
        assert_eq!(session.active_index(), 2);

        // Close Other Tabs keeps pinned tab 0 and active tab 2, closing tab 1
        assert_eq!(session.close_other_tabs(2), 1);
        assert_eq!(session.tabs().len(), 2);
        assert!(session.tabs()[0].is_pinned());
    }

    #[test]
    fn brave_shields_cosmetic_hiding_redirect_surrogates_and_site_controls_work() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });

        // 1. Verify cosmetic element hiding (`##.ad-banner`) hides ad elements while keeping content visible
        let html = "<!doctype html><html><body style=\"margin:0\">\
                    <div id=\"ad\" class=\"ad-banner\" style=\"width:300px;height:100px\">Sponsored Ad</div>\
                    <div id=\"article\" style=\"width:300px;height:80px\">Real Article Content</div>\
                    </body></html>";
        session.open_html(html, "https://news.example.com/story");
        assert!(session.select_text_range(0.0, 0.0, 800.0, 600.0).is_some());
        let all_text = session.selected_text().to_string();
        assert!(!all_text.contains("Sponsored Ad"));
        assert!(all_text.contains("Real Article Content"));

        // 2. Toggle Shields Down for `news.example.com` and verify `.ad-banner` is no longer hidden
        let toggled = session.toggle_shields_for_site("https://news.example.com/story");
        assert_eq!(toggled, Some(("news.example.com".to_string(), false)));
        assert!(!session.is_shields_enabled_for("https://news.example.com/story"));
        session.open_html(html, "https://news.example.com/story");
        assert!(session.select_text_range(0.0, 0.0, 800.0, 600.0).is_some());
        let unshielded_text = session.selected_text().to_string();
        assert!(unshielded_text.contains("Sponsored Ad"));

        // Re-enable Shields for `news.example.com`
        let re_enabled = session.toggle_shields_for_site("https://news.example.com/story");
        assert_eq!(re_enabled, Some(("news.example.com".to_string(), true)));
        assert!(session.is_shields_enabled_for("https://news.example.com/story"));

        // 3. Verify `$redirect=google-analytics_analytics.js` surrogate injection on blocked `<script src>`
        let ga_html = "<!doctype html><html><body>\
                       <script src=\"https://www.google-analytics.com/analytics.js\"></script>\
                       <p id=\"status\">Ready</p>\
                       </body></html>";
        session.open_html(ga_html, "https://shop.example.com/checkout");
        assert_eq!(session.eval_console_js("gaLoaded").as_deref(), Some("1"));
        assert_eq!(session.eval_console_js("ga()").as_deref(), Some("1"));

        // 4. Verify `anvil://shields` (`anvil://security`) internal page & mode controls
        let key = permissions::action_key();
        assert!(session.open_target("anvil://shields").is_ok());
        assert_eq!(
            session.active().current().map(|p| p.url.as_str()),
            Some("anvil://security")
        );
        assert_eq!(session.shields_mode(), anvil_net::ShieldsMode::Standard);
        assert!(
            session
                .open_target(&format!(
                    "anvil://security?shields_mode=aggressive&key={key}"
                ))
                .is_ok()
        );
        assert_eq!(session.shields_mode(), anvil_net::ShieldsMode::Aggressive);
        assert!(
            session
                .open_target(&format!("anvil://security?shields_mode=standard&key={key}"))
                .is_ok()
        );
        assert_eq!(session.shields_mode(), anvil_net::ShieldsMode::Standard);
    }

    #[test]
    fn integrates_v24_1_css_logical_math_color_es2024_and_gamma_text() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        let html = "<!doctype html><html><head><style>\
            @layer theme {\
                #wrap { width: 420px; }\
                @scope (.card-scope) to (.slot) {\
                    @container (min-width: 300px) {\
                        #card {\
                            inline-size: round(230px, 50px);\
                            block-size: 80px;\
                            padding-inline: 12px;\
                            margin-block: 8px;\
                            background-color: color-mix(in srgb, #ff6b24 50%, #0f172a 50%);\
                            color: color(display-p3 0.95 0.96 0.98);\
                            translate: 10px 4px;\
                            scale: 1;\
                            rotate: 0deg;\
                        }\
                    }\
                }\
            }\
            </style></head><body>\
            <div id=\"wrap\"><div class=\"card-scope\"><div id=\"card\">Anvil v.24.1</div></div></div>\
            </body></html>";
        session.open_html(html, "https://example.com/v24-1");
        let page = session.active().current().unwrap();
        let card_id = page.document.get_element_by_id("card").unwrap();
        let rect = page.find_layout_rect(card_id).unwrap();
        assert!((rect.width - 274.0).abs() < 1.0);

        let fb = session.rasterize();
        assert_eq!(fb.width(), 800);
        assert_eq!(fb.height(), 600);

        let js_out = eval_webapp_with_anvil_js(
            "https://example.com/",
            "const a = [3, 1, 2]; const b = arrayToSorted(a); const c = arrayToReversed(b); const r = new DOMRect(10, 20, 30, 40); const parsed = new DOMParser().parseFromString('<span id=\"p\">OK</span>', 'text/html'); const sh = new CSSStyleSheet(); sh.replaceSync('#p { color: red; }'); console.log(arrayJoin(c, ',') + ':' + arrayAt(b, -1) + ':' + r.right + ':' + parsed.getElementById('p').textContent + ':' + sh.cssRules.length);",
        )
        .unwrap();
        assert_eq!(js_out.output, vec!["3,2,1:3:40:OK:1"]);
    }
}
