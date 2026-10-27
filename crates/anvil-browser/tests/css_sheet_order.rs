// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! `media` on `<style>` limits the sheet. Linked sheets are checked in the
//! unit tests next to `page_stylesheet`, which can hand in fetched bodies.

use anvil_browser::{Page, Viewport, render_html};
use anvil_paint::DisplayItem;
use anvil_style::Color;

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
};

/// Colour the text `needle` was drawn with.
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

#[test]
fn media_on_a_style_element_limits_the_sheet() {
    let page = render_html(
        "<style>p { color: #00ff00 }</style>\
         <style media=print>p { color: #ff0000 }</style>\
         <style media='not print'>b { color: #00ff00 }</style>\
         <style media='(max-width: 200px)'>b { color: #ff0000 }</style>\
         <p>Print</p><b>Screen</b>",
        "index.html",
        VP,
    );
    assert_eq!(text_color(&page, "Print"), color("#00ff00"));
    assert_eq!(text_color(&page, "Screen"), color("#00ff00"));
}

#[test]
fn restaging_keeps_style_media() {
    let mut page = render_html(
        "<style>p { color: #00ff00 }</style>\
         <style media=print>p { color: #ff0000 }</style><p>Text</p>",
        "index.html",
        VP,
    );
    page.restage(VP);
    assert_eq!(text_color(&page, "Text"), color("#00ff00"));
}
