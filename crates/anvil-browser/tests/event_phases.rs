// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Phase order and cancellation for the platform event walk.

use anvil_browser::event_adapter::{
    PhaseListener, dispatch_phases, edit_allowed, implicit_submit_target,
};
use anvil_dom::{WalkPhase, parse_html};

fn tree() -> (anvil_dom::Document, anvil_dom::NodeId, anvil_dom::NodeId) {
    let doc = parse_html("<div id=\"outer\"><input id=\"t\" type=\"text\" value=\"a\"></div>");
    let outer = doc.get_element_by_id("outer").unwrap();
    let target = doc.get_element_by_id("t").unwrap();
    (doc, outer, target)
}

fn phases_for(type_name: &str, prevent_on_target: bool) -> (Vec<WalkPhase>, bool) {
    let (doc, outer, target) = tree();
    let listeners = [
        PhaseListener {
            node: outer,
            type_name: type_name.to_string(),
            capture: true,
            prevent: false,
            stop: false,
        },
        PhaseListener {
            node: target,
            type_name: type_name.to_string(),
            capture: false,
            prevent: prevent_on_target,
            stop: false,
        },
        PhaseListener {
            node: outer,
            type_name: type_name.to_string(),
            capture: false,
            prevent: false,
            stop: false,
        },
    ];
    let mut log = Vec::new();
    let flags = dispatch_phases(&doc, target, type_name, &listeners, &mut log);
    let phases = log
        .into_iter()
        .filter(|step| step.current_target == outer || step.current_target == target)
        .map(|step| step.phase)
        .collect();
    (phases, flags.default_prevented)
}

#[test]
fn each_event_type_uses_its_bubble_and_cancel_behavior() {
    let bubbling = [
        "keydown",
        "keyup",
        "beforeinput",
        "input",
        "change",
        "submit",
        "click",
        "focusin",
        "focusout",
    ];
    for type_name in bubbling {
        let (phases, _) = phases_for(type_name, false);
        assert!(
            phases.contains(&WalkPhase::Capture),
            "{type_name} capture {phases:?}"
        );
        assert!(
            phases.contains(&WalkPhase::Target),
            "{type_name} target {phases:?}"
        );
        assert!(
            phases.contains(&WalkPhase::Bubble),
            "{type_name} bubble {phases:?}"
        );
        let capture_at = phases.iter().position(|phase| *phase == WalkPhase::Capture);
        let target_at = phases.iter().position(|phase| *phase == WalkPhase::Target);
        let bubble_at = phases.iter().position(|phase| *phase == WalkPhase::Bubble);
        assert!(
            capture_at < target_at && target_at < bubble_at,
            "{type_name}"
        );
    }
    for type_name in ["focus", "blur"] {
        let (phases, _) = phases_for(type_name, false);
        assert!(phases.contains(&WalkPhase::Target), "{type_name}");
        assert!(
            !phases.contains(&WalkPhase::Bubble),
            "{type_name} bubbled {phases:?}"
        );
    }
    let (_phases, click_cancelled) = phases_for("click", true);
    assert!(click_cancelled);
    let (_phases, input_cancelled) = phases_for("input", true);
    assert!(!input_cancelled, "input is not cancelable");
}

#[test]
fn enter_submits_unless_keydown_is_cancelled_and_beforeinput_can_block_the_edit() {
    let doc = parse_html("<form id=\"f\"><input id=\"t\" type=\"text\" value=\"a\"></form>");
    let target = doc.get_element_by_id("t").unwrap();
    let listeners = [PhaseListener {
        node: target,
        type_name: "keydown".to_string(),
        capture: false,
        prevent: false,
        stop: false,
    }];
    let mut log = Vec::new();
    let flags = dispatch_phases(&doc, target, "keydown", &listeners, &mut log);
    assert!(implicit_submit_target(&doc, target, "keydown", Some("Enter"), &flags).is_some());

    let listeners = [PhaseListener {
        node: target,
        type_name: "keydown".to_string(),
        capture: false,
        prevent: true,
        stop: false,
    }];
    let flags = dispatch_phases(&doc, target, "keydown", &listeners, &mut log);
    assert!(implicit_submit_target(&doc, target, "keydown", Some("Enter"), &flags).is_none());

    let listeners = [PhaseListener {
        node: target,
        type_name: "beforeinput".to_string(),
        capture: false,
        prevent: true,
        stop: false,
    }];
    let flags = dispatch_phases(&doc, target, "beforeinput", &listeners, &mut log);
    assert!(!edit_allowed("beforeinput", &flags));
    let flags = dispatch_phases(
        &doc,
        target,
        "beforeinput",
        &[PhaseListener {
            node: target,
            type_name: "beforeinput".to_string(),
            capture: false,
            prevent: false,
            stop: false,
        }],
        &mut log,
    );
    assert!(edit_allowed("beforeinput", &flags));
}
