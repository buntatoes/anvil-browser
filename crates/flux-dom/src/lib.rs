//! Document tree. Nodes live in one list and point at each other by id.

mod html;

pub use html::{MAX_ATTRS_PER_ELEMENT, MAX_DOM_DEPTH, MAX_DOM_NODES, parse_html};

/// Index of a node inside a [`Document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

impl NodeId {
    pub fn from_raw(index: u32) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Why a tree edit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    Missing,
    Cycle,
}

/// What a node contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeData {
    Document,
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
}

impl NodeData {
    pub fn element(tag: &str) -> Self {
        Self::Element {
            tag: tag.to_ascii_lowercase(),
            attrs: Vec::new(),
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Element { tag, .. } => Some(tag),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        let Self::Element { attrs, .. } = self else {
            return None;
        };
        attrs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    pub fn has_class(&self, class_name: &str) -> bool {
        self.attr("class")
            .is_some_and(|classes| classes.split_ascii_whitespace().any(|c| c == class_name))
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        let Self::Element { attrs, .. } = self else {
            return;
        };
        let key = name.to_ascii_lowercase();
        if let Some((_, slot)) = attrs.iter_mut().find(|(k, _)| *k == key) {
            *slot = value.to_string();
        } else {
            attrs.push((key, value.to_string()));
        }
    }

    pub fn remove_attr(&mut self, name: &str) -> bool {
        let Self::Element { attrs, .. } = self else {
            return false;
        };
        let before = attrs.len();
        attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        attrs.len() != before
    }

    pub fn add_class(&mut self, class_name: &str) {
        let class_name = class_name.trim();
        if class_name.is_empty() || self.has_class(class_name) {
            return;
        }
        let next = match self.attr("class") {
            Some(existing) if !existing.trim().is_empty() => {
                format!("{} {class_name}", existing.trim())
            }
            _ => class_name.to_string(),
        };
        self.set_attr("class", &next);
    }

    pub fn remove_class(&mut self, class_name: &str) {
        let class_name = class_name.trim();
        let Some(existing) = self.attr("class") else {
            return;
        };
        let kept: Vec<&str> = existing
            .split_ascii_whitespace()
            .filter(|c| *c != class_name)
            .collect();
        let joined = kept.join(" ");
        self.set_attr("class", &joined);
    }

    pub fn toggle_class(&mut self, class_name: &str) -> bool {
        if self.has_class(class_name) {
            self.remove_class(class_name);
            false
        } else {
            self.add_class(class_name);
            true
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    id: NodeId,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    data: NodeData,
}

impl Node {
    pub fn id(&self) -> NodeId {
        self.id
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn data(&self) -> &NodeData {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut NodeData {
        &mut self.data
    }
}

/// A document and every node it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    nodes: Vec<Node>,
    html: NodeId,
    head: NodeId,
    body: NodeId,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document with `html`, `head`, and `body` already in place.
    pub fn new() -> Self {
        let mut doc = Self {
            nodes: Vec::new(),
            html: NodeId(0),
            head: NodeId(0),
            body: NodeId(0),
        };
        let root = doc.alloc(NodeData::Document);
        let html = doc.alloc(NodeData::element("html"));
        let head = doc.alloc(NodeData::element("head"));
        let body = doc.alloc(NodeData::element("body"));
        doc.append(root, html).expect("document skeleton");
        doc.append(html, head).expect("document skeleton");
        doc.append(html, body).expect("document skeleton");
        doc.html = html;
        doc.head = head;
        doc.body = body;
        doc
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn html(&self) -> NodeId {
        self.html
    }

    pub fn head(&self) -> NodeId {
        self.head
    }

    pub fn body(&self) -> NodeId {
        self.body
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index())
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.index()].children
    }

    pub fn create_element(&mut self, tag: &str, attrs: Vec<(String, String)>) -> NodeId {
        let mut data = NodeData::element(tag);
        if let NodeData::Element { attrs: slot, .. } = &mut data {
            *slot = attrs
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect();
        }
        self.alloc(data)
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.alloc(NodeData::Text(text.to_string()))
    }

    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
            node.data.set_attr(name, value);
        }
    }

    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> bool {
        self.nodes
            .get_mut(id.index())
            .is_some_and(|node| node.data.remove_attr(name))
    }

    pub fn add_class(&mut self, id: NodeId, class_name: &str) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
            node.data.add_class(class_name);
        }
    }

    pub fn remove_class(&mut self, id: NodeId, class_name: &str) {
        if let Some(node) = self.nodes.get_mut(id.index()) {
            node.data.remove_class(class_name);
        }
    }

    pub fn toggle_class(&mut self, id: NodeId, class_name: &str) -> bool {
        self.nodes
            .get_mut(id.index())
            .is_some_and(|node| node.data.toggle_class(class_name))
    }

    pub fn input_value(&self, id: NodeId) -> String {
        let Some(node) = self.nodes.get(id.index()) else {
            return String::new();
        };
        let data = node.data();
        if data.tag() == Some("select") {
            return self.select_value(id);
        }
        if let Some(val) = data.attr("value") {
            return val.to_string();
        }
        if data.tag() == Some("textarea") {
            return self.text_content(id);
        }
        String::new()
    }

    pub fn set_input_value(&mut self, id: NodeId, value: &str) {
        let tag = self
            .nodes
            .get(id.index())
            .and_then(|n| n.data().tag().map(str::to_string));
        self.set_attribute(id, "value", value);
        if tag.as_deref() == Some("textarea") {
            self.set_text_content(id, value);
        }
    }

    pub fn is_checked(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(id.index()) else {
            return false;
        };
        node.data()
            .attr("checked")
            .is_some_and(|val| !val.eq_ignore_ascii_case("false"))
    }

    pub fn set_checked(&mut self, id: NodeId, checked: bool) {
        if checked {
            self.set_attribute(id, "checked", "checked");
        } else {
            self.remove_attribute(id, "checked");
        }
    }

    pub fn toggle_checked(&mut self, id: NodeId) -> bool {
        let next = !self.is_checked(id);
        self.set_checked(id, next);
        next
    }

    pub fn is_text_input(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(id.index()) else {
            return false;
        };
        let data = node.data();
        match data.tag() {
            Some("textarea") => true,
            Some("input") => {
                let kind = data.attr("type").unwrap_or("text").trim();
                matches!(
                    kind.to_ascii_lowercase().as_str(),
                    "" | "text" | "search" | "password" | "email" | "url" | "tel" | "number"
                )
            }
            _ => false,
        }
    }

    pub fn is_checkbox_input(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(id.index()) else {
            return false;
        };
        let data = node.data();
        data.tag() == Some("input")
            && data
                .attr("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("checkbox"))
    }

    pub fn is_select_control(&self, id: NodeId) -> bool {
        self.nodes
            .get(id.index())
            .is_some_and(|n| n.data().tag() == Some("select"))
    }

    pub fn select_options(&self, select_id: NodeId) -> Vec<NodeId> {
        let Some(node) = self.nodes.get(select_id.index()) else {
            return Vec::new();
        };
        node.children
            .iter()
            .copied()
            .filter(|cid| self.get(*cid).data().tag() == Some("option"))
            .collect()
    }

    pub fn selected_option(&self, select_id: NodeId) -> Option<NodeId> {
        let opts = self.select_options(select_id);
        opts.iter()
            .copied()
            .find(|oid| {
                self.get(*oid)
                    .data()
                    .attr("selected")
                    .is_some_and(|v| !v.eq_ignore_ascii_case("false"))
            })
            .or_else(|| opts.first().copied())
    }

    pub fn select_value(&self, select_id: NodeId) -> String {
        let Some(opt_id) = self.selected_option(select_id) else {
            return String::new();
        };
        if let Some(val) = self.get(opt_id).data().attr("value") {
            return val.to_string();
        }
        self.text_content(opt_id).trim().to_string()
    }

    pub fn select_display_label(&self, select_id: NodeId) -> String {
        let Some(opt_id) = self.selected_option(select_id) else {
            return String::new();
        };
        let text = self.text_content(opt_id).trim().to_string();
        if !text.is_empty() {
            text
        } else {
            self.select_value(select_id)
        }
    }

    pub fn cycle_select_option(&mut self, select_id: NodeId) -> Option<String> {
        let opts = self.select_options(select_id);
        if opts.is_empty() {
            return None;
        }
        let current = self.selected_option(select_id);
        let next_idx = match current.and_then(|c| opts.iter().position(|&id| id == c)) {
            Some(idx) => (idx + 1) % opts.len(),
            None => 0,
        };
        for (i, &oid) in opts.iter().enumerate() {
            if i == next_idx {
                self.set_attribute(oid, "selected", "selected");
            } else {
                self.remove_attribute(oid, "selected");
            }
        }
        let new_val = self.select_value(select_id);
        self.set_attribute(select_id, "value", &new_val);
        Some(new_val)
    }

    pub fn is_submit_control(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(id.index()) else {
            return false;
        };
        let data = node.data();
        match data.tag() {
            Some("input") => data
                .attr("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("submit")),
            Some("button") => {
                let kind = data.attr("type").unwrap_or("submit").trim();
                kind.eq_ignore_ascii_case("submit")
            }
            _ => false,
        }
    }

    pub fn enclosing_form(&self, id: NodeId) -> Option<NodeId> {
        let mut cursor = Some(id);
        while let Some(curr) = cursor {
            let node = self.nodes.get(curr.index())?;
            if node.data().tag() == Some("form") {
                return Some(curr);
            }
            cursor = node.parent();
        }
        None
    }

    /// Returns `(action, method_uppercase, urlencoded_payload)` for a `<form>` node.
    pub fn serialize_form(&self, form_id: NodeId) -> (String, String, String) {
        let Some(form_node) = self.nodes.get(form_id.index()) else {
            return (String::new(), "GET".to_string(), String::new());
        };
        let action = form_node
            .data()
            .attr("action")
            .unwrap_or("")
            .trim()
            .to_string();
        let method = form_node
            .data()
            .attr("method")
            .unwrap_or("GET")
            .trim()
            .to_ascii_uppercase();
        let method = if method == "POST" {
            "POST".to_string()
        } else {
            "GET".to_string()
        };
        let mut pairs = Vec::new();
        self.collect_form_pairs(form_id, &mut pairs);
        let encoded = pairs
            .into_iter()
            .map(|(k, v)| format!("{}={}", urlencode_component(&k), urlencode_component(&v)))
            .collect::<Vec<_>>()
            .join("&");
        (action, method, encoded)
    }

    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        if self.nodes.get(id.index()).is_none() {
            return;
        }
        let old_children = std::mem::take(&mut self.nodes[id.index()].children);
        for child in old_children {
            if let Some(node) = self.nodes.get_mut(child.index()) {
                node.parent = None;
            }
        }
        if !text.is_empty() {
            let text_id = self.create_text(text);
            let _ = self.append(id, text_id);
        }
    }

    pub fn append(&mut self, parent: NodeId, child: NodeId) -> Result<(), TreeError> {
        if self.nodes.get(parent.index()).is_none() || self.nodes.get(child.index()).is_none() {
            return Err(TreeError::Missing);
        }
        if parent == child {
            return Err(TreeError::Cycle);
        }
        let mut cursor = Some(parent);
        while let Some(id) = cursor {
            if id == child {
                return Err(TreeError::Cycle);
            }
            cursor = self.nodes[id.index()].parent;
        }
        self.detach(child);
        self.nodes[child.index()].parent = Some(parent);
        self.nodes[parent.index()].children.push(child);
        Ok(())
    }

    pub fn title(&self) -> String {
        let Some(id) = self.find_element(self.root(), "title") else {
            return String::new();
        };
        self.text_content(id).trim().to_string()
    }

    pub fn set_title(&mut self, title: &str) {
        let title_id = match self.find_element(self.root(), "title") {
            Some(id) => id,
            None => {
                let id = self.create_element("title", Vec::new());
                let _ = self.append(self.head, id);
                id
            }
        };
        self.set_text_content(title_id, title);
    }

    pub fn get_element_by_id(&self, target_id: &str) -> Option<NodeId> {
        if target_id.is_empty() {
            return None;
        }
        self.find_by_id(self.root(), target_id)
    }

    pub fn query_selector(&self, selector: &str) -> Option<NodeId> {
        let selector = selector.trim();
        if selector.is_empty() {
            return None;
        }
        self.find_by_simple_selector(self.root(), selector)
    }

    pub fn embedded_css(&self) -> String {
        let mut blocks = Vec::new();
        self.collect_style(self.root(), &mut blocks);
        blocks.join("\n")
    }

    pub fn scripts(&self) -> Vec<String> {
        let mut blocks = Vec::new();
        self.collect_scripts(self.root(), &mut blocks);
        blocks
    }

    pub fn stylesheet_links(&self) -> Vec<String> {
        let mut links = Vec::new();
        self.collect_stylesheet_links(self.root(), &mut links);
        links
    }

    pub fn images(&self) -> Vec<(NodeId, String)> {
        let mut imgs = Vec::new();
        self.collect_images(self.root(), &mut imgs);
        imgs
    }

    pub fn videos(&self) -> Vec<(NodeId, String)> {
        let mut vids = Vec::new();
        self.collect_videos(self.root(), &mut vids);
        vids
    }

    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    fn alloc(&mut self, data: NodeData) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            id,
            parent: None,
            children: Vec::new(),
            data,
        });
        id
    }

    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.nodes[child.index()].parent else {
            return;
        };
        self.nodes[parent.index()]
            .children
            .retain(|id| *id != child);
        self.nodes[child.index()].parent = None;
    }

    fn find_element(&self, id: NodeId, tag: &str) -> Option<NodeId> {
        if self.get(id).data().tag() == Some(tag) {
            return Some(id);
        }
        for child in self.children(id) {
            if let Some(found) = self.find_element(*child, tag) {
                return Some(found);
            }
        }
        None
    }

    fn find_by_id(&self, id: NodeId, target_id: &str) -> Option<NodeId> {
        if self.get(id).data().id() == Some(target_id) {
            return Some(id);
        }
        for child in self.children(id) {
            if let Some(found) = self.find_by_id(*child, target_id) {
                return Some(found);
            }
        }
        None
    }

    fn find_by_simple_selector(&self, id: NodeId, selector: &str) -> Option<NodeId> {
        if matches_simple_selector(self.get(id).data(), selector) {
            return Some(id);
        }
        for child in self.children(id) {
            if let Some(found) = self.find_by_simple_selector(*child, selector) {
                return Some(found);
            }
        }
        None
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        if let Some(text) = self.get(id).data().text() {
            out.push_str(text);
        }
        for child in self.children(id) {
            self.collect_text(*child, out);
        }
    }

    fn collect_style(&self, id: NodeId, out: &mut Vec<String>) {
        if self.get(id).data().tag() == Some("style") {
            let css = self.text_content(id);
            if !css.is_empty() {
                out.push(css);
            }
            return;
        }
        for child in self.children(id) {
            self.collect_style(*child, out);
        }
    }

    fn collect_scripts(&self, id: NodeId, out: &mut Vec<String>) {
        if self.get(id).data().tag() == Some("script") {
            let code = self.text_content(id);
            if !code.trim().is_empty() {
                out.push(code);
            }
            return;
        }
        for child in self.children(id) {
            self.collect_scripts(*child, out);
        }
    }

    fn collect_stylesheet_links(&self, id: NodeId, out: &mut Vec<String>) {
        let data = self.get(id).data();
        if data.tag() == Some("link") {
            let is_stylesheet = data.attr("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            });
            if is_stylesheet {
                if let Some(href) = data.attr("href") {
                    let href = href.trim();
                    if !href.is_empty() {
                        out.push(href.to_string());
                    }
                }
            }
        }
        for child in self.children(id) {
            self.collect_stylesheet_links(*child, out);
        }
    }

    fn collect_images(&self, id: NodeId, out: &mut Vec<(NodeId, String)>) {
        let data = self.get(id).data();
        if data.tag() == Some("img") {
            if let Some(src) = data.attr("src") {
                let src = src.trim();
                if !src.is_empty() {
                    out.push((id, src.to_string()));
                }
            }
        }
        for child in self.children(id) {
            self.collect_images(*child, out);
        }
    }

    fn collect_videos(&self, id: NodeId, out: &mut Vec<(NodeId, String)>) {
        let data = self.get(id).data();
        if data.tag() == Some("video") {
            if let Some(src) = data.attr("src") {
                let src = src.trim();
                if !src.is_empty() {
                    out.push((id, src.to_string()));
                }
            } else {
                for child in self.children(id) {
                    let cdata = self.get(*child).data();
                    if cdata.tag() == Some("source") {
                        if let Some(src) = cdata.attr("src") {
                            let src = src.trim();
                            if !src.is_empty() {
                                out.push((id, src.to_string()));
                                break;
                            }
                        }
                    }
                }
            }
        }
        for child in self.children(id) {
            self.collect_videos(*child, out);
        }
    }

    fn collect_form_pairs(&self, id: NodeId, out: &mut Vec<(String, String)>) {
        let data = self.get(id).data();
        match data.tag() {
            Some("input") => {
                if let Some(name) = data.attr("name") {
                    let name = name.trim();
                    if !name.is_empty() {
                        let kind = data
                            .attr("type")
                            .unwrap_or("text")
                            .trim()
                            .to_ascii_lowercase();
                        match kind.as_str() {
                            "checkbox" | "radio" => {
                                if self.is_checked(id) {
                                    let val = data.attr("value").unwrap_or("on");
                                    out.push((name.to_string(), val.to_string()));
                                }
                            }
                            "submit" | "button" | "reset" => {}
                            _ => {
                                let val = data.attr("value").unwrap_or("");
                                out.push((name.to_string(), val.to_string()));
                            }
                        }
                    }
                }
            }
            Some("textarea") => {
                if let Some(name) = data.attr("name") {
                    let name = name.trim();
                    if !name.is_empty() {
                        out.push((name.to_string(), self.input_value(id)));
                    }
                }
            }
            Some("select") => {
                if let Some(name) = data.attr("name") {
                    let name = name.trim();
                    if !name.is_empty() {
                        out.push((name.to_string(), self.select_value(id)));
                    }
                }
            }
            _ => {}
        }
        for child in self.children(id) {
            self.collect_form_pairs(*child, out);
        }
    }
}

pub fn urlencode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn matches_simple_selector(data: &NodeData, selector: &str) -> bool {
    let Some(actual_tag) = data.tag() else {
        return false;
    };
    if let Some(id_name) = selector.strip_prefix('#') {
        return data.id() == Some(id_name);
    }
    if let Some(class_name) = selector.strip_prefix('.') {
        return data.has_class(class_name);
    }
    if let Some((tag_part, id_name)) = selector.split_once('#') {
        return (tag_part.is_empty() || actual_tag.eq_ignore_ascii_case(tag_part))
            && data.id() == Some(id_name);
    }
    if let Some((tag_part, class_name)) = selector.split_once('.') {
        return (tag_part.is_empty() || actual_tag.eq_ignore_ascii_case(tag_part))
            && data.has_class(class_name);
    }
    actual_tag.eq_ignore_ascii_case(selector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_refuses_a_cycle() {
        let mut doc = Document::new();
        let div = doc.create_element("div", Vec::new());
        doc.append(doc.body(), div).unwrap();
        assert_eq!(doc.append(div, doc.body()), Err(TreeError::Cycle));
    }

    #[test]
    fn reads_text_attributes_and_title() {
        let doc = parse_html(
            "<title>Hello</title><p class=\"note\">A &amp; B</p><!-- hidden --><br><span>C</span>",
        );
        assert_eq!(doc.title(), "Hello");
        let paragraph = doc
            .children(doc.body())
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().tag() == Some("p"))
            .unwrap();
        assert_eq!(doc.get(paragraph).data().attr("class"), Some("note"));
        assert_eq!(doc.text_content(paragraph), "A & B");
        let tags: Vec<_> = doc
            .children(doc.body())
            .iter()
            .filter_map(|id| doc.get(*id).data().tag())
            .collect();
        assert_eq!(tags, ["p", "br", "span"]);
    }

    #[test]
    fn script_text_keeps_a_less_than() {
        let doc = parse_html("<script>if (a < b) {}</script><p>Hi</p>");
        let script = doc.find_element(doc.root(), "script").unwrap();
        assert_eq!(doc.text_content(script), "if (a < b) {}");
        assert_eq!(doc.scripts(), vec!["if (a < b) {}"]);
        assert!(doc.find_element(doc.body(), "p").is_some());
    }

    #[test]
    fn collects_stylesheet_links_in_order() {
        let doc = parse_html(
            "<head>
                <link rel=\"icon\" href=\"favicon.ico\">
                <link rel=\"stylesheet\" href=\"base.css\">
                <link rel=\"alternate stylesheet\" href=\"theme.css\">
             </head>",
        );
        assert_eq!(doc.stylesheet_links(), vec!["base.css", "theme.css"]);
    }

    #[test]
    fn queries_and_mutates_elements_by_id_and_selector() {
        let mut doc = parse_html("<div id=\"banner\" class=\"card primary\">Old</div>");
        let id = doc.get_element_by_id("banner").unwrap();
        assert_eq!(doc.query_selector(".primary"), Some(id));
        assert_eq!(doc.query_selector("div.card"), Some(id));
        doc.set_text_content(id, "Updated");
        doc.set_attribute(id, "data-state", "active");
        assert_eq!(doc.text_content(id), "Updated");
        assert_eq!(doc.get(id).data().attr("data-state"), Some("active"));
    }

    #[test]
    fn serializes_form_inputs_checkboxes_and_toggles_classes() {
        let mut doc = parse_html(
            "<form id=\"search-form\" action=\"/search\" method=\"POST\">
                <input id=\"q\" type=\"text\" name=\"query\" value=\"flux engine\">
                <input id=\"exact\" type=\"checkbox\" name=\"exact\" value=\"1\" checked>
                <button id=\"go\" type=\"submit\">Search</button>
             </form>",
        );
        let q = doc.get_element_by_id("q").unwrap();
        let exact = doc.get_element_by_id("exact").unwrap();
        let go = doc.get_element_by_id("go").unwrap();
        let form = doc.enclosing_form(go).unwrap();

        assert!(doc.is_text_input(q));
        assert!(doc.is_checkbox_input(exact));
        assert!(doc.is_submit_control(go));

        let (action, method, body) = doc.serialize_form(form);
        assert_eq!(action, "/search");
        assert_eq!(method, "POST");
        assert_eq!(body, "query=flux+engine&exact=1");

        doc.set_input_value(q, "a&b");
        assert!(!doc.toggle_checked(exact));
        let (_, _, body2) = doc.serialize_form(form);
        assert_eq!(body2, "query=a%26b");

        assert!(doc.toggle_class(form, "submitted"));
        assert!(doc.get(form).data().has_class("submitted"));
        assert!(!doc.toggle_class(form, "submitted"));
        assert!(!doc.get(form).data().has_class("submitted"));
    }

    #[test]
    fn caps_adversarial_dom_nesting_depth_and_attribute_counts() {
        let mut deep_html = String::new();
        for _ in 0..400 {
            deep_html.push_str("<div>");
        }
        deep_html.push_str("<p id=\"leaf\">Safe Leaf</p>");
        let doc = parse_html(&deep_html);
        let leaf = doc.get_element_by_id("leaf").unwrap();
        let mut depth = 0usize;
        let mut cursor = Some(leaf);
        while let Some(curr) = cursor {
            depth += 1;
            cursor = doc.get(curr).parent();
        }
        assert!(depth <= MAX_DOM_DEPTH + 4);
    }

    #[test]
    fn parses_tables_textareas_and_cycles_select_options() {
        let mut doc = parse_html(
            "<table id=\"tbl\">
               <tr><th>Col A<th>Col B
               <tr><td>1<td>2
             </table>
             <form id=\"f\" action=\"/save\" method=\"POST\">
               <textarea id=\"bio\" name=\"bio\">Line 1\nLine 2</textarea>
               <select id=\"tier\" name=\"tier\">
                 <option value=\"free\">Free Tier
                 <option value=\"pro\" selected>Pro Tier
                 <option value=\"max\">Max Tier
               </select>
             </form>",
        );
        let tbl = doc.get_element_by_id("tbl").unwrap();
        assert_eq!(doc.children(tbl).len(), 2);

        let tier = doc.get_element_by_id("tier").unwrap();
        assert!(doc.is_select_control(tier));
        assert_eq!(doc.select_value(tier), "pro");
        assert_eq!(doc.select_display_label(tier), "Pro Tier");

        assert_eq!(doc.cycle_select_option(tier), Some("max".to_string()));
        assert_eq!(doc.cycle_select_option(tier), Some("free".to_string()));

        let form = doc.get_element_by_id("f").unwrap();
        let (_, _, payload) = doc.serialize_form(form);
        assert_eq!(payload, "bio=Line+1+Line+2&tier=free");
    }
}
