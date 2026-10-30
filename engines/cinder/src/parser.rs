// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Recursive-descent parser for ES2022-level scripts and modules.
//!
//! Statements use plain recursive descent. Binary operators use precedence
//! climbing. Arrow parameters and destructuring assignment are parsed as
//! expressions first and turned into patterns once the parser sees `=>` or
//! `=` (the spec's "cover grammar").
//!
//! The parser also runs the common early errors: duplicate lexical names,
//! bad assignment targets, `break`/`continue` labels, `yield` and `await`
//! context, strict-mode reserved words and octal literals, duplicate or
//! misplaced parameters, private names that were never declared, and module
//! import/export rules.
//!
//! Limits, both set in [`ParseOptions`]:
//!
//! - Source size. Longer input is rejected up front.
//! - Nesting depth. Every recursive step, and every link in a long
//!   left-nested chain such as `a.b.c` or `1 + 2 + 3`, counts one level. The
//!   syntax tree is never deeper than the limit, so later passes can rely on
//!   it too. Going over returns an error instead of using up the stack.

use std::collections::{HashMap, HashSet};

use crate::cinder::ast::*;
use crate::cinder::error::{Span, SyntaxError};
use crate::cinder::lexer::{
    Goal, Lexer, Punct, TemplatePart, Token, TokenKind, is_always_reserved, is_strict_reserved,
};

/// Default cap on source size: 640 MiB.
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 640 * 1024 * 1024;

/// Default cap on nesting depth. See the crate README for the stack each
/// level needs.
pub const DEFAULT_MAX_DEPTH: u32 = 10_000;

/// Parser settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseOptions {
    /// Largest source text accepted, in bytes.
    pub max_source_bytes: usize,
    /// Deepest nesting accepted.
    pub max_depth: u32,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

/// Stack reserved for each level of nesting on the parser's own thread.
/// Measured worst cases are about 25 KiB per level in debug builds and about
/// 5 KiB in release builds; these leave room on top of that.
#[cfg(debug_assertions)]
const STACK_PER_LEVEL: usize = 64 * 1024;
#[cfg(not(debug_assertions))]
const STACK_PER_LEVEL: usize = 16 * 1024;

/// Fixed stack on top of the per-level amount.
const STACK_BASE: usize = 1024 * 1024;

/// Parse `src` as a classic script.
pub fn parse_script(src: &str, opts: &ParseOptions) -> Result<Program, SyntaxError> {
    run(src, Goal::Script, opts)
}

/// Parse `src` as a module.
pub fn parse_module(src: &str, opts: &ParseOptions) -> Result<Program, SyntaxError> {
    run(src, Goal::Module, opts)
}

/// Run the parser on its own thread, with a stack sized from
/// `opts.max_depth`. The depth limit then always fires before the stack runs
/// out, whatever stack the caller happens to be on. A panic inside the
/// parser (which would be a bug) comes back as an error, not a crash.
fn run(src: &str, goal: Goal, opts: &ParseOptions) -> Result<Program, SyntaxError> {
    let at_start = Span {
        start: 0,
        end: 0,
        line: 1,
        column: 1,
    };
    if src.len() > opts.max_source_bytes {
        return Err(SyntaxError::new(
            format!(
                "source is {} bytes, over the {} byte limit",
                src.len(),
                opts.max_source_bytes
            ),
            at_start,
        ));
    }
    let stack = (opts.max_depth as usize)
        .saturating_mul(STACK_PER_LEVEL)
        .saturating_add(STACK_BASE);
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("cinder-parser".into())
            .stack_size(stack)
            .spawn_scoped(scope, || Parser::new(src, goal, opts)?.parse_program());
        match spawned {
            Ok(handle) => match handle.join() {
                Ok(result) => result,
                Err(_) => Err(SyntaxError::new("internal parser error", at_start)),
            },
            Err(e) => Err(SyntaxError::new(
                format!("could not start the parser thread ({e}); try a lower max_depth"),
                at_start,
            )),
        }
    })
}

type PResult<T> = Result<T, SyntaxError>;

#[derive(Clone, Debug)]
struct Label {
    name: String,
    is_loop: bool,
}

/// Per-function context. Saved and restored around every function body.
#[derive(Clone, Debug, Default)]
struct Ctx {
    strict: bool,
    in_function: bool,
    is_async: bool,
    is_generator: bool,
    super_prop: bool,
    super_call: bool,
    new_target: bool,
    /// Inside a class field initializer or static block: no `arguments`.
    class_init: bool,
    /// Inside a class static block: `await` is reserved.
    static_block: bool,
    labels: Vec<Label>,
    loop_depth: u32,
    switch_depth: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    Function,
    Block,
}

#[derive(Debug)]
struct Scope {
    kind: ScopeKind,
    lexical: HashSet<String>,
    vars: HashSet<String>,
    annex_fns: HashSet<String>,
    params: HashSet<String>,
    catch_params: HashSet<String>,
    catch_simple: bool,
}

impl Scope {
    fn new(kind: ScopeKind) -> Self {
        Scope {
            kind,
            lexical: HashSet::new(),
            vars: HashSet::new(),
            annex_fns: HashSet::new(),
            params: HashSet::new(),
            catch_params: HashSet::new(),
            catch_simple: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivKind {
    Field,
    Method,
    Get,
    Set,
    GetSet,
}

#[derive(Debug, Default)]
struct ClassPrivates {
    declared: HashMap<String, (PrivKind, bool)>,
    refs: Vec<(String, Span)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    Normal,
    Method,
    Getter,
    Setter,
    Constructor { derived: bool },
}

impl Flavor {
    fn is_method(self) -> bool {
        !matches!(self, Flavor::Normal)
    }
}

struct Parser<'a> {
    lx: Lexer<'a>,
    src: &'a str,
    goal: Goal,
    cur: Token,
    prev_end: usize,
    max_depth: u32,
    depth: u32,
    no_in: bool,
    ctx: Ctx,
    scopes: Vec<Scope>,
    classes: Vec<ClassPrivates>,
    cover_err: Option<SyntaxError>,
    last_yield_await: Option<usize>,
    last_await_ident: Option<usize>,
    bad_rest: HashSet<usize>,
    exported: HashSet<String>,
    export_locals: Vec<(String, Span)>,
}

fn starts_key(t: &Token, allow_private: bool) -> bool {
    match &t.kind {
        TokenKind::Ident(_)
        | TokenKind::String(_)
        | TokenKind::Number(_)
        | TokenKind::BigInt(_) => true,
        TokenKind::PrivateName(_) => allow_private,
        TokenKind::Punct(p) => *p == Punct::LBracket,
        _ => false,
    }
}

fn describe(t: &Token) -> String {
    match &t.kind {
        TokenKind::Ident(n) => format!("`{n}`"),
        TokenKind::PrivateName(n) => format!("`#{n}`"),
        TokenKind::Punct(p) => format!("`{}`", p.as_str()),
        TokenKind::Number(_) | TokenKind::BigInt(_) => "number".into(),
        TokenKind::String(_) => "string".into(),
        TokenKind::Template(_) => "template".into(),
        TokenKind::Regex { .. } => "regular expression".into(),
        TokenKind::Eof => "end of input".into(),
    }
}

fn unparen(e: &Expr) -> &Expr {
    let mut e = e;
    while let ExprKind::Paren(inner) = &e.kind {
        e = inner;
    }
    e
}

fn is_arrow(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Arrow(_))
}

fn key_name(key: &PropKey) -> Option<String> {
    match key {
        PropKey::Ident(i) => Some(i.name.clone()),
        PropKey::Str(s, _) => Some(String::from_utf16_lossy(s)),
        _ => None,
    }
}

fn well_formed(units: &[u16]) -> bool {
    char::decode_utf16(units.iter().copied()).all(|r| r.is_ok())
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, goal: Goal, opts: &ParseOptions) -> PResult<Self> {
        if src.len() > opts.max_source_bytes {
            return Err(SyntaxError::new(
                format!(
                    "source is {} bytes, over the {} byte limit",
                    src.len(),
                    opts.max_source_bytes
                ),
                Span {
                    start: 0,
                    end: 0,
                    line: 1,
                    column: 1,
                },
            ));
        }
        let mut lx = Lexer::new(src, goal);
        let cur = lx.next_token()?;
        let ctx = Ctx {
            strict: goal == Goal::Module,
            ..Ctx::default()
        };
        Ok(Parser {
            lx,
            src,
            goal,
            cur,
            prev_end: 0,
            max_depth: opts.max_depth,
            depth: 0,
            no_in: false,
            ctx,
            scopes: Vec::new(),
            classes: Vec::new(),
            cover_err: None,
            last_yield_await: None,
            last_await_ident: None,
            bad_rest: HashSet::new(),
            exported: HashSet::new(),
            export_locals: Vec::new(),
        })
    }

    // ---- token helpers ----

    fn bump(&mut self) -> PResult<Token> {
        let next = self.lx.next_token()?;
        self.prev_end = self.cur.span.end;
        Ok(std::mem::replace(&mut self.cur, next))
    }

    fn peek(&mut self) -> PResult<Token> {
        let saved = self.lx.save();
        let t = self.lx.next_token();
        self.lx.restore(saved);
        t
    }

    fn at(&self, p: Punct) -> bool {
        self.cur.is_punct(p)
    }

    fn at_word(&self, w: &str) -> bool {
        self.cur.is_word(w)
    }

    fn eat(&mut self, p: Punct) -> PResult<bool> {
        if self.at(p) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, p: Punct) -> PResult<Token> {
        if self.at(p) {
            self.bump()
        } else {
            Err(SyntaxError::new(
                format!(
                    "expected `{}` but found {}",
                    p.as_str(),
                    describe(&self.cur)
                ),
                self.cur.span,
            ))
        }
    }

    fn expect_word(&mut self, w: &str) -> PResult<Token> {
        if self.at_word(w) {
            self.bump()
        } else {
            Err(SyntaxError::new(
                format!("expected `{w}` but found {}", describe(&self.cur)),
                self.cur.span,
            ))
        }
    }

    fn unexpected<T>(&self) -> PResult<T> {
        Err(SyntaxError::new(
            format!("unexpected {}", describe(&self.cur)),
            self.cur.span,
        ))
    }

    fn err<T>(&self, msg: impl Into<String>, span: Span) -> PResult<T> {
        Err(SyntaxError::new(msg, span))
    }

    fn finish(&self, start: Span) -> Span {
        Span {
            end: self.prev_end.max(start.start),
            ..start
        }
    }

    fn semicolon(&mut self) -> PResult<()> {
        if self.eat(Punct::Semicolon)? {
            return Ok(());
        }
        if self.at(Punct::RBrace) || self.cur.kind == TokenKind::Eof || self.cur.newline_before {
            return Ok(());
        }
        Err(SyntaxError::new(
            format!("expected `;` but found {}", describe(&self.cur)),
            self.cur.span,
        ))
    }

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return self.err(
                format!("nesting is too deep (limit {})", self.max_depth),
                self.cur.span,
            );
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn check_legacy(&self, tok: &Token) -> PResult<()> {
        if tok.legacy_octal && self.ctx.strict {
            return self.err(
                "legacy octal literals and escapes are not allowed in strict mode",
                tok.span,
            );
        }
        Ok(())
    }

    fn await_is_keyword(&self) -> bool {
        self.ctx.is_async
            || (self.goal == Goal::Module
                && !self.ctx.in_function
                && !self.ctx.static_block
                && !self.ctx.class_init)
    }

    // ---- name checks ----

    fn check_ident_ref(&self, name: &str, span: Span) -> PResult<()> {
        if is_always_reserved(name) {
            return self.err(format!("`{name}` is a reserved word"), span);
        }
        if self.ctx.strict && is_strict_reserved(name) {
            return self.err(format!("`{name}` is reserved in strict mode"), span);
        }
        if name == "yield" && self.ctx.is_generator {
            return self.err("`yield` can't be a name inside a generator", span);
        }
        if name == "await"
            && (self.await_is_keyword() || self.goal == Goal::Module || self.ctx.static_block)
        {
            return self.err("`await` can't be a name here", span);
        }
        Ok(())
    }

    fn check_binding_ident(&self, name: &str, span: Span) -> PResult<()> {
        self.check_ident_ref(name, span)?;
        if self.ctx.strict && (name == "eval" || name == "arguments") {
            return self.err(format!("`{name}` can't be bound in strict mode"), span);
        }
        Ok(())
    }

    fn check_strict_names(&self, names: &[Ident]) -> PResult<()> {
        for n in names {
            if n.name == "eval" || n.name == "arguments" {
                return self.err(
                    format!("`{}` can't be bound in strict mode", n.name),
                    n.span,
                );
            }
            if is_strict_reserved(&n.name) {
                return self.err(format!("`{}` is reserved in strict mode", n.name), n.span);
            }
        }
        Ok(())
    }

    fn check_unique(&self, names: &[Ident]) -> PResult<()> {
        let mut seen = HashSet::new();
        for n in names {
            if !seen.insert(n.name.as_str()) {
                return self.err(format!("duplicate parameter `{}`", n.name), n.span);
            }
        }
        Ok(())
    }

    // ---- scopes ----

    fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope::new(kind));
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn top_scope(&mut self) -> &mut Scope {
        if self.scopes.is_empty() {
            self.scopes.push(Scope::new(ScopeKind::Function));
        }
        let last = self.scopes.len() - 1;
        &mut self.scopes[last]
    }

    fn declare_lexical(&mut self, id: &Ident) -> PResult<()> {
        let s = self.top_scope();
        if s.lexical.contains(&id.name)
            || s.vars.contains(&id.name)
            || s.params.contains(&id.name)
            || s.catch_params.contains(&id.name)
        {
            return self.err(format!("`{}` has already been declared", id.name), id.span);
        }
        s.lexical.insert(id.name.clone());
        Ok(())
    }

    fn declare_var(&mut self, id: &Ident) -> PResult<()> {
        for s in self.scopes.iter_mut().rev() {
            if s.lexical.contains(&id.name)
                || (s.catch_params.contains(&id.name) && !s.catch_simple)
            {
                return Err(SyntaxError::new(
                    format!("`{}` has already been declared", id.name),
                    id.span,
                ));
            }
            s.vars.insert(id.name.clone());
            if s.kind == ScopeKind::Function {
                break;
            }
        }
        Ok(())
    }

    fn declare_function(&mut self, id: &Ident, plain: bool) -> PResult<()> {
        let module_top = self.goal == Goal::Module && self.scopes.len() == 1;
        let strict = self.ctx.strict;
        let kind = self.top_scope().kind;
        if kind == ScopeKind::Function && !module_top {
            return self.declare_var(id);
        }
        if kind == ScopeKind::Block && !strict && plain {
            let s = self.top_scope();
            if s.lexical.contains(&id.name) && s.annex_fns.contains(&id.name) {
                return Ok(());
            }
            self.declare_lexical(id)?;
            self.top_scope().annex_fns.insert(id.name.clone());
            return Ok(());
        }
        self.declare_lexical(id)
    }

    fn declare_pattern(&mut self, kind: VarKind, pat: &Pat) -> PResult<()> {
        for id in pat.bound_names() {
            match kind {
                VarKind::Var => self.declare_var(&id)?,
                VarKind::Let | VarKind::Const => {
                    if id.name == "let" {
                        return self.err("`let` can't be a lexical binding name", id.span);
                    }
                    self.declare_lexical(&id)?
                }
            }
        }
        Ok(())
    }

    // ---- private names ----

    fn declare_private(&mut self, name: &Ident, kind: PrivKind, is_static: bool) -> PResult<()> {
        if name.name == "constructor" {
            return self.err("`#constructor` is not allowed", name.span);
        }
        let Some(class) = self.classes.last_mut() else {
            return self.err("private names are only allowed in classes", name.span);
        };
        match class.declared.get(&name.name).copied() {
            None => {
                class.declared.insert(name.name.clone(), (kind, is_static));
                Ok(())
            }
            Some((PrivKind::Get, s)) if kind == PrivKind::Set && s == is_static => {
                class
                    .declared
                    .insert(name.name.clone(), (PrivKind::GetSet, s));
                Ok(())
            }
            Some((PrivKind::Set, s)) if kind == PrivKind::Get && s == is_static => {
                class
                    .declared
                    .insert(name.name.clone(), (PrivKind::GetSet, s));
                Ok(())
            }
            Some(_) => self.err(
                format!("duplicate private name `#{}`", name.name),
                name.span,
            ),
        }
    }

    fn reference_private(&mut self, name: &str, span: Span) -> PResult<()> {
        match self.classes.last_mut() {
            Some(c) => {
                c.refs.push((name.to_string(), span));
                Ok(())
            }
            None => self.err(format!("private name `#{name}` is not defined"), span),
        }
    }

    fn close_class(&mut self) -> PResult<()> {
        let Some(class) = self.classes.pop() else {
            return Ok(());
        };
        for (name, span) in class.refs {
            if class.declared.contains_key(&name) {
                continue;
            }
            match self.classes.last_mut() {
                Some(outer) => outer.refs.push((name, span)),
                None => return self.err(format!("private name `#{name}` is not defined"), span),
            }
        }
        Ok(())
    }

    // ---- program and statement lists ----

    fn parse_program(mut self) -> PResult<Program> {
        let start = self.cur.span;
        let start = Span {
            start: 0,
            line: 1,
            column: 1,
            ..start
        };
        self.push_scope(ScopeKind::Function);
        let (body, _) = self.parse_body(false)?;
        if self.goal == Goal::Module {
            let locals = std::mem::take(&mut self.export_locals);
            let top = &self.scopes[0];
            for (name, span) in locals {
                if !top.lexical.contains(&name) && !top.vars.contains(&name) {
                    return self.err(format!("exported name `{name}` is not declared"), span);
                }
            }
        }
        let span = Span {
            end: self.src.len(),
            ..start
        };
        Ok(Program {
            source_type: if self.goal == Goal::Module {
                SourceType::Module
            } else {
                SourceType::Script
            },
            body,
            strict: self.ctx.strict,
            span,
        })
    }

    /// Statements up to `}` (when `braced`) or end of input, with a directive
    /// prologue first. Returns the statements and whether a `"use strict"`
    /// directive switched strict mode on.
    fn parse_body(&mut self, braced: bool) -> PResult<(Vec<Stmt>, bool)> {
        let mut body = Vec::new();
        let mut prologue = true;
        let mut octal: Option<Span> = None;
        let mut became_strict = false;
        loop {
            if braced && self.at(Punct::RBrace) {
                break;
            }
            if self.cur.kind == TokenKind::Eof {
                if braced {
                    return self.unexpected();
                }
                break;
            }
            if prologue {
                if let TokenKind::String(_) = self.cur.kind {
                    let tok = self.cur.clone();
                    let mut stmt = self.parse_statement_list_item()?;
                    let mut is_directive = false;
                    if let StmtKind::Expr { expr, directive } = &mut stmt.kind
                        && matches!(expr.kind, ExprKind::Str(_))
                        && expr.span == tok.span
                    {
                        is_directive = true;
                        let raw = self.src[tok.span.start + 1..tok.span.end - 1].to_string();
                        if tok.legacy_octal && octal.is_none() {
                            octal = Some(tok.span);
                        }
                        if raw == "use strict" {
                            if !self.ctx.strict {
                                self.ctx.strict = true;
                                became_strict = true;
                            }
                            if let Some(span) = octal {
                                return self.err(
                                    "legacy octal escapes are not allowed in strict mode",
                                    span,
                                );
                            }
                        }
                        *directive = Some(raw);
                    }
                    if !is_directive {
                        prologue = false;
                    }
                    body.push(stmt);
                    continue;
                }
                prologue = false;
            }
            body.push(self.parse_statement_list_item()?);
        }
        Ok((body, became_strict))
    }

    fn parse_statement_list_item(&mut self) -> PResult<Stmt> {
        self.enter()?;
        let r = self.statement_list_item_inner();
        self.leave();
        r
    }

    fn statement_list_item_inner(&mut self) -> PResult<Stmt> {
        if !matches!(self.cur.kind, TokenKind::Ident(_)) {
            return self.statement_inner();
        }
        self.declaration_or_statement()
    }

    fn declaration_or_statement(&mut self) -> PResult<Stmt> {
        let start = self.cur.span;
        if self.at_word("function") {
            return self.parse_function_declaration(start, false);
        }
        if self.at_word("async") {
            let p = self.peek()?;
            if p.is_word("function") && !p.newline_before {
                self.bump()?;
                return self.parse_function_declaration(start, true);
            }
        }
        if self.at_word("class") {
            let class = self.parse_class(true)?;
            if let Some(id) = &class.id {
                self.declare_lexical(id)?;
            }
            return Ok(Stmt {
                kind: StmtKind::Class(Box::new(class)),
                span: self.finish(start),
            });
        }
        if self.at_word("const") || (self.at_word("let") && self.let_starts_declaration()?) {
            let kind = if self.at_word("const") {
                VarKind::Const
            } else {
                VarKind::Let
            };
            let decl = self.parse_var_decl(kind, false)?;
            self.semicolon()?;
            return Ok(Stmt {
                kind: StmtKind::Var(decl),
                span: self.finish(start),
            });
        }
        if self.at_word("import") {
            let p = self.peek()?;
            if !p.is_punct(Punct::LParen) && !p.is_punct(Punct::Dot) {
                return self.parse_import(start);
            }
        }
        if self.at_word("export") {
            return self.parse_export(start);
        }
        self.statement_inner()
    }

    fn let_starts_declaration(&mut self) -> PResult<bool> {
        let p = self.peek()?;
        Ok(match &p.kind {
            TokenKind::Punct(Punct::LBracket) | TokenKind::Punct(Punct::LBrace) => true,
            TokenKind::Ident(n) => !is_always_reserved(n) || p.escaped,
            _ => false,
        })
    }

    fn parse_statement(&mut self) -> PResult<Stmt> {
        self.enter()?;
        let r = self.statement_inner();
        self.leave();
        r
    }

    /// A body in a spot that takes a single statement (loop bodies, `if`
    /// branches, labeled statements).
    fn statement_inner(&mut self) -> PResult<Stmt> {
        if self.at(Punct::LBrace) {
            let start = self.cur.span;
            let body = self.parse_block(None)?;
            return Ok(Stmt {
                kind: StmtKind::Block(body),
                span: self.finish(start),
            });
        }
        if !matches!(self.cur.kind, TokenKind::Ident(_)) || self.cur.escaped {
            let start = self.cur.span;
            if self.at(Punct::Semicolon) {
                self.bump()?;
                return Ok(Stmt {
                    kind: StmtKind::Empty,
                    span: start,
                });
            }
            return self.parse_expression_or_labeled(start);
        }
        self.statement_keyword()
    }

    fn statement_keyword(&mut self) -> PResult<Stmt> {
        // Statements that nest go first, each in its own function, so the
        // big match below stays off the recursion path.
        let start = self.cur.span;
        if self.at_word("if") {
            return self.parse_if(start);
        }
        if self.at_word("for") {
            return self.parse_for(start);
        }
        if self.at_word("while") {
            return self.parse_while(start);
        }
        if self.at_word("do") {
            return self.parse_do(start);
        }
        if self.at_word("try") {
            return self.parse_try(start);
        }
        if self.at_word("switch") {
            return self.parse_switch(start);
        }
        if self.at_word("with") {
            return self.parse_with(start);
        }
        self.statement_other()
    }

    fn parse_while(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let test = self.parse_expression_in()?;
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(Stmt {
            kind: StmtKind::While {
                test: Box::new(test),
                body: Box::new(body),
            },
            span: self.finish(start),
        })
    }

    fn parse_do(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        let body = self.parse_loop_body()?;
        self.expect_word("while")?;
        self.expect(Punct::LParen)?;
        let test = self.parse_expression_in()?;
        self.expect(Punct::RParen)?;
        self.eat(Punct::Semicolon)?;
        Ok(Stmt {
            kind: StmtKind::DoWhile {
                body: Box::new(body),
                test: Box::new(test),
            },
            span: self.finish(start),
        })
    }

    fn parse_with(&mut self, start: Span) -> PResult<Stmt> {
        if self.ctx.strict {
            return self.err("`with` is not allowed in strict mode", start);
        }
        self.bump()?;
        self.expect(Punct::LParen)?;
        let object = self.parse_expression_in()?;
        self.expect(Punct::RParen)?;
        let body = self.parse_statement()?;
        Ok(Stmt {
            kind: StmtKind::With {
                object: Box::new(object),
                body: Box::new(body),
            },
            span: self.finish(start),
        })
    }

    fn statement_other(&mut self) -> PResult<Stmt> {
        let start = self.cur.span;
        let kind = match &self.cur.kind {
            TokenKind::Punct(Punct::LBrace) => StmtKind::Block(self.parse_block(None)?),
            TokenKind::Punct(Punct::Semicolon) => {
                self.bump()?;
                StmtKind::Empty
            }
            TokenKind::Ident(_) if !self.cur.escaped => {
                let word = match &self.cur.kind {
                    TokenKind::Ident(n) => n.clone(),
                    _ => String::new(),
                };
                match word.as_str() {
                    "var" => {
                        let d = self.parse_var_decl(VarKind::Var, false)?;
                        self.semicolon()?;
                        StmtKind::Var(d)
                    }
                    "continue" | "break" => {
                        return self.parse_break_continue(start, word == "break");
                    }
                    "return" => {
                        if !self.ctx.in_function {
                            return self.err("`return` is only allowed inside a function", start);
                        }
                        self.bump()?;
                        let arg = if self.at(Punct::Semicolon)
                            || self.at(Punct::RBrace)
                            || self.cur.kind == TokenKind::Eof
                            || self.cur.newline_before
                        {
                            None
                        } else {
                            Some(Box::new(self.parse_expression_in()?))
                        };
                        self.semicolon()?;
                        StmtKind::Return(arg)
                    }
                    "throw" => {
                        self.bump()?;
                        if self.cur.newline_before {
                            return self
                                .err("line break is not allowed after `throw`", self.cur.span);
                        }
                        let arg = self.parse_expression_in()?;
                        self.semicolon()?;
                        StmtKind::Throw(Box::new(arg))
                    }
                    "debugger" => {
                        self.bump()?;
                        self.semicolon()?;
                        StmtKind::Debugger
                    }
                    "function" => {
                        return self.err("a function declaration is not allowed here", start);
                    }
                    "class" | "const" => {
                        return self.err("a declaration is not allowed here", start);
                    }
                    "let" => {
                        let p = self.peek()?;
                        if p.is_punct(Punct::LBracket) {
                            return self.err("a statement can't start with `let [`", start);
                        }
                        if (matches!(p.kind, TokenKind::Ident(_)) || p.is_punct(Punct::LBrace))
                            && !p.newline_before
                        {
                            return self.err("a lexical declaration is not allowed here", start);
                        }
                        return self.parse_expression_or_labeled(start);
                    }
                    "async" => {
                        let p = self.peek()?;
                        if p.is_word("function") && !p.newline_before {
                            return self
                                .err("an async function declaration is not allowed here", start);
                        }
                        return self.parse_expression_or_labeled(start);
                    }
                    _ => return self.parse_expression_or_labeled(start),
                }
            }
            _ => return self.parse_expression_or_labeled(start),
        };
        Ok(Stmt {
            kind,
            span: self.finish(start),
        })
    }

    fn parse_expression_or_labeled(&mut self, start: Span) -> PResult<Stmt> {
        if matches!(self.cur.kind, TokenKind::Ident(_)) && self.peek()?.is_punct(Punct::Colon) {
            return self.parse_labeled(start);
        }
        let expr = self.parse_expression()?;
        self.semicolon()?;
        Ok(Stmt {
            kind: StmtKind::Expr {
                expr: Box::new(expr),
                directive: None,
            },
            span: self.finish(start),
        })
    }

    fn parse_labeled(&mut self, start: Span) -> PResult<Stmt> {
        let mut labels: Vec<Ident> = Vec::new();
        while matches!(self.cur.kind, TokenKind::Ident(_)) && self.peek()?.is_punct(Punct::Colon) {
            let tok = self.bump()?;
            let name = match tok.kind {
                TokenKind::Ident(n) => n,
                _ => String::new(),
            };
            self.check_ident_ref(&name, tok.span)?;
            if self.ctx.labels.iter().any(|l| l.name == name) {
                return self.err(format!("label `{name}` is already in use"), tok.span);
            }
            self.bump()?;
            self.ctx.labels.push(Label {
                name: name.clone(),
                is_loop: false,
            });
            labels.push(Ident {
                name,
                span: tok.span,
            });
        }
        if self.at_word("for") || self.at_word("while") || self.at_word("do") {
            let n = self.ctx.labels.len();
            for l in &mut self.ctx.labels[n - labels.len()..] {
                l.is_loop = true;
            }
        }
        let body = if self.at_word("function") {
            if self.ctx.strict {
                return self.err(
                    "labeled function declarations are not allowed in strict mode",
                    self.cur.span,
                );
            }
            let fstart = self.cur.span;
            let f = self.parse_function_declaration(fstart, false)?;
            if let StmtKind::Function(func) = &f.kind
                && (func.is_generator || func.is_async)
            {
                return self.err("only plain functions can be labeled", fstart);
            }
            f
        } else {
            self.parse_statement()?
        };
        let n = self.ctx.labels.len();
        self.ctx.labels.truncate(n - labels.len());
        let mut stmt = body;
        for label in labels.into_iter().rev() {
            stmt = Stmt {
                kind: StmtKind::Labeled {
                    label,
                    body: Box::new(stmt),
                },
                span: self.finish(start),
            };
        }
        Ok(stmt)
    }

    fn parse_break_continue(&mut self, start: Span, is_break: bool) -> PResult<Stmt> {
        self.bump()?;
        let mut label = None;
        if !self.cur.newline_before
            && let TokenKind::Ident(name) = &self.cur.kind
        {
            let name = name.clone();
            let span = self.cur.span;
            match self.ctx.labels.iter().find(|l| l.name == name) {
                None => return self.err(format!("label `{name}` is not defined"), span),
                Some(l) if !is_break && !l.is_loop => {
                    return self.err(format!("`continue` label `{name}` is not on a loop"), span);
                }
                Some(_) => {}
            }
            self.bump()?;
            label = Some(Ident { name, span });
        }
        if label.is_none() {
            if is_break && self.ctx.loop_depth == 0 && self.ctx.switch_depth == 0 {
                return self.err("`break` must be inside a loop or switch", start);
            }
            if !is_break && self.ctx.loop_depth == 0 {
                return self.err("`continue` must be inside a loop", start);
            }
        }
        self.semicolon()?;
        let kind = if is_break {
            StmtKind::Break(label)
        } else {
            StmtKind::Continue(label)
        };
        Ok(Stmt {
            kind,
            span: self.finish(start),
        })
    }

    fn parse_block(&mut self, catch: Option<(HashSet<String>, bool)>) -> PResult<Vec<Stmt>> {
        self.expect(Punct::LBrace)?;
        self.push_scope(ScopeKind::Block);
        if let Some((names, simple)) = catch {
            let s = self.top_scope();
            s.catch_params = names;
            s.catch_simple = simple;
        }
        let mut body = Vec::new();
        while !self.at(Punct::RBrace) {
            if self.cur.kind == TokenKind::Eof {
                return self.unexpected();
            }
            body.push(self.parse_statement_list_item()?);
        }
        self.bump()?;
        self.pop_scope();
        Ok(body)
    }

    fn parse_loop_body(&mut self) -> PResult<Stmt> {
        self.ctx.loop_depth += 1;
        let body = self.parse_statement();
        self.ctx.loop_depth -= 1;
        body
    }

    fn parse_if(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let test = self.parse_expression_in()?;
        self.expect(Punct::RParen)?;
        let cons = self.parse_if_branch()?;
        let alt = if self.at_word("else") {
            self.bump()?;
            Some(Box::new(self.parse_if_branch()?))
        } else {
            None
        };
        Ok(Stmt {
            kind: StmtKind::If {
                test: Box::new(test),
                cons: Box::new(cons),
                alt,
            },
            span: self.finish(start),
        })
    }

    fn parse_if_branch(&mut self) -> PResult<Stmt> {
        if self.at_word("function") && !self.ctx.strict {
            // Annex B: a plain function declaration as an `if` branch acts
            // as if it sat in its own block.
            let fstart = self.cur.span;
            self.push_scope(ScopeKind::Block);
            let f = self.parse_function_declaration(fstart, false);
            self.pop_scope();
            let f = f?;
            if let StmtKind::Function(func) = &f.kind
                && func.is_generator
            {
                return self.err("a generator declaration is not allowed here", fstart);
            }
            return Ok(f);
        }
        self.parse_statement()
    }

    fn parse_switch(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let disc = self.parse_expression_in()?;
        self.expect(Punct::RParen)?;
        self.expect(Punct::LBrace)?;
        self.push_scope(ScopeKind::Block);
        self.ctx.switch_depth += 1;
        let mut cases = Vec::new();
        let mut seen_default = false;
        while !self.at(Punct::RBrace) {
            let cstart = self.cur.span;
            let test = if self.at_word("case") {
                self.bump()?;
                Some(Box::new(self.parse_expression_in()?))
            } else if self.at_word("default") {
                if seen_default {
                    return self.err("a switch can only have one `default`", cstart);
                }
                seen_default = true;
                self.bump()?;
                None
            } else {
                return self.unexpected();
            };
            self.expect(Punct::Colon)?;
            let mut body = Vec::new();
            while !self.at(Punct::RBrace) && !self.at_word("case") && !self.at_word("default") {
                if self.cur.kind == TokenKind::Eof {
                    return self.unexpected();
                }
                body.push(self.parse_statement_list_item()?);
            }
            cases.push(SwitchCase {
                test,
                body,
                span: self.finish(cstart),
            });
        }
        self.bump()?;
        self.ctx.switch_depth -= 1;
        self.pop_scope();
        Ok(Stmt {
            kind: StmtKind::Switch {
                disc: Box::new(disc),
                cases,
            },
            span: self.finish(start),
        })
    }

    fn parse_try(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        let block = self.parse_block(None)?;
        let mut handler = None;
        if self.at_word("catch") {
            let cstart = self.cur.span;
            self.bump()?;
            let mut param = None;
            let mut names = HashSet::new();
            let mut simple = false;
            if self.eat(Punct::LParen)? {
                let p = self.parse_binding_target()?;
                let bound = p.bound_names();
                for id in &bound {
                    if !names.insert(id.name.clone()) {
                        return self
                            .err(format!("duplicate catch parameter `{}`", id.name), id.span);
                    }
                }
                simple = matches!(p.kind, PatKind::Ident(_));
                param = Some(p);
                self.expect(Punct::RParen)?;
            }
            let body = self.parse_block(Some((names, simple)))?;
            handler = Some(CatchClause {
                param,
                body,
                span: self.finish(cstart),
            });
        }
        let finalizer = if self.at_word("finally") {
            self.bump()?;
            Some(self.parse_block(None)?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return self.err("`try` needs a `catch` or `finally`", start);
        }
        Ok(Stmt {
            kind: StmtKind::Try {
                block,
                handler,
                finalizer,
            },
            span: self.finish(start),
        })
    }

    fn parse_var_decl(&mut self, kind: VarKind, for_head: bool) -> PResult<VarDecl> {
        let start = self.cur.span;
        self.bump()?;
        let mut decls = Vec::new();
        loop {
            let dstart = self.cur.span;
            let target = self.parse_binding_target()?;
            self.declare_pattern(kind, &target)?;
            let init = if self.eat(Punct::Eq)? {
                Some(Box::new(self.parse_assign()?))
            } else {
                if !for_head || !(self.at_word("in") || self.at_word("of")) {
                    if kind == VarKind::Const {
                        return self.err("`const` declarations need an initializer", dstart);
                    }
                    if !matches!(target.kind, PatKind::Ident(_)) {
                        return self
                            .err("a destructuring declaration needs an initializer", dstart);
                    }
                }
                None
            };
            decls.push(VarDeclarator {
                target,
                init,
                span: self.finish(dstart),
            });
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        Ok(VarDecl {
            kind,
            decls,
            span: self.finish(start),
        })
    }

    fn parse_for(&mut self, start: Span) -> PResult<Stmt> {
        self.bump()?;
        let mut is_await = false;
        if self.at_word("await") {
            if !self.await_is_keyword() {
                return self.err("`for await` is only allowed in async code", self.cur.span);
            }
            self.bump()?;
            is_await = true;
        }
        self.expect(Punct::LParen)?;
        self.push_scope(ScopeKind::Block);
        let r = self.parse_for_rest(start, is_await);
        self.pop_scope();
        r
    }

    fn parse_for_rest(&mut self, start: Span, is_await: bool) -> PResult<Stmt> {
        let mut decl: Option<VarDecl> = None;
        let mut init_expr: Option<Expr> = None;
        let starts_with_let = self.at_word("let");
        let starts_with_async = self.at_word("async");
        if self.at(Punct::Semicolon) {
            // empty init
        } else if self.at_word("var")
            || self.at_word("const")
            || (self.at_word("let") && {
                let p = self.peek()?;
                match &p.kind {
                    TokenKind::Punct(Punct::LBracket) | TokenKind::Punct(Punct::LBrace) => true,
                    TokenKind::Ident(n) => n != "in" || p.escaped,
                    _ => false,
                }
            })
        {
            let kind = if self.at_word("var") {
                VarKind::Var
            } else if self.at_word("const") {
                VarKind::Const
            } else {
                VarKind::Let
            };
            let saved = std::mem::replace(&mut self.no_in, true);
            let d = self.parse_var_decl(kind, true);
            self.no_in = saved;
            decl = Some(d?);
        } else {
            let saved_in = std::mem::replace(&mut self.no_in, true);
            let saved_cover = self.cover_err.take();
            let first = self.parse_assign_inner(true);
            self.no_in = saved_in;
            let first = first?;
            if self.at(Punct::Comma) {
                if let Some(e) = self.cover_err.take() {
                    return Err(e);
                }
                let mut items = vec![first];
                let saved_in = std::mem::replace(&mut self.no_in, true);
                while self.eat(Punct::Comma)? {
                    items.push(self.parse_assign()?);
                }
                self.no_in = saved_in;
                let span = items[0].span.to(items[items.len() - 1].span);
                init_expr = Some(Expr {
                    kind: ExprKind::Seq(items),
                    span,
                });
            } else {
                init_expr = Some(first);
            }
            if !(self.at_word("of") || self.at_word("in"))
                && let Some(e) = self.cover_err.take()
            {
                return Err(e);
            }
            self.cover_err = saved_cover;
        }

        let is_of = self.at_word("of");
        let is_in = self.at_word("in");
        if is_of || is_in {
            if is_in && is_await {
                return self.err("`for await` needs `of`", self.cur.span);
            }
            let head = if let Some(d) = decl {
                if d.decls.len() != 1 {
                    return self.err(
                        "only one binding is allowed in a for-in or for-of head",
                        d.span,
                    );
                }
                if let Some(init) = &d.decls[0].init {
                    let annex_b = is_in
                        && !self.ctx.strict
                        && d.kind == VarKind::Var
                        && matches!(d.decls[0].target.kind, PatKind::Ident(_));
                    if !annex_b {
                        return self.err(
                            "a for-in or for-of binding can't have an initializer",
                            init.span,
                        );
                    }
                }
                ForHead::Var(d)
            } else {
                let Some(e) = init_expr else {
                    return self.unexpected();
                };
                if is_of && starts_with_let {
                    return self.err("a for-of target can't start with `let`", e.span);
                }
                if is_of
                    && starts_with_async
                    && !is_await
                    && matches!(&e.kind, ExprKind::Ident(n) if n == "async")
                {
                    return self.err("`for (async of` is not allowed", e.span);
                }
                let pat = self.expr_to_target(e)?;
                self.cover_err = None;
                ForHead::Pat(pat)
            };
            self.bump()?;
            let right = if is_of {
                self.parse_assign_in()?
            } else {
                self.parse_expression_in()?
            };
            self.expect(Punct::RParen)?;
            let body = self.parse_loop_body()?;
            let kind = if is_of {
                StmtKind::ForOf {
                    left: head,
                    right: Box::new(right),
                    body: Box::new(body),
                    is_await,
                }
            } else {
                StmtKind::ForIn {
                    left: head,
                    right: Box::new(right),
                    body: Box::new(body),
                }
            };
            return Ok(Stmt {
                kind,
                span: self.finish(start),
            });
        }
        if is_await {
            return self.err("`for await` needs `of`", self.cur.span);
        }
        if let Some(d) = &decl {
            for v in &d.decls {
                if v.init.is_none() {
                    if d.kind == VarKind::Const {
                        return self.err("`const` declarations need an initializer", v.span);
                    }
                    if !matches!(v.target.kind, PatKind::Ident(_)) {
                        return self
                            .err("a destructuring declaration needs an initializer", v.span);
                    }
                }
            }
        }
        let init = match (decl, init_expr) {
            (Some(d), _) => Some(ForInit::Var(d)),
            (None, Some(e)) => Some(ForInit::Expr(Box::new(e))),
            (None, None) => None,
        };
        self.expect(Punct::Semicolon)?;
        let test = if self.at(Punct::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expression_in()?))
        };
        self.expect(Punct::Semicolon)?;
        let update = if self.at(Punct::RParen) {
            None
        } else {
            Some(Box::new(self.parse_expression_in()?))
        };
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(Stmt {
            kind: StmtKind::For {
                init,
                test,
                update,
                body: Box::new(body),
            },
            span: self.finish(start),
        })
    }

    // ---- functions ----

    fn parse_function_declaration(&mut self, start: Span, is_async: bool) -> PResult<Stmt> {
        let f = self.parse_function(start, is_async, true, false)?;
        Ok(Stmt {
            kind: StmtKind::Function(Box::new(f)),
            span: self.finish(start),
        })
    }

    /// `function` keyword onward. `async` has already been consumed.
    fn parse_function(
        &mut self,
        start: Span,
        is_async: bool,
        is_decl: bool,
        name_optional: bool,
    ) -> PResult<Function> {
        self.expect_word("function")?;
        let is_generator = self.eat(Punct::Star)?;
        let mut id = None;
        if let TokenKind::Ident(name) = &self.cur.kind {
            let name = name.clone();
            let span = self.cur.span;
            if is_decl {
                self.check_binding_ident(&name, span)?;
            } else {
                // A function expression's name follows the function's own
                // generator and async rules.
                let saved = self.ctx.clone();
                self.ctx.is_generator = is_generator;
                self.ctx.is_async = is_async;
                self.ctx.in_function = true;
                let r = self.check_binding_ident(&name, span);
                self.ctx = saved;
                r?;
            }
            self.bump()?;
            id = Some(Ident { name, span });
        } else if is_decl && !name_optional {
            return self.err("a function declaration needs a name", self.cur.span);
        }
        if is_decl && let Some(i) = &id {
            self.declare_function(i, !is_async && !is_generator)?;
        }
        self.parse_function_rest(start, id, is_async, is_generator, Flavor::Normal)
    }

    /// Parameters and body.
    fn parse_function_rest(
        &mut self,
        start: Span,
        id: Option<Ident>,
        is_async: bool,
        is_generator: bool,
        flavor: Flavor,
    ) -> PResult<Function> {
        let outer = self.ctx.clone();
        let saved_marks = (self.last_yield_await, self.last_await_ident);
        let saved_in = std::mem::replace(&mut self.no_in, false);
        self.ctx = Ctx {
            strict: outer.strict,
            in_function: true,
            is_async,
            is_generator,
            super_prop: flavor.is_method(),
            super_call: matches!(flavor, Flavor::Constructor { derived: true }),
            new_target: true,
            ..Ctx::default()
        };
        self.push_scope(ScopeKind::Function);
        let r = self.function_rest_inner(start, id, is_async, is_generator, flavor);
        self.pop_scope();
        self.ctx = outer;
        self.no_in = saved_in;
        self.last_yield_await = saved_marks.0;
        self.last_await_ident = saved_marks.1;
        r
    }

    fn function_rest_inner(
        &mut self,
        start: Span,
        id: Option<Ident>,
        is_async: bool,
        is_generator: bool,
        flavor: Flavor,
    ) -> PResult<Function> {
        let pstart = self.cur.span;
        self.expect(Punct::LParen)?;
        let params = self.parse_formal_params()?;
        if let Some(at) = self.last_yield_await
            && at >= pstart.start
        {
            return self.err(
                "`yield` and `await` expressions are not allowed in parameters",
                pstart,
            );
        }
        match flavor {
            Flavor::Getter if !params.is_empty() => {
                return self.err("a getter takes no parameters", pstart);
            }
            Flavor::Setter if params.len() != 1 || matches!(params[0].kind, PatKind::Rest(_)) => {
                return self.err("a setter takes exactly one parameter", pstart);
            }
            _ => {}
        }
        let simple = params.iter().all(|p| matches!(p.kind, PatKind::Ident(_)));
        let names: Vec<Ident> = params.iter().flat_map(|p| p.bound_names()).collect();
        {
            let s = self.top_scope();
            for n in &names {
                s.params.insert(n.name.clone());
            }
        }
        self.expect(Punct::LBrace)?;
        let (body, became_strict) = self.parse_body(true)?;
        self.expect(Punct::RBrace)?;
        if became_strict && !simple {
            return self.err(
                "\"use strict\" is not allowed in a function with non-simple parameters",
                start,
            );
        }
        if became_strict {
            self.check_strict_names(&names)?;
            if let Some(i) = &id {
                self.check_strict_names(std::slice::from_ref(i))?;
            }
        }
        if self.ctx.strict || !simple || flavor.is_method() {
            self.check_unique(&names)?;
        }
        Ok(Function {
            id,
            params,
            body,
            is_async,
            is_generator,
            strict: self.ctx.strict,
            simple_params: simple,
            span: self.finish(start),
        })
    }

    /// After `(`, through `)`.
    fn parse_formal_params(&mut self) -> PResult<Vec<Pat>> {
        let mut params = Vec::new();
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let rstart = self.cur.span;
                self.bump()?;
                let target = self.parse_binding_target()?;
                if self.at(Punct::Eq) {
                    return self.err("a rest parameter can't have a default", self.cur.span);
                }
                params.push(Pat {
                    kind: PatKind::Rest(Box::new(target)),
                    span: self.finish(rstart),
                });
                if !self.at(Punct::RParen) {
                    return self.err("a rest parameter must be last", self.cur.span);
                }
                break;
            }
            params.push(self.parse_binding_element()?);
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        self.expect(Punct::RParen)?;
        Ok(params)
    }

    fn parse_arrow_rest(
        &mut self,
        start: Span,
        params: Vec<Pat>,
        is_async: bool,
        params_start: usize,
    ) -> PResult<Expr> {
        if self.cur.newline_before {
            return self.err("line break is not allowed before `=>`", self.cur.span);
        }
        self.expect(Punct::Arrow)?;
        if let Some(at) = self.last_yield_await
            && at >= params_start
        {
            return self.err(
                "`yield` and `await` expressions are not allowed in arrow parameters",
                start,
            );
        }
        let names: Vec<Ident> = params.iter().flat_map(|p| p.bound_names()).collect();
        if is_async {
            if let Some(at) = self.last_await_ident
                && at >= params_start
            {
                return self.err("`await` can't be used in async arrow parameters", start);
            }
            if let Some(n) = names.iter().find(|n| n.name == "await") {
                return self.err("`await` can't be a parameter name here", n.span);
            }
        }
        self.check_unique(&names)?;
        let simple = params.iter().all(|p| matches!(p.kind, PatKind::Ident(_)));
        let outer = self.ctx.clone();
        let saved_marks = (self.last_yield_await, self.last_await_ident);
        self.ctx.in_function = true;
        self.ctx.is_async = is_async;
        self.ctx.is_generator = false;
        self.ctx.labels = Vec::new();
        self.ctx.loop_depth = 0;
        self.ctx.switch_depth = 0;
        self.push_scope(ScopeKind::Function);
        {
            let s = self.top_scope();
            for n in &names {
                s.params.insert(n.name.clone());
            }
        }
        let r = (|| -> PResult<ArrowBody> {
            if self.at(Punct::LBrace) {
                let saved_in = std::mem::replace(&mut self.no_in, false);
                self.bump()?;
                let r = self.parse_body(true);
                self.no_in = saved_in;
                let (body, became_strict) = r?;
                self.expect(Punct::RBrace)?;
                if became_strict && !simple {
                    return self.err(
                        "\"use strict\" is not allowed in a function with non-simple parameters",
                        start,
                    );
                }
                if became_strict {
                    self.check_strict_names(&names)?;
                }
                Ok(ArrowBody::Block(body))
            } else {
                Ok(ArrowBody::Expr(Box::new(self.parse_assign()?)))
            }
        })();
        let strict = self.ctx.strict;
        self.pop_scope();
        self.ctx = outer;
        self.last_yield_await = saved_marks.0;
        self.last_await_ident = saved_marks.1;
        let body = r?;
        let span = self.finish(start);
        Ok(Expr {
            kind: ExprKind::Arrow(Box::new(ArrowFunction {
                params,
                body,
                is_async,
                strict,
                span,
            })),
            span,
        })
    }

    // ---- classes ----

    fn parse_class(&mut self, is_decl: bool) -> PResult<Class> {
        self.parse_class_with(is_decl, false)
    }

    fn parse_class_with(&mut self, is_decl: bool, name_optional: bool) -> PResult<Class> {
        let start = self.cur.span;
        self.expect_word("class")?;
        let outer_strict = self.ctx.strict;
        self.ctx.strict = true;
        let r = self.class_inner(start, is_decl, name_optional);
        self.ctx.strict = outer_strict;
        r
    }

    fn class_inner(&mut self, start: Span, is_decl: bool, name_optional: bool) -> PResult<Class> {
        let mut id = None;
        if let TokenKind::Ident(name) = &self.cur.kind
            && !self.at_word("extends")
        {
            let name = name.clone();
            let span = self.cur.span;
            self.check_binding_ident(&name, span)?;
            self.bump()?;
            id = Some(Ident { name, span });
        }
        if id.is_none() && is_decl && !name_optional {
            return self.err("a class declaration needs a name", self.cur.span);
        }
        let super_class = if self.at_word("extends") {
            self.bump()?;
            let saved_in = std::mem::replace(&mut self.no_in, false);
            let e = self.parse_lhs();
            self.no_in = saved_in;
            let e = e?;
            if is_arrow(&e) {
                return self.err("unexpected arrow function", e.span);
            }
            Some(Box::new(e))
        } else {
            None
        };
        let derived = super_class.is_some();
        self.expect(Punct::LBrace)?;
        self.classes.push(ClassPrivates::default());
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = (|| -> PResult<Vec<ClassMember>> {
            let mut body = Vec::new();
            let mut has_ctor = false;
            while !self.at(Punct::RBrace) {
                if self.eat(Punct::Semicolon)? {
                    continue;
                }
                if self.cur.kind == TokenKind::Eof {
                    return self.unexpected();
                }
                let m = self.parse_class_member(derived)?;
                if let ClassMemberKind::Method {
                    kind: MethodKind::Constructor,
                    ..
                } = m.kind
                {
                    if has_ctor {
                        return self.err("a class can only have one constructor", m.span);
                    }
                    has_ctor = true;
                }
                body.push(m);
            }
            self.bump()?;
            Ok(body)
        })();
        self.no_in = saved_in;
        let body = r?;
        self.close_class()?;
        Ok(Class {
            id,
            super_class,
            body,
            span: self.finish(start),
        })
    }

    fn parse_class_member(&mut self, derived: bool) -> PResult<ClassMember> {
        let start = self.cur.span;
        let mut is_static = false;
        if self.at_word("static") {
            let p = self.peek()?;
            if p.is_punct(Punct::LBrace) {
                self.bump()?;
                let body = self.parse_static_block()?;
                return Ok(ClassMember {
                    kind: ClassMemberKind::StaticBlock(body),
                    span: self.finish(start),
                });
            }
            if starts_key(&p, true) || p.is_punct(Punct::Star) {
                self.bump()?;
                is_static = true;
            }
        }
        let mut is_async = false;
        if self.at_word("async") {
            let p = self.peek()?;
            if (starts_key(&p, true) || p.is_punct(Punct::Star)) && !p.newline_before {
                self.bump()?;
                is_async = true;
            }
        }
        let is_generator = self.eat(Punct::Star)?;
        let mut accessor = None;
        if !is_async && !is_generator && (self.at_word("get") || self.at_word("set")) {
            let p = self.peek()?;
            if starts_key(&p, true) {
                accessor = Some(if self.at_word("get") {
                    MethodKind::Get
                } else {
                    MethodKind::Set
                });
                self.bump()?;
            }
        }
        let key = self.parse_prop_key(true)?;
        let name = key_name(&key);
        if self.at(Punct::LParen) {
            let is_ctor = !is_static && name.as_deref() == Some("constructor");
            if is_ctor && (accessor.is_some() || is_async || is_generator) {
                return self.err(
                    "the constructor can't be a getter, setter, async, or generator",
                    start,
                );
            }
            if is_static && name.as_deref() == Some("prototype") {
                return self.err("a static method can't be named `prototype`", start);
            }
            let (kind, flavor) = if is_ctor {
                (MethodKind::Constructor, Flavor::Constructor { derived })
            } else {
                match accessor {
                    Some(MethodKind::Get) => (MethodKind::Get, Flavor::Getter),
                    Some(MethodKind::Set) => (MethodKind::Set, Flavor::Setter),
                    _ => (MethodKind::Method, Flavor::Method),
                }
            };
            if let PropKey::Private(p) = &key {
                let pk = match kind {
                    MethodKind::Get => PrivKind::Get,
                    MethodKind::Set => PrivKind::Set,
                    _ => PrivKind::Method,
                };
                self.declare_private(p, pk, is_static)?;
            }
            let fstart = self.cur.span;
            let value = self.parse_function_rest(fstart, None, is_async, is_generator, flavor)?;
            return Ok(ClassMember {
                kind: ClassMemberKind::Method {
                    key,
                    kind,
                    is_static,
                    value: Box::new(value),
                },
                span: self.finish(start),
            });
        }
        if is_async || is_generator || accessor.is_some() {
            return self.unexpected();
        }
        if name.as_deref() == Some("constructor") {
            return self.err("a field can't be named `constructor`", start);
        }
        if is_static && name.as_deref() == Some("prototype") {
            return self.err("a static field can't be named `prototype`", start);
        }
        if let PropKey::Private(p) = &key {
            self.declare_private(p, PrivKind::Field, is_static)?;
        }
        let value = if self.eat(Punct::Eq)? {
            let outer = self.ctx.clone();
            let saved_marks = (self.last_yield_await, self.last_await_ident);
            self.ctx = Ctx {
                strict: true,
                in_function: true,
                super_prop: true,
                new_target: true,
                class_init: true,
                ..Ctx::default()
            };
            self.push_scope(ScopeKind::Function);
            let e = self.parse_assign();
            self.pop_scope();
            self.ctx = outer;
            self.last_yield_await = saved_marks.0;
            self.last_await_ident = saved_marks.1;
            Some(Box::new(e?))
        } else {
            None
        };
        self.semicolon()?;
        Ok(ClassMember {
            kind: ClassMemberKind::Field {
                key,
                is_static,
                value,
            },
            span: self.finish(start),
        })
    }

    fn parse_static_block(&mut self) -> PResult<Vec<Stmt>> {
        let outer = self.ctx.clone();
        let saved_marks = (self.last_yield_await, self.last_await_ident);
        self.ctx = Ctx {
            strict: true,
            super_prop: true,
            new_target: true,
            class_init: true,
            static_block: true,
            ..Ctx::default()
        };
        self.push_scope(ScopeKind::Function);
        let r = (|| -> PResult<Vec<Stmt>> {
            self.expect(Punct::LBrace)?;
            let mut body = Vec::new();
            while !self.at(Punct::RBrace) {
                if self.cur.kind == TokenKind::Eof {
                    return self.unexpected();
                }
                body.push(self.parse_statement_list_item()?);
            }
            self.bump()?;
            Ok(body)
        })();
        self.pop_scope();
        self.ctx = outer;
        self.last_yield_await = saved_marks.0;
        self.last_await_ident = saved_marks.1;
        r
    }

    fn parse_prop_key(&mut self, allow_private: bool) -> PResult<PropKey> {
        let tok = self.cur.clone();
        match tok.kind {
            TokenKind::Ident(name) => {
                self.bump()?;
                Ok(PropKey::Ident(Ident {
                    name,
                    span: tok.span,
                }))
            }
            TokenKind::String(ref v) => {
                self.check_legacy(&tok)?;
                let v = v.clone();
                self.bump()?;
                Ok(PropKey::Str(v, tok.span))
            }
            TokenKind::Number(n) => {
                self.check_legacy(&tok)?;
                self.bump()?;
                Ok(PropKey::Num(n, tok.span))
            }
            TokenKind::BigInt(ref b) => {
                let b = b.clone();
                self.bump()?;
                Ok(PropKey::BigInt(b, tok.span))
            }
            TokenKind::PrivateName(name) if allow_private => {
                self.bump()?;
                Ok(PropKey::Private(Ident {
                    name,
                    span: tok.span,
                }))
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump()?;
                let saved_in = std::mem::replace(&mut self.no_in, false);
                let e = self.parse_assign();
                self.no_in = saved_in;
                let e = e?;
                self.expect(Punct::RBracket)?;
                Ok(PropKey::Computed(Box::new(e)))
            }
            _ => self.unexpected(),
        }
    }

    // ---- binding patterns ----

    fn parse_binding_target(&mut self) -> PResult<Pat> {
        self.enter()?;
        let r = self.binding_target_inner();
        self.leave();
        r
    }

    fn binding_target_inner(&mut self) -> PResult<Pat> {
        let start = self.cur.span;
        match &self.cur.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.check_binding_ident(&name, start)?;
                self.bump()?;
                Ok(Pat {
                    kind: PatKind::Ident(name),
                    span: start,
                })
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump()?;
                let saved_in = std::mem::replace(&mut self.no_in, false);
                let r = (|| -> PResult<Vec<Option<Pat>>> {
                    let mut items = Vec::new();
                    while !self.at(Punct::RBracket) {
                        if self.eat(Punct::Comma)? {
                            items.push(None);
                            continue;
                        }
                        if self.at(Punct::Ellipsis) {
                            let rstart = self.cur.span;
                            self.bump()?;
                            let t = self.parse_binding_target()?;
                            items.push(Some(Pat {
                                kind: PatKind::Rest(Box::new(t)),
                                span: self.finish(rstart),
                            }));
                            if !self.at(Punct::RBracket) {
                                return self.err("a rest element must be last", self.cur.span);
                            }
                            break;
                        }
                        items.push(Some(self.parse_binding_element()?));
                        if !self.at(Punct::RBracket) {
                            self.expect(Punct::Comma)?;
                        }
                    }
                    self.bump()?;
                    Ok(items)
                })();
                self.no_in = saved_in;
                Ok(Pat {
                    kind: PatKind::Array(r?),
                    span: self.finish(start),
                })
            }
            TokenKind::Punct(Punct::LBrace) => {
                self.bump()?;
                let saved_in = std::mem::replace(&mut self.no_in, false);
                let r = (|| -> PResult<Vec<PatProp>> {
                    let mut props = Vec::new();
                    while !self.at(Punct::RBrace) {
                        if self.at(Punct::Ellipsis) {
                            self.bump()?;
                            let tok = self.cur.clone();
                            let TokenKind::Ident(name) = tok.kind else {
                                return self
                                    .err("an object rest element must be a plain name", tok.span);
                            };
                            self.check_binding_ident(&name, tok.span)?;
                            self.bump()?;
                            props.push(PatProp::Rest(Pat {
                                kind: PatKind::Ident(name),
                                span: tok.span,
                            }));
                            if !self.at(Punct::RBrace) {
                                return self.err("a rest element must be last", self.cur.span);
                            }
                            break;
                        }
                        let ktok = self.cur.clone();
                        let key = self.parse_prop_key(false)?;
                        if self.eat(Punct::Colon)? {
                            let value = self.parse_binding_element()?;
                            props.push(PatProp::Prop {
                                key,
                                value,
                                shorthand: false,
                            });
                        } else {
                            let PropKey::Ident(id) = &key else {
                                return self
                                    .err("expected `:` after the property name", self.cur.span);
                            };
                            self.check_binding_ident(&id.name, id.span)?;
                            let mut value = Pat {
                                kind: PatKind::Ident(id.name.clone()),
                                span: ktok.span,
                            };
                            if self.eat(Punct::Eq)? {
                                let d = self.parse_assign()?;
                                value = Pat {
                                    span: ktok.span.to(d.span),
                                    kind: PatKind::Assign {
                                        target: Box::new(value),
                                        default: Box::new(d),
                                    },
                                };
                            }
                            props.push(PatProp::Prop {
                                key,
                                value,
                                shorthand: true,
                            });
                        }
                        if !self.at(Punct::RBrace) {
                            self.expect(Punct::Comma)?;
                        }
                    }
                    self.bump()?;
                    Ok(props)
                })();
                self.no_in = saved_in;
                Ok(Pat {
                    kind: PatKind::Object(r?),
                    span: self.finish(start),
                })
            }
            _ => self.unexpected(),
        }
    }

    fn parse_binding_element(&mut self) -> PResult<Pat> {
        let start = self.cur.span;
        let target = self.parse_binding_target()?;
        if self.eat(Punct::Eq)? {
            let saved_in = std::mem::replace(&mut self.no_in, false);
            let d = self.parse_assign();
            self.no_in = saved_in;
            let d = d?;
            return Ok(Pat {
                kind: PatKind::Assign {
                    target: Box::new(target),
                    default: Box::new(d),
                },
                span: self.finish(start),
            });
        }
        Ok(target)
    }

    // ---- expressions ----

    fn parse_expression_in(&mut self) -> PResult<Expr> {
        let saved = std::mem::replace(&mut self.no_in, false);
        let r = self.parse_expression();
        self.no_in = saved;
        r
    }

    fn parse_assign_in(&mut self) -> PResult<Expr> {
        let saved = std::mem::replace(&mut self.no_in, false);
        let r = self.parse_assign();
        self.no_in = saved;
        r
    }

    fn parse_expression(&mut self) -> PResult<Expr> {
        let first = self.parse_assign()?;
        if !self.at(Punct::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat(Punct::Comma)? {
            items.push(self.parse_assign()?);
        }
        let span = items[0].span.to(items[items.len() - 1].span);
        Ok(Expr {
            kind: ExprKind::Seq(items),
            span,
        })
    }

    fn parse_assign(&mut self) -> PResult<Expr> {
        self.parse_assign_inner(false)
    }

    fn parse_assign_cover(&mut self) -> PResult<Expr> {
        self.parse_assign_inner(true)
    }

    fn parse_assign_inner(&mut self, cover: bool) -> PResult<Expr> {
        self.enter()?;
        let r = self.assign_body(cover);
        self.leave();
        r
    }

    fn assign_body(&mut self, cover: bool) -> PResult<Expr> {
        if self.ctx.is_generator && self.at_word("yield") {
            return self.parse_yield();
        }
        let saved_cover = self.cover_err.take();
        let lhs = self.parse_conditional()?;
        if is_arrow(&lhs) {
            self.cover_err = saved_cover;
            return Ok(lhs);
        }
        let op = self.current_assign_op();
        if let Some(op) = op {
            return self.finish_assign(lhs, op, saved_cover);
        }
        if let Some(e) = self.cover_err.take() {
            if cover && matches!(lhs.kind, ExprKind::Object(_) | ExprKind::Array(_)) {
                self.cover_err = saved_cover.or(Some(e));
                return Ok(lhs);
            }
            return Err(e);
        }
        self.cover_err = saved_cover;
        Ok(lhs)
    }

    fn finish_assign(
        &mut self,
        lhs: Expr,
        op: AssignOp,
        saved_cover: Option<SyntaxError>,
    ) -> PResult<Expr> {
        let target = if op == AssignOp::Assign
            && matches!(lhs.kind, ExprKind::Object(_) | ExprKind::Array(_))
        {
            let p = self.expr_to_pattern(lhs, false)?;
            self.cover_err = None;
            p
        } else {
            if let Some(e) = self.cover_err.take() {
                return Err(e);
            }
            self.simple_target(lhs)?
        };
        self.bump()?;
        let value = self.parse_assign()?;
        self.cover_err = saved_cover;
        let span = target.span.to(value.span);
        Ok(Expr {
            kind: ExprKind::Assign {
                op,
                target: Box::new(target),
                value: Box::new(value),
            },
            span,
        })
    }

    fn current_assign_op(&self) -> Option<AssignOp> {
        match &self.cur.kind {
            TokenKind::Punct(p) => match p {
                Punct::Eq => Some(AssignOp::Assign),
                Punct::PlusEq => Some(AssignOp::Add),
                Punct::MinusEq => Some(AssignOp::Sub),
                Punct::StarEq => Some(AssignOp::Mul),
                Punct::SlashEq => Some(AssignOp::Div),
                Punct::PercentEq => Some(AssignOp::Mod),
                Punct::StarStarEq => Some(AssignOp::Exp),
                Punct::ShlEq => Some(AssignOp::Shl),
                Punct::ShrEq => Some(AssignOp::Shr),
                Punct::UShrEq => Some(AssignOp::UShr),
                Punct::AmpEq => Some(AssignOp::BitAnd),
                Punct::PipeEq => Some(AssignOp::BitOr),
                Punct::CaretEq => Some(AssignOp::BitXor),
                Punct::AmpAmpEq => Some(AssignOp::And),
                Punct::PipePipeEq => Some(AssignOp::Or),
                Punct::QuestionQuestionEq => Some(AssignOp::Nullish),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_yield(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        self.last_yield_await = Some(start.start);
        let no_arg = self.cur.newline_before
            || self.cur.kind == TokenKind::Eof
            || matches!(
                self.cur.kind,
                TokenKind::Punct(
                    Punct::RParen
                        | Punct::RBracket
                        | Punct::RBrace
                        | Punct::Comma
                        | Punct::Semicolon
                        | Punct::Colon
                        | Punct::Question
                )
            )
            || self.at_word("in")
            || self.at_word("of");
        if no_arg {
            return Ok(Expr {
                kind: ExprKind::Yield {
                    arg: None,
                    delegate: false,
                },
                span: start,
            });
        }
        let delegate = self.eat(Punct::Star)?;
        let arg = self.parse_assign()?;
        Ok(Expr {
            kind: ExprKind::Yield {
                arg: Some(Box::new(arg)),
                delegate,
            },
            span: self.finish(start),
        })
    }

    fn parse_conditional(&mut self) -> PResult<Expr> {
        let test = self.parse_binary(0)?;
        if is_arrow(&test) || !self.at(Punct::Question) {
            return Ok(test);
        }
        self.bump()?;
        let cons = self.parse_assign_in()?;
        self.expect(Punct::Colon)?;
        let alt = self.parse_assign()?;
        let span = test.span.to(alt.span);
        Ok(Expr {
            kind: ExprKind::Cond {
                test: Box::new(test),
                cons: Box::new(cons),
                alt: Box::new(alt),
            },
            span,
        })
    }

    fn current_binop(&self) -> Option<(u8, Result<BinaryOp, LogicalOp>)> {
        use Punct::*;
        let r = match &self.cur.kind {
            TokenKind::Punct(p) => match p {
                QuestionQuestion => (1, Err(LogicalOp::Nullish)),
                PipePipe => (2, Err(LogicalOp::Or)),
                AmpAmp => (3, Err(LogicalOp::And)),
                Pipe => (4, Ok(BinaryOp::BitOr)),
                Caret => (5, Ok(BinaryOp::BitXor)),
                Amp => (6, Ok(BinaryOp::BitAnd)),
                EqEq => (7, Ok(BinaryOp::Eq)),
                NotEq => (7, Ok(BinaryOp::NotEq)),
                EqEqEq => (7, Ok(BinaryOp::StrictEq)),
                NotEqEq => (7, Ok(BinaryOp::StrictNotEq)),
                Lt => (8, Ok(BinaryOp::Lt)),
                Gt => (8, Ok(BinaryOp::Gt)),
                LtEq => (8, Ok(BinaryOp::LtEq)),
                GtEq => (8, Ok(BinaryOp::GtEq)),
                Shl => (9, Ok(BinaryOp::Shl)),
                Shr => (9, Ok(BinaryOp::Shr)),
                UShr => (9, Ok(BinaryOp::UShr)),
                Plus => (10, Ok(BinaryOp::Add)),
                Minus => (10, Ok(BinaryOp::Sub)),
                Star => (11, Ok(BinaryOp::Mul)),
                Slash => (11, Ok(BinaryOp::Div)),
                Percent => (11, Ok(BinaryOp::Mod)),
                StarStar => (12, Ok(BinaryOp::Exp)),
                _ => return None,
            },
            TokenKind::Ident(_) if self.at_word("instanceof") => (8, Ok(BinaryOp::Instanceof)),
            TokenKind::Ident(_) if self.at_word("in") && !self.no_in => (8, Ok(BinaryOp::In)),
            _ => return None,
        };
        Some(r)
    }

    fn parse_binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut left = if let TokenKind::PrivateName(name) = &self.cur.kind {
            let name = Ident {
                name: name.clone(),
                span: self.cur.span,
            };
            if min_prec > 8 {
                return self.unexpected();
            }
            self.bump()?;
            if !self.at_word("in") || self.no_in {
                return self.err("a private name here must be followed by `in`", name.span);
            }
            self.reference_private(&name.name, name.span)?;
            self.bump()?;
            self.enter()?;
            let right = self.parse_binary(9);
            self.leave();
            let right = right?;
            let span = name.span.to(right.span);
            Expr {
                kind: ExprKind::PrivateIn {
                    name,
                    right: Box::new(right),
                },
                span,
            }
        } else {
            self.parse_unary()?
        };
        let mut links = 0u32;
        let r = (|| -> PResult<Expr> {
            loop {
                if is_arrow(&left) {
                    break;
                }
                let Some((prec, op)) = self.current_binop() else {
                    break;
                };
                if prec < min_prec {
                    break;
                }
                if op == Ok(BinaryOp::Exp)
                    && matches!(left.kind, ExprKind::Unary { .. } | ExprKind::Await(_))
                {
                    return self.err(
                        "put parentheses around a unary expression before `**`",
                        left.span,
                    );
                }
                self.bump()?;
                self.enter()?;
                links += 1;
                let right = if op == Ok(BinaryOp::Exp) {
                    self.parse_binary(prec)?
                } else {
                    self.parse_binary(prec + 1)?
                };
                if is_arrow(&right) {
                    return self.err("an arrow function can't be an operand here", right.span);
                }
                let span = left.span.to(right.span);
                let l = Box::new(std::mem::replace(
                    &mut left,
                    Expr {
                        kind: ExprKind::Null,
                        span,
                    },
                ));
                let kind = match op {
                    Ok(bop) => ExprKind::Binary {
                        op: bop,
                        left: l,
                        right: Box::new(right),
                    },
                    Err(lop) => {
                        let mixes = |e: &Expr| {
                            matches!(
                                (&e.kind, lop),
                                (
                                    ExprKind::Logical {
                                        op: LogicalOp::Nullish,
                                        ..
                                    },
                                    LogicalOp::And | LogicalOp::Or
                                ) | (
                                    ExprKind::Logical {
                                        op: LogicalOp::And | LogicalOp::Or,
                                        ..
                                    },
                                    LogicalOp::Nullish
                                )
                            )
                        };
                        if mixes(&l) || mixes(&right) {
                            return self.err(
                                "`??` can't be mixed with `&&` or `||` without parentheses",
                                span,
                            );
                        }
                        ExprKind::Logical {
                            op: lop,
                            left: l,
                            right: Box::new(right),
                        }
                    }
                };
                left = Expr { kind, span };
            }
            Ok(left)
        })();
        self.depth = self.depth.saturating_sub(links);
        r
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let prefix = match &self.cur.kind {
            TokenKind::Punct(
                Punct::Bang
                | Punct::Tilde
                | Punct::Plus
                | Punct::Minus
                | Punct::PlusPlus
                | Punct::MinusMinus,
            ) => true,
            TokenKind::Ident(_) => {
                self.at_word("typeof")
                    || self.at_word("void")
                    || self.at_word("delete")
                    || (self.at_word("await") && self.await_is_keyword())
            }
            _ => false,
        };
        if prefix {
            self.unary_prefix()
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        let e = self.parse_lhs()?;
        if (self.at(Punct::PlusPlus) || self.at(Punct::MinusMinus))
            && !self.cur.newline_before
            && !is_arrow(&e)
        {
            return self.finish_postfix(start, e);
        }
        Ok(e)
    }

    fn finish_postfix(&mut self, start: Span, e: Expr) -> PResult<Expr> {
        let op = if self.at(Punct::PlusPlus) {
            UpdateOp::Inc
        } else {
            UpdateOp::Dec
        };
        self.check_simple_target(&e)?;
        self.bump()?;
        Ok(Expr {
            kind: ExprKind::Update {
                op,
                prefix: false,
                arg: Box::new(e),
            },
            span: self.finish(start),
        })
    }

    fn unary_prefix(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        let op = match &self.cur.kind {
            TokenKind::Punct(Punct::Bang) => Some(UnaryOp::Not),
            TokenKind::Punct(Punct::Tilde) => Some(UnaryOp::BitNot),
            TokenKind::Punct(Punct::Plus) => Some(UnaryOp::Plus),
            TokenKind::Punct(Punct::Minus) => Some(UnaryOp::Minus),
            TokenKind::Ident(_) if self.at_word("typeof") => Some(UnaryOp::Typeof),
            TokenKind::Ident(_) if self.at_word("void") => Some(UnaryOp::Void),
            TokenKind::Ident(_) if self.at_word("delete") => Some(UnaryOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.bump()?;
            self.enter()?;
            let arg = self.parse_unary();
            self.leave();
            let arg = arg?;
            if is_arrow(&arg) {
                return self.err("an arrow function can't be an operand here", arg.span);
            }
            if op == UnaryOp::Delete {
                let inner = unparen(&arg);
                if self.ctx.strict && matches!(inner.kind, ExprKind::Ident(_)) {
                    return self.err(
                        "deleting a plain name is not allowed in strict mode",
                        arg.span,
                    );
                }
                let mut probe = inner;
                if let ExprKind::OptionalChain(c) = &probe.kind {
                    probe = c;
                }
                if let ExprKind::Member {
                    prop: MemberProp::Private(_),
                    ..
                } = &probe.kind
                {
                    return self.err("private fields can't be deleted", arg.span);
                }
            }
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op,
                    arg: Box::new(arg),
                },
                span: self.finish(start),
            });
        }
        if self.at(Punct::PlusPlus) || self.at(Punct::MinusMinus) {
            let op = if self.at(Punct::PlusPlus) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            self.bump()?;
            self.enter()?;
            let arg = self.parse_unary();
            self.leave();
            let arg = arg?;
            self.check_simple_target(&arg)?;
            return Ok(Expr {
                kind: ExprKind::Update {
                    op,
                    prefix: true,
                    arg: Box::new(arg),
                },
                span: self.finish(start),
            });
        }
        if self.at_word("await") && self.await_is_keyword() {
            self.bump()?;
            self.last_yield_await = Some(start.start);
            self.enter()?;
            let arg = self.parse_unary();
            self.leave();
            let arg = arg?;
            if is_arrow(&arg) {
                return self.err("an arrow function can't be an operand here", arg.span);
            }
            return Ok(Expr {
                kind: ExprKind::Await(Box::new(arg)),
                span: self.finish(start),
            });
        }
        self.parse_postfix()
    }

    fn check_simple_target(&self, e: &Expr) -> PResult<()> {
        match &unparen(e).kind {
            ExprKind::Ident(name) => {
                if self.ctx.strict && (name == "eval" || name == "arguments") {
                    return self.err(format!("can't assign to `{name}` in strict mode"), e.span);
                }
                Ok(())
            }
            ExprKind::Member { .. } => Ok(()),
            _ => self.err("invalid assignment target", e.span),
        }
    }

    fn simple_target(&self, e: Expr) -> PResult<Pat> {
        self.check_simple_target(&e)?;
        let span = e.span;
        if let ExprKind::Ident(name) = &e.kind {
            return Ok(Pat {
                kind: PatKind::Ident(name.clone()),
                span,
            });
        }
        Ok(Pat {
            kind: PatKind::Expr(Box::new(e)),
            span,
        })
    }

    /// Target of a for-in/for-of head: a simple target or a destructuring
    /// pattern.
    fn expr_to_target(&mut self, e: Expr) -> PResult<Pat> {
        if matches!(e.kind, ExprKind::Object(_) | ExprKind::Array(_)) {
            self.expr_to_pattern(e, false)
        } else {
            self.simple_target(e)
        }
    }

    /// Turn a cover expression into a pattern. `binding` is true for arrow
    /// parameters, where only names are allowed as targets.
    fn expr_to_pattern(&mut self, e: Expr, binding: bool) -> PResult<Pat> {
        self.enter()?;
        let r = self.expr_to_pattern_inner(e, binding);
        self.leave();
        r
    }

    fn expr_to_pattern_inner(&mut self, mut e: Expr, binding: bool) -> PResult<Pat> {
        let span = e.span;
        if !binding && matches!(e.kind, ExprKind::Member { .. }) {
            return Ok(Pat {
                kind: PatKind::Expr(Box::new(e)),
                span,
            });
        }
        if !binding && matches!(e.kind, ExprKind::Paren(_)) {
            let inner = unparen(&e);
            if matches!(inner.kind, ExprKind::Ident(_) | ExprKind::Member { .. }) {
                return self.simple_target(e);
            }
            return self.err("invalid destructuring target", span);
        }
        match e.take_kind() {
            ExprKind::Ident(name) => {
                if binding {
                    self.check_binding_ident(&name, span)?;
                } else if self.ctx.strict && (name == "eval" || name == "arguments") {
                    return self.err(format!("can't assign to `{name}` in strict mode"), span);
                }
                Ok(Pat {
                    kind: PatKind::Ident(name),
                    span,
                })
            }
            ExprKind::Array(items) => {
                let n = items.len();
                let mut out = Vec::with_capacity(n);
                for (i, item) in items.into_iter().enumerate() {
                    match item {
                        None => out.push(None),
                        Some(mut x) if matches!(x.kind, ExprKind::Spread(_)) => {
                            let sspan = x.span;
                            let ExprKind::Spread(arg) = x.take_kind() else {
                                return self.err("invalid destructuring target", sspan);
                            };
                            if i + 1 != n || self.bad_rest.contains(&sspan.start) {
                                return self.err("a rest element must be last", sspan);
                            }
                            if matches!(arg.kind, ExprKind::Assign { .. }) {
                                return self.err("a rest element can't have a default", sspan);
                            }
                            let t = self.expr_to_pattern(*arg, binding)?;
                            out.push(Some(Pat {
                                kind: PatKind::Rest(Box::new(t)),
                                span: sspan,
                            }));
                        }
                        Some(x) => out.push(Some(self.expr_to_pattern(x, binding)?)),
                    }
                }
                Ok(Pat {
                    kind: PatKind::Array(out),
                    span,
                })
            }
            ExprKind::Object(props) => {
                let n = props.len();
                let mut out = Vec::with_capacity(n);
                for (i, prop) in props.into_iter().enumerate() {
                    match prop.kind {
                        PropKind::Init { key, value } => {
                            let shorthand = matches!(
                                (&key, &value.kind),
                                (PropKey::Ident(k), ExprKind::Assign { target, .. }) if target.span == k.span
                            );
                            let value = self.expr_to_pattern(*value, binding)?;
                            out.push(PatProp::Prop {
                                key,
                                value,
                                shorthand,
                            });
                        }
                        PropKind::Shorthand(id) => {
                            if binding {
                                self.check_binding_ident(&id.name, id.span)?;
                            } else if self.ctx.strict
                                && (id.name == "eval" || id.name == "arguments")
                            {
                                return self.err(
                                    format!("can't assign to `{}` in strict mode", id.name),
                                    id.span,
                                );
                            }
                            let value = Pat {
                                kind: PatKind::Ident(id.name.clone()),
                                span: id.span,
                            };
                            out.push(PatProp::Prop {
                                key: PropKey::Ident(id),
                                value,
                                shorthand: true,
                            });
                        }
                        PropKind::Spread(arg) => {
                            if i + 1 != n || self.bad_rest.contains(&prop.span.start) {
                                return self.err("a rest element must be last", prop.span);
                            }
                            let ok = match &arg.kind {
                                ExprKind::Ident(_) => true,
                                ExprKind::Member { .. } | ExprKind::Paren(_) => !binding,
                                _ => false,
                            };
                            if !ok {
                                return self
                                    .err("an object rest element must be a name", prop.span);
                            }
                            out.push(PatProp::Rest(self.expr_to_pattern(*arg, binding)?));
                        }
                        PropKind::Method { .. } => {
                            return self.err("a method can't be a destructuring target", prop.span);
                        }
                    }
                }
                Ok(Pat {
                    kind: PatKind::Object(out),
                    span,
                })
            }
            ExprKind::Assign {
                op: AssignOp::Assign,
                target,
                value,
            } => {
                if binding {
                    self.validate_binding_pat(&target)?;
                }
                Ok(Pat {
                    kind: PatKind::Assign {
                        target,
                        default: value,
                    },
                    span,
                })
            }
            _ => self.err("invalid destructuring target", span),
        }
    }

    fn validate_binding_pat(&mut self, p: &Pat) -> PResult<()> {
        self.enter()?;
        let r = (|| -> PResult<()> {
            match &p.kind {
                PatKind::Ident(n) => self.check_binding_ident(n, p.span),
                PatKind::Expr(_) => self.err("invalid binding target", p.span),
                PatKind::Array(items) => {
                    for i in items.iter().flatten() {
                        self.validate_binding_pat(i)?;
                    }
                    Ok(())
                }
                PatKind::Object(props) => {
                    for prop in props {
                        match prop {
                            PatProp::Prop { value, .. } => self.validate_binding_pat(value)?,
                            PatProp::Rest(r) => self.validate_binding_pat(r)?,
                        }
                    }
                    Ok(())
                }
                PatKind::Assign { target, .. } => self.validate_binding_pat(target),
                PatKind::Rest(t) => self.validate_binding_pat(t),
            }
        })();
        self.leave();
        r
    }

    // ---- member, call, new ----

    fn parse_lhs(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        let e = if self.at_word("new") {
            self.parse_new()?
        } else if self.at_word("super") {
            self.parse_super(true)?
        } else if self.at_word("import") {
            self.parse_import_expr()?
        } else {
            self.parse_primary()?
        };
        if is_arrow(&e) {
            return Ok(e);
        }
        self.parse_call_tail(e, start, true)
    }

    fn parse_super(&mut self, allow_call: bool) -> PResult<Expr> {
        let tok = self.bump()?;
        if self.at(Punct::LParen) && allow_call {
            if !self.ctx.super_call {
                return self.err(
                    "`super()` is only allowed in a derived class constructor",
                    tok.span,
                );
            }
        } else if self.at(Punct::Dot) || self.at(Punct::LBracket) {
            if !self.ctx.super_prop {
                return self.err("`super` properties are only allowed in methods", tok.span);
            }
        } else {
            return self.err("`super` must be followed by `(`, `.`, or `[`", tok.span);
        }
        Ok(Expr {
            kind: ExprKind::Super,
            span: tok.span,
        })
    }

    fn parse_new(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        if self.at(Punct::Dot) {
            self.bump()?;
            if !self.at_word("target") {
                return self.err("expected `new.target`", self.cur.span);
            }
            self.bump()?;
            if !self.ctx.new_target {
                return self.err("`new.target` is only allowed in functions", start);
            }
            return Ok(Expr {
                kind: ExprKind::NewTarget,
                span: self.finish(start),
            });
        }
        let cstart = self.cur.span;
        let callee = if self.at_word("new") {
            self.enter()?;
            let c = self.parse_new();
            self.leave();
            c?
        } else if self.at_word("super") {
            self.parse_super(false)?
        } else if self.at_word("import") {
            if !self.peek()?.is_punct(Punct::Dot) {
                return self.err("`new import(...)` is not allowed", self.cur.span);
            }
            self.parse_import_expr()?
        } else {
            self.parse_primary()?
        };
        if is_arrow(&callee) {
            return self.err("an arrow function can't be used with `new`", callee.span);
        }
        let callee = self.parse_call_tail(callee, cstart, false)?;
        let args = if self.at(Punct::LParen) {
            self.parse_args()?
        } else {
            Vec::new()
        };
        Ok(Expr {
            kind: ExprKind::New {
                callee: Box::new(callee),
                args,
            },
            span: self.finish(start),
        })
    }

    fn parse_call_tail(&mut self, e: Expr, start: Span, allow_call: bool) -> PResult<Expr> {
        let mut links = 0u32;
        let r = self.call_tail_inner(e, start, allow_call, &mut links);
        self.depth = self.depth.saturating_sub(links);
        r
    }

    fn call_tail_inner(
        &mut self,
        mut e: Expr,
        start: Span,
        allow_call: bool,
        links: &mut u32,
    ) -> PResult<Expr> {
        let mut in_chain = false;
        let is_super = matches!(e.kind, ExprKind::Super);
        loop {
            let kind = match &self.cur.kind {
                TokenKind::Punct(Punct::Dot) => {
                    self.bump()?;
                    let prop = self.parse_member_name()?;
                    ExprKind::Member {
                        object: Box::new(e),
                        prop,
                        optional: false,
                    }
                }
                TokenKind::Punct(Punct::QuestionDot) => {
                    if !allow_call {
                        return self.err(
                            "optional chaining is not allowed in a `new` callee",
                            self.cur.span,
                        );
                    }
                    if is_super && !in_chain && matches!(e.kind, ExprKind::Super) {
                        return self.err("`super?.` is not allowed", self.cur.span);
                    }
                    self.bump()?;
                    in_chain = true;
                    match &self.cur.kind {
                        TokenKind::Punct(Punct::LParen) => {
                            let args = self.parse_args()?;
                            ExprKind::Call {
                                callee: Box::new(e),
                                args,
                                optional: true,
                            }
                        }
                        TokenKind::Punct(Punct::LBracket) => {
                            self.bump()?;
                            let p = self.parse_expression_in()?;
                            self.expect(Punct::RBracket)?;
                            ExprKind::Member {
                                object: Box::new(e),
                                prop: MemberProp::Computed(Box::new(p)),
                                optional: true,
                            }
                        }
                        TokenKind::Template(_) => {
                            return self.err("a tagged template can't follow `?.`", self.cur.span);
                        }
                        _ => {
                            let prop = self.parse_member_name()?;
                            ExprKind::Member {
                                object: Box::new(e),
                                prop,
                                optional: true,
                            }
                        }
                    }
                }
                TokenKind::Punct(Punct::LBracket) => {
                    self.bump()?;
                    let p = self.parse_expression_in()?;
                    self.expect(Punct::RBracket)?;
                    ExprKind::Member {
                        object: Box::new(e),
                        prop: MemberProp::Computed(Box::new(p)),
                        optional: false,
                    }
                }
                TokenKind::Template(_) => {
                    if in_chain {
                        return self.err("a tagged template can't follow `?.`", self.cur.span);
                    }
                    let quasi = self.parse_template(true)?;
                    ExprKind::TaggedTemplate {
                        tag: Box::new(e),
                        quasi,
                    }
                }
                TokenKind::Punct(Punct::LParen) if allow_call => {
                    let args = self.parse_args()?;
                    ExprKind::Call {
                        callee: Box::new(e),
                        args,
                        optional: false,
                    }
                }
                _ => break,
            };
            self.enter()?;
            *links += 1;
            e = Expr {
                kind,
                span: self.finish(start),
            };
        }
        if in_chain {
            let span = e.span;
            e = Expr {
                kind: ExprKind::OptionalChain(Box::new(e)),
                span,
            };
        }
        Ok(e)
    }

    fn parse_member_name(&mut self) -> PResult<MemberProp> {
        let tok = self.cur.clone();
        match tok.kind {
            TokenKind::Ident(name) => {
                self.bump()?;
                Ok(MemberProp::Name(Ident {
                    name,
                    span: tok.span,
                }))
            }
            TokenKind::PrivateName(name) => {
                self.reference_private(&name, tok.span)?;
                self.bump()?;
                Ok(MemberProp::Private(Ident {
                    name,
                    span: tok.span,
                }))
            }
            _ => self.unexpected(),
        }
    }

    fn parse_args(&mut self) -> PResult<Vec<Expr>> {
        self.expect(Punct::LParen)?;
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = (|| -> PResult<Vec<Expr>> {
            let mut args = Vec::new();
            while !self.at(Punct::RParen) {
                if self.at(Punct::Ellipsis) {
                    let s = self.cur.span;
                    self.bump()?;
                    let a = self.parse_assign()?;
                    args.push(Expr {
                        span: s.to(a.span),
                        kind: ExprKind::Spread(Box::new(a)),
                    });
                } else {
                    args.push(self.parse_assign()?);
                }
                if !self.eat(Punct::Comma)? {
                    break;
                }
            }
            self.expect(Punct::RParen)?;
            Ok(args)
        })();
        self.no_in = saved_in;
        r
    }

    fn parse_import_expr(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        if self.eat(Punct::Dot)? {
            if !self.at_word("meta") {
                return self.err("expected `import.meta`", self.cur.span);
            }
            self.bump()?;
            if self.goal != Goal::Module {
                return self.err("`import.meta` is only allowed in modules", start);
            }
            return Ok(Expr {
                kind: ExprKind::ImportMeta,
                span: self.finish(start),
            });
        }
        if !self.at(Punct::LParen) {
            return self.err("expected `(` after `import`", self.cur.span);
        }
        self.bump()?;
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = (|| -> PResult<(Expr, Option<Box<Expr>>)> {
            let source = self.parse_assign()?;
            let mut options = None;
            if self.eat(Punct::Comma)? && !self.at(Punct::RParen) {
                options = Some(Box::new(self.parse_assign()?));
                self.eat(Punct::Comma)?;
            }
            self.expect(Punct::RParen)?;
            Ok((source, options))
        })();
        self.no_in = saved_in;
        let (source, options) = r?;
        Ok(Expr {
            kind: ExprKind::Import {
                source: Box::new(source),
                options,
            },
            span: self.finish(start),
        })
    }

    // ---- primary expressions ----

    fn parse_primary(&mut self) -> PResult<Expr> {
        // Kept small: debug builds give every local its own stack slot, and
        // this frame sits on the path of every nesting level.
        match &self.cur.kind {
            TokenKind::Punct(Punct::LParen) => self.parse_paren(),
            TokenKind::Punct(Punct::LBracket) => self.parse_array(),
            TokenKind::Punct(Punct::LBrace) => self.parse_object(),
            _ => self.primary_other(),
        }
    }

    fn primary_other(&mut self) -> PResult<Expr> {
        let tok = self.cur.clone();
        let start = tok.span;
        match &tok.kind {
            TokenKind::Ident(name) => {
                if !tok.escaped {
                    match name.as_str() {
                        "this" => {
                            self.bump()?;
                            return Ok(Expr {
                                kind: ExprKind::This,
                                span: start,
                            });
                        }
                        "null" => {
                            self.bump()?;
                            return Ok(Expr {
                                kind: ExprKind::Null,
                                span: start,
                            });
                        }
                        "true" | "false" => {
                            self.bump()?;
                            return Ok(Expr {
                                kind: ExprKind::Bool(name == "true"),
                                span: start,
                            });
                        }
                        "function" => {
                            let f = self.parse_function(start, false, false, false)?;
                            return Ok(Expr {
                                kind: ExprKind::Function(Box::new(f)),
                                span: self.finish(start),
                            });
                        }
                        "class" => {
                            let c = self.parse_class(false)?;
                            return Ok(Expr {
                                kind: ExprKind::Class(Box::new(c)),
                                span: self.finish(start),
                            });
                        }
                        "async" => {
                            if let Some(e) = self.try_async(start)? {
                                return Ok(e);
                            }
                        }
                        _ => {}
                    }
                }
                let name = name.clone();
                self.check_ident_ref(&name, start)?;
                if name == "arguments" && self.ctx.class_init {
                    return self.err(
                        "`arguments` is not allowed in class field initializers or static blocks",
                        start,
                    );
                }
                if name == "await" {
                    self.last_await_ident = Some(start.start);
                }
                self.bump()?;
                if self.at(Punct::Arrow) && !self.cur.newline_before {
                    self.check_binding_ident(&name, start)?;
                    let params = vec![Pat {
                        kind: PatKind::Ident(name),
                        span: start,
                    }];
                    return self.parse_arrow_rest(start, params, false, start.start);
                }
                Ok(Expr {
                    kind: ExprKind::Ident(name),
                    span: start,
                })
            }
            TokenKind::Number(n) => {
                self.check_legacy(&tok)?;
                let n = *n;
                self.bump()?;
                Ok(Expr {
                    kind: ExprKind::Num(n),
                    span: start,
                })
            }
            TokenKind::BigInt(b) => {
                let b = b.clone();
                self.bump()?;
                Ok(Expr {
                    kind: ExprKind::BigInt(b),
                    span: start,
                })
            }
            TokenKind::String(v) => {
                self.check_legacy(&tok)?;
                let v = v.clone();
                self.bump()?;
                Ok(Expr {
                    kind: ExprKind::Str(v),
                    span: start,
                })
            }
            TokenKind::Template(_) => {
                let t = self.parse_template(false)?;
                Ok(Expr {
                    kind: ExprKind::Template(t),
                    span: self.finish(start),
                })
            }
            TokenKind::Punct(Punct::Slash) | TokenKind::Punct(Punct::SlashEq) => {
                let re = self.lx.rescan_regex(&tok)?;
                self.cur = re;
                let TokenKind::Regex { pattern, flags } = self.cur.kind.clone() else {
                    return self.unexpected();
                };
                self.bump()?;
                Ok(Expr {
                    kind: ExprKind::Regex { pattern, flags },
                    span: self.finish(start),
                })
            }
            TokenKind::Punct(Punct::LParen) => self.parse_paren(),
            TokenKind::Punct(Punct::LBracket) => self.parse_array(),
            TokenKind::Punct(Punct::LBrace) => self.parse_object(),
            _ => self.unexpected(),
        }
    }

    /// `async function`, `async x => ...`, and `async (...) => ...`.
    /// Returns `None` when `async` is just a name.
    fn try_async(&mut self, start: Span) -> PResult<Option<Expr>> {
        let p = self.peek()?;
        if p.newline_before {
            return Ok(None);
        }
        if p.is_word("function") {
            self.bump()?;
            let f = self.parse_function(start, true, false, false)?;
            return Ok(Some(Expr {
                kind: ExprKind::Function(Box::new(f)),
                span: self.finish(start),
            }));
        }
        if let TokenKind::Ident(_) = p.kind {
            self.bump()?;
            let tok = self.bump()?;
            let TokenKind::Ident(name) = tok.kind else {
                return self.unexpected();
            };
            if name == "await" {
                return self.err("`await` can't be a parameter name here", tok.span);
            }
            let saved = self.ctx.clone();
            self.ctx.is_async = true;
            let r = self.check_binding_ident(&name, tok.span);
            self.ctx = saved;
            r?;
            if !self.at(Punct::Arrow) {
                return self.err("expected `=>` after async arrow parameter", self.cur.span);
            }
            let params = vec![Pat {
                kind: PatKind::Ident(name),
                span: tok.span,
            }];
            return Ok(Some(self.parse_arrow_rest(
                start,
                params,
                true,
                tok.span.start,
            )?));
        }
        if p.is_punct(Punct::LParen) {
            self.bump()?;
            let pstart = self.cur.span;
            self.bump()?;
            let saved_in = std::mem::replace(&mut self.no_in, false);
            let saved_cover = self.cover_err.take();
            let r = self.parse_cover_list();
            self.no_in = saved_in;
            let (items, rest, trailing) = r?;
            if self.at(Punct::Arrow) && !self.cur.newline_before {
                self.cover_err = None;
                let params = self.cover_to_params(items, rest)?;
                let e = self.parse_arrow_rest(start, params, true, pstart.start)?;
                self.cover_err = saved_cover;
                return Ok(Some(e));
            }
            if let Some(e) = self.cover_err.take() {
                return Err(e);
            }
            self.cover_err = saved_cover;
            if rest.is_some() {
                return self.err("expected `=>` after a rest parameter", self.cur.span);
            }
            let _ = trailing;
            let callee = Expr {
                kind: ExprKind::Ident("async".into()),
                span: start,
            };
            let args = items;
            return Ok(Some(Expr {
                kind: ExprKind::Call {
                    callee: Box::new(callee),
                    args,
                    optional: false,
                },
                span: self.finish(start),
            }));
        }
        Ok(None)
    }

    /// After `(`: comma-separated cover items up to `)`. A trailing `...x`
    /// comes back as the rest pattern. Spread items inside the list are kept
    /// as `Spread` expressions (valid for calls).
    fn parse_cover_list(&mut self) -> PResult<(Vec<Expr>, Option<Pat>, bool)> {
        let mut items = Vec::new();
        let mut trailing = false;
        let mut rest = None;
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let s = self.cur.span;
                self.bump()?;
                let a = self.parse_assign_cover()?;
                let spread = Expr {
                    span: s.to(a.span),
                    kind: ExprKind::Spread(Box::new(a)),
                };
                if self.at(Punct::RParen) {
                    rest = Some(spread);
                    break;
                }
                self.bad_rest.insert(s.start);
                items.push(spread);
            } else {
                items.push(self.parse_assign_cover()?);
            }
            if !self.eat(Punct::Comma)? {
                break;
            }
            if self.at(Punct::RParen) {
                trailing = true;
            }
        }
        self.expect(Punct::RParen)?;
        // A final spread is a rest parameter for arrows and a spread argument
        // for calls. Keep it as an expression here; the caller decides.
        let rest_pat = None::<Pat>;
        if let Some(r) = rest {
            items.push(r);
        }
        Ok((items, rest_pat, trailing))
    }

    fn cover_to_params(&mut self, items: Vec<Expr>, rest: Option<Pat>) -> PResult<Vec<Pat>> {
        let n = items.len();
        let mut params = Vec::with_capacity(n + 1);
        for (i, mut item) in items.into_iter().enumerate() {
            if let ExprKind::Spread(_) = item.kind {
                if i + 1 != n || self.bad_rest.contains(&item.span.start) {
                    return self.err("a rest parameter must be last", item.span);
                }
                let span = item.span;
                let ExprKind::Spread(arg) = item.take_kind() else {
                    return self.err("invalid parameter", span);
                };
                if matches!(arg.kind, ExprKind::Assign { .. }) {
                    return self.err("a rest parameter can't have a default", span);
                }
                let t = self.expr_to_pattern(*arg, true)?;
                params.push(Pat {
                    kind: PatKind::Rest(Box::new(t)),
                    span,
                });
            } else {
                params.push(self.expr_to_pattern(item, true)?);
            }
        }
        if let Some(r) = rest {
            params.push(r);
        }
        Ok(params)
    }

    fn parse_paren(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = self.parse_cover_list();
        self.no_in = saved_in;
        let (items, rest, trailing) = r?;
        if self.at(Punct::Arrow) && !self.cur.newline_before {
            self.cover_err = None;
            let params = self.cover_to_params(items, rest)?;
            return self.parse_arrow_rest(start, params, false, start.start);
        }
        if let Some(e) = self.cover_err.take() {
            return Err(e);
        }
        if items.is_empty() || trailing {
            return self.err("expected `=>` after the parameter list", self.cur.span);
        }
        if let Some(s) = items.iter().find(|e| matches!(e.kind, ExprKind::Spread(_))) {
            return self.err("unexpected `...` in a parenthesized expression", s.span);
        }
        let inner = if items.len() == 1 {
            items.into_iter().next().unwrap_or(Expr {
                kind: ExprKind::Null,
                span: start,
            })
        } else {
            let span = items[0].span.to(items[items.len() - 1].span);
            Expr {
                kind: ExprKind::Seq(items),
                span,
            }
        };
        Ok(Expr {
            kind: ExprKind::Paren(Box::new(inner)),
            span: self.finish(start),
        })
    }

    fn parse_array(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = (|| -> PResult<Vec<Option<Expr>>> {
            let mut items = Vec::new();
            while !self.at(Punct::RBracket) {
                if self.eat(Punct::Comma)? {
                    items.push(None);
                    continue;
                }
                let e = if self.at(Punct::Ellipsis) {
                    let s = self.cur.span;
                    self.bump()?;
                    let a = self.parse_assign_cover()?;
                    if self.at(Punct::Comma) {
                        self.bad_rest.insert(s.start);
                    }
                    Expr {
                        span: s.to(a.span),
                        kind: ExprKind::Spread(Box::new(a)),
                    }
                } else {
                    self.parse_assign_cover()?
                };
                items.push(Some(e));
                if !self.at(Punct::RBracket) {
                    self.expect(Punct::Comma)?;
                }
            }
            self.bump()?;
            Ok(items)
        })();
        self.no_in = saved_in;
        Ok(Expr {
            kind: ExprKind::Array(r?),
            span: self.finish(start),
        })
    }

    fn parse_object(&mut self) -> PResult<Expr> {
        let start = self.cur.span;
        self.bump()?;
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let r = (|| -> PResult<Vec<Prop>> {
            let mut props = Vec::new();
            let mut has_proto = false;
            while !self.at(Punct::RBrace) {
                let p = self.parse_object_prop(&mut has_proto)?;
                props.push(p);
                if !self.at(Punct::RBrace) {
                    self.expect(Punct::Comma)?;
                }
            }
            self.bump()?;
            Ok(props)
        })();
        self.no_in = saved_in;
        Ok(Expr {
            kind: ExprKind::Object(r?),
            span: self.finish(start),
        })
    }

    fn parse_object_prop(&mut self, has_proto: &mut bool) -> PResult<Prop> {
        let start = self.cur.span;
        if self.at(Punct::Ellipsis) {
            self.bump()?;
            let a = self.parse_assign_cover()?;
            if self.at(Punct::Comma) {
                self.bad_rest.insert(start.start);
            }
            return Ok(Prop {
                kind: PropKind::Spread(Box::new(a)),
                span: self.finish(start),
            });
        }
        let mut is_async = false;
        if self.at_word("async") {
            let p = self.peek()?;
            if (starts_key(&p, false) || p.is_punct(Punct::Star)) && !p.newline_before {
                self.bump()?;
                is_async = true;
            }
        }
        let is_generator = self.eat(Punct::Star)?;
        let mut accessor = None;
        if !is_async && !is_generator && (self.at_word("get") || self.at_word("set")) {
            let p = self.peek()?;
            if starts_key(&p, false) {
                accessor = Some(if self.at_word("get") {
                    MethodKind::Get
                } else {
                    MethodKind::Set
                });
                self.bump()?;
            }
        }
        let key_tok = self.cur.clone();
        let key = self.parse_prop_key(false)?;
        if is_async || is_generator || accessor.is_some() || self.at(Punct::LParen) {
            let (kind, flavor) = match accessor {
                Some(MethodKind::Get) => (MethodKind::Get, Flavor::Getter),
                Some(MethodKind::Set) => (MethodKind::Set, Flavor::Setter),
                _ => (MethodKind::Method, Flavor::Method),
            };
            let fstart = self.cur.span;
            let f = self.parse_function_rest(fstart, None, is_async, is_generator, flavor)?;
            return Ok(Prop {
                kind: PropKind::Method {
                    key,
                    kind,
                    value: Box::new(f),
                },
                span: self.finish(start),
            });
        }
        if self.eat(Punct::Colon)? {
            if key_name(&key).as_deref() == Some("__proto__")
                && !matches!(key, PropKey::Computed(_))
            {
                if *has_proto && self.cover_err.is_none() {
                    self.cover_err =
                        Some(SyntaxError::new("duplicate `__proto__` property", start));
                }
                *has_proto = true;
            }
            let value = self.parse_assign_cover()?;
            return Ok(Prop {
                kind: PropKind::Init {
                    key,
                    value: Box::new(value),
                },
                span: self.finish(start),
            });
        }
        // Shorthand.
        let PropKey::Ident(id) = key else {
            return self.err("expected `:` after the property name", self.cur.span);
        };
        if key_tok.escaped && is_always_reserved(&id.name) {
            return self.err("a keyword can't contain escapes", id.span);
        }
        self.check_ident_ref(&id.name, id.span)?;
        if id.name == "arguments" && self.ctx.class_init {
            return self.err(
                "`arguments` is not allowed in class field initializers or static blocks",
                id.span,
            );
        }
        if id.name == "await" {
            self.last_await_ident = Some(id.span.start);
        }
        if self.at(Punct::Eq) {
            // Only valid once this object turns into a pattern.
            let eq = self.bump()?;
            let d = self.parse_assign()?;
            if self.cover_err.is_none() {
                self.cover_err = Some(SyntaxError::new(
                    "`=` in an object literal is only allowed in destructuring",
                    eq.span,
                ));
            }
            let target = Pat {
                kind: PatKind::Ident(id.name.clone()),
                span: id.span,
            };
            let span = id.span.to(d.span);
            let value = Expr {
                kind: ExprKind::Assign {
                    op: AssignOp::Assign,
                    target: Box::new(target),
                    value: Box::new(d),
                },
                span,
            };
            return Ok(Prop {
                kind: PropKind::Init {
                    key: PropKey::Ident(id),
                    value: Box::new(value),
                },
                span: self.finish(start),
            });
        }
        Ok(Prop {
            kind: PropKind::Shorthand(id),
            span: self.finish(start),
        })
    }

    fn parse_template(&mut self, tagged: bool) -> PResult<Template> {
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        loop {
            let tok = self.cur.clone();
            let TokenKind::Template(part) = tok.kind else {
                return self.unexpected();
            };
            let TemplatePart {
                cooked,
                bad_escape,
                raw,
                tail,
            } = part;
            if !tagged && cooked.is_none() {
                return self.err(
                    "invalid escape in a template literal",
                    bad_escape.unwrap_or(tok.span),
                );
            }
            quasis.push(TemplateElement {
                cooked,
                raw,
                span: tok.span,
            });
            self.bump()?;
            if tail {
                break;
            }
            let e = self.parse_expression_in()?;
            exprs.push(e);
            if !self.at(Punct::RBrace) {
                return self.err(
                    "expected `}` to close the template substitution",
                    self.cur.span,
                );
            }
            let close = self.cur.clone();
            self.cur = self.lx.rescan_template_continuation(&close)?;
        }
        Ok(Template { quasis, exprs })
    }

    // ---- modules ----

    fn require_module_top(&self, what: &str, span: Span) -> PResult<()> {
        if self.goal != Goal::Module {
            return self.err(
                format!("`{what}` declarations are only allowed in modules"),
                span,
            );
        }
        if self.scopes.len() != 1 || self.ctx.in_function {
            return self.err(
                format!("`{what}` declarations must be at the top level"),
                span,
            );
        }
        Ok(())
    }

    fn parse_string_lit(&mut self) -> PResult<StrLit> {
        let tok = self.cur.clone();
        let TokenKind::String(value) = tok.kind else {
            return self.err(
                format!("expected a string but found {}", describe(&self.cur)),
                tok.span,
            );
        };
        self.check_legacy(&self.cur.clone())?;
        self.bump()?;
        Ok(StrLit {
            value,
            span: tok.span,
        })
    }

    fn parse_module_name(&mut self) -> PResult<ModuleName> {
        match &self.cur.kind {
            TokenKind::String(_) => {
                let s = self.parse_string_lit()?;
                if !well_formed(&s.value) {
                    return self.err("a module export name must be well-formed Unicode", s.span);
                }
                Ok(ModuleName::Str(s))
            }
            TokenKind::Ident(name) => {
                let id = Ident {
                    name: name.clone(),
                    span: self.cur.span,
                };
                self.bump()?;
                Ok(ModuleName::Ident(id))
            }
            _ => self.unexpected(),
        }
    }

    fn parse_from_clause(&mut self) -> PResult<(StrLit, Vec<ImportAttribute>)> {
        self.expect_word("from")?;
        let source = self.parse_string_lit()?;
        let attrs = self.parse_import_attributes()?;
        Ok((source, attrs))
    }

    fn parse_import_attributes(&mut self) -> PResult<Vec<ImportAttribute>> {
        let mut attrs = Vec::new();
        if !self.at_word("with") {
            return Ok(attrs);
        }
        self.bump()?;
        self.expect(Punct::LBrace)?;
        let mut seen = HashSet::new();
        while !self.at(Punct::RBrace) {
            let key = match &self.cur.kind {
                TokenKind::String(_) => ModuleName::Str(self.parse_string_lit()?),
                TokenKind::Ident(name) => {
                    let id = Ident {
                        name: name.clone(),
                        span: self.cur.span,
                    };
                    self.bump()?;
                    ModuleName::Ident(id)
                }
                _ => return self.unexpected(),
            };
            if !seen.insert(key.text()) {
                return self.err(
                    format!("duplicate import attribute `{}`", key.text()),
                    key.span(),
                );
            }
            self.expect(Punct::Colon)?;
            let value = self.parse_string_lit()?;
            attrs.push(ImportAttribute { key, value });
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        self.bump()?;
        Ok(attrs)
    }

    fn import_binding(&mut self) -> PResult<Ident> {
        let tok = self.cur.clone();
        let TokenKind::Ident(name) = tok.kind else {
            return self.unexpected();
        };
        self.check_binding_ident(&name, tok.span)?;
        self.bump()?;
        let id = Ident {
            name,
            span: tok.span,
        };
        self.declare_lexical(&id)?;
        Ok(id)
    }

    fn parse_import(&mut self, start: Span) -> PResult<Stmt> {
        self.require_module_top("import", start)?;
        self.bump()?;
        let mut specifiers = Vec::new();
        if let TokenKind::String(_) = self.cur.kind {
            let source = self.parse_string_lit()?;
            let attributes = self.parse_import_attributes()?;
            self.semicolon()?;
            return Ok(Stmt {
                kind: StmtKind::Import(ImportDecl {
                    specifiers,
                    source,
                    attributes,
                }),
                span: self.finish(start),
            });
        }
        let mut need_more = true;
        if matches!(self.cur.kind, TokenKind::Ident(_)) && !self.at_word("from")
            || (self.at_word("from") && self.peek()?.is_word("from"))
        {
            let id = self.import_binding()?;
            specifiers.push(ImportSpec::Default(id));
            need_more = self.eat(Punct::Comma)?;
        }
        if need_more {
            if self.eat(Punct::Star)? {
                self.expect_word("as")?;
                let id = self.import_binding()?;
                specifiers.push(ImportSpec::Namespace(id));
            } else if self.eat(Punct::LBrace)? {
                while !self.at(Punct::RBrace) {
                    let imported = self.parse_module_name()?;
                    let local = if self.at_word("as") {
                        self.bump()?;
                        self.import_binding()?
                    } else {
                        match &imported {
                            ModuleName::Ident(id) => {
                                let id = id.clone();
                                self.check_binding_ident(&id.name, id.span)?;
                                self.declare_lexical(&id)?;
                                id
                            }
                            ModuleName::Str(s) => {
                                return self.err("a string import name needs `as`", s.span);
                            }
                        }
                    };
                    specifiers.push(ImportSpec::Named { imported, local });
                    if !self.at(Punct::RBrace) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.bump()?;
            } else {
                return self.unexpected();
            }
        }
        let (source, attributes) = self.parse_from_clause()?;
        self.semicolon()?;
        Ok(Stmt {
            kind: StmtKind::Import(ImportDecl {
                specifiers,
                source,
                attributes,
            }),
            span: self.finish(start),
        })
    }

    fn add_export(&mut self, name: String, span: Span) -> PResult<()> {
        if !self.exported.insert(name.clone()) {
            return self.err(format!("`{name}` is exported more than once"), span);
        }
        Ok(())
    }

    fn parse_export(&mut self, start: Span) -> PResult<Stmt> {
        self.require_module_top("export", start)?;
        self.bump()?;
        let decl = if self.eat(Punct::Star)? {
            let exported = if self.at_word("as") {
                self.bump()?;
                let n = self.parse_module_name()?;
                self.add_export(n.text(), n.span())?;
                Some(n)
            } else {
                None
            };
            let (source, attributes) = self.parse_from_clause()?;
            self.semicolon()?;
            ExportDecl::All {
                exported,
                source,
                attributes,
            }
        } else if self.eat(Punct::LBrace)? {
            let mut specs = Vec::new();
            while !self.at(Punct::RBrace) {
                let local = self.parse_module_name()?;
                let exported = if self.at_word("as") {
                    self.bump()?;
                    self.parse_module_name()?
                } else {
                    local.clone()
                };
                self.add_export(exported.text(), exported.span())?;
                specs.push(ExportSpec { local, exported });
                if !self.at(Punct::RBrace) {
                    self.expect(Punct::Comma)?;
                }
            }
            self.bump()?;
            let (source, attributes) = if self.at_word("from") {
                let (s, a) = self.parse_from_clause()?;
                (Some(s), a)
            } else {
                for spec in &specs {
                    match &spec.local {
                        ModuleName::Str(s) => {
                            return self.err("a string can only be exported with `from`", s.span);
                        }
                        ModuleName::Ident(id) => {
                            if is_always_reserved(&id.name)
                                || is_strict_reserved(&id.name)
                                || id.name == "await"
                            {
                                return self
                                    .err(format!("`{}` is a reserved word", id.name), id.span);
                            }
                            self.export_locals.push((id.name.clone(), id.span));
                        }
                    }
                }
                (None, Vec::new())
            };
            self.semicolon()?;
            ExportDecl::Named {
                specifiers: specs,
                source,
                attributes,
            }
        } else if self.at_word("default") {
            let dspan = self.cur.span;
            self.bump()?;
            self.add_export("default".into(), dspan)?;
            let fstart = self.cur.span;
            if self.at_word("function") {
                let f = self.parse_function(fstart, false, true, true)?;
                ExportDecl::Default(ExportDefault::Function(Box::new(f)))
            } else if self.at_word("async") && {
                let p = self.peek()?;
                p.is_word("function") && !p.newline_before
            } {
                self.bump()?;
                let f = self.parse_function(fstart, true, true, true)?;
                ExportDecl::Default(ExportDefault::Function(Box::new(f)))
            } else if self.at_word("class") {
                let c = self.parse_class_with(true, true)?;
                if let Some(id) = &c.id {
                    self.declare_lexical(id)?;
                }
                ExportDecl::Default(ExportDefault::Class(Box::new(c)))
            } else {
                let e = self.parse_assign_in()?;
                self.semicolon()?;
                ExportDecl::Default(ExportDefault::Expr(Box::new(e)))
            }
        } else {
            let dstart = self.cur.span;
            let stmt = if self.at_word("var") {
                let d = self.parse_var_decl(VarKind::Var, false)?;
                self.semicolon()?;
                Stmt {
                    kind: StmtKind::Var(d),
                    span: self.finish(dstart),
                }
            } else if self.at_word("let")
                || self.at_word("const")
                || self.at_word("function")
                || self.at_word("class")
                || self.at_word("async")
            {
                self.statement_list_item_inner()?
            } else {
                return self.unexpected();
            };
            let names: Vec<Ident> = match &stmt.kind {
                StmtKind::Var(d) => d
                    .decls
                    .iter()
                    .flat_map(|v| v.target.bound_names())
                    .collect(),
                StmtKind::Function(f) => f.id.iter().cloned().collect(),
                StmtKind::Class(c) => c.id.iter().cloned().collect(),
                _ => return self.err("expected a declaration after `export`", dstart),
            };
            for n in names {
                self.add_export(n.name, n.span)?;
            }
            ExportDecl::Decl(Box::new(stmt))
        };
        Ok(Stmt {
            kind: StmtKind::Export(decl),
            span: self.finish(start),
        })
    }
}
