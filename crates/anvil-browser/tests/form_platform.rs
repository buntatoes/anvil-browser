// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Browser form activation and submission. These do not call into the script
//! host's requestSubmit implementation.

use anvil_browser::{ClickOutcome, Session, Viewport};

const VP: Viewport = Viewport {
    width: 320.0,
    height: 200.0,
};

fn hit(session: &Session, id: &str) -> (f32, f32) {
    let page = session.active().current().unwrap();
    let node = page.document.get_element_by_id(id).unwrap();
    let rect = page.find_layout_rect(node).expect(id);
    (rect.x + 2.0, rect.y + 2.0)
}

#[test]
fn a_disabled_control_click_does_not_run_or_activate() {
    let mut session = Session::new(VP);
    session.open_html(
        "<style>html,body,button,input,p{margin:0;padding:0}button,input{display:block;width:80px;height:20px}</style>
         <button id=\"b\" disabled onclick=\"document.getElementById('log').textContent='go'\">x</button>
         <input id=\"box\" type=\"checkbox\" disabled>
         <p id=\"log\">init</p>",
        "http://127.0.0.1/form",
    );
    let (x, y) = hit(&session, "b");
    assert_eq!(session.click_at(x, y), ClickOutcome::None);
    let (x, y) = hit(&session, "box");
    assert_eq!(session.click_at(x, y), ClickOutcome::None);
    let page = session.active().current().unwrap();
    let log = page.document.get_element_by_id("log").unwrap();
    assert_eq!(page.document.text_content(log), "init");
    let box_id = page.document.get_element_by_id("box").unwrap();
    assert!(!page.document.is_checked(box_id));
}

#[test]
fn an_enabled_checkbox_still_toggles() {
    let mut session = Session::new(VP);
    session.open_html(
        "<style>html,body,input{margin:0;padding:0}input{display:block;width:80px;height:20px}</style>
         <input id=\"box\" type=\"checkbox\">",
        "http://127.0.0.1/form",
    );
    let (x, y) = hit(&session, "box");
    let outcome = session.click_at(x, y);
    let page = session.active().current().unwrap();
    let box_id = page.document.get_element_by_id("box").unwrap();
    assert_eq!(outcome, ClickOutcome::CheckboxToggled(box_id, true));
    assert!(page.document.is_checked(box_id));
}

#[test]
fn native_get_is_a_url_and_native_post_keeps_the_body_and_type() {
    let mut session = Session::new(VP);
    session.open_html(
        "<form id=\"g\" action=\"/search\" method=\"GET\">
           <input name=\"q\" value=\"anvil\">
           <button id=\"go\" type=\"submit\">Go</button>
         </form>",
        "http://127.0.0.1/page",
    );
    let go = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("go")
        .unwrap();
    assert_eq!(
        session.submit_form_for(go),
        ClickOutcome::Navigate("http://127.0.0.1/search?q=anvil".to_string())
    );

    let mut session = Session::new(VP);
    session.open_html(
        "<form id=\"p\" action=\"/save\" method=\"POST\">
           <input name=\"q\" value=\"anvil\">
           <button id=\"go\" type=\"submit\">Go</button>
         </form>",
        "http://127.0.0.1/page",
    );
    let go = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("go")
        .unwrap();
    assert_eq!(
        session.submit_form_for(go),
        ClickOutcome::SubmitPost {
            url: "http://127.0.0.1/save".to_string(),
            body: b"q=anvil".to_vec(),
            content_type: "application/x-www-form-urlencoded".to_string(),
        }
    );
}

#[test]
fn a_failing_module_does_not_block_the_classic_script() {
    let mut session = Session::new(VP);
    session.open_html(
        "<p id=\"mod\">mod-init</p><p id=\"classic\">classic-init</p>\
         <script type=\"module\">throw 1;</script>\
         <script>document.getElementById('classic').textContent='classic'</script>",
        "http://127.0.0.1/modules",
    );
    let page = session.active().current().unwrap();
    let classic = page.document.get_element_by_id("classic").unwrap();
    assert_eq!(page.document.text_content(classic), "classic");
    assert!(
        page.document
            .get_element_by_id("anvil-module-unsupported")
            .is_none()
    );
}

#[test]
fn an_invalid_form_does_not_submit_unless_validation_is_skipped() {
    let mut session = Session::new(VP);
    session.open_html(
        "<form action=\"/save\" method=\"POST\"><input id=\"em\" type=\"email\" name=\"em\" value=\"not-an-email\"><button id=\"go\" type=\"submit\">Go</button></form>",
        "http://127.0.0.1/page",
    );
    let go = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("go")
        .unwrap();
    assert_eq!(session.submit_form_for(go), ClickOutcome::None);

    session.open_html(
        "<form action=\"/save\" method=\"POST\" novalidate><input type=\"email\" name=\"em\" value=\"not-an-email\"><button id=\"go\" type=\"submit\">Go</button></form>",
        "http://127.0.0.1/page",
    );
    let go = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("go")
        .unwrap();
    assert!(matches!(
        session.submit_form_for(go),
        ClickOutcome::SubmitPost { .. }
    ));

    session.open_html(
        "<form action=\"/save\" method=\"POST\"><input type=\"email\" name=\"em\" value=\"not-an-email\"><button id=\"go\" type=\"submit\" formnovalidate>Go</button></form>",
        "http://127.0.0.1/page",
    );
    let go = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("go")
        .unwrap();
    assert!(matches!(
        session.submit_form_for(go),
        ClickOutcome::SubmitPost { .. }
    ));
}

#[test]
fn a_disabled_submit_control_does_not_navigate() {
    let mut session = Session::new(VP);
    session.open_html(
        "<style>html,body,button{margin:0;padding:0}button{display:block;width:80px;height:20px}</style>
         <form action=\"/save\" method=\"POST\">
           <input name=\"q\" value=\"anvil\">
           <button id=\"go\" type=\"submit\" disabled>Go</button>
         </form>",
        "http://127.0.0.1/page",
    );
    let (x, y) = hit(&session, "go");
    assert_eq!(session.click_at(x, y), ClickOutcome::None);
}
