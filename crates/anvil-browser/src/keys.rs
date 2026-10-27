// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Keyboard input for the window, as plain data.
//!
//! The X11 loop turns each key press into a [`Key`] and [`Modifiers`] here,
//! asks [`command_for`] what that key means for whatever has focus, and then
//! carries out the [`KeyCommand`]. Nothing in this file talks to X11, so the
//! mapping and the focus rules are tested without a display.
//!
//! Keysym values come from the X11 protocol keysym list. Printable keys are
//! read from the keysym, not from the text XLookupString writes, because with
//! Control held that text is a control byte instead of the letter.
//!
//! [`apply_key_command`] then changes the session and window state. It also
//! has no X11 in it, so typing into the address bar, find, and page fields is
//! tested headless too.

use crate::frontend::{DOWNLOADS_URL, HISTORY_URL, HOME_URL};
use crate::ui::UiState;
use crate::{ClickOutcome, Session};

/// Shift, Control, and Alt state for one key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// `ShiftMask` in an X11 key event's `state` word.
const X11_SHIFT_MASK: u32 = 1 << 0;
/// `ControlMask` in an X11 key event's `state` word.
const X11_CONTROL_MASK: u32 = 1 << 2;
/// `Mod1Mask`, which is Alt on common X11 setups.
const X11_MOD1_MASK: u32 = 1 << 3;

impl Modifiers {
    /// Read Shift, Control, and Alt from an X11 key event `state` word.
    /// Caps Lock and Num Lock are already applied to the keysym by X11.
    pub fn from_x11_state(state: u32) -> Self {
        Self {
            shift: state & X11_SHIFT_MASK != 0,
            ctrl: state & X11_CONTROL_MASK != 0,
            alt: state & X11_MOD1_MASK != 0,
        }
    }
}

/// One key, after the keysym has been decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A key that types this character.
    Char(char),
    Backspace,
    Delete,
    Tab,
    Enter,
    Escape,
    Home,
    End,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    /// Function key 1 through 12.
    F(u8),
    /// A key this browser does not use, such as a lone modifier.
    Other,
}

// X11 keysym values used below.
const XK_BACKSPACE: u64 = 0xff08;
const XK_TAB: u64 = 0xff09;
const XK_RETURN: u64 = 0xff0d;
const XK_ESCAPE: u64 = 0xff1b;
const XK_HOME: u64 = 0xff50;
const XK_LEFT: u64 = 0xff51;
const XK_UP: u64 = 0xff52;
const XK_RIGHT: u64 = 0xff53;
const XK_DOWN: u64 = 0xff54;
const XK_PAGE_UP: u64 = 0xff55;
const XK_PAGE_DOWN: u64 = 0xff56;
const XK_END: u64 = 0xff57;
const XK_DELETE: u64 = 0xffff;
const XK_ISO_LEFT_TAB: u64 = 0xfe20;
const XK_KP_SPACE: u64 = 0xff80;
const XK_KP_TAB: u64 = 0xff89;
const XK_KP_ENTER: u64 = 0xff8d;
const XK_KP_HOME: u64 = 0xff95;
const XK_KP_LEFT: u64 = 0xff96;
const XK_KP_UP: u64 = 0xff97;
const XK_KP_RIGHT: u64 = 0xff98;
const XK_KP_DOWN: u64 = 0xff99;
const XK_KP_PAGE_UP: u64 = 0xff9a;
const XK_KP_PAGE_DOWN: u64 = 0xff9b;
const XK_KP_END: u64 = 0xff9c;
const XK_KP_DELETE: u64 = 0xff9f;
const XK_KP_MULTIPLY: u64 = 0xffaa;
const XK_KP_ADD: u64 = 0xffab;
const XK_KP_SEPARATOR: u64 = 0xffac;
const XK_KP_SUBTRACT: u64 = 0xffad;
const XK_KP_DECIMAL: u64 = 0xffae;
const XK_KP_DIVIDE: u64 = 0xffaf;
const XK_KP_0: u64 = 0xffb0;
const XK_KP_9: u64 = 0xffb9;
const XK_KP_EQUAL: u64 = 0xffbd;
const XK_F1: u64 = 0xffbe;
const XK_F12: u64 = 0xffc9;
const XK_EURO_SIGN: u64 = 0x20ac;
/// Keysyms from this value up carry a Unicode code point in the low 24 bits.
const XK_UNICODE_BASE: u64 = 0x0100_0000;

/// Decode an X11 keysym into a [`Key`].
pub fn key_from_x11(keysym: u64) -> Key {
    match keysym {
        XK_BACKSPACE => Key::Backspace,
        XK_TAB | XK_KP_TAB | XK_ISO_LEFT_TAB => Key::Tab,
        XK_RETURN | XK_KP_ENTER => Key::Enter,
        XK_ESCAPE => Key::Escape,
        XK_HOME | XK_KP_HOME => Key::Home,
        XK_END | XK_KP_END => Key::End,
        XK_LEFT | XK_KP_LEFT => Key::Left,
        XK_RIGHT | XK_KP_RIGHT => Key::Right,
        XK_UP | XK_KP_UP => Key::Up,
        XK_DOWN | XK_KP_DOWN => Key::Down,
        XK_PAGE_UP | XK_KP_PAGE_UP => Key::PageUp,
        XK_PAGE_DOWN | XK_KP_PAGE_DOWN => Key::PageDown,
        XK_DELETE | XK_KP_DELETE => Key::Delete,
        XK_F1..=XK_F12 => Key::F((keysym - XK_F1 + 1) as u8),
        _ => keysym_to_char(keysym).map_or(Key::Other, Key::Char),
    }
}

/// The character a keysym types, if it types one.
///
/// Covers Latin-1 (which shares its numbers with Unicode), the keypad digits
/// and operators, the euro sign, and the Unicode keysym range. Control
/// characters never come back from here.
pub fn keysym_to_char(keysym: u64) -> Option<char> {
    let code = match keysym {
        0x20..=0x7e | 0xa0..=0xff => keysym as u32,
        XK_KP_SPACE => ' ' as u32,
        XK_KP_0..=XK_KP_9 => '0' as u32 + (keysym - XK_KP_0) as u32,
        XK_KP_MULTIPLY => '*' as u32,
        XK_KP_ADD => '+' as u32,
        XK_KP_SEPARATOR => ',' as u32,
        XK_KP_SUBTRACT => '-' as u32,
        XK_KP_DECIMAL => '.' as u32,
        XK_KP_DIVIDE => '/' as u32,
        XK_KP_EQUAL => '=' as u32,
        XK_EURO_SIGN => 0x20ac,
        k if (XK_UNICODE_BASE..=XK_UNICODE_BASE + 0x10_ffff).contains(&k) => {
            (k - XK_UNICODE_BASE) as u32
        }
        _ => return None,
    };
    char::from_u32(code).filter(|ch| !ch.is_control())
}

/// What has keyboard focus when a key is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The address bar is being edited.
    Address,
    /// The find bar is open and no page field is focused.
    Find,
    /// A text field on the page is focused.
    PageField,
    /// Nothing is being edited.
    Page,
}

/// What a key press should do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyCommand {
    /// Type a character into the focused field.
    Insert(char),
    /// Delete the character before the caret in the focused field.
    DeleteBack,
    /// Enter in the address bar or a page field.
    Submit,
    /// Tab in the address bar.
    AcceptSuggestion,
    /// Leave the address bar or page field, or close find.
    Leave,
    FindNext,
    FindPrevious,
    NewTab,
    CloseTab,
    FocusAddress,
    OpenFind,
    OpenHistory,
    OpenDownloads,
    Reload,
    Copy,
    Cut,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Back,
    Forward,
    StartPage,
    ToggleInspector,
    ScrollToTop,
    ScrollToBottom,
    /// Scroll by this many 40 pixel steps. Negative is up.
    ScrollLines(i32),
    /// Scroll by this many viewport pages (80 percent each). Negative is up.
    ScrollPages(i32),
    /// Escape with nothing focused. Close the top overlay, or the window when none is open.
    Dismiss,
    /// The key does nothing here.
    Nothing,
}

/// Decide what `key` does, given the modifiers and what has focus.
///
/// Control and Alt shortcuts work from any focus, so Control+L or Control+F
/// still work while typing in a page field. A Control or Alt chord that is
/// not a shortcut does not type anything.
pub fn command_for(key: Key, mods: Modifiers, focus: Focus) -> KeyCommand {
    if mods.ctrl
        && !mods.alt
        && let Some(command) = control_shortcut(key, mods)
    {
        return command;
    }
    if mods.alt && !mods.ctrl {
        match key {
            Key::Left => return KeyCommand::Back,
            Key::Right => return KeyCommand::Forward,
            Key::Home => return KeyCommand::StartPage,
            _ => {}
        }
    }
    match key {
        Key::F(5) => return KeyCommand::Reload,
        Key::F(6) => return KeyCommand::FocusAddress,
        Key::F(12) => return KeyCommand::ToggleInspector,
        Key::F(3) if mods.shift => return KeyCommand::FindPrevious,
        Key::F(3) => return KeyCommand::FindNext,
        _ => {}
    }
    let typing = !mods.ctrl && !mods.alt;
    match focus {
        Focus::Address => match key {
            Key::Char(ch) if typing => KeyCommand::Insert(ch),
            Key::Backspace => KeyCommand::DeleteBack,
            Key::Enter => KeyCommand::Submit,
            Key::Tab => KeyCommand::AcceptSuggestion,
            Key::Escape => KeyCommand::Leave,
            _ => KeyCommand::Nothing,
        },
        Focus::Find => match key {
            Key::Char(ch) if typing => KeyCommand::Insert(ch),
            Key::Backspace => KeyCommand::DeleteBack,
            Key::Enter if mods.shift => KeyCommand::FindPrevious,
            Key::Enter => KeyCommand::FindNext,
            Key::Escape => KeyCommand::Leave,
            _ => KeyCommand::Nothing,
        },
        Focus::PageField => match key {
            Key::Char(ch) if typing => KeyCommand::Insert(ch),
            Key::Backspace => KeyCommand::DeleteBack,
            Key::Enter => KeyCommand::Submit,
            Key::Escape => KeyCommand::Leave,
            _ => KeyCommand::Nothing,
        },
        Focus::Page => match key {
            Key::Escape => KeyCommand::Dismiss,
            Key::Backspace | Key::Left if typing => KeyCommand::Back,
            Key::Right if typing => KeyCommand::Forward,
            Key::Home => KeyCommand::ScrollToTop,
            Key::End => KeyCommand::ScrollToBottom,
            Key::Up => KeyCommand::ScrollLines(-1),
            Key::Down => KeyCommand::ScrollLines(1),
            Key::PageUp => KeyCommand::ScrollPages(-1),
            Key::PageDown => KeyCommand::ScrollPages(1),
            Key::Char(' ') if typing && mods.shift => KeyCommand::ScrollPages(-1),
            Key::Char(' ') if typing => KeyCommand::ScrollPages(1),
            Key::Char('/') if typing => KeyCommand::OpenFind,
            _ => KeyCommand::Nothing,
        },
    }
}

/// Control chords. Letters match either case, since Shift changes the keysym.
fn control_shortcut(key: Key, mods: Modifiers) -> Option<KeyCommand> {
    let Key::Char(ch) = key else {
        return None;
    };
    let command = match ch.to_ascii_lowercase() {
        't' => KeyCommand::NewTab,
        'w' => KeyCommand::CloseTab,
        'l' => KeyCommand::FocusAddress,
        'f' => KeyCommand::OpenFind,
        'g' if mods.shift => KeyCommand::FindPrevious,
        'g' => KeyCommand::FindNext,
        'h' => KeyCommand::OpenHistory,
        'j' => KeyCommand::OpenDownloads,
        'r' => KeyCommand::Reload,
        'c' => KeyCommand::Copy,
        'x' => KeyCommand::Cut,
        'v' => KeyCommand::Paste,
        '+' | '=' => KeyCommand::ZoomIn,
        '-' | '_' => KeyCommand::ZoomOut,
        '0' => KeyCommand::ZoomReset,
        _ => return None,
    };
    Some(command)
}

/// What the window has to do after a key command ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyEffect {
    /// Nothing changed.
    Nothing,
    /// Session or window state changed. Draw again.
    Redraw,
    /// Start loading this target in the active tab.
    Navigate(String),
    /// Submit a form body to this URL in the active tab.
    Post { url: String, body: String },
    /// Escape with nothing left to close. The window closes.
    CloseWindow,
}

/// What has focus right now, in the order the window checks it.
pub fn focus_of(session: &Session, ui: &UiState) -> Focus {
    if ui.is_editing_address() {
        Focus::Address
    } else if session.has_focused_input() {
        Focus::PageField
    } else if ui.is_find_open() {
        Focus::Find
    } else {
        Focus::Page
    }
}

/// Carry out `command` on the session and window state.
///
/// Loads are not started here. They come back as [`KeyEffect::Navigate`] or
/// [`KeyEffect::Post`] so the window can run them off its event loop.
pub fn apply_key_command(
    session: &mut Session,
    ui: &mut UiState,
    command: KeyCommand,
) -> KeyEffect {
    let focus = focus_of(session, ui);
    let redraw_if = |changed: bool| {
        if changed {
            KeyEffect::Redraw
        } else {
            KeyEffect::Nothing
        }
    };
    match command {
        KeyCommand::Insert(ch) => match focus {
            Focus::Address => {
                ui.push_char(ch);
                KeyEffect::Redraw
            }
            Focus::Find => {
                ui.push_char(ch);
                scroll_to_find_match(session, ui);
                KeyEffect::Redraw
            }
            Focus::PageField => redraw_if(session.type_into_focused_input(ch)),
            Focus::Page => KeyEffect::Nothing,
        },
        KeyCommand::DeleteBack => match focus {
            Focus::Address => {
                ui.backspace();
                KeyEffect::Redraw
            }
            Focus::Find => {
                ui.backspace();
                scroll_to_find_match(session, ui);
                KeyEffect::Redraw
            }
            Focus::PageField => redraw_if(session.backspace_focused_input()),
            Focus::Page => KeyEffect::Nothing,
        },
        KeyCommand::Submit => match focus {
            Focus::Address => {
                let engine = session.search_engine();
                match ui.submit_address_with_engine(engine) {
                    Some(target) => KeyEffect::Navigate(target),
                    None => KeyEffect::Redraw,
                }
            }
            Focus::PageField => match session.submit_focused_input() {
                ClickOutcome::Navigate(target) => KeyEffect::Navigate(target),
                ClickOutcome::SubmitPost { url, body } => KeyEffect::Post { url, body },
                _ => KeyEffect::Redraw,
            },
            _ => KeyEffect::Nothing,
        },
        KeyCommand::AcceptSuggestion => redraw_if(ui.accept_autocomplete(session)),
        KeyCommand::Leave => match focus {
            Focus::Address => {
                ui.blur_address();
                KeyEffect::Redraw
            }
            Focus::Find => {
                ui.close_find();
                KeyEffect::Redraw
            }
            Focus::PageField => redraw_if(session.blur_page_input()),
            Focus::Page => KeyEffect::Nothing,
        },
        KeyCommand::FindNext | KeyCommand::FindPrevious => {
            if !ui.is_find_open() {
                session.blur_page_input();
                ui.open_find();
                return KeyEffect::Redraw;
            }
            let count = find_match_count(session, ui);
            if command == KeyCommand::FindNext {
                ui.find_next(count);
            } else {
                ui.find_prev(count);
            }
            scroll_to_find_match(session, ui);
            KeyEffect::Redraw
        }
        KeyCommand::NewTab => {
            ui.blur_address();
            session.blur_page_input();
            session.new_tab();
            KeyEffect::Navigate(HOME_URL.to_string())
        }
        KeyCommand::CloseTab => {
            ui.blur_address();
            session.blur_page_input();
            let active = session.active_index();
            redraw_if(session.close_tab(active))
        }
        KeyCommand::FocusAddress => {
            session.blur_page_input();
            let url = current_url(session);
            ui.focus_address(&url);
            KeyEffect::Redraw
        }
        KeyCommand::OpenFind => {
            session.blur_page_input();
            ui.open_find();
            KeyEffect::Redraw
        }
        KeyCommand::OpenHistory | KeyCommand::OpenDownloads | KeyCommand::StartPage => {
            ui.blur_address();
            session.blur_page_input();
            let target = match command {
                KeyCommand::OpenHistory => HISTORY_URL,
                KeyCommand::OpenDownloads => DOWNLOADS_URL,
                _ => HOME_URL,
            };
            KeyEffect::Navigate(target.to_string())
        }
        KeyCommand::Reload => {
            ui.blur_address();
            session.blur_page_input();
            let url = current_url(session);
            if url.is_empty() {
                KeyEffect::Redraw
            } else {
                KeyEffect::Navigate(url)
            }
        }
        KeyCommand::Copy => redraw_if(session.copy_selection_or_focused().is_some()),
        KeyCommand::Cut => redraw_if(session.cut_focused_input().is_some()),
        KeyCommand::Paste => {
            let clip = session.clipboard_text().to_string();
            if clip.is_empty() {
                return KeyEffect::Nothing;
            }
            match focus {
                Focus::Address => redraw_if(ui.paste_into_address(&clip)),
                Focus::Find => {
                    for ch in clip.chars() {
                        ui.push_char(ch);
                    }
                    scroll_to_find_match(session, ui);
                    KeyEffect::Redraw
                }
                Focus::PageField => redraw_if(session.paste_into_focused_input(&clip)),
                Focus::Page => KeyEffect::Nothing,
            }
        }
        KeyCommand::ZoomIn => {
            let before = session.zoom_percent();
            redraw_if(session.zoom_in() != before)
        }
        KeyCommand::ZoomOut => {
            let before = session.zoom_percent();
            redraw_if(session.zoom_out() != before)
        }
        KeyCommand::ZoomReset => {
            let before = session.zoom_percent();
            redraw_if(session.reset_zoom() != before)
        }
        KeyCommand::Back => redraw_if(session.back()),
        KeyCommand::Forward => redraw_if(session.forward()),
        KeyCommand::ToggleInspector => {
            ui.toggle_inspector();
            KeyEffect::Redraw
        }
        KeyCommand::ScrollToTop => scroll_effect(session, |s| s.scroll_to_ratio(0.0)),
        KeyCommand::ScrollToBottom => scroll_effect(session, |s| s.scroll_to_ratio(1.0)),
        KeyCommand::ScrollLines(lines) => {
            scroll_effect(session, |s| s.scroll_by(lines as f32 * 40.0))
        }
        KeyCommand::ScrollPages(pages) => scroll_effect(session, |s| {
            let step = s.viewport().height * 0.8;
            s.scroll_by(pages as f32 * step)
        }),
        KeyCommand::Dismiss => {
            if ui.context_menu_pos().is_some() {
                ui.close_context_menu();
            } else if ui.is_app_menu_open() {
                ui.close_app_menu();
            } else if ui.is_find_open() {
                ui.close_find();
            } else if ui.is_inspector_open() {
                ui.close_inspector();
            } else {
                return KeyEffect::CloseWindow;
            }
            KeyEffect::Redraw
        }
        KeyCommand::Nothing => KeyEffect::Nothing,
    }
}

/// URL of the active page, or an empty string.
fn current_url(session: &Session) -> String {
    session
        .active()
        .current()
        .map(|page| page.url.clone())
        .unwrap_or_default()
}

/// Run a scroll and report whether the position moved.
fn scroll_effect(session: &mut Session, scroll: impl FnOnce(&mut Session) -> f32) -> KeyEffect {
    let before = session.scroll();
    let after = scroll(session);
    if (after - before).abs() > f32::EPSILON {
        KeyEffect::Redraw
    } else {
        KeyEffect::Nothing
    }
}

/// How many times the find query occurs on the active page.
pub fn find_match_count(session: &Session, ui: &UiState) -> usize {
    let query = ui.find_query();
    if query.is_empty() {
        return 0;
    }
    session
        .active()
        .current()
        .map(|page| page.find_text_matches(query).len())
        .unwrap_or(0)
}

/// Scroll so the active find match sits about 80 pixels below the top.
pub fn scroll_to_find_match(session: &mut Session, ui: &UiState) {
    let query = ui.find_query();
    if query.is_empty() {
        return;
    }
    let Some(rect) = session.active().current().and_then(|page| {
        let matches = page.find_text_matches(query);
        let idx = ui.find_active_match().min(matches.len().checked_sub(1)?);
        matches.get(idx).copied()
    }) else {
        return;
    };
    let target_y = (rect.y * session.zoom_factor() - 80.0).max(0.0);
    let delta = target_y - session.scroll();
    let _ = session.scroll_by(delta);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: false,
    };
    const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const ALT: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: true,
    };

    use crate::Viewport;

    /// Feed X11 keysyms through the mapping and the session, as the window does.
    fn press(session: &mut Session, ui: &mut UiState, keysym: u64, state: u32) -> KeyEffect {
        let focus = focus_of(session, ui);
        let command = command_for(
            key_from_x11(keysym),
            Modifiers::from_x11_state(state),
            focus,
        );
        apply_key_command(session, ui, command)
    }

    fn type_text(session: &mut Session, ui: &mut UiState, text: &str) {
        for ch in text.chars() {
            let shift = if ch.is_ascii_uppercase() { 1 } else { 0 };
            press(session, ui, ch as u64, shift);
        }
    }

    fn small_session() -> Session {
        Session::new(Viewport {
            width: 320.0,
            height: 240.0,
        })
    }

    #[test]
    fn typing_in_the_address_bar_navigates() {
        let mut session = small_session();
        let mut ui = UiState::new();
        // Control+L focuses the address bar.
        assert_eq!(press(&mut session, &mut ui, 0x6c, 4), KeyEffect::Redraw);
        assert_eq!(focus_of(&session, &ui), Focus::Address);
        type_text(&mut session, &mut ui, "example.org/Path");
        press(&mut session, &mut ui, 0xff08, 0);
        press(&mut session, &mut ui, 0x68, 0);
        let effect = press(&mut session, &mut ui, 0xff0d, 0);
        assert_eq!(
            effect,
            KeyEffect::Navigate("https://example.org/Path".to_string())
        );
        assert_eq!(focus_of(&session, &ui), Focus::Page);
    }

    #[test]
    fn escape_leaves_the_address_bar_before_closing_the_window() {
        let mut session = small_session();
        let mut ui = UiState::new();
        press(&mut session, &mut ui, 0xffc3, 0);
        assert_eq!(focus_of(&session, &ui), Focus::Address);
        assert_eq!(press(&mut session, &mut ui, 0xff1b, 0), KeyEffect::Redraw);
        assert_eq!(focus_of(&session, &ui), Focus::Page);
        // Find open: Escape closes find first.
        press(&mut session, &mut ui, 0x66, 4);
        assert!(ui.is_find_open());
        assert_eq!(press(&mut session, &mut ui, 0xff1b, 0), KeyEffect::Redraw);
        assert!(!ui.is_find_open());
        assert_eq!(
            press(&mut session, &mut ui, 0xff1b, 0),
            KeyEffect::CloseWindow
        );
    }

    #[test]
    fn find_takes_typed_text_and_steps_through_matches() {
        let mut session = small_session();
        let mut ui = UiState::new();
        session.open_html(
            "<p>needle one</p><p>hay</p><p>needle two</p><p>needle three</p>",
            "https://example.com/find",
        );
        press(&mut session, &mut ui, 0x66, 4);
        assert_eq!(focus_of(&session, &ui), Focus::Find);
        type_text(&mut session, &mut ui, "needlx");
        press(&mut session, &mut ui, 0xff08, 0);
        type_text(&mut session, &mut ui, "e");
        assert_eq!(ui.find_query(), "needle");
        assert_eq!(find_match_count(&session, &ui), 3);
        assert_eq!(ui.find_active_match(), 0);
        press(&mut session, &mut ui, 0xff0d, 0);
        assert_eq!(ui.find_active_match(), 1);
        press(&mut session, &mut ui, 0xff8d, 0);
        assert_eq!(ui.find_active_match(), 2);
        // Shift+Enter goes back.
        press(&mut session, &mut ui, 0xff0d, 1);
        assert_eq!(ui.find_active_match(), 1);
        // Control+G and F3 also step.
        press(&mut session, &mut ui, 0x67, 4);
        assert_eq!(ui.find_active_match(), 2);
        press(&mut session, &mut ui, 0xffc0, 0);
        assert_eq!(ui.find_active_match(), 0);
    }

    #[test]
    fn typing_in_a_page_field_fills_and_submits_the_form() {
        let mut session = small_session();
        let mut ui = UiState::new();
        session.open_html(
            "<style>html, body, form, input { margin: 0; }</style>
             <form action=\"https://example.com/login\" method=\"POST\">
               <input id=\"user\" type=\"text\" name=\"user\" value=\"\">
               <button id=\"send\" type=\"submit\">Send</button>
             </form>",
            "https://example.com/index.html",
        );
        assert!(matches!(
            session.click_at(16.0, 10.0),
            ClickOutcome::InputFocused(_)
        ));
        assert_eq!(focus_of(&session, &ui), Focus::PageField);
        type_text(&mut session, &mut ui, "Ann");
        // Keypad digits type digits.
        press(&mut session, &mut ui, 0xffb7, 16);
        // Page fields hold printable ASCII, the same as on Windows, so é is not added.
        assert_eq!(
            press(&mut session, &mut ui, 0x0100_00e9, 0),
            KeyEffect::Nothing
        );
        type_text(&mut session, &mut ui, "x");
        press(&mut session, &mut ui, 0xff08, 0);
        // Control+A is not a shortcut and types nothing.
        assert_eq!(press(&mut session, &mut ui, 0x61, 4), KeyEffect::Nothing);
        let effect = press(&mut session, &mut ui, 0xff0d, 0);
        assert_eq!(
            effect,
            KeyEffect::Post {
                url: "https://example.com/login".to_string(),
                body: "user=Ann7".to_string(),
            }
        );
    }

    #[test]
    fn copy_and_paste_move_text_between_fields() {
        let mut session = small_session();
        let mut ui = UiState::new();
        session.set_clipboard_text("pasted.example");
        press(&mut session, &mut ui, 0x6c, 4);
        // Focusing selects nothing, so clear the current URL first.
        while !ui.input().is_empty() {
            press(&mut session, &mut ui, 0xff08, 0);
        }
        assert_eq!(press(&mut session, &mut ui, 0x76, 4), KeyEffect::Redraw);
        assert_eq!(ui.input(), "pasted.example");
    }

    #[test]
    fn reads_modifier_bits_from_the_x11_state_word() {
        assert_eq!(Modifiers::from_x11_state(0), NONE);
        assert_eq!(Modifiers::from_x11_state(1), SHIFT);
        assert_eq!(Modifiers::from_x11_state(4), CTRL);
        assert_eq!(Modifiers::from_x11_state(8), ALT);
        // Caps Lock (2) and Num Lock (Mod2, 16) are not shortcuts.
        assert_eq!(Modifiers::from_x11_state(2 | 16), NONE);
        let all = Modifiers::from_x11_state(1 | 4 | 8);
        assert!(all.shift && all.ctrl && all.alt);
    }

    #[test]
    fn maps_keysyms_to_keys_and_characters() {
        assert_eq!(key_from_x11(0x61), Key::Char('a'));
        assert_eq!(key_from_x11(0x41), Key::Char('A'));
        assert_eq!(key_from_x11(0x20), Key::Char(' '));
        assert_eq!(key_from_x11(0x7e), Key::Char('~'));
        assert_eq!(key_from_x11(0xe9), Key::Char('é'));
        assert_eq!(key_from_x11(0xdf), Key::Char('ß'));
        assert_eq!(key_from_x11(0x20ac), Key::Char('€'));
        assert_eq!(key_from_x11(0x0100_0416), Key::Char('Ж'));
        assert_eq!(key_from_x11(0x0100_4e2d), Key::Char('中'));
        assert_eq!(key_from_x11(0xffb7), Key::Char('7'));
        assert_eq!(key_from_x11(0xffab), Key::Char('+'));
        assert_eq!(key_from_x11(0xffae), Key::Char('.'));
        assert_eq!(key_from_x11(0xff8d), Key::Enter);
        assert_eq!(key_from_x11(0xff0d), Key::Enter);
        assert_eq!(key_from_x11(0xff08), Key::Backspace);
        assert_eq!(key_from_x11(0xffff), Key::Delete);
        assert_eq!(key_from_x11(0xfe20), Key::Tab);
        assert_eq!(key_from_x11(0xff1b), Key::Escape);
        assert_eq!(key_from_x11(0xff96), Key::Left);
        assert_eq!(key_from_x11(0xff56), Key::PageDown);
        assert_eq!(key_from_x11(0xffbe), Key::F(1));
        assert_eq!(key_from_x11(0xffc2), Key::F(5));
        assert_eq!(key_from_x11(0xffc9), Key::F(12));
        // Shift_L and Control_L are modifiers, not keys.
        assert_eq!(key_from_x11(0xffe1), Key::Other);
        assert_eq!(key_from_x11(0xffe3), Key::Other);
    }

    #[test]
    fn never_types_control_characters_or_bad_code_points() {
        for keysym in 0..0x20u64 {
            assert_eq!(keysym_to_char(keysym), None);
        }
        assert_eq!(keysym_to_char(0x7f), None);
        assert_eq!(keysym_to_char(0x80), None);
        assert_eq!(keysym_to_char(0x9f), None);
        // A Unicode keysym for a control character or a surrogate.
        assert_eq!(keysym_to_char(0x0100_0007), None);
        assert_eq!(keysym_to_char(0x0100_d800), None);
        assert_eq!(keysym_to_char(0x0111_0000), None);
        // Every keysym below 0x10000 maps to a printable character or to nothing.
        for keysym in 0..0x1_0000u64 {
            if let Some(ch) = keysym_to_char(keysym) {
                assert!(!ch.is_control(), "{keysym:#x}");
            }
        }
    }

    #[test]
    fn typing_goes_to_the_focused_field() {
        for focus in [Focus::Address, Focus::Find, Focus::PageField] {
            assert_eq!(
                command_for(Key::Char('q'), NONE, focus),
                KeyCommand::Insert('q')
            );
            assert_eq!(
                command_for(Key::Char('Q'), SHIFT, focus),
                KeyCommand::Insert('Q')
            );
            assert_eq!(
                command_for(Key::Backspace, NONE, focus),
                KeyCommand::DeleteBack
            );
            assert_eq!(command_for(Key::Escape, NONE, focus), KeyCommand::Leave);
        }
        assert_eq!(
            command_for(Key::Char('é'), NONE, Focus::PageField),
            KeyCommand::Insert('é')
        );
        assert_eq!(
            command_for(Key::Enter, NONE, Focus::Address),
            KeyCommand::Submit
        );
        assert_eq!(
            command_for(Key::Tab, NONE, Focus::Address),
            KeyCommand::AcceptSuggestion
        );
        assert_eq!(
            command_for(Key::Enter, NONE, Focus::PageField),
            KeyCommand::Submit
        );
        assert_eq!(
            command_for(Key::Enter, NONE, Focus::Find),
            KeyCommand::FindNext
        );
        assert_eq!(
            command_for(Key::Enter, SHIFT, Focus::Find),
            KeyCommand::FindPrevious
        );
        // Space and slash type in fields instead of scrolling or opening find.
        assert_eq!(
            command_for(Key::Char(' '), NONE, Focus::Find),
            KeyCommand::Insert(' ')
        );
        assert_eq!(
            command_for(Key::Char('/'), NONE, Focus::Address),
            KeyCommand::Insert('/')
        );
        // Backspace in a field deletes. It does not go back.
        assert_ne!(
            command_for(Key::Backspace, NONE, Focus::PageField),
            KeyCommand::Back
        );
    }

    #[test]
    fn shortcuts_work_from_every_focus_and_do_not_type() {
        for focus in [Focus::Address, Focus::Find, Focus::PageField, Focus::Page] {
            assert_eq!(
                command_for(Key::Char('f'), CTRL, focus),
                KeyCommand::OpenFind
            );
            assert_eq!(
                command_for(
                    Key::Char('L'),
                    Modifiers {
                        shift: true,
                        ..CTRL
                    },
                    focus
                ),
                KeyCommand::FocusAddress
            );
            assert_eq!(command_for(Key::Char('t'), CTRL, focus), KeyCommand::NewTab);
            assert_eq!(
                command_for(Key::Char('w'), CTRL, focus),
                KeyCommand::CloseTab
            );
            assert_eq!(command_for(Key::Char('v'), CTRL, focus), KeyCommand::Paste);
            assert_eq!(command_for(Key::Char('='), CTRL, focus), KeyCommand::ZoomIn);
            assert_eq!(
                command_for(Key::Char('-'), CTRL, focus),
                KeyCommand::ZoomOut
            );
            assert_eq!(
                command_for(Key::Char('0'), CTRL, focus),
                KeyCommand::ZoomReset
            );
            assert_eq!(command_for(Key::Left, ALT, focus), KeyCommand::Back);
            assert_eq!(command_for(Key::Right, ALT, focus), KeyCommand::Forward);
            assert_eq!(command_for(Key::F(5), NONE, focus), KeyCommand::Reload);
            assert_eq!(
                command_for(Key::F(6), NONE, focus),
                KeyCommand::FocusAddress
            );
            // An unbound chord types nothing.
            assert_eq!(
                command_for(Key::Char('q'), CTRL, focus),
                KeyCommand::Nothing
            );
            assert_eq!(command_for(Key::Char('q'), ALT, focus), KeyCommand::Nothing);
        }
        assert_eq!(
            command_for(Key::Char('g'), CTRL, Focus::Find),
            KeyCommand::FindNext
        );
        assert_eq!(
            command_for(
                Key::Char('G'),
                Modifiers {
                    shift: true,
                    ..CTRL
                },
                Focus::Find
            ),
            KeyCommand::FindPrevious
        );
        assert_eq!(
            command_for(Key::F(3), SHIFT, Focus::Page),
            KeyCommand::FindPrevious
        );
    }

    #[test]
    fn page_keys_scroll_and_navigate_when_nothing_is_focused() {
        let page = Focus::Page;
        assert_eq!(command_for(Key::Escape, NONE, page), KeyCommand::Dismiss);
        assert_eq!(command_for(Key::Backspace, NONE, page), KeyCommand::Back);
        assert_eq!(command_for(Key::Left, NONE, page), KeyCommand::Back);
        assert_eq!(command_for(Key::Right, NONE, page), KeyCommand::Forward);
        assert_eq!(command_for(Key::Home, NONE, page), KeyCommand::ScrollToTop);
        assert_eq!(
            command_for(Key::End, NONE, page),
            KeyCommand::ScrollToBottom
        );
        assert_eq!(
            command_for(Key::Up, NONE, page),
            KeyCommand::ScrollLines(-1)
        );
        assert_eq!(
            command_for(Key::Down, NONE, page),
            KeyCommand::ScrollLines(1)
        );
        assert_eq!(
            command_for(Key::PageDown, NONE, page),
            KeyCommand::ScrollPages(1)
        );
        assert_eq!(
            command_for(Key::Char(' '), SHIFT, page),
            KeyCommand::ScrollPages(-1)
        );
        assert_eq!(
            command_for(Key::Char('/'), NONE, page),
            KeyCommand::OpenFind
        );
        assert_eq!(command_for(Key::Char('a'), NONE, page), KeyCommand::Nothing);
        assert_eq!(
            command_for(Key::F(12), NONE, page),
            KeyCommand::ToggleInspector
        );
        assert_eq!(command_for(Key::Home, ALT, page), KeyCommand::StartPage);
    }

    #[test]
    fn a_typed_sentence_reaches_a_field_unchanged() {
        // "Hi, café 42!" as an X11 keyboard would send it: keysyms with Shift where needed.
        let presses: [(u64, u32); 12] = [
            (0x48, 1),
            (0x69, 0),
            (0x2c, 0),
            (0x20, 0),
            (0x63, 0),
            (0x61, 0),
            (0x66, 0),
            (0xe9, 0),
            (0x20, 0),
            (0xffb4, 16),
            (0xffb2, 16),
            (0x21, 1),
        ];
        let mut typed = String::new();
        for (keysym, state) in presses {
            let command = command_for(
                key_from_x11(keysym),
                Modifiers::from_x11_state(state),
                Focus::PageField,
            );
            match command {
                KeyCommand::Insert(ch) => typed.push(ch),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(typed, "Hi, café 42!");
    }
}
