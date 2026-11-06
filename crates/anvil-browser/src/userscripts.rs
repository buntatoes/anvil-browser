// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Local scripts, listed on `anvil://settings/scripts`.
//!
//! You put `.js` files in the `userscripts` folder next to `profile.anvil`.
//! Each file says where it runs with `// @match` lines, for example
//! `// @match https://example.com/*`. A new file starts off; you turn it on
//! from the page. The profile keeps only file names and on/off flags. The
//! script text stays in the folder and is read again when the page opens.
//!
//! A script that is on runs after the page's own scripts, as one more inline
//! script in the page. So it gets the page and `fetch()` and nothing else,
//! and everything the page's own scripts go through applies to it too: the
//! renderer, the window's checks on every fetch, the script size cap, the
//! fetch count and the time limit for the page, and the page's
//! Content-Security-Policy. Local scripts run only on `http` and `https`
//! pages, never on `anvil://` pages, and not when JavaScript is off for that
//! site. A file that asks for `@grant` anything but `none` is not run.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::anvil_json::JsonValue;
use crate::permissions::SitePermissions;

/// Folder next to the profile that holds the script files.
pub const USERSCRIPTS_DIR: &str = "userscripts";

/// Most script files read from the folder.
pub const MAX_USERSCRIPTS: usize = 32;

/// Largest script file read, the same cap a page script has.
pub const MAX_USERSCRIPT_BYTES: usize = anvil_script::MAX_SCRIPT_SOURCE;

/// Most `@match` lines kept for one file.
const MAX_MATCHES: usize = 32;

/// Why a file in the folder can't run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptProblem {
    /// Bigger than [`MAX_USERSCRIPT_BYTES`].
    TooBig,
    /// Not UTF-8 text, or could not be read.
    Unreadable,
    /// Has no `// @match` line.
    NoMatch,
    /// Asks for a `@grant` other than `none`.
    Grant(String),
}

impl ScriptProblem {
    /// One line for the settings page.
    pub fn describe(&self) -> String {
        match self {
            Self::TooBig => format!("too big, the limit is {} KiB", MAX_USERSCRIPT_BYTES / 1024),
            Self::Unreadable => "couldn't be read as text".to_string(),
            Self::NoMatch => "has no // @match line, so it runs nowhere".to_string(),
            Self::Grant(name) => format!("asks for @grant {name}, which isn't offered"),
        }
    }
}

/// One file from the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserScript {
    /// File name inside the folder.
    pub file: String,
    /// Turned on from the settings page.
    pub enabled: bool,
    /// The `@match` lines, as written.
    pub matches: Vec<String>,
    /// Why it can't run, if it can't.
    pub problem: Option<ScriptProblem>,
    source: String,
}

impl UserScript {
    /// Whether this file runs on pages it matches.
    pub fn runs(&self) -> bool {
        self.enabled && self.problem.is_none()
    }
}

/// A script that is on, ready to put into pages.
#[derive(Debug)]
struct Ready {
    patterns: Vec<MatchPattern>,
    source: String,
}

/// The files in the folder and which are on.
#[derive(Clone, Debug, Default)]
pub struct UserScripts {
    scripts: Vec<UserScript>,
    /// Names kept on; a file keeps its flag while it is missing.
    enabled: Vec<String>,
    ready: Arc<Vec<Ready>>,
}

impl PartialEq for UserScripts {
    fn eq(&self, other: &Self) -> bool {
        self.scripts == other.scripts && self.enabled == other.enabled
    }
}

impl Eq for UserScripts {}

/// Whether `name` is a plain file name we read: letters, digits, `.`, `_`
/// and `-`, ending in `.js`, and not hidden.
pub fn is_script_name(name: &str) -> bool {
    name.len() <= 128
        && name.ends_with(".js")
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl UserScripts {
    /// No files and nothing on.
    pub fn new() -> Self {
        Self::default()
    }

    /// The files found by the last [`Self::scan`], sorted by name.
    pub fn scripts(&self) -> &[UserScript] {
        &self.scripts
    }

    /// Names that are on, sorted.
    pub fn enabled_names(&self) -> &[String] {
        &self.enabled
    }

    /// Whether any file runs.
    pub fn any_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Turn `file` on or off. `false` when `file` is not a script name.
    pub fn set_enabled(&mut self, file: &str, on: bool) -> bool {
        if !is_script_name(file) {
            return false;
        }
        self.enabled.retain(|n| n != file);
        if on && self.enabled.len() < MAX_USERSCRIPTS * 4 {
            self.enabled.push(file.to_string());
            self.enabled.sort();
        }
        for script in &mut self.scripts {
            if script.file == file {
                script.enabled = on;
            }
        }
        self.rebuild();
        true
    }

    /// Turn everything off and forget the files.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Read the folder again. A missing folder means no files. Only plain
    /// files with a script name are read; links and folders are skipped.
    pub fn scan(&mut self, dir: &Path) {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| is_script_name(n))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names.truncate(MAX_USERSCRIPTS);
        self.scripts = names
            .into_iter()
            .map(|file| {
                let enabled = self.enabled.contains(&file);
                read_script(&dir.join(&file), file, enabled)
            })
            .collect();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let ready = self
            .scripts
            .iter()
            .filter(|s| s.runs())
            .map(|s| Ready {
                patterns: s
                    .matches
                    .iter()
                    .filter_map(|m| MatchPattern::parse(m))
                    .collect(),
                source: s.source.clone(),
            })
            .filter(|r| !r.patterns.is_empty())
            .collect();
        self.ready = Arc::new(ready);
    }

    /// What a page load needs, or `None` when no file runs. `perms` and
    /// `scripts_global` are checked again against the address the page ends
    /// up at, after redirects.
    pub fn injection(&self, perms: &SitePermissions, scripts_global: bool) -> Option<Injection> {
        self.any_ready().then(|| Injection {
            ready: Arc::clone(&self.ready),
            perms: perms.clone(),
            scripts_global,
        })
    }

    /// The on/off list for the profile: `[{"file": NAME, "enabled": true}]`.
    pub fn to_json(&self) -> JsonValue {
        let mut list = JsonValue::empty_array();
        for name in &self.enabled {
            let mut item = JsonValue::empty_object();
            item.insert("file", JsonValue::String(name.clone()));
            item.insert("enabled", JsonValue::Bool(true));
            list.push(item);
        }
        list
    }

    /// Read a list written by [`Self::to_json`]. Bad names are skipped.
    /// Call [`Self::scan`] afterwards to read the files.
    pub fn from_json(value: &JsonValue) -> Self {
        let mut out = Self::new();
        for item in value
            .as_array()
            .unwrap_or(&[])
            .iter()
            .take(MAX_USERSCRIPTS * 4)
        {
            let Some(name) = item.get_str("file") else {
                continue;
            };
            if item.get_bool("enabled").unwrap_or(false)
                && is_script_name(name)
                && !out.enabled.iter().any(|n| n == name)
            {
                out.enabled.push(name.to_string());
            }
        }
        out.enabled.sort();
        out
    }
}

/// The folder next to `profile_path`.
pub fn dir_for_profile(profile_path: &Path) -> PathBuf {
    profile_path.with_file_name(USERSCRIPTS_DIR)
}

fn read_script(path: &Path, file: String, enabled: bool) -> UserScript {
    let mut script = UserScript {
        file,
        enabled,
        matches: Vec::new(),
        problem: None,
        source: String::new(),
    };
    let too_big = fs::metadata(path).map_or(true, |m| m.len() > MAX_USERSCRIPT_BYTES as u64);
    if too_big {
        script.problem = Some(if fs::metadata(path).is_ok() {
            ScriptProblem::TooBig
        } else {
            ScriptProblem::Unreadable
        });
        return script;
    }
    let Ok(source) = fs::read_to_string(path) else {
        script.problem = Some(ScriptProblem::Unreadable);
        return script;
    };
    if source.len() > MAX_USERSCRIPT_BYTES {
        script.problem = Some(ScriptProblem::TooBig);
        return script;
    }
    let header = parse_header(&source);
    script.matches = header.matches;
    script.problem = if let Some(grant) = header.grant {
        Some(ScriptProblem::Grant(grant))
    } else if script.matches.is_empty() {
        Some(ScriptProblem::NoMatch)
    } else {
        None
    };
    script.source = source;
    script
}

/// What the `// @key value` lines at the top of a file say.
#[derive(Debug, Default, PartialEq, Eq)]
struct Header {
    matches: Vec<String>,
    /// The first `@grant` that is not `none`.
    grant: Option<String>,
}

/// Read `// @match` and `// @grant` lines from the comment lines at the top
/// of `source`. Reading stops at the first line that is not blank and not a
/// `//` comment.
fn parse_header(source: &str) -> Header {
    let mut header = Header::default();
    for line in source.lines().take(200) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(comment) = line.strip_prefix("//") else {
            break;
        };
        let Some(rest) = comment.trim().strip_prefix('@') else {
            continue;
        };
        let (key, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let value = value.trim();
        match key {
            "match" if !value.is_empty() && header.matches.len() < MAX_MATCHES => {
                header.matches.push(value.to_string());
            }
            "grant" if value != "none" && header.grant.is_none() => {
                header.grant = Some(if value.is_empty() {
                    "(blank)".to_string()
                } else {
                    value.chars().take(64).collect()
                });
            }
            _ => {}
        }
    }
    header
}

/// One `@match` line: `SCHEME://HOST/PATH`.
///
/// `SCHEME` is `http`, `https`, or `*` for both. `HOST` is a host name,
/// `*.` and a host name for it and the hosts under it, or `*` for any host.
/// `PATH` starts with `/`, and each `*` in it stands for any run of
/// characters. The query is part of the path. Other schemes never match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPattern {
    scheme: Option<String>,
    host: HostPattern,
    path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HostPattern {
    Any,
    Exact(String),
    Under(String),
}

impl MatchPattern {
    /// Read one `@match` value. `None` when it is not in the form above.
    pub fn parse(text: &str) -> Option<Self> {
        let (scheme, rest) = text.trim().split_once("://")?;
        let scheme = match scheme {
            "*" => None,
            "http" | "https" => Some(scheme.to_string()),
            _ => return None,
        };
        let slash = rest.find('/')?;
        let (host, path) = rest.split_at(slash);
        let host = if host == "*" {
            HostPattern::Any
        } else if let Some(under) = host.strip_prefix("*.") {
            HostPattern::Under(crate::permissions::normalize_host(under)?)
        } else {
            let exact = crate::permissions::normalize_host(host)?;
            if exact != host.to_ascii_lowercase() {
                return None;
            }
            HostPattern::Exact(exact)
        };
        Some(Self {
            scheme,
            host,
            path: path.to_string(),
        })
    }

    /// Whether `url` is matched. Only `http` and `https` addresses can be.
    pub fn matches(&self, url: &str) -> bool {
        let Ok(parsed) = anvil_net::parse_url(url) else {
            return false;
        };
        if parsed.scheme != "http" && parsed.scheme != "https" {
            return false;
        }
        if self.scheme.as_deref().is_some_and(|s| s != parsed.scheme) {
            return false;
        }
        let host_ok = match &self.host {
            HostPattern::Any => true,
            HostPattern::Exact(h) => parsed.host == *h,
            HostPattern::Under(h) => {
                parsed.host == *h
                    || (parsed.host.len() > h.len()
                        && parsed.host.ends_with(h.as_str())
                        && parsed.host.as_bytes()[parsed.host.len() - h.len() - 1] == b'.')
            }
        };
        if !host_ok {
            return false;
        }
        let path = match &parsed.query {
            Some(q) => format!("{}?{q}", parsed.path),
            None => parsed.path.clone(),
        };
        glob(&self.path, &path)
    }
}

/// `*` in `pattern` is any run of characters; everything else is literal.
fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let first = parts[0];
    let last = parts[parts.len() - 1];
    if !text.starts_with(first) || text.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    if !text.ends_with(last) {
        return false;
    }
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(pos) => rest = &rest[pos + part.len()..],
            None => return false,
        }
    }
    true
}

/// The scripts a page load may add, and the rules to check first.
#[derive(Clone, Debug)]
pub struct Injection {
    ready: Arc<Vec<Ready>>,
    perms: SitePermissions,
    scripts_global: bool,
}

thread_local! {
    static ACTIVE: RefCell<Option<Injection>> = const { RefCell::new(None) };
}

/// Puts the previous scripts back when dropped. See [`enter`].
#[must_use = "local scripts are only used while this is alive"]
pub struct InjectionScope {
    previous: Option<Injection>,
}

impl Drop for InjectionScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ACTIVE.with(|a| *a.borrow_mut() = previous);
    }
}

/// Use `injection` for page loads on this thread until the returned value is
/// dropped. The browser sets this around each load, next to HTTPS-only.
pub fn enter(injection: Option<Injection>) -> InjectionScope {
    let previous = ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), injection));
    InjectionScope { previous }
}

/// Add the local scripts that match `url` to the end of `html`.
///
/// Nothing is added when `scripts_enabled` is false, when no scripts were
/// set with [`enter`] on this thread, when `url` is not `http` or `https`,
/// or when JavaScript is off for the site of `url`.
pub fn inject_into(url: &str, html: String, scripts_enabled: bool) -> String {
    if !scripts_enabled {
        return html;
    }
    ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(injection) = active.as_ref() else {
            return html;
        };
        let lower = url.trim_start().to_ascii_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return html;
        }
        if !injection
            .perms
            .scripts_allowed(url, injection.scripts_global)
        {
            return html;
        }
        let tags: String = injection
            .ready
            .iter()
            .filter(|r| r.patterns.iter().any(|p| p.matches(url)))
            .map(|r| format!("<script>{}</script>", script_text(&r.source)))
            .collect();
        if tags.is_empty() {
            return html;
        }
        insert_before_body_end(html, &tags)
    })
}

/// Keep `source` from ending its `<script>` element early.
fn script_text(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(pos) = rest.find('<') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let lower: String = tail
            .chars()
            .take(8)
            .collect::<String>()
            .to_ascii_lowercase();
        if lower.starts_with("</script") {
            out.push_str("<\\/");
            rest = &tail[2..];
        } else if let Some(after) = tail.strip_prefix("<!--") {
            out.push_str("<\\!--");
            rest = after;
        } else {
            out.push('<');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

fn insert_before_body_end(mut html: String, tags: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let at = lower.rfind("</body").or_else(|| lower.rfind("</html"));
    match at {
        Some(pos) => html.insert_str(pos, tags),
        None => html.push_str(tags),
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::ScriptRule;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anvil-userscripts-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn match_patterns_cover_scheme_host_and_path() {
        let p = MatchPattern::parse("https://example.com/*").unwrap();
        assert!(p.matches("https://example.com/"));
        assert!(p.matches("https://example.com/a/b?c=1"));
        assert!(!p.matches("http://example.com/"));
        assert!(!p.matches("https://www.example.com/"));
        assert!(!p.matches("https://example.com.evil.test/"));

        let p = MatchPattern::parse("*://*.example.com/news/*").unwrap();
        assert!(p.matches("http://example.com/news/1"));
        assert!(p.matches("https://www.example.com/news/"));
        assert!(!p.matches("https://badexample.com/news/1"));
        assert!(!p.matches("https://example.com/sports"));

        let any = MatchPattern::parse("*://*/*").unwrap();
        assert!(any.matches("https://anything.test/x"));
        for url in [
            "anvil://settings",
            "anvil://home",
            "file:///tmp/a.html",
            "view-source:https://a.test/",
        ] {
            assert!(!any.matches(url), "{url} must never match");
        }

        for bad in [
            "anvil://*/*",
            "file:///*",
            "https://example.com",
            "ftp://a.test/*",
            "https://ex ample.com/*",
        ] {
            assert!(MatchPattern::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn glob_handles_stars_at_both_ends() {
        assert!(glob("/a*b*c", "/aXXbYYc"));
        assert!(!glob("/a*b*c", "/aXXc"));
        assert!(glob("*", ""));
        assert!(glob("/x", "/x"));
        assert!(!glob("/x", "/xy"));
        assert!(!glob("/ab*ba", "/aba"));
    }

    #[test]
    fn header_reads_matches_and_refuses_grants() {
        let h = parse_header(
            "// ==UserScript==\n// @name Demo\n// @match https://a.test/*\n// @grant none\n// ==/UserScript==\ncode();\n// @match https://late.test/*\n",
        );
        assert_eq!(h.matches, vec!["https://a.test/*".to_string()]);
        assert_eq!(h.grant, None);
        let h = parse_header("// @match https://a.test/*\n// @grant GM_xmlhttpRequest\n");
        assert_eq!(h.grant.as_deref(), Some("GM_xmlhttpRequest"));
    }

    #[test]
    fn scan_reads_files_and_new_ones_start_off() {
        let dir = temp_dir("scan");
        fs::write(
            dir.join("a.js"),
            "// @match https://a.test/*\nwindow.a = 1;\n",
        )
        .unwrap();
        fs::write(
            dir.join("grant.js"),
            "// @match https://a.test/*\n// @grant GM_setValue\n",
        )
        .unwrap();
        fs::write(dir.join("nomatch.js"), "window.x = 1;\n").unwrap();
        fs::write(dir.join("big.js"), "x".repeat(MAX_USERSCRIPT_BYTES + 1)).unwrap();
        fs::write(dir.join("notes.txt"), "// @match https://a.test/*\n").unwrap();
        fs::write(dir.join(".hidden.js"), "// @match https://a.test/*\n").unwrap();
        let mut scripts = UserScripts::new();
        scripts.scan(&dir);
        let names: Vec<&str> = scripts.scripts().iter().map(|s| s.file.as_str()).collect();
        assert_eq!(names, vec!["a.js", "big.js", "grant.js", "nomatch.js"]);
        assert!(scripts.scripts().iter().all(|s| !s.enabled));
        assert!(!scripts.any_ready());

        for name in ["a.js", "big.js", "grant.js", "nomatch.js"] {
            assert!(scripts.set_enabled(name, true));
        }
        let problems: Vec<Option<ScriptProblem>> = scripts
            .scripts()
            .iter()
            .map(|s| s.problem.clone())
            .collect();
        assert_eq!(
            problems,
            vec![
                None,
                Some(ScriptProblem::TooBig),
                Some(ScriptProblem::Grant("GM_setValue".to_string())),
                Some(ScriptProblem::NoMatch),
            ]
        );
        assert_eq!(scripts.ready.len(), 1, "only a.js runs");
        assert!(!scripts.set_enabled("../escape.js", true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_keeps_names_and_flags_but_not_source() {
        let dir = temp_dir("json");
        fs::write(
            dir.join("a.js"),
            "// @match https://a.test/*\nwindow.secretish = 1;\n",
        )
        .unwrap();
        let mut scripts = UserScripts::new();
        scripts.scan(&dir);
        scripts.set_enabled("a.js", true);
        let json = crate::anvil_json::to_json_pretty(&scripts.to_json());
        assert!(json.contains("a.js"));
        assert!(!json.contains("secretish"));
        let mut back = UserScripts::from_json(&crate::anvil_json::parse_json(&json).unwrap());
        assert_eq!(back.enabled_names(), ["a.js".to_string()]);
        back.scan(&dir);
        assert!(back.any_ready());
        let _ = fs::remove_dir_all(&dir);
    }

    fn ready_injection(perms: SitePermissions, global: bool) -> Injection {
        let dir = temp_dir(&format!("inject-{global}-{}", perms.len()));
        fs::write(
            dir.join("a.js"),
            "// @match *://*.a.test/*\ndocument.title = 'x</script><b>';\n",
        )
        .unwrap();
        let mut scripts = UserScripts::new();
        scripts.scan(&dir);
        scripts.set_enabled("a.js", true);
        let _ = fs::remove_dir_all(&dir);
        scripts.injection(&perms, global).unwrap()
    }

    #[test]
    fn inject_adds_matching_scripts_before_body_end() {
        let html = "<html><body><p>hi</p></body></html>".to_string();
        let _scope = enter(Some(ready_injection(SitePermissions::new(), true)));
        let out = inject_into("https://www.a.test/", html.clone(), true);
        assert!(out.contains("<script>"));
        assert!(out.ends_with("</script></body></html>"));
        assert!(out.contains("x<\\/script><b>"), "{out}");
        assert_eq!(out.matches("</script>").count(), 1);
        assert_eq!(inject_into("https://b.test/", html.clone(), true), html);
        assert_eq!(inject_into("anvil://settings", html.clone(), true), html);
        assert_eq!(
            inject_into("https://www.a.test/", html.clone(), false),
            html
        );
    }

    #[test]
    fn inject_follows_the_site_script_rules() {
        let html = "<p>hi</p>".to_string();
        let mut perms = SitePermissions::new();
        perms.set_scripts("a.test", ScriptRule::Block);
        {
            let _scope = enter(Some(ready_injection(perms, true)));
            assert_eq!(inject_into("https://www.a.test/", html.clone(), true), html);
        }
        {
            let _scope = enter(Some(ready_injection(SitePermissions::new(), false)));
            assert_eq!(inject_into("https://www.a.test/", html.clone(), true), html);
        }
        let mut perms = SitePermissions::new();
        perms.set_scripts("a.test", ScriptRule::Allow);
        let _scope = enter(Some(ready_injection(perms, false)));
        assert!(inject_into("https://www.a.test/", html.clone(), true).contains("<script>"));
    }

    #[test]
    fn scope_puts_the_previous_value_back() {
        let html = "<p>hi</p>".to_string();
        {
            let _scope = enter(Some(ready_injection(SitePermissions::new(), true)));
            {
                let _inner = enter(None);
                assert_eq!(inject_into("https://a.test/", html.clone(), true), html);
            }
            assert_ne!(inject_into("https://a.test/", html.clone(), true), html);
        }
        assert_eq!(inject_into("https://a.test/", html.clone(), true), html);
    }
}
