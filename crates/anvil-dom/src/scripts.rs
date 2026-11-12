// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Classic script slots in document order.
//!
//! `type=module` is recorded and not treated as classic script.
//! An inline script with `async` or `defer` is still classic.

use crate::{Document, NodeId};

/// How the loader should run one `script` element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptClass {
    /// Parser-blocking classic script, inline or external.
    Classic,
    /// External classic script with `async`.
    Async,
    /// External classic script with `defer` and without `async`.
    Defer,
    /// `type=module`.
    Module,
    /// `type=importmap` inline JSON specifier map.
    ImportMap,
    /// `nomodule`, or a type that is not classic JavaScript.
    Skip,
}

/// One script element and the source the loader needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptSlot {
    /// The `script` element.
    pub node: NodeId,
    /// How to run it.
    pub class: ScriptClass,
    /// Trimmed `src`, when the element has one.
    pub src: Option<String>,
    /// Inline text. Empty for an external script.
    pub text: String,
    /// `integrity`, when present.
    pub integrity: Option<String>,
}

/// Script elements in tree order. Descendants of a script are not separate slots.
pub fn script_slots(doc: &Document) -> Vec<ScriptSlot> {
    let mut out = Vec::new();
    walk(doc, doc.root(), &mut out);
    out
}

fn walk(doc: &Document, id: NodeId, out: &mut Vec<ScriptSlot>) {
    let data = doc.get(id).data();
    if matches!(data.tag(), Some("noscript" | "template")) {
        return;
    }
    if data.tag() == Some("script") {
        out.push(classify(doc, id));
        return;
    }
    for child in doc.children(id) {
        walk(doc, *child, out);
    }
}

fn classify(doc: &Document, id: NodeId) -> ScriptSlot {
    let data = doc.get(id).data();
    let raw_type = data.attr("type").unwrap_or("").trim();
    let mime = raw_type.split(';').next().unwrap_or(raw_type).trim();
    let src = data
        .attr("src")
        .map(str::trim)
        .filter(|src| !src.is_empty())
        .map(str::to_string);
    let integrity = data
        .attr("integrity")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let class = if mime.eq_ignore_ascii_case("module") {
        ScriptClass::Module
    } else if raw_type.eq_ignore_ascii_case("importmap") {
        ScriptClass::ImportMap
    } else if data.attr("nomodule").is_some() || !is_classic_type(mime) {
        ScriptClass::Skip
    } else if src.is_some() && data.attr("async").is_some() {
        ScriptClass::Async
    } else if src.is_some() && data.attr("defer").is_some() {
        ScriptClass::Defer
    } else {
        ScriptClass::Classic
    };
    let text = if src.is_some() {
        String::new()
    } else {
        doc.text_content(id)
    };
    ScriptSlot {
        node: id,
        class,
        src,
        text,
        integrity,
    }
}

fn is_classic_type(mime: &str) -> bool {
    if mime.is_empty() {
        return true;
    }
    matches!(
        mime.to_ascii_lowercase().as_str(),
        "text/javascript"
            | "application/javascript"
            | "text/ecmascript"
            | "application/ecmascript"
            | "text/jscript"
    )
}

/// A script inserted after parse. `async_false` keeps insertion order.
/// `type=module` stays unsupported. Anything else is async unless `async_false`.
pub fn dynamic_slot(doc: &Document, id: NodeId, async_false: bool) -> ScriptSlot {
    let mut slot = classify(doc, id);
    if matches!(
        slot.class,
        ScriptClass::Module | ScriptClass::ImportMap | ScriptClass::Skip
    ) {
        return slot;
    }
    if slot.src.is_some() && !async_false {
        slot.class = ScriptClass::Async;
    } else {
        slot.class = ScriptClass::Classic;
    }
    slot
}
