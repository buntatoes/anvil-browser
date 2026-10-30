// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// C-ABI Bridge between the Native C++20 Desktop UI (`ui/`) and the Rust Browser Core (`crates/anvil-browser`).

#ifndef ANVIL_CORE_ABI_H
#define ANVIL_CORE_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ANVIL_UI_MAX_TABS 32
#define ANVIL_UI_MAX_BOOKMARKS 24
#define ANVIL_UI_MAX_SUGGESTIONS 12
#define ANVIL_UI_MAX_MENU_ROWS 24
#define ANVIL_UI_MAX_SELECT_ROWS 32

typedef struct AnvilUiColor {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
} AnvilUiColor;

typedef struct AnvilUiRect {
    float x;
    float y;
    float w;
    float h;
} AnvilUiRect;

typedef struct AnvilUiTabInfo {
    const char* title_ptr;
    size_t title_len;
    AnvilUiRect rect;
    AnvilUiRect close_rect;
    AnvilUiColor dot_color;
    uint8_t is_active;
    uint8_t is_hovered;
    uint8_t is_close_hovered;
    uint8_t is_pinned;
    uint8_t can_close;
} AnvilUiTabInfo;

typedef struct AnvilUiBookmarkInfo {
    const char* label_ptr;
    size_t label_len;
    AnvilUiRect rect;
    uint8_t is_hovered;
} AnvilUiBookmarkInfo;

typedef struct AnvilUiSuggestionInfo {
    const char* badge_ptr;
    size_t badge_len;
    const char* title_ptr;
    size_t title_len;
    const char* url_ptr;
    size_t url_len;
    AnvilUiRect rect;
    uint8_t is_selected;
    uint8_t is_hovered;
    uint8_t is_search;
} AnvilUiSuggestionInfo;

typedef struct AnvilUiMenuRowInfo {
    const char* label_ptr;
    size_t label_len;
    const char* shortcut_ptr;
    size_t shortcut_len;
    AnvilUiRect rect;
    uint8_t is_hovered;
    uint8_t has_divider_above;
} AnvilUiMenuRowInfo;

typedef struct AnvilUiSelectRowInfo {
    const char* label_ptr;
    size_t label_len;
    AnvilUiRect rect;
    uint8_t is_selected;
    uint8_t is_hovered;
} AnvilUiSelectRowInfo;

typedef struct AnvilUiChromeState {
    // Theme palette
    AnvilUiColor strip_bg;
    AnvilUiColor tab_idle_bg;
    AnvilUiColor nav_top_bg;
    AnvilUiColor nav_bg;
    AnvilUiColor control_bg;
    AnvilUiColor control_hover_bg;
    AnvilUiColor control_active_bg;
    AnvilUiColor overlay_bg;
    AnvilUiColor field_bg;
    AnvilUiColor divider;
    AnvilUiColor accent;
    AnvilUiColor accent_cyan;
    AnvilUiColor ink_primary;
    AnvilUiColor ink_muted;
    AnvilUiColor ink_dim;

    // Geometry
    float window_width;
    float window_height;
    float tab_strip_height;
    float nav_bar_height;
    float top_bar_height;
    float top_header_height;

    // Tabs & Caption buttons
    size_t tab_count;
    AnvilUiTabInfo tabs[ANVIL_UI_MAX_TABS];
    AnvilUiRect new_tab_rect;
    uint8_t new_tab_hovered;
    AnvilUiRect win_min_rect;
    AnvilUiRect win_max_rect;
    AnvilUiRect win_close_rect;
    uint8_t win_min_hovered;
    uint8_t win_max_hovered;
    uint8_t win_close_hovered;
    uint8_t window_maximized;

    // Navigation buttons
    AnvilUiRect back_rect;
    AnvilUiRect forward_rect;
    AnvilUiRect reload_rect;
    AnvilUiRect home_rect;
    uint8_t can_go_back;
    uint8_t can_go_forward;
    uint8_t has_current_page;
    uint8_t back_hovered;
    uint8_t forward_hovered;
    uint8_t reload_hovered;
    uint8_t home_hovered;
    uint8_t is_loading;

    // Pill Omnibox & Domain Highlighting
    AnvilUiRect omnibox_rect;
    AnvilUiRect security_badge_rect;
    AnvilUiRect address_rect;
    AnvilUiRect clear_address_rect;
    AnvilUiRect star_rect;
    const char* sec_label_ptr;
    size_t sec_label_len;
    AnvilUiColor sec_bg;
    AnvilUiColor sec_fg;
    uint8_t sec_hovered;
    uint8_t omnibox_hovered;
    uint8_t is_editing_address;
    uint8_t is_input_select_all;
    uint8_t show_clear_btn;
    uint8_t clear_hovered;
    uint8_t is_bookmarked;
    uint8_t star_hovered;

    // URL display (either domain-split or editing/placeholder)
    uint8_t use_domain_highlight;
    const char* url_scheme_ptr;
    size_t url_scheme_len;
    const char* url_domain_ptr;
    size_t url_domain_len;
    const char* url_path_ptr;
    size_t url_path_len;
    const char* display_text_ptr;
    size_t display_text_len;
    uint8_t display_text_muted;
    size_t selection_char_count;

    // Right Action Cluster
    AnvilUiRect shields_pill_rect;
    const char* shields_text_ptr;
    size_t shields_text_len;
    uint8_t shields_hovered;
    uint8_t shields_active;

    AnvilUiRect zoom_pill_rect;
    const char* zoom_text_ptr;
    size_t zoom_text_len;
    uint8_t zoom_hovered;
    uint8_t zoom_non_default;

    AnvilUiRect downloads_btn_rect;
    uint8_t downloads_hovered;
    uint8_t downloads_active;
    uint8_t downloads_has_entries;

    AnvilUiRect settings_btn_rect;
    uint8_t settings_hovered;
    uint8_t settings_active;

    AnvilUiRect inspector_btn_rect;
    uint8_t inspector_hovered;
    uint8_t inspector_active;

    AnvilUiRect app_menu_btn_rect;
    uint8_t app_menu_hovered;
    uint8_t app_menu_active;

    // Bookmarks bar
    size_t bookmark_count;
    AnvilUiBookmarkInfo bookmarks[ANVIL_UI_MAX_BOOKMARKS];

    // Floating bottom-left status / link-hover pill
    uint8_t show_floating_status_pill;
    const char* status_pill_ptr;
    size_t status_pill_len;

    // Floating Find bar
    uint8_t find_open;
    AnvilUiRect find_bar_rect;
    AnvilUiRect find_prev_rect;
    AnvilUiRect find_next_rect;
    AnvilUiRect find_close_rect;
    const char* find_query_ptr;
    size_t find_query_len;
    const char* find_count_ptr;
    size_t find_count_len;
    uint8_t find_prev_hovered;
    uint8_t find_next_hovered;
    uint8_t find_close_hovered;

    // Floating Omnibox Suggestions
    size_t suggestion_count;
    AnvilUiRect suggestions_rect;
    AnvilUiSuggestionInfo suggestions[ANVIL_UI_MAX_SUGGESTIONS];

    // Floating App Menu
    uint8_t app_menu_open;
    AnvilUiRect app_menu_rect;
    size_t app_menu_row_count;
    AnvilUiMenuRowInfo app_menu_rows[ANVIL_UI_MAX_MENU_ROWS];

    // Floating Context Menu
    uint8_t context_menu_open;
    AnvilUiRect context_menu_rect;
    size_t context_menu_row_count;
    AnvilUiMenuRowInfo context_menu_rows[ANVIL_UI_MAX_MENU_ROWS];

    // Floating Select Dropdown
    uint8_t select_dropdown_open;
    AnvilUiRect select_dropdown_rect;
    size_t select_dropdown_row_count;
    AnvilUiSelectRowInfo select_dropdown_rows[ANVIL_UI_MAX_SELECT_ROWS];
} AnvilUiChromeState;

// Configure DWM attributes on a Win32 HWND (dark mode, rounded corners, frameless client extension).
void anvil_ui_dwm_configure_window(void* hwnd, int dark_mode, int frameless);

// Query whether the Win32 HWND is currently maximized (`IsZoomed`).
int anvil_ui_is_maximized(void* hwnd);

// Perform a native window action: 1 = Minimize, 2 = Toggle Maximize/Restore, 3 = Close, 4 = Begin Caption Drag.
void anvil_ui_window_action(void* hwnd, int action);

// Compute the Win32 WM_NCHITTEST result for frameless resize borders, HTCAPTION drag, and HTMAXBUTTON Snap Layouts.
intptr_t anvil_ui_handle_nchittest(
    void* hwnd,
    int screen_x,
    int screen_y,
    int on_draggable_strip,
    int on_max_button
);

// Render the top chrome (tabs, caption buttons, nav bar, pill omnibox, bookmarks) via Direct2D + DirectWrite.
// Returns 1 on success, 0 if Direct2D/DirectWrite is unavailable.
int anvil_ui_render_top_chrome_d2d(
    uint8_t* rgba_pixels,
    uint32_t width,
    uint32_t height,
    const AnvilUiChromeState* state
);

// Render floating overlays (status pill, find bar, suggestions, app menu, context menu, select dropdown)
// over the composited page framebuffer via Direct2D + DirectWrite.
int anvil_ui_render_overlays_d2d(
    uint8_t* rgba_pixels,
    uint32_t width,
    uint32_t height,
    const AnvilUiChromeState* state
);

#ifdef __cplusplus
}
#endif

#endif // ANVIL_CORE_ABI_H
