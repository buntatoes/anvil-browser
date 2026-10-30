// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The four entry points, used the way a style system would use them.

mod common;

use prism::media::MediaEnv;
use prism::selectors::{MatchContext, RuleIndex};
use prism::{
    Options, PropertyValue, Rule, Value, parse_declarations, parse_selector_list, parse_stylesheet,
    parse_value,
};

#[test]
fn stylesheet_to_matched_declarations() {
    let css = "
        @media print { p { color: black } }
        @media screen { .note { color: blue } }
        p { margin: 0 }
        #main .note { color: green !important }
    ";
    let sheet = parse_stylesheet(css, &Options::default()).unwrap();
    let env = MediaEnv::default();
    let mut index = RuleIndex::new();
    let mut stack: Vec<&Rule> = sheet.rules.iter().rev().collect();
    while let Some(rule) = stack.pop() {
        match rule {
            Rule::Media(m) if !m.queries.evaluate(&env) => {}
            Rule::Style(s) => {
                for sel in s.selectors.selectors() {
                    index.insert(sel.clone(), &s.declarations);
                }
            }
            other => stack.extend(other.children().iter().rev()),
        }
    }
    let mut t = common::Tree::default();
    let root = t.add(None, "div#main");
    let p = t.add(Some(root), "p.note");
    let tree = t.finish();
    let el = common::El::new(&tree, p);
    let found = index.matching(&el, &MatchContext::default());
    let names: Vec<String> = found
        .iter()
        .flat_map(|(_, decls)| decls.iter().map(|d| d.name.clone()))
        .collect();
    assert!(names.contains(&"margin".to_string()));
    assert_eq!(
        names.iter().filter(|n| *n == "color").count(),
        2,
        "print rule skipped"
    );
}

#[test]
fn style_attribute() {
    let block = parse_declarations("color: red; width: calc(100% - 2em)").unwrap();
    assert_eq!(block.declarations.len(), 2);
    assert!(block.diagnostics.is_empty());
    assert!(block.declarations[1].parse_value().is_ok());
}

#[test]
fn query_selector_use() {
    let list = parse_selector_list("div > p.note, #main").unwrap();
    assert_eq!(list.selectors().len(), 2);
}

#[test]
fn parse_value_entry_point() {
    let v = parse_value("margin", "1px auto").unwrap();
    let PropertyValue::Values(items) = v else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[1], Value::Ident(ref s) if s == "auto"));
}

#[test]
fn diagnostics_display_text() {
    let sheet = parse_stylesheet("a { color red }", &Options::default()).unwrap();
    let text = sheet.diagnostics[0].to_string();
    assert!(text.contains(".."), "{text}");
}

#[test]
fn types_are_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<prism::Stylesheet>();
    check::<prism::SelectorList>();
    check::<prism::PropertyValue>();
    check::<prism::Error>();
}

#[test]
fn stylesheet_is_clone_and_eq() {
    let a = parse_stylesheet("a { b: c }", &Options::default()).unwrap();
    let b = a.clone();
    assert_eq!(a, b);
}
