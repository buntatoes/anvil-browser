//! Tabs, history, scripting integration, front-end pages, installer, and the page pipeline.

pub mod frontend;
pub mod installer;
#[cfg(windows)]
pub mod shell;
pub mod ui;

/// Release name for this tree.
pub const VERSION: &str = "v.05";

use std::collections::HashMap;
use std::fs;

use flux_dom::{Document, NodeId};
pub use flux_image::Image;
use flux_image::decode_auto;
use flux_json::{JsonValue, parse_json, to_json_pretty};
use flux_layout::{LayoutBox, hit_test, layout_document_interactive};
use flux_net::{NetError, append_query, fetch, fetch_post, resolve_url};
use flux_paint::{DisplayList, paint};
use flux_raster::{Color, Framebuffer, rasterize};
use flux_script::ScriptRuntime;
use flux_style::{compute_styles, parse_css};

const PAPER: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    None,
    ScriptHandled,
    InputFocused(NodeId),
    CheckboxToggled(NodeId, bool),
    Navigate(String),
    SubmitPost { url: String, body: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub source: String,
    pub external_css: String,
    pub images: HashMap<NodeId, Image>,
    pub focused_input: Option<NodeId>,
    pub document: Document,
    pub runtime: ScriptRuntime,
    pub layout: LayoutBox,
    pub display_list: DisplayList,
}

impl Page {
    pub fn content_height(&self) -> f32 {
        (self.layout.rect.y + self.layout.rect.height).max(0.0)
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        hit_test(&self.layout, x, y)
    }

    pub fn text_input_at(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut cursor = self.hit_test(x, y);
        while let Some(id) = cursor {
            if self.document.is_text_input(id) {
                return Some(id);
            }
            cursor = self.document.get(id).parent();
        }
        None
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        let mut cursor = self.hit_test(x, y);
        while let Some(id) = cursor {
            let node = self.document.get(id);
            if node.data().tag() == Some("a") {
                if let Some(href) = node.data().attr("href") {
                    if !href.trim().is_empty() {
                        return Some(href);
                    }
                }
            }
            cursor = node.parent();
        }
        None
    }

    pub fn restage(&mut self, viewport: Viewport) {
        let embedded = self.document.embedded_css();
        let combined_css = match (self.external_css.is_empty(), embedded.is_empty()) {
            (true, _) => embedded,
            (false, true) => self.external_css.clone(),
            (false, false) => format!("{}\n{embedded}", self.external_css),
        };
        let sheet = parse_css(&combined_css);
        let styles = compute_styles(&self.document, &sheet);
        self.layout = layout_document_interactive(
            &self.document,
            &styles,
            viewport.width,
            &self.images,
            self.focused_input,
        );
        self.display_list = paint(&self.layout);
        self.title = self.document.title();
    }
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
    render_html_with_css(html, "", url, viewport)
}

pub fn render_html_with_css(html: &str, external_css: &str, url: &str, viewport: Viewport) -> Page {
    render_html_with_resources(html, external_css, HashMap::new(), url, viewport)
}

pub fn render_html_with_resources(
    html: &str,
    external_css: &str,
    images: HashMap<NodeId, Image>,
    url: &str,
    viewport: Viewport,
) -> Page {
    let mut document = flux_dom::parse_html(html);
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin(url);
    runtime.run_document_scripts(&mut document);

    let embedded = document.embedded_css();
    let combined_css = match (external_css.is_empty(), embedded.is_empty()) {
        (true, _) => embedded,
        (false, true) => external_css.to_string(),
        (false, false) => format!("{external_css}\n{embedded}"),
    };
    let sheet = parse_css(&combined_css);
    let styles = compute_styles(&document, &sheet);
    let layout = layout_document_interactive(&document, &styles, viewport.width, &images, None);
    let display_list = paint(&layout);
    Page {
        url: url.to_string(),
        title: document.title(),
        source: html.to_string(),
        external_css: external_css.to_string(),
        images,
        focused_input: None,
        document,
        runtime,
        layout,
        display_list,
    }
}

pub fn load_url(url: &str, viewport: Viewport) -> Result<Page, BrowserError> {
    let (final_url, response) = fetch(url).map_err(BrowserError::Net)?;
    if !(200..300).contains(&response.status) {
        return Err(BrowserError::Status(response.status));
    }
    let html = String::from_utf8(response.body).map_err(|_| BrowserError::Utf8)?;
    let parsed_doc = flux_dom::parse_html(&html);
    let external_css = fetch_linked_stylesheets(&final_url, &parsed_doc.stylesheet_links());
    let images = fetch_page_images(&final_url, &parsed_doc.images());
    Ok(render_html_with_resources(
        &html,
        &external_css,
        images,
        &final_url,
        viewport,
    ))
}

pub fn load_post_target(url: &str, body: &str, viewport: Viewport) -> Result<Page, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let (final_url, response) =
            fetch_post(url, "application/x-www-form-urlencoded", body.as_bytes())
                .map_err(|err| err.to_string())?;
        if !(200..300).contains(&response.status) {
            return Err(format!("the server returned {}", response.status));
        }
        let html =
            String::from_utf8(response.body).map_err(|_| "the page was not text".to_string())?;
        let parsed_doc = flux_dom::parse_html(&html);
        let external_css = fetch_linked_stylesheets(&final_url, &parsed_doc.stylesheet_links());
        let images = fetch_page_images(&final_url, &parsed_doc.images());
        Ok(render_html_with_resources(
            &html,
            &external_css,
            images,
            &final_url,
            viewport,
        ))
    } else {
        load_target(url, viewport)
    }
}

pub fn load_target(target: &str, viewport: Viewport) -> Result<Page, String> {
    if let Some(dest) = frontend::resolve_navigation_query(target) {
        return load_target(&dest, viewport);
    }
    if let Some((canonical_url, html)) = frontend::resolve_builtin_page(target) {
        return Ok(render_html(&html, &canonical_url, viewport));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        load_url(target, viewport).map_err(|err| err.to_string())
    } else {
        let html =
            fs::read_to_string(target).map_err(|err| format!("could not read {target}: {err}"))?;
        let parsed_doc = flux_dom::parse_html(&html);
        let external_css = fetch_linked_stylesheets(target, &parsed_doc.stylesheet_links());
        let images = fetch_page_images(target, &parsed_doc.images());
        Ok(render_html_with_resources(
            &html,
            &external_css,
            images,
            target,
            viewport,
        ))
    }
}

fn is_remote_origin(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

fn fetch_page_images(base: &str, imgs: &[(NodeId, String)]) -> HashMap<NodeId, Image> {
    let mut out = HashMap::new();
    let base_is_remote = is_remote_origin(base);
    for (node_id, src) in imgs {
        let Some(resolved) = resolve_url(base, src) else {
            continue;
        };
        let raw_bytes = if is_remote_origin(&resolved) {
            fetch(&resolved).ok().and_then(|(_, resp)| {
                if (200..300).contains(&resp.status) {
                    Some(resp.body)
                } else {
                    None
                }
            })
        } else if !base_is_remote && !base.starts_with("flux://") {
            fs::read(&resolved).ok()
        } else {
            None
        };
        if let Some(bytes) = raw_bytes {
            if let Ok(image) = decode_auto(&bytes) {
                out.insert(*node_id, image);
            }
        }
    }
    out
}

fn fetch_linked_stylesheets(base: &str, links: &[String]) -> String {
    let mut sheets = Vec::new();
    let base_is_remote = is_remote_origin(base);
    for href in links {
        let Some(resolved) = resolve_url(base, href) else {
            continue;
        };
        if is_remote_origin(&resolved) {
            if let Ok((_, response)) = fetch(&resolved) {
                if (200..300).contains(&response.status) {
                    if let Ok(css) = String::from_utf8(response.body) {
                        if !css.trim().is_empty() {
                            sheets.push(css);
                        }
                    }
                }
            }
        } else if !base_is_remote && !base.starts_with("flux://") {
            if let Ok(css) = fs::read_to_string(&resolved) {
                if !css.trim().is_empty() {
                    sheets.push(css);
                }
            }
        }
    }
    sheets.join("\n")
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

    pub fn current_mut(&mut self) -> Option<&mut Page> {
        self.history.get_mut(self.index)
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty() && self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    pub fn title(&self) -> &str {
        let Some(page) = self.current() else {
            return "New Tab";
        };
        if !page.title.is_empty() {
            page.title.as_str()
        } else if !page.url.is_empty() {
            page.url.as_str()
        } else {
            "New Tab"
        }
    }

    fn replace_current(&mut self, page: Page) {
        if let Some(slot) = self.history.get_mut(self.index) {
            *slot = page;
        }
    }

    fn navigate(&mut self, page: Page) {
        if !self.history.is_empty() {
            self.history.truncate(self.index + 1);
        }
        self.history.push(page);
        self.index = self.history.len() - 1;
    }

    fn back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.index -= 1;
        true
    }

    fn forward(&mut self) -> bool {
        if !self.can_go_forward() {
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
    scroll: f32,
    bookmarks: Vec<Bookmark>,
}

impl Session {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            tabs: vec![Tab::empty()],
            active: 0,
            viewport,
            scroll: 0.0,
            bookmarks: Vec::new(),
        }
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    pub fn max_scroll(&self) -> f32 {
        let Some(page) = self.active().current() else {
            return 0.0;
        };
        (page.content_height() - self.viewport.height).max(0.0)
    }

    pub fn scroll_by(&mut self, delta: f32) -> f32 {
        self.scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
        self.scroll
    }

    pub fn scroll_to_ratio(&mut self, ratio: f32) -> f32 {
        let max_s = self.max_scroll();
        self.scroll = (ratio.clamp(0.0, 1.0) * max_s).clamp(0.0, max_s);
        self.scroll
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        if let Some(current) = self.tabs[self.active].current_mut() {
            current.restage(self.viewport);
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    pub fn hit_link(&self, x: f32, y: f32) -> Option<String> {
        let page = self.active().current()?;
        let href = page.link_at(x, y + self.scroll)?;
        resolve_url(&page.url, href)
    }

    pub fn hit_text_input(&self, x: f32, y: f32) -> Option<NodeId> {
        let page = self.active().current()?;
        page.text_input_at(x, y + self.scroll)
    }

    pub fn has_focused_input(&self) -> bool {
        self.active()
            .current()
            .is_some_and(|p| p.focused_input.is_some())
    }

    pub fn blur_page_input(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if page.focused_input.take().is_some() {
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn type_into_focused_input(&mut self, ch: char) -> bool {
        if !(' '..='~').contains(&ch) {
            return false;
        }
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(id) = page.focused_input {
                let mut val = page.document.input_value(id);
                val.push(ch);
                page.document.set_input_value(id, &val);
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn backspace_focused_input(&mut self) -> bool {
        let viewport = self.viewport;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(id) = page.focused_input {
                let mut val = page.document.input_value(id);
                val.pop();
                page.document.set_input_value(id, &val);
                page.restage(viewport);
                return true;
            }
        }
        false
    }

    pub fn submit_focused_input(&mut self) -> ClickOutcome {
        let focused = self
            .tabs
            .get(self.active)
            .and_then(|t| t.current())
            .and_then(|p| p.focused_input);
        let Some(id) = focused else {
            return ClickOutcome::None;
        };
        self.blur_page_input();
        self.submit_form_for(id)
    }

    pub fn submit_form_for(&mut self, control_id: NodeId) -> ClickOutcome {
        let Some(page) = self.tabs[self.active].current() else {
            return ClickOutcome::None;
        };
        let Some(form_id) = page.document.enclosing_form(control_id) else {
            return ClickOutcome::None;
        };
        let (action, method, encoded) = page.document.serialize_form(form_id);
        let target_base = if action.is_empty() {
            page.url.clone()
        } else {
            resolve_url(&page.url, &action).unwrap_or(action)
        };
        if method == "POST" {
            ClickOutcome::SubmitPost {
                url: target_base,
                body: encoded,
            }
        } else {
            ClickOutcome::Navigate(append_query(&target_base, &encoded))
        }
    }

    pub fn click_at(&mut self, x: f32, y: f32) -> ClickOutcome {
        let page_y = y + self.scroll;
        let viewport = self.viewport;
        let mut submit_control = None;
        if let Some(page) = self.tabs[self.active].current_mut() {
            if let Some(node_id) = page.hit_test(x, page_y) {
                if page.runtime.dispatch_click(&mut page.document, node_id) {
                    page.restage(viewport);
                    self.scroll = self.scroll.clamp(0.0, self.max_scroll());
                    return ClickOutcome::ScriptHandled;
                }
                let mut cursor = Some(node_id);
                while let Some(curr) = cursor {
                    if page.document.is_text_input(curr) {
                        page.focused_input = Some(curr);
                        page.restage(viewport);
                        return ClickOutcome::InputFocused(curr);
                    }
                    if page.document.is_checkbox_input(curr) {
                        page.focused_input = None;
                        let checked = page.document.toggle_checked(curr);
                        page.restage(viewport);
                        return ClickOutcome::CheckboxToggled(curr, checked);
                    }
                    if page.document.is_submit_control(curr) {
                        page.focused_input = None;
                        submit_control = Some(curr);
                        break;
                    }
                    cursor = page.document.get(curr).parent();
                }
            }
            if page.focused_input.take().is_some() {
                page.restage(viewport);
            }
        }
        if let Some(ctrl_id) = submit_control {
            return self.submit_form_for(ctrl_id);
        }
        if let Some(target) = self.hit_link(x, y) {
            return ClickOutcome::Navigate(target);
        }
        ClickOutcome::None
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn is_active_bookmarked(&self) -> bool {
        let Some(page) = self.active().current() else {
            return false;
        };
        !page.url.is_empty() && self.bookmarks.iter().any(|b| b.url == page.url)
    }

    pub fn add_bookmark(&mut self, title: &str, url: &str) -> bool {
        let url = url.trim();
        if url.is_empty() || self.bookmarks.iter().any(|b| b.url == url) {
            return false;
        }
        let label = if title.trim().is_empty() {
            url.to_string()
        } else {
            title.trim().to_string()
        };
        self.bookmarks.push(Bookmark {
            title: label,
            url: url.to_string(),
        });
        true
    }

    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        self.bookmarks.len() != before
    }

    pub fn toggle_bookmark_active(&mut self) -> bool {
        let Some(page) = self.active().current() else {
            return false;
        };
        let url = page.url.clone();
        let title = page.title.clone();
        if url.is_empty() {
            return false;
        }
        if self.remove_bookmark(&url) {
            false
        } else {
            self.add_bookmark(&title, &url)
        }
    }

    pub fn rasterize(&self) -> Framebuffer {
        let width = (self.viewport.width.round() as u32).max(1);
        let height = (self.viewport.height.round() as u32).max(1);
        let Some(page) = self.active().current() else {
            return Framebuffer::new(width, height, PAPER);
        };
        rasterize(&page.display_list, width, height, self.scroll)
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
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
        self.scroll = 0.0;
        true
    }

    pub fn new_tab(&mut self) {
        self.tabs.push(Tab::empty());
        self.active = self.tabs.len() - 1;
        self.scroll = 0.0;
    }

    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return false;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.scroll = 0.0;
        true
    }

    pub fn open_page(&mut self, page: Page) {
        self.tabs[self.active].navigate(page);
        self.scroll = 0.0;
    }

    pub fn open_page_in_tab(&mut self, tab_index: usize, page: Page) {
        if let Some(tab) = self.tabs.get_mut(tab_index) {
            tab.navigate(page);
            if tab_index == self.active {
                self.scroll = 0.0;
            }
        }
    }

    pub fn open_html(&mut self, html: &str, url: &str) {
        let page = render_html(html, url, self.viewport);
        self.open_page(page);
    }

    pub fn open_url(&mut self, url: &str) -> Result<(), BrowserError> {
        let page = load_url(url, self.viewport)?;
        self.open_page(page);
        Ok(())
    }

    pub fn open_target(&mut self, target: &str) -> Result<(), String> {
        let page = load_target(target, self.viewport)?;
        self.open_page(page);
        Ok(())
    }

    pub fn reload_active(&mut self) -> Result<(), String> {
        let Some(current) = self.tabs[self.active].current() else {
            return Ok(());
        };
        let target = current.url.clone();
        if target.is_empty() {
            return Ok(());
        }
        let page = load_target(&target, self.viewport)?;
        self.tabs[self.active].replace_current(page);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        Ok(())
    }

    pub fn back(&mut self) -> bool {
        let moved = self.tabs[self.active].back();
        if moved {
            self.scroll = 0.0;
        }
        moved
    }

    pub fn forward(&mut self) -> bool {
        let moved = self.tabs[self.active].forward();
        if moved {
            self.scroll = 0.0;
        }
        moved
    }

    /// Export the current session tabs, active index, and bookmarks as formatted JSON (`flux-json`).
    pub fn export_json(&self) -> String {
        let mut root = JsonValue::empty_object();
        root.insert("version", JsonValue::String(VERSION.to_string()));
        root.insert("active_tab", JsonValue::Number(self.active as f64));
        let mut tabs_arr = JsonValue::empty_array();
        for tab in &self.tabs {
            let mut item = JsonValue::empty_object();
            item.insert("title", JsonValue::String(tab.title().to_string()));
            let url = tab.current().map(|p| p.url.clone()).unwrap_or_default();
            item.insert("url", JsonValue::String(url));
            tabs_arr.push(item);
        }
        root.insert("tabs", tabs_arr);

        let mut bm_arr = JsonValue::empty_array();
        for bm in &self.bookmarks {
            let mut item = JsonValue::empty_object();
            item.insert("title", JsonValue::String(bm.title.clone()));
            item.insert("url", JsonValue::String(bm.url.clone()));
            bm_arr.push(item);
        }
        root.insert("bookmarks", bm_arr);
        to_json_pretty(&root)
    }

    pub fn import_bookmarks_from_json(&mut self, json_src: &str) -> Result<usize, String> {
        let doc = parse_json(json_src).map_err(|e| e.to_string())?;
        let Some(items) = doc.get("bookmarks").and_then(JsonValue::as_array) else {
            return Ok(0);
        };
        let mut added = 0usize;
        for item in items {
            let title = item.get("title").and_then(JsonValue::as_str).unwrap_or("");
            let url = item.get("url").and_then(JsonValue::as_str).unwrap_or("");
            if self.add_bookmark(title, url) {
                added += 1;
            }
        }
        Ok(added)
    }

    pub fn save_profile(&self, path: &str) -> Result<(), String> {
        fs::write(path, self.export_json()).map_err(|e| e.to_string())
    }

    pub fn load_profile(&mut self, path: &str) -> Result<usize, String> {
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        self.import_bookmarks_from_json(&raw)
    }

    /// Parse exported session JSON and return the list of tab URLs.
    pub fn parse_exported_urls(json_src: &str) -> Result<Vec<String>, String> {
        let doc = parse_json(json_src).map_err(|e| e.to_string())?;
        let Some(tabs) = doc.get("tabs").and_then(JsonValue::as_array) else {
            return Err("missing tabs array in session JSON".to_string());
        };
        let mut urls = Vec::new();
        for tab in tabs {
            if let Some(url) = tab.get("url").and_then(JsonValue::as_str) {
                urls.push(url.to_string());
            }
        }
        Ok(urls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_paint::DisplayItem;
    use flux_style::Color;

    #[test]
    fn release_is_v05() {
        assert_eq!(VERSION, "v.05");
    }

    #[test]
    fn loads_builtin_flux_home_and_security_pages() {
        let vp = Viewport {
            width: 960.0,
            height: 720.0,
        };
        let home = load_target("flux://home", vp).expect("load flux://home");
        assert_eq!(home.url, "flux://home");
        assert_eq!(home.title, "Flux Start");

        let sec = load_target("flux://security", vp).expect("load flux://security");
        assert_eq!(sec.url, "flux://security");
        assert_eq!(sec.title, "Flux Security Center");

        let nav = load_target("flux://navigate?q=flux%3A%2F%2Fsecurity", vp)
            .expect("resolve flux://navigate query");
        assert_eq!(nav.url, "flux://security");
    }

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
    fn applies_external_stylesheet_before_embedded_rules() {
        let page = render_html_with_css(
            "<style>p { color: #00ff00; }</style><h1>Heading</h1><p>Body</p>",
            "h1 { color: #ff0000; } p { color: #0000ff; }",
            "index.html",
            Viewport {
                width: 400.0,
                height: 300.0,
            },
        );
        let texts: Vec<_> = page
            .display_list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, color, .. } => Some((text.as_str(), *color)),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&("Heading", Color::parse("#ff0000").unwrap())));
        assert!(texts.contains(&("Body", Color::parse("#00ff00").unwrap())));
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
    fn a_new_tab_starts_empty_and_can_close() {
        let mut session = Session::new(Viewport {
            width: 100.0,
            height: 100.0,
        });
        session.open_html("<title>Stay</title>", "stay");
        session.new_tab();
        assert_eq!(session.len(), 2);
        assert!(session.active().current().is_none());
        assert!(session.close_tab(1));
        assert_eq!(session.len(), 1);
        assert_eq!(session.active().current().unwrap().title, "Stay");
        assert!(!session.close_tab(0));
    }

    #[test]
    fn hits_a_relative_link_and_scrolls_the_viewport() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 40.0,
        });
        session.open_html(
            "<style>
                html, body, p { margin: 0; padding: 0; font-size: 16px; }
                div { height: 100px; }
             </style>
             <p><a href=\"second.html\">Next page</a></p>
             <div>Spacer</div>",
            "examples/hello.html",
        );
        assert_eq!(
            session.hit_link(4.0, 4.0),
            Some("examples/second.html".to_string())
        );
        assert!(session.scroll_by(50.0) > 0.0);
        assert_eq!(session.hit_link(4.0, 4.0), None);
        let fb = session.rasterize();
        assert_eq!(fb.width(), 200);
        assert_eq!(fb.height(), 40);
    }

    #[test]
    fn dispatches_script_click_and_restages_page() {
        let mut session = Session::new(Viewport {
            width: 200.0,
            height: 100.0,
        });
        session.open_html(
            "<style>html, body, p { margin: 0; padding: 0; font-size: 16px; }</style>
             <p id=\"btn\">Tap here</p>
             <script>
               let b = document.getElementById('btn');
               b.addEventListener('click', function() {
                 document.title = 'Tapped';
                 b.textContent = 'Done';
               });
             </script>",
            "test.html",
        );
        assert_eq!(session.click_at(4.0, 4.0), ClickOutcome::ScriptHandled);
        let page = session.active().current().unwrap();
        assert_eq!(page.title, "Tapped");
        assert_eq!(
            page.document
                .text_content(page.document.get_element_by_id("btn").unwrap()),
            "Done"
        );
    }

    #[test]
    fn exports_and_parses_session_tabs_via_flux_json() {
        let mut session = Session::new(Viewport {
            width: 400.0,
            height: 300.0,
        });
        session.open_html("<title>Alpha</title>", "https://example.com/a");
        session.new_tab();
        session.open_html("<title>Beta</title>", "https://example.com/b");

        let exported = session.export_json();
        let urls = Session::parse_exported_urls(&exported).unwrap();
        assert_eq!(
            urls,
            vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string()
            ]
        );
    }

    #[test]
    fn focuses_input_types_text_submits_form_and_persists_bookmarks() {
        let mut session = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        session.open_html(
            "<style>html, body, form, input { margin: 0; }</style>
             <form action=\"https://example.com/login\" method=\"POST\">
               <input id=\"user\" type=\"text\" name=\"user\" value=\"al\">
               <button id=\"send\" type=\"submit\">Send</button>
             </form>",
            "https://example.com/index.html",
        );

        let user_id = session
            .active()
            .current()
            .unwrap()
            .document
            .get_element_by_id("user")
            .unwrap();
        assert_eq!(
            session.click_at(16.0, 10.0),
            ClickOutcome::InputFocused(user_id)
        );
        assert!(session.has_focused_input());
        assert!(session.backspace_focused_input());
        assert!(session.type_into_focused_input('d'));
        assert!(session.type_into_focused_input('m'));
        assert_eq!(
            session.submit_focused_input(),
            ClickOutcome::SubmitPost {
                url: "https://example.com/login".to_string(),
                body: "user=adm".to_string(),
            }
        );

        assert!(session.toggle_bookmark_active());
        assert!(session.is_active_bookmarked());
        let json = session.export_json();
        let mut restored = Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        });
        assert_eq!(restored.import_bookmarks_from_json(&json).unwrap(), 1);
        assert_eq!(
            restored.bookmarks()[0].url,
            "https://example.com/index.html"
        );
    }
}
