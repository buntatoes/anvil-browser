// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The virtual machine: loads bytecode, runs it, and reports how it ended.
//!
//! ```
//! use anvil_script::cinder::bytecode::{BinOp, CodeBlock, Op};
//! use anvil_script::cinder::limits::Limits;
//! use anvil_script::cinder::vm::{BasicHost, Vm};
//!
//! let mut b = CodeBlock::new("main", 0, 3);
//! b.emit(Op::LoadInt { dst: 0, value: 20 });
//! b.emit(Op::LoadInt { dst: 1, value: 22 });
//! b.emit(Op::Binary { op: BinOp::Add, dst: 2, a: 0, b: 1 });
//! b.emit(Op::Return { src: 2 });
//!
//! let mut vm = Vm::new(Limits::default()).unwrap();
//! let code = vm.load(b).unwrap();
//! let result = vm.run(&mut BasicHost::default(), code).unwrap();
//! assert_eq!(result.as_number(), Some(42.0));
//! ```
//!
//! # How a run can end
//!
//! A run returns a value or a [`RunError`]: a script exception nobody
//! caught (`Thrown`), an uncatchable limit (`Abort`: heap bytes, the step
//! budget, a host interrupt), a load failure (`Compile`), or an engine bug
//! (`Internal`).
//!
//! # Collections
//!
//! Allocation only asks for a collection; it runs at the start of the next
//! op (a safepoint). Roots are every register, every frame's `this`,
//! function and environment, the global object and the intrinsics, string
//! constants of loaded code, values native functions asked to keep, the
//! host's persistent roots, and whatever [`Host::trace`] adds.
//!
//! # Conversions
//!
//! Turning an object into a primitive doesn't call script `valueOf` or
//! `toString` yet (that comes with the builtins): plain objects give
//! `"[object Object]"`, arrays are joined with commas, errors give
//! `"Name: message"`, and boxed primitives give their value.

mod builtins;
mod error;
mod extra;
mod frame;
mod host;
pub mod ic;
mod interp;
mod regex;
mod stdlib;

pub use error::{RunError, StackLine};
pub use host::{BasicHost, Host, Job, NativeCx, NativeFn};
pub use ic::{IcEntry, IcSite, IcState, IcStats};

use crate::cinder::bytecode::CodeBlock;
use crate::cinder::gc::{Heap, HeapError};
use crate::cinder::limits::{LimitKind, Limits, group_digits};
use crate::cinder::value::atom::PropertyKey;
use crate::cinder::value::number::{number_to_string, string_to_number, to_int32};
use crate::cinder::value::object::{ObjectClass, Property, Slot};
use crate::cinder::value::shape::Attrs;
use crate::cinder::value::string::compare_units;
use crate::cinder::value::{Kind, ObjectRef, StringRef, Value};
use frame::Frame;
use std::collections::HashSet;

/// A loaded code block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CodeId(pub u32);

/// The error types the engine makes itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `Error`
    Error,
    /// `TypeError`
    Type,
    /// `RangeError`
    Range,
    /// `ReferenceError`
    Reference,
    /// `SyntaxError`
    Syntax,
}

impl ErrorKind {
    /// The constructor name.
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Error => "Error",
            ErrorKind::Type => "TypeError",
            ErrorKind::Range => "RangeError",
            ErrorKind::Reference => "ReferenceError",
            ErrorKind::Syntax => "SyntaxError",
        }
    }

    fn index(self) -> usize {
        match self {
            ErrorKind::Error => 0,
            ErrorKind::Type => 1,
            ErrorKind::Range => 2,
            ErrorKind::Reference => 3,
            ErrorKind::Syntax => 4,
        }
    }

    /// The kind with this number (`ThrowError` uses it).
    pub fn from_index(i: u8) -> Option<ErrorKind> {
        Some(match i {
            0 => ErrorKind::Error,
            1 => ErrorKind::Type,
            2 => ErrorKind::Range,
            3 => ErrorKind::Reference,
            4 => ErrorKind::Syntax,
            _ => return None,
        })
    }

    /// Every kind, in number order.
    pub const ALL: [ErrorKind; 5] = [
        ErrorKind::Error,
        ErrorKind::Type,
        ErrorKind::Range,
        ErrorKind::Reference,
        ErrorKind::Syntax,
    ];
}

/// Why the dispatch loop stopped an op. `Pending` is an error object still
/// to be made, so helpers that only have the heap can throw.
#[derive(Debug)]
pub(crate) enum Exit {
    Throw {
        value: Value,
        stack: Option<Vec<StackLine>>,
    },
    Pending(ErrorKind, String),
    Fatal(RunError),
}

impl From<HeapError> for Exit {
    fn from(e: HeapError) -> Exit {
        match error::lower(e) {
            error::Lowered::Run(r) => Exit::Fatal(r),
            error::Lowered::Range(m) => Exit::Pending(ErrorKind::Range, m),
        }
    }
}

impl From<RunError> for Exit {
    fn from(e: RunError) -> Exit {
        match e {
            RunError::Thrown { value, stack, .. } => Exit::Throw {
                value,
                stack: if stack.is_empty() { None } else { Some(stack) },
            },
            other => Exit::Fatal(other),
        }
    }
}

pub(crate) fn type_err(msg: impl Into<String>) -> Exit {
    Exit::Pending(ErrorKind::Type, msg.into())
}

pub(crate) fn range_err(msg: impl Into<String>) -> Exit {
    Exit::Pending(ErrorKind::Range, msg.into())
}

pub(crate) fn internal_err(msg: impl Into<String>) -> Exit {
    Exit::Fatal(RunError::internal(msg))
}

#[derive(Debug)]
pub(crate) struct LoadedCode {
    pub block: CodeBlock,
    pub keys: Vec<PropertyKey>,
    pub strings: Vec<StringRef>,
    pub children: Vec<u32>,
    pub ic: Vec<IcSite>,
    pub loop_trips: Vec<u64>,
}

#[derive(Debug)]
pub(crate) struct Intrinsics {
    pub global: ObjectRef,
    pub object_prototype: ObjectRef,
    pub function_prototype: ObjectRef,
    pub array_prototype: ObjectRef,
    pub string_prototype: ObjectRef,
    pub number_prototype: ObjectRef,
    pub boolean_prototype: ObjectRef,
    pub symbol_prototype: ObjectRef,
    pub error_prototypes: [ObjectRef; 5],
    /// Global `let`, `const` and `class` bindings. Uninitialized ones hold
    /// `Uninit`; `const` ones aren't writable.
    pub lexicals: ObjectRef,
    pub key_eval: PropertyKey,
    pub key_callee: PropertyKey,
    pub key_constructor: PropertyKey,
    pub typeof_strings: [StringRef; 8],
    pub key_length: PropertyKey,
    pub key_prototype: PropertyKey,
    pub key_message: PropertyKey,
    pub key_name: PropertyKey,
}

const TYPEOF_NAMES: [&str; 8] = [
    "undefined",
    "object",
    "boolean",
    "number",
    "string",
    "symbol",
    "function",
    "bigint",
];

#[derive(Debug)]
pub(crate) struct NativeEntry {
    pub name: String,
    pub f: NativeFn,
}

/// The virtual machine. One `Vm` is one realm: its own heap, globals and
/// loaded code.
#[derive(Debug)]
pub struct Vm {
    pub(crate) heap: Heap,
    pub(crate) codes: Vec<LoadedCode>,
    pub(crate) frames: Vec<Frame>,
    pub(crate) regs: Vec<Value>,
    pub(crate) natives: Vec<NativeEntry>,
    pub(crate) native_roots: Vec<Value>,
    pub(crate) native_depth: u32,
    pub(crate) steps: u64,
    pub(crate) total_steps: u64,
    pub(crate) ic_enabled: bool,
    pub(crate) intr: Intrinsics,
    pub(crate) root_buf: Vec<Value>,
    /// The built-in `eval` function, once installed.
    pub(crate) eval_fn: Option<Value>,
    /// Compiles and runs eval code; set by the compiler.
    pub(crate) eval_hook: Option<EvalHook>,
}

/// Runs the text of an `eval` call: `(vm, host, source, strict, direct)`.
pub type EvalHook = fn(&mut Vm, &mut dyn Host, &[u16], bool, bool) -> Result<Value, RunError>;

impl Vm {
    /// A new VM with its own heap. Fails when the limits don't make sense
    /// (`Internal`, with the reason) or are too small to set up the realm
    /// (`Abort`).
    pub fn new(limits: Limits) -> Result<Vm, RunError> {
        let mut heap =
            Heap::new(limits).map_err(|m| RunError::internal(format!("bad limits: {m}")))?;
        let intr = Self::make_intrinsics(&mut heap).map_err(|e| match error::lower(e) {
            error::Lowered::Run(r) => r,
            error::Lowered::Range(m) => RunError::internal(m),
        })?;
        Ok(Vm {
            heap,
            codes: Vec::new(),
            frames: Vec::new(),
            regs: Vec::new(),
            natives: Vec::new(),
            native_roots: Vec::new(),
            native_depth: 0,
            steps: 0,
            total_steps: 0,
            ic_enabled: true,
            intr,
            root_buf: Vec::new(),
            eval_fn: None,
            eval_hook: None,
        })
    }

    fn make_intrinsics(heap: &mut Heap) -> Result<Intrinsics, HeapError> {
        let object_prototype = heap.new_object(None)?;
        let function_prototype = heap.new_object(Some(object_prototype))?;
        let array_prototype = heap.new_object(Some(object_prototype))?;
        let string_prototype = heap.new_object(Some(object_prototype))?;
        let number_prototype = heap.new_object(Some(object_prototype))?;
        let boolean_prototype = heap.new_object(Some(object_prototype))?;
        let symbol_prototype = heap.new_object(Some(object_prototype))?;
        let global = heap.new_object(Some(object_prototype))?;
        let key_length = heap.key_from_str("length")?;
        let key_prototype = heap.key_from_str("prototype")?;
        let key_message = heap.key_from_str("message")?;
        let key_name = heap.key_from_str("name")?;
        let hidden = Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE);
        let base = heap.new_object(Some(object_prototype))?;
        let mut error_prototypes = [base; 5];
        for kind in ErrorKind::ALL {
            let p = if kind == ErrorKind::Error {
                base
            } else {
                heap.new_object(Some(base))?
            };
            let name = heap.alloc_string(kind.name())?;
            heap.define_own(
                p,
                key_name,
                Property {
                    attrs: hidden,
                    slot: Slot::Data(Value::string(name)),
                },
            )?;
            let empty = heap.alloc_string("")?;
            heap.define_own(
                p,
                key_message,
                Property {
                    attrs: hidden,
                    slot: Slot::Data(Value::string(empty)),
                },
            )?;
            error_prototypes[kind.index()] = p;
        }
        let mut typeof_strings = Vec::with_capacity(8);
        for n in TYPEOF_NAMES {
            typeof_strings.push(heap.alloc_string(n)?);
        }
        let typeof_strings: [StringRef; 8] = typeof_strings
            .try_into()
            .map_err(|_| HeapError::Internal("typeof table".to_string()))?;
        let lexicals = heap.new_object(None)?;
        let key_eval = heap.key_from_str("eval")?;
        let key_callee = heap.key_from_str("callee")?;
        let key_constructor = heap.key_from_str("constructor")?;
        Ok(Intrinsics {
            lexicals,
            key_eval,
            key_callee,
            key_constructor,
            global,
            object_prototype,
            function_prototype,
            array_prototype,
            string_prototype,
            number_prototype,
            boolean_prototype,
            symbol_prototype,
            error_prototypes,
            typeof_strings,
            key_length,
            key_prototype,
            key_message,
            key_name,
        })
    }

    // ---- access -----------------------------------------------------------

    /// The heap.
    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// The heap, for building values from Rust. Don't run a collection
    /// while you hold values that aren't rooted.
    pub fn heap_mut(&mut self) -> &mut Heap {
        &mut self.heap
    }

    /// The limits in force.
    pub fn limits(&self) -> &Limits {
        self.heap.limits()
    }

    /// The global object.
    pub fn global(&self) -> ObjectRef {
        self.intr.global
    }

    /// `Object.prototype`.
    pub fn object_prototype(&self) -> ObjectRef {
        self.intr.object_prototype
    }

    /// `Function.prototype`.
    pub fn function_prototype(&self) -> ObjectRef {
        self.intr.function_prototype
    }

    /// `Array.prototype`.
    pub fn array_prototype(&self) -> ObjectRef {
        self.intr.array_prototype
    }

    /// The prototype of one of the built-in error types.
    pub fn error_prototype(&self, kind: ErrorKind) -> ObjectRef {
        self.intr.error_prototypes[kind.index()]
    }

    /// Frames on the call stack (0 between runs).
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Steps in the current (or last) run.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Steps over the life of the VM.
    pub fn total_steps(&self) -> u64 {
        self.total_steps
    }

    /// Turn inline caches on or off. Off, every access takes the generic
    /// path and nothing is recorded; results must be the same either way.
    pub fn set_ic_enabled(&mut self, on: bool) {
        self.ic_enabled = on;
    }

    /// True when inline caches are on.
    pub fn ic_enabled(&self) -> bool {
        self.ic_enabled
    }

    /// One cache site of a loaded block.
    pub fn ic_site(&self, code: CodeId, site: u32) -> Option<&IcSite> {
        self.codes.get(code.0 as usize)?.ic.get(site as usize)
    }

    /// Cache counters over all loaded code.
    pub fn ic_stats(&self) -> IcStats {
        let mut s = IcStats::default();
        for c in &self.codes {
            for site in &c.ic {
                s.hits += site.hits;
                s.misses += site.misses;
                match site.state() {
                    IcState::Empty => s.empty += 1,
                    IcState::Mono => s.mono += 1,
                    IcState::Poly => s.poly += 1,
                    IcState::Mega => s.mega += 1,
                }
            }
        }
        s
    }

    /// The loaded code for `id` (nested functions are their own ids; see
    /// [`Vm::children`]).
    pub fn code(&self, id: CodeId) -> Option<&CodeBlock> {
        self.codes.get(id.0 as usize).map(|c| &c.block)
    }

    /// Ids of the nested functions of `id`, in table order.
    pub fn children(&self, id: CodeId) -> Vec<CodeId> {
        self.codes
            .get(id.0 as usize)
            .map(|c| c.children.iter().map(|i| CodeId(*i)).collect())
            .unwrap_or_default()
    }

    /// Trip counts of each `LoopHeader` in `id`.
    pub fn loop_trips(&self, id: CodeId) -> Vec<u64> {
        self.codes
            .get(id.0 as usize)
            .map(|c| c.loop_trips.clone())
            .unwrap_or_default()
    }

    // ---- loading ----------------------------------------------------------

    /// Check and load a code block and all its nested functions. Nothing
    /// is loaded if any of them is refused.
    pub fn load(&mut self, block: CodeBlock) -> Result<CodeId, RunError> {
        let first = self.codes.len();
        let result = self.load_all(block);
        if result.is_err() {
            self.codes.truncate(first);
        }
        result.map(|_| CodeId(first as u32))
    }

    fn load_all(&mut self, block: CodeBlock) -> Result<(), RunError> {
        let max_regs = self.heap.limits().max_registers_per_function;
        let mut pending: Vec<(CodeBlock, Option<(usize, usize)>)> = vec![(block, None)];
        while let Some((mut b, parent)) = pending.pop() {
            if b.register_count > max_regs {
                return Err(RunError::Compile {
                    message: format!(
                        "function '{}' needs {} registers, limit {}",
                        b.display_name(),
                        group_digits(u64::from(b.register_count)),
                        group_digits(u64::from(max_regs))
                    ),
                    span: b.span,
                });
            }
            b.validate().map_err(|message| RunError::Compile {
                message,
                span: b.span,
            })?;
            if self.codes.len() >= u32::MAX as usize {
                return Err(RunError::internal("too many loaded functions"));
            }
            let mut keys = Vec::with_capacity(b.keys.len());
            for k in &b.keys {
                keys.push(self.heap.key_from_units(k).map_err(heap_to_run)?);
            }
            let mut strings = Vec::with_capacity(b.strings.len());
            for s in &b.strings {
                strings.push(
                    self.heap
                        .alloc_string_units(s.clone())
                        .map_err(heap_to_run)?,
                );
            }
            let children = std::mem::take(&mut b.functions);
            let id = self.codes.len();
            self.codes.push(LoadedCode {
                ic: vec![IcSite::default(); b.ic_sites as usize],
                loop_trips: vec![0; b.loops as usize],
                children: vec![0; children.len()],
                block: b,
                keys,
                strings,
            });
            if let Some((p, i)) = parent {
                self.codes[p].children[i] = id as u32;
            }
            for (i, c) in children.into_iter().enumerate().rev() {
                pending.push((c, Some((id, i))));
            }
        }
        Ok(())
    }

    // ---- running ----------------------------------------------------------

    fn begin_entry(&mut self) {
        if self.frames.is_empty() && self.native_depth == 0 {
            self.steps = 0;
        }
    }

    /// Run top-level code, with `this` set to the global object.
    pub fn run(&mut self, host: &mut dyn Host, code: CodeId) -> Result<Value, RunError> {
        if code.0 as usize >= self.codes.len() {
            return Err(RunError::internal(format!("no loaded code {}", code.0)));
        }
        self.begin_entry();
        let base = self.frames.len();
        let this = Value::object(self.intr.global);
        self.push_frame(
            code.0,
            Value::UNDEFINED,
            None,
            this,
            &[],
            frame::DISCARD,
            frame::FrameKind::Normal,
        )
        .map_err(|e| self.exit_to_run(e))?;
        self.execute(host, base)
    }

    /// Call a function from Rust.
    pub fn call(
        &mut self,
        host: &mut dyn Host,
        f: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, RunError> {
        self.begin_entry();
        self.call_value(host, f, this, args)
    }

    /// Run a job the host got from [`Host::enqueue_job`]. Its roots are
    /// released first.
    pub fn run_job(&mut self, host: &mut dyn Host, job: Job) -> Result<Value, RunError> {
        let f = self.heap.persistent(job.function).map_err(heap_to_run)?;
        let this = self.heap.persistent(job.this).map_err(heap_to_run)?;
        let mut args = Vec::with_capacity(job.args.len());
        for a in &job.args {
            args.push(self.heap.persistent(*a).map_err(heap_to_run)?);
        }
        let mark = self.native_roots.len();
        self.native_roots.push(f);
        self.native_roots.push(this);
        self.native_roots.extend_from_slice(&args);
        let mut released = Ok(());
        for p in std::iter::once(job.function)
            .chain(std::iter::once(job.this))
            .chain(job.args)
        {
            if let Err(e) = self.heap.release(p) {
                released = Err(heap_to_run(e));
            }
        }
        let r = released.and_then(|_| self.call(host, f, this, &args));
        self.native_roots.truncate(mark);
        r
    }

    pub(crate) fn make_job(
        &mut self,
        f: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Job, RunError> {
        let function = self.heap.persist(f).map_err(heap_to_run)?;
        let this = self.heap.persist(this).map_err(heap_to_run)?;
        let mut held = Vec::with_capacity(args.len());
        for a in args {
            held.push(self.heap.persist(*a).map_err(heap_to_run)?);
        }
        Ok(Job {
            function,
            this,
            args: held,
        })
    }

    /// Make a job from Rust (the host's own timers, say).
    pub fn job(&mut self, f: Value, this: Value, args: &[Value]) -> Result<Job, RunError> {
        self.make_job(f, this, args)
    }

    /// Drain all queued microtask jobs from `host` via [`Host::take_job`].
    pub fn drain_jobs(&mut self, host: &mut dyn Host) -> Result<usize, RunError> {
        let mut ran = 0usize;
        while let Some(job) = host.take_job() {
            self.run_job(host, job)?;
            ran += 1;
        }
        Ok(ran)
    }

    #[doc(hidden)]
    pub fn job_legacy(&mut self, f: Value, this: Value, args: &[Value]) -> Result<Job, RunError> {
        self.make_job(f, this, args)
    }

    /// Register a native function and return the function object.
    pub fn register_native(&mut self, name: &str, f: NativeFn) -> Result<Value, RunError> {
        let id = self.natives.len() as u32;
        let obj = self
            .heap
            .alloc_object(
                Some(self.intr.function_prototype),
                ObjectClass::Native { id },
            )
            .map_err(heap_to_run)?;
        self.natives.push(NativeEntry {
            name: name.to_string(),
            f,
        });
        Ok(Value::object(obj))
    }

    /// Set the function that compiles and runs `eval` code.
    pub fn set_eval_hook(&mut self, hook: EvalHook) {
        self.eval_hook = Some(hook);
    }

    /// Define a global binding (writable, enumerable, configurable).
    pub fn define_global(&mut self, name: &str, value: Value) -> Result<(), RunError> {
        let key = self.heap.key_from_str(name).map_err(heap_to_run)?;
        let ok = self
            .heap
            .define_own(self.intr.global, key, Property::data(value))
            .map_err(heap_to_run)?;
        if ok {
            Ok(())
        } else {
            Err(self.make_thrown(ErrorKind::Type, &format!("Cannot redefine global '{name}'")))
        }
    }

    /// A global's value, if it is a data property.
    pub fn get_global(&self, name: &str) -> Option<Value> {
        let key = match self
            .heap
            .atoms()
            .find(&name.encode_utf16().collect::<Vec<_>>())
        {
            Some(a) => PropertyKey::Atom(a),
            None => PropertyKey::Index(crate::cinder::value::atom::array_index(
                &name.encode_utf16().collect::<Vec<_>>(),
            )?),
        };
        match self.heap.get_own(self.intr.global, key).ok()?? {
            Property {
                slot: Slot::Data(v),
                ..
            } => Some(v),
            _ => None,
        }
    }

    /// A string value.
    pub fn string(&mut self, s: &str) -> Result<Value, RunError> {
        Ok(Value::string(
            self.heap.alloc_string(s).map_err(heap_to_run)?,
        ))
    }

    /// Run a collection now. Only call this between runs or from a native
    /// function that kept its values with [`NativeCx::keep`].
    pub fn collect_garbage(&mut self, host: &mut dyn Host) {
        let roots = self.gather_roots(host);
        let stats = self.heap.collect(&roots);
        self.root_buf = roots;
        for d in stats.finalized {
            host.finalized(d);
        }
    }

    pub(crate) fn gather_roots(&mut self, host: &mut dyn Host) -> Vec<Value> {
        let mut roots = std::mem::take(&mut self.root_buf);
        roots.clear();
        roots.extend_from_slice(&self.regs);
        for f in &self.frames {
            roots.push(f.this);
            roots.push(f.callee);
            if let Some(e) = f.env {
                roots.push(Value::object(e));
            }
        }
        let i = &self.intr;
        for o in [
            i.global,
            i.object_prototype,
            i.function_prototype,
            i.array_prototype,
            i.string_prototype,
            i.number_prototype,
            i.boolean_prototype,
            i.symbol_prototype,
            i.lexicals,
        ]
        .iter()
        .chain(i.error_prototypes.iter())
        {
            roots.push(Value::object(*o));
        }
        roots.extend(i.typeof_strings.iter().map(|s| Value::string(*s)));
        for c in &self.codes {
            roots.extend(c.strings.iter().map(|s| Value::string(*s)));
        }
        roots.extend_from_slice(&self.native_roots);
        roots.extend(self.eval_fn);
        host.trace(&mut roots);
        roots
    }

    // ---- errors -----------------------------------------------------------

    /// Make an error object of `kind` with `message`.
    pub fn make_error(&mut self, kind: ErrorKind, message: &str) -> Result<Value, RunError> {
        self.make_error_value(kind, message)
            .map_err(|e| self.exit_to_run(e))
    }

    pub(crate) fn make_error_value(
        &mut self,
        kind: ErrorKind,
        message: &str,
    ) -> Result<Value, Exit> {
        let proto = self.intr.error_prototypes[kind.index()];
        let obj = self.heap.alloc_object(Some(proto), ObjectClass::Error)?;
        let msg = self.heap.alloc_string(message)?;
        self.heap.define_own(
            obj,
            self.intr.key_message,
            Property {
                attrs: Attrs(Attrs::WRITABLE | Attrs::CONFIGURABLE),
                slot: Slot::Data(Value::string(msg)),
            },
        )?;
        Ok(Value::object(obj))
    }

    /// A thrown error, ready to return from a native function.
    pub(crate) fn make_thrown(&mut self, kind: ErrorKind, message: &str) -> RunError {
        match self.make_error_value(kind, message) {
            Ok(value) => RunError::Thrown {
                value,
                message: format!("{}: {message}", kind.name()),
                stack: Vec::new(),
            },
            Err(e) => self.exit_to_run(e),
        }
    }

    pub(crate) fn exit_to_run(&mut self, e: Exit) -> RunError {
        match e {
            Exit::Throw { value, stack } => {
                let message = self
                    .to_display_string(value)
                    .unwrap_or_else(|_| "<unprintable value>".to_string());
                RunError::Thrown {
                    value,
                    message,
                    stack: stack.unwrap_or_else(|| self.stack_lines()),
                }
            }
            Exit::Pending(kind, m) => {
                let r = self.make_thrown(kind, &m);
                r.with_stack(self.stack_lines())
            }
            Exit::Fatal(r) => r.with_stack(self.stack_lines()),
        }
    }

    pub(crate) fn stack_lines(&self) -> Vec<StackLine> {
        let max = self.heap.limits().max_stack_lines as usize;
        let mut lines: Vec<StackLine> = self
            .frames
            .iter()
            .rev()
            .take(max)
            .map(|f| {
                let code = &self.codes[f.code as usize].block;
                let span = code.span_at(f.pc.saturating_sub(1));
                StackLine {
                    function: code.display_name().to_string(),
                    line: span.map(|s| s.line).unwrap_or(0),
                    column: span.map(|s| s.column).unwrap_or(0),
                }
            })
            .collect();
        if self.frames.len() > max {
            lines.push(StackLine::omitted(self.frames.len() - max));
        }
        lines
    }

    // ---- conversions ------------------------------------------------------

    /// Text for a value, without calling script: what an uncaught error
    /// message shows.
    pub fn to_display_string(&mut self, v: Value) -> Result<String, RunError> {
        match self.display_units(v) {
            Ok(u) => Ok(String::from_utf16_lossy(&u)),
            Err(e) => Err(self.exit_to_run(e)),
        }
    }

    pub(crate) fn display_units(&mut self, v: Value) -> Result<Vec<u16>, Exit> {
        if let Some(s) = v.as_symbol() {
            let d = match self.heap.symbol_description(s)? {
                Some(d) => self.heap.string_to_rust(d)?,
                None => String::new(),
            };
            return Ok(format!("Symbol({d})").encode_utf16().collect());
        }
        let p = self.conv_primitive(v)?;
        if let Some(s) = p.as_symbol() {
            return self.display_units(Value::symbol(s));
        }
        self.primitive_units(p)
    }

    /// Units of a primitive (symbols give a TypeError, as in `"" + sym`).
    pub(crate) fn primitive_units(&mut self, p: Value) -> Result<Vec<u16>, Exit> {
        let text = match p.kind() {
            Kind::String => {
                let s = p
                    .as_string()
                    .ok_or_else(|| internal_err("string without a handle"))?;
                return Ok(self.heap.string_units(s)?.to_vec());
            }
            Kind::Number | Kind::Int32 => number_to_string(p.as_number().unwrap_or(f64::NAN)),
            Kind::Bool => (if p.as_bool() == Some(true) {
                "true"
            } else {
                "false"
            })
            .to_string(),
            Kind::Null => "null".to_string(),
            Kind::Undefined | Kind::Uninit | Kind::Hole => "undefined".to_string(),
            Kind::Symbol => return Err(type_err("Cannot convert a Symbol value to a string")),
            Kind::BigInt => p.as_bigint().unwrap_or(0).to_string(),
            Kind::Object => return Err(internal_err("primitive_units got an object")),
        };
        Ok(text.encode_utf16().collect())
    }

    /// ToPrimitive without calling script (see the module docs).
    pub(crate) fn conv_primitive(&mut self, v: Value) -> Result<Value, Exit> {
        let Some(obj) = v.as_object() else {
            return Ok(v);
        };
        let units = match &self.heap.object(obj)?.class {
            ObjectClass::Primitive(p) => return Ok(*p),
            ObjectClass::Array { .. } => self.join_units(obj)?,
            ObjectClass::Error => self.error_units(obj)?,
            ObjectClass::Function { code, .. } => {
                let name = self
                    .codes
                    .get(*code as usize)
                    .map(|c| c.block.name.clone())
                    .unwrap_or_default();
                format!("function {name}() {{ [code] }}")
                    .encode_utf16()
                    .collect()
            }
            ObjectClass::Native { id } => {
                let name = self
                    .natives
                    .get(*id as usize)
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                format!("function {name}() {{ [native code] }}")
                    .encode_utf16()
                    .collect()
            }
            ObjectClass::Bound(b) => format!("function {}() {{ [native code] }}", b.name)
                .encode_utf16()
                .collect(),
            _ => "[object Object]".encode_utf16().collect(),
        };
        Ok(Value::string(self.heap.alloc_string_units(units)?))
    }

    fn data_along_chain(&self, obj: ObjectRef, key: PropertyKey) -> Option<Value> {
        match self.heap.lookup(obj, key).ok()? {
            crate::cinder::gc::Lookup::Found {
                prop:
                    Property {
                        slot: Slot::Data(v),
                        ..
                    },
                ..
            } => Some(v),
            _ => None,
        }
    }

    fn error_units(&mut self, obj: ObjectRef) -> Result<Vec<u16>, Exit> {
        let name = self.data_along_chain(obj, self.intr.key_name);
        let msg = self.data_along_chain(obj, self.intr.key_message);
        let mut out: Vec<u16> = match name.and_then(|n| n.as_string()) {
            Some(s) => self.heap.string_units(s)?.to_vec(),
            None => "Error".encode_utf16().collect(),
        };
        if let Some(m) = msg.and_then(|m| m.as_string()) {
            let m = self.heap.string_units(m)?.to_vec();
            if !m.is_empty() {
                out.extend(": ".encode_utf16());
                out.extend(m);
            }
        }
        Ok(out)
    }

    /// Join an array with commas, the way `String(array)` does, without
    /// using the Rust stack for nested arrays. A cycle gives an empty
    /// string at the repeat, nesting past `max_join_depth` and results past
    /// `max_string_length` give a `RangeError`.
    pub(crate) fn join_units(&mut self, arr: ObjectRef) -> Result<Vec<u16>, Exit> {
        let max_depth = self.heap.limits().max_join_depth as usize;
        let max_len = self.heap.limits().max_string_length;
        let too_long = |n: usize| {
            range_err(format!(
                "Invalid string length: {} code units, limit {}",
                group_digits(n as u64),
                group_digits(max_len as u64)
            ))
        };
        let mut out: Vec<u16> = Vec::new();
        let len = self.heap.array_length(arr)?;
        let mut stack: Vec<(ObjectRef, u32, u32)> = vec![(arr, 0, len)];
        let mut active: HashSet<ObjectRef> = HashSet::new();
        active.insert(arr);
        while let Some(top) = stack.last_mut() {
            let (a, i, len) = *top;
            if i >= len {
                stack.pop();
                active.remove(&a);
                continue;
            }
            if i == 0 && out.len().saturating_add(len as usize - 1) > max_len {
                return Err(too_long(out.len().saturating_add(len as usize - 1)));
            }
            top.1 += 1;
            if i > 0 {
                out.push(u16::from(b','));
            }
            let el = match self.heap.get_own(a, PropertyKey::Index(i))? {
                Some(Property {
                    slot: Slot::Data(v),
                    ..
                }) => v,
                _ => Value::UNDEFINED,
            };
            if el.is_nullish() || el.is_hole() || el.is_uninit() {
                continue;
            }
            if let Some(o) = el.as_object()
                && self.heap.object(o)?.is_array()
            {
                if active.contains(&o) {
                    continue;
                }
                if stack.len() >= max_depth {
                    return Err(range_err(format!(
                        "Array nesting too deep to join: more than {} levels (max_join_depth)",
                        group_digits(max_depth as u64)
                    )));
                }
                let l = self.heap.array_length(o)?;
                active.insert(o);
                stack.push((o, 0, l));
                continue;
            }
            let units = self.display_units_for_join(el)?;
            out.extend(units);
            if out.len() > max_len {
                return Err(too_long(out.len()));
            }
        }
        Ok(out)
    }

    fn display_units_for_join(&mut self, el: Value) -> Result<Vec<u16>, Exit> {
        if el.is_object() {
            let p = self.conv_primitive(el)?;
            self.primitive_units(p)
        } else {
            self.primitive_units(el)
        }
    }

    /// ToNumber.
    pub(crate) fn conv_number(&mut self, v: Value) -> Result<f64, Exit> {
        if let Some(n) = v.as_number() {
            return Ok(n);
        }
        let p = self.conv_primitive(v)?;
        Ok(match p.kind() {
            Kind::Number | Kind::Int32 => p.as_number().unwrap_or(f64::NAN),
            Kind::Bool => {
                if p.as_bool() == Some(true) {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Null => 0.0,
            Kind::Undefined | Kind::Uninit | Kind::Hole => f64::NAN,
            Kind::String => {
                let s = p
                    .as_string()
                    .ok_or_else(|| internal_err("string without a handle"))?;
                string_to_number(self.heap.string_units(s)?)
            }
            Kind::Symbol => return Err(type_err("Cannot convert a Symbol value to a number")),
            Kind::BigInt => p.as_bigint().unwrap_or(0) as f64,
            Kind::Object => return Err(internal_err("to_primitive returned an object")),
        })
    }

    pub(crate) fn conv_int32(&mut self, v: Value) -> Result<i32, Exit> {
        if let Some(i) = v.as_int32() {
            return Ok(i);
        }
        Ok(to_int32(self.conv_number(v)?))
    }

    /// ToString, as a heap string.
    pub(crate) fn conv_string(&mut self, v: Value) -> Result<StringRef, Exit> {
        if let Some(s) = v.as_string() {
            return Ok(s);
        }
        let p = self.conv_primitive(v)?;
        if let Some(s) = p.as_string() {
            return Ok(s);
        }
        let u = self.primitive_units(p)?;
        Ok(self.heap.alloc_string_units(u)?)
    }

    /// ToBoolean.
    pub(crate) fn truthy(&self, v: Value) -> Result<bool, Exit> {
        Ok(match v.kind() {
            Kind::Bool => v.as_bool() == Some(true),
            Kind::Int32 => v.as_int32() != Some(0),
            Kind::Number => {
                let n = v.as_number().unwrap_or(f64::NAN);
                !(n == 0.0 || n.is_nan())
            }
            Kind::Null | Kind::Undefined | Kind::Uninit | Kind::Hole => false,
            Kind::String => {
                let s = v
                    .as_string()
                    .ok_or_else(|| internal_err("string without a handle"))?;
                self.heap.string_len(s)? > 0
            }
            Kind::BigInt => v.as_bigint() != Some(0),
            Kind::Object | Kind::Symbol => true,
        })
    }

    pub(crate) fn strings_equal(&mut self, a: StringRef, b: StringRef) -> Result<bool, Exit> {
        if a == b {
            return Ok(true);
        }
        if self.heap.string_len(a)? != self.heap.string_len(b)? {
            return Ok(false);
        }
        let ua = self.heap.string_units(a)?.to_vec();
        Ok(self.heap.string_units(b)? == ua.as_slice())
    }

    pub(crate) fn compare_strings(
        &mut self,
        a: StringRef,
        b: StringRef,
    ) -> Result<std::cmp::Ordering, Exit> {
        let ua = self.heap.string_units(a)?.to_vec();
        Ok(compare_units(&ua, self.heap.string_units(b)?))
    }

    /// `===`
    pub(crate) fn strict_eq(&mut self, a: Value, b: Value) -> Result<bool, Exit> {
        if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
            return Ok(x == y);
        }
        if let (Some(x), Some(y)) = (a.as_string(), b.as_string()) {
            return self.strings_equal(x, y);
        }
        Ok(a.same_bits(b))
    }

    /// `==`
    pub(crate) fn loose_eq(&mut self, a: Value, b: Value) -> Result<bool, Exit> {
        let (mut a, mut b) = (a, b);
        if let (Some(x), Some(y)) = (a.as_bigint(), b.as_number()) {
            return Ok(y.is_finite() && y.fract() == 0.0 && (x as f64) == y);
        }
        if let (Some(x), Some(y)) = (a.as_number(), b.as_bigint()) {
            return Ok(x.is_finite() && x.fract() == 0.0 && x == (y as f64));
        }
        for _ in 0..4 {
            let same_type = a.is_number() && b.is_number() || a.kind() == b.kind();
            if same_type {
                return self.strict_eq(a, b);
            }
            if a.is_nullish() && b.is_nullish() {
                return Ok(true);
            }
            if a.is_nullish() || b.is_nullish() {
                return Ok(false);
            }
            if a.is_number() && b.is_string() || b.is_bool() {
                b = Value::number(self.conv_number(b)?);
            } else if a.is_string() && b.is_number() || a.is_bool() {
                a = Value::number(self.conv_number(a)?);
            } else if a.is_object() && !b.is_object() {
                a = self.conv_primitive(a)?;
            } else if b.is_object() && !a.is_object() {
                b = self.conv_primitive(b)?;
            } else {
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// `a < b`: `None` when either side is NaN.
    pub(crate) fn less_than(&mut self, a: Value, b: Value) -> Result<Option<bool>, Exit> {
        if let (Some(x), Some(y)) = (a.as_bigint(), b.as_bigint()) {
            return Ok(Some(x < y));
        }
        if let (Some(x), Some(y)) = (a.as_int32(), b.as_int32()) {
            return Ok(Some(x < y));
        }
        let pa = self.conv_primitive(a)?;
        let pb = self.conv_primitive(b)?;
        if let (Some(x), Some(y)) = (pa.as_string(), pb.as_string()) {
            return Ok(Some(
                self.compare_strings(x, y)? == std::cmp::Ordering::Less,
            ));
        }
        let x = self.conv_number(pa)?;
        let y = self.conv_number(pb)?;
        if x.is_nan() || y.is_nan() {
            Ok(None)
        } else {
            Ok(Some(x < y))
        }
    }

    /// ToPropertyKey.
    pub(crate) fn conv_key(&mut self, v: Value) -> Result<PropertyKey, Exit> {
        if let Some(i) = v.as_int32()
            && i >= 0
        {
            return Ok(PropertyKey::Index(i as u32));
        }
        if let Some(n) = v.as_number() {
            if (0.0..4_294_967_295.0).contains(&n) && n.fract() == 0.0 {
                return Ok(PropertyKey::Index(n as u32));
            }
            return Ok(self.heap.key_from_str(&number_to_string(n))?);
        }
        if let Some(s) = v.as_symbol() {
            return Ok(PropertyKey::Symbol(s));
        }
        let s = self.conv_string(v)?;
        Ok(self.heap.key_from_string(s)?)
    }

    /// A key as text, for messages.
    pub(crate) fn key_text(&self, key: PropertyKey) -> String {
        match key {
            PropertyKey::Index(i) => i.to_string(),
            PropertyKey::Atom(a) => self
                .heap
                .atoms()
                .to_rust_string(a)
                .unwrap_or_else(|| "?".to_string()),
            PropertyKey::Symbol(s) => {
                let d = self
                    .heap
                    .symbol_description(s)
                    .ok()
                    .flatten()
                    .and_then(|d| self.heap.string_to_rust(d).ok())
                    .unwrap_or_default();
                format!("Symbol({d})")
            }
        }
    }

    /// `typeof v`
    pub(crate) fn type_of(&self, v: Value) -> Result<StringRef, Exit> {
        let name = match v.typeof_primitive() {
            Some(n) => n,
            None => {
                let o = v
                    .as_object()
                    .ok_or_else(|| internal_err("typeof of an unknown kind"))?;
                if self.is_callable_obj(o)? {
                    "function"
                } else {
                    "object"
                }
            }
        };
        let i = TYPEOF_NAMES.iter().position(|n| *n == name).unwrap_or(0);
        Ok(self.intr.typeof_strings[i])
    }
}

pub(crate) fn heap_to_run(e: HeapError) -> RunError {
    match error::lower(e) {
        error::Lowered::Run(r) => r,
        error::Lowered::Range(m) => RunError::Thrown {
            value: Value::UNDEFINED,
            message: format!("RangeError: {m}"),
            stack: Vec::new(),
        },
    }
}

/// Used by tests and messages: the abort for a spent step budget.
pub(crate) fn steps_abort(max: u64) -> RunError {
    RunError::Abort {
        kind: LimitKind::Steps,
        reason: format!(
            "step budget used up: {} steps in this run (max_steps)",
            group_digits(max)
        ),
        limit: max,
        observed: max,
        stack: Vec::new(),
    }
}
