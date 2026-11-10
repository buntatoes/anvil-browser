// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Picker registry and snapshot consume.
//!
//! Versions pinned by a pending snapshot stay until that snapshot is
//! consumed, or the page is released or navigated. The platform compares
//! `document` and `epoch` with its own trusted values. Entries are not
//! read from the DOM again.

use std::collections::HashMap;

use anvil_dom::{
    FileSelectionRef, FormMultipartEntry, NodeId, SelectionLookup, SnapshotEnctype, SnapshotEntry,
    SnapshotMethod, SubmissionSnapshot, serialize_form_multipart,
};
use anvil_net::ContentSecurityPolicy;

use crate::form_submit::{
    BodyKind, Method, NavigationRequest, SubmissionEffect, SubmitBlock, navigation_from,
};

/// Pending snapshots kept for one page. Further holds fail.
pub const MAX_PENDING_SNAPSHOTS_PER_PAGE: usize = 4;

/// Why a pin or a hold was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// This page already holds [`MAX_PENDING_SNAPSHOTS_PER_PAGE`] snapshots.
    Cap,
}

#[derive(Debug)]
struct Pin {
    selection: FileSelectionRef,
    filename: String,
    content_type: String,
    bytes: Vec<u8>,
    /// Pending snapshots that still need this exact version.
    holders: u32,
    /// This is the version `SelectionLookup::current` returns.
    current: bool,
}

#[derive(Debug)]
struct PageTrust {
    epoch: u64,
    callbacks_live: bool,
    /// Generation of a frame staged for this document. Cleared on commit.
    staged_generation: Option<u64>,
    /// A cross-document commit rejected the staged frame.
    staged_rejected: bool,
}

/// What the browser did with a navigation.
///
/// Only [`NavigationCommit::CrossDocument`] retires the current page.
/// Intent, a fragment, a policy block, and a failure before commit do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationCommit {
    /// Same document. The hash may change.
    Fragment,
    /// CSP or another policy refused the load.
    Blocked,
    /// Cancelled, or the load failed before a document was installed.
    Abandoned,
    /// A different document is now the current page.
    CrossDocument,
}

/// The snapshot action names the pending navigation's origin, not the
/// trusted document. A same-origin action, and an action on any other
/// origin, is left as the runtime resolved it.
pub fn action_follows_pending_origin(
    action: &str,
    trusted_url: &str,
    pending_origin: Option<&str>,
) -> bool {
    let Some(pending) = pending_origin.map(str::trim).filter(|url| !url.is_empty()) else {
        return false;
    };
    if anvil_net::is_same_origin(trusted_url, action) {
        return false;
    }
    !anvil_net::is_same_origin(trusted_url, pending) && anvil_net::is_same_origin(pending, action)
}

/// Epoch, pins, and staged frames move only when a document is committed.
pub fn commit_retires_page(commit: NavigationCommit) -> bool {
    matches!(commit, NavigationCommit::CrossDocument)
}

/// ASCII tab, line feed, and carriage return. The URL parser removes these
/// from the whole input before it reads a scheme.
fn strip_url_tab_and_newline(url: &str) -> String {
    url.chars()
        .filter(|ch| !matches!(ch, '\t' | '\n' | '\r'))
        .collect()
}

/// Leading and trailing C0 controls, space, and DEL. An embedded C0 is left
/// in place so it cannot disappear into a normal scheme token.
fn trim_c0_space_and_del(url: &str) -> &str {
    url.trim_matches(|ch: char| ch == ' ' || ch == '\u{007F}' || ch <= '\u{001F}')
}

fn has_c0_or_del(url: &str) -> bool {
    url.chars().any(|ch| ch == '\u{007F}' || ch <= '\u{001F}')
}

fn scheme_token(url: &str) -> Option<String> {
    let (raw, _) = url.split_once(':')?;
    let scheme = raw.trim();
    if scheme.is_empty() || !scheme.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return None;
    }
    if !scheme
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Scheme of `url` after the URL parser's tab/newline removal and a trim of
/// leading C0 controls and spaces.
///
/// ` JavaScript:x` and `\tjavascript:x` are `javascript`. A scheme token that
/// still contains U+0001, NUL, or another C0 has no scheme.
pub fn navigation_scheme(url: &str) -> Option<String> {
    let stripped = strip_url_tab_and_newline(url);
    let trimmed = trim_c0_space_and_del(&stripped);
    if has_c0_or_del(trimmed) {
        return None;
    }
    scheme_token(trimmed)
}

/// `javascript:` and `data:` navigations are unsupported. An unknown scheme
/// is not fetched. `http`, `https`, `file`, `anvil`, `about`, and `view-source`
/// stay ordinary.
///
/// A C0 control or DEL in the input is not ordinary. Tab and newline are
/// removed first, the same way the URL parser removes them. If that leaves
/// `javascript:` or `data:`, the navigation is unsupported. Any other C0,
/// including an embedded U+0001 or NUL in the scheme, is refused and is not
/// fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationScheme {
    Ordinary,
    /// Explicitly unsupported. The browser does nothing.
    Unsupported,
    /// Not a scheme this browser loads. The browser does nothing.
    Refused,
}

pub fn classify_navigation_scheme(url: &str) -> NavigationScheme {
    if has_c0_or_del(url) {
        let stripped = strip_url_tab_and_newline(url);
        let trimmed = trim_c0_space_and_del(&stripped);
        if !has_c0_or_del(trimmed)
            && matches!(
                scheme_token(trimmed).as_deref(),
                Some("javascript" | "data")
            )
        {
            return NavigationScheme::Unsupported;
        }
        return NavigationScheme::Refused;
    }
    match navigation_scheme(url).as_deref() {
        None => NavigationScheme::Ordinary,
        Some("http" | "https" | "file" | "anvil" | "about" | "view-source") => {
            NavigationScheme::Ordinary
        }
        Some("javascript" | "data") => NavigationScheme::Unsupported,
        Some(_) => NavigationScheme::Refused,
    }
}

/// Trusted picker store. Implements [`SelectionLookup`].
#[derive(Debug)]
pub struct SelectionRegistry {
    pins: Vec<Pin>,
    next_version: HashMap<(u64, NodeId), u64>,
    pending: Vec<SubmissionSnapshot>,
    pages: HashMap<u64, PageTrust>,
}

impl Default for SelectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            pins: Vec::new(),
            next_version: HashMap::new(),
            pending: Vec::new(),
            pages: HashMap::new(),
        }
    }

    /// Epoch and liveness the platform currently trusts for `document`.
    pub fn trusted(&mut self, document: u64) -> (u64, bool) {
        let page = self.pages.entry(document).or_insert(PageTrust {
            epoch: 1,
            callbacks_live: true,
            staged_generation: None,
            staged_rejected: false,
        });
        (page.epoch, page.callbacks_live)
    }

    /// Record a new pick. The previous current version is dropped unless a
    /// pending snapshot still holds it.
    pub fn pick(
        &mut self,
        document: u64,
        control: NodeId,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> FileSelectionRef {
        let key = (document, control);
        let version = self.next_version.entry(key).or_insert(1);
        let assigned = *version;
        *version = version.saturating_add(1);
        for pin in &mut self.pins {
            if pin.selection.document == document && pin.selection.control == control && pin.current
            {
                pin.current = false;
                break;
            }
        }
        self.pins.retain(|pin| {
            !(pin.selection.document == document
                && pin.selection.control == control
                && !pin.current
                && pin.holders == 0)
        });
        let selection = FileSelectionRef {
            document,
            control,
            version: assigned,
        };
        self.pins.push(Pin {
            selection,
            filename: filename.into(),
            content_type: content_type.into(),
            bytes,
            holders: 0,
            current: true,
        });
        let _ = self.trusted(document);
        selection
    }

    /// Hold `snapshot` until consume, release, or navigation.
    pub fn hold(&mut self, snapshot: SubmissionSnapshot) -> Result<(), RegistryError> {
        let document = snapshot.document;
        let pending = self
            .pending
            .iter()
            .filter(|held| held.document == document)
            .count();
        if pending >= MAX_PENDING_SNAPSHOTS_PER_PAGE {
            return Err(RegistryError::Cap);
        }
        for entry in &snapshot.entries {
            if let SnapshotEntry::File {
                selection: Some(selection),
                ..
            } = entry
                && let Some(pin) = self.pins.iter_mut().find(|pin| pin.selection == *selection)
            {
                pin.holders = pin.holders.saturating_add(1);
            }
        }
        self.pending.push(snapshot);
        Ok(())
    }

    /// Bytes still stored for `document`.
    pub fn pinned_bytes(&self, document: u64) -> usize {
        self.pins
            .iter()
            .filter(|pin| pin.selection.document == document)
            .map(|pin| pin.bytes.len())
            .sum()
    }

    /// Drop every pin and pending snapshot for `document`, and bump the epoch.
    pub fn release(&mut self, document: u64) {
        self.pins.retain(|pin| pin.selection.document != document);
        self.pending.retain(|snap| snap.document != document);
        let page = self.pages.entry(document).or_insert(PageTrust {
            epoch: 1,
            callbacks_live: true,
            staged_generation: None,
            staged_rejected: false,
        });
        page.epoch = page.epoch.saturating_add(1);
        page.callbacks_live = false;
        page.staged_generation = None;
        page.staged_rejected = true;
    }

    /// Remember a frame painted for `document` at `generation`.
    pub fn stage_frame(&mut self, document: u64, generation: u64) {
        let page = self.pages.entry(document).or_insert(PageTrust {
            epoch: 1,
            callbacks_live: true,
            staged_generation: None,
            staged_rejected: false,
        });
        if page.callbacks_live {
            page.staged_generation = Some(generation);
            page.staged_rejected = false;
        }
    }

    /// The staged frame is current when this document is still trusted and
    /// the generation matches. A commit clears it.
    pub fn staged_frame_current(&self, document: u64, generation: u64) -> bool {
        self.pages.get(&document).is_some_and(|page| {
            page.callbacks_live
                && !page.staged_rejected
                && page.staged_generation == Some(generation)
        })
    }

    /// Navigation uses the same epoch bump and pin release as [`Self::release`].
    pub fn navigate(&mut self, document: u64) {
        self.release(document);
    }

    /// Build the request from `snapshot` alone.
    ///
    /// Identity, epoch, and `callbacks_live` are this registry's current
    /// values. A release or navigation after the snapshot was taken makes
    /// those values disagree, so consume rejects it. The runtime is not asked.
    /// A missing pinned version also rejects the snapshot.
    ///
    /// `page_url` is the browser's trusted document URL, not
    /// `ScriptRuntime::page_origin`. `pending_origin` is only a witness for a
    /// not-yet-committed intent. When the snapshot action was resolved against
    /// that intent instead of `page_url`, consume rejects it and releases the
    /// pin. The action is not resolved again.
    pub fn consume(
        &mut self,
        snapshot: &SubmissionSnapshot,
        page_url: &str,
        csp: &ContentSecurityPolicy,
    ) -> Result<NavigationRequest, SubmitBlock> {
        self.consume_with_intent(snapshot, page_url, None, csp)
    }

    /// [`Self::consume`] with the runtime's current `page_origin`, if any.
    pub fn consume_with_intent(
        &mut self,
        snapshot: &SubmissionSnapshot,
        page_url: &str,
        pending_origin: Option<&str>,
        csp: &ContentSecurityPolicy,
    ) -> Result<NavigationRequest, SubmitBlock> {
        let Some(page) = self.pages.get(&snapshot.document) else {
            return Err(SubmitBlock::Stale);
        };
        if snapshot.epoch != page.epoch || !page.callbacks_live {
            return Err(SubmitBlock::Stale);
        }
        if action_follows_pending_origin(&snapshot.action, page_url, pending_origin) {
            self.release_hold(snapshot);
            return Err(SubmitBlock::Stale);
        }
        let request = request_from_snapshot(snapshot, self, page_url, csp)?;
        self.release_hold(snapshot);
        Ok(request)
    }

    fn release_hold(&mut self, snapshot: &SubmissionSnapshot) {
        if let Some(index) = self.pending.iter().position(|held| held == snapshot) {
            self.pending.remove(index);
        }
        for entry in &snapshot.entries {
            if let SnapshotEntry::File {
                selection: Some(selection),
                ..
            } = entry
                && let Some(pin) = self.pins.iter_mut().find(|pin| pin.selection == *selection)
            {
                pin.holders = pin.holders.saturating_sub(1);
            }
        }
        self.pins.retain(|pin| pin.current || pin.holders > 0);
    }

    fn resolve(&self, selection: &FileSelectionRef) -> Option<&Pin> {
        self.pins.iter().find(|pin| pin.selection == *selection)
    }
}

/// Shared picker registry. The runtime holds this as `Arc<dyn SelectionLookup>`.
#[derive(Debug)]
pub struct SharedSelectionRegistry(pub std::sync::Mutex<SelectionRegistry>);

impl PartialEq for SharedSelectionRegistry {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl SelectionLookup for SharedSelectionRegistry {
    fn current(&self, document: u64, control: NodeId) -> Option<FileSelectionRef> {
        self.0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .current(document, control)
    }
}

impl SelectionLookup for SelectionRegistry {
    fn current(&self, document: u64, control: NodeId) -> Option<FileSelectionRef> {
        self.pins.iter().find_map(|pin| {
            (pin.current && pin.selection.document == document && pin.selection.control == control)
                .then_some(pin.selection)
        })
    }
}

/// Replace the planned snapshot. The previous one is dropped.
pub fn plan_submit(planned: &mut Option<SubmissionSnapshot>, next: SubmissionSnapshot) {
    *planned = Some(next);
}

/// Record a snapshot only after interactive validation succeeded and `submit`
/// was not cancelled. Otherwise the planned navigation is left unchanged.
pub fn record_interactive(
    planned: &mut Option<SubmissionSnapshot>,
    validation_succeeded: bool,
    submit_cancelled: bool,
    snapshot: SubmissionSnapshot,
) -> bool {
    if !validation_succeeded || submit_cancelled {
        return false;
    }
    *planned = Some(snapshot);
    true
}

fn request_from_snapshot(
    snapshot: &SubmissionSnapshot,
    registry: &SelectionRegistry,
    page_url: &str,
    csp: &ContentSecurityPolicy,
) -> Result<NavigationRequest, SubmitBlock> {
    let mut parts = Vec::new();
    for entry in &snapshot.entries {
        match entry {
            SnapshotEntry::Text { name, value } => parts.push(FormMultipartEntry {
                name: name.clone(),
                value: value.clone(),
                filename: None,
                content_type: None,
            }),
            SnapshotEntry::File { name, selection } => {
                let (filename, content_type, value) = if let Some(selection) = selection {
                    if selection.document != snapshot.document {
                        return Err(SubmitBlock::Stale);
                    }
                    let pin = registry.resolve(selection).ok_or(SubmitBlock::Stale)?;
                    if pin.selection.document != snapshot.document
                        || pin.selection.control != selection.control
                        || pin.selection.version != selection.version
                    {
                        return Err(SubmitBlock::Stale);
                    }
                    (
                        pin.filename.clone(),
                        pin.content_type.clone(),
                        String::from_utf8_lossy(&pin.bytes).into_owned(),
                    )
                } else {
                    (
                        String::new(),
                        "application/octet-stream".to_string(),
                        String::new(),
                    )
                };
                parts.push(FormMultipartEntry {
                    name: name.clone(),
                    value,
                    filename: Some(filename),
                    content_type: Some(content_type),
                });
            }
        }
    }
    let method = match snapshot.method {
        SnapshotMethod::Get => Method::Get,
        SnapshotMethod::Post => Method::Post,
    };
    let (kind, body) = match snapshot.enctype {
        SnapshotEnctype::Multipart => {
            let (body, _) = serialize_form_multipart(&parts, anvil_dom::DEFAULT_MULTIPART_BOUNDARY);
            (
                BodyKind::Multipart {
                    boundary: anvil_dom::DEFAULT_MULTIPART_BOUNDARY.to_string(),
                },
                body.into_bytes(),
            )
        }
        SnapshotEnctype::TextPlain => {
            let mut body = String::new();
            for part in parts.iter().filter(|part| part.filename.is_none()) {
                body.push_str(&part.name);
                body.push('=');
                body.push_str(&part.value);
                body.push_str("\r\n");
            }
            (BodyKind::TextPlain, body.into_bytes())
        }
        SnapshotEnctype::UrlEncoded => {
            let body = parts
                .iter()
                .filter(|part| part.filename.is_none())
                .map(|part| {
                    format!(
                        "{}={}",
                        anvil_dom::urlencode_component(&part.name),
                        anvil_dom::urlencode_component(&part.value)
                    )
                })
                .collect::<Vec<_>>()
                .join("&");
            (BodyKind::UrlEncoded, body.into_bytes())
        }
    };
    let effect = SubmissionEffect {
        action: snapshot.action.clone(),
        method,
        kind,
        body,
        submitter: snapshot.submitter,
        generation: snapshot.epoch,
    };
    navigation_from(&effect, page_url, csp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_dom::{SubmissionUrls, parse_html};

    fn urls<'a>() -> SubmissionUrls<'a> {
        SubmissionUrls {
            document_url: "https://page.example/app/",
            base_url: "https://page.example/app/",
        }
    }

    fn form_doc() -> (anvil_dom::Document, anvil_dom::NodeId, anvil_dom::NodeId) {
        let doc = parse_html(
            "<form id=\"f\" action=\"/save\" method=\"POST\" enctype=\"multipart/form-data\"><input id=\"file\" type=\"file\" name=\"up\"><input name=\"q\" value=\"a\"></form>",
        );
        let form = doc.get_element_by_id("f").unwrap();
        let file = doc.get_element_by_id("file").unwrap();
        (doc, form, file)
    }

    #[test]
    fn pick_a_then_b_consumes_a() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let _a = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        let _b = reg.pick(
            doc.identity(),
            file,
            "b.txt",
            "text/plain",
            b"BBBB".to_vec(),
        );
        let request = reg
            .consume(&snap, "https://page.example/app/", &Default::default())
            .unwrap();
        let body = String::from_utf8(request.body).unwrap();
        assert!(body.contains("AAAA"), "{body}");
        assert!(body.contains("filename=\"a.txt\""), "{body}");
        assert!(!body.contains("BBBB"), "{body}");
        assert!(!body.contains("b.txt"), "{body}");
    }

    #[test]
    fn release_before_consume_rejects_and_frees_the_pin() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let _a = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        assert!(reg.pinned_bytes(doc.identity()) >= 4);
        reg.release(doc.identity());
        assert_eq!(reg.pinned_bytes(doc.identity()), 0);
        let (_now, _live) = reg.trusted(doc.identity());
        let err = reg.consume(&snap, "https://page.example/app/", &Default::default());
        assert_eq!(err.err(), Some(SubmitBlock::Stale));
    }

    #[test]
    fn the_pending_snapshot_cap_is_enforced() {
        let (doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        for _ in 0..MAX_PENDING_SNAPSHOTS_PER_PAGE {
            let snap = doc
                .snapshot_form_submission(form, None, epoch, urls(), &reg)
                .unwrap();
            reg.hold(snap).unwrap();
        }
        let extra = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        assert_eq!(reg.hold(extra).err(), Some(RegistryError::Cap));
    }

    #[test]
    fn a_later_dom_edit_does_not_change_the_consumed_body() {
        let (mut doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        let field = doc.get_element_by_id("f").unwrap();
        // The text control is inside the form. Change its value after capture.
        let input = doc
            .children(form)
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().attr("name") == Some("q"))
            .unwrap();
        doc.set_input_value(input, "changed-later");
        let _ = field;
        let request = reg
            .consume(&snap, "https://page.example/app/", &Default::default())
            .unwrap();
        let body = String::from_utf8(request.body).unwrap();
        assert!(body.contains("name=\"q\""), "{body}");
        assert!(
            body.contains("\r\na\r\n") || body.contains("a\r\n"),
            "{body}"
        );
        assert!(!body.contains("changed-later"), "{body}");
    }

    #[test]
    fn navigation_between_hold_and_consume_rejects() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let _a = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        reg.navigate(doc.identity());
        let (now, live) = reg.trusted(doc.identity());
        assert_ne!(now, snap.epoch);
        assert!(!live);
        assert_eq!(reg.pinned_bytes(doc.identity()), 0);
        assert_eq!(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .err(),
            Some(SubmitBlock::Stale)
        );
    }

    #[test]
    fn a_second_submit_replaces_the_planned_snapshot() {
        let (doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let first = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        let mut planned = None;
        plan_submit(&mut planned, first);
        let mut doc = doc;
        let input = doc
            .children(form)
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().attr("name") == Some("q"))
            .unwrap();
        doc.set_input_value(input, "second");
        let second = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        plan_submit(&mut planned, second);
        let body = format!("{:?}", planned.unwrap().entries);
        assert!(body.contains("second"), "{body}");
        assert!(!body.contains("\"a\""), "{body}");
    }

    #[test]
    fn removing_the_form_after_planning_does_not_cancel_consume() {
        let (mut doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        doc.remove_node(form);
        assert!(doc.get(form).parent().is_none());
        let request = reg
            .consume(&snap, "https://page.example/app/", &Default::default())
            .unwrap();
        assert!(
            request.url.contains("/save"),
            "detached form still navigates: {}",
            request.url
        );
        assert_eq!(request.method, Method::Post);
        let body = String::from_utf8(request.body).unwrap();
        assert!(body.contains("name=\"q\""), "{body}");
        assert!(!body.contains("changed"), "{body}");
    }

    #[test]
    fn a_missing_selection_is_an_empty_file_part() {
        let doc = parse_html(
            "<form id=\"f\" action=\"/up\" method=\"POST\" enctype=\"multipart/form-data\"><input id=\"file\" type=\"file\" name=\"up\" value=\"secret-name.txt\" data-file-content=\"SECRETBYTES\" filename=\"note.txt\"></form>",
        );
        let form = doc.get_element_by_id("f").unwrap();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        let request = reg
            .consume(&snap, "https://page.example/app/", &Default::default())
            .unwrap();
        let body = String::from_utf8(request.body).unwrap();
        assert!(body.contains("filename=\"\""), "{body}");
        assert!(!body.contains("SECRETBYTES"), "{body}");
        assert!(!body.contains("secret-name"), "{body}");
        assert!(!body.contains("note.txt"), "{body}");
    }

    #[test]
    fn a_synchronous_edit_before_capture_is_in_the_body() {
        let (mut doc, form, _) = form_doc();
        let input = doc
            .children(form)
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().attr("name") == Some("q"))
            .unwrap();
        doc.set_input_value(input, "from-submit");
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        let body = String::from_utf8(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .unwrap()
                .body,
        )
        .unwrap();
        assert!(body.contains("from-submit"), "{body}");
    }

    #[test]
    fn a_microtask_edit_after_capture_is_not_in_the_body() {
        let (mut doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        let input = doc
            .children(form)
            .iter()
            .copied()
            .find(|id| doc.get(*id).data().attr("name") == Some("q"))
            .unwrap();
        doc.set_input_value(input, "from-micro");
        let body = String::from_utf8(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .unwrap()
                .body,
        )
        .unwrap();
        assert!(body.contains("name=\"q\""), "{body}");
        assert!(!body.contains("from-micro"), "{body}");
    }

    #[test]
    fn an_invalid_or_cancelled_submit_records_no_snapshot() {
        let (doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        let mut planned = None;
        assert!(!record_interactive(
            &mut planned,
            false,
            false,
            snap.clone()
        ));
        assert!(planned.is_none());
        assert!(!record_interactive(&mut planned, true, true, snap));
        assert!(planned.is_none());
    }

    fn file_entry_mut(snap: &mut SubmissionSnapshot) -> &mut SnapshotEntry {
        snap.entries
            .iter_mut()
            .find(|entry| matches!(entry, SnapshotEntry::File { .. }))
            .unwrap()
    }

    #[test]
    fn a_mismatched_document_ref_is_rejected() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let pinned = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let mut snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        let SnapshotEntry::File { selection, .. } = file_entry_mut(&mut snap) else {
            panic!("file");
        };
        *selection = Some(FileSelectionRef {
            document: doc.identity().wrapping_add(100),
            control: pinned.control,
            version: pinned.version,
        });
        reg.hold(snap.clone()).unwrap();
        assert_eq!(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .err(),
            Some(SubmitBlock::Stale)
        );
    }

    #[test]
    fn a_mismatched_control_ref_is_rejected() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let pinned = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let mut snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        let SnapshotEntry::File { selection, .. } = file_entry_mut(&mut snap) else {
            panic!("file");
        };
        *selection = Some(FileSelectionRef {
            document: pinned.document,
            control: NodeId::from_raw(pinned.control.index() as u32 + 9),
            version: pinned.version,
        });
        reg.hold(snap.clone()).unwrap();
        assert_eq!(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .err(),
            Some(SubmitBlock::Stale)
        );
    }

    #[test]
    fn a_stale_version_is_rejected() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let pinned = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let mut snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        let SnapshotEntry::File { selection, .. } = file_entry_mut(&mut snap) else {
            panic!("file");
        };
        *selection = Some(FileSelectionRef {
            document: pinned.document,
            control: pinned.control,
            version: pinned.version.wrapping_add(50),
        });
        reg.hold(snap.clone()).unwrap();
        assert_eq!(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .err(),
            Some(SubmitBlock::Stale)
        );
    }

    #[test]
    fn a_legacy_record_still_uses_the_generation_guard() {
        let mut doc = parse_html(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" value=\"a\"></form>",
        );
        let form = doc.get_element_by_id("f").unwrap();
        let host = anvil_dom::HostSubmission {
            form,
            submitter: None,
            mode: anvil_dom::SubmitDispatch::Interactive,
            generation: doc.mutation_generation(),
            document: doc.identity(),
        };
        assert!(host.stale_against(&doc).is_none());
        doc.set_input_value(doc.children(form).iter().copied().next().unwrap(), "later");
        assert_eq!(host.stale_against(&doc), Some("form submission is stale"));
    }

    fn held_text_snapshot() -> (SelectionRegistry, SubmissionSnapshot) {
        let (doc, form, _) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        reg.hold(snap.clone()).unwrap();
        reg.stage_frame(doc.identity(), 7);
        (reg, snap)
    }

    #[test]
    fn fragment_block_and_abandon_keep_the_snapshot_and_staged_frame() {
        for commit in [
            NavigationCommit::Fragment,
            NavigationCommit::Blocked,
            NavigationCommit::Abandoned,
        ] {
            let (mut reg, snap) = held_text_snapshot();
            assert!(!commit_retires_page(commit));
            let (epoch, live) = reg.trusted(snap.document);
            assert_eq!(epoch, snap.epoch);
            assert!(live);
            assert!(reg.staged_frame_current(snap.document, 7));
            assert!(
                reg.consume(&snap, "https://page.example/app/", &Default::default())
                    .is_ok()
            );
        }
    }

    #[test]
    fn a_cross_document_commit_makes_the_snapshot_stale_and_drops_the_frame() {
        let (mut reg, snap) = held_text_snapshot();
        assert!(commit_retires_page(NavigationCommit::CrossDocument));
        reg.navigate(snap.document);
        assert!(!reg.staged_frame_current(snap.document, 7));
        assert_eq!(
            reg.consume(&snap, "https://page.example/app/", &Default::default())
                .err(),
            Some(SubmitBlock::Stale)
        );
    }

    #[test]
    fn javascript_and_data_are_unsupported_and_unknown_schemes_are_refused() {
        assert_eq!(
            classify_navigation_scheme("javascript:alert(1)"),
            NavigationScheme::Unsupported
        );
        assert_eq!(
            classify_navigation_scheme(" JavaScript:x"),
            NavigationScheme::Unsupported
        );
        assert_eq!(
            classify_navigation_scheme("data:text/html,hi"),
            NavigationScheme::Unsupported
        );
        assert_eq!(
            classify_navigation_scheme("foo:bar"),
            NavigationScheme::Refused
        );
        assert_eq!(
            classify_navigation_scheme("https://127.0.0.1/next"),
            NavigationScheme::Ordinary
        );
        assert_eq!(
            classify_navigation_scheme("#section"),
            NavigationScheme::Ordinary
        );
        assert_eq!(
            classify_navigation_scheme("/save"),
            NavigationScheme::Ordinary
        );
    }

    #[test]
    fn a_control_character_in_the_scheme_is_not_ordinary() {
        let cases = [
            ("java\u{0001}script:alert(1)", NavigationScheme::Refused),
            ("java\u{0000}script:alert(1)", NavigationScheme::Refused),
            ("\u{0001}javascript:alert(1)", NavigationScheme::Unsupported),
            ("\tjavascript:alert(1)", NavigationScheme::Unsupported),
            ("java\tscript:alert(1)", NavigationScheme::Unsupported),
            ("java\nscript:alert(1)", NavigationScheme::Unsupported),
            ("java\rscript:alert(1)", NavigationScheme::Unsupported),
            ("\ndata:text/html,hi", NavigationScheme::Unsupported),
            ("da\u{0001}ta:text/html,hi", NavigationScheme::Refused),
            ("da\u{0007}ta:text/html,hi", NavigationScheme::Refused),
            ("http://127.0.0.1/a\u{0001}", NavigationScheme::Refused),
        ];
        for (url, expected) in cases {
            assert_eq!(classify_navigation_scheme(url), expected, "{url:?}");
        }
    }

    #[test]
    fn a_relative_action_resolved_against_a_pending_origin_is_not_sent() {
        let (doc, form, file) = form_doc();
        let mut reg = SelectionRegistry::new();
        let (epoch, _) = reg.trusted(doc.identity());
        let _first = reg.pick(
            doc.identity(),
            file,
            "a.txt",
            "text/plain",
            b"AAAA".to_vec(),
        );
        let mut snap = doc
            .snapshot_form_submission(form, None, epoch, urls(), &reg)
            .unwrap();
        snap.action = "https://other.example/send".to_string();
        reg.hold(snap.clone()).unwrap();
        let _second = reg.pick(
            doc.identity(),
            file,
            "b.txt",
            "text/plain",
            b"BBBB".to_vec(),
        );
        assert!(reg.pinned_bytes(doc.identity()) >= 8);
        let err = reg.consume_with_intent(
            &snap,
            "https://app.example/app/index.html",
            Some("https://other.example/away"),
            &Default::default(),
        );
        assert_eq!(err.err(), Some(SubmitBlock::Stale));
        assert_eq!(reg.pinned_bytes(doc.identity()), 4);
        assert!(!action_follows_pending_origin(
            "https://app.example/app/send",
            "https://app.example/app/index.html",
            Some("https://other.example/away"),
        ));
    }
}
