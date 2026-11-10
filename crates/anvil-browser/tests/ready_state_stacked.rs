// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Page scripts read `document.readyState` on the runtime cumulative.
//!
//! Classic scripts run while the state is `loading`. Deferred scripts run
//! after it becomes `interactive` and before `DOMContentLoaded`. The loader
//! then stores `complete` and dispatches `load`. It does not drain timers:
//! `ScriptRuntime::run_timers` fast-forwards due times. A timer set during
//! load is still pending when load finishes, so this test does not claim a
//! page script read the string `complete`. The runtime's `ready_state()` is
//! `Complete`. A load listener registered during a classic script is queued
//! by the runtime at `addEventListener` time.

use anvil_browser::script_order::run_document_scripts;
use anvil_dom::{ReadyState, ScriptSlot, parse_html};
use anvil_script::ScriptRuntime;

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    let node = doc.get_element_by_id(id).unwrap();
    doc.text_content(node)
}

#[test]
fn a_page_script_reads_ready_state_through_complete() {
    let mut doc = parse_html(
        "<p id=\"early\"></p><p id=\"mid\"></p><p id=\"done\">pending</p>\
         <script>\
           document.getElementById('early').textContent = document.readyState;\
           setTimeout(function() {\
             document.getElementById('done').textContent = 'fired';\
           }, 100000);\
         </script>\
         <script src=\"late.js\" defer></script>",
    );
    let mut runtime = ScriptRuntime::new();
    let fetch = |slot: &ScriptSlot| {
        assert_eq!(slot.src.as_deref(), Some("late.js"));
        Ok(b"document.getElementById('mid').textContent = document.readyState;".to_vec())
    };
    let log = run_document_scripts(
        &mut doc,
        &mut runtime,
        true,
        &fetch,
        |runtime, state| runtime.set_ready_state(state),
        |runtime| runtime.take_submission_snapshot(),
        false,
    );
    assert_eq!(text(&doc, "early"), "loading", "{:?}", log.events);
    assert_eq!(text(&doc, "mid"), "interactive", "{:?}", log.events);
    assert_eq!(
        text(&doc, "done"),
        "pending",
        "100s timer fired during load"
    );
    assert!(
        runtime.has_pending_timers(),
        "the 100s timeout should still be queued"
    );
    assert_eq!(runtime.ready_state(), ReadyState::Complete);
    assert_eq!(
        log.events,
        vec![
            "readystatechange",
            "readystatechange",
            "DOMContentLoaded",
            "readystatechange",
            "load",
        ]
    );
}
