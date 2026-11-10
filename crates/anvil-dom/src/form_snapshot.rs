// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! A form captured at submit time.
//!
//! [`Document::snapshot_form_submission`] does not validate controls and does
//! not care whether the form is connected. The caller supplies [`SubmissionSnapshot::epoch`].
//! This module does not invent an epoch and does not store file bytes.

use crate::{Document, NodeId};

/// The two URLs a form action needs.
///
/// An empty action is the document URL. Any other action is parsed against
/// `base_url`, which is the document URL or the `<base href>` the caller
/// resolved. One string cannot carry both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmissionUrls<'a> {
    /// `Document` URL. Used when the chosen action is empty.
    pub document_url: &'a str,
    /// Effective base URL. Used when the chosen action is not empty.
    pub base_url: &'a str,
}

/// Opaque reference to one trusted picker selection.
///
/// Script, element values, data attributes, and synthetic bytes cannot mint
/// this. Only a picker-backed registry can. `version` is the pick that was
/// current when the snapshot was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileSelectionRef {
    /// [`Document::identity`] of the page that owns the control.
    pub document: u64,
    /// The file control.
    pub control: NodeId,
    /// Registry version current at submit time.
    pub version: u64,
}

/// Looks up the selection that is current for one control.
///
/// Object-safe. The runtime holds `Arc<dyn SelectionLookup + Send + Sync>`
/// from `set_selection_lookup` and never mints refs.
pub trait SelectionLookup: Send + Sync {
    /// The current ref, if the registry has one for this document and control.
    fn current(&self, document: u64, control: NodeId) -> Option<FileSelectionRef>;
}

/// One successful control in a snapshot. File bytes are not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotEntry {
    /// A named text control.
    Text {
        /// Control name.
        name: String,
        /// Control value at capture.
        value: String,
    },
    /// A file control. `selection` is only what [`SelectionLookup`] returned.
    ///
    /// There are no bytes in this variant. `value`, `filename`, and `data-*`
    /// attributes are not read.
    File {
        /// Control name.
        name: String,
        /// Picker ref current at capture. `None` when the lookup has none.
        selection: Option<FileSelectionRef>,
    },
}

/// `GET` or `POST`. Dialog and unknown methods are `Get`, matching the
/// existing serializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotMethod {
    /// `method` is not `POST`.
    Get,
    /// `method` is `POST`.
    Post,
}

/// The three form encodings. `text/plain` stays `text/plain` here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotEnctype {
    /// `application/x-www-form-urlencoded`.
    UrlEncoded,
    /// `multipart/form-data`.
    Multipart,
    /// `text/plain`.
    TextPlain,
}

/// Immutable capture of one submit.
///
/// `epoch` is whatever the caller passed in. A later DOM edit does not
/// rewrite `entries` or `action`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionSnapshot {
    /// [`Document::identity`].
    pub document: u64,
    /// Caller-supplied epoch. Not read from the document.
    pub epoch: u64,
    /// The `form` element.
    pub form: NodeId,
    /// Submitter control, when the caller named one.
    pub submitter: Option<NodeId>,
    /// Resolved method.
    pub method: SnapshotMethod,
    /// Absolute action. Empty input becomes [`SubmissionUrls::document_url`].
    pub action: String,
    /// Resolved encoding.
    pub enctype: SnapshotEnctype,
    /// `formtarget`, else `target`, else empty.
    pub target: String,
    /// Successful controls, in tree order. The submitter's name is last.
    pub entries: Vec<SnapshotEntry>,
}

/// Why a snapshot could not be built. Validity and connection are not errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// `form` is not a slot in this document.
    MissingForm,
    /// `form` exists and is not a `form` element.
    NotAForm,
}

impl Document {
    /// Capture `form` once.
    ///
    /// `epoch` is stored as given. `urls.document_url` is the action when the
    /// chosen action string is empty. Otherwise the action is resolved against
    /// `urls.base_url`. The submitter's `formaction` is chosen before the
    /// form's `action`, including when `formaction` is present and empty.
    ///
    /// This does not check constraint validity and does not check that the
    /// form is connected. File bytes are not copied. `lookup` supplies the
    /// pinned [`FileSelectionRef`] for each file control.
    pub fn snapshot_form_submission(
        &self,
        form: NodeId,
        submitter: Option<NodeId>,
        epoch: u64,
        urls: SubmissionUrls<'_>,
        lookup: &dyn SelectionLookup,
    ) -> Result<SubmissionSnapshot, SnapshotError> {
        if form.index() >= self.node_count() {
            return Err(SnapshotError::MissingForm);
        }
        if self.get(form).data().tag() != Some("form") {
            return Err(SnapshotError::NotAForm);
        }
        let action_attr = chosen_action(self, form, submitter);
        let action = resolve_submission_action(&action_attr, urls);
        let method = chosen_method(self, form, submitter);
        let enctype = chosen_enctype(self, form, submitter);
        let target = chosen_target(self, form, submitter);
        let mut entries = Vec::new();
        if self
            .get(form)
            .data()
            .attr("id")
            .is_some_and(|id| !id.trim().is_empty())
        {
            collect_owned(self, self.root(), form, lookup, &mut entries);
        } else {
            collect_subtree(self, form, lookup, &mut entries);
        }
        push_submitter_name(self, submitter, &mut entries);
        Ok(SubmissionSnapshot {
            document: self.identity(),
            epoch,
            form,
            submitter,
            method,
            action,
            enctype,
            target,
            entries,
        })
    }
}

fn chosen_action(doc: &Document, form: NodeId, submitter: Option<NodeId>) -> String {
    if let Some(raw) = submitter_attr(doc, submitter, "formaction") {
        return raw;
    }
    doc.get(form)
        .data()
        .attr("action")
        .unwrap_or("")
        .trim()
        .to_string()
}

fn chosen_method(doc: &Document, form: NodeId, submitter: Option<NodeId>) -> SnapshotMethod {
    let raw = submitter_attr(doc, submitter, "formmethod")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            doc.get(form)
                .data()
                .attr("method")
                .unwrap_or("GET")
                .trim()
                .to_string()
        });
    if raw.eq_ignore_ascii_case("POST") {
        SnapshotMethod::Post
    } else {
        SnapshotMethod::Get
    }
}

fn chosen_enctype(doc: &Document, form: NodeId, submitter: Option<NodeId>) -> SnapshotEnctype {
    let raw = submitter_attr(doc, submitter, "formenctype")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            doc.get(form)
                .data()
                .attr("enctype")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    match raw.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("multipart/form-data") => SnapshotEnctype::Multipart,
        Some("text/plain") => SnapshotEnctype::TextPlain,
        _ => SnapshotEnctype::UrlEncoded,
    }
}

fn chosen_target(doc: &Document, form: NodeId, submitter: Option<NodeId>) -> String {
    if let Some(raw) = submitter_attr(doc, submitter, "formtarget") {
        return raw;
    }
    doc.get(form)
        .data()
        .attr("target")
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Attribute present on the submitter, including an empty value.
/// `None` when the submitter or the attribute is absent.
fn submitter_attr(doc: &Document, submitter: Option<NodeId>, name: &str) -> Option<String> {
    let id = submitter?;
    if id.index() >= doc.node_count() {
        return None;
    }
    doc.get(id)
        .data()
        .attr(name)
        .map(|value| value.trim().to_string())
}

fn collect_owned(
    doc: &Document,
    id: NodeId,
    form: NodeId,
    lookup: &dyn SelectionLookup,
    out: &mut Vec<SnapshotEntry>,
) {
    if id != form && doc.enclosing_form(id) == Some(form) {
        push_control(doc, id, lookup, out);
    }
    for child in doc.children(id) {
        collect_owned(doc, *child, form, lookup, out);
    }
}

fn collect_subtree(
    doc: &Document,
    id: NodeId,
    lookup: &dyn SelectionLookup,
    out: &mut Vec<SnapshotEntry>,
) {
    push_control(doc, id, lookup, out);
    for child in doc.children(id) {
        collect_subtree(doc, *child, lookup, out);
    }
}

fn push_control(
    doc: &Document,
    id: NodeId,
    lookup: &dyn SelectionLookup,
    out: &mut Vec<SnapshotEntry>,
) {
    let data = doc.get(id).data();
    if data.attr("disabled").is_some() {
        return;
    }
    let Some(raw_name) = data.attr("name") else {
        return;
    };
    let name = raw_name.trim();
    if name.is_empty() {
        return;
    }
    match data.tag() {
        Some("input") => {
            let kind = data
                .attr("type")
                .unwrap_or("text")
                .trim()
                .to_ascii_lowercase();
            match kind.as_str() {
                "submit" | "button" | "reset" | "image" => {}
                "checkbox" | "radio" => {
                    if doc.is_checked(id) {
                        out.push(SnapshotEntry::Text {
                            name: name.to_string(),
                            value: data.attr("value").unwrap_or("on").to_string(),
                        });
                    }
                }
                "file" => {
                    let selection = lookup
                        .current(doc.identity(), id)
                        .filter(|picked| picked.document == doc.identity() && picked.control == id);
                    out.push(SnapshotEntry::File {
                        name: name.to_string(),
                        selection,
                    });
                }
                _ => out.push(SnapshotEntry::Text {
                    name: name.to_string(),
                    value: data.attr("value").unwrap_or("").to_string(),
                }),
            }
        }
        Some("textarea") => out.push(SnapshotEntry::Text {
            name: name.to_string(),
            value: doc.input_value(id),
        }),
        Some("select") => out.push(SnapshotEntry::Text {
            name: name.to_string(),
            value: doc.select_value(id),
        }),
        _ => {}
    }
}

fn push_submitter_name(doc: &Document, submitter: Option<NodeId>, out: &mut Vec<SnapshotEntry>) {
    let Some(id) = submitter else {
        return;
    };
    if id.index() >= doc.node_count() {
        return;
    }
    let data = doc.get(id).data();
    if data.attr("disabled").is_some() {
        return;
    }
    let Some(raw_name) = data.attr("name") else {
        return;
    };
    let name = raw_name.trim();
    if name.is_empty() {
        return;
    }
    out.push(SnapshotEntry::Text {
        name: name.to_string(),
        value: data.attr("value").unwrap_or("").to_string(),
    });
}

/// Empty `action` is the document URL. Anything else is joined to `base_url`.
pub(crate) fn resolve_submission_action(action: &str, urls: SubmissionUrls<'_>) -> String {
    let action = action.trim();
    if action.is_empty() {
        return urls.document_url.to_string();
    }
    if has_scheme(action) {
        return action.to_string();
    }
    join_base(urls.base_url, action)
}

fn has_scheme(url: &str) -> bool {
    let Some(end) = url.find(':') else {
        return false;
    };
    let scheme = &url[..end];
    let mut chars = scheme.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn join_base(base: &str, rel: &str) -> String {
    if rel.starts_with("//") {
        let scheme = base.split(':').next().unwrap_or("https");
        return format!("{scheme}:{rel}");
    }
    let (origin, path) = split_origin_path(base);
    if rel.starts_with('/') {
        return format!("{origin}{rel}");
    }
    if rel.starts_with('?') || rel.starts_with('#') {
        return format!("{origin}{path}{rel}");
    }
    let (path_part, suffix) = split_suffix(rel);
    let dir = parent_dir(&path);
    let mut segments: Vec<&str> = dir.split('/').filter(|seg| !seg.is_empty()).collect();
    for seg in path_part.split('/') {
        if seg == "." || seg.is_empty() {
            continue;
        }
        if seg == ".." {
            segments.pop();
        } else {
            segments.push(seg);
        }
    }
    let joined = segments.join("/");
    format!("{origin}/{joined}{suffix}")
}

fn split_suffix(rel: &str) -> (&str, &str) {
    let cut = rel.find(['?', '#']).unwrap_or(rel.len());
    (&rel[..cut], &rel[cut..])
}

fn parent_dir(path: &str) -> &str {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    if path.ends_with('/') {
        return path;
    }
    match path.rfind('/') {
        Some(index) => &path[..=index],
        None => "/",
    }
}

fn split_origin_path(url: &str) -> (String, String) {
    let Some(scheme_end) = url.find("://") else {
        return (String::new(), url.to_string());
    };
    let after = &url[scheme_end + 3..];
    let path_at = after.find(['/', '?', '#']).unwrap_or(after.len());
    let origin = url[..scheme_end + 3 + path_at].to_string();
    let path = if after[path_at..].starts_with('/') {
        after[path_at..].to_string()
    } else {
        "/".to_string()
    };
    (origin, path)
}

/// Keeps `Arc<dyn SelectionLookup + Send + Sync>` in the type the runtime stores.
#[cfg(test)]
fn lookup_slot() -> std::sync::Arc<dyn SelectionLookup + Send + Sync> {
    struct Empty;
    impl SelectionLookup for Empty {
        fn current(&self, _document: u64, _control: NodeId) -> Option<FileSelectionRef> {
            None
        }
    }
    std::sync::Arc::new(Empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_html;

    struct Fixed(Option<FileSelectionRef>);

    impl SelectionLookup for Fixed {
        fn current(&self, document: u64, control: NodeId) -> Option<FileSelectionRef> {
            self.0
                .filter(|picked| picked.document == document && picked.control == control)
        }
    }

    /// Returns one ref no matter which control is being collected.
    struct Always(FileSelectionRef);

    impl SelectionLookup for Always {
        fn current(&self, _document: u64, _control: NodeId) -> Option<FileSelectionRef> {
            Some(self.0)
        }
    }

    fn urls<'a>(document_url: &'a str, base_url: &'a str) -> SubmissionUrls<'a> {
        SubmissionUrls {
            document_url,
            base_url,
        }
    }

    fn snap(
        html: &str,
        submitter: Option<&str>,
        document_url: &str,
        base_url: &str,
    ) -> SubmissionSnapshot {
        let doc = parse_html(html);
        let form = doc.get_element_by_id("f").unwrap();
        let submitter = submitter.map(|id| doc.get_element_by_id(id).unwrap());
        doc.snapshot_form_submission(
            form,
            submitter,
            4,
            urls(document_url, base_url),
            &Fixed(None),
        )
        .unwrap()
    }

    #[test]
    fn an_empty_action_uses_the_document_url_not_the_base() {
        let shot = snap(
            "<base href=\"https://cdn.example/assets/\"><form id=\"f\" action=\"\" method=\"post\"><input name=\"q\" value=\"a\"></form>",
            None,
            "https://page.example/app/index.html",
            "https://cdn.example/assets/",
        );
        assert_eq!(shot.action, "https://page.example/app/index.html");
        assert_eq!(shot.epoch, 4);
        assert_eq!(shot.method, SnapshotMethod::Post);
    }

    #[test]
    fn a_relative_action_resolves_against_the_base_url() {
        let shot = snap(
            "<base href=\"https://cdn.example/assets/\"><form id=\"f\" action=\"submit\"><input name=\"q\" value=\"a\"></form>",
            None,
            "https://page.example/app/index.html",
            "https://cdn.example/assets/",
        );
        assert_eq!(shot.action, "https://cdn.example/assets/submit");
    }

    #[test]
    fn the_submitter_formaction_overrides_the_form_action() {
        let shot = snap(
            "<form id=\"f\" action=\"/from-form\"><input name=\"q\" value=\"a\"><button id=\"go\" formaction=\"override\"></button></form>",
            Some("go"),
            "https://page.example/app/index.html",
            "https://cdn.example/assets/",
        );
        assert_eq!(shot.action, "https://cdn.example/assets/override");
    }

    #[test]
    fn an_empty_formaction_uses_the_document_url() {
        let shot = snap(
            "<form id=\"f\" action=\"/from-form\"><button id=\"go\" formaction=\"\"></button></form>",
            Some("go"),
            "https://page.example/app/index.html",
            "https://cdn.example/assets/",
        );
        assert_eq!(shot.action, "https://page.example/app/index.html");
    }

    #[test]
    fn a_direct_submit_captures_an_invalid_control() {
        let shot = snap(
            "<form id=\"f\" action=\"/save\" method=\"POST\"><input name=\"q\" pattern=\"a+\" value=\"b\"></form>",
            None,
            "https://page.example/",
            "https://page.example/",
        );
        assert_eq!(shot.action, "https://page.example/save");
        assert_eq!(
            shot.entries,
            vec![SnapshotEntry::Text {
                name: "q".to_string(),
                value: "b".to_string(),
            }]
        );
    }

    #[test]
    fn a_forged_file_attribute_is_not_a_selection() {
        let doc = parse_html(
            "<form id=\"f\" enctype=\"multipart/form-data\"><input id=\"file\" type=\"file\" name=\"up\" value=\"secret-name.txt\" filename=\"note.txt\" data-filename=\"note.txt\" data-file-content=\"SECRETBYTES\" data-content-type=\"text/plain\"></form>",
        );
        let form = doc.get_element_by_id("f").unwrap();
        let shot = doc
            .snapshot_form_submission(
                form,
                None,
                1,
                urls("https://page.example/", "https://page.example/"),
                &Fixed(None),
            )
            .unwrap();
        assert_eq!(
            shot.entries,
            vec![SnapshotEntry::File {
                name: "up".to_string(),
                selection: None,
            }]
        );
        let rendered = format!("{shot:?}");
        assert!(!rendered.contains("SECRETBYTES"));
        assert!(!rendered.contains("secret-name"));
        assert!(!rendered.contains("note.txt"));
        assert!(!rendered.contains("text/plain"));
    }

    #[test]
    fn a_ref_for_another_document_is_not_a_selection() {
        let doc = parse_html("<form id=\"f\"><input id=\"file\" type=\"file\" name=\"up\"></form>");
        let form = doc.get_element_by_id("f").unwrap();
        let file = doc.get_element_by_id("file").unwrap();
        let foreign = FileSelectionRef {
            document: doc.identity().wrapping_add(100),
            control: file,
            version: 1,
        };
        let shot = doc
            .snapshot_form_submission(
                form,
                None,
                1,
                urls("https://page.example/", "https://page.example/"),
                &Always(foreign),
            )
            .unwrap();
        assert_eq!(
            shot.entries,
            vec![SnapshotEntry::File {
                name: "up".to_string(),
                selection: None,
            }]
        );
    }

    #[test]
    fn a_ref_for_another_control_is_not_a_selection() {
        let doc = parse_html("<form id=\"f\"><input id=\"file\" type=\"file\" name=\"up\"></form>");
        let form = doc.get_element_by_id("f").unwrap();
        let file = doc.get_element_by_id("file").unwrap();
        let other = FileSelectionRef {
            document: doc.identity(),
            control: NodeId::from_raw(file.index() as u32 + 4),
            version: 1,
        };
        let shot = doc
            .snapshot_form_submission(
                form,
                None,
                1,
                urls("https://page.example/", "https://page.example/"),
                &Always(other),
            )
            .unwrap();
        assert_eq!(
            shot.entries,
            vec![SnapshotEntry::File {
                name: "up".to_string(),
                selection: None,
            }]
        );
    }

    #[test]
    fn a_picker_ref_is_pinned_and_attributes_are_still_ignored() {
        let doc = parse_html(
            "<form id=\"f\" enctype=\"multipart/form-data\"><input id=\"file\" type=\"file\" name=\"up\" value=\"hint.txt\" data-file-content=\"SECRETBYTES\"></form>",
        );
        let form = doc.get_element_by_id("f").unwrap();
        let file = doc.get_element_by_id("file").unwrap();
        let picked = FileSelectionRef {
            document: doc.identity(),
            control: file,
            version: 3,
        };
        let shot = doc
            .snapshot_form_submission(
                form,
                None,
                1,
                urls("https://page.example/", "https://page.example/"),
                &Fixed(Some(picked)),
            )
            .unwrap();
        assert_eq!(
            shot.entries,
            vec![SnapshotEntry::File {
                name: "up".to_string(),
                selection: Some(picked),
            }]
        );
        let rendered = format!("{shot:?}");
        assert!(!rendered.contains("SECRETBYTES"));
        assert!(!rendered.contains("hint.txt"));
    }

    #[test]
    fn target_prefers_formtarget_then_the_form_target() {
        let overridden = snap(
            "<form id=\"f\" target=\"_self\"><button id=\"go\" formtarget=\"_blank\"></button></form>",
            Some("go"),
            "https://page.example/",
            "https://page.example/",
        );
        assert_eq!(overridden.target, "_blank");
        let fallback = snap(
            "<form id=\"f\" target=\"_parent\"></form>",
            None,
            "https://page.example/",
            "https://page.example/",
        );
        assert_eq!(fallback.target, "_parent");
        let empty = snap(
            "<form id=\"f\"></form>",
            None,
            "https://page.example/",
            "https://page.example/",
        );
        assert_eq!(empty.target, "");
    }

    #[test]
    fn text_plain_is_captured_as_text_plain() {
        let shot = snap(
            "<form id=\"f\" method=\"POST\" enctype=\"text/plain\"><input name=\"q\" value=\"a b\"></form>",
            None,
            "https://page.example/",
            "https://page.example/",
        );
        assert_eq!(shot.enctype, SnapshotEnctype::TextPlain);
        assert_eq!(shot.method, SnapshotMethod::Post);
    }

    #[test]
    fn missing_and_non_form_nodes_are_the_only_errors() {
        let doc = parse_html("<div id=\"d\"></div><form id=\"f\"></form>");
        let div = doc.get_element_by_id("d").unwrap();
        assert_eq!(
            doc.snapshot_form_submission(
                div,
                None,
                0,
                urls("https://d/", "https://d/"),
                &Fixed(None)
            )
            .err(),
            Some(SnapshotError::NotAForm)
        );
        assert_eq!(
            doc.snapshot_form_submission(
                NodeId::from_raw(9_999),
                None,
                0,
                urls("https://d/", "https://d/"),
                &Fixed(None)
            )
            .err(),
            Some(SnapshotError::MissingForm)
        );
    }

    #[test]
    fn the_lookup_trait_object_is_send_and_sync() {
        let held: std::sync::Arc<dyn SelectionLookup + Send + Sync> = lookup_slot();
        let doc = parse_html("<form id=\"f\" action=\"\"></form>");
        let form = doc.get_element_by_id("f").unwrap();
        let shot = doc
            .snapshot_form_submission(
                form,
                None,
                9,
                urls("https://doc/", "https://base/"),
                held.as_ref(),
            )
            .unwrap();
        assert_eq!(shot.epoch, 9);
        assert_eq!(shot.document, doc.identity());
        assert_eq!(shot.action, "https://doc/");
    }
}
