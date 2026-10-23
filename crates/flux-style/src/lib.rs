//! Style sheets, selector matching, and the cascade.

use flux_dom::{Document, NodeData, NodeId};

/// A parsed style sheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaQuery {
    pub min_width_px: Option<u32>,
    pub max_width_px: Option<u32>,
    pub color_scheme: Option<String>,
}

impl MediaQuery {
    pub fn parse(cond: &str) -> Self {
        let mut mq = Self::default();
        let lower = cond.to_ascii_lowercase();
        let mut rest = lower.as_str();
        while let Some(open) = rest.find('(') {
            let after_open = &rest[open + 1..];
            let Some(close) = after_open.find(')') else {
                break;
            };
            let expr = after_open[..close].trim();
            if let Some((k, v)) = expr.split_once(':') {
                let key = k.trim();
                let val = v.trim().trim_end_matches("px").trim();
                match key {
                    "min-width" => {
                        mq.min_width_px = val.parse::<f32>().ok().map(|f| f.round() as u32);
                    }
                    "max-width" => {
                        mq.max_width_px = val.parse::<f32>().ok().map(|f| f.round() as u32);
                    }
                    "prefers-color-scheme" => {
                        mq.color_scheme = Some(val.to_string());
                    }
                    _ => {}
                }
            }
            rest = &after_open[close + 1..];
        }
        mq
    }

    pub fn matches(&self, viewport_width: f32) -> bool {
        let vw = viewport_width.max(0.0).round() as u32;
        if let Some(min_w) = self.min_width_px {
            if vw < min_w {
                return false;
            }
        }
        if let Some(max_w) = self.max_width_px {
            if vw > max_w {
                return false;
            }
        }
        if let Some(scheme) = &self.color_scheme {
            if !scheme.eq_ignore_ascii_case("dark") {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub media: Option<MediaQuery>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompoundSelector {
    pub universal: bool,
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

impl CompoundSelector {
    fn is_empty(&self) -> bool {
        !self.universal && self.tag.is_none() && self.id.is_none() && self.classes.is_empty()
    }

    fn matches(&self, doc: &Document, node_id: NodeId) -> bool {
        let data = doc.get(node_id).data();
        let Some(actual_tag) = data.tag() else {
            return false;
        };
        if let Some(tag) = &self.tag {
            if actual_tag != tag {
                return false;
            }
        }
        if let Some(id) = &self.id {
            if data.id() != Some(id.as_str()) {
                return false;
            }
        }
        for class_name in &self.classes {
            if !data.has_class(class_name) {
                return false;
            }
        }
        true
    }

    fn specificity(&self) -> (u16, u16, u16) {
        let ids = u16::from(self.id.is_some());
        let classes = self.classes.len() as u16;
        let tags = u16::from(self.tag.is_some());
        (ids, classes, tags)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub chain: Vec<CompoundSelector>,
}

impl Selector {
    pub fn matches(&self, doc: &Document, node_id: NodeId) -> bool {
        let Some((last, ancestors)) = self.chain.split_last() else {
            return false;
        };
        if !last.matches(doc, node_id) {
            return false;
        }
        let mut current = doc.get(node_id).parent();
        for required in ancestors.iter().rev() {
            let mut found = None;
            while let Some(parent_id) = current {
                if required.matches(doc, parent_id) {
                    found = doc.get(parent_id).parent();
                    break;
                }
                current = doc.get(parent_id).parent();
            }
            if found.is_none() && current.is_none() {
                return false;
            }
            current = found;
        }
        true
    }

    pub fn specificity(&self) -> (u16, u16, u16) {
        self.chain.iter().fold((0, 0, 0), |(i, c, t), part| {
            let (pi, pc, pt) = part.specificity();
            (i + pi, c + pc, t + pt)
        })
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

    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some(hex) = value.strip_prefix('#') {
            return parse_hex(hex);
        }
        let lower = value.to_ascii_lowercase();
        if let Some(inner) = lower.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
            return parse_rgb(inner, false);
        }
        if let Some(inner) = lower
            .strip_prefix("rgba(")
            .and_then(|s| s.strip_suffix(')'))
        {
            return parse_rgb(inner, true);
        }
        Some(match lower.as_str() {
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
    Flex,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignItems {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
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
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearGradient {
    pub angle_deg: f32,
    pub start_color: Color,
    pub end_color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub position: Position,
    pub top: Size,
    pub right: Size,
    pub bottom: Size,
    pub left: Size,
    pub z_index: i32,
    pub overflow: Overflow,
    pub color: Color,
    pub background: Option<Color>,
    pub background_gradient: Option<LinearGradient>,
    pub box_shadow: Option<BoxShadow>,
    pub font_size: f32,
    pub margin: Edge,
    pub padding: Edge,
    pub border: Edge,
    pub border_color: Color,
    pub border_radius: f32,
    pub width: Size,
    pub height: Size,
    pub max_width: Size,
    pub underline: bool,
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub gap: f32,
}

impl ComputedStyle {
    pub const fn initial() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            top: Size::Auto,
            right: Size::Auto,
            bottom: Size::Auto,
            left: Size::Auto,
            z_index: 0,
            overflow: Overflow::Visible,
            color: Color::BLACK,
            background: None,
            background_gradient: None,
            box_shadow: None,
            font_size: 16.0,
            margin: Edge::zero(),
            padding: Edge::zero(),
            border: Edge::zero(),
            border_color: Color::BLACK,
            border_radius: 0.0,
            width: Size::Auto,
            height: Size::Auto,
            max_width: Size::Auto,
            underline: false,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Stretch,
            gap: 0.0,
        }
    }

    fn inherited_child(self) -> Self {
        Self {
            color: self.color,
            font_size: self.font_size,
            underline: self.underline,
            border_color: self.color,
            ..Self::initial()
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

const FALLBACK: ComputedStyle = ComputedStyle::initial();

pub fn parse_css(input: &str) -> Stylesheet {
    let input = strip_comments(input);
    let mut rules = Vec::new();
    parse_css_rules(&input, None, &mut rules);
    Stylesheet { rules }
}

fn find_matching_brace(src: &str) -> usize {
    let mut depth = 1usize;
    for (idx, ch) in src.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return idx;
                }
            }
            _ => {}
        }
    }
    src.len()
}

fn parse_css_rules(mut rest: &str, active_media: Option<MediaQuery>, rules: &mut Vec<Rule>) {
    while let Some(open) = rest.find('{') {
        let selector_src = rest[..open].trim();
        rest = &rest[open + 1..];
        let close = find_matching_brace(rest);
        let body_src = &rest[..close];
        rest = if close < rest.len() {
            &rest[close + 1..]
        } else {
            ""
        };
        if let Some(cond) = selector_src.strip_prefix("@media") {
            let mq = MediaQuery::parse(cond);
            parse_css_rules(body_src, Some(mq), rules);
            continue;
        }
        if selector_src.starts_with('@') {
            continue;
        }
        let selectors = parse_selectors(selector_src);
        if selectors.is_empty() {
            continue;
        }
        rules.push(Rule {
            selectors,
            declarations: parse_declarations(body_src),
            media: active_media.clone(),
        });
    }
}

pub fn compute_styles(doc: &Document, sheet: &Stylesheet) -> StyleMap {
    compute_styles_with_viewport(doc, sheet, 960.0)
}

pub fn compute_styles_with_viewport(
    doc: &Document,
    sheet: &Stylesheet,
    viewport_width: f32,
) -> StyleMap {
    let mut styles = vec![ComputedStyle::initial(); doc_len(doc)];
    walk(
        doc,
        doc.root(),
        &ComputedStyle::initial(),
        sheet,
        viewport_width,
        &mut styles,
    );
    StyleMap { styles }
}

fn doc_len(doc: &Document) -> usize {
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
    viewport_width: f32,
    out: &mut [ComputedStyle],
) {
    let style = match doc.get(id).data() {
        NodeData::Element { tag, .. } => {
            let mut style = parent.inherited_child();
            apply_user_agent(&mut style, tag);
            apply_author(&mut style, doc, id, sheet, viewport_width);
            if let Some(inline_css) = doc.get(id).data().attr("style") {
                for declaration in parse_declarations(inline_css) {
                    apply_declaration(&mut style, &declaration);
                }
            }
            style
        }
        _ => *parent,
    };
    if let Some(slot) = out.get_mut(id.index()) {
        *slot = style;
    }
    let children: Vec<NodeId> = doc.children(id).to_vec();
    for child in children {
        walk(doc, child, &style, sheet, viewport_width, out);
    }
}

fn apply_author(
    style: &mut ComputedStyle,
    doc: &Document,
    id: NodeId,
    sheet: &Stylesheet,
    viewport_width: f32,
) {
    let mut matched = Vec::new();
    for (order, rule) in sheet.rules.iter().enumerate() {
        if let Some(mq) = &rule.media {
            if !mq.matches(viewport_width) {
                continue;
            }
        }
        let specificity = rule
            .selectors
            .iter()
            .filter(|selector| selector.matches(doc, id))
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
        "base" | "head" | "link" | "meta" | "option" | "script" | "style" | "title"
    ) {
        style.display = Display::None;
    }
    if tag == "body" {
        style.margin = Edge::all(8.0);
    }
    if tag == "table" {
        style.margin.top = 6.0;
        style.margin.bottom = 6.0;
    }
    if tag == "th" || tag == "td" {
        style.padding = Edge::all(6.0);
        style.border = Edge::all(1.0);
        style.border_color = Color {
            r: 68,
            g: 60,
            b: 52,
            a: 255,
        };
    }
    if tag == "ul" || tag == "ol" {
        style.padding.left = 24.0;
        style.margin.top = 6.0;
        style.margin.bottom = 6.0;
    }
    if tag == "hr" {
        style.height = Size::Px(1.0);
        style.background = Some(Color {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        });
        style.margin.top = 8.0;
        style.margin.bottom = 8.0;
    }
    if tag == "a" {
        style.color = Color {
            r: 0,
            g: 102,
            b: 204,
            a: 255,
        };
        style.underline = true;
    }
    if tag == "form" {
        style.margin.top = 6.0;
        style.margin.bottom = 10.0;
    }
    if tag == "input" || tag == "textarea" || tag == "select" {
        style.display = Display::Block;
        style.background = Some(Color::WHITE);
        style.color = Color {
            r: 24,
            g: 28,
            b: 36,
            a: 255,
        };
        style.border = Edge::all(1.0);
        style.border_color = Color {
            r: 138,
            g: 148,
            b: 166,
            a: 255,
        };
        style.border_radius = 4.0;
        style.padding = Edge {
            top: 6.0,
            right: 8.0,
            bottom: 6.0,
            left: 8.0,
        };
        style.margin.top = 4.0;
        style.margin.bottom = 4.0;
    }
    if tag == "button" {
        style.display = Display::Block;
        style.background = Some(Color {
            r: 43,
            g: 53,
            b: 72,
            a: 255,
        });
        style.color = Color {
            r: 244,
            g: 241,
            b: 234,
            a: 255,
        };
        style.border = Edge::all(1.0);
        style.border_color = Color {
            r: 68,
            g: 82,
            b: 108,
            a: 255,
        };
        style.border_radius = 4.0;
        style.padding = Edge {
            top: 6.0,
            right: 12.0,
            bottom: 6.0,
            left: 12.0,
        };
        style.margin.top = 4.0;
        style.margin.bottom = 4.0;
    }
    style.font_size = match tag {
        "h1" => 32.0,
        "h2" => 24.0,
        "h3" => 20.0,
        "h4" => 18.0,
        "small" => 13.0,
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
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
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
                "flex" => Display::Flex,
                "none" => Display::None,
                _ => return,
            };
        }
        "color" => {
            if let Some(color) = Color::parse(value) {
                style.color = color;
            }
        }
        "background" | "background-color" | "background-image" => {
            if value.eq_ignore_ascii_case("transparent") || value.eq_ignore_ascii_case("none") {
                style.background = None;
                style.background_gradient = None;
            } else if let Some(grad) = parse_linear_gradient(value) {
                style.background_gradient = Some(grad);
                style.background = Some(grad.start_color);
            } else if let Some(color) = Color::parse(value) {
                style.background = Some(color);
                style.background_gradient = None;
            }
        }
        "box-shadow" => {
            if value.eq_ignore_ascii_case("none") {
                style.box_shadow = None;
            } else if let Some(shadow) = parse_box_shadow(value) {
                style.box_shadow = Some(shadow);
            }
        }
        "font-size" => {
            if let Some(size) = parse_px(value) {
                style.font_size = size;
            }
        }
        "text-decoration" => {
            if value.eq_ignore_ascii_case("underline") {
                style.underline = true;
            } else if value.eq_ignore_ascii_case("none") {
                style.underline = false;
            }
        }
        "margin" => {
            if let Some(edge) = parse_edge(value) {
                style.margin = edge;
            }
        }
        "margin-top" => {
            if let Some(px) = parse_px(value) {
                style.margin.top = px;
            }
        }
        "margin-right" => {
            if let Some(px) = parse_px(value) {
                style.margin.right = px;
            }
        }
        "margin-bottom" => {
            if let Some(px) = parse_px(value) {
                style.margin.bottom = px;
            }
        }
        "margin-left" => {
            if let Some(px) = parse_px(value) {
                style.margin.left = px;
            }
        }
        "padding" => {
            if let Some(edge) = parse_edge(value) {
                style.padding = edge;
            }
        }
        "padding-top" => {
            if let Some(px) = parse_px(value) {
                style.padding.top = px;
            }
        }
        "padding-right" => {
            if let Some(px) = parse_px(value) {
                style.padding.right = px;
            }
        }
        "padding-bottom" => {
            if let Some(px) = parse_px(value) {
                style.padding.bottom = px;
            }
        }
        "padding-left" => {
            if let Some(px) = parse_px(value) {
                style.padding.left = px;
            }
        }
        "border" => {
            apply_border_shorthand(style, value);
        }
        "border-width" => {
            if let Some(edge) = parse_edge(value) {
                style.border = edge;
            }
        }
        "border-color" => {
            if let Some(color) = Color::parse(value) {
                style.border_color = color;
            }
        }
        "border-top" => {
            if let Some((w, c)) = parse_border_side(value) {
                style.border.top = w;
                if let Some(color) = c {
                    style.border_color = color;
                }
            }
        }
        "border-right" => {
            if let Some((w, c)) = parse_border_side(value) {
                style.border.right = w;
                if let Some(color) = c {
                    style.border_color = color;
                }
            }
        }
        "border-bottom" => {
            if let Some((w, c)) = parse_border_side(value) {
                style.border.bottom = w;
                if let Some(color) = c {
                    style.border_color = color;
                }
            }
        }
        "border-left" => {
            if let Some((w, c)) = parse_border_side(value) {
                style.border.left = w;
                if let Some(color) = c {
                    style.border_color = color;
                }
            }
        }
        "border-radius" => {
            if let Some(px) = parse_px(value) {
                style.border_radius = px.max(0.0);
            }
        }
        "width" => style.width = parse_size(value),
        "height" => style.height = parse_size(value),
        "max-width" => style.max_width = parse_size(value),
        "position" => {
            style.position = match value.to_ascii_lowercase().as_str() {
                "static" => Position::Static,
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                "fixed" => Position::Fixed,
                _ => return,
            };
        }
        "top" => style.top = parse_size(value),
        "right" => style.right = parse_size(value),
        "bottom" => style.bottom = parse_size(value),
        "left" => style.left = parse_size(value),
        "z-index" => {
            if let Ok(z) = value.parse::<i32>() {
                style.z_index = z;
            }
        }
        "overflow" => {
            style.overflow = match value.to_ascii_lowercase().as_str() {
                "hidden" | "scroll" | "auto" => Overflow::Hidden,
                "visible" => Overflow::Visible,
                _ => return,
            };
        }
        "flex-direction" => {
            style.flex_direction = match value {
                "column" => FlexDirection::Column,
                "row" => FlexDirection::Row,
                _ => return,
            };
        }
        "justify-content" => {
            style.justify_content = match value {
                "flex-start" | "start" => JustifyContent::Start,
                "center" => JustifyContent::Center,
                "flex-end" | "end" => JustifyContent::End,
                "space-between" => JustifyContent::SpaceBetween,
                _ => return,
            };
        }
        "align-items" => {
            style.align_items = match value {
                "flex-start" | "start" => AlignItems::Start,
                "center" => AlignItems::Center,
                "flex-end" | "end" => AlignItems::End,
                "stretch" => AlignItems::Stretch,
                _ => return,
            };
        }
        "gap" => {
            if let Some(px) = parse_px(value) {
                style.gap = px.max(0.0);
            }
        }
        _ => {}
    }
}

fn apply_border_shorthand(style: &mut ComputedStyle, value: &str) {
    if value.eq_ignore_ascii_case("none") || value == "0" {
        style.border = Edge::zero();
        return;
    }
    if let Some((width, color)) = parse_border_side(value) {
        style.border = Edge::all(width);
        if let Some(color) = color {
            style.border_color = color;
        }
    }
}

fn parse_border_side(value: &str) -> Option<(f32, Option<Color>)> {
    let mut width = None;
    let mut color = None;
    for token in split_css_tokens(value) {
        if let Some(px) = parse_px(token) {
            width = Some(px);
        } else if !matches!(
            token.to_ascii_lowercase().as_str(),
            "solid" | "dashed" | "dotted" | "double"
        ) {
            if let Some(c) = Color::parse(token) {
                color = Some(c);
            }
        }
    }
    Some((width?, color))
}

fn split_css_tokens(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    for (idx, ch) in value.char_indices() {
        match ch {
            '(' => {
                if start.is_none() {
                    start = Some(idx);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
            }
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    out.push(&value[s..idx]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(idx);
                }
            }
        }
    }
    if let Some(s) = start {
        out.push(&value[s..]);
    }
    out
}

fn parse_size(value: &str) -> Size {
    if value == "auto" || value == "none" {
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

fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let mut lengths = Vec::new();
    let mut color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 128,
    };
    for token in split_css_tokens(value) {
        if let Some(px) = parse_px(token) {
            lengths.push(px);
        } else if let Some(c) = Color::parse(token) {
            color = c;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
        spread_radius: lengths.get(3).copied().unwrap_or(0.0),
        color,
    })
}

fn parse_linear_gradient(value: &str) -> Option<LinearGradient> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))?;

    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (idx, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(inner[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim());
    if args.len() < 2 {
        return None;
    }

    let mut angle_deg = 180.0f32;
    let mut color_start_idx = 0usize;
    let first = args[0];
    if let Some(deg_str) = first.strip_suffix("deg") {
        if let Ok(deg) = deg_str.trim().parse::<f32>() {
            angle_deg = deg;
            color_start_idx = 1;
        }
    } else if first.starts_with("to ") {
        angle_deg = match first {
            "to right" => 90.0,
            "to left" => 270.0,
            "to top" => 0.0,
            "to bottom" => 180.0,
            _ => 135.0,
        };
        color_start_idx = 1;
    }

    if args.len() < color_start_idx + 2 {
        return None;
    }
    let parse_stop_color = |stop: &str| -> Option<Color> {
        let tokens = split_css_tokens(stop);
        Color::parse(tokens.first().copied()?)
    };
    let start_color = parse_stop_color(args[color_start_idx])?;
    let end_color = parse_stop_color(args[args.len() - 1])?;
    Some(LinearGradient {
        angle_deg,
        start_color,
        end_color,
    })
}

fn parse_selectors(source: &str) -> Vec<Selector> {
    source.split(',').filter_map(parse_selector).collect()
}

fn parse_selector(source: &str) -> Option<Selector> {
    let mut chain = Vec::new();
    for token in source.split_whitespace() {
        let compound = parse_compound_selector(token)?;
        chain.push(compound);
    }
    if chain.is_empty() {
        None
    } else {
        Some(Selector { chain })
    }
}

fn parse_compound_selector(token: &str) -> Option<CompoundSelector> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if token == "*" {
        return Some(CompoundSelector {
            universal: true,
            ..CompoundSelector::default()
        });
    }
    let mut out = CompoundSelector::default();
    let mut chars = token.chars().peekable();
    let mut mode = 't'; // 't' = tag, '#' = id, '.' = class

    while chars.peek().is_some() {
        if let Some(&prefix @ ('#' | '.')) = chars.peek() {
            mode = prefix;
            chars.next();
        }
        let mut ident = String::new();
        while let Some(&ch) = chars.peek() {
            if ch == '#' || ch == '.' {
                break;
            }
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ident.push(ch);
                chars.next();
            } else {
                return None;
            }
        }
        if ident.is_empty() {
            return None;
        }
        match mode {
            't' => out.tag = Some(ident.to_ascii_lowercase()),
            '#' => out.id = Some(ident),
            '.' => out.classes.push(ident),
            _ => return None,
        }
        mode = ' ';
    }

    if out.is_empty() { None } else { Some(out) }
}

pub fn parse_declarations(source: &str) -> Vec<Declaration> {
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

fn parse_rgb(inner: &str, has_alpha: bool) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if (!has_alpha && parts.len() != 3) || (has_alpha && parts.len() != 4) {
        return None;
    }
    let r: u8 = parts[0].parse().ok()?;
    let g: u8 = parts[1].parse().ok()?;
    let b: u8 = parts[2].parse().ok()?;
    let a = if has_alpha {
        let raw: f32 = parts[3].parse().ok()?;
        if raw <= 1.0 && parts[3].contains('.') {
            (raw.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            raw.clamp(0.0, 255.0).round() as u8
        }
    } else {
        255
    };
    Some(Color { r, g, b, a })
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

    #[test]
    fn links_get_a_default_color_that_author_rules_can_override() {
        let doc = parse_html("<p><a href=\"next.html\">Go</a></p>");
        let default_styles = compute_styles(&doc, &parse_css("p { color: #111111 }"));
        let p = doc.children(doc.body())[0];
        let a = doc.children(p)[0];
        assert_eq!(
            default_styles.get(a).color,
            Color {
                r: 0,
                g: 102,
                b: 204,
                a: 255
            }
        );
        assert!(default_styles.get(a).underline);

        let custom_styles = compute_styles(
            &doc,
            &parse_css("a { color: #e8c37a; text-decoration: none; }"),
        );
        assert_eq!(custom_styles.get(a).color, Color::parse("#e8c37a").unwrap());
        assert!(!custom_styles.get(a).underline);
    }

    #[test]
    fn matches_id_class_descendant_and_inline_styles() {
        let doc = parse_html(
            "<div id=\"card\" class=\"panel\"><p class=\"lead\" style=\"padding-left: 14px\">Hi</p></div>",
        );
        let sheet = parse_css(
            ".panel { border: 2px solid rgb(10, 20, 30); display: flex; gap: 12px; }
             #card .lead { color: #123456; }
             p { color: #ffffff; }",
        );
        let styles = compute_styles(&doc, &sheet);
        let card = doc.get_element_by_id("card").unwrap();
        let lead = doc.query_selector(".lead").unwrap();
        assert_eq!(styles.get(card).display, Display::Flex);
        assert_eq!(styles.get(card).gap, 12.0);
        assert_eq!(styles.get(card).border, Edge::all(2.0));
        assert_eq!(
            styles.get(card).border_color,
            Color {
                r: 10,
                g: 20,
                b: 30,
                a: 255
            }
        );
        assert_eq!(styles.get(lead).color, Color::parse("#123456").unwrap());
        assert_eq!(styles.get(lead).padding.left, 14.0);
    }

    #[test]
    fn parses_positioned_layout_z_index_and_overflow() {
        let doc = parse_html("<div id=\"modal\"><input id=\"q\" type=\"text\"></div>");
        let sheet = parse_css(
            "#modal { position: fixed; top: 12px; left: 24px; z-index: 10; overflow: hidden; }",
        );
        let styles = compute_styles(&doc, &sheet);
        let modal = doc.get_element_by_id("modal").unwrap();
        let q = doc.get_element_by_id("q").unwrap();
        assert_eq!(styles.get(modal).position, Position::Fixed);
        assert_eq!(styles.get(modal).top, Size::Px(12.0));
        assert_eq!(styles.get(modal).left, Size::Px(24.0));
        assert_eq!(styles.get(modal).z_index, 10);
        assert_eq!(styles.get(modal).overflow, Overflow::Hidden);
        assert_eq!(styles.get(q).border_radius, 4.0);
    }

    #[test]
    fn parses_box_shadow_and_linear_gradient() {
        let doc = parse_html("<div id=\"card\"></div>");
        let sheet = parse_css(
            "#card { background: linear-gradient(135deg, #3a342c, #25221e); box-shadow: 0px 4px 12px rgba(0, 0, 0, 160); }",
        );
        let styles = compute_styles(&doc, &sheet);
        let card = doc.get_element_by_id("card").unwrap();
        let grad = styles.get(card).background_gradient.expect("gradient");
        assert_eq!(grad.angle_deg, 135.0);
        let shadow = styles.get(card).box_shadow.expect("box-shadow");
        assert_eq!(shadow.offset_y, 4.0);
        assert_eq!(shadow.blur_radius, 12.0);
    }

    #[test]
    fn evaluates_media_queries_across_viewport_widths_and_color_schemes() {
        let doc = parse_html("<div id=\"box\" class=\"responsive\">Text</div>");
        let sheet = parse_css(
            ".responsive { width: 800px; color: #111111; }
             @media (max-width: 600px) {
               .responsive { width: 320px; }
             }
             @media (prefers-color-scheme: dark) {
               .responsive { color: #e8c37a; }
             }
             @media (prefers-color-scheme: light) {
               .responsive { color: #ff0000; }
             }",
        );
        let box_id = doc.get_element_by_id("box").unwrap();

        let desktop = compute_styles_with_viewport(&doc, &sheet, 960.0);
        assert_eq!(desktop.get(box_id).width, Size::Px(800.0));
        assert_eq!(desktop.get(box_id).color, Color::parse("#e8c37a").unwrap());

        let mobile = compute_styles_with_viewport(&doc, &sheet, 480.0);
        assert_eq!(mobile.get(box_id).width, Size::Px(320.0));
        assert_eq!(mobile.get(box_id).color, Color::parse("#e8c37a").unwrap());
    }
}
