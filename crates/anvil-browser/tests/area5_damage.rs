// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Incremental layout reuses clean subtrees. Damage is the changed border
//! boxes, and a clipped repaint leaves every other pixel alone.

use std::collections::HashMap;

use anvil_dom::{Document, NodeId, parse_html};
use anvil_layout::{
    Image, LayoutBox, LayoutCache, layout_document, layout_document_incremental,
    layout_document_interactive,
};
use anvil_paint::{damage_between, paint};
use anvil_raster::{paint_clipped, rasterize};
use anvil_style::{Stylesheet, compute_styles, compute_styles_for_viewport};

fn layout_pair(
    html: &str,
) -> (
    anvil_layout::LayoutBox,
    anvil_layout::LayoutBox,
    anvil_dom::Document,
) {
    let doc = parse_html(html);
    let styles = compute_styles(&doc, &Stylesheet::default());
    let full = layout_document(&doc, &styles, 400.0);
    let mut cache = LayoutCache::new();
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    (full, incremental, doc)
}

#[test]
fn incremental_layout_matches_a_full_layout_after_several_mutations() {
    let html = "<div id=\"a\">one</div><div id=\"b\">two</div><div id=\"c\">three</div>";
    let (full, first, mut doc) = layout_pair(html);
    assert_eq!(full, first);
    let a = doc.get_element_by_id("a").unwrap();
    doc.set_text_content(a, "changed");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let _ =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    doc.set_attribute(doc.get_element_by_id("b").unwrap(), "style", "color:#00f");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let full = layout_document(&doc, &styles, 400.0);
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(full, incremental);

    let extra = doc.create_element("div", vec![]);
    doc.set_text_content(extra, "added");
    doc.append(doc.body(), extra).unwrap();
    let styles = compute_styles(&doc, &Stylesheet::default());
    let full = layout_document(&doc, &styles, 400.0);
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(full, incremental);

    doc.remove_node(extra);
    let styles = compute_styles(&doc, &Stylesheet::default());
    let full = layout_document(&doc, &styles, 400.0);
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(full, incremental);

    doc.set_attribute(doc.get_element_by_id("c").unwrap(), "style", "display:none");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let full = layout_document(&doc, &styles, 400.0);
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(full, incremental);
}

#[test]
fn a_leaf_mutation_reuses_clean_subtrees_and_damages_only_that_leaf() {
    let mut html = String::from("<div id=\"row\">");
    for i in 0..24 {
        html.push_str(&format!("<div id=\"n{i}\">item {i}</div>"));
    }
    html.push_str("</div><div id=\"far\">untouched</div>");
    let doc = parse_html(&html);
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let before =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let before_reuses = cache.subtree_reuses();
    let mut doc = doc;
    let leaf = doc.get_element_by_id("n3").unwrap();
    doc.set_text_content(leaf, "edited leaf");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let after =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let full = layout_document(&doc, &styles, 400.0);
    assert_eq!(after, full);
    assert!(
        cache.subtree_reuses() > before_reuses + 8,
        "clean siblings were not reused: {} -> {}",
        before_reuses,
        cache.subtree_reuses()
    );
    let damage = damage_between(&before, &after);
    let far = after
        .find_node_box(doc.get_element_by_id("far").unwrap())
        .unwrap();
    assert!(
        damage.x + damage.width <= far.rect.x + 1.0 || damage.y + damage.height <= far.rect.y + 1.0,
        "damage {:?} reached the untouched box {:?}",
        damage,
        far.rect
    );
    let leaf_box = after.find_node_box(leaf).unwrap();
    assert!(
        damage.x <= leaf_box.rect.x && damage.y <= leaf_box.rect.y,
        "damage {:?} missed the edited leaf {:?}",
        damage,
        leaf_box.rect
    );

    let old_list = paint(&before);
    let new_list = paint(&after);
    let old_pixels = rasterize(&old_list, 400, 200, 0.0);
    let mut clipped = rasterize(&old_list, 400, 200, 0.0);
    paint_clipped(&mut clipped, &new_list, 0.0, damage);
    let full_pixels = rasterize(&new_list, 400, 200, 0.0);
    for y in 0..200 {
        for x in 0..400 {
            let fx = x as f32;
            let fy = y as f32;
            let inside = fx >= damage.x + 2.0
                && fy >= damage.y + 2.0
                && fx + 2.0 < damage.x + damage.width
                && fy + 2.0 < damage.y + damage.height;
            let outside = fx + 2.0 < damage.x
                || fy + 2.0 < damage.y
                || fx >= damage.x + damage.width + 2.0
                || fy >= damage.y + damage.height + 2.0;
            let clipped_px = clipped.pixel(x, y);
            if inside {
                assert_eq!(clipped_px, full_pixels.pixel(x, y), "inside {x},{y}");
            } else if outside {
                assert_eq!(clipped_px, old_pixels.pixel(x, y), "outside {x},{y}");
            }
        }
    }
}

fn live_nodes(doc: &Document) -> usize {
    fn walk(doc: &Document, id: NodeId) -> usize {
        1 + doc
            .children(id)
            .iter()
            .map(|child| walk(doc, *child))
            .sum::<usize>()
    }
    walk(doc, doc.html())
}

fn pixels(tree: &LayoutBox) -> anvil_raster::Framebuffer {
    rasterize(&paint(tree), 400, 200, 0.0)
}

#[test]
fn a_font_change_matches_a_full_layout() {
    let mut doc = parse_html("<p id=\"t\">Anvil</p>");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let _ =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    doc.set_attribute(
        doc.get_element_by_id("t").unwrap(),
        "style",
        "font-size:32px;font-family:monospace",
    );
    let styles = compute_styles(&doc, &Stylesheet::default());
    let hits = cache.hits();
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(cache.hits(), hits);
    let full = layout_document(&doc, &styles, 400.0);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
}

fn assert_same_pixels(left: &LayoutBox, right: &LayoutBox) {
    let a = pixels(left);
    let b = pixels(right);
    for y in 0..200 {
        for x in 0..400 {
            assert_eq!(a.pixel(x, y), b.pixel(x, y), "{x},{y}");
        }
    }
}

#[test]
fn two_documents_on_one_thread_do_not_share_boxes() {
    let html = "<div id=\"a\">one</div><div id=\"b\">two</div><div id=\"c\">three</div>";
    let mut first = parse_html(html);
    let styles = compute_styles(&first, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let _ = layout_document_incremental(
        &first,
        &styles,
        400.0,
        &Default::default(),
        None,
        &mut cache,
    );
    assert_eq!(cache.nodes_fingerprinted(), live_nodes(&first));
    first.set_text_content(first.get_element_by_id("a").unwrap(), "changed");
    let styles = compute_styles(&first, &Stylesheet::default());
    let _ = layout_document_incremental(
        &first,
        &styles,
        400.0,
        &Default::default(),
        None,
        &mut cache,
    );
    let reuses = cache.subtree_reuses();
    assert!(
        reuses > 0,
        "expected sibling reuse inside the first document"
    );
    let second = parse_html(html);
    let styles = compute_styles(&second, &Stylesheet::default());
    let incremental = layout_document_incremental(
        &second,
        &styles,
        400.0,
        &Default::default(),
        None,
        &mut cache,
    );
    assert_eq!(cache.subtree_reuses(), reuses);
    assert_eq!(cache.hits(), 0);
    let full = layout_document(&second, &styles, 400.0);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
}

#[test]
fn a_cloned_document_does_not_reuse_the_source_cache() {
    let doc = parse_html("<div id=\"a\">one</div><div id=\"b\">two</div>");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let staged =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let cloned = doc.clone();
    assert_ne!(cloned.identity(), doc.identity());
    let styles = compute_styles(&cloned, &Stylesheet::default());
    let reuses = cache.subtree_reuses();
    let incremental = layout_document_incremental(
        &cloned,
        &styles,
        400.0,
        &Default::default(),
        None,
        &mut cache,
    );
    assert_eq!(cache.subtree_reuses(), reuses);
    assert_eq!(
        cache.hits(),
        0,
        "the staged frame was accepted for the clone"
    );
    let full = layout_document(&cloned, &styles, 400.0);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
    let _ = staged;
}

#[test]
fn focus_changes_the_laid_out_value() {
    let field = parse_html("<input id=\"q\" value=\"ab\">");
    let mut cache = LayoutCache::new();
    let styles = compute_styles(&field, &Stylesheet::default());
    let id = field.get_element_by_id("q").unwrap();
    let plain = layout_document_incremental(
        &field,
        &styles,
        200.0,
        &Default::default(),
        None,
        &mut cache,
    );
    let focused = layout_document_incremental(
        &field,
        &styles,
        200.0,
        &Default::default(),
        Some(id),
        &mut cache,
    );
    fn joined(tree: &anvil_layout::LayoutBox) -> String {
        let mut out = String::new();
        for run in &tree.text {
            out.push_str(&run.text);
        }
        for child in &tree.children {
            out.push_str(&joined(child));
        }
        out
    }
    let plain_text = joined(&plain);
    let focused_text = joined(&focused);
    assert_ne!(
        plain_text, focused_text,
        "plain={plain_text} focused={focused_text}"
    );
    assert!(plain_text.contains("ab"), "{plain_text}");
    assert!(focused_text.contains("ab"), "{focused_text}");
}

#[test]
fn a_focus_change_matches_a_full_layout() {
    let doc = parse_html("<input id=\"q\" type=\"text\" value=\"hi\">");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let id = doc.get_element_by_id("q").unwrap();
    let mut cache = LayoutCache::new();
    let _ =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let hits = cache.hits();
    let incremental = layout_document_incremental(
        &doc,
        &styles,
        400.0,
        &Default::default(),
        Some(id),
        &mut cache,
    );
    assert_eq!(cache.hits(), hits);
    let full = layout_document_interactive(&doc, &styles, 400.0, &Default::default(), Some(id));
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
}

/// `:focus` paints a solid outline and background. The damage rect covers
/// the control, pixels inside that rect change, and pixels outside it do not.
#[test]
fn focus_paints_a_visible_outline_inside_the_damage_rect() {
    let html = "<style>\
            input:focus { outline: 4px solid rgb(255, 0, 0); background-color: rgb(255, 255, 0); }\
         </style>\
         <input id=\"q\" value=\"ab\" style=\"position:absolute;left:8px;top:8px;width:80px;height:24px\">\
         <div id=\"far\" style=\"position:absolute;left:8px;top:140px\">untouched</div>";
    let mut doc = parse_html(html);
    let id = doc.get_element_by_id("q").unwrap();
    let far = doc.get_element_by_id("far").unwrap();
    let sheet = anvil_style::parse_css(&doc.embedded_css());
    let plain_styles = compute_styles(&doc, &sheet);
    let mut cache = LayoutCache::new();
    let before = layout_document_incremental(
        &doc,
        &plain_styles,
        400.0,
        &Default::default(),
        None,
        &mut cache,
    );
    let before_box = before.find_node_box(id).expect("input");
    assert_eq!(
        before_box.outline_width, 0.0,
        "the unfocused control already had an outline"
    );
    doc.set_focused_node(Some(id));
    let focused_styles = compute_styles(&doc, &sheet);
    let after = layout_document_incremental(
        &doc,
        &focused_styles,
        400.0,
        &Default::default(),
        Some(id),
        &mut cache,
    );
    let after_box = after.find_node_box(id).expect("focused input");
    assert!(
        after_box.outline_width >= 4.0,
        "focus did not paint an outline: {}",
        after_box.outline_width
    );
    let damage = damage_between(&before, &after);
    let control = after_box.rect;
    assert!(
        damage.x <= control.x
            && damage.y <= control.y
            && damage.x + damage.width >= control.x + control.width
            && damage.y + damage.height >= control.y + control.height,
        "damage {damage:?} does not cover {control:?}"
    );
    let far_box = after.find_node_box(far).expect("far");
    assert!(
        far_box.rect.y >= damage.y + damage.height,
        "damage {damage:?} reached the untouched box {:?}",
        far_box.rect
    );
    let old_pixels = rasterize(&paint(&before), 400, 200, 0.0);
    let new_pixels = rasterize(&paint(&after), 400, 200, 0.0);
    let mut changed_inside = 0u32;
    for y in 0..200 {
        for x in 0..400 {
            let fx = x as f32;
            let fy = y as f32;
            let inside = fx >= damage.x
                && fy >= damage.y
                && fx < damage.x + damage.width
                && fy < damage.y + damage.height;
            let outside = fx + 1.0 < damage.x
                || fy + 1.0 < damage.y
                || fx >= damage.x + damage.width + 1.0
                || fy >= damage.y + damage.height + 1.0;
            let old_px = old_pixels.pixel(x, y);
            let new_px = new_pixels.pixel(x, y);
            if inside && old_px != new_px {
                changed_inside += 1;
            } else if outside {
                assert_eq!(old_px, new_px, "outside {x},{y}");
            }
        }
    }
    assert!(
        changed_inside > 0,
        "focus left the damage rect {damage:?} unchanged"
    );
}

#[test]
fn an_image_load_matches_a_full_layout() {
    let doc = parse_html("<img id=\"p\" alt=\"\">");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let id = doc.get_element_by_id("p").unwrap();
    let mut cache = LayoutCache::new();
    let _ =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let mut images = HashMap::new();
    images.insert(id, Image::new(40, 20));
    let hits = cache.hits();
    let incremental = layout_document_incremental(&doc, &styles, 400.0, &images, None, &mut cache);
    assert_eq!(cache.hits(), hits);
    let full = layout_document_interactive(&doc, &styles, 400.0, &images, None);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
}

#[test]
fn a_viewport_resize_matches_a_full_layout() {
    let doc = parse_html("<div style=\"width:50vw\">wide</div>");
    let wide = compute_styles_for_viewport(&doc, &Stylesheet::default(), 400.0, 200.0);
    let mut cache = LayoutCache::new();
    let _ = layout_document_incremental(&doc, &wide, 400.0, &Default::default(), None, &mut cache);
    let narrow = compute_styles_for_viewport(&doc, &Stylesheet::default(), 200.0, 100.0);
    let hits = cache.hits();
    let incremental =
        layout_document_incremental(&doc, &narrow, 200.0, &Default::default(), None, &mut cache);
    assert_eq!(cache.hits(), hits);
    let full = layout_document(&doc, &narrow, 200.0);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
}

#[test]
fn a_mutation_during_publish_is_not_stored_under_the_new_generation() {
    let mut doc = parse_html("<div id=\"a\">one</div><div id=\"b\">two</div>");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let staged =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let started = doc.mutation_generation();
    doc.set_text_content(doc.get_element_by_id("a").unwrap(), "later");
    assert!(!cache.commit_incremental_pass(&doc, started, staged));
    let styles = compute_styles(&doc, &Stylesheet::default());
    let incremental =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let full = layout_document(&doc, &styles, 400.0);
    assert_eq!(incremental, full);
    assert_same_pixels(&incremental, &full);
    assert_ne!(doc.mutation_generation(), started);
}

#[test]
fn navigation_rejects_the_previous_generation_on_restage() {
    let mut doc = parse_html("<div id=\"a\">first page</div>");
    let styles = compute_styles(&doc, &Stylesheet::default());
    let mut cache = LayoutCache::new();
    let staged =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    let old_generation = doc.mutation_generation();
    doc.set_text_content(doc.get_element_by_id("a").unwrap(), "second page");
    assert_ne!(doc.mutation_generation(), old_generation);
    let styles = compute_styles(&doc, &Stylesheet::default());
    let hits = cache.hits();
    let restaged =
        layout_document_incremental(&doc, &styles, 400.0, &Default::default(), None, &mut cache);
    assert_eq!(
        cache.hits(),
        hits,
        "old generation was restaged from the cache"
    );
    let full = layout_document(&doc, &styles, 400.0);
    assert_eq!(restaged, full);
    assert_same_pixels(&restaged, &full);
    assert_ne!(restaged, staged);
}
