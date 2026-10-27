// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The `anvil://` page key against the ways a web page could try to learn or
//! skip it: scheme case and spacing, script fetches, history and the back
//! list, and a key from another page or run.

use anvil_browser::permissions::action_key;
use anvil_browser::{ClickOutcome, Session, Viewport};
use anvil_net::resolve_url;

fn session() -> Session {
    Session::new(Viewport {
        width: 640.0,
        height: 480.0,
    })
}

const WEB: &str = "https://pages.example/attack.html";

#[test]
fn web_links_never_resolve_to_anvil_addresses_in_any_spelling() {
    for href in [
        "anvil://settings?toggle=https_only",
        "ANVIL://settings?toggle=https_only",
        "Anvil://Settings?toggle=https_only",
        "  anvil://settings?toggle=https_only",
        "anvil:settings?toggle=https_only",
        "anvil:/settings?toggle=https_only",
        "anvil:///settings?toggle=https_only",
        "an\tvil://settings?toggle=https_only",
        "/anvil://settings?toggle=https_only",
    ] {
        if let Some(url) = resolve_url(WEB, href) {
            assert!(
                !url.to_ascii_lowercase().starts_with("anvil:"),
                "{href:?} resolved to {url:?} from a web page"
            );
        }
    }
}

#[test]
fn links_in_any_spelling_change_nothing() {
    let mut session = session();
    let before = session.settings();
    for href in [
        "ANVIL://settings?toggle=https_only",
        "Anvil://Settings?Toggle=Https_Only",
        "anvil://settings?toggle=https_only&key=",
        "anvil://settings?toggle=https_only&key&key=",
    ] {
        let html = format!(r#"<p><a href="{href}">go go go go go go go go</a></p>"#);
        for step in 0..30 {
            session.open_html(&html, WEB);
            if let ClickOutcome::Navigate(target) = session.click_at(20.0, step as f32 * 4.0) {
                session.open_target(&target).unwrap();
            }
        }
        assert_eq!(session.settings(), before, "{href}");
    }
}

#[test]
fn a_script_cannot_fetch_a_built_in_page() {
    let mut session = session();
    session.open_target("anvil://settings").unwrap();
    let html = r#"<p id="out">none</p><script>
        var r = fetch('anvil://settings');
        document.getElementById('out').textContent = 'status ' + r.status + ' ' + r.body;
    </script>"#;
    session.open_html(html, WEB);
    let doc = session.active().current().unwrap().document.to_html();
    assert!(!doc.contains(action_key()), "the key reached a web page");
    assert!(doc.contains("status 403"), "{doc}");
}

#[test]
fn the_key_stays_out_of_history_and_the_back_list() {
    let mut session = session();
    session.open_html("<p>start</p>", WEB);
    for target in [
        format!("anvil://settings?toggle=gpu_video&key={}", action_key()),
        format!("anvil://settings?toggle=gpu_video&key={}", action_key()),
        format!("anvil://home?engine=bing&key={}", action_key()),
        format!(
            "anvil://downloads?action=sample_report&key={}",
            action_key()
        ),
        format!("anvil://history?key={}", action_key()),
    ] {
        session.open_target(&target).unwrap();
        assert!(!session.active().current().unwrap().url.contains("key="));
    }
    assert!(
        session
            .history_log()
            .iter()
            .all(|entry| !entry.url.contains(action_key()) && !entry.title.contains(action_key())),
        "history holds the key"
    );
    while session.back() {
        assert!(
            !session
                .active()
                .current()
                .unwrap()
                .url
                .contains(action_key()),
            "the back list holds the key"
        );
    }
}

#[test]
fn the_key_works_more_than_once_and_only_in_full() {
    let mut session = session();
    let start = session.settings().gpu_video_acceleration;
    let key = action_key();
    for (n, k) in [
        key.to_string(),
        key.to_string(),
        key.to_ascii_uppercase(),
        key[..31].to_string(),
        format!("{key}0"),
        format!(" {key}"),
    ]
    .iter()
    .enumerate()
    {
        session
            .open_target(&format!("anvil://settings?toggle=gpu_video&key={k}"))
            .unwrap();
        // Only the first two (the exact key) flip the switch.
        let flips = (n + 1).min(2);
        let expect = if flips % 2 == 0 { start } else { !start };
        assert_eq!(session.settings().gpu_video_acceleration, expect, "{k:?}");
    }
}
