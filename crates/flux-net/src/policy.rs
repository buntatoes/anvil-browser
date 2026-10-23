//! Content Security Policy (`CSP`), Cross-Origin Resource Policy (`CORP`),
//! and hardened RFC 6265bis Cookie Jar (`HttpOnly`, `Secure`, `SameSite`, `__Host-`/`__Secure-`).

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
