// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Where a typed submission is consumed, and the order requestSubmit uses.

use anvil_browser::{ClickOutcome, Session, Viewport};
use anvil_dom::parse_html;
use anvil_script::ScriptRuntime;

fn page() -> Session {
    Session::new(Viewport {
        width: 320.0,
        height: 240.0,
    })
}

/// Pending until `ScriptRuntime` calls `form_submit::run_submit_gate` inside
/// the `requestSubmit` turn. The runtime records a handshake and returns
/// before script listeners run, so the page log is `after;` today. Do not
/// treat an end-of-task walk as this order.
#[test]
fn request_submit_runs_submit_before_the_following_statement() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>\
         <script>\
           var log = document.getElementById('log');\
           document.getElementById('f').addEventListener('submit', function() {\
             log.textContent += 'submit;';\
           });\
           document.getElementById('f').requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let shown = doc.text_content(doc.get_element_by_id("log").unwrap());
    assert_eq!(shown, "submit;after;", "{:?}", runtime.console_logs());
    assert!(runtime.take_submission_snapshot().is_some());
}

/// `invalid` is fired at the control and does not bubble. Expected log is
/// `invalid;after;`, with no snapshot.
#[test]
fn an_invalid_request_submit_runs_invalid_before_the_following_statement() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"POST\">\
           <input id=\"q\" name=\"q\" pattern=\"a+\" value=\"b\">\
         </form>\
         <script>\
           var log = document.getElementById('log');\
           document.getElementById('q').addEventListener('invalid', function() {\
             log.textContent += 'invalid;';\
           });\
           document.getElementById('f').addEventListener('submit', function() {\
             log.textContent += 'submit;';\
           });\
           document.getElementById('f').requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let shown = doc.text_content(doc.get_element_by_id("log").unwrap());
    assert_eq!(shown, "invalid;after;");
    assert!(runtime.take_submission_snapshot().is_none());
}

/// Pending for the same missing host callback. `preventDefault` means no handshake.
#[test]
fn a_submit_handler_prevent_default_runs_before_the_following_statement() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>\
         <script>\
           var log = document.getElementById('log');\
           document.getElementById('f').addEventListener('submit', function(ev) {\
             ev.preventDefault();\
             log.textContent += 'submit;';\
           });\
           document.getElementById('f').requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let shown = doc.text_content(doc.get_element_by_id("log").unwrap());
    assert_eq!(shown, "submit;after;");
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn a_microtask_submit_is_consumed_by_the_loader() {
    let mut session = page();
    session.open_html(
        "<form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"micro\"></form>\
         <script>queueMicrotask(function() { document.getElementById('f').requestSubmit(); });</script>",
        "http://127.0.0.1/page",
    );
    let saved = session
        .active()
        .current()
        .unwrap()
        .script_submission
        .clone()
        .expect("microtask submission");
    assert!(saved.url.contains("q=micro"), "{}", saved.url);
}

#[test]
fn a_timer_submit_stays_pending_after_load() {
    let mut session = page();
    session.open_html(
        "<p id=\"out\">init</p>\
         <form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"anvil\"></form>\
         <script>\
           setTimeout(function() {\
             document.getElementById('out').textContent = 'fired';\
             document.getElementById('f').requestSubmit();\
           }, 100000);\
         </script>",
        "http://127.0.0.1/page",
    );
    let page = session.active().current().unwrap();
    let out = page.document.get_element_by_id("out").unwrap();
    assert_eq!(page.document.text_content(out), "init");
    assert!(page.script_submission.is_none());
    assert!(page.runtime.has_pending_timers());
}

#[test]
fn input_is_handled_while_a_future_timer_is_still_pending() {
    let mut session = page();
    session.open_html(
        "<p id=\"tick\">waiting</p>\
         <form id=\"f\" action=\"/save\" method=\"GET\"><input id=\"q\" name=\"q\" value=\"\"></form>\
         <script>\
           setTimeout(function() { document.getElementById('tick').textContent = 'fired'; }, 100000);\
           document.getElementById('q').addEventListener('input', function() {\
             document.getElementById('f').requestSubmit();\
           });\
         </script>",
        "http://127.0.0.1/page",
    );
    let id = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("q")
        .unwrap();
    assert!(session.focus_text_input(id));
    assert!(session.type_into_focused_input('a'));
    let page = session.active().current().unwrap();
    let tick = page.document.get_element_by_id("tick").unwrap();
    assert_eq!(page.document.text_content(tick), "waiting");
    assert_eq!(page.document.input_value(id), "a");
    let saved = page
        .script_submission
        .clone()
        .expect("input submission consumed after the default action");
    assert!(saved.url.contains("q=a"), "{}", saved.url);
    assert!(page.runtime.has_pending_timers());
}

#[test]
fn an_input_handler_submit_is_consumed() {
    let mut session = page();
    session.open_html(
        "<form id=\"f\" action=\"/save\" method=\"GET\"><input id=\"q\" name=\"q\" value=\"\"></form>\
         <script>\
           document.getElementById('q').addEventListener('input', function() {\
             document.getElementById('f').requestSubmit();\
           });\
         </script>",
        "http://127.0.0.1/page",
    );
    let id = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("q")
        .unwrap();
    assert!(session.focus_text_input(id));
    assert!(session.type_into_focused_input('a'));
    let page = session.active().current().unwrap();
    let saved = page.script_submission.clone().expect("input submission");
    assert!(saved.url.contains("q="), "{}", saved.url);
    let _ = id;
}

#[test]
fn a_keydown_handler_submit_is_consumed() {
    let mut session = page();
    session.open_html(
        "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"q\" name=\"q\" value=\"z\"></form>\
         <script>\
           document.getElementById('q').addEventListener('keydown', function() {\
             document.getElementById('f').requestSubmit();\
           });\
         </script>",
        "http://127.0.0.1/page",
    );
    let id = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("q")
        .unwrap();
    assert!(session.focus_text_input(id));
    let outcome = session.submit_focused_input();
    assert!(
        matches!(
            outcome,
            ClickOutcome::SubmitPost { .. } | ClickOutcome::Navigate(_)
        ),
        "{outcome:?} focused {id:?}"
    );
}

#[test]
fn an_invalid_request_submit_produces_no_navigation() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"GET\">\
           <input id=\"q\" name=\"q\" required value=\"\">\
         </form>\
         <script>\
           document.getElementById('q').addEventListener('invalid', function() {\
             document.getElementById('log').textContent += 'invalid;';\
           });\
           document.getElementById('f').requestSubmit();\
           document.getElementById('log').textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "invalid;after;"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn a_non_submit_button_throws_type_error_and_does_not_navigate() {
    let mut doc = parse_html(
        "<p id=\"log\">init</p>\
         <form id=\"f\" action=\"/save\"><input id=\"q\" name=\"q\" value=\"a\"></form>\
         <script>\
           try {\
             document.getElementById('f').requestSubmit(document.getElementById('q'));\
             document.getElementById('log').textContent = 'accepted';\
           } catch (e) {\
             document.getElementById('log').textContent = e.name;\
           }\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "TypeError"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn a_second_valid_submit_replaces_the_planned_snapshot() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"http://127.0.0.1/save\" method=\"GET\">\
           <input id=\"q\" name=\"q\" value=\"a\">\
         </form>\
         <script>\
           var f = document.getElementById('f');\
           f.requestSubmit();\
           document.getElementById('q').value = 'b';\
           f.requestSubmit();\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime
        .take_submission_snapshot()
        .expect("replaced snapshot");
    assert_eq!(text_values(&snapshot, "q"), vec!["b"]);
}

#[test]
fn an_invalid_submit_keeps_the_planned_snapshot() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"http://127.0.0.1/save\" method=\"GET\">\
           <input id=\"q\" name=\"q\" pattern=\"a+\" value=\"a\">\
         </form>\
         <script>\
           var f = document.getElementById('f');\
           f.requestSubmit();\
           document.getElementById('q').value = '!!!';\
           f.requestSubmit();\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime
        .take_submission_snapshot()
        .expect("planned snapshot kept");
    assert_eq!(text_values(&snapshot, "q"), vec!["a"]);
}

#[test]
fn a_cancelled_submit_keeps_the_planned_snapshot() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"http://127.0.0.1/save\" method=\"GET\">\
           <input id=\"q\" name=\"q\" value=\"a\">\
         </form>\
         <script>\
           var f = document.getElementById('f');\
           var n = 0;\
           f.addEventListener('submit', function(ev) {\
             n += 1;\
             if (n === 2) ev.preventDefault();\
           });\
           f.requestSubmit();\
           document.getElementById('q').value = 'b';\
           f.requestSubmit();\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime
        .take_submission_snapshot()
        .expect("planned snapshot kept");
    assert_eq!(text_values(&snapshot, "q"), vec!["a"]);
}

#[test]
fn a_nested_request_submit_is_one_submission() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"http://127.0.0.1/save\" method=\"GET\">\
           <input id=\"q\" name=\"q\" value=\"a\">\
         </form>\
         <script>\
           var log = document.getElementById('log');\
           var f = document.getElementById('f');\
           f.addEventListener('submit', function() {\
             log.textContent += 'submit;';\
             f.requestSubmit();\
           });\
           f.requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "submit;after;"
    );
    assert!(runtime.take_submission_snapshot().is_some());
}

fn text_values(snapshot: &anvil_dom::SubmissionSnapshot, name: &str) -> Vec<String> {
    snapshot
        .entries
        .iter()
        .filter_map(|entry| match entry {
            anvil_dom::SnapshotEntry::Text {
                name: entry_name,
                value,
            } if entry_name == name => Some(value.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn direct_submit_captures_an_invalid_control_once() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"http://127.0.0.1/save\" method=\"GET\">\
           <input name=\"q\" pattern=\"a+\" value=\"b\">\
         </form>\
         <script>document.getElementById('f').submit();</script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime
        .take_submission_snapshot()
        .expect("direct submit snapshot");
    assert_eq!(snapshot.method, anvil_dom::SnapshotMethod::Get);
    let values: Vec<_> = snapshot
        .entries
        .iter()
        .filter_map(|entry| match entry {
            anvil_dom::SnapshotEntry::Text { name, value } if name == "q" => Some(value.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec!["b"]);
}

#[test]
fn a_foreign_submitter_throws_not_found_and_does_not_navigate() {
    let mut doc = parse_html(
        "<p id=\"log\">init</p>\
         <form id=\"f\" action=\"/save\"><input name=\"q\" value=\"a\"></form>\
         <form id=\"g\"><button id=\"other\" type=\"submit\">x</button></form>\
         <script>\
           try {\
             document.getElementById('f').requestSubmit(document.getElementById('other'));\
             document.getElementById('log').textContent = 'accepted';\
           } catch (e) {\
             document.getElementById('log').textContent = e.name;\
           }\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "NotFoundError"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn a_sync_submit_edit_is_captured_and_a_microtask_edit_is_not() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"q\" name=\"q\" value=\"old\"></form>\
         <script>\
           document.getElementById('f').addEventListener('submit', function() {\
             document.getElementById('q').value = 'from-submit';\
             queueMicrotask(function() {\
               document.getElementById('q').value = 'from-micro';\
             });\
           });\
           document.getElementById('f').requestSubmit();\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime
        .take_submission_snapshot()
        .expect("snapshot before the microtask checkpoint");
    runtime.checkpoint_microtasks(&mut doc);
    let body = format!("{:?}", snapshot.entries);
    assert!(body.contains("from-submit"), "{body}");
    assert!(!body.contains("from-micro"), "{body}");
}

#[test]
fn removing_the_form_after_the_snapshot_still_navigates() {
    let mut session = page();
    session.open_html(
        "<form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"kept\"></form>\
         <script>\
           var f = document.getElementById('f');\
           f.requestSubmit();\
           queueMicrotask(function() { f.remove(); });\
         </script>",
        "http://127.0.0.1/page",
    );
    let page = session.active().current().unwrap();
    assert!(page.document.get_element_by_id("f").is_none());
    let saved = page
        .script_submission
        .clone()
        .expect("snapshot consumed after the form was removed");
    assert!(saved.url.contains("q=kept"), "{}", saved.url);
    assert!(saved.url.contains("/save"), "{}", saved.url);
}

#[test]
fn a_detached_form_submit_and_request_submit_do_nothing() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"a\"></form>\
         <script>\
           var log = document.getElementById('log');\
           var f = document.getElementById('f');\
           f.remove();\
           f.addEventListener('submit', function() { log.textContent += 'submit;'; });\
           f.requestSubmit();\
           f.submit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "after;"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn detach_inside_submit_fires_once_and_does_not_snapshot() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"a\"></form>\
         <script>\
           var log = document.getElementById('log');\
           var f = document.getElementById('f');\
           f.addEventListener('submit', function() {\
             log.textContent += 'submit;';\
             f.remove();\
           });\
           f.requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "submit;after;"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn detach_inside_submit_keeps_the_earlier_plan() {
    let mut doc = parse_html(
        "<form id=\"f\" action=\"/one\" method=\"GET\"><input id=\"q\" name=\"q\" value=\"a\"></form>\
         <script>\
           var f = document.getElementById('f');\
           var n = 0;\
           f.addEventListener('submit', function() {\
             n += 1;\
             if (n === 2) {\
               document.getElementById('q').value = 'replaced';\
               f.remove();\
             }\
           });\
           f.requestSubmit();\
           f.requestSubmit();\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let snapshot = runtime.take_submission_snapshot().expect("earlier plan");
    let body = format!("{:?}", snapshot.entries);
    assert!(body.contains("value: \"a\""), "{body}");
    assert!(
        !body.contains("replaced"),
        "detached submit replaced the plan: {body}"
    );
}

#[test]
fn detach_inside_formdata_does_not_plan() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>\
         <script>\
           var log = document.getElementById('log');\
           var f = document.getElementById('f');\
           f.addEventListener('formdata', function() {\
             log.textContent += 'formdata;';\
             f.remove();\
           });\
           f.addEventListener('submit', function() { log.textContent += 'submit;'; });\
           f.requestSubmit();\
           log.textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let source = doc.scripts().into_iter().next().unwrap();
    runtime.eval_script(&mut doc, &source);
    let shown = doc.text_content(doc.get_element_by_id("log").unwrap());
    assert_eq!(shown, "submit;formdata;after;", "{shown}");
    assert!(runtime.take_submission_snapshot().is_none());
}

#[test]
fn an_inactive_document_submit_does_nothing() {
    let mut doc = parse_html(
        "<p id=\"log\"></p>\
         <form id=\"f\" action=\"/save\" method=\"GET\"><input name=\"q\" value=\"a\"></form>\
         <script id=\"bind\">\
           document.getElementById('f').addEventListener('submit', function() {\
             document.getElementById('log').textContent += 'submit;';\
           });\
         </script>\
         <script id=\"go\">\
           document.getElementById('f').requestSubmit();\
           document.getElementById('f').submit();\
           document.getElementById('log').textContent += 'after;';\
         </script>",
    );
    let mut runtime = ScriptRuntime::new();
    runtime.set_page_origin("http://127.0.0.1/page");
    let scripts = doc.scripts();
    runtime.eval_script(&mut doc, &scripts[0]);
    runtime.release_page_callbacks();
    runtime.eval_script(&mut doc, &scripts[1]);
    assert_eq!(
        doc.text_content(doc.get_element_by_id("log").unwrap()),
        "after;"
    );
    assert!(runtime.take_submission_snapshot().is_none());
}
