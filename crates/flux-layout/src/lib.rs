//! Block and inline layout.
//!
//! Vertical margins do not collapse. A character's width is half the font
//! size until a font shaper is wired in. Line height is 1.25 times the font size.

pub use flux_dom::NodeId;
use flux_dom::{Document, NodeData};
pub use flux_style::Color;
use flux_style::{ComputedStyle, Display, Size, StyleMap};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutBox {
    pub node: NodeId,
    pub rect: Rect,
    pub background: Option<Color>,
    pub text: Vec<TextRun>,
    pub children: Vec<LayoutBox>,
}

#[derive(Clone, Debug)]
struct Segment {
    text: String,
    font_size: f32,
    color: Color,
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    font_size: f32,
    color: Color,
}

pub fn layout_document(doc: &Document, styles: &StyleMap, viewport_width: f32) -> LayoutBox {
    layout_block(doc, styles, doc.html(), 0.0, 0.0, viewport_width.max(0.0))
}

fn layout_block(
    doc: &Document,
    styles: &StyleMap,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
) -> LayoutBox {
    let style = *styles.get(id);
    let padding = style.padding;
    let margin = style.margin;
    let border_width = match style.width {
        Size::Px(width) => (width + padding.left + padding.right).max(0.0),
        Size::Auto => (containing_width - margin.left - margin.right).max(0.0),
    };
    let content_width = (border_width - padding.left - padding.right).max(0.0);
    let border_x = x + margin.left;
    let border_y = y + margin.top;
    let content_x = border_x + padding.left;
    let mut cursor_y = border_y + padding.top;
    let mut children = Vec::new();
    let mut text = Vec::new();
    let mut inline_ids = Vec::new();

    for child in doc.children(id) {
        if is_skipped(doc, styles, *child) {
            continue;
        }
        if is_block_box(doc, styles, *child) {
            place_inlines(
                doc,
                styles,
                &mut inline_ids,
                content_x,
                &mut cursor_y,
                content_width,
                &mut text,
            );
            let child_box = layout_block(doc, styles, *child, content_x, cursor_y, content_width);
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
        &mut inline_ids,
        content_x,
        &mut cursor_y,
        content_width,
        &mut text,
    );

    let flow_height = cursor_y - (border_y + padding.top);
    let content_height = match style.height {
        Size::Px(height) => height.max(flow_height),
        Size::Auto => flow_height,
    };
    let border_height = content_height + padding.top + padding.bottom;
    LayoutBox {
        node: id,
        rect: Rect {
            x: border_x,
            y: border_y,
            width: border_width,
            height: border_height,
        },
        background: style.background,
        text,
        children,
    }
}

fn place_inlines(
    doc: &Document,
    styles: &StyleMap,
    inline_ids: &mut Vec<NodeId>,
    content_x: f32,
    cursor_y: &mut f32,
    content_width: f32,
    text: &mut Vec<TextRun>,
) {
    if inline_ids.is_empty() {
        return;
    }
    let mut segments = Vec::new();
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
            text: text.clone(),
            font_size: style.font_size,
            color: style.color,
        }),
        NodeData::Element { .. } => {
            if styles.get(id).display == Display::None {
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
        NodeData::Element { .. } => {
            styles.get(id).display == Display::Block || contains_block(doc, styles, id)
        }
        _ => false,
    }
}

fn contains_block(doc: &Document, styles: &StyleMap, id: NodeId) -> bool {
    for child in doc.children(id) {
        if matches!(doc.get(*child).data(), NodeData::Element { .. })
            && styles.get(*child).display == Display::Block
        {
            return true;
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
        let height = line
            .iter()
            .map(|token| line_height(token.font_size))
            .fold(0.0_f32, f32::max);
        let mut x = origin_x;
        let mut combined: Option<TextRun> = None;
        for token in line {
            let same = combined
                .as_ref()
                .is_some_and(|run| run.font_size == token.font_size && run.color == token.color);
            if same {
                combined
                    .as_mut()
                    .expect("checked")
                    .text
                    .push_str(&token.text);
            } else {
                if let Some(run) = combined.take() {
                    x += measure(&run.text, run.font_size);
                    runs.push(run);
                }
                combined = Some(TextRun {
                    text: token.text,
                    x,
                    y,
                    font_size: token.font_size,
                    color: token.color,
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
        let mut word = String::new();
        for ch in segment.text.chars() {
            if ch == ' ' {
                if !word.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut word),
                        font_size: segment.font_size,
                        color: segment.color,
                    });
                }
                tokens.push(Token {
                    text: " ".to_string(),
                    font_size: segment.font_size,
                    color: segment.color,
                });
            } else {
                word.push(ch);
            }
        }
        if !word.is_empty() {
            tokens.push(Token {
                text: word,
                font_size: segment.font_size,
                color: segment.color,
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
        if token.text == " " || measure(&token.text, token.font_size) <= max_width {
            out.push(token);
            continue;
        }
        for ch in token.text.chars() {
            out.push(Token {
                text: ch.to_string(),
                font_size: token.font_size,
                color: token.color,
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

fn measure(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * 0.5
}

fn line_height(font_size: f32) -> f32 {
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
            text: "hello hello".to_string(),
            font_size: 10.0,
            color: Color::BLACK,
        }];
        let (runs, height) = place_lines(&segments, 0.0, 0.0, 30.0);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "hello");
        assert_eq!(runs[1].text, "hello");
        assert_eq!(runs[1].y, 12.5);
        assert_eq!(height, 25.0);
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
}
