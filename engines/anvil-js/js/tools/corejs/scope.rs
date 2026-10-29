// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Scope analysis: every name occurrence resolved to a binding or marked
//! free, with captured bindings flagged, plus the redeclaration early
//! errors of strict code.

use std::rc::Rc;

use super::ast::*;
use super::lexer::{Diag, DiagKind};

/// What made a scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// The whole file.
    Top,
    /// A function's own scope.
    Function {
        /// Arrow functions have no `this` of their own.
        arrow: bool,
    },
    /// The scope holding a named function expression's own name.
    FuncName,
    /// A block, loop head, `switch` body.
    Block,
    /// A catch clause parameter.
    Catch,
    /// A class's inner name.
    Class,
}

/// One scope.
#[derive(Clone, Debug)]
pub struct Scope {
    /// What made it.
    pub kind: ScopeKind,
    /// Enclosing scope.
    pub parent: Option<u32>,
    /// Nearest enclosing function (or top) scope, itself included.
    pub func: u32,
    /// Bindings in declaration order.
    pub bindings: Vec<u32>,
}

/// Binding kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    /// `var`
    Var,
    /// `let`
    Let,
    /// `const`
    Const,
    /// Function declaration.
    Func,
    /// Class declaration.
    Class,
    /// Parameter.
    Param,
    /// Catch parameter.
    Catch,
    /// Imported name.
    Import,
    /// `this`, `%fn`, `%nt`.
    Special,
    /// A class's own name inside its body.
    ClassInner,
    /// A named function expression's own name.
    FuncName,
}

impl BindKind {
    /// Bindings that start out uninitialized (temporal dead zone).
    pub fn has_tdz(self) -> bool {
        matches!(
            self,
            BindKind::Let | BindKind::Const | BindKind::Class | BindKind::ClassInner
        )
    }

    /// Bindings that can't be assigned to.
    pub fn is_const(self) -> bool {
        matches!(
            self,
            BindKind::Const | BindKind::Import | BindKind::ClassInner | BindKind::FuncName
        )
    }
}

/// One binding.
#[derive(Clone, Debug)]
pub struct Binding {
    /// Its name.
    pub name: Rc<str>,
    /// Kind.
    pub kind: BindKind,
    /// Scope it lives in.
    pub scope: u32,
    /// Referenced from a function other than the one it is declared in.
    pub captured: bool,
    /// Where it is declared.
    pub pos: Pos,
}

/// How a name occurrence resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    /// Not visited (dead code inside an `Invalid` node).
    None,
    /// A binding.
    Binding(u32),
    /// No binding: a global.
    Free,
}

/// The result of the analysis.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Scopes, indexed by scope number.
    pub scopes: Vec<Scope>,
    /// Bindings.
    pub bindings: Vec<Binding>,
    /// Resolution of every name occurrence, indexed by name id.
    pub res: Vec<Res>,
    /// The scope each name occurrence appears in, indexed by name id.
    pub ref_scope: Vec<u32>,
    /// Where each name occurrence is in the source, indexed by name id.
    pub name_pos: Vec<Option<Pos>>,
    /// Name ids that are the value of a shorthand property `{ a }`.
    pub shorthand: Vec<bool>,
}

impl Analysis {
    /// The binding a name occurrence resolved to.
    pub fn binding_of(&self, n: &Name) -> Option<&Binding> {
        match self.res.get(n.id as usize) {
            Some(Res::Binding(b)) => self.bindings.get(*b as usize),
            _ => None,
        }
    }
}

struct Walker {
    a: Analysis,
    cur: u32,
}

type SResult<T> = Result<T, Diag>;

fn err(msg: String, p: Pos) -> Diag {
    Diag {
        kind: DiagKind::Syntax,
        message: msg,
        start: p.start as usize,
        line: p.line,
        col: p.col,
    }
}

/// Analyse a parsed program.
pub fn analyse(p: &Program) -> Result<Analysis, Diag> {
    let mut w = Walker {
        a: Analysis {
            scopes: (0..p.n_scopes)
                .map(|_| Scope {
                    kind: ScopeKind::Block,
                    parent: None,
                    func: 0,
                    bindings: Vec::new(),
                })
                .collect(),
            bindings: Vec::new(),
            res: vec![Res::None; p.n_names as usize],
            ref_scope: vec![0; p.n_names as usize],
            name_pos: vec![None; p.n_names as usize],
            shorthand: vec![false; p.n_names as usize],
        },
        cur: 0,
    };
    w.a.scopes[0].kind = ScopeKind::Top;
    w.declare(&p.this, BindKind::Special)?;
    w.hoist_vars(&p.body)?;
    w.declare_lexical(&p.body, true)?;
    w.stmts(&p.body)?;
    Ok(w.a)
}

impl Walker {
    fn open(&mut self, id: u32, kind: ScopeKind) -> u32 {
        let parent = self.cur;
        let func = match kind {
            ScopeKind::Top | ScopeKind::Function { .. } => id,
            _ => self.a.scopes[parent as usize].func,
        };
        let s = &mut self.a.scopes[id as usize];
        s.kind = kind;
        s.parent = Some(parent);
        s.func = func;
        std::mem::replace(&mut self.cur, id)
    }

    fn lookup_in(&self, scope: u32, name: &str) -> Option<u32> {
        self.a.scopes[scope as usize]
            .bindings
            .iter()
            .copied()
            .find(|b| &*self.a.bindings[*b as usize].name == name)
    }

    fn declare(&mut self, n: &Name, kind: BindKind) -> SResult<u32> {
        self.declare_in(self.cur, n, kind)
    }

    fn declare_in(&mut self, scope: u32, n: &Name, kind: BindKind) -> SResult<u32> {
        if let Some(r) = self.a.ref_scope.get_mut(n.id as usize) {
            *r = self.cur;
        }
        if let Some(r) = self.a.name_pos.get_mut(n.id as usize) {
            *r = Some(n.pos);
        }
        if let Some(b) = self.lookup_in(scope, &n.name) {
            let old = self.a.bindings[b as usize].kind;
            let top_level_fn = matches!(
                self.a.scopes[scope as usize].kind,
                ScopeKind::Top | ScopeKind::Function { .. }
            );
            let var_like = |k: BindKind| {
                k == BindKind::Var || k == BindKind::Param || (top_level_fn && k == BindKind::Func)
            };
            if var_like(old) && var_like(kind) {
                // `var` twice, or a `var` next to a parameter or function
                // declaration in the same function: one binding.
                if kind == BindKind::Func {
                    self.a.bindings[b as usize].kind = BindKind::Func;
                }
                self.a.res[n.id as usize] = Res::Binding(b);
                return Ok(b);
            }
            return Err(err(
                format!("`{}` is already declared in this scope", n.name),
                n.pos,
            ));
        }
        let b = self.a.bindings.len() as u32;
        self.a.bindings.push(Binding {
            name: n.name.clone(),
            kind,
            scope,
            captured: false,
            pos: n.pos,
        });
        self.a.scopes[scope as usize].bindings.push(b);
        self.a.res[n.id as usize] = Res::Binding(b);
        Ok(b)
    }

    /// Declare a `var` in the current function scope, checking it doesn't
    /// pass a lexical binding of the same name on the way.
    fn declare_var(&mut self, n: &Name) -> SResult<()> {
        let func = self.a.scopes[self.cur as usize].func;
        let mut s = self.cur;
        while s != func {
            if let Some(b) = self.lookup_in(s, &n.name) {
                let k = self.a.bindings[b as usize].kind;
                if k != BindKind::Catch {
                    return Err(err(
                        format!(
                            "`var {}` conflicts with a block-scoped `{}`",
                            n.name, n.name
                        ),
                        n.pos,
                    ));
                }
            }
            s = self.a.scopes[s as usize].parent.unwrap_or(func);
        }
        self.declare_in(func, n, BindKind::Var)?;
        Ok(())
    }

    fn resolve(&mut self, n: &Name) {
        self.a.ref_scope[n.id as usize] = self.cur;
        self.a.name_pos[n.id as usize] = Some(n.pos);
        let mut s = Some(self.cur);
        while let Some(id) = s {
            if let Some(b) = self.lookup_in(id, &n.name) {
                let here = self.a.scopes[self.cur as usize].func;
                let there = self.a.scopes[id as usize].func;
                if here != there {
                    self.a.bindings[b as usize].captured = true;
                }
                self.a.res[n.id as usize] = Res::Binding(b);
                return;
            }
            s = self.a.scopes[id as usize].parent;
        }
        self.a.res[n.id as usize] = Res::Free;
    }

    /// Mark the names of `var` declarations in `stmts` (not inside nested
    /// functions). They are declared when walked, in [`Walker::declare_var`];
    /// this pass declares them up front so earlier references resolve.
    fn hoist_vars(&mut self, stmts: &[Stmt]) -> SResult<()> {
        for s in stmts {
            self.hoist_stmt(s)?;
        }
        Ok(())
    }

    fn hoist_stmt(&mut self, s: &Stmt) -> SResult<()> {
        let func = self.a.scopes[self.cur as usize].func;
        let decl = |w: &mut Self, n: &Name| -> SResult<()> {
            if w.lookup_in(func, &n.name).is_none() {
                w.declare_in(func, n, BindKind::Var)?;
            }
            Ok(())
        };
        match &s.kind {
            StmtKind::Var(VarKind::Var, ds) => {
                for (n, _) in ds {
                    decl(self, n)?;
                }
            }
            StmtKind::For { init, body, .. } => {
                if let Some(ForInit::Var(VarKind::Var, ds)) = init {
                    for (n, _) in ds {
                        decl(self, n)?;
                    }
                }
                self.hoist_stmt(body)?;
            }
            StmtKind::ForOf {
                decl: Some(VarKind::Var),
                target,
                body,
                ..
            } => {
                decl(self, target)?;
                self.hoist_stmt(body)?;
            }
            StmtKind::ForOf { body, .. }
            | StmtKind::While(_, body)
            | StmtKind::DoWhile(body, _)
            | StmtKind::Labeled(_, body) => self.hoist_stmt(body)?,
            StmtKind::If(_, a, b) => {
                self.hoist_stmt(a)?;
                if let Some(b) = b {
                    self.hoist_stmt(b)?;
                }
            }
            StmtKind::Block(b) => self.hoist_vars(&b.stmts)?,
            StmtKind::Try(b, c, f) => {
                self.hoist_vars(&b.stmts)?;
                if let Some(c) = c {
                    self.hoist_vars(&c.body.stmts)?;
                }
                if let Some(f) = f {
                    self.hoist_vars(&f.stmts)?;
                }
            }
            StmtKind::Switch { cases, .. } => {
                for c in cases {
                    self.hoist_vars(&c.body)?;
                }
            }
            StmtKind::Export(inner) => self.hoist_stmt(inner)?,
            _ => {}
        }
        Ok(())
    }

    /// Declare the lexical names of a statement list in the current scope.
    fn declare_lexical(&mut self, stmts: &[Stmt], fn_top: bool) -> SResult<()> {
        for s in stmts {
            let s = match &s.kind {
                StmtKind::Export(inner) => inner,
                _ => s,
            };
            match &s.kind {
                StmtKind::Var(k @ (VarKind::Let | VarKind::Const), ds) => {
                    let kind = if *k == VarKind::Let {
                        BindKind::Let
                    } else {
                        BindKind::Const
                    };
                    for (n, _) in ds {
                        self.declare(n, kind)?;
                    }
                }
                StmtKind::Class(c) => {
                    if let Some(n) = &c.name {
                        self.declare(n, BindKind::Class)?;
                    }
                }
                StmtKind::Func(f) => {
                    if let Some(n) = &f.name {
                        let _ = fn_top;
                        self.declare(n, BindKind::Func)?;
                    }
                }
                StmtKind::Import(names, _) => {
                    for i in names {
                        self.declare(&i.local, BindKind::Import)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn block(&mut self, b: &Block) -> SResult<()> {
        let saved = self.open(b.scope, ScopeKind::Block);
        // A catch body may not redeclare the catch parameter.
        let parent = self.a.scopes[b.scope as usize].parent;
        self.declare_lexical(&b.stmts, false)?;
        if let Some(p) = parent {
            if self.a.scopes[p as usize].kind == ScopeKind::Catch {
                for &pb in &self.a.scopes[p as usize].bindings.clone() {
                    let name = self.a.bindings[pb as usize].name.clone();
                    if let Some(lb) = self.lookup_in(b.scope, &name) {
                        let pos = self.a.bindings[lb as usize].pos;
                        return Err(err(
                            format!("`{name}` is already declared as the catch parameter"),
                            pos,
                        ));
                    }
                }
            }
        }
        self.stmts(&b.stmts)?;
        self.cur = saved;
        Ok(())
    }

    fn stmts(&mut self, stmts: &[Stmt]) -> SResult<()> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn var_decls(
        &mut self,
        kind: VarKind,
        ds: &[(Name, Option<Expr>)],
        declared: bool,
    ) -> SResult<()> {
        for (n, init) in ds {
            if let Some(e) = init {
                self.expr(e)?;
            }
            if kind == VarKind::Var {
                self.declare_var(n)?;
            } else if !declared {
                let k = if kind == VarKind::Let {
                    BindKind::Let
                } else {
                    BindKind::Const
                };
                self.declare(n, k)?;
            }
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> SResult<()> {
        match &s.kind {
            StmtKind::Expr(e) | StmtKind::Throw(e) => self.expr(e)?,
            StmtKind::Var(k, ds) => self.var_decls(*k, ds, true)?,
            StmtKind::Func(f) => self.func(f)?,
            StmtKind::Class(c) => self.class(c)?,
            StmtKind::Block(b) => self.block(b)?,
            StmtKind::If(t, a, b) => {
                self.expr(t)?;
                self.stmt(a)?;
                if let Some(b) = b {
                    self.stmt(b)?;
                }
            }
            StmtKind::While(t, b) => {
                self.expr(t)?;
                self.stmt(b)?;
            }
            StmtKind::DoWhile(b, t) => {
                self.stmt(b)?;
                self.expr(t)?;
            }
            StmtKind::For {
                scope,
                init,
                test,
                update,
                body,
            } => {
                let saved = self.open(*scope, ScopeKind::Block);
                match init {
                    Some(ForInit::Var(k, ds)) => self.var_decls(*k, ds, false)?,
                    Some(ForInit::Expr(e)) => self.expr(e)?,
                    None => {}
                }
                if let Some(t) = test {
                    self.expr(t)?;
                }
                if let Some(u) = update {
                    self.expr(u)?;
                }
                self.stmt(body)?;
                self.cur = saved;
            }
            StmtKind::ForOf {
                scope,
                decl,
                target,
                iter,
                body,
            } => {
                // The array is evaluated with the loop's names in their
                // dead zone, as the spec's TDZ scope for the head does.
                let saved = self.open(*scope, ScopeKind::Block);
                match decl {
                    Some(VarKind::Var) => {
                        self.cur = saved;
                        self.declare_var(target)?;
                        self.cur = *scope;
                    }
                    Some(k) => {
                        let bk = if *k == VarKind::Let {
                            BindKind::Let
                        } else {
                            BindKind::Const
                        };
                        self.declare(target, bk)?;
                    }
                    None => self.resolve(target),
                }
                self.expr(iter)?;
                self.stmt(body)?;
                self.cur = saved;
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e)?;
                }
            }
            StmtKind::Try(b, c, f) => {
                self.block(b)?;
                if let Some(c) = c {
                    let saved = self.open(c.scope, ScopeKind::Catch);
                    if let Some(p) = &c.param {
                        self.declare(p, BindKind::Catch)?;
                    }
                    self.block(&c.body)?;
                    self.cur = saved;
                }
                if let Some(f) = f {
                    self.block(f)?;
                }
            }
            StmtKind::Switch { disc, scope, cases } => {
                self.expr(disc)?;
                let saved = self.open(*scope, ScopeKind::Block);
                for c in cases {
                    self.declare_lexical(&c.body, false)?;
                }
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(t)?;
                    }
                    self.stmts(&c.body)?;
                }
                self.cur = saved;
            }
            StmtKind::Labeled(_, b) => self.stmt(b)?,
            StmtKind::Export(inner) => self.stmt(inner)?,
            StmtKind::Break(_)
            | StmtKind::Continue(_)
            | StmtKind::Empty
            | StmtKind::Import(..)
            | StmtKind::Invalid => {}
        }
        Ok(())
    }

    fn func(&mut self, f: &Func) -> SResult<()> {
        let outer = self.cur;
        if let Some(ns) = f.name_scope {
            self.open(ns, ScopeKind::FuncName);
            if let Some(n) = &f.name {
                self.declare(n, BindKind::FuncName)?;
            }
        }
        self.open(
            f.scope,
            ScopeKind::Function {
                arrow: f.kind == FuncKind::Arrow,
            },
        );
        if let Some(sp) = &f.specials {
            for n in sp {
                self.declare(n, BindKind::Special)?;
            }
        }
        for p in &f.params {
            self.declare(p, BindKind::Param)?;
        }
        self.hoist_vars(&f.body)?;
        self.declare_lexical(&f.body, true)?;
        self.stmts(&f.body)?;
        self.cur = outer;
        Ok(())
    }

    fn class(&mut self, c: &Class) -> SResult<()> {
        if let Some(e) = &c.extends {
            // The heritage is evaluated inside the class scope (so the
            // class name is in its dead zone there).
            let saved = self.open(c.scope, ScopeKind::Class);
            if let Some(n) = &c.name {
                let inner = Name {
                    id: u32::MAX,
                    ..n.clone()
                };
                self.declare_inner(&inner)?;
            }
            self.expr(e)?;
            self.cur = saved;
        }
        let saved = if c.extends.is_some() {
            std::mem::replace(&mut self.cur, c.scope)
        } else {
            let s = self.open(c.scope, ScopeKind::Class);
            if let Some(n) = &c.name {
                let inner = Name {
                    id: u32::MAX,
                    ..n.clone()
                };
                self.declare_inner(&inner)?;
            }
            s
        };
        if let Some(k) = &c.ctor {
            self.func(k)?;
        }
        for m in &c.methods {
            self.func(&m.func)?;
        }
        self.cur = saved;
        Ok(())
    }

    fn declare_inner(&mut self, n: &Name) -> SResult<()> {
        let b = self.a.bindings.len() as u32;
        self.a.bindings.push(Binding {
            name: n.name.clone(),
            kind: BindKind::ClassInner,
            scope: self.cur,
            captured: false,
            pos: n.pos,
        });
        self.a.scopes[self.cur as usize].bindings.push(b);
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> SResult<()> {
        match &e.kind {
            ExprKind::Num(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::Null
            | ExprKind::Invalid => {}
            ExprKind::Template(_, es) | ExprKind::Array(es) | ExprKind::Seq(es) => {
                for x in es {
                    self.expr(x)?;
                }
            }
            ExprKind::Ident(n) | ExprKind::This(n) => self.resolve(n),
            ExprKind::Object(props) => {
                for p in props {
                    if p.shorthand {
                        if let ExprKind::Ident(n) = &p.value.kind {
                            self.a.shorthand[n.id as usize] = true;
                        }
                    }
                    self.expr(&p.value)?;
                }
            }
            ExprKind::Function(f) => self.func(f)?,
            ExprKind::Class(c) => self.class(c)?,
            ExprKind::Unary(_, a) | ExprKind::Update { target: a, .. } => self.expr(a)?,
            ExprKind::Binary(_, a, b) | ExprKind::Logical(_, a, b) | ExprKind::Assign(_, a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::Index(a, b) => {
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::Cond(a, b, c) => {
                self.expr(a)?;
                self.expr(b)?;
                self.expr(c)?;
            }
            ExprKind::Call(c, args) | ExprKind::New(c, args) => {
                self.expr(c)?;
                for x in args {
                    self.expr(x)?;
                }
            }
            ExprKind::Member(o, _) => self.expr(o)?,
            ExprKind::SuperCall(r, args) => {
                self.super_refs(r);
                for x in args {
                    self.expr(x)?;
                }
            }
            ExprKind::SuperMember(r, _) => self.super_refs(r),
        }
        Ok(())
    }

    fn super_refs(&mut self, r: &SuperRefs) {
        self.resolve(&r.this);
        self.resolve(&r.func);
        self.resolve(&r.nt);
    }
}
