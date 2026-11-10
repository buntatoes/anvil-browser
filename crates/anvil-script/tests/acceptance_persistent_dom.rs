// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Canonical persistent DOM VM: the register engine. A closure captured in
//! one classic script is the same value in a later classic script and in a
//! DOM callback. A throw does not rerun the handler. A promise continuation
//! touches the DOM inside the same task.

use anvil_dom::parse_html;
use anvil_script::{ScriptError, ScriptFetchReply, ScriptRuntime};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// `0` refuses `./still.js`. `1` returns that relative key. `2` returns `./other.js`.
static STILL_RESOLVE: AtomicU8 = AtomicU8::new(0);

fn resolve_still(base: &str, specifier: &str) -> Option<String> {
    let _ = base;
    if specifier == "./Still.js" {
        return Some("./Still.js".to_string());
    }
    if specifier != "./still.js" {
        return None;
    }
    match STILL_RESOLVE.load(Ordering::SeqCst) {
        0 => None,
        1 => Some("./still.js".to_string()),
        _ => Some("./other.js".to_string()),
    }
}

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    doc.text_content(doc.get_element_by_id(id).expect(id))
}

#[test]
fn a_closure_survives_a_second_classic_script_and_a_click() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           function make() {\
             var n = 1;\
             function inc() { n = n + 1; return n; }\
             return inc;\
           }\
           var inc = make();\
           function onClick() {\
             document.getElementById('out').textContent = 'c:' + inc();\
           }\
           document.getElementById('b').addEventListener('click', onClick);\
           document.getElementById('out').textContent = 's:' + inc();\
         </script>\
         <script>\
           inc();\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert!(
        rt.register_steps_charged() > 0,
        "closure scripts did not use the register VM; logs={:?} text={}",
        rt.console_logs(),
        text(&doc, "out")
    );
    assert_eq!(text(&doc, "out"), "s:2", "{:?}", rt.console_logs());
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    assert_eq!(
        text(&doc, "out"),
        "c:4",
        "click did not see the captured n; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn a_throw_in_a_callback_does_not_rerun_the_handler() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           function onClick() {\
             var cur = document.getElementById('out').textContent;\
             document.getElementById('out').textContent = cur + 'a';\
             throw 'boom';\
             document.getElementById('out').textContent = cur + 'b';\
           }\
           document.getElementById('b').addEventListener('click', onClick);\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    let got = text(&doc, "out");
    assert!(
        got.ends_with('a') && !got.contains('b'),
        "handler continued or reran after throw: {got:?} logs={:?}",
        rt.console_logs()
    );
    rt.dispatch_click(&mut doc, button);
    let again = text(&doc, "out");
    assert_eq!(
        again.matches('a').count(),
        2,
        "second click replayed: {again}"
    );
}

#[test]
fn a_promise_continuation_updates_the_dom_in_the_same_task() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           var n = 0;\
           function step() {\
             n = n + 1;\
             document.getElementById('out').textContent = 'p:' + n;\
           }\
           function onClick() {}\
           document.getElementById('b').addEventListener('click', onClick);\
           Promise.resolve().then(step).then(step);\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "p:2", "{:?}", rt.console_logs());
    let budget = rt.page_budget();
    assert!(
        budget.steps_used < budget.step_limit,
        "promise chain took a second slice; {budget:?}"
    );
    assert!(rt.register_steps_charged() > 0, "{:?}", rt.console_logs());
}

#[test]
fn an_exhausted_turn_drops_microtasks_on_both_routes() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           async function ignore() {}\
           queueMicrotask(function() {\
             document.getElementById('out').textContent = 'old-js';\
           });\
           var i = 0;\
           while (i < 1000000) { i = i + 1; }\
         </script>\
         <script>\
           function onClick() {}\
           document.getElementById('b').addEventListener('click', onClick);\
           queueMicrotask(function() {\
             document.getElementById('out').textContent = 'old-reg';\
           });\
           var j = 0;\
           while (j < 1000000) { j = j + 1; }\
         </script>\
         <script>\
           document.getElementById('out').textContent = 'next';\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    rt.run_timers(&mut doc);
    let got = text(&doc, "out");
    assert_eq!(got, "next", "old microtask ran after a new task: {got}");
    assert!(!got.contains("old"), "{got}");
}

#[test]
fn an_arrow_and_a_function_expression_share_the_register_vm() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           var n = 1;\
           var inc = () => { n = n + 1; return n; };\
           var anon = function() { return inc(); };\
           function onClick() {\
             document.getElementById('out').textContent = 'c:' + anon();\
           }\
           document.getElementById('b').addEventListener('click', onClick);\
           document.getElementById('out').textContent = 's:' + anon();\
         </script>\
         <script>\
           anon();\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert!(
        rt.register_steps_charged() > 0,
        "arrow script left the register VM; logs={:?} text={}",
        rt.console_logs(),
        text(&doc, "out")
    );
    assert_eq!(text(&doc, "out"), "s:2", "{:?}", rt.console_logs());
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    assert_eq!(text(&doc, "out"), "c:4", "{:?}", rt.console_logs());
}

#[test]
fn arrow_and_anonymous_callbacks_share_one_register_environment() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           let n = 1;\
           const bump = () => { n = n + 1; };\
           document.getElementById('b').addEventListener('click', function() {\
             bump();\
             document.getElementById('out').textContent = 'c:' + n;\
           });\
           Promise.resolve().then(() => {\
             n = n + 10;\
             document.getElementById('out').textContent = 'p:' + n;\
           });\
         </script>\
         <script>\
           document.getElementById('out').textContent =\
             document.getElementById('out').textContent + '/s:' + n;\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    let before = rt.register_steps_charged();
    rt.run_document_scripts(&mut doc);
    let after_scripts = rt.register_steps_charged();
    assert!(
        after_scripts > before,
        "classic and promise did not run on the register VM: {after_scripts}"
    );
    assert_eq!(text(&doc, "out"), "p:11/s:11", "{:?}", rt.console_logs());
    assert!(
        rt.get_var("n").is_none(),
        "interpreter kept a second binding: {:?}",
        rt.get_var("n")
    );
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    let after_click = rt.register_steps_charged();
    assert!(
        after_click > after_scripts,
        "click callback did not charge the register VM: before={after_scripts} after={after_click}"
    );
    assert_eq!(text(&doc, "out"), "c:12", "{:?}", rt.console_logs());
    assert!(
        rt.get_var("n").is_none(),
        "click split n into the interpreter"
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'later:' + n;",
    );
    let after_later = rt.register_steps_charged();
    assert!(
        after_later > after_click,
        "later classic left the register VM"
    );
    assert_eq!(text(&doc, "out"), "later:12", "{:?}", rt.console_logs());
    assert!(rt.get_var("n").is_none(), "later classic copied n");
}

#[test]
fn an_anonymous_listener_throw_is_not_replayed() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           document.getElementById('b').addEventListener('click', function() {\
             var cur = document.getElementById('out').textContent;\
             document.getElementById('out').textContent = cur + 'a';\
             throw 'boom';\
             document.getElementById('out').textContent = cur + 'b';\
           });\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert!(rt.register_steps_charged() > 0, "{:?}", rt.console_logs());
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    let got = text(&doc, "out");
    assert!(
        got.ends_with('a') && !got.contains('b'),
        "anonymous handler continued or reran: {got:?} logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn listener_slices_do_not_leave_a_promise_chain_unrun() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           function onClick() {}\
           document.getElementById('b').addEventListener('click', onClick);\
           var n = 0;\
           function step() {\
             n = n + 1;\
             document.getElementById('out').textContent = '' + n;\
             if (n < 40) { Promise.resolve().then(step); }\
           }\
           Promise.resolve().then(step);\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(
        text(&doc, "out"),
        "40",
        "promise chain stopped at the 32-job slice; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn a_missing_static_import_suspends_until_the_host_compiles_it() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let importer = rt
        .compile_module(
            "import { n } from \"https://example.test/a.js\";\n\
             document.getElementById('out').textContent = 'imp:' + n;\n",
            "https://example.test/b.js",
        )
        .expect("compile importer");
    let status = rt.module_status(importer).expect("status");
    assert_eq!(status.phase, anvil_script::ModulePhase::Compiled);
    assert_eq!(status.requests, ["https://example.test/a.js"]);
    let suspended = rt.instantiate_module(importer).expect_err("fetch");
    match suspended {
        ScriptError::NeedsFetch { specifier, from } => {
            assert_eq!(specifier, "https://example.test/a.js");
            assert_eq!(from, importer);
        }
        other => panic!("expected NeedsFetch, got {other:?}"),
    }
    assert_eq!(
        rt.module_status(importer).expect("still compiled").phase,
        anvil_script::ModulePhase::Compiled
    );
    let exporter = rt
        .compile_module("export var n = 2;\n", "https://example.test/a.js")
        .expect("compile exporter");
    assert_eq!(
        rt.module_status(exporter).expect("export status").exports,
        ["n"]
    );
    rt.instantiate_module(importer).expect("resume instantiate");
    let before_eval = rt.register_steps_charged();
    rt.eval_module(&mut doc, importer).expect("eval");
    assert!(
        rt.register_steps_charged() > before_eval,
        "module evaluation did not run on the register VM"
    );
    assert_eq!(text(&doc, "out"), "imp:2", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(importer).expect("evaluated").phase,
        anvil_script::ModulePhase::Evaluated
    );
}

#[test]
fn export_function_links_and_dynamic_import_suspends_until_compiled() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let importer = rt
        .compile_module(
            "import { add } from \"https://example.test/math.js\";\n\
             document.getElementById('out').textContent = 'f:' + add(2, 5);\n",
            "https://example.test/app.js",
        )
        .expect("compile importer");
    let missing = rt
        .import_dynamic(&mut doc, "https://example.test/math.js", importer)
        .expect_err("dynamic import suspends");
    match missing {
        ScriptError::NeedsFetch { specifier, from } => {
            assert_eq!(specifier, "https://example.test/math.js");
            assert_eq!(from, importer);
        }
        other => panic!("expected NeedsFetch, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
    let math = rt
        .compile_module(
            "export function add(a, b) { return a + b; }\n",
            "https://example.test/math.js",
        )
        .expect("compile math");
    assert_eq!(rt.module_status(math).expect("math").exports, ["add"]);
    let before = rt.register_steps_charged();
    let loaded = rt
        .import_dynamic(&mut doc, "https://example.test/math.js", importer)
        .expect("dynamic import resumes");
    assert_eq!(loaded, math);
    assert!(
        rt.register_steps_charged() > before,
        "dynamic import evaluated off the register VM"
    );
    assert_eq!(
        rt.module_status(math).expect("evaluated").phase,
        anvil_script::ModulePhase::Evaluated
    );
    rt.instantiate_module(importer).expect("link importer");
    rt.eval_module(&mut doc, importer).expect("eval importer");
    assert_eq!(text(&doc, "out"), "f:7", "{:?}", rt.console_logs());
    assert!(
        rt.get_var("add").is_none(),
        "module export leaked into the interpreter"
    );
}

#[test]
fn js_listener_and_microtask_caps_fail_visibly() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var i = 0;\
         var el = document.getElementById('b');\
         el.addEventListener('click', function() {\
           document.getElementById('out').textContent = 'one';\
         });\
         while (i < 1030) {\
           el.__nativeAddListener('click', function() {});\
           i = i + 1;\
         }\
         document.getElementById('out').textContent = 'listeners';",
    );
    assert_eq!(text(&doc, "out"), "listeners", "{:?}", rt.console_logs());
    assert_eq!(
        rt.register_listener_count(),
        1024,
        "{:?}",
        rt.console_logs()
    );
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("listener queue full")),
        "{:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "function noop() {}\
         function mark() { document.getElementById('out').textContent = 'ran'; }\
         var i = 0;\
         while (i < 8192) { queueMicrotask(noop); i = i + 1; }\
         queueMicrotask(mark);",
    );
    assert_ne!(text(&doc, "out"), "ran", "{:?}", rt.console_logs());
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("microtask queue full")),
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn an_earlier_microtask_survives_a_later_exhausted_classic() {
    let mut doc = parse_html(
        "<p id=\"early\">init-early</p><p id=\"late\">init-late</p><p id=\"mark\">init-mark</p>\
         <script>\
           async function ignore() {}\
           queueMicrotask(function() {\
             document.getElementById('early').textContent = 'kept';\
           });\
         </script>\
         <script>\
           async function ignore2() {}\
           queueMicrotask(function() {\
             document.getElementById('late').textContent = 'dropped';\
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
    assert_eq!(text(&doc, "early"), "kept", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "late"), "init-late", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "mark"), "next");
}

#[test]
fn lone_eval_script_drains_its_microtask_but_not_its_timer() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "async function ignore() {}\
         queueMicrotask(function() {\
           document.getElementById('out').textContent = 'job';\
         });\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'late';\
         }, 0);",
    );
    assert_eq!(text(&doc, "out"), "job", "{:?}", rt.console_logs());
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "late", "{:?}", rt.console_logs());
}

#[test]
fn listener_plus_fetch_reports_the_host_that_ran() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/");
    rt.register_fetch_response("/api/status", 200, "{\"state\":\"online\"}");
    rt.eval_script(
        &mut doc,
        "document.getElementById('b').addEventListener('click', function() {});\
         var resp = fetch('/api/status');\
         var info = resp.json();\
         document.getElementById('out').textContent = '' + info.state;",
    );
    let charged = rt.register_steps_charged();
    assert!(
        charged > 0 && charged < 50_000,
        "same-origin mock fetch did not stay on a bounded register heap: {charged}"
    );
    assert_eq!(text(&doc, "out"), "online", "{:?}", rt.console_logs());
}

#[test]
fn a_later_classic_reuses_the_closure_for_timeout_promise_and_click() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           let n = 1;\
           const bump = () => { n = n + 1; };\
           document.getElementById('b').addEventListener('click', function() {\
             bump();\
             document.getElementById('out').textContent = 'c:' + n;\
           });\
         </script>\
         <script>\
           queueMicrotask(function() { n = n + 10; });\
           Promise.resolve().then(function() { n = n + 100; });\
           setTimeout(function() {\
             bump();\
             document.getElementById('out').textContent = 't:' + n;\
           }, 0);\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "init", "load ran the 0 ms timer");
    assert!(
        rt.run_timers(&mut doc) > 0,
        "due poll did not run the timeout"
    );
    let charged = rt.register_steps_charged();
    let budget = rt.page_budget();
    assert_eq!(text(&doc, "out"), "t:112", "{:?}", rt.console_logs());
    assert!(rt.get_var("n").is_none(), "n leaked to the interpreter");
    assert!(
        charged > 0 && charged < 50_000,
        "register charge was not a bounded VM run: {charged} {budget:?}"
    );
    assert!(
        budget.steps_used > 0 && budget.steps_used < 50_000 && !budget.turn_limited,
        "page slice was not bounded: {budget:?}"
    );
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    let click_delta = rt.register_steps_charged() - charged;
    assert!(
        click_delta > 0 && click_delta < 5_000,
        "click charge was not a short callback: {click_delta}"
    );
    assert_eq!(text(&doc, "out"), "c:113", "{:?}", rt.console_logs());
}

#[test]
fn a_timer_classic_opens_the_heap_and_a_later_global_reads_it() {
    // No listener and no earlier session. The timer classic itself selects
    // the register VM. The next classic's `extra` is on that heap and reads `n`.
    let mut doc = parse_html(
        "<p id=\"out\">init</p><p id=\"side\">init-side</p><button id=\"b\"></button>\
         <script>\
           let n = 1;\
           const bump = () => { n = n + 1; };\
           queueMicrotask(function() { n = n + 10; });\
           setTimeout(function() {\
             bump();\
             document.getElementById('out').textContent = 't:' + n;\
           }, 0);\
         </script>\
         <script>\
           let extra = 7;\
           document.getElementById('side').textContent = '' + (n + extra);\
           document.getElementById('b').addEventListener('click', function() {\
             bump();\
             document.getElementById('out').textContent = 'c:' + (n + extra);\
           });\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "init", "load ran the 0 ms timer");
    assert!(
        rt.run_timers(&mut doc) > 0,
        "due poll did not run the timeout"
    );
    let charged = rt.register_steps_charged();
    assert_eq!(text(&doc, "side"), "18", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "out"), "t:12", "{:?}", rt.console_logs());
    assert!(rt.get_var("n").is_none() && rt.get_var("extra").is_none());
    assert!(
        charged > 0 && charged < 50_000,
        "timer classic was not a bounded register run: {charged}"
    );
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    let click_delta = rt.register_steps_charged() - charged;
    assert!(click_delta > 0 && click_delta < 5_000, "{click_delta}");
    assert_eq!(text(&doc, "out"), "c:20", "{:?}", rt.console_logs());
}

#[test]
fn interval_cancel_release_and_register_delay_order() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "let n = 0;\
         let id = setInterval(function() {\
           n = n + 1;\
           if (n >= 3) {\
             clearInterval(id);\
             document.getElementById('out').textContent = 'ticks:' + n;\
           }\
         }, 5);",
    );
    assert_eq!(text(&doc, "out"), "init");
    assert!(rt.register_steps_charged() > 0);
    rt.run_timers(&mut doc);
    assert_eq!(
        text(&doc, "out"),
        "init",
        "a future interval fired without a clock advance"
    );
    for _ in 0..3 {
        rt.advance_register_clock(5.0);
        rt.run_timers(&mut doc);
    }
    assert_eq!(text(&doc, "out"), "ticks:3", "{:?}", rt.console_logs());
    assert!(rt.get_var("n").is_none());
    let charged = rt.register_steps_charged();
    assert!(charged < 50_000, "{charged}");

    let mut released = parse_html("<p id=\"out\">stay</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut released,
        "setTimeout(function(){ document.getElementById('out').textContent = 'late'; }, 0);",
    );
    assert!(rt.register_steps_charged() > 0);
    assert_eq!(text(&released, "out"), "stay");
    rt.release_page_callbacks();
    rt.run_timers(&mut released);
    assert_eq!(text(&released, "out"), "stay", "{:?}", rt.console_logs());

    let mut ordered = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut ordered,
        "document.getElementById('out');\
         setTimeout(function() { console.log('huge'); setTimeout(function() { console.log('after-huge'); }, 1e300); }, 1e300);\
         setTimeout(function() { console.log('inf'); }, Infinity);\
         setTimeout(function() { console.log('neg'); }, -5);\
         setTimeout(function() { console.log('nan'); }, NaN);\
         setTimeout(function() { console.log('ten'); }, 10);\
         queueMicrotask(function() { console.log('micro'); });",
    );
    rt.run_timers(&mut ordered);
    let logs: Vec<&str> = rt
        .console_logs()
        .iter()
        .map(String::as_str)
        .filter(|line| !line.starts_with("[error]") && !line.starts_with("[warn]"))
        .collect();
    assert_eq!(
        logs,
        ["micro", "neg", "nan"],
        "a 10ms timeout fired before its due time: {:?}",
        rt.console_logs()
    );
    rt.advance_register_clock(10.0);
    rt.run_timers(&mut ordered);
    let logs: Vec<&str> = rt
        .console_logs()
        .iter()
        .map(String::as_str)
        .filter(|line| !line.starts_with("[error]") && !line.starts_with("[warn]"))
        .collect();
    assert_eq!(
        logs,
        ["micro", "neg", "nan", "ten"],
        "{:?}",
        rt.console_logs()
    );
    let due = rt.next_timer_due_ms().expect("huge timer still queued");
    assert!(
        due.is_finite() && due > 1.0e6,
        "next deadline was not the far timer: {due}"
    );
    // Synthetic clock, not the browser route. It only moves time.
    rt.advance_register_clock(1.0e301);
    rt.run_timers(&mut ordered);
    rt.advance_register_clock(1.0e301);
    rt.run_timers(&mut ordered);
    let logs: Vec<&str> = rt
        .console_logs()
        .iter()
        .map(String::as_str)
        .filter(|line| !line.starts_with("[error]") && !line.starts_with("[warn]"))
        .collect();
    assert_eq!(
        logs,
        ["micro", "neg", "nan", "ten", "huge", "inf", "after-huge"],
        "{:?}",
        rt.console_logs()
    );
    assert!(rt.register_steps_charged() > 0);
    assert!(rt.register_steps_charged() < 50_000);
}

#[test]
fn input_still_runs_after_the_timer_queue_is_full() {
    let mut doc = parse_html("<p id=\"out\">init</p><input id=\"field\">");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var field = document.getElementById('field');\
         field.addEventListener('input', function() {\
           document.getElementById('out').textContent = 'in:' + field.value;\
         });\
         var i = 0;\
         while (i < 130) { setTimeout(function() {}, 1000000); i = i + 1; }",
    );
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("timer queue full")),
        "{:?}",
        rt.console_logs()
    );
    let before = rt.register_steps_charged();
    let field = doc.get_element_by_id("field").expect("field");
    assert!(rt.dispatch_input(&mut doc, field, "typed"));
    let delta = rt.register_steps_charged() - before;
    assert!(delta > 0 && delta < 5_000, "{delta}");
    assert_eq!(text(&doc, "out"), "in:typed", "{:?}", rt.console_logs());
    assert!(
        rt.page_budget().steps_used < 50_000,
        "{:?}",
        rt.page_budget()
    );
}

#[test]
fn fetch_and_async_open_the_heap_and_keep_the_closure() {
    let mut doc =
        parse_html("<p id=\"out\">init</p><button id=\"b\"></button><input id=\"field\">");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.register_fetch_response("/api/status", 200, "{\"state\":\"online\",\"k\":1}");
    rt.register_fetch_response("https://app.example/ok", 200, "{\"k\":3}");
    rt.eval_script(
        &mut doc,
        "let n = 1;\
         const bump = () => { n = n + 1; };\
         document.getElementById('field').addEventListener('input', function() {\
           document.getElementById('out').textContent = 'in:' + n;\
         });\
         fetch('/api/status').then(function(resp) { return resp.json(); }).then(function(info) {\
           bump();\
           document.getElementById('out').textContent = 'ok:' + info.state + ':' + n;\
         });\
         setTimeout(function() {\
           document.getElementById('out').textContent =\
             document.getElementById('out').textContent + '/t:' + n;\
         }, 0);",
    );
    let after_fetch = rt.register_steps_charged();
    assert!(
        after_fetch > 0 && after_fetch < 50_000,
        "fetch classic did not open a bounded register heap: {after_fetch}"
    );
    assert!(rt.get_var("n").is_none(), "n was copied to the interpreter");
    assert_eq!(text(&doc, "out"), "ok:online:2", "{:?}", rt.console_logs());
    rt.run_timers(&mut doc);
    assert_eq!(
        text(&doc, "out"),
        "ok:online:2/t:2",
        "{:?}",
        rt.console_logs()
    );
    let field = doc.get_element_by_id("field").expect("field");
    let before_input = rt.register_steps_charged();
    assert!(rt.dispatch_input(&mut doc, field, "x"));
    let input_delta = rt.register_steps_charged() - before_input;
    assert!(input_delta > 0 && input_delta < 5_000, "{input_delta}");
    assert_eq!(text(&doc, "out"), "in:2", "{:?}", rt.console_logs());

    let mut rejected = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.eval_script(
        &mut rejected,
        "let n = 5;\
         fetch('https://other.example/secret').then(function() {\
           document.getElementById('out').textContent = 'bad';\
         }).catch(function() {\
           document.getElementById('out').textContent = 'rej:' + n;\
         });",
    );
    assert!(rt.register_steps_charged() > 0);
    assert!(rt.get_var("n").is_none());
    assert_eq!(text(&rejected, "out"), "rej:5", "{:?}", rt.console_logs());

    let mut async_doc = parse_html("<p id=\"out\">init</p><p id=\"side\">init-side</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.register_fetch_response("https://app.example/ok", 200, "{\"k\":3}");
    rt.eval_script(
        &mut async_doc,
        "let n = 2;\
         let tag = 'kept';\
         const mark = () => 'm' + n;\
         async function go() {\
           let resp = await fetch('https://app.example/ok');\
           let info = await resp.json();\
           n = n + info.k;\
           document.getElementById('out').textContent = mark();\
         }\
         go();\
         async function fail() {\
           try {\
             await fetch('https://evil.example/x');\
             document.getElementById('side').textContent = 'bad';\
           } catch (e) {\
             document.getElementById('side').textContent = 'no:' + tag;\
           }\
         }\
         fail();",
    );
    assert!(
        rt.register_steps_charged() > 0 && rt.register_steps_charged() < 50_000,
        "{:?}",
        rt.console_logs()
    );
    assert!(rt.get_var("n").is_none());
    assert_eq!(text(&async_doc, "out"), "m5", "{:?}", rt.console_logs());
    assert_eq!(
        text(&async_doc, "side"),
        "no:kept",
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn fetch_continuation_does_not_run_after_release_or_rebind() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.register_fetch_response("/api/status", 200, "{\"state\":\"online\"}");
    rt.eval_script(
        &mut doc,
        "let n = 1;\
         setTimeout(function() {\
           fetch('/api/status').then(function() {\
             n = n + 1;\
             document.getElementById('out').textContent = 'late:' + n;\
           });\
         }, 0);",
    );
    assert_eq!(text(&doc, "out"), "init");
    assert!(rt.register_steps_charged() > 0);
    rt.release_page_callbacks();
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());

    let mut first = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.register_fetch_response("/api/status", 200, "{\"state\":\"online\"}");
    rt.eval_script(
        &mut first,
        "setTimeout(function() {\
           fetch('/api/status').then(function() {\
             document.getElementById('out').textContent = 'stale';\
           });\
         }, 0);",
    );
    let mut other = parse_html("<p id=\"out\">fresh</p>");
    rt.rebind_page_document(&other);
    rt.run_timers(&mut other);
    assert_eq!(text(&other, "out"), "fresh", "{:?}", rt.console_logs());
    assert_eq!(text(&first, "out"), "init");
}

#[test]
fn timeout_promise_and_click_share_one_register_heap() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "let n = 1;\
         const bump = () => { n = n + 1; };\
         document.getElementById('b').addEventListener('click', function() {\
           bump();\
           document.getElementById('out').textContent = 'c:' + n;\
         });\
         queueMicrotask(function() { n = n + 10; });\
         Promise.resolve().then(function() { n = n + 100; });\
         var iv = setInterval(function() {\
           n = n + 1;\
           document.getElementById('out').textContent = 'i:' + n;\
           clearInterval(iv);\
         }, 5);\
         setTimeout(function() {\
           bump();\
           document.getElementById('out').textContent = 't:' + n;\
         }, 0);",
    );
    let after_classic = rt.register_steps_charged();
    assert!(after_classic > 0, "classic did not run on the register VM");
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    rt.run_timers(&mut doc);
    let after_timers = rt.register_steps_charged();
    assert!(
        after_timers > after_classic,
        "timer callbacks did not charge the register VM: {after_classic} -> {after_timers}"
    );
    assert_eq!(text(&doc, "out"), "t:112", "{:?}", rt.console_logs());
    rt.advance_register_clock(5.0);
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "i:113", "{:?}", rt.console_logs());
    assert!(
        rt.get_var("n").is_none(),
        "n was copied onto the interpreter"
    );
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    assert!(
        rt.register_steps_charged() > after_timers,
        "click did not charge the register VM"
    );
    assert_eq!(text(&doc, "out"), "c:114", "{:?}", rt.console_logs());
}

#[test]
fn a_far_timeout_stays_queued_and_input_runs_while_it_waits() {
    let mut doc = parse_html("<p id=\"out\">init</p><input id=\"field\">");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var field = document.getElementById('field');\
         field.addEventListener('input', function() {\
           document.getElementById('out').textContent = 'in:' + field.value;\
         });\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'far';\
         }, 100000);\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'now';\
         }, 0);",
    );
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "now", "{:?}", rt.console_logs());
    let due = rt.next_timer_due_ms().expect("100s timer still queued");
    assert!(due > 1_000.0, "{due}");
    let field = doc.get_element_by_id("field").expect("field");
    let before = rt.register_steps_charged();
    assert!(rt.dispatch_input(&mut doc, field, "typed"));
    assert_eq!(text(&doc, "out"), "in:typed", "{:?}", rt.console_logs());
    assert!(rt.register_steps_charged() - before < 5_000);
    assert!(
        rt.next_timer_due_ms().is_some(),
        "input drained the far timer"
    );
}

#[test]
fn an_exhausted_turn_drops_only_its_timeout() {
    let mut doc = parse_html("<p id=\"out\">init</p><p id=\"mark\">init-mark</p>");
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    rt.eval_script(
        &mut doc,
        "setTimeout(function() {\
           document.getElementById('out').textContent = 'late';\
         }, 0);\
         var i = 0;\
         while (i < 1000000) { i = i + 1; }",
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('mark').textContent = 'next';",
    );
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "mark"), "next");
    assert!(rt.next_timer_due_ms().is_none());
}

#[test]
fn a_cloned_document_does_not_receive_timer_callbacks() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "setTimeout(function() {\
           document.getElementById('out').textContent = 'late';\
         }, 0);",
    );
    assert_eq!(text(&doc, "out"), "init");
    assert!(rt.has_pending_timers());
    let mut clone = doc.clone();
    assert_ne!(clone.identity(), doc.identity());
    assert_eq!(rt.run_timers(&mut clone), 0);
    assert_eq!(text(&clone, "out"), "init", "timer wrote the clone");
    assert_eq!(text(&doc, "out"), "init");
    assert!(rt.has_pending_timers(), "wrong document consumed the timer");
    assert!(rt.run_timers(&mut doc) > 0);
    assert_eq!(text(&doc, "out"), "late");
    assert_eq!(text(&clone, "out"), "init");
    rt.rebind_page_document(&clone);
    assert!(!rt.has_pending_timers(), "rebind kept the previous queue");
    assert_eq!(rt.run_timers(&mut clone), 0);
    assert_eq!(text(&clone, "out"), "init");
}

#[test]
fn an_interpreter_far_timeout_does_not_jump_the_clock() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "async function ignore() {}\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'late';\
         }, 100000);",
    );
    rt.run_document_scripts(&mut doc);
    rt.run_timers(&mut doc);
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    let due = rt
        .next_timer_due_ms()
        .expect("interpreter timer still queued");
    assert!(due >= 100_000.0, "{due}");
    assert!(!rt.sync_timer_clock(f64::NAN));
    assert!(rt.sync_timer_clock(1_000.0));
    assert!(!rt.sync_timer_clock(10.0), "a backward now moved the clock");
    assert_eq!(rt.next_timer_due_ms(), Some(due));
    let poll = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert!(poll.fired > 0, "{poll:?}");
    assert_eq!(text(&doc, "out"), "late");
}

#[test]
fn sync_timer_clock_makes_a_later_timeout_relative_to_now() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    assert!(rt.sync_timer_clock(5_000.0));
    rt.eval_script(
        &mut doc,
        "setTimeout(function() {\
           document.getElementById('out').textContent = 'ten';\
         }, 10);",
    );
    let due = rt.next_timer_due_ms().expect("timeout queued");
    assert!(
        (due - 5_010.0).abs() < 1.0,
        "timeout was scheduled against a stale clock: {due}"
    );
    let early = rt.run_due_timers(
        &mut doc,
        5_000.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert_eq!(early.fired, 0, "{early:?}");
    assert_eq!(text(&doc, "out"), "init");
    let ready = rt.run_due_timers(
        &mut doc,
        5_010.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert!(ready.fired > 0, "{ready:?}");
    assert_eq!(text(&doc, "out"), "ten");
}

#[test]
fn request_submit_records_the_handshake_after_the_listener() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"g\"><input id=\"q\" name=\"q\" value=\"before\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('g').addEventListener('submit', function() {\
           document.getElementById('q').value = 'from-listener';\
         });\
         document.getElementById('g').requestSubmit();",
    );
    let input = doc.get_element_by_id("q").expect("q");
    assert_eq!(doc.input_value(input), "from-listener");
    let handshake = rt.take_host_submission().expect("handshake");
    assert!(
        handshake.stale_against(&doc).is_none(),
        "listener mutation made the record stale: {handshake:?} gen={}",
        doc.mutation_generation()
    );
    let snapshot = rt.take_submission_snapshot().expect("snapshot");
    let captured = format!("{snapshot:?}");
    assert!(
        captured.contains("from-listener"),
        "snapshot missed the listener value: {captured}"
    );
    assert_eq!(snapshot.epoch, rt.submission_epoch());
    assert!(handshake.stale_against(&doc).is_none());
    doc.set_text_content(doc.get_element_by_id("out").expect("out"), "later");
    assert!(
        format!("{snapshot:?}").contains("from-listener"),
        "later write changed the snapshot"
    );
    assert!(handshake.stale_against(&doc).is_some());
    let epoch_at_take = snapshot.epoch;
    rt.release_page_callbacks();
    assert_ne!(epoch_at_take, rt.submission_epoch());
}

#[test]
fn request_submit_rejects_a_foreign_submitter_before_events() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"a\"><button id=\"go\" type=\"submit\"></button></form>\
         <form id=\"b\"><input id=\"q\" name=\"q\" value=\"keep\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var out = document.getElementById('out');\
         try {\
           document.getElementById('b').requestSubmit(document.getElementById('go'));\
           out.textContent = 'no-throw';\
         } catch (e) {\
           out.textContent = e.name;\
         }",
    );
    assert_eq!(
        text(&doc, "out"),
        "NotFoundError",
        "{:?}",
        rt.console_logs()
    );
    assert!(rt.take_host_submission().is_none());
    assert!(rt.take_submission_snapshot().is_none());
    rt.eval_script(
        &mut doc,
        "var out = document.getElementById('out');\
         try {\
           document.getElementById('b').requestSubmit(document.getElementById('q'));\
           out.textContent = 'no-throw';\
         } catch (e) {\
           out.textContent = e.name;\
         }",
    );
    assert_eq!(text(&doc, "out"), "TypeError", "{:?}", rt.console_logs());
    assert!(rt.take_submission_snapshot().is_none());
}

#[test]
fn an_invalid_request_submit_does_not_replace_a_planned_snapshot() {
    let mut doc = parse_html(
        "<form id=\"ok\"><input name=\"q\" value=\"planned\"></form>\
         <form id=\"bad\"><input name=\"q\" required value=\"\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('ok').requestSubmit();\
         document.getElementById('bad').requestSubmit();",
    );
    let snapshot = rt.take_submission_snapshot().expect("planned snapshot");
    assert_eq!(snapshot.form, doc.get_element_by_id("ok").unwrap());
    assert!(format!("{snapshot:?}").contains("planned"), "{snapshot:?}");
    let handshake = rt.take_host_submission().expect("planned handshake");
    assert_eq!(handshake.form, snapshot.form);
}

#[test]
fn a_submit_snapshot_ignores_a_listener_microtask() {
    let mut doc = parse_html("<form id=\"g\"><input id=\"q\" name=\"q\" value=\"before\"></form>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var q = document.getElementById('q');\
         document.getElementById('g').addEventListener('submit', function() {\
           q.value = 'from-event';\
           queueMicrotask(function() { q.value = 'from-micro'; });\
         });\
         document.getElementById('g').requestSubmit();",
    );
    let snapshot = rt.take_submission_snapshot().expect("snapshot");
    let rendered = format!("{snapshot:?}");
    assert!(
        rendered.contains("from-event"),
        "snapshot missed the event value: {rendered}"
    );
    assert!(
        !rendered.contains("from-micro"),
        "snapshot included the microtask: {rendered}"
    );
    assert_eq!(
        doc.input_value(doc.get_element_by_id("q").unwrap()),
        "from-micro"
    );
}

#[test]
fn request_submit_dispatches_before_the_next_statement() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"g\"><input id=\"q\" name=\"q\" value=\"ok\"><button id=\"go\" type=\"submit\"></button></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = '';\
         document.getElementById('g').addEventListener('submit', function(ev) {\
           var node = document.getElementById('out');\
           node.textContent = node.textContent + 'submit;';\
           ev.preventDefault();\
         });\
         document.getElementById('g').requestSubmit();\
         var node = document.getElementById('out');\
         node.textContent = node.textContent + 'after;';",
    );
    assert_eq!(
        text(&doc, "out"),
        "submit;after;",
        "steps={} unsupported={:?} logs={:?}",
        rt.register_steps_charged(),
        rt.last_unsupported(),
        rt.console_logs()
    );
    assert!(rt.register_steps_charged() > 0);
    assert!(
        rt.take_host_submission().is_none(),
        "preventDefault still submitted"
    );

    let mut invalid = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"g\"><input id=\"q\" name=\"q\" required value=\"\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut invalid,
        "document.getElementById('out').textContent = '';\
         document.getElementById('q').addEventListener('invalid', function() {\
           var node = document.getElementById('out');\
           node.textContent = node.textContent + 'invalid;';\
         });\
         document.getElementById('g').addEventListener('submit', function() {\
           var node = document.getElementById('out');\
           node.textContent = node.textContent + 'submit;';\
         });\
         document.getElementById('g').requestSubmit();\
         var node = document.getElementById('out');\
         node.textContent = node.textContent + 'after;';",
    );
    assert_eq!(
        text(&invalid, "out"),
        "invalid;after;",
        "{:?}",
        rt.console_logs()
    );
    rt.checkpoint_microtasks(&mut invalid);
    rt.run_timers(&mut invalid);
    assert!(rt.take_host_submission().is_none());
    assert!(rt.take_submission_snapshot().is_none());
    assert!(rt.take_pending_navigation().is_none());
}

#[test]
fn an_exhausted_callback_drops_only_its_descendants_on_both_queues() {
    let script = "setTimeout(function() {\
         document.getElementById('early').textContent = 'kept';\
       }, 0);\
       setTimeout(function() {\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'child-t';\
         }, 0);\
         setInterval(function() {\
           document.getElementById('out').textContent = 'child-i';\
         }, 10);\
         queueMicrotask(function() {\
           document.getElementById('out').textContent = 'child-j';\
         });\
         var i = 0;\
         while (i < 1000000) { i = i + 1; }\
       }, 0);\
       setTimeout(function() {\
         document.getElementById('later').textContent = 'later';\
       }, 1000);";
    for marker in ["async function ignore() {}", ""] {
        let mut doc =
            parse_html("<p id=\"early\">init</p><p id=\"out\">init</p><p id=\"later\">init</p>");
        let mut rt = ScriptRuntime::new();
        rt.eval_script(&mut doc, &format!("{marker}{script}"));
        let _ = rt.run_timers(&mut doc);
        assert_eq!(
            text(&doc, "early"),
            "kept",
            "{marker} {:?}",
            rt.console_logs()
        );
        assert_eq!(text(&doc, "out"), "init", "{marker}");
        assert_eq!(text(&doc, "later"), "init", "{marker}");
        let due = rt.next_timer_due_ms().expect("unrelated timer");
        assert!(due >= 1_000.0, "{marker} {due}");
        let _ = rt.run_due_timers(
            &mut doc,
            due,
            anvil_script::TimerBudget {
                max_callbacks: 4,
                max_steps: 10_000,
            },
        );
        assert_eq!(text(&doc, "later"), "later", "{marker}");
        assert_eq!(text(&doc, "out"), "init", "{marker} descendant ran");
    }
}

#[test]
fn dispatch_input_does_not_run_a_zero_delay_timer() {
    let mut doc = parse_html("<p id=\"out\">init</p><input id=\"field\">");
    let mut rt = ScriptRuntime::new();
    assert!(rt.sync_timer_clock(2_000.0));
    rt.eval_script(
        &mut doc,
        "var field = document.getElementById('field');\
         field.addEventListener('input', function() {\
           document.getElementById('out').textContent = 'in';\
           setTimeout(function() {\
             document.getElementById('out').textContent = 'timer';\
           }, 0);\
         });",
    );
    let field = doc.get_element_by_id("field").expect("field");
    assert!(rt.dispatch_input(&mut doc, field, "x"));
    assert_eq!(text(&doc, "out"), "in", "{:?}", rt.console_logs());
    let due = rt.next_timer_due_ms().expect("timer queued by input");
    assert!((due - 2_000.0).abs() < 1.0, "{due}");
    assert!(
        rt.run_due_timers(
            &mut doc,
            2_000.0,
            anvil_script::TimerBudget {
                max_callbacks: 4,
                max_steps: 10_000,
            },
        )
        .fired
            > 0
    );
    assert_eq!(text(&doc, "out"), "timer");
}

#[test]
fn a_due_poll_fires_an_overdue_interval_once_and_stops_at_the_budget() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var n = 0;\
         setInterval(function() {\
           n = n + 1;\
           document.getElementById('out').textContent = 'i' + n;\
         }, 10);\
         setTimeout(function() { document.getElementById('out').textContent = 'a'; }, 0);\
         setTimeout(function() { document.getElementById('out').textContent = 'b'; }, 0);\
         setTimeout(function() { document.getElementById('out').textContent = 'c'; }, 0);",
    );
    let limited = rt.run_due_timers(
        &mut doc,
        0.0,
        anvil_script::TimerBudget {
            max_callbacks: 2,
            max_steps: 10_000,
        },
    );
    assert_eq!(limited.fired, 2, "{limited:?}");
    assert!(matches!(
        limited.outcome,
        anvil_script::TimerPollOutcome::Exhausted
    ));
    assert_ne!(text(&doc, "out"), "c");
    assert!(rt.sync_timer_clock(1_000.0));
    let once = rt.run_due_timers(
        &mut doc,
        1_000.0,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert!(once.fired >= 1, "{once:?}");
    let after = text(&doc, "out");
    assert!(
        after == "i1" || after == "c" || after.starts_with('i'),
        "{after}"
    );
    let next = rt.next_timer_due_ms();
    if let Some(due) = next {
        assert!(due > 1_000.0, "interval catch-up stayed due now: {due}");
    }
}

#[test]
fn detached_form_submit_and_request_submit_are_no_ops() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"g\" action=\"/one\"><input name=\"q\" value=\"kept\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    let form = doc.get_element_by_id("g").expect("g");
    rt.eval_script(
        &mut doc,
        "var g = document.getElementById('g');\
         var n = 0;\
         g.addEventListener('submit', function() { n = n + 1; });\
         g.remove();\
         g.requestSubmit();\
         g.submit();\
         document.getElementById('out').textContent = '' + n;",
    );
    assert_eq!(text(&doc, "out"), "0", "{:?}", rt.console_logs());
    assert!(doc.get(form).parent().is_none());
    assert!(rt.take_submission_snapshot().is_none());
    assert!(rt.take_host_submission().is_none());
}

#[test]
fn detach_in_submit_listener_aborts_and_keeps_the_earlier_plan() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"one\" action=\"/one\"><input name=\"q\" value=\"first\"></form>\
         <form id=\"two\" action=\"/two\"><input name=\"q\" value=\"second\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var log = '';\
         document.getElementById('one').requestSubmit();\
         var two = document.getElementById('two');\
         two.addEventListener('submit', function() {\
           log = log + 'submit;';\
           two.remove();\
         });\
         two.addEventListener('formdata', function() {\
           log = log + 'formdata;';\
         });\
         two.requestSubmit();\
         log = log + 'after;';\
         document.getElementById('out').textContent = log;",
    );
    assert_eq!(
        text(&doc, "out"),
        "submit;after;",
        "{:?}",
        rt.console_logs()
    );
    let snapshot = rt.take_submission_snapshot().expect("earlier plan");
    assert_eq!(snapshot.form, doc.get_element_by_id("one").unwrap());
    assert!(
        snapshot.action.contains("/one"),
        "plan was replaced or cleared: {snapshot:?}"
    );
    assert!(rt.take_pending_navigation().is_none());
}

#[test]
fn detach_in_formdata_listener_aborts_and_keeps_the_earlier_plan() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"one\" action=\"/one\"><input name=\"q\" value=\"first\"></form>\
         <form id=\"two\" action=\"/two\"><input name=\"q\" value=\"second\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var log = '';\
         document.getElementById('one').requestSubmit();\
         var two = document.getElementById('two');\
         two.addEventListener('submit', function() { log = log + 'submit;'; });\
         two.addEventListener('formdata', function() {\
           log = log + 'formdata;';\
           two.remove();\
         });\
         two.requestSubmit();\
         log = log + 'after;';\
         document.getElementById('out').textContent = log;",
    );
    assert_eq!(
        text(&doc, "out"),
        "submit;formdata;after;",
        "{:?}",
        rt.console_logs()
    );
    let snapshot = rt.take_submission_snapshot().expect("earlier plan");
    assert!(snapshot.action.contains("/one"), "{snapshot:?}");
}

#[test]
fn removal_after_snapshot_does_not_cancel() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><form id=\"g\" action=\"/stay\"><input name=\"q\" value=\"kept\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var g = document.getElementById('g');\
         g.requestSubmit();\
         queueMicrotask(function() { g.remove(); });",
    );
    let form = doc.get_element_by_id("g");
    assert!(form.is_none() || doc.get(form.unwrap()).parent().is_none());
    let snapshot = rt.take_submission_snapshot().expect("planned snapshot");
    assert!(snapshot.action.contains("/stay"), "{snapshot:?}");
    assert!(format!("{snapshot:?}").contains("kept"), "{snapshot:?}");
}

#[test]
fn location_intent_keeps_the_page_active_until_commit() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><p id=\"n\">0</p>\
         <form id=\"g\" action=\"/one\"><input name=\"q\" value=\"kept\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/");
    rt.sync_timer_clock(1_000.0);
    rt.eval_script(
        &mut doc,
        "var n = 0;\
         var g = document.getElementById('g');\
         g.addEventListener('submit', function() { n = n + 1; });\
         g.requestSubmit();\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'timer';\
         }, 0);\
         location.href = '#section';\
         location.href = '';\
         location.href = 'javascript:alert(1)';\
         location.href = 'https://example.test/search?gbv=1';\
         g.requestSubmit();\
         document.getElementById('n').textContent = '' + n;",
    );
    assert!(
        rt.register_steps_charged() > 0,
        "register path did not run; logs={:?}",
        rt.console_logs()
    );
    assert_eq!(text(&doc, "n"), "2", "{:?}", rt.console_logs());
    assert_eq!(
        text(&doc, "out"),
        "init",
        "the timeout ran inside the script"
    );
    let origin = rt.page_origin().unwrap_or("");
    assert!(
        origin.ends_with("#section"),
        "fragment was not kept: {origin}"
    );
    assert!(
        !origin.contains("javascript:"),
        "javascript: was resolved into the page URL: {origin}"
    );
    assert!(rt.take_pending_navigation().is_none());
    assert_eq!(rt.submission_epoch(), 1);
    let planned = rt
        .take_submission_snapshot()
        .expect("plan survives the intent");
    assert!(format!("{planned:?}").contains("kept"), "{planned:?}");
    let due = rt.next_timer_due_ms().expect("timer still queued");
    let poll = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 50_000,
        },
    );
    assert!(poll.fired > 0, "{poll:?}");
    assert_eq!(text(&doc, "out"), "timer");

    rt.eval_script(
        &mut doc,
        "location.href = 'https://example.test/away';\
         g.requestSubmit();\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'late';\
         }, 10);\
         document.getElementById('n').textContent = '' + n;",
    );
    assert_eq!(
        text(&doc, "n"),
        "3",
        "next eval restored active and submitted"
    );
    assert_eq!(
        rt.take_pending_navigation().as_deref(),
        Some("https://example.test/away")
    );
    let still = rt.page_origin().unwrap_or("");
    assert!(
        still.starts_with("https://app.example/") && still.ends_with("#section"),
        "document URL became the navigation target: {still}"
    );
    assert!(
        rt.storage_origin_key().contains("app.example"),
        "storage origin followed the intent: {}",
        rt.storage_origin_key()
    );
    assert_eq!(rt.submission_epoch(), 1);
    assert!(rt.next_timer_due_ms().is_some());

    rt.commit_page_navigation();
    assert_eq!(rt.submission_epoch(), 2);
    assert!(rt.take_submission_snapshot().is_none());
    assert!(rt.next_timer_due_ms().is_none());
    let released = rt.run_due_timers(
        &mut doc,
        2_000.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 50_000,
        },
    );
    assert_eq!(released.fired, 0);
    assert_eq!(released.outcome, anvil_script::TimerPollOutcome::Released);
    assert_eq!(text(&doc, "out"), "timer");
    rt.eval_script(
        &mut doc,
        "g.requestSubmit();\
         document.getElementById('n').textContent = '' + n;",
    );
    assert_eq!(text(&doc, "n"), "3", "commit still suppresses submit");
}

#[test]
fn interpreter_location_intent_matches_the_register_rules() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/");
    rt.eval_script(&mut doc, "location.href = '#section';");
    rt.eval_script(&mut doc, "location.href = '';");
    rt.eval_script(&mut doc, "location.href = 'javascript:alert(1)';");
    rt.eval_script(
        &mut doc,
        "location.href = 'https://example.test/page?gbv=1';",
    );
    assert_eq!(rt.register_steps_charged(), 0, "{:?}", rt.console_logs());
    assert_eq!(rt.page_origin(), Some("https://app.example/#section"));
    assert!(rt.take_pending_navigation().is_none());
    assert_eq!(rt.submission_epoch(), 1);
    rt.eval_script(&mut doc, "location.href = 'https://example.test/away';");
    assert_eq!(
        rt.take_pending_navigation().as_deref(),
        Some("https://example.test/away")
    );
    assert_eq!(rt.page_origin(), Some("https://app.example/#section"));
    assert!(rt.storage_origin_key().contains("app.example"));
    assert_eq!(rt.submission_epoch(), 1);
    assert_eq!(rt.register_steps_charged(), 0, "{:?}", rt.console_logs());
}

#[test]
fn settled_await_resumes_after_the_caller_on_the_register_heap() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.sync_timer_clock(5_000.0);
    rt.eval_script(
        &mut doc,
        "var log = '';\
         var n = 1;\
         function bump() { n = n + 1; }\
         async function worker(name) {\
           await null;\
           bump();\
           log = log + 'a:' + name + ':' + n + ';';\
           try {\
             await Promise.reject(new Error('x'));\
             log = log + 'no;';\
           } catch (e) {\
             log = log + 'catch:' + e.message + ';';\
           }\
           setTimeout(function() {\
             bump();\
             document.getElementById('out').textContent = log + 't:' + n;\
           }, 0);\
           return name;\
         }\
         var p = worker('bee');\
         log = log + 'caller:' + n + ';';\
         p.then(function(v) { log = log + 'ret:' + v + ';'; });\
         document.getElementById('b').addEventListener('click', function() {\
           bump();\
           document.getElementById('out').textContent = log + 'c:' + n;\
         });",
    );
    assert!(
        rt.register_steps_charged() > 0,
        "await did not run on the register heap; logs={:?}",
        rt.console_logs()
    );
    let due = rt
        .next_timer_due_ms()
        .expect("timer queued by the resumed function");
    let poll = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 50_000,
        },
    );
    assert!(poll.fired > 0, "{poll:?}");
    assert_eq!(
        text(&doc, "out"),
        "caller:1;a:bee:2;catch:x;ret:bee;t:3",
        "{:?}",
        rt.console_logs()
    );
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    assert_eq!(text(&doc, "out"), "caller:1;a:bee:2;catch:x;ret:bee;c:4");
    assert!(rt.get_var("n").is_none() && rt.get_var("log").is_none());
    let charged = rt.register_steps_charged();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'later:' + n;",
    );
    assert_eq!(text(&doc, "out"), "later:4");
    assert!(
        rt.register_steps_charged() > charged,
        "the later classic did not run on the same register heap"
    );
}

#[test]
fn an_await_continuation_shares_the_turn_budget_and_is_not_replayed() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "async function burn() {\
           await null;\
           var i = 0;\
           while (i < 1000000) { i = i + 1; }\
           document.getElementById('out').textContent = 'finished';\
         }\
         burn();\
         document.getElementById('out').textContent = 'after';",
    );
    assert_eq!(text(&doc, "out"), "after", "{:?}", rt.console_logs());
    let spent = rt.page_budget().steps_used;
    assert!(
        spent > 100_000,
        "the continuation did not share the turn budget: {spent}"
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'next';",
    );
    assert_eq!(text(&doc, "out"), "next", "{:?}", rt.console_logs());
}

#[test]
fn release_drops_an_await_that_has_not_resumed() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.sync_timer_clock(1_000.0);
    rt.eval_script(
        &mut doc,
        "async function hang() {\
           await new Promise(function(r) { setTimeout(r, 50); });\
           document.getElementById('out').textContent = 'resumed';\
         }\
         hang();\
         document.getElementById('out').textContent = 'waiting';",
    );
    assert_eq!(text(&doc, "out"), "waiting");
    assert!(rt.next_timer_due_ms().is_some());
    rt.release_page_callbacks();
    assert!(rt.next_timer_due_ms().is_none());
    let poll = rt.run_due_timers(
        &mut doc,
        2_000.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 50_000,
        },
    );
    assert_eq!(poll.fired, 0);
    assert_eq!(text(&doc, "out"), "waiting");
}

#[test]
fn a_committed_document_does_not_run_script_or_fetch() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\"></button>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/app/");
    rt.register_fetch_response("https://app.example/api/mine", 200, "mine");
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'ready';",
    );
    assert_eq!(text(&doc, "out"), "ready");
    rt.commit_page_navigation();
    assert!(rt.navigation_has_committed());
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'ran';\
         fetch('https://app.example/api/mine');",
    );
    assert_eq!(text(&doc, "out"), "ready", "{:?}", rt.console_logs());
    assert!(
        rt.last_fetch_request().is_none(),
        "retired document fetched: {:?}",
        rt.last_fetch_request()
    );
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("not fully active")),
        "{:?}",
        rt.console_logs()
    );
    let button = doc.get_element_by_id("b").expect("b");
    assert!(!rt.dispatch_click(&mut doc, button));
    rt.rebind_page_document(&doc);
    assert!(!rt.navigation_has_committed());
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'rebound';",
    );
    assert_eq!(text(&doc, "out"), "rebound", "{:?}", rt.console_logs());
}

#[test]
fn release_keeps_body_reads_and_both_fetches_refuse_until_rebind() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    let calls = Arc::new(AtomicUsize::new(0));
    let pages = Arc::new(Mutex::new(Vec::new()));
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    let seen = Arc::clone(&calls);
    let seen_pages = Arc::clone(&pages);
    rt.set_fetch_host(Arc::new(move |url, page| {
        seen.fetch_add(1, Ordering::SeqCst);
        seen_pages
            .lock()
            .expect("fetch log")
            .push((url.to_string(), page.to_string()));
        anvil_script::ScriptFetchReply {
            status: 200,
            body: "from-host".to_string(),
        }
    }));
    rt.eval_script(&mut doc, "var kept = 4;");
    rt.eval_script(&mut doc, "var before = fetch('https://app.example/mine');");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "live page did not call the host"
    );
    assert_eq!(
        pages.lock().expect("pages").as_slice(),
        [(
            "https://app.example/mine".to_string(),
            "https://app.example/index.html".to_string()
        )]
    );
    rt.release_page_callbacks();
    assert!(
        !rt.navigation_has_committed(),
        "release is not a navigation commit"
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = '' + kept;",
    );
    assert_eq!(text(&doc, "out"), "4", "{:?}", rt.console_logs());
    assert!(matches!(rt.get_var("kept"), Some(anvil_script::Value::Number(n)) if *n == 4.0));
    rt.eval_script(&mut doc, "var denied = fetch('https://app.example/mine');");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "interpreter fetch on a released page called the old origin host"
    );
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'reg';\
         var denied = fetch('https://app.example/mine');\
         document.getElementById('out').textContent = 'reg:' + denied.status;",
    );
    assert!(
        text(&doc, "out").starts_with("reg"),
        "released register script did not run: {}",
        text(&doc, "out")
    );
    assert_ne!(text(&doc, "out"), "reg:200", "{:?}", rt.console_logs());
    assert!(
        rt.last_fetch_request().is_none(),
        "{:?}",
        rt.last_fetch_request()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    rt.rebind_page_document(&doc);
    assert!(
        rt.get_var("kept").is_none(),
        "rebind kept the retired global"
    );
    rt.eval_script(&mut doc, "var again = fetch('https://app.example/mine');");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    rt.eval_script(
        &mut doc,
        "var again = fetch('https://app.example/mine');\
         document.getElementById('out').textContent = 'back:' + again.status;",
    );
    assert_eq!(text(&doc, "out"), "back:200", "{:?}", rt.console_logs());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let logged = pages.lock().expect("pages");
    assert!(
        logged
            .iter()
            .all(|(_, page)| page == "https://app.example/index.html"),
        "fetch used a different page origin: {logged:?}"
    );
    assert_eq!(logged.len(), 3);
}

fn assert_relative_send_stays_on_app(rt: &mut ScriptRuntime, register: bool) {
    assert_eq!(
        rt.page_origin(),
        Some("https://app.example/app/index.html"),
        "pending intent replaced the active document URL"
    );
    assert!(
        rt.storage_origin_key().contains("app.example")
            && !rt.storage_origin_key().contains("other.example"),
        "storage origin followed the intent: {}",
        rt.storage_origin_key()
    );
    assert_eq!(
        rt.take_pending_navigation().as_deref(),
        Some("https://other.example/next")
    );
    let snapshot = rt
        .take_submission_snapshot()
        .expect("submit while the old page is still active");
    assert_eq!(
        snapshot.action, "https://app.example/send",
        "relative action joined the pending origin: {snapshot:?}"
    );
    assert_eq!(rt.submission_epoch(), 1);
    if register {
        assert!(
            rt.register_steps_charged() > 0,
            "register form did not run on the register heap"
        );
    } else {
        assert_eq!(
            rt.register_steps_charged(),
            0,
            "interpreter form opened the register heap: {:?}",
            rt.console_logs()
        );
    }
}

#[test]
fn register_cross_origin_intent_keeps_relative_form_and_fetch_on_the_old_origin() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"g\" action=\"/send\"><input name=\"q\" value=\"kept\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/app/index.html");
    rt.register_fetch_response("https://app.example/api/mine", 200, "mine");
    rt.eval_script(
        &mut doc,
        "location.href = 'https://other.example/next';\
         var same = fetch('https://app.example/api/mine');\
         var other = fetch('https://other.example/x');\
         document.getElementById('g').requestSubmit();\
         document.getElementById('out').textContent = same.status + ':' + other.status;",
    );
    assert_eq!(
        text(&doc, "out"),
        "200:undefined",
        "fetch authority followed the intent; logs={:?}",
        rt.console_logs()
    );
    assert_relative_send_stays_on_app(&mut rt, true);
    rt.commit_page_navigation();
    assert_eq!(rt.submission_epoch(), 2);
    assert_eq!(
        rt.page_origin(),
        Some("https://app.example/app/index.html"),
        "commit rewrote the old document URL to the target"
    );
    assert!(rt.take_submission_snapshot().is_none());
}

#[test]
fn interpreter_cross_origin_intent_keeps_relative_form_on_the_old_origin() {
    let mut doc =
        parse_html("<form id=\"g\" action=\"/send\"><input name=\"q\" value=\"kept\"></form>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/app/index.html");
    // The space keeps `.requestSubmit (` off the register router, so this
    // submit runs on the interpreter. The register test uses the normal call.
    rt.eval_script(
        &mut doc,
        "location.href = 'https://other.example/next';\
         document.getElementById('g').requestSubmit ();",
    );
    assert_relative_send_stays_on_app(&mut rt, false);
}

#[test]
fn a_detached_form_does_not_snapshot_and_removal_after_does() {
    let mut doc =
        parse_html("<p id=\"out\">init</p><form id=\"g\"><input name=\"q\" value=\"kept\"></form>");
    let mut rt = ScriptRuntime::new();
    let form = doc.get_element_by_id("g").expect("g");
    rt.eval_script(
        &mut doc,
        "var g = document.getElementById('g');\
         var n = 0;\
         g.remove();\
         g.addEventListener('submit', function() { n = n + 1; });\
         g.requestSubmit();\
         document.getElementById('out').textContent = '' + n;",
    );
    assert_eq!(text(&doc, "out"), "0", "{:?}", rt.console_logs());
    assert!(
        doc.get(form).parent().is_none(),
        "remove left the form connected"
    );
    assert!(
        rt.take_submission_snapshot().is_none(),
        "a disconnected form still produced a snapshot"
    );

    let mut live =
        parse_html("<p id=\"out\">init</p><form id=\"g\"><input name=\"q\" value=\"kept\"></form>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut live,
        "var g = document.getElementById('g');\
         g.addEventListener('submit', function() {\
           document.getElementById('out').textContent = 'saw';\
           g.remove();\
         });\
         g.requestSubmit();",
    );
    assert_eq!(text(&live, "out"), "saw", "{:?}", rt.console_logs());
    assert!(
        rt.take_submission_snapshot().is_none(),
        "detached during submit still planned navigation"
    );

    let mut kept = parse_html("<form id=\"g\"><input name=\"q\" value=\"kept\"></form>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut kept,
        "var g = document.getElementById('g');\
         g.requestSubmit();\
         g.remove();",
    );
    let snapshot = rt.take_submission_snapshot().expect("planned snapshot");
    assert!(format!("{snapshot:?}").contains("kept"), "{snapshot:?}");
}

#[test]
fn hydrate_refuses_a_cloned_document_without_running_it() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><script>document.getElementById('out').textContent = 'orig';</script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'orig';\
         setTimeout(function() {}, 0);",
    );
    assert_eq!(text(&doc, "out"), "orig");
    let before = rt.register_steps_charged();
    assert!(before > 0, "original did not open the register heap");
    let mut clone = doc.clone();
    assert_ne!(clone.identity(), doc.identity());
    let err = rt
        .eval_with_cinder_engine(
            &mut clone,
            "document.getElementById('out').textContent = 'clone';",
        )
        .expect_err("clone eval");
    assert!(err.contains("wrong document"), "{err}");
    rt.hydrate_interactive_bindings(&mut clone);
    assert_eq!(text(&clone, "out"), "orig");
    assert_eq!(text(&doc, "out"), "orig");
    assert_eq!(rt.register_steps_charged(), before);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'still';",
    );
    assert_eq!(text(&doc, "out"), "still");
    assert_eq!(text(&clone, "out"), "orig");
}

#[test]
fn load_checkpoints_microtasks_and_leaves_a_zero_delay_timer() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var log = 's';\
         queueMicrotask(function() {\
           log = log + 'm';\
           document.getElementById('out').textContent = log;\
         });\
         setTimeout(function() {\
           document.getElementById('out').textContent = 'timer';\
         }, 0);",
    );
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "sm", "{:?}", rt.console_logs());
    assert!(rt.has_pending_timers());
    assert!(rt.run_timers(&mut doc) > 0);
    assert_eq!(text(&doc, "out"), "timer");
}

#[test]
fn a_non_fetch_async_classic_shares_the_register_heap_and_clock() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    assert!(rt.sync_timer_clock(1_000.0));
    rt.eval_with_cinder_engine(
        &mut doc,
        "let n = 1;\
         async function go() {\
           await null;\
           n = n + 4;\
           document.getElementById('out').textContent = 'a' + n;\
           setTimeout(function() {\
             document.getElementById('out').textContent = 'due';\
           }, 10);\
         }\
         go();",
    )
    .expect("register async");
    let charged = rt.register_steps_charged();
    assert!(charged > 0 && charged < 50_000, "{charged}");
    assert!(rt.get_var("n").is_none());
    assert_eq!(text(&doc, "out"), "a5", "{:?}", rt.console_logs());
    let due = rt.next_timer_due_ms().expect("timeout queued after await");
    assert!((due - 1_010.0).abs() < 1.0, "{due}");
    let early = rt.run_due_timers(
        &mut doc,
        1_000.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert_eq!(early.fired, 0, "{early:?}");
    assert_eq!(text(&doc, "out"), "a5");
    let ready = rt.run_due_timers(
        &mut doc,
        1_010.0,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert!(ready.fired > 0, "{ready:?}");
    assert_eq!(text(&doc, "out"), "due");
}

#[test]
fn a_comment_or_string_is_not_a_module_import() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "// import { nope } from \"./missing.js\";\n\
             const note = \"import { z } from './also.js'\";\n\
             export const kept = 2;\n\
             document.getElementById('out').textContent = '' + kept;\n",
            "https://example.test/kept.js",
        )
        .expect("compile");
    assert!(
        rt.module_requests(id).expect("requests").is_empty(),
        "a comment or string was linked as an import"
    );
    rt.instantiate_module(id).expect("instantiate");
    rt.eval_module(&mut doc, id).expect("eval");
    assert_eq!(text(&doc, "out"), "2", "{:?}", rt.console_logs());
}

#[test]
fn an_import_assignment_does_not_change_the_export_and_a_local_object_is_visible() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let exporter = rt
        .compile_module(
            "var obj = { n: 4 };\n\
             export let n = 1;\n\
             export function read() { return obj.n + ':' + n; }\n\
             export function bump() { n = n + 1; }\n",
            "https://example.test/obj.js",
        )
        .expect("compile exporter");
    let importer = rt
        .compile_module(
            "import { n, read, bump } from \"https://example.test/obj.js\";\n\
             document.getElementById('out').textContent = read();\n\
             bump();\n\
             document.getElementById('out').textContent =\n\
               document.getElementById('out').textContent + '|' + read();\n",
            "https://example.test/read.js",
        )
        .expect("compile importer");
    rt.instantiate_module(importer).expect("link");
    rt.eval_module(&mut doc, importer).expect("eval");
    assert_eq!(text(&doc, "out"), "4:1|4:2", "{:?}", rt.console_logs());
    let writer = rt
        .compile_module(
            "import { n, read } from \"https://example.test/obj.js\";\n\
             n = 9;\n\
             document.getElementById('out').textContent = '' + read();\n",
            "https://example.test/write.js",
        )
        .expect("compile writer");
    rt.instantiate_module(writer).expect("link writer");
    let err = rt.eval_module(&mut doc, writer).expect_err("import write");
    let message = match err {
        ScriptError::Unsupported(message) => message,
        other => panic!("expected unsupported, got {other:?}"),
    };
    assert!(
        message.contains("Assignment to constant variable"),
        "{message}"
    );
    assert_eq!(text(&doc, "out"), "4:1|4:2", "{:?}", rt.console_logs());
    let _ = exporter;
}

#[test]
fn a_shadowed_binding_and_a_template_keep_their_lexical_meaning() {
    let mut doc = parse_html("<p id=\"out\">init</p><p id=\"mark\">init</p>");
    let mut rt = ScriptRuntime::new();
    let exporter = rt
        .compile_module("export let n = 5;\n", "https://example.test/n.js")
        .expect("compile n");
    let importer = rt
        .compile_module(
            "import { n } from \"https://example.test/n.js\";\n\
             function take(n) { return n; }\n\
             function caught(v) { try { throw v; } catch (n) { return n; } }\n\
             function block() { let n = 3; return n; }\n\
             var text = `n=${n}`;\n\
             document.getElementById('out').textContent =\n\
               take(9) + ':' + caught(8) + ':' + block() + ':' + text;\n",
            "https://example.test/shadow.js",
        )
        .expect("compile shadow");
    rt.instantiate_module(importer).expect("link");
    rt.eval_module(&mut doc, importer).expect("eval");
    assert_eq!(text(&doc, "out"), "9:8:3:n=5", "{:?}", rt.console_logs());
    rt.eval_script(
        &mut doc,
        "var leaked = 0; try { __anvil_open_box(1); leaked = 1; } catch (e) { leaked = 2; }\n\
         document.getElementById('mark').textContent = '' + leaked;",
    );
    assert_eq!(text(&doc, "mark"), "2", "{:?}", rt.console_logs());
    let _ = exporter;
}

#[test]
fn a_duplicate_named_import_is_rejected_before_either_module_runs() {
    let doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let left = rt
        .compile_module(
            "export let n = 1;\n\
             document.getElementById('out').textContent = 'left';\n",
            "https://example.test/left.js",
        )
        .expect("compile left");
    let right = rt
        .compile_module(
            "export let n = 2;\n\
             document.getElementById('out').textContent = 'right';\n",
            "https://example.test/right.js",
        )
        .expect("compile right");
    let err = rt
        .compile_module(
            "import { n } from \"https://example.test/left.js\";\n\
             import { n } from \"https://example.test/right.js\";\n\
             document.getElementById('out').textContent = 'DOMdup:' + n;\n",
            "https://example.test/dup.js",
        )
        .expect_err("duplicate local");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("duplicate import binding 'n'")
                    && message.contains("before evaluation"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(left).expect("left").phase,
        anvil_script::ModulePhase::Compiled
    );
    assert_eq!(
        rt.module_status(right).expect("right").phase,
        anvil_script::ModulePhase::Compiled
    );
}

#[test]
fn an_alias_namespace_reexport_and_default_stay_live() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let base = rt
        .compile_module(
            "export let n = 1;\n\
             export default 8;\n\
             export function bump() { n = n + 1; }\n",
            "https://example.test/base.js",
        )
        .expect("compile base");
    let mid = rt
        .compile_module(
            "export { n as p } from \"https://example.test/base.js\";\n\
             export * from \"https://example.test/base.js\";\n",
            "https://example.test/mid.js",
        )
        .expect("compile mid");
    let user = rt
        .compile_module(
            "import { p, bump } from \"https://example.test/mid.js\";\n\
             import { n as m } from \"https://example.test/base.js\";\n\
             import * as ns from \"https://example.test/base.js\";\n\
             import def from \"https://example.test/base.js\";\n\
             document.getElementById('out').textContent =\n\
               p + ':' + m + ':' + ns.n + ':' + def;\n\
             bump();\n\
             document.getElementById('out').textContent =\n\
               document.getElementById('out').textContent + ':' + p + ':' + ns.n;\n",
            "https://example.test/use.js",
        )
        .expect("compile use");
    assert_eq!(text(&doc, "out"), "init");
    rt.instantiate_module(user).expect("link");
    assert_eq!(text(&doc, "out"), "init", "instantiate ran a body");
    rt.eval_module(&mut doc, user).expect("eval");
    assert_eq!(text(&doc, "out"), "1:1:1:8:2:2", "{:?}", rt.console_logs());
    let writer = rt
        .compile_module(
            "import { m } from \"https://example.test/use-m.js\";\n",
            "https://example.test/unused.js",
        )
        .expect("compile");
    let _ = (base, mid, writer);
    let assign = rt
        .compile_module(
            "import { n as m } from \"https://example.test/base.js\";\n\
             m = 9;\n",
            "https://example.test/assign.js",
        )
        .expect("compile assign");
    rt.instantiate_module(assign).expect("link assign");
    let err = rt.eval_module(&mut doc, assign).expect_err("readonly");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("Assignment to constant variable"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "1:1:1:8:2:2");
    let again = rt.eval_module(&mut doc, assign).expect_err("cached");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("Assignment to constant variable"),
                "{message}"
            );
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(
        text(&doc, "out"),
        "1:1:1:8:2:2",
        "readonly failure ran again"
    );
}

#[test]
fn module_cycle_evaluates_once_and_a_compiled_module_evaluates_on_the_register_vm() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let a = rt
        .compile_module(
            "import { b } from \"https://example.test/b.js\";\n\
             export function a() { return b(); }\n\
             document.getElementById('out').textContent += 'A' + a();\n",
            "https://example.test/a.js",
        )
        .expect("compile a");
    let b = rt
        .compile_module(
            "import { a } from \"https://example.test/a.js\";\n\
             export function b() { return 4; }\n\
             document.getElementById('out').textContent += 'B';\n",
            "https://example.test/b.js",
        )
        .expect("compile b");
    rt.instantiate_module(a).expect("cycle links");
    assert_eq!(text(&doc, "out"), "init");
    rt.eval_module(&mut doc, a).expect("cycle eval");
    assert_eq!(text(&doc, "out"), "initBA4", "{:?}", rt.console_logs());
    rt.eval_module(&mut doc, a)
        .expect("second eval is cached success");
    assert_eq!(text(&doc, "out"), "initBA4");
    assert_eq!(
        rt.module_status(b).expect("b").phase,
        anvil_script::ModulePhase::Evaluated
    );
    let id = rt
        .compile_module(
            "var m = 3;\n\
             document.getElementById('out').textContent = '' + m;\n",
            "https://example.test/c.js",
        )
        .expect("compile c");
    assert!(rt.module_requests(id).expect("requests").is_empty());
    rt.instantiate_module(id).expect("instantiate");
    rt.eval_module(&mut doc, id).expect("eval");
    assert_eq!(text(&doc, "out"), "3", "{:?}", rt.console_logs());
    let missing = rt
        .import_dynamic(&mut doc, "https://example.test/missing.js", id)
        .expect_err("fetch");
    match missing {
        ScriptError::NeedsFetch { specifier, from } => {
            assert_eq!(specifier, "https://example.test/missing.js");
            assert_eq!(from, id);
        }
        other => panic!("expected NeedsFetch, got {other:?}"),
    }
}

#[test]
fn a_live_export_updates_the_importer_in_the_same_evaluation() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let exporter = rt
        .compile_module(
            "export let n = 1;\n\
             export function bump() {\n\
               n = n + 1;\n\
             }\n",
            "https://example.test/cell.js",
        )
        .expect("compile exporter");
    let importer = rt
        .compile_module(
            "import { n, bump } from \"https://example.test/cell.js\";\n\
             document.getElementById('out').textContent = '' + n;\n\
             bump();\n\
             document.getElementById('out').textContent =\n\
               document.getElementById('out').textContent + ':' + n;\n",
            "https://example.test/app.js",
        )
        .expect("compile importer");
    rt.instantiate_module(importer).expect("link");
    rt.eval_module(&mut doc, importer).expect("eval");
    assert_eq!(text(&doc, "out"), "1:2", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(exporter).expect("exporter").phase,
        anvil_script::ModulePhase::Evaluated
    );
    let _ = exporter;
}

#[test]
fn a_legal_indirect_cycle_evaluates_and_a_missing_export_stays_compiled() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let a = rt
        .compile_module(
            "import \"https://example.test/b.js\";\n",
            "https://example.test/a.js",
        )
        .expect("compile a");
    let _b = rt
        .compile_module(
            "import \"https://example.test/c.js\";\n",
            "https://example.test/b.js",
        )
        .expect("compile b");
    let _c = rt
        .compile_module(
            "import \"https://example.test/a.js\";\n",
            "https://example.test/c.js",
        )
        .expect("compile c");
    rt.instantiate_module(a).expect("indirect cycle links");
    rt.eval_module(&mut doc, a)
        .expect("indirect cycle evaluates");
    assert_eq!(
        rt.module_status(a).expect("a").phase,
        anvil_script::ModulePhase::Evaluated
    );
    assert_eq!(text(&doc, "out"), "init");

    let source = rt
        .compile_module("export var n = 1;\n", "https://example.test/named.js")
        .expect("compile named");
    let buyer = rt
        .compile_module(
            "import { missing } from \"https://example.test/named.js\";\n\
             document.getElementById('out').textContent = 'ran';\n",
            "https://example.test/buyer.js",
        )
        .expect("compile buyer");
    let missing = rt.instantiate_module(buyer).expect_err("missing export");
    match missing {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("does not export missing"), "{message}");
        }
        other => panic!("expected missing export, got {other:?}"),
    }
    assert_eq!(
        rt.module_status(buyer).expect("buyer").phase,
        anvil_script::ModulePhase::Compiled
    );
    assert_eq!(
        rt.module_status(source).expect("named").phase,
        anvil_script::ModulePhase::Compiled
    );
    assert_eq!(text(&doc, "out"), "init");
}

fn resolve_page_specifier(base: &str, specifier: &str) -> Option<String> {
    if specifier == "./dep.js" && base.ends_with("/a/main.js") {
        return Some("https://example.test/a/dep.js".to_string());
    }
    if specifier == "./dep.js" && base.ends_with("/b/main.js") {
        return Some("https://example.test/b/dep.js".to_string());
    }
    if specifier == "cdn:dep" {
        return Some("https://example.test/a/dep.js".to_string());
    }
    None
}

#[test]
fn relative_specifiers_resolve_from_the_referring_module_and_aliases_run_once() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_module_specifier_resolver(resolve_page_specifier);
    let dep_a = rt
        .compile_module(
            "document.getElementById('out').textContent += 'A';\n",
            "https://example.test/a/dep.js",
        )
        .expect("dep a");
    let again = rt
        .compile_module(
            "document.getElementById('out').textContent += 'A';\n",
            "https://example.test/a/dep.js",
        )
        .expect("same canonical module");
    assert_eq!(again, dep_a);
    let _dep_b = rt
        .compile_module(
            "document.getElementById('out').textContent += 'B';\n",
            "https://example.test/b/dep.js",
        )
        .expect("dep b");
    let main_a = rt
        .compile_module("import \"./dep.js\";\n", "https://example.test/a/main.js")
        .expect("main a");
    let main_b = rt
        .compile_module("import \"./dep.js\";\n", "https://example.test/b/main.js")
        .expect("main b");
    let alias = rt
        .compile_module("import \"cdn:dep\";\n", "https://example.test/alias.js")
        .expect("alias");
    assert_eq!(
        rt.resolve_module_specifier(main_a, "./dep.js")
            .expect("resolve a"),
        "https://example.test/a/dep.js"
    );
    assert_eq!(
        rt.resolve_module_specifier(main_b, "./dep.js")
            .expect("resolve b"),
        "https://example.test/b/dep.js"
    );
    rt.instantiate_module(main_a).expect("link a");
    rt.instantiate_module(main_b).expect("link b");
    rt.instantiate_module(alias).expect("link alias");
    rt.eval_module(&mut doc, main_a).expect("eval a");
    rt.eval_module(&mut doc, main_b).expect("eval b");
    assert_eq!(text(&doc, "out"), "initAB", "{:?}", rt.console_logs());
    rt.eval_module(&mut doc, alias).expect("alias reuses a");
    assert_eq!(text(&doc, "out"), "initAB");
}

#[test]
fn a_namespace_is_not_extensible_and_its_exports_stay_live() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let base = rt
        .compile_module(
            "export let n = 1;\n\
             export function bump() { n = n + 1; }\n",
            "https://example.test/ns-base.js",
        )
        .expect("base");
    let user = rt
        .compile_module(
            "import * as ns from \"https://example.test/ns-base.js\";\n\
             var added = false;\n\
             try { ns.zz = 7; added = true; } catch (e) { added = false; }\n\
             var removed = false;\n\
             try { removed = delete ns.n; } catch (e) { removed = false; }\n\
             document.getElementById('out').textContent =\n\
               Object.isExtensible(ns) + ':' + added + ':' + removed + ':' + ns.n;\n\
             ns.bump();\n\
             document.getElementById('out').textContent += ':' + ns.n;\n",
            "https://example.test/ns-user.js",
        )
        .expect("user");
    rt.instantiate_module(user).expect("link");
    rt.eval_module(&mut doc, user).expect("eval");
    assert_eq!(
        text(&doc, "out"),
        "false:false:false:1:2",
        "{:?}",
        rt.console_logs()
    );
    let _ = base;
}

#[test]
fn a_namespace_define_property_reports_failure_and_keeps_a_compatible_export() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let _base = rt
        .compile_module(
            "export let n = 1;\n\
             export function bump() { n = n + 1; }\n",
            "https://example.test/ns-def-base.js",
        )
        .expect("base");
    let user = rt
        .compile_module(
            "import * as ns from \"https://example.test/ns-def-base.js\";\n\
             function step(name, desc) {\n\
               try { Object.defineProperty(ns, name, desc); return 'ok'; }\n\
               catch (e) { return 'throw'; }\n\
             }\n\
             var d = Object.getOwnPropertyDescriptor(ns, 'n');\n\
             var parts = [\n\
               Object.getPrototypeOf(ns) === null,\n\
               'toString' in ns,\n\
               'n' in ns,\n\
               step('zz', { value: 7 }),\n\
               Reflect.defineProperty(ns, 'yy', { value: 8 }),\n\
               Object.getOwnPropertyDescriptor(ns, 'zz') === undefined,\n\
               Object.getOwnPropertyDescriptor(ns, 'yy') === undefined,\n\
               d.value, d.writable, d.enumerable, d.configurable, d.get === undefined,\n\
               step('n', { value: ns.n, writable: true, enumerable: true, configurable: false }),\n\
               step('n', {}),\n\
               step('n', { enumerable: true }),\n\
               step('n', { writable: true }),\n\
               step('n', { configurable: false }),\n\
               step('n', { value: 99 }),\n\
               step('n', { enumerable: false }),\n\
               step('n', { writable: false }),\n\
               step('n', { configurable: true }),\n\
               step('n', { get: function () { return 0; } })\n\
             ];\n\
             var proto = 'throw';\n\
             try { Object.setPrototypeOf(ns, Object.prototype); proto = 'ok'; } catch (e) {}\n\
             parts.push(proto);\n\
             parts.push(Object.setPrototypeOf(ns, null) === ns);\n\
             parts.push(Reflect.setPrototypeOf(ns, {}));\n\
             parts.push(Reflect.setPrototypeOf(ns, null));\n\
             ns.bump();\n\
             parts.push(ns.n);\n\
             parts.push(step('n', { value: 1 }));\n\
             parts.push(step('n', { value: ns.n }));\n\
             document.getElementById('out').textContent = parts.join(':');\n",
            "https://example.test/ns-def-user.js",
        )
        .expect("user");
    rt.instantiate_module(user).expect("link");
    rt.eval_module(&mut doc, user).expect("eval");
    assert_eq!(
        text(&doc, "out"),
        "true:false:true:throw:false:true:true:1:true:true:false:true:ok:ok:ok:ok:ok:throw:throw:throw:throw:throw:throw:true:false:true:2:throw:ok",
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn the_host_resolver_owns_canonical_identity_and_path_case_stays_distinct() {
    let mut rt = ScriptRuntime::new();
    let page = rt
        .compile_module("import './still.js';\n", "https://example.test/page.js")
        .expect("page");
    match rt.resolve_module_specifier(page, "./still.js") {
        Err(ScriptError::NeedsFetch { specifier, from }) => {
            assert_eq!(specifier, "./still.js");
            assert_eq!(from, page);
        }
        other => panic!("relative specifier must not be rewritten, got {other:?}"),
    }
    fn resolve(base: &str, specifier: &str) -> Option<String> {
        if base != "https://example.test/page.js" {
            return None;
        }
        match specifier {
            "./still.js" => Some("https://example.test/lib/still.js".to_string()),
            "./Still.js" => Some("https://example.test/lib/Still.js".to_string()),
            "cdn:still" => Some("https://cdn.test/still.js".to_string()),
            "./raw.js" => Some("./raw.js".to_string()),
            _ => None,
        }
    }
    rt.set_module_specifier_resolver(resolve);
    assert_eq!(
        rt.resolve_module_specifier(page, "./still.js")
            .expect("host canonical url"),
        "https://example.test/lib/still.js"
    );
    assert_eq!(
        rt.resolve_module_specifier(page, "./Still.js")
            .expect("host keeps path case"),
        "https://example.test/lib/Still.js"
    );
    assert_eq!(
        rt.resolve_module_specifier(page, "cdn:still")
            .expect("host alias"),
        "https://cdn.test/still.js"
    );
    assert_eq!(
        rt.resolve_module_specifier(page, "./raw.js")
            .expect("host may return the specifier unchanged"),
        "./raw.js"
    );
    match rt.resolve_module_specifier(page, "./other.js") {
        Err(ScriptError::NeedsFetch { specifier, from }) => {
            assert_eq!(specifier, "./other.js");
            assert_eq!(from, page);
        }
        other => panic!("refusal keeps the raw specifier, got {other:?}"),
    }
    let lower = rt
        .compile_module("export let n = 1;\n", "https://example.test/lib/still.js")
        .expect("lower");
    let upper = rt
        .compile_module("export let n = 2;\n", "https://example.test/lib/Still.js")
        .expect("upper");
    assert_ne!(lower, upper);
    let raw = rt
        .compile_module("export let n = 3;\n", "./raw.js")
        .expect("raw key");
    assert_ne!(raw, lower);
}

#[test]
fn a_linked_relative_key_executes_and_stays_frozen_when_the_resolver_changes() {
    STILL_RESOLVE.store(0, Ordering::SeqCst);
    let mut rt = ScriptRuntime::new();
    let empty = rt
        .compile_module("export let n = 1;\n", " ")
        .expect_err("empty base");
    match empty {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("base URL"), "{message}");
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    let main = rt
        .compile_module(
            "import * as ns from './still.js';\n\
             document.getElementById('out').textContent += 'M' + ns.n;\n",
            "https://example.test/page.js",
        )
        .expect("main");
    match rt.instantiate_module(main).expect_err("no resolver") {
        ScriptError::NeedsFetch { specifier, from } => {
            assert_eq!(specifier, "./still.js");
            assert_eq!(from, main);
        }
        other => panic!("expected the original specifier, got {other:?}"),
    }
    STILL_RESOLVE.store(0, Ordering::SeqCst);
    rt.set_module_specifier_resolver(resolve_still);
    match rt.instantiate_module(main).expect_err("resolver none") {
        ScriptError::NeedsFetch { specifier, .. } => assert_eq!(specifier, "./still.js"),
        other => panic!("none returns the original text, got {other:?}"),
    }
    STILL_RESOLVE.store(1, Ordering::SeqCst);
    match rt.instantiate_module(main).expect_err("not compiled") {
        ScriptError::NeedsFetch { specifier, .. } => assert_eq!(specifier, "./still.js"),
        other => panic!("absent target is NeedsFetch, got {other:?}"),
    }
    let still = rt
        .compile_module(
            "export let n = 1;\n\
             document.getElementById('out').textContent += 'A';\n",
            "./still.js",
        )
        .expect("relative base");
    let other = rt
        .compile_module(
            "export let n = 2;\n\
             document.getElementById('out').textContent += 'B';\n",
            "./other.js",
        )
        .expect("other");
    let upper = rt
        .compile_module("export let n = 9;\n", "./Still.js")
        .expect("path case");
    assert_ne!(still, other);
    assert_ne!(still, upper);
    rt.instantiate_module(main).expect("link the relative key");
    STILL_RESOLVE.store(2, Ordering::SeqCst);
    let mut doc = parse_html("<p id=\"out\">init</p>");
    rt.eval_module(&mut doc, main)
        .expect("eval keeps the linked module");
    assert_eq!(text(&doc, "out"), "initAM1", "{:?}", rt.console_logs());
    rt.eval_module(&mut doc, main).expect("no replay");
    assert_eq!(text(&doc, "out"), "initAM1");
    STILL_RESOLVE.store(0, Ordering::SeqCst);
    let _ = (still, other, upper);
}

#[test]
fn a_raw_relative_specifier_is_not_a_shared_module_identity() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let raw = rt
        .compile_module(
            "document.getElementById('out').textContent += 'D';\n",
            "./dep.js",
        )
        .expect("raw compile is not resolution");
    let left = rt
        .compile_module("import \"./dep.js\";\n", "https://example.test/a/main.js")
        .expect("left");
    let right = rt
        .compile_module("import \"./dep.js\";\n", "https://example.test/b/main.js")
        .expect("right");
    let left_err = rt
        .instantiate_module(left)
        .expect_err("left needs a resolver");
    let right_err = rt
        .instantiate_module(right)
        .expect_err("right needs a resolver");
    match (left_err, right_err) {
        (
            ScriptError::NeedsFetch {
                specifier: a,
                from: from_a,
            },
            ScriptError::NeedsFetch {
                specifier: b,
                from: from_b,
            },
        ) => {
            assert_eq!(a, "./dep.js");
            assert_eq!(b, "./dep.js");
            assert_eq!(from_a, left);
            assert_eq!(from_b, right);
        }
        other => panic!("expected two NeedsFetch results, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
    let _ = raw;
    let upper = rt
        .compile_module(
            "document.getElementById('out').textContent += 'U';\n",
            "HTTPS://example.test/Dep.js",
        )
        .expect("upper");
    let lower = rt
        .compile_module(
            "document.getElementById('out').textContent += 'L';\n",
            "https://example.test/dep.js",
        )
        .expect("lower");
    assert_ne!(upper, lower);
    let clash = rt
        .compile_module(
            "document.getElementById('out').textContent += 'Z';\n",
            "https://example.test/dep.js",
        )
        .expect_err("a second source for the same URL is not a new module");
    match clash {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("already compiled"), "{message}");
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    rt.instantiate_module(upper).expect("link upper");
    rt.instantiate_module(lower).expect("link lower");
    rt.eval_module(&mut doc, upper).expect("eval upper");
    rt.eval_module(&mut doc, lower).expect("eval lower");
    assert_eq!(text(&doc, "out"), "initUL", "{:?}", rt.console_logs());
}

#[test]
fn top_level_await_does_not_run_past_an_unsettled_await() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "var resolve;\n\
             globalThis.__release = function () { resolve(); };\n\
             var gate = new Promise(function (r) { resolve = r; });\n\
             document.getElementById('out').textContent += 'before';\n\
             await gate;\n\
             document.getElementById('out').textContent += 'after';\n",
            "https://example.test/await.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id)
        .expect("evaluation parks without an error");
    assert_eq!(text(&doc, "out"), "initbefore", "{:?}", rt.console_logs());
    let parked = rt.module_status(id).expect("status");
    assert_eq!(parked.phase, anvil_script::ModulePhase::Evaluating);
    assert!(parked.error.is_none());
    rt.eval_module(&mut doc, id)
        .expect("a second eval does not replay or finish the body");
    assert_eq!(text(&doc, "out"), "initbefore");
    rt.eval_script(&mut doc, "__release();");
    assert_eq!(
        text(&doc, "out"),
        "initbeforeafter",
        "{:?}",
        rt.console_logs()
    );
    assert_eq!(
        rt.module_status(id).expect("done").phase,
        anvil_script::ModulePhase::Evaluated
    );
    rt.eval_module(&mut doc, id).expect("no second run");
    assert_eq!(text(&doc, "out"), "initbeforeafter");
}

#[test]
fn a_settled_top_level_await_finishes_in_the_evaluation_turn() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "document.getElementById('out').textContent += 'before';\n\
             await null;\n\
             document.getElementById('out').textContent += 'after';\n",
            "https://example.test/settled.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id)
        .expect("settled await still returns before the continuation");
    assert_eq!(text(&doc, "out"), "initbefore", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(id).expect("parked").phase,
        anvil_script::ModulePhase::Evaluating
    );
    rt.checkpoint_microtasks(&mut doc);
    assert_eq!(
        text(&doc, "out"),
        "initbeforeafter",
        "{:?}",
        rt.console_logs()
    );
    assert_eq!(
        rt.module_status(id).expect("evaluated").phase,
        anvil_script::ModulePhase::Evaluated
    );
}

#[test]
fn a_rejected_top_level_await_is_cached_at_the_checkpoint() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "document.getElementById('out').textContent += 'before';\n\
             await Promise.reject(new Error('nope'));\n\
             document.getElementById('out').textContent += 'after';\n",
            "https://example.test/reject.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id)
        .expect("parks on the rejection");
    assert_eq!(text(&doc, "out"), "initbefore");
    rt.checkpoint_microtasks(&mut doc);
    assert_eq!(text(&doc, "out"), "initbefore");
    let status = rt.module_status(id).expect("status");
    assert_eq!(status.phase, anvil_script::ModulePhase::Instantiated);
    let message = status.error.expect("cached rejection");
    assert!(message.contains("nope"), "{message}");
    let again = rt.eval_module(&mut doc, id).expect_err("same failure");
    match again {
        ScriptError::Unsupported(message) => assert!(message.contains("nope"), "{message}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initbefore");
}

#[test]
fn a_dependent_module_waits_for_an_evaluating_dependency() {
    fn resolve(base: &str, specifier: &str) -> Option<String> {
        if specifier == "./dep.js" && base == "https://example.test/main.js" {
            Some("https://example.test/dep.js".to_string())
        } else {
            None
        }
    }
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_module_specifier_resolver(resolve);
    let dep = rt
        .compile_module(
            "document.getElementById('out').textContent += 'D1';\n\
             await null;\n\
             document.getElementById('out').textContent += 'D2';\n",
            "https://example.test/dep.js",
        )
        .expect("dep");
    let main = rt
        .compile_module(
            "import './dep.js';\n\
             document.getElementById('out').textContent += 'M';\n",
            "https://example.test/main.js",
        )
        .expect("main");
    rt.instantiate_module(dep).expect("link dep");
    rt.instantiate_module(main).expect("link main");
    rt.eval_module(&mut doc, main).expect("main waits");
    assert_eq!(text(&doc, "out"), "initD1", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(dep).expect("dep").phase,
        anvil_script::ModulePhase::Evaluating
    );
    assert_eq!(
        rt.module_status(main).expect("main").phase,
        anvil_script::ModulePhase::Evaluating
    );
    rt.checkpoint_microtasks(&mut doc);
    assert_eq!(text(&doc, "out"), "initD1D2M", "{:?}", rt.console_logs());
    assert_eq!(
        rt.module_status(dep).expect("dep done").phase,
        anvil_script::ModulePhase::Evaluated
    );
    assert_eq!(
        rt.module_status(main).expect("main done").phase,
        anvil_script::ModulePhase::Evaluated
    );
}

#[test]
fn release_cancels_a_pending_module_evaluation() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "var resolve;\n\
             globalThis.__release = function () { resolve(); };\n\
             var gate = new Promise(function (r) { resolve = r; });\n\
             document.getElementById('out').textContent += 'before';\n\
             await gate;\n\
             document.getElementById('out').textContent += 'after';\n",
            "https://example.test/cancel.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id).expect("parked");
    rt.release_page_callbacks();
    rt.checkpoint_microtasks(&mut doc);
    assert_eq!(text(&doc, "out"), "initbefore");
    let status = rt.module_status(id).expect("status");
    assert_eq!(status.phase, anvil_script::ModulePhase::Instantiated);
    let message = status.error.expect("cancelled");
    assert!(message.contains("not fully active"), "{message}");
    let again = rt.eval_module(&mut doc, id).expect_err("stays cancelled");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("not fully active"), "{message}")
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initbefore");
}

#[test]
fn a_wrong_document_does_not_cache_a_module_failure() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut other = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(&mut doc, "var bound = 1;");
    let id = rt
        .compile_module(
            "document.getElementById('out').textContent = 'M';\n",
            "https://example.test/page.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    let err = rt.eval_module(&mut other, id).expect_err("other document");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("wrong document for this page runtime"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&other, "out"), "init");
    rt.eval_module(&mut doc, id)
        .expect("real document still evaluates");
    assert_eq!(text(&doc, "out"), "M", "{:?}", rt.console_logs());
    rt.eval_module(&mut doc, id).expect("success is cached");
    assert_eq!(text(&doc, "out"), "M");
}

#[test]
fn a_non_dom_async_call_suspends_on_the_register_heap() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let before = rt.register_steps_charged();
    rt.eval_script(
        &mut doc,
        "async function f() { console.log('a'); await null; console.log('c'); }\n\
         f();\n\
         console.log('b');\n\
         async function bad() { await null; throw new Error('nope'); }\n\
         bad().catch(function (e) { console.log('caught ' + e.message); });\n\
         console.log('after');",
    );
    assert!(
        rt.register_steps_charged() > before,
        "non-DOM async did not charge the register heap"
    );
    assert!(
        rt.get_var("f").is_none(),
        "async function fell back to the interpreter"
    );
    assert_eq!(
        rt.console_logs(),
        ["a", "b", "after", "c", "caught nope"],
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn an_async_generator_does_not_run_its_body_until_next() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "async function* g() {\n\
           console.log('body');\n\
           yield 1;\n\
           console.log('after');\n\
           yield await 2;\n\
         }\n\
         globalThis.it = g();\n\
         console.log('made');\n\
         it.next().then(function (r) { console.log('n' + r.value + ':' + r.done); });",
    );
    assert_eq!(
        rt.console_logs(),
        ["made", "body", "n1:false"],
        "construction or the first next ran past the yield: {:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "it.next().then(function (r) { console.log('n' + r.value + ':' + r.done); });",
    );
    assert_eq!(
        rt.console_logs(),
        ["made", "body", "n1:false", "after", "n2:false"],
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn an_async_generator_queues_next_until_the_current_await_yields() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let before = rt.register_steps_charged();
    rt.eval_script(
        &mut doc,
        "var go;\n\
         var gate = new Promise(function (r) { go = r; });\n\
         async function* g() {\n\
           console.log('start');\n\
           var x = yield await gate;\n\
           console.log('resume ' + x);\n\
           yield 9;\n\
         }\n\
         var it = g();\n\
         var p1 = it.next();\n\
         var p2 = it.next();\n\
         p1.then(function (r) { console.log('n1:' + r.value + ':' + r.done); });\n\
         p2.then(function (r) { console.log('n2:' + r.value + ':' + r.done); });\n\
         console.log('queued');\n\
         go(7);",
    );
    assert!(
        rt.register_steps_charged().saturating_sub(before) < 50_000,
        "generator exhaustion burned the step budget: {}",
        rt.register_steps_charged()
    );
    assert_eq!(
        rt.console_logs(),
        [
            "start",
            "queued",
            "n1:7:false",
            "resume undefined",
            "n2:9:false"
        ],
        "{:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "it.next().then(function (r) { console.log('n3:' + r.value + ':' + r.done); });",
    );
    assert_eq!(
        rt.console_logs().last().map(String::as_str),
        Some("n3:undefined:true")
    );
    assert!(rt.register_steps_charged() < 200_000);
}

#[test]
fn a_classic_for_await_outside_async_is_a_syntax_error() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "for await (const x of [1]) { document.getElementById('out').textContent += x; }",
    );
    assert_eq!(text(&doc, "out"), "init");
    let logs = rt.console_logs().join("\n");
    assert!(
        logs.contains("SyntaxError") && logs.contains("for await"),
        "{logs}"
    );
}

#[test]
fn an_async_generator_awaits_a_yielded_promise_and_throws_its_rejection() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "function kind(v) {\
           return (v && typeof v.then === 'function') ? 'thenable' : String(v);\
         }\
         async function* settled() {\
           yield Promise.resolve(4);\
           yield await Promise.resolve(5);\
         }\
         globalThis.s = settled();\
         s.next().then(function (r) { console.log('settled:' + kind(r.value) + ':' + r.done); });",
    );
    assert_eq!(
        rt.console_logs(),
        ["settled:4:false"],
        "{:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "var go;\
         async function* later() {\
           var x = yield new Promise(function (r) { go = r; });\
           console.log('resumed ' + x);\
           yield 9;\
         }\
         var it = later();\
         var first = it.next();\
         var second = it.next(7);\
         first.then(function (r) { console.log('n1:' + kind(r.value) + ':' + r.done); });\
         second.then(function (r) { console.log('n2:' + kind(r.value) + ':' + r.done); });\
         console.log('queued');\
         go(4);",
    );
    assert_eq!(
        rt.console_logs(),
        [
            "settled:4:false",
            "queued",
            "n1:4:false",
            "resumed 7",
            "n2:9:false",
        ],
        "{:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "async function* caught() {\
           try {\
             yield Promise.reject(new Error('nope'));\
             console.log('after-yield');\
           } catch (e) {\
             console.log('caught ' + e.message);\
             yield 3;\
           }\
         }\
         caught().next().then(function (r) { console.log('c:' + kind(r.value) + ':' + r.done); });",
    );
    assert_eq!(
        rt.console_logs().last().map(String::as_str),
        Some("c:3:false"),
        "{:?}",
        rt.console_logs()
    );
    assert!(!rt.console_logs().iter().any(|line| line == "after-yield"));
    let before = rt.register_steps_charged();
    rt.eval_script(
        &mut doc,
        "async function* failed() {\
           console.log('body');\
           yield Promise.reject(new Error('boom'));\
           console.log('replay');\
         }\
         globalThis.bad = failed();\
         var bad1 = bad.next();\
         var bad2 = bad.next();\
         bad1.then(function () { console.log('bad-ok'); }, function (e) { console.log('rej ' + e.message); });\
         bad2.then(function (r) { console.log('tail:' + r.done + ':' + kind(r.value)); });",
    );
    assert!(
        rt.register_steps_charged().saturating_sub(before) < 50_000,
        "rejected yield spun until the step budget: {}",
        rt.register_steps_charged()
    );
    assert_eq!(
        &rt.console_logs()[rt.console_logs().len() - 3..],
        ["body", "rej boom", "tail:true:undefined"],
        "{:?}",
        rt.console_logs()
    );
    rt.eval_script(
        &mut doc,
        "s.next().then(function (r) { console.log('next5:' + kind(r.value) + ':' + r.done); });\
         bad.next().then(function (r) { console.log('again:' + r.done); });",
    );
    let logs = rt.console_logs();
    assert!(logs.iter().any(|line| line == "next5:5:false"), "{logs:?}");
    assert!(logs.iter().any(|line| line == "again:true"), "{logs:?}");
    assert!(!logs.iter().any(|line| line == "replay"));
}

#[test]
fn a_module_environment_name_is_not_a_source_binding() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "var __anvil_mod_env = 1;\n\
             export let n = 5;\n\
             function take(__anvil_mod_env) { return __anvil_mod_env; }\n\
             function read() { return n; }\n\
             document.getElementById('out').textContent =\n\
               read() + ':' + __anvil_mod_env + ':' + take(9);\n",
            "https://example.test/env.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id).expect("eval");
    assert_eq!(text(&doc, "out"), "5:1:9", "{:?}", rt.console_logs());
}

#[test]
fn a_cycle_that_reads_an_uninitialized_let_fails_once() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let a = rt
        .compile_module(
            "import { b } from \"https://example.test/tdz-b.js\";\n\
             export let a = b + 1;\n\
             document.getElementById('out').textContent += 'A';\n",
            "https://example.test/tdz-a.js",
        )
        .expect("compile a");
    let _b = rt
        .compile_module(
            "import { a } from \"https://example.test/tdz-a.js\";\n\
             document.getElementById('out').textContent += 'B';\n\
             export let b = a;\n",
            "https://example.test/tdz-b.js",
        )
        .expect("compile b");
    rt.instantiate_module(a).expect("link");
    let err = rt.eval_module(&mut doc, a).expect_err("tdz");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("binding is not initialized"), "{message}");
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initB", "{:?}", rt.console_logs());
    let again = rt.eval_module(&mut doc, a).expect_err("cached tdz");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("binding is not initialized"), "{message}");
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initB");
    let imported = rt
        .import_dynamic(&mut doc, "https://example.test/tdz-b.js", a)
        .expect_err("import returns the same failure");
    match imported {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("binding is not initialized"), "{message}");
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initB");
}

#[test]
fn an_ambiguous_star_is_rejected_before_evaluation() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let left = rt
        .compile_module(
            "export let n = 1;\n\
             document.getElementById('out').textContent = 'L';\n",
            "https://example.test/star-a.js",
        )
        .expect("left");
    let right = rt
        .compile_module(
            "export let n = 2;\n\
             document.getElementById('out').textContent = 'R';\n",
            "https://example.test/star-b.js",
        )
        .expect("right");
    let both = rt
        .compile_module(
            "export * from \"https://example.test/star-a.js\";\n\
             export * from \"https://example.test/star-b.js\";\n",
            "https://example.test/star-c.js",
        )
        .expect("both");
    let err = rt.instantiate_module(both).expect_err("ambiguous");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("ambiguous export 'n'") && message.contains("before evaluation"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
    assert_eq!(
        rt.module_status(left).expect("left").phase,
        anvil_script::ModulePhase::Compiled
    );
    assert_eq!(
        rt.module_status(right).expect("right").phase,
        anvil_script::ModulePhase::Compiled
    );
    let local = rt
        .compile_module(
            "export let n = 3;\n\
             export * from \"https://example.test/star-a.js\";\n\
             export * from \"https://example.test/star-b.js\";\n",
            "https://example.test/star-d.js",
        )
        .expect("local wins");
    let reader = rt
        .compile_module(
            "import { n } from \"https://example.test/star-d.js\";\n\
             document.getElementById('out').textContent = '' + n;\n",
            "https://example.test/star-e.js",
        )
        .expect("reader");
    rt.instantiate_module(reader).expect("link local");
    rt.eval_module(&mut doc, reader).expect("eval local");
    assert_eq!(text(&doc, "out"), "3", "{:?}", rt.console_logs());
    let _ = local;
}

#[test]
fn a_failed_module_body_runs_once_and_a_later_import_returns_that_failure() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let dep = rt
        .compile_module(
            "document.getElementById('out').textContent += 'D';\n\
             throw new Error('dep-fail');\n",
            "https://example.test/fail-dep.js",
        )
        .expect("dep");
    let parent = rt
        .compile_module(
            "import \"https://example.test/fail-dep.js\";\n\
             document.getElementById('out').textContent += 'P';\n",
            "https://example.test/fail-parent.js",
        )
        .expect("parent");
    assert_eq!(text(&doc, "out"), "init");
    rt.instantiate_module(parent).expect("link");
    assert_eq!(text(&doc, "out"), "init", "instantiate ran a body");
    let err = rt.eval_module(&mut doc, parent).expect_err("dep fails");
    match err {
        ScriptError::Unsupported(message) => assert!(message.contains("dep-fail"), "{message}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initD", "{:?}", rt.console_logs());
    rt.begin_task();
    let again = rt
        .import_dynamic(&mut doc, "https://example.test/fail-dep.js", parent)
        .expect_err("cached");
    match again {
        ScriptError::Unsupported(message) => assert!(message.contains("dep-fail"), "{message}"),
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initD");
    let _ = dep;
}

#[test]
fn a_module_stopped_by_the_deadline_or_a_released_page_is_not_rerun() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let looping = rt
        .compile_module(
            "document.getElementById('out').textContent += 'L';\n\
             while (true) {}\n",
            "https://example.test/loop.js",
        )
        .expect("loop");
    rt.instantiate_module(looping).expect("link loop");
    let err = rt.eval_module(&mut doc, looping).expect_err("step budget");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("max_steps") || message.contains("step budget"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initL", "{:?}", rt.console_logs());
    rt.begin_task();
    let again = rt
        .eval_module(&mut doc, looping)
        .expect_err("cached budget");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("max_steps") || message.contains("step budget"),
                "{message}"
            );
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "initL");

    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let due = rt
        .compile_module(
            "document.getElementById('out').textContent = 'ran';\n",
            "https://example.test/due.js",
        )
        .expect("due");
    rt.instantiate_module(due).expect("link due");
    rt.arm_wall_deadline(std::time::Duration::ZERO);
    let err = rt.eval_module(&mut doc, due).expect_err("deadline");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("wall deadline"), "{message}");
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
    rt.clear_wall_deadline();
    rt.begin_task();
    let again = rt.eval_module(&mut doc, due).expect_err("cached deadline");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(message.contains("wall deadline"), "{message}");
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");

    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let body = rt
        .compile_module(
            "document.getElementById('out').textContent = 'ran';\n",
            "https://example.test/retired.js",
        )
        .expect("retired");
    rt.instantiate_module(body).expect("link");
    rt.commit_page_navigation();
    let err = rt.eval_module(&mut doc, body).expect_err("inactive");
    match err {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("document is not fully active"),
                "{message}"
            );
        }
        other => panic!("expected unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
    rt.rebind_page_document(&doc);
    let again = rt.eval_module(&mut doc, body).expect_err("cached inactive");
    match again {
        ScriptError::Unsupported(message) => {
            assert!(
                message.contains("document is not fully active"),
                "{message}"
            );
        }
        other => panic!("expected cached unsupported, got {other:?}"),
    }
    assert_eq!(text(&doc, "out"), "init");
}

#[test]
fn a_wall_deadline_stops_script_and_leaves_an_earlier_timer() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "setTimeout(function() {\
            document.getElementById('out').textContent = 'later';\
         }, 50);",
    );
    let due = rt.next_timer_due_ms().expect("timer queued");
    rt.arm_wall_deadline(std::time::Duration::from_secs(30));
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'open';",
    );
    assert_eq!(text(&doc, "out"), "open", "{:?}", rt.console_logs());
    rt.arm_wall_deadline(std::time::Duration::ZERO);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'done';",
    );
    assert_eq!(text(&doc, "out"), "open", "{:?}", rt.console_logs());
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("wall deadline")),
        "{:?}",
        rt.console_logs()
    );
    assert_eq!(rt.next_timer_due_ms(), Some(due));
    let blocked = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert_eq!(blocked.fired, 0);
    assert_eq!(blocked.outcome, anvil_script::TimerPollOutcome::Exhausted);
    assert_eq!(rt.abort_reason(), Some("wall deadline exhausted"));
    assert_eq!(text(&doc, "out"), "open");
    assert_eq!(rt.next_timer_due_ms(), Some(due));
    rt.clear_wall_deadline();
    let fired = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert!(fired.fired >= 1, "{fired:?}");
    assert_eq!(text(&doc, "out"), "later", "{:?}", rt.console_logs());
}

#[test]
fn a_running_loop_stops_at_the_wall_deadline_without_replay() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "setTimeout(function() {\
            document.getElementById('out').textContent = 'kept';\
         }, 80);",
    );
    let due = rt.next_timer_due_ms().expect("timer queued");
    rt.arm_wall_deadline(std::time::Duration::from_millis(40));
    rt.eval_script(
        &mut doc,
        "queueMicrotask(function() {\
            document.getElementById('out').textContent = 'micro';\
         });\
         var i = 0;\
         while (i < 1000000) { i = i + 1; }\
         document.getElementById('out').textContent = 'done';",
    );
    assert_ne!(text(&doc, "out"), "done", "{:?}", rt.console_logs());
    assert_ne!(text(&doc, "out"), "micro", "{:?}", rt.console_logs());
    assert_eq!(text(&doc, "out"), "init");
    assert!(
        rt.abort_reason().unwrap_or("").contains("wall deadline"),
        "step budget must not be the stop that hides the deadline: {:?} logs={:?}",
        rt.abort_reason(),
        rt.console_logs()
    );
    assert_eq!(rt.next_timer_due_ms(), Some(due));
    rt.clear_wall_deadline();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'next';",
    );
    assert_eq!(text(&doc, "out"), "next", "{:?}", rt.console_logs());
    let fired = rt.run_due_timers(
        &mut doc,
        due,
        anvil_script::TimerBudget {
            max_callbacks: 4,
            max_steps: 10_000,
        },
    );
    assert!(fired.fired >= 1, "{fired:?}");
    assert_eq!(text(&doc, "out"), "kept", "{:?}", rt.console_logs());
}

#[test]
fn an_interpreter_loop_stops_at_the_wall_deadline_and_a_nested_arm_cannot_extend_it() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.arm_wall_deadline(std::time::Duration::from_millis(40));
    rt.arm_wall_deadline(std::time::Duration::from_secs(30));
    rt.eval_script(
        &mut doc,
        "var i = 0; while (i < 1000000) { i = i + 1; } var done = 1; void document;",
    );
    assert!(
        rt.get_var("done").is_none(),
        "nested arm extended the deadline: {:?} logs={:?}",
        rt.abort_reason(),
        rt.console_logs()
    );
    assert!(
        rt.abort_reason().unwrap_or("").contains("wall deadline"),
        "{:?}",
        rt.abort_reason()
    );
    rt.clear_wall_deadline();
    rt.clear_wall_deadline();
    rt.eval_script(&mut doc, "var done = 2;");
    assert!(matches!(rt.get_var("done"), Some(anvil_script::Value::Number(n)) if *n == 2.0));
}

#[test]
fn a_duration_that_overflows_the_clock_is_immediately_due() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.arm_wall_deadline(std::time::Duration::MAX);
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'ran';",
    );
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    assert_eq!(rt.abort_reason(), Some("wall deadline exhausted"));
    rt.clear_wall_deadline();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'after';",
    );
    assert_eq!(text(&doc, "out"), "after", "{:?}", rt.console_logs());
}

#[test]
fn a_host_fetch_already_inside_the_call_is_not_preempted() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.set_fetch_host(Arc::new(move |_url, _page| {
        seen.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(80));
        anvil_script::ScriptFetchReply {
            status: 200,
            body: "slow".to_string(),
        }
    }));
    rt.arm_wall_deadline(std::time::Duration::from_millis(20));
    rt.eval_script(
        &mut doc,
        "var reply = fetch('https://app.example/slow'); var after = 1;",
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "host was preempted inside fetch"
    );
    assert!(
        rt.get_var("after").is_none(),
        "the statement after the host call ran past the deadline"
    );
    rt.clear_wall_deadline();
}

#[test]
fn a_rejected_history_url_does_not_change_origin_storage_fetch_or_form_base() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <form id=\"f\" action=\"send\"><input name=\"q\" value=\"v\"></form>",
    );
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/app/index.html");
    let epoch = rt.submission_epoch();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    rt.set_fetch_host(Arc::new(move |url, page| {
        record
            .lock()
            .expect("fetch record")
            .push((url.to_string(), page.to_string()));
        ScriptFetchReply {
            status: 200,
            body: "{}".to_string(),
        }
    }));
    rt.eval_script(
        &mut doc,
        "localStorage.setItem('keep', 'orig');\
         var caught = 0;\
         function reject(url) {\
           try { history.pushState(null, '', url); }\
           catch (e) { caught++; }\
         }\
         reject('https://evil.example/phish');\
         reject('http://app.example/x');\
         reject('https://app.example:8443/x');\
         reject('//evil.example/x');\
         history.pushState(null, '', '/stolen?q=1#f');\
         history.replaceState(null, '', 'HTTPS://App.Example/yes');\
         localStorage.setItem('after', 'still');\
         fetch('/api/who');\
         document.getElementById('f').requestSubmit();\
         document.getElementById('out').textContent = String(caught);",
    );
    assert_eq!(text(&doc, "out"), "4", "{:?}", rt.console_logs());
    assert_eq!(
        rt.page_origin(),
        Some("HTTPS://App.Example/yes"),
        "same-origin history spelling is kept and the origin is not poisoned"
    );
    assert_eq!(rt.storage_origin_key(), "https://app.example:443");
    assert_eq!(rt.local_storage_get("keep"), Some("orig"));
    assert_eq!(rt.local_storage_get("after"), Some("still"));
    assert_eq!(rt.submission_epoch(), epoch);
    assert!(!rt.navigation_has_committed());
    let fetches = seen.lock().expect("fetch record");
    assert_eq!(fetches.len(), 1, "{fetches:?}");
    assert_eq!(fetches[0].0, "https://app.example:443/api/who");
    assert_eq!(fetches[0].1, "HTTPS://App.Example/yes");
    drop(fetches);
    let snapshot = rt.take_submission_snapshot().expect("form snapshot");
    let action = snapshot.action.to_ascii_lowercase();
    assert!(
        action.starts_with("https://app.example/") && action.ends_with("/send"),
        "form base left the page origin: {}",
        snapshot.action
    );
    assert!(
        !action.contains("evil.example") && !action.contains(":8443"),
        "form base used a rejected history URL: {}",
        snapshot.action
    );
    assert_eq!(snapshot.epoch, epoch);
    rt.eval_script(&mut doc, "location.href = 'https://other.example/away';");
    assert_eq!(rt.page_origin(), Some("HTTPS://App.Example/yes"));
    assert_eq!(
        rt.take_pending_navigation().as_deref(),
        Some("https://other.example/away")
    );
    assert_eq!(rt.submission_epoch(), epoch);
    assert_eq!(rt.local_storage_get("keep"), Some("orig"));
}

#[test]
fn the_interpreter_rejects_a_cross_origin_history_url_before_changing_the_page() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/app/index.html");
    let epoch = rt.submission_epoch();
    rt.eval_script(
        &mut doc,
        "document.title = 't';\
         var caught = 0;\
         try { history.pushState(null, '', 'https://evil.example/phish'); }\
         catch (e) { caught = 1; }\
         history.replaceState(null, '', '/kept');\
         document.getElementById('out').textContent = String(caught);",
    );
    assert_eq!(text(&doc, "out"), "1", "{:?}", rt.console_logs());
    assert_eq!(rt.page_origin(), Some("https://app.example:443/kept"));
    assert_eq!(rt.storage_origin_key(), "https://app.example:443");
    assert_eq!(rt.submission_epoch(), epoch);
    assert!(rt.take_pending_navigation().is_none());
}

#[test]
fn a_suspended_page_keeps_timers_and_modules_until_the_same_token_resumes() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var n = 0; setInterval(function() { n = n + 1; }, 10);",
    );
    let epoch = rt.submission_epoch();
    let token = rt.suspend_page();
    assert!(rt.page_is_suspended());
    assert!(rt.sync_timer_clock(500.0));
    assert_eq!(rt.submission_epoch(), epoch.wrapping_add(1));
    let held = rt.run_due_timers(
        &mut doc,
        500.0,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert_eq!(held.fired, 0);
    assert_eq!(held.outcome, anvil_script::TimerPollOutcome::Suspended);
    assert!(held.next_due_ms.is_some());
    rt.eval_script(&mut doc, "n = n + 10;");
    assert!(matches!(rt.get_var("n"), Some(anvil_script::Value::Number(n)) if *n == 0.0));
    rt.resume_page(token).expect("matching token");
    assert!(!rt.page_is_suspended());
    assert_eq!(rt.submission_epoch(), epoch.wrapping_add(1));
    let once = rt.run_due_timers(
        &mut doc,
        500.0,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert_eq!(once.fired, 1, "{once:?}");
    assert!(matches!(rt.get_var("n"), Some(anvil_script::Value::Number(n)) if *n == 1.0));
    let again = rt.run_due_timers(
        &mut doc,
        500.0,
        anvil_script::TimerBudget {
            max_callbacks: 8,
            max_steps: 10_000,
        },
    );
    assert_eq!(
        again.fired, 0,
        "missed intervals must not catch up: {again:?}"
    );
    assert!(matches!(rt.get_var("n"), Some(anvil_script::Value::Number(n)) if *n == 1.0));
}

#[test]
fn a_suspended_module_continuation_waits_for_resume() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "var resolve;\n\
             globalThis.__release = function () { resolve(); };\n\
             var gate = new Promise(function (r) { resolve = r; });\n\
             document.getElementById('out').textContent += 'before';\n\
             await gate;\n\
             document.getElementById('out').textContent += 'after';\n",
            "https://example.test/freeze.js",
        )
        .expect("compile");
    rt.instantiate_module(id).expect("link");
    rt.eval_module(&mut doc, id).expect("park");
    assert_eq!(text(&doc, "out"), "initbefore");
    let token = rt.suspend_page();
    rt.checkpoint_microtasks(&mut doc);
    rt.eval_script(&mut doc, "__release();");
    assert_eq!(text(&doc, "out"), "initbefore");
    assert_eq!(
        rt.module_status(id).expect("status").phase,
        anvil_script::ModulePhase::Evaluating
    );
    rt.resume_page(token).expect("resume");
    rt.eval_script(&mut doc, "__release();");
    assert_eq!(text(&doc, "out"), "initbeforeafter");
    assert_eq!(
        rt.module_status(id).expect("done").phase,
        anvil_script::ModulePhase::Evaluated
    );
}

#[test]
fn suspension_tokens_resume_once_and_terminal_lifecycle_invalidates_them() {
    let mut doc = parse_html("<p id=\"out\">init</p><button id=\"b\">go</button>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('b').addEventListener('click', function() {\
           document.getElementById('out').textContent += 'c';\
         });",
    );
    let outer = rt.suspend_page();
    let inner = rt.suspend_page();
    let button = doc.get_element_by_id("b").expect("b");
    assert!(rt.resume_page(outer).is_err());
    assert!(rt.page_is_suspended());
    assert!(!rt.dispatch_click(&mut doc, button));
    assert_eq!(text(&doc, "out"), "init");
    rt.resume_page(inner).expect("newest token");
    assert!(rt.dispatch_click(&mut doc, button));
    assert_eq!(text(&doc, "out"), "initc");

    let mut other = ScriptRuntime::new();
    let foreign = other.suspend_page();
    assert!(rt.resume_page(foreign).is_err());
    assert!(other.page_is_suspended());
    let live = other.suspend_page();
    other.resume_page(live).expect("other runtime");
    assert!(!other.page_is_suspended());

    let released = rt.suspend_page();
    rt.release_page_callbacks();
    assert!(rt.resume_page(released).is_err());
    assert!(!rt.page_is_suspended());

    let mut committed = ScriptRuntime::new();
    let token = committed.suspend_page();
    committed.commit_page_navigation();
    assert!(committed.resume_page(token).is_err());
    assert!(committed.navigation_has_committed());

    let mut rebound = ScriptRuntime::new();
    rebound.eval_script(&mut doc, "var n = 1;");
    let token = rebound.suspend_page();
    let fresh = parse_html("<p id=\"out\">new</p>");
    rebound.rebind_page_document(&fresh);
    assert!(rebound.resume_page(token).is_err());
    assert!(!rebound.page_is_suspended());
    assert!(rebound.callbacks_live());
}

#[test]
fn a_snapshot_from_before_suspension_does_not_match_the_restored_epoch() {
    let mut doc = parse_html("<form id=\"g\"><input id=\"q\" name=\"q\" value=\"v\"></form>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/");
    rt.eval_script(&mut doc, "document.getElementById('g').requestSubmit();");
    let snapshot = rt.take_submission_snapshot().expect("before freeze");
    let before = snapshot.epoch;
    assert_eq!(before, rt.submission_epoch());
    let token = rt.suspend_page();
    assert!(rt.take_submission_snapshot().is_none());
    assert!(rt.take_host_submission().is_none());
    rt.resume_page(token).expect("resume");
    assert_ne!(snapshot.epoch, rt.submission_epoch());
    assert!(rt.take_submission_snapshot().is_none());
    assert_eq!(rt.submission_epoch(), before.wrapping_add(1));
    rt.eval_script(&mut doc, "document.getElementById('q').value = 'after';");
    let input = doc.get_element_by_id("q").expect("q");
    assert_eq!(doc.input_value(input), "after");
    assert!(!rt.navigation_has_committed());
}

#[test]
fn a_direct_engine_eval_does_not_run_while_the_page_is_suspended() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><script>document.getElementById('out').textContent = 'ran';</script>",
    );
    let mut rt = ScriptRuntime::new();
    let token = rt.suspend_page();
    let steps = rt.register_steps_charged();
    let lifetime = rt.page_budget().lifetime_steps;
    let used = rt.page_budget().steps_used;
    let blocked = rt.eval_with_cinder_engine(
        &mut doc,
        "document.getElementById('out').textContent = 'engine';",
    );
    assert!(
        matches!(blocked, Err(ref message) if message.contains("suspended")),
        "{blocked:?}"
    );
    assert_eq!(text(&doc, "out"), "init");
    assert_eq!(rt.register_steps_charged(), steps);
    assert_eq!(rt.page_budget().lifetime_steps, lifetime);
    assert_eq!(rt.page_budget().steps_used, used);
    assert!(rt.take_submitted_form().is_none());
    rt.run_document_scripts(&mut doc);
    assert_eq!(text(&doc, "out"), "init");
    rt.resume_page(token).expect("resume");
    assert_eq!(text(&doc, "out"), "init");
    assert!(rt.take_submitted_form().is_none());
    rt.eval_with_cinder_engine(
        &mut doc,
        "document.getElementById('out').textContent = 'engine';",
    )
    .expect("engine runs after resume");
    assert_eq!(text(&doc, "out"), "engine");
    assert!(rt.register_steps_charged() > steps);
}
