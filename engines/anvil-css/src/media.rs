// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Media queries, container queries, and `@supports` conditions.
//!
//! Written from Media Queries Level 4, CSS Conditional Rules Level 3, and
//! CSS Conditional Rules Level 5 (container queries). Conditions are parsed
//! from component values and evaluated against an environment the host
//! fills in: [`MediaEnv`] for media queries, [`ContainerEnv`] for container
//! queries. `@supports` is evaluated against this crate's own property
//! table and selector parser.
//!
//! Media and container conditions use three-valued logic: an unknown
//! feature, or text that only matches `<general-enclosed>`, is "unknown".
//! `not unknown` stays unknown, and an unknown result at the top counts as
//! false. A query that does not parse becomes `not all`, which never
//! matches, and the other queries in the list are unaffected.
//!
//! Condition nesting is limited by [`Limits::max_condition_depth`]. Going
//! past it makes the condition invalid (an error, never a silent cut).

use crate::anvil_css::error::{LimitKind, Limits, Span};
use crate::anvil_css::parser::{BlockKind, ComponentValue};
use crate::anvil_css::selectors::{SelectorOptions, parse_selector_list_from};
use crate::anvil_css::tokenizer::Token;
use crate::anvil_css::values::{self, Calc, LengthUnit, ResolveContext};

/// Why a condition is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionError {
    /// Plain description.
    pub message: String,
    /// Where it went wrong.
    pub span: Span,
    /// The limit that was reached, when a limit caused it.
    pub limit: Option<LimitKind>,
}

fn cerr<T>(message: &str, span: Span) -> Result<T, ConditionError> {
    Err(ConditionError {
        message: message.to_string(),
        span,
        limit: None,
    })
}

/// Three-valued logic result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    /// True.
    True,
    /// False.
    False,
    /// Unknown: treated as false at the top level.
    Unknown,
}

impl Tri {
    fn from_bool(b: bool) -> Tri {
        if b { Tri::True } else { Tri::False }
    }

    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }

    /// True only for [`Tri::True`].
    pub fn is_true(self) -> bool {
        self == Tri::True
    }
}

/// A media type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaType {
    /// `all`
    All,
    /// `screen`
    Screen,
    /// `print`
    Print,
    /// Any other name. The deprecated types (`tv`, `handheld`, and so on)
    /// and unknown names match nothing.
    Other(String),
}

/// `only` or `not` before a media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qualifier {
    /// `only`: no effect on matching.
    Only,
    /// `not`: negates the whole query.
    Not,
}

/// A comparison in a range feature, with the feature on the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `=`
    Eq,
}

impl RangeOp {
    fn flip(self) -> RangeOp {
        match self {
            RangeOp::Lt => RangeOp::Gt,
            RangeOp::Le => RangeOp::Ge,
            RangeOp::Gt => RangeOp::Lt,
            RangeOp::Ge => RangeOp::Le,
            RangeOp::Eq => RangeOp::Eq,
        }
    }

    fn test(self, a: f64, b: f64) -> bool {
        match self {
            RangeOp::Lt => a < b,
            RangeOp::Le => a <= b,
            RangeOp::Gt => a > b,
            RangeOp::Ge => a >= b,
            RangeOp::Eq => (a - b).abs() < 1e-9,
        }
    }
}

/// A media feature value.
#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    /// A number (also used for integers).
    Number(f64),
    /// A length, kept with its unit.
    Length(f64, LengthUnit),
    /// A ratio `a / b`.
    Ratio(f64, f64),
    /// A resolution, in dots per CSS pixel.
    Resolution(f64),
    /// A keyword, lowercased.
    Ident(String),
    /// A math function.
    Calc(Box<Calc>),
}

/// One media or container feature test.
#[derive(Clone, Debug, PartialEq)]
pub enum Feature {
    /// `(name)`: boolean context.
    Boolean(String),
    /// `(name: value)`, with `min-`/`max-` turned into `>=`/`<=`.
    Plain {
        /// Feature name, lowercased, without `min-`/`max-`.
        name: String,
        /// The comparison: `Eq` for a plain feature, `Ge` for `min-`, `Le`
        /// for `max-`.
        op: RangeOp,
        /// The value.
        value: FeatureValue,
    },
    /// Range syntax: every comparison must hold (feature on the left).
    Range {
        /// Feature name, lowercased.
        name: String,
        /// One or two comparisons.
        tests: Vec<(RangeOp, FeatureValue)>,
    },
}

/// A media, container, or general condition tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// `not <in-parens>`
    Not(Box<Condition>),
    /// `a and b and c`
    And(Vec<Condition>),
    /// `a or b or c`
    Or(Vec<Condition>),
    /// A feature test.
    Feature(Feature),
    /// `<general-enclosed>`: always unknown.
    Unknown(Vec<ComponentValue>),
}

/// One media query.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    /// `only` or `not`, when present.
    pub qualifier: Option<Qualifier>,
    /// The media type, when present.
    pub media_type: Option<MediaType>,
    /// The condition, when present.
    pub condition: Option<Condition>,
    /// True when the query did not parse: it behaves as `not all`.
    pub invalid: bool,
}

/// A media query list. An empty list matches everything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaQueryList(pub Vec<MediaQuery>);

/// A container query condition.
pub type ContainerCondition = Condition;

/// `prefers-color-scheme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    /// Light.
    Light,
    /// Dark.
    Dark,
}

/// `pointer` and `any-pointer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    /// No pointing device.
    None,
    /// Limited accuracy (touch).
    Coarse,
    /// Accurate (mouse).
    Fine,
}

/// `prefers-contrast`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contrast {
    /// `no-preference`
    NoPreference,
    /// `more`
    More,
    /// `less`
    Less,
    /// `custom`
    Custom,
}

/// What the host knows about the output device.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaEnv {
    /// `screen` or `print` (other types never match).
    pub media_type: MediaType,
    /// Viewport width in CSS pixels.
    pub width: f64,
    /// Viewport height in CSS pixels.
    pub height: f64,
    /// Screen width in CSS pixels (`device-width`).
    pub device_width: f64,
    /// Screen height in CSS pixels (`device-height`).
    pub device_height: f64,
    /// Device pixels per CSS pixel.
    pub resolution_dppx: f64,
    /// Bits per color component (0 for a monochrome device).
    pub color_bits: u32,
    /// Bits per pixel on a monochrome device (0 otherwise).
    pub monochrome_bits: u32,
    /// `prefers-color-scheme`.
    pub color_scheme: ColorScheme,
    /// `prefers-reduced-motion: reduce` when true.
    pub reduced_motion: bool,
    /// `prefers-reduced-transparency: reduce` when true.
    pub reduced_transparency: bool,
    /// `prefers-contrast`.
    pub contrast: Contrast,
    /// `forced-colors: active` when true.
    pub forced_colors: bool,
    /// `hover: hover` when true.
    pub hover: bool,
    /// `any-hover: hover` when true.
    pub any_hover: bool,
    /// `pointer`.
    pub pointer: Pointer,
    /// `any-pointer`.
    pub any_pointer: Pointer,
    /// `scripting: enabled` when true, `none` otherwise.
    pub scripting: bool,
    /// `display-mode`, lowercased (`browser`, `fullscreen`, ...).
    pub display_mode: String,
    /// The initial font size in pixels, for `em` and `rem` in queries.
    pub font_size: f64,
}

impl Default for MediaEnv {
    fn default() -> Self {
        MediaEnv {
            media_type: MediaType::Screen,
            width: 1280.0,
            height: 800.0,
            device_width: 1280.0,
            device_height: 800.0,
            resolution_dppx: 1.0,
            color_bits: 8,
            monochrome_bits: 0,
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
            reduced_transparency: false,
            contrast: Contrast::NoPreference,
            forced_colors: false,
            hover: true,
            any_hover: true,
            pointer: Pointer::Fine,
            any_pointer: Pointer::Fine,
            scripting: true,
            display_mode: "browser".to_string(),
            font_size: 16.0,
        }
    }
}

/// The size of a query container.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerEnv {
    /// Content-box width in CSS pixels.
    pub width: f64,
    /// Content-box height in CSS pixels.
    pub height: f64,
    /// True for a vertical writing mode (inline size is the height).
    pub vertical: bool,
    /// The container's font size, for `em`.
    pub font_size: f64,
    /// The root font size, for `rem`.
    pub root_font_size: f64,
    /// Viewport width, for `vw`.
    pub viewport_width: f64,
    /// Viewport height, for `vh`.
    pub viewport_height: f64,
}

impl Default for ContainerEnv {
    fn default() -> Self {
        ContainerEnv {
            width: 0.0,
            height: 0.0,
            vertical: false,
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 1280.0,
            viewport_height: 800.0,
        }
    }
}

// ---- parsing ----

fn non_ws(values: &[ComponentValue]) -> Vec<&ComponentValue> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

fn span_of(items: &[&ComponentValue]) -> Span {
    match (items.first(), items.last()) {
        (Some(a), Some(b)) => a.span().to(b.span()),
        _ => Span::default(),
    }
}

struct CondParser<'l> {
    limits: &'l Limits,
}

impl CondParser<'_> {
    fn depth_check(&self, depth: usize, span: Span) -> Result<(), ConditionError> {
        if depth > self.limits.max_condition_depth {
            return Err(ConditionError {
                message: format!(
                    "conditions nest deeper than {} levels",
                    self.limits.max_condition_depth
                ),
                span,
                limit: Some(LimitKind::ConditionDepth),
            });
        }
        Ok(())
    }

    /// `<media-condition>` (or `<media-condition-without-or>`).
    fn condition(
        &self,
        items: &[&ComponentValue],
        allow_or: bool,
        depth: usize,
    ) -> Result<Condition, ConditionError> {
        self.depth_check(depth, span_of(items))?;
        let Some(first) = items.first() else {
            return cerr("empty condition", Span::default());
        };
        if first.is_ident("not") {
            if items.len() != 2 {
                return cerr("'not' takes exactly one condition", span_of(items));
            }
            return Ok(Condition::Not(Box::new(self.in_parens(items[1], depth)?)));
        }
        let head = self.in_parens(first, depth)?;
        if items.len() == 1 {
            return Ok(head);
        }
        let joiner = if items[1].is_ident("and") {
            "and"
        } else if items[1].is_ident("or") {
            if !allow_or {
                return cerr("'or' is not allowed here", items[1].span());
            }
            "or"
        } else {
            return cerr("expected 'and' or 'or'", items[1].span());
        };
        let mut parts = vec![head];
        let mut i = 1;
        while i < items.len() {
            if !items[i].is_ident(joiner) {
                return cerr(
                    "'and' and 'or' cannot be mixed without parentheses",
                    items[i].span(),
                );
            }
            let Some(next) = items.get(i + 1) else {
                return cerr("condition ends with a keyword", items[i].span());
            };
            parts.push(self.in_parens(next, depth)?);
            i += 2;
        }
        Ok(if joiner == "and" {
            Condition::And(parts)
        } else {
            Condition::Or(parts)
        })
    }

    fn in_parens(&self, v: &ComponentValue, depth: usize) -> Result<Condition, ConditionError> {
        match v {
            ComponentValue::Block(b) if b.kind == BlockKind::Paren => {
                self.depth_check(depth + 1, b.span)?;
                let inner = non_ws(&b.contents);
                match self.condition(&inner, true, depth + 1) {
                    Ok(c) => return Ok(c),
                    Err(e) if e.limit.is_some() => return Err(e),
                    Err(_) => {}
                }
                match self.feature(&b.contents, depth + 1) {
                    Ok(f) => Ok(Condition::Feature(f)),
                    Err(e) if e.limit.is_some() => Err(e),
                    Err(_) => Ok(Condition::Unknown(b.contents.clone())),
                }
            }
            ComponentValue::Function(_) => Ok(Condition::Unknown(vec![v.clone()])),
            _ => cerr("expected a parenthesized condition", v.span()),
        }
    }

    fn feature(
        &self,
        contents: &[ComponentValue],
        depth: usize,
    ) -> Result<Feature, ConditionError> {
        let items = non_ws(contents);
        let span = span_of(&items);
        match items.as_slice() {
            [] => return cerr("empty feature", span),
            [one] => {
                return match one.token() {
                    Some(Token::Ident(n)) => Ok(Feature::Boolean(n.to_ascii_lowercase())),
                    _ => cerr("expected a feature name", span),
                };
            }
            [name, colon, rest @ ..] if matches!(colon.token(), Some(Token::Colon)) => {
                let Some(Token::Ident(n)) = name.token() else {
                    return cerr("expected a feature name", span);
                };
                let lower = n.to_ascii_lowercase();
                let value = self.value(rest, depth)?;
                let (name, op) = if let Some(base) = lower.strip_prefix("min-") {
                    (base.to_string(), RangeOp::Ge)
                } else if let Some(base) = lower.strip_prefix("max-") {
                    (base.to_string(), RangeOp::Le)
                } else {
                    (lower, RangeOp::Eq)
                };
                return Ok(Feature::Plain { name, op, value });
            }
            _ => {}
        }
        // Range syntax: split on comparison operators.
        let mut segments: Vec<Vec<&ComponentValue>> = vec![Vec::new()];
        let mut ops = Vec::new();
        let mut i = 0;
        while i < items.len() {
            let v = items[i];
            let op = match v.token() {
                Some(Token::Delim(c @ ('<' | '>'))) => {
                    let eq = items
                        .get(i + 1)
                        .is_some_and(|n| n.is_delim('=') && n.span().start == v.span().end);
                    if eq {
                        i += 1;
                    }
                    Some(match (c, eq) {
                        ('<', false) => RangeOp::Lt,
                        ('<', true) => RangeOp::Le,
                        ('>', false) => RangeOp::Gt,
                        _ => RangeOp::Ge,
                    })
                }
                Some(Token::Delim('=')) => Some(RangeOp::Eq),
                _ => None,
            };
            match op {
                Some(op) => {
                    ops.push(op);
                    segments.push(Vec::new());
                }
                None => {
                    if let Some(s) = segments.last_mut() {
                        s.push(v);
                    }
                }
            }
            i += 1;
        }
        let ident = |seg: &[&ComponentValue]| -> Option<String> {
            match seg {
                [one] => match one.token() {
                    Some(Token::Ident(n)) => Some(n.to_ascii_lowercase()),
                    _ => None,
                },
                _ => None,
            }
        };
        match (segments.as_slice(), ops.as_slice()) {
            ([left, right], [op]) => {
                if let Some(name) = ident(left) {
                    let value = self.value(right, depth)?;
                    Ok(Feature::Range {
                        name,
                        tests: vec![(*op, value)],
                    })
                } else if let Some(name) = ident(right) {
                    let value = self.value(left, depth)?;
                    Ok(Feature::Range {
                        name,
                        tests: vec![(op.flip(), value)],
                    })
                } else {
                    cerr("range needs a feature name", span)
                }
            }
            ([low, mid, high], [op1, op2]) => {
                let less = |o: &RangeOp| matches!(o, RangeOp::Lt | RangeOp::Le);
                let more = |o: &RangeOp| matches!(o, RangeOp::Gt | RangeOp::Ge);
                if !((less(op1) && less(op2)) || (more(op1) && more(op2))) {
                    return cerr("both comparisons must point the same way", span);
                }
                let Some(name) = ident(mid) else {
                    return cerr("range needs a feature name in the middle", span);
                };
                let a = self.value(low, depth)?;
                let b = self.value(high, depth)?;
                Ok(Feature::Range {
                    name,
                    tests: vec![(op1.flip(), a), (*op2, b)],
                })
            }
            _ => cerr("invalid media feature", span),
        }
    }

    fn value(
        &self,
        items: &[&ComponentValue],
        depth: usize,
    ) -> Result<FeatureValue, ConditionError> {
        let span = span_of(items);
        match items {
            [one] => match one {
                ComponentValue::Token(t) => match &t.token {
                    Token::Number(n) => Ok(FeatureValue::Number(n.value)),
                    Token::Ident(s) => Ok(FeatureValue::Ident(s.to_ascii_lowercase())),
                    Token::Dimension { value, unit } => {
                        if let Some(u) = LengthUnit::from_name(unit) {
                            Ok(FeatureValue::Length(value.value, u))
                        } else if let Some(dppx) = values::resolution_to_dppx(value.value, unit) {
                            Ok(FeatureValue::Resolution(dppx))
                        } else {
                            cerr("unknown unit in media feature", span)
                        }
                    }
                    _ => cerr("invalid media feature value", span),
                },
                ComponentValue::Function(_) => {
                    let mut limits = self.limits.clone();
                    limits.max_value_depth = limits.max_value_depth.saturating_sub(depth);
                    match values::parse_math(one, &limits) {
                        Ok(c) => Ok(FeatureValue::Calc(Box::new(c))),
                        Err(e) => Err(ConditionError {
                            message: e.message,
                            span,
                            limit: e.limit,
                        }),
                    }
                }
                ComponentValue::Block(_) => cerr("invalid media feature value", span),
            },
            [a, slash, b] if slash.is_delim('/') => match (a.token(), b.token()) {
                (Some(Token::Number(x)), Some(Token::Number(y))) => {
                    Ok(FeatureValue::Ratio(x.value, y.value))
                }
                _ => cerr("invalid ratio", span),
            },
            _ => cerr("invalid media feature value", span),
        }
    }
}

/// Parse a media query list. Never fails: a query that does not parse is
/// kept with `invalid: true` and never matches.
pub fn parse_media_query_list(values: &[ComponentValue], limits: &Limits) -> MediaQueryList {
    parse_media_query_list_checked(values, limits).0
}

/// Parse a media query list and also return why each invalid query was
/// rejected. Invalid queries still become `not all` in the list.
pub fn parse_media_query_list_checked(
    values: &[ComponentValue],
    limits: &Limits,
) -> (MediaQueryList, Vec<ConditionError>) {
    let items = non_ws(values);
    if items.is_empty() {
        return (MediaQueryList(Vec::new()), Vec::new());
    }
    let p = CondParser { limits };
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for group in values.split(|v| matches!(v.token(), Some(Token::Comma))) {
        let items = non_ws(group);
        out.push(match parse_query(&p, &items) {
            Ok(q) => q,
            Err(e) => {
                errors.push(e);
                MediaQuery {
                    qualifier: None,
                    media_type: None,
                    condition: None,
                    invalid: true,
                }
            }
        });
    }
    (MediaQueryList(out), errors)
}

/// Parse a media query list from text.
pub fn parse_media_query_list_str(src: &str) -> MediaQueryList {
    let options = crate::anvil_css::parser::Options::default();
    match crate::anvil_css::parser::parse_component_values(src, &options) {
        Ok(v) => parse_media_query_list(&v, &options.limits),
        Err(_) => MediaQueryList(vec![MediaQuery {
            qualifier: None,
            media_type: None,
            condition: None,
            invalid: true,
        }]),
    }
}

fn parse_query(
    p: &CondParser<'_>,
    items: &[&ComponentValue],
) -> Result<MediaQuery, ConditionError> {
    let span = span_of(items);
    let Some(first) = items.first() else {
        return cerr("empty media query", span);
    };
    let mut q = MediaQuery {
        qualifier: None,
        media_type: None,
        condition: None,
        invalid: false,
    };
    let starts_condition = match first {
        ComponentValue::Block(_) | ComponentValue::Function(_) => true,
        _ => {
            first.is_ident("not")
                && matches!(
                    items.get(1),
                    Some(ComponentValue::Block(_) | ComponentValue::Function(_))
                )
        }
    };
    if starts_condition {
        q.condition = Some(p.condition(items, true, 0)?);
        return Ok(q);
    }
    let mut i = 0;
    if first.is_ident("only") {
        q.qualifier = Some(Qualifier::Only);
        i = 1;
    } else if first.is_ident("not") {
        q.qualifier = Some(Qualifier::Not);
        i = 1;
    }
    let Some(Token::Ident(ty)) = items.get(i).and_then(|v| v.token()) else {
        return cerr("expected a media type", span);
    };
    let lower = ty.to_ascii_lowercase();
    if ["only", "not", "and", "or", "layer"].contains(&lower.as_str()) {
        return cerr("reserved word used as a media type", span);
    }
    q.media_type = Some(match lower.as_str() {
        "all" => MediaType::All,
        "screen" => MediaType::Screen,
        "print" => MediaType::Print,
        _ => MediaType::Other(lower),
    });
    i += 1;
    if i < items.len() {
        if !items[i].is_ident("and") {
            return cerr("expected 'and' after the media type", items[i].span());
        }
        let rest = &items[i + 1..];
        if rest.is_empty() {
            return cerr("media query ends with 'and'", items[i].span());
        }
        q.condition = Some(p.condition(rest, false, 0)?);
    }
    Ok(q)
}

/// Parse an `@container` prelude: an optional name, then an optional
/// condition. At least one must be present.
pub fn parse_container_prelude(
    values: &[ComponentValue],
    limits: &Limits,
) -> Result<(Option<String>, Option<ContainerCondition>), ConditionError> {
    let items = non_ws(values);
    let span = span_of(&items);
    let mut i = 0;
    let mut name = None;
    if let Some(Token::Ident(n)) = items.first().and_then(|v| v.token()) {
        let lower = n.to_ascii_lowercase();
        if ["not", "and", "or", "none"].contains(&lower.as_str())
            || crate::anvil_css::parser::is_css_wide_keyword(n)
        {
            if lower != "not" {
                return cerr("reserved word used as a container name", span);
            }
        } else {
            name = Some(n.clone());
            i = 1;
        }
    }
    let rest = &items[i..];
    if rest.is_empty() {
        if name.is_none() {
            return cerr("@container needs a name or a condition", span);
        }
        return Ok((name, None));
    }
    let p = CondParser { limits };
    Ok((name, Some(p.condition(rest, true, 0)?)))
}

/// An `@supports` condition.
#[derive(Clone, Debug, PartialEq)]
pub enum SupportsCondition {
    /// `not (...)`
    Not(Box<SupportsCondition>),
    /// `(...) and (...)`
    And(Vec<SupportsCondition>),
    /// `(...) or (...)`
    Or(Vec<SupportsCondition>),
    /// `(name: value)`
    Declaration {
        /// Property name, lowercased unless custom.
        name: String,
        /// The value.
        value: Vec<ComponentValue>,
    },
    /// `selector(...)`
    Selector(Vec<ComponentValue>),
    /// `<general-enclosed>`, including `font-tech()` and `font-format()`:
    /// evaluates to false.
    Unknown(Vec<ComponentValue>),
}

/// Parse an `@supports` condition.
pub fn parse_supports_condition(
    values: &[ComponentValue],
    limits: &Limits,
) -> Result<SupportsCondition, ConditionError> {
    supports_condition(&non_ws(values), limits, 0)
}

fn supports_condition(
    items: &[&ComponentValue],
    limits: &Limits,
    depth: usize,
) -> Result<SupportsCondition, ConditionError> {
    let span = span_of(items);
    if depth > limits.max_condition_depth {
        return Err(ConditionError {
            message: format!(
                "conditions nest deeper than {} levels",
                limits.max_condition_depth
            ),
            span,
            limit: Some(LimitKind::ConditionDepth),
        });
    }
    let Some(first) = items.first() else {
        return cerr("empty @supports condition", span);
    };
    if first.is_ident("not") {
        if items.len() != 2 {
            return cerr("'not' takes exactly one condition", span);
        }
        return Ok(SupportsCondition::Not(Box::new(supports_in_parens(
            items[1], limits, depth,
        )?)));
    }
    let head = supports_in_parens(first, limits, depth)?;
    if items.len() == 1 {
        return Ok(head);
    }
    let joiner = if items[1].is_ident("and") {
        "and"
    } else if items[1].is_ident("or") {
        "or"
    } else {
        return cerr("expected 'and' or 'or'", items[1].span());
    };
    let mut parts = vec![head];
    let mut i = 1;
    while i < items.len() {
        if !items[i].is_ident(joiner) {
            return cerr(
                "'and' and 'or' cannot be mixed without parentheses",
                items[i].span(),
            );
        }
        let Some(next) = items.get(i + 1) else {
            return cerr("condition ends with a keyword", items[i].span());
        };
        parts.push(supports_in_parens(next, limits, depth)?);
        i += 2;
    }
    Ok(if joiner == "and" {
        SupportsCondition::And(parts)
    } else {
        SupportsCondition::Or(parts)
    })
}

fn supports_in_parens(
    v: &ComponentValue,
    limits: &Limits,
    depth: usize,
) -> Result<SupportsCondition, ConditionError> {
    match v {
        ComponentValue::Block(b) if b.kind == BlockKind::Paren => {
            let inner = non_ws(&b.contents);
            match supports_condition(&inner, limits, depth + 1) {
                Ok(c) => return Ok(c),
                Err(e) if e.limit.is_some() => return Err(e),
                Err(_) => {}
            }
            Ok(supports_declaration_from(&b.contents, limits)
                .unwrap_or_else(|| SupportsCondition::Unknown(b.contents.clone())))
        }
        ComponentValue::Function(f) if f.name.eq_ignore_ascii_case("selector") => {
            Ok(SupportsCondition::Selector(f.arguments.clone()))
        }
        ComponentValue::Function(_) => Ok(SupportsCondition::Unknown(vec![v.clone()])),
        _ => cerr("expected a parenthesized condition", v.span()),
    }
}

/// Read `name: value` as a supports declaration.
pub fn supports_declaration_from(
    values: &[ComponentValue],
    _limits: &Limits,
) -> Option<SupportsCondition> {
    let mut i = 0;
    while values.get(i).is_some_and(|v| v.is_whitespace()) {
        i += 1;
    }
    let Some(Token::Ident(name)) = values.get(i).and_then(|v| v.token()) else {
        return None;
    };
    i += 1;
    while values.get(i).is_some_and(|v| v.is_whitespace()) {
        i += 1;
    }
    if !matches!(values.get(i).and_then(|v| v.token()), Some(Token::Colon)) {
        return None;
    }
    let mut value: Vec<ComponentValue> = values[i + 1..].to_vec();
    while value.first().is_some_and(|v| v.is_whitespace()) {
        value.remove(0);
    }
    while value.last().is_some_and(|v| v.is_whitespace()) {
        value.pop();
    }
    // A trailing `!important` is part of declaration syntax and does not
    // change whether the declaration is supported.
    if value.last().is_some_and(|v| v.is_ident("important")) {
        let mut j = value.len() - 1;
        while j > 0 && value[j - 1].is_whitespace() {
            j -= 1;
        }
        if j > 0 && value[j - 1].is_delim('!') {
            value.truncate(j - 1);
            while value.last().is_some_and(|v| v.is_whitespace()) {
                value.pop();
            }
        }
    }
    let name = if name.starts_with("--") {
        name.clone()
    } else {
        name.to_ascii_lowercase()
    };
    Some(SupportsCondition::Declaration { name, value })
}

// ---- evaluation ----

enum EnvValue {
    Length(f64),
    Resolution(f64),
    Integer(f64),
    Ratio(f64),
    Keyword(&'static str, bool),
}

fn keyword_value(v: &FeatureValue) -> Option<&str> {
    match v {
        FeatureValue::Ident(s) => Some(s),
        _ => None,
    }
}

fn numeric(env_value: &EnvValue, v: &FeatureValue, ctx: &ResolveContext) -> Option<(f64, f64)> {
    match (env_value, v) {
        (EnvValue::Length(x), FeatureValue::Length(n, u)) => Some((*x, u.to_px(*n, ctx))),
        (EnvValue::Length(x), FeatureValue::Number(n)) if *n == 0.0 => Some((*x, 0.0)),
        (EnvValue::Length(x), FeatureValue::Calc(c)) if c.ty == values::CalcType::Length => {
            Some((*x, c.resolve(ctx)))
        }
        (EnvValue::Resolution(x), FeatureValue::Resolution(r)) => Some((*x, *r)),
        (EnvValue::Resolution(x), FeatureValue::Calc(c))
            if c.ty == values::CalcType::Resolution =>
        {
            Some((*x, c.resolve(ctx)))
        }
        (EnvValue::Integer(x), FeatureValue::Number(n)) => Some((*x, *n)),
        (EnvValue::Integer(x), FeatureValue::Calc(c)) if c.ty == values::CalcType::Number => {
            Some((*x, c.resolve(ctx)))
        }
        (EnvValue::Ratio(x), FeatureValue::Ratio(a, b)) => {
            let r = if *b == 0.0 { f64::INFINITY } else { a / b };
            Some((*x, r))
        }
        (EnvValue::Ratio(x), FeatureValue::Number(n)) => Some((*x, *n)),
        _ => None,
    }
}

fn eval_feature(
    f: &Feature,
    lookup: &dyn Fn(&str) -> Option<EnvValue>,
    ctx: &ResolveContext,
) -> Tri {
    match f {
        Feature::Boolean(name) => match lookup(name) {
            None => Tri::Unknown,
            Some(EnvValue::Keyword(_, truthy)) => Tri::from_bool(truthy),
            Some(
                EnvValue::Length(x)
                | EnvValue::Resolution(x)
                | EnvValue::Integer(x)
                | EnvValue::Ratio(x),
            ) => Tri::from_bool(x != 0.0),
        },
        Feature::Plain { name, op, value } => match lookup(name) {
            None => Tri::Unknown,
            Some(EnvValue::Keyword(k, _)) => {
                if *op != RangeOp::Eq {
                    return Tri::Unknown;
                }
                match keyword_value(value) {
                    Some(v) => Tri::from_bool(v == k),
                    None => Tri::Unknown,
                }
            }
            Some(ev) => match numeric(&ev, value, ctx) {
                Some((a, b)) => Tri::from_bool(op.test(a, b)),
                None => Tri::Unknown,
            },
        },
        Feature::Range { name, tests } => match lookup(name) {
            None | Some(EnvValue::Keyword(..)) => Tri::Unknown,
            Some(ev) => {
                let mut result = true;
                for (op, value) in tests {
                    match numeric(&ev, value, ctx) {
                        Some((a, b)) => result &= op.test(a, b),
                        None => return Tri::Unknown,
                    }
                }
                Tri::from_bool(result)
            }
        },
    }
}

fn eval_condition(
    c: &Condition,
    lookup: &dyn Fn(&str) -> Option<EnvValue>,
    ctx: &ResolveContext,
) -> Tri {
    match c {
        Condition::Not(inner) => eval_condition(inner, lookup, ctx).not(),
        Condition::And(parts) => {
            let mut out = Tri::True;
            for p in parts {
                match eval_condition(p, lookup, ctx) {
                    Tri::False => return Tri::False,
                    Tri::Unknown => out = Tri::Unknown,
                    Tri::True => {}
                }
            }
            out
        }
        Condition::Or(parts) => {
            let mut out = Tri::False;
            for p in parts {
                match eval_condition(p, lookup, ctx) {
                    Tri::True => return Tri::True,
                    Tri::Unknown => out = Tri::Unknown,
                    Tri::False => {}
                }
            }
            out
        }
        Condition::Feature(f) => eval_feature(f, lookup, ctx),
        Condition::Unknown(_) => Tri::Unknown,
    }
}

fn pointer_name(p: Pointer) -> &'static str {
    match p {
        Pointer::None => "none",
        Pointer::Coarse => "coarse",
        Pointer::Fine => "fine",
    }
}

impl MediaEnv {
    fn resolve_context(&self) -> ResolveContext {
        ResolveContext {
            font_size: self.font_size,
            root_font_size: self.font_size,
            viewport_width: self.width,
            viewport_height: self.height,
            ..ResolveContext::default()
        }
    }

    fn lookup(&self, name: &str) -> Option<EnvValue> {
        Some(match name {
            "width" => EnvValue::Length(self.width),
            "height" => EnvValue::Length(self.height),
            "device-width" => EnvValue::Length(self.device_width),
            "device-height" => EnvValue::Length(self.device_height),
            "aspect-ratio" => EnvValue::Ratio(if self.height == 0.0 {
                f64::INFINITY
            } else {
                self.width / self.height
            }),
            "device-aspect-ratio" => EnvValue::Ratio(if self.device_height == 0.0 {
                f64::INFINITY
            } else {
                self.device_width / self.device_height
            }),
            "orientation" => {
                if self.height >= self.width {
                    EnvValue::Keyword("portrait", true)
                } else {
                    EnvValue::Keyword("landscape", true)
                }
            }
            "resolution" => EnvValue::Resolution(self.resolution_dppx),
            "color" => EnvValue::Integer(f64::from(self.color_bits)),
            "color-index" => EnvValue::Integer(0.0),
            "monochrome" => EnvValue::Integer(f64::from(self.monochrome_bits)),
            "grid" => EnvValue::Integer(0.0),
            "color-gamut" => EnvValue::Keyword("srgb", true),
            "update" => {
                if self.media_type == MediaType::Print {
                    EnvValue::Keyword("none", false)
                } else {
                    EnvValue::Keyword("fast", true)
                }
            }
            "scan" => EnvValue::Keyword("progressive", true),
            "overflow-block" => {
                if self.media_type == MediaType::Print {
                    EnvValue::Keyword("paged", true)
                } else {
                    EnvValue::Keyword("scroll", true)
                }
            }
            "overflow-inline" => EnvValue::Keyword("scroll", true),
            "prefers-color-scheme" => match self.color_scheme {
                ColorScheme::Light => EnvValue::Keyword("light", true),
                ColorScheme::Dark => EnvValue::Keyword("dark", true),
            },
            "prefers-reduced-motion" => {
                if self.reduced_motion {
                    EnvValue::Keyword("reduce", true)
                } else {
                    EnvValue::Keyword("no-preference", false)
                }
            }
            "prefers-reduced-transparency" => {
                if self.reduced_transparency {
                    EnvValue::Keyword("reduce", true)
                } else {
                    EnvValue::Keyword("no-preference", false)
                }
            }
            "prefers-contrast" => match self.contrast {
                Contrast::NoPreference => EnvValue::Keyword("no-preference", false),
                Contrast::More => EnvValue::Keyword("more", true),
                Contrast::Less => EnvValue::Keyword("less", true),
                Contrast::Custom => EnvValue::Keyword("custom", true),
            },
            "forced-colors" => {
                if self.forced_colors {
                    EnvValue::Keyword("active", true)
                } else {
                    EnvValue::Keyword("none", false)
                }
            }
            "hover" => {
                if self.hover {
                    EnvValue::Keyword("hover", true)
                } else {
                    EnvValue::Keyword("none", false)
                }
            }
            "any-hover" => {
                if self.any_hover {
                    EnvValue::Keyword("hover", true)
                } else {
                    EnvValue::Keyword("none", false)
                }
            }
            "pointer" => {
                EnvValue::Keyword(pointer_name(self.pointer), self.pointer != Pointer::None)
            }
            "any-pointer" => EnvValue::Keyword(
                pointer_name(self.any_pointer),
                self.any_pointer != Pointer::None,
            ),
            "scripting" => {
                if self.scripting {
                    EnvValue::Keyword("enabled", true)
                } else {
                    EnvValue::Keyword("none", false)
                }
            }
            "display-mode" => {
                let mode: &'static str = match self.display_mode.as_str() {
                    "fullscreen" => "fullscreen",
                    "standalone" => "standalone",
                    "minimal-ui" => "minimal-ui",
                    "picture-in-picture" => "picture-in-picture",
                    _ => "browser",
                };
                EnvValue::Keyword(mode, true)
            }
            _ => return None,
        })
    }
}

impl MediaQuery {
    /// Evaluate against `env`.
    pub fn evaluate(&self, env: &MediaEnv) -> bool {
        self.evaluate_tri(env).is_true()
    }

    /// Evaluate with three-valued logic.
    pub fn evaluate_tri(&self, env: &MediaEnv) -> Tri {
        if self.invalid {
            return Tri::False;
        }
        let type_ok = match &self.media_type {
            None | Some(MediaType::All) => true,
            Some(MediaType::Other(_)) => false,
            Some(t) => *t == env.media_type,
        };
        let ctx = env.resolve_context();
        let lookup = |n: &str| env.lookup(n);
        let cond = match &self.condition {
            Some(c) => eval_condition(c, &lookup, &ctx),
            None => Tri::True,
        };
        let mut r = if !type_ok { Tri::False } else { cond };
        if self.qualifier == Some(Qualifier::Not) {
            r = r.not();
        }
        r
    }
}

impl MediaQueryList {
    /// True when the list is empty or any query matches.
    pub fn evaluate(&self, env: &MediaEnv) -> bool {
        self.0.is_empty() || self.0.iter().any(|q| q.evaluate(env))
    }
}

impl Condition {
    /// Evaluate as a container condition against `env`.
    pub fn evaluate_container(&self, env: &ContainerEnv) -> Tri {
        let ctx = ResolveContext {
            font_size: env.font_size,
            root_font_size: env.root_font_size,
            viewport_width: env.viewport_width,
            viewport_height: env.viewport_height,
            container_width: env.width,
            container_height: env.height,
            ..ResolveContext::default()
        };
        let (inline, block) = if env.vertical {
            (env.height, env.width)
        } else {
            (env.width, env.height)
        };
        let lookup = |n: &str| -> Option<EnvValue> {
            Some(match n {
                "width" => EnvValue::Length(env.width),
                "height" => EnvValue::Length(env.height),
                "inline-size" => EnvValue::Length(inline),
                "block-size" => EnvValue::Length(block),
                "aspect-ratio" => EnvValue::Ratio(if env.height == 0.0 {
                    f64::INFINITY
                } else {
                    env.width / env.height
                }),
                "orientation" => {
                    if env.height >= env.width {
                        EnvValue::Keyword("portrait", true)
                    } else {
                        EnvValue::Keyword("landscape", true)
                    }
                }
                _ => return None,
            })
        };
        eval_condition(self, &lookup, &ctx)
    }

    /// Evaluate as a media condition against `env`.
    pub fn evaluate_media(&self, env: &MediaEnv) -> Tri {
        let ctx = env.resolve_context();
        let lookup = |n: &str| env.lookup(n);
        eval_condition(self, &lookup, &ctx)
    }
}

impl SupportsCondition {
    /// Evaluate against this crate's property table and selector parser.
    /// A declaration is supported when its property is known and the value
    /// parses. A custom property declaration is always supported.
    pub fn evaluate(&self) -> bool {
        match self {
            SupportsCondition::Not(c) => !c.evaluate(),
            SupportsCondition::And(parts) => parts.iter().all(|p| p.evaluate()),
            SupportsCondition::Or(parts) => parts.iter().any(|p| p.evaluate()),
            SupportsCondition::Declaration { name, value } => {
                if name.starts_with("--") {
                    return true;
                }
                values::parse_value_from(name, value, &Limits::default()).is_ok()
            }
            SupportsCondition::Selector(args) => {
                match parse_selector_list_from(args, &SelectorOptions::default()) {
                    Ok(list) => list.0.len() == 1,
                    Err(_) => false,
                }
            }
            SupportsCondition::Unknown(_) => false,
        }
    }
}
