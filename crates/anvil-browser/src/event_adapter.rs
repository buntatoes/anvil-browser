// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Browser-side event walk.
//!
//! The script host owns one listener invocation. This module owns the path:
//! capture, target, then bubble when the event bubbles. Field names used on
//! each step match the host record: `type_name`, `current_target`, and
//! `immediate_stopped`.

use anvil_dom::{Document, NodeId, WalkFlags, WalkPhase, walk_event};

/// One listener on the platform path.
#[derive(Clone, Debug)]
pub struct PhaseListener {
    /// Node the listener was added on.
    pub node: NodeId,
    /// Event `type_name`.
    pub type_name: String,
    /// Capture listeners run on the way down. At the target, order is registration.
    pub capture: bool,
    /// Call [`WalkFlags::prevent_default`] when this listener runs.
    pub prevent: bool,
    /// Set [`WalkFlags::propagation_stopped`].
    pub stop: bool,
}

/// One listener invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseStep {
    /// Event type.
    pub type_name: String,
    /// Node whose listener ran.
    pub current_target: NodeId,
    /// Phase of this call.
    pub phase: WalkPhase,
}

/// Walk `type_name` on `target` and run matching listeners.
///
/// Returns the flags after the walk. The default action runs only when
/// [`WalkFlags::default_prevented`] is clear.
pub fn dispatch_phases(
    doc: &Document,
    target: NodeId,
    type_name: &str,
    listeners: &[PhaseListener],
    log: &mut Vec<PhaseStep>,
) -> WalkFlags {
    let mut flags = WalkFlags::for_type(type_name).unwrap_or(WalkFlags {
        bubbles: false,
        cancelable: false,
        default_prevented: false,
        propagation_stopped: false,
        immediate_stopped: false,
    });
    walk_event(doc, target, type_name, &mut flags, |node, phase, flags| {
        for listener in listeners
            .iter()
            .filter(|listener| listener.node == node && listener.type_name == type_name)
        {
            let run = match phase {
                WalkPhase::Capture => listener.capture,
                WalkPhase::Target => true,
                WalkPhase::Bubble => !listener.capture,
            };
            if !run {
                continue;
            }
            if flags.immediate_stopped {
                break;
            }
            log.push(PhaseStep {
                type_name: type_name.to_string(),
                current_target: node,
                phase,
            });
            if listener.prevent {
                flags.prevent_default();
            }
            if listener.stop {
                flags.propagation_stopped = true;
            }
        }
    });
    flags
}

/// Enter in a text control submits the enclosing form when the keydown was not cancelled.
pub fn implicit_submit_target(
    doc: &Document,
    target: NodeId,
    type_name: &str,
    key: Option<&str>,
    flags: &WalkFlags,
) -> Option<NodeId> {
    if flags.default_prevented || type_name != "keydown" || key != Some("Enter") {
        return None;
    }
    if !doc.is_text_input(target) {
        return None;
    }
    doc.enclosing_form(target)
}

/// A cancelled `beforeinput` does not change the control.
pub fn edit_allowed(type_name: &str, flags: &WalkFlags) -> bool {
    !(type_name == "beforeinput" && flags.default_prevented)
}
