// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! `anvil://settings`, `history`, `downloads`, and `home` links from web pages.
//!
//! A web page can link, submit a form, or run a click handler that points at
//! `anvil://settings?toggle=https_only`. That must show the settings page and
//! change nothing. The same switch on the settings page itself must still work.

use anvil_browser::permissions::action_key;
use anvil_browser::{ClickOutcome, Session, Viewport};

fn session() -> Session {
    Session::new(Viewport {
        width: 640.0,
        height: 480.0,
    })
}

/// Open `html` as if it came from a web site, click down the left side of
/// the page, and follow every navigation the way the window does (anything
/// that isn't http or https goes to `open_target`). Returns the targets.
fn click_through(session: &mut Session, html: &str) -> Vec<String> {
    let mut followed = Vec::new();
    for step in 0..40 {
        session.open_html(html, "https://pages.example/attack.html");
        match session.click_at(20.0, step as f32 * 4.0) {
            ClickOutcome::Navigate(target) => {
                session.open_target(&target).unwrap();
                followed.push(target);
            }
            ClickOutcome::SubmitPost { url, .. } => {
                session.open_target(&url).unwrap();
                followed.push(url);
            }
            _ => {}
        }
    }
    followed
}

/// A link, a GET form, a POST form, and a click handler, all aimed at `target`
/// (an `anvil://` address with a query).
fn attack_pages(target: &str) -> Vec<String> {
    let (page, query) = target.split_once('?').unwrap_or((target, ""));
    let mut hidden = String::new();
    for (k, v) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        hidden.push_str(&format!(r#"<input type="hidden" name="{k}" value="{v}">"#));
    }
    vec![
        format!(r#"<p><a href="{target}">win a prize</a></p>"#),
        format!(
            r#"<form action="{page}" method="GET">{hidden}<input type="submit" value="Continue"></form>"#
        ),
        format!(
            r#"<form action="{}" method="GET">{hidden}<input type="submit" value="Continue"></form>"#,
            page.to_ascii_uppercase()
        ),
        format!(
            r#"<form action="{page}" method="POST">{hidden}<input type="submit" value="Continue"></form>"#
        ),
        format!(
            r#"<div id="b">click me please</div><script>document.getElementById('b').addEventListener('click', function () {{ location.href = '{target}'; }});</script>"#
        ),
    ]
}

/// The `href` of the first link on the active page whose address contains `needle`.
fn link_on_active_page(session: &Session, needle: &str) -> String {
    let html = session.active().current().unwrap().document.to_html();
    for piece in html.split("href=\"").skip(1) {
        let href = piece.split('"').next().unwrap_or("").replace("&amp;", "&");
        if href.contains(needle) {
            return href;
        }
    }
    panic!("no link with {needle} on the page");
}

#[test]
fn a_web_page_link_cannot_turn_https_only_off() {
    let mut session = session();
    let toggle = link_on_active_page_after_settings(&mut session, "toggle=https_only");
    session.open_target(&toggle).unwrap();
    assert!(
        session.settings().https_only_mode,
        "settings page switch works"
    );

    for html in attack_pages("anvil://settings?toggle=https_only") {
        click_through(&mut session, &html);
        assert!(
            session.settings().https_only_mode,
            "a web page must not turn HTTPS-only off: {html}"
        );
    }
}

#[test]
fn a_form_on_a_web_page_cannot_submit_to_anvil_pages() {
    let mut session = session();
    let html = r#"<form action="anvil://settings" method="GET"><input type="hidden" name="toggle" value="https_only"><input type="submit" value="Continue"></form>"#;
    assert!(
        click_through(&mut session, html).is_empty(),
        "the form action must not resolve from a web page"
    );
}

#[test]
fn a_missing_or_wrong_key_changes_nothing_even_if_the_address_arrives() {
    let mut session = session();
    let before = session.settings();
    for target in [
        "anvil://settings?toggle=https_only",
        "ANVIL://SETTINGS?TOGGLE=HTTPS_ONLY",
        "anvil://settings?x=1&toggle=scripts",
        "anvil://settings?xtoggle=jit",
        "anvil://settings?toggle=gpu_video&key=",
        "anvil://settings?toggle=gpu_video&key=00000000000000000000000000000000",
        "anvil://settings?engine=bing",
        "anvil://settings?action=clear_profile",
        "anvil://home?engine=bing",
    ] {
        session.open_target(target).unwrap();
        assert_eq!(session.settings(), before, "{target}");
        assert!(
            !session.active().current().unwrap().url.contains("key="),
            "{target}"
        );
    }
}

fn link_on_active_page_after_settings(session: &mut Session, needle: &str) -> String {
    session.open_target("anvil://settings").unwrap();
    link_on_active_page(session, needle)
}

#[test]
fn web_pages_cannot_flip_any_other_switch() {
    let mut session = session();
    let before = session.settings();
    for target in [
        "anvil://settings?toggle=scripts",
        "anvil://settings?toggle=jit",
        "anvil://settings?toggle=gpu_video",
        "anvil://settings?toggle=search_engine",
        "anvil://settings?engine=bing",
        "anvil://home?engine=duckduckgo",
    ] {
        for html in attack_pages(target) {
            click_through(&mut session, &html);
            assert_eq!(session.settings(), before, "{html}");
        }
    }
}

#[test]
fn a_web_page_link_cannot_clear_history_or_downloads() {
    let mut session = session();
    session.open_html("<p>one</p>", "https://pages.example/one.html");
    session
        .open_target(&format!(
            "anvil://downloads?action=sample_report&key={}",
            action_key()
        ))
        .unwrap();
    assert_eq!(session.downloads().len(), 1);
    let history = session.history_log().len();
    assert!(history > 0);
    for target in [
        "anvil://history?action=clear",
        "anvil://settings?action=clear_profile",
        "anvil://downloads?action=clear",
    ] {
        for html in attack_pages(target) {
            click_through(&mut session, &html);
        }
    }
    assert!(session.history_log().len() >= history, "history kept");
    assert_eq!(session.downloads().len(), 1, "downloads kept");

    session.open_target("anvil://history").unwrap();
    let clear = link_on_active_page(&session, "action=clear");
    session.open_target(&clear).unwrap();
    assert!(
        session.history_log().is_empty(),
        "the history page's own link works"
    );
}

#[test]
fn the_key_never_shows_in_the_address() {
    let mut session = session();
    let toggle = link_on_active_page_after_settings(&mut session, "toggle=scripts");
    session.open_target(&toggle).unwrap();
    assert_eq!(session.active().current().unwrap().url, "anvil://settings");
    assert!(
        session
            .history_log()
            .iter()
            .all(|entry| !entry.url.contains("key=")),
        "history never stores a keyed address"
    );
}
