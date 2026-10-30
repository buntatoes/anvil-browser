// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! CSS Syntax Level 3 parser and the typed rule tree.
//!
//! Written from section 5 of CSS Syntax Level 3 ("Parsing"), including the
//! nesting-aware algorithms: "consume a block's contents" mixes declarations
//! and rules, declarations that follow a nested rule become a
//! [`Rule::NestedDeclarations`], and a declaration attempt that fails is
//! retried as a nested qualified rule.
//!
//! Error recovery follows the specification. A bad declaration is skipped to
//! the next `;`. A rule whose selector is invalid is dropped as a whole,
//! block included, and the next rule is read normally. Strings, comments,
//! and blocks are whole tokens or matched pairs, so a `}` inside a string
//! never ends a rule. Every drop is recorded as a diagnostic.
//!
//! Blocks are found with an explicit stack, not recursion. Component values
//! and nested rules recurse, but every level checks [`Limits::max_depth`].
//! A top-level rule that nests deeper than that is dropped as a whole with a
//! [`DiagnosticKind::LimitReached`] diagnostic, and a declaration that nests
//! too deep is dropped on its own the same way.

use crate::prism::error::{Diagnostic, DiagnosticKind, Error, LimitError, LimitKind, Limits, Span};
use crate::prism::media::{
    self, ConditionError, ContainerCondition, MediaQueryList, SupportsCondition,
    parse_container_prelude, parse_media_query_list, parse_media_query_list_checked,
    parse_supports_condition,
};
use crate::prism::selectors::{SelectorList, SelectorOptions, parse_selector_list_from};
use crate::prism::tokenizer::{SpannedToken, Token, Tokenizer, tokenize_with};

/// Parse options. Build one with `Options::default()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Caps on input size, tokens, nesting, and rules.
    pub limits: Limits,
}

/// Which bracket a [`SimpleBlock`] uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// `( ... )`
    Paren,
    /// `[ ... ]`
    Bracket,
    /// `{ ... }`
    Brace,
}

/// A function: a name, `(`, arguments, `)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// Name as written (escapes resolved, case kept).
    pub name: String,
    /// Arguments as component values, whitespace and commas included.
    pub arguments: Vec<ComponentValue>,
    /// Source bytes from the name through `)`.
    pub span: Span,
}

/// A simple block: an opening bracket, contents, the matching closer.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleBlock {
    /// Which bracket.
    pub kind: BlockKind,
    /// The contents.
    pub contents: Vec<ComponentValue>,
    /// Source bytes from the opener through the closer.
    pub span: Span,
}

/// A component value: a preserved token, a function, or a simple block.
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    /// Any token other than an opener or function token.
    Token(SpannedToken),
    /// A function and its arguments.
    Function(Function),
    /// A bracketed block.
    Block(SimpleBlock),
}

impl Drop for ComponentValue {
    fn drop(&mut self) {
        let mut stack: Vec<Vec<ComponentValue>> = Vec::new();
        match self {
            ComponentValue::Function(f) if !f.arguments.is_empty() => {
                stack.push(std::mem::take(&mut f.arguments));
            }
            ComponentValue::Block(b) if !b.contents.is_empty() => {
                stack.push(std::mem::take(&mut b.contents));
            }
            _ => return,
        }
        while let Some(list) = stack.pop() {
            for mut item in list {
                match &mut item {
                    ComponentValue::Function(f) if !f.arguments.is_empty() => {
                        stack.push(std::mem::take(&mut f.arguments));
                    }
                    ComponentValue::Block(b) if !b.contents.is_empty() => {
                        stack.push(std::mem::take(&mut b.contents));
                    }
                    _ => {}
                }
            }
        }
    }
}

impl ComponentValue {
    /// Source bytes.
    pub fn span(&self) -> Span {
        match self {
            ComponentValue::Token(t) => t.span,
            ComponentValue::Function(f) => f.span,
            ComponentValue::Block(b) => b.span,
        }
    }

    /// The token, when this is a preserved token.
    pub fn token(&self) -> Option<&Token> {
        match self {
            ComponentValue::Token(t) => Some(&t.token),
            _ => None,
        }
    }

    /// True for a whitespace token.
    pub fn is_whitespace(&self) -> bool {
        matches!(self.token(), Some(Token::Whitespace))
    }

    /// True when this is an ident equal to `name`, ignoring ASCII case.
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(self.token(), Some(t) if t.is_ident(name))
    }

    /// True when this is the delimiter `c`.
    pub fn is_delim(&self, c: char) -> bool {
        matches!(self.token(), Some(t) if t.is_delim(c))
    }
}

/// A declaration: `name: value [!important]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// Property name. ASCII-lowercased, except custom properties (`--x`),
    /// which keep their case.
    pub name: String,
    /// The value, without leading and trailing whitespace and without
    /// `!important`.
    pub value: Vec<ComponentValue>,
    /// True when the declaration ended in `!important`.
    pub important: bool,
    /// Source bytes from the name through the end of the value.
    pub span: Span,
}

impl Declaration {
    /// True for a custom property (`--name`).
    pub fn is_custom(&self) -> bool {
        self.name.starts_with("--")
    }

    /// Parse the value with [`crate::prism::values::parse_value_from`].
    pub fn parse_value(
        &self,
    ) -> Result<crate::prism::values::PropertyValue, crate::prism::values::ValueError> {
        crate::prism::values::parse_value_from(&self.name, &self.value, &Limits::default())
    }
}

/// A style rule: selectors and a block.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The parsed selectors. Inside another style rule they may contain `&`
    /// or start with a combinator; see [`SelectorList::resolve_nesting`].
    pub selectors: SelectorList,
    /// Declarations before the first nested rule.
    pub declarations: Vec<Declaration>,
    /// Nested rules, in order. Declarations that follow a nested rule are
    /// kept here as [`Rule::NestedDeclarations`].
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// `@media`.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The parsed query list.
    pub queries: MediaQueryList,
    /// Child rules.
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// `@supports`.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportsRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The parsed condition.
    pub condition: SupportsCondition,
    /// Child rules.
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// The `layer` part of `@import`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportLayer {
    /// `layer` with no name.
    Anonymous,
    /// `layer(name)`.
    Named(LayerName),
}

/// `@import`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The URL, from `url(...)` or a string.
    pub url: String,
    /// `layer` or `layer(name)`, when present.
    pub layer: Option<ImportLayer>,
    /// `supports(...)`, when present.
    pub supports: Option<SupportsCondition>,
    /// The media query list. Empty means all media.
    pub media: MediaQueryList,
    /// Source bytes.
    pub span: Span,
}

/// `@font-face`.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFaceRule {
    /// The descriptors.
    pub declarations: Vec<Declaration>,
    /// Source bytes.
    pub span: Span,
}

/// One keyframe inside `@keyframes`.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// The keyframe selectors as percentages: `from` is 0, `to` is 100.
    pub selectors: Vec<f64>,
    /// The declarations.
    pub declarations: Vec<Declaration>,
    /// Source bytes.
    pub span: Span,
}

/// `@keyframes`.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframesRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The animation name.
    pub name: String,
    /// The keyframes, in source order.
    pub keyframes: Vec<Keyframe>,
    /// Source bytes.
    pub span: Span,
}

/// A layer name: `a.b.c` is `["a", "b", "c"]`.
pub type LayerName = Vec<String>;

/// `@layer a, b;`
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStatementRule {
    /// The declared names.
    pub names: Vec<LayerName>,
    /// Source bytes.
    pub span: Span,
}

/// `@layer [name] { ... }`
#[derive(Clone, Debug, PartialEq)]
pub struct LayerBlockRule {
    /// The name, or `None` for an anonymous layer.
    pub name: Option<LayerName>,
    /// Child rules.
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// `@container`.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The container name, when present.
    pub name: Option<String>,
    /// The size condition, when present.
    pub condition: Option<ContainerCondition>,
    /// Child rules.
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// `@namespace`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceRule {
    /// The prefix, or `None` for the default namespace.
    pub prefix: Option<String>,
    /// The namespace URL.
    pub url: String,
    /// Source bytes.
    pub span: Span,
}

/// One page selector: `name:first:left`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSelector {
    /// The page type name, when present.
    pub name: Option<String>,
    /// Page pseudo-classes, lowercased: `left`, `right`, `first`, `blank`.
    pub pseudo_classes: Vec<String>,
}

/// A margin box rule inside `@page`, such as `@top-left`.
#[derive(Clone, Debug, PartialEq)]
pub struct PageMarginRule {
    /// The margin box name, lowercased.
    pub name: String,
    /// The declarations.
    pub declarations: Vec<Declaration>,
    /// Source bytes.
    pub span: Span,
}

/// `@page`.
#[derive(Clone, Debug, PartialEq)]
pub struct PageRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The page selectors. Empty matches every page.
    pub selectors: Vec<PageSelector>,
    /// The declarations.
    pub declarations: Vec<Declaration>,
    /// Margin box rules.
    pub margin_rules: Vec<PageMarginRule>,
    /// Source bytes.
    pub span: Span,
}

/// `@property --name { syntax: "..."; inherits: true|false; initial-value: ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyRule {
    /// The custom property name (`--name`).
    pub name: String,
    /// The `syntax` descriptor string, when present.
    pub syntax: Option<String>,
    /// The `inherits` descriptor boolean, when present.
    pub inherits: Option<bool>,
    /// The `initial-value` descriptor tokens, when present.
    pub initial_value: Option<Vec<ComponentValue>>,
    /// All descriptors in source order.
    pub declarations: Vec<Declaration>,
    /// Source bytes.
    pub span: Span,
}

/// `@scope [(<scope-start>)] [to (<scope-end>)] { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeRule {
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The `<scope-start>` selector list, when present.
    pub start: Option<SelectorList>,
    /// The `<scope-end>` selector list, when present.
    pub end: Option<SelectorList>,
    /// Child rules.
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// `@starting-style { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct StartingStyleRule {
    /// Child rules (or nested declarations when inside a style rule).
    pub rules: Vec<Rule>,
    /// Source bytes.
    pub span: Span,
}

/// Declarations that follow a nested rule inside a style rule or a nested
/// group rule. They apply with the parent's selectors.
#[derive(Clone, Debug, PartialEq)]
pub struct NestedDeclarationsRule {
    /// The declarations.
    pub declarations: Vec<Declaration>,
    /// Source bytes.
    pub span: Span,
}

/// An at-rule this crate does not interpret. It is kept whole so that a
/// consumer can handle it.
#[derive(Clone, Debug, PartialEq)]
pub struct UnknownAtRule {
    /// Name without `@`, lowercased.
    pub name: String,
    /// The prelude as component values.
    pub prelude: Vec<ComponentValue>,
    /// The block contents, or `None` for a statement.
    pub block: Option<Vec<ComponentValue>>,
    /// Source bytes.
    pub span: Span,
}

/// Any rule.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// A style rule.
    Style(StyleRule),
    /// `@media`.
    Media(MediaRule),
    /// `@supports`.
    Supports(SupportsRule),
    /// `@import`.
    Import(ImportRule),
    /// `@font-face`.
    FontFace(FontFaceRule),
    /// `@keyframes`.
    Keyframes(KeyframesRule),
    /// `@layer` statement.
    LayerStatement(LayerStatementRule),
    /// `@layer` block.
    LayerBlock(LayerBlockRule),
    /// `@container`.
    Container(ContainerRule),
    /// `@namespace`.
    Namespace(NamespaceRule),
    /// `@page`.
    Page(PageRule),
    /// `@property`.
    Property(PropertyRule),
    /// `@scope`.
    Scope(ScopeRule),
    /// `@starting-style`.
    StartingStyle(StartingStyleRule),
    /// Declarations after a nested rule.
    NestedDeclarations(NestedDeclarationsRule),
    /// Any other at-rule.
    Unknown(UnknownAtRule),
}

impl Rule {
    /// Source bytes.
    pub fn span(&self) -> Span {
        match self {
            Rule::Style(r) => r.span,
            Rule::Media(r) => r.span,
            Rule::Supports(r) => r.span,
            Rule::Import(r) => r.span,
            Rule::FontFace(r) => r.span,
            Rule::Keyframes(r) => r.span,
            Rule::LayerStatement(r) => r.span,
            Rule::LayerBlock(r) => r.span,
            Rule::Container(r) => r.span,
            Rule::Namespace(r) => r.span,
            Rule::Page(r) => r.span,
            Rule::Property(r) => r.span,
            Rule::Scope(r) => r.span,
            Rule::StartingStyle(r) => r.span,
            Rule::NestedDeclarations(r) => r.span,
            Rule::Unknown(r) => r.span,
        }
    }

    /// Child rules of a grouping rule, or an empty slice.
    pub fn children(&self) -> &[Rule] {
        match self {
            Rule::Style(r) => &r.rules,
            Rule::Media(r) => &r.rules,
            Rule::Supports(r) => &r.rules,
            Rule::LayerBlock(r) => &r.rules,
            Rule::Container(r) => &r.rules,
            Rule::Scope(r) => &r.rules,
            Rule::StartingStyle(r) => &r.rules,
            _ => &[],
        }
    }
}

/// A parsed style sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    /// Top-level rules in source order.
    pub rules: Vec<Rule>,
    /// Every recovered problem, in the order it was found.
    pub diagnostics: Vec<Diagnostic>,
}

impl Stylesheet {
    /// Count of rules, nested rules included.
    pub fn rule_count(&self) -> usize {
        fn walk(rules: &[Rule]) -> usize {
            // Iterative walk: rule nesting is already capped, but a work list
            // keeps this independent of the cap.
            let mut stack: Vec<&[Rule]> = vec![rules];
            let mut n = 0;
            while let Some(list) = stack.pop() {
                for r in list {
                    n += 1;
                    if let Rule::Keyframes(k) = r {
                        n += k.keyframes.len();
                    }
                    if let Rule::Page(p) = r {
                        n += p.margin_rules.len();
                    }
                    stack.push(r.children());
                }
            }
            n
        }
        walk(&self.rules)
    }
}

/// A list of declarations, as from a `style` attribute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclarationBlock {
    /// The declarations in source order. Later ones override earlier ones
    /// in the cascade; all are kept here.
    pub declarations: Vec<Declaration>,
    /// Every recovered problem.
    pub diagnostics: Vec<Diagnostic>,
}

fn run_with_stack<T: Send>(tokens_len: usize, f: impl FnOnce() -> T + Send) -> T {
    if tokens_len > 64 {
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(64 * 1024 * 1024)
                .spawn_scoped(s, f)
                .expect("failed to spawn CSS parser stack thread")
                .join()
                .expect("CSS parser stack thread panicked")
        })
    } else {
        f()
    }
}

/// Parse a whole style sheet.
pub fn parse_stylesheet(src: &str, options: &Options) -> Result<Stylesheet, Error> {
    let list = tokenize_with(
        Tokenizer::new(src),
        src.len(),
        options.limits.max_input_bytes,
        options.limits.max_tokens,
    )?;
    run_with_stack(list.tokens.len(), || {
        let mut p = Parser::new(&list.tokens, &options.limits);
        p.src = src;
        p.diagnostics = list.diagnostics;
        let rules = p.stylesheet()?;
        Ok(Stylesheet {
            rules,
            diagnostics: p.diagnostics,
        })
    })
}

/// Parse a declaration list, as in a `style` attribute, with default limits.
pub fn parse_declarations(src: &str) -> Result<DeclarationBlock, Error> {
    parse_declarations_with(src, &Options::default())
}

/// Parse a declaration list with the given options.
pub fn parse_declarations_with(src: &str, options: &Options) -> Result<DeclarationBlock, Error> {
    let list = tokenize_with(
        Tokenizer::new(src),
        src.len(),
        options.limits.max_input_bytes,
        options.limits.max_tokens,
    )?;
    run_with_stack(list.tokens.len(), || {
        let mut p = Parser::new(&list.tokens, &options.limits);
        p.diagnostics = list.diagnostics;
        let range = Range {
            start: 0,
            end: list.tokens.len(),
        };
        let declarations = p.declaration_list(range)?;
        Ok(DeclarationBlock {
            declarations,
            diagnostics: p.diagnostics,
        })
    })
}

/// Parse a list of component values, as for a property value.
pub fn parse_component_values(src: &str, options: &Options) -> Result<Vec<ComponentValue>, Error> {
    parse_component_values_tokens(Tokenizer::new(src), src.len(), options)
}

pub(crate) fn parse_component_values_tokens(
    tokenizer: Tokenizer<'_>,
    len: usize,
    options: &Options,
) -> Result<Vec<ComponentValue>, Error> {
    let list = tokenize_with(
        tokenizer,
        len,
        options.limits.max_input_bytes,
        options.limits.max_tokens,
    )?;
    run_with_stack(list.tokens.len(), || {
        let mut p = Parser::new(&list.tokens, &options.limits);
        let mut out = Vec::new();
        while p.peek().is_some() {
            out.push(p.component_value());
        }
        if p.overflow {
            return Err(Error::Limit(LimitError {
                kind: LimitKind::Depth,
                limit: options.limits.max_depth,
                found: options.limits.max_depth + 1,
                span: Span::new(0, len),
            }));
        }
        Ok(out)
    })
}

#[derive(Clone, Copy, Debug)]
struct Range {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctx {
    /// Directly inside the style sheet.
    Stylesheet,
    /// Inside a top-level grouping rule such as `@media`.
    Group,
    /// Inside a style rule, or a grouping rule inside one.
    Nested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Imports,
    Namespaces,
    Body,
}

enum Item {
    Decl(Declaration),
    Rule(Rule),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Closer {
    Paren,
    Bracket,
    Brace,
}

fn opener(t: &Token) -> Option<Closer> {
    match t {
        Token::LeftParen | Token::Function(_) => Some(Closer::Paren),
        Token::LeftBracket => Some(Closer::Bracket),
        Token::LeftBrace => Some(Closer::Brace),
        _ => None,
    }
}

fn closes(t: &Token, c: Closer) -> bool {
    matches!(
        (t, c),
        (Token::RightParen, Closer::Paren)
            | (Token::RightBracket, Closer::Bracket)
            | (Token::RightBrace, Closer::Brace)
    )
}

struct Parser<'a> {
    src: &'a str,
    tokens: &'a [SpannedToken],
    pos: usize,
    end: usize,
    depth: usize,
    limits: &'a Limits,
    diagnostics: Vec<Diagnostic>,
    rules: usize,
    overflow: bool,
    phase: Phase,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [SpannedToken], limits: &'a Limits) -> Self {
        Parser {
            src: "",
            tokens,
            pos: 0,
            end: tokens.len(),
            depth: 0,
            limits,
            diagnostics: Vec::new(),
            rules: 0,
            overflow: false,
            phase: Phase::Imports,
        }
    }

    fn peek(&self) -> Option<&'a Token> {
        if self.pos < self.end {
            self.tokens.get(self.pos).map(|t| &t.token)
        } else {
            None
        }
    }

    fn span_at(&self, i: usize) -> Span {
        match self.tokens.get(i) {
            Some(t) => t.span,
            None => {
                let end = self.tokens.last().map(|t| t.span.end).unwrap_or(0);
                Span::new(end, end)
            }
        }
    }

    /// Span from token `start` through the last consumed token.
    fn span_from(&self, start: usize) -> Span {
        let first = self.span_at(start);
        if self.pos > start {
            first.to(self.span_at(self.pos - 1))
        } else {
            Span::new(first.start, first.start)
        }
    }

    fn diag(&mut self, kind: DiagnosticKind, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::new(kind, message, span));
    }

    fn count_rule(&mut self, span: Span) -> Result<(), LimitError> {
        self.rules += 1;
        if self.rules > self.limits.max_rules {
            return Err(LimitError {
                kind: LimitKind::Rules,
                limit: self.limits.max_rules,
                found: self.rules,
                span,
            });
        }
        Ok(())
    }

    /// Skip from just after an opener to just after its closer, iteratively.
    /// Returns the index of the closer, or `None` at the end of the range.
    fn skip_to_closer(&mut self, closer: Closer) -> Option<usize> {
        let mut stack = vec![closer];
        while self.pos < self.end {
            let t = &self.tokens[self.pos].token;
            let i = self.pos;
            self.pos += 1;
            if let Some(top) = stack.last()
                && closes(t, *top)
            {
                stack.pop();
                if stack.is_empty() {
                    return Some(i);
                }
                continue;
            }
            if let Some(c) = opener(t) {
                stack.push(c);
            }
        }
        None
    }

    fn component_value(&mut self) -> ComponentValue {
        let start = self.pos;
        let t = &self.tokens[self.pos];
        self.pos += 1;
        let (closer, kind, name) = match &t.token {
            Token::LeftParen => (Closer::Paren, Some(BlockKind::Paren), None),
            Token::LeftBracket => (Closer::Bracket, Some(BlockKind::Bracket), None),
            Token::LeftBrace => (Closer::Brace, Some(BlockKind::Brace), None),
            Token::Function(n) => (Closer::Paren, None, Some(n.clone())),
            _ => return ComponentValue::Token(t.clone()),
        };
        let mut contents = Vec::new();
        if self.depth >= self.limits.max_depth {
            self.overflow = true;
            if self.skip_to_closer(closer).is_none() {
                self.diag(
                    DiagnosticKind::UnexpectedEof,
                    "block or function not closed",
                    self.span_from(start),
                );
            }
        } else {
            self.depth += 1;
            loop {
                match self.peek() {
                    None => {
                        self.diag(
                            DiagnosticKind::UnexpectedEof,
                            "block or function not closed",
                            self.span_from(start),
                        );
                        break;
                    }
                    Some(tok) if closes(tok, closer) => {
                        self.pos += 1;
                        break;
                    }
                    Some(_) => contents.push(self.component_value()),
                }
            }
            self.depth -= 1;
        }
        let span = self.span_from(start);
        match (kind, name) {
            (Some(kind), _) => ComponentValue::Block(SimpleBlock {
                kind,
                contents,
                span,
            }),
            (None, name) => ComponentValue::Function(Function {
                name: name.unwrap_or_default(),
                arguments: contents,
                span,
            }),
        }
    }

    /// At a `{`: consume through the matching `}` and return the inside.
    fn block_range(&mut self) -> Range {
        let open = self.pos;
        self.pos += 1;
        match self.skip_to_closer(Closer::Brace) {
            Some(close) => Range {
                start: open + 1,
                end: close,
            },
            None => {
                self.diag(
                    DiagnosticKind::UnexpectedEof,
                    "block not closed before end of input",
                    self.span_from(open),
                );
                Range {
                    start: open + 1,
                    end: self.pos,
                }
            }
        }
    }

    /// Run `f` over the tokens of `range`, one nesting level deeper.
    fn in_range<T>(
        &mut self,
        range: Range,
        empty: T,
        f: impl FnOnce(&mut Self) -> Result<T, LimitError>,
    ) -> Result<T, LimitError> {
        if self.depth >= self.limits.max_depth {
            self.overflow = true;
            return Ok(empty);
        }
        let saved = (self.pos, self.end);
        self.pos = range.start;
        self.end = range.end;
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        self.pos = saved.0;
        self.end = saved.1;
        out
    }

    fn values_in(&mut self, range: Range) -> Vec<ComponentValue> {
        let r = self.in_range(range, Vec::new(), |p| {
            let mut out = Vec::new();
            while p.peek().is_some() {
                out.push(p.component_value());
            }
            Ok(out)
        });
        r.unwrap_or_default()
    }

    // ---- style sheet level ----

    fn stylesheet(&mut self) -> Result<Vec<Rule>, LimitError> {
        let mut rules = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some(Token::Whitespace | Token::Cdo | Token::Cdc) => {
                    self.pos += 1;
                }
                Some(t) => {
                    let start = self.pos;
                    self.overflow = false;
                    let rule = if matches!(t, Token::AtKeyword(_)) {
                        self.at_rule(Ctx::Stylesheet)?
                    } else {
                        self.qualified_rule(Ctx::Stylesheet, false)?
                    };
                    if self.overflow {
                        self.overflow = false;
                        let span = self.span_from(start);
                        self.diag(
                            DiagnosticKind::LimitReached(LimitKind::Depth),
                            format!(
                                "rule nests deeper than {} levels and was dropped",
                                self.limits.max_depth
                            ),
                            span,
                        );
                        continue;
                    }
                    if let Some(rule) = rule {
                        self.advance_phase(&rule);
                        rules.push(rule);
                    }
                }
            }
        }
        Ok(rules)
    }

    fn advance_phase(&mut self, rule: &Rule) {
        match rule {
            Rule::Import(_) | Rule::LayerStatement(_) => {}
            Rule::Namespace(_) => self.phase = self.phase.max(Phase::Namespaces),
            Rule::Unknown(u) if u.name == "charset" => {}
            _ => self.phase = Phase::Body,
        }
    }

    // ---- rules ----

    fn at_rule(&mut self, ctx: Ctx) -> Result<Option<Rule>, LimitError> {
        let start = self.pos;
        let name = match self.peek() {
            Some(Token::AtKeyword(n)) => n.to_ascii_lowercase(),
            _ => return Ok(None),
        };
        self.pos += 1;
        let mut prelude = Vec::new();
        let mut block = None;
        loop {
            match self.peek() {
                None => break,
                Some(Token::Semicolon) => {
                    self.pos += 1;
                    break;
                }
                Some(Token::LeftBrace) => {
                    block = Some(self.block_range());
                    break;
                }
                Some(Token::RightBrace) if ctx == Ctx::Nested => break,
                Some(_) => prelude.push(self.component_value()),
            }
        }
        let span = self.span_from(start);
        self.typed_at_rule(name, prelude, block, span, ctx)
    }

    fn condition_diag(&mut self, what: &str, e: ConditionError) {
        let kind = match e.limit {
            Some(k) => DiagnosticKind::LimitReached(k),
            None => DiagnosticKind::InvalidAtRule,
        };
        self.diag(kind, format!("{what}: {}", e.message), e.span);
    }

    fn invalid_at(
        &mut self,
        name: &str,
        why: &str,
        span: Span,
    ) -> Result<Option<Rule>, LimitError> {
        self.diag(
            DiagnosticKind::InvalidAtRule,
            format!("@{name} dropped: {why}"),
            span,
        );
        Ok(None)
    }

    fn group_rules(&mut self, block: Range, ctx: Ctx) -> Result<Vec<Rule>, LimitError> {
        let inner = if ctx == Ctx::Nested {
            Ctx::Nested
        } else {
            Ctx::Group
        };
        let items = self.in_range(block, Vec::new(), |p| p.block_contents(inner))?;
        let mut rules = Vec::new();
        let mut run: Vec<Declaration> = Vec::new();
        for item in items {
            match item {
                Item::Decl(d) => {
                    if inner == Ctx::Nested {
                        run.push(d);
                    } else {
                        self.diag(
                            DiagnosticKind::InvalidDeclaration,
                            format!("declaration '{}' is not allowed here", d.name),
                            d.span,
                        );
                    }
                }
                Item::Rule(r) => {
                    self.flush_nested(&mut run, &mut rules)?;
                    rules.push(r);
                }
            }
        }
        self.flush_nested(&mut run, &mut rules)?;
        Ok(rules)
    }

    fn flush_nested(
        &mut self,
        run: &mut Vec<Declaration>,
        rules: &mut Vec<Rule>,
    ) -> Result<(), LimitError> {
        if run.is_empty() {
            return Ok(());
        }
        let declarations = std::mem::take(run);
        let span = declarations[0]
            .span
            .to(declarations[declarations.len() - 1].span);
        self.count_rule(span)?;
        rules.push(Rule::NestedDeclarations(NestedDeclarationsRule {
            declarations,
            span,
        }));
        Ok(())
    }

    fn typed_at_rule(
        &mut self,
        name: String,
        prelude: Vec<ComponentValue>,
        block: Option<Range>,
        span: Span,
        ctx: Ctx,
    ) -> Result<Option<Rule>, LimitError> {
        let limits = self.limits;
        match name.as_str() {
            "media" => {
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                self.count_rule(span)?;
                let (queries, errors) = parse_media_query_list_checked(&prelude, limits);
                for e in errors {
                    self.condition_diag("media query treated as 'not all'", e);
                }
                let rules = self.group_rules(block, ctx)?;
                Ok(Some(Rule::Media(MediaRule {
                    prelude,
                    queries,
                    rules,
                    span,
                })))
            }
            "supports" => {
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let condition = match parse_supports_condition(&prelude, limits) {
                    Ok(c) => c,
                    Err(e) => {
                        self.condition_diag("@supports dropped", e);
                        return Ok(None);
                    }
                };
                self.count_rule(span)?;
                let rules = self.group_rules(block, ctx)?;
                Ok(Some(Rule::Supports(SupportsRule {
                    prelude,
                    condition,
                    rules,
                    span,
                })))
            }
            "container" => {
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let (cname, condition) = match parse_container_prelude(&prelude, limits) {
                    Ok(v) => v,
                    Err(e) => {
                        self.condition_diag("@container dropped", e);
                        return Ok(None);
                    }
                };
                self.count_rule(span)?;
                let rules = self.group_rules(block, ctx)?;
                Ok(Some(Rule::Container(ContainerRule {
                    prelude,
                    name: cname,
                    condition,
                    rules,
                    span,
                })))
            }
            "layer" => {
                let names = match parse_layer_names(&prelude) {
                    Some(n) => n,
                    None => return self.invalid_at(&name, "invalid layer name", span),
                };
                match block {
                    Some(block) => {
                        if names.len() > 1 {
                            return self.invalid_at(&name, "a layer block takes one name", span);
                        }
                        self.count_rule(span)?;
                        let rules = self.group_rules(block, ctx)?;
                        Ok(Some(Rule::LayerBlock(LayerBlockRule {
                            name: names.into_iter().next(),
                            rules,
                            span,
                        })))
                    }
                    None => {
                        if names.is_empty() {
                            return self.invalid_at(&name, "a layer statement needs a name", span);
                        }
                        self.count_rule(span)?;
                        Ok(Some(Rule::LayerStatement(LayerStatementRule {
                            names,
                            span,
                        })))
                    }
                }
            }
            "import" => {
                if ctx != Ctx::Stylesheet || self.phase > Phase::Imports {
                    return self.invalid_at(&name, "@import must come before other rules", span);
                }
                if block.is_some() {
                    return self.invalid_at(&name, "@import takes no block", span);
                }
                match parse_import_prelude(&prelude, limits) {
                    Some((url, layer, supports, media)) => {
                        self.count_rule(span)?;
                        Ok(Some(Rule::Import(ImportRule {
                            prelude,
                            url,
                            layer,
                            supports,
                            media,
                            span,
                        })))
                    }
                    None => self.invalid_at(&name, "invalid prelude", span),
                }
            }
            "namespace" => {
                if ctx != Ctx::Stylesheet || self.phase > Phase::Namespaces {
                    return self.invalid_at(
                        &name,
                        "@namespace must come after @import and before other rules",
                        span,
                    );
                }
                if block.is_some() {
                    return self.invalid_at(&name, "@namespace takes no block", span);
                }
                match parse_namespace_prelude(&prelude) {
                    Some((prefix, url)) => {
                        self.count_rule(span)?;
                        Ok(Some(Rule::Namespace(NamespaceRule { prefix, url, span })))
                    }
                    None => self.invalid_at(&name, "invalid prelude", span),
                }
            }
            "font-face" => {
                if ctx == Ctx::Nested {
                    return self.invalid_at(&name, "not allowed inside a style rule", span);
                }
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                if prelude.iter().any(|v| !v.is_whitespace()) {
                    return self.invalid_at(&name, "@font-face takes no prelude", span);
                }
                self.count_rule(span)?;
                let mut declarations = self.declaration_list(block)?;
                for d in &mut declarations {
                    if d.name == "unicode-range" {
                        self.retokenize_unicode_range(d);
                    }
                }
                Ok(Some(Rule::FontFace(FontFaceRule { declarations, span })))
            }
            "keyframes" => {
                if ctx == Ctx::Nested {
                    return self.invalid_at(&name, "not allowed inside a style rule", span);
                }
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let Some(kname) = parse_keyframes_name(&prelude) else {
                    return self.invalid_at(&name, "invalid animation name", span);
                };
                self.count_rule(span)?;
                let keyframes = self.keyframe_list(block)?;
                Ok(Some(Rule::Keyframes(KeyframesRule {
                    prelude,
                    name: kname,
                    keyframes,
                    span,
                })))
            }
            "page" => {
                if ctx == Ctx::Nested {
                    return self.invalid_at(&name, "not allowed inside a style rule", span);
                }
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let Some(selectors) = parse_page_selectors(&prelude) else {
                    return self.invalid_at(&name, "invalid page selector", span);
                };
                self.count_rule(span)?;
                let (declarations, margin_rules) = self.page_body(block)?;
                Ok(Some(Rule::Page(PageRule {
                    prelude,
                    selectors,
                    declarations,
                    margin_rules,
                    span,
                })))
            }
            "property" => {
                if ctx == Ctx::Nested {
                    return self.invalid_at(&name, "not allowed inside a style rule", span);
                }
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let Some(pname) = parse_property_name(&prelude) else {
                    return self.invalid_at(&name, "invalid custom property name", span);
                };
                self.count_rule(span)?;
                let declarations = self.declaration_list(block)?;
                let mut syntax = None;
                let mut inherits = None;
                let mut initial_value = None;
                for d in &declarations {
                    if d.important {
                        return self.invalid_at(
                            &name,
                            "!important is not allowed in @property",
                            span,
                        );
                    }
                    match d.name.as_str() {
                        "syntax" => {
                            let parts = non_ws(&d.value);
                            if let [only] = parts.as_slice()
                                && let Some(Token::String(s)) = only.token()
                                && !s.trim().is_empty()
                            {
                                syntax = Some(s.clone());
                            } else {
                                return self.invalid_at(&name, "invalid syntax descriptor", span);
                            }
                        }
                        "inherits" => {
                            let parts = non_ws(&d.value);
                            if let [only] = parts.as_slice()
                                && let Some(Token::Ident(s)) = only.token()
                            {
                                if s.eq_ignore_ascii_case("true") {
                                    inherits = Some(true);
                                } else if s.eq_ignore_ascii_case("false") {
                                    inherits = Some(false);
                                } else {
                                    return self.invalid_at(
                                        &name,
                                        "invalid inherits descriptor",
                                        span,
                                    );
                                }
                            } else {
                                return self.invalid_at(&name, "invalid inherits descriptor", span);
                            }
                        }
                        "initial-value" => {
                            initial_value = Some(d.value.clone());
                        }
                        _ => {}
                    }
                }
                Ok(Some(Rule::Property(PropertyRule {
                    name: pname,
                    syntax,
                    inherits,
                    initial_value,
                    declarations,
                    span,
                })))
            }
            "scope" => {
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                let Some((start_sel, end_sel)) = parse_scope_prelude(&prelude, limits) else {
                    return self.invalid_at(&name, "invalid @scope prelude", span);
                };
                self.count_rule(span)?;
                let rules = self.group_rules(block, ctx)?;
                Ok(Some(Rule::Scope(ScopeRule {
                    prelude,
                    start: start_sel,
                    end: end_sel,
                    rules,
                    span,
                })))
            }
            "starting-style" => {
                let Some(block) = block else {
                    return self.invalid_at(&name, "missing block", span);
                };
                if prelude.iter().any(|v| !v.is_whitespace()) {
                    return self.invalid_at(&name, "@starting-style takes no prelude", span);
                }
                self.count_rule(span)?;
                let rules = self.group_rules(block, ctx)?;
                Ok(Some(Rule::StartingStyle(StartingStyleRule { rules, span })))
            }
            _ => {
                self.diag(
                    DiagnosticKind::UnknownAtRule,
                    format!("@{name} is kept as an unknown at-rule"),
                    span,
                );
                self.count_rule(span)?;
                let block = block.map(|b| self.values_in(b));
                Ok(Some(Rule::Unknown(UnknownAtRule {
                    name,
                    prelude,
                    block,
                    span,
                })))
            }
        }
    }

    /// "Consume a qualified rule". `stop_at_semicolon` is the nested form.
    fn qualified_rule(
        &mut self,
        ctx: Ctx,
        stop_at_semicolon: bool,
    ) -> Result<Option<Rule>, LimitError> {
        let start = self.pos;
        let mut prelude = Vec::new();
        let block = loop {
            match self.peek() {
                None => {
                    let span = self.span_from(start);
                    self.diag(
                        DiagnosticKind::InvalidRule,
                        "rule ends before its block starts",
                        span,
                    );
                    return Ok(None);
                }
                Some(Token::Semicolon) if stop_at_semicolon => {
                    let span = self.span_from(start);
                    self.diag(
                        DiagnosticKind::InvalidRule,
                        "text ending in ';' is neither a declaration nor a rule",
                        span,
                    );
                    return Ok(None);
                }
                Some(Token::RightBrace) if ctx == Ctx::Nested => {
                    let span = self.span_from(start);
                    self.diag(DiagnosticKind::InvalidRule, "rule has no block", span);
                    return Ok(None);
                }
                Some(Token::LeftBrace) => break self.block_range(),
                Some(_) => prelude.push(self.component_value()),
            }
        };
        let span = self.span_from(start);
        if starts_like_custom_property(&prelude) {
            self.diag(
                DiagnosticKind::InvalidRule,
                "a rule prelude may not look like a custom property",
                span,
            );
            return Ok(None);
        }
        let options = SelectorOptions {
            nested: ctx == Ctx::Nested,
            max_depth: self.limits.max_selector_depth,
            max_length: self.limits.max_selector_length,
        };
        let selectors = match parse_selector_list_from(&prelude, &options) {
            Ok(s) => s,
            Err(e) => {
                let kind = match e.limit {
                    Some(k) => DiagnosticKind::LimitReached(k),
                    None => DiagnosticKind::InvalidSelector,
                };
                self.diag(kind, format!("rule dropped: {}", e.message), span);
                return Ok(None);
            }
        };
        self.count_rule(span)?;
        let items = self.in_range(block, Vec::new(), |p| p.block_contents(Ctx::Nested))?;
        let mut declarations = Vec::new();
        let mut rules = Vec::new();
        let mut run = Vec::new();
        let mut seen_rule = false;
        for item in items {
            match item {
                Item::Decl(d) if !seen_rule => declarations.push(d),
                Item::Decl(d) => run.push(d),
                Item::Rule(r) => {
                    seen_rule = true;
                    self.flush_nested(&mut run, &mut rules)?;
                    rules.push(r);
                }
            }
        }
        self.flush_nested(&mut run, &mut rules)?;
        Ok(Some(Rule::Style(StyleRule {
            prelude,
            selectors,
            declarations,
            rules,
            span,
        })))
    }

    /// "Consume a block's contents": declarations and rules, mixed.
    fn block_contents(&mut self, ctx: Ctx) -> Result<Vec<Item>, LimitError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => return Ok(items),
                Some(Token::Whitespace | Token::Semicolon) => self.pos += 1,
                Some(Token::AtKeyword(_)) => {
                    if let Some(r) = self.at_rule(ctx)? {
                        items.push(Item::Rule(r));
                    }
                }
                Some(Token::RightBrace) => {
                    // Only reachable in a declaration list parsed on its
                    // own (a style attribute); inside a block the closing
                    // brace is outside the range.
                    let span = self.span_at(self.pos);
                    self.diag(
                        DiagnosticKind::InvalidDeclaration,
                        "stray '}' skipped",
                        span,
                    );
                    self.pos += 1;
                }
                Some(_) => {
                    let mark = self.pos;
                    let outer = self.overflow;
                    self.overflow = false;
                    let decl = self.declaration();
                    if let Some(d) = decl {
                        if self.overflow {
                            self.diag(
                                DiagnosticKind::LimitReached(LimitKind::Depth),
                                format!(
                                    "declaration '{}' nests deeper than {} levels and was dropped",
                                    d.name, self.limits.max_depth
                                ),
                                d.span,
                            );
                        } else {
                            items.push(Item::Decl(d));
                        }
                        self.overflow = outer;
                        continue;
                    }
                    self.overflow = outer;
                    self.pos = mark;
                    let qctx = if ctx == Ctx::Group {
                        Ctx::Group
                    } else {
                        Ctx::Nested
                    };
                    if let Some(r) = self.qualified_rule(qctx, true)? {
                        items.push(Item::Rule(r));
                    }
                }
            }
        }
    }

    /// "Consume a declaration". Returns `None` (with the position somewhere
    /// after the start) when the text is not a declaration; the caller
    /// rewinds and tries a rule.
    fn declaration(&mut self) -> Option<Declaration> {
        let start = self.pos;
        let name = match self.peek() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return None,
        };
        self.pos += 1;
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        if !matches!(self.peek(), Some(Token::Colon)) {
            return None;
        }
        self.pos += 1;
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        let custom = name.starts_with("--");
        let mut value = Vec::new();
        // For an ordinary property, a `{}` block next to any other value means
        // this is not a declaration. Stopping there keeps a failed attempt
        // from scanning past the rule it belongs to.
        let mut brace_block = false;
        let mut other = false;
        while let Some(t) = self.peek() {
            if matches!(t, Token::Semicolon) {
                break;
            }
            let v = self.component_value();
            if !custom {
                if matches!(&v, ComponentValue::Block(b) if b.kind == BlockKind::Brace) {
                    brace_block = true;
                } else if !v.is_whitespace() {
                    other = true;
                }
                if brace_block && other {
                    return None;
                }
            }
            value.push(v);
        }
        let span = self.span_from(start);
        let important = strip_important(&mut value);
        while value.last().is_some_and(|v| v.is_whitespace()) {
            value.pop();
        }
        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.pos += 1;
        }
        Some(Declaration {
            name: if custom {
                name
            } else {
                name.to_ascii_lowercase()
            },
            value,
            important,
            span,
        })
    }

    fn bad_declaration_remnants(&mut self) {
        while let Some(t) = self.peek() {
            if matches!(t, Token::Semicolon) {
                self.pos += 1;
                return;
            }
            self.component_value();
        }
    }

    /// A declaration list: rules in it are dropped with a diagnostic.
    fn declaration_list(&mut self, range: Range) -> Result<Vec<Declaration>, LimitError> {
        let items = self.in_range(range, Vec::new(), |p| p.block_contents(Ctx::Nested))?;
        let mut out = Vec::new();
        for item in items {
            match item {
                Item::Decl(d) => out.push(d),
                Item::Rule(r) => self.diag(
                    DiagnosticKind::InvalidRule,
                    "rules are not allowed in a declaration list",
                    r.span(),
                ),
            }
        }
        Ok(out)
    }

    /// `unicode-range` values need the tokenizer's unicode-range mode,
    /// which is off for the rest of a style sheet. Re-read the value text
    /// with it on.
    fn retokenize_unicode_range(&self, d: &mut Declaration) {
        let (Some(first), Some(last)) = (d.value.first(), d.value.last()) else {
            return;
        };
        let (start, end) = (first.span().start, last.span().end);
        if end > self.src.len() || start > end {
            return;
        }
        let tokenizer = Tokenizer::new(&self.src[..end])
            .with_unicode_ranges(true)
            .starting_at(start);
        let options = Options {
            limits: self.limits.clone(),
        };
        if let Ok(value) = parse_component_values_tokens(tokenizer, end - start, &options) {
            d.value = value;
        }
    }

    fn keyframe_list(&mut self, range: Range) -> Result<Vec<Keyframe>, LimitError> {
        self.in_range(range, Vec::new(), |p| {
            let mut out = Vec::new();
            loop {
                match p.peek() {
                    None => return Ok(out),
                    Some(Token::Whitespace | Token::Semicolon) => p.pos += 1,
                    Some(_) => {
                        let start = p.pos;
                        let mut prelude = Vec::new();
                        let mut block = None;
                        while let Some(t) = p.peek() {
                            match t {
                                Token::LeftBrace => {
                                    block = Some(p.block_range());
                                    break;
                                }
                                Token::Semicolon => break,
                                _ => prelude.push(p.component_value()),
                            }
                        }
                        let span = p.span_from(start);
                        let Some(block) = block else {
                            p.diag(DiagnosticKind::InvalidRule, "keyframe has no block", span);
                            continue;
                        };
                        match parse_keyframe_selectors(&prelude) {
                            Some(selectors) => {
                                p.count_rule(span)?;
                                let mut declarations = p.declaration_list(block)?;
                                declarations.retain(|d| {
                                    if d.important {
                                        p.diagnostics.push(Diagnostic::new(
                                            DiagnosticKind::InvalidDeclaration,
                                            "declaration dropped: !important is ignored in keyframes",
                                            d.span,
                                        ));
                                    }
                                    !d.important
                                });
                                out.push(Keyframe {
                                    selectors,
                                    declarations,
                                    span,
                                });
                            }
                            None => p.diag(
                                DiagnosticKind::InvalidSelector,
                                "keyframe dropped: invalid keyframe selector",
                                span,
                            ),
                        }
                    }
                }
            }
        })
    }

    fn page_body(
        &mut self,
        range: Range,
    ) -> Result<(Vec<Declaration>, Vec<PageMarginRule>), LimitError> {
        self.in_range(range, (Vec::new(), Vec::new()), |p| {
            let mut decls = Vec::new();
            let mut margins = Vec::new();
            loop {
                match p.peek() {
                    None => return Ok((decls, margins)),
                    Some(Token::Whitespace | Token::Semicolon) => p.pos += 1,
                    Some(Token::AtKeyword(n)) => {
                        let start = p.pos;
                        let name = n.to_ascii_lowercase();
                        p.pos += 1;
                        let mut block = None;
                        let mut prelude_ok = true;
                        while let Some(t) = p.peek() {
                            match t {
                                Token::LeftBrace => {
                                    block = Some(p.block_range());
                                    break;
                                }
                                Token::Semicolon => {
                                    p.pos += 1;
                                    break;
                                }
                                Token::Whitespace => p.pos += 1,
                                _ => {
                                    prelude_ok = false;
                                    p.component_value();
                                }
                            }
                        }
                        let span = p.span_from(start);
                        match block {
                            Some(b) if prelude_ok && PAGE_MARGIN_BOXES.contains(&name.as_str()) => {
                                p.count_rule(span)?;
                                let declarations = p.declaration_list(b)?;
                                margins.push(PageMarginRule {
                                    name,
                                    declarations,
                                    span,
                                });
                            }
                            _ => p.diag(
                                DiagnosticKind::InvalidAtRule,
                                format!("@{name} is not a page margin rule"),
                                span,
                            ),
                        }
                    }
                    Some(_) => {
                        let outer = p.overflow;
                        p.overflow = false;
                        let mark = p.pos;
                        match p.declaration() {
                            Some(d) if !p.overflow => decls.push(d),
                            Some(d) => p.diag(
                                DiagnosticKind::LimitReached(LimitKind::Depth),
                                "declaration nests too deep and was dropped",
                                d.span,
                            ),
                            None => {
                                p.pos = mark;
                                p.bad_declaration_remnants();
                                let span = p.span_from(mark);
                                p.diag(
                                    DiagnosticKind::InvalidDeclaration,
                                    "invalid declaration in @page",
                                    span,
                                );
                            }
                        }
                        p.overflow = outer;
                    }
                }
            }
        })
    }
}

/// The margin boxes of CSS Paged Media.
pub const PAGE_MARGIN_BOXES: &[&str] = &[
    "top-left-corner",
    "top-left",
    "top-center",
    "top-right",
    "top-right-corner",
    "bottom-left-corner",
    "bottom-left",
    "bottom-center",
    "bottom-right",
    "bottom-right-corner",
    "left-top",
    "left-middle",
    "left-bottom",
    "right-top",
    "right-middle",
    "right-bottom",
];

/// Remove a trailing `! important` and report whether it was there.
fn strip_important(value: &mut Vec<ComponentValue>) -> bool {
    let mut idx: Vec<usize> = Vec::new();
    for (i, v) in value.iter().enumerate().rev() {
        if !v.is_whitespace() {
            idx.push(i);
            if idx.len() == 2 {
                break;
            }
        }
    }
    if idx.len() == 2 && value[idx[0]].is_ident("important") && value[idx[1]].is_delim('!') {
        value.truncate(idx[1]);
        true
    } else {
        false
    }
}

fn starts_like_custom_property(prelude: &[ComponentValue]) -> bool {
    let mut it = prelude.iter().filter(|v| !v.is_whitespace());
    matches!(
        (it.next().and_then(|v| v.token()), it.next().and_then(|v| v.token())),
        (Some(Token::Ident(n)), Some(Token::Colon)) if n.starts_with("--")
    )
}

fn non_ws(values: &[ComponentValue]) -> Vec<&ComponentValue> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

fn parse_property_name(prelude: &[ComponentValue]) -> Option<String> {
    let parts = non_ws(prelude);
    let [one] = parts.as_slice() else {
        return None;
    };
    match one.token() {
        Some(Token::Ident(n)) if n.starts_with("--") && n.len() > 2 => Some(n.clone()),
        _ => None,
    }
}

fn parse_scope_prelude(
    prelude: &[ComponentValue],
    limits: &Limits,
) -> Option<(Option<SelectorList>, Option<SelectorList>)> {
    let parts = non_ws(prelude);
    if parts.is_empty() {
        return Some((None, None));
    }
    let opts = SelectorOptions {
        nested: true,
        max_depth: limits.max_selector_depth,
        max_length: limits.max_selector_length,
    };
    let parse_paren_sel = |cv: &ComponentValue| -> Option<SelectorList> {
        let ComponentValue::Block(b) = cv else {
            return None;
        };
        if b.kind != BlockKind::Paren {
            return None;
        }
        parse_selector_list_from(&b.contents, &opts).ok()
    };
    match parts.as_slice() {
        [start_block] => Some((Some(parse_paren_sel(start_block)?), None)),
        [to_kw, end_block] if to_kw.is_ident("to") => {
            Some((None, Some(parse_paren_sel(end_block)?)))
        }
        [start_block, to_kw, end_block] if to_kw.is_ident("to") => Some((
            Some(parse_paren_sel(start_block)?),
            Some(parse_paren_sel(end_block)?),
        )),
        _ => None,
    }
}

/// Parse `a.b, c` into layer names. Whitespace is not allowed around `.`.
/// Returns `None` when invalid; an empty prelude gives an empty list.
fn parse_layer_names(prelude: &[ComponentValue]) -> Option<Vec<LayerName>> {
    let mut names = Vec::new();
    let mut groups: Vec<Vec<&ComponentValue>> = vec![Vec::new()];
    for v in prelude {
        if matches!(v.token(), Some(Token::Comma)) {
            groups.push(Vec::new());
        } else if let Some(g) = groups.last_mut() {
            g.push(v);
        }
    }
    if groups.len() == 1 && groups[0].iter().all(|v| v.is_whitespace()) {
        return Some(names);
    }
    for g in groups {
        let mut parts = g.as_slice();
        while parts.first().is_some_and(|v| v.is_whitespace()) {
            parts = &parts[1..];
        }
        while parts.last().is_some_and(|v| v.is_whitespace()) {
            parts = &parts[..parts.len() - 1];
        }
        let mut name = Vec::new();
        let mut expect_ident = true;
        for v in parts {
            match v.token() {
                Some(Token::Ident(s)) if expect_ident => {
                    if is_css_wide_keyword(s) {
                        return None;
                    }
                    name.push(s.clone());
                    expect_ident = false;
                }
                Some(Token::Delim('.')) if !expect_ident => expect_ident = true,
                _ => return None,
            }
        }
        if name.is_empty() || expect_ident {
            return None;
        }
        names.push(name);
    }
    Some(names)
}

/// The CSS-wide keywords, which are never valid as custom names.
pub fn is_css_wide_keyword(s: &str) -> bool {
    [
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ]
    .iter()
    .any(|k| s.eq_ignore_ascii_case(k))
}

type ImportParts = (
    String,
    Option<ImportLayer>,
    Option<SupportsCondition>,
    MediaQueryList,
);

fn parse_import_prelude(prelude: &[ComponentValue], limits: &Limits) -> Option<ImportParts> {
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while prelude.get(*i).is_some_and(|v| v.is_whitespace()) {
            *i += 1;
        }
    };
    skip_ws(&mut i);
    let url = url_or_string(prelude.get(i)?)?;
    i += 1;
    skip_ws(&mut i);
    let mut layer = None;
    if let Some(v) = prelude.get(i) {
        if v.is_ident("layer") {
            layer = Some(ImportLayer::Anonymous);
            i += 1;
        } else if let ComponentValue::Function(f) = v
            && f.name.eq_ignore_ascii_case("layer")
        {
            let names = parse_layer_names(&f.arguments)?;
            if names.len() != 1 {
                return None;
            }
            layer = names.into_iter().next().map(ImportLayer::Named);
            i += 1;
        }
    }
    skip_ws(&mut i);
    let mut supports = None;
    if let Some(ComponentValue::Function(f)) = prelude.get(i)
        && f.name.eq_ignore_ascii_case("supports")
    {
        let cond = match parse_supports_condition(&f.arguments, limits) {
            Ok(c) => c,
            Err(_) => media::supports_declaration_from(&f.arguments, limits)?,
        };
        supports = Some(cond);
        i += 1;
    }
    let media = parse_media_query_list(&prelude[i.min(prelude.len())..], limits);
    Some((url, layer, supports, media))
}

/// The URL from a `url(...)` token, a `url("...")` function, or a string.
pub fn url_or_string(v: &ComponentValue) -> Option<String> {
    match v {
        ComponentValue::Token(t) => match &t.token {
            Token::Url(u) | Token::String(u) => Some(u.clone()),
            _ => None,
        },
        ComponentValue::Function(f) if f.name.eq_ignore_ascii_case("url") => {
            let args = non_ws(&f.arguments);
            match args.as_slice() {
                [only] => match only.token() {
                    Some(Token::String(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_namespace_prelude(prelude: &[ComponentValue]) -> Option<(Option<String>, String)> {
    let parts = non_ws(prelude);
    match parts.as_slice() {
        [url] => Some((None, url_or_string(url)?)),
        [prefix, url] => match prefix.token() {
            Some(Token::Ident(p)) => Some((Some(p.clone()), url_or_string(url)?)),
            _ => None,
        },
        _ => None,
    }
}

fn parse_keyframes_name(prelude: &[ComponentValue]) -> Option<String> {
    let parts = non_ws(prelude);
    match parts.as_slice() {
        [one] => match one.token() {
            Some(Token::Ident(n)) if !is_css_wide_keyword(n) && !n.eq_ignore_ascii_case("none") => {
                Some(n.clone())
            }
            Some(Token::String(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn parse_keyframe_selectors(prelude: &[ComponentValue]) -> Option<Vec<f64>> {
    let mut out = Vec::new();
    for group in prelude.split(|v| matches!(v.token(), Some(Token::Comma))) {
        let parts = non_ws(group);
        let [one] = parts.as_slice() else {
            return None;
        };
        let pct = match one.token() {
            Some(Token::Ident(n)) if n.eq_ignore_ascii_case("from") => 0.0,
            Some(Token::Ident(n)) if n.eq_ignore_ascii_case("to") => 100.0,
            Some(Token::Percentage(n)) if (0.0..=100.0).contains(&n.value) => n.value,
            _ => return None,
        };
        out.push(pct);
    }
    Some(out)
}

fn parse_page_selectors(prelude: &[ComponentValue]) -> Option<Vec<PageSelector>> {
    let mut out = Vec::new();
    if prelude.iter().all(|v| v.is_whitespace()) {
        return Some(out);
    }
    for group in prelude.split(|v| matches!(v.token(), Some(Token::Comma))) {
        let mut parts = group;
        while parts.first().is_some_and(|v| v.is_whitespace()) {
            parts = &parts[1..];
        }
        while parts.last().is_some_and(|v| v.is_whitespace()) {
            parts = &parts[..parts.len() - 1];
        }
        if parts.is_empty() {
            return None;
        }
        let mut sel = PageSelector {
            name: None,
            pseudo_classes: Vec::new(),
        };
        let mut i = 0;
        if let Some(Token::Ident(n)) = parts[0].token() {
            sel.name = Some(n.clone());
            i = 1;
        }
        while i < parts.len() {
            match (parts[i].token(), parts.get(i + 1).and_then(|v| v.token())) {
                (Some(Token::Colon), Some(Token::Ident(p))) => {
                    let p = p.to_ascii_lowercase();
                    if !["left", "right", "first", "blank"].contains(&p.as_str()) {
                        return None;
                    }
                    sel.pseudo_classes.push(p);
                    i += 2;
                }
                _ => return None,
            }
        }
        out.push(sel);
    }
    Some(out)
}
