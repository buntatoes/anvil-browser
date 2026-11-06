// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Saved site logins, kept in `logins.anvil` next to `profile.anvil`.
//!
//! The file is sealed the same way as the profile and with the same key
//! (Linux AES-256-GCM `ANVP\x02` under `profile.key`, Windows DPAPI
//! `ANVP\x01`), but it is its own file, so the profile never holds a
//! password. There is no extra password to unlock it.
//!
//! A login is offered for saving after you send a form with a password on
//! an `https` page to the same host, and it is only filled on `https` pages
//! of exactly that host, never over plain `http`. Passwords never go into a
//! `Debug` string, an error message, a page address, or the logins page
//! list; the page shows one only when you ask it to.

use std::fmt;
use std::path::{Path, PathBuf};

use anvil_dom::{Document, NodeId};

use crate::anvil_json::{JsonValue, parse_json, to_json_pretty};

/// File name next to `profile.anvil`.
pub const LOGINS_FILE: &str = "logins.anvil";

/// Most logins kept. Saving one more drops the oldest.
pub const MAX_LOGINS: usize = 256;

/// Most hosts on the "never save" list.
pub const MAX_NEVER: usize = 256;

/// Longest username or password kept, in bytes.
pub const MAX_FIELD_BYTES: usize = 1024;

/// One saved login.
#[derive(Clone, PartialEq, Eq)]
pub struct LoginEntry {
    /// Lowercased host it was saved for.
    pub host: String,
    /// Username, which may be empty.
    pub username: String,
    password: String,
}

impl LoginEntry {
    /// The saved password. Don't print or log it.
    pub fn password(&self) -> &str {
        &self.password
    }
}

impl fmt::Debug for LoginEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginEntry")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("password", &"(hidden)")
            .finish()
    }
}

/// A login just sent from a form, waiting for you to save it or not.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingLogin {
    /// Host of the page and of the form target.
    pub host: String,
    /// Username field, which may be empty.
    pub username: String,
    password: String,
}

impl fmt::Debug for PendingLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingLogin")
            .field("host", &self.host)
            .field("username", &self.username)
            .field("password", &"(hidden)")
            .finish()
    }
}

/// Why `logins.anvil` could not be read or written. The text never holds
/// a username or password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginsError {
    /// The file is not sealed for this system, or its seal check failed.
    Unreadable,
    /// The file opened but its contents are not a login list.
    Damaged,
    /// The folder or file could not be written or read.
    Io,
    /// There's no place for the file (no home folder).
    NoPlace,
}

impl fmt::Display for LoginsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => write!(f, "saved logins could not be opened (seal check failed)"),
            Self::Damaged => write!(f, "saved logins could not be read (file damaged)"),
            Self::Io => write!(f, "saved logins could not be read or written (file error)"),
            Self::NoPlace => write!(f, "saved logins have no place to be kept"),
        }
    }
}

impl std::error::Error for LoginsError {}

impl From<crate::profile::ProfileError> for LoginsError {
    fn from(err: crate::profile::ProfileError) -> Self {
        use crate::profile::ProfileError;
        match err {
            ProfileError::InvalidMagic
            | ProfileError::TruncatedContainer
            | ProfileError::IntegrityCheckFailed => Self::Unreadable,
            ProfileError::DecompressFailed | ProfileError::JsonError(_) => Self::Damaged,
            ProfileError::IoError(_) => Self::Io,
        }
    }
}

/// The saved logins and the hosts never to offer saving for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoginStore {
    entries: Vec<LoginEntry>,
    never: Vec<String>,
}

/// `logins.anvil` next to `profile_path`.
pub fn path_for_profile(profile_path: &Path) -> PathBuf {
    profile_path.with_file_name(LOGINS_FILE)
}

/// The host of `url` when it is `https`, lowercased. `None` for anything else.
pub fn https_host(url: &str) -> Option<String> {
    let parsed = anvil_net::parse_url(url).ok()?;
    (parsed.scheme == "https").then_some(parsed.host)
}

fn clip(text: &str) -> Option<String> {
    (text.len() <= MAX_FIELD_BYTES).then(|| text.to_string())
}

impl LoginStore {
    /// No logins.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every saved login, oldest first.
    pub fn entries(&self) -> &[LoginEntry] {
        &self.entries
    }

    /// Number of saved logins.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is saved.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hosts never to offer saving for.
    pub fn never_hosts(&self) -> &[String] {
        &self.never
    }

    /// Save or update the login for `host` and `username`. `false` when the
    /// host is not a host name, the password is empty, or a field is too long.
    pub fn save(&mut self, host: &str, username: &str, password: &str) -> bool {
        let Some(host) = crate::permissions::normalize_host(host) else {
            return false;
        };
        let (Some(username), Some(password)) = (clip(username), clip(password)) else {
            return false;
        };
        if password.is_empty() {
            return false;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.host == host && e.username == username)
        {
            entry.password = password;
            return true;
        }
        if self.entries.len() >= MAX_LOGINS {
            self.entries.remove(0);
        }
        self.entries.push(LoginEntry {
            host,
            username,
            password,
        });
        true
    }

    /// Remove the login for `host` and `username`. `true` if it was there.
    pub fn remove(&mut self, host: &str, username: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !(e.host == host && e.username == username));
        before != self.entries.len()
    }

    /// The login for exactly `host` and `username`.
    pub fn get(&self, host: &str, username: &str) -> Option<&LoginEntry> {
        self.entries
            .iter()
            .find(|e| e.host == host && e.username == username)
    }

    /// Logins saved for exactly `host`. A login for `example.com` is not
    /// offered on `www.example.com`.
    pub fn for_host(&self, host: &str) -> Vec<&LoginEntry> {
        let host = host.to_ascii_lowercase();
        self.entries.iter().filter(|e| e.host == host).collect()
    }

    /// Never offer to save for `host`, or offer again.
    pub fn set_never(&mut self, host: &str, never: bool) -> bool {
        let Some(host) = crate::permissions::normalize_host(host) else {
            return false;
        };
        self.never.retain(|h| *h != host);
        if never {
            if self.never.len() >= MAX_NEVER {
                self.never.remove(0);
            }
            self.never.push(host);
        }
        true
    }

    /// Whether saving is never offered for `host`.
    pub fn is_never(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.never.contains(&host)
    }

    /// Whether `pending` would change nothing: the same login is saved, or
    /// its host is on the never list.
    pub fn already_has(&self, pending: &PendingLogin) -> bool {
        self.is_never(&pending.host)
            || self
                .get(&pending.host, &pending.username)
                .is_some_and(|e| e.password == pending.password)
    }

    /// Save `pending`.
    pub fn save_pending(&mut self, pending: &PendingLogin) -> bool {
        self.save(&pending.host, &pending.username, &pending.password)
    }

    fn to_json(&self) -> String {
        let mut root = JsonValue::empty_object();
        root.insert("kind", JsonValue::String("anvil-logins-v1".to_string()));
        let mut list = JsonValue::empty_array();
        for entry in &self.entries {
            let mut item = JsonValue::empty_object();
            item.insert("host", JsonValue::String(entry.host.clone()));
            item.insert("username", JsonValue::String(entry.username.clone()));
            item.insert("password", JsonValue::String(entry.password.clone()));
            list.push(item);
        }
        root.insert("logins", list);
        let mut never = JsonValue::empty_array();
        for host in &self.never {
            never.push(JsonValue::String(host.clone()));
        }
        root.insert("never", never);
        to_json_pretty(&root)
    }

    fn from_json(text: &str) -> Result<Self, LoginsError> {
        let doc = parse_json(text).map_err(|_| LoginsError::Damaged)?;
        if doc.get_str("kind") != Some("anvil-logins-v1") {
            return Err(LoginsError::Damaged);
        }
        let mut out = Self::new();
        for item in doc
            .get("logins")
            .and_then(JsonValue::as_array)
            .unwrap_or(&[])
            .iter()
            .take(MAX_LOGINS)
        {
            let (Some(host), Some(password)) = (item.get_str("host"), item.get_str("password"))
            else {
                continue;
            };
            out.save(host, item.get_str("username").unwrap_or(""), password);
        }
        for host in doc
            .get("never")
            .and_then(JsonValue::as_array)
            .unwrap_or(&[])
            .iter()
            .take(MAX_NEVER)
        {
            if let Some(host) = host.as_str() {
                out.set_never(host, true);
            }
        }
        Ok(out)
    }

    /// Seal the list into `path`. See the module note.
    pub fn save_to_path(&self, path: &Path) -> Result<(), LoginsError> {
        Ok(crate::profile::seal_text_to_path(&self.to_json(), path)?)
    }

    /// Read a list sealed by [`Self::save_to_path`]. A missing file is an
    /// empty list.
    pub fn load_from_path(path: &Path) -> Result<Self, LoginsError> {
        if !path.exists() {
            return Ok(Self::new());
        }
        Self::from_json(&crate::profile::open_text_from_path(path)?)
    }
}

/// Whether `id` is an `<input type="password">`.
fn is_password(doc: &Document, id: NodeId) -> bool {
    let data = doc.get(id).data();
    data.tag() == Some("input")
        && data
            .attr("type")
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("password"))
}

/// Whether `id` is a text-like `<input>` that can hold a username.
fn is_username_field(doc: &Document, id: NodeId) -> bool {
    let data = doc.get(id).data();
    if data.tag() != Some("input") {
        return false;
    }
    let kind = data
        .attr("type")
        .unwrap_or("text")
        .trim()
        .to_ascii_lowercase();
    matches!(kind.as_str(), "" | "text" | "email" | "tel")
}

/// The username and password a form is about to send: the first password
/// field with a value, and the last text or email field before it. `None`
/// when no password field has a value.
fn form_fields(doc: &Document, form_id: NodeId) -> Option<(Option<NodeId>, NodeId)> {
    let inputs = doc.get_elements_by_tag_name(form_id, "input");
    let pos = inputs
        .iter()
        .position(|&id| is_password(doc, id) && !doc.input_value(id).is_empty())?;
    let user = inputs[..pos]
        .iter()
        .rev()
        .copied()
        .find(|&id| is_username_field(doc, id));
    Some((user, inputs[pos]))
}

/// The login a form on `page_url` is sending to `target_url`, when both are
/// `https` on the same host and the form has a filled password field.
pub fn offer_from_form(
    doc: &Document,
    form_id: NodeId,
    page_url: &str,
    target_url: &str,
) -> Option<PendingLogin> {
    let host = https_host(page_url)?;
    if https_host(target_url)? != host {
        return None;
    }
    let (user, pass) = form_fields(doc, form_id)?;
    let username = clip(&user.map(|id| doc.input_value(id)).unwrap_or_default())?;
    let password = clip(&doc.input_value(pass))?;
    Some(PendingLogin {
        host,
        username,
        password,
    })
}

/// Put `entry` into the first password field of `doc` and the text field
/// just before it. `true` when a password field was filled.
pub fn fill_document(doc: &mut Document, entry: &LoginEntry) -> bool {
    let inputs = doc.get_elements_by_tag_name(doc.body(), "input");
    let Some(pos) = inputs.iter().position(|&id| is_password(doc, id)) else {
        return false;
    };
    let pass = inputs[pos];
    let form = doc.enclosing_form(pass);
    let user = inputs[..pos]
        .iter()
        .rev()
        .copied()
        .filter(|&id| doc.enclosing_form(id) == form)
        .find(|&id| is_username_field(doc, id));
    if let Some(user) = user
        && !entry.username.is_empty()
    {
        doc.set_input_value(user, &entry.username);
    }
    doc.set_input_value(pass, &entry.password);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // Placeholders only; no real login appears in tests.
    const USER: &str = "user-placeholder";
    const PASS: &str = "pass-placeholder-1";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anvil-logins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_updates_and_hosts_match_exactly() {
        let mut store = LoginStore::new();
        assert!(store.save("https://Example.com/login", USER, PASS));
        assert!(store.save("example.com", USER, "pass-placeholder-2"));
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.for_host("example.com")[0].password(),
            "pass-placeholder-2"
        );
        assert!(store.for_host("www.example.com").is_empty());
        assert!(!store.save("example.com", USER, ""));
        assert!(!store.save("bad host", USER, PASS));
        assert!(!store.save("example.com", USER, &"x".repeat(MAX_FIELD_BYTES + 1)));
        assert!(store.remove("example.com", USER));
        assert!(store.is_empty());
    }

    #[test]
    fn debug_and_errors_never_show_the_password() {
        let mut store = LoginStore::new();
        store.save("example.com", USER, PASS);
        let text = format!("{store:?} {:?}", store.entries()[0]);
        assert!(text.contains(USER));
        assert!(!text.contains(PASS), "{text}");
        for err in [
            LoginsError::Unreadable,
            LoginsError::Damaged,
            LoginsError::Io,
            LoginsError::NoPlace,
        ] {
            assert!(!err.to_string().to_ascii_lowercase().contains("vault"));
            assert!(!err.to_string().contains("profile.anvil"));
        }
    }

    #[test]
    fn sealed_file_round_trips_and_holds_no_plain_text() {
        let dir = temp_dir("seal");
        let path = dir.join(LOGINS_FILE);
        let mut store = LoginStore::new();
        store.save("example.com", USER, PASS);
        store.set_never("nope.test", true);
        store.save_to_path(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        #[cfg(not(windows))]
        assert!(bytes.starts_with(crate::profile::PROFILE_MAGIC_V2));
        for needle in [USER, PASS, "example.com", "nope.test", "password"] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} in the sealed file"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            assert!(dir.join("profile.key").exists());
        }
        assert!(!path.with_extension("tmp").exists());
        let back = LoginStore::load_from_path(&path).unwrap();
        assert_eq!(back, store);

        // A flipped byte fails the seal check; a plain JSON file is refused.
        let mut bad = bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x40;
        std::fs::write(&path, &bad).unwrap();
        assert_eq!(
            LoginStore::load_from_path(&path),
            Err(LoginsError::Unreadable)
        );
        std::fs::write(&path, b"{\"kind\": \"anvil-logins-v1\", \"logins\": []}").unwrap();
        assert_eq!(
            LoginStore::load_from_path(&path),
            Err(LoginsError::Unreadable)
        );
        assert_eq!(
            LoginStore::load_from_path(&dir.join("missing.anvil")),
            Ok(LoginStore::new())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn login_page() -> Document {
        anvil_dom::parse_html(&format!(
            "<form id=\"f\" method=\"post\" action=\"/session\">\
             <input name=\"q\" type=\"search\" value=\"x\">\
             <input id=\"u\" name=\"user\" type=\"email\" value=\"{USER}\">\
             <input id=\"p\" name=\"pass\" type=\"password\" value=\"{PASS}\">\
             <button type=\"submit\">Go</button></form>"
        ))
    }

    #[test]
    fn offers_only_https_logins_to_the_same_host() {
        let doc = login_page();
        let form = doc.get_element_by_id("f").unwrap();
        let offer = offer_from_form(
            &doc,
            form,
            "https://example.com/login",
            "https://example.com/session",
        )
        .unwrap();
        assert_eq!(offer.host, "example.com");
        assert_eq!(offer.username, USER);
        assert!(!format!("{offer:?}").contains(PASS));
        for (page, target) in [
            ("http://example.com/login", "http://example.com/session"),
            ("https://example.com/login", "http://example.com/session"),
            ("https://example.com/login", "https://other.test/session"),
            ("http://example.com/login", "https://example.com/session"),
        ] {
            assert!(
                offer_from_form(&doc, form, page, target).is_none(),
                "{page} -> {target}"
            );
        }
        let empty = anvil_dom::parse_html(
            "<form id=\"f\"><input type=\"text\" value=\"a\"><input type=\"password\"></form>",
        );
        let form = empty.get_element_by_id("f").unwrap();
        assert!(offer_from_form(&empty, form, "https://a.test/", "https://a.test/").is_none());
    }

    #[test]
    fn fill_puts_the_login_into_the_password_form() {
        let mut store = LoginStore::new();
        store.save("example.com", USER, PASS);
        let mut doc = anvil_dom::parse_html(
            "<input id=\"other\" type=\"text\"><form><input id=\"u\" type=\"text\"><input id=\"p\" type=\"password\"></form>",
        );
        assert!(fill_document(&mut doc, store.for_host("example.com")[0]));
        let u = doc.get_element_by_id("u").unwrap();
        let p = doc.get_element_by_id("p").unwrap();
        let other = doc.get_element_by_id("other").unwrap();
        assert_eq!(doc.input_value(u), USER);
        assert_eq!(doc.input_value(p), PASS);
        assert_eq!(doc.input_value(other), "");
        let mut plain = anvil_dom::parse_html("<input type=\"text\">");
        assert!(!fill_document(&mut plain, store.for_host("example.com")[0]));
    }
}

#[cfg(test)]
mod session_tests {
    use crate::permissions::action_key;
    use crate::{ClickOutcome, Session, Viewport};
    use std::path::PathBuf;

    const FORM: &str = r#"<form method="post" action="/signin">
        <input type="text" name="user" value="user-placeholder">
        <input type="password" name="pass" value="pass-placeholder-1">
        <button type="submit">Sign in</button></form>"#;
    const EMPTY_FORM: &str = r#"<form method="post" action="/signin">
        <input type="text" name="user">
        <input type="password" name="pass">
        </form>"#;

    fn session(name: &str) -> (Session, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("anvil-login-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(super::LOGINS_FILE);
        let mut s = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(s.set_logins_path(&path).unwrap(), 0);
        (s, path)
    }

    fn submit(s: &mut Session, html: &str, url: &str) -> ClickOutcome {
        s.open_html(html, url);
        let page = s.active().current().unwrap();
        let pass = page
            .document
            .get_elements_by_tag_name(page.document.body(), "input")[1];
        s.submit_form_for(pass)
    }

    fn page_source(s: &Session) -> String {
        s.active().current().unwrap().source.clone()
    }

    fn input_values(s: &Session) -> Vec<String> {
        let page = s.active().current().unwrap();
        page.document
            .get_elements_by_tag_name(page.document.body(), "input")
            .into_iter()
            .map(|id| page.document.input_value(id))
            .collect()
    }

    fn open_logins(s: &mut Session, query: &str) {
        s.open_target(&format!("anvil://settings/logins?{query}"))
            .unwrap();
    }

    #[test]
    fn https_sign_in_is_offered_and_saved_only_with_the_key() {
        let (mut s, path) = session("save");
        let out = submit(&mut s, FORM, "https://login.example/start");
        assert!(matches!(out, ClickOutcome::SubmitPost { .. }));
        assert_eq!(
            s.pending_login(),
            Some(("login.example", "user-placeholder"))
        );
        assert!(s.pending_login_note().unwrap().contains("login.example"));

        open_logins(&mut s, "offer=save");
        assert!(s.logins().is_empty());
        assert!(!path.exists());
        assert!(s.pending_login().is_some());

        open_logins(&mut s, &format!("offer=save&key={}", action_key()));
        assert_eq!(s.logins().len(), 1);
        assert!(s.pending_login().is_none());
        assert!(path.exists());
        let html = page_source(&s);
        assert!(html.contains("login.example"));
        assert!(html.contains("user-placeholder"));
        assert!(!html.contains("pass-placeholder-1"));

        // The same login sent again isn't offered again.
        submit(&mut s, FORM, "https://login.example/start");
        assert!(s.pending_login().is_none());

        // A new session reads it back from the sealed file.
        let mut again = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(again.set_logins_path(&path).unwrap(), 1);
        assert_eq!(again.logins().entries()[0].username, "user-placeholder");
    }

    #[test]
    fn an_unreadable_file_is_kept_until_you_choose() {
        let dir = std::env::temp_dir().join(format!(
            "anvil-login-session-unreadable-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(super::LOGINS_FILE);
        let junk = b"not a sealed logins file".to_vec();
        std::fs::write(&path, &junk).unwrap();
        let mut s = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert!(s.set_logins_path(&path).is_err());
        assert!(s.logins_problem().is_some());
        let note = s.pending_login_note().unwrap();
        assert!(note.contains("ERROR ID-31"), "{note}");

        // Nothing writes over it: not a save, not a change, not a link
        // without the key.
        submit(&mut s, FORM, "https://login.example/start");
        open_logins(&mut s, &format!("offer=save&key={}", action_key()));
        assert!(page_source(&s).contains("ERROR ID-31"));
        open_logins(
            &mut s,
            &format!("offer_again=a.example&key={}", action_key()),
        );
        open_logins(&mut s, "set_aside=1");
        assert_eq!(std::fs::read(&path).unwrap(), junk);
        assert!(s.logins_problem().is_some());
        let html = page_source(&s);
        assert!(html.contains("logins-problem"), "{html}");
        assert!(html.contains("set_aside=1"));

        // Trying again leaves it as it is.
        open_logins(&mut s, &format!("retry=1&key={}", action_key()));
        assert_eq!(std::fs::read(&path).unwrap(), junk);
        assert!(s.logins_problem().is_some());

        // Setting it aside keeps the old bytes and saves a fresh list.
        open_logins(&mut s, &format!("set_aside=1&key={}", action_key()));
        assert!(s.logins_problem().is_none());
        let mut aside = path.clone().into_os_string();
        aside.push(".unreadable");
        assert_eq!(std::fs::read(&aside).unwrap(), junk);
        assert!(!page_source(&s).contains("logins-problem"));
        let mut again = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(again.set_logins_path(&path).unwrap(), s.logins().len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_http_and_other_hosts_are_never_offered() {
        let (mut s, _) = session("plain");
        submit(&mut s, FORM, "http://login.example/start");
        assert!(s.pending_login().is_none());
        let other = FORM.replace("/signin", "https://collect.example/signin");
        submit(&mut s, &other, "https://login.example/start");
        assert!(s.pending_login().is_none());
    }

    #[test]
    fn fills_only_https_pages_of_the_same_host() {
        let (mut s, _) = session("fill");
        submit(&mut s, FORM, "https://login.example/start");
        open_logins(&mut s, &format!("offer=save&key={}", action_key()));

        s.open_html(EMPTY_FORM, "http://login.example/start");
        assert!(!s.fill_saved_login());
        assert_eq!(input_values(&s), ["", ""]);

        s.open_html(EMPTY_FORM, "https://sub.login.example/start");
        assert!(!s.fill_saved_login());

        s.open_html(EMPTY_FORM, "https://login.example/start");
        assert!(s.fill_saved_login());
        assert_eq!(input_values(&s), ["user-placeholder", "pass-placeholder-1"]);
    }

    #[test]
    fn reveal_remove_and_never_need_the_key() {
        let (mut s, _) = session("manage");
        submit(&mut s, FORM, "https://login.example/start");
        open_logins(&mut s, &format!("offer=save&key={}", action_key()));

        open_logins(&mut s, "reveal=1&host=login.example&user=user-placeholder");
        assert!(!page_source(&s).contains("pass-placeholder-1"));
        open_logins(
            &mut s,
            &format!(
                "reveal=1&host=login.example&user=user-placeholder&key={}",
                action_key()
            ),
        );
        assert!(page_source(&s).contains("pass-placeholder-1"));

        open_logins(&mut s, "remove=1&host=login.example&user=user-placeholder");
        assert_eq!(s.logins().len(), 1);
        open_logins(
            &mut s,
            &format!(
                "remove=1&host=login.example&user=user-placeholder&key={}",
                action_key()
            ),
        );
        assert!(s.logins().is_empty());

        submit(&mut s, FORM, "https://login.example/start");
        open_logins(&mut s, &format!("offer=never&key={}", action_key()));
        assert!(s.logins().is_empty());
        submit(&mut s, FORM, "https://login.example/start");
        assert!(s.pending_login().is_none());
        open_logins(
            &mut s,
            &format!("offer_again=login.example&key={}", action_key()),
        );
        submit(&mut s, FORM, "https://login.example/start");
        assert!(s.pending_login().is_some());
    }
}
