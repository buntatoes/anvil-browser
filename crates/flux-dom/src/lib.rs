//! Document tree. Nodes live in one list and point at each other by id.

mod html;

pub use html::parse_html;

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
}
