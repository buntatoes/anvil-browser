// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Per-site permissions, edited on `anvil://settings/permissions`.
//!
//! A site is a host name. A rule for `example.com` also covers
//! `www.example.com` and every other host under it; when two rules match,
//! the longer host wins.
//!
//! Three things can be set per site:
//!
//! - JavaScript: follow the global switch, always allow, or always block.
//! - Cookies: allowed, or blocked. Blocked hosts never store or send cookies,
//!   and their stored cookies are removed.
//! - Plain HTTP: when HTTPS-only is on, a site can be allowed to load over
//!   plain `http`. This is the only way plain `http` is used in that mode.
//!
//! The list is saved in the profile. At most [`MAX_SITES`] sites are kept.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;

use anvil_json::JsonValue;

/// Most sites kept in the list. Adding one more drops the oldest.
pub const MAX_SITES: usize = 512;

/// What a site may do with JavaScript.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptRule {
    /// Follow the global JavaScript switch on `anvil://settings`.
    #[default]
    Default,
    /// Run scripts even when the global switch is off.
    Allow,
    /// Never run scripts on this site.
    Block,
}

impl ScriptRule {
    /// Name used in links and the profile.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Allow => "allow",
            Self::Block => "block",
        }
    }

    /// Read a name written by [`Self::as_str`].
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "default" => Some(Self::Default),
            "allow" => Some(Self::Allow),
            "block" => Some(Self::Block),
            _ => None,
        }
    }
}

/// The settings for one site.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitePermission {
    /// Lowercased host name.
    pub host: String,
    /// JavaScript rule.
    pub scripts: ScriptRule,
    /// Cookies are blocked for this site.
    pub block_cookies: bool,
    /// Plain `http` is allowed while HTTPS-only is on.
    pub allow_http: bool,
}

impl SitePermission {
    /// Whether every setting is at its default, so the entry can be dropped.
    pub fn is_default(&self) -> bool {
        self.scripts == ScriptRule::Default && !self.block_cookies && !self.allow_http
    }
}

/// The per-site list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitePermissions {
    sites: Vec<SitePermission>,
}

/// Clean up a typed site: take the host out of a URL, drop a port and a
/// leading dot, and lowercase it. `None` when what is left is not a host name.
pub fn normalize_host(input: &str) -> Option<String> {
    let mut text = input.trim();
    if let Some((scheme, rest)) = text.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return None;
        }
        text = rest;
    }
    let text = text.split(['/', '?', '#']).next().unwrap_or("");
    let text = text.rsplit_once(':').map_or(text, |(host, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            host
        } else {
            text
        }
    });
    let host = text
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with('-')
        && !host.contains("..");
    valid.then_some(host)
}

/// Whether `host` is `rule_host` or a host under it.
fn covers(rule_host: &str, host: &str) -> bool {
    host == rule_host
        || (host.len() > rule_host.len()
            && host.ends_with(rule_host)
            && host.as_bytes()[host.len() - rule_host.len() - 1] == b'.')
}

impl SitePermissions {
    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every site with a setting, in the order they were added.
    pub fn sites(&self) -> &[SitePermission] {
        &self.sites
    }

    /// Number of sites in the list.
    pub fn len(&self) -> usize {
        self.sites.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// The entry for exactly `host`, if there is one.
    pub fn get(&self, host: &str) -> Option<&SitePermission> {
        let host = normalize_host(host)?;
        self.sites.iter().find(|s| s.host == host)
    }

    /// The most specific entry that covers `host`.
    pub fn rule_for(&self, host: &str) -> Option<&SitePermission> {
        let host = host.to_ascii_lowercase();
        self.sites
            .iter()
            .filter(|s| covers(&s.host, &host))
            .max_by_key(|s| s.host.len())
    }

    /// Add `host` with default settings, so it shows on the page. Returns the
    /// cleaned host, or `None` if it is not a host name.
    pub fn add(&mut self, host: &str) -> Option<String> {
        let host = normalize_host(host)?;
        self.entry(&host);
        Some(host)
    }

    fn entry(&mut self, host: &str) -> &mut SitePermission {
        if let Some(pos) = self.sites.iter().position(|s| s.host == host) {
            return &mut self.sites[pos];
        }
        if self.sites.len() >= MAX_SITES {
            self.sites.remove(0);
        }
        self.sites.push(SitePermission {
            host: host.to_string(),
            ..SitePermission::default()
        });
        self.sites.last_mut().expect("just pushed")
    }

    /// Set the JavaScript rule for `host`. `false` if `host` is not a host name.
    pub fn set_scripts(&mut self, host: &str, rule: ScriptRule) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        self.entry(&host).scripts = rule;
        true
    }

    /// Block or allow cookies for `host`. `false` if `host` is not a host name.
    pub fn set_block_cookies(&mut self, host: &str, block: bool) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        self.entry(&host).block_cookies = block;
        true
    }

    /// Allow or stop plain `http` for `host` under HTTPS-only. `false` if
    /// `host` is not a host name.
    pub fn set_allow_http(&mut self, host: &str, allow: bool) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        self.entry(&host).allow_http = allow;
        true
    }

    /// Remove `host` from the list. `true` if it was there.
    pub fn remove(&mut self, host: &str) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        let before = self.sites.len();
        self.sites.retain(|s| s.host != host);
        before != self.sites.len()
    }

    /// Remove everything.
    pub fn clear(&mut self) {
        self.sites.clear();
    }

    /// Whether scripts run on `url`, given the global switch.
    pub fn scripts_allowed(&self, url: &str, global: bool) -> bool {
        let Some(host) = host_of(url) else {
            return global;
        };
        match self.rule_for(&host).map(|s| s.scripts) {
            Some(ScriptRule::Allow) => true,
            Some(ScriptRule::Block) => false,
            _ => global,
        }
    }

    /// Hosts whose cookies are blocked.
    pub fn cookie_blocked_hosts(&self) -> Vec<String> {
        self.sites
            .iter()
            .filter(|s| s.block_cookies)
            .map(|s| s.host.clone())
            .collect()
    }

    /// Hosts allowed to use plain `http` under HTTPS-only.
    pub fn http_allowed_hosts(&self) -> Vec<String> {
        self.sites
            .iter()
            .filter(|s| s.allow_http)
            .map(|s| s.host.clone())
            .collect()
    }

    /// The list as JSON for the profile.
    pub fn to_json(&self) -> JsonValue {
        let mut list = JsonValue::empty_array();
        for site in &self.sites {
            let mut item = JsonValue::empty_object();
            item.insert("host", JsonValue::String(site.host.clone()));
            item.insert(
                "scripts",
                JsonValue::String(site.scripts.as_str().to_string()),
            );
            item.insert("block_cookies", JsonValue::Bool(site.block_cookies));
            item.insert("allow_http", JsonValue::Bool(site.allow_http));
            list.push(item);
        }
        list
    }

    /// Read a list written by [`Self::to_json`]. Entries that do not read
    /// cleanly are skipped, and at most [`MAX_SITES`] are kept.
    pub fn from_json(value: &JsonValue) -> Self {
        let mut out = Self::new();
        for item in value.as_array().unwrap_or(&[]).iter().take(MAX_SITES) {
            let Some(host) = item.get_str("host").and_then(normalize_host) else {
                continue;
            };
            if out.sites.iter().any(|s| s.host == host) {
                continue;
            }
            out.sites.push(SitePermission {
                host,
                scripts: item
                    .get_str("scripts")
                    .and_then(ScriptRule::parse)
                    .unwrap_or_default(),
                block_cookies: item.get_bool("block_cookies").unwrap_or(false),
                allow_http: item.get_bool("allow_http").unwrap_or(false),
            });
        }
        out
    }
}

/// Host of an `http` or `https` URL, lowercased.
pub fn host_of(url: &str) -> Option<String> {
    anvil_net::parse_url(url).ok().map(|u| u.host)
}

/// A key that built-in pages put in the links that change settings.
///
/// Web pages can link to `anvil://` addresses, but they cannot read a
/// built-in page, so they never learn this key. A permission change without
/// it is ignored. The key is made fresh for each run of the browser from the
/// random seed the standard library's hash maps use.
pub fn action_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut out = String::with_capacity(32);
        for round in 0u64..2 {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u64(round);
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0),
            );
            hasher.write_u32(std::process::id());
            out.push_str(&format!("{:016x}", hasher.finish()));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_cleaned_and_rules_cover_subdomains() {
        assert_eq!(
            normalize_host("https://WWW.Example.com:8443/a?b").as_deref(),
            Some("www.example.com")
        );
        assert_eq!(
            normalize_host(" .example.com ").as_deref(),
            Some("example.com")
        );
        assert_eq!(normalize_host("bad host"), None);
        assert_eq!(normalize_host("ftp://example.com"), None);
        assert_eq!(normalize_host(""), None);

        let mut perms = SitePermissions::new();
        assert!(perms.set_scripts("example.com", ScriptRule::Block));
        assert!(perms.set_scripts("app.example.com", ScriptRule::Allow));
        assert!(!perms.scripts_allowed("https://www.example.com/", true));
        assert!(perms.scripts_allowed("https://app.example.com/x", false));
        assert!(perms.scripts_allowed("https://notexample.com/", true));
        assert!(!perms.scripts_allowed("https://other.test/", false));
    }

    #[test]
    fn lists_for_cookies_and_http_and_a_profile_round_trip() {
        let mut perms = SitePermissions::new();
        perms.set_block_cookies("ads.test", true);
        perms.set_allow_http("intranet.test", true);
        perms.add("plain.test");
        assert_eq!(perms.cookie_blocked_hosts(), vec!["ads.test".to_string()]);
        assert_eq!(
            perms.http_allowed_hosts(),
            vec!["intranet.test".to_string()]
        );
        assert!(perms.get("plain.test").unwrap().is_default());

        let back = SitePermissions::from_json(&perms.to_json());
        assert_eq!(back, perms);
        assert!(perms.remove("plain.test"));
        assert!(!perms.remove("plain.test"));
        assert_eq!(perms.len(), 2);
    }

    #[test]
    fn the_list_is_capped() {
        let mut perms = SitePermissions::new();
        for i in 0..MAX_SITES + 3 {
            perms.add(&format!("site{i}.test"));
        }
        assert_eq!(perms.len(), MAX_SITES);
        assert_eq!(perms.sites()[0].host, "site3.test");
    }

    #[test]
    fn the_action_key_is_stable_within_a_run() {
        assert_eq!(action_key(), action_key());
        assert_eq!(action_key().len(), 32);
    }
}
