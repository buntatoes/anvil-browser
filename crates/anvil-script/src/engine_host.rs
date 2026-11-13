// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Live DOM, `Storage`, `Location`, `Event`, and `console` host implementation
//! ([`CinderDomHost`]) for the `cinder` bytecode virtual machine ([`cinder::vm::Vm`]).

use std::any::Any;
use std::collections::VecDeque;

use crate::cinder;
use anvil_dom::{
    Document, FileSelectionRef, HostSubmission, NodeData, NodeId, ReadyState, SelectionLookup,
    SnapshotError, SubmissionSnapshot, SubmissionUrls, SubmitDispatch, parse_html,
};
use cinder::gc::roots::Persistent;
use cinder::value::object::{HostData, ObjectClass};
use cinder::value::{ObjectRef, Value as JsValue};
use cinder::vm::{Host, Job, NativeCx, NativeFn, RunError, Vm};

use crate::{
    camel_to_kebab, kebab_to_camel, read_inline_style, remove_inline_style_prop,
    update_inline_style,
};

const TAG_MASK: u64 = 0xF000_0000_0000_0000;
const TAG_NODE: u64 = 0x1000_0000_0000_0000;
const TAG_CLASS_LIST: u64 = 0x2000_0000_0000_0000;
const TAG_STYLE: u64 = 0x3000_0000_0000_0000;
const TAG_DATASET: u64 = 0x4000_0000_0000_0000;

/// Live WebSocket event yielded by [`WebSocketConnection::poll_events`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostWsEvent {
    MessageText(String),
    MessageBinary(Vec<u8>),
    Closed { code: u16, reason: String },
    Error(String),
}

/// Host-supplied WebSocket connection interface (implemented by `anvil-browser` over `anvil-net`).
pub trait WebSocketConnection: Send {
    fn ready_state(&self) -> u8;
    fn protocol(&self) -> Option<&str>;
    fn send_text(&mut self, text: &str) -> Result<(), String>;
    fn send_binary(&mut self, data: &[u8]) -> Result<(), String>;
    fn close(&mut self, code: u16, reason: &str) -> Result<(), String>;
    fn poll_events(&mut self) -> Vec<HostWsEvent>;
}

impl std::fmt::Debug for dyn WebSocketConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSocketConnection")
            .field("ready_state", &self.ready_state())
            .finish()
    }
}

/// Callback invoked when JavaScript opens `new WebSocket(url, protocols)`.
pub type WebSocketConnectFn =
    dyn Fn(&str, &str, &[String]) -> Result<Box<dyn WebSocketConnection>, String> + Send + Sync;

#[derive(Clone, Default)]
pub(crate) struct WebSocketHandler(pub(crate) Option<std::sync::Arc<WebSocketConnectFn>>);

impl std::fmt::Debug for WebSocketHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSocketHandler")
            .field("set", &self.0.is_some())
            .finish()
    }
}

/// Maximum per-origin `IndexedDB` serialized storage size (2 MiB).
pub const MAX_IDB_ORIGIN_BYTES: usize = 2 * 1024 * 1024;
/// Maximum `IndexedDB` databases per origin.
pub const MAX_IDB_DATABASES_PER_ORIGIN: usize = 16;
/// Maximum concurrent open `WebSocket` connections per document host.
const MAX_HOST_WEBSOCKETS: usize = 32;

/// Page fetch callback. Debug hides the closure.
#[derive(Clone, Default)]
pub(crate) struct FetchHandler(
    pub(crate) Option<std::sync::Arc<dyn Fn(&str, &str) -> crate::ScriptFetchReply + Send + Sync>>,
    pub(crate)  Option<
        std::sync::Arc<
            dyn Fn(&crate::ScriptFetchRequest) -> crate::ScriptFetchResponse + Send + Sync,
        >,
    >,
);

impl std::fmt::Debug for FetchHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FetchHandler")
            .field("set", &(self.0.is_some() || self.1.is_some()))
            .finish()
    }
}

/// Picker registry. The host does not mint file refs.
#[derive(Clone, Default)]
pub(crate) struct SelectionHook(
    pub(crate) Option<std::sync::Arc<dyn SelectionLookup + Send + Sync>>,
);

impl std::fmt::Debug for SelectionHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectionHook")
            .field("set", &self.0.is_some())
            .finish()
    }
}

impl PartialEq for SelectionHook {
    fn eq(&self, other: &Self) -> bool {
        self.0.is_some() == other.0.is_some()
    }
}

pub(crate) struct EmptySelection;

impl SelectionLookup for EmptySelection {
    fn current(&self, _document: u64, _control: NodeId) -> Option<FileSelectionRef> {
        None
    }
}

/// Drops a picker ref whose document or control is not the pair that was asked
/// for. The DOM helper stores whatever this returns. The platform still checks
/// document, control, and version against the registry, not the control name.
pub(crate) struct CheckedSelection<'a> {
    pub(crate) inner: &'a dyn SelectionLookup,
}

impl SelectionLookup for CheckedSelection<'_> {
    fn current(&self, document: u64, control: NodeId) -> Option<FileSelectionRef> {
        let found = self.inner.current(document, control)?;
        if found.document == document && found.control == control {
            Some(found)
        } else {
            None
        }
    }
}

/// One virtual timer on the register host.
#[derive(Debug)]
pub struct HostTimer {
    pub id: u32,
    pub due: f64,
    pub interval: Option<f64>,
    pub callback: Persistent,
    pub is_raf: bool,
    pub task: u64,
    /// Insertion order shared with the interpreter queue.
    pub seq: u64,
    /// Nonzero while queued by a timer callback. Dropped if that callback exhausts.
    pub spawn: u64,
}

impl HostTimer {
    fn snapshot(&self) -> Self {
        Self {
            id: self.id,
            due: self.due,
            interval: self.interval,
            callback: self.callback,
            is_raf: self.is_raf,
            task: self.task,
            seq: self.seq,
            spawn: self.spawn,
        }
    }
}

/// Next interval due. One step, never a catch-up burst, always strictly after `now`.
pub(crate) fn next_interval_due(prev_due: f64, period: f64, now: f64) -> f64 {
    let period = if period.is_finite() {
        period.max(1.0)
    } else {
        1.0
    };
    let stepped = prev_due + period;
    if stepped.is_finite() && stepped > now {
        stepped
    } else {
        now + period
    }
}

/// A host microtask and the page task that queued it.
struct OwnedHostJob {
    task: u64,
    spawn: u64,
    job: Job,
}

impl std::fmt::Debug for OwnedHostJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedHostJob")
            .field("task", &self.task)
            .finish()
    }
}

/// Microtask jobs waiting on one register host. A job past this is dropped.
const MAX_HOST_JOBS: usize = 8_192;
/// `addEventListener` callbacks kept for one document. A later one is dropped.
const MAX_HOST_LISTENERS: usize = 1_024;
/// Timers kept for one document. A later one is dropped.
const MAX_HOST_TIMERS: usize = 128;
/// Mutation observers kept for one document.
const MAX_HOST_OBSERVERS: usize = 256;

/// Queued `import(specifier)` request waiting for the host module loader checkpoint.
#[derive(Clone, Debug)]
pub struct PendingDynamicImport {
    pub specifier: String,
    pub referrer_module_id: Option<u64>,
    pub referrer_url: String,
    pub type_attr: Option<String>,
    pub resolve: JsValue,
    pub reject: JsValue,
    pub task: u64,
    pub spawn: u64,
}

/// Instantiated/evaluating module awaited by a dynamic `import(specifier)` call.
#[derive(Clone, Debug)]
pub struct WaitingDynamicImport {
    pub module_id: u64,
    pub resolve: JsValue,
    pub reject: JsValue,
    pub task: u64,
    pub spawn: u64,
}

/// Live DOM, `localStorage`, `sessionStorage`, `location`, event listener, and
/// microtask host for [`cinder::vm::Vm`].
#[derive(Debug)]
pub struct CinderDomHost {
    /// Live HTML document mutated directly by JavaScript DOM operations.
    pub doc: Document,
    /// FIFO microtask queue for `Promise` reactions and `queueMicrotask`.
    /// Each entry records the task that queued it.
    jobs: VecDeque<OwnedHostJob>,
    /// Task id stamped on jobs queued from now on.
    pub owner_task: u64,
    /// Insertion counter copied from the page runtime around each host turn.
    pub schedule_seq: u64,
    /// Spawn id of the timer callback currently running, or 0.
    pub timer_spawn: u64,
    /// Callbacks this host timer drain may still run.
    pub timer_callback_budget: u32,
    /// How many callbacks the last [`Self::run_timers`] actually ran.
    pub last_timer_fires: u32,
    /// Formatted `console.log` / `info` / `warn` / `error` output lines.
    pub console: Vec<String>,
    /// Active origin's `localStorage` entries.
    pub local_storage: Vec<(String, String)>,
    /// Active origin's `sessionStorage` entries.
    pub session_storage: Vec<(String, String)>,
    /// Active origin's serialized `IndexedDB` databases (`(db_name, json)`).
    pub indexed_db: Vec<(String, String)>,
    /// Current page URL (`window.location.href` / `document.URL`).
    pub location_href: String,
    /// Pending navigation URL requested by `location.assign` / `replace` / `href = ...`.
    pub pending_navigation: Option<String>,
    /// True when the active event handler called `event.preventDefault()`.
    pub event_default_prevented: bool,
    /// True when the active event handler called `event.stopPropagation()`.
    pub event_propagation_stopped: bool,
    /// Monotonic clock in milliseconds for `Date.now()` and `performance.now()`.
    pub clock_ms: f64,
    /// Prototype for wrapped DOM `Element` / `Node` instances.
    pub element_proto: Option<ObjectRef>,
    /// Prototype for `element.classList` `DOMTokenList` wrappers.
    pub class_list_proto: Option<ObjectRef>,
    /// Prototype for `element.style` `CSSStyleDeclaration` wrappers.
    pub style_proto: Option<ObjectRef>,
    /// Prototype for `element.dataset` `DOMStringMap` wrappers.
    pub dataset_proto: Option<ObjectRef>,
    /// Global `document` object.
    pub document_obj: Option<ObjectRef>,
    /// Registered live DOM event listeners: `(target_node, event_type_lowercase, callback)`.
    pub listeners: Vec<(NodeId, String, Persistent)>,
    /// Scheduled virtual timers. `due` is absolute virtual milliseconds.
    /// `task` is the page task that queued the timer.
    pub timers: Vec<HostTimer>,
    /// Next monotonically increasing timer handle id.
    pub next_timer_id: u32,
    /// Active 2D `<canvas>` subpaths per canvas `NodeId`.
    pub canvas_paths: std::collections::HashMap<NodeId, Vec<(i32, i32)>>,
    /// Active 2D `<canvas>` transform `(tx, ty, sx, sy)` per canvas `NodeId`.
    pub canvas_transform: std::collections::HashMap<NodeId, (f64, f64, f64, f64)>,
    /// Saved 2D `<canvas>` state stacks `(tx, ty, sx, sy, fill_style, stroke_style, line_width, global_alpha)`.
    pub canvas_stacks:
        std::collections::HashMap<NodeId, Vec<(f64, f64, f64, f64, String, String, f64, f64)>>,
    /// Active `MutationObserver` registrations: `(callback, observed_node)`.
    pub mutation_observers: Vec<(Persistent, NodeId)>,
    /// True when one or more custom elements have been defined via `customElements.define`.
    pub has_custom_elements: bool,
    /// Cached stylesheet, resolved `StyleMap`, and `Animator` for incremental style invalidation.
    style_cache: Option<(
        String,
        u32,
        u32,
        anvil_style::Stylesheet,
        anvil_style::StyleMap,
        anvil_style::Animator,
    )>,
    /// Incremental layout cache for coalescing DOM mutations across script layout queries.
    pub layout_cache: anvil_layout::LayoutCache,
    /// Registered mock responses for `fetch(url)`: `(url, status, body)`.
    pub fetch_mocks: Vec<(String, u16, String)>,
    /// Page fetch host. Cross-origin URLs never reach it. `None` means no socket.
    pub(crate) fetch_handler: FetchHandler,
    /// Most recent `fetch()` / `XMLHttpRequest` request captured as `(url, method, content_type, body)`.
    pub last_fetch_request: Option<(String, String, String, String)>,
    /// Most recent `<form>` submission payload captured as `(action_url, method, content_type, body)`.
    /// Not the platform handshake. See [`Self::last_host_submission`].
    pub last_submitted_form: Option<(String, String, String, String)>,
    /// Script submit handshake. The browser rebuilds the body.
    pub last_host_submission: Option<HostSubmission>,
    /// Immutable capture taken after an uncancelled submit. A later DOM write
    /// does not change it. A second successful submit replaces it.
    pub pending_snapshot: Option<SubmissionSnapshot>,
    /// Epoch copied from the page runtime. The document does not own it.
    pub submission_epoch: u64,
    /// False after release or a navigation away. Not a substitute for
    /// the form being connected.
    pub document_fully_active: bool,
    /// True while the page runtime holds a [`crate::PageSuspension`].
    /// Network callbacks stay queued or refused. Timers are not fired here.
    pub page_suspended: bool,
    /// Forms currently inside `requestSubmit` event dispatch. Per form.
    pub firing_submission: Vec<NodeId>,
    /// Forms currently building an entry list. Per form.
    pub constructing_entry: Vec<NodeId>,
    /// Trusted picker lookup. Empty means every file entry has no selection.
    pub(crate) selection_lookup: SelectionHook,
    /// `document.readyState` for this host. The accessor reads it.
    pub ready_state: ReadyState,
    /// Transient user activation for this turn. `play()` on an unmuted
    /// element consumes it. The page runtime copies it in and back.
    pub user_activation: bool,
    /// Active layout viewport width in CSS pixels (`window.innerWidth`).
    pub viewport_width: f32,
    /// Active layout viewport height in CSS pixels (`window.innerHeight`).
    pub viewport_height: f32,
    /// Active vertical viewport scroll offset in CSS pixels (`window.scrollY`).
    pub viewport_scroll_y: f32,
    /// When set, each register run is capped at `turn_budget_left` and the
    /// steps it actually takes are added to `turn_charged`.
    budget_armed: bool,
    turn_budget_left: u64,
    turn_charged: u64,
    /// Nonzero while a timer callback is running. A timer scheduled from
    /// inside one has the 4 ms floor.
    timer_depth: u32,
    /// Monotonic stop copied from the page runtime. `None` does not stop.
    /// Independent of [`Self::clock_ms`], which is the platform timer clock.
    pub(crate) wall_deadline: Option<std::time::Instant>,
    /// Queued dynamic `import(specifier)` requests awaiting fetch/link/eval.
    pub pending_dynamic_imports: VecDeque<PendingDynamicImport>,
    /// Dynamic `import(specifier)` promises parked on a module still evaluating.
    pub waiting_dynamic_imports: Vec<WaitingDynamicImport>,
    /// Video element currently in picture-in-picture.
    pub picture_in_picture: Option<NodeId>,
    /// Decoded WebAssembly modules created by this page.
    pub wasm_modules: Vec<crate::wasm::WasmModule>,
    /// Live WebAssembly instances created by this page.
    pub wasm_instances: Vec<crate::wasm::WasmInstance>,
    /// Host callback opening `WebSocket` connections.
    pub(crate) ws_handler: WebSocketHandler,
    /// Open `WebSocket` instances keyed by handle id.
    pub(crate) websockets: std::collections::HashMap<u32, Box<dyn WebSocketConnection>>,
    /// Next `WebSocket` handle id.
    pub(crate) next_ws_id: u32,
}

impl Default for CinderDomHost {
    fn default() -> Self {
        Self::new(Document::new(), "https://localhost/")
    }
}

impl CinderDomHost {
    /// Create a new [`CinderDomHost`] owning `doc` and `location_href`.
    pub fn new(doc: Document, location_href: &str) -> Self {
        Self {
            doc,
            jobs: VecDeque::new(),
            owner_task: 0,
            schedule_seq: 0,
            timer_spawn: 0,
            timer_callback_budget: u32::MAX,
            last_timer_fires: 0,
            console: Vec::new(),
            local_storage: Vec::new(),
            session_storage: Vec::new(),
            indexed_db: Vec::new(),
            location_href: if location_href.trim().is_empty() {
                "https://localhost/".to_string()
            } else {
                location_href.trim().to_string()
            },
            pending_navigation: None,
            event_default_prevented: false,
            event_propagation_stopped: false,
            clock_ms: 0.0,
            element_proto: None,
            class_list_proto: None,
            style_proto: None,
            dataset_proto: None,
            document_obj: None,
            listeners: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            canvas_paths: std::collections::HashMap::new(),
            canvas_transform: std::collections::HashMap::new(),
            canvas_stacks: std::collections::HashMap::new(),
            mutation_observers: Vec::new(),
            has_custom_elements: false,
            style_cache: None,
            layout_cache: anvil_layout::LayoutCache::new(),
            fetch_mocks: Vec::new(),
            fetch_handler: FetchHandler::default(),
            last_fetch_request: None,
            last_submitted_form: None,
            last_host_submission: None,
            pending_snapshot: None,
            submission_epoch: 1,
            document_fully_active: true,
            page_suspended: false,
            firing_submission: Vec::new(),
            constructing_entry: Vec::new(),
            selection_lookup: SelectionHook::default(),
            ready_state: ReadyState::Complete,
            user_activation: false,
            viewport_width: 1280.0,
            viewport_height: 800.0,
            viewport_scroll_y: 0.0,
            budget_armed: false,
            turn_budget_left: 0,
            turn_charged: 0,
            timer_depth: 0,
            wall_deadline: None,
            pending_dynamic_imports: VecDeque::new(),
            waiting_dynamic_imports: Vec::new(),
            picture_in_picture: None,
            wasm_modules: Vec::new(),
            wasm_instances: Vec::new(),
            ws_handler: WebSocketHandler::default(),
            websockets: std::collections::HashMap::new(),
            next_ws_id: 1,
        }
    }

    /// Arm the shared turn slice for the next register runs. Zero refuses
    /// the next run instead of granting one step.
    pub fn arm_turn_budget(&mut self, steps_left: u64) {
        self.budget_armed = true;
        self.turn_budget_left = steps_left;
        self.turn_charged = 0;
    }

    /// Steps the register VM actually ran since [`Self::arm_turn_budget`].
    pub fn take_turn_charge(&mut self) -> u64 {
        std::mem::take(&mut self.turn_charged)
    }

    /// Record steps from a `vm.run` that already returned. Call this before
    /// another run resets the counter.
    pub fn note_external_steps(&mut self, steps: u64) {
        if !self.budget_armed {
            return;
        }
        let used = steps.min(self.turn_budget_left);
        self.turn_budget_left = self.turn_budget_left.saturating_sub(used);
        self.turn_charged = self.turn_charged.saturating_add(used);
    }

    /// Keep one listener. Returns false when the queue is already full.
    pub fn push_listener(&mut self, node: NodeId, event: String, callback: Persistent) -> bool {
        if self.listeners.len() >= MAX_HOST_LISTENERS {
            return false;
        }
        self.listeners.push((node, event, callback));
        true
    }

    /// Drop listeners, jobs, timers, and observers. The register VM's globals
    /// stay; these handles must not fire or replay.
    pub fn drop_callbacks(&mut self) {
        self.jobs.clear();
        self.listeners.clear();
        self.timers.clear();
        self.mutation_observers.clear();
        self.last_host_submission = None;
        self.pending_snapshot = None;
        self.firing_submission.clear();
        self.constructing_entry.clear();
        self.pending_dynamic_imports.clear();
        self.waiting_dynamic_imports.clear();
        self.event_default_prevented = false;
        self.event_propagation_stopped = false;
        self.last_fetch_request = None;
        self.websockets.clear();
    }

    /// Capture the form after the submit listener has returned. Does not
    /// validate and does not read file attributes. Replaces any earlier
    /// pending snapshot for this page.
    pub fn capture_form_snapshot(&mut self, form: NodeId, submitter: Option<NodeId>) -> bool {
        if self.budget_armed && self.turn_budget_left == 0 {
            return false;
        }
        if self.constructing_entry.contains(&form) {
            return false;
        }
        self.constructing_entry.push(form);
        let document_url = self.location_href.clone();
        let base_url = submission_base_url(&self.doc, &document_url);
        let owned = self.selection_lookup.0.clone();
        let empty = EmptySelection;
        let raw: &dyn SelectionLookup = match owned.as_deref() {
            Some(lookup) => lookup,
            None => &empty,
        };
        let lookup = CheckedSelection { inner: raw };
        let shot = self.doc.snapshot_form_submission(
            form,
            submitter,
            self.submission_epoch,
            SubmissionUrls {
                document_url: &document_url,
                base_url: &base_url,
            },
            &lookup,
        );
        self.constructing_entry.retain(|id| *id != form);
        match shot {
            Ok(snapshot) => {
                self.pending_snapshot = Some(snapshot);
                true
            }
            Err(SnapshotError::MissingForm | SnapshotError::NotAForm) => false,
        }
    }

    /// Drop jobs and timers queued by `task`. Other tasks' entries stay.
    pub(crate) fn discard_owner_jobs(&mut self, task: u64) {
        self.jobs.retain(|owned| owned.task != task);
        self.timers.retain(|timer| timer.task != task);
        self.pending_dynamic_imports.retain(|req| req.task != task);
        self.waiting_dynamic_imports.retain(|w| w.task != task);
    }

    /// Drop timers and jobs created by one exhausted timer callback.
    pub(crate) fn discard_spawn(&mut self, spawn: u64) {
        if spawn == 0 {
            return;
        }
        self.jobs.retain(|owned| owned.spawn != spawn);
        self.timers.retain(|timer| timer.spawn != spawn);
        self.pending_dynamic_imports
            .retain(|req| req.spawn != spawn);
        self.waiting_dynamic_imports.retain(|w| w.spawn != spawn);
    }

    /// Soonest absolute due time still queued, if any.
    pub fn next_timer_due_ms(&self) -> Option<f64> {
        self.timers
            .iter()
            .map(|timer| timer.due)
            .min_by(|a, b| a.total_cmp(b))
    }

    fn budget_abort(&self) -> RunError {
        RunError::Abort {
            kind: cinder::limits::LimitKind::Steps,
            reason: "turn step budget exhausted".to_string(),
            limit: 0,
            observed: self.turn_charged,
            stack: Vec::new(),
        }
    }

    pub(crate) fn wall_deadline_reached(&self) -> bool {
        self.wall_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
    }

    fn deadline_abort(&self) -> RunError {
        RunError::Abort {
            kind: cinder::limits::LimitKind::Interrupt,
            reason: "wall deadline exhausted".to_string(),
            limit: 0,
            observed: 0,
            stack: Vec::new(),
        }
    }

    fn guard_run(&mut self, vm: &mut Vm) -> Result<(), RunError> {
        if self.wall_deadline_reached() {
            return Err(self.deadline_abort());
        }
        if !self.budget_armed {
            return Ok(());
        }
        if self.turn_budget_left == 0 {
            return Err(self.budget_abort());
        }
        vm.set_max_steps(Some(self.turn_budget_left))
            .map_err(RunError::internal)
    }

    fn charge_run(&mut self, vm: &Vm) {
        if !self.budget_armed {
            return;
        }
        let used = vm.steps().min(self.turn_budget_left);
        self.turn_budget_left = self.turn_budget_left.saturating_sub(used);
        self.turn_charged = self.turn_charged.saturating_add(used);
    }

    fn run_budgeted(&mut self, vm: &mut Vm, id: cinder::vm::CodeId) -> Result<JsValue, RunError> {
        self.guard_run(vm)?;
        let result = vm.run(self, id);
        self.charge_run(vm);
        result
    }

    fn call_budgeted(
        &mut self,
        vm: &mut Vm,
        f: JsValue,
        this: JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, RunError> {
        self.guard_run(vm)?;
        let result = vm.call(self, f, this, args);
        self.charge_run(vm);
        result
    }

    /// Drain in slices of `per_slice` (0 means the whole queue) while the
    /// turn still has steps above `step_reserve`. A listener cap is a slice
    /// size, not a point where queued promises are abandoned. Stopping with
    /// jobs left writes an explicit console line.
    pub(crate) fn drain_with_progress(
        &mut self,
        vm: &mut Vm,
        per_slice: usize,
        step_reserve: u64,
    ) -> Result<usize, RunError> {
        let mut total = 0usize;
        loop {
            let cap = if per_slice == 0 {
                usize::MAX
            } else {
                per_slice
            };
            let ran = self.drain_for_task(vm, cap, step_reserve)?;
            total += ran;
            if self.jobs.is_empty() {
                return Ok(total);
            }
            if !self.budget_armed || self.turn_budget_left <= step_reserve {
                if step_reserve == 0 && self.budget_armed && self.turn_budget_left == 0 {
                    return Err(RunError::Abort {
                        kind: cinder::LimitKind::Steps,
                        reason: "step budget used up".to_string(),
                        limit: 0,
                        observed: 0,
                        stack: Vec::new(),
                    });
                }
                self.console
                    .push("[error] microtask backlog deferred: input step reserve".to_string());
                return Ok(total);
            }
            if ran == 0 {
                self.console
                    .push("[error] microtask backlog made no progress".to_string());
                return Ok(total);
            }
        }
    }

    pub(crate) fn drain_for_task(
        &mut self,
        vm: &mut Vm,
        max_jobs: usize,
        step_reserve: u64,
    ) -> Result<usize, RunError> {
        let mut ran = 0usize;
        while ran < max_jobs {
            if self.wall_deadline_reached() {
                return Err(self.deadline_abort());
            }
            if self.budget_armed && self.turn_budget_left <= step_reserve {
                break;
            }
            let Some(owned) = self.jobs.pop_front() else {
                break;
            };
            self.guard_run(vm)?;
            let result = vm.run_job(self, owned.job);
            self.charge_run(vm);
            result?;
            ran += 1;
        }
        Ok(ran)
    }

    pub(crate) fn queued_jobs(&self) -> usize {
        self.jobs.len()
    }

    pub(crate) fn turn_budget_left(&self) -> u64 {
        self.turn_budget_left
    }

    fn drain_budgeted(&mut self, vm: &mut Vm) -> Result<usize, RunError> {
        let mut ran = 0usize;
        while let Some(owned) = self.jobs.pop_front() {
            self.guard_run(vm)?;
            let result = vm.run_job(self, owned.job);
            self.charge_run(vm);
            result?;
            ran += 1;
        }
        Ok(ran)
    }

    /// Collect document `<style>` rules and `adoptedStyleSheets` and cascade styles
    /// (incrementally re-evaluating only dirty subtrees when the stylesheet and viewport are unchanged).
    pub fn compute_styles(&mut self) -> anvil_style::StyleMap {
        let css_src = self.doc.embedded_css();
        let vw_bits = self.viewport_width.to_bits();
        let vh_bits = self.viewport_height.to_bits();
        if let Some((cached_css, cached_vw, cached_vh, sheet, styles, animator)) =
            self.style_cache.as_mut()
            && *cached_css == css_src
            && *cached_vw == vw_bits
            && *cached_vh == vh_bits
        {
            let _ = anvil_style::compute_styles_incremental(
                &mut self.doc,
                sheet,
                styles,
                self.viewport_width,
                self.viewport_height,
                Some(animator),
            );
            return styles.clone();
        }
        self.doc.clear_dirty_styles();
        let sheet = anvil_style::parse_css(&css_src);
        let mut animator = anvil_style::Animator::new();
        let styles = anvil_style::compute_styles_animated(
            &self.doc,
            &sheet,
            self.viewport_width,
            self.viewport_height,
            &mut animator,
        );
        self.style_cache = Some((css_src, vw_bits, vh_bits, sheet, styles.clone(), animator));
        styles
    }

    /// Resolve styles and compute layout for `self.doc` incrementally, returning the
    /// [`anvil_layout::LayoutBox`] for `nid` if it participates in layout.
    pub fn compute_node_box(&mut self, nid: NodeId) -> Option<anvil_layout::LayoutBox> {
        let styles = self.compute_styles();
        let focused = self.doc.focused_node();
        let layout = anvil_layout::layout_document_incremental(
            &self.doc,
            &styles,
            self.viewport_width,
            &std::collections::HashMap::new(),
            focused,
            &mut self.layout_cache,
        );
        layout.find_node_box(nid).cloned()
    }

    /// Install all Web/DOM globals (`window`, `document`, `Element.prototype`,
    /// `localStorage`, `sessionStorage`, `location`, `navigator`, `customElements`,
    /// `DOMParser`, `CSSStyleSheet`, `matchMedia`, `getComputedStyle`) onto `vm`.
    pub fn install(&mut self, vm: &mut Vm) -> Result<(), RunError> {
        let g = vm.global();

        // 1. Element.prototype
        let el_proto = vm.new_plain_object()?;
        for (name, func) in [
            ("getAttribute", el_get_attribute as NativeFn),
            ("setAttribute", el_set_attribute),
            ("removeAttribute", el_remove_attribute),
            ("hasAttribute", el_has_attribute),
            ("getAttributeNames", el_get_attribute_names),
            ("hasAttributes", el_has_attributes),
            ("appendChild", el_append_child),
            ("removeChild", el_remove_child),
            ("insertBefore", el_insert_before),
            ("replaceChild", el_replace_child),
            ("cloneNode", el_clone_node),
            ("remove", el_remove),
            ("querySelector", el_query_selector),
            ("querySelectorAll", el_query_selector_all),
            ("getElementById", el_get_element_by_id),
            ("getElementsByTagName", el_get_elements_by_tag_name),
            ("getElementsByClassName", el_get_elements_by_class_name),
            ("insertAdjacentHTML", el_insert_adjacent_html),
            ("insertAdjacentText", el_insert_adjacent_text),
            ("insertAdjacentElement", el_insert_adjacent_element),
            ("matches", el_matches),
            ("closest", el_closest),
            ("contains", el_contains),
            ("attachShadow", el_attach_shadow),
            ("addEventListener", el_add_event_listener),
            ("removeEventListener", el_remove_event_listener),
            ("dispatchEvent", el_dispatch_event),
            ("getBoundingClientRect", el_get_bounding_client_rect),
            ("getContext", el_get_context),
            ("toDataURL", el_to_data_url),
            ("setTextContent", el_set_text_content_method),
            ("getTextContent", el_get_text_content_method),
            ("setInnerHTML", el_set_inner_html_method),
            ("setValue", el_set_value_method),
            ("focus", el_focus),
            ("blur", el_blur),
            ("scrollIntoView", el_noop),
            ("scrollTo", el_scroll_to),
            ("scrollBy", el_scroll_by),
            ("show", el_dialog_show),
            ("showModal", el_dialog_show_modal),
            ("close", el_dialog_close),
            ("checkValidity", el_check_validity),
            ("reportValidity", el_report_validity),
            ("setCustomValidity", el_set_custom_validity),
            ("submit", el_form_submit),
            ("requestSubmit", el_form_request_submit),
            ("requestPictureInPicture", el_request_picture_in_picture),
            ("leavePictureInPicture", el_leave_picture_in_picture),
            ("assignedNodes", el_assigned_nodes),
            ("assignedElements", el_assigned_elements),
        ] {
            vm.define_method(el_proto, name, func)?;
        }

        for (prop, getter, setter) in [
            (
                "id",
                Some(el_get_id as NativeFn),
                Some(el_set_id as NativeFn),
            ),
            (
                "className",
                Some(el_get_class_name),
                Some(el_set_class_name),
            ),
            (
                "textContent",
                Some(el_get_text_content),
                Some(el_set_text_content),
            ),
            (
                "innerText",
                Some(el_get_text_content),
                Some(el_set_text_content),
            ),
            (
                "innerHTML",
                Some(el_get_inner_html),
                Some(el_set_inner_html),
            ),
            ("outerHTML", Some(el_get_outer_html), None),
            ("value", Some(el_get_value), Some(el_set_value)),
            ("checked", Some(el_get_checked), Some(el_set_checked)),
            ("tagName", Some(el_get_tag_name), None),
            ("nodeName", Some(el_get_node_name), None),
            ("nodeType", Some(el_get_node_type), None),
            (
                "nodeValue",
                Some(el_get_node_value),
                Some(el_set_node_value),
            ),
            ("data", Some(el_get_node_value), Some(el_set_node_value)),
            ("ownerDocument", Some(el_get_owner_document), None),
            ("isConnected", Some(el_get_is_connected), None),
            ("parentElement", Some(el_get_parent_element), None),
            ("parentNode", Some(el_get_parent_node), None),
            ("firstElementChild", Some(el_get_first_element_child), None),
            ("lastElementChild", Some(el_get_last_element_child), None),
            ("firstChild", Some(el_get_first_child), None),
            ("lastChild", Some(el_get_last_child), None),
            ("childNodes", Some(el_get_child_nodes), None),
            ("nextSibling", Some(el_get_next_sibling), None),
            ("previousSibling", Some(el_get_previous_sibling), None),
            (
                "nextElementSibling",
                Some(el_get_next_element_sibling),
                None,
            ),
            (
                "previousElementSibling",
                Some(el_get_previous_element_sibling),
                None,
            ),
            ("children", Some(el_get_children), None),
            ("childElementCount", Some(el_get_child_element_count), None),
            ("shadowRoot", Some(el_get_shadow_root), None),
            ("host", Some(el_get_shadow_host), None),
            ("mode", Some(el_get_shadow_mode), None),
            ("slot", Some(el_get_slot), Some(el_set_slot)),
            ("assignedSlot", Some(el_get_assigned_slot), None),
            ("classList", Some(el_get_class_list), None),
            ("style", Some(el_get_style), None),
            ("dataset", Some(el_get_dataset), None),
            ("offsetWidth", Some(el_get_offset_width), None),
            ("offsetHeight", Some(el_get_offset_height), None),
            ("clientWidth", Some(el_get_client_width), None),
            ("clientHeight", Some(el_get_client_height), None),
            ("scrollWidth", Some(el_get_scroll_width), None),
            ("scrollHeight", Some(el_get_scroll_height), None),
            (
                "scrollTop",
                Some(el_get_scroll_top),
                Some(el_set_scroll_top),
            ),
            (
                "scrollLeft",
                Some(el_get_scroll_left),
                Some(el_set_scroll_left),
            ),
            ("open", Some(el_get_dialog_open), Some(el_set_dialog_open)),
            (
                "returnValue",
                Some(el_get_dialog_return_value),
                Some(el_set_dialog_return_value),
            ),
            ("validity", Some(el_get_validity), None),
            ("validationMessage", Some(el_get_validation_message), None),
            ("willValidate", Some(el_get_will_validate), None),
        ] {
            vm.define_accessor(el_proto, prop, getter, setter)?;
        }
        self.element_proto = Some(el_proto);

        // 2. DOMTokenList (element.classList) prototype
        let cl_proto = vm.new_plain_object()?;
        for (name, func) in [
            ("add", cl_add as NativeFn),
            ("remove", cl_remove),
            ("toggle", cl_toggle),
            ("contains", cl_contains),
            ("toString", cl_to_string),
        ] {
            vm.define_method(cl_proto, name, func)?;
        }
        vm.define_accessor(cl_proto, "value", Some(cl_to_string), None)?;
        self.class_list_proto = Some(cl_proto);

        // 3. CSSStyleDeclaration (element.style) prototype
        let st_proto = vm.new_plain_object()?;
        for (name, func) in [
            ("setProperty", style_set_property as NativeFn),
            ("getPropertyValue", style_get_property_value),
            ("removeProperty", style_remove_property),
        ] {
            vm.define_method(st_proto, name, func)?;
        }
        vm.define_accessor(
            st_proto,
            "cssText",
            Some(style_get_css_text),
            Some(style_set_css_text),
        )?;
        for prop in [
            "color",
            "backgroundColor",
            "background",
            "display",
            "width",
            "height",
            "minWidth",
            "minHeight",
            "maxWidth",
            "maxHeight",
            "opacity",
            "transform",
            "fontSize",
            "fontWeight",
            "margin",
            "marginTop",
            "marginBottom",
            "marginLeft",
            "marginRight",
            "padding",
            "paddingTop",
            "paddingBottom",
            "paddingLeft",
            "paddingRight",
            "border",
            "borderRadius",
            "visibility",
            "position",
            "top",
            "left",
            "right",
            "bottom",
            "zIndex",
            "overflow",
            "overflowX",
            "overflowY",
            "cursor",
            "textAlign",
            "lineHeight",
            "gap",
            "rowGap",
            "columnGap",
            "flexDirection",
            "alignItems",
            "justifyContent",
            "boxSizing",
            "flex",
            "flexGrow",
            "flexShrink",
            "flexBasis",
            "flexWrap",
            "gridTemplateColumns",
            "gridTemplateRows",
            "gridColumn",
            "gridRow",
            "outline",
            "boxShadow",
            "textDecoration",
            "fontFamily",
            "whiteSpace",
            "wordBreak",
            "verticalAlign",
            "pointerEvents",
            "userSelect",
            "letterSpacing",
            "transition",
            "animation",
            "fill",
            "stroke",
            "float",
            "clear",
        ] {
            self.install_style_accessor(vm, st_proto, prop)?;
        }
        self.style_proto = Some(st_proto);

        // 4. DOMStringMap (element.dataset) prototype
        let ds_proto = vm.new_plain_object()?;
        vm.define_method(ds_proto, "get", dataset_get)?;
        vm.define_method(ds_proto, "set", dataset_set)?;
        for key in [
            "id",
            "index",
            "value",
            "state",
            "mode",
            "tab",
            "section",
            "url",
            "action",
            "target",
            "type",
            "name",
            "role",
            "theme",
            "status",
            "count",
            "active",
            "open",
            "loaded",
            "recovered",
            "threadId",
            "testid",
        ] {
            self.install_dataset_accessor(vm, ds_proto, key)?;
        }
        self.dataset_proto = Some(ds_proto);

        // 5. document object
        let doc_obj = vm.new_plain_object()?;
        for (name, func) in [
            ("getElementById", doc_get_element_by_id as NativeFn),
            ("querySelector", doc_query_selector),
            ("querySelectorAll", doc_query_selector_all),
            ("createElement", doc_create_element),
            ("createTextNode", doc_create_text_node),
            ("createComment", doc_create_text_node),
            ("createDocumentFragment", doc_create_document_fragment),
            ("getElementsByTagName", doc_get_elements_by_tag_name),
            ("getElementsByClassName", doc_get_elements_by_class_name),
            ("getElementsByName", doc_get_elements_by_name),
            ("addEventListener", doc_add_event_listener),
            ("removeEventListener", doc_remove_event_listener),
            ("dispatchEvent", doc_dispatch_event),
            ("adoptNode", doc_adopt_node),
            ("importNode", doc_import_node),
            ("__parseHTML", doc_parse_html_fragment),
            ("__addAdoptedSheet", doc_add_adopted_sheet),
            ("__setAdoptedSheets", doc_set_adopted_sheets),
            ("exitPictureInPicture", doc_exit_picture_in_picture),
        ] {
            vm.define_method(doc_obj, name, func)?;
        }
        vm.define_accessor(
            doc_obj,
            "pictureInPictureElement",
            Some(doc_get_picture_in_picture),
            None,
        )?;
        vm.define_accessor(doc_obj, "title", Some(doc_get_title), Some(doc_set_title))?;
        vm.define_accessor(doc_obj, "body", Some(doc_get_body), None)?;
        vm.define_accessor(doc_obj, "head", Some(doc_get_head), None)?;
        vm.define_accessor(
            doc_obj,
            "documentElement",
            Some(doc_get_document_element),
            None,
        )?;
        vm.define_accessor(doc_obj, "activeElement", Some(doc_get_active_element), None)?;
        vm.define_accessor(doc_obj, "readyState", Some(doc_get_ready_state), None)?;
        vm.define_accessor(doc_obj, "URL", Some(doc_get_url), None)?;
        vm.define_accessor(doc_obj, "documentURI", Some(doc_get_url), None)?;
        vm.define_accessor(doc_obj, "nodeType", Some(doc_get_node_type), None)?;
        vm.define_accessor(doc_obj, "nodeName", Some(doc_get_node_name), None)?;
        vm.define_accessor(doc_obj, "childNodes", Some(doc_get_child_nodes), None)?;
        vm.define_accessor(doc_obj, "firstChild", Some(doc_get_document_element), None)?;
        vm.define_accessor(doc_obj, "lastChild", Some(doc_get_document_element), None)?;
        vm.define_accessor(doc_obj, "isConnected", Some(doc_get_is_connected), None)?;
        let empty_sheets = vm.new_array_from(&[])?;
        vm.define_data(doc_obj, "_adoptedSheets", empty_sheets)?;
        self.document_obj = Some(doc_obj);

        // 6. localStorage & sessionStorage
        let local_obj = vm.new_plain_object()?;
        for (name, func) in [
            ("getItem", local_storage_get_item as NativeFn),
            ("setItem", local_storage_set_item),
            ("removeItem", local_storage_remove_item),
            ("clear", local_storage_clear),
            ("key", local_storage_key),
        ] {
            vm.define_method(local_obj, name, func)?;
        }
        vm.define_accessor(local_obj, "length", Some(local_storage_length), None)?;

        let session_obj = vm.new_plain_object()?;
        for (name, func) in [
            ("getItem", session_storage_get_item as NativeFn),
            ("setItem", session_storage_set_item),
            ("removeItem", session_storage_remove_item),
            ("clear", session_storage_clear),
            ("key", session_storage_key),
        ] {
            vm.define_method(session_obj, name, func)?;
        }
        vm.define_accessor(session_obj, "length", Some(session_storage_length), None)?;

        // 7. location, history, navigator, customElements
        let loc_obj = vm.new_plain_object()?;
        vm.define_accessor(loc_obj, "href", Some(loc_get_href), Some(loc_set_href))?;
        vm.define_accessor(loc_obj, "origin", Some(loc_get_origin), None)?;
        vm.define_accessor(loc_obj, "pathname", Some(loc_get_pathname), None)?;
        vm.define_accessor(loc_obj, "search", Some(loc_get_search), None)?;
        vm.define_accessor(loc_obj, "hash", Some(loc_get_hash), None)?;
        vm.define_method(loc_obj, "assign", loc_assign)?;
        vm.define_method(loc_obj, "replace", loc_assign)?;
        vm.define_method(loc_obj, "toString", loc_get_href)?;

        let hist_obj = vm.new_plain_object()?;
        vm.define_method(hist_obj, "pushState", hist_push_state)?;
        vm.define_method(hist_obj, "replaceState", hist_push_state)?;

        let nav_obj = vm.new_plain_object()?;
        let ua = vm.string("AnvilBrowser/0.24")?;
        let lang = vm.string("en-US")?;
        vm.define_data(nav_obj, "userAgent", ua)?;
        vm.define_data(nav_obj, "language", lang)?;

        let ce_obj = vm.new_plain_object()?;
        vm.define_method(ce_obj, "define", custom_elements_define)?;
        vm.define_method(ce_obj, "get", custom_elements_get)?;

        // 8. Global bindings
        vm.define_data(g, "window", JsValue::object(g))?;
        vm.define_data(g, "self", JsValue::object(g))?;
        vm.define_data(g, "__elementProto", JsValue::object(el_proto))?;
        vm.define_data(g, "document", JsValue::object(doc_obj))?;
        vm.define_data(g, "localStorage", JsValue::object(local_obj))?;
        vm.define_data(g, "sessionStorage", JsValue::object(session_obj))?;
        vm.define_data(g, "location", JsValue::object(loc_obj))?;
        vm.define_data(g, "history", JsValue::object(hist_obj))?;
        vm.define_data(g, "navigator", JsValue::object(nav_obj))?;
        vm.define_data(g, "customElements", JsValue::object(ce_obj))?;
        for (name, func) in [
            ("getComputedStyle", window_get_computed_style as NativeFn),
            ("matchMedia", window_match_media),
            ("addEventListener", doc_add_event_listener),
            ("removeEventListener", doc_remove_event_listener),
            ("dispatchEvent", doc_dispatch_event),
            ("setTimeout", window_set_timeout),
            ("clearTimeout", window_clear_timer),
            ("setInterval", window_set_interval),
            ("clearInterval", window_clear_timer),
            ("requestAnimationFrame", window_request_animation_frame),
            ("cancelAnimationFrame", window_clear_timer),
            ("__nativePreventDefault", ev_prevent_default),
            ("__nativeStopPropagation", ev_stop_propagation),
            ("__nativeFetch", window_native_fetch),
            ("__nativeWebSocketConnect", window_native_ws_connect),
            ("__nativeWebSocketSend", window_native_ws_send),
            ("__nativeWebSocketClose", window_native_ws_close),
            ("__nativeWebSocketPoll", window_native_ws_poll),
            ("__nativeIdbLoad", window_native_idb_load),
            ("__nativeIdbSave", window_native_idb_save),
            ("__nativeIdbDelete", window_native_idb_delete),
            ("__nativeIdbList", window_native_idb_list),
            ("__anvil_dynamic_import", window_dynamic_import),
            ("__anvilMediaPlay", window_media_play),
            ("__anvilWasmCall", wasm_call),
            ("__sha256Bytes", window_sha256_bytes),
            (
                "__registerMutationObserver",
                window_register_mutation_observer,
            ),
            (
                "__unregisterMutationObserver",
                window_unregister_mutation_observer,
            ),
        ] {
            vm.define_method(g, name, func)?;
        }
        install_webassembly(vm)?;

        self.install_web_api_classes(vm)?;

        Ok(())
    }

    fn install_web_api_classes(&mut self, vm: &mut Vm) -> Result<(), RunError> {
        let bootstrap = cinder::DOM_WEB_BOOTSTRAP_JS;
        let limits = vm.limits().clone();
        let code = cinder::compile_script(bootstrap, &limits)
            .map_err(|e| RunError::internal(e.to_string()))?;
        let id = vm.load(code)?;
        self.run_budgeted(vm, id)?;
        self.drain_budgeted(vm)?;
        Ok(())
    }

    /// True when one or more `setTimeout`, `setInterval`, or `requestAnimationFrame`
    /// callbacks remain scheduled on this host.
    pub fn has_pending_timers(&self) -> bool {
        !self.timers.is_empty()
    }

    /// Advance the virtual clock by `delta_ms` and execute any due `setTimeout`,
    /// `setInterval`, or `requestAnimationFrame` callbacks for one animation frame tick.
    pub fn tick_timers(&mut self, vm: &mut Vm, delta_ms: f64) -> Result<bool, RunError> {
        if self.timers.is_empty() {
            return Ok(false);
        }
        self.clock_ms += delta_ms.max(0.0);
        let target_clock = self.clock_ms;
        let max_id_before_tick = self.next_timer_id;
        let mut ran = false;
        let mut fired_intervals = std::collections::HashSet::new();
        for _ in 0..64 {
            let mut next_idx: Option<usize> = None;
            for (i, timer) in self.timers.iter().enumerate() {
                if timer.id >= max_id_before_tick {
                    continue;
                }
                if timer.interval.is_some() && fired_intervals.contains(&timer.id) {
                    continue;
                }
                if !timer.is_raf && timer.due > target_clock {
                    continue;
                }
                if next_idx.is_none_or(|best| timer.due < self.timers[best].due) {
                    next_idx = Some(i);
                }
            }
            let Some(idx) = next_idx else {
                break;
            };
            let timer = self.timers[idx].snapshot();
            if let Some(iv) = timer.interval {
                self.timers[idx].due = next_interval_due(timer.due, iv, self.clock_ms);
                fired_intervals.insert(timer.id);
            } else {
                self.timers.remove(idx);
            }
            let cb_val = vm
                .heap()
                .persistent(timer.callback)
                .map_err(|e| RunError::internal(format!("{e:?}")))?;
            let args = if timer.is_raf {
                vec![JsValue::number(self.clock_ms)]
            } else {
                Vec::new()
            };
            let _ = self.call_budgeted(vm, cb_val, JsValue::UNDEFINED, &args)?;
            self.drain_budgeted(vm)?;
            ran = true;
        }
        Ok(ran)
    }

    /// Fire timers whose due time is at or before the virtual clock.
    /// The clock is not moved. Each interval runs at most once; its next
    /// due stays in the future so a later poll can fire it. Returns after
    /// the due set, leaving [`Self::next_timer_due_ms`] for the platform.
    pub fn run_timers(&mut self, vm: &mut Vm) -> Result<bool, RunError> {
        let mut ran = false;
        let mut fired = 0u32;
        let initial_max_raf_id = self.next_timer_id;
        let mut fired_intervals = std::collections::HashSet::new();
        let rounds = if self.budget_armed { usize::MAX } else { 64 };
        for _ in 0..rounds {
            if self.wall_deadline_reached() {
                return Err(self.deadline_abort());
            }
            if fired >= self.timer_callback_budget {
                break;
            }
            if self.budget_armed && self.turn_budget_left == 0 {
                return Err(self.budget_abort());
            }
            let now = self.clock_ms;
            let mut next_idx: Option<usize> = None;
            for (i, timer) in self.timers.iter().enumerate() {
                if timer.is_raf && timer.id >= initial_max_raf_id {
                    continue;
                }
                if timer.interval.is_some() && fired_intervals.contains(&timer.id) {
                    continue;
                }
                if !timer.due.is_finite() || timer.due > now {
                    continue;
                }
                if next_idx.is_none_or(|best| timer.due < self.timers[best].due) {
                    next_idx = Some(i);
                }
            }
            let Some(idx) = next_idx else {
                break;
            };
            let timer = self.timers[idx].snapshot();
            if let Some(iv) = timer.interval {
                self.timers[idx].due = next_interval_due(timer.due, iv, now);
                fired_intervals.insert(timer.id);
            } else {
                self.timers.remove(idx);
            }
            let cb_val = vm
                .heap()
                .persistent(timer.callback)
                .map_err(|e| RunError::internal(format!("{e:?}")))?;
            let args = if timer.is_raf {
                vec![JsValue::number(self.clock_ms)]
            } else {
                Vec::new()
            };
            self.timer_depth = self.timer_depth.saturating_add(1);
            self.timer_spawn = self.timer_spawn.wrapping_add(1).max(1);
            let spawn = self.timer_spawn;
            let called = self.call_budgeted(vm, cb_val, JsValue::UNDEFINED, &args);
            self.timer_depth = self.timer_depth.saturating_sub(1);
            let exhausted = self.budget_armed && self.turn_budget_left == 0;
            let deadline_hit = self.wall_deadline_reached()
                || called
                    .as_ref()
                    .err()
                    .is_some_and(|err| err.message().contains("wall deadline"));
            fired = fired.saturating_add(1);
            self.last_timer_fires = fired;
            if exhausted {
                self.discard_spawn(spawn);
                self.timer_spawn = 0;
                return Err(self.budget_abort());
            }
            if deadline_hit {
                self.discard_spawn(spawn);
                self.timer_spawn = 0;
                return Err(self.deadline_abort());
            }
            self.timer_spawn = 0;
            called?;
            self.drain_budgeted(vm)?;
            ran = true;
        }
        self.last_timer_fires = fired;
        Ok(ran)
    }

    fn install_style_accessor(
        &self,
        vm: &mut Vm,
        st_proto: ObjectRef,
        prop: &str,
    ) -> Result<(), RunError> {
        let css_name = camel_to_kebab(prop);
        let getter_src = format!(
            "(function(prop) {{ return function() {{ return this.getPropertyValue(prop); }}; }})({:?})",
            css_name
        );
        let setter_src = format!(
            "(function(prop) {{ return function(val) {{ this.setProperty(prop, val); }}; }})({:?})",
            css_name
        );
        let mut dummy_host = cinder::vm::BasicHost::default();
        let limits = vm.limits().clone();
        let get_code = cinder::compile_script(&getter_src, &limits)
            .map_err(|e| RunError::internal(e.to_string()))?;
        let get_id = vm.load(get_code)?;
        let get_fn = vm.run(&mut dummy_host, get_id)?;

        let set_code = cinder::compile_script(&setter_src, &limits)
            .map_err(|e| RunError::internal(e.to_string()))?;
        let set_id = vm.load(set_code)?;
        let set_fn = vm.run(&mut dummy_host, set_id)?;

        let key = vm
            .heap_mut()
            .key_from_str(prop)
            .map_err(|e| RunError::internal(format!("{e:?}")))?;
        vm.heap_mut()
            .define_own(
                st_proto,
                key,
                cinder::value::object::Property {
                    attrs: cinder::value::shape::Attrs(
                        cinder::value::shape::Attrs::ENUMERABLE
                            | cinder::value::shape::Attrs::CONFIGURABLE,
                    ),
                    slot: cinder::value::object::Slot::Accessor {
                        get: get_fn,
                        set: set_fn,
                    },
                },
            )
            .map_err(|e| RunError::internal(format!("{e:?}")))?;
        Ok(())
    }

    fn install_dataset_accessor(
        &self,
        vm: &mut Vm,
        ds_proto: ObjectRef,
        key_name: &str,
    ) -> Result<(), RunError> {
        let getter_src = format!(
            "(function(k) {{ return function() {{ return this.get(k); }}; }})({:?})",
            key_name
        );
        let setter_src = format!(
            "(function(k) {{ return function(v) {{ this.set(k, v); }}; }})({:?})",
            key_name
        );
        let mut dummy_host = cinder::vm::BasicHost::default();
        let limits = vm.limits().clone();
        let get_code = cinder::compile_script(&getter_src, &limits)
            .map_err(|e| RunError::internal(e.to_string()))?;
        let get_id = vm.load(get_code)?;
        let get_fn = vm.run(&mut dummy_host, get_id)?;

        let set_code = cinder::compile_script(&setter_src, &limits)
            .map_err(|e| RunError::internal(e.to_string()))?;
        let set_id = vm.load(set_code)?;
        let set_fn = vm.run(&mut dummy_host, set_id)?;

        let key = vm
            .heap_mut()
            .key_from_str(key_name)
            .map_err(|e| RunError::internal(format!("{e:?}")))?;
        vm.heap_mut()
            .define_own(
                ds_proto,
                key,
                cinder::value::object::Property {
                    attrs: cinder::value::shape::Attrs(
                        cinder::value::shape::Attrs::ENUMERABLE
                            | cinder::value::shape::Attrs::CONFIGURABLE,
                    ),
                    slot: cinder::value::object::Slot::Accessor {
                        get: get_fn,
                        set: set_fn,
                    },
                },
            )
            .map_err(|e| RunError::internal(format!("{e:?}")))?;
        Ok(())
    }

    /// Dispatch a live DOM event (`click`, `input`, `change`, `submit`, `keydown`)
    /// along the target's ancestor chain in `self.doc`, invoking all matching
    /// JS callbacks registered via `addEventListener` and draining microtasks.
    pub fn dispatch_event(
        &mut self,
        vm: &mut Vm,
        target: NodeId,
        event_type: &str,
        value: Option<&str>,
        key: Option<&str>,
    ) -> Result<bool, RunError> {
        self.event_default_prevented = false;
        self.event_propagation_stopped = false;
        let wanted = event_type.trim().to_ascii_lowercase();
        let bubbles = !matches!(wanted.as_str(), "close" | "invalid");
        let cancelable = !matches!(wanted.as_str(), "close" | "input" | "change");
        let mut chain = Vec::new();
        if bubbles {
            let mut cursor = Some(target);
            while let Some(id) = cursor {
                chain.push(id);
                cursor = self.doc.get(id).parent();
            }
            if !chain.contains(&self.doc.body()) {
                chain.push(self.doc.body());
            }
            if !chain.contains(&self.doc.root()) {
                chain.push(self.doc.root());
            }
        } else {
            chain.push(target);
        }

        let target_val = self.wrap_node_in_vm(vm, target)?;
        let g = vm.global();
        if let Ok(fn_key) = vm.heap_mut().key_from_str("__dispatchNativeDomEvent")
            && let Ok(Some(prop)) = vm.heap().get_own(g, fn_key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
        {
            let ev_js = vm.string(&wanted)?;
            let val_js = match value {
                Some(v) => vm.string(v)?,
                None => JsValue::UNDEFINED,
            };
            let key_js = match key {
                Some(k) => vm.string(k)?,
                None => JsValue::UNDEFINED,
            };
            let ret = self.call_budgeted(
                vm,
                fn_val,
                JsValue::object(g),
                &[target_val, ev_js, val_js, key_js],
            )?;
            self.drain_budgeted(vm)?;
            if ret.as_bool() == Some(true) {
                return Ok(true);
            }
        }
        let mut handled = false;

        for current_id in chain {
            if self.event_propagation_stopped {
                break;
            }
            let matching: Vec<Persistent> = self
                .listeners
                .iter()
                .filter(|(nid, ev, _)| {
                    *nid == current_id && (ev == &wanted || (wanted == "input" && ev == "change"))
                })
                .map(|(_, _, cb)| *cb)
                .collect();
            if matching.is_empty() {
                continue;
            }
            let cur_val = self.wrap_node_in_vm(vm, current_id)?;
            for cb_pers in matching {
                let cb_val = vm
                    .heap()
                    .persistent(cb_pers)
                    .map_err(|e| RunError::internal(format!("{e:?}")))?;
                let ev_obj = vm.new_plain_object()?;
                let t_str = vm.string(&wanted)?;
                vm.define_data(ev_obj, "type", t_str)?;
                vm.define_data(ev_obj, "target", target_val)?;
                vm.define_data(ev_obj, "currentTarget", cur_val)?;
                vm.define_data(ev_obj, "bubbles", JsValue::boolean(bubbles))?;
                vm.define_data(ev_obj, "cancelable", JsValue::boolean(cancelable))?;
                if let Some(v) = value {
                    let vs = vm.string(v)?;
                    vm.define_data(ev_obj, "value", vs)?;
                    vm.define_data(ev_obj, "data", vs)?;
                }
                if let Some(k) = key {
                    let ks = vm.string(k)?;
                    vm.define_data(ev_obj, "key", ks)?;
                    vm.define_data(ev_obj, "code", ks)?;
                }
                vm.define_accessor(
                    ev_obj,
                    "defaultPrevented",
                    Some(ev_get_default_prevented),
                    None,
                )?;
                if cancelable {
                    vm.define_method(ev_obj, "preventDefault", ev_prevent_default)?;
                } else {
                    vm.define_method(ev_obj, "preventDefault", ev_noop)?;
                }
                vm.define_method(ev_obj, "stopPropagation", ev_stop_propagation)?;

                let _ = self.call_budgeted(vm, cb_val, cur_val, &[JsValue::object(ev_obj)])?;
                handled = true;
                self.drain_budgeted(vm)?;
            }
        }
        if wanted == "close"
            && let Some(target_obj) = target_val.as_object()
            && let Ok(on_key) = vm.heap_mut().key_from_str("onclose")
            && let Ok(Some(prop)) = vm.heap().get_own(target_obj, on_key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
        {
            let ev_obj = vm.new_plain_object()?;
            let t_str = vm.string("close")?;
            vm.define_data(ev_obj, "type", t_str)?;
            vm.define_data(ev_obj, "target", target_val)?;
            vm.define_data(ev_obj, "currentTarget", target_val)?;
            vm.define_data(ev_obj, "bubbles", JsValue::FALSE)?;
            vm.define_data(ev_obj, "cancelable", JsValue::FALSE)?;
            vm.define_method(ev_obj, "preventDefault", ev_noop)?;
            vm.define_method(ev_obj, "stopPropagation", ev_stop_propagation)?;
            let _ = self.call_budgeted(vm, fn_val, target_val, &[JsValue::object(ev_obj)])?;
            handled = true;
            self.drain_budgeted(vm)?;
        }
        Ok(handled)
    }

    /// Invoke `window.__reportUncaughtError(msg)` if installed, firing `window.onerror`
    /// and any registered `'error'` event listeners without aborting the host session.
    pub fn report_uncaught_error(&mut self, vm: &mut Vm, message: &str) {
        let g = vm.global();
        if let Ok(fn_key) = vm.heap_mut().key_from_str("__reportUncaughtError")
            && let Ok(Some(prop)) = vm.heap().get_own(g, fn_key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
            && let Ok(msg_js) = vm.string(message)
        {
            let _ = self.call_budgeted(vm, fn_val, JsValue::object(g), &[msg_js]);
            let _ = self.drain_budgeted(vm);
        }
    }

    fn wrap_node_in_vm(&self, vm: &mut Vm, nid: NodeId) -> Result<JsValue, RunError> {
        let data = HostData(TAG_NODE | (nid.index() as u64));
        if let Some(existing) = vm.heap().wrapper(data) {
            return Ok(JsValue::object(existing));
        }
        let obj = vm.new_host_object(self.element_proto, data)?;
        vm.heap_mut().set_wrapper(data, obj);
        Ok(JsValue::object(obj))
    }
}

impl CinderDomHost {
    fn dispatch_page_transition_hook(
        &mut self,
        vm: &mut Vm,
        event_type: &str,
        persisted: bool,
    ) -> Result<bool, RunError> {
        let g = vm.global();
        let key = vm
            .heap_mut()
            .key_from_str("__dispatchPageTransition")
            .map_err(|e| RunError::internal(e.to_string()))?;
        let Some(func) = vm
            .heap()
            .get_own(g, key)
            .map_err(|e| RunError::internal(e.to_string()))?
            .and_then(|prop| prop.slot.data())
            .filter(|value| value.as_object().is_some())
        else {
            return Ok(false);
        };
        let name = vm.string(event_type)?;
        let fired = self.call_budgeted(
            vm,
            func,
            JsValue::object(g),
            &[name, JsValue::boolean(persisted)],
        )?;
        self.drain_budgeted(vm)?;
        Ok(fired.as_bool() == Some(true))
    }

    /// Run `pageshow` or `pagehide` listeners and tell them `event.persisted`.
    pub fn dispatch_page_transition(
        &mut self,
        vm: &mut Vm,
        event_type: &str,
        persisted: bool,
    ) -> Result<bool, RunError> {
        let wanted = event_type.trim().to_ascii_lowercase();
        if self.dispatch_page_transition_hook(vm, &wanted, persisted)? {
            return Ok(true);
        }
        let root = self.doc.root();
        let matching: Vec<Persistent> = self
            .listeners
            .iter()
            .filter(|(nid, ev, _)| *nid == root && ev == &wanted)
            .map(|(_, _, cb)| *cb)
            .collect();
        if matching.is_empty() {
            return Ok(false);
        }
        let target = self.doc.body();
        let target_val = self.wrap_node_in_vm(vm, target)?;
        for cb_pers in matching {
            let cb_val = vm
                .heap()
                .persistent(cb_pers)
                .map_err(|e| RunError::internal(format!("{e:?}")))?;
            let ev_obj = vm.new_plain_object()?;
            let t_str = vm.string(&wanted)?;
            vm.define_data(ev_obj, "type", t_str)?;
            vm.define_data(ev_obj, "persisted", JsValue::boolean(persisted))?;
            vm.define_data(ev_obj, "target", target_val)?;
            vm.define_data(ev_obj, "currentTarget", target_val)?;
            vm.define_data(ev_obj, "bubbles", JsValue::FALSE)?;
            vm.define_data(ev_obj, "cancelable", JsValue::FALSE)?;
            let _ = self.call_budgeted(vm, cb_val, target_val, &[JsValue::object(ev_obj)])?;
            self.drain_budgeted(vm)?;
        }
        Ok(true)
    }
}

impl Host for CinderDomHost {
    fn interrupt(&mut self) -> Option<String> {
        if self.page_suspended {
            Some("page is suspended".to_string())
        } else if self.wall_deadline_reached() {
            Some("wall deadline exhausted".to_string())
        } else {
            None
        }
    }

    fn enqueue_job(&mut self, job: Job) {
        if self.jobs.len() >= MAX_HOST_JOBS {
            self.console
                .push("[error] microtask queue full".to_string());
            return;
        }
        self.jobs.push_back(OwnedHostJob {
            task: self.owner_task,
            spawn: self.timer_spawn,
            job,
        });
    }

    fn take_job(&mut self) -> Option<Job> {
        self.jobs.pop_front().map(|owned| owned.job)
    }

    fn trace(&self, roots: &mut Vec<JsValue>) {
        for proto in [
            self.element_proto,
            self.class_list_proto,
            self.style_proto,
            self.dataset_proto,
            self.document_obj,
        ]
        .into_iter()
        .flatten()
        {
            roots.push(JsValue::object(proto));
        }
        for req in &self.pending_dynamic_imports {
            roots.push(req.resolve);
            roots.push(req.reject);
        }
        for wait in &self.waiting_dynamic_imports {
            roots.push(wait.resolve);
            roots.push(wait.reject);
        }
    }

    fn now_ms(&mut self) -> f64 {
        // The page clock moves only when the platform syncs `now_ms`.
        self.clock_ms
    }

    fn console(&mut self, level: &str, text: &str) {
        let prefix = match level {
            "warn" => "[warn] ",
            "error" => "[error] ",
            _ => "",
        };
        self.console.push(format!("{prefix}{text}"));
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---- Host/Wrapper helpers -------------------------------------------------

fn wrap_node(cx: &mut NativeCx<'_>, nid: NodeId) -> Result<JsValue, RunError> {
    let data = HostData(TAG_NODE | (nid.index() as u64));
    if let Some(existing) = cx.heap().wrapper(data) {
        return Ok(JsValue::object(existing));
    }
    let proto = cx.host_as::<CinderDomHost>().and_then(|h| h.element_proto);
    let obj = cx.vm().new_host_object(proto, data)?;
    cx.heap().set_wrapper(data, obj);
    Ok(JsValue::object(obj))
}

fn wrap_optional_node(cx: &mut NativeCx<'_>, nid: Option<NodeId>) -> Result<JsValue, RunError> {
    match nid {
        Some(id) => wrap_node(cx, id),
        None => Ok(JsValue::NULL),
    }
}

fn wrap_node_list(cx: &mut NativeCx<'_>, ids: Vec<NodeId>) -> Result<JsValue, RunError> {
    let mut vals = Vec::with_capacity(ids.len());
    for id in ids {
        let v = wrap_node(cx, id)?;
        cx.keep(v);
        vals.push(v);
    }
    cx.vm().new_array_from(&vals)
}

fn unwrap_node(cx: &mut NativeCx<'_>, val: JsValue) -> Option<NodeId> {
    let obj = val.as_object()?;
    if let Some(doc_obj) = cx.host_as::<CinderDomHost>().and_then(|h| h.document_obj)
        && obj == doc_obj
    {
        return cx.host_as::<CinderDomHost>().map(|h| h.doc.root());
    }
    let class = cx.heap().object(obj).ok()?.class.clone();
    match class {
        ObjectClass::Host(HostData(raw)) => {
            let tag = raw & TAG_MASK;
            if matches!(tag, TAG_NODE | TAG_CLASS_LIST | TAG_STYLE | TAG_DATASET) {
                Some(NodeId::from_raw((raw & 0xFFFF_FFFF) as u32))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn arg_str(cx: &mut NativeCx<'_>, args: &[JsValue], idx: usize) -> Result<String, RunError> {
    match args.get(idx).copied() {
        Some(v) if !v.is_undefined() => cx.to_display_string(v),
        _ => Ok(String::new()),
    }
}

// ---- Event methods --------------------------------------------------------

fn ev_get_default_prevented(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let prevented = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.event_default_prevented);
    Ok(JsValue::boolean(prevented))
}

fn ev_prevent_default(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.event_default_prevented = true;
    }
    Ok(JsValue::UNDEFINED)
}

fn ev_noop(_cx: &mut NativeCx<'_>, _this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    Ok(JsValue::UNDEFINED)
}

fn ev_stop_propagation(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.event_propagation_stopped = true;
    }
    Ok(JsValue::UNDEFINED)
}

// ---- Document methods & accessors -----------------------------------------

fn doc_get_element_by_id(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id_str = arg_str(cx, args, 0)?;
    let found = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get_element_by_id(&id_str));
    wrap_optional_node(cx, found)
}

fn doc_query_selector(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let sel = arg_str(cx, args, 0)?;
    let found = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.query_selector(&sel));
    wrap_optional_node(cx, found)
}

fn doc_query_selector_all(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let sel = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.query_selector_all(&sel))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn doc_create_element(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let tag = arg_str(cx, args, 0)?;
    let tag = if tag.trim().is_empty() {
        "div"
    } else {
        tag.trim()
    };
    let Some(h) = cx.host_as::<CinderDomHost>() else {
        return Ok(JsValue::NULL);
    };
    let nid = h.doc.create_element(tag, Vec::new());
    wrap_node(cx, nid)
}

fn doc_create_text_node(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let text = arg_str(cx, args, 0)?;
    let Some(h) = cx.host_as::<CinderDomHost>() else {
        return Ok(JsValue::NULL);
    };
    let nid = h.doc.create_text(&text);
    wrap_node(cx, nid)
}

fn doc_create_document_fragment(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(h) = cx.host_as::<CinderDomHost>() else {
        return Ok(JsValue::NULL);
    };
    let nid = h.doc.create_element("anvil-fragment", Vec::new());
    wrap_node(cx, nid)
}

fn doc_get_elements_by_tag_name(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let tag = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.get_elements_by_tag_name(h.doc.root(), &tag))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn doc_get_elements_by_class_name(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let cls = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.get_elements_by_class_name(h.doc.root(), &cls))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn doc_get_elements_by_name(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.get_elements_by_name(&name))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn doc_add_event_listener(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let ev_type = arg_str(cx, args, 0)?.trim().to_ascii_lowercase();
    let Some(cb) = args.get(1).copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    if matches!(
        ev_type.as_str(),
        "domcontentloaded" | "load" | "readystatechange"
    ) {
        cx.enqueue_job(cb, JsValue::UNDEFINED, &[])?;
        return Ok(JsValue::UNDEFINED);
    }
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let root = h.doc.root();
        if !h.push_listener(root, ev_type, pers) {
            h.console.push("[error] listener queue full".to_string());
            return Ok(JsValue::FALSE);
        }
    }
    Ok(JsValue::TRUE)
}

fn doc_remove_event_listener(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let ev_type = arg_str(cx, args, 0)?.trim().to_ascii_lowercase();
    let Some(cb) = args.get(1).copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let root = cx.host_as::<CinderDomHost>().map(|h| h.doc.root());
    if let Some(root_id) = root {
        remove_listener_for_node(cx, root_id, &ev_type, cb);
    }
    Ok(JsValue::UNDEFINED)
}

fn doc_dispatch_event(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let root = cx.host_as::<CinderDomHost>().map(|h| h.doc.root());
    let Some(root_id) = root else {
        return Ok(JsValue::TRUE);
    };
    dispatch_js_event_on_node(cx, root_id, args)
}

fn doc_adopt_node(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = args.first().copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::NULL);
    };
    let parent = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).parent());
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.remove_node(nid);
    }
    if let Some(p) = parent {
        notify_child_mutation(cx, p, &[], &[nid], None, None)?;
    }
    let has_ce = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.has_custom_elements);
    let node_val = wrap_node(cx, nid)?;
    if has_ce {
        cx.keep(node_val);
        let g = cx.vm().global();
        if let Ok(key) = cx.heap().key_from_str("__onCustomElementAdopted")
            && let Ok(Some(prop)) = cx.heap().get_own(g, key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
        {
            let doc_val = el_get_owner_document(cx, JsValue::UNDEFINED, &[])?;
            let _ = cx.call(fn_val, JsValue::object(g), &[node_val, doc_val, doc_val]);
        }
    }
    Ok(node_val)
}

fn doc_import_node(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = args.first().copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::NULL);
    };
    let deep = args
        .get(1)
        .copied()
        .is_none_or(|v| v.is_undefined() || v.as_bool() == Some(true));
    let cloned = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.clone_node(nid, deep));
    wrap_optional_node(cx, cloned)
}

fn doc_parse_html_fragment(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let markup = arg_str(cx, args, 0)?;
    let parsed = parse_html(&markup);
    let Some(h) = cx.host_as::<CinderDomHost>() else {
        return Ok(JsValue::NULL);
    };
    let doc_root = h.doc.create_element("document", Vec::new());
    let head_el = h.doc.create_element("head", Vec::new());
    let body_el = h.doc.create_element("body", Vec::new());
    let _ = h.doc.append(doc_root, head_el);
    let _ = h.doc.append(doc_root, body_el);
    for &c in parsed.children(parsed.head()) {
        if let Some(imported) = h.doc.import_subtree(&parsed, c, true) {
            let _ = h.doc.append(head_el, imported);
        }
    }
    for &c in parsed.children(parsed.body()) {
        if let Some(imported) = h.doc.import_subtree(&parsed, c, true) {
            let _ = h.doc.append(body_el, imported);
        }
    }
    let doc_val = wrap_node(cx, doc_root)?;
    let head_val = wrap_node(cx, head_el)?;
    let body_val = wrap_node(cx, body_el)?;
    if let Some(obj) = doc_val.as_object() {
        cx.vm().define_data(obj, "head", head_val)?;
        cx.vm().define_data(obj, "body", body_val)?;
        cx.vm().define_data(obj, "documentElement", doc_val)?;
    }
    Ok(doc_val)
}

fn doc_add_adopted_sheet(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let css = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.add_adopted_style_sheet(css);
    }
    Ok(JsValue::UNDEFINED)
}

fn doc_set_adopted_sheets(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let joined = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if joined.trim().is_empty() {
            h.doc.set_adopted_style_sheets(Vec::new());
        } else {
            h.doc.set_adopted_style_sheets(vec![joined]);
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn doc_get_active_element(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let active = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.focused_node().or_else(|| Some(h.doc.body())));
    wrap_optional_node(cx, active)
}

fn doc_get_title(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let title = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.title())
        .unwrap_or_default();
    cx.vm().string(&title)
}

fn doc_set_title(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let title = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.set_title(&title);
    }
    Ok(JsValue::UNDEFINED)
}

fn doc_get_body(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let body = cx.host_as::<CinderDomHost>().map(|h| h.doc.body());
    wrap_optional_node(cx, body)
}

fn doc_get_head(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let head = cx.host_as::<CinderDomHost>().map(|h| h.doc.head());
    wrap_optional_node(cx, head)
}

fn doc_get_document_element(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let html = cx.host_as::<CinderDomHost>().map(|h| h.doc.html());
    wrap_optional_node(cx, html)
}

fn doc_get_ready_state(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let state = cx
        .host_as::<CinderDomHost>()
        .map(|host| host.ready_state.as_dom_str())
        .unwrap_or("complete");
    cx.vm().string(state)
}

fn doc_get_url(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let url = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    cx.vm().string(&url)
}

// ---- Element methods & accessors ------------------------------------------

fn el_get_attribute(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let name = arg_str(cx, args, 0)?;
    let val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr(&name).map(str::to_string));
    match val {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn el_set_attribute(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let name = arg_str(cx, args, 0)?;
    let val = arg_str(cx, args, 1)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr(&name).map(str::to_string);
        h.doc.set_attribute(nid, &name, &val);
        prev
    });
    notify_attr_mutation(cx, nid, &name, old_val, Some(val))?;
    Ok(JsValue::UNDEFINED)
}

fn el_remove_attribute(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let name = arg_str(cx, args, 0)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr(&name).map(str::to_string);
        h.doc.remove_attribute(nid, &name);
        prev
    });
    notify_attr_mutation(cx, nid, &name, old_val, None)?;
    Ok(JsValue::UNDEFINED)
}

fn el_has_attribute(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let name = arg_str(cx, args, 0)?;
    let has = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.get(nid).data().attr(&name).is_some());
    Ok(JsValue::boolean(has))
}

fn el_append_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(parent_id) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let child_val = args.first().copied().unwrap_or(JsValue::NULL);
    let Some(child_id) = unwrap_node(cx, child_val) else {
        return Ok(child_val);
    };
    let mut added = Vec::new();
    let mut prev_sib = None;
    let mut old_parent_removed: Option<(NodeId, NodeId)> = None;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        prev_sib = h.doc.last_child(parent_id);
        if h.doc.get(child_id).data().tag() == Some("anvil-fragment") {
            let kids: Vec<NodeId> = h.doc.children(child_id).to_vec();
            for k in kids {
                let _ = h.doc.append(parent_id, k);
                added.push(k);
            }
        } else {
            if let Some(old_p) = h.doc.get(child_id).parent()
                && old_p != parent_id
            {
                old_parent_removed = Some((old_p, child_id));
            }
            let _ = h.doc.append(parent_id, child_id);
            added.push(child_id);
        }
    }
    if let Some((old_p, rem_id)) = old_parent_removed {
        notify_child_mutation(cx, old_p, &[], &[rem_id], None, None)?;
    }
    if !added.is_empty() {
        notify_child_mutation(cx, parent_id, &added, &[], prev_sib, None)?;
    }
    Ok(child_val)
}

fn el_remove_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(parent_id) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let child_val = args.first().copied().unwrap_or(JsValue::NULL);
    let mut removed = Vec::new();
    let mut prev_sib = None;
    let mut next_sib = None;
    if let Some(child_id) = unwrap_node(cx, child_val)
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        prev_sib = h.doc.previous_sibling(child_id);
        next_sib = h.doc.next_sibling(child_id);
        if h.doc.remove_child(parent_id, child_id).is_ok() {
            removed.push(child_id);
        }
    }
    if !removed.is_empty() {
        notify_child_mutation(cx, parent_id, &[], &removed, prev_sib, next_sib)?;
    }
    Ok(child_val)
}

fn el_insert_before(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(parent_id) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let new_val = args.first().copied().unwrap_or(JsValue::NULL);
    let Some(new_id) = unwrap_node(cx, new_val) else {
        return Ok(new_val);
    };
    let ref_id = args.get(1).copied().and_then(|v| unwrap_node(cx, v));
    let mut added = Vec::new();
    let mut prev_sib = None;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        prev_sib = match ref_id {
            Some(rid) => h.doc.previous_sibling(rid),
            None => h.doc.last_child(parent_id),
        };
        if h.doc.get(new_id).data().tag() == Some("anvil-fragment") {
            let kids: Vec<NodeId> = h.doc.children(new_id).to_vec();
            for k in kids {
                let _ = h.doc.insert_before(parent_id, k, ref_id);
                added.push(k);
            }
        } else {
            let _ = h.doc.insert_before(parent_id, new_id, ref_id);
            added.push(new_id);
        }
    }
    if !added.is_empty() {
        notify_child_mutation(cx, parent_id, &added, &[], prev_sib, ref_id)?;
    }
    Ok(new_val)
}

fn el_replace_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(parent_id) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let new_val = args.first().copied().unwrap_or(JsValue::NULL);
    let old_val = args.get(1).copied().unwrap_or(JsValue::NULL);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut prev_sib = None;
    let mut next_sib = None;
    if let (Some(new_id), Some(old_id)) = (unwrap_node(cx, new_val), unwrap_node(cx, old_val))
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        prev_sib = h.doc.previous_sibling(old_id);
        next_sib = h.doc.next_sibling(old_id);
        if h.doc.replace_child(parent_id, new_id, old_id).is_ok() {
            added.push(new_id);
            removed.push(old_id);
        }
    }
    if !added.is_empty() || !removed.is_empty() {
        notify_child_mutation(cx, parent_id, &added, &removed, prev_sib, next_sib)?;
    }
    Ok(old_val)
}

fn el_clone_node(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let deep = args
        .first()
        .copied()
        .is_some_and(|v| v.as_bool() == Some(true));
    let cloned = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.clone_node(nid, deep));
    wrap_optional_node(cx, cloned)
}

fn el_remove(cx: &mut NativeCx<'_>, this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let mut parent = None;
    let mut prev_sib = None;
    let mut next_sib = None;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        parent = h.doc.get(nid).parent();
        prev_sib = h.doc.previous_sibling(nid);
        next_sib = h.doc.next_sibling(nid);
        h.doc.remove_node(nid);
    }
    if let Some(p) = parent {
        notify_child_mutation(cx, p, &[], &[nid], prev_sib, next_sib)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_query_selector(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let sel = arg_str(cx, args, 0)?;
    let found = cx.host_as::<CinderDomHost>().and_then(|h| {
        if nid == h.doc.root() {
            h.doc.query_selector(&sel)
        } else {
            h.doc.query_selector_within(nid, &sel)
        }
    });
    wrap_optional_node(cx, found)
}

fn el_query_selector_all(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().new_array_from(&[]);
    };
    let sel = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            if nid == h.doc.root() {
                h.doc.query_selector_all(&sel)
            } else {
                h.doc.query_selector_all_within(nid, &sel)
            }
        })
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn el_get_element_by_id(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id_str = arg_str(cx, args, 0)?;
    let nid = unwrap_node(cx, this);
    let found = cx.host_as::<CinderDomHost>().and_then(|h| {
        if let Some(sub_root) = nid
            && sub_root != h.doc.root()
        {
            h.doc
                .query_selector_within(sub_root, &format!("#{id_str}"))
                .or_else(|| h.doc.get_element_by_id(&id_str))
        } else {
            h.doc.get_element_by_id(&id_str)
        }
    });
    wrap_optional_node(cx, found)
}

fn el_get_elements_by_tag_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().new_array_from(&[]);
    };
    let tag = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.get_elements_by_tag_name(nid, &tag))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn el_get_elements_by_class_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().new_array_from(&[]);
    };
    let cls = arg_str(cx, args, 0)?;
    let ids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.get_elements_by_class_name(nid, &cls))
        .unwrap_or_default();
    wrap_node_list(cx, ids)
}

fn el_insert_adjacent_html(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let pos = arg_str(cx, args, 0)?;
    let html = arg_str(cx, args, 1)?;
    let mut target_parent = nid;
    let mut added = Vec::new();
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let p_norm = pos.trim().to_ascii_lowercase();
        target_parent = if matches!(p_norm.as_str(), "beforebegin" | "afterend") {
            h.doc.get(nid).parent().unwrap_or(nid)
        } else {
            nid
        };
        let before: Vec<NodeId> = h.doc.children(target_parent).to_vec();
        h.doc.insert_adjacent_html(nid, &pos, &html);
        for &cid in h.doc.children(target_parent) {
            if !before.contains(&cid) && h.doc.shadow_host(cid).is_none() {
                added.push(cid);
            }
        }
    }
    if !added.is_empty() {
        notify_child_mutation(cx, target_parent, &added, &[], None, None)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_insert_adjacent_text(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let pos = arg_str(cx, args, 0)?;
    let text = arg_str(cx, args, 1)?;
    let mut target_parent = nid;
    let mut added = Vec::new();
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let p_norm = pos.trim().to_ascii_lowercase();
        target_parent = if matches!(p_norm.as_str(), "beforebegin" | "afterend") {
            h.doc.get(nid).parent().unwrap_or(nid)
        } else {
            nid
        };
        let before: Vec<NodeId> = h.doc.children(target_parent).to_vec();
        h.doc.insert_adjacent_text(nid, &pos, &text);
        for &cid in h.doc.children(target_parent) {
            if !before.contains(&cid) && h.doc.shadow_host(cid).is_none() {
                added.push(cid);
            }
        }
    }
    if !added.is_empty() {
        notify_child_mutation(cx, target_parent, &added, &[], None, None)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_insert_adjacent_element(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let pos = arg_str(cx, args, 0)?;
    let Some(new_el) = args.get(1).copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::NULL);
    };
    let mut target_parent = nid;
    let inserted = cx.host_as::<CinderDomHost>().and_then(|h| {
        let p_norm = pos.trim().to_ascii_lowercase();
        target_parent = if matches!(p_norm.as_str(), "beforebegin" | "afterend") {
            h.doc.get(nid).parent().unwrap_or(nid)
        } else {
            nid
        };
        h.doc.insert_adjacent_element(nid, &pos, new_el)
    });
    if let Some(ins_id) = inserted {
        notify_child_mutation(cx, target_parent, &[ins_id], &[], None, None)?;
    }
    wrap_optional_node(cx, inserted)
}

fn el_matches(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let sel = arg_str(cx, args, 0)?;
    let matched = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.matches_selector(nid, &sel));
    Ok(JsValue::boolean(matched))
}

fn el_closest(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let sel = arg_str(cx, args, 0)?;
    let found = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.closest(nid, &sel));
    wrap_optional_node(cx, found)
}

fn el_contains(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let Some(other_id) = args.first().copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::FALSE);
    };
    let contains = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.contains_node(nid, other_id));
    Ok(JsValue::boolean(contains))
}

fn el_attach_shadow(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let mut open = true;
    if let Some(init_obj) = args.first().copied().and_then(|v| v.as_object())
        && let Ok(key) = cx.heap().key_from_str("mode")
        && let Ok(Some(prop)) = cx.heap().get_own(init_obj, key)
        && let Some(val) = prop.slot.data()
        && let Ok(mode_str) = cx.to_display_string(val)
        && mode_str.trim().eq_ignore_ascii_case("closed")
    {
        open = false;
    }
    let sr = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.attach_shadow(nid, open));
    wrap_optional_node(cx, sr)
}

fn el_assigned_nodes(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return wrap_node_list(cx, Vec::new());
    };
    let mut flatten = false;
    if let Some(opts_obj) = args.first().copied().and_then(|v| v.as_object())
        && let Ok(key) = cx.heap().key_from_str("flatten")
        && let Ok(Some(prop)) = cx.heap().get_own(opts_obj, key)
        && let Some(val) = prop.slot.data()
    {
        flatten = val.as_bool() == Some(true);
    }
    let assigned = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.assigned_nodes(nid, flatten))
        .unwrap_or_default();
    wrap_node_list(cx, assigned)
}

fn el_assigned_elements(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return wrap_node_list(cx, Vec::new());
    };
    let mut flatten = false;
    if let Some(opts_obj) = args.first().copied().and_then(|v| v.as_object())
        && let Ok(key) = cx.heap().key_from_str("flatten")
        && let Ok(Some(prop)) = cx.heap().get_own(opts_obj, key)
        && let Some(val) = prop.slot.data()
    {
        flatten = val.as_bool() == Some(true);
    }
    let assigned = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.assigned_elements(nid, flatten))
        .unwrap_or_default();
    wrap_node_list(cx, assigned)
}

fn el_get_slot(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let s = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("slot").map(str::to_string))
        .unwrap_or_default();
    cx.vm().string(&s)
}

fn el_set_slot(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let val = arg_str(cx, args, 0)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr("slot").map(str::to_string);
        h.doc.set_attribute(nid, "slot", &val);
        prev
    });
    notify_attr_mutation(cx, nid, "slot", old_val, Some(val))?;
    Ok(JsValue::UNDEFINED)
}

fn el_get_assigned_slot(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let slot_id = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.assigned_slot(nid));
    wrap_optional_node(cx, slot_id)
}

fn el_get_shadow_host(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let host_id = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.shadow_host(nid));
    wrap_optional_node(cx, host_id)
}

fn el_get_shadow_mode(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let mode = cx.host_as::<CinderDomHost>().and_then(|h| {
        let host_id = h.doc.shadow_host(nid)?;
        if h.doc.shadow_root(host_id) == Some(nid) {
            Some("open")
        } else {
            Some("closed")
        }
    });
    match mode {
        Some(m) => cx.vm().string(m),
        None => Ok(JsValue::UNDEFINED),
    }
}

fn el_add_event_listener(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let ev_type = arg_str(cx, args, 0)?.trim().to_ascii_lowercase();
    let Some(cb) = args.get(1).copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if !h.push_listener(nid, ev_type, pers) {
            h.console.push("[error] listener queue full".to_string());
            return Ok(JsValue::FALSE);
        }
    }
    Ok(JsValue::TRUE)
}

fn el_remove_event_listener(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let ev_type = arg_str(cx, args, 0)?.trim().to_ascii_lowercase();
    let Some(cb) = args.get(1).copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    remove_listener_for_node(cx, nid, &ev_type, cb);
    Ok(JsValue::UNDEFINED)
}

fn remove_listener_for_node(cx: &mut NativeCx<'_>, nid: NodeId, ev_type: &str, cb: JsValue) {
    let Some(listeners) = cx.host_as::<CinderDomHost>().map(|h| h.listeners.clone()) else {
        return;
    };
    let mut kept = Vec::with_capacity(listeners.len());
    for (lnid, lev, lpers) in listeners {
        let is_same = lnid == nid
            && lev == ev_type
            && cx
                .heap()
                .persistent(lpers)
                .is_ok_and(|stored| stored.as_object() == cb.as_object());
        if !is_same {
            kept.push((lnid, lev, lpers));
        }
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.listeners = kept;
    }
}

fn el_dispatch_event(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::TRUE);
    };
    dispatch_js_event_on_node(cx, nid, args)
}

fn dispatch_js_event_on_node(
    cx: &mut NativeCx<'_>,
    target: NodeId,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(ev_val) = args.first().copied() else {
        return Ok(JsValue::TRUE);
    };
    let Some(ev_obj) = ev_val.as_object() else {
        return Ok(JsValue::TRUE);
    };
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.event_default_prevented = false;
    }
    cx.vm().define_accessor(
        ev_obj,
        "defaultPrevented",
        Some(ev_get_default_prevented),
        None,
    )?;
    cx.vm()
        .define_method(ev_obj, "preventDefault", ev_prevent_default)?;
    cx.vm()
        .define_method(ev_obj, "stopPropagation", ev_stop_propagation)?;
    let type_key = cx
        .heap()
        .key_from_str("type")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let ev_type = if let Ok(Some(prop)) = cx.heap().get_own(ev_obj, type_key)
        && let Some(val) = prop.slot.data()
    {
        cx.to_display_string(val)?.trim().to_ascii_lowercase()
    } else {
        String::new()
    };
    if ev_type.is_empty() {
        return Ok(JsValue::TRUE);
    }
    let bubbles_key = cx
        .heap()
        .key_from_str("bubbles")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let bubbles = if let Ok(Some(prop)) = cx.heap().get_own(ev_obj, bubbles_key)
        && let Some(val) = prop.slot.data()
    {
        val.as_bool().unwrap_or(false)
    } else {
        false
    };
    let target_val = wrap_node(cx, target)?;
    cx.vm().define_data(ev_obj, "target", target_val)?;

    let mut chain = vec![target];
    if bubbles && let Some(h) = cx.host_as::<CinderDomHost>() {
        let mut cur = h.doc.get(target).parent();
        while let Some(p) = cur {
            chain.push(p);
            cur = h.doc.get(p).parent();
        }
    }
    let stopped_key = cx
        .heap()
        .key_from_str("_stopped")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    for current_id in chain {
        if let Ok(Some(prop)) = cx.heap().get_own(ev_obj, stopped_key)
            && prop.slot.data().and_then(|v| v.as_bool()) == Some(true)
        {
            break;
        }
        let matching: Vec<Persistent> = cx
            .host_as::<CinderDomHost>()
            .map(|h| {
                h.listeners
                    .iter()
                    .filter(|(nid, ev, _)| *nid == current_id && ev == &ev_type)
                    .map(|(_, _, cb)| *cb)
                    .collect()
            })
            .unwrap_or_default();
        if matching.is_empty() {
            continue;
        }
        let cur_val = wrap_node(cx, current_id)?;
        cx.vm().define_data(ev_obj, "currentTarget", cur_val)?;
        for cb_pers in matching {
            let cb_val = cx
                .heap()
                .persistent(cb_pers)
                .map_err(|e| RunError::internal(format!("{e:?}")))?;
            let _ = cx.call(cb_val, cur_val, &[ev_val])?;
        }
    }
    let dp_key = cx
        .heap()
        .key_from_str("defaultPrevented")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let prevented = if let Ok(Some(prop)) = cx.heap().get_own(ev_obj, dp_key)
        && let Some(val) = prop.slot.data()
    {
        val.as_bool().unwrap_or(false)
    } else {
        false
    } || cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.event_default_prevented);
    Ok(JsValue::boolean(!prevented))
}

fn el_focus(cx: &mut NativeCx<'_>, this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        h.doc.set_focused_node(Some(nid));
    }
    Ok(JsValue::UNDEFINED)
}

fn el_blur(cx: &mut NativeCx<'_>, this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(h) = cx.host_as::<CinderDomHost>()
        && h.doc.focused_node() == Some(nid)
    {
        h.doc.set_focused_node(None);
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_bounding_client_rect(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let (x, y, w, h) = if let Some(nid) = unwrap_node(cx, this)
        && let Some(host) = cx.host_as::<CinderDomHost>()
    {
        if let Some(b) = host.compute_node_box(nid) {
            let styles = host.compute_styles();
            let cs = styles.get(nid);
            let inline_w = read_inline_style(&host.doc, nid, "width");
            let inline_h = read_inline_style(&host.doc, nid, "height");
            let sized = matches!(cs.width, anvil_style::Size::Px(_))
                || matches!(cs.height, anvil_style::Size::Px(_))
                || !inline_w.is_empty()
                || !inline_h.is_empty();
            // An inline box with no specified size keeps the page's 100 by 24
            // rectangle. A specified size still comes from layout.
            if cs.display == anvil_style::Display::Inline && !sized {
                (0.0, 0.0, 100.0, 24.0)
            } else {
                let (dx, dy) = b.sticky.map_or((0.0, 0.0), |s| {
                    s.offset(anvil_layout::Rect {
                        x: 0.0,
                        y: host.viewport_scroll_y,
                        width: host.viewport_width,
                        height: host.viewport_height,
                    })
                });
                (
                    (b.rect.x + dx) as f64,
                    (b.rect.y + dy - host.viewport_scroll_y) as f64,
                    b.rect.width as f64,
                    b.rect.height as f64,
                )
            }
        } else {
            let styles = host.compute_styles();
            let cs = styles.get(nid);
            let inline = host
                .doc
                .get(nid)
                .data()
                .attr("style")
                .map(str::to_string)
                .unwrap_or_default();
            let parse_px = |prop: &str| -> Option<f64> {
                let s = read_inline_style(&host.doc, nid, prop);
                if s.is_empty() {
                    None
                } else {
                    s.trim().trim_end_matches("px").trim().parse::<f64>().ok()
                }
            };
            let size_px = |sz: anvil_style::Size| -> Option<f64> {
                match sz {
                    anvil_style::Size::Px(v) => Some(v as f64),
                    _ => None,
                }
            };
            if cs.display == anvil_style::Display::None && inline.is_empty() {
                (0.0, 0.0, 0.0, 0.0)
            } else {
                (
                    parse_px("left").or_else(|| size_px(cs.left)).unwrap_or(0.0),
                    parse_px("top").or_else(|| size_px(cs.top)).unwrap_or(0.0),
                    parse_px("width")
                        .or_else(|| size_px(cs.width))
                        .unwrap_or(100.0),
                    parse_px("height")
                        .or_else(|| size_px(cs.height))
                        .unwrap_or(24.0),
                )
            }
        }
    } else {
        (0.0, 0.0, 100.0, 24.0)
    };
    let rect = cx.vm().new_plain_object()?;
    for (k, v) in [
        ("x", x),
        ("y", y),
        ("top", y),
        ("left", x),
        ("width", w),
        ("height", h),
        ("right", x + w),
        ("bottom", y + h),
    ] {
        cx.vm().define_data(rect, k, JsValue::number(v))?;
    }
    Ok(JsValue::object(rect))
}

fn el_to_data_url(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("data:,");
    };
    let data_url = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.canvas_to_data_url(nid))
        .unwrap_or_else(|| "data:,".to_string());
    cx.vm().string(&data_url)
}

fn el_get_context(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let ctx_obj = cx
        .vm()
        .new_host_object(None, HostData(TAG_NODE | (nid.index() as u64)))?;
    let black = cx.vm().string("#000000")?;
    let font = cx.vm().string("10px sans-serif")?;
    cx.vm().define_data(ctx_obj, "canvas", this)?;
    cx.vm().define_data(ctx_obj, "fillStyle", black)?;
    cx.vm().define_data(ctx_obj, "strokeStyle", black)?;
    cx.vm().define_data(ctx_obj, "font", font)?;
    cx.vm()
        .define_data(ctx_obj, "lineWidth", JsValue::number(1.0))?;
    cx.vm()
        .define_data(ctx_obj, "globalAlpha", JsValue::number(1.0))?;
    for (name, func) in [
        ("fillRect", canvas_fill_rect as NativeFn),
        ("strokeRect", canvas_stroke_rect),
        ("clearRect", canvas_clear_rect),
        ("fillText", canvas_fill_text),
        ("strokeText", canvas_fill_text),
        ("beginPath", canvas_begin_path),
        ("moveTo", canvas_move_to),
        ("lineTo", canvas_line_to),
        ("rect", canvas_rect),
        ("arc", canvas_arc),
        ("closePath", canvas_close_path),
        ("fill", canvas_fill),
        ("stroke", canvas_stroke),
        ("measureText", canvas_measure_text),
        ("getImageData", canvas_get_image_data),
        ("putImageData", canvas_put_image_data),
        ("createImageData", canvas_create_image_data),
        ("drawImage", canvas_draw_image),
        ("save", canvas_save),
        ("restore", canvas_restore),
        ("scale", canvas_scale),
        ("translate", canvas_translate),
        ("rotate", el_noop),
        ("transform", el_noop),
        ("setTransform", canvas_set_transform),
        ("resetTransform", canvas_reset_transform),
        ("setLineDash", el_noop),
        ("clip", el_noop),
        ("__fillLinearGradient", canvas_fill_linear_gradient),
    ] {
        cx.vm().define_method(ctx_obj, name, func)?;
    }
    Ok(JsValue::object(ctx_obj))
}

fn canvas_get_transform(host: &CinderDomHost, nid: NodeId) -> (f64, f64, f64, f64) {
    host.canvas_transform
        .get(&nid)
        .copied()
        .unwrap_or((0.0, 0.0, 1.0, 1.0))
}

fn canvas_map_point(host: &CinderDomHost, nid: NodeId, x: f64, y: f64) -> (i32, i32) {
    let (tx, ty, sx, sy) = canvas_get_transform(host, nid);
    ((tx + x * sx).round() as i32, (ty + y * sy).round() as i32)
}

fn canvas_map_rect(
    host: &CinderDomHost,
    nid: NodeId,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> (i32, i32, u32, u32) {
    let (tx, ty, sx, sy) = canvas_get_transform(host, nid);
    let mx = (tx + x * sx).round() as i32;
    let my = (ty + y * sy).round() as i32;
    let mw = (w * sx.abs()).round().max(0.0) as u32;
    let mh = (h * sy.abs()).round().max(0.0) as u32;
    (mx, my, mw, mh)
}

fn canvas_prop_str(cx: &mut NativeCx<'_>, this: JsValue, prop_name: &str, default: &str) -> String {
    if let Some(obj) = this.as_object()
        && let Ok(key) = cx.heap().key_from_str(prop_name)
        && let Ok(Some(prop)) = cx.heap().get_own(obj, key)
        && let Some(val) = prop.slot.data()
    {
        cx.to_display_string(val)
            .unwrap_or_else(|_| default.to_string())
    } else {
        default.to_string()
    }
}

fn canvas_prop_num(cx: &mut NativeCx<'_>, this: JsValue, prop_name: &str, default: f64) -> f64 {
    if let Some(obj) = this.as_object()
        && let Ok(key) = cx.heap().key_from_str(prop_name)
        && let Ok(Some(prop)) = cx.heap().get_own(obj, key)
        && let Some(val) = prop.slot.data()
        && let Some(n) = val.as_number()
    {
        n
    } else {
        default
    }
}

fn canvas_style_color(cx: &mut NativeCx<'_>, this: JsValue, prop_name: &str) -> [u8; 4] {
    let style_str = canvas_prop_str(cx, this, prop_name, "#000000");
    let mut rgba = crate::parse_canvas_color(&style_str);
    let alpha = canvas_prop_num(cx, this, "globalAlpha", 1.0).clamp(0.0, 1.0);
    if alpha < 1.0 {
        rgba[3] = ((rgba[3] as f64) * alpha).round().clamp(0.0, 255.0) as u8;
    }
    rgba
}

fn canvas_save(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let fill = canvas_prop_str(cx, this, "fillStyle", "#000000");
    let stroke = canvas_prop_str(cx, this, "strokeStyle", "#000000");
    let lw = canvas_prop_num(cx, this, "lineWidth", 1.0);
    let alpha = canvas_prop_num(cx, this, "globalAlpha", 1.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (tx, ty, sx, sy) = canvas_get_transform(host, nid);
        host.canvas_stacks
            .entry(nid)
            .or_default()
            .push((tx, ty, sx, sy, fill, stroke, lw, alpha));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_restore(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let popped = cx.host_as::<CinderDomHost>().and_then(|host| {
        let st = host.canvas_stacks.get_mut(&nid)?.pop()?;
        host.canvas_transform.insert(nid, (st.0, st.1, st.2, st.3));
        Some((st.4, st.5, st.6, st.7))
    });
    if let (Some(obj), Some((fill, stroke, lw, alpha))) = (this.as_object(), popped) {
        let fill_js = cx.vm().string(&fill)?;
        let stroke_js = cx.vm().string(&stroke)?;
        cx.vm().define_data(obj, "fillStyle", fill_js)?;
        cx.vm().define_data(obj, "strokeStyle", stroke_js)?;
        cx.vm().define_data(obj, "lineWidth", JsValue::number(lw))?;
        cx.vm()
            .define_data(obj, "globalAlpha", JsValue::number(alpha))?;
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_translate(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let dx = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let dy = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (tx, ty, sx, sy) = canvas_get_transform(host, nid);
        host.canvas_transform
            .insert(nid, (tx + dx * sx, ty + dy * sy, sx, sy));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_scale(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let dsx = args.first().and_then(|v| v.as_number()).unwrap_or(1.0);
    let dsy = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (tx, ty, sx, sy) = canvas_get_transform(host, nid);
        host.canvas_transform
            .insert(nid, (tx, ty, sx * dsx, sy * dsy));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_reset_transform(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(host) = cx.host_as::<CinderDomHost>()
    {
        host.canvas_transform.insert(nid, (0.0, 0.0, 1.0, 1.0));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_set_transform(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    if args.is_empty() {
        return canvas_reset_transform(cx, this, args);
    }
    let a = args.first().and_then(|v| v.as_number()).unwrap_or(1.0);
    let d = args.get(3).and_then(|v| v.as_number()).unwrap_or(1.0);
    let e = args.get(4).and_then(|v| v.as_number()).unwrap_or(0.0);
    let f = args.get(5).and_then(|v| v.as_number()).unwrap_or(0.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        host.canvas_transform.insert(nid, (e, f, a, d));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_draw_image(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let Some(src_nid) = args.first().copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::UNDEFINED);
    };
    let dx = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let dy = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
    let dw = args
        .get(3)
        .and_then(|v| v.as_number())
        .map(|n| n.max(0.0) as u32);
    let dh = args
        .get(4)
        .and_then(|v| v.as_number())
        .map(|n| n.max(0.0) as u32);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (mx, my) = canvas_map_point(host, nid, dx, dy);
        let (_, _, sx, sy) = canvas_get_transform(host, nid);
        let scaled_dw = dw.map(|w| ((w as f64) * sx.abs()).round().max(1.0) as u32);
        let scaled_dh = dh.map(|h| ((h as f64) * sy.abs()).round().max(1.0) as u32);
        host.doc
            .canvas_draw_canvas(nid, src_nid, mx, my, scaled_dw, scaled_dh);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_fill_rect(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let w = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let h = args
        .get(3)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let rgba = canvas_style_color(cx, this, "fillStyle");
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (mx, my, mw, mh) = canvas_map_rect(host, nid, x, y, w, h);
        host.doc.canvas_fill_rect(nid, mx, my, mw, mh, rgba);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_stroke_rect(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let w = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let h = args
        .get(3)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let rgba = canvas_style_color(cx, this, "strokeStyle");
    let lw = canvas_prop_num(cx, this, "lineWidth", 1.0).round().max(1.0) as u32;
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (mx, my, mw, mh) = canvas_map_rect(host, nid, x, y, w, h);
        host.doc.canvas_stroke_rect(nid, mx, my, mw, mh, lw, rgba);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_clear_rect(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let w = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let h = args
        .get(3)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (mx, my, mw, mh) = canvas_map_rect(host, nid, x, y, w, h);
        host.doc.canvas_clear_rect(nid, mx, my, mw, mh);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_fill_text(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let text = arg_str(cx, args, 0)?;
    let x = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
    let rgba = canvas_style_color(cx, this, "fillStyle");
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (mx, my) = canvas_map_point(host, nid, x, y);
        host.doc.canvas_draw_text_bitmap(nid, &text, mx, my, rgba);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_begin_path(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        h.canvas_paths.insert(nid, Vec::new());
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_move_to(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let (mx, my) = canvas_map_point(h, nid, x, y);
        h.canvas_paths.entry(nid).or_default().push((mx, my));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_line_to(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    canvas_move_to(cx, this, args)
}

fn canvas_rect(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let w = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0);
    let h = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let (p0x, p0y) = canvas_map_point(host, nid, x, y);
        let (p1x, p1y) = canvas_map_point(host, nid, x + w, y);
        let (p2x, p2y) = canvas_map_point(host, nid, x + w, y + h);
        let (p3x, p3y) = canvas_map_point(host, nid, x, y + h);
        let pts = host.canvas_paths.entry(nid).or_default();
        pts.push((p0x, p0y));
        pts.push((p1x, p1y));
        pts.push((p2x, p2y));
        pts.push((p3x, p3y));
        pts.push((p0x, p0y));
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_arc(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let cx_pt = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
    let cy_pt = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let r = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0);
    let a0 = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0);
    let a1 = args
        .get(4)
        .and_then(|v| v.as_number())
        .unwrap_or(std::f64::consts::TAU);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let mut mapped = Vec::with_capacity(17);
        let steps = 16usize;
        for i in 0..=steps {
            let t = a0 + (a1 - a0) * (i as f64 / steps as f64);
            let px = cx_pt + r * t.cos();
            let py = cy_pt + r * t.sin();
            mapped.push(canvas_map_point(host, nid, px, py));
        }
        host.canvas_paths.entry(nid).or_default().extend(mapped);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_close_path(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(host) = cx.host_as::<CinderDomHost>()
        && let Some(pts) = host.canvas_paths.get_mut(&nid)
        && let Some(&first) = pts.first()
    {
        pts.push(first);
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_stroke(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let rgba = canvas_style_color(cx, this, "strokeStyle");
    let lw = if let Some(obj) = this.as_object()
        && let Ok(key) = cx.heap().key_from_str("lineWidth")
        && let Ok(Some(prop)) = cx.heap().get_own(obj, key)
        && let Some(val) = prop.slot.data()
        && let Some(n) = val.as_number()
    {
        n.round().max(1.0) as u32
    } else {
        1
    };
    if let Some(host) = cx.host_as::<CinderDomHost>()
        && let Some(pts) = host.canvas_paths.get(&nid).cloned()
    {
        for win in pts.windows(2) {
            host.doc
                .canvas_draw_line_width(nid, win[0].0, win[0].1, win[1].0, win[1].1, lw, rgba);
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_fill(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let rgba = canvas_style_color(cx, this, "fillStyle");
    if let Some(host) = cx.host_as::<CinderDomHost>()
        && let Some(pts) = host.canvas_paths.get(&nid).cloned()
        && !pts.is_empty()
    {
        if pts.len() >= 3 {
            let poly: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x as f32, y as f32)).collect();
            host.doc.canvas_fill_polygon(nid, &poly, rgba);
        } else {
            let min_x = pts.iter().map(|p| p.0).min().unwrap_or(0);
            let max_x = pts.iter().map(|p| p.0).max().unwrap_or(0);
            let min_y = pts.iter().map(|p| p.1).min().unwrap_or(0);
            let max_y = pts.iter().map(|p| p.1).max().unwrap_or(0);
            let w = (max_x - min_x).max(1) as u32;
            let h = (max_y - min_y).max(1) as u32;
            host.doc.canvas_fill_rect(nid, min_x, min_y, w, h, rgba);
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn canvas_measure_text(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let text = arg_str(cx, args, 0)?;
    let metrics = cx.vm().new_plain_object()?;
    let w = (text.chars().count() as f64) * 6.0;
    cx.vm().define_data(metrics, "width", JsValue::number(w))?;
    Ok(JsValue::object(metrics))
}

fn canvas_create_image_data(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let w = args
        .first()
        .and_then(|v| v.as_number())
        .unwrap_or(1.0)
        .clamp(1.0, 256.0) as u32;
    let h = args
        .get(1)
        .and_then(|v| v.as_number())
        .unwrap_or(1.0)
        .clamp(1.0, 256.0) as u32;
    let len = (w as usize) * (h as usize) * 4;
    let zeros = vec![JsValue::int32(0); len];
    let data_arr = cx.vm().new_array_from(&zeros)?;
    cx.keep(data_arr);
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "width", JsValue::number(w as f64))?;
    cx.vm()
        .define_data(obj, "height", JsValue::number(h as f64))?;
    cx.vm().define_data(obj, "data", data_arr)?;
    Ok(JsValue::object(obj))
}

fn canvas_get_image_data(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return canvas_create_image_data(cx, this, args);
    };
    let sx = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let sy = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let sw = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(1.0)
        .clamp(1.0, 256.0) as u32;
    let sh = args
        .get(3)
        .and_then(|v| v.as_number())
        .unwrap_or(1.0)
        .clamp(1.0, 256.0) as u32;
    let mut bytes = Vec::with_capacity((sw as usize) * (sh as usize) * 4);
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        let surf = host.doc.ensure_canvas_surface(nid);
        for ry in 0..(sh as i32) {
            for rx in 0..(sw as i32) {
                let px = sx + rx;
                let py = sy + ry;
                if px >= 0 && px < surf.width as i32 && py >= 0 && py < surf.height as i32 {
                    let off = ((py as usize) * (surf.width as usize) + (px as usize)) * 4;
                    if off + 4 <= surf.rgba.len() {
                        for &b in &surf.rgba[off..off + 4] {
                            bytes.push(JsValue::int32(b as i32));
                        }
                        continue;
                    }
                }
                bytes.extend_from_slice(&[JsValue::int32(0); 4]);
            }
        }
    }
    let data_arr = cx.vm().new_array_from(&bytes)?;
    cx.keep(data_arr);
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "width", JsValue::number(sw as f64))?;
    cx.vm()
        .define_data(obj, "height", JsValue::number(sh as f64))?;
    cx.vm().define_data(obj, "data", data_arr)?;
    Ok(JsValue::object(obj))
}

fn canvas_put_image_data(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let Some(img_obj) = args.first().and_then(|v| v.as_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let dx = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let dy = args.get(2).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let w_key = cx
        .heap()
        .key_from_str("width")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let h_key = cx
        .heap()
        .key_from_str("height")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let d_key = cx
        .heap()
        .key_from_str("data")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let sw = cx
        .heap()
        .get_own(img_obj, w_key)
        .ok()
        .flatten()
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_number())
        .unwrap_or(0.0) as u32;
    let sh = cx
        .heap()
        .get_own(img_obj, h_key)
        .ok()
        .flatten()
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_number())
        .unwrap_or(0.0) as u32;
    let data_obj = cx
        .heap()
        .get_own(img_obj, d_key)
        .ok()
        .flatten()
        .and_then(|p| p.slot.data())
        .and_then(|v| v.as_object());
    if sw > 0
        && sh > 0
        && let Some(arr_ref) = data_obj
    {
        let mut rgba = Vec::new();
        if let Ok(obj) = cx.heap().object(arr_ref)
            && let ObjectClass::Array { length } = obj.class
        {
            rgba.reserve(length as usize);
            for i in 0..length {
                let v = obj
                    .elements
                    .get(i)
                    .and_then(|p| p.slot.data())
                    .unwrap_or(JsValue::int32(0));
                let b = v.as_number().unwrap_or(0.0).clamp(0.0, 255.0) as u8;
                rgba.push(b);
            }
        }
        if !rgba.is_empty()
            && let Some(host) = cx.host_as::<CinderDomHost>()
        {
            host.doc.canvas_put_image_data(nid, dx, dy, sw, sh, &rgba);
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn el_noop(_cx: &mut NativeCx<'_>, _this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    Ok(JsValue::UNDEFINED)
}

/// Negative and NaN delays are 0. A positive infinity stays after every
/// finite delay. A timer scheduled from a timer callback waits at least 4 ms.
fn normalize_timer_delay(raw: f64, nested: bool) -> f64 {
    let mut delay = if raw.is_nan() || raw < 0.0 {
        0.0
    } else if raw.is_infinite() {
        1.0e300
    } else {
        raw
    };
    if nested && delay < 4.0 {
        delay = 4.0;
    }
    delay
}

fn window_set_timeout(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(cb) = args.first().copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::int32(0));
    };
    let nested = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.timer_depth > 0);
    let delay = normalize_timer_delay(
        args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0),
        nested,
    );
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let id = if let Some(h) = cx.host_as::<CinderDomHost>() {
        if h.timers.len() >= MAX_HOST_TIMERS {
            h.console.push("[error] timer queue full".to_string());
            0
        } else {
            let id = h.next_timer_id;
            h.next_timer_id = h.next_timer_id.wrapping_add(1).max(1);
            let due = h.clock_ms + delay;
            let seq = h.schedule_seq;
            h.schedule_seq = h.schedule_seq.wrapping_add(1);
            h.timers.push(HostTimer {
                id,
                due,
                interval: None,
                callback: pers,
                is_raf: false,
                task: h.owner_task,
                seq,
                spawn: h.timer_spawn,
            });
            id
        }
    } else {
        0
    };
    Ok(JsValue::number(id as f64))
}

fn window_set_interval(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(cb) = args.first().copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::int32(0));
    };
    let nested = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.timer_depth > 0);
    let interval = normalize_timer_delay(
        args.get(1).and_then(|v| v.as_number()).unwrap_or(10.0),
        nested,
    )
    .max(1.0);
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let id = if let Some(h) = cx.host_as::<CinderDomHost>() {
        if h.timers.len() >= MAX_HOST_TIMERS {
            h.console.push("[error] timer queue full".to_string());
            0
        } else {
            let id = h.next_timer_id;
            h.next_timer_id = h.next_timer_id.wrapping_add(1).max(1);
            let due = h.clock_ms + interval;
            let seq = h.schedule_seq;
            h.schedule_seq = h.schedule_seq.wrapping_add(1);
            h.timers.push(HostTimer {
                id,
                due,
                interval: Some(interval),
                callback: pers,
                is_raf: false,
                task: h.owner_task,
                seq,
                spawn: h.timer_spawn,
            });
            id
        }
    } else {
        0
    };
    Ok(JsValue::number(id as f64))
}

fn window_request_animation_frame(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(cb) = args.first().copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::int32(0));
    };
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    let id = if let Some(h) = cx.host_as::<CinderDomHost>() {
        if h.timers.len() >= MAX_HOST_TIMERS {
            h.console.push("[error] timer queue full".to_string());
            0
        } else {
            let id = h.next_timer_id;
            h.next_timer_id = h.next_timer_id.wrapping_add(1).max(1);
            let due = h.clock_ms + 16.0;
            let seq = h.schedule_seq;
            h.schedule_seq = h.schedule_seq.wrapping_add(1);
            h.timers.push(HostTimer {
                id,
                due,
                interval: None,
                callback: pers,
                is_raf: true,
                task: h.owner_task,
                seq,
                spawn: h.timer_spawn,
            });
            id
        }
    } else {
        0
    };
    Ok(JsValue::number(id as f64))
}

fn window_clear_timer(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(id_num) = args.first().and_then(|v| v.as_number()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let target_id = id_num as u32;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.timers.retain(|timer| timer.id != target_id);
    }
    Ok(JsValue::UNDEFINED)
}

fn window_register_mutation_observer(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(cb) = args.first().copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let Some(nid) = args.get(1).copied().and_then(|v| unwrap_node(cx, v)) else {
        return Ok(JsValue::UNDEFINED);
    };
    let pers = cx
        .heap()
        .persist(cb)
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if h.mutation_observers.len() < MAX_HOST_OBSERVERS {
            h.mutation_observers.push((pers, nid));
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn window_unregister_mutation_observer(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(cb) = args.first().copied().filter(|v| v.is_object()) else {
        return Ok(JsValue::UNDEFINED);
    };
    let Some(existing) = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.mutation_observers.clone())
    else {
        return Ok(JsValue::UNDEFINED);
    };
    let mut kept = Vec::new();
    for (pers, nid) in existing {
        if cx.heap().persistent(pers).ok().and_then(|v| v.as_object()) != cb.as_object() {
            kept.push((pers, nid));
        }
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.mutation_observers = kept;
    }
    Ok(JsValue::UNDEFINED)
}

fn notify_attr_mutation(
    cx: &mut NativeCx<'_>,
    target: NodeId,
    attr_name: &str,
    old_val: Option<String>,
    new_val: Option<String>,
) -> Result<(), RunError> {
    let (matching, has_ce): (Vec<Persistent>, bool) = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let m = h
                .mutation_observers
                .iter()
                .filter(|(_, obs_nid)| h.doc.contains_node(*obs_nid, target))
                .map(|(pers, _)| *pers)
                .collect();
            (m, h.has_custom_elements)
        })
        .unwrap_or_default();
    if matching.is_empty() && !has_ce {
        return Ok(());
    }
    let target_val = wrap_node(cx, target)?;
    cx.keep(target_val);
    if !matching.is_empty() {
        let rec = cx.vm().new_plain_object()?;
        let mtype_val = cx.vm().string("attributes")?;
        cx.vm().define_data(rec, "type", mtype_val)?;
        cx.vm().define_data(rec, "target", target_val)?;
        let empty_added = cx.vm().new_array_from(&[])?;
        cx.keep(empty_added);
        let empty_removed = cx.vm().new_array_from(&[])?;
        cx.keep(empty_removed);
        cx.vm().define_data(rec, "addedNodes", empty_added)?;
        cx.vm().define_data(rec, "removedNodes", empty_removed)?;
        cx.vm().define_data(rec, "previousSibling", JsValue::NULL)?;
        cx.vm().define_data(rec, "nextSibling", JsValue::NULL)?;
        let attr_val = cx.vm().string(attr_name)?;
        cx.vm().define_data(rec, "attributeName", attr_val)?;
        cx.vm()
            .define_data(rec, "attributeNamespace", JsValue::NULL)?;
        let old_js = match old_val.as_deref() {
            Some(s) => cx.vm().string(s)?,
            None => JsValue::NULL,
        };
        cx.vm().define_data(rec, "oldValue", old_js)?;
        let rec_val = JsValue::object(rec);
        cx.keep(rec_val);
        for pers in matching {
            if let Ok(cb_val) = cx.heap().persistent(pers) {
                let _ = cx.call(cb_val, JsValue::UNDEFINED, &[rec_val]);
            }
        }
    }
    if has_ce {
        let g = cx.vm().global();
        if let Ok(key) = cx.heap().key_from_str("__onCustomElementAttributeChanged")
            && let Ok(Some(prop)) = cx.heap().get_own(g, key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
        {
            let name_js = cx.vm().string(attr_name)?;
            cx.keep(name_js);
            let old_js = match old_val.as_deref() {
                Some(s) => cx.vm().string(s)?,
                None => JsValue::NULL,
            };
            cx.keep(old_js);
            let new_js = match new_val.as_deref() {
                Some(s) => cx.vm().string(s)?,
                None => JsValue::NULL,
            };
            cx.keep(new_js);
            let _ = cx.call(
                fn_val,
                JsValue::object(g),
                &[target_val, name_js, old_js, new_js],
            );
        }
    }
    Ok(())
}

fn notify_child_mutation(
    cx: &mut NativeCx<'_>,
    target: NodeId,
    added: &[NodeId],
    removed: &[NodeId],
    prev_sib: Option<NodeId>,
    next_sib: Option<NodeId>,
) -> Result<(), RunError> {
    let (matching, has_ce): (Vec<Persistent>, bool) = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let m = h
                .mutation_observers
                .iter()
                .filter(|(_, obs_nid)| h.doc.contains_node(*obs_nid, target))
                .map(|(pers, _)| *pers)
                .collect();
            (m, h.has_custom_elements)
        })
        .unwrap_or_default();
    if matching.is_empty() && !has_ce {
        return Ok(());
    }
    let target_val = wrap_node(cx, target)?;
    cx.keep(target_val);
    let added_arr = wrap_node_list(cx, added.to_vec())?;
    cx.keep(added_arr);
    let removed_arr = wrap_node_list(cx, removed.to_vec())?;
    cx.keep(removed_arr);
    if !matching.is_empty() {
        let prev_val = wrap_optional_node(cx, prev_sib)?;
        cx.keep(prev_val);
        let next_val = wrap_optional_node(cx, next_sib)?;
        cx.keep(next_val);
        let rec = cx.vm().new_plain_object()?;
        let mtype_val = cx.vm().string("childList")?;
        cx.vm().define_data(rec, "type", mtype_val)?;
        cx.vm().define_data(rec, "target", target_val)?;
        cx.vm().define_data(rec, "addedNodes", added_arr)?;
        cx.vm().define_data(rec, "removedNodes", removed_arr)?;
        cx.vm().define_data(rec, "previousSibling", prev_val)?;
        cx.vm().define_data(rec, "nextSibling", next_val)?;
        cx.vm().define_data(rec, "attributeName", JsValue::NULL)?;
        cx.vm()
            .define_data(rec, "attributeNamespace", JsValue::NULL)?;
        cx.vm().define_data(rec, "oldValue", JsValue::NULL)?;
        let rec_val = JsValue::object(rec);
        cx.keep(rec_val);
        for pers in matching {
            if let Ok(cb_val) = cx.heap().persistent(pers) {
                let _ = cx.call(cb_val, JsValue::UNDEFINED, &[rec_val]);
            }
        }
    }
    if has_ce {
        let g = cx.vm().global();
        if let Ok(key) = cx.heap().key_from_str("__onCustomElementChildListChanged")
            && let Ok(Some(prop)) = cx.heap().get_own(g, key)
            && let Some(fn_val) = prop.slot.data()
            && fn_val.as_object().is_some()
        {
            let _ = cx.call(
                fn_val,
                JsValue::object(g),
                &[target_val, added_arr, removed_arr],
            );
        }
    }
    Ok(())
}

fn notify_char_data_mutation(
    cx: &mut NativeCx<'_>,
    target: NodeId,
    old_val: Option<String>,
) -> Result<(), RunError> {
    let matching: Vec<Persistent> = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            h.mutation_observers
                .iter()
                .filter(|(_, obs_nid)| h.doc.contains_node(*obs_nid, target))
                .map(|(pers, _)| *pers)
                .collect()
        })
        .unwrap_or_default();
    if matching.is_empty() {
        return Ok(());
    }
    let target_val = wrap_node(cx, target)?;
    cx.keep(target_val);
    let rec = cx.vm().new_plain_object()?;
    let mtype_val = cx.vm().string("characterData")?;
    cx.vm().define_data(rec, "type", mtype_val)?;
    cx.vm().define_data(rec, "target", target_val)?;
    let empty_added = cx.vm().new_array_from(&[])?;
    cx.keep(empty_added);
    let empty_removed = cx.vm().new_array_from(&[])?;
    cx.keep(empty_removed);
    cx.vm().define_data(rec, "addedNodes", empty_added)?;
    cx.vm().define_data(rec, "removedNodes", empty_removed)?;
    cx.vm().define_data(rec, "previousSibling", JsValue::NULL)?;
    cx.vm().define_data(rec, "nextSibling", JsValue::NULL)?;
    cx.vm().define_data(rec, "attributeName", JsValue::NULL)?;
    cx.vm()
        .define_data(rec, "attributeNamespace", JsValue::NULL)?;
    let old_js = match old_val.as_deref() {
        Some(s) => cx.vm().string(s)?,
        None => JsValue::NULL,
    };
    cx.vm().define_data(rec, "oldValue", old_js)?;
    let rec_val = JsValue::object(rec);
    cx.keep(rec_val);
    for pers in matching {
        if let Ok(cb_val) = cx.heap().persistent(pers) {
            let _ = cx.call(cb_val, JsValue::UNDEFINED, &[rec_val]);
        }
    }
    Ok(())
}

fn el_get_id(cx: &mut NativeCx<'_>, this: JsValue, _args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let id = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().id().map(str::to_string))
        .unwrap_or_default();
    cx.vm().string(&id)
}

fn el_set_id(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let val = arg_str(cx, args, 0)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr("id").map(str::to_string);
        h.doc.set_attribute(nid, "id", &val);
        prev
    });
    notify_attr_mutation(cx, nid, "id", old_val, Some(val))?;
    Ok(JsValue::UNDEFINED)
}

fn el_get_class_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let cls = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string))
        .unwrap_or_default();
    cx.vm().string(&cls)
}

fn el_set_class_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let val = arg_str(cx, args, 0)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr("class").map(str::to_string);
        h.doc.set_attribute(nid, "class", &val);
        prev
    });
    notify_attr_mutation(cx, nid, "class", old_val, Some(val))?;
    Ok(JsValue::UNDEFINED)
}

fn el_get_text_content(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let text = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.text_content(nid))
        .unwrap_or_default();
    cx.vm().string(&text)
}

fn el_set_text_content(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let text = arg_str(cx, args, 0)?;
    let mut is_text_node = false;
    let mut old_text = None;
    let mut removed = Vec::new();
    let mut added = Vec::new();
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if let NodeData::Text(t) = h.doc.get(nid).data() {
            is_text_node = true;
            old_text = Some(t.clone());
            h.doc.set_text_content(nid, &text);
        } else {
            removed = h
                .doc
                .children(nid)
                .iter()
                .copied()
                .filter(|&cid| h.doc.shadow_host(cid).is_none())
                .collect();
            h.doc.set_text_content(nid, &text);
            added = h
                .doc
                .children(nid)
                .iter()
                .copied()
                .filter(|&cid| h.doc.shadow_host(cid).is_none())
                .collect();
        }
    }
    if is_text_node {
        notify_char_data_mutation(cx, nid, old_text)?;
    } else if !added.is_empty() || !removed.is_empty() {
        notify_child_mutation(cx, nid, &added, &removed, None, None)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_text_content_method(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_get_text_content(cx, this, args)
}

fn el_set_text_content_method(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_set_text_content(cx, this, args)
}

fn el_get_inner_html(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let html = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.inner_html(nid))
        .unwrap_or_default();
    cx.vm().string(&html)
}

fn el_set_inner_html(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let html = arg_str(cx, args, 0)?;
    let mut removed = Vec::new();
    let mut added = Vec::new();
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        removed = h
            .doc
            .children(nid)
            .iter()
            .copied()
            .filter(|&cid| h.doc.shadow_host(cid).is_none())
            .collect();
        h.doc.set_inner_html(nid, &html);
        added = h
            .doc
            .children(nid)
            .iter()
            .copied()
            .filter(|&cid| h.doc.shadow_host(cid).is_none())
            .collect();
    }
    if !added.is_empty() || !removed.is_empty() {
        notify_child_mutation(cx, nid, &added, &removed, None, None)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_set_inner_html_method(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_set_inner_html(cx, this, args)
}

fn el_get_outer_html(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let html = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.outer_html(nid))
        .unwrap_or_default();
    cx.vm().string(&html)
}

fn el_get_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let val = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.input_value(nid))
        .unwrap_or_default();
    cx.vm().string(&val)
}

fn el_set_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let val = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.set_input_value(nid, &val);
    }
    Ok(JsValue::UNDEFINED)
}

fn el_set_value_method(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_set_value(cx, this, args)
}

fn el_get_checked(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let checked = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.is_checked(nid));
    Ok(JsValue::boolean(checked))
}

fn el_set_checked(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let checked = args
        .first()
        .copied()
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_number().is_some_and(|n| n != 0.0));
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.set_checked(nid, checked);
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_tag_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let tag = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().tag().map(str::to_ascii_uppercase))
        .unwrap_or_default();
    cx.vm().string(&tag)
}

fn el_get_parent_element(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let parent = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.parent_element(nid));
    wrap_optional_node(cx, parent)
}

fn el_get_first_element_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let first = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.first_element_child(nid));
    wrap_optional_node(cx, first)
}

fn el_get_last_element_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let last = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.last_element_child(nid));
    wrap_optional_node(cx, last)
}

fn el_get_first_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let first = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.first_child(nid));
    wrap_optional_node(cx, first)
}

fn el_get_last_child(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let last = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.last_child(nid));
    wrap_optional_node(cx, last)
}

fn el_get_next_element_sibling(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let next = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.next_element_sibling(nid));
    wrap_optional_node(cx, next)
}

fn el_get_previous_element_sibling(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let prev = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.previous_element_sibling(nid));
    wrap_optional_node(cx, prev)
}

fn el_get_children(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().new_array_from(&[]);
    };
    let kids = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.element_children(nid))
        .unwrap_or_default();
    wrap_node_list(cx, kids)
}

fn el_get_child_element_count(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::number(0.0));
    };
    let count = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.element_children(nid).len())
        .unwrap_or(0);
    Ok(JsValue::number(count as f64))
}

fn el_get_shadow_root(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let sr = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.shadow_root(nid));
    wrap_optional_node(cx, sr)
}

fn el_get_class_list(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let data = HostData(TAG_CLASS_LIST | (nid.index() as u64));
    if let Some(existing) = cx.heap().wrapper(data) {
        return Ok(JsValue::object(existing));
    }
    let proto = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.class_list_proto);
    let obj = cx.vm().new_host_object(proto, data)?;
    cx.heap().set_wrapper(data, obj);
    Ok(JsValue::object(obj))
}

fn el_get_style(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let data = HostData(TAG_STYLE | (nid.index() as u64));
    if let Some(existing) = cx.heap().wrapper(data) {
        return Ok(JsValue::object(existing));
    }
    let proto = cx.host_as::<CinderDomHost>().and_then(|h| h.style_proto);
    let obj = cx.vm().new_host_object(proto, data)?;
    cx.heap().set_wrapper(data, obj);
    Ok(JsValue::object(obj))
}

fn el_get_dataset(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let data = HostData(TAG_DATASET | (nid.index() as u64));
    if let Some(existing) = cx.heap().wrapper(data) {
        return Ok(JsValue::object(existing));
    }
    let proto = cx.host_as::<CinderDomHost>().and_then(|h| h.dataset_proto);
    let obj = cx.vm().new_host_object(proto, data)?;
    cx.heap().set_wrapper(data, obj);
    Ok(JsValue::object(obj))
}

// ---- classList methods ----------------------------------------------------

fn cl_add(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let old_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    for i in 0..args.len() {
        let token = arg_str(cx, args, i)?;
        if let Some(h) = cx.host_as::<CinderDomHost>() {
            h.doc.add_class(nid, token.trim());
        }
    }
    let new_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    notify_attr_mutation(cx, nid, "class", old_val, new_val)?;
    Ok(JsValue::UNDEFINED)
}

fn cl_remove(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let old_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    for i in 0..args.len() {
        let token = arg_str(cx, args, i)?;
        if let Some(h) = cx.host_as::<CinderDomHost>() {
            h.doc.remove_class(nid, token.trim());
        }
    }
    let new_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    notify_attr_mutation(cx, nid, "class", old_val, new_val)?;
    Ok(JsValue::UNDEFINED)
}

fn cl_toggle(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let token = arg_str(cx, args, 0)?;
    let old_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    let present = if let Some(force) = args.get(1).copied()
        && !force.is_undefined()
    {
        let on = force.as_bool() == Some(true) || force.as_number().is_some_and(|n| n != 0.0);
        if let Some(h) = cx.host_as::<CinderDomHost>() {
            if on {
                h.doc.add_class(nid, token.trim());
            } else {
                h.doc.remove_class(nid, token.trim());
            }
        }
        on
    } else {
        cx.host_as::<CinderDomHost>()
            .is_some_and(|h| h.doc.toggle_class(nid, token.trim()))
    };
    let new_val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("class").map(str::to_string));
    notify_attr_mutation(cx, nid, "class", old_val, new_val)?;
    Ok(JsValue::boolean(present))
}

fn cl_contains(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let token = arg_str(cx, args, 0)?;
    let has = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.get(nid).data().has_class(token.trim()));
    Ok(JsValue::boolean(has))
}

fn cl_to_string(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_get_class_name(cx, this, args)
}

// ---- style methods --------------------------------------------------------

fn style_set_property(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let raw_prop = arg_str(cx, args, 0)?;
    let css_prop = camel_to_kebab(raw_prop.trim());
    let val = arg_str(cx, args, 1)?;
    let (old_val, new_val) = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let prev = h.doc.get(nid).data().attr("style").map(str::to_string);
            if val.trim().is_empty() {
                remove_inline_style_prop(&mut h.doc, nid, &css_prop);
            } else {
                update_inline_style(&mut h.doc, nid, &css_prop, val.trim());
            }
            let next = h.doc.get(nid).data().attr("style").map(str::to_string);
            (prev, next)
        })
        .unwrap_or((None, None));
    notify_attr_mutation(cx, nid, "style", old_val, new_val)?;
    Ok(JsValue::UNDEFINED)
}

fn style_get_property_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let raw_prop = arg_str(cx, args, 0)?;
    let css_prop = camel_to_kebab(raw_prop.trim());
    let val = cx
        .host_as::<CinderDomHost>()
        .map(|h| read_inline_style(&h.doc, nid, &css_prop))
        .unwrap_or_default();
    cx.vm().string(&val)
}

fn style_remove_property(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let raw_prop = arg_str(cx, args, 0)?;
    let css_prop = camel_to_kebab(raw_prop.trim());
    let (prev, old_attr, new_attr) = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let prev_attr = h.doc.get(nid).data().attr("style").map(str::to_string);
            let old = read_inline_style(&h.doc, nid, &css_prop);
            remove_inline_style_prop(&mut h.doc, nid, &css_prop);
            let next_attr = h.doc.get(nid).data().attr("style").map(str::to_string);
            (old, prev_attr, next_attr)
        })
        .unwrap_or_default();
    notify_attr_mutation(cx, nid, "style", old_attr, new_attr)?;
    cx.vm().string(&prev)
}

fn style_get_css_text(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let css = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr("style").map(str::to_string))
        .unwrap_or_default();
    cx.vm().string(&css)
}

fn style_set_css_text(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let css = arg_str(cx, args, 0)?;
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr("style").map(str::to_string);
        h.doc.set_attribute(nid, "style", &css);
        prev
    });
    notify_attr_mutation(cx, nid, "style", old_val, Some(css))?;
    Ok(JsValue::UNDEFINED)
}

// ---- dataset methods ------------------------------------------------------

fn dataset_get(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let key = arg_str(cx, args, 0)?;
    let attr_name = format!("data-{}", camel_to_kebab(key.trim()));
    let val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).data().attr(&attr_name).map(str::to_string));
    match val {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::UNDEFINED),
    }
}

fn dataset_set(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let key = arg_str(cx, args, 0)?;
    let val = arg_str(cx, args, 1)?;
    let attr_name = format!("data-{}", camel_to_kebab(key.trim()));
    let old_val = cx.host_as::<CinderDomHost>().and_then(|h| {
        let prev = h.doc.get(nid).data().attr(&attr_name).map(str::to_string);
        h.doc.set_attribute(nid, &attr_name, &val);
        prev
    });
    notify_attr_mutation(cx, nid, &attr_name, old_val, Some(val))?;
    Ok(JsValue::UNDEFINED)
}

// ---- Storage methods ------------------------------------------------------

fn local_storage_get_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    let val = cx.host_as::<CinderDomHost>().and_then(|h| {
        h.local_storage
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, v)| v.clone())
    });
    match val {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn local_storage_set_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    let val = arg_str(cx, args, 1)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        storage_put_capped(&mut h.local_storage, key, val);
    }
    Ok(JsValue::UNDEFINED)
}

fn local_storage_remove_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.local_storage.retain(|(k, _)| k != &key);
    }
    Ok(JsValue::UNDEFINED)
}

fn local_storage_clear(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.local_storage.clear();
    }
    Ok(JsValue::UNDEFINED)
}

fn local_storage_key(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let idx = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
    let key = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.local_storage.get(idx).map(|(k, _)| k.clone()));
    match key {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn local_storage_length(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let len = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.local_storage.len())
        .unwrap_or(0);
    Ok(JsValue::number(len as f64))
}

fn session_storage_get_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    let val = cx.host_as::<CinderDomHost>().and_then(|h| {
        h.session_storage
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, v)| v.clone())
    });
    match val {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn session_storage_set_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    let val = arg_str(cx, args, 1)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        storage_put_capped(&mut h.session_storage, key, val);
    }
    Ok(JsValue::UNDEFINED)
}

fn session_storage_remove_item(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let key = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.session_storage.retain(|(k, _)| k != &key);
    }
    Ok(JsValue::UNDEFINED)
}

fn session_storage_clear(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.session_storage.clear();
    }
    Ok(JsValue::UNDEFINED)
}

fn session_storage_key(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let idx = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
    let key = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.session_storage.get(idx).map(|(k, _)| k.clone()));
    match key {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn session_storage_length(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let len = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.session_storage.len())
        .unwrap_or(0);
    Ok(JsValue::number(len as f64))
}

// ---- Location, History, CustomElements & Window helpers -------------------

fn loc_get_href(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let href = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    cx.vm().string(&href)
}

fn loc_set_href(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let next = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        match crate::classify_navigation_intent(&h.location_href, &next) {
            crate::NavigationIntent::Ignore => {}
            crate::NavigationIntent::Fragment(href) => {
                h.location_href = href;
                h.pending_navigation = None;
            }
            crate::NavigationIntent::Document(href) => {
                // The displayed document URL stays the active page. Fetch,
                // form actions, and the storage origin keep reading it.
                h.pending_navigation = Some(href);
            }
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn loc_assign(cx: &mut NativeCx<'_>, this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    loc_set_href(cx, this, args)
}

fn loc_get_origin(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let href = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    let origin = if let Some((scheme, host, port)) = crate::extract_origin_tuple(&href) {
        if (scheme == "https" && port == 443) || (scheme == "http" && port == 80) {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{port}")
        }
    } else {
        href
    };
    cx.vm().string(&origin)
}

fn loc_get_pathname(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let href = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    let after_scheme = href
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(href.as_str());
    let before_q = after_scheme.split(['?', '#']).next().unwrap_or("/");
    let path = before_q.find('/').map(|i| &before_q[i..]).unwrap_or("/");
    cx.vm().string(path)
}

fn loc_get_search(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let href = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    let before_hash = href.split('#').next().unwrap_or("");
    let search = before_hash
        .split_once('?')
        .map(|(_, q)| format!("?{q}"))
        .unwrap_or_default();
    cx.vm().string(&search)
}

fn loc_get_hash(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let href = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.location_href.clone())
        .unwrap_or_default();
    let hash = href
        .split_once('#')
        .map(|(_, h)| format!("#{h}"))
        .unwrap_or_default();
    cx.vm().string(&hash)
}

fn hist_push_state(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    if args.len() >= 3 && !args[2].is_nullish() {
        let next = arg_str(cx, args, 2)?;
        if !next.trim().is_empty() {
            let current = cx
                .host_as::<CinderDomHost>()
                .map(|host| host.location_href.clone())
                .unwrap_or_default();
            match crate::same_origin_history_url(&current, &next) {
                Ok(url) => {
                    if let Some(host) = cx.host_as::<CinderDomHost>() {
                        host.location_href = url;
                    }
                }
                Err(()) => {
                    return Err(cx.type_error("SecurityError: history URL is not same-origin"));
                }
            }
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn custom_elements_define(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.has_custom_elements = true;
        h.doc.define_custom_element(&name);
    }
    Ok(JsValue::UNDEFINED)
}

fn custom_elements_get(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    let defined = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.is_custom_element_defined(&name));
    if defined {
        cx.vm().string(&name)
    } else {
        Ok(JsValue::UNDEFINED)
    }
}

fn window_match_media(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let q = arg_str(cx, args, 0)?;
    let mql = cx.vm().new_plain_object()?;
    let qs = cx.vm().string(&q)?;
    cx.vm().define_data(mql, "matches", JsValue::FALSE)?;
    cx.vm().define_data(mql, "media", qs)?;
    Ok(JsValue::object(mql))
}

fn format_css_color(c: anvil_style::Color) -> String {
    if c.a == 255 {
        format!("rgb({}, {}, {})", c.r, c.g, c.b)
    } else if c.a == 0 {
        "rgba(0, 0, 0, 0)".to_string()
    } else {
        let alpha = (c.a as f32 / 255.0 * 100.0).round() / 100.0;
        format!("rgba({}, {}, {}, {alpha})", c.r, c.g, c.b)
    }
}

fn format_css_px(px: f32) -> String {
    if px.fract().abs() < 1e-4 {
        format!("{}px", px.round() as i64)
    } else {
        format!("{px:.2}px")
    }
}

fn window_get_computed_style(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let obj = cx.vm().new_plain_object()?;
    let pseudo_raw = if args.len() >= 2 && !args[1].is_nullish() {
        arg_str(cx, args, 1).unwrap_or_default()
    } else {
        String::new()
    };
    let pseudo_norm = pseudo_raw.trim().to_ascii_lowercase();
    if let Some(nid) = args.first().copied().and_then(|v| unwrap_node(cx, v)) {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut inline = String::new();
        if let Some(h) = cx.host_as::<CinderDomHost>() {
            inline = h
                .doc
                .get(nid)
                .data()
                .attr("style")
                .map(str::to_string)
                .unwrap_or_default();
            let styles = h.compute_styles();
            let layout = anvil_layout::layout_document(&h.doc, &styles, h.viewport_width);
            let box_rect = layout.find_node_box(nid).map(|b| b.rect);
            let (cs, pseudo_content) = match pseudo_norm.as_str() {
                "::before" | ":before" => {
                    if let Some((txt, ps)) = styles.before_style(nid) {
                        (ps.clone(), Some(format!("\"{txt}\"")))
                    } else {
                        (styles.get(nid).clone(), Some("none".to_string()))
                    }
                }
                "::after" | ":after" => {
                    if let Some((txt, ps)) = styles.after_style(nid) {
                        (ps.clone(), Some(format!("\"{txt}\"")))
                    } else {
                        (styles.get(nid).clone(), Some("none".to_string()))
                    }
                }
                _ => (styles.get(nid).clone(), None),
            };
            let display_str = match cs.display {
                anvil_style::Display::Block => "block",
                anvil_style::Display::Inline => "inline",
                anvil_style::Display::Flex => "flex",
                anvil_style::Display::Grid => "grid",
                anvil_style::Display::None => "none",
            };
            let position_str = match cs.position {
                anvil_style::Position::Static => "static",
                anvil_style::Position::Relative => "relative",
                anvil_style::Position::Absolute => "absolute",
                anvil_style::Position::Fixed => "fixed",
                anvil_style::Position::Sticky => "sticky",
            };
            let visibility_str = match cs.visibility {
                anvil_style::Visibility::Visible => "visible",
                anvil_style::Visibility::Hidden => "hidden",
                anvil_style::Visibility::Collapse => "collapse",
            };
            let overflow_str = if cs.scrollable_overflow {
                "auto"
            } else {
                match cs.overflow {
                    anvil_style::Overflow::Visible => "visible",
                    anvil_style::Overflow::Hidden => "hidden",
                }
            };
            let font_family_str = match cs.font_family {
                anvil_style::FontFamily::SansSerif => "sans-serif",
                anvil_style::FontFamily::Serif => "serif",
                anvil_style::FontFamily::Monospace => "monospace",
                anvil_style::FontFamily::Custom(_) => "custom",
            };
            let text_transform_str = match cs.text_transform {
                anvil_style::TextTransform::None => "none",
                anvil_style::TextTransform::Uppercase => "uppercase",
                anvil_style::TextTransform::Lowercase => "lowercase",
                anvil_style::TextTransform::Capitalize => "capitalize",
            };
            let object_fit_str = match cs.object_fit {
                anvil_style::ObjectFit::Fill => "fill",
                anvil_style::ObjectFit::Contain => "contain",
                anvil_style::ObjectFit::Cover => "cover",
                anvil_style::ObjectFit::None => "none",
                anvil_style::ObjectFit::ScaleDown => "scale-down",
            };
            let width_px = box_rect.map(|r| r.width).unwrap_or(match cs.width {
                anvil_style::Size::Px(v) => v,
                _ => 0.0,
            });
            let height_px = box_rect.map(|r| r.height).unwrap_or(match cs.height {
                anvil_style::Size::Px(v) => v,
                _ => 0.0,
            });
            pairs.push(("display".to_string(), display_str.to_string()));
            pairs.push(("position".to_string(), position_str.to_string()));
            pairs.push(("visibility".to_string(), visibility_str.to_string()));
            pairs.push(("overflow".to_string(), overflow_str.to_string()));
            pairs.push(("color".to_string(), format_css_color(cs.color)));
            pairs.push((
                "background-color".to_string(),
                cs.background
                    .map(format_css_color)
                    .unwrap_or_else(|| "rgba(0, 0, 0, 0)".to_string()),
            ));
            pairs.push(("width".to_string(), format_css_px(width_px)));
            pairs.push(("height".to_string(), format_css_px(height_px)));
            pairs.push(("font-size".to_string(), format_css_px(cs.font_size)));
            pairs.push(("font-weight".to_string(), cs.font_weight.to_string()));
            pairs.push((
                "font-style".to_string(),
                if cs.italic { "italic" } else { "normal" }.to_string(),
            ));
            pairs.push(("font-family".to_string(), font_family_str.to_string()));
            pairs.push(("text-transform".to_string(), text_transform_str.to_string()));
            pairs.push((
                "letter-spacing".to_string(),
                format_css_px(cs.letter_spacing),
            ));
            pairs.push(("word-spacing".to_string(), format_css_px(cs.word_spacing)));
            pairs.push(("opacity".to_string(), cs.opacity.to_string()));
            pairs.push(("z-index".to_string(), cs.z_index.to_string()));
            pairs.push(("object-fit".to_string(), object_fit_str.to_string()));
            pairs.push(("margin-top".to_string(), format_css_px(cs.margin.top)));
            pairs.push(("margin-right".to_string(), format_css_px(cs.margin.right)));
            pairs.push(("margin-bottom".to_string(), format_css_px(cs.margin.bottom)));
            pairs.push(("margin-left".to_string(), format_css_px(cs.margin.left)));
            pairs.push(("padding-top".to_string(), format_css_px(cs.padding.top)));
            pairs.push(("padding-right".to_string(), format_css_px(cs.padding.right)));
            pairs.push((
                "padding-bottom".to_string(),
                format_css_px(cs.padding.bottom),
            ));
            pairs.push(("padding-left".to_string(), format_css_px(cs.padding.left)));
            pairs.push(("border-top-width".to_string(), format_css_px(cs.border.top)));
            pairs.push((
                "border-right-width".to_string(),
                format_css_px(cs.border.right),
            ));
            pairs.push((
                "border-bottom-width".to_string(),
                format_css_px(cs.border.bottom),
            ));
            pairs.push((
                "border-left-width".to_string(),
                format_css_px(cs.border.left),
            ));
            if let Some(cnt) = pseudo_content {
                pairs.push(("content".to_string(), cnt));
            }
        }
        for (k, v) in pairs {
            let vs = cx.vm().string(&v)?;
            cx.vm().define_data(obj, &k, vs)?;
            let camel = kebab_to_camel(&k);
            cx.vm().define_data(obj, &camel, vs)?;
        }
        for part in inline.split(';') {
            if let Some((k, v)) = part.split_once(':') {
                let k_clean = k.trim().to_ascii_lowercase();
                let v_clean = v.trim().to_string();
                if !k_clean.is_empty() {
                    let vs = cx.vm().string(&v_clean)?;
                    cx.vm().define_data(obj, &k_clean, vs)?;
                    let camel = kebab_to_camel(&k_clean);
                    cx.vm().define_data(obj, &camel, vs)?;
                }
            }
        }
    }
    cx.vm()
        .define_method(obj, "getPropertyValue", computed_style_get_prop)?;
    Ok(JsValue::object(obj))
}

fn computed_style_get_prop(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(obj) = this.as_object() else {
        return cx.vm().string("");
    };
    let name = arg_str(cx, args, 0)?;
    let key = cx
        .heap()
        .key_from_str(name.trim())
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    if let Ok(Some(prop)) = cx.heap().get_own(obj, key)
        && let Some(val) = prop.slot.data()
    {
        return Ok(val);
    }
    cx.vm().string("")
}

fn el_get_attribute_names(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().new_array_from(&[]);
    };
    let names: Vec<String> = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| {
            if let NodeData::Element { attrs, .. } = h.doc.get(nid).data() {
                Some(attrs.iter().map(|(k, _)| k.clone()).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let mut elems = Vec::with_capacity(names.len());
    for n in names {
        let s = cx.vm().string(&n)?;
        cx.keep(s);
        elems.push(s);
    }
    cx.vm().new_array_from(&elems)
}

fn el_has_attributes(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let has = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| {
            if let NodeData::Element { attrs, .. } = h.doc.get(nid).data() {
                Some(!attrs.is_empty())
            } else {
                None
            }
        })
        .unwrap_or(false);
    Ok(JsValue::boolean(has))
}

fn el_get_node_name(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return cx.vm().string("");
    };
    let name = cx
        .host_as::<CinderDomHost>()
        .map(|h| match h.doc.get(nid).data() {
            NodeData::Document => "#document".to_string(),
            NodeData::Element { tag, .. } => tag.to_ascii_uppercase(),
            NodeData::Text(_) => "#text".to_string(),
        })
        .unwrap_or_default();
    cx.vm().string(&name)
}

fn el_get_node_type(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::number(1.0));
    };
    let nt = cx
        .host_as::<CinderDomHost>()
        .map(|h| match h.doc.get(nid).data() {
            NodeData::Element { .. } => 1.0,
            NodeData::Text(_) => 3.0,
            NodeData::Document => 9.0,
        })
        .unwrap_or(1.0);
    Ok(JsValue::number(nt))
}

fn el_get_node_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let val = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| match h.doc.get(nid).data() {
            NodeData::Text(t) => Some(t.clone()),
            _ => None,
        });
    match val {
        Some(s) => cx.vm().string(&s),
        None => Ok(JsValue::NULL),
    }
}

fn el_set_node_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let text = arg_str(cx, args, 0)?;
    let mut old_val = None;
    let mut is_text = false;
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        if let NodeData::Text(t) = host.doc.get(nid).data() {
            is_text = true;
            old_val = Some(t.clone());
            host.doc.set_text_content(nid, &text);
        }
    }
    if is_text {
        notify_char_data_mutation(cx, nid, old_val)?;
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_parent_node(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let parent = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.get(nid).parent());
    match parent {
        Some(pid) => wrap_node(cx, pid),
        None => Ok(JsValue::NULL),
    }
}

fn el_get_owner_document(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let g = cx.vm().global();
    if let Ok(key) = cx.heap().key_from_str("document")
        && let Ok(Some(prop)) = cx.heap().get_own(g, key)
        && let Some(val) = prop.slot.data()
    {
        return Ok(val);
    }
    Ok(JsValue::NULL)
}

fn el_get_is_connected(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::FALSE);
    };
    let connected = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let root = h.doc.root();
            let mut cur = Some(nid);
            while let Some(id) = cur {
                if id == root {
                    return true;
                }
                cur = h.doc.get(id).parent();
            }
            false
        })
        .unwrap_or(false);
    Ok(JsValue::boolean(connected))
}

fn el_get_child_nodes(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return wrap_node_list(cx, Vec::new());
    };
    let kids: Vec<NodeId> = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            h.doc
                .children(nid)
                .iter()
                .copied()
                .filter(|&cid| h.doc.shadow_host(cid).is_none())
                .collect()
        })
        .unwrap_or_default();
    wrap_node_list(cx, kids)
}

fn el_get_next_sibling(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let next = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.next_sibling(nid));
    wrap_optional_node(cx, next)
}

fn el_get_previous_sibling(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::NULL);
    };
    let prev = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.previous_sibling(nid));
    wrap_optional_node(cx, prev)
}

fn doc_get_node_type(
    _cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    Ok(JsValue::number(9.0))
}

fn doc_get_node_name(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    cx.vm().string("#document")
}

fn doc_get_child_nodes(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let kids: Vec<NodeId> = cx
        .host_as::<CinderDomHost>()
        .map(|h| h.doc.children(h.doc.root()).to_vec())
        .unwrap_or_default();
    wrap_node_list(cx, kids)
}

fn doc_get_is_connected(
    _cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    Ok(JsValue::TRUE)
}

fn window_native_fetch(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let url = arg_str(cx, args, 0)?;
    let method = if args.len() > 1 {
        let m = arg_str(cx, args, 1)?;
        if m.trim().is_empty() {
            "GET".to_string()
        } else {
            m.trim().to_ascii_uppercase()
        }
    } else {
        "GET".to_string()
    };
    let req_body = if args.len() > 2 {
        arg_str(cx, args, 2)?
    } else {
        String::new()
    };
    let content_type = if args.len() > 3 {
        arg_str(cx, args, 3)?
    } else {
        String::new()
    };
    let headers_json = if args.len() > 4 {
        arg_str(cx, args, 4)?
    } else {
        String::new()
    };
    let creds_include = args.get(5).and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = if args.len() > 6 {
        arg_str(cx, args, 6)?
    } else {
        "same-origin".to_string()
    };
    let mut req_headers: Vec<(String, String)> = Vec::new();
    if !headers_json.is_empty()
        && let Ok(crate::anvil_json::JsonValue::Array(items)) =
            crate::anvil_json::parse(&headers_json)
    {
        for item in items {
            if let crate::anvil_json::JsonValue::Array(pair) = item
                && pair.len() >= 2
                && let (Some(k), Some(v)) = (pair[0].as_str(), pair[1].as_str())
            {
                req_headers.push((k.to_string(), v.to_string()));
            }
        }
    }
    if !content_type.is_empty()
        && !req_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        req_headers.push(("content-type".to_string(), content_type.clone()));
    }
    let allow_cors = mode.eq_ignore_ascii_case("cors")
        || !matches!(method.as_str(), "GET" | "HEAD")
        || !req_headers.is_empty()
        || creds_include;
    let (status, body, resp_headers) = if let Some(host) = cx.host_as::<CinderDomHost>() {
        if host.page_suspended {
            (0u16, "page is suspended".to_string(), Vec::new())
        } else if !host.document_fully_active {
            (0u16, "document is not fully active".to_string(), Vec::new())
        } else {
            host.last_fetch_request = Some((
                url.clone(),
                method.clone(),
                content_type.clone(),
                req_body.clone(),
            ));
            let page = host.location_href.clone();
            if let Some(req_handler) = host.fetch_handler.1.clone() {
                let allowed = if allow_cors {
                    super::is_allowed_cors_script_fetch(Some(page.as_str()), &url)
                } else {
                    super::is_allowed_script_fetch(Some(page.as_str()), &url)
                };
                if !allowed {
                    (
                        0u16,
                        "same-origin policy blocked fetch".to_string(),
                        Vec::new(),
                    )
                } else if let Some((_, st, mock_body)) =
                    host.fetch_mocks.iter().find(|(k, _, _)| {
                        k == &url || url.ends_with(k.as_str()) || k.ends_with(url.as_str())
                    })
                {
                    (*st, mock_body.clone(), Vec::new())
                } else {
                    let absolute = super::resolve_script_fetch_url(&page, &url);
                    let req = crate::ScriptFetchRequest {
                        url: absolute,
                        page_url: page,
                        method: method.clone(),
                        headers: req_headers,
                        body: if req_body.is_empty() && matches!(method.as_str(), "GET" | "HEAD") {
                            None
                        } else if !req_body.is_empty() {
                            Some(req_body)
                        } else {
                            None
                        },
                        credentials_include: creds_include,
                    };
                    let mut reply = req_handler(&req);
                    super::limit_script_fetch_body(&mut reply.body);
                    if reply.status == 0 {
                        let reason = if reply.body.is_empty() {
                            "network error".to_string()
                        } else {
                            reply.body
                        };
                        (0u16, reason, Vec::new())
                    } else {
                        (reply.status, reply.body, reply.headers)
                    }
                }
            } else if !super::is_allowed_script_fetch(Some(page.as_str()), &url) {
                (
                    0u16,
                    "same-origin policy blocked fetch".to_string(),
                    Vec::new(),
                )
            } else if let Some((_, st, mock_body)) = host.fetch_mocks.iter().find(|(k, _, _)| {
                k == &url || url.ends_with(k.as_str()) || k.ends_with(url.as_str())
            }) {
                (*st, mock_body.clone(), Vec::new())
            } else if let Some(handler) = host.fetch_handler.0.clone() {
                let absolute = super::resolve_script_fetch_url(&page, &url);
                let mut reply = handler(&absolute, &page);
                super::limit_script_fetch_body(&mut reply.body);
                if reply.status == 0 {
                    let reason = if reply.body.is_empty() {
                        "network error".to_string()
                    } else {
                        reply.body
                    };
                    (0u16, reason, Vec::new())
                } else {
                    (reply.status, reply.body, Vec::new())
                }
            } else {
                (200u16, "{}".to_string(), Vec::new())
            }
        }
    } else {
        (
            0u16,
            "same-origin policy blocked fetch".to_string(),
            Vec::new(),
        )
    };
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "status", JsValue::number(status as f64))?;
    let body_js = cx.vm().string(&body)?;
    cx.vm().define_data(obj, "body", body_js)?;
    let url_js = cx.vm().string(&url)?;
    cx.vm().define_data(obj, "url", url_js)?;
    let headers_obj = cx.vm().new_plain_object()?;
    for (hk, hv) in &resp_headers {
        let hv_js = cx.vm().string(hv)?;
        cx.vm()
            .define_data(headers_obj, &hk.to_ascii_lowercase(), hv_js)?;
    }
    cx.vm()
        .define_data(obj, "headers", JsValue::object(headers_obj))?;
    Ok(JsValue::object(obj))
}

fn window_dynamic_import(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let inactive_reason = cx.host_as::<CinderDomHost>().and_then(|host| {
        if host.page_suspended {
            Some("page is suspended")
        } else if !host.document_fully_active {
            Some("document is not fully active")
        } else {
            None
        }
    });
    if let Some(reason) = inactive_reason {
        let err = cx
            .vm()
            .new_error_value(cinder::vm::ErrorKind::Type, reason)?;
        cx.keep(err);
        return cx.new_rejected_promise(err);
    }
    let mut type_attr: Option<String> = None;
    if let Some(opts) = args.get(2).copied().filter(|opts| !opts.is_undefined()) {
        let Some(opts_obj) = opts.as_object() else {
            let err = cx.vm().new_error_value(
                cinder::vm::ErrorKind::Type,
                "import() options must be an object",
            )?;
            cx.keep(err);
            return cx.new_rejected_promise(err);
        };
        if let Ok(assert_key) = cx.heap().key_from_str("assert")
            && let Ok(Some(prop)) = cx.heap().get_own(opts_obj, assert_key)
            && prop.slot.data().is_some_and(|v| !v.is_undefined())
        {
            let err = cx.vm().new_error_value(
                cinder::vm::ErrorKind::Type,
                "import assertions are not supported",
            )?;
            cx.keep(err);
            return cx.new_rejected_promise(err);
        }
        if let Ok(with_key) = cx.heap().key_from_str("with")
            && let Ok(Some(prop)) = cx.heap().get_own(opts_obj, with_key)
            && let Some(with_val) = prop.slot.data()
            && !with_val.is_undefined()
        {
            let Some(with_obj) = with_val.as_object() else {
                let err = cx.vm().new_error_value(
                    cinder::vm::ErrorKind::Type,
                    "import() with option must be an object",
                )?;
                cx.keep(err);
                return cx.new_rejected_promise(err);
            };
            for key in cx.heap().own_keys(with_obj).unwrap_or_default() {
                let Some(entry) = cx.heap().get_own(with_obj, key).ok().flatten() else {
                    continue;
                };
                if !entry.attrs.enumerable() {
                    continue;
                }
                let key_name = cx.vm().key_text(key);
                let val = entry.slot.data().unwrap_or(JsValue::UNDEFINED);
                if val.as_string().is_none() {
                    let err = cx.vm().new_error_value(
                        cinder::vm::ErrorKind::Type,
                        "import attribute value must be a string",
                    )?;
                    cx.keep(err);
                    return cx.new_rejected_promise(err);
                }
                let val_str = cx.to_display_string(val).unwrap_or_default();
                if key_name != "type" || val_str != "json" {
                    let err = cx.vm().new_error_value(
                        cinder::vm::ErrorKind::Type,
                        "unsupported import attribute",
                    )?;
                    cx.keep(err);
                    return cx.new_rejected_promise(err);
                }
                type_attr = Some(val_str);
            }
        }
    }
    let raw_spec = args.first().copied().unwrap_or(JsValue::UNDEFINED);
    let specifier = match cx.coerce_js_string(raw_spec) {
        Ok(s) => s,
        Err(RunError::Thrown { value, .. }) => {
            cx.keep(value);
            return cx.new_rejected_promise(value);
        }
        Err(other) => return Err(other),
    };
    let referrer_module_id = args
        .get(1)
        .and_then(|v| v.as_number())
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map(|n| n as u64);
    let (promise, resolve_fn, reject_fn) = cx.new_promise_with_resolvers()?;
    cx.keep(promise);
    cx.keep(resolve_fn);
    cx.keep(reject_fn);
    let queue_full = if let Some(host) = cx.host_as::<CinderDomHost>() {
        if host.pending_dynamic_imports.len() >= MAX_HOST_JOBS {
            true
        } else {
            host.pending_dynamic_imports
                .push_back(PendingDynamicImport {
                    specifier,
                    referrer_module_id,
                    referrer_url: host.location_href.clone(),
                    type_attr,
                    resolve: resolve_fn,
                    reject: reject_fn,
                    task: host.owner_task,
                    spawn: host.timer_spawn,
                });
            false
        }
    } else {
        true
    };
    if queue_full {
        let err = cx
            .vm()
            .new_error_value(cinder::vm::ErrorKind::Type, "dynamic import queue full")?;
        cx.keep(err);
        return cx.new_rejected_promise(err);
    }
    Ok(promise)
}

fn element_is_muted(doc: &Document, id: NodeId) -> bool {
    let data = doc.get(id).data();
    if data.attr("muted").is_some() {
        return true;
    }
    if let Some(vol) = data.attr("volume")
        && let Ok(value) = vol.trim().parse::<f64>()
        && value == 0.0
    {
        return true;
    }
    false
}

/// `true` when `play()` may start. An unmuted element consumes activation.
fn window_media_play(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(id) = args
        .first()
        .copied()
        .and_then(|value| unwrap_node(cx, value))
    else {
        return Ok(JsValue::FALSE);
    };
    let Some(host) = cx.host_as::<CinderDomHost>() else {
        return Ok(JsValue::FALSE);
    };
    let muted = element_is_muted(&host.doc, id);
    if !muted && !host.user_activation {
        return Ok(JsValue::FALSE);
    }
    if !muted {
        host.user_activation = false;
    }
    Ok(JsValue::TRUE)
}

fn window_sha256_bytes(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let mut input = Vec::new();
    if let Some(first) = args.first().copied() {
        if let Some(obj) = first.as_object() {
            let len_key = cx
                .heap()
                .key_from_str("length")
                .map_err(|e| RunError::internal(format!("{e:?}")))?;
            let len = cx
                .heap()
                .get_own(obj, len_key)
                .ok()
                .flatten()
                .and_then(|p| p.slot.data())
                .and_then(|v| v.as_number())
                .unwrap_or(0.0) as usize;
            for i in 0..len.min(1_048_576) {
                let k = cx
                    .heap()
                    .key_from_str(&i.to_string())
                    .map_err(|e| RunError::internal(format!("{e:?}")))?;
                let b = cx
                    .heap()
                    .get_own(obj, k)
                    .ok()
                    .flatten()
                    .and_then(|p| p.slot.data())
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0) as i64;
                input.push((b & 0xFF) as u8);
            }
        } else if let Ok(s) = cx.to_display_string(first) {
            input.extend_from_slice(s.as_bytes());
        }
    }
    let digest = sha256_digest(&input);
    let elems: Vec<JsValue> = digest.iter().map(|&b| JsValue::number(b as f64)).collect();
    cx.vm().new_array_from(&elems)
}

fn canvas_fill_linear_gradient(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let x = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let y = args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as i32;
    let w = args
        .get(2)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0) as u32;
    let h = args
        .get(3)
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0) as u32;
    let c0_str = arg_str(cx, args, 4).unwrap_or_else(|_| "#000000".to_string());
    let c1_str = arg_str(cx, args, 5).unwrap_or_else(|_| "#ffffff".to_string());
    let horizontal = args.get(6).and_then(|v| v.as_bool()).unwrap_or(true);
    let c0 = crate::parse_canvas_color(&c0_str);
    let c1 = crate::parse_canvas_color(&c1_str);
    let x0 = x as f32;
    let y0 = y as f32;
    let x1 = if horizontal {
        (x + w as i32) as f32
    } else {
        x0
    };
    let y1 = if horizontal {
        y0
    } else {
        (y + h as i32) as f32
    };
    if let Some(host) = cx.host_as::<CinderDomHost>() {
        host.doc.canvas_fill_linear_gradient(
            nid,
            x,
            y,
            w,
            h,
            x0,
            y0,
            x1,
            y1,
            &[(0.0, c0), (1.0, c1)],
        );
    }
    Ok(JsValue::UNDEFINED)
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[allow(dead_code)]
fn _keep_dom_imports_used(d: &Document) -> bool {
    let _ = parse_html;
    matches!(d.get(d.root()).data(), NodeData::Document)
}

// ---- Pillar 2.3 & Pillar 4: Live Geometry, Scroll, <dialog>, & Form Validation ----

fn el_get_offset_width(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let w = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| b.rect.width.round() as f64)
        .unwrap_or(0.0);
    Ok(JsValue::number(w))
}

fn el_get_offset_height(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let h = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| b.rect.height.round() as f64)
        .unwrap_or(0.0);
    Ok(JsValue::number(h))
}

fn el_get_client_width(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let w = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| {
            (b.rect.width - b.border.left - b.border.right)
                .max(0.0)
                .round() as f64
        })
        .unwrap_or(0.0);
    Ok(JsValue::number(w))
}

fn el_get_client_height(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let h = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| {
            (b.rect.height - b.border.top - b.border.bottom)
                .max(0.0)
                .round() as f64
        })
        .unwrap_or(0.0);
    Ok(JsValue::number(h))
}

fn el_get_scroll_width(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let w = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| {
            let cw = (b.rect.width - b.border.left - b.border.right).max(0.0);
            b.scroll_size.0.max(cw).round() as f64
        })
        .unwrap_or(0.0);
    Ok(JsValue::number(w))
}

fn el_get_scroll_height(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let h = unwrap_node(cx, this)
        .and_then(|nid| cx.host_as::<CinderDomHost>()?.compute_node_box(nid))
        .map(|b| {
            let ch = (b.rect.height - b.border.top - b.border.bottom).max(0.0);
            b.scroll_size.1.max(ch).round() as f64
        })
        .unwrap_or(0.0);
    Ok(JsValue::number(h))
}

fn el_get_scroll_top(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let sy = unwrap_node(cx, this)
        .and_then(|nid| Some(cx.host_as::<CinderDomHost>()?.doc.node_scroll_offset(nid).1 as f64))
        .unwrap_or(0.0);
    Ok(JsValue::number(sy))
}

fn el_set_scroll_top(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let next_y = args
        .first()
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0) as f32;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let (sx, _) = h.doc.node_scroll_offset(nid);
        h.doc.set_node_scroll_offset(nid, sx, next_y);
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_scroll_left(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let sx = unwrap_node(cx, this)
        .and_then(|nid| Some(cx.host_as::<CinderDomHost>()?.doc.node_scroll_offset(nid).0 as f64))
        .unwrap_or(0.0);
    Ok(JsValue::number(sx))
}

fn el_set_scroll_left(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let next_x = args
        .first()
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
        .max(0.0) as f32;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let (_, sy) = h.doc.node_scroll_offset(nid);
        h.doc.set_node_scroll_offset(nid, next_x, sy);
    }
    Ok(JsValue::UNDEFINED)
}

fn extract_scroll_coords(cx: &mut NativeCx<'_>, args: &[JsValue]) -> (Option<f32>, Option<f32>) {
    if let Some(first) = args.first().copied() {
        if let Some(obj) = first.as_object() {
            let mut left = None;
            let mut top = None;
            if let Ok(k_left) = cx.heap().key_from_str("left")
                && let Ok(Some(p)) = cx.heap().get_own(obj, k_left)
                && let Some(v) = p.slot.data().and_then(|v| v.as_number())
            {
                left = Some(v as f32);
            }
            if let Ok(k_top) = cx.heap().key_from_str("top")
                && let Ok(Some(p)) = cx.heap().get_own(obj, k_top)
                && let Some(v) = p.slot.data().and_then(|v| v.as_number())
            {
                top = Some(v as f32);
            }
            return (left, top);
        }
        let x = first.as_number().map(|v| v as f32);
        let y = args.get(1).and_then(|v| v.as_number()).map(|v| v as f32);
        return (x, y);
    }
    (None, None)
}

fn el_scroll_to(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let (nx, ny) = extract_scroll_coords(cx, args);
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let (cx_off, cy_off) = h.doc.node_scroll_offset(nid);
        h.doc.set_node_scroll_offset(
            nid,
            nx.unwrap_or(cx_off).max(0.0),
            ny.unwrap_or(cy_off).max(0.0),
        );
    }
    Ok(JsValue::UNDEFINED)
}

fn el_scroll_by(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let (dx, dy) = extract_scroll_coords(cx, args);
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        let (cx_off, cy_off) = h.doc.node_scroll_offset(nid);
        h.doc.set_node_scroll_offset(
            nid,
            (cx_off + dx.unwrap_or(0.0)).max(0.0),
            (cy_off + dy.unwrap_or(0.0)).max(0.0),
        );
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_dialog_open(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let open = unwrap_node(cx, this)
        .and_then(|nid| {
            let h = cx.host_as::<CinderDomHost>()?;
            Some(
                h.doc
                    .get(nid)
                    .data()
                    .attr("open")
                    .is_some_and(|v| !v.eq_ignore_ascii_case("false")),
            )
        })
        .unwrap_or(false);
    Ok(JsValue::boolean(open))
}

fn el_set_dialog_open(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let open = args.first().copied().is_some_and(|v| {
        v.as_bool() == Some(true)
            || v.as_number().is_some_and(|n| n != 0.0 && !n.is_nan())
            || v.is_object()
            || (v.is_string() && cx.to_display_string(v).is_ok_and(|s| !s.is_empty()))
    });
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if open {
            h.doc.set_attribute(nid, "open", "");
        } else {
            h.doc.remove_attribute(nid, "open");
            h.doc.remove_attribute(nid, "data-modal");
        }
    }
    Ok(JsValue::UNDEFINED)
}

fn el_get_dialog_return_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let rv = unwrap_node(cx, this)
        .and_then(|nid| {
            cx.host_as::<CinderDomHost>()?
                .doc
                .get(nid)
                .data()
                .attr("data-return-value")
                .map(str::to_string)
        })
        .unwrap_or_default();
    cx.vm().string(&rv)
}

fn el_set_dialog_return_value(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let rv = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.set_attribute(nid, "data-return-value", &rv);
    }
    Ok(JsValue::UNDEFINED)
}

fn el_dialog_show(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        h.doc.set_attribute(nid, "open", "");
    }
    Ok(JsValue::UNDEFINED)
}

fn el_dialog_show_modal(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(nid) = unwrap_node(cx, this)
        && let Some(h) = cx.host_as::<CinderDomHost>()
    {
        h.doc.set_attribute(nid, "open", "");
        h.doc.set_attribute(nid, "data-modal", "true");
    }
    Ok(JsValue::UNDEFINED)
}

fn fire_dialog_close_event(cx: &mut NativeCx<'_>, nid: NodeId) -> Result<(), RunError> {
    let g = cx.vm().global();
    let has_js_dispatch = cx
        .heap()
        .key_from_str("__dispatchNativeDomEvent")
        .ok()
        .and_then(|fn_key| cx.heap().get_own(g, fn_key).ok().flatten())
        .and_then(|prop| prop.slot.data())
        .is_some_and(|fn_val| fn_val.as_object().is_some());
    if has_js_dispatch {
        let _ = dispatch_dom_event(cx, nid, "close")?;
        return Ok(());
    }
    let target_val = wrap_node(cx, nid)?;
    let ev = cx.vm().new_plain_object()?;
    let close_str = cx.vm().string("close")?;
    cx.vm().define_data(ev, "type", close_str)?;
    cx.vm().define_data(ev, "bubbles", JsValue::FALSE)?;
    cx.vm().define_data(ev, "cancelable", JsValue::FALSE)?;
    let _ = el_dispatch_event(cx, target_val, &[JsValue::object(ev)])?;
    Ok(())
}

fn el_dialog_close(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let ret_val = if let Some(first) = args.first().copied()
        && !first.is_undefined()
    {
        Some(cx.to_display_string(first)?)
    } else {
        None
    };
    let closed = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.doc.close_dialog_element(nid, ret_val.as_deref()));
    if closed {
        fire_dialog_close_event(cx, nid)?;
    }
    Ok(JsValue::UNDEFINED)
}

struct ValidityBreakdown {
    valid: bool,
    value_missing: bool,
    type_mismatch: bool,
    pattern_mismatch: bool,
    too_long: bool,
    too_short: bool,
    range_underflow: bool,
    range_overflow: bool,
    custom_error: bool,
    message: String,
}

fn evaluate_node_validity(doc: &Document, nid: NodeId) -> ValidityBreakdown {
    let mut out = ValidityBreakdown {
        valid: true,
        value_missing: false,
        type_mismatch: false,
        pattern_mismatch: false,
        too_long: false,
        too_short: false,
        range_underflow: false,
        range_overflow: false,
        custom_error: false,
        message: String::new(),
    };
    let custom = doc.custom_validity(nid);
    if !custom.is_empty() {
        out.valid = false;
        out.custom_error = true;
        out.message = custom.to_string();
    }
    let data = doc.get(nid).data();
    let Some(tag) = data.tag() else {
        return out;
    };
    if tag == "form" || tag == "fieldset" {
        if doc.is_control_valid(nid) == Some(false) {
            out.valid = false;
            if out.message.is_empty() {
                out.message = "Form contains invalid controls.".to_string();
            }
        }
        return out;
    }
    if !matches!(tag, "input" | "textarea" | "select") {
        return out;
    }
    if data
        .attr("disabled")
        .is_some_and(|v| !v.eq_ignore_ascii_case("false"))
        || data
            .attr("readonly")
            .is_some_and(|v| !v.eq_ignore_ascii_case("false"))
    {
        return out;
    }
    let input_type = if tag == "input" {
        data.attr("type")
            .unwrap_or("text")
            .trim()
            .to_ascii_lowercase()
    } else {
        String::new()
    };
    let is_required = data
        .attr("required")
        .is_some_and(|v| !v.eq_ignore_ascii_case("false"));
    if matches!(input_type.as_str(), "checkbox" | "radio") {
        if is_required && !doc.is_checked(nid) {
            out.valid = false;
            out.value_missing = true;
            if out.message.is_empty() {
                out.message = "Please check this box if you want to proceed.".to_string();
            }
        }
        return out;
    }
    let val = doc.input_value(nid);
    if is_required && val.trim().is_empty() {
        out.valid = false;
        out.value_missing = true;
        if out.message.is_empty() {
            out.message = "Please fill out this field.".to_string();
        }
        return out;
    }
    if val.is_empty() {
        return out;
    }
    let char_len = val.chars().count();
    if let Some(min_len) = data
        .attr("minlength")
        .and_then(|s| s.trim().parse::<usize>().ok())
        && char_len < min_len
    {
        out.valid = false;
        out.too_short = true;
        if out.message.is_empty() {
            out.message = format!("Please lengthen this text to {min_len} characters or more.");
        }
    }
    if let Some(max_len) = data
        .attr("maxlength")
        .and_then(|s| s.trim().parse::<usize>().ok())
        && char_len > max_len
    {
        out.valid = false;
        out.too_long = true;
        if out.message.is_empty() {
            out.message = format!("Please shorten this text to {max_len} characters or fewer.");
        }
    }
    match input_type.as_str() {
        "email" => {
            let trimmed = val.trim();
            let valid_email = trimmed.split_once('@').is_some_and(|(l, d)| {
                !l.is_empty()
                    && !d.is_empty()
                    && !d.contains('@')
                    && !trimmed.chars().any(char::is_whitespace)
            });
            if !valid_email {
                out.valid = false;
                out.type_mismatch = true;
                if out.message.is_empty() {
                    out.message = "Please enter an email address.".to_string();
                }
            }
        }
        "url" => {
            let trimmed = val.trim();
            let valid_url = trimmed.split_once("://").is_some_and(|(s, r)| {
                !s.is_empty()
                    && !r.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                    && !trimmed.chars().any(char::is_whitespace)
            });
            if !valid_url {
                out.valid = false;
                out.type_mismatch = true;
                if out.message.is_empty() {
                    out.message = "Please enter a URL.".to_string();
                }
            }
        }
        "number" | "range" => {
            if let Ok(num) = val.trim().parse::<f64>() {
                if let Some(min_v) = data.attr("min").and_then(|s| s.trim().parse::<f64>().ok())
                    && num < min_v
                {
                    out.valid = false;
                    out.range_underflow = true;
                    if out.message.is_empty() {
                        out.message = format!("Value must be greater than or equal to {min_v}.");
                    }
                }
                if let Some(max_v) = data.attr("max").and_then(|s| s.trim().parse::<f64>().ok())
                    && num > max_v
                {
                    out.valid = false;
                    out.range_overflow = true;
                    if out.message.is_empty() {
                        out.message = format!("Value must be less than or equal to {max_v}.");
                    }
                }
            } else {
                out.valid = false;
                out.type_mismatch = true;
                if out.message.is_empty() {
                    out.message = "Please enter a number.".to_string();
                }
            }
        }
        _ => {}
    }
    if out.valid && doc.is_control_valid(nid) == Some(false) {
        out.valid = false;
        out.pattern_mismatch = true;
        if out.message.is_empty() {
            out.message = "Please match the requested format.".to_string();
        }
    }
    out
}

fn el_get_validity(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let bd = unwrap_node(cx, this)
        .and_then(|nid| {
            Some(evaluate_node_validity(
                &cx.host_as::<CinderDomHost>()?.doc,
                nid,
            ))
        })
        .unwrap_or(ValidityBreakdown {
            valid: true,
            value_missing: false,
            type_mismatch: false,
            pattern_mismatch: false,
            too_long: false,
            too_short: false,
            range_underflow: false,
            range_overflow: false,
            custom_error: false,
            message: String::new(),
        });
    let obj = cx.vm().new_plain_object()?;
    for (k, v) in [
        ("valid", bd.valid),
        ("valueMissing", bd.value_missing),
        ("typeMismatch", bd.type_mismatch),
        ("patternMismatch", bd.pattern_mismatch),
        ("tooLong", bd.too_long),
        ("tooShort", bd.too_short),
        ("rangeUnderflow", bd.range_underflow),
        ("rangeOverflow", bd.range_overflow),
        ("stepMismatch", false),
        ("badInput", false),
        ("customError", bd.custom_error),
    ] {
        cx.vm().define_data(obj, k, JsValue::boolean(v))?;
    }
    Ok(JsValue::object(obj))
}

fn el_get_validation_message(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let msg = unwrap_node(cx, this)
        .and_then(|nid| {
            Some(evaluate_node_validity(&cx.host_as::<CinderDomHost>()?.doc, nid).message)
        })
        .unwrap_or_default();
    cx.vm().string(&msg)
}

fn el_get_will_validate(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let will = unwrap_node(cx, this)
        .and_then(|nid| {
            let h = cx.host_as::<CinderDomHost>()?;
            let data = h.doc.get(nid).data();
            let tag = data.tag()?;
            if !matches!(tag, "input" | "textarea" | "select") {
                return Some(false);
            }
            let disabled = data
                .attr("disabled")
                .is_some_and(|v| !v.eq_ignore_ascii_case("false"));
            let readonly = data
                .attr("readonly")
                .is_some_and(|v| !v.eq_ignore_ascii_case("false"));
            Some(!disabled && !readonly)
        })
        .unwrap_or(false);
    Ok(JsValue::boolean(will))
}

fn el_check_validity(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::TRUE);
    };
    let valid = cx
        .host_as::<CinderDomHost>()
        .map(|h| evaluate_node_validity(&h.doc, nid).valid)
        .unwrap_or(true);
    if !valid {
        let ev = cx.vm().new_plain_object()?;
        let inv_str = cx.vm().string("invalid")?;
        cx.vm().define_data(ev, "type", inv_str)?;
        cx.vm().define_data(ev, "bubbles", JsValue::FALSE)?;
        cx.vm().define_data(ev, "cancelable", JsValue::TRUE)?;
        let _ = el_dispatch_event(cx, this, &[JsValue::object(ev)]);
    }
    Ok(JsValue::boolean(valid))
}

fn el_report_validity(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_check_validity(cx, this, args)
}

fn el_set_custom_validity(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let msg = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.doc.set_custom_validity(nid, &msg);
    }
    Ok(JsValue::UNDEFINED)
}

fn note_script_submission(
    cx: &mut NativeCx<'_>,
    form_id: NodeId,
    submitter_id: Option<NodeId>,
    mode: SubmitDispatch,
) {
    let Some(h) = cx.host_as::<CinderDomHost>() else {
        return;
    };
    let Some(form_id) = h.doc.enclosing_form(form_id) else {
        return;
    };
    let generation = h.doc.mutation_generation();
    let document = h.doc.identity();
    h.last_host_submission = Some(HostSubmission {
        form: form_id,
        submitter: submitter_id,
        mode,
        generation,
        document,
    });
}

fn el_form_submit(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.budget_armed && h.turn_budget_left == 0)
    {
        return Ok(JsValue::UNDEFINED);
    }
    let Some(form_id) = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.enclosing_form(nid))
    else {
        return Ok(JsValue::UNDEFINED);
    };
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| form_cannot_navigate(&h.doc, form_id, h.document_fully_active))
    {
        return Ok(JsValue::UNDEFINED);
    }
    commit_submission_plan(cx, form_id, None, SubmitDispatch::Direct)?;
    Ok(JsValue::UNDEFINED)
}

pub(crate) fn submission_base_url(doc: &Document, document_url: &str) -> String {
    let bases = doc.get_elements_by_tag_name(doc.root(), "base");
    for id in bases {
        if let Some(href) = doc.get(id).data().attr("href") {
            let href = href.trim();
            if !href.is_empty() {
                return crate::resolve_script_fetch_url(document_url, href);
            }
        }
    }
    document_url.to_string()
}

fn form_cannot_navigate(doc: &Document, form: NodeId, fully_active: bool) -> bool {
    !fully_active || !doc.contains_node(doc.root(), form)
}

fn is_image_submit(doc: &Document, id: NodeId) -> bool {
    let data = doc.get(id).data();
    data.tag() == Some("input")
        && data
            .attr("type")
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("image"))
}

fn checked_request_submitter(
    cx: &mut NativeCx<'_>,
    form_id: NodeId,
    args: &[JsValue],
) -> Result<Option<NodeId>, RunError> {
    let Some(arg) = args.first().copied() else {
        return Ok(None);
    };
    if arg.is_undefined() || arg.is_null() {
        return Ok(None);
    }
    let Some(id) = unwrap_node(cx, arg) else {
        return Err(cx.type_error("requestSubmit submitter is not a submit button"));
    };
    let (button, owner) = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            let button = h.doc.is_submit_control(id) || is_image_submit(&h.doc, id);
            let owner = h.doc.enclosing_form(id);
            (button, owner)
        })
        .unwrap_or((false, None));
    if !button {
        return Err(cx.type_error("requestSubmit submitter is not a submit button"));
    }
    if owner != Some(form_id) {
        return Err(not_found_error(
            cx,
            "requestSubmit submitter is not owned by this form",
        ));
    }
    Ok(Some(id))
}

fn not_found_error(cx: &mut NativeCx<'_>, message: &str) -> RunError {
    let Ok(value) = cx.vm().make_error(cinder::vm::ErrorKind::Error, message) else {
        return cx.type_error(message);
    };
    if let Some(obj) = value.as_object()
        && let Ok(name) = cx.vm().string("NotFoundError")
    {
        let _ = cx.vm().define_data(obj, "name", name);
    }
    RunError::Thrown {
        value,
        message: format!("NotFoundError: {message}"),
        stack: Vec::new(),
    }
}

fn clear_firing(cx: &mut NativeCx<'_>, form: NodeId) {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.firing_submission.retain(|id| *id != form);
    }
}

fn el_form_request_submit(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(nid) = unwrap_node(cx, this) else {
        return Ok(JsValue::UNDEFINED);
    };
    let form_id = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.doc.enclosing_form(nid));
    let Some(form_id) = form_id else {
        return Ok(JsValue::UNDEFINED);
    };
    // Checkpoint 1: before the submit event. An early return keeps any
    // earlier plan. Connection and document activity are separate from
    // identity and epoch.
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| form_cannot_navigate(&h.doc, form_id, h.document_fully_active))
    {
        return Ok(JsValue::UNDEFINED);
    }
    // Submitter type and form owner are checked before events or a snapshot.
    // A bad argument leaves any earlier planned snapshot where it is.
    let submitter_id = checked_request_submitter(cx, form_id, args)?;
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.budget_armed && h.turn_budget_left == 0)
    {
        return Ok(JsValue::UNDEFINED);
    }
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.firing_submission.contains(&form_id))
    {
        return Ok(JsValue::UNDEFINED);
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.firing_submission.push(form_id);
    }
    let allowed = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| super::form_request_allowed(&h.doc, form_id, submitter_id));
    if !allowed {
        // Invalid controls are notified before requestSubmit returns, in
        // this task. No navigation snapshot.
        let invalid = dispatch_invalid_controls(cx, form_id);
        clear_firing(cx, form_id);
        invalid?;
        return Ok(JsValue::UNDEFINED);
    }
    let dispatched = dispatch_dom_event(cx, form_id, "submit");
    clear_firing(cx, form_id);
    let continued = match dispatched {
        Ok(continued) => continued,
        Err(error) => {
            // A throw is not preventDefault. One submission continues.
            let prevented = cx
                .host_as::<CinderDomHost>()
                .is_some_and(|h| h.event_default_prevented);
            let stopped = cx
                .host_as::<CinderDomHost>()
                .is_some_and(|h| h.budget_armed && h.turn_budget_left == 0);
            if !prevented && !stopped {
                commit_submission_plan(cx, form_id, submitter_id, SubmitDispatch::Interactive)?;
            }
            return Err(error);
        }
    };
    if continued {
        let stopped = cx
            .host_as::<CinderDomHost>()
            .is_some_and(|h| h.budget_armed && h.turn_budget_left == 0);
        if !stopped {
            commit_submission_plan(cx, form_id, submitter_id, SubmitDispatch::Interactive)?;
        }
    }
    Ok(JsValue::UNDEFINED)
}

/// After the submit listener. Checkpoint 2, then `formdata`, then checkpoint 3.
/// Only then does a new plan replace the previous one.
fn commit_submission_plan(
    cx: &mut NativeCx<'_>,
    form_id: NodeId,
    submitter_id: Option<NodeId>,
    mode: SubmitDispatch,
) -> Result<(), RunError> {
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| form_cannot_navigate(&h.doc, form_id, h.document_fully_active))
    {
        return Ok(());
    }
    let dialog_outcome = cx.host_as::<CinderDomHost>().and_then(|h| {
        if !h.doc.is_dialog_form_submission(form_id, submitter_id) {
            return None;
        }
        let result = h.doc.dialog_submission_result(submitter_id);
        let closed = h
            .doc
            .enclosing_dialog(form_id)
            .filter(|&dlg| h.doc.close_dialog_element(dlg, result.as_deref()));
        Some(closed)
    });
    if let Some(closed_dialog) = dialog_outcome {
        if let Some(dlg_id) = closed_dialog {
            fire_dialog_close_event(cx, dlg_id)?;
        }
        return Ok(());
    }
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.constructing_entry.contains(&form_id))
    {
        return Ok(());
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.constructing_entry.push(form_id);
    }
    let formdata = dispatch_dom_event(cx, form_id, "formdata");
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.constructing_entry.retain(|id| *id != form_id);
    }
    formdata?;
    if cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| form_cannot_navigate(&h.doc, form_id, h.document_fully_active))
    {
        return Ok(());
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        if h.capture_form_snapshot(form_id, submitter_id) {
            note_script_submission(cx, form_id, submitter_id, mode);
        }
    }
    Ok(())
}

fn dispatch_invalid_controls(cx: &mut NativeCx<'_>, form: NodeId) -> Result<(), RunError> {
    let controls = cx.host_as::<CinderDomHost>().map(|h| {
        let mut out = Vec::new();
        fn walk(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
            if doc.is_control_valid(id) == Some(false)
                && matches!(
                    doc.get(id).data().tag(),
                    Some("input" | "textarea" | "select" | "button")
                )
            {
                out.push(id);
            }
            for child in doc.children(id) {
                walk(doc, *child, out);
            }
        }
        walk(&h.doc, form, &mut out);
        out
    });
    let Some(controls) = controls else {
        return Ok(());
    };
    for control in controls {
        let _ = dispatch_dom_event(cx, control, "invalid")?;
    }
    Ok(())
}

/// Run the page's listener through the JS event dispatcher. The native
/// listener slot is only a capacity reservation; the callback lives in
/// the JS listener list. Returns whether the default action is still
/// allowed. One dispatch, not a second pass over the native slot.
fn dispatch_dom_event(
    cx: &mut NativeCx<'_>,
    target: NodeId,
    event_type: &str,
) -> Result<bool, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.event_default_prevented = false;
    }
    let target_val = wrap_node(cx, target)?;
    let g = cx.vm().global();
    let fn_key = cx
        .heap()
        .key_from_str("__dispatchNativeDomEvent")
        .map_err(|e| RunError::internal(format!("{e:?}")))?;
    if let Ok(Some(prop)) = cx.heap().get_own(g, fn_key)
        && let Some(fn_val) = prop.slot.data()
        && fn_val.as_object().is_some()
    {
        let ev_js = cx.vm().string(event_type)?;
        let _ = cx.call(
            fn_val,
            JsValue::object(g),
            &[target_val, ev_js, JsValue::UNDEFINED, JsValue::UNDEFINED],
        )?;
    }
    let prevented = cx
        .host_as::<CinderDomHost>()
        .is_some_and(|h| h.event_default_prevented);
    Ok(!prevented)
}

fn storage_byte_len(rows: &[(String, String)]) -> usize {
    rows.iter()
        .map(|(key, value)| key.len().saturating_add(value.len()))
        .fold(0usize, usize::saturating_add)
}

/// Keep the previous value when an update would pass the origin cap.
fn storage_put_capped(rows: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some(index) = rows.iter().position(|(stored, _)| stored == &key) {
        let next = storage_byte_len(rows)
            .saturating_sub(rows[index].1.len())
            .saturating_add(value.len());
        if next > crate::MAX_STORAGE_ORIGIN_BYTES {
            return;
        }
        rows[index].1 = value;
        return;
    }
    let next = storage_byte_len(rows)
        .saturating_add(key.len())
        .saturating_add(value.len());
    if next > crate::MAX_STORAGE_ORIGIN_BYTES || rows.len() >= crate::MAX_STORAGE_ENTRIES {
        return;
    }
    rows.push((key, value));
}

fn el_request_picture_in_picture(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let Some(id) = unwrap_node(cx, this) else {
        return rejected_named(
            cx,
            "InvalidStateError",
            "requestPictureInPicture needs an element",
        );
    };
    let is_video = cx.host_as::<CinderDomHost>().is_some_and(|h| {
        h.doc
            .get(id)
            .data()
            .tag()
            .is_some_and(|tag| tag.eq_ignore_ascii_case("video"))
    });
    if !is_video {
        return rejected_named(
            cx,
            "InvalidStateError",
            "requestPictureInPicture is only for video elements",
        );
    }
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.picture_in_picture = Some(id);
    }
    let node = wrap_node(cx, id)?;
    resolved_promise(cx, node)
}

fn el_leave_picture_in_picture(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    if let Some(h) = cx.host_as::<CinderDomHost>() {
        h.picture_in_picture = None;
    }
    resolved_promise(cx, JsValue::UNDEFINED)
}

fn doc_exit_picture_in_picture(
    cx: &mut NativeCx<'_>,
    this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    el_leave_picture_in_picture(cx, this, args)
}

fn doc_get_picture_in_picture(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.picture_in_picture);
    match id {
        Some(id) => wrap_node(cx, id),
        None => Ok(JsValue::NULL),
    }
}

fn rejected_named(cx: &mut NativeCx<'_>, name: &str, message: &str) -> Result<JsValue, RunError> {
    let err = cx
        .vm()
        .new_error_value(cinder::vm::ErrorKind::Type, message)?;
    if let Some(obj) = err.as_object() {
        let name_js = cx.vm().string(name)?;
        cx.vm().define_data(obj, "name", name_js)?;
    }
    cx.keep(err);
    cx.new_rejected_promise(err)
}

fn resolved_promise(cx: &mut NativeCx<'_>, value: JsValue) -> Result<JsValue, RunError> {
    let (promise, resolve, _reject) = cx.new_promise_with_resolvers()?;
    cx.keep(promise);
    cx.keep(resolve);
    cx.call(resolve, JsValue::UNDEFINED, &[value])?;
    Ok(promise)
}

fn install_webassembly(vm: &mut Vm) -> Result<(), RunError> {
    let g = vm.global();
    let ns = vm.new_plain_object()?;
    vm.define_method(ns, "validate", wasm_validate)?;
    vm.define_method(ns, "Module", wasm_module)?;
    vm.define_method(ns, "Instance", wasm_instance)?;
    vm.define_data(g, "WebAssembly", JsValue::object(ns))?;
    Ok(())
}

fn wasm_validate(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let bytes = wasm_bytes(cx, args.first().copied().unwrap_or(JsValue::UNDEFINED))?;
    Ok(JsValue::boolean(crate::wasm::validate_wasm(&bytes)))
}

fn wasm_module(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let bytes = wasm_bytes(cx, args.first().copied().unwrap_or(JsValue::UNDEFINED))?;
    let module = match crate::wasm::WasmModule::decode(&bytes) {
        Ok(module) => module,
        Err(err) => return Err(cx.type_error(&err.to_string())),
    };
    let index = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            h.wasm_modules.push(module);
            h.wasm_modules.len() - 1
        })
        .unwrap_or(0);
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "__wasmMod", JsValue::number(index as f64))?;
    Ok(JsValue::object(obj))
}

fn wasm_instance(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let module_index = args.first().and_then(|v| v.as_object()).and_then(|obj| {
        let key = cx.heap().key_from_str("__wasmMod").ok()?;
        cx.heap()
            .get_own(obj, key)
            .ok()
            .flatten()
            .and_then(|p| p.slot.data())
            .and_then(|v| v.as_number())
            .map(|n| n as usize)
    });
    let Some(index) = module_index else {
        return Err(cx.type_error("WebAssembly.Instance expects a module"));
    };
    let module = cx
        .host_as::<CinderDomHost>()
        .and_then(|h| h.wasm_modules.get(index).cloned());
    let Some(module) = module else {
        return Err(cx.type_error("unknown WebAssembly module"));
    };
    let instance =
        match crate::wasm::WasmInstance::instantiate(module, std::collections::HashMap::new()) {
            Ok(instance) => instance,
            Err(err) => return Err(cx.type_error(&err.to_string())),
        };
    let export_names: Vec<String> = instance
        .module
        .exports
        .iter()
        .filter(|exp| exp.kind == crate::wasm::WasmExportKind::Func)
        .map(|exp| exp.name.clone())
        .collect();
    let inst_index = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            h.wasm_instances.push(instance);
            h.wasm_instances.len() - 1
        })
        .unwrap_or(0);
    let exports = cx.vm().new_plain_object()?;
    let raw = cx.vm().register_native("__anvilWasmCall", wasm_call)?;
    let bind = {
        let Some(fo) = raw.as_object() else {
            return Err(cx.type_error("wasm call is not a function"));
        };
        let key = cx
            .heap()
            .key_from_str("bind")
            .map_err(|e| RunError::internal(e.to_string()))?;
        match cx.heap().lookup(fo, key) {
            Ok(cinder::gc::Lookup::Found { prop, .. }) => {
                prop.slot.data().unwrap_or(JsValue::UNDEFINED)
            }
            _ => JsValue::UNDEFINED,
        }
    };
    for name in export_names {
        let name_js = cx.vm().string(&name)?;
        let bound = cx.call(
            bind,
            raw,
            &[
                JsValue::UNDEFINED,
                JsValue::number(inst_index as f64),
                name_js,
            ],
        )?;
        cx.vm().define_data(exports, &name, bound)?;
    }
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "exports", JsValue::object(exports))?;
    Ok(JsValue::object(obj))
}

fn wasm_call(cx: &mut NativeCx<'_>, _this: JsValue, args: &[JsValue]) -> Result<JsValue, RunError> {
    let index = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
    let name = arg_str(cx, args, 1)?;
    let nums: Vec<f64> = args.iter().skip(2).filter_map(|v| v.as_number()).collect();
    let result = cx.host_as::<CinderDomHost>().and_then(|h| {
        h.wasm_instances
            .get_mut(index)
            .and_then(|inst| inst.call_export(&name, &nums).ok())
    });
    let number = result
        .and_then(|vals| vals.first().copied())
        .map(|value| match value {
            crate::wasm::WasmValue::I32(n) => n as f64,
            crate::wasm::WasmValue::I64(n) => n as f64,
            crate::wasm::WasmValue::F32(n) => n as f64,
            crate::wasm::WasmValue::F64(n) => n,
        })
        .unwrap_or(0.0);
    Ok(JsValue::number(number))
}

fn wasm_bytes(cx: &mut NativeCx<'_>, value: JsValue) -> Result<Vec<u8>, RunError> {
    let Some(obj) = value.as_object() else {
        return Ok(Vec::new());
    };
    if let Ok(buf_key) = cx.heap().key_from_str("buffer")
        && let Ok(Some(buf_prop)) = cx.heap().get_own(obj, buf_key)
        && let Some(buf) = buf_prop.slot.data().and_then(|v| v.as_object())
        && let Ok(bytes_key) = cx.heap().key_from_str("_bytes")
        && let Ok(Some(bytes_prop)) = cx.heap().get_own(buf, bytes_key)
        && let Some(bytes) = bytes_prop.slot.data()
    {
        return Ok(number_list(cx, bytes));
    }
    Ok(number_list(cx, value))
}

fn number_list(cx: &mut NativeCx<'_>, value: JsValue) -> Vec<u8> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let len = cx.heap().array_length(obj).unwrap_or(0);
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let Some(item) = cx.heap().try_get_dense_element(obj, i) else {
            break;
        };
        let Some(n) = item.as_number() else {
            break;
        };
        out.push(n as u8);
    }
    out
}

fn ws_error_obj(cx: &mut NativeCx<'_>, msg: &str) -> Result<JsValue, RunError> {
    let obj = cx.vm().new_plain_object()?;
    cx.vm().define_data(obj, "ok", JsValue::FALSE)?;
    let err_js = cx.vm().string(msg)?;
    cx.vm().define_data(obj, "error", err_js)?;
    Ok(JsValue::object(obj))
}

fn window_native_ws_connect(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let url = arg_str(cx, args, 0)?;
    let protos_json = if args.len() > 1 {
        arg_str(cx, args, 1)?
    } else {
        String::new()
    };
    let mut protocols = Vec::new();
    if !protos_json.is_empty()
        && let Ok(crate::anvil_json::JsonValue::Array(items)) =
            crate::anvil_json::parse(&protos_json)
    {
        for item in items {
            if let Some(p) = item.as_str()
                && !p.trim().is_empty()
            {
                protocols.push(p.trim().to_string());
            }
        }
    }

    let (page, handler, suspended, active, count) = match cx.host_as::<CinderDomHost>() {
        Some(h) => (
            h.location_href.clone(),
            h.ws_handler.0.clone(),
            h.page_suspended,
            h.document_fully_active,
            h.websockets.len(),
        ),
        None => return ws_error_obj(cx, "WebSocket host unavailable"),
    };
    if suspended {
        return ws_error_obj(cx, "page is suspended");
    }
    if !active {
        return ws_error_obj(cx, "document is not fully active");
    }
    if count >= MAX_HOST_WEBSOCKETS {
        return ws_error_obj(cx, "too many open WebSocket connections");
    }
    let Some(connect_fn) = handler else {
        return ws_error_obj(cx, "WebSocket networking is unavailable");
    };
    let conn = match connect_fn(&url, &page, &protocols) {
        Ok(c) => c,
        Err(err) => return ws_error_obj(cx, &err),
    };
    let ready_state = conn.ready_state();
    let proto = conn.protocol().unwrap_or("").to_string();
    let id = {
        let Some(h) = cx.host_as::<CinderDomHost>() else {
            return ws_error_obj(cx, "WebSocket host lost");
        };
        let id = h.next_ws_id;
        h.next_ws_id = h.next_ws_id.wrapping_add(1).max(1);
        h.websockets.insert(id, conn);
        id
    };
    let obj = cx.vm().new_plain_object()?;
    cx.vm().define_data(obj, "ok", JsValue::TRUE)?;
    cx.vm().define_data(obj, "id", JsValue::number(id as f64))?;
    cx.vm()
        .define_data(obj, "readyState", JsValue::number(ready_state as f64))?;
    let proto_js = cx.vm().string(&proto)?;
    cx.vm().define_data(obj, "protocol", proto_js)?;
    Ok(JsValue::object(obj))
}

fn window_native_ws_send(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let is_binary = args.get(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let result = if is_binary {
        let bytes = args
            .get(2)
            .copied()
            .map(|v| number_list(cx, v))
            .unwrap_or_default();
        if let Some(h) = cx.host_as::<CinderDomHost>()
            && let Some(ws) = h.websockets.get_mut(&id)
        {
            ws.send_binary(&bytes)
        } else {
            Err("WebSocket connection not found".to_string())
        }
    } else {
        let text = arg_str(cx, args, 2)?;
        if let Some(h) = cx.host_as::<CinderDomHost>()
            && let Some(ws) = h.websockets.get_mut(&id)
        {
            ws.send_text(&text)
        } else {
            Err("WebSocket connection not found".to_string())
        }
    };
    match result {
        Ok(()) => {
            let obj = cx.vm().new_plain_object()?;
            cx.vm().define_data(obj, "ok", JsValue::TRUE)?;
            Ok(JsValue::object(obj))
        }
        Err(err) => ws_error_obj(cx, &err),
    }
}

fn window_native_ws_close(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let code = args
        .get(1)
        .and_then(|v| v.as_number())
        .map(|n| n as u16)
        .unwrap_or(1000);
    let reason = if args.len() > 2 {
        arg_str(cx, args, 2)?
    } else {
        String::new()
    };
    if let Some(h) = cx.host_as::<CinderDomHost>()
        && let Some(ws) = h.websockets.get_mut(&id)
    {
        let _ = ws.close(code, &reason);
        return Ok(JsValue::TRUE);
    }
    Ok(JsValue::FALSE)
}

fn window_native_ws_poll(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let id = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let (ready_state, events) = {
        let Some(h) = cx.host_as::<CinderDomHost>() else {
            let obj = cx.vm().new_plain_object()?;
            cx.vm()
                .define_data(obj, "readyState", JsValue::number(3.0))?;
            let empty = cx.vm().new_array_from(&[])?;
            cx.vm().define_data(obj, "events", empty)?;
            return Ok(JsValue::object(obj));
        };
        let Some(ws) = h.websockets.get_mut(&id) else {
            let obj = cx.vm().new_plain_object()?;
            cx.vm()
                .define_data(obj, "readyState", JsValue::number(3.0))?;
            let empty = cx.vm().new_array_from(&[])?;
            cx.vm().define_data(obj, "events", empty)?;
            return Ok(JsValue::object(obj));
        };
        let events = ws.poll_events();
        let ready_state = ws.ready_state();
        if ready_state == 3 {
            h.websockets.remove(&id);
        }
        (ready_state, events)
    };
    let mut js_events = Vec::with_capacity(events.len());
    for ev in events {
        let ev_obj = cx.vm().new_plain_object()?;
        match ev {
            HostWsEvent::MessageText(text) => {
                let kind = cx.vm().string("text")?;
                cx.vm().define_data(ev_obj, "kind", kind)?;
                let data = cx.vm().string(&text)?;
                cx.vm().define_data(ev_obj, "data", data)?;
            }
            HostWsEvent::MessageBinary(bytes) => {
                let kind = cx.vm().string("binary")?;
                cx.vm().define_data(ev_obj, "kind", kind)?;
                let nums: Vec<JsValue> = bytes
                    .into_iter()
                    .map(|b| JsValue::number(b as f64))
                    .collect();
                let arr = cx.vm().new_array_from(&nums)?;
                cx.vm().define_data(ev_obj, "data", arr)?;
            }
            HostWsEvent::Closed { code, reason } => {
                let kind = cx.vm().string("closed")?;
                cx.vm().define_data(ev_obj, "kind", kind)?;
                cx.vm()
                    .define_data(ev_obj, "code", JsValue::number(code as f64))?;
                let r = cx.vm().string(&reason)?;
                cx.vm().define_data(ev_obj, "reason", r)?;
            }
            HostWsEvent::Error(msg) => {
                let kind = cx.vm().string("error")?;
                cx.vm().define_data(ev_obj, "kind", kind)?;
                let m = cx.vm().string(&msg)?;
                cx.vm().define_data(ev_obj, "data", m)?;
            }
        }
        js_events.push(JsValue::object(ev_obj));
    }
    let arr = cx.vm().new_array_from(&js_events)?;
    let obj = cx.vm().new_plain_object()?;
    cx.vm()
        .define_data(obj, "readyState", JsValue::number(ready_state as f64))?;
    cx.vm().define_data(obj, "events", arr)?;
    Ok(JsValue::object(obj))
}

fn idb_storage_put_capped(rows: &mut Vec<(String, String)>, db_name: String, json: String) -> bool {
    if db_name.is_empty() {
        return false;
    }
    if let Some(idx) = rows.iter().position(|(name, _)| name == &db_name) {
        let next = storage_byte_len(rows)
            .saturating_sub(rows[idx].1.len())
            .saturating_add(json.len());
        if next > MAX_IDB_ORIGIN_BYTES {
            return false;
        }
        rows[idx].1 = json;
        return true;
    }
    let next = storage_byte_len(rows)
        .saturating_add(db_name.len())
        .saturating_add(json.len());
    if next > MAX_IDB_ORIGIN_BYTES || rows.len() >= MAX_IDB_DATABASES_PER_ORIGIN {
        return false;
    }
    rows.push((db_name, json));
    true
}

fn window_native_idb_load(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    let found = cx.host_as::<CinderDomHost>().and_then(|h| {
        h.indexed_db
            .iter()
            .find(|(db, _)| db == &name)
            .map(|(_, json)| json.clone())
    });
    match found {
        Some(json) => cx.vm().string(&json),
        None => Ok(JsValue::NULL),
    }
}

fn window_native_idb_save(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    let json = arg_str(cx, args, 1)?;
    let ok = cx.host_as::<CinderDomHost>().is_some_and(|h| {
        !h.page_suspended && idb_storage_put_capped(&mut h.indexed_db, name, json)
    });
    Ok(JsValue::boolean(ok))
}

fn window_native_idb_delete(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    args: &[JsValue],
) -> Result<JsValue, RunError> {
    let name = arg_str(cx, args, 0)?;
    if let Some(h) = cx.host_as::<CinderDomHost>()
        && !h.page_suspended
    {
        h.indexed_db.retain(|(db, _)| db != &name);
        return Ok(JsValue::TRUE);
    }
    Ok(JsValue::FALSE)
}

fn window_native_idb_list(
    cx: &mut NativeCx<'_>,
    _this: JsValue,
    _args: &[JsValue],
) -> Result<JsValue, RunError> {
    let entries: Vec<(String, u64)> = cx
        .host_as::<CinderDomHost>()
        .map(|h| {
            h.indexed_db
                .iter()
                .map(|(name, json)| {
                    let version = crate::anvil_json::parse(json)
                        .ok()
                        .and_then(|v| v.get("version").and_then(|n| n.as_f64()))
                        .map(|n| n.max(1.0) as u64)
                        .unwrap_or(1);
                    (name.clone(), version)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut items = Vec::with_capacity(entries.len());
    for (name, version) in entries {
        let obj = cx.vm().new_plain_object()?;
        let n_js = cx.vm().string(&name)?;
        cx.vm().define_data(obj, "name", n_js)?;
        cx.vm()
            .define_data(obj, "version", JsValue::number(version as f64))?;
        items.push(JsValue::object(obj));
    }
    cx.vm().new_array_from(&items)
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn host_listener_queue_stops_at_the_cap() {
        let mut vm = cinder::vm::Vm::new(cinder::Limits {
            max_heap_bytes: 16 * 1024 * 1024,
            initial_gc_trigger: 1024 * 1024,
            max_steps: Some(1_000),
            ..cinder::Limits::default()
        })
        .expect("vm");
        let callback = vm.heap_mut().persist(JsValue::UNDEFINED).expect("persist");
        let mut host = CinderDomHost::new(Document::new(), "https://localhost/");
        let node = host.doc.body();
        for _ in 0..(MAX_HOST_LISTENERS + 5) {
            let _ = host.push_listener(node, "click".to_string(), callback);
        }
        assert_eq!(host.listeners.len(), MAX_HOST_LISTENERS);
        assert!(!host.push_listener(node, "click".to_string(), callback));
    }
}
