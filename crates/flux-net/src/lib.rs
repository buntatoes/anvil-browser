//! URLs, HTTP, HTTPS, redirect resolution, and network security sandboxing.

pub mod policy;
pub mod shield;
#[cfg(any(windows, target_os = "linux"))]
mod tls;

pub use policy::{
    ContentSecurityPolicy, Cookie, CookieJar, CspDirective, CspSource, MAX_COOKIE_BYTES,
    MAX_COOKIES_PER_HOST, MAX_COOKIES_TOTAL, SameSite, allows_corp_subresource, is_same_site,
};
pub use shield::{NetworkShield, ShieldBlockReason, is_url_blocked_by_default_shield};

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Maximum raw HTTP response bytes accepted over the wire (8 MiB).
pub const MAX_RESPONSE: usize = 8 * 1024 * 1024;

/// Maximum decompressed HTTP response body bytes (16 MiB) to defeat zip bombs.
pub const MAX_DECOMPRESSED_BODY: usize = 16 * 1024 * 1024;

/// Maximum HTTP response header block size (64 KiB).
pub const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Maximum number of HTTP response headers accepted.
pub const MAX_HEADER_COUNT: usize = 128;

/// Maximum HTTP redirect hops followed per request.
pub const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: Option<String>,
}

impl Url {
    pub fn default_port(scheme: &str) -> Option<u16> {
        match scheme {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        }
    }

    pub fn authority(&self) -> String {
        if Self::default_port(&self.scheme) == Some(self.port) {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

#[derive(Debug)]
pub enum NetError {
    UnsupportedScheme,
    InvalidUrl,
    InvalidResponse,
    TooManyRedirects,
    InsecureRedirectDowngrade,
    BlockedByShield(String),
    BodyTooLarge,
    Io(std::io::Error),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme => write!(f, "only http and https URLs can be fetched"),
            Self::InvalidUrl => write!(f, "the URL could not be read"),
            Self::InvalidResponse => write!(f, "the response could not be read"),
            Self::TooManyRedirects => write!(f, "the page redirected too many times"),
            Self::InsecureRedirectDowngrade => {
                write!(f, "blocked insecure redirect from https to http")
            }
            Self::BlockedByShield(reason) => write!(f, "blocked by Flux Shield: {reason}"),
            Self::BodyTooLarge => write!(f, "the response exceeds safety size limits"),
            Self::Io(err) => write!(f, "the connection failed: {err}"),
        }
    }
}

impl std::error::Error for NetError {}

pub fn parse_url(input: &str) -> Result<Url, NetError> {
    let input = input.trim();
    if input.bytes().any(|b| b < 0x20 || b == 0x7F || b == b' ') {
        return Err(NetError::InvalidUrl);
    }
    let (scheme, rest) = input.split_once("://").ok_or(NetError::InvalidUrl)?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = Url::default_port(&scheme).ok_or(NetError::UnsupportedScheme)?;
    if rest.is_empty() {
        return Err(NetError::InvalidUrl);
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path_and_query) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    // Reject empty authority, IPv6 brackets, or userinfo (@) credential phishing URLs
    if authority.is_empty() || authority.starts_with('[') || authority.contains('@') {
        return Err(NetError::InvalidUrl);
    }
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        if host.is_empty() {
            return Err(NetError::InvalidUrl);
        }
        let port: u16 = port.parse().map_err(|_| NetError::InvalidUrl)?;
        if port == 0 {
            return Err(NetError::InvalidUrl);
        }
        (host.to_ascii_lowercase(), port)
    } else {
        (authority.to_ascii_lowercase(), default_port)
    };
    if !is_valid_hostname(&host) {
        return Err(NetError::InvalidUrl);
    }
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (empty_as_root(path), Some(query.to_string())),
        None => (empty_as_root(path_and_query), None),
    };
    Ok(Url {
        scheme,
        host,
        port,
        path,
        query,
    })
}

fn is_valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.starts_with('-')
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
}

/// Verify whether `base` and `target` share the same security origin `(scheme, host, port)`.
pub fn is_same_origin(base: &str, target: &str) -> bool {
    let base = base.trim();
    let target = target.trim();
    if target.is_empty() {
        return false;
    }
    let Some(resolved) = resolve_url(base, target) else {
        return false;
    };
    if base.starts_with("http://") || base.starts_with("https://") {
        let (Ok(a), Ok(b)) = (parse_url(base), parse_url(&resolved)) else {
            return false;
        };
        a.scheme == b.scheme && a.host.eq_ignore_ascii_case(&b.host) && a.port == b.port
    } else if base.starts_with("flux://") {
        resolved.starts_with("flux://")
    } else {
        !resolved.contains("://") && !has_scheme(&resolved)
    }
}

fn empty_as_root(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

/// Resolve a link reference against a page URL or a relative file path.
pub fn resolve_url(base: &str, reference: &str) -> Option<String> {
    let reference = reference.trim();
    if reference.is_empty()
        || reference.starts_with('#')
        || reference.bytes().any(|b| b < 0x20 || b == 0x7F)
    {
        return None;
    }
    let reference = reference.split('#').next().unwrap_or("").trim();
    if reference.is_empty() {
        return None;
    }
    if let Some(rest) = reference.strip_prefix("//") {
        let base_url = parse_url(base).ok()?;
        let candidate = format!("{}://{rest}", base_url.scheme);
        parse_url(&candidate).ok()?;
        return Some(candidate);
    }
    if reference.starts_with("flux://") {
        // Only internal flux:// pages may navigate to other flux:// targets via links/forms
        if base.starts_with("flux://") {
            return Some(reference.to_string());
        }
        return None;
    }
    if reference.contains("://") {
        parse_url(reference).ok()?;
        return Some(reference.to_string());
    }
    if has_scheme(reference) {
        return None;
    }
    if base.starts_with("flux://") {
        return Some(format!("flux://{}", reference.trim_start_matches('/')));
    }
    if base.starts_with("http://") || base.starts_with("https://") {
        let base_url = parse_url(base).ok()?;
        let authority = base_url.authority();
        let (ref_path, query) = match reference.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (reference, None),
        };
        let combined = if ref_path.starts_with('/') {
            ref_path.to_string()
        } else {
            let dir = match base_url.path.rfind('/') {
                Some(index) => &base_url.path[..=index],
                None => "/",
            };
            format!("{dir}{ref_path}")
        };
        let normalized = normalize_http_path(&combined);
        return Some(match query {
            Some(query) => format!("{authority}{normalized}?{query}"),
            None => format!("{authority}{normalized}"),
        });
    }
    resolve_file_path(base, reference)
}

fn has_scheme(reference: &str) -> bool {
    let Some((prefix, _)) = reference.split_once(':') else {
        return false;
    };
    !prefix.is_empty()
        && !prefix.contains('/')
        && !prefix.contains('\\')
        && prefix
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn normalize_http_path(path: &str) -> String {
    let mut stack = Vec::new();
    let trailing_slash = path.ends_with('/') && path.len() > 1;
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            segment => stack.push(segment),
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else if trailing_slash {
        format!("/{}/", stack.join("/"))
    } else {
        format!("/{}", stack.join("/"))
    }
}

fn resolve_file_path(base: &str, reference: &str) -> Option<String> {
    let ref_path = reference.split('?').next().unwrap_or("").trim();
    if ref_path.is_empty()
        || ref_path.starts_with('/')
        || ref_path.starts_with('\\')
        || ref_path.contains(':')
    {
        return None;
    }
    let base_norm = base.replace('\\', "/");
    let mut stack: Vec<&str> = match base_norm.rfind('/') {
        Some(index) => base_norm[..index]
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect(),
        None => Vec::new(),
    };
    for part in ref_path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            segment => stack.push(segment),
        }
    }
    if stack.is_empty() {
        None
    } else {
        Some(stack.join("/"))
    }
}

pub fn parse_http_response(bytes: &[u8]) -> Result<Response, NetError> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(NetError::InvalidResponse)?;
    if split > MAX_HEADER_BYTES {
        return Err(NetError::InvalidResponse);
    }
    let header_text =
        std::str::from_utf8(&bytes[..split]).map_err(|_| NetError::InvalidResponse)?;
    let raw_body = &bytes[split + 4..];
    let mut lines = header_text.lines();
    let status_line = lines.next().ok_or(NetError::InvalidResponse)?;
    let mut parts = status_line.split_whitespace();
    let _version = parts.next().ok_or(NetError::InvalidResponse)?;
    let status = parts
        .next()
        .ok_or(NetError::InvalidResponse)?
        .parse()
        .map_err(|_| NetError::InvalidResponse)?;
    let reason = parts.collect::<Vec<_>>().join(" ");
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() >= MAX_HEADER_COUNT {
            return Err(NetError::InvalidResponse);
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let raw_decoded = if is_chunked(&headers) {
        decode_chunked(raw_body)?
    } else {
        let mut body = raw_body.to_vec();
        if let Some(length) = content_length(&headers) {
            if length > MAX_RESPONSE {
                return Err(NetError::BodyTooLarge);
            }
            if body.len() > length {
                body.truncate(length);
            }
        }
        if body.len() > MAX_RESPONSE {
            return Err(NetError::BodyTooLarge);
        }
        body
    };
    let body = decode_content_encoding(&headers, raw_decoded)?;
    Ok(Response {
        status,
        reason,
        headers,
        body,
    })
}

fn decode_content_encoding(
    headers: &[(String, String)],
    body: Vec<u8>,
) -> Result<Vec<u8>, NetError> {
    let Some(enc) = headers
        .iter()
        .find(|(name, _)| name == "content-encoding")
        .map(|(_, val)| val.trim().to_ascii_lowercase())
    else {
        return Ok(body);
    };
    match enc.as_str() {
        "" | "identity" => Ok(body),
        "gzip" | "x-gzip" => flux_compress::decompress_gzip_bounded(&body, MAX_DECOMPRESSED_BODY)
            .map_err(|err| match err {
                flux_compress::CompressError::OutputTooLarge { .. } => NetError::BodyTooLarge,
                _ => NetError::InvalidResponse,
            }),
        "deflate" => flux_compress::decompress_zlib_bounded(&body, MAX_DECOMPRESSED_BODY)
            .or_else(|_| flux_compress::decompress_deflate_bounded(&body, MAX_DECOMPRESSED_BODY))
            .map_err(|err| match err {
                flux_compress::CompressError::OutputTooLarge { .. } => NetError::BodyTooLarge,
                _ => NetError::InvalidResponse,
            }),
        _ => Ok(body),
    }
}

fn content_length(headers: &[(String, String)]) -> Option<usize> {
    headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
}

fn is_chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name == "transfer-encoding"
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, NetError> {
    let mut out = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(NetError::InvalidResponse)?;
        let size_line =
            std::str::from_utf8(&input[..line_end]).map_err(|_| NetError::InvalidResponse)?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let chunk_len =
            usize::from_str_radix(size_hex, 16).map_err(|_| NetError::InvalidResponse)?;
        input = &input[line_end + 2..];
        if chunk_len == 0 {
            break;
        }
        if input.len() < chunk_len + 2 {
            return Err(NetError::InvalidResponse);
        }
        if out.len() + chunk_len > MAX_RESPONSE {
            return Err(NetError::BodyTooLarge);
        }
        out.extend_from_slice(&input[..chunk_len]);
        if &input[chunk_len..chunk_len + 2] != b"\r\n" {
            return Err(NetError::InvalidResponse);
        }
        input = &input[chunk_len + 2..];
    }
    Ok(out)
}

pub fn http_get(url: &Url) -> Result<Response, NetError> {
    http_exchange(url, build_request_with_cookie(url, None).as_bytes())
}

pub fn http_get_with_cookie(url: &Url, cookie_header: Option<&str>) -> Result<Response, NetError> {
    http_exchange(
        url,
        build_request_with_cookie(url, cookie_header).as_bytes(),
    )
}

pub fn http_post(url: &Url, content_type: &str, body: &[u8]) -> Result<Response, NetError> {
    let request = build_post_request_with_cookie(url, content_type, body, None);
    http_exchange(url, &request)
}

pub fn http_post_with_cookie(
    url: &Url,
    content_type: &str,
    body: &[u8],
    cookie_header: Option<&str>,
) -> Result<Response, NetError> {
    let request = build_post_request_with_cookie(url, content_type, body, cookie_header);
    http_exchange(url, &request)
}

fn http_exchange(url: &Url, request: &[u8]) -> Result<Response, NetError> {
    if url.scheme != "http" && url.scheme != "https" {
        return Err(NetError::UnsupportedScheme);
    }
    let mut stream =
        TcpStream::connect(format!("{}:{}", url.host, url.port)).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;

    if url.scheme == "https" {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let plain = tls::https_exchange(stream, &url.host, request, MAX_RESPONSE)?;
            return parse_http_response(&plain);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            return Err(NetError::UnsupportedScheme);
        }
    }

    stream.write_all(request).map_err(NetError::Io)?;

    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(size) => {
                if buf.len() + size > MAX_RESPONSE {
                    return Err(NetError::BodyTooLarge);
                }
                buf.extend_from_slice(&chunk[..size]);
            }
            Err(err)
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
            {
                if buf.is_empty() {
                    return Err(NetError::Io(err));
                }
                break;
            }
            Err(err) => return Err(NetError::Io(err)),
        }
    }
    parse_http_response(&buf)
}

/// Fetch an `http://` or `https://` URL, following up to five HTTP redirects.
/// Blocks HTTPS -> HTTP protocol downgrades. Returns the final URL and response.
pub fn fetch(url: &str) -> Result<(String, Response), NetError> {
    let mut ephemeral_jar = CookieJar::new();
    fetch_with_cookies(url, &mut ephemeral_jar, None, true)
}

/// Fetch an `http://` or `https://` URL with RFC 6265bis `CookieJar` attachment and
/// `Set-Cookie` capture across up to five HTTP redirects.
pub fn fetch_with_cookies(
    url: &str,
    jar: &mut CookieJar,
    initiator_url: Option<&str>,
    is_top_level_get: bool,
) -> Result<(String, Response), NetError> {
    let mut current = url.trim().to_string();
    for _ in 0..=MAX_REDIRECTS {
        let parsed = parse_url(&current)?;
        if let Some(reason) = is_url_blocked_by_default_shield(&current, initiator_url) {
            return Err(NetError::BlockedByShield(reason.to_string()));
        }
        let cookie_hdr = jar.http_cookie_header(&current, initiator_url, is_top_level_get);
        let response = http_get_with_cookie(&parsed, cookie_hdr.as_deref())?;
        for set_cookie in response.headers_named("set-cookie") {
            jar.store_set_cookie(&current, set_cookie);
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .header("location")
                .ok_or(NetError::InvalidResponse)?;
            let next = resolve_url(&current, location).ok_or(NetError::InvalidUrl)?;
            let next_parsed = parse_url(&next)?;
            if parsed.scheme == "https" && next_parsed.scheme == "http" {
                return Err(NetError::InsecureRedirectDowngrade);
            }
            current = next;
            continue;
        }
        return Ok((current, response));
    }
    Err(NetError::TooManyRedirects)
}

/// Submit an HTTP `POST` request to an `http://` or `https://` URL, following up
/// to five redirects (`301`/`302`/`303` switch to `GET`; `307`/`308` preserve `POST`).
/// Blocks HTTPS -> HTTP protocol downgrades.
pub fn fetch_post(
    url: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(String, Response), NetError> {
    let mut ephemeral_jar = CookieJar::new();
    fetch_post_with_cookies(url, content_type, body, &mut ephemeral_jar, None)
}

/// Submit an HTTP `POST` request with `CookieJar` attachment and `Set-Cookie` capture.
pub fn fetch_post_with_cookies(
    url: &str,
    content_type: &str,
    body: &[u8],
    jar: &mut CookieJar,
    initiator_url: Option<&str>,
) -> Result<(String, Response), NetError> {
    let mut current = url.trim().to_string();
    let mut use_post = true;
    for _ in 0..=MAX_REDIRECTS {
        let parsed = parse_url(&current)?;
        if let Some(reason) = is_url_blocked_by_default_shield(&current, initiator_url) {
            return Err(NetError::BlockedByShield(reason.to_string()));
        }
        let cookie_hdr = jar.http_cookie_header(&current, initiator_url, !use_post);
        let response = if use_post {
            http_post_with_cookie(&parsed, content_type, body, cookie_hdr.as_deref())?
        } else {
            http_get_with_cookie(&parsed, cookie_hdr.as_deref())?
        };
        for set_cookie in response.headers_named("set-cookie") {
            jar.store_set_cookie(&current, set_cookie);
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            if matches!(response.status, 301 | 302 | 303) {
                use_post = false;
            }
            let location = response
                .header("location")
                .ok_or(NetError::InvalidResponse)?;
            let next = resolve_url(&current, location).ok_or(NetError::InvalidUrl)?;
            let next_parsed = parse_url(&next)?;
            if parsed.scheme == "https" && next_parsed.scheme == "http" {
                return Err(NetError::InsecureRedirectDowngrade);
            }
            current = next;
            continue;
        }
        return Ok((current, response));
    }
    Err(NetError::TooManyRedirects)
}

pub fn append_query(url: &str, query: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    }
}

fn build_request_with_cookie(url: &Url, cookie_header: Option<&str>) -> String {
    let path = if url.path.is_empty() {
        "/"
    } else {
        url.path.as_str()
    };
    let query = url
        .query
        .as_ref()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let host = if Url::default_port(&url.scheme) == Some(url.port) {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let cookie_line = match cookie_header {
        Some(c) if !c.is_empty() => format!("Cookie: {c}\r\n"),
        _ => String::new(),
    };
    format!(
        "GET {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: flux-browser/v.11\r\nAccept: text/html,text/css,image/webp,image/png,image/jpeg,image/gif,image/x-icon,image/bmp,*/*\r\nAccept-Encoding: gzip, deflate, identity\r\nReferrer-Policy: no-referrer\r\nDNT: 1\r\nX-Content-Type-Options: nosniff\r\n{cookie_line}Connection: close\r\n\r\n"
    )
}

pub fn build_post_request(url: &Url, content_type: &str, body: &[u8]) -> Vec<u8> {
    build_post_request_with_cookie(url, content_type, body, None)
}

pub fn build_post_request_with_cookie(
    url: &Url,
    content_type: &str,
    body: &[u8],
    cookie_header: Option<&str>,
) -> Vec<u8> {
    let path = if url.path.is_empty() {
        "/"
    } else {
        url.path.as_str()
    };
    let query = url
        .query
        .as_ref()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let host = if Url::default_port(&url.scheme) == Some(url.port) {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let safe_ct: String = content_type
        .chars()
        .filter(|&c| c != '\r' && c != '\n' && !c.is_control())
        .collect();
    let cookie_line = match cookie_header {
        Some(c) if !c.is_empty() => format!("Cookie: {c}\r\n"),
        _ => String::new(),
    };
    let mut head = format!(
        "POST {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: flux-browser/v.11\r\nAccept: text/html,text/css,image/webp,image/png,image/jpeg,image/gif,image/x-icon,image/bmp,*/*\r\nAccept-Encoding: gzip, deflate, identity\r\nReferrer-Policy: no-referrer\r\nDNT: 1\r\nX-Content-Type-Options: nosniff\r\nContent-Type: {safe_ct}\r\nContent-Length: {}\r\n{cookie_line}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    head.extend_from_slice(body);
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_https_urls() {
        let http = parse_url("http://example.com/a/b?x=1").unwrap();
        assert_eq!(http.scheme, "http");
        assert_eq!(http.host, "example.com");
        assert_eq!(http.port, 80);
        assert_eq!(http.path, "/a/b");
        assert_eq!(http.query.as_deref(), Some("x=1"));

        let https = parse_url("https://example.com/secure").unwrap();
        assert_eq!(https.scheme, "https");
        assert_eq!(https.port, 443);
        assert_eq!(https.path, "/secure");
    }

    #[test]
    fn refuses_unknown_schemes() {
        assert!(matches!(
            parse_url("ftp://example.com/"),
            Err(NetError::UnsupportedScheme)
        ));
    }

    #[test]
    fn parses_a_response_and_chunked_body() {
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        let response = parse_http_response(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.reason, "OK");
        assert_eq!(response.body, b"hello");
        assert_eq!(response.header("Content-Type"), Some("text/html"));

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nFlux\r\n7\r\nBrowser\r\n0\r\n\r\n";
        let decoded = parse_http_response(chunked).unwrap();
        assert_eq!(decoded.body, b"FluxBrowser");

        let gz_payload = flux_compress::compress_gzip(b"<h1>Compressed Flux</h1>");
        let mut gz_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            gz_payload.len()
        )
        .into_bytes();
        gz_http.extend_from_slice(&gz_payload);
        let gz_resp = parse_http_response(&gz_http).unwrap();
        assert_eq!(gz_resp.body, b"<h1>Compressed Flux</h1>");
    }

    #[test]
    fn resolves_relative_http_https_and_file_links() {
        assert_eq!(
            resolve_url("http://example.com/docs/intro.html", "guide.html"),
            Some("http://example.com/docs/guide.html".to_string())
        );
        assert_eq!(
            resolve_url("https://example.com/app/index.html", "/assets/site.css"),
            Some("https://example.com/assets/site.css".to_string())
        );
        assert_eq!(
            resolve_url("http://example.com:8080/a/b/c.html", "../index.html?x=1"),
            Some("http://example.com:8080/a/index.html?x=1".to_string())
        );
        assert_eq!(
            resolve_url("examples/hello.html", "second.html"),
            Some("examples/second.html".to_string())
        );
        assert_eq!(resolve_url("examples/hello.html", "#top"), None);
        assert_eq!(resolve_url("examples/hello.html", "/etc/passwd"), None);
        assert_eq!(
            resolve_url("examples/hello.html", "javascript:alert(1)"),
            None
        );
    }

    #[test]
    fn builds_post_requests_and_appends_query_strings() {
        let url = parse_url("http://example.com/submit").unwrap();
        let req = build_post_request(&url, "application/x-www-form-urlencoded", b"q=flux&page=2");
        let text = String::from_utf8(req).unwrap();
        assert!(text.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(text.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(text.contains("Content-Length: 13\r\n"));
        assert!(text.ends_with("\r\n\r\nq=flux&page=2"));
        assert_eq!(
            append_query("http://example.com/search?old=1", "q=flux"),
            "http://example.com/search?q=flux"
        );
    }

    #[test]
    fn enforces_same_origin_policy_and_blocks_phishing_or_injection_urls() {
        assert!(matches!(
            parse_url("https://trusted.example.com@evil.example.com/login"),
            Err(NetError::InvalidUrl)
        ));
        assert!(matches!(
            parse_url("http://example.com/path\r\nX-Injected: 1"),
            Err(NetError::InvalidUrl)
        ));
        assert_eq!(
            resolve_url("https://example.com/index.html", "flux://security"),
            None
        );
        assert_eq!(
            resolve_url("https://example.com/index.html", "file:///etc/passwd"),
            None
        );
        assert_eq!(
            resolve_url("flux://home", "flux://security"),
            Some("flux://security".to_string())
        );
        assert!(is_same_origin(
            "https://example.com/app/index.html",
            "/api/data"
        ));
        assert!(is_same_origin(
            "https://example.com/app/index.html",
            "https://example.com/other"
        ));
        assert!(!is_same_origin(
            "https://example.com/app/index.html",
            "https://attacker.example.com/steal"
        ));
        assert!(!is_same_origin(
            "https://example.com/app/index.html",
            "http://example.com/app/index.html"
        ));
    }

    #[test]
    fn enforces_csp_corp_and_cookie_jar_security_policies() {
        let csp = ContentSecurityPolicy::parse(
            "default-src 'self'; img-src 'self' *.cdn.example.com; script-src 'self'",
        );
        assert!(!csp.allows_inline_script());
        assert!(csp.allows_resource(
            CspDirective::ScriptSrc,
            "https://example.com/index.html",
            "/app.js"
        ));
        assert!(!csp.allows_resource(
            CspDirective::ScriptSrc,
            "https://example.com/index.html",
            "https://evil.example.org/payload.js"
        ));
        assert!(csp.allows_resource(
            CspDirective::ImgSrc,
            "https://example.com/index.html",
            "https://static.cdn.example.com/logo.png"
        ));

        assert!(allows_corp_subresource(
            Some("same-origin"),
            "https://example.com/index.html",
            "https://example.com/img.png"
        ));
        assert!(!allows_corp_subresource(
            Some("same-origin"),
            "https://example.com/index.html",
            "https://sub.example.com/img.png"
        ));
        assert!(allows_corp_subresource(
            Some("same-site"),
            "https://example.com/index.html",
            "https://sub.example.com/img.png"
        ));

        let mut jar = CookieJar::new();
        assert!(jar.store_set_cookie(
            "https://example.com/login",
            "__Host-sid=abc123; Secure; HttpOnly; Path=/; SameSite=Strict"
        ));
        assert!(jar.store_set_cookie(
            "https://example.com/login",
            "theme=dark; Secure; Path=/; SameSite=Lax"
        ));
        // Reject __Host- cookie with Domain attribute or over HTTP
        assert!(!jar.store_set_cookie(
            "https://example.com/login",
            "__Host-bad=1; Secure; Domain=example.com; Path=/"
        ));
        assert!(
            !jar.store_set_cookie("http://example.com/login", "__Secure-tok=1; Secure; Path=/")
        );

        assert_eq!(
            jar.http_cookie_header(
                "https://example.com/dashboard",
                Some("https://example.com/login"),
                true
            ),
            Some("__Host-sid=abc123; theme=dark".to_string())
        );
        // Cross-site top-level GET omits SameSite=Strict (__Host-sid), includes SameSite=Lax (theme)
        assert_eq!(
            jar.http_cookie_header(
                "https://example.com/dashboard",
                Some("https://other.org/link"),
                true
            ),
            Some("theme=dark".to_string())
        );
        // Script cookie string strictly omits HttpOnly cookies (__Host-sid)
        assert_eq!(
            jar.script_cookie_string("https://example.com/dashboard"),
            "theme=dark"
        );

        let exported = jar.to_json();
        let restored = CookieJar::from_json(&exported).unwrap();
        assert_eq!(restored, jar);
    }

    #[test]
    fn blocks_tracker_ad_and_malware_domains_before_opening_socket() {
        let mut shield = NetworkShield::new();
        assert!(
            shield
                .check_and_count(
                    "https://stats.doubleclick.net/pixel.gif",
                    Some("https://example.com")
                )
                .is_some()
        );
        assert!(
            shield
                .check_and_count(
                    "https://telemetry.vendor.org/collect",
                    Some("https://example.com")
                )
                .is_some()
        );
        assert!(
            shield
                .check_and_count(
                    "https://cdn.other.org/fingerprint.js",
                    Some("https://example.com")
                )
                .is_some()
        );
        assert_eq!(shield.blocked_total(), 3);
        assert!(
            shield
                .check_and_count(
                    "https://example.com/assets/app.css",
                    Some("https://example.com")
                )
                .is_none()
        );
        assert!(matches!(
            fetch("https://adserver.example.net/banner.js"),
            Err(NetError::BlockedByShield(_))
        ));
    }
}
