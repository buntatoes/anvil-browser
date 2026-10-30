// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Built-in `anvil://` pages.
//!
//! `Session::open_target` in `lib.rs` calls the page builders. `load_target`
//! calls `resolve_builtin_page` and `resolve_navigation_query`. `ui.rs` is used
//! for `normalize_address_input_with_engine`. `anvil-dom::parse_html` parses the
//! shell string in `render_react_shell_document`. `anvil-video::GpuVideoSession`
//! only supplies the default backend label for the inspect and settings snapshots.
//!
//! `frontend/App.tsx`, `frontend/components/*.tsx`, `frontend/styles/browser.css`,
//! and `frontend/scripts/*.js` are stored as text. `compile_tsx_component` copies
//! the `return (` body and replaces `{props.name}`, `={{props.name}}`, `{{SLOT}}`,
//! and `className=` with `class=`. There is no component compiler and no Node build.
//!
//! `frontend/pages/*.html` are plain HTML templates. [`fill_template`] swaps
//! each `{{name}}` for text the caller has already escaped.

use anvil_dom::{Document, parse_html};
use anvil_video::GpuVideoSession;

use crate::downloads::DownloadEntry;
use crate::ui::{UiState, normalize_address_input_with_engine};
use crate::{Bookmark, BrowserTheme, SearchEngine, Session, VERSION};

/// Canonical URL for the start page.
pub const HOME_URL: &str = "anvil://home";
/// Canonical URL for the inspector page.
pub const INSPECT_URL: &str = "anvil://inspect";
/// Canonical URL for the security page.
pub const SECURITY_URL: &str = "anvil://security";
/// Canonical URL for the settings page.
pub const SETTINGS_URL: &str = "anvil://settings";
/// Canonical URL for the history page.
pub const HISTORY_URL: &str = "anvil://history";
/// Canonical URL for the downloads page.
pub const DOWNLOADS_URL: &str = "anvil://downloads";
/// Canonical URL for the per-site permissions page.
pub const SITE_PERMISSIONS_URL: &str = "anvil://settings/permissions";
/// Prefix for the in-page navigation form. The query key is `q`.
pub const NAVIGATE_URL_PREFIX: &str = "anvil://navigate";

/// Text of `frontend/pages/SitePermissions.html`.
pub const SITE_PERMISSIONS_HTML: &str =
    include_str!("../../../frontend/pages/SitePermissions.html");

/// Text of `frontend/pages/SitePermissionRow.html`, one per site.
pub const SITE_PERMISSION_ROW_HTML: &str =
    include_str!("../../../frontend/pages/SitePermissionRow.html");

/// Text of `frontend/pages/HttpsOnly.html`.
pub const HTTPS_ONLY_HTML: &str = include_str!("../../../frontend/pages/HttpsOnly.html");

/// Text of `frontend/App.tsx`. The shell page fills its `return (` body.
pub const APP_TSX: &str = include_str!("../../../frontend/App.tsx");

/// Text of `frontend/components/StartDashboard.tsx`.
pub const START_DASHBOARD_TSX: &str =
    include_str!("../../../frontend/components/StartDashboard.tsx");

/// Text of `frontend/components/InspectorPage.tsx`.
pub const INSPECTOR_PAGE_TSX: &str = include_str!("../../../frontend/components/InspectorPage.tsx");

/// Text of `frontend/components/SecurityCenter.tsx`.
pub const SECURITY_CENTER_TSX: &str =
    include_str!("../../../frontend/components/SecurityCenter.tsx");

/// Text of `frontend/components/SettingsPage.tsx`.
pub const SETTINGS_PAGE_TSX: &str = include_str!("../../../frontend/components/SettingsPage.tsx");

/// Text of `frontend/components/HistoryPage.tsx`.
pub const HISTORY_PAGE_TSX: &str = include_str!("../../../frontend/components/HistoryPage.tsx");

/// Text of `frontend/components/DownloadsPage.tsx`.
pub const DOWNLOADS_PAGE_TSX: &str = include_str!("../../../frontend/components/DownloadsPage.tsx");

/// Embedded stylesheet (`frontend/styles/browser.css`).
pub const BROWSER_CSS: &str = include_str!("../../../frontend/styles/browser.css");

/// Embedded internal pages stylesheet (`frontend/styles/pages.css`).
pub const PAGES_CSS: &str = include_str!("../../../frontend/styles/pages.css");

/// Embedded developer inspector stylesheet (`frontend/styles/inspector.css`).
pub const INSPECTOR_CSS: &str = include_str!("../../../frontend/styles/inspector.css");

/// Shared front-end controller utilities (`frontend/scripts/anvil_core.js`).
pub const ANVIL_CORE_JS: &str = include_str!("../../../frontend/scripts/anvil_core.js");

/// Text of `frontend/scripts/home.js`, inserted as a script on the start page.
pub const HOME_JS: &str = include_str!("../../../frontend/scripts/home.js");

/// Text of `frontend/scripts/settings.js`, inserted as a script on the settings page.
pub const SETTINGS_JS: &str = include_str!("../../../frontend/scripts/settings.js");

/// Text of `frontend/scripts/inspect.js`, inserted as a script on the inspector page.
pub const INSPECT_JS: &str = include_str!("../../../frontend/scripts/inspect.js");

/// Text of `frontend/scripts/security.js`, inserted as a script on the security page.
pub const SECURITY_JS: &str = include_str!("../../../frontend/scripts/security.js");

/// Text of `frontend/scripts/downloads.js`, inserted as a script on the downloads page.
pub const DOWNLOADS_JS: &str = include_str!("../../../frontend/scripts/downloads.js");

/// Text of `frontend/scripts/history.js`, inserted as a script on the history page.
pub const HISTORY_JS: &str = include_str!("../../../frontend/scripts/history.js");

/// Slice between the first `return (` and the last `);`.
///
/// If `return (` is missing, the whole source is returned. This is a string slice, not a parse.
pub fn extract_tsx_return_body(tsx_source: &str) -> &str {
    let Some(ret_idx) = tsx_source.find("return (") else {
        return tsx_source;
    };
    let after_ret = &tsx_source[ret_idx + "return (".len()..];
    let Some(end_idx) = after_ret.rfind(");") else {
        return after_ret.trim();
    };
    after_ret[..end_idx].trim()
}

/// Replace prop and slot tokens in the `return (` body, then turn `className=` into `class=`.
///
/// `props` replaces `{props.NAME}` and `={{props.NAME}}`. `slots` replaces `{{NAME}}`.
/// No other syntax is interpreted.
pub fn compile_tsx_component(
    tsx_source: &str,
    props: &[(&str, &str)],
    slots: &[(&str, &str)],
) -> String {
    let mut markup = extract_tsx_return_body(tsx_source).to_string();

    for (prop_name, prop_val) in props {
        let quoted_prop = format!("{{props.{prop_name}}}");
        let attr_expr = format!("={{props.{prop_name}}}");
        let quoted_attr_replacement = format!("=\"{prop_val}\"");
        markup = markup.replace(&attr_expr, &quoted_attr_replacement);
        markup = markup.replace(&quoted_prop, prop_val);
    }

    for (slot_name, slot_html) in slots {
        let token = format!("{{{{{slot_name}}}}}");
        markup = markup.replace(&token, slot_html);
    }

    markup.replace("className=", "class=")
}

/// Escape plain text for safe insertion into HTML/JSX text or attribute contexts.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Decode URL-encoded form query strings (e.g. `q=https%3A%2F%2Fexample.com`).
pub fn urldecode_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encode a search query component for URL query strings (`application/x-www-form-urlencoded`).
pub fn urlencode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Resolve `anvil://navigate?q=...` with `SearchEngine::Google`.
///
/// `Session::open_target` does not use this. It calls `resolve_navigation_query_with_engine`
/// with the engine stored in settings. `load_target` does call this function.
pub fn resolve_navigation_query(target: &str) -> Option<String> {
    resolve_navigation_query_with_engine(target, SearchEngine::Google)
}

/// Resolve `anvil://navigate?q=...` for `engine`.
///
/// An empty `q` returns that engine's home URL. Other text goes through
/// `normalize_address_input_with_engine`. Google Search, DuckDuckGo, and
/// Microsoft Bing are the three `SearchEngine` values.
pub fn resolve_navigation_query_with_engine(target: &str, engine: SearchEngine) -> Option<String> {
    let trimmed = target.trim();
    let rest = trimmed.strip_prefix(NAVIGATE_URL_PREFIX)?;
    let query = rest.strip_prefix('?').unwrap_or("");
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k == "q" {
            let decoded = urldecode_component(v);
            if decoded.trim().is_empty() {
                return Some(engine.home_url().to_string());
            }
            return normalize_address_input_with_engine(&decoded, engine)
                .or_else(|| Some(HOME_URL.to_string()));
        }
    }
    Some(engine.home_url().to_string())
}

/// Add `key` to every link in `html` that changes something through
/// `anvil://home?`, `settings?`, `history?`, or `downloads?`.
///
/// `Session::open_target` ignores those changes without the key, so only
/// links on the built-in pages themselves work. `settings/permissions` links
/// already carry the key and are left alone.
pub fn with_action_key(html: &str, key: &str) -> String {
    let mut out = html.to_string();
    for page in ["home", "settings", "history", "downloads", "security"] {
        let from = format!("href=\"anvil://{page}?");
        let to = format!("href=\"anvil://{page}?key={key}&");
        out = out.replace(&from, &to);
    }
    out
}

/// Return `(canonical_url, html_source)` if `target` names a built-in `anvil://` page.
pub fn resolve_builtin_page(target: &str) -> Option<(String, String)> {
    let trimmed = target.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "anvil://home" | "anvil://" | "about:home" | "about:blank" => {
            Some((HOME_URL.to_string(), home_html()))
        }
        s if s.starts_with("anvil://home?") => {
            let engine = if s.contains("engine=duckduckgo") || s.contains("engine=ddg") {
                SearchEngine::DuckDuckGo
            } else if s.contains("engine=bing") {
                SearchEngine::Bing
            } else {
                SearchEngine::Google
            };
            Some((HOME_URL.to_string(), home_html_with_engine(engine)))
        }
        s if s == "anvil://inspect"
            || s == "anvil://devtools"
            || s.starts_with("anvil://inspect?") =>
        {
            Some((INSPECT_URL.to_string(), inspect_html()))
        }
        "anvil://security" | "anvil://shields" => Some((SECURITY_URL.to_string(), security_html())),
        s if s == "anvil://settings" || s.starts_with("anvil://settings?") => {
            Some((SETTINGS_URL.to_string(), settings_html()))
        }
        s if s == "anvil://history" || s.starts_with("anvil://history?") => {
            Some((HISTORY_URL.to_string(), history_html()))
        }
        s if s == "anvil://downloads" || s.starts_with("anvil://downloads?") => {
            Some((DOWNLOADS_URL.to_string(), downloads_html()))
        }
        _ => None,
    }
}

/// CSS rules appended for non-default themes on built-in `anvil://` pages.
pub fn theme_css_override(theme: BrowserTheme) -> &'static str {
    match theme {
        BrowserTheme::Dark => "",
        BrowserTheme::Midnight => {
            r#"
html, body, .anvil-shell, .home-blank {
  background-color: #06080c;
  color: #f8fafc;
}
.settings-card, .section, .card, .history-item, .home-activity-card, .brand-bar, .hero, .banner {
  background: linear-gradient(180deg, #0c1018, #080b11);
  border-color: #1c2434;
}
.setting-row, .engine-card, .sidebar-item, .search-row, .home-pill, td {
  background-color: #090c13;
  border-color: #1a2232;
}
.settings-sidebar {
  background: linear-gradient(180deg, #0b0e16, #07090f);
  border-color: #1c2434;
}
input {
  background-color: #040609;
  border-color: #1c2434;
}
"#
        }
        BrowserTheme::Light => {
            r#"
html, body, .anvil-shell, .home-blank {
  background-color: #f8fafc;
  color: #0f172a;
}
h1, .home-title, .setting-label, .engine-name {
  color: #0f172a;
}
.subtitle, .sub, .card-body, .sec-desc, .brand-nav, .engine-url, .sidebar-heading {
  color: #475569;
}
.settings-card, .section, .card, .history-item, .home-activity-card, .brand-bar, .hero, .banner {
  background: linear-gradient(180deg, #ffffff, #f1f5f9);
  border-color: #cbd5e1;
  box-shadow: 0px 4px 12px rgba(15, 23, 42, 18);
}
.setting-row, .engine-card, .sidebar-item, .search-row {
  background-color: #ffffff;
  border-color: #cbd5e1;
}
.engine-card-active, .sidebar-item-active, .home-pill-active {
  background-color: #fff7ed;
  border-color: #ea580c;
}
.sidebar-item a {
  color: #334155;
}
.sidebar-item-active a, .home-pill-active {
  color: #c2410c;
}
.home-pill {
  background-color: #ffffff;
  color: #475569;
  border-color: #cbd5e1;
}
.settings-sidebar {
  background: linear-gradient(180deg, #ffffff, #f1f5f9);
  border-color: #cbd5e1;
}
.setting-action-pill {
  background-color: #e0f2fe;
  color: #0369a1;
  border-color: #bae6fd;
}
input {
  background-color: #ffffff;
  color: #0f172a;
  border-color: #94a3b8;
}
th {
  background-color: #f1f5f9;
  color: #c2410c;
  border-color: #cbd5e1;
}
td {
  background-color: #ffffff;
  color: #1e293b;
  border-color: #e2e8f0;
}
a, .home-apps a {
  color: #2563eb;
}
.card-title, .sec-title, .brand-title, .badge {
  color: #ea580c;
}
"#
        }
        BrowserTheme::Nordic => {
            r#"
html, body, .anvil-shell, .home-blank {
  background-color: #2e3440;
  color: #eceff4;
}
h1, .home-title, .setting-label, .engine-name {
  color: #eceff4;
}
.subtitle, .sub, .card-body, .sec-desc, .brand-nav, .engine-url {
  color: #d8dee9;
}
.settings-card, .section, .card, .history-item, .home-activity-card, .brand-bar, .hero, .banner {
  background: linear-gradient(180deg, #3b4252, #2e3440);
  border-color: #4c566a;
}
.setting-row, .engine-card, .sidebar-item, .search-row, .home-pill, td {
  background-color: #3b4252;
  border-color: #4c566a;
  color: #e5e9f0;
}
.settings-sidebar {
  background: linear-gradient(180deg, #3b4252, #2e3440);
  border-color: #4c566a;
}
.engine-card-active, .sidebar-item-active, .home-pill-active {
  background-color: #434c5e;
  border-color: #88c0d0;
}
.sidebar-item-active a, .home-pill-active, .card-title, .sec-title, .brand-title, .badge {
  color: #88c0d0;
}
input {
  background-color: #242933;
  color: #eceff4;
  border-color: #4c566a;
}
button {
  background: linear-gradient(180deg, #5e81ac, #4c6a92);
  border-color: #81a1c1;
}
a, .home-apps a {
  color: #88c0d0;
}
"#
        }
    }
}

/// Inject the active theme CSS override into a built-in `anvil://` HTML document.
pub fn with_theme_css(html: &str, theme: BrowserTheme) -> String {
    let extra = theme_css_override(theme);
    if extra.is_empty() {
        return html.to_string();
    }
    if let Some(idx) = html.find("</style>") {
        let mut out = String::with_capacity(html.len() + extra.len() + 2);
        out.push_str(&html[..idx]);
        out.push('\n');
        out.push_str(extra);
        out.push_str(&html[idx..]);
        out
    } else {
        format!("<style>{extra}</style>\n{html}")
    }
}

/// Start page HTML for Google Search. Calls `home_html_with_engine`.
pub fn home_html() -> String {
    home_html_with_engine(SearchEngine::Google)
}

/// Start page HTML for `engine`, including `browser.css`, `anvil_core.js`, and `home.js`.
///
/// The engine mark is a `anvil://engine-*.png` URL. `lib.rs` draws that URL later.
pub fn home_html_with_engine(engine: SearchEngine) -> String {
    home_html_with_state(engine, BrowserTheme::Dark, &[], &[])
}

/// Start page HTML with the active search engine, theme, bookmarks, and recent history.
pub fn home_html_with_state(
    engine: SearchEngine,
    theme: BrowserTheme,
    bookmarks: &[Bookmark],
    history: &[Bookmark],
) -> String {
    let placeholder = format!("Search {} or enter web address...", engine.label());
    let engine_logo = match engine {
        SearchEngine::Google => "anvil://engine-google.png",
        SearchEngine::DuckDuckGo => "anvil://engine-ddg.png",
        SearchEngine::Bing => "anvil://engine-bing.png",
    };
    let google_pill_cls = if engine == SearchEngine::Google {
        "home-pill-active"
    } else {
        "home-pill"
    };
    let ddg_pill_cls = if engine == SearchEngine::DuckDuckGo {
        "home-pill-active"
    } else {
        "home-pill"
    };
    let bing_pill_cls = if engine == SearchEngine::Bing {
        "home-pill-active"
    } else {
        "home-pill"
    };

    let recent_history: Vec<&Bookmark> = history
        .iter()
        .rev()
        .filter(|h| !h.url.starts_with("anvil://home") && !h.url.contains("key="))
        .take(4)
        .collect();

    let activity_html = if bookmarks.is_empty() && recent_history.is_empty() {
        String::new()
    } else {
        let mut s = String::from(r#"<div class="home-activity">"#);
        if !bookmarks.is_empty() {
            s.push_str(
                r#"<div class="home-activity-card"><p class="sec-title">Saved Bookmarks</p>"#,
            );
            for bm in bookmarks.iter().take(4) {
                let title = escape_html(&bm.title);
                let url = escape_html(&bm.url);
                s.push_str(&format!(
                    r#"<p class="sec-desc"><a href="{url}">{title}</a></p>"#
                ));
            }
            s.push_str("</div>");
        }
        if !recent_history.is_empty() {
            s.push_str(r#"<div class="home-activity-card"><p class="sec-title">Recent Visits</p>"#);
            for item in recent_history {
                let title = escape_html(&item.title);
                let url = escape_html(&item.url);
                s.push_str(&format!(
                    r#"<p class="sec-desc"><a href="{url}">{title}</a></p>"#
                ));
            }
            s.push_str("</div>");
        }
        s.push_str("</div>");
        s
    };

    let body = compile_tsx_component(
        START_DASHBOARD_TSX,
        &[
            ("version", VERSION),
            ("defaultQuery", ""),
            ("activeEngineName", engine.label()),
            ("activeEngineLogo", engine_logo),
            ("searchPlaceholder", &placeholder),
            ("googlePillClass", google_pill_cls),
            ("ddgPillClass", ddg_pill_cls),
            ("bingPillClass", bing_pill_cls),
        ],
        &[("HOME_ACTIVITY_SLOT", &activity_html)],
    );
    let html = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Start</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{HOME_JS}\n  </script>\n</body>\n</html>"
    );
    with_theme_css(&html, theme)
}

/// Inspector page HTML filled with the counts passed in, plus `anvil_core.js` and `inspect.js`.
#[allow(clippy::too_many_arguments)]
pub fn inspect_html_with_state(
    tab_count: usize,
    zoom_percent: u32,
    gpu_backend: &str,
    shield_blocked: u64,
    hsts_count: usize,
    download_count: usize,
    cookie_count: usize,
) -> String {
    let tabs_str = tab_count.to_string();
    let zoom_str = format!("{zoom_percent}%");
    let shield_str = shield_blocked.to_string();
    let hsts_str = hsts_count.to_string();
    let dl_str = download_count.to_string();
    let ck_str = cookie_count.to_string();
    let body = compile_tsx_component(
        INSPECTOR_PAGE_TSX,
        &[
            ("version", VERSION),
            ("tabCount", &tabs_str),
            ("activeZoom", &zoom_str),
            ("gpuBackend", gpu_backend),
            ("shieldBlocked", &shield_str),
            ("hstsCount", &hsts_str),
            ("downloadCount", &dl_str),
            ("cookieCount", &ck_str),
        ],
        &[],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Developer Inspector</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{INSPECT_JS}\n  </script>\n</body>\n</html>"
    )
}

/// Inspector HTML with placeholder counts and the default `GpuVideoSession` backend label.
pub fn inspect_html() -> String {
    let gpu = GpuVideoSession::default();
    inspect_html_with_state(1, 100, gpu.backend().label(), 0, 5, 0, 0)
}

/// Security page HTML with `anvil_core.js` and `security.js`.
pub fn security_html() -> String {
    security_html_with_key_exchanges(&anvil_net::recent_key_exchanges())
}

/// The key exchange part of anvil://security: what is offered, then the
/// group each recent handshake actually used. A connection is only called
/// post-quantum when the negotiated group includes ML-KEM.
pub fn key_exchange_section_html(recent: &[anvil_net::KeyExchange]) -> String {
    let mut html = String::new();
    if cfg!(target_os = "linux") {
        html.push_str(
            r#"<p class="sec-desc">HTTPS asks OpenSSL to offer the post-quantum hybrid X25519MLKEM768 first, then X25519, P-256, and P-384. The server picks one. Older OpenSSL versions without the hybrid fall back to the classical groups. The table lists the group each recent connection from this window really used.</p>"#,
        );
    } else {
        html.push_str(
            r#"<p class="sec-desc">On Windows, Schannel negotiates the TLS connection and ALPN (h2 and http/1.1), while anvil-pqc and anvil-cipher verify the in-house X25519MLKEM768 hybrid key exchange and TLS 1.3 key schedule. The table lists the key exchange each recent connection from this window really used.</p>"#,
        );
    }
    if recent.is_empty() {
        html.push_str(r#"<p class="sec-desc">No HTTPS connections from this window yet.</p>"#);
        return html;
    }
    html.push_str(
        "<table id=\"key-exchange-table\"><tr><th>Site</th><th>Key exchange</th><th>Post-quantum</th></tr>",
    );
    for entry in recent {
        let verdict = if entry.is_post_quantum() {
            "Yes, hybrid with ML-KEM"
        } else {
            "No, classical only"
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{verdict}</td></tr>",
            escape_html(&entry.host),
            escape_html(&entry.group)
        ));
    }
    html.push_str("</table>");
    html
}

/// What the confinement part of anvil://security says when the page is
/// built without a window, so there is no renderer to ask.
pub const CONFINEMENT_WITHOUT_WINDOW: &str = r#"<p class="sec-desc">Open this page from a browser window to see how its renderers are confined.</p>"#;

/// One running tab renderer, for anvil://security: the tab id, the
/// renderer's process id, and the tool it was started through, if any.
pub type RunningRenderer = (u64, u32, Option<anvil_sandbox::confine::ToolKind>);

/// The renderer confinement part of anvil://security.
///
/// `rows` come from `anvil_sandbox::confine::protections`, so every line
/// about the renderer is what a fresh renderer read about itself. All
/// text is escaped.
pub fn confinement_section_html(
    rows: &[anvil_sandbox::confine::Protection],
    mode: crate::ConfineMode,
    running: &[RunningRenderer],
) -> String {
    use anvil_sandbox::confine::State;
    let mut html = String::new();
    html.push_str(
        r#"<p class="sec-desc">On Linux each tab's renderer is started through bwrap, or unshare when bwrap is missing or fails, if the system has one and it passes a self-test when the window starts. Every line below about the renderer is read from its own /proc/self by a renderer started for this page, the same way tab renderers are started, before it has seen any page. A protection is only marked active when that reading shows it.</p>"#,
    );
    let (now, action) = match mode {
        crate::ConfineMode::Auto => ("automatic", "Turn confinement off"),
        crate::ConfineMode::Off => ("off", "Turn confinement on"),
    };
    html.push_str(&format!(
        r#"<p class="sec-desc" id="confinement-mode">Confinement: {now}. A change applies to renderers started after it.</p><p class="sec-desc"><a href="anvil://security?toggle=confinement">{action}</a></p>"#
    ));
    html.push_str(
        r#"<table id="confinement-table"><tr><th>Protection</th><th>State</th><th>What the OS says</th></tr>"#,
    );
    for row in rows {
        let state = match row.state {
            State::Active => "Active",
            State::Limited => "Limited",
            State::NotActive => "Not active",
            State::Unknown => "Unknown",
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{state}</td><td>{}</td></tr>",
            escape_html(row.name),
            escape_html(&row.detail)
        ));
    }
    html.push_str("</table>");
    if running.is_empty() {
        html.push_str(r#"<p class="sec-desc">No tab renderers are running.</p>"#);
    } else {
        html.push_str(
            r#"<p class="sec-desc">Tab renderers running now:</p><ul id="confinement-running">"#,
        );
        for (tab, pid, tool) in running {
            let how = match tool {
                Some(kind) => format!("started through {}", kind.name()),
                None => "started without a confinement tool".to_string(),
            };
            html.push_str(&format!(
                "<li>tab {tab}: process {pid}, {}</li>",
                escape_html(&how)
            ));
        }
        html.push_str("</ul>");
    }
    html
}

/// The part of `anvil://security` that says what web pages are drawn
/// from, with a link to change it. `on` is whether they are drawn from the
/// page their renderer painted.
pub fn drawing_section_html(on: bool) -> String {
    let (now, action) = if on {
        (
            "from the page their renderer painted, after this window checks it. Clicks, typing and animations still run on the window's own copy of the page, and once one changes it, the window draws its own copy again",
            "Draw web pages from the window's own copy",
        )
    } else {
        (
            "from the window's own copy of the page",
            "Draw web pages from what their renderer painted",
        )
    };
    format!(
        r#"<p class="sec-desc" id="drawing-mode">Web pages are drawn {now}.</p><p class="sec-desc"><a href="anvil://security?toggle=drawing">{action}</a></p>"#
    )
}

/// The Brave Shields & Adblock (`adblock-rust`) section of `anvil://security`.
pub fn shields_section_html(shield: &anvil_net::NetworkShield) -> String {
    let mode = shield.mode();
    let mode_label = mode.label();
    let net_rules = shield.network_rule_count();
    let cos_rules = shield.cosmetic_rule_count();
    let blocked = shield.blocked_total();
    let stripped = shield.params_stripped_total();
    let disabled_sites = shield.disabled_sites();
    let disabled_summary = if disabled_sites.is_empty() {
        "None (Shields active on all sites)".to_string()
    } else {
        disabled_sites
            .iter()
            .map(|s| escape_html(s))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        r#"<div class="section" id="shields-engine-section"><p class="sec-title">Brave Shields &amp; Adblock Engine (adblock-rust)</p><p class="sec-desc" id="shields-mode-status">Mode: {mode_label} | Network Rules: {net_rules} | Cosmetic Selectors: {cos_rules} | Blocked: {blocked} | Tracking Params Stripped: {stripped}</p><p class="sec-desc"><a href="anvil://security?toggle=shields_mode">Cycle Shields Mode (Standard / Aggressive / Disabled)</a> | <a href="anvil://security?shields_mode=standard">Standard</a> | <a href="anvil://security?shields_mode=aggressive">Aggressive</a> | <a href="anvil://security?shields_mode=disabled">Disabled</a></p><p class="sec-desc">Per-Site Exceptions (Shields Down): {disabled_summary}</p></div>"#
    )
}

/// Security center HTML with the given recent key exchanges listed.
pub fn security_html_with_key_exchanges(recent: &[anvil_net::KeyExchange]) -> String {
    security_html_with_sections(recent, CONFINEMENT_WITHOUT_WINDOW)
}

/// Security center HTML with the given recent key exchanges and the
/// renderer confinement part from [`confinement_section_html`].
pub fn security_html_with_sections(recent: &[anvil_net::KeyExchange], confinement: &str) -> String {
    let key_exchange = key_exchange_section_html(recent);
    let body = compile_tsx_component(
        SECURITY_CENTER_TSX,
        &[("version", VERSION)],
        &[
            ("KEY_EXCHANGE_SLOT", &key_exchange),
            ("CONFINEMENT_SLOT", confinement),
        ],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Security Center</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{SECURITY_JS}\n  </script>\n</body>\n</html>"
    )
}

/// Settings HTML with scripts and JIT shown as enabled and Google Search selected.
pub fn settings_html_with_state(
    gpu_backend_label: &str,
    gpu_video_enabled: bool,
    https_only: bool,
    scripts_enabled: bool,
    frames_decoded: u64,
    frames_encoded: u64,
) -> String {
    settings_html_with_profile_state_and_engine(
        gpu_backend_label,
        gpu_video_enabled,
        https_only,
        scripts_enabled,
        true,
        SearchEngine::Google,
        frames_decoded,
        frames_encoded,
        0,
        0,
        0,
    )
}

/// Settings HTML with bookmark, history, and cookie counts. JIT stays enabled and the engine is Google Search.
#[allow(clippy::too_many_arguments)]
pub fn settings_html_with_profile_state(
    gpu_backend_label: &str,
    gpu_video_enabled: bool,
    https_only: bool,
    scripts_enabled: bool,
    frames_decoded: u64,
    frames_encoded: u64,
    bookmark_count: usize,
    history_count: usize,
    cookie_count: usize,
) -> String {
    settings_html_with_profile_state_and_engine(
        gpu_backend_label,
        gpu_video_enabled,
        https_only,
        scripts_enabled,
        true,
        SearchEngine::Google,
        frames_decoded,
        frames_encoded,
        bookmark_count,
        history_count,
        cookie_count,
    )
}

/// Settings HTML for the given toggles, engine, frame counters, and profile counts.
///
/// The profile status line inserted into the page is a fixed string. It is not
/// read back from `profile.anvil`. The GPU status text says frames are decoded on
/// the CPU when the toggle is on. `anvil_core.js` and `settings.js` are included as a script tag.
#[allow(clippy::too_many_arguments)]
pub fn settings_html_with_profile_state_and_engine(
    gpu_backend_label: &str,
    gpu_video_enabled: bool,
    https_only: bool,
    scripts_enabled: bool,
    jit_enabled: bool,
    search_engine: SearchEngine,
    frames_decoded: u64,
    frames_encoded: u64,
    bookmark_count: usize,
    history_count: usize,
    cookie_count: usize,
) -> String {
    settings_html_with_full_state(
        gpu_backend_label,
        gpu_video_enabled,
        https_only,
        scripts_enabled,
        jit_enabled,
        search_engine,
        BrowserTheme::Dark,
        100,
        "all",
        frames_decoded,
        frames_encoded,
        bookmark_count,
        history_count,
        cookie_count,
    )
}

/// Settings HTML with full appearance, theme, zoom, category filter, and profile state.
#[allow(clippy::too_many_arguments)]
pub fn settings_html_with_full_state(
    gpu_backend_label: &str,
    gpu_video_enabled: bool,
    https_only: bool,
    scripts_enabled: bool,
    jit_enabled: bool,
    search_engine: SearchEngine,
    theme: BrowserTheme,
    zoom_percent: u16,
    active_section: &str,
    frames_decoded: u64,
    frames_encoded: u64,
    bookmark_count: usize,
    history_count: usize,
    cookie_count: usize,
) -> String {
    let (gpu_status, gpu_toggle_label, gpu_toggle_cls) = if gpu_video_enabled {
        ("On. Frames are decoded on the CPU.", "On", "toggle-pill-on")
    } else {
        ("Off.", "Off", "toggle-pill-off")
    };
    let (https_status, https_toggle_label, https_toggle_cls) = if https_only {
        (
            "ENABLED (Strict HTTPS-Only)",
            "Enabled (Click to Disable)",
            "toggle-pill-on",
        )
    } else {
        (
            "STANDARD (HTTPS Preferred)",
            "Standard (Click to Enable Strict)",
            "toggle-pill-off",
        )
    };
    let (scripts_status, scripts_toggle_label, scripts_toggle_cls) = if scripts_enabled {
        (
            "ENABLED (200K-Step Budget & SOP)",
            "Enabled (Click to Disable)",
            "toggle-pill-on",
        )
    } else {
        (
            "DISABLED (No Scripts Executed)",
            "Disabled (Click to Enable)",
            "toggle-pill-off",
        )
    };
    let (jit_status, jit_toggle_label, jit_toggle_cls) = if jit_enabled {
        (
            "On. Math can run in the C++ helper.",
            "On",
            "toggle-pill-on",
        )
    } else {
        (
            "Off. Scripts stay in the Rust interpreter.",
            "Off",
            "toggle-pill-off",
        )
    };
    let google_card_cls = if search_engine == SearchEngine::Google {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let ddg_card_cls = if search_engine == SearchEngine::DuckDuckGo {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let bing_card_cls = if search_engine == SearchEngine::Bing {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let dark_theme_cls = if theme == BrowserTheme::Dark {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let midnight_theme_cls = if theme == BrowserTheme::Midnight {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let light_theme_cls = if theme == BrowserTheme::Light {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let nordic_theme_cls = if theme == BrowserTheme::Nordic {
        "engine-card-active"
    } else {
        "engine-card"
    };
    let zoom_label = format!("{zoom_percent}%");
    let dec_str = frames_decoded.to_string();
    let enc_str = frames_encoded.to_string();
    let bm_str = bookmark_count.to_string();
    let hist_str = history_count.to_string();
    let ck_str = cookie_count.to_string();
    let profile_status = "ENCRYPTED & AUTHENTICATED (ANVP v1 / Windows DPAPI + 64-Bit Tag)";
    let mut body = compile_tsx_component(
        SETTINGS_PAGE_TSX,
        &[
            ("version", VERSION),
            ("activeThemeLabel", theme.label()),
            ("darkThemeClass", dark_theme_cls),
            ("midnightThemeClass", midnight_theme_cls),
            ("lightThemeClass", light_theme_cls),
            ("nordicThemeClass", nordic_theme_cls),
            ("activeZoomLabel", &zoom_label),
            ("activeSearchEngineLabel", search_engine.label()),
            ("googleCardClass", google_card_cls),
            ("ddgCardClass", ddg_card_cls),
            ("bingCardClass", bing_card_cls),
            ("scriptsEnabledStatus", scripts_status),
            ("scriptsToggleLabel", scripts_toggle_label),
            ("scriptsToggleClass", scripts_toggle_cls),
            ("jitEnabledStatus", jit_status),
            ("jitToggleLabel", jit_toggle_label),
            ("jitToggleClass", jit_toggle_cls),
            ("httpsOnlyStatus", https_status),
            ("httpsToggleLabel", https_toggle_label),
            ("httpsToggleClass", https_toggle_cls),
            ("gpuBackendLabel", gpu_backend_label),
            ("gpuVideoStatus", gpu_status),
            ("gpuToggleLabel", gpu_toggle_label),
            ("gpuToggleClass", gpu_toggle_cls),
            ("framesDecoded", &dec_str),
            ("framesEncoded", &enc_str),
            ("profileStatus", profile_status),
            ("bookmarkCount", &bm_str),
            ("historyCount", &hist_str),
            ("cookieCount", &ck_str),
        ],
        &[],
    );

    let normalized_section = match active_section {
        "appearance" | "search" | "js" | "privacy" | "system" => active_section,
        _ => "all",
    };
    if normalized_section != "all" {
        body = body.replace(
            r#"id="nav-sec-all" class="sidebar-item-active""#,
            r#"id="nav-sec-all" class="sidebar-item""#,
        );
        let from_nav = format!(r#"id="nav-sec-{normalized_section}" class="sidebar-item""#);
        let to_nav = format!(r#"id="nav-sec-{normalized_section}" class="sidebar-item-active""#);
        body = body.replace(&from_nav, &to_nav);

        for sec in ["appearance", "search", "js", "privacy", "system"] {
            if sec != normalized_section {
                let from_card = format!(r#"<div class="settings-card" data-section="{sec}">"#);
                let to_card = format!(
                    r#"<div class="settings-card" data-section="{sec}" style="display: none;">"#
                );
                body = body.replace(&from_card, &to_card);
            }
        }
    }

    let html = format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Settings</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{SETTINGS_JS}\n  </script>\n</body>\n</html>"
    );
    with_theme_css(&html, theme)
}

/// Replace each `{{name}}` in `template` with its value. Values go in as
/// given, so text must already be escaped with [`escape_html`].
pub fn fill_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    out
}

/// A link on the permissions page that changes `host`. `change` is the
/// query text for the change, such as `scripts=block`.
pub fn site_permission_link(host: &str, change: &str, key: &str) -> String {
    format!(
        "{SITE_PERMISSIONS_URL}?site={}&{change}&key={}",
        urlencode_component(host),
        urlencode_component(key)
    )
}

/// The per-site permissions page.
///
/// `cookie_count` gives how many cookies are stored for a host. `message`
/// is shown above the list. `key` goes into every link that changes
/// something; see `permissions::action_key`.
pub fn site_permissions_html(
    perms: &crate::permissions::SitePermissions,
    scripts_global: bool,
    https_only: bool,
    cookie_count: &dyn Fn(&str) -> usize,
    message: &str,
    key: &str,
) -> String {
    use crate::permissions::ScriptRule;
    let mut rows = String::new();
    for site in perms.sites() {
        let host = site.host.as_str();
        let link = |change: &str| escape_html(&site_permission_link(host, change, key));
        let scripts = match site.scripts {
            ScriptRule::Default => "follows the main switch",
            ScriptRule::Allow => "allowed",
            ScriptRule::Block => "blocked",
        };
        let (cookies, cookies_action, cookies_change) = if site.block_cookies {
            ("blocked", "Allow cookies", "cookies=allow")
        } else {
            ("allowed", "Block cookies", "cookies=block")
        };
        let (http, http_action, http_change) = if site.allow_http {
            ("allowed", "Require HTTPS", "http=require")
        } else {
            ("HTTPS required", "Allow plain HTTP", "http=allow")
        };
        let count = cookie_count(host).to_string();
        rows.push_str(&fill_template(
            SITE_PERMISSION_ROW_HTML,
            &[
                ("host", &escape_html(host)),
                ("scripts", scripts),
                ("cookies", cookies),
                ("cookieCount", &count),
                ("http", http),
                ("scriptsDefaultLink", &link("scripts=default")),
                ("scriptsAllowLink", &link("scripts=allow")),
                ("scriptsBlockLink", &link("scripts=block")),
                ("cookiesLink", &link(cookies_change)),
                ("cookiesAction", cookies_action),
                ("httpLink", &link(http_change)),
                ("httpAction", http_action),
                ("clearLink", &link("clear_cookies=1")),
                ("removeLink", &link("remove=1")),
            ],
        ));
    }
    let count = perms.len().to_string();
    let empty_note = if perms.is_empty() {
        "No sites yet. Add one above, or use the link on a page that couldn't load over HTTPS."
    } else {
        ""
    };
    let body = fill_template(
        SITE_PERMISSIONS_HTML,
        &[
            ("scriptsGlobal", if scripts_global { "on" } else { "off" }),
            ("httpsOnly", if https_only { "on" } else { "off" }),
            ("message", &escape_html(message)),
            ("key", &escape_html(key)),
            ("siteCount", &count),
            ("emptyNote", empty_note),
            ("rows", &rows),
        ],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Site Permissions</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// The page shown when HTTPS-only is on and `http_url` could not be loaded
/// over HTTPS. Its one action allows plain HTTP for that host, using `key`.
pub fn https_only_html(http_url: &str, key: &str) -> String {
    let host = crate::permissions::host_of(http_url).unwrap_or_default();
    let continue_link = format!(
        "{SITE_PERMISSIONS_URL}?allow_http={}&open={}&key={}",
        urlencode_component(&host),
        urlencode_component(http_url),
        urlencode_component(key)
    );
    fill_template(
        HTTPS_ONLY_HTML,
        &[
            ("host", &escape_html(&host)),
            ("url", &escape_html(http_url)),
            ("continueLink", &escape_html(&continue_link)),
        ],
    )
}

/// Settings HTML from a default `GpuVideoSession`, with placeholder frame counts of 12.
pub fn settings_html() -> String {
    let session = GpuVideoSession::default();
    settings_html_with_state(
        session.backend().label(),
        session.is_hardware_decode_enabled(),
        false,
        true,
        12,
        12,
    )
}

/// History page HTML. Entries are escaped and listed newest first. Includes `anvil_core.js` and `history.js`.
pub fn history_html_with_entries(entries: &[Bookmark]) -> String {
    let count_str = entries.len().to_string();
    let mut items_html = String::new();
    if entries.is_empty() {
        items_html.push_str(
            r#"<div class="history-item"><p class="sec-desc">No pages recorded in the current browsing session.</p></div>"#,
        );
    } else {
        for (idx, item) in entries.iter().enumerate().rev() {
            let title = escape_html(&item.title);
            let url = escape_html(&item.url);
            let num = idx + 1;
            items_html.push_str(&format!(
                r#"<div class="history-item"><p class="sec-title">{num}. {title}</p><p class="sec-desc"><a href="{url}">{url}</a></p></div>"#
            ));
        }
    }
    let body = compile_tsx_component(
        HISTORY_PAGE_TSX,
        &[("version", VERSION), ("entryCount", &count_str)],
        &[("HISTORY_ITEMS_SLOT", &items_html)],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil History</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{HISTORY_JS}\n  </script>\n</body>\n</html>"
    )
}

/// History page HTML with no entries.
pub fn history_html() -> String {
    history_html_with_entries(&[])
}

/// Downloads page HTML. Entries are escaped and listed newest first. Includes `anvil_core.js` and `downloads.js`.
pub fn downloads_html_with_entries(entries: &[DownloadEntry]) -> String {
    let count_str = entries.len().to_string();
    let mut items_html = String::new();
    if entries.is_empty() {
        items_html.push_str(
            r#"<div class="history-item"><p class="sec-desc">No files downloaded in the current session. Use the verification links above to test SHA-256 download verification.</p></div>"#,
        );
    } else {
        for item in entries.iter().rev() {
            let name = escape_html(&item.file_name);
            let url = escape_html(&item.source_url);
            let mime = escape_html(&item.mime_type);
            let sha = escape_html(&item.sha256_hex);
            let status = item.state.label();
            let motw = if item.motw_tagged {
                "MOTW: ZoneId=3"
            } else {
                "MOTW: Local"
            };
            let bytes = item.byte_size;
            items_html.push_str(&format!(
                r#"<div class="history-item"><p class="sec-title">#{id} — {name} ({bytes} bytes) [{status} | {motw}] <a href="anvil://downloads?export={id}">[Export Receipt]</a></p><p class="sec-desc">Source: {url} ({mime})</p><p class="sec-desc">SHA-256: {sha}</p></div>"#,
                id = item.id
            ));
        }
    }
    let body = compile_tsx_component(
        DOWNLOADS_PAGE_TSX,
        &[("version", VERSION), ("entryCount", &count_str)],
        &[("DOWNLOAD_ITEMS_SLOT", &items_html)],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Downloads</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n  <script>\n{ANVIL_CORE_JS}\n{DOWNLOADS_JS}\n  </script>\n</body>\n</html>"
    )
}

/// Downloads page HTML with no entries.
pub fn downloads_html() -> String {
    downloads_html_with_entries(&[])
}

/// Line-numbered HTML source inspector page for `view-source:<url>` (`Ctrl+U`).
pub fn view_source_html(target_url: &str, raw_html: &str) -> String {
    let escaped_url = escape_html(target_url);
    let byte_len = raw_html.len();
    let lines: Vec<&str> = raw_html.lines().collect();
    let line_count = lines.len().max(1);
    let mut numbered = String::with_capacity(raw_html.len().saturating_add(line_count * 12));
    if lines.is_empty() {
        numbered.push_str("   1 | ");
    } else {
        for (idx, line) in lines.iter().take(2500).enumerate() {
            let line_no = idx + 1;
            numbered.push_str(&format!("{line_no:>4} | {}\n", escape_html(line)));
        }
    }
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>view-source:{escaped_url}</title>\n  <style>\n{BROWSER_CSS}\n  .source-pre {{ background: #0d111a; color: #dbe6f8; border: 1px solid #293246; border-radius: 8px; padding: 14px; font-size: 13px; line-height: 20px; white-space: pre-wrap; }}\n  </style>\n</head>\n<body>\n  <div class=\"wrap\">\n    <div class=\"brand-bar\">\n      <img id=\"anvil-brand-logo\" src=\"anvil://logo.png\" width=\"38\" height=\"38\" alt=\"Anvil\" />\n      <p class=\"brand-title\">Anvil Source Inspector</p>\n      <p class=\"brand-nav\"><a href=\"anvil://home\">Start</a> | <a href=\"anvil://inspect\">Inspector</a> | <a href=\"anvil://settings\">Settings</a> | <a href=\"anvil://downloads\">Downloads</a> | <a href=\"anvil://history\">History</a></p>\n    </div>\n    <div class=\"hero\">\n      <p class=\"badge\">ANVIL {VERSION} — VIEW-SOURCE PROTOCOL</p>\n      <h1>Source of {escaped_url}</h1>\n      <p class=\"subtitle\">{line_count} lines | {byte_len} bytes | <a href=\"{escaped_url}\">Open Rendered Page</a></p>\n    </div>\n    <div class=\"section\">\n      <pre id=\"anvil-view-source-pre\" class=\"source-pre\">{numbered}</pre>\n    </div>\n  </div>\n</body>\n</html>"
    )
}

/// Fill `frontend/App.tsx` text from `session` and `ui`.
///
/// The function name is historical. The return value is an HTML string. The
/// viewport slot is left empty. `shell` and `ui` paint the page separately.
pub fn render_react_shell_html(session: &Session, ui: &UiState) -> String {
    let active_idx = session.active_index();
    let tab_count = session.tabs().len();
    let mut tabs_html = String::new();
    for (idx, tab) in session.tabs().iter().enumerate() {
        let class_name = if idx == active_idx {
            "tab-active"
        } else {
            "tab-pill"
        };
        let title = escape_html(tab.title());
        let close_btn = if tab_count > 1 {
            format!(
                r#" <span class="tab-close" data-ui-action="close-tab" data-tab-index="{idx}">x</span>"#
            )
        } else {
            String::new()
        };
        tabs_html.push_str(&format!(
            r#"<div class="{class_name}" data-ui-action="select-tab" data-tab-index="{idx}">{title}{close_btn}</div>"#
        ));
    }

    let bookmarks_html = if session.bookmarks().is_empty() {
        String::new()
    } else {
        let mut bar = String::from(r#"<div id="anvil-bookmarks-bar" class="bookmarks-bar">"#);
        for (idx, bm) in session.bookmarks().iter().enumerate() {
            let label = escape_html(&bm.title);
            bar.push_str(&format!(
                r#"<button class="bookmark-pill" data-ui-action="open-bookmark" data-bookmark-index="{idx}">{label}</button>"#
            ));
        }
        bar.push_str("</div>");
        bar
    };

    let active_url = session
        .active()
        .current()
        .map(|p| p.url.as_str())
        .unwrap_or(HOME_URL);

    let (sec_label, sec_class) = if active_url.starts_with("https://") {
        ("TLS", "security-pill-tls")
    } else if active_url.starts_with("anvil://") || active_url.starts_with("about:") {
        ("SYS", "security-pill-sys")
    } else if active_url.starts_with("http://") {
        ("HTTP", "security-pill-http")
    } else {
        ("FILE", "security-pill-file")
    };

    let (address_display, address_class) = if ui.is_editing_address() {
        (
            format!("{}|", escape_html(ui.input())),
            "address-input-focused",
        )
    } else {
        (escape_html(active_url), "address-input")
    };

    let star_class = if session.is_active_bookmarked() {
        "nav-btn-active"
    } else {
        "nav-btn"
    };
    let inspector_class = if ui.is_inspector_open() {
        "nav-btn-active"
    } else {
        "nav-btn"
    };
    let app_menu_class = if ui.is_app_menu_open() {
        "nav-btn-active"
    } else {
        "nav-btn"
    };
    let zoom_display = format!("{}%", session.zoom_percent());

    let find_bar_html = if ui.is_find_open() {
        let q = escape_html(ui.find_query());
        format!(
            r#"<div id="anvil-find-bar" class="find-bar"><span id="anvil-find-input">Find: {q}|</span><button id="anvil-find-prev" data-ui-action="find-prev">&lt;</button><button id="anvil-find-next" data-ui-action="find-next">&gt;</button><button id="anvil-find-close" data-ui-action="find-close">x</button></div>"#
        )
    } else {
        String::new()
    };

    let app_menu_html = if ui.is_app_menu_open() {
        r#"<div id="anvil-app-menu" class="app-menu"><div data-ui-action="menu-new-tab">New Tab</div><div data-ui-action="menu-home">Start Dashboard</div><div data-ui-action="menu-downloads">Downloads &amp; Vault</div><div data-ui-action="menu-history">Browsing History</div><div data-ui-action="menu-security">Security &amp; Shields</div><div data-ui-action="menu-find">Find in Page</div><div data-ui-action="menu-inspector-dock">Developer Inspector Dock</div><div data-ui-action="menu-inspector-page">Full-Page Inspector</div><div data-ui-action="menu-zoom">Adjust / Reset Zoom</div><div data-ui-action="menu-theme">Switch Theme</div><div data-ui-action="menu-settings">Browser Settings</div></div>"#.to_string()
    } else {
        String::new()
    };

    let context_menu_html = if ui.context_menu_pos().is_some() {
        r#"<div id="anvil-context-menu" class="context-menu"><div data-ui-action="ctx-back">Back</div><div data-ui-action="ctx-forward">Forward</div><div data-ui-action="ctx-reload">Reload</div><div data-ui-action="ctx-copy">Copy</div><div data-ui-action="ctx-paste">Paste</div><div data-ui-action="ctx-find">Find in Page</div><div data-ui-action="ctx-zoom">Reset Zoom</div><div data-ui-action="ctx-inspect">Inspect Page</div><div data-ui-action="ctx-security">Security Shield</div></div>"#.to_string()
    } else {
        String::new()
    };

    let inspector_dock_html = if ui.is_inspector_open() {
        let tab_label = ui.inspector_tab().label();
        format!(
            r#"<div id="anvil-inspector-dock" class="inspector-dock"><span id="anvil-inspector-active-tab">DEVTOOLS — {tab_label}</span><button id="anvil-inspector-full-page" data-ui-action="inspector-full-page">Full Page</button><button id="anvil-inspector-close" data-ui-action="inspector-close">x</button></div>"#
        )
    } else {
        String::new()
    };

    let status_left = if ui.is_loading() {
        "Loading resource...".to_string()
    } else if let Some(link) = ui.hovered_link() {
        format!("Link: {}", escape_html(link))
    } else {
        format!("Ready — {}", escape_html(active_url))
    };

    let status_right = format!("Anvil {VERSION}");

    let body = compile_tsx_component(
        APP_TSX,
        &[
            ("version", VERSION),
            ("activeUrl", &escape_html(active_url)),
            ("addressDisplay", &address_display),
            ("addressClass", address_class),
            ("securityLabel", sec_label),
            ("securityClass", sec_class),
            ("zoomDisplay", &zoom_display),
            ("starClass", star_class),
            ("inspectorClass", inspector_class),
            ("appMenuClass", app_menu_class),
            ("statusLeft", &status_left),
            ("statusRight", &status_right),
        ],
        &[
            ("TABS_SLOT", &tabs_html),
            ("BOOKMARKS_SLOT", &bookmarks_html),
            ("FIND_BAR_SLOT", &find_bar_html),
            ("APP_MENU_SLOT", &app_menu_html),
            ("CONTEXT_MENU_SLOT", &context_menu_html),
            ("INSPECTOR_DOCK_SLOT", &inspector_dock_html),
            ("VIEWPORT_SLOT", ""),
        ],
    );

    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Anvil Browser React Shell</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Parse `render_react_shell_html` with `anvil-dom`. This does not run scripts.
pub fn render_react_shell_document(session: &Session, ui: &UiState) -> Document {
    let html = render_react_shell_html(session, ui);
    parse_html(&html)
}

#[cfg(test)]
mod tests {
    //! Checks that the text replacements remove `className=` and leave the expected element ids.
    use super::*;

    #[test]
    /// The text replacement removes `className=` and leaves the expected element ids.
    fn compiles_react_tsx_shell_and_builtin_dashboards_into_dom() {
        let home = home_html();
        assert!(home.contains("Google Search"));
        assert!(!home.contains("className="));
        let home_doc = parse_html(&home);
        assert!(home_doc.get_element_by_id("home-q").is_some());
        assert!(home_doc.get_element_by_id("home-engine-badge").is_some());

        let insp = inspect_html();
        assert!(insp.contains("Developer Inspector"));
        assert!(!insp.contains("className="));
        let insp_doc = parse_html(&insp);
        assert!(
            insp_doc
                .get_element_by_id("anvil-inspector-table")
                .is_some()
        );

        let sec = security_html();
        assert!(sec.contains("Security"));
        assert!(!sec.contains("className="));
        assert!(!sec.contains("KEY_EXCHANGE_SLOT"));

        let settings = settings_html();
        assert!(settings.contains("Video"));
        assert!(settings.contains("Google Search"));
        assert!(settings.contains("DuckDuckGo"));
        assert!(settings.contains("Microsoft Bing"));
        assert!(settings.contains("JavaScript"));
        assert!(!settings.contains("className="));
        let settings_doc = parse_html(&settings);
        assert!(settings_doc.get_element_by_id("gpu-demo-video").is_some());
        assert!(settings_doc.get_element_by_id("nav-sec-search").is_some());
        assert!(settings_doc.get_element_by_id("nav-sec-js").is_some());

        let hist = history_html();
        assert!(hist.contains("Browsing History"));
        assert!(!hist.contains("className="));
        let hist_doc = parse_html(&hist);
        assert!(hist_doc.get_element_by_id("anvil-history-list").is_some());

        let dl = downloads_html();
        assert!(dl.contains("Download Manager"));
        assert!(!dl.contains("className="));
        let dl_doc = parse_html(&dl);
        assert!(dl_doc.get_element_by_id("anvil-downloads-list").is_some());

        let session = Session::new(crate::Viewport {
            width: 960.0,
            height: 720.0,
        });
        let mut ui = UiState::new();
        ui.toggle_app_menu();
        ui.toggle_inspector();
        let shell_doc = render_react_shell_document(&session, &ui);
        assert!(shell_doc.get_element_by_id("anvil-react-shell").is_some());
        assert!(shell_doc.get_element_by_id("anvil-shell-logo").is_some());
        assert!(shell_doc.get_element_by_id("anvil-tab-strip").is_some());
        assert!(shell_doc.get_element_by_id("anvil-nav-bar").is_some());
        assert!(
            shell_doc
                .get_element_by_id("anvil-security-badge")
                .is_some()
        );
        assert!(shell_doc.get_element_by_id("anvil-address-bar").is_some());
        assert!(shell_doc.get_element_by_id("anvil-zoom-pill").is_some());
        assert!(shell_doc.get_element_by_id("anvil-btn-settings").is_some());
        assert!(shell_doc.get_element_by_id("anvil-btn-downloads").is_some());
        assert!(shell_doc.get_element_by_id("anvil-btn-inspector").is_some());
        assert!(shell_doc.get_element_by_id("anvil-btn-app-menu").is_some());
        assert!(shell_doc.get_element_by_id("anvil-app-menu").is_some());
        assert!(
            shell_doc
                .get_element_by_id("anvil-inspector-dock")
                .is_some()
        );
        assert!(shell_doc.get_element_by_id("anvil-status-bar").is_some());
    }
}
