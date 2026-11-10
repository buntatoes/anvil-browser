// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Damage from one layout to the next.
//!
//! The rect is the union of the old and new border boxes of nodes that
//! changed, expanded by outline and shadow. Unchanged nodes are not included.

use std::collections::HashMap;

use anvil_layout::{LayoutBox, NodeId};

use crate::Rect;

/// Union of the border boxes that changed between `before` and `after`.
pub fn damage_between(before: &LayoutBox, after: &LayoutBox) -> Rect {
    let mut old = HashMap::new();
    index(before, &mut old);
    let mut acc: Option<Rect> = None;
    walk_new(after, &old, &mut acc);
    for (id, previous) in &old {
        if find(after, *id).is_none() {
            acc = Some(union(acc, inflated(previous)));
        }
    }
    acc.unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    })
}

fn index(tree: &LayoutBox, out: &mut HashMap<NodeId, LayoutBox>) {
    out.insert(tree.node, tree.clone());
    for child in &tree.children {
        index(child, out);
    }
}

fn walk_new(tree: &LayoutBox, old: &HashMap<NodeId, LayoutBox>, acc: &mut Option<Rect>) {
    match old.get(&tree.node) {
        None => *acc = Some(union(*acc, inflated(tree))),
        Some(previous) if changed(previous, tree) => {
            *acc = Some(union(Some(union(*acc, inflated(previous))), inflated(tree)));
        }
        Some(_) => {}
    }
    for child in &tree.children {
        walk_new(child, old, acc);
    }
}

fn changed(before: &LayoutBox, after: &LayoutBox) -> bool {
    before.rect != after.rect
        || before.text != after.text
        || before.background != after.background
        || before.children.len() != after.children.len()
        || before.outline_width != after.outline_width
        || before.box_shadow != after.box_shadow
}

fn inflated(tree: &LayoutBox) -> Rect {
    let mut extra = tree.outline_width + tree.outline_offset.abs();
    if let Some(shadow) = tree.box_shadow {
        let reach = shadow.blur_radius
            + shadow.spread_radius
            + shadow.offset_x.abs().max(shadow.offset_y.abs());
        extra = extra.max(reach);
    }
    Rect {
        x: tree.rect.x - extra,
        y: tree.rect.y - extra,
        width: tree.rect.width + extra * 2.0,
        height: tree.rect.height + extra * 2.0,
    }
}

fn union(acc: Option<Rect>, next: Rect) -> Rect {
    let Some(acc) = acc else {
        return next;
    };
    if acc.width <= 0.0 || acc.height <= 0.0 {
        return next;
    }
    if next.width <= 0.0 || next.height <= 0.0 {
        return acc;
    }
    let x0 = acc.x.min(next.x);
    let y0 = acc.y.min(next.y);
    let x1 = (acc.x + acc.width).max(next.x + next.width);
    let y1 = (acc.y + acc.height).max(next.y + next.height);
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

fn find(tree: &LayoutBox, id: NodeId) -> Option<&LayoutBox> {
    if tree.node == id {
        return Some(tree);
    }
    for child in &tree.children {
        if let Some(found) = find(child, id) {
            return Some(found);
        }
    }
    None
}
