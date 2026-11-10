// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! One wall deadline per top-level browser task.
//!
//! [`budget`] starts at [`PROPOSED_TASK_WALL_BUDGET`]. That provisional
//! default is enabled. [`set_budget`] overrides it for tests. `None` is the
//! only request that disarms. An invalid duration is rejected and the
//! previous budget stays. Each arm pushes one runtime frame and the matching
//! clear pops that same runtime, even if the active page changed. Nested host
//! paths do not arm or clear.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, Instant};

use crate::Session;

/// Provisional enabled budget for one top-level task.
///
/// Chosen from per-task guard timings on this 4-core Xeon.
/// One worker, deadline off: script median 124 ms, p95 133 ms, max 362 ms
/// across 10_987 samples. Three debug fixture passes were 109–130 ms;
/// three release passes were about 51 ms. Input, event, and timer stayed
/// under 10 ms. A parallel suite with a 1500 ms budget aborted synchronous
/// XHR scripts before they recorded a result, so 1500 ms is not enough on
/// a busy worker. 5000 ms is about fourteen times the one-worker maximum
/// and still far under the 30 s cap. It does not raise the 200_000-step
/// limit. A host call already in progress is not preempted (fetch overran
/// a 20 ms budget by 60 ms). Layout and raster do not check the clock.
/// [`set_budget`] overrides this for tests.
pub const PROPOSED_TASK_WALL_BUDGET: Duration = Duration::from_millis(5000);

/// Largest budget the guard will pass to `arm_wall_deadline`.
/// `Duration::MAX` overflows `Instant::now` and is already due.
pub const MAX_TASK_WALL_BUDGET: Duration = Duration::from_secs(30);

/// `true` when `budget` can be added to `Instant::now` and is at most
/// [`MAX_TASK_WALL_BUDGET`]. `Duration::ZERO` is in range and already due.
pub fn duration_in_range(budget: Duration) -> bool {
    budget <= MAX_TASK_WALL_BUDGET && Instant::now().checked_add(budget).is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Script,
    LoadScript,
    Parse,
    Input,
    Timer,
    Event,
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static BUDGET: Cell<Option<Duration>> = const { Cell::new(Some(PROPOSED_TASK_WALL_BUDGET)) };
    static ARMS: Cell<u32> = const { Cell::new(0) };
    static CLEARS: Cell<u32> = const { Cell::new(0) };
    static LEDGER: std::cell::RefCell<std::collections::BTreeMap<u64, (u32, u32)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// The budget a fresh thread starts with.
pub fn default_budget() -> Duration {
    PROPOSED_TASK_WALL_BUDGET
}

/// `Some(default)` until [`set_budget`] changes it.
pub fn budget() -> Option<Duration> {
    BUDGET.with(|cell| cell.get())
}

/// Arms and clears recorded for one document runtime. `(arms, clears)`.
pub fn runtime_arm_clear(document: u64) -> (u32, u32) {
    LEDGER.with(|cell| cell.borrow().get(&document).copied().unwrap_or((0, 0)))
}

pub fn reset_runtime_ledger() {
    LEDGER.with(|cell| cell.borrow_mut().clear());
}

pub(crate) fn note_runtime_armed(document: u64) {
    note_runtime(document, true);
}

fn note_runtime(document: u64, arm: bool) {
    LEDGER.with(|cell| {
        let mut ledger = cell.borrow_mut();
        let entry = ledger.entry(document).or_insert((0, 0));
        if arm {
            entry.0 = entry.0.saturating_add(1);
        } else {
            entry.1 = entry.1.saturating_add(1);
        }
    });
}

/// Why [`set_budget`] left the previous budget in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError {
    /// Zero is not a task budget here. `Duration::ZERO` is already due in the
    /// runtime, and installing it would not be protection.
    Zero,
    /// Longer than [`MAX_TASK_WALL_BUDGET`], or `Instant::now() + budget` overflows.
    /// `Duration::MAX` is in this class. The runtime would treat it as already due.
    OutOfRange,
}

/// Install `budget` for later top-level tasks on this thread.
///
/// `None` disarms, which is how 4ad6 ships the deadline. `Some` must be in
/// range. An invalid value returns [`BudgetError`] and keeps the previous
/// budget. It is not turned into `None`.
pub fn set_budget(budget: Option<Duration>) -> Result<(), BudgetError> {
    match budget {
        None => {
            BUDGET.with(|cell| cell.set(None));
            Ok(())
        }
        Some(budget) if budget.is_zero() => Err(BudgetError::Zero),
        Some(budget) if !duration_in_range(budget) => Err(BudgetError::OutOfRange),
        Some(budget) => {
            BUDGET.with(|cell| cell.set(Some(budget)));
            Ok(())
        }
    }
}

/// How many times this thread has armed through the guard.
pub fn arm_count() -> u32 {
    ARMS.with(|cell| cell.get())
}

/// How many times this thread has cleared through the guard.
pub fn clear_count() -> u32 {
    CLEARS.with(|cell| cell.get())
}

pub fn task_depth() -> u32 {
    DEPTH.with(|cell| cell.get())
}

pub fn reset_arm_count() {
    ARMS.with(|cell| cell.set(0));
    CLEARS.with(|cell| cell.set(0));
}

/// Recorded task durations for the timing pass. Cleared by [`take_samples`].
fn samples() -> &'static std::sync::Mutex<Vec<(TaskKind, Duration)>> {
    use std::sync::OnceLock;
    static SAMPLES: OnceLock<std::sync::Mutex<Vec<(TaskKind, Duration)>>> = OnceLock::new();
    SAMPLES.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

pub fn take_samples() -> Vec<(TaskKind, Duration)> {
    let mut guard = samples().lock().unwrap_or_else(|err| err.into_inner());
    std::mem::take(&mut *guard)
}

fn record(kind: TaskKind, elapsed: Duration) {
    if let Ok(mut guard) = samples().lock() {
        guard.push((kind, elapsed));
    }
    if let Ok(path) = std::env::var("ANVIL_TASK_TIMING_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write;
        let thread = std::thread::current().name().unwrap_or("-").to_string();
        let _ = writeln!(file, "{thread} {kind:?} {}", elapsed.as_nanos());
    }
}

/// Time `body` without arming. Used for the load pipeline and HTML parse,
/// which contain the script tasks rather than being one script task.
pub fn measure<R>(kind: TaskKind, body: impl FnOnce() -> R) -> R {
    let started = Instant::now();
    let value = body();
    record(kind, started.elapsed());
    value
}

/// Run `body` as one task on `runtime` when no task is already open.
///
/// A nested call does not arm and does not clear. The outer guard clears
/// after `body` returns and after `body` panics.
pub fn with_runtime<R>(
    runtime: &mut anvil_script::ScriptRuntime,
    kind: TaskKind,
    body: impl FnOnce(&mut anvil_script::ScriptRuntime) -> R,
) -> R {
    let outer = DEPTH.with(|cell| cell.get()) == 0;
    let budget = if outer { budget() } else { None };
    DEPTH.with(|cell| cell.set(cell.get().saturating_add(1)));
    let mut armed = false;
    if let Some(budget) = budget.filter(|budget| duration_in_range(*budget)) {
        runtime.arm_wall_deadline(budget);
        ARMS.with(|cell| cell.set(cell.get().saturating_add(1)));
        armed = true;
    }
    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| body(runtime)));
    record(kind, started.elapsed());
    DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    if armed {
        runtime.clear_wall_deadline();
        CLEARS.with(|cell| cell.set(cell.get().saturating_add(1)));
    }
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}

/// Same guard as [`with_runtime`], using the active page's runtime.
pub fn with_session<R>(
    session: &mut Session,
    kind: TaskKind,
    body: impl FnOnce(&mut Session) -> R,
) -> R {
    let outer = DEPTH.with(|cell| cell.get()) == 0;
    let budget = if outer { budget() } else { None };
    DEPTH.with(|cell| cell.set(cell.get().saturating_add(1)));
    let armed_document = if let Some(budget) = budget.filter(|budget| duration_in_range(*budget)) {
        session.arm_active_document_deadline(budget)
    } else {
        None
    };
    if armed_document.is_some() {
        ARMS.with(|cell| cell.set(cell.get().saturating_add(1)));
    }
    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| body(session)));
    record(kind, started.elapsed());
    DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    if let Some(document) = armed_document
        && session.clear_document_deadline(document)
    {
        CLEARS.with(|cell| cell.set(cell.get().saturating_add(1)));
        note_runtime(document, false);
    }
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}
