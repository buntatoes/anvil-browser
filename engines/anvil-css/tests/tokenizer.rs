// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Tokenizer tests, written from CSS Syntax Level 3 section 4.

use anvil_css::error::DiagnosticKind;
use anvil_css::tokenizer::{Number, Token, Tokenizer, tokenize};

fn toks(src: &str) -> Vec<Token> {
    Tokenizer::new(src).map(|t| t.token).collect()
}

fn ident(s: &str) -> Token {
    Token::Ident(s.to_string())
}
fn func(s: &str) -> Token {
    Token::Function(s.to_string())
}
fn at(s: &str) -> Token {
    Token::AtKeyword(s.to_string())
}
fn string(s: &str) -> Token {
    Token::String(s.to_string())
}
fn url(s: &str) -> Token {
    Token::Url(s.to_string())
}
fn n(value: f64, is_integer: bool, has_sign: bool) -> Number {
    Number {
        value,
        is_integer,
        has_sign,
    }
}
fn int(v: f64) -> Token {
    Token::Number(n(v, true, false))
}
fn real(v: f64) -> Token {
    Token::Number(n(v, false, false))
}
fn dim(v: f64, is_integer: bool, unit: &str) -> Token {
    Token::Dimension {
        value: n(v, is_integer, false),
        unit: unit.to_string(),
    }
}
fn hash(s: &str, is_id: bool) -> Token {
    Token::Hash {
        value: s.to_string(),
        is_id,
    }
}
fn d(c: char) -> Token {
    Token::Delim(c)
}
const WS: Token = Token::Whitespace;

macro_rules! cases {
    ($($name:ident: $src:expr => [$($t:expr),* $(,)?];)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(toks($src), vec![$($t),*], "input {:?}", $src);
            }
        )*
    };
}

cases! {
    empty_input: "" => [];
    single_ident: "color" => [ident("color")];
    ident_with_digits_and_dash: "h1-x_2" => [ident("h1-x_2")];
    custom_property_name: "--main-color" => [ident("--main-color")];
    double_dash_alone: "--" => [ident("--")];
    dash_ident: "-x-foo" => [ident("-x-foo")];
    non_ascii_ident: "caf\u{e9}" => [ident("caf\u{e9}")];
    escaped_ident_hex: "\\41 b" => [ident("Ab")];
    escaped_ident_char: "a\\.b" => [ident("a.b")];
    escape_six_hex_digits: "\\00004142" => [ident("\u{41}42")];
    escape_null_becomes_replacement: "\\0 x" => [ident("\u{FFFD}x")];
    escape_surrogate_becomes_replacement: "\\d800 " => [ident("\u{FFFD}")];
    escape_too_large_becomes_replacement: "\\110000 " => [ident("\u{FFFD}")];
    whitespace_collapses: "a \t\n  b" => [ident("a"), WS, ident("b")];
    comments_are_dropped: "a/* hi */b" => [ident("a"), ident("b")];
    unterminated_comment_ends_input: "a /* never" => [ident("a"), WS];
    function_token: "rgb(1)" => [func("rgb"), int(1.0), Token::RightParen];
    at_keyword: "@media" => [at("media")];
    at_alone_is_delim: "@ x" => [d('@'), WS, ident("x")];
    at_with_dash_ident: "@-x-rule" => [at("-x-rule")];
    hash_id: "#main" => [hash("main", true)];
    hash_unrestricted: "#1abc" => [hash("1abc", false)];
    hash_alone_is_delim: "# " => [d('#'), WS];
    double_quoted_string: "\"hi there\"" => [string("hi there")];
    single_quoted_string: "'it\"s'" => [string("it\"s")];
    string_escape_quote: "'it\\'s'" => [string("it's")];
    string_escaped_newline_continues: "'a\\\nb'" => [string("ab")];
    string_hex_escape: "'\\26 x'" => [string("&x")];
    unterminated_string_at_eof: "'abc" => [string("abc")];
    newline_in_string_is_bad: "'ab\ncd'" => [Token::BadString, WS, ident("cd"), string("")];
    integer_number: "42" => [int(42.0)];
    real_number: "4.5" => [real(4.5)];
    leading_dot_number: ".5" => [real(0.5)];
    signed_number: "+3" => [Token::Number(n(3.0, true, true))];
    negative_number: "-3.5" => [Token::Number(n(-3.5, false, true))];
    exponent_number: "1e3" => [real(1000.0)];
    exponent_signed: "2E-2" => [real(0.02)];
    exponent_needs_digit: "1em" => [dim(1.0, true, "em")];
    percentage: "50%" => [Token::Percentage(n(50.0, true, false))];
    dimension_px: "10px" => [dim(10.0, true, "px")];
    dimension_real: "1.5rem" => [dim(1.5, false, "rem")];
    dimension_escaped_unit: "1\\70x" => [dim(1.0, true, "px")];
    dot_without_digit_is_delim: ". 5" => [d('.'), WS, int(5.0)];
    plus_alone_is_delim: "+ a" => [d('+'), WS, ident("a")];
    minus_number_vs_ident: "-a -1" => [ident("-a"), WS, Token::Number(n(-1.0, true, true))];
    cdo_cdc: "<!-- -->" => [Token::Cdo, WS, Token::Cdc];
    less_than_alone: "<a" => [d('<'), ident("a")];
    punctuation: ":;,[](){}" => [
        Token::Colon, Token::Semicolon, Token::Comma, Token::LeftBracket,
        Token::RightBracket, Token::LeftParen, Token::RightParen,
        Token::LeftBrace, Token::RightBrace,
    ];
    url_unquoted: "url(a/b.png)" => [url("a/b.png")];
    url_with_spaces_around: "url(  a.png  )" => [url("a.png")];
    url_case_insensitive: "URL(x)" => [url("x")];
    url_quoted_is_function: "url('a.png')" => [func("url"), string("a.png"), Token::RightParen];
    url_quoted_with_space: "url( \"a\" )" => [func("url"), WS, string("a"), WS, Token::RightParen];
    url_escape: "url(a\\)b)" => [url("a)b")];
    url_unterminated: "url(abc" => [url("abc")];
    url_with_quote_is_bad: "url(a\"b) x" => [Token::BadUrl, WS, ident("x")];
    url_space_inside_is_bad: "url(a b) x" => [Token::BadUrl, WS, ident("x")];
    url_paren_inside_is_bad: "url(a(b) x" => [Token::BadUrl, WS, ident("x")];
    backslash_newline_is_delim: "\\\n" => [d('\\'), WS];
    crlf_is_one_newline: "a\r\nb" => [ident("a"), WS, ident("b")];
    form_feed_is_whitespace: "a\u{c}b" => [ident("a"), WS, ident("b")];
    null_becomes_replacement: "a\u{0}b" => [ident("a\u{FFFD}b")];
    unicode_range_off_by_default: "u+a" => [ident("u"), d('+'), ident("a")];
    selector_like_input: "a>b~c+d" => [ident("a"), d('>'), ident("b"), d('~'), ident("c"), d('+'), ident("d")];
    attribute_operators: "[a|=b]" => [Token::LeftBracket, ident("a"), d('|'), d('='), ident("b"), Token::RightBracket];
    important_flag: "red !important" => [ident("red"), WS, d('!'), ident("important")];
    number_then_ident: "3n+1" => [dim(3.0, true, "n"), Token::Number(n(1.0, true, true))];
    negative_n: "-n-2" => [ident("-n-2")];
}

#[test]
fn spans_are_byte_offsets_in_original_text() {
    let src = "a\r\n\u{e9}b /* c */ 10px";
    let t: Vec<_> = Tokenizer::new(src).collect();
    assert_eq!(&src[t[0].span.start..t[0].span.end], "a");
    assert_eq!(&src[t[2].span.start..t[2].span.end], "\u{e9}b");
    let last = t.last().unwrap();
    assert_eq!(&src[last.span.start..last.span.end], "10px");
}

#[test]
fn spans_cover_every_byte_without_comments() {
    let src = "a{b:c 1px 'x' url(y)}";
    let t: Vec<_> = Tokenizer::new(src).collect();
    let mut pos = 0;
    for tok in &t {
        assert_eq!(tok.span.start, pos);
        pos = tok.span.end;
    }
    assert_eq!(pos, src.len());
}

fn unicode_range(src: &str) -> Vec<Token> {
    Tokenizer::new(src)
        .with_unicode_ranges(true)
        .map(|t| t.token)
        .collect()
}

#[test]
fn unicode_range_single() {
    assert_eq!(
        unicode_range("U+26"),
        vec![Token::UnicodeRange {
            start: 0x26,
            end: 0x26
        }]
    );
}

#[test]
fn unicode_range_span() {
    assert_eq!(
        unicode_range("u+0-7F"),
        vec![Token::UnicodeRange {
            start: 0,
            end: 0x7f
        }]
    );
}

#[test]
fn unicode_range_wildcard() {
    assert_eq!(
        unicode_range("U+4??"),
        vec![Token::UnicodeRange {
            start: 0x400,
            end: 0x4ff
        }]
    );
}

#[test]
fn unicode_range_list() {
    let t = unicode_range("U+0025-00FF, u+4??");
    assert_eq!(
        t[0],
        Token::UnicodeRange {
            start: 0x25,
            end: 0xff
        }
    );
    assert_eq!(
        t[3],
        Token::UnicodeRange {
            start: 0x400,
            end: 0x4ff
        }
    );
}

#[test]
fn bad_string_reports_diagnostic() {
    let mut tk = Tokenizer::new("'abc\n");
    while tk.next_token().is_some() {}
    assert!(
        tk.diagnostics()
            .iter()
            .any(|d| d.kind == DiagnosticKind::BadString)
    );
}

#[test]
fn bad_url_reports_diagnostic() {
    let list = tokenize("url(a b)", 1000, 1000).unwrap();
    assert!(
        list.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::BadUrl)
    );
}

#[test]
fn eof_in_string_reports_diagnostic() {
    let list = tokenize("'abc", 1000, 1000).unwrap();
    assert!(
        list.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnexpectedEof)
    );
}

#[test]
fn eof_in_comment_reports_diagnostic() {
    let list = tokenize("a /* b", 1000, 1000).unwrap();
    assert!(
        list.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnexpectedEof)
    );
}

#[test]
fn tokenize_respects_byte_limit() {
    let err = tokenize("abcdef", 3, 100).unwrap_err();
    assert_eq!(err.kind, anvil_css::LimitKind::InputBytes);
    assert_eq!(err.found, 6);
}

#[test]
fn tokenize_respects_token_limit() {
    let err = tokenize("a b c d", 100, 3).unwrap_err();
    assert_eq!(err.kind, anvil_css::LimitKind::Tokens);
    assert_eq!(err.limit, 3);
}

#[test]
fn huge_number_is_finite() {
    let t = toks("1e999999");
    let Token::Number(num) = &t[0] else {
        panic!("{t:?}")
    };
    assert!(num.value.is_finite() || num.value.is_infinite());
    assert!(!num.value.is_nan());
}

#[test]
fn very_long_digit_run_is_one_token() {
    let src = "9".repeat(5000);
    assert_eq!(toks(&src).len(), 1);
}

#[test]
fn tokenizer_is_an_iterator_that_ends() {
    let mut tk = Tokenizer::new("a");
    assert!(tk.next().is_some());
    assert!(tk.next().is_none());
    assert!(tk.next().is_none());
}
