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
/// Permanent bottom status bar height (0.0; replaced by the floating bottom-left link-hover/status pill).
pub const STATUS_BAR_HEIGHT: f32 = 0.0;
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
    TogglePinTab,
    CloseOtherTabs,
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
    pub const ALL: [Self; 17] = [
        Self::OpenLinkInNewTab,
        Self::CopyLinkAddress,
        Self::Back,
        Self::Forward,
        Self::Reload,
        Self::ReopenClosedTab,
        Self::DuplicateTab,
        Self::TogglePinTab,
        Self::CloseOtherTabs,
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
    pub const PAGE_ITEMS: [Self; 15] = [
        Self::Back,
        Self::Forward,
        Self::Reload,
        Self::ReopenClosedTab,
        Self::DuplicateTab,
        Self::TogglePinTab,
        Self::CloseOtherTabs,
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
            Self::TogglePinTab => "Pin / Unpin Tab",
            Self::CloseOtherTabs => "Close Other Tabs",
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
            Self::TogglePinTab => "Pin",
            Self::CloseOtherTabs => "Tabs",
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

/// Category badge for a floating Omnibox suggestion row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Builtin,
    Bookmark,
    History,
    Search,
}

impl SuggestionKind {
    /// Short badge label drawn on the left of the suggestion row.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Builtin => "SYS",
            Self::Bookmark => "STAR",
            Self::History => "HIST",
            Self::Search => "WEB",
        }
    }
}

/// One suggestion row displayed in the floating Omnibox autocomplete dropdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniboxSuggestion {
    pub kind: SuggestionKind,
    pub title: String,
    pub url: String,
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
    Storage,
}

impl InspectorTab {
    /// Order used both to draw the rows and to turn a click into an action.
    pub const ALL: [Self; 5] = [
        Self::Elements,
        Self::Styles,
        Self::Network,
        Self::Console,
        Self::Storage,
    ];

    /// Text drawn for this inspector tab.
    pub fn label(self) -> &'static str {
        match self {
            Self::Elements => "Elements",
            Self::Styles => "Styles",
            Self::Network => "Network",
            Self::Console => "Console",
            Self::Storage => "Storage",
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
    TitlebarDrag,
    WindowMinimize,
    WindowMaximize,
    WindowClose,
    Back,
    Forward,
    Reload,
    Home,
    SecurityBadge,
    AddressField,
    ClearAddress,
    ShieldsButton,
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
    OmniboxSuggestion(usize),
    InspectorConsoleInput,
    InspectorStyleInput,
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
    omnibox_selected: Option<usize>,
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
    drag_select_start: Option<(f32, f32)>,
    select_dropdown: Option<SelectDropdownOverlay>,
    editing_console: bool,
    console_input: String,
    console_cursor: usize,
    editing_style: bool,
    style_input: String,
    style_cursor: usize,
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
        self.editing_console = false;
        self.editing_style = false;
        self.omnibox_selected = None;
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
        self.omnibox_selected = None;
    }

    /// Clear the address bar input while keeping address editing active.
    pub fn clear_address(&mut self) {
        self.editing_address = true;
        self.input.clear();
        self.input_cursor = 0;
        self.input_select_all = false;
        self.omnibox_selected = None;
    }

    /// Focus the DevTools Console JS input bar.
    pub fn focus_console(&mut self) {
        self.editing_console = true;
        self.editing_style = false;
        self.editing_address = false;
        self.input_select_all = false;
        self.omnibox_selected = None;
        self.find_open = false;
        self.context_menu = None;
        self.context_menu_link = None;
        self.app_menu_open = false;
        self.select_dropdown = None;
        self.console_cursor = self.console_input.chars().count();
    }

    /// Blur the DevTools Console JS input bar.
    pub fn blur_console(&mut self) {
        self.editing_console = false;
    }

    /// True when the DevTools Console JS input bar has keyboard focus.
    pub fn is_editing_console(&self) -> bool {
        self.editing_console
    }

    /// Current text in the DevTools Console JS input bar.
    pub fn console_input(&self) -> &str {
        &self.console_input
    }

    /// Caret index inside the DevTools Console JS input bar.
    pub fn console_cursor(&self) -> usize {
        self.console_cursor.min(self.console_input.chars().count())
    }

    /// Take and clear the DevTools Console JS input when non-empty.
    pub fn take_console_input(&mut self) -> Option<String> {
        let trimmed = self.console_input.trim().to_string();
        self.console_input.clear();
        self.console_cursor = 0;
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Focus the DevTools Styles live CSS declaration override input bar.
    pub fn focus_style(&mut self) {
        self.editing_style = true;
        self.editing_console = false;
        self.editing_address = false;
        self.input_select_all = false;
        self.omnibox_selected = None;
        self.find_open = false;
        self.context_menu = None;
        self.context_menu_link = None;
        self.app_menu_open = false;
        self.select_dropdown = None;
        self.style_cursor = self.style_input.chars().count();
    }

    /// Blur the DevTools Styles live CSS declaration override input bar.
    pub fn blur_style(&mut self) {
        self.editing_style = false;
    }

    /// True when the DevTools Styles live CSS declaration override input bar has keyboard focus.
    pub fn is_editing_style(&self) -> bool {
        self.editing_style
    }

    /// Current text in the DevTools Styles live CSS declaration override input bar.
    pub fn style_input(&self) -> &str {
        &self.style_input
    }

    /// Caret index inside the DevTools Styles live CSS declaration override input bar.
    pub fn style_cursor(&self) -> usize {
        self.style_cursor.min(self.style_input.chars().count())
    }

    /// Take and clear the DevTools Styles live CSS declaration override input when non-empty.
    pub fn take_style_input(&mut self) -> Option<String> {
        let trimmed = self.style_input.trim().to_string();
        self.style_input.clear();
        self.style_cursor = 0;
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Record the page-space start coordinate `(x, y)` of a mouse-drag text selection.
    pub fn start_drag_select(&mut self, x: f32, y: f32) {
        self.drag_select_start = Some((x, y));
    }

    /// Return the page-space start coordinate `(x, y)` if a mouse-drag text selection is active.
    pub fn drag_select_start(&self) -> Option<(f32, f32)> {
        self.drag_select_start
    }

    /// End an active mouse-drag text selection.
    pub fn stop_drag_select(&mut self) {
        self.drag_select_start = None;
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
        self.omnibox_selected = None;
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
        self.omnibox_selected = None;
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
        self.omnibox_selected = None;
        Some(std::mem::take(&mut self.input))
    }

    /// Insert one printable ASCII character at the active caret.
    /// The address bar wins when it is focused. Otherwise the character goes to find or console.
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
            self.omnibox_selected = None;
        } else if self.find_open && (' '..='~').contains(&ch) {
            let mut chars: Vec<char> = self.find_query.chars().collect();
            let cur = self.find_cursor.min(chars.len());
            chars.insert(cur, ch);
            self.find_query = chars.into_iter().collect();
            self.find_cursor = cur + 1;
            self.find_active_match = 0;
        } else if self.editing_console && (' '..='~').contains(&ch) {
            let mut chars: Vec<char> = self.console_input.chars().collect();
            let cur = self.console_cursor.min(chars.len());
            chars.insert(cur, ch);
            self.console_input = chars.into_iter().collect();
            self.console_cursor = cur + 1;
        } else if self.editing_style && (' '..='~').contains(&ch) {
            let mut chars: Vec<char> = self.style_input.chars().collect();
            let cur = self.style_cursor.min(chars.len());
            chars.insert(cur, ch);
            self.style_input = chars.into_iter().collect();
            self.style_cursor = cur + 1;
        }
    }

    /// Delete one character before the caret from the address bar, find bar, console, or style input.
    pub fn backspace(&mut self) {
        if self.editing_address {
            self.omnibox_selected = None;
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
        } else if self.editing_console {
            let mut chars: Vec<char> = self.console_input.chars().collect();
            let cur = self.console_cursor.min(chars.len());
            if cur > 0 {
                chars.remove(cur - 1);
                self.console_input = chars.into_iter().collect();
                self.console_cursor = cur - 1;
            }
        } else if self.editing_style {
            let mut chars: Vec<char> = self.style_input.chars().collect();
            let cur = self.style_cursor.min(chars.len());
            if cur > 0 {
                chars.remove(cur - 1);
                self.style_input = chars.into_iter().collect();
                self.style_cursor = cur - 1;
            }
        }
    }

    /// Delete the character at the caret (`Delete` key) in the address bar, find bar, console, or style input.
    pub fn delete_forward(&mut self) -> bool {
        if self.editing_address {
            self.omnibox_selected = None;
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
        } else if self.editing_console {
            let mut chars: Vec<char> = self.console_input.chars().collect();
            let cur = self.console_cursor.min(chars.len());
            if cur < chars.len() {
                chars.remove(cur);
                self.console_input = chars.into_iter().collect();
                self.console_cursor = cur;
                return true;
            }
            false
        } else if self.editing_style {
            let mut chars: Vec<char> = self.style_input.chars().collect();
            let cur = self.style_cursor.min(chars.len());
            if cur < chars.len() {
                chars.remove(cur);
                self.style_input = chars.into_iter().collect();
                self.style_cursor = cur;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Delete the word before the caret (`Ctrl+Backspace`) in the address bar, find bar, or console.
    pub fn delete_word_back(&mut self) -> bool {
        if self.editing_address {
            self.omnibox_selected = None;
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
        } else if self.editing_console {
            let mut chars: Vec<char> = self.console_input.chars().collect();
            let cur = self.console_cursor.min(chars.len());
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
            self.console_input = chars.into_iter().collect();
            self.console_cursor = start;
            true
        } else if self.editing_style {
            let mut chars: Vec<char> = self.style_input.chars().collect();
            let cur = self.style_cursor.min(chars.len());
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
            self.style_input = chars.into_iter().collect();
            self.style_cursor = start;
            true
        } else {
            false
        }
    }

    /// Move the caret one character left (`Left` arrow) in the address bar, find bar, console, or style input.
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
        } else if self.editing_console {
            let cur = self.console_cursor();
            if cur > 0 {
                self.console_cursor = cur - 1;
                return true;
            }
            false
        } else if self.editing_style {
            let cur = self.style_cursor();
            if cur > 0 {
                self.style_cursor = cur - 1;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Move the caret one character right (`Right` arrow) in the address bar, find bar, console, or style input.
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
        } else if self.editing_console {
            let len = self.console_input.chars().count();
            let cur = self.console_cursor();
            if cur < len {
                self.console_cursor = cur + 1;
                return true;
            }
            false
        } else if self.editing_style {
            let len = self.style_input.chars().count();
            let cur = self.style_cursor();
            if cur < len {
                self.style_cursor = cur + 1;
                return true;
            }
            false
        } else {
            false
        }
    }

    /// Move the caret to the start of the address bar, find bar, console, or style input (`Home`).
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
        } else if self.editing_console {
            let changed = self.console_cursor() > 0;
            self.console_cursor = 0;
            changed
        } else if self.editing_style {
            let changed = self.style_cursor() > 0;
            self.style_cursor = 0;
            changed
        } else {
            false
        }
    }

    /// Move the caret to the end of the address bar, find bar, console, or style input (`End`).
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
        } else if self.editing_console {
            let len = self.console_input.chars().count();
            let changed = self.console_cursor() < len;
            self.console_cursor = len;
            changed
        } else if self.editing_style {
            let len = self.style_input.chars().count();
            let changed = self.style_cursor() < len;
            self.style_cursor = len;
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
        self.omnibox_selected = None;
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
        self.editing_console = false;
        self.input_select_all = false;
        self.omnibox_selected = None;
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
            self.editing_console = false;
            self.input_select_all = false;
            self.omnibox_selected = None;
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
        if !self.inspector_open {
            self.editing_console = false;
            self.editing_style = false;
        }
        self.inspector_open
    }

    /// Open the inspector dock.
    pub fn open_inspector(&mut self) {
        self.inspector_open = true;
    }

    /// Close the inspector dock.
    pub fn close_inspector(&mut self) {
        self.inspector_open = false;
        self.editing_console = false;
        self.editing_style = false;
    }

    /// Which inspector tab is selected.
    pub fn inspector_tab(&self) -> InspectorTab {
        self.inspector_tab
    }

    /// Select an inspector tab. The dock stays as it was.
    pub fn set_inspector_tab(&mut self, tab: InspectorTab) {
        self.inspector_tab = tab;
        if tab != InspectorTab::Console {
            self.editing_console = false;
        }
        if tab != InspectorTab::Styles {
            self.editing_style = false;
        }
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

    /// Up to 6 smart suggestions matching the current address bar input across built-in routes,
    /// bookmarks, history, and a live search fallback.
    pub fn omnibox_suggestions(&self, session: &Session) -> Vec<OmniboxSuggestion> {
        if !self.editing_address {
            return Vec::new();
        }
        let needle = self.input.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<OmniboxSuggestion> = Vec::with_capacity(6);
        let builtins = [
            ("Anvil Home Dashboard", "anvil://home"),
            ("Developer Inspector", "anvil://inspect"),
            ("Browser Settings", "anvil://settings"),
            ("Downloads & SHA-256 Vault", "anvil://downloads"),
            ("Security & Shield Center", "anvil://security"),
            ("Browsing History", "anvil://history"),
        ];
        for (title, route) in builtins {
            if route.contains(&needle) || title.to_ascii_lowercase().contains(&needle) {
                out.push(OmniboxSuggestion {
                    kind: SuggestionKind::Builtin,
                    title: title.to_string(),
                    url: route.to_string(),
                });
                if out.len() >= 5 {
                    break;
                }
            }
        }
        if out.len() < 5 {
            for bm in session.bookmarks() {
                if bm.url.to_ascii_lowercase().contains(&needle)
                    || bm.title.to_ascii_lowercase().contains(&needle)
                {
                    if !out.iter().any(|s| s.url == bm.url) {
                        out.push(OmniboxSuggestion {
                            kind: SuggestionKind::Bookmark,
                            title: if bm.title.is_empty() {
                                bm.url.clone()
                            } else {
                                bm.title.clone()
                            },
                            url: bm.url.clone(),
                        });
                        if out.len() >= 5 {
                            break;
                        }
                    }
                }
            }
        }
        if out.len() < 5 {
            for entry in session.history_log().iter().rev() {
                if entry.url.to_ascii_lowercase().contains(&needle)
                    || entry.title.to_ascii_lowercase().contains(&needle)
                {
                    if !out.iter().any(|s| s.url == entry.url) {
                        out.push(OmniboxSuggestion {
                            kind: SuggestionKind::History,
                            title: if entry.title.is_empty() {
                                entry.url.clone()
                            } else {
                                entry.title.clone()
                            },
                            url: entry.url.clone(),
                        });
                        if out.len() >= 5 {
                            break;
                        }
                    }
                }
            }
        }
        let raw_trimmed = self.input.trim();
        let search_url = session.search_engine().search_url(raw_trimmed);
        if !out.iter().any(|s| s.url == search_url) && out.len() < 6 {
            out.push(OmniboxSuggestion {
                kind: SuggestionKind::Search,
                title: format!(
                    "Search {} for \"{}\"",
                    session.search_engine().label(),
                    raw_trimmed
                ),
                url: search_url,
            });
        }
        out
    }

    /// Currently highlighted row index inside the floating Omnibox suggestion popover.
    pub fn omnibox_selected(&self) -> Option<usize> {
        self.omnibox_selected
    }

    /// URL of the currently highlighted Omnibox suggestion row, if any.
    pub fn selected_omnibox_url(&self, session: &Session) -> Option<String> {
        let idx = self.omnibox_selected?;
        self.omnibox_suggestions(session)
            .get(idx)
            .map(|s| s.url.clone())
    }

    /// Move the Omnibox suggestion highlight down (`Down` arrow).
    pub fn move_omnibox_selection_down(&mut self, session: &Session) -> bool {
        let len = self.omnibox_suggestions(session).len();
        if len == 0 {
            return false;
        }
        let next = match self.omnibox_selected {
            Some(idx) => (idx + 1) % len,
            None => 0,
        };
        self.omnibox_selected = Some(next);
        true
    }

    /// Move the Omnibox suggestion highlight up (`Up` arrow).
    pub fn move_omnibox_selection_up(&mut self, session: &Session) -> bool {
        let len = self.omnibox_suggestions(session).len();
        if len == 0 {
            return false;
        }
        let prev = match self.omnibox_selected {
            Some(0) | None => len - 1,
            Some(idx) => idx - 1,
        };
        self.omnibox_selected = Some(prev);
        true
    }

    /// Return the best matching URL suggestion from built-in routes, bookmarks, or history
    /// while the user is typing in the address bar.
    pub fn autocomplete_suggestion(&self, session: &Session) -> Option<String> {
        if !self.editing_address {
            return None;
        }
        if let Some(url) = self.selected_omnibox_url(session) {
            return Some(url);
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
            self.omnibox_selected = None;
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

    // 1c. Floating Omnibox Autocomplete Suggestion Popover
    if ui.is_editing_address() {
        let suggestions = ui.omnibox_suggestions(session);
        if !suggestions.is_empty() {
            let srect = omnibox_suggestions_rect(window_width, suggestions.len());
            if srect.contains(x, y) {
                let rel_y = (y - (srect.y + 4.0)).max(0.0);
                let idx = (rel_y / 26.0).floor() as usize;
                if idx < suggestions.len() {
                    return Some(UiHit::OmniboxSuggestion(idx));
                }
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

    // 4. Tab Strip (with integrated Windows 11 Caption Controls & Titlebar Drag)
    if y < TAB_STRIP_HEIGHT {
        if window_close_rect(window_width).contains(x, y) {
            return Some(UiHit::WindowClose);
        }
        if window_maximize_rect(window_width).contains(x, y) {
            return Some(UiHit::WindowMaximize);
        }
        if window_minimize_rect(window_width).contains(x, y) {
            return Some(UiHit::WindowMinimize);
        }
        if logo_rect().contains(x, y) {
            return Some(UiHit::Home);
        }
        let count = session.len().max(1);
        let tab_w = tab_width(window_width, count);
        let can_close = count > 1;
        for index in 0..count {
            let rect = tab_rect(index, tab_w);
            if rect.contains(x, y) {
                let is_pinned = session.tabs().get(index).is_some_and(|t| t.is_pinned());
                if can_close && !is_pinned && close_rect(rect).contains(x, y) {
                    return Some(UiHit::CloseTab(index));
                }
                return Some(UiHit::Tab(index));
            }
        }
        if new_tab_rect(count, tab_w).contains(x, y) {
            return Some(UiHit::NewTab);
        }
        return Some(UiHit::TitlebarDrag);
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
        if shields_pill_rect(window_width).contains(x, y) {
            return Some(UiHit::ShieldsButton);
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
            if ui.inspector_tab() == InspectorTab::Console
                && inspector_console_input_rect(dock).contains(x, y)
            {
                return Some(UiHit::InspectorConsoleInput);
            }
            if ui.inspector_tab() == InspectorTab::Styles
                && inspector_style_input_rect(dock).contains(x, y)
            {
                return Some(UiHit::InspectorStyleInput);
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

/// Split a URL into `(scheme, domain_or_host, path_and_query)` for proportional domain highlighting.
/// For example, `"https://docs.rust-lang.org/std/vec"` splits into
/// `("https://", "docs.rust-lang.org", "/std/vec")`.
pub fn split_url_for_domain_highlight(url: &str) -> (&str, &str, &str) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return ("", "", "");
    }
    for scheme in ["https://", "http://", "anvil://", "view-source:"] {
        if let Some(rest) = trimmed.strip_prefix(scheme) {
            let split_idx = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            return (scheme, &rest[..split_idx], &rest[split_idx..]);
        }
    }
    let split_idx = trimmed.find(['/', '?', '#']).unwrap_or(trimmed.len());
    ("", &trimmed[..split_idx], &trimmed[split_idx..])
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
    fb.draw_image(logo.x, logo.y, logo.width, logo.height, 0.0, top_bar_logo());

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

        let tab_can_close = can_close && !tab.is_pinned();
        let text_space = if tab_can_close {
            (rect.width - 38.0).max(16.0)
        } else {
            (rect.width - 22.0).max(16.0)
        };
        let max_chars = ((text_space / 7.0).floor() as usize).max(1);
        let raw_title = if tab.is_pinned() {
            format!("[P] {}", tab.title())
        } else {
            tab.title().to_string()
        };
        let label = truncate(&raw_title, max_chars);
        fb.draw_text(rect.x + 18.0, rect.y + 7.0, &label, 13.0, fg);

        if tab_can_close {
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
        14.0,
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

    // 1b. Windows 11 Top-Right Caption Controls (Minimize, Maximize, Close)
    let win_min = window_minimize_rect(width_f);
    let win_max = window_maximize_rect(width_f);
    let win_close = window_close_rect(width_f);
    if ui.is_hovering(win_min) {
        fb.fill_rect(
            win_min.x,
            win_min.y,
            win_min.width,
            win_min.height,
            pal.control_hover_bg,
        );
    }
    fb.fill_rect(
        win_min.x + 18.0,
        win_min.y + 16.0,
        10.0,
        1.5,
        pal.ink_primary,
    );
    if ui.is_hovering(win_max) {
        fb.fill_rect(
            win_max.x,
            win_max.y,
            win_max.width,
            win_max.height,
            pal.control_hover_bg,
        );
    }
    fb.fill_rounded_rect(
        win_max.x + 18.0,
        win_max.y + 11.0,
        10.0,
        10.0,
        1.0,
        1.2,
        if ui.is_hovering(win_max) {
            pal.control_hover_bg
        } else {
            pal.strip_bg
        },
        pal.ink_primary,
    );
    let close_win_hov = ui.is_hovering(win_close);
    if close_win_hov {
        fb.fill_rect(
            win_close.x,
            win_close.y,
            win_close.width,
            win_close.height,
            Color {
                r: 232,
                g: 17,
                b: 35,
                a: 255,
            },
        );
    }
    draw_close_x_icon(
        &mut fb,
        win_close,
        if close_win_hov {
            Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            }
        } else {
            pal.ink_primary
        },
    );

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

    // 3. Unified Pill Omnibox Capsule (Security Pill + Domain-Highlighted URL + Clear + Shields Pill + Bookmark Star)
    let current_page_url = active_tab
        .current()
        .map(|page| page.url.as_str())
        .unwrap_or("");
    let (sec_label, sec_bg, sec_fg, _sec_summary) = classify_url_security(current_page_url);
    let badge = security_badge_rect();
    let addr = address_rect(width_f);
    let shields_pill = shields_pill_rect(width_f);
    let star = star_rect(width_f);

    let omnibox_x = badge.x;
    let omnibox_w = ((star.x + star.width) - badge.x).max(120.0);
    let omnibox_hovered = ui.is_hovering(addr)
        || ui.is_hovering(badge)
        || ui.is_hovering(shields_pill)
        || ui.is_hovering(star);
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
        14.0,
        1.2,
        pal.field_bg,
        omnibox_border,
    );

    // Security Badge inside left end of Pill Omnibox
    let badge_hovered = ui.is_hovering(badge);
    fb.fill_rounded_rect(
        badge.x + 2.0,
        badge.y + 2.0,
        badge.width - 4.0,
        badge.height - 4.0,
        11.0,
        1.0,
        sec_bg,
        if badge_hovered { pal.accent } else { sec_fg },
    );
    fb.draw_text(badge.x + 9.0, badge.y + 6.0, sec_label, 12.0, sec_fg);

    // URL / Search text inside Omnibox with Domain Highlighting
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
    let text_right_pad = if show_clear_btn { 36.0 } else { 12.0 };
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
    let placeholder_label = format!(
        "Search with {} or enter web address",
        session.search_engine().label()
    );
    if !ui.is_editing_address() && !shown_url.is_empty() {
        let (scheme, domain, path) = split_url_for_domain_highlight(&shown_url);
        if !domain.is_empty() {
            let mut cursor_x = addr.x + 8.0;
            if !scheme.is_empty() {
                fb.draw_text(cursor_x, addr.y + 6.0, scheme, 13.0, pal.ink_muted);
                cursor_x += (scheme.chars().count() as f32) * 7.0;
            }
            fb.draw_text(cursor_x, addr.y + 6.0, domain, 13.0, pal.ink_primary);
            cursor_x += (domain.chars().count() as f32) * 7.2;
            if !path.is_empty() {
                fb.draw_text(cursor_x, addr.y + 6.0, path, 13.0, pal.ink_muted);
            }
        } else {
            fb.draw_text(
                addr.x + 8.0,
                addr.y + 6.0,
                &shown_url,
                13.0,
                pal.ink_primary,
            );
        }
    } else {
        let addr_color = if raw_url.is_empty() {
            pal.ink_muted
        } else {
            pal.ink_primary
        };
        let display_text = if shown_url.is_empty() {
            placeholder_label.as_str()
        } else {
            shown_url.as_str()
        };
        fb.draw_text(addr.x + 8.0, addr.y + 6.0, display_text, 13.0, addr_color);
    }

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

    // Inline Shields Pill inside Omnibox
    let page_blocked = active_tab
        .current()
        .map(|p| p.blocked_csp_count + p.blocked_tracker_count)
        .unwrap_or(0);
    let total_blocked = session.network_shield().blocked_total() as usize + page_blocked;
    let shields_hovered = ui.is_hovering(shields_pill);
    let shields_active = current_page_url.starts_with("anvil://security");
    fb.fill_rounded_rect(
        shields_pill.x + 1.0,
        shields_pill.y + 3.0,
        shields_pill.width - 2.0,
        shields_pill.height - 6.0,
        10.0,
        1.0,
        if shields_hovered {
            pal.control_hover_bg
        } else {
            pal.control_bg
        },
        if shields_active || shields_hovered {
            pal.accent
        } else {
            pal.divider
        },
    );
    let shields_label = truncate(&format!("S:{total_blocked}"), 6);
    fb.draw_text(
        shields_pill.x + 8.0,
        shields_pill.y + 6.0,
        &shields_label,
        11.0,
        if total_blocked > 0 || shields_hovered || shields_active {
            pal.accent
        } else {
            pal.ink_muted
        },
    );

    // Bookmark Star inside right end of Omnibox
    let star_hovered = ui.is_hovering(star);
    let is_bookmarked = session.is_active_bookmarked();
    if star_hovered || is_bookmarked {
        fb.fill_rounded_rect(
            star.x + 2.0,
            star.y + 2.0,
            star.width - 4.0,
            star.height - 4.0,
            11.0,
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

    // 5b. Native C++20 Direct2D + DirectWrite Top Chrome Pass (Proportional ClearType UI)
    #[cfg(anvil_cpp_ui)]
    {
        render_cpp_d2d_top_chrome(&mut fb, session, ui, &pal, width, height);
        fb.draw_image(logo.x, logo.y, logo.width, logo.height, 0.0, top_bar_logo());
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

    // 8b. Mouse-Drag Page Text Selection Highlights
    if !session.selection_rects().is_empty() {
        let z = session.zoom_factor();
        let viewport_bottom = header_h + avail_h as f32;
        let sel_fill = Color {
            r: 56,
            g: 189,
            b: 248,
            a: 72,
        };
        let sel_border = Color {
            r: 56,
            g: 189,
            b: 248,
            a: 160,
        };
        for r in session.selection_rects() {
            let sx = r.x * z;
            let sy = header_h + (r.y * z) - session.scroll();
            let sw = r.width * z;
            let sh = r.height * z;
            if sy + sh > header_h && sy < viewport_bottom {
                fb.fill_rounded_rect(sx, sy, sw, sh, 2.0, 1.0, sel_fill, sel_border);
            }
        }
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

    // 10c. Floating Omnibox Autocomplete Suggestion Popover
    if ui.is_editing_address() {
        let suggestions = ui.omnibox_suggestions(session);
        if !suggestions.is_empty() {
            let srect = omnibox_suggestions_rect(width_f, suggestions.len());
            fb.fill_box_shadow(
                srect.x,
                srect.y,
                srect.width,
                srect.height,
                0.0,
                8.0,
                18.0,
                0.0,
                8.0,
                OVERLAY_SHADOW,
            );
            fb.fill_rounded_rect(
                srect.x,
                srect.y,
                srect.width,
                srect.height,
                8.0,
                1.0,
                pal.overlay_bg,
                pal.accent,
            );
            let max_sug_chars = (((srect.width - 74.0).max(20.0) / 6.8).floor() as usize).max(8);
            for (idx, sug) in suggestions.iter().enumerate() {
                let ry = srect.y + 4.0 + (idx as f32) * 26.0;
                let row_rect = Rect {
                    x: srect.x + 4.0,
                    y: ry,
                    width: srect.width - 8.0,
                    height: 24.0,
                };
                let is_sel = ui.omnibox_selected() == Some(idx);
                let hovered = ui.is_hovering(row_rect);
                if is_sel || hovered {
                    fb.fill_rounded_rect(
                        row_rect.x,
                        row_rect.y,
                        row_rect.width,
                        row_rect.height,
                        5.0,
                        0.0,
                        if is_sel {
                            pal.control_active_bg
                        } else {
                            pal.control_hover_bg
                        },
                        pal.accent,
                    );
                }
                fb.fill_rounded_rect(
                    row_rect.x + 6.0,
                    row_rect.y + 4.0,
                    38.0,
                    16.0,
                    4.0,
                    1.0,
                    pal.control_bg,
                    pal.divider,
                );
                fb.draw_text(
                    row_rect.x + 10.0,
                    row_rect.y + 6.0,
                    sug.kind.badge(),
                    10.0,
                    pal.accent_cyan,
                );
                let line = if sug.kind == SuggestionKind::Search {
                    truncate(&sug.title, max_sug_chars)
                } else {
                    truncate(&format!("{} -> {}", sug.title, sug.url), max_sug_chars)
                };
                fb.draw_text(
                    row_rect.x + 52.0,
                    row_rect.y + 5.0,
                    &line,
                    12.0,
                    if is_sel || hovered {
                        pal.accent
                    } else {
                        pal.ink_primary
                    },
                );
            }
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

    // 12. Floating Bottom-Left Link-Hover / Loading Status Pill (replaces permanent bottom status bar)
    let floating_status_text = if ui.is_loading() {
        Some("Loading resource over sandboxed transport...".to_string())
    } else {
        ui.hovered_link().map(|link| format!("Link -> {link}"))
    };
    if let Some(status_text) = &floating_status_text {
        let max_status_chars = (((width_f - 64.0).max(24.0) / 6.8).floor() as usize).max(1);
        let trimmed = truncate(status_text, max_status_chars);
        let pill_w = ((trimmed.chars().count() as f32) * 6.8 + 24.0)
            .clamp(110.0, (width_f - 24.0).max(110.0));
        let pill_h = 24.0;
        let pill_x = 10.0;
        let pill_y = (height_f - (dock_h_u32 as f32) - pill_h - 10.0).max(header_h + 4.0);
        fb.fill_rounded_rect(
            pill_x,
            pill_y,
            pill_w,
            pill_h,
            6.0,
            1.0,
            pal.overlay_bg,
            pal.divider,
        );
        fb.draw_text(pill_x + 10.0, pill_y + 5.0, &trimmed, 12.0, pal.ink_muted);
    }

    // 12b. Native C++20 Direct2D + DirectWrite Floating Overlays Pass
    #[cfg(anvil_cpp_ui)]
    {
        render_cpp_d2d_overlays(&mut fb, session, ui, &pal, width, height);
    }

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

            if session.inspected_node().is_some() {
                let (tag_desc, style_desc, box_desc, text_desc) =
                    session.inspected_element_summary();
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 8.0,
                    "Live Inspected DOM Node (Click / Right-Click Inspect)",
                    13.0,
                    pal.accent,
                );
                let r1 = truncate(&tag_desc, max_chars);
                let r2 = truncate(&box_desc, max_chars);
                let r3 = truncate(&style_desc, max_chars);
                let r4 = truncate(&text_desc, max_chars);
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
            } else {
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
        }
        InspectorTab::Styles => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "Prism CSS & Forge Layout (anvil-style / anvil-layout)",
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
                "Layout Modes: Block, Inline, Flexbox, Table, Aspect-Ratio & OOF",
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

            if session.inspected_node().is_some() {
                let (tag_desc, style_desc, box_desc, _) = session.inspected_element_summary();
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 8.0,
                    "Inspected Computed Style & Live CSS Override",
                    13.0,
                    pal.accent,
                );
                let r1 = truncate(&format!("Target: {tag_desc}"), max_chars);
                let r2 = truncate(&style_desc, max_chars);
                let r3 = truncate(&box_desc, max_chars);
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 28.0,
                    &r1,
                    12.0,
                    pal.ink_primary,
                );
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 46.0,
                    &r2,
                    12.0,
                    pal.ink_muted,
                );
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 64.0,
                    &r3,
                    12.0,
                    pal.ink_muted,
                );
            } else {
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 8.0,
                    "DisplayList & Live CSS Override (anvil-paint / anvil-style)",
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
                    "Typography: Multi-Font Sans/Serif/Mono + @font-face & Text-Transform",
                    max_chars,
                );
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 28.0,
                    &r1,
                    12.0,
                    pal.ink_primary,
                );
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 46.0,
                    &r2,
                    12.0,
                    pal.ink_muted,
                );
                fb.draw_text(
                    right_card.x + 10.0,
                    right_card.y + 64.0,
                    &r3,
                    12.0,
                    pal.ink_muted,
                );
            }
            let sinput_rect = inspector_style_input_rect(dock);
            let sinput_hovered = ui.is_hovering(sinput_rect);
            fb.fill_rounded_rect(
                sinput_rect.x,
                sinput_rect.y,
                sinput_rect.width,
                sinput_rect.height,
                5.0,
                1.0,
                pal.overlay_bg,
                if ui.is_editing_style() {
                    pal.accent
                } else if sinput_hovered {
                    pal.accent_cyan
                } else {
                    pal.divider
                },
            );
            let sprompt_str = if ui.is_editing_style() {
                let mut schars: Vec<char> = ui.style_input().chars().collect();
                let scur = ui.style_cursor().min(schars.len());
                schars.insert(scur, '|');
                let with_caret: String = schars.into_iter().collect();
                format!("style: {with_caret}")
            } else if ui.style_input().is_empty() {
                "style: e.g. color: #ff6b24; border: 2px solid #38bdf8 [Enter]".to_string()
            } else {
                format!("style: {}", ui.style_input())
            };
            let sprompt_shown = truncate(&sprompt_str, max_chars);
            fb.draw_text(
                sinput_rect.x + 8.0,
                sinput_rect.y + 4.0,
                &sprompt_shown,
                12.0,
                if ui.is_editing_style() || !ui.style_input().is_empty() {
                    pal.ink_primary
                } else {
                    pal.ink_muted
                },
            );
        }
        InspectorTab::Network => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "Network Waterfall Log (Document & Subresources)",
                13.0,
                pal.accent,
            );
            let net_log = session.active_network_log();
            for row_idx in 0..4 {
                let text = if let Some(rec) = net_log.get(row_idx) {
                    truncate(
                        &format!(
                            "[{}] {} {} ({}, {} B)",
                            rec.status, rec.method, rec.url, rec.kind, rec.bytes
                        ),
                        max_chars,
                    )
                } else if row_idx == 0 {
                    truncate("No network requests recorded on active tab.", max_chars)
                } else {
                    truncate(
                        &format!(
                            "Total Requests: {} | Blocked CSP: {} | Blocked Trackers: {}",
                            net_log.len(),
                            page.map(|p| p.blocked_csp_count).unwrap_or(0),
                            page.map(|p| p.blocked_tracker_count).unwrap_or(0)
                        ),
                        max_chars,
                    )
                };
                fb.draw_text(
                    left_card.x + 10.0,
                    left_card.y + 30.0 + (row_idx as f32) * 20.0,
                    &text,
                    12.0,
                    if row_idx == 0 {
                        pal.ink_primary
                    } else if row_idx == 3 {
                        pal.accent_cyan
                    } else {
                        pal.ink_muted
                    },
                );
            }

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
                "Cinder JS Console (anvil-script / CinderVM)",
                13.0,
                pal.accent,
            );
            let hist = session.console_history();
            let (l1, l2, l3) = if !hist.is_empty() {
                let last = &hist[hist.len() - 1];
                let prev = if hist.len() >= 2 {
                    let p = &hist[hist.len() - 2];
                    format!("> {}  =>  {}", p.0, p.1)
                } else {
                    format!(
                        "Script Execution: {} | Step Budget: 200,000",
                        if session.settings().scripts_enabled {
                            "ENABLED"
                        } else {
                            "DISABLED"
                        }
                    )
                };
                (
                    truncate(&prev, max_chars),
                    truncate(&format!("> {}", last.0), max_chars),
                    truncate(&format!("=> {}", last.1), max_chars),
                )
            } else {
                let page_logs = page.map(|p| p.runtime.console_logs()).unwrap_or(&[]);
                let last_log = page_logs
                    .last()
                    .map(|s| format!("console.log: {s}"))
                    .unwrap_or_else(|| {
                        "Console Status: [OK] Click input box below to evaluate JS".to_string()
                    });
                (
                    truncate(
                        &format!(
                            "Script Execution: {} | Step Budget: 200,000 | Call Depth: 64",
                            if session.settings().scripts_enabled {
                                "ENABLED"
                            } else {
                                "DISABLED"
                            }
                        ),
                        max_chars,
                    ),
                    truncate(
                        "Same-Origin Policy (SOP): Enforced on fetch() & DOM mutations",
                        max_chars,
                    ),
                    truncate(&last_log, max_chars),
                )
            };
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 28.0,
                &l1,
                12.0,
                pal.ink_muted,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 46.0,
                &l2,
                12.0,
                pal.ink_primary,
            );
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 64.0,
                &l3,
                12.0,
                BADGE_TLS_FG,
            );

            let cinput_rect = inspector_console_input_rect(dock);
            let cinput_hovered = ui.is_hovering(cinput_rect);
            fb.fill_rounded_rect(
                cinput_rect.x,
                cinput_rect.y,
                cinput_rect.width,
                cinput_rect.height,
                5.0,
                1.0,
                pal.overlay_bg,
                if ui.is_editing_console() {
                    pal.accent
                } else if cinput_hovered {
                    pal.accent_cyan
                } else {
                    pal.divider
                },
            );
            let prompt_str = if ui.is_editing_console() {
                let mut cchars: Vec<char> = ui.console_input().chars().collect();
                let ccur = ui.console_cursor().min(cchars.len());
                cchars.insert(ccur, '|');
                let with_caret: String = cchars.into_iter().collect();
                format!("> {with_caret}")
            } else if ui.console_input().is_empty() {
                "> Evaluate JS expression (press Enter to run)...".to_string()
            } else {
                format!("> {}", ui.console_input())
            };
            let prompt_shown = truncate(&prompt_str, max_chars);
            fb.draw_text(
                cinput_rect.x + 8.0,
                cinput_rect.y + 4.0,
                &prompt_shown,
                12.0,
                if ui.is_editing_console() || !ui.console_input().is_empty() {
                    pal.ink_primary
                } else {
                    pal.ink_muted
                },
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
        InspectorTab::Storage => {
            fb.draw_text(
                left_card.x + 10.0,
                left_card.y + 8.0,
                "Origin-Partitioned Web Storage (localStorage / sessionStorage)",
                13.0,
                pal.accent,
            );
            let entries = session.local_storage_entries();
            let l1 = truncate(
                &format!(
                    "Total localStorage Keys Across Origins: {} (Cap: 64 / origin)",
                    entries.len()
                ),
                max_chars,
            );
            let l2 = entries
                .first()
                .map(|(orig, k, v)| truncate(&format!("[{orig}] {k} = \"{v}\""), max_chars))
                .unwrap_or_else(|| {
                    truncate("No localStorage keys set yet on active session.", max_chars)
                });
            let l3 = entries
                .get(1)
                .map(|(orig, k, v)| truncate(&format!("[{orig}] {k} = \"{v}\""), max_chars))
                .unwrap_or_else(|| {
                    truncate(
                        "Origin Isolation: Strict scheme+host+port partitioning",
                        max_chars,
                    )
                });
            let l4 = truncate(
                "Persistence: Encrypted via Windows DPAPI (crypt32.dll) in profile.anvil",
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
                "RFC 6265 CookieJar, HSTS & Site Permissions",
                13.0,
                pal.accent,
            );
            let r1 = truncate(
                &format!(
                    "Cookies in Jar: {} (HttpOnly, Secure, SameSite, __Host-/__Secure-)",
                    session.cookie_jar().len()
                ),
                max_chars,
            );
            let r2 = truncate(
                &format!(
                    "HSTS Strict-Transport Hosts: {} | HTTPS-Only Mode: {}",
                    session.hsts_store().len(),
                    if session.settings().https_only_mode {
                        "ON"
                    } else {
                        "STANDARD"
                    }
                ),
                max_chars,
            );
            let r3 = truncate(
                "Third-Party Cookie Partitioning: Blocked across cross-site subresources",
                max_chars,
            );
            let r4 = truncate(
                "Clipboard & FileReader APIs: User-gesture & same-origin gated",
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

/// Floating Omnibox autocomplete suggestion popover bounds anchored under `address_rect`.
fn omnibox_suggestions_rect(window_width: f32, count: usize) -> Rect {
    let addr = address_rect(window_width);
    Rect {
        x: addr.x,
        y: addr.y + addr.height + 4.0,
        width: addr.width.max(220.0),
        height: 8.0 + (count as f32) * 26.0,
    }
}

/// Interactive Console JS input box inside the left card of `InspectorTab::Console`.
fn inspector_console_input_rect(dock: Rect) -> Rect {
    let body_y = dock.y + INSPECTOR_HEADER_HEIGHT + 8.0;
    let body_h = (dock.height - INSPECTOR_HEADER_HEIGHT - 16.0).max(40.0);
    let half_w = ((dock.width - 28.0) * 0.5).max(120.0);
    Rect {
        x: dock.x + 18.0,
        y: body_y + body_h - 30.0,
        width: (half_w - 16.0).max(80.0),
        height: 24.0,
    }
}

/// Interactive live CSS override input box inside the right card of `InspectorTab::Styles`.
fn inspector_style_input_rect(dock: Rect) -> Rect {
    let body_y = dock.y + INSPECTOR_HEADER_HEIGHT + 8.0;
    let body_h = (dock.height - INSPECTOR_HEADER_HEIGHT - 16.0).max(40.0);
    let half_w = ((dock.width - 28.0) * 0.5).max(120.0);
    Rect {
        x: dock.x + 26.0 + half_w,
        y: body_y + body_h - 30.0,
        width: (half_w - 16.0).max(80.0),
        height: 24.0,
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

/// One inspector tab. index is zero through four.
fn inspector_tab_rect(dock: Rect, index: usize) -> Rect {
    let tab_w = 96.0;
    Rect {
        x: dock.x + 86.0 + (index as f32) * (tab_w + 5.0),
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

/// Tab width from the leftover strip (reserving top-right space for caption buttons), scaling on 1080p/1440p/4K monitors.
fn tab_width(window_width: f32, count: usize) -> f32 {
    let available = (window_width - 220.0).max(72.0);
    let max_w = if window_width >= 3000.0 {
        300.0
    } else if window_width >= 2000.0 {
        264.0
    } else if window_width >= 1400.0 {
        236.0
    } else {
        188.0
    };
    (available / (count as f32)).clamp(72.0, max_w)
}

/// Top-right Windows 11 Minimize button (`_`).
fn window_minimize_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 138.0).max(200.0),
        y: 0.0,
        width: 46.0,
        height: 32.0,
    }
}

/// Top-right Windows 11 Maximize / Restore button (`[]`).
fn window_maximize_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 92.0).max(246.0),
        y: 0.0,
        width: 46.0,
        height: 32.0,
    }
}

/// Top-right Windows 11 Close button (`X`).
fn window_close_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 46.0).max(292.0),
        y: 0.0,
        width: 46.0,
        height: 32.0,
    }
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
        width: (window_width - 458.0).max(60.0),
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

/// Inline Shields telemetry pill inside the right side of the Pill Omnibox.
fn shields_pill_rect(window_width: f32) -> Rect {
    Rect {
        x: (window_width - 270.0).max(248.0),
        y: TAB_STRIP_HEIGHT + 7.0,
        width: 50.0,
        height: 28.0,
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

// ============================================================================
// C++20 Direct2D / DirectWrite / Win32 DWM UI Shell FFI Bridge (`ui/`)
// ============================================================================

#[cfg(anvil_cpp_ui)]
mod cpp_ui_ffi {
    use super::*;

    pub const ANVIL_UI_MAX_TABS: usize = 32;
    pub const ANVIL_UI_MAX_BOOKMARKS: usize = 24;
    pub const ANVIL_UI_MAX_SUGGESTIONS: usize = 12;
    pub const ANVIL_UI_MAX_MENU_ROWS: usize = 24;
    pub const ANVIL_UI_MAX_SELECT_ROWS: usize = 32;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct AnvilUiColor {
        pub r: u8,
        pub g: u8,
        pub b: u8,
        pub a: u8,
    }

    impl From<Color> for AnvilUiColor {
        fn from(c: Color) -> Self {
            Self {
                r: c.r,
                g: c.g,
                b: c.b,
                a: c.a,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct AnvilUiRect {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
    }

    impl From<Rect> for AnvilUiRect {
        fn from(r: Rect) -> Self {
            Self {
                x: r.x,
                y: r.y,
                w: r.width,
                h: r.height,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct AnvilUiTabInfo {
        pub title_ptr: *const u8,
        pub title_len: usize,
        pub rect: AnvilUiRect,
        pub close_rect: AnvilUiRect,
        pub dot_color: AnvilUiColor,
        pub is_active: u8,
        pub is_hovered: u8,
        pub is_close_hovered: u8,
        pub is_pinned: u8,
        pub can_close: u8,
    }

    impl Default for AnvilUiTabInfo {
        fn default() -> Self {
            Self {
                title_ptr: std::ptr::null(),
                title_len: 0,
                rect: AnvilUiRect::default(),
                close_rect: AnvilUiRect::default(),
                dot_color: AnvilUiColor::default(),
                is_active: 0,
                is_hovered: 0,
                is_close_hovered: 0,
                is_pinned: 0,
                can_close: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct AnvilUiBookmarkInfo {
        pub label_ptr: *const u8,
        pub label_len: usize,
        pub rect: AnvilUiRect,
        pub is_hovered: u8,
    }

    impl Default for AnvilUiBookmarkInfo {
        fn default() -> Self {
            Self {
                label_ptr: std::ptr::null(),
                label_len: 0,
                rect: AnvilUiRect::default(),
                is_hovered: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct AnvilUiSuggestionInfo {
        pub badge_ptr: *const u8,
        pub badge_len: usize,
        pub title_ptr: *const u8,
        pub title_len: usize,
        pub url_ptr: *const u8,
        pub url_len: usize,
        pub rect: AnvilUiRect,
        pub is_selected: u8,
        pub is_hovered: u8,
        pub is_search: u8,
    }

    impl Default for AnvilUiSuggestionInfo {
        fn default() -> Self {
            Self {
                badge_ptr: std::ptr::null(),
                badge_len: 0,
                title_ptr: std::ptr::null(),
                title_len: 0,
                url_ptr: std::ptr::null(),
                url_len: 0,
                rect: AnvilUiRect::default(),
                is_selected: 0,
                is_hovered: 0,
                is_search: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct AnvilUiMenuRowInfo {
        pub label_ptr: *const u8,
        pub label_len: usize,
        pub shortcut_ptr: *const u8,
        pub shortcut_len: usize,
        pub rect: AnvilUiRect,
        pub is_hovered: u8,
        pub has_divider_above: u8,
    }

    impl Default for AnvilUiMenuRowInfo {
        fn default() -> Self {
            Self {
                label_ptr: std::ptr::null(),
                label_len: 0,
                shortcut_ptr: std::ptr::null(),
                shortcut_len: 0,
                rect: AnvilUiRect::default(),
                is_hovered: 0,
                has_divider_above: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct AnvilUiSelectRowInfo {
        pub label_ptr: *const u8,
        pub label_len: usize,
        pub rect: AnvilUiRect,
        pub is_selected: u8,
        pub is_hovered: u8,
    }

    impl Default for AnvilUiSelectRowInfo {
        fn default() -> Self {
            Self {
                label_ptr: std::ptr::null(),
                label_len: 0,
                rect: AnvilUiRect::default(),
                is_selected: 0,
                is_hovered: 0,
            }
        }
    }

    #[repr(C)]
    pub struct AnvilUiChromeState {
        pub strip_bg: AnvilUiColor,
        pub tab_idle_bg: AnvilUiColor,
        pub nav_top_bg: AnvilUiColor,
        pub nav_bg: AnvilUiColor,
        pub control_bg: AnvilUiColor,
        pub control_hover_bg: AnvilUiColor,
        pub control_active_bg: AnvilUiColor,
        pub overlay_bg: AnvilUiColor,
        pub field_bg: AnvilUiColor,
        pub divider: AnvilUiColor,
        pub accent: AnvilUiColor,
        pub accent_cyan: AnvilUiColor,
        pub ink_primary: AnvilUiColor,
        pub ink_muted: AnvilUiColor,
        pub ink_dim: AnvilUiColor,

        pub window_width: f32,
        pub window_height: f32,
        pub tab_strip_height: f32,
        pub nav_bar_height: f32,
        pub top_bar_height: f32,
        pub top_header_height: f32,

        pub tab_count: usize,
        pub tabs: [AnvilUiTabInfo; ANVIL_UI_MAX_TABS],
        pub new_tab_rect: AnvilUiRect,
        pub new_tab_hovered: u8,
        pub win_min_rect: AnvilUiRect,
        pub win_max_rect: AnvilUiRect,
        pub win_close_rect: AnvilUiRect,
        pub win_min_hovered: u8,
        pub win_max_hovered: u8,
        pub win_close_hovered: u8,
        pub window_maximized: u8,

        pub back_rect: AnvilUiRect,
        pub forward_rect: AnvilUiRect,
        pub reload_rect: AnvilUiRect,
        pub home_rect: AnvilUiRect,
        pub can_go_back: u8,
        pub can_go_forward: u8,
        pub has_current_page: u8,
        pub back_hovered: u8,
        pub forward_hovered: u8,
        pub reload_hovered: u8,
        pub home_hovered: u8,
        pub is_loading: u8,

        pub omnibox_rect: AnvilUiRect,
        pub security_badge_rect: AnvilUiRect,
        pub address_rect: AnvilUiRect,
        pub clear_address_rect: AnvilUiRect,
        pub star_rect: AnvilUiRect,
        pub sec_label_ptr: *const u8,
        pub sec_label_len: usize,
        pub sec_bg: AnvilUiColor,
        pub sec_fg: AnvilUiColor,
        pub sec_hovered: u8,
        pub omnibox_hovered: u8,
        pub is_editing_address: u8,
        pub is_input_select_all: u8,
        pub show_clear_btn: u8,
        pub clear_hovered: u8,
        pub is_bookmarked: u8,
        pub star_hovered: u8,

        pub use_domain_highlight: u8,
        pub url_scheme_ptr: *const u8,
        pub url_scheme_len: usize,
        pub url_domain_ptr: *const u8,
        pub url_domain_len: usize,
        pub url_path_ptr: *const u8,
        pub url_path_len: usize,
        pub display_text_ptr: *const u8,
        pub display_text_len: usize,
        pub display_text_muted: u8,
        pub selection_char_count: usize,

        pub shields_pill_rect: AnvilUiRect,
        pub shields_text_ptr: *const u8,
        pub shields_text_len: usize,
        pub shields_hovered: u8,
        pub shields_active: u8,

        pub zoom_pill_rect: AnvilUiRect,
        pub zoom_text_ptr: *const u8,
        pub zoom_text_len: usize,
        pub zoom_hovered: u8,
        pub zoom_non_default: u8,

        pub downloads_btn_rect: AnvilUiRect,
        pub downloads_hovered: u8,
        pub downloads_active: u8,
        pub downloads_has_entries: u8,

        pub settings_btn_rect: AnvilUiRect,
        pub settings_hovered: u8,
        pub settings_active: u8,

        pub inspector_btn_rect: AnvilUiRect,
        pub inspector_hovered: u8,
        pub inspector_active: u8,

        pub app_menu_btn_rect: AnvilUiRect,
        pub app_menu_hovered: u8,
        pub app_menu_active: u8,

        pub bookmark_count: usize,
        pub bookmarks: [AnvilUiBookmarkInfo; ANVIL_UI_MAX_BOOKMARKS],

        pub show_floating_status_pill: u8,
        pub status_pill_ptr: *const u8,
        pub status_pill_len: usize,

        pub find_open: u8,
        pub find_bar_rect: AnvilUiRect,
        pub find_prev_rect: AnvilUiRect,
        pub find_next_rect: AnvilUiRect,
        pub find_close_rect: AnvilUiRect,
        pub find_query_ptr: *const u8,
        pub find_query_len: usize,
        pub find_count_ptr: *const u8,
        pub find_count_len: usize,
        pub find_prev_hovered: u8,
        pub find_next_hovered: u8,
        pub find_close_hovered: u8,

        pub suggestion_count: usize,
        pub suggestions_rect: AnvilUiRect,
        pub suggestions: [AnvilUiSuggestionInfo; ANVIL_UI_MAX_SUGGESTIONS],

        pub app_menu_open: u8,
        pub app_menu_rect: AnvilUiRect,
        pub app_menu_row_count: usize,
        pub app_menu_rows: [AnvilUiMenuRowInfo; ANVIL_UI_MAX_MENU_ROWS],

        pub context_menu_open: u8,
        pub context_menu_rect: AnvilUiRect,
        pub context_menu_row_count: usize,
        pub context_menu_rows: [AnvilUiMenuRowInfo; ANVIL_UI_MAX_MENU_ROWS],

        pub select_dropdown_open: u8,
        pub select_dropdown_rect: AnvilUiRect,
        pub select_dropdown_row_count: usize,
        pub select_dropdown_rows: [AnvilUiSelectRowInfo; ANVIL_UI_MAX_SELECT_ROWS],
    }

    unsafe extern "C" {
        pub fn anvil_ui_dwm_configure_window(
            hwnd: *mut std::ffi::c_void,
            dark_mode: i32,
            frameless: i32,
        );
        pub fn anvil_ui_is_maximized(hwnd: *mut std::ffi::c_void) -> i32;
        pub fn anvil_ui_window_action(hwnd: *mut std::ffi::c_void, action: i32);
        pub fn anvil_ui_handle_nchittest(
            hwnd: *mut std::ffi::c_void,
            screen_x: i32,
            screen_y: i32,
            on_draggable_strip: i32,
            on_max_button: i32,
        ) -> isize;
        pub fn anvil_ui_render_top_chrome_d2d(
            rgba_pixels: *mut u8,
            width: u32,
            height: u32,
            state: *const AnvilUiChromeState,
        ) -> i32;
        pub fn anvil_ui_render_overlays_d2d(
            rgba_pixels: *mut u8,
            width: u32,
            height: u32,
            state: *const AnvilUiChromeState,
        ) -> i32;
    }
}

/// Configure Win32 DWM attributes (immersive dark mode, Windows 11 rounded corners, frameless caption).
pub fn cpp_ui_dwm_configure_window(hwnd: isize, dark_mode: bool, frameless: bool) {
    #[cfg(anvil_cpp_ui)]
    unsafe {
        cpp_ui_ffi::anvil_ui_dwm_configure_window(
            hwnd as *mut std::ffi::c_void,
            if dark_mode { 1 } else { 0 },
            if frameless { 1 } else { 0 },
        );
    }
    #[cfg(not(anvil_cpp_ui))]
    {
        let _ = (hwnd, dark_mode, frameless);
    }
}

/// Query whether the Win32 window is currently maximized.
pub fn cpp_ui_is_maximized(hwnd: isize) -> bool {
    #[cfg(anvil_cpp_ui)]
    unsafe {
        cpp_ui_ffi::anvil_ui_is_maximized(hwnd as *mut std::ffi::c_void) != 0
    }
    #[cfg(not(anvil_cpp_ui))]
    {
        let _ = hwnd;
        false
    }
}

/// Perform a native Win32 window action (`1` = Minimize, `2` = Maximize/Restore, `3` = Close, `4` = Begin Caption Drag).
pub fn cpp_ui_window_action(hwnd: isize, action: i32) {
    #[cfg(anvil_cpp_ui)]
    unsafe {
        cpp_ui_ffi::anvil_ui_window_action(hwnd as *mut std::ffi::c_void, action);
    }
    #[cfg(not(anvil_cpp_ui))]
    {
        let _ = (hwnd, action);
    }
}

/// Compute the Win32 `WM_NCHITTEST` hit code for frameless resize borders, caption drag, and Windows 11 Snap Layouts.
pub fn cpp_ui_handle_nchittest(
    hwnd: isize,
    screen_x: i32,
    screen_y: i32,
    on_draggable_strip: bool,
    on_max_button: bool,
) -> isize {
    #[cfg(anvil_cpp_ui)]
    unsafe {
        cpp_ui_ffi::anvil_ui_handle_nchittest(
            hwnd as *mut std::ffi::c_void,
            screen_x,
            screen_y,
            if on_draggable_strip { 1 } else { 0 },
            if on_max_button { 1 } else { 0 },
        )
    }
    #[cfg(not(anvil_cpp_ui))]
    {
        let _ = (hwnd, screen_x, screen_y, on_draggable_strip, on_max_button);
        1 // HTCLIENT
    }
}

#[cfg(anvil_cpp_ui)]
#[allow(clippy::too_many_arguments)]
fn with_cpp_chrome_state<F>(
    session: &Session,
    ui: &UiState,
    pal: &ThemePalette,
    width: u32,
    height: u32,
    f: F,
) where
    F: FnOnce(&cpp_ui_ffi::AnvilUiChromeState),
{
    use cpp_ui_ffi::*;

    let width_f = width as f32;
    let height_f = height as f32;
    let header_h = top_header_height(session);

    let tab_count = session.tab_count().min(ANVIL_UI_MAX_TABS);
    let tab_w = tab_width(width_f, session.tab_count().max(1));
    let mut tab_titles: Vec<String> = Vec::with_capacity(tab_count);
    let mut tabs = [AnvilUiTabInfo::default(); ANVIL_UI_MAX_TABS];

    for idx in 0..tab_count {
        let rect = tab_rect(idx, tab_w);
        let c_rect = close_rect(rect);
        let is_active = idx == session.active_index();
        let is_pinned = session.is_tab_pinned(idx);
        let raw_title = session
            .tabs()
            .get(idx)
            .and_then(|t| t.current())
            .map(|p| {
                if p.title.is_empty() {
                    p.url.as_str()
                } else {
                    p.title.as_str()
                }
            })
            .unwrap_or("New Tab");
        let label = if is_pinned {
            format!("[P] {raw_title}")
        } else {
            raw_title.to_string()
        };
        tab_titles.push(label);
        let tab_url = session
            .tabs()
            .get(idx)
            .and_then(|t| t.current())
            .map(|p| p.url.as_str())
            .unwrap_or("");
        let (_, _, dot_col, _) = classify_url_security(tab_url);
        tabs[idx] = AnvilUiTabInfo {
            title_ptr: std::ptr::null(),
            title_len: 0,
            rect: rect.into(),
            close_rect: c_rect.into(),
            dot_color: dot_col.into(),
            is_active: u8::from(is_active),
            is_hovered: u8::from(ui.is_hovering(rect)),
            is_close_hovered: u8::from(ui.is_hovering(c_rect)),
            is_pinned: u8::from(is_pinned),
            can_close: u8::from(session.tab_count() > 1 && !is_pinned),
        };
    }
    for (idx, title) in tab_titles.iter().enumerate() {
        tabs[idx].title_ptr = title.as_ptr();
        tabs[idx].title_len = title.len();
    }

    let plus_rect = new_tab_rect(session.tab_count(), tab_w);
    let win_min = window_minimize_rect(width_f);
    let win_max = window_maximize_rect(width_f);
    let win_close = window_close_rect(width_f);

    let active_tab = session.active();
    let current_page_url = active_tab
        .current()
        .map(|page| page.url.as_str())
        .unwrap_or("");
    let (sec_label, sec_bg, sec_fg, _) = classify_url_security(current_page_url);
    let badge = security_badge_rect();
    let addr = address_rect(width_f);
    let shields_pill = shields_pill_rect(width_f);
    let star = star_rect(width_f);
    let clear_btn = clear_address_rect(addr);

    let omnibox_rect = Rect {
        x: badge.x,
        y: addr.y,
        width: ((star.x + star.width) - badge.x).max(120.0),
        height: addr.height,
    };
    let omnibox_hovered = ui.is_hovering(addr)
        || ui.is_hovering(badge)
        || ui.is_hovering(shields_pill)
        || ui.is_hovering(star);

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
    let placeholder_label = format!(
        "Search with {} or enter web address",
        session.search_engine().label()
    );
    let display_str = if raw_url.is_empty() {
        placeholder_label.clone()
    } else {
        raw_url.clone()
    };
    let use_domain_highlight = !ui.is_editing_address() && !raw_url.is_empty();
    let (scheme_s, domain_s, path_s) = if use_domain_highlight {
        split_url_for_domain_highlight(&raw_url)
    } else {
        ("", "", "")
    };

    let page_blocked = active_tab
        .current()
        .map(|p| p.blocked_csp_count + p.blocked_tracker_count)
        .unwrap_or(0);
    let total_blocked = session.network_shield().blocked_total() as usize + page_blocked;
    let shields_text = format!("S:{total_blocked}");
    let zoom_text = format!("{}%", session.zoom_percent());

    let bm_count = session.bookmarks().len().min(ANVIL_UI_MAX_BOOKMARKS);
    let mut bm_labels: Vec<String> = Vec::with_capacity(bm_count);
    let mut bookmarks = [AnvilUiBookmarkInfo::default(); ANVIL_UI_MAX_BOOKMARKS];
    for (idx, bm) in session.bookmarks().iter().take(bm_count).enumerate() {
        let r = bookmark_rect(idx);
        let lbl = if bm.title.is_empty() {
            bm.url.clone()
        } else {
            bm.title.clone()
        };
        bm_labels.push(lbl);
        bookmarks[idx] = AnvilUiBookmarkInfo {
            label_ptr: std::ptr::null(),
            label_len: 0,
            rect: r.into(),
            is_hovered: u8::from(ui.is_hovering(r)),
        };
    }
    for (idx, lbl) in bm_labels.iter().enumerate() {
        bookmarks[idx].label_ptr = lbl.as_ptr();
        bookmarks[idx].label_len = lbl.len();
    }

    let status_pill_text = if ui.is_loading() {
        Some("Loading resource over sandboxed transport...".to_string())
    } else {
        ui.hovered_link().map(|link| format!("Link -> {link}"))
    };
    let status_pill_str = status_pill_text.clone().unwrap_or_default();

    let fbar = find_bar_rect(width_f, header_h);
    let fprev = find_prev_rect(fbar);
    let fnext = find_next_rect(fbar);
    let fclose = find_close_rect(fbar);
    let (find_query_str, find_count_str) = if ui.is_find_open() {
        let matches = active_tab
            .current()
            .map(|p| p.find_text_matches(ui.find_query()))
            .unwrap_or_default();
        let mut fchars: Vec<char> = ui.find_query().chars().collect();
        let fcur = ui.find_cursor().min(fchars.len());
        fchars.insert(fcur, '|');
        let fstr: String = fchars.into_iter().collect();
        let active_idx = ui.find_active_match();
        let cnt = if matches.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", (active_idx + 1).min(matches.len()), matches.len())
        };
        (format!("Find: {fstr}"), cnt)
    } else {
        (String::new(), String::new())
    };

    let sugs = if ui.is_editing_address() && !ui.input().trim().is_empty() {
        ui.omnibox_suggestions(session)
    } else {
        Vec::new()
    };
    let sug_count = sugs.len().min(ANVIL_UI_MAX_SUGGESTIONS);
    let srect = omnibox_suggestions_rect(width_f, sug_count.max(1));
    let mut sug_badges: Vec<&'static str> = Vec::with_capacity(sug_count);
    let mut sug_titles: Vec<String> = Vec::with_capacity(sug_count);
    let mut sug_urls: Vec<String> = Vec::with_capacity(sug_count);
    let mut suggestions = [AnvilUiSuggestionInfo::default(); ANVIL_UI_MAX_SUGGESTIONS];
    for (idx, s) in sugs.iter().take(sug_count).enumerate() {
        let r = omnibox_suggestion_row_rect(srect, idx);
        let badge_s = match s.kind {
            SuggestionKind::Bookmark => "BMK",
            SuggestionKind::History => "HIS",
            SuggestionKind::Search => "SRC",
        };
        sug_badges.push(badge_s);
        sug_titles.push(s.title.clone());
        sug_urls.push(s.url.clone());
        suggestions[idx] = AnvilUiSuggestionInfo {
            badge_ptr: std::ptr::null(),
            badge_len: 0,
            title_ptr: std::ptr::null(),
            title_len: 0,
            url_ptr: std::ptr::null(),
            url_len: 0,
            rect: r.into(),
            is_selected: u8::from(ui.omnibox_selected() == Some(idx)),
            is_hovered: u8::from(ui.is_hovering(r)),
            is_search: u8::from(s.kind == SuggestionKind::Search),
        };
    }
    for idx in 0..sug_count {
        suggestions[idx].badge_ptr = sug_badges[idx].as_ptr();
        suggestions[idx].badge_len = sug_badges[idx].len();
        suggestions[idx].title_ptr = sug_titles[idx].as_ptr();
        suggestions[idx].title_len = sug_titles[idx].len();
        suggestions[idx].url_ptr = sug_urls[idx].as_ptr();
        suggestions[idx].url_len = sug_urls[idx].len();
    }

    let app_items = AppMenuAction::all();
    let app_row_count = app_items.len().min(ANVIL_UI_MAX_MENU_ROWS);
    let app_mrect = app_menu_rect(width_f, header_h);
    let mut app_rows = [AnvilUiMenuRowInfo::default(); ANVIL_UI_MAX_MENU_ROWS];
    for (idx, item) in app_items.iter().take(app_row_count).enumerate() {
        let r = Rect {
            x: app_mrect.x + 4.0,
            y: app_mrect.y + APP_MENU_HEADER_H + (idx as f32) * APP_MENU_ITEM_H + 2.0,
            width: app_mrect.width - 8.0,
            height: APP_MENU_ITEM_H,
        };
        let lbl = item.label();
        let sc = item.shortcut();
        app_rows[idx] = AnvilUiMenuRowInfo {
            label_ptr: lbl.as_ptr(),
            label_len: lbl.len(),
            shortcut_ptr: sc.as_ptr(),
            shortcut_len: sc.len(),
            rect: r.into(),
            is_hovered: u8::from(ui.is_hovering(r)),
            has_divider_above: u8::from(idx == 2 || idx == 6 || idx == 9),
        };
    }

    let (ctx_open, ctx_rect, ctx_row_count, ctx_rows) = if let Some((cx, cy)) = ui.context_menu() {
        let has_link = ui.context_menu_link().is_some();
        let items = ContextMenuAction::items_for_context(has_link);
        let cnt = items.len().min(ANVIL_UI_MAX_MENU_ROWS);
        let mrect = context_menu_rect_for_items(cx, cy, width_f, header_h + 480.0, cnt);
        let mut rows = [AnvilUiMenuRowInfo::default(); ANVIL_UI_MAX_MENU_ROWS];
        for (idx, item) in items.iter().take(cnt).enumerate() {
            let r = Rect {
                x: mrect.x + 4.0,
                y: mrect.y + 4.0 + (idx as f32) * CONTEXT_MENU_ITEM_H,
                width: mrect.width - 8.0,
                height: CONTEXT_MENU_ITEM_H,
            };
            let lbl = item.label();
            let sc = item.shortcut();
            let div = if has_link {
                idx == 2 || idx == 6 || idx == 10
            } else {
                idx == 4 || idx == 8
            };
            rows[idx] = AnvilUiMenuRowInfo {
                label_ptr: lbl.as_ptr(),
                label_len: lbl.len(),
                shortcut_ptr: sc.as_ptr(),
                shortcut_len: sc.len(),
                rect: r.into(),
                is_hovered: u8::from(ui.is_hovering(r)),
                has_divider_above: u8::from(div),
            };
        }
        (1u8, mrect.into(), cnt, rows)
    } else {
        (
            0u8,
            AnvilUiRect::default(),
            0usize,
            [AnvilUiMenuRowInfo::default(); ANVIL_UI_MAX_MENU_ROWS],
        )
    };

    let mut sel_labels: Vec<String> = Vec::new();
    let (sel_open, sel_rect, sel_row_count, mut sel_rows) =
        if let Some((_, anchor, options)) = ui.select_dropdown() {
            let cnt = options.len().min(ANVIL_UI_MAX_SELECT_ROWS);
            let drect = select_dropdown_rect(anchor, cnt, width_f, height_f);
            let mut rows = [AnvilUiSelectRowInfo::default(); ANVIL_UI_MAX_SELECT_ROWS];
            for (idx, (_, label, selected)) in options.iter().take(cnt).enumerate() {
                let r = select_dropdown_row_rect(drect, idx);
                let formatted = if *selected {
                    format!("* {label}")
                } else {
                    format!("  {label}")
                };
                sel_labels.push(formatted);
                rows[idx] = AnvilUiSelectRowInfo {
                    label_ptr: std::ptr::null(),
                    label_len: 0,
                    rect: r.into(),
                    is_selected: u8::from(*selected),
                    is_hovered: u8::from(ui.is_hovering(r)),
                };
            }
            (1u8, drect.into(), cnt, rows)
        } else {
            (
                0u8,
                AnvilUiRect::default(),
                0usize,
                [AnvilUiSelectRowInfo::default(); ANVIL_UI_MAX_SELECT_ROWS],
            )
        };
    for (idx, lbl) in sel_labels.iter().enumerate() {
        sel_rows[idx].label_ptr = lbl.as_ptr();
        sel_rows[idx].label_len = lbl.len();
    }

    let b_rect = back_rect();
    let f_rect = forward_rect();
    let r_rect = reload_rect();
    let h_rect = home_rect();
    let z_rect = zoom_pill_rect(width_f);
    let dl_rect = downloads_btn_rect(width_f);
    let set_rect = settings_btn_rect(width_f);
    let insp_rect = inspector_btn_rect(width_f);
    let menu_btn = app_menu_btn_rect(width_f);

    let state = AnvilUiChromeState {
        strip_bg: pal.strip_bg.into(),
        tab_idle_bg: pal.tab_idle_bg.into(),
        nav_top_bg: pal.nav_top_bg.into(),
        nav_bg: pal.nav_bg.into(),
        control_bg: pal.control_bg.into(),
        control_hover_bg: pal.control_hover_bg.into(),
        control_active_bg: pal.control_active_bg.into(),
        overlay_bg: pal.overlay_bg.into(),
        field_bg: pal.field_bg.into(),
        divider: pal.divider.into(),
        accent: pal.accent.into(),
        accent_cyan: pal.accent_cyan.into(),
        ink_primary: pal.ink_primary.into(),
        ink_muted: pal.ink_muted.into(),
        ink_dim: pal.ink_dim.into(),

        window_width: width_f,
        window_height: height_f,
        tab_strip_height: TAB_STRIP_HEIGHT,
        nav_bar_height: NAV_BAR_HEIGHT,
        top_bar_height: TOP_BAR_HEIGHT,
        top_header_height: header_h,

        tab_count,
        tabs,
        new_tab_rect: plus_rect.into(),
        new_tab_hovered: u8::from(ui.is_hovering(plus_rect)),
        win_min_rect: win_min.into(),
        win_max_rect: win_max.into(),
        win_close_rect: win_close.into(),
        win_min_hovered: u8::from(ui.is_hovering(win_min)),
        win_max_hovered: u8::from(ui.is_hovering(win_max)),
        win_close_hovered: u8::from(ui.is_hovering(win_close)),
        window_maximized: 0,

        back_rect: b_rect.into(),
        forward_rect: f_rect.into(),
        reload_rect: r_rect.into(),
        home_rect: h_rect.into(),
        can_go_back: u8::from(active_tab.can_go_back()),
        can_go_forward: u8::from(active_tab.can_go_forward()),
        has_current_page: u8::from(active_tab.current().is_some()),
        back_hovered: u8::from(ui.is_hovering(b_rect)),
        forward_hovered: u8::from(ui.is_hovering(f_rect)),
        reload_hovered: u8::from(ui.is_hovering(r_rect)),
        home_hovered: u8::from(ui.is_hovering(h_rect)),
        is_loading: u8::from(ui.is_loading()),

        omnibox_rect: omnibox_rect.into(),
        security_badge_rect: badge.into(),
        address_rect: addr.into(),
        clear_address_rect: clear_btn.into(),
        star_rect: star.into(),
        sec_label_ptr: sec_label.as_ptr(),
        sec_label_len: sec_label.len(),
        sec_bg: sec_bg.into(),
        sec_fg: sec_fg.into(),
        sec_hovered: u8::from(ui.is_hovering(badge)),
        omnibox_hovered: u8::from(omnibox_hovered),
        is_editing_address: u8::from(ui.is_editing_address()),
        is_input_select_all: u8::from(ui.is_input_select_all()),
        show_clear_btn: u8::from(show_clear_btn),
        clear_hovered: u8::from(ui.is_hovering(clear_btn)),
        is_bookmarked: u8::from(session.is_active_bookmarked()),
        star_hovered: u8::from(ui.is_hovering(star)),

        use_domain_highlight: u8::from(use_domain_highlight && !domain_s.is_empty()),
        url_scheme_ptr: scheme_s.as_ptr(),
        url_scheme_len: scheme_s.len(),
        url_domain_ptr: domain_s.as_ptr(),
        url_domain_len: domain_s.len(),
        url_path_ptr: path_s.as_ptr(),
        url_path_len: path_s.len(),
        display_text_ptr: display_str.as_ptr(),
        display_text_len: display_str.len(),
        display_text_muted: u8::from(raw_url.is_empty()),
        selection_char_count: ui.input().chars().count(),

        shields_pill_rect: shields_pill.into(),
        shields_text_ptr: shields_text.as_ptr(),
        shields_text_len: shields_text.len(),
        shields_hovered: u8::from(ui.is_hovering(shields_pill)),
        shields_active: u8::from(current_page_url.starts_with("anvil://security")),

        zoom_pill_rect: z_rect.into(),
        zoom_text_ptr: zoom_text.as_ptr(),
        zoom_text_len: zoom_text.len(),
        zoom_hovered: u8::from(ui.is_hovering(z_rect)),
        zoom_non_default: u8::from(session.zoom_percent() != 100),

        downloads_btn_rect: dl_rect.into(),
        downloads_hovered: u8::from(ui.is_hovering(dl_rect)),
        downloads_active: u8::from(current_page_url.starts_with("anvil://downloads")),
        downloads_has_entries: u8::from(!session.downloads().entries().is_empty()),

        settings_btn_rect: set_rect.into(),
        settings_hovered: u8::from(ui.is_hovering(set_rect)),
        settings_active: u8::from(current_page_url.starts_with("anvil://settings")),

        inspector_btn_rect: insp_rect.into(),
        inspector_hovered: u8::from(ui.is_hovering(insp_rect)),
        inspector_active: u8::from(
            ui.is_inspector_open() || current_page_url.starts_with("anvil://inspect"),
        ),

        app_menu_btn_rect: menu_btn.into(),
        app_menu_hovered: u8::from(ui.is_hovering(menu_btn)),
        app_menu_active: u8::from(ui.is_app_menu_open()),

        bookmark_count: bm_count,
        bookmarks,

        show_floating_status_pill: u8::from(status_pill_text.is_some()),
        status_pill_ptr: status_pill_str.as_ptr(),
        status_pill_len: status_pill_str.len(),

        find_open: u8::from(ui.is_find_open()),
        find_bar_rect: fbar.into(),
        find_prev_rect: fprev.into(),
        find_next_rect: fnext.into(),
        find_close_rect: fclose.into(),
        find_query_ptr: find_query_str.as_ptr(),
        find_query_len: find_query_str.len(),
        find_count_ptr: find_count_str.as_ptr(),
        find_count_len: find_count_str.len(),
        find_prev_hovered: u8::from(ui.is_hovering(fprev)),
        find_next_hovered: u8::from(ui.is_hovering(fnext)),
        find_close_hovered: u8::from(ui.is_hovering(fclose)),

        suggestion_count: sug_count,
        suggestions_rect: srect.into(),
        suggestions,

        app_menu_open: u8::from(ui.is_app_menu_open()),
        app_menu_rect: app_mrect.into(),
        app_menu_row_count: app_row_count,
        app_menu_rows: app_rows,

        context_menu_open: ctx_open,
        context_menu_rect: ctx_rect,
        context_menu_row_count: ctx_row_count,
        context_menu_rows: ctx_rows,

        select_dropdown_open: sel_open,
        select_dropdown_rect: sel_rect,
        select_dropdown_row_count: sel_row_count,
        select_dropdown_rows: sel_rows,
    };

    f(&state);
}

#[cfg(anvil_cpp_ui)]
fn render_cpp_d2d_top_chrome(
    fb: &mut Framebuffer,
    session: &Session,
    ui: &UiState,
    pal: &ThemePalette,
    width: u32,
    height: u32,
) {
    with_cpp_chrome_state(session, ui, pal, width, height, |state| unsafe {
        let _ = cpp_ui_ffi::anvil_ui_render_top_chrome_d2d(
            fb.pixels_mut().as_mut_ptr(),
            width,
            height,
            state,
        );
    });
}

#[cfg(anvil_cpp_ui)]
fn render_cpp_d2d_overlays(
    fb: &mut Framebuffer,
    session: &Session,
    ui: &UiState,
    pal: &ThemePalette,
    width: u32,
    height: u32,
) {
    with_cpp_chrome_state(session, ui, pal, width, height, |state| unsafe {
        let _ = cpp_ui_ffi::anvil_ui_render_overlays_d2d(
            fb.pixels_mut().as_mut_ptr(),
            width,
            height,
            state,
        );
    });
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

    #[test]
    fn omnibox_suggestions_pinned_tab_hit_and_console_input_work() {
        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.add_bookmark("Rust Lang", "https://www.rust-lang.org");
        session.open_html(
            "<title>Rust Documentation</title><p>Hello</p>",
            "https://doc.rust-lang.org/std",
        );

        let mut ui = UiState::new();
        ui.focus_address("rust");
        let sugs = ui.omnibox_suggestions(&session);
        assert!(sugs.len() >= 2);
        assert!(sugs.iter().any(|s| s.kind == SuggestionKind::Bookmark));
        assert!(sugs.iter().any(|s| s.kind == SuggestionKind::History));
        assert!(sugs.iter().any(|s| s.kind == SuggestionKind::Search));

        // Up/Down arrow cycling through suggestions
        assert_eq!(ui.omnibox_selected(), None);
        assert!(ui.move_omnibox_selection_down(&session));
        assert_eq!(ui.omnibox_selected(), Some(0));
        assert!(ui.selected_omnibox_url(&session).is_some());
        assert!(ui.move_omnibox_selection_down(&session));
        assert_eq!(ui.omnibox_selected(), Some(1));
        assert!(ui.move_omnibox_selection_up(&session));
        assert_eq!(ui.omnibox_selected(), Some(0));

        // Hit testing on the floating Omnibox suggestion popover
        let srect = omnibox_suggestions_rect(800.0, sugs.len());
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, srect.x + 20.0, srect.y + 12.0),
            Some(UiHit::OmniboxSuggestion(0))
        );
        ui.blur_address();

        // Pinned tab suppresses CloseTab hit on its right edge
        assert!(session.toggle_pin_tab(0));
        let tab0 = tab_rect(0, tab_width(800.0, 1));
        assert_eq!(
            hit_test_ui_with_state(&session, &ui, 800.0, tab0.x + tab0.width - 10.0, 20.0),
            Some(UiHit::Tab(0))
        );

        // Inspector Console input hit testing & editing
        ui.open_inspector();
        ui.set_inspector_tab(InspectorTab::Console);
        let dock = inspector_dock_rect(800.0, 600.0);
        let cinput = inspector_console_input_rect(dock);
        assert_eq!(
            hit_test_ui_with_state_and_height(
                &session,
                &ui,
                800.0,
                600.0,
                cinput.x + 20.0,
                cinput.y + 10.0
            ),
            Some(UiHit::InspectorConsoleInput)
        );
        ui.focus_console();
        assert!(ui.is_editing_console());
        ui.push_char('4');
        ui.push_char('0');
        ui.push_char('+');
        ui.push_char('2');
        assert_eq!(ui.console_input(), "40+2");
        assert_eq!(ui.take_console_input().as_deref(), Some("40+2"));
        assert_eq!(ui.console_input(), "");

        // 5-Tab DevTools: Styles live CSS override input and Storage tab hit testing
        assert_eq!(InspectorTab::ALL.len(), 5);
        let storage_tab = inspector_tab_rect(dock, 4);
        assert_eq!(
            hit_test_ui_with_state_and_height(
                &session,
                &ui,
                800.0,
                600.0,
                storage_tab.x + 20.0,
                storage_tab.y + 10.0
            ),
            Some(UiHit::InspectorTab(InspectorTab::Storage))
        );
        ui.set_inspector_tab(InspectorTab::Styles);
        let sinput = inspector_style_input_rect(dock);
        assert_eq!(
            hit_test_ui_with_state_and_height(
                &session,
                &ui,
                800.0,
                600.0,
                sinput.x + 20.0,
                sinput.y + 10.0
            ),
            Some(UiHit::InspectorStyleInput)
        );
        ui.focus_style();
        assert!(ui.is_editing_style());
        for ch in "color: #ff6b24".chars() {
            ui.push_char(ch);
        }
        assert_eq!(ui.style_input(), "color: #ff6b24");
        let decl = ui.take_style_input().expect("style input");
        assert!(session.apply_inspected_style_override(&decl));
    }

    #[test]
    fn domain_highlight_caption_buttons_shields_and_frameless_status_work() {
        assert_eq!(
            split_url_for_domain_highlight("https://doc.rust-lang.org/std/vec/struct.Vec.html"),
            ("https://", "doc.rust-lang.org", "/std/vec/struct.Vec.html")
        );
        assert_eq!(
            split_url_for_domain_highlight("anvil://settings"),
            ("anvil://", "settings", "")
        );

        let mut session = Session::new(content_viewport(800.0, 600.0));
        session.open_html(
            "<title>Test</title><p>Hello</p>",
            "https://example.com/index.html",
        );
        assert_eq!(STATUS_BAR_HEIGHT, 0.0);

        assert_eq!(
            hit_test_ui(&session, 800.0, 680.0, 14.0),
            Some(UiHit::WindowMinimize)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 725.0, 14.0),
            Some(UiHit::WindowMaximize)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 775.0, 14.0),
            Some(UiHit::WindowClose)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 520.0, 14.0),
            Some(UiHit::TitlebarDrag)
        );
        assert_eq!(
            hit_test_ui(&session, 800.0, 550.0, 52.0),
            Some(UiHit::ShieldsButton)
        );
    }
}
