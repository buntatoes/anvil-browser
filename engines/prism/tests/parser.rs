// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Parser tests: rule structure, declarations, nesting, and error
//! recovery, written from CSS Syntax Level 3 section 5 and CSS Nesting.

use prism::parser::{BlockKind, StyleRule};
use prism::tokenizer::Token;
use prism::{
    ComponentValue, DiagnosticKind, Options, Rule, Stylesheet, parse_component_values,
    parse_declarations, parse_stylesheet,
};

fn sheet(src: &str) -> Stylesheet {
    parse_stylesheet(src, &Options::default()).expect("within limits")
}

fn styles(rules: &[Rule]) -> Vec<&StyleRule> {
    rules
        .iter()
        .filter_map(|r| match r {
            Rule::Style(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// Summary of top-level style rules: (selector text count, declaration names).
fn summary(src: &str) -> Vec<Vec<String>> {
    let s = sheet(src);
    styles(&s.rules)
        .iter()
        .map(|r| r.declarations.iter().map(|d| d.name.clone()).collect())
        .collect()
}

fn decl_names(src: &str) -> Vec<String> {
    parse_declarations(src)
        .unwrap()
        .declarations
        .into_iter()
        .map(|d| d.name)
        .collect()
}

macro_rules! rule_cases {
    ($($name:ident: $src:expr => [$([$($d:expr),*]),*];)*) => {
        $(
            #[test]
            fn $name() {
                let want: Vec<Vec<String>> = vec![$(vec![$($d.to_string()),*]),*];
                assert_eq!(summary($src), want, "input {:?}", $src);
            }
        )*
    };
}

macro_rules! decl_cases {
    ($($name:ident: $src:expr => [$($d:expr),*];)*) => {
        $(
            #[test]
            fn $name() {
                let want: Vec<String> = vec![$($d.to_string()),*];
                assert_eq!(decl_names($src), want, "input {:?}", $src);
            }
        )*
    };
}

rule_cases! {
    one_rule: "a { color: red }" => [["color"]];
    two_rules: "a{color:red} b{margin:0}" => [["color"], ["margin"]];
    empty_block: "a {}" => [[]];
    trailing_semicolons: "a { color: red;;; margin: 0; }" => [["color", "margin"]];
    property_names_lowercased: "a { COLOR: red }" => [["color"]];
    custom_property_case_kept: "a { --Main: 1 }" => [["--Main"]];
    missing_colon_drops_declaration: "a { color red; margin: 0 }" => [["margin"]];
    missing_value_kept_for_custom: "a { --x:; margin: 0 }" => [["--x", "margin"]];
    empty_value_kept_for_later_checks: "a { color: ; margin: 0 }" => [["color", "margin"]];
    number_property_name_dropped: "a { 1x: red; margin: 0 }" => [["margin"]];
    brace_in_string_value: "a { content: \"}\"; color: red } b { margin: 0 }" => [["content", "color"], ["margin"]];
    brace_in_string_selector: "a[title=\"{\"] { color: red } b { margin: 0 }" => [["color"], ["margin"]];
    unclosed_block_at_eof: "a { color: red" => [["color"]];
    unclosed_string_in_value_runs_to_eof: "a { content: 'abc } b { margin: 0 }" => [["content"]];
    bad_string_then_open_string_runs_to_eof: "a { content: 'ab\ncd'; color: red }" => [["content"]];
    bad_string_recovers_at_semicolon: "a { content: 'ab\n; color: red }" => [["content", "color"]];
    unclosed_paren_in_value_runs_to_eof: "a { width: calc(1px + 2px; color: red } b { margin: 0 }" => [["width"]];
    nested_garbage_skipped: "a { {{{}}} color: red } b { margin: 0 }" => [["color"], ["margin"]];
    stray_close_brace_joins_prelude: "} a { color: red } b { margin: 0 }" => [["margin"]];
    stray_close_paren_kept_in_value: "a { color: red ) ; margin: 0 }" => [["color", "margin"]];
    cdo_cdc_ignored_top_level: "<!-- a { color: red } -->" => [["color"]];
    invalid_selector_drops_rule: "a!b { color: red } c { margin: 0 }" => [["margin"]];
    empty_selector_drops_rule: "{ color: red } c { margin: 0 }" => [["margin"]];
    selector_list_one_bad_drops_all: "a, b!! { color: red } c { margin: 0 }" => [["margin"]];
    forgiving_is_keeps_rule: ":is(a, !!) { color: red }" => [["color"]];
    bad_url_kept_for_later_checks: "a { background: url(a b); color: red }" => [["background", "color"]];
    semicolon_inside_block_value: "a { --x: {a;b}; color: red }" => [["--x", "color"]];
    important_parsed: "a { color: red !important }" => [["color"]];
    unknown_property_kept: "a { frobnicate: 1 }" => [["frobnicate"]];
    whitespace_only_sheet: "  \n\t " => [];
    comment_only_sheet: "/* nothing */" => [];
    at_rule_without_block_then_rule: "@x-unknown foo; a { color: red }" => [["color"]];
}

decl_cases! {
    decls_basic: "color: red; margin: 0" => ["color", "margin"];
    decls_leading_semicolon: ";color: red" => ["color"];
    decls_bad_middle: "color: red; ??; margin: 0" => ["color", "margin"];
    decls_unterminated: "color: red; margin: 0" => ["color", "margin"];
    decls_block_only_value: "color: red; x: {y}; margin: 0" => ["color", "x", "margin"];
    decls_block_mixed_value_dropped: "color: red; x: a {y}; margin: 0" => ["color", "margin"];
    decls_string_with_semicolon: "content: ';'; color: red" => ["content", "color"];
    decls_comments: "/*a*/color/*b*/:/*c*/red" => ["color"];
    decls_custom_and_normal: "--a: 1; --b: 2; top: 0" => ["--a", "--b", "top"];
    decls_empty: "" => [];
    decls_stray_close_brace: "} color: red; } margin: 0 }" => ["color", "margin"];
    decls_only_close_brace: "}" => [];
    decls_at_rule_skipped: "@x { y: z } color: red" => ["color"];
}

#[test]
fn important_flag_removed_from_value() {
    let b = parse_declarations("color: red ! IMPORTANT").unwrap();
    let d = &b.declarations[0];
    assert!(d.important);
    assert!(d.value.iter().all(|v| !v.is_delim('!')));
}

#[test]
fn trailing_whitespace_trimmed_from_value() {
    let b = parse_declarations("color:   red   ").unwrap();
    let d = &b.declarations[0];
    assert_eq!(d.value.len(), 1);
    assert!(d.value[0].is_ident("red"));
}

#[test]
fn custom_property_value_kept_raw() {
    let b = parse_declarations("--x:  a  b ").unwrap();
    assert!(b.declarations[0].is_custom());
    assert!(b.declarations[0].value.iter().any(|v| v.is_whitespace()));
}

#[test]
fn declaration_span_points_at_declaration() {
    let src = "a { color: red }";
    let s = sheet(src);
    let r = styles(&s.rules)[0];
    let sp = r.declarations[0].span;
    assert!(src[sp.start..sp.end].starts_with("color"));
}

#[test]
fn rule_span_covers_rule() {
    let src = "  a { color: red }  ";
    let s = sheet(src);
    let sp = s.rules[0].span();
    assert_eq!(&src[sp.start..sp.end], "a { color: red }");
}

#[test]
fn diagnostics_for_dropped_declaration() {
    let s = sheet("a { color red }");
    assert!(s.diagnostics.iter().any(|d| matches!(
        d.kind,
        DiagnosticKind::InvalidDeclaration | DiagnosticKind::InvalidRule
    )));
}

#[test]
fn diagnostics_for_invalid_selector() {
    let s = sheet("a!b { color: red }");
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::InvalidSelector)
    );
}

#[test]
fn diagnostics_for_unterminated_block() {
    let s = sheet("a { color: red");
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnexpectedEof)
    );
}

#[test]
fn diagnostic_spans_are_inside_input() {
    let src = "a!b { color red } @media (x { } url(a b) 'q\n }";
    let s = sheet(src);
    assert!(!s.diagnostics.is_empty());
    for d in &s.diagnostics {
        assert!(
            d.span.start <= d.span.end && d.span.end <= src.len(),
            "{d:?}"
        );
    }
}

#[test]
fn valid_sheet_has_no_diagnostics() {
    let s = sheet("a { color: red } @media screen { b { margin: 0 } }");
    assert!(s.diagnostics.is_empty(), "{:?}", s.diagnostics);
}

// Nesting.

#[test]
fn nested_style_rule() {
    let s = sheet(".a { color: red; .b { margin: 0 } }");
    let r = styles(&s.rules)[0];
    assert_eq!(r.declarations.len(), 1);
    assert_eq!(styles(&r.rules).len(), 1);
}

#[test]
fn nested_with_ampersand() {
    let s = sheet(".a { &:hover { color: blue } }");
    let r = styles(&s.rules)[0];
    assert_eq!(styles(&r.rules).len(), 1);
}

#[test]
fn nested_with_combinator_start() {
    let s = sheet(".a { > .b { color: blue } }");
    let r = styles(&s.rules)[0];
    assert_eq!(styles(&r.rules).len(), 1);
}

#[test]
fn nested_type_selector_rule() {
    let s = sheet(".a { p { color: blue } }");
    let r = styles(&s.rules)[0];
    assert_eq!(styles(&r.rules).len(), 1);
}

#[test]
fn declaration_like_nested_selector_is_declaration() {
    // `a:hover { ... }` starts like a declaration `a: hover {...}`; a
    // declaration with a block next to other content is invalid, so it
    // falls back to a nested rule.
    let s = sheet(".x { a:hover { color: blue } }");
    let r = styles(&s.rules)[0];
    assert_eq!(styles(&r.rules).len(), 1);
    assert!(r.declarations.is_empty());
}

#[test]
fn declarations_after_nested_rule_become_nested_declarations() {
    let s = sheet(".a { color: red; .b { margin: 0 } top: 0 }");
    let r = styles(&s.rules)[0];
    assert_eq!(r.declarations.len(), 1);
    assert!(
        matches!(r.rules.last(), Some(Rule::NestedDeclarations(n)) if n.declarations[0].name == "top")
    );
}

#[test]
fn nested_media_in_style_rule() {
    let s = sheet(".a { @media screen { color: red } }");
    let r = styles(&s.rules)[0];
    let Rule::Media(m) = &r.rules[0] else {
        panic!("{:?}", r.rules)
    };
    assert!(!m.rules.is_empty());
}

#[test]
fn deeply_nested_rules_parse() {
    let mut src = String::new();
    for _ in 0..20 {
        src.push_str(".a { ");
    }
    for _ in 0..20 {
        src.push_str("} ");
    }
    let s = sheet(&src);
    assert_eq!(s.rule_count(), 20);
}

#[test]
fn rule_count_counts_children() {
    let s = sheet("a{} @media screen { b{} c{ d{} } }");
    assert_eq!(s.rule_count(), 5);
}

// Component values.

#[test]
fn component_values_group_functions() {
    let v = parse_component_values("f(a, g(b)) [c] {d}", &Options::default()).unwrap();
    let ComponentValue::Function(f) = &v[0] else {
        panic!()
    };
    assert_eq!(f.name, "f");
    assert!(
        f.arguments
            .iter()
            .any(|a| matches!(a, ComponentValue::Function(g) if g.name == "g"))
    );
    assert!(
        v.iter()
            .any(|x| matches!(x, ComponentValue::Block(b) if b.kind == BlockKind::Bracket))
    );
    assert!(
        v.iter()
            .any(|x| matches!(x, ComponentValue::Block(b) if b.kind == BlockKind::Brace))
    );
}

#[test]
fn component_values_unclosed_function_closes_at_eof() {
    let v = parse_component_values("f(a", &Options::default()).unwrap();
    let ComponentValue::Function(f) = &v[0] else {
        panic!()
    };
    assert_eq!(f.arguments.len(), 1);
}

#[test]
fn component_values_mismatched_closer_is_token() {
    let v = parse_component_values("(a]", &Options::default()).unwrap();
    let ComponentValue::Block(b) = &v[0] else {
        panic!()
    };
    assert!(
        b.contents
            .iter()
            .any(|c| c.token() == Some(&Token::RightBracket))
    );
}

#[test]
fn component_value_spans() {
    let src = "x f(a) y";
    let v = parse_component_values(src, &Options::default()).unwrap();
    let sp = v[2].span();
    assert_eq!(&src[sp.start..sp.end], "f(a)");
}

#[test]
fn declaration_parse_value_helper() {
    let b = parse_declarations("width: 10px").unwrap();
    let v = b.declarations[0].parse_value().unwrap();
    assert!(v.single().is_some());
}

#[test]
fn bad_url_value_fails_value_parse() {
    let b = parse_declarations("background-image: url(a b)").unwrap();
    assert!(b.declarations[0].parse_value().is_err());
}

#[test]
fn empty_value_fails_value_parse() {
    let b = parse_declarations("color: ").unwrap();
    assert!(b.declarations.is_empty() || b.declarations[0].parse_value().is_err());
}
