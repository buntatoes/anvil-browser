// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Wall-deadline guard. The default budget stays disarmed, matching d045.

use std::time::{Duration, Instant};

use anvil_browser::task_deadline::{
    self, MAX_TASK_WALL_BUDGET, PROPOSED_TASK_WALL_BUDGET, TaskKind,
};
use anvil_dom::parse_html;
use anvil_script::ScriptRuntime;

fn text(doc: &anvil_dom::Document, id: &str) -> String {
    let node = doc.get_element_by_id(id).unwrap();
    doc.text_content(node)
}

#[test]
fn the_provisional_default_arms_and_none_is_the_only_disarm() {
    assert_eq!(task_deadline::default_budget(), PROPOSED_TASK_WALL_BUDGET);
    task_deadline::set_budget(Some(task_deadline::default_budget())).expect("default");
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    task_deadline::reset_arm_count();
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'ran';",
        );
    });
    assert_eq!(task_deadline::arm_count(), 1);
    assert_eq!(task_deadline::clear_count(), 1);
    assert_eq!(text(&doc, "out"), "ran");
    task_deadline::set_budget(None).expect("disarm");
    task_deadline::reset_arm_count();
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'off';",
        );
    });
    assert_eq!(task_deadline::arm_count(), 0);
    assert_eq!(text(&doc, "out"), "off");
}

#[test]
fn nested_host_path_does_not_rearm_or_extend_the_deadline() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    task_deadline::reset_arm_count();
    let previous = task_deadline::budget();
    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        task_deadline::with_runtime(rt, TaskKind::Event, |rt| {
            rt.eval_script(
                &mut doc,
                "document.getElementById('out').textContent = 'extended';",
            );
        });
    });
    task_deadline::set_budget(previous).expect("budget");
    assert_eq!(
        task_deadline::arm_count(),
        1,
        "nested host path armed again"
    );
    assert_eq!(text(&doc, "out"), "init", "{:?}", rt.console_logs());
    assert!(
        rt.console_logs()
            .iter()
            .any(|line| line.contains("wall deadline"))
            || rt.abort_reason().unwrap_or("").contains("wall deadline"),
        "{:?} {:?}",
        rt.abort_reason(),
        rt.console_logs()
    );
}

#[test]
fn a_second_runtime_arm_does_not_extend_and_each_clear_pops_one_frame() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.arm_wall_deadline(Duration::ZERO);
    rt.arm_wall_deadline(Duration::from_secs(30));
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'extended';",
    );
    assert_eq!(
        text(&doc, "out"),
        "init",
        "a longer nested arm extended the earlier deadline: {:?}",
        rt.console_logs()
    );
    rt.clear_wall_deadline();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'still';",
    );
    assert_eq!(
        text(&doc, "out"),
        "init",
        "one clear popped both frames: {:?}",
        rt.console_logs()
    );
    rt.clear_wall_deadline();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'after';",
    );
    assert_eq!(text(&doc, "out"), "after", "{:?}", rt.console_logs());
}

#[test]
fn the_deadline_is_cleared_after_a_normal_task_a_panic_and_between_tasks() {
    let previous = task_deadline::budget();
    task_deadline::set_budget(Some(Duration::from_secs(30))).expect("budget");
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'ok';",
        );
    });
    assert_eq!(text(&doc, "out"), "ok");
    task_deadline::set_budget(None).expect("budget");
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'after';",
    );
    assert_eq!(text(&doc, "out"), "after", "{:?}", rt.console_logs());

    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        task_deadline::with_runtime(&mut rt, TaskKind::Script, |_rt| {
            panic!("task failed");
        });
    }));
    assert!(panicked.is_err());
    task_deadline::set_budget(None).expect("budget");
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'recovered';",
    );
    assert_eq!(text(&doc, "out"), "recovered", "{:?}", rt.console_logs());

    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'cut';",
        );
    });
    assert_ne!(text(&doc, "out"), "cut");
    task_deadline::set_budget(Some(Duration::from_secs(30))).expect("budget");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'next';",
        );
    });
    assert_eq!(text(&doc, "out"), "next", "{:?}", rt.console_logs());
    task_deadline::set_budget(previous).expect("budget");
}

#[test]
fn register_route_aborts_inside_a_tight_loop() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'warm'; setTimeout(function(){}, 1000);",
    );
    assert!(
        rt.register_steps_charged() > 0,
        "warmup did not take the register route: {:?}",
        rt.console_logs()
    );
    let charged = rt.register_steps_charged();
    let started = Instant::now();
    rt.arm_wall_deadline(Duration::from_millis(20));
    rt.eval_script(
        &mut doc,
        "var i = 0;\
         while (i < 4000000) { i = i + 1; }\
         document.getElementById('out').textContent = 'done';",
    );
    let elapsed = started.elapsed();
    rt.clear_wall_deadline();
    let reason = rt.abort_reason().unwrap_or("").to_string();
    let logs = format!("{:?}", rt.console_logs());
    let line = format!(
        "register_route elapsed_ms={:.1} steps={} reason={reason} text={} logs={logs}",
        elapsed.as_secs_f64() * 1000.0,
        rt.register_steps_charged().saturating_sub(charged),
        text(&doc, "out")
    );
    eprintln!("{line}");
    let _ = std::fs::write("/tmp/register-route-timing.txt", &line);
    assert_ne!(text(&doc, "out"), "done", "{line}");
    assert!(
        reason.contains("wall deadline") || logs.contains("wall deadline"),
        "register route did not abort on the wall deadline: {line}"
    );
    assert!(
        rt.register_steps_charged() > charged,
        "no register steps ran under the deadline: {line}"
    );
}

#[test]
fn string_interpreter_tight_loop_aborts_at_the_wall_deadline() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let started = Instant::now();
    rt.arm_wall_deadline(Duration::from_millis(40));
    rt.eval_script(
        &mut doc,
        "var i = 0; while (i < 1000000) { i = i + 1; } var done = 1; void document;",
    );
    let elapsed = started.elapsed();
    let reason = rt.abort_reason().unwrap_or("").to_string();
    let logs = format!("{:?}", rt.console_logs());
    let done = rt.get_var("done").map(|value| format!("{value:?}"));
    let line = format!(
        "string_interpreter elapsed_ms={:.1} steps={} reason={reason} done={done:?} logs={logs}",
        elapsed.as_secs_f64() * 1000.0,
        rt.register_steps_charged()
    );
    eprintln!("{line}");
    let _ = std::fs::write("/tmp/string-interpreter-timing.txt", &line);
    rt.clear_wall_deadline();
    assert_eq!(
        rt.register_steps_charged(),
        0,
        "tight loop left the string interpreter: {line}"
    );
    assert!(done.is_none(), "the loop finished: {line}");
    assert!(
        reason.contains("wall deadline") || logs.contains("wall deadline"),
        "string interpreter did not abort on the wall deadline: {line}"
    );
}

#[test]
fn a_microtask_checkpoint_shares_the_task_deadline() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.eval_script(
        &mut doc,
        "document.getElementById('out').textContent = 'warm'; setTimeout(function(){}, 1000);",
    );
    let previous = task_deadline::budget();
    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "queueMicrotask(function(){ document.getElementById('out').textContent = 'micro'; });\
             document.getElementById('out').textContent = 'script';",
        );
        let _ = rt.checkpoint_microtasks(&mut doc);
    });
    task_deadline::set_budget(previous).expect("budget");
    assert_eq!(text(&doc, "out"), "warm", "{:?}", rt.console_logs());
}

#[test]
fn a_runaway_timer_aborts_and_later_timer_and_input_still_run() {
    use anvil_browser::{Session, Viewport};
    let previous = task_deadline::budget();
    task_deadline::set_budget(None).expect("budget");
    let mut session = Session::new(Viewport {
        width: 320.0,
        height: 240.0,
    });
    session.open_html(
        "<p id=\"out\">init</p><input id=\"q\" value=\"\">\
         <script>\
           setTimeout(function(){\
             var i = 0; while (i < 4000000) { i = i + 1; }\
             document.getElementById('out').textContent = 'timer-done';\
           }, 0);\
           setTimeout(function(){\
             document.getElementById('out').textContent = 'later';\
           }, 50);\
         </script>",
        "http://127.0.0.1/page",
    );
    task_deadline::set_budget(Some(Duration::from_millis(20))).expect("budget");
    let _ = session.animation_frame_at(10.0);
    let mid = {
        let page = session.active().current().unwrap();
        let id = page.document.get_element_by_id("out").unwrap();
        page.document.text_content(id)
    };
    assert_ne!(
        mid, "timer-done",
        "the runaway timer finished under a 20 ms deadline"
    );
    task_deadline::set_budget(None).expect("budget");
    let _ = session.animation_frame_at(80.0);
    let input = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("q")
        .unwrap();
    assert!(session.focus_text_input(input));
    assert!(session.type_into_focused_input('z'));
    let page = session.active().current().unwrap();
    let out = page.document.get_element_by_id("out").unwrap();
    let field = page.document.get_element_by_id("q").unwrap();
    assert_eq!(
        page.document.text_content(out),
        "later",
        "{:?}",
        page.runtime.console_logs()
    );
    assert_eq!(page.document.input_value(field), "z");
    assert!(page.url.starts_with("http://127.0.0.1/page"));
    task_deadline::set_budget(previous).expect("budget");
}

#[test]
fn a_runaway_click_handler_aborts_without_killing_the_page() {
    use anvil_browser::{Session, Viewport};
    let previous = task_deadline::budget();
    task_deadline::set_budget(None).expect("budget");
    let mut session = Session::new(Viewport {
        width: 400.0,
        height: 200.0,
    });
    session.open_html(
        "<body style=\"margin:0\"><p id=\"out\">init</p>\
         <button id=\"b\" style=\"width:120px;height:40px\">go</button></body>\
         <script>\
           document.getElementById('b').addEventListener('click', function(){\
             var i = 0; while (i < 4000000) { i = i + 1; }\
             document.getElementById('out').textContent = 'clicked';\
           });\
         </script>",
        "http://127.0.0.1/page",
    );
    task_deadline::set_budget(Some(Duration::from_millis(20))).expect("budget");
    let _ = session.click_at(8.0, 30.0);
    let mid = {
        let page = session.active().current().unwrap();
        let id = page.document.get_element_by_id("out").unwrap();
        page.document.text_content(id)
    };
    assert_ne!(mid, "clicked");
    assert!(
        session
            .active()
            .current()
            .unwrap()
            .url
            .starts_with("http://127.0.0.1/")
    );
    task_deadline::set_budget(None).expect("budget");
    session.open_html("<p id=\"out\">next</p>", "http://127.0.0.1/next");
    let page = session.active().current().unwrap();
    let id = page.document.get_element_by_id("out").unwrap();
    assert_eq!(page.document.text_content(id), "next");
    task_deadline::set_budget(previous).expect("budget");
}

fn assert_frames_clear(rt: &mut ScriptRuntime, doc: &mut anvil_dom::Document) {
    assert_eq!(task_deadline::arm_count(), task_deadline::clear_count());
    assert_eq!(task_deadline::task_depth(), 0);
    let previous = task_deadline::budget();
    task_deadline::set_budget(None).expect("budget");
    rt.eval_script(doc, "document.getElementById('out').textContent = 'clear';");
    task_deadline::set_budget(previous).expect("budget");
    assert_eq!(text(doc, "out"), "clear", "{:?}", rt.console_logs());
}

#[test]
fn arm_and_clear_balance_on_return_error_abort_and_panic() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    let previous = task_deadline::budget();
    task_deadline::reset_arm_count();
    task_deadline::set_budget(Some(Duration::from_secs(30))).expect("budget");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'ok';",
        );
    });
    assert_eq!(text(&doc, "out"), "ok");
    assert_frames_clear(&mut rt, &mut doc);

    task_deadline::set_budget(Some(Duration::from_secs(30))).expect("budget");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(&mut doc, "throw 'script-error';");
    });
    assert_frames_clear(&mut rt, &mut doc);

    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'cut';",
        );
    });
    assert_ne!(text(&doc, "out"), "cut");
    assert_frames_clear(&mut rt, &mut doc);

    task_deadline::set_budget(Some(Duration::from_secs(30))).expect("budget");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        task_deadline::with_runtime(&mut rt, TaskKind::Script, |_rt| panic!("unwind"));
    }));
    assert!(panicked.is_err());
    assert_frames_clear(&mut rt, &mut doc);
    task_deadline::set_budget(previous).expect("budget");
}

#[test]
fn an_invalid_budget_keeps_the_previous_valid_one() {
    task_deadline::set_budget(Some(PROPOSED_TASK_WALL_BUDGET)).expect("1500");
    assert_eq!(
        task_deadline::set_budget(Some(Duration::MAX)),
        Err(task_deadline::BudgetError::OutOfRange)
    );
    assert_eq!(task_deadline::budget(), Some(PROPOSED_TASK_WALL_BUDGET));
    assert_eq!(
        task_deadline::set_budget(Some(Duration::from_secs(31))),
        Err(task_deadline::BudgetError::OutOfRange)
    );
    assert_eq!(task_deadline::budget(), Some(PROPOSED_TASK_WALL_BUDGET));
    let overflow = Duration::new(u64::MAX / 2, 0);
    assert!(std::time::Instant::now().checked_add(overflow).is_none());
    assert_eq!(
        task_deadline::set_budget(Some(overflow)),
        Err(task_deadline::BudgetError::OutOfRange)
    );
    assert_eq!(task_deadline::budget(), Some(PROPOSED_TASK_WALL_BUDGET));
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    task_deadline::reset_arm_count();
    task_deadline::with_runtime(&mut rt, TaskKind::Script, |rt| {
        rt.eval_script(
            &mut doc,
            "document.getElementById('out').textContent = 'still';",
        );
    });
    assert_eq!(task_deadline::arm_count(), 1, "invalid set must not disarm");
    assert_eq!(text(&doc, "out"), "still", "{:?}", rt.console_logs());
    task_deadline::set_budget(None).expect("explicit disarm");
    assert_eq!(task_deadline::budget(), None);
}

#[test]
fn the_named_constant_and_overrides_stay_in_range() {
    assert!(task_deadline::duration_in_range(PROPOSED_TASK_WALL_BUDGET));
    assert!(task_deadline::duration_in_range(Duration::from_millis(20)));
    assert!(PROPOSED_TASK_WALL_BUDGET <= MAX_TASK_WALL_BUDGET);
    assert!(PROPOSED_TASK_WALL_BUDGET > Duration::ZERO);
    assert!(!task_deadline::duration_in_range(Duration::MAX));
    assert!(
        std::time::Instant::now()
            .checked_add(PROPOSED_TASK_WALL_BUDGET)
            .is_some()
    );
    task_deadline::set_budget(Some(Duration::from_millis(20))).expect("20 ms");
    assert_eq!(
        task_deadline::set_budget(Some(Duration::ZERO)),
        Err(task_deadline::BudgetError::Zero)
    );
    assert_eq!(
        task_deadline::budget(),
        Some(Duration::from_millis(20)),
        "zero must not disable the previous budget"
    );
    assert_eq!(
        task_deadline::set_budget(Some(Duration::MAX)),
        Err(task_deadline::BudgetError::OutOfRange)
    );
    assert_eq!(task_deadline::budget(), Some(Duration::from_millis(20)));
    task_deadline::set_budget(None).expect("disarm");
}

#[test]
fn the_runtime_treats_duration_max_as_immediately_due() {
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.arm_wall_deadline(Duration::MAX);
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
fn a_slow_fetch_host_overruns_until_the_next_statement() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let mut doc = parse_html("<p id=\"out\">init</p>");
    let mut rt = ScriptRuntime::new();
    rt.set_page_origin("https://app.example/index.html");
    rt.set_fetch_host(Arc::new(move |_url, _page| {
        seen.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(80));
        anvil_script::ScriptFetchReply {
            status: 200,
            body: "slow".to_string(),
        }
    }));
    let budget = Duration::from_millis(20);
    let started = Instant::now();
    rt.arm_wall_deadline(budget);
    rt.eval_script(
        &mut doc,
        "var reply = fetch('https://app.example/slow'); var after = 1;",
    );
    let elapsed = started.elapsed();
    rt.clear_wall_deadline();
    let overrun = elapsed.saturating_sub(budget);
    let line = format!(
        "fetch_host elapsed_ms={:.1} overrun_ms={:.1} calls={} after={:?} reason={:?}",
        elapsed.as_secs_f64() * 1000.0,
        overrun.as_secs_f64() * 1000.0,
        calls.load(Ordering::SeqCst),
        rt.get_var("after"),
        rt.abort_reason()
    );
    eprintln!("{line}");
    let _ = std::fs::write("/tmp/host-overrun.txt", &line);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "{line}");
    assert!(rt.get_var("after").is_none(), "{line}");
}

#[test]
fn parse_layout_and_raster_run_to_completion() {
    let mut html = String::from("<div>");
    for i in 0..400 {
        html.push_str(&format!("<p id=\"n{i}\">item {i}</p>"));
    }
    html.push_str("</div>");
    let started = Instant::now();
    let doc = parse_html(&html);
    let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
    let styles = anvil_style::compute_styles(&doc, &anvil_style::Stylesheet::default());
    let layout_started = Instant::now();
    let tree = anvil_layout::layout_document(&doc, &styles, 800.0);
    let layout_ms = layout_started.elapsed().as_secs_f64() * 1000.0;
    let paint_started = Instant::now();
    let list = anvil_paint::paint(&tree);
    let frame = anvil_raster::rasterize(&list, 800, 200, 0.0);
    let raster_ms = paint_started.elapsed().as_secs_f64() * 1000.0;
    let line = format!(
        "parse_ms={parse_ms:.3} layout_ms={layout_ms:.3} raster_ms={raster_ms:.3} pixels={}",
        frame.width()
    );
    eprintln!("{line}");
    let _ = std::fs::write("/tmp/host-windows.txt", &line);
    assert!(frame.width() > 0);
}

fn document_id(session: &anvil_browser::Session) -> u64 {
    session.active().current().unwrap().document.identity()
}

#[test]
fn the_guard_clears_the_runtime_it_armed_when_navigation_switches_pages() {
    use anvil_browser::{Session, Viewport};
    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::reset_arm_count();
    task_deadline::reset_runtime_ledger();
    let mut session = Session::new(Viewport {
        width: 320.0,
        height: 200.0,
    });
    session.open_html("<p id=\"out\">a</p>", "http://127.0.0.1/a");
    let first = document_id(&session);
    task_deadline::with_session(&mut session, TaskKind::Event, |session| {
        session.open_html("<p id=\"out\">b</p>", "http://127.0.0.1/b");
    });
    let second = document_id(&session);
    assert_ne!(first, second);
    assert_eq!(task_deadline::runtime_arm_clear(first), (1, 1), "old page");
    assert_eq!(task_deadline::runtime_arm_clear(second), (0, 0), "new page");
    task_deadline::set_budget(None).expect("disarm");
    assert!(session.back());
    session
        .eval_console_js("document.getElementById('out').textContent = 'back-ok';")
        .expect("old runtime still accepts a script");
    let page = session.active().current().unwrap();
    let id = page.document.get_element_by_id("out").unwrap();
    assert_eq!(page.document.text_content(id), "back-ok");
    assert!(session.forward());
    task_deadline::reset_runtime_ledger();
    task_deadline::set_budget(Some(PROPOSED_TASK_WALL_BUDGET)).expect("rearm");
    task_deadline::with_session(&mut session, TaskKind::Event, |_session| {});
    let current = document_id(&session);
    assert_eq!(task_deadline::runtime_arm_clear(current), (1, 1));
}

#[test]
fn the_guard_clears_the_armed_runtime_after_back_and_after_a_panic() {
    use anvil_browser::{Session, Viewport};
    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::reset_runtime_ledger();
    let mut session = Session::new(Viewport {
        width: 320.0,
        height: 200.0,
    });
    session.open_html("<p id=\"out\">a</p>", "http://127.0.0.1/a");
    session.open_html("<p id=\"out\">b</p>", "http://127.0.0.1/b");
    let second = document_id(&session);
    task_deadline::reset_runtime_ledger();
    task_deadline::with_session(&mut session, TaskKind::Event, |session| {
        assert!(session.back());
    });
    let restored = document_id(&session);
    assert_ne!(restored, second);
    assert_eq!(
        task_deadline::runtime_arm_clear(second),
        (1, 1),
        "bfcache left the armed page"
    );
    assert_eq!(task_deadline::runtime_arm_clear(restored), (0, 0));
    task_deadline::set_budget(None).expect("disarm");
    assert!(session.forward());
    session
        .eval_console_js("document.getElementById('out').textContent = 'fwd-ok';")
        .expect("armed runtime was cleared");

    task_deadline::set_budget(Some(Duration::from_nanos(1))).expect("due");
    task_deadline::reset_runtime_ledger();
    let armed = document_id(&session);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        task_deadline::with_session(&mut session, TaskKind::Event, |session| {
            session.open_html("<p id=\"out\">c</p>", "http://127.0.0.1/c");
            panic!("navigation panic");
        });
    }));
    assert!(panicked.is_err());
    assert_eq!(task_deadline::runtime_arm_clear(armed), (1, 1));
    let arrived = document_id(&session);
    assert_ne!(arrived, armed);
    assert_eq!(task_deadline::runtime_arm_clear(arrived), (0, 0));
}

/// One pass of the guard's own timer. The printed line is `kind nanos`.
#[test]
fn guard_timer_one_pass() {
    use anvil_browser::{Session, Viewport};
    let _ = task_deadline::take_samples();
    let mut session = Session::new(Viewport {
        width: 320.0,
        height: 200.0,
    });
    session.open_html(
        "<p id=\"out\">init</p><input id=\"q\"><button id=\"b\">go</button>\
         <script>\
           setTimeout(function(){ document.getElementById('out').textContent = 't'; }, 0);\
           document.getElementById('b').addEventListener('click', function(){\
             document.getElementById('out').textContent = 'c';\
           });\
         </script>",
        "http://127.0.0.1/timer-sample",
    );
    let input = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("q")
        .unwrap();
    assert!(session.focus_text_input(input));
    assert!(session.type_into_focused_input('a'));
    let _ = session.click_at(8.0, 40.0);
    let _ = session.animation_frame_at(20.0);
    let samples = task_deadline::take_samples();
    assert!(!samples.is_empty());
    for (kind, elapsed) in &samples {
        eprintln!("SAMPLE {kind:?} {}", elapsed.as_nanos());
    }
}
