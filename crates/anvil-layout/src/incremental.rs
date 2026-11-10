// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Real subtree reuse for one layout pass.
//!
//! A cached box is returned only when it belongs to this document identity,
//! its fingerprint still matches, and it was laid out at the same containing
//! width. The box is translated to the new origin. A newer mutation does not
//! reuse the previous full tree.

use std::cell::RefCell;

use anvil_dom::NodeId;

use crate::{LayoutBox, LayoutCache, offset_box};

std::thread_local! {
    static ACTIVE: RefCell<Option<LayoutCache>> = const { RefCell::new(None) };
}

pub(crate) fn install(cache: LayoutCache) {
    ACTIVE.with(|slot| {
        *slot.borrow_mut() = Some(cache);
    });
}

pub(crate) fn take() -> LayoutCache {
    ACTIVE
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_default()
}

/// A previously laid-out subtree for this document.
///
/// `document_id` is [`Document::identity`]. A box from another document, or
/// from a clone, is not returned. The node's fingerprint, containing width,
/// and containing height must also match. The fingerprint covers the computed
/// style (including fonts), image size, and viewport-sensitive fields.
pub(crate) fn take_reused(
    document_id: u64,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
    containing_height: Option<f32>,
) -> Option<LayoutBox> {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cache = slot.as_mut()?;
        if cache.document_id != document_id {
            return None;
        }
        let fingerprint = *cache.pending_fingerprints.get(&id)?;
        let stored = *cache.node_fingerprints.get(&id)?;
        if fingerprint != stored {
            return None;
        }
        let width = *cache.containing_widths.get(&id)?;
        if (width - containing_width).abs() > 0.01 {
            return None;
        }
        let height = *cache.containing_heights.get(&id)?;
        if !heights_match(height, containing_height) {
            return None;
        }
        let cached = cache.node_boxes.get(&id)?.clone();
        let mut reused = cached;
        let dx = x - reused.rect.x;
        let dy = y - reused.rect.y;
        if dx.abs() > 0.01 || dy.abs() > 0.01 {
            offset_box(&mut reused, dx, dy);
        }
        cache.subtree_reuses += 1;
        cache.node_boxes.insert(id, reused.clone());
        Some(reused)
    })
}

fn heights_match(stored: Option<f32>, now: Option<f32>) -> bool {
    match (stored, now) {
        (None, None) => true,
        (Some(left), Some(right)) => (left - right).abs() <= 0.01,
        _ => false,
    }
}

pub(crate) fn remember(
    id: NodeId,
    containing_width: f32,
    containing_height: Option<f32>,
    built: &LayoutBox,
) {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cache) = slot.as_mut() else {
            return;
        };
        let Some(fingerprint) = cache.pending_fingerprints.get(&id).copied() else {
            return;
        };
        cache.node_fingerprints.insert(id, fingerprint);
        cache.containing_widths.insert(id, containing_width);
        cache.containing_heights.insert(id, containing_height);
        cache.node_boxes.insert(id, built.clone());
    });
}

#[cfg(test)]
mod crossdoc_repro {
    use anvil_dom::parse_html;
    use anvil_style::{Stylesheet, compute_styles};

    use super::install;
    use crate::{LayoutCache, layout_document, layout_document_incremental};

    fn joined(tree: &crate::LayoutBox) -> String {
        let mut out = String::new();
        for run in &tree.text {
            out.push_str(&run.text);
        }
        for child in &tree.children {
            out.push_str(&joined(child));
        }
        out
    }

    #[test]
    fn thread_local_node_id_cache_does_not_serve_another_document() {
        let doc_a = parse_html("<div id=\"t\">ALPHA</div>");
        let doc_b = parse_html("<div id=\"t\">BETA</div>");
        let styles_a = compute_styles(&doc_a, &Stylesheet::default());
        let styles_b = compute_styles(&doc_b, &Stylesheet::default());
        let mut cache = LayoutCache::new();
        let _ = layout_document_incremental(
            &doc_a,
            &styles_a,
            200.0,
            &Default::default(),
            None,
            &mut cache,
        );
        install(cache);
        let tree = layout_document(&doc_b, &styles_b, 200.0);
        let text = joined(&tree);
        assert!(
            text.contains("BETA") && !text.contains("ALPHA"),
            "thread-local NodeId cache reused the other document: {text}"
        );
        let _ = super::take();
    }
}
