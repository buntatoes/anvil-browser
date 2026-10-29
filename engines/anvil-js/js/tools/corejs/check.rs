// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Core JS rules that need the whole tree: which globals exist, how the
//! built-in namespaces may be used, and the levels (which construct a file
//! may use, given what the VM it runs on provides).
//!
//! The level check is syntactic. Rules that depend on run-time types (a
//! `<` between two strings, `s[i]` on a string, implicit string and number
//! mixing, `===` on two string or two function values held in variables)
//! can't be seen here; the stand-in host runs the code with the language's
//! real semantics instead.

use super::ast::*;
use super::lexer::{Diag, DiagKind};
use super::scope::{Analysis, BindKind, Res};

/// What a file may use, from the plainest code to everything. Each level
/// allows everything the ones before it allow.
pub type Level = u8;

/// Top-level functions, numbers, arrays, records and loops.
pub const PLAIN: Level = 0;
/// Nested functions, functions as values, captured variables, and
/// `console.log`.
pub const CLOSURES: Level = 1;
/// `this`, `new`, classes, `instanceof` and `super`.
pub const OBJECTS: Level = 2;
/// Strings: templates, concatenation, comparison, methods.
pub const STRINGS: Level = 3;
/// `throw`, `try` and the error constructors.
pub const ERRORS: Level = 4;
/// `switch`, labels, the comma operator, `typeof`, `in`, `for...of`,
/// `**`, `NaN` and `Infinity`, and updates of properties.
pub const SYNTAX: Level = 5;
/// `Map`, `Set` and typed arrays.
pub const COLLECTIONS: Level = 6;
/// The host object: everything.
pub const HOST: Level = 7;

/// Everything (no gate).
pub const LEVEL_ALL: Level = HOST;

/// The levels' names, as written in MODULES.txt and test files.
pub const LEVEL_NAMES: [&str; 8] = [
    "plain",
    "closures",
    "objects",
    "strings",
    "errors",
    "syntax",
    "collections",
    "host",
];

/// Parse a level name.
pub fn parse_level(s: &str) -> Option<Level> {
    LEVEL_NAMES.iter().position(|n| *n == s).map(|i| i as Level)
}

/// A level's name.
pub fn level_name(l: Level) -> &'static str {
    LEVEL_NAMES[usize::from(l.min(HOST))]
}

/// A global name, the level it needs, and the members allowed on it (empty
/// for a plain value or constructor).
pub const GLOBALS: &[(&str, Level, &[&str])] = &[
    ("undefined", PLAIN, &[]),
    // The names `NaN` and `Infinity` arrive with `typeof` (the syntax level). Before
    // that, test for NaN with `x !== x` and build infinities from numbers.
    ("NaN", SYNTAX, &[]),
    ("Infinity", SYNTAX, &[]),
    (
        "Math",
        PLAIN,
        &[
            "floor", "ceil", "round", "abs", "sqrt", "min", "max", "pow", "trunc", "sign", "imul",
        ],
    ),
    ("Number", PLAIN, &["isInteger", "isNaN", "isFinite"]),
    ("isNaN", PLAIN, &[]),
    ("isFinite", PLAIN, &[]),
    ("String", STRINGS, &["fromCharCode"]),
    ("Error", ERRORS, &[]),
    ("TypeError", ERRORS, &[]),
    ("RangeError", ERRORS, &[]),
    ("Map", COLLECTIONS, &[]),
    ("Set", COLLECTIONS, &[]),
    ("ArrayBuffer", COLLECTIONS, &[]),
    ("Uint8Array", COLLECTIONS, &[]),
    ("Uint16Array", COLLECTIONS, &[]),
    ("Int32Array", COLLECTIONS, &[]),
    ("Uint32Array", COLLECTIONS, &[]),
    ("Float64Array", COLLECTIONS, &[]),
    // A minimal `console.log` comes with closures, ahead of the rest of
    // the host.
    ("console", CLOSURES, &["log"]),
    ("host", HOST, &[]),
];

/// Built-ins the VM does not have yet: they come separately from the
/// levels. A module may use them only when it is
/// marked `builtins=yes` in MODULES.txt (the stand-in has them all).
/// `Math.floor`, `ceil`, `round`, `abs` and `sqrt` are already there.
pub const NEEDS_EXTRA_BUILTINS: &[&str] = &[
    "Math.min",
    "Math.max",
    "Math.pow",
    "Math.trunc",
    "Math.sign",
    "Math.imul",
    "Number.isInteger",
    "Number.isNaN",
    "Number.isFinite",
    "isNaN",
    "isFinite",
];

/// String methods, which need the strings level.
const STRING_METHODS: &[&str] = &["charCodeAt", "slice", "substring", "indexOf"];

struct Checker<'a> {
    a: &'a Analysis,
    level: Level,
    builtins: bool,
    out: Vec<Diag>,
    fn_depth: u32,
}

/// Every subset and level problem in `p`, in source order. `level` is the
/// file's declared level ([`LEVEL_ALL`] for no gate); `builtins` allows
/// the names in [`NEEDS_EXTRA_BUILTINS`].
pub fn check(p: &Program, a: &Analysis, level: Level, builtins: bool) -> Vec<Diag> {
    let mut c = Checker {
        a,
        level,
        builtins,
        out: Vec::new(),
        fn_depth: 0,
    };
    c.stmts(&p.body);
    c.out.sort_by_key(|d| d.start);
    c.out
}

impl Checker<'_> {
    fn subset(&mut self, msg: String, p: Pos) {
        self.out.push(Diag {
            kind: DiagKind::Subset,
            message: msg,
            start: p.start as usize,
            line: p.line,
            col: p.col,
        });
    }

    fn need(&mut self, t: Level, what: &str, p: Pos) {
        if t > self.level {
            self.out.push(Diag {
                kind: DiagKind::Level,
                message: format!(
                    "{what} needs level {} (this file is level {})",
                    level_name(t),
                    level_name(self.level)
                ),
                start: p.start as usize,
                line: p.line,
                col: p.col,
            });
        }
    }

    fn builtin(&mut self, name: &str, p: Pos) {
        if !self.builtins && NEEDS_EXTRA_BUILTINS.contains(&name) {
            self.out.push(Diag {
                kind: DiagKind::Level,
                message: format!(
                    "`{name}` is one of the extra built-ins (mark the module `builtins=yes` in MODULES.txt)"
                ),
                start: p.start as usize,
                line: p.line,
                col: p.col,
            });
        }
    }

    fn stmts(&mut self, ss: &[Stmt]) {
        for s in ss {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Expr(e) => self.expr(e, false),
            StmtKind::Var(_, ds) => {
                for (_, i) in ds {
                    if let Some(e) = i {
                        self.expr(e, false);
                    }
                }
            }
            StmtKind::Func(f) => {
                if self.fn_depth > 0 {
                    self.need(CLOSURES, "a nested function", s.pos);
                }
                self.func(f);
            }
            StmtKind::Class(c) => self.class(c, s.pos),
            StmtKind::Block(b) => self.stmts(&b.stmts),
            StmtKind::If(t, a, b) => {
                self.expr(t, false);
                self.stmt(a);
                if let Some(b) = b {
                    self.stmt(b);
                }
            }
            StmtKind::While(t, b) | StmtKind::DoWhile(b, t) => {
                self.expr(t, false);
                self.stmt(b);
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
                ..
            } => {
                match init {
                    Some(ForInit::Var(_, ds)) => {
                        for (_, i) in ds {
                            if let Some(e) = i {
                                self.expr(e, false);
                            }
                        }
                    }
                    Some(ForInit::Expr(e)) => self.expr(e, false),
                    None => {}
                }
                if let Some(t) = test {
                    self.expr(t, false);
                }
                if let Some(u) = update {
                    self.expr(u, false);
                }
                self.stmt(body);
            }
            StmtKind::ForOf { iter, body, .. } => {
                self.need(SYNTAX, "`for...of`", s.pos);
                self.expr(iter, false);
                self.stmt(body);
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e, false);
                }
            }
            StmtKind::Break(l) | StmtKind::Continue(l) => {
                if l.is_some() {
                    self.need(SYNTAX, "a labeled jump", s.pos);
                }
            }
            StmtKind::Throw(e) => {
                self.need(ERRORS, "`throw`", s.pos);
                self.expr(e, false);
            }
            StmtKind::Try(b, c, f) => {
                self.need(ERRORS, "`try`", s.pos);
                self.stmts(&b.stmts);
                if let Some(c) = c {
                    self.stmts(&c.body.stmts);
                }
                if let Some(f) = f {
                    self.stmts(&f.stmts);
                }
            }
            StmtKind::Switch { disc, cases, .. } => {
                self.need(SYNTAX, "`switch`", s.pos);
                self.expr(disc, false);
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(t, false);
                    }
                    self.stmts(&c.body);
                }
            }
            StmtKind::Labeled(_, b) => {
                self.need(SYNTAX, "a label", s.pos);
                self.stmt(b);
            }
            StmtKind::Export(inner) => self.stmt(inner),
            StmtKind::Empty | StmtKind::Import(..) | StmtKind::Invalid => {}
        }
    }

    fn func(&mut self, f: &Func) {
        self.fn_depth += 1;
        self.stmts(&f.body);
        self.fn_depth -= 1;
    }

    fn class(&mut self, c: &Class, pos: Pos) {
        self.need(OBJECTS, "`class`", pos);
        if let Some(e) = &c.extends {
            self.expr(e, false);
        }
        if let Some(k) = &c.ctor {
            self.func(k);
        }
        for m in &c.methods {
            self.func(&m.func);
        }
    }

    fn is_string_lit(e: &Expr) -> bool {
        matches!(e.kind, ExprKind::Str(_) | ExprKind::Template(..))
    }

    fn global_entry(n: &str) -> Option<&'static (&'static str, Level, &'static [&'static str])> {
        GLOBALS.iter().find(|g| g.0 == n)
    }

    /// A free name. `member` is the property read from it, if any.
    fn free_name(&mut self, n: &Name, member: Option<&PropName>) {
        let Some(&(_, level, members)) = Self::global_entry(&n.name) else {
            let msg = match &*n.name {
                "eval" | "arguments" | "Proxy" | "Reflect" | "Symbol" | "Date" => {
                    format!("`{}` is outside Core JS", n.name)
                }
                _ => format!(
                    "`{}` is not declared, and it is not one of the globals Core JS has",
                    n.name
                ),
            };
            self.subset(msg, n.pos);
            return;
        };
        self.need(level, &format!("`{}`", n.name), n.pos);
        if members.is_empty() {
            self.builtin(&n.name, n.pos);
        } else {
            match member {
                Some(m) if members.contains(&&*m.text) => {
                    self.builtin(&format!("{}.{}", n.name, m.text), n.pos);
                }
                Some(m) => self.subset(
                    format!(
                        "`{}.{}` is outside Core JS (only {} are)",
                        n.name,
                        m.text,
                        members
                            .iter()
                            .map(|x| format!("`{}.{x}`", n.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    m.pos,
                ),
                None => self.subset(
                    format!("`{}` may only be used to reach its members", n.name),
                    n.pos,
                ),
            }
        }
    }

    fn ident(&mut self, n: &Name, callee: bool, member: Option<&PropName>) {
        match self.a.res.get(n.id as usize) {
            Some(Res::Free) => self.free_name(n, member),
            Some(Res::Binding(b)) => {
                let b = &self.a.bindings[*b as usize];
                if b.kind == BindKind::Func && !callee {
                    self.need(CLOSURES, "a function used as a value", n.pos);
                }
                // Top-level names are globals once bundled, not captures.
                if b.captured && self.a.scopes[b.scope as usize].func != 0 {
                    self.need(CLOSURES, "a captured variable", n.pos);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, callee: bool) {
        match &e.kind {
            ExprKind::Num(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::Null
            | ExprKind::Invalid => {}
            ExprKind::Template(_, es) => {
                self.need(STRINGS, "a template literal", e.pos);
                for x in es {
                    self.expr(x, false);
                }
            }
            ExprKind::Array(es) => {
                for x in es {
                    self.expr(x, false);
                }
            }
            ExprKind::Seq(es) => {
                self.need(SYNTAX, "the comma operator", e.pos);
                for x in es {
                    self.expr(x, false);
                }
            }
            ExprKind::Ident(n) => self.ident(n, callee, None),
            ExprKind::This(_) => self.need(OBJECTS, "`this`", e.pos),
            ExprKind::Object(props) => {
                for p in props {
                    self.expr(&p.value, false);
                }
            }
            ExprKind::Function(f) => {
                let what = if f.kind == FuncKind::Arrow {
                    "an arrow function"
                } else {
                    "a function expression"
                };
                self.need(CLOSURES, what, e.pos);
                self.func(f);
            }
            ExprKind::Class(c) => self.class(c, e.pos),
            ExprKind::Unary(op, a) => {
                if *op == UnOp::Typeof {
                    self.need(SYNTAX, "`typeof`", e.pos);
                }
                self.expr(a, false);
            }
            ExprKind::Update { target, .. } => {
                if !matches!(target.kind, ExprKind::Ident(_)) {
                    self.need(SYNTAX, "`++`/`--` on a property", e.pos);
                }
                self.expr(target, false);
            }
            ExprKind::Binary(op, a, b) => {
                match op {
                    BinOp::Instanceof => self.need(OBJECTS, "`instanceof`", e.pos),
                    BinOp::In => self.need(SYNTAX, "`in`", e.pos),
                    BinOp::Exp => self.need(SYNTAX, "`**`", e.pos),
                    BinOp::Add if Self::is_string_lit(a) || Self::is_string_lit(b) => {
                        self.need(STRINGS, "string concatenation", e.pos)
                    }
                    // `===` on strings needs the strings level and on functions
                    // closures. Only a string literal operand can be seen here;
                    // the rest depends on run-time types.
                    BinOp::StrictEq | BinOp::StrictNe
                        if Self::is_string_lit(a) || Self::is_string_lit(b) =>
                    {
                        self.need(STRINGS, "`===` or `!==` on strings", e.pos)
                    }
                    _ => {}
                }
                self.expr(a, false);
                self.expr(b, false);
            }
            ExprKind::Logical(_, a, b) => {
                self.expr(a, false);
                self.expr(b, false);
            }
            ExprKind::Assign(op, t, v) => {
                if op.is_some() && !matches!(t.kind, ExprKind::Ident(_)) {
                    self.need(SYNTAX, "compound assignment to a property", e.pos);
                }
                if *op == Some(BinOp::Exp) {
                    self.need(SYNTAX, "`**=`", e.pos);
                }
                if *op == Some(BinOp::Add) && Self::is_string_lit(v) {
                    self.need(STRINGS, "string concatenation", e.pos);
                }
                if let ExprKind::Ident(n) = &t.kind {
                    if let Some(b) = self.a.binding_of(n) {
                        if b.kind == BindKind::Func {
                            self.subset(
                                format!(
                                    "assigning to the function `{}` is outside Core JS",
                                    n.name
                                ),
                                n.pos,
                            );
                        }
                    }
                }
                self.expr(t, false);
                self.expr(v, false);
            }
            ExprKind::Cond(a, b, c) => {
                self.expr(a, false);
                self.expr(b, false);
                self.expr(c, false);
            }
            ExprKind::Call(c, args) => {
                self.expr(c, true);
                for x in args {
                    self.expr(x, false);
                }
            }
            ExprKind::New(c, args) => {
                self.need(OBJECTS, "`new`", e.pos);
                self.expr(c, true);
                for x in args {
                    self.expr(x, false);
                }
            }
            ExprKind::Member(o, name) => {
                if STRING_METHODS.contains(&&*name.text) {
                    self.need(STRINGS, &format!("`.{}`", name.text), name.pos);
                }
                if let ExprKind::Ident(n) = &o.kind {
                    self.ident(n, false, Some(name));
                } else {
                    self.expr(o, false);
                }
            }
            ExprKind::Index(o, i) => {
                self.expr(o, false);
                self.expr(i, false);
            }
            ExprKind::SuperCall(_, args) => {
                self.need(OBJECTS, "`super`", e.pos);
                for x in args {
                    self.expr(x, false);
                }
            }
            ExprKind::SuperMember(..) => self.need(OBJECTS, "`super`", e.pos),
        }
    }
}
