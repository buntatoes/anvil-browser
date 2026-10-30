// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Selector parsing, specificity, matching, and a rule index.
//!
//! Written from Selectors Level 4 and CSS Nesting. Parsing works on
//! component values, so escapes, strings, and nesting are already resolved
//! by the tokenizer and parser.
//!
//! - Lists: a selector list is invalid when any of its selectors is
//!   invalid, which drops the whole style rule. The arguments of `:is()` and
//!   `:where()` are forgiving lists: invalid entries are dropped and the rest
//!   is kept. `:not()` and `:has()` are not forgiving.
//! - Nesting: `&` is accepted anywhere. Inside a nested style rule, a
//!   selector may start with a combinator (`> .a`).
//!   [`SelectorList::resolve_nesting`] turns nested selectors into plain
//!   ones by replacing `&` with `:is(<parent>)`. An unresolved `&` matches
//!   like `:scope`.
//! - Specificity follows section 17 of Selectors Level 4.
//! - Matching runs right to left over the host's tree through the small
//!   [`Element`] trait. Failed descendant and sibling searches return early
//!   when no further candidate can succeed, so matching stays linear in the
//!   depth of the tree for common selectors.
//!
//! Every recursion is bounded. Selector function nesting stops at
//! [`SelectorOptions::max_depth`] and selector length at
//! [`SelectorOptions::max_length`], both reported as errors. Matching
//! recursion follows the parsed selector, so it inherits those bounds.

use std::collections::HashMap;

use crate::prism::error::{LimitKind, MAX_SELECTOR_DEPTH, MAX_SELECTOR_LENGTH, Span};
use crate::prism::parser::{BlockKind, ComponentValue, Options, parse_component_values};
use crate::prism::tokenizer::Token;

/// Options for selector parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorOptions {
    /// True inside a nested style rule: selectors may start with a
    /// combinator.
    pub nested: bool,
    /// Deepest nesting of selector functions.
    pub max_depth: usize,
    /// Most compound selectors in one complex selector.
    pub max_length: usize,
}

impl Default for SelectorOptions {
    fn default() -> Self {
        SelectorOptions {
            nested: false,
            max_depth: MAX_SELECTOR_DEPTH,
            max_length: MAX_SELECTOR_LENGTH,
        }
    }
}

/// Why a selector is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorError {
    /// Plain description.
    pub message: String,
    /// Where it went wrong.
    pub span: Span,
    /// The limit that was reached, when a limit caused it.
    pub limit: Option<LimitKind>,
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid selector at bytes {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for SelectorError {}

fn err<T>(message: impl Into<String>, span: Span) -> Result<T, SelectorError> {
    Err(SelectorError {
        message: message.into(),
        span,
        limit: None,
    })
}

/// A combinator between two compound selectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Combinator {
    /// Whitespace.
    Descendant,
    /// `>`
    Child,
    /// `+`
    NextSibling,
    /// `~`
    SubsequentSibling,
}

impl Combinator {
    fn is_sibling(self) -> bool {
        matches!(
            self,
            Combinator::NextSibling | Combinator::SubsequentSibling
        )
    }
}

/// A namespace prefix on a type or attribute selector.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NamespaceConstraint {
    /// `*|`: any namespace.
    Any,
    /// `|`: no namespace.
    NoNamespace,
    /// `prefix|`.
    Prefix(String),
}

/// Attribute selector operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrOperator {
    /// `[a]`
    Exists,
    /// `[a=v]`
    Equals,
    /// `[a~=v]`: one of the whitespace-separated words.
    Includes,
    /// `[a|=v]`: equal, or starts with `v-`.
    DashMatch,
    /// `[a^=v]`
    Prefix,
    /// `[a$=v]`
    Suffix,
    /// `[a*=v]`
    Substring,
}

/// The case flag of an attribute selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaseFlag {
    /// No flag: the document language decides.
    Default,
    /// `i`: ASCII case-insensitive.
    Insensitive,
    /// `s`: case-sensitive.
    Sensitive,
}

/// `[ns|name op "value" flag]`
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AttributeSelector {
    /// Namespace prefix, when written.
    pub namespace: Option<NamespaceConstraint>,
    /// Attribute name as written.
    pub name: String,
    /// Attribute name, ASCII-lowercased (used for HTML elements).
    pub lower_name: String,
    /// The operator.
    pub operator: AttrOperator,
    /// The value (empty for [`AttrOperator::Exists`]).
    pub value: String,
    /// The case flag.
    pub case: CaseFlag,
}

/// Element states the host reports through [`Element::has_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementState {
    /// `:hover`
    Hover,
    /// `:active`
    Active,
    /// `:focus`
    Focus,
    /// `:focus-visible`
    FocusVisible,
    /// `:focus-within`
    FocusWithin,
    /// `:link`: an unvisited link.
    Link,
    /// `:visited`
    Visited,
    /// `:target`
    Target,
    /// `:checked`
    Checked,
    /// `:indeterminate`
    Indeterminate,
    /// `:disabled`
    Disabled,
    /// `:enabled`
    Enabled,
    /// `:required`
    Required,
    /// `:optional`
    Optional,
    /// `:read-only`
    ReadOnly,
    /// `:read-write`
    ReadWrite,
    /// `:placeholder-shown`
    PlaceholderShown,
    /// `:default`
    Default,
    /// `:valid`
    Valid,
    /// `:invalid`
    Invalid,
    /// `:in-range`
    InRange,
    /// `:out-of-range`
    OutOfRange,
    /// `:defined`
    Defined,
    /// `:open`
    Open,
    /// `:modal`
    Modal,
    /// `:fullscreen`
    Fullscreen,
    /// `:autofill`
    Autofill,
}

const STATES: &[(&str, ElementState)] = &[
    ("hover", ElementState::Hover),
    ("active", ElementState::Active),
    ("focus", ElementState::Focus),
    ("focus-visible", ElementState::FocusVisible),
    ("focus-within", ElementState::FocusWithin),
    ("link", ElementState::Link),
    ("visited", ElementState::Visited),
    ("target", ElementState::Target),
    ("checked", ElementState::Checked),
    ("indeterminate", ElementState::Indeterminate),
    ("disabled", ElementState::Disabled),
    ("enabled", ElementState::Enabled),
    ("required", ElementState::Required),
    ("optional", ElementState::Optional),
    ("read-only", ElementState::ReadOnly),
    ("read-write", ElementState::ReadWrite),
    ("placeholder-shown", ElementState::PlaceholderShown),
    ("default", ElementState::Default),
    ("valid", ElementState::Valid),
    ("invalid", ElementState::Invalid),
    ("in-range", ElementState::InRange),
    ("out-of-range", ElementState::OutOfRange),
    ("defined", ElementState::Defined),
    ("open", ElementState::Open),
    ("modal", ElementState::Modal),
    ("fullscreen", ElementState::Fullscreen),
    ("autofill", ElementState::Autofill),
];

const USER_ACTION_STATES: &[ElementState] = &[
    ElementState::Hover,
    ElementState::Active,
    ElementState::Focus,
    ElementState::FocusVisible,
    ElementState::FocusWithin,
];

/// Text direction for `:dir()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

/// The `An+B` microsyntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnB {
    /// Step.
    pub a: i32,
    /// Offset.
    pub b: i32,
}

impl AnB {
    /// True when the 1-based `index` is `a*n + b` for some `n >= 0`.
    pub fn matches(self, index: i64) -> bool {
        let a = i64::from(self.a);
        let b = i64::from(self.b);
        if a == 0 {
            return index == b;
        }
        let diff = index - b;
        diff % a == 0 && diff / a >= 0
    }
}

/// Pseudo-classes.
#[derive(Clone, Debug, PartialEq)]
pub enum PseudoClass {
    /// A state the host reports.
    State(ElementState),
    /// `:any-link`: `:link` or `:visited`.
    AnyLink,
    /// `:is()`
    Is(SelectorList),
    /// `:where()`: like `:is()` with zero specificity.
    Where(SelectorList),
    /// `:not()`
    Not(SelectorList),
    /// `:has()`: relative selectors, each with a leading combinator.
    Has(SelectorList),
    /// `:nth-child(An+B [of S])`
    NthChild(AnB, Option<SelectorList>),
    /// `:nth-last-child(An+B [of S])`
    NthLastChild(AnB, Option<SelectorList>),
    /// `:nth-of-type()`
    NthOfType(AnB),
    /// `:nth-last-of-type()`
    NthLastOfType(AnB),
    /// `:first-child`
    FirstChild,
    /// `:last-child`
    LastChild,
    /// `:only-child`
    OnlyChild,
    /// `:first-of-type`
    FirstOfType,
    /// `:last-of-type`
    LastOfType,
    /// `:only-of-type`
    OnlyOfType,
    /// `:empty`
    Empty,
    /// `:root`
    Root,
    /// `:scope`
    Scope,
    /// `:host`: matches the Shadow DOM host element.
    Host,
    /// `:host(<compound-selector-list>)`: matches the Shadow DOM host when it matches the inner selector list.
    HostFunction(SelectorList),
    /// `:host-context(<compound-selector-list>)`: matches the Shadow DOM host when it or one of its ancestors matches the inner selector list.
    HostContext(SelectorList),
    /// `:lang()` ranges, as written.
    Lang(Vec<String>),
    /// `:dir()`
    Dir(Direction),
    /// A vendor-prefixed pseudo-class starting with `-`.
    Vendor(String),
}

/// Pseudo-elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    /// `::before`
    Before,
    /// `::after`
    After,
    /// `::first-line`
    FirstLine,
    /// `::first-letter`
    FirstLetter,
    /// `::placeholder`
    Placeholder,
    /// `::selection`
    Selection,
    /// `::marker`
    Marker,
    /// `::backdrop`
    Backdrop,
    /// `::file-selector-button`
    FileSelectorButton,
    /// A vendor-prefixed pseudo-element starting with `-`.
    Vendor,
}

const PSEUDO_ELEMENTS: &[(&str, PseudoElement)] = &[
    ("before", PseudoElement::Before),
    ("after", PseudoElement::After),
    ("first-line", PseudoElement::FirstLine),
    ("first-letter", PseudoElement::FirstLetter),
    ("placeholder", PseudoElement::Placeholder),
    ("selection", PseudoElement::Selection),
    ("marker", PseudoElement::Marker),
    ("backdrop", PseudoElement::Backdrop),
    ("file-selector-button", PseudoElement::FileSelectorButton),
];

/// A simple selector.
#[derive(Clone, Debug, PartialEq)]
pub enum SimpleSelector {
    /// A type selector.
    Type {
        /// Namespace prefix, when written.
        namespace: Option<NamespaceConstraint>,
        /// Name as written.
        name: String,
        /// Name, ASCII-lowercased.
        lower: String,
    },
    /// `*`, with an optional namespace prefix.
    Universal {
        /// Namespace prefix, when written.
        namespace: Option<NamespaceConstraint>,
    },
    /// `#id`
    Id(String),
    /// `.class`
    Class(String),
    /// `[attr]`
    Attribute(Box<AttributeSelector>),
    /// `:pseudo-class`
    PseudoClass(PseudoClass),
    /// `&`
    Nesting,
}

/// A compound selector: simple selectors with no combinator between them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Compound {
    /// The simple selectors, in source order.
    pub simple: Vec<SimpleSelector>,
    /// A pseudo-element at the end, when present.
    pub pseudo_element: Option<PseudoElement>,
    /// User-action pseudo-classes written after the pseudo-element, such as
    /// `::before:hover`. They test the originating element.
    pub pseudo_element_states: Vec<ElementState>,
}

/// Specificity `(a, b, c)`: ids, then classes/attributes/pseudo-classes,
/// then types/pseudo-elements. Compared in that order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Specificity {
    /// Id selectors.
    pub a: u32,
    /// Class, attribute, and pseudo-class selectors.
    pub b: u32,
    /// Type selectors and pseudo-elements.
    pub c: u32,
}

impl Specificity {
    /// Build from the three counts.
    pub fn new(a: u32, b: u32, c: u32) -> Self {
        Specificity { a, b, c }
    }

    fn add(self, o: Specificity) -> Specificity {
        Specificity {
            a: self.a.saturating_add(o.a),
            b: self.b.saturating_add(o.b),
            c: self.c.saturating_add(o.c),
        }
    }
}

/// A complex selector: compounds joined by combinators, left to right.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    /// A leading combinator, for relative selectors (`:has(> a)`, and
    /// nested rules such as `> .child`).
    pub leading: Option<Combinator>,
    /// The compounds, left to right. Never empty.
    pub compounds: Vec<Compound>,
    /// `combinators[i]` sits between `compounds[i]` and `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
}

/// A comma-separated list of complex selectors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SelectorList(pub Vec<Selector>);

impl SelectorList {
    /// The selectors.
    pub fn selectors(&self) -> &[Selector] {
        &self.0
    }

    /// The largest specificity in the list (0 for an empty list).
    pub fn max_specificity(&self) -> Specificity {
        self.0
            .iter()
            .map(Selector::specificity)
            .max()
            .unwrap_or_default()
    }

    /// True when any selector matches `el` (without a pseudo-element).
    pub fn matches<E: Element>(&self, el: &E) -> bool {
        self.0.iter().any(|s| s.matches(el))
    }

    /// Resolve nesting against the parent rule's selectors. `&` becomes
    /// `:is(parent)`. A selector without `&` is made relative to the
    /// parent: `> .a` becomes `:is(parent) > .a`, and `.a` becomes
    /// `:is(parent) .a`. Without a parent (a top-level rule) the selectors
    /// are returned as they are, and `&` keeps matching like `:scope`.
    ///
    /// Each `&` copies the parent, so heavy use can grow selectors
    /// exponentially. More than `max_simple` simple selectors in one result
    /// is an error.
    pub fn resolve_nesting(
        &self,
        parent: Option<&SelectorList>,
        max_simple: usize,
    ) -> Result<SelectorList, SelectorError> {
        let Some(parent) = parent else {
            return Ok(self.clone());
        };
        let parent_size = parent
            .0
            .iter()
            .map(Selector::size)
            .fold(0usize, usize::saturating_add)
            .max(1);
        let mut out = Vec::new();
        for sel in &self.0 {
            let amps = sel.nesting_count();
            let own = sel.size();
            let total = own.saturating_add(amps.max(1).saturating_mul(parent_size));
            if total > max_simple {
                return Err(SelectorError {
                    message: format!(
                        "resolved nested selector would have {total} simple selectors (limit {max_simple})"
                    ),
                    span: Span::default(),
                    limit: Some(LimitKind::SelectorLength),
                });
            }
            let mut s = sel.clone();
            let parent_compound = Compound {
                simple: vec![SimpleSelector::PseudoClass(PseudoClass::Is(parent.clone()))],
                ..Compound::default()
            };
            if amps > 0 {
                s.replace_nesting(parent);
                if let Some(lead) = s.leading.take() {
                    s.compounds.insert(0, parent_compound);
                    s.combinators.insert(0, lead);
                }
            } else {
                let lead = s.leading.take().unwrap_or(Combinator::Descendant);
                s.compounds.insert(0, parent_compound);
                s.combinators.insert(0, lead);
            }
            out.push(s);
        }
        Ok(SelectorList(out))
    }
}

impl Selector {
    /// The specificity. An unresolved `&` counts as zero.
    pub fn specificity(&self) -> Specificity {
        let mut s = Specificity::default();
        for c in &self.compounds {
            s = s.add(compound_specificity(c));
        }
        s
    }

    /// The pseudo-element this selector targets, if any.
    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        self.compounds.last().and_then(|c| c.pseudo_element)
    }

    /// The rightmost compound, or `None` for an empty selector.
    pub fn rightmost(&self) -> Option<&Compound> {
        self.compounds.last()
    }

    /// True when `el` matches (selectors with a pseudo-element never do).
    pub fn matches<E: Element>(&self, el: &E) -> bool {
        self.matches_in(el, &MatchContext::default())
    }

    /// Match with a context (scope element, requested pseudo-element).
    pub fn matches_in<E: Element>(&self, el: &E, ctx: &MatchContext<E>) -> bool {
        if self.pseudo_element() != ctx.pseudo_element || self.compounds.is_empty() {
            return false;
        }
        let anchor = if self.leading.is_some() {
            match &ctx.scope {
                Some(s) => Some(s),
                None => return false,
            }
        } else {
            None
        };
        let m = Matcher { ctx };
        m.match_from(self, self.compounds.len() - 1, el, anchor) == Res::Matched
    }

    fn size(&self) -> usize {
        let mut n = 0usize;
        for c in &self.compounds {
            n = n.saturating_add(c.simple.len().max(1));
            for s in &c.simple {
                if let SimpleSelector::PseudoClass(p) = s {
                    n = n.saturating_add(pseudo_size(p));
                }
            }
        }
        n
    }

    fn nesting_count(&self) -> usize {
        let mut n = 0usize;
        for c in &self.compounds {
            for s in &c.simple {
                match s {
                    SimpleSelector::Nesting => n = n.saturating_add(1),
                    SimpleSelector::PseudoClass(p) => {
                        for l in pseudo_lists(p) {
                            for sel in &l.0 {
                                n = n.saturating_add(sel.nesting_count());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        n
    }

    fn replace_nesting(&mut self, parent: &SelectorList) {
        for c in &mut self.compounds {
            for s in &mut c.simple {
                match s {
                    SimpleSelector::Nesting => {
                        *s = SimpleSelector::PseudoClass(PseudoClass::Is(parent.clone()));
                    }
                    SimpleSelector::PseudoClass(p) => {
                        for l in pseudo_lists_mut(p) {
                            for sel in &mut l.0 {
                                sel.replace_nesting(parent);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn pseudo_lists(p: &PseudoClass) -> Vec<&SelectorList> {
    match p {
        PseudoClass::Is(l)
        | PseudoClass::Where(l)
        | PseudoClass::Not(l)
        | PseudoClass::Has(l)
        | PseudoClass::HostFunction(l)
        | PseudoClass::HostContext(l) => {
            vec![l]
        }
        PseudoClass::NthChild(_, Some(l)) | PseudoClass::NthLastChild(_, Some(l)) => vec![l],
        _ => Vec::new(),
    }
}

fn pseudo_lists_mut(p: &mut PseudoClass) -> Vec<&mut SelectorList> {
    match p {
        PseudoClass::Is(l)
        | PseudoClass::Where(l)
        | PseudoClass::Not(l)
        | PseudoClass::Has(l)
        | PseudoClass::HostFunction(l)
        | PseudoClass::HostContext(l) => {
            vec![l]
        }
        PseudoClass::NthChild(_, Some(l)) | PseudoClass::NthLastChild(_, Some(l)) => vec![l],
        _ => Vec::new(),
    }
}

fn pseudo_size(p: &PseudoClass) -> usize {
    pseudo_lists(p)
        .iter()
        .flat_map(|l| l.0.iter())
        .map(Selector::size)
        .fold(0usize, usize::saturating_add)
}

fn compound_specificity(c: &Compound) -> Specificity {
    let mut s = Specificity::default();
    for simple in &c.simple {
        s = s.add(simple_specificity(simple));
    }
    if c.pseudo_element.is_some() {
        s.c = s.c.saturating_add(1);
    }
    let states = u32::try_from(c.pseudo_element_states.len()).unwrap_or(u32::MAX);
    s.b = s.b.saturating_add(states);
    s
}

fn simple_specificity(s: &SimpleSelector) -> Specificity {
    match s {
        SimpleSelector::Id(_) => Specificity::new(1, 0, 0),
        SimpleSelector::Class(_) | SimpleSelector::Attribute(_) => Specificity::new(0, 1, 0),
        SimpleSelector::Type { .. } => Specificity::new(0, 0, 1),
        SimpleSelector::Universal { .. } | SimpleSelector::Nesting => Specificity::default(),
        SimpleSelector::PseudoClass(p) => match p {
            PseudoClass::Is(l) | PseudoClass::Not(l) | PseudoClass::Has(l) => l.max_specificity(),
            PseudoClass::Where(_) => Specificity::default(),
            PseudoClass::HostFunction(l) | PseudoClass::HostContext(l) => {
                Specificity::new(0, 1, 0).add(l.max_specificity())
            }
            PseudoClass::NthChild(_, Some(l)) | PseudoClass::NthLastChild(_, Some(l)) => {
                Specificity::new(0, 1, 0).add(l.max_specificity())
            }
            _ => Specificity::new(0, 1, 0),
        },
    }
}

// ---- parsing ----

/// Parse a selector list from text, with default options.
pub fn parse_selector_list(src: &str) -> Result<SelectorList, SelectorError> {
    parse_selector_list_with(src, &SelectorOptions::default())
}

/// Parse a forgiving selector list from text, dropping invalid entries while keeping valid ones.
pub fn parse_forgiving_selector_list(src: &str) -> Result<SelectorList, SelectorError> {
    let options = SelectorOptions::default();
    let values = parse_component_values(src, &Options::default()).map_err(|e| SelectorError {
        message: e.to_string(),
        span: Span::new(0, src.len()),
        limit: match e {
            crate::prism::error::Error::Limit(l) => Some(l.kind),
        },
    })?;
    let p = SelParser { options: &options };
    p.list(&values, 0, ListMode::ForgivingTop, false)
}

/// Parse a selector list from text.
pub fn parse_selector_list_with(
    src: &str,
    options: &SelectorOptions,
) -> Result<SelectorList, SelectorError> {
    let values = parse_component_values(src, &Options::default()).map_err(|e| SelectorError {
        message: e.to_string(),
        span: Span::new(0, src.len()),
        limit: match e {
            crate::prism::error::Error::Limit(l) => Some(l.kind),
        },
    })?;
    parse_selector_list_from(&values, options)
}

/// Parse a selector list from component values (a style rule's prelude).
pub fn parse_selector_list_from(
    values: &[ComponentValue],
    options: &SelectorOptions,
) -> Result<SelectorList, SelectorError> {
    let p = SelParser { options };
    p.list(values, 0, ListMode::Top, false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListMode {
    /// A style rule's selector list.
    Top,
    /// A top-level forgiving selector list (allows pseudo-elements, drops invalid selectors).
    ForgivingTop,
    /// `:is()` / `:where()`: forgiving, no pseudo-elements.
    Forgiving,
    /// `:not()` and `of S`: unforgiving, no pseudo-elements.
    Inner,
    /// `:has()`: relative, unforgiving, no pseudo-elements.
    Relative,
}

struct SelParser<'o> {
    options: &'o SelectorOptions,
}

fn span_of(values: &[ComponentValue]) -> Span {
    match (values.first(), values.last()) {
        (Some(a), Some(b)) => a.span().to(b.span()),
        _ => Span::default(),
    }
}

fn split_commas(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    values
        .split(|v| matches!(v.token(), Some(Token::Comma)))
        .collect()
}

fn trim(mut v: &[ComponentValue]) -> &[ComponentValue] {
    while v.first().is_some_and(|x| x.is_whitespace()) {
        v = &v[1..];
    }
    while v.last().is_some_and(|x| x.is_whitespace()) {
        v = &v[..v.len() - 1];
    }
    v
}

fn skip_ws(v: &[ComponentValue], i: &mut usize) -> bool {
    let start = *i;
    while v.get(*i).is_some_and(|x| x.is_whitespace()) {
        *i += 1;
    }
    *i > start
}

fn combinator_of(v: &ComponentValue) -> Option<Combinator> {
    match v.token() {
        Some(Token::Delim('>')) => Some(Combinator::Child),
        Some(Token::Delim('+')) => Some(Combinator::NextSibling),
        Some(Token::Delim('~')) => Some(Combinator::SubsequentSibling),
        _ => None,
    }
}

fn lookup_pseudo_element(name: &str) -> Option<PseudoElement> {
    if name.starts_with('-') {
        return Some(PseudoElement::Vendor);
    }
    PSEUDO_ELEMENTS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, p)| *p)
}

impl SelParser<'_> {
    fn list(
        &self,
        values: &[ComponentValue],
        depth: usize,
        mode: ListMode,
        in_has: bool,
    ) -> Result<SelectorList, SelectorError> {
        if depth > self.options.max_depth {
            return Err(SelectorError {
                message: format!(
                    "selector functions nest deeper than {} levels",
                    self.options.max_depth
                ),
                span: span_of(values),
                limit: Some(LimitKind::SelectorDepth),
            });
        }
        let is_forgiving = matches!(mode, ListMode::Forgiving | ListMode::ForgivingTop);
        let mut out = Vec::new();
        for group in split_commas(values) {
            let group = trim(group);
            let r = if group.is_empty() {
                err("empty selector in list", span_of(values))
            } else {
                self.complex(group, depth, mode, in_has)
            };
            match r {
                Ok(s) => out.push(s),
                Err(e) if is_forgiving && e.limit.is_none() => {}
                Err(e) => return Err(e),
            }
        }
        if out.is_empty() && !is_forgiving {
            return err("empty selector list", span_of(values));
        }
        Ok(SelectorList(out))
    }

    fn complex(
        &self,
        v: &[ComponentValue],
        depth: usize,
        mode: ListMode,
        in_has: bool,
    ) -> Result<Selector, SelectorError> {
        let mut i = 0;
        let mut leading = None;
        let relative_ok = mode == ListMode::Relative
            || (matches!(mode, ListMode::Top | ListMode::ForgivingTop) && self.options.nested);
        if let Some(c) = v.first().and_then(combinator_of) {
            if !relative_ok {
                return err("selector starts with a combinator", v[0].span());
            }
            leading = Some(c);
            i = 1;
            skip_ws(v, &mut i);
        }
        if mode == ListMode::Relative && leading.is_none() {
            leading = Some(Combinator::Descendant);
        }
        let allow_pe = matches!(mode, ListMode::Top | ListMode::ForgivingTop);
        let mut compounds = Vec::new();
        let mut combinators = Vec::new();
        loop {
            let c = self.compound(v, &mut i, depth, allow_pe, in_has)?;
            let has_pe = c.pseudo_element.is_some();
            compounds.push(c);
            if compounds.len() > self.options.max_length {
                return Err(SelectorError {
                    message: format!(
                        "more than {} compound selectors in one selector",
                        self.options.max_length
                    ),
                    span: span_of(v),
                    limit: Some(LimitKind::SelectorLength),
                });
            }
            let saw_ws = skip_ws(v, &mut i);
            if i >= v.len() {
                break;
            }
            if has_pe {
                return err("nothing may follow a pseudo-element", v[i].span());
            }
            let comb = if let Some(c) = combinator_of(&v[i]) {
                i += 1;
                skip_ws(v, &mut i);
                if i >= v.len() {
                    return err("selector ends with a combinator", v[i - 1].span());
                }
                c
            } else if saw_ws {
                Combinator::Descendant
            } else {
                return err("unexpected token in selector", v[i].span());
            };
            combinators.push(comb);
        }
        Ok(Selector {
            leading,
            compounds,
            combinators,
        })
    }

    fn compound(
        &self,
        v: &[ComponentValue],
        i: &mut usize,
        depth: usize,
        allow_pe: bool,
        in_has: bool,
    ) -> Result<Compound, SelectorError> {
        let start = *i;
        let mut c = Compound::default();
        if let Some(t) = type_selector(v, i)? {
            c.simple.push(t);
        }
        while *i < v.len() {
            match &v[*i] {
                ComponentValue::Token(t) => match &t.token {
                    Token::Hash { value, is_id } => {
                        if c.pseudo_element.is_some() {
                            return err("id after a pseudo-element", t.span);
                        }
                        if !is_id {
                            return err("id selector must be a valid identifier", t.span);
                        }
                        c.simple.push(SimpleSelector::Id(value.clone()));
                        *i += 1;
                    }
                    Token::Delim('.') => {
                        if c.pseudo_element.is_some() {
                            return err("class after a pseudo-element", t.span);
                        }
                        match v.get(*i + 1).and_then(|x| x.token()) {
                            Some(Token::Ident(name)) => {
                                c.simple.push(SimpleSelector::Class(name.clone()));
                                *i += 2;
                            }
                            _ => return err("'.' must be followed by a class name", t.span),
                        }
                    }
                    Token::Delim('&') => {
                        if c.pseudo_element.is_some() {
                            return err("'&' after a pseudo-element", t.span);
                        }
                        c.simple.push(SimpleSelector::Nesting);
                        *i += 1;
                    }
                    Token::Colon => {
                        self.colon(v, i, depth, allow_pe, in_has, &mut c, t.span)?;
                    }
                    _ => break,
                },
                ComponentValue::Block(b) if b.kind == BlockKind::Bracket => {
                    if c.pseudo_element.is_some() {
                        return err("attribute after a pseudo-element", b.span);
                    }
                    let a = attribute(&b.contents, b.span)?;
                    c.simple.push(SimpleSelector::Attribute(Box::new(a)));
                    *i += 1;
                }
                _ => break,
            }
        }
        if *i == start {
            return match v.get(*i) {
                Some(x) => err("expected a selector", x.span()),
                None => err("expected a selector", span_of(v)),
            };
        }
        Ok(c)
    }

    /// At a `:`: a pseudo-class, a pseudo-element, or a legacy
    /// single-colon pseudo-element.
    #[allow(clippy::too_many_arguments)]
    fn colon(
        &self,
        v: &[ComponentValue],
        i: &mut usize,
        depth: usize,
        allow_pe: bool,
        in_has: bool,
        c: &mut Compound,
        span: Span,
    ) -> Result<(), SelectorError> {
        let double = matches!(v.get(*i + 1).and_then(|x| x.token()), Some(Token::Colon));
        if double {
            let Some(next) = v.get(*i + 2) else {
                return err("'::' must be followed by a name", span);
            };
            let pe = match next.token() {
                Some(Token::Ident(n)) => lookup_pseudo_element(n),
                _ => None,
            };
            let Some(pe) = pe else {
                return err("unknown pseudo-element", next.span());
            };
            if !allow_pe {
                return err("pseudo-elements are not allowed here", next.span());
            }
            if c.pseudo_element.is_some() {
                return err("only one pseudo-element is allowed", next.span());
            }
            c.pseudo_element = Some(pe);
            *i += 3;
            return Ok(());
        }
        let Some(next) = v.get(*i + 1) else {
            return err("':' must be followed by a name", span);
        };
        if let Some(Token::Ident(n)) = next.token() {
            let lower = n.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "before" | "after" | "first-line" | "first-letter"
            ) {
                if !allow_pe {
                    return err("pseudo-elements are not allowed here", next.span());
                }
                if c.pseudo_element.is_some() {
                    return err("only one pseudo-element is allowed", next.span());
                }
                c.pseudo_element = lookup_pseudo_element(&lower);
                *i += 2;
                return Ok(());
            }
        }
        let pc = self.pseudo_class(next, depth, in_has)?;
        if c.pseudo_element.is_some() {
            match pc {
                PseudoClass::State(s) if USER_ACTION_STATES.contains(&s) => {
                    c.pseudo_element_states.push(s);
                }
                _ if c.pseudo_element == Some(PseudoElement::Vendor) => {}
                _ => {
                    return err(
                        "only user-action pseudo-classes may follow a pseudo-element",
                        next.span(),
                    );
                }
            }
        } else {
            c.simple.push(SimpleSelector::PseudoClass(pc));
        }
        *i += 2;
        Ok(())
    }

    fn pseudo_class(
        &self,
        v: &ComponentValue,
        depth: usize,
        in_has: bool,
    ) -> Result<PseudoClass, SelectorError> {
        let f = match v {
            ComponentValue::Token(t) => {
                let Token::Ident(name) = &t.token else {
                    return err("':' must be followed by a name", t.span);
                };
                let lower = name.to_ascii_lowercase();
                if lower.starts_with('-') {
                    return Ok(PseudoClass::Vendor(lower));
                }
                if let Some((_, s)) = STATES.iter().find(|(n, _)| *n == lower) {
                    return Ok(PseudoClass::State(*s));
                }
                return Ok(match lower.as_str() {
                    "any-link" => PseudoClass::AnyLink,
                    "first-child" => PseudoClass::FirstChild,
                    "last-child" => PseudoClass::LastChild,
                    "only-child" => PseudoClass::OnlyChild,
                    "first-of-type" => PseudoClass::FirstOfType,
                    "last-of-type" => PseudoClass::LastOfType,
                    "only-of-type" => PseudoClass::OnlyOfType,
                    "empty" => PseudoClass::Empty,
                    "root" => PseudoClass::Root,
                    "scope" => PseudoClass::Scope,
                    "host" => PseudoClass::Host,
                    _ => return err(format!("unknown pseudo-class ':{name}'"), t.span),
                });
            }
            ComponentValue::Block(b) => return err("':' must be followed by a name", b.span),
            ComponentValue::Function(f) => f,
        };
        let lower = f.name.to_ascii_lowercase();
        if lower.starts_with('-') {
            return Ok(PseudoClass::Vendor(lower));
        }
        let args = &f.arguments;
        let d = depth + 1;
        let bad_anb = || SelectorError {
            message: "invalid An+B".into(),
            span: f.span,
            limit: None,
        };
        Ok(match lower.as_str() {
            "is" => PseudoClass::Is(self.list(args, d, ListMode::Forgiving, in_has)?),
            "where" => PseudoClass::Where(self.list(args, d, ListMode::Forgiving, in_has)?),
            "not" => PseudoClass::Not(self.list(args, d, ListMode::Inner, in_has)?),
            "host" => PseudoClass::HostFunction(self.list(args, d, ListMode::Inner, in_has)?),
            "host-context" => {
                PseudoClass::HostContext(self.list(args, d, ListMode::Inner, in_has)?)
            }
            "has" => {
                if in_has {
                    return err(":has() may not be nested", f.span);
                }
                PseudoClass::Has(self.list(args, d, ListMode::Relative, true)?)
            }
            "nth-child" | "nth-last-child" => {
                let (anb_part, of_part) = split_of(args);
                let anb = parse_anb(anb_part).ok_or_else(bad_anb)?;
                let of = match of_part {
                    Some(rest) => Some(self.list(rest, d, ListMode::Inner, in_has)?),
                    None => None,
                };
                if lower == "nth-child" {
                    PseudoClass::NthChild(anb, of)
                } else {
                    PseudoClass::NthLastChild(anb, of)
                }
            }
            "nth-of-type" => PseudoClass::NthOfType(parse_anb(args).ok_or_else(bad_anb)?),
            "nth-last-of-type" => PseudoClass::NthLastOfType(parse_anb(args).ok_or_else(bad_anb)?),
            "lang" => {
                let mut ranges = Vec::new();
                for g in split_commas(args) {
                    match trim(g) {
                        [one] => match one.token() {
                            Some(Token::Ident(s)) | Some(Token::String(s)) => {
                                ranges.push(s.clone())
                            }
                            _ => return err("invalid :lang() argument", one.span()),
                        },
                        _ => return err("invalid :lang() argument", f.span),
                    }
                }
                PseudoClass::Lang(ranges)
            }
            "dir" => match trim(args) {
                [one] if one.is_ident("ltr") => PseudoClass::Dir(Direction::Ltr),
                [one] if one.is_ident("rtl") => PseudoClass::Dir(Direction::Rtl),
                _ => return err("invalid :dir() argument", f.span),
            },
            _ => return err(format!("unknown pseudo-class ':{}()'", f.name), f.span),
        })
    }
}

fn type_selector(
    v: &[ComponentValue],
    i: &mut usize,
) -> Result<Option<SimpleSelector>, SelectorError> {
    let tok = |k: usize| v.get(k).and_then(|x| x.token());
    let name_at = |k: usize| -> Option<Option<String>> {
        match tok(k) {
            Some(Token::Ident(n)) => Some(Some(n.clone())),
            Some(Token::Delim('*')) => Some(None),
            _ => None,
        }
    };
    let make = |ns: Option<NamespaceConstraint>, name: Option<String>| match name {
        Some(n) => SimpleSelector::Type {
            namespace: ns,
            lower: n.to_ascii_lowercase(),
            name: n,
        },
        None => SimpleSelector::Universal { namespace: ns },
    };
    if matches!(tok(*i), Some(Token::Delim('|'))) {
        if let Some(name) = name_at(*i + 1) {
            *i += 2;
            return Ok(Some(make(Some(NamespaceConstraint::NoNamespace), name)));
        }
        return err("'|' must be followed by a name", v[*i].span());
    }
    let Some(first) = name_at(*i) else {
        return Ok(None);
    };
    if matches!(tok(*i + 1), Some(Token::Delim('|')))
        && let Some(name) = name_at(*i + 2)
    {
        let ns = match first {
            Some(p) => NamespaceConstraint::Prefix(p),
            None => NamespaceConstraint::Any,
        };
        *i += 3;
        return Ok(Some(make(Some(ns), name)));
    }
    *i += 1;
    Ok(Some(make(None, first)))
}

fn attribute(contents: &[ComponentValue], span: Span) -> Result<AttributeSelector, SelectorError> {
    let v = trim(contents);
    let tok = |k: usize| v.get(k).and_then(|x| x.token());
    let (namespace, name, mut i) = match (tok(0), tok(1), tok(2)) {
        (Some(Token::Delim('|')), Some(Token::Ident(n)), _) => {
            (Some(NamespaceConstraint::NoNamespace), n.clone(), 2)
        }
        (Some(Token::Delim('*')), Some(Token::Delim('|')), Some(Token::Ident(n))) => {
            (Some(NamespaceConstraint::Any), n.clone(), 3)
        }
        (Some(Token::Ident(p)), Some(Token::Delim('|')), Some(Token::Ident(n))) => {
            (Some(NamespaceConstraint::Prefix(p.clone())), n.clone(), 3)
        }
        (Some(Token::Ident(n)), _, _) => (None, n.clone(), 1),
        _ => return err("attribute selector needs a name", span),
    };
    skip_ws(v, &mut i);
    let mut sel = AttributeSelector {
        namespace,
        lower_name: name.to_ascii_lowercase(),
        name,
        operator: AttrOperator::Exists,
        value: String::new(),
        case: CaseFlag::Default,
    };
    if i >= v.len() {
        return Ok(sel);
    }
    let op = match (tok(i), tok(i + 1)) {
        (Some(Token::Delim('=')), _) => {
            i += 1;
            AttrOperator::Equals
        }
        (Some(Token::Delim(c)), Some(Token::Delim('='))) => {
            let op = match c {
                '~' => AttrOperator::Includes,
                '|' => AttrOperator::DashMatch,
                '^' => AttrOperator::Prefix,
                '$' => AttrOperator::Suffix,
                '*' => AttrOperator::Substring,
                _ => return err("unknown attribute operator", span),
            };
            i += 2;
            op
        }
        _ => return err("unknown attribute operator", span),
    };
    skip_ws(v, &mut i);
    sel.value = match tok(i) {
        Some(Token::Ident(s)) | Some(Token::String(s)) => s.clone(),
        _ => return err("attribute selector needs a value", span),
    };
    sel.operator = op;
    i += 1;
    skip_ws(v, &mut i);
    if i < v.len() {
        sel.case = match tok(i) {
            Some(t) if t.is_ident("i") => CaseFlag::Insensitive,
            Some(t) if t.is_ident("s") => CaseFlag::Sensitive,
            _ => return err("unexpected text in attribute selector", span),
        };
        i += 1;
        skip_ws(v, &mut i);
        if i < v.len() {
            return err("unexpected text in attribute selector", span);
        }
    }
    Ok(sel)
}

/// Split `An+B of S` at the `of` keyword.
fn split_of(args: &[ComponentValue]) -> (&[ComponentValue], Option<&[ComponentValue]>) {
    for (k, v) in args.iter().enumerate() {
        if v.is_ident("of") {
            let before_ws = k > 0 && args[k - 1].is_whitespace();
            let after_ws = args.get(k + 1).is_some_and(|x| x.is_whitespace());
            if before_ws && after_ws {
                return (&args[..k], Some(&args[k + 1..]));
            }
        }
    }
    (args, None)
}

fn to_i32(v: f64) -> i32 {
    if v >= f64::from(i32::MAX) {
        i32::MAX
    } else if v <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        v as i32
    }
}

/// Parse the `An+B` microsyntax (CSS Syntax section 6) from component
/// values. Returns `None` when invalid. Values outside `i32` saturate.
pub fn parse_anb(values: &[ComponentValue]) -> Option<AnB> {
    let v = trim(values);
    let mut toks: Vec<&Token> = Vec::new();
    for x in v {
        toks.push(x.token()?);
    }
    match toks.first()? {
        Token::Ident(s) if toks.len() == 1 && s.eq_ignore_ascii_case("odd") => {
            Some(AnB { a: 2, b: 1 })
        }
        Token::Ident(s) if toks.len() == 1 && s.eq_ignore_ascii_case("even") => {
            Some(AnB { a: 2, b: 0 })
        }
        Token::Number(n) if n.is_integer && toks.len() == 1 => Some(AnB {
            a: 0,
            b: to_i32(n.value),
        }),
        Token::Dimension { value, unit } if value.is_integer => Some(AnB {
            a: to_i32(value.value),
            b: n_form(unit, &toks[1..])?,
        }),
        Token::Ident(s) => {
            let lower = s.to_ascii_lowercase();
            match lower.strip_prefix('-') {
                Some(rest) => Some(AnB {
                    a: -1,
                    b: n_form(rest, &toks[1..])?,
                }),
                None => Some(AnB {
                    a: 1,
                    b: n_form(&lower, &toks[1..])?,
                }),
            }
        }
        Token::Delim('+') => match toks.get(1)? {
            Token::Ident(s) if !s.starts_with('-') => Some(AnB {
                a: 1,
                b: n_form(s, &toks[2..])?,
            }),
            _ => None,
        },
        _ => None,
    }
}

/// After the `A` part: `unit` is the text starting at `n` (`n`, `n-`,
/// `n-3`), `rest` the tokens after it. Returns `B`.
fn n_form(unit: &str, rest: &[&Token]) -> Option<i32> {
    let lower = unit.to_ascii_lowercase();
    let mut r = rest;
    while r.first().is_some_and(|t| t.is_whitespace()) {
        r = &r[1..];
    }
    if lower == "n" {
        return match r {
            [] => Some(0),
            [Token::Number(n)] if n.is_integer && n.has_sign => Some(to_i32(n.value)),
            [Token::Delim(c), tail @ ..] if *c == '+' || *c == '-' => {
                let mut t = tail;
                while t.first().is_some_and(|x| x.is_whitespace()) {
                    t = &t[1..];
                }
                match t {
                    [Token::Number(n)] if n.is_integer && !n.has_sign => {
                        let b = to_i32(n.value);
                        Some(if *c == '-' { b.saturating_neg() } else { b })
                    }
                    _ => None,
                }
            }
            _ => None,
        };
    }
    if lower == "n-" {
        return match r {
            [Token::Number(n)] if n.is_integer && !n.has_sign => {
                Some(to_i32(n.value).saturating_neg())
            }
            _ => None,
        };
    }
    if let Some(digits) = lower.strip_prefix("n-")
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && r.is_empty()
    {
        let b: f64 = digits.parse().ok()?;
        return Some(to_i32(b).saturating_neg());
    }
    None
}

// ---- matching ----

/// The host's view of one element. Handles are cheap to clone.
pub trait Element: Clone {
    /// The parent element, or `None` at the root.
    fn parent_element(&self) -> Option<Self>;
    /// The previous sibling element.
    fn prev_sibling_element(&self) -> Option<Self>;
    /// The next sibling element.
    fn next_sibling_element(&self) -> Option<Self>;
    /// The first child element.
    fn first_child_element(&self) -> Option<Self>;
    /// True when both handles name the same element.
    fn is_same(&self, other: &Self) -> bool;
    /// The local name. For HTML elements it is compared ASCII
    /// case-insensitively.
    fn local_name(&self) -> &str;
    /// The namespace URL. Empty means no namespace.
    fn namespace_url(&self) -> &str {
        "http://www.w3.org/1999/xhtml"
    }
    /// The `id`, if any.
    fn id(&self) -> Option<&str>;
    /// True when the element has the class `name` (case-sensitive).
    fn has_class(&self, name: &str) -> bool;
    /// Call `f` with each class (used by [`RuleIndex`]).
    fn for_each_class<F: FnMut(&str)>(&self, f: F);
    /// The attribute value. For HTML elements `name` is lowercase.
    fn attribute(&self, name: &str) -> Option<&str>;
    /// True when the element is in `state`.
    fn has_state(&self, _state: ElementState) -> bool {
        false
    }
    /// True when the element has no element children and no text.
    fn is_empty(&self) -> bool;
    /// True for the document's root element.
    fn is_root(&self) -> bool {
        self.parent_element().is_none()
    }
    /// True for an HTML element in an HTML document (names fold case).
    fn is_html(&self) -> bool {
        true
    }
    /// The element's language, inherited from ancestors.
    fn lang(&self) -> Option<String> {
        let mut cur = Some(self.clone());
        while let Some(e) = cur {
            if let Some(l) = e.attribute("lang") {
                return Some(l.to_string());
            }
            cur = e.parent_element();
        }
        None
    }
    /// The element's text direction.
    fn direction(&self) -> Direction {
        Direction::Ltr
    }
    /// True when this element is the Shadow DOM host in the current shadow tree evaluation.
    fn is_shadow_host(&self) -> bool {
        false
    }
}

/// Extra inputs to matching.
#[derive(Clone, Debug)]
pub struct MatchContext<E> {
    /// The `:scope` element, and the anchor for relative selectors.
    pub scope: Option<E>,
    /// The Shadow DOM host element when evaluating shadow tree rules.
    pub shadow_host: Option<E>,
    /// The pseudo-element being matched, or `None` for the element itself.
    pub pseudo_element: Option<PseudoElement>,
    /// Namespace prefixes declared with `@namespace`: (prefix, URL).
    pub namespaces: Vec<(String, String)>,
}

impl<E> Default for MatchContext<E> {
    fn default() -> Self {
        MatchContext {
            scope: None,
            shadow_host: None,
            pseudo_element: None,
            namespaces: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Res {
    Matched,
    /// Failed; a later sibling or a higher ancestor may still match.
    FailLaterSibling,
    /// Failed; sibling searches can stop, but an outer descendant search
    /// may still try a higher ancestor.
    FailDescendant,
    /// Failed; no other candidate anywhere can match.
    FailGlobally,
}

/// Attributes whose values HTML compares ASCII case-insensitively in
/// selectors (HTML Living Standard, section "Selectors").
const CASE_INSENSITIVE_ATTRS: &[&str] = &[
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

const HTML_SPACE: [char; 5] = [' ', '\t', '\n', '\r', '\u{C}'];

struct Matcher<'c, E> {
    ctx: &'c MatchContext<E>,
}

fn step<E: Element>(el: &E, comb: Combinator) -> Option<E> {
    match comb {
        Combinator::Descendant | Combinator::Child => el.parent_element(),
        Combinator::NextSibling | Combinator::SubsequentSibling => el.prev_sibling_element(),
    }
}

/// True when `anchor` relates to `el` through `comb` (anchor on the left).
fn related<E: Element>(el: &E, anchor: &E, comb: Combinator) -> bool {
    match comb {
        Combinator::Child => el.parent_element().is_some_and(|p| p.is_same(anchor)),
        Combinator::NextSibling => el.prev_sibling_element().is_some_and(|p| p.is_same(anchor)),
        Combinator::Descendant | Combinator::SubsequentSibling => {
            let mut cur = step(el, comb);
            while let Some(c) = cur {
                if c.is_same(anchor) {
                    return true;
                }
                cur = step(&c, comb);
            }
            false
        }
    }
}

fn same_type<E: Element>(a: &E, b: &E) -> bool {
    a.local_name().eq_ignore_ascii_case(b.local_name()) && a.namespace_url() == b.namespace_url()
}

fn sibling_of_type<E: Element>(el: &E, next: fn(&E) -> Option<E>) -> bool {
    let mut cur = next(el);
    while let Some(s) = cur {
        if same_type(&s, el) {
            return true;
        }
        cur = next(&s);
    }
    false
}

/// `:lang()` matching, a simplified form of RFC 4647 extended filtering. A
/// range matches the same tag, or a tag that continues it after `-`,
/// ignoring ASCII case. `*` matches any non-empty tag, and `*-x` matches a
/// tag with `x` among its later subtags.
fn lang_matches(lang: Option<&str>, range: &str) -> bool {
    let Some(lang) = lang else {
        return false;
    };
    let lang = lang.to_ascii_lowercase();
    let range = range.to_ascii_lowercase();
    if range.is_empty() {
        return lang.is_empty();
    }
    if range == "*" {
        return !lang.is_empty();
    }
    if let Some(rest) = range.strip_prefix("*-") {
        let tail: Vec<&str> = lang.split('-').skip(1).collect();
        let want: Vec<&str> = rest.split('-').collect();
        return tail.windows(want.len()).any(|w| w == want.as_slice());
    }
    lang == range || (lang.starts_with(&range) && lang.as_bytes().get(range.len()) == Some(&b'-'))
}

impl<E: Element> Matcher<'_, E> {
    fn match_from(&self, sel: &Selector, i: usize, el: &E, anchor: Option<&E>) -> Res {
        if !self.compound(&sel.compounds[i], el) {
            return Res::FailLaterSibling;
        }
        if i == 0 {
            return match (sel.leading, anchor) {
                (Some(lead), Some(a)) if !related(el, a, lead) => Res::FailLaterSibling,
                _ => Res::Matched,
            };
        }
        let comb = sel.combinators[i - 1];
        let mut cand = step(el, comb);
        loop {
            let Some(c) = cand else {
                return if comb.is_sibling() {
                    Res::FailDescendant
                } else {
                    Res::FailGlobally
                };
            };
            let r = self.match_from(sel, i - 1, &c, anchor);
            if matches!(r, Res::Matched | Res::FailGlobally) {
                return r;
            }
            match comb {
                Combinator::Child => return Res::FailDescendant,
                Combinator::NextSibling => return r,
                Combinator::SubsequentSibling if r == Res::FailDescendant => return r,
                _ => {}
            }
            cand = step(&c, comb);
        }
    }

    fn compound(&self, c: &Compound, el: &E) -> bool {
        c.simple.iter().all(|s| self.simple(s, el))
            && c.pseudo_element_states.iter().all(|s| el.has_state(*s))
    }

    fn namespace_ok(&self, ns: &Option<NamespaceConstraint>, el: &E) -> bool {
        match ns {
            None | Some(NamespaceConstraint::Any) => true,
            Some(NamespaceConstraint::NoNamespace) => el.namespace_url().is_empty(),
            Some(NamespaceConstraint::Prefix(p)) => self
                .ctx
                .namespaces
                .iter()
                .find(|(k, _)| k == p)
                .is_some_and(|(_, url)| url == el.namespace_url()),
        }
    }

    fn simple(&self, s: &SimpleSelector, el: &E) -> bool {
        match s {
            SimpleSelector::Type {
                namespace,
                name,
                lower,
            } => {
                let name_ok = if el.is_html() {
                    el.local_name().eq_ignore_ascii_case(lower)
                } else {
                    el.local_name() == name
                };
                name_ok && self.namespace_ok(namespace, el)
            }
            SimpleSelector::Universal { namespace } => self.namespace_ok(namespace, el),
            SimpleSelector::Id(id) => el.id() == Some(id.as_str()),
            SimpleSelector::Class(c) => el.has_class(c),
            SimpleSelector::Attribute(a) => self.attribute(a, el),
            SimpleSelector::PseudoClass(p) => self.pseudo(p, el),
            SimpleSelector::Nesting => self.is_scope(el),
        }
    }

    fn is_scope(&self, el: &E) -> bool {
        match &self.ctx.scope {
            Some(s) => s.is_same(el),
            None => el.is_root(),
        }
    }

    fn is_host(&self, el: &E) -> bool {
        match &self.ctx.shadow_host {
            Some(h) => h.is_same(el),
            None => el.is_shadow_host(),
        }
    }

    fn attribute(&self, a: &AttributeSelector, el: &E) -> bool {
        if matches!(a.namespace, Some(NamespaceConstraint::Prefix(_)))
            && !self.namespace_ok(&a.namespace, el)
        {
            return false;
        }
        let name = if el.is_html() { &a.lower_name } else { &a.name };
        let Some(value) = el.attribute(name) else {
            return false;
        };
        let insensitive = match a.case {
            CaseFlag::Insensitive => true,
            CaseFlag::Sensitive => false,
            CaseFlag::Default => {
                el.is_html() && CASE_INSENSITIVE_ATTRS.contains(&a.lower_name.as_str())
            }
        };
        let (value, want) = if insensitive {
            (value.to_ascii_lowercase(), a.value.to_ascii_lowercase())
        } else {
            (value.to_string(), a.value.clone())
        };
        match a.operator {
            AttrOperator::Exists => true,
            AttrOperator::Equals => value == want,
            AttrOperator::Includes => {
                !want.is_empty()
                    && !want.contains(HTML_SPACE)
                    && value.split(HTML_SPACE).any(|w| w == want)
            }
            AttrOperator::DashMatch => {
                value == want
                    || (value.len() > want.len()
                        && value.starts_with(want.as_str())
                        && value.as_bytes().get(want.len()) == Some(&b'-'))
            }
            AttrOperator::Prefix => !want.is_empty() && value.starts_with(want.as_str()),
            AttrOperator::Suffix => !want.is_empty() && value.ends_with(want.as_str()),
            AttrOperator::Substring => !want.is_empty() && value.contains(want.as_str()),
        }
    }

    fn list(&self, l: &SelectorList, el: &E) -> bool {
        l.0.iter().any(|s| {
            if s.pseudo_element().is_some() || s.compounds.is_empty() {
                return false;
            }
            let anchor = match (s.leading, &self.ctx.scope) {
                (None, _) => None,
                (Some(_), Some(sc)) => Some(sc),
                (Some(_), None) => return false,
            };
            self.match_from(s, s.compounds.len() - 1, el, anchor) == Res::Matched
        })
    }

    fn count_siblings(
        &self,
        el: &E,
        next: fn(&E) -> Option<E>,
        filter: &dyn Fn(&E) -> bool,
    ) -> i64 {
        let mut n = 1i64;
        let mut cur = next(el);
        while let Some(s) = cur {
            if filter(&s) {
                n += 1;
            }
            cur = next(&s);
        }
        n
    }

    fn pseudo(&self, p: &PseudoClass, el: &E) -> bool {
        match p {
            PseudoClass::State(s) => el.has_state(*s),
            PseudoClass::AnyLink => {
                el.has_state(ElementState::Link) || el.has_state(ElementState::Visited)
            }
            PseudoClass::Is(l) | PseudoClass::Where(l) => self.list(l, el),
            PseudoClass::Not(l) => !self.list(l, el),
            PseudoClass::Has(l) => self.has(l, el),
            PseudoClass::NthChild(anb, of) | PseudoClass::NthLastChild(anb, of) => {
                if let Some(of) = of
                    && !self.list(of, el)
                {
                    return false;
                }
                let next: fn(&E) -> Option<E> = if matches!(p, PseudoClass::NthChild(..)) {
                    E::prev_sibling_element
                } else {
                    E::next_sibling_element
                };
                let n = match of {
                    Some(l) => self.count_siblings(el, next, &|s| self.list(l, s)),
                    None => self.count_siblings(el, next, &|_| true),
                };
                anb.matches(n)
            }
            PseudoClass::NthOfType(anb) => {
                anb.matches(self.count_siblings(el, E::prev_sibling_element, &|s| same_type(s, el)))
            }
            PseudoClass::NthLastOfType(anb) => {
                anb.matches(self.count_siblings(el, E::next_sibling_element, &|s| same_type(s, el)))
            }
            PseudoClass::FirstChild => el.prev_sibling_element().is_none(),
            PseudoClass::LastChild => el.next_sibling_element().is_none(),
            PseudoClass::OnlyChild => {
                el.prev_sibling_element().is_none() && el.next_sibling_element().is_none()
            }
            PseudoClass::FirstOfType => !sibling_of_type(el, E::prev_sibling_element),
            PseudoClass::LastOfType => !sibling_of_type(el, E::next_sibling_element),
            PseudoClass::OnlyOfType => {
                !sibling_of_type(el, E::prev_sibling_element)
                    && !sibling_of_type(el, E::next_sibling_element)
            }
            PseudoClass::Empty => el.is_empty(),
            PseudoClass::Root => el.is_root(),
            PseudoClass::Scope => self.is_scope(el),
            PseudoClass::Host => self.is_host(el),
            PseudoClass::HostFunction(l) => self.is_host(el) && self.list(l, el),
            PseudoClass::HostContext(l) => {
                if !self.is_host(el) {
                    return false;
                }
                let mut cur = Some(el.clone());
                while let Some(c) = cur {
                    if self.list(l, &c) {
                        return true;
                    }
                    cur = c.parent_element();
                }
                false
            }
            PseudoClass::Lang(ranges) => {
                let lang = el.lang();
                ranges.iter().any(|r| lang_matches(lang.as_deref(), r))
            }
            PseudoClass::Dir(d) => el.direction() == *d,
            PseudoClass::Vendor(_) => false,
        }
    }

    /// `:has()`: walk the anchor's subtree (or its later siblings and their
    /// subtrees) with an explicit stack and test each candidate.
    fn has(&self, l: &SelectorList, el: &E) -> bool {
        for rel in &l.0 {
            if rel.compounds.is_empty() {
                continue;
            }
            let lead = rel.leading.unwrap_or(Combinator::Descendant);
            let all_sibling = rel.combinators.iter().all(|c| c.is_sibling());
            let mut roots = Vec::new();
            if lead.is_sibling() {
                let mut cur = el.next_sibling_element();
                while let Some(s) = cur {
                    cur = s.next_sibling_element();
                    roots.push(s);
                }
            } else {
                let mut cur = el.first_child_element();
                while let Some(s) = cur {
                    cur = s.next_sibling_element();
                    roots.push(s);
                }
            }
            let siblings_only = lead.is_sibling() && all_sibling;
            let one_child = lead == Combinator::Child && rel.compounds.len() == 1;
            let descend = !(siblings_only || one_child);
            let mut stack: Vec<E> = roots.into_iter().rev().collect();
            while let Some(c) = stack.pop() {
                if self.match_from(rel, rel.compounds.len() - 1, &c, Some(el)) == Res::Matched {
                    return true;
                }
                if descend {
                    let mut kids = Vec::new();
                    let mut k = c.first_child_element();
                    while let Some(x) = k {
                        k = x.next_sibling_element();
                        kids.push(x);
                    }
                    stack.extend(kids.into_iter().rev());
                }
            }
        }
        false
    }
}

// ---- rule index ----

/// Which bucket a selector goes into in a [`RuleIndex`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexKey {
    /// Keyed by id.
    Id(String),
    /// Keyed by class.
    Class(String),
    /// Keyed by lowercase type name.
    Type(String),
    /// Tested against every element.
    Other,
}

/// The bucket key for a selector: the rightmost compound's id, else its
/// first class, else its type name, else [`IndexKey::Other`].
pub fn index_key(sel: &Selector) -> IndexKey {
    let Some(c) = sel.rightmost() else {
        return IndexKey::Other;
    };
    let mut class = None;
    let mut ty = None;
    for s in &c.simple {
        match s {
            SimpleSelector::Id(id) => return IndexKey::Id(id.clone()),
            SimpleSelector::Class(k) if class.is_none() => class = Some(k.clone()),
            SimpleSelector::Type { lower, .. } if ty.is_none() => ty = Some(lower.clone()),
            _ => {}
        }
    }
    match (class, ty) {
        (Some(k), _) => IndexKey::Class(k),
        (None, Some(t)) => IndexKey::Type(t),
        (None, None) => IndexKey::Other,
    }
}

/// Selectors bucketed by the most selective key of their rightmost
/// compound. A lookup only tests the selectors whose key the element has,
/// plus the catch-all bucket.
#[derive(Clone, Debug)]
pub struct RuleIndex<T> {
    entries: Vec<(Selector, T)>,
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_type: HashMap<String, Vec<usize>>,
    other: Vec<usize>,
}

impl<T> Default for RuleIndex<T> {
    fn default() -> Self {
        RuleIndex {
            entries: Vec::new(),
            by_id: HashMap::new(),
            by_class: HashMap::new(),
            by_type: HashMap::new(),
            other: Vec::new(),
        }
    }
}

impl<T> RuleIndex<T> {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of selectors stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Size of the catch-all bucket (selectors tested on every element).
    pub fn catch_all_len(&self) -> usize {
        self.other.len()
    }

    /// Add one selector with its payload. Lookups report entries in
    /// insertion order.
    pub fn insert(&mut self, selector: Selector, data: T) {
        let i = self.entries.len();
        match index_key(&selector) {
            IndexKey::Id(k) => self.by_id.entry(k).or_default().push(i),
            IndexKey::Class(k) => self.by_class.entry(k).or_default().push(i),
            IndexKey::Type(k) => self.by_type.entry(k).or_default().push(i),
            IndexKey::Other => self.other.push(i),
        }
        self.entries.push((selector, data));
    }

    /// Entry numbers of selectors that might match `el`, in insertion
    /// order.
    pub fn candidates<E: Element>(&self, el: &E) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        if let Some(v) = el.id().and_then(|id| self.by_id.get(id)) {
            out.extend_from_slice(v);
        }
        el.for_each_class(|c| {
            if let Some(v) = self.by_class.get(c) {
                out.extend_from_slice(v);
            }
        });
        if let Some(v) = self.by_type.get(&el.local_name().to_ascii_lowercase()) {
            out.extend_from_slice(v);
        }
        out.extend_from_slice(&self.other);
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Selectors that match `el` in `ctx`, with their payloads, in
    /// insertion order.
    pub fn matching<E: Element>(&self, el: &E, ctx: &MatchContext<E>) -> Vec<(&Selector, &T)> {
        self.candidates(el)
            .into_iter()
            .filter_map(|i| self.entries.get(i))
            .filter(|(s, _)| s.matches_in(el, ctx))
            .map(|(s, t)| (s, t))
            .collect()
    }

    /// Entry `i`.
    pub fn get(&self, i: usize) -> Option<(&Selector, &T)> {
        self.entries.get(i).map(|(s, t)| (s, t))
    }
}
