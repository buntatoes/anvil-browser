use crate::{Document, NodeId};

/// Maximum DOM tree nesting depth enforced by the HTML parser to prevent stack overflows.
pub const MAX_DOM_DEPTH: usize = 256;

/// Maximum total DOM nodes allocated per document to prevent memory exhaustion.
pub const MAX_DOM_NODES: usize = 50_000;

/// Maximum attributes accepted per HTML start tag.
pub const MAX_ATTRS_PER_ELEMENT: usize = 128;

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const HEAD_TAGS: &[&str] = &["base", "link", "meta", "style", "title"];

/// Parse a subset of HTML into a document.
///
/// `script` and `style` keep their raw text, including a `<` that would
/// otherwise look like a tag. Void tags do not take children.
pub fn parse_html(input: &str) -> Document {
    let mut doc = Document::new();
    let mut stack = vec![doc.body()];
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if doc.node_count() >= MAX_DOM_NODES {
            break;
        }
        if bytes[index] == b'<' && is_markup(bytes, index) {
            if input[index..].starts_with("<!--") {
                index = skip_until(input, index + 4, "-->").unwrap_or(input.len());
                continue;
            }
            if input[index..].starts_with("<!") || input[index..].starts_with("<?") {
                index = skip_char(input, index + 2, '>').unwrap_or(input.len());
                continue;
            }
            if bytes.get(index + 1) == Some(&b'/') {
                let (name, next) = read_end_tag(input, index);
                index = next;
                close_tag(&doc, &mut stack, &name);
                continue;
            }

            let (name, attrs, next) = read_start_tag(input, index);
            index = next;
            if name.is_empty() {
                continue;
            }
            if name == "html" {
                continue;
            }
            if name == "head" {
                stack.clear();
                stack.push(doc.head());
                continue;
            }
            if name == "body" {
                stack.clear();
                stack.push(doc.body());
                continue;
            }

            auto_close_for_start_tag(&doc, &mut stack, &name);

            let parent = placement_parent(&doc, &stack, &name);
            let id = doc.create_element(&name, attrs);
            doc.append(parent, id).expect("new node has no children");

            if name == "script" || name == "style" {
                if let Some((text, next)) = read_raw_text(input, index, &name) {
                    if let Some(text) = normalize_text(&decode_entities(&text)) {
                        if doc.node_count() < MAX_DOM_NODES {
                            let text_id = doc.create_text(&text);
                            doc.append(id, text_id).expect("new text node");
                        }
                    }
                    index = next;
                }
                continue;
            }

            if !is_void(&name) && stack.len() < MAX_DOM_DEPTH {
                stack.push(id);
            }
            continue;
        }

        let start = index;
        while index < bytes.len() && bytes[index] != b'<' {
            index += 1;
        }
        if let Some(text) = normalize_text(&decode_entities(&input[start..index])) {
            if doc.node_count() < MAX_DOM_NODES {
                let parent = *stack.last().unwrap_or(&doc.body());
                let text_id = doc.create_text(&text);
                doc.append(parent, text_id).expect("new text node");
            }
        }
    }

    doc
}

fn is_markup(bytes: &[u8], index: usize) -> bool {
    matches!(
        bytes.get(index + 1).copied(),
        Some(b'/' | b'!' | b'?' | b'a'..=b'z' | b'A'..=b'Z')
    )
}

fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag)
}

fn auto_close_for_start_tag(doc: &Document, stack: &mut Vec<NodeId>, new_tag: &str) {
    match new_tag {
        "td" | "th" => {
            if let Some(&top) = stack.last() {
                if matches!(doc.get(top).data().tag(), Some("td" | "th")) {
                    stack.pop();
                }
            }
        }
        "tr" => {
            while stack.len() > 1 {
                let top = *stack.last().unwrap();
                match doc.get(top).data().tag() {
                    Some("td" | "th" | "tr") => {
                        stack.pop();
                    }
                    _ => break,
                }
            }
        }
        "thead" | "tbody" | "tfoot" => {
            while stack.len() > 1 {
                let top = *stack.last().unwrap();
                match doc.get(top).data().tag() {
                    Some("td" | "th" | "tr" | "thead" | "tbody" | "tfoot") => {
                        stack.pop();
                    }
                    _ => break,
                }
            }
        }
        "option" => {
            if let Some(&top) = stack.last() {
                if doc.get(top).data().tag() == Some("option") {
                    stack.pop();
                }
            }
        }
        _ => {}
    }
}

fn placement_parent(doc: &Document, stack: &[NodeId], tag: &str) -> NodeId {
    if HEAD_TAGS.contains(&tag) {
        if let Some(top) = stack.last() {
            if *top == doc.body() || doc.get(*top).data().tag() == Some("html") {
                return doc.head();
            }
        }
    }
    stack.last().copied().unwrap_or_else(|| doc.body())
}

fn close_tag(doc: &Document, stack: &mut Vec<NodeId>, name: &str) {
    if name.is_empty() {
        return;
    }
    if name == "head" || name == "body" || name == "html" {
        stack.clear();
        stack.push(doc.body());
        return;
    }
    while stack.len() > 1 {
        let top = stack.pop().expect("stack length was checked");
        if doc.get(top).data().tag() == Some(name) {
            break;
        }
    }
    if stack.is_empty() {
        stack.push(doc.body());
    }
}

fn read_start_tag(input: &str, start: usize) -> (String, Vec<(String, String)>, usize) {
    let bytes = input.as_bytes();
    let mut index = start + 1;
    let name = read_name(input, &mut index);
    let mut attrs = Vec::new();
    loop {
        index = skip_ascii_space(bytes, index);
        if index >= bytes.len() || bytes[index] == b'>' {
            break;
        }
        if bytes[index] == b'/' {
            index += 1;
            continue;
        }
        let attr_name = read_name(input, &mut index);
        if attr_name.is_empty() {
            index += 1;
            continue;
        }
        index = skip_ascii_space(bytes, index);
        let value = if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            index = skip_ascii_space(bytes, index);
            read_attr_value(input, &mut index)
        } else {
            String::new()
        };
        if attrs.len() < MAX_ATTRS_PER_ELEMENT {
            attrs.push((attr_name, value));
        }
    }
    if index < bytes.len() && bytes[index] == b'>' {
        index += 1;
    }
    (name, attrs, index)
}

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

fn read_name(input: &str, index: &mut usize) -> String {
    let bytes = input.as_bytes();
    let start = *index;
    while *index < bytes.len() && is_name_byte(bytes[*index]) {
        *index += 1;
    }
    input[start..*index].to_ascii_lowercase()
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b':'
}

fn read_attr_value(input: &str, index: &mut usize) -> String {
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
        let value = decode_entities(&input[start..*index]);
        if *index < bytes.len() {
            *index += 1;
        }
        return value;
    }
    let start = *index;
    while *index < bytes.len()
        && !matches!(bytes[*index], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')
    {
        *index += 1;
    }
    decode_entities(&input[start..*index])
}

fn read_raw_text(input: &str, start: usize, tag: &str) -> Option<(String, usize)> {
    let lower = input[start..].to_ascii_lowercase();
    let marker = format!("</{tag}");
    let found = lower.find(&marker)?;
    let text = input[start..start + found].to_string();
    let mut end = start + found + marker.len();
    let bytes = input.as_bytes();
    while end < bytes.len() && bytes[end] != b'>' {
        end += 1;
    }
    if end < bytes.len() {
        end += 1;
    }
    Some((text, end))
}

fn skip_until(input: &str, start: usize, marker: &str) -> Option<usize> {
    input[start..]
        .find(marker)
        .map(|offset| start + offset + marker.len())
}

fn skip_char(input: &str, start: usize, marker: char) -> Option<usize> {
    input[start..]
        .find(marker)
        .map(|offset| start + offset + marker.len_utf8())
}

fn skip_ascii_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn decode_entities(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let entity = &rest[1..end];
        let replacement = match entity {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => "\u{00a0}",
            _ => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        out.push_str(replacement);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Collapse whitespace, but keep one leading or trailing space when the
/// source had it. A node that is only whitespace is dropped.
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
