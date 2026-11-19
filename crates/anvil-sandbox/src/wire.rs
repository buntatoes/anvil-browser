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
///
/// Version 2 added `Frame` and `Resize`. Version 3 added live tab
/// `Interact`/`InteractResult` and rich `ScriptFetch`/`ScriptFetchReply`.
/// Version 4 adds sandboxed `WsCommand`/`WsReply` IPC, post-load timer and SPA
/// history metadata on `RenderedDocument`, and extended interactive actions
/// (`HistoryGo`, `ViewportScroll`, `InputAction`, `ResizeEvent`).
/// The window and the renderer ship in the same binary, so this only stops
/// a mismatched pair.
pub const WIRE_VERSION: u16 = 4;

/// Size of the frame header.
pub const HEADER_BYTES: usize = 16;

/// Largest document a request or a reply may carry (16 MiB).
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Largest fetched body the browser hands to a renderer (16 MiB).
pub const MAX_FETCH_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Largest URL, in bytes.
pub const MAX_URL_BYTES: usize = 8 * 1024;

/// Most fetches a renderer may ask for while it works on one page.
pub const MAX_FETCHES_PER_RENDER: usize = 512;

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

/// Longest `uid_map` line in a [`RendererStatus`], in bytes.
pub const MAX_UID_MAP_BYTES: usize = 256;

/// The namespaces a [`RendererStatus`] reports, in its order.
pub const NAMESPACES: [&str; 7] = ["pid", "user", "net", "ipc", "uts", "mnt", "cgroup"];

/// The soft limits a [`RendererStatus`] reports, in its order, as they are
/// named in `/proc/self/limits`.
pub const LIMITS: [&str; 5] = [
    "Max address space",
    "Max core file size",
    "Max open files",
    "Max file size",
    "Max processes",
];

/// Most headers carried in a `ScriptFetch` or `ScriptFetchReply`.
pub const MAX_FETCH_HEADERS: usize = 64;

/// Longest HTTP method token in `ScriptFetch`.
pub const MAX_METHOD_BYTES: usize = 32;

/// Longest interactive text or clipboard payload carried across `Interact` / `InteractResult`.
pub const MAX_INTERACT_TEXT_BYTES: usize = 64 * 1024;

/// Most subprotocol tokens carried in a `WsCommandOp::Connect`.
pub const MAX_WS_PROTOCOLS: usize = 16;

/// Most queued WebSocket events returned in a single `WsReplyBody::Polled`.
pub const MAX_WS_EVENTS: usize = 256;

/// Largest single WebSocket text or binary message payload carried over wire IPC (4 MiB).
pub const MAX_WS_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Largest body of a message that carries a document or a fetched body.
/// This is the document cap plus room for the other fields.
pub const MAX_BODY_BYTES: usize = MAX_DOCUMENT_BYTES + 128 * 1024;

/// Largest encoded display list a [`RenderedFrame`] may carry (32 MiB).
///
/// The wire only carries these bytes. The browser decodes and checks them
/// with its own frame checker before it draws anything.
pub const MAX_DISPLAY_LIST_BYTES: usize = 32 * 1024 * 1024;

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
    StatusQuery = 10,
    Status = 11,
    Frame = 12,
    Resize = 13,
    Interact = 14,
    InteractResult = 15,
    ScriptFetch = 16,
    ScriptFetchReply = 17,
    WsCommand = 18,
    WsReply = 19,
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
            10 => Self::StatusQuery,
            11 => Self::Status,
            12 => Self::Frame,
            13 => Self::Resize,
            14 => Self::Interact,
            15 => Self::InteractResult,
            16 => Self::ScriptFetch,
            17 => Self::ScriptFetchReply,
            18 => Self::WsCommand,
            19 => Self::WsReply,
            _ => return None,
        })
    }

    /// True when this kind may travel in `direction`.
    pub fn allowed(self, direction: Direction) -> bool {
        match direction {
            Direction::ToRenderer => matches!(
                self,
                Self::Hello
                    | Self::Ping
                    | Self::Render
                    | Self::FetchReply
                    | Self::Shutdown
                    | Self::StatusQuery
                    | Self::Resize
                    | Self::Interact
                    | Self::ScriptFetchReply
                    | Self::WsReply
            ),
            Direction::ToBrowser => matches!(
                self,
                Self::Hello
                    | Self::Pong
                    | Self::Rendered
                    | Self::Failed
                    | Self::Fetch
                    | Self::Status
                    | Self::Frame
                    | Self::InteractResult
                    | Self::ScriptFetch
                    | Self::WsCommand
            ),
        }
    }

    /// Largest body this kind can have. The header is refused above this,
    /// before the body is read.
    pub fn max_body(self) -> usize {
        match self {
            Self::Hello => 3,
            Self::Ping | Self::Pong => 8,
            Self::Shutdown | Self::StatusQuery => 0,
            Self::Status => {
                1 + 1
                    + 8 * NAMESPACES.len()
                    + 4
                    + MAX_UID_MAP_BYTES
                    + 4
                    + 4
                    + 4
                    + MAX_SHORT_TEXT_BYTES
                    + 4
                    + MAX_SHORT_TEXT_BYTES
                    + 9 * LIMITS.len()
            }
            Self::Fetch => 8 + 1 + 4 + MAX_URL_BYTES,
            Self::Failed => 8 + 8 + 1 + 4 + MAX_SHORT_TEXT_BYTES,
            Self::Render
            | Self::Rendered
            | Self::FetchReply
            | Self::InteractResult
            | Self::ScriptFetch
            | Self::ScriptFetchReply
            | Self::WsCommand
            | Self::WsReply => MAX_BODY_BYTES,
            Self::Resize => 8 + 8 + 4 + 4,
            Self::Interact => 8 + 8 + 1 + 4 + 4 + 4 + 8 + 4 + 4 + 1 + 4 + MAX_INTERACT_TEXT_BYTES,
            Self::Frame => 8 + 8 + 4 + MAX_TITLE_BYTES + 4 + 4 + 4 + MAX_DISPLAY_LIST_BYTES,
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
///
/// The browser picks its rules by kind. `ScriptFetch` is a page script
/// calling `fetch()`: it must stay on the page's origin, and it is the only
/// kind that sends the page's cookies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchKind {
    Stylesheet = 1,
    Script = 2,
    Image = 3,
    Font = 4,
    ScriptFetch = 5,
    CorsFetch = 6,
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
    pub extended_script_reach: bool,
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
    /// Effective document URL after initial script `history.pushState`/`replaceState`, or empty if unchanged.
    pub final_url: String,
    /// True when initial scripts scheduled a live animation frame, timer, or open WebSocket/SSE stream.
    pub animating: bool,
    /// True when `next_timer_ms_bits` carries a scheduled JS timer due timestamp.
    pub has_next_timer: bool,
    /// `f64::to_bits()` virtual timestamp of the soonest pending timer (`0` when `!has_next_timer`).
    pub next_timer_ms_bits: u64,
    /// Number of SPA history entries behind the active entry (`history.pushState`).
    pub spa_back_depth: u32,
    /// Number of SPA history entries ahead of the active entry.
    pub spa_forward_depth: u32,
}

/// A renderer's painted page: the page's display list, encoded by the
/// renderer and checked by the browser before it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedFrame {
    pub tab_id: u64,
    pub nav_id: u64,
    pub title: String,
    pub dom_nodes: u32,
    pub blocked_resources: u32,
    /// At most [`MAX_DISPLAY_LIST_BYTES`]. Opaque to the wire.
    pub display_list: Vec<u8>,
}

/// The browser's window changed size, so the page needs painting again at
/// the new viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeRequest {
    pub tab_id: u64,
    pub nav_id: u64,
    pub viewport_width: u32,
    pub viewport_height: u32,
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

/// What a renderer read about its own process from the OS, for the
/// browser's security page.
///
/// A renderer that has been taken over could send anything here, so the
/// browser only asks a fresh renderer, before it has seen any page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RendererStatus {
    /// `NoNewPrivs:` from `/proc/self/status`. `None` where it can't be read.
    pub no_new_privs: Option<bool>,
    /// `Seccomp:` mode from `/proc/self/status`, 0 to 2.
    pub seccomp: Option<u8>,
    /// The id of each of [`NAMESPACES`] from `/proc/self/ns`, 0 where it
    /// can't be read.
    pub namespaces: [u64; 7],
    /// The first line of `/proc/self/uid_map`, spaces squeezed, for example
    /// `0 1000 1`.
    pub uid_map: String,
    /// Its own process id, as it sees it.
    pub pid: u32,
    /// How many processes it can see in `/proc`.
    pub visible_processes: u32,
    /// What it sees at the top of its filesystem: the names in `/`, sorted
    /// and separated by spaces. Cut short to fit.
    pub root_entries: String,
    /// Every mount it can write to, from `/proc/self/mountinfo`, as
    /// `<mount point>:<filesystem type>` separated by spaces, `none` if
    /// there is none, and empty where it can't be read. Mount points keep
    /// the kernel's octal escapes, so they never hold a space. A list cut
    /// short to fit ends in ` ...`.
    pub writable_mounts: String,
    /// The soft value of each of [`LIMITS`]. `None` where it can't be read,
    /// `u64::MAX` for unlimited.
    pub limits: [Option<u64>; 5],
}

/// An interactive event sent to a persistent tab renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractOp {
    Click = 1,
    Scroll = 2,
    InputChar = 3,
    KeyDown = 4,
    TimerStep = 5,
    FocusBlur = 6,
    KeyText = 7,
    HistoryGo = 8,
    ViewportScroll = 9,
    InputAction = 10,
    ResizeEvent = 11,
}

impl InteractOp {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Self::Click,
            2 => Self::Scroll,
            3 => Self::InputChar,
            4 => Self::KeyDown,
            5 => Self::TimerStep,
            6 => Self::FocusBlur,
            7 => Self::KeyText,
            8 => Self::HistoryGo,
            9 => Self::ViewportScroll,
            10 => Self::InputAction,
            11 => Self::ResizeEvent,
            _ => return None,
        })
    }
}

/// Interactive input, scroll, or timer request delivered to a live tab renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractRequest {
    pub tab_id: u64,
    pub nav_id: u64,
    pub op: InteractOp,
    pub x_bits: u32,
    pub y_bits: u32,
    pub delta_bits: u32,
    pub now_ms_bits: u64,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub modifiers: u8,
    pub text: String,
}

/// Outcome of an [`InteractRequest`] processed inside the persistent renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InteractOutcome {
    pub tab_id: u64,
    pub nav_id: u64,
    pub changed: bool,
    pub handled_by_script: bool,
    pub animating: bool,
    pub focus_input: bool,
    pub title: String,
    pub navigate_url: Option<String>,
    pub post_url: Option<String>,
    pub post_content_type: String,
    pub post_body: Vec<u8>,
    pub scroll_to_bits: u32,
    pub has_scroll_to: bool,
    pub next_timer_ms_bits: u64,
    pub has_next_timer: bool,
    /// Current document URL after SPA `history.pushState`/`replaceState`/`go`, or empty if unchanged.
    pub current_url: String,
    /// Number of SPA history entries behind the current entry.
    pub spa_back_depth: u32,
    /// Number of SPA history entries ahead of the current entry.
    pub spa_forward_depth: u32,
    /// Clipboard text produced by a copy/cut `InputAction`, if any.
    pub copied_text: Option<String>,
}

/// A full HTTP script `fetch()` / `XMLHttpRequest` request from the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFetchWireRequest {
    pub nav_id: u64,
    pub method: String,
    pub url: String,
    pub credentials_include: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The browser's answer to a [`ScriptFetchWireRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFetchWireReply {
    pub nav_id: u64,
    pub status: u16,
    pub final_url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Operation requested by the renderer over the sandboxed WebSocket IPC bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsCommandOp {
    Connect {
        url: String,
        protocols: Vec<String>,
    },
    SendText {
        handle: u64,
        text: String,
    },
    SendBinary {
        handle: u64,
        data: Vec<u8>,
    },
    Close {
        handle: u64,
        code: u16,
        reason: String,
    },
    Poll {
        handle: u64,
    },
}

/// A sandboxed WebSocket command sent from renderer to browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsWireCommand {
    pub nav_id: u64,
    pub op: WsCommandOp,
}

/// Single WebSocket event returned from the browser process to the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsWireEvent {
    Text(String),
    Binary(Vec<u8>),
    Closed { code: u16, reason: String },
    Error(String),
}

/// Payload of a [`WsWireReply`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsReplyBody {
    Connected {
        ok: bool,
        handle: u64,
        protocol: String,
        resolved_url: String,
        error: String,
    },
    Ack {
        ok: bool,
        error: String,
    },
    Polled {
        ready_state: u8,
        events: Vec<WsWireEvent>,
    },
}

/// Browser response to a [`WsWireCommand`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsWireReply {
    pub nav_id: u64,
    pub body: WsReplyBody,
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
    /// The browser asks what the OS has put the renderer under.
    StatusQuery,
    Status(RendererStatus),
    /// A painted page, in place of `Rendered`'s HTML.
    Frame(RenderedFrame),
    /// The browser asks for the page again at a new viewport.
    Resize(ResizeRequest),
    /// Browser dispatches interactive UI/timer work to the live tab renderer.
    Interact(InteractRequest),
    /// Live tab renderer outcome after processing an [`InteractRequest`].
    InteractResult(InteractOutcome),
    /// Rich HTTP script fetch request (`fetch()` / `XMLHttpRequest`) to browser.
    ScriptFetch(ScriptFetchWireRequest),
    /// Rich HTTP script fetch reply returned from the browser.
    ScriptFetchReply(ScriptFetchWireReply),
    /// Sandboxed WebSocket command sent from renderer to browser.
    WsCommand(WsWireCommand),
    /// Sandboxed WebSocket reply returned from browser to renderer.
    WsReply(WsWireReply),
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
            Self::StatusQuery => MessageKind::StatusQuery,
            Self::Status(_) => MessageKind::Status,
            Self::Frame(_) => MessageKind::Frame,
            Self::Resize(_) => MessageKind::Resize,
            Self::Interact(_) => MessageKind::Interact,
            Self::InteractResult(_) => MessageKind::InteractResult,
            Self::ScriptFetch(_) => MessageKind::ScriptFetch,
            Self::ScriptFetchReply(_) => MessageKind::ScriptFetchReply,
            Self::WsCommand(_) => MessageKind::WsCommand,
            Self::WsReply(_) => MessageKind::WsReply,
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
            body.push(u8::from(r.extended_script_reach));
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
            if !d.has_next_timer && d.next_timer_ms_bits != 0 {
                return Err(WireError::OutOfRange("next_timer_ms_bits"));
            }
            put_u64(&mut body, d.tab_id);
            put_u64(&mut body, d.nav_id);
            put_text(&mut body, &d.title, MAX_TITLE_BYTES, "title")?;
            put_u32(&mut body, d.dom_nodes);
            put_u32(&mut body, d.blocked_resources);
            put_text(&mut body, &d.html, MAX_DOCUMENT_BYTES, "html")?;
            put_text(&mut body, &d.final_url, MAX_URL_BYTES, "final_url")?;
            body.push(u8::from(d.animating));
            body.push(u8::from(d.has_next_timer));
            put_u64(
                &mut body,
                if d.has_next_timer {
                    d.next_timer_ms_bits
                } else {
                    0
                },
            );
            put_u32(&mut body, d.spa_back_depth);
            put_u32(&mut body, d.spa_forward_depth);
        }
        Message::Frame(f) => {
            if f.dom_nodes > MAX_REPORTED_NODES {
                return Err(WireError::OutOfRange("dom_nodes"));
            }
            put_u64(&mut body, f.tab_id);
            put_u64(&mut body, f.nav_id);
            put_text(&mut body, &f.title, MAX_TITLE_BYTES, "title")?;
            put_u32(&mut body, f.dom_nodes);
            put_u32(&mut body, f.blocked_resources);
            put_bytes(
                &mut body,
                &f.display_list,
                MAX_DISPLAY_LIST_BYTES,
                "display_list",
            )?;
        }
        Message::Resize(r) => {
            check_viewport(r.viewport_width, r.viewport_height)?;
            put_u64(&mut body, r.tab_id);
            put_u64(&mut body, r.nav_id);
            put_u32(&mut body, r.viewport_width);
            put_u32(&mut body, r.viewport_height);
        }
        Message::Interact(r) => {
            check_viewport(r.viewport_width, r.viewport_height)?;
            put_u64(&mut body, r.tab_id);
            put_u64(&mut body, r.nav_id);
            body.push(r.op as u8);
            put_u32(&mut body, r.x_bits);
            put_u32(&mut body, r.y_bits);
            put_u32(&mut body, r.delta_bits);
            put_u64(&mut body, r.now_ms_bits);
            put_u32(&mut body, r.viewport_width);
            put_u32(&mut body, r.viewport_height);
            body.push(r.modifiers);
            put_text(&mut body, &r.text, MAX_INTERACT_TEXT_BYTES, "text")?;
        }
        Message::InteractResult(r) => {
            if !r.has_scroll_to && r.scroll_to_bits != 0 {
                return Err(WireError::OutOfRange("scroll_to_bits"));
            }
            if !r.has_next_timer && r.next_timer_ms_bits != 0 {
                return Err(WireError::OutOfRange("next_timer_ms_bits"));
            }
            put_u64(&mut body, r.tab_id);
            put_u64(&mut body, r.nav_id);
            body.push(u8::from(r.changed));
            body.push(u8::from(r.handled_by_script));
            body.push(u8::from(r.animating));
            body.push(u8::from(r.focus_input));
            put_text(&mut body, &r.title, MAX_TITLE_BYTES, "title")?;
            match &r.navigate_url {
                Some(url) => {
                    body.push(1);
                    put_text(&mut body, url, MAX_URL_BYTES, "navigate_url")?;
                }
                None => body.push(0),
            }
            match &r.post_url {
                Some(url) => {
                    body.push(1);
                    put_text(&mut body, url, MAX_URL_BYTES, "post_url")?;
                }
                None => body.push(0),
            }
            put_text(
                &mut body,
                &r.post_content_type,
                MAX_SHORT_TEXT_BYTES,
                "post_content_type",
            )?;
            put_bytes(&mut body, &r.post_body, MAX_FETCH_BODY_BYTES, "post_body")?;
            body.push(u8::from(r.has_scroll_to));
            put_u32(
                &mut body,
                if r.has_scroll_to { r.scroll_to_bits } else { 0 },
            );
            body.push(u8::from(r.has_next_timer));
            put_u64(
                &mut body,
                if r.has_next_timer {
                    r.next_timer_ms_bits
                } else {
                    0
                },
            );
            put_text(&mut body, &r.current_url, MAX_URL_BYTES, "current_url")?;
            put_u32(&mut body, r.spa_back_depth);
            put_u32(&mut body, r.spa_forward_depth);
            match &r.copied_text {
                Some(text) => {
                    body.push(1);
                    put_text(&mut body, text, MAX_INTERACT_TEXT_BYTES, "copied_text")?;
                }
                None => body.push(0),
            }
        }
        Message::ScriptFetch(f) => {
            check_http_method(&f.method)?;
            check_fetch_url(&f.url)?;
            if f.headers.len() > MAX_FETCH_HEADERS {
                return Err(WireError::TooMany("headers"));
            }
            put_u64(&mut body, f.nav_id);
            put_text(&mut body, &f.method, MAX_METHOD_BYTES, "method")?;
            put_text(&mut body, &f.url, MAX_URL_BYTES, "url")?;
            body.push(u8::from(f.credentials_include));
            body.extend_from_slice(&(f.headers.len() as u16).to_le_bytes());
            for (name, value) in &f.headers {
                put_text(&mut body, name, MAX_SHORT_TEXT_BYTES, "header_name")?;
                put_text(&mut body, value, MAX_SHORT_TEXT_BYTES, "header_value")?;
            }
            put_bytes(&mut body, &f.body, MAX_FETCH_BODY_BYTES, "body")?;
        }
        Message::ScriptFetchReply(r) => {
            check_status(r.status)?;
            if !r.final_url.is_empty() {
                check_fetch_url(&r.final_url)?;
            }
            if r.headers.len() > MAX_FETCH_HEADERS {
                return Err(WireError::TooMany("headers"));
            }
            put_u64(&mut body, r.nav_id);
            body.extend_from_slice(&r.status.to_le_bytes());
            put_text(&mut body, &r.final_url, MAX_URL_BYTES, "final_url")?;
            body.extend_from_slice(&(r.headers.len() as u16).to_le_bytes());
            for (name, value) in &r.headers {
                put_text(&mut body, name, MAX_SHORT_TEXT_BYTES, "header_name")?;
                put_text(&mut body, value, MAX_SHORT_TEXT_BYTES, "header_value")?;
            }
            put_bytes(&mut body, &r.body, MAX_FETCH_BODY_BYTES, "body")?;
        }
        Message::WsCommand(cmd) => {
            put_u64(&mut body, cmd.nav_id);
            match &cmd.op {
                WsCommandOp::Connect { url, protocols } => {
                    check_ws_url(url)?;
                    if protocols.len() > MAX_WS_PROTOCOLS {
                        return Err(WireError::TooMany("protocols"));
                    }
                    body.push(1);
                    put_text(&mut body, url, MAX_URL_BYTES, "url")?;
                    body.extend_from_slice(&(protocols.len() as u16).to_le_bytes());
                    for proto in protocols {
                        put_text(&mut body, proto, MAX_SHORT_TEXT_BYTES, "protocol")?;
                    }
                }
                WsCommandOp::SendText { handle, text } => {
                    body.push(2);
                    put_u64(&mut body, *handle);
                    put_text(&mut body, text, MAX_WS_PAYLOAD_BYTES, "ws_text")?;
                }
                WsCommandOp::SendBinary { handle, data } => {
                    body.push(3);
                    put_u64(&mut body, *handle);
                    put_bytes(&mut body, data, MAX_WS_PAYLOAD_BYTES, "ws_binary")?;
                }
                WsCommandOp::Close {
                    handle,
                    code,
                    reason,
                } => {
                    body.push(4);
                    put_u64(&mut body, *handle);
                    body.extend_from_slice(&code.to_le_bytes());
                    put_text(&mut body, reason, MAX_SHORT_TEXT_BYTES, "ws_reason")?;
                }
                WsCommandOp::Poll { handle } => {
                    body.push(5);
                    put_u64(&mut body, *handle);
                }
            }
        }
        Message::WsReply(reply) => {
            put_u64(&mut body, reply.nav_id);
            match &reply.body {
                WsReplyBody::Connected {
                    ok,
                    handle,
                    protocol,
                    resolved_url,
                    error,
                } => {
                    if !resolved_url.is_empty() {
                        check_ws_url(resolved_url)?;
                    }
                    body.push(1);
                    body.push(u8::from(*ok));
                    put_u64(&mut body, *handle);
                    put_text(&mut body, protocol, MAX_SHORT_TEXT_BYTES, "protocol")?;
                    put_text(&mut body, resolved_url, MAX_URL_BYTES, "resolved_url")?;
                    put_text(&mut body, error, MAX_SHORT_TEXT_BYTES, "error")?;
                }
                WsReplyBody::Ack { ok, error } => {
                    body.push(2);
                    body.push(u8::from(*ok));
                    put_text(&mut body, error, MAX_SHORT_TEXT_BYTES, "error")?;
                }
                WsReplyBody::Polled {
                    ready_state,
                    events,
                } => {
                    if *ready_state > 3 {
                        return Err(WireError::OutOfRange("ready_state"));
                    }
                    if events.len() > MAX_WS_EVENTS {
                        return Err(WireError::TooMany("ws_events"));
                    }
                    body.push(3);
                    body.push(*ready_state);
                    body.extend_from_slice(&(events.len() as u16).to_le_bytes());
                    for ev in events {
                        match ev {
                            WsWireEvent::Text(text) => {
                                body.push(1);
                                put_text(&mut body, text, MAX_WS_PAYLOAD_BYTES, "ws_text")?;
                            }
                            WsWireEvent::Binary(data) => {
                                body.push(2);
                                put_bytes(&mut body, data, MAX_WS_PAYLOAD_BYTES, "ws_binary")?;
                            }
                            WsWireEvent::Closed { code, reason } => {
                                body.push(3);
                                body.extend_from_slice(&code.to_le_bytes());
                                put_text(&mut body, reason, MAX_SHORT_TEXT_BYTES, "ws_reason")?;
                            }
                            WsWireEvent::Error(msg) => {
                                body.push(4);
                                put_text(&mut body, msg, MAX_SHORT_TEXT_BYTES, "ws_error")?;
                            }
                        }
                    }
                }
            }
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
        Message::Shutdown | Message::StatusQuery => {}
        Message::Status(st) => {
            body.push(match st.no_new_privs {
                Some(false) => 0,
                Some(true) => 1,
                None => 2,
            });
            match st.seccomp {
                Some(mode) if mode <= 2 => body.push(mode),
                Some(_) => return Err(WireError::OutOfRange("seccomp")),
                None => body.push(255),
            }
            for id in st.namespaces {
                put_u64(&mut body, id);
            }
            put_text(&mut body, &st.uid_map, MAX_UID_MAP_BYTES, "uid_map")?;
            put_u32(&mut body, st.pid);
            put_u32(&mut body, st.visible_processes);
            put_text(
                &mut body,
                &st.root_entries,
                MAX_SHORT_TEXT_BYTES,
                "root_entries",
            )?;
            put_text(
                &mut body,
                &st.writable_mounts,
                MAX_SHORT_TEXT_BYTES,
                "writable_mounts",
            )?;
            for limit in st.limits {
                match limit {
                    Some(value) => {
                        body.push(1);
                        put_u64(&mut body, value);
                    }
                    None => {
                        body.push(0);
                        put_u64(&mut body, 0);
                    }
                }
            }
        }
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
            let extended_script_reach = r.bool("extended_script_reach")?;
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
                extended_script_reach,
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
            let final_url = r.text(MAX_URL_BYTES, "final_url")?;
            let animating = r.bool("animating")?;
            let has_next_timer = r.bool("has_next_timer")?;
            let next_timer_ms_bits = r.u64()?;
            if !has_next_timer && next_timer_ms_bits != 0 {
                return Err(WireError::OutOfRange("next_timer_ms_bits"));
            }
            let spa_back_depth = r.u32()?;
            let spa_forward_depth = r.u32()?;
            Message::Rendered(RenderedDocument {
                tab_id,
                nav_id,
                title,
                dom_nodes,
                blocked_resources,
                html,
                final_url,
                animating,
                has_next_timer,
                next_timer_ms_bits,
                spa_back_depth,
                spa_forward_depth,
            })
        }
        MessageKind::Frame => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let title = r.text(MAX_TITLE_BYTES, "title")?;
            let dom_nodes = r.u32()?;
            if dom_nodes > MAX_REPORTED_NODES {
                return Err(WireError::OutOfRange("dom_nodes"));
            }
            let blocked_resources = r.u32()?;
            let display_list = r.bytes(MAX_DISPLAY_LIST_BYTES, "display_list")?.to_vec();
            Message::Frame(RenderedFrame {
                tab_id,
                nav_id,
                title,
                dom_nodes,
                blocked_resources,
                display_list,
            })
        }
        MessageKind::Resize => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let viewport_width = r.u32()?;
            let viewport_height = r.u32()?;
            check_viewport(viewport_width, viewport_height)?;
            Message::Resize(ResizeRequest {
                tab_id,
                nav_id,
                viewport_width,
                viewport_height,
            })
        }
        MessageKind::Interact => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let op = InteractOp::from_byte(r.u8()?).ok_or(WireError::BadEnum("op"))?;
            let x_bits = r.u32()?;
            let y_bits = r.u32()?;
            let delta_bits = r.u32()?;
            let now_ms_bits = r.u64()?;
            let viewport_width = r.u32()?;
            let viewport_height = r.u32()?;
            check_viewport(viewport_width, viewport_height)?;
            let modifiers = r.u8()?;
            let text = r.text(MAX_INTERACT_TEXT_BYTES, "text")?;
            Message::Interact(InteractRequest {
                tab_id,
                nav_id,
                op,
                x_bits,
                y_bits,
                delta_bits,
                now_ms_bits,
                viewport_width,
                viewport_height,
                modifiers,
                text,
            })
        }
        MessageKind::InteractResult => {
            let tab_id = r.u64()?;
            let nav_id = r.u64()?;
            let changed = r.bool("changed")?;
            let handled_by_script = r.bool("handled_by_script")?;
            let animating = r.bool("animating")?;
            let focus_input = r.bool("focus_input")?;
            let title = r.text(MAX_TITLE_BYTES, "title")?;
            let navigate_url = if r.bool("navigate_url")? {
                Some(r.text(MAX_URL_BYTES, "navigate_url")?)
            } else {
                None
            };
            let post_url = if r.bool("post_url")? {
                Some(r.text(MAX_URL_BYTES, "post_url")?)
            } else {
                None
            };
            let post_content_type = r.text(MAX_SHORT_TEXT_BYTES, "post_content_type")?;
            let post_body = r.bytes(MAX_FETCH_BODY_BYTES, "post_body")?.to_vec();
            let has_scroll_to = r.bool("has_scroll_to")?;
            let scroll_to_bits = r.u32()?;
            if !has_scroll_to && scroll_to_bits != 0 {
                return Err(WireError::OutOfRange("scroll_to_bits"));
            }
            let has_next_timer = r.bool("has_next_timer")?;
            let next_timer_ms_bits = r.u64()?;
            if !has_next_timer && next_timer_ms_bits != 0 {
                return Err(WireError::OutOfRange("next_timer_ms_bits"));
            }
            let current_url = r.text(MAX_URL_BYTES, "current_url")?;
            let spa_back_depth = r.u32()?;
            let spa_forward_depth = r.u32()?;
            let copied_text = if r.bool("copied_text")? {
                Some(r.text(MAX_INTERACT_TEXT_BYTES, "copied_text")?)
            } else {
                None
            };
            Message::InteractResult(InteractOutcome {
                tab_id,
                nav_id,
                changed,
                handled_by_script,
                animating,
                focus_input,
                title,
                navigate_url,
                post_url,
                post_content_type,
                post_body,
                scroll_to_bits,
                has_scroll_to,
                next_timer_ms_bits,
                has_next_timer,
                current_url,
                spa_back_depth,
                spa_forward_depth,
                copied_text,
            })
        }
        MessageKind::ScriptFetch => {
            let nav_id = r.u64()?;
            let method = r.text(MAX_METHOD_BYTES, "method")?;
            check_http_method(&method)?;
            let url = r.text(MAX_URL_BYTES, "url")?;
            check_fetch_url(&url)?;
            let credentials_include = r.bool("credentials_include")?;
            let header_count = r.u16()? as usize;
            if header_count > MAX_FETCH_HEADERS {
                return Err(WireError::TooMany("headers"));
            }
            let mut headers = Vec::with_capacity(header_count);
            for _ in 0..header_count {
                let name = r.text(MAX_SHORT_TEXT_BYTES, "header_name")?;
                let value = r.text(MAX_SHORT_TEXT_BYTES, "header_value")?;
                headers.push((name, value));
            }
            let body = r.bytes(MAX_FETCH_BODY_BYTES, "body")?.to_vec();
            Message::ScriptFetch(ScriptFetchWireRequest {
                nav_id,
                method,
                url,
                credentials_include,
                headers,
                body,
            })
        }
        MessageKind::ScriptFetchReply => {
            let nav_id = r.u64()?;
            let status = r.u16()?;
            check_status(status)?;
            let final_url = r.text(MAX_URL_BYTES, "final_url")?;
            if !final_url.is_empty() {
                check_fetch_url(&final_url)?;
            }
            let header_count = r.u16()? as usize;
            if header_count > MAX_FETCH_HEADERS {
                return Err(WireError::TooMany("headers"));
            }
            let mut headers = Vec::with_capacity(header_count);
            for _ in 0..header_count {
                let name = r.text(MAX_SHORT_TEXT_BYTES, "header_name")?;
                let value = r.text(MAX_SHORT_TEXT_BYTES, "header_value")?;
                headers.push((name, value));
            }
            let body = r.bytes(MAX_FETCH_BODY_BYTES, "body")?.to_vec();
            Message::ScriptFetchReply(ScriptFetchWireReply {
                nav_id,
                status,
                final_url,
                headers,
                body,
            })
        }
        MessageKind::WsCommand => {
            let nav_id = r.u64()?;
            let op = match r.u8()? {
                1 => {
                    let url = r.text(MAX_URL_BYTES, "url")?;
                    check_ws_url(&url)?;
                    let count = r.u16()? as usize;
                    if count > MAX_WS_PROTOCOLS {
                        return Err(WireError::TooMany("protocols"));
                    }
                    let mut protocols = Vec::with_capacity(count);
                    for _ in 0..count {
                        protocols.push(r.text(MAX_SHORT_TEXT_BYTES, "protocol")?);
                    }
                    WsCommandOp::Connect { url, protocols }
                }
                2 => {
                    let handle = r.u64()?;
                    let text = r.text(MAX_WS_PAYLOAD_BYTES, "ws_text")?;
                    WsCommandOp::SendText { handle, text }
                }
                3 => {
                    let handle = r.u64()?;
                    let data = r.bytes(MAX_WS_PAYLOAD_BYTES, "ws_binary")?.to_vec();
                    WsCommandOp::SendBinary { handle, data }
                }
                4 => {
                    let handle = r.u64()?;
                    let code = r.u16()?;
                    let reason = r.text(MAX_SHORT_TEXT_BYTES, "ws_reason")?;
                    WsCommandOp::Close {
                        handle,
                        code,
                        reason,
                    }
                }
                5 => {
                    let handle = r.u64()?;
                    WsCommandOp::Poll { handle }
                }
                _ => return Err(WireError::BadEnum("ws_op")),
            };
            Message::WsCommand(WsWireCommand { nav_id, op })
        }
        MessageKind::WsReply => {
            let nav_id = r.u64()?;
            let body = match r.u8()? {
                1 => {
                    let ok = r.bool("ok")?;
                    let handle = r.u64()?;
                    let protocol = r.text(MAX_SHORT_TEXT_BYTES, "protocol")?;
                    let resolved_url = r.text(MAX_URL_BYTES, "resolved_url")?;
                    if !resolved_url.is_empty() {
                        check_ws_url(&resolved_url)?;
                    }
                    let error = r.text(MAX_SHORT_TEXT_BYTES, "error")?;
                    WsReplyBody::Connected {
                        ok,
                        handle,
                        protocol,
                        resolved_url,
                        error,
                    }
                }
                2 => {
                    let ok = r.bool("ok")?;
                    let error = r.text(MAX_SHORT_TEXT_BYTES, "error")?;
                    WsReplyBody::Ack { ok, error }
                }
                3 => {
                    let ready_state = r.u8()?;
                    if ready_state > 3 {
                        return Err(WireError::OutOfRange("ready_state"));
                    }
                    let count = r.u16()? as usize;
                    if count > MAX_WS_EVENTS {
                        return Err(WireError::TooMany("ws_events"));
                    }
                    let mut events = Vec::with_capacity(count);
                    for _ in 0..count {
                        let ev = match r.u8()? {
                            1 => WsWireEvent::Text(r.text(MAX_WS_PAYLOAD_BYTES, "ws_text")?),
                            2 => WsWireEvent::Binary(
                                r.bytes(MAX_WS_PAYLOAD_BYTES, "ws_binary")?.to_vec(),
                            ),
                            3 => {
                                let code = r.u16()?;
                                let reason = r.text(MAX_SHORT_TEXT_BYTES, "ws_reason")?;
                                WsWireEvent::Closed { code, reason }
                            }
                            4 => WsWireEvent::Error(r.text(MAX_SHORT_TEXT_BYTES, "ws_error")?),
                            _ => return Err(WireError::BadEnum("ws_event")),
                        };
                        events.push(ev);
                    }
                    WsReplyBody::Polled {
                        ready_state,
                        events,
                    }
                }
                _ => return Err(WireError::BadEnum("ws_reply")),
            };
            Message::WsReply(WsWireReply { nav_id, body })
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
                5 => FetchKind::ScriptFetch,
                6 => FetchKind::CorsFetch,
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
        MessageKind::StatusQuery => Message::StatusQuery,
        MessageKind::Status => {
            let no_new_privs = match r.u8()? {
                0 => Some(false),
                1 => Some(true),
                2 => None,
                _ => return Err(WireError::BadEnum("no_new_privs")),
            };
            let seccomp = match r.u8()? {
                mode @ 0..=2 => Some(mode),
                255 => None,
                _ => return Err(WireError::OutOfRange("seccomp")),
            };
            let mut namespaces = [0u64; 7];
            for id in &mut namespaces {
                *id = r.u64()?;
            }
            let uid_map = r.text(MAX_UID_MAP_BYTES, "uid_map")?;
            let pid = r.u32()?;
            let visible_processes = r.u32()?;
            let root_entries = r.text(MAX_SHORT_TEXT_BYTES, "root_entries")?;
            let writable_mounts = r.text(MAX_SHORT_TEXT_BYTES, "writable_mounts")?;
            let mut limits = [None; 5];
            for limit in &mut limits {
                let known = r.bool("limits")?;
                let value = r.u64()?;
                *limit = match (known, value) {
                    (true, value) => Some(value),
                    (false, 0) => None,
                    // An unknown limit is always sent as 0, so decoding
                    // stays exact.
                    (false, _) => return Err(WireError::OutOfRange("limits")),
                };
            }
            Message::Status(RendererStatus {
                no_new_privs,
                seccomp,
                namespaces,
                uid_map,
                pid,
                visible_processes,
                root_entries,
                writable_mounts,
                limits,
            })
        }
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

/// A renderer WebSocket URL must use `ws://` or `wss://` and visible ASCII bytes only.
fn check_ws_url(url: &str) -> Result<(), WireError> {
    let scheme_ok = url.starts_with("ws://") || url.starts_with("wss://");
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

/// Valid ASCII token characters for an HTTP method name.
fn check_http_method(method: &str) -> Result<(), WireError> {
    let ok = !method.is_empty()
        && method.len() <= MAX_METHOD_BYTES
        && method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    if ok {
        Ok(())
    } else {
        Err(WireError::OutOfRange("method"))
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
            extended_script_reach: false,
            viewport_width: 960,
            viewport_height: 720,
            https_only: true,
            http_allowed_hosts: vec!["intranet.test".to_string()],
        }
    }

    fn status() -> RendererStatus {
        RendererStatus {
            no_new_privs: Some(true),
            seccomp: Some(0),
            namespaces: [4_026_532_216, 4_026_532_333, 4_026_532_218, 7, 8, 9, 0],
            uid_map: "0 1000 1".to_string(),
            pid: 2,
            visible_processes: 2,
            root_entries: "bin dev lib lib64 proc tmp usr".to_string(),
            writable_mounts: "/:tmpfs /proc:proc /tmp:tmpfs".to_string(),
            limits: [Some(256 << 20), Some(0), Some(64), Some(0), None],
        }
    }

    #[test]
    fn status_rules_are_strict() {
        // Only the renderer reports a status, and only the browser asks.
        let frame = encode(1, &Message::Status(status())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToRenderer),
            Err(WireError::WrongDirection(MessageKind::Status))
        );
        let frame = encode(1, &Message::StatusQuery).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToBrowser),
            Err(WireError::WrongDirection(MessageKind::StatusQuery))
        );

        let good = encode(1, &Message::Status(status())).unwrap();
        let at = HEADER_BYTES;
        // NoNewPrivs is 0, 1 or 2 (unknown).
        let mut bad = good.clone();
        bad[at] = 3;
        assert_eq!(
            decode(&bad, Direction::ToBrowser),
            Err(WireError::BadEnum("no_new_privs"))
        );
        // The seccomp mode is 0 to 2, or 255 for unknown.
        let mut bad = good.clone();
        bad[at + 1] = 3;
        assert_eq!(
            decode(&bad, Direction::ToBrowser),
            Err(WireError::OutOfRange("seccomp"))
        );
        assert!(
            encode(
                1,
                &Message::Status(RendererStatus {
                    seccomp: Some(9),
                    ..status()
                })
            )
            .is_err()
        );
        // An unknown limit must carry 0.
        let last_limit = good.len() - 9;
        let mut bad = good.clone();
        bad[last_limit + 1] = 1;
        assert_eq!(
            decode(&bad, Direction::ToBrowser),
            Err(WireError::OutOfRange("limits"))
        );
        // Text fields keep their caps.
        let long = RendererStatus {
            uid_map: "x".repeat(MAX_UID_MAP_BYTES + 1),
            ..status()
        };
        assert_eq!(
            encode(1, &Message::Status(long)),
            Err(WireError::FieldTooLong("uid_map"))
        );
        // The largest status still fits its kind.
        let largest = RendererStatus {
            uid_map: "x".repeat(MAX_UID_MAP_BYTES),
            root_entries: "y".repeat(MAX_SHORT_TEXT_BYTES),
            writable_mounts: "z".repeat(MAX_SHORT_TEXT_BYTES),
            ..status()
        };
        assert!(encode(1, &Message::Status(largest)).is_ok());
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
                    final_url: "https://example.test/page".to_string(),
                    animating: true,
                    has_next_timer: true,
                    next_timer_ms_bits: 16.0f64.to_bits(),
                    spa_back_depth: 2,
                    spa_forward_depth: 1,
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
                Direction::ToBrowser,
                Message::Fetch(FetchRequest {
                    nav_id: 9,
                    kind: FetchKind::ScriptFetch,
                    url: "https://example.test/api?q=1".to_string(),
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
            (Direction::ToRenderer, Message::StatusQuery),
            (Direction::ToBrowser, Message::Status(status())),
            (
                Direction::ToBrowser,
                Message::Status(RendererStatus::default()),
            ),
            (Direction::ToBrowser, Message::Frame(painted())),
            (
                Direction::ToBrowser,
                Message::Frame(RenderedFrame {
                    display_list: Vec::new(),
                    ..painted()
                }),
            ),
            (Direction::ToRenderer, Message::Resize(resize())),
            (
                Direction::ToRenderer,
                Message::Interact(InteractRequest {
                    tab_id: 3,
                    nav_id: 9,
                    op: InteractOp::Click,
                    x_bits: 120.5f32.to_bits(),
                    y_bits: 44.0f32.to_bits(),
                    delta_bits: 0,
                    now_ms_bits: 32.0f64.to_bits(),
                    viewport_width: 1280,
                    viewport_height: 720,
                    modifiers: 1,
                    text: "ok".to_string(),
                }),
            ),
            (
                Direction::ToBrowser,
                Message::InteractResult(InteractOutcome {
                    tab_id: 3,
                    nav_id: 9,
                    changed: true,
                    handled_by_script: true,
                    animating: false,
                    focus_input: false,
                    title: "Updated".to_string(),
                    navigate_url: Some("https://example.test/next".to_string()),
                    post_url: None,
                    post_content_type: String::new(),
                    post_body: Vec::new(),
                    scroll_to_bits: 0,
                    has_scroll_to: false,
                    next_timer_ms_bits: 0,
                    has_next_timer: false,
                    current_url: "https://example.test/spa".to_string(),
                    spa_back_depth: 1,
                    spa_forward_depth: 0,
                    copied_text: Some("copied".to_string()),
                }),
            ),
            (
                Direction::ToBrowser,
                Message::ScriptFetch(ScriptFetchWireRequest {
                    nav_id: 9,
                    method: "POST".to_string(),
                    url: "https://example.test/graphql".to_string(),
                    credentials_include: true,
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: b"{\"q\":1}".to_vec(),
                }),
            ),
            (
                Direction::ToRenderer,
                Message::ScriptFetchReply(ScriptFetchWireReply {
                    nav_id: 9,
                    status: 200,
                    final_url: "https://example.test/graphql".to_string(),
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: b"{\"ok\":true}".to_vec(),
                }),
            ),
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: 9,
                    op: WsCommandOp::Connect {
                        url: "wss://example.test/live".to_string(),
                        protocols: vec!["chat".to_string()],
                    },
                }),
            ),
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: 9,
                    op: WsCommandOp::SendText {
                        handle: 42,
                        text: "hello".to_string(),
                    },
                }),
            ),
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: 9,
                    op: WsCommandOp::SendBinary {
                        handle: 42,
                        data: vec![1, 2, 3],
                    },
                }),
            ),
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: 9,
                    op: WsCommandOp::Close {
                        handle: 42,
                        code: 1000,
                        reason: "done".to_string(),
                    },
                }),
            ),
            (
                Direction::ToBrowser,
                Message::WsCommand(WsWireCommand {
                    nav_id: 9,
                    op: WsCommandOp::Poll { handle: 42 },
                }),
            ),
            (
                Direction::ToRenderer,
                Message::WsReply(WsWireReply {
                    nav_id: 9,
                    body: WsReplyBody::Connected {
                        ok: true,
                        handle: 42,
                        protocol: "chat".to_string(),
                        resolved_url: "wss://example.test/live".to_string(),
                        error: String::new(),
                    },
                }),
            ),
            (
                Direction::ToRenderer,
                Message::WsReply(WsWireReply {
                    nav_id: 9,
                    body: WsReplyBody::Ack {
                        ok: true,
                        error: String::new(),
                    },
                }),
            ),
            (
                Direction::ToRenderer,
                Message::WsReply(WsWireReply {
                    nav_id: 9,
                    body: WsReplyBody::Polled {
                        ready_state: 1,
                        events: vec![
                            WsWireEvent::Text("pong".to_string()),
                            WsWireEvent::Binary(vec![9, 8]),
                            WsWireEvent::Closed {
                                code: 1000,
                                reason: "bye".to_string(),
                            },
                            WsWireEvent::Error("net".to_string()),
                        ],
                    },
                }),
            ),
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

    fn painted() -> RenderedFrame {
        RenderedFrame {
            tab_id: 3,
            nav_id: 9,
            title: "A page".to_string(),
            dom_nodes: 120,
            blocked_resources: 2,
            display_list: vec![0, 1, 2, 3, 255],
        }
    }

    fn resize() -> ResizeRequest {
        ResizeRequest {
            tab_id: 3,
            nav_id: 9,
            viewport_width: 1280,
            viewport_height: 720,
        }
    }

    #[test]
    fn frames_go_only_to_the_browser_and_resizes_only_to_the_renderer() {
        let frame = encode(1, &Message::Frame(painted())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToRenderer),
            Err(WireError::WrongDirection(MessageKind::Frame))
        );
        let frame = encode(1, &Message::Resize(resize())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToBrowser),
            Err(WireError::WrongDirection(MessageKind::Resize))
        );
    }

    #[test]
    fn frame_and_resize_fields_are_checked() {
        let big = RenderedFrame {
            display_list: vec![0; MAX_DISPLAY_LIST_BYTES + 1],
            ..painted()
        };
        assert_eq!(
            encode(1, &Message::Frame(big)),
            Err(WireError::FieldTooLong("display_list"))
        );
        let largest = RenderedFrame {
            title: "t".repeat(MAX_TITLE_BYTES),
            display_list: vec![7; MAX_DISPLAY_LIST_BYTES],
            ..painted()
        };
        let frame = encode(1, &Message::Frame(largest.clone())).unwrap();
        assert_eq!(
            decode(&frame, Direction::ToBrowser).unwrap().1,
            Message::Frame(largest)
        );
        let nodes = RenderedFrame {
            dom_nodes: MAX_REPORTED_NODES + 1,
            ..painted()
        };
        assert_eq!(
            encode(1, &Message::Frame(nodes)),
            Err(WireError::OutOfRange("dom_nodes"))
        );

        // A length inside the body over the cap is refused before the bytes
        // are copied: the display list length sits just before its bytes.
        let mut frame = encode(1, &Message::Frame(painted())).unwrap();
        let at = frame.len() - painted().display_list.len() - 4;
        frame[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode(&frame, Direction::ToBrowser),
            Err(WireError::FieldTooLong("display_list"))
        );

        for (w, h) in [(0, 720), (1280, 0), (MAX_VIEWPORT_SIDE + 1, 720)] {
            let r = ResizeRequest {
                viewport_width: w,
                viewport_height: h,
                ..resize()
            };
            assert!(encode(1, &Message::Resize(r)).is_err());
        }
        let mut frame = encode(1, &Message::Resize(resize())).unwrap();
        frame[HEADER_BYTES + 16..HEADER_BYTES + 20].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            decode(&frame, Direction::ToRenderer),
            Err(WireError::OutOfRange("viewport_width"))
        );
        assert_eq!(MessageKind::Resize.max_body(), 24);
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

        for version in [0, 1, WIRE_VERSION + 1, u16::MAX] {
            let mut bad = good.clone();
            bad[4..6].copy_from_slice(&version.to_le_bytes());
            assert_eq!(
                decode(&bad, Direction::ToRenderer),
                Err(WireError::UnsupportedVersion(version))
            );
        }

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
                final_url: String::new(),
                animating: false,
                has_next_timer: false,
                next_timer_ms_bits: 0,
                spa_back_depth: 0,
                spa_forward_depth: 0,
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
