//! Content Security Policy, Cross-Origin Resource Policy, and a cookie jar
//! (`HttpOnly`, `Secure`, `SameSite`, `__Host-`, `__Secure-`).

use crate::{is_same_origin, parse_url, resolve_url};
use flux_json::{JsonValue, parse_json_bounded, to_json_string};

/// Maximum total cookies retained in the session jar.
pub const MAX_COOKIES_TOTAL: usize = 256;

/// Maximum cookies retained for a single host.
pub const MAX_COOKIES_PER_HOST: usize = 32;

/// Maximum byte length of a single cookie `name=value` pair.
pub const MAX_COOKIE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspDirective {
    DefaultSrc,
    ScriptSrc,
    StyleSrc,
    ImgSrc,
    MediaSrc,
    ConnectSrc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CspSource {
    None,
    SelfOrigin,
    UnsafeInline,
    Scheme(String),
    Host(String),
    WildcardSubdomain(String),
}

/// Parsed `Content-Security-Policy` (`CSP`) ruleset for a document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentSecurityPolicy {
    pub default_src: Vec<CspSource>,
    pub script_src: Vec<CspSource>,
    pub style_src: Vec<CspSource>,
    pub img_src: Vec<CspSource>,
    pub media_src: Vec<CspSource>,
    pub connect_src: Vec<CspSource>,
}

impl ContentSecurityPolicy {
    /// Parse a `Content-Security-Policy` header string into structured directives.
    pub fn parse(header: &str) -> Self {
        let mut csp = Self::default();
        for raw_directive in header.split(';') {
            let trimmed = raw_directive.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut tokens = trimmed.split_whitespace();
            let Some(name) = tokens.next() else {
                continue;
            };
            let sources: Vec<CspSource> = tokens.map(parse_csp_source).collect();
            match name.to_ascii_lowercase().as_str() {
                "default-src" => csp.default_src = sources,
                "script-src" => csp.script_src = sources,
                "style-src" => csp.style_src = sources,
                "img-src" => csp.img_src = sources,
                "media-src" => csp.media_src = sources,
                "connect-src" => csp.connect_src = sources,
                _ => {}
            }
        }
        csp
    }

    fn effective_sources(&self, directive: CspDirective) -> &[CspSource] {
        let specific = match directive {
            CspDirective::DefaultSrc => &self.default_src,
            CspDirective::ScriptSrc => &self.script_src,
            CspDirective::StyleSrc => &self.style_src,
            CspDirective::ImgSrc => &self.img_src,
            CspDirective::MediaSrc => &self.media_src,
            CspDirective::ConnectSrc => &self.connect_src,
        };
        if !specific.is_empty() {
            specific
        } else {
            &self.default_src
        }
    }

    /// Check whether inline `<script>` execution is permitted under this policy.
    pub fn allows_inline_script(&self) -> bool {
        let sources = self.effective_sources(CspDirective::ScriptSrc);
        if sources.is_empty() {
            return true;
        }
        if sources.contains(&CspSource::None) {
            return false;
        }
        sources.contains(&CspSource::UnsafeInline)
    }

    /// Check whether inline `<style>` execution is permitted under this policy.
    pub fn allows_inline_style(&self) -> bool {
        let sources = self.effective_sources(CspDirective::StyleSrc);
        if sources.is_empty() {
            return true;
        }
        if sources.contains(&CspSource::None) {
            return false;
        }
        sources.contains(&CspSource::UnsafeInline)
    }

    /// Check whether loading `target_url` from `page_url` is permitted for `directive`.
    pub fn allows_resource(
        &self,
        directive: CspDirective,
        page_url: &str,
        target_url: &str,
    ) -> bool {
        let sources = self.effective_sources(directive);
        if sources.is_empty() {
            return true;
        }
        if sources.contains(&CspSource::None) {
            return false;
        }
        let resolved = resolve_url(page_url, target_url).unwrap_or_else(|| target_url.to_string());
        for src in sources {
            match src {
                CspSource::None | CspSource::UnsafeInline => {}
                CspSource::SelfOrigin => {
                    if is_same_origin(page_url, &resolved) {
                        return true;
                    }
                }
                CspSource::Scheme(scheme) => {
                    if let Ok(parsed) = parse_url(&resolved) {
                        if parsed.scheme.eq_ignore_ascii_case(scheme) {
                            return true;
                        }
                    } else if resolved
                        .to_ascii_lowercase()
                        .starts_with(&format!("{scheme}:"))
                    {
                        return true;
                    }
                }
                CspSource::WildcardSubdomain(suffix) => {
                    if suffix == "*" {
                        return true;
                    }
                    if let Ok(parsed) = parse_url(&resolved) {
                        if parsed.host.eq_ignore_ascii_case(suffix)
                            || parsed
                                .host
                                .to_ascii_lowercase()
                                .ends_with(&format!(".{suffix}"))
                        {
                            return true;
                        }
                    }
                }
                CspSource::Host(host_expr) => {
                    if host_expr.contains("://") {
                        if is_same_origin(host_expr, &resolved) {
                            return true;
                        }
                    } else if let Ok(parsed) = parse_url(&resolved) {
                        if parsed.host.eq_ignore_ascii_case(host_expr) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn parse_csp_source(token: &str) -> CspSource {
    let clean = token.trim().to_ascii_lowercase();
    match clean.as_str() {
        "'none'" => CspSource::None,
        "'self'" => CspSource::SelfOrigin,
        "'unsafe-inline'" => CspSource::UnsafeInline,
        "*" => CspSource::WildcardSubdomain("*".to_string()),
        _ if clean.ends_with(':') && !clean.contains('/') => {
            CspSource::Scheme(clean.trim_end_matches(':').to_string())
        }
        _ if clean.starts_with("*.") => {
            CspSource::WildcardSubdomain(clean.trim_start_matches("*.").to_string())
        }
        _ => CspSource::Host(clean.trim_matches('\'').to_string()),
    }
}

/// Evaluate a response's `Cross-Origin-Resource-Policy` (`CORP`) header for subresource loads.
pub fn allows_corp_subresource(
    corp_header: Option<&str>,
    page_url: &str,
    resource_url: &str,
) -> bool {
    let Some(raw) = corp_header else {
        return true;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "same-origin" => is_same_origin(page_url, resource_url),
        "same-site" => is_same_site(page_url, resource_url),
        "cross-origin" | "" => true,
        _ => false,
    }
}

/// Check whether two URLs share the same registrable site (`scheme` + registrable domain).
pub fn is_same_site(a_url: &str, b_url: &str) -> bool {
    if is_same_origin(a_url, b_url) {
        return true;
    }
    let (Ok(a), Ok(b)) = (parse_url(a_url), parse_url(b_url)) else {
        return false;
    };
    if a.scheme != b.scheme {
        return false;
    }
    registrable_domain(&a.host) == registrable_domain(&b.host)
}

fn registrable_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]).to_ascii_lowercase()
    } else {
        host.to_ascii_lowercase()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SameSite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "Strict",
            Self::Lax => "Lax",
            Self::None => "None",
        }
    }

    pub fn from_str_attr(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "strict" => Self::Strict,
            "none" => Self::None,
            _ => Self::Lax,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

/// Hardened RFC 6265bis Cookie Jar enforcing `HttpOnly`, `Secure`, `SameSite`,
/// and `__Host-` / `__Secure-` cookie prefixes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self {
            cookies: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }

    pub fn cookies(&self) -> &[Cookie] {
        &self.cookies
    }

    /// Parse and store a `Set-Cookie` header received from `request_url`.
    /// Rejects insecure `SameSite=None`, forged domains, control characters, and violated
    /// `__Host-` / `__Secure-` prefix invariants.
    pub fn store_set_cookie(&mut self, request_url: &str, header_value: &str) -> bool {
        if header_value.len() > MAX_COOKIE_BYTES
            || header_value.bytes().any(|b| b < 0x20 || b == 0x7F)
        {
            return false;
        }
        let Ok(req) = parse_url(request_url) else {
            return false;
        };
        let is_https = req.scheme == "https";

        let mut parts = header_value.split(';');
        let Some(name_val) = parts.next() else {
            return false;
        };
        let Some((raw_name, raw_val)) = name_val.split_once('=') else {
            return false;
        };
        let name = raw_name.trim();
        let value = raw_val.trim();
        if name.is_empty()
            || name
                .bytes()
                .any(|b| matches!(b, b' ' | b'\t' | b';' | b',' | b'='))
        {
            return false;
        }

        let mut domain_attr: Option<String> = None;
        let mut path_attr: Option<String> = None;
        let mut secure = false;
        let mut http_only = false;
        let mut same_site = SameSite::Lax;
        let mut expired = false;

        for attr in parts {
            let trimmed = attr.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim().to_ascii_lowercase();
                let val = v.trim();
                match key.as_str() {
                    "domain" => {
                        let clean_dom = val.trim_start_matches('.').to_ascii_lowercase();
                        if !clean_dom.is_empty() {
                            domain_attr = Some(clean_dom);
                        }
                    }
                    "path" => {
                        if val.starts_with('/') {
                            path_attr = Some(val.to_string());
                        }
                    }
                    "samesite" => {
                        same_site = SameSite::from_str_attr(val);
                    }
                    "max-age" => {
                        if let Ok(seconds) = val.parse::<i64>() {
                            if seconds <= 0 {
                                expired = true;
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                match trimmed.to_ascii_lowercase().as_str() {
                    "secure" => secure = true,
                    "httponly" => http_only = true,
                    _ => {}
                }
            }
        }

        // Enforce Secure transport for Secure cookies and SameSite=None
        if secure && !is_https {
            return false;
        }
        if same_site == SameSite::None && !secure {
            return false;
        }

        // Enforce __Secure- and __Host- cookie prefix rules (RFC 6265bis)
        if name.starts_with("__Secure-") && !(secure && is_https) {
            return false;
        }
        if name.starts_with("__Host-") {
            if !secure || !is_https || domain_attr.is_some() || path_attr.as_deref() != Some("/") {
                return false;
            }
        }

        let (domain, host_only) = match domain_attr {
            Some(dom) => {
                if !domain_matches(&req.host, &dom) || !dom.contains('.') {
                    return false;
                }
                (dom, false)
            }
            None => (req.host.clone(), true),
        };
        let path = path_attr.unwrap_or_else(|| "/".to_string());

        if expired {
            self.cookies
                .retain(|c| !(c.name == name && c.domain == domain && c.path == path));
            return true;
        }

        if let Some(existing) = self
            .cookies
            .iter_mut()
            .find(|c| c.name == name && c.domain == domain && c.path == path)
        {
            if existing.secure && !is_https {
                return false;
            }
            existing.value = value.to_string();
            existing.secure = secure;
            existing.http_only = http_only;
            existing.same_site = same_site;
            existing.host_only = host_only;
            return true;
        }

        let host_count = self.cookies.iter().filter(|c| c.domain == domain).count();
        if host_count >= MAX_COOKIES_PER_HOST {
            if let Some(pos) = self.cookies.iter().position(|c| c.domain == domain) {
                self.cookies.remove(pos);
            }
        }
        if self.cookies.len() >= MAX_COOKIES_TOTAL {
            self.cookies.remove(0);
        }

        self.cookies.push(Cookie {
            name: name.to_string(),
            value: value.to_string(),
            domain,
            host_only,
            path,
            secure,
            http_only,
            same_site,
        });
        true
    }

    /// Format the outgoing `Cookie` HTTP header for `target_url` initiated from `initiator_url`.
    /// Includes `HttpOnly` cookies (since this is for the network layer) while enforcing
    /// `Secure`, `Domain`, `Path`, and `SameSite` rules.
    pub fn http_cookie_header(
        &self,
        target_url: &str,
        initiator_url: Option<&str>,
        is_top_level_get: bool,
    ) -> Option<String> {
        self.matching_cookie_string(target_url, initiator_url, is_top_level_get, true)
    }

    /// Format the script-visible `document.cookie` string for `page_url`.
    /// Strictly excludes all `HttpOnly` cookies.
    pub fn script_cookie_string(&self, page_url: &str) -> String {
        self.matching_cookie_string(page_url, Some(page_url), true, false)
            .unwrap_or_default()
    }

    fn matching_cookie_string(
        &self,
        target_url: &str,
        initiator_url: Option<&str>,
        is_top_level_get: bool,
        include_http_only: bool,
    ) -> Option<String> {
        let Ok(target) = parse_url(target_url) else {
            return None;
        };
        let is_https = target.scheme == "https";
        let same_site_req = match initiator_url {
            Some(init) => is_same_site(init, target_url),
            None => true,
        };

        let mut pairs = Vec::new();
        for cookie in &self.cookies {
            if !include_http_only && cookie.http_only {
                continue;
            }
            if cookie.secure && !is_https {
                continue;
            }
            if cookie.host_only {
                if !target.host.eq_ignore_ascii_case(&cookie.domain) {
                    continue;
                }
            } else if !domain_matches(&target.host, &cookie.domain) {
                continue;
            }
            if !path_matches(&target.path, &cookie.path) {
                continue;
            }
            match cookie.same_site {
                SameSite::Strict if !same_site_req => continue,
                SameSite::Lax if !same_site_req && !is_top_level_get => continue,
                SameSite::None if !is_https => continue,
                _ => {}
            }
            pairs.push(format!("{}={}", cookie.name, cookie.value));
        }

        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    /// Serialize the Cookie Jar into a `flux_json` string.
    pub fn to_json(&self) -> String {
        let mut root = JsonValue::empty_object();
        let mut list = JsonValue::empty_array();
        for c in &self.cookies {
            let mut item = JsonValue::empty_object();
            item.insert("name", JsonValue::String(c.name.clone()));
            item.insert("value", JsonValue::String(c.value.clone()));
            item.insert("domain", JsonValue::String(c.domain.clone()));
            item.insert("host_only", JsonValue::Bool(c.host_only));
            item.insert("path", JsonValue::String(c.path.clone()));
            item.insert("secure", JsonValue::Bool(c.secure));
            item.insert("http_only", JsonValue::Bool(c.http_only));
            item.insert(
                "same_site",
                JsonValue::String(c.same_site.as_str().to_string()),
            );
            list.push(item);
        }
        root.insert("cookies", list);
        to_json_string(&root)
    }

    /// Deserialize a `CookieJar` from bounded JSON.
    pub fn from_json(source: &str) -> Result<Self, String> {
        let doc = parse_json_bounded(source, 16, 1024 * 1024).map_err(|e| e.to_string())?;
        let items = doc.get_array("cookies").ok_or("missing cookies array")?;
        let mut jar = Self::new();
        for item in items.iter().take(MAX_COOKIES_TOTAL) {
            let Some(name) = item.get_str("name") else {
                continue;
            };
            let Some(value) = item.get_str("value") else {
                continue;
            };
            let Some(domain) = item.get_str("domain") else {
                continue;
            };
            let host_only = item.get_bool("host_only").unwrap_or(true);
            let path = item.get_str("path").unwrap_or("/").to_string();
            let secure = item.get_bool("secure").unwrap_or(false);
            let http_only = item.get_bool("http_only").unwrap_or(false);
            let same_site = item
                .get_str("same_site")
                .map(SameSite::from_str_attr)
                .unwrap_or(SameSite::Lax);
            jar.cookies.push(Cookie {
                name: name.to_string(),
                value: value.to_string(),
                domain: domain.to_string(),
                host_only,
                path,
                secure,
                http_only,
                same_site,
            });
        }
        Ok(jar)
    }
}

fn domain_matches(host: &str, cookie_domain: &str) -> bool {
    let host_low = host.to_ascii_lowercase();
    let dom_low = cookie_domain.to_ascii_lowercase();
    host_low == dom_low || host_low.ends_with(&format!(".{dom_low}"))
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if req_path == cookie_path {
        return true;
    }
    if req_path.starts_with(cookie_path) {
        return cookie_path.ends_with('/') || req_path[cookie_path.len()..].starts_with('/');
    }
    false
}

/// Maximum number of learned HSTS host entries retained in [`HstsStore`].
pub const MAX_HSTS_HOSTS: usize = 512;

const PRELOADED_HSTS_DOMAINS: &[&str] = &[
    "github.com",
    "githubusercontent.com",
    "wikipedia.org",
    "eff.org",
    "cloudflare.com",
    "duckduckgo.com",
    "proton.me",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HstsEntry {
    pub host: String,
    pub include_subdomains: bool,
    pub max_age: u64,
}

/// RFC 6797 HTTP Strict-Transport-Security (`HSTS`) policy store with built-in preload domains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HstsStore {
    entries: Vec<HstsEntry>,
}

impl HstsStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> &[HstsEntry] {
        &self.entries
    }

    /// Parse and record a `Strict-Transport-Security` header received over an `https://` connection.
    pub fn record_hsts_header(&mut self, request_url: &str, header_value: &str) -> bool {
        let Ok(parsed) = parse_url(request_url) else {
            return false;
        };
        // RFC 6797 Section 8.1: HSTS headers over insecure HTTP must be ignored
        if parsed.scheme != "https" {
            return false;
        }
        let mut max_age: Option<u64> = None;
        let mut include_subdomains = false;

        for directive in header_value.split(';') {
            let trimmed = directive.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("max-age") {
                    let clean_v = v.trim().trim_matches('"');
                    if let Ok(seconds) = clean_v.parse::<u64>() {
                        max_age = Some(seconds);
                    }
                }
            } else if trimmed.eq_ignore_ascii_case("includesubdomains") {
                include_subdomains = true;
            }
        }

        let Some(age) = max_age else {
            return false;
        };
        let host = parsed.host.to_ascii_lowercase();
        if age == 0 {
            self.entries.retain(|e| e.host != host);
            return true;
        }

        if let Some(existing) = self.entries.iter_mut().find(|e| e.host == host) {
            existing.max_age = age;
            existing.include_subdomains = include_subdomains;
            return true;
        }

        if self.entries.len() >= MAX_HSTS_HOSTS {
            self.entries.remove(0);
        }
        self.entries.push(HstsEntry {
            host,
            include_subdomains,
            max_age: age,
        });
        true
    }

    /// Check whether `host` requires HTTPS via the built-in preload list or a learned HSTS policy.
    pub fn requires_https(&self, host: &str) -> bool {
        let clean = host.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return false;
        }
        for &preload in PRELOADED_HSTS_DOMAINS {
            if clean == preload || clean.ends_with(&format!(".{preload}")) {
                return true;
            }
        }
        for entry in &self.entries {
            if clean == entry.host {
                return true;
            }
            if entry.include_subdomains && clean.ends_with(&format!(".{}", entry.host)) {
                return true;
            }
        }
        false
    }

    /// Automatically rewrite `http://` URLs to `https://` when the target host matches an HSTS policy.
    pub fn upgrade_url_if_hsts(&self, url: &str) -> String {
        let trimmed = url.trim();
        let Some(rest) = trimmed.strip_prefix("http://") else {
            return trimmed.to_string();
        };
        let Ok(parsed) = parse_url(trimmed) else {
            return trimmed.to_string();
        };
        if self.requires_https(&parsed.host) {
            if parsed.port == 80 {
                format!("https://{rest}")
            } else {
                trimmed.to_string()
            }
        } else {
            trimmed.to_string()
        }
    }

    pub fn to_json(&self) -> String {
        let mut root = JsonValue::empty_object();
        let mut list = JsonValue::empty_array();
        for entry in &self.entries {
            let mut item = JsonValue::empty_object();
            item.insert("host", JsonValue::String(entry.host.clone()));
            item.insert(
                "include_subdomains",
                JsonValue::Bool(entry.include_subdomains),
            );
            item.insert("max_age", JsonValue::Number(entry.max_age as f64));
            list.push(item);
        }
        root.insert("hsts", list);
        to_json_string(&root)
    }

    pub fn from_json(source: &str) -> Result<Self, String> {
        let doc = parse_json_bounded(source, 16, 1024 * 1024).map_err(|e| e.to_string())?;
        let items = doc.get_array("hsts").ok_or("missing hsts array")?;
        let mut store = Self::new();
        for item in items.iter().take(MAX_HSTS_HOSTS) {
            let Some(host) = item.get_str("host") else {
                continue;
            };
            let include_subdomains = item.get_bool("include_subdomains").unwrap_or(false);
            let max_age = item.get_i64("max_age").unwrap_or(31_536_000).max(1) as u64;
            store.entries.push(HstsEntry {
                host: host.to_ascii_lowercase(),
                include_subdomains,
                max_age,
            });
        }
        Ok(store)
    }
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Compute a FIPS 180-4 SHA-256 digest over `data`.
pub fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word_bytes) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word_bytes[0], word_bytes[1], word_bytes[2], word_bytes[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Compute a W3C Subresource Integrity (`SRI`) `sha256-<base64>` token for `payload`.
pub fn sri_sha256_token(payload: &[u8]) -> String {
    let digest = sha256_digest(payload);
    format!("sha256-{}", base64_encode(&digest))
}

/// Verify a W3C Subresource Integrity (`integrity="sha256-..."`) attribute against `payload`.
/// Returns `true` if `integrity_attr` is `None`/empty, or if at least one `sha256-` digest matches.
pub fn verify_subresource_integrity(integrity_attr: Option<&str>, payload: &[u8]) -> bool {
    let Some(raw) = integrity_attr else {
        return true;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    let sha256_tokens: Vec<&str> = trimmed
        .split_whitespace()
        .filter(|tok| tok.starts_with("sha256-"))
        .collect();
    if sha256_tokens.is_empty() {
        return false;
    }
    let expected = sri_sha256_token(payload);
    sha256_tokens.contains(&expected.as_str())
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
