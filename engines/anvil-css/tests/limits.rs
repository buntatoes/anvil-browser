// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Every limit in `Limits` is reported, either as an error or as a
//! `LimitReached` diagnostic. Nothing is cut off silently.

use anvil_css::error::{Error, LimitKind, Limits};
use anvil_css::parser::parse_component_values;
use anvil_css::selectors::{SelectorOptions, parse_selector_list_with};
use anvil_css::{DiagnosticKind, Options, Rule, parse_declarations_with, parse_stylesheet};

fn options(f: impl FnOnce(&mut Limits)) -> Options {
    let mut o = Options::default();
    f(&mut o.limits);
    o
}

fn has_limit_diag(sheet: &anvil_css::Stylesheet, kind: LimitKind) -> bool {
    sheet
        .diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::LimitReached(kind))
}

fn nested(open: &str, close: &str, n: usize, middle: &str) -> String {
    let mut s = open.repeat(n);
    s.push_str(middle);
    s.push_str(&close.repeat(n));
    s
}

#[test]
fn default_limits_documented_values() {
    let l = Limits::default();
    assert_eq!(l.max_input_bytes, 320 << 20);
    assert_eq!(l.max_tokens, 80_000_000);
    assert_eq!(l.max_depth, 1_280);
    assert_eq!(l.max_rules, 10_000_000);
    assert_eq!(l.max_selector_depth, 320);
    assert_eq!(l.max_selector_length, 2_560);
    assert_eq!(l.max_value_depth, 320);
    assert_eq!(l.max_condition_depth, 320);
}

#[test]
fn input_bytes_is_an_error() {
    let o = options(|l| l.max_input_bytes = 10);
    let Err(Error::Limit(e)) = parse_stylesheet("a { color: red }", &o) else {
        panic!()
    };
    assert_eq!(e.kind, LimitKind::InputBytes);
    assert_eq!(e.limit, 10);
    assert_eq!(e.found, 16);
}

#[test]
fn input_bytes_exact_is_fine() {
    let o = options(|l| l.max_input_bytes = 16);
    assert!(parse_stylesheet("a { color: red }", &o).is_ok());
}

#[test]
fn tokens_is_an_error() {
    let o = options(|l| l.max_tokens = 5);
    let Err(Error::Limit(e)) = parse_stylesheet("a { color: red }", &o) else {
        panic!()
    };
    assert_eq!(e.kind, LimitKind::Tokens);
}

#[test]
fn tokens_limit_applies_to_declarations() {
    let o = options(|l| l.max_tokens = 2);
    assert!(parse_declarations_with("color: red", &o).is_err());
}

#[test]
fn rules_is_an_error() {
    let o = options(|l| l.max_rules = 3);
    let Err(Error::Limit(e)) = parse_stylesheet("a{} b{} c{} d{}", &o) else {
        panic!()
    };
    assert_eq!(e.kind, LimitKind::Rules);
    assert_eq!(e.limit, 3);
}

#[test]
fn rules_counts_nested_rules() {
    let o = options(|l| l.max_rules = 3);
    assert!(parse_stylesheet("@media screen { a{} b{} c{} }", &o).is_err());
    assert!(parse_stylesheet("@media screen { a{} b{} }", &o).is_ok());
}

#[test]
fn error_message_names_the_limit() {
    let o = options(|l| l.max_rules = 1);
    let e = parse_stylesheet("a{} b{}", &o).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("rules"), "{text}");
    assert!(text.contains('1'), "{text}");
}

#[test]
fn depth_drops_rule_with_diagnostic() {
    let o = options(|l| l.max_depth = 10);
    let src = format!(
        "{} b {{ color: red }}",
        nested("@media screen { ", "}", 20, "a {}")
    );
    let s = parse_stylesheet(&src, &o).unwrap();
    assert!(has_limit_diag(&s, LimitKind::Depth), "{:?}", s.diagnostics);
    assert!(
        s.rules.iter().any(|r| matches!(r, Rule::Style(_))),
        "later rules survive"
    );
}

#[test]
fn depth_in_declaration_value_drops_declaration() {
    let o = options(|l| l.max_depth = 10);
    let src = format!("a {{ x: {}; color: red }}", nested("(", ")", 50, "1"));
    let s = parse_stylesheet(&src, &o).unwrap();
    assert!(has_limit_diag(&s, LimitKind::Depth));
    let Rule::Style(r) = &s.rules[0] else {
        panic!()
    };
    assert_eq!(r.declarations.len(), 1);
    assert_eq!(r.declarations[0].name, "color");
}

#[test]
fn depth_in_component_values_is_an_error() {
    let o = options(|l| l.max_depth = 10);
    let Err(Error::Limit(e)) = parse_component_values(&nested("[", "]", 50, ""), &o) else {
        panic!()
    };
    assert_eq!(e.kind, LimitKind::Depth);
}

#[test]
fn default_depth_handles_deep_input_without_overflow() {
    let src = nested("{", "}", 100_000, "");
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(s.rules.is_empty());
    let src = nested("a{", "}", 5_000, "");
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(has_limit_diag(&s, LimitKind::Depth));
}

#[test]
fn selector_depth_error() {
    let opts = SelectorOptions {
        max_depth: 5,
        ..SelectorOptions::default()
    };
    let e = parse_selector_list_with(&nested(":is(", ")", 10, "a"), &opts).unwrap_err();
    assert_eq!(e.limit, Some(LimitKind::SelectorDepth));
}

#[test]
fn selector_depth_diagnostic_in_sheet() {
    let src = format!("{} {{ color: red }}", nested(":is(", ")", 350, "a"));
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(
        has_limit_diag(&s, LimitKind::SelectorDepth),
        "{:?}",
        s.diagnostics
    );
}

#[test]
fn selector_length_error() {
    let opts = SelectorOptions {
        max_length: 10,
        ..SelectorOptions::default()
    };
    let e = parse_selector_list_with(&vec!["a"; 20].join(" "), &opts).unwrap_err();
    assert_eq!(e.limit, Some(LimitKind::SelectorLength));
}

#[test]
fn selector_length_diagnostic_in_sheet() {
    let src = format!("{} {{ color: red }}", vec!["a"; 2_600].join(" "));
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(
        has_limit_diag(&s, LimitKind::SelectorLength),
        "{:?}",
        s.diagnostics
    );
}

#[test]
fn value_depth_error() {
    let e = anvil_css::parse_value("width", &nested("calc(", ")", 350, "1px")).unwrap_err();
    assert_eq!(e.limit, Some(LimitKind::ValueDepth));
}

#[test]
fn condition_depth_media_diagnostic() {
    let src = format!("@media {} {{ a {{}} }}", nested("(", ")", 350, "color"));
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(
        has_limit_diag(&s, LimitKind::ConditionDepth),
        "{:?}",
        s.diagnostics
    );
}

#[test]
fn condition_depth_supports_diagnostic() {
    let src = format!(
        "@supports {} {{ a {{}} }}",
        nested("(", ")", 350, "color: red")
    );
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(
        has_limit_diag(&s, LimitKind::ConditionDepth),
        "{:?}",
        s.diagnostics
    );
}

#[test]
fn condition_depth_container_diagnostic() {
    let src = format!(
        "@container {} {{ a {{}} }}",
        nested("(", ")", 350, "width > 1px")
    );
    let s = parse_stylesheet(&src, &Options::default()).unwrap();
    assert!(
        has_limit_diag(&s, LimitKind::ConditionDepth),
        "{:?}",
        s.diagnostics
    );
}

#[test]
fn every_limit_kind_has_a_name() {
    for k in [
        LimitKind::InputBytes,
        LimitKind::Tokens,
        LimitKind::Depth,
        LimitKind::Rules,
        LimitKind::SelectorDepth,
        LimitKind::SelectorLength,
        LimitKind::ValueDepth,
        LimitKind::ConditionDepth,
    ] {
        assert!(!k.name().is_empty());
    }
}
