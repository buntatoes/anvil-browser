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
//!
//! No API key, client secret, or password is stored in this file.

use std::sync::Mutex;

use anvil_json::parse_json;
use anvil_net::{CookieJar, fetch, fetch_with_cookies, parse_url};

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
    let html = scan_prefix(html);
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
        if out.len() > 300 {
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
}
