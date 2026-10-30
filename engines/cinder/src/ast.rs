// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The syntax tree the parser produces.
//!
//! Every node owns its children. Nodes that matter for error messages carry
//! a [`Span`]. Strings are stored as UTF-16 code units ([`JsString`]), so a
//! lone surrogate written as `"\uD800"` survives unchanged. Names
//! (identifiers, property names) are Rust `String`s, because a name can't
//! hold a lone surrogate.
//!
//! Parenthesized expressions keep a [`ExprKind::Paren`] node. The compiler
//! can skip it; it's kept so the early-error checks and any later printer can
//! tell `(a) = 1` from `a = 1`.
//!
//! This module is the contract between the parser and the compiler. Changes
//! here should be additive where possible.

use crate::cinder::error::Span;
pub use crate::cinder::lexer::BigIntLit;

/// A string value in UTF-16 code units.
pub type JsString = Vec<u16>;

/// Script or module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceType {
    /// A classic script.
    Script,
    /// An ES module.
    Module,
}

/// A whole parsed source text.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Script or module.
    pub source_type: SourceType,
    /// Top-level statements, directives first.
    pub body: Vec<Stmt>,
    /// True when the whole program is strict (modules always are).
    pub strict: bool,
    /// The whole source.
    pub span: Span,
}

/// A name with its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    /// The decoded name.
    pub name: String,
    /// Where it is.
    pub span: Span,
}

/// A statement or declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// What it is.
    pub kind: StmtKind,
    /// Where it is.
    pub span: Span,
}

/// Statement kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// An expression statement. `directive` holds the raw text (without
    /// quotes) when the statement is part of a directive prologue.
    Expr {
        /// The expression.
        expr: Box<Expr>,
        /// Raw directive text, such as `use strict`.
        directive: Option<String>,
    },
    /// `var`, `let`, or `const`.
    Var(VarDecl),
    /// A function declaration.
    Function(Box<Function>),
    /// A class declaration.
    Class(Box<Class>),
    /// `{ ... }`
    Block(Vec<Stmt>),
    /// `;`
    Empty,
    /// `if (test) cons else alt`
    If {
        /// Condition.
        test: Box<Expr>,
        /// Then branch.
        cons: Box<Stmt>,
        /// Else branch.
        alt: Option<Box<Stmt>>,
    },
    /// `for (init; test; update) body`
    For {
        /// Initializer.
        init: Option<ForInit>,
        /// Condition.
        test: Option<Box<Expr>>,
        /// Update.
        update: Option<Box<Expr>>,
        /// Body.
        body: Box<Stmt>,
    },
    /// `for (left in right) body`
    ForIn {
        /// Binding or target.
        left: ForHead,
        /// Object.
        right: Box<Expr>,
        /// Body.
        body: Box<Stmt>,
    },
    /// `for (left of right) body` and `for await (...)`.
    ForOf {
        /// Binding or target.
        left: ForHead,
        /// Iterable.
        right: Box<Expr>,
        /// Body.
        body: Box<Stmt>,
        /// `for await`.
        is_await: bool,
    },
    /// `while (test) body`
    While {
        /// Condition.
        test: Box<Expr>,
        /// Body.
        body: Box<Stmt>,
    },
    /// `do body while (test)`
    DoWhile {
        /// Body.
        body: Box<Stmt>,
        /// Condition.
        test: Box<Expr>,
    },
    /// `continue label;`
    Continue(Option<Ident>),
    /// `break label;`
    Break(Option<Ident>),
    /// `return arg;`
    Return(Option<Box<Expr>>),
    /// `with (object) body` (sloppy mode only).
    With {
        /// Scope object.
        object: Box<Expr>,
        /// Body.
        body: Box<Stmt>,
    },
    /// `switch (disc) { cases }`
    Switch {
        /// Discriminant.
        disc: Box<Expr>,
        /// Cases in source order.
        cases: Vec<SwitchCase>,
    },
    /// `label: body`
    Labeled {
        /// Label.
        label: Ident,
        /// Body.
        body: Box<Stmt>,
    },
    /// `throw arg;`
    Throw(Box<Expr>),
    /// `try { } catch (p) { } finally { }`
    Try {
        /// Protected block.
        block: Vec<Stmt>,
        /// Catch clause.
        handler: Option<CatchClause>,
        /// Finally block.
        finalizer: Option<Vec<Stmt>>,
    },
    /// `debugger;`
    Debugger,
    /// An import declaration (modules only).
    Import(ImportDecl),
    /// An export declaration (modules only).
    Export(ExportDecl),
}

/// `var`, `let`, or `const`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// `var`
    Var,
    /// `let`
    Let,
    /// `const`
    Const,
}

/// A variable declaration with one or more declarators.
#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    /// Which keyword.
    pub kind: VarKind,
    /// The declarators.
    pub decls: Vec<VarDeclarator>,
    /// Where it is.
    pub span: Span,
}

/// One `target = init` inside a declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct VarDeclarator {
    /// Name or destructuring pattern.
    pub target: Pat,
    /// Initializer.
    pub init: Option<Box<Expr>>,
    /// Where it is.
    pub span: Span,
}

/// The first part of a classic `for` loop.
#[derive(Clone, Debug, PartialEq)]
pub enum ForInit {
    /// A declaration.
    Var(VarDecl),
    /// An expression.
    Expr(Box<Expr>),
}

/// The left side of `for-in` and `for-of`.
#[derive(Clone, Debug, PartialEq)]
pub enum ForHead {
    /// A declaration with exactly one declarator.
    Var(VarDecl),
    /// An assignment target.
    Pat(Pat),
}

/// One `case` or `default` clause.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchCase {
    /// `None` for `default`.
    pub test: Option<Box<Expr>>,
    /// Statements under the clause.
    pub body: Vec<Stmt>,
    /// Where it is.
    pub span: Span,
}

/// `catch (param) { body }`
#[derive(Clone, Debug, PartialEq)]
pub struct CatchClause {
    /// `None` for `catch { }`.
    pub param: Option<Pat>,
    /// The block.
    pub body: Vec<Stmt>,
    /// Where it is.
    pub span: Span,
}

/// A function of any kind except arrows: declarations, expressions,
/// methods, getters, setters, and constructors.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// The name, if written.
    pub id: Option<Ident>,
    /// Parameters. The last one may be a [`PatKind::Rest`].
    pub params: Vec<Pat>,
    /// Body statements, directives first.
    pub body: Vec<Stmt>,
    /// `async function`
    pub is_async: bool,
    /// `function*`
    pub is_generator: bool,
    /// The body is strict code.
    pub strict: bool,
    /// Every parameter is a plain name, with no defaults, rest, or patterns.
    pub simple_params: bool,
    /// Where it is.
    pub span: Span,
}

/// An arrow function.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowFunction {
    /// Parameters. The last one may be a [`PatKind::Rest`].
    pub params: Vec<Pat>,
    /// Body.
    pub body: ArrowBody,
    /// `async (...) => ...`
    pub is_async: bool,
    /// The body is strict code.
    pub strict: bool,
    /// Where it is.
    pub span: Span,
}

/// An arrow function body.
#[derive(Clone, Debug, PartialEq)]
pub enum ArrowBody {
    /// `=> expr`
    Expr(Box<Expr>),
    /// `=> { ... }`
    Block(Vec<Stmt>),
}

/// A class declaration or expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    /// The name, if written.
    pub id: Option<Ident>,
    /// `extends` expression.
    pub super_class: Option<Box<Expr>>,
    /// Members in source order.
    pub body: Vec<ClassMember>,
    /// Where it is.
    pub span: Span,
}

/// One class member.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassMember {
    /// What it is.
    pub kind: ClassMemberKind,
    /// Where it is.
    pub span: Span,
}

/// Method flavours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    /// A plain method.
    Method,
    /// `get name() {}`
    Get,
    /// `set name(v) {}`
    Set,
    /// The class constructor.
    Constructor,
}

/// Class member kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ClassMemberKind {
    /// A method, accessor, or the constructor.
    Method {
        /// Name.
        key: PropKey,
        /// Flavour.
        kind: MethodKind,
        /// `static`
        is_static: bool,
        /// The function.
        value: Box<Function>,
    },
    /// A field, with an optional initializer.
    Field {
        /// Name.
        key: PropKey,
        /// `static`
        is_static: bool,
        /// Initializer.
        value: Option<Box<Expr>>,
    },
    /// `static { ... }`
    StaticBlock(Vec<Stmt>),
}

/// A property name in an object literal, class, or pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum PropKey {
    /// An identifier name, reserved words included.
    Ident(Ident),
    /// A string literal key.
    Str(JsString, Span),
    /// A numeric literal key.
    Num(f64, Span),
    /// A BigInt literal key.
    BigInt(BigIntLit, Span),
    /// `[expr]`
    Computed(Box<Expr>),
    /// `#name` (classes only).
    Private(Ident),
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub span: Span,
}

/// Expression kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// An identifier reference.
    Ident(String),
    /// `this`
    This,
    /// `super`, only as the callee of a call or the object of a member.
    Super,
    /// `null`
    Null,
    /// `true` or `false`
    Bool(bool),
    /// A number.
    Num(f64),
    /// A BigInt.
    BigInt(BigIntLit),
    /// A string.
    Str(JsString),
    /// A regex literal. The pattern is checked later by the regex compiler.
    Regex {
        /// Text between the slashes.
        pattern: String,
        /// Flags.
        flags: String,
    },
    /// An untagged template.
    Template(Template),
    /// `` tag`...` ``
    TaggedTemplate {
        /// The tag.
        tag: Box<Expr>,
        /// The template.
        quasi: Template,
    },
    /// `[a, , ...b]`. `None` is a hole. Spread elements are
    /// [`ExprKind::Spread`].
    Array(Vec<Option<Expr>>),
    /// `{ ... }`
    Object(Vec<Prop>),
    /// A function expression.
    Function(Box<Function>),
    /// An arrow function.
    Arrow(Box<ArrowFunction>),
    /// A class expression.
    Class(Box<Class>),
    /// A prefix operator.
    Unary {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        arg: Box<Expr>,
    },
    /// `++` or `--`.
    Update {
        /// Operator.
        op: UpdateOp,
        /// Prefix or postfix.
        prefix: bool,
        /// Operand.
        arg: Box<Expr>,
    },
    /// A binary operator.
    Binary {
        /// Operator.
        op: BinaryOp,
        /// Left side.
        left: Box<Expr>,
        /// Right side.
        right: Box<Expr>,
    },
    /// `&&`, `||`, `??`.
    Logical {
        /// Operator.
        op: LogicalOp,
        /// Left side.
        left: Box<Expr>,
        /// Right side.
        right: Box<Expr>,
    },
    /// An assignment.
    Assign {
        /// Operator.
        op: AssignOp,
        /// Target. Only `=` can have a destructuring pattern.
        target: Box<Pat>,
        /// Value.
        value: Box<Expr>,
    },
    /// `test ? cons : alt`
    Cond {
        /// Condition.
        test: Box<Expr>,
        /// Value when true.
        cons: Box<Expr>,
        /// Value when false.
        alt: Box<Expr>,
    },
    /// A call. `optional` marks `f?.()`.
    Call {
        /// Callee.
        callee: Box<Expr>,
        /// Arguments, spread included.
        args: Vec<Expr>,
        /// `?.(`
        optional: bool,
    },
    /// `new callee(args)`. `new X` with no parentheses has no arguments.
    New {
        /// Constructor.
        callee: Box<Expr>,
        /// Arguments.
        args: Vec<Expr>,
    },
    /// Property access. `optional` marks `a?.b` and `a?.[b]`.
    Member {
        /// Object.
        object: Box<Expr>,
        /// Property.
        prop: MemberProp,
        /// `?.`
        optional: bool,
    },
    /// Wraps a whole chain that contains at least one `?.`, so the
    /// short-circuit covers all of it.
    OptionalChain(Box<Expr>),
    /// `a, b, c`
    Seq(Vec<Expr>),
    /// `...expr` in arrays, calls, and `new`.
    Spread(Box<Expr>),
    /// `yield arg` or `yield* arg`.
    Yield {
        /// Operand.
        arg: Option<Box<Expr>>,
        /// `yield*`
        delegate: bool,
    },
    /// `await arg`
    Await(Box<Expr>),
    /// `new.target`
    NewTarget,
    /// `import.meta`
    ImportMeta,
    /// `import(source, options)`
    Import {
        /// Specifier.
        source: Box<Expr>,
        /// Second argument.
        options: Option<Box<Expr>>,
    },
    /// `(expr)`
    Paren(Box<Expr>),
    /// `#name in obj`
    PrivateIn {
        /// The private name.
        name: Ident,
        /// The object.
        right: Box<Expr>,
    },
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum UnaryOp {
    Minus,
    Plus,
    Not,
    BitNot,
    Typeof,
    Void,
    Delete,
}

/// `++` and `--`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum UpdateOp {
    Inc,
    Dec,
}

/// Binary operators other than the logical ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum BinaryOp {
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Shl,
    Shr,
    UShr,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    In,
    Instanceof,
}

/// Short-circuit operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum LogicalOp {
    And,
    Or,
    Nullish,
}

/// Assignment operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    And,
    Or,
    Nullish,
}

/// The property part of a member expression.
#[derive(Clone, Debug, PartialEq)]
pub enum MemberProp {
    /// `.name`
    Name(Ident),
    /// `[expr]`
    Computed(Box<Expr>),
    /// `.#name`
    Private(Ident),
}

/// A template literal.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// Literal pieces. Always one more than `exprs`.
    pub quasis: Vec<TemplateElement>,
    /// Substitutions.
    pub exprs: Vec<Expr>,
}

/// One literal piece of a template.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateElement {
    /// Cooked value. `None` only in tagged templates with a bad escape.
    pub cooked: Option<JsString>,
    /// Raw text with line endings turned into LF.
    pub raw: String,
    /// Where it is.
    pub span: Span,
}

/// An object literal entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    /// What it is.
    pub kind: PropKind,
    /// Where it is.
    pub span: Span,
}

/// Object literal entry kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum PropKind {
    /// `key: value`
    Init {
        /// Name.
        key: PropKey,
        /// Value.
        value: Box<Expr>,
    },
    /// `{ name }`
    Shorthand(Ident),
    /// A method, getter, or setter.
    Method {
        /// Name.
        key: PropKey,
        /// Flavour (never `Constructor`).
        kind: MethodKind,
        /// The function.
        value: Box<Function>,
    },
    /// `...expr`
    Spread(Box<Expr>),
}

/// A binding or assignment pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Pat {
    /// What it is.
    pub kind: PatKind,
    /// Where it is.
    pub span: Span,
}

/// Pattern kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum PatKind {
    /// A plain name.
    Ident(String),
    /// `[a, , b, ...rest]`. `None` is a hole.
    Array(Vec<Option<Pat>>),
    /// `{ a, b: c, ...rest }`
    Object(Vec<PatProp>),
    /// `target = default`
    Assign {
        /// The target.
        target: Box<Pat>,
        /// Value used when the incoming value is `undefined`.
        default: Box<Expr>,
    },
    /// `...target`, always last in its list.
    Rest(Box<Pat>),
    /// A member expression target (assignment patterns only).
    Expr(Box<Expr>),
}

/// An entry in an object pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum PatProp {
    /// `key: value`, or `name` / `name = default` when `shorthand` is set.
    Prop {
        /// Property name.
        key: PropKey,
        /// Where the value goes.
        value: Pat,
        /// Written as `{ name }` or `{ name = default }`.
        shorthand: bool,
    },
    /// `...rest`
    Rest(Pat),
}

/// `import ... from "source"`
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDecl {
    /// What is imported. Empty for `import "x"`.
    pub specifiers: Vec<ImportSpec>,
    /// The module specifier.
    pub source: StrLit,
    /// `with { type: "json" }` entries.
    pub attributes: Vec<ImportAttribute>,
}

/// A string literal with its span.
#[derive(Clone, Debug, PartialEq)]
pub struct StrLit {
    /// The value.
    pub value: JsString,
    /// Where it is.
    pub span: Span,
}

/// One `key: "value"` import attribute.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportAttribute {
    /// Attribute name.
    pub key: ModuleName,
    /// Attribute value.
    pub value: StrLit,
}

/// A name in an import or export list: an identifier or a string.
#[derive(Clone, Debug, PartialEq)]
pub enum ModuleName {
    /// `name`
    Ident(Ident),
    /// `"name"`
    Str(StrLit),
}

/// Import specifier kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ImportSpec {
    /// `import x from`
    Default(Ident),
    /// `import * as x from`
    Namespace(Ident),
    /// `import { a as b } from`
    Named {
        /// Name in the other module.
        imported: ModuleName,
        /// Local binding.
        local: Ident,
    },
}

/// Export declaration kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportDecl {
    /// `export { a as b }` or `export { a } from "x"`.
    Named {
        /// The list.
        specifiers: Vec<ExportSpec>,
        /// `from` source.
        source: Option<StrLit>,
        /// Import attributes on the source.
        attributes: Vec<ImportAttribute>,
    },
    /// `export * from "x"` or `export * as ns from "x"`.
    All {
        /// `as` name.
        exported: Option<ModuleName>,
        /// Source.
        source: StrLit,
        /// Import attributes.
        attributes: Vec<ImportAttribute>,
    },
    /// `export var/let/const/function/class ...`
    Decl(Box<Stmt>),
    /// `export default ...`
    Default(ExportDefault),
}

/// One `local as exported` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSpec {
    /// Local name (or name in the source module).
    pub local: ModuleName,
    /// Exported name.
    pub exported: ModuleName,
}

/// What follows `export default`.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportDefault {
    /// A function declaration, name optional.
    Function(Box<Function>),
    /// A class declaration, name optional.
    Class(Box<Class>),
    /// Any other expression.
    Expr(Box<Expr>),
}

impl ModuleName {
    /// The name as text (strings are converted lossily).
    pub fn text(&self) -> String {
        match self {
            ModuleName::Ident(i) => i.name.clone(),
            ModuleName::Str(s) => String::from_utf16_lossy(&s.value),
        }
    }

    /// Where it is.
    pub fn span(&self) -> Span {
        match self {
            ModuleName::Ident(i) => i.span,
            ModuleName::Str(s) => s.span,
        }
    }
}

impl Pat {
    /// Names bound by this pattern, in source order.
    pub fn bound_names(&self) -> Vec<Ident> {
        let mut out = Vec::new();
        collect_names(self, &mut out);
        out
    }
}

fn collect_names(p: &Pat, out: &mut Vec<Ident>) {
    match &p.kind {
        PatKind::Ident(n) => out.push(Ident {
            name: n.clone(),
            span: p.span,
        }),
        PatKind::Array(items) => {
            for item in items.iter().flatten() {
                collect_names(item, out);
            }
        }
        PatKind::Object(props) => {
            for prop in props {
                match prop {
                    PatProp::Prop { value, .. } => collect_names(value, out),
                    PatProp::Rest(r) => collect_names(r, out),
                }
            }
        }
        PatKind::Assign { target, .. } => collect_names(target, out),
        PatKind::Rest(inner) => collect_names(inner, out),
        PatKind::Expr(_) => {}
    }
}

// ---- dropping deep trees ----
//
// A long chain such as `a.b.c.d...` or `1 + 2 + 3 + ...` builds a tree as
// deep as the chain is long. The default drop glue would free it with one
// stack frame per level. These `Drop` impls free children from a heap work
// list instead, so freeing never depends on how deep the tree is.
//
// One side effect: fields of `Expr`, `Stmt`, and `Pat` can't be moved out
// by pattern matching. Use `std::mem::replace` (or `take_kind`) to get the
// kind out first.

impl Expr {
    /// A childless placeholder node.
    pub fn placeholder() -> Expr {
        Expr {
            kind: ExprKind::Null,
            span: Span::default(),
        }
    }

    /// Move the kind out, leaving a placeholder behind.
    pub fn take_kind(&mut self) -> ExprKind {
        std::mem::replace(&mut self.kind, ExprKind::Null)
    }
}

impl Stmt {
    /// A childless placeholder node.
    pub fn placeholder() -> Stmt {
        Stmt {
            kind: StmtKind::Empty,
            span: Span::default(),
        }
    }

    /// Move the kind out, leaving a placeholder behind.
    pub fn take_kind(&mut self) -> StmtKind {
        std::mem::replace(&mut self.kind, StmtKind::Empty)
    }
}

impl Pat {
    /// A childless placeholder node.
    pub fn placeholder() -> Pat {
        Pat {
            kind: PatKind::Ident(String::new()),
            span: Span::default(),
        }
    }

    /// Move the kind out, leaving a placeholder behind.
    pub fn take_kind(&mut self) -> PatKind {
        std::mem::replace(&mut self.kind, PatKind::Ident(String::new()))
    }
}

enum Node {
    E(Expr),
    S(Stmt),
    P(Pat),
}

fn take_e(b: &mut Expr) -> Node {
    Node::E(std::mem::replace(b, Expr::placeholder()))
}

fn take_s(b: &mut Stmt) -> Node {
    Node::S(std::mem::replace(b, Stmt::placeholder()))
}

fn take_p(b: &mut Pat) -> Node {
    Node::P(std::mem::replace(b, Pat::placeholder()))
}

fn drain_stmts(v: &mut Vec<Stmt>, out: &mut Vec<Node>) {
    out.extend(std::mem::take(v).into_iter().map(Node::S));
}

fn drain_exprs(v: &mut Vec<Expr>, out: &mut Vec<Node>) {
    out.extend(std::mem::take(v).into_iter().map(Node::E));
}

fn drain_pats(v: &mut Vec<Pat>, out: &mut Vec<Node>) {
    out.extend(std::mem::take(v).into_iter().map(Node::P));
}

fn drain_key(k: &mut PropKey, out: &mut Vec<Node>) {
    if let PropKey::Computed(e) = k {
        out.push(take_e(e));
    }
}

fn drain_function(f: &mut Function, out: &mut Vec<Node>) {
    drain_pats(&mut f.params, out);
    drain_stmts(&mut f.body, out);
}

fn drain_class(c: &mut Class, out: &mut Vec<Node>) {
    if let Some(s) = &mut c.super_class {
        out.push(take_e(s));
    }
    for mut m in std::mem::take(&mut c.body) {
        match &mut m.kind {
            ClassMemberKind::Method { key, value, .. } => {
                drain_key(key, out);
                drain_function(value, out);
            }
            ClassMemberKind::Field { key, value, .. } => {
                drain_key(key, out);
                if let Some(v) = value {
                    out.push(take_e(v));
                }
            }
            ClassMemberKind::StaticBlock(body) => drain_stmts(body, out),
        }
    }
}

fn drain_var(d: &mut VarDecl, out: &mut Vec<Node>) {
    for mut v in std::mem::take(&mut d.decls) {
        out.push(take_p(&mut v.target));
        if let Some(i) = &mut v.init {
            out.push(take_e(i));
        }
    }
}

fn expr_is_leaf(k: &ExprKind) -> bool {
    matches!(
        k,
        ExprKind::Ident(_)
            | ExprKind::This
            | ExprKind::Super
            | ExprKind::Null
            | ExprKind::Bool(_)
            | ExprKind::Num(_)
            | ExprKind::BigInt(_)
            | ExprKind::Str(_)
            | ExprKind::Regex { .. }
            | ExprKind::NewTarget
            | ExprKind::ImportMeta
    )
}

fn drain_expr(k: &mut ExprKind, out: &mut Vec<Node>) {
    match k {
        ExprKind::Ident(_)
        | ExprKind::This
        | ExprKind::Super
        | ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Num(_)
        | ExprKind::BigInt(_)
        | ExprKind::Str(_)
        | ExprKind::Regex { .. }
        | ExprKind::NewTarget
        | ExprKind::ImportMeta => {}
        ExprKind::Template(t) => drain_exprs(&mut t.exprs, out),
        ExprKind::TaggedTemplate { tag, quasi } => {
            out.push(take_e(tag));
            drain_exprs(&mut quasi.exprs, out);
        }
        ExprKind::Array(items) => {
            out.extend(std::mem::take(items).into_iter().flatten().map(Node::E));
        }
        ExprKind::Object(props) => {
            for mut p in std::mem::take(props) {
                match &mut p.kind {
                    PropKind::Init { key, value } => {
                        drain_key(key, out);
                        out.push(take_e(value));
                    }
                    PropKind::Shorthand(_) => {}
                    PropKind::Method { key, value, .. } => {
                        drain_key(key, out);
                        drain_function(value, out);
                    }
                    PropKind::Spread(e) => out.push(take_e(e)),
                }
            }
        }
        ExprKind::Function(f) => drain_function(f, out),
        ExprKind::Arrow(a) => {
            drain_pats(&mut a.params, out);
            match &mut a.body {
                ArrowBody::Expr(e) => out.push(take_e(e)),
                ArrowBody::Block(b) => drain_stmts(b, out),
            }
        }
        ExprKind::Class(c) => drain_class(c, out),
        ExprKind::Unary { arg, .. }
        | ExprKind::Update { arg, .. }
        | ExprKind::Await(arg)
        | ExprKind::Spread(arg)
        | ExprKind::Paren(arg)
        | ExprKind::OptionalChain(arg) => out.push(take_e(arg)),
        ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. } => {
            out.push(take_e(left));
            out.push(take_e(right));
        }
        ExprKind::Assign { target, value, .. } => {
            out.push(take_p(target));
            out.push(take_e(value));
        }
        ExprKind::Cond { test, cons, alt } => {
            out.push(take_e(test));
            out.push(take_e(cons));
            out.push(take_e(alt));
        }
        ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
            out.push(take_e(callee));
            drain_exprs(args, out);
        }
        ExprKind::Member { object, prop, .. } => {
            out.push(take_e(object));
            if let MemberProp::Computed(e) = prop {
                out.push(take_e(e));
            }
        }
        ExprKind::Seq(items) => drain_exprs(items, out),
        ExprKind::Yield { arg, .. } => {
            if let Some(a) = arg {
                out.push(take_e(a));
            }
        }
        ExprKind::Import { source, options } => {
            out.push(take_e(source));
            if let Some(o) = options {
                out.push(take_e(o));
            }
        }
        ExprKind::PrivateIn { right, .. } => out.push(take_e(right)),
    }
}

fn drain_stmt(k: &mut StmtKind, out: &mut Vec<Node>) {
    match k {
        StmtKind::Empty
        | StmtKind::Debugger
        | StmtKind::Continue(_)
        | StmtKind::Break(_)
        | StmtKind::Import(_) => {}
        StmtKind::Expr { expr, .. } | StmtKind::Throw(expr) => out.push(take_e(expr)),
        StmtKind::Var(d) => drain_var(d, out),
        StmtKind::Function(f) => drain_function(f, out),
        StmtKind::Class(c) => drain_class(c, out),
        StmtKind::Block(b) => drain_stmts(b, out),
        StmtKind::If { test, cons, alt } => {
            out.push(take_e(test));
            out.push(take_s(cons));
            if let Some(a) = alt {
                out.push(take_s(a));
            }
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            match init {
                Some(ForInit::Var(d)) => drain_var(d, out),
                Some(ForInit::Expr(e)) => out.push(take_e(e)),
                None => {}
            }
            for e in [test, update].into_iter().flatten() {
                out.push(take_e(e));
            }
            out.push(take_s(body));
        }
        StmtKind::ForIn { left, right, body }
        | StmtKind::ForOf {
            left, right, body, ..
        } => {
            match left {
                ForHead::Var(d) => drain_var(d, out),
                ForHead::Pat(p) => out.push(take_p(p)),
            }
            out.push(take_e(right));
            out.push(take_s(body));
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            out.push(take_e(test));
            out.push(take_s(body));
        }
        StmtKind::Return(arg) => {
            if let Some(a) = arg {
                out.push(take_e(a));
            }
        }
        StmtKind::With { object, body } => {
            out.push(take_e(object));
            out.push(take_s(body));
        }
        StmtKind::Switch { disc, cases } => {
            out.push(take_e(disc));
            for mut c in std::mem::take(cases) {
                if let Some(t) = &mut c.test {
                    out.push(take_e(t));
                }
                drain_stmts(&mut c.body, out);
            }
        }
        StmtKind::Labeled { body, .. } => out.push(take_s(body)),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            drain_stmts(block, out);
            if let Some(h) = handler {
                if let Some(p) = &mut h.param {
                    out.push(take_p(p));
                }
                drain_stmts(&mut h.body, out);
            }
            if let Some(f) = finalizer {
                drain_stmts(f, out);
            }
        }
        StmtKind::Export(e) => match e {
            ExportDecl::Named { .. } | ExportDecl::All { .. } => {}
            ExportDecl::Decl(s) => out.push(take_s(s)),
            ExportDecl::Default(ExportDefault::Function(f)) => drain_function(f, out),
            ExportDecl::Default(ExportDefault::Class(c)) => drain_class(c, out),
            ExportDecl::Default(ExportDefault::Expr(x)) => out.push(take_e(x)),
        },
    }
}

fn drain_pat(k: &mut PatKind, out: &mut Vec<Node>) {
    match k {
        PatKind::Ident(_) => {}
        PatKind::Array(items) => {
            out.extend(std::mem::take(items).into_iter().flatten().map(Node::P))
        }
        PatKind::Object(props) => {
            for p in std::mem::take(props) {
                match p {
                    PatProp::Prop { mut key, value, .. } => {
                        drain_key(&mut key, out);
                        out.push(Node::P(value));
                    }
                    PatProp::Rest(r) => out.push(Node::P(r)),
                }
            }
        }
        PatKind::Assign { target, default } => {
            out.push(take_p(target));
            out.push(take_e(default));
        }
        PatKind::Rest(t) => out.push(take_p(t)),
        PatKind::Expr(e) => out.push(take_e(e)),
    }
}

fn drain_all(mut work: Vec<Node>) {
    while let Some(node) = work.pop() {
        match node {
            Node::E(mut e) => {
                drain_expr(&mut e.kind, &mut work);
                e.kind = ExprKind::Null;
            }
            Node::S(mut s) => {
                drain_stmt(&mut s.kind, &mut work);
                s.kind = StmtKind::Empty;
            }
            Node::P(mut p) => {
                drain_pat(&mut p.kind, &mut work);
                p.kind = PatKind::Ident(String::new());
            }
        }
    }
}

impl Drop for Expr {
    fn drop(&mut self) {
        if expr_is_leaf(&self.kind) {
            return;
        }
        let mut work = Vec::new();
        drain_expr(&mut self.kind, &mut work);
        drain_all(work);
    }
}

impl Drop for Stmt {
    fn drop(&mut self) {
        if matches!(
            self.kind,
            StmtKind::Empty | StmtKind::Debugger | StmtKind::Break(_) | StmtKind::Continue(_)
        ) {
            return;
        }
        let mut work = Vec::new();
        drain_stmt(&mut self.kind, &mut work);
        drain_all(work);
    }
}

impl Drop for Pat {
    fn drop(&mut self) {
        if matches!(self.kind, PatKind::Ident(_)) {
            return;
        }
        let mut work = Vec::new();
        drain_pat(&mut self.kind, &mut work);
        drain_all(work);
    }
}
