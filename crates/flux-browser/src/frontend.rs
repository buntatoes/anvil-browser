//! React/TSX front-end shell compiler and built-in `flux://` dashboard pages.
//!
//! Embeds the privileged React/TSX UI components (`frontend/App.tsx`,
//! `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`,
//! `frontend/components/SettingsPage.tsx`, `frontend/components/HistoryPage.tsx`)
//! and stylesheet (`frontend/styles/browser.css`), compiling them into Flux DOM trees.

use flux_dom::{Document, parse_html};
use flux_video::GpuVideoSession;

use crate::ui::{UiState, normalize_address_input};
use crate::{Bookmark, Session, VERSION};

pub const HOME_URL: &str = "flux://home";
pub const SECURITY_URL: &str = "flux://security";
pub const SETTINGS_URL: &str = "flux://settings";
pub const HISTORY_URL: &str = "flux://history";
pub const NAVIGATE_URL_PREFIX: &str = "flux://navigate";

/// Embedded React/TSX root browser shell component (`frontend/App.tsx`).
pub const APP_TSX: &str = include_str!("../../../frontend/App.tsx");

/// Embedded React/TSX start dashboard component (`frontend/components/StartDashboard.tsx`).
pub const START_DASHBOARD_TSX: &str =
    include_str!("../../../frontend/components/StartDashboard.tsx");

/// Embedded React/TSX security center component (`frontend/components/SecurityCenter.tsx`).
pub const SECURITY_CENTER_TSX: &str =
    include_str!("../../../frontend/components/SecurityCenter.tsx");

/// Embedded React/TSX settings & GPU video component (`frontend/components/SettingsPage.tsx`).
pub const SETTINGS_PAGE_TSX: &str = include_str!("../../../frontend/components/SettingsPage.tsx");

/// Embedded React/TSX browsing history component (`frontend/components/HistoryPage.tsx`).
pub const HISTORY_PAGE_TSX: &str = include_str!("../../../frontend/components/HistoryPage.tsx");

/// Embedded stylesheet (`frontend/styles/browser.css`).
pub const BROWSER_CSS: &str = include_str!("../../../frontend/styles/browser.css");

/// Extract the JSX markup inside a TSX component's `return ( ... );` block.
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

/// Compile a TSX component source into HTML by binding `props` and converting JSX attributes.
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

/// Intercept internal `flux://navigate?q=...` form submissions and resolve the destination URL.
pub fn resolve_navigation_query(target: &str) -> Option<String> {
    let trimmed = target.trim();
    let rest = trimmed.strip_prefix(NAVIGATE_URL_PREFIX)?;
    let query = rest.strip_prefix('?').unwrap_or("");
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k == "q" {
            let decoded = urldecode_component(v);
            return normalize_address_input(&decoded).or_else(|| Some(HOME_URL.to_string()));
        }
    }
    Some(HOME_URL.to_string())
}

/// Return `(canonical_url, html_source)` if `target` names a built-in `flux://` page.
pub fn resolve_builtin_page(target: &str) -> Option<(String, String)> {
    let trimmed = target.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "flux://home" | "flux://" | "about:home" | "about:blank" => {
            Some((HOME_URL.to_string(), home_html()))
        }
        "flux://security" | "flux://shields" => Some((SECURITY_URL.to_string(), security_html())),
        s if s == "flux://settings" || s.starts_with("flux://settings?") => {
            Some((SETTINGS_URL.to_string(), settings_html()))
        }
        s if s == "flux://history" || s.starts_with("flux://history?") => {
            Some((HISTORY_URL.to_string(), history_html()))
        }
        _ => None,
    }
}

/// Built-in `flux://home` start dashboard compiled from `frontend/components/StartDashboard.tsx`.
pub fn home_html() -> String {
    let body = compile_tsx_component(
        START_DASHBOARD_TSX,
        &[
            ("version", VERSION),
            ("defaultQuery", "https://example.com"),
        ],
        &[],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Flux Start</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Built-in `flux://security` dashboard compiled from `frontend/components/SecurityCenter.tsx`.
pub fn security_html() -> String {
    let body = compile_tsx_component(SECURITY_CENTER_TSX, &[("version", VERSION)], &[]);
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Flux Security Center</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Built-in `flux://settings` page compiled from `frontend/components/SettingsPage.tsx`.
pub fn settings_html_with_state(
    gpu_backend_label: &str,
    gpu_video_enabled: bool,
    https_only: bool,
    scripts_enabled: bool,
    frames_decoded: u64,
    frames_encoded: u64,
) -> String {
    let gpu_status = if gpu_video_enabled {
        "ENABLED (NV12 Hardware Decode & Encode)"
    } else {
        "DISABLED (Software Fallback)"
    };
    let https_status = if https_only {
        "ENABLED (Strict HTTPS-Only)"
    } else {
        "STANDARD (HTTPS Preferred)"
    };
    let scripts_status = if scripts_enabled {
        "ENABLED (100K-Step Budget & SOP)"
    } else {
        "DISABLED (No Scripts Executed)"
    };
    let dec_str = frames_decoded.to_string();
    let enc_str = frames_encoded.to_string();
    let body = compile_tsx_component(
        SETTINGS_PAGE_TSX,
        &[
            ("version", VERSION),
            ("gpuBackendLabel", gpu_backend_label),
            ("gpuVideoStatus", gpu_status),
            ("httpsOnlyStatus", https_status),
            ("scriptsEnabledStatus", scripts_status),
            ("framesDecoded", &dec_str),
            ("framesEncoded", &enc_str),
        ],
        &[],
    );
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Flux Settings</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Default `flux://settings` HTML snapshot using default `GpuVideoSession` state.
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

/// Built-in `flux://history` page compiled from `frontend/components/HistoryPage.tsx`.
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
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Flux History</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Default `flux://history` HTML snapshot.
pub fn history_html() -> String {
    history_html_with_entries(&[])
}

/// Compile the full React/TSX browser shell (`frontend/App.tsx`) for the current `Session` and `UiState`.
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
        let mut bar = String::from(r#"<div id="flux-bookmarks-bar" class="bookmarks-bar">"#);
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
        .map(|p| p.source.as_str())
        .unwrap_or(HOME_URL);

    let (sec_label, sec_class) = if active_url.starts_with("https://") {
        ("TLS", "security-pill-tls")
    } else if active_url.starts_with("flux://") || active_url.starts_with("about:") {
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
        "nav-btn bookmark-active"
    } else {
        "nav-btn"
    };

    let status_left = if ui.is_loading() {
        "Loading resource...".to_string()
    } else if let Some(link) = ui.hovered_link() {
        format!("Link: {}", escape_html(link))
    } else {
        format!("Ready — {}", escape_html(active_url))
    };

    let status_right = format!("React TSX Shell | Sandbox: Enforced | Flux {VERSION}");

    let body = compile_tsx_component(
        APP_TSX,
        &[
            ("version", VERSION),
            ("activeUrl", &escape_html(active_url)),
            ("addressDisplay", &address_display),
            ("addressClass", address_class),
            ("securityLabel", sec_label),
            ("securityClass", sec_class),
            ("starClass", star_class),
            ("statusLeft", &status_left),
            ("statusRight", &status_right),
        ],
        &[
            ("TABS_SLOT", &tabs_html),
            ("BOOKMARKS_SLOT", &bookmarks_html),
            ("VIEWPORT_SLOT", ""),
        ],
    );

    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <title>Flux Browser React Shell</title>\n  <style>\n{BROWSER_CSS}\n  </style>\n</head>\n<body>\n{body}\n</body>\n</html>"
    )
}

/// Compile `frontend/App.tsx` and parse it into a native `flux_dom::Document`.
pub fn render_react_shell_document(session: &Session, ui: &UiState) -> Document {
    let html = render_react_shell_html(session, ui);
    parse_html(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_react_tsx_shell_and_builtin_dashboards_into_dom() {
        let home = home_html();
        assert!(home.contains("REACT TSX SHELL"));
        assert!(!home.contains("className="));
        let home_doc = parse_html(&home);
        assert!(home_doc.get_element_by_id("home-q").is_some());

        let sec = security_html();
        assert!(sec.contains("Security &amp; Sandbox Status"));
        assert!(!sec.contains("className="));

        let settings = settings_html();
        assert!(settings.contains("GPU Hardware Video Encode"));
        assert!(!settings.contains("className="));
        let settings_doc = parse_html(&settings);
        assert!(settings_doc.get_element_by_id("gpu-demo-video").is_some());

        let hist = history_html();
        assert!(hist.contains("Browsing History"));
        assert!(!hist.contains("className="));
        let hist_doc = parse_html(&hist);
        assert!(hist_doc.get_element_by_id("flux-history-list").is_some());

        let session = Session::new(crate::Viewport {
            width: 960.0,
            height: 720.0,
        });
        let ui = UiState::new();
        let shell_doc = render_react_shell_document(&session, &ui);
        assert!(shell_doc.get_element_by_id("flux-react-shell").is_some());
        assert!(shell_doc.get_element_by_id("flux-tab-strip").is_some());
        assert!(shell_doc.get_element_by_id("flux-nav-bar").is_some());
        assert!(shell_doc.get_element_by_id("flux-security-badge").is_some());
        assert!(shell_doc.get_element_by_id("flux-address-bar").is_some());
        assert!(shell_doc.get_element_by_id("flux-status-bar").is_some());
    }
}
