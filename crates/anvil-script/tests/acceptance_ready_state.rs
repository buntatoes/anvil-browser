// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! `document.readyState` follows the platform on the interpreter and the
//! register host. The runtime does not emit readiness events.

use anvil_dom::{ReadyState, parse_html};
use anvil_script::ScriptRuntime;

fn text(doc: &anvil_dom::Document) -> String {
    let node = doc.get_element_by_id("out").expect("out");
    doc.text_content(node)
}

#[test]
fn ready_state_is_stored_and_read_by_both_dom_vms() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    assert_eq!(rt.ready_state(), ReadyState::Complete);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = document.readyState;",
    );
    assert_eq!(text(&doc), "complete", "{:?}", rt.console_logs());

    rt.set_ready_state(ReadyState::Loading);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = document.readyState;",
    );
    assert_eq!(text(&doc), "loading");

    rt.set_ready_state(ReadyState::Interactive);
    rt.eval_with_cinder_engine(
        &mut doc,
        "document.getElementById('out').textContent = document.readyState;",
    )
    .expect("register read");
    assert_eq!(text(&doc), "interactive", "{:?}", rt.console_logs());

    rt.set_ready_state(ReadyState::Complete);
    rt.eval_with_cinder_engine(
        &mut doc,
        "document.getElementById('out').textContent = document.readyState;",
    )
    .expect("register update");
    assert_eq!(text(&doc), "complete");
    assert!(
        rt.console_logs().iter().all(|line| {
            !line.contains("readystatechange")
                && !line.contains("DOMContentLoaded")
                && !line.contains("load")
        }),
        "runtime emitted a readiness event: {:?}",
        rt.console_logs()
    );

    rt.set_ready_state(ReadyState::Loading);
    rt.rebind_page_document(&doc);
    assert_eq!(rt.ready_state(), ReadyState::Complete);
    rt.eval_with_cinder_engine(
        &mut doc,
        "document.getElementById('out').textContent = document.readyState;",
    )
    .expect("after rebind");
    assert_eq!(text(&doc), "complete");
}
