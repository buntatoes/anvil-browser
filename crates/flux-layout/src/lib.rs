//! Block, inline, and flex layout with coordinate hit testing.

use std::collections::HashMap;
use std::sync::OnceLock;

pub use flux_dom::NodeId;
use flux_dom::{Document, NodeData};
pub use flux_image::Image;
pub use flux_style::{AlignItems, Color, Edge, FlexDirection, JustifyContent};
use flux_style::{ComputedStyle, Display, Size, StyleMap};
use flux_ttf::TtfFont;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub node: NodeId,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub font_size: f32,
    pub color: Color,
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutBox {
    pub node: NodeId,
    pub rect: Rect,
    pub background: Option<Color>,
    pub border: Edge,
    pub border_color: Color,
    pub border_radius: f32,
    pub image: Option<Image>,
    pub text: Vec<TextRun>,
    pub children: Vec<LayoutBox>,
}

#[derive(Clone, Debug)]
struct Segment {
    node: NodeId,
    text: String,
    font_size: f32,
    color: Color,
    underline: bool,
}

#[derive(Clone, Debug)]
struct Token {
    node: NodeId,
    text: String,
    font_size: f32,
    color: Color,
    underline: bool,
}

pub fn layout_document(doc: &Document, styles: &StyleMap, viewport_width: f32) -> LayoutBox {
    let empty = HashMap::new();
    layout_document_with_images(doc, styles, viewport_width, &empty)
}

pub fn layout_document_with_images(
    doc: &Document,
    styles: &StyleMap,
    viewport_width: f32,
    images: &HashMap<NodeId, Image>,
) -> LayoutBox {
    layout_node(
        doc,
        styles,
        images,
        doc.html(),
        0.0,
        0.0,
        viewport_width.max(0.0),
        None,
    )
}

pub fn hit_test(tree: &LayoutBox, x: f32, y: f32) -> Option<NodeId> {
    for child in tree.children.iter().rev() {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }
    for run in tree.text.iter().rev() {
        let rect = Rect {
            x: run.x,
            y: run.y,
            width: run.width,
            height: line_height(run.font_size),
        };
        if rect.contains(x, y) {
            return Some(run.node);
        }
    }
    tree.rect.contains(x, y).then_some(tree.node)
}

#[allow(clippy::too_many_arguments)]
fn layout_node(
    doc: &Document,
    styles: &StyleMap,
    images: &HashMap<NodeId, Image>,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
    list_marker: Option<String>,
) -> LayoutBox {
    let style = *styles.get(id);
    if style.display == Display::Flex {
        return layout_flex(doc, styles, images, id, x, y, containing_width);
    }
    layout_block(doc, styles, images, id, x, y, containing_width, list_marker)
}

fn resolve_border_width(style: &ComputedStyle, containing_width: f32) -> f32 {
    let padding_h = style.padding.left + style.padding.right;
    let border_h = style.border.left + style.border.right;
    let mut border_width = match style.width {
        Size::Px(width) => (width + padding_h + border_h).max(0.0),
        Size::Auto => (containing_width - style.margin.left - style.margin.right).max(0.0),
    };
    if let Size::Px(max_w) = style.max_width {
        let max_border_w = (max_w + padding_h + border_h).max(0.0);
        border_width = border_width.min(max_border_w);
    }
    border_width
}

#[allow(clippy::too_many_arguments)]
fn layout_block(
    doc: &Document,
    styles: &StyleMap,
    images: &HashMap<NodeId, Image>,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
    list_marker: Option<String>,
) -> LayoutBox {
    let style = *styles.get(id);
    let padding = style.padding;
    let margin = style.margin;
    let border = style.border;

    let mut border_width = resolve_border_width(&style, containing_width);
    let data = doc.get(id).data();
    let is_img = data.tag() == Some("img");
    let decoded_image = if is_img {
        images.get(&id).cloned()
    } else {
        None
    };
    if is_img && matches!(style.width, Size::Auto) {
        let attr_w = data
            .attr("width")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .or_else(|| decoded_image.as_ref().map(|img| img.width as f32))
            .unwrap_or(120.0);
        border_width = attr_w + padding.left + padding.right + border.left + border.right;
    }

    let content_width =
        (border_width - padding.left - padding.right - border.left - border.right).max(0.0);
    let border_x = x + margin.left;
    let border_y = y + margin.top;
    let content_x = border_x + border.left + padding.left;
    let mut cursor_y = border_y + border.top + padding.top;
    let mut children = Vec::new();
    let mut text = Vec::new();
    let mut inline_ids = Vec::new();
    let mut pending_marker = list_marker;

    if is_img && decoded_image.is_none() {
        if let Some(alt) = data.attr("alt") {
            if !alt.trim().is_empty() {
                let segments = vec![Segment {
                    node: id,
                    text: format!("[{alt}]"),
                    font_size: style.font_size,
                    color: style.color,
                    underline: false,
                }];
                let (runs, h) = place_lines(&segments, content_x, cursor_y, content_width);
                text.extend(runs);
                cursor_y += h;
            }
        }
    }

    let parent_tag = data.tag().unwrap_or("");
    let mut li_index = 0_usize;

    for child in doc.children(id) {
        if is_skipped(doc, styles, *child) {
            continue;
        }
        if is_block_box(doc, styles, *child) {
            place_inlines(
                doc,
                styles,
                id,
                &mut pending_marker,
                &mut inline_ids,
                content_x,
                &mut cursor_y,
                content_width,
                &mut text,
            );
            let child_marker = if doc.get(*child).data().tag() == Some("li") {
                li_index += 1;
                if parent_tag == "ol" {
                    Some(format!("{li_index}. "))
                } else {
                    Some("- ".to_string())
                }
            } else {
                None
            };
            let child_box = layout_node(
                doc,
                styles,
                images,
                *child,
                content_x,
                cursor_y,
                content_width,
                child_marker,
            );
            let child_style = styles.get(*child);
            cursor_y += child_style.margin.top + child_box.rect.height + child_style.margin.bottom;
            children.push(child_box);
        } else {
            inline_ids.push(*child);
        }
    }
    place_inlines(
        doc,
        styles,
        id,
        &mut pending_marker,
        &mut inline_ids,
        content_x,
        &mut cursor_y,
        content_width,
        &mut text,
    );

    let flow_height = cursor_y - (border_y + border.top + padding.top);
    let content_height = match style.height {
        Size::Px(height) => height.max(flow_height),
        Size::Auto if is_img => data
            .attr("height")
            .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            .or_else(|| decoded_image.as_ref().map(|img| img.height as f32))
            .unwrap_or(36.0)
            .max(flow_height),
        Size::Auto => flow_height,
    };
    let border_height = content_height + padding.top + padding.bottom + border.top + border.bottom;
    LayoutBox {
        node: id,
        rect: Rect {
            x: border_x,
            y: border_y,
            width: border_width,
            height: border_height,
        },
        background: style.background,
        border,
        border_color: style.border_color,
        border_radius: style.border_radius,
        image: decoded_image,
        text,
        children,
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_flex(
    doc: &Document,
    styles: &StyleMap,
    images: &HashMap<NodeId, Image>,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
) -> LayoutBox {
    let style = *styles.get(id);
    let padding = style.padding;
    let margin = style.margin;
    let border = style.border;
    let border_width = resolve_border_width(&style, containing_width);
    let content_width =
        (border_width - padding.left - padding.right - border.left - border.right).max(0.0);
    let border_x = x + margin.left;
    let border_y = y + margin.top;
    let content_x = border_x + border.left + padding.left;
    let content_y = border_y + border.top + padding.top;

    let visible_children: Vec<NodeId> = doc
        .children(id)
        .iter()
        .copied()
        .filter(|child| !is_skipped(doc, styles, *child))
        .collect();

    if style.flex_direction == FlexDirection::Column {
        let mut cursor_y = content_y;
        let mut children = Vec::new();
        for (idx, child) in visible_children.iter().copied().enumerate() {
            if idx > 0 {
                cursor_y += style.gap;
            }
            let child_box = layout_node(
                doc,
                styles,
                images,
                child,
                content_x,
                cursor_y,
                content_width,
                None,
            );
            let child_style = styles.get(child);
            cursor_y += child_style.margin.top + child_box.rect.height + child_style.margin.bottom;
            children.push(child_box);
        }
        let flow_height = (cursor_y - content_y).max(0.0);
        let content_height = match style.height {
            Size::Px(h) => h.max(flow_height),
            Size::Auto => flow_height,
        };
        return LayoutBox {
            node: id,
            rect: Rect {
                x: border_x,
                y: border_y,
                width: border_width,
                height: content_height + padding.top + padding.bottom + border.top + border.bottom,
            },
            background: style.background,
            border,
            border_color: style.border_color,
            border_radius: style.border_radius,
            image: None,
            text: Vec::new(),
            children,
        };
    }

    // Row flex layout
    let mut children = Vec::new();
    let mut total_item_width = 0.0_f32;
    let mut max_item_height = 0.0_f32;

    for child in &visible_children {
        let child_style = styles.get(*child);
        let basis_width = match child_style.width {
            Size::Px(w) => {
                w + child_style.padding.left
                    + child_style.padding.right
                    + child_style.border.left
                    + child_style.border.right
                    + child_style.margin.left
                    + child_style.margin.right
            }
            Size::Auto => {
                let text_w = measure(&doc.text_content(*child), child_style.font_size);
                (text_w
                    + child_style.padding.left
                    + child_style.padding.right
                    + child_style.border.left
                    + child_style.border.right
                    + child_style.margin.left
                    + child_style.margin.right)
                    .min(content_width)
            }
        };
        let child_box = layout_node(doc, styles, images, *child, 0.0, 0.0, basis_width, None);
        let outer_w = child_style.margin.left + child_box.rect.width + child_style.margin.right;
        let outer_h = child_style.margin.top + child_box.rect.height + child_style.margin.bottom;
        total_item_width += outer_w;
        max_item_height = max_item_height.max(outer_h);
        children.push(child_box);
    }

    let count = children.len();
    let total_gaps = if count > 1 {
        style.gap * ((count - 1) as f32)
    } else {
        0.0
    };
    let free_space = (content_width - total_item_width - total_gaps).max(0.0);
    let (mut cursor_x, step_gap) = match style.justify_content {
        JustifyContent::Start => (content_x, style.gap),
        JustifyContent::Center => (content_x + free_space * 0.5, style.gap),
        JustifyContent::End => (content_x + free_space, style.gap),
        JustifyContent::SpaceBetween if count > 1 => {
            (content_x, style.gap + free_space / ((count - 1) as f32))
        }
        JustifyContent::SpaceBetween => (content_x, style.gap),
    };

    let row_height = match style.height {
        Size::Px(h) => h.max(max_item_height),
        Size::Auto => max_item_height,
    };

    for child_box in &mut children {
        let child_style = styles.get(child_box.node);
        let outer_h = child_style.margin.top + child_box.rect.height + child_style.margin.bottom;
        let y_offset = match style.align_items {
            AlignItems::Start | AlignItems::Stretch => 0.0,
            AlignItems::Center => ((row_height - outer_h) * 0.5).max(0.0),
            AlignItems::End => (row_height - outer_h).max(0.0),
        };
        let dx = (cursor_x + child_style.margin.left) - child_box.rect.x;
        let dy = (content_y + y_offset + child_style.margin.top) - child_box.rect.y;
        offset_box(child_box, dx, dy);
        if style.align_items == AlignItems::Stretch && matches!(child_style.height, Size::Auto) {
            child_box.rect.height =
                (row_height - child_style.margin.top - child_style.margin.bottom)
                    .max(child_box.rect.height);
        }
        cursor_x +=
            child_style.margin.left + child_box.rect.width + child_style.margin.right + step_gap;
    }

    LayoutBox {
        node: id,
        rect: Rect {
            x: border_x,
            y: border_y,
            width: border_width,
            height: row_height + padding.top + padding.bottom + border.top + border.bottom,
        },
        background: style.background,
        border,
        border_color: style.border_color,
        border_radius: style.border_radius,
        image: None,
        text: Vec::new(),
        children,
    }
}

fn offset_box(tree: &mut LayoutBox, dx: f32, dy: f32) {
    tree.rect.x += dx;
    tree.rect.y += dy;
    for run in &mut tree.text {
        run.x += dx;
        run.y += dy;
    }
    for child in &mut tree.children {
        offset_box(child, dx, dy);
    }
}

#[allow(clippy::too_many_arguments)]
fn place_inlines(
    doc: &Document,
    styles: &StyleMap,
    block_id: NodeId,
    pending_marker: &mut Option<String>,
    inline_ids: &mut Vec<NodeId>,
    content_x: f32,
    cursor_y: &mut f32,
    content_width: f32,
    text: &mut Vec<TextRun>,
) {
    if inline_ids.is_empty() && pending_marker.is_none() {
        return;
    }
    let mut segments = Vec::new();
    if let Some(marker) = pending_marker.take() {
        let block_style = styles.get(block_id);
        segments.push(Segment {
            node: block_id,
            text: marker,
            font_size: block_style.font_size,
            color: block_style.color,
            underline: false,
        });
    }
    for id in inline_ids.drain(..) {
        gather(doc, styles, id, styles.get(id), &mut segments);
    }
    let (runs, height) = place_lines(&segments, content_x, *cursor_y, content_width);
    text.extend(runs);
    *cursor_y += height;
}

fn gather(
    doc: &Document,
    styles: &StyleMap,
    id: NodeId,
    style: &ComputedStyle,
    out: &mut Vec<Segment>,
) {
    match doc.get(id).data() {
        NodeData::Text(text) => out.push(Segment {
            node: id,
            text: text.clone(),
            font_size: style.font_size,
            color: style.color,
            underline: style.underline,
        }),
        NodeData::Element { tag, .. } => {
            if styles.get(id).display == Display::None {
                return;
            }
            if tag == "br" {
                out.push(Segment {
                    node: id,
                    text: "\n".to_string(),
                    font_size: style.font_size,
                    color: style.color,
                    underline: false,
                });
                return;
            }
            let style = *styles.get(id);
            for child in doc.children(id) {
                gather(doc, styles, *child, &style, out);
            }
        }
        NodeData::Document => {}
    }
}

fn is_skipped(doc: &Document, styles: &StyleMap, id: NodeId) -> bool {
    matches!(doc.get(id).data(), NodeData::Element { .. })
        && styles.get(id).display == Display::None
}

fn is_block_box(doc: &Document, styles: &StyleMap, id: NodeId) -> bool {
    match doc.get(id).data() {
        NodeData::Element { tag, .. } => {
            tag == "img"
                || matches!(styles.get(id).display, Display::Block | Display::Flex)
                || contains_block(doc, styles, id)
        }
        _ => false,
    }
}

fn contains_block(doc: &Document, styles: &StyleMap, id: NodeId) -> bool {
    for child in doc.children(id) {
        if let NodeData::Element { tag, .. } = doc.get(*child).data() {
            if tag == "img" || matches!(styles.get(*child).display, Display::Block | Display::Flex)
            {
                return true;
            }
        }
        if contains_block(doc, styles, *child) {
            return true;
        }
    }
    false
}

fn place_lines(
    segments: &[Segment],
    origin_x: f32,
    origin_y: f32,
    max_width: f32,
) -> (Vec<TextRun>, f32) {
    let tokens = explode_wide(tokenize(segments), max_width);
    let mut lines: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    let mut line_width = 0.0;

    for token in tokens {
        if token.text == "\n" {
            trim_trailing_space(&mut current);
            lines.push(std::mem::take(&mut current));
            line_width = 0.0;
            continue;
        }
        if token.text == " " && (current.is_empty() || line_width == 0.0) {
            continue;
        }
        let width = measure(&token.text, token.font_size);
        if line_width > 0.0 && line_width + width > max_width {
            trim_trailing_space(&mut current);
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            line_width = 0.0;
            if token.text == " " {
                continue;
            }
        }
        line_width += width;
        current.push(token);
    }
    trim_trailing_space(&mut current);
    if !current.is_empty() {
        lines.push(current);
    }

    let mut runs = Vec::new();
    let mut y = origin_y;
    for line in lines {
        if line.is_empty() {
            y += 16.0;
            continue;
        }
        let height = line
            .iter()
            .map(|token| line_height(token.font_size))
            .fold(0.0_f32, f32::max);
        let mut x = origin_x;
        let mut combined: Option<TextRun> = None;
        for token in line {
            let same = combined.as_ref().is_some_and(|run| {
                run.node == token.node
                    && run.font_size == token.font_size
                    && run.color == token.color
                    && run.underline == token.underline
            });
            if same {
                let run = combined.as_mut().expect("checked");
                run.text.push_str(&token.text);
                run.width = measure(&run.text, run.font_size);
            } else {
                if let Some(run) = combined.take() {
                    x += run.width;
                    runs.push(run);
                }
                let width = measure(&token.text, token.font_size);
                combined = Some(TextRun {
                    node: token.node,
                    text: token.text,
                    x,
                    y,
                    width,
                    font_size: token.font_size,
                    color: token.color,
                    underline: token.underline,
                });
            }
        }
        if let Some(run) = combined {
            runs.push(run);
        }
        y += height;
    }
    (runs, y - origin_y)
}

fn tokenize(segments: &[Segment]) -> Vec<Token> {
    let mut tokens = Vec::new();
    for segment in segments {
        if segment.text == "\n" {
            tokens.push(Token {
                node: segment.node,
                text: "\n".to_string(),
                font_size: segment.font_size,
                color: segment.color,
                underline: segment.underline,
            });
            continue;
        }
        let mut word = String::new();
        for ch in segment.text.chars() {
            if ch == ' ' {
                if !word.is_empty() {
                    tokens.push(Token {
                        node: segment.node,
                        text: std::mem::take(&mut word),
                        font_size: segment.font_size,
                        color: segment.color,
                        underline: segment.underline,
                    });
                }
                tokens.push(Token {
                    node: segment.node,
                    text: " ".to_string(),
                    font_size: segment.font_size,
                    color: segment.color,
                    underline: segment.underline,
                });
            } else {
                word.push(ch);
            }
        }
        if !word.is_empty() {
            tokens.push(Token {
                node: segment.node,
                text: word,
                font_size: segment.font_size,
                color: segment.color,
                underline: segment.underline,
            });
        }
    }
    tokens
}

fn explode_wide(tokens: Vec<Token>, max_width: f32) -> Vec<Token> {
    if max_width <= 0.0 {
        return tokens;
    }
    let mut out = Vec::new();
    for token in tokens {
        if token.text == " "
            || token.text == "\n"
            || measure(&token.text, token.font_size) <= max_width
        {
            out.push(token);
            continue;
        }
        for ch in token.text.chars() {
            out.push(Token {
                node: token.node,
                text: ch.to_string(),
                font_size: token.font_size,
                color: token.color,
                underline: token.underline,
            });
        }
    }
    out
}

fn trim_trailing_space(line: &mut Vec<Token>) {
    while line.last().is_some_and(|token| token.text == " ") {
        line.pop();
    }
}

pub fn measure(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * 0.5
}

pub fn measure_proportional(text: &str, font_size: f32) -> f32 {
    static SYSTEM_FONT: OnceLock<Option<TtfFont>> = OnceLock::new();
    let font = SYSTEM_FONT.get_or_init(|| TtfFont::load_system_font(false, false));
    if let Some(ttf) = font {
        ttf.measure_text(text, font_size)
    } else {
        flux_ttf::measure_proportional_fallback(text, font_size)
    }
}

pub fn line_height(font_size: f32) -> f32 {
    font_size * 1.25
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_dom::parse_html;
    use flux_style::{compute_styles, parse_css};

    fn render(html: &str, width: f32) -> (Document, LayoutBox) {
        let doc = parse_html(html);
        let styles = compute_styles(&doc, &parse_css(&doc.embedded_css()));
        let tree = layout_document(&doc, &styles, width);
        (doc, tree)
    }

    fn collect_tag<'a>(
        doc: &Document,
        tree: &'a LayoutBox,
        tag: &str,
        out: &mut Vec<&'a LayoutBox>,
    ) {
        if doc.get(tree.node).data().tag() == Some(tag) {
            out.push(tree);
        }
        for child in &tree.children {
            collect_tag(doc, child, tag, out);
        }
    }

    #[test]
    fn wraps_a_line_on_the_width() {
        let segments = vec![Segment {
            node: NodeId::from_raw(1),
            text: "hello hello".to_string(),
            font_size: 10.0,
            color: Color::BLACK,
            underline: false,
        }];
        let (runs, height) = place_lines(&segments, 0.0, 0.0, 30.0);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[1].text, "hello");
        assert_eq!(runs[1].y, 12.5);
        assert_eq!(height, 25.0);
    }

    #[test]
    fn hits_an_inline_text_node_before_its_block() {
        let (doc, tree) = render(
            "<style>html, body, p { margin: 0; padding: 0; font-size: 10px; }</style>
             <p><a href=\"next.html\">go</a></p>",
            100.0,
        );
        let hit = hit_test(&tree, 2.0, 2.0).unwrap();
        assert_eq!(doc.get(hit).data().text(), Some("go"));
        let parent = doc.get(hit).parent().unwrap();
        assert_eq!(doc.get(parent).data().tag(), Some("a"));
    }

    #[test]
    fn stacks_blocks_and_pads_text() {
        let (doc, tree) = render(
            "<style>
                html, body { margin: 0; padding: 0; }
                div { margin: 0; padding: 10px; font-size: 10px; background: #ff0000; }
             </style>
             <div>a</div><div>b</div>",
            100.0,
        );
        let mut divs = Vec::new();
        collect_tag(&doc, &tree, "div", &mut divs);
        assert_eq!(divs.len(), 2);
        assert_eq!(divs[0].rect.y, 0.0);
        assert_eq!(divs[1].rect.y, 32.5);
        assert_eq!(divs[0].text[0].x, 10.0);
        assert_eq!(divs[0].text[0].y, 10.0);
        assert_eq!(divs[0].background, Color::parse("#ff0000"));
    }

    #[test]
    fn lays_out_flex_row_children_with_gap() {
        let (doc, tree) = render(
            "<style>
                html, body { margin: 0; padding: 0; }
                .row { display: flex; gap: 10px; }
                .item { width: 40px; height: 20px; }
             </style>
             <div class=\"row\"><div class=\"item\">A</div><div class=\"item\">B</div></div>",
            200.0,
        );
        let mut divs = Vec::new();
        collect_tag(&doc, &tree, "div", &mut divs);
        assert_eq!(divs.len(), 3);
        assert_eq!(divs[1].rect.x, 0.0);
        assert_eq!(divs[2].rect.x, 50.0);
    }

    #[test]
    fn sizes_decoded_image_and_measures_proportional_text() {
        let doc = parse_html("<img id=\"logo\" src=\"logo.png\" alt=\"Logo\">");
        let styles = compute_styles(&doc, &parse_css("html, body { margin: 0; padding: 0; }"));
        let logo_id = doc.get_element_by_id("logo").unwrap();
        let mut images = HashMap::new();
        images.insert(logo_id, Image::new(64, 48));

        let tree = layout_document_with_images(&doc, &styles, 300.0, &images);
        let mut imgs = Vec::new();
        collect_tag(&doc, &tree, "img", &mut imgs);
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].rect.width, 64.0);
        assert_eq!(imgs[0].rect.height, 48.0);
        assert!(imgs[0].image.is_some());

        assert!(measure_proportional("MMMM", 16.0) > measure_proportional("iiii", 16.0));
    }
}
