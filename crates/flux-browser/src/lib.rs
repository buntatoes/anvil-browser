//! Tabs, history, and the page pipeline.

/// Release name for this tree.
pub const VERSION: &str = "v.01";

use flux_dom::Document;
use flux_layout::layout_document;
use flux_net::{NetError, http_get, parse_url};
use flux_paint::{DisplayList, paint};
use flux_style::{compute_styles, parse_css};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub document: Document,
    pub display_list: DisplayList,
}

#[derive(Debug)]
pub enum BrowserError {
    Net(NetError),
    Status(u16),
    Utf8,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(err) => write!(f, "{err}"),
            Self::Status(status) => write!(f, "the server returned {status}"),
            Self::Utf8 => write!(f, "the page was not text"),
        }
    }
}

impl std::error::Error for BrowserError {}

pub fn render_html(html: &str, url: &str, viewport: Viewport) -> Page {
    let document = flux_dom::parse_html(html);
    let sheet = parse_css(&document.embedded_css());
    let styles = compute_styles(&document, &sheet);
    let tree = layout_document(&document, &styles, viewport.width);
    let display_list = paint(&tree);
    Page {
        url: url.to_string(),
        title: document.title(),
        document,
        display_list,
    }
}

pub fn load_url(url: &str, viewport: Viewport) -> Result<Page, BrowserError> {
    let parsed = parse_url(url).map_err(BrowserError::Net)?;
    let response = http_get(&parsed).map_err(BrowserError::Net)?;
    if !(200..300).contains(&response.status) {
        return Err(BrowserError::Status(response.status));
    }
    let html = String::from_utf8(response.body).map_err(|_| BrowserError::Utf8)?;
    Ok(render_html(&html, url, viewport))
}

#[derive(Debug)]
pub struct Tab {
    history: Vec<Page>,
    index: usize,
}

impl Tab {
    fn empty() -> Self {
        Self {
            history: Vec::new(),
            index: 0,
        }
    }

    pub fn current(&self) -> Option<&Page> {
        self.history.get(self.index)
    }

    fn navigate(&mut self, page: Page) {
        if !self.history.is_empty() {
            self.history.truncate(self.index + 1);
        }
        self.history.push(page);
        self.index = self.history.len() - 1;
    }

    fn back(&mut self) -> bool {
        if self.history.is_empty() || self.index == 0 {
            return false;
        }
        self.index -= 1;
        true
    }

    fn forward(&mut self) -> bool {
        if self.index + 1 >= self.history.len() {
            return false;
        }
        self.index += 1;
        true
    }
}

#[derive(Debug)]
pub struct Session {
    tabs: Vec<Tab>,
    active: usize,
    viewport: Viewport,
}

impl Session {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            tabs: vec![Tab::empty()],
            active: 0,
            viewport,
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active = index;
        true
    }

    pub fn new_tab(&mut self) {
        self.tabs.push(Tab::empty());
        self.active = self.tabs.len() - 1;
    }

    pub fn open_html(&mut self, html: &str, url: &str) {
        let page = render_html(html, url, self.viewport);
        self.tabs[self.active].navigate(page);
    }

    pub fn open_url(&mut self, url: &str) -> Result<(), BrowserError> {
        let page = load_url(url, self.viewport)?;
        self.tabs[self.active].navigate(page);
        Ok(())
    }

    pub fn back(&mut self) -> bool {
        self.tabs[self.active].back()
    }

    pub fn forward(&mut self) -> bool {
        self.tabs[self.active].forward()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_is_v01() {
        assert_eq!(VERSION, "v.01");
    }
    use flux_paint::DisplayItem;
    use flux_style::Color;

    #[test]
    fn renders_the_sample_page() {
        let page = render_html(
            include_str!("../../../examples/hello.html"),
            "hello.html",
            Viewport {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(page.title, "Hello");
        let texts: Vec<_> = page
            .display_list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, color, .. } => Some((text.as_str(), *color)),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|(text, color)| {
            *text == "Flux" && *color == Color::parse("#e8c37a").unwrap()
        }));
        assert!(page.display_list.items.iter().any(|item| {
            matches!(item, DisplayItem::Rect { color, .. } if *color == Color::parse("#1c1a17").unwrap())
        }));
    }

    #[test]
    fn history_drops_the_forward_page() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 100.0,
        });
        session.open_html("<title>One</title><p>One</p>", "one");
        session.open_html("<title>Two</title><p>Two</p>", "two");
        assert!(session.back());
        assert_eq!(session.active().current().unwrap().title, "One");
        assert!(session.forward());
        assert_eq!(session.active().current().unwrap().title, "Two");
        assert!(session.back());
        session.open_html("<title>Three</title><p>Three</p>", "three");
        assert!(!session.forward());
        assert_eq!(session.active().current().unwrap().title, "Three");
        assert_eq!(session.viewport().height, 100.0);
    }

    #[test]
    fn a_new_tab_starts_empty() {
        let mut session = Session::new(Viewport {
            width: 100.0,
            height: 100.0,
        });
        session.open_html("<title>Stay</title>", "stay");
        session.new_tab();
        assert_eq!(session.len(), 2);
        assert!(session.active().current().is_none());
        assert!(session.select(0));
        assert_eq!(session.active().current().unwrap().title, "Stay");
    }
}
