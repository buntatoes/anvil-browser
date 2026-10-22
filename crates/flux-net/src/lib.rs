//! URLs, HTTP, HTTPS, and redirect resolution.

#[cfg(windows)]
mod tls;

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const MAX_RESPONSE: usize = 2 * 1024 * 1024 + 64 * 1024;
const MAX_REDIRECTS: usize = 5;

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
}

#[derive(Debug)]
pub enum NetError {
    UnsupportedScheme,
    InvalidUrl,
    InvalidResponse,
    TooManyRedirects,
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
            Self::BodyTooLarge => write!(f, "the response is too large"),
            Self::Io(err) => write!(f, "the connection failed: {err}"),
        }
    }
}

impl std::error::Error for NetError {}

pub fn parse_url(input: &str) -> Result<Url, NetError> {
    let input = input.trim();
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
    if authority.is_empty() || authority.starts_with('[') {
        return Err(NetError::InvalidUrl);
    }
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        if host.is_empty() {
            return Err(NetError::InvalidUrl);
        }
        let port = port.parse().map_err(|_| NetError::InvalidUrl)?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), default_port)
    };
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
    if reference.is_empty() || reference.starts_with('#') {
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
    if reference.contains("://") {
        parse_url(reference).ok()?;
        return Some(reference.to_string());
    }
    if has_scheme(reference) {
        return None;
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
            if body.len() > length {
                body.truncate(length);
            }
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
        "gzip" | "x-gzip" => {
            flux_compress::decompress_gzip(&body).map_err(|_| NetError::InvalidResponse)
        }
        "deflate" => flux_compress::decompress_zlib(&body)
            .or_else(|_| flux_compress::decompress_deflate(&body))
            .map_err(|_| NetError::InvalidResponse),
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

    let request = build_request(url);
    if url.scheme == "https" {
        #[cfg(windows)]
        {
            let plain = tls::https_exchange(stream, &url.host, request.as_bytes(), MAX_RESPONSE)?;
            return parse_http_response(&plain);
        }
        #[cfg(not(windows))]
        {
            return Err(NetError::UnsupportedScheme);
        }
    }

    stream.write_all(request.as_bytes()).map_err(NetError::Io)?;

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
/// Returns the final URL and response.
pub fn fetch(url: &str) -> Result<(String, Response), NetError> {
    let mut current = url.trim().to_string();
    for _ in 0..=MAX_REDIRECTS {
        let parsed = parse_url(&current)?;
        let response = http_get(&parsed)?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .header("location")
                .ok_or(NetError::InvalidResponse)?;
            current = resolve_url(&current, location).ok_or(NetError::InvalidUrl)?;
            continue;
        }
        return Ok((current, response));
    }
    Err(NetError::TooManyRedirects)
}

fn build_request(url: &Url) -> String {
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
    format!(
        "GET {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: flux-browser/v.03\r\nAccept: text/html,text/css,image/png,image/bmp,*/*\r\nAccept-Encoding: gzip, deflate, identity\r\nConnection: close\r\n\r\n"
    )
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
}
