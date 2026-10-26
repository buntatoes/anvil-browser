// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! The renderer wire format.
//!
//! This is the message format between the browser process and a renderer
//! process. It is small on purpose, so it is easy to read, check and fuzz.
//!
//! Every frame is a 16-byte header and a body:
//!
//! | Bytes | Field | Rule |
//! | --- | --- | --- |
//! | 0..4 | magic | `ANVR` |
//! | 4..6 | version | little-endian, must be [`WIRE_VERSION`] |
//! | 6 | kind | one of [`MessageKind`] |
//! | 7 | flags | must be 0 |
//! | 8..12 | seq | little-endian, a reply carries the seq of its request |
//! | 12..16 | body length | little-endian, at most the cap for that kind |
//!
//! The body is a fixed list of fields for each kind. Numbers are
//! little-endian. A bool is one byte, 0 or 1. Text is a `u32` length and
//! UTF-8 bytes. Raw bytes are a `u32` length and the bytes. A list is a `u16`
//! count and its items. Each field has its own size cap, and the body must be
//! used up exactly.
//!
//! The rules that keep this safe:
//!
//! - No pointers, handles, offsets or shared memory. Only values.
//! - Every length is checked against its cap before anything is allocated.
//! - A frame is checked against the direction it travels in, so a renderer
//!   can't send a message that only the browser may send.
//! - Anything unknown is refused: a new version, a set flag, an unknown kind,
//!   a bool that isn't 0 or 1, an unknown enum value, or spare bytes.
//! - Decoding is exact, so a frame that decodes encodes back to the same bytes.
//!   The fuzz test checks that.
//!
//! The renderer is treated as hostile. A frame from it that breaks any rule
//! means the browser kills it, so there is no recovery path to get wrong.

#![forbid(unsafe_code)]

use std::fmt;
use std::io::{self, Read, Write};

/// First four bytes of every frame.
pub const WIRE_MAGIC: [u8; 4] = *b"ANVR";

/// The only version this build reads and writes.
pub const WIRE_VERSION: u16 = 1;

/// Size of the frame header.
pub const HEADER_BYTES: usize = 16;

/// Largest document a request or a reply may carry (8 MiB).
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

/// Largest fetched body the browser hands to a renderer (8 MiB).
pub const MAX_FETCH_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Largest URL, in bytes.
pub const MAX_URL_BYTES: usize = 8 * 1024;

/// Largest Content-Security-Policy text, in bytes.
pub const MAX_CSP_BYTES: usize = 16 * 1024;

/// Largest page title, in bytes.
pub const MAX_TITLE_BYTES: usize = 4 * 1024;

/// Largest error detail or content type, in bytes.
pub const MAX_SHORT_TEXT_BYTES: usize = 1024;

/// Most hosts in the HTTPS-only allow list.
pub const MAX_ALLOWED_HOSTS: usize = 256;

/// Longest host name (the DNS limit).
pub const MAX_HOST_BYTES: usize = 253;

/// Largest viewport side, in CSS pixels.
pub const MAX_VIEWPORT_SIDE: u32 = 16_384;

/// Most DOM nodes a renderer may report.
pub const MAX_REPORTED_NODES: u32 = 16 * 1024 * 1024;

/// Largest body of any frame. This is the document cap plus room for the
/// other fields of the biggest message.
pub const MAX_BODY_BYTES: usize = MAX_DOCUMENT_BYTES + 128 * 1024;

/// Which side wrote a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Browser to renderer.
    ToRenderer,
    /// Renderer to browser. These frames are untrusted.
    ToBrowser,
}

/// Every message kind, with its byte on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Hello = 1,
    Ping = 2,
    Pong = 3,
    Render = 4,
    Rendered = 5,
    Failed = 6,
    Fetch = 7,
    FetchReply = 8,
    Shutdown = 9,
}

impl MessageKind {
    /// The kind for a wire byte, or `None`.
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Hello,
            2 => Self::Ping,
            3 => Self::Pong,
            4 => Self::Render,
            5 => Self::Rendered,
            6 => Self::Failed,
            7 => Self::Fetch,
            8 => Self::FetchReply,
            9 => Self::Shutdown,
            _ => return None,
        })
    }

    /// True when this kind may travel in `direction`.
    pub fn allowed(self, direction: Direction) -> bool {
        match direction {
            Direction::ToRenderer => matches!(
                self,
                Self::Hello | Self::Ping | Self::Render | Self::FetchReply | Self::Shutdown
            ),
            Direction::ToBrowser => matches!(
                self,
                Self::Hello | Self::Pong | Self::Rendered | Self::Failed | Self::Fetch
            ),
        }
    }

    /// Largest body this kind can have. The header is refused above this,
    /// before the body is read.
    pub fn max_body(self) -> usize {
        match self {
            Self::Hello => 3,
            Self::Ping | Self::Pong => 8,
            Self::Shutdown => 0,
            Self::Fetch => 8 + 1 + 4 + MAX_URL_BYTES,
            Self::Failed => 8 + 8 + 1 + 4 + MAX_SHORT_TEXT_BYTES,
            Self::Render | Self::Rendered | Self::FetchReply => MAX_BODY_BYTES,
        }
    }
}

/// Which process sent a `Hello`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Browser = 1,
    Renderer = 2,
}

/// What a renderer wants fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchKind {
    Stylesheet = 1,
    Script = 2,
    Image = 3,
    Font = 4,
}

/// Why a renderer gave up on a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCode {
    /// A parser or script limit was hit.
    Limit = 1,
    /// The finished document doesn't fit in a frame.
    TooLarge = 2,
    /// Something else went wrong inside the renderer.
    Internal = 3,
}

/// One document for a renderer to parse and run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRequest {
    pub tab_id: u64,
    /// Counts up per navigation, so a late reply can't land on a newer page.
    pub nav_id: u64,
    pub url: String,
    pub html: String,
    pub csp_header: Option<String>,
    pub scripts_enabled: bool,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub https_only: bool,
    pub http_allowed_hosts: Vec<String>,
}

/// A renderer's finished document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDocument {
    pub tab_id: u64,
    pub nav_id: u64,
    pub title: String,
    pub dom_nodes: u32,
    pub blocked_resources: u32,
    pub html: String,
}

/// A renderer's report that it could not finish a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderFailure {
    pub tab_id: u64,
    pub nav_id: u64,
    pub code: FailureCode,
    pub detail: String,
}

/// A renderer asking the browser for a subresource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    pub nav_id: u64,
    pub kind: FetchKind,
    /// Always `http://` or `https://`, visible ASCII only.
    pub url: String,
}

/// The browser's answer to a [`FetchRequest`]. A blocked or failed fetch
/// is status 0 with an empty body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchReply {
    pub nav_id: u64,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// First frame each way. The renderer answers the browser's with its own.
    Hello {
        role: Role,
        version: u16,
    },
    /// Watchdog check. The renderer answers with the same nonce.
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    Render(RenderRequest),
    Rendered(RenderedDocument),
    Failed(RenderFailure),
    Fetch(FetchRequest),
    FetchReply(FetchReply),
    /// The browser asks the renderer to exit.
    Shutdown,
}

impl Message {
    /// The kind byte this message is sent with.
    pub fn kind(&self) -> MessageKind {
        match self {
            Self::Hello { .. } => MessageKind::Hello,
            Self::Ping { .. } => MessageKind::Ping,
            Self::Pong { .. } => MessageKind::Pong,
            Self::Render(_) => MessageKind::Render,
            Self::Rendered(_) => MessageKind::Rendered,
            Self::Failed(_) => MessageKind::Failed,
            Self::Fetch(_) => MessageKind::Fetch,
            Self::FetchReply(_) => MessageKind::FetchReply,
            Self::Shutdown => MessageKind::Shutdown,
        }
    }
}

/// Why a frame was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The first four bytes are not `ANVR`.
    BadMagic,
    /// A version this build doesn't speak.
    UnsupportedVersion(u16),
    /// A flag bit is set.
    BadFlags(u8),
    /// A kind byte this build doesn't know.
    UnknownKind(u8),
    /// A known kind sent the wrong way.
    WrongDirection(MessageKind),
    /// The body is bigger than its kind allows.
    TooLarge { kind: MessageKind, len: usize },
    /// The frame or a field ended early.
    Truncated,
    /// Bytes are left over after the last field.
    TrailingBytes,
    /// A text field is not UTF-8.
    BadUtf8(&'static str),
    /// A field is longer than its cap.
    FieldTooLong(&'static str),
    /// A list has more items than its cap.
    TooMany(&'static str),
    /// A bool byte that is not 0 or 1.
    BadBool(&'static str),
    /// An enum byte with no meaning.
    BadEnum(&'static str),
    /// A number or text outside the allowed range.
    OutOfRange(&'static str),
    /// Reading or writing the pipe failed.
    Io(io::ErrorKind),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "frame does not start with ANVR"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            Self::BadFlags(b) => write!(f, "unknown flags {b:#04x}"),
            Self::UnknownKind(b) => write!(f, "unknown message kind {b}"),
            Self::WrongDirection(k) => write!(f, "{k:?} is not allowed in this direction"),
            Self::TooLarge { kind, len } => write!(f, "{kind:?} body of {len} bytes is too large"),
            Self::Truncated => write!(f, "frame ended early"),
            Self::TrailingBytes => write!(f, "frame has spare bytes"),
            Self::BadUtf8(field) => write!(f, "{field} is not UTF-8"),
            Self::FieldTooLong(field) => write!(f, "{field} is too long"),
            Self::TooMany(field) => write!(f, "{field} has too many items"),
            Self::BadBool(field) => write!(f, "{field} is not 0 or 1"),
            Self::BadEnum(field) => write!(f, "{field} has an unknown value"),
            Self::OutOfRange(field) => write!(f, "{field} is out of range"),
            Self::Io(kind) => write!(f, "pipe error: {kind:?}"),
        }
    }
}

impl std::error::Error for WireError {}

// ---------------------------------------------------------------------------
// Encoding

/// Encode `message` as one frame with sequence number `seq`.
///
/// Encoding checks the same caps as decoding, so the browser can't build a
/// frame the renderer would refuse, and the other way round.
pub fn encode(seq: u32, message: &Message) -> Result<Vec<u8>, WireError> {
    let mut body = Vec::new();
    match message {
        Message::Hello { role, version } => {
            body.push(*role as u8);
            body.extend_from_slice(&version.to_le_bytes());
        }
        Message::Ping { nonce } | Message::Pong { nonce } => {
            body.extend_from_slice(&nonce.to_le_bytes());
        }
        Message::Render(r) => {
            check_viewport(r.viewport_width, r.viewport_height)?;
            put_u64(&mut body, r.tab_id);
            put_u64(&mut body, r.nav_id);
            put_text(&mut body, &r.url, MAX_URL_BYTES, "url")?;
            put_text(&mut body, &r.html, MAX_DOCUMENT_BYTES, "html")?;
            match &r.csp_header {
                Some(csp) => {
                    body.push(1);
                    put_text(&mut body, csp, MAX_CSP_BYTES, "csp_header")?;
                }
                None => body.push(0),
            }
            body.push(u8::from(r.scripts_enabled));
            put_u32(&mut body, r.viewport_width);
            put_u32(&mut body, r.viewport_height);
            body.push(u8::from(r.https_only));
            if r.http_allowed_hosts.len() > MAX_ALLOWED_HOSTS {
                return Err(WireError::TooMany("http_allowed_hosts"));
            }
            body.extend_from_slice(&(r.http_allowed_hosts.len() as u16).to_le_bytes());
            for host in &r.http_allowed_hosts {
                check_host(host)?;
                put_text(&mut body, host, MAX_HOST_BYTES, "http_allowed_hosts")?;
            }
        }
        Message::Rendered(d) => {
            if d.dom_nodes > MAX_REPORTED_NODES {
                return Err(WireError::OutOfRange("dom_nodes"));
            }
            put_u64(&mut body, d.tab_id);
            put_u64(&mut body, d.nav_id);
            put_text(&mut body, &d.title, MAX_TITLE_BYTES, "title")?;
            put_u32(&mut body, d.dom_nodes);
            put_u32(&mut body, d.blocked_resources);
            put_text(&mut body, &d.html, MAX_DOCUMENT_BYTES, "html")?;
        }
        Message::Failed(e) => {
            put_u64(&mut body, e.tab_id);
            put_u64(&mut body, e.nav_id);
            body.push(e.code as u8);
            put_text(&mut body, &e.detail, MAX_SHORT_TEXT_BYTES, "detail")?;
        }
        Message::Fetch(f) => {
            check_fetch_url(&f.url)?;
            put_u64(&mut body, f.nav_id);
            body.push(f.kind as u8);
            put_text(&mut body, &f.url, MAX_URL_BYTES, "url")?;
        }
        Message::FetchReply(r) => {
            check_status(r.status)?;
            put_u64(&mut body, r.nav_id);
            body.extend_from_slice(&r.status.to_le_bytes());
            put_text(
                &mut body,
                &r.content_type,
                MAX_SHORT_TEXT_BYTES,
                "content_type",
            )?;
            put_bytes(&mut body, &r.body, MAX_FETCH_BODY_BYTES, "body")?;
        }
        Message::Shutdown => {}
    }
    let kind = message.kind();
    if body.len() > kind.max_body() {
        return Err(WireError::TooLarge {
            kind,
            len: body.len(),
        });
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + body.len());
    frame.extend_from_slice(&WIRE_MAGIC);
    frame.extend_from_slice(&WIRE_VERSION.to_le_bytes());
    frame.push(kind as u8);
    frame.push(0);
    frame.extend_from_slice(&seq.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Encode and write one frame, then flush.
pub fn write_message<W: Write>(
    writer: &mut W,
    seq: u32,
    message: &Message,
) -> Result<(), WireError> {
    let frame = encode(seq, message)?;
    writer
        .write_all(&frame)
        .and_then(|()| writer.flush())
        .map_err(|err| WireError::Io(err.kind()))
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_text(
    out: &mut Vec<u8>,
    text: &str,
    cap: usize,
    field: &'static str,
) -> Result<(), WireError> {
    put_bytes(out, text.as_bytes(), cap, field)
}

fn put_bytes(
    out: &mut Vec<u8>,
    bytes: &[u8],
    cap: usize,
    field: &'static str,
) -> Result<(), WireError> {
    if bytes.len() > cap {
        return Err(WireError::FieldTooLong(field));
    }
    put_u32(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
    Ok(())
}

// ---------------------------------------------------------------------------
// Decoding

/// The parts of a checked header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: MessageKind,
    pub seq: u32,
    pub body_len: usize,
}

/// Check a 16-byte header for `direction`.
///
/// This is where a huge or wrong frame is stopped, before its body is read
/// or any memory is set aside for it.
pub fn decode_header(
    bytes: &[u8; HEADER_BYTES],
    direction: Direction,
) -> Result<Header, WireError> {
    if bytes[0..4] != WIRE_MAGIC {
        return Err(WireError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != WIRE_VERSION {
        return Err(WireError::UnsupportedVersion(version));
    }
    let kind = MessageKind::from_byte(bytes[6]).ok_or(WireError::UnknownKind(bytes[6]))?;
    if bytes[7] != 0 {
        return Err(WireError::BadFlags(bytes[7]));
    }
    if !kind.allowed(direction) {
        return Err(WireError::WrongDirection(kind));
    }
    let seq = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let body_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if body_len > kind.max_body() {
        return Err(WireError::TooLarge {
            kind,
            len: body_len,
        });
    }
    Ok(Header {
        kind,
        seq,
        body_len,
    })
}

/// Decode one whole frame that travelled in `direction`. Returns the seq and
/// the message. The slice must be exactly one frame.
pub fn decode(frame: &[u8], direction: Direction) -> Result<(u32, Message), WireError> {
    let Some((head, body)) = frame.split_first_chunk::<HEADER_BYTES>() else {
        return Err(WireError::Truncated);
    };
    let header = decode_header(head, direction)?;
    if body.len() < header.body_len {
        return Err(WireError::Truncated);
    }
    if body.len() > header.body_len {
        return Err(WireError::TrailingBytes);
    }
    let message = decode_body(header.kind, body)?;
    Ok((header.seq, message))
}

/// Read exactly one frame from `reader`.
///
/// The header is checked first. The body buffer is only made after its length
/// passed the cap for its kind.
pub fn read_message<R: Read>(
    reader: &mut R,
    direction: Direction,
) -> Result<(u32, Message), WireError> {
    let mut head = [0u8; HEADER_BYTES];
    reader
        .read_exact(&mut head)
        .map_err(|err| WireError::Io(err.kind()))?;
    let header = decode_header(&head, direction)?;
    let mut body = vec![0u8; header.body_len];
    reader
        .read_exact(&mut body)
        .map_err(|err| match err.kind() {
            io::ErrorKind::UnexpectedEof => WireError::Truncated,
            kind => WireError::Io(kind),
        })?;
    let message = decode_body(header.kind, &body)?;
    Ok((header.seq, message))
}

fn decode_body(kind: MessageKind, body: &[u8]) -> Result<Message, WireError> {
    let mut r = Cursor { bytes: body, at: 0 };
    let message = match kind {
        MessageKind::Hello => {
            let role = match r.u8()? {
                1 => Role::Browser,
                2 => Role::Renderer,
                _ => return Err(WireError::BadEnum("role")),
            };
            let version = r.u16()?;
            Message::Hello { role, version }
        }
        MessageKind::Ping => Message::Ping { nonce: r.u64()? },
        MessageKind::Pong => Message::Pong { nonce: r.u64()? },
        MessageKind::Render => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let url = r.text(MAX_URL_BYTES, "url")?;
            let html = r.text(MAX_DOCUMENT_BYTES, "html")?;
            let csp_header = if r.bool("csp_header")? {
                Some(r.text(MAX_CSP_BYTES, "csp_header")?)
            } else {
                None
            };
            let scripts_enabled = r.bool("scripts_enabled")?;
            let viewport_width = r.u32()?;
            let viewport_height = r.u32()?;
            check_viewport(viewport_width, viewport_height)?;
            let https_only = r.bool("https_only")?;
            let count = r.u16()? as usize;
            if count > MAX_ALLOWED_HOSTS {
                return Err(WireError::TooMany("http_allowed_hosts"));
            }
            let mut http_allowed_hosts = Vec::with_capacity(count);
            for _ in 0..count {
                let host = r.text(MAX_HOST_BYTES, "http_allowed_hosts")?;
                check_host(&host)?;
                http_allowed_hosts.push(host);
            }
            Message::Render(RenderRequest {
                tab_id,
                nav_id,
                url,
                html,
                csp_header,
                scripts_enabled,
                viewport_width,
                viewport_height,
                https_only,
                http_allowed_hosts,
            })
        }
        MessageKind::Rendered => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let title = r.text(MAX_TITLE_BYTES, "title")?;
            let dom_nodes = r.u32()?;
            if dom_nodes > MAX_REPORTED_NODES {
                return Err(WireError::OutOfRange("dom_nodes"));
            }
            let blocked_resources = r.u32()?;
            let html = r.text(MAX_DOCUMENT_BYTES, "html")?;
            Message::Rendered(RenderedDocument {
                tab_id,
                nav_id,
                title,
                dom_nodes,
                blocked_resources,
                html,
            })
        }
        MessageKind::Failed => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let code = match r.u8()? {
                1 => FailureCode::Limit,
                2 => FailureCode::TooLarge,
                3 => FailureCode::Internal,
                _ => return Err(WireError::BadEnum("code")),
            };
            let detail = r.text(MAX_SHORT_TEXT_BYTES, "detail")?;
            Message::Failed(RenderFailure {
                tab_id,
                nav_id,
                code,
                detail,
            })
        }
        MessageKind::Fetch => {
            let nav_id = r.u64()?;
            let kind = match r.u8()? {
                1 => FetchKind::Stylesheet,
                2 => FetchKind::Script,
                3 => FetchKind::Image,
                4 => FetchKind::Font,
                _ => return Err(WireError::BadEnum("kind")),
            };
            let url = r.text(MAX_URL_BYTES, "url")?;
            check_fetch_url(&url)?;
            Message::Fetch(FetchRequest { nav_id, kind, url })
        }
        MessageKind::FetchReply => {
            let nav_id = r.u64()?;
            let status = r.u16()?;
            check_status(status)?;
            let content_type = r.text(MAX_SHORT_TEXT_BYTES, "content_type")?;
            let body = r.bytes(MAX_FETCH_BODY_BYTES, "body")?.to_vec();
            Message::FetchReply(FetchReply {
                nav_id,
                status,
                content_type,
                body,
            })
        }
        MessageKind::Shutdown => Message::Shutdown,
    };
    if r.at != body.len() {
        return Err(WireError::TrailingBytes);
    }
    Ok(message)
}

/// Reads fields from a body. Every read checks what is left first.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(n).ok_or(WireError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(WireError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(b);
        Ok(u64::from_le_bytes(raw))
    }

    fn bool(&mut self, field: &'static str) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::BadBool(field)),
        }
    }

    fn bytes(&mut self, cap: usize, field: &'static str) -> Result<&'a [u8], WireError> {
        let len = self.u32()? as usize;
        if len > cap {
            return Err(WireError::FieldTooLong(field));
        }
        self.take(len)
    }

    fn text(&mut self, cap: usize, field: &'static str) -> Result<String, WireError> {
        let raw = self.bytes(cap, field)?;
        std::str::from_utf8(raw)
            .map(str::to_string)
            .map_err(|_| WireError::BadUtf8(field))
    }
}

// ---------------------------------------------------------------------------
// Field rules shared by both sides

fn check_viewport(width: u32, height: u32) -> Result<(), WireError> {
    if !(1..=MAX_VIEWPORT_SIDE).contains(&width) {
        return Err(WireError::OutOfRange("viewport_width"));
    }
    if !(1..=MAX_VIEWPORT_SIDE).contains(&height) {
        return Err(WireError::OutOfRange("viewport_height"));
    }
    Ok(())
}

/// A host is lowercase letters, digits, `-` and `.`, and not empty.
fn check_host(host: &str) -> Result<(), WireError> {
    let ok = !host.is_empty()
        && host.len() <= MAX_HOST_BYTES
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(WireError::OutOfRange("http_allowed_hosts"))
    }
}

/// A renderer may only ask for `http://` or `https://` URLs made of visible
/// ASCII. Local files, built-in pages and anything with spaces or control
/// bytes are refused here, before the browser looks at the URL at all.
fn check_fetch_url(url: &str) -> Result<(), WireError> {
    let scheme_ok = url.starts_with("http://") || url.starts_with("https://");
    let bytes_ok = url.bytes().all(|b| (0x21..=0x7e).contains(&b));
    if scheme_ok && bytes_ok && url.len() <= MAX_URL_BYTES {
        Ok(())
    } else {
        Err(WireError::OutOfRange("url"))
    }
}

/// HTTP status, or 0 for a fetch that was blocked or failed.
fn check_status(status: u16) -> Result<(), WireError> {
    if status == 0 || (100..=599).contains(&status) {
        Ok(())
    } else {
        Err(WireError::OutOfRange("status"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render() -> RenderRequest {
        RenderRequest {
            tab_id: 3,
            nav_id: 9,
            url: "https://example.test/page".to_string(),
            html: "<p>hello</p>".to_string(),
            csp_header: Some("default-src 'self'".to_string()),
            scripts_enabled: true,
            viewport_width: 960,
            viewport_height: 720,
            https_only: true,
            http_allowed_hosts: vec!["intranet.test".to_string()],
        }
    }

    #[test]
    fn every_message_round_trips() {
        let messages = [
            (
                Direction::ToRenderer,
                Message::Hello {
                    role: Role::Browser,
                    version: WIRE_VERSION,
                },
            ),
            (
                Direction::ToBrowser,
                Message::Hello {
                    role: Role::Renderer,
                    version: WIRE_VERSION,
                },
            ),
            (Direction::ToRenderer, Message::Ping { nonce: 77 }),
            (Direction::ToBrowser, Message::Pong { nonce: 77 }),
            (Direction::ToRenderer, Message::Render(render())),
            (
                Direction::ToBrowser,
                Message::Rendered(RenderedDocument {
                    tab_id: 3,
                    nav_id: 9,
                    title: "Hello".to_string(),
                    dom_nodes: 12,
                    blocked_resources: 1,
                    html: "<p>hello</p>".to_string(),
                }),
            ),
            (
                Direction::ToBrowser,
                Message::Failed(RenderFailure {
                    tab_id: 3,
                    nav_id: 9,
                    code: FailureCode::Limit,
                    detail: "script budget".to_string(),
                }),
            ),
            (
                Direction::ToBrowser,
                Message::Fetch(FetchRequest {
                    nav_id: 9,
                    kind: FetchKind::Stylesheet,
                    url: "https://example.test/site.css".to_string(),
                }),
            ),
            (
                Direction::ToRenderer,
                Message::FetchReply(FetchReply {
                    nav_id: 9,
                    status: 200,
                    content_type: "text/css".to_string(),
                    body: b"p{color:red}".to_vec(),
                }),
            ),
            (Direction::ToRenderer, Message::Shutdown),
        ];
        for (seq, (direction, message)) in messages.iter().enumerate() {
            let frame = encode(seq as u32, message).unwrap();
            let (back_seq, back) = decode(&frame, *direction).unwrap();
            assert_eq!(back_seq, seq as u32);
            assert_eq!(&back, message);
            let mut reader = io::Cursor::new(frame.clone());
            assert_eq!(read_message(&mut reader, *direction).unwrap().1, back);
        }
    }

    #[test]
    fn a_renderer_cannot_send_browser_messages() {
        let frame = encode(1, &Message::Render(render())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToBrowser),
            Err(WireError::WrongDirection(MessageKind::Render))
        );
        let reply = FetchReply {
            nav_id: 1,
            status: 200,
            content_type: String::new(),
            body: Vec::new(),
        };
        let frame = encode(1, &Message::FetchReply(reply)).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToBrowser),
            Err(WireError::WrongDirection(MessageKind::FetchReply))
        );
    }

    #[test]
    fn header_rules_are_strict() {
        let good = encode(5, &Message::Ping { nonce: 1 }).unwrap();

        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::BadMagic)
        );

        let mut bad = good.clone();
        bad[4] = 2;
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::UnsupportedVersion(2))
        );

        let mut bad = good.clone();
        bad[6] = 200;
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::UnknownKind(200))
        );

        let mut bad = good.clone();
        bad[7] = 1;
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::BadFlags(1))
        );

        let mut bad = good.clone();
        bad.push(0);
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::TrailingBytes)
        );

        let bad = &good[..good.len() - 1];
        assert_eq!(
            decode(bad, Direction::ToRenderer),
            Err(WireError::Truncated)
        );
    }

    #[test]
    fn a_huge_length_is_refused_before_reading_the_body() {
        let mut frame = encode(
            1,
            &Message::Rendered(RenderedDocument {
                tab_id: 1,
                nav_id: 1,
                title: String::new(),
                dom_nodes: 0,
                blocked_resources: 0,
                html: String::new(),
            }),
        )
        .unwrap();
        frame[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        // Only the header is there. A reader that trusted the length would
        // try to set aside 4 GiB, or wait for bytes that never come.
        let mut reader = io::Cursor::new(frame[..HEADER_BYTES].to_vec());
        assert!(matches!(
            read_message(&mut reader, Direction::ToBrowser),
            Err(WireError::TooLarge { .. })
        ));

        // A small kind can't claim a big body either.
        let mut ping = encode(1, &Message::Pong { nonce: 1 }).unwrap();
        ping[12..16].copy_from_slice(&9u32.to_le_bytes());
        ping.push(0);
        assert_eq!(
            decode(&ping, Direction::ToBrowser),
            Err(WireError::TooLarge {
                kind: MessageKind::Pong,
                len: 9
            })
        );
    }

    #[test]
    fn field_rules_are_strict() {
        let mut r = render();
        r.viewport_width = 0;
        assert_eq!(
            encode(1, &Message::Render(r)),
            Err(WireError::OutOfRange("viewport_width"))
        );

        let mut r = render();
        r.http_allowed_hosts = vec!["Bad Host".to_string()];
        assert!(encode(1, &Message::Render(r)).is_err());

        let mut r = render();
        r.url = "x".repeat(MAX_URL_BYTES + 1);
        assert_eq!(
            encode(1, &Message::Render(r)),
            Err(WireError::FieldTooLong("url"))
        );

        for url in [
            "file:///etc/passwd",
            "anvil://settings",
            "https://example.test/a b",
            "https://example.test/\n",
            "",
        ] {
            let fetch = Message::Fetch(FetchRequest {
                nav_id: 1,
                kind: FetchKind::Image,
                url: url.to_string(),
            });
            assert_eq!(
                encode(1, &fetch),
                Err(WireError::OutOfRange("url")),
                "{url}"
            );
        }

        // A bool byte of 2 in the scripts flag.
        let frame = encode(1, &Message::Render(render())).unwrap();
        let mut bad = frame.clone();
        let flag_at = HEADER_BYTES + 8 + 8 + 4 + 25 + 4 + 12 + 1 + 4 + 18;
        assert_eq!(bad[flag_at], 1);
        bad[flag_at] = 2;
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::BadBool("scripts_enabled"))
        );

        // Text that isn't UTF-8.
        let mut bad = frame;
        let html_at = HEADER_BYTES + 8 + 8 + 4 + 25 + 4;
        bad[html_at] = 0xff;
        assert_eq!(
            decode(&bad, Direction::ToRenderer),
            Err(WireError::BadUtf8("html"))
        );
    }

    #[test]
    fn the_largest_document_fits() {
        let mut r = render();
        r.html = "a".repeat(MAX_DOCUMENT_BYTES);
        r.url = "u".repeat(MAX_URL_BYTES);
        r.csp_header = Some("c".repeat(MAX_CSP_BYTES));
        r.http_allowed_hosts = vec!["h".repeat(MAX_HOST_BYTES); MAX_ALLOWED_HOSTS];
        let frame = encode(1, &Message::Render(r.clone())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToRenderer).unwrap().1,
            Message::Render(r)
        );
    }
}
