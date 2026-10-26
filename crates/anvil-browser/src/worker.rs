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
//! [`exchange_tab_worker`] is the one-document form used by the `render` and
//! `raster` commands. The session cookie jar is never part of a request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anvil_sandbox::confine::{self, ConfineMode, Confinement, ToolKind};
use anvil_sandbox::launcher::{
    RelaunchBudget, RenderOutcome, RendererCommand, RendererFailure, RendererLimits,
    RendererProcess,
};
use anvil_sandbox::wire::{
    FailureCode, FetchReply, FetchRequest, MAX_ALLOWED_HOSTS, MAX_CSP_BYTES, MAX_DOCUMENT_BYTES,
    MAX_HOST_BYTES, MAX_URL_BYTES, MAX_VIEWPORT_SIDE, RenderRequest,
};
use anvil_sandbox::{MANDATORY_WORKER_TIMEOUT_MS, TabWorkerRequest, TabWorkerResponse};

/// The argument that starts `anvil` as a renderer.
pub const RENDERER_ARG: &str = "--renderer";

/// How often an idle renderer gets a heartbeat.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

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
    let wire_request = render_request(request, 1)?;
    let mut process =
        RendererProcess::launch(&renderer_command(program, args), renderer_limits(timeout))
            .map_err(|err| err.to_string())?;
    let outcome = process
        .render(&wire_request, fetch)
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
    ///
    /// After an `Err`, check [`is_running`](Self::is_running): a request the
    /// window could not send leaves the renderer as it was, and anything
    /// else means the renderer has been killed.
    pub fn exchange(
        &mut self,
        request: &TabWorkerRequest,
        nav_id: u64,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    ) -> Result<TabWorkerResponse, String> {
        let wire_request = render_request(request, nav_id)?;
        let outcome = self
            .process
            .render(&wire_request, fetch)
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
    confine: ConfineMode,
}

/// One slot per tab. The inner lock keeps one navigation per tab at a time.
type TabSlot = Arc<Mutex<Option<TabProcess>>>;

/// Every tab's slot, keyed by tab id.
type Slots = Mutex<HashMap<u64, TabSlot>>;

/// Every tab's crash count, keyed by tab id.
type Budgets = Mutex<HashMap<u64, RelaunchBudget>>;

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
    next_nav: AtomicU64,
    heartbeat_ms: Arc<AtomicU64>,
    /// Dropping this stops the heartbeat thread.
    heartbeat_stop: Mutex<Option<mpsc::Sender<()>>>,
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
            next_nav: AtomicU64::new(1),
            heartbeat_ms: Arc::new(AtomicU64::new(millis(HEARTBEAT_INTERVAL))),
            heartbeat_stop: Mutex::new(None),
        }
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
        let result = match guard.as_mut() {
            Some(process) => process.exchange(request, nav_id, fetch),
            None => Err("the renderer did not start".to_string()),
        };
        if result.is_err() && guard.as_mut().is_some_and(|process| !process.is_running()) {
            *guard = None;
            self.record_crash(request.tab_id);
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
            if let Ok(mut budgets) = self.budgets.lock() {
                budgets.remove(&request.tab_id);
            }
        }
        result
    }

    /// End the renderer for `tab_id`, if it has one, and forget its crashes.
    pub fn close(&self, tab_id: u64) {
        if let Ok(mut budgets) = self.budgets.lock() {
            budgets.remove(&tab_id);
        }
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
        let started = thread::Builder::new()
            .name("renderer-heartbeat".to_string())
            .spawn(move || heartbeat_loop(&slots, &budgets, &interval, &rx));
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
/// dropped. A tab that is loading a page holds its slot, and that load has
/// its own deadline, so it is skipped.
fn heartbeat_loop(
    slots: &Weak<Slots>,
    budgets: &Weak<Budgets>,
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
