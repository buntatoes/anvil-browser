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
//! The window keeps running. Closing a tab ends its renderer. `anvil://`
//! pages do not use this module.
//!
//! [`exchange_tab_worker`] is the one-document form used by the `render` and
//! `raster` commands. The session cookie jar is never part of a request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anvil_sandbox::launcher::{RenderOutcome, RendererCommand, RendererLimits, RendererProcess};
use anvil_sandbox::wire::{
    FailureCode, FetchReply, FetchRequest, MAX_ALLOWED_HOSTS, MAX_CSP_BYTES, MAX_DOCUMENT_BYTES,
    MAX_HOST_BYTES, MAX_URL_BYTES, MAX_VIEWPORT_SIDE, RenderRequest,
};
use anvil_sandbox::{MANDATORY_WORKER_TIMEOUT_MS, TabWorkerRequest, TabWorkerResponse};

/// The argument that starts `anvil` as a renderer.
pub const RENDERER_ARG: &str = "--renderer";

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
        viewport_width: side(request.viewport_width),
        viewport_height: side(request.viewport_height),
        https_only: request.https_only,
        http_allowed_hosts,
    })
}

/// The renderer doesn't ask the browser for subresources yet. Until it does,
/// any request it makes is answered as a failed fetch.
fn refuse_fetch(want: &FetchRequest) -> FetchReply {
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
        RenderOutcome::Done(doc) => TabWorkerResponse {
            tab_id: doc.tab_id,
            ok: true,
            title: doc.title,
            dom_nodes: doc.dom_nodes as usize,
            blocked_csp_resources: doc.blocked_resources as usize,
            error: None,
            html: doc.html,
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
            }
        }
    }
}

/// Start `program` with `args` as a renderer, send `request`, and wait up
/// to `timeout` for it to start and again for its answer.
///
/// A timeout, a start failure, a bad frame, or a crash is `Err`, and the
/// process is gone. `Ok` is the renderer's answer, including `ok: false`.
/// The caller must not paint `html` unless `ok` is true.
pub fn exchange_tab_worker(
    program: &Path,
    args: &[String],
    request: &TabWorkerRequest,
    timeout: Duration,
) -> Result<TabWorkerResponse, String> {
    let wire_request = render_request(request, 1)?;
    let mut process =
        RendererProcess::launch(&renderer_command(program, args), renderer_limits(timeout))
            .map_err(|err| err.to_string())?;
    let outcome = process
        .render(&wire_request, &mut refuse_fetch)
        .map_err(|err| err.to_string())?;
    process.shutdown();
    Ok(into_response(outcome, request.tab_id))
}

/// One long-lived renderer process that belongs to one tab.
///
/// The window sends one `Render` per navigation and reads one answer. The
/// launcher writes and reads frames on helper threads, so a renderer that
/// stops reading or stops answering can't block the window past the
/// deadline. A renderer that times out, exits, or writes a bad frame is
/// killed and is not used again. Dropping a `TabProcess` ends the process.
#[derive(Debug)]
pub struct TabProcess {
    process: RendererProcess,
    served: u64,
}

impl TabProcess {
    /// Start `program` with `args` and wait up to `timeout` for its hello.
    pub fn spawn(program: &Path, args: &[String], timeout: Duration) -> Result<Self, String> {
        let process =
            RendererProcess::launch(&renderer_command(program, args), renderer_limits(timeout))
                .map_err(|err| err.to_string())?;
        Ok(Self { process, served: 0 })
    }

    /// Process id of the renderer.
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Number of documents this renderer has answered.
    pub fn served(&self) -> u64 {
        self.served
    }

    /// True while the process is running and has not misbehaved.
    pub fn is_running(&mut self) -> bool {
        self.process.is_alive()
    }

    /// Send `request` as navigation `nav_id` and wait for the answer.
    ///
    /// After an `Err`, check [`is_running`](Self::is_running): a request the
    /// window could not send leaves the renderer as it was, and anything
    /// else means the renderer has been killed.
    pub fn exchange(
        &mut self,
        request: &TabWorkerRequest,
        nav_id: u64,
    ) -> Result<TabWorkerResponse, String> {
        let wire_request = render_request(request, nav_id)?;
        let outcome = self
            .process
            .render(&wire_request, &mut refuse_fetch)
            .map_err(|err| err.to_string())?;
        self.served += 1;
        Ok(into_response(outcome, request.tab_id))
    }
}

/// The program and arguments used to start a tab's renderer.
#[derive(Clone, Debug)]
struct WorkerCommand {
    program: Option<PathBuf>,
    args: Vec<String>,
    timeout: Duration,
}

/// One slot per tab. The inner lock keeps one navigation per tab at a time.
type TabSlot = Arc<Mutex<Option<TabProcess>>>;

/// Renderers for one window, keyed by tab id.
///
/// A tab gets its process on its first http(s) or file navigation and keeps it
/// for later navigations. [`TabWorkerPool::close`] ends that process when the
/// tab closes. A renderer that fails is dropped, and the next navigation in
/// that tab starts a fresh one. `anvil://` pages never start a renderer.
///
/// Every navigation gets a new navigation id from the pool, so an answer can
/// only ever be taken for the request it belongs to.
#[derive(Debug)]
pub struct TabWorkerPool {
    command: Mutex<WorkerCommand>,
    slots: Mutex<HashMap<u64, TabSlot>>,
    next_nav: AtomicU64,
}

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
            }),
            slots: Mutex::new(HashMap::new()),
            next_nav: AtomicU64::new(1),
        }
    }

    /// Use `program` and `args` for renderers started from now on, with
    /// `timeout` to start and for each answer. Tests use this to point at
    /// the `anvil` binary or at a stand-in process.
    pub fn set_command(&self, program: &Path, args: &[String], timeout: Duration) {
        if let Ok(mut command) = self.command.lock() {
            *command = WorkerCommand {
                program: Some(program.to_path_buf()),
                args: args.to_vec(),
                timeout,
            };
        }
    }

    /// Send `request` to the renderer for `request.tab_id`, starting it if
    /// needed.
    ///
    /// A failed exchange ends that renderer, unless the request was refused
    /// before it was sent. The error text never contains the document that
    /// was sent.
    pub fn exchange(&self, request: &TabWorkerRequest) -> Result<TabWorkerResponse, String> {
        let command = self
            .command
            .lock()
            .map_err(|_| "renderer settings are unavailable".to_string())?
            .clone();
        let slot = self.slot(request.tab_id)?;
        let mut guard = slot
            .lock()
            .map_err(|_| "the renderer is unavailable".to_string())?;
        if guard.as_mut().is_some_and(|process| !process.is_running()) {
            *guard = None;
        }
        if guard.is_none() {
            let program = match &command.program {
                Some(program) => program.clone(),
                None => current_anvil_exe()?,
            };
            *guard = Some(TabProcess::spawn(&program, &command.args, command.timeout)?);
        }
        let nav_id = self.next_nav.fetch_add(1, Ordering::Relaxed);
        let result = match guard.as_mut() {
            Some(process) => process.exchange(request, nav_id),
            None => Err("the renderer did not start".to_string()),
        };
        if result.is_err() && guard.as_mut().is_some_and(|process| !process.is_running()) {
            *guard = None;
        }
        drop(guard);
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
        }
        result
    }

    /// End the renderer for `tab_id`, if it has one.
    pub fn close(&self, tab_id: u64) {
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
        self.close_all();
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

        let done = RenderOutcome::Done(RenderedDocument {
            tab_id: 3,
            nav_id: 1,
            title: "t".to_string(),
            dom_nodes: 5,
            blocked_resources: 2,
            html: "<p>ok</p>".to_string(),
        });
        let resp = into_response(done, 3);
        assert!(resp.ok);
        assert_eq!(resp.dom_nodes, 5);
        assert_eq!(resp.blocked_csp_resources, 2);
        assert_eq!(resp.html, "<p>ok</p>");
    }
}
