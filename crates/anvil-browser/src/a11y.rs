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
use anvil_layout::Rect;
use std::sync::{Mutex, OnceLock};

/// Win32 `WM_GETOBJECT` window message constant (`0x003D`).
pub const WM_GETOBJECT: u32 = 0x003D;

/// UI Automation `UiaRootObjectId` (`lParam == -25` on `WM_GETOBJECT`).
pub const UIA_ROOT_OBJECT_ID: i32 = -25;

// UI Automation ControlType IDs.
pub const UIA_BUTTON_CONTROL_TYPE_ID: i32 = 50000;
pub const UIA_EDIT_CONTROL_TYPE_ID: i32 = 50004;
pub const UIA_MENU_CONTROL_TYPE_ID: i32 = 50009;
pub const UIA_MENU_ITEM_CONTROL_TYPE_ID: i32 = 50011;
pub const UIA_TAB_ITEM_CONTROL_TYPE_ID: i32 = 50019;
pub const UIA_TOOL_BAR_CONTROL_TYPE_ID: i32 = 50021;
pub const UIA_DOCUMENT_CONTROL_TYPE_ID: i32 = 50030;
pub const UIA_WINDOW_CONTROL_TYPE_ID: i32 = 50032;

// UI Automation Property IDs.
pub const UIA_CONTROL_TYPE_PROPERTY_ID: i32 = 30003;
pub const UIA_NAME_PROPERTY_ID: i32 = 30005;
pub const UIA_HAS_KEYBOARD_FOCUS_PROPERTY_ID: i32 = 30008;
pub const UIA_IS_KEYBOARD_FOCUSABLE_PROPERTY_ID: i32 = 30009;
pub const UIA_AUTOMATION_ID_PROPERTY_ID: i32 = 30011;
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
}

impl AccessibleRole {
    /// Windows UI Automation `UIA_*ControlTypeId` for this role.
    pub const fn uia_control_type_id(self) -> i32 {
        match self {
            Self::Window => UIA_WINDOW_CONTROL_TYPE_ID,
            Self::Toolbar => UIA_TOOL_BAR_CONTROL_TYPE_ID,
            Self::TabItem => UIA_TAB_ITEM_CONTROL_TYPE_ID,
            Self::Button => UIA_BUTTON_CONTROL_TYPE_ID,
            Self::Edit => UIA_EDIT_CONTROL_TYPE_ID,
            Self::Menu => UIA_MENU_CONTROL_TYPE_ID,
            Self::MenuItem => UIA_MENU_ITEM_CONTROL_TYPE_ID,
            Self::Document => UIA_DOCUMENT_CONTROL_TYPE_ID,
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
}

/// Complete snapshot of the browser window's accessible tree and currently focused node.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessibilityTreeSnapshot {
    pub window_title: String,
    pub focused_node: AccessibleNode,
    pub nodes: Vec<AccessibleNode>,
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
        });

        for ctrl in chrome_focus_order(session, ui, width) {
            let is_focused = focus == Focus::Toolbar && ui.chrome_focus() == Some(ctrl);
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
                _ => None,
            };
            nodes.push(AccessibleNode {
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
                focused: is_focused,
                expanded,
            });
        }

        let vp = content_viewport(width, height);
        nodes.push(AccessibleNode {
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
            focused: focus == Focus::Page,
            expanded: None,
        });

        Self {
            window_title,
            focused_node,
            nodes,
        }
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
    }
}

#[derive(Default)]
struct A11yBridgeState {
    hwnd: isize,
    last_focus_signature: String,
    snapshot: Option<AccessibilityTreeSnapshot>,
    serial: u32,
    uia_queried: bool,
}

static A11Y_STATE: OnceLock<Mutex<A11yBridgeState>> = OnceLock::new();

fn bridge_state() -> &'static Mutex<A11yBridgeState> {
    A11Y_STATE.get_or_init(|| Mutex::new(A11yBridgeState::default()))
}

/// Synchronize the OS accessibility bridge with the latest [`Session`] and [`UiState`],
/// emitting a platform focus-changed event (`UiaRaiseAutomationEvent` on Windows or
/// an AT-SPI2 D-Bus signal on Linux) whenever the focused accessible item changes.
pub fn notify_focus_if_changed(
    hwnd: isize,
    session: &Session,
    ui: &UiState,
    width: u32,
    height: u32,
) {
    let snapshot =
        AccessibilityTreeSnapshot::from_session_and_ui(session, ui, width as f32, height as f32);
    let sig = format!(
        "{:?}|{}|{}|{}",
        snapshot.focused_node.role,
        snapshot.focused_node.automation_id,
        snapshot.focused_node.name,
        snapshot.focused_node.value
    );
    let (changed, serial, uia_queried, focused_clone) = {
        let Ok(mut guard) = bridge_state().lock() else {
            return;
        };
        guard.hwnd = hwnd;
        let changed = guard.last_focus_signature != sig;
        if changed {
            guard.last_focus_signature = sig;
            guard.serial = guard.serial.wrapping_add(1).max(1);
        }
        let serial = guard.serial;
        let uia_queried = guard.uia_queried;
        let focused_clone = snapshot.focused_node.clone();
        guard.snapshot = Some(snapshot);
        (changed, serial, uia_queried, focused_clone)
    };

    if !changed {
        return;
    }

    #[cfg(target_os = "windows")]
    if uia_queried {
        win32_uia::raise_focus_changed_event();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = uia_queried;

    #[cfg(target_os = "linux")]
    linux_atspi::emit_focus_signal_nonblocking(serial, &focused_clone);
    #[cfg(not(target_os = "linux"))]
    let _ = (serial, focused_clone);
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
    let snapshot =
        AccessibilityTreeSnapshot::from_session_and_ui(session, ui, width as f32, height as f32);
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

/// Marshal a little-endian D-Bus `SIGNAL` frame (`msg_type = 4`, `NO_REPLY_EXPECTED = 1`,
/// protocol `1`) for `org.a11y.atspi.Event.Object.StateChanged` carrying `"focused"`,
/// `detail1 = 1`, `detail2 = 0`, and a string variant `"{role}: {name}"`.
pub fn encode_atspi_focus_signal_frame(serial: u32, node: &AccessibleNode) -> Vec<u8> {
    fn align_to(buf: &mut Vec<u8>, align: usize) {
        while !buf.len().is_multiple_of(align) {
            buf.push(0);
        }
    }
    fn push_dbus_string(buf: &mut Vec<u8>, s: &str) {
        align_to(buf, 4);
        buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
    }
    fn push_dbus_signature(buf: &mut Vec<u8>, sig: &str) {
        buf.push(sig.len() as u8);
        buf.extend_from_slice(sig.as_bytes());
        buf.push(0);
    }
    fn push_header_field_string(buf: &mut Vec<u8>, code: u8, type_sig: &str, value: &str) {
        align_to(buf, 8);
        buf.push(code);
        push_dbus_signature(buf, type_sig);
        push_dbus_string(buf, value);
    }
    fn push_header_field_signature(buf: &mut Vec<u8>, code: u8, value: &str) {
        align_to(buf, 8);
        buf.push(code);
        push_dbus_signature(buf, "g");
        push_dbus_signature(buf, value);
    }

    // Build body with signature `siiva{sv}`:
    // - `s`: "focused"
    // - `i`: 1 (enabled)
    // - `i`: 0
    // - `v`: signature `"s"` + string `"{role}: {name} ({value})"`
    // - `a{sv}`: 0-byte empty dictionary array
    let mut body = Vec::with_capacity(128);
    push_dbus_string(&mut body, "focused");
    align_to(&mut body, 4);
    body.extend_from_slice(&1i32.to_le_bytes());
    body.extend_from_slice(&0i32.to_le_bytes());
    push_dbus_signature(&mut body, "s");
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
    push_dbus_string(&mut body, &summary);
    align_to(&mut body, 4);
    body.extend_from_slice(&0u32.to_le_bytes()); // empty `a{sv}` array byte length

    let mut header_fields = Vec::with_capacity(160);
    // 1 = PATH (`o`), 2 = INTERFACE (`s`), 3 = MEMBER (`s`), 8 = SIGNATURE (`g`)
    push_header_field_string(
        &mut header_fields,
        1,
        "o",
        "/org/a11y/atspi/accessible/root",
    );
    push_header_field_string(&mut header_fields, 2, "s", "org.a11y.atspi.Event.Object");
    push_header_field_string(&mut header_fields, 3, "s", "StateChanged");
    push_header_field_signature(&mut header_fields, 8, "siiva{sv}");

    let mut msg = Vec::with_capacity(16 + header_fields.len() + 8 + body.len());
    msg.push(b'l'); // Little-endian
    msg.push(4); // Message type: SIGNAL
    msg.push(1); // Flags: NO_REPLY_EXPECTED
    msg.push(1); // Major protocol version: 1
    msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
    msg.extend_from_slice(&serial.max(1).to_le_bytes());
    msg.extend_from_slice(&(header_fields.len() as u32).to_le_bytes());
    msg.extend_from_slice(&header_fields);
    align_to(&mut msg, 8);
    msg.extend_from_slice(&body);
    msg
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
    use super::{AccessibleNode, encode_atspi_focus_signal_frame, parse_dbus_unix_socket_path};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    pub fn emit_focus_signal_nonblocking(serial: u32, node: &AccessibleNode) {
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
        let frame = encode_atspi_focus_signal_frame(serial, node);
        let _ = stream.write_all(b"\0AUTH EXTERNAL\r\nBEGIN\r\n");
        let _ = stream.write_all(&frame);
    }
}

// ============================================================================
// Windows UI Automation (`IRawElementProviderSimple`) COM Provider
// ============================================================================

#[cfg(target_os = "windows")]
mod win32_uia {
    use super::*;
    use std::ffi::c_void;

    const S_OK: i32 = 0;
    const E_NOINTERFACE: i32 = -2147467262i32; // 0x80004002
    const E_INVALIDARG: i32 = -2147024809i32; // 0x80070057

    const PROVIDER_OPTIONS_SERVER_SIDE_PROVIDER: u32 = 0x0002;
    const PROVIDER_OPTIONS_USE_COM_THREADING: u32 = 0x0020;

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
    struct StaticUiaProvider {
        lp_vtbl: *const IRawElementProviderSimpleVtbl,
    }

    unsafe impl Sync for StaticUiaProvider {}

    static PROVIDER_VTBL: IRawElementProviderSimpleVtbl = IRawElementProviderSimpleVtbl {
        query_interface: uia_query_interface,
        add_ref: uia_add_ref,
        release: uia_release,
        get_provider_options: uia_get_provider_options,
        get_pattern_provider: uia_get_pattern_provider,
        get_property_value: uia_get_property_value,
        get_host_raw_element_provider: uia_get_host_raw_element_provider,
    };

    static ROOT_PROVIDER: StaticUiaProvider = StaticUiaProvider {
        lp_vtbl: &PROVIDER_VTBL,
    };

    type FnUiaReturnRawElementProvider =
        unsafe extern "system" fn(isize, usize, isize, *mut c_void) -> isize;
    type FnUiaHostProviderFromHwnd = unsafe extern "system" fn(isize, *mut *mut c_void) -> i32;
    type FnUiaRaiseAutomationEvent = unsafe extern "system" fn(*mut c_void, i32) -> i32;
    type FnSysAllocStringLen = unsafe extern "system" fn(*const u16, u32) -> *mut u16;

    struct UiaProcs {
        return_raw_element_provider: Option<FnUiaReturnRawElementProvider>,
        host_provider_from_hwnd: Option<FnUiaHostProviderFromHwnd>,
        raise_automation_event: Option<FnUiaRaiseAutomationEvent>,
        sys_alloc_string_len: Option<FnSysAllocStringLen>,
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
            let h_uia = LoadLibraryW(uia_dll.as_ptr());
            let h_ole = LoadLibraryW(ole_dll.as_ptr());

            let return_raw_element_provider = if !h_uia.is_null() {
                let p = GetProcAddress(h_uia, c"UiaReturnRawElementProvider".as_ptr());
                if !p.is_null() {
                    Some(std::mem::transmute::<
                        *mut c_void,
                        FnUiaReturnRawElementProvider,
                    >(p))
                } else {
                    None
                }
            } else {
                None
            };

            let host_provider_from_hwnd = if !h_uia.is_null() {
                let p = GetProcAddress(h_uia, c"UiaHostProviderFromHwnd".as_ptr());
                if !p.is_null() {
                    Some(std::mem::transmute::<*mut c_void, FnUiaHostProviderFromHwnd>(p))
                } else {
                    None
                }
            } else {
                None
            };

            let raise_automation_event = if !h_uia.is_null() {
                let p = GetProcAddress(h_uia, c"UiaRaiseAutomationEvent".as_ptr());
                if !p.is_null() {
                    Some(std::mem::transmute::<*mut c_void, FnUiaRaiseAutomationEvent>(p))
                } else {
                    None
                }
            } else {
                None
            };

            let sys_alloc_string_len = if !h_ole.is_null() {
                let p = GetProcAddress(h_ole, c"SysAllocStringLen".as_ptr());
                if !p.is_null() {
                    Some(std::mem::transmute::<*mut c_void, FnSysAllocStringLen>(p))
                } else {
                    None
                }
            } else {
                None
            };

            UiaProcs {
                return_raw_element_provider,
                host_provider_from_hwnd,
                raise_automation_event,
                sys_alloc_string_len,
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

    unsafe extern "system" fn uia_query_interface(
        this: *mut c_void,
        riid: *const Guid,
        ppv: *mut *mut c_void,
    ) -> i32 {
        if ppv.is_null() || riid.is_null() {
            return E_INVALIDARG;
        }
        let iid = unsafe { *riid };
        if iid == IID_IUNKNOWN || iid == IID_IRAW_ELEMENT_PROVIDER_SIMPLE {
            unsafe {
                *ppv = this;
            }
            S_OK
        } else {
            unsafe {
                *ppv = std::ptr::null_mut();
            }
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn uia_add_ref(_this: *mut c_void) -> u32 {
        2
    }

    unsafe extern "system" fn uia_release(_this: *mut c_void) -> u32 {
        1
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
        _this: *mut c_void,
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
        let Ok(guard) = bridge_state().lock() else {
            return S_OK;
        };
        let Some(snap) = guard.snapshot.as_ref() else {
            return S_OK;
        };
        let node = &snap.focused_node;
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
            _ => {}
        }
        S_OK
    }

    unsafe extern "system" fn uia_get_host_raw_element_provider(
        _this: *mut c_void,
        p_ret_val: *mut *mut c_void,
    ) -> i32 {
        if p_ret_val.is_null() {
            return E_INVALIDARG;
        }
        unsafe {
            *p_ret_val = std::ptr::null_mut();
        }
        let hwnd = bridge_state().lock().map(|g| g.hwnd).unwrap_or(0);
        if hwnd == 0 {
            return S_OK;
        }
        if let Some(host_fn) = uia_procs().host_provider_from_hwnd {
            return unsafe { host_fn(hwnd, p_ret_val) };
        }
        S_OK
    }

    pub fn provider_ptr() -> *mut c_void {
        (&ROOT_PROVIDER as *const StaticUiaProvider) as *mut c_void
    }

    pub fn return_raw_element_provider(
        hwnd: isize,
        w_param: usize,
        l_param: isize,
    ) -> Option<isize> {
        let f = uia_procs().return_raw_element_provider?;
        Some(unsafe { f(hwnd, w_param, l_param, provider_ptr()) })
    }

    pub fn raise_focus_changed_event() {
        if let Some(f) = uia_procs().raise_automation_event {
            let _ = unsafe { f(provider_ptr(), UIA_AUTOMATION_FOCUS_CHANGED_EVENT_ID) };
        }
    }

    #[cfg(test)]
    pub fn test_query_property_i4(property_id: i32) -> Option<(u16, i32)> {
        let mut var = Variant {
            vt: 0,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            data0: 0,
            data1: 0,
        };
        let hr = unsafe { uia_get_property_value(provider_ptr(), property_id, &mut var) };
        if hr == S_OK {
            Some((var.vt, var.data0 as i32))
        } else {
            None
        }
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
    fn encodes_valid_dbus_atspi_focus_signal_wire_frame() {
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
        session.open_html("<title>UIA Check</title>", "https://example.org/");
        let mut ui = UiState::new();
        ui.focus_chrome(ChromeControl::AppMenu);
        notify_focus_if_changed(0, &session, &ui, 640, 480);
        let (vt, ctrl_type) =
            win32_uia::test_query_property_i4(UIA_CONTROL_TYPE_PROPERTY_ID).unwrap();
        assert_eq!(vt, 3); // VT_I4
        assert_eq!(ctrl_type, UIA_BUTTON_CONTROL_TYPE_ID);
    }
}
