// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Typed values: units, colors, math functions, and property values.
//!
//! Written from CSS Values and Units Level 4, CSS Color Level 4/5, and the
//! property definitions in each CSS module. [`parse_value`] reads a
//! property's value text and returns a [`PropertyValue`]:
//!
//! - a CSS-wide keyword (`inherit`, `initial`, `unset`, `revert`,
//!   `revert-layer`) alone becomes [`PropertyValue::Global`];
//! - a custom property (`--name`) keeps its token stream as
//!   [`PropertyValue::Custom`];
//! - any value that uses `var()`, `env()`, or `attr()` is kept as
//!   [`PropertyValue::Unresolved`] for substitution at computed-value time;
//! - everything else is checked against the property's grammar in
//!   [`crate::prism::properties`] and becomes a list of [`Value`]s.
//!
//! Math functions (`calc()`, `min()`, `max()`, `clamp()`, `round()`, `mod()`,
//! `rem()`, `sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`,
//! `atan2()`, `pow()`, `sqrt()`, `hypot()`, `log()`, `exp()`, `abs()`,
//! `sign()`) become a typed [`CalcNode`] tree. Parsing simplifies it: like
//! units are added, absolute lengths become `px`, angles `deg`, times `s`,
//! and number factors are multiplied in. Relative units stay symbolic until
//! [`Calc::resolve`] is given a [`ResolveContext`].
//!
//! Approximations, all documented on the items involved: `ex`, `ch`, and
//! `cap` use fixed fractions of `em` when no font metrics are available;
//! the small, large, and dynamic viewport units use the one viewport size;
//! colors outside sRGB are clipped per channel, not gamut-mapped.

use crate::prism::error::{LimitKind, Limits, Span};
use crate::prism::parser::{BlockKind, ComponentValue, Options};
use crate::prism::properties::{self, Grammar};
use crate::prism::tokenizer::{Token, Tokenizer};

/// Why a value is invalid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueError {
    /// Plain description.
    pub message: String,
    /// Where it went wrong.
    pub span: Span,
    /// The limit that was reached, when a limit caused it.
    pub limit: Option<LimitKind>,
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid value at bytes {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for ValueError {}

fn verr<T>(message: impl Into<String>, span: Span) -> Result<T, ValueError> {
    Err(ValueError {
        message: message.into(),
        span,
        limit: None,
    })
}

macro_rules! units {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $(
                #[doc = concat!("`", $text, "`")]
                $variant,
            )*
        }

        impl $name {
            /// Look up a unit name, ignoring ASCII case.
            pub fn from_name(s: &str) -> Option<Self> {
                $(
                    if s.eq_ignore_ascii_case($text) {
                        return Some($name::$variant);
                    }
                )*
                None
            }

            /// The unit as written in CSS.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }
        }
    };
}

units! {
    /// Length units.
    LengthUnit {
        Px = "px",
        Cm = "cm",
        Mm = "mm",
        Q = "q",
        In = "in",
        Pt = "pt",
        Pc = "pc",
        Em = "em",
        Rem = "rem",
        Ex = "ex",
        Rex = "rex",
        Ch = "ch",
        Rch = "rch",
        Cap = "cap",
        Rcap = "rcap",
        Ic = "ic",
        Ric = "ric",
        Lh = "lh",
        Rlh = "rlh",
        Vw = "vw",
        Vh = "vh",
        Vi = "vi",
        Vb = "vb",
        Vmin = "vmin",
        Vmax = "vmax",
        Svw = "svw",
        Svh = "svh",
        Svi = "svi",
        Svb = "svb",
        Svmin = "svmin",
        Svmax = "svmax",
        Lvw = "lvw",
        Lvh = "lvh",
        Lvi = "lvi",
        Lvb = "lvb",
        Lvmin = "lvmin",
        Lvmax = "lvmax",
        Dvw = "dvw",
        Dvh = "dvh",
        Dvi = "dvi",
        Dvb = "dvb",
        Dvmin = "dvmin",
        Dvmax = "dvmax",
        Cqw = "cqw",
        Cqh = "cqh",
        Cqi = "cqi",
        Cqb = "cqb",
        Cqmin = "cqmin",
        Cqmax = "cqmax",
    }
}

units! {
    /// Angle units.
    AngleUnit {
        Deg = "deg",
        Grad = "grad",
        Rad = "rad",
        Turn = "turn",
    }
}

units! {
    /// Time units.
    TimeUnit {
        S = "s",
        Ms = "ms",
    }
}

units! {
    /// Frequency units.
    FrequencyUnit {
        Hz = "hz",
        Khz = "khz",
    }
}

units! {
    /// Resolution units.
    ResolutionUnit {
        Dpi = "dpi",
        Dpcm = "dpcm",
        Dppx = "dppx",
        X = "x",
    }
}

/// Inputs for turning relative units and percentages into numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolveContext {
    /// The element's font size in px (`em`).
    pub font_size: f64,
    /// The root element's font size in px (`rem`).
    pub root_font_size: f64,
    /// The element's line height in px (`lh`).
    pub line_height: f64,
    /// The root element's line height in px (`rlh`).
    pub root_line_height: f64,
    /// Viewport width in px. All viewport unit sizes (small, large,
    /// dynamic) use this one value.
    pub viewport_width: f64,
    /// Viewport height in px.
    pub viewport_height: f64,
    /// Query container width in px (`cqw`).
    pub container_width: f64,
    /// Query container height in px (`cqh`).
    pub container_height: f64,
    /// True for a vertical writing mode (`vi`, `vb`, `cqi`, `cqb` swap).
    pub vertical: bool,
    /// What `100%` means, in the result's canonical unit. `None` resolves
    /// percentages to 0.
    pub percent_basis: Option<f64>,
}

impl Default for ResolveContext {
    fn default() -> Self {
        ResolveContext {
            font_size: 16.0,
            root_font_size: 16.0,
            line_height: 19.2,
            root_line_height: 19.2,
            viewport_width: 1280.0,
            viewport_height: 800.0,
            container_width: 0.0,
            container_height: 0.0,
            vertical: false,
            percent_basis: None,
        }
    }
}

/// `ex` and `ch` as a fraction of `em`, used because no font metrics are
/// available here (the CSS fallback for both is 0.5em).
pub const EX_PER_EM: f64 = 0.5;
/// `cap` as a fraction of `em`, used because no font metrics are available
/// here.
pub const CAP_PER_EM: f64 = 0.7;

impl LengthUnit {
    /// Pixels per unit for absolute units, `None` for relative ones.
    pub fn absolute_px(self) -> Option<f64> {
        Some(match self {
            LengthUnit::Px => 1.0,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Q => 96.0 / 101.6,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            _ => return None,
        })
    }

    /// Convert `value` of this unit to px.
    pub fn to_px(self, value: f64, ctx: &ResolveContext) -> f64 {
        use LengthUnit::*;
        if let Some(f) = self.absolute_px() {
            return value * f;
        }
        let (vi, vb) = if ctx.vertical {
            (ctx.viewport_height, ctx.viewport_width)
        } else {
            (ctx.viewport_width, ctx.viewport_height)
        };
        let (ci, cb) = if ctx.vertical {
            (ctx.container_height, ctx.container_width)
        } else {
            (ctx.container_width, ctx.container_height)
        };
        let vmin = ctx.viewport_width.min(ctx.viewport_height);
        let vmax = ctx.viewport_width.max(ctx.viewport_height);
        let per = match self {
            Em => ctx.font_size,
            Rem => ctx.root_font_size,
            Ex | Ch => ctx.font_size * EX_PER_EM,
            Rex | Rch => ctx.root_font_size * EX_PER_EM,
            Cap => ctx.font_size * CAP_PER_EM,
            Rcap => ctx.root_font_size * CAP_PER_EM,
            Ic => ctx.font_size,
            Ric => ctx.root_font_size,
            Lh => ctx.line_height,
            Rlh => ctx.root_line_height,
            Vw | Svw | Lvw | Dvw => ctx.viewport_width / 100.0,
            Vh | Svh | Lvh | Dvh => ctx.viewport_height / 100.0,
            Vi | Svi | Lvi | Dvi => vi / 100.0,
            Vb | Svb | Lvb | Dvb => vb / 100.0,
            Vmin | Svmin | Lvmin | Dvmin => vmin / 100.0,
            Vmax | Svmax | Lvmax | Dvmax => vmax / 100.0,
            Cqw => ctx.container_width / 100.0,
            Cqh => ctx.container_height / 100.0,
            Cqi => ci / 100.0,
            Cqb => cb / 100.0,
            Cqmin => ctx.container_width.min(ctx.container_height) / 100.0,
            Cqmax => ctx.container_width.max(ctx.container_height) / 100.0,
            Px | Cm | Mm | Q | In | Pt | Pc => 1.0,
        };
        value * per
    }
}

impl AngleUnit {
    /// Degrees per unit.
    pub fn degrees(self) -> f64 {
        match self {
            AngleUnit::Deg => 1.0,
            AngleUnit::Grad => 0.9,
            AngleUnit::Rad => 180.0 / std::f64::consts::PI,
            AngleUnit::Turn => 360.0,
        }
    }
}

impl TimeUnit {
    /// Seconds per unit.
    pub fn seconds(self) -> f64 {
        match self {
            TimeUnit::S => 1.0,
            TimeUnit::Ms => 0.001,
        }
    }
}

impl FrequencyUnit {
    /// Hertz per unit.
    pub fn hertz(self) -> f64 {
        match self {
            FrequencyUnit::Hz => 1.0,
            FrequencyUnit::Khz => 1000.0,
        }
    }
}

impl ResolutionUnit {
    /// Dots per CSS pixel per unit.
    pub fn dppx(self) -> f64 {
        match self {
            ResolutionUnit::Dppx | ResolutionUnit::X => 1.0,
            ResolutionUnit::Dpi => 1.0 / 96.0,
            ResolutionUnit::Dpcm => 2.54 / 96.0,
        }
    }
}

/// Convert a resolution to dots per CSS pixel, or `None` for another unit.
pub fn resolution_to_dppx(value: f64, unit: &str) -> Option<f64> {
    ResolutionUnit::from_name(unit).map(|u| value * u.dppx())
}

/// A length with its unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length {
    /// The number.
    pub value: f64,
    /// The unit.
    pub unit: LengthUnit,
}

impl Length {
    /// Build a px length.
    pub fn px(value: f64) -> Length {
        Length {
            value,
            unit: LengthUnit::Px,
        }
    }

    /// Resolve to px.
    pub fn to_px(self, ctx: &ResolveContext) -> f64 {
        self.unit.to_px(self.value, ctx)
    }
}

/// The CSS-wide keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlobalKeyword {
    /// `initial`
    Initial,
    /// `inherit`
    Inherit,
    /// `unset`
    Unset,
    /// `revert`
    Revert,
    /// `revert-layer`
    RevertLayer,
}

impl GlobalKeyword {
    /// Look up a keyword, ignoring ASCII case.
    pub fn from_name(s: &str) -> Option<GlobalKeyword> {
        Some(match s.to_ascii_lowercase().as_str() {
            "initial" => GlobalKeyword::Initial,
            "inherit" => GlobalKeyword::Inherit,
            "unset" => GlobalKeyword::Unset,
            "revert" => GlobalKeyword::Revert,
            "revert-layer" => GlobalKeyword::RevertLayer,
            _ => return None,
        })
    }
}

/// One typed component of a value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An identifier, case kept. Compare keywords with
    /// [`Value::is_keyword`].
    Ident(String),
    /// A number with a fractional part or exponent.
    Number(f64),
    /// An integer.
    Integer(i64),
    /// A percentage (50% is `50.0`).
    Percentage(f64),
    /// A length.
    Length(Length),
    /// An angle.
    Angle(f64, AngleUnit),
    /// A time.
    Time(f64, TimeUnit),
    /// A frequency.
    Frequency(f64, FrequencyUnit),
    /// A resolution.
    Resolution(f64, ResolutionUnit),
    /// A flex value (`fr`).
    Flex(f64),
    /// A color.
    Color(Color),
    /// A URL.
    Url(String),
    /// A string.
    String(String),
    /// A math function.
    Calc(Box<Calc>),
    /// Any other function, with its arguments parsed the same way.
    Function {
        /// Name, lowercased.
        name: String,
        /// Arguments, separators included.
        args: Vec<Value>,
    },
    /// Bracketed line names, as in grid templates: `[a b]`.
    LineNames(Vec<String>),
    /// A unicode range.
    UnicodeRange(u32, u32),
    /// A `,` separator.
    Comma,
    /// A `/` separator.
    Slash,
}

impl Value {
    /// True when this is an identifier equal to `k`, ignoring ASCII case.
    pub fn is_keyword(&self, k: &str) -> bool {
        matches!(self, Value::Ident(s) if s.eq_ignore_ascii_case(k))
    }

    /// The color this value names: a color value, or a named color,
    /// system color, `currentcolor`, or `transparent` identifier.
    pub fn as_color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            Value::Ident(s) => color_keyword(s),
            _ => None,
        }
    }

    /// Resolve a length, percentage, or math function to px (or the
    /// canonical unit of the math function's type).
    pub fn resolve(&self, ctx: &ResolveContext) -> Option<f64> {
        match self {
            Value::Length(l) => Some(l.to_px(ctx)),
            Value::Percentage(p) => ctx.percent_basis.map(|b| b * p / 100.0),
            Value::Number(n) => Some(*n),
            Value::Integer(i) => Some(*i as f64),
            Value::Calc(c) => Some(c.resolve(ctx)),
            Value::Angle(v, u) => Some(v * u.degrees()),
            Value::Time(v, u) => Some(v * u.seconds()),
            _ => None,
        }
    }
}

/// A parsed property value.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// A CSS-wide keyword.
    Global(GlobalKeyword),
    /// A custom property's value: the token stream, trimmed.
    Custom(Vec<ComponentValue>),
    /// A value with `var()`, `env()`, or `attr()`: kept as tokens until
    /// computed-value time.
    Unresolved(Vec<ComponentValue>),
    /// A typed value. Space-separated parts are consecutive entries;
    /// commas and slashes appear as [`Value::Comma`] and [`Value::Slash`].
    Values(Vec<Value>),
}

impl PropertyValue {
    /// The single value, when there is exactly one.
    pub fn single(&self) -> Option<&Value> {
        match self {
            PropertyValue::Values(v) if v.len() == 1 => v.first(),
            _ => None,
        }
    }
}

// ---- color ----

/// An sRGB color. Channels are 0 to 255 (not rounded); alpha is 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    /// Red, 0 to 255.
    pub r: f32,
    /// Green, 0 to 255.
    pub g: f32,
    /// Blue, 0 to 255.
    pub b: f32,
    /// Alpha, 0 to 1.
    pub alpha: f32,
}

impl Rgba {
    /// Build from 8-bit channels and an alpha.
    pub fn new(r: u8, g: u8, b: u8, alpha: f32) -> Rgba {
        Rgba {
            r: f32::from(r),
            g: f32::from(g),
            b: f32::from(b),
            alpha,
        }
    }

    /// Round to 8-bit channels and an 8-bit alpha.
    pub fn to_rgba8(self) -> [u8; 4] {
        let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        [c(self.r), c(self.g), c(self.b), c(self.alpha * 255.0)]
    }

    fn from_unit(r: f64, g: f64, b: f64, alpha: f64) -> Rgba {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as f32;
        Rgba {
            r: c(r),
            g: c(g),
            b: c(b),
            alpha: alpha.clamp(0.0, 1.0) as f32,
        }
    }
}

/// A color value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    /// A concrete sRGB color.
    Rgba(Rgba),
    /// `currentcolor`.
    CurrentColor,
    /// A system color such as `Canvas`; the host decides its value.
    System(SystemColor),
}

/// System colors (CSS Color 4, section 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemColor {
    /// `AccentColor`
    AccentColor,
    /// `AccentColorText`
    AccentColorText,
    /// `ActiveText`
    ActiveText,
    /// `ButtonBorder`
    ButtonBorder,
    /// `ButtonFace`
    ButtonFace,
    /// `ButtonText`
    ButtonText,
    /// `Canvas`
    Canvas,
    /// `CanvasText`
    CanvasText,
    /// `Field`
    Field,
    /// `FieldText`
    FieldText,
    /// `GrayText`
    GrayText,
    /// `Highlight`
    Highlight,
    /// `HighlightText`
    HighlightText,
    /// `LinkText`
    LinkText,
    /// `Mark`
    Mark,
    /// `MarkText`
    MarkText,
    /// `SelectedItem`
    SelectedItem,
    /// `SelectedItemText`
    SelectedItemText,
    /// `VisitedText`
    VisitedText,
}

const SYSTEM_COLORS: &[(&str, SystemColor)] = &[
    ("accentcolor", SystemColor::AccentColor),
    ("accentcolortext", SystemColor::AccentColorText),
    ("activetext", SystemColor::ActiveText),
    ("buttonborder", SystemColor::ButtonBorder),
    ("buttonface", SystemColor::ButtonFace),
    ("buttontext", SystemColor::ButtonText),
    ("canvas", SystemColor::Canvas),
    ("canvastext", SystemColor::CanvasText),
    ("field", SystemColor::Field),
    ("fieldtext", SystemColor::FieldText),
    ("graytext", SystemColor::GrayText),
    ("highlight", SystemColor::Highlight),
    ("highlighttext", SystemColor::HighlightText),
    ("linktext", SystemColor::LinkText),
    ("mark", SystemColor::Mark),
    ("marktext", SystemColor::MarkText),
    ("selecteditem", SystemColor::SelectedItem),
    ("selecteditemtext", SystemColor::SelectedItemText),
    ("visitedtext", SystemColor::VisitedText),
];

/// The 148 named colors of CSS Color 4, as `0xRRGGBB`.
pub const NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

/// A color keyword: a named color, a system color, `transparent`, or
/// `currentcolor`, ignoring ASCII case.
pub fn color_keyword(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    if lower == "transparent" {
        return Some(Color::Rgba(Rgba::new(0, 0, 0, 0.0)));
    }
    if lower == "currentcolor" {
        return Some(Color::CurrentColor);
    }
    if let Some((_, rgb)) = NAMED_COLORS.iter().find(|(n, _)| *n == lower) {
        let [_, r, g, b] = rgb.to_be_bytes();
        return Some(Color::Rgba(Rgba::new(r, g, b, 1.0)));
    }
    SYSTEM_COLORS
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, s)| Color::System(*s))
}

/// Parse a hex color body (without `#`): 3, 4, 6, or 8 hex digits.
pub fn parse_hex_color(hex: &str) -> Option<Color> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d: Vec<u8> = hex
        .chars()
        .filter_map(|c| c.to_digit(16))
        .map(|v| v as u8)
        .collect();
    let (r, g, b, a) = match d.len() {
        3 => (d[0] * 17, d[1] * 17, d[2] * 17, 255),
        4 => (d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17),
        6 => (d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], 255),
        8 => (
            d[0] * 16 + d[1],
            d[2] * 16 + d[3],
            d[4] * 16 + d[5],
            d[6] * 16 + d[7],
        ),
        _ => return None,
    };
    Some(Color::Rgba(Rgba::new(r, g, b, f32::from(a) / 255.0)))
}

/// A color function argument.
#[derive(Clone, Copy, Debug)]
enum Arg {
    Number(f64),
    Percent(f64),
    Angle(f64),
    None,
}

/// Split color function arguments into components and an optional alpha.
/// Returns (components, alpha, legacy comma syntax).
fn color_args(
    args: &[ComponentValue],
    limits: &Limits,
    depth: usize,
) -> Result<(Vec<Arg>, Option<Arg>, bool), ValueError> {
    let span = span_of(args);
    let items: Vec<&ComponentValue> = args.iter().filter(|v| !v.is_whitespace()).collect();
    let legacy = items
        .iter()
        .any(|v| matches!(v.token(), Some(Token::Comma)));
    let mut comps = Vec::new();
    let mut alpha = None;
    if legacy {
        for (k, group) in items
            .split(|v| matches!(v.token(), Some(Token::Comma)))
            .enumerate()
        {
            let [one] = group else {
                return verr(
                    "each comma-separated color argument must be one value",
                    span,
                );
            };
            let a = color_arg(one, limits, depth)?;
            if matches!(a, Arg::None) {
                return verr("'none' is not allowed in the comma syntax", span);
            }
            match k.cmp(&3) {
                std::cmp::Ordering::Less => comps.push(a),
                std::cmp::Ordering::Equal => alpha = Some(a),
                std::cmp::Ordering::Greater => return verr("too many color arguments", span),
            }
        }
    } else {
        let mut seen_slash = false;
        for v in &items {
            if v.is_delim('/') {
                if seen_slash {
                    return verr("only one '/' is allowed", span);
                }
                seen_slash = true;
                continue;
            }
            let a = color_arg(v, limits, depth)?;
            if seen_slash {
                if alpha.is_some() {
                    return verr("only one alpha value is allowed", span);
                }
                alpha = Some(a);
            } else {
                comps.push(a);
            }
        }
        if seen_slash && alpha.is_none() {
            return verr("'/' must be followed by an alpha value", span);
        }
    }
    Ok((comps, alpha, legacy))
}

fn color_arg(v: &ComponentValue, limits: &Limits, depth: usize) -> Result<Arg, ValueError> {
    match v {
        ComponentValue::Token(t) => match &t.token {
            Token::Number(n) => Ok(Arg::Number(n.value)),
            Token::Percentage(n) => Ok(Arg::Percent(n.value)),
            Token::Dimension { value, unit } => match AngleUnit::from_name(unit) {
                Some(u) => Ok(Arg::Angle(value.value * u.degrees())),
                None => verr("unexpected unit in a color", t.span),
            },
            Token::Ident(s) if s.eq_ignore_ascii_case("none") => Ok(Arg::None),
            _ => verr("unexpected value in a color", t.span),
        },
        ComponentValue::Function(_) => {
            let c = parse_math_depth(v, limits, depth + 1)?;
            let x = c.resolve(&ResolveContext::default());
            match c.ty {
                CalcType::Number => Ok(Arg::Number(x)),
                CalcType::Percentage => Ok(Arg::Percent(x)),
                CalcType::Angle => Ok(Arg::Angle(x)),
                _ => verr("unexpected math type in a color", v.span()),
            }
        }
        ComponentValue::Block(b) => verr("unexpected block in a color", b.span),
    }
}

fn alpha_value(a: Option<Arg>, span: Span) -> Result<f64, ValueError> {
    match a {
        None => Ok(1.0),
        Some(Arg::Number(n)) => Ok(n.clamp(0.0, 1.0)),
        Some(Arg::Percent(p)) => Ok((p / 100.0).clamp(0.0, 1.0)),
        Some(Arg::None) => Ok(0.0),
        Some(Arg::Angle(_)) => verr("alpha cannot be an angle", span),
    }
}

fn hue(a: Arg, span: Span) -> Result<f64, ValueError> {
    match a {
        Arg::Number(n) | Arg::Angle(n) => Ok(n.rem_euclid(360.0)),
        Arg::None => Ok(0.0),
        Arg::Percent(_) => verr("hue cannot be a percentage", span),
    }
}

/// A number or percentage, where `pct_scale` is what 100% means.
fn num_or_pct(a: Arg, pct_scale: f64, span: Span) -> Result<f64, ValueError> {
    match a {
        Arg::Number(n) => Ok(n),
        Arg::Percent(p) => Ok(p / 100.0 * pct_scale),
        Arg::None => Ok(0.0),
        Arg::Angle(_) => verr("unexpected angle in a color", span),
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    (f(0.0), f(8.0), f(4.0))
}

fn linear_to_srgb(c: f64) -> f64 {
    let sign = if c < 0.0 { -1.0 } else { 1.0 };
    let a = c.abs();
    if a <= 0.003_130_8 {
        c * 12.92
    } else {
        sign * (1.055 * a.powf(1.0 / 2.4) - 0.055)
    }
}

fn srgb_to_linear(c: f64) -> f64 {
    let sign = if c < 0.0 { -1.0 } else { 1.0 };
    let a = c.abs();
    if a <= 0.040_45 {
        c / 12.92
    } else {
        sign * ((a + 0.055) / 1.055).powf(2.4)
    }
}

fn mat3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// CIE XYZ (D65) to linear sRGB.
const XYZ_D65_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [
        3.240_969_941_904_522_6,
        -1.537_383_177_570_094,
        -0.498_610_760_293_003_4,
    ],
    [
        -0.969_243_636_280_879_6,
        1.875_967_501_507_720_2,
        0.041_555_057_407_175_59,
    ],
    [
        0.055_630_079_696_993_66,
        -0.203_976_958_888_976_52,
        1.056_971_514_242_878_6,
    ],
];

/// Bradford chromatic adaptation from D50 to D65.
const D50_TO_D65: [[f64; 3]; 3] = [
    [
        0.955_473_421_488_075,
        -0.023_098_454_948_764_71,
        0.063_259_243_200_570_72,
    ],
    [
        -0.028_369_709_333_863_7,
        1.009_995_398_081_304_1,
        0.021_041_441_191_917_323,
    ],
    [
        0.012_314_014_864_481_998,
        -0.020_507_649_298_898_964,
        1.330_365_926_242_124,
    ],
];

/// Linear Display-P3 to CIE XYZ (D65).
const DISPLAY_P3_TO_XYZ_D65: [[f64; 3]; 3] = [
    [0.486_570_948_6, 0.265_667_693_2, 0.198_217_285_2],
    [0.228_974_564_1, 0.691_738_521_8, 0.079_286_914_1],
    [0.0, 0.045_113_381_9, 1.043_944_368_9],
];

fn lab_to_rgb(l: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let kappa = 24389.0 / 27.0;
    let epsilon = 216.0 / 24389.0;
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let x = if f0.powi(3) > epsilon {
        f0.powi(3)
    } else {
        (116.0 * f0 - 16.0) / kappa
    };
    let y = if l > kappa * epsilon {
        f1.powi(3)
    } else {
        l / kappa
    };
    let z = if f2.powi(3) > epsilon {
        f2.powi(3)
    } else {
        (116.0 * f2 - 16.0) / kappa
    };
    // D50 white point.
    let xyz50 = [x * 0.3457 / 0.3585, y, z * (1.0 - 0.3457 - 0.3585) / 0.3585];
    let xyz65 = mat3(&D50_TO_D65, xyz50);
    let lin = mat3(&XYZ_D65_TO_LINEAR_SRGB, xyz65);
    (
        linear_to_srgb(lin[0]),
        linear_to_srgb(lin[1]),
        linear_to_srgb(lin[2]),
    )
}

fn oklab_to_rgb(l: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3;
    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(bl))
}

fn rgb_to_oklab(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (rl, gl, bl) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let l = (0.412_221_470_8 * rl + 0.536_332_536_3 * gl + 0.051_445_992_9 * bl).cbrt();
    let m = (0.211_903_498_2 * rl + 0.680_699_545_1 * gl + 0.107_396_956_6 * bl).cbrt();
    let s = (0.088_302_461_9 * rl + 0.281_718_837_6 * gl + 0.629_978_700_5 * bl).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    )
}

fn polar(c: f64, h_deg: f64) -> (f64, f64) {
    let h = h_deg.to_radians();
    (c * h.cos(), c * h.sin())
}

fn three(comps: &[Arg], span: Span) -> Result<[Arg; 3], ValueError> {
    match comps {
        [a, b, c] => Ok([*a, *b, *c]),
        _ => verr("a color needs three components", span),
    }
}

/// Parse a color function (`rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`,
/// `oklab()`, `oklch()`, `color()`, `color-mix()`, `light-dark()`), or
/// `None` when `name` is not one.
fn color_function(
    name: &str,
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Option<Result<Color, ValueError>> {
    let lower = name.to_ascii_lowercase();
    let known = [
        "rgb",
        "rgba",
        "hsl",
        "hsla",
        "hwb",
        "lab",
        "lch",
        "oklab",
        "oklch",
        "color",
        "color-mix",
        "light-dark",
    ];
    if !known.contains(&lower.as_str()) {
        return None;
    }
    Some(color_function_inner(&lower, args, span, limits, depth))
}

fn color_function_inner(
    lower: &str,
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<Color, ValueError> {
    if lower == "color" {
        return color_space_function(args, span, limits, depth);
    }
    if lower == "color-mix" {
        return color_mix_function(args, span, limits, depth);
    }
    if lower == "light-dark" {
        return light_dark_function(args, span, limits, depth);
    }
    let (comps, alpha, legacy) = color_args(args, limits, depth)?;
    let a = alpha_value(alpha, span)?;
    let [c0, c1, c2] = three(&comps, span)?;
    let (r, g, b) = match lower {
        "rgb" | "rgba" => {
            if legacy {
                let all_pct = comps.iter().all(|x| matches!(x, Arg::Percent(_)));
                let all_num = comps.iter().all(|x| matches!(x, Arg::Number(_)));
                if !(all_pct || all_num) {
                    return verr(
                        "rgb() comma syntax needs all numbers or all percentages",
                        span,
                    );
                }
            }
            let ch = |x: Arg| num_or_pct(x, 255.0, span).map(|v| v / 255.0);
            (ch(c0)?, ch(c1)?, ch(c2)?)
        }
        "hsl" | "hsla" => {
            if legacy && !(matches!(c1, Arg::Percent(_)) && matches!(c2, Arg::Percent(_))) {
                return verr("hsl() comma syntax needs percentages", span);
            }
            let h = hue(c0, span)?;
            let s = num_or_pct(c1, 100.0, span)? / 100.0;
            let l = num_or_pct(c2, 100.0, span)? / 100.0;
            hsl_to_rgb(h, s, l)
        }
        "hwb" => {
            if legacy {
                return verr("hwb() has no comma syntax", span);
            }
            let h = hue(c0, span)?;
            let w = (num_or_pct(c1, 100.0, span)? / 100.0).clamp(0.0, 1.0);
            let bk = (num_or_pct(c2, 100.0, span)? / 100.0).clamp(0.0, 1.0);
            if w + bk >= 1.0 {
                let gray = w / (w + bk);
                (gray, gray, gray)
            } else {
                let (r, g, b) = hsl_to_rgb(h, 1.0, 0.5);
                let k = 1.0 - w - bk;
                (r * k + w, g * k + w, b * k + w)
            }
        }
        "lab" | "lch" | "oklab" | "oklch" => {
            if legacy {
                return verr("this color function has no comma syntax", span);
            }
            let ok = lower.starts_with("ok");
            let l = num_or_pct(c0, if ok { 1.0 } else { 100.0 }, span)?;
            let (a2, b2) = if lower.ends_with("lab") {
                let scale = if ok { 0.4 } else { 125.0 };
                (num_or_pct(c1, scale, span)?, num_or_pct(c2, scale, span)?)
            } else {
                let scale = if ok { 0.4 } else { 150.0 };
                let c = num_or_pct(c1, scale, span)?.max(0.0);
                polar(c, hue(c2, span)?)
            };
            if ok {
                oklab_to_rgb(l.clamp(0.0, 1.0), a2, b2)
            } else {
                lab_to_rgb(l.clamp(0.0, 100.0), a2, b2)
            }
        }
        _ => return verr("unknown color function", span),
    };
    Ok(Color::Rgba(Rgba::from_unit(r, g, b, a)))
}

fn color_space_function(
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<Color, ValueError> {
    let mut i = 0;
    while args.get(i).is_some_and(|v| v.is_whitespace()) {
        i += 1;
    }
    let Some(Token::Ident(space)) = args.get(i).and_then(|v| v.token()) else {
        return verr("color() needs a color space", span);
    };
    let space = space.to_ascii_lowercase();
    let (comps, alpha, legacy) = color_args(&args[i + 1..], limits, depth)?;
    if legacy {
        return verr("color() has no comma syntax", span);
    }
    let a = alpha_value(alpha, span)?;
    let [c0, c1, c2] = three(&comps, span)?;
    let ch = |x: Arg| num_or_pct(x, 1.0, span);
    let (r, g, b) = (ch(c0)?, ch(c1)?, ch(c2)?);
    let (r, g, b) = match space.as_str() {
        "srgb" => (r, g, b),
        "srgb-linear" => (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b)),
        "display-p3" => {
            let lin_p3 = [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)];
            let xyz65 = mat3(&DISPLAY_P3_TO_XYZ_D65, lin_p3);
            let lin = mat3(&XYZ_D65_TO_LINEAR_SRGB, xyz65);
            (
                linear_to_srgb(lin[0]),
                linear_to_srgb(lin[1]),
                linear_to_srgb(lin[2]),
            )
        }
        "xyz" | "xyz-d65" => {
            let lin = mat3(&XYZ_D65_TO_LINEAR_SRGB, [r, g, b]);
            (
                linear_to_srgb(lin[0]),
                linear_to_srgb(lin[1]),
                linear_to_srgb(lin[2]),
            )
        }
        "xyz-d50" => {
            let xyz65 = mat3(&D50_TO_D65, [r, g, b]);
            let lin = mat3(&XYZ_D65_TO_LINEAR_SRGB, xyz65);
            (
                linear_to_srgb(lin[0]),
                linear_to_srgb(lin[1]),
                linear_to_srgb(lin[2]),
            )
        }
        _ => return verr(format!("color space '{space}' is not supported yet"), span),
    };
    Ok(Color::Rgba(Rgba::from_unit(r, g, b, a)))
}

fn color_to_rgba(c: Color, span: Span) -> Result<Rgba, ValueError> {
    match c {
        Color::Rgba(rgba) => Ok(rgba),
        Color::CurrentColor => Ok(Rgba::new(0, 0, 0, 1.0)),
        Color::System(sys) => {
            let [r, g, b, a] = match sys {
                SystemColor::Canvas | SystemColor::Field => [255, 255, 255, 255],
                SystemColor::ButtonFace => [240, 240, 240, 255],
                SystemColor::ButtonBorder => [118, 118, 118, 255],
                SystemColor::GrayText => [128, 128, 128, 255],
                SystemColor::LinkText | SystemColor::AccentColor | SystemColor::Highlight => {
                    [0, 102, 204, 255]
                }
                SystemColor::AccentColorText
                | SystemColor::HighlightText
                | SystemColor::SelectedItemText => [255, 255, 255, 255],
                SystemColor::Mark => [255, 255, 0, 255],
                _ => [0, 0, 0, 255],
            };
            let _ = span;
            Ok(Rgba::new(r, g, b, f32::from(a) / 255.0))
        }
    }
}

fn parse_mix_stop(
    group: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<(Rgba, Option<f64>), ValueError> {
    let items: Vec<&ComponentValue> = group.iter().filter(|v| !v.is_whitespace()).collect();
    match items.as_slice() {
        [one] => {
            let c = parse_color_depth(one, limits, depth)?;
            Ok((color_to_rgba(c, span)?, None))
        }
        [first, second] => {
            if let Some(Token::Percentage(p)) = second.token() {
                if !(0.0..=100.0).contains(&p.value) {
                    return verr("color-mix() percentage must be in 0%..=100%", second.span());
                }
                let c = parse_color_depth(first, limits, depth)?;
                Ok((color_to_rgba(c, span)?, Some(p.value)))
            } else if let Some(Token::Percentage(p)) = first.token() {
                if !(0.0..=100.0).contains(&p.value) {
                    return verr("color-mix() percentage must be in 0%..=100%", first.span());
                }
                let c = parse_color_depth(second, limits, depth)?;
                Ok((color_to_rgba(c, span)?, Some(p.value)))
            } else {
                verr(
                    "expected a color and optional percentage in color-mix()",
                    span,
                )
            }
        }
        _ => verr("invalid color-mix() stop", span),
    }
}

fn color_mix_function(
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<Color, ValueError> {
    let groups: Vec<&[ComponentValue]> = args
        .split(|v| matches!(v.token(), Some(Token::Comma)))
        .collect();
    let [space_group, stop1_group, stop2_group] = groups.as_slice() else {
        return verr("color-mix() takes a color space and two colors", span);
    };
    let space_tokens: Vec<&ComponentValue> =
        space_group.iter().filter(|v| !v.is_whitespace()).collect();
    if space_tokens.len() < 2 || !space_tokens[0].is_ident("in") {
        return verr("color-mix() must start with 'in <color-space>'", span);
    }
    let Some(Token::Ident(space_ident)) = space_tokens[1].token() else {
        return verr("color-mix() needs a color space name", span);
    };
    let space = space_ident.to_ascii_lowercase();
    let valid_spaces = [
        "srgb",
        "srgb-linear",
        "lab",
        "oklab",
        "xyz",
        "xyz-d50",
        "xyz-d65",
        "hsl",
        "hwb",
        "lch",
        "oklch",
        "display-p3",
    ];
    if !valid_spaces.contains(&space.as_str()) {
        return verr(
            format!("unknown color space '{space}' in color-mix()"),
            span,
        );
    }
    let (c1, p1) = parse_mix_stop(stop1_group, span, limits, depth)?;
    let (c2, p2) = parse_mix_stop(stop2_group, span, limits, depth)?;
    let (mut p1, mut p2) = match (p1, p2) {
        (None, None) => (50.0, 50.0),
        (Some(a), None) => (a, 100.0 - a),
        (None, Some(b)) => (100.0 - b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum <= 0.0 {
        return verr("color-mix() percentages cannot sum to zero", span);
    }
    let alpha_mult = if sum < 100.0 { sum / 100.0 } else { 1.0 };
    p1 /= sum;
    p2 /= sum;

    let (r1, g1, b1, a1) = (
        f64::from(c1.r) / 255.0,
        f64::from(c1.g) / 255.0,
        f64::from(c1.b) / 255.0,
        f64::from(c1.alpha),
    );
    let (r2, g2, b2, a2) = (
        f64::from(c2.r) / 255.0,
        f64::from(c2.g) / 255.0,
        f64::from(c2.b) / 255.0,
        f64::from(c2.alpha),
    );
    let premul_a = a1 * p1 + a2 * p2;
    let out_alpha = premul_a * alpha_mult;
    let (w1, w2) = if premul_a > 0.0 {
        ((a1 * p1) / premul_a, (a2 * p2) / premul_a)
    } else {
        (p1, p2)
    };
    let (r, g, b) = match space.as_str() {
        "srgb-linear" | "xyz" | "xyz-d65" | "xyz-d50" => (
            linear_to_srgb(srgb_to_linear(r1) * w1 + srgb_to_linear(r2) * w2),
            linear_to_srgb(srgb_to_linear(g1) * w1 + srgb_to_linear(g2) * w2),
            linear_to_srgb(srgb_to_linear(b1) * w1 + srgb_to_linear(b2) * w2),
        ),
        "oklab" | "oklch" | "lab" | "lch" => {
            let (l1, oa1, ob1) = rgb_to_oklab(r1, g1, b1);
            let (l2, oa2, ob2) = rgb_to_oklab(r2, g2, b2);
            oklab_to_rgb(l1 * w1 + l2 * w2, oa1 * w1 + oa2 * w2, ob1 * w1 + ob2 * w2)
        }
        _ => (r1 * w1 + r2 * w2, g1 * w1 + g2 * w2, b1 * w1 + b2 * w2),
    };
    Ok(Color::Rgba(Rgba::from_unit(r, g, b, out_alpha)))
}

fn light_dark_function(
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<Color, ValueError> {
    let groups: Vec<&[ComponentValue]> = args
        .split(|v| matches!(v.token(), Some(Token::Comma)))
        .collect();
    let [light_group, dark_group] = groups.as_slice() else {
        return verr("light-dark() takes two color arguments", span);
    };
    let light_tokens = trim(light_group);
    let dark_tokens = trim(dark_group);
    let ([light_cv], [dark_cv]) = (light_tokens, dark_tokens) else {
        return verr("light-dark() arguments must each be a single color", span);
    };
    let light = parse_color_depth(light_cv, limits, depth)?;
    let _dark = parse_color_depth(dark_cv, limits, depth)?;
    Ok(light)
}

/// Parse one component value as a color.
pub fn parse_color_value(v: &ComponentValue, limits: &Limits) -> Result<Color, ValueError> {
    parse_color_depth(v, limits, 0)
}

/// Parse a standalone CSS color string (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`,
/// named color, system color, `transparent`, `currentColor`, `rgb()`, `rgba()`,
/// `hsl()`, `hsla()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`,
/// `color()`, `color-mix()`, or `light-dark()`).
pub fn parse_css_color(src: &str) -> Option<Color> {
    let options = Options::default();
    let values = crate::prism::parser::parse_component_values(src, &options).ok()?;
    let trimmed = trim(&values);
    let [one] = trimmed else {
        return None;
    };
    parse_color_value(one, &options.limits).ok()
}

/// Parse a standalone CSS color string into an `[r, g, b, a]` byte array (`0..=255`).
/// Returns `None` for `currentColor` or invalid color strings.
pub fn parse_css_rgba8(src: &str) -> Option<[u8; 4]> {
    match parse_css_color(src)? {
        Color::Rgba(rgba) => Some(rgba.to_rgba8()),
        Color::CurrentColor => None,
        Color::System(sys) => Some(match sys {
            SystemColor::Canvas | SystemColor::Field => [255, 255, 255, 255],
            SystemColor::ButtonFace => [240, 240, 240, 255],
            SystemColor::ButtonBorder => [118, 118, 118, 255],
            SystemColor::GrayText => [128, 128, 128, 255],
            SystemColor::LinkText | SystemColor::AccentColor | SystemColor::Highlight => {
                [0, 102, 204, 255]
            }
            SystemColor::AccentColorText
            | SystemColor::HighlightText
            | SystemColor::SelectedItemText => [255, 255, 255, 255],
            SystemColor::Mark => [255, 255, 0, 255],
            _ => [0, 0, 0, 255],
        }),
    }
}

fn parse_color_depth(
    v: &ComponentValue,
    limits: &Limits,
    depth: usize,
) -> Result<Color, ValueError> {
    match v {
        ComponentValue::Token(t) => match &t.token {
            Token::Hash { value, .. } => parse_hex_color(value).ok_or_else(|| ValueError {
                message: "invalid hex color".into(),
                span: t.span,
                limit: None,
            }),
            Token::Ident(s) => color_keyword(s).ok_or_else(|| ValueError {
                message: format!("'{s}' is not a color"),
                span: t.span,
                limit: None,
            }),
            _ => verr("expected a color", t.span),
        },
        ComponentValue::Function(f) => {
            if depth >= limits.max_value_depth {
                return Err(depth_error(limits, f.span));
            }
            match color_function(&f.name, &f.arguments, f.span, limits, depth + 1) {
                Some(r) => r,
                None => verr(format!("'{}()' is not a color function", f.name), f.span),
            }
        }
        ComponentValue::Block(b) => verr("expected a color", b.span),
    }
}

fn depth_error(limits: &Limits, span: Span) -> ValueError {
    ValueError {
        message: format!(
            "value functions nest deeper than {} levels",
            limits.max_value_depth
        ),
        span,
        limit: Some(LimitKind::ValueDepth),
    }
}

fn span_of(values: &[ComponentValue]) -> Span {
    match (values.first(), values.last()) {
        (Some(a), Some(b)) => a.span().to(b.span()),
        _ => Span::default(),
    }
}

// ---- math functions ----

/// The type a math function resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalcType {
    /// A plain number.
    Number,
    /// A length.
    Length,
    /// A percentage on its own.
    Percentage,
    /// A mix of length and percentage.
    LengthPercentage,
    /// An angle (canonical unit: degrees).
    Angle,
    /// A time (canonical unit: seconds).
    Time,
    /// A frequency (canonical unit: hertz).
    Frequency,
    /// A resolution (canonical unit: dppx).
    Resolution,
}

/// Rounding strategy for `round()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundingStrategy {
    /// `nearest` (default)
    Nearest,
    /// `up`
    Up,
    /// `down`
    Down,
    /// `to-zero`
    ToZero,
}

/// A leaf in a math expression, in canonical units where the unit is
/// absolute: px, degrees, seconds, hertz, dppx.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalcLeaf {
    /// A number.
    Number(f64),
    /// A percentage (50% is `50.0`).
    Percentage(f64),
    /// A length. Absolute units are already converted to px.
    Length(Length),
    /// Degrees.
    Angle(f64),
    /// Seconds.
    Time(f64),
    /// Hertz.
    Frequency(f64),
    /// Dots per px.
    Resolution(f64),
}

impl CalcLeaf {
    fn ty(self) -> CalcType {
        match self {
            CalcLeaf::Number(_) => CalcType::Number,
            CalcLeaf::Percentage(_) => CalcType::Percentage,
            CalcLeaf::Length(_) => CalcType::Length,
            CalcLeaf::Angle(_) => CalcType::Angle,
            CalcLeaf::Time(_) => CalcType::Time,
            CalcLeaf::Frequency(_) => CalcType::Frequency,
            CalcLeaf::Resolution(_) => CalcType::Resolution,
        }
    }

    fn key(self) -> (u8, Option<LengthUnit>) {
        match self {
            CalcLeaf::Number(_) => (0, None),
            CalcLeaf::Percentage(_) => (1, None),
            CalcLeaf::Length(l) => (2, Some(l.unit)),
            CalcLeaf::Angle(_) => (3, None),
            CalcLeaf::Time(_) => (4, None),
            CalcLeaf::Frequency(_) => (5, None),
            CalcLeaf::Resolution(_) => (6, None),
        }
    }

    /// The number in the leaf's unit.
    pub fn value(self) -> f64 {
        match self {
            CalcLeaf::Number(v)
            | CalcLeaf::Percentage(v)
            | CalcLeaf::Angle(v)
            | CalcLeaf::Time(v)
            | CalcLeaf::Frequency(v)
            | CalcLeaf::Resolution(v) => v,
            CalcLeaf::Length(l) => l.value,
        }
    }

    fn with_value(self, v: f64) -> CalcLeaf {
        match self {
            CalcLeaf::Number(_) => CalcLeaf::Number(v),
            CalcLeaf::Percentage(_) => CalcLeaf::Percentage(v),
            CalcLeaf::Length(l) => CalcLeaf::Length(Length {
                value: v,
                unit: l.unit,
            }),
            CalcLeaf::Angle(_) => CalcLeaf::Angle(v),
            CalcLeaf::Time(_) => CalcLeaf::Time(v),
            CalcLeaf::Frequency(_) => CalcLeaf::Frequency(v),
            CalcLeaf::Resolution(_) => CalcLeaf::Resolution(v),
        }
    }
}

/// A math expression tree.
#[derive(Clone, Debug, PartialEq)]
pub enum CalcNode {
    /// A value.
    Leaf(CalcLeaf),
    /// A sum of terms. Subtraction is a term multiplied by -1.
    Sum(Vec<CalcNode>),
    /// A product of factors. Division is a factor wrapped in `Invert`.
    Product(Vec<CalcNode>),
    /// `1 / x`.
    Invert(Box<CalcNode>),
    /// `min()`
    Min(Vec<CalcNode>),
    /// `max()`
    Max(Vec<CalcNode>),
    /// `clamp(min, value, max)`
    Clamp(Box<[CalcNode; 3]>),
    /// `round(strategy, value, step)`
    Round(RoundingStrategy, Box<CalcNode>, Box<CalcNode>),
    /// `mod(a, b)`
    Mod(Box<CalcNode>, Box<CalcNode>),
    /// `rem(a, b)`
    Rem(Box<CalcNode>, Box<CalcNode>),
    /// `sin(x)`
    Sin(Box<CalcNode>),
    /// `cos(x)`
    Cos(Box<CalcNode>),
    /// `tan(x)`
    Tan(Box<CalcNode>),
    /// `asin(x)`
    Asin(Box<CalcNode>),
    /// `acos(x)`
    Acos(Box<CalcNode>),
    /// `atan(x)`
    Atan(Box<CalcNode>),
    /// `atan2(y, x)`
    Atan2(Box<CalcNode>, Box<CalcNode>),
    /// `pow(base, exp)`
    Pow(Box<CalcNode>, Box<CalcNode>),
    /// `sqrt(x)`
    Sqrt(Box<CalcNode>),
    /// `hypot(x1, ...)`
    Hypot(Vec<CalcNode>),
    /// `log(x, base)`
    Log(Box<CalcNode>, Option<Box<CalcNode>>),
    /// `exp(x)`
    Exp(Box<CalcNode>),
    /// `abs(x)`
    Abs(Box<CalcNode>),
    /// `sign(x)`
    Sign(Box<CalcNode>),
}

/// A typed, simplified math function.
#[derive(Clone, Debug, PartialEq)]
pub struct Calc {
    /// The simplified expression.
    pub node: CalcNode,
    /// What it resolves to.
    pub ty: CalcType,
}

impl Calc {
    /// The single leaf, when simplification reduced the expression to one.
    pub fn as_leaf(&self) -> Option<CalcLeaf> {
        match self.node {
            CalcNode::Leaf(l) => Some(l),
            _ => None,
        }
    }

    /// Resolve to a number in the canonical unit of [`Calc::ty`] (px for
    /// lengths). NaN resolves to 0 and infinities to `±f64::MAX`.
    pub fn resolve(&self, ctx: &ResolveContext) -> f64 {
        let v = eval(&self.node, ctx);
        if v.is_nan() {
            0.0
        } else if v.is_infinite() {
            if v > 0.0 { f64::MAX } else { -f64::MAX }
        } else {
            v
        }
    }
}

fn eval_trig_radians(n: &CalcNode, ctx: &ResolveContext) -> f64 {
    let v = eval(n, ctx);
    if type_of(n) == Some(CalcType::Angle) {
        v.to_radians()
    } else {
        v
    }
}

fn eval_round(strategy: RoundingStrategy, a: f64, b: f64) -> f64 {
    if b == 0.0 || a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let step = b.abs();
    match strategy {
        RoundingStrategy::Nearest => (a / step).round() * step,
        RoundingStrategy::Up => (a / step).ceil() * step,
        RoundingStrategy::Down => (a / step).floor() * step,
        RoundingStrategy::ToZero => (a / step).trunc() * step,
    }
}

fn eval(n: &CalcNode, ctx: &ResolveContext) -> f64 {
    match n {
        CalcNode::Leaf(l) => match l {
            CalcLeaf::Percentage(p) => ctx.percent_basis.unwrap_or(0.0) * p / 100.0,
            CalcLeaf::Length(len) => len.to_px(ctx),
            other => other.value(),
        },
        CalcNode::Sum(v) => v.iter().map(|c| eval(c, ctx)).sum(),
        CalcNode::Product(v) => v.iter().map(|c| eval(c, ctx)).product(),
        CalcNode::Invert(c) => 1.0 / eval(c, ctx),
        CalcNode::Min(v) => v.iter().map(|c| eval(c, ctx)).fold(f64::INFINITY, f64::min),
        CalcNode::Max(v) => v
            .iter()
            .map(|c| eval(c, ctx))
            .fold(f64::NEG_INFINITY, f64::max),
        CalcNode::Clamp(b) => {
            let lo = eval(&b[0], ctx);
            let v = eval(&b[1], ctx);
            let hi = eval(&b[2], ctx);
            lo.max(v.min(hi))
        }
        CalcNode::Round(strat, a, b) => eval_round(*strat, eval(a, ctx), eval(b, ctx)),
        CalcNode::Mod(a, b) => {
            let x = eval(a, ctx);
            let y = eval(b, ctx);
            if y == 0.0 {
                f64::NAN
            } else {
                x - (x / y).floor() * y
            }
        }
        CalcNode::Rem(a, b) => {
            let x = eval(a, ctx);
            let y = eval(b, ctx);
            if y == 0.0 { f64::NAN } else { x % y }
        }
        CalcNode::Sin(a) => eval_trig_radians(a, ctx).sin(),
        CalcNode::Cos(a) => eval_trig_radians(a, ctx).cos(),
        CalcNode::Tan(a) => eval_trig_radians(a, ctx).tan(),
        CalcNode::Asin(a) => eval(a, ctx).asin().to_degrees(),
        CalcNode::Acos(a) => eval(a, ctx).acos().to_degrees(),
        CalcNode::Atan(a) => eval(a, ctx).atan().to_degrees(),
        CalcNode::Atan2(y, x) => eval(y, ctx).atan2(eval(x, ctx)).to_degrees(),
        CalcNode::Pow(base, exp) => eval(base, ctx).powf(eval(exp, ctx)),
        CalcNode::Sqrt(a) => eval(a, ctx).sqrt(),
        CalcNode::Hypot(v) => v.iter().map(|c| eval(c, ctx).powi(2)).sum::<f64>().sqrt(),
        CalcNode::Log(a, base) => {
            let x = eval(a, ctx);
            match base {
                Some(b) => x.log(eval(b, ctx)),
                None => x.ln(),
            }
        }
        CalcNode::Exp(a) => eval(a, ctx).exp(),
        CalcNode::Abs(a) => eval(a, ctx).abs(),
        CalcNode::Sign(a) => {
            let x = eval(a, ctx);
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }
    }
}

fn add_types(a: CalcType, b: CalcType) -> Option<CalcType> {
    use CalcType::*;
    if a == b {
        return Some(a);
    }
    match (a, b) {
        (Length | Percentage | LengthPercentage, Length | Percentage | LengthPercentage) => {
            Some(LengthPercentage)
        }
        _ => None,
    }
}

fn type_of(n: &CalcNode) -> Option<CalcType> {
    match n {
        CalcNode::Leaf(l) => Some(l.ty()),
        CalcNode::Sum(v) | CalcNode::Min(v) | CalcNode::Max(v) | CalcNode::Hypot(v) => {
            let mut it = v.iter();
            let mut t = type_of(it.next()?)?;
            for c in it {
                t = add_types(t, type_of(c)?)?;
            }
            Some(t)
        }
        CalcNode::Clamp(b) => {
            let t = add_types(type_of(&b[0])?, type_of(&b[1])?)?;
            add_types(t, type_of(&b[2])?)
        }
        CalcNode::Round(_, a, b) | CalcNode::Mod(a, b) | CalcNode::Rem(a, b) => {
            add_types(type_of(a)?, type_of(b)?)
        }
        CalcNode::Product(v) => {
            let mut out = CalcType::Number;
            for c in v {
                let t = type_of(c)?;
                if t != CalcType::Number {
                    if out != CalcType::Number {
                        return None;
                    }
                    out = t;
                }
            }
            Some(out)
        }
        CalcNode::Invert(c) => {
            if type_of(c)? == CalcType::Number {
                Some(CalcType::Number)
            } else {
                None
            }
        }
        CalcNode::Sin(a) | CalcNode::Cos(a) | CalcNode::Tan(a) => match type_of(a)? {
            CalcType::Number | CalcType::Angle => Some(CalcType::Number),
            _ => None,
        },
        CalcNode::Asin(a) | CalcNode::Acos(a) | CalcNode::Atan(a) => match type_of(a)? {
            CalcType::Number => Some(CalcType::Angle),
            _ => None,
        },
        CalcNode::Atan2(y, x) => {
            add_types(type_of(y)?, type_of(x)?)?;
            Some(CalcType::Angle)
        }
        CalcNode::Pow(a, b) => match (type_of(a)?, type_of(b)?) {
            (CalcType::Number, CalcType::Number) => Some(CalcType::Number),
            _ => None,
        },
        CalcNode::Sqrt(a) | CalcNode::Exp(a) => match type_of(a)? {
            CalcType::Number => Some(CalcType::Number),
            _ => None,
        },
        CalcNode::Log(a, b) => {
            if type_of(a)? != CalcType::Number {
                return None;
            }
            if let Some(base) = b
                && type_of(base)? != CalcType::Number
            {
                return None;
            }
            Some(CalcType::Number)
        }
        CalcNode::Abs(a) => type_of(a),
        CalcNode::Sign(a) => {
            let _ = type_of(a)?;
            Some(CalcType::Number)
        }
    }
}

fn combine_leaves(items: Vec<CalcNode>) -> Vec<CalcNode> {
    let mut leaves: Vec<CalcLeaf> = Vec::new();
    let mut others = Vec::new();
    for c in items {
        match c {
            CalcNode::Leaf(l) => {
                if let Some(existing) = leaves.iter_mut().find(|x| x.key() == l.key()) {
                    *existing = existing.with_value(existing.value() + l.value());
                } else {
                    leaves.push(l);
                }
            }
            other => others.push(other),
        }
    }
    let mut out: Vec<CalcNode> = leaves.into_iter().map(CalcNode::Leaf).collect();
    out.extend(others);
    out
}

fn same_key_leaves(v: &[CalcNode]) -> Option<Vec<CalcLeaf>> {
    let mut out = Vec::new();
    for c in v {
        match c {
            CalcNode::Leaf(l) => out.push(*l),
            _ => return None,
        }
    }
    let first = out.first()?.key();
    if out.iter().all(|l| l.key() == first) {
        Some(out)
    } else {
        None
    }
}

fn scale(n: CalcNode, f: f64) -> CalcNode {
    if f == 1.0 {
        return n;
    }
    match n {
        CalcNode::Leaf(l) => CalcNode::Leaf(l.with_value(l.value() * f)),
        CalcNode::Sum(v) => CalcNode::Sum(v.into_iter().map(|c| scale(c, f)).collect()),
        other => CalcNode::Product(vec![CalcNode::Leaf(CalcLeaf::Number(f)), other]),
    }
}

fn simplify(n: CalcNode) -> CalcNode {
    match n {
        CalcNode::Leaf(_) => n,
        CalcNode::Sum(v) => {
            let mut flat = Vec::new();
            for c in v {
                match simplify(c) {
                    CalcNode::Sum(inner) => flat.extend(inner),
                    other => flat.push(other),
                }
            }
            let mut out = combine_leaves(flat);
            if out.len() == 1 {
                out.remove(0)
            } else {
                CalcNode::Sum(out)
            }
        }
        CalcNode::Product(v) => {
            let mut factor = 1.0;
            let mut rest = Vec::new();
            let mut flat = Vec::new();
            for c in v {
                match simplify(c) {
                    CalcNode::Product(inner) => flat.extend(inner),
                    other => flat.push(other),
                }
            }
            for c in flat {
                match c {
                    CalcNode::Leaf(CalcLeaf::Number(x)) => factor *= x,
                    other => rest.push(other),
                }
            }
            match rest.len() {
                0 => CalcNode::Leaf(CalcLeaf::Number(factor)),
                1 => scale(rest.remove(0), factor),
                _ => {
                    if factor != 1.0 {
                        rest.insert(0, CalcNode::Leaf(CalcLeaf::Number(factor)));
                    }
                    CalcNode::Product(rest)
                }
            }
        }
        CalcNode::Invert(c) => match simplify(*c) {
            CalcNode::Leaf(CalcLeaf::Number(x)) => CalcNode::Leaf(CalcLeaf::Number(1.0 / x)),
            other => CalcNode::Invert(Box::new(other)),
        },
        CalcNode::Min(v) => {
            let v: Vec<CalcNode> = v.into_iter().map(simplify).collect();
            match same_key_leaves(&v) {
                Some(leaves) => {
                    let m = leaves
                        .iter()
                        .map(|l| l.value())
                        .fold(f64::INFINITY, f64::min);
                    CalcNode::Leaf(leaves[0].with_value(m))
                }
                None => CalcNode::Min(v),
            }
        }
        CalcNode::Max(v) => {
            let v: Vec<CalcNode> = v.into_iter().map(simplify).collect();
            match same_key_leaves(&v) {
                Some(leaves) => {
                    let m = leaves
                        .iter()
                        .map(|l| l.value())
                        .fold(f64::NEG_INFINITY, f64::max);
                    CalcNode::Leaf(leaves[0].with_value(m))
                }
                None => CalcNode::Max(v),
            }
        }
        CalcNode::Clamp(b) => {
            let [lo, v, hi] = *b;
            let parts = [simplify(lo), simplify(v), simplify(hi)];
            match same_key_leaves(&parts) {
                Some(l) => {
                    let r = l[0].value().max(l[1].value().min(l[2].value()));
                    CalcNode::Leaf(l[0].with_value(r))
                }
                None => CalcNode::Clamp(Box::new(parts)),
            }
        }
        CalcNode::Round(strat, a, b) => {
            let (a, b) = (simplify(*a), simplify(*b));
            if let (CalcNode::Leaf(la), CalcNode::Leaf(lb)) = (&a, &b)
                && la.key() == lb.key()
            {
                return CalcNode::Leaf(la.with_value(eval_round(strat, la.value(), lb.value())));
            }
            CalcNode::Round(strat, Box::new(a), Box::new(b))
        }
        CalcNode::Mod(a, b) => {
            let (a, b) = (simplify(*a), simplify(*b));
            if let (CalcNode::Leaf(la), CalcNode::Leaf(lb)) = (&a, &b)
                && la.key() == lb.key()
                && lb.value() != 0.0
            {
                let (x, y) = (la.value(), lb.value());
                return CalcNode::Leaf(la.with_value(x - (x / y).floor() * y));
            }
            CalcNode::Mod(Box::new(a), Box::new(b))
        }
        CalcNode::Rem(a, b) => {
            let (a, b) = (simplify(*a), simplify(*b));
            if let (CalcNode::Leaf(la), CalcNode::Leaf(lb)) = (&a, &b)
                && la.key() == lb.key()
                && lb.value() != 0.0
            {
                return CalcNode::Leaf(la.with_value(la.value() % lb.value()));
            }
            CalcNode::Rem(Box::new(a), Box::new(b))
        }
        CalcNode::Sin(a) => {
            let a = simplify(*a);
            match &a {
                CalcNode::Leaf(CalcLeaf::Angle(deg)) => {
                    CalcNode::Leaf(CalcLeaf::Number(deg.to_radians().sin()))
                }
                CalcNode::Leaf(CalcLeaf::Number(rad)) => {
                    CalcNode::Leaf(CalcLeaf::Number(rad.sin()))
                }
                _ => CalcNode::Sin(Box::new(a)),
            }
        }
        CalcNode::Cos(a) => {
            let a = simplify(*a);
            match &a {
                CalcNode::Leaf(CalcLeaf::Angle(deg)) => {
                    CalcNode::Leaf(CalcLeaf::Number(deg.to_radians().cos()))
                }
                CalcNode::Leaf(CalcLeaf::Number(rad)) => {
                    CalcNode::Leaf(CalcLeaf::Number(rad.cos()))
                }
                _ => CalcNode::Cos(Box::new(a)),
            }
        }
        CalcNode::Tan(a) => {
            let a = simplify(*a);
            match &a {
                CalcNode::Leaf(CalcLeaf::Angle(deg)) => {
                    CalcNode::Leaf(CalcLeaf::Number(deg.to_radians().tan()))
                }
                CalcNode::Leaf(CalcLeaf::Number(rad)) => {
                    CalcNode::Leaf(CalcLeaf::Number(rad.tan()))
                }
                _ => CalcNode::Tan(Box::new(a)),
            }
        }
        CalcNode::Asin(a) => match simplify(*a) {
            CalcNode::Leaf(CalcLeaf::Number(v)) => {
                CalcNode::Leaf(CalcLeaf::Angle(v.asin().to_degrees()))
            }
            other => CalcNode::Asin(Box::new(other)),
        },
        CalcNode::Acos(a) => match simplify(*a) {
            CalcNode::Leaf(CalcLeaf::Number(v)) => {
                CalcNode::Leaf(CalcLeaf::Angle(v.acos().to_degrees()))
            }
            other => CalcNode::Acos(Box::new(other)),
        },
        CalcNode::Atan(a) => match simplify(*a) {
            CalcNode::Leaf(CalcLeaf::Number(v)) => {
                CalcNode::Leaf(CalcLeaf::Angle(v.atan().to_degrees()))
            }
            other => CalcNode::Atan(Box::new(other)),
        },
        CalcNode::Atan2(y, x) => {
            let (y, x) = (simplify(*y), simplify(*x));
            if let (CalcNode::Leaf(ly), CalcNode::Leaf(lx)) = (&y, &x)
                && ly.key() == lx.key()
            {
                return CalcNode::Leaf(CalcLeaf::Angle(ly.value().atan2(lx.value()).to_degrees()));
            }
            CalcNode::Atan2(Box::new(y), Box::new(x))
        }
        CalcNode::Pow(base, exp) => {
            let (base, exp) = (simplify(*base), simplify(*exp));
            if let (CalcNode::Leaf(CalcLeaf::Number(b)), CalcNode::Leaf(CalcLeaf::Number(e))) =
                (&base, &exp)
            {
                return CalcNode::Leaf(CalcLeaf::Number(b.powf(*e)));
            }
            CalcNode::Pow(Box::new(base), Box::new(exp))
        }
        CalcNode::Sqrt(a) => match simplify(*a) {
            CalcNode::Leaf(CalcLeaf::Number(v)) => CalcNode::Leaf(CalcLeaf::Number(v.sqrt())),
            other => CalcNode::Sqrt(Box::new(other)),
        },
        CalcNode::Hypot(v) => {
            let v: Vec<CalcNode> = v.into_iter().map(simplify).collect();
            match same_key_leaves(&v) {
                Some(leaves) => {
                    let h = leaves.iter().map(|l| l.value().powi(2)).sum::<f64>().sqrt();
                    CalcNode::Leaf(leaves[0].with_value(h))
                }
                None => CalcNode::Hypot(v),
            }
        }
        CalcNode::Log(a, base) => {
            let a = simplify(*a);
            let base = base.map(|b| Box::new(simplify(*b)));
            if let CalcNode::Leaf(CalcLeaf::Number(x)) = &a {
                match &base {
                    None => return CalcNode::Leaf(CalcLeaf::Number(x.ln())),
                    Some(b) => {
                        if let CalcNode::Leaf(CalcLeaf::Number(bv)) = b.as_ref() {
                            return CalcNode::Leaf(CalcLeaf::Number(x.log(*bv)));
                        }
                    }
                }
            }
            CalcNode::Log(Box::new(a), base)
        }
        CalcNode::Exp(a) => match simplify(*a) {
            CalcNode::Leaf(CalcLeaf::Number(v)) => CalcNode::Leaf(CalcLeaf::Number(v.exp())),
            other => CalcNode::Exp(Box::new(other)),
        },
        CalcNode::Abs(a) => match simplify(*a) {
            CalcNode::Leaf(l) => CalcNode::Leaf(l.with_value(l.value().abs())),
            other => CalcNode::Abs(Box::new(other)),
        },
        CalcNode::Sign(a) => match simplify(*a) {
            CalcNode::Leaf(l) => {
                let v = l.value();
                let s = if v > 0.0 {
                    1.0
                } else if v < 0.0 {
                    -1.0
                } else {
                    v
                };
                CalcNode::Leaf(CalcLeaf::Number(s))
            }
            other => CalcNode::Sign(Box::new(other)),
        },
    }
}

/// Turn a dimension into a math leaf, converting absolute units.
fn dimension_leaf(value: f64, unit: &str) -> Option<CalcLeaf> {
    if let Some(u) = LengthUnit::from_name(unit) {
        return Some(match u.absolute_px() {
            Some(f) => CalcLeaf::Length(Length::px(value * f)),
            None => CalcLeaf::Length(Length { value, unit: u }),
        });
    }
    if let Some(u) = AngleUnit::from_name(unit) {
        return Some(CalcLeaf::Angle(value * u.degrees()));
    }
    if let Some(u) = TimeUnit::from_name(unit) {
        return Some(CalcLeaf::Time(value * u.seconds()));
    }
    if let Some(u) = FrequencyUnit::from_name(unit) {
        return Some(CalcLeaf::Frequency(value * u.hertz()));
    }
    ResolutionUnit::from_name(unit).map(|u| CalcLeaf::Resolution(value * u.dppx()))
}

/// The math function names this crate parses.
pub const MATH_FUNCTIONS: &[&str] = &[
    "calc", "min", "max", "clamp", "round", "mod", "rem", "sin", "cos", "tan", "asin", "acos",
    "atan", "atan2", "pow", "sqrt", "hypot", "log", "exp", "abs", "sign",
];

/// True when `name` is a math function this crate parses.
pub fn is_math_function(name: &str) -> bool {
    MATH_FUNCTIONS.iter().any(|m| m.eq_ignore_ascii_case(name))
}

/// Parse a math function (`calc()`, `min()`, `max()`, `clamp()`, `round()`,
/// `mod()`, `rem()`, `sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`,
/// `atan2()`, `pow()`, `sqrt()`, `hypot()`, `log()`, `exp()`, `abs()`,
/// `sign()`).
pub fn parse_math(v: &ComponentValue, limits: &Limits) -> Result<Calc, ValueError> {
    parse_math_depth(v, limits, 0)
}

fn parse_math_depth(v: &ComponentValue, limits: &Limits, depth: usize) -> Result<Calc, ValueError> {
    let ComponentValue::Function(f) = v else {
        return verr("expected a math function", v.span());
    };
    let node = if depth == 0 {
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(s, || {
                    math_function(&f.name, &f.arguments, f.span, limits, depth)
                })
                .expect("failed to spawn CSS math parser stack thread")
                .join()
                .expect("CSS math parser stack thread panicked")
        })?
    } else {
        math_function(&f.name, &f.arguments, f.span, limits, depth)?
    };
    let Some(ty) = type_of(&node) else {
        return verr("math function mixes incompatible types", f.span);
    };
    Ok(Calc {
        node: simplify(node),
        ty,
    })
}

fn math_function(
    name: &str,
    args: &[ComponentValue],
    span: Span,
    limits: &Limits,
    depth: usize,
) -> Result<CalcNode, ValueError> {
    if depth >= limits.max_value_depth {
        return Err(depth_error(limits, span));
    }
    let lower = name.to_ascii_lowercase();
    let groups: Vec<&[ComponentValue]> = args
        .split(|v| matches!(v.token(), Some(Token::Comma)))
        .collect();
    match lower.as_str() {
        "calc" => {
            if groups.len() != 1 {
                return verr("calc() takes one expression", span);
            }
            calc_sum(args, limits, depth + 1)
        }
        "min" | "max" => {
            let mut parts = Vec::new();
            for g in groups {
                parts.push(calc_sum(g, limits, depth + 1)?);
            }
            Ok(if lower == "min" {
                CalcNode::Min(parts)
            } else {
                CalcNode::Max(parts)
            })
        }
        "clamp" => {
            let [a, b, c] = groups.as_slice() else {
                return verr("clamp() takes three arguments", span);
            };
            Ok(CalcNode::Clamp(Box::new([
                calc_sum(a, limits, depth + 1)?,
                calc_sum(b, limits, depth + 1)?,
                calc_sum(c, limits, depth + 1)?,
            ])))
        }
        "round" => {
            let parse_strat = |g: &[ComponentValue]| -> Option<RoundingStrategy> {
                let [one] = trim(g) else { return None };
                let Some(Token::Ident(s)) = one.token() else {
                    return None;
                };
                Some(match s.to_ascii_lowercase().as_str() {
                    "nearest" => RoundingStrategy::Nearest,
                    "up" => RoundingStrategy::Up,
                    "down" => RoundingStrategy::Down,
                    "to-zero" => RoundingStrategy::ToZero,
                    _ => return None,
                })
            };
            match groups.as_slice() {
                [a] => Ok(CalcNode::Round(
                    RoundingStrategy::Nearest,
                    Box::new(calc_sum(a, limits, depth + 1)?),
                    Box::new(CalcNode::Leaf(CalcLeaf::Number(1.0))),
                )),
                [a, b] => {
                    if let Some(strat) = parse_strat(a) {
                        Ok(CalcNode::Round(
                            strat,
                            Box::new(calc_sum(b, limits, depth + 1)?),
                            Box::new(CalcNode::Leaf(CalcLeaf::Number(1.0))),
                        ))
                    } else {
                        Ok(CalcNode::Round(
                            RoundingStrategy::Nearest,
                            Box::new(calc_sum(a, limits, depth + 1)?),
                            Box::new(calc_sum(b, limits, depth + 1)?),
                        ))
                    }
                }
                [s, a, b] => {
                    let Some(strat) = parse_strat(s) else {
                        return verr("invalid rounding strategy in round()", span);
                    };
                    Ok(CalcNode::Round(
                        strat,
                        Box::new(calc_sum(a, limits, depth + 1)?),
                        Box::new(calc_sum(b, limits, depth + 1)?),
                    ))
                }
                _ => verr("round() takes 1 to 3 arguments", span),
            }
        }
        "mod" | "rem" => {
            let [a, b] = groups.as_slice() else {
                return verr(format!("{lower}() takes two arguments"), span);
            };
            let lhs = Box::new(calc_sum(a, limits, depth + 1)?);
            let rhs = Box::new(calc_sum(b, limits, depth + 1)?);
            Ok(if lower == "mod" {
                CalcNode::Mod(lhs, rhs)
            } else {
                CalcNode::Rem(lhs, rhs)
            })
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sqrt" | "exp" | "abs" | "sign" => {
            let [a] = groups.as_slice() else {
                return verr(format!("{lower}() takes one argument"), span);
            };
            let inner = Box::new(calc_sum(a, limits, depth + 1)?);
            Ok(match lower.as_str() {
                "sin" => CalcNode::Sin(inner),
                "cos" => CalcNode::Cos(inner),
                "tan" => CalcNode::Tan(inner),
                "asin" => CalcNode::Asin(inner),
                "acos" => CalcNode::Acos(inner),
                "atan" => CalcNode::Atan(inner),
                "sqrt" => CalcNode::Sqrt(inner),
                "exp" => CalcNode::Exp(inner),
                "abs" => CalcNode::Abs(inner),
                _ => CalcNode::Sign(inner),
            })
        }
        "atan2" | "pow" => {
            let [a, b] = groups.as_slice() else {
                return verr(format!("{lower}() takes two arguments"), span);
            };
            let lhs = Box::new(calc_sum(a, limits, depth + 1)?);
            let rhs = Box::new(calc_sum(b, limits, depth + 1)?);
            Ok(if lower == "atan2" {
                CalcNode::Atan2(lhs, rhs)
            } else {
                CalcNode::Pow(lhs, rhs)
            })
        }
        "hypot" => {
            let mut parts = Vec::new();
            for g in groups {
                parts.push(calc_sum(g, limits, depth + 1)?);
            }
            Ok(CalcNode::Hypot(parts))
        }
        "log" => match groups.as_slice() {
            [a] => Ok(CalcNode::Log(
                Box::new(calc_sum(a, limits, depth + 1)?),
                None,
            )),
            [a, b] => Ok(CalcNode::Log(
                Box::new(calc_sum(a, limits, depth + 1)?),
                Some(Box::new(calc_sum(b, limits, depth + 1)?)),
            )),
            _ => verr("log() takes 1 or 2 arguments", span),
        },
        _ => verr(format!("'{name}()' is not a math function"), span),
    }
}

/// `<calc-sum>`: products joined by `+` and `-`, which need whitespace on
/// both sides.
fn calc_sum(v: &[ComponentValue], limits: &Limits, depth: usize) -> Result<CalcNode, ValueError> {
    // Non-whitespace items, each with "whitespace before" and "after" flags.
    let mut items: Vec<(&ComponentValue, bool, bool)> = Vec::new();
    let mut ws = false;
    for c in v {
        if c.is_whitespace() {
            ws = true;
            if let Some(last) = items.last_mut() {
                last.2 = true;
            }
        } else {
            items.push((c, ws, false));
            ws = false;
        }
    }
    let span = span_of(v);
    if items.is_empty() {
        return verr("empty math expression", span);
    }
    let mut i = 0;
    let mut terms = vec![calc_product(&items, &mut i, limits, depth)?];
    while i < items.len() {
        let (op, before, after) = items[i];
        let sign = if op.is_delim('+') {
            1.0
        } else if op.is_delim('-') {
            -1.0
        } else {
            return verr("expected '+' or '-' between terms", op.span());
        };
        if !before || !after {
            return verr("'+' and '-' need whitespace on both sides", op.span());
        }
        i += 1;
        let t = calc_product(&items, &mut i, limits, depth)?;
        terms.push(if sign < 0.0 {
            CalcNode::Product(vec![CalcNode::Leaf(CalcLeaf::Number(-1.0)), t])
        } else {
            t
        });
    }
    Ok(if terms.len() == 1 {
        terms.remove(0)
    } else {
        CalcNode::Sum(terms)
    })
}

fn calc_product(
    items: &[(&ComponentValue, bool, bool)],
    i: &mut usize,
    limits: &Limits,
    depth: usize,
) -> Result<CalcNode, ValueError> {
    let Some((first, _, _)) = items.get(*i) else {
        return verr("expression ends early", Span::default());
    };
    *i += 1;
    let mut factors = vec![calc_value(first, limits, depth)?];
    while let Some((op, _, _)) = items.get(*i) {
        let invert = if op.is_delim('*') {
            false
        } else if op.is_delim('/') {
            true
        } else {
            break;
        };
        *i += 1;
        let Some((next, _, _)) = items.get(*i) else {
            return verr("expression ends after an operator", op.span());
        };
        *i += 1;
        let f = calc_value(next, limits, depth)?;
        factors.push(if invert {
            CalcNode::Invert(Box::new(f))
        } else {
            f
        });
    }
    Ok(if factors.len() == 1 {
        factors.remove(0)
    } else {
        CalcNode::Product(factors)
    })
}

fn calc_value(v: &ComponentValue, limits: &Limits, depth: usize) -> Result<CalcNode, ValueError> {
    match v {
        ComponentValue::Token(t) => Ok(CalcNode::Leaf(match &t.token {
            Token::Number(n) => CalcLeaf::Number(n.value),
            Token::Percentage(n) => CalcLeaf::Percentage(n.value),
            Token::Dimension { value, unit } => match dimension_leaf(value.value, unit) {
                Some(l) => l,
                None => return verr(format!("unknown unit '{unit}'"), t.span),
            },
            Token::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "e" => CalcLeaf::Number(std::f64::consts::E),
                "pi" => CalcLeaf::Number(std::f64::consts::PI),
                "infinity" => CalcLeaf::Number(f64::INFINITY),
                "-infinity" => CalcLeaf::Number(f64::NEG_INFINITY),
                "nan" => CalcLeaf::Number(f64::NAN),
                _ => return verr(format!("unexpected '{s}' in a math expression"), t.span),
            },
            _ => return verr("unexpected token in a math expression", t.span),
        })),
        ComponentValue::Block(b) if b.kind == BlockKind::Paren => {
            if depth >= limits.max_value_depth {
                return Err(depth_error(limits, b.span));
            }
            calc_sum(&b.contents, limits, depth + 1)
        }
        ComponentValue::Function(f) => {
            math_function(&f.name, &f.arguments, f.span, limits, depth + 1)
        }
        ComponentValue::Block(b) => verr("unexpected block in a math expression", b.span),
    }
}

// ---- property values ----

fn trim(mut v: &[ComponentValue]) -> &[ComponentValue] {
    while v.first().is_some_and(|x| x.is_whitespace()) {
        v = &v[1..];
    }
    while v.last().is_some_and(|x| x.is_whitespace()) {
        v = &v[..v.len() - 1];
    }
    v
}

/// The functions whose presence defers parsing to computed-value time.
pub const SUBSTITUTION_FUNCTIONS: &[&str] = &["var", "env", "attr"];

/// True when `values` uses `var()`, `env()`, or `attr()` at any depth.
/// Uses a work list, not recursion.
pub fn contains_substitution(values: &[ComponentValue]) -> bool {
    let mut stack: Vec<&[ComponentValue]> = vec![values];
    while let Some(list) = stack.pop() {
        for v in list {
            match v {
                ComponentValue::Function(f) => {
                    if SUBSTITUTION_FUNCTIONS
                        .iter()
                        .any(|n| f.name.eq_ignore_ascii_case(n))
                    {
                        return true;
                    }
                    stack.push(&f.arguments);
                }
                ComponentValue::Block(b) => stack.push(&b.contents),
                ComponentValue::Token(_) => {}
            }
        }
    }
    false
}

/// Parse `src` as the value of `property`, with default limits.
pub fn parse_value(property: &str, src: &str) -> Result<PropertyValue, ValueError> {
    let options = Options::default();
    let unicode = property.eq_ignore_ascii_case("unicode-range");
    let tokenizer = Tokenizer::new(src).with_unicode_ranges(unicode);
    let values =
        crate::prism::parser::parse_component_values_tokens(tokenizer, src.len(), &options)
            .map_err(|e| ValueError {
                message: e.to_string(),
                span: Span::new(0, src.len()),
                limit: match e {
                    crate::prism::error::Error::Limit(l) => Some(l.kind),
                },
            })?;
    parse_value_from(property, &values, &options.limits)
}

/// Parse component values as the value of `property`.
pub fn parse_value_from(
    property: &str,
    values: &[ComponentValue],
    limits: &Limits,
) -> Result<PropertyValue, ValueError> {
    let v = trim(values);
    let custom = property.starts_with("--");
    if let [one] = v
        && let Some(Token::Ident(s)) = one.token()
        && let Some(g) = GlobalKeyword::from_name(s)
    {
        return Ok(PropertyValue::Global(g));
    }
    if custom {
        return Ok(PropertyValue::Custom(v.to_vec()));
    }
    let span = span_of(values);
    if v.is_empty() {
        return verr("empty value", span);
    }
    if contains_substitution(v) {
        return Ok(PropertyValue::Unresolved(v.to_vec()));
    }
    let name = property.to_ascii_lowercase();
    if name == "unicode-range" {
        return unicode_ranges(v).map(PropertyValue::Values);
    }
    let Some(def) = properties::property(&name) else {
        return verr(format!("unknown property '{property}'"), span);
    };
    parse_grammar(&def.grammar, v, limits).map(PropertyValue::Values)
}

fn unicode_ranges(v: &[ComponentValue]) -> Result<Vec<Value>, ValueError> {
    let mut out = Vec::new();
    for (k, group) in v
        .split(|x| matches!(x.token(), Some(Token::Comma)))
        .enumerate()
    {
        if k > 0 {
            out.push(Value::Comma);
        }
        match trim(group) {
            [one] => match one.token() {
                Some(Token::UnicodeRange { start, end }) if start <= end && *end <= 0x10FFFF => {
                    out.push(Value::UnicodeRange(*start, *end));
                }
                _ => return verr("invalid unicode range", one.span()),
            },
            _ => return verr("invalid unicode range", span_of(group)),
        }
    }
    Ok(out)
}

fn keyword_in(s: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| k.eq_ignore_ascii_case(s))
}

fn number_value(n: f64, is_integer: bool) -> Value {
    if is_integer && n.abs() < 9.0e15 {
        Value::Integer(n as i64)
    } else {
        Value::Number(n)
    }
}

/// Parse one component value with no grammar beyond its own type.
pub fn parse_component(v: &ComponentValue, limits: &Limits) -> Result<Value, ValueError> {
    component(v, limits, 0)
}

fn component(v: &ComponentValue, limits: &Limits, depth: usize) -> Result<Value, ValueError> {
    match v {
        ComponentValue::Token(t) => match &t.token {
            Token::Ident(s) => Ok(Value::Ident(s.clone())),
            Token::Number(n) => Ok(number_value(n.value, n.is_integer)),
            Token::Percentage(n) => Ok(Value::Percentage(n.value)),
            Token::Dimension { value, unit } => {
                let x = value.value;
                if let Some(u) = LengthUnit::from_name(unit) {
                    Ok(Value::Length(Length { value: x, unit: u }))
                } else if let Some(u) = AngleUnit::from_name(unit) {
                    Ok(Value::Angle(x, u))
                } else if let Some(u) = TimeUnit::from_name(unit) {
                    Ok(Value::Time(x, u))
                } else if let Some(u) = FrequencyUnit::from_name(unit) {
                    Ok(Value::Frequency(x, u))
                } else if let Some(u) = ResolutionUnit::from_name(unit) {
                    Ok(Value::Resolution(x, u))
                } else if unit.eq_ignore_ascii_case("fr") {
                    Ok(Value::Flex(x))
                } else {
                    verr(format!("unknown unit '{unit}'"), t.span)
                }
            }
            Token::String(s) => Ok(Value::String(s.clone())),
            Token::Url(u) => Ok(Value::Url(u.clone())),
            Token::Hash { value, .. } => match parse_hex_color(value) {
                Some(c) => Ok(Value::Color(c)),
                None => verr("invalid hex color", t.span),
            },
            Token::Comma => Ok(Value::Comma),
            Token::Delim('/') => Ok(Value::Slash),
            Token::UnicodeRange { start, end } if start <= end && *end <= 0x10FFFF => {
                Ok(Value::UnicodeRange(*start, *end))
            }
            Token::BadString => verr("unterminated string", t.span),
            Token::BadUrl => verr("invalid url()", t.span),
            _ => verr("unexpected token in a value", t.span),
        },
        ComponentValue::Function(f) => {
            if depth >= limits.max_value_depth {
                return Err(depth_error(limits, f.span));
            }
            if is_math_function(&f.name) {
                return parse_math_depth(v, limits, depth).map(|c| Value::Calc(Box::new(c)));
            }
            if let Some(c) = color_function(&f.name, &f.arguments, f.span, limits, depth + 1) {
                return c.map(Value::Color);
            }
            if f.name.eq_ignore_ascii_case("url") {
                return match crate::prism::parser::url_or_string(v) {
                    Some(u) => Ok(Value::Url(u)),
                    None => verr("invalid url()", f.span),
                };
            }
            let mut args = Vec::new();
            for a in &f.arguments {
                if !a.is_whitespace() {
                    args.push(component(a, limits, depth + 1)?);
                }
            }
            Ok(Value::Function {
                name: f.name.to_ascii_lowercase(),
                args,
            })
        }
        ComponentValue::Block(b) if b.kind == BlockKind::Bracket => {
            let mut names = Vec::new();
            for x in &b.contents {
                match x.token() {
                    Some(Token::Whitespace) => {}
                    Some(Token::Ident(s)) => names.push(s.clone()),
                    _ => return verr("line names may only contain identifiers", b.span),
                }
            }
            Ok(Value::LineNames(names))
        }
        ComponentValue::Block(b) => verr("unexpected block in a value", b.span),
    }
}

fn generic(v: &[ComponentValue], limits: &Limits) -> Result<Vec<Value>, ValueError> {
    let mut out = Vec::new();
    for c in v {
        if !c.is_whitespace() {
            out.push(component(c, limits, 0)?);
        }
    }
    Ok(out)
}

fn negative(v: f64, nonneg: bool, span: Span) -> Result<(), ValueError> {
    if nonneg && v < 0.0 {
        return verr("negative values are not allowed here", span);
    }
    Ok(())
}

fn length_item(
    v: &ComponentValue,
    keywords: &[&str],
    percent: bool,
    nonneg: bool,
    limits: &Limits,
) -> Result<Value, ValueError> {
    let span = v.span();
    match v {
        ComponentValue::Token(t) => match &t.token {
            Token::Ident(s) if keyword_in(s, keywords) => Ok(Value::Ident(s.to_ascii_lowercase())),
            Token::Dimension { value, unit } => match LengthUnit::from_name(unit) {
                Some(u) => {
                    negative(value.value, nonneg, span)?;
                    Ok(Value::Length(Length {
                        value: value.value,
                        unit: u,
                    }))
                }
                None => verr(format!("'{unit}' is not a length unit"), span),
            },
            Token::Number(n) if n.value == 0.0 => Ok(Value::Length(Length::px(0.0))),
            Token::Percentage(n) if percent => {
                negative(n.value, nonneg, span)?;
                Ok(Value::Percentage(n.value))
            }
            _ => verr("expected a length", span),
        },
        ComponentValue::Function(f) if is_math_function(&f.name) => {
            let c = parse_math(v, limits)?;
            let ok = match c.ty {
                CalcType::Length => true,
                CalcType::Percentage | CalcType::LengthPercentage => percent,
                _ => false,
            };
            if !ok {
                return verr("math function has the wrong type for a length", span);
            }
            Ok(Value::Calc(Box::new(c)))
        }
        _ => verr("expected a length", span),
    }
}

fn single(v: &[ComponentValue]) -> Result<&ComponentValue, ValueError> {
    match v {
        [one] => Ok(one),
        _ => verr("expected a single value", span_of(v)),
    }
}

fn parse_grammar(
    g: &Grammar,
    v: &[ComponentValue],
    limits: &Limits,
) -> Result<Vec<Value>, ValueError> {
    match g {
        Grammar::Generic => generic(v, limits),
        Grammar::KeywordList { keywords, max } => {
            let items: Vec<&ComponentValue> = v.iter().filter(|x| !x.is_whitespace()).collect();
            if items.is_empty() || items.len() > usize::from(*max) {
                return verr(format!("expected 1 to {max} keywords"), span_of(v));
            }
            let mut out: Vec<Value> = Vec::new();
            for item in items {
                match item.token() {
                    Some(Token::Ident(s)) if keyword_in(s, keywords) => {
                        let k = Value::Ident(s.to_ascii_lowercase());
                        if out.contains(&k) {
                            return verr("keyword repeated", item.span());
                        }
                        out.push(k);
                    }
                    _ => return verr("not one of the allowed keywords", item.span()),
                }
            }
            Ok(out)
        }
        Grammar::Keywords(list) => {
            let one = single(v)?;
            match one.token() {
                Some(Token::Ident(s)) if keyword_in(s, list) => {
                    Ok(vec![Value::Ident(s.to_ascii_lowercase())])
                }
                _ => verr("not one of the allowed keywords", one.span()),
            }
        }
        Grammar::Length {
            keywords,
            percent,
            nonneg,
        } => Ok(vec![length_item(
            single(v)?,
            keywords,
            *percent,
            *nonneg,
            limits,
        )?]),
        Grammar::LengthList {
            keywords,
            percent,
            nonneg,
            max,
        } => {
            let items: Vec<&ComponentValue> = v.iter().filter(|x| !x.is_whitespace()).collect();
            if items.is_empty() || items.len() > usize::from(*max) {
                return verr(format!("expected 1 to {max} values"), span_of(v));
            }
            items
                .into_iter()
                .map(|x| length_item(x, keywords, *percent, *nonneg, limits))
                .collect()
        }
        Grammar::Number {
            keywords,
            percent,
            nonneg,
        } => {
            let one = single(v)?;
            let span = one.span();
            let value = match one {
                ComponentValue::Token(t) => match &t.token {
                    Token::Ident(s) if keyword_in(s, keywords) => {
                        Value::Ident(s.to_ascii_lowercase())
                    }
                    Token::Number(n) => {
                        negative(n.value, *nonneg, span)?;
                        number_value(n.value, n.is_integer)
                    }
                    Token::Percentage(n) if *percent => {
                        negative(n.value, *nonneg, span)?;
                        Value::Percentage(n.value)
                    }
                    _ => return verr("expected a number", span),
                },
                ComponentValue::Function(f) if is_math_function(&f.name) => {
                    let c = parse_math(one, limits)?;
                    let ok = c.ty == CalcType::Number || (*percent && c.ty == CalcType::Percentage);
                    if !ok {
                        return verr("math function has the wrong type for a number", span);
                    }
                    Value::Calc(Box::new(c))
                }
                _ => return verr("expected a number", span),
            };
            Ok(vec![value])
        }
        Grammar::Integer { keywords } => {
            let one = single(v)?;
            let span = one.span();
            match one {
                ComponentValue::Token(t) => match &t.token {
                    Token::Ident(s) if keyword_in(s, keywords) => {
                        Ok(vec![Value::Ident(s.to_ascii_lowercase())])
                    }
                    Token::Number(n) if n.is_integer => Ok(vec![number_value(n.value, true)]),
                    _ => verr("expected an integer", span),
                },
                ComponentValue::Function(f) if is_math_function(&f.name) => {
                    let c = parse_math(one, limits)?;
                    if c.ty != CalcType::Number {
                        return verr("math function has the wrong type for an integer", span);
                    }
                    Ok(vec![Value::Calc(Box::new(c))])
                }
                _ => verr("expected an integer", span),
            }
        }
        Grammar::Color { keywords } => {
            let one = single(v)?;
            if let Some(Token::Ident(s)) = one.token()
                && keyword_in(s, keywords)
            {
                return Ok(vec![Value::Ident(s.to_ascii_lowercase())]);
            }
            Ok(vec![Value::Color(parse_color_value(one, limits)?)])
        }
        Grammar::TimeList => {
            let mut out = Vec::new();
            for (k, group) in v
                .split(|x| matches!(x.token(), Some(Token::Comma)))
                .enumerate()
            {
                if k > 0 {
                    out.push(Value::Comma);
                }
                let one = single(trim(group))?;
                let span = one.span();
                match one {
                    ComponentValue::Token(t) => match &t.token {
                        Token::Dimension { value, unit } => match TimeUnit::from_name(unit) {
                            Some(u) => out.push(Value::Time(value.value, u)),
                            None => return verr("expected a time", span),
                        },
                        _ => return verr("expected a time", span),
                    },
                    ComponentValue::Function(f) if is_math_function(&f.name) => {
                        let c = parse_math(one, limits)?;
                        if c.ty != CalcType::Time {
                            return verr("math function has the wrong type for a time", span);
                        }
                        out.push(Value::Calc(Box::new(c)));
                    }
                    _ => return verr("expected a time", span),
                }
            }
            Ok(out)
        }
    }
}
