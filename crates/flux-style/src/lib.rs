//! Style sheets and the cascade.

use flux_dom::{Document, NodeData, NodeId};

/// A parsed style sheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Universal,
    Type(String),
}

impl Selector {
    fn matches(&self, tag: &str) -> bool {
        match self {
            Self::Universal => true,
            Self::Type(name) => name == tag,
        }
    }

    fn specificity(&self) -> u8 {
        match self {
            Self::Universal => 0,
            Self::Type(_) => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some(hex) = value.strip_prefix('#') {
            return parse_hex(hex);
        }
        Some(match value.to_ascii_lowercase().as_str() {
            "black" => Self::BLACK,
            "white" => Self {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            "red" => Self {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            "green" => Self {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            },
            "blue" => Self {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
            "gray" | "grey" => Self {
                r: 128,
                g: 128,
                b: 128,
                a: 255,
            },
            "silver" => Self {
                r: 192,
                g: 192,
                b: 192,
                a: 255,
            },
            "yellow" => Self {
                r: 255,
                g: 255,
                b: 0,
                a: 255,
            },
            "orange" => Self {
                r: 255,
                g: 165,
                b: 0,
                a: 255,
            },
            "transparent" => Self {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            },
            _ => return None,
        })
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    Inline,
    Block,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Auto,
    Px(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edge {
    pub const fn zero() -> Self {
        Self {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub margin: Edge,
    pub padding: Edge,
    pub width: Size,
    pub height: Size,
}

impl ComputedStyle {
    pub fn initial() -> Self {
        Self {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: 16.0,
            margin: Edge::zero(),
            padding: Edge::zero(),
            width: Size::Auto,
            height: Size::Auto,
        }
    }

    fn inherited_child(self) -> Self {
        Self {
            display: Display::Inline,
            color: self.color,
            background: None,
            font_size: self.font_size,
            margin: Edge::zero(),
            padding: Edge::zero(),
            width: Size::Auto,
            height: Size::Auto,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleMap {
    styles: Vec<ComputedStyle>,
}

impl StyleMap {
    pub fn get(&self, id: NodeId) -> &ComputedStyle {
        self.styles.get(id.index()).unwrap_or(&FALLBACK)
    }
}

const FALLBACK: ComputedStyle = ComputedStyle {
    display: Display::Inline,
    color: Color::BLACK,
    background: None,
    font_size: 16.0,
    margin: Edge::zero(),
    padding: Edge::zero(),
    width: Size::Auto,
    height: Size::Auto,
};

pub fn parse_css(input: &str) -> Stylesheet {
    let input = strip_comments(input);
    let mut rules = Vec::new();
    let mut rest = input.as_str();
    while let Some(open) = rest.find('{') {
        let selector_src = &rest[..open];
        rest = &rest[open + 1..];
        let close = rest.find('}').unwrap_or(rest.len());
        let declaration_src = &rest[..close];
        rest = if close < rest.len() {
            &rest[close + 1..]
        } else {
            ""
        };
        let selectors = parse_selectors(selector_src);
        if selectors.is_empty() {
            continue;
        }
        rules.push(Rule {
            selectors,
            declarations: parse_declarations(declaration_src),
        });
    }
    Stylesheet { rules }
}

pub fn compute_styles(doc: &Document, sheet: &Stylesheet) -> StyleMap {
    let mut styles = vec![ComputedStyle::initial(); doc_len(doc)];
    walk(
        doc,
        doc.root(),
        &ComputedStyle::initial(),
        sheet,
        &mut styles,
    );
    StyleMap { styles }
}

fn doc_len(doc: &Document) -> usize {
    // The root id is 0 and nodes are allocated in order, so the highest
    // child index plus one is the length. Walk once from the root.
    let mut count = 0;
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        count = count.max(id.index() + 1);
        stack.extend(doc.children(id).iter().copied());
    }
    count
}

fn walk(
    doc: &Document,
    id: NodeId,
    parent: &ComputedStyle,
    sheet: &Stylesheet,
    out: &mut [ComputedStyle],
) {
    let style = match doc.get(id).data() {
        NodeData::Element { tag, .. } => {
            let mut style = parent.inherited_child();
            apply_user_agent(&mut style, tag);
            apply_author(&mut style, tag, sheet);
            style
        }
        _ => *parent,
    };
    if let Some(slot) = out.get_mut(id.index()) {
        *slot = style;
    }
    let children: Vec<NodeId> = doc.children(id).to_vec();
    for child in children {
        walk(doc, child, &style, sheet, out);
    }
}

fn apply_author(style: &mut ComputedStyle, tag: &str, sheet: &Stylesheet) {
    let mut matched = Vec::new();
    for (order, rule) in sheet.rules.iter().enumerate() {
        let specificity = rule
            .selectors
            .iter()
            .filter(|selector| selector.matches(tag))
            .map(Selector::specificity)
            .max();
        if let Some(specificity) = specificity {
            matched.push((specificity, order, rule));
        }
    }
    matched.sort_by_key(|(specificity, order, _)| (*specificity, *order));
    for (_, _, rule) in matched {
        for declaration in &rule.declarations {
            apply_declaration(style, declaration);
        }
    }
}

fn apply_user_agent(style: &mut ComputedStyle, tag: &str) {
    if is_block_tag(tag) {
        style.display = Display::Block;
    }
    if matches!(
        tag,
        "base" | "head" | "link" | "meta" | "script" | "style" | "title"
    ) {
        style.display = Display::None;
    }
    if tag == "body" {
        style.margin = Edge::all(8.0);
    }
    style.font_size = match tag {
        "h1" => 32.0,
        "h2" => 24.0,
        "h3" => 20.0,
        _ => style.font_size,
    };
}

fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "blockquote"
            | "body"
            | "div"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "html"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "td"
            | "th"
            | "tr"
            | "ul"
    )
}

fn apply_declaration(style: &mut ComputedStyle, declaration: &Declaration) {
    let value = declaration.value.trim();
    match declaration.name.as_str() {
        "display" => {
            style.display = match value {
                "block" => Display::Block,
                "inline" => Display::Inline,
                "none" => Display::None,
                _ => return,
            };
        }
        "color" => {
            if let Some(color) = Color::parse(value) {
                style.color = color;
            }
        }
        "background" | "background-color" => {
            if value.eq_ignore_ascii_case("transparent") {
                style.background = None;
            } else if let Some(color) = Color::parse(value) {
                style.background = Some(color);
            }
        }
        "font-size" => {
            if let Some(size) = parse_px(value) {
                style.font_size = size;
            }
        }
        "margin" => {
            if let Some(edge) = parse_edge(value) {
                style.margin = edge;
            }
        }
        "padding" => {
            if let Some(edge) = parse_edge(value) {
                style.padding = edge;
            }
        }
        "width" => style.width = parse_size(value),
        "height" => style.height = parse_size(value),
        _ => {}
    }
}

fn parse_size(value: &str) -> Size {
    if value == "auto" {
        Size::Auto
    } else {
        parse_px(value).map(Size::Px).unwrap_or(Size::Auto)
    }
}

fn parse_edge(value: &str) -> Option<Edge> {
    let parts: Vec<f32> = value
        .split_whitespace()
        .map(parse_px)
        .collect::<Option<_>>()?;
    Some(match parts.as_slice() {
        [all] => Edge::all(*all),
        [vertical, horizontal] => Edge {
            top: *vertical,
            right: *horizontal,
            bottom: *vertical,
            left: *horizontal,
        },
        [top, horizontal, bottom] => Edge {
            top: *top,
            right: *horizontal,
            bottom: *bottom,
            left: *horizontal,
        },
        [top, right, bottom, left] => Edge {
            top: *top,
            right: *right,
            bottom: *bottom,
            left: *left,
        },
        _ => return None,
    })
}

fn parse_px(value: &str) -> Option<f32> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    value.strip_suffix("px")?.parse().ok()
}

fn parse_selectors(source: &str) -> Vec<Selector> {
    source.split(',').filter_map(parse_selector).collect()
}

fn parse_selector(source: &str) -> Option<Selector> {
    let source = source.trim().to_ascii_lowercase();
    if source.is_empty() {
        return None;
    }
    if source == "*" {
        return Some(Selector::Universal);
    }
    if source
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Some(Selector::Type(source));
    }
    None
}

fn parse_declarations(source: &str) -> Vec<Declaration> {
    source
        .split(';')
        .filter_map(|part| {
            let (name, value) = part.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name.is_empty() || value.is_empty() {
                None
            } else {
                Some(Declaration { name, value })
            }
        })
        .collect()
}

fn strip_comments(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(inner) = chars.next() {
                if inner == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_hex(hex: &str) -> Option<Color> {
    let chars: Vec<char> = hex.chars().collect();
    let component = |text: &[char]| -> Option<u8> {
        let value = u8::from_str_radix(&text.iter().collect::<String>(), 16).ok()?;
        Some(value)
    };
    let expand = |ch: char| -> Option<u8> {
        let nibble = ch.to_digit(16)? as u8;
        Some(nibble * 16 + nibble)
    };
    match chars.as_slice() {
        [r, g, b] => Some(Color {
            r: expand(*r)?,
            g: expand(*g)?,
            b: expand(*b)?,
            a: 255,
        }),
        [r, g, b, a] => Some(Color {
            r: expand(*r)?,
            g: expand(*g)?,
            b: expand(*b)?,
            a: expand(*a)?,
        }),
        [r1, r2, g1, g2, b1, b2] => Some(Color {
            r: component(&[*r1, *r2])?,
            g: component(&[*g1, *g2])?,
            b: component(&[*b1, *b2])?,
            a: 255,
        }),
        [r1, r2, g1, g2, b1, b2, a1, a2] => Some(Color {
            r: component(&[*r1, *r2])?,
            g: component(&[*g1, *g2])?,
            b: component(&[*b1, *b2])?,
            a: component(&[*a1, *a2])?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_dom::parse_html;

    #[test]
    fn later_type_rule_beats_an_earlier_universal() {
        let doc = parse_html("<p>Hi</p>");
        let sheet = parse_css("* { color: red } p { color: #00ff00; font-size: 20px; }");
        let styles = compute_styles(&doc, &sheet);
        let paragraph = doc
            .children(doc.body())
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().tag() == Some("p"))
            .unwrap();
        let style = styles.get(paragraph);
        assert_eq!(
            style.color,
            Color {
                r: 0,
                g: 255,
                b: 0,
                a: 255
            }
        );
        assert_eq!(style.font_size, 20.0);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn color_inherits_and_head_stays_hidden() {
        let doc = parse_html("<style>div { color: #ff0000 }</style><div><span>Hi</span></div>");
        let styles = compute_styles(&doc, &parse_css(&doc.embedded_css()));
        let style_node = doc
            .children(doc.head())
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().tag() == Some("style"))
            .unwrap();
        assert_eq!(styles.get(style_node).display, Display::None);
        let div = doc.children(doc.body())[0];
        let span = doc.children(div)[0];
        assert_eq!(doc.get(span).data().tag(), Some("span"));
        assert_eq!(styles.get(span).color, Color::parse("#ff0000").unwrap());
    }
}
