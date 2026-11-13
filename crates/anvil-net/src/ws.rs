// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Clean-room RFC 6455 WebSocket client (`ws://` and `wss://`).
//!
//! Enforces [`crate::https_only_policy`] (`ws://` upgrade to `wss://` unless
//! exempted), evaluates every connection target against [`crate::shield`] with
//! [`RequestType::Websocket`], negotiates TLS via [`crate::tls::TlsSession`] on
//! `wss://`, validates the HTTP/1.1 `101 Switching Protocols` handshake and
//! `Sec-WebSocket-Accept` digest, masks all client-to-server frames, handles
//! fragmented messages and interleaved control frames (`Ping` -> auto `Pong`,
//! `Close` handshake), and enforces [`MAX_WS_MESSAGE_BYTES`].

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{CONNECT_TIMEOUT, MAX_HEADER_BYTES, NetError, RequestType};

/// Maximum assembled WebSocket message size accepted by the client (8 MiB).
pub const MAX_WS_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum buffered incoming wire bytes held by a single WebSocket client.
const MAX_WS_READ_BUFFER: usize = 16 * 1024 * 1024;

/// RFC 6455 Section 1.3 handshake accept GUID.
const WS_ACCEPT_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Parsed `ws://` or `wss://` endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsUrl {
    /// Lowercased scheme (`ws` or `wss`).
    pub scheme: String,
    /// Lowercased target hostname.
    pub host: String,
    /// Target TCP port (`80` default for `ws`, `443` default for `wss`).
    pub port: u16,
    /// Path and optional query string starting with `/`.
    pub path_and_query: String,
}

impl WsUrl {
    /// Canonical `ws://` or `wss://` URL string.
    pub fn canonical_url(&self) -> String {
        let default_port = if self.scheme == "wss" { 443 } else { 80 };
        if self.port == default_port {
            format!("{}://{}{}", self.scheme, self.host, self.path_and_query)
        } else {
            format!(
                "{}://{}:{}{}",
                self.scheme, self.host, self.port, self.path_and_query
            )
        }
    }

    /// HTTP `Host` header value for the handshake request.
    fn host_header(&self) -> String {
        let default_port = if self.scheme == "wss" { 443 } else { 80 };
        if self.port == default_port {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Parse an absolute `ws://` or `wss://` URL.
pub fn parse_ws_url(input: &str) -> Result<WsUrl, NetError> {
    let input = input.trim();
    if input.bytes().any(|b| b < 0x20 || b == 0x7F || b == b' ') {
        return Err(NetError::InvalidUrl);
    }
    let (scheme, rest) = input.split_once("://").ok_or(NetError::InvalidUrl)?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "ws" => 80_u16,
        "wss" => 443_u16,
        _ => return Err(NetError::UnsupportedScheme),
    };
    if rest.is_empty() {
        return Err(NetError::InvalidUrl);
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.starts_with('[') || authority.contains('@') {
        return Err(NetError::InvalidUrl);
    }
    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        if h.is_empty() {
            return Err(NetError::InvalidUrl);
        }
        let port: u16 = p.parse().map_err(|_| NetError::InvalidUrl)?;
        if port == 0 {
            return Err(NetError::InvalidUrl);
        }
        (h.to_ascii_lowercase(), port)
    } else {
        (authority.to_ascii_lowercase(), default_port)
    };
    if !crate::is_valid_hostname(&host) {
        return Err(NetError::InvalidUrl);
    }
    let path_and_query = if path_and_query.is_empty() {
        "/".to_string()
    } else if path_and_query.starts_with("?") {
        format!("/{path_and_query}")
    } else {
        path_and_query.to_string()
    };
    Ok(WsUrl {
        scheme,
        host,
        port,
        path_and_query,
    })
}

/// Connection options for [`WebSocketClient::connect_with_options`].
#[derive(Clone, Debug, Default)]
pub struct WsConnectOptions {
    /// Optional `Origin` header sent during the upgrade handshake.
    pub origin: Option<String>,
    /// Optional ordered subprotocol names offered in `Sec-WebSocket-Protocol`.
    pub protocols: Vec<String>,
    /// TCP connect timeout; defaults to [`CONNECT_TIMEOUT`].
    pub connect_timeout: Option<Duration>,
}

/// WebSocket lifecycle state matching WHATWG `WebSocket.readyState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WsReadyState {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
}

/// Payload carried by a WebSocket message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

/// Event yielded by [`WebSocketClient::recv_event`] or [`WebSocketClient::poll_events`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsEvent {
    Message(WsMessage),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Closed { code: u16, reason: String },
}

/// Error returned by WebSocket URL validation, handshake, or frame I/O.
#[derive(Debug)]
pub enum WsError {
    Network(NetError),
    Handshake(String),
    Protocol(String),
    InvalidState(&'static str),
    MessageTooLarge,
}

impl From<NetError> for WsError {
    fn from(err: NetError) -> Self {
        Self::Network(err)
    }
}

impl From<std::io::Error> for WsError {
    fn from(err: std::io::Error) -> Self {
        Self::Network(NetError::Io(err))
    }
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(err) => write!(f, "{err}"),
            Self::Handshake(msg) => write!(f, "WebSocket handshake failed: {msg}"),
            Self::Protocol(msg) => write!(f, "WebSocket protocol error: {msg}"),
            Self::InvalidState(msg) => write!(f, "WebSocket invalid state: {msg}"),
            Self::MessageTooLarge => write!(f, "WebSocket message exceeds size limit"),
        }
    }
}

impl std::error::Error for WsError {}

enum WsTransport {
    Plain(TcpStream),
    #[cfg(any(windows, target_os = "linux"))]
    Tls(crate::tls::TlsSession),
}

impl WsTransport {
    fn write_all(&mut self, data: &[u8]) -> Result<(), NetError> {
        match self {
            Self::Plain(stream) => stream.write_all(data).map_err(NetError::Io),
            #[cfg(any(windows, target_os = "linux"))]
            Self::Tls(tls) => tls.write_all(data),
        }
    }

    fn read_blocking(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            #[cfg(any(windows, target_os = "linux"))]
            Self::Tls(tls) => tls.try_read(buf),
        }
    }

    fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_nonblocking(nonblocking),
            #[cfg(any(windows, target_os = "linux"))]
            Self::Tls(tls) => tls.set_nonblocking(nonblocking),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.set_read_timeout(timeout),
            #[cfg(any(windows, target_os = "linux"))]
            Self::Tls(tls) => tls.set_read_timeout(timeout),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FragKind {
    None,
    Text,
    Binary,
}

/// Active RFC 6455 WebSocket client session.
pub struct WebSocketClient {
    url: WsUrl,
    transport: WsTransport,
    state: WsReadyState,
    negotiated_protocol: Option<String>,
    read_buf: Vec<u8>,
    frag_kind: FragKind,
    frag_buf: Vec<u8>,
    close_sent: bool,
    nonblocking: bool,
}

impl WebSocketClient {
    /// Connect to a `ws://` or `wss://` URL with default options.
    pub fn connect(url: &str) -> Result<Self, WsError> {
        Self::connect_with_options(url, &WsConnectOptions::default())
    }

    /// Connect to a `ws://` or `wss://` URL with explicit origin/subprotocols.
    pub fn connect_with_options(url: &str, options: &WsConnectOptions) -> Result<Self, WsError> {
        let mut parsed = parse_ws_url(url)?;
        let mut upgraded_by_https_only = false;
        if parsed.scheme == "ws" && !crate::https_only_policy().allows_http(&parsed.host) {
            parsed.scheme = "wss".to_string();
            if parsed.port == 80 {
                parsed.port = 443;
            }
            upgraded_by_https_only = true;
        }
        let target_url = parsed.canonical_url();
        let initiator = options.origin.as_deref().unwrap_or(&target_url);
        let shield = crate::check_default_shield_request(
            &target_url,
            Some(initiator),
            RequestType::Websocket,
        );
        if shield.matched {
            let reason = shield
                .reason
                .map(|r| format!("{r:?}"))
                .unwrap_or_else(|| "blocked WebSocket endpoint".to_string());
            return Err(WsError::Network(NetError::BlockedByShield(reason)));
        }

        let timeout = options.connect_timeout.unwrap_or(CONNECT_TIMEOUT);
        let tcp = match crate::connect_with_timeout(&parsed.host, parsed.port, timeout) {
            Ok(sock) => sock,
            Err(err) => {
                if upgraded_by_https_only {
                    return Err(WsError::Network(NetError::HttpsOnly(target_url)));
                }
                return Err(WsError::Network(NetError::Io(err)));
            }
        };
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(15)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(15)));

        let mut transport = if parsed.scheme == "wss" {
            #[cfg(any(windows, target_os = "linux"))]
            {
                match crate::tls::TlsSession::connect(tcp, &parsed.host, &[b"http/1.1"]) {
                    Ok(tls) => WsTransport::Tls(tls),
                    Err(err) => {
                        if upgraded_by_https_only {
                            return Err(WsError::Network(NetError::HttpsOnly(target_url)));
                        }
                        return Err(WsError::Network(err));
                    }
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = tcp;
                return Err(WsError::Network(NetError::UnsupportedScheme));
            }
        } else {
            WsTransport::Plain(tcp)
        };

        let ws_key = base64_encode(&random_16_bytes());
        let mut req = String::new();
        req.push_str(&format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n",
            parsed.path_and_query,
            parsed.host_header(),
            ws_key
        ));
        if let Some(origin) = &options.origin {
            let clean = sanitize_header_value(origin);
            if !clean.is_empty() {
                req.push_str(&format!("Origin: {clean}\r\n"));
            }
        }
        let offered_protocols: Vec<String> = options
            .protocols
            .iter()
            .map(|p| sanitize_header_value(p))
            .filter(|p| is_valid_subprotocol_token(p))
            .collect();
        if !offered_protocols.is_empty() {
            req.push_str(&format!(
                "Sec-WebSocket-Protocol: {}\r\n",
                offered_protocols.join(", ")
            ));
        }
        req.push_str("\r\n");

        transport.write_all(req.as_bytes())?;
        let (head_bytes, leftover) = read_http_upgrade_head(&mut transport)?;
        let negotiated_protocol =
            validate_upgrade_response(&head_bytes, &ws_key, &offered_protocols)?;

        Ok(Self {
            url: parsed,
            transport,
            state: WsReadyState::Open,
            negotiated_protocol,
            read_buf: leftover,
            frag_kind: FragKind::None,
            frag_buf: Vec::new(),
            close_sent: false,
            nonblocking: false,
        })
    }

    /// Canonical `ws://` or `wss://` URL of this connection.
    pub fn url(&self) -> String {
        self.url.canonical_url()
    }

    /// Current RFC 6455 ready state.
    pub fn ready_state(&self) -> WsReadyState {
        self.state
    }

    /// Subprotocol agreed upon by the server, if any.
    pub fn protocol(&self) -> Option<&str> {
        self.negotiated_protocol.as_deref()
    }

    /// Send a masked UTF-8 text frame.
    pub fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        if self.state != WsReadyState::Open {
            return Err(WsError::InvalidState("connection is not open"));
        }
        self.write_client_frame(OPCODE_TEXT, text.as_bytes())
    }

    /// Send a masked binary frame.
    pub fn send_binary(&mut self, data: &[u8]) -> Result<(), WsError> {
        if self.state != WsReadyState::Open {
            return Err(WsError::InvalidState("connection is not open"));
        }
        self.write_client_frame(OPCODE_BINARY, data)
    }

    /// Send a masked RFC 6455 Ping frame (payload at most 125 bytes).
    pub fn send_ping(&mut self, payload: &[u8]) -> Result<(), WsError> {
        if self.state != WsReadyState::Open {
            return Err(WsError::InvalidState("connection is not open"));
        }
        if payload.len() > 125 {
            return Err(WsError::Protocol(
                "control frame payload exceeds 125 bytes".into(),
            ));
        }
        self.write_client_frame(OPCODE_PING, payload)
    }

    /// Initiate or complete an RFC 6455 close handshake.
    pub fn close(&mut self, code: u16, reason: &str) -> Result<(), WsError> {
        if matches!(self.state, WsReadyState::Closing | WsReadyState::Closed) {
            return Ok(());
        }
        let mut payload = Vec::with_capacity(2 + reason.len().min(123));
        payload.extend_from_slice(&code.to_be_bytes());
        let mut truncated = reason.as_bytes();
        if truncated.len() > 123 {
            let mut end = 123;
            while end > 0 && std::str::from_utf8(&truncated[..end]).is_err() {
                end -= 1;
            }
            truncated = &truncated[..end];
        }
        payload.extend_from_slice(truncated);
        self.state = WsReadyState::Closing;
        self.close_sent = true;
        let res = self.write_client_frame(OPCODE_CLOSE, &payload);
        if res.is_err() {
            self.state = WsReadyState::Closed;
        }
        res
    }

    /// Block until the next complete WebSocket message or lifecycle event arrives.
    pub fn recv_event(&mut self) -> Result<WsEvent, WsError> {
        self.set_nonblocking_mode(false)?;
        loop {
            if let Some(ev) = self.try_decode_next_event()? {
                return Ok(ev);
            }
            if self.state == WsReadyState::Closed {
                return Ok(WsEvent::Closed {
                    code: 1000,
                    reason: String::new(),
                });
            }
            match self.fill_read_buf() {
                Ok(0) => {
                    self.state = WsReadyState::Closed;
                    return Ok(WsEvent::Closed {
                        code: 1006,
                        reason: "connection closed".into(),
                    });
                }
                Ok(_) => continue,
                Err(err) => return Err(err),
            }
        }
    }

    /// Non-blocking drain of any pending WebSocket frames on the socket.
    pub fn poll_events(&mut self) -> Vec<WsEvent> {
        let mut events = Vec::new();
        if self.state == WsReadyState::Closed {
            return events;
        }
        if self.set_nonblocking_mode(true).is_err() {
            self.state = WsReadyState::Closed;
            events.push(WsEvent::Closed {
                code: 1006,
                reason: "socket error".into(),
            });
            return events;
        }
        loop {
            match self.try_decode_next_event() {
                Ok(Some(ev)) => {
                    let is_closed = matches!(ev, WsEvent::Closed { .. });
                    events.push(ev);
                    if is_closed {
                        return events;
                    }
                    continue;
                }
                Ok(None) => {}
                Err(err) => {
                    let _ = self.close_with_error(1002, &err.to_string());
                    self.state = WsReadyState::Closed;
                    events.push(WsEvent::Closed {
                        code: 1002,
                        reason: err.to_string(),
                    });
                    return events;
                }
            }
            let mut chunk = [0_u8; 8192];
            match self.transport.read_blocking(&mut chunk) {
                Ok(0) => {
                    self.state = WsReadyState::Closed;
                    events.push(WsEvent::Closed {
                        code: 1006,
                        reason: String::new(),
                    });
                    return events;
                }
                Ok(n) => {
                    if self.read_buf.len() + n > MAX_WS_READ_BUFFER {
                        self.state = WsReadyState::Closed;
                        events.push(WsEvent::Closed {
                            code: 1009,
                            reason: "frame buffer limit exceeded".into(),
                        });
                        return events;
                    }
                    self.read_buf.extend_from_slice(&chunk[..n]);
                }
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    break;
                }
                Err(_) => {
                    self.state = WsReadyState::Closed;
                    events.push(WsEvent::Closed {
                        code: 1006,
                        reason: String::new(),
                    });
                    return events;
                }
            }
        }
        events
    }

    fn set_nonblocking_mode(&mut self, nonblocking: bool) -> std::io::Result<()> {
        if self.nonblocking != nonblocking {
            self.transport.set_nonblocking(nonblocking)?;
            if !nonblocking {
                let _ = self
                    .transport
                    .set_read_timeout(Some(Duration::from_secs(15)));
            }
            self.nonblocking = nonblocking;
        }
        Ok(())
    }

    fn fill_read_buf(&mut self) -> Result<usize, WsError> {
        let mut chunk = [0_u8; 8192];
        let n = self.transport.read_blocking(&mut chunk)?;
        if self.read_buf.len() + n > MAX_WS_READ_BUFFER {
            return Err(WsError::MessageTooLarge);
        }
        self.read_buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    fn try_decode_next_event(&mut self) -> Result<Option<WsEvent>, WsError> {
        loop {
            let Some(frame) = decode_server_frame(&mut self.read_buf)? else {
                return Ok(None);
            };
            if frame.rsv != 0 {
                return Err(WsError::Protocol("non-zero RSV bits unsupported".into()));
            }
            if frame.masked {
                return Err(WsError::Protocol(
                    "server frame must not have MASK bit set".into(),
                ));
            }
            match frame.opcode {
                OPCODE_PING => {
                    if !frame.fin || frame.payload.len() > 125 {
                        return Err(WsError::Protocol("invalid Ping control frame".into()));
                    }
                    if self.state == WsReadyState::Open {
                        let _ = self.write_client_frame(OPCODE_PONG, &frame.payload);
                    }
                    return Ok(Some(WsEvent::Ping(frame.payload)));
                }
                OPCODE_PONG => {
                    if !frame.fin || frame.payload.len() > 125 {
                        return Err(WsError::Protocol("invalid Pong control frame".into()));
                    }
                    return Ok(Some(WsEvent::Pong(frame.payload)));
                }
                OPCODE_CLOSE => {
                    if !frame.fin || frame.payload.len() > 125 {
                        return Err(WsError::Protocol("invalid Close control frame".into()));
                    }
                    let (code, reason) = parse_close_payload(&frame.payload)?;
                    if !self.close_sent {
                        self.close_sent = true;
                        let echo_code = if code == 1005 { 1000 } else { code };
                        let _ = self.write_client_frame(OPCODE_CLOSE, &echo_code.to_be_bytes());
                    }
                    self.state = WsReadyState::Closed;
                    return Ok(Some(WsEvent::Closed { code, reason }));
                }
                OPCODE_TEXT | OPCODE_BINARY => {
                    if self.frag_kind != FragKind::None {
                        return Err(WsError::Protocol(
                            "unexpected new data frame during fragmented message".into(),
                        ));
                    }
                    if frame.payload.len() > MAX_WS_MESSAGE_BYTES {
                        return Err(WsError::MessageTooLarge);
                    }
                    if frame.fin {
                        if frame.opcode == OPCODE_TEXT {
                            let text = String::from_utf8(frame.payload).map_err(|_| {
                                WsError::Protocol("invalid UTF-8 in text frame".into())
                            })?;
                            return Ok(Some(WsEvent::Message(WsMessage::Text(text))));
                        }
                        return Ok(Some(WsEvent::Message(WsMessage::Binary(frame.payload))));
                    }
                    self.frag_kind = if frame.opcode == OPCODE_TEXT {
                        FragKind::Text
                    } else {
                        FragKind::Binary
                    };
                    self.frag_buf = frame.payload;
                }
                OPCODE_CONTINUATION => {
                    if self.frag_kind == FragKind::None {
                        return Err(WsError::Protocol(
                            "unexpected continuation frame without initial data frame".into(),
                        ));
                    }
                    if self.frag_buf.len() + frame.payload.len() > MAX_WS_MESSAGE_BYTES {
                        self.frag_buf.clear();
                        self.frag_kind = FragKind::None;
                        return Err(WsError::MessageTooLarge);
                    }
                    self.frag_buf.extend_from_slice(&frame.payload);
                    if frame.fin {
                        let kind = self.frag_kind;
                        self.frag_kind = FragKind::None;
                        let bytes = std::mem::take(&mut self.frag_buf);
                        if kind == FragKind::Text {
                            let text = String::from_utf8(bytes).map_err(|_| {
                                WsError::Protocol("invalid UTF-8 in fragmented text message".into())
                            })?;
                            return Ok(Some(WsEvent::Message(WsMessage::Text(text))));
                        }
                        return Ok(Some(WsEvent::Message(WsMessage::Binary(bytes))));
                    }
                }
                other => {
                    return Err(WsError::Protocol(format!(
                        "unknown WebSocket opcode 0x{other:x}"
                    )));
                }
            }
        }
    }

    fn close_with_error(&mut self, code: u16, reason: &str) -> Result<(), WsError> {
        if !self.close_sent {
            self.close(code, reason)
        } else {
            self.state = WsReadyState::Closed;
            Ok(())
        }
    }

    fn write_client_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        if payload.len() > MAX_WS_MESSAGE_BYTES {
            return Err(WsError::MessageTooLarge);
        }
        let mut frame = Vec::with_capacity(14 + payload.len());
        frame.push(0x80 | (opcode & 0x0F));
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len <= u16::MAX as usize {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = random_mask_key();
        frame.extend_from_slice(&mask);
        let start = frame.len();
        frame.extend_from_slice(payload);
        for (i, byte) in frame[start..].iter_mut().enumerate() {
            *byte ^= mask[i & 3];
        }
        self.transport.write_all(&frame).map_err(WsError::Network)
    }
}

struct DecodedFrame {
    fin: bool,
    rsv: u8,
    opcode: u8,
    masked: bool,
    payload: Vec<u8>,
}

fn decode_server_frame(buf: &mut Vec<u8>) -> Result<Option<DecodedFrame>, WsError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let fin = (b0 & 0x80) != 0;
    let rsv = (b0 >> 4) & 0x07;
    let opcode = b0 & 0x0F;
    let masked = (b1 & 0x80) != 0;
    let len7 = b1 & 0x7F;
    let mut header_len = 2_usize;
    let payload_len = match len7 {
        0..=125 => len7 as usize,
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            header_len = 4;
            u16::from_be_bytes([buf[2], buf[3]]) as usize
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            header_len = 10;
            let raw = u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            ]);
            if (raw >> 63) != 0 {
                return Err(WsError::Protocol("64-bit length has MSB set".into()));
            }
            usize::try_from(raw).map_err(|_| WsError::MessageTooLarge)?
        }
        _ => unreachable!(),
    };
    if payload_len > MAX_WS_MESSAGE_BYTES {
        return Err(WsError::MessageTooLarge);
    }
    let mask_offset = header_len;
    if masked {
        header_len += 4;
    }
    let total = header_len
        .checked_add(payload_len)
        .ok_or(WsError::MessageTooLarge)?;
    if buf.len() < total {
        return Ok(None);
    }
    let mut payload = buf[header_len..total].to_vec();
    if masked {
        let mask = [
            buf[mask_offset],
            buf[mask_offset + 1],
            buf[mask_offset + 2],
            buf[mask_offset + 3],
        ];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[i & 3];
        }
    }
    buf.drain(..total);
    Ok(Some(DecodedFrame {
        fin,
        rsv,
        opcode,
        masked,
        payload,
    }))
}

fn parse_close_payload(payload: &[u8]) -> Result<(u16, String), WsError> {
    if payload.is_empty() {
        return Ok((1005, String::new()));
    }
    if payload.len() == 1 {
        return Err(WsError::Protocol("1-byte close frame payload".into()));
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    let reason = std::str::from_utf8(&payload[2..])
        .map_err(|_| WsError::Protocol("invalid UTF-8 in close reason".into()))?
        .to_string();
    Ok((code, reason))
}

fn read_http_upgrade_head(transport: &mut WsTransport) -> Result<(Vec<u8>, Vec<u8>), WsError> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 2048];
    loop {
        if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = buf[..split].to_vec();
            let leftover = buf[split + 4..].to_vec();
            return Ok((head, leftover));
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Err(WsError::Handshake("upgrade header exceeded limit".into()));
        }
        let n = transport.read_blocking(&mut chunk)?;
        if n == 0 {
            return Err(WsError::Handshake("peer closed during upgrade".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn validate_upgrade_response(
    head_bytes: &[u8],
    sent_key: &str,
    offered_protocols: &[String],
) -> Result<Option<String>, WsError> {
    let head = std::str::from_utf8(head_bytes)
        .map_err(|_| WsError::Handshake("non-UTF-8 response header".into()))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| WsError::Handshake("missing status line".into()))?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().unwrap_or("");
    let code = status_parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") || code != "101" {
        return Err(WsError::Handshake(format!(
            "expected HTTP 101 Switching Protocols, got '{status_line}'"
        )));
    }

    let mut has_upgrade = false;
    let mut has_connection = false;
    let mut accept_header: Option<String> = None;
    let mut proto_header: Option<String> = None;

    for line in lines {
        let Some((name, val)) = line.split_once(':') else {
            continue;
        };
        let key = name.trim();
        let val = val.trim();
        if key.eq_ignore_ascii_case("upgrade") && val.eq_ignore_ascii_case("websocket") {
            has_upgrade = true;
        } else if key.eq_ignore_ascii_case("connection")
            && val
                .split(',')
                .any(|tok| tok.trim().eq_ignore_ascii_case("upgrade"))
        {
            has_connection = true;
        } else if key.eq_ignore_ascii_case("sec-websocket-accept") {
            accept_header = Some(val.to_string());
        } else if key.eq_ignore_ascii_case("sec-websocket-protocol") {
            proto_header = Some(val.to_string());
        }
    }

    if !has_upgrade || !has_connection {
        return Err(WsError::Handshake(
            "missing Upgrade: websocket or Connection: Upgrade header".into(),
        ));
    }
    let expected = compute_accept_key(sent_key);
    if accept_header.as_deref() != Some(expected.as_str()) {
        return Err(WsError::Handshake("Sec-WebSocket-Accept mismatch".into()));
    }
    if let Some(proto) = proto_header {
        if offered_protocols.is_empty() || !offered_protocols.iter().any(|p| p == &proto) {
            return Err(WsError::Handshake(format!(
                "server selected unrequested subprotocol '{proto}'"
            )));
        }
        return Ok(Some(proto));
    }
    Ok(None)
}

fn sanitize_header_value(raw: &str) -> String {
    raw.bytes()
        .filter(|&b| b >= 0x20 && b != 0x7F)
        .map(char::from)
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_valid_subprotocol_token(token: &str) -> bool {
    !token.is_empty()
        && token.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn next_prng_u64() -> u64 {
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((&raw const NONCE_COUNTER) as usize as u64);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn random_16_bytes() -> [u8; 16] {
    let a = next_prng_u64().to_le_bytes();
    let b = next_prng_u64().to_le_bytes();
    let mut out = [0_u8; 16];
    out[..8].copy_from_slice(&a);
    out[8..].copy_from_slice(&b);
    out
}

fn random_mask_key() -> [u8; 4] {
    let v = (next_prng_u64() as u32) | 1;
    v.to_le_bytes()
}

/// Compute the RFC 6455 `Sec-WebSocket-Accept` response value for `key`.
pub fn compute_accept_key(key: &str) -> String {
    let mut input = Vec::with_capacity(key.len() + WS_ACCEPT_GUID.len());
    input.extend_from_slice(key.as_bytes());
    input.extend_from_slice(WS_ACCEPT_GUID);
    let digest = sha1_digest(&input);
    base64_encode(&digest)
}

/// Clean-room RFC 3174 SHA-1 digest used exclusively for the RFC 6455
/// `Sec-WebSocket-Key` -> `Sec-WebSocket-Accept` handshake header.
pub(crate) fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0_u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999_u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1_u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC_u32),
                _ => (b ^ c ^ d, 0xCA62_C1D6_u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0_u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = *chunk.get(1).unwrap_or(&0) as usize;
        let b2 = *chunk.get(2).unwrap_or(&0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) & 63] as char);
        out.push(TABLE[(n >> 12) & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn read_client_masked_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut head = [0_u8; 2];
        stream.read_exact(&mut head).unwrap();
        let opcode = head[0] & 0x0F;
        let masked = (head[1] & 0x80) != 0;
        assert!(masked, "client frames must be masked per RFC 6455");
        let len7 = (head[1] & 0x7F) as usize;
        let len = match len7 {
            0..=125 => len7,
            126 => {
                let mut ext = [0_u8; 2];
                stream.read_exact(&mut ext).unwrap();
                u16::from_be_bytes(ext) as usize
            }
            127 => {
                let mut ext = [0_u8; 8];
                stream.read_exact(&mut ext).unwrap();
                u64::from_be_bytes(ext) as usize
            }
            _ => unreachable!(),
        };
        let mut mask = [0_u8; 4];
        stream.read_exact(&mut mask).unwrap();
        let mut payload = vec![0_u8; len];
        stream.read_exact(&mut payload).unwrap();
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i & 3];
        }
        (opcode, payload)
    }

    fn write_server_frame(stream: &mut TcpStream, fin: bool, opcode: u8, payload: &[u8]) {
        let mut frame = Vec::new();
        frame.push((if fin { 0x80 } else { 0x00 }) | (opcode & 0x0F));
        if payload.len() < 126 {
            frame.push(payload.len() as u8);
        } else {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        stream.write_all(&frame).unwrap();
    }

    #[test]
    fn rfc6455_accept_key_matches_spec_vector() {
        assert_eq!(
            compute_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn parses_and_rejects_ws_urls() {
        let ws = parse_ws_url("ws://localhost/chat?room=1#frag").unwrap();
        assert_eq!(ws.scheme, "ws");
        assert_eq!(ws.host, "localhost");
        assert_eq!(ws.port, 80);
        assert_eq!(ws.path_and_query, "/chat?room=1");
        assert_eq!(ws.canonical_url(), "ws://localhost/chat?room=1");

        let wss = parse_ws_url("WSS://Example.COM:8443").unwrap();
        assert_eq!(wss.scheme, "wss");
        assert_eq!(wss.host, "example.com");
        assert_eq!(wss.port, 8443);
        assert_eq!(wss.path_and_query, "/");

        assert!(matches!(
            parse_ws_url("http://example.com"),
            Err(NetError::UnsupportedScheme)
        ));
        assert!(matches!(
            parse_ws_url("ws://user@example.com/"),
            Err(NetError::InvalidUrl)
        ));
    }

    #[test]
    fn loopback_websocket_handshake_fragment_ping_and_close() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req_buf = Vec::new();
            let mut tmp = [0_u8; 1024];
            loop {
                let n = sock.read(&mut tmp).unwrap();
                req_buf.extend_from_slice(&tmp[..n]);
                if req_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let req_text = String::from_utf8(req_buf).unwrap();
            let key_line = req_text
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("sec-websocket-key:"))
                .unwrap();
            let key = key_line.split_once(':').unwrap().1.trim();
            let accept = compute_accept_key(key);
            let resp = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: anvil-chat\r\n\r\n"
            );
            sock.write_all(resp.as_bytes()).unwrap();

            // Read client text frame and reply with fragmented text + interleaved ping.
            let (op, data) = read_client_masked_frame(&mut sock);
            assert_eq!(op, OPCODE_TEXT);
            assert_eq!(data, b"hello ws");

            write_server_frame(&mut sock, false, OPCODE_TEXT, b"hi ");
            write_server_frame(&mut sock, true, OPCODE_PING, b"keep");
            write_server_frame(&mut sock, true, OPCODE_CONTINUATION, b"client");

            let (pong_op, pong_payload) = read_client_masked_frame(&mut sock);
            assert_eq!(pong_op, OPCODE_PONG);
            assert_eq!(pong_payload, b"keep");

            let (close_op, close_payload) = read_client_masked_frame(&mut sock);
            assert_eq!(close_op, OPCODE_CLOSE);
            assert_eq!(&close_payload[..2], &1000_u16.to_be_bytes());
            write_server_frame(&mut sock, true, OPCODE_CLOSE, &close_payload);
        });

        let policy = crate::HttpsOnlyPolicy::new(false, &[]);
        let _scope = crate::enter_https_only(policy);

        let opts = WsConnectOptions {
            origin: Some("http://127.0.0.1".into()),
            protocols: vec!["anvil-chat".into()],
            connect_timeout: None,
        };
        let mut client =
            WebSocketClient::connect_with_options(&format!("ws://127.0.0.1:{port}/live"), &opts)
                .unwrap();
        assert_eq!(client.ready_state(), WsReadyState::Open);
        assert_eq!(client.protocol(), Some("anvil-chat"));

        client.send_text("hello ws").unwrap();
        assert_eq!(
            client.recv_event().unwrap(),
            WsEvent::Ping(b"keep".to_vec())
        );
        assert_eq!(
            client.recv_event().unwrap(),
            WsEvent::Message(WsMessage::Text("hi client".into()))
        );
        client.close(1000, "done").unwrap();
        assert_eq!(
            client.recv_event().unwrap(),
            WsEvent::Closed {
                code: 1000,
                reason: "done".into(),
            }
        );
        assert_eq!(client.ready_state(), WsReadyState::Closed);
        server.join().unwrap();
    }
}
