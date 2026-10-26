// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! The key exchange section of anvil://security.

use anvil_browser::frontend::{
    key_exchange_section_html, security_html, security_html_with_key_exchanges,
};
use anvil_browser::{Viewport, load_target};
use anvil_net::KeyExchange;

fn entry(host: &str, group: &str) -> KeyExchange {
    KeyExchange {
        host: host.to_string(),
        group: group.to_string(),
    }
}

#[test]
fn says_post_quantum_only_for_ml_kem_groups() {
    let html = key_exchange_section_html(&[
        entry("pq.example", "X25519MLKEM768"),
        entry("classic.example", "x25519"),
        entry("p256.example", "secp256r1"),
    ]);
    let rows: Vec<&str> = html.split("<tr>").skip(2).collect();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].contains("pq.example") && rows[0].contains("Yes, hybrid with ML-KEM"));
    assert!(rows[1].contains("classic.example") && rows[1].contains("No, classical only"));
    assert!(rows[2].contains("secp256r1") && rows[2].contains("No, classical only"));
}

#[test]
fn an_empty_list_makes_no_claim() {
    let html = key_exchange_section_html(&[]);
    assert!(html.contains("No HTTPS connections from this window yet."));
    assert!(!html.contains("Yes, hybrid"));
    assert!(!html.contains("<table"));
}

#[test]
fn host_and_group_are_escaped() {
    let html = key_exchange_section_html(&[entry("<b>x</b>", "a&b")]);
    assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"));
    assert!(html.contains("a&amp;b"));
    assert!(!html.contains("<b>x</b>"));
}

#[test]
fn the_security_page_has_the_section() {
    let page = security_html_with_key_exchanges(&[entry("pq.example", "X25519MLKEM768")]);
    assert!(page.contains("HTTPS key exchange"));
    assert!(page.contains("key-exchange-table"));
    assert!(!page.contains("KEY_EXCHANGE_SLOT"));
    if cfg!(target_os = "linux") {
        assert!(page.contains("X25519MLKEM768 first"));
    }
    assert!(!security_html().contains("KEY_EXCHANGE_SLOT"));
    let loaded = load_target(
        "anvil://security",
        Viewport {
            width: 800.0,
            height: 600.0,
        },
    )
    .expect("anvil://security loads");
    assert_eq!(loaded.url, "anvil://security");
}
