// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Selector tests: parsing, specificity, and matching over a small
//! element tree, written from Selectors Level 4.

mod common;

use std::rc::Rc;

use prism::parser::parse_component_values;
use prism::selectors::{
    AnB, ElementState, MatchContext, PseudoElement, RuleIndex, SelectorList, SelectorOptions,
    Specificity, parse_anb, parse_selector_list_with,
};
use prism::{Options, parse_selector_list};
use common::{El, HTML_NS, SVG_NS, Tree};

const LABELS: [&str; 17] = [
    "html", "head", "body", "main", "p1", "p2", "span", "ul", "li1", "li2", "li3", "p3", "footer",
    "a", "input", "svg", "rect",
];

fn doc() -> Rc<Tree> {
    let mut t = Tree::default();
    let html = t.add(None, "html");
    t.add(Some(html), "head");
    let body = t.add(Some(html), "body.page");
    let main = t.add(Some(body), "div#main.container.wide");
    t.attr(main, "data-x", "a-b c");
    t.attr(main, "lang", "en-US");
    t.attr(main, "title", "Hello World");
    let p1 = t.add(Some(main), "p.intro");
    let p2 = t.add(Some(main), "p.note");
    let span = t.add(Some(main), "span.note");
    let ul = t.add(Some(main), "ul");
    let li1 = t.add(Some(ul), "li#i1.item");
    let li2 = t.add(Some(ul), "li#i2.item.active");
    t.state(li2, ElementState::Hover);
    let li3 = t.add(Some(ul), "li#i3.item");
    t.add(Some(main), "p.last");
    let footer = t.add(Some(body), "footer");
    t.nodes[footer].rtl = true;
    t.attr(footer, "lang", "fr");
    let a = t.add(Some(footer), "a.link");
    t.attr(a, "href", "https://x.org/page.pdf");
    t.state(a, ElementState::Link);
    let input = t.add(Some(footer), "input");
    t.attr(input, "type", "Checkbox");
    t.state(input, ElementState::Checked);
    t.state(input, ElementState::Disabled);
    let svg = t.add(Some(body), "svg");
    let rect = t.add(Some(svg), "rect");
    t.nodes[svg].namespace = SVG_NS.to_string();
    t.nodes[rect].namespace = SVG_NS.to_string();
    for i in [p1, p2, span, li1, li2, li3, a] {
        t.nodes[i].has_text = true;
    }
    assert_eq!(t.nodes.len(), LABELS.len());
    assert_eq!(t.nodes[0].namespace, HTML_NS);
    t.finish()
}

fn matching(sel: &str) -> Vec<&'static str> {
    let list = parse_selector_list(sel).unwrap_or_else(|e| panic!("{sel:?}: {e}"));
    let tree = doc();
    (0..tree.nodes.len())
        .filter(|&i| list.matches(&El::new(&tree, i)))
        .map(|i| LABELS[i])
        .collect()
}

macro_rules! match_cases {
    ($($name:ident: $sel:expr => [$($l:expr),*];)*) => {
        $(
            #[test]
            fn $name() {
                let want: Vec<&str> = vec![$($l),*];
                assert_eq!(matching($sel), want, "selector {:?}", $sel);
            }
        )*
    };
}

match_cases! {
    m_type: "p" => ["p1", "p2", "p3"];
    m_type_html_case_insensitive: "P" => ["p1", "p2", "p3"];
    m_type_foreign_case_sensitive: "RECT" => [];
    m_type_foreign: "rect" => ["rect"];
    m_universal_count: "body *" => ["main", "p1", "p2", "span", "ul", "li1", "li2", "li3", "p3", "footer", "a", "input", "svg", "rect"];
    m_class: ".note" => ["p2", "span"];
    m_class_case_sensitive: ".NOTE" => [];
    m_id: "#main" => ["main"];
    m_compound: "p.note" => ["p2"];
    m_two_classes: ".container.wide" => ["main"];
    m_descendant: "div p" => ["p1", "p2", "p3"];
    m_child_none: "body > p" => [];
    m_child: "div > p" => ["p1", "p2", "p3"];
    m_child_chain: ".page > .container > ul > .item.active" => ["li2"];
    m_long_descendant: "html body div ul li#i2" => ["li2"];
    m_next_sibling: "p + p" => ["p2"];
    m_subsequent_sibling: "p ~ p" => ["p2", "p3"];
    m_subsequent_other_type: "p ~ span" => ["span"];
    m_subsequent_universal: ".intro ~ *" => ["p2", "span", "ul", "p3"];
    m_children_of_body: "body > *" => ["main", "footer", "svg"];
    m_id_descendant: "#main #i2" => ["li2"];
    m_mixed_combinators: "div > ul li + li" => ["li2", "li3"];
    m_descendant_backtracking: "div * li" => ["li1", "li2", "li3"];
    m_sibling_then_descendant: "head + body li" => ["li1", "li2", "li3"];
    m_first_child: "li:first-child" => ["li1"];
    m_last_child: "li:last-child" => ["li3"];
    m_only_child: "body :only-child" => ["rect"];
    m_nth_child: "li:nth-child(2)" => ["li2"];
    m_nth_child_odd: "li:nth-child(odd)" => ["li1", "li3"];
    m_nth_child_even: "li:nth-child(even)" => ["li2"];
    m_nth_child_first_two: "li:nth-child(-n+2)" => ["li1", "li2"];
    m_nth_child_past_end: "li:nth-child(n+4)" => [];
    m_nth_last_child: "li:nth-last-child(1)" => ["li3"];
    m_nth_of_type: "p:nth-of-type(2)" => ["p2"];
    m_nth_last_of_type: "p:nth-last-of-type(1)" => ["p3"];
    m_first_of_type: "p:first-of-type" => ["p1"];
    m_last_of_type: "#main > :last-of-type" => ["span", "ul", "p3"];
    m_only_of_type: "span:only-of-type" => ["span"];
    m_only_of_type_none: "p:only-of-type" => [];
    m_nth_child_of: ":nth-child(2 of .note)" => ["span"];
    m_nth_last_child_of: ":nth-last-child(1 of p)" => ["p3"];
    m_first_children: "body :nth-child(1)" => ["main", "p1", "li1", "a", "rect"];
    m_last_children: "body :last-child" => ["li3", "p3", "input", "svg", "rect"];
    m_empty: "p:empty" => ["p3"];
    m_root: ":root" => ["html"];
    m_not: "li:not(.active)" => ["li1", "li3"];
    m_not_list: "li:not(#i1, #i3)" => ["li2"];
    m_not_universal: ":not(*)" => [];
    m_not_complex: "p:not(div > .intro)" => ["p2", "p3"];
    m_is: ":is(p, span).note" => ["p2", "span"];
    m_is_complex: ":is(footer a, ul .active)" => ["li2", "a"];
    m_is_forgiving: ":is(p!!, span)" => ["span"];
    m_where: ":where(#main) > p" => ["p1", "p2", "p3"];
    m_is_nested: "p:is(:is(.intro))" => ["p1"];
    m_has_child: "div:has(> ul)" => ["main"];
    m_has_descendant: "div:has(.active)" => ["main"];
    m_has_next_sibling: "p:has(+ span)" => ["p2"];
    m_has_subsequent_sibling: "p:has(~ ul)" => ["p1", "p2"];
    m_has_none: "body:has(+ p)" => [];
    m_has_complex: ":has(> li + li.active)" => ["ul"];
    m_has_not: "li:not(:has(*))" => ["li1", "li2", "li3"];
    m_hover: "li:hover" => ["li2"];
    m_checked: ":checked" => ["input"];
    m_disabled: ":disabled" => ["input"];
    m_link: ":link" => ["a"];
    m_any_link: ":any-link" => ["a"];
    m_attr_exists: "[title]" => ["main"];
    m_attr_equals: "[title='Hello World']" => ["main"];
    m_attr_case_sensitive_value: "[title='hello world']" => [];
    m_attr_i_flag: "[title='hello world' i]" => ["main"];
    m_attr_s_flag_on_html_insensitive: "[type=checkbox s]" => [];
    m_attr_html_case_insensitive_value: "[type=checkbox]" => ["input"];
    m_attr_includes: "[data-x~=c]" => ["main"];
    m_attr_includes_empty: "[data-x~='']" => [];
    m_attr_dash: "[data-x|=a]" => ["main"];
    m_attr_prefix: "[data-x^=a-]" => ["main"];
    m_attr_prefix_empty: "[data-x^='']" => [];
    m_attr_suffix: "[href$='.pdf']" => ["a"];
    m_attr_substring: "[data-x*='b c']" => ["main"];
    m_attr_lang_dash: "[lang|=en]" => ["main"];
    m_attr_name_case_insensitive_in_html: "[TITLE]" => ["main"];
    m_lang: ":lang(en)" => ["main", "p1", "p2", "span", "ul", "li1", "li2", "li3", "p3"];
    m_lang_other: ":lang(fr)" => ["footer", "a", "input"];
    m_lang_list: ":lang(de, fr)" => ["footer", "a", "input"];
    m_lang_wildcard: ":lang('*-US')" => ["main", "p1", "p2", "span", "ul", "li1", "li2", "li3", "p3"];
    m_dir: ":dir(rtl)" => ["footer"];
    m_any_namespace: "*|rect" => ["rect"];
    m_scope_without_scope_is_root: ":scope" => ["html"];
    m_selector_list: "#i1, #i3, .link" => ["li1", "li3", "a"];
    m_pseudo_element_never_plain: "p::before" => [];
    m_ampersand_top_level: "& > head" => ["head"];
}

fn spec(sel: &str) -> (u32, u32, u32) {
    let l = parse_selector_list(sel).unwrap_or_else(|e| panic!("{sel:?}: {e}"));
    let s = l.max_specificity();
    (s.a, s.b, s.c)
}

macro_rules! specificity_cases {
    ($($name:ident: $sel:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(spec($sel), $want, "selector {:?}", $sel);
            }
        )*
    };
}

specificity_cases! {
    s_universal: "*" => (0, 0, 0);
    s_type: "li" => (0, 0, 1);
    s_two_types: "ul li" => (0, 0, 2);
    s_three_types: "ul ol+li" => (0, 0, 3);
    s_attr_and_type: "h1 + *[rel=up]" => (0, 1, 1);
    s_class_and_types: "ul ol li.red" => (0, 1, 3);
    s_two_classes: "li.red.level" => (0, 2, 1);
    s_id: "#x34y" => (1, 0, 0);
    s_not_takes_argument: "#s12:not(foo)" => (1, 0, 1);
    s_is_takes_max: ".foo :is(.bar, #baz)" => (1, 1, 0);
    s_where_is_zero: ":where(#a) p" => (0, 0, 1);
    s_pseudo_element: "p::before" => (0, 0, 2);
    s_legacy_pseudo_element: "p:before" => (0, 0, 2);
    s_nth_child_of: ":nth-child(2 of #a, .b)" => (1, 1, 0);
    s_has: ":has(> #a)" => (1, 0, 0);
    s_selection: "::selection" => (0, 0, 1);
    s_hover: "a:hover" => (0, 1, 1);
    s_attribute: "[x]" => (0, 1, 0);
    s_root: ":root" => (0, 1, 0);
    s_any_namespace_universal: "*|*" => (0, 0, 0);
    s_list_max: "a, #b, .c" => (1, 0, 0);
}

#[test]
fn specificity_ordering() {
    assert!(Specificity::new(1, 0, 0) > Specificity::new(0, 99, 99));
    assert!(Specificity::new(0, 1, 0) > Specificity::new(0, 0, 99));
    assert!(Specificity::new(0, 0, 2) > Specificity::new(0, 0, 1));
}

macro_rules! parse_ok {
    ($($name:ident: $sel:expr;)*) => {
        $(
            #[test]
            fn $name() {
                if let Err(e) = parse_selector_list($sel) {
                    panic!("{:?} should parse: {e}", $sel);
                }
            }
        )*
    };
}

macro_rules! parse_err {
    ($($name:ident: $sel:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert!(parse_selector_list($sel).is_err(), "{:?} should fail", $sel);
            }
        )*
    };
}

parse_ok! {
    ok_hover_before: "a:hover::before";
    ok_state_after_pseudo_element: "a::before:hover";
    ok_legacy_after: "a:after";
    ok_any_namespace_type: "*|a";
    ok_no_namespace_type: "|a";
    ok_any_namespace_attr: "[*|x]";
    ok_case_flag_s: "a[x='y' s]";
    ok_case_flag_upper: "a[x=y I]";
    ok_anb_spaces: ":nth-child( +3n - 2 )";
    ok_lang_strings: ":lang('en', fr)";
    ok_whitespace_around_combinators: "a  >  b   ~  c";
    ok_comments_in_selector: "a/**/ b /**/> c";
    ok_is_with_only_bad: ":is(!!)";
    ok_where_empty_after_drop: ":where(::before)";
    ok_escaped_class: ".a\\:b";
    ok_numeric_escaped_id: "#\\31 a";
    ok_many_selectors: "a, b, c, d, e, f";
    ok_marker: "li::marker";
    ok_placeholder: "input::placeholder";
    ok_file_button: "input::file-selector-button";
    ok_backdrop: "dialog::backdrop";
    ok_first_line: "p::first-line";
    ok_first_letter: "p::first-letter";
    ok_focus_states: "a:focus, a:focus-visible, a:focus-within, a:active, a:visited";
    ok_form_states: ":required:optional:read-only:read-write:placeholder-shown:default:valid:invalid:in-range:out-of-range";
    ok_other_states: ":target:indeterminate:enabled:defined:open:modal:fullscreen:autofill";
}

parse_err! {
    err_empty: "";
    err_only_space: "   ";
    err_bang: "a!";
    err_trailing_comma: "a,";
    err_leading_comma: ",a";
    err_double_comma: "a,,b";
    err_trailing_combinator: "a >";
    err_leading_combinator_top_level: "> a";
    err_double_combinator: "a > > b";
    err_after_pseudo_element: "a::before b";
    err_two_pseudo_elements: "a::before::after";
    err_structural_after_pseudo_element: "a::before:first-child";
    err_unknown_pseudo_element: "::nope";
    err_unknown_pseudo_class: ":nope";
    err_unknown_function: ":nope(a)";
    err_unclosed_attribute: "a[";
    err_attribute_no_value: "a[x=]";
    err_attribute_two_values: "a[x=y z]";
    err_attribute_bad_flag: "a[x=y q]";
    err_attribute_number_value: "a[x=1]";
    err_hash_not_identifier: "#1a";
    err_class_number: ".1a";
    err_nth_garbage: ":nth-child(foo)";
    err_nth_space_after_sign: ":nth-child(+ 3n)";
    err_nth_space_before_n: ":nth-child(3 n)";
    err_not_empty: ":not()";
    err_not_bad: ":not(!!)";
    err_nested_has: ":has(:has(a))";
    err_has_pseudo_element: ":has(::before)";
    err_not_pseudo_element: ":not(::before)";
    err_column_combinator: "a || b";
    err_dir_bad: ":dir(up)";
    err_type_after_class: ".a/**/p";
    err_pseudo_element_function_args: "::before(x)";
    err_universal_after_type: "a*";
    err_bad_delim: "a % b";
}

#[test]
fn comment_is_not_a_combinator() {
    // A comment is not whitespace, so ".a/**/p" puts a type selector
    // after a class, which is invalid; ".a p" is a descendant selector.
    assert!(parse_selector_list(".a p").is_ok());
    assert!(parse_selector_list("a/**/b").is_err());
}

#[test]
fn leading_combinator_allowed_when_nested() {
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    assert!(parse_selector_list_with("> a", &opts).is_ok());
    assert!(parse_selector_list_with("+ a, ~ b", &opts).is_ok());
}

#[test]
fn selector_error_has_span_inside_input() {
    let src = "a, b!, c";
    let e = parse_selector_list(src).unwrap_err();
    assert!(e.span.end <= src.len());
    assert!(!e.message.is_empty());
}

#[test]
fn pseudo_element_is_recorded() {
    let l = parse_selector_list("p::before").unwrap();
    assert_eq!(
        l.selectors()[0].pseudo_element(),
        Some(PseudoElement::Before)
    );
    let l = parse_selector_list("p:after").unwrap();
    assert_eq!(
        l.selectors()[0].pseudo_element(),
        Some(PseudoElement::After)
    );
}

#[test]
fn pseudo_element_matches_with_context() {
    let tree = doc();
    let l = parse_selector_list("p.intro::before").unwrap();
    let ctx = MatchContext {
        pseudo_element: Some(PseudoElement::Before),
        ..MatchContext::default()
    };
    assert!(l.selectors()[0].matches_in(&El::new(&tree, 4), &ctx));
    assert!(!l.selectors()[0].matches_in(&El::new(&tree, 5), &ctx));
    let after = MatchContext {
        pseudo_element: Some(PseudoElement::After),
        ..MatchContext::default()
    };
    assert!(!l.selectors()[0].matches_in(&El::new(&tree, 4), &after));
}

#[test]
fn scope_matching() {
    let tree = doc();
    let l = parse_selector_list(":scope > p").unwrap();
    let ctx = MatchContext {
        scope: Some(El::new(&tree, 3)),
        ..MatchContext::default()
    };
    let got: Vec<_> = (0..17)
        .filter(|&i| l.selectors()[0].matches_in(&El::new(&tree, i), &ctx))
        .map(|i| LABELS[i])
        .collect();
    assert_eq!(got, ["p1", "p2", "p3"]);
}

#[test]
fn relative_selector_needs_scope() {
    let tree = doc();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let l = parse_selector_list_with("> ul", &opts).unwrap();
    assert!(!l.selectors()[0].matches(&El::new(&tree, 7)));
    let ctx = MatchContext {
        scope: Some(El::new(&tree, 3)),
        ..MatchContext::default()
    };
    assert!(l.selectors()[0].matches_in(&El::new(&tree, 7), &ctx));
}

#[test]
fn namespace_prefix_matching() {
    let tree = doc();
    let l = parse_selector_list("svg|rect").unwrap();
    let rect = El::new(&tree, 16);
    assert!(
        !l.selectors()[0].matches(&rect),
        "undeclared prefix matches nothing"
    );
    let ctx = MatchContext {
        namespaces: vec![("svg".into(), SVG_NS.into())],
        ..MatchContext::default()
    };
    assert!(l.selectors()[0].matches_in(&rect, &ctx));
}

#[test]
fn no_namespace_matching() {
    let tree = doc();
    let l = parse_selector_list("|rect").unwrap();
    assert!(!l.matches(&El::new(&tree, 16)));
}

// Nesting.

#[test]
fn resolve_nesting_ampersand() {
    let tree = doc();
    let parent = parse_selector_list("#main").unwrap();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let child = parse_selector_list_with("& > p", &opts).unwrap();
    let r = child.resolve_nesting(Some(&parent), 1000).unwrap();
    assert!(r.matches(&El::new(&tree, 4)));
    assert!(!r.matches(&El::new(&tree, 8)));
    assert_eq!(r.max_specificity(), Specificity::new(1, 0, 1));
}

#[test]
fn resolve_nesting_implicit_descendant() {
    let tree = doc();
    let parent = parse_selector_list("ul").unwrap();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let child = parse_selector_list_with(".active", &opts).unwrap();
    let r = child.resolve_nesting(Some(&parent), 1000).unwrap();
    assert!(r.matches(&El::new(&tree, 9)));
    assert!(!r.matches(&El::new(&tree, 5)));
}

#[test]
fn resolve_nesting_leading_combinator() {
    let tree = doc();
    let parent = parse_selector_list("p.intro").unwrap();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let child = parse_selector_list_with("+ p", &opts).unwrap();
    let r = child.resolve_nesting(Some(&parent), 1000).unwrap();
    assert!(r.matches(&El::new(&tree, 5)));
    assert!(!r.matches(&El::new(&tree, 11)));
}

#[test]
fn resolve_nesting_ampersand_suffix() {
    let tree = doc();
    let parent = parse_selector_list("li").unwrap();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let child = parse_selector_list_with("&.active", &opts).unwrap();
    let r = child.resolve_nesting(Some(&parent), 1000).unwrap();
    assert!(r.matches(&El::new(&tree, 9)));
    assert!(!r.matches(&El::new(&tree, 8)));
}

#[test]
fn resolve_nesting_size_limit() {
    let parent = parse_selector_list("a b c d e f g h").unwrap();
    let opts = SelectorOptions {
        nested: true,
        ..SelectorOptions::default()
    };
    let child = parse_selector_list_with("& & & & & & & & & &", &opts).unwrap();
    let e = child.resolve_nesting(Some(&parent), 50).unwrap_err();
    assert_eq!(e.limit, Some(prism::LimitKind::SelectorLength));
}

#[test]
fn resolve_nesting_without_parent_is_identity() {
    let l = parse_selector_list("a").unwrap();
    assert_eq!(l.resolve_nesting(None, 10).unwrap(), l);
}

// An+B.

fn anb(src: &str) -> Option<(i32, i32)> {
    let v = parse_component_values(src, &Options::default()).unwrap();
    parse_anb(&v).map(|x| (x.a, x.b))
}

macro_rules! anb_cases {
    ($($name:ident: $src:expr => $want:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(anb($src), $want, "input {:?}", $src);
            }
        )*
    };
}

anb_cases! {
    anb_odd: "odd" => Some((2, 1));
    anb_even: "EVEN" => Some((2, 0));
    anb_integer: "5" => Some((0, 5));
    anb_negative_integer: "-5" => Some((0, -5));
    anb_n: "n" => Some((1, 0));
    anb_minus_n: "-n" => Some((-1, 0));
    anb_plus_n: "+n" => Some((1, 0));
    anb_upper_n: "N" => Some((1, 0));
    anb_2n_plus_1: "2n+1" => Some((2, 1));
    anb_2n_minus_1: "2n-1" => Some((2, -1));
    anb_spaced_minus: "2n- 1" => Some((2, -1));
    anb_spaced_plus: "2n + 1" => Some((2, 1));
    anb_minus_n_minus_2: "-n-2" => Some((-1, -2));
    anb_n_spaced: "n - 2" => Some((1, -2));
    anb_plus_n_plus: "+n+3" => Some((1, 3));
    anb_double_sign: "3n+-1" => None;
    anb_dangling_sign: "2n +" => None;
    anb_real: "1.5n" => None;
    anb_space_after_plus: "+ n" => None;
    anb_word: "foo" => None;
    anb_empty: "" => None;
    anb_signed_b_needs_sign: "2n 1" => None;
}

#[test]
fn anb_matching() {
    let m = |a, b, i| AnB { a, b }.matches(i);
    assert!(m(2, 1, 1) && m(2, 1, 3) && !m(2, 1, 2));
    assert!(m(0, 3, 3) && !m(0, 3, 4));
    assert!(m(-1, 3, 1) && m(-1, 3, 3) && !m(-1, 3, 4));
    assert!(m(1, 0, 1) && m(1, 0, 100));
    assert!(!m(3, -10, 1) && m(3, -10, 2) && m(3, -10, 5));
    assert!(!m(0, 0, 1));
}

// Rule index.

#[test]
fn rule_index_buckets_and_matches() {
    let tree = doc();
    let mut idx = RuleIndex::new();
    let sels = [
        "#i2",
        ".note",
        "li",
        "*",
        ":hover",
        "ul > .item",
        "#nope",
        "p.intro, span",
    ];
    for (n, s) in sels.iter().enumerate() {
        for sel in parse_selector_list(s).unwrap().0 {
            idx.insert(sel, n);
        }
    }
    assert_eq!(idx.len(), 9);
    assert!(idx.catch_all_len() >= 2);
    let ctx = MatchContext::default();
    for (i, label) in LABELS.iter().enumerate() {
        let el = El::new(&tree, i);
        let mut via_index: Vec<usize> = idx
            .matching(&el, &ctx)
            .into_iter()
            .map(|(_, d)| *d)
            .collect();
        via_index.sort();
        let mut brute: Vec<usize> = (0..idx.len())
            .filter_map(|k| idx.get(k))
            .filter(|(s, _)| s.matches_in(&el, &ctx))
            .map(|(_, d)| *d)
            .collect();
        brute.sort();
        assert_eq!(via_index, brute, "element {label}");
    }
}

#[test]
fn rule_index_candidates_skip_unrelated() {
    let tree = doc();
    let mut idx = RuleIndex::new();
    for s in ["#a", "#b", ".x", ".y", "table"] {
        idx.insert(parse_selector_list(s).unwrap().0.remove(0), ());
    }
    assert!(idx.candidates(&El::new(&tree, 4)).is_empty());
    assert!(!idx.is_empty());
}

// Matching against a simple reference matcher on random trees.

#[derive(Clone)]
struct Simple {
    tag: Option<&'static str>,
    class: Option<&'static str>,
}

const TAGS: [&str; 4] = ["div", "p", "span", "li"];
const CLASSES: [&str; 3] = ["a", "b", "c"];
const COMBS: [&str; 4] = [" ", " > ", " + ", " ~ "];

fn simple_ok(t: &Tree, i: usize, s: &Simple) -> bool {
    let n = &t.nodes[i];
    s.tag.is_none_or(|tag| n.name == tag)
        && s.class.is_none_or(|c| n.classes.iter().any(|k| k == c))
}

fn prev_sib(t: &Tree, i: usize) -> Option<usize> {
    let p = t.nodes[i].parent?;
    let kids = &t.nodes[p].children;
    let pos = kids.iter().position(|&k| k == i)?;
    pos.checked_sub(1).map(|q| kids[q])
}

/// Plain recursive matcher, right to left, trying every candidate.
fn reference(t: &Tree, parts: &[Simple], combs: &[usize], i: usize) -> bool {
    let last = parts.len() - 1;
    if !simple_ok(t, i, &parts[last]) {
        return false;
    }
    if last == 0 {
        return true;
    }
    let rest = &parts[..last];
    let rest_c = &combs[..last - 1];
    match combs[last - 1] {
        0 => {
            let mut cur = t.nodes[i].parent;
            while let Some(p) = cur {
                if reference(t, rest, rest_c, p) {
                    return true;
                }
                cur = t.nodes[p].parent;
            }
            false
        }
        1 => t.nodes[i]
            .parent
            .is_some_and(|p| reference(t, rest, rest_c, p)),
        2 => prev_sib(t, i).is_some_and(|s| reference(t, rest, rest_c, s)),
        _ => {
            let mut cur = prev_sib(t, i);
            while let Some(s) = cur {
                if reference(t, rest, rest_c, s) {
                    return true;
                }
                cur = prev_sib(t, s);
            }
            false
        }
    }
}

#[test]
fn matcher_agrees_with_reference_on_random_trees() {
    let mut rng = common::Rng(0x9e37_79b9_7f4a_7c15);
    for _round in 0..40 {
        let mut t = Tree::default();
        let root = t.add(None, "div");
        let mut all = vec![root];
        for _ in 0..40 {
            let parent = all[rng.below(all.len())];
            let mut spec = TAGS[rng.below(4)].to_string();
            for c in CLASSES {
                if rng.below(3) == 0 {
                    spec.push('.');
                    spec.push_str(c);
                }
            }
            all.push(t.add(Some(parent), &spec));
        }
        let tree = t.finish();
        for _ in 0..30 {
            let len = 1 + rng.below(4);
            let mut parts = Vec::new();
            let mut combs = Vec::new();
            let mut text = String::new();
            for k in 0..len {
                if k > 0 {
                    let c = rng.below(4);
                    combs.push(c);
                    text.push_str(COMBS[c]);
                }
                let tag = (rng.below(2) == 0).then(|| TAGS[rng.below(4)]);
                let class = (tag.is_none() || rng.below(2) == 0).then(|| CLASSES[rng.below(3)]);
                if let Some(tg) = tag {
                    text.push_str(tg);
                }
                if let Some(c) = class {
                    text.push('.');
                    text.push_str(c);
                }
                parts.push(Simple { tag, class });
            }
            let list: SelectorList = parse_selector_list(&text).unwrap();
            for i in 0..tree.nodes.len() {
                let want = reference(&tree, &parts, &combs, i);
                let got = list.matches(&El::new(&tree, i));
                assert_eq!(got, want, "selector {text:?} on node {i}");
            }
        }
    }
}

#[test]
fn deep_tree_matching_is_fast_enough() {
    // A long chain with a selector that fails at the top; the matcher
    // must stop early instead of retrying every ancestor pair.
    let mut t = Tree::default();
    let mut cur = t.add(None, "div");
    for _ in 0..3000 {
        cur = t.add(Some(cur), "div");
    }
    let tree = t.finish();
    let l = parse_selector_list("p div div div div div div div div div div div").unwrap();
    let start = std::time::Instant::now();
    assert!(!l.matches(&El::new(&tree, cur)));
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn vendor_prefixed_pseudo_and_forgiving_list_preserve_valid_selectors() {
    let tree = doc();
    let list = parse_selector_list("input, ::-vendor-file-upload-button, button:-vendor-autofill")
        .unwrap();
    assert!(list.matches(&El::new(&tree, 14)));
    let forgiving =
        prism::parse_forgiving_selector_list("p.intro, ::unknown-pseudo, #i2").unwrap();
    assert!(forgiving.matches(&El::new(&tree, 4)));
    assert!(forgiving.matches(&El::new(&tree, 9)));
    assert_eq!(
        prism::parse_css_rgba8("rgb(26 115 232 / 50%)"),
        Some([26, 115, 232, 128])
    );
    assert_eq!(
        prism::parse_css_rgba8("rebeccapurple"),
        Some([102, 51, 153, 255])
    );
}
