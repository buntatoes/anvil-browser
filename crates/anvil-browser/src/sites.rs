// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Which large sites the engine loads itself, and which scripts it must not fetch.
//!
//! YouTube, Gmail, Facebook, Instagram, X, Outlook, TikTok, LinkedIn, Google
//! Maps, OpenStreetMap, Reddit, other social hosts, and bank and payment hosts
//! are recognized by host. Their documents load through the normal HTTP path,
//! except map addresses. External scripts are not fetched: those bundles are
//! not the subset interpreter, and downloading them keeps the tab worker from
//! painting the document. Inline scripts still run when they fit
//! [`anvil_script::script_runs_here`].
//!
//! Reddit is not rewritten. `www.reddit.com` stays `www.reddit.com`. Requests
//! to a Reddit host use a longer `User-Agent` that names this browser.
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
const MAP_COLS: u32 = 3;
const MAP_ROWS: u32 = 3;
const MAP_TILE: u32 = 256;

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
    Reddit,
    Social,
    Bank,
}

/// Load the URL the caller asked for.
///
/// Reddit is not sent to `old.reddit.com`. `www.reddit.com` stays on
/// `www.reddit.com`. This returns `None`.
pub fn passthrough_rewrite(_target: &str) -> Option<String> {
    None
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
        | AppKind::LinkedIn
        | AppKind::Reddit
        | AppKind::Social
        | AppKind::Bank => card_html(kind, target, jar),
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
    if host_is(host, "reddit.com") || host_is(host, "redd.it") {
        return Some(AppKind::Reddit);
    }
    if host_in(host, SOCIAL_ROOTS) {
        return Some(AppKind::Social);
    }
    if host_in(host, BANK_ROOTS) {
        return Some(AppKind::Bank);
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

fn host_is(host: &str, root: &str) -> bool {
    host == root
        || host
            .strip_suffix(root)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn host_in(host: &str, roots: &[&str]) -> bool {
    roots.iter().any(|root| host_is(host, root))
}

const SOCIAL_ROOTS: &[&str] = &[
    "snapchat.com",
    "pinterest.com",
    "pin.it",
    "tumblr.com",
    "discord.com",
    "discordapp.com",
    "twitch.tv",
    "threads.net",
    "bsky.app",
    "whatsapp.com",
    "telegram.org",
    "t.me",
    "signal.org",
    "nextdoor.com",
    "quora.com",
    "medium.com",
    "vk.com",
    "weibo.com",
    "ok.ru",
    "line.me",
    "messenger.com",
    "slack.com",
    "mastodon.social",
    "bere.al",
];

const BANK_ROOTS: &[&str] = &[
    "chase.com",
    "bankofamerica.com",
    "wellsfargo.com",
    "citi.com",
    "citibank.com",
    "capitalone.com",
    "usbank.com",
    "pnc.com",
    "td.com",
    "tdbank.com",
    "ally.com",
    "discover.com",
    "americanexpress.com",
    "paypal.com",
    "venmo.com",
    "schwab.com",
    "fidelity.com",
    "vanguard.com",
    "usaa.com",
    "navyfederal.org",
    "regions.com",
    "fifththird.com",
    "huntington.com",
    "key.com",
    "truist.com",
    "citizensbank.com",
    "santanderbank.com",
    "sofi.com",
    "chime.com",
    "synchrony.com",
    "synchronybank.com",
    "barclays.com",
    "hsbc.com",
    "bmo.com",
    "rbc.com",
    "scotiabank.com",
    "cibc.com",
    "robinhood.com",
    "etrade.com",
    "morganstanley.com",
    "ml.com",
    "wealthfront.com",
    "betterment.com",
    "cash.app",
    "zellepay.com",
    "commercebank.com",
    "umb.com",
];

pub(crate) fn youtube_video_id(target: &str) -> Option<String> {
    let parsed = parse_url(target.trim()).ok()?;
    if !is_youtube_host(&parsed.host) {
        return None;
    }
    if let Some(query) = &parsed.query
        && let Some(id) = query_item(query, "v")
        && is_video_id(&id)
    {
        return Some(id);
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
    if let Ok(parsed) = parse_url(target.trim())
        && (parsed.path == "/results" || parsed.path.starts_with("/results/"))
    {
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
                    if let Ok(code) = u32::from_str_radix(&hex, 16)
                        && let Some(decoded) = char::from_u32(code)
                    {
                        out.push(decoded);
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
    map_page_html(&map_view_from_url(target))
}

/// A map address is drawn here. Google Maps, OpenStreetMap, and ArcGIS are not fetched.
///
/// The page is one bitmap of nine tiles, plus links for pan, zoom, and the
/// basemap. OpenStreetMap is the default. ArcGIS imagery, streets, and topo are
/// optional. A `webmap=` item uses that item's tiled basemap and one feature
/// layer. Tiles already fetched are reused. A token on the address is sent to
/// ArcGIS and is not saved. A place name is looked up once. No maps API key is
/// stored. Anything that is not a map address returns `None`.
pub(crate) fn gis_map_page(url: &str) -> Option<String> {
    if !is_gis_map(url) {
        return None;
    }
    Some(map_page_html(&resolve_map_view(url)))
}

/// Bitmap for an `anvil-map://` image. Nine tiles, fetched together.
///
/// `anvil-map://zoom/lat/lon` is OpenStreetMap. `imagery`, `streets`, and `topo`
/// are ArcGIS. `webmap/{id}/zoom/lat/lon` is a saved ArcGIS web map. Tiles already
/// fetched stay in memory for the next pan. A token is not part of this address.
pub(crate) fn map_bitmap(src: &str) -> Option<Image> {
    let rest = src.trim().strip_prefix("anvil-map://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut parts = path.split('/');
    let first = parts.next()?;
    let (layer, service, features, zoom, lat, lon) = if first == "webmap" {
        let id = clean_item_id(parts.next()?)?;
        let zoom: u32 = parts.next()?.parse().ok()?;
        let lat: f64 = parts.next()?.parse().ok()?;
        let lon: f64 = parts.next()?.parse().ok()?;
        let info = load_webmap(&id)?;
        (
            info.layer.unwrap_or(MapLayer::Streets),
            info.service,
            info.features,
            zoom,
            lat,
            lon,
        )
    } else if let Ok(zoom) = first.parse::<u32>() {
        let lat: f64 = parts.next()?.parse().ok()?;
        let lon: f64 = parts.next()?.parse().ok()?;
        (MapLayer::Osm, None, feature_query(query), zoom, lat, lon)
    } else {
        let layer = MapLayer::from_name(first)?;
        let zoom: u32 = parts.next()?.parse().ok()?;
        let lat: f64 = parts.next()?.parse().ok()?;
        let lon: f64 = parts.next()?.parse().ok()?;
        (layer, None, feature_query(query), zoom, lat, lon)
    };
    if !(1..=16).contains(&zoom) || !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    if !(-85.0..=85.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(render_map(
        layer,
        service.as_deref(),
        features.as_deref(),
        lat,
        lon,
        zoom,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapLayer {
    Osm,
    Imagery,
    Streets,
    Topo,
}

impl MapLayer {
    fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "osm" | "openstreetmap" => Some(Self::Osm),
            "arcgis" | "imagery" => Some(Self::Imagery),
            "streets" | "street" => Some(Self::Streets),
            "topo" => Some(Self::Topo),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Osm => "osm",
            Self::Imagery => "imagery",
            Self::Streets => "streets",
            Self::Topo => "topo",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Osm => "OpenStreetMap",
            Self::Imagery => "ArcGIS imagery",
            Self::Streets => "ArcGIS streets",
            Self::Topo => "ArcGIS topo",
        }
    }

    fn tile_url(self, zoom: u32, x: i32, y: i32) -> String {
        match self {
            Self::Osm => format!("https://tile.openstreetmap.org/{zoom}/{x}/{y}.png"),
            Self::Imagery => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{zoom}/{y}/{x}"
            ),
            Self::Streets => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{zoom}/{y}/{x}"
            ),
            Self::Topo => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{zoom}/{y}/{x}"
            ),
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Osm => {
                "Map tiles are loaded from tile.openstreetmap.org. No maps API key is stored."
            }
            Self::Imagery | Self::Streets | Self::Topo => {
                "ArcGIS tiles are loaded from server.arcgisonline.com. No maps API key is stored."
            }
        }
    }

    fn credit(self) -> &'static str {
        match self {
            Self::Osm => "Map data © OpenStreetMap contributors.",
            Self::Imagery => "Imagery tiles © Esri, Maxar, Earthstar Geographics.",
            Self::Streets => "Street tiles © Esri.",
            Self::Topo => "Topo tiles © Esri.",
        }
    }
}

fn is_gis_map(url: &str) -> bool {
    let url = url.trim();
    if url == "anvil://maps" || url.starts_with("anvil://maps?") {
        return true;
    }
    let Ok(parsed) = parse_url(url) else {
        return false;
    };
    if parsed.host == "maps.google.com" {
        return true;
    }
    if (parsed.host == "google.com" || parsed.host == "www.google.com")
        && parsed.path.starts_with("/maps")
    {
        return true;
    }
    parsed.host == "openstreetmap.org"
        || parsed.host == "www.openstreetmap.org"
        || is_arcgis_host(&parsed.host)
}

fn is_arcgis_host(host: &str) -> bool {
    host == "arcgis.com"
        || host.ends_with(".arcgis.com")
        || host == "arcgisonline.com"
        || host.ends_with(".arcgisonline.com")
}

struct MapView {
    lat: Option<f64>,
    lon: Option<f64>,
    zoom: Option<u32>,
    query: Option<String>,
    layer: MapLayer,
    layer_explicit: bool,
    item: Option<String>,
    features: Option<String>,
    token: Option<String>,
}

fn map_view_from_url(url: &str) -> MapView {
    let mut view = MapView {
        lat: None,
        lon: None,
        zoom: None,
        query: None,
        layer: MapLayer::Osm,
        layer_explicit: false,
        item: None,
        features: None,
        token: None,
    };
    if let Some((lat, lon, zoom)) = coords_at(url).or_else(|| coords_osm(url)) {
        view.lat = Some(lat);
        view.lon = Some(lon);
        view.zoom = Some(zoom);
    }
    if url.trim().starts_with("anvil://maps") {
        let query = url.trim().split_once('?').map(|(_, q)| q).unwrap_or("");
        if let (Some(lat), Some(lon)) = (
            query_item(query, "lat").and_then(|v| v.parse().ok()),
            query_item(query, "lon").and_then(|v| v.parse().ok()),
        ) {
            view.lat = Some(lat);
            view.lon = Some(lon);
        }
        if let Some(zoom) = query_item(query, "z").and_then(|v| v.parse().ok()) {
            view.zoom = Some(zoom);
        }
        if let Some(q) = query_item(query, "q").filter(|q| !q.is_empty()) {
            view.query = Some(q);
        }
        if let Some(layer) = query_item(query, "layer").and_then(|name| MapLayer::from_name(&name))
        {
            view.layer = layer;
            view.layer_explicit = true;
        }
        view.item = query_item(query, "webmap").and_then(|id| clean_item_id(&id));
        view.features = query_item(query, "features").and_then(|url| clean_feature_url(&url));
        view.token = query_item(query, "token").and_then(|token| clean_token(&token));
    }
    if view.query.is_none() {
        view.query = query_item_from_url(url, "q")
            .or_else(|| query_item_from_url(url, "query"))
            .filter(|q| !q.is_empty());
    }
    if view.lat.is_none()
        && let Some(ll) =
            query_item_from_url(url, "ll").or_else(|| query_item_from_url(url, "center"))
        && let Some((lat, lon)) = pair_coords(&ll)
    {
        view.lat = Some(lat);
        view.lon = Some(lon);
    }
    if view.zoom.is_none() {
        view.zoom = query_item_from_url(url, "z").and_then(|v| v.parse().ok());
    }
    if view.lat.is_none()
        && let Some((lat, lon, zoom)) = coords_arcgis(url)
    {
        view.lat = Some(lat);
        view.lon = Some(lon);
        view.zoom = Some(zoom);
    }
    if view.item.is_none() {
        view.item = query_item_from_url(url, "webmap").and_then(|id| clean_item_id(&id));
    }
    if view.features.is_none() {
        view.features =
            query_item_from_url(url, "features").and_then(|url| clean_feature_url(&url));
    }
    if view.token.is_none() {
        view.token = query_item_from_url(url, "token").and_then(|token| clean_token(&token));
    }
    if !view.layer_explicit
        && let Some(layer) =
            query_item_from_url(url, "layer").and_then(|name| MapLayer::from_name(&name))
    {
        view.layer = layer;
        view.layer_explicit = true;
    }
    if view.lat.is_none()
        && let Some(name) = place_from_path(url)
    {
        view.query = Some(name);
    }
    if !view.layer_explicit {
        view.layer = layer_from_url(url);
    }
    view
}

fn resolve_map_view(url: &str) -> MapView {
    let mut view = map_view_from_url(url);
    if let Some(token) = view.token.clone() {
        remember_map_token(&token);
    }
    if let Some(id) = view.item.clone()
        && let Some(info) = load_webmap(&id)
    {
        if !view.layer_explicit
            && let Some(layer) = info.layer
        {
            view.layer = layer;
        }
        if view.features.is_none() {
            view.features = info.features;
        }
    }
    if view.lat.is_none()
        && let Some(query) = view.query.clone()
    {
        if let Some((lat, lon)) = pair_coords(&query) {
            view.lat = Some(lat);
            view.lon = Some(lon);
        } else if let Some((lat, lon, name)) = geocode_place(&query) {
            view.lat = Some(lat);
            view.lon = Some(lon);
            view.query = Some(name);
            if view.zoom.is_none() {
                view.zoom = Some(14);
            }
        }
    }
    view
}

fn map_page_html(view: &MapView) -> String {
    let shown = view.query.clone().unwrap_or_default();
    let hidden = hidden_map_fields(view);
    let form = format!(
        "<form action=\"anvil://maps\" method=\"get\"><input type=\"hidden\" name=\"layer\" value=\"{}\">{hidden}<input type=\"text\" name=\"q\" value=\"{}\"><button type=\"submit\">Search</button></form>",
        view.layer.id(),
        escape_html(&shown)
    );
    let (body, note) = if let Some((lat, lon, zoom)) = placed(view) {
        (
            format!(
                "{}{}",
                map_controls(view, lat, lon, zoom),
                map_image(view, lat, lon, zoom)
            ),
            map_note(view),
        )
    } else if view.query.is_some() {
        (
            String::new(),
            format!(
                "That place was not found. No maps API key is stored. {}",
                view.layer.credit()
            ),
        )
    } else {
        (
            format!(
                "{}{}",
                layer_links(view, 0.0, 0.0, 2),
                map_image(view, 0.0, 0.0, 2)
            ),
            map_note(view),
        )
    };
    page("Maps", &format!("{form}{body}<p class=\"note\">{note}</p>"))
}

fn hidden_map_fields(view: &MapView) -> String {
    let mut html = String::new();
    if let Some(id) = &view.item {
        html.push_str("<input type=\"hidden\" name=\"webmap\" value=\"");
        html.push_str(&escape_html(id));
        html.push_str("\">");
    }
    if let Some(features) = &view.features {
        html.push_str("<input type=\"hidden\" name=\"features\" value=\"");
        html.push_str(&escape_html(features));
        html.push_str("\">");
    }
    html
}

fn map_note(view: &MapView) -> String {
    let mut note = view.layer.note().to_string();
    if view.item.is_some() {
        note.push_str(" A saved web map is drawn from its tiled basemap.");
    }
    if view.features.is_some() {
        note.push_str(" One feature layer is drawn on top. At most 80 features.");
    }
    if view.token.is_some() {
        note.push_str(" A token on this address is sent to ArcGIS and is not saved.");
    }
    note.push(' ');
    note.push_str(view.layer.credit());
    note
}

fn map_image(view: &MapView, lat: f64, lon: f64, zoom: u32) -> String {
    let src = map_src(view, lat, lon, zoom);
    format!(
        "<p><img src=\"{src}\" width=\"768\" height=\"768\" alt=\"Map\"></p><p>{lat:.5}, {lon:.5} · zoom {zoom} · {}</p>",
        view.layer.label()
    )
}

fn map_src(view: &MapView, lat: f64, lon: f64, zoom: u32) -> String {
    let mut src = if let Some(id) = &view.item {
        format!("anvil-map://webmap/{id}/{zoom}/{lat:.5}/{lon:.5}")
    } else {
        match view.layer {
            MapLayer::Osm => format!("anvil-map://{zoom}/{lat:.5}/{lon:.5}"),
            MapLayer::Imagery | MapLayer::Streets | MapLayer::Topo => {
                format!("anvil-map://{}/{zoom}/{lat:.5}/{lon:.5}", view.layer.id())
            }
        }
    };
    if view.item.is_none()
        && let Some(features) = &view.features
    {
        src.push_str("?f=");
        src.push_str(&urlencode_component(features));
    }
    src
}

fn map_controls(view: &MapView, lat: f64, lon: f64, zoom: u32) -> String {
    let north = shift(lat, lon, zoom, 0.0, -(MAP_ROWS as f64) * 0.7);
    let south = shift(lat, lon, zoom, 0.0, (MAP_ROWS as f64) * 0.7);
    let west = shift(lat, lon, zoom, -(MAP_COLS as f64) * 0.7, 0.0);
    let east = shift(lat, lon, zoom, (MAP_COLS as f64) * 0.7, 0.0);
    format!(
        "<p><a href=\"{}\">Zoom in</a> · <a href=\"{}\">Zoom out</a> · <a href=\"{}\">North</a> · <a href=\"{}\">South</a> · <a href=\"{}\">West</a> · <a href=\"{}\">East</a></p>{}",
        escape_html(&map_href(
            view,
            lat,
            lon,
            zoom.saturating_add(1).min(16),
            view.layer
        )),
        escape_html(&map_href(
            view,
            lat,
            lon,
            zoom.saturating_sub(1).max(1),
            view.layer
        )),
        escape_html(&map_href(view, north.0, north.1, zoom, view.layer)),
        escape_html(&map_href(view, south.0, south.1, zoom, view.layer)),
        escape_html(&map_href(view, west.0, west.1, zoom, view.layer)),
        escape_html(&map_href(view, east.0, east.1, zoom, view.layer)),
        layer_links(view, lat, lon, zoom),
    )
}

fn layer_links(view: &MapView, lat: f64, lon: f64, zoom: u32) -> String {
    let mut html = String::from("<p>");
    for (index, layer) in [
        MapLayer::Osm,
        MapLayer::Imagery,
        MapLayer::Streets,
        MapLayer::Topo,
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            html.push_str(" · ");
        }
        if layer == view.layer {
            html.push_str(layer.label());
        } else {
            html.push_str("<a href=\"");
            html.push_str(&escape_html(&map_href(view, lat, lon, zoom, layer)));
            html.push_str("\">");
            html.push_str(layer.label());
            html.push_str("</a>");
        }
    }
    html.push_str("</p>");
    html
}

fn placed(view: &MapView) -> Option<(f64, f64, u32)> {
    let lat = view.lat?;
    let lon = view.lon?;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    let zoom = view.zoom.unwrap_or(14).clamp(1, 16);
    Some((lat.clamp(-85.0, 85.0), wrap_lon(lon), zoom))
}

fn map_href(view: &MapView, lat: f64, lon: f64, zoom: u32, layer: MapLayer) -> String {
    if layer == MapLayer::Osm && view.item.is_none() && view.features.is_none() {
        return format!(
            "https://www.google.com/maps/@{:.5},{:.5},{}z",
            lat, lon, zoom
        );
    }
    let mut url = format!(
        "anvil://maps?lat={lat:.5}&lon={lon:.5}&z={zoom}&layer={}",
        layer.id()
    );
    if let Some(id) = &view.item {
        url.push_str("&webmap=");
        url.push_str(id);
    }
    if let Some(features) = &view.features {
        url.push_str("&features=");
        url.push_str(&urlencode_component(features));
    }
    url
}

fn coords_arcgis(url: &str) -> Option<(f64, f64, u32)> {
    let parsed = parse_url(url).ok()?;
    if !is_arcgis_host(&parsed.host) {
        return None;
    }
    let center = query_item(parsed.query.as_deref()?, "center")?;
    let (lon_text, lat_text) = center.split_once(',')?;
    let lon: f64 = lon_text.trim().parse().ok()?;
    let lat: f64 = lat_text.trim().parse().ok()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let zoom = query_item(parsed.query.as_deref()?, "level")
        .or_else(|| query_item(parsed.query.as_deref()?, "z"))
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value.round() as u32)
        .unwrap_or(12);
    Some((lat, lon, zoom.clamp(1, 16)))
}

fn layer_from_url(url: &str) -> MapLayer {
    if let Some(name) = query_item_from_url(url, "layer")
        && let Some(layer) = MapLayer::from_name(&name)
    {
        return layer;
    }
    let lower = url.to_ascii_lowercase();
    if lower.contains("world_imagery") {
        return MapLayer::Imagery;
    }
    if lower.contains("world_topo") {
        return MapLayer::Topo;
    }
    if lower.contains("world_street") {
        return MapLayer::Streets;
    }
    if parse_url(url)
        .ok()
        .is_some_and(|parsed| is_arcgis_host(&parsed.host))
    {
        return MapLayer::Streets;
    }
    MapLayer::Osm
}

fn coords_at(url: &str) -> Option<(f64, f64, u32)> {
    let at = url.split('@').nth(1)?;
    let at = at.split(['/', '?', '#', '&']).next().unwrap_or(at);
    let mut parts = at.split(',');
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    let zraw = parts.next().unwrap_or("14z");
    let zraw = zraw.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let zoom = zraw.parse::<f64>().ok()?.round() as u32;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    Some((lat, lon, zoom.clamp(1, 16)))
}

fn coords_osm(url: &str) -> Option<(f64, f64, u32)> {
    let frag = url.split_once('#')?.1;
    let map = frag.split("map=").nth(1)?;
    let map = map.split(['&', ' ']).next().unwrap_or(map);
    let mut parts = map.split('/');
    let zoom = parts.next()?.parse::<f64>().ok()?.round() as u32;
    let lat: f64 = parts.next()?.parse().ok()?;
    let lon: f64 = parts.next()?.parse().ok()?;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    Some((lat, lon, zoom.clamp(1, 16)))
}

fn pair_coords(text: &str) -> Option<(f64, f64)> {
    let (lat, lon) = text.split_once(',')?;
    let lat: f64 = lat.trim().parse().ok()?;
    let lon: f64 = lon.trim().parse().ok()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

fn place_from_path(url: &str) -> Option<String> {
    let path = parse_url(url).ok()?.path;
    for marker in ["/maps/place/", "/maps/search/"] {
        let Some(rest) = path.split(marker).nth(1) else {
            continue;
        };
        let seg = rest.split('/').next().unwrap_or("");
        if seg.is_empty() || seg.contains('@') {
            continue;
        }
        let name = urldecode_component(seg);
        if !name.trim().is_empty() {
            return Some(name);
        }
    }
    None
}

fn geocode_place(query: &str) -> Option<(f64, f64, String)> {
    let query = query.trim();
    if query.is_empty() || query.len() > 200 {
        return None;
    }
    let url = format!(
        "https://nominatim.openstreetmap.org/search?format=jsonv2&limit=1&q={}",
        urlencode_component(query)
    );
    let (_final, response) = fetch(&url).ok()?;
    if !(200..300).contains(&response.status) {
        return None;
    }
    let text = String::from_utf8_lossy(&response.body);
    let value = parse_json(&text).ok()?;
    let first = value.as_array()?.first()?;
    let lat = first.get_str("lat")?.parse().ok()?;
    let lon = first.get_str("lon")?.parse().ok()?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let name = first
        .get_str("display_name")
        .filter(|name| !name.is_empty())
        .unwrap_or(query)
        .to_string();
    Some((lat, lon, name))
}

fn render_map(
    layer: MapLayer,
    service: Option<&str>,
    features: Option<&str>,
    lat: f64,
    lon: f64,
    zoom: u32,
) -> Image {
    let (x, y) = mercator_xy(lat, lon, zoom);
    let origin_x = (x - MAP_COLS as f64 / 2.0).floor() as i32;
    let origin_y = (y - MAP_ROWS as f64 / 2.0).floor() as i32;
    let tiles = fetch_tiles(layer, service, origin_x, origin_y, zoom);
    let mut canvas = paint_map(&tiles);
    if let Some(url) = features {
        draw_feature_layer(&mut canvas, origin_x, origin_y, zoom, url);
    }
    canvas
}

fn fetch_tiles(
    layer: MapLayer,
    service: Option<&str>,
    origin_x: i32,
    origin_y: i32,
    zoom: u32,
) -> Vec<Option<Image>> {
    let n = 1i32 << zoom;
    let service = service.map(str::to_string);
    let mut slots = Vec::with_capacity((MAP_COLS * MAP_ROWS) as usize);
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for row in 0..MAP_ROWS {
            for col in 0..MAP_COLS {
                let x = origin_x.wrapping_add(col as i32).rem_euclid(n);
                let y = origin_y + row as i32;
                let service = service.clone();
                jobs.push(
                    scope.spawn(move || fetch_one_tile(layer, service.as_deref(), zoom, x, y, n)),
                );
            }
        }
        for job in jobs {
            slots.push(job.join().ok().flatten());
        }
    });
    slots
}

fn fetch_one_tile(
    layer: MapLayer,
    service: Option<&str>,
    zoom: u32,
    x: i32,
    y: i32,
    n: i32,
) -> Option<Image> {
    if y < 0 || y >= n {
        return None;
    }
    let plain = match service {
        Some(service) => format!("{service}/tile/{zoom}/{y}/{x}"),
        None => layer.tile_url(zoom, x, y),
    };
    if let Some(hit) = cache_get(&plain) {
        return Some(hit);
    }
    let url = with_map_token(&plain);
    let (_final, response) = fetch(&url).ok()?;
    if !(200..300).contains(&response.status) {
        return None;
    }
    let image = decode_auto(&response.body).ok()?;
    if !plain.contains("token=") {
        cache_put(plain, image.clone());
    }
    Some(image)
}

fn paint_map(tiles: &[Option<Image>]) -> Image {
    let mut canvas = Image::new(MAP_COLS * MAP_TILE, MAP_ROWS * MAP_TILE);
    for px in canvas.pixels.as_chunks_mut::<4>().0 {
        px[0] = 214;
        px[1] = 210;
        px[2] = 196;
        px[3] = 255;
    }
    for (index, tile) in tiles.iter().enumerate() {
        let Some(src) = tile else {
            continue;
        };
        let col = (index as u32) % MAP_COLS;
        let row = (index as u32) / MAP_COLS;
        blit(&mut canvas, src, col * MAP_TILE, row * MAP_TILE);
    }
    canvas
}

fn blit(dst: &mut Image, src: &Image, dx: u32, dy: u32) {
    if dx >= dst.width || dy >= dst.height || src.width == 0 || src.height == 0 {
        return;
    }
    let copy_w = src.width.min(dst.width - dx).min(MAP_TILE);
    let copy_h = src.height.min(dst.height - dy).min(MAP_TILE);
    let need = (copy_h as usize)
        .saturating_mul(src.width as usize)
        .saturating_mul(4);
    if src.pixels.len() < need {
        return;
    }
    for y in 0..copy_h {
        let si = (y as usize) * (src.width as usize) * 4;
        let di = ((dy + y) as usize * dst.width as usize + dx as usize) * 4;
        let n = copy_w as usize * 4;
        dst.pixels[di..di + n].copy_from_slice(&src.pixels[si..si + n]);
    }
}

#[derive(Clone)]
struct WebmapInfo {
    layer: Option<MapLayer>,
    service: Option<String>,
    features: Option<String>,
}

fn clean_item_id(id: &str) -> Option<String> {
    let id = id.trim();
    if (10..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        Some(id.to_string())
    } else {
        None
    }
}

fn clean_token(token: &str) -> Option<String> {
    let token = token.trim();
    if !(8..=512).contains(&token.len()) {
        return None;
    }
    if token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'='))
    {
        Some(token.to_string())
    } else {
        None
    }
}

fn https_arcgis(url: &str) -> bool {
    let Ok(parsed) = parse_url(url) else {
        return false;
    };
    parsed.scheme == "https" && is_arcgis_host(&parsed.host)
}

fn clean_feature_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    if !https_arcgis(url) {
        return None;
    }
    if !url.to_ascii_lowercase().contains("featureserver") {
        return None;
    }
    Some(url.to_string())
}

fn clean_map_server(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    if !https_arcgis(url) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if !lower.contains("/mapserver") || lower.contains("featureserver") {
        return None;
    }
    let root = url.split("/tile/").next().unwrap_or(url);
    Some(root.trim_end_matches('/').to_string())
}

fn layer_for_service(url: &str) -> Option<MapLayer> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("world_imagery") {
        Some(MapLayer::Imagery)
    } else if lower.contains("world_topo") {
        Some(MapLayer::Topo)
    } else if lower.contains("world_street") {
        Some(MapLayer::Streets)
    } else {
        None
    }
}

fn feature_query(query: &str) -> Option<String> {
    query_item(query, "f").and_then(|url| clean_feature_url(&url))
}

fn map_token() -> &'static std::sync::Mutex<Option<String>> {
    static TOKEN: std::sync::LazyLock<std::sync::Mutex<Option<String>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(None));
    &TOKEN
}

fn remember_map_token(token: &str) {
    *map_token().lock().unwrap_or_else(|err| err.into_inner()) = Some(token.to_string());
}

fn with_map_token(url: &str) -> String {
    if !https_arcgis(url) {
        return url.to_string();
    }
    let guard = map_token().lock().unwrap_or_else(|err| err.into_inner());
    let Some(token) = guard.as_deref() else {
        return url.to_string();
    };
    let token = urlencode_component(token);
    if url.contains('?') {
        format!("{url}&token={token}")
    } else {
        format!("{url}?token={token}")
    }
}

fn tile_cache() -> &'static std::sync::Mutex<Vec<(String, Image)>> {
    static CACHE: std::sync::LazyLock<std::sync::Mutex<Vec<(String, Image)>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
    &CACHE
}

fn cache_get(url: &str) -> Option<Image> {
    if url.contains("token=") {
        return None;
    }
    let guard = tile_cache().lock().unwrap_or_else(|err| err.into_inner());
    guard
        .iter()
        .find(|(key, _)| key == url)
        .map(|(_, image)| image.clone())
}

fn cache_put(url: String, image: Image) {
    if url.contains("token=") {
        return;
    }
    let mut guard = tile_cache().lock().unwrap_or_else(|err| err.into_inner());
    if guard.iter().any(|(key, _)| key == &url) {
        return;
    }
    if guard.len() >= 96 {
        guard.remove(0);
    }
    guard.push((url, image));
}

fn webmap_cache() -> &'static std::sync::Mutex<Vec<(String, WebmapInfo)>> {
    static CACHE: std::sync::LazyLock<std::sync::Mutex<Vec<(String, WebmapInfo)>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
    &CACHE
}

fn load_webmap(id: &str) -> Option<WebmapInfo> {
    {
        let guard = webmap_cache().lock().unwrap_or_else(|err| err.into_inner());
        if let Some((_, info)) = guard.iter().find(|(key, _)| key == id) {
            return Some(info.clone());
        }
    }
    let url = with_map_token(&format!(
        "https://www.arcgis.com/sharing/rest/content/items/{id}/data?f=json"
    ));
    let (_final, response) = fetch(&url).ok()?;
    if !(200..300).contains(&response.status) {
        return None;
    }
    let text = String::from_utf8_lossy(&response.body);
    let value = parse_json(&text).ok()?;
    if value.get("error").is_some() {
        return None;
    }
    let mut service = None;
    let mut layer = None;
    if let Some(layers) = value
        .get("baseMap")
        .and_then(|base| base.get("baseMapLayers"))
        .and_then(|layers| layers.as_array())
    {
        for entry in layers {
            let Some(url) = entry.get_str("url") else {
                continue;
            };
            let Some(root) = clean_map_server(url) else {
                continue;
            };
            layer = layer_for_service(&root);
            service = Some(root);
            break;
        }
    }
    let mut features = None;
    if let Some(layers) = value
        .get("operationalLayers")
        .and_then(|layers| layers.as_array())
    {
        for entry in layers {
            let Some(url) = entry.get_str("url") else {
                continue;
            };
            if let Some(url) = clean_feature_url(url) {
                features = Some(url);
                break;
            }
        }
    }
    let info = WebmapInfo {
        layer,
        service,
        features,
    };
    let mut guard = webmap_cache().lock().unwrap_or_else(|err| err.into_inner());
    if guard.len() >= 16 {
        guard.remove(0);
    }
    guard.push((id.to_string(), info.clone()));
    Some(info)
}

fn draw_feature_layer(canvas: &mut Image, origin_x: i32, origin_y: i32, zoom: u32, features: &str) {
    let Some(url) = clean_feature_url(features) else {
        return;
    };
    let (north, west) = mercator_latlon(f64::from(origin_x), f64::from(origin_y), zoom);
    let (south, east) = mercator_latlon(
        f64::from(origin_x + MAP_COLS as i32),
        f64::from(origin_y + MAP_ROWS as i32),
        zoom,
    );
    let request = with_map_token(&format!(
        "{url}/query?where=1%3D1&geometry={west:.5},{south:.5},{east:.5},{north:.5}&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=objectid&returnGeometry=true&outSR=4326&f=json&resultRecordCount=80"
    ));
    let Ok((_final, response)) = fetch(&request) else {
        return;
    };
    if !(200..300).contains(&response.status) {
        return;
    }
    let text = String::from_utf8_lossy(&response.body);
    let Ok(value) = parse_json(&text) else {
        return;
    };
    let Some(features) = value
        .get("features")
        .and_then(|features| features.as_array())
    else {
        return;
    };
    for feature in features.iter().take(80) {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };
        if let (Some(x), Some(y)) = (
            geometry.get("x").and_then(|n| n.as_f64()),
            geometry.get("y").and_then(|n| n.as_f64()),
        ) {
            let (px, py) = canvas_px(y, x, origin_x, origin_y, zoom);
            mark(canvas, px, py);
        }
        if let Some(paths) = geometry.get("paths").and_then(|paths| paths.as_array()) {
            for path in paths {
                stroke_ring(canvas, path, origin_x, origin_y, zoom, false);
            }
        }
        if let Some(rings) = geometry.get("rings").and_then(|rings| rings.as_array()) {
            for ring in rings {
                stroke_ring(canvas, ring, origin_x, origin_y, zoom, true);
            }
        }
    }
}

fn stroke_ring(
    canvas: &mut Image,
    line: &anvil_json::JsonValue,
    origin_x: i32,
    origin_y: i32,
    zoom: u32,
    close: bool,
) {
    let Some(points) = line.as_array() else {
        return;
    };
    let mut prev: Option<(i32, i32)> = None;
    let mut first: Option<(i32, i32)> = None;
    for point in points.iter().take(400) {
        let Some(pair) = point.as_array() else {
            continue;
        };
        if pair.len() < 2 {
            continue;
        }
        let Some(lon) = pair[0].as_f64() else {
            continue;
        };
        let Some(lat) = pair[1].as_f64() else {
            continue;
        };
        let here = canvas_px(lat, lon, origin_x, origin_y, zoom);
        if first.is_none() {
            first = Some(here);
        }
        if let Some(before) = prev {
            stroke(canvas, before.0, before.1, here.0, here.1);
        }
        prev = Some(here);
    }
    if close && let (Some(start), Some(end)) = (first, prev) {
        stroke(canvas, end.0, end.1, start.0, start.1);
    }
}

fn canvas_px(lat: f64, lon: f64, origin_x: i32, origin_y: i32, zoom: u32) -> (i32, i32) {
    let (x, y) = mercator_xy(lat, lon, zoom);
    let px = ((x - f64::from(origin_x)) * f64::from(MAP_TILE)).round() as i32;
    let py = ((y - f64::from(origin_y)) * f64::from(MAP_TILE)).round() as i32;
    (px, py)
}

fn mark(canvas: &mut Image, x: i32, y: i32) {
    for dy in -3..=3 {
        for dx in -3..=3 {
            paint_px(canvas, x + dx, y + dy);
        }
    }
}

fn stroke(canvas: &mut Image, mut x0: i32, mut y0: i32, x1: i32, y1: i32) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    for _ in 0..2048 {
        paint_px(canvas, x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = err * 2;
        if doubled >= dy {
            err += dy;
            x0 += sx;
        }
        if doubled <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn paint_px(canvas: &mut Image, x: i32, y: i32) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= canvas.width || y >= canvas.height {
        return;
    }
    canvas.set_pixel(x, y, [232, 168, 56, 255]);
}

fn mercator_xy(lat: f64, lon: f64, zoom: u32) -> (f64, f64) {
    let n = f64::from(1u32 << zoom);
    let x = (wrap_lon(lon) + 180.0) / 360.0 * n;
    let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
    let y = (1.0 - ((1.0 + lat.sin()).ln() / std::f64::consts::PI)) / 2.0 * n;
    (x, y)
}

fn mercator_latlon(x: f64, y: f64, zoom: u32) -> (f64, f64) {
    let n = f64::from(1u32 << zoom);
    let lon = x / n * 360.0 - 180.0;
    let t = std::f64::consts::PI * (1.0 - 2.0 * y / n);
    let lat = t.sinh().atan().to_degrees();
    (lat, wrap_lon(lon))
}

fn shift(lat: f64, lon: f64, zoom: u32, dx: f64, dy: f64) -> (f64, f64) {
    let (x, y) = mercator_xy(lat, lon, zoom);
    let n = f64::from(1u32 << zoom);
    mercator_latlon(x + dx, y.clamp(0.0, n) + dy, zoom)
}

fn wrap_lon(lon: f64) -> f64 {
    if !lon.is_finite() {
        return 0.0;
    }
    (lon + 180.0).rem_euclid(360.0) - 180.0
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
            Self::Reddit => "Reddit",
            Self::Social => "Social",
            Self::Bank => "Bank",
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
            Self::Reddit => "https://www.reddit.com/",
            Self::Social => "https://discord.com/",
            Self::Bank => "https://www.chase.com/",
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
        if matches_key && let Some(content) = attr_value(tag, "content") {
            let text = decode_entities(&strip_tags(&content));
            if !text.trim().is_empty() {
                return Some(text.trim().to_string());
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

/// Keep a YouTube document and, when it is a watch or results page, add a player,
/// or normalize a Google Search JS-challenge/Wiz shell into a clean search document.
///
/// The player points at `anvil-yt-frames` or `anvil-yt-board`. Those are not
/// network URLs. [`youtube_player_clip`] turns them into frames later. Other
/// hosts are returned unchanged. A second call sees `id="anvil-youtube"` or
/// `id="anvil-google-search"` and does not rewrite again.
pub(crate) fn prepare_document(html: &str, url: &str) -> String {
    if let Some(google_doc) = prepare_google_search_document(html, url) {
        return google_doc;
    }
    if let Some(ddg_doc) = prepare_ddg_search_document(html, url) {
        return ddg_doc;
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

fn prepare_google_search_document(html: &str, url: &str) -> Option<String> {
    if html.contains("id=\"anvil-google-search\"") {
        return None;
    }
    let parsed = parse_url(url.trim()).ok()?;
    let host = parsed.host.as_str();
    if host != "google.com" && host != "www.google.com" {
        return None;
    }
    let path = parsed.path.as_str();
    let is_home = path.is_empty() || path == "/" || path == "/webhp";
    let is_search = path == "/search"
        || path.starts_with("/search/")
        || path.starts_with("/httpservice/retry/enablejs")
        || path == "/url";
    if !is_home && !is_search {
        return None;
    }
    let query = parsed
        .query
        .as_deref()
        .and_then(|q| query_item(q, "q").or_else(|| query_item(q, "as_q")))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let is_challenge_or_wiz = html.contains("/httpservice/retry/enablejs")
        || html.contains("SG_SS=")
        || html.contains("jsaction=\"")
        || html.contains("Please click here if you are not redirected");

    if is_home && query.is_empty() {
        if is_challenge_or_wiz || !html.to_ascii_lowercase().contains("<form") {
            return Some(render_google_home_html());
        }
        return None;
    }

    let mut results = parse_google_results_in(html);
    if results.is_empty() && is_challenge_or_wiz && !query.is_empty() {
        results = fetch_fallback_search_results(&query);
    }
    if !results.is_empty() || is_challenge_or_wiz {
        return Some(render_google_results_html(&query, &results));
    }
    None
}

fn prepare_ddg_search_document(html: &str, url: &str) -> Option<String> {
    if html.contains("id=\"anvil-ddg-search\"") {
        return None;
    }
    let parsed = parse_url(url.trim()).ok()?;
    let host = parsed.host.as_str();
    if host != "duckduckgo.com"
        && host != "www.duckduckgo.com"
        && host != "html.duckduckgo.com"
        && host != "lite.duckduckgo.com"
    {
        return None;
    }
    let query = parsed
        .query
        .as_deref()
        .and_then(|q| query_item(q, "q"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let results = parse_ddg_results_in(html);
    if !results.is_empty() || (!query.is_empty() && parsed.path.starts_with("/html")) {
        return Some(render_ddg_results_html(&query, &results));
    }
    None
}

fn render_ddg_results_html(query: &str, results: &[WebSearchResult]) -> String {
    let escaped_q = escape_html(query);
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>");
    if !escaped_q.is_empty() {
        out.push_str(&escaped_q);
        out.push_str(" - ");
    }
    out.push_str(
        "DuckDuckGo Search</title>\
         <style>\
         html, body { margin: 0; padding: 0; background-color: #11131a; color: #eceff4; font-family: sans-serif; }\
         .top { background-color: #171a24; border-bottom: 1px solid #282e3d; padding: 14px 24px; }\
         .bar { display: flex; flex-direction: row; align-items: center; }\
         .logo { margin-right: 16px; }\
         .sinput { width: 420px; padding: 8px 14px; background-color: #202431; color: #eceff4; border: 1px solid #3b4252; border-radius: 18px; font-size: 14px; margin-right: 10px; }\
         .sbtn { padding: 8px 16px; background-color: #de5833; color: #ffffff; border: 1px solid #de5833; border-radius: 16px; font-size: 13px; font-weight: bold; }\
         .main { max-width: 760px; padding: 20px 28px; }\
         .meta { color: #9aa5b8; font-size: 12px; margin-bottom: 16px; }\
         .card { background-color: #171a24; border: 1px solid #262c3a; border-radius: 10px; padding: 14px 18px; margin-bottom: 12px; }\
         .cite { color: #a3be8c; font-size: 12px; margin-bottom: 4px; }\
         .rtitle { color: #8ab4f8; font-size: 17px; font-weight: bold; text-decoration: none; display: block; margin-bottom: 6px; }\
         .snip { color: #d8dee9; font-size: 14px; line-height: 20px; }\
         </style></head><body>\
         <div id=\"anvil-ddg-search\">\
           <div class=\"top\">\
             <form class=\"bar\" action=\"https://html.duckduckgo.com/html/\" method=\"GET\">\
               <img class=\"logo\" src=\"anvil://engine-ddg.png\" width=\"28\" height=\"28\" alt=\"DuckDuckGo\">\
               <input class=\"sinput\" type=\"text\" name=\"q\" value=\"",
    );
    out.push_str(&escaped_q);
    out.push_str(
        "\">\
               <button class=\"sbtn\" type=\"submit\">Search</button>\
             </form>\
           </div>\
           <div class=\"main\">",
    );
    if results.is_empty() {
        out.push_str("<div class=\"card\"><div class=\"rtitle\">No results matched your search</div><div class=\"snip\">Try another search phrase in the bar above.</div></div>");
    } else {
        out.push_str(&format!(
            "<div class=\"meta\">Showing {} web results for <b>{}</b></div>",
            results.len(),
            escaped_q
        ));
        for hit in results {
            let href = escape_html(&hit.url);
            let title = escape_html(&hit.title);
            let snip = escape_html(&hit.snippet);
            out.push_str("<div class=\"card\">");
            out.push_str(&format!("<div class=\"cite\">{href}</div>"));
            out.push_str(&format!("<a class=\"rtitle\" href=\"{href}\">{title}</a>"));
            if !snip.is_empty() {
                out.push_str(&format!("<div class=\"snip\">{snip}</div>"));
            }
            out.push_str("</div>");
        }
    }
    out.push_str("</div></div></body></html>");
    out
}

fn render_google_home_html() -> String {
    String::from(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Google</title>\
        <style>\
        html, body { margin: 0; padding: 0; background-color: #202124; color: #e8eaed; font-size: 16px; }\
        .wrap { max-width: 640px; margin: 72px auto 0 auto; padding: 24px; text-align: center; box-sizing: border-box; }\
        .brand { margin-bottom: 20px; }\
        h1 { font-size: 40px; margin: 8px 0 20px 0; color: #ffffff; }\
        form { margin: 0 auto 20px auto; }\
        input { width: 460px; padding: 10px 14px; font-size: 16px; color: #e8eaed; background-color: #303134; border: 1px solid #5f6368; border-radius: 20px; box-sizing: border-box; }\
        .actions { margin-top: 16px; }\
        button { padding: 8px 18px; font-size: 14px; color: #e8eaed; background-color: #303134; border: 1px solid #5f6368; border-radius: 6px; }\
        a { color: #8ab4f8; }\
        </style></head><body>\
        <div id=\"anvil-google-search\" class=\"wrap\">\
        <div class=\"brand\"><img src=\"anvil://engine-google.png\" width=\"36\" height=\"36\" alt=\"Google\"></div>\
        <h1>Google</h1>\
        <form action=\"https://www.google.com/search\" method=\"get\">\
        <input type=\"text\" name=\"q\" value=\"\" placeholder=\"Search Google or type a query\">\
        <div class=\"actions\"><button type=\"submit\">Google Search</button></div>\
        </form>\
        <p><a href=\"anvil://home\">Return to Start</a></p>\
        </div></body></html>",
    )
}

fn render_google_results_html(query: &str, results: &[WebSearchResult]) -> String {
    let escaped_q = escape_html(query);
    let title = if query.is_empty() {
        "Google Search".to_string()
    } else {
        format!("{} - Google Search", escape_html(query))
    };
    let mut body = String::with_capacity(4096);
    body.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>");
    body.push_str(&title);
    body.push_str(
        "</title><style>\
        html, body { margin: 0; padding: 0; background-color: #202124; color: #e8eaed; font-size: 15px; }\
        .top { padding: 18px 28px; background-color: #171717; border-bottom: 1px solid #3c4043; }\
        .top form { display: flex; flex-direction: row; align-items: center; gap: 12px; }\
        .top input { width: 480px; padding: 8px 14px; font-size: 15px; color: #e8eaed; background-color: #303134; border: 1px solid #5f6368; border-radius: 18px; box-sizing: border-box; }\
        .top button { padding: 8px 16px; font-size: 14px; color: #e8eaed; background-color: #303134; border: 1px solid #5f6368; border-radius: 6px; }\
        .main { max-width: 760px; padding: 20px 28px 36px 28px; box-sizing: border-box; }\
        .card { margin-bottom: 22px; padding-bottom: 12px; border-bottom: 1px solid #303134; }\
        .cite { font-size: 13px; color: #9aa0a6; margin: 0 0 4px 0; }\
        .card h3 { font-size: 19px; margin: 0 0 6px 0; }\
        .card a { color: #8ab4f8; }\
        .snippet { color: #bdc1c6; margin: 0; }\
        </style></head><body><div id=\"anvil-google-search\">",
    );
    body.push_str(
        "<div class=\"top\"><form action=\"https://www.google.com/search\" method=\"get\">",
    );
    body.push_str(
        "<img src=\"anvil://engine-google.png\" width=\"36\" height=\"36\" alt=\"Google\">",
    );
    body.push_str("<input type=\"text\" name=\"q\" value=\"");
    body.push_str(&escaped_q);
    body.push_str("\"><button type=\"submit\">Google Search</button></form></div>");
    body.push_str("<div class=\"main\">");
    if results.is_empty() {
        if query.is_empty() {
            body.push_str("<p>Enter a search term above to search the web.</p>");
        } else {
            body.push_str("<p>No web results were returned for <strong>");
            body.push_str(&escaped_q);
            body.push_str("</strong>.</p>");
        }
    } else {
        for item in results {
            body.push_str("<div class=\"card\"><p class=\"cite\">");
            body.push_str(&escape_html(&item.url));
            body.push_str("</p><h3><a href=\"");
            body.push_str(&escape_html(&item.url));
            body.push_str("\">");
            body.push_str(&escape_html(&item.title));
            body.push_str("</a></h3>");
            if !item.snippet.is_empty() {
                body.push_str("<p class=\"snippet\">");
                body.push_str(&escape_html(&item.snippet));
                body.push_str("</p>");
            }
            body.push_str("</div>");
        }
    }
    body.push_str("<p><a href=\"anvil://home\">Return to Start</a></p></div></div></body></html>");
    body
}

pub(crate) fn parse_google_results_in(html: &str) -> Vec<WebSearchResult> {
    let scan = player_scan(html);
    let lower = scan.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while out.len() < 10 && cursor < lower.len() {
        let Some(h3_rel) = lower[cursor..].find("<h3") else {
            break;
        };
        let h3_start = cursor + h3_rel;
        let Some(h3_open_rel) = lower[h3_start..].find('>') else {
            break;
        };
        let h3_body_start = h3_start + h3_open_rel + 1;
        let Some(h3_close_rel) = lower[h3_body_start..].find("</h3>") else {
            break;
        };
        let h3_end = h3_body_start + h3_close_rel;
        let title = decode_entities(&strip_tags(&scan[h3_body_start..h3_end]))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        cursor = h3_end + "</h3>".len();
        if title.is_empty() {
            continue;
        }

        // Find enclosing `<a href="...">` either before `<h3>` or inside `<h3>`.
        let inside_href = xml_attr(&scan[h3_start..h3_end], "a", "href");
        let before_window_start = h3_start.saturating_sub(600);
        let before_window = &scan[before_window_start..h3_start];
        let before_lower = before_window.to_ascii_lowercase();
        let before_href = before_lower
            .rfind("<a ")
            .and_then(|pos| attr_value(&before_window[pos..], "href"));

        let raw_href = inside_href.or(before_href).unwrap_or_default();
        let Some(clean_url) = clean_search_result_href(&raw_href) else {
            continue;
        };
        if out.iter().any(|r: &WebSearchResult| r.url == clean_url) {
            continue;
        }

        // Extract optional snippet following `</h3>`.
        let snippet_end = scan.len().min(cursor + 900);
        let after_h3 = &scan[cursor..snippet_end];
        let after_lower = after_h3.to_ascii_lowercase();
        let stop_rel = after_lower
            .find("<h3")
            .or_else(|| after_lower.find("<a "))
            .unwrap_or(after_h3.len());
        let snippet = decode_entities(&strip_tags(&after_h3[..stop_rel]))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let snippet: String = snippet.chars().take(240).collect();

        out.push(WebSearchResult {
            title,
            url: clean_url,
            snippet,
        });
    }
    out
}

pub(crate) fn parse_ddg_results_in(html: &str) -> Vec<WebSearchResult> {
    let scan = player_scan(html);
    let lower = scan.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while out.len() < 10 && cursor < lower.len() {
        let Some(rel) = lower[cursor..].find("result__a") else {
            break;
        };
        let marker = cursor + rel;
        let tag_start = lower[..marker].rfind("<a").unwrap_or(marker);
        let Some(tag_end_rel) = lower[tag_start..].find('>') else {
            cursor = marker + 9;
            continue;
        };
        let tag_end = tag_start + tag_end_rel;
        let Some(close_rel) = lower[tag_end + 1..].find("</a>") else {
            cursor = tag_end + 1;
            continue;
        };
        let close_pos = tag_end + 1 + close_rel;
        let raw_href = attr_value(&scan[tag_start..=tag_end], "href").unwrap_or_default();
        let title = decode_entities(&strip_tags(&scan[tag_end + 1..close_pos]))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        cursor = close_pos + 4;
        let Some(clean_url) = clean_search_result_href(&raw_href) else {
            continue;
        };
        if title.is_empty() || out.iter().any(|r: &WebSearchResult| r.url == clean_url) {
            continue;
        }
        let window_end = scan.len().min(cursor + 1200);
        let window = &scan[cursor..window_end];
        let window_lower = window.to_ascii_lowercase();
        let snippet = if let Some(snip_rel) = window_lower.find("result__snippet") {
            let after_snip = &window[snip_rel..];
            let after_snip_lower = after_snip.to_ascii_lowercase();
            if let Some(gt) = after_snip_lower.find('>') {
                let body = &after_snip[gt + 1..];
                let body_lower = body.to_ascii_lowercase();
                let end = body_lower
                    .find("</a>")
                    .or_else(|| body_lower.find("</div>"))
                    .or_else(|| body_lower.find("</span>"))
                    .unwrap_or(body.len());
                decode_entities(&strip_tags(&body[..end]))
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        out.push(WebSearchResult {
            title,
            url: clean_url,
            snippet,
        });
    }
    out
}

fn clean_search_result_href(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("/url?") {
        if let Some(target) = query_item(rest, "q").or_else(|| query_item(rest, "url")) {
            return safe_https_url(&target).map(str::to_string);
        }
    }
    if let Some((_, query)) = trimmed.split_once("uddg=") {
        let encoded = query.split('&').next().unwrap_or(query);
        let decoded = urldecode_component(encoded);
        if let Some(valid) = safe_https_url(&decoded) {
            return Some(valid.to_string());
        }
    }
    let candidate = safe_https_url(trimmed)?;
    let parsed = parse_url(candidate).ok()?;
    if parsed.host == "google.com"
        || parsed.host.ends_with(".google.com")
        || parsed.host == "duckduckgo.com"
        || parsed.host.ends_with(".duckduckgo.com")
    {
        return None;
    }
    Some(candidate.to_string())
}

fn fetch_fallback_search_results(query: &str) -> Vec<WebSearchResult> {
    if cfg!(test) || query.trim().is_empty() {
        return Vec::new();
    }
    let endpoint = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencode_component(query.trim())
    );
    let Ok((_final_url, response)) = fetch(&endpoint) else {
        return Vec::new();
    };
    if !(200..300).contains(&response.status) {
        return Vec::new();
    }
    let html = String::from_utf8_lossy(&response.body);
    parse_ddg_results_in(&html)
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
        if let Some(clip) = storyboard_clip(rest)
            && !clip.frames.is_empty()
        {
            return Some(clip);
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
    if let Some(idx) = lower.find("<body")
        && let Some(rel) = lower[idx..].find('>')
    {
        let at = idx + rel + 1;
        let mut out = String::with_capacity(html.len() + block.len());
        out.push_str(&html[..at]);
        out.push_str(block);
        out.push_str(&html[at..]);
        return out;
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
    let sheets = count.div_ceil(per).min(2);
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
    fn reddit_stays_on_the_address_you_opened() {
        assert_eq!(
            passthrough_rewrite("https://www.reddit.com/r/rust?sort=hot"),
            None
        );
        assert_eq!(passthrough_rewrite("https://old.reddit.com/r/rust"), None);
        assert_eq!(
            app_kind("https://www.reddit.com/r/rust"),
            Some(AppKind::Reddit)
        );
        assert_eq!(
            app_kind("https://old.reddit.com/r/rust"),
            Some(AppKind::Reddit)
        );
        assert!(!fetch_page_scripts("https://www.reddit.com/"));
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
        assert_eq!(
            app_kind("https://secure.chase.com/web/auth"),
            Some(AppKind::Bank)
        );
        assert_eq!(
            app_kind("https://www.bankofamerica.com/"),
            Some(AppKind::Bank)
        );
        assert_eq!(
            app_kind("https://www.paypal.com/signin"),
            Some(AppKind::Bank)
        );
        assert_eq!(
            app_kind("https://www.commercebank.com/"),
            Some(AppKind::Bank)
        );
        assert_eq!(
            app_kind("https://discord.com/channels/@me"),
            Some(AppKind::Social)
        );
        assert_eq!(app_kind("https://bsky.app/"), Some(AppKind::Social));
        assert_eq!(app_kind("https://web.whatsapp.com/"), Some(AppKind::Social));
        assert!(!fetch_page_scripts("https://www.wellsfargo.com/"));
        assert!(fetch_page_scripts("https://en.wikipedia.org/"));
        assert_eq!(app_kind("https://notchase.com/"), None);
        assert_eq!(app_kind("https://chase.com.evil.example/"), None);
        assert_eq!(app_kind("https://redditstatic.com/"), None);
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
    fn maps_page_quotes_the_query_and_points_at_tiles() {
        let html = maps_html("https://www.google.com/maps?q=cafe+%26+tea");
        assert!(html.contains("value=\"cafe &amp; tea\""));
        assert!(!html.contains("cafe & tea"));
        assert!(html.contains("OpenStreetMap"));
        assert!(!html.contains("anvil-map://"));
        let placed = maps_html("https://www.google.com/maps/@38.62700,-90.19940,12z");
        assert!(placed.contains("anvil-map://12/38.62700/-90.19940"));
        assert!(placed.contains("https://www.google.com/maps/@"));
        let osm = maps_html("https://www.openstreetmap.org/#map=5/51.5/-0.12");
        assert!(osm.contains("anvil-map://5/51.50000/-0.12000"));
        assert!(safe_https_url("javascript:alert(1)").is_none());
        assert!(safe_https_url("https://example.com/a.jpg").is_some());
        let arc = maps_html(
            "https://www.arcgis.com/apps/mapviewer/index.html?center=-90.19940,38.62700&level=12",
        );
        assert!(arc.contains("anvil-map://streets/12/38.62700/-90.19940"));
        assert!(arc.contains("layer=imagery"));
        assert!(arc.contains("ArcGIS streets"));
        let photo = maps_html(
            "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/2/1/1",
        );
        assert!(photo.contains("anvil-map://imagery/2/0.00000/0.00000"));
        assert!(map_bitmap("anvil-map://nope/2/0/0").is_none());
        let (x, y) = mercator_xy(0.0, 0.0, 2);
        assert!((x - 2.0).abs() < 1e-9);
        assert!((y - 2.0).abs() < 1e-9);
        let (lat, lon) = mercator_latlon(x, y, 2);
        assert!(lat.abs() < 1e-6);
        assert!(lon.abs() < 1e-6);
        let mut red = Image::new(1, 1);
        red.set_pixel(0, 0, [200, 10, 10, 255]);
        let mut tiles = vec![None, None, None, None, None, None, None, None, None];
        tiles[0] = Some(red);
        let painted = paint_map(&tiles);
        assert_eq!(painted.width, 768);
        assert_eq!(painted.height, 768);
        assert_eq!(painted.pixel(0, 0), Some([200, 10, 10, 255]));
        assert_eq!(painted.pixel(1, 0), Some([214, 210, 196, 255]));
        assert!(map_bitmap("anvil-map://99/0/0").is_none());
        assert!(gis_map_page("https://example.com/").is_none());
        assert!(
            gis_map_page("anvil://maps")
                .unwrap()
                .contains("anvil-map://2/")
        );
        let saved = maps_html("https://www.arcgis.com/home/webmap/viewer.html?webmap=abc123def456");
        assert!(saved.contains("anvil-map://webmap/abc123def456/"));
        assert!(saved.contains("name=\"webmap\""));
        let secret = "abcdefghijk";
        let with_token = maps_html(&format!("anvil://maps?lat=1&lon=2&z=3&token={secret}"));
        assert!(!with_token.contains(secret));
        assert!(with_token.contains("is not saved"));
        assert!(clean_feature_url("https://evil.example/FeatureServer/0").is_none());
        assert!(
            clean_feature_url(
                "https://services.arcgis.com/abc/arcgis/rest/services/Places/FeatureServer/0"
            )
            .is_some()
        );
        let mut canvas = Image::new(8, 8);
        stroke(&mut canvas, 0, 0, 4, 0);
        assert_eq!(canvas.pixel(0, 0), Some([232, 168, 56, 255]));
        assert_eq!(canvas.pixel(4, 0), Some([232, 168, 56, 255]));
        let mut cached = Image::new(1, 1);
        cached.set_pixel(0, 0, [9, 9, 9, 255]);
        cache_put(
            "https://tile.openstreetmap.org/test/cache.png".to_string(),
            cached,
        );
        assert!(cache_get("https://tile.openstreetmap.org/test/cache.png").is_some());
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

    #[test]
    fn google_search_challenge_and_results_prepare_clean_document() {
        let challenge_home = "<html><body><script>var a='SG_SS=';</script><noscript><meta content=\"0;url=/httpservice/retry/enablejs\" http-equiv=\"refresh\">Please click here if you are not redirected</noscript></body></html>";
        let out_home = prepare_document(challenge_home, "https://www.google.com/");
        assert!(out_home.contains("id=\"anvil-google-search\""));
        assert!(out_home.contains("action=\"https://www.google.com/search\""));
        assert!(!out_home.contains("enablejs"));

        let raw_search = "<html><body><noscript><style>table,div,span,p{display:none}</style></noscript><div class=\"g\"><a href=\"/url?q=https://www.rust-lang.org/&amp;sa=U\"><h3>Rust Programming Language</h3></a><div>A language empowering everyone to build reliable software.</div></div></body></html>";
        let out_search = prepare_document(raw_search, "https://www.google.com/search?q=rust+lang");
        assert!(out_search.contains("id=\"anvil-google-search\""));
        assert!(out_search.contains("value=\"rust lang\""));
        assert!(out_search.contains("https://www.rust-lang.org/"));
        assert!(out_search.contains("Rust Programming Language"));
        assert!(out_search.contains("empowering everyone"));

        let ddg_html = "<div><a class=\"result__a\" href=\"//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F&amp;rut=123\">The Rust Programming Language Book</a><a class=\"result__snippet\">Affectionately nicknamed the book.</a></div>";
        let ddg_parsed = parse_ddg_results_in(ddg_html);
        assert_eq!(ddg_parsed.len(), 1);
        assert_eq!(ddg_parsed[0].url, "https://doc.rust-lang.org/book/");
        assert_eq!(ddg_parsed[0].title, "The Rust Programming Language Book");
        assert_eq!(ddg_parsed[0].snippet, "Affectionately nicknamed the book.");

        let out_ddg = prepare_document(ddg_html, "https://html.duckduckgo.com/html/?q=rust+book");
        assert!(out_ddg.contains("id=\"anvil-ddg-search\""));
        assert!(out_ddg.contains("https://doc.rust-lang.org/book/"));
        assert!(out_ddg.contains("The Rust Programming Language Book"));
    }
}
