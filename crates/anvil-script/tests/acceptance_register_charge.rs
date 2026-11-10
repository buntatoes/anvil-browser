// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Bounded checks of register microtasks and the numeric page helper.
//! These assert the turn contract. A failure is the observation.

use anvil_dom::parse_html;
use anvil_script::{ScriptRuntime, is_cpp_jit_kernel_active};

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    let node = doc.get_element_by_id(id).expect(id);
    doc.text_content(node)
}

#[test]
fn a_register_microtask_is_charged_on_the_same_turn() {
    let mut doc = parse_html("<p id=\"a\">init-a</p><p id=\"b\">init-b</p>");
    let mut rt = ScriptRuntime::new();
    let result = rt.eval_with_cinder_engine(
        &mut doc,
        "queueMicrotask(function() {\
            var i = 0;\
            while (i < 3000) { i = i + 1; }\
            document.getElementById('a').textContent = 'a:' + i;\
         });\
         queueMicrotask(function() {\
            var j = 0;\
            while (j < 3000) { j = j + 1; }\
            document.getElementById('b').textContent = 'b:' + j;\
         });",
    );
    let a = text(&doc, "a");
    let b = text(&doc, "b");
    let charged = rt.register_steps_charged();
    let budget = rt.page_budget();
    eprintln!("charge-register a={a} b={b} charged={charged} budget={budget:?}");
    assert!(
        result.is_ok(),
        "register entry failed: {result:?} a={a} b={b} charged={charged} budget={budget:?} logs={:?}",
        rt.console_logs()
    );
    assert_eq!(
        (a.as_str(), b.as_str()),
        ("a:3000", "b:3000"),
        "register microtasks did not both run; charged={charged} budget={budget:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        charged > 6_000,
        "register microtasks ran but their steps were not charged; charged={charged} budget={budget:?}"
    );
    assert!(
        budget.steps_used > 6_000,
        "page turn did not record the register jobs; charged={charged} budget={budget:?}"
    );
}

#[test]
fn a_register_microtask_after_the_main_run_hits_the_ceiling() {
    // One register call. The job is queued first, then the main script runs
    // until the armed ceiling. Drain happens after that charge is recorded.
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let result = rt.eval_with_cinder_engine(
        &mut doc,
        "queueMicrotask(function() {\
            document.getElementById('out').textContent = 'started';\
            var n = 0;\
            while (n < 20000) { n = n + 1; }\
            document.getElementById('out').textContent = 'done:' + n;\
         });\
         var i = 0;\
         while (i < 1000000) { i = i + 1; }",
    );
    let out = text(&doc, "out");
    let charged = rt.register_steps_charged();
    let budget = rt.page_budget();
    eprintln!(
        "ceiling-register result={result:?} out={out:?} charged={charged} budget={budget:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        budget.steps_used >= budget.step_limit,
        "main register run did not spend the turn before drain; result={result:?} out={out} charged={charged} budget={budget:?}"
    );
    assert_eq!(
        out, "init",
        "register microtask ran after the main script had already spent the ceiling; result={result:?} charged={charged} budget={budget:?}"
    );
}

#[test]
fn an_unconfigured_numeric_run_stops_at_the_builtin_ceiling() {
    if !is_cpp_jit_kernel_active() {
        panic!("C++ numeric VM is not active, so this seam was not exercised");
    }
    let run = anvil_script::eval_cpp_vm_script_bindings(
        "let i = 0; while (i < 150000) { i = i + 1; } i;",
    );
    let Some((_, _bindings, telemetry)) = run else {
        panic!("eval_cpp_vm_script_bindings returned None");
    };
    let stop = telemetry.stop_reason();
    assert_eq!(
        stop.as_ref().map(|error| error.message.as_str()),
        Some("step limit reached: 100000 steps"),
        "unconfigured numeric entry stop={stop:?} ops={}",
        telemetry.bytecode_ops_executed
    );
}

#[test]
fn a_numeric_helper_loop_does_not_finish_after_the_turn_is_nearly_spent() {
    if !is_cpp_jit_kernel_active() {
        panic!("C++ numeric VM is not active, so the page helper was not exercised");
    }
    let mut doc = parse_html(
        "<p id=\"spot\"></p><p id=\"out\">init</p>\
         <script>\
           document.getElementById('out').textContent = 'burn';\
           var i = 0;\
           while (i < 15000) { i = i + 1; document.getElementById('spot'); }\
           var k = 0;\
           while (k < 50000) { k = k + 1; }\
           document.getElementById('out').textContent = 'k:' + k;\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let out = text(&doc, "out");
    let budget = rt.page_budget();
    let k = rt.get_var("k").cloned();
    eprintln!(
        "near-spent-helper out={out:?} budget={budget:?} i={:?} k={k:?}",
        rt.get_var("i")
    );
    assert!(
        k.is_some(),
        "numeric loop was not reached, so the page helper was not exercised; out={out} budget={budget:?} i={:?}",
        rt.get_var("i")
    );
    assert_ne!(
        out,
        "k:50000",
        "numeric loop finished 50000 iterations after the document burn; budget={budget:?} i={:?} k={k:?} logs={:?}",
        rt.get_var("i"),
        rt.console_logs()
    );
    assert!(
        budget.steps_used >= 100_000,
        "burn did not approach the turn limit, so this was not a near-spent turn; out={out} budget={budget:?}"
    );
    assert!(
        budget.steps_used <= budget.step_limit,
        "turn account passed the ceiling; budget={budget:?}"
    );
}
