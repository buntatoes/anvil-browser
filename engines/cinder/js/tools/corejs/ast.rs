// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Syntax tree for Core JS.
//!
//! Only what Core JS allows has a node. Anything else is reported by the
//! parser as a subset error and replaced by an `Invalid` node so parsing can
//! go on and report the rest.
//!
//! Every name occurrence ([`Name`]) has a program-unique `id`, and every
//! scope-making node a program-unique `scope` number, so later passes keep
//! their results in side tables indexed by them.

use std::rc::Rc;

/// A string in UTF-16 code units.
pub type JStr = Rc<[u16]>;

/// A source position: byte range plus line and UTF-16 column of the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    /// Byte offset of the start.
    pub start: u32,
    /// Byte offset just past the end.
    pub end: u32,
    /// 1-based line.
    pub line: u32,
    /// 1-based column in UTF-16 code units.
    pub col: u32,
}

/// One occurrence of a name, as a declaration or a reference.
#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    /// The name.
    pub name: Rc<str>,
    /// Unique per program.
    pub id: u32,
    /// Where it is.
    pub pos: Pos,
}

/// A property name (after `.`, as an object key, a method name).
#[derive(Clone, Debug, PartialEq)]
pub struct PropName {
    /// The name as text.
    pub text: Rc<str>,
    /// The name as code units.
    pub units: JStr,
    /// Where it is.
    pub pos: Pos,
}

/// `var`, `let` or `const`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    /// `var`
    Var,
    /// `let`
    Let,
    /// `const`
    Const,
}

/// A statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// What it is.
    pub kind: StmtKind,
    /// Where it is.
    pub pos: Pos,
}

/// A `{ ... }` block with its scope number.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Statements.
    pub stmts: Vec<Stmt>,
    /// Scope number.
    pub scope: u32,
}

/// The first clause of a `for` loop.
#[derive(Clone, Debug, PartialEq)]
pub enum ForInit {
    /// A declaration.
    Var(VarKind, Vec<(Name, Option<Expr>)>),
    /// An expression.
    Expr(Expr),
}

/// `catch (param) { body }`
#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    /// Scope holding the parameter.
    pub scope: u32,
    /// Parameter, if written.
    pub param: Option<Name>,
    /// Body.
    pub body: Block,
}

/// A `case` or `default` clause.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// `None` for `default`.
    pub test: Option<Expr>,
    /// Statements.
    pub body: Vec<Stmt>,
}

/// One imported name.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportName {
    /// Name exported by the other module.
    pub imported: Rc<str>,
    /// Local binding (the same text; renaming with `as` is not part of the
    /// module form).
    pub local: Name,
}

/// Statement kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// Expression statement.
    Expr(Expr),
    /// Declaration.
    Var(VarKind, Vec<(Name, Option<Expr>)>),
    /// Function declaration.
    Func(Rc<Func>),
    /// Class declaration.
    Class(Rc<Class>),
    /// Block.
    Block(Block),
    /// `if`
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    /// `while`
    While(Expr, Box<Stmt>),
    /// `do ... while`
    DoWhile(Box<Stmt>, Expr),
    /// `for (init; test; update)`
    For {
        /// Scope of the loop head.
        scope: u32,
        /// Initializer.
        init: Option<ForInit>,
        /// Condition.
        test: Option<Expr>,
        /// Update.
        update: Option<Expr>,
        /// Body.
        body: Box<Stmt>,
    },
    /// `for (x of array)`
    ForOf {
        /// Scope of the loop head.
        scope: u32,
        /// Declaration keyword, or `None` for an existing variable.
        decl: Option<VarKind>,
        /// The name.
        target: Name,
        /// The array.
        iter: Expr,
        /// Body.
        body: Box<Stmt>,
    },
    /// `return`
    Return(Option<Expr>),
    /// `break`
    Break(Option<Rc<str>>),
    /// `continue`
    Continue(Option<Rc<str>>),
    /// `throw`
    Throw(Expr),
    /// `try`
    Try(Block, Option<Catch>, Option<Block>),
    /// `switch`
    Switch {
        /// Discriminant.
        disc: Expr,
        /// Scope of the case block.
        scope: u32,
        /// Clauses.
        cases: Vec<Case>,
    },
    /// `label: body`
    Labeled(Rc<str>, Box<Stmt>),
    /// `;`
    Empty,
    /// `import { a, b } from "./x.js";`
    Import(Vec<ImportName>, String),
    /// `export` before a declaration.
    Export(Box<Stmt>),
    /// Placeholder after a subset error.
    Invalid,
}

/// Function flavours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuncKind {
    /// Declaration or expression.
    Normal,
    /// Arrow function.
    Arrow,
    /// Class method (static or not).
    Method,
    /// Class constructor.
    Ctor {
        /// The class has `extends`.
        derived: bool,
    },
}

/// A function.
#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    /// Name, if written.
    pub name: Option<Name>,
    /// Plain parameter names.
    pub params: Vec<Name>,
    /// Body. An arrow with an expression body has one `return`.
    pub body: Vec<Stmt>,
    /// Flavour.
    pub kind: FuncKind,
    /// The function's own scope.
    pub scope: u32,
    /// For a named function expression, the scope holding its name.
    pub name_scope: Option<u32>,
    /// Declared in the special bindings `this`, `%fn`, `%nt` (not arrows).
    pub specials: Option<[Name; 3]>,
    /// Where it is.
    pub pos: Pos,
}

/// A class method.
#[derive(Clone, Debug, PartialEq)]
pub struct Method {
    /// Name.
    pub key: PropName,
    /// `static`
    pub is_static: bool,
    /// The function.
    pub func: Rc<Func>,
}

/// A class.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    /// Name, if written.
    pub name: Option<Name>,
    /// Scope holding the inner name binding.
    pub scope: u32,
    /// `extends` expression.
    pub extends: Option<Box<Expr>>,
    /// Constructor, if written.
    pub ctor: Option<Rc<Func>>,
    /// Methods in order.
    pub methods: Vec<Method>,
    /// Where it is.
    pub pos: Pos,
}

/// Prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `-`
    Neg,
    /// `+`
    Plus,
    /// `!`
    Not,
    /// `~`
    BitNot,
    /// `typeof`
    Typeof,
}

/// Binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `===`
    StrictEq,
    /// `!==`
    StrictNe,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `**`
    Exp,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `>>>`
    UShr,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `in`
    In,
    /// `instanceof`
    Instanceof,
}

impl BinOp {
    /// Source text.
    pub fn text(self) -> &'static str {
        use BinOp::*;
        match self {
            StrictEq => "===",
            StrictNe => "!==",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Mod => "%",
            Exp => "**",
            Shl => "<<",
            Shr => ">>",
            UShr => ">>>",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            In => "in",
            Instanceof => "instanceof",
        }
    }
}

/// An object literal entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    /// Key.
    pub key: PropName,
    /// Value. For shorthand `{ a }` this is the reference to `a`.
    pub value: Expr,
    /// Written as `{ a }`.
    pub shorthand: bool,
}

/// The three special bindings `super` needs: `this`, the running function
/// and `new.target`.
#[derive(Clone, Debug, PartialEq)]
pub struct SuperRefs {
    /// `this`
    pub this: Name,
    /// `%fn`, the running method or constructor.
    pub func: Name,
    /// `%nt`, the `new.target` of the running constructor.
    pub nt: Name,
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub pos: Pos,
}

/// Expression kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Number literal.
    Num(f64),
    /// String literal.
    Str(JStr),
    /// Untagged template: pieces (one more than expressions) and expressions.
    Template(Vec<JStr>, Vec<Expr>),
    /// `true` / `false`
    Bool(bool),
    /// `null`
    Null,
    /// Name reference.
    Ident(Name),
    /// `this`, resolved like a name.
    This(Name),
    /// Array literal.
    Array(Vec<Expr>),
    /// Object literal.
    Object(Vec<Prop>),
    /// Function or arrow expression.
    Function(Rc<Func>),
    /// Class expression.
    Class(Rc<Class>),
    /// Prefix operator.
    Unary(UnOp, Box<Expr>),
    /// `++` / `--`.
    Update {
        /// `++`
        inc: bool,
        /// Prefix form.
        prefix: bool,
        /// Target.
        target: Box<Expr>,
    },
    /// Binary operator.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `&&` (true) or `||` (false).
    Logical(bool, Box<Expr>, Box<Expr>),
    /// `=` (op `None`) or compound assignment.
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    /// `a ? b : c`
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Call.
    Call(Box<Expr>, Vec<Expr>),
    /// `new`
    New(Box<Expr>, Vec<Expr>),
    /// `obj.name`
    Member(Box<Expr>, PropName),
    /// `obj[index]`
    Index(Box<Expr>, Box<Expr>),
    /// `super(args)`
    SuperCall(Box<SuperRefs>, Vec<Expr>),
    /// `super.name`
    SuperMember(Box<SuperRefs>, PropName),
    /// `a, b`
    Seq(Vec<Expr>),
    /// Placeholder after a subset error.
    Invalid,
}

/// A parsed file.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Top-level statements.
    pub body: Vec<Stmt>,
    /// Top-level scope number (always 0).
    pub scope: u32,
    /// The top-level `this` binding.
    pub this: Name,
    /// Number of name ids handed out.
    pub n_names: u32,
    /// Number of scope numbers handed out.
    pub n_scopes: u32,
    /// Parsed as a module (imports and exports allowed).
    pub is_module: bool,
}
