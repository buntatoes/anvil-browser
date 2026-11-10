// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Direct ScriptRuntime checks. A script submit records a handshake.
//! The browser navigates after its gate. This runtime does not.

use anvil_dom::{Document, SubmitDispatch, parse_html};
use anvil_script::{DomEvent, EventPhase, ListenerSpec, ScriptError, ScriptRuntime};

fn page(html: &str) -> (Document, ScriptRuntime) {
    let mut doc = parse_html(html);
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("http://127.0.0.1/");
    rt.run_document_scripts(&mut doc);
    (doc, rt)
}

fn text(doc: &Document, id: &str) -> String {
    let node = doc
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    doc.text_content(node)
}

#[test]
fn capture_stop_does_not_reach_the_target() {
    let (mut doc, mut rt) = page(
        "<div id=\"outer\"><button id=\"target\">t</button></div><p id=\"log\"></p>\
         <script>\
           var log = document.getElementById('log');\
           document.getElementById('outer').addEventListener('click', function() {\
             log.textContent = log.textContent + 'cap-stop';\
             event.stopPropagation();\
           }, true);\
           document.getElementById('target').addEventListener('click', function() {\
             log.textContent = log.textContent + 'target';\
           });\
           document.getElementById('outer').addEventListener('click', function() {\
             log.textContent = log.textContent + 'bubble';\
           });\
         </script>",
    );
    let target = doc.get_element_by_id("target").expect("target");
    rt.dispatch_click(&mut doc, target);
    let got = text(&doc, "log");
    assert_eq!(
        got,
        "cap-stop",
        "ancestor capture stopPropagation must not reach the target; log={got:?} prevented={}",
        rt.last_event_default_prevented()
    );
}

#[test]
fn stop_propagation_still_runs_later_listeners_on_the_same_node() {
    let (mut doc, mut rt) = page(
        "<button id=\"b\">t</button><p id=\"log\"></p>\
         <script>\
           var log = document.getElementById('log');\
           var b = document.getElementById('b');\
           b.addEventListener('click', function() {\
             log.textContent = log.textContent + 'one';\
             event.stopPropagation();\
           });\
           b.addEventListener('click', function() {\
             log.textContent = log.textContent + 'two';\
           });\
         </script>",
    );
    let b = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, b);
    let got = text(&doc, "log");
    assert_eq!(
        got, "onetwo",
        "same-node listener after stopPropagation; log={got:?}"
    );
}

#[test]
fn listener_return_false_does_not_cancel_but_inline_return_false_does() {
    let (mut doc, mut rt) = page(
        "<button id=\"inl\" onclick=\"return false\">i</button>\
         <button id=\"lis\">l</button>\
         <script>\
           document.getElementById('lis').addEventListener('click', function() {\
             return false;\
           });\
         </script>",
    );
    let inl = doc.get_element_by_id("inl").expect("inl");
    rt.dispatch_click(&mut doc, inl);
    let inline_cancelled = rt.last_event_default_prevented();
    let lis = doc.get_element_by_id("lis").expect("lis");
    rt.dispatch_click(&mut doc, lis);
    let listener_cancelled = rt.last_event_default_prevented();
    assert!(
        inline_cancelled,
        "inline onclick return false should cancel; listener_cancelled={listener_cancelled}"
    );
    assert!(
        !listener_cancelled,
        "addEventListener return false must not cancel; inline_cancelled={inline_cancelled}"
    );
}

#[test]
fn a_disabled_control_does_not_activate() {
    let (mut doc, mut rt) = page(
        "<button id=\"b\" disabled onclick=\"document.getElementById('log').textContent='go'\">x</button>\
         <p id=\"log\">init</p>",
    );
    let b = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, b);
    let got = text(&doc, "log");
    assert_eq!(got, "init", "disabled click wrote {got:?}");
}

#[test]
fn an_out_of_tree_form_owner_is_serialized_and_strangers_are_not() {
    let (doc, _) = page(
        "<form id=\"main\" action=\"/search\" method=\"POST\">\
           <input name=\"inside\" value=\"a\">\
         </form>\
         <input name=\"linked\" form=\"main\" value=\"b\">\
         <input name=\"loose\" value=\"d\">\
         <form id=\"side\"><input name=\"other\" value=\"c\"></form>",
    );
    let form = doc.get_element_by_id("main").expect("main");
    let (action, method, body) = doc.serialize_form(form);
    assert_eq!(action, "/search", "action");
    assert_eq!(method, "POST", "method");
    assert_eq!(
        body, "inside=a&linked=b",
        "out-of-tree form=main must be included; other form and unassociated controls must not"
    );
}

fn effect_fields(
    form: &Option<(String, String, String, String)>,
) -> Option<(&str, &str, &str, &str)> {
    form.as_ref().map(|(action, method, ctype, body)| {
        (
            action.as_str(),
            method.as_str(),
            ctype.as_str(),
            body.as_str(),
        )
    })
}

#[test]
fn request_submit_get_is_a_handshake_without_navigation() {
    let (doc, mut rt) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\">\
           <input name=\"q\" value=\"anvil\">\
         </form>\
         <script>document.getElementById('g').requestSubmit();</script>",
    );
    let handshake = rt.take_host_submission().expect("GET requestSubmit");
    assert_eq!(handshake.mode, SubmitDispatch::Interactive);
    assert_eq!(handshake.form, doc.get_element_by_id("g").expect("g"));
    assert!(handshake.submitter.is_none());
    assert!(
        rt.take_pending_navigation().is_none(),
        "runtime navigated before the browser gate"
    );
    assert!(rt.take_submitted_form().is_none());
}

#[test]
fn request_submit_post_is_a_handshake_without_navigation() {
    let (doc, mut rt) = page(
        "<form id=\"p\" action=\"/save\" method=\"POST\">\
           <input name=\"q\" value=\"anvil\">\
         </form>\
         <script>document.getElementById('p').requestSubmit();</script>",
    );
    let handshake = rt.take_host_submission().expect("POST requestSubmit");
    assert_eq!(handshake.mode, SubmitDispatch::Interactive);
    assert_eq!(handshake.form, doc.get_element_by_id("p").expect("p"));
    assert!(
        rt.take_pending_navigation().is_none(),
        "runtime navigated before the browser gate"
    );
    assert!(rt.take_submitted_form().is_none());
}

#[test]
fn request_submit_is_interactive_and_submit_is_direct() {
    let (mut doc, mut rt) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\">\
           <input id=\"q\" name=\"q\" value=\"anvil\">\
           <button id=\"go\" type=\"submit\"></button>\
         </form>\
         <script>\
           document.getElementById('g').requestSubmit(document.getElementById('go'));\
         </script>",
    );
    let handshake = rt.take_host_submission().expect("requestSubmit handshake");
    assert_eq!(handshake.mode, SubmitDispatch::Interactive);
    assert_eq!(handshake.form, doc.get_element_by_id("g").expect("g"));
    assert_eq!(
        handshake.submitter,
        Some(doc.get_element_by_id("go").expect("go"))
    );
    assert_eq!(handshake.document, doc.identity());
    assert_eq!(handshake.generation, doc.mutation_generation());
    assert!(handshake.stale_against(&doc).is_none());
    let cloned = doc.clone();
    assert_eq!(
        handshake.stale_against(&cloned),
        Some("wrong document for this page runtime")
    );
    doc.set_input_value(doc.get_element_by_id("q").expect("q"), "later");
    assert_eq!(
        handshake.stale_against(&doc),
        Some("form submission is stale")
    );
    let (_doc, mut direct) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\">\
           <input name=\"q\" required value=\"\">\
         </form>\
         <script>document.getElementById('g').submit();</script>",
    );
    let sent = direct.take_host_submission().expect("submit handshake");
    assert_eq!(sent.mode, SubmitDispatch::Direct);
    assert!(sent.submitter.is_none());
    assert!(
        direct.take_pending_navigation().is_none(),
        "form.submit navigated before the browser gate"
    );
    let (doc, mut again) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\">\
           <input name=\"q\" value=\"anvil\">\
         </form>\
         <script>document.getElementById('g').submit();</script>",
    );
    again.rebind_page_document(&doc);
    assert!(
        again.take_host_submission().is_none(),
        "rebind replayed the previous submit"
    );
}

#[test]
fn request_submit_does_not_navigate_when_the_form_is_invalid() {
    let (_doc, mut rt) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\">\
           <input name=\"q\" required value=\"\">\
         </form>\
         <script>document.getElementById('g').requestSubmit();</script>",
    );
    assert!(
        rt.take_host_submission().is_none(),
        "invalid requestSubmit left a success handshake"
    );
    assert!(rt.take_submission_snapshot().is_none());
    assert!(
        rt.take_pending_navigation().is_none(),
        "invalid requestSubmit navigated; effect={:?}",
        effect_fields(&rt.take_submitted_form())
    );
    let (_doc, mut open) = page(
        "<form id=\"g\" action=\"/search\" method=\"GET\" novalidate>\
           <input name=\"q\" required value=\"\">\
         </form>\
         <script>document.getElementById('g').requestSubmit();</script>",
    );
    let open_handshake = open.take_host_submission().expect("novalidate");
    assert_eq!(open_handshake.mode, SubmitDispatch::Interactive);
    assert!(
        open.take_pending_navigation().is_none(),
        "novalidate requestSubmit navigated before the browser gate"
    );
}

#[test]
fn input_and_keydown_stop_propagation_at_the_current_node() {
    let (mut doc, mut rt) = page(
        "<div id=\"p\" oninput=\"document.getElementById('log').textContent = document.getElementById('log').textContent + 'parent'\"\
             onkeydown=\"document.getElementById('log').textContent = document.getElementById('log').textContent + 'parent'\">\
           <input id=\"c\" oninput=\"document.getElementById('log').textContent = 'child'; event.stopPropagation();\"\
                  onkeydown=\"document.getElementById('log').textContent = 'child'; event.stopPropagation();\">\
         </div>\
         <p id=\"log\">init</p>",
    );
    let child = doc.get_element_by_id("c").expect("c");
    rt.dispatch_input(&mut doc, child, "a");
    assert_eq!(
        text(&doc, "log"),
        "child",
        "input walked past stopPropagation"
    );
    rt.dispatch_keydown(&mut doc, child, "a");
    assert_eq!(
        text(&doc, "log"),
        "child",
        "keydown walked past stopPropagation"
    );
}

#[test]
fn invoke_listener_reports_a_throw_and_does_not_cancel_on_return_false() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let node = doc.get_element_by_id("out").expect("out");
    let id = rt
        .add_listener(
            ListenerSpec {
                node,
                type_name: "click".to_string(),
                capture: false,
                once: true,
            },
            "document.getElementById('out').textContent = 'ran'; return false;",
        )
        .expect("listener");
    let mut event = DomEvent {
        phase: EventPhase::Target,
        type_name: "click".to_string(),
        target: node,
        current_target: node,
        bubbles: true,
        cancelable: true,
        default_prevented: false,
        propagation_stopped: false,
        immediate_stopped: false,
        key: None,
        value: None,
    };
    rt.invoke_listener(&mut doc, id, &mut event)
        .expect("invoke");
    assert_eq!(text(&doc, "out"), "ran");
    assert!(!event.default_prevented);
    assert!(!rt.remove_listener(id), "once listener was kept");
    let thrown = rt
        .add_listener(
            ListenerSpec {
                node,
                type_name: "click".to_string(),
                capture: false,
                once: false,
            },
            "throw 'nope';",
        )
        .expect("throw listener");
    let err = rt
        .invoke_listener(&mut doc, thrown, &mut event)
        .expect_err("throw");
    assert!(matches!(err, ScriptError::Thrown(_)), "{err:?}");
}
