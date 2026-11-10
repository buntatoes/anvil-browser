// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! One task per classic script.
//!
//! Parser-blocking scripts run in document order. Async externals run in the
//! order the fetch callback returns them. Defer externals run after the
//! document is interactive, in document order, before `DOMContentLoaded`.
//! Each script is one `eval_script` call. A stop is an error for that script.
//! The next script is a new task. A `type=module` script is fetched and linked
//! by the host. Top-level await, dynamic `import()`, import attributes, and
//! import maps are recorded as unsupported and are not run.

use anvil_dom::{
    Document, NodeId, ReadyState, ScriptClass, ScriptSlot, SubmissionSnapshot, WalkFlags,
    dynamic_slot, script_slots, walk_event,
};
use anvil_net::{parse_url, resolve_url};
use anvil_script::{MAX_SCRIPT_SOURCE, ModuleId, ModulePhase, ScriptError, ScriptRuntime};

/// What the loader observed while running classic scripts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptLog {
    /// `classic`, `async`, `defer`, `module`, or `error`, then the element index.
    pub order: Vec<String>,
    /// Ready states stored, in order.
    pub ready: Vec<ReadyState>,
    /// Snapshots taken after each task.
    pub submissions: Vec<SubmissionSnapshot>,
    /// Readiness events the loader dispatched, in order.
    pub events: Vec<String>,
    /// Microtask jobs drained after script tasks. This is not a timer drain.
    pub microtasks: usize,
}

/// Fetch one external script. `Err` is a failure for that script only.
pub type ScriptFetch<'a> = dyn Fn(&ScriptSlot) -> Result<Vec<u8>, String> + Sync + 'a;

/// Run the classic scripts of `doc`.
///
/// `set_ready` is `ScriptRuntime::set_ready_state`. This function also
/// dispatches `readystatechange`, `DOMContentLoaded`, and `load`.
/// `take_submission` is `ScriptRuntime::take_submission_snapshot`.
pub fn run_document_scripts(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    allow_inline: bool,
    fetch: &ScriptFetch<'_>,
    mut set_ready: impl FnMut(&mut ScriptRuntime, ReadyState),
    mut take_submission: impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    load_modules: bool,
) -> ScriptLog {
    let mut log = ScriptLog::default();
    let slots = script_slots(doc);
    if !load_modules && slots.iter().any(|slot| slot.class == ScriptClass::Module) {
        log.order.push("module".to_string());
    }
    mark_ready(doc, runtime, ReadyState::Loading, &mut set_ready, &mut log);
    let mut deferred = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut inflight = 0usize;
    std::thread::scope(|scope| {
        for slot in slots {
            match slot.class {
                ScriptClass::Skip => {}
                ScriptClass::Module => {
                    if load_modules {
                        run_module_slot(doc, runtime, &slot, allow_inline, fetch, &mut log);
                    }
                }
                ScriptClass::Defer => deferred.push(slot),
                ScriptClass::Async => {
                    inflight += 1;
                    let tx = tx.clone();
                    let fetch_fn = fetch;
                    scope.spawn(move || {
                        let result = fetch_fn(&slot);
                        let _ = tx.send((slot, result));
                    });
                    drain_arrived(
                        doc,
                        runtime,
                        &rx,
                        &mut inflight,
                        false,
                        &mut take_submission,
                        &mut log,
                    );
                }
                ScriptClass::Classic => {
                    drain_arrived(
                        doc,
                        runtime,
                        &rx,
                        &mut inflight,
                        false,
                        &mut take_submission,
                        &mut log,
                    );
                    if slot.src.is_some() {
                        run_external(
                            doc,
                            runtime,
                            &slot,
                            "classic",
                            fetch,
                            &mut take_submission,
                            &mut log,
                        );
                    } else if slot.text.trim().is_empty() {
                    } else if allow_inline {
                        run_source(
                            doc,
                            runtime,
                            slot.node,
                            "classic",
                            &slot.text,
                            &mut take_submission,
                            &mut log,
                        );
                    } else {
                        log.order.push(format!("blocked:{}", slot.node.index()));
                    }
                }
            }
        }
        drop(tx);
        drain_arrived(
            doc,
            runtime,
            &rx,
            &mut inflight,
            true,
            &mut take_submission,
            &mut log,
        );
    });
    mark_ready(
        doc,
        runtime,
        ReadyState::Interactive,
        &mut set_ready,
        &mut log,
    );
    for slot in deferred {
        run_external(
            doc,
            runtime,
            &slot,
            "defer",
            fetch,
            &mut take_submission,
            &mut log,
        );
    }
    dispatch_readiness(doc, runtime, "DOMContentLoaded", &mut log);
    mark_ready(doc, runtime, ReadyState::Complete, &mut set_ready, &mut log);
    dispatch_readiness(doc, runtime, "load", &mut log);
    // Timers stay pending. `ScriptRuntime::run_timers` fast-forwards due times,
    // so the loader does not call it. A later due-poll, when the runtime
    // provides one, is what fires a timer. Submissions recorded by a timer
    // are not taken here.
    log
}

/// Run one script inserted after the document pipeline.
/// `async_false` keeps it in insertion order as a classic script.
pub fn run_dynamic_script(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    node: NodeId,
    async_false: bool,
    fetch: &ScriptFetch<'_>,
    mut take_submission: impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    load_modules: bool,
) -> ScriptLog {
    let mut log = ScriptLog::default();
    let slot = dynamic_slot(doc, node, async_false);
    match slot.class {
        ScriptClass::Module => {
            if load_modules {
                run_module_slot(doc, runtime, &slot, true, fetch, &mut log);
            } else {
                log.order.push("module".to_string());
            }
        }
        ScriptClass::Skip => {}
        ScriptClass::Classic => {
            if slot.src.is_some() {
                run_external(
                    doc,
                    runtime,
                    &slot,
                    "classic",
                    fetch,
                    &mut take_submission,
                    &mut log,
                );
            } else if !slot.text.trim().is_empty() {
                run_source(
                    doc,
                    runtime,
                    slot.node,
                    "classic",
                    &slot.text,
                    &mut take_submission,
                    &mut log,
                );
            }
        }
        ScriptClass::Async | ScriptClass::Defer => run_external(
            doc,
            runtime,
            &slot,
            "async",
            fetch,
            &mut take_submission,
            &mut log,
        ),
    }
    log
}

fn mark_ready(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    state: ReadyState,
    set_ready: &mut impl FnMut(&mut ScriptRuntime, ReadyState),
    log: &mut ScriptLog,
) {
    set_ready(runtime, state);
    log.ready.push(state);
    dispatch_readiness(doc, runtime, "readystatechange", log);
}

fn drain_arrived(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    rx: &std::sync::mpsc::Receiver<(ScriptSlot, Result<Vec<u8>, String>)>,
    inflight: &mut usize,
    wait: bool,
    take_submission: &mut impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    log: &mut ScriptLog,
) {
    loop {
        if *inflight == 0 {
            return;
        }
        let arrived = if wait {
            rx.recv().ok()
        } else {
            rx.try_recv().ok()
        };
        let Some((slot, result)) = arrived else {
            return;
        };
        *inflight -= 1;
        match result {
            Ok(bytes) => finish_fetched(doc, runtime, &slot, "async", bytes, take_submission, log),
            Err(reason) => note_error(doc, slot.node, "async", &reason, log),
        }
        if !wait {
            continue;
        }
    }
}

fn finish_fetched(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    slot: &ScriptSlot,
    label: &str,
    bytes: Vec<u8>,
    take_submission: &mut impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    log: &mut ScriptLog,
) {
    if bytes.len() > MAX_SCRIPT_SOURCE {
        note_error(
            doc,
            slot.node,
            label,
            "script source is over the limit",
            log,
        );
        return;
    }
    match String::from_utf8(bytes) {
        Ok(code) => run_source(doc, runtime, slot.node, label, &code, take_submission, log),
        Err(_) => note_error(doc, slot.node, label, "script was not utf-8", log),
    }
}

fn dispatch_readiness(
    doc: &mut Document,
    _runtime: &mut ScriptRuntime,
    type_name: &str,
    log: &mut ScriptLog,
) {
    log.events.push(type_name.to_string());
    let mut flags = WalkFlags {
        bubbles: false,
        cancelable: false,
        default_prevented: false,
        propagation_stopped: false,
        immediate_stopped: false,
    };
    let target = doc.root();
    walk_event(
        doc,
        target,
        type_name,
        &mut flags,
        |_node, _phase, _flags| {},
    );
}

fn run_external(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    slot: &ScriptSlot,
    label: &str,
    fetch: &ScriptFetch<'_>,
    take_submission: &mut impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    log: &mut ScriptLog,
) {
    if slot.src.is_none() {
        return;
    }
    match fetch(slot) {
        Ok(bytes) => {
            if bytes.len() > MAX_SCRIPT_SOURCE {
                note_error(
                    doc,
                    slot.node,
                    label,
                    "script source is over the limit",
                    log,
                );
                return;
            }
            match String::from_utf8(bytes) {
                Ok(code) => run_source(doc, runtime, slot.node, label, &code, take_submission, log),
                Err(_) => note_error(doc, slot.node, label, "script was not utf-8", log),
            }
        }
        Err(reason) => note_error(doc, slot.node, label, &reason, log),
    }
}

fn run_source(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    node: NodeId,
    label: &str,
    source: &str,
    take_submission: &mut impl FnMut(&mut ScriptRuntime) -> Option<SubmissionSnapshot>,
    log: &mut ScriptLog,
) {
    // A new top-level task. On the standalone host this clears the previous
    // script's stop. On the host that opens a turn inside `eval_script`, that
    // call is the fresh slice. This does not run the failed script again.
    crate::task_deadline::with_runtime(
        runtime,
        crate::task_deadline::TaskKind::Script,
        |runtime| {
            if runtime.page_is_suspended() {
                return;
            }
            runtime.reset_execution_budget();
            let cleared = runtime.abort_reason().map(str::to_string);
            let before = runtime.console_logs().len();
            let _ = runtime.sync_timer_clock(0.0);
            runtime.eval_script(doc, source);
            // `checkpoint_microtasks` returns how many interpreter jobs ran. It also
            // drains register jobs and does not fire timers. The snapshot was taken
            // by the runtime before this checkpoint. It shares this task's deadline.
            let _ = runtime.sync_timer_clock(0.0);
            log.microtasks += runtime.checkpoint_microtasks(doc);
            let console_failed = runtime.console_logs()[before..]
                .iter()
                .any(|line| line.starts_with("[error]"));
            let after = runtime.abort_reason().map(str::to_string);
            let failed = console_failed || after.is_some() && after != cleared;
            if failed {
                let reason = after.unwrap_or_else(|| "script error".to_string());
                note_error(doc, node, label, &reason, log);
            } else {
                log.order.push(format!("{label}:{}", node.index()));
            }
            if let Some(host) = take_submission(runtime) {
                log.submissions.push(host);
            }
        },
    );
}

fn note_error(doc: &mut Document, node: NodeId, label: &str, reason: &str, log: &mut ScriptLog) {
    log.order
        .push(format!("error:{label}:{}:{reason}", node.index()));
    let mut flags = WalkFlags {
        bubbles: true,
        cancelable: false,
        default_prevented: false,
        propagation_stopped: false,
        immediate_stopped: false,
    };
    walk_event(doc, node, "error", &mut flags, |_, _, _| {});
}

/// Absolute module identity for `specifier` imported by `base_url`.
///
/// Scheme and host are lowercased, default ports are omitted, and `.` / `..`
/// are collapsed. Path case is kept. A bare specifier, `javascript:`, or
/// `data:` returns `None` so the host does not fetch it.
pub fn canonicalize_module_specifier(base_url: &str, specifier: &str) -> Option<String> {
    let specifier = specifier.trim();
    if specifier.is_empty() || specifier.starts_with('#') {
        return None;
    }
    if specifier.contains(" with ") || specifier.contains(" assert ") {
        return None;
    }
    let joined = if specifier.contains("://") {
        parse_url(specifier).ok()?;
        specifier.to_string()
    } else if specifier.starts_with("//")
        || specifier.starts_with('.')
        || specifier.starts_with('/')
    {
        resolve_url(base_url, specifier)?
    } else {
        return None;
    };
    let parsed = parse_url(&joined).ok()?;
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return None;
    }
    let origin = parsed.authority();
    let path = parsed.path.trim_start_matches('/');
    let relative = match parsed.query.as_deref() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    };
    resolve_url(&format!("{origin}/"), &relative)
}

fn unsupported_module_syntax(source: &str) -> Option<&'static str> {
    scan_module_program(source.as_bytes(), 0, source.len())
}

/// Walk module source. Strings and comments do not count. A dynamic
/// `import` may have spaces or comments before `(`. An import attribute is
/// `assert` or `with` followed by `{` in the import statement itself.
fn scan_module_program(bytes: &[u8], mut index: usize, end: usize) -> Option<&'static str> {
    while index < end {
        match bytes[index] {
            b'\'' | b'"' => index = skip_quoted(bytes, index, bytes[index]),
            b'`' => {
                if let Some(kind) = scan_template(bytes, index, end) {
                    return Some(kind);
                }
                index = skip_quoted(bytes, index, b'`');
            }
            b'/' if index + 1 < end && bytes[index + 1] == b'/' => {
                index = skip_line_comment(bytes, index);
            }
            b'/' if index + 1 < end && bytes[index + 1] == b'*' => {
                index = skip_block_comment(bytes, index);
            }
            _ if ident_at(bytes, index, "import") => {
                let after = skip_space_and_comments(bytes, index + "import".len());
                if after < end && bytes[after] == b'(' {
                    return Some("dynamic-import");
                }
                if import_statement_has_attributes(bytes, after, end) {
                    return Some("import-attributes");
                }
                index += "import".len();
            }
            _ => index += 1,
        }
    }
    None
}

fn scan_template(bytes: &[u8], start: usize, end: usize) -> Option<&'static str> {
    let mut index = start + 1;
    while index < end {
        match bytes[index] {
            b'\\' => index = (index + 2).min(end),
            b'`' => return None,
            b'$' if index + 1 < end && bytes[index + 1] == b'{' => {
                let expr = index + 2;
                let close = matching_brace(bytes, expr, end)?;
                if let Some(kind) = scan_module_program(bytes, expr, close) {
                    return Some(kind);
                }
                index = close + 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn import_statement_has_attributes(bytes: &[u8], mut index: usize, end: usize) -> bool {
    let mut depth = 0i32;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' | b'`' => index = skip_quoted(bytes, index, bytes[index]),
            b'/' if index + 1 < end && bytes[index + 1] == b'/' => {
                index = skip_line_comment(bytes, index);
            }
            b'/' if index + 1 < end && bytes[index + 1] == b'*' => {
                index = skip_block_comment(bytes, index);
            }
            b'(' | b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b')' | b'}' | b']' => {
                depth -= 1;
                index += 1;
            }
            b';' | b'\n' if depth <= 0 => return false,
            _ if depth <= 0
                && (ident_at(bytes, index, "assert") || ident_at(bytes, index, "with")) =>
            {
                let word = if ident_at(bytes, index, "assert") {
                    "assert"
                } else {
                    "with"
                };
                let after = skip_space_and_comments(bytes, index + word.len());
                if after < end && bytes[after] == b'{' {
                    return true;
                }
                index += word.len();
            }
            _ => index += 1,
        }
    }
    false
}

fn ident_at(bytes: &[u8], index: usize, word: &str) -> bool {
    let raw = word.as_bytes();
    if index + raw.len() > bytes.len() || &bytes[index..index + raw.len()] != raw {
        return false;
    }
    let before_ok = index == 0 || !is_ident_byte(bytes[index - 1]);
    let after = index + raw.len();
    let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
    before_ok && after_ok
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn skip_space_and_comments(bytes: &[u8], mut index: usize) -> usize {
    loop {
        if index >= bytes.len() {
            return index;
        }
        match bytes[index] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => index += 1,
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                index = skip_line_comment(bytes, index);
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                index = skip_block_comment(bytes, index);
            }
            _ => return index,
        }
    }
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    bytes
        .iter()
        .skip(start)
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset + 1)
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn matching_brace(bytes: &[u8], mut index: usize, end: usize) -> Option<usize> {
    let mut depth = 1i32;
    while index < end {
        match bytes[index] {
            b'\'' | b'"' | b'`' => index = skip_quoted(bytes, index, bytes[index]),
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn run_module_slot(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    slot: &ScriptSlot,
    allow_inline: bool,
    fetch: &ScriptFetch<'_>,
    log: &mut ScriptLog,
) {
    if runtime.page_is_suspended() {
        return;
    }
    let page_url = runtime.page_origin().unwrap_or("").to_string();
    if page_url.is_empty() {
        log.order.push(format!(
            "error:module:{}:missing page url",
            slot.node.index()
        ));
        return;
    }
    let (source, canonical) = if let Some(src) = slot.src.as_deref() {
        let Some(canonical) = canonicalize_module_specifier(&page_url, src) else {
            log.order
                .push(format!("unsupported:scheme:{}", slot.node.index()));
            return;
        };
        if let Some(id) = module_already_compiled(runtime, &canonical) {
            finish_module(doc, runtime, id, fetch, &page_url, slot, log);
            return;
        }
        match fetch_module_bytes(fetch, slot, &canonical) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => (text, canonical),
                Err(_) => {
                    log.order.push(format!(
                        "error:module:{}:source is not utf-8",
                        slot.node.index()
                    ));
                    return;
                }
            },
            Err(reason) => {
                note_module_fetch_error(slot, &reason, log);
                return;
            }
        }
    } else if slot.text.trim().is_empty() {
        return;
    } else if !allow_inline {
        log.order.push(format!("blocked:{}", slot.node.index()));
        return;
    } else {
        let Some(canonical) = inline_module_base(&page_url, slot.node) else {
            log.order
                .push(format!("unsupported:scheme:{}", slot.node.index()));
            return;
        };
        (slot.text.clone(), canonical)
    };
    if let Some(kind) = unsupported_module_syntax(&source) {
        log.order
            .push(format!("unsupported:{kind}:{}", slot.node.index()));
        return;
    }
    let id = match runtime.compile_module(&source, &canonical) {
        Ok(id) => id,
        Err(error) => {
            log.order.push(format!(
                "error:module:{}:{}",
                slot.node.index(),
                script_error_text(&error)
            ));
            return;
        }
    };
    finish_module(doc, runtime, id, fetch, &page_url, slot, log);
}

fn link_module(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    id: ModuleId,
    fetch: &ScriptFetch<'_>,
    page_url: &str,
    slot: &ScriptSlot,
    log: &mut ScriptLog,
) -> bool {
    let _ = doc;
    for _ in 0..32 {
        match runtime.instantiate_module(id) {
            Ok(()) => return true,
            Err(ScriptError::NeedsFetch { specifier, from }) => {
                let referrer = runtime
                    .module_status(from)
                    .map(|status| status.url)
                    .unwrap_or_else(|_| page_url.to_string());
                let canonical = if specifier.contains("://") {
                    canonicalize_module_specifier(&referrer, &specifier).unwrap_or(specifier)
                } else if let Some(canonical) = canonicalize_module_specifier(&referrer, &specifier)
                {
                    canonical
                } else {
                    log.order
                        .push(format!("unsupported:import-maps:{}", slot.node.index()));
                    return false;
                };
                if unsupported_scheme(&canonical) {
                    log.order
                        .push(format!("unsupported:scheme:{}", slot.node.index()));
                    return false;
                }
                let bytes = match fetch_module_bytes(fetch, slot, &canonical) {
                    Ok(bytes) => bytes,
                    Err(reason) => {
                        note_module_fetch_error(slot, &reason, log);
                        return false;
                    }
                };
                let Ok(source) = String::from_utf8(bytes) else {
                    log.order.push(format!(
                        "error:module:{}:source is not utf-8",
                        slot.node.index()
                    ));
                    return false;
                };
                if let Some(kind) = unsupported_module_syntax(&source) {
                    log.order
                        .push(format!("unsupported:{kind}:{}", slot.node.index()));
                    return false;
                }
                if let Err(error) = runtime.compile_module(&source, &canonical) {
                    log.order.push(format!(
                        "error:module:{}:{}",
                        slot.node.index(),
                        script_error_text(&error)
                    ));
                    return false;
                }
            }
            Err(error) => {
                log.order.push(format!(
                    "error:module:{}:{}",
                    slot.node.index(),
                    script_error_text(&error)
                ));
                return false;
            }
        }
    }
    log.order.push(format!(
        "error:module:{}:module graph did not link",
        slot.node.index()
    ));
    false
}

fn inline_module_base(page_url: &str, node: NodeId) -> Option<String> {
    let page = canonicalize_module_specifier(page_url, page_url)?;
    Some(format!("{page}#anvil-inline-{}", node.index()))
}

fn module_already_compiled(runtime: &ScriptRuntime, canonical: &str) -> Option<ModuleId> {
    let mut id = 1u64;
    while id < 1_000_000 {
        match runtime.module_status(ModuleId(id)) {
            Ok(status) if status.url == canonical => return Some(ModuleId(id)),
            Ok(_) => id += 1,
            Err(_) => return None,
        }
    }
    None
}

fn finish_module(
    doc: &mut Document,
    runtime: &mut ScriptRuntime,
    id: ModuleId,
    fetch: &ScriptFetch<'_>,
    page_url: &str,
    slot: &ScriptSlot,
    log: &mut ScriptLog,
) {
    if !link_module(doc, runtime, id, fetch, page_url, slot, log) {
        return;
    }
    match runtime.eval_module(doc, id) {
        Ok(()) => {
            if !runtime.page_is_suspended() {
                let _ = runtime.checkpoint_microtasks(doc);
            }
            match runtime.module_status(id) {
                Ok(status) if status.phase == ModulePhase::Evaluated => {
                    log.order.push(format!("module:{}", slot.node.index()));
                }
                Ok(status) if status.phase == ModulePhase::Evaluating => {
                    log.order
                        .push(format!("unsupported:top-level-await:{}", slot.node.index()));
                }
                Ok(status) => {
                    if let Some(error) = status.error {
                        log.order
                            .push(format!("error:module:{}:{error}", slot.node.index()));
                    } else {
                        log.order.push(format!("module:{}", slot.node.index()));
                    }
                }
                Err(error) => {
                    log.order.push(format!(
                        "error:module:{}:{}",
                        slot.node.index(),
                        script_error_text(&error)
                    ));
                }
            }
        }
        Err(error) => {
            log.order.push(format!(
                "error:module:{}:{}",
                slot.node.index(),
                script_error_text(&error)
            ));
        }
    }
}

fn note_module_fetch_error(slot: &ScriptSlot, reason: &str, log: &mut ScriptLog) {
    if reason.contains("blocked") {
        log.order.push(format!("blocked:{}", slot.node.index()));
    } else {
        log.order
            .push(format!("error:module:{}:{reason}", slot.node.index()));
    }
}

fn fetch_module_bytes(
    fetch: &ScriptFetch<'_>,
    slot: &ScriptSlot,
    canonical: &str,
) -> Result<Vec<u8>, String> {
    let mut request = slot.clone();
    request.src = Some(canonical.to_string());
    fetch(&request)
}

fn unsupported_scheme(url: &str) -> bool {
    parse_url(url)
        .ok()
        .is_none_or(|parsed| parsed.scheme != "http" && parsed.scheme != "https")
}

fn script_error_text(error: &ScriptError) -> String {
    match error {
        ScriptError::Unsupported(message)
        | ScriptError::Thrown(message)
        | ScriptError::BudgetExceeded(message) => message.clone(),
        ScriptError::NeedsFetch { specifier, .. } => format!("needs {specifier}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_dom::parse_html;

    #[test]
    fn module_specifiers_canonicalize_host_case_default_port_and_dot_segments() {
        let from_relative =
            canonicalize_module_specifier("https://App.Example/app/page.html", "./sub/../Dep.js")
                .unwrap();
        let from_absolute = canonicalize_module_specifier(
            "https://app.example/app/page.html",
            "HTTPS://App.Example:443/app/Dep.js",
        )
        .unwrap();
        assert_eq!(from_relative, "https://app.example/app/Dep.js");
        assert_eq!(from_relative, from_absolute);
        assert!(
            canonicalize_module_specifier("https://app.example/app/page.html", "lodash").is_none()
        );
        assert!(
            canonicalize_module_specifier("https://app.example/", "javascript:alert(1)").is_none()
        );
    }

    fn text(doc: &Document, id: &str) -> String {
        let node = doc.get_element_by_id(id).unwrap();
        doc.text_content(node)
    }

    #[test]
    fn a_runaway_classic_does_not_block_the_next_script() {
        let mut doc = parse_html(
            "<p id=\"out\">init</p>\
             <script>var i = 0; while (i < 1000000) { i = i + 1; }</script>\
             <script>document.getElementById('out').textContent = 'done';</script>",
        );
        let mut runtime = ScriptRuntime::new();
        let fetch = |_slot: &ScriptSlot| Err("unused".to_string());
        let log = run_document_scripts(
            &mut doc,
            &mut runtime,
            true,
            &fetch,
            |_runtime, _state| {},
            |_runtime| None,
            false,
        );
        assert_eq!(text(&doc, "out"), "done", "{:?}", log.order);
        assert!(
            log.order.iter().any(|entry| entry.starts_with("error:")),
            "{:?}",
            log.order
        );
    }

    /// Adapter stub. This records the loader's ready-state calls. It does not
    /// read `document.readyState` from the script host.
    #[test]
    fn ready_state_calls_are_loading_interactive_then_complete() {
        let mut doc = parse_html("<p id=\"out\">init</p><script>1;</script>");
        let mut runtime = ScriptRuntime::new();
        let fetch = |_slot: &ScriptSlot| Err("unused".to_string());
        let mut seen = Vec::new();
        let log = run_document_scripts(
            &mut doc,
            &mut runtime,
            true,
            &fetch,
            |_runtime, state| seen.push(state),
            |_runtime| None,
            false,
        );
        assert_eq!(
            seen,
            vec![
                ReadyState::Loading,
                ReadyState::Interactive,
                ReadyState::Complete
            ]
        );
        assert_eq!(log.ready, seen);
    }

    #[test]
    fn defer_runs_after_interactive_and_async_uses_fetch_order() {
        let mut doc = parse_html(
            "<p id=\"out\">init</p>\
             <script src=\"a.js\" async></script>\
             <script>document.getElementById('out').textContent += 'C';</script>\
             <script src=\"d.js\" defer></script>",
        );
        let mut runtime = ScriptRuntime::new();
        let fetch = |slot: &ScriptSlot| match slot.src.as_deref() {
            Some("a.js") => Ok(b"document.getElementById('out').textContent += 'A';".to_vec()),
            Some("d.js") => Ok(b"document.getElementById('out').textContent += 'D';".to_vec()),
            other => Err(format!("unexpected {other:?}")),
        };
        let log = run_document_scripts(
            &mut doc,
            &mut runtime,
            true,
            &fetch,
            |_runtime, _state| {},
            |_runtime| None,
            false,
        );
        let shown = text(&doc, "out");
        assert!(
            shown.contains('A') && shown.contains('C') && shown.contains('D'),
            "{shown} {:?}",
            log.order
        );
        let classic_at = log
            .order
            .iter()
            .position(|entry| entry.starts_with("classic:"))
            .unwrap();
        let defer_at = log
            .order
            .iter()
            .position(|entry| entry.starts_with("defer:"))
            .unwrap();
        assert!(classic_at < defer_at, "{:?}", log.order);
        assert!(
            log.order.iter().any(|entry| entry.starts_with("async:")),
            "{:?}",
            log.order
        );
    }

    fn run_modules(
        html: &str,
        page_url: &str,
        fetch: &super::ScriptFetch<'_>,
    ) -> (Document, ScriptRuntime, ScriptLog) {
        let mut doc = parse_html(html);
        let mut runtime = ScriptRuntime::new();
        runtime.set_page_origin(page_url);
        runtime.set_module_specifier_resolver(super::canonicalize_module_specifier);
        let log = run_document_scripts(
            &mut doc,
            &mut runtime,
            true,
            fetch,
            |_runtime, _state| {},
            |_runtime| None,
            true,
        );
        (doc, runtime, log)
    }

    #[test]
    fn a_string_that_mentions_import_still_runs() {
        let fetch = |_slot: &ScriptSlot| Err("no fetch".to_string());
        let (doc, _rt, log) = run_modules(
            "<p id=\"out\">init</p>\
             <script type=\"module\">var s = \"import(x)\"; document.getElementById('out').textContent = s;</script>",
            "https://app.example/page.html",
            &fetch,
        );
        assert_eq!(text(&doc, "out"), "import(x)");
        assert!(
            log.order.iter().any(|line| line.starts_with("module:")),
            "{:?}",
            log.order
        );
        assert!(
            log.order
                .iter()
                .all(|line| !line.contains("dynamic-import"))
        );
    }

    #[test]
    fn a_spaced_or_commented_dynamic_import_is_unsupported() {
        let fetch = |_slot: &ScriptSlot| Err("fetched".to_string());
        for source in [
            "import ('./x.js');",
            "import/*c*/('./x.js');",
            "import\n('./https-not-used.js');",
            "import ('javascript:alert(1)');",
            "import x from './a.js' with { type: 'json' };",
            "import './a.js' assert { type: 'json' };",
        ] {
            let (_doc, _rt, log) = run_modules(
                &format!("<p id=\"out\">init</p><script type=\"module\">{source}</script>"),
                "https://app.example/page.html",
                &fetch,
            );
            assert!(
                log.order.iter().any(|line| {
                    line.contains("dynamic-import") || line.contains("import-attributes")
                }),
                "{source} {:?}",
                log.order
            );
            assert!(
                log.order.iter().all(|line| !line.starts_with("module:")),
                "{source} ran {:?}",
                log.order
            );
        }
    }

    #[test]
    fn two_inline_modules_compile_apart_and_import_from_the_page() {
        let seen = std::sync::Mutex::new(Vec::new());
        let fetch = |slot: &ScriptSlot| {
            seen.lock()
                .unwrap()
                .push(slot.src.clone().unwrap_or_default());
            let src = slot.src.clone().unwrap_or_default();
            if src.ends_with("/app/dep.js") {
                Ok(b"export const n = 1;\n".to_vec())
            } else {
                Err(format!("unexpected {src}"))
            }
        };
        let (doc, runtime, log) = run_modules(
            "<p id=\"out\">init</p>\
             <script type=\"module\">document.getElementById('out').textContent = 'one';</script>\
             <script type=\"module\">import './dep.js'; document.getElementById('out').textContent = 'two';</script>\
             <script>document.getElementById('out').textContent += ':classic';</script>",
            "https://app.example/app/page.html",
            &fetch,
        );
        assert_eq!(
            text(&doc, "out"),
            "two:classic",
            "{:?} {:?}",
            text(&doc, "out"),
            log.order
        );
        assert!(
            log.order
                .iter()
                .all(|line| !line.contains("already compiled")),
            "{:?}",
            log.order
        );
        let fetched = seen.lock().unwrap().clone();
        assert_eq!(
            fetched,
            vec!["https://app.example/app/dep.js".to_string()],
            "{fetched:?}"
        );
        let mut urls = Vec::new();
        let mut id = 1u64;
        while let Ok(status) = runtime.module_status(anvil_script::ModuleId(id)) {
            urls.push(status.url);
            id += 1;
        }
        assert!(
            urls.iter().any(|url| url.contains("#anvil-inline-")),
            "{urls:?}"
        );
        assert_eq!(
            urls.iter()
                .filter(|url| url.contains("#anvil-inline-"))
                .count(),
            2,
            "{urls:?}"
        );
    }

    #[test]
    fn a_compiled_module_at_id_64_is_not_fetched_again() {
        let mut runtime = ScriptRuntime::new();
        runtime.set_page_origin("https://app.example/page.html");
        runtime.set_module_specifier_resolver(super::canonicalize_module_specifier);
        for n in 1..=64 {
            runtime
                .compile_module(
                    "export const n = 1;\n",
                    &format!("https://app.example/m{n}.js"),
                )
                .unwrap_or_else(|err| panic!("compile {n}: {err:?}"));
        }
        let seen = std::sync::Mutex::new(Vec::new());
        let fetch = |slot: &ScriptSlot| {
            seen.lock()
                .unwrap()
                .push(slot.src.clone().unwrap_or_default());
            Ok(b"export const n = 1;\n".to_vec())
        };
        let mut doc = parse_html(
            "<script type=\"module\" src=\"https://app.example/m1.js\"></script>\
             <script type=\"module\" src=\"https://app.example/m64.js\"></script>",
        );
        let log = run_document_scripts(
            &mut doc,
            &mut runtime,
            true,
            &fetch,
            |_runtime, _state| {},
            |_runtime| None,
            true,
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "fetched {:?} log {:?}",
            seen.lock().unwrap(),
            log.order
        );
    }
}
