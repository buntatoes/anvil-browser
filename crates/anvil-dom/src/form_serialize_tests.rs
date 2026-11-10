// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato

use super::*;

#[test]
fn text_plain_is_crlf_and_files_are_empty_without_a_choice() {
    let doc = parse_html(
        "<form id=\"f\" method=\"POST\" enctype=\"text/plain\"><input name=\"q\" value=\"a b\"></form>",
    );
    let form = doc.get_element_by_id("f").unwrap();
    let (_action, method, content_type, body) = doc.serialize_form_with_submitter(form, None);
    assert_eq!(method, "POST");
    assert_eq!(content_type, "text/plain;charset=UTF-8");
    assert_eq!(body, "q=a b\r\n");

    let doc = parse_html(
        "<form id=\"f\" method=\"POST\" enctype=\"multipart/form-data\"><input type=\"file\" name=\"up\" value=\"secret-name.txt\" filename=\"note.txt\" data-filename=\"note.txt\" data-file-content=\"SECRETBYTES\" data-content-type=\"text/plain\"></form>",
    );
    let form = doc.get_element_by_id("f").unwrap();
    let (_a, _m, content_type, body) = doc.serialize_form_with_submitter(form, None);
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    assert!(!body.contains("SECRETBYTES"), "{body}");
    assert!(!body.contains("secret-name"), "{body}");
    assert!(!body.contains("note.txt"), "{body}");
    assert!(!body.contains("text/plain"), "{body}");
    assert!(body.contains("filename=\"\""), "{body}");
    assert!(body.contains("application/octet-stream"), "{body}");
}

#[test]
fn pattern_rejects_a_non_empty_mismatch_and_ignores_a_bad_pattern() {
    let doc = parse_html("<input id=\"p\" pattern=\"a+\" value=\"b\">");
    let id = doc.get_element_by_id("p").unwrap();
    assert_eq!(doc.is_control_valid(id), Some(false));
    let doc = parse_html("<input id=\"p\" pattern=\"a+\" value=\"aaa\">");
    let id = doc.get_element_by_id("p").unwrap();
    assert_eq!(doc.is_control_valid(id), Some(true));
    let doc = parse_html("<input id=\"p\" pattern=\"(?=a)\" value=\"zzz\">");
    let id = doc.get_element_by_id("p").unwrap();
    assert_eq!(doc.is_control_valid(id), Some(true));
}
