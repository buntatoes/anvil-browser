//! Flux Browser (Proof of Concept) — Tabs, history, scripting integration, front-end pages, installer, and the page pipeline.

pub mod downloads;
pub mod frontend;
pub mod installer;
pub mod logo;
pub mod profile;
#[cfg(any(windows, target_os = "linux"))]
pub mod shell;
pub mod ui;

/// Release name for this tree.
pub const VERSION: &str = "v.15";

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub use downloads::{DownloadEntry, DownloadManager, DownloadState};
use flux_dom::{Document, NodeId};
pub use flux_image::Image;
use flux_image::decode_auto;
use flux_json::{JsonValue, parse_json, to_json_pretty};
use flux_layout::{LayoutBox, Rect, hit_test, layout_document_interactive};
use flux_net::{
    ContentSecurityPolicy, CookieJar, CspDirective, HstsStore, NetError, allows_corp_subresource,
    append_query, fetch, fetch_post, is_url_blocked_by_default_shield, resolve_url,
    verify_subresource_integrity,
};
pub use flux_net::{NetworkShield, ShieldBlockReason, sri_sha256_token};
use flux_paint::{DisplayItem, DisplayList, paint};
use flux_raster::{Color, Framebuffer, rasterize};
pub use flux_sandbox::{
    MandatorySandboxGuard, SandboxError, SandboxTelemetry, TabWorkerRequest, TabWorkerResponse,
    enforce_mandatory_sandbox_or_abort, lock_current_worker_process_or_abort,
};
use flux_script::ScriptRuntime;
use flux_style::{compute_styles_with_viewport, parse_css};
pub use flux_video::{
    CdmSession, DEFAULT_EME_DEMO_KEY, DEFAULT_EME_DEMO_KID, EmeKeySystem, Mp4Metadata, VideoClip,
    generate_encrypted_demo_mp4, inspect_mp4_metadata,
};
use flux_video::{GpuVideoSession, generate_demo_mp4};
pub use profile::{ProfileError, ProfileSnapshot, default_profile_path};

const PAPER: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchEngine {
    Google,
    DuckDuckGo,
    Bing,
}

impl SearchEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Google => "Google Search",
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Bing => "Microsoft Bing",
        }
    }

    pub fn short_badge(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::DuckDuckGo => "DDG",
            Self::Bing => "Bing",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::DuckDuckGo => "duckduckgo",
            Self::Bing => "bing",
        }
    }

    pub fn home_url(self) -> &'static str {
        match self {
            Self::Google => "https://www.google.com",
            Self::DuckDuckGo => "https://duckduckgo.com",
            Self::Bing => "https://www.bing.com",
        }
    }

    pub fn search_url(self, query: &str) -> String {
        let encoded = frontend::urlencode_component(query.trim());
        match self {
            Self::Google => format!("https://www.google.com/search?q={encoded}"),
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
            Self::Bing => format!("https://www.bing.com/search?q={encoded}"),
        }
    }

    pub fn cycle_next(self) -> Self {
        match self {
            Self::Google => Self::DuckDuckGo,
            Self::DuckDuckGo => Self::Bing,
            Self::Bing => Self::Google,
        }
    }

    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "duckduckgo" | "ddg" => Self::DuckDuckGo,
            "bing" | "microsoft bing" => Self::Bing,
            _ => Self::Google,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserSettings {
    pub gpu_video_acceleration: bool,
    pub https_only_mode: bool,
    pub scripts_enabled: bool,
    pub jit_enabled: bool,
    pub search_engine: SearchEngine,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            gpu_video_acceleration: true,
            https_only_mode: false,
            scripts_enabled: true,
            jit_enabled: true,
            search_engine: SearchEngine::Google,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    None,
    ScriptHandled,
    InputFocused(NodeId),
    CheckboxToggled(NodeId, bool),
    SelectChanged(NodeId, String),
    VideoToggled(NodeId, bool),
    VideoSeeked(NodeId, u32),
    Navigate(String),
    SubmitPost { url: String, body: String },
}

/// Interactive `<video>` playback and timeline scrubbing state for a DOM video element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoPlayerState {
    pub clip: VideoClip,
    pub playing: bool,
    pub current_ms: u32,
    pub duration_ms: u32,
}

impl VideoPlayerState {
    pub fn new(clip: VideoClip) -> Self {
        let duration_ms = clip.duration_ms().max(33);
        Self {
            clip,
            playing: false,
            current_ms: 0,
            duration_ms,
        }
    }

    pub fn seek_to_ms(&mut self, target_ms: u32) {
        self.current_ms = target_ms.min(self.duration_ms);
    }

    pub fn step_by_ms(&mut self, delta_ms: u32) {
        let span = self.duration_ms.max(1);
        self.current_ms = (self.current_ms + delta_ms) % (span + 1);
    }

    pub fn render_surface(&self) -> Option<Image> {
        let frame = self
            .clip
            .frame_at_ms(self.current_ms)
            .or_else(|| self.clip.poster_frame())?;
        let mut img = Image::from_rgba(frame.width, frame.height, frame.rgba.clone()).ok()?;
        overlay_video_controls_hud(&mut img, self.playing, self.current_ms, self.duration_ms);
        Some(img)
    }
}

fn overlay_video_controls_hud(img: &mut Image, playing: bool, current_ms: u32, duration_ms: u32) {
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
    // Scrubber playhead knob
    for py in (mid_y.saturating_sub(4))..=(mid_y + 4).min(img.height - 1) {
        for px in (head_x.saturating_sub(2))..=(head_x + 2).min(img.width - 1) {
            img.set_pixel(px, py, [252, 238, 198, 255]);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub source: String,
    pub external_css: String,
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
}

impl Page {
    pub fn content_height(&self) -> f32 {
        (self.layout.rect.y + self.layout.rect.height).max(0.0)
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        hit_test(&self.layout, x, y)
    }

    /// Find the laid-out bounding rectangle of `target` in page coordinates.
    pub fn find_layout_rect(&self, target: NodeId) -> Option<Rect> {
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

    /// Return the bounding boxes `(x, y, width, height)` of all rendered text runs matching `query`
    /// (case-insensitive) for Find-in-Page (`Ctrl+F`).
    pub fn find_text_matches(&self, query: &str) -> Vec<Rect> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut matches = Vec::new();
        for item in &self.display_list.items {
            if let DisplayItem::Text {
                x,
                y,
                text,
                font_size,
                ..
            } = item
            {
                if text.to_ascii_lowercase().contains(&needle) {
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
        }
        matches
    }

    /// Return the text content of the DOM node at `(x, y)` for text selection / copy.
    pub fn text_at(&self, x: f32, y: f32) -> Option<String> {
        let id = self.hit_test(x, y)?;
        let raw = self.document.text_content(id);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub fn text_input_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut cursor = self.hit_test(x, y);
        while let Some(id) = cursor {
            if self.document.is_text_input(id) {
                return Some(id);
            }
            cursor = self.document.get(id).parent();
        }
        None
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        let mut cursor = self.hit_test(x, y);
        while let Some(id) = cursor {
            let node = self.document.get(id);
            if node.data().tag() == Some("a") {
                if let Some(href) = node.data().attr("href") {
                    if !href.trim().is_empty() {
                        return Some(href);
                    }
                }
            }
            cursor = node.parent();
        }
        None
    }

    pub fn restage(&mut self, viewport: Viewport) {
        let embedded = self.document.embedded_css();
        let combined_css = match (self.external_css.is_empty(), embedded.is_empty()) {
            (true, _) => embedded,
            (false, true) => self.external_css.clone(),
            (false, false) => format!("{}\n{embedded}", self.external_css),
        };
        let sheet = parse_css(&combined_css);
        let styles = compute_styles_with_viewport(&self.document, &sheet, viewport.width);
        self.layout = layout_document_interactive(
            &self.document,
            &styles,
            viewport.width,
            &self.images,
            self.focused_input,
        );
        self.display_list = paint(&self.layout);
        self.title = self.document.title();
    }
}

#[derive(Debug)]
pub enum BrowserError {
    Net(NetError),
    Status(u16),
    Utf8,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(err) => write!(f, "{err}"),
            Self::Status(status) => write!(f, "the server returned {status}"),
            Self::Utf8 => write!(f, "the page was not text"),
        }
    }
}

impl std::error::Error for BrowserError {}

pub fn render_html(html: &str, url: &str, viewport: Viewport) -> Page {
    render_html_with_css(html, "", url, viewport)
}

pub fn render_html_with_css(html: &str, external_css: &str, url: &str, viewport: Viewport) -> Page {
    render_html_with_resources(html, external_css, HashMap::new(), url, viewport)
}

pub fn render_html_with_resources(
    html: &str,
    external_css: &str,
    images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
) -> Page {
    render_html_with_resources_and_csp(html, external_css, images, url, viewport, None, 0)
}

pub fn render_html_with_resources_and_csp(
    html: &str,
    external_css: &str,
    mut images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
    http_csp_header: Option<&str>,
    initial_blocked_csp: usize,
) -> Page {
    let mut document = flux_dom::parse_html(html);
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
    for (src, integrity) in document.external_scripts_with_integrity() {
        if !csp.allows_resource(CspDirective::ScriptSrc, url, &src) {
            blocked_csp_count += 1;
            continue;
        }
        let Some(resolved) = resolve_url(url, &src) else {
            continue;
        };
        if is_url_blocked_by_default_shield(&resolved, Some(url)).is_some() {
            blocked_csp_count += 1;
            continue;
        }
        let raw_bytes = if is_remote_origin(&resolved) {
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
        } else if !is_remote_origin(url) && !url.starts_with("flux://") {
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

    for (img_id, src) in document.images() {
        let trimmed = src.trim();
        if trimmed.eq_ignore_ascii_case("flux://logo.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_flux_logo(64));
        } else if trimmed.eq_ignore_ascii_case("flux://engine-google.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::Google, 36));
        } else if trimmed.eq_ignore_ascii_case("flux://engine-ddg.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::DuckDuckGo, 36));
        } else if trimmed.eq_ignore_ascii_case("flux://engine-bing.png") {
            images
                .entry(img_id)
                .or_insert_with(|| logo::render_search_engine_logo(SearchEngine::Bing, 36));
        }
    }

    let vids = document.videos();
    let mut blocked_tracker_count = 0usize;
    let mut videos = HashMap::new();
    if !vids.is_empty() {
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

    let embedded = if csp.allows_inline_style() {
        document.embedded_css()
    } else {
        if !document.embedded_css().is_empty() {
            blocked_csp_count += 1;
        }
        String::new()
    };
    let combined_css = match (external_css.is_empty(), embedded.is_empty()) {
        (true, _) => embedded,
        (false, true) => external_css.to_string(),
        (false, false) => format!("{external_css}\n{embedded}"),
    };
    let sheet = parse_css(&combined_css);
    let styles = compute_styles_with_viewport(&document, &sheet, viewport.width);
    let layout = layout_document_interactive(&document, &styles, viewport.width, &images, None);
    let display_list = paint(&layout);
    Page {
        url: url.to_string(),
        title: document.title(),
        source: html.to_string(),
        external_css: external_css.to_string(),
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
    }
}

pub fn load_url(url: &str, viewport: Viewport) -> Result<Page, BrowserError> {
    let (final_url, response) = fetch(url).map_err(BrowserError::Net)?;
    if !(200..300).contains(&response.status) {
        return Err(BrowserError::Status(response.status));
    }
    let http_csp = response
        .header("content-security-policy")
        .map(str::to_string);
    let html = String::from_utf8(response.body).map_err(|_| BrowserError::Utf8)?;
    let parsed_doc = flux_dom::parse_html(&html);
    let combined_csp_raw = match (http_csp.as_deref(), parsed_doc.meta_csp()) {
        (Some(h), Some(m)) => format!("{h}; {m}"),
        (Some(h), None) => h.to_string(),
        (None, Some(m)) => m,
        (None, None) => String::new(),
    };
    let csp = ContentSecurityPolicy::parse(&combined_csp_raw);
    let mut blocked = 0usize;
    let external_css = fetch_linked_stylesheets(
        &final_url,
        &parsed_doc.stylesheet_links_with_integrity(),
        &csp,
        &mut blocked,
    );
    let images = fetch_page_images(&final_url, &parsed_doc.images(), &csp, &mut blocked);
    Ok(render_html_with_resources_and_csp(
        &html,
        &external_css,
        images,
        &final_url,
        viewport,
        http_csp.as_deref(),
        blocked,
    ))
}

pub fn load_post_target(url: &str, body: &str, viewport: Viewport) -> Result<Page, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let (final_url, response) =
            fetch_post(url, "application/x-www-form-urlencoded", body.as_bytes())
                .map_err(|err| err.to_string())?;
        if !(200..300).contains(&response.status) {
            return Err(format!("the server returned {}", response.status));
        }
        let http_csp = response
            .header("content-security-policy")
            .map(str::to_string);
        let html =
            String::from_utf8(response.body).map_err(|_| "the page was not text".to_string())?;
        let parsed_doc = flux_dom::parse_html(&html);
        let combined_csp_raw = match (http_csp.as_deref(), parsed_doc.meta_csp()) {
            (Some(h), Some(m)) => format!("{h}; {m}"),
            (Some(h), None) => h.to_string(),
            (None, Some(m)) => m,
            (None, None) => String::new(),
        };
        let csp = ContentSecurityPolicy::parse(&combined_csp_raw);
        let mut blocked = 0usize;
        let external_css = fetch_linked_stylesheets(
            &final_url,
            &parsed_doc.stylesheet_links_with_integrity(),
            &csp,
            &mut blocked,
        );
        let images = fetch_page_images(&final_url, &parsed_doc.images(), &csp, &mut blocked);
        Ok(render_html_with_resources_and_csp(
            &html,
            &external_css,
            images,
            &final_url,
            viewport,
            http_csp.as_deref(),
            blocked,
        ))
    } else {
        load_target(url, viewport)
    }
}

pub fn load_target(target: &str, viewport: Viewport) -> Result<Page, String> {
    if let Some(dest) = frontend::resolve_navigation_query(target) {
        return load_target(&dest, viewport);
    }
    if let Some((canonical_url, html)) = frontend::resolve_builtin_page(target) {
        return Ok(render_html(&html, &canonical_url, viewport));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        load_url(target, viewport).map_err(|err| err.to_string())
    } else {
        let html =
            fs::read_to_string(target).map_err(|err| format!("could not read {target}: {err}"))?;
        let parsed_doc = flux_dom::parse_html(&html);
        let csp = ContentSecurityPolicy::parse(&parsed_doc.meta_csp().unwrap_or_default());
        let mut blocked = 0usize;
        let external_css = fetch_linked_stylesheets(
            target,
            &parsed_doc.stylesheet_links_with_integrity(),
            &csp,
            &mut blocked,
        );
        let images = fetch_page_images(target, &parsed_doc.images(), &csp, &mut blocked);
        Ok(render_html_with_resources_and_csp(
            &html,
            &external_css,
            images,
            target,
            viewport,
            None,
            blocked,
        ))
    }
}

fn is_remote_origin(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

fn fetch_page_images(
    base: &str,
    imgs: &[(NodeId, String)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> HashMap<NodeId, Image> {
    let mut out = HashMap::new();
    let base_is_remote = is_remote_origin(base);
    for (node_id, src) in imgs {
        let trimmed = src.trim();
        if trimmed.eq_ignore_ascii_case("flux://logo.png") {
            out.insert(*node_id, logo::render_flux_logo(64));
            continue;
        } else if trimmed.eq_ignore_ascii_case("flux://engine-google.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::Google, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("flux://engine-ddg.png") {
            out.insert(
                *node_id,
                logo::render_search_engine_logo(SearchEngine::DuckDuckGo, 36),
            );
            continue;
        } else if trimmed.eq_ignore_ascii_case("flux://engine-bing.png") {
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
        } else if !base_is_remote && !base.starts_with("flux://") {
            fs::read(&resolved).ok()
        } else {
            None
        };
        if let Some(bytes) = raw_bytes {
            if let Ok(image) = decode_auto(&bytes) {
                out.insert(*node_id, image);
            }
        }
    }
    out
}

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
        let raw_bytes = if trimmed.eq_ignore_ascii_case("flux://demo.mp4") {
            generate_demo_mp4(240, 120, 6).ok()
        } else if trimmed.eq_ignore_ascii_case("flux://encrypted.mp4") {
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
                if resolved.eq_ignore_ascii_case("flux://demo.mp4") {
                    generate_demo_mp4(240, 120, 6).ok()
                } else if resolved.eq_ignore_ascii_case("flux://encrypted.mp4") {
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
                } else if !base_is_remote && !base.starts_with("flux://") {
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

fn fetch_linked_stylesheets(
    base: &str,
    links: &[(String, Option<String>)],
    csp: &ContentSecurityPolicy,
    blocked_csp: &mut usize,
) -> String {
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
                    if let Ok(css) = String::from_utf8(response.body) {
                        if !css.trim().is_empty() {
                            sheets.push(css);
                        }
                    }
                }
            }
        } else if !base_is_remote && !base.starts_with("flux://") {
            if let Ok(bytes) = fs::read(&resolved) {
                if !verify_subresource_integrity(integrity.as_deref(), &bytes) {
                    *blocked_csp += 1;
                    continue;
                }
                if let Ok(css) = String::from_utf8(bytes) {
                    if !css.trim().is_empty() {
                        sheets.push(css);
                    }
                }
            }
        }
    }
    sheets.join("\n")
}

#[derive(Debug)]
pub struct Tab {
    history: Vec<Page>,
    index: usize,
}

impl Tab {
    fn empty() -> Self {
        Self {
            history: Vec::new(),
            index: 0,
        }
    }

    pub fn current(&self) -> Option<&Page> {
        self.history.get(self.index)
    }

    pub fn current_mut(&mut self) -> Option<&mut Page> {
        self.history.get_mut(self.index)
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty() && self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

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

    fn replace_current(&mut self, page: Page) {
        if let Some(slot) = self.history.get_mut(self.index) {
            *slot = page;
        }
    }

    fn navigate(&mut self, page: Page) {
        if !self.history.is_empty() {
            self.history.truncate(self.index + 1);
        }
        self.history.push(page);
        self.index = self.history.len() - 1;
    }

    fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.index -= 1;
        true
    }

    fn forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.index += 1;
        true
    }
}

#[derive(Debug)]
pub struct Session {
    tabs: Vec<Tab>,
    active: usize,
    viewport: Viewport,
    scroll: f32,
    zoom_percent: u16,
    bookmarks: Vec<Bookmark>,
    history_log: Vec<Bookmark>,
    settings: BrowserSettings,
    gpu_video: GpuVideoSession,
    sandbox_guard: MandatorySandboxGuard,
    cookie_jar: CookieJar,
    network_shield: NetworkShield,
    hsts_store: HstsStore,
    downloads: DownloadManager,
    clipboard: String,
    selected_text: String,
}

impl Session {
    pub fn new(viewport: Viewport) -> Self {
        let sandbox_guard = enforce_mandatory_sandbox_or_abort();
        Self {
            tabs: vec![Tab::empty()],
            active: 0,
            viewport,
            scroll: 0.0,
            zoom_percent: 100,
            bookmarks: Vec::new(),
            history_log: Vec::new(),
            settings: BrowserSettings::default(),
            gpu_video: GpuVideoSession::default(),
            sandbox_guard,
            cookie_jar: CookieJar::new(),
            network_shield: NetworkShield::new(),
            hsts_store: HstsStore::new(),
            downloads: DownloadManager::new(),
            clipboard: String::new(),
            selected_text: String::new(),
        }
    }

    pub fn sandbox_telemetry(&self) -> SandboxTelemetry {
        self.sandbox_guard.telemetry()
    }

    pub fn cookie_jar(&self) -> &CookieJar {
        &self.cookie_jar
    }

    pub fn cookie_jar_mut(&mut self) -> &mut CookieJar {
        &mut self.cookie_jar
    }

    pub fn network_shield(&self) -> &NetworkShield {
        &self.network_shield
    }

    pub fn network_shield_mut(&mut self) -> &mut NetworkShield {
        &mut self.network_shield
    }

    pub fn hsts_store(&self) -> &HstsStore {
        &self.hsts_store
    }

    pub fn hsts_store_mut(&mut self) -> &mut HstsStore {
        &mut self.hsts_store
    }

    pub fn downloads(&self) -> &DownloadManager {
        &self.downloads
    }

    pub fn downloads_mut(&mut self) -> &mut DownloadManager {
        &mut self.downloads
    }

    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    pub fn search_engine(&self) -> SearchEngine {
        self.settings.search_engine
    }

    pub fn set_search_engine(&mut self, engine: SearchEngine) {
        self.settings.search_engine = engine;
    }

    pub fn cycle_search_engine(&mut self) -> SearchEngine {
        self.settings.search_engine = self.settings.search_engine.cycle_next();
        self.settings.search_engine
    }

    pub fn zoom_factor(&self) -> f32 {
        (self.zoom_percent as f32 / 100.0).clamp(0.5, 2.5)
    }

    pub fn set_zoom_percent(&mut self, percent: u16) -> u16 {
        self.zoom_percent = percent.clamp(50, 250);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.zoom_percent
    }

    pub fn zoom_in(&mut self) -> u16 {
        let next = self.zoom_percent.saturating_add(25).min(250);
        self.set_zoom_percent(next)
    }

    pub fn zoom_out(&mut self) -> u16 {
        let next = self.zoom_percent.saturating_sub(25).max(50);
        self.set_zoom_percent(next)
    }

    pub fn reset_zoom(&mut self) -> u16 {
        self.set_zoom_percent(100)
    }

    pub fn clipboard_text(&self) -> &str {
        &self.clipboard
    }

    pub fn set_clipboard_text(&mut self, text: &str) {
        self.clipboard = text.to_string();
    }

    pub fn selected_text(&self) -> &str {
        &self.selected_text
    }

    pub fn select_text_at(&mut self, x: f32, y: f32) -> Option<String> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        let found = page.text_at(x / z, (y + self.scroll) / z)?;
        self.selected_text = found.clone();
        Some(found)
    }

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

    pub fn paste_into_focused_input(&mut self, text: &str) -> bool {
        let clean: String = text.chars().filter(|c| (' '..='~').contains(c)).collect();
        if clean.is_empty() {
            return false;
        }
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(id) = page.focused_input {
                let mut val = page.document.input_value(id);
                val.push_str(&clean);
                page.document.set_input_value(id, &val);
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn settings(&self) -> BrowserSettings {
        self.settings
    }

    pub fn gpu_video(&self) -> &GpuVideoSession {
        &self.gpu_video
    }

    pub fn history_log(&self) -> &[Bookmark] {
        &self.history_log
    }

    pub fn clear_history_log(&mut self) {
        self.history_log.clear();
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    pub fn max_scroll(&self) -> f32 {
        let Some(page) = self.active().current() else {
            return 0.0;
        };
        (page.content_height() * self.zoom_factor() - self.viewport.height).max(0.0)
    }

    pub fn scroll_by(&mut self, delta: f32) -> f32 {
        self.scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        self.scroll
    }

    pub fn scroll_to_ratio(&mut self, ratio: f32) -> f32 {
        let max_s = self.max_scroll();
        self.scroll = (ratio.clamp(0.0, 1.0) * max_s).clamp(0.0, max_s);
        self.scroll
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        if let Some(current) = self.tabs[self.active].current_mut() {
            current.restage(self.viewport);
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    pub fn hit_link(&self, x: f32, y: f32) -> Option<String> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        let href = page.link_at(x / z, (y + self.scroll) / z)?;
        resolve_url(&page.url, href)
    }

    pub fn hit_text_input(&self, x: f32, y: f32) -> Option<NodeId> {
        let z = self.zoom_factor();
        let page = self.active().current()?;
        page.text_input_at(x / z, (y + self.scroll) / z)
    }

    pub fn has_focused_input(&self) -> bool {
        self.active()
            .current()
            .is_some_and(|p| p.focused_input.is_some())
    }

    pub fn blur_page_input(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if page.focused_input.take().is_some() {
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn type_into_focused_input(&mut self, ch: char) -> bool {
        if !(' '..='~').contains(&ch) {
            return false;
        }
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(id) = page.focused_input {
                let mut val = page.document.input_value(id);
                val.push(ch);
                page.document.set_input_value(id, &val);
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn backspace_focused_input(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(id) = page.focused_input {
                let mut val = page.document.input_value(id);
                val.pop();
                page.document.set_input_value(id, &val);
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn submit_focused_input(&mut self) -> ClickOutcome {
        let focused = self
            .tabs
            .get(self.active)
            .and_then(|t| t.current())
            .and_then(|p| p.focused_input);
        let Some(id) = focused else {
            return ClickOutcome::None;
        };
        self.blur_page_input();
        self.submit_form_for(id)
    }

    pub fn submit_form_for(&mut self, control_id: NodeId) -> ClickOutcome {
        let Some(page) = self.tabs[self.active].current() else {
            return ClickOutcome::None;
        };
        let Some(form_id) = page.document.enclosing_form(control_id) else {
            return ClickOutcome::None;
        };
        let (action, method, encoded) = page.document.serialize_form(form_id);
        let target_base = if action.is_empty() {
            page.url.clone()
        } else {
            resolve_url(&page.url, &action).unwrap_or(action)
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

    pub fn click_at(&mut self, x: f32, y: f32) -> ClickOutcome {
        let z = self.zoom_factor();
        let page_x = x / z;
        let page_y = (y + self.scroll) / z;
        let viewport = self.viewport;
        let mut submit_control = None;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(node_id) = page.hit_test(page_x, page_y) {
                if page.runtime.dispatch_click(&mut page.document, node_id) {
                    if let Some(dest) = page.runtime.take_pending_navigation() {
                        return ClickOutcome::Navigate(dest);
                    }
                    page.restage(viewport);
                    self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                    return ClickOutcome::ScriptHandled;
                }
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
                        let in_scrubber = rel_y >= (vrect.height - 26.0).max(0.0) && rel_x >= 32.0;
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
                    if page.document.is_select_control(curr) {
                        page.focused_input = None;
                        let val = page.document.cycle_select_option(curr).unwrap_or_default();
                        page.restage(viewport);
                        return ClickOutcome::SelectChanged(curr, val);
                    }
                    if page.document.is_submit_control(curr) {
                        page.focused_input = None;
                        submit_control = Some(curr);
                        break;
                    }
                    cursor = page.document.get(curr).parent();
                }
            }
            if page.focused_input.take().is_some() {
                page.restage(viewport);
            }
        }
        if let Some(ctrl_id) = submit_control {
            return self.submit_form_for(ctrl_id);
        }
        if let Some(target) = self.hit_link(x, y) {
            return ClickOutcome::Navigate(target);
        }
        ClickOutcome::None
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn is_active_bookmarked(&self) -> bool {
        let Some(page) = self.active().current() else {
            return false;
        };
        !page.url.is_empty() && self.bookmarks.iter().any(|b| b.url == page.url)
    }

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

    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        self.bookmarks.len() != before
    }

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

    pub fn rasterize(&self) -> Framebuffer {
        let width = (self.viewport.width.round() as u32).max(1);
        let height = (self.viewport.height.round() as u32).max(1);
        let Some(page) = self.active().current() else {
            return Framebuffer::new(width, height, PAPER);
        };
        if self.zoom_percent == 100 {
            rasterize(&page.display_list, width, height, self.scroll)
        } else {
            let scaled = page.display_list.scaled(self.zoom_factor());
            rasterize(&scaled, width, height, self.scroll)
        }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active = index;
        self.scroll = 0.0;
        true
    }

    pub fn new_tab(&mut self) {
        self.tabs.push(Tab::empty());
        self.active = self.tabs.len() - 1;
        self.scroll = 0.0;
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return false;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.scroll = 0.0;
        true
    }

    pub fn open_page(&mut self, page: Page) {
        self.record_history(&page);
        self.tabs[self.active].navigate(page);
        self.scroll = 0.0;
    }

    pub fn open_page_in_tab(&mut self, tab_index: usize, page: Page) {
        self.record_history(&page);
        if let Some(tab) = self.tabs.get_mut(tab_index) {
            tab.navigate(page);
            if tab_index == self.active {
                self.scroll = 0.0;
            }
        }
    }

    fn record_history(&mut self, page: &Page) {
        let url = page.url.trim();
        if url.is_empty()
            || url.starts_with("flux://history")
            || url.starts_with("flux://downloads")
            || url.starts_with("flux://inspect")
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

    pub fn open_html(&mut self, html: &str, url: &str) {
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        let page = match self
            .sandbox_guard
            .execute_isolated(&label, || Ok(render_html(html, url, viewport)))
        {
            Ok(p) => p,
            Err(err) => render_sandbox_recovery_page(url, &err.to_string(), viewport),
        };
        self.open_page(page);
    }

    /// Execute an arbitrary tab page workload inside the hardcoded mandatory OS sandbox boundary.
    /// If the workload faults or panics, only the active tab displays a recovery card.
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

    pub fn open_url(&mut self, url: &str) -> Result<(), BrowserError> {
        let upgraded = self.hsts_store.upgrade_url_if_hsts(url.trim());
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        match self
            .sandbox_guard
            .execute_isolated(&label, || Ok(load_url(&upgraded, viewport)))
        {
            Ok(Ok(page)) => {
                self.open_page(page);
                Ok(())
            }
            Ok(Err(err)) => Err(err),
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.open_page(page);
                Ok(())
            }
        }
    }

    pub fn open_target(&mut self, target: &str) -> Result<(), String> {
        let trimmed = target.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower == "flux://home"
            || lower == "flux://"
            || lower == "about:home"
            || lower == "about:blank"
            || lower.starts_with("flux://home?")
        {
            if lower.contains("engine=duckduckgo") || lower.contains("engine=ddg") {
                self.settings.search_engine = SearchEngine::DuckDuckGo;
            } else if lower.contains("engine=bing") {
                self.settings.search_engine = SearchEngine::Bing;
            } else if lower.contains("engine=google") {
                self.settings.search_engine = SearchEngine::Google;
            }
            let html = frontend::home_html_with_engine(self.settings.search_engine);
            let page = render_html(&html, frontend::HOME_URL, self.viewport);
            self.open_page(page);
            return Ok(());
        }
        if lower.starts_with("flux://navigate") {
            if let Some(dest) =
                frontend::resolve_navigation_query_with_engine(trimmed, self.settings.search_engine)
            {
                return self.open_target(&dest);
            }
        }
        if lower == "flux://inspect"
            || lower == "flux://devtools"
            || lower.starts_with("flux://inspect?")
        {
            let html = frontend::inspect_html_with_state(
                self.tabs.len(),
                self.zoom_percent as u32,
                self.gpu_video.backend().label(),
                self.network_shield.blocked_total(),
                self.hsts_store.len(),
                self.downloads.entries().len(),
                self.cookie_jar.len(),
            );
            let page = render_html(&html, frontend::INSPECT_URL, self.viewport);
            self.open_page(page);
            return Ok(());
        }
        if lower == "flux://settings" || lower.starts_with("flux://settings?") {
            if lower.contains("toggle=gpu_video") {
                self.settings.gpu_video_acceleration = !self.settings.gpu_video_acceleration;
                self.gpu_video.set_hardware_acceleration(
                    self.settings.gpu_video_acceleration,
                    self.settings.gpu_video_acceleration,
                );
            } else if lower.contains("toggle=https_only") {
                self.settings.https_only_mode = !self.settings.https_only_mode;
            } else if lower.contains("toggle=scripts") {
                self.settings.scripts_enabled = !self.settings.scripts_enabled;
            } else if lower.contains("toggle=jit") {
                self.settings.jit_enabled = !self.settings.jit_enabled;
            } else if lower.contains("toggle=search_engine") {
                self.settings.search_engine = self.settings.search_engine.cycle_next();
            } else if lower.contains("engine=duckduckgo") {
                self.settings.search_engine = SearchEngine::DuckDuckGo;
            } else if lower.contains("engine=bing") {
                self.settings.search_engine = SearchEngine::Bing;
            } else if lower.contains("engine=google") {
                self.settings.search_engine = SearchEngine::Google;
            } else if lower.contains("action=save_profile") {
                let _ = self.save_default_profile();
            } else if lower.contains("action=clear_profile") {
                self.clear_all_profile_data();
            }
            let html = frontend::settings_html_with_profile_state_and_engine(
                self.gpu_video.backend().label(),
                self.settings.gpu_video_acceleration,
                self.settings.https_only_mode,
                self.settings.scripts_enabled,
                self.settings.jit_enabled,
                self.settings.search_engine,
                self.gpu_video.frames_decoded().max(12),
                self.gpu_video.frames_encoded().max(12),
                self.bookmarks.len(),
                self.history_log.len(),
                self.cookie_jar.len(),
            );
            let mut page = render_html(&html, frontend::SETTINGS_URL, self.viewport);
            page.runtime.set_jit_enabled(self.settings.jit_enabled);
            self.open_page(page);
            return Ok(());
        }
        if lower == "flux://history" || lower.starts_with("flux://history?") {
            if lower.contains("action=clear") {
                self.clear_history_log();
            }
            let html = frontend::history_html_with_entries(&self.history_log);
            let page = render_html(&html, frontend::HISTORY_URL, self.viewport);
            self.open_page(page);
            return Ok(());
        }
        if lower == "flux://downloads" || lower.starts_with("flux://downloads?") {
            if lower.contains("action=clear") {
                self.downloads.clear();
            } else if lower.contains("action=sample_report") {
                let report = format!(
                    "{{\"version\":\"{VERSION}\",\"sandbox\":\"enforced\",\"eme\":\"org.w3.clearkey\",\"cenc\":true}}"
                );
                self.downloads.record_download(
                    "flux://security/diagnostic-report.json",
                    Some("flux-diagnostic-report.json"),
                    "application/json",
                    report.as_bytes(),
                );
            } else if lower.contains("action=sample_mp4") {
                if let Ok(mp4_bytes) = generate_encrypted_demo_mp4(120, 68, 4) {
                    self.downloads.record_download(
                        "flux://encrypted.mp4",
                        Some("flux-cenc-encrypted-demo.mp4"),
                        "video/mp4",
                        &mp4_bytes,
                    );
                }
            }
            let html = frontend::downloads_html_with_entries(self.downloads.entries());
            let page = render_html(&html, frontend::DOWNLOADS_URL, self.viewport);
            self.open_page(page);
            return Ok(());
        }
        let mut upgraded = self.hsts_store.upgrade_url_if_hsts(trimmed);
        if self.settings.https_only_mode
            && upgraded.len() >= 7
            && upgraded[..7].eq_ignore_ascii_case("http://")
        {
            upgraded = format!("https://{}", &upgraded[7..]);
        }
        let viewport = self.viewport;
        let label = format!("tab-{}", self.active);
        match self
            .sandbox_guard
            .execute_isolated(&label, || Ok(load_target(&upgraded, viewport)))
        {
            Ok(Ok(page)) => {
                self.open_page(page);
                Ok(())
            }
            Ok(Err(err)) => Err(err),
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.open_page(page);
                Ok(())
            }
        }
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

    /// Capture the current session state into a [`ProfileSnapshot`].
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
            cookie_jar: self.cookie_jar.clone(),
            hsts_store: self.hsts_store.clone(),
            downloads: self.downloads.clone(),
        }
    }

    /// Apply a restored [`ProfileSnapshot`] into this session.
    pub fn apply_profile_snapshot(&mut self, snapshot: ProfileSnapshot) -> usize {
        self.settings = snapshot.settings;
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
        self.cookie_jar = snapshot.cookie_jar;
        self.hsts_store = snapshot.hsts_store;
        self.downloads = snapshot.downloads;
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        added
    }

    /// Save an encrypted & authenticated `FLXP\x01` profile container to `path`.
    pub fn save_encrypted_profile(&self, path: &str) -> Result<(), String> {
        let snap = self.to_profile_snapshot();
        profile::save_encrypted_profile_to_path(&snap, Path::new(path)).map_err(|e| e.to_string())
    }

    /// Load an encrypted `FLXP\x01` (or JSON) profile from `path` into this session.
    pub fn load_encrypted_profile(&mut self, path: &str) -> Result<usize, String> {
        let snap = profile::load_profile_from_path(Path::new(path)).map_err(|e| e.to_string())?;
        Ok(self.apply_profile_snapshot(snap))
    }

    /// Save the session profile to the default per-user encrypted profile location (`profile.flx`).
    pub fn save_default_profile(&self) -> Result<(), String> {
        let Some(path) = default_profile_path() else {
            return Err("could not resolve default profile path".to_string());
        };
        let snap = self.to_profile_snapshot();
        profile::save_encrypted_profile_to_path(&snap, &path).map_err(|e| e.to_string())
    }

    /// Load the session profile from the default per-user encrypted profile location (`profile.flx`).
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

    /// Clear all bookmarks, browsing history, cookies, and reset settings to defaults.
    pub fn clear_all_profile_data(&mut self) {
        self.bookmarks.clear();
        self.history_log.clear();
        self.cookie_jar = CookieJar::new();
        self.hsts_store = HstsStore::new();
        self.downloads.clear();
        self.settings = BrowserSettings::default();
        self.zoom_percent = 100;
        if let Some(path) = default_profile_path() {
            let _ = fs::remove_file(path);
        }
    }

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
        match self
            .sandbox_guard
            .execute_isolated(&label, || Ok(load_target(&upgraded, viewport)))
        {
            Ok(Ok(page)) => {
                self.tabs[self.active].replace_current(page);
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                Ok(())
            }
            Ok(Err(err)) => Err(err),
            Err(sandbox_err) => {
                let page =
                    render_sandbox_recovery_page(&upgraded, &sandbox_err.to_string(), viewport);
                self.tabs[self.active].replace_current(page);
                self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                Ok(())
            }
        }
    }

    pub fn back(&mut self) -> bool {
        let moved = self.tabs[self.active].back();
        if moved {
            self.scroll = 0.0;
        }
        moved
    }

    pub fn forward(&mut self) -> bool {
        let moved = self.tabs[self.active].forward();
        if moved {
            self.scroll = 0.0;
        }
        moved
    }

    /// Export the current session tabs, active index, and bookmarks as formatted JSON (`flux-json`).
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

    pub fn save_profile(&self, path: &str) -> Result<(), String> {
        fs::write(path, self.export_json()).map_err(|e| e.to_string())
    }

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

fn render_sandbox_recovery_page(url: &str, reason: &str, viewport: Viewport) -> Page {
    let html = format!(
        "<!DOCTYPE html>
<html>
<head>
  <title>Tab Isolated by Flux Sandbox</title>
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
    <p><a href=\"flux://security\">Open Flux Security Center</a> | <a href=\"flux://home\">Return to Start</a></p>
  </div>
</body>
</html>"
    );
    render_html(&html, url, viewport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_paint::DisplayItem;
    use flux_style::Color;

    #[test]
    fn release_is_v15() {
        assert_eq!(VERSION, "v.15");
    }

    #[test]
    fn supports_w3c_eme_cenc_encrypted_video_and_sha256_download_manager() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        // 1. Render a page with CENC-encrypted `<video src="flux://encrypted.mp4">` and W3C EME script check
        session.open_html(
            "<style>html, body, video { margin: 0; padding: 0; }</style>
             <video id=\"drm_vid\" src=\"flux://encrypted.mp4\" width=\"240\" height=\"120\"></video>
             <script>
               let access = navigator.requestMediaKeySystemAccess('org.w3.clearkey');
               if (access.supported) {
                 document.title = 'EME ClearKey Ready';
               }
             </script>",
            "flux://settings",
        );
        let page = session.active().current().unwrap();
        assert_eq!(page.title, "EME ClearKey Ready");
        let vid_id = page.document.get_element_by_id("drm_vid").unwrap();
        assert!(page.videos.contains_key(&vid_id));

        // 2. Test Download Manager (`flux://downloads`) & SHA-256 verification
        session
            .open_target("flux://downloads?action=sample_report")
            .unwrap();
        assert_eq!(session.downloads().len(), 1);
        assert_eq!(
            session.downloads().entries()[0].file_name,
            "flux-diagnostic-report.json"
        );
        assert_eq!(session.downloads().entries()[0].sha256_hex.len(), 64);

        session
            .open_target("flux://downloads?action=sample_mp4")
            .unwrap();
        assert_eq!(session.downloads().len(), 2);

        session
            .open_target("flux://downloads?action=clear")
            .unwrap();
        assert!(session.downloads().is_empty());
    }

    #[test]
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
    fn supports_encrypted_profile_persistence_and_interactive_video_scrubbing() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<style>html, body, video { margin: 0; padding: 0; }</style>
             <video id=\"demo\" src=\"flux://demo.mp4\" width=\"240\" height=\"120\"></video>",
            "flux://settings",
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
        session.add_bookmark("Security", "flux://security");
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
        assert_eq!(s2.bookmarks()[0].url, "flux://security");
    }

    #[test]
    fn integrates_webp_ico_images_and_hardened_mp4_video_metadata() {
        let mut src = Image::new(16, 16);
        src.set_pixel(4, 4, [255, 200, 80, 180]);
        let webp_bytes = flux_image::encode_webp(&src);
        let decoded_webp = decode_auto(&webp_bytes).expect("decode WebP via decode_auto");
        assert_eq!(decoded_webp.width, 16);
        assert_eq!(decoded_webp.height, 16);
        assert_eq!(decoded_webp.pixel(4, 4), Some([255, 200, 80, 180]));

        let ico_bytes = flux_image::encode_ico(&src);
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
    fn enforces_hardcoded_os_sandbox_csp_and_cookie_isolation_in_session() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        let telemetry = session.sandbox_telemetry();
        assert!(telemetry.os_enforced);
        assert_eq!(telemetry.memory_limit_bytes, 256 * 1024 * 1024);
        assert_eq!(telemetry.active_process_limit, 1);
        assert_eq!(telemetry.ui_restrictions_mask, 0xFF);
        assert!(telemetry.kill_on_close);

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
            "Tab Isolated by Flux Sandbox"
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
    fn loads_builtin_flux_home_security_settings_and_history_with_gpu_video() {
        let vp = Viewport {
            width: 960.0,
            height: 720.0,
        };
        let home = load_target("flux://home", vp).expect("load flux://home");
        assert_eq!(home.url, "flux://home");
        assert_eq!(home.title, "Flux Start");

        let sec = load_target("flux://security", vp).expect("load flux://security");
        assert_eq!(sec.url, "flux://security");
        assert_eq!(sec.title, "Flux Security Center");

        let nav = load_target("flux://navigate?q=flux%3A%2F%2Fsecurity", vp)
            .expect("resolve flux://navigate query");
        assert_eq!(nav.url, "flux://security");

        let mut session = Session::new(vp);
        session.open_target("flux://home").unwrap();
        session.open_target("flux://settings").unwrap();
        let settings_page = session.active().current().unwrap();
        assert_eq!(settings_page.url, "flux://settings");
        assert_eq!(settings_page.title, "Flux Settings");
        // Verify the embedded <video src="flux://demo.mp4"> decoded an NV12/BT.709 video frame
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
            .open_target("flux://settings?toggle=gpu_video")
            .unwrap();
        assert!(!session.settings().gpu_video_acceleration);

        session.open_target("flux://history").unwrap();
        assert_eq!(session.history_log().len(), 2);
        session.open_target("flux://history?action=clear").unwrap();
        assert!(session.history_log().is_empty());
    }

    #[test]
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
            *text == "Flux" && *color == Color::parse("#e8c37a").unwrap()
        }));
        assert!(page.display_list.items.iter().any(|item| {
            matches!(item, DisplayItem::Rect { color, .. } if *color == Color::parse("#1c1a17").unwrap())
        }));
    }

    #[test]
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
    fn exports_and_parses_session_tabs_via_flux_json() {
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
}
