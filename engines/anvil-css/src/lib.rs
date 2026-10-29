// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The Anvil-CSS engine (`engines/anvil-css`), linked directly into `anvil-style`.
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
//! use anvil_style::anvil_css::{parse_stylesheet, Options, Rule};
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

#[cfg(anvil_cpp_css)]
unsafe extern "C" {
    fn anvil_css_bloom_hash(data: *const u8, len: usize) -> u64;
    fn anvil_css_skip_whitespace_and_comments(data: *const u8, len: usize) -> usize;
    fn anvil_css_mix_srgb_rgba8(rgba_a: u32, rgba_b: u32, w: f32) -> u32;
}

/// Compute a 64-bit ASCII-lowercased FNV-1a bloom hash for selector matching.
/// Dispatches to the native C++20 `anvil_css_engine` kernel when linked.
pub fn bloom_hash_ident(ident: &str) -> u64 {
    #[cfg(anvil_cpp_css)]
    unsafe {
        return anvil_css_bloom_hash(ident.as_ptr(), ident.len());
    }
    #[cfg(not(anvil_cpp_css))]
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

/// Skip leading CSS whitespace and `/* ... */` comments using the C++20 `anvil_css_engine` kernel.
pub fn skip_css_trivia(input: &str) -> usize {
    #[cfg(anvil_cpp_css)]
    unsafe {
        return anvil_css_skip_whitespace_and_comments(input.as_ptr(), input.len());
    }
    #[cfg(not(anvil_cpp_css))]
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
    #[cfg(anvil_cpp_css)]
    unsafe {
        return anvil_css_mix_srgb_rgba8(rgba_a, rgba_b, w);
    }
    #[cfg(not(anvil_cpp_css))]
    {
        let w = w.clamp(0.0, 1.0);
        let inv = 1.0 - w;
        let mix_ch = |shift: u32| -> u32 {
            let a = ((rgba_a >> shift) & 0xFF) as f32;
            let b = ((rgba_b >> shift) & 0xFF) as f32;
            ((a * inv + b * w + 0.5).clamp(0.0, 255.0)) as u32
        };
        (mix_ch(24) << 24) | (mix_ch(16) << 16) | (mix_ch(8) << 8) | mix_ch(0)
    }
}
