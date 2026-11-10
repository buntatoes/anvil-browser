// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! A nested entry must not refill the same turn. A later host call is a
//! separate turn and may run. Register execution is charged on that VM's
//! own step counter.

use anvil_dom::parse_html;
use anvil_script::ScriptRuntime;

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    let node = doc.get_element_by_id(id).expect(id);
    doc.text_content(node)
}

#[test]
fn a_nested_click_does_not_refill_the_same_turn() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"btn\"></button>\
         <script>\
           document.getElementById('btn').addEventListener('click', function() {\
             document.getElementById('out').textContent = 'nested';\
           });\
           var i = 0;\
           while (i < 200) { i = i + 1; }\
           document.getElementById('btn').click();\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let budget = rt.page_budget();
    let out = text(&doc, "out");
    assert_eq!(
        out, "nested",
        "click handler did not run; budget={budget:?}"
    );
    assert!(
        budget.steps_used > 200,
        "nested click refilled this turn; budget={budget:?} logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn a_later_host_eval_may_run_after_the_turn_stops() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <script>let i = 0; while (i < 1000000) { i = i + 1; }</script>\
         <script>let j = 0; while (j < 1000000) { j = j + 1; }</script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let spent = rt.page_budget();
    assert!(
        spent.steps_used >= spent.step_limit,
        "page turn was not spent before the later host call; budget={spent:?} logs={:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'turn2';",
    );
    let after = rt.page_budget();
    assert_eq!(
        text(&doc, "out"),
        "turn2",
        "later host eval_script did not run; spent={spent:?} after={after:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        after.lifetime_steps >= spent.step_limit,
        "the spent turn was not kept in the lifetime count; spent={spent:?} after={after:?}"
    );
}

#[test]
fn a_script_executed_on_a_second_document_with_the_same_root_slot() {
    let mut first = parse_html(
        "<p id=\"out\">A</p><script>var n = 3; document.getElementById('out').textContent = 'from-a';</script>",
    );
    let mut second = parse_html("<p id=\"out\">B</p>");
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut first);
    let bound = rt.bound_document_root();
    let first_slot = first.root().index();
    let second_slot = second.root().index();
    let session_before = rt.page_session().expect("session").session_id;
    rt.eval_script(
        &mut second,
        "document.getElementById('out').textContent = 'b:' + n;",
    );
    let a = text(&first, "out");
    let b = text(&second, "out");
    assert_eq!(
        first_slot, second_slot,
        "premise failed: roots differ, so this is not the same-slot case"
    );
    assert_eq!(a, "from-a", "document A changed; B={b:?} bound={bound:?}");
    assert_eq!(
        b,
        "B",
        "unrelated document B was written with A's global; A={a:?} bound={bound:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("wrong document")),
        "missing rejection; logs={:?}",
        rt.console_logs()
    );
    assert_eq!(
        rt.page_session().expect("session").session_id,
        session_before,
        "a rejected document must not retarget the session"
    );
    assert_ne!(
        first.identity(),
        second.identity(),
        "the two documents were not given distinct identities"
    );
    assert_eq!(
        rt.page_session().expect("session").document_id,
        first.identity()
    );
    rt.rebind_page_document(&second);
    assert_ne!(
        rt.page_session().expect("session").session_id,
        session_before
    );
    rt.eval_script(
        &mut second,
        "document.getElementById('out').textContent = 'fresh';",
    );
    assert_eq!(text(&second, "out"), "fresh");
    assert_eq!(text(&first, "out"), "from-a");
    assert!(
        rt.get_var("n").is_none(),
        "rebind kept A's global {:?}",
        rt.get_var("n")
    );
}

#[test]
fn release_keeps_an_interpreter_global() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(&mut doc, "var n = 7;");
    rt.release_page_callbacks();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = '' + n;",
    );
    assert_eq!(
        text(&doc, "out"),
        "7",
        "release_page_callbacks dropped a global the comment says it keeps; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn a_register_script_charges_steps_from_that_vm() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "\"use cinder\"; var i = 0; while (i < 400) { i = i + 1; } document.getElementById('out').textContent = '' + i;",
    );
    assert_eq!(
        text(&doc, "out"),
        "400",
        "register script did not finish; logs={:?} charged={}",
        rt.console_logs(),
        rt.register_steps_charged()
    );
    assert!(
        rt.register_steps_charged() > 50,
        "register VM did not charge its own steps; budget={:?} charged={}",
        rt.page_budget(),
        rt.register_steps_charged()
    );
}

#[test]
fn release_drops_register_listeners_and_timers_but_keeps_the_global() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"btn\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "\"use cinder\"; var kept = 9;\
         function tick(){ document.getElementById('out').textContent = 'timer'; }\
         setTimeout(tick, 1000000);\
         function onClick(){ document.getElementById('out').textContent = 'click'; }\
         document.getElementById('btn').addEventListener('click', onClick);",
    );
    assert!(
        rt.register_steps_charged() > 0,
        "register script did not run; logs={:?}",
        rt.console_logs()
    );
    assert!(
        rt.has_pending_timers(),
        "register timer was not scheduled; listeners={} logs={:?}",
        rt.register_listener_count(),
        rt.console_logs()
    );
    assert!(
        rt.register_listener_count() > 0,
        "register listener was not stored"
    );
    rt.release_page_callbacks();
    assert!(!rt.has_pending_timers(), "release kept a timer");
    assert_eq!(rt.register_listener_count(), 0, "release kept a listener");
    rt.eval_script(
        &mut doc,
        "\"use cinder\"; document.getElementById('out').textContent = '' + kept;",
    );
    assert_eq!(
        text(&doc, "out"),
        "9",
        "register global dropped; logs={:?}",
        rt.console_logs()
    );
    rt.tick_timers(&mut doc, 2_000_000.0);
    assert_eq!(text(&doc, "out"), "9", "released timer still fired");
    let btn = doc.get_element_by_id("btn").expect("btn");
    rt.dispatch_click(&mut doc, btn);
    assert_eq!(text(&doc, "out"), "9", "released listener still fired");
}

#[test]
fn compile_heap_refusal_does_not_fall_through() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_numeric_heap_budget(128);
    rt.eval_script(&mut doc, "function f(){ return 1; } let marker = 4;");
    let err = rt.last_vm_error().expect("vm error");
    assert_eq!(err.code, 50, "{err}");
    assert!(err.is_limit(), "{err}");
    assert!(
        rt.get_var("marker").is_none(),
        "heap refusal reran on the interpreter: {:?}",
        rt.get_var("marker")
    );
    assert_eq!(text(&doc, "out"), "init");
}

#[test]
fn a_stopped_classic_does_not_block_the_next_classic() {
    // Independent classics each get a slice. The 1_000_000 loop spends
    // its own 200_000 and does not finish. The marker script is a new
    // task and runs. Nested calls do not get this reset.
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <script>let i = 0; while (i < 40) { i = i + 1; }</script>\
         <script>let j = 0; while (j < 1000000) { j = j + 1; }</script>\
         <script>let marker = 1; document.getElementById('out').textContent = 'marker';</script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "marker");
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("step limit reached: 200000 steps")),
        "{:?}",
        rt.console_logs()
    );
    match rt.get_var("j") {
        Some(anvil_script::Value::Number(n)) => assert!(*n < 1_000_000.0, "{n}"),
        other => assert!(other.is_none(), "{other:?}"),
    }
}

#[test]
fn a_cloned_document_is_not_the_bound_page() {
    let mut doc = parse_html("<p id=\"out\">A</p><script>var n = 1;</script>");
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let mut cloned = doc.clone();
    assert_ne!(
        doc.identity(),
        cloned.identity(),
        "clone kept the source page identity"
    );
    rt.eval_script(
        &mut cloned,
        "document.getElementById('out').textContent = 'cloned';",
    );
    assert_eq!(text(&doc, "out"), "A");
    assert_eq!(text(&cloned, "out"), "A");
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("wrong document")),
        "clone was accepted; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn a_full_timer_queue_does_not_report_success() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "function tick() {}\
         var last = 1;\
         var i = 0;\
         while (i < 130) {\
           last = setTimeout(tick, 1000000);\
           i = i + 1;\
         }\
         document.getElementById('out').textContent = '' + last;",
    );
    assert_eq!(text(&doc, "out"), "0", "{:?}", rt.console_logs());
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("timer queue full")),
        "full timer queue was silent; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn anonymous_click_listeners_run_on_the_register_host() {
    // Unique addEventListener callbacks are scanned on every registration,
    // so a few hundred of them spend the 200_000 slice before the click.
    // Twenty fit. The host cap of 1024 is js_listener_and_microtask_caps_fail_visibly.
    // The old interpreter vector stopped at 256; these listeners are not on it.
    let mut doc = parse_html("<button id=\"b\"></button><p id=\"out\"></p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var i = 0; var el = document.getElementById('b');\
         while (i < 20) {\
           el.addEventListener('click', function(){\
             var n = document.getElementById('out');\
             n.textContent = n.textContent + 'x';\
           });\
           i = i + 1;\
         }",
    );
    let b = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, b);
    assert_eq!(text(&doc, "out").len(), 20, "{:?}", rt.console_logs());
    assert!(rt.register_steps_charged() > 0, "{:?}", rt.console_logs());
}

#[test]
fn a_module_script_is_an_explicit_unsupported_error() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><script type=\"module\">document.getElementById('out').textContent = 'mod';</script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "init");
    assert_eq!(rt.last_unsupported(), Some("modules are not supported"));
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("modules are not supported")),
        "{:?}",
        rt.console_logs()
    );
}
