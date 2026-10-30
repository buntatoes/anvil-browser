// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Parser for Core JS.
//!
//! Recursive descent for statements, precedence climbing for binary
//! operators. All code is strict. Syntax errors stop the parse. Constructs
//! that are valid JavaScript but outside Core JS are recorded as subset
//! errors in [`Parser::subset`], replaced by placeholders, and parsing goes
//! on so every one of them is reported.

use std::rc::Rc;

use super::ast::*;
use super::lexer::{Diag, DiagKind, LexState, Lexer, Tok, Token};
use super::num::number_to_string;

/// Default cap on nesting depth, the same as the Rust engine's parser.
pub const DEFAULT_MAX_DEPTH: u32 = 1000;

type PResult<T> = Result<T, Diag>;

/// Declarators of one `var`, `let` or `const`: each name and its value.
type Decls = Vec<(Name, Option<Expr>)>;

const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
];

#[derive(Clone, Default)]
struct Ctx {
    in_func: bool,
    this_ok: bool,
    super_member: bool,
    super_call: bool,
    loops: u32,
    breakable: u32,
    labels: Vec<(Rc<str>, bool)>,
    /// Inside a generator body, where `yield` is an operator.
    generator: bool,
}

/// The parser. Use [`parse`].
pub struct Parser<'a> {
    lx: Lexer<'a>,
    cur: Token,
    prev_end: usize,
    names: u32,
    scopes: u32,
    /// Subset errors found so far.
    pub subset: Vec<Diag>,
    depth: u32,
    max_depth: u32,
    ctx: Ctx,
    is_module: bool,
    no_in: bool,
    /// Set by a `*` before a method name, taken by the method.
    next_gen: bool,
}

/// Result of a parse: the tree, or the first syntax error; plus every
/// subset error.
pub struct ParseOutput {
    /// The tree, or the syntax error that stopped the parse.
    pub program: Result<Program, Diag>,
    /// Subset errors, in source order.
    pub subset: Vec<Diag>,
}

/// Parse `src` as a script (`is_module` false) or a module.
pub fn parse(src: &str, is_module: bool, max_depth: u32) -> ParseOutput {
    let mut p = match Parser::new(src, is_module, max_depth) {
        Ok(p) => p,
        Err(e) => {
            return ParseOutput {
                program: Err(e),
                subset: Vec::new(),
            };
        }
    };
    let program = p.program();
    let mut subset = std::mem::take(&mut p.subset);
    subset.sort_by_key(|d| d.start);
    ParseOutput { program, subset }
}

fn tok_pos(t: &Token) -> Pos {
    Pos {
        start: t.start as u32,
        end: t.end as u32,
        line: t.line,
        col: t.col,
    }
}

fn units(s: &str) -> JStr {
    s.encode_utf16().collect::<Vec<u16>>().into()
}

fn is_reserved(w: &str) -> bool {
    RESERVED.contains(&w)
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, is_module: bool, max_depth: u32) -> PResult<Self> {
        let mut lx = Lexer::new(src);
        let cur = lx.next_token()?;
        Ok(Parser {
            lx,
            cur,
            prev_end: 0,
            names: 0,
            scopes: 1,
            subset: Vec::new(),
            depth: 0,
            max_depth,
            ctx: Ctx::default(),
            is_module,
            no_in: false,
            next_gen: false,
        })
    }

    // ---- token helpers ----

    fn next(&mut self) -> PResult<Token> {
        let t = self.lx.next_token()?;
        let old = std::mem::replace(&mut self.cur, t);
        self.prev_end = old.end;
        Ok(old)
    }

    fn is_p(&self, p: &str) -> bool {
        matches!(&self.cur.tok, Tok::Punct(q) if *q == p)
    }

    fn eat_p(&mut self, p: &str) -> PResult<bool> {
        if self.is_p(p) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_p(&mut self, p: &str) -> PResult<Token> {
        if self.is_p(p) {
            self.next()
        } else {
            Err(self.unexpected(&format!("expected `{p}`")))
        }
    }

    fn is_kw(&self, w: &str) -> bool {
        !self.cur.escaped && matches!(&self.cur.tok, Tok::Ident(n) if &**n == w)
    }

    fn eat_kw(&mut self, w: &str) -> PResult<bool> {
        if self.is_kw(w) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn describe(t: &Token) -> String {
        match &t.tok {
            Tok::Ident(n) => format!("`{n}`"),
            Tok::PrivateName(n) => format!("`#{n}`"),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Num(_) | Tok::BigInt => "a number".to_string(),
            Tok::Str(_) => "a string".to_string(),
            Tok::Template(..) => "a template".to_string(),
            Tok::Regex => "a regular expression".to_string(),
            Tok::Eof => "end of input".to_string(),
        }
    }

    fn unexpected(&self, want: &str) -> Diag {
        self.err_tok(format!("{want}, found {}", Self::describe(&self.cur)))
    }

    fn err_tok(&self, msg: impl Into<String>) -> Diag {
        Diag {
            kind: DiagKind::Syntax,
            message: msg.into(),
            start: self.cur.start,
            line: self.cur.line,
            col: self.cur.col,
        }
    }

    fn err_at(&self, msg: impl Into<String>, p: Pos) -> Diag {
        Diag {
            kind: DiagKind::Syntax,
            message: msg.into(),
            start: p.start as usize,
            line: p.line,
            col: p.col,
        }
    }

    fn subset_at(&mut self, msg: impl Into<String>, p: Pos) {
        self.subset.push(Diag {
            kind: DiagKind::Subset,
            message: msg.into(),
            start: p.start as usize,
            line: p.line,
            col: p.col,
        });
    }

    fn subset_here(&mut self, msg: impl Into<String>) {
        let p = tok_pos(&self.cur);
        self.subset_at(msg, p);
    }

    fn here(&self) -> Pos {
        tok_pos(&self.cur)
    }

    fn finish(&self, start: Pos) -> Pos {
        Pos {
            end: self.prev_end.max(start.start as usize) as u32,
            ..start
        }
    }

    fn save(&self) -> (Token, LexState, usize) {
        (self.cur.clone(), self.lx.save(), self.prev_end)
    }

    fn restore(&mut self, s: (Token, LexState, usize)) {
        self.cur = s.0;
        self.lx.restore(s.1);
        self.prev_end = s.2;
    }

    /// The token after the current one.
    fn peek(&mut self) -> PResult<Token> {
        let s = self.lx.save();
        let t = self.lx.next_token();
        self.lx.restore(s);
        t
    }

    fn semi(&mut self) -> PResult<()> {
        if self.eat_p(";")? {
            return Ok(());
        }
        if self.is_p("}") || self.cur.tok == Tok::Eof || self.cur.nl_before {
            return Ok(());
        }
        Err(self.unexpected("expected `;`"))
    }

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(self.err_tok(format!("nesting is too deep (limit {})", self.max_depth)));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn new_name(&mut self, name: Rc<str>, pos: Pos) -> Name {
        let id = self.names;
        self.names += 1;
        Name { name, id, pos }
    }

    fn new_scope(&mut self) -> u32 {
        let s = self.scopes;
        self.scopes += 1;
        s
    }

    fn specials(&mut self, pos: Pos) -> [Name; 3] {
        [
            self.new_name("this".into(), pos),
            self.new_name("%fn".into(), pos),
            self.new_name("%nt".into(), pos),
        ]
    }

    fn super_refs(&mut self, pos: Pos) -> Box<SuperRefs> {
        let [this, func, nt] = self.specials(pos);
        Box::new(SuperRefs { this, func, nt })
    }

    fn invalid(&self, start: Pos) -> Expr {
        Expr {
            kind: ExprKind::Invalid,
            pos: self.finish(start),
        }
    }

    /// Skip a balanced bracket group starting at the current token.
    fn skip_group(&mut self) -> PResult<()> {
        let mut stack: Vec<char> = Vec::new();
        loop {
            let open = match &self.cur.tok {
                Tok::Punct("(") => Some('('),
                Tok::Punct("[") => Some('['),
                Tok::Punct("{") => Some('{'),
                Tok::Template(_, false) => Some('`'),
                _ => None,
            };
            if let Some(c) = open {
                stack.push(c);
                self.next()?;
                continue;
            }
            if self.is_p("}") && stack.last() == Some(&'`') {
                let close = self.cur.clone();
                self.cur = self.lx.rescan_template(&close)?;
                if matches!(self.cur.tok, Tok::Template(_, true)) {
                    stack.pop();
                }
                self.next()?;
                if stack.is_empty() {
                    return Ok(());
                }
                continue;
            }
            if self.is_p(")") || self.is_p("]") || self.is_p("}") {
                stack.pop();
                self.next()?;
                if stack.is_empty() {
                    return Ok(());
                }
                continue;
            }
            if self.cur.tok == Tok::Eof {
                return Err(self.unexpected("unbalanced brackets"));
            }
            self.next()?;
            if stack.is_empty() {
                return Ok(());
            }
        }
    }
}

// ---- program, modules, declarations ----
impl Parser<'_> {
    fn program(&mut self) -> PResult<Program> {
        let pos = self.here();
        let this = self.new_name("this".into(), pos);
        let mut body = Vec::new();
        while self.cur.tok != Tok::Eof {
            body.push(self.stmt_list_item(true)?);
        }
        Ok(Program {
            body,
            scope: 0,
            this,
            n_names: self.names,
            n_scopes: self.scopes,
            is_module: self.is_module,
        })
    }

    fn stmt_list_item(&mut self, top: bool) -> PResult<Stmt> {
        let start = self.here();
        if self.is_kw("import") {
            let next = self.peek()?;
            if !matches!(next.tok, Tok::Punct("(" | ".")) {
                return self.import_decl(top);
            }
        }
        if self.is_kw("export") {
            return self.export_decl(top);
        }
        if self.is_kw("function") {
            let f = self.function(true)?;
            return Ok(Stmt {
                kind: StmtKind::Func(Rc::new(f)),
                pos: self.finish(start),
            });
        }
        if self.is_kw("async") {
            let next = self.peek()?;
            if !next.nl_before && matches!(&next.tok, Tok::Ident(n) if &**n == "function") {
                self.subset_here("`async` functions are outside Core JS");
                self.next()?;
                self.function(true)?;
                return Ok(Stmt {
                    kind: StmtKind::Invalid,
                    pos: self.finish(start),
                });
            }
        }
        if self.is_kw("class") {
            let c = self.class(true)?;
            return Ok(Stmt {
                kind: StmtKind::Class(Rc::new(c)),
                pos: self.finish(start),
            });
        }
        if self.is_kw("let") || self.is_kw("const") {
            let kind = if self.is_kw("let") {
                VarKind::Let
            } else {
                VarKind::Const
            };
            self.next()?;
            let decls = self.declarators(kind)?;
            self.semi()?;
            return Ok(Stmt {
                kind: decls.map_or(StmtKind::Invalid, |d| StmtKind::Var(kind, d)),
                pos: self.finish(start),
            });
        }
        self.stmt()
    }

    fn import_decl(&mut self, top: bool) -> PResult<Stmt> {
        let start = self.here();
        if !self.is_module || !top {
            return Err(self.err_tok("`import` is only allowed at the top level of a module"));
        }
        self.next()?;
        let mut names = Vec::new();
        let mut ok = true;
        if self.is_p("{") {
            self.next()?;
            while !self.is_p("}") {
                let t = self.cur.clone();
                let Tok::Ident(n) = &t.tok else {
                    return Err(self.unexpected("expected an imported name"));
                };
                let n = n.clone();
                self.next()?;
                if self.is_kw("as") {
                    self.subset_here(
                        "`as` in imports is not part of the module form; import the name as exported",
                    );
                    self.next()?;
                    self.next()?;
                    ok = false;
                }
                if is_reserved(&n) {
                    return Err(
                        self.err_at(format!("`{n}` can't be imported as a binding"), tok_pos(&t))
                    );
                }
                let local = self.new_name(n.clone(), tok_pos(&t));
                names.push(ImportName { imported: n, local });
                if !self.eat_p(",")? {
                    break;
                }
            }
            self.expect_p("}")?;
        } else {
            self.subset_here("only `import { a, b } from \"./x.js\"` is part of the module form");
            ok = false;
            while !self.is_kw("from") && !matches!(self.cur.tok, Tok::Str(_) | Tok::Eof) {
                self.next()?;
            }
        }
        if !self.eat_kw("from")? && ok {
            return Err(self.unexpected("expected `from`"));
        }
        let Tok::Str(s) = self.cur.tok.clone() else {
            return Err(self.unexpected("expected a module path string"));
        };
        self.next()?;
        self.semi()?;
        Ok(Stmt {
            kind: if ok {
                StmtKind::Import(names, String::from_utf16_lossy(&s))
            } else {
                StmtKind::Invalid
            },
            pos: self.finish(start),
        })
    }

    fn export_decl(&mut self, top: bool) -> PResult<Stmt> {
        let start = self.here();
        if !self.is_module || !top {
            return Err(self.err_tok("`export` is only allowed at the top level of a module"));
        }
        self.next()?;
        if self.is_kw("function") || self.is_kw("class") || self.is_kw("let") || self.is_kw("const")
        {
            let inner = self.stmt_list_item(false)?;
            return Ok(Stmt {
                kind: StmtKind::Export(Box::new(inner)),
                pos: self.finish(start),
            });
        }
        self.subset_here(
            "only `export function`, `export class`, `export const` and `export let` are part of the module form",
        );
        if self.is_kw("var") {
            self.stmt()?;
        } else {
            while !self.is_p(";") && self.cur.tok != Tok::Eof && !self.cur.nl_before {
                if self.is_p("{") {
                    self.skip_group()?;
                } else {
                    self.next()?;
                }
            }
            self.eat_p(";")?;
        }
        Ok(Stmt {
            kind: StmtKind::Invalid,
            pos: self.finish(start),
        })
    }

    fn binding_name(&mut self) -> PResult<Option<Name>> {
        let t = self.cur.clone();
        match &t.tok {
            Tok::Ident(n) => {
                if is_reserved(n) && !t.escaped {
                    return Err(self.err_tok(format!("`{n}` can't be used as a name")));
                }
                if &**n == "eval" || &**n == "arguments" {
                    return Err(self.err_tok(format!("`{n}` can't be declared in strict code")));
                }
                if &**n == "await" {
                    self.subset_here("`await` is outside Core JS");
                }
                if t.escaped {
                    self.subset_here("names written with escapes are outside Core JS");
                }
                self.next()?;
                Ok(Some(self.new_name(n.clone(), tok_pos(&t))))
            }
            Tok::Punct("[" | "{") => {
                self.subset_here("destructuring is outside Core JS");
                self.skip_group()?;
                Ok(None)
            }
            _ => Err(self.unexpected("expected a name")),
        }
    }

    /// Declarators after `let`/`const`/`var`. `None` when a subset error
    /// made the declaration unusable.
    fn declarators(&mut self, kind: VarKind) -> PResult<Option<Decls>> {
        let mut out = Vec::new();
        let mut ok = true;
        loop {
            let at = self.here();
            let name = self.binding_name()?;
            let init = if self.eat_p("=")? {
                Some(self.assign()?)
            } else {
                None
            };
            if kind == VarKind::Const && init.is_none() && name.is_some() {
                return Err(self.err_at("a `const` declaration needs a value", at));
            }
            match name {
                Some(n) => out.push((n, init)),
                None => ok = false,
            }
            if !self.eat_p(",")? {
                break;
            }
        }
        Ok(if ok { Some(out) } else { None })
    }

    fn params(&mut self) -> PResult<Vec<Name>> {
        self.expect_p("(")?;
        let mut out: Vec<Name> = Vec::new();
        while !self.is_p(")") {
            if self.is_p("...") {
                self.subset_here("rest parameters are outside Core JS");
                self.next()?;
            }
            let n = self.binding_name()?;
            if self.is_p("=") {
                self.subset_here("default parameters are outside Core JS");
                self.next()?;
                self.assign()?;
            }
            if let Some(n) = n {
                if out.iter().any(|o| o.name == n.name) {
                    return Err(self.err_at(format!("duplicate parameter `{}`", n.name), n.pos));
                }
                out.push(n);
            }
            if !self.eat_p(",")? {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(out)
    }

    fn func_body(&mut self, ctx: Ctx) -> PResult<Vec<Stmt>> {
        let saved = std::mem::replace(&mut self.ctx, ctx);
        let saved_in = std::mem::replace(&mut self.no_in, false);
        self.expect_p("{")?;
        let mut body = Vec::new();
        while !self.is_p("}") {
            if self.cur.tok == Tok::Eof {
                return Err(self.unexpected("expected `}`"));
            }
            body.push(self.stmt_list_item(false)?);
        }
        self.next()?;
        self.ctx = saved;
        self.no_in = saved_in;
        Ok(body)
    }

    /// `function name(params) { body }`; the current token is `function`.
    fn function(&mut self, decl: bool) -> PResult<Func> {
        let start = self.here();
        self.enter()?;
        self.next()?;
        let generator = self.is_p("*");
        if generator {
            self.subset_here("generators are outside Core JS");
            self.next()?;
        }
        let name = if matches!(self.cur.tok, Tok::Ident(_)) {
            self.binding_name()?
        } else if decl {
            return Err(self.unexpected("expected a function name"));
        } else {
            None
        };
        let name_scope = if !decl && name.is_some() {
            Some(self.new_scope())
        } else {
            None
        };
        let scope = self.new_scope();
        let specials = self.specials(start);
        let params = self.params()?;
        let ctx = Ctx {
            in_func: true,
            generator,
            ..Ctx::default()
        };
        let body = self.func_body(ctx)?;
        self.leave();
        Ok(Func {
            name,
            params,
            body,
            kind: FuncKind::Normal,
            scope,
            name_scope,
            specials: Some(specials),
            pos: self.finish(start),
        })
    }

    fn method(&mut self, kind: FuncKind, key_pos: Pos) -> PResult<Func> {
        let generator = std::mem::take(&mut self.next_gen);
        self.enter()?;
        let scope = self.new_scope();
        let specials = self.specials(key_pos);
        let params = self.params()?;
        let ctx = Ctx {
            in_func: true,
            this_ok: true,
            super_member: true,
            super_call: matches!(kind, FuncKind::Ctor { derived: true }),
            generator,
            ..Ctx::default()
        };
        let body = self.func_body(ctx)?;
        self.leave();
        Ok(Func {
            name: None,
            params,
            body,
            kind,
            scope,
            name_scope: None,
            specials: Some(specials),
            pos: self.finish(key_pos),
        })
    }

    fn arrow(&mut self, params: Vec<Name>, start: Pos) -> PResult<Expr> {
        self.enter()?;
        let scope = self.new_scope();
        self.expect_p("=>")?;
        let ctx = Ctx {
            in_func: true,
            this_ok: self.ctx.this_ok,
            super_member: self.ctx.super_member,
            super_call: self.ctx.super_call,
            ..Ctx::default()
        };
        let body = if self.is_p("{") {
            self.func_body(ctx)?
        } else {
            let saved = std::mem::replace(&mut self.ctx, ctx);
            let e = self.assign()?;
            self.ctx = saved;
            let pos = e.pos;
            vec![Stmt {
                kind: StmtKind::Return(Some(e)),
                pos,
            }]
        };
        self.leave();
        let pos = self.finish(start);
        Ok(Expr {
            kind: ExprKind::Function(Rc::new(Func {
                name: None,
                params,
                body,
                kind: FuncKind::Arrow,
                scope,
                name_scope: None,
                specials: None,
                pos,
            })),
            pos,
        })
    }

    fn prop_name(&mut self) -> PResult<Option<PropName>> {
        let t = self.cur.clone();
        let pos = tok_pos(&t);
        let text: Rc<str> = match &t.tok {
            Tok::Ident(n) => n.clone(),
            Tok::Str(s) => String::from_utf16_lossy(s).into(),
            Tok::Num(v) => number_to_string(*v).into(),
            Tok::Punct("[") => {
                self.subset_here("computed property names are outside Core JS");
                self.skip_group()?;
                return Ok(None);
            }
            Tok::PrivateName(_) => {
                self.subset_here("private names are outside Core JS");
                self.next()?;
                return Ok(None);
            }
            Tok::BigInt => {
                self.subset_here("BigInt literals are outside Core JS");
                self.next()?;
                return Ok(None);
            }
            _ => return Err(self.unexpected("expected a property name")),
        };
        self.next()?;
        let u = match &t.tok {
            Tok::Str(s) => s.clone(),
            _ => units(&text),
        };
        Ok(Some(PropName {
            text,
            units: u,
            pos,
        }))
    }
}

// ---- classes and statements ----
impl Parser<'_> {
    /// `class Name extends X { ... }`; the current token is `class`.
    fn class(&mut self, decl: bool) -> PResult<Class> {
        let start = self.here();
        self.enter()?;
        self.next()?;
        let name = if matches!(self.cur.tok, Tok::Ident(_)) && !self.is_kw("extends") {
            self.binding_name()?
        } else if decl {
            return Err(self.unexpected("expected a class name"));
        } else {
            None
        };
        let scope = self.new_scope();
        let extends = if self.eat_kw("extends")? {
            Some(Box::new(self.lhs(true)?))
        } else {
            None
        };
        let derived = extends.is_some();
        self.expect_p("{")?;
        let mut ctor: Option<Rc<Func>> = None;
        let mut methods = Vec::new();
        while !self.is_p("}") {
            if self.cur.tok == Tok::Eof {
                return Err(self.unexpected("expected `}`"));
            }
            if self.eat_p(";")? {
                continue;
            }
            let member_start = self.here();
            let mut is_static = false;
            if self.is_kw("static") {
                let n = self.peek()?;
                if !matches!(n.tok, Tok::Punct("(" | "=" | ";" | "}")) {
                    self.next()?;
                    is_static = true;
                    if self.is_p("{") {
                        self.subset_here("static blocks are outside Core JS");
                        self.skip_group()?;
                        continue;
                    }
                }
            }
            let mut ok = true;
            if self.is_kw("get") || self.is_kw("set") || self.is_kw("async") {
                let n = self.peek()?;
                if !matches!(n.tok, Tok::Punct("(" | "=" | ";" | "}")) && !n.nl_before {
                    let what = if self.is_kw("async") {
                        "`async` methods are outside Core JS"
                    } else {
                        "getters and setters are outside Core JS"
                    };
                    self.subset_here(what);
                    self.next()?;
                    ok = false;
                }
            }
            if self.is_p("*") {
                self.subset_here("generators are outside Core JS");
                self.next_gen = true;
                self.next()?;
                ok = false;
            }
            let key = self.prop_name()?;
            if !self.is_p("(") {
                self.subset_at(
                    "class fields are outside Core JS; set fields in the constructor",
                    member_start,
                );
                if self.eat_p("=")? {
                    self.assign()?;
                }
                self.semi()?;
                continue;
            }
            let key_pos = key.as_ref().map_or(member_start, |k| k.pos);
            let is_ctor = !is_static && key.as_ref().is_some_and(|k| &*k.text == "constructor");
            let kind = if is_ctor {
                FuncKind::Ctor { derived }
            } else {
                FuncKind::Method
            };
            let f = self.method(kind, key_pos)?;
            if !ok {
                continue;
            }
            let Some(key) = key else { continue };
            if is_ctor {
                if ctor.is_some() {
                    return Err(self.err_at("a class can only have one constructor", key_pos));
                }
                ctor = Some(Rc::new(f));
            } else {
                methods.push(Method {
                    key,
                    is_static,
                    func: Rc::new(f),
                });
            }
        }
        self.next()?;
        self.leave();
        Ok(Class {
            name,
            scope,
            extends,
            ctor,
            methods,
            pos: self.finish(start),
        })
    }

    fn block(&mut self) -> PResult<Block> {
        self.expect_p("{")?;
        let scope = self.new_scope();
        let mut stmts = Vec::new();
        while !self.is_p("}") {
            if self.cur.tok == Tok::Eof {
                return Err(self.unexpected("expected `}`"));
            }
            stmts.push(self.stmt_list_item(false)?);
        }
        self.next()?;
        Ok(Block { stmts, scope })
    }

    fn sub_stmt(&mut self) -> PResult<Stmt> {
        if self.is_kw("function") || self.is_kw("class") || self.is_kw("let") || self.is_kw("const")
        {
            return Err(
                self.err_tok("a declaration can't be the body of a statement here; use a block")
            );
        }
        self.stmt()
    }

    fn loop_body(&mut self) -> PResult<Stmt> {
        self.ctx.loops += 1;
        self.ctx.breakable += 1;
        let r = self.sub_stmt();
        self.ctx.loops -= 1;
        self.ctx.breakable -= 1;
        r
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        self.enter()?;
        let r = self.stmt_inner();
        self.leave();
        r
    }

    fn paren_expr(&mut self) -> PResult<Expr> {
        self.expect_p("(")?;
        let e = self.expr()?;
        self.expect_p(")")?;
        Ok(e)
    }

    fn jump(&mut self, is_break: bool) -> PResult<StmtKind> {
        let at = self.here();
        self.next()?;
        let label = match &self.cur.tok {
            Tok::Ident(l) if !self.cur.nl_before && !is_reserved(l) => {
                let l = l.clone();
                self.next()?;
                Some(l)
            }
            _ => None,
        };
        match &label {
            Some(l) => match self.ctx.labels.iter().rev().find(|(n, _)| n == l) {
                None => return Err(self.err_at(format!("no label `{l}` here"), at)),
                Some((_, is_loop)) if !is_break && !is_loop => {
                    return Err(self.err_at(format!("`continue {l}` must name a loop"), at));
                }
                _ => {}
            },
            None => {
                if is_break && self.ctx.breakable == 0 {
                    return Err(self.err_at("`break` outside a loop or `switch`", at));
                }
                if !is_break && self.ctx.loops == 0 {
                    return Err(self.err_at("`continue` outside a loop", at));
                }
            }
        }
        self.semi()?;
        Ok(if is_break {
            StmtKind::Break(label)
        } else {
            StmtKind::Continue(label)
        })
    }

    fn stmt_inner(&mut self) -> PResult<Stmt> {
        let start = self.here();
        let word = match &self.cur.tok {
            Tok::Ident(w) if !self.cur.escaped => Some(w.clone()),
            _ => None,
        };
        let tok = self.cur.tok.clone();
        let kind = match (word.as_deref(), &tok) {
            (_, Tok::Punct("{")) => StmtKind::Block(self.block()?),
            (_, Tok::Punct(";")) => {
                self.next()?;
                StmtKind::Empty
            }
            (Some("var"), _) => {
                self.next()?;
                let d = self.declarators(VarKind::Var)?;
                self.semi()?;
                d.map_or(StmtKind::Invalid, |d| StmtKind::Var(VarKind::Var, d))
            }
            (Some("if"), _) => {
                self.next()?;
                let test = self.paren_expr()?;
                let cons = self.sub_stmt()?;
                let alt = if self.eat_kw("else")? {
                    Some(Box::new(self.sub_stmt()?))
                } else {
                    None
                };
                StmtKind::If(test, Box::new(cons), alt)
            }
            (Some("while"), _) => {
                self.next()?;
                let test = self.paren_expr()?;
                let body = self.loop_body()?;
                StmtKind::While(test, Box::new(body))
            }
            (Some("do"), _) => {
                self.next()?;
                let body = self.loop_body()?;
                if !self.eat_kw("while")? {
                    return Err(self.unexpected("expected `while`"));
                }
                let test = self.paren_expr()?;
                // The `;` after `do ... while (...)` may always be left out.
                self.eat_p(";")?;
                StmtKind::DoWhile(Box::new(body), test)
            }
            (Some("for"), _) => self.for_stmt()?,
            (Some("return"), _) => {
                if !self.ctx.in_func {
                    return Err(self.err_tok("`return` outside a function"));
                }
                self.next()?;
                let arg = if self.is_p(";")
                    || self.is_p("}")
                    || self.cur.tok == Tok::Eof
                    || self.cur.nl_before
                {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.semi()?;
                StmtKind::Return(arg)
            }
            (Some("break"), _) => self.jump(true)?,
            (Some("continue"), _) => self.jump(false)?,
            (Some("throw"), _) => {
                self.next()?;
                if self.cur.nl_before {
                    return Err(self.err_tok("no line break is allowed after `throw`"));
                }
                let e = self.expr()?;
                self.semi()?;
                StmtKind::Throw(e)
            }
            (Some("try"), _) => self.try_stmt()?,
            (Some("switch"), _) => self.switch_stmt()?,
            (Some("with"), _) => {
                self.subset_here("`with` is outside Core JS (and not allowed in strict code)");
                self.next()?;
                self.skip_group()?;
                self.sub_stmt()?;
                StmtKind::Invalid
            }
            (Some("debugger"), _) => {
                self.subset_here("`debugger` is outside Core JS");
                self.next()?;
                self.semi()?;
                StmtKind::Invalid
            }
            (Some("function" | "class" | "let" | "const"), _) => {
                return Err(self.err_tok("a declaration is not allowed here"));
            }
            (Some(w), _) if !is_reserved(w) && self.peek()?.tok == Tok::Punct(":") => {
                return self.labeled();
            }
            _ => {
                let e = self.expr()?;
                self.semi()?;
                StmtKind::Expr(e)
            }
        };
        Ok(Stmt {
            kind,
            pos: self.finish(start),
        })
    }

    fn labeled(&mut self) -> PResult<Stmt> {
        let start = self.here();
        let mut labels: Vec<Rc<str>> = Vec::new();
        while let Tok::Ident(l) = &self.cur.tok {
            if is_reserved(l) || self.cur.escaped {
                break;
            }
            let l = l.clone();
            if self.peek()?.tok != Tok::Punct(":") {
                break;
            }
            if &*l == "await" {
                self.subset_here("`await` is outside Core JS");
            }
            if self.ctx.labels.iter().any(|(m, _)| *m == l) || labels.contains(&l) {
                return Err(self.err_tok(format!("label `{l}` is already in use")));
            }
            self.next()?;
            self.next()?;
            labels.push(l);
        }
        if self.is_kw("function") {
            return Err(self.err_tok("a labeled function is not allowed in strict code"));
        }
        let is_loop = self.is_kw("for") || self.is_kw("while") || self.is_kw("do");
        for l in &labels {
            self.ctx.labels.push((l.clone(), is_loop));
        }
        let body = self.sub_stmt();
        for _ in &labels {
            self.ctx.labels.pop();
        }
        let mut s = body?;
        for l in labels.into_iter().rev() {
            s = Stmt {
                kind: StmtKind::Labeled(l, Box::new(s)),
                pos: self.finish(start),
            };
        }
        Ok(s)
    }
}

// ---- for, try, switch ----
impl Parser<'_> {
    fn for_stmt(&mut self) -> PResult<StmtKind> {
        let for_pos = self.here();
        self.next()?;
        if self.is_kw("await") {
            self.subset_here("`for await` is outside Core JS");
            self.next()?;
        }
        self.expect_p("(")?;
        let scope = self.new_scope();
        let mut init = None;
        if self.is_kw("let") || self.is_kw("const") || self.is_kw("var") {
            let kind = if self.is_kw("let") {
                VarKind::Let
            } else if self.is_kw("const") {
                VarKind::Const
            } else {
                VarKind::Var
            };
            self.next()?;
            let name = self.binding_name()?;
            if self.eat_kw("of")? {
                return self.for_of_rest(scope, Some(kind), name);
            }
            if self.is_kw("in") {
                return self.for_in_rest(for_pos);
            }
            let saved = std::mem::replace(&mut self.no_in, true);
            let mut decls = Vec::new();
            let mut ok = true;
            let mut name = name;
            let mut first = true;
            loop {
                if !first {
                    name = self.binding_name()?;
                }
                first = false;
                let i = if self.eat_p("=")? {
                    Some(self.assign()?)
                } else {
                    None
                };
                if kind == VarKind::Const && i.is_none() {
                    return Err(self.err_tok("a `const` declaration needs a value"));
                }
                match name.take() {
                    Some(n) => decls.push((n, i)),
                    None => ok = false,
                }
                if !self.eat_p(",")? {
                    break;
                }
            }
            self.no_in = saved;
            if self.is_kw("of") || self.is_kw("in") {
                return Err(
                    self.err_tok("a `for` loop head can declare only one name before `of` or `in`")
                );
            }
            if ok {
                init = Some(ForInit::Var(kind, decls));
            }
        } else if !self.is_p(";") {
            let saved = std::mem::replace(&mut self.no_in, true);
            let e = self.expr()?;
            self.no_in = saved;
            if self.eat_kw("of")? {
                return match e.kind {
                    ExprKind::Ident(n) => self.for_of_rest(scope, None, Some(n)),
                    _ => {
                        self.subset_at("`for...of` must assign to a plain name in Core JS", e.pos);
                        self.for_of_rest(scope, None, None)
                    }
                };
            }
            if self.is_kw("in") {
                return self.for_in_rest(for_pos);
            }
            init = Some(ForInit::Expr(e));
        }
        self.expect_p(";")?;
        let test = if self.is_p(";") {
            None
        } else {
            Some(self.expr()?)
        };
        self.expect_p(";")?;
        let update = if self.is_p(")") {
            None
        } else {
            Some(self.expr()?)
        };
        self.expect_p(")")?;
        let body = self.loop_body()?;
        Ok(StmtKind::For {
            scope,
            init,
            test,
            update,
            body: Box::new(body),
        })
    }

    fn for_of_rest(
        &mut self,
        scope: u32,
        decl: Option<VarKind>,
        target: Option<Name>,
    ) -> PResult<StmtKind> {
        let iter = self.assign()?;
        self.expect_p(")")?;
        let body = self.loop_body()?;
        Ok(match target {
            Some(target) => StmtKind::ForOf {
                scope,
                decl,
                target,
                iter,
                body: Box::new(body),
            },
            None => StmtKind::Invalid,
        })
    }

    fn for_in_rest(&mut self, for_pos: Pos) -> PResult<StmtKind> {
        self.subset_at("`for...in` is outside Core JS", for_pos);
        self.next()?;
        self.expr()?;
        self.expect_p(")")?;
        self.loop_body()?;
        Ok(StmtKind::Invalid)
    }

    fn try_stmt(&mut self) -> PResult<StmtKind> {
        self.next()?;
        let block = self.block()?;
        let mut handler = None;
        if self.eat_kw("catch")? {
            let scope = self.new_scope();
            let param = if self.eat_p("(")? {
                let p = self.binding_name()?;
                self.expect_p(")")?;
                p
            } else {
                None
            };
            let body = self.block()?;
            handler = Some(Catch { scope, param, body });
        }
        let finalizer = if self.eat_kw("finally")? {
            Some(self.block()?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return Err(self.unexpected("expected `catch` or `finally`"));
        }
        Ok(StmtKind::Try(block, handler, finalizer))
    }

    fn switch_stmt(&mut self) -> PResult<StmtKind> {
        self.next()?;
        let disc = self.paren_expr()?;
        self.expect_p("{")?;
        let scope = self.new_scope();
        let mut cases = Vec::new();
        let mut seen_default = false;
        self.ctx.breakable += 1;
        while !self.is_p("}") {
            let test = if self.eat_kw("case")? {
                Some(self.expr()?)
            } else if self.is_kw("default") {
                if seen_default {
                    return Err(self.err_tok("a `switch` can only have one `default`"));
                }
                seen_default = true;
                self.next()?;
                None
            } else {
                return Err(self.unexpected("expected `case` or `default`"));
            };
            self.expect_p(":")?;
            let mut body = Vec::new();
            while !self.is_kw("case") && !self.is_kw("default") && !self.is_p("}") {
                if self.cur.tok == Tok::Eof {
                    return Err(self.unexpected("expected `}`"));
                }
                body.push(self.stmt_list_item(false)?);
            }
            cases.push(Case { test, body });
        }
        self.ctx.breakable -= 1;
        self.next()?;
        Ok(StmtKind::Switch { disc, scope, cases })
    }
}

// ---- expressions ----
impl Parser<'_> {
    fn expr(&mut self) -> PResult<Expr> {
        let start = self.here();
        let first = self.assign()?;
        if !self.is_p(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_p(",")? {
            items.push(self.assign()?);
        }
        Ok(Expr {
            kind: ExprKind::Seq(items),
            pos: self.finish(start),
        })
    }

    fn assign(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.assign_inner();
        self.leave();
        r
    }

    fn async_arrow(&mut self, start: Pos) -> PResult<Option<Expr>> {
        let n = self.peek()?;
        if n.nl_before || !matches!(n.tok, Tok::Ident(_) | Tok::Punct("(")) {
            return Ok(None);
        }
        if matches!(&n.tok, Tok::Ident(f) if &**f == "function") {
            return Ok(None);
        }
        let s = self.save();
        self.next()?;
        let looks_arrow = if self.is_p("(") {
            self.skip_group()?;
            self.is_p("=>")
        } else {
            self.next()?;
            self.is_p("=>")
        };
        self.restore(s);
        if !looks_arrow {
            return Ok(None);
        }
        self.subset_here("`async` arrows are outside Core JS");
        self.next()?;
        let params = if self.is_p("(") {
            self.skip_group()?;
            Vec::new()
        } else {
            self.binding_name()?.into_iter().collect()
        };
        self.arrow(params, start)?;
        Ok(Some(self.invalid(start)))
    }

    fn assign_op(&mut self) -> Option<Option<BinOp>> {
        let Tok::Punct(p) = &self.cur.tok else {
            return None;
        };
        Some(match *p {
            "=" => None,
            "+=" => Some(BinOp::Add),
            "-=" => Some(BinOp::Sub),
            "*=" => Some(BinOp::Mul),
            "/=" => Some(BinOp::Div),
            "%=" => Some(BinOp::Mod),
            "**=" => Some(BinOp::Exp),
            "<<=" => Some(BinOp::Shl),
            ">>=" => Some(BinOp::Shr),
            ">>>=" => Some(BinOp::UShr),
            "&=" => Some(BinOp::BitAnd),
            "|=" => Some(BinOp::BitOr),
            "^=" => Some(BinOp::BitXor),
            "&&=" | "||=" | "??=" => {
                self.subset_here("logical assignment is outside Core JS");
                None
            }
            _ => return None,
        })
    }

    fn assign_inner(&mut self) -> PResult<Expr> {
        let start = self.here();
        if self.is_kw("yield") {
            if !self.ctx.generator {
                return Err(self.err_tok("`yield` is reserved in strict code"));
            }
            // A `yield` expression: generators are already reported, so
            // this only reads the operand and keeps going.
            self.subset_here("`yield` is outside Core JS (generators are left out)");
            self.next()?;
            if self.is_p("*") {
                self.next()?;
                return self.assign_inner();
            }
            let ends = self.cur.nl_before
                || self.cur.tok == Tok::Eof
                || matches!(self.cur.tok, Tok::Punct(")" | "]" | "}" | "," | ";" | ":"));
            if !ends {
                return self.assign_inner();
            }
            return Ok(Expr {
                kind: ExprKind::Null,
                pos: self.finish(start),
            });
        }
        if self.is_kw("async") {
            if let Some(e) = self.async_arrow(start)? {
                return Ok(e);
            }
        }
        let left = self.cond()?;
        let Some(op) = self.assign_op() else {
            return Ok(left);
        };
        match &left.kind {
            ExprKind::Ident(n) => {
                if &*n.name == "eval" || &*n.name == "arguments" {
                    return Err(self.err_at(
                        format!("can't assign to `{}` in strict code", n.name),
                        left.pos,
                    ));
                }
            }
            ExprKind::Member(..) | ExprKind::Index(..) | ExprKind::SuperMember(..) => {}
            ExprKind::Invalid => {}
            ExprKind::Array(_) | ExprKind::Object(_) if op.is_none() => {
                self.subset_at("destructuring assignment is outside Core JS", left.pos);
            }
            _ => return Err(self.err_at("invalid assignment target", left.pos)),
        }
        self.next()?;
        let value = self.assign()?;
        Ok(Expr {
            kind: ExprKind::Assign(op, Box::new(left), Box::new(value)),
            pos: self.finish(start),
        })
    }

    fn cond(&mut self) -> PResult<Expr> {
        let start = self.here();
        let test = self.binary(0)?;
        if !self.is_p("?") {
            return Ok(test);
        }
        self.next()?;
        let saved = std::mem::replace(&mut self.no_in, false);
        let cons = self.assign()?;
        self.no_in = saved;
        self.expect_p(":")?;
        let alt = self.assign()?;
        Ok(Expr {
            kind: ExprKind::Cond(Box::new(test), Box::new(cons), Box::new(alt)),
            pos: self.finish(start),
        })
    }

    fn binop(&self) -> Option<(u8, Option<BinOp>, &'static str)> {
        let r: (u8, Option<BinOp>, &'static str) = match &self.cur.tok {
            Tok::Punct(p) => match *p {
                "??" => (1, None, "??"),
                "||" => (2, None, "||"),
                "&&" => (3, None, "&&"),
                "|" => (4, Some(BinOp::BitOr), "|"),
                "^" => (5, Some(BinOp::BitXor), "^"),
                "&" => (6, Some(BinOp::BitAnd), "&"),
                "===" => (7, Some(BinOp::StrictEq), "==="),
                "!==" => (7, Some(BinOp::StrictNe), "!=="),
                "==" => (7, Some(BinOp::StrictEq), "=="),
                "!=" => (7, Some(BinOp::StrictNe), "!="),
                "<" => (8, Some(BinOp::Lt), "<"),
                ">" => (8, Some(BinOp::Gt), ">"),
                "<=" => (8, Some(BinOp::Le), "<="),
                ">=" => (8, Some(BinOp::Ge), ">="),
                "<<" => (9, Some(BinOp::Shl), "<<"),
                ">>" => (9, Some(BinOp::Shr), ">>"),
                ">>>" => (9, Some(BinOp::UShr), ">>>"),
                "+" => (10, Some(BinOp::Add), "+"),
                "-" => (10, Some(BinOp::Sub), "-"),
                "*" => (11, Some(BinOp::Mul), "*"),
                "/" => (11, Some(BinOp::Div), "/"),
                "%" => (11, Some(BinOp::Mod), "%"),
                _ => return None,
            },
            Tok::Ident(w) if !self.cur.escaped => match &**w {
                "instanceof" => (8, Some(BinOp::Instanceof), "instanceof"),
                "in" if !self.no_in => (8, Some(BinOp::In), "in"),
                _ => return None,
            },
            _ => return None,
        };
        Some(r)
    }

    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let start = self.here();
        let mut left = if let Tok::PrivateName(_) = self.cur.tok {
            self.subset_here("private names are outside Core JS");
            self.next()?;
            self.invalid(start)
        } else {
            self.unary()?
        };
        let mut links = 0;
        while let Some((prec, op, text)) = self.binop() {
            if prec <= min {
                break;
            }
            match text {
                "==" => self.subset_here("`==` is outside Core JS; use `===`"),
                "!=" => self.subset_here("`!=` is outside Core JS; use `!==`"),
                "??" => self.subset_here("`??` is outside Core JS"),
                _ => {}
            }
            self.next()?;
            self.enter()?;
            links += 1;
            let right = self.binary(prec)?;
            let pos = self.finish(start);
            left = match op {
                Some(op) => Expr {
                    kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                    pos,
                },
                None => Expr {
                    kind: ExprKind::Logical(text == "&&", Box::new(left), Box::new(right)),
                    pos,
                },
            };
        }
        for _ in 0..links {
            self.leave();
        }
        Ok(left)
    }
}

// ---- unary, member chains, primaries ----
impl Parser<'_> {
    fn check_update_target(&self, e: &Expr) -> PResult<()> {
        match &e.kind {
            ExprKind::Ident(n) if &*n.name == "eval" || &*n.name == "arguments" => Err(self
                .err_at(
                    format!("can't assign to `{}` in strict code", n.name),
                    e.pos,
                )),
            ExprKind::Ident(_)
            | ExprKind::Member(..)
            | ExprKind::Index(..)
            | ExprKind::SuperMember(..)
            | ExprKind::Invalid => Ok(()),
            _ => Err(self.err_at("invalid target for `++` or `--`", e.pos)),
        }
    }

    /// ExponentiationExpression: a unary expression, or an update
    /// expression followed by `**` and another exponentiation expression.
    fn unary(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.unary_inner(true);
        self.leave();
        r
    }

    /// UnaryExpression, without a trailing `**`.
    fn unary_operand(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.unary_inner(false);
        self.leave();
        r
    }

    fn unary_inner(&mut self, exp: bool) -> PResult<Expr> {
        let start = self.here();
        let word = match &self.cur.tok {
            Tok::Ident(w) if !self.cur.escaped => Some(w.clone()),
            _ => None,
        };
        let op = match (&self.cur.tok, word.as_deref()) {
            (Tok::Punct("-"), _) => Some(Some(UnOp::Neg)),
            (Tok::Punct("+"), _) => Some(Some(UnOp::Plus)),
            (Tok::Punct("!"), _) => Some(Some(UnOp::Not)),
            (Tok::Punct("~"), _) => Some(Some(UnOp::BitNot)),
            (_, Some("typeof")) => Some(Some(UnOp::Typeof)),
            (_, Some(w @ ("void" | "delete" | "await"))) => {
                self.subset_here(format!("`{w}` is outside Core JS"));
                Some(None)
            }
            _ => None,
        };
        if let Some(op) = op {
            self.next()?;
            let arg = self.unary_operand()?;
            if exp && self.is_p("**") {
                return Err(self.err_tok("put parentheses around a unary expression before `**`"));
            }
            let pos = self.finish(start);
            return Ok(match op {
                Some(op) => Expr {
                    kind: ExprKind::Unary(op, Box::new(arg)),
                    pos,
                },
                None => self.invalid(start),
            });
        }
        if self.is_p("++") || self.is_p("--") {
            let inc = self.is_p("++");
            self.next()?;
            let target = self.unary_operand()?;
            self.check_update_target(&target)?;
            let e = Expr {
                kind: ExprKind::Update {
                    inc,
                    prefix: true,
                    target: Box::new(target),
                },
                pos: self.finish(start),
            };
            return self.exp_tail(e, start, exp);
        }
        let mut e = self.lhs(true)?;
        if (self.is_p("++") || self.is_p("--")) && !self.cur.nl_before {
            let inc = self.is_p("++");
            self.check_update_target(&e)?;
            self.next()?;
            e = Expr {
                kind: ExprKind::Update {
                    inc,
                    prefix: false,
                    target: Box::new(e),
                },
                pos: self.finish(start),
            };
        }
        self.exp_tail(e, start, exp)
    }

    fn exp_tail(&mut self, e: Expr, start: Pos, exp: bool) -> PResult<Expr> {
        if exp && self.is_p("**") {
            self.next()?;
            let right = self.unary()?;
            return Ok(Expr {
                kind: ExprKind::Binary(BinOp::Exp, Box::new(e), Box::new(right)),
                pos: self.finish(start),
            });
        }
        Ok(e)
    }

    fn args(&mut self) -> PResult<Vec<Expr>> {
        self.expect_p("(")?;
        let saved = std::mem::replace(&mut self.no_in, false);
        let mut out = Vec::new();
        while !self.is_p(")") {
            if self.is_p("...") {
                self.subset_here("spread is outside Core JS");
                self.next()?;
            }
            out.push(self.assign()?);
            if !self.eat_p(",")? {
                break;
            }
        }
        self.expect_p(")")?;
        self.no_in = saved;
        Ok(out)
    }

    fn member_name(&mut self) -> PResult<Option<PropName>> {
        let t = self.cur.clone();
        match &t.tok {
            Tok::Ident(n) => {
                self.next()?;
                Ok(Some(PropName {
                    text: n.clone(),
                    units: units(n),
                    pos: tok_pos(&t),
                }))
            }
            Tok::PrivateName(_) => {
                self.subset_here("private names are outside Core JS");
                self.next()?;
                Ok(None)
            }
            _ => Err(self.unexpected("expected a property name")),
        }
    }

    /// Member, call and `new` chains. With `calls` false the chain stops
    /// before a call (the callee of `new`).
    fn lhs(&mut self, calls: bool) -> PResult<Expr> {
        let start = self.here();
        let mut e = if self.is_kw("new") {
            self.new_expr()?
        } else {
            self.primary()?
        };
        let mut links = 0;
        loop {
            if self.is_p(".") || self.is_p("?.") {
                let optional = self.is_p("?.");
                if optional {
                    self.subset_here("optional chaining is outside Core JS");
                }
                self.next()?;
                if optional && (self.is_p("(") || self.is_p("[")) {
                    continue;
                }
                let name = self.member_name()?;
                e = match name {
                    Some(n) => Expr {
                        kind: ExprKind::Member(Box::new(e), n),
                        pos: self.finish(start),
                    },
                    None => self.invalid(start),
                };
            } else if self.is_p("[") {
                self.next()?;
                let saved = std::mem::replace(&mut self.no_in, false);
                let idx = self.expr()?;
                self.no_in = saved;
                self.expect_p("]")?;
                e = Expr {
                    kind: ExprKind::Index(Box::new(e), Box::new(idx)),
                    pos: self.finish(start),
                };
            } else if self.is_p("(") && calls {
                let args = self.args()?;
                e = Expr {
                    kind: ExprKind::Call(Box::new(e), args),
                    pos: self.finish(start),
                };
            } else if matches!(self.cur.tok, Tok::Template(..)) {
                self.subset_here("tagged templates are outside Core JS");
                self.template()?;
                e = self.invalid(start);
            } else {
                break;
            }
            self.enter()?;
            links += 1;
        }
        for _ in 0..links {
            self.leave();
        }
        Ok(e)
    }

    fn new_expr(&mut self) -> PResult<Expr> {
        let start = self.here();
        self.enter()?;
        self.next()?;
        if self.is_p(".") {
            self.subset_at("`new.target` is outside Core JS", start);
            self.next()?;
            self.member_name()?;
            self.leave();
            return Ok(self.invalid(start));
        }
        let callee = self.lhs(false)?;
        let args = if self.is_p("(") {
            self.args()?
        } else {
            Vec::new()
        };
        self.leave();
        Ok(Expr {
            kind: ExprKind::New(Box::new(callee), args),
            pos: self.finish(start),
        })
    }

    fn template(&mut self) -> PResult<Expr> {
        let start = self.here();
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        loop {
            let Tok::Template(cooked, tail) = self.cur.tok.clone() else {
                return Err(self.unexpected("expected a template"));
            };
            quasis.push(cooked);
            self.next()?;
            if tail {
                break;
            }
            let saved = std::mem::replace(&mut self.no_in, false);
            exprs.push(self.expr()?);
            self.no_in = saved;
            if !self.is_p("}") {
                return Err(self.unexpected("expected `}` to close the template substitution"));
            }
            let close = self.cur.clone();
            self.cur = self.lx.rescan_template(&close)?;
        }
        Ok(Expr {
            kind: ExprKind::Template(quasis, exprs),
            pos: self.finish(start),
        })
    }

    /// `(a, b) =>`: read the parameters; on anything else, go back and
    /// return `None`.
    fn try_arrow_params(&mut self) -> PResult<Option<Vec<Name>>> {
        let s = self.save();
        self.next()?;
        let ok = loop {
            if self.is_p(")") {
                break true;
            }
            match &self.cur.tok {
                Tok::Ident(n) if !is_reserved(n) => {
                    self.next()?;
                }
                _ => break false,
            }
            if self.is_p(")") {
                break true;
            }
            if !self.eat_p(",")? {
                break false;
            }
        };
        if ok {
            self.next()?;
            if self.is_p("=>") && !self.cur.nl_before {
                self.restore(s);
                return Ok(Some(self.params()?));
            }
        }
        self.restore(s);
        Ok(None)
    }

    fn array(&mut self) -> PResult<ExprKind> {
        self.next()?;
        let saved = std::mem::replace(&mut self.no_in, false);
        let mut items = Vec::new();
        while !self.is_p("]") {
            if self.is_p(",") {
                self.subset_here("array holes are outside Core JS");
                self.next()?;
                continue;
            }
            if self.is_p("...") {
                self.subset_here("spread is outside Core JS");
                self.next()?;
            }
            items.push(self.assign()?);
            if !self.is_p("]") {
                self.expect_p(",")?;
            }
        }
        self.next()?;
        self.no_in = saved;
        Ok(ExprKind::Array(items))
    }

    fn paren_or_arrow(&mut self, start: Pos) -> PResult<Expr> {
        if let Some(params) = self.try_arrow_params()? {
            return self.arrow(params, start);
        }
        self.next()?;
        if self.is_p(")") {
            return Err(self.unexpected("expected an expression"));
        }
        let saved = std::mem::replace(&mut self.no_in, false);
        let e = self.expr()?;
        self.no_in = saved;
        self.expect_p(")")?;
        if self.is_p("=>") {
            self.subset_at("arrow parameters must be plain names in Core JS", start);
            self.arrow(Vec::new(), start)?;
            return Ok(self.invalid(start));
        }
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.here();
        let t = self.cur.clone();
        let kind = match &t.tok {
            Tok::Num(v) => {
                self.next()?;
                ExprKind::Num(*v)
            }
            Tok::Str(s) => {
                self.next()?;
                ExprKind::Str(s.clone())
            }
            Tok::Template(..) => return self.template(),
            Tok::BigInt => {
                self.subset_here("BigInt literals are outside Core JS");
                self.next()?;
                return Ok(self.invalid(start));
            }
            Tok::Punct("/" | "/=") => {
                self.subset_here("regex literals are outside Core JS");
                let t = self.cur.clone();
                self.cur = self.lx.rescan_regex(&t)?;
                self.next()?;
                return Ok(self.invalid(start));
            }
            Tok::Punct("(") => return self.paren_or_arrow(start),
            Tok::Punct("[") => self.array()?,
            Tok::Punct("{") => return self.object(),
            Tok::Ident(w) => return self.word(w.clone(), &t, start),
            _ => return Err(self.unexpected("expected an expression")),
        };
        Ok(Expr {
            kind,
            pos: self.finish(start),
        })
    }

    fn word(&mut self, w: Rc<str>, t: &Token, start: Pos) -> PResult<Expr> {
        let done = |p: &Self, kind| Expr {
            kind,
            pos: p.finish(start),
        };
        if !t.escaped {
            match &*w {
                "this" => {
                    if !self.ctx.this_ok {
                        self.subset_here(
                            "`this` is only part of Core JS inside class methods and constructors",
                        );
                    }
                    self.next()?;
                    let n = self.new_name("this".into(), tok_pos(t));
                    return Ok(done(self, ExprKind::This(n)));
                }
                "null" => {
                    self.next()?;
                    return Ok(done(self, ExprKind::Null));
                }
                "true" | "false" => {
                    self.next()?;
                    return Ok(done(self, ExprKind::Bool(&*w == "true")));
                }
                "function" => {
                    let f = self.function(false)?;
                    return Ok(done(self, ExprKind::Function(Rc::new(f))));
                }
                "async" => {
                    let n = self.peek()?;
                    if !n.nl_before && matches!(&n.tok, Tok::Ident(f) if &**f == "function") {
                        self.subset_here("`async` functions are outside Core JS");
                        self.next()?;
                        self.function(false)?;
                        return Ok(self.invalid(start));
                    }
                }
                "class" => {
                    let c = self.class(false)?;
                    return Ok(done(self, ExprKind::Class(Rc::new(c))));
                }
                "super" => return self.super_expr(),
                "import" => {
                    self.subset_here("`import()` and `import.meta` are outside Core JS");
                    self.next()?;
                    if self.eat_p(".")? {
                        self.member_name()?;
                    } else {
                        self.args()?;
                    }
                    return Ok(self.invalid(start));
                }
                "new" => return self.new_expr(),
                _ => {}
            }
            if is_reserved(&w) {
                return Err(self.err_tok(format!("unexpected `{w}`")));
            }
        }
        if t.escaped {
            self.subset_here("names written with escapes are outside Core JS");
        }
        if &*w == "await" {
            self.subset_here("`await` is outside Core JS");
        }
        self.next()?;
        if self.is_p("=>") && !self.cur.nl_before {
            if &*w == "eval" || &*w == "arguments" {
                return Err(self.err_at(format!("`{w}` can't be declared in strict code"), start));
            }
            let n = self.new_name(w, tok_pos(t));
            return self.arrow(vec![n], start);
        }
        let n = self.new_name(w, tok_pos(t));
        Ok(done(self, ExprKind::Ident(n)))
    }

    fn super_expr(&mut self) -> PResult<Expr> {
        let start = self.here();
        self.next()?;
        if self.is_p("(") {
            if !self.ctx.super_call {
                return Err(self.err_at(
                    "`super()` is only allowed in a derived class constructor",
                    start,
                ));
            }
            let refs = self.super_refs(start);
            let args = self.args()?;
            return Ok(Expr {
                kind: ExprKind::SuperCall(refs, args),
                pos: self.finish(start),
            });
        }
        if !self.ctx.super_member {
            return Err(self.err_at("`super` is only allowed in class methods", start));
        }
        if self.is_p("[") {
            self.subset_here("computed `super[...]` is outside Core JS");
            self.skip_group()?;
            return Ok(self.invalid(start));
        }
        self.expect_p(".")?;
        let refs = self.super_refs(start);
        let name = self.member_name()?;
        Ok(match name {
            Some(n) => Expr {
                kind: ExprKind::SuperMember(refs, n),
                pos: self.finish(start),
            },
            None => self.invalid(start),
        })
    }
}

// ---- object literals ----
impl Parser<'_> {
    fn object(&mut self) -> PResult<Expr> {
        let start = self.here();
        self.next()?;
        let saved = std::mem::replace(&mut self.no_in, false);
        let mut props = Vec::new();
        while !self.is_p("}") {
            let at = self.here();
            if self.is_p("...") {
                self.subset_here("object spread is outside Core JS");
                self.next()?;
                self.assign()?;
            } else {
                self.object_entry(at, &mut props)?;
            }
            if !self.is_p("}") {
                self.expect_p(",")?;
            }
        }
        self.next()?;
        self.no_in = saved;
        Ok(Expr {
            kind: ExprKind::Object(props),
            pos: self.finish(start),
        })
    }

    fn object_entry(&mut self, at: Pos, props: &mut Vec<Prop>) -> PResult<()> {
        let mut special = false;
        if self.is_kw("get") || self.is_kw("set") || self.is_kw("async") {
            let n = self.peek()?;
            if !matches!(n.tok, Tok::Punct("(" | ":" | "," | "}" | "=")) {
                let what = if self.is_kw("async") {
                    "`async` methods are outside Core JS"
                } else {
                    "getters and setters are outside Core JS"
                };
                self.subset_here(what);
                self.next()?;
                special = true;
            }
        }
        if self.is_p("*") {
            self.subset_here("generators are outside Core JS");
            self.next_gen = true;
            self.next()?;
            special = true;
        }
        let key_tok = self.cur.clone();
        let key = self.prop_name()?;
        if self.is_p("(") {
            if !special {
                self.subset_at(
                    "object literal methods are outside Core JS; use a class, or a property holding a function",
                    at,
                );
            }
            self.method(FuncKind::Method, at)?;
            return Ok(());
        }
        if special {
            return Err(self.unexpected("expected `(`"));
        }
        if self.eat_p(":")? {
            let value = self.assign()?;
            if let Some(key) = key {
                if &*key.text == "__proto__" {
                    self.subset_at("a `__proto__` key is outside Core JS", key.pos);
                }
                props.push(Prop {
                    key,
                    value,
                    shorthand: false,
                });
            }
            return Ok(());
        }
        // Shorthand `{ a }`.
        let Tok::Ident(n) = &key_tok.tok else {
            return Err(self.unexpected("expected `:`"));
        };
        if is_reserved(n) {
            return Err(self.err_at(
                format!("`{n}` can't be used as a shorthand property"),
                tok_pos(&key_tok),
            ));
        }
        if self.is_p("=") {
            self.subset_here("destructuring defaults are outside Core JS");
            self.next()?;
            self.assign()?;
        }
        if &**n == "await" {
            self.subset_at("`await` is outside Core JS", tok_pos(&key_tok));
        }
        let name = self.new_name(n.clone(), tok_pos(&key_tok));
        if let Some(key) = key {
            props.push(Prop {
                key,
                value: Expr {
                    kind: ExprKind::Ident(name),
                    pos: tok_pos(&key_tok),
                },
                shorthand: true,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    fn ok(src: &str) -> Program {
        let out = parse(src, false, DEFAULT_MAX_DEPTH);
        assert!(out.subset.is_empty(), "{src}: {:?}", out.subset);
        out.program.unwrap_or_else(|e| panic!("{src}: {e:?}"))
    }

    fn syntax(src: &str) -> Diag {
        match parse(src, false, DEFAULT_MAX_DEPTH).program {
            Err(e) => e,
            Ok(_) => panic!("{src}: parsed"),
        }
    }

    fn subset(src: &str) -> Vec<String> {
        let out = parse(src, false, DEFAULT_MAX_DEPTH);
        if let Err(e) = &out.program {
            panic!("{src}: {e:?}");
        }
        out.subset.into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn core_programs_parse() {
        ok("let a = [1, 2]; const o = { a, b: 2, 'c': 3, 4: 5 }; a.push(o.b);");
        ok("function f(x, y) { if (x < y) { return x; } else return y; }");
        ok("for (let i = 0; i < 3; i++) { continue; } for (const x of [1]) {}");
        ok("class A { constructor(x) { this.x = x; } m() { return this.x; } static s() {} }");
        ok("class B extends A { constructor() { super(1); } m() { return super.m() + 1; } }");
        ok("const f = (a, b) => a + b; const g = x => { return x; }; const h = () => 1;");
        ok("outer: for (;;) { inner: while (true) { break outer; } }");
        ok("switch (1) { case 1: break; default: }");
        ok("try { throw new Error('x'); } catch (e) { } finally { }");
        ok("let s = `a${1 + 2}b${`c${3}`}`; typeof s === 'string';");
        ok("let x = a\n++b");
        ok("x = 2 ** 3 ** 2; y = (-2) ** 2; z = a ? b : c, d;");
        ok("for (x of a) {} for (var i = 0, j = 1; i < j; i++) {}");
        ok("m.delete(k); o.class = 1; x = new A().b; y = new A;");
        ok("blk: { break blk; }");
    }

    #[test]
    fn syntax_errors() {
        assert!(syntax("let = 1;").message.contains("expected a name"));
        assert!(syntax("return 1;").message.contains("outside a function"));
        assert!(syntax("break;").message.contains("`break`"));
        assert!(syntax("blk: { break; }").message.contains("`break`"));
        assert!(
            syntax("x: while (1) { continue y; }")
                .message
                .contains("no label")
        );
        assert!(syntax("-2 ** 2").message.contains("parentheses"));
        assert!(syntax("const a;").message.contains("needs a value"));
        assert!(syntax("function f(a, a) {}").message.contains("duplicate"));
        assert!(syntax("1 = 2").message.contains("invalid assignment"));
        assert!(syntax("throw\n1").message.contains("line break"));
        assert!(
            syntax("if (1) function f() {}")
                .message
                .contains("declaration")
        );
        assert!(
            syntax("class A { m() { super(); } }")
                .message
                .contains("derived")
        );
        let e = syntax("let a = 1\nlet b = ;");
        assert_eq!((e.line, e.col), (2, 9));
    }

    #[test]
    fn subset_errors_are_all_reported() {
        let found = subset("a == b; c != d; for (k in o) {} delete o.x; void 0; let {p} = o;");
        assert_eq!(found.len(), 6, "{found:?}");
        assert!(subset("x = /re/g;")[0].contains("regex"));
        assert!(subset("f(...a);")[0].contains("spread"));
        assert!(subset("function f(a = 1) {}")[0].contains("default"));
        assert!(subset("o = { get x() { return 1; } };")[0].contains("getters"));
        assert!(subset("class A { x = 1; }")[0].contains("fields"));
        assert!(subset("function* g() {}")[0].contains("generators"));
        let found = subset("function* g() { yield 1; yield; yield* h(); }");
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found[1].contains("`yield`"));
        let found = subset("class A { *m() { const x = yield 2; } }");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(subset("a?.b;")[0].contains("optional"));
        assert!(subset("a ?? b;")[0].contains("??"));
        assert!(subset("[1, , 2];")[0].contains("holes"));
        assert!(subset("o = { m() {} };")[0].contains("methods"));
        assert!(subset("function f() { return this; }")[0].contains("`this`"));
    }

    #[test]
    fn depth_is_capped() {
        // Debug builds use a lot of stack per level; the tools run the
        // parser on a large stack, and so does this test.
        let t = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(|| {
                let deep = format!("{}1{}", "(".repeat(2000), ")".repeat(2000));
                assert!(syntax(&deep).message.contains("limit"));
                let chain = format!("x{}", ".a".repeat(5000));
                assert!(syntax(&chain).message.contains("limit"));
            })
            .expect("thread starts");
        t.join().expect("no panic");
    }
}
