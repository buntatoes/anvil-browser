// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! HTML parser for [`crate::Document`].
//!
//! This is the front of the engine pipeline. Bytes become a node list that
//! [`crate::Document`] owns. anvil-style reads that tree next; this module
//! does not compute style, layout, or pixels. anvil-script may mutate the
//! tree later, and anvil-browser keeps the document.
//!
//! Image and video elements are stored as ordinary elements. Their `src`
//! values are collected by the document so a later stage can decode images
//! with anvil-image and hand `<video>` to anvil-video. This parser does not
//! decode either, and this crate does not depend on those libraries.
//!
//! Caps, checked while scanning:
//! - [`MAX_DOM_DEPTH`] (256) limits how deep the open-element stack grows.
//! - [`MAX_DOM_NODES`] (50_000) stops allocation. Leftover input is ignored.
//! - [`MAX_ATTRS_PER_ELEMENT`] (128) drops further attributes on one start tag.

use crate::{Document, NodeData, NodeId};

/// Open-element stack limit. A start tag past this depth is still created,
/// but it is not pushed, so following tokens stay under the current parent.
pub const MAX_DOM_DEPTH: usize = 256;

/// Node-list limit for one document, including text nodes created here.
/// [`crate::Document::set_inner_html`] uses the same cap when cloning.
pub const MAX_DOM_NODES: usize = 250_000;

/// Attributes kept from one start tag. Later attributes on that tag are dropped.
pub const MAX_ATTRS_PER_ELEMENT: usize = 256;

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const HEAD_TAGS: &[&str] = &["base", "link", "meta", "style", "title"];

/// Parse an HTML document.
pub fn parse_html(input: &str) -> Document {
    parse_html_with_options(input, false)
}

/// Parse an HTML fragment for `innerHTML` / `insertAdjacentHTML` without diverting
/// `<style>`, `<link>`, `<meta>`, `<title>`, or `<base>` into `doc.head()`.
pub(crate) fn parse_html_fragment(input: &str) -> Document {
    parse_html_with_options(input, true)
}

fn find_comment_end(input: &str, start: usize) -> usize {
    let bytes = input.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"-->") {
            return i + 3;
        }
        if bytes[i..].starts_with(b"--!>") {
            return i + 4;
        }
        i += 1;
    }
    input.len()
}

fn doctype_triggers_quirks(decl: &str) -> bool {
    let trimmed = decl.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("!doctype") {
        return false;
    }
    let after = lower["!doctype".len()..].trim();
    if after == "html" {
        return false;
    }
    !after.starts_with("html ")
        || lower.contains("html 4.0 ")
        || lower.contains("html 3.")
        || lower.contains("html 2.")
        || lower.contains("transitional") && !lower.contains("loose.dtd")
}

fn canonicalize_svg_attr(name: &str) -> String {
    match name {
        "viewbox" => "viewBox".to_string(),
        "preserveaspectratio" => "preserveAspectRatio".to_string(),
        "gradientunits" => "gradientUnits".to_string(),
        "gradienttransform" => "gradientTransform".to_string(),
        "spreadmethod" => "spreadMethod".to_string(),
        "stddeviation" => "stdDeviation".to_string(),
        "patternunits" => "patternUnits".to_string(),
        "patterncontentunits" => "patternContentUnits".to_string(),
        "patterntransform" => "patternTransform".to_string(),
        "markerwidth" => "markerWidth".to_string(),
        "markerheight" => "markerHeight".to_string(),
        "markerunits" => "markerUnits".to_string(),
        "refx" => "refX".to_string(),
        "refy" => "refY".to_string(),
        "basefrequency" => "baseFrequency".to_string(),
        "numoctaves" => "numOctaves".to_string(),
        "clippathunits" => "clipPathUnits".to_string(),
        "maskunits" => "maskUnits".to_string(),
        "maskcontentunits" => "maskContentUnits".to_string(),
        "pathlength" => "pathLength".to_string(),
        "calcmode" => "calcMode".to_string(),
        "keytimes" => "keyTimes".to_string(),
        "keysplines" => "keySplines".to_string(),
        "repeatcount" => "repeatCount".to_string(),
        other => other.to_string(),
    }
}

fn parse_html_with_options(input: &str, fragment_mode: bool) -> Document {
    let mut doc = Document::new();
    let mut stack = vec![doc.body()];
    let mut foreign_depth = 0usize;
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        // The node list is not compacted. Hitting the cap drops the rest of the source.
        if doc.node_count() >= MAX_DOM_NODES {
            break;
        }
        if bytes[index] == b'<' && is_markup(bytes, index) {
            if input[index..].starts_with("<!--") {
                index = find_comment_end(input, index + 4);
                continue;
            }
            if foreign_depth > 0 && input[index..].starts_with("<![CDATA[") {
                let cdata_start = index + 9;
                let end = skip_until(input, cdata_start, "]]>").unwrap_or(input.len());
                let raw_end = if end > cdata_start + 3 && input[..end].ends_with("]]>") {
                    end - 3
                } else {
                    end
                };
                let text = &input[cdata_start..raw_end];
                if !text.is_empty() && doc.node_count() < MAX_DOM_NODES {
                    let parent = *stack.last().unwrap_or(&doc.body());
                    let text_id = doc.create_text(text);
                    doc.append(parent, text_id).expect("new text node");
                }
                index = end;
                continue;
            }
            if input[index..].starts_with("<!") || input[index..].starts_with("<?") {
                let end = skip_char(input, index + 2, '>').unwrap_or(input.len());
                if !fragment_mode && end > index + 2 {
                    let decl_end = if bytes.get(end.saturating_sub(1)) == Some(&b'>') {
                        end - 1
                    } else {
                        end
                    };
                    if decl_end > index + 1 {
                        let decl = &input[index + 1..decl_end];
                        if doctype_triggers_quirks(decl) {
                            doc.set_quirks_mode(true);
                        }
                    }
                }
                index = end;
                continue;
            }
            if bytes.get(index + 1) == Some(&b'/') {
                let (name, next) = read_end_tag(input, index);
                index = next;
                let prev_len = stack.len();
                close_tag(&mut doc, &mut stack, &name);
                if stack.len() < prev_len {
                    foreign_depth = stack
                        .iter()
                        .filter(|&&nid| matches!(doc.get(nid).data().tag(), Some("svg" | "math")))
                        .count();
                }
                continue;
            }

            let (name, mut attrs, self_closing, next) =
                read_start_tag(input, index, foreign_depth > 0);
            index = next;
            if name.is_empty() {
                continue;
            }
            if foreign_depth > 0 {
                for (k, _) in &mut attrs {
                    *k = canonicalize_svg_attr(k);
                }
            }
            if name == "html" {
                if !fragment_mode {
                    let html = doc.html();
                    merge_attributes(&mut doc, html, attrs);
                }
                continue;
            }
            if name == "head" {
                if !fragment_mode {
                    let head = doc.head();
                    merge_attributes(&mut doc, head, attrs);
                    stack.clear();
                    stack.push(head);
                }
                continue;
            }
            if name == "body" {
                if !fragment_mode {
                    let body = doc.body();
                    merge_attributes(&mut doc, body, attrs);
                    stack.clear();
                    stack.push(body);
                    foreign_depth = 0;
                }
                continue;
            }

            // Auto-close <head> if an ordinary body start tag is encountered while still in <head>.
            if !fragment_mode
                && stack.last().copied() == Some(doc.head())
                && !HEAD_TAGS.contains(&name.as_str())
                && name != "script"
                && name != "noscript"
                && name != "template"
            {
                stack.clear();
                stack.push(doc.body());
            }

            auto_close_for_start_tag(&doc, &mut stack, &name);

            let parent = placement_parent(&doc, &stack, &name, fragment_mode);
            if name == "base"
                && doc.base_href().is_none()
                && let Some((_, href)) = attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case("href"))
            {
                doc.set_base_href(href.clone());
            }
            let id = doc.create_element(&name, attrs);
            doc.append(parent, id).expect("new node has no children");

            if name == "noscript" {
                if let Some((_, next)) = read_raw_text_html5(input, index, "noscript", false) {
                    index = next;
                }
                continue;
            }

            if name == "script" || name == "style" {
                if let Some((text, next)) =
                    read_raw_text_html5(input, index, &name, name == "script")
                {
                    if let Some(normalized) = preserve_text(&text)
                        && !normalized.is_empty()
                        && doc.node_count() < MAX_DOM_NODES
                    {
                        let text_id = doc.create_text(&normalized);
                        doc.append(id, text_id).expect("new text node");
                    }
                    index = next;
                }
                continue;
            }

            if foreign_depth == 0 && (name == "title" || name == "textarea") {
                if let Some((text, next)) = read_raw_text_html5(input, index, &name, false) {
                    let decoded = decode_entities(&text, false);
                    let stored = if name == "textarea" {
                        preserve_text(&decoded)
                    } else {
                        normalize_text(&decoded)
                    };
                    if let Some(val) = stored
                        && doc.node_count() < MAX_DOM_NODES
                    {
                        let text_id = doc.create_text(&val);
                        doc.append(id, text_id).expect("new text node");
                    }
                    index = next;
                }
                continue;
            }

            if foreign_depth == 0
                && matches!(name.as_str(), "iframe" | "xmp" | "noembed" | "noframes")
            {
                if let Some((text, next)) = read_raw_text_html5(input, index, &name, false) {
                    if !text.is_empty() && doc.node_count() < MAX_DOM_NODES {
                        let text_id = doc.create_text(&text);
                        doc.append(id, text_id).expect("new text node");
                    }
                    index = next;
                }
                continue;
            }

            let enters_foreign = matches!(name.as_str(), "svg" | "math");
            if !(is_void(&name) || (foreign_depth > 0 && self_closing))
                && !(enters_foreign && self_closing)
                && stack.len() < MAX_DOM_DEPTH
            {
                stack.push(id);
                if enters_foreign {
                    foreign_depth += 1;
                }
            }
            continue;
        }

        // Text runs to the next `<` that starts markup.
        let start = index;
        index += 1;
        while index < bytes.len() && !(bytes[index] == b'<' && is_markup(bytes, index)) {
            index += 1;
        }
        let decoded = decode_entities(&input[start..index], false);
        if !fragment_mode
            && stack.last().copied() == Some(doc.head())
            && decoded.chars().any(|c| !c.is_whitespace())
        {
            stack.clear();
            stack.push(doc.body());
        }
        let parent = *stack.last().unwrap_or(&doc.body());
        if let Some(text) = text_for_parent(&doc, parent, &decoded)
            && doc.node_count() < MAX_DOM_NODES
        {
            let text_id = doc.create_text(&text);
            doc.append(parent, text_id).expect("new text node");
        }
    }

    doc.clear_dirty_styles();
    doc
}

/// True when `<` begins a tag, comment, or declaration rather than text.
fn is_markup(bytes: &[u8], index: usize) -> bool {
    matches!(
        bytes.get(index + 1).copied(),
        Some(b'/' | b'!' | b'?' | b'a'..=b'z' | b'A'..=b'Z')
    )
}

/// Void tags never take children and are not pushed on the open stack.
fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag)
}

/// Pop implied end tags before block elements, list items, definition items,
/// headings, table cells/rows/sections, `<option>`, or `<optgroup>`.
///
/// Only the open stack is changed. Already-built nodes stay where they are.
fn auto_close_for_start_tag(doc: &Document, stack: &mut Vec<NodeId>, new_tag: &str) {
    match new_tag {
        "address" | "article" | "aside" | "blockquote" | "details" | "div" | "dl" | "fieldset"
        | "figcaption" | "figure" | "footer" | "form" | "header" | "hgroup" | "hr" | "main"
        | "menu" | "nav" | "ol" | "p" | "pre" | "search" | "section" | "table" | "ul" => {
            close_p_in_button_scope(doc, stack);
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            close_p_in_button_scope(doc, stack);
            if let Some(&top) = stack.last()
                && matches!(
                    doc.get(top).data().tag(),
                    Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                )
            {
                stack.pop();
            }
        }
        "li" => {
            close_p_in_button_scope(doc, stack);
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if tag == Some("li") {
                    stack.truncate(idx);
                    break;
                }
                if matches!(
                    tag,
                    Some("ul" | "ol" | "menu" | "table" | "template" | "html" | "body")
                ) {
                    break;
                }
            }
        }
        "dt" | "dd" => {
            close_p_in_button_scope(doc, stack);
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if matches!(tag, Some("dt" | "dd")) {
                    stack.truncate(idx);
                    break;
                }
                if matches!(tag, Some("dl" | "table" | "template" | "html" | "body")) {
                    break;
                }
            }
        }
        "td" | "th" => {
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if matches!(tag, Some("td" | "th")) {
                    stack.truncate(idx);
                    break;
                }
                if matches!(
                    tag,
                    Some("tr" | "thead" | "tbody" | "tfoot" | "table" | "template")
                ) {
                    break;
                }
            }
        }
        "tr" => {
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if tag == Some("tr") {
                    stack.truncate(idx);
                    break;
                }
                if matches!(
                    tag,
                    Some("thead" | "tbody" | "tfoot" | "table" | "template")
                ) {
                    break;
                }
            }
        }
        "thead" | "tbody" | "tfoot" => {
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if matches!(tag, Some("thead" | "tbody" | "tfoot")) {
                    stack.truncate(idx);
                    break;
                }
                if matches!(tag, Some("table" | "template")) {
                    break;
                }
            }
        }
        "option" => {
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if tag == Some("option") {
                    stack.truncate(idx);
                    break;
                }
                if matches!(tag, Some("optgroup" | "select" | "datalist" | "template")) {
                    break;
                }
            }
        }
        "optgroup" => {
            let mut idx = stack.len();
            while idx > 1 {
                idx -= 1;
                let tag = doc.get(stack[idx]).data().tag();
                if matches!(tag, Some("option" | "optgroup")) {
                    stack.truncate(idx);
                    if tag == Some("optgroup") {
                        break;
                    }
                    continue;
                }
                if matches!(tag, Some("select" | "template")) {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn close_p_in_button_scope(doc: &Document, stack: &mut Vec<NodeId>) {
    let mut idx = stack.len();
    while idx > 1 {
        idx -= 1;
        let tag = doc.get(stack[idx]).data().tag();
        if tag == Some("p") {
            stack.truncate(idx);
            return;
        }
        if matches!(
            tag,
            Some("table" | "template" | "html" | "body" | "td" | "th" | "caption" | "button")
        ) {
            return;
        }
    }
}

/// Parent for a new element. Head-only tags under `body` or `html` go to `head`.
///
/// An empty stack falls back to `body`. The returned id is copied from `stack`
/// or from the document skeleton. It is not a newly allocated node.
fn placement_parent(doc: &Document, stack: &[NodeId], tag: &str, fragment_mode: bool) -> NodeId {
    if !fragment_mode
        && HEAD_TAGS.contains(&tag)
        && let Some(top) = stack.last()
        && (*top == doc.body() || doc.get(*top).data().tag() == Some("html"))
    {
        return doc.head();
    }
    stack.last().copied().unwrap_or_else(|| doc.body())
}

/// Copy attributes from an `html`, `head`, or `body` start tag onto the
/// element [`Document::new`] already made. An attribute the element has
/// already got keeps its value, so a second `<body>` only adds new ones.
/// The element never ends up with more than [`MAX_ATTRS_PER_ELEMENT`].
fn merge_attributes(doc: &mut Document, id: NodeId, attrs: Vec<(String, String)>) {
    for (name, value) in attrs {
        let data = doc.get(id).data();
        let count = match data {
            NodeData::Element { attrs, .. } => attrs.len(),
            _ => return,
        };
        if data.attr(&name).is_none() && count < MAX_ATTRS_PER_ELEMENT {
            doc.set_attribute(id, &name, &value);
        }
    }
}

/// Pop the open stack through the matching start tag when that tag is in scope.
///
/// `head`, `body`, and `html` reset the stack to `body`. Stray closing tags whose
/// opening tag is not in scope on the stack are ignored without unwinding outer
/// containers. Per HTML5, a stray `</p>` when no `<p>` is in scope inserts an
/// empty `<p></p>` element, and `</br>` inserts a `<br>`.
fn close_tag(doc: &mut Document, stack: &mut Vec<NodeId>, name: &str) {
    if name.is_empty() {
        return;
    }
    if name == "head" || name == "body" || name == "html" {
        stack.clear();
        stack.push(doc.body());
        return;
    }
    if name == "br" {
        if doc.node_count() < MAX_DOM_NODES {
            let parent = *stack.last().unwrap_or(&doc.body());
            let br = doc.create_element("br", Vec::new());
            let _ = doc.append(parent, br);
        }
        return;
    }
    let is_heading = matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    let is_table_scope_target = matches!(
        name,
        "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "caption"
    );
    let mut found_idx: Option<usize> = None;
    let mut idx = stack.len();
    while idx > 1 {
        idx -= 1;
        let tag = doc.get(stack[idx]).data().tag();
        if tag == Some(name)
            || (is_heading && matches!(tag, Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")))
        {
            found_idx = Some(idx);
            break;
        }
        if !is_table_scope_target
            && matches!(
                tag,
                Some("table" | "template" | "html" | "td" | "th" | "caption")
            )
        {
            break;
        }
    }
    if let Some(target_idx) = found_idx {
        stack.truncate(target_idx);
    } else if name == "p" && doc.node_count() < MAX_DOM_NODES {
        let parent = *stack.last().unwrap_or(&doc.body());
        let p = doc.create_element("p", Vec::new());
        let _ = doc.append(parent, p);
    }
    if stack.is_empty() {
        stack.push(doc.body());
    }
}

/// Read a start tag at `start` (the `<` byte).
///
/// Returns the lowercase name, owned attribute pairs, and the index after `>`.
/// A `/` before `>` is ignored, so a self-closing slash does not by itself make
/// the tag void. Attribute values are entity-decoded. Past
/// [`MAX_ATTRS_PER_ELEMENT`], further attributes are skipped.
fn read_start_tag(
    input: &str,
    start: usize,
    foreign: bool,
) -> (String, Vec<(String, String)>, bool, usize) {
    let bytes = input.as_bytes();
    let mut index = start + 1;
    let name = read_name(input, &mut index);
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        index = skip_ascii_space(bytes, index);
        if index >= bytes.len() || bytes[index] == b'>' {
            break;
        }
        if bytes[index] == b'/' {
            if bytes.get(index + 1) == Some(&b'>') {
                self_closing = true;
            }
            index += 1;
            continue;
        }
        let attr_name = read_attr_name(input, &mut index);
        if attr_name.is_empty() {
            // Skip one whole character so the next slice starts on a boundary.
            index += input[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        index = skip_ascii_space(bytes, index);
        let value = if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            index = skip_ascii_space(bytes, index);
            read_attr_value(input, &mut index, foreign)
        } else {
            String::new()
        };
        // Overflow attributes are discarded. The tag itself is still created.
        if attrs.len() < MAX_ATTRS_PER_ELEMENT {
            attrs.push((attr_name, value));
        }
    }
    if index < bytes.len() && bytes[index] == b'>' {
        index += 1;
    }
    (name, attrs, self_closing, index)
}

/// Read an end tag at `start` (the `<` of `</`). Returns the lowercase name
/// and the index after `>`, or the end of input when `>` is missing.
fn read_end_tag(input: &str, start: usize) -> (String, usize) {
    let bytes = input.as_bytes();
    let mut index = start + 2;
    let name = read_name(input, &mut index);
    while index < bytes.len() && bytes[index] != b'>' {
        index += 1;
    }
    if index < bytes.len() {
        index += 1;
    }
    (name, index)
}

/// Read a tag name and advance `index` past it.
fn read_name(input: &str, index: &mut usize) -> String {
    let bytes = input.as_bytes();
    let start = *index;
    while *index < bytes.len() && is_name_byte(bytes[*index]) {
        *index += 1;
    }
    input[start..*index].to_ascii_lowercase()
}

/// Read an attribute name (supporting `@`, `.`, `_`, `[`, `]`, `(`, `)`, `:`, `$`).
fn read_attr_name(input: &str, index: &mut usize) -> String {
    let bytes = input.as_bytes();
    let start = *index;
    while *index < bytes.len() && is_attr_name_byte(bytes[*index]) {
        *index += 1;
    }
    input[start..*index].to_ascii_lowercase()
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b':'
}

fn is_attr_name_byte(byte: u8) -> bool {
    !matches!(
        byte,
        b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'>' | b'/' | b'<' | b'"' | b'\''
    )
}

fn read_attr_value(input: &str, index: &mut usize, foreign: bool) -> String {
    let bytes = input.as_bytes();
    if *index >= bytes.len() {
        return String::new();
    }
    let quote = bytes[*index];
    if quote == b'"' || quote == b'\'' {
        *index += 1;
        let start = *index;
        while *index < bytes.len() && bytes[*index] != quote {
            *index += 1;
        }
        let value = decode_entities(&input[start..*index], true);
        if *index < bytes.len() {
            *index += 1;
        }
        return value;
    }
    let start = *index;
    while *index < bytes.len() && !matches!(bytes[*index], b' ' | b'\t' | b'\n' | b'\r' | b'>') {
        if foreign && bytes[*index] == b'/' && bytes.get(*index + 1) == Some(&b'>') {
            break;
        }
        *index += 1;
    }
    decode_entities(&input[start..*index], true)
}

/// HTML5 rawtext / script-data end tag scanner.
fn read_raw_text_html5(
    input: &str,
    start: usize,
    tag: &str,
    track_js_strings: bool,
) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    let tag_bytes = tag.as_bytes();
    let mut i = start;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_template = false;
    let mut escaped = false;
    let mut html_escaped_comment = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < bytes.len() {
        let b = bytes[i];
        if track_js_strings {
            if in_line_comment {
                if b == b'\n' || b == b'\r' {
                    in_line_comment = false;
                }
            } else if in_block_comment {
                if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    in_block_comment = false;
                    i += 2;
                    continue;
                }
            } else if in_single || in_double || in_template {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if (in_single && b == b'\'')
                    || (in_double && b == b'"')
                    || (in_template && b == b'`')
                {
                    in_single = false;
                    in_double = false;
                    in_template = false;
                } else if b == b'\n' && (in_single || in_double) {
                    in_single = false;
                    in_double = false;
                }
                i += 1;
                continue;
            } else {
                if b == b'<' && bytes[i..].starts_with(b"<!--") {
                    html_escaped_comment = true;
                } else if html_escaped_comment && b == b'-' && bytes[i..].starts_with(b"-->") {
                    html_escaped_comment = false;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    in_line_comment = true;
                    i += 2;
                    continue;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    in_block_comment = true;
                    i += 2;
                    continue;
                } else if b == b'\'' {
                    in_single = true;
                    i += 1;
                    continue;
                } else if b == b'"' {
                    in_double = true;
                    i += 1;
                    continue;
                } else if b == b'`' {
                    in_template = true;
                    i += 1;
                    continue;
                }
            }
        }

        if b == b'<'
            && bytes.get(i + 1) == Some(&b'/')
            && bytes
                .get(i + 2..i + 2 + tag_bytes.len())
                .is_some_and(|cand| cand.eq_ignore_ascii_case(tag_bytes))
            && matches!(
                bytes.get(i + 2 + tag_bytes.len()),
                Some(b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r') | None
            )
            && !(track_js_strings && html_escaped_comment)
        {
            let text = input[start..i].to_string();
            let mut end = i + 2 + tag_bytes.len();
            while end < bytes.len() && bytes[end] != b'>' {
                end += 1;
            }
            if end < bytes.len() {
                end += 1;
            }
            return Some((text, end));
        }
        i += 1;
    }

    Some((input[start..].to_string(), input.len()))
}

/// Index just after `marker`, or `None` if `marker` does not occur.
fn skip_until(input: &str, start: usize, marker: &str) -> Option<usize> {
    input[start..]
        .find(marker)
        .map(|offset| start + offset + marker.len())
}

/// Index just after the next `marker` character, or `None` if it does not occur.
fn skip_char(input: &str, start: usize, marker: char) -> Option<usize> {
    input[start..]
        .find(marker)
        .map(|offset| start + offset + marker.len_utf8())
}

/// Advance `index` past ASCII whitespace. The returned index may be `bytes.len()`.
fn skip_ascii_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

/// Decode named and numeric character references into an owned string.
///
/// Named names: `amp`, `lt`, `gt`, `quot`, `apos`, `nbsp`. A decimal
/// reference is `&#39;`. A hex reference is `&#x27;` or `&#X27;`. A `&` with
/// no `;`, a name this list does not know, or a number that is not a scalar
/// is copied through. The input is borrowed; the output is owned.
fn legacy_semicolonless_entity(rest: &str, in_attr: bool) -> Option<(char, usize)> {
    const LEGACY: &[(&str, char)] = &[
        ("amp", '&'),
        ("AMP", '&'),
        ("lt", '<'),
        ("LT", '<'),
        ("gt", '>'),
        ("GT", '>'),
        ("quot", '"'),
        ("QUOT", '"'),
        ("nbsp", '\u{00a0}'),
        ("copy", '\u{00a9}'),
        ("reg", '\u{00ae}'),
    ];
    for &(name, ch) in LEGACY {
        if let Some(after) = rest[1..].strip_prefix(name) {
            let next_b = after.as_bytes().first().copied();
            if in_attr && next_b.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric()) {
                continue;
            }
            if next_b.is_some_and(|b| b.is_ascii_alphanumeric()) {
                continue;
            }
            return Some((ch, 1 + name.len()));
        }
    }
    None
}

fn decode_entities(input: &str, in_attr: bool) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let semi = rest[1..].find(';').map(|n| n + 1).filter(|&n| n <= 32);
        if let Some(end) = semi {
            let entity = &rest[1..end];
            if let Some(ch) = decode_one_entity(entity) {
                out.push(ch);
                rest = &rest[end + 1..];
                continue;
            }
        }
        if let Some((ch, consumed)) = legacy_semicolonless_entity(rest, in_attr) {
            out.push(ch);
            rest = &rest[consumed..];
            continue;
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

/// One character for an entity name or numeric body, without the leading `&`
/// or the trailing `;`. `None` when it is not a reference this parser knows.
fn decode_one_entity(entity: &str) -> Option<char> {
    let named = match entity {
        "amp" | "AMP" => Some('&'),
        "lt" | "LT" => Some('<'),
        "gt" | "GT" => Some('>'),
        "quot" | "QUOT" => Some('"'),
        "apos" => Some('\''),
        "nbsp" | "NBSP" => Some('\u{00a0}'),
        "iexcl" => Some('\u{00a1}'),
        "cent" => Some('\u{00a2}'),
        "pound" => Some('\u{00a3}'),
        "curren" => Some('\u{00a4}'),
        "yen" => Some('\u{00a5}'),
        "brvbar" => Some('\u{00a6}'),
        "sect" => Some('\u{00a7}'),
        "uml" => Some('\u{00a8}'),
        "copy" | "COPY" => Some('\u{00a9}'),
        "ordf" => Some('\u{00aa}'),
        "laquo" => Some('\u{00ab}'),
        "not" => Some('\u{00ac}'),
        "shy" => Some('\u{00ad}'),
        "reg" | "REG" => Some('\u{00ae}'),
        "macr" => Some('\u{00af}'),
        "deg" => Some('\u{00b0}'),
        "plusmn" => Some('\u{00b1}'),
        "sup2" => Some('\u{00b2}'),
        "sup3" => Some('\u{00b3}'),
        "acute" => Some('\u{00b4}'),
        "micro" => Some('\u{00b5}'),
        "para" => Some('\u{00b6}'),
        "middot" => Some('\u{00b7}'),
        "cedil" => Some('\u{00b8}'),
        "sup1" => Some('\u{00b9}'),
        "ordm" => Some('\u{00ba}'),
        "raquo" => Some('\u{00bb}'),
        "frac14" => Some('\u{00bc}'),
        "frac12" => Some('\u{00bd}'),
        "frac34" => Some('\u{00be}'),
        "iquest" => Some('\u{00bf}'),
        "times" => Some('\u{00d7}'),
        "divide" => Some('\u{00f7}'),
        "Alpha" => Some('\u{0391}'),
        "Beta" => Some('\u{0392}'),
        "Gamma" => Some('\u{0393}'),
        "Delta" => Some('\u{0394}'),
        "Omega" => Some('\u{03a9}'),
        "alpha" => Some('\u{03b1}'),
        "beta" => Some('\u{03b2}'),
        "gamma" => Some('\u{03b3}'),
        "delta" => Some('\u{03b4}'),
        "epsilon" => Some('\u{03b5}'),
        "pi" => Some('\u{03c0}'),
        "sigma" => Some('\u{03c3}'),
        "omega" => Some('\u{03c9}'),
        "ensp" => Some('\u{2002}'),
        "emsp" => Some('\u{2003}'),
        "thinsp" => Some('\u{2009}'),
        "zwnj" => Some('\u{200c}'),
        "zwj" => Some('\u{200d}'),
        "lrm" => Some('\u{200e}'),
        "rlm" => Some('\u{200f}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "sbquo" => Some('\u{201a}'),
        "ldquo" => Some('\u{201c}'),
        "rdquo" => Some('\u{201d}'),
        "bdquo" => Some('\u{201e}'),
        "dagger" => Some('\u{2020}'),
        "Dagger" => Some('\u{2021}'),
        "bull" => Some('\u{2022}'),
        "hellip" => Some('\u{2026}'),
        "permil" => Some('\u{2030}'),
        "prime" => Some('\u{2032}'),
        "Prime" => Some('\u{2033}'),
        "lsaquo" => Some('\u{2039}'),
        "rsaquo" => Some('\u{203a}'),
        "oline" => Some('\u{203e}'),
        "frasl" => Some('\u{2044}'),
        "euro" => Some('\u{20ac}'),
        "trade" | "TRADE" => Some('\u{2122}'),
        "larr" => Some('\u{2190}'),
        "uarr" => Some('\u{2191}'),
        "rarr" => Some('\u{2192}'),
        "darr" => Some('\u{2193}'),
        "harr" => Some('\u{2194}'),
        "crarr" => Some('\u{21b5}'),
        "forall" => Some('\u{2200}'),
        "part" => Some('\u{2202}'),
        "exist" => Some('\u{2203}'),
        "empty" => Some('\u{2205}'),
        "nabla" => Some('\u{2207}'),
        "isin" => Some('\u{2208}'),
        "notin" => Some('\u{2209}'),
        "ni" => Some('\u{220b}'),
        "prod" => Some('\u{220f}'),
        "sum" => Some('\u{2211}'),
        "minus" => Some('\u{2212}'),
        "lowast" => Some('\u{2217}'),
        "radic" => Some('\u{221a}'),
        "prop" => Some('\u{221d}'),
        "infin" => Some('\u{221e}'),
        "ang" => Some('\u{2220}'),
        "and" => Some('\u{2227}'),
        "or" => Some('\u{2228}'),
        "cap" => Some('\u{2229}'),
        "cup" => Some('\u{222a}'),
        "int" => Some('\u{222b}'),
        "there4" => Some('\u{2234}'),
        "sim" => Some('\u{223c}'),
        "cong" => Some('\u{2245}'),
        "asymp" => Some('\u{2248}'),
        "ne" => Some('\u{2260}'),
        "equiv" => Some('\u{2261}'),
        "le" => Some('\u{2264}'),
        "ge" => Some('\u{2265}'),
        "sub" => Some('\u{2282}'),
        "sup" => Some('\u{2283}'),
        "nsub" => Some('\u{2284}'),
        "sube" => Some('\u{2286}'),
        "supe" => Some('\u{2287}'),
        "oplus" => Some('\u{2295}'),
        "otimes" => Some('\u{2297}'),
        "perp" => Some('\u{22a5}'),
        "sdot" => Some('\u{22c5}'),
        "spades" => Some('\u{2660}'),
        "clubs" => Some('\u{2663}'),
        "hearts" => Some('\u{2665}'),
        "diams" => Some('\u{2666}'),
        "check" => Some('\u{2713}'),
        "cross" => Some('\u{2717}'),
        "Agrave" => Some('\u{00c0}'),
        "Aacute" => Some('\u{00c1}'),
        "Acirc" => Some('\u{00c2}'),
        "Atilde" => Some('\u{00c3}'),
        "Auml" => Some('\u{00c4}'),
        "Aring" => Some('\u{00c5}'),
        "AElig" => Some('\u{00c6}'),
        "Ccedil" => Some('\u{00c7}'),
        "Egrave" => Some('\u{00c8}'),
        "Eacute" => Some('\u{00c9}'),
        "Ecirc" => Some('\u{00ca}'),
        "Euml" => Some('\u{00cb}'),
        "Iacute" => Some('\u{00cd}'),
        "Ntilde" => Some('\u{00d1}'),
        "Oacute" => Some('\u{00d3}'),
        "Ouml" => Some('\u{00d6}'),
        "Uacute" => Some('\u{00da}'),
        "Uuml" => Some('\u{00dc}'),
        "szlig" => Some('\u{00df}'),
        "agrave" => Some('\u{00e0}'),
        "aacute" => Some('\u{00e1}'),
        "acirc" => Some('\u{00e2}'),
        "atilde" => Some('\u{00e3}'),
        "auml" => Some('\u{00e4}'),
        "aring" => Some('\u{00e5}'),
        "aelig" => Some('\u{00e6}'),
        "ccedil" => Some('\u{00e7}'),
        "egrave" => Some('\u{00e8}'),
        "eacute" => Some('\u{00e9}'),
        "ecirc" => Some('\u{00ea}'),
        "euml" => Some('\u{00eb}'),
        "igrave" => Some('\u{00ec}'),
        "iacute" => Some('\u{00ed}'),
        "icirc" => Some('\u{00ee}'),
        "iuml" => Some('\u{00ef}'),
        "ntilde" => Some('\u{00f1}'),
        "ograve" => Some('\u{00f2}'),
        "oacute" => Some('\u{00f3}'),
        "ocirc" => Some('\u{00f4}'),
        "otilde" => Some('\u{00f5}'),
        "ouml" => Some('\u{00f6}'),
        "oslash" => Some('\u{00f8}'),
        "ugrave" => Some('\u{00f9}'),
        "uacute" => Some('\u{00fa}'),
        "ucirc" => Some('\u{00fb}'),
        "uuml" => Some('\u{00fc}'),
        "yuml" => Some('\u{00ff}'),
        "theta" => Some('\u{03b8}'),
        "lambda" => Some('\u{03bb}'),
        "mu" => Some('\u{03bc}'),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let hex = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"));
    let code = if let Some(digits) = hex {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok()?
    } else {
        let digits = entity.strip_prefix('#')?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.parse::<u32>().ok()?
    };
    // HTML5 numeric character reference Windows-1252 replacement table (0x80..=0x9F)
    let remapped = match code {
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        other => other,
    };
    if remapped == 0 {
        return Some('\u{FFFD}');
    }
    Some(char::from_u32(remapped).unwrap_or('\u{FFFD}'))
}

fn is_structural_whitespace_dropper(tag: &str) -> bool {
    matches!(
        tag,
        "html" | "head" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "colgroup" | "select"
    )
}

fn is_inline_or_phrasing_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "abbr"
            | "b"
            | "bdi"
            | "bdo"
            | "br"
            | "button"
            | "cite"
            | "code"
            | "data"
            | "del"
            | "dfn"
            | "em"
            | "i"
            | "img"
            | "input"
            | "ins"
            | "kbd"
            | "label"
            | "mark"
            | "q"
            | "ruby"
            | "s"
            | "samp"
            | "select"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
            | "svg"
            | "textarea"
            | "time"
            | "u"
            | "var"
            | "wbr"
    )
}

fn should_keep_inline_space(doc: &Document, parent: NodeId, raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    if let Some(ptag) = doc.get(parent).data().tag()
        && is_structural_whitespace_dropper(ptag)
    {
        return false;
    }
    if let Some(prev) = doc.children(parent).last().copied() {
        match doc.get(prev).data() {
            NodeData::Text(t) => !t.trim().is_empty(),
            NodeData::Element { tag, .. } => is_inline_or_phrasing_tag(tag),
            NodeData::Document => false,
        }
    } else {
        false
    }
}

/// Collapse whitespace, or keep a single inline separator space between phrasing elements.
fn text_for_parent(doc: &Document, parent: NodeId, raw: &str) -> Option<String> {
    if preserves_whitespace(doc, parent) {
        preserve_text(raw)
    } else if let Some(norm) = normalize_text(raw) {
        Some(norm)
    } else if should_keep_inline_space(doc, parent, raw) {
        Some(" ".to_string())
    } else {
        None
    }
}

fn preserves_whitespace(doc: &Document, mut id: NodeId) -> bool {
    loop {
        if matches!(doc.get(id).data().tag(), Some("pre" | "textarea")) {
            return true;
        }
        match doc.parent_element(id) {
            Some(parent) => id = parent,
            None => return false,
        }
    }
}

fn preserve_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn normalize_text(raw: &str) -> Option<String> {
    if !raw.chars().any(|ch| !ch.is_whitespace()) {
        return None;
    }
    let lead = raw.chars().next().is_some_and(|ch| ch.is_whitespace());
    let trail = raw.chars().next_back().is_some_and(|ch| ch.is_whitespace());
    let mut collapsed = String::new();
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        pending_space = false;
        collapsed.push(ch);
    }
    let mut out = String::new();
    if lead {
        out.push(' ');
    }
    out.push_str(&collapsed);
    if trail {
        out.push(' ');
    }
    Some(out)
}
