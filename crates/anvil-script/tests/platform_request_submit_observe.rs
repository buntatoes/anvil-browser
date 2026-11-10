// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Script submit on the integration runtime. Neither `requestSubmit` nor
//! `form.submit` builds a navigation or a payload tuple. The browser does
//! that after its gate, from the current document. The baseline observation
//! that `form.submit` put `q=b` on the query is the bypass this replaces.

use anvil_dom::{SubmitDispatch, parse_html};
use anvil_script::ScriptRuntime;

fn run(source: &str) -> (anvil_dom::Document, ScriptRuntime) {
    let mut doc = parse_html(source);
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("http://127.0.0.1/");
    rt.run_document_scripts(&mut doc);
    (doc, rt)
}

#[test]
fn request_submit_does_not_navigate_when_pattern_fails() {
    let (_doc, mut rt) = run(
        "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" pattern=\"a+\" value=\"b\"></form>\
         <script>document.getElementById('f').requestSubmit();</script>",
    );
    let nav = rt.take_pending_navigation();
    let form = rt.take_submitted_form();
    assert!(
        nav.is_none() && form.is_none(),
        "requestSubmit navigated an invalid pattern; nav={nav:?} submitted={form:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        rt.take_host_submission().is_none(),
        "invalid requestSubmit left a handshake; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn form_submit_records_direct_and_does_not_navigate() {
    let (doc, mut rt) = run(
        "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" pattern=\"a+\" value=\"b\"></form>\
         <script>document.getElementById('f').submit();</script>",
    );
    let nav = rt.take_pending_navigation();
    let form = rt.take_submitted_form();
    assert!(
        nav.is_none() && form.is_none(),
        "form.submit navigated before the browser gate; nav={nav:?} submitted={form:?} logs={:?}",
        rt.console_logs()
    );
    let handshake = rt.take_host_submission().expect("Direct handshake");
    assert_eq!(handshake.mode, SubmitDispatch::Direct);
    assert_eq!(handshake.form, doc.get_element_by_id("f").expect("f"));
    assert!(handshake.submitter.is_none());
}
