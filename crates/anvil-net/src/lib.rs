// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! HTTP/1.1 and HTTP/2 GET and POST for `http` and `https`.
//!
//! Windows HTTPS uses Schannel with certificate verification on. Linux HTTPS
//! uses OpenSSL `libssl` and `libcrypto` with peer verification and a hostname
//! check. Other targets refuse `https`. Live GET requests reuse pooled HTTP/1.1
//! keep-alive sockets and persistent multi-stream HTTP/2 sessions per origin
//! ([`connection_pool_stats`], [`fetch_batch`]).
//!
//! On Linux and Windows, `https` connections offer `h2` and `http/1.1` through
//! ALPN. When the server picks `h2`, the request goes over HTTP/2 ([`h2`], with
//! header compression from `anvil-hpack`). When it picks `http/1.1` or nothing,
//! or when the HTTP/2 server asks for HTTP/1.1, the request goes over HTTP/1.1
//! with `Connection: close`. Plain `http` stays on HTTP/1.1.
//! [`set_http2_enabled`] turns HTTP/2 off for the whole process.
//!
//! Also on Linux, the TLS handshake asks OpenSSL to offer the post-quantum
//! hybrid X25519MLKEM768 first, then X25519, P-256, and P-384 (see
//! [`TLS_GROUP_LISTS`]). Whether the hybrid is used is up to the OpenSSL
//! version and the server. The group each handshake ended up with is on
//! [`Response::key_exchange`] and in [`recent_key_exchanges`]. Windows leaves
//! the choice to Schannel and reads the group back from it after the handshake.
//!
//! `anvil-browser` page loads call [`fetch`] and [`fetch_post`]. Those functions
//! evaluate requests against the `adblock-rust` engine in [`shield`] before
//! connecting, follow at most [`MAX_REDIRECTS`] redirects, and refuse an `https`
//! to `http` downgrade. Script `fetch()` is same-origin only. The Rust interpreter
//! calls a host in `anvil-browser`, and that host calls [`http_get_with_cookie`]
//! with the session cookie jar. The numeric VM does not call this crate. `gzip`
//! and `deflate` bodies go through `anvil-compress`. Cookie and HSTS snapshots
//! go through `anvil-json` in [`policy`].
//!
//! Limits: [`MAX_RESPONSE`] raw bytes, [`MAX_DECOMPRESSED_BODY`] after inflate,
//! [`MAX_HEADER_BYTES`], [`MAX_HEADER_COUNT`], and [`MAX_REDIRECTS`] hops.

pub mod anvil_compress;
pub mod anvil_hpack;
pub mod anvil_json;
pub mod anvil_pqc;
pub mod brotli;
pub mod h2;
pub use h2::StreamPriority;
pub mod policy;
pub mod pqc_tls13;
pub mod shield;
#[cfg(any(windows, target_os = "linux"))]
mod tls;
pub mod ws;

pub use policy::{
    ContentSecurityPolicy, Cookie, CookieJar, CspDirective, CspSource, HstsEntry, HstsStore,
    MAX_COOKIE_BYTES, MAX_COOKIES_PER_HOST, MAX_COOKIES_TOTAL, MAX_HSTS_HOSTS, SameSite,
    allows_corp_subresource, is_same_site, registrable_domain, sha3_256_digest, sha256_digest,
    site_for_url, sri_sha3_256_token, sri_sha256_token, verify_subresource_integrity,
};
pub use shield::{
    BlockerResult, CosmeticFilter, CosmeticResources, NetworkFilter, NetworkFilterMask,
    NetworkShield, RedirectResourceStorage, RequestType, ShieldBlockReason, ShieldsMode,
    check_default_shield_request, clean_url_with_default_shield, cosmetic_css_from_default_shield,
    is_url_blocked_by_default_shield, sync_default_shield,
};
pub use ws::{
    MAX_WS_MESSAGE_BYTES, WebSocketClient, WsConnectOptions, WsError, WsEvent, WsMessage,
    WsReadyState, WsUrl, compute_accept_key, parse_ws_url,
};

use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// HTTPS-only rules for the requests made on one thread.
///
/// When `enabled` is set, every `http` request is sent as `https` instead,
/// including redirect hops. If the `https` request cannot connect, the
/// fetch fails with [`NetError::HttpsOnly`] and nothing is sent over plain
/// `http`. Hosts in `allowed_http_hosts` are the exceptions the person chose;
/// they keep plain `http`. A direct [`http_get_with_cookie`] or
/// [`http_post_with_cookie`] to an `http` URL that is not an exception is
/// refused the same way, without connecting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpsOnlyPolicy {
    /// HTTPS-only is on.
    pub enabled: bool,
    /// Lowercased host names that may still use plain `http`. Each one also
    /// covers the hosts under it.
    pub allowed_http_hosts: Vec<String>,
    /// Lowercased host names whose certificate problems the person chose to
    /// get past. Each one covers that exact host only, not the hosts under
    /// it. Only an untrusted,
    /// expired, or wrong-name certificate is let through for them; every
    /// other TLS failure still fails. Empty unless set with
    /// [`Self::with_tls_exceptions`].
    pub insecure_tls_hosts: Vec<String>,
    /// Hosts that must use HTTPS (HSTS, learned or built in). A certificate
    /// problem on one of them is never let through, even when it is listed
    /// in `insecure_tls_hosts`. Set with [`Self::with_hsts`].
    pub hsts: std::sync::Arc<HstsStore>,
}

impl HttpsOnlyPolicy {
    /// A policy with the given switch and exceptions.
    pub fn new(enabled: bool, allowed_http_hosts: &[String]) -> Self {
        Self {
            enabled,
            allowed_http_hosts: allowed_http_hosts
                .iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| !h.is_empty())
                .collect(),
            insecure_tls_hosts: Vec::new(),
            hsts: std::sync::Arc::default(),
        }
    }

    /// This policy, plus hosts whose certificate problems are let through.
    pub fn with_tls_exceptions(mut self, hosts: &[String]) -> Self {
        self.insecure_tls_hosts = hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        self
    }

    /// This policy, plus the hosts that must use HTTPS.
    pub fn with_hsts(mut self, hsts: &HstsStore) -> Self {
        self.hsts = std::sync::Arc::new(hsts.clone());
        self
    }

    /// Whether a certificate problem on `host` is let through: only for a
    /// host listed exactly, and never for a host that must use HTTPS.
    pub fn allows_insecure_tls(&self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        self.can_make_tls_exception(&host) && self.insecure_tls_hosts.contains(&host)
    }

    /// Whether the person may be offered a way past a certificate problem on
    /// `host`. False on systems without exceptions and for hosts that must
    /// use HTTPS.
    pub fn can_make_tls_exception(&self, host: &str) -> bool {
        TLS_EXCEPTIONS_SUPPORTED && !self.hsts.requires_https(host)
    }

    /// Whether a plain `http` request to `host` is allowed.
    pub fn allows_http(&self, host: &str) -> bool {
        if !self.enabled {
            return true;
        }
        host_listed(&self.allowed_http_hosts, host)
    }

    /// The `https` form of `url` when this policy requires it, or `None` when
    /// `url` may be used as it is. Port 80 becomes 443; any other port is kept.
    pub fn upgrade(&self, url: &str) -> Option<String> {
        let parsed = parse_url(url).ok()?;
        if parsed.scheme != "http" || self.allows_http(&parsed.host) {
            return None;
        }
        let port = if parsed.port == 80 || parsed.port == 443 {
            String::new()
        } else {
            format!(":{}", parsed.port)
        };
        let query = parsed
            .query
            .as_ref()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        Some(format!(
            "https://{}{port}{}{query}",
            parsed.host, parsed.path
        ))
    }
}

/// Whether `host` is one of `list` or a host under one.
fn host_listed(list: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    list.iter().any(|allowed| {
        host == *allowed
            || host
                .strip_suffix(allowed.as_str())
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

/// What was wrong with a server's certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertProblem {
    /// Not signed by anyone this system trusts, or self-signed.
    Untrusted,
    /// Past its end date, or not valid yet.
    Expired,
    /// Issued for a different name than the one asked for.
    HostnameMismatch,
}

impl CertProblem {
    /// Short name used in links and tests.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::Expired => "expired",
            Self::HostnameMismatch => "hostname-mismatch",
        }
    }

    /// The problem for an OpenSSL `X509_V_ERR_*` verify result, or `None`
    /// for one that is not about trust, dates, or the name.
    pub fn from_openssl_verify(code: i64) -> Option<Self> {
        match code {
            // UNABLE_TO_GET_ISSUER_CERT, DEPTH_ZERO_SELF_SIGNED_CERT,
            // SELF_SIGNED_CERT_IN_CHAIN, UNABLE_TO_GET_ISSUER_CERT_LOCALLY,
            // UNABLE_TO_VERIFY_LEAF_SIGNATURE, CERT_UNTRUSTED.
            2 | 18 | 19 | 20 | 21 | 27 => Some(Self::Untrusted),
            // CERT_NOT_YET_VALID, CERT_HAS_EXPIRED.
            9 | 10 => Some(Self::Expired),
            // HOSTNAME_MISMATCH.
            62 => Some(Self::HostnameMismatch),
            _ => None,
        }
    }

    /// The problem for a Schannel or certificate-chain status, or `None`.
    pub fn from_schannel_status(status: u32) -> Option<Self> {
        match status {
            // SEC_E_UNTRUSTED_ROOT, CERT_E_UNTRUSTEDROOT, CERT_E_CHAINING.
            0x8009_0325 | 0x800B_0109 | 0x800B_010A => Some(Self::Untrusted),
            // SEC_E_CERT_EXPIRED, CERT_E_EXPIRED.
            0x8009_0328 | 0x800B_0101 => Some(Self::Expired),
            // SEC_E_WRONG_PRINCIPAL, CERT_E_CN_NO_MATCH.
            0x8009_0322 | 0x800B_010F => Some(Self::HostnameMismatch),
            _ => None,
        }
    }
}

/// Whether this build can get past a certificate problem for a host the
/// person chose. Only the Linux TLS path can; elsewhere the problem always
/// stops the connection.
pub const TLS_EXCEPTIONS_SUPPORTED: bool = cfg!(target_os = "linux");

thread_local! {
    static HTTPS_ONLY: RefCell<HttpsOnlyPolicy> = RefCell::new(HttpsOnlyPolicy::default());
}

/// The HTTPS-only policy for requests made on this thread. Off by default.
pub fn https_only_policy() -> HttpsOnlyPolicy {
    HTTPS_ONLY.with(|p| p.borrow().clone())
}

/// Puts back the previous HTTPS-only policy when dropped.
#[must_use = "the policy only lasts while this value is alive"]
pub struct HttpsOnlyScope {
    previous: Option<HttpsOnlyPolicy>,
}

impl Drop for HttpsOnlyScope {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            HTTPS_ONLY.with(|p| *p.borrow_mut() = previous);
        }
    }
}

/// Use `policy` for requests made on this thread until the returned value
/// is dropped. The browser sets this around each page load, on the thread
/// that does the load.
pub fn enter_https_only(policy: HttpsOnlyPolicy) -> HttpsOnlyScope {
    let previous = HTTPS_ONLY.with(|p| std::mem::replace(&mut *p.borrow_mut(), policy));
    HttpsOnlyScope {
        previous: Some(previous),
    }
}

/// Whether `https` connections may use HTTP/2. On by default.
static HTTP2_ENABLED: AtomicBool = AtomicBool::new(true);

/// Turn HTTP/2 on or off for every later `https` request in this process.
/// Off means ALPN only offers `http/1.1`.
pub fn set_http2_enabled(enabled: bool) {
    HTTP2_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether `https` requests may use HTTP/2.
pub fn http2_enabled() -> bool {
    HTTP2_ENABLED.load(Ordering::Relaxed)
}

/// Origin-keyed keep-alive and HTTP/2 multiplexing pool statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionPoolStats {
    /// Number of idle keep-alive connections currently cached by origin.
    pub idle_connections: usize,
    /// Total number of requests served by reusing a pooled keep-alive or multiplexed session.
    pub reused_requests: usize,
    /// Total number of HTTP/2 streams multiplexed across shared sessions.
    pub multiplexed_streams: usize,
}

#[cfg(any(windows, target_os = "linux"))]
type IdleH2Pool =
    std::collections::HashMap<String, Vec<(h2::H2ClientSession<tls::TlsSession>, Option<String>)>>;

struct OriginPoolState {
    idle: std::collections::HashMap<String, Vec<TcpStream>>,
    #[cfg(any(windows, target_os = "linux"))]
    idle_h2: IdleH2Pool,
    reused_requests: usize,
    multiplexed_streams: usize,
}

impl OriginPoolState {
    fn new() -> Self {
        Self {
            idle: std::collections::HashMap::new(),
            #[cfg(any(windows, target_os = "linux"))]
            idle_h2: std::collections::HashMap::new(),
            reused_requests: 0,
            multiplexed_streams: 0,
        }
    }
}

static CONNECTION_POOL: std::sync::OnceLock<Mutex<OriginPoolState>> = std::sync::OnceLock::new();

fn pool_state() -> &'static Mutex<OriginPoolState> {
    CONNECTION_POOL.get_or_init(|| Mutex::new(OriginPoolState::new()))
}

/// Return current origin-keyed connection pool and HTTP/2 multiplexing statistics.
pub fn connection_pool_stats() -> ConnectionPoolStats {
    let guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut idle_connections: usize = guard.idle.values().map(|v| v.len()).sum();
    #[cfg(any(windows, target_os = "linux"))]
    {
        idle_connections =
            idle_connections.saturating_add(guard.idle_h2.values().map(|v| v.len()).sum());
    }
    ConnectionPoolStats {
        idle_connections,
        reused_requests: guard.reused_requests,
        multiplexed_streams: guard.multiplexed_streams,
    }
}

/// Close and drop all idle connections in the origin-keyed connection pool.
pub fn clear_connection_pool() {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.idle.clear();
    #[cfg(any(windows, target_os = "linux"))]
    {
        for sessions in guard.idle_h2.values_mut() {
            for (session, _) in sessions.drain(..) {
                session.close();
            }
        }
        guard.idle_h2.clear();
    }
}

/// Store a live keep-alive `TcpStream` under `origin_key` (`scheme://host:port`), bounded at 4 per origin.
pub fn put_pooled_connection(origin_key: &str, stream: TcpStream) {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let list = guard.idle.entry(origin_key.to_string()).or_default();
    if list.len() < 4 {
        list.push(stream);
    }
}

fn is_pooled_stream_alive(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut probe = [0u8; 1];
    let alive = matches!(
        stream.peek(&mut probe),
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
    );
    let _ = stream.set_nonblocking(false);
    alive
}

/// Take a live idle `TcpStream` from the origin pool if one is available,
/// automatically discarding sockets closed by the peer or holding stale data.
pub fn take_pooled_connection(origin_key: &str) -> Option<TcpStream> {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let list = guard.idle.get_mut(origin_key)?;
    while let Some(stream) = list.pop() {
        if is_pooled_stream_alive(&stream) {
            guard.reused_requests = guard.reused_requests.saturating_add(1);
            return Some(stream);
        }
    }
    None
}

fn rollback_reused_request() {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.reused_requests = guard.reused_requests.saturating_sub(1);
}

/// Pool key for an `https` origin. A session opened while a certificate
/// exception covers the host is kept apart, so a load without that
/// exception never reuses it.
#[cfg(any(windows, target_os = "linux"))]
fn h2_pool_key(origin_key: &str) -> String {
    let host = parse_url(origin_key).map(|u| u.host).unwrap_or_default();
    if https_only_policy().allows_insecure_tls(&host) {
        format!("insecure-tls {origin_key}")
    } else {
        origin_key.to_string()
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn put_pooled_h2_session(
    origin_key: &str,
    session: h2::H2ClientSession<tls::TlsSession>,
    group: Option<String>,
) {
    let key = h2_pool_key(origin_key);
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let list = guard.idle_h2.entry(key).or_default();
    if list.len() < 4 {
        list.push((session, group));
    } else {
        session.close();
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn take_pooled_h2_session(
    origin_key: &str,
) -> Option<(h2::H2ClientSession<tls::TlsSession>, Option<String>)> {
    let key = h2_pool_key(origin_key);
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.idle_h2.get_mut(&key).and_then(|v| v.pop());
    if entry.is_some() {
        guard.reused_requests = guard.reused_requests.saturating_add(1);
        guard.multiplexed_streams = guard.multiplexed_streams.saturating_add(1);
    }
    entry
}

#[cfg(any(windows, target_os = "linux"))]
fn rollback_reused_h2_stream() {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.reused_requests = guard.reused_requests.saturating_sub(1);
    guard.multiplexed_streams = guard.multiplexed_streams.saturating_sub(1);
}

/// Record multiplexed HTTP/2 streams completed on a shared session.
pub fn record_multiplexed_streams(count: usize) {
    let mut guard = pool_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.multiplexed_streams = guard.multiplexed_streams.saturating_add(count);
    if count > 1 {
        guard.reused_requests = guard.reused_requests.saturating_add(count - 1);
    }
}

/// The groups lists the Linux TLS handshake hands to OpenSSL, in order. The
/// first one OpenSSL accepts is used.
#[cfg(any(windows, target_os = "linux"))]
#[cfg(target_os = "linux")]
pub use tls::GROUP_LISTS as TLS_GROUP_LISTS;

/// How many handshakes [`recent_key_exchanges`] remembers.
pub const MAX_RECENT_KEY_EXCHANGES: usize = 16;

/// The key exchange one TLS handshake used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyExchange {
    /// Host name the connection was for.
    pub host: String,
    /// Group name as OpenSSL reports it, for example `X25519MLKEM768`.
    pub group: String,
}

impl KeyExchange {
    /// True only when the group that was actually negotiated includes ML-KEM.
    pub fn is_post_quantum(&self) -> bool {
        is_post_quantum_group(&self.group)
    }
}

/// True when `group` names a hybrid or pure ML-KEM group. A name that merely
/// was offered doesn't count; pass the negotiated one.
pub fn is_post_quantum_group(group: &str) -> bool {
    group.to_ascii_uppercase().contains("MLKEM")
}

static RECENT_KEY_EXCHANGES: Mutex<Vec<KeyExchange>> = Mutex::new(Vec::new());

/// Remember the group a handshake used. Newest first, capped at
/// [`MAX_RECENT_KEY_EXCHANGES`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn record_key_exchange(host: &str, group: &str) {
    let mut list = RECENT_KEY_EXCHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    list.insert(
        0,
        KeyExchange {
            host: host.to_string(),
            group: group.to_string(),
        },
    );
    list.truncate(MAX_RECENT_KEY_EXCHANGES);
}

/// The key exchange groups of the most recent TLS handshakes in this
/// process, newest first. Only Linux fills this in; on Windows it stays empty.
pub fn recent_key_exchanges() -> Vec<KeyExchange> {
    RECENT_KEY_EXCHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Which protocol carried a response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HttpVersion {
    /// HTTP/1.1, one request per connection.
    #[default]
    Http11,
    /// HTTP/2, one stream per connection.
    Http2,
}

/// Maximum raw response bytes, including headers, before decompression (80 MiB).
///
/// The cleartext and TLS reads stop with [`NetError::BodyTooLarge`] at this size.
/// Chunked bodies use the same cap on the decoded chunks.
pub const MAX_RESPONSE: usize = 80 * 1024 * 1024;

/// Maximum bytes after `gzip` or `deflate` decompression (160 MiB).
///
/// A larger inflate from `anvil-compress` becomes [`NetError::BodyTooLarge`].
pub const MAX_DECOMPRESSED_BODY: usize = 160 * 1024 * 1024;

/// Maximum header-block bytes before the blank line (640 KiB).
///
/// A larger block is [`NetError::InvalidResponse`], not [`NetError::BodyTooLarge`].
pub const MAX_HEADER_BYTES: usize = 640 * 1024;

/// Maximum header fields accepted.
///
/// The blank line is not counted. A further field is [`NetError::InvalidResponse`].
/// A line with no colon is skipped and does not count.
pub const MAX_HEADER_COUNT: usize = 1280;

/// Maximum redirects followed by [`fetch`] and [`fetch_post`].
///
/// The loop allows the first response plus this many hops. One more redirect
/// status after that returns [`NetError::TooManyRedirects`].
pub const MAX_REDIRECTS: usize = 10;

/// An `http` or `https` URL with no fragment and no userinfo.
///
/// [`parse_url`] is the only constructor. IPv6 brackets are rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// Lowercased `http` or `https`.
    pub scheme: String,
    /// Lowercased host. No brackets and no userinfo.
    pub host: String,
    /// Explicit port, or 80 for `http` and 443 for `https`. Port 0 is rejected.
    pub port: u16,
    /// Path beginning with `/`. An empty path is stored as `/`.
    pub path: String,
    /// Query without `?`, if one was present. Not decoded.
    pub query: Option<String>,
}

impl Url {
    /// Default port for `http` and `https`. Any other scheme is `None`.
    pub fn default_port(scheme: &str) -> Option<u16> {
        match scheme {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        }
    }

    /// `scheme://host`, or `scheme://host:port` when the port is not the default.
    pub fn authority(&self) -> String {
        if Self::default_port(&self.scheme) == Some(self.port) {
            format!("{}://{}", self.scheme, self.host)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

/// One parsed HTTP response.
///
/// The body is after chunk decoding and after `gzip` or `deflate` decompression.
/// Header names are lowercased. The reason phrase may be empty, and is always
/// empty for HTTP/2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// Status code from the status line. Not range-checked.
    pub status: u16,
    /// Reason phrase with internal whitespace collapsed to single spaces. May be empty.
    pub reason: String,
    /// Header names lowercased. Repeated names are kept in order.
    pub headers: Vec<(String, String)>,
    /// Decoded body. Not truncated to a character boundary.
    pub body: Vec<u8>,
    /// Protocol the response came over.
    pub version: HttpVersion,
    /// TLS key exchange group the connection used, as OpenSSL names it.
    /// `None` for plain `http`, on Windows, and when OpenSSL can't say.
    pub key_exchange: Option<String>,
}

impl Response {
    /// First header value for `name`, compared case-insensitively. Names are already lowercased.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Every value for `name`, in arrival order. Used for repeated `Set-Cookie` fields.
    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

/// Failure from parsing, policy, or the socket.
#[derive(Debug)]
pub enum NetError {
    /// The scheme is not `http` or `https`, including `https` on a target with no TLS backend.
    UnsupportedScheme,
    /// The URL is empty, has userinfo, has controls, or failed host or port checks.
    InvalidUrl,
    /// The status line, headers, chunks, or content encoding could not be read.
    InvalidResponse,
    /// More than [`MAX_REDIRECTS`] redirect statuses were followed.
    TooManyRedirects,
    /// A redirect moved from `https` to `http`.
    InsecureRedirectDowngrade,
    /// The shield refused the host or path before connect. The string is the display reason.
    BlockedByShield(String),
    /// The raw response or the decompressed body exceeded its cap.
    BodyTooLarge,
    /// DNS, TCP, timeout, or TLS failure, including a failed certificate check.
    Io(std::io::Error),
    /// HTTPS-only is on and this `http` URL could not be loaded over `https`.
    /// Nothing was sent over plain `http`.
    HttpsOnly(String),
    /// The server's certificate failed the check for one of the reasons in
    /// [`CertProblem`]. The connection stopped before any request was sent.
    Certificate {
        /// Host the certificate was checked against.
        host: String,
        /// What was wrong with it.
        problem: CertProblem,
    },
    /// A download was stopped while its body was still arriving.
    Cancelled,
}

impl From<std::io::Error> for NetError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
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
            Self::BlockedByShield(reason) => write!(f, "blocked by Anvil Shield: {reason}"),
            Self::BodyTooLarge => write!(f, "the response exceeds safety size limits"),
            Self::Io(err) => write!(f, "the connection failed: {err}"),
            Self::HttpsOnly(url) => write!(
                f,
                "HTTPS-only mode is on and {url} could not be loaded over a secure connection"
            ),
            Self::Cancelled => write!(f, "the download was cancelled"),
            Self::Certificate { host, problem } => match problem {
                CertProblem::Untrusted => {
                    write!(f, "the TLS certificate for {host} is not trusted")
                }
                CertProblem::Expired => write!(
                    f,
                    "the TLS certificate for {host} has expired or is not valid yet"
                ),
                CertProblem::HostnameMismatch => write!(
                    f,
                    "the TLS certificate for {host} was issued for a different name"
                ),
            },
        }
    }
}

impl std::error::Error for NetError {}

/// Parse an absolute `http` or `https` URL.
///
/// Trims whitespace, rejects ASCII controls, space, and DEL, and drops the fragment.
/// The scheme must be `http` or `https`. An empty authority, a leading `[` (IPv6),
/// or `@` (userinfo) is [`NetError::InvalidUrl`]. The host is lowercased, at most
/// 253 characters, and limited to ASCII letters, digits, `.`, and `-`. It must
/// not start with `.` or `-`. The port must be a non-zero `u16`. A missing path
/// becomes `/`.
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
    // Reject an empty authority and userinfo. Userinfo is not parsed.
    if authority.is_empty() || authority.contains('@') {
        return Err(NetError::InvalidUrl);
    }
    let (host, port) = if authority.starts_with('[') {
        let close = authority.find(']').ok_or(NetError::InvalidUrl)?;
        let ipv6_host = &authority[..=close];
        let after = &authority[close + 1..];
        let port = if after.is_empty() {
            default_port
        } else if let Some(port_str) = after.strip_prefix(':') {
            let p: u16 = port_str.parse().map_err(|_| NetError::InvalidUrl)?;
            if p == 0 {
                return Err(NetError::InvalidUrl);
            }
            p
        } else {
            return Err(NetError::InvalidUrl);
        };
        (ipv6_host.to_ascii_lowercase(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
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

/// Hostname shape used by [`parse_url`].
///
/// Accepts bracketed IPv6 literals (`[::1]`), dotted IPv4 text, and ASCII
/// hostnames up to 253 characters that do not start with `.` or `-`.
pub(crate) fn is_valid_hostname(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.starts_with('-')
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
}

/// Whether `base` and `target` share scheme, host, and port.
///
/// `target` is resolved against `base` first. An empty target, a failed resolve,
/// or a failed parse is not same-origin. For an `http` or `https` base, scheme,
/// host, and port must all match. For a `anvil://` base, the resolved URL must
/// also start with `anvil://`. Any other base is same-origin only when the
/// resolved reference has no scheme.
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
    } else if base.starts_with("anvil://") {
        resolved.starts_with("anvil://")
    } else {
        !resolved.contains("://") && !has_scheme(&resolved)
    }
}

/// Store an empty path as `/`. Other paths are kept, including those without a leading slash.
fn empty_as_root(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

/// Resolve `reference` against `base`.
///
/// Returns `None` for an empty reference, a fragment-only reference, ASCII
/// controls, or DEL. The fragment is stripped before resolution. A `//` reference
/// keeps the base scheme and must parse as `http` or `https`. A `anvil://`
/// reference is kept only when `base` is also `anvil://`. Any other scheme that
/// contains `://` must parse as `http` or `https`, so `file:` and similar
/// schemes fail. A scheme without `://`, such as `javascript:`, also fails.
/// Relative references on `http` or `https` are path-joined and `.` and `..`
/// are collapsed. Other bases use file-path joining, which rejects absolute
/// paths and a `..` that leaves the base directory.
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
    if reference.starts_with("anvil://") {
        // A anvil:// target is accepted only from a anvil:// base.
        if base.starts_with("anvil://") {
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
    if base.starts_with("anvil://") {
        return Some(format!("anvil://{}", reference.trim_start_matches('/')));
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

/// Whether `reference` begins with a scheme token and a colon.
///
/// The token must be non-empty and contain only ASCII alphanumeric, `+`, `-`,
/// or `.`. A slash or backslash in the token means this is not a scheme, so a
/// Windows path is not treated as one. Used to reject `javascript:` and similar
/// references that are not `http`, `https`, or `anvil`.
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

/// Collapse `.` and `..` in an HTTP path.
///
/// Empty segments and `.` are dropped. `..` pops one segment and does not fail
/// when the stack is already empty, so `..` above the root stays at `/`.
/// A trailing slash is preserved except for the root.
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

/// Join a relative file reference onto the directory of `base`.
///
/// Returns `None` when the reference is empty, absolute (`/` or `\`), contains
/// `:`, or a `..` would pop past the base directory. The query is discarded.
/// `.` segments are ignored. Backslashes in the base and the reference are
/// treated as separators.
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

/// Parse one HTTP/1.1 response from bytes already read off the socket.
///
/// Requires a `\r\n\r\n` header terminator. A header block larger than
/// [`MAX_HEADER_BYTES`] is [`NetError::InvalidResponse`]. The status line must
/// have a version token and a `u16` status. The version is not checked. The
/// remaining tokens are the reason phrase. Header names are lowercased. A line
/// without `:` is skipped. More than [`MAX_HEADER_COUNT`] fields fails.
/// `Transfer-Encoding: chunked` wins over `Content-Length`. A declared length
/// above [`MAX_RESPONSE`], or a body above that cap, is [`NetError::BodyTooLarge`].
/// A shorter body than `Content-Length` is kept as read. Bytes past
/// `Content-Length` are dropped. The body is then decoded from `gzip` or `deflate`.
pub fn parse_http_response(bytes: &[u8]) -> Result<Response, NetError> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(NetError::InvalidResponse)?;
    // Header block cap. Oversized headers are an invalid response, not a body-size error.
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
        // Header count cap. Lines without a colon are ignored and do not count.
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
            // A declared length above the raw cap fails before the body is kept.
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
        version: HttpVersion::Http11,
        key_exchange: None,
    })
}

/// Decode `Content-Encoding` after chunk or length handling.
///
/// Missing, empty, and `identity` return the body unchanged. `gzip` and
/// `x-gzip` use bounded gzip inflate. `deflate` tries zlib first, then raw
/// deflate. An output over [`MAX_DECOMPRESSED_BODY`] is [`NetError::BodyTooLarge`].
/// Any other inflate error is [`NetError::InvalidResponse`]. An unrecognized
/// encoding is returned unchanged, still compressed.
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
        "gzip" | "x-gzip" => anvil_compress::decompress_gzip_bounded(&body, MAX_DECOMPRESSED_BODY)
            .map_err(|err| match err {
                anvil_compress::CompressError::OutputTooLarge { .. } => NetError::BodyTooLarge,
                _ => NetError::InvalidResponse,
            }),
        "deflate" => anvil_compress::decompress_zlib_bounded(&body, MAX_DECOMPRESSED_BODY)
            .or_else(|_| anvil_compress::decompress_deflate_bounded(&body, MAX_DECOMPRESSED_BODY))
            .map_err(|err| match err {
                anvil_compress::CompressError::OutputTooLarge { .. } => NetError::BodyTooLarge,
                _ => NetError::InvalidResponse,
            }),
        "br" => {
            brotli::decompress_brotli_bounded(&body, MAX_DECOMPRESSED_BODY).map_err(|err| match err
            {
                anvil_compress::CompressError::OutputTooLarge { .. } => NetError::BodyTooLarge,
                _ => NetError::InvalidResponse,
            })
        }
        // Unknown encodings are not rejected. The caller receives the raw bytes.
        _ => Ok(body),
    }
}

/// Bytes of a response body seen before the read has finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlightBody {
    pub received: usize,
    pub total: Option<usize>,
    pub content_type: String,
    pub content_disposition: String,
}

type InFlightHook = fn(&InFlightBody);

fn inflight_slot() -> &'static std::sync::Mutex<Option<InFlightHook>> {
    static HOOK: std::sync::Mutex<Option<InFlightHook>> = std::sync::Mutex::new(None);
    &HOOK
}

fn inflight_hook() -> Option<InFlightHook> {
    inflight_slot().lock().ok().and_then(|guard| *guard)
}

/// Install a callback for body bytes as they arrive. `None` removes it.
///
/// The callback must return quickly and must not call this function.
/// A missing callback means progress is not reported.
pub fn set_inflight_hook(hook: Option<InFlightHook>) {
    if let Ok(mut guard) = inflight_slot().lock() {
        *guard = hook;
    }
}

type StopHook = fn(&str, &str) -> bool;

fn stop_slot() -> &'static std::sync::Mutex<Option<StopHook>> {
    static STOP: std::sync::Mutex<Option<StopHook>> = std::sync::Mutex::new(None);
    &STOP
}

/// Ask whether the body being read should stop.
///
/// `content_type` and `content_disposition` are the headers seen so far.
/// A missing hook does not stop the read.
pub fn set_transfer_stop_hook(hook: Option<StopHook>) {
    if let Ok(mut guard) = stop_slot().lock() {
        *guard = hook;
    }
}

fn transfer_should_stop(content_type: &str, content_disposition: &str) -> bool {
    stop_slot()
        .lock()
        .ok()
        .and_then(|guard| *guard)
        .is_some_and(|hook| hook(content_type, content_disposition))
}

pub(crate) fn stop_raw_response(buf: &[u8]) -> bool {
    let Some(note) = inflight_from_partial(buf) else {
        return false;
    };
    transfer_should_stop(&note.content_type, &note.content_disposition)
}

pub(crate) fn stop_headers(headers: &[(String, String)]) -> bool {
    let mut content_type = String::new();
    let mut content_disposition = String::new();
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.clone();
        } else if name.eq_ignore_ascii_case("content-disposition") {
            content_disposition = value.clone();
        }
    }
    transfer_should_stop(&content_type, &content_disposition)
}

/// What a partial HTTP/1 response has of its body so far.
///
/// `None` until the header block is complete. Chunked bodies count only
/// finished chunks, and they have no total. A declared length is the total
/// only when the body is not chunked.
pub fn inflight_from_partial(buf: &[u8]) -> Option<InFlightBody> {
    let (split, declared, chunked) = header_end_and_framing(buf)?;
    let head = std::str::from_utf8(&buf[..split]).ok()?;
    let mut content_type = String::new();
    let mut content_disposition = String::new();
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.trim().eq_ignore_ascii_case("content-type") {
            content_type = value.to_string();
        } else if name.trim().eq_ignore_ascii_case("content-disposition") {
            content_disposition = value.to_string();
        }
    }
    let raw_body = &buf[split + 4..];
    let (received, total) = if chunked {
        (chunked_payload_so_far(raw_body), None)
    } else {
        let received = match declared {
            Some(len) => raw_body.len().min(len),
            None => raw_body.len(),
        };
        (received, declared)
    };
    Some(InFlightBody {
        received,
        total,
        content_type,
        content_disposition,
    })
}

fn chunked_payload_so_far(body: &[u8]) -> usize {
    let mut index = 0;
    let mut total = 0usize;
    while index < body.len() {
        let Some(mark) = body[index..].windows(2).position(|pair| pair == b"\r\n") else {
            break;
        };
        let line = &body[index..index + mark];
        let hex = std::str::from_utf8(line).unwrap_or("").trim();
        let size_text = hex.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            break;
        };
        let start = index + mark + 2;
        if start + size + 2 > body.len() {
            break;
        }
        total = total.saturating_add(size);
        if size == 0 {
            break;
        }
        index = start + size + 2;
    }
    total
}

pub(crate) fn notify_inflight_buffer(buf: &[u8]) {
    let Some(note) = inflight_from_partial(buf) else {
        return;
    };
    if let Some(hook) = inflight_hook() {
        hook(&note);
    }
}

pub(crate) fn notify_inflight_headers(
    headers: &[(String, String)],
    received: usize,
    total: Option<usize>,
) {
    let mut content_type = String::new();
    let mut content_disposition = String::new();
    let mut declared = total;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-type") {
            content_type = value.clone();
        } else if name.eq_ignore_ascii_case("content-disposition") {
            content_disposition = value.clone();
        } else if declared.is_none() && name.eq_ignore_ascii_case("content-length") {
            declared = value.trim().parse().ok();
        }
    }
    let note = InFlightBody {
        received,
        total: declared,
        content_type,
        content_disposition,
    };
    if let Some(hook) = inflight_hook() {
        hook(&note);
    }
}

/// First `Content-Length` that parses as `usize`. A missing or non-numeric value is ignored.
fn content_length(headers: &[(String, String)]) -> Option<usize> {
    headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
}

/// Whether any `Transfer-Encoding` list contains `chunked`, case-insensitively.
///
/// Comma-separated tokens are trimmed. Other transfer encodings are not decoded.
fn is_chunked(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name == "transfer-encoding"
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    })
}

/// Decode a chunked body.
///
/// Each size line is hex before an optional `;` extension. The extension is
/// ignored. A missing CRLF, a non-hex size, a short chunk, or a chunk that is
/// not followed by CRLF is [`NetError::InvalidResponse`]. A zero size ends the
/// body. Trailers after that chunk are not parsed. The summed chunk bytes must
/// stay within [`MAX_RESPONSE`] or the result is [`NetError::BodyTooLarge`].
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
        // Decoded chunk cap. The raw read is also capped before this runs.
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

/// HTTP/1.1 GET with no `Cookie` header. Does not follow redirects and does not apply the shield.
pub fn http_get(url: &Url) -> Result<Response, NetError> {
    http_exchange(url, build_request_with_cookie(url, None).as_bytes())
}

/// HTTP/1.1 GET. `cookie_header` is sent only when it is `Some` and non-empty.
/// Does not follow redirects and does not apply the shield.
pub fn http_get_with_cookie(url: &Url, cookie_header: Option<&str>) -> Result<Response, NetError> {
    http_exchange(
        url,
        build_request_with_cookie(url, cookie_header).as_bytes(),
    )
}

/// HTTP/1.1 POST with no `Cookie` header. Does not follow redirects and does not apply the shield.
pub fn http_post(url: &Url, content_type: &str, body: &[u8]) -> Result<Response, NetError> {
    let request = build_post_request_with_cookie(url, content_type, body, None);
    http_exchange(url, &request)
}

/// Chunk size in bytes used when streaming POST request bodies with upload progress.
pub const UPLOAD_CHUNK_BYTES: usize = 16 * 1024;

/// HTTP/1.1 POST. `cookie_header` is sent only when it is `Some` and non-empty.
/// Does not follow redirects and does not apply the shield.
pub fn http_post_with_cookie(
    url: &Url,
    content_type: &str,
    body: &[u8],
    cookie_header: Option<&str>,
) -> Result<Response, NetError> {
    http_post_with_cookie_progress(url, content_type, body, cookie_header, |_, _| {})
}

/// HTTP POST with streaming `16 KiB` body chunk writes and `(sent_bytes, total_bytes)` progress callbacks.
///
/// `cookie_header` is sent only when it is `Some` and non-empty. Does not follow
/// redirects and does not apply the shield.
pub fn http_post_with_cookie_progress<F: FnMut(u64, u64)>(
    url: &Url,
    content_type: &str,
    body: &[u8],
    cookie_header: Option<&str>,
    mut on_progress: F,
) -> Result<Response, NetError> {
    let head = build_post_headers_with_cookie(url, content_type, body.len(), cookie_header);
    http_exchange_streaming_body(url, &head, body, &mut on_progress)
}

/// How long one TCP connect attempt may take before the next address is tried.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static NETWORK_OFF: AtomicBool = AtomicBool::new(false);

/// Turn networking off in this process for good.
///
/// After this, every request fails with an I/O error before a name is
/// looked up or a socket is opened. There is no way to turn it back on. A
/// tab renderer calls this first, because the browser does its fetches.
pub fn turn_off_network() {
    NETWORK_OFF.store(true, Ordering::SeqCst);
}

/// True once [`turn_off_network`] has been called in this process.
pub fn network_is_off() -> bool {
    NETWORK_OFF.load(Ordering::SeqCst)
}

/// Resolve `host:port` and connect to the first address that answers.
///
/// Each address gets `timeout`. The error from the last address is returned
/// when none of them connect. A name that resolves to nothing is an error.
/// Nothing is looked up after [`turn_off_network`].
pub(crate) fn connect_with_timeout(
    host: &str,
    port: u16,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    if network_is_off() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "this process has no network",
        ));
    }
    let lookup_host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    let mut last_err = None;
    for addr in (lookup_host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            ErrorKind::NotFound,
            "host name did not resolve to an address",
        )
    }))
}

fn header_end_and_framing(buf: &[u8]) -> Option<(usize, Option<usize>, bool)> {
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..split]).ok()?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok());
    if matches!(status, Some(100..=199 | 204 | 304)) {
        return Some((split, Some(0), false));
    }
    let mut cl: Option<usize> = None;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            let name = name.trim();
            let val = val.trim();
            if name.eq_ignore_ascii_case("content-length") {
                if let Ok(n) = val.parse::<usize>() {
                    cl = Some(n);
                }
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && val
                    .split(',')
                    .any(|p| p.trim().eq_ignore_ascii_case("chunked"))
            {
                chunked = true;
            }
        }
    }
    Some((split, cl, chunked))
}

fn write_body_chunks_stream<W: Write, F: FnMut(u64, u64)>(
    writer: &mut W,
    body: &[u8],
    on_progress: &mut F,
) -> std::io::Result<()> {
    let total = body.len() as u64;
    on_progress(0, total);
    if body.is_empty() {
        return writer.flush();
    }
    let mut sent = 0usize;
    while sent < body.len() {
        let end = (sent + UPLOAD_CHUNK_BYTES).min(body.len());
        writer.write_all(&body[sent..end])?;
        let _ = writer.flush();
        sent = end;
        on_progress(sent as u64, total);
    }
    Ok(())
}

fn read_http1_response_from_stream(
    stream: &mut TcpStream,
) -> Result<(Response, bool), (NetError, bool)> {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut exact_framing = false;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => {
                if buf.is_empty() {
                    return Err((NetError::InvalidResponse, true));
                }
                break;
            }
            Ok(size) => {
                if buf.len() + size > MAX_RESPONSE {
                    return Err((NetError::BodyTooLarge, false));
                }
                buf.extend_from_slice(&chunk[..size]);
                notify_inflight_buffer(&buf);
                if stop_raw_response(&buf) {
                    return Err((NetError::Cancelled, false));
                }
                if let Some((split, cl, chunked)) = header_end_and_framing(&buf) {
                    if chunked {
                        if buf[split + 4..].ends_with(b"0\r\n\r\n")
                            && decode_chunked(&buf[split + 4..]).is_ok()
                        {
                            exact_framing = true;
                            break;
                        }
                    } else if let Some(len) = cl {
                        if len > MAX_RESPONSE {
                            return Err((NetError::BodyTooLarge, false));
                        }
                        if buf.len() >= split + 4 + len {
                            exact_framing = buf.len() == split + 4 + len;
                            break;
                        }
                    }
                }
            }
            Err(err)
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
            {
                if buf.is_empty() {
                    return Err((NetError::Io(err), true));
                }
                break;
            }
            Err(err) => return Err((NetError::Io(err), buf.is_empty())),
        }
    }
    let response = parse_http_response(&buf).map_err(|e| (e, false))?;
    let close_hdr = response.headers.iter().any(|(name, value)| {
        name == "connection"
            && value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("close"))
    });
    Ok((response, exact_framing && !close_hdr))
}

fn exchange_http1_on_stream(
    stream: &mut TcpStream,
    request: &[u8],
) -> Result<(Response, bool), (NetError, bool)> {
    if let Err(err) = stream.write_all(request) {
        return Err((NetError::Io(err), true));
    }
    read_http1_response_from_stream(stream)
}

fn exchange_http1_streaming_on_stream<F: FnMut(u64, u64)>(
    stream: &mut TcpStream,
    head: &[u8],
    body: &[u8],
    on_progress: &mut F,
) -> Result<(Response, bool), (NetError, bool)> {
    if let Err(err) = stream.write_all(head) {
        return Err((NetError::Io(err), true));
    }
    if let Err(err) = write_body_chunks_stream(stream, body, on_progress) {
        return Err((NetError::Io(err), false));
    }
    read_http1_response_from_stream(stream)
}

fn http_exchange_streaming_body<F: FnMut(u64, u64)>(
    url: &Url,
    head: &[u8],
    body: &[u8],
    on_progress: &mut F,
) -> Result<Response, NetError> {
    if url.scheme != "http" && url.scheme != "https" {
        return Err(NetError::UnsupportedScheme);
    }
    if url.scheme == "http" && !https_only_policy().allows_http(&url.host) {
        let query = url
            .query
            .as_ref()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        return Err(NetError::HttpsOnly(format!(
            "{}{}{query}",
            url.authority(),
            url.path
        )));
    }
    if url.scheme == "https" {
        #[cfg(any(windows, target_os = "linux"))]
        {
            return https_exchange_tls_streaming(url, head, body, on_progress);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            return Err(NetError::UnsupportedScheme);
        }
    }

    let origin_key = url.authority();
    if let Some(mut pooled) = take_pooled_connection(&origin_key) {
        match exchange_http1_streaming_on_stream(&mut pooled, head, body, on_progress) {
            Ok((response, keep_alive)) => {
                if keep_alive {
                    put_pooled_connection(&origin_key, pooled);
                }
                return Ok(response);
            }
            Err((_, true)) => {
                rollback_reused_request();
            }
            Err((err, false)) => return Err(err),
        }
    }

    let mut stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;

    match exchange_http1_streaming_on_stream(&mut stream, head, body, on_progress) {
        Ok((response, keep_alive)) => {
            if keep_alive {
                put_pooled_connection(&origin_key, stream);
            }
            Ok(response)
        }
        Err((err, _)) => Err(err),
    }
}

/// Exchange one HTTP message over a pooled keep-alive connection, a pooled HTTP/2
/// session, or a new TCP/TLS connection.
fn http_exchange(url: &Url, request: &[u8]) -> Result<Response, NetError> {
    if url.scheme != "http" && url.scheme != "https" {
        return Err(NetError::UnsupportedScheme);
    }
    // HTTPS-only: plain http only for the hosts the person allowed.
    if url.scheme == "http" && !https_only_policy().allows_http(&url.host) {
        let query = url
            .query
            .as_ref()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        return Err(NetError::HttpsOnly(format!(
            "{}{}{query}",
            url.authority(),
            url.path
        )));
    }
    if url.scheme == "https" {
        #[cfg(any(windows, target_os = "linux"))]
        {
            return https_exchange_tls(url, request);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            return Err(NetError::UnsupportedScheme);
        }
    }

    let origin_key = url.authority();
    if let Some(mut pooled) = take_pooled_connection(&origin_key) {
        match exchange_http1_on_stream(&mut pooled, request) {
            Ok((response, keep_alive)) => {
                if keep_alive {
                    put_pooled_connection(&origin_key, pooled);
                }
                return Ok(response);
            }
            Err((_, true)) => {
                rollback_reused_request();
            }
            Err((err, false)) => return Err(err),
        }
    }

    let mut stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;

    match exchange_http1_on_stream(&mut stream, request) {
        Ok((response, keep_alive)) => {
            if keep_alive {
                put_pooled_connection(&origin_key, stream);
            }
            Ok(response)
        }
        Err((err, _)) => Err(err),
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn https_fallback_http1(url: &Url, request: &[u8]) -> Result<Response, NetError> {
    let stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    let mut session = tls::TlsSession::connect(stream, &url.host, &[])?;
    let group = session.key_exchange_group();
    session.write_all(request)?;
    let plain = tls::read_to_end(&mut session, MAX_RESPONSE)?;
    let mut response = parse_http_response(&plain)?;
    response.key_exchange = group;
    Ok(response)
}

#[cfg(any(windows, target_os = "linux"))]
fn write_tls_body_chunks<F: FnMut(u64, u64)>(
    session: &mut tls::TlsSession,
    body: &[u8],
    on_progress: &mut F,
) -> Result<(), NetError> {
    let total = body.len() as u64;
    on_progress(0, total);
    if body.is_empty() {
        return Ok(());
    }
    let mut sent = 0usize;
    while sent < body.len() {
        let end = (sent + UPLOAD_CHUNK_BYTES).min(body.len());
        session.write_all(&body[sent..end])?;
        sent = end;
        on_progress(sent as u64, total);
    }
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn report_h2_upload_progress<F: FnMut(u64, u64)>(body: &[u8], on_progress: &mut F) {
    let total = body.len() as u64;
    on_progress(0, total);
    if body.is_empty() {
        return;
    }
    let mut sent = 0usize;
    while sent < body.len() {
        sent = (sent + UPLOAD_CHUNK_BYTES).min(body.len());
        on_progress(sent as u64, total);
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn https_fallback_http1_streaming<F: FnMut(u64, u64)>(
    url: &Url,
    head: &[u8],
    body: &[u8],
    on_progress: &mut F,
) -> Result<Response, NetError> {
    let stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    let mut session = tls::TlsSession::connect(stream, &url.host, &[])?;
    let group = session.key_exchange_group();
    session.write_all(head)?;
    write_tls_body_chunks(&mut session, body, on_progress)?;
    let plain = tls::read_to_end(&mut session, MAX_RESPONSE)?;
    let mut response = parse_http_response(&plain)?;
    response.key_exchange = group;
    Ok(response)
}

#[cfg(any(windows, target_os = "linux"))]
fn https_exchange_tls_streaming<F: FnMut(u64, u64)>(
    url: &Url,
    head: &[u8],
    body: &[u8],
    on_progress: &mut F,
) -> Result<Response, NetError> {
    let origin_key = url.authority();
    let mut combined = Vec::new();
    if http2_enabled() {
        combined.reserve(head.len() + body.len());
        combined.extend_from_slice(head);
        combined.extend_from_slice(body);
        if let Some((mut h2_session, group)) = take_pooled_h2_session(&origin_key) {
            if let Some(h2_request) = h2::Request::from_http1("https", &combined)
                && let Ok(mut response) = h2_session.exchange(&h2_request, MAX_RESPONSE)
            {
                report_h2_upload_progress(body, on_progress);
                response.key_exchange = group.clone();
                put_pooled_h2_session(&origin_key, h2_session, group);
                return Ok(response);
            }
            rollback_reused_h2_stream();
        }
    }

    let stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    let alpn: &[&[u8]] = if http2_enabled() {
        &[b"h2", b"http/1.1"]
    } else {
        &[]
    };
    let mut session = tls::TlsSession::connect(stream, &url.host, alpn)?;
    let group = session.key_exchange_group();
    if session.alpn_protocol().as_deref() != Some(b"h2".as_slice()) {
        session.write_all(head)?;
        write_tls_body_chunks(&mut session, body, on_progress)?;
        let plain = tls::read_to_end(&mut session, MAX_RESPONSE)?;
        let mut response = parse_http_response(&plain)?;
        response.key_exchange = group;
        return Ok(response);
    }
    let h2_request = h2::Request::from_http1("https", &combined).ok_or(NetError::InvalidUrl)?;
    match h2::H2ClientSession::open(session) {
        Ok(mut h2_session) => match h2_session.exchange(&h2_request, MAX_RESPONSE) {
            Ok(mut response) => {
                report_h2_upload_progress(body, on_progress);
                response.key_exchange = group.clone();
                put_pooled_h2_session(&origin_key, h2_session, group);
                Ok(response)
            }
            Err(h2::H2Error::Net(err)) => Err(err),
            Err(h2::H2Error::Fallback) => {
                https_fallback_http1_streaming(url, head, body, on_progress)
            }
        },
        Err(h2::H2Error::Net(err)) => Err(err),
        Err(h2::H2Error::Fallback) => https_fallback_http1_streaming(url, head, body, on_progress),
    }
}

/// One `https` exchange on Linux or Windows, reusing a pooled `H2ClientSession`
/// when available or negotiating HTTP/2 (`h2`) via ALPN on a new TLS session.
#[cfg(any(windows, target_os = "linux"))]
fn https_exchange_tls(url: &Url, request: &[u8]) -> Result<Response, NetError> {
    let origin_key = url.authority();
    if http2_enabled()
        && let Some((mut h2_session, group)) = take_pooled_h2_session(&origin_key)
    {
        if let Some(h2_request) = h2::Request::from_http1("https", request)
            && let Ok(mut response) = h2_session.exchange(&h2_request, MAX_RESPONSE)
        {
            response.key_exchange = group.clone();
            put_pooled_h2_session(&origin_key, h2_session, group);
            return Ok(response);
        }
        rollback_reused_h2_stream();
    }

    let stream =
        connect_with_timeout(&url.host, url.port, CONNECT_TIMEOUT).map_err(NetError::Io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .map_err(NetError::Io)?;
    let alpn: &[&[u8]] = if http2_enabled() {
        &[b"h2", b"http/1.1"]
    } else {
        &[]
    };
    let mut session = tls::TlsSession::connect(stream, &url.host, alpn)?;
    let group = session.key_exchange_group();
    if session.alpn_protocol().as_deref() != Some(b"h2".as_slice()) {
        session.write_all(request)?;
        let plain = tls::read_to_end(&mut session, MAX_RESPONSE)?;
        let mut response = parse_http_response(&plain)?;
        response.key_exchange = group;
        return Ok(response);
    }
    let h2_request = h2::Request::from_http1("https", request).ok_or(NetError::InvalidUrl)?;
    match h2::H2ClientSession::open(session) {
        Ok(mut h2_session) => match h2_session.exchange(&h2_request, MAX_RESPONSE) {
            Ok(mut response) => {
                response.key_exchange = group.clone();
                put_pooled_h2_session(&origin_key, h2_session, group);
                Ok(response)
            }
            Err(h2::H2Error::Net(err)) => Err(err),
            Err(h2::H2Error::Fallback) => https_fallback_http1(url, request),
        },
        Err(h2::H2Error::Net(err)) => Err(err),
        Err(h2::H2Error::Fallback) => https_fallback_http1(url, request),
    }
}

/// Rewrite `current` to `https` when the policy asks for it. Returns the
/// original `http` URL when it did.
fn upgrade_hop(policy: &HttpsOnlyPolicy, current: &mut String) -> Option<String> {
    let upgraded = policy.upgrade(current)?;
    Some(std::mem::replace(current, upgraded))
}

/// A connection failure on an upgraded hop is reported as [`NetError::HttpsOnly`].
fn https_only_error(err: NetError, upgraded_from: Option<String>) -> NetError {
    match (err, upgraded_from) {
        (NetError::Io(_) | NetError::Certificate { .. }, Some(original)) => {
            NetError::HttpsOnly(original)
        }
        (err, _) => err,
    }
}

#[cfg(test)]
mod https_only_error_tests {
    use super::*;

    #[test]
    fn a_certificate_problem_on_an_upgraded_hop_is_still_https_only() {
        let cert = || NetError::Certificate {
            host: "a.test".to_string(),
            problem: CertProblem::Untrusted,
        };
        assert!(matches!(
            https_only_error(cert(), Some("http://a.test/".to_string())),
            NetError::HttpsOnly(url) if url == "http://a.test/"
        ));
        assert!(matches!(
            https_only_error(cert(), None),
            NetError::Certificate { .. }
        ));
    }

    #[test]
    fn tls_exceptions_never_cover_hosts_that_must_use_https() {
        let mut hsts = HstsStore::new();
        assert!(hsts.record_hsts_header("https://strict.test/", "max-age=600"));
        assert!(hsts.record_hsts_header("https://wide.test/", "max-age=600; includeSubDomains"));
        let policy = HttpsOnlyPolicy::new(true, &[])
            .with_tls_exceptions(&[
                "strict.test".to_string(),
                "nas.wide.test".to_string(),
                "github.com".to_string(),
                "plain.test".to_string(),
            ])
            .with_hsts(&hsts);
        assert!(!policy.allows_insecure_tls("strict.test"));
        assert!(!policy.allows_insecure_tls("nas.wide.test"));
        assert!(!policy.can_make_tls_exception("strict.test"));
        // Built-in HSTS hosts count too.
        assert!(!policy.allows_insecure_tls("github.com"));
        assert_eq!(
            policy.allows_insecure_tls("plain.test"),
            TLS_EXCEPTIONS_SUPPORTED
        );
        assert_eq!(
            policy.can_make_tls_exception("plain.test"),
            TLS_EXCEPTIONS_SUPPORTED
        );
    }

    #[test]
    fn tls_exceptions_cover_the_exact_host_only() {
        let policy =
            HttpsOnlyPolicy::new(true, &[]).with_tls_exceptions(&[" Intranet.Test ".to_string()]);
        assert_eq!(
            policy.allows_insecure_tls("intranet.test"),
            TLS_EXCEPTIONS_SUPPORTED
        );
        assert_eq!(
            policy.allows_insecure_tls("Intranet.Test"),
            TLS_EXCEPTIONS_SUPPORTED
        );
        assert!(!policy.allows_insecure_tls("nas.intranet.test"));
        assert!(!policy.allows_insecure_tls("badintranet.test"));
        assert!(!HttpsOnlyPolicy::default().allows_insecure_tls("intranet.test"));
        assert_eq!(
            CertProblem::from_openssl_verify(62),
            Some(CertProblem::HostnameMismatch)
        );
        assert_eq!(
            CertProblem::from_openssl_verify(10),
            Some(CertProblem::Expired)
        );
        assert_eq!(
            CertProblem::from_openssl_verify(18),
            Some(CertProblem::Untrusted)
        );
        assert_eq!(CertProblem::from_openssl_verify(0), None);
        assert_eq!(CertProblem::from_openssl_verify(28), None);
        assert_eq!(
            CertProblem::from_schannel_status(0x8009_0328),
            Some(CertProblem::Expired)
        );
        assert_eq!(CertProblem::from_schannel_status(0x8009_0308), None);
    }
}

/// GET `url`, following at most [`MAX_REDIRECTS`] redirects.
///
/// Uses a new cookie jar for this call only. That jar is discarded with the
/// function, so cookies do not survive a later [`fetch`]. No initiator is passed,
/// so `SameSite` treats every hop as same-site. The shield runs before connect.
/// An `https` to `http` redirect is [`NetError::InsecureRedirectDowngrade`].
/// The returned URL is the final request URL, which may differ after redirects.
pub fn fetch(url: &str) -> Result<(String, Response), NetError> {
    let mut ephemeral_jar = CookieJar::new();
    fetch_with_cookies(url, &mut ephemeral_jar, None, true)
}

/// Fetch a batch of subresource URLs, multiplexing same-origin `https` requests
/// over a shared [`h2::H2ClientSession`] when HTTP/2 is active and reusing pooled
/// keep-alive connections across origins.
pub fn fetch_batch(
    urls: &[String],
    initiator_url: Option<&str>,
) -> Vec<Result<(String, Response), NetError>> {
    if urls.is_empty() {
        return Vec::new();
    }
    if urls.len() == 1 {
        let mut jar = CookieJar::new();
        return vec![fetch_with_cookies(&urls[0], &mut jar, initiator_url, false)];
    }
    let policy = https_only_policy();
    let mut results: Vec<Option<Result<(String, Response), NetError>>> =
        (0..urls.len()).map(|_| None).collect();

    #[cfg(any(windows, target_os = "linux"))]
    if http2_enabled() {
        let mut origin_groups: std::collections::HashMap<String, Vec<(usize, String, Url)>> =
            std::collections::HashMap::new();
        for (idx, raw_url) in urls.iter().enumerate() {
            let mut candidate = raw_url.trim().to_string();
            let _ = upgrade_hop(&policy, &mut candidate);
            if is_url_blocked_by_default_shield(&candidate, initiator_url).is_some() {
                continue;
            }
            if let Ok(parsed) = parse_url(&candidate)
                && parsed.scheme == "https"
            {
                origin_groups
                    .entry(parsed.authority())
                    .or_default()
                    .push((idx, candidate, parsed));
            }
        }
        for (origin_key, group_items) in origin_groups {
            if group_items.len() < 2 {
                continue;
            }
            let mut h2_reqs = Vec::with_capacity(group_items.len());
            for (_, _, parsed) in &group_items {
                let raw_req = build_request_with_cookie(parsed, None);
                if let Some(req) = h2::Request::from_http1("https", raw_req.as_bytes()) {
                    h2_reqs.push(req);
                }
            }
            if h2_reqs.len() != group_items.len() {
                continue;
            }
            let session_pair = take_pooled_h2_session(&origin_key).or_else(|| {
                let first = &group_items[0].2;
                let stream = connect_with_timeout(&first.host, first.port, CONNECT_TIMEOUT).ok()?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
                let tls_sess =
                    tls::TlsSession::connect(stream, &first.host, &[b"h2", b"http/1.1"]).ok()?;
                if tls_sess.alpn_protocol().as_deref() != Some(b"h2".as_slice()) {
                    return None;
                }
                let group = tls_sess.key_exchange_group();
                let h2_sess = h2::H2ClientSession::open(tls_sess).ok()?;
                Some((h2_sess, group))
            });
            if let Some((mut h2_session, group)) = session_pair
                && let Ok(responses) = h2_session.exchange_multiplexed(&h2_reqs, MAX_RESPONSE)
            {
                record_multiplexed_streams(h2_reqs.len());
                put_pooled_h2_session(&origin_key, h2_session, group.clone());
                for ((idx, final_url, _), mut resp) in group_items.into_iter().zip(responses) {
                    resp.key_exchange = group.clone();
                    if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                        results[idx] = Some(Ok((final_url, resp)));
                    }
                }
            }
        }
    }

    for (idx, raw_url) in urls.iter().enumerate() {
        if results[idx].is_none() {
            let mut jar = CookieJar::new();
            results[idx] = Some(fetch_with_cookies(raw_url, &mut jar, initiator_url, false));
        }
    }
    results
        .into_iter()
        .map(|slot| slot.unwrap_or(Err(NetError::InvalidUrl)))
        .collect()
}

/// GET `url` with `jar`, following at most [`MAX_REDIRECTS`] redirects.
///
/// The shield is checked on every hop before [`http_get_with_cookie`] connects.
/// `Set-Cookie` from each response, including redirect responses, is stored in
/// `jar` before the next hop. `is_top_level_get` is forwarded to the cookie
/// header so `SameSite=Lax` can be included on a cross-site top-level GET.
/// A missing `Location` on a redirect status is [`NetError::InvalidResponse`].
/// A `Location` that does not resolve is [`NetError::InvalidUrl`]. Status codes
/// other than 301, 302, 303, 307, and 308 are returned as-is, including errors.
///
/// Cookies are partitioned by the top-level page. For a top-level GET that is
/// each hop's own URL. Otherwise it is `initiator_url`, the page the request
/// came from. The thread's [`HttpsOnlyPolicy`] upgrades `http` hops to `https`;
/// an upgraded hop that cannot connect is [`NetError::HttpsOnly`].
pub fn fetch_with_cookies(
    url: &str,
    jar: &mut CookieJar,
    initiator_url: Option<&str>,
    is_top_level_get: bool,
) -> Result<(String, Response), NetError> {
    let policy = https_only_policy();
    let mut current = url.trim().to_string();
    for _ in 0..=MAX_REDIRECTS {
        let upgraded_from = upgrade_hop(&policy, &mut current);
        let parsed = parse_url(&current)?;
        // Block before DNS and TCP. A shield hit never reaches http_get_with_cookie.
        if let Some(reason) = is_url_blocked_by_default_shield(&current, initiator_url) {
            return Err(NetError::BlockedByShield(reason.to_string()));
        }
        let top_level = if is_top_level_get {
            current.clone()
        } else {
            initiator_url.unwrap_or(&current).to_string()
        };
        let cookie_hdr =
            jar.http_cookie_header_under(&current, &top_level, initiator_url, is_top_level_get);
        let response = http_get_with_cookie(&parsed, cookie_hdr.as_deref())
            .map_err(|err| https_only_error(err, upgraded_from))?;
        for set_cookie in response.headers_named("set-cookie") {
            jar.store_set_cookie_under(&current, &top_level, set_cookie);
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .header("location")
                .ok_or(NetError::InvalidResponse)?;
            let next = resolve_url(&current, location).ok_or(NetError::InvalidUrl)?;
            let next_parsed = parse_url(&next)?;
            // Refuse an https to http downgrade. http to https is allowed, and so
            // is a hop that HTTPS-only will upgrade anyway.
            if parsed.scheme == "https"
                && next_parsed.scheme == "http"
                && policy.upgrade(&next).is_none()
            {
                return Err(NetError::InsecureRedirectDowngrade);
            }
            current = next;
            continue;
        }
        return Ok((current, response));
    }
    // The loop bound is the first response plus MAX_REDIRECTS hops.
    Err(NetError::TooManyRedirects)
}

/// POST `url`, following at most [`MAX_REDIRECTS`] redirects.
///
/// Uses a new cookie jar for this call only. 301, 302, and 303 switch the next
/// hop to GET and drop the body. 307 and 308 send the same body again. An
/// `https` to `http` redirect is refused. The shield runs before each connect.
pub fn fetch_post(
    url: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(String, Response), NetError> {
    let mut ephemeral_jar = CookieJar::new();
    fetch_post_with_cookies(url, content_type, body, &mut ephemeral_jar, None)
}

/// POST `url` with `jar`, following at most [`MAX_REDIRECTS`] redirects.
///
/// The shield is checked before every connect. `Set-Cookie` is stored before
/// the next hop. 301, 302, and 303 set the next method to GET. On those GET
/// hops the cookie header is built as a top-level GET, so cross-site
/// `SameSite=Lax` cookies are included. 307 and 308 keep POST, and the cookie
/// header is not treated as a top-level GET. The original body is reused for
/// repeated POST hops. It is not sent after the method switches to GET.
///
/// Cookies are partitioned with each hop as its own top-level page, because a
/// form POST is a navigation. HTTPS-only applies the same way as in
/// [`fetch_with_cookies`].
pub fn fetch_post_with_cookies(
    url: &str,
    content_type: &str,
    body: &[u8],
    jar: &mut CookieJar,
    initiator_url: Option<&str>,
) -> Result<(String, Response), NetError> {
    let policy = https_only_policy();
    let mut current = url.trim().to_string();
    let mut use_post = true;
    for _ in 0..=MAX_REDIRECTS {
        let upgraded_from = upgrade_hop(&policy, &mut current);
        let parsed = parse_url(&current)?;
        // Block before DNS and TCP. A shield hit never reaches the POST or GET exchange.
        if let Some(reason) = is_url_blocked_by_default_shield(&current, initiator_url) {
            return Err(NetError::BlockedByShield(reason.to_string()));
        }
        // A form POST is a navigation, so each hop is its own top-level page.
        // After a 301, 302, or 303, !use_post is true, so Lax cookies are sent.
        let cookie_hdr = jar.http_cookie_header_under(&current, &current, initiator_url, !use_post);
        let response = if use_post {
            http_post_with_cookie(&parsed, content_type, body, cookie_hdr.as_deref())
        } else {
            http_get_with_cookie(&parsed, cookie_hdr.as_deref())
        }
        .map_err(|err| https_only_error(err, upgraded_from))?;
        for set_cookie in response.headers_named("set-cookie") {
            jar.store_set_cookie_under(&current, &current, set_cookie);
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            // 301, 302, and 303 drop the POST body. 307 and 308 keep it.
            if matches!(response.status, 301..=303) {
                use_post = false;
            }
            let location = response
                .header("location")
                .ok_or(NetError::InvalidResponse)?;
            let next = resolve_url(&current, location).ok_or(NetError::InvalidUrl)?;
            let next_parsed = parse_url(&next)?;
            // Refuse an https to http downgrade unless HTTPS-only upgrades the hop.
            if parsed.scheme == "https"
                && next_parsed.scheme == "http"
                && policy.upgrade(&next).is_none()
            {
                return Err(NetError::InsecureRedirectDowngrade);
            }
            current = next;
            continue;
        }
        return Ok((current, response));
    }
    // The loop bound is the first response plus MAX_REDIRECTS hops.
    Err(NetError::TooManyRedirects)
}

/// Replace any existing query with `query`.
///
/// The fragment is not handled. An empty `query` returns the URL with no `?`.
/// `query` is not encoded.
pub fn append_query(url: &str, query: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    }
}

/// Build an HTTP/1.1 GET. `Connection: close` is always set. No `Referer` is sent.
///
/// An empty or missing cookie header omits the `Cookie` line. The caller is
/// responsible for cookie flags. This function does not inspect them.
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
    let user_agent = user_agent_for_host(&url.host);
    format!(
        "GET {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {user_agent}\r\nAccept: text/html,text/css,image/webp,image/png,image/jpeg,image/gif,image/x-icon,image/bmp,*/*\r\nAccept-Encoding: gzip, deflate, br, identity\r\nReferrer-Policy: no-referrer\r\nDNT: 1\r\nX-Content-Type-Options: nosniff\r\n{cookie_line}Connection: close\r\n\r\n"
    )
}

/// POST request bytes with no `Cookie` header. The body is copied after the header.
pub fn build_post_request(url: &Url, content_type: &str, body: &[u8]) -> Vec<u8> {
    build_post_request_with_cookie(url, content_type, body, None)
}

/// POST request bytes. `Connection: close` is always set. No `Referer` is sent.
///
/// CR, LF, and other control characters are removed from `content_type` so the
/// value cannot add another header. `Content-Length` is `body.len()`. An empty
/// or missing cookie header omits the `Cookie` line.
pub fn build_post_request_with_cookie(
    url: &Url,
    content_type: &str,
    body: &[u8],
    cookie_header: Option<&str>,
) -> Vec<u8> {
    let mut head = build_post_headers_with_cookie(url, content_type, body.len(), cookie_header);
    head.extend_from_slice(body);
    head
}

fn build_post_headers_with_cookie(
    url: &Url,
    content_type: &str,
    content_length: usize,
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
    // Strip controls so Content-Type cannot inject another header line.
    let safe_ct: String = content_type
        .chars()
        .filter(|&c| c != '\r' && c != '\n' && !c.is_control())
        .collect();
    let cookie_line = match cookie_header {
        Some(c) if !c.is_empty() => format!("Cookie: {c}\r\n"),
        _ => String::new(),
    };
    let user_agent = user_agent_for_host(&url.host);
    format!(
        "POST {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {user_agent}\r\nAccept: text/html,text/css,image/webp,image/png,image/jpeg,image/gif,image/x-icon,image/bmp,*/*\r\nAccept-Encoding: gzip, deflate, br, identity\r\nReferrer-Policy: no-referrer\r\nDNT: 1\r\nX-Content-Type-Options: nosniff\r\nContent-Type: {safe_ct}\r\nContent-Length: {content_length}\r\n{cookie_line}Connection: close\r\n\r\n"
    )
    .into_bytes()
}

/// Sanitize a header name or value by stripping CR, LF, and control characters.
fn sanitize_header_field(raw: &str) -> String {
    raw.chars()
        .filter(|&c| c != '\r' && c != '\n' && !c.is_control())
        .collect()
}

/// True when `name` is a Fetch-forbidden request header name that scripts may not override.
pub fn is_forbidden_script_request_header(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "accept-charset"
            | "accept-encoding"
            | "access-control-request-headers"
            | "access-control-request-method"
            | "connection"
            | "content-length"
            | "cookie"
            | "cookie2"
            | "date"
            | "dnt"
            | "expect"
            | "host"
            | "keep-alive"
            | "origin"
            | "referer"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "via"
    ) || lower.starts_with("proxy-")
        || lower.starts_with("sec-")
}

/// Build an HTTP/1.1 request with custom method, extra headers, optional body, and optional `Cookie`.
pub fn build_custom_request_with_cookie(
    method: &str,
    url: &Url,
    extra_headers: &[(String, String)],
    body: &[u8],
    cookie_header: Option<&str>,
) -> Vec<u8> {
    let safe_method: String = method
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let method_token = if safe_method.is_empty() {
        "GET".to_string()
    } else {
        safe_method.to_ascii_uppercase()
    };
    let path = if url.path.is_empty() {
        "/"
    } else {
        url.path.as_str()
    };
    let query = url
        .query
        .as_ref()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let host = if Url::default_port(&url.scheme) == Some(url.port) {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let user_agent = user_agent_for_host(&url.host);
    let mut extra_lines = String::new();
    let mut has_accept = false;
    for (k, v) in extra_headers {
        let name = sanitize_header_field(k.trim());
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host" | "connection" | "content-length" | "cookie" | "transfer-encoding"
        ) {
            continue;
        }
        if lower == "accept" {
            has_accept = true;
        }
        let val = sanitize_header_field(v.trim());
        extra_lines.push_str(&format!("{name}: {val}\r\n"));
    }
    let accept_line = if has_accept { "" } else { "Accept: */*\r\n" };
    let content_length_line =
        if !body.is_empty() || matches!(method_token.as_str(), "POST" | "PUT" | "PATCH") {
            format!("Content-Length: {}\r\n", body.len())
        } else {
            String::new()
        };
    let cookie_line = match cookie_header {
        Some(c) if !c.is_empty() => format!("Cookie: {c}\r\n"),
        _ => String::new(),
    };
    let mut bytes = format!(
        "{method_token} {path}{query} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {user_agent}\r\n{accept_line}Accept-Encoding: gzip, deflate, br, identity\r\nReferrer-Policy: no-referrer\r\nDNT: 1\r\nX-Content-Type-Options: nosniff\r\n{extra_lines}{content_length_line}{cookie_line}Connection: close\r\n\r\n"
    )
    .into_bytes();
    if !body.is_empty() && !matches!(method_token.as_str(), "GET" | "HEAD") {
        bytes.extend_from_slice(body);
    }
    bytes
}

/// Execute an HTTP/1.1 or HTTP/2 request with custom method, headers, body, and optional `Cookie`.
pub fn http_request_with_headers(
    method: &str,
    url: &Url,
    extra_headers: &[(String, String)],
    body: &[u8],
    cookie_header: Option<&str>,
) -> Result<Response, NetError> {
    let req = build_custom_request_with_cookie(method, url, extra_headers, body, cookie_header);
    http_exchange(url, &req)
}

/// Origin serialized as `scheme://host` or `scheme://host:port` (when non-default).
pub fn origin_string(url: &Url) -> String {
    if Url::default_port(&url.scheme) == Some(url.port) {
        format!("{}://{}", url.scheme, url.host)
    } else {
        format!("{}://{}:{}", url.scheme, url.host, url.port)
    }
}

/// True when `method` is a CORS-safelisted method (`GET`, `HEAD`, `POST`).
pub fn is_cors_safelisted_method(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "POST"
    )
}

/// True when `(name, value)` is a CORS-safelisted request header.
pub fn is_cors_safelisted_request_header(name: &str, value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.trim().to_ascii_lowercase().as_str() {
        "accept" | "accept-language" | "content-language" => true,
        "content-type" => {
            let essence = value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            matches!(
                essence.as_str(),
                "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
            )
        }
        _ => false,
    }
}

/// True when a cross-origin request with `method` and `headers` requires an `OPTIONS` preflight.
pub fn requires_cors_preflight(method: &str, headers: &[(String, String)]) -> bool {
    !is_cors_safelisted_method(method)
        || headers
            .iter()
            .any(|(k, v)| !is_cors_safelisted_request_header(k, v))
}

/// Validate `Access-Control-Allow-Origin` and `Access-Control-Allow-Credentials` on a cross-origin response.
pub fn validate_cors_response(
    response: &Response,
    origin: &str,
    credentials_include: bool,
) -> Result<(), String> {
    let acao = response
        .header("access-control-allow-origin")
        .map(str::trim)
        .unwrap_or("");
    if acao.is_empty() {
        return Err("CORS blocked: missing Access-Control-Allow-Origin".to_string());
    }
    if credentials_include {
        if acao == "*" {
            return Err(
                "CORS blocked: wildcard Access-Control-Allow-Origin not allowed with credentials"
                    .to_string(),
            );
        }
        if !acao.eq_ignore_ascii_case(origin) {
            return Err(format!(
                "CORS blocked: Access-Control-Allow-Origin '{acao}' does not match '{origin}'"
            ));
        }
        let acac = response
            .header("access-control-allow-credentials")
            .map(str::trim)
            .unwrap_or("");
        if acac != "true" {
            return Err(
                "CORS blocked: Access-Control-Allow-Credentials must be 'true' when credentials are included"
                    .to_string(),
            );
        }
    } else if acao != "*" && !acao.eq_ignore_ascii_case(origin) {
        return Err(format!(
            "CORS blocked: Access-Control-Allow-Origin '{acao}' does not match '{origin}'"
        ));
    }
    Ok(())
}

/// Validate an `OPTIONS` CORS preflight response against `origin`, `method`, `req_headers`, and `credentials_include`.
pub fn validate_cors_preflight(
    response: &Response,
    origin: &str,
    method: &str,
    req_headers: &[(String, String)],
    credentials_include: bool,
) -> Result<(), String> {
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "CORS preflight failed with status {}",
            response.status
        ));
    }
    validate_cors_response(response, origin, credentials_include)?;
    let method_upper = method.trim().to_ascii_uppercase();
    if !is_cors_safelisted_method(&method_upper) {
        let allow_methods: Vec<String> = response
            .headers_named("access-control-allow-methods")
            .into_iter()
            .flat_map(|v| v.split(','))
            .map(|t| t.trim().to_ascii_uppercase())
            .filter(|t| !t.is_empty())
            .collect();
        let method_ok = (!credentials_include && allow_methods.iter().any(|m| m == "*"))
            || allow_methods.iter().any(|m| m == &method_upper);
        if !method_ok {
            return Err(format!("CORS preflight blocked method '{method_upper}'"));
        }
    }
    let non_safelisted: Vec<String> = req_headers
        .iter()
        .filter(|(k, v)| !is_cors_safelisted_request_header(k, v))
        .map(|(k, _)| k.trim().to_ascii_lowercase())
        .collect();
    if !non_safelisted.is_empty() {
        let allow_headers: Vec<String> = response
            .headers_named("access-control-allow-headers")
            .into_iter()
            .flat_map(|v| v.split(','))
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let wildcard = !credentials_include && allow_headers.iter().any(|h| h == "*");
        for needed in &non_safelisted {
            let allowed = if needed == "authorization" {
                allow_headers.iter().any(|h| h == "authorization")
            } else {
                wildcard || allow_headers.iter().any(|h| h == needed)
            };
            if !allowed {
                return Err(format!("CORS preflight blocked request header '{needed}'"));
            }
        }
    }
    Ok(())
}

/// Filter response headers on a cross-origin response down to CORS-safelisted
/// response headers plus any headers listed in `Access-Control-Expose-Headers`.
pub fn filter_cors_exposed_headers(
    headers: &[(String, String)],
    credentials_include: bool,
) -> Vec<(String, String)> {
    let mut exposed: Vec<String> = Vec::new();
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("access-control-expose-headers") {
            for token in v.split(',') {
                let t = token.trim().to_ascii_lowercase();
                if !t.is_empty() {
                    exposed.push(t);
                }
            }
        }
    }
    let wildcard = !credentials_include && exposed.iter().any(|h| h == "*");
    headers
        .iter()
        .filter(|(k, _)| {
            let lower = k.trim().to_ascii_lowercase();
            if lower == "set-cookie" || lower == "set-cookie2" {
                return false;
            }
            if wildcard {
                return true;
            }
            matches!(
                lower.as_str(),
                "cache-control"
                    | "content-language"
                    | "content-length"
                    | "content-type"
                    | "expires"
                    | "last-modified"
                    | "pragma"
            ) || exposed.iter().any(|h| h == &lower)
        })
        .cloned()
        .collect()
}

/// `User-Agent` for one request.
///
/// Reddit refuses an empty or generic agent and asks for a unique one, so
/// those hosts get a longer name that still says `anvil-browser/23.3.1`.
/// OpenStreetMap tile and place lookups get the same longer name. Every
/// other host keeps the short name. A name copied from another browser is not
/// used, because Gmail would then hide the HTML sign-in form.
fn user_agent_for_host(host: &str) -> &'static str {
    let host = host.trim().to_ascii_lowercase();
    if host == "reddit.com"
        || host.ends_with(".reddit.com")
        || host == "tile.openstreetmap.org"
        || host == "nominatim.openstreetmap.org"
    {
        "anvil-browser/23.3.1 (desktop; +https://github.com/buntatoes/anvil-browser)"
    } else {
        "anvil-browser/23.3.1"
    }
}

/// Priority tier assigned to a discovered HTML subresource for HTTP/2 stream
/// prioritization and connection-pool scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubresourcePriority {
    /// Render-blocking stylesheets (`<link rel="stylesheet">` or `<link rel="preload" as="style">`).
    CriticalStyle = 0,
    /// Preloaded web fonts (`<link rel="preload" as="font">`).
    HighPriorityFont = 1,
    /// Parser-blocking scripts (`<script src="...">` without `async` or `defer`).
    BlockingScript = 2,
    /// Eager / above-the-fold images (`<img src="...">` without `loading="lazy"`).
    AboveFoldImage = 3,
    /// Deferred or asynchronous scripts (`<script async|defer src="...">`).
    AsyncScript = 4,
    /// Lazy-loaded images or media (`<img loading="lazy" src="...">`).
    LazyImageOrMedia = 5,
}

impl SubresourcePriority {
    /// Convert this subresource priority tier into an HTTP/2 [`StreamPriority`]
    /// descriptor carrying RFC 9113 stream weight and RFC 9218 extensible urgency.
    pub fn stream_priority(self) -> h2::StreamPriority {
        match self {
            Self::CriticalStyle => h2::StreamPriority::critical_style(),
            Self::HighPriorityFont => h2::StreamPriority {
                exclusive: false,
                stream_dependency: 0,
                weight: 240,
                urgency: 1,
                incremental: false,
            },
            Self::BlockingScript => h2::StreamPriority::high_script(),
            Self::AboveFoldImage => h2::StreamPriority::eager_image(),
            Self::AsyncScript => h2::StreamPriority {
                exclusive: false,
                stream_dependency: 0,
                weight: 96,
                urgency: 4,
                incremental: true,
            },
            Self::LazyImageOrMedia => h2::StreamPriority::low_background(),
        }
    }
}

/// Kind of subresource discovered during an HTML preload/prefetch scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubresourceKind {
    Stylesheet,
    Font,
    Script,
    Image,
}

/// A subresource discovered in an HTML document along with its resolved URL and priority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubresourceDescriptor {
    pub url: String,
    pub kind: SubresourceKind,
    pub priority: SubresourcePriority,
}

fn extract_tag_attr(tag_body: &str, attr_name: &str) -> Option<String> {
    let bytes = tag_body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = &tag_body[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let val = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let out = tag_body[v_start..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
                out
            } else {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                tag_body[v_start..i].to_string()
            }
        } else {
            String::new()
        };
        if name.eq_ignore_ascii_case(attr_name) {
            return Some(val);
        }
    }
    None
}

fn resolve_discovered_subresource_url(base_url: &str, raw_ref: &str) -> Option<String> {
    let trimmed = raw_ref.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("data:")
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with('#')
    {
        return None;
    }
    if !base_url.trim().is_empty() {
        resolve_url(base_url, trimmed)
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Scan raw HTML markup for `<link>`, `<script src>`, and `<img src>` subresources,
/// resolve them against `base_url`, deduplicate by URL (keeping the highest priority),
/// and return them ordered from highest priority ([`SubresourcePriority::CriticalStyle`])
/// to lowest ([`SubresourcePriority::LazyImageOrMedia`]).
pub fn discover_html_subresources(html: &str, base_url: &str) -> Vec<SubresourceDescriptor> {
    let mut items: Vec<(usize, SubresourceDescriptor)> = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel_lt) = html[i..].find('<') else {
            break;
        };
        let lt = i + rel_lt;
        if html[lt..].starts_with("<!--") {
            if let Some(end_comment) = html[lt + 4..].find("-->") {
                i = lt + 4 + end_comment + 3;
                continue;
            } else {
                break;
            }
        }
        let Some(rel_gt) = html[lt + 1..].find('>') else {
            break;
        };
        let gt = lt + 1 + rel_gt;
        let tag_slice = html[lt + 1..gt].trim();
        i = gt + 1;
        if tag_slice.is_empty() || tag_slice.starts_with('/') || tag_slice.starts_with('!') {
            continue;
        }
        let tag_end = tag_slice
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(tag_slice.len());
        let tag_name = &tag_slice[..tag_end];
        let attrs = &tag_slice[tag_end..];
        let candidate = if tag_name.eq_ignore_ascii_case("link") {
            let rel = extract_tag_attr(attrs, "rel").unwrap_or_default();
            let href = extract_tag_attr(attrs, "href").unwrap_or_default();
            let rel_tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
            if rel_tokens
                .iter()
                .any(|t| t.eq_ignore_ascii_case("stylesheet"))
            {
                resolve_discovered_subresource_url(base_url, &href).map(|url| {
                    SubresourceDescriptor {
                        url,
                        kind: SubresourceKind::Stylesheet,
                        priority: SubresourcePriority::CriticalStyle,
                    }
                })
            } else if rel_tokens.iter().any(|t| t.eq_ignore_ascii_case("preload")) {
                let as_attr = extract_tag_attr(attrs, "as").unwrap_or_default();
                let (kind, prio) = if as_attr.eq_ignore_ascii_case("style") {
                    (
                        SubresourceKind::Stylesheet,
                        SubresourcePriority::CriticalStyle,
                    )
                } else if as_attr.eq_ignore_ascii_case("font") {
                    (SubresourceKind::Font, SubresourcePriority::HighPriorityFont)
                } else if as_attr.eq_ignore_ascii_case("script") {
                    (SubresourceKind::Script, SubresourcePriority::BlockingScript)
                } else {
                    (SubresourceKind::Image, SubresourcePriority::AboveFoldImage)
                };
                resolve_discovered_subresource_url(base_url, &href).map(|url| {
                    SubresourceDescriptor {
                        url,
                        kind,
                        priority: prio,
                    }
                })
            } else {
                None
            }
        } else if tag_name.eq_ignore_ascii_case("script") {
            let src = extract_tag_attr(attrs, "src").unwrap_or_default();
            let is_async = extract_tag_attr(attrs, "async").is_some()
                || extract_tag_attr(attrs, "defer").is_some();
            let prio = if is_async {
                SubresourcePriority::AsyncScript
            } else {
                SubresourcePriority::BlockingScript
            };
            resolve_discovered_subresource_url(base_url, &src).map(|url| SubresourceDescriptor {
                url,
                kind: SubresourceKind::Script,
                priority: prio,
            })
        } else if tag_name.eq_ignore_ascii_case("img") {
            let src = extract_tag_attr(attrs, "src").unwrap_or_default();
            let loading = extract_tag_attr(attrs, "loading").unwrap_or_default();
            let fetch_prio = extract_tag_attr(attrs, "fetchpriority").unwrap_or_default();
            let prio =
                if loading.eq_ignore_ascii_case("lazy") || fetch_prio.eq_ignore_ascii_case("low") {
                    SubresourcePriority::LazyImageOrMedia
                } else {
                    SubresourcePriority::AboveFoldImage
                };
            resolve_discovered_subresource_url(base_url, &src).map(|url| SubresourceDescriptor {
                url,
                kind: SubresourceKind::Image,
                priority: prio,
            })
        } else {
            None
        };

        if let Some(desc) = candidate {
            if let Some((_, existing)) = items.iter_mut().find(|(_, d)| d.url == desc.url) {
                if desc.priority < existing.priority {
                    existing.priority = desc.priority;
                    existing.kind = desc.kind;
                }
            } else {
                let order = items.len();
                items.push((order, desc));
            }
        }
    }
    items.sort_by_key(|(order, desc)| (desc.priority, *order));
    items.into_iter().map(|(_, desc)| desc).collect()
}

/// Fetch a prioritized batch of `(url, SubresourcePriority)` items, dispatching
/// highest-priority subresources first and attaching HTTP/2 RFC 9113/9218 stream
/// priority metadata on multiplexed `https` streams while returning results in the
/// caller's original slice order.
pub fn fetch_batch_prioritized(
    items: &[(String, SubresourcePriority)],
    initiator_url: Option<&str>,
) -> Vec<Result<(String, Response), NetError>> {
    if items.is_empty() {
        return Vec::new();
    }
    if items.len() == 1 {
        let mut jar = CookieJar::new();
        return vec![fetch_with_cookies(
            &items[0].0,
            &mut jar,
            initiator_url,
            false,
        )];
    }
    let policy = https_only_policy();
    let mut results: Vec<Option<Result<(String, Response), NetError>>> =
        (0..items.len()).map(|_| None).collect();

    // Order indices by priority (CriticalStyle -> LazyImageOrMedia), preserving original order on ties.
    let mut ordered_indices: Vec<usize> = (0..items.len()).collect();
    ordered_indices.sort_by_key(|&idx| (items[idx].1, idx));

    #[cfg(any(windows, target_os = "linux"))]
    if http2_enabled() {
        let mut origin_groups: std::collections::HashMap<
            String,
            Vec<(usize, String, Url, SubresourcePriority)>,
        > = std::collections::HashMap::new();
        for &idx in &ordered_indices {
            let (ref raw_url, prio) = items[idx];
            let mut candidate = raw_url.trim().to_string();
            let _ = upgrade_hop(&policy, &mut candidate);
            if is_url_blocked_by_default_shield(&candidate, initiator_url).is_some() {
                continue;
            }
            if let Ok(parsed) = parse_url(&candidate)
                && parsed.scheme == "https"
            {
                origin_groups
                    .entry(parsed.authority())
                    .or_default()
                    .push((idx, candidate, parsed, prio));
            }
        }
        for (origin_key, group_items) in origin_groups {
            if group_items.len() < 2 {
                continue;
            }
            let mut h2_reqs = Vec::with_capacity(group_items.len());
            for (_, _, parsed, prio) in &group_items {
                let raw_req = build_request_with_cookie(parsed, None);
                if let Some(mut req) = h2::Request::from_http1("https", raw_req.as_bytes()) {
                    let stream_prio = prio.stream_priority();
                    req.headers
                        .push(("priority".to_string(), stream_prio.priority_header_value()));
                    h2_reqs.push((req, stream_prio));
                }
            }
            if h2_reqs.len() != group_items.len() {
                continue;
            }
            let session_pair = take_pooled_h2_session(&origin_key).or_else(|| {
                let first = &group_items[0].2;
                let stream = connect_with_timeout(&first.host, first.port, CONNECT_TIMEOUT).ok()?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
                let tls_sess =
                    tls::TlsSession::connect(stream, &first.host, &[b"h2", b"http/1.1"]).ok()?;
                if tls_sess.alpn_protocol().as_deref() != Some(b"h2".as_slice()) {
                    return None;
                }
                let group = tls_sess.key_exchange_group();
                let h2_sess = h2::H2ClientSession::open(tls_sess).ok()?;
                Some((h2_sess, group))
            });
            if let Some((mut h2_session, group)) = session_pair
                && let Ok(responses) =
                    h2_session.exchange_multiplexed_prioritized(&h2_reqs, MAX_RESPONSE)
            {
                record_multiplexed_streams(h2_reqs.len());
                put_pooled_h2_session(&origin_key, h2_session, group.clone());
                for ((idx, final_url, _, _), mut resp) in group_items.into_iter().zip(responses) {
                    resp.key_exchange = group.clone();
                    if !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                        results[idx] = Some(Ok((final_url, resp)));
                    }
                }
            }
        }
    }

    for idx in ordered_indices {
        if results[idx].is_none() {
            let mut jar = CookieJar::new();
            results[idx] = Some(fetch_with_cookies(
                &items[idx].0,
                &mut jar,
                initiator_url,
                false,
            ));
        }
    }
    results
        .into_iter()
        .map(|slot| slot.unwrap_or(Err(NetError::InvalidUrl)))
        .collect()
}

#[cfg(test)]
mod tests {
    //! URL parsing, response decoding, cookie rules, the pre-connect shield,
    //! HSTS upgrades, and SHA-256 integrity checks. The shield case expects
    //! [`fetch`] to refuse a blocked host before a document is returned.

    use super::*;

    #[test]
    fn only_ml_kem_groups_count_as_post_quantum() {
        assert!(is_post_quantum_group("X25519MLKEM768"));
        assert!(is_post_quantum_group("SecP256r1MLKEM768"));
        assert!(is_post_quantum_group("MLKEM1024"));
        for classical in [
            "x25519",
            "X25519",
            "prime256v1",
            "secp384r1",
            "ffdhe2048",
            "",
        ] {
            assert!(!is_post_quantum_group(classical), "{classical}");
        }
    }

    #[test]
    fn recent_key_exchanges_are_newest_first_and_capped() {
        for i in 0..MAX_RECENT_KEY_EXCHANGES + 3 {
            record_key_exchange(&format!("cap-test-{i}.example"), "x25519");
        }
        record_key_exchange("newest.example", "X25519MLKEM768");
        let recent = recent_key_exchanges();
        assert_eq!(recent.len(), MAX_RECENT_KEY_EXCHANGES);
        assert_eq!(recent[0].host, "newest.example");
        assert!(recent[0].is_post_quantum());
        assert!(!recent[1].is_post_quantum());
    }

    #[test]
    fn plain_http_responses_carry_no_key_exchange() {
        let resp = parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!(resp.key_exchange, None);
    }

    #[test]
    fn reddit_user_agent_names_this_browser_and_other_hosts_stay_short() {
        let reddit = parse_url("https://www.reddit.com/r/rust").unwrap();
        let req = build_request_with_cookie(&reddit, None);
        assert!(req.contains(
            "User-Agent: anvil-browser/23.3.1 (desktop; +https://github.com/buntatoes/anvil-browser)\r\n"
        ));
        let old = parse_url("https://old.reddit.com/r/rust").unwrap();
        let req = build_request_with_cookie(&old, None);
        assert!(req.contains(
            "User-Agent: anvil-browser/23.3.1 (desktop; +https://github.com/buntatoes/anvil-browser)\r\n"
        ));
        let bank = parse_url("https://www.chase.com/").unwrap();
        let req = build_request_with_cookie(&bank, None);
        assert!(req.contains("User-Agent: anvil-browser/23.3.1\r\n"));
        assert!(!req.contains("github.com"));
        let tiles = parse_url("https://tile.openstreetmap.org/2/1/1.png").unwrap();
        let req = build_request_with_cookie(&tiles, None);
        assert!(req.contains(
            "User-Agent: anvil-browser/23.3.1 (desktop; +https://github.com/buntatoes/anvil-browser)\r\n"
        ));
        let other = parse_url("https://example.com/").unwrap();
        let req = build_request_with_cookie(&other, None);
        assert!(req.contains("User-Agent: anvil-browser/23.3.1\r\n"));
        assert!(!req.contains("github.com"));
    }

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

        let ipv6 = parse_url("http://[::1]:8080/ipv6?ok=1").unwrap();
        assert_eq!(ipv6.scheme, "http");
        assert_eq!(ipv6.host, "[::1]");
        assert_eq!(ipv6.port, 8080);
        assert_eq!(ipv6.path, "/ipv6");
        assert_eq!(ipv6.authority(), "http://[::1]:8080");
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

        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nAnvil\r\n7\r\nBrowser\r\n0\r\n\r\n";
        let decoded = parse_http_response(chunked).unwrap();
        assert_eq!(decoded.body, b"AnvilBrowser");

        let gz_payload = anvil_compress::compress_gzip(b"<h1>Compressed Anvil</h1>");
        let mut gz_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n",
            gz_payload.len()
        )
        .into_bytes();
        gz_http.extend_from_slice(&gz_payload);
        let gz_resp = parse_http_response(&gz_http).unwrap();
        assert_eq!(gz_resp.body, b"<h1>Compressed Anvil</h1>");

        let br_payload = brotli::compress_brotli(b"<h1>Brotli Anvil</h1>");
        let mut br_http = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: br\r\nContent-Length: {}\r\n\r\n",
            br_payload.len()
        )
        .into_bytes();
        br_http.extend_from_slice(&br_payload);
        let br_resp = parse_http_response(&br_http).unwrap();
        assert_eq!(br_resp.body, b"<h1>Brotli Anvil</h1>");
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
        let req = build_post_request(&url, "application/x-www-form-urlencoded", b"q=anvil&page=2");
        let text = String::from_utf8(req).unwrap();
        assert!(text.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(text.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(text.contains("Content-Length: 14\r\n"));
        assert!(text.ends_with("\r\n\r\nq=anvil&page=2"));
        assert_eq!(
            append_query("http://example.com/search?old=1", "q=anvil"),
            "http://example.com/search?q=anvil"
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
            resolve_url("https://example.com/index.html", "anvil://security"),
            None
        );
        assert_eq!(
            resolve_url("https://example.com/index.html", "file:///etc/passwd"),
            None
        );
        assert_eq!(
            resolve_url("anvil://home", "anvil://security"),
            Some("anvil://security".to_string())
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

    #[test]
    fn enforces_hsts_upgrades_and_verifies_subresource_integrity_sha256() {
        let mut hsts = HstsStore::new();
        // Preloaded domain automatically upgrades HTTP -> HTTPS
        assert_eq!(
            hsts.upgrade_url_if_hsts("http://github.com/buntatoes"),
            "https://github.com/buntatoes"
        );
        // Ignore HSTS header sent over insecure HTTP
        assert!(!hsts.record_hsts_header(
            "http://bank.example.com/login",
            "max-age=31536000; includeSubDomains"
        ));
        // Accept HSTS header over HTTPS
        assert!(hsts.record_hsts_header(
            "https://bank.example.com/login",
            "max-age=31536000; includeSubDomains"
        ));
        assert_eq!(
            hsts.upgrade_url_if_hsts("http://api.bank.example.com/v1/status"),
            "https://api.bank.example.com/v1/status"
        );
        let hsts_json = hsts.to_json();
        let restored = HstsStore::from_json(&hsts_json).expect("valid HSTS JSON");
        assert_eq!(restored, hsts);

        let script_bytes = b"let verified = 1;";
        let token = sri_sha256_token(script_bytes);
        assert!(token.starts_with("sha256-"));
        assert!(verify_subresource_integrity(Some(&token), script_bytes));
        assert!(!verify_subresource_integrity(
            Some(&token),
            b"let verified = 0;"
        ));
        assert!(verify_subresource_integrity(None, script_bytes));
    }

    #[test]
    fn connects_to_a_local_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stream = connect_with_timeout("127.0.0.1", port, Duration::from_secs(2));
        assert!(stream.is_ok(), "{stream:?}");
    }

    #[test]
    fn connect_gives_up_within_its_timeout() {
        // 192.0.2.0/24 is reserved for documentation and is never routed, so
        // the attempt either hangs until the timeout or fails at once.
        let started = std::time::Instant::now();
        let result = connect_with_timeout("192.0.2.1", 9, Duration::from_millis(300));
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "connect took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn connect_to_a_closed_port_is_an_error() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        assert!(connect_with_timeout("127.0.0.1", port, Duration::from_secs(2)).is_err());
    }

    #[test]
    fn h2_client_session_multiplexes_streams_and_connection_pool_tracks_reuse() {
        struct MockH2Peer {
            sent: Vec<u8>,
            rx: Vec<u8>,
            pos: usize,
        }
        impl h2::Transport for MockH2Peer {
            fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
                self.sent.extend_from_slice(data);
                Ok(())
            }
            fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
                if self.pos >= self.rx.len() {
                    return Ok(0);
                }
                let n = (self.rx.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.rx[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }

        let mut server_enc = anvil_hpack::Encoder::new();
        let mut server_wire = Vec::new();
        h2::Frame::new(h2::kind::SETTINGS, 0, 0, Vec::new()).write_to(&mut server_wire);
        for (sid, body_bytes) in [
            (1u32, &b"first"[..]),
            (3u32, &b"second"[..]),
            (5u32, &b"third"[..]),
        ] {
            let mut hblock = Vec::new();
            server_enc.encode(
                &[
                    anvil_hpack::Header::new(":status", "200"),
                    anvil_hpack::Header::new("content-length", body_bytes.len().to_string()),
                ],
                &mut hblock,
            );
            h2::Frame::new(h2::kind::HEADERS, h2::flag::END_HEADERS, sid, hblock)
                .write_to(&mut server_wire);
        }
        // Interleave DATA frames in reverse order (stream 5, then 3, then 1) to verify multiplexing
        for (sid, body_bytes) in [
            (5u32, &b"third"[..]),
            (3u32, &b"second"[..]),
            (1u32, &b"first"[..]),
        ] {
            h2::Frame::new(
                h2::kind::DATA,
                h2::flag::END_STREAM,
                sid,
                body_bytes.to_vec(),
            )
            .write_to(&mut server_wire);
        }

        let peer = MockH2Peer {
            sent: Vec::new(),
            rx: server_wire,
            pos: 0,
        };
        let mut session = h2::H2ClientSession::open(peer).expect("open session");
        assert_eq!(session.next_stream_id(), 1);

        let make_req = |path: &str| h2::Request {
            method: "GET".to_string(),
            scheme: "https".to_string(),
            authority: "example.com".to_string(),
            path: path.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        let batch = vec![make_req("/one"), make_req("/two"), make_req("/three")];
        let responses = session
            .exchange_multiplexed(&batch, MAX_RESPONSE)
            .expect("multiplexed exchange");
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].body, b"first");
        assert_eq!(responses[1].body, b"second");
        assert_eq!(responses[2].body, b"third");
        assert_eq!(session.streams_completed(), 3);
        assert_eq!(session.next_stream_id(), 7);
        session.close();

        clear_connection_pool();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let conn = TcpStream::connect(addr).unwrap();
        put_pooled_connection("https://example.com:443", conn);
        assert_eq!(connection_pool_stats().idle_connections, 1);
        assert!(take_pooled_connection("https://example.com:443").is_some());
        assert_eq!(connection_pool_stats().idle_connections, 0);
        record_multiplexed_streams(3);
        assert!(connection_pool_stats().multiplexed_streams >= 3);
    }

    #[test]
    fn live_fetch_batch_reuses_keep_alive_socket_and_recovers_from_closed_peer() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted_srv = std::sync::Arc::clone(&accepted);
        std::thread::spawn(move || {
            // Connection 1 serves two keep-alive requests on the same socket, then closes.
            if let Ok((mut sock, _)) = listener.accept() {
                accepted_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
                for body in [&b"first-keep-alive"[..], &b"second-keep-alive"[..]] {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 512];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => req.extend_from_slice(&buf[..n]),
                        }
                    }
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(reply.as_bytes());
                    let _ = sock.write_all(body);
                }
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
            // Connection 2 serves a third request after the pooled socket was closed by the server.
            if let Ok((mut sock, _)) = listener.accept() {
                accepted_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
                let mut req = Vec::new();
                let mut buf = [0u8; 512];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let body = b"third-reconnected";
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.shutdown(std::net::Shutdown::Both);
            }
        });

        let reused_before = connection_pool_stats().reused_requests;
        let urls = vec![
            format!("http://127.0.0.1:{port}/one.css"),
            format!("http://127.0.0.1:{port}/two.css"),
            format!("http://127.0.0.1:{port}/three.css"),
        ];
        let results = fetch_batch(&urls, None);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_ref().unwrap().1.body, b"first-keep-alive");
        assert_eq!(results[1].as_ref().unwrap().1.body, b"second-keep-alive");
        assert_eq!(results[2].as_ref().unwrap().1.body, b"third-reconnected");
        // Three requests were served over only two TCP connections (request 2 reused connection 1).
        assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(connection_pool_stats().reused_requests > reused_before);
    }

    #[test]
    fn discovers_and_prioritizes_html_subresources_and_h2_stream_priority() {
        let html = r#"<html><head><script defer src="/static/deferred.js"></script><link rel="preload" as="font" href="/fonts/inter.ttf"><link rel="stylesheet" href="/css/critical.css"><script src="/js/blocking.js"></script></head><body><img loading="lazy" src="/img/footer.png"><img src="/img/hero.png"></body></html>"#;
        let discovered = discover_html_subresources(html, "https://example.com/app/index.html");
        assert_eq!(discovered.len(), 6);
        assert_eq!(discovered[0].url, "https://example.com/css/critical.css");
        assert_eq!(discovered[0].priority, SubresourcePriority::CriticalStyle);
        assert_eq!(discovered[1].url, "https://example.com/fonts/inter.ttf");
        assert_eq!(
            discovered[1].priority,
            SubresourcePriority::HighPriorityFont
        );
        assert_eq!(discovered[2].url, "https://example.com/js/blocking.js");
        assert_eq!(discovered[2].priority, SubresourcePriority::BlockingScript);
        assert_eq!(discovered[3].url, "https://example.com/img/hero.png");
        assert_eq!(discovered[3].priority, SubresourcePriority::AboveFoldImage);
        assert_eq!(discovered[4].url, "https://example.com/static/deferred.js");
        assert_eq!(discovered[4].priority, SubresourcePriority::AsyncScript);
        assert_eq!(discovered[5].url, "https://example.com/img/footer.png");
        assert_eq!(
            discovered[5].priority,
            SubresourcePriority::LazyImageOrMedia
        );
        assert_eq!(
            SubresourcePriority::CriticalStyle
                .stream_priority()
                .priority_header_value(),
            "u=0"
        );
        assert_eq!(
            SubresourcePriority::LazyImageOrMedia
                .stream_priority()
                .priority_header_value(),
            "u=5, i"
        );
    }

    #[test]
    fn connection_pool_discards_dirty_or_closed_socket_before_reuse() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let client_live = std::net::TcpStream::connect(addr).unwrap();
        let (mut server_side, _) = listener.accept().unwrap();
        assert!(is_pooled_stream_alive(&client_live));

        // Unread trailing bytes make the idle socket ineligible for clean HTTP reuse
        server_side.write_all(b"X").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(!is_pooled_stream_alive(&client_live));

        let client_closed = std::net::TcpStream::connect(addr).unwrap();
        let (server_closed, _) = listener.accept().unwrap();
        let _ = server_closed.shutdown(std::net::Shutdown::Both);
        drop(server_closed);
        std::thread::sleep(Duration::from_millis(10));
        assert!(!is_pooled_stream_alive(&client_closed));
    }
}

#[cfg(test)]
mod inflight_tests {
    use super::inflight_from_partial;

    #[test]
    fn partial_body_reports_bytes_before_the_response_is_complete() {
        let partial = b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 100\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\n\r\nHELLO";
        let note = inflight_from_partial(partial).unwrap();
        assert_eq!(note.received, 5);
        assert_eq!(note.total, Some(100));
        assert!(note.content_type.starts_with("application/pdf"));
        assert!(note.content_disposition.contains("attachment"));
        assert!(inflight_from_partial(b"HTTP/1.1 200 OK\r\n").is_none());
    }

    #[test]
    fn chunked_body_counts_finished_chunks_and_has_no_total() {
        let partial =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/octet-stream\r\n\r\n5\r\nHELLO\r\n3\r\n";
        let note = inflight_from_partial(partial).unwrap();
        assert_eq!(note.received, 5);
        assert_eq!(note.total, None);
    }
}

#[cfg(test)]
mod cors_tests {
    use super::*;

    #[test]
    fn cors_safelisted_methods_and_headers_classify_preflight_need() {
        assert!(is_cors_safelisted_method("GET"));
        assert!(is_cors_safelisted_method("POST"));
        assert!(is_cors_safelisted_method("HEAD"));
        assert!(!is_cors_safelisted_method("PUT"));
        assert!(!is_cors_safelisted_method("DELETE"));

        assert!(is_cors_safelisted_request_header(
            "Content-Type",
            "text/plain; charset=utf-8"
        ));
        assert!(is_cors_safelisted_request_header(
            "Content-Type",
            "application/x-www-form-urlencoded"
        ));
        assert!(!is_cors_safelisted_request_header(
            "Content-Type",
            "application/json"
        ));
        assert!(!is_cors_safelisted_request_header("X-Custom", "1"));

        assert!(!requires_cors_preflight(
            "POST",
            &[("content-type".to_string(), "text/plain".to_string())]
        ));
        assert!(requires_cors_preflight(
            "POST",
            &[("content-type".to_string(), "application/json".to_string())]
        ));
        assert!(requires_cors_preflight("DELETE", &[]));
    }

    #[test]
    fn cors_response_and_preflight_validation_enforce_origin_and_credentials() {
        let resp_wildcard = parse_http_response(
            b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 2\r\n\r\nok",
        )
        .unwrap();
        assert!(validate_cors_response(&resp_wildcard, "https://app.example", false).is_ok());
        assert!(validate_cors_response(&resp_wildcard, "https://app.example", true).is_err());

        let resp_creds = parse_http_response(
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: https://app.example\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: PUT, DELETE\r\nAccess-Control-Allow-Headers: x-token, content-type\r\n\r\n",
        )
        .unwrap();
        assert!(
            validate_cors_preflight(
                &resp_creds,
                "https://app.example",
                "DELETE",
                &[("X-Token".to_string(), "abc".to_string())],
                true,
            )
            .is_ok()
        );
        assert!(
            validate_cors_preflight(&resp_creds, "https://other.example", "DELETE", &[], true,)
                .is_err()
        );
    }

    #[test]
    fn cors_exposed_headers_filter_non_safelisted_headers() {
        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-secret".to_string(), "hidden".to_string()),
            ("x-rate-limit".to_string(), "42".to_string()),
            (
                "access-control-expose-headers".to_string(),
                "X-Rate-Limit".to_string(),
            ),
            ("set-cookie".to_string(), "sid=1".to_string()),
        ];
        let filtered = filter_cors_exposed_headers(&headers, false);
        assert!(filtered.iter().any(|(k, _)| k == "content-type"));
        assert!(filtered.iter().any(|(k, _)| k == "x-rate-limit"));
        assert!(!filtered.iter().any(|(k, _)| k == "x-secret"));
        assert!(!filtered.iter().any(|(k, _)| k == "set-cookie"));
    }

    #[test]
    fn bodyless_204_framing_returns_immediately_without_content_length() {
        let buf = b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        let (split, cl, chunked) = header_end_and_framing(buf).unwrap();
        assert_eq!(split, buf.len() - 4);
        assert_eq!(cl, Some(0));
        assert!(!chunked);
    }
}
