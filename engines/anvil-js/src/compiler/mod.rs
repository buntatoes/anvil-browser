// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The compiler: syntax tree to register bytecode.
//!
//! Two passes. [`scope`] walks the tree and works out every scope's names
//! and which of them inner functions capture. Code generation ([`expr`],
//! [`stmt`]) then walks it again and emits ops: captured names live in
//! environment slots, the rest in registers handed out by [`regalloc`],
//! and the program's own top-level names go to the global object (`var`,
//! functions) or the global lexical table (`let`, `const`).
//!
//! Strictness is per function, from the parser's directives. Sloppy code
//! gets the sloppy rules: `this` coercion, implicit globals, a mapped
//! `arguments` object, Annex B block functions, quiet failed writes.
//! `with` is refused, and direct `eval` runs only where no local binding is
//! visible; both say so in their error.
//!
//! Errors are [`SyntaxError`]s with a span. Nothing here panics on any
//! input: recursion is counted against `max_compile_depth` and runs on a
//! thread with a stack sized for that depth.

pub mod expr;
pub mod lower;
pub mod regalloc;
pub mod scope;
pub mod stmt;

use crate::anvil_js::ast::{Pat, PatKind, Program, SourceType, Stmt};
use crate::anvil_js::bytecode::{CodeBlock, Op, Reg};
use crate::anvil_js::error::{Span, SyntaxError};
use crate::anvil_js::limits::{Limits, group_digits};
use crate::anvil_js::parser::{DEFAULT_MAX_DEPTH, ParseOptions, parse_module, parse_script};
use crate::anvil_js::value::Value;
use crate::anvil_js::vm::{ErrorKind, Host, RunError, Vm};
use regalloc::RegAlloc;
use scope::{Analysis, BindKind, FuncId, ScopeId, Tag, TopMode};
use std::collections::HashMap;

/// A compile error: a message and where.
pub type CompileError = SyntaxError;

type R<T> = Result<T, CompileError>;

/// Stack bytes the compiler thread gets per level of `max_compile_depth`.
/// Measured: one level of the deepest recursion (a nested call argument,
/// or a statement inside a block) uses under a fifth of this in a debug
/// build.
#[cfg(debug_assertions)]
const STACK_PER_LEVEL: usize = 48 * 1024;
#[cfg(not(debug_assertions))]
const STACK_PER_LEVEL: usize = 12 * 1024;

/// Fixed part of the compiler thread's stack.
const STACK_BASE: usize = 1 << 20;

/// Counts recursion against `max_compile_depth`.
#[derive(Debug)]
pub struct Depth {
    cur: u32,
    max: u32,
    peak: u32,
    /// Lowest stack address seen in `enter`, for measuring stack use.
    #[cfg(test)]
    pub(crate) stack_low: usize,
}

impl Depth {
    /// A counter that allows `max` levels.
    pub fn new(max: u32) -> Depth {
        Depth {
            cur: 0,
            max,
            peak: 0,
            #[cfg(test)]
            stack_low: usize::MAX,
        }
    }

    /// One level deeper; an error past the limit.
    pub fn enter(&mut self, span: Span) -> R<()> {
        if self.cur >= self.max {
            return Err(SyntaxError::new(
                format!(
                    "nesting too deep to compile: more than {} levels (max_compile_depth)",
                    group_digits(u64::from(self.max))
                ),
                span,
            ));
        }
        self.cur += 1;
        self.peak = self.peak.max(self.cur);
        #[cfg(test)]
        {
            let probe = 0u8;
            self.stack_low = self.stack_low.min(std::ptr::addr_of!(probe) as usize);
        }
        Ok(())
    }

    /// The deepest level reached so far.
    pub fn peak(&self) -> u32 {
        self.peak
    }

    /// One level back up.
    pub fn leave(&mut self) {
        self.cur = self.cur.saturating_sub(1);
    }
}

/// Compile a classic script.
pub fn compile_script(src: &str, limits: &Limits) -> Result<CodeBlock, SyntaxError> {
    check_source(src, limits)?;
    let opts = parse_options(limits);
    let program = match parse_script(src, &opts) {
        Ok(p) => p,
        Err(e) => {
            if src.contains("import") || src.contains("export") {
                parse_module(src, &opts).map_err(|_| e)?
            } else {
                return Err(e);
            }
        }
    };
    compile_program(&program, limits)
}

/// Compile an already-parsed program. Modules aren't supported yet.
pub fn compile_program(program: &Program, limits: &Limits) -> Result<CodeBlock, SyntaxError> {
    if false && program.source_type == SourceType::Module {
        return Err(SyntaxError::new(
            "modules are not supported yet",
            program.span,
        ));
    }
    compile_top(program, limits, TopMode::Script, program.strict, "script")
}

/// Compile eval code. `strict` is inherited from the calling code (direct
/// eval in strict code); a `"use strict"` directive in the code also makes
/// it strict.
pub fn compile_eval(src: &str, limits: &Limits, strict: bool) -> Result<CodeBlock, SyntaxError> {
    check_source(src, limits)?;
    let opts = parse_options(limits);
    let program = match parse_script(src, &opts) {
        Ok(p) => p,
        Err(e) => {
            if src.contains("import") || src.contains("export") {
                parse_module(src, &opts).map_err(|_| e)?
            } else {
                return Err(e);
            }
        }
    };
    let strict = strict || program.strict;
    let mode = if strict {
        TopMode::StrictEval
    } else {
        TopMode::SloppyEval
    };
    compile_top(&program, limits, mode, strict, "eval")
}

/// Compile and run a script; its completion value comes back. Parse and
/// compile errors come back as `RunError::Compile`.
pub fn run_script(vm: &mut Vm, host: &mut dyn Host, src: &str) -> Result<Value, RunError> {
    let block = compile_script(src, vm.heap().limits()).map_err(|e| RunError::Compile {
        message: format!("SyntaxError: {}", e.message),
        span: e.span,
    })?;
    let id = vm.load(block)?;
    let out = vm.run(host, id)?;
    vm.drain_jobs(host)?;
    Ok(out)
}

/// Add the basic globals (see [`Vm::install_basics`]) and make `eval`
/// work.
pub fn install(vm: &mut Vm) -> Result<(), RunError> {
    vm.install_basics()?;
    vm.set_eval_hook(eval_hook);
    Ok(())
}

fn eval_hook(
    vm: &mut Vm,
    host: &mut dyn Host,
    units: &[u16],
    strict: bool,
    _direct: bool,
) -> Result<Value, RunError> {
    let src = String::from_utf16_lossy(units);
    let limits = vm.heap().limits().clone();
    match compile_eval(&src, &limits, strict) {
        Ok(block) => {
            let id = vm.load(block)?;
            vm.run(host, id)
        }
        Err(e) => Err(vm.make_thrown(ErrorKind::Syntax, &e.message)),
    }
}

fn parse_options(limits: &Limits) -> ParseOptions {
    ParseOptions {
        max_source_bytes: limits.max_source_bytes,
        max_depth: DEFAULT_MAX_DEPTH,
    }
}

fn check_source(src: &str, limits: &Limits) -> R<()> {
    if src.len() > limits.max_source_bytes {
        return Err(SyntaxError::new(
            format!(
                "script is {} bytes, limit {} bytes (max_source_bytes)",
                group_digits(src.len() as u64),
                group_digits(limits.max_source_bytes as u64)
            ),
            Span::default(),
        ));
    }
    Ok(())
}

fn compile_top(
    program: &Program,
    limits: &Limits,
    mode: TopMode,
    strict: bool,
    name: &str,
) -> Result<CodeBlock, SyntaxError> {
    let stack = STACK_BASE + STACK_PER_LEVEL * limits.max_compile_depth as usize;
    std::thread::scope(|s| {
        let h = std::thread::Builder::new()
            .name("anvil-compile".to_string())
            .stack_size(stack)
            .spawn_scoped(s, || {
                compile_on_this_thread(program, limits, mode, strict, name)
            });
        match h {
            Ok(h) => h.join().unwrap_or_else(|_| {
                Err(SyntaxError::new(
                    "internal compiler error: the compiler thread stopped",
                    program.span,
                ))
            }),
            Err(e) => Err(SyntaxError::new(
                format!("could not start the compiler thread: {e}"),
                program.span,
            )),
        }
    })
}

fn compile_on_this_thread(
    program: &Program,
    limits: &Limits,
    mode: TopMode,
    strict: bool,
    name: &str,
) -> Result<CodeBlock, SyntaxError> {
    let lowered;
    let prog = if lower::needs_lowering(program) {
        let mut ldepth = Depth::new(limits.max_compile_depth);
        lowered = lower::lower_program(program, &mut ldepth)?;
        &lowered
    } else {
        program
    };
    let mut depth = Depth::new(limits.max_compile_depth);
    let analysis = scope::analyze(&prog.body, prog.span, strict, mode, &mut depth)?;
    let mut cg = Cg {
        a: &analysis,
        depth: Depth::new(limits.max_compile_depth),
        fns: Vec::new(),
        scopes: Vec::new(),
    };
    cg.top(prog, name)
}

// ---- code generation state ------------------------------------------------

/// Where a binding lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// A register of its function.
    Reg(Reg),
    /// A slot of its scope's environment.
    Env(u32),
    /// The global object or global lexical table.
    Global,
}

/// A scope while code is generated for it.
#[derive(Debug)]
pub(crate) struct Cs {
    pub id: ScopeId,
    pub env: bool,
    pub stores: Vec<Store>,
    /// Register mark to release to when the scope ends.
    pub mark: u32,
}

/// Where a `break`, `continue` or `return` goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    Break(usize),
    Continue(usize),
    Return,
}

#[derive(Debug)]
pub(crate) enum CtlKind {
    Loop,
    Switch,
    Block,
    Finally {
        kind_reg: Reg,
        exits: Vec<Exit>,
        jumps: Vec<u32>,
    },
}

/// A statement `break` and friends can leave.
#[derive(Debug)]
pub(crate) struct Control {
    pub labels: Vec<String>,
    pub kind: CtlKind,
    pub breaks: Vec<u32>,
    pub continues: Vec<u32>,
    pub break_depth: u16,
    pub cont_depth: u16,
}

/// One function being generated.
#[derive(Debug)]
pub(crate) struct Fs {
    pub b: CodeBlock,
    pub regs: RegAlloc,
    pub func: FuncId,
    pub env_depth: u16,
    pub controls: Vec<Control>,
    pub ret_reg: Reg,
    pub completion: Option<Reg>,
    pub chains: Vec<Vec<u32>>,
    pub strings: HashMap<Vec<u16>, u32>,
    pub last_span: Option<Span>,
}

/// The code generator.
pub(crate) struct Cg<'a> {
    pub a: &'a Analysis,
    pub depth: Depth,
    pub fns: Vec<Fs>,
    pub scopes: Vec<Cs>,
}

/// A name, resolved.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Place {
    /// `scopes[cs]`, binding `bi`.
    Local { cs: usize, bi: usize },
    /// Not declared anywhere: a global property (or a top-level name).
    Global,
}

pub(crate) fn err<T>(msg: impl Into<String>, span: Span) -> R<T> {
    Err(SyntaxError::new(msg, span))
}

impl Cg<'_> {
    pub fn f(&mut self) -> &mut Fs {
        let n = self.fns.len() - 1;
        &mut self.fns[n]
    }

    pub fn fr(&self) -> &Fs {
        &self.fns[self.fns.len() - 1]
    }

    pub fn emit(&mut self, op: Op) -> u32 {
        self.f().b.emit(op)
    }

    pub fn pc(&self) -> u32 {
        self.fr().b.ops.len() as u32
    }

    /// Note that ops from here on come from `span` (for stack lines).
    pub fn at(&mut self, span: Span) {
        let f = self.f();
        if f.last_span != Some(span) {
            let pc = f.b.ops.len() as u32;
            if let Some(last) = f.b.spans.last_mut()
                && last.0 == pc
            {
                last.1 = span;
                f.last_span = Some(span);
                return;
            }
            f.b.spans.push((pc, span));
            f.last_span = Some(span);
        }
    }

    pub fn tmp(&mut self, span: Span) -> R<Reg> {
        self.f().regs.alloc().or_else(|_| too_many_regs(span))
    }

    pub fn tmps(&mut self, n: usize, span: Span) -> R<Reg> {
        if n > u16::MAX as usize {
            return err(
                format!(
                    "too many arguments: {} (at most 65,535)",
                    group_digits(n as u64)
                ),
                span,
            );
        }
        self.f()
            .regs
            .alloc_n(n as u32)
            .or_else(|_| too_many_regs(span))
    }

    pub fn mark(&self) -> u32 {
        self.fr().regs.mark()
    }

    pub fn release(&mut self, m: u32) {
        self.f().regs.release_to(m);
    }

    pub fn jump(&mut self) -> u32 {
        self.emit(Op::Jump { offset: 0 })
    }

    /// Point the jump at `at` to `target`.
    pub fn patch_to(&mut self, at: u32, target: u32) {
        let offset = target as i64 - (at as i64 + 1);
        let offset = offset as i32;
        if let Some(
            Op::Jump { offset: o }
            | Op::JumpIfTrue { offset: o, .. }
            | Op::JumpIfFalse { offset: o, .. }
            | Op::JumpIfNullish { offset: o, .. },
        ) = self.f().b.ops.get_mut(at as usize)
        {
            *o = offset;
        }
    }

    /// Point the jump at `at` here.
    pub fn patch(&mut self, at: u32) {
        let here = self.pc();
        self.patch_to(at, here);
    }

    pub fn jump_to(&mut self, target: u32) {
        let j = self.jump();
        self.patch_to(j, target);
    }

    pub fn key(&mut self, name: &str) -> u32 {
        self.f().b.add_key(name)
    }

    pub fn string_units(&mut self, units: &[u16]) -> u32 {
        let f = self.f();
        if let Some(i) = f.strings.get(units) {
            return *i;
        }
        let i = f.b.strings.len() as u32;
        f.b.strings.push(units.to_vec());
        f.strings.insert(units.to_vec(), i);
        i
    }

    pub fn load_str(&mut self, s: &str, dst: Reg) {
        let units: Vec<u16> = s.encode_utf16().collect();
        let idx = self.string_units(&units);
        self.emit(Op::LoadString { dst, idx });
    }

    pub fn load_num(&mut self, n: f64, dst: Reg) {
        if n.fract() == 0.0
            && n >= f64::from(i32::MIN)
            && n <= f64::from(i32::MAX)
            && !(n == 0.0 && n.is_sign_negative())
        {
            self.emit(Op::LoadInt {
                dst,
                value: n as i32,
            });
        } else {
            let idx = self.f().b.add_number(n);
            self.emit(Op::LoadNumber { dst, idx });
        }
    }

    pub fn ic(&mut self) -> u32 {
        self.f().b.add_ic()
    }

    pub fn strict(&self) -> bool {
        self.a.funcs[self.fr().func].strict
    }

    pub fn throw_error(&mut self, kind: ErrorKind, msg: &str) {
        let units: Vec<u16> = msg.encode_utf16().collect();
        let m = self.string_units(&units);
        self.emit(Op::ThrowError {
            kind: kind_number(kind),
            msg: m,
        });
    }

    // ---- scopes -----------------------------------------------------------

    /// Find `name` from the innermost scope out.
    pub fn resolve(&self, name: &str) -> Place {
        self.resolve_below(self.scopes.len(), name)
    }

    /// Find `name` starting at `scopes[top - 1]`.
    pub fn resolve_below(&self, top: usize, name: &str) -> Place {
        for cs in (0..top).rev() {
            if let Some(bi) = self.a.scopes[self.scopes[cs].id].find(name) {
                return Place::Local { cs, bi };
            }
        }
        Place::Global
    }

    /// Environment hops from the running code to `scopes[cs]`.
    pub fn hops(&self, cs: usize, span: Span) -> R<u16> {
        let n = self.scopes[cs + 1..].iter().filter(|s| s.env).count();
        u16::try_from(n).or_else(|_| err("too many nested scopes (at most 65,535)", span))
    }

    /// Open the analysis scope `id`: give each name its place, push an
    /// environment if anything is captured, and start `let`/`const` off
    /// uninitialized. Parameters (`params`) already sit in registers.
    pub fn enter_scope(&mut self, id: ScopeId, params: &HashMap<String, Reg>, span: Span) -> R<()> {
        let mark = self.mark();
        let sc = &self.a.scopes[id];
        let mut stores = Vec::with_capacity(sc.bindings.len());
        let mut slots = 0u32;
        let mut to_copy = Vec::new();
        for b in &sc.bindings {
            let st = if self.a.is_global(id, b) {
                Store::Global
            } else if b.captured {
                let s = Store::Env(slots);
                if let Some(r) = params.get(&b.name)
                    && b.kind == BindKind::Param
                {
                    to_copy.push((*r, slots));
                }
                slots += 1;
                s
            } else if let (BindKind::Param, Some(r)) = (b.kind, params.get(&b.name)) {
                Store::Reg(*r)
            } else {
                Store::Reg(self.tmp(span)?)
            };
            stores.push(st);
        }
        let env = slots > 0;
        if env {
            self.emit(Op::PushEnv { size: slots });
            let f = self.f();
            if f.env_depth == u16::MAX {
                return err("too many nested scopes (at most 65,535)", span);
            }
            f.env_depth += 1;
            for (r, slot) in to_copy {
                self.emit(Op::SetEnv {
                    src: r,
                    depth: 0,
                    slot,
                });
            }
        }
        self.scopes.push(Cs {
            id,
            env,
            stores,
            mark,
        });
        // Let and const start uninitialized; block functions are created
        // when the scope starts.
        let cs = self.scopes.len() - 1;
        let n = self.a.scopes[id].bindings.len();
        let mut t = None;
        for bi in 0..n {
            let b = &self.a.scopes[id].bindings[bi];
            if b.kind.has_tdz() {
                match self.scopes[cs].stores[bi] {
                    Store::Reg(r) => {
                        self.emit(Op::LoadUninit { dst: r });
                    }
                    Store::Env(slot) => {
                        let tr = match t {
                            Some(x) => x,
                            None => {
                                let x = self.tmp(span)?;
                                self.emit(Op::LoadUninit { dst: x });
                                t = Some(x);
                                x
                            }
                        };
                        self.emit(Op::SetEnv {
                            src: tr,
                            depth: 0,
                            slot,
                        });
                    }
                    Store::Global => {
                        let key = self.key(&b.name.clone());
                        self.emit(Op::DeclareLex {
                            key,
                            konst: b.kind == BindKind::Const,
                        });
                    }
                }
            } else if self.scopes[cs].stores[bi] == Store::Global {
                let name = b.name.clone();
                let key = self.key(&name);
                self.emit(Op::DeclareVar { key });
            }
        }
        if let Some(x) = t {
            // The temporary isn't a binding; give it back if it's on top.
            if self.mark() == u32::from(x) + 1 {
                self.release(u32::from(x));
            }
        }
        Ok(())
    }

    /// Close the innermost scope.
    pub fn exit_scope(&mut self) {
        if let Some(cs) = self.scopes.pop() {
            if cs.env {
                self.emit(Op::PopEnv);
                let f = self.f();
                f.env_depth = f.env_depth.saturating_sub(1);
            }
            self.release(cs.mark);
        }
    }

    /// Enter the scope a node opened, by its key.
    pub fn enter_at(&mut self, span: Span, tag: Tag) -> R<()> {
        let Some(id) = self.a.scope_at(span, tag) else {
            return err("internal compiler error: scope not found", span);
        };
        self.enter_scope(id, &HashMap::new(), span)
    }

    // ---- names ------------------------------------------------------------

    /// True when a read of this binding from `pos` needs a TDZ check.
    fn needs_tdz(&self, cs: usize, bi: usize, pos: usize) -> bool {
        let id = self.scopes[cs].id;
        let sc = &self.a.scopes[id];
        let b = &sc.bindings[bi];
        if !b.kind.has_tdz() {
            return false;
        }
        let same_fn = sc.func == self.fr().func;
        !(same_fn && pos >= b.decl_end && sc.tag != Tag::Switch && b.decl_end > 0)
    }

    /// Read a binding into `dst`.
    pub fn load_place(&mut self, place: Place, name: &str, dst: Reg, span: Span) -> R<()> {
        match place {
            Place::Global => {
                let key = self.key(name);
                let ic = self.ic();
                self.emit(Op::GetGlobal { dst, key, ic });
            }
            Place::Local { cs, bi } => match self.scopes[cs].stores[bi] {
                Store::Global => {
                    let key = self.key(name);
                    let ic = self.ic();
                    self.emit(Op::GetGlobal { dst, key, ic });
                }
                Store::Reg(r) => {
                    if r != dst {
                        self.emit(Op::Move { dst, src: r });
                    }
                    if self.needs_tdz(cs, bi, span.start) {
                        let key = self.key(name);
                        self.emit(Op::CheckTdz {
                            src: dst,
                            name: key,
                        });
                    }
                }
                Store::Env(slot) => {
                    let depth = self.hops(cs, span)?;
                    self.emit(Op::GetEnv { dst, depth, slot });
                    if self.needs_tdz(cs, bi, span.start) {
                        let key = self.key(name);
                        self.emit(Op::CheckTdz {
                            src: dst,
                            name: key,
                        });
                    }
                }
            },
        }
        Ok(())
    }

    /// Read a name.
    pub fn load_name(&mut self, name: &str, dst: Reg, span: Span) -> R<()> {
        let p = self.resolve(name);
        self.load_place(p, name, dst, span)
    }

    /// Write `src` to a binding. `init` is the declaration's own write
    /// (no TDZ or `const` check, and global lexicals get initialized).
    pub fn store_place(
        &mut self,
        place: Place,
        name: &str,
        src: Reg,
        init: bool,
        span: Span,
    ) -> R<()> {
        match place {
            Place::Global => {
                let key = self.key(name);
                let ic = self.ic();
                self.emit(Op::SetGlobal { src, key, ic });
            }
            Place::Local { cs, bi } => {
                let id = self.scopes[cs].id;
                let kind = self.a.scopes[id].bindings[bi].kind;
                let store = self.scopes[cs].stores[bi];
                if !init {
                    if kind == BindKind::FuncSelf {
                        if self.strict() {
                            self.throw_error(
                                ErrorKind::Type,
                                &format!("Assignment to constant variable '{name}'."),
                            );
                        }
                        return Ok(());
                    }
                    if kind == BindKind::Const && store != Store::Global {
                        if self.needs_tdz(cs, bi, span.start) {
                            let m = self.mark();
                            let t = self.tmp(span)?;
                            self.load_place(place, name, t, span)?;
                            self.release(m);
                        }
                        self.throw_error(
                            ErrorKind::Type,
                            &format!("Assignment to constant variable '{name}'."),
                        );
                        return Ok(());
                    }
                    if kind.has_tdz()
                        && store != Store::Global
                        && self.needs_tdz(cs, bi, span.start)
                    {
                        let m = self.mark();
                        let t = self.tmp(span)?;
                        self.load_place(place, name, t, span)?;
                        self.release(m);
                    }
                }
                match store {
                    Store::Global => {
                        let key = self.key(name);
                        if init && kind.has_tdz() {
                            self.emit(Op::InitLex { src, key });
                        } else {
                            let ic = self.ic();
                            self.emit(Op::SetGlobal { src, key, ic });
                        }
                    }
                    Store::Reg(r) => {
                        if r != src {
                            self.emit(Op::Move { dst: r, src });
                        }
                    }
                    Store::Env(slot) => {
                        let depth = self.hops(cs, span)?;
                        self.emit(Op::SetEnv { src, depth, slot });
                    }
                }
            }
        }
        Ok(())
    }

    /// Write a name.
    pub fn store_name(&mut self, name: &str, src: Reg, init: bool, span: Span) -> R<()> {
        let p = self.resolve(name);
        self.store_place(p, name, src, init, span)
    }

    // ---- control ----------------------------------------------------------

    /// Leave for `exit` from here, running `finally` blocks on the way.
    /// `value` is the return value for [`Exit::Return`].
    pub fn emit_exit(&mut self, exit: Exit, value: Option<Reg>, span: Span) -> R<()> {
        let n = self.fr().controls.len();
        let cur_depth = self.fr().env_depth;
        let mut i = n;
        while i > 0 {
            i -= 1;
            let target = match exit {
                Exit::Break(t) | Exit::Continue(t) => t == i,
                Exit::Return => false,
            };
            if target {
                let c = &self.fr().controls[i];
                let want = match exit {
                    Exit::Continue(_) => c.cont_depth,
                    _ => c.break_depth,
                };
                for _ in want..cur_depth {
                    self.emit(Op::PopEnv);
                }
                let j = self.jump();
                let c = &mut self.f().controls[i];
                match exit {
                    Exit::Continue(_) => c.continues.push(j),
                    _ => c.breaks.push(j),
                }
                return Ok(());
            }
            if let CtlKind::Finally { kind_reg, .. } = self.fr().controls[i].kind {
                let want = self.fr().controls[i].break_depth;
                if let (Exit::Return, Some(v)) = (exit, value) {
                    let rr = self.fr().ret_reg;
                    if v != rr {
                        self.emit(Op::Move { dst: rr, src: v });
                    }
                }
                for _ in want..cur_depth {
                    self.emit(Op::PopEnv);
                }
                let k = match &self.fr().controls[i].kind {
                    CtlKind::Finally { exits, .. } => exits.len(),
                    _ => 0,
                };
                let code =
                    i32::try_from(2 + k).or_else(|_| err("too many exits from one try", span))?;
                self.emit(Op::LoadInt {
                    dst: kind_reg,
                    value: code,
                });
                let j = self.jump();
                if let CtlKind::Finally { exits, jumps, .. } = &mut self.f().controls[i].kind {
                    exits.push(exit);
                    jumps.push(j);
                }
                return Ok(());
            }
        }
        match exit {
            Exit::Return => {
                let src = match value {
                    Some(v) => v,
                    None => self.fr().ret_reg,
                };
                self.emit(Op::Return { src });
                Ok(())
            }
            _ => err("internal compiler error: jump target not found", span),
        }
    }

    // ---- programs ---------------------------------------------------------

    fn top(&mut self, program: &Program, name: &str) -> Result<CodeBlock, SyntaxError> {
        let func = 0;
        let mut b = CodeBlock::new(name, 0, 0);
        b.strict = self.a.funcs[func].strict;
        b.span = program.span;
        self.fns.push(Fs {
            b,
            regs: RegAlloc::with_fixed(0),
            func,
            env_depth: 0,
            controls: Vec::new(),
            ret_reg: 0,
            completion: None,
            chains: Vec::new(),
            strings: HashMap::new(),
            last_span: None,
        });
        let span = program.span;
        let ret = self.tmp(span)?;
        self.f().ret_reg = ret;
        self.emit(Op::LoadUndefined { dst: ret });
        self.f().completion = Some(ret);
        self.enter_scope(0, &HashMap::new(), span)?;
        self.pseudo_bindings(span)?;
        self.hoist_functions(&program.body)?;
        self.stmts(&program.body)?;
        self.emit(Op::Return { src: ret });
        let mut fs = self
            .fns
            .pop()
            .ok_or_else(|| SyntaxError::new("no function", span))?;
        fs.b.register_count = fs.regs.high_water();
        Ok(fs.b)
    }

    /// Fill in `this` and `arguments` bindings of the current function
    /// scope, and a named function expression's own name.
    pub fn pseudo_bindings(&mut self, span: Span) -> R<()> {
        let cs = self.scopes.len() - 1;
        let id = self.scopes[cs].id;
        for bi in 0..self.a.scopes[id].bindings.len() {
            let kind = self.a.scopes[id].bindings[bi].kind;
            let place = Place::Local { cs, bi };
            let name = self.a.scopes[id].bindings[bi].name.clone();
            match kind {
                BindKind::This => {
                    let m = self.mark();
                    let t = self.tmp(span)?;
                    self.emit(Op::LoadThis { dst: t });
                    self.store_place(place, &name, t, true, span)?;
                    self.release(m);
                }
                BindKind::FuncSelf => {
                    let m = self.mark();
                    let t = self.tmp(span)?;
                    self.emit(Op::LoadCallee { dst: t });
                    self.store_place(place, &name, t, true, span)?;
                    self.release(m);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Create the function declarations of a statement list (hoisted to
    /// the start of their scope).
    pub fn hoist_functions(&mut self, body: &[Stmt]) -> R<()> {
        for st in body {
            if let crate::anvil_js::ast::StmtKind::Function(f) = &st.kind {
                let Some(id) = &f.id else { continue };
                if f.is_async || f.is_generator {
                    return err(unsupported_fn(f.is_async, f.is_generator), st.span);
                }
                let m = self.mark();
                let t = self.tmp(st.span)?;
                self.closure(expr::FnRef::Decl(f), &id.name, t)?;
                let place = self.resolve_below(self.scopes.len(), &id.name);
                self.store_place(place, &id.name, t, true, st.span)?;
                self.release(m);
            }
        }
        Ok(())
    }
}

pub(crate) fn too_many_regs<T>(span: Span) -> R<T> {
    err(
        "function needs more than 65,535 registers (max_registers_per_function)",
        span,
    )
}

pub(crate) fn unsupported_fn(is_async: bool, is_generator: bool) -> String {
    match (is_async, is_generator) {
        (true, true) => "async generator functions are not supported yet".to_string(),
        (true, false) => "async functions are not supported yet".to_string(),
        _ => "generator functions are not supported yet".to_string(),
    }
}

pub(crate) fn kind_number(kind: ErrorKind) -> u8 {
    match kind {
        ErrorKind::Error => 0,
        ErrorKind::Type => 1,
        ErrorKind::Range => 2,
        ErrorKind::Reference => 3,
        ErrorKind::Syntax => 4,
    }
}

/// The plain name a parameter binds, if it's that simple.
pub(crate) fn simple_param(p: &Pat) -> Option<(&str, Option<&crate::anvil_js::ast::Expr>, bool)> {
    match &p.kind {
        PatKind::Ident(n) => Some((n.as_str(), None, false)),
        PatKind::Assign { target, default } => match &target.kind {
            PatKind::Ident(n) => Some((n.as_str(), Some(&**default), false)),
            _ => None,
        },
        PatKind::Rest(inner) => match &inner.kind {
            PatKind::Ident(n) => Some((n.as_str(), None, true)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
