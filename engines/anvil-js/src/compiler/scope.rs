// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Scope analysis: which names each scope declares, and which of those an
//! inner function reaches (those live in an environment instead of a
//! register).
//!
//! The pass walks the tree once, declaring names as each scope opens
//! (hoisting `var` and function declarations to their function, `let` and
//! `const` to their block) and noting every name use. Uses are resolved at
//! the end, when every scope is complete. Code generation walks the same
//! tree and finds each scope again by its key: the span of the node that
//! opens it plus a tag.
//!
//! Names are resolved statically. Sloppy-mode `with` isn't supported and
//! direct `eval` only runs where no local binding is visible, so nothing
//! can add a binding at run time that this pass doesn't see.

use super::{CompileError, Depth};
use crate::anvil_js::ast::{
    ArrowBody, ArrowFunction, CatchClause, Expr, ExprKind, ForHead, ForInit, Function, MemberProp,
    Pat, PatKind, PatProp, Prop, PropKey, PropKind, Stmt, StmtKind, VarDecl, VarKind,
};
use crate::anvil_js::error::Span;
use std::collections::HashMap;

/// Index of a scope in [`Analysis::scopes`].
pub type ScopeId = usize;
/// Index of a function in [`Analysis::funcs`].
pub type FuncId = usize;

/// What opened a scope. With the node's span this finds the scope again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tag {
    /// A script or eval program.
    Script,
    /// A function or arrow (parameters and body together).
    Func,
    /// A block statement.
    Block,
    /// The head of a `for (;;)` loop.
    For,
    /// One iteration of `for-in` / `for-of`.
    ForEach,
    /// The cases of a `switch`.
    Switch,
    /// The block of a `try`.
    TryBlock,
    /// A `catch` clause (its parameter and body together).
    Catch,
    /// A `finally` block.
    Finally,
}

/// A scope's key: start, end and tag.
pub type Key = (usize, usize, Tag);

/// The key for the node at `span`.
pub fn key(span: Span, tag: Tag) -> Key {
    (span.start, span.end, tag)
}

/// How a name was declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    /// `var`, or an Annex B copy of a block function.
    Var,
    /// `let`
    Let,
    /// `const`
    Const,
    /// A function declaration.
    Function,
    /// A parameter.
    Param,
    /// A `catch` parameter.
    CatchParam,
    /// A named function expression's own name.
    FuncSelf,
    /// `this`, for arrows inside the function.
    This,
    /// The `arguments` object.
    Arguments,
}

impl BindKind {
    /// `let` and `const` start uninitialized.
    pub fn has_tdz(self) -> bool {
        matches!(self, BindKind::Let | BindKind::Const)
    }
}

/// One declared name.
#[derive(Clone, Debug)]
pub struct Binding {
    /// The name (`this` and `arguments` for the pseudo-bindings).
    pub name: String,
    /// How it was declared.
    pub kind: BindKind,
    /// An inner function uses it, so it lives in an environment slot.
    pub captured: bool,
    /// Where its declaration ends. Uses in the same function after this
    /// point need no TDZ check (outside `switch`).
    pub decl_end: usize,
}

/// One scope.
#[derive(Clone, Debug)]
pub struct Scope {
    /// What opened it.
    pub tag: Tag,
    /// The enclosing scope.
    pub parent: Option<ScopeId>,
    /// The function it belongs to.
    pub func: FuncId,
    /// Declared names, in declaration order.
    pub bindings: Vec<Binding>,
    index: HashMap<String, usize>,
}

impl Scope {
    /// The binding for `name`, if this scope declares it.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }
}

/// One function (or the program itself).
#[derive(Clone, Debug)]
pub struct FuncInfo {
    /// Its outermost scope.
    pub scope: ScopeId,
    /// Arrows take `this` and `arguments` from outside.
    pub arrow: bool,
    /// Strict code.
    pub strict: bool,
    /// The script or eval program, not a function.
    pub top: bool,
    /// Something in it (or an arrow inside) uses `arguments`.
    pub uses_arguments: bool,
    /// `arguments[i]` is tied to parameter `i`: sloppy code with plain,
    /// distinct parameter names that uses `arguments`.
    pub mapped: bool,
    /// Parameter names are all different.
    pub distinct_params: bool,
    /// Parameters are plain names with no defaults or rest.
    pub simple_params: bool,
}

/// How the program's own top-level names are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopMode {
    /// A script: `var` and functions on the global object, `let` and
    /// `const` in the global lexical table.
    Script,
    /// Sloppy eval code: `var` and functions on the global object, `let`
    /// and `const` local to the eval.
    SloppyEval,
    /// Strict eval code: everything local to the eval.
    StrictEval,
}

/// The result of the pass.
#[derive(Debug)]
pub struct Analysis {
    /// Every scope.
    pub scopes: Vec<Scope>,
    /// Every function; 0 is the program.
    pub funcs: Vec<FuncInfo>,
    /// Scope lookup by key.
    pub by_key: HashMap<Key, ScopeId>,
    /// How top-level names are stored.
    pub top_mode: TopMode,
}

impl Analysis {
    /// The scope opened by the node at `span` with `tag`.
    pub fn scope_at(&self, span: Span, tag: Tag) -> Option<ScopeId> {
        self.by_key.get(&key(span, tag)).copied()
    }

    /// True when the binding is stored outside the program's frame: a
    /// top-level name of a script, or a top-level `var` or function of
    /// sloppy eval code.
    pub fn is_global(&self, scope: ScopeId, b: &Binding) -> bool {
        if scope != 0 {
            return false;
        }
        match b.kind {
            BindKind::This | BindKind::Arguments => false,
            BindKind::Var | BindKind::Function => self.top_mode != TopMode::StrictEval,
            _ => self.top_mode == TopMode::Script,
        }
    }
}

struct Pass<'d> {
    a: Analysis,
    refs: Vec<(ScopeId, String)>,
    depth: &'d mut Depth,
}

/// Analyze a program. `strict` is the program's strictness.
pub fn analyze(
    body: &[Stmt],
    span: Span,
    strict: bool,
    mode: TopMode,
    depth: &mut Depth,
) -> Result<Analysis, CompileError> {
    let mut p = Pass {
        a: Analysis {
            scopes: Vec::new(),
            funcs: Vec::new(),
            by_key: HashMap::new(),
            top_mode: mode,
        },
        refs: Vec::new(),
        depth,
    };
    p.a.funcs.push(FuncInfo {
        scope: 0,
        arrow: false,
        strict,
        top: true,
        uses_arguments: false,
        mapped: false,
        distinct_params: true,
        simple_params: true,
    });
    let s = p.open(span, Tag::Script, None, 0);
    p.hoist_vars(s, body)?;
    p.declare_lexicals(s, body, true)?;
    p.stmts(s, body)?;
    p.resolve_all();
    Ok(p.a)
}

impl Pass<'_> {
    fn open(&mut self, span: Span, tag: Tag, parent: Option<ScopeId>, func: FuncId) -> ScopeId {
        let id = self.a.scopes.len();
        self.a.scopes.push(Scope {
            tag,
            parent,
            func,
            bindings: Vec::new(),
            index: HashMap::new(),
        });
        self.a.by_key.insert(key(span, tag), id);
        id
    }

    fn declare(&mut self, s: ScopeId, name: &str, kind: BindKind, decl_end: usize) {
        let scope = &mut self.a.scopes[s];
        if let Some(i) = scope.find(name) {
            let b = &mut scope.bindings[i];
            // A function declaration replaces a `var` of the same name; a
            // `var` never replaces a parameter or function.
            if kind == BindKind::Function && b.kind == BindKind::Var {
                b.kind = BindKind::Function;
            }
            return;
        }
        scope.index.insert(name.to_string(), scope.bindings.len());
        scope.bindings.push(Binding {
            name: name.to_string(),
            kind,
            captured: false,
            decl_end,
        });
    }

    fn use_name(&mut self, s: ScopeId, name: &str) {
        self.refs.push((s, name.to_string()));
    }

    // ---- declarations -----------------------------------------------------

    /// Every `var` name in `body`, not counting nested functions.
    fn hoist_vars(&mut self, s: ScopeId, body: &[Stmt]) -> Result<(), CompileError> {
        for st in body {
            self.hoist_stmt(s, st)?;
        }
        Ok(())
    }

    fn hoist_decl(&mut self, s: ScopeId, d: &VarDecl) {
        if d.kind == VarKind::Var {
            for dc in &d.decls {
                for n in dc.target.bound_names() {
                    self.declare(s, &n.name, BindKind::Var, 0);
                }
            }
        }
    }

    fn hoist_stmt(&mut self, s: ScopeId, st: &Stmt) -> Result<(), CompileError> {
        self.depth.enter(st.span)?;
        match &st.kind {
            StmtKind::Var(d) => self.hoist_decl(s, d),
            StmtKind::Block(b) => self.hoist_vars(s, b)?,
            StmtKind::If { cons, alt, .. } => {
                self.hoist_stmt(s, cons)?;
                if let Some(a) = alt {
                    self.hoist_stmt(s, a)?;
                }
            }
            StmtKind::For { init, body, .. } => {
                if let Some(ForInit::Var(d)) = init {
                    self.hoist_decl(s, d);
                }
                self.hoist_stmt(s, body)?;
            }
            StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } => {
                if let ForHead::Var(d) = left {
                    self.hoist_decl(s, d);
                }
                self.hoist_stmt(s, body)?;
            }
            StmtKind::While { body, .. }
            | StmtKind::DoWhile { body, .. }
            | StmtKind::Labeled { body, .. }
            | StmtKind::With { body, .. } => self.hoist_stmt(s, body)?,
            StmtKind::Switch { cases, .. } => {
                for c in cases {
                    self.hoist_vars(s, &c.body)?;
                }
            }
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => {
                self.hoist_vars(s, block)?;
                if let Some(h) = handler {
                    self.hoist_vars(s, &h.body)?;
                }
                if let Some(f) = finalizer {
                    self.hoist_vars(s, f)?;
                }
            }
            _ => {}
        }
        self.depth.leave();
        Ok(())
    }

    /// `let`, `const`, class and (in blocks) function names declared
    /// directly in `body`. At a function's top level, functions are
    /// var-scoped instead.
    fn declare_lexicals(
        &mut self,
        s: ScopeId,
        body: &[Stmt],
        top: bool,
    ) -> Result<(), CompileError> {
        for st in body {
            match &st.kind {
                StmtKind::Var(d) if d.kind != VarKind::Var => {
                    let kind = if d.kind == VarKind::Const {
                        BindKind::Const
                    } else {
                        BindKind::Let
                    };
                    for dc in &d.decls {
                        for n in dc.target.bound_names() {
                            self.declare(s, &n.name, kind, dc.span.end);
                        }
                    }
                }
                StmtKind::Class(c) => {
                    if let Some(id) = &c.id {
                        self.declare(s, &id.name, BindKind::Let, st.span.end);
                    }
                }
                StmtKind::Function(f) => {
                    if let Some(id) = &f.id {
                        if top {
                            self.declare(s, &id.name, BindKind::Function, 0);
                        } else {
                            self.declare(s, &id.name, BindKind::Let, 0);
                            let b = self.a.scopes[s].find(&id.name).unwrap_or(0);
                            self.a.scopes[s].bindings[b].kind = BindKind::Function;
                            self.annex_b(s, &id.name);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Annex B: a function declared in a block of sloppy code is also a
    /// `var` of its function, unless that would clash with a lexical name
    /// in between or the name is a parameter.
    fn annex_b(&mut self, block: ScopeId, name: &str) {
        let func = self.a.scopes[block].func;
        if self.a.funcs[func].strict {
            return;
        }
        let fscope = self.a.funcs[func].scope;
        let mut cur = self.a.scopes[block].parent;
        while let Some(c) = cur {
            if c == fscope {
                break;
            }
            if self.a.scopes[c].find(name).is_some() {
                return;
            }
            cur = self.a.scopes[c].parent;
        }
        if self.a.scopes[fscope].find(name).is_none() {
            self.declare(fscope, name, BindKind::Var, 0);
        }
    }

    /// Whether a block function binding gets the Annex B copy: the same
    /// test as [`Pass::annex_b`], made again by code generation.
    pub fn annex_b_target(a: &Analysis, block: ScopeId, name: &str) -> bool {
        let func = a.scopes[block].func;
        if a.funcs[func].strict {
            return false;
        }
        let fscope = a.funcs[func].scope;
        let mut cur = a.scopes[block].parent;
        while let Some(c) = cur {
            if c == fscope {
                break;
            }
            if a.scopes[c].find(name).is_some() {
                return false;
            }
            cur = a.scopes[c].parent;
        }
        match a.scopes[fscope].find(name) {
            Some(i) => matches!(
                a.scopes[fscope].bindings[i].kind,
                BindKind::Var | BindKind::Function
            ),
            None => false,
        }
    }

    // ---- statements -------------------------------------------------------

    fn stmts(&mut self, s: ScopeId, body: &[Stmt]) -> Result<(), CompileError> {
        for st in body {
            self.stmt(s, st)?;
        }
        Ok(())
    }

    fn block(
        &mut self,
        s: ScopeId,
        span: Span,
        tag: Tag,
        body: &[Stmt],
    ) -> Result<(), CompileError> {
        let func = self.a.scopes[s].func;
        let b = self.open(span, tag, Some(s), func);
        self.declare_lexicals(b, body, false)?;
        self.stmts(b, body)
    }

    fn stmt(&mut self, s: ScopeId, st: &Stmt) -> Result<(), CompileError> {
        self.depth.enter(st.span)?;
        let func = self.a.scopes[s].func;
        match &st.kind {
            StmtKind::Expr { expr, .. } => self.expr(s, expr)?,
            StmtKind::Var(d) => self.var_decl(s, d)?,
            StmtKind::Function(f) => self.function(s, f, false)?,
            StmtKind::Class(_) => {}
            StmtKind::Block(b) => self.block(s, st.span, Tag::Block, b)?,
            StmtKind::Empty | StmtKind::Debugger => {}
            StmtKind::If { test, cons, alt } => {
                self.expr(s, test)?;
                self.branch(s, cons)?;
                if let Some(a) = alt {
                    self.branch(s, a)?;
                }
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                let f = self.open(st.span, Tag::For, Some(s), func);
                match init {
                    Some(ForInit::Var(d)) => {
                        if d.kind != VarKind::Var {
                            let kind = if d.kind == VarKind::Const {
                                BindKind::Const
                            } else {
                                BindKind::Let
                            };
                            for dc in &d.decls {
                                for n in dc.target.bound_names() {
                                    self.declare(f, &n.name, kind, dc.span.end);
                                }
                            }
                        }
                        self.var_decl(f, d)?;
                    }
                    Some(ForInit::Expr(e)) => self.expr(f, e)?,
                    None => {}
                }
                if let Some(t) = test {
                    self.expr(f, t)?;
                }
                if let Some(u) = update {
                    self.expr(f, u)?;
                }
                self.stmt(f, body)?;
            }
            StmtKind::ForIn { left, right, body }
            | StmtKind::ForOf {
                left, right, body, ..
            } => {
                self.expr(s, right)?;
                let f = self.open(st.span, Tag::ForEach, Some(s), func);
                match left {
                    ForHead::Var(d) => {
                        if d.kind != VarKind::Var {
                            let kind = if d.kind == VarKind::Const {
                                BindKind::Const
                            } else {
                                BindKind::Let
                            };
                            for dc in &d.decls {
                                for n in dc.target.bound_names() {
                                    self.declare(f, &n.name, kind, 0);
                                }
                            }
                        }
                        self.var_decl(f, d)?;
                    }
                    ForHead::Pat(p) => self.pat(f, p)?,
                }
                self.stmt(f, body)?;
            }
            StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
                self.expr(s, test)?;
                self.stmt(s, body)?;
            }
            StmtKind::Continue(_) | StmtKind::Break(_) => {}
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(s, e)?;
                }
            }
            StmtKind::With { object, body } => {
                self.expr(s, object)?;
                self.stmt(s, body)?;
            }
            StmtKind::Switch { disc, cases } => {
                self.expr(s, disc)?;
                let sw = self.open(st.span, Tag::Switch, Some(s), func);
                for c in cases {
                    self.declare_lexicals(sw, &c.body, false)?;
                }
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(sw, t)?;
                    }
                    self.stmts(sw, &c.body)?;
                }
            }
            StmtKind::Labeled { body, .. } => self.stmt(s, body)?,
            StmtKind::Throw(e) => self.expr(s, e)?,
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => {
                self.block(s, st.span, Tag::TryBlock, block)?;
                if let Some(h) = handler {
                    self.catch(s, h)?;
                }
                if let Some(f) = finalizer {
                    self.block(s, st.span, Tag::Finally, f)?;
                }
            }
            StmtKind::Import(_) | StmtKind::Export(_) => {}
        }
        self.depth.leave();
        Ok(())
    }

    /// An `if` branch. A function declaration written straight in one
    /// (sloppy code, Annex B) acts as if it were in a block of its own.
    fn branch(&mut self, s: ScopeId, st: &Stmt) -> Result<(), CompileError> {
        if matches!(st.kind, StmtKind::Function(_)) {
            self.block(s, st.span, Tag::Block, std::slice::from_ref(st))
        } else {
            self.stmt(s, st)
        }
    }

    fn catch(&mut self, s: ScopeId, h: &CatchClause) -> Result<(), CompileError> {
        let func = self.a.scopes[s].func;
        let c = self.open(h.span, Tag::Catch, Some(s), func);
        if let Some(p) = &h.param {
            for n in p.bound_names() {
                self.declare(c, &n.name, BindKind::CatchParam, 0);
            }
            self.pat(c, p)?;
        }
        self.declare_lexicals(c, &h.body, false)?;
        self.stmts(c, &h.body)
    }

    fn var_decl(&mut self, s: ScopeId, d: &VarDecl) -> Result<(), CompileError> {
        for dc in &d.decls {
            self.pat(s, &dc.target)?;
            if let Some(e) = &dc.init {
                self.expr(s, e)?;
            }
        }
        Ok(())
    }

    // ---- functions --------------------------------------------------------

    fn function(&mut self, s: ScopeId, f: &Function, is_expr: bool) -> Result<(), CompileError> {
        let parent = self.a.scopes[s].func;
        let id = self.a.funcs.len();
        let fs = self.open(f.span, Tag::Func, Some(s), id);
        let mut names = Vec::new();
        for p in &f.params {
            names.extend(p.bound_names().into_iter().map(|i| i.name));
        }
        let distinct = {
            let mut seen = std::collections::HashSet::new();
            names.iter().all(|n| seen.insert(n.clone()))
        };
        self.a.funcs.push(FuncInfo {
            scope: fs,
            arrow: false,
            strict: f.strict,
            top: false,
            uses_arguments: false,
            mapped: false,
            distinct_params: distinct,
            simple_params: f.simple_params,
        });
        let _ = parent;
        for n in &names {
            self.declare(fs, n, BindKind::Param, 0);
        }
        self.hoist_vars(fs, &f.body)?;
        self.declare_lexicals(fs, &f.body, true)?;
        if is_expr
            && let Some(idn) = &f.id
            && self.a.scopes[fs].find(&idn.name).is_none()
        {
            self.declare(fs, &idn.name, BindKind::FuncSelf, 0);
        }
        for p in &f.params {
            self.pat(fs, p)?;
        }
        self.stmts(fs, &f.body)
    }

    fn arrow(&mut self, s: ScopeId, f: &ArrowFunction) -> Result<(), CompileError> {
        let id = self.a.funcs.len();
        let fs = self.open(f.span, Tag::Func, Some(s), id);
        let mut names = Vec::new();
        for p in &f.params {
            names.extend(p.bound_names().into_iter().map(|i| i.name));
        }
        let simple = f.params.iter().all(|p| matches!(p.kind, PatKind::Ident(_)));
        self.a.funcs.push(FuncInfo {
            scope: fs,
            arrow: true,
            strict: f.strict,
            top: false,
            uses_arguments: false,
            mapped: false,
            distinct_params: true,
            simple_params: simple,
        });
        for n in &names {
            self.declare(fs, n, BindKind::Param, 0);
        }
        for p in &f.params {
            self.pat(fs, p)?;
        }
        match &f.body {
            ArrowBody::Expr(e) => self.expr(fs, e),
            ArrowBody::Block(b) => {
                self.hoist_vars(fs, b)?;
                self.declare_lexicals(fs, b, true)?;
                self.stmts(fs, b)
            }
        }
    }

    // ---- expressions ------------------------------------------------------

    fn pat(&mut self, s: ScopeId, p: &Pat) -> Result<(), CompileError> {
        self.depth.enter(p.span)?;
        match &p.kind {
            PatKind::Ident(n) => self.use_name(s, n),
            PatKind::Array(items) => {
                for q in items.iter().flatten() {
                    self.pat(s, q)?;
                }
            }
            PatKind::Object(props) => {
                for q in props {
                    match q {
                        PatProp::Prop { key, value, .. } => {
                            self.prop_key(s, key)?;
                            self.pat(s, value)?;
                        }
                        PatProp::Rest(r) => self.pat(s, r)?,
                    }
                }
            }
            PatKind::Assign { target, default } => {
                self.pat(s, target)?;
                self.expr(s, default)?;
            }
            PatKind::Rest(r) => self.pat(s, r)?,
            PatKind::Expr(e) => self.expr(s, e)?,
        }
        self.depth.leave();
        Ok(())
    }

    fn prop_key(&mut self, s: ScopeId, k: &PropKey) -> Result<(), CompileError> {
        if let PropKey::Computed(e) = k {
            self.expr(s, e)?;
        }
        Ok(())
    }

    fn uses_this(&mut self, s: ScopeId) {
        let func = self.a.scopes[s].func;
        if self.a.funcs[func].arrow {
            self.use_name(s, "this");
        }
    }

    fn expr(&mut self, s: ScopeId, e: &Expr) -> Result<(), CompileError> {
        self.depth.enter(e.span)?;
        match &e.kind {
            ExprKind::Ident(n) => self.use_name(s, n),
            ExprKind::This => self.uses_this(s),
            ExprKind::Super
            | ExprKind::Null
            | ExprKind::Bool(_)
            | ExprKind::Num(_)
            | ExprKind::BigInt(_)
            | ExprKind::Str(_)
            | ExprKind::Regex { .. }
            | ExprKind::NewTarget
            | ExprKind::ImportMeta => {}
            ExprKind::Template(t) => {
                for x in &t.exprs {
                    self.expr(s, x)?;
                }
            }
            ExprKind::TaggedTemplate { tag, quasi } => {
                self.expr(s, tag)?;
                for x in &quasi.exprs {
                    self.expr(s, x)?;
                }
            }
            ExprKind::Array(items) => {
                for x in items.iter().flatten() {
                    self.expr(s, x)?;
                }
            }
            ExprKind::Object(props) => {
                for p in props {
                    self.prop(s, p)?;
                }
            }
            ExprKind::Function(f) => self.function(s, f, true)?,
            ExprKind::Arrow(a) => self.arrow(s, a)?,
            ExprKind::Class(_) => {}
            ExprKind::Unary { arg, .. }
            | ExprKind::Update { arg, .. }
            | ExprKind::Spread(arg)
            | ExprKind::Paren(arg)
            | ExprKind::Await(arg)
            | ExprKind::OptionalChain(arg) => self.expr(s, arg)?,
            ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. } => {
                self.expr(s, left)?;
                self.expr(s, right)?;
            }
            ExprKind::Assign { target, value, .. } => {
                self.pat(s, target)?;
                self.expr(s, value)?;
            }
            ExprKind::Cond { test, cons, alt } => {
                self.expr(s, test)?;
                self.expr(s, cons)?;
                self.expr(s, alt)?;
            }
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
                self.expr(s, callee)?;
                for a in args {
                    self.expr(s, a)?;
                }
            }
            ExprKind::Member { object, prop, .. } => {
                self.expr(s, object)?;
                if let MemberProp::Computed(k) = prop {
                    self.expr(s, k)?;
                }
            }
            ExprKind::Seq(xs) => {
                for x in xs {
                    self.expr(s, x)?;
                }
            }
            ExprKind::Yield { arg, .. } => {
                if let Some(a) = arg {
                    self.expr(s, a)?;
                }
            }
            ExprKind::Import { source, options } => {
                self.expr(s, source)?;
                if let Some(o) = options {
                    self.expr(s, o)?;
                }
            }
            ExprKind::PrivateIn { right, .. } => self.expr(s, right)?,
        }
        self.depth.leave();
        Ok(())
    }

    fn prop(&mut self, s: ScopeId, p: &Prop) -> Result<(), CompileError> {
        match &p.kind {
            PropKind::Init { key, value } => {
                self.prop_key(s, key)?;
                self.expr(s, value)
            }
            PropKind::Shorthand(id) => {
                self.use_name(s, &id.name);
                Ok(())
            }
            PropKind::Method { key, value, .. } => {
                self.prop_key(s, key)?;
                self.function(s, value, false)
            }
            PropKind::Spread(e) => self.expr(s, e),
        }
    }

    // ---- resolution -------------------------------------------------------

    fn resolve_all(&mut self) {
        let refs = std::mem::take(&mut self.refs);
        for (s, name) in refs {
            self.resolve(s, &name);
        }
        for f in &mut self.a.funcs {
            f.mapped = !f.strict
                && !f.arrow
                && !f.top
                && f.uses_arguments
                && f.simple_params
                && f.distinct_params;
        }
        for i in 0..self.a.funcs.len() {
            if self.a.funcs[i].mapped {
                let fs = self.a.funcs[i].scope;
                for b in &mut self.a.scopes[fs].bindings {
                    if b.kind == BindKind::Param {
                        b.captured = true;
                    }
                }
            }
        }
    }

    fn resolve(&mut self, from: ScopeId, name: &str) {
        let from_func = self.a.scopes[from].func;
        let mut cur = Some(from);
        while let Some(c) = cur {
            if let Some(i) = self.a.scopes[c].find(name) {
                if self.a.scopes[c].func != from_func {
                    self.a.scopes[c].bindings[i].captured = true;
                }
                return;
            }
            let func = self.a.scopes[c].func;
            let fi = &self.a.funcs[func];
            if fi.scope == c && !fi.arrow {
                if name == "this" {
                    self.declare(c, "this", BindKind::This, 0);
                    let i = self.a.scopes[c].bindings.len() - 1;
                    if func != from_func {
                        self.a.scopes[c].bindings[i].captured = true;
                    }
                    return;
                }
                if name == "arguments" && !fi.top {
                    self.declare(c, "arguments", BindKind::Arguments, 0);
                    self.a.funcs[func].uses_arguments = true;
                    let i = self.a.scopes[c].bindings.len() - 1;
                    if func != from_func {
                        self.a.scopes[c].bindings[i].captured = true;
                    }
                    return;
                }
            }
            cur = self.a.scopes[c].parent;
        }
    }
}

/// Whether a block function binding gets the Annex B copy, for code
/// generation. Same test as `Pass::annex_b_target`.
pub fn annex_b_target(a: &Analysis, block: ScopeId, name: &str) -> bool {
    Pass::annex_b_target(a, block, name)
}
