// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Typed at-rule tests: @media, @supports, @import, @font-face,
//! @keyframes, @layer, @container, @namespace, @page, @property, @scope,
//! @starting-style, and unknown rules.

use prism::media::{MediaEnv, MediaType};
use prism::parser::ImportLayer;
use prism::{DiagnosticKind, Options, Rule, Stylesheet, parse_stylesheet};

fn sheet(src: &str) -> Stylesheet {
    parse_stylesheet(src, &Options::default()).expect("within limits")
}

fn first(src: &str) -> Rule {
    let s = sheet(src);
    assert!(
        !s.rules.is_empty(),
        "no rules for {src:?}: {:?}",
        s.diagnostics
    );
    s.rules.into_iter().next().unwrap()
}

fn kinds(src: &str) -> Vec<&'static str> {
    sheet(src)
        .rules
        .iter()
        .map(|r| match r {
            Rule::Style(_) => "style",
            Rule::Media(_) => "media",
            Rule::Supports(_) => "supports",
            Rule::Import(_) => "import",
            Rule::FontFace(_) => "font-face",
            Rule::Keyframes(_) => "keyframes",
            Rule::LayerStatement(_) => "layer-statement",
            Rule::LayerBlock(_) => "layer-block",
            Rule::Container(_) => "container",
            Rule::Namespace(_) => "namespace",
            Rule::Page(_) => "page",
            Rule::Property(_) => "property",
            Rule::Scope(_) => "scope",
            Rule::StartingStyle(_) => "starting-style",
            Rule::NestedDeclarations(_) => "nested-declarations",
            Rule::Unknown(_) => "unknown",
        })
        .collect()
}

macro_rules! kind_cases {
    ($($name:ident: $src:expr => [$($k:expr),*];)*) => {
        $(
            #[test]
            fn $name() {
                let want: Vec<&str> = vec![$($k),*];
                assert_eq!(kinds($src), want, "input {:?}", $src);
            }
        )*
    };
}

kind_cases! {
    media_block: "@media screen { a {} }" => ["media"];
    media_names_case_insensitive: "@MEDIA screen { a {} }" => ["media"];
    supports_block: "@supports (display: grid) { a {} }" => ["supports"];
    import_first: "@import 'a.css'; a {}" => ["import", "style"];
    import_after_rule_dropped: "a {} @import 'a.css';" => ["style"];
    import_after_charset: "@charset \"utf-8\"; @import 'a.css';" => ["unknown", "import"];
    import_after_layer_statement: "@layer a; @import 'a.css';" => ["layer-statement", "import"];
    import_after_layer_block_dropped: "@layer a {} @import 'a.css';" => ["layer-block"];
    namespace_after_import: "@import 'a'; @namespace svg url(x); a {}" => ["import", "namespace", "style"];
    import_after_namespace_dropped: "@namespace url(x); @import 'a';" => ["namespace"];
    namespace_after_rule_dropped: "a {} @namespace url(x);" => ["style"];
    font_face: "@font-face { font-family: x; src: url(a.woff2) }" => ["font-face"];
    keyframes: "@keyframes spin { from {} to {} }" => ["keyframes"];
    layer_statement: "@layer a, b.c;" => ["layer-statement"];
    layer_block_named: "@layer a { b {} }" => ["layer-block"];
    layer_block_anonymous: "@layer { b {} }" => ["layer-block"];
    container_rule: "@container (min-width: 100px) { a {} }" => ["container"];
    page_rule: "@page { margin: 1in }" => ["page"];
    property_rule: "@property --brand { syntax: '<color>'; inherits: true; initial-value: red }" => ["property"];
    scope_rule: "@scope (.card) to (.footer) { p { color: red } }" => ["scope"];
    starting_style_rule: "@starting-style { .modal { opacity: 0 } }" => ["starting-style"];
    unknown_with_block: "@x-thing foo { bar }" => ["unknown"];
    unknown_statement: "@x-thing foo;" => ["unknown"];
    media_without_block_dropped: "@media screen; a {}" => ["style"];
    font_face_with_prelude_dropped: "@font-face x { src: url(a) } a {}" => ["style"];
    keyframes_without_name_dropped: "@keyframes { from {} } a {}" => ["style"];
    keyframes_name_none_dropped: "@keyframes none { from {} } a {}" => ["style"];
    keyframes_string_name: "@keyframes 'spin' { from {} }" => ["keyframes"];
    layer_statement_needs_name: "@layer; a {}" => ["style"];
    layer_bad_name_dropped: "@layer 1a { } a {}" => ["style"];
    nested_groups: "@media screen { @supports (color: red) { @layer x { a {} } } }" => ["media"];
    unclosed_media_block: "@media screen { a { color: red }" => ["media"];
    at_rule_in_selector_position_in_media: "@media all { @x; a {} }" => ["media"];
}

#[test]
fn property_rule_descriptors() {
    let Rule::Property(p) =
        first("@property --accent { syntax: '<color>'; inherits: false; initial-value: #ff0000 }")
    else {
        panic!()
    };
    assert_eq!(p.name, "--accent");
    assert_eq!(p.syntax.as_deref(), Some("<color>"));
    assert_eq!(p.inherits, Some(false));
    assert!(p.initial_value.is_some());
}

#[test]
fn scope_rule_selectors_and_children() {
    let Rule::Scope(s) = first("@scope (.card) to (.card-content) { a { color: blue } }") else {
        panic!()
    };
    assert!(s.start.is_some());
    assert!(s.end.is_some());
    assert_eq!(s.rules.len(), 1);
}

#[test]
fn starting_style_nested_and_top_level() {
    let Rule::StartingStyle(st) = first("@starting-style { .box { opacity: 0 } }") else {
        panic!()
    };
    assert_eq!(st.rules.len(), 1);
}

#[test]
fn media_queries_typed_and_evaluated() {
    let Rule::Media(m) = first("@media print, (min-width: 600px) { a {} }") else {
        panic!()
    };
    assert_eq!(m.queries.0.len(), 2);
    assert_eq!(m.queries.0[0].media_type, Some(MediaType::Print));
    assert!(m.queries.evaluate(&MediaEnv::default()));
    assert!(!m.prelude.is_empty());
}

#[test]
fn media_children_parsed() {
    let Rule::Media(m) = first("@media screen { a {} b {} }") else {
        panic!()
    };
    assert_eq!(m.rules.len(), 2);
}

#[test]
fn supports_condition_evaluated() {
    let Rule::Supports(s) =
        first("@supports (display: grid) and (not (color: nonsense-value)) { a {} }")
    else {
        panic!()
    };
    assert!(s.condition.evaluate());
}

#[test]
fn supports_invalid_condition_dropped() {
    let s = sheet("@supports display: grid { a {} } b {}");
    assert_eq!(s.rules.len(), 1);
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::InvalidAtRule)
    );
}

#[test]
fn import_string_url() {
    let Rule::Import(i) = first("@import \"theme.css\";") else {
        panic!()
    };
    assert_eq!(i.url, "theme.css");
    assert!(i.layer.is_none());
    assert!(i.supports.is_none());
    assert!(i.media.0.is_empty());
}

#[test]
fn import_url_function() {
    let Rule::Import(i) = first("@import url(theme.css) screen;") else {
        panic!()
    };
    assert_eq!(i.url, "theme.css");
    assert_eq!(i.media.0.len(), 1);
}

#[test]
fn import_quoted_url_function() {
    let Rule::Import(i) = first("@import url('a b.css');") else {
        panic!()
    };
    assert_eq!(i.url, "a b.css");
}

#[test]
fn import_layer_anonymous() {
    let Rule::Import(i) = first("@import 'a.css' layer;") else {
        panic!()
    };
    assert_eq!(i.layer, Some(ImportLayer::Anonymous));
}

#[test]
fn import_layer_named() {
    let Rule::Import(i) = first("@import 'a.css' layer(base.reset);") else {
        panic!()
    };
    assert_eq!(
        i.layer,
        Some(ImportLayer::Named(vec!["base".into(), "reset".into()]))
    );
}

#[test]
fn import_supports_and_media() {
    let Rule::Import(i) =
        first("@import 'a.css' layer(x) supports(display: grid) print and (color);")
    else {
        panic!()
    };
    assert!(i.supports.is_some());
    assert_eq!(i.media.0.len(), 1);
    assert!(!i.media.evaluate(&MediaEnv::default()));
}

#[test]
fn import_without_url_dropped() {
    let s = sheet("@import screen; a {}");
    assert_eq!(s.rules.len(), 1);
    assert!(!s.diagnostics.is_empty());
}

#[test]
fn import_with_block_dropped() {
    let s = sheet("@import 'a' { } a {}");
    assert!(s.rules.iter().all(|r| !matches!(r, Rule::Import(_))));
}

#[test]
fn import_in_group_dropped() {
    let Rule::Media(m) = first("@media screen { @import 'a'; a {} }") else {
        panic!()
    };
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn font_face_descriptors() {
    let Rule::FontFace(f) = first(
        "@font-face { font-family: 'X'; src: url(a.woff2) format('woff2'); unicode-range: U+0-7F }",
    ) else {
        panic!()
    };
    let names: Vec<_> = f.declarations.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["font-family", "src", "unicode-range"]);
}

#[test]
fn font_face_unicode_range_tokens() {
    let Rule::FontFace(f) = first("@font-face { unicode-range: U+4?? }") else {
        panic!()
    };
    let v = f.declarations[0].parse_value().unwrap();
    let prism::PropertyValue::Values(vals) = v else {
        panic!("{v:?}")
    };
    assert_eq!(vals[0], prism::Value::UnicodeRange(0x400, 0x4ff));
}

#[test]
fn keyframe_selectors() {
    let Rule::Keyframes(k) =
        first("@keyframes spin { from { opacity: 0 } 50%, 75% { opacity: .5 } to { opacity: 1 } }")
    else {
        panic!()
    };
    assert_eq!(k.name, "spin");
    assert_eq!(k.keyframes.len(), 3);
    assert_eq!(k.keyframes[0].selectors, vec![0.0]);
    assert_eq!(k.keyframes[1].selectors, vec![50.0, 75.0]);
    assert_eq!(k.keyframes[2].selectors, vec![100.0]);
}

#[test]
fn keyframe_bad_selector_dropped() {
    let Rule::Keyframes(k) = first("@keyframes x { 150% { a: b } foo { a: b } 10% { a: b } }")
    else {
        panic!()
    };
    assert_eq!(k.keyframes.len(), 1);
    assert_eq!(k.keyframes[0].selectors, vec![10.0]);
}

#[test]
fn keyframe_important_dropped() {
    let Rule::Keyframes(k) = first("@keyframes x { to { opacity: 1 !important; color: red } }")
    else {
        panic!()
    };
    let names: Vec<_> = k.keyframes[0]
        .declarations
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, ["color"]);
}

#[test]
fn layer_names() {
    let Rule::LayerStatement(l) = first("@layer reset, base.components , theme;") else {
        panic!()
    };
    assert_eq!(
        l.names,
        vec![
            vec!["reset".to_string()],
            vec!["base".into(), "components".into()],
            vec!["theme".into()]
        ]
    );
}

#[test]
fn layer_block_contents() {
    let Rule::LayerBlock(l) = first("@layer base { a {} @media print { b {} } }") else {
        panic!()
    };
    assert_eq!(l.name, Some(vec!["base".to_string()]));
    assert_eq!(l.rules.len(), 2);
}

#[test]
fn container_named() {
    let Rule::Container(c) = first("@container sidebar (min-width: 400px) { a {} }") else {
        panic!()
    };
    assert_eq!(c.name.as_deref(), Some("sidebar"));
    assert!(c.condition.is_some());
}

#[test]
fn container_name_only() {
    let Rule::Container(c) = first("@container sidebar { a {} }") else {
        panic!()
    };
    assert_eq!(c.name.as_deref(), Some("sidebar"));
    assert!(c.condition.is_none());
}

#[test]
fn container_reserved_name_dropped() {
    let s = sheet("@container none (width > 1px) { a {} } b {}");
    assert_eq!(s.rules.len(), 1);
}

#[test]
fn namespace_default_and_prefixed() {
    let s = sheet(
        "@namespace url(http://www.w3.org/1999/xhtml); @namespace svg 'http://www.w3.org/2000/svg';",
    );
    let Rule::Namespace(a) = &s.rules[0] else {
        panic!()
    };
    let Rule::Namespace(b) = &s.rules[1] else {
        panic!()
    };
    assert_eq!(a.prefix, None);
    assert_eq!(a.url, "http://www.w3.org/1999/xhtml");
    assert_eq!(b.prefix.as_deref(), Some("svg"));
}

#[test]
fn namespace_prefix_used_in_selectors() {
    let s = sheet("@namespace svg url(http://www.w3.org/2000/svg); svg|rect { fill: red }");
    assert_eq!(s.rules.len(), 2);
}

#[test]
fn page_selectors_and_margins() {
    let Rule::Page(p) =
        first("@page cover:first, :left { margin: 1in; @top-center { content: 'T' } size: a4 }")
    else {
        panic!()
    };
    assert_eq!(p.selectors.len(), 2);
    assert_eq!(p.selectors[0].name.as_deref(), Some("cover"));
    assert_eq!(p.selectors[0].pseudo_classes, vec!["first".to_string()]);
    assert_eq!(p.selectors[1].name, None);
    assert_eq!(p.declarations.len(), 2);
    assert_eq!(p.margin_rules.len(), 1);
    assert_eq!(p.margin_rules[0].name, "top-center");
}

#[test]
fn page_bad_pseudo_dropped() {
    let s = sheet("@page :nope { margin: 0 } a {}");
    assert_eq!(s.rules.len(), 1);
}

#[test]
fn unknown_at_rule_kept_with_prelude_and_block() {
    let Rule::Unknown(u) = first("@x-custom one two { three }") else {
        panic!()
    };
    assert_eq!(u.name, "x-custom");
    assert!(!u.prelude.is_empty());
    assert!(u.block.is_some());
}

#[test]
fn unknown_at_rule_reports_diagnostic() {
    let s = sheet("@x-custom one;");
    assert!(
        s.diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnknownAtRule)
    );
}

#[test]
fn unknown_at_rule_block_with_braces_in_strings() {
    let s = sheet("@x-custom { '}' } a { color: red }");
    assert_eq!(s.rules.len(), 2);
}

#[test]
fn at_rule_unterminated_at_eof() {
    let s = sheet("@media screen");
    assert!(s.rules.is_empty());
    assert!(!s.diagnostics.is_empty());
}

#[test]
fn children_helper() {
    let r = first("@media screen { a {} b {} }");
    assert_eq!(r.children().len(), 2);
}
