// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Renderer processes for tabs.
//!
//! Each tab in the window has its own renderer process ([`TabWorkerPool`]),
//! started as `anvil --renderer` through the `anvil-sandbox` launcher and
//! spoken to in its binary wire format. http(s) documents and local files are
//! parsed, scripted, and serialized in that process. The window only lays out
//! and paints the document that comes back.
//!
//! The launcher starts the renderer with an empty environment (so on Linux it
//! has no `DISPLAY` and can't reach the X11 session), checks its hello, puts a
//! deadline on every answer, and kills it on the first bad frame or on an
//! answer for another tab or navigation. Every request carries a navigation
//! id. The renderer also runs under the `anvil-sandbox` worker limits. A
//! renderer that hangs or crashes is killed and the tab shows an error page.
//! The window keeps running. A tab whose renderer crashes three times in a
//! minute stops starting new ones for a while and shows an error page.
//! Closing a tab ends its renderer. `anvil://` pages do not use this module.
//!
//! Between pages, every renderer gets a heartbeat every few seconds and again
//! before its next page: it has to answer a ping, and on Linux it must not
//! have used more than a second of CPU since it last finished a page. One
//! that fails is killed, and the tab's next page starts a fresh one.
//!
//! On Linux the pool starts renderers through `bwrap` or `unshare` when the
//! system has one and it passes a self-test, so each renderer gets its own
//! namespaces and can't see or read the window. Without such a tool, or with
//! confinement turned off, renderers start as before. See
//! `anvil_sandbox::confine`. The decision is made once, when the first
//! renderer is needed or when the window starts, and is kept with its
//! reasons for `anvil://security`.
//!
//! The window also keeps all of its renderers together under
//! [`WINDOW_RENDERER_MEMORY_BUDGET_BYTES`]. When the idle ones add up to
//! more than that, the pool ends idle renderers of background tabs, the one
//! left alone longest first, until the rest fit. The page those tabs show
//! stays in the window. Their next page just starts a fresh renderer, so
//! this is not counted as a crash. The tab in front is never ended to make
//! room, and a renderer in the middle of a page is left to finish it.
//!
//! While it works on a page, a renderer may only use so much CPU: the
//! launcher's [`DEFAULT_RENDER_CPU`] for the tab in front, and
//! [`BACKGROUND_RENDER_CPU`] for a page loading in a tab behind it, so a
//! background tab can't take the machine away from the one being read.
//! Waiting on the network costs nothing. A page over its share is
//! stopped and the tab shows an error page.
//!
//! [`exchange_tab_worker`] is the one-document form used by the `render` and
//! `raster` commands. The session cookie jar is never part of a request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anvil_net::{
    ContentSecurityPolicy, CspDirective, WebSocketClient, WsConnectOptions, WsEvent, WsMessage,
    parse_url,
};
use anvil_sandbox::confine::{self, ConfineMode, Confinement, ToolKind};
use anvil_sandbox::launcher::{
    DEFAULT_RENDER_CPU, RelaunchBudget, RenderOutcome, RendererCommand, RendererFailure,
    RendererLimits, RendererProcess, resident_bytes,
};
use anvil_sandbox::wire::{
    FailureCode, FetchReply, FetchRequest, InteractOutcome, InteractRequest, MAX_ALLOWED_HOSTS,
    MAX_CSP_BYTES, MAX_DOCUMENT_BYTES, MAX_HOST_BYTES, MAX_URL_BYTES, MAX_VIEWPORT_SIDE,
    MAX_WS_EVENTS, RenderRequest, ResizeRequest, ScriptFetchWireReply, ScriptFetchWireRequest,
    WsCommandOp, WsReplyBody, WsWireCommand, WsWireEvent, WsWireReply,
};
use anvil_sandbox::{
    MANDATORY_TAB_MEMORY_LIMIT_BYTES, MANDATORY_WORKER_TIMEOUT_MS, TabWorkerRequest,
    TabWorkerResponse,
};

/// Maximum concurrent open WebSocket connections mediated for a single tab renderer.
const MAX_TAB_WEBSOCKETS: usize = 32;

/// Parent-browser registry holding active [`WebSocketClient`] sockets for one sandboxed tab.
#[derive(Default)]
pub struct RendererWsRegistry {
    sockets: HashMap<u64, WebSocketClient>,
    next_handle: u64,
}

impl std::fmt::Debug for RendererWsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererWsRegistry")
            .field("active_sockets", &self.sockets.len())
            .finish()
    }
}

impl RendererWsRegistry {
    pub fn new() -> Self {
        Self {
            sockets: HashMap::new(),
            next_handle: 1,
        }
    }

    pub fn clear(&mut self) {
        self.sockets.clear();
    }

    pub fn handle(
        &mut self,
        cmd: &WsWireCommand,
        page_url: &str,
        csp: &ContentSecurityPolicy,
    ) -> WsWireReply {
        let body = match &cmd.op {
            WsCommandOp::Connect { url, protocols } => {
                self.connect_socket(url, protocols, page_url, csp)
            }
            WsCommandOp::SendText { handle, text } => match self.sockets.get_mut(handle) {
                Some(ws) => match ws.send_text(text) {
                    Ok(()) => WsReplyBody::Ack {
                        ok: true,
                        error: String::new(),
                    },
                    Err(err) => WsReplyBody::Ack {
                        ok: false,
                        error: err.to_string(),
                    },
                },
                None => WsReplyBody::Ack {
                    ok: false,
                    error: "WebSocket connection not found".to_string(),
                },
            },
            WsCommandOp::SendBinary { handle, data } => match self.sockets.get_mut(handle) {
                Some(ws) => match ws.send_binary(data) {
                    Ok(()) => WsReplyBody::Ack {
                        ok: true,
                        error: String::new(),
                    },
                    Err(err) => WsReplyBody::Ack {
                        ok: false,
                        error: err.to_string(),
                    },
                },
                None => WsReplyBody::Ack {
                    ok: false,
                    error: "WebSocket connection not found".to_string(),
                },
            },
            WsCommandOp::Close {
                handle,
                code,
                reason,
            } => match self.sockets.get_mut(handle) {
                Some(ws) => match ws.close(*code, reason) {
                    Ok(()) => WsReplyBody::Ack {
                        ok: true,
                        error: String::new(),
                    },
                    Err(err) => WsReplyBody::Ack {
                        ok: false,
                        error: err.to_string(),
                    },
                },
                None => WsReplyBody::Ack {
                    ok: false,
                    error: "WebSocket connection not found".to_string(),
                },
            },
            WsCommandOp::Poll { handle } => {
                let Some(ws) = self.sockets.get_mut(handle) else {
                    return WsWireReply {
                        nav_id: cmd.nav_id,
                        body: WsReplyBody::Polled {
                            ready_state: 3,
                            events: Vec::new(),
                        },
                    };
                };
                let polled = ws.poll_events();
                let ready_state = (ws.ready_state() as u8).min(3);
                if ready_state == 3 {
                    self.sockets.remove(handle);
                }
                let mut events = Vec::new();
                for ev in polled {
                    if events.len() >= MAX_WS_EVENTS {
                        break;
                    }
                    match ev {
                        WsEvent::Message(WsMessage::Text(text)) => {
                            events.push(WsWireEvent::Text(text));
                        }
                        WsEvent::Message(WsMessage::Binary(bytes)) => {
                            events.push(WsWireEvent::Binary(bytes));
                        }
                        WsEvent::Closed { code, reason } => {
                            events.push(WsWireEvent::Closed { code, reason });
                        }
                        WsEvent::Ping(_) | WsEvent::Pong(_) => {}
                    }
                }
                WsReplyBody::Polled {
                    ready_state,
                    events,
                }
            }
        };
        WsWireReply {
            nav_id: cmd.nav_id,
            body,
        }
    }

    fn connect_socket(
        &mut self,
        url: &str,
        protocols: &[String],
        page_url: &str,
        csp: &ContentSecurityPolicy,
    ) -> WsReplyBody {
        if self.sockets.len() >= MAX_TAB_WEBSOCKETS {
            return WsReplyBody::Connected {
                ok: false,
                handle: 0,
                protocol: String::new(),
                resolved_url: String::new(),
                error: "too many open WebSocket connections".to_string(),
            };
        }
        if is_ws_mixed_content(page_url, url) {
            return WsReplyBody::Connected {
                ok: false,
                handle: 0,
                protocol: String::new(),
                resolved_url: String::new(),
                error: "mixed content blocked insecure WebSocket".to_string(),
            };
        }
        if !csp.allows_resource(CspDirective::ConnectSrc, page_url, url) {
            return WsReplyBody::Connected {
                ok: false,
                handle: 0,
                protocol: String::new(),
                resolved_url: String::new(),
                error: "connect-src blocked the WebSocket connection".to_string(),
            };
        }
        let origin = parse_url(page_url)
            .ok()
            .map(|p| format!("{}://{}:{}", p.scheme, p.host, p.port));
        let opts = WsConnectOptions {
            origin,
            protocols: protocols.to_vec(),
            connect_timeout: Some(Duration::from_secs(5)),
        };
        match WebSocketClient::connect_with_options(url, &opts) {
            Ok(client) => {
                if self.next_handle == 0 {
                    self.next_handle = 1;
                }
                let handle = self.next_handle;
                self.next_handle = self.next_handle.wrapping_add(1).max(1);
                let proto = client.protocol().unwrap_or("").to_string();
                let resolved_url = client.url().to_string();
                self.sockets.insert(handle, client);
                WsReplyBody::Connected {
                    ok: true,
                    handle,
                    protocol: proto,
                    resolved_url,
                    error: String::new(),
                }
            }
            Err(err) => WsReplyBody::Connected {
                ok: false,
                handle: 0,
                protocol: String::new(),
                resolved_url: String::new(),
                error: err.to_string(),
            },
        }
    }
}

fn is_ws_mixed_content(page_url: &str, ws_url: &str) -> bool {
    let page_is_https = parse_url(page_url).is_ok_and(|p| p.scheme == "https");
    if !page_is_https || !ws_url.starts_with("ws://") {
        return false;
    }
    let http_equiv = format!("http://{}", &ws_url[5..]);
    if let Ok(parsed) = parse_url(&http_equiv)
        && matches!(
            parsed.host.to_ascii_lowercase().as_str(),
            "localhost" | "127.0.0.1" | "::1" | "[::1]"
        )
    {
        return false;
    }
    anvil_net::https_only_policy()
        .upgrade(&http_equiv)
        .is_none()
}

/// The argument that starts `anvil` as a renderer.
pub const RENDERER_ARG: &str = "--renderer";

/// How often an idle renderer gets a heartbeat.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Most resident memory one window's idle tab renderers may use together:
/// four renderers at their own cap. Each renderer still has its own cap
/// on top of this.
pub const WINDOW_RENDERER_MEMORY_BUDGET_BYTES: u64 = 4 * MANDATORY_TAB_MEMORY_LIMIT_BYTES as u64;

/// Most CPU time one page may use in a tab that is not in front.
pub const BACKGROUND_RENDER_CPU: Duration = Duration::from_secs(10);

/// One idle renderer, as the window memory budget sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleRenderer {
    pub tab_id: u64,
    pub resident_bytes: u64,
    /// When it last finished a page. `None` if it never has.
    pub last_used: Option<Instant>,
}

/// Which idle renderers to end so the rest fit in `budget_bytes`.
///
/// Nothing is ended while the total fits. Otherwise background tabs go
/// first by how long they have been left alone, longest first, and only
/// as many as it takes. The `focused` tab is never picked, even when it
/// alone is over the budget.
pub fn renderers_to_end(
    idle: &[IdleRenderer],
    focused: Option<u64>,
    budget_bytes: u64,
) -> Vec<u64> {
    let mut total = idle
        .iter()
        .fold(0u64, |sum, r| sum.saturating_add(r.resident_bytes));
    if total <= budget_bytes {
        return Vec::new();
    }
    let mut background: Vec<&IdleRenderer> =
        idle.iter().filter(|r| Some(r.tab_id) != focused).collect();
    background.sort_by_key(|r| (r.last_used, r.tab_id));
    let mut ended = Vec::new();
    for renderer in background {
        if total <= budget_bytes {
            break;
        }
        total = total.saturating_sub(renderer.resident_bytes);
        ended.push(renderer.tab_id);
    }
    ended
}

/// `MANDATORY_WORKER_TIMEOUT_MS` as a `Duration`. The window uses this.
pub fn worker_timeout() -> Duration {
    Duration::from_millis(MANDATORY_WORKER_TIMEOUT_MS)
}

/// Path of the running executable. The window passes this to [`exchange_tab_worker`]
/// with [`RENDERER_ARG`].
pub fn current_anvil_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|err| format!("could not find the anvil executable: {err}"))
}

/// Launcher limits for a renderer that gets `timeout` to start and for
/// each answer.
fn renderer_limits(timeout: Duration) -> RendererLimits {
    RendererLimits {
        startup: timeout,
        reply: timeout,
        ..RendererLimits::default()
    }
}

fn renderer_command(program: &Path, args: &[String]) -> RendererCommand {
    args.iter()
        .fold(RendererCommand::new(program), |command, arg| {
            command.arg(arg.as_str())
        })
}

/// The wire request for `request`, as navigation `nav_id`.
///
/// The viewport is clamped to what the wire format allows. HTTPS-only
/// exceptions that aren't plain host names, or that don't fit, are left
/// out, which only makes the renderer stricter. A request that still can't
/// be sent is `Err`, before any renderer sees it.
fn render_request(request: &TabWorkerRequest, nav_id: u64) -> Result<RenderRequest, String> {
    if request.url.len() > MAX_URL_BYTES {
        return Err("the address is too long to hand to the renderer".to_string());
    }
    if request.html.len() > MAX_DOCUMENT_BYTES {
        return Err("the page is too large to hand to the renderer".to_string());
    }
    if request
        .csp_header
        .as_ref()
        .is_some_and(|csp| csp.len() > MAX_CSP_BYTES)
    {
        return Err("the page's content security policy is too long".to_string());
    }
    let side = |value: u32| value.clamp(1, MAX_VIEWPORT_SIDE);
    let http_allowed_hosts = request
        .http_allowed_hosts
        .iter()
        .filter(|host| {
            !host.is_empty()
                && host.len() <= MAX_HOST_BYTES
                && host
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        })
        .take(MAX_ALLOWED_HOSTS)
        .cloned()
        .collect();
    Ok(RenderRequest {
        tab_id: request.tab_id,
        nav_id,
        url: request.url.clone(),
        html: request.html.clone(),
        csp_header: request.csp_header.clone(),
        scripts_enabled: request.scripts_enabled,
        extended_script_reach: request.extended_script_reach,
        viewport_width: side(request.viewport_width),
        viewport_height: side(request.viewport_height),
        https_only: request.https_only,
        http_allowed_hosts,
    })
}

/// Answers every fetch as failed. For callers that don't want a renderer
/// to load anything.
pub fn refuse_fetch(want: &FetchRequest) -> FetchReply {
    FetchReply {
        nav_id: want.nav_id,
        status: 0,
        content_type: String::new(),
        body: Vec::new(),
    }
}

/// Turn the renderer's answer into the response the window paints from.
///
/// A refusal becomes `ok: false` with a fixed text for its code. The
/// renderer's own detail text is not shown.
fn into_response(outcome: RenderOutcome, tab_id: u64) -> TabWorkerResponse {
    match outcome {
        RenderOutcome::Done(doc, frame) => TabWorkerResponse {
            tab_id: doc.tab_id,
            ok: true,
            title: doc.title,
            dom_nodes: doc.dom_nodes as usize,
            blocked_csp_resources: doc.blocked_resources as usize,
            error: None,
            html: doc.html,
            frame: frame.map(|painted| painted.display_list),
            final_url: doc.final_url,
            animating: doc.animating,
            has_next_timer: doc.has_next_timer,
            next_timer_ms_bits: doc.next_timer_ms_bits,
            spa_back_depth: doc.spa_back_depth,
            spa_forward_depth: doc.spa_forward_depth,
        },
        RenderOutcome::Refused(failure) => {
            let error = match failure.code {
                FailureCode::Limit => "the page hit a renderer limit",
                FailureCode::TooLarge => "the finished page is too large",
                FailureCode::Internal => "the renderer could not finish the page",
            };
            TabWorkerResponse {
                tab_id,
                ok: false,
                title: String::new(),
                dom_nodes: 0,
                blocked_csp_resources: 0,
                error: Some(error.to_string()),
                html: String::new(),
                frame: None,
                final_url: String::new(),
                animating: false,
                has_next_timer: false,
                next_timer_ms_bits: 0,
                spa_back_depth: 0,
                spa_forward_depth: 0,
            }
        }
    }
}

/// Start `program` with `args` as a renderer, send `request`, and wait up
/// to `timeout` for it to start and again for its answer.
///
/// Each fetch the renderer asks for on the way goes to `fetch`, which is
/// where the browser applies its rules.
///
/// A timeout, a start failure, a bad frame, or a crash is `Err`, and the
/// process is gone. `Ok` is the renderer's answer, including `ok: false`.
/// The caller must not paint `html` unless `ok` is true.
pub fn exchange_tab_worker(
    program: &Path,
    args: &[String],
    request: &TabWorkerRequest,
    timeout: Duration,
    fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
) -> Result<TabWorkerResponse, String> {
    let mut no_script = |want: &ScriptFetchWireRequest| ScriptFetchWireReply {
        nav_id: want.nav_id,
        status: 0,
        final_url: String::new(),
        headers: Vec::new(),
        body: Vec::new(),
    };
    exchange_tab_worker_with_script_fetch(program, args, request, timeout, fetch, &mut no_script)
}

/// [`exchange_tab_worker`] forwarding rich `fetch()` / `XMLHttpRequest` calls via `script_fetch`.
pub fn exchange_tab_worker_with_script_fetch(
    program: &Path,
    args: &[String],
    request: &TabWorkerRequest,
    timeout: Duration,
    fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
) -> Result<TabWorkerResponse, String> {
    let parsed_doc_csp = anvil_dom::parse_html(&request.html).meta_csp();
    let combined_csp = match (request.csp_header.as_deref(), parsed_doc_csp.as_deref()) {
        (Some(h), Some(m)) => ContentSecurityPolicy::parse(&format!("{h}; {m}")),
        (Some(h), None) => ContentSecurityPolicy::parse(h),
        (None, Some(m)) => ContentSecurityPolicy::parse(m),
        (None, None) => ContentSecurityPolicy::default(),
    };
    let mut ws_registry = RendererWsRegistry::new();
    let page_url = request.url.clone();
    let mut ws_bridge = |cmd: &WsWireCommand| ws_registry.handle(cmd, &page_url, &combined_csp);
    let wire_request = render_request(request, 1)?;
    let mut process =
        RendererProcess::launch(&renderer_command(program, args), renderer_limits(timeout))
            .map_err(|err| err.to_string())?;
    let outcome = process
        .render_with_ipc(&wire_request, fetch, script_fetch, &mut ws_bridge)
        .map_err(|err| err.to_string())?;
    process.shutdown();
    Ok(into_response(outcome, request.tab_id))
}

/// One long-lived renderer process that belongs to one tab.
///
/// The window sends `Render`, `Resize`, and interactive `Interact` events over
/// the wire channel while keeping the page's live DOM and scripts intact inside
/// the sandboxed worker.
#[derive(Debug)]
pub struct TabProcess {
    process: RendererProcess,
    served: u64,
    active_nav: u64,
    confined_by: Option<ToolKind>,
}

impl TabProcess {
    /// Start `program` with `args` and wait up to `timeout` for its hello.
    pub fn spawn(program: &Path, args: &[String], timeout: Duration) -> Result<Self, String> {
        Self::start_command(&renderer_command(program, args), timeout, None)
            .map_err(|err| err.to_string())
    }

    fn start_command(
        command: &RendererCommand,
        timeout: Duration,
        confined_by: Option<ToolKind>,
    ) -> Result<Self, RendererFailure> {
        let process = RendererProcess::launch(command, renderer_limits(timeout))?;
        Ok(Self {
            process,
            served: 0,
            active_nav: 0,
            confined_by,
        })
    }

    /// The tool this renderer was started through, if any.
    pub fn confined_by(&self) -> Option<ToolKind> {
        self.confined_by
    }

    /// Process id of the renderer.
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Check that the renderer is alive, answers a ping, and has not kept
    /// the CPU busy since it last finished a page. Any `Err` means it has
    /// been killed.
    pub fn heartbeat(&mut self) -> Result<(), String> {
        self.process.heartbeat().map_err(|err| err.to_string())
    }

    /// Let each page from now on use at most `limit` of CPU.
    pub fn set_render_cpu(&mut self, limit: Duration) {
        self.process.set_render_cpu(limit);
    }

    /// Number of documents this renderer has answered.
    pub fn served(&self) -> u64 {
        self.served
    }

    /// True while the process is running and has not misbehaved.
    pub fn is_running(&mut self) -> bool {
        self.process.is_alive()
    }

    /// Send `request` as navigation `nav_id` and wait for the answer. Each
    /// fetch the renderer asks for on the way goes to `fetch`.
    pub fn exchange(
        &mut self,
        request: &TabWorkerRequest,
        nav_id: u64,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    ) -> Result<TabWorkerResponse, String> {
        let mut no_script = |want: &ScriptFetchWireRequest| ScriptFetchWireReply {
            nav_id: want.nav_id,
            status: 0,
            final_url: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        self.exchange_with_script_fetch(request, nav_id, fetch, &mut no_script)
    }

    /// Send `request` forwarding both subresource fetches and rich `fetch()` requests.
    pub fn exchange_with_script_fetch(
        &mut self,
        request: &TabWorkerRequest,
        nav_id: u64,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
    ) -> Result<TabWorkerResponse, String> {
        let wire_request = render_request(request, nav_id)?;
        let outcome = self
            .process
            .render_with_script_fetch(&wire_request, fetch, script_fetch)
            .map_err(|err| err.to_string())?;
        self.served += 1;
        if matches!(outcome, RenderOutcome::Done(..)) {
            self.active_nav = nav_id;
        } else {
            self.active_nav = 0;
        }
        Ok(into_response(outcome, request.tab_id))
    }

    /// Send `request` forwarding subresource fetches, rich `fetch()` requests, and WebSocket IPC.
    pub fn exchange_with_ipc(
        &mut self,
        request: &TabWorkerRequest,
        nav_id: u64,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
        ws_bridge: &mut dyn FnMut(&WsWireCommand) -> WsWireReply,
    ) -> Result<TabWorkerResponse, String> {
        let wire_request = render_request(request, nav_id)?;
        let outcome = self
            .process
            .render_with_ipc(&wire_request, fetch, script_fetch, ws_bridge)
            .map_err(|err| err.to_string())?;
        self.served += 1;
        if matches!(outcome, RenderOutcome::Done(..)) {
            self.active_nav = nav_id;
        } else {
            self.active_nav = 0;
        }
        Ok(into_response(outcome, request.tab_id))
    }

    /// Re-stage the active document in this worker at a new viewport size.
    pub fn resize(
        &mut self,
        tab_id: u64,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Option<Vec<u8>>, String> {
        if self.active_nav == 0 {
            return Ok(None);
        }
        let req = ResizeRequest {
            tab_id,
            nav_id: self.active_nav,
            viewport_width: viewport_width.clamp(1, MAX_VIEWPORT_SIDE),
            viewport_height: viewport_height.clamp(1, MAX_VIEWPORT_SIDE),
        };
        let frame = self.process.resize(&req).map_err(|err| err.to_string())?;
        Ok(frame.map(|f| f.display_list))
    }

    /// Dispatch an interactive click, input, key, scroll, or timer tick to the live page.
    pub fn interact(
        &mut self,
        mut request: InteractRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
    ) -> Result<Option<InteractFrameUpdate>, String> {
        if self.active_nav == 0 {
            return Ok(None);
        }
        request.nav_id = self.active_nav;
        let (outcome, frame) = self
            .process
            .interact(&request, fetch, script_fetch)
            .map_err(|err| err.to_string())?;
        Ok(Some((outcome, frame.map(|f| f.display_list))))
    }

    /// Dispatch an interactive event while forwarding subresource fetches, script `fetch()`, and WebSocket IPC.
    pub fn interact_with_ipc(
        &mut self,
        mut request: InteractRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
        ws_bridge: &mut dyn FnMut(&WsWireCommand) -> WsWireReply,
    ) -> Result<Option<InteractFrameUpdate>, String> {
        if self.active_nav == 0 {
            return Ok(None);
        }
        request.nav_id = self.active_nav;
        let (outcome, frame) = self
            .process
            .interact_with_ipc(&request, fetch, script_fetch, ws_bridge)
            .map_err(|err| err.to_string())?;
        Ok(Some((outcome, frame.map(|f| f.display_list))))
    }
}

/// Result pair returned by an interactive renderer turn: outcome plus optional new frame bytes.
pub type InteractFrameUpdate = (InteractOutcome, Option<Vec<u8>>);

/// The program and arguments used to start a tab's renderer.
#[derive(Clone, Debug)]
struct WorkerCommand {
    program: Option<PathBuf>,
    args: Vec<String>,
    timeout: Duration,
    confine: ConfineMode,
}

/// One slot per tab. The inner lock keeps one navigation per tab at a time.
type TabSlot = Arc<Mutex<Option<TabProcess>>>;

/// Every tab's slot, keyed by tab id.
type Slots = Mutex<HashMap<u64, TabSlot>>;

/// Every tab's crash count, keyed by tab id.
type Budgets = Mutex<HashMap<u64, RelaunchBudget>>;

/// What the window memory budget needs to know.
#[derive(Debug)]
struct WindowBudget {
    memory_bytes: AtomicU64,
    /// The tab in front. Its renderer is never ended to make room.
    focused: Mutex<Option<u64>>,
    /// When each tab's renderer last finished a page.
    last_used: Mutex<HashMap<u64, Instant>>,
    /// CPU for one page in the tab in front, and in any other, in ms.
    front_cpu_ms: AtomicU64,
    background_cpu_ms: AtomicU64,
}

impl WindowBudget {
    fn new() -> Self {
        Self {
            memory_bytes: AtomicU64::new(WINDOW_RENDERER_MEMORY_BUDGET_BYTES),
            focused: Mutex::new(None),
            last_used: Mutex::new(HashMap::new()),
            front_cpu_ms: AtomicU64::new(millis(DEFAULT_RENDER_CPU)),
            background_cpu_ms: AtomicU64::new(millis(BACKGROUND_RENDER_CPU)),
        }
    }

    /// How much CPU a page in `tab_id` may use.
    fn render_cpu(&self, tab_id: u64) -> Duration {
        let in_front = self
            .focused
            .lock()
            .ok()
            .and_then(|focused| *focused)
            .is_none_or(|focused| focused == tab_id);
        let ms = if in_front {
            &self.front_cpu_ms
        } else {
            &self.background_cpu_ms
        };
        Duration::from_millis(ms.load(Ordering::Relaxed))
    }

    fn forget(&self, tab_id: u64) {
        if let Ok(mut last_used) = self.last_used.lock() {
            last_used.remove(&tab_id);
        }
    }
}

/// Measure every idle renderer in `slots` and end the ones
/// [`renderers_to_end`] picks. Returns their tab ids.
///
/// The idle renderers stay locked while this decides, so none of them
/// starts a page halfway through. A renderer whose memory can't be read
/// counts as using none. Ending one this way is not a crash.
fn end_renderers_over_budget(
    slots: &Slots,
    budget: &WindowBudget,
    measure: &dyn Fn(u32) -> Option<u64>,
) -> Vec<u64> {
    let tabs: Vec<(u64, TabSlot)> = match slots.lock() {
        Ok(slots) => slots
            .iter()
            .map(|(id, slot)| (*id, Arc::clone(slot)))
            .collect(),
        Err(_) => return Vec::new(),
    };
    let focused = budget.focused.lock().ok().and_then(|focused| *focused);
    let last_used = budget
        .last_used
        .lock()
        .map(|last_used| last_used.clone())
        .unwrap_or_default();
    let mut held = Vec::new();
    let mut idle = Vec::new();
    for (tab_id, slot) in &tabs {
        let Ok(mut guard) = slot.try_lock() else {
            continue;
        };
        let Some(process) = guard.as_mut() else {
            continue;
        };
        if !process.is_running() {
            continue;
        }
        idle.push(IdleRenderer {
            tab_id: *tab_id,
            resident_bytes: measure(process.pid()).unwrap_or(0),
            last_used: last_used.get(tab_id).copied(),
        });
        held.push((*tab_id, guard));
    }
    let limit = budget.memory_bytes.load(Ordering::Relaxed);
    let ended = renderers_to_end(&idle, focused, limit);
    for (tab_id, guard) in &mut held {
        if ended.contains(tab_id) {
            **guard = None;
        }
    }
    ended
}

/// Renderers for one window, keyed by tab id.
///
/// A tab gets its process on its first http(s) or file navigation and keeps it
/// for later navigations. [`TabWorkerPool::close`] ends that process when the
/// tab closes. A renderer that fails is dropped, and the next navigation in
/// that tab starts a fresh one. `anvil://` pages never start a renderer.
///
/// Every navigation gets a new navigation id from the pool, so an answer can
/// only ever be taken for the request it belongs to.
///
/// Each tab also has a [`RelaunchBudget`]. A renderer that hangs, exits or
/// misbehaves counts as a crash. Once a tab has had three in a minute, its
/// next pages show an error page without starting a renderer, until the
/// oldest crash is a minute old. A page that crashes the renderer every time
/// can't keep it restarting.
///
/// Once the first renderer starts, a helper thread gives every idle renderer
/// a heartbeat each [`HEARTBEAT_INTERVAL`]. A failed heartbeat ends that
/// renderer and counts as a crash. The thread stops when the pool is dropped.
#[derive(Debug)]
pub struct TabWorkerPool {
    command: Mutex<WorkerCommand>,
    /// The confinement decision for `command`, once it has been made.
    confinement: Mutex<Option<Arc<Confinement>>>,
    slots: Arc<Slots>,
    budgets: Arc<Budgets>,
    ws_registries: Mutex<HashMap<u64, Arc<Mutex<RendererWsRegistry>>>>,
    next_nav: AtomicU64,
    heartbeat_ms: Arc<AtomicU64>,
    /// Dropping this stops the heartbeat thread.
    heartbeat_stop: Mutex<Option<mpsc::Sender<()>>>,
    window_budget: Arc<WindowBudget>,
}

/// What a tab shows once its renderer has crashed too often.
pub const CRASH_LOOP_ERROR: &str = "the renderer for this tab keeps crashing, so it was not \
     started again. Try again in a minute";

impl Default for TabWorkerPool {
    fn default() -> Self {
        Self::new()
    }
}

impl TabWorkerPool {
    /// Renderers run this executable with [`RENDERER_ARG`] and the standard
    /// timeout.
    pub fn new() -> Self {
        Self {
            command: Mutex::new(WorkerCommand {
                program: None,
                args: vec![RENDERER_ARG.to_string()],
                timeout: worker_timeout(),
                confine: ConfineMode::Auto,
            }),
            confinement: Mutex::new(None),
            slots: Arc::new(Mutex::new(HashMap::new())),
            budgets: Arc::new(Mutex::new(HashMap::new())),
            ws_registries: Mutex::new(HashMap::new()),
            next_nav: AtomicU64::new(1),
            heartbeat_ms: Arc::new(AtomicU64::new(millis(HEARTBEAT_INTERVAL))),
            heartbeat_stop: Mutex::new(None),
            window_budget: Arc::new(WindowBudget::new()),
        }
    }

    fn ws_registry_for(&self, tab_id: u64, reset: bool) -> Arc<Mutex<RendererWsRegistry>> {
        let mut map = self
            .ws_registries
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let reg = map
            .entry(tab_id)
            .or_insert_with(|| Arc::new(Mutex::new(RendererWsRegistry::new())))
            .clone();
        if reset && let Ok(mut g) = reg.lock() {
            g.clear();
        }
        reg
    }

    /// Tell the pool which tab is in front. Its renderer is never ended to
    /// keep the window under its memory budget.
    pub fn set_focused_tab(&self, tab_id: Option<u64>) {
        if let Ok(mut focused) = self.window_budget.focused.lock() {
            *focused = tab_id;
        }
    }

    /// The tab the pool treats as in front.
    pub fn focused_tab(&self) -> Option<u64> {
        self.window_budget
            .focused
            .lock()
            .ok()
            .and_then(|focused| *focused)
    }

    /// Keep the idle renderers of this window under `bytes` together from
    /// now on. Starts at [`WINDOW_RENDERER_MEMORY_BUDGET_BYTES`]. Tests use
    /// this to make it small.
    pub fn set_memory_budget(&self, bytes: u64) {
        self.window_budget
            .memory_bytes
            .store(bytes, Ordering::Relaxed);
    }

    /// Let one page use at most `front` of CPU in the tab in front and
    /// `background` in any other, from the next page on. Start at
    /// [`DEFAULT_RENDER_CPU`] and [`BACKGROUND_RENDER_CPU`]. Tests use this
    /// to make them short.
    pub fn set_render_cpu(&self, front: Duration, background: Duration) {
        self.window_budget
            .front_cpu_ms
            .store(millis(front), Ordering::Relaxed);
        self.window_budget
            .background_cpu_ms
            .store(millis(background), Ordering::Relaxed);
    }

    /// The memory budget for this window's idle renderers, in bytes.
    pub fn memory_budget(&self) -> u64 {
        self.window_budget.memory_bytes.load(Ordering::Relaxed)
    }

    /// End idle background renderers until the idle ones fit in the
    /// window's memory budget, and return their tab ids. The heartbeat
    /// thread and every finished page do this too. Memory is only read on
    /// Linux for now, so elsewhere this ends nothing.
    pub fn enforce_memory_budget(&self) -> Vec<u64> {
        end_renderers_over_budget(&self.slots, &self.window_budget, &resident_bytes)
    }

    /// Give idle renderers a heartbeat every `interval` from now on.
    /// Tests use this to make the wait shorter.
    pub fn set_heartbeat_interval(&self, interval: Duration) {
        self.heartbeat_ms
            .store(millis(interval).max(1), Ordering::Relaxed);
    }

    /// Use `program` and `args` for renderers started from now on, with
    /// `timeout` to start and for each answer. Tests use this to point at
    /// the `anvil` binary or at a stand-in process. Such a command is
    /// started directly, until [`set_confinement`](Self::set_confinement)
    /// asks for more.
    pub fn set_command(&self, program: &Path, args: &[String], timeout: Duration) {
        if let Ok(mut command) = self.command.lock() {
            *command = WorkerCommand {
                program: Some(program.to_path_buf()),
                args: args.to_vec(),
                timeout,
                confine: ConfineMode::Off,
            };
        }
        self.forget_confinement();
    }

    /// Confine renderers started from now on as `mode` says. Renderers
    /// already running keep how they were started.
    pub fn set_confinement(&self, mode: ConfineMode) {
        let changed = self
            .command
            .lock()
            .map(|mut command| std::mem::replace(&mut command.confine, mode) != mode)
            .unwrap_or(false);
        if changed {
            self.forget_confinement();
        }
    }

    fn forget_confinement(&self) {
        if let Ok(mut decision) = self.confinement.lock() {
            *decision = None;
        }
    }

    /// How renderers are started: the decision, its reasons and the
    /// self-test result. The first call looks for the tools and runs the
    /// self-test, which takes a moment, so the window calls this once at
    /// startup on a helper thread.
    pub fn confinement(&self) -> Result<Arc<Confinement>, String> {
        let command = self
            .command
            .lock()
            .map_err(|_| "renderer settings are unavailable".to_string())?
            .clone();
        let mut decision = self
            .confinement
            .lock()
            .map_err(|_| "the confinement decision is unavailable".to_string())?;
        if let Some(decision) = decision.as_ref() {
            return Ok(Arc::clone(decision));
        }
        let program = match &command.program {
            Some(program) => program.clone(),
            None => current_anvil_exe()?,
        };
        let plain = renderer_command(&program, &command.args);
        let made = Arc::new(confine::decide(
            command.confine,
            &plain,
            &renderer_limits(command.timeout),
        ));
        *decision = Some(Arc::clone(&made));
        Ok(made)
    }

    /// Start a fresh renderer the way tab renderers are started now, ask
    /// it what the OS has put it under, and end it. It has not seen any
    /// page, so its answer can be shown.
    pub fn probe_renderer_status(&self) -> Result<anvil_sandbox::wire::RendererStatus, String> {
        let decision = self.confinement()?;
        let command = self
            .command
            .lock()
            .map_err(|_| "renderer settings are unavailable".to_string())?
            .clone();
        let program = match &command.program {
            Some(program) => program.clone(),
            None => current_anvil_exe()?,
        };
        let start = decision.command_for(&renderer_command(&program, &command.args));
        let mut process = RendererProcess::launch(&start, renderer_limits(command.timeout))
            .map_err(|err| err.to_string())?;
        let status = process.status().map_err(|err| err.to_string())?;
        process.shutdown();
        Ok(status)
    }

    /// Send `request` to the renderer for `request.tab_id`, starting it if
    /// needed. Each fetch the renderer asks for on the way goes to `fetch`.
    ///
    /// A failed exchange ends that renderer, unless the request was refused
    /// before it was sent. The error text never contains the document that
    /// was sent.
    pub fn exchange(
        &self,
        request: &TabWorkerRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    ) -> Result<TabWorkerResponse, String> {
        let mut no_script = |want: &ScriptFetchWireRequest| ScriptFetchWireReply {
            nav_id: want.nav_id,
            status: 0,
            final_url: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        self.exchange_with_script_fetch(request, fetch, &mut no_script)
    }

    /// Render `request` in `request.tab_id`'s renderer forwarding both regular
    /// subresource fetches and rich `fetch()` / `XMLHttpRequest` calls.
    pub fn exchange_with_script_fetch(
        &self,
        request: &TabWorkerRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
    ) -> Result<TabWorkerResponse, String> {
        let command = self
            .command
            .lock()
            .map_err(|_| "renderer settings are unavailable".to_string())?
            .clone();
        let slot = self.slot(request.tab_id)?;
        let mut guard = slot
            .lock()
            .map_err(|_| "the renderer is unavailable".to_string())?;
        if let Some(process) = guard.as_mut() {
            // Between pages: it must still answer and must not have been busy.
            if process.heartbeat().is_err() {
                *guard = None;
                self.record_crash(request.tab_id);
            }
        }
        if guard.is_none() {
            if !self.may_launch(request.tab_id) {
                return Err(CRASH_LOOP_ERROR.to_string());
            }
            let program = match &command.program {
                Some(program) => program.clone(),
                None => current_anvil_exe()?,
            };
            let decision = self.confinement()?;
            let start = decision.command_for(&renderer_command(&program, &command.args));
            let confined_by = decision.in_use.as_ref().map(|helper| helper.kind);
            match TabProcess::start_command(&start, command.timeout, confined_by) {
                Ok(process) => {
                    *guard = Some(process);
                    self.start_heartbeat();
                }
                Err(failure) => {
                    if failure.is_crash() {
                        self.record_crash(request.tab_id);
                    }
                    return Err(failure.to_string());
                }
            }
        }
        let nav_id = self.next_nav.fetch_add(1, Ordering::Relaxed);
        let render_cpu = self.window_budget.render_cpu(request.tab_id);
        let parsed_doc_csp = anvil_dom::parse_html(&request.html).meta_csp();
        let combined_csp = match (request.csp_header.as_deref(), parsed_doc_csp.as_deref()) {
            (Some(h), Some(m)) => ContentSecurityPolicy::parse(&format!("{h}; {m}")),
            (Some(h), None) => ContentSecurityPolicy::parse(h),
            (None, Some(m)) => ContentSecurityPolicy::parse(m),
            (None, None) => ContentSecurityPolicy::default(),
        };
        let ws_reg = self.ws_registry_for(request.tab_id, true);
        let page_url = request.url.clone();
        let mut ws_bridge = |cmd: &WsWireCommand| {
            ws_reg.lock().unwrap_or_else(|err| err.into_inner()).handle(
                cmd,
                &page_url,
                &combined_csp,
            )
        };
        let result = match guard.as_mut() {
            Some(process) => {
                process.set_render_cpu(render_cpu);
                process.exchange_with_ipc(request, nav_id, fetch, script_fetch, &mut ws_bridge)
            }
            None => Err("the renderer did not start".to_string()),
        };
        if result.is_err() && guard.as_mut().is_some_and(|process| !process.is_running()) {
            *guard = None;
            self.record_crash(request.tab_id);
        }
        drop(guard);
        if let Ok(mut last_used) = self.window_budget.last_used.lock() {
            last_used.insert(request.tab_id, Instant::now());
        }
        // The tab may have closed while this page was loading.
        let still_open = self
            .slots
            .lock()
            .map(|slots| {
                slots
                    .get(&request.tab_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &slot))
            })
            .unwrap_or(false);
        if !still_open {
            if let Ok(mut guard) = slot.lock() {
                *guard = None;
            }
            if let Ok(mut budgets) = self.budgets.lock() {
                budgets.remove(&request.tab_id);
            }
            self.window_budget.forget(request.tab_id);
        }
        self.enforce_memory_budget();
        result
    }

    /// Re-stage an already-loaded document in `tab_id`'s live renderer at `(width, height)`.
    /// Returns `Ok(None)` if the tab has no running worker holding that document.
    pub fn resize_tab(
        &self,
        tab_id: u64,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Option<Vec<u8>>, String> {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&tab_id).cloned());
        let Some(slot) = slot else {
            return Ok(None);
        };
        let mut guard = match slot.try_lock() {
            Ok(g) => g,
            Err(_) => return Ok(None),
        };
        let Some(process) = guard.as_mut() else {
            return Ok(None);
        };
        if !process.is_running() {
            *guard = None;
            return Ok(None);
        }
        process.set_render_cpu(self.window_budget.render_cpu(tab_id));
        let res = process.resize(tab_id, viewport_width, viewport_height);
        if res.is_err() && !process.is_running() {
            *guard = None;
        } else if let Ok(mut last_used) = self.window_budget.last_used.lock() {
            last_used.insert(tab_id, Instant::now());
        }
        res
    }

    /// Forward an interactive input/scroll/timer request to `tab_id`'s running renderer.
    /// Returns `Ok(None)` if no live worker holds a page for this tab.
    pub fn interact_tab(
        &self,
        tab_id: u64,
        request: InteractRequest,
        page_url: &str,
        csp: &ContentSecurityPolicy,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
        script_fetch: &mut dyn FnMut(&ScriptFetchWireRequest) -> ScriptFetchWireReply,
    ) -> Result<Option<InteractFrameUpdate>, String> {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&tab_id).cloned());
        let Some(slot) = slot else {
            return Ok(None);
        };
        let mut guard = match slot.try_lock() {
            Ok(g) => g,
            Err(_) => return Ok(None),
        };
        let Some(process) = guard.as_mut() else {
            return Ok(None);
        };
        if !process.is_running() {
            *guard = None;
            return Ok(None);
        }
        let ws_reg = self.ws_registry_for(tab_id, false);
        let mut ws_bridge = |cmd: &WsWireCommand| {
            ws_reg
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .handle(cmd, page_url, csp)
        };
        process.set_render_cpu(self.window_budget.render_cpu(tab_id));
        let res = process.interact_with_ipc(request, fetch, script_fetch, &mut ws_bridge);
        if res.is_err() && !process.is_running() {
            *guard = None;
        } else if let Ok(mut last_used) = self.window_budget.last_used.lock() {
            last_used.insert(tab_id, Instant::now());
        }
        res
    }

    /// End the renderer for `tab_id` because it sent something only a
    /// broken renderer sends, and count that as a crash.
    pub fn end_broken(&self, tab_id: u64) {
        if let Ok(mut map) = self.ws_registries.lock() {
            map.remove(&tab_id);
        }
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|slots| slots.get(&tab_id).cloned());
        if let Some(slot) = slot
            && let Ok(mut guard) = slot.lock()
        {
            *guard = None;
        }
        self.record_crash(tab_id);
    }

    /// End the renderer for `tab_id`, if it has one, and forget its crashes.
    pub fn close(&self, tab_id: u64) {
        if let Ok(mut map) = self.ws_registries.lock() {
            map.remove(&tab_id);
        }
        if let Ok(mut budgets) = self.budgets.lock() {
            budgets.remove(&tab_id);
        }
        self.window_budget.forget(tab_id);
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|mut slots| slots.remove(&tab_id));
        if let Some(slot) = slot {
            // A load in progress holds the lock. It drops the renderer when it finishes.
            if let Ok(mut guard) = slot.try_lock() {
                *guard = None;
            }
        }
    }

    /// End every renderer.
    pub fn close_all(&self) {
        let ids: Vec<u64> = self
            .slots
            .lock()
            .map(|slots| slots.keys().copied().collect())
            .unwrap_or_default();
        for id in ids {
            self.close(id);
        }
    }

    /// Process id of the running renderer for `tab_id`.
    ///
    /// `None` while another thread holds that renderer, for a page or for
    /// a heartbeat, so a caller that needs to know whether it still runs
    /// asks again a moment later.
    pub fn worker_pid(&self, tab_id: u64) -> Option<u32> {
        let slot = self.slots.lock().ok()?.get(&tab_id).cloned()?;
        let mut guard = slot.try_lock().ok()?;
        let process = guard.as_mut()?;
        if process.is_running() {
            Some(process.pid())
        } else {
            None
        }
    }

    /// Every running tab renderer: its tab id, process id, and the tool it
    /// was started through. A renderer busy with a page is left out.
    pub fn running_renderers(&self) -> Vec<(u64, u32, Option<ToolKind>)> {
        let slots: Vec<(u64, TabSlot)> = self
            .slots
            .lock()
            .map(|slots| {
                slots
                    .iter()
                    .map(|(id, slot)| (*id, Arc::clone(slot)))
                    .collect()
            })
            .unwrap_or_default();
        let mut running: Vec<(u64, u32, Option<ToolKind>)> = slots
            .into_iter()
            .filter_map(|(id, slot)| {
                let mut guard = slot.try_lock().ok()?;
                let process = guard.as_mut()?;
                process
                    .is_running()
                    .then(|| (id, process.pid(), process.confined_by()))
            })
            .collect();
        running.sort_unstable_by_key(|(id, _, _)| *id);
        running
    }

    /// Number of tabs with a running renderer.
    pub fn running_workers(&self) -> usize {
        let ids: Vec<u64> = self
            .slots
            .lock()
            .map(|slots| slots.keys().copied().collect())
            .unwrap_or_default();
        ids.into_iter()
            .filter(|id| self.worker_pid(*id).is_some())
            .count()
    }

    /// True when tab `tab_id` may start a renderer now.
    fn may_launch(&self, tab_id: u64) -> bool {
        self.budgets
            .lock()
            .map(|mut budgets| {
                budgets
                    .get_mut(&tab_id)
                    .is_none_or(|budget| budget.may_launch(Instant::now()))
            })
            .unwrap_or(false)
    }

    /// Count a crash against tab `tab_id`.
    fn record_crash(&self, tab_id: u64) {
        record_crash(&self.budgets, tab_id);
    }

    /// Start the heartbeat thread, unless it is already running.
    fn start_heartbeat(&self) {
        let Ok(mut stop) = self.heartbeat_stop.lock() else {
            return;
        };
        if stop.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let slots = Arc::downgrade(&self.slots);
        let budgets = Arc::downgrade(&self.budgets);
        let interval = Arc::clone(&self.heartbeat_ms);
        let window_budget = Arc::downgrade(&self.window_budget);
        let started = thread::Builder::new()
            .name("renderer-heartbeat".to_string())
            .spawn(move || heartbeat_loop(&slots, &budgets, &window_budget, &interval, &rx));
        if started.is_ok() {
            *stop = Some(tx);
        }
    }

    fn slot(&self, tab_id: u64) -> Result<TabSlot, String> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| "the renderer table is unavailable".to_string())?;
        Ok(Arc::clone(slots.entry(tab_id).or_default()))
    }
}

impl Drop for TabWorkerPool {
    fn drop(&mut self) {
        if let Ok(mut stop) = self.heartbeat_stop.lock() {
            *stop = None;
        }
        self.close_all();
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn record_crash(budgets: &Budgets, tab_id: u64) {
    if let Ok(mut budgets) = budgets.lock() {
        budgets
            .entry(tab_id)
            .or_default()
            .record_crash(Instant::now());
    }
}

/// Give every idle renderer a heartbeat each interval, until the pool is
/// dropped, then hold the idle ones to the window memory budget. A tab
/// that is loading a page holds its slot, and that load has its own
/// deadline, so it is skipped.
fn heartbeat_loop(
    slots: &Weak<Slots>,
    budgets: &Weak<Budgets>,
    window_budget: &Weak<WindowBudget>,
    interval_ms: &AtomicU64,
    stop: &mpsc::Receiver<()>,
) {
    loop {
        let interval = Duration::from_millis(interval_ms.load(Ordering::Relaxed).max(1));
        if !matches!(
            stop.recv_timeout(interval),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return;
        }
        let Some(table) = slots.upgrade() else {
            return;
        };
        let tabs: Vec<(u64, TabSlot)> = match table.lock() {
            Ok(table) => table
                .iter()
                .map(|(id, slot)| (*id, Arc::clone(slot)))
                .collect(),
            Err(_) => return,
        };
        drop(table);
        for (tab_id, slot) in tabs {
            let Ok(mut guard) = slot.try_lock() else {
                continue;
            };
            let Some(process) = guard.as_mut() else {
                continue;
            };
            if process.heartbeat().is_err() {
                *guard = None;
                if let Some(budgets) = budgets.upgrade() {
                    record_crash(&budgets, tab_id);
                }
            }
        }
        if let (Some(table), Some(window_budget)) = (slots.upgrade(), window_budget.upgrade()) {
            end_renderers_over_budget(&table, &window_budget, &resident_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_sandbox::wire::{self, Message, RenderFailure, RenderedDocument};

    fn request() -> TabWorkerRequest {
        TabWorkerRequest {
            tab_id: 3,
            url: "https://example.test/".to_string(),
            html: "<p>hi</p>".to_string(),
            scripts_enabled: true,
            extended_script_reach: false,
            csp_header: None,
            viewport_width: 0,
            viewport_height: 100_000,
            https_only: true,
            http_allowed_hosts: vec![
                "intranet.test".to_string(),
                "localhost:8080".to_string(),
                String::new(),
            ],
        }
    }

    #[test]
    fn a_request_carries_its_navigation_and_fits_the_wire_caps() {
        let request_for_wire = render_request(&request(), 42).unwrap();
        assert_eq!(request_for_wire.tab_id, 3);
        assert_eq!(request_for_wire.nav_id, 42);
        assert_eq!(request_for_wire.viewport_width, 1);
        assert_eq!(request_for_wire.viewport_height, MAX_VIEWPORT_SIDE);
        // Exceptions the wire can't carry are left out, never widened.
        assert_eq!(
            request_for_wire.http_allowed_hosts,
            vec!["intranet.test".to_string()]
        );
        assert!(wire::encode(0, &Message::Render(request_for_wire)).is_ok());

        let mut long = request();
        long.url = format!("https://example.test/{}", "a".repeat(MAX_URL_BYTES));
        assert!(render_request(&long, 1).is_err());
        let mut big = request();
        big.html = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        assert!(render_request(&big, 1).is_err());
    }

    fn idle(tab_id: u64, mib: u64, last_used: Option<Instant>) -> IdleRenderer {
        IdleRenderer {
            tab_id,
            resident_bytes: mib * 1024 * 1024,
            last_used,
        }
    }

    #[test]
    fn the_window_budget_ends_background_tabs_left_alone_longest_first() {
        let start = Instant::now();
        let at = |secs| Some(start + Duration::from_secs(secs));
        let budget = 300 * 1024 * 1024;
        let renderers = [
            idle(1, 100, at(30)),
            idle(2, 100, at(10)),
            idle(3, 100, at(20)),
            idle(4, 100, at(5)),
        ];
        // 400 MiB in 300: one has to go. Tab 4 is the stalest but in front.
        assert_eq!(renderers_to_end(&renderers, Some(4), budget), vec![2]);
        // With nothing in front the stalest goes.
        assert_eq!(renderers_to_end(&renderers, None, budget), vec![4]);
        // Two have to go for 200 MiB.
        assert_eq!(
            renderers_to_end(&renderers, Some(4), 200 * 1024 * 1024),
            vec![2, 3]
        );
        // Under the budget nothing goes.
        assert!(renderers_to_end(&renderers, Some(4), 400 * 1024 * 1024).is_empty());
        // One that never finished a page counts as left alone longest.
        let fresh = [idle(1, 200, at(1)), idle(2, 200, None)];
        assert_eq!(renderers_to_end(&fresh, None, budget), vec![2]);
    }

    #[test]
    fn the_tab_in_front_is_never_ended_for_the_window_budget() {
        let start = Instant::now();
        let alone = [idle(7, 900, Some(start))];
        assert!(renderers_to_end(&alone, Some(7), 1).is_empty());
        let pair = [idle(7, 900, Some(start)), idle(8, 1, None)];
        // Ending everything else still isn't enough, and that is all it does.
        assert_eq!(renderers_to_end(&pair, Some(7), 1), vec![8]);
        assert_eq!(
            WINDOW_RENDERER_MEMORY_BUDGET_BYTES,
            4 * MANDATORY_TAB_MEMORY_LIMIT_BYTES as u64
        );
    }

    #[test]
    fn the_published_window_budget_and_page_cpu_caps_hold() {
        assert_eq!(MANDATORY_TAB_MEMORY_LIMIT_BYTES, 1024 * 1024 * 1024);
        assert_eq!(WINDOW_RENDERER_MEMORY_BUDGET_BYTES, 4096 * 1024 * 1024);
        assert_eq!(
            WINDOW_RENDERER_MEMORY_BUDGET_BYTES,
            4 * MANDATORY_TAB_MEMORY_LIMIT_BYTES as u64
        );
        assert_eq!(DEFAULT_RENDER_CPU, Duration::from_secs(20));
        assert_eq!(BACKGROUND_RENDER_CPU, Duration::from_secs(10));

        let budget = WindowBudget::new();
        assert_eq!(budget.render_cpu(1), DEFAULT_RENDER_CPU);
        *budget.focused.lock().unwrap() = Some(1);
        assert_eq!(budget.render_cpu(1), DEFAULT_RENDER_CPU);
        assert_eq!(budget.render_cpu(2), BACKGROUND_RENDER_CPU);

        let pool = TabWorkerPool::new();
        assert_eq!(pool.memory_budget(), WINDOW_RENDERER_MEMORY_BUDGET_BYTES);
    }

    #[test]
    fn a_refusal_shows_a_fixed_text_not_the_renderers() {
        let refused = RenderOutcome::Refused(RenderFailure {
            tab_id: 3,
            nav_id: 1,
            code: FailureCode::Internal,
            detail: "<script>steal()</script>".to_string(),
        });
        let resp = into_response(refused, 3);
        assert!(!resp.ok);
        assert!(resp.html.is_empty());
        let error = resp.error.unwrap();
        assert!(!error.contains("steal"));
        assert_eq!(error, "the renderer could not finish the page");

        let done = RenderOutcome::Done(
            RenderedDocument {
                tab_id: 3,
                nav_id: 1,
                title: "t".to_string(),
                dom_nodes: 5,
                blocked_resources: 2,
                html: "<p>ok</p>".to_string(),
                final_url: String::new(),
                animating: false,
                has_next_timer: false,
                next_timer_ms_bits: 0,
                spa_back_depth: 0,
                spa_forward_depth: 0,
            },
            None,
        );
        let resp = into_response(done, 3);
        assert!(resp.ok);
        assert_eq!(resp.dom_nodes, 5);
        assert_eq!(resp.blocked_csp_resources, 2);
        assert_eq!(resp.html, "<p>ok</p>");
        assert_eq!(resp.frame, None);
    }
}
