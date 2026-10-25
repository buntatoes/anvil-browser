//! Which large sites the engine loads itself, and which scripts it must not fetch.
//!
//! YouTube, Gmail, Facebook, Instagram, X, Outlook, TikTok, LinkedIn, and
//! Google Maps are recognized by host. Their documents load through the normal
//! HTTP path. Their external scripts are not fetched: those bundles are not
//! the subset interpreter, and downloading them keeps the tab worker from
//! painting the document. Inline scripts still run when they fit
//! [`anvil_script::script_runs_here`].
//!
//! Reddit is rewritten to `old.reddit.com`, which is server-rendered HTML.
//! Requests to a Reddit host use a longer `User-Agent` that names this browser.
//!
//! A YouTube watch URL keeps the document YouTube returned and adds a player.
//! The player is built from the JPEG frames YouTube publishes for that video,
//! or from a storyboard sheet when the document includes one. This crate does
//! not decode H.264, and it does not add a codec library.
//!
//! No API key, client secret, or password is stored in this file.

use std::sync::Mutex;

use anvil_image::{Image, decode_auto};
use anvil_json::parse_json;
use anvil_net::{CookieJar, fetch, fetch_with_cookies, parse_url};
use anvil_video::{VideoClip, VideoCodecProfile, VideoFrame, probe_platform_gpu_backend};

use crate::frontend::{escape_html, urlencode_component};

const MAX_SCAN: usize = 256 * 1024;
const MAX_RESULTS: usize = 8;
const MAX_MAIL: usize = 15;

/// Which large site a URL belongs to, if this module handles it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppKind {
    YouTube,
    Gmail,
    Facebook,
    Instagram,
    X,
    Outlook,
    TikTok,
    LinkedIn,
    Maps,
}

/// Send Reddit (not `old.reddit.com`) to the server-rendered host.
///
/// Other URLs return `None`. The caller loads the rewritten URL through the
/// normal pipeline, so this function does not fetch.
pub fn passthrough_rewrite(target: &str) -> Option<String> {
    let parsed = parse_url(target.trim()).ok()?;
    if !is_reddit_shell_host(&parsed.host) {
        return None;
    }
    let mut out = format!("{}://old.reddit.com{}", parsed.scheme, parsed.path);
    if let Some(query) = &parsed.query {
        out.push('?');
        out.push_str(query);
    }
    Some(out)
}

/// External script files on a recognized app host are not downloaded.
///
/// The document is still loaded. Inline scripts are decided separately by
/// `anvil_script::script_runs_here`.
pub fn fetch_page_scripts(page_url: &str) -> bool {
    app_kind(page_url).is_none()
}

/// HTML summary kept for tests. Navigation does not use this.
pub fn basic_html(target: &str, jar: Option<&Mutex<CookieJar>>) -> Option<String> {
    let kind = app_kind(target)?;
    Some(match kind {
        AppKind::YouTube => youtube_html(target, jar),
        AppKind::Gmail => mail_html(
            target,
            jar,
            "Gmail",
            "https://mail.google.com/mail/feed/atom",
        ),
        AppKind::Outlook => card_html(AppKind::Outlook, target, jar),
        AppKind::Maps => maps_html(target),
        AppKind::Facebook
        | AppKind::Instagram
        | AppKind::X
        | AppKind::TikTok
        | AppKind::LinkedIn => card_html(kind, target, jar),
    })
}

pub(crate) fn app_kind(target: &str) -> Option<AppKind> {
    let parsed = parse_url(target.trim()).ok()?;
    let host = parsed.host.as_str();
    if is_youtube_host(host) {
        return Some(AppKind::YouTube);
    }
    if host == "mail.google.com"
        || host == "gmail.com"
        || host.ends_with(".gmail.com")
        || host == "googlemail.com"
        || host.ends_with(".googlemail.com")
    {
        return Some(AppKind::Gmail);
    }
    if host == "facebook.com"
        || host.ends_with(".facebook.com")
        || host == "fb.com"
        || host.ends_with(".fb.com")
        || host == "fb.watch"
        || host.ends_with(".fb.watch")
    {
        return Some(AppKind::Facebook);
    }
    if host == "instagram.com" || host.ends_with(".instagram.com") {
        return Some(AppKind::Instagram);
    }
    if host == "x.com"
        || host.ends_with(".x.com")
        || host == "twitter.com"
        || host.ends_with(".twitter.com")
    {
        return Some(AppKind::X);
    }
    if host == "outlook.live.com" || host == "outlook.office.com" || host == "outlook.office365.com"
    {
        return Some(AppKind::Outlook);
    }
    if host == "tiktok.com" || host.ends_with(".tiktok.com") {
        return Some(AppKind::TikTok);
    }
    if host == "linkedin.com" || host.ends_with(".linkedin.com") {
        return Some(AppKind::LinkedIn);
    }
    if host == "maps.google.com"
        || ((host == "google.com" || host == "www.google.com") && parsed.path.starts_with("/maps"))
    {
        return Some(AppKind::Maps);
    }
    None
}

fn is_youtube_host(host: &str) -> bool {
    host == "youtu.be"
        || host.ends_with(".youtu.be")
        || host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtube-nocookie.com"
        || host.ends_with(".youtube-nocookie.com")
}

fn is_reddit_shell_host(host: &str) -> bool {
    if host == "old.reddit.com" {
        return false;
    }
    host == "reddit.com"
        || host == "www.reddit.com"
        || host == "np.reddit.com"
        || host == "new.reddit.com"
        || host == "amp.reddit.com"
        || host == "i.reddit.com"
        || host == "m.reddit.com"
}

pub(crate) fn youtube_video_id(target: &str) -> Option<String> {
    let parsed = parse_url(target.trim()).ok()?;
    if !is_youtube_host(&parsed.host) {
        return None;
    }
    if let Some(query) = &parsed.query {
        if let Some(id) = query_item(query, "v") {
            if is_video_id(&id) {
                return Some(id);
            }
        }
    }
    let path = parsed.path.trim_matches('/');
    if parsed.host == "youtu.be" || parsed.host.ends_with(".youtu.be") {
        let id = path.split('/').next().unwrap_or("");
        if is_video_id(id) {
            return Some(id.to_string());
        }
        return None;
    }
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    if matches!(first, "embed" | "shorts" | "live" | "v" | "e") && is_video_id(second) {
        return Some(second.to_string());
    }
    None
}

fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn youtube_html(target: &str, jar: Option<&Mutex<CookieJar>>) -> String {
    let form = youtube_form();
    if let Some(id) = youtube_video_id(target) {
        let watch = format!("https://www.youtube.com/watch?v={id}");
        let endpoint = format!(
            "https://www.youtube.com/oembed?format=json&url={}",
            urlencode_component(&watch)
        );
        let fetched = http_text(&endpoint, jar);
        let card = match fetched {
            Ok(body) => match parse_oembed(&body) {
                Some(card) => render_youtube_watch(&id, &card),
                None => {
                    format!(
                        "<p>YouTube did not return a title for this video.</p>{}",
                        thumb(&id)
                    )
                }
            },
            Err(err) => format!("<p>{}</p>{}", escape_html(&err), thumb(&id)),
        };
        return page(
            "YouTube",
            &format!(
                "{form}{card}<p class=\"note\">The YouTube web app is not run. Video bytes are not downloaded.</p>"
            ),
        );
    }
    if let Some(parsed) = parse_url(target.trim()).ok() {
        if parsed.path == "/results" || parsed.path.starts_with("/results/") {
            let query = parsed
                .query
                .as_deref()
                .and_then(|q| query_item(q, "search_query"))
                .unwrap_or_default();
            let list = if query.trim().is_empty() {
                "<p>Type a search, then press Search.</p>".to_string()
            } else {
                let url = format!(
                    "https://www.youtube.com/results?hl=en&search_query={}",
                    urlencode_component(query.trim())
                );
                match http_text(&url, jar) {
                    Ok(body) => render_youtube_results(&parse_youtube_results(&body)),
                    Err(err) => format!("<p>{}</p>", escape_html(&err)),
                }
            };
            return page(
                "YouTube",
                &format!("{form}{list}<p class=\"note\">The YouTube web app is not run.</p>"),
            );
        }
    }
    page(
        "YouTube",
        &format!(
            "{form}<p class=\"note\">The YouTube web app is not run. A watch link or a search shows the basic view.</p>"
        ),
    )
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OEmbed {
    pub title: String,
    pub author: String,
    pub thumbnail: String,
}

pub(crate) fn parse_oembed(json: &str) -> Option<OEmbed> {
    let value = parse_json(json).ok()?;
    let title = value.get_str("title")?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let author = value
        .get_str("author_name")
        .unwrap_or("")
        .trim()
        .to_string();
    let thumbnail = value
        .get_str("thumbnail_url")
        .unwrap_or("")
        .trim()
        .to_string();
    Some(OEmbed {
        title,
        author,
        thumbnail,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct YtHit {
    pub id: String,
    pub title: String,
}

/// Pull up to eight `videoId` values out of a YouTube results document.
pub(crate) fn parse_youtube_results(html: &str) -> Vec<YtHit> {
    parse_youtube_hits_in(scan_prefix(html))
}

fn parse_youtube_hits_in(html: &str) -> Vec<YtHit> {
    let mut out: Vec<YtHit> = Vec::new();
    let mut rest = html;
    while out.len() < MAX_RESULTS {
        let Some(idx) = rest.find("\"videoId\":\"") else {
            break;
        };
        let after = &rest[idx + "\"videoId\":\"".len()..];
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(11)
            .collect();
        let advance = id.len().max(1);
        if is_video_id(&id) && !out.iter().any(|hit| hit.id == id) {
            let window = &after[..after.len().min(1500)];
            let title = title_near_video(window).unwrap_or_default();
            out.push(YtHit { id, title });
        }
        rest = &after[advance.min(after.len())..];
    }
    out
}

fn title_near_video(window: &str) -> Option<String> {
    const MARK: &str = "\"title\":{\"runs\":[{\"text\":\"";
    if let Some(idx) = window.find(MARK) {
        return read_json_string(&window[idx + MARK.len()..]);
    }
    const TEXT: &str = "\"text\":\"";
    let idx = window.find(TEXT)?;
    read_json_string(&window[idx + TEXT.len()..])
}

fn read_json_string(input: &str) -> Option<String> {
    read_json_string_capped(input, 300)
}

fn read_json_string_capped(input: &str, max_chars: usize) -> Option<String> {
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            let trimmed = out.trim().to_string();
            if trimmed.is_empty() {
                return None;
            }
            return Some(trimmed);
        }
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(decoded) = char::from_u32(code) {
                            out.push(decoded);
                        }
                    }
                }
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(ch);
        }
        if out.len() > max_chars {
            break;
        }
    }
    None
}

fn render_youtube_watch(id: &str, card: &OEmbed) -> String {
    let mut html = String::new();
    html.push_str(&thumb_or(id, &card.thumbnail));
    html.push_str("<h1>");
    html.push_str(&escape_html(&card.title));
    html.push_str("</h1>");
    if !card.author.is_empty() {
        html.push_str("<p>");
        html.push_str(&escape_html(&card.author));
        html.push_str("</p>");
    }
    html
}

fn render_youtube_results(hits: &[YtHit]) -> String {
    if hits.is_empty() {
        return "<p>No video ids were found in the results page.</p>".to_string();
    }
    let mut html = String::from("<ul>");
    for hit in hits {
        let href = format!("https://www.youtube.com/watch?v={}", hit.id);
        html.push_str("<li><a href=\"");
        html.push_str(&href);
        html.push_str("\">");
        if hit.title.is_empty() {
            html.push_str(&escape_html(&hit.id));
        } else {
            html.push_str(&escape_html(&hit.title));
        }
        html.push_str("</a></li>");
    }
    html.push_str("</ul>");
    html
}

fn thumb(id: &str) -> String {
    thumb_or(id, "")
}

fn thumb_or(id: &str, preferred: &str) -> String {
    let src = safe_https_url(preferred).unwrap_or("").to_string();
    let src = if src.is_empty() {
        format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")
    } else {
        src
    };
    format!("<p><img src=\"{src}\" width=\"320\" height=\"180\" alt=\"\"></p>")
}

fn youtube_form() -> &'static str {
    "<form action=\"https://www.youtube.com/results\" method=\"get\"><input type=\"text\" name=\"search_query\"><button type=\"submit\">Search</button></form>"
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AtomEntry {
    pub title: String,
    pub author: String,
    pub summary: String,
    pub href: String,
}

pub(crate) fn parse_atom_feed(xml: &str) -> Vec<AtomEntry> {
    let xml = scan_prefix(xml);
    let lower = xml.to_ascii_lowercase();
    if !lower.contains("<feed") || !lower.contains("<entry") {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = xml;
    while out.len() < MAX_MAIL {
        let lower_rest = rest.to_ascii_lowercase();
        let Some(start) = lower_rest.find("<entry") else {
            break;
        };
        let after = &rest[start + "<entry".len()..];
        let lower_after = after.to_ascii_lowercase();
        let end = lower_after.find("</entry>").unwrap_or(after.len());
        let entry = &after[..end];
        let title = xml_text(entry, "title");
        if !title.is_empty() {
            out.push(AtomEntry {
                title,
                author: xml_text(entry, "name"),
                summary: xml_text(entry, "summary"),
                href: xml_attr(entry, "link", "href").unwrap_or_default(),
            });
        }
        rest = &after[end.min(after.len())..];
    }
    out
}

fn mail_html(target: &str, jar: Option<&Mutex<CookieJar>>, name: &str, feed: &str) -> String {
    let body = match http_text(feed, jar) {
        Ok(xml) => {
            let entries = parse_atom_feed(&xml);
            if entries.is_empty() {
                format!(
                    "<p>No mailbox entries were returned. {name} only lists mail when a session cookie is already stored. This page does not ask for a password or store an API key.</p>"
                )
            } else {
                render_mail(&entries)
            }
        }
        Err(err) => format!(
            "<p>{}</p><p>{name} only lists mail when a session cookie is already stored. This page does not ask for a password or store an API key.</p>",
            escape_html(&err)
        ),
    };
    page(
        name,
        &format!(
            "<p>Opened {}</p>{body}<p class=\"note\">The {name} web app is not run. Mail is not sent from this page.</p>",
            escape_html(target.trim())
        ),
    )
}

fn render_mail(entries: &[AtomEntry]) -> String {
    let mut html = String::from("<ul>");
    for entry in entries {
        html.push_str("<li><strong>");
        html.push_str(&escape_html(&entry.title));
        html.push_str("</strong>");
        if !entry.author.is_empty() {
            html.push_str(" — ");
            html.push_str(&escape_html(&entry.author));
        }
        if !entry.summary.is_empty() {
            html.push_str("<br>");
            html.push_str(&escape_html(&entry.summary));
        }
        if let Some(href) = safe_https_url(&entry.href) {
            html.push_str("<br><a href=\"");
            html.push_str(href);
            html.push_str("\">Open</a>");
        }
        html.push_str("</li>");
    }
    html.push_str("</ul>");
    html
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OpenGraph {
    pub title: String,
    pub description: String,
    pub image: String,
}

pub(crate) fn parse_open_graph(html: &str) -> OpenGraph {
    let html = scan_prefix(html);
    OpenGraph {
        title: meta_content(html, "og:title")
            .or_else(|| html_title(html))
            .unwrap_or_default(),
        description: meta_content(html, "og:description").unwrap_or_default(),
        image: meta_content(html, "og:image").unwrap_or_default(),
    }
}

fn card_html(kind: AppKind, target: &str, jar: Option<&Mutex<CookieJar>>) -> String {
    let name = kind.label();
    let home = kind.home_url();
    let form = format!(
        "<form action=\"{home}\" method=\"get\"><input type=\"text\" name=\"anvil_u\" placeholder=\"https address or path\"><button type=\"submit\">Open</button></form>"
    );
    let fetch_url = query_item_from_url(target, "anvil_u")
        .and_then(|raw| same_app_url(kind, &raw))
        .unwrap_or_else(|| target.trim().to_string());
    let card = if is_app_home(kind, &fetch_url) && query_item_from_url(target, "anvil_u").is_none()
    {
        "<p>Open a public page or post. The home feed is not loaded.</p>".to_string()
    } else {
        match http_text(&fetch_url, jar) {
            Ok(body) => render_card(&parse_open_graph(&body)),
            Err(err) => format!("<p>{}</p>", escape_html(&err)),
        }
    };
    page(
        name,
        &format!(
            "{form}{card}<p class=\"note\">The {name} web app is not run. This page does not ask for a password.</p>"
        ),
    )
}

fn render_card(card: &OpenGraph) -> String {
    if card.title.is_empty() && card.description.is_empty() && card.image.is_empty() {
        return "<p>The response had no title or description.</p>".to_string();
    }
    let mut html = String::new();
    if let Some(src) = safe_https_url(&card.image) {
        html.push_str("<p><img src=\"");
        html.push_str(src);
        html.push_str("\" width=\"320\" height=\"180\" alt=\"\"></p>");
    }
    if !card.title.is_empty() {
        html.push_str("<h1>");
        html.push_str(&escape_html(&card.title));
        html.push_str("</h1>");
    }
    if !card.description.is_empty() {
        html.push_str("<p>");
        html.push_str(&escape_html(&card.description));
        html.push_str("</p>");
    }
    html
}

fn maps_html(target: &str) -> String {
    let query = query_item_from_url(target, "q")
        .or_else(|| query_item_from_url(target, "query"))
        .unwrap_or_default();
    let query_line = if query.is_empty() {
        "<p>No map query was in the address.</p>".to_string()
    } else {
        format!("<p>Map search: {}</p>", escape_html(&query))
    };
    page(
        "Maps",
        &format!(
            "{query_line}<p class=\"note\">Map tiles are not loaded. No maps API key is stored.</p>"
        ),
    )
}

fn is_app_home(kind: AppKind, target: &str) -> bool {
    let Some(parsed) = parse_url(target.trim()).ok() else {
        return false;
    };
    if app_kind(target) != Some(kind) {
        return false;
    }
    let path = parsed.path.trim_matches('/');
    path.is_empty() || path == "home" || path == "login"
}

fn same_app_url(kind: AppKind, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("https://") || raw.starts_with("http://") {
        if app_kind(raw) == Some(kind) {
            return Some(raw.to_string());
        }
        return None;
    }
    if raw.contains("://") || raw.contains(' ') || raw.contains('"') {
        return None;
    }
    let path = raw.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!(
        "{}/{}",
        kind.home_url().trim_end_matches('/'),
        path
    ))
}

impl AppKind {
    fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::Gmail => "Gmail",
            Self::Facebook => "Facebook",
            Self::Instagram => "Instagram",
            Self::X => "X",
            Self::Outlook => "Outlook",
            Self::TikTok => "TikTok",
            Self::LinkedIn => "LinkedIn",
            Self::Maps => "Maps",
        }
    }

    fn home_url(self) -> &'static str {
        match self {
            Self::YouTube => "https://www.youtube.com/",
            Self::Gmail => "https://mail.google.com/",
            Self::Facebook => "https://www.facebook.com/",
            Self::Instagram => "https://www.instagram.com/",
            Self::X => "https://x.com/",
            Self::Outlook => "https://outlook.live.com/",
            Self::TikTok => "https://www.tiktok.com/",
            Self::LinkedIn => "https://www.linkedin.com/",
            Self::Maps => "https://maps.google.com/",
        }
    }
}

fn http_text(url: &str, jar: Option<&Mutex<CookieJar>>) -> Result<String, String> {
    let (status, body) = if let Some(lock) = jar {
        let mut guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let (_final_url, response) =
            fetch_with_cookies(url, &mut guard, None, true).map_err(|err| err.to_string())?;
        (response.status, response.body)
    } else {
        let (_final_url, response) = fetch(url).map_err(|err| err.to_string())?;
        (response.status, response.body)
    };
    if !(200..300).contains(&status) {
        return Err(format!("the server returned {status}"));
    }
    let end = body.len().min(MAX_SCAN);
    Ok(String::from_utf8_lossy(&body[..end]).into_owned())
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title><style>html, body {{ margin: 0; padding: 24px; background-color: #0e1015; color: #eceff4; font-size: 16px; }} h1 {{ font-size: 22px; margin-bottom: 8px; }} p, li {{ margin-bottom: 8px; }} a {{ color: #88c0d0; }} form {{ margin-bottom: 16px; }} .note {{ color: #d8dee9; }}</style></head><body>{}<p><a href=\"anvil://home\">Return to Start</a></p></body></html>",
        escape_html(title),
        body
    )
}

fn scan_prefix(text: &str) -> &str {
    if text.len() <= MAX_SCAN {
        text
    } else {
        let mut end = MAX_SCAN;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
}

fn query_item_from_url(target: &str, key: &str) -> Option<String> {
    let parsed = parse_url(target.trim()).ok()?;
    query_item(parsed.query.as_deref()?, key)
}

fn query_item(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(urldecode_component(v));
        }
    }
    None
}

fn urldecode_component(input: &str) -> String {
    crate::frontend::urldecode_component(input)
}

fn safe_https_url(url: &str) -> Option<&str> {
    let url = url.trim();
    if !url.starts_with("https://") {
        return None;
    }
    if url
        .bytes()
        .any(|b| b.is_ascii_whitespace() || matches!(b, b'"' | b'\'' | b'<' | b'>' | b'`'))
    {
        return None;
    }
    Some(url)
}

fn meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let key_lower = key.to_ascii_lowercase();
    let mut search = 0usize;
    while let Some(rel) = lower[search..].find("<meta") {
        let start = search + rel;
        let tag_end = lower[start..]
            .find('>')
            .map(|i| start + i)
            .unwrap_or(html.len());
        let tag = &html[start..tag_end];
        let tag_lower = tag.to_ascii_lowercase();
        let matches_key = tag_lower.contains(&format!("property=\"{key_lower}\""))
            || tag_lower.contains(&format!("property='{key_lower}'"))
            || tag_lower.contains(&format!("name=\"{key_lower}\""))
            || tag_lower.contains(&format!("name='{key_lower}'"))
            || tag_lower.contains(&format!("property={key_lower}"))
            || tag_lower.contains(&format!("name={key_lower}"));
        if matches_key {
            if let Some(content) = attr_value(tag, "content") {
                let text = decode_entities(&strip_tags(&content));
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }
        }
        search = tag_end.saturating_add(1).min(html.len());
        if search >= html.len() {
            break;
        }
    }
    None
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after = &html[start + "<title".len()..];
    let lower_after = after.to_ascii_lowercase();
    let content_start = lower_after.find('>')? + 1;
    let end = lower_after[content_start..]
        .find("</title>")
        .map(|i| content_start + i)
        .unwrap_or(after.len());
    let text = decode_entities(&strip_tags(&after[content_start..end]));
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn xml_text(entry: &str, tag: &str) -> String {
    let lower = entry.to_ascii_lowercase();
    let open = format!("<{tag}");
    let Some(start) = lower.find(&open) else {
        return String::new();
    };
    let after = &entry[start + open.len()..];
    let lower_after = after.to_ascii_lowercase();
    let Some(content_rel) = lower_after.find('>') else {
        return String::new();
    };
    let close = format!("</{tag}>");
    let content = &after[content_rel + 1..];
    let lower_content = content.to_ascii_lowercase();
    let end = lower_content.find(&close).unwrap_or(content.len());
    decode_entities(&strip_tags(&content[..end]))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn xml_attr(entry: &str, tag: &str, attr: &str) -> Option<String> {
    let lower = entry.to_ascii_lowercase();
    let open = format!("<{tag}");
    let start = lower.find(&open)?;
    let after = &entry[start..];
    let lower_after = after.to_ascii_lowercase();
    let end = lower_after.find('>').unwrap_or(after.len());
    attr_value(&after[..end], attr)
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let idx = lower.find(&key)?;
    let rest = &tag[idx + key.len()..];
    let bytes = rest.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote == b'"' || quote == b'\'' {
        let rest = &rest[1..];
        let end = rest.find(quote as char).unwrap_or(rest.len());
        Some(decode_entities(&rest[..end]))
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(decode_entities(&rest[..end]))
    }
}

fn strip_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn decode_entities(input: &str) -> String {
    let mut out = input.replace("&lt;", "<");
    out = out.replace("&gt;", ">");
    out = out.replace("&quot;", "\"");
    out = out.replace("&#39;", "\'");
    out = out.replace("&apos;", "\'");
    out.replace("&amp;", "&")
}

const PLAYER_SCAN: usize = 2 * 1024 * 1024;
const PLAYER_MAX_WIDTH: u32 = 480;

/// Keep a YouTube document and, when it is a watch or results page, add a player.
///
/// The player points at `anvil-yt-frames` or `anvil-yt-board`. Those are not
/// network URLs. [`youtube_player_clip`] turns them into frames later. Other
/// hosts are returned unchanged. A second call sees `id="anvil-youtube"` and
/// does not add another player.
pub(crate) fn prepare_document(html: &str, url: &str) -> String {
    if app_kind(url) != Some(AppKind::YouTube) || html.contains("id=\"anvil-youtube\"") {
        return html.to_string();
    }
    let scan = player_scan(html);
    let watch_id = youtube_video_id(url);
    let hits = parse_youtube_hits_in(scan);
    if watch_id.is_none() && hits.is_empty() {
        return html.to_string();
    }
    let block = youtube_block(scan, watch_id.as_deref(), &hits);
    insert_after_body(html, &block)
}

/// Build a clip for an `anvil-yt-frames` or `anvil-yt-board` video `src`.
///
/// Frames come from the four public JPEG names on `i.ytimg.com`, or from a
/// storyboard sheet on that same host. A storyboard that does not decode falls
/// back to those JPEGs. Anything else returns `None`. This does not decode H.264.
pub(crate) fn youtube_player_clip(src: &str) -> Option<VideoClip> {
    let src = src.trim();
    if let Some(rest) = src.strip_prefix("anvil-yt-frames://") {
        return public_frames_clip(rest);
    }
    if let Some(rest) = src.strip_prefix("anvil-yt-board://") {
        if let Some(clip) = storyboard_clip(rest) {
            if !clip.frames.is_empty() {
                return Some(clip);
            }
        }
        return public_frames_clip(rest);
    }
    None
}

fn youtube_block(scan: &str, watch_id: Option<&str>, hits: &[YtHit]) -> String {
    let Some(id) = watch_id else {
        return results_block(hits);
    };
    let title = watch_title(scan)
        .or_else(|| {
            hits.iter()
                .find(|hit| hit.id == id)
                .map(|hit| hit.title.clone())
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| id.to_string());
    let author = watch_author(scan).unwrap_or_default();
    let views = watch_views(scan).unwrap_or_default();
    let description = watch_description(scan).unwrap_or_default();
    let seconds = watch_seconds(scan, id);
    let src = player_src(id, scan, seconds);
    let mut html = String::from("<div id=\"anvil-youtube\">");
    html.push_str("<video src=\"");
    html.push_str(&escape_html(&src));
    html.push_str("\" width=\"480\" height=\"270\"></video>");
    html.push_str("<h1>");
    html.push_str(&escape_html(&title));
    html.push_str("</h1>");
    if !author.is_empty() || !views.is_empty() {
        html.push_str("<p>");
        if !author.is_empty() {
            html.push_str(&escape_html(&author));
        }
        if !author.is_empty() && !views.is_empty() {
            html.push_str(" · ");
        }
        if !views.is_empty() {
            html.push_str(&escape_html(&views));
        }
        html.push_str("</p>");
    }
    if !description.is_empty() {
        html.push_str("<p>");
        html.push_str(&escape_html(&description));
        html.push_str("</p>");
    }
    let related: Vec<&YtHit> = hits.iter().filter(|hit| hit.id != id).collect();
    if !related.is_empty() {
        html.push_str(&hits_list(&related));
    }
    html.push_str("</div>");
    html
}

fn results_block(hits: &[YtHit]) -> String {
    let mut html = String::from("<div id=\"anvil-youtube\">");
    html.push_str(&hits_list(&hits.iter().collect::<Vec<_>>()));
    html.push_str("</div>");
    html
}

fn hits_list(hits: &[&YtHit]) -> String {
    let mut html = String::from("<ul>");
    for hit in hits {
        let href = format!("https://www.youtube.com/watch?v={}", hit.id);
        html.push_str("<li><a href=\"");
        html.push_str(&escape_html(&href));
        html.push_str("\">");
        let label = if hit.title.is_empty() {
            hit.id.as_str()
        } else {
            hit.title.as_str()
        };
        html.push_str(&escape_html(label));
        html.push_str("</a></li>");
    }
    html.push_str("</ul>");
    html
}

fn player_src(id: &str, scan: &str, seconds: u32) -> String {
    if let Some(choice) = storyboard_choice(scan) {
        let encoded = urlencode_component(&choice.template);
        if encoded.len() < 1500 {
            return format!(
                "anvil-yt-board://{id}?s={seconds}&lv={}&n={}&cols={}&rows={}&t={encoded}",
                choice.level, choice.count, choice.cols, choice.rows
            );
        }
    }
    format!("anvil-yt-frames://{id}?s={seconds}")
}

struct BoardChoice {
    template: String,
    level: u32,
    count: u32,
    cols: u32,
    rows: u32,
}

fn storyboard_choice(scan: &str) -> Option<BoardChoice> {
    let idx = scan.find("playerStoryboardSpecRenderer")?;
    let end = scan.len().min(idx + 8000);
    let window = &scan[idx..end];
    let spec_at = window.find("https://i.ytimg.com/sb/")?;
    let spec = read_json_string_capped(&window[spec_at..], 2000)?;
    choose_storyboard(&spec)
}

fn choose_storyboard(spec: &str) -> Option<BoardChoice> {
    if !spec.starts_with("https://i.ytimg.com/sb/") || !spec.contains('|') {
        return None;
    }
    let (template, levels) = spec.split_once('|')?;
    let mut best: Option<BoardChoice> = None;
    for (index, part) in levels.split('|').enumerate() {
        let fields: Vec<&str> = part.split('#').collect();
        if fields.len() < 5 {
            continue;
        }
        if fields[0].parse::<u32>().is_err() || fields[1].parse::<u32>().is_err() {
            continue;
        }
        let Ok(count) = fields[2].parse::<u32>() else {
            continue;
        };
        let Ok(cols) = fields[3].parse::<u32>() else {
            continue;
        };
        let Ok(rows) = fields[4].parse::<u32>() else {
            continue;
        };
        if count == 0 || cols == 0 || rows == 0 || cols > 20 || rows > 20 {
            continue;
        }
        let choice = BoardChoice {
            template: template.to_string(),
            level: index as u32,
            count,
            cols,
            rows,
        };
        best = Some(match best {
            None => choice,
            Some(prev) => {
                if storyboard_better(&choice, &prev) {
                    choice
                } else {
                    prev
                }
            }
        });
    }
    best
}

fn storyboard_better(next: &BoardChoice, prev: &BoardChoice) -> bool {
    let next_ok = next.count <= 24;
    let prev_ok = prev.count <= 24;
    match (next_ok, prev_ok) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => next.count > prev.count,
        (false, false) => next.count < prev.count,
    }
}

fn watch_title(scan: &str) -> Option<String> {
    const MARK: &str = "\"videoPrimaryInfoRenderer\":{\"title\":{\"runs\":[{\"text\":\"";
    let idx = scan.find(MARK)?;
    read_json_string(&scan[idx + MARK.len()..])
}

fn watch_author(scan: &str) -> Option<String> {
    const MARK: &str = "\"videoOwnerRenderer\":";
    let idx = scan.find(MARK)?;
    let end = scan.len().min(idx + MARK.len() + 2500);
    let window = &scan[idx..end];
    const TEXT: &str = "\"text\":\"";
    let rel = window.find(TEXT)?;
    read_json_string(&window[rel + TEXT.len()..])
}

fn watch_views(scan: &str) -> Option<String> {
    const MARK: &str = "\"videoViewCountRenderer\":";
    let idx = scan.find(MARK)?;
    let end = scan.len().min(idx + MARK.len() + 600);
    let window = &scan[idx..end];
    const TEXT: &str = "\"simpleText\":\"";
    let rel = window.find(TEXT)?;
    read_json_string(&window[rel + TEXT.len()..])
}

fn watch_description(scan: &str) -> Option<String> {
    const MARK: &str = "\"attributedDescription\":{\"content\":\"";
    let idx = scan.find(MARK)?;
    let text = read_json_string_capped(&scan[idx + MARK.len()..], 240)?;
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

fn watch_seconds(scan: &str, id: &str) -> u32 {
    let mark = format!("\"videoId\":\"{id}\"");
    let Some(idx) = scan.find(&mark) else {
        return 0;
    };
    let end = scan.len().min(idx + 2000);
    number_after(&scan[idx..end], "\"lengthInSeconds\":").unwrap_or(0)
}

fn number_after(window: &str, key: &str) -> Option<u32> {
    let idx = window.find(key)?;
    let rest = window[idx + key.len()..].trim_start();
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    if (1..=86_400).contains(&n) {
        Some(n)
    } else {
        None
    }
}

fn insert_after_body(html: &str, block: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if let Some(idx) = lower.find("<body") {
        if let Some(rel) = lower[idx..].find('>') {
            let at = idx + rel + 1;
            let mut out = String::with_capacity(html.len() + block.len());
            out.push_str(&html[..at]);
            out.push_str(block);
            out.push_str(&html[at..]);
            return out;
        }
    }
    let mut out = String::with_capacity(html.len() + block.len());
    out.push_str(block);
    out.push_str(html);
    out
}

fn player_scan(text: &str) -> &str {
    if text.len() <= PLAYER_SCAN {
        text
    } else {
        let mut end = PLAYER_SCAN;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
}

fn split_player_src(rest: &str) -> Option<(String, &str)> {
    let (id, query) = rest.split_once('?').unwrap_or((rest, ""));
    let id = id.trim().trim_end_matches('/');
    if !is_video_id(id) {
        return None;
    }
    Some((id.to_string(), query))
}

fn query_u32(query: &str, key: &str) -> Option<u32> {
    query_item(query, key)?.parse().ok()
}

fn storyboard_clip(rest: &str) -> Option<VideoClip> {
    let (_id, query) = split_player_src(rest)?;
    let seconds = query_u32(query, "s").unwrap_or(0);
    let level = query_u32(query, "lv")?;
    let count = query_u32(query, "n")?;
    let cols = query_u32(query, "cols")?;
    let rows = query_u32(query, "rows")?;
    let template = query_item(query, "t")?;
    if !template.starts_with("https://i.ytimg.com/sb/") {
        return None;
    }
    if count == 0 || cols == 0 || rows == 0 || cols > 20 || rows > 20 || count > 200 {
        return None;
    }
    let per = cols.saturating_mul(rows).max(1);
    let sheets = ((count + per - 1) / per).min(2);
    let left = count.min(24);
    let mut images = Vec::new();
    for sheet in 0..sheets {
        let url = sheet_url(&template, level, sheet)?;
        let Some(image) = fetch_jpeg(&url, true) else {
            continue;
        };
        for tile in slice_tiles(&image, cols, rows) {
            if images.len() as u32 >= left {
                break;
            }
            images.push(tile);
        }
        if images.len() as u32 >= left {
            break;
        }
    }
    frames_clip(&images, seconds)
}

fn sheet_url(template: &str, level: u32, sheet: u32) -> Option<String> {
    let url = template
        .replace("$L", &level.to_string())
        .replace("$N", &format!("M{sheet}"));
    if storyboard_url_ok(&url) {
        Some(url)
    } else {
        None
    }
}

fn storyboard_url_ok(url: &str) -> bool {
    let Ok(parsed) = parse_url(url) else {
        return false;
    };
    parsed.scheme == "https" && parsed.host == "i.ytimg.com" && parsed.path.starts_with("/sb/")
}

fn public_frames_clip(rest: &str) -> Option<VideoClip> {
    let (id, query) = split_player_src(rest)?;
    let seconds = query_u32(query, "s").unwrap_or(0);
    let mut images = Vec::new();
    for name in ["hqdefault.jpg", "hq1.jpg", "hq2.jpg", "hq3.jpg"] {
        let url = format!("https://i.ytimg.com/vi/{id}/{name}");
        if let Some(image) = fetch_jpeg(&url, false) {
            images.push(image);
        }
    }
    frames_clip(&images, seconds)
}

fn fetch_jpeg(url: &str, storyboard: bool) -> Option<Image> {
    let (final_url, response) = fetch(url).ok()?;
    if !(200..300).contains(&response.status) {
        return None;
    }
    let allowed = if storyboard {
        storyboard_url_ok(&final_url)
    } else {
        public_frame_url_ok(&final_url)
    };
    if !allowed {
        return None;
    }
    decode_auto(&response.body).ok()
}

fn public_frame_url_ok(url: &str) -> bool {
    let Ok(parsed) = parse_url(url) else {
        return false;
    };
    parsed.scheme == "https"
        && (parsed.host == "i.ytimg.com" || parsed.host.ends_with(".ytimg.com"))
        && parsed.path.starts_with("/vi/")
}

fn slice_tiles(image: &Image, cols: u32, rows: u32) -> Vec<Image> {
    if cols == 0 || rows == 0 || image.width < cols || image.height < rows {
        return Vec::new();
    }
    let tile_w = image.width / cols;
    let tile_h = image.height / rows;
    if tile_w == 0 || tile_h == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let mut pixels = Vec::with_capacity((tile_w * tile_h * 4) as usize);
            for y in 0..tile_h {
                for x in 0..tile_w {
                    let sx = col * tile_w + x;
                    let sy = row * tile_h + y;
                    let idx = ((sy * image.width + sx) * 4) as usize;
                    if idx + 4 <= image.pixels.len() {
                        pixels.extend_from_slice(&image.pixels[idx..idx + 4]);
                        let last = pixels.len() - 1;
                        pixels[last] = 255;
                    } else {
                        pixels.extend_from_slice(&[0, 0, 0, 255]);
                    }
                }
            }
            if let Ok(tile) = Image::from_rgba(tile_w, tile_h, pixels) {
                out.push(tile);
            }
        }
    }
    out
}

fn frames_clip(images: &[Image], seconds: u32) -> Option<VideoClip> {
    let images: Vec<&Image> = images
        .iter()
        .filter(|image| image.width >= 1 && image.height >= 1 && !image.pixels.is_empty())
        .collect();
    if images.is_empty() {
        return None;
    }
    let (width, height) = target_size(images[0].width, images[0].height)?;
    let count = images.len() as u32;
    let seconds = if seconds == 0 {
        count
    } else {
        seconds.min(86_400)
    };
    let duration_ms = seconds.saturating_mul(1000).max(33);
    let mut frames = Vec::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let rgba = scale_rgba(image, width, height);
        let pts = if index + 1 == images.len() {
            duration_ms
        } else {
            (index as u32).saturating_mul(duration_ms) / count
        };
        let frame = VideoFrame::new(width, height, pts, index == 0, rgba).ok()?;
        frames.push(frame);
    }
    Some(VideoClip {
        width,
        height,
        fps: 1,
        backend: probe_platform_gpu_backend(false),
        profile: VideoCodecProfile::AnvilNv12Temporal,
        frames,
        audio: None,
    })
}

fn target_size(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut w = width;
    let mut h = height;
    if w > PLAYER_MAX_WIDTH {
        h = ((u64::from(h) * u64::from(PLAYER_MAX_WIDTH)) / u64::from(w)).max(1) as u32;
        w = PLAYER_MAX_WIDTH;
    }
    Some((even_side(w), even_side(h)))
}

fn even_side(side: u32) -> u32 {
    let side = if side % 2 == 1 {
        side.saturating_sub(1)
    } else {
        side
    };
    side.max(2)
}

fn scale_rgba(image: &Image, width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; (width as usize) * (height as usize) * 4];
    if image.width == 0 || image.height == 0 || width == 0 || height == 0 {
        return out;
    }
    for y in 0..height {
        let sy = ((u64::from(y) * u64::from(image.height)) / u64::from(height))
            .min(u64::from(image.height - 1)) as u32;
        for x in 0..width {
            let sx = ((u64::from(x) * u64::from(image.width)) / u64::from(width))
                .min(u64::from(image.width - 1)) as u32;
            let src = ((sy * image.width + sx) * 4) as usize;
            let dst = ((y * width + x) * 4) as usize;
            if src + 3 < image.pixels.len() && dst + 3 < out.len() {
                out[dst] = image.pixels[src];
                out[dst + 1] = image.pixels[src + 1];
                out[dst + 2] = image.pixels[src + 2];
                out[dst + 3] = 255;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reddit_rewrites_to_old_and_does_not_loop() {
        assert_eq!(
            passthrough_rewrite("https://www.reddit.com/r/rust?sort=hot"),
            Some("https://old.reddit.com/r/rust?sort=hot".to_string())
        );
        assert_eq!(passthrough_rewrite("https://old.reddit.com/r/rust"), None);
        assert_eq!(passthrough_rewrite("https://www.youtube.com/"), None);
    }

    #[test]
    fn recognizes_apps_and_leaves_other_hosts_alone() {
        assert_eq!(
            app_kind("https://youtu.be/dQw4w9WgXcQ"),
            Some(AppKind::YouTube)
        );
        assert_eq!(
            app_kind("https://mail.google.com/mail/u/0/"),
            Some(AppKind::Gmail)
        );
        assert_eq!(
            app_kind("https://www.facebook.com/zuck"),
            Some(AppKind::Facebook)
        );
        assert_eq!(app_kind("https://x.com/home"), Some(AppKind::X));
        assert_eq!(
            app_kind("https://www.google.com/maps?q=cafe"),
            Some(AppKind::Maps)
        );
        assert_eq!(app_kind("https://www.google.com/search?q=cafe"), None);
        assert_eq!(app_kind("https://example.com/"), None);
        assert!(!fetch_page_scripts("https://www.youtube.com/"));
        assert!(!fetch_page_scripts("https://mail.google.com/mail/u/0/"));
        assert!(!fetch_page_scripts("https://www.facebook.com/"));
        assert!(fetch_page_scripts("https://example.com/"));
    }

    #[test]
    fn reads_youtube_ids() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=4"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            youtube_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            youtube_video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=short"),
            None
        );
    }

    #[test]
    fn parses_oembed_results_atom_and_open_graph() {
        let card = parse_oembed(
            r#"{"title":"A & B","author_name":"Ada","thumbnail_url":"https://i.ytimg.com/vi/x/hqdefault.jpg"}"#,
        )
        .unwrap();
        assert_eq!(card.title, "A & B");
        assert_eq!(card.author, "Ada");
        let hits = parse_youtube_results(
            r#"{"videoId":"dQw4w9WgXcQ","title":{"runs":[{"text":"Song \"one\""}]},"videoId":"dQw4w9WgXcQ","videoId":"abcdefghijk","title":{"runs":[{"text":"Second"}]}}"#,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Song \"one\"");
        let mail = parse_atom_feed(
            r#"<feed><entry><title>Hello &amp; co</title><author><name>Ada</name></author><summary>Line</summary><link href="https://mail.google.com/mail/" /></entry></feed>"#,
        );
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].title, "Hello & co");
        assert_eq!(mail[0].href, "https://mail.google.com/mail/");
        assert!(parse_atom_feed("<html>login</html>").is_empty());
        let og = parse_open_graph(
            r#"<meta property="og:title" content="Post &amp; pic"><meta property="og:description" content="Hi"><meta property="og:image" content="https://example.com/a.jpg">"#,
        );
        assert_eq!(og.title, "Post & pic");
        assert_eq!(og.image, "https://example.com/a.jpg");
    }

    #[test]
    fn rendered_pages_escape_text_and_store_no_secret() {
        let html = render_youtube_watch(
            "dQw4w9WgXcQ",
            &OEmbed {
                title: "<script>".to_string(),
                author: "Ada".to_string(),
                thumbnail: "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg".to_string(),
            },
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        let src = include_str!("sites.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        let lower = code.to_ascii_lowercase();
        let client_secret = ["client", "_secret"].concat();
        let api_key = ["api", "_key"].concat();
        let bearer = ["bearer", " "].concat();
        assert!(!lower.contains(&client_secret));
        assert!(!lower.contains(&api_key));
        assert!(!lower.contains(&bearer));
        let google_key_prefix = ["AI", "za"].concat();
        assert!(!src.contains(&google_key_prefix));
    }

    #[test]
    fn maps_page_quotes_the_query_and_rejects_a_bad_image() {
        let html = maps_html("https://www.google.com/maps?q=cafe+%26+tea");
        assert!(html.contains("cafe &amp; tea"));
        assert!(safe_https_url("javascript:alert(1)").is_none());
        assert!(safe_https_url("https://example.com/a.jpg").is_some());
    }

    #[test]
    fn youtube_watch_inserts_a_frame_player_and_escapes_text() {
        let spec = "https://i.ytimg.com/sb/jNQXAC9IVRw/storyboard3_L$L/$N.jpg?sqp=abc|160#90#4#2#2#0#default";
        let mut html = String::from("<html><body>");
        html.push_str(r#""videoPrimaryInfoRenderer":{"title":{"runs":[{"text":"Me & zoo"}]}}"#);
        html.push_str(r#","videoOwnerRenderer":{"title":{"text":"jawed"}}"#);
        html.push_str(r#","videoViewCountRenderer":{"simpleText":"446 views"}"#);
        html.push_str(r#","videoId":"jNQXAC9IVRw","lengthInSeconds":"19""#);
        html.push_str(r#","title":{"runs":[{"text":"Me & zoo"}]}"#);
        html.push_str(
            r#","videoId":"abcdefghijk","title":{"runs":[{"text":"\u003cscript\u003e"}]}"#,
        );
        html.push_str(r#","playerStoryboardSpecRenderer":{"spec":""#);
        html.push_str(spec);
        html.push_str(r#""}</body></html>"#);
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        let out = prepare_document(&html, url);
        assert!(out.contains("id=\"anvil-youtube\""));
        assert!(out.contains("anvil-yt-board://jNQXAC9IVRw"));
        assert!(out.contains("lv=0"));
        let amp = format!("{}amp;", '&');
        assert!(out.contains(&format!("Me {amp} zoo")));
        assert!(out.contains("jawed"));
        assert!(out.contains("https://www.youtube.com/watch?v=abcdefghijk"));
        let lt = format!("{}lt;", '&');
        let gt = format!("{}gt;", '&');
        assert!(out.contains(&format!("{lt}script{gt}")));
        assert!(!out.contains("<script"));
        let again = prepare_document(&out, url);
        assert_eq!(again, out);
    }

    #[test]
    fn frames_clip_uses_the_given_seconds_and_even_size() {
        let pixels = vec![255, 0, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 1, 2, 3, 10];
        let first = Image::from_rgba(2, 2, pixels.clone()).unwrap();
        let second = Image::from_rgba(2, 2, pixels).unwrap();
        let clip = frames_clip(&[first, second], 4).unwrap();
        assert_eq!(clip.frames.len(), 2);
        assert_eq!(clip.frames.last().unwrap().pts_ms, 4000);
        assert_eq!(clip.width % 2, 0);
        assert_eq!(clip.height % 2, 0);
        assert_eq!(clip.frames[0].rgba[3], 255);
    }

    #[test]
    fn other_hosts_stay_put_and_storyboards_stay_on_ytimg() {
        let plain = "<html><body>Hi</body></html>";
        assert_eq!(prepare_document(plain, "https://example.com/"), plain);
        assert_eq!(prepare_document(plain, "https://www.youtube.com/"), plain);
        assert!(choose_storyboard("https://evil.example/sb/x|160#90#4#2#2").is_none());
        assert!(choose_storyboard("https://i.ytimg.com.evil.example/sb/x|160#90#4#2#2").is_none());
        let src = player_src("jNQXAC9IVRw", "no storyboard here", 0);
        assert_eq!(src, "anvil-yt-frames://jNQXAC9IVRw?s=0");
        assert!(!src.contains("evil"));
    }
}
