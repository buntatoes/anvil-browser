// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Restyling reuses the parsed page sheet until a sheet changes.

use anvil_browser::{Page, Viewport, render_html};
use anvil_paint::DisplayItem;
use anvil_style::Color;

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
};

const PAGE: &str = "<html><head><style id=s>p { color: #00ff00 } \
                    @media (max-width: 300px) { p { color: #0000ff } } \
                    .on { color: #ff00ff }</style></head>\
                    <body><p id=t>Text</p></body></html>";

fn text_color(page: &Page, needle: &str) -> Color {
    page.display_list
        .items
        .iter()
        .find_map(|item| match item {
            DisplayItem::Text { text, color, .. } if text == needle => Some(*color),
            _ => None,
        })
        .expect("text is drawn")
}

fn color(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

fn style_node(page: &Page) -> anvil_dom::NodeId {
    page.document.get_element_by_id("s").expect("style element")
}

#[test]
fn restaging_without_changes_parses_once() {
    let mut page = render_html(PAGE, "index.html", VP);
    assert_eq!(page.style_cache.parse_count(), 1);
    for frame in 0..20 {
        page.restage_at(VP, f64::from(frame) * 16.0);
    }
    assert_eq!(page.style_cache.parse_count(), 1);
    let fresh = render_html(PAGE, "index.html", VP);
    assert_eq!(page.display_list, fresh.display_list);
}

#[test]
fn a_new_viewport_reuses_the_sheet_but_tests_media_again() {
    let mut page = render_html(PAGE, "index.html", VP);
    let narrow = Viewport {
        width: 250.0,
        height: 300.0,
    };
    page.restage(narrow);
    assert_eq!(text_color(&page, "Text"), color("#0000ff"));
    page.restage(VP);
    assert_eq!(text_color(&page, "Text"), color("#00ff00"));
    assert_eq!(page.style_cache.parse_count(), 1);
}

#[test]
fn class_changes_restyle_without_parsing() {
    let mut page = render_html(PAGE, "index.html", VP);
    let p = page.document.get_element_by_id("t").unwrap();
    page.document.add_class(p, "on");
    page.restage(VP);
    assert_eq!(text_color(&page, "Text"), color("#ff00ff"));
    assert_eq!(page.style_cache.parse_count(), 1);
}

#[test]
fn changing_style_text_parses_again() {
    let mut page = render_html(PAGE, "index.html", VP);
    let style = style_node(&page);
    page.document
        .set_text_content(style, "p { color: #ff0000 }");
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 2);
    assert_eq!(text_color(&page, "Text"), color("#ff0000"));
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 2);
}

#[test]
fn changing_media_parses_again() {
    let mut page = render_html(PAGE, "index.html", VP);
    let style = style_node(&page);
    page.document.set_attribute(style, "media", "print");
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 2);
    assert_eq!(text_color(&page, "Text"), Color::parse("#000000").unwrap());
}

#[test]
fn adding_or_removing_a_style_element_parses_again() {
    let mut page = render_html(PAGE, "index.html", VP);
    let head = page.document.head();
    let extra = page.document.create_element("style", Vec::new());
    let text = page.document.create_text("p { color: #123456 }");
    page.document.append(extra, text).unwrap();
    page.document.append(head, extra).unwrap();
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 2);
    assert_eq!(text_color(&page, "Text"), color("#123456"));
    page.document.remove_child(head, extra).unwrap();
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 3);
    assert_eq!(text_color(&page, "Text"), color("#00ff00"));
}

#[test]
fn changing_the_supplied_css_parses_again() {
    let mut page = render_html(PAGE, "index.html", VP);
    page.external_css = "b { color: #ff0000 }".to_string();
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 2);
    page.linked_css
        .push(("x.css".to_string(), "p { color: #ff0000 }".to_string()));
    page.restage(VP);
    assert_eq!(page.style_cache.parse_count(), 3);
    // No link names x.css, so the page still looks the same.
    assert_eq!(text_color(&page, "Text"), color("#00ff00"));
}
