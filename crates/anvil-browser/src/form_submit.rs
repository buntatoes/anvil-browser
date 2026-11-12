// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Form submission the loader sends.
//!
//! [`submission_effect`] reads the document. [`navigation_from`] turns that
//! into the navigation: GET puts the pairs on the URL and sends no body, and
//! POST sends the body with its content type. [`consume_host_submission`] is
//! the only consumer of a [`HostSubmission`].

pub use anvil_dom::SubmitDispatch;
use anvil_dom::{
    ChosenFormFile, Document, FormMultipartEntry, HostSubmission, NodeId, WalkFlags, WalkPhase,
    urlencode_component,
};
use anvil_net::{
    ContentSecurityPolicy, CookieJar, MAX_REDIRECTS, NetError, http_get_with_cookie,
    http_post_with_cookie_progress, is_same_origin, is_url_blocked_by_default_shield, parse_url,
    resolve_url,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Files the picker stored for this browser. Keyed by document identity and control.
///
/// [`SelectedFiles::choose_from_picker`] is the only inserter. Attributes,
/// clipboard data, and drag data never create an entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedFiles {
    files: Vec<ChosenFormFile>,
}

impl SelectedFiles {
    /// An empty store.
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Replace the files for one control. This is the picker path.
    pub fn choose_from_picker(&mut self, file: ChosenFormFile) {
        self.files.retain(|existing| {
            !(existing.document == file.document && existing.control == file.control)
        });
        self.files.push(file);
    }

    /// Replace the files for one control with several picker results.
    pub fn choose_many_from_picker(
        &mut self,
        document: u64,
        control: NodeId,
        files: Vec<ChosenFormFile>,
    ) {
        self.files
            .retain(|existing| !(existing.document == document && existing.control == control));
        for mut file in files {
            file.document = document;
            file.control = control;
            self.files.push(file);
        }
    }

    /// Files whose [`ChosenFormFile::matches`] is this document and control.
    pub fn for_control<'a>(
        &'a self,
        doc: &'a Document,
        control: NodeId,
    ) -> Vec<&'a ChosenFormFile> {
        self.files
            .iter()
            .filter(|file| file.matches(doc, control))
            .collect()
    }

    /// Drop every file stored for `document`.
    pub fn clear_document(&mut self, document: u64) {
        self.files.retain(|file| file.document != document);
    }

    /// Drop files whose control is gone or is no longer `input type=file`.
    pub fn retain_live(&mut self, doc: &Document) {
        let live = live_file_controls(doc);
        let identity = doc.identity();
        self.files
            .retain(|file| file.document == identity && live.contains(&file.control));
    }
}

fn live_file_controls(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    fn walk(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
        let data = doc.get(id).data();
        if data.tag() == Some("input")
            && data
                .attr("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
            && doc.get(id).parent().is_some()
        {
            out.push(id);
        }
        for child in doc.children(id) {
            walk(doc, *child, out);
        }
    }
    walk(doc, doc.root(), &mut out);
    out
}

/// GET or POST. Dialog and unknown methods are GET.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Query on the URL, no request body.
    Get,
    /// Body and content type, no submission query added.
    Post,
}

/// How [`SubmissionEffect::body`] is encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// `application/x-www-form-urlencoded`.
    UrlEncoded,
    /// `multipart/form-data` with this boundary token.
    Multipart {
        /// Boundary token, without the leading `--`.
        boundary: String,
    },
    /// `text/plain` lines.
    TextPlain,
}

/// One form's payload, before it becomes a navigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionEffect {
    /// Trimmed `action`, or empty when the attribute is missing.
    pub action: String,
    /// Form method after a submitter `formmethod` override.
    pub method: Method,
    /// Encoding of `body`.
    pub kind: BodyKind,
    /// Encoded pairs, multipart bytes, or `text/plain` lines.
    pub body: Vec<u8>,
    /// Submitter control, when the activation named one.
    pub submitter: Option<NodeId>,
    /// [`Document::mutation_generation`] at build time.
    pub generation: u64,
}

/// Why a submission did not become a navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitBlock {
    /// A successful control is invalid, and validation was not skipped.
    Invalid,
    /// The submit event was cancelled.
    Cancelled,
    /// `form-action` denied the URL. An absent directive does not do this.
    FormAction,
    /// `javascript:`, `data:`, `file:`, or a non-http(s) POST.
    Scheme,
    /// The action does not resolve against the page. `anvil://` from another page is this.
    Unresolved,
    /// [`HostSubmission::stale_against`] rejected the record. No events and no navigation.
    Stale,
}

/// The request the loader performs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationRequest {
    /// Absolute URL. GET carries the query here.
    pub url: String,
    /// Method to send.
    pub method: Method,
    /// `None` for GET. POST always has one.
    pub content_type: Option<String>,
    /// Empty for GET.
    pub body: Vec<u8>,
}

/// The value shown for a file input: `C:\fakepath\<name>`, or empty.
pub fn exposed_file_input_value(doc: &Document, files: &SelectedFiles, control: NodeId) -> String {
    let Some(file) = files.for_control(doc, control).into_iter().next() else {
        return String::new();
    };
    if file.filename.is_empty() {
        String::new()
    } else {
        format!("C:\\fakepath\\{}", file.filename)
    }
}

/// Build the effect for `form` from the document as it is now.
///
/// `validate` is true for a native activation and for `requestSubmit`.
/// It is false for `form.submit()`, which does not validate.
/// File bytes come from `files` looked up by [`ChosenFormFile::matches`].
/// `data-file-content`, `filename`, and `value` are not file bytes.
pub fn submission_effect(
    doc: &Document,
    form: NodeId,
    submitter: Option<NodeId>,
    files: &SelectedFiles,
    validate: bool,
) -> Result<SubmissionEffect, SubmitBlock> {
    if doc.get(form).data().tag() != Some("form") {
        return Err(SubmitBlock::Unresolved);
    }
    if validate && !skips_validation(doc, form, submitter) && control_invalid(doc, form) {
        return Err(SubmitBlock::Invalid);
    }
    let (action, method, _content_type, _encoded) =
        doc.serialize_form_with_submitter(form, submitter);
    let method = if method == "POST" {
        Method::Post
    } else {
        Method::Get
    };
    let parts = form_parts(doc, form, submitter, files);
    let enctype = doc.form_enctype(form, submitter);
    let (kind, body) = encode_parts(method, &enctype, &parts);
    Ok(SubmissionEffect {
        action,
        method,
        kind,
        body,
        submitter,
        generation: doc.mutation_generation(),
    })
}

/// Resolve `effect` against `page_url` and `csp`.
///
/// GET: query on `url`, empty `body`, no content type.
/// POST: `url` has no submission query, `body` is the payload, content type matches it.
/// A non-http(s) POST is [`SubmitBlock::Scheme`] and is not turned into a GET.
pub fn navigation_from(
    effect: &SubmissionEffect,
    page_url: &str,
    csp: &ContentSecurityPolicy,
) -> Result<NavigationRequest, SubmitBlock> {
    let resolved = resolve_action(page_url, &effect.action)?;
    let (url, content_type, body) = match effect.method {
        Method::Get => (
            with_query(&resolved, &string_body(&effect.body)),
            None,
            Vec::new(),
        ),
        Method::Post => {
            if !is_http_url(&resolved) {
                return Err(SubmitBlock::Scheme);
            }
            let content_type = match &effect.kind {
                BodyKind::UrlEncoded => "application/x-www-form-urlencoded".to_string(),
                BodyKind::Multipart { boundary } => {
                    format!("multipart/form-data; boundary={boundary}")
                }
                BodyKind::TextPlain => "text/plain;charset=UTF-8".to_string(),
            };
            (resolved, Some(content_type), effect.body.clone())
        }
    };
    if !csp.allows_form_action(page_url, &url) {
        return Err(SubmitBlock::FormAction);
    }
    Ok(NavigationRequest {
        url,
        method: effect.method,
        content_type,
        body,
    })
}

/// The document, policy, and file store one submission reads.
pub struct SubmissionContext<'a> {
    /// Page URL the action resolves against.
    pub page_url: &'a str,
    /// `form-action` policy for this page.
    pub csp: &'a ContentSecurityPolicy,
    /// Document the submission is about. Serialized after the gate.
    pub doc: &'a mut Document,
    /// Picker files for this page.
    pub files: &'a mut SelectedFiles,
    /// Events the gate walked. Stays empty when the record is stale.
    pub report: &'a mut GateReport,
}

/// The one production consumer of a [`HostSubmission`].
///
/// Rejects a stale record with no events and no navigation. Otherwise runs
/// the gate, then serializes the current document once.
pub fn consume_host_submission(
    ctx: &mut SubmissionContext<'_>,
    host: &HostSubmission,
    listener: impl FnMut(&mut Document, &str, NodeId, WalkPhase, &mut WalkFlags),
) -> Result<NavigationRequest, SubmitBlock> {
    if host.stale_against(ctx.doc).is_some() {
        return Err(SubmitBlock::Stale);
    }
    activation_gate(
        ctx.doc,
        host.form,
        host.submitter,
        host.mode,
        ctx.report,
        listener,
    )?;
    ctx.files.retain_live(ctx.doc);
    let effect = submission_effect(ctx.doc, host.form, host.submitter, ctx.files, false)?;
    navigation_from(&effect, ctx.page_url, ctx.csp)
}

/// Events the activation gate walked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateReport {
    /// Invalid controls that received an `invalid` event.
    pub invalid_controls: Vec<NodeId>,
    /// True after the submit event was walked.
    pub submit_fired: bool,
    /// The submit event was cancelled.
    pub submit_cancelled: bool,
    /// `(type_name, node, phase)` in walk order.
    pub events: Vec<(String, NodeId, WalkPhase)>,
}

/// Steps left in the script turn that called the gate.
///
/// The runtime shares this with the calling script. Each listener phase
/// spends one step. Zero steps stops the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitBudget {
    /// Remaining listener phases. `u32::MAX` is a browser activation that is
    /// not inside a script turn.
    pub steps: u32,
}

impl SubmitBudget {
    /// A browser click or Enter key, not nested in a script turn.
    pub fn browser() -> Self {
        Self { steps: u32::MAX }
    }

    fn charge(&mut self) -> bool {
        if self.steps == 0 {
            return false;
        }
        self.steps -= 1;
        true
    }
}

/// What [`run_submit_gate`] decided. Navigation is not one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    /// Validation passed and `submit` was not cancelled. The runtime records
    /// [`anvil_dom::HostSubmission`] after this and navigates at the end of the task.
    Proceed,
    /// At least one control is invalid. `invalid` ran. `submit` did not.
    Invalid,
    /// A `submit` handler cancelled the event. No navigation.
    Cancelled,
    /// The shared budget ran out inside the gate. Handlers are not replayed
    /// and there is no default action.
    Exhausted,
}

/// Synchronous validation and event dispatch for `requestSubmit`.
///
/// Callable reentrantly from a host callback inside the script turn that
/// invoked `requestSubmit`. `dispatch` runs one listener phase. It returns
/// [`GateOutcome::Exhausted`] when the shared budget is spent inside a
/// handler. This function does not queue the event again and does not navigate.
/// `Direct` (`form.submit()`) skips validation and events and returns
/// [`GateOutcome::Proceed`].
pub fn run_submit_gate(
    doc: &mut Document,
    form: NodeId,
    submitter: Option<NodeId>,
    mode: SubmitDispatch,
    budget: &mut SubmitBudget,
    report: &mut GateReport,
    mut dispatch: impl FnMut(
        &mut Document,
        &str,
        NodeId,
        WalkPhase,
        &mut WalkFlags,
        &mut SubmitBudget,
    ) -> Result<(), GateOutcome>,
) -> GateOutcome {
    if mode == SubmitDispatch::Direct {
        return GateOutcome::Proceed;
    }
    if !budget.charge() {
        return GateOutcome::Exhausted;
    }
    if !skips_validation(doc, form, submitter) {
        let invalid = invalid_controls(doc, form);
        for control in invalid.clone() {
            if let Err(outcome) =
                fire_budgeted(doc, control, "invalid", report, budget, &mut dispatch)
            {
                return outcome;
            }
        }
        if !invalid.is_empty() {
            report.invalid_controls = invalid;
            return GateOutcome::Invalid;
        }
    }
    match fire_budgeted(doc, form, "submit", report, budget, &mut dispatch) {
        Err(outcome) => outcome,
        Ok(flags) => {
            report.submit_fired = true;
            if flags.default_prevented {
                report.submit_cancelled = true;
                GateOutcome::Cancelled
            } else {
                GateOutcome::Proceed
            }
        }
    }
}

fn fire_budgeted(
    doc: &mut Document,
    target: NodeId,
    type_name: &str,
    report: &mut GateReport,
    budget: &mut SubmitBudget,
    dispatch: &mut impl FnMut(
        &mut Document,
        &str,
        NodeId,
        WalkPhase,
        &mut WalkFlags,
        &mut SubmitBudget,
    ) -> Result<(), GateOutcome>,
) -> Result<WalkFlags, GateOutcome> {
    let mut flags = WalkFlags::for_type(type_name).unwrap_or(WalkFlags {
        bubbles: false,
        cancelable: false,
        default_prevented: false,
        propagation_stopped: false,
        immediate_stopped: false,
    });
    let chain = ancestor_chain(doc, target);
    let name = type_name.to_string();
    let mut visit = |doc: &mut Document,
                     id: NodeId,
                     phase: WalkPhase,
                     flags: &mut WalkFlags,
                     budget: &mut SubmitBudget|
     -> Result<(), GateOutcome> {
        if !budget.charge() {
            return Err(GateOutcome::Exhausted);
        }
        report.events.push((name.clone(), id, phase));
        dispatch(doc, type_name, id, phase, flags, budget)
    };
    if chain.len() > 1 {
        for id in chain[1..].iter().rev().copied() {
            if flags.propagation_stopped {
                return Ok(flags);
            }
            flags.immediate_stopped = false;
            visit(doc, id, WalkPhase::Capture, &mut flags, budget)?;
            if flags.propagation_stopped {
                return Ok(flags);
            }
        }
    }
    if flags.propagation_stopped {
        return Ok(flags);
    }
    flags.immediate_stopped = false;
    visit(doc, target, WalkPhase::Target, &mut flags, budget)?;
    if flags.propagation_stopped || !flags.bubbles {
        return Ok(flags);
    }
    for id in chain.iter().skip(1).copied() {
        if flags.propagation_stopped {
            return Ok(flags);
        }
        flags.immediate_stopped = false;
        visit(doc, id, WalkPhase::Bubble, &mut flags, budget)?;
    }
    Ok(flags)
}

/// Interactive validation and the submit event.
///
/// `Interactive` fires `invalid` on each invalid control and does not fire
/// `submit` or navigate when any control is invalid, unless `novalidate` or
/// `formnovalidate` is set. A successful interactive activation fires `submit`.
/// `Direct` skips both. `listener` runs for each node of each event and may
/// cancel a cancelable event through [`WalkFlags::prevent_default`].
/// This is [`run_submit_gate`] with a browser budget. A script turn must call
/// [`run_submit_gate`] itself so the handlers run before the next statement.
pub fn activation_gate(
    doc: &mut Document,
    form: NodeId,
    submitter: Option<NodeId>,
    mode: SubmitDispatch,
    report: &mut GateReport,
    mut listener: impl FnMut(&mut Document, &str, NodeId, WalkPhase, &mut WalkFlags),
) -> Result<(), SubmitBlock> {
    let mut budget = SubmitBudget::browser();
    // Snapshot once inside [`run_submit_gate`]. A listener that repairs a
    // value does not get a second check.
    match run_submit_gate(
        doc,
        form,
        submitter,
        mode,
        &mut budget,
        report,
        |doc, name, id, phase, flags, _budget| {
            listener(doc, name, id, phase, flags);
            Ok(())
        },
    ) {
        GateOutcome::Proceed => Ok(()),
        GateOutcome::Invalid => Err(SubmitBlock::Invalid),
        GateOutcome::Cancelled => Err(SubmitBlock::Cancelled),
        GateOutcome::Exhausted => Err(SubmitBlock::Cancelled),
    }
}

fn ancestor_chain(doc: &Document, target: NodeId) -> Vec<NodeId> {
    let mut chain = vec![target];
    let mut ancestor = doc.get(target).parent();
    while let Some(id) = ancestor {
        chain.push(id);
        ancestor = doc.get(id).parent();
    }
    chain
}

fn invalid_controls(doc: &Document, form: NodeId) -> Vec<NodeId> {
    fn walk(doc: &Document, id: NodeId, form: NodeId, out: &mut Vec<NodeId>) {
        if id != form
            && doc.enclosing_form(id) == Some(form)
            && doc.is_control_valid(id) == Some(false)
        {
            out.push(id);
        }
        for child in doc.children(id) {
            walk(doc, *child, form, out);
        }
    }
    let mut out = Vec::new();
    walk(doc, doc.root(), form, &mut out);
    out
}

/// [`super::ClickOutcome`] for a navigation request.
pub fn click_outcome(req: NavigationRequest) -> super::ClickOutcome {
    match req.method {
        Method::Get => super::ClickOutcome::Navigate(req.url),
        Method::Post => super::ClickOutcome::SubmitPost {
            url: req.url,
            body: req.body,
            content_type: req
                .content_type
                .unwrap_or_else(|| "application/x-www-form-urlencoded".to_string()),
        },
    }
}

fn resolve_action(page_url: &str, action: &str) -> Result<String, SubmitBlock> {
    let trimmed = action.trim();
    if let Some(block) = scheme_block(trimmed) {
        return Err(block);
    }
    if trimmed.to_ascii_lowercase().starts_with("anvil:") && scheme_of(page_url) != "anvil" {
        return Err(SubmitBlock::Unresolved);
    }
    let target = if trimmed.is_empty() {
        page_url
    } else {
        trimmed
    };
    match resolve_url(page_url, target) {
        Some(url) => {
            if let Some(block) = scheme_block(&url) {
                return Err(block);
            }
            Ok(url)
        }
        None => Err(SubmitBlock::Unresolved),
    }
}

fn scheme_block(url: &str) -> Option<SubmitBlock> {
    // A C0 control or DEL is not fetched. Tab and newline hide `javascript:`
    // and `data:`; any other control, including an embedded U+0001, is refused
    // here. A clean unknown scheme such as `foo:bar` is left to resolution.
    if url.chars().any(|ch| ch == '\u{007F}' || ch <= '\u{001F}') {
        return Some(SubmitBlock::Scheme);
    }
    if crate::form_registry::classify_navigation_scheme(url)
        == crate::form_registry::NavigationScheme::Unsupported
    {
        return Some(SubmitBlock::Scheme);
    }
    match scheme_of(url).as_str() {
        "javascript" | "data" | "file" => Some(SubmitBlock::Scheme),
        _ => None,
    }
}

fn scheme_of(url: &str) -> String {
    url.split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn is_http_url(url: &str) -> bool {
    let scheme = scheme_of(url);
    scheme == "http" || scheme == "https"
}

pub(crate) fn interactive_submission_blocked(
    doc: &Document,
    form: NodeId,
    submitter: Option<NodeId>,
) -> bool {
    !skips_validation(doc, form, submitter) && control_invalid(doc, form)
}

fn skips_validation(doc: &Document, form: NodeId, submitter: Option<NodeId>) -> bool {
    if doc.is_dialog_form_submission(form, submitter) {
        return true;
    }
    if doc.get(form).data().attr("novalidate").is_some() {
        return true;
    }
    submitter.is_some_and(|id| doc.get(id).data().attr("formnovalidate").is_some())
}

fn control_invalid(doc: &Document, form: NodeId) -> bool {
    fn walk(doc: &Document, id: NodeId, form: NodeId) -> bool {
        if id != form
            && doc.enclosing_form(id) == Some(form)
            && doc.is_control_valid(id) == Some(false)
        {
            return true;
        }
        doc.children(id).iter().any(|child| walk(doc, *child, form))
    }
    walk(doc, doc.root(), form)
}

fn push_submitter_entry(
    doc: &Document,
    submitter: Option<NodeId>,
    entries: &mut Vec<FormMultipartEntry>,
) {
    let Some(id) = submitter else {
        return;
    };
    let data = doc.get(id).data();
    if data.attr("disabled").is_some() {
        return;
    }
    let Some(name) = data.attr("name") else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    entries.push(FormMultipartEntry {
        name: name.to_string(),
        value: data.attr("value").unwrap_or("").to_string(),
        filename: None,
        content_type: None,
    });
}

enum OwnedPart {
    Text {
        name: String,
        value: String,
    },
    File {
        name: String,
        filename: String,
        content_type: String,
        bytes: Vec<u8>,
    },
}

fn form_parts(
    doc: &Document,
    form: NodeId,
    submitter: Option<NodeId>,
    files: &SelectedFiles,
) -> Vec<OwnedPart> {
    let mut entries = doc.collect_form_entries(form);
    push_submitter_entry(doc, submitter, &mut entries);
    let file_ids = file_controls(doc, form);
    let mut next_file = 0usize;
    let mut parts = Vec::new();
    for entry in entries {
        if entry.filename.is_none() {
            parts.push(OwnedPart::Text {
                name: entry.name,
                value: entry.value,
            });
            continue;
        }
        let control = file_ids.get(next_file).copied();
        next_file += 1;
        let chosen = control
            .map(|id| files.for_control(doc, id))
            .unwrap_or_default();
        if chosen.is_empty() {
            parts.push(OwnedPart::File {
                name: entry.name,
                filename: String::new(),
                content_type: "application/octet-stream".to_string(),
                bytes: Vec::new(),
            });
            continue;
        }
        for file in chosen {
            let content_type = if file.content_type.trim().is_empty() {
                "application/octet-stream".to_string()
            } else {
                file.content_type.clone()
            };
            parts.push(OwnedPart::File {
                name: entry.name.clone(),
                filename: file.filename.clone(),
                content_type,
                bytes: file.bytes.clone(),
            });
        }
    }
    parts
}

fn file_controls(doc: &Document, form: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let rooted = doc
        .get(form)
        .data()
        .attr("id")
        .is_some_and(|value| !value.trim().is_empty());
    if rooted {
        fn walk(doc: &Document, id: NodeId, form: NodeId, out: &mut Vec<NodeId>) {
            if id != form && doc.enclosing_form(id) == Some(form) {
                push_file(doc, id, out);
            }
            for child in doc.children(id) {
                walk(doc, *child, form, out);
            }
        }
        walk(doc, doc.root(), form, &mut out);
    } else {
        fn walk(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
            push_file(doc, id, out);
            for child in doc.children(id) {
                walk(doc, *child, out);
            }
        }
        walk(doc, form, &mut out);
    }
    out
}

fn push_file(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
    let data = doc.get(id).data();
    if data.attr("disabled").is_some() {
        return;
    }
    if data.tag() == Some("input")
        && data
            .attr("type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
        && data
            .attr("name")
            .is_some_and(|name| !name.trim().is_empty())
    {
        out.push(id);
    }
}

fn encode_parts(method: Method, enctype: &str, parts: &[OwnedPart]) -> (BodyKind, Vec<u8>) {
    if method == Method::Post && enctype == "multipart/form-data" {
        let boundary = anvil_dom::DEFAULT_MULTIPART_BOUNDARY.to_string();
        return (
            BodyKind::Multipart {
                boundary: boundary.clone(),
            },
            multipart_bytes(&boundary, parts),
        );
    }
    if method == Method::Post && enctype == "text/plain" {
        return (BodyKind::TextPlain, text_plain_body(parts).into_bytes());
    }
    (BodyKind::UrlEncoded, urlencoded_body(parts).into_bytes())
}

fn multipart_bytes(boundary: &str, parts: &[OwnedPart]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend(format!("--{boundary}\r\n").into_bytes());
        match part {
            OwnedPart::Text { name, value } => {
                let header = format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                    escape_quoted(name)
                );
                out.extend(header.into_bytes());
                out.extend(value.as_bytes());
                out.extend(b"\r\n");
            }
            OwnedPart::File {
                name,
                filename,
                content_type,
                bytes,
            } => {
                let header = format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
                    escape_quoted(name),
                    escape_quoted(filename)
                );
                out.extend(header.into_bytes());
                out.extend_from_slice(bytes);
                out.extend(b"\r\n");
            }
        }
    }
    out.extend(format!("--{boundary}--\r\n").into_bytes());
    out
}

fn escape_quoted(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn urlencoded_body(parts: &[OwnedPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            OwnedPart::Text { name, value } => Some(format!(
                "{}={}",
                urlencode_component(name),
                urlencode_component(value)
            )),
            OwnedPart::File { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn text_plain_body(parts: &[OwnedPart]) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            OwnedPart::File { name, .. } => {
                out.push_str(&flatten_plain(name));
                out.push('=');
                out.push_str("\r\n");
            }
            OwnedPart::Text { name, value } => {
                out.push_str(&flatten_plain(name));
                out.push('=');
                out.push_str(&flatten_plain(value));
                out.push_str("\r\n");
            }
        }
    }
    out
}

fn flatten_plain(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn string_body(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

fn with_query(url: &str, query: &str) -> String {
    let (without_frag, frag) = split_fragment(url);
    let base = without_frag.split('?').next().unwrap_or(without_frag);
    let mut out = if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    };
    if let Some(fragment) = frag {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn split_fragment(url: &str) -> (&str, Option<&str>) {
    match url.split_once('#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (url, None),
    }
}

/// Byte progress of an in-flight HTTP POST or `multipart/form-data` form upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadProgress {
    /// Bytes written to the transport so far.
    pub sent_bytes: u64,
    /// Total encoded request body size in bytes.
    pub total_bytes: u64,
}

impl UploadProgress {
    /// Upload completion percentage (`0..=100`), or `None` when `sent_bytes > total_bytes`.
    pub fn percent(self) -> Option<u8> {
        if self.total_bytes == 0 {
            return Some(100);
        }
        if self.sent_bytes > self.total_bytes {
            return None;
        }
        Some((self.sent_bytes.saturating_mul(100) / self.total_bytes) as u8)
    }

    /// Human-readable status pill label, e.g. `"Uploading 32.0 KB / 128.0 KB (25%)"`.
    pub fn status_label(self) -> String {
        let sent = crate::downloads::format_byte_size(self.sent_bytes as usize);
        let total = crate::downloads::format_byte_size(self.total_bytes as usize);
        match self.percent() {
            Some(pct) => format!("Uploading {sent} / {total} ({pct}%)"),
            None => format!("Uploading {sent}"),
        }
    }
}

struct LiveUploadAtomics {
    active: AtomicBool,
    sent: AtomicU64,
    total: AtomicU64,
    generation: AtomicU64,
}

static LIVE_UPLOAD: LiveUploadAtomics = LiveUploadAtomics {
    active: AtomicBool::new(false),
    sent: AtomicU64::new(0),
    total: AtomicU64::new(0),
    generation: AtomicU64::new(0),
};

/// Current in-flight POST / multipart upload progress, if an upload is active.
pub fn live_upload() -> Option<UploadProgress> {
    if !LIVE_UPLOAD.active.load(Ordering::Acquire) {
        return None;
    }
    Some(UploadProgress {
        sent_bytes: LIVE_UPLOAD.sent.load(Ordering::Relaxed),
        total_bytes: LIVE_UPLOAD.total.load(Ordering::Relaxed),
    })
}

/// Monotonic generation bumped whenever [`live_upload`] is updated or cleared.
pub fn live_upload_generation() -> u64 {
    LIVE_UPLOAD.generation.load(Ordering::Acquire)
}

/// Publish an in-flight upload byte count for status pill rendering.
pub fn set_live_upload(sent_bytes: u64, total_bytes: u64) {
    LIVE_UPLOAD.sent.store(sent_bytes, Ordering::Relaxed);
    LIVE_UPLOAD.total.store(total_bytes, Ordering::Relaxed);
    LIVE_UPLOAD.active.store(true, Ordering::Release);
    LIVE_UPLOAD.generation.fetch_add(1, Ordering::Release);
}

/// Clear any active in-flight upload state.
pub fn clear_live_upload() {
    if LIVE_UPLOAD.active.swap(false, Ordering::Release) {
        LIVE_UPLOAD.generation.fetch_add(1, Ordering::Release);
    }
}

struct LiveUploadGuard;

impl Drop for LiveUploadGuard {
    fn drop(&mut self) {
        clear_live_upload();
    }
}

/// A form hop was refused or the socket failed.
#[derive(Debug)]
pub enum ExchangeError {
    /// `anvil-net` failed.
    Net(NetError),
    /// `form-action` denied this hop. Nothing was connected and no cookie was attached.
    FormAction,
}

/// Send one form navigation, checking `form-action` and cookies on every hop.
///
/// The cookie decision is made before the request is built. A hop gets cookies
/// only when it is same-origin with `page_url` and no earlier hop was
/// cross-origin. A denied hop returns before connect.
pub fn exchange_form(
    page_url: &str,
    url: &str,
    use_post: bool,
    content_type: &str,
    body: &[u8],
    jar: &mut CookieJar,
    csp: &ContentSecurityPolicy,
) -> Result<(String, anvil_net::Response), ExchangeError> {
    exchange_form_with_progress(
        page_url,
        url,
        use_post,
        content_type,
        body,
        jar,
        csp,
        |_| {},
    )
}

/// Like [`exchange_form`], additionally reporting chunked POST / multipart upload progress.
#[allow(clippy::too_many_arguments)]
pub fn exchange_form_with_progress<F: FnMut(UploadProgress)>(
    page_url: &str,
    url: &str,
    mut use_post: bool,
    content_type: &str,
    body: &[u8],
    jar: &mut CookieJar,
    csp: &ContentSecurityPolicy,
    mut on_progress: F,
) -> Result<(String, anvil_net::Response), ExchangeError> {
    let mut current = url.trim().to_string();
    let mut tainted = false;
    for _ in 0..=MAX_REDIRECTS {
        if !csp.allows_form_action(page_url, &current) {
            return Err(ExchangeError::FormAction);
        }
        if let Some(reason) = is_url_blocked_by_default_shield(&current, Some(page_url)) {
            return Err(ExchangeError::Net(NetError::BlockedByShield(
                reason.to_string(),
            )));
        }
        let send_cookies = !tainted && is_same_origin(page_url, &current);
        let cookie = if send_cookies {
            jar.http_cookie_header_under(&current, page_url, Some(page_url), !use_post)
        } else {
            None
        };
        let parsed = parse_url(&current).map_err(ExchangeError::Net)?;
        let response = if use_post {
            let _upload_guard = LiveUploadGuard;
            http_post_with_cookie_progress(
                &parsed,
                content_type,
                body,
                cookie.as_deref(),
                |sent, total| {
                    set_live_upload(sent, total);
                    on_progress(UploadProgress {
                        sent_bytes: sent,
                        total_bytes: total,
                    });
                },
            )
        } else {
            http_get_with_cookie(&parsed, cookie.as_deref())
        }
        .map_err(ExchangeError::Net)?;
        if !is_same_origin(page_url, &current) {
            tainted = true;
        }
        for set_cookie in response.headers_named("set-cookie") {
            jar.store_set_cookie_under(&current, page_url, set_cookie);
        }
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            if matches!(response.status, 301..=303) {
                use_post = false;
            }
            let location = response
                .header("location")
                .ok_or(ExchangeError::Net(NetError::InvalidResponse))?;
            current =
                resolve_url(&current, location).ok_or(ExchangeError::Net(NetError::InvalidUrl))?;
            continue;
        }
        return Ok((current, response));
    }
    Err(ExchangeError::Net(NetError::TooManyRedirects))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_dom::WalkPhase;
    use anvil_dom::parse_html;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn doc(html: &str) -> Document {
        parse_html(html)
    }

    fn form_id(document: &Document) -> NodeId {
        document.get_element_by_id("f").expect("form")
    }

    fn nav(html: &str, page: &str, validate: bool) -> Result<NavigationRequest, SubmitBlock> {
        let document = doc(html);
        let form = form_id(&document);
        let effect = submission_effect(&document, form, None, &SelectedFiles::new(), validate)?;
        navigation_from(&effect, page, &ContentSecurityPolicy::default())
    }

    fn read_http(listener: TcpListener) -> String {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            match sock.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..split]).to_string();
                        let length = head.lines().find_map(|line| {
                            let (name, val) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                val.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        });
                        let need = split + 4 + length.unwrap_or(0);
                        if length.is_none() || buf.len() >= need {
                            buf.truncate(need);
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let body = b"ok";
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(header.as_bytes());
        let _ = sock.write_all(body);
        String::from_utf8(buf).unwrap_or_default()
    }

    #[test]
    fn get_navigation_has_an_empty_body_and_the_query_on_the_url() {
        let request = nav(
            "<form id=\"f\" action=\"/search\" method=\"GET\"><input name=\"q\" value=\"a b\"></form>",
            "http://127.0.0.1/page",
            true,
        )
        .unwrap();
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.url, "http://127.0.0.1/search?q=a+b");
        assert!(request.body.is_empty());
        assert_eq!(request.content_type, None);
    }

    #[test]
    fn post_urlencoded_keeps_the_body_off_the_url() {
        let request = nav(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"anvil\"></form>",
            "http://127.0.0.1/page",
            true,
        )
        .unwrap();
        assert_eq!(request.url, "http://127.0.0.1/save");
        assert_eq!(request.body, b"q=anvil");
        assert_eq!(
            request.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn post_text_plain_is_crlf_lines_with_its_content_type() {
        let request = nav(
            "<form id=\"f\" action=\"/save\" method=\"POST\" enctype=\"text/plain\"><input name=\"q\" value=\"a b\"></form>",
            "http://127.0.0.1/page",
            true,
        )
        .unwrap();
        assert_eq!(request.body, b"q=a b\r\n");
        assert_eq!(
            request.content_type.as_deref(),
            Some("text/plain;charset=UTF-8")
        );
    }

    #[test]
    fn post_multipart_boundary_matches_and_attribute_files_are_empty() {
        let request = nav(
            "<form id=\"f\" action=\"/save\" method=\"POST\" enctype=\"multipart/form-data\">\
                <input name=\"q\" value=\"anvil\">\
                <input type=\"file\" name=\"upload\" filename=\"note.txt\" data-file-content=\"SECRET\" value=\"note.txt\">\
             </form>",
            "http://127.0.0.1/page",
            true,
        )
        .unwrap();
        let content_type = request.content_type.expect("content type");
        let boundary = content_type.split("boundary=").nth(1).expect("boundary");
        let body = String::from_utf8(request.body).unwrap();
        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(body.contains(boundary), "{body}");
        assert!(body.contains("name=\"q\""));
        assert!(body.contains("anvil"));
        assert!(!body.contains("SECRET"));
        assert!(!body.contains("note.txt"));
        assert!(body.contains("filename=\"\""));
    }

    #[test]
    fn a_chosen_file_is_the_only_file_bytes() {
        let document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\" enctype=\"multipart/form-data\">\
                <input type=\"file\" name=\"upload\" data-file-content=\"SECRET\">\
             </form>",
        );
        let form = form_id(&document);
        let control = document
            .children(form)
            .iter()
            .find(|id| document.get(**id).data().attr("type") == Some("file"))
            .copied()
            .expect("file");
        let mut files = SelectedFiles::new();
        files.choose_from_picker(ChosenFormFile {
            document: document.identity(),
            control,
            filename: "picked.txt".to_string(),
            content_type: "text/plain".to_string(),
            bytes: b"from-picker".to_vec(),
        });
        let effect = submission_effect(&document, form, None, &files, true).unwrap();
        let body = String::from_utf8(effect.body).unwrap();
        assert!(body.contains("filename=\"picked.txt\""));
        assert!(body.contains("from-picker"));
        assert!(!body.contains("SECRET"));
    }

    #[test]
    fn invalid_email_blocks_until_novalidate() {
        let html = "<form id=\"f\" action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"not-an-email\"></form>";
        assert_eq!(
            nav(html, "http://127.0.0.1/page", true).err(),
            Some(SubmitBlock::Invalid)
        );
        let skipped = nav(
            "<form id=\"f\" action=\"/save\" method=\"POST\" novalidate><input type=\"email\" name=\"em\" value=\"not-an-email\"></form>",
            "http://127.0.0.1/page",
            true,
        )
        .unwrap();
        assert_eq!(skipped.body, b"em=not-an-email");
        let document = doc(html);
        let direct = submission_effect(
            &document,
            form_id(&document),
            None,
            &SelectedFiles::new(),
            false,
        )
        .unwrap();
        assert_eq!(direct.body, b"em=not-an-email");
    }

    #[test]
    fn scheme_and_form_action_block_before_a_request() {
        assert_eq!(
            nav(
                "<form id=\"f\" action=\"javascript:alert(1)\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Scheme)
        );
        assert_eq!(
            nav(
                "<form id=\"f\" action=\" JavaScript:x\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Scheme)
        );
        assert_eq!(
            nav(
                "<form id=\"f\" action=\"data:text/html,hi\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Scheme)
        );
        assert_eq!(
            nav(
                "<form id=\"f\" action=\"foo:bar\" method=\"GET\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Unresolved)
        );
        assert_eq!(
            nav(
                "<form id=\"f\" action=\"anvil://home\" method=\"GET\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Unresolved)
        );
        assert_eq!(
            nav(
                "<form id=\"f\" action=\"file:///tmp/x\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
                "http://127.0.0.1/page",
                true
            )
            .err(),
            Some(SubmitBlock::Scheme)
        );
        let document = doc(
            "<form id=\"f\" action=\"https://evil.test/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
        );
        let effect = submission_effect(
            &document,
            form_id(&document),
            None,
            &SelectedFiles::new(),
            true,
        )
        .unwrap();
        let absent = ContentSecurityPolicy::parse("default-src 'none'");
        assert!(
            navigation_from(&effect, "http://127.0.0.1/page", &absent).is_ok(),
            "absent form-action allows the action when default-src is 'none'"
        );
        let empty = ContentSecurityPolicy::parse("form-action");
        assert_eq!(
            navigation_from(&effect, "http://127.0.0.1/page", &empty).err(),
            Some(SubmitBlock::FormAction)
        );
        let denied = ContentSecurityPolicy::parse("form-action 'none'");
        assert_eq!(
            navigation_from(&effect, "http://127.0.0.1/page", &denied).err(),
            Some(SubmitBlock::FormAction)
        );
    }

    #[test]
    fn a_non_http_post_is_not_rebuilt_as_get() {
        let document = doc(
            "<form id=\"f\" action=\"anvil://home\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
        );
        let effect = submission_effect(
            &document,
            form_id(&document),
            None,
            &SelectedFiles::new(),
            true,
        )
        .unwrap();
        assert_eq!(effect.method, Method::Post);
        assert_eq!(
            navigation_from(&effect, "anvil://home", &ContentSecurityPolicy::default()).err(),
            Some(SubmitBlock::Scheme)
        );
    }

    #[test]
    fn consume_rebuilds_get_and_post_from_the_document() {
        let page = "http://127.0.0.1/";
        let csp = ContentSecurityPolicy::default();
        let mut get_doc = doc(
            "<form id=\"f\" action=\"/search\" method=\"GET\"><input name=\"q\" value=\"anvil\"></form>",
        );
        let get_form = form_id(&get_doc);
        let get_host = HostSubmission {
            form: get_form,
            submitter: None,
            mode: SubmitDispatch::Direct,
            generation: get_doc.mutation_generation(),
            document: get_doc.identity(),
        };
        let mut files = SelectedFiles::new();
        let mut report = GateReport::default();
        let get_req = consume_host_submission(
            &mut SubmissionContext {
                page_url: page,
                csp: &csp,
                doc: &mut get_doc,
                files: &mut files,
                report: &mut report,
            },
            &get_host,
            |_, _, _, _, _| {},
        )
        .unwrap();
        assert_eq!(get_req.url, "http://127.0.0.1/search?q=anvil");
        assert!(get_req.body.is_empty());
        assert_eq!(get_req.content_type, None);

        let mut post_doc = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"anvil\"></form>",
        );
        let post_form = form_id(&post_doc);
        let post_host = HostSubmission {
            form: post_form,
            submitter: None,
            mode: SubmitDispatch::Direct,
            generation: post_doc.mutation_generation(),
            document: post_doc.identity(),
        };
        let mut report = GateReport::default();
        let post_req = consume_host_submission(
            &mut SubmissionContext {
                page_url: page,
                csp: &csp,
                doc: &mut post_doc,
                files: &mut files,
                report: &mut report,
            },
            &post_host,
            |_, _, _, _, _| {},
        )
        .unwrap();
        assert_eq!(post_req.url, "http://127.0.0.1/save");
        assert_eq!(post_req.body, b"q=anvil");
        assert_eq!(
            post_req.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn the_get_request_on_the_wire_has_no_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(read_http(listener));
        });
        let url = format!("http://127.0.0.1:{}/search?q=anvil", addr.port());
        let request = NavigationRequest {
            url: url.clone(),
            method: Method::Get,
            content_type: None,
            body: Vec::new(),
        };
        assert!(request.body.is_empty());
        let (_final, response) = crate::navigation_get(&request.url, None).unwrap();
        assert_eq!(response.body, b"ok");
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("GET /search?q=anvil HTTP/1.1\r\n"), "{raw}");
        let head_end = raw.find("\r\n\r\n").expect("headers");
        assert!(
            !raw[..head_end]
                .to_ascii_lowercase()
                .contains("content-length"),
            "{raw}"
        );
        assert_eq!(&raw[head_end + 4..], "");
    }

    #[test]
    fn the_post_request_on_the_wire_sends_the_multipart_type_and_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(read_http(listener));
        });
        let html = format!(
            "<form id=\"f\" action=\"http://127.0.0.1:{}/save\" method=\"POST\" enctype=\"multipart/form-data\"><input name=\"q\" value=\"anvil\"></form>",
            addr.port()
        );
        let request = nav(
            &html,
            &format!("http://127.0.0.1:{}/page", addr.port()),
            true,
        )
        .unwrap();
        let content_type = request.content_type.clone().unwrap();
        let body = String::from_utf8(request.body.clone()).unwrap();
        let (_final, response) = crate::navigation_post(
            &request.url,
            request.body.as_slice(),
            &content_type,
            None,
            &request.url,
            &ContentSecurityPolicy::default(),
        )
        .unwrap();
        assert_eq!(response.body, b"ok");
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("POST /save HTTP/1.1\r\n"), "{raw}");
        assert!(!raw.contains("POST /save?"), "{raw}");
        assert!(
            raw.contains(&format!("Content-Type: {content_type}\r\n")),
            "{raw}"
        );
        assert!(raw.contains(&body), "{raw}");
    }

    #[test]
    fn interactive_request_submit_fires_invalid_and_does_not_submit() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"em\" type=\"email\" name=\"em\" value=\"not-an-email\"></form>",
        );
        let form = form_id(&document);
        let mut report = GateReport::default();
        let err = activation_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut report,
            |_, _, _, _, _| {},
        )
        .unwrap_err();
        assert_eq!(err, SubmitBlock::Invalid);
        assert!(!report.submit_fired);
        assert!(!report.invalid_controls.is_empty());
        assert!(
            report
                .events
                .iter()
                .any(|(name, _, phase)| { name == "invalid" && *phase == WalkPhase::Target })
        );
        let host = HostSubmission {
            form,
            submitter: None,
            mode: SubmitDispatch::Interactive,
            generation: document.mutation_generation(),
            document: document.identity(),
        };
        let mut files = SelectedFiles::new();
        let blocked = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut document,
                files: &mut files,
                report: &mut GateReport::default(),
            },
            &host,
            |_, _, _, _, _| {},
        );
        assert_eq!(blocked.err(), Some(SubmitBlock::Invalid));
    }

    #[test]
    fn direct_submit_skips_validation_and_the_submit_event() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"not-an-email\"></form>",
        );
        let form = form_id(&document);
        let mut report = GateReport::default();
        assert!(
            activation_gate(
                &mut document,
                form,
                None,
                SubmitDispatch::Direct,
                &mut report,
                |_, _, _, _, _| {},
            )
            .is_ok()
        );
        assert!(report.events.is_empty());
        assert!(!report.submit_fired);
        let host = HostSubmission {
            form,
            submitter: None,
            mode: SubmitDispatch::Direct,
            generation: document.mutation_generation(),
            document: document.identity(),
        };
        let mut files = SelectedFiles::new();
        let sent = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut document,
                files: &mut files,
                report: &mut report,
            },
            &host,
            |_, _, _, _, _| {},
        )
        .unwrap();
        assert_eq!(sent.body, b"em=not-an-email");
    }

    #[test]
    fn novalidate_and_formnovalidate_still_fire_submit() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\" novalidate><input type=\"email\" name=\"em\" value=\"not-an-email\"></form>",
        );
        let form = form_id(&document);
        let mut report = GateReport::default();
        assert!(
            activation_gate(
                &mut document,
                form,
                None,
                SubmitDispatch::Interactive,
                &mut report,
                |_, _, _, _, _| {},
            )
            .is_ok()
        );
        assert!(report.invalid_controls.is_empty());
        assert!(report.submit_fired);

        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"not-an-email\"><button id=\"go\" formnovalidate>Go</button></form>",
        );
        let form = form_id(&document);
        let submitter = document.get_element_by_id("go");
        let mut report = GateReport::default();
        assert!(
            activation_gate(
                &mut document,
                form,
                submitter,
                SubmitDispatch::Interactive,
                &mut report,
                |_, _, _, _, flags| flags.prevent_default(),
            )
            .is_err()
        );
        assert!(report.submit_fired);
        assert!(report.submit_cancelled);
    }

    fn serve(response: impl Into<String>) -> (std::net::SocketAddr, mpsc::Receiver<Vec<u8>>) {
        let response = response.into();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                let _ = tx.send(Vec::new());
                return;
            };
            sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let _ = sock
                .read(&mut tmp)
                .map(|n| buf.extend_from_slice(&tmp[..n]));
            let _ = tx.send(buf);
            let _ = sock.write_all(response.as_bytes());
        });
        (addr, rx)
    }

    #[test]
    fn form_action_is_checked_on_every_hop_before_cookies() {
        let blocked = TcpListener::bind("127.0.0.1:0").unwrap();
        let blocked_addr = blocked.local_addr().unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        thread::spawn(move || {
            blocked.set_nonblocking(true).ok();
            std::thread::sleep(Duration::from_millis(200));
            let hit = blocked.accept().is_ok();
            let _ = blocked_tx.send(hit);
        });

        let next = format!("http://127.0.0.1:{}/nope", blocked_addr.port());
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: {next}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (allowed, allowed_rx) = serve(redirect);
        let page = format!("http://127.0.0.1:{}/page", allowed.port());
        let action = format!("http://127.0.0.1:{}/save", allowed.port());
        let mut jar = CookieJar::new();
        jar.store_set_cookie_under(&page, &page, "session=abc; Path=/");
        let csp = ContentSecurityPolicy::parse(&format!(
            "form-action http://127.0.0.1:{}",
            allowed.port()
        ));
        let err = exchange_form(&page, &action, true, "text/plain", b"q=1", &mut jar, &csp);
        assert!(matches!(err, Err(ExchangeError::FormAction)));
        let raw = String::from_utf8_lossy(&allowed_rx.recv().unwrap()).to_string();
        assert!(raw.starts_with("POST /save "), "{raw}");
        assert!(!blocked_rx.recv().unwrap(), "blocked hop was connected");
    }

    #[test]
    fn a_redirect_chain_is_checked_at_each_hop() {
        let (final_addr, final_rx) =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let to_final = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            final_addr.port()
        );
        let (mid, mid_rx) = serve(to_final);
        let to_mid = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/mid\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            mid.port()
        );
        let (start, start_rx) = serve(to_mid);
        let page = format!("http://127.0.0.1:{}/page", start.port());
        let action = format!("http://127.0.0.1:{}/go", start.port());
        let csp = ContentSecurityPolicy::parse(&format!(
            "form-action http://127.0.0.1:{} http://127.0.0.1:{} http://127.0.0.1:{}",
            start.port(),
            mid.port(),
            final_addr.port()
        ));
        let mut jar = CookieJar::new();
        let (_url, response) =
            exchange_form(&page, &action, false, "", b"", &mut jar, &csp).unwrap();
        assert_eq!(response.body, b"ok");
        assert!(
            String::from_utf8_lossy(&start_rx.recv().unwrap()).starts_with("GET /go "),
            "first hop"
        );
        assert!(
            String::from_utf8_lossy(&mid_rx.recv().unwrap()).starts_with("GET /mid "),
            "second hop"
        );
        assert!(
            String::from_utf8_lossy(&final_rx.recv().unwrap()).starts_with("GET /end "),
            "final hop"
        );
    }

    fn host_for(doc: &Document, form: NodeId, mode: SubmitDispatch) -> HostSubmission {
        HostSubmission {
            form,
            submitter: None,
            mode,
            generation: doc.mutation_generation(),
            document: doc.identity(),
        }
    }

    #[test]
    fn a_wrong_document_or_generation_is_stale_and_does_not_fire() {
        let mut doc = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"q\" name=\"q\" value=\"a\"></form>",
        );
        let form = form_id(&doc);
        let mut host = host_for(&doc, form, SubmitDispatch::Interactive);
        host.document = host.document.wrapping_add(1);
        let mut files = SelectedFiles::new();
        let mut report = GateReport::default();
        let err = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut doc,
                files: &mut files,
                report: &mut report,
            },
            &host,
            |_, _, _, _, _| {},
        );
        assert_eq!(err.err(), Some(SubmitBlock::Stale));
        assert!(report.events.is_empty());

        let host = host_for(&doc, form, SubmitDispatch::Interactive);
        doc.set_input_value(doc.get_element_by_id("q").unwrap(), "changed");
        let mut report = GateReport::default();
        let err = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut doc,
                files: &mut files,
                report: &mut report,
            },
            &host,
            |_, _, _, _, _| {},
        );
        assert_eq!(err.err(), Some(SubmitBlock::Stale));
        assert!(report.events.is_empty());
    }

    #[test]
    fn a_submit_listener_edit_is_serialized_and_stays_dirty() {
        let mut doc = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"q\" name=\"q\" value=\"old\"></form>",
        );
        let form = form_id(&doc);
        let field = doc.get_element_by_id("q").unwrap();
        let host = host_for(&doc, form, SubmitDispatch::Interactive);
        let mut files = SelectedFiles::new();
        let mut report = GateReport::default();
        let sent = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut doc,
                files: &mut files,
                report: &mut report,
            },
            &host,
            |document, name, _node, _phase, _flags| {
                if name == "submit" {
                    document.set_input_value(field, "edited");
                }
            },
        )
        .unwrap();
        assert_eq!(sent.body, b"q=edited");
        assert!(doc.has_dirty_styles());
    }

    #[test]
    fn an_invalid_listener_repair_does_not_submit() {
        let mut doc = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input id=\"em\" type=\"email\" name=\"em\" value=\"bad\"></form>",
        );
        let form = form_id(&doc);
        let field = doc.get_element_by_id("em").unwrap();
        let host = host_for(&doc, form, SubmitDispatch::Interactive);
        let mut files = SelectedFiles::new();
        let mut report = GateReport::default();
        let err = consume_host_submission(
            &mut SubmissionContext {
                page_url: "http://127.0.0.1/",
                csp: &ContentSecurityPolicy::default(),
                doc: &mut doc,
                files: &mut files,
                report: &mut report,
            },
            &host,
            |document, name, _node, _phase, _flags| {
                if name == "invalid" {
                    document.set_input_value(field, "ada@example.com");
                }
            },
        );
        assert_eq!(err.err(), Some(SubmitBlock::Invalid));
        assert!(!report.submit_fired);
        assert_eq!(doc.input_value(field), "ada@example.com");
    }

    #[test]
    fn picker_files_follow_the_control_and_not_the_name() {
        let mut doc = doc(
            "<form id=\"f\" method=\"POST\" enctype=\"multipart/form-data\" action=\"/up\">\
               <input id=\"a\" type=\"file\" name=\"up\">\
               <input id=\"b\" type=\"file\" name=\"up\" data-file-content=\"SECRET\">\
             </form>",
        );
        let form = form_id(&doc);
        let a = doc.get_element_by_id("a").unwrap();
        let b = doc.get_element_by_id("b").unwrap();
        let mut files = SelectedFiles::new();
        files.choose_from_picker(ChosenFormFile {
            document: doc.identity(),
            control: a,
            filename: "a.txt".into(),
            content_type: "text/plain".into(),
            bytes: b"AAA".to_vec(),
        });
        files.choose_from_picker(ChosenFormFile {
            document: doc.identity(),
            control: b,
            filename: "b.txt".into(),
            content_type: "text/plain".into(),
            bytes: b"BBB".to_vec(),
        });
        let effect = submission_effect(&doc, form, None, &files, false).unwrap();
        let body = String::from_utf8(effect.body).unwrap();
        assert!(body.contains("AAA") && body.contains("BBB"), "{body}");
        assert!(!body.contains("SECRET"), "{body}");
        let other = parse_html("<form id=\"f\"></form>");
        assert!(files.for_control(&other, a).is_empty());
        doc.set_attribute(a, "type", "text");
        files.retain_live(&doc);
        assert!(files.for_control(&doc, a).is_empty());
        assert!(!files.for_control(&doc, b).is_empty());
    }

    #[test]
    fn run_submit_gate_runs_submit_before_it_returns() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
        );
        let form = form_id(&document);
        let mut log = String::new();
        let mut report = GateReport::default();
        let mut budget = SubmitBudget { steps: 8 };
        let outcome = run_submit_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut budget,
            &mut report,
            |_, name, _, phase, _, _| {
                if phase == WalkPhase::Target {
                    log.push_str(name);
                    log.push(';');
                }
                Ok(())
            },
        );
        assert_eq!(outcome, GateOutcome::Proceed);
        assert_eq!(log, "submit;");
    }

    #[test]
    fn run_submit_gate_runs_invalid_and_skips_submit() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"bad\"></form>",
        );
        let form = form_id(&document);
        let mut log = String::new();
        let mut report = GateReport::default();
        let mut budget = SubmitBudget { steps: 8 };
        let outcome = run_submit_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut budget,
            &mut report,
            |_, name, _, phase, _, _| {
                if phase == WalkPhase::Target {
                    log.push_str(name);
                    log.push(';');
                }
                Ok(())
            },
        );
        assert_eq!(outcome, GateOutcome::Invalid);
        assert_eq!(log, "invalid;");
        assert!(!report.submit_fired);
    }

    #[test]
    fn run_submit_gate_prevent_default_cancels_without_navigation() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
        );
        let form = form_id(&document);
        let mut log = String::new();
        let mut report = GateReport::default();
        let mut budget = SubmitBudget { steps: 8 };
        let outcome = run_submit_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut budget,
            &mut report,
            |_, name, _, phase, flags, _| {
                if phase == WalkPhase::Target {
                    log.push_str(name);
                    log.push(';');
                    flags.prevent_default();
                }
                Ok(())
            },
        );
        assert_eq!(outcome, GateOutcome::Cancelled);
        assert_eq!(log, "submit;");
        assert!(report.submit_cancelled);
    }

    #[test]
    fn run_submit_gate_exhaustion_does_not_replay() {
        let mut document = doc(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"bad\"></form>",
        );
        let form = form_id(&document);
        let mut calls = 0u32;
        let mut report = GateReport::default();
        let mut budget = SubmitBudget { steps: 1 };
        let outcome = run_submit_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut budget,
            &mut report,
            |_, _, _, _, _, _| {
                calls += 1;
                Ok(())
            },
        );
        assert_eq!(
            outcome,
            GateOutcome::Exhausted,
            "{calls} {:?}",
            report.events
        );
        assert_eq!(calls, 0, "a phase with no step must not run");
        assert!(!report.submit_fired);
        let events = report.events.clone();
        let mut again = GateReport::default();
        let mut budget = SubmitBudget { steps: 1 };
        let second = run_submit_gate(
            &mut document,
            form,
            None,
            SubmitDispatch::Interactive,
            &mut budget,
            &mut again,
            |_, _, _, _, _, _| Ok(()),
        );
        assert_eq!(second, GateOutcome::Exhausted);
        assert_eq!(
            again.events, events,
            "exhaustion is not replayed into a longer walk"
        );
        assert!(!again.submit_fired);
    }

    #[test]
    fn dialog_method_and_formmethod_skip_constraint_validation_in_gate() {
        let mut document = doc("<dialog id=\"d\" open>\
               <form id=\"f\" method=\"dialog\">\
                 <input type=\"email\" name=\"em\" value=\"not-an-email\" required>\
                 <button id=\"ok\" type=\"submit\" value=\"done\">OK</button>\
               </form>\
             </dialog>");
        let form = form_id(&document);
        let ok_btn = document.get_element_by_id("ok").unwrap();
        let mut report = GateReport::default();
        let res = activation_gate(
            &mut document,
            form,
            Some(ok_btn),
            SubmitDispatch::Interactive,
            &mut report,
            |_, _, _, _, _| {},
        );
        assert!(res.is_ok(), "dialog method should skip validation: {res:?}");
        assert!(report.invalid_controls.is_empty());
        assert!(report.submit_fired);
    }

    #[test]
    fn multipart_streaming_upload_reports_monotonic_chunks_and_clears_live_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (body_tx, body_rx) = mpsc::channel::<usize>();
        thread::spawn(move || {
            let raw = read_http(listener);
            body_tx.send(raw.len()).unwrap();
        });
        // Note: `read_http` closes without sending a response unless we write HTTP 200 OK;
        // let's spin up a dedicated responder socket for full round-trip verification.
        drop(body_rx);
        let _ = port;

        let responder = TcpListener::bind("127.0.0.1:0").unwrap();
        let rport = responder.local_addr().unwrap().port();
        let (rx_len_tx, rx_len_rx) = mpsc::channel::<(String, usize)>();
        let server = thread::spawn(move || {
            let (mut sock, _) = responder.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            loop {
                match sock.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..split]).to_string();
                            let cl = head
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.trim()
                                        .eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            while buf.len() < split + 4 + cl {
                                let m = sock.read(&mut tmp).unwrap_or(0);
                                if m == 0 {
                                    break;
                                }
                                buf.extend_from_slice(&tmp[..m]);
                            }
                            rx_len_tx.send((head, buf.len() - (split + 4))).unwrap();
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            );
        });

        let mut document = doc(
            "<form id=\"f\" action=\"/upload\" method=\"post\" enctype=\"multipart/form-data\">\
               <input type=\"text\" name=\"title\" value=\"Quarterly Archive\">\
               <input id=\"pick\" type=\"file\" name=\"blob\">\
             </form>",
        );
        let form = form_id(&document);
        let pick = document.get_element_by_id("pick").unwrap();
        let mut files = SelectedFiles::new();
        let payload = vec![b'X'; 70 * 1024];
        files.choose_from_picker(ChosenFormFile {
            document: document.identity(),
            control: pick,
            filename: "archive.bin".to_string(),
            content_type: "application/octet-stream".to_string(),
            bytes: payload,
        });

        let page_url = format!("http://127.0.0.1:{rport}/form");
        let csp = ContentSecurityPolicy::default();
        let effect = submission_effect(&document, form, None, &files, true).unwrap();
        let nav_req = navigation_from(&effect, &page_url, &csp).unwrap();
        assert!(nav_req.body.len() > 70 * 1024);

        let mut observed = Vec::<UploadProgress>::new();
        let mut saw_live_label = None;
        let mut jar = CookieJar::new();
        let (_final_url, resp) = exchange_form_with_progress(
            &page_url,
            &nav_req.url,
            true,
            nav_req.content_type.as_deref().unwrap(),
            &nav_req.body,
            &mut jar,
            &csp,
            |prog| {
                if prog.sent_bytes > 0
                    && prog.sent_bytes < prog.total_bytes
                    && saw_live_label.is_none()
                {
                    saw_live_label = live_upload().map(UploadProgress::status_label);
                }
                observed.push(prog);
            },
        )
        .unwrap();
        assert_eq!(resp.status, 200);
        assert!(
            observed.len() >= 6,
            "expected multi-chunk progress updates across >70 KiB body, got {observed:?}"
        );
        assert_eq!(observed.first().unwrap().sent_bytes, 0);
        assert_eq!(
            observed.last().unwrap().sent_bytes,
            nav_req.body.len() as u64
        );
        assert_eq!(observed.last().unwrap().percent(), Some(100));
        for pair in observed.windows(2) {
            assert!(pair[1].sent_bytes >= pair[0].sent_bytes);
            assert_eq!(pair[1].total_bytes, nav_req.body.len() as u64);
        }
        let label = saw_live_label.expect("mid-flight live_upload status label");
        assert!(
            label.starts_with("Uploading ") && label.contains('%'),
            "{label}"
        );
        assert!(
            live_upload().is_none(),
            "live_upload must clear automatically when upload finishes"
        );
        let (head, received_body_len) = rx_len_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(head.to_ascii_lowercase().contains("multipart/form-data"));
        assert_eq!(received_body_len, nav_req.body.len());
        server.join().unwrap();
        let _ = &mut document;
    }
}

#[cfg(test)]
#[path = "platform_loopback_independent.rs"]
mod platform_loopback_independent;
