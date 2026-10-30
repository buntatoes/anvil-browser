// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The Prism engine (`engines/prism`), linked directly into `anvil-style`.
//!
//! Turns CSS text into typed data: tokens, rules, selectors, property values,
//! and media, container, and support conditions. It matches selectors against
//! any element tree that implements [`selectors::Element`], and evaluates media
//! queries against a host environment.
//!
//! Every input size and nesting depth is bounded by [`Limits`]. Going past
//! a whole-input limit returns [`Error`]; going past a nesting limit drops
//! the smallest enclosing construct and records a [`Diagnostic`].
//!
//! ```
//! use anvil_style::prism::{parse_stylesheet, Options, Rule};
//!
//! let sheet = parse_stylesheet("p.note { color: red; margin: 0 4px }", &Options::default())
//!     .expect("within limits");
//! let Rule::Style(rule) = &sheet.rules[0] else { panic!("expected a style rule") };
//! assert_eq!(rule.declarations.len(), 2);
//! assert_eq!(rule.declarations[0].name, "color");
//! assert!(sheet.diagnostics.is_empty());
//! ```

#![warn(missing_docs)]

pub mod error;
pub mod media;
pub mod parser;
pub mod properties;
pub mod selectors;
pub mod serialize;
pub mod tokenizer;
pub mod values;

pub use error::{Diagnostic, DiagnosticKind, Error, LimitError, LimitKind, Limits, Span};
pub use parser::{
    ComponentValue, Declaration, DeclarationBlock, Options, PropertyRule, Rule, ScopeRule,
    StartingStyleRule, Stylesheet, parse_component_values, parse_declarations,
    parse_declarations_with, parse_stylesheet,
};
pub use selectors::{
    SelectorError, SelectorList, parse_forgiving_selector_list, parse_selector_list,
};
pub use values::{
    Color, Length, LengthUnit, NAMED_COLORS, PropertyValue, ResolveContext, Rgba, SystemColor,
    Value, ValueError, color_keyword, parse_css_color, parse_css_rgba8, parse_hex_color,
    parse_value,
};

/// Embedded WHATWG HTML5 User-Agent Default Stylesheet (`engines/prism/stylesheets/ua_html5_default.css`).
pub const UA_HTML5_DEFAULT_CSS: &str = include_str!("../stylesheets/ua_html5_default.css");

/// Embedded Form & Interactive Control Stylesheet (`engines/prism/stylesheets/ua_forms_controls.css`).
pub const UA_FORMS_CONTROLS_CSS: &str = include_str!("../stylesheets/ua_forms_controls.css");

/// Embedded Tables, Lists & Media Stylesheet (`engines/prism/stylesheets/ua_tables_lists_media.css`).
pub const UA_TABLES_LISTS_MEDIA_CSS: &str =
    include_str!("../stylesheets/ua_tables_lists_media.css");

/// Embedded Forge Design Tokens & Layout Utilities (`engines/prism/stylesheets/prism_design_tokens.css`).
pub const PRISM_DESIGN_TOKENS_CSS: &str = include_str!("../stylesheets/prism_design_tokens.css");

/// Return all 4 built-in Prism engine user-agent and token stylesheets.
pub fn builtin_user_agent_stylesheets() -> [&'static str; 4] {
    [
        UA_HTML5_DEFAULT_CSS,
        UA_FORMS_CONTROLS_CSS,
        UA_TABLES_LISTS_MEDIA_CSS,
        PRISM_DESIGN_TOKENS_CSS,
    ]
}

#[cfg(prism_cpp_css)]
unsafe extern "C" {
    fn prism_css_bloom_hash(data: *const u8, len: usize) -> u64;
    fn prism_css_skip_whitespace_and_comments(data: *const u8, len: usize) -> usize;
    fn prism_css_mix_srgb_rgba8(rgba_a: u32, rgba_b: u32, w: f32) -> u32;
    fn prism_css_classify_pseudo_element(data: *const u8, len: usize) -> u32;
    fn prism_css_resolve_length_px(
        value: f32,
        unit_code: u32,
        font_size_px: f32,
        root_font_size_px: f32,
        viewport_w: f32,
        viewport_h: f32,
        container_px: f32,
    ) -> f32;
    fn prism_css_eval_clamp(min_px: f32, preferred_px: f32, max_px: f32) -> f32;
    fn prism_css_sample_radial_gradient(
        norm_x: f32,
        norm_y: f32,
        inner_rgba: u32,
        outer_rgba: u32,
    ) -> u32;
    fn prism_css_pack_specificity(id_count: u32, class_count: u32, element_count: u32) -> u32;
}

/// Compute a 64-bit ASCII-lowercased FNV-1a bloom hash for selector matching.
/// Dispatches to the native C++20 `prism_css_engine` kernel when linked.
pub fn bloom_hash_ident(ident: &str) -> u64 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_bloom_hash(ident.as_ptr(), ident.len())
    }
    #[cfg(not(prism_cpp_css))]
    {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for &b in ident.as_bytes() {
            let lower = if b.is_ascii_uppercase() { b + 32 } else { b };
            hash ^= u64::from(lower);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }
}

/// Skip leading CSS whitespace and `/* ... */` comments using the C++20 `prism_css_engine` kernel.
pub fn skip_css_trivia(input: &str) -> usize {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_skip_whitespace_and_comments(input.as_ptr(), input.len())
    }
    #[cfg(not(prism_cpp_css))]
    {
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0C) {
                i += 1;
                continue;
            }
            if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = if i + 1 < bytes.len() {
                    i + 2
                } else {
                    bytes.len()
                };
                continue;
            }
            break;
        }
        i
    }
}

/// Mix two packed RGBA8 colors (`0xRRGGBBAA`) in sRGB space with weight `w` in `[0.0, 1.0]`.
pub fn mix_srgb_rgba8(rgba_a: u32, rgba_b: u32, w: f32) -> u32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_mix_srgb_rgba8(rgba_a, rgba_b, w)
    }
    #[cfg(not(prism_cpp_css))]
    {
        let w = w.clamp(0.0, 1.0);
        let inv = 1.0 - w;
        let r = (((rgba_a >> 24) & 0xFF) as f32 * inv + ((rgba_b >> 24) & 0xFF) as f32 * w).round()
            as u32;
        let g = (((rgba_a >> 16) & 0xFF) as f32 * inv + ((rgba_b >> 16) & 0xFF) as f32 * w).round()
            as u32;
        let b = (((rgba_a >> 8) & 0xFF) as f32 * inv + ((rgba_b >> 8) & 0xFF) as f32 * w).round()
            as u32;
        let a = ((rgba_a & 0xFF) as f32 * inv + (rgba_b & 0xFF) as f32 * w).round() as u32;
        (r << 24) | (g << 16) | (b << 8) | a
    }
}

/// Classify a trailing pseudo-element token (`1 = ::before`, `2 = ::after`, etc.) via C++20 `prism_css_engine`.
pub fn classify_pseudo_element_fast(selector: &str) -> u32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_classify_pseudo_element(selector.as_ptr(), selector.len())
    }
    #[cfg(not(prism_cpp_css))]
    {
        let s = selector.trim();
        let Some((_, tail)) = s.rsplit_once(':') else {
            return 0;
        };
        match tail.to_ascii_lowercase().as_str() {
            "before" => 1,
            "after" => 2,
            "first-line" => 3,
            "first-letter" => 4,
            "placeholder" => 5,
            "selection" => 6,
            "marker" => 7,
            "backdrop" => 8,
            _ => 0,
        }
    }
}

/// Resolve a CSS dimension into pixels via the native C++20 `prism_css_engine` kernel.
pub fn resolve_length_px_fast(
    value: f32,
    unit_code: u32,
    font_size_px: f32,
    root_font_size_px: f32,
    viewport_w: f32,
    viewport_h: f32,
    container_px: f32,
) -> f32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_resolve_length_px(
            value,
            unit_code,
            font_size_px,
            root_font_size_px,
            viewport_w,
            viewport_h,
            container_px,
        )
    }
    #[cfg(not(prism_cpp_css))]
    {
        match unit_code {
            1 => value * font_size_px,
            2 => value * root_font_size_px,
            3 => value * 0.01 * container_px,
            4 => value * 0.01 * viewport_w,
            5 => value * 0.01 * viewport_h,
            6 => value * 0.01 * viewport_w.min(viewport_h),
            7 => value * 0.01 * viewport_w.max(viewport_h),
            8 | 9 => value * font_size_px * 0.5,
            10 => value * (96.0 / 72.0),
            11 => value * 16.0,
            12 => value * (96.0 / 2.54),
            13 => value * (96.0 / 25.4),
            14 => value * 96.0,
            15 => value * (96.0 / 101.6),
            _ => value,
        }
    }
}

/// Evaluate CSS `clamp(min_px, preferred_px, max_px)` via C++20 `prism_css_engine`.
pub fn eval_clamp_fast(min_px: f32, preferred_px: f32, max_px: f32) -> f32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_eval_clamp(min_px, preferred_px, max_px)
    }
    #[cfg(not(prism_cpp_css))]
    {
        preferred_px.min(max_px).max(min_px)
    }
}

/// Sample a 2-stop radial gradient at `(norm_x, norm_y)` in `[0.0, 1.0]` via C++20 `prism_css_engine`.
pub fn sample_radial_gradient_fast(
    norm_x: f32,
    norm_y: f32,
    inner_rgba: u32,
    outer_rgba: u32,
) -> u32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_sample_radial_gradient(norm_x, norm_y, inner_rgba, outer_rgba)
    }
    #[cfg(not(prism_cpp_css))]
    {
        let dx = (norm_x - 0.5) * 2.0;
        let dy = (norm_y - 0.5) * 2.0;
        let dist = (dx * dx + dy * dy).sqrt().clamp(0.0, 1.0);
        mix_srgb_rgba8(inner_rgba, outer_rgba, dist)
    }
}

/// Pack Selectors Level 4 specificity `(ids, classes, elements)` via C++20 `prism_css_engine`.
pub fn pack_specificity_fast(id_count: u32, class_count: u32, element_count: u32) -> u32 {
    #[cfg(prism_cpp_css)]
    unsafe {
        prism_css_pack_specificity(id_count, class_count, element_count)
    }
    #[cfg(not(prism_cpp_css))]
    {
        (id_count.min(1023) << 20) | (class_count.min(1023) << 10) | element_count.min(1023)
    }
}
