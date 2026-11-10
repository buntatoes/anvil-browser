// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! A resource-exhausted classic drops its queued microtask. The next classic
//! still runs. A shape the numeric VM cannot export or limit is selected on
//! the interpreter before that VM starts. A host call followed by a
//! non-limit VM error is not run a second time.

use anvil_dom::parse_html;
use anvil_script::ScriptRuntime;

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    doc.text_content(doc.get_element_by_id(id).expect(id))
}

#[test]
fn anonymous_microtask_on_the_next_classic() {
    let mut doc = parse_html(
        "<p id=\"job\">init-job</p><p id=\"mark\">init-mark</p>\
         <script>\
           queueMicrotask(function() {\
             document.getElementById('job').textContent = 'job';\
           });\
           var i = 0;\
           while (i < 1000000) { i = i + 1; }\
         </script>\
         <script>\
           document.getElementById('mark').textContent = 'next';\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    eprintln!(
        "route-register job={} mark={} charged={} budget={:?}",
        text(&doc, "job"),
        text(&doc, "mark"),
        rt.register_steps_charged(),
        rt.page_budget()
    );
    assert_eq!(text(&doc, "job"), "init-job", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "mark"), "next", "{:?}", rt.console_logs());
    let budget = rt.page_budget();
    assert!(
        budget.lifetime_steps >= budget.step_limit,
        "the spent classic was not charged: {budget:?}"
    );
    assert!(
        budget.steps_used < budget.step_limit,
        "the next classic did not get its own slice: {budget:?}"
    );
}

#[test]
fn interpreter_microtask_on_the_next_classic() {
    // queueMicrotask plus document opens the register heap. The million-step
    // loop spends that turn, so the microtask is dropped. The next classic runs.
    let mut doc = parse_html(
        "<p id=\"job\">init-job</p><p id=\"mark\">init-mark</p>\
         <script>\
           async function ignore() {}\
           queueMicrotask(function() {\
             document.getElementById('job').textContent = 'job';\
           });\
           var i = 0;\
           while (i < 1000000) { i = i + 1; }\
         </script>\
         <script>\
           document.getElementById('mark').textContent = 'next';\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    rt.run_timers(&mut doc);
    eprintln!(
        "route-interpreter job={} mark={} charged={} budget={:?}",
        text(&doc, "job"),
        text(&doc, "mark"),
        rt.register_steps_charged(),
        rt.page_budget()
    );
    assert_eq!(text(&doc, "job"), "init-job", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "mark"), "next");
    assert!(
        rt.register_steps_charged() > 100_000,
        "the exhausted classic did not spend the register turn: {}",
        rt.register_steps_charged()
    );
}

#[test]
fn collection_binding_is_preflighted_once() {
    // The numeric VM cannot export `o`. It must not run the loop and then
    // let the interpreter run it again. One console tick, one result.
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "console.log('once'); let o = {n: 0}; while (o.n < 20) { o.n = o.n + 1; } let total = o.n;",
    );
    let ticks = rt
        .console_logs()
        .iter()
        .filter(|line| line.as_str() == "once")
        .count();
    eprintln!(
        "preflight-object ticks={ticks} err={:?} total={:?} logs={:?}",
        rt.last_vm_error().map(|e| (e.code, e.message.clone())),
        rt.get_var("total"),
        rt.console_logs()
    );
    assert_eq!(
        ticks,
        1,
        "the loop's host call ran twice: {:?}",
        rt.console_logs()
    );
    match rt.get_var("total") {
        Some(anvil_script::Value::Number(n)) => assert_eq!(*n, 20.0),
        other => panic!("the VM number should be visible, got {other:?}"),
    }
    assert!(
        rt.get_var("o").is_none(),
        "object binding leaked: {:?}",
        rt.get_var("o")
    );
    assert!(
        rt.last_vm_error().is_none(),
        "export refusal was left after the number was kept: {:?}",
        rt.last_vm_error()
    );
}

#[test]
fn array_limit_shape_is_preflighted_once() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><script>console.log('arr'); let a = []; a[10000] = 1;</script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let ticks = rt
        .console_logs()
        .iter()
        .filter(|line| line.as_str() == "arr")
        .count();
    eprintln!("preflight-array ticks={ticks} logs={:?}", rt.console_logs());
    assert_eq!(ticks, 1, "{:?}", rt.console_logs());
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("array length limit reached: 10000 elements")),
        "{:?}",
        rt.console_logs()
    );
    assert!(
        rt.console_logs()
            .iter()
            .all(|line| !line.contains("holds an object")),
        "export refusal after a numeric run: {:?}",
        rt.console_logs()
    );
}

#[test]
fn preexec_refusal_falls_through_once() {
    // `notAFunction` is a refusal at code 32 or above, before bytecode.
    // The interpreter runs the script once. `refused()` is every such code,
    // not code 32 alone.
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(&mut doc, "console.log('tick'); let n = 1; notAFunction();");
    let ticks = rt
        .console_logs()
        .iter()
        .filter(|line| line.as_str() == "tick")
        .count();
    let err = rt.last_vm_error().expect("pre-execution refusal");
    eprintln!(
        "preexec code={} refused={} limit={} ticks={ticks} n={:?}",
        err.code,
        err.refused(),
        err.is_limit(),
        rt.get_var("n")
    );
    assert!(err.refused() && err.code >= 32, "{err}");
    assert!(!err.is_limit(), "{err}");
    assert_eq!(ticks, 1, "{:?}", rt.console_logs());
    assert!(
        rt.get_var("n").is_some(),
        "interpreter did not run after the pre-execution refusal"
    );
}

#[test]
fn postexec_error_after_a_host_call_is_not_replayed() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(&mut doc, "console.log('tick'); let n = 1; let f = 1; f();");
    let ticks = rt
        .console_logs()
        .iter()
        .filter(|line| line.as_str() == "tick")
        .count();
    let err = rt.last_vm_error().expect("post-execution error");
    eprintln!(
        "postexec code={} refused={} limit={} ticks={ticks} n={:?} logs={:?}",
        err.code,
        err.refused(),
        err.is_limit(),
        rt.get_var("n"),
        rt.console_logs()
    );
    assert!(!err.refused(), "not a post-execution error: {err}");
    assert!(!err.is_limit(), "{err}");
    assert_eq!(ticks, 1, "host call ran twice: {:?}", rt.console_logs());
    assert!(
        rt.get_var("n").is_none(),
        "interpreter bound n after the VM had already run: {:?}",
        rt.get_var("n")
    );
}

#[test]
fn host_call_then_export_refusal_is_not_replayed() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "console.log('tick'); let s = 'x'; let i = 0; while (i < 12) { s = s + s; i = i + 1; }",
    );
    let ticks = rt
        .console_logs()
        .iter()
        .filter(|line| line.as_str() == "tick")
        .count();
    let err = rt.last_vm_error().expect("export refusal after execution");
    eprintln!(
        "export-after-host code={} limit={} ticks={ticks} s={:?} i={:?}",
        err.code,
        err.is_limit(),
        rt.get_var("s"),
        rt.get_var("i")
    );
    assert_eq!(ticks, 1, "{:?}", rt.console_logs());
    assert_eq!(err.code, 33, "{err}");
    assert!(!err.is_limit(), "{err}");
    assert!(rt.get_var("s").is_none() && rt.get_var("i").is_none());
}
