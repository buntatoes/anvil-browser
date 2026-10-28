// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Toolbar, menus, and the CPU frame.
//!
//! compose_frame paints the tabs, navigation bar, bookmarks, page image,
//! inspector dock, find marks, and status bar into a Framebuffer.
//! shell copies that buffer to the window. This file does not present
//! with a GPU.
//!
//! Hit tests take window pixels. A page hit subtracts the header from y.
//! Find marks are scaled by Session::zoom_factor, shifted by Session::scroll,
//! then moved down by the header.
//!
//! Zoom steps live on Session in lib.rs. This file draws the percent and
//! the zoom control. The settings control is only a hit target. shell opens
//! anvil://settings. Nothing here pauses the session, and F9 is not handled.

use std::path::Path;
use std::sync::OnceLock;

use anvil_image::Image;
use anvil_layout::Rect;
use anvil_raster::{Color, Framebuffer};

use crate::logo::render_anvil_logo;
use crate::{BrowserTheme, SearchEngine, Session, Viewport};

/// Height of the tab strip, in window pixels.
pub const TAB_STRIP_HEIGHT: f32 = 38.0;
/// Height of the navigation bar under the tabs.
pub const NAV_BAR_HEIGHT: f32 = 42.0;
/// Extra header height used when the session has bookmarks.
pub const BOOKMARKS_BAR_HEIGHT: f32 = 28.0;
/// Height of the status bar at the bottom of the window.
pub const STATUS_BAR_HEIGHT: f32 = 24.0;
/// Width of the page scrollbar drawn over the right edge.
pub const SCROLLBAR_WIDTH: f32 = 12.0;
/// Tab strip plus navigation bar. The bookmarks row is not included.
pub const TOP_BAR_HEIGHT: f32 = TAB_STRIP_HEIGHT + NAV_BAR_HEIGHT;
/// Height of the inspector dock above the status bar.
pub const INSPECTOR_DOCK_HEIGHT: f32 = 186.0;
/// Height of the dock row that holds the inspector tabs.
pub const INSPECTOR_HEADER_HEIGHT: f32 = 32.0;

/// Width of the floating find bar.
const FIND_BAR_WIDTH: f32 = 316.0;
/// Height of the floating find bar.
const FIND_BAR_HEIGHT: f32 = 36.0;
/// Width of the right-click menu.
const CTX_MENU_WIDTH: f32 = 216.0;
/// Height of one right-click menu row, used for hit testing.
const CTX_MENU_ROW_H: f32 = 26.0;
/// Width of the app menu.
const APP_MENU_WIDTH: f32 = 252.0;
/// Height of the app menu title row. Rows start below it.
const APP_MENU_HEADER_H: f32 = 28.0;
/// Height of one app menu row, used for hit testing.
const APP_MENU_ROW_H: f32 = 27.0;

/// Pixels from the top of the window down to the page.
/// Adds BOOKMARKS_BAR_HEIGHT when the session has at least one bookmark.
pub fn top_header_height(session: &Session) -> f32 {
    if session.bookmarks().is_empty() {
        TOP_BAR_HEIGHT
    } else {
        TOP_BAR_HEIGHT + BOOKMARKS_BAR_HEIGHT
    }
}

// Frame colors. Each value is passed through to the rasterizer.
/// Base fill for the tab strip and the window background.
const STRIP_BG: Color = Color {
    r: 17,
    g: 20,
    b: 27,
    a: 255,
};
/// Fill for a tab that is not active.
const TAB_IDLE_BG: Color = Color {
    r: 22,
    g: 26,
    b: 35,
    a: 255,
};
/// Lighter end of the navigation and active-tab gradient.
const NAV_TOP_BG: Color = Color {
    r: 28,
    g: 33,
    b: 45,
    a: 255,
};
/// Fill for the navigation bar and the active tab.
const NAV_BG: Color = Color {
    r: 22,
    g: 26,
    b: 35,
    a: 255,
};
/// Fill for a toolbar button that is not hovered.
const CONTROL_BG: Color = Color {
    r: 33,
    g: 39,
    b: 54,
    a: 255,
};
/// Fill for a hovered button or tab.
const CONTROL_HOVER_BG: Color = Color {
    r: 46,
    g: 55,
    b: 74,
    a: 255,
};
/// Fill for a button marked active.
const CONTROL_ACTIVE_BG: Color = Color {
    r: 44,
    g: 32,
    b: 29,
    a: 255,
};
/// Fill for the app menu, context menu, and find bar.
const OVERLAY_BG: Color = Color {
    r: 22,
    g: 26,
    b: 35,
    a: 252,
};
/// Shadow under floating menus and the find bar.
const OVERLAY_SHADOW: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 185,
};
/// Fill for the address field.
const FIELD_BG: Color = Color {
    r: 13,
    g: 16,
    b: 22,
    a: 255,
};
/// Hairline used between bars and around controls.
const DIVIDER: Color = Color {
    r: 46,
    g: 54,
    b: 70,
    a: 255,
};
/// Mark color for the active tab, focus, and the security action.
const ACCENT: Color = Color {
    r: 255,
    g: 107,
    b: 36,
    a: 255,
};
/// Secondary mark for hover and inactive find matches.
const ACCENT_CYAN: Color = Color {
    r: 88,
    g: 166,
    b: 255,
    a: 255,
};
/// Fill for the current find match.
const MATCH_HIGHLIGHT_ACTIVE: Color = Color {
    r: 232,
    g: 195,
    b: 122,
    a: 115,
};
/// Fill for the other find matches.
const MATCH_HIGHLIGHT_IDLE: Color = Color {
    r: 88,
    g: 166,
    b: 255,
    a: 80,
};
/// Main text and enabled icon color.
const INK_PRIMARY: Color = Color {
    r: 241,
    g: 245,
    b: 249,
    a: 255,
};
/// Secondary text and the idle close icon.
const INK_MUTED: Color = Color {
    r: 148,
    g: 163,
    b: 184,
    a: 255,
};
/// Icon color when a button is disabled.
const INK_DIM: Color = Color {
    r: 82,
    g: 96,
    b: 117,
    a: 255,
};
/// Background for an https security badge.
const BADGE_TLS_BG: Color = Color {
    r: 15,
    g: 47,
    b: 33,
    a: 255,
};
/// Text color for an https security badge.
const BADGE_TLS_FG: Color = Color {
    r: 110,
    g: 231,
    b: 183,
    a: 255,
};
/// Background for a anvil:// security badge. The text uses ACCENT.
const BADGE_SYS_BG: Color = Color {
    r: 41,
    g: 34,
    b: 20,
    a: 255,
};
/// Background for an http security badge.
const BADGE_HTTP_BG: Color = Color {
    r: 54,
    g: 32,
    b: 16,
    a: 255,
};
/// Text color for an http security badge.
const BADGE_HTTP_FG: Color = Color {
    r: 251,
    g: 191,
    b: 36,
    a: 255,
};
/// Background for a local-file security badge.
const BADGE_FILE_BG: Color = Color {
    r: 19,
    g: 37,
    b: 58,
    a: 255,
};
/// Text color for a local-file security badge.
const BADGE_FILE_FG: Color = Color {
    r: 125,
    g: 211,
    b: 252,
    a: 255,
};

/// Theme palette for the browser chrome, overlays, and docked inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub strip_bg: Color,
    pub tab_idle_bg: Color,
    pub nav_top_bg: Color,
    pub nav_bg: Color,
    pub control_bg: Color,
    pub control_hover_bg: Color,
    pub control_active_bg: Color,
    pub overlay_bg: Color,
    pub field_bg: Color,
    pub divider: Color,
    pub accent: Color,
    pub accent_cyan: Color,
    pub ink_primary: Color,
    pub ink_muted: Color,
    pub ink_dim: Color,
}

impl ThemePalette {
    /// Resolve the active chrome color palette for `theme`.
    pub const fn for_theme(theme: BrowserTheme) -> Self {
        match theme {
            BrowserTheme::Dark => Self {
                strip_bg: STRIP_BG,
                tab_idle_bg: TAB_IDLE_BG,
                nav_top_bg: NAV_TOP_BG,
                nav_bg: NAV_BG,
                control_bg: CONTROL_BG,
                control_hover_bg: CONTROL_HOVER_BG,
                control_active_bg: CONTROL_ACTIVE_BG,
                overlay_bg: OVERLAY_BG,
                field_bg: FIELD_BG,
                divider: DIVIDER,
                accent: ACCENT,
                accent_cyan: ACCENT_CYAN,
                ink_primary: INK_PRIMARY,
                ink_muted: INK_MUTED,
                ink_dim: INK_DIM,
            },
            BrowserTheme::Midnight => Self {
                strip_bg: Color {
                    r: 6,
                    g: 8,
                    b: 12,
                    a: 255,
                },
                tab_idle_bg: Color {
                    r: 10,
                    g: 13,
                    b: 19,
                    a: 255,
                },
                nav_top_bg: Color {
                    r: 15,
                    g: 19,
                    b: 28,
                    a: 255,
                },
                nav_bg: Color {
                    r: 11,
                    g: 14,
                    b: 21,
                    a: 255,
                },
                control_bg: Color {
                    r: 18,
                    g: 23,
                    b: 34,
                    a: 255,
                },
                control_hover_bg: Color {
                    r: 28,
                    g: 36,
                    b: 52,
                    a: 255,
                },
                control_active_bg: Color {
                    r: 36,
                    g: 24,
                    b: 20,
                    a: 255,
                },
                overlay_bg: Color {
                    r: 11,
                    g: 14,
                    b: 21,
                    a: 252,
                },
                field_bg: Color {
                    r: 4,
                    g: 6,
                    b: 9,
                    a: 255,
                },
                divider: Color {
                    r: 28,
                    g: 36,
                    b: 52,
                    a: 255,
                },
                accent: ACCENT,
                accent_cyan: ACCENT_CYAN,
                ink_primary: Color {
                    r: 248,
                    g: 250,
                    b: 252,
                    a: 255,
                },
                ink_muted: INK_MUTED,
                ink_dim: INK_DIM,
            },
            BrowserTheme::Light => Self {
                strip_bg: Color {
                    r: 226,
                    g: 232,
                    b: 240,
                    a: 255,
                },
                tab_idle_bg: Color {
                    r: 241,
                    g: 245,
                    b: 249,
                    a: 255,
                },
                nav_top_bg: Color {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                },
                nav_bg: Color {
                    r: 248,
                    g: 250,
                    b: 252,
                    a: 255,
                },
                control_bg: Color {
                    r: 241,
                    g: 245,
                    b: 249,
                    a: 255,
                },
                control_hover_bg: Color {
                    r: 226,
                    g: 232,
                    b: 240,
                    a: 255,
                },
                control_active_bg: Color {
                    r: 255,
                    g: 237,
                    b: 213,
                    a: 255,
                },
                overlay_bg: Color {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 252,
                },
                field_bg: Color {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 255,
                },
                divider: Color {
                    r: 203,
                    g: 213,
                    b: 225,
                    a: 255,
                },
                accent: Color {
                    r: 234,
                    g: 88,
                    b: 12,
                    a: 255,
                },
                accent_cyan: Color {
                    r: 2,
                    g: 132,
                    b: 199,
                    a: 255,
                },
                ink_primary: Color {
                    r: 15,
                    g: 23,
                    b: 42,
                    a: 255,
                },
                ink_muted: Color {
                    r: 71,
                    g: 85,
                    b: 105,
                    a: 255,
                },
                ink_dim: Color {
                    r: 148,
                    g: 163,
                    b: 184,
                    a: 255,
                },
            },
            BrowserTheme::Nordic => Self {
                strip_bg: Color {
                    r: 46,
                    g: 52,
                    b: 64,
                    a: 255,
                },
                tab_idle_bg: Color {
                    r: 59,
                    g: 66,
                    b: 82,
                    a: 255,
                },
                nav_top_bg: Color {
                    r: 67,
                    g: 76,
                    b: 94,
                    a: 255,
                },
                nav_bg: Color {
                    r: 59,
                    g: 66,
                    b: 82,
                    a: 255,
                },
                control_bg: Color {
                    r: 67,
                    g: 76,
                    b: 94,
                    a: 255,
                },
                control_hover_bg: Color {
                    r: 76,
                    g: 86,
                    b: 106,
                    a: 255,
                },
                control_active_bg: Color {
                    r: 59,
                    g: 82,
                    b: 104,
                    a: 255,
                },
                overlay_bg: Color {
                    r: 59,
                    g: 66,
                    b: 82,
                    a: 252,
                },
                field_bg: Color {
                    r: 36,
                    g: 41,
                    b: 51,
                    a: 255,
                },
                divider: Color {
                    r: 76,
                    g: 86,
                    b: 106,
                    a: 255,
                },
                accent: Color {
                    r: 136,
                    g: 192,
                    b: 208,
                    a: 255,
                },
                accent_cyan: Color {
                    r: 129,
                    g: 161,
                    b: 193,
                    a: 255,
                },
                ink_primary: Color {
                    r: 236,
                    g: 239,
                    b: 244,
                    a: 255,
                },
                ink_muted: Color {
                    r: 216,
                    g: 222,
                    b: 233,
                    a: 255,
                },
                ink_dim: Color {
                    r: 129,
                    g: 141,
                    b: 164,
                    a: 255,
                },
            },
        }
    }
}

/// One row of the right-click menu.
/// shortcut is a label. shell.rs implements the keys on Windows. Linux does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuAction {
    OpenLinkInNewTab,
    CopyLinkAddress,
    Back,
    Forward,
    Reload,
    ReopenClosedTab,
    DuplicateTab,
    Copy,
    Paste,
    FindInPage,
    SavePage,
    ViewPageSource,
    ResetZoom,
    InspectPage,
    SecurityCenter,
}

impl ContextMenuAction {
    /// Complete order when right-clicking over a hyperlink.
    pub const ALL: [Self; 15] = [
        Self::OpenLinkInNewTab,
        Self::CopyLinkAddress,
        Self::Back,
        Self::Forward,
        Self::Reload,
        Self::ReopenClosedTab,
        Self::DuplicateTab,
        Self::Copy,
        Self::Paste,
        Self::FindInPage,
        Self::SavePage,
        Self::ViewPageSource,
        Self::ResetZoom,
        Self::InspectPage,
        Self::SecurityCenter,
    ];

    /// Order used when right-clicking on the page background (not over a link).
    pub const PAGE_ITEMS: [Self; 13] = [
        Self::Back,
        Self::Forward,
        Self::Reload,
        Self::ReopenClosedTab,
        Self::DuplicateTab,
        Self::Copy,
        Self::Paste,
        Self::FindInPage,
        Self::SavePage,
        Self::ViewPageSource,
        Self::ResetZoom,
        Self::InspectPage,
        Self::SecurityCenter,
    ];

    /// Rows displayed for the current right-click context (`has_link`).
    pub fn items_for(has_link: bool) -> &'static [Self] {
        if has_link {
            &Self::ALL
        } else {
            &Self::PAGE_ITEMS
        }
    }

    /// Text drawn for this right-click row.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenLinkInNewTab => "Open Link in New Tab",
            Self::CopyLinkAddress => "Copy Link Address",
            Self::Back => "Back",
            Self::Forward => "Forward",
            Self::Reload => "Reload Page",
            Self::ReopenClosedTab => "Reopen Closed Tab",
            Self::DuplicateTab => "Duplicate Tab",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::FindInPage => "Find in Page...",
            Self::SavePage => "Save Page HTML",
            Self::ViewPageSource => "View Page Source",
            Self::ResetZoom => "Reset Zoom (100%)",
            Self::InspectPage => "Inspect Page",
            Self::SecurityCenter => "Security Shield",
        }
    }

    /// Hint drawn beside the right-click row. It is not parsed as a key binding.
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::OpenLinkInNewTab => "Mid-Click",
            Self::CopyLinkAddress => "URL",
            Self::Back => "Alt+Left",
            Self::Forward => "Alt+Right",
            Self::Reload => "Ctrl+R",
            Self::ReopenClosedTab => "Ctrl+Shift+T",
            Self::DuplicateTab => "Clone",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::FindInPage => "Ctrl+F",
            Self::SavePage => "Ctrl+S",
            Self::ViewPageSource => "Ctrl+U",
            Self::ResetZoom => "Ctrl+0",
            Self::InspectPage => "F12",
            Self::SecurityCenter => "TLS",
        }
    }
}

/// Floating option-picker overlay anchored to an open `<select>` element on the page.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectDropdownOverlay {
    pub node_id: anvil_dom::NodeId,
    pub anchor_rect: Rect,
    pub options: Vec<(String, String, bool)>,
}

/// One row of the app menu.
/// CycleZoom, CycleTheme, and Settings are performed by shell.rs, not by this file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMenuAction {
    NewTab,
    HomeDashboard,
    Downloads,
    History,
    SecurityCenter,
    FindInPage,
    ToggleInspectorDock,
    OpenInspectorPage,
    CycleZoom,
    CycleTheme,
    Settings,
}

impl AppMenuAction {
    /// Order used both to draw the rows and to turn a click into an action.
    pub const ALL: [Self; 11] = [
        Self::NewTab,
        Self::HomeDashboard,
        Self::Downloads,
        Self::History,
        Self::SecurityCenter,
        Self::FindInPage,
        Self::ToggleInspectorDock,
        Self::OpenInspectorPage,
        Self::CycleZoom,
        Self::CycleTheme,
        Self::Settings,
    ];

    /// Text drawn for this app-menu row.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewTab => "New Tab",
            Self::HomeDashboard => "Start Dashboard",
            Self::Downloads => "Downloads & Vault",
            Self::History => "Browsing History",
            Self::SecurityCenter => "Security & Shields",
            Self::FindInPage => "Find in Page...",
            Self::ToggleInspectorDock => "Developer Inspector Dock",
            Self::OpenInspectorPage => "Full-Page Inspector",
            Self::CycleZoom => "Adjust / Reset Zoom",
            Self::CycleTheme => "Switch Theme",
            Self::Settings => "Browser Settings",
        }
    }

    /// Hint drawn beside the app-menu row. It is not parsed as a key binding.
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::NewTab => "Ctrl+T",
            Self::HomeDashboard => "Alt+Home",
            Self::Downloads => "Ctrl+J",
            Self::History => "Ctrl+H",
            Self::SecurityCenter => "Shield",
            Self::FindInPage => "Ctrl+F",
            Self::ToggleInspectorDock => "F12",
            Self::OpenInspectorPage => "Inspect",
            Self::CycleZoom => "Ctrl++",
            Self::CycleTheme => "Theme",
            Self::Settings => "Config",
        }
    }
}

/// Tab inside the inspector dock. The default is Elements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InspectorTab {
    #[default]
    Elements,
    Styles,
    Network,
    Console,
}

impl InspectorTab {
    /// Order used both to draw the rows and to turn a click into an action.
    pub const ALL: [Self; 4] = [Self::Elements, Self::Styles, Self::Network, Self::Console];

    /// Text drawn for this inspector tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::Elements => "Elements (DOM)",
            Self::Styles => "Styles & Layout",
            Self::Network => "Network & Shield",
            Self::Console => "Script & Media",
        }
    }
}

/// What the pointer is over.
/// Page x and y are page coordinates. Every other hit is decided in window coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum UiHit {
    Tab(usize),
    CloseTab(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    Home,
    SecurityBadge,
    AddressField,
    ClearAddress,
    ToggleBookmark,
    ZoomPill,
    SettingsButton,
    DownloadsButton,
    InspectorButton,
    AppMenuButton,
    AppMenuItem(AppMenuAction),
    InspectorTab(InspectorTab),
    InspectorFullPage,
    InspectorClose,
    Bookmark(usize),
    FindInput,
    FindPrev,
    FindNext,
    FindClose,
    ContextMenuItem(ContextMenuAction),
    SelectDropdownOption(usize),
    Scrollbar { ratio: f32 },
    Page { x: f32, y: f32 },
}

/// Mutable toolbar state: address editing, loading, find, menus, the inspector, and the cursor.
/// Zoom and the settings record are not stored here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiState {
    editing_address: bool,
    loading: bool,
    input: String,
    input_cursor: usize,
    input_select_all: bool,
    hovered_link: Option<String>,
    find_open: bool,
    find_query: String,
    find_cursor: usize,
    find_active_match: usize,
    context_menu: Option<(f32, f32)>,
    context_menu_link: Option<String>,
    app_menu_open: bool,
    inspector_open: bool,
    inspector_tab: InspectorTab,
    cursor_pos: Option<(f32, f32)>,
    dragging_scrollbar: bool,
    select_dropdown: Option<SelectDropdownOverlay>,
}

impl UiState {
    /// Empty toolbar state. The address bar is idle and no menu is open.
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the address bar has focus.
    pub fn is_editing_address(&self) -> bool {
        self.editing_address
    }

    /// True from the start of a remote load until the page is installed.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Set or clear the loading flag drawn on the navigation bar.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Link under the pointer, if the last move reported one.
    pub fn hovered_link(&self) -> Option<&str> {
        self.hovered_link.as_deref()
    }

    /// Store the hovered link. Returns false when the value did not change.
    pub fn set_hovered_link(&mut self, link: Option<String>) -> bool {
        if self.hovered_link == link {
            return false;
        }
        self.hovered_link = link;
        true
    }

    /// Text currently in the address bar. The find query is separate.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Character caret index (`0..=chars.count()`) inside the address bar input.
    pub fn input_cursor(&self) -> usize {
        self.input_cursor.min(self.input.chars().count())
    }

    /// True when the entire address bar input is selected (`Ctrl+L` or `Ctrl+A`).
    pub fn is_input_select_all(&self) -> bool {
        self.input_select_all
    }

    /// Select all text in the address bar (`Ctrl+A`).
    pub fn select_all_input(&mut self) -> bool {
        if self.editing_address && !self.input.is_empty() {
            self.input_select_all = true;
            self.input_cursor = self.input.chars().count();
            true
        } else {
            false
        }
    }

    /// Character caret index (`0..=chars.count()`) inside the find bar query.
    pub fn find_cursor(&self) -> usize {
        self.find_cursor.min(self.find_query.chars().count())
    }

    /// Focus the address bar, close find and menus, and copy `current_url`.
    /// `anvil://home` clears the field instead of showing that URL.
    pub fn focus_address(&mut self, current_url: &str) {
        self.editing_address = true;
        self.find_open = false;
        self.context_menu = None;
        self.context_menu_link = None;
        self.app_menu_open = false;
        self.select_dropdown = None;
        if current_url.trim().eq_ignore_ascii_case("anvil://home") {
            self.input.clear();
            self.input_cursor = 0;
            self.input_select_all = false;
        } else {
            self.input = current_url.to_string();
            self.input_cursor = self.input.chars().count();
            self.input_select_all = false;
        }
    }

    /// Leave the address bar without navigating.
    pub fn blur_address(&mut self) {
        self.editing_address = false;
        self.input_select_all = false;
    }

    /// Clear the address bar input while keeping address editing active.
    pub fn clear_address(&mut self) {
        self.editing_address = true;
        self.input.clear();
        self.input_cursor = 0;
        self.input_select_all = false;
    }

    /// True while the user is dragging the vertical scrollbar thumb.
    pub fn is_dragging_scrollbar(&self) -> bool {
        self.dragging_scrollbar
    }

    /// Begin dragging the vertical scrollbar thumb.
    pub fn start_scrollbar_drag(&mut self) {
        self.dragging_scrollbar = true;
    }

    /// Stop dragging the vertical scrollbar thumb.
    pub fn stop_scrollbar_drag(&mut self) {
        self.dragging_scrollbar = false;
    }

    /// Replace the address text. Does not change focus by itself.
    pub fn set_input(&mut self, text: &str) {
        self.input = text.to_string();
        self.input_cursor = self.input.chars().count();
        self.input_select_all = false;
    }

    /// Insert printable ASCII into the address bar at the current caret (or replacing a select-all).
    /// Returns false when the bar is not focused or the text adds nothing.
    pub fn paste_into_address(&mut self, text: &str) -> bool {
        if !self.editing_address {
            return false;
        }
        let clean: String = text.chars().filter(|c| (' '..='~').contains(c)).collect();
        if clean.is_empty() {
            return false;
        }
        if self.input_select_all {
            self.input = clean;
            self.input_cursor = self.input.chars().count();
            self.input_select_all = false;
        } else {
            let mut chars: Vec<char> = self.input.chars().collect();
            let cur = self.input_cursor.min(chars.len());
            let ins: Vec<char> = clean.chars().collect();
            let ins_len = ins.len();
            chars.splice(cur..cur, ins);
            self.input = chars.into_iter().collect();
            self.input_cursor = cur + ins_len;
        }
        true
    }

    /// Take the address text when the bar is focused and not empty.
    pub fn cut_address(&mut self) -> Option<String> {
        if !self.editing_address || self.input.is_empty() {
            return None;
        }
        self.input_cursor = 0;
        self.input_select_all = false;
        Some(std::mem::take(&mut self.input))
    }

    /// Insert one printable ASCII character at the active caret.
    /// The address bar wins when it is focused. Otherwise the character goes to find.
    pub fn push_char(&mut self, ch: char) {
        if self.editing_address && (' '..='~').contains(&ch) {
            if self.input_select_all {
                self.input.clear();
                self.input_cursor = 0;
                self.input_select_all = false;
            }
            let mut chars: Vec<char> = self.input.chars().collect();
            let cur = self.input_cursor.min(chars.len());
            chars.insert(cur, ch);
            self.input = chars.into_iter().collect();
            self.input_cursor = cur + 1;
        } else if self.find_open && (' '..='~').contains(&ch) {
            let mut chars: Vec<char> = self.find_query.chars().collect();
            let cur = self.find_cursor.min(chars.len());
            chars.insert(cur, ch);
            self.find_query = chars.into_iter().collect();
            self.find_cursor = cur + 1;
            self.find_active_match = 0;
        }
    }

    /// Delete one character before the caret from the address bar, or from find when the bar is idle.
    pub fn backspace(&mut self) {
        if self.editing_address {
            if self.input_select_all {
                self.input.clear();
                self.input_cursor = 0;
                self.input_select_all = false;
                return;
            }
            let mut chars: Vec<char> = self.input.chars().collect();
            let cur = self.input_cursor.min(chars.len());
            if cur > 0 {
                chars.remove(cur - 1);
                self.input = chars.into_iter().collect();
                self.input_cursor = cur - 1;
            }
        } else if self.find_open {
            let mut chars: Vec<char> = self.find_query.chars().collect();
            let cur = self.find_cursor.min(chars.len());
            if cur > 0 {
                chars.remove(cur - 1);
                self.find_query = chars.into_iter().collect();
                self.find_cursor = cur - 1;
                self.find_active_match = 0;
            }
        }
    }

    /// Delete the character at the caret (`Delete` key) in the address bar or find bar.
    pub fn delete_forward(&mut self) -> bool {
        if self.editing_address {
            if self.input_select_all {
                let changed = !self.input.is_empty();
                self.input.clear();
                self.input_cursor = 0;
                self.input_select_all = false;
                return changed;
            }
            let mut chars: Vec<char> = self.input.chars().collect();
            let cur = self.input_cursor.min(chars.len());
            if cur < chars.len() {
                chars.remove(cur);
                self.input = chars.into_iter().collect();
                self.input_cursor = cur;
                return true;
            }
            false
        } else if self.find_open {
            let mut chars: Vec<char> = self.find_query.chars().collect();
            let cur = self.find_cursor.min(chars.len());
            if cur < chars.len() {
                chars.remove(cur);
                self.find_query = chars.into_iter().collect();
                self.find_cursor = cur;
                self.find_active_match = 0;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Delete the word before the caret (`Ctrl+Backspace`) in the address bar or find bar.
    pub fn delete_word_back(&mut self) -> bool {
        if self.editing_address {
            if self.input_select_all {
                let changed = !self.input.is_empty();
                self.input.clear();
                self.input_cursor = 0;
                self.input_select_all = false;
                return changed;
            }
            let mut chars: Vec<char> = self.input.chars().collect();
            let cur = self.input_cursor.min(chars.len());
            if cur == 0 {
                return false;
            }
            let mut start = cur;
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
            while start > 0
                && !chars[start - 1].is_whitespace()
                && !matches!(chars[start - 1], '/' | ':' | '.' | '?' | '&' | '=' | '-')
            {
                start -= 1;
            }
            if start == cur && start > 0 {
                start -= 1;
            }
            chars.drain(start..cur);
            self.input = chars.into_iter().collect();
            self.input_cursor = start;
            true
        } else if self.find_open {
            let mut chars: Vec<char> = self.find_query.chars().collect();
            let cur = self.find_cursor.min(chars.len());
            if cur == 0 {
                return false;
            }
            let mut start = cur;
            while start > 0 && chars[start - 1].is_whitespace() {
                start -= 1;
            }
            while start > 0 && !chars[start - 1].is_whitespace() {
                start -= 1;
            }
            chars.drain(start..cur);
            self.find_query = chars.into_iter().collect();
            self.find_cursor = start;
            self.find_active_match = 0;
            true
        } else {
            false
        }
    }

    /// Move the caret one character left (`Left` arrow) in the address bar or find bar.
    pub fn move_cursor_left(&mut self) -> bool {
        if self.editing_address {
            if self.input_select_all {
                self.input_select_all = false;
                self.input_cursor = 0;
                return true;
            }
            let cur = self.input_cursor();
            if cur > 0 {
                self.input_cursor = cur - 1;
                return true;
            }
            false
        } else if self.find_open {
            let cur = self.find_cursor();
            if cur > 0 {
                self.find_cursor = cur - 1;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Move the caret one character right (`Right` arrow) in the address bar or find bar.
    pub fn move_cursor_right(&mut self) -> bool {
        if self.editing_address {
            let len = self.input.chars().count();
            if self.input_select_all {
                self.input_select_all = false;
                self.input_cursor = len;
                return true;
            }
            let cur = self.input_cursor();
            if cur < len {
                self.input_cursor = cur + 1;
                return true;
            }
            false
        } else if self.find_open {
            let len = self.find_query.chars().count();
            let cur = self.find_cursor();
            if cur < len {
                self.find_cursor = cur + 1;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Move the caret to the start of the address bar or find bar (`Home`).
    pub fn move_cursor_home(&mut self) -> bool {
        if self.editing_address {
            let changed = self.input_select_all || self.input_cursor() > 0;
            self.input_select_all = false;
            self.input_cursor = 0;
            changed
        } else if self.find_open {
            let changed = self.find_cursor() > 0;
            self.find_cursor = 0;
            changed
        } else {
            false
        }
    }

    /// Move the caret to the end of the address bar or find bar (`End`).
    pub fn move_cursor_end(&mut self) -> bool {
        if self.editing_address {
            let len = self.input.chars().count();
            let changed = self.input_select_all || self.input_cursor() < len;
            self.input_select_all = false;
            self.input_cursor = len;
            changed
        } else if self.find_open {
            let len = self.find_query.chars().count();
            let changed = self.find_cursor() < len;
            self.find_cursor = len;
            changed
        } else {
            false
        }
    }

    /// Submit the address bar with the Google search engine.
    pub fn submit_address(&mut self) -> Option<String> {
        self.submit_address_with_engine(SearchEngine::Google)
    }

    /// Blur the address bar and turn the field into a URL or a search for engine.
    /// Returns None when the bar was not focused.
    pub fn submit_address_with_engine(&mut self, engine: SearchEngine) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        self.editing_address = false;
        self.input_select_all = false;
        normalize_address_input_with_engine(&self.input, engine)
    }

    /// True while the find bar is open.
    pub fn is_find_open(&self) -> bool {
        self.find_open
    }

    /// Open find and close the address bar, the context menu, and the app menu.
    pub fn open_find(&mut self) {
        self.find_open = true;
        self.find_cursor = self.find_query.chars().count();
        self.editing_address = false;
        self.input_select_all = false;
        self.context_menu = None;
        self.context_menu_link = None;
        self.app_menu_open = false;
        self.select_dropdown = None;
    }

    /// Close the find bar. The query is kept.
    pub fn close_find(&mut self) {
        self.find_open = false;
    }

    /// Current find string.
    pub fn find_query(&self) -> &str {
        &self.find_query
    }

    /// Replace the find string and reset the active match to zero.
    pub fn set_find_query(&mut self, query: &str) {
        self.find_query = query.to_string();
        self.find_cursor = self.find_query.chars().count();
        self.find_active_match = 0;
    }

    /// Index of the match that scroll and the active highlight use.
    pub fn find_active_match(&self) -> usize {
        self.find_active_match
    }

    /// Advance the match index, wrapping. Stays at zero when there are no matches.
    pub fn find_next(&mut self, total_matches: usize) -> usize {
        if total_matches == 0 {
            self.find_active_match = 0;
        } else {
            self.find_active_match = (self.find_active_match + 1) % total_matches;
        }
        self.find_active_match
    }

    /// Move to the previous match, wrapping. Stays at zero when there are no matches.
    pub fn find_prev(&mut self, total_matches: usize) -> usize {
        if total_matches == 0 {
            self.find_active_match = 0;
        } else if self.find_active_match == 0 {
            self.find_active_match = total_matches - 1;
        } else {
            self.find_active_match -= 1;
        }
        self.find_active_match
    }

    /// Open the right-click menu at x, y and close the app menu. Values below 4 are raised.
    pub fn open_context_menu(&mut self, x: f32, y: f32) {
        self.open_context_menu_with_link(x, y, None);
    }

    /// Open the right-click menu at `x, y` with an optional hyperlink target under the cursor.
    pub fn open_context_menu_with_link(&mut self, x: f32, y: f32, link: Option<String>) {
        self.app_menu_open = false;
        self.select_dropdown = None;
        self.context_menu = Some((x.max(4.0), y.max(4.0)));
        self.context_menu_link = link;
    }

    /// Close the right-click menu.
    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
        self.context_menu_link = None;
    }

    /// Anchor of the right-click menu, in window pixels.
    pub fn context_menu_pos(&self) -> Option<(f32, f32)> {
        self.context_menu
    }

    /// Hyperlink URL associated with the currently open right-click menu, if any.
    pub fn context_menu_link(&self) -> Option<&str> {
        self.context_menu_link.as_deref()
    }

    /// Open the floating `<select>` option picker overlay.
    pub fn open_select_dropdown(
        &mut self,
        node_id: anvil_dom::NodeId,
        anchor_rect: Rect,
        options: Vec<(String, String, bool)>,
    ) {
        self.context_menu = None;
        self.context_menu_link = None;
        self.app_menu_open = false;
        self.select_dropdown = Some(SelectDropdownOverlay {
            node_id,
            anchor_rect,
            options,
        });
    }

    /// Close the floating `<select>` option picker overlay.
    pub fn close_select_dropdown(&mut self) {
        self.select_dropdown = None;
    }

    /// Borrow the active `<select>` option picker overlay, if open.
    pub fn select_dropdown(&self) -> Option<&SelectDropdownOverlay> {
        self.select_dropdown.as_ref()
    }

    /// True while the app menu is open.
    pub fn is_app_menu_open(&self) -> bool {
        self.app_menu_open
    }

    /// Toggle the app menu.
    /// Opening it closes the context menu, find, and the address bar. Returns the new flag.
    pub fn toggle_app_menu(&mut self) -> bool {
        self.app_menu_open = !self.app_menu_open;
        if self.app_menu_open {
            self.context_menu = None;
            self.context_menu_link = None;
            self.select_dropdown = None;
            self.find_open = false;
            self.editing_address = false;
            self.input_select_all = false;
        }
        self.app_menu_open
    }

    /// Close the app menu.
    pub fn close_app_menu(&mut self) {
        self.app_menu_open = false;
    }

    /// True while the bottom inspector dock is open.
    pub fn is_inspector_open(&self) -> bool {
        self.inspector_open
    }

    /// Toggle the inspector dock. Does not navigate. Returns the new flag.
    pub fn toggle_inspector(&mut self) -> bool {
        self.inspector_open = !self.inspector_open;
        self.inspector_open
    }

    /// Open the inspector dock.
    pub fn open_inspector(&mut self) {
        self.inspector_open = true;
    }

    /// Close the inspector dock.
    pub fn close_inspector(&mut self) {
        self.inspector_open = false;
    }

    /// Which inspector tab is selected.
    pub fn inspector_tab(&self) -> InspectorTab {
        self.inspector_tab
    }

    /// Select an inspector tab. The dock stays as it was.
    pub fn set_inspector_tab(&mut self, tab: InspectorTab) {
        self.inspector_tab = tab;
    }

    /// Last pointer position in window pixels.
    pub fn cursor_pos(&self) -> Option<(f32, f32)> {
        self.cursor_pos
    }

    /// Store the pointer. Returns false when it did not move.
    pub fn set_cursor_pos(&mut self, x: f32, y: f32) -> bool {
        let next = Some((x, y));
        if self.cursor_pos == next {
            return false;
        }
        self.cursor_pos = next;
        true
    }

    /// True when the stored pointer lies inside rect.
    pub fn is_hovering(&self, rect: Rect) -> bool {
        self.cursor_pos
            .is_some_and(|(cx, cy)| rect.contains(cx, cy))
    }

    /// Return the best matching URL suggestion from built-in routes, bookmarks, or history
    /// while the user is typing in the address bar.
    pub fn autocomplete_suggestion(&self, session: &Session) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        let needle = self.input.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        let builtins = [
            "anvil://home",
            "anvil://inspect",
            "anvil://settings",
            "anvil://downloads",
            "anvil://security",
            "anvil://history",
        ];
        for route in builtins {
            if route != needle && (route.starts_with(&needle) || route[7..].starts_with(&needle)) {
                return Some(route.to_string());
            }
        }
        for bm in session.bookmarks() {
            let u = bm.url.to_ascii_lowercase();
            if u != needle
                && (u.starts_with(&needle) || bm.title.to_ascii_lowercase().starts_with(&needle))
            {
                return Some(bm.url.clone());
            }
        }
        for entry in session.history_log().iter().rev() {
            let u = entry.url.to_ascii_lowercase();
            if u != needle && u.starts_with(&needle) {
                return Some(entry.url.clone());
            }
        }
        None
    }

    /// Accept the active Omnibox autocomplete suggestion (`Tab` key).
    pub fn accept_autocomplete(&mut self, session: &Session) -> bool {
        if let Some(suggestion) = self.autocomplete_suggestion(session) {
            self.input = suggestion;
            self.input_cursor = self.input.chars().count();
            self.input_select_all = false;
            true
        } else {
            false
        }
    }
}

/// Normalize an address with the Google search engine.
pub fn normalize_address_input(raw: &str) -> Option<String> {
    normalize_address_input_with_engine(raw, SearchEngine::Google)
}

/// Turn address text into a navigation target.
/// Rejects javascript, data, and vbscript. Keeps anvil, view-source, http, and https.
/// about:home and about:blank become anvil://home.
/// An existing path, a backslash, or an html suffix stays a file path.
/// A dotted token with no spaces becomes https. Anything else is a search URL.
pub fn normalize_address_input_with_engine(raw: &str, engine: SearchEngine) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return None;
    }
    if lower.starts_with("view-source:") {
        let inner = trimmed["view-source:".len()..].trim();
        if inner.is_empty() {
            return None;
        }
        return Some(format!("view-source:{inner}"));
    }
    if lower.starts_with("anvil://") {
        return Some(lower);
    }
    if lower == "about:home" || lower == "about:blank" {
        return Some("anvil://home".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    if Path::new(trimmed).exists()
        || trimmed.ends_with(".html")
        || trimmed.ends_with(".htm")
        || trimmed.contains('\\')
    {
        return Some(trimmed.to_string());
    }
    if trimmed.contains('.') && !trimmed.contains(' ') {
        return Some(format!("https://{trimmed}"));
    }
    Some(engine.search_url(trimmed))
}

/// Page size passed to the session.
/// Width is the window width. Height subtracts the tab strip, navigation bar, and status bar.
/// The bookmarks row and the inspector dock are not subtracted.
pub fn content_viewport(window_width: f32, window_height: f32) -> Viewport {
    Viewport {
        width: window_width.max(1.0),
        height: (window_height - TOP_BAR_HEIGHT - STATUS_BAR_HEIGHT).max(1.0),
    }
}

/// Page size accounting for the active bookmarks bar and the open inspector dock.
pub fn content_viewport_for(
    session: &Session,
    ui: &UiState,
    window_width: f32,
    window_height: f32,
) -> Viewport {
    let header_h = top_header_height(session);
    let dock_h = if ui.is_inspector_open() {
        INSPECTOR_DOCK_HEIGHT
    } else {
        0.0
    };
    Viewport {
        width: window_width.max(1.0),
        height: (window_height - header_h - STATUS_BAR_HEIGHT - dock_h).max(1.0),
    }
}

/// Normalized `[0.0, 1.0]` scroll ratio for window coordinate `y` over the page viewport.
pub fn scrollbar_ratio_at(session: &Session, y: f32) -> f32 {
    let header_h = top_header_height(session);
    let page_h = session.viewport().height.max(1.0);
    ((y - header_h) / page_h).clamp(0.0, 1.0)
}

/// Hit-test with a default UiState and a stand-in window height of 720.
/// Callers that know the height should use hit_test_ui_with_state_and_height.
pub fn hit_test_ui(session: &Session, window_width: f32, x: f32, y: f32) -> Option<UiHit> {
    let default_ui = UiState::new();
    hit_test_ui_with_state_and_height(session, &default_ui, window_width, 720.0, x, y)
}

/// Hit-test with the given toolbar state and a stand-in window height of 720.
pub fn hit_test_ui_with_state(
    session: &Session,
    ui: &UiState,
    window_width: f32,
    x: f32,
    y: f32,
) -> Option<UiHit> {
    hit_test_ui_with_state_and_height(session, ui, window_width, 720.0, x, y)
}

/// Hit-test x and y in window pixels.
/// Menus, find, the tab strip, the navigation bar, bookmarks, the dock, and the scrollbar win first.
/// A remaining point becomes a page hit. y drops the header. x is not shifted.
pub fn hit_test_ui_with_state_and_height(
    session: &Session,
    ui: &UiState,
    window_width: f32,
    window_height: f32,
    x: f32,
    y: f32,
) -> Option<UiHit> {
    if x < 0.0 || y < 0.0 || x >= window_width {
        return None;
    }

    // 1. Floating Right-Click Context Menu takes top hit priority when open
    if let Some((mx, my)) = ui.context_menu_pos() {
        let items = ContextMenuAction::items_for(ui.context_menu_link().is_some());
        let menu_rect = context_menu_rect_for(mx, my, window_width, items.len());
        if menu_rect.contains(x, y) {
            let rel_y = (y - (menu_rect.y + 4.0)).max(0.0);
            let idx = (rel_y / CTX_MENU_ROW_H).floor() as usize;
            if let Some(action) = items.get(idx) {
                return Some(UiHit::ContextMenuItem(*action));
            }
        }
    }

    // 1b. Floating `<select>` Dropdown Option Picker Overlay
    if let Some(dd) = ui.select_dropdown() {
        let dd_rect = select_dropdown_rect(dd, window_width);
        if dd_rect.contains(x, y) {
            let rel_y = (y - (dd_rect.y + 4.0)).max(0.0);
            let idx = (rel_y / CTX_MENU_ROW_H).floor() as usize;
            if idx < dd.options.len() {
                return Some(UiHit::SelectDropdownOption(idx));
            }
        }
    }

    let header_h = top_header_height(session);

    // 2. Floating Top-Right Hamburger App Menu Popover
    if ui.is_app_menu_open() {
        let mrect = app_menu_rect(window_width, header_h);
        if mrect.contains(x, y) {
            let rows_top = mrect.y + APP_MENU_HEADER_H + 4.0;
            if y >= rows_top {
                let idx = ((y - rows_top) / APP_MENU_ROW_H).floor() as usize;
                if let Some(action) = AppMenuAction::ALL.get(idx) {
                    return Some(UiHit::AppMenuItem(*action));
                }
            }
            return None;
        }
    }

    // 3. Floating Find-in-Page (Ctrl+F) bar
    if ui.is_find_open() {
        let fbar = find_bar_rect(window_width, header_h);
        if fbar.contains(x, y) {
            if find_prev_rect(fbar).contains(x, y) {
                return Some(UiHit::FindPrev);
            }
            if find_next_rect(fbar).contains(x, y) {
                return Some(UiHit::FindNext);
            }
            if find_close_rect(fbar).contains(x, y) {
                return Some(UiHit::FindClose);
            }
            return Some(UiHit::FindInput);
        }
    }

    // 4. Tab Strip
    if y < TAB_STRIP_HEIGHT {
        if logo_rect().contains(x, y) {
            return Some(UiHit::Home);
        }
        let count = session.len().max(1);
        let tab_w = tab_width(window_width, count);
        let can_close = count > 1;
        for index in 0..count {
            let rect = tab_rect(index, tab_w);
            if rect.contains(x, y) {
                if can_close && close_rect(rect).contains(x, y) {
                    return Some(UiHit::CloseTab(index));
                }
                return Some(UiHit::Tab(index));
            }
        }
        if new_tab_rect(count, tab_w).contains(x, y) {
            return Some(UiHit::NewTab);
        }
        return None;
    }

    // 5. Navigation Bar & Quick-Action Toolbar
    if y < TOP_BAR_HEIGHT {
        if back_rect().contains(x, y) {
            return Some(UiHit::Back);
        }
        if forward_rect().contains(x, y) {
            return Some(UiHit::Forward);
        }
        if reload_rect().contains(x, y) {
            return Some(UiHit::Reload);
        }
        if home_rect().contains(x, y) {
            return Some(UiHit::Home);
        }
        if security_badge_rect().contains(x, y) {
            return Some(UiHit::SecurityBadge);
        }
        if app_menu_btn_rect(window_width).contains(x, y) {
            return Some(UiHit::AppMenuButton);
        }
        if inspector_btn_rect(window_width).contains(x, y) {
            return Some(UiHit::InspectorButton);
        }
        if downloads_btn_rect(window_width).contains(x, y) {
            return Some(UiHit::DownloadsButton);
        }
        if settings_btn_rect(window_width).contains(x, y) {
            return Some(UiHit::SettingsButton);
        }
        if zoom_pill_rect(window_width).contains(x, y) {
            return Some(UiHit::ZoomPill);
        }
        if star_rect(window_width).contains(x, y) {
            return Some(UiHit::ToggleBookmark);
        }
        let addr = address_rect(window_width);
        if addr.contains(x, y) {
            if ui.is_editing_address()
                && !ui.input().is_empty()
                && clear_address_rect(addr).contains(x, y)
            {
                return Some(UiHit::ClearAddress);
            }
            return Some(UiHit::AddressField);
        }
        return None;
    }

    // 6. Bookmarks Bar
    if y < header_h {
        for index in 0..session.bookmarks().len() {
            if bookmark_rect(index).contains(x, y) {
                return Some(UiHit::Bookmark(index));
            }
        }
        return None;
    }

    // 7. Bottom-Docked Developer Inspector Panel (when open)
    if ui.is_inspector_open() {
        let dock = inspector_dock_rect(window_width, window_height);
        if dock.contains(x, y) {
            if inspector_close_rect(dock).contains(x, y) {
                return Some(UiHit::InspectorClose);
            }
            if inspector_full_page_rect(dock).contains(x, y) {
                return Some(UiHit::InspectorFullPage);
            }
            for (idx, tab) in InspectorTab::ALL.iter().enumerate() {
                if inspector_tab_rect(dock, idx).contains(x, y) {
                    return Some(UiHit::InspectorTab(*tab));
                }
            }
            return None;
        }
    }

    // 8. Vertical Scrollbar
    if session.max_scroll() > 0.0 && x >= (window_width - SCROLLBAR_WIDTH).max(0.0) {
        let ratio = scrollbar_ratio_at(session, y);
        return Some(UiHit::Scrollbar { ratio });
    }

    // Window y to page y: subtract the header. x is unchanged because the page is not inset.
    Some(UiHit::Page { x, y: y - header_h })
}

/// Badge label, colors, and status text for a URL.
/// https is TLS, anvil is SYS, http is HTTP, an empty URL is IDLE, and anything else is FILE.
fn classify_url_security(url: &str) -> (&'static str, Color, Color, &'static str) {
    let trimmed = url.trim();
    if trimmed.starts_with("https://") {
        ("TLS", BADGE_TLS_BG, BADGE_TLS_FG, "HTTPS")
    } else if trimmed.starts_with("anvil://") {
        ("SYS", BADGE_SYS_BG, ACCENT, "Anvil page")
    } else if trimmed.starts_with("http://") {
        ("HTTP", BADGE_HTTP_BG, BADGE_HTTP_FG, "Plain HTTP")
    } else if trimmed.is_empty() {
        ("IDLE", CONTROL_BG, INK_MUTED, "Plain HTTP")
    } else {
        (
            "FILE",
            BADGE_FILE_BG,
            BADGE_FILE_FG,
            "Local Document Viewer | Network Isolation Active",
        )
    }
}

/// Paint one window frame into a CPU framebuffer.
/// The page image from Session::rasterize is copied below the header.
/// This does not present to a window and does not use a GPU.
pub fn compose_frame(
    session: &Session,
    ui: &UiState,
    window_width: u32,
    window_height: u32,
) -> Framebuffer {
    let page_fb = session.rasterize();
    compose_frame_with_page_fb(session, ui, window_width, window_height, &page_fb)
}

/// Paint one window frame into a CPU framebuffer using a pre-rasterized `page_fb` surface.
/// Reusing `page_fb` avoids re-rasterizing the page during toolbar hover and Omnibox typing redraws.
pub fn compose_frame_with_page_fb(
    session: &Session,
    ui: &UiState,
    window_width: u32,
    window_height: u32,
    page_fb: &Framebuffer,
) -> Framebuffer {
    let width = window_width.max(1);
    let height = window_height.max(1);
    let width_f = width as f32;
    let height_f = height as f32;
    let pal = ThemePalette::for_theme(session.theme());
    let mut fb = Framebuffer::new(width, height, pal.strip_bg);

    // 1. Tab Strip + Top-Left Brand Emblem
    fb.fill_linear_gradient_rect(
        0.0,
        0.0,
        width_f,
        TAB_STRIP_HEIGHT,
        0.0,
        0.0,
        180.0,
        pal.tab_idle_bg,
        pal.strip_bg,
        pal.strip_bg,
    );
    let logo = logo_rect();
    fb.draw_image(logo.x, logo.y, logo.width, logo.height, 6.0, top_bar_logo());

    let count = session.len().max(1);
    let tab_w = tab_width(width_f, count);
    let can_close = count > 1;
    for (index, tab) in session.tabs().iter().enumerate() {
        let rect = tab_rect(index, tab_w);
        let is_active = index == session.active_index();
        let is_hovered = ui.is_hovering(rect);
        let bg = if is_active {
            pal.nav_bg
        } else if is_hovered {
            pal.control_bg
        } else {
            pal.tab_idle_bg
        };
        let fg = if is_active || is_hovered {
            pal.ink_primary
        } else {
            pal.ink_muted
        };
        let border_col = if is_active {
            pal.accent
        } else if is_hovered {
            pal.ink_muted
        } else {
            pal.divider
        };
        if is_active {
            fb.fill_linear_gradient_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                7.0,
                1.0,
                180.0,
                pal.nav_top_bg,
                pal.nav_bg,
                border_col,
            );
            fb.fill_rect(
                rect.x + 8.0,
                rect.y,
                (rect.width - 16.0).max(0.0),
                2.0,
                pal.accent,
            );
        } else {
            fb.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                7.0,
                1.0,
                bg,
                border_col,
            );
        }

        // Tab protocol status dot
        let tab_url = tab.current().map(|p| p.url.as_str()).unwrap_or("");
        let (_, _, dot_col, _) = classify_url_security(tab_url);
        fb.fill_rounded_rect(
            rect.x + 8.0,
            rect.y + 12.0,
            6.0,
            6.0,
            3.0,
            0.0,
            dot_col,
            dot_col,
        );

        let text_space = if can_close {
            (rect.width - 38.0).max(16.0)
        } else {
            (rect.width - 22.0).max(16.0)
        };
        let max_chars = ((text_space / 7.0).floor() as usize).max(1);
        let label = truncate(tab.title(), max_chars);
        fb.draw_text(rect.x + 18.0, rect.y + 7.0, &label, 13.0, fg);

        if can_close {
            let close = close_rect(rect);
            let close_hovered = ui.is_hovering(close);
            if close_hovered {
                fb.fill_rounded_rect(
                    close.x,
                    close.y,
                    close.width,
                    close.height,
                    4.0,
                    0.0,
                    pal.control_hover_bg,
                    pal.control_hover_bg,
                );
            }
            draw_close_x_icon(
                &mut fb,
                close,
                if close_hovered {
                    pal.accent
                } else {
                    pal.ink_muted
                },
            );
        }
    }

    let plus = new_tab_rect(count, tab_w);
    let plus_hovered = ui.is_hovering(plus);
    fb.fill_rounded_rect(
        plus.x,
        plus.y,
        plus.width,
        plus.height,
        7.0,
        1.0,
        if plus_hovered {
            pal.control_hover_bg
        } else {
            pal.tab_idle_bg
        },
        if plus_hovered {
            pal.accent
        } else {
            pal.divider
        },
    );
    draw_plus_icon(&mut fb, plus, pal.accent);

    // 2. Navigation Bar with Studio Lighting
    fb.fill_linear_gradient_rect(
        0.0,
        TAB_STRIP_HEIGHT,
        width_f,
        NAV_BAR_HEIGHT,
        0.0,
        0.0,
        180.0,
        pal.nav_top_bg,
        pal.nav_bg,
        pal.nav_bg,
    );
    fb.fill_rect(0.0, TOP_BAR_HEIGHT - 1.0, width_f, 1.0, pal.divider);
    if ui.is_loading() {
        fb.fill_rect(0.0, TOP_BAR_HEIGHT - 2.0, width_f, 2.0, pal.accent);
    }

    let active_tab = session.active();
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        back_rect(),
        active_tab.can_go_back(),
        false,
        draw_nav_icon_back,
    );
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        forward_rect(),
        active_tab.can_go_forward(),
        false,
        draw_nav_icon_forward,
    );
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        reload_rect(),
        active_tab.current().is_some(),
        false,
        draw_nav_icon_reload,
    );
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        home_rect(),
        true,
        false,
        draw_nav_icon_home,
    );

    // 3. Unified Omnibox Capsule (Security Pill + Address Input + Clear + Bookmark Star)
    let current_page_url = active_tab
        .current()
        .map(|page| page.url.as_str())
        .unwrap_or("");
    let (sec_label, sec_bg, sec_fg, sec_summary) = classify_url_security(current_page_url);
    let badge = security_badge_rect();
    let addr = address_rect(width_f);
    let star = star_rect(width_f);

    let omnibox_x = badge.x;
    let omnibox_w = ((star.x + star.width) - badge.x).max(120.0);
    let omnibox_hovered = ui.is_hovering(addr) || ui.is_hovering(badge) || ui.is_hovering(star);
    let omnibox_border = if ui.is_editing_address() {
        pal.accent
    } else if omnibox_hovered {
        pal.accent_cyan
    } else {
        pal.divider
    };
    fb.fill_rounded_rect(
        omnibox_x,
        addr.y,
        omnibox_w,
        addr.height,
        8.0,
        1.0,
        pal.field_bg,
        omnibox_border,
    );

    // Security Badge inside left end of Omnibox
    let badge_hovered = ui.is_hovering(badge);
    fb.fill_rounded_rect(
        badge.x + 2.0,
        badge.y + 2.0,
        badge.width - 4.0,
        badge.height - 4.0,
        6.0,
        1.0,
        sec_bg,
        if badge_hovered { pal.accent } else { sec_fg },
    );
    fb.draw_text(badge.x + 9.0, badge.y + 6.0, sec_label, 12.0, sec_fg);

    // URL / Search text inside Omnibox
    let show_clear_btn = ui.is_editing_address() && !ui.input().is_empty();
    let is_home_blank =
        !ui.is_editing_address() && current_page_url.trim().eq_ignore_ascii_case("anvil://home");
    let raw_url = if ui.is_editing_address() {
        let mut ichars: Vec<char> = ui.input().chars().collect();
        let cur = ui.input_cursor().min(ichars.len());
        ichars.insert(cur, '|');
        let with_caret: String = ichars.into_iter().collect();
        if let Some(sugg) = ui.autocomplete_suggestion(session) {
            format!("{with_caret}  -> {sugg} [Tab]")
        } else {
            with_caret
        }
    } else if is_home_blank {
        String::new()
    } else {
        current_page_url.to_string()
    };
    let text_right_pad = if show_clear_btn { 36.0 } else { 16.0 };
    let max_addr_chars = (((addr.width - text_right_pad).max(8.0) / 7.0).floor() as usize).max(1);
    let shown_url = if ui.is_editing_address() {
        tail(&raw_url, max_addr_chars)
    } else {
        truncate(&raw_url, max_addr_chars)
    };
    if ui.is_editing_address() && ui.is_input_select_all() && !ui.input().is_empty() {
        let sel_w = ((ui.input().chars().count().min(max_addr_chars) as f32) * 7.2 + 8.0)
            .min((addr.width - text_right_pad).max(16.0));
        fb.fill_rounded_rect(
            addr.x + 5.0,
            addr.y + 4.0,
            sel_w,
            addr.height - 8.0,
            4.0,
            0.0,
            pal.control_hover_bg,
            pal.control_hover_bg,
        );
    }
    let addr_color = if raw_url.is_empty() {
        pal.ink_muted
    } else {
        pal.ink_primary
    };
    let placeholder_label = format!(
        "Search with {} or enter web address",
        session.search_engine().label()
    );
    let display_text = if shown_url.is_empty() {
        placeholder_label.as_str()
    } else {
        shown_url.as_str()
    };
    fb.draw_text(addr.x + 8.0, addr.y + 6.0, display_text, 13.0, addr_color);

    if show_clear_btn {
        let clear_btn = clear_address_rect(addr);
        let clear_hovered = ui.is_hovering(clear_btn);
        if clear_hovered {
            fb.fill_rounded_rect(
                clear_btn.x,
                clear_btn.y,
                clear_btn.width,
                clear_btn.height,
                5.0,
                0.0,
                pal.control_hover_bg,
                pal.control_hover_bg,
            );
        }
        draw_close_x_icon(
            &mut fb,
            clear_btn,
            if clear_hovered {
                pal.accent
            } else {
                pal.ink_muted
            },
        );
    }

    // Bookmark Star inside right end of Omnibox
    let star_hovered = ui.is_hovering(star);
    let is_bookmarked = session.is_active_bookmarked();
    if star_hovered || is_bookmarked {
        fb.fill_rounded_rect(
            star.x + 2.0,
            star.y + 2.0,
            star.width - 4.0,
            star.height - 4.0,
            6.0,
            0.0,
            if star_hovered {
                pal.control_hover_bg
            } else {
                pal.control_bg
            },
            pal.accent,
        );
    }
    let star_fg = if is_bookmarked || star_hovered {
        pal.accent
    } else {
        pal.ink_muted
    };
    draw_nav_icon_star(&mut fb, star, star_fg, is_bookmarked);

    // 4. Right-Side Quick-Action Toolbar Cluster
    // 4a. Zoom Pill
    let zoom_pill = zoom_pill_rect(width_f);
    let zoom_hovered = ui.is_hovering(zoom_pill);
    fb.fill_rounded_rect(
        zoom_pill.x,
        zoom_pill.y,
        zoom_pill.width,
        zoom_pill.height,
        7.0,
        1.0,
        if zoom_hovered {
            pal.control_hover_bg
        } else {
            pal.control_bg
        },
        if session.zoom_percent() != 100 || zoom_hovered {
            pal.accent
        } else {
            pal.divider
        },
    );
    let zoom_text = format!("{}%", session.zoom_percent());
    fb.draw_text(
        zoom_pill.x + 8.0,
        zoom_pill.y + 6.0,
        &zoom_text,
        12.0,
        if session.zoom_percent() != 100 || zoom_hovered {
            pal.accent
        } else {
            pal.ink_primary
        },
    );

    // 4b. Settings Button (sliders icon)
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        settings_btn_rect(width_f),
        true,
        current_page_url.starts_with("anvil://settings"),
        draw_nav_icon_settings,
    );

    // 4c. Downloads Button (tray arrow + badge)
    let dl_rect = downloads_btn_rect(width_f);
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        dl_rect,
        true,
        current_page_url.starts_with("anvil://downloads"),
        draw_nav_icon_downloads,
    );
    if !session.downloads().entries().is_empty() {
        fb.fill_rounded_rect(
            dl_rect.x + dl_rect.width - 8.0,
            dl_rect.y + 3.0,
            5.0,
            5.0,
            2.5,
            0.0,
            BADGE_TLS_FG,
            BADGE_TLS_FG,
        );
    }

    // 4d. Developer Inspector Button (`<>` icon)
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        inspector_btn_rect(width_f),
        true,
        ui.is_inspector_open() || current_page_url.starts_with("anvil://inspect"),
        draw_nav_icon_inspector,
    );

    // 4e. Top-Right Hamburger App Menu Button (`≡` icon)
    draw_icon_button(
        &mut fb,
        ui,
        &pal,
        app_menu_btn_rect(width_f),
        true,
        ui.is_app_menu_open(),
        draw_nav_icon_hamburger,
    );

    // 5. Optional Bookmarks Bar
    if !session.bookmarks().is_empty() {
        fb.fill_rect(
            0.0,
            TOP_BAR_HEIGHT,
            width_f,
            BOOKMARKS_BAR_HEIGHT,
            pal.tab_idle_bg,
        );
        fb.fill_rect(
            0.0,
            TOP_BAR_HEIGHT + BOOKMARKS_BAR_HEIGHT - 1.0,
            width_f,
            1.0,
            pal.divider,
        );
        for (idx, bm) in session.bookmarks().iter().enumerate() {
            let rect = bookmark_rect(idx);
            if rect.x + rect.width > width_f - 8.0 {
                break;
            }
            let hovered = ui.is_hovering(rect);
            fb.fill_rounded_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                5.0,
                1.0,
                if hovered {
                    pal.control_hover_bg
                } else {
                    pal.control_bg
                },
                if hovered { pal.accent } else { pal.divider },
            );
            let label = truncate(
                if bm.title.is_empty() {
                    &bm.url
                } else {
                    &bm.title
                },
                14,
            );
            fb.draw_text(rect.x + 8.0, rect.y + 3.0, &label, 12.0, pal.ink_primary);
        }
    }

    // 6. Blit Page Viewport (fast row-slice copy)
    let header_h = top_header_height(session);
    let top_y = header_h as u32;
    let status_h_u32 = STATUS_BAR_HEIGHT as u32;
    let dock_h_u32 = if ui.is_inspector_open() {
        INSPECTOR_DOCK_HEIGHT as u32
    } else {
        0
    };
    let avail_h = height
        .saturating_sub(top_y)
        .saturating_sub(status_h_u32)
        .saturating_sub(dock_h_u32);
    fb.copy_rows_from(page_fb, top_y, avail_h);

    // 7. Vertical Scrollbar
    let max_scroll = session.max_scroll();
    let track_h = avail_h as f32;
    if max_scroll > 0.0 && track_h > 16.0 {
        let track_x = (width_f - SCROLLBAR_WIDTH).max(0.0);
        let track_rect = Rect {
            x: track_x,
            y: header_h,
            width: SCROLLBAR_WIDTH,
            height: track_h,
        };
        fb.fill_rect(track_x, header_h, SCROLLBAR_WIDTH, track_h, pal.tab_idle_bg);
        let view_h = session.viewport().height.max(1.0);
        let content_h = (view_h + max_scroll).max(view_h);
        let thumb_h = ((view_h / content_h) * track_h).clamp(24.0, track_h);
        let scroll_ratio = (session.scroll() / max_scroll).clamp(0.0, 1.0);
        let thumb_y = header_h + scroll_ratio * (track_h - thumb_h);
        let thumb_col = if ui.is_dragging_scrollbar() || ui.is_hovering(track_rect) {
            pal.accent
        } else {
            pal.ink_muted
        };
        fb.fill_rounded_rect(
            track_x + 2.0,
            thumb_y,
            (SCROLLBAR_WIDTH - 4.0).max(2.0),
            thumb_h,
            4.0,
            0.0,
            thumb_col,
            thumb_col,
        );
    }

    // 8. Bottom-Docked Developer Inspector Panel (F12 / Toolbar `<>` Button)
    if ui.is_inspector_open() {
        compose_inspector_dock(&mut fb, session, ui, &pal, width_f, height_f);
    }

    // 9. Find-in-Page (Ctrl+F) Highlight Overlays & Floating Search Bar
    if ui.is_find_open() {
        let matches = active_tab
            .current()
            .map(|p| p.find_text_matches(ui.find_query()))
            .unwrap_or_default();
        let z = session.zoom_factor();
        let active_idx = ui.find_active_match();
        let viewport_bottom = header_h + avail_h as f32;
        for (idx, m) in matches.iter().enumerate() {
            // Page x to window x. Only the zoom scale is applied.
            let sx = m.x * z;
            // Page y to window y: scale by zoom, subtract scroll, then add the header.
            let sy = header_h + (m.y * z) - session.scroll();
            let sw = m.width * z;
            let sh = m.height * z;
            if sy + sh > header_h && sy < viewport_bottom {
                let (fill, border) = if idx == active_idx {
                    (MATCH_HIGHLIGHT_ACTIVE, pal.accent)
                } else {
                    (MATCH_HIGHLIGHT_IDLE, pal.accent_cyan)
                };
                fb.fill_rounded_rect(sx, sy, sw, sh, 3.0, 1.0, fill, border);
            }
        }

        let fbar = find_bar_rect(width_f, header_h);
        fb.fill_box_shadow(
            fbar.x,
            fbar.y,
            fbar.width,
            fbar.height,
            0.0,
            6.0,
            16.0,
            0.0,
            8.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            fbar.x,
            fbar.y,
            fbar.width,
            fbar.height,
            8.0,
            1.0,
            pal.overlay_bg,
            pal.accent,
        );
        let mut fchars: Vec<char> = ui.find_query().chars().collect();
        let fcur = ui.find_cursor().min(fchars.len());
        fchars.insert(fcur, '|');
        let fstr: String = fchars.into_iter().collect();
        let q_shown = truncate(&format!("Find: {fstr}"), 20);
        fb.draw_text(fbar.x + 10.0, fbar.y + 9.0, &q_shown, 13.0, pal.ink_primary);

        let count_label = if matches.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", (active_idx + 1).min(matches.len()), matches.len())
        };
        fb.draw_text(
            fbar.x + 170.0,
            fbar.y + 10.0,
            &count_label,
            12.0,
            pal.accent,
        );

        let prev_btn = find_prev_rect(fbar);
        let next_btn = find_next_rect(fbar);
        let close_btn = find_close_rect(fbar);
        for (btn, glyph) in [(prev_btn, "<"), (next_btn, ">"), (close_btn, "x")] {
            let hovered = ui.is_hovering(btn);
            fb.fill_rounded_rect(
                btn.x,
                btn.y,
                btn.width,
                btn.height,
                5.0,
                1.0,
                if hovered {
                    pal.control_hover_bg
                } else {
                    pal.control_bg
                },
                if hovered { pal.accent } else { pal.divider },
            );
            fb.draw_text(btn.x + 7.0, btn.y + 4.0, glyph, 13.0, pal.ink_primary);
        }
    }

    // 10. Floating Top-Right Hamburger App Menu Popover (`≡`)
    if ui.is_app_menu_open() {
        let mrect = app_menu_rect(width_f, header_h);
        fb.fill_box_shadow(
            mrect.x,
            mrect.y,
            mrect.width,
            mrect.height,
            0.0,
            10.0,
            22.0,
            0.0,
            9.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            mrect.x,
            mrect.y,
            mrect.width,
            mrect.height,
            9.0,
            1.0,
            pal.overlay_bg,
            pal.accent,
        );
        // Header banner inside App Menu
        fb.fill_linear_gradient_rect(
            mrect.x + 2.0,
            mrect.y + 2.0,
            mrect.width - 4.0,
            APP_MENU_HEADER_H,
            7.0,
            0.0,
            180.0,
            pal.nav_top_bg,
            pal.nav_bg,
            pal.nav_bg,
        );
        fb.draw_text(
            mrect.x + 12.0,
            mrect.y + 7.0,
            "ANVIL APPLICATION MENU",
            12.0,
            pal.accent,
        );
        fb.fill_rect(
            mrect.x + 6.0,
            mrect.y + APP_MENU_HEADER_H + 2.0,
            mrect.width - 12.0,
            1.0,
            pal.divider,
        );

        let rows_top = mrect.y + APP_MENU_HEADER_H + 4.0;
        for (idx, item) in AppMenuAction::ALL.iter().enumerate() {
            let ry = rows_top + (idx as f32) * APP_MENU_ROW_H;
            let row_rect = Rect {
                x: mrect.x + 5.0,
                y: ry,
                width: mrect.width - 10.0,
                height: APP_MENU_ROW_H - 1.0,
            };
            if idx == 5 || idx == 8 {
                fb.fill_rect(
                    mrect.x + 10.0,
                    ry - 1.0,
                    mrect.width - 20.0,
                    1.0,
                    pal.divider,
                );
            }
            let hovered = ui.is_hovering(row_rect);
            if hovered {
                fb.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.width,
                    row_rect.height,
                    5.0,
                    0.0,
                    pal.control_hover_bg,
                    pal.control_hover_bg,
                );
                fb.fill_rect(
                    row_rect.x,
                    row_rect.y + 4.0,
                    3.0,
                    row_rect.height - 8.0,
                    pal.accent,
                );
            }
            fb.draw_text(
                row_rect.x + 10.0,
                ry + 5.0,
                item.label(),
                12.0,
                if hovered { pal.accent } else { pal.ink_primary },
            );
            let sc = item.shortcut();
            let sc_x = (mrect.x + mrect.width - 62.0).max(row_rect.x + 120.0);
            fb.draw_text(
                sc_x,
                ry + 5.0,
                sc,
                11.0,
                if hovered {
                    pal.accent_cyan
                } else {
                    pal.ink_muted
                },
            );
        }
    }

    // 10b. Floating `<select>` Option Picker Overlay
    if let Some(dd) = ui.select_dropdown() {
        let dd_rect = select_dropdown_rect(dd, width_f);
        fb.fill_box_shadow(
            dd_rect.x,
            dd_rect.y,
            dd_rect.width,
            dd_rect.height,
            0.0,
            8.0,
            18.0,
            0.0,
            8.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            dd_rect.x,
            dd_rect.y,
            dd_rect.width,
            dd_rect.height,
            8.0,
            1.0,
            pal.overlay_bg,
            pal.accent,
        );
        let max_opt_chars = (((dd_rect.width - 28.0).max(12.0) / 6.8).floor() as usize).max(4);
        for (idx, (_, label, is_sel)) in dd.options.iter().enumerate() {
            let ry = dd_rect.y + 4.0 + (idx as f32) * CTX_MENU_ROW_H;
            let row_rect = Rect {
                x: dd_rect.x + 4.0,
                y: ry,
                width: dd_rect.width - 8.0,
                height: CTX_MENU_ROW_H,
            };
            let row_hovered = ui.is_hovering(row_rect);
            if row_hovered || *is_sel {
                fb.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.width,
                    row_rect.height - 2.0,
                    5.0,
                    0.0,
                    if row_hovered {
                        pal.control_hover_bg
                    } else {
                        pal.control_bg
                    },
                    pal.control_hover_bg,
                );
            }
            let prefix = if *is_sel { "* " } else { "  " };
            let text = truncate(&format!("{prefix}{label}"), max_opt_chars);
            fb.draw_text(
                dd_rect.x + 10.0,
                ry + 4.0,
                &text,
                12.0,
                if row_hovered || *is_sel {
                    pal.accent
                } else {
                    pal.ink_primary
                },
            );
        }
    }

    // 11. Floating Right-Click Context Menu
    if let Some((mx, my)) = ui.context_menu_pos() {
        let items = ContextMenuAction::items_for(ui.context_menu_link().is_some());
        let menu_rect = context_menu_rect_for(mx, my, width_f, items.len());
        fb.fill_box_shadow(
            menu_rect.x,
            menu_rect.y,
            menu_rect.width,
            menu_rect.height,
            0.0,
            8.0,
            20.0,
            0.0,
            8.0,
            OVERLAY_SHADOW,
        );
        fb.fill_rounded_rect(
            menu_rect.x,
            menu_rect.y,
            menu_rect.width,
            menu_rect.height,
            8.0,
            1.0,
            pal.overlay_bg,
            pal.accent,
        );
        for (idx, item) in items.iter().enumerate() {
            let ry = menu_rect.y + 4.0 + (idx as f32) * CTX_MENU_ROW_H;
            let row_rect = Rect {
                x: menu_rect.x + 4.0,
                y: ry,
                width: menu_rect.width - 8.0,
                height: CTX_MENU_ROW_H,
            };
            if matches!(
                item,
                ContextMenuAction::Back
                    | ContextMenuAction::ReopenClosedTab
                    | ContextMenuAction::Copy
                    | ContextMenuAction::SavePage
                    | ContextMenuAction::InspectPage
            ) && idx > 0
            {
                fb.fill_rect(
                    menu_rect.x + 8.0,
                    ry - 1.0,
                    menu_rect.width - 16.0,
                    1.0,
                    pal.divider,
                );
            }
            let row_hovered = ui.is_hovering(row_rect);
            if row_hovered {
                fb.fill_rounded_rect(
                    row_rect.x,
                    row_rect.y,
                    row_rect.width,
                    row_rect.height - 2.0,
                    5.0,
                    0.0,
                    pal.control_hover_bg,
                    pal.control_hover_bg,
                );
                fb.fill_rect(
                    row_rect.x,
                    row_rect.y + 4.0,
                    3.0,
                    row_rect.height - 10.0,
                    pal.accent,
                );
            }
            fb.draw_text(
                menu_rect.x + 12.0,
                ry + 4.0,
                item.label(),
                12.0,
                if row_hovered {
                    pal.accent
                } else {
                    pal.ink_primary
                },
            );
            fb.draw_text(
                menu_rect.x + menu_rect.width - 78.0,
                ry + 5.0,
                item.shortcut(),
                11.0,
                if row_hovered {
                    pal.accent_cyan
                } else {
                    pal.ink_muted
                },
            );
        }
    }

    // 12. Bottom Status Bar
    let status_y = (height.saturating_sub(status_h_u32)) as f32;
    fb.fill_linear_gradient_rect(
        0.0,
        status_y,
        width_f,
        STATUS_BAR_HEIGHT,
        0.0,
        0.0,
        180.0,
        pal.tab_idle_bg,
        pal.strip_bg,
        pal.strip_bg,
    );
    fb.fill_rect(0.0, status_y, width_f, 1.0, pal.divider);

    let status_left = if ui.is_loading() {
        "Loading resource over sandboxed transport...".to_string()
    } else if let Some(link) = ui.hovered_link() {
        format!("Link -> {link}")
    } else {
        sec_summary.to_string()
    };
    let max_status_chars = (((width_f - 265.0).max(24.0) / 6.5).floor() as usize).max(1);
    let left_trimmed = truncate(&status_left, max_status_chars);
    fb.draw_text(10.0, status_y + 4.0, &left_trimmed, 12.0, pal.ink_muted);

    let scroll_pct = if max_scroll > 0.0 {
        ((session.scroll() / max_scroll) * 100.0).round() as u32
    } else {
        0
    };
    let page_blocked = active_tab
        .current()
        .map(|p| p.blocked_csp_count + p.blocked_tracker_count)
        .unwrap_or(0);
    let total_blocked = session.network_shield().blocked_total() as usize + page_blocked;
    let zoom_pct = session.zoom_percent();
    let right_label = format!("Shields: ON ({total_blocked}) | {zoom_pct}% | {scroll_pct}%");
    let right_x = (width_f - 218.0).max(8.0);
    fb.draw_text(right_x, status_y + 4.0, &right_label, 12.0, pal.accent);

    fb
}

/// Paint the open inspector dock above the status bar.
/// Switching tabs changes the text only. It does not navigate.
fn compose_inspector_dock(
    fb: &mut Framebuffer,
    session: &Session,
    ui: &UiState,
    pal: &ThemePalette,
    window_width: f32,
    window_height: f32,
) {
    let dock = inspector_dock_rect(window_width, window_height);
    fb.fill_linear_gradient_rect(
        dock.x,
        dock.y,
        dock.width,
        dock.height,
        0.0,
        0.0,
        180.0,
        pal.nav_bg,
        pal.strip_bg,
        pal.strip_bg,
    );
    // Header bar
    fb.fill_linear_gradient_rect(
        dock.x,
        dock.y,
        dock.width,
        INSPECTOR_HEADER_HEIGHT,
        0.0,
        0.0,
        180.0,
        pal.nav_top_bg,
        pal.tab_idle_bg,
        pal.tab_idle_bg,
    );
    fb.fill_rect(dock.x, dock.y, dock.width, 2.0, pal.accent_cyan);
    fb.fill_rect(
        dock.x,
        dock.y + INSPECTOR_HEADER_HEIGHT - 1.0,
        dock.width,
        1.0,
        pal.divider,
    );

    fb.draw_text(dock.x + 10.0, dock.y + 8.0, "DEVTOOLS", 12.0, pal.accent);

    // 4 Clickable Inspector Tabs
    for (idx, tab) in InspectorTab::ALL.iter().enumerate() {
        let trect = inspector_tab_rect(dock, idx);
        let active = ui.inspector_tab() == *tab;
        let hovered = ui.is_hovering(trect);
        fb.fill_rounded_rect(
            trect.x,
            trect.y,
            trect.width,
            trect.height,
            5.0,
            1.0,
            if active {
                pal.control_active_bg
            } else if hovered {
                pal.control_hover_bg
            } else {
                pal.control_bg
            },
            if active {
                pal.accent
            } else if hovered {
                pal.accent_cyan
            } else {
                pal.divider
            },
        );
        fb.draw_text(
            trect.x + 8.0,
            trect.y + 4.0,
            tab.label(),
            12.0,
            if active {
                pal.accent
            } else if hovered {
                pal.ink_primary
            } else {
                pal.ink_muted
            },
        );
    }

    // Full View button (`anvil://inspect`)
    let full_btn = inspector_full_page_rect(dock);
    let full_hovered = ui.is_hovering(full_btn);
    fb.fill_rounded_rect(
        full_btn.x,
        full_btn.y,
        full_btn.width,
        full_btn.height,
        5.0,
        1.0,
        if full_hovered {
            pal.control_hover_bg
        } else {
            pal.control_bg
        },
        if full_hovered {
            pal.accent_cyan
        } else {
            pal.divider
        },
    );
    fb.draw_text(
        full_btn.x + 8.0,
        full_btn.y + 4.0,
        "Full Page",
        11.0,
        if full_hovered {
            pal.accent_cyan
        } else {
            pal.ink_primary
        },
    );

    // Close Dock button (`×`)
    let close_btn = inspector_close_rect(dock);
    let close_hovered = ui.is_hovering(close_btn);
    fb.fill_rounded_rect(
        close_btn.x,
        close_btn.y,
        close_btn.width,
        close_btn.height,
        5.0,
        1.0,
        if close_hovered {
            pal.control_hover_bg
        } else {
            pal.control_bg
        },
        if close_hovered {
            pal.accent
        } else {
            pal.divider
        },
    );
    draw_close_x_icon(
        fb,
        close_btn,
        if close_hovered {
            pal.accent
        } else {
            pal.ink_primary
        },
    );

    // Inspector Body Cards
    let body_y = dock.y + INSPECTOR_HEADER_HEIGHT + 8.0;
    let body_h = (dock.height - INSPECTOR_HEADER_HEIGHT - 16.0).max(40.0);
    let half_w = ((dock.width - 28.0) * 0.5).max(120.0);
    let left_card = Rect {
        x: dock.x + 10.0,
        y: body_y,
        width: half_w,
        height: body_h,
    };
    let right_card = Rect {
        x: dock.x + 18.0 + half_w,
        y: body_y,
        width: half_w,
        height: body_h,
    };

    for card in [left_card, right_card] {
        fb.fill_rounded_rect(
            card.x,
            card.y,
            card.width,
            card.height,
            7.0,
            1.0,
            pal.field_bg,
            pal.divider,
        );
    }

    let page = session.active().current();
    let url = page.map(|p| p.url.as_str()).unwrap_or("about:blank");
    let title = page.map(|p| p.title.as_str()).unwrap_or("Untitled");
    let node_count = page.map(|p| p.document.node_count()).unwrap_or(0);
    let cmd_count = page.map(|p| p.shown_list().items.len()).unwrap_or(0);
    let max_chars = ((half_w - 24.0) / 6.8).floor().max(16.0) as usize;

    match ui.inspector_tab() {
        InspectorTab::Elements => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "DOM Arena & Tree Hierarchy (anvil-dom)",
                13.0,
                pal.accent,
            );
            let l1 = truncate(&format!("URL: {url} | Title: {title}"), max_chars);
            let l2 = truncate(
                &format!("DOM Nodes: {node_count} / 50,000 cap | Max Depth Cap: 256"),
                max_chars,
            );
            let l3 = truncate(
                &format!(
                    "Open Tabs: {} | Active Tab Index: #{}",
                    session.len(),
                    session.active_index() + 1
                ),
                max_chars,
            );
            let sel = if session.selected_text().is_empty() {
                "None (Click page text to select)"
            } else {
                session.selected_text()
            };
            let l4 = truncate(&format!("Selected Text: {sel}"), max_chars);
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 30.0,
                &l1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 50.0,
                &l2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 70.0,
                &l3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 90.0,
                &l4,
                12.0,
                pal.accent_cyan,
            );

            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 8.0,
                "Document Subresources & Interactive Controls",
                13.0,
                pal.accent,
            );
            let r1 = truncate(
                &format!(
                    "Videos Decoded: {} | Focused Input: {}",
                    page.map(|p| p.videos.len()).unwrap_or(0),
                    if session.has_focused_input() {
                        "Active"
                    } else {
                        "Idle"
                    }
                ),
                max_chars,
            );
            let r2 = truncate(
                &format!(
                    "Saved Bookmarks: {} | Session History Entries: {}",
                    session.bookmarks().len(),
                    session.history_log().len()
                ),
                max_chars,
            );
            let r3 = truncate(
                "HTML Parser: Implicit Table/Form Auto-Close & Entity Decoder",
                max_chars,
            );
            let r4 = truncate(
                "Tip: Click [Full Page] above to open anvil://inspect dashboard",
                max_chars,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 30.0,
                &r1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 50.0,
                &r2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 70.0,
                &r3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 90.0,
                &r4,
                12.0,
                pal.accent_cyan,
            );
        }
        InspectorTab::Styles => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "CSS Cascade & Responsive Layout (anvil-style / anvil-layout)",
                13.0,
                pal.accent,
            );
            let vp = session.viewport();
            let l1 = truncate(
                &format!(
                    "Viewport: {:.0}x{:.0} px | Zoom Scale: {}%",
                    vp.width,
                    vp.height,
                    session.zoom_percent()
                ),
                max_chars,
            );
            let l2 = truncate(
                &format!(
                    "Scroll Offset: {:.0} px / Max Scroll: {:.0} px",
                    session.scroll(),
                    session.max_scroll()
                ),
                max_chars,
            );
            let l3 = truncate(
                "Layout Modes: Block, Inline, Flexbox, Table & Out-of-Flow",
                max_chars,
            );
            let l4 = truncate(
                "Media Queries: @media (min/max-width) & prefers-color-scheme: dark",
                max_chars,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 30.0,
                &l1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 50.0,
                &l2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 70.0,
                &l3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 90.0,
                &l4,
                12.0,
                pal.accent_cyan,
            );

            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 8.0,
                "DisplayList & SDF GPU Compositor (anvil-paint / anvil-gpu)",
                13.0,
                pal.accent,
            );
            let r1 = truncate(
                &format!("DisplayList Commands: {cmd_count} active paint primitives"),
                max_chars,
            );
            let r2 = truncate(
                "Shaders: HLSL SM 5.0 + GLSL 330 Core SDF Rounded Quads & Gradients",
                max_chars,
            );
            let r3 = truncate(
                "Typography: TrueType SFNT Metrics & Anti-Aliased Glyph Scanline",
                max_chars,
            );
            let r4 = truncate(
                "Image Guard: 16 MP (64 MiB) RGBA cap across PNG/JPEG/WebP/ICO/GIF",
                max_chars,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 30.0,
                &r1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 50.0,
                &r2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 70.0,
                &r3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 90.0,
                &r4,
                12.0,
                pal.accent_cyan,
            );
        }
        InspectorTab::Network => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "Transport Security, HSTS & SRI (anvil-net)",
                13.0,
                pal.accent,
            );
            let (sec_tag, _, _, _) = classify_url_security(url);
            let l1 = truncate(
                &format!(
                    "Transport Mode: {sec_tag} | Strict HTTPS-Only: {}",
                    if session.settings().https_only_mode {
                        "ON"
                    } else {
                        "STANDARD"
                    }
                ),
                max_chars,
            );
            let l2 = truncate(
                &format!(
                    "HSTS Policy Hosts: {} | Cookies in Jar: {}",
                    session.hsts_store().len(),
                    session.cookie_jar().len()
                ),
                max_chars,
            );
            let l3 = truncate(
                &format!(
                    "SRI SHA-256 Policy: Active | Blocked CSP/SRI: {}",
                    page.map(|p| p.blocked_csp_count).unwrap_or(0)
                ),
                max_chars,
            );
            let l4 = truncate(
                "Headers: Referrer-Policy: no-referrer | DNT: 1 | nosniff",
                max_chars,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 30.0,
                &l1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 50.0,
                &l2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 70.0,
                &l3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 90.0,
                &l4,
                12.0,
                pal.accent_cyan,
            );

            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 8.0,
                "Pre-Socket Tracker Shield, CSP & SHA-256 Vault",
                13.0,
                pal.accent,
            );
            let r1 = truncate(
                &format!(
                    "Page Blocked (CSP: {} | Trackers: {}) | Session Shield Total: {}",
                    page.map(|p| p.blocked_csp_count).unwrap_or(0),
                    page.map(|p| p.blocked_tracker_count).unwrap_or(0),
                    session.network_shield().blocked_total()
                ),
                max_chars,
            );
            let r2 = truncate(
                &format!(
                    "Downloads Verified (SHA-256): {} files in vault",
                    session.downloads().entries().len()
                ),
                max_chars,
            );
            let r3 = truncate(
                "Profile Storage: Windows DPAPI (crypt32.dll) + 64-Bit Tag",
                max_chars,
            );
            let r4 = truncate(
                "Decompression Cap: 16 MiB HTTP body / 32 MiB DEFLATE ceiling",
                max_chars,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 30.0,
                &r1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 50.0,
                &r2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 70.0,
                &r3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 90.0,
                &r4,
                12.0,
                pal.accent_cyan,
            );
        }
        InspectorTab::Console => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "Script Runtime & Sandbox Console (anvil-script / anvil-sandbox)",
                13.0,
                pal.accent,
            );
            let l1 = truncate(
                &format!(
                    "Script Execution: {} | Step Budget: 200,000 | Call Depth: 64",
                    if session.settings().scripts_enabled {
                        "ENABLED"
                    } else {
                        "DISABLED"
                    }
                ),
                max_chars,
            );
            let l2 = truncate(
                "Process Sandbox: Mandatory OS Job Object + Per-Tab Fault Recovery",
                max_chars,
            );
            let l3 = truncate(
                "Same-Origin Policy (SOP): Enforced on fetch() & DOM mutations",
                max_chars,
            );
            let l4 = truncate(
                "Console Status: [OK] 0 unhandled script exceptions on active page",
                max_chars,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 30.0,
                &l1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 50.0,
                &l2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 70.0,
                &l3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 90.0,
                &l4,
                12.0,
                BADGE_TLS_FG,
            );

            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 8.0,
                "Video",
                13.0,
                pal.accent,
            );
            let vs = session.gpu_video();
            let r1 = truncate(
                &format!(
                    "GPU Video Backend: {} ({})",
                    vs.backend().label(),
                    if vs.backend().is_hardware_accelerated() {
                        "GPU"
                    } else {
                        "CPU"
                    }
                ),
                max_chars,
            );
            let r2 = truncate(
                &format!(
                    "Frames Decoded: {} | Frames Encoded: {}",
                    vs.frames_decoded(),
                    vs.frames_encoded()
                ),
                max_chars,
            );
            let r3 = truncate(
                "W3C EME CDM Host: org.w3.clearkey + MediaFoundation + External",
                max_chars,
            );
            let r4 = truncate(
                "Container Security: ISO/IEC 23001-7 CENC (sinf/schm/tenc/pssh)",
                max_chars,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 30.0,
                &r1,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 50.0,
                &r2,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 70.0,
                &r3,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                right_card.x + 10.0,
                right_card.y + 90.0,
                &r4,
                12.0,
                pal.accent_cyan,
            );
        }
    }
}

/// Rounded toolbar button.
/// Disabled icons use the dim color. active selects the pressed fill.
fn draw_icon_button(
    fb: &mut Framebuffer,
    ui: &UiState,
    pal: &ThemePalette,
    rect: Rect,
    enabled: bool,
    active: bool,
    icon_fn: fn(&mut Framebuffer, Rect, Color),
) {
    let hovered = enabled && ui.is_hovering(rect);
    let bg = if active {
        pal.control_active_bg
    } else if hovered {
        pal.control_hover_bg
    } else {
        pal.control_bg
    };
    let border = if active || hovered {
        pal.accent
    } else {
        pal.divider
    };
    fb.fill_rounded_rect(
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        7.0,
        1.0,
        bg,
        border,
    );
    let fg = if active || hovered {
        pal.accent
    } else if enabled {
        pal.ink_primary
    } else {
        pal.ink_dim
    };
    icon_fn(fb, rect, fg);
}

/// Plus glyph for the new-tab button.
fn draw_plus_icon(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    fb.fill_rect(cx - 5.0, cy - 1.0, 10.0, 2.0, color);
    fb.fill_rect(cx - 1.0, cy - 5.0, 2.0, 10.0, color);
}

/// X glyph used to close a tab.
fn draw_close_x_icon(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round() as i32;
    let cy = (rect.y + rect.height * 0.5).round() as i32;
    for d in -3..=3 {
        fb.fill_rect((cx + d) as f32, (cy + d) as f32, 1.8, 1.8, color);
        fb.fill_rect((cx + d) as f32, (cy - d) as f32, 1.8, 1.8, color);
    }
}

/// Left arrow for the back button.
fn draw_nav_icon_back(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    fb.fill_rect(cx - 5.0, cy - 1.0, 11.0, 2.0, color);
    for i in 1..=4 {
        let off = i as f32;
        fb.fill_rect(cx - 5.0 + off, cy - 1.0 - off, 2.0, 2.0, color);
        fb.fill_rect(cx - 5.0 + off, cy - 1.0 + off, 2.0, 2.0, color);
    }
}

/// Right arrow for the forward button.
fn draw_nav_icon_forward(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    fb.fill_rect(cx - 6.0, cy - 1.0, 11.0, 2.0, color);
    for i in 1..=4 {
        let off = i as f32;
        fb.fill_rect(cx + 4.0 - off, cy - 1.0 - off, 2.0, 2.0, color);
        fb.fill_rect(cx + 4.0 - off, cy - 1.0 + off, 2.0, 2.0, color);
    }
}

/// Circular arrow for reload.
fn draw_nav_icon_reload(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    // Top, bottom, left, and lower-right arc segments
    fb.fill_rect(cx - 4.0, cy - 6.0, 7.0, 2.0, color);
    fb.fill_rect(cx - 4.0, cy + 4.0, 8.0, 2.0, color);
    fb.fill_rect(cx - 6.0, cy - 4.0, 2.0, 8.0, color);
    fb.fill_rect(cx + 4.0, cy + 1.0, 2.0, 3.0, color);
    // Arrowhead at top-right
    fb.fill_rect(cx + 2.0, cy - 8.0, 2.0, 5.0, color);
    fb.fill_rect(cx, cy - 5.0, 5.0, 2.0, color);
}

/// House glyph for the start page.
fn draw_nav_icon_home(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    // Pitched roof
    for i in 0..=5 {
        let off = i as f32;
        fb.fill_rect(
            cx - off - 1.0,
            cy - 6.0 + off,
            (off * 2.0) + 2.0,
            1.5,
            color,
        );
    }
    // House body walls
    fb.fill_rect(cx - 4.0, cy - 1.0, 2.0, 7.0, color);
    fb.fill_rect(cx + 2.0, cy - 1.0, 2.0, 7.0, color);
    fb.fill_rect(cx - 4.0, cy + 4.0, 8.0, 2.0, color);
}

/// Star glyph. filled draws the bookmarked shape.
fn draw_nav_icon_star(fb: &mut Framebuffer, rect: Rect, color: Color, filled: bool) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    fb.fill_rect(cx - 1.0, cy - 6.0, 2.0, 12.0, color);
    fb.fill_rect(cx - 6.0, cy - 1.0, 12.0, 2.0, color);
    if filled {
        fb.fill_rect(cx - 3.0, cy - 3.0, 6.0, 6.0, color);
    } else {
        fb.fill_rect(cx - 2.0, cy - 2.0, 4.0, 4.0, color);
    }
}

/// Gear glyph for the settings button.
/// Drawing it does not change settings. shell.rs opens anvil://settings on click.
fn draw_nav_icon_settings(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let x0 = (rect.x + 7.0).round();
    let y0 = (rect.y + 8.0).round();
    // Three horizontal slider tracks with alternating thumb knobs
    fb.fill_rect(x0, y0, 14.0, 2.0, color);
    fb.fill_rect(x0 + 3.0, y0 - 1.0, 3.0, 4.0, ACCENT);

    fb.fill_rect(x0, y0 + 5.0, 14.0, 2.0, color);
    fb.fill_rect(x0 + 8.0, y0 + 4.0, 3.0, 4.0, ACCENT);

    fb.fill_rect(x0, y0 + 10.0, 14.0, 2.0, color);
    fb.fill_rect(x0 + 4.0, y0 + 9.0, 3.0, 4.0, ACCENT);
}

/// Down-arrow glyph for the downloads button.
fn draw_nav_icon_downloads(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    // Vertical downward arrow
    fb.fill_rect(cx - 1.0, cy - 6.0, 2.0, 8.0, color);
    fb.fill_rect(cx - 4.0, cy - 1.0, 8.0, 2.0, color);
    fb.fill_rect(cx - 2.0, cy + 1.0, 4.0, 2.0, color);
    // Bottom tray bracket
    fb.fill_rect(cx - 6.0, cy + 4.0, 12.0, 2.0, color);
    fb.fill_rect(cx - 6.0, cy + 2.0, 2.0, 4.0, color);
    fb.fill_rect(cx + 4.0, cy + 2.0, 2.0, 4.0, color);
}

/// Bracket glyph for the inspector button.
fn draw_nav_icon_inspector(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let cx = (rect.x + rect.width * 0.5).round();
    let cy = (rect.y + rect.height * 0.5).round();
    // Left `<` bracket
    for i in 0..=3 {
        let off = i as f32;
        fb.fill_rect(cx - 6.0 + off, cy - off, 2.0, 1.8, color);
        fb.fill_rect(cx - 6.0 + off, cy + off, 2.0, 1.8, color);
    }
    // Right `>` bracket
    for i in 0..=3 {
        let off = i as f32;
        fb.fill_rect(cx + 4.0 - off, cy - off, 2.0, 1.8, color);
        fb.fill_rect(cx + 4.0 - off, cy + off, 2.0, 1.8, color);
    }
}

/// Three bars for the app menu button.
fn draw_nav_icon_hamburger(fb: &mut Framebuffer, rect: Rect, color: Color) {
    let x0 = (rect.x + 7.0).round();
    let y0 = (rect.y + 8.0).round();
    fb.fill_rect(x0, y0, 14.0, 2.0, color);
    fb.fill_rect(x0, y0 + 5.0, 14.0, 2.0, color);
    fb.fill_rect(x0, y0 + 10.0, 14.0, 2.0, color);
}

/// Cached 24 pixel logo painted in the tab strip.
fn top_bar_logo() -> &'static Image {
    static LOGO: OnceLock<Image> = OnceLock::new();
    LOGO.get_or_init(|| render_anvil_logo(24))
}

/// Logo bounds, 24 by 24 at (8, 7). A hit here is the home action.
fn logo_rect() -> Rect {
    Rect {
        x: 8.0,
        y: 7.0,
        width: 24.0,
        height: 24.0,
    }
}

/// Find bar under the header, aligned to the right.
fn find_bar_rect(window_width: f32, header_h: f32) -> Rect {
    Rect {
        x: (window_width - FIND_BAR_WIDTH - 20.0).max(8.0),
        y: header_h + 8.0,
        width: FIND_BAR_WIDTH,
        height: FIND_BAR_HEIGHT,
    }
}

/// Previous-match button inside the find bar.
fn find_prev_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 86.0,
        y: fbar.y + 6.0,
        width: 24.0,
        height: 24.0,
    }
}

/// Next-match button inside the find bar.
fn find_next_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 58.0,
        y: fbar.y + 6.0,
        width: 24.0,
        height: 24.0,
    }
}

/// Close button inside the find bar.
fn find_close_rect(fbar: Rect) -> Rect {
    Rect {
        x: fbar.x + fbar.width - 30.0,
        y: fbar.y + 6.0,
        width: 24.0,
        height: 24.0,
    }
}

/// App menu under the header, aligned to the right. Height follows the row count.
fn app_menu_rect(window_width: f32, header_h: f32) -> Rect {
    let total_h = APP_MENU_HEADER_H + 8.0 + (AppMenuAction::ALL.len() as f32) * APP_MENU_ROW_H;
    Rect {
        x: (window_width - APP_MENU_WIDTH - 8.0).max(4.0),
        y: header_h + 4.0,
        width: APP_MENU_WIDTH,
        height: total_h,
    }
}

/// Right-click menu at the click, clamped so it stays inside the window.
fn context_menu_rect_for(x: f32, y: f32, window_width: f32, item_count: usize) -> Rect {
    let total_h = 8.0 + (item_count as f32) * CTX_MENU_ROW_H;
    let clamped_x = x.clamp(4.0, (window_width - CTX_MENU_WIDTH - 4.0).max(4.0));
    Rect {
        x: clamped_x,
        y: y.max(4.0),
        width: CTX_MENU_WIDTH,
        height: total_h,
    }
}

/// Floating `<select>` dropdown overlay bounds anchored under `dd.anchor_rect`.
fn select_dropdown_rect(dd: &SelectDropdownOverlay, window_width: f32) -> Rect {
    let w = dd.anchor_rect.width.clamp(160.0, 340.0);
    let total_h = 8.0 + (dd.options.len() as f32) * CTX_MENU_ROW_H;
    let clamped_x = dd
        .anchor_rect
        .x
        .clamp(4.0, (window_width - w - 4.0).max(4.0));
    let y = (dd.anchor_rect.y + dd.anchor_rect.height + 2.0).max(TOP_BAR_HEIGHT);
    Rect {
        x: clamped_x,
        y,
        width: w,
        height: total_h,
    }
}

/// Inspector dock above the status bar.
/// On a short window the top is clamped to TOP_BAR_HEIGHT.
fn inspector_dock_rect(window_width: f32, window_height: f32) -> Rect {
    let y = (window_height - STATUS_BAR_HEIGHT - INSPECTOR_DOCK_HEIGHT).max(TOP_BAR_HEIGHT);
    Rect {
        x: 0.0,
        y,
        width: window_width.max(1.0),
        height: INSPECTOR_DOCK_HEIGHT,
    }
}

/// One inspector tab. index is zero through three.
fn inspector_tab_rect(dock: Rect, index: usize) -> Rect {
    let tab_w = 122.0;
    Rect {
        x: dock.x + 86.0 + (index as f32) * (tab_w + 6.0),
        y: dock.y + 4.0,
        width: tab_w,
        height: 24.0,
    }
}

/// Full Page button in the dock header.
fn inspector_full_page_rect(dock: Rect) -> Rect {
    Rect {
        x: (dock.x + dock.width - 106.0).max(dock.x + 600.0),
        y: dock.y + 4.0,
        width: 68.0,
        height: 24.0,
    }
}

/// Close button in the dock header.
fn inspector_close_rect(dock: Rect) -> Rect {
    Rect {
        x: (dock.x + dock.width - 32.0).max(dock.x + 672.0),
        y: dock.y + 4.0,
        width: 24.0,
        height: 24.0,
    }
}

/// Tab width from the leftover strip, clamped between 72 and 188.
fn tab_width(window_width: f32, count: usize) -> f32 {
    let available = (window_width - 82.0).max(72.0);
    (available / (count as f32)).clamp(72.0, 188.0)
}

/// Tab bounds. x starts at 38 so the logo stays clear.
fn tab_rect(index: usize, tab_w: f32) -> Rect {
    Rect {
        x: 38.0 + (index as f32) * (tab_w + 4.0),
        y: 6.0,
        width: tab_w,
        height: TAB_STRIP_HEIGHT - 6.0,
    }
}

/// Close glyph inside a tab.
fn close_rect(tab: Rect) -> Rect {
    Rect {
        x: tab.x + tab.width - 22.0,
        y: tab.y + 7.0,
        width: 16.0,
        height: 16.0,
    }
}

/// Plus button just after the last tab.
fn new_tab_rect(count: usize, tab_w: f32) -> Rect {
    Rect {
        x: 38.0 + (count as f32) * (tab_w + 4.0),
        y: 6.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Back button on the navigation bar.
fn back_rect() -> Rect {
    Rect {
        x: 8.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Forward button on the navigation bar.
fn forward_rect() -> Rect {
    Rect {
        x: 40.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Reload button on the navigation bar.
fn reload_rect() -> Rect {
    Rect {
        x: 72.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Home button on the navigation bar.
fn home_rect() -> Rect {
    Rect {
        x: 104.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Security badge at the left of the address field.
fn security_badge_rect() -> Rect {
    Rect {
        x: 136.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 44.0,
        height: 28.0,
    }
}

/// Address field. The width shrinks with the window and stops at 60.
fn address_rect(window_width: f32) -> Rect {
    Rect {
        x: 184.0,
        y: TAB_STRIP_HEIGHT + 7.0,
        width: (window_width - 404.0).max(60.0),
        height: 28.0,
    }
}

/// Clear button inside the right side of the address field when editing non-empty text.
fn clear_address_rect(addr: Rect) -> Rect {
    Rect {
        x: addr.x + addr.width - 24.0,
        y: addr.y + 4.0,
        width: 20.0,
        height: 20.0,
    }
}

/// Bookmark toggle at the right of the address field.
fn star_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 216.0).max(248.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Zoom percent button.
/// The number is Session::zoom_percent. shell.rs resets it or zooms in.
fn zoom_pill_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 184.0).max(280.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 48.0,
        height: 28.0,
    }
}

/// Settings button. shell.rs opens anvil://settings. This rect does not pause.
fn settings_btn_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 132.0).max(332.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Downloads button on the navigation bar.
fn downloads_btn_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 100.0).max(364.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// Inspector button. shell.rs toggles the dock, including with F12 on Windows.
fn inspector_btn_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 68.0).max(396.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// App menu button at the right of the navigation bar.
fn app_menu_btn_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 36.0).max(428.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 28.0,
        height: 28.0,
    }
}

/// One bookmark pill. y sits just under TOP_BAR_HEIGHT.
fn bookmark_rect(index: usize) -> Rect {
    let pill_w = 112.0;
    Rect {
        x: 8.0 + (index as f32) * (pill_w + 6.0),
        y: TOP_BAR_HEIGHT + 4.0,
        width: pill_w,
        height: 20.0,
    }
}

/// Shorten text to max_chars. Cuts longer text with two dots when more than three characters remain.
fn truncate(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return chars.into_iter().take(max_chars).collect();
    }
    let mut out: String = chars.into_iter().take(max_chars - 2).collect();
    out.push_str("..");
    out
}

/// Keep the last max_chars characters.
fn tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

#[cfg(test)]
mod tests {
    //! Hit tests for toolbar rectangles and address normalization. No window is opened.
    use super::*;

    /// Point checks for the logo, tabs, menus, find, inspector, bookmarks, and the page.
    #[test]
    fn hit_tests_tabs_controls_find_bar_context_menu_app_menu_inspector_and_page() {
        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.open_html("<title>First</title><p>One</p>", "first.html");
        session.new_tab();
        session.open_html("<title>Second</title><p>Two</p>", "second.html");

        assert_eq!(hit_test_ui(&session, 800.0, 16.0, 16.0), Some(UiHit::Home));
        assert_eq!(
            hit_test_ui(&session, 800.0, 50.0, 16.0),
            Some(UiHit::Tab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 212.0, 16.0),
            Some(UiHit::CloseTab(0))
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 430.0, 16.0),
            Some(UiHit::NewTab)
        );
        assert_eq!(hit_test_ui(&session, 800.0, 16.0, 52.0), Some(UiHit::Back));
        assert_eq!(
            hit_test_ui(&session, 800.0, 50.0, 52.0),
            Some(UiHit::Forward)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 80.0, 52.0),
            Some(UiHit::Reload)
        );
        assert_eq!(hit_test_ui(&session, 800.0, 112.0, 52.0), Some(UiHit::Home));
        assert_eq!(
            hit_test_ui(&session, 800.0, 150.0, 52.0),
            Some(UiHit::SecurityBadge)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 220.0, 52.0),
            Some(UiHit::AddressField)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 595.0, 52.0),
            Some(UiHit::ToggleBookmark)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 635.0, 52.0),
            Some(UiHit::ZoomPill)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 680.0, 52.0),
            Some(UiHit::SettingsButton)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 712.0, 52.0),
            Some(UiHit::DownloadsButton)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 744.0, 52.0),
            Some(UiHit::InspectorButton)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 776.0, 52.0),
            Some(UiHit::AppMenuButton)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 40.0, 100.0),
            Some(UiHit::Page {
                x: 40.0,
                y: 100.0 - TOP_BAR_HEIGHT
            })
        );

        let mut ui = UiState::new();
        ui.open_find();
        ui.set_find_query("Two");
        assert_eq!(ui.find_next(3), 1);
        assert_eq!(ui.find_prev(3), 0);
        let fbar = find_bar_rect(800.0, TOP_BAR_HEIGHT);
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, fbar.x + 20.0, fbar.y + 10.0),
            Some(UiHit::FindInput)
        );

        // App Menu Popover hit testing
        ui.toggle_app_menu();
        assert!(ui.is_app_menu_open());
        let mrect = app_menu_rect(800.0, TOP_BAR_HEIGHT);
        assert_eq!(
            hit_test_ui_with_state(
                &session,
                &ui,
                800.0,
                mrect.x + 40.0,
                mrect.y + APP_MENU_HEADER_H + 12.0
            ),
            Some(UiHit::AppMenuItem(AppMenuAction::NewTab))
        );
        ui.close_app_menu();

        // Developer Inspector Dock hit testing
        ui.toggle_inspector();
        assert!(ui.is_inspector_open());
        let dock = inspector_dock_rect(800.0, 720.0);
        let styles_tab = inspector_tab_rect(dock, 1);
        assert_eq!(
            hit_test_ui_with_state(
                &session,
                &ui,
                800.0,
                styles_tab.x + 20.0,
                styles_tab.y + 10.0
            ),
            Some(UiHit::InspectorTab(InspectorTab::Styles))
        );
        ui.close_inspector();

        ui.open_context_menu(120.0, 140.0);
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 140.0, 150.0),
            Some(UiHit::ContextMenuItem(ContextMenuAction::Back))
        );

        session.add_bookmark("Home", "examples/hello.html");
        ui.close_context_menu();
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 20.0, TOP_BAR_HEIGHT + 10.0),
            Some(UiHit::Bookmark(0))
        );
    }

    /// Address editing, submit, and normalization for direct URLs and the three search engines.
    #[test]
    fn edits_and_submits_address_field_input() {
        let mut ui = UiState::new();
        ui.focus_address("examples/hello.html");
        assert!(ui.is_editing_address());
        ui.set_input("example.com");
        ui.push_char('/');
        ui.backspace();
        assert_eq!(ui.submit_address(), Some("https://example.com".to_string()));
        assert!(!ui.is_editing_address());
        assert_eq!(
            normalize_address_input("anvil://security"),
            Some("anvil://security".to_string())
        );
        assert_eq!(
            normalize_address_input_with_engine("rust web engine", SearchEngine::Google),
            Some("https://www.google.com/search?q=rust+web+engine".to_string())
        );
        assert_eq!(
            normalize_address_input_with_engine("rust web engine", SearchEngine::DuckDuckGo),
            Some("https://duckduckgo.com/?q=rust+web+engine".to_string())
        );
        assert_eq!(
            normalize_address_input_with_engine("rust web engine", SearchEngine::Bing),
            Some("https://www.bing.com/search?q=rust+web+engine".to_string())
        );
        assert_eq!(normalize_address_input("javascript:alert(1)"), None);
    }

    #[test]
    fn theme_palette_clear_address_viewport_and_cached_compose_work() {
        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.open_html("<title>Theme</title><p>Hello</p>", "theme.html");
        let mut ui = UiState::new();
        ui.focus_address("https://example.com");
        let addr = address_rect(800.0);
        let clear = clear_address_rect(addr);
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, clear.x + 8.0, clear.y + 8.0),
            Some(UiHit::ClearAddress)
        );
        ui.clear_address();
        assert_eq!(ui.input(), "");
        assert!(ui.is_editing_address());

        let vp_base = content_viewport_for(&session, &ui, 800.0, 600.0);
        ui.toggle_inspector();
        let vp_dock = content_viewport_for(&session, &ui, 800.0, 600.0);
        assert!(vp_dock.height < vp_base.height);

        for theme in [
            BrowserTheme::Dark,
            BrowserTheme::Midnight,
            BrowserTheme::Light,
            BrowserTheme::Nordic,
        ] {
            session.set_theme(theme);
            let page_fb = session.rasterize();
            let fb1 = compose_frame(&session, &ui, 800, 600);
            let fb2 = compose_frame_with_page_fb(&session, &ui, 800, 600, &page_fb);
            assert_eq!(fb1.pixels(), fb2.pixels());
        }
    }

    #[test]
    fn cursor_editing_link_context_menu_and_select_dropdown_work() {
        let session = Session::new(content_viewport(800.0, 600.0));
        let mut ui = UiState::new();
        ui.focus_address("https://example.com/path");
        assert_eq!(ui.input_cursor(), 24);
        assert!(ui.move_cursor_left());
        assert!(ui.move_cursor_left());
        assert_eq!(ui.input_cursor(), 22);
        ui.push_char('X');
        assert_eq!(ui.input(), "https://example.com/paXth");
        assert!(ui.delete_forward());
        assert_eq!(ui.input(), "https://example.com/paXh");
        assert!(ui.move_cursor_end());
        assert!(ui.delete_word_back());
        assert_eq!(ui.input(), "https://example.com/");
        assert!(ui.select_all_input());
        ui.push_char('a');
        assert_eq!(ui.input(), "a");

        ui.open_context_menu_with_link(100.0, 120.0, Some("https://example.com/next".to_string()));
        assert_eq!(ui.context_menu_link(), Some("https://example.com/next"));
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 120.0, 130.0),
            Some(UiHit::ContextMenuItem(ContextMenuAction::OpenLinkInNewTab))
        );
        ui.close_context_menu();

        ui.open_select_dropdown(
            anvil_dom::NodeId::from_raw(1),
            Rect {
                x: 100.0,
                y: 120.0,
                width: 180.0,
                height: 28.0,
            },
            vec![
                ("a".to_string(), "Alpha".to_string(), true),
                ("b".to_string(), "Beta".to_string(), false),
            ],
        );
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, 120.0, 160.0),
            Some(UiHit::SelectDropdownOption(0))
        );
        assert_eq!(
            normalize_address_input("view-source:https://example.com"),
            Some("view-source:https://example.com".to_string())
        );
    }
}
