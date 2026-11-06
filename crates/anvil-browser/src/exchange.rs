// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Moving bookmarks, open tabs, and a few settings between two Anvil
//! installs through a plain JSON file (`anvil-exchange-v1`).
//!
//! The file is not sealed, so it only ever holds things that are fine to
//! read in a text editor: bookmarks, the addresses of open tabs, the theme,
//! the search engine, zoom, and whether HTTPS-only is on. History goes in
//! only when asked. Cookies, `localStorage`, downloads, and saved logins
//! never go in, and an import leaves them out with a warning if a file
//! has them.
//!
//! Nothing here runs by itself: a file is written or read only when you
//! name its path, and only while `local_exchange` is turned on. Moving the
//! file over the network or Bluetooth is not part of this.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::anvil_json::{JsonValue, parse_json_bounded, to_json_pretty};
use crate::{Bookmark, BrowserTheme, SearchEngine};

/// The `kind` every exchange file carries.
pub const EXCHANGE_KIND: &str = "anvil-exchange-v1";
/// Biggest file an import reads.
pub const MAX_EXCHANGE_BYTES: u64 = 8 * 1024 * 1024;
/// Most bookmarks or history rows taken from one file.
pub const MAX_EXCHANGE_ROWS: usize = 10_000;
/// Most tab addresses taken from one file.
pub const MAX_EXCHANGE_TABS: usize = 500;
const MAX_TITLE_BYTES: usize = 1024;
const MAX_URL_BYTES: usize = 4096;
const MAX_DEPTH: usize = 16;

/// Top-level keys that hold private browsing data. A file with any of
/// these still imports, without them.
const PRIVATE_KEYS: [(&str, &str); 10] = [
    ("cookies", "cookies"),
    ("cookie_jar", "cookies"),
    ("localStorage", "localStorage"),
    ("local_storage", "localStorage"),
    ("sessionStorage", "sessionStorage"),
    ("downloads", "downloads"),
    ("logins", "saved logins"),
    ("passwords", "saved logins"),
    ("hsts", "HSTS records"),
    ("site_permissions", "site permissions"),
];

/// What goes into an export besides bookmarks, tabs, and settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Put browsing history in too. Off unless asked for.
    pub history: bool,
}

/// The settings an exchange file carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExchangeSettings {
    pub theme: BrowserTheme,
    pub search_engine: SearchEngine,
    pub zoom_percent: u16,
    pub https_only_mode: bool,
}

/// The settings an imported file had. A missing one stays as it is here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportedSettings {
    pub theme: Option<BrowserTheme>,
    pub search_engine: Option<SearchEngine>,
    pub zoom_percent: Option<u16>,
    pub https_only_mode: Option<bool>,
}

/// What a file held once checked. Only these parts are ever applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExchangeData {
    pub bookmarks: Vec<Bookmark>,
    pub tabs: Vec<String>,
    pub settings: Option<ImportedSettings>,
    pub history: Option<Vec<Bookmark>>,
    /// Plain notes about what was left out.
    pub warnings: Vec<String>,
}

/// Why an export or import didn't happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// `local_exchange` is off.
    Off,
    /// No path was given.
    NoPath,
    /// The file couldn't be read or written.
    Io(String),
    /// Over [`MAX_EXCHANGE_BYTES`].
    TooBig,
    /// Not JSON, or not an `anvil-exchange-v1` file.
    NotExchange(String),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "import and export are turned off"),
            Self::NoPath => write!(f, "no file was named"),
            Self::Io(e) => write!(f, "the file could not be used: {e}"),
            Self::TooBig => write!(f, "the file is bigger than 8 MiB"),
            Self::NotExchange(why) => write!(f, "that isn't an {EXCHANGE_KIND} file: {why}"),
        }
    }
}

impl std::error::Error for ExchangeError {}

/// Whether `url` may go in or come out of an exchange file: `http`,
/// `https`, or a built-in page without a query.
pub fn exchange_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL_BYTES {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(url.to_string());
    }
    if lower.starts_with("anvil://") {
        // A query on a built-in page can carry the page key.
        let bare = url.split(['?', '#']).next().unwrap_or(url);
        return Some(bare.to_string());
    }
    None
}

fn rows_json(rows: &[Bookmark]) -> JsonValue {
    let mut list = JsonValue::empty_array();
    for row in rows {
        if let Some(url) = exchange_url(&row.url) {
            let mut item = JsonValue::empty_object();
            item.insert("title", JsonValue::String(row.title.clone()));
            item.insert("url", JsonValue::String(url));
            list.push(item);
        }
    }
    list
}

/// The text of an export. Only bookmarks, tab addresses, the settings
/// above, and history when `options.history` is set.
pub fn export_json(
    bookmarks: &[Bookmark],
    tabs: &[String],
    settings: ExchangeSettings,
    history: &[Bookmark],
    options: ExportOptions,
) -> String {
    let mut root = JsonValue::empty_object();
    root.insert("kind", JsonValue::String(EXCHANGE_KIND.to_string()));
    root.insert("bookmarks", rows_json(bookmarks));
    let mut tab_list = JsonValue::empty_array();
    for url in tabs.iter().filter_map(|u| exchange_url(u)) {
        tab_list.push(JsonValue::String(url));
    }
    root.insert("tabs", tab_list);
    let mut s = JsonValue::empty_object();
    s.insert(
        "theme",
        JsonValue::String(settings.theme.slug().to_string()),
    );
    s.insert(
        "search_engine",
        JsonValue::String(settings.search_engine.slug().to_string()),
    );
    s.insert(
        "zoom_percent",
        JsonValue::Number(settings.zoom_percent as f64),
    );
    s.insert("https_only_mode", JsonValue::Bool(settings.https_only_mode));
    root.insert("settings", s);
    if options.history {
        root.insert("history", rows_json(history));
    }
    to_json_pretty(&root)
}

fn read_rows(value: &JsonValue, name: &str, warnings: &mut Vec<String>) -> Vec<Bookmark> {
    let Some(items) = value.as_array() else {
        warnings.push(format!(
            "The {name} list wasn't a list, so it was left out."
        ));
        return Vec::new();
    };
    let mut rows = Vec::new();
    let mut skipped = 0usize;
    for item in items {
        let url = item
            .get("url")
            .and_then(JsonValue::as_str)
            .and_then(exchange_url);
        let title = item.get("title").and_then(JsonValue::as_str).unwrap_or("");
        match url {
            Some(url) if rows.len() < MAX_EXCHANGE_ROWS && title.len() <= MAX_TITLE_BYTES => rows
                .push(Bookmark {
                    title: title.to_string(),
                    url,
                }),
            _ => skipped += 1,
        }
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} {name} entries were left out (not an http, https, or anvil address, or too long)."
        ));
    }
    rows
}

/// Check an exchange file's text and keep only what may be imported.
/// Private data (cookies, `localStorage`, downloads, logins) and unknown
/// keys are dropped, each with a warning.
pub fn parse_exchange(text: &str) -> Result<ExchangeData, ExchangeError> {
    let root = parse_json_bounded(text, MAX_DEPTH, MAX_EXCHANGE_BYTES as usize)
        .map_err(|e| ExchangeError::NotExchange(e.to_string()))?;
    let Some(members) = root.as_object() else {
        return Err(ExchangeError::NotExchange("not a JSON object".to_string()));
    };
    if root.get("kind").and_then(JsonValue::as_str) != Some(EXCHANGE_KIND) {
        return Err(ExchangeError::NotExchange("wrong kind".to_string()));
    }
    let mut data = ExchangeData::default();
    for (key, value) in members {
        match key.as_str() {
            "kind" => {}
            "bookmarks" => data.bookmarks = read_rows(value, "bookmark", &mut data.warnings),
            "history" => data.history = Some(read_rows(value, "history", &mut data.warnings)),
            "tabs" => {
                let items = value.as_array().unwrap_or(&[]);
                let mut skipped = 0usize;
                for item in items {
                    match item.as_str().and_then(exchange_url) {
                        Some(url) if data.tabs.len() < MAX_EXCHANGE_TABS => data.tabs.push(url),
                        _ => skipped += 1,
                    }
                }
                if skipped > 0 {
                    data.warnings
                        .push(format!("{skipped} tab addresses were left out."));
                }
            }
            "settings" => data.settings = Some(read_settings(value, &mut data.warnings)),
            other => {
                let note = match PRIVATE_KEYS
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(other))
                {
                    Some((_, what)) => format!(
                        "The file had {what} in it. Those are never imported, so they were left out."
                    ),
                    None => format!(
                        "Left out \"{}\", which this browser doesn't import.",
                        clip_key(other)
                    ),
                };
                data.warnings.push(note);
            }
        }
    }
    Ok(data)
}

fn clip_key(key: &str) -> String {
    key.chars().filter(|c| !c.is_control()).take(40).collect()
}

fn read_settings(value: &JsonValue, warnings: &mut Vec<String>) -> ImportedSettings {
    let mut out = ImportedSettings::default();
    let Some(members) = value.as_object() else {
        warnings.push("The settings weren't an object, so they were left out.".to_string());
        return out;
    };
    for (key, v) in members {
        match key.as_str() {
            "theme" => {
                out.theme = v.as_str().map(BrowserTheme::from_slug);
            }
            "search_engine" => {
                out.search_engine = v.as_str().map(SearchEngine::from_slug);
            }
            "zoom_percent" => {
                out.zoom_percent = v.as_f64().map(|z| z.clamp(50.0, 250.0) as u16);
            }
            "https_only_mode" => out.https_only_mode = v.as_bool(),
            other => warnings.push(format!(
                "Left out the setting \"{}\", which isn't moved between browsers.",
                clip_key(other)
            )),
        }
    }
    out
}

/// Write `text` to `path` and nowhere else. A new file is readable only by
/// you on Unix.
pub fn write_file(path: &Path, text: &str) -> Result<(), ExchangeError> {
    use std::io::Write;
    if path.as_os_str().is_empty() {
        return Err(ExchangeError::NoPath);
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|e| ExchangeError::Io(e.kind().to_string()))?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| ExchangeError::Io(e.kind().to_string()))
}

/// Read and check the exchange file at `path`.
pub fn read_file(path: &Path) -> Result<ExchangeData, ExchangeError> {
    use std::io::Read;
    if path.as_os_str().is_empty() {
        return Err(ExchangeError::NoPath);
    }
    let file = fs::File::open(path).map_err(|e| ExchangeError::Io(e.kind().to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_EXCHANGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ExchangeError::Io(e.kind().to_string()))?;
    if bytes.len() as u64 > MAX_EXCHANGE_BYTES {
        return Err(ExchangeError::TooBig);
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| ExchangeError::NotExchange("not UTF-8 text".to_string()))?;
    parse_exchange(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ExchangeSettings {
        ExchangeSettings {
            theme: BrowserTheme::Nordic,
            search_engine: SearchEngine::DuckDuckGo,
            zoom_percent: 125,
            https_only_mode: true,
        }
    }

    fn bm(title: &str, url: &str) -> Bookmark {
        Bookmark {
            title: title.to_string(),
            url: url.to_string(),
        }
    }

    #[test]
    fn export_has_bookmarks_tabs_and_settings_but_no_history_by_default() {
        let text = export_json(
            &[
                bm("Docs", "https://docs.example/"),
                bm("Local", "file:///tmp/x.html"),
            ],
            &[
                "https://news.example/".to_string(),
                "anvil://settings?toggle=https_only&key=abc".to_string(),
            ],
            settings(),
            &[bm("Seen", "https://seen.example/")],
            ExportOptions::default(),
        );
        assert!(text.contains(EXCHANGE_KIND));
        assert!(text.contains("https://docs.example/"));
        assert!(text.contains("https://news.example/"));
        assert!(text.contains("\"anvil://settings\""));
        assert!(!text.contains("key=abc"));
        assert!(!text.contains("file://"));
        assert!(!text.contains("seen.example"));
        assert!(!text.contains("\"history\""));
        for private in [
            "cookie",
            "localStorage",
            "local_storage",
            "downloads",
            "logins",
        ] {
            assert!(!text.contains(private), "{private}");
        }

        let with_history = export_json(
            &[],
            &[],
            settings(),
            &[bm("Seen", "https://seen.example/")],
            ExportOptions { history: true },
        );
        assert!(with_history.contains("seen.example"));
    }

    #[test]
    fn round_trip_keeps_what_was_exported() {
        let text = export_json(
            &[bm("Docs", "https://docs.example/")],
            &["https://news.example/".to_string()],
            settings(),
            &[bm("Seen", "https://seen.example/")],
            ExportOptions { history: true },
        );
        let data = parse_exchange(&text).unwrap();
        assert_eq!(data.bookmarks, vec![bm("Docs", "https://docs.example/")]);
        assert_eq!(data.tabs, vec!["https://news.example/".to_string()]);
        assert_eq!(
            data.settings,
            Some(ImportedSettings {
                theme: Some(BrowserTheme::Nordic),
                search_engine: Some(SearchEngine::DuckDuckGo),
                zoom_percent: Some(125),
                https_only_mode: Some(true),
            })
        );
        assert_eq!(
            data.history,
            Some(vec![bm("Seen", "https://seen.example/")])
        );
        assert!(data.warnings.is_empty(), "{:?}", data.warnings);
    }

    #[test]
    fn import_strips_cookies_and_other_private_keys_with_a_warning() {
        let text = r#"{
            "kind": "anvil-exchange-v1",
            "bookmarks": [{"title": "Docs", "url": "https://docs.example/"},
                          {"title": "Bad", "url": "javascript:alert(1)"}],
            "cookies": [{"name": "sid", "value": "cookie-placeholder"}],
            "localStorage": [["https://a.example", "k", "v"]],
            "downloads": [],
            "logins": [{"host": "a.example", "password": "pass-placeholder"}],
            "mystery": 1,
            "settings": {"theme": "nordic", "jit_enabled": false}
        }"#;
        let data = parse_exchange(text).unwrap();
        assert_eq!(data.bookmarks, vec![bm("Docs", "https://docs.example/")]);
        let notes = data.warnings.join("\n");
        assert!(notes.contains("had cookies"), "{notes}");
        assert!(notes.contains("had localStorage"), "{notes}");
        assert!(notes.contains("had downloads"), "{notes}");
        assert!(notes.contains("had saved logins"), "{notes}");
        assert!(notes.contains("\"mystery\""), "{notes}");
        assert!(notes.contains("\"jit_enabled\""), "{notes}");
        assert!(
            notes.contains("1 bookmark entries were left out"),
            "{notes}"
        );
        assert!(!notes.contains("cookie-placeholder"));
        assert!(!notes.contains("pass-placeholder"));
        assert_eq!(data.history, None);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(matches!(
            parse_exchange("{\"kind\":\"anvil-profile\"}"),
            Err(ExchangeError::NotExchange(_))
        ));
        assert!(matches!(
            parse_exchange("[1,2]"),
            Err(ExchangeError::NotExchange(_))
        ));
        assert!(matches!(
            parse_exchange("not json"),
            Err(ExchangeError::NotExchange(_))
        ));
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::frontend::urlencode_component;
    use crate::permissions::action_key;
    use crate::{Session, Viewport};
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        })
    }

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anvil-exchange-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(s: &mut Session, query: &str) -> String {
        s.open_target(&format!("anvil://settings/exchange?{query}"))
            .unwrap();
        s.active().current().unwrap().source.clone()
    }

    fn with_key(query: &str) -> String {
        format!("{query}&key={}", action_key())
    }

    #[test]
    fn off_by_default_and_the_switch_needs_the_key() {
        let dir = dir("off");
        let file = dir.join("out.json");
        let mut s = session();
        assert!(!s.settings().local_exchange);
        assert_eq!(
            s.export_exchange_to_path(&file, ExportOptions::default()),
            Err(ExchangeError::Off)
        );
        assert_eq!(
            s.import_exchange_from_path(&file).unwrap_err(),
            ExchangeError::Off
        );
        let page = open(&mut s, "toggle=exchange");
        assert!(!s.settings().local_exchange);
        assert!(!page.contains("name=\"export\""));
        open(
            &mut s,
            &format!("export={}", urlencode_component(file.to_str().unwrap())),
        );
        assert!(!file.exists());

        let page = open(&mut s, &with_key("toggle=exchange"));
        assert!(s.settings().local_exchange);
        assert!(page.contains("name=\"export\""));
        // Being on writes nothing by itself.
        s.open_html("<p>hi</p>", "https://news.example/");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn export_writes_only_bookmarks_tabs_and_settings_to_the_named_file() {
        let dir = dir("export");
        let file = dir.join("out.json");
        let mut s = session();
        s.set_local_exchange(true);
        s.add_bookmark("Docs", "https://docs.example/");
        s.open_html("<p>hi</p>", "https://news.example/");
        assert!(
            s.cookie_jar_mut()
                .store_set_cookie("https://news.example/", "sid=cookie-placeholder; Path=/")
        );
        // The page below opens in its own tab, so the news tab stays.
        s.new_tab();
        let path = urlencode_component(file.to_str().unwrap());
        open(&mut s, &format!("export={path}"));
        assert!(!file.exists());
        let page = open(&mut s, &with_key(&format!("export={path}")));
        assert!(page.contains("Exported."), "{page}");

        let text = fs::read_to_string(&file).unwrap();
        let root = crate::anvil_json::parse_json(&text).unwrap();
        let keys: Vec<&str> = root
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["kind", "bookmarks", "tabs", "settings"]);
        assert!(text.contains("https://docs.example/"));
        assert!(text.contains("https://news.example/"));
        assert!(!text.contains("cookie-placeholder"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        open(&mut s, &with_key(&format!("export={path}&history=1")));
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"history\""));
    }

    #[test]
    fn import_with_a_cookies_key_strips_it_and_warns() {
        let dir = dir("import");
        let file = dir.join("in.json");
        fs::write(
            &file,
            r#"{
                "kind": "anvil-exchange-v1",
                "bookmarks": [{"title": "Docs", "url": "https://docs.example/"}],
                "tabs": ["https://news.example/", "file:///etc/passwd"],
                "settings": {"theme": "nordic", "https_only_mode": false, "zoom_percent": 150},
                "cookies": [{"domain": "news.example", "name": "sid", "value": "cookie-placeholder"}]
            }"#,
        )
        .unwrap();
        let mut s = session();
        s.set_local_exchange(true);
        s.settings.https_only_mode = true;
        let page = open(
            &mut s,
            &with_key(&format!(
                "import={}",
                urlencode_component(file.to_str().unwrap())
            )),
        );
        assert!(page.contains("Imported 1 bookmarks."), "{page}");
        assert!(page.contains("The file had cookies in it."), "{page}");
        assert!(page.contains("1 tab addresses were left out."), "{page}");
        assert!(page.contains("href=\"https://news.example/\""));
        assert!(!page.contains("file:///etc/passwd"));
        assert!(!page.contains("cookie-placeholder"));
        assert_eq!(s.bookmarks().len(), 1);
        assert_eq!(s.theme(), BrowserTheme::Nordic);
        assert_eq!(s.zoom_percent(), 150);
        // An import never turns HTTPS-only off, and brings no cookies.
        assert!(s.settings().https_only_mode);
        assert!(!s.cookie_jar().to_json().contains("cookie-placeholder"));
        // Tabs are listed, not opened.
        assert_eq!(s.tabs().len(), 1);
    }
}
