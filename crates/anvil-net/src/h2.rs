// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! HTTP/2 client for one request per connection, written from RFC 9113.
//!
//! The browser opens a TLS connection, offers `h2` and `http/1.1` through
//! ALPN, and comes here only when the server picks `h2`. One request goes out
//! on stream 1 and the connection closes after the response. Header blocks
//! use `anvil-hpack`.
//!
//! What the client sends first: the connection preface, a SETTINGS frame
//! that turns server push off and opens a larger receive window, and a
//! WINDOW_UPDATE for the connection. Then the request's HEADERS (split into
//! CONTINUATION frames when the block is large) and any body as DATA, sent
//! only as fast as the server's flow control windows allow.
//!
//! While reading, the client answers SETTINGS with an ACK and PING with a
//! PING ACK, gives received DATA back to the server's window, and skips
//! frames for other streams and frame types it does not know. Anything the
//! RFC calls a connection error ends the request with
//! [`NetError::InvalidResponse`].
//!
//! [`H2Error::Fallback`] means the server will not take this request over
//! HTTP/2: it sent `HTTP_1_1_REQUIRED`, refused the stream, or said goodbye
//! before it started on stream 1. The caller then repeats the request on a
//! new HTTP/1.1 connection. Nothing reached the server's application in
//! those cases, so the repeat is safe for a POST too.
//!
//! Limits: frames over [`MAX_FRAME_SIZE`] are refused, a header block over
//! [`MAX_HEADER_BLOCK`] is refused before it is decoded, decoded header lists
//! are capped at `MAX_HEADER_BYTES` and `MAX_HEADER_COUNT`, and the body is
//! capped at the `max_body` the caller passes.

use crate::{MAX_HEADER_BYTES, MAX_HEADER_COUNT, NetError, Response};
use anvil_hpack::{Decoder, Encoder, Header};

/// The 24 bytes every HTTP/2 client sends first.
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Frame payloads larger than this are refused. It is the RFC's default and
/// the client never asks for more.
pub const MAX_FRAME_SIZE: usize = 16_384;

/// A header block (HEADERS plus CONTINUATION) larger than this is refused.
pub const MAX_HEADER_BLOCK: usize = 256 * 1024;

/// Receive window the client offers for stream 1 and for the connection.
pub const RECEIVE_WINDOW: u32 = 1 << 20;

/// Largest flow control window the RFC allows.
const MAX_WINDOW: i64 = (1 << 31) - 1;

/// Frame type numbers.
pub mod kind {
    /// Body bytes.
    pub const DATA: u8 = 0x0;
    /// Start of a header block.
    pub const HEADERS: u8 = 0x1;
    /// Stream priority. Read and ignored.
    pub const PRIORITY: u8 = 0x2;
    /// Ends one stream.
    pub const RST_STREAM: u8 = 0x3;
    /// Connection settings.
    pub const SETTINGS: u8 = 0x4;
    /// Server push. Refused, because push is turned off.
    pub const PUSH_PROMISE: u8 = 0x5;
    /// Liveness check.
    pub const PING: u8 = 0x6;
    /// The connection is closing.
    pub const GOAWAY: u8 = 0x7;
    /// Flow control credit.
    pub const WINDOW_UPDATE: u8 = 0x8;
    /// More of a header block.
    pub const CONTINUATION: u8 = 0x9;
}

/// Frame flag bits.
pub mod flag {
    /// Last frame of the stream (DATA, HEADERS).
    pub const END_STREAM: u8 = 0x1;
    /// Acknowledgement (SETTINGS, PING).
    pub const ACK: u8 = 0x1;
    /// Last frame of a header block (HEADERS, CONTINUATION, PUSH_PROMISE).
    pub const END_HEADERS: u8 = 0x4;
    /// The payload starts with a pad length.
    pub const PADDED: u8 = 0x8;
    /// HEADERS carries priority fields.
    pub const PRIORITY: u8 = 0x20;
}

/// Error codes used in RST_STREAM and GOAWAY.
pub mod code {
    /// Not an error.
    pub const NO_ERROR: u32 = 0x0;
    /// The peer broke the protocol.
    pub const PROTOCOL_ERROR: u32 = 0x1;
    /// A window grew past its limit.
    pub const FLOW_CONTROL_ERROR: u32 = 0x3;
    /// A frame had the wrong size.
    pub const FRAME_SIZE_ERROR: u32 = 0x6;
    /// The stream was refused before any processing.
    pub const REFUSED_STREAM: u32 = 0x7;
    /// A header block could not be decoded.
    pub const COMPRESSION_ERROR: u32 = 0x9;
    /// The server wants HTTP/1.1 for this request.
    pub const HTTP_1_1_REQUIRED: u32 = 0xd;
}

/// Settings identifiers.
pub mod setting {
    /// Size of the peer's HPACK table.
    pub const HEADER_TABLE_SIZE: u16 = 0x1;
    /// Whether server push may be used.
    pub const ENABLE_PUSH: u16 = 0x2;
    /// Streams the peer allows at once.
    pub const MAX_CONCURRENT_STREAMS: u16 = 0x3;
    /// Starting window for new streams.
    pub const INITIAL_WINDOW_SIZE: u16 = 0x4;
    /// Largest frame payload the sender will accept.
    pub const MAX_FRAME_SIZE: u16 = 0x5;
    /// Largest header list the sender will accept.
    pub const MAX_HEADER_LIST_SIZE: u16 = 0x6;
}

/// Why an HTTP/2 exchange stopped.
#[derive(Debug)]
pub enum H2Error {
    /// Repeat the request over HTTP/1.1. See the module notes.
    Fallback,
    /// Any other failure.
    Net(NetError),
}

impl From<NetError> for H2Error {
    fn from(err: NetError) -> Self {
        H2Error::Net(err)
    }
}

/// A connection the client can read and write in turns.
pub trait Transport {
    /// Write all of `data`.
    fn send(&mut self, data: &[u8]) -> Result<(), NetError>;
    /// Read some bytes. Zero means the connection ended.
    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError>;
}

impl Transport for std::net::TcpStream {
    fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        std::io::Write::write_all(self, data).map_err(NetError::Io)
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        match std::io::Read::read(self, buf) {
            Ok(n) => Ok(n),
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Ok(0)
            }
            Err(err) => Err(NetError::Io(err)),
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
impl Transport for crate::tls::TlsSession {
    fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.write_all(data)
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        Ok(self.read(buf))
    }
}

impl<S: std::io::Read + std::io::Write> Transport for crate::pqc_tls13::Tls13Stream<S> {
    fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        std::io::Write::write_all(self, data).map_err(NetError::Io)
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, NetError> {
        std::io::Read::read(self, buf).map_err(NetError::Io)
    }
}

/// One raw frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Frame type, see [`kind`].
    pub kind: u8,
    /// Flag bits, see [`flag`].
    pub flags: u8,
    /// Stream number with the reserved bit cleared.
    pub stream_id: u32,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame with the given parts.
    pub fn new(kind: u8, flags: u8, stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            kind,
            flags,
            stream_id: stream_id & 0x7fff_ffff,
            payload,
        }
    }

    /// Append this frame in wire format to `out`. The payload must fit in 24 bits.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        let len = self.payload.len().min(0xff_ffff);
        out.push((len >> 16) as u8);
        out.push((len >> 8) as u8);
        out.push(len as u8);
        out.push(self.kind);
        out.push(self.flags);
        out.extend_from_slice(&(self.stream_id & 0x7fff_ffff).to_be_bytes());
        out.extend_from_slice(&self.payload[..len]);
    }

    /// This frame in wire format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.payload.len());
        self.write_to(&mut out);
        out
    }

    /// Whether `bit` is set in the flags.
    pub fn has(&self, bit: u8) -> bool {
        self.flags & bit != 0
    }
}

/// Cuts a byte stream into frames.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    start: usize,
}

impl FrameReader {
    /// An empty reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add received bytes.
    pub fn push(&mut self, data: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// The next whole frame, `None` if more bytes are needed, or an error for
    /// a frame longer than [`MAX_FRAME_SIZE`].
    pub fn next_frame(&mut self) -> Result<Option<Frame>, NetError> {
        let rest = &self.buf[self.start..];
        if rest.len() < 9 {
            return Ok(None);
        }
        let len = (usize::from(rest[0]) << 16) | (usize::from(rest[1]) << 8) | usize::from(rest[2]);
        if len > MAX_FRAME_SIZE {
            return Err(NetError::InvalidResponse);
        }
        if rest.len() < 9 + len {
            return Ok(None);
        }
        let stream_id = u32::from_be_bytes([rest[5], rest[6], rest[7], rest[8]]) & 0x7fff_ffff;
        let frame = Frame {
            kind: rest[3],
            flags: rest[4],
            stream_id,
            payload: rest[9..9 + len].to_vec(),
        };
        self.start += 9 + len;
        if self.start > 64 * 1024 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(frame))
    }
}

/// A frame checked against RFC 9113 and split into its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// DATA with padding removed. `flow_len` is the whole payload, which is
    /// what flow control counts.
    Data {
        /// Stream number.
        stream_id: u32,
        /// Body bytes.
        data: Vec<u8>,
        /// Bytes counted against the windows.
        flow_len: u32,
        /// Last frame of the stream.
        end_stream: bool,
    },
    /// HEADERS with padding and priority removed.
    Headers {
        /// Stream number.
        stream_id: u32,
        /// Header block fragment.
        block: Vec<u8>,
        /// Last frame of the stream.
        end_stream: bool,
        /// Last frame of the header block.
        end_headers: bool,
    },
    /// PRIORITY. Nothing to do.
    Priority,
    /// RST_STREAM.
    RstStream {
        /// Stream number.
        stream_id: u32,
        /// Error code.
        code: u32,
    },
    /// SETTINGS.
    Settings {
        /// This is an acknowledgement.
        ack: bool,
        /// Identifier and value pairs.
        values: Vec<(u16, u32)>,
    },
    /// PUSH_PROMISE. The client turned push off, so this is a protocol error
    /// at the connection level; it is still parsed so the caller decides.
    PushPromise,
    /// PING.
    Ping {
        /// This is an answer.
        ack: bool,
        /// Opaque data to echo.
        data: [u8; 8],
    },
    /// GOAWAY.
    GoAway {
        /// Highest stream the server may have worked on.
        last_stream_id: u32,
        /// Error code.
        code: u32,
    },
    /// WINDOW_UPDATE.
    WindowUpdate {
        /// Stream number, 0 for the connection.
        stream_id: u32,
        /// Credit added.
        increment: u32,
    },
    /// CONTINUATION.
    Continuation {
        /// Stream number.
        stream_id: u32,
        /// Header block fragment.
        block: Vec<u8>,
        /// Last frame of the header block.
        end_headers: bool,
    },
    /// A frame type this client does not know. Ignored.
    Unknown,
}

/// Strip the pad length byte and the padding from a padded payload.
fn strip_padding(frame: &Frame) -> Result<&[u8], NetError> {
    if !frame.has(flag::PADDED) {
        return Ok(&frame.payload);
    }
    let (&pad, rest) = frame
        .payload
        .split_first()
        .ok_or(NetError::InvalidResponse)?;
    let pad = usize::from(pad);
    if pad > rest.len() {
        return Err(NetError::InvalidResponse);
    }
    Ok(&rest[..rest.len() - pad])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Check one frame's size, stream number, and padding, and split out its fields.
pub fn parse_frame(frame: &Frame) -> Result<Parsed, NetError> {
    let bad = Err(NetError::InvalidResponse);
    let sid = frame.stream_id;
    let len = frame.payload.len();
    match frame.kind {
        kind::DATA => {
            if sid == 0 {
                return bad;
            }
            let data = strip_padding(frame)?.to_vec();
            Ok(Parsed::Data {
                stream_id: sid,
                data,
                flow_len: len as u32,
                end_stream: frame.has(flag::END_STREAM),
            })
        }
        kind::HEADERS => {
            if sid == 0 {
                return bad;
            }
            let mut block = strip_padding(frame)?;
            if frame.has(flag::PRIORITY) {
                if block.len() < 5 {
                    return bad;
                }
                block = &block[5..];
            }
            Ok(Parsed::Headers {
                stream_id: sid,
                block: block.to_vec(),
                end_stream: frame.has(flag::END_STREAM),
                end_headers: frame.has(flag::END_HEADERS),
            })
        }
        kind::PRIORITY => {
            if sid == 0 || len != 5 {
                return bad;
            }
            Ok(Parsed::Priority)
        }
        kind::RST_STREAM => {
            if sid == 0 || len != 4 {
                return bad;
            }
            Ok(Parsed::RstStream {
                stream_id: sid,
                code: read_u32(&frame.payload),
            })
        }
        kind::SETTINGS => {
            if sid != 0 || !len.is_multiple_of(6) {
                return bad;
            }
            let ack = frame.has(flag::ACK);
            if ack && len != 0 {
                return bad;
            }
            let values = frame
                .payload
                .as_chunks::<6>()
                .0
                .iter()
                .map(|c| (u16::from_be_bytes([c[0], c[1]]), read_u32(&c[2..6])))
                .collect();
            Ok(Parsed::Settings { ack, values })
        }
        kind::PUSH_PROMISE => {
            if sid == 0 {
                return bad;
            }
            Ok(Parsed::PushPromise)
        }
        kind::PING => {
            if sid != 0 || len != 8 {
                return bad;
            }
            let mut data = [0u8; 8];
            data.copy_from_slice(&frame.payload);
            Ok(Parsed::Ping {
                ack: frame.has(flag::ACK),
                data,
            })
        }
        kind::GOAWAY => {
            if sid != 0 || len < 8 {
                return bad;
            }
            Ok(Parsed::GoAway {
                last_stream_id: read_u32(&frame.payload) & 0x7fff_ffff,
                code: read_u32(&frame.payload[4..]),
            })
        }
        kind::WINDOW_UPDATE => {
            if len != 4 {
                return bad;
            }
            let increment = read_u32(&frame.payload) & 0x7fff_ffff;
            if increment == 0 {
                return bad;
            }
            Ok(Parsed::WindowUpdate {
                stream_id: sid,
                increment,
            })
        }
        kind::CONTINUATION => {
            if sid == 0 {
                return bad;
            }
            Ok(Parsed::Continuation {
                stream_id: sid,
                block: frame.payload.clone(),
                end_headers: frame.has(flag::END_HEADERS),
            })
        }
        _ => Ok(Parsed::Unknown),
    }
}

/// One request to send over HTTP/2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// `GET`, `POST`, and so on.
    pub method: String,
    /// `https` or `http`.
    pub scheme: String,
    /// Host, with the port when it is not the default.
    pub authority: String,
    /// Path and query. Never empty.
    pub path: String,
    /// Regular header fields. Names are lowercased when sent, and fields
    /// HTTP/2 does not allow (`connection`, `host`, and the like) are dropped.
    pub headers: Vec<(String, String)>,
    /// Request body.
    pub body: Vec<u8>,
}

/// Header names that belong to one HTTP/1.1 connection and must not be sent
/// over HTTP/2.
const CONNECTION_HEADERS: &[&str] = &[
    "connection",
    "host",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

impl Request {
    /// Read back an HTTP/1.1 request that this crate built, so the same bytes
    /// can go out over either protocol. `None` if it does not parse.
    pub fn from_http1(scheme: &str, raw: &[u8]) -> Option<Self> {
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&raw[..split]).ok()?;
        let body = raw[split + 4..].to_vec();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_string();
        let path = first.next()?.to_string();
        if method.is_empty() || path.is_empty() {
            return None;
        }
        let mut authority = String::new();
        let mut headers = Vec::new();
        for line in lines {
            let (name, value) = line.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "host" {
                authority = value;
            } else {
                headers.push((name, value));
            }
        }
        if authority.is_empty() {
            return None;
        }
        Some(Self {
            method,
            scheme: scheme.to_string(),
            authority,
            path,
            headers,
            body,
        })
    }

    /// The header list as sent: pseudo-fields first, then regular fields.
    pub fn header_list(&self) -> Vec<Header> {
        let mut list = vec![
            Header::new(":method", &self.method),
            Header::new(":scheme", &self.scheme),
            Header::new(":authority", &self.authority),
            Header::new(":path", &self.path),
        ];
        for (name, value) in &self.headers {
            let name = name.to_ascii_lowercase();
            if name.is_empty()
                || name.starts_with(':')
                || CONNECTION_HEADERS.contains(&name.as_str())
            {
                continue;
            }
            // `te` may only say `trailers` over HTTP/2.
            if name == "te" && !value.eq_ignore_ascii_case("trailers") {
                continue;
            }
            if name == "cookie" || name == "authorization" {
                // Credentials stay out of the compression tables.
                list.push(Header::sensitive(&name, value));
            } else {
                list.push(Header::new(&name, value));
            }
        }
        list
    }
}

/// The client's opening bytes: preface, SETTINGS, and a connection WINDOW_UPDATE.
pub fn client_preface() -> Vec<u8> {
    let mut out = PREFACE.to_vec();
    let mut settings = Vec::new();
    for (id, value) in [
        (setting::ENABLE_PUSH, 0u32),
        (setting::INITIAL_WINDOW_SIZE, RECEIVE_WINDOW),
        (setting::MAX_FRAME_SIZE, MAX_FRAME_SIZE as u32),
        (setting::MAX_HEADER_LIST_SIZE, MAX_HEADER_BYTES as u32),
    ] {
        settings.extend_from_slice(&id.to_be_bytes());
        settings.extend_from_slice(&value.to_be_bytes());
    }
    Frame::new(kind::SETTINGS, 0, 0, settings).write_to(&mut out);
    // The connection window starts at 65,535 whatever SETTINGS says; raise it
    // to match the stream window.
    let bump = RECEIVE_WINDOW - 65_535;
    Frame::new(kind::WINDOW_UPDATE, 0, 0, bump.to_be_bytes().to_vec()).write_to(&mut out);
    out
}

/// HEADERS for `stream_id`, plus CONTINUATION frames when the block is large.
pub fn headers_frames_for_stream(
    stream_id: u32,
    block: &[u8],
    end_stream: bool,
    max_frame: usize,
) -> Vec<u8> {
    let max_frame = max_frame.max(1);
    let mut out = Vec::new();
    let mut chunks = block.chunks(max_frame).peekable();
    let first = chunks.next().unwrap_or(&[]);
    let mut flags = if end_stream { flag::END_STREAM } else { 0 };
    if chunks.peek().is_none() {
        flags |= flag::END_HEADERS;
    }
    Frame::new(kind::HEADERS, flags, stream_id, first.to_vec()).write_to(&mut out);
    while let Some(chunk) = chunks.next() {
        let flags = if chunks.peek().is_none() {
            flag::END_HEADERS
        } else {
            0
        };
        Frame::new(kind::CONTINUATION, flags, stream_id, chunk.to_vec()).write_to(&mut out);
    }
    out
}

/// HEADERS for stream 1, plus CONTINUATION frames when the block is large.
pub fn headers_frames(block: &[u8], end_stream: bool, max_frame: usize) -> Vec<u8> {
    let max_frame = max_frame.max(1);
    let mut out = Vec::new();
    let mut chunks = block.chunks(max_frame).peekable();
    let first = chunks.next().unwrap_or(&[]);
    let mut flags = if end_stream { flag::END_STREAM } else { 0 };
    if chunks.peek().is_none() {
        flags |= flag::END_HEADERS;
    }
    Frame::new(kind::HEADERS, flags, 1, first.to_vec()).write_to(&mut out);
    while let Some(chunk) = chunks.next() {
        let flags = if chunks.peek().is_none() {
            flag::END_HEADERS
        } else {
            0
        };
        Frame::new(kind::CONTINUATION, flags, 1, chunk.to_vec()).write_to(&mut out);
    }
    out
}

/// What is known about the response so far.
struct StreamState {
    status: Option<u16>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    done: bool,
}

/// Header block being collected across HEADERS and CONTINUATION.
struct PendingBlock {
    stream_id: u32,
    block: Vec<u8>,
    end_stream: bool,
}

/// The client's side of one connection.
struct Connection<'a, T: Transport> {
    io: &'a mut T,
    reader: FrameReader,
    decoder: Decoder,
    send_window_conn: i64,
    send_window_stream: i64,
    peer_initial_window: i64,
    peer_max_frame: usize,
    stream: StreamState,
    pending: Option<PendingBlock>,
    max_body: usize,
}

impl<T: Transport> Connection<'_, T> {
    fn read_frame(&mut self) -> Result<Frame, NetError> {
        loop {
            if let Some(frame) = self.reader.next_frame()? {
                return Ok(frame);
            }
            let mut buf = [0u8; 16 * 1024];
            let n = self.io.recv(&mut buf)?;
            if n == 0 {
                return Err(NetError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP/2 connection closed before the response finished",
                )));
            }
            self.reader.push(&buf[..n]);
        }
    }

    fn handle(&mut self, frame: Frame) -> Result<(), H2Error> {
        let parsed = parse_frame(&frame)?;
        // Inside a header block only CONTINUATION for the same stream may come.
        if let Some(pending) = &self.pending {
            match &parsed {
                Parsed::Continuation { stream_id, .. } if *stream_id == pending.stream_id => {}
                _ => return Err(NetError::InvalidResponse.into()),
            }
        }
        match parsed {
            Parsed::Data {
                stream_id,
                data,
                flow_len,
                end_stream,
            } => {
                if stream_id == 1 {
                    // Body before the headers or after the end is a protocol error.
                    if self.stream.status.is_none() || self.stream.done {
                        return Err(NetError::InvalidResponse.into());
                    }
                    if self.stream.body.len() + data.len() > self.max_body {
                        return Err(NetError::BodyTooLarge.into());
                    }
                    self.stream.body.extend_from_slice(&data);
                    if end_stream {
                        self.stream.done = true;
                    }
                }
                if flow_len > 0 {
                    // Give the bytes back so the server can keep sending.
                    let mut out = Vec::new();
                    let inc = flow_len.to_be_bytes().to_vec();
                    Frame::new(kind::WINDOW_UPDATE, 0, 0, inc.clone()).write_to(&mut out);
                    if stream_id == 1 && !self.stream.done {
                        Frame::new(kind::WINDOW_UPDATE, 0, 1, inc).write_to(&mut out);
                    }
                    self.io.send(&out)?;
                }
            }
            Parsed::Headers {
                stream_id,
                block,
                end_stream,
                end_headers,
            } => {
                if stream_id % 2 == 0 {
                    // Even streams are server-initiated; with push off there are none.
                    return Err(NetError::InvalidResponse.into());
                }
                let pending = PendingBlock {
                    stream_id,
                    block,
                    end_stream,
                };
                if end_headers {
                    self.finish_block(pending)?;
                } else {
                    self.check_block_size(pending.block.len())?;
                    self.pending = Some(pending);
                }
            }
            Parsed::Continuation {
                stream_id,
                block,
                end_headers,
            } => {
                let Some(mut pending) = self.pending.take() else {
                    return Err(NetError::InvalidResponse.into());
                };
                if pending.stream_id != stream_id {
                    return Err(NetError::InvalidResponse.into());
                }
                self.check_block_size(pending.block.len() + block.len())?;
                pending.block.extend_from_slice(&block);
                if end_headers {
                    self.finish_block(pending)?;
                } else {
                    self.pending = Some(pending);
                }
            }
            Parsed::Priority | Parsed::Unknown => {}
            Parsed::RstStream { stream_id, code } => {
                if stream_id == 1 && !self.stream.done {
                    return Err(match code {
                        code::HTTP_1_1_REQUIRED => H2Error::Fallback,
                        code::REFUSED_STREAM if self.stream.status.is_none() => H2Error::Fallback,
                        _ => H2Error::Net(NetError::Io(std::io::Error::other(format!(
                            "HTTP/2 stream reset by the server (code {code})"
                        )))),
                    });
                }
            }
            Parsed::Settings { ack, values } => {
                if !ack {
                    self.apply_settings(&values)?;
                    self.io
                        .send(&Frame::new(kind::SETTINGS, flag::ACK, 0, Vec::new()).to_bytes())?;
                }
            }
            Parsed::PushPromise => return Err(NetError::InvalidResponse.into()),
            Parsed::Ping { ack, data } => {
                if !ack {
                    self.io
                        .send(&Frame::new(kind::PING, flag::ACK, 0, data.to_vec()).to_bytes())?;
                }
            }
            Parsed::GoAway {
                last_stream_id,
                code,
            } => {
                if code == code::HTTP_1_1_REQUIRED
                    || (last_stream_id < 1 && self.stream.status.is_none())
                {
                    return Err(H2Error::Fallback);
                }
                if last_stream_id < 1 && !self.stream.done {
                    return Err(NetError::InvalidResponse.into());
                }
                // Stream 1 is still being answered; keep reading.
            }
            Parsed::WindowUpdate {
                stream_id,
                increment,
            } => {
                let window = if stream_id == 0 {
                    &mut self.send_window_conn
                } else if stream_id == 1 {
                    &mut self.send_window_stream
                } else {
                    return Ok(());
                };
                *window += i64::from(increment);
                if *window > MAX_WINDOW {
                    return Err(NetError::InvalidResponse.into());
                }
            }
        }
        Ok(())
    }

    fn check_block_size(&self, len: usize) -> Result<(), H2Error> {
        if len > MAX_HEADER_BLOCK {
            return Err(NetError::InvalidResponse.into());
        }
        Ok(())
    }

    fn apply_settings(&mut self, values: &[(u16, u32)]) -> Result<(), H2Error> {
        for &(id, value) in values {
            match id {
                setting::ENABLE_PUSH if value > 1 => {
                    return Err(NetError::InvalidResponse.into());
                }
                setting::INITIAL_WINDOW_SIZE => {
                    let value = i64::from(value);
                    if value > MAX_WINDOW {
                        return Err(NetError::InvalidResponse.into());
                    }
                    self.send_window_stream += value - self.peer_initial_window;
                    self.peer_initial_window = value;
                    if self.send_window_stream > MAX_WINDOW {
                        return Err(NetError::InvalidResponse.into());
                    }
                }
                setting::MAX_FRAME_SIZE => {
                    if !(16_384..=16_777_215).contains(&value) {
                        return Err(NetError::InvalidResponse.into());
                    }
                    // Never send more than the client itself accepts.
                    self.peer_max_frame = (value as usize).min(MAX_FRAME_SIZE);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn finish_block(&mut self, pending: PendingBlock) -> Result<(), H2Error> {
        self.check_block_size(pending.block.len())?;
        // Every block is decoded, even for other streams, so the table stays in step.
        let fields = self
            .decoder
            .decode(&pending.block)
            .map_err(|_| NetError::InvalidResponse)?;
        if pending.stream_id != 1 {
            return Ok(());
        }
        if self.stream.done {
            return Err(NetError::InvalidResponse.into());
        }
        if self.stream.status.is_some() {
            // Trailers. They must end the stream and carry no pseudo-fields.
            if !pending.end_stream || fields.iter().any(|f| f.name.first() == Some(&b':')) {
                return Err(NetError::InvalidResponse.into());
            }
            self.stream.done = true;
            return Ok(());
        }
        let (status, headers) = response_fields(&fields)?;
        if (100..200).contains(&status) {
            // An interim response. The final one follows.
            if pending.end_stream || status == 101 {
                return Err(NetError::InvalidResponse.into());
            }
            return Ok(());
        }
        self.stream.status = Some(status);
        self.stream.headers = headers;
        if pending.end_stream {
            self.stream.done = true;
        }
        Ok(())
    }

    /// Send as much of `body` as the windows allow and return how much went out.
    fn send_body(&mut self, body: &[u8], mut offset: usize) -> Result<usize, H2Error> {
        let mut out = Vec::new();
        while offset < body.len() {
            let window = self.send_window_conn.min(self.send_window_stream);
            if window <= 0 {
                break;
            }
            let n = (body.len() - offset)
                .min(self.peer_max_frame)
                .min(window as usize);
            let end = offset + n == body.len();
            let flags = if end { flag::END_STREAM } else { 0 };
            Frame::new(kind::DATA, flags, 1, body[offset..offset + n].to_vec()).write_to(&mut out);
            self.send_window_conn -= n as i64;
            self.send_window_stream -= n as i64;
            offset += n;
        }
        if !out.is_empty() {
            self.io.send(&out)?;
        }
        Ok(offset)
    }
}

/// Split a decoded response header list into the status and regular fields.
fn response_fields(fields: &[Header]) -> Result<(u16, Vec<(String, String)>), NetError> {
    let mut status = None;
    let mut headers = Vec::new();
    let mut seen_regular = false;
    for field in fields {
        if field.name.iter().any(|b| b.is_ascii_uppercase()) || field.name.is_empty() {
            return Err(NetError::InvalidResponse);
        }
        // RFC 9113 section 8.2.1: CR, LF, or NUL anywhere, or a space in a
        // name, makes the response malformed. HTTP/1.1 parsing can't produce
        // these, and the window treats both kinds of response the same.
        let bad_byte = |b: &u8| matches!(b, b'\r' | b'\n' | 0);
        if field.name.iter().any(|b| bad_byte(b) || *b == b' ') || field.value.iter().any(bad_byte)
        {
            return Err(NetError::InvalidResponse);
        }
        if field.name.first() == Some(&b':') {
            // Pseudo-fields come first, once, and a response has only `:status`.
            if seen_regular || field.name != b":status" || status.is_some() {
                return Err(NetError::InvalidResponse);
            }
            let text = std::str::from_utf8(&field.value).map_err(|_| NetError::InvalidResponse)?;
            if text.len() != 3 || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(NetError::InvalidResponse);
            }
            status = Some(text.parse::<u16>().map_err(|_| NetError::InvalidResponse)?);
            continue;
        }
        seen_regular = true;
        let name = field.name_str();
        if CONNECTION_HEADERS.contains(&name.as_str()) && name != "host" {
            return Err(NetError::InvalidResponse);
        }
        if headers.len() >= MAX_HEADER_COUNT {
            return Err(NetError::InvalidResponse);
        }
        headers.push((name, field.value_str().trim().to_string()));
    }
    let status = status.ok_or(NetError::InvalidResponse)?;
    Ok((status, headers))
}

/// Status, header fields, and body bytes before content decoding.
pub type RawResponse = (u16, Vec<(String, String)>, Vec<u8>);

/// Send `request` on stream 1 of a fresh HTTP/2 connection and read the
/// response. The body is returned as received, before any content decoding.
pub fn exchange_raw<T: Transport>(
    io: &mut T,
    request: &Request,
    max_body: usize,
) -> Result<RawResponse, H2Error> {
    let mut encoder = Encoder::new();
    let mut block = Vec::new();
    encoder.encode(&request.header_list(), &mut block);
    if block.len() > MAX_HEADER_BLOCK {
        return Err(NetError::InvalidUrl.into());
    }
    let has_body = !request.body.is_empty();
    let mut opening = client_preface();
    opening.extend_from_slice(&headers_frames(&block, !has_body, MAX_FRAME_SIZE));
    io.send(&opening)?;

    let mut conn = Connection {
        io,
        reader: FrameReader::new(),
        decoder: Decoder::with_limits(anvil_hpack::DEFAULT_TABLE_SIZE, MAX_HEADER_BYTES),
        send_window_conn: 65_535,
        send_window_stream: 65_535,
        peer_initial_window: 65_535,
        peer_max_frame: MAX_FRAME_SIZE,
        stream: StreamState {
            status: None,
            headers: Vec::new(),
            body: Vec::new(),
            done: false,
        },
        pending: None,
        max_body,
    };

    let mut sent = 0usize;
    loop {
        if has_body && sent < request.body.len() && !conn.stream.done {
            sent = conn.send_body(&request.body, sent)?;
        }
        if conn.stream.done {
            break;
        }
        let frame = conn.read_frame()?;
        conn.handle(frame)?;
    }
    // Say goodbye; the socket closes right after, so a failed write is fine.
    let mut bye = Vec::new();
    bye.extend_from_slice(&1u32.to_be_bytes());
    bye.extend_from_slice(&code::NO_ERROR.to_be_bytes());
    let _ = conn
        .io
        .send(&Frame::new(kind::GOAWAY, 0, 0, bye).to_bytes());

    let status = conn.stream.status.ok_or(NetError::InvalidResponse)?;
    let headers = std::mem::take(&mut conn.stream.headers);
    let body = std::mem::take(&mut conn.stream.body);
    let length_mismatch = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .is_some_and(|len| len != body.len());
    if length_mismatch {
        return Err(NetError::InvalidResponse.into());
    }
    Ok((status, headers, body))
}

struct ActiveStream {
    state: StreamState,
    send_window: i64,
    body_to_send: Vec<u8>,
    body_sent: usize,
}

/// Persistent HTTP/2 client session supporting multi-stream keep-alive (`1, 3, 5, ...`)
/// and concurrent stream multiplexing over a single transport.
pub struct H2ClientSession<T: Transport> {
    io: T,
    reader: FrameReader,
    encoder: Encoder,
    decoder: Decoder,
    send_window_conn: i64,
    peer_initial_window: i64,
    peer_max_frame: usize,
    next_stream_id: u32,
    streams_completed: usize,
    pending: Option<PendingBlock>,
}

impl<T: Transport> H2ClientSession<T> {
    /// Open a persistent HTTP/2 client session on `io` and send the client preface.
    pub fn open(mut io: T) -> Result<Self, H2Error> {
        io.send(&client_preface())?;
        Ok(Self {
            io,
            reader: FrameReader::new(),
            encoder: Encoder::new(),
            decoder: Decoder::with_limits(anvil_hpack::DEFAULT_TABLE_SIZE, MAX_HEADER_BYTES),
            send_window_conn: 65_535,
            peer_initial_window: 65_535,
            peer_max_frame: MAX_FRAME_SIZE,
            next_stream_id: 1,
            streams_completed: 0,
            pending: None,
        })
    }

    /// Next odd client-initiated stream ID (`1, 3, 5, ...`).
    pub fn next_stream_id(&self) -> u32 {
        self.next_stream_id
    }

    /// Number of HTTP/2 streams completed on this connection.
    pub fn streams_completed(&self) -> usize {
        self.streams_completed
    }

    /// Execute one request on the next odd stream ID (`1, 3, 5, ...`) while keeping the connection open.
    pub fn exchange(&mut self, request: &Request, max_body: usize) -> Result<Response, H2Error> {
        let mut results = self.exchange_multiplexed(std::slice::from_ref(request), max_body)?;
        results.pop().ok_or(H2Error::Net(NetError::InvalidResponse))
    }

    /// Multiplex `requests` concurrently across odd client-initiated streams (`1, 3, 5, ...`)
    /// on this single HTTP/2 connection and return responses in request order.
    pub fn exchange_multiplexed(
        &mut self,
        requests: &[Request],
        max_body: usize,
    ) -> Result<Vec<Response>, H2Error> {
        use std::collections::BTreeMap;

        if requests.is_empty() {
            return Ok(Vec::new());
        }

        let mut streams: BTreeMap<u32, ActiveStream> = BTreeMap::new();
        let mut order: Vec<u32> = Vec::with_capacity(requests.len());
        let mut batch_wire = Vec::new();

        for req in requests {
            let sid = self.next_stream_id;
            self.next_stream_id = self.next_stream_id.saturating_add(2);
            let mut block = Vec::new();
            self.encoder.encode(&req.header_list(), &mut block);
            if block.len() > MAX_HEADER_BLOCK {
                return Err(NetError::InvalidUrl.into());
            }
            let has_body = !req.body.is_empty();
            batch_wire.extend_from_slice(&headers_frames_for_stream(
                sid,
                &block,
                !has_body,
                self.peer_max_frame,
            ));
            streams.insert(
                sid,
                ActiveStream {
                    state: StreamState {
                        status: None,
                        headers: Vec::new(),
                        body: Vec::new(),
                        done: false,
                    },
                    send_window: self.peer_initial_window,
                    body_to_send: req.body.clone(),
                    body_sent: 0,
                },
            );
            order.push(sid);
        }
        self.io.send(&batch_wire)?;

        loop {
            let mut data_wire = Vec::new();
            for (&sid, active) in streams.iter_mut() {
                while active.body_sent < active.body_to_send.len() && !active.state.done {
                    let window = self.send_window_conn.min(active.send_window);
                    if window <= 0 {
                        break;
                    }
                    let n = (active.body_to_send.len() - active.body_sent)
                        .min(self.peer_max_frame)
                        .min(window as usize);
                    let end = active.body_sent + n == active.body_to_send.len();
                    let flags = if end { flag::END_STREAM } else { 0 };
                    Frame::new(
                        kind::DATA,
                        flags,
                        sid,
                        active.body_to_send[active.body_sent..active.body_sent + n].to_vec(),
                    )
                    .write_to(&mut data_wire);
                    self.send_window_conn -= n as i64;
                    active.send_window -= n as i64;
                    active.body_sent += n;
                }
            }
            if !data_wire.is_empty() {
                self.io.send(&data_wire)?;
            }

            if streams.values().all(|s| s.state.done) {
                break;
            }

            let frame = self.read_frame()?;
            let parsed = parse_frame(&frame)?;
            if let Some(pending) = &self.pending {
                match &parsed {
                    Parsed::Continuation { stream_id, .. } if *stream_id == pending.stream_id => {}
                    _ => return Err(NetError::InvalidResponse.into()),
                }
            }

            match parsed {
                Parsed::Data {
                    stream_id,
                    data,
                    flow_len,
                    end_stream,
                } => {
                    let mut stream_still_open = false;
                    if let Some(active) = streams.get_mut(&stream_id) {
                        if active.state.status.is_none() || active.state.done {
                            return Err(NetError::InvalidResponse.into());
                        }
                        if active.state.body.len() + data.len() > max_body {
                            return Err(NetError::BodyTooLarge.into());
                        }
                        active.state.body.extend_from_slice(&data);
                        if end_stream {
                            active.state.done = true;
                        }
                        stream_still_open = !active.state.done;
                    }
                    if flow_len > 0 {
                        let mut out = Vec::new();
                        let inc = flow_len.to_be_bytes().to_vec();
                        Frame::new(kind::WINDOW_UPDATE, 0, 0, inc.clone()).write_to(&mut out);
                        if stream_still_open {
                            Frame::new(kind::WINDOW_UPDATE, 0, stream_id, inc).write_to(&mut out);
                        }
                        self.io.send(&out)?;
                    }
                }
                Parsed::Headers {
                    stream_id,
                    block,
                    end_stream,
                    end_headers,
                } => {
                    if stream_id % 2 == 0 {
                        return Err(NetError::InvalidResponse.into());
                    }
                    let pending = PendingBlock {
                        stream_id,
                        block,
                        end_stream,
                    };
                    if end_headers {
                        self.finish_multiplex_block(pending, &mut streams)?;
                    } else {
                        if pending.block.len() > MAX_HEADER_BLOCK {
                            return Err(NetError::InvalidResponse.into());
                        }
                        self.pending = Some(pending);
                    }
                }
                Parsed::Continuation {
                    stream_id,
                    block,
                    end_headers,
                } => {
                    let Some(mut pending) = self.pending.take() else {
                        return Err(NetError::InvalidResponse.into());
                    };
                    if pending.stream_id != stream_id
                        || pending.block.len() + block.len() > MAX_HEADER_BLOCK
                    {
                        return Err(NetError::InvalidResponse.into());
                    }
                    pending.block.extend_from_slice(&block);
                    if end_headers {
                        self.finish_multiplex_block(pending, &mut streams)?;
                    } else {
                        self.pending = Some(pending);
                    }
                }
                Parsed::Priority | Parsed::Unknown => {}
                Parsed::RstStream { stream_id, code } => {
                    if let Some(active) = streams.get(&stream_id)
                        && !active.state.done
                    {
                        return Err(match code {
                            code::HTTP_1_1_REQUIRED => H2Error::Fallback,
                            code::REFUSED_STREAM if active.state.status.is_none() => {
                                H2Error::Fallback
                            }
                            _ => H2Error::Net(NetError::Io(std::io::Error::other(format!(
                                "HTTP/2 stream {stream_id} reset by server (code {code})"
                            )))),
                        });
                    }
                }
                Parsed::Settings { ack, values } => {
                    if !ack {
                        for &(id, value) in &values {
                            match id {
                                setting::ENABLE_PUSH if value > 1 => {
                                    return Err(NetError::InvalidResponse.into());
                                }
                                setting::INITIAL_WINDOW_SIZE => {
                                    let value = i64::from(value);
                                    if value > MAX_WINDOW {
                                        return Err(NetError::InvalidResponse.into());
                                    }
                                    let delta = value - self.peer_initial_window;
                                    self.peer_initial_window = value;
                                    for active in streams.values_mut() {
                                        active.send_window += delta;
                                        if active.send_window > MAX_WINDOW {
                                            return Err(NetError::InvalidResponse.into());
                                        }
                                    }
                                }
                                setting::MAX_FRAME_SIZE => {
                                    if !(16_384..=16_777_215).contains(&value) {
                                        return Err(NetError::InvalidResponse.into());
                                    }
                                    self.peer_max_frame = (value as usize).min(MAX_FRAME_SIZE);
                                }
                                _ => {}
                            }
                        }
                        self.io.send(
                            &Frame::new(kind::SETTINGS, flag::ACK, 0, Vec::new()).to_bytes(),
                        )?;
                    }
                }
                Parsed::PushPromise => return Err(NetError::InvalidResponse.into()),
                Parsed::Ping { ack, data } => {
                    if !ack {
                        self.io.send(
                            &Frame::new(kind::PING, flag::ACK, 0, data.to_vec()).to_bytes(),
                        )?;
                    }
                }
                Parsed::GoAway {
                    last_stream_id,
                    code,
                } => {
                    let highest = *order.last().unwrap_or(&1);
                    if code == code::HTTP_1_1_REQUIRED || last_stream_id < highest {
                        return Err(H2Error::Fallback);
                    }
                }
                Parsed::WindowUpdate {
                    stream_id,
                    increment,
                } => {
                    if stream_id == 0 {
                        self.send_window_conn += i64::from(increment);
                        if self.send_window_conn > MAX_WINDOW {
                            return Err(NetError::InvalidResponse.into());
                        }
                    } else if let Some(active) = streams.get_mut(&stream_id) {
                        active.send_window += i64::from(increment);
                        if active.send_window > MAX_WINDOW {
                            return Err(NetError::InvalidResponse.into());
                        }
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(order.len());
        for sid in order {
            let Some(active) = streams.remove(&sid) else {
                return Err(NetError::InvalidResponse.into());
            };
            let status = active.state.status.ok_or(NetError::InvalidResponse)?;
            let headers = active.state.headers;
            let raw = active.state.body;
            let length_mismatch = headers
                .iter()
                .find(|(name, _)| name == "content-length")
                .and_then(|(_, value)| value.parse::<usize>().ok())
                .is_some_and(|len| len != raw.len());
            if length_mismatch {
                return Err(NetError::InvalidResponse.into());
            }
            let body = crate::decode_content_encoding(&headers, raw)?;
            self.streams_completed += 1;
            out.push(Response {
                status,
                reason: String::new(),
                headers,
                body,
                version: crate::HttpVersion::Http2,
                key_exchange: None,
            });
        }
        Ok(out)
    }

    fn read_frame(&mut self) -> Result<Frame, NetError> {
        loop {
            if let Some(frame) = self.reader.next_frame()? {
                return Ok(frame);
            }
            let mut buf = [0u8; 16 * 1024];
            let n = self.io.recv(&mut buf)?;
            if n == 0 {
                return Err(NetError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP/2 connection closed before stream finished",
                )));
            }
            self.reader.push(&buf[..n]);
        }
    }

    fn finish_multiplex_block(
        &mut self,
        pending: PendingBlock,
        streams: &mut std::collections::BTreeMap<u32, ActiveStream>,
    ) -> Result<(), H2Error> {
        if pending.block.len() > MAX_HEADER_BLOCK {
            return Err(NetError::InvalidResponse.into());
        }
        let fields = self
            .decoder
            .decode(&pending.block)
            .map_err(|_| NetError::InvalidResponse)?;
        let Some(active) = streams.get_mut(&pending.stream_id) else {
            return Ok(());
        };
        let state = &mut active.state;
        if state.done {
            return Err(NetError::InvalidResponse.into());
        }
        if state.status.is_some() {
            if !pending.end_stream || fields.iter().any(|f| f.name.first() == Some(&b':')) {
                return Err(NetError::InvalidResponse.into());
            }
            state.done = true;
            return Ok(());
        }
        let (status, headers) = response_fields(&fields)?;
        if (100..200).contains(&status) {
            if pending.end_stream || status == 101 {
                return Err(NetError::InvalidResponse.into());
            }
            return Ok(());
        }
        state.status = Some(status);
        state.headers = headers;
        if pending.end_stream {
            state.done = true;
        }
        Ok(())
    }

    /// Send GOAWAY and consume the session.
    pub fn close(mut self) {
        let last_sid = self.next_stream_id.saturating_sub(2);
        let mut bye = Vec::with_capacity(8);
        bye.extend_from_slice(&last_sid.to_be_bytes());
        bye.extend_from_slice(&code::NO_ERROR.to_be_bytes());
        let _ = self
            .io
            .send(&Frame::new(kind::GOAWAY, 0, 0, bye).to_bytes());
    }
}

/// Like [`exchange_raw`], with the body decoded from `gzip` or `deflate` the
/// same way HTTP/1.1 responses are. The reason phrase is empty because
/// HTTP/2 has none.
pub fn exchange<T: Transport>(
    io: &mut T,
    request: &Request,
    max_body: usize,
) -> Result<Response, H2Error> {
    let (status, headers, raw) = exchange_raw(io, request, max_body)?;
    let body = crate::decode_content_encoding(&headers, raw)?;
    Ok(Response {
        status,
        reason: String::new(),
        headers,
        body,
        version: crate::HttpVersion::Http2,
        key_exchange: None,
    })
}
