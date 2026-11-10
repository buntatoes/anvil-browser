// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! One page session keeps one global environment and one document.
//! Each top-level classic script is its own step slice.

use anvil_dom::{Document, parse_html};
use anvil_script::{MAX_EXECUTION_STEPS, ScriptRuntime};

fn run(scripts: &[&str]) -> (Document, ScriptRuntime) {
    let mut html = String::from("<html><body><p id=\"out\">init</p>");
    for script in scripts {
        html.push_str("<script>");
        html.push_str(script);
        html.push_str("</script>");
    }
    html.push_str("</body></html>");
    let mut doc = parse_html(&html);
    let root = doc.root().index() as u32;
    let mut rt = ScriptRuntime::new();
    rt.run_document_scripts(&mut doc);
    assert_eq!(rt.bound_document_root(), Some(root));
    (doc, rt)
}

fn out_text(doc: &Document) -> String {
    let out = doc.get_element_by_id("out").expect("out");
    doc.text_content(out)
}

#[test]
fn two_scripts_share_one_global_and_one_document() {
    let (doc, rt) = run(&[
        "var n = 1; document.getElementById('out').textContent = 'a';",
        "n = n + 1; document.getElementById('out').textContent = '' + n;",
    ]);
    assert_eq!(out_text(&doc), "2");
    let budget = rt.page_budget();
    assert!(budget.steps_used > 0, "{budget:?}");
    assert!(budget.steps_used < budget.step_limit, "{budget:?}");
    assert_eq!(budget.step_limit, MAX_EXECUTION_STEPS as u64);
    assert_eq!(budget.heap_limit_bytes, 64 * 1024 * 1024);
}

#[test]
fn a_later_classic_still_runs_after_a_script_stops() {
    // Each top-level classic is its own task. A nested click does not
    // open one; that is covered by the turn-budget tests. A stopped
    // script must not leave the rest of the page inert.
    let (doc, rt) = run(&[
        "let i = 0; while (i < 1000000) { i = i + 1; }",
        "document.getElementById('out').textContent = 'ran';",
    ]);
    assert_eq!(out_text(&doc), "ran");
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("step limit reached: 200000 steps")),
        "{:?}",
        rt.console_logs()
    );
}

#[test]
fn release_drops_a_timer_and_keeps_the_document_text() {
    let mut doc = parse_html("<p id=\"out\">stay</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "setTimeout(function(){ document.getElementById('out').textContent = 'late'; }, 0);",
    );
    assert_eq!(out_text(&doc), "stay");
    rt.release_page_callbacks();
    rt.run_timers(&mut doc);
    assert_eq!(out_text(&doc), "stay");
}
