// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Operating-system accessibility bridge for Anvil Browser (`v.28`).
//!
//! Exposes a clean-room, zero-dependency accessibility tree snapshot derived
//! from [`Session`] and [`UiState`], and bridges it to:
//! - **Windows UI Automation (`uiautomationcore.dll` / `WM_GETOBJECT`)**:
//!   Implements a COM [`IRawElementProviderSimple`] vtable (`IUnknown` +
//!   `get_ProviderOptions`, `GetPatternProvider`, `GetPropertyValue`,
//!   `get_HostRawElementProvider`) returned via `UiaReturnRawElementProvider`
//!   when `WM_GETOBJECT` (`0x003D`) is sent with `UiaRootObjectId` (`-25`), and
//!   emits `UiaRaiseAutomationEvent` (`UIA_AutomationFocusChangedEventId` =
//!   `20005`) when keyboard focus moves across toolbar controls, menus,
//!   popover cards, or page documents.
//! - **Linux AT-SPI2 / D-Bus Wire Protocol (`org.a11y.atspi.Event.Object`)**:
//!   Encodes little-endian D-Bus `SIGNAL` frames (`StateChanged` `"focused"`)
//!   with 8-byte header alignment and dispatches them over a non-blocking
//!   `UnixStream` when `AT_SPI_BUS_ADDRESS` or `DBUS_SESSION_BUS_ADDRESS` is
//!   configured in the environment.

use crate::Session;
use crate::keys::{Focus, focus_of};
use crate::ui::{
    AppMenuAction, ChromeControl, ContextMenuAction, SitePopoverAction, UiState,
    chrome_focus_order, content_viewport, top_header_height_for,
};
use anvil_dom::{Document, NodeData, NodeId};
use anvil_layout::{LayoutBox, Rect};
use std::sync::{Arc, Mutex, OnceLock};

/// Win32 `WM_GETOBJECT` window message constant (`0x003D`).
pub const WM_GETOBJECT: u32 = 0x003D;

/// UI Automation `UiaRootObjectId` (`lParam == -25` on `WM_GETOBJECT`).
pub const UIA_ROOT_OBJECT_ID: i32 = -25;

// UI Automation ControlType IDs.
pub const UIA_BUTTON_CONTROL_TYPE_ID: i32 = 50000;
pub const UIA_COMBO_BOX_CONTROL_TYPE_ID: i32 = 50003;
pub const UIA_EDIT_CONTROL_TYPE_ID: i32 = 50004;
pub const UIA_HYPERLINK_CONTROL_TYPE_ID: i32 = 50005;
pub const UIA_IMAGE_CONTROL_TYPE_ID: i32 = 50006;
pub const UIA_LIST_ITEM_CONTROL_TYPE_ID: i32 = 50007;
pub const UIA_LIST_CONTROL_TYPE_ID: i32 = 50008;
pub const UIA_MENU_CONTROL_TYPE_ID: i32 = 50009;
pub const UIA_MENU_ITEM_CONTROL_TYPE_ID: i32 = 50011;
pub const UIA_TAB_ITEM_CONTROL_TYPE_ID: i32 = 50019;
pub const UIA_TEXT_CONTROL_TYPE_ID: i32 = 50020;
pub const UIA_TOOL_BAR_CONTROL_TYPE_ID: i32 = 50021;
pub const UIA_GROUP_CONTROL_TYPE_ID: i32 = 50026;
pub const UIA_DOCUMENT_CONTROL_TYPE_ID: i32 = 50030;
pub const UIA_WINDOW_CONTROL_TYPE_ID: i32 = 50032;
pub const UIA_PANE_CONTROL_TYPE_ID: i32 = 50033;

// UI Automation Property IDs.
pub const UIA_CONTROL_TYPE_PROPERTY_ID: i32 = 30003;
pub const UIA_NAME_PROPERTY_ID: i32 = 30005;
pub const UIA_HAS_KEYBOARD_FOCUS_PROPERTY_ID: i32 = 30008;
pub const UIA_IS_KEYBOARD_FOCUSABLE_PROPERTY_ID: i32 = 30009;
pub const UIA_AUTOMATION_ID_PROPERTY_ID: i32 = 30011;
pub const UIA_IS_CONTROL_ELEMENT_PROPERTY_ID: i32 = 30016;
pub const UIA_IS_CONTENT_ELEMENT_PROPERTY_ID: i32 = 30017;
pub const UIA_VALUE_VALUE_PROPERTY_ID: i32 = 30045;

// UI Automation Event IDs.
pub const UIA_AUTOMATION_FOCUS_CHANGED_EVENT_ID: i32 = 20005;

/// Semantic role for an accessible browser chrome or document node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibleRole {
    Window,
    Toolbar,
    TabItem,
    Button,
    Edit,
    Menu,
    MenuItem,
    Document,
    Heading,
    Link,
    Image,
    Text,
    ComboBox,
    List,
    ListItem,
    Group,
    Dialog,
}

impl AccessibleRole {
    /// Windows UI Automation `UIA_*ControlTypeId` for this role.
    pub const fn uia_control_type_id(self) -> i32 {
        match self {
            Self::Window | Self::Dialog => UIA_WINDOW_CONTROL_TYPE_ID,
            Self::Toolbar => UIA_TOOL_BAR_CONTROL_TYPE_ID,
            Self::TabItem => UIA_TAB_ITEM_CONTROL_TYPE_ID,
            Self::Button => UIA_BUTTON_CONTROL_TYPE_ID,
            Self::Edit => UIA_EDIT_CONTROL_TYPE_ID,
            Self::Menu => UIA_MENU_CONTROL_TYPE_ID,
            Self::MenuItem => UIA_MENU_ITEM_CONTROL_TYPE_ID,
            Self::Document => UIA_DOCUMENT_CONTROL_TYPE_ID,
            Self::Heading | Self::Text => UIA_TEXT_CONTROL_TYPE_ID,
            Self::Link => UIA_HYPERLINK_CONTROL_TYPE_ID,
            Self::Image => UIA_IMAGE_CONTROL_TYPE_ID,
            Self::ComboBox => UIA_COMBO_BOX_CONTROL_TYPE_ID,
            Self::List => UIA_LIST_CONTROL_TYPE_ID,
            Self::ListItem => UIA_LIST_ITEM_CONTROL_TYPE_ID,
            Self::Group => UIA_GROUP_CONTROL_TYPE_ID,
        }
    }

    /// Linux AT-SPI2 role name string.
    pub const fn atspi_role_name(self) -> &'static str {
        match self {
            Self::Window => "frame",
            Self::Toolbar => "tool bar",
            Self::TabItem => "page tab",
            Self::Button => "push button",
            Self::Edit => "entry",
            Self::Menu => "menu",
            Self::MenuItem => "menu item",
            Self::Document => "document web",
            Self::Heading => "heading",
            Self::Link => "link",
            Self::Image => "image",
            Self::Text => "static",
            Self::ComboBox => "combo box",
            Self::List => "list",
            Self::ListItem => "list item",
            Self::Group => "panel",
            Self::Dialog => "dialog",
        }
    }
}

/// Accessible node exposed to platform screen readers and automation tools.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibleNode {
    pub automation_id: String,
    pub role: AccessibleRole,
    pub name: String,
    pub value: String,
    pub bounds: Rect,
    pub focusable: bool,
    pub focused: bool,
    pub expanded: Option<bool>,
    pub parent_index: Option<usize>,
    pub children: Vec<usize>,
    pub heading_level: Option<u8>,
}

/// Complete snapshot of the browser window's accessible tree and currently focused node.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityTreeSnapshot {
    pub window_title: String,
    pub focused_node: AccessibleNode,
    pub focused_index: Option<usize>,
    pub nodes: Vec<AccessibleNode>,
}

fn push_child_node(
    nodes: &mut Vec<AccessibleNode>,
    parent_idx: usize,
    mut node: AccessibleNode,
) -> usize {
    let idx = nodes.len();
    node.parent_index = Some(parent_idx);
    nodes.push(node);
    if let Some(parent) = nodes.get_mut(parent_idx) {
        parent.children.push(idx);
    }
    idx
}

fn collapse_a11y_text(raw: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut prev_ws = true;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
        if out.len() >= max_len {
            break;
        }
    }
    out.trim().to_string()
}

fn element_accessible_name(doc: &Document, id: NodeId, fallback: &str) -> String {
    let data = doc.get(id).data();
    for key in ["aria-label", "alt", "title", "placeholder"] {
        if let Some(val) = data.attr(key) {
            let trimmed = collapse_a11y_text(val, 240);
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    if let Some(val) = data.attr("value")
        && matches!(data.tag(), Some("input" | "button"))
    {
        let trimmed = collapse_a11y_text(val, 240);
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let text = collapse_a11y_text(&doc.text_content(id), 240);
    if !text.is_empty() {
        text
    } else {
        fallback.to_string()
    }
}

fn map_dom_element_role(tag: &str, data: &NodeData) -> Option<(AccessibleRole, bool, Option<u8>)> {
    if let Some(role_attr) = data.attr("role") {
        match role_attr.trim().to_ascii_lowercase().as_str() {
            "button" => return Some((AccessibleRole::Button, true, None)),
            "link" => return Some((AccessibleRole::Link, true, None)),
            "textbox" | "searchbox" => return Some((AccessibleRole::Edit, true, None)),
            "combobox" => return Some((AccessibleRole::ComboBox, true, None)),
            "heading" => return Some((AccessibleRole::Heading, false, Some(2))),
            "img" => return Some((AccessibleRole::Image, false, None)),
            "list" => return Some((AccessibleRole::List, false, None)),
            "listitem" => return Some((AccessibleRole::ListItem, false, None)),
            "dialog" | "alertdialog" => return Some((AccessibleRole::Dialog, false, None)),
            "menu" => return Some((AccessibleRole::Menu, false, None)),
            "menuitem" => return Some((AccessibleRole::MenuItem, true, None)),
            "navigation" | "main" | "region" | "group" | "banner" | "contentinfo" => {
                return Some((AccessibleRole::Group, false, None));
            }
            "presentation" | "none" => return None,
            _ => {}
        }
    }
    match tag {
        "h1" => Some((AccessibleRole::Heading, false, Some(1))),
        "h2" => Some((AccessibleRole::Heading, false, Some(2))),
        "h3" => Some((AccessibleRole::Heading, false, Some(3))),
        "h4" => Some((AccessibleRole::Heading, false, Some(4))),
        "h5" => Some((AccessibleRole::Heading, false, Some(5))),
        "h6" => Some((AccessibleRole::Heading, false, Some(6))),
        "a" if data.attr("href").is_some() => Some((AccessibleRole::Link, true, None)),
        "button" | "summary" => Some((AccessibleRole::Button, true, None)),
        "input" => {
            let ty = data.attr("type").unwrap_or("text").to_ascii_lowercase();
            match ty.as_str() {
                "hidden" => None,
                "button" | "submit" | "reset" | "checkbox" | "radio" => {
                    Some((AccessibleRole::Button, true, None))
                }
                _ => Some((AccessibleRole::Edit, true, None)),
            }
        }
        "textarea" => Some((AccessibleRole::Edit, true, None)),
        "select" => Some((AccessibleRole::ComboBox, true, None)),
        "img" => Some((AccessibleRole::Image, false, None)),
        "ul" | "ol" => Some((AccessibleRole::List, false, None)),
        "li" => Some((AccessibleRole::ListItem, false, None)),
        "dialog" => Some((AccessibleRole::Dialog, false, None)),
        "nav" | "main" | "header" | "footer" | "aside" | "section" | "article" | "form" => {
            Some((AccessibleRole::Group, false, None))
        }
        "p" | "label" | "blockquote" | "figcaption" | "pre" | "code" => {
            Some((AccessibleRole::Text, false, None))
        }
        _ if data.attr("aria-label").is_some() => Some((AccessibleRole::Group, false, None)),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_dom_accessible_children(
    doc: &Document,
    layout: &LayoutBox,
    node_id: NodeId,
    parent_idx: usize,
    parent_bounds: Rect,
    header_h: f32,
    scroll_y: f32,
    focused_input: Option<NodeId>,
    focus: Focus,
    nodes: &mut Vec<AccessibleNode>,
    max_nodes: usize,
) {
    if nodes.len() >= max_nodes {
        return;
    }
    let data = doc.get(node_id).data();
    let mut next_parent_idx = parent_idx;
    let mut next_parent_bounds = parent_bounds;

    if let NodeData::Element { tag, .. } = data {
        let tag_l = tag.as_str();
        if matches!(
            tag_l,
            "head" | "script" | "style" | "noscript" | "template" | "meta" | "link" | "title"
        ) || data.attr("hidden").is_some()
            || data
                .attr("aria-hidden")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        {
            return;
        }
        let layout_rect = layout.find_node_box(node_id).map(|b| Rect {
            x: b.rect.x,
            y: header_h + b.rect.y - scroll_y,
            width: b.rect.width,
            height: b.rect.height,
        });
        if let Some((role, focusable, heading_level)) = map_dom_element_role(tag_l, data) {
            let bounds = layout_rect.unwrap_or(parent_bounds);
            let name = element_accessible_name(doc, node_id, tag_l);
            let value = match role {
                AccessibleRole::Link => data.attr("href").unwrap_or("").to_string(),
                AccessibleRole::Edit | AccessibleRole::ComboBox => {
                    data.attr("value").unwrap_or("").to_string()
                }
                _ => String::new(),
            };
            let is_focused = focus == Focus::PageField && focused_input == Some(node_id);
            let should_emit = !name.is_empty()
                || !value.is_empty()
                || focusable
                || matches!(role, AccessibleRole::Image | AccessibleRole::Dialog);
            if should_emit && (bounds.width > 0.0 || bounds.height > 0.0 || focusable) {
                next_parent_idx = push_child_node(
                    nodes,
                    parent_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.document.dom.{}", node_id.index()),
                        role,
                        name,
                        value,
                        bounds,
                        focusable,
                        focused: is_focused,
                        expanded: data
                            .attr("aria-expanded")
                            .map(|v| v.eq_ignore_ascii_case("true")),
                        parent_index: None,
                        children: Vec::new(),
                        heading_level,
                    },
                );
                next_parent_bounds = bounds;
            }
        } else if let Some(rect) = layout_rect {
            next_parent_bounds = rect;
        }
    }

    for &child in doc.children(node_id) {
        if nodes.len() >= max_nodes {
            break;
        }
        collect_dom_accessible_children(
            doc,
            layout,
            child,
            next_parent_idx,
            next_parent_bounds,
            header_h,
            scroll_y,
            focused_input,
            focus,
            nodes,
            max_nodes,
        );
    }
}

impl AccessibilityTreeSnapshot {
    /// Build a live accessibility snapshot from the current [`Session`] and [`UiState`].
    pub fn from_session_and_ui(session: &Session, ui: &UiState, width: f32, height: f32) -> Self {
        let focus = focus_of(session, ui);
        let active_page = session.active().current();
        let current_url = active_page
            .map(|p| p.url.as_str())
            .unwrap_or("anvil://home");
        let page_title = active_page
            .map(|p| {
                if p.title.trim().is_empty() {
                    p.url.as_str()
                } else {
                    p.title.trim()
                }
            })
            .unwrap_or("New Tab");
        let window_title = format!("{page_title} - Anvil Browser");
        let header_h = top_header_height_for(session, ui);

        let focused_node = compute_focused_node(
            session,
            ui,
            focus,
            current_url,
            page_title,
            width,
            height,
            header_h,
        );

        let mut nodes = Vec::new();
        nodes.push(AccessibleNode {
            automation_id: "anvil.window".to_string(),
            role: AccessibleRole::Window,
            name: window_title.clone(),
            value: current_url.to_string(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            focusable: true,
            focused: false,
            expanded: None,
            parent_index: None,
            children: Vec::new(),
            heading_level: None,
        });

        let toolbar_idx = push_child_node(
            &mut nodes,
            0,
            AccessibleNode {
                automation_id: "anvil.toolbar".to_string(),
                role: AccessibleRole::Toolbar,
                name: "Browser toolbar".to_string(),
                value: String::new(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: header_h,
                },
                focusable: false,
                focused: false,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            },
        );

        for ctrl in chrome_focus_order(session, ui, width) {
            let is_focused = (focus == Focus::Toolbar && ui.chrome_focus() == Some(ctrl))
                || (focus == Focus::Address && matches!(ctrl, ChromeControl::Address));
            let role = match ctrl {
                ChromeControl::Tab(_) => AccessibleRole::TabItem,
                ChromeControl::Address => AccessibleRole::Edit,
                _ => AccessibleRole::Button,
            };
            let expanded = match ctrl {
                ChromeControl::SiteInfo | ChromeControl::Shields => Some(ui.is_site_popover_open()),
                ChromeControl::Downloads => Some(ui.is_downloads_popover_open()),
                ChromeControl::AppMenu => Some(ui.is_app_menu_open()),
                ChromeControl::TabSearch => Some(ui.is_tab_search_open()),
                ChromeControl::Address if focus == Focus::Address => {
                    Some(!ui.omnibox_suggestions(session).is_empty())
                }
                _ => None,
            };
            push_child_node(
                &mut nodes,
                toolbar_idx,
                AccessibleNode {
                    automation_id: format!("anvil.toolbar.{ctrl:?}"),
                    role,
                    name: ctrl.accessible_name(session),
                    value: if matches!(ctrl, ChromeControl::Address) {
                        if focus == Focus::Address {
                            ui.input().to_string()
                        } else {
                            current_url.to_string()
                        }
                    } else {
                        String::new()
                    },
                    bounds: ctrl.rect(session, width),
                    focusable: true,
                    focused: is_focused,
                    expanded,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
        }

        let vp = content_viewport(width, height);
        let doc_bounds = Rect {
            x: 0.0,
            y: header_h,
            width: vp.width,
            height: vp.height,
        };
        let doc_idx = push_child_node(
            &mut nodes,
            0,
            AccessibleNode {
                automation_id: "anvil.document".to_string(),
                role: AccessibleRole::Document,
                name: page_title.to_string(),
                value: current_url.to_string(),
                bounds: doc_bounds,
                focusable: true,
                focused: focus == Focus::Page,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            },
        );

        if let Some(page) = active_page {
            collect_dom_accessible_children(
                &page.document,
                &page.layout,
                page.document.root(),
                doc_idx,
                doc_bounds,
                header_h,
                page.scroll_y,
                page.focused_input,
                focus,
                &mut nodes,
                512,
            );
        }

        // Open menus, popovers, and find/inspector overlays are mounted after toolbar & document
        // so hit-testing resolves top-most overlays first.
        if ui.is_app_menu_open() {
            let menu_bounds = Rect {
                x: (width - 252.0).max(8.0),
                y: header_h + 26.0,
                width: 244.0,
                height: (AppMenuAction::ALL.len() as f32) * 26.0 + 8.0,
            };
            let menu_idx = push_child_node(
                &mut nodes,
                0,
                AccessibleNode {
                    automation_id: "anvil.menu.app".to_string(),
                    role: AccessibleRole::Menu,
                    name: "Application menu".to_string(),
                    value: String::new(),
                    bounds: menu_bounds,
                    focusable: true,
                    focused: false,
                    expanded: Some(true),
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
            let sel = ui.menu_index() % AppMenuAction::ALL.len();
            for (idx, action) in AppMenuAction::ALL.iter().enumerate() {
                let value = if *action == AppMenuAction::CycleZoom {
                    format!("{}%", session.zoom_percent())
                } else {
                    action.shortcut().to_string()
                };
                push_child_node(
                    &mut nodes,
                    menu_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.menu.app.{idx}"),
                        role: AccessibleRole::MenuItem,
                        name: format!("App menu: {}", action.label()),
                        value,
                        bounds: Rect {
                            x: menu_bounds.x,
                            y: header_h + 30.0 + (idx as f32) * 26.0,
                            width: 244.0,
                            height: 26.0,
                        },
                        focusable: true,
                        focused: focus == Focus::Menu && idx == sel,
                        expanded: None,
                        parent_index: None,
                        children: Vec::new(),
                        heading_level: None,
                    },
                );
            }
        } else if let Some((cx, cy)) = ui.context_menu_pos() {
            let items = ContextMenuAction::items_for(ui.context_kind());
            let menu_bounds = Rect {
                x: cx,
                y: cy,
                width: 228.0,
                height: (items.len().max(1) as f32) * 26.0 + 8.0,
            };
            let menu_idx = push_child_node(
                &mut nodes,
                0,
                AccessibleNode {
                    automation_id: "anvil.menu.context".to_string(),
                    role: AccessibleRole::Menu,
                    name: "Context menu".to_string(),
                    value: String::new(),
                    bounds: menu_bounds,
                    focusable: true,
                    focused: false,
                    expanded: Some(true),
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
            let sel = ui.menu_index() % items.len().max(1);
            for (idx, action) in items.iter().enumerate() {
                push_child_node(
                    &mut nodes,
                    menu_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.menu.context.{idx}"),
                        role: AccessibleRole::MenuItem,
                        name: format!("Context menu: {}", action.label()),
                        value: action.shortcut().to_string(),
                        bounds: Rect {
                            x: cx,
                            y: cy + 4.0 + (idx as f32) * 26.0,
                            width: 228.0,
                            height: 26.0,
                        },
                        focusable: true,
                        focused: focus == Focus::Menu && idx == sel,
                        expanded: None,
                        parent_index: None,
                        children: Vec::new(),
                        heading_level: None,
                    },
                );
            }
        } else if ui.is_site_popover_open() {
            let pop_bounds = Rect {
                x: 104.0,
                y: header_h + 48.0,
                width: 300.0,
                height: (SitePopoverAction::ALL.len() as f32) * 28.0 + 8.0,
            };
            let pop_idx = push_child_node(
                &mut nodes,
                0,
                AccessibleNode {
                    automation_id: "anvil.popover.site".to_string(),
                    role: AccessibleRole::Menu,
                    name: "Site information".to_string(),
                    value: current_url.to_string(),
                    bounds: pop_bounds,
                    focusable: true,
                    focused: false,
                    expanded: Some(true),
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
            let sel = ui.menu_index() % SitePopoverAction::ALL.len();
            for (idx, action) in SitePopoverAction::ALL.iter().enumerate() {
                push_child_node(
                    &mut nodes,
                    pop_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.popover.site.{idx}"),
                        role: AccessibleRole::MenuItem,
                        name: format!("Site information: {}", action.label()),
                        value: action.status_badge(session, current_url),
                        bounds: Rect {
                            x: 104.0,
                            y: header_h + 52.0 + (idx as f32) * 28.0,
                            width: 300.0,
                            height: 28.0,
                        },
                        focusable: true,
                        focused: focus == Focus::Menu && idx == sel,
                        expanded: None,
                        parent_index: None,
                        children: Vec::new(),
                        heading_level: None,
                    },
                );
            }
        } else if ui.is_downloads_popover_open() {
            let count = ui.open_menu_count_with_session(session).max(1);
            let pop_bounds = Rect {
                x: (width - 332.0).max(8.0),
                y: header_h + 44.0,
                width: 320.0,
                height: (count as f32) * 28.0 + 8.0,
            };
            let pop_idx = push_child_node(
                &mut nodes,
                0,
                AccessibleNode {
                    automation_id: "anvil.popover.downloads".to_string(),
                    role: AccessibleRole::Menu,
                    name: "Downloads".to_string(),
                    value: String::new(),
                    bounds: pop_bounds,
                    focusable: true,
                    focused: false,
                    expanded: Some(true),
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
            let sel = ui.menu_index() % count;
            let entries = session.downloads().entries();
            for idx in 0..count {
                let label = if idx == count.saturating_sub(1) {
                    "Open Full Downloads Vault".to_string()
                } else if idx == count.saturating_sub(2) {
                    "Save Active Page to Vault".to_string()
                } else if entries.is_empty() {
                    "No files downloaded in this session".to_string()
                } else {
                    entries
                        .iter()
                        .rev()
                        .nth(idx)
                        .map(|e| e.file_name.clone())
                        .unwrap_or_else(|| "Show in folder".to_string())
                };
                push_child_node(
                    &mut nodes,
                    pop_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.popover.downloads.{idx}"),
                        role: AccessibleRole::MenuItem,
                        name: format!("Downloads: {label}"),
                        value: format!("Row {} of {count}", idx + 1),
                        bounds: Rect {
                            x: (width - 332.0).max(8.0),
                            y: header_h + 48.0 + (idx as f32) * 28.0,
                            width: 320.0,
                            height: 28.0,
                        },
                        focusable: true,
                        focused: focus == Focus::Menu && idx == sel,
                        expanded: None,
                        parent_index: None,
                        children: Vec::new(),
                        heading_level: None,
                    },
                );
            }
        } else if ui.is_tab_search_open() {
            let matches = ui.filtered_tabs(session);
            let sel = ui.tab_search_selected();
            let menu_bounds = Rect {
                x: ChromeControl::TabSearch.rect(session, width).x,
                y: header_h,
                width: 320.0,
                height: (matches.len().max(1) as f32) * 28.0 + 36.0,
            };
            let ts_idx = push_child_node(
                &mut nodes,
                0,
                AccessibleNode {
                    automation_id: "anvil.tab_search".to_string(),
                    role: AccessibleRole::Menu,
                    name: "Tab search filter".to_string(),
                    value: ui.tab_search_query().to_string(),
                    bounds: menu_bounds,
                    focusable: true,
                    focused: focus == Focus::TabSearch && matches.is_empty(),
                    expanded: Some(true),
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                },
            );
            for (idx, (_, title, url)) in matches.iter().enumerate() {
                push_child_node(
                    &mut nodes,
                    ts_idx,
                    AccessibleNode {
                        automation_id: format!("anvil.tab_search.{idx}"),
                        role: AccessibleRole::MenuItem,
                        name: format!("Tab search: {title}"),
                        value: url.clone(),
                        bounds: Rect {
                            x: menu_bounds.x,
                            y: menu_bounds.y + 32.0 + (idx as f32) * 28.0,
                            width: 320.0,
                            height: 28.0,
                        },
                        focusable: true,
                        focused: focus == Focus::TabSearch && idx == sel,
                        expanded: None,
                        parent_index: None,
                        children: Vec::new(),
                        heading_level: None,
                    },
                );
            }
        }

        if matches!(focus, Focus::Find | Focus::Console | Focus::StyleOverride) {
            push_child_node(&mut nodes, 0, focused_node.clone());
        }

        let focused_index = nodes
            .iter()
            .position(|n| n.focused)
            .or_else(|| {
                nodes
                    .iter()
                    .position(|n| n.automation_id == focused_node.automation_id)
            })
            .or(Some(doc_idx));

        Self {
            window_title,
            focused_node,
            focused_index,
            nodes,
        }
    }

    /// Return the node at `index`, if present.
    pub fn node(&self, index: usize) -> Option<&AccessibleNode> {
        self.nodes.get(index)
    }

    /// Return the parent index of `index`.
    pub fn parent_of(&self, index: usize) -> Option<usize> {
        self.nodes.get(index).and_then(|n| n.parent_index)
    }

    /// Return the first child index of `index`.
    pub fn first_child_of(&self, index: usize) -> Option<usize> {
        self.nodes
            .get(index)
            .and_then(|n| n.children.first().copied())
    }

    /// Return the last child index of `index`.
    pub fn last_child_of(&self, index: usize) -> Option<usize> {
        self.nodes
            .get(index)
            .and_then(|n| n.children.last().copied())
    }

    /// Return the next sibling index of `index`.
    pub fn next_sibling_of(&self, index: usize) -> Option<usize> {
        let parent_idx = self.parent_of(index)?;
        let siblings = &self.nodes.get(parent_idx)?.children;
        let pos = siblings.iter().position(|&c| c == index)?;
        siblings.get(pos + 1).copied()
    }

    /// Return the previous sibling index of `index`.
    pub fn prev_sibling_of(&self, index: usize) -> Option<usize> {
        let parent_idx = self.parent_of(index)?;
        let siblings = &self.nodes.get(parent_idx)?.children;
        let pos = siblings.iter().position(|&c| c == index)?;
        pos.checked_sub(1).and_then(|p| siblings.get(p).copied())
    }

    /// Look up the tree node index matching an `automation_id`.
    pub fn find_by_automation_id(&self, automation_id: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.automation_id == automation_id)
    }

    /// Hit-test logical window coordinates `(x, y)` to the deepest matching child node.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        fn contains(r: Rect, x: f32, y: f32) -> bool {
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        }
        fn visit(snap: &AccessibilityTreeSnapshot, idx: usize, x: f32, y: f32) -> Option<usize> {
            let node = snap.nodes.get(idx)?;
            if !contains(node.bounds, x, y) {
                return None;
            }
            for &child in node.children.iter().rev() {
                if let Some(hit) = visit(snap, child, x, y) {
                    return Some(hit);
                }
            }
            Some(idx)
        }
        visit(self, 0, x, y)
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_focused_node(
    session: &Session,
    ui: &UiState,
    focus: Focus,
    current_url: &str,
    page_title: &str,
    width: f32,
    height: f32,
    header_h: f32,
) -> AccessibleNode {
    match focus {
        Focus::Menu => {
            if ui.is_app_menu_open() {
                let count = AppMenuAction::ALL.len();
                let idx = ui.menu_index() % count;
                let action = AppMenuAction::ALL[idx];
                let value = if action == AppMenuAction::CycleZoom {
                    format!("{}%", session.zoom_percent())
                } else {
                    action.shortcut().to_string()
                };
                return AccessibleNode {
                    automation_id: format!("anvil.menu.app.{idx}"),
                    role: AccessibleRole::MenuItem,
                    name: format!("App menu: {}", action.label()),
                    value,
                    bounds: Rect {
                        x: (width - 252.0).max(8.0),
                        y: header_h + 30.0 + (idx as f32) * 26.0,
                        width: 244.0,
                        height: 26.0,
                    },
                    focusable: true,
                    focused: true,
                    expanded: None,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                };
            }
            if let Some((cx, cy)) = ui.context_menu_pos() {
                let items = ContextMenuAction::items_for(ui.context_kind());
                let count = items.len().max(1);
                let idx = ui.menu_index() % count;
                let action = items.get(idx).copied().unwrap_or(ContextMenuAction::Reload);
                return AccessibleNode {
                    automation_id: format!("anvil.menu.context.{idx}"),
                    role: AccessibleRole::MenuItem,
                    name: format!("Context menu: {}", action.label()),
                    value: action.shortcut().to_string(),
                    bounds: Rect {
                        x: cx,
                        y: cy + 4.0 + (idx as f32) * 26.0,
                        width: 228.0,
                        height: 26.0,
                    },
                    focusable: true,
                    focused: true,
                    expanded: None,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                };
            }
            if ui.is_site_popover_open() {
                let count = SitePopoverAction::ALL.len();
                let idx = ui.menu_index() % count;
                let action = SitePopoverAction::ALL[idx];
                let badge = action.status_badge(session, current_url);
                return AccessibleNode {
                    automation_id: format!("anvil.popover.site.{idx}"),
                    role: AccessibleRole::MenuItem,
                    name: format!("Site information: {}", action.label()),
                    value: badge,
                    bounds: Rect {
                        x: 104.0,
                        y: header_h + 52.0 + (idx as f32) * 28.0,
                        width: 300.0,
                        height: 28.0,
                    },
                    focusable: true,
                    focused: true,
                    expanded: None,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                };
            }
            if ui.is_downloads_popover_open() {
                let count = ui.open_menu_count_with_session(session).max(1);
                let idx = ui.menu_index() % count;
                let entries = session.downloads().entries();
                let label = if idx == count.saturating_sub(1) {
                    "Open Full Downloads Vault".to_string()
                } else if idx == count.saturating_sub(2) {
                    "Save Active Page to Vault".to_string()
                } else if entries.is_empty() {
                    "No files downloaded in this session".to_string()
                } else {
                    entries
                        .iter()
                        .rev()
                        .nth(idx)
                        .map(|e| e.file_name.clone())
                        .unwrap_or_else(|| "Show in folder".to_string())
                };
                return AccessibleNode {
                    automation_id: format!("anvil.popover.downloads.{idx}"),
                    role: AccessibleRole::MenuItem,
                    name: format!("Downloads: {label}"),
                    value: format!("Row {} of {count}", idx + 1),
                    bounds: Rect {
                        x: (width - 332.0).max(8.0),
                        y: header_h + 48.0 + (idx as f32) * 28.0,
                        width: 320.0,
                        height: 28.0,
                    },
                    focusable: true,
                    focused: true,
                    expanded: None,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                };
            }
        }
        Focus::Toolbar => {
            if let Some(ctrl) = ui.chrome_focus() {
                let role = match ctrl {
                    ChromeControl::Tab(_) => AccessibleRole::TabItem,
                    ChromeControl::Address => AccessibleRole::Edit,
                    _ => AccessibleRole::Button,
                };
                return AccessibleNode {
                    automation_id: format!("anvil.toolbar.{ctrl:?}"),
                    role,
                    name: ctrl.accessible_name(session),
                    value: if matches!(ctrl, ChromeControl::Address) {
                        current_url.to_string()
                    } else {
                        String::new()
                    },
                    bounds: ctrl.rect(session, width),
                    focusable: true,
                    focused: true,
                    expanded: None,
                    parent_index: None,
                    children: Vec::new(),
                    heading_level: None,
                };
            }
        }
        Focus::Address => {
            return AccessibleNode {
                automation_id: "anvil.omnibox".to_string(),
                role: AccessibleRole::Edit,
                name: "Address and search bar".to_string(),
                value: ui.input().to_string(),
                bounds: ChromeControl::Address.rect(session, width),
                focusable: true,
                focused: true,
                expanded: Some(!ui.omnibox_suggestions(session).is_empty()),
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::TabSearch => {
            let matches = ui.filtered_tabs(session);
            let sel = ui.tab_search_selected();
            let (name, value) = matches
                .get(sel)
                .map(|(_, t, b)| (format!("Tab search: {t}"), b.clone()))
                .unwrap_or_else(|| {
                    (
                        "Tab search filter".to_string(),
                        ui.tab_search_query().to_string(),
                    )
                });
            return AccessibleNode {
                automation_id: "anvil.tab_search".to_string(),
                role: if matches.is_empty() {
                    AccessibleRole::Edit
                } else {
                    AccessibleRole::MenuItem
                },
                name,
                value,
                bounds: ChromeControl::TabSearch.rect(session, width),
                focusable: true,
                focused: true,
                expanded: Some(true),
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::Find => {
            return AccessibleNode {
                automation_id: "anvil.find".to_string(),
                role: AccessibleRole::Edit,
                name: "Find in page".to_string(),
                value: ui.find_query().to_string(),
                bounds: Rect {
                    x: (width - 332.0).max(8.0),
                    y: header_h + 6.0,
                    width: 320.0,
                    height: 32.0,
                },
                focusable: true,
                focused: true,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::Console => {
            return AccessibleNode {
                automation_id: "anvil.inspector.console".to_string(),
                role: AccessibleRole::Edit,
                name: "Inspector JavaScript console input".to_string(),
                value: String::new(),
                bounds: Rect {
                    x: 0.0,
                    y: (height - 180.0).max(header_h),
                    width,
                    height: 28.0,
                },
                focusable: true,
                focused: true,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::StyleOverride => {
            return AccessibleNode {
                automation_id: "anvil.inspector.style".to_string(),
                role: AccessibleRole::Edit,
                name: "Inspector inline CSS declaration input".to_string(),
                value: String::new(),
                bounds: Rect {
                    x: 0.0,
                    y: (height - 180.0).max(header_h),
                    width,
                    height: 28.0,
                },
                focusable: true,
                focused: true,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::PageField => {
            return AccessibleNode {
                automation_id: "anvil.page.field".to_string(),
                role: AccessibleRole::Edit,
                name: format!("Form input on {page_title}"),
                value: current_url.to_string(),
                bounds: Rect {
                    x: 0.0,
                    y: header_h,
                    width,
                    height: (height - header_h).max(1.0),
                },
                focusable: true,
                focused: true,
                expanded: None,
                parent_index: None,
                children: Vec::new(),
                heading_level: None,
            };
        }
        Focus::Page => {}
    }

    let vp = content_viewport(width, height);
    AccessibleNode {
        automation_id: "anvil.document".to_string(),
        role: AccessibleRole::Document,
        name: page_title.to_string(),
        value: current_url.to_string(),
        bounds: Rect {
            x: 0.0,
            y: header_h,
            width: vp.width,
            height: vp.height,
        },
        focusable: true,
        focused: true,
        expanded: None,
        parent_index: None,
        children: Vec::new(),
        heading_level: None,
    }
}

#[derive(Default)]
struct A11yBridgeState {
    hwnd: isize,
    last_focus_signature: String,
    last_tree_signature: String,
    snapshot: Option<Arc<AccessibilityTreeSnapshot>>,
    serial: u32,
    uia_queried: bool,
}

static A11Y_STATE: OnceLock<Mutex<A11yBridgeState>> = OnceLock::new();

fn bridge_state() -> &'static Mutex<A11yBridgeState> {
    A11Y_STATE.get_or_init(|| Mutex::new(A11yBridgeState::default()))
}

/// Synchronize the OS accessibility bridge with the latest [`Session`] and [`UiState`],
/// emitting a platform focus-changed or children-changed event whenever focus or tree structure updates.
pub fn notify_focus_if_changed(
    hwnd: isize,
    session: &Session,
    ui: &UiState,
    width: u32,
    height: u32,
) {
    let snapshot = Arc::new(AccessibilityTreeSnapshot::from_session_and_ui(
        session,
        ui,
        width as f32,
        height as f32,
    ));
    let sig = format!(
        "{:?}|{}|{}|{}",
        snapshot.focused_node.role,
        snapshot.focused_node.automation_id,
        snapshot.focused_node.name,
        snapshot.focused_node.value
    );
    let tree_sig = format!(
        "{}|{}|{}",
        snapshot.nodes.len(),
        snapshot.window_title,
        snapshot
            .nodes
            .first()
            .map(|n| n.children.len())
            .unwrap_or(0)
    );
    let (changed, tree_changed, serial, uia_queried, focused_clone, first_child_clone) = {
        let Ok(mut guard) = bridge_state().lock() else {
            return;
        };
        guard.hwnd = hwnd;
        let changed = guard.last_focus_signature != sig;
        let tree_changed = guard.last_tree_signature != tree_sig;
        if changed || tree_changed {
            guard.serial = guard.serial.wrapping_add(1).max(1);
        }
        if changed {
            guard.last_focus_signature = sig;
        }
        if tree_changed {
            guard.last_tree_signature = tree_sig;
        }
        let serial = guard.serial;
        let uia_queried = guard.uia_queried;
        let focused_clone = snapshot.focused_node.clone();
        let first_child_clone = snapshot
            .nodes
            .first()
            .and_then(|root| root.children.last().and_then(|&c| snapshot.nodes.get(c)))
            .cloned();
        guard.snapshot = Some(Arc::clone(&snapshot));
        (
            changed,
            tree_changed,
            serial,
            uia_queried,
            focused_clone,
            first_child_clone,
        )
    };

    if !changed && !tree_changed {
        return;
    }

    #[cfg(target_os = "windows")]
    if uia_queried && changed {
        win32_uia::raise_focus_changed_event();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = uia_queried;

    #[cfg(target_os = "linux")]
    {
        if changed {
            linux_atspi::emit_focus_signal_nonblocking(serial, &focused_clone);
        }
        if tree_changed && let Some(child) = first_child_clone.as_ref() {
            linux_atspi::emit_children_changed_nonblocking(
                serial.wrapping_add(1).max(1),
                "anvil.window",
                0,
                true,
                child,
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (serial, focused_clone, first_child_clone);
}

/// Handle Win32 `WM_GETOBJECT` (`0x003D`) by updating the live accessibility snapshot
/// and returning an `IRawElementProviderSimple` pointer via `UiaReturnRawElementProvider`.
#[cfg(target_os = "windows")]
pub fn handle_wm_getobject(
    hwnd: isize,
    w_param: usize,
    l_param: isize,
    session: &Session,
    ui: &UiState,
    width: u32,
    height: u32,
) -> Option<isize> {
    if (l_param as i32) != UIA_ROOT_OBJECT_ID {
        return None;
    }
    let snapshot = Arc::new(AccessibilityTreeSnapshot::from_session_and_ui(
        session,
        ui,
        width as f32,
        height as f32,
    ));
    if let Ok(mut guard) = bridge_state().lock() {
        guard.hwnd = hwnd;
        guard.uia_queried = true;
        guard.snapshot = Some(snapshot);
    }
    win32_uia::return_raw_element_provider(hwnd, w_param, l_param)
}

// ============================================================================
// Linux AT-SPI2 / D-Bus Wire-Protocol Signal Marshaling
// ============================================================================

/// Convert an Anvil automation ID (e.g. `"anvil.document.dom.4"`) into a valid
/// D-Bus AT-SPI2 object path under `/org/a11y/atspi/accessible/`.
pub fn atspi_node_object_path(automation_id: &str) -> String {
    if automation_id == "anvil.window" {
        return "/org/a11y/atspi/accessible/root".to_string();
    }
    let mut slug = String::with_capacity(automation_id.len());
    for b in automation_id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' {
            slug.push(b as char);
        } else {
            slug.push('_');
        }
    }
    if slug.is_empty() {
        slug.push_str("node");
    }
    format!("/org/a11y/atspi/accessible/{slug}")
}

fn dbus_align_to(buf: &mut Vec<u8>, align: usize) {
    while !buf.len().is_multiple_of(align) {
        buf.push(0);
    }
}

fn push_dbus_string(buf: &mut Vec<u8>, s: &str) {
    dbus_align_to(buf, 4);
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

fn push_dbus_signature(buf: &mut Vec<u8>, sig: &str) {
    buf.push(sig.len() as u8);
    buf.extend_from_slice(sig.as_bytes());
    buf.push(0);
}

fn push_dbus_header_field_string(buf: &mut Vec<u8>, code: u8, type_sig: &str, value: &str) {
    dbus_align_to(buf, 8);
    buf.push(code);
    push_dbus_signature(buf, type_sig);
    push_dbus_string(buf, value);
}

fn push_dbus_header_field_signature(buf: &mut Vec<u8>, code: u8, value: &str) {
    dbus_align_to(buf, 8);
    buf.push(code);
    push_dbus_signature(buf, "g");
    push_dbus_signature(buf, value);
}

fn build_atspi_object_signal(
    serial: u32,
    object_path: &str,
    member: &str,
    detail: &str,
    detail1: i32,
    detail2: i32,
    variant_text: &str,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(128);
    push_dbus_string(&mut body, detail);
    dbus_align_to(&mut body, 4);
    body.extend_from_slice(&detail1.to_le_bytes());
    body.extend_from_slice(&detail2.to_le_bytes());
    push_dbus_signature(&mut body, "s");
    push_dbus_string(&mut body, variant_text);
    dbus_align_to(&mut body, 4);
    body.extend_from_slice(&0u32.to_le_bytes()); // empty `a{sv}` array byte length

    let mut header_fields = Vec::with_capacity(160);
    // 1 = PATH (`o`), 2 = INTERFACE (`s`), 3 = MEMBER (`s`), 8 = SIGNATURE (`g`)
    push_dbus_header_field_string(&mut header_fields, 1, "o", object_path);
    push_dbus_header_field_string(&mut header_fields, 2, "s", "org.a11y.atspi.Event.Object");
    push_dbus_header_field_string(&mut header_fields, 3, "s", member);
    push_dbus_header_field_signature(&mut header_fields, 8, "siiva{sv}");

    let mut msg = Vec::with_capacity(16 + header_fields.len() + 8 + body.len());
    msg.push(b'l'); // Little-endian
    msg.push(4); // Message type: SIGNAL
    msg.push(1); // Flags: NO_REPLY_EXPECTED
    msg.push(1); // Major protocol version: 1
    msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
    msg.extend_from_slice(&serial.max(1).to_le_bytes());
    msg.extend_from_slice(&(header_fields.len() as u32).to_le_bytes());
    msg.extend_from_slice(&header_fields);
    dbus_align_to(&mut msg, 8);
    msg.extend_from_slice(&body);
    msg
}

/// Marshal a little-endian D-Bus `SIGNAL` frame (`msg_type = 4`, `NO_REPLY_EXPECTED = 1`,
/// protocol `1`) for `org.a11y.atspi.Event.Object.StateChanged` carrying `"focused"`,
/// `detail1 = 1`, `detail2 = 0`, and a string variant `"{role}: {name}"`.
pub fn encode_atspi_focus_signal_frame(serial: u32, node: &AccessibleNode) -> Vec<u8> {
    let summary = if node.value.is_empty() {
        format!("{}: {}", node.role.atspi_role_name(), node.name)
    } else {
        format!(
            "{}: {} ({})",
            node.role.atspi_role_name(),
            node.name,
            node.value
        )
    };
    let path = atspi_node_object_path(&node.automation_id);
    build_atspi_object_signal(serial, &path, "StateChanged", "focused", 1, 0, &summary)
}

/// Marshal a little-endian D-Bus `SIGNAL` frame for
/// `org.a11y.atspi.Event.Object.ChildrenChanged` when tree children are inserted or removed.
pub fn encode_atspi_children_changed_signal_frame(
    serial: u32,
    parent_automation_id: &str,
    child_index: i32,
    added: bool,
    child: &AccessibleNode,
) -> Vec<u8> {
    let path = atspi_node_object_path(parent_automation_id);
    let detail = if added { "add" } else { "remove" };
    let child_ref = format!(
        "{} ({}: {})",
        atspi_node_object_path(&child.automation_id),
        child.role.atspi_role_name(),
        child.name
    );
    build_atspi_object_signal(
        serial,
        &path,
        "ChildrenChanged",
        detail,
        child_index,
        0,
        &child_ref,
    )
}

/// Extract a Unix filesystem socket path from a D-Bus address string such as
/// `unix:path=/run/user/1000/at-spi/bus_0` or `unix:path=/run/user/1000/bus,guid=...`.
pub fn parse_dbus_unix_socket_path(address: &str) -> Option<String> {
    for entry in address.split(';') {
        let entry = entry.trim();
        let Some(rest) = entry.strip_prefix("unix:") else {
            continue;
        };
        for kv in rest.split(',') {
            if let Some(path) = kv.trim().strip_prefix("path=")
                && !path.is_empty()
            {
                return Some(path.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod linux_atspi {
    use super::{
        AccessibleNode, encode_atspi_children_changed_signal_frame,
        encode_atspi_focus_signal_frame, parse_dbus_unix_socket_path,
    };
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn send_atspi_frame(frame: &[u8]) {
        let addr = std::env::var("AT_SPI_BUS_ADDRESS")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::env::var("DBUS_SESSION_BUS_ADDRESS")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
            });
        let Some(addr) = addr else {
            return;
        };
        let Some(sock_path) = parse_dbus_unix_socket_path(&addr) else {
            return;
        };
        let Ok(mut stream) = UnixStream::connect(&sock_path) else {
            return;
        };
        let _ = stream.set_nonblocking(true);
        let _ = stream.set_write_timeout(Some(Duration::from_millis(5)));
        let _ = stream.write_all(b"\0AUTH EXTERNAL\r\nBEGIN\r\n");
        let _ = stream.write_all(frame);
    }

    pub fn emit_focus_signal_nonblocking(serial: u32, node: &AccessibleNode) {
        let frame = encode_atspi_focus_signal_frame(serial, node);
        send_atspi_frame(&frame);
    }

    pub fn emit_children_changed_nonblocking(
        serial: u32,
        parent_automation_id: &str,
        child_index: i32,
        added: bool,
        child: &AccessibleNode,
    ) {
        let frame = encode_atspi_children_changed_signal_frame(
            serial,
            parent_automation_id,
            child_index,
            added,
            child,
        );
        send_atspi_frame(&frame);
    }
}

// ============================================================================
// Windows UI Automation COM Fragment & FragmentRoot Provider
// ============================================================================

#[cfg(target_os = "windows")]
mod win32_uia {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};

    const S_OK: i32 = 0;
    const E_NOINTERFACE: i32 = -2147467262i32; // 0x80004002
    const E_INVALIDARG: i32 = -2147024809i32; // 0x80070057

    const PROVIDER_OPTIONS_SERVER_SIDE_PROVIDER: u32 = 0x0002;
    const PROVIDER_OPTIONS_USE_COM_THREADING: u32 = 0x0020;

    pub const NAVIGATE_DIRECTION_PARENT: i32 = 0;
    pub const NAVIGATE_DIRECTION_NEXT_SIBLING: i32 = 1;
    pub const NAVIGATE_DIRECTION_PREVIOUS_SIBLING: i32 = 2;
    pub const NAVIGATE_DIRECTION_FIRST_CHILD: i32 = 3;
    pub const NAVIGATE_DIRECTION_LAST_CHILD: i32 = 4;

    const VT_EMPTY: u16 = 0;
    const VT_I4: u16 = 3;
    const VT_BSTR: u16 = 8;
    const VT_BOOL: u16 = 11;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const IID_IUNKNOWN: Guid = Guid {
        data1: 0x0000_0000,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    const IID_IRAW_ELEMENT_PROVIDER_SIMPLE: Guid = Guid {
        data1: 0xD6DD_68D1,
        data2: 0x86FD,
        data3: 0x4332,
        data4: [0x86, 0x66, 0x9A, 0xBE, 0xDE, 0xA2, 0xD2, 0x4C],
    };

    const IID_IRAW_ELEMENT_PROVIDER_FRAGMENT: Guid = Guid {
        data1: 0xF706_3DA8,
        data2: 0x8359,
        data3: 0x439C,
        data4: [0x92, 0x97, 0xBB, 0xC5, 0x29, 0x9A, 0x7D, 0x87],
    };

    const IID_IRAW_ELEMENT_PROVIDER_FRAGMENT_ROOT: Guid = Guid {
        data1: 0x620C_E2A5,
        data2: 0xAB8F,
        data3: 0x40A9,
        data4: [0x86, 0xCB, 0xDE, 0x3C, 0x75, 0x59, 0x9B, 0x58],
    };

    #[repr(C)]
    pub struct Variant {
        pub vt: u16,
        pub reserved1: u16,
        pub reserved2: u16,
        pub reserved3: u16,
        pub data0: usize,
        pub data1: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct UiaRect {
        pub left: f64,
        pub top: f64,
        pub width: f64,
        pub height: f64,
    }

    #[repr(C)]
    struct WinPoint {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct IRawElementProviderSimpleVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_provider_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
        get_pattern_provider: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32,
        get_property_value: unsafe extern "system" fn(*mut c_void, i32, *mut Variant) -> i32,
        get_host_raw_element_provider:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct IRawElementProviderFragmentVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        navigate: unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32,
        get_runtime_id: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
        get_bounding_rectangle: unsafe extern "system" fn(*mut c_void, *mut UiaRect) -> i32,
        get_embedded_fragment_roots:
            unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
        set_focus: unsafe extern "system" fn(*mut c_void) -> i32,
        get_fragment_root: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct IRawElementProviderFragmentRootVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        element_provider_from_point:
            unsafe extern "system" fn(*mut c_void, f64, f64, *mut *mut c_void) -> i32,
        get_focus: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct UiaNodeProvider {
        lp_simple: *const IRawElementProviderSimpleVtbl,
        lp_frag: *const IRawElementProviderFragmentVtbl,
        lp_root: *const IRawElementProviderFragmentRootVtbl,
        ref_count: AtomicU32,
        snapshot: Arc<AccessibilityTreeSnapshot>,
        node_index: usize,
        hwnd: isize,
    }

    static SIMPLE_VTBL: IRawElementProviderSimpleVtbl = IRawElementProviderSimpleVtbl {
        query_interface: simple_query_interface,
        add_ref: simple_add_ref,
        release: simple_release,
        get_provider_options: uia_get_provider_options,
        get_pattern_provider: uia_get_pattern_provider,
        get_property_value: uia_get_property_value,
        get_host_raw_element_provider: uia_get_host_raw_element_provider,
    };

    static FRAG_VTBL: IRawElementProviderFragmentVtbl = IRawElementProviderFragmentVtbl {
        query_interface: frag_query_interface,
        add_ref: frag_add_ref,
        release: frag_release,
        navigate: frag_navigate,
        get_runtime_id: frag_get_runtime_id,
        get_bounding_rectangle: frag_get_bounding_rectangle,
        get_embedded_fragment_roots: frag_get_embedded_fragment_roots,
        set_focus: frag_set_focus,
        get_fragment_root: frag_get_fragment_root,
    };

    static ROOT_VTBL: IRawElementProviderFragmentRootVtbl = IRawElementProviderFragmentRootVtbl {
        query_interface: root_query_interface,
        add_ref: root_add_ref,
        release: root_release,
        element_provider_from_point: root_element_provider_from_point,
        get_focus: root_get_focus,
    };

    impl UiaNodeProvider {
        fn alloc(
            snapshot: Arc<AccessibilityTreeSnapshot>,
            node_index: usize,
            hwnd: isize,
        ) -> *mut Self {
            Box::into_raw(Box::new(Self {
                lp_simple: &SIMPLE_VTBL,
                lp_frag: &FRAG_VTBL,
                lp_root: &ROOT_VTBL,
                ref_count: AtomicU32::new(1),
                snapshot,
                node_index,
                hwnd,
            }))
        }

        unsafe fn from_simple(this: *mut c_void) -> *mut Self {
            this as *mut Self
        }

        unsafe fn from_frag(this: *mut c_void) -> *mut Self {
            let offset = std::mem::offset_of!(Self, lp_frag);
            unsafe { (this as *mut u8).sub(offset) as *mut Self }
        }

        unsafe fn from_root(this: *mut c_void) -> *mut Self {
            let offset = std::mem::offset_of!(Self, lp_root);
            unsafe { (this as *mut u8).sub(offset) as *mut Self }
        }

        unsafe fn as_simple(base: *mut Self) -> *mut c_void {
            base as *mut c_void
        }

        unsafe fn as_frag(base: *mut Self) -> *mut c_void {
            unsafe { (&raw mut (*base).lp_frag) as *mut c_void }
        }

        unsafe fn as_root(base: *mut Self) -> *mut c_void {
            unsafe { (&raw mut (*base).lp_root) as *mut c_void }
        }
    }

    type FnUiaReturnRawElementProvider =
        unsafe extern "system" fn(isize, usize, isize, *mut c_void) -> isize;
    type FnUiaHostProviderFromHwnd = unsafe extern "system" fn(isize, *mut *mut c_void) -> i32;
    type FnUiaRaiseAutomationEvent = unsafe extern "system" fn(*mut c_void, i32) -> i32;
    type FnSysAllocStringLen = unsafe extern "system" fn(*const u16, u32) -> *mut u16;
    type FnSafeArrayCreateVector = unsafe extern "system" fn(u16, i32, u32) -> *mut c_void;
    type FnSafeArrayAccessData = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
    type FnSafeArrayUnaccessData = unsafe extern "system" fn(*mut c_void) -> i32;
    type FnClientToScreen = unsafe extern "system" fn(isize, *mut WinPoint) -> i32;
    type FnScreenToClient = unsafe extern "system" fn(isize, *mut WinPoint) -> i32;

    struct UiaProcs {
        return_raw_element_provider: Option<FnUiaReturnRawElementProvider>,
        host_provider_from_hwnd: Option<FnUiaHostProviderFromHwnd>,
        raise_automation_event: Option<FnUiaRaiseAutomationEvent>,
        sys_alloc_string_len: Option<FnSysAllocStringLen>,
        safe_array_create_vector: Option<FnSafeArrayCreateVector>,
        safe_array_access_data: Option<FnSafeArrayAccessData>,
        safe_array_unaccess_data: Option<FnSafeArrayUnaccessData>,
        client_to_screen: Option<FnClientToScreen>,
        screen_to_client: Option<FnScreenToClient>,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
    }

    fn uia_procs() -> &'static UiaProcs {
        static PROCS: OnceLock<UiaProcs> = OnceLock::new();
        PROCS.get_or_init(|| unsafe {
            let uia_dll: Vec<u16> = "uiautomationcore.dll\0".encode_utf16().collect();
            let ole_dll: Vec<u16> = "oleaut32.dll\0".encode_utf16().collect();
            let usr_dll: Vec<u16> = "user32.dll\0".encode_utf16().collect();
            let h_uia = LoadLibraryW(uia_dll.as_ptr());
            let h_ole = LoadLibraryW(ole_dll.as_ptr());
            let h_usr = LoadLibraryW(usr_dll.as_ptr());

            macro_rules! load_proc {
                ($mod_h:expr, $sym:expr, $ty:ty) => {
                    if !$mod_h.is_null() {
                        let p = GetProcAddress($mod_h, $sym.as_ptr());
                        if !p.is_null() {
                            Some(std::mem::transmute::<*mut c_void, $ty>(p))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
            }

            UiaProcs {
                return_raw_element_provider: load_proc!(
                    h_uia,
                    c"UiaReturnRawElementProvider",
                    FnUiaReturnRawElementProvider
                ),
                host_provider_from_hwnd: load_proc!(
                    h_uia,
                    c"UiaHostProviderFromHwnd",
                    FnUiaHostProviderFromHwnd
                ),
                raise_automation_event: load_proc!(
                    h_uia,
                    c"UiaRaiseAutomationEvent",
                    FnUiaRaiseAutomationEvent
                ),
                sys_alloc_string_len: load_proc!(h_ole, c"SysAllocStringLen", FnSysAllocStringLen),
                safe_array_create_vector: load_proc!(
                    h_ole,
                    c"SafeArrayCreateVector",
                    FnSafeArrayCreateVector
                ),
                safe_array_access_data: load_proc!(
                    h_ole,
                    c"SafeArrayAccessData",
                    FnSafeArrayAccessData
                ),
                safe_array_unaccess_data: load_proc!(
                    h_ole,
                    c"SafeArrayUnaccessData",
                    FnSafeArrayUnaccessData
                ),
                client_to_screen: load_proc!(h_usr, c"ClientToScreen", FnClientToScreen),
                screen_to_client: load_proc!(h_usr, c"ScreenToClient", FnScreenToClient),
            }
        })
    }

    fn alloc_bstr(text: &str) -> *mut u16 {
        let Some(alloc) = uia_procs().sys_alloc_string_len else {
            return std::ptr::null_mut();
        };
        let utf16: Vec<u16> = text.encode_utf16().collect();
        unsafe { alloc(utf16.as_ptr(), utf16.len() as u32) }
    }

    unsafe fn base_query_interface(
        base: *mut UiaNodeProvider,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32 {
        if base.is_null() || ppv.is_null() || riid.is_null() {
            return E_INVALIDARG;
        }
        let iid = unsafe { *riid };
        let node_idx = unsafe { (*base).node_index };
        let out = if iid == IID_IUNKNOWN || iid == IID_IRAW_ELEMENT_PROVIDER_SIMPLE {
            unsafe { UiaNodeProvider::as_simple(base) }
        } else if iid == IID_IRAW_ELEMENT_PROVIDER_FRAGMENT {
            unsafe { UiaNodeProvider::as_frag(base) }
        } else if iid == IID_IRAW_ELEMENT_PROVIDER_FRAGMENT_ROOT && node_idx == 0 {
            unsafe { UiaNodeProvider::as_root(base) }
        } else {
            unsafe {
                *ppv = std::ptr::null_mut();
            }
            return E_NOINTERFACE;
        };
        unsafe {
            (*base).ref_count.fetch_add(1, Ordering::Relaxed);
            *ppv = out;
        }
        S_OK
    }

    unsafe fn base_add_ref(base: *mut UiaNodeProvider) -> u32 {
        if base.is_null() {
            return 0;
        }
        unsafe { (*base).ref_count.fetch_add(1, Ordering::Relaxed) + 1 }
    }

    unsafe fn base_release(base: *mut UiaNodeProvider) -> u32 {
        if base.is_null() {
            return 0;
        }
        let prev = unsafe { (*base).ref_count.fetch_sub(1, Ordering::Release) };
        if prev == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            unsafe {
                drop(Box::from_raw(base));
            }
            0
        } else {
            prev.saturating_sub(1)
        }
    }

    unsafe extern "system" fn simple_query_interface(
        this: *mut c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32 {
        unsafe { base_query_interface(UiaNodeProvider::from_simple(this), riid, ppv) }
    }

    unsafe extern "system" fn simple_add_ref(this: *mut c_void) -> u32 {
        unsafe { base_add_ref(UiaNodeProvider::from_simple(this)) }
    }

    unsafe extern "system" fn simple_release(this: *mut c_void) -> u32 {
        unsafe { base_release(UiaNodeProvider::from_simple(this)) }
    }

    unsafe extern "system" fn frag_query_interface(
        this: *mut c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32 {
        unsafe { base_query_interface(UiaNodeProvider::from_frag(this), riid, ppv) }
    }

    unsafe extern "system" fn frag_add_ref(this: *mut c_void) -> u32 {
        unsafe { base_add_ref(UiaNodeProvider::from_frag(this)) }
    }

    unsafe extern "system" fn frag_release(this: *mut c_void) -> u32 {
        unsafe { base_release(UiaNodeProvider::from_frag(this)) }
    }

    unsafe extern "system" fn root_query_interface(
        this: *mut c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32 {
        unsafe { base_query_interface(UiaNodeProvider::from_root(this), riid, ppv) }
    }

    unsafe extern "system" fn root_add_ref(this: *mut c_void) -> u32 {
        unsafe { base_add_ref(UiaNodeProvider::from_root(this)) }
    }

    unsafe extern "system" fn root_release(this: *mut c_void) -> u32 {
        unsafe { base_release(UiaNodeProvider::from_root(this)) }
    }

    unsafe extern "system" fn uia_get_provider_options(
        _this: *mut c_void,
        p_ret_val: *mut u32,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = PROVIDER_OPTIONS_SERVER_SIDE_PROVIDER | PROVIDER_OPTIONS_USE_COM_THREADING;
        }
        S_OK
    }

    unsafe extern "system" fn uia_get_pattern_provider(
        _this: *mut c_void,
        _pattern_id: i32,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        S_OK
    }

    unsafe extern "system" fn uia_get_property_value(
        this: *mut c_void,
        property_id: i32,
        p_ret_val: *mut Variant,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = Variant {
                vt: VT_EMPTY,
                reserved1: 0,
                reserved2: 0,
                reserved3: 0,
                data0: 0,
                data1: 0,
            };
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_simple(this) };
        let Some(node) = base.snapshot.node(base.node_index) else {
            return S_OK;
        };
        match property_id {
            UIA_CONTROL_TYPE_PROPERTY_ID => unsafe {
                (*p_ret_val).vt = VT_I4;
                (*p_ret_val).data0 = node.role.uia_control_type_id() as u32 as usize;
            },
            UIA_NAME_PROPERTY_ID => {
                let bstr = alloc_bstr(&node.name);
                if !bstr.is_null() {
                    unsafe {
                        (*p_ret_val).vt = VT_BSTR;
                        (*p_ret_val).data0 = bstr as usize;
                    }
                }
            }
            UIA_VALUE_VALUE_PROPERTY_ID => {
                let bstr = alloc_bstr(&node.value);
                if !bstr.is_null() {
                    unsafe {
                        (*p_ret_val).vt = VT_BSTR;
                        (*p_ret_val).data0 = bstr as usize;
                    }
                }
            }
            UIA_AUTOMATION_ID_PROPERTY_ID => {
                let bstr = alloc_bstr(&node.automation_id);
                if !bstr.is_null() {
                    unsafe {
                        (*p_ret_val).vt = VT_BSTR;
                        (*p_ret_val).data0 = bstr as usize;
                    }
                }
            }
            UIA_HAS_KEYBOARD_FOCUS_PROPERTY_ID => unsafe {
                (*p_ret_val).vt = VT_BOOL;
                (*p_ret_val).data0 = if node.focused { 0xFFFFusize } else { 0 };
            },
            UIA_IS_KEYBOARD_FOCUSABLE_PROPERTY_ID => unsafe {
                (*p_ret_val).vt = VT_BOOL;
                (*p_ret_val).data0 = if node.focusable { 0xFFFFusize } else { 0 };
            },
            UIA_IS_CONTROL_ELEMENT_PROPERTY_ID | UIA_IS_CONTENT_ELEMENT_PROPERTY_ID => unsafe {
                (*p_ret_val).vt = VT_BOOL;
                (*p_ret_val).data0 = 0xFFFFusize;
            },
            _ => {}
        }
        S_OK
    }

    unsafe extern "system" fn uia_get_host_raw_element_provider(
        this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_simple(this) };
        if base.node_index != 0 || base.hwnd == 0 {
            return S_OK;
        }
        if let Some(host_fn) = uia_procs().host_provider_from_hwnd {
            return unsafe { host_fn(base.hwnd, p_ret_val) };
        }
        S_OK
    }

    unsafe extern "system" fn frag_navigate(
        this: *mut c_void,
        direction: i32,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_frag(this) };
        let target = match direction {
            NAVIGATE_DIRECTION_PARENT => base.snapshot.parent_of(base.node_index),
            NAVIGATE_DIRECTION_NEXT_SIBLING => base.snapshot.next_sibling_of(base.node_index),
            NAVIGATE_DIRECTION_PREVIOUS_SIBLING => base.snapshot.prev_sibling_of(base.node_index),
            NAVIGATE_DIRECTION_FIRST_CHILD => base.snapshot.first_child_of(base.node_index),
            NAVIGATE_DIRECTION_LAST_CHILD => base.snapshot.last_child_of(base.node_index),
            _ => None,
        };
        if let Some(idx) = target {
            let child = UiaNodeProvider::alloc(Arc::clone(&base.snapshot), idx, base.hwnd);
            unsafe {
                *p_ret_val = UiaNodeProvider::as_frag(child);
            }
        }
        S_OK
    }

    unsafe extern "system" fn frag_get_runtime_id(
        this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_frag(this) };
        if base.node_index == 0 {
            return S_OK;
        }
        let procs = uia_procs();
        if let (Some(create), Some(access), Some(unaccess)) = (
            procs.safe_array_create_vector,
            procs.safe_array_access_data,
            procs.safe_array_unaccess_data,
        ) {
            let psa = unsafe { create(VT_I4, 0, 2) };
            if !psa.is_null() {
                let mut data: *mut c_void = std::ptr::null_mut();
                if unsafe { access(psa, &mut data) } == S_OK && !data.is_null() {
                    let ids = unsafe { std::slice::from_raw_parts_mut(data as *mut i32, 2) };
                    ids[0] = 42; // UiaAppendRuntimeId magic prefix
                    ids[1] = base.node_index as i32;
                    unsafe {
                        let _ = unaccess(psa);
                        *p_ret_val = psa;
                    }
                }
            }
        }
        S_OK
    }

    unsafe extern "system" fn frag_get_bounding_rectangle(
        this: *mut c_void,
        p_ret_val: *mut UiaRect,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = UiaRect {
                left: 0.0,
                top: 0.0,
                width: 0.0,
                height: 0.0,
            };
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_frag(this) };
        let Some(node) = base.snapshot.node(base.node_index) else {
            return S_OK;
        };
        let mut left = node.bounds.x as f64;
        let mut top = node.bounds.y as f64;
        if base.hwnd != 0
            && let Some(cts) = uia_procs().client_to_screen
        {
            let mut pt = WinPoint {
                x: node.bounds.x.round() as i32,
                y: node.bounds.y.round() as i32,
            };
            if unsafe { cts(base.hwnd, &mut pt) } != 0 {
                left = pt.x as f64;
                top = pt.y as f64;
            }
        }
        unsafe {
            *p_ret_val = UiaRect {
                left,
                top,
                width: node.bounds.width.max(0.0) as f64,
                height: node.bounds.height.max(0.0) as f64,
            };
        }
        S_OK
    }

    unsafe extern "system" fn frag_get_embedded_fragment_roots(
        _this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        S_OK
    }

    unsafe extern "system" fn frag_set_focus(_this: *mut c_void) -> i32 {
        S_OK
    }

    unsafe extern "system" fn frag_get_fragment_root(
        this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_frag(this) };
        let root = UiaNodeProvider::alloc(Arc::clone(&base.snapshot), 0, base.hwnd);
        unsafe {
            *p_ret_val = UiaNodeProvider::as_root(root);
        }
        S_OK
    }

    unsafe extern "system" fn root_element_provider_from_point(
        this: *mut c_void,
        x: f64,
        y: f64,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_root(this) };
        let mut cx = x as f32;
        let mut cy = y as f32;
        if base.hwnd != 0
            && let Some(stc) = uia_procs().screen_to_client
        {
            let mut pt = WinPoint {
                x: x.round() as i32,
                y: y.round() as i32,
            };
            if unsafe { stc(base.hwnd, &mut pt) } != 0 {
                cx = pt.x as f32;
                cy = pt.y as f32;
            }
        }
        if let Some(hit) = base.snapshot.hit_test(cx, cy) {
            let child = UiaNodeProvider::alloc(Arc::clone(&base.snapshot), hit, base.hwnd);
            unsafe {
                *p_ret_val = UiaNodeProvider::as_frag(child);
            }
        }
        S_OK
    }

    unsafe extern "system" fn root_get_focus(
        this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        if this.is_null() {
            return S_OK;
        }
        let base = unsafe { &*UiaNodeProvider::from_root(this) };
        if let Some(idx) = base.snapshot.focused_index {
            let focused = UiaNodeProvider::alloc(Arc::clone(&base.snapshot), idx, base.hwnd);
            unsafe {
                *p_ret_val = UiaNodeProvider::as_frag(focused);
            }
        }
        S_OK
    }

    pub fn return_raw_element_provider(
        hwnd: isize,
        w_param: usize,
        l_param: isize,
    ) -> Option<isize> {
        let f = uia_procs().return_raw_element_provider?;
        let snap = bridge_state()
            .lock()
            .ok()
            .and_then(|g| g.snapshot.clone())?;
        let root = UiaNodeProvider::alloc(snap, 0, hwnd);
        Some(unsafe { f(hwnd, w_param, l_param, UiaNodeProvider::as_simple(root)) })
    }

    pub fn raise_focus_changed_event() {
        let Some(f) = uia_procs().raise_automation_event else {
            return;
        };
        let Ok(guard) = bridge_state().lock() else {
            return;
        };
        let Some(snap) = guard.snapshot.clone() else {
            return;
        };
        let hwnd = guard.hwnd;
        drop(guard);
        let idx = snap.focused_index.unwrap_or(0);
        let provider = UiaNodeProvider::alloc(snap, idx, hwnd);
        let simple = unsafe { UiaNodeProvider::as_simple(provider) };
        let _ = unsafe { f(simple, UIA_AUTOMATION_FOCUS_CHANGED_EVENT_ID) };
        unsafe {
            simple_release(simple);
        }
    }

    #[cfg(test)]
    pub fn test_query_property_i4(property_id: i32) -> Option<(u16, i32)> {
        let (snap, hwnd) = {
            let guard = bridge_state().lock().ok()?;
            (guard.snapshot.clone()?, guard.hwnd)
        };
        let root_alloc = UiaNodeProvider::alloc(Arc::clone(&snap), 0, hwnd);
        let root_iface = unsafe { UiaNodeProvider::as_root(root_alloc) };
        let mut frag_ptr: *mut c_void = std::ptr::null_mut();
        let hr_focus = unsafe { root_get_focus(root_iface, &mut frag_ptr) };
        unsafe {
            root_release(root_iface);
        }
        if hr_focus != S_OK || frag_ptr.is_null() {
            return None;
        }
        let mut simple_ptr: *mut c_void = std::ptr::null_mut();
        let hr_qi = unsafe {
            frag_query_interface(frag_ptr, &IID_IRAW_ELEMENT_PROVIDER_SIMPLE, &mut simple_ptr)
        };
        unsafe {
            frag_release(frag_ptr);
        }
        if hr_qi != S_OK || simple_ptr.is_null() {
            return None;
        }
        let mut var = Variant {
            vt: 0,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            data0: 0,
            data1: 0,
        };
        let hr = unsafe { uia_get_property_value(simple_ptr, property_id, &mut var) };
        unsafe {
            simple_release(simple_ptr);
        }
        if hr == S_OK {
            Some((var.vt, var.data0 as i32))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn test_fragment_navigation_roundtrip(
        snap: Arc<AccessibilityTreeSnapshot>,
    ) -> (usize, Option<UiaRect>, bool, Option<usize>) {
        let root_alloc = UiaNodeProvider::alloc(Arc::clone(&snap), 0, 0);
        let root_iface = unsafe { UiaNodeProvider::as_root(root_alloc) };
        let root_frag = unsafe { UiaNodeProvider::as_frag(root_alloc) };

        let mut first_child_frag: *mut c_void = std::ptr::null_mut();
        let hr1 = unsafe {
            frag_navigate(
                root_frag,
                NAVIGATE_DIRECTION_FIRST_CHILD,
                &mut first_child_frag,
            )
        };
        assert_eq!(hr1, S_OK);
        assert!(!first_child_frag.is_null());

        let mut sibling_frag: *mut c_void = std::ptr::null_mut();
        let hr2 = unsafe {
            frag_navigate(
                first_child_frag,
                NAVIGATE_DIRECTION_NEXT_SIBLING,
                &mut sibling_frag,
            )
        };
        assert_eq!(hr2, S_OK);
        assert!(!sibling_frag.is_null());

        let sibling_idx = unsafe { (*UiaNodeProvider::from_frag(sibling_frag)).node_index };
        let mut sibling_rect = UiaRect {
            left: 0.0,
            top: 0.0,
            width: 0.0,
            height: 0.0,
        };
        let hr3 = unsafe { frag_get_bounding_rectangle(sibling_frag, &mut sibling_rect) };
        assert_eq!(hr3, S_OK);

        let mut returned_root: *mut c_void = std::ptr::null_mut();
        let hr4 = unsafe { frag_get_fragment_root(sibling_frag, &mut returned_root) };
        assert_eq!(hr4, S_OK);
        let root_ok = !returned_root.is_null()
            && unsafe { (*UiaNodeProvider::from_root(returned_root)).node_index == 0 };

        let mut hit_frag: *mut c_void = std::ptr::null_mut();
        let hr5 = unsafe {
            root_element_provider_from_point(
                root_iface,
                sibling_rect.left + 8.0,
                sibling_rect.top + 8.0,
                &mut hit_frag,
            )
        };
        assert_eq!(hr5, S_OK);
        let hit_idx = if hit_frag.is_null() {
            None
        } else {
            let idx = unsafe { (*UiaNodeProvider::from_frag(hit_frag)).node_index };
            unsafe {
                frag_release(hit_frag);
            }
            Some(idx)
        };

        unsafe {
            frag_release(first_child_frag);
            frag_release(sibling_frag);
            root_release(returned_root);
            root_release(root_iface);
        }
        (sibling_idx, Some(sibling_rect), root_ok, hit_idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Viewport;

    #[test]
    fn snapshot_tracks_document_toolbar_and_popover_focus() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            "<title>Forge Portal</title><p>Hello</p>",
            "https://example.org/",
        );
        let mut ui = UiState::new();

        // 1. Default page focus exposes Document role and page title/URL.
        let snap_page = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);
        assert_eq!(snap_page.focused_node.role, AccessibleRole::Document);
        assert_eq!(snap_page.focused_node.name, "Forge Portal");
        assert_eq!(snap_page.focused_node.value, "https://example.org/");

        // 2. Focusing a toolbar control exposes Button role and accessible label.
        ui.focus_chrome(ChromeControl::SiteInfo);
        let snap_tb = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);
        assert_eq!(snap_tb.focused_node.role, AccessibleRole::Button);
        assert_eq!(snap_tb.focused_node.name, "Site info and permissions");

        // 3. Opening the Site Information popover transitions focused node to MenuItem.
        ui.toggle_site_popover();
        ui.move_menu_with_session(&session, 2);
        let snap_site = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);
        assert_eq!(snap_site.focused_node.role, AccessibleRole::MenuItem);
        assert!(
            snap_site
                .focused_node
                .name
                .contains("Site JavaScript (Cinder)")
        );
        assert_eq!(snap_site.focused_node.value, "ALLOWED");

        // 4. Opening the Downloads popover transitions focused node to Downloads MenuItem.
        ui.toggle_downloads_popover();
        ui.move_menu_edge_with_session(&session, true);
        let snap_dl = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);
        assert_eq!(snap_dl.focused_node.role, AccessibleRole::MenuItem);
        assert!(
            snap_dl
                .focused_node
                .name
                .contains("Open Full Downloads Vault")
        );
    }

    #[test]
    fn snapshot_builds_navigable_dom_and_menu_tree() {
        let mut session = Session::new(Viewport {
            width: 800.0,
            height: 600.0,
        });
        session.open_html(
            r#"
            <title>Semantic Tree</title>
            <main>
              <h1>Welcome Heading</h1>
              <nav aria-label="Primary Nav">
                <a href="https://anvil.local/docs">Documentation</a>
              </nav>
              <button>Launch Action</button>
              <ul><li>First Bullet</li><li>Second Bullet</li></ul>
              <img src="logo.png" alt="Anvil Emblem">
            </main>
            "#,
            "https://example.org/tree",
        );
        let mut ui = UiState::new();
        let snap = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);

        let root_children = &snap.nodes[0].children;
        assert!(root_children.len() >= 2);
        let tb_idx = snap.first_child_of(0).unwrap();
        assert_eq!(snap.nodes[tb_idx].role, AccessibleRole::Toolbar);
        let doc_idx = snap.next_sibling_of(tb_idx).unwrap();
        assert_eq!(snap.nodes[doc_idx].role, AccessibleRole::Document);
        assert_eq!(snap.parent_of(doc_idx), Some(0));
        assert_eq!(snap.prev_sibling_of(doc_idx), Some(tb_idx));

        let heading_idx = snap
            .nodes
            .iter()
            .position(|n| n.role == AccessibleRole::Heading && n.name == "Welcome Heading")
            .expect("h1 heading node");
        assert_eq!(snap.nodes[heading_idx].heading_level, Some(1));
        assert!(snap.parent_of(heading_idx).is_some());

        let link_idx = snap
            .nodes
            .iter()
            .position(|n| n.role == AccessibleRole::Link && n.name == "Documentation")
            .expect("link node");
        assert_eq!(snap.nodes[link_idx].value, "https://anvil.local/docs");
        assert!(snap.nodes[link_idx].focusable);

        let btn_idx = snap
            .nodes
            .iter()
            .position(|n| n.role == AccessibleRole::Button && n.name == "Launch Action")
            .expect("dom button node");
        let btn_bounds = snap.nodes[btn_idx].bounds;
        assert!(btn_bounds.width > 0.0 && btn_bounds.height > 0.0);
        let hit = snap.hit_test(btn_bounds.x + 4.0, btn_bounds.y + 4.0);
        assert_eq!(hit, Some(btn_idx));

        // Open application menu and verify Menu + MenuItem parent/child navigation.
        ui.toggle_app_menu();
        let snap_menu = AccessibilityTreeSnapshot::from_session_and_ui(&session, &ui, 800.0, 600.0);
        let menu_idx = snap_menu
            .find_by_automation_id("anvil.menu.app")
            .expect("app menu container node");
        assert_eq!(snap_menu.nodes[menu_idx].role, AccessibleRole::Menu);
        assert_eq!(
            snap_menu.nodes[menu_idx].children.len(),
            AppMenuAction::ALL.len()
        );
        let first_item = snap_menu.first_child_of(menu_idx).unwrap();
        let last_item = snap_menu.last_child_of(menu_idx).unwrap();
        assert_eq!(snap_menu.nodes[first_item].role, AccessibleRole::MenuItem);
        assert_eq!(snap_menu.parent_of(last_item), Some(menu_idx));
    }

    #[test]
    fn encodes_valid_dbus_atspi_focus_and_children_changed_wire_frames() {
        let node = AccessibleNode {
            automation_id: "anvil.toolbar.Back".to_string(),
            role: AccessibleRole::Button,
            name: "Back".to_string(),
            value: String::new(),
            bounds: Rect {
                x: 8.0,
                y: 43.0,
                width: 28.0,
                height: 28.0,
            },
            focusable: true,
            focused: true,
            expanded: None,
            parent_index: Some(1),
            children: Vec::new(),
            heading_level: None,
        };
        let frame = encode_atspi_focus_signal_frame(7, &node);
        assert_eq!(frame[0], b'l', "little-endian D-Bus wire marker");
        assert_eq!(frame[1], 4, "D-Bus SIGNAL message type");
        assert_eq!(frame[2], 1, "NO_REPLY_EXPECTED flag");
        assert_eq!(frame[3], 1, "D-Bus major protocol version 1");
        let body_len = u32::from_le_bytes(frame[4..8].try_into().unwrap()) as usize;
        let serial = u32::from_le_bytes(frame[8..12].try_into().unwrap());
        let hdr_fields_len = u32::from_le_bytes(frame[12..16].try_into().unwrap()) as usize;
        assert_eq!(serial, 7);
        let padded_hdr_end = (16 + hdr_fields_len + 7) & !7;
        assert_eq!(frame.len(), padded_hdr_end + body_len);

        let cc_frame =
            encode_atspi_children_changed_signal_frame(8, "anvil.document", 0, true, &node);
        let cc_body_len = u32::from_le_bytes(cc_frame[4..8].try_into().unwrap()) as usize;
        let cc_hdr_len = u32::from_le_bytes(cc_frame[12..16].try_into().unwrap()) as usize;
        let cc_padded = (16 + cc_hdr_len + 7) & !7;
        assert_eq!(cc_frame.len(), cc_padded + cc_body_len);
        assert_eq!(
            atspi_node_object_path("anvil.document.dom.4"),
            "/org/a11y/atspi/accessible/anvil_document_dom_4"
        );
        assert_eq!(
            parse_dbus_unix_socket_path("unix:path=/run/user/1000/at-spi/bus_0,guid=0123"),
            Some("/run/user/1000/at-spi/bus_0".to_string())
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uia_provider_answers_control_type_property() {
        let mut session = Session::new(Viewport {
            width: 640.0,
            height: 480.0,
        });
        session.open_html(
            "<title>UIA Check</title><h1>Title</h1>",
            "https://example.org/",
        );
        let mut ui = UiState::new();
        ui.focus_chrome(ChromeControl::AppMenu);
        notify_focus_if_changed(0, &session, &ui, 640, 480);
        let (vt, ctrl_type) =
            win32_uia::test_query_property_i4(UIA_CONTROL_TYPE_PROPERTY_ID).unwrap();
        assert_eq!(vt, 3); // VT_I4
        assert_eq!(ctrl_type, UIA_BUTTON_CONTROL_TYPE_ID);

        let snap = Arc::new(AccessibilityTreeSnapshot::from_session_and_ui(
            &session, &ui, 640.0, 480.0,
        ));
        let (doc_idx, rect, root_ok, hit_idx) = win32_uia::test_fragment_navigation_roundtrip(snap);
        assert!(root_ok);
        assert!(rect.unwrap().width > 0.0);
        assert!(hit_idx.is_some());
        assert!(doc_idx > 0);
    }
}
