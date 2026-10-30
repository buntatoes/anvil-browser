// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! How the host (the DOM layer) plugs in, and how native functions work.
//!
//! The engine knows nothing about pages. The host implements [`Host`] for
//! the few things only it can do: run jobs later, decide when a script has
//! run too long, report the values it holds, and hear about wrappers that
//! died.
//!
//! Builtins and host functions are both [`NativeFn`]s: plain Rust functions
//! that get a [`NativeCx`]. Through it they can reach the heap, throw
//! errors, call back into script, and reach the host's own state with
//! [`NativeCx::host_as`].
//!
//! A collection can run while a native function is calling back into
//! script. Values the native holds in Rust locals across such a call must be
//! kept with [`NativeCx::keep`]; everything kept is released when the
//! native returns.

use super::{RunError, Vm};
use crate::cinder::gc::Heap;
use crate::cinder::gc::roots::Persistent;
use crate::cinder::value::Value;
use crate::cinder::value::object::HostData;
use std::any::Any;

/// A native function: `(cx, this, args) -> result`.
pub type NativeFn = fn(&mut NativeCx<'_>, Value, &[Value]) -> Result<Value, RunError>;

/// A call the host should make later (a promise reaction, a timer).
/// Its values are held as persistent roots until the job runs.
#[derive(Debug)]
pub struct Job {
    pub(crate) function: Persistent,
    pub(crate) this: Persistent,
    pub(crate) args: Vec<Persistent>,
}

/// What the engine needs from its host.
pub trait Host {
    /// Keep `job` and hand it back to [`Vm::run_job`] later.
    fn enqueue_job(&mut self, job: Job);

    /// Pop the next queued job (FIFO), if the host keeps an internal microtask queue.
    fn take_job(&mut self) -> Option<Job> {
        None
    }

    /// Asked every `interrupt_check_interval` steps. `Some(reason)` stops
    /// the script (uncatchable), with the reason in the error.
    fn interrupt(&mut self) -> Option<String> {
        None
    }

    /// Add any values the host holds outside persistent roots.
    fn trace(&self, _roots: &mut Vec<Value>) {}

    /// A wrapper for `data` was collected.
    fn finalized(&mut self, _data: HostData) {}

    /// Milliseconds since some fixed point, for `Date.now` and timers.
    fn now_ms(&mut self) -> f64 {
        0.0
    }

    /// One line of `console` output. `level` is `log`, `info`, `warn` or
    /// `error`. The default drops it.
    fn console(&mut self, _level: &str, _text: &str) {}

    /// The host itself, so native functions can reach its state.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A host that does nothing: jobs are kept in a list, nothing is
/// interrupted. Handy for tests and for running plain scripts.
#[derive(Debug, Default)]
pub struct BasicHost {
    /// Jobs enqueued so far.
    pub jobs: Vec<Job>,
    /// Interrupt after this many checks (`None` never).
    pub interrupt_after: Option<u64>,
    /// Interrupt checks so far.
    pub checks: u64,
    /// Host data of wrappers that died.
    pub finalized: Vec<HostData>,
    /// `console` output so far, one entry per call: `(level, text)`.
    pub console: Vec<(String, String)>,
}

impl Host for BasicHost {
    fn console(&mut self, level: &str, text: &str) {
        self.console.push((level.to_string(), text.to_string()));
    }

    fn enqueue_job(&mut self, job: Job) {
        self.jobs.push(job);
    }

    fn take_job(&mut self) -> Option<Job> {
        if self.jobs.is_empty() {
            None
        } else {
            Some(self.jobs.remove(0))
        }
    }

    fn interrupt(&mut self) -> Option<String> {
        self.checks += 1;
        match self.interrupt_after {
            Some(n) if self.checks >= n => {
                Some(format!("interrupted after {} checks", self.checks))
            }
            _ => None,
        }
    }

    fn finalized(&mut self, data: HostData) {
        self.finalized.push(data);
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// What a native function can reach while it runs.
pub struct NativeCx<'a> {
    pub(crate) vm: &'a mut Vm,
    pub(crate) host: &'a mut dyn Host,
}

impl NativeCx<'_> {
    /// The heap.
    pub fn heap(&mut self) -> &mut Heap {
        &mut self.vm.heap
    }

    /// The VM.
    pub fn vm(&mut self) -> &mut Vm {
        self.vm
    }

    /// The host as its own type, if it is one.
    pub fn host_as<T: 'static>(&mut self) -> Option<&mut T> {
        self.host.as_any_mut().downcast_mut::<T>()
    }

    /// Keep `v` alive until this native returns, even if a collection runs
    /// during a callback into script.
    pub fn keep(&mut self, v: Value) {
        self.vm.native_roots.push(v);
    }

    /// Call a function (script or native) and wait for its result. This
    /// counts toward `max_native_reentry`.
    pub fn call(&mut self, f: Value, this: Value, args: &[Value]) -> Result<Value, RunError> {
        self.vm.call_reentrant(self.host, f, this, args)
    }

    /// A `TypeError` to return from a native.
    pub fn type_error(&mut self, message: &str) -> RunError {
        self.vm.make_thrown(super::ErrorKind::Type, message)
    }

    /// A `RangeError` to return from a native.
    pub fn range_error(&mut self, message: &str) -> RunError {
        self.vm.make_thrown(super::ErrorKind::Range, message)
    }

    /// Ask the host to call `f` later with `args`.
    pub fn enqueue_job(&mut self, f: Value, this: Value, args: &[Value]) -> Result<(), RunError> {
        let job = self.vm.make_job(f, this, args)?;
        self.host.enqueue_job(job);
        Ok(())
    }

    /// The value as a string, the way `String(v)` would give it for
    /// primitives and plain objects.
    pub fn to_display_string(&mut self, v: Value) -> Result<String, RunError> {
        self.vm.to_display_string(v)
    }
}
