// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Tree-order event path. The caller invokes listeners.
//!
//! Names here are the walk, not the script host's event record. `WalkPhase`
//! is capture, then target, then bubble. Bubble is skipped when the event
//! does not bubble.

use crate::Document;
use crate::NodeId;

/// One step of the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkPhase {
    /// Ancestor, on the way to the target.
    Capture,
    /// The target node.
    Target,
    /// Ancestor, on the way back to the root.
    Bubble,
}

/// Flags the invoker updates while one event is walking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkFlags {
    /// From [`event_behavior`]. Bubble runs only when this is set.
    pub bubbles: bool,
    /// `prevent_default` is ignored when this is clear.
    pub cancelable: bool,
    /// A cancelable listener cancelled the default action.
    pub default_prevented: bool,
    /// Do not visit later nodes. Listeners on the current node still run.
    pub propagation_stopped: bool,
    /// Do not run later listeners on the current node.
    pub immediate_stopped: bool,
}

impl WalkFlags {
    /// Flags for `type_name`, or `None` when this walker does not know it.
    pub fn for_type(type_name: &str) -> Option<Self> {
        let (bubbles, cancelable) = event_behavior(type_name)?;
        Some(Self {
            bubbles,
            cancelable,
            default_prevented: false,
            propagation_stopped: false,
            immediate_stopped: false,
        })
    }

    /// Set the cancelled flag when the event is cancelable.
    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.default_prevented = true;
        }
    }
}

/// `(bubbles, cancelable)` for one event type.
///
/// `focus` and `blur` do not bubble. `focusin` and `focusout` do.
/// `input` and `change` bubble and are not cancelable. `invalid` does not
/// bubble and is cancelable.
pub fn event_behavior(type_name: &str) -> Option<(bool, bool)> {
    match type_name {
        "keydown" | "keyup" | "beforeinput" | "click" | "submit" => Some((true, true)),
        "input" | "change" | "focusin" | "focusout" => Some((true, false)),
        "focus" | "blur" => Some((false, false)),
        "invalid" => Some((false, true)),
        _ => None,
    }
}

/// Walk `target` for `type_name`.
///
/// Capture runs from the root down to the parent of `target`, then the
/// target, then bubble back out when the event bubbles. `invoke` may set
/// [`WalkFlags::propagation_stopped`] or [`WalkFlags::immediate_stopped`].
/// The same node is one call per phase; the invoker runs each listener there.
pub fn walk_event(
    doc: &Document,
    target: NodeId,
    type_name: &str,
    flags: &mut WalkFlags,
    mut invoke: impl FnMut(NodeId, WalkPhase, &mut WalkFlags),
) {
    let mut chain = vec![target];
    let mut cursor = doc.get(target).parent();
    while let Some(id) = cursor {
        chain.push(id);
        cursor = doc.get(id).parent();
    }
    if chain.len() > 1 {
        for id in chain[1..].iter().rev().copied() {
            if flags.propagation_stopped {
                return;
            }
            flags.immediate_stopped = false;
            invoke(id, WalkPhase::Capture, flags);
            if flags.propagation_stopped {
                return;
            }
        }
    }
    if flags.propagation_stopped {
        return;
    }
    flags.immediate_stopped = false;
    invoke(target, WalkPhase::Target, flags);
    if flags.propagation_stopped || !flags.bubbles {
        return;
    }
    for id in chain.iter().skip(1).copied() {
        if flags.propagation_stopped {
            return;
        }
        flags.immediate_stopped = false;
        invoke(id, WalkPhase::Bubble, flags);
    }
    let _ = type_name;
}

/// `<script type=module>` elements, in tree order. Their source is not classic script.
pub fn module_script_ids(doc: &Document) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
        let data = doc.get(id).data();
        if matches!(data.tag(), Some("noscript" | "template")) {
            return;
        }
        if data.tag() == Some("script") {
            let mime = data
                .attr("type")
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            if mime.eq_ignore_ascii_case("module") {
                out.push(id);
            }
            return;
        }
        for child in doc.children(id) {
            walk(doc, *child, out);
        }
    }
    let mut out = Vec::new();
    walk(doc, doc.root(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_html;

    #[test]
    fn focus_does_not_bubble_and_focusin_does() {
        let doc = parse_html("<div id=\"outer\"><button id=\"t\">x</button></div>");
        let target = doc.get_element_by_id("t").unwrap();
        let mut flags = WalkFlags::for_type("focus").unwrap();
        let mut phases = Vec::new();
        walk_event(&doc, target, "focus", &mut flags, |id, phase, _| {
            phases.push((id, phase));
        });
        assert!(phases.iter().all(|(_, phase)| *phase != WalkPhase::Bubble));
        assert_eq!(
            phases.last().map(|(_, phase)| *phase),
            Some(WalkPhase::Target)
        );

        let mut flags = WalkFlags::for_type("focusin").unwrap();
        let mut bubbled = false;
        walk_event(&doc, target, "focusin", &mut flags, |_, phase, _| {
            bubbled |= phase == WalkPhase::Bubble;
        });
        assert!(bubbled);
    }
}
