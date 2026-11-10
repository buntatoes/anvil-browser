// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Persistent register VM for classic DOM scripts: one closure and one
//! callback across script tags. Compile steps stay off the execution turn.
//! Modules are compiled here; the host fetches.

use anvil_dom::parse_html;
use anvil_script::{COMPILE_STEP_LIMIT, ScriptError, ScriptRuntime};

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    doc.text_content(doc.get_element_by_id(id).expect(id))
}

#[test]
fn classic_scripts_share_one_register_closure_and_click_callback() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p><button id=\"b\"></button>\
         <script>\
           var n = 1;\
           function bump() { n = n + 1; }\
           function onClick() {\
             bump();\
             document.getElementById('out').textContent = '' + n;\
           }\
           document.getElementById('b').addEventListener('click', onClick);\
           document.getElementById('out').textContent = '' + n;\
         </script>\
         <script>\
           n = n + 10;\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert!(
        rt.register_steps_charged() > 0,
        "classic DOM scripts did not run on the register VM; logs={:?} text={}",
        rt.console_logs(),
        text(&doc, "out")
    );
    assert_eq!(text(&doc, "out"), "1", "first script did not publish n");
    let button = doc.get_element_by_id("b").expect("b");
    rt.dispatch_click(&mut doc, button);
    assert_eq!(
        text(&doc, "out"),
        "12",
        "click callback did not see the second script; logs={:?}",
        rt.console_logs()
    );
    let budget = rt.page_budget();
    assert_eq!(budget.compile_limit, COMPILE_STEP_LIMIT);
    assert_ne!(budget.compile_limit, budget.step_limit);
    assert!(
        budget.compile_steps > 0,
        "bootstrap ran before a budget was armed; {budget:?}"
    );
    assert!(budget.compile_steps < COMPILE_STEP_LIMIT);
    assert_ne!(
        budget.compile_steps, budget.steps_used,
        "execution charged the compiler; {budget:?}"
    );
}

#[test]
fn register_microtasks_are_charged_against_the_remaining_turn() {
    let mut doc = parse_html(
        "<p id=\"out\">init</p>\
         <script>\
           function onClick() {}\
           document.getElementById('out').addEventListener('click', onClick);\
           var i = 0;\
           while (i < 20000) { i = i + 1; }\
           function burn() {\
             document.getElementById('out').textContent = 'job';\
             var j = 0;\
             while (j < 100000000) { j = j + 1; }\
           }\
           Promise.resolve().then(burn);\
         </script>",
    );
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    let budget = rt.page_budget();
    assert_eq!(
        text(&doc, "out"),
        "job",
        "microtask did not run; {budget:?} logs={:?}",
        rt.console_logs()
    );
    assert!(
        budget.steps_used > 100_000,
        "microtask steps were not charged onto the turn; {budget:?}"
    );
    assert!(
        budget.steps_used <= budget.step_limit + 10_000,
        "microtask reused the pre-script ceiling; {budget:?}"
    );
}

#[test]
fn a_script_the_register_vm_rejects_is_not_rerun() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "var n = 1; function onClick() {} document.getElementById('out').textContent = '' + n; document.getElementById('out').addEventListener('click', onClick);",
    );
    assert!(rt.register_steps_charged() > 0, "{:?}", rt.console_logs());
    rt.eval_script(
        &mut doc,
        "let marker = 5;\nimport \"https://example.test/missing.js\";",
    );
    assert!(
        rt.get_var("marker").is_none(),
        "rejected script reran on the interpreter: {:?}",
        rt.get_var("marker")
    );
    assert_eq!(text(&doc, "out"), "1");
    assert!(
        rt.last_unsupported().is_some(),
        "missing explicit unsupported; logs={:?}",
        rt.console_logs()
    );
}

#[test]
fn module_eval_uses_the_register_vm_and_dynamic_import_asks_the_host_to_fetch() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let id = rt
        .compile_module(
            "var m = 2;\n\
             document.getElementById('out').textContent = '' + m;\n",
            "https://example.test/a.js",
        )
        .expect("compile");
    assert!(rt.module_requests(id).expect("requests").is_empty());
    rt.instantiate_module(id).expect("instantiate");
    rt.eval_module(&mut doc, id).expect("eval");
    assert_eq!(text(&doc, "out"), "2", "{:?}", rt.console_logs());
    let err = rt
        .import_dynamic(&mut doc, "https://example.test/missing.js", id)
        .expect_err("fetch");
    match err {
        ScriptError::NeedsFetch { specifier, from } => {
            assert_eq!(specifier, "https://example.test/missing.js");
            assert_eq!(from, id);
        }
        other => panic!("expected NeedsFetch, got {other:?}"),
    }
    let dep = rt
        .compile_module("var dep = 4;\n", "https://example.test/dep.js")
        .expect("dep");
    let parent = rt
        .compile_module(
            "import \"https://example.test/dep.js\";\nvar linked = 1;\n",
            "https://example.test/parent.js",
        )
        .expect("parent");
    let got = rt
        .import_dynamic(&mut doc, "https://example.test/dep.js", parent)
        .expect("dynamic");
    assert_eq!(got, dep);
}
