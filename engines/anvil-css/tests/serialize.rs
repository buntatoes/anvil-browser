// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Serialization tests: written text reads back as the same tokens.

use anvil_css::Options;
use anvil_css::parser::parse_component_values;
use anvil_css::serialize::{serialize_identifier, serialize_string, to_css};
use anvil_css::tokenizer::{Token, Tokenizer};

fn tokens(src: &str) -> Vec<Token> {
    Tokenizer::new(src)
        .map(|t| t.token)
        .filter(|t| !t.is_whitespace())
        .collect()
}

fn round_trip(src: &str) -> String {
    let v = parse_component_values(src, &Options::default()).unwrap();
    let out = to_css(&v);
    assert_eq!(tokens(&out), tokens(src), "{src:?} was written as {out:?}");
    out
}

fn ident(s: &str) -> String {
    let mut out = String::new();
    serialize_identifier(s, &mut out);
    out
}

macro_rules! round_trips {
    ($($name:ident: $src:expr;)*) => {
        $(
            #[test]
            fn $name() {
                round_trip($src);
            }
        )*
    };
}

round_trips! {
    rt_simple: "a b c";
    rt_numbers: "1 -2 +3 4.5 .5 -0 +0 1e3 1.0";
    rt_dimensions: "10px -1.5em 2E3x 1\\65 3";
    rt_unit_starting_with_e: "1\\65 m 2\\45 -3";
    rt_unit_starting_with_dash: "1\\-x";
    rt_percent: "50% -1.5%";
    rt_strings: "'a' \"b\\\"c\" 'line\\a break' '\\\\'";
    rt_urls: "url(a.png) url( a\\ b ) url(\\(x\\))";
    rt_functions: "f(a, g(b)) calc(1px + 2px)";
    rt_blocks: "[a] (b) {c}";
    rt_hashes: "#abc #1a #-x";
    rt_at_keywords: "@media @-x-y";
    rt_adjacent_ident_number: "a/**/1";
    rt_adjacent_numbers: "1/**/2";
    rt_adjacent_idents: "a/**/b";
    rt_ident_paren: "a/**/(b)";
    rt_number_percent_delim: "1/**/%";
    rt_slash_star: "//**/*";
    rt_dash_number: "-/**/1";
    rt_cdo_cdc: "<!-- -->";
    rt_escaped_ident: "\\31 a \\-- \\@x";
    rt_non_ascii: "caf\u{e9} \u{1F600}";
    rt_delims: "! # $ % & * + , - . / ; < = > ? @ ^ | ~";
    rt_bad_url: "url(a b) x";
    rt_bad_string: "'a\n b";
    rt_huge_number: "1e999 -1e999";
    rt_hash_then_ident: "#a/**/b";
    rt_dimension_then_number: "1px/**/2";
    rt_dash_dash_then_greater: "\\-->";
    rt_dash_dash_ident: "--/**/>";
}

#[test]
fn identifier_escapes_leading_digit() {
    assert_eq!(ident("1a"), "\\31 a");
    assert_eq!(ident("-1a"), "-\\31 a");
}

#[test]
fn identifier_lone_dash() {
    assert_eq!(ident("-"), "\\-");
}

#[test]
fn identifier_plain_kept() {
    assert_eq!(ident("main-color_2"), "main-color_2");
    assert_eq!(ident("--x"), "--x");
}

#[test]
fn identifier_special_chars_escaped() {
    assert_eq!(ident("a.b"), "a\\.b");
    assert_eq!(ident("a b"), "a\\ b");
}

#[test]
fn identifier_control_chars_hex() {
    assert_eq!(ident("a\u{1}"), "a\\1 ");
}

#[test]
fn string_quotes_and_backslashes() {
    let mut out = String::new();
    serialize_string("a\"b\\c\n", &mut out);
    assert_eq!(out, "\"a\\\"b\\\\c\\a \"");
}

#[test]
fn comment_only_where_needed() {
    assert_eq!(round_trip("a b"), "a b");
    assert_eq!(round_trip("a:b"), "a:b");
    assert_eq!(round_trip("a/**/b"), "a/**/b");
}

#[test]
fn unclosed_blocks_are_closed() {
    let v = parse_component_values("f([a", &Options::default()).unwrap();
    assert_eq!(to_css(&v), "f([a])");
}
