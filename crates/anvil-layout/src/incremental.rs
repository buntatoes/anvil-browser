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

use crate::{CachedSubtree, LayoutBox, LayoutCache, translate_reused_box};

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

/// A previously laid-out pre-stretch subtree for this document.
///
/// `document_id` is [`Document::identity`]. A box from another document, or
/// from a clone, is not returned. The node's fingerprint, containing width,
/// containing height, and list marker must also match. The subtree was stored
/// before any parent flex/grid stretch or post-layout sticky/effect pass, so
/// translating from `(entry.origin_x, entry.origin_y)` to `(x, y)` preserves
/// horizontal auto margins and natural unstretched dimensions.
pub(crate) fn take_reused(
    document_id: u64,
    id: NodeId,
    x: f32,
    y: f32,
    containing_width: f32,
    containing_height: Option<f32>,
    list_marker: Option<&str>,
) -> Option<LayoutBox> {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cache = slot.as_mut()?;
        if cache.document_id != document_id {
            return None;
        }
        let fingerprint = *cache.pending_fingerprints.get(&id)?;
        let entry = cache.reusable_subtrees.get(&id)?;
        if entry.fingerprint != fingerprint {
            return None;
        }
        if (entry.containing_width - containing_width).abs() > 1e-5 {
            return None;
        }
        if !heights_match(entry.containing_height, containing_height) {
            return None;
        }
        if entry.list_marker.as_deref() != list_marker {
            return None;
        }
        let mut reused = entry.subtree.clone();
        let dx = x - entry.origin_x;
        let dy = y - entry.origin_y;
        if dx != 0.0 || dy != 0.0 {
            translate_reused_box(&mut reused, dx, dy);
        }
        cache.subtree_reuses += 1;
        Some(reused)
    })
}

fn heights_match(stored: Option<f32>, now: Option<f32>) -> bool {
    match (stored, now) {
        (None, None) => true,
        (Some(left), Some(right)) => (left - right).abs() <= 1e-5,
        _ => false,
    }
}

pub(crate) fn remember(
    id: NodeId,
    origin_x: f32,
    origin_y: f32,
    containing_width: f32,
    containing_height: Option<f32>,
    list_marker: Option<String>,
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
        cache.reusable_subtrees.insert(
            id,
            CachedSubtree {
                fingerprint,
                origin_x,
                origin_y,
                containing_width,
                containing_height,
                list_marker,
                subtree: built.clone(),
            },
        );
    });
}

pub(crate) fn lookup_intrinsic_width(
    document_id: u64,
    id: NodeId,
    kind: u8,
    containing_width: f32,
) -> Option<f32> {
    ACTIVE.with(|slot| {
        let slot = slot.borrow();
        let cache = slot.as_ref()?;
        if cache.document_id != document_id {
            return None;
        }
        let fp = *cache.pending_fingerprints.get(&id)?;
        let &(stored_fp, width) =
            cache
                .intrinsic_widths
                .get(&(id, kind, containing_width.to_bits()))?;
        (stored_fp == fp).then_some(width)
    })
}

pub(crate) fn remember_intrinsic_width(
    document_id: u64,
    id: NodeId,
    kind: u8,
    containing_width: f32,
    width: f32,
) {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(cache) = slot.as_mut() else {
            return;
        };
        if cache.document_id != document_id {
            return;
        }
        let Some(fp) = cache.pending_fingerprints.get(&id).copied() else {
            return;
        };
        cache
            .intrinsic_widths
            .insert((id, kind, containing_width.to_bits()), (fp, width));
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
