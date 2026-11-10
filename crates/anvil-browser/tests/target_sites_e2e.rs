// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Multipart form posts, history routing, and sticky scroll clamping.

use anvil_browser::{Viewport, render_html};

const VP: Viewport = Viewport {
    width: 1280.0,
    height: 800.0,
};

#[test]
fn test_multipart_form_data_history_routing_and_sticky_scroll_clamping() {
    use anvil_browser::{ClickOutcome, Session};

    let html = r#"<!DOCTYPE html>
<html>
<head>
<style>
  body { margin: 0; }
  #sec { height: 600px; }
  #sticky-hdr { position: sticky; top: 0px; height: 48px; background: #1e293b; color: #fff; }
  #filler { height: 1400px; }
</style>
</head>
<body>
  <div id="sec">
    <div id="sticky-hdr">Pinned Sticky Header</div>
    <form id="upload-form" action="/api/upload" method="post" enctype="multipart/form-data">
      <input type="text" name="title" value="Quarterly Report">
      <input type="file" name="attachment" value="summary.txt">
      <button id="send-btn" type="submit" name="submitter" value="upload">Upload</button>
    </form>
  </div>
  <div id="filler">Tall scrollable body</div>
  <div id="route-out">initial</div>
</body>
</html>"#;

    let mut session = Session::new(VP);
    session.open_html(html, "https://app.example.com/workspace?tab=overview");

    // 1. Native <form enctype="multipart/form-data"> submission via Session::submit_form_for
    let send_btn = session
        .active()
        .current()
        .unwrap()
        .document
        .get_element_by_id("send-btn")
        .unwrap();
    match session.submit_form_for(send_btn) {
        ClickOutcome::SubmitPost {
            url,
            body,
            content_type,
        } => {
            assert_eq!(url, "https://app.example.com/api/upload");
            assert!(
                content_type.starts_with("multipart/form-data; boundary="),
                "{content_type}"
            );
            let boundary = content_type.split("boundary=").nth(1).unwrap();
            let body = String::from_utf8(body).expect("multipart text");
            assert!(body.contains(boundary), "{body}");
            assert!(body.contains("Content-Disposition: form-data; name=\"title\""));
            assert!(body.contains("Quarterly Report"));
            assert!(
                !body.contains("summary.txt"),
                "a value attribute is not file bytes: {body}"
            );
            assert!(body.contains("filename=\"\""));
            assert!(body.contains("Content-Disposition: form-data; name=\"submitter\""));
        }
        other => panic!("expected SubmitPost with multipart body, got {other:?}"),
    }

    // 2. FormData(formElement, submitter) + fetch() multipart serialization + History API & URLSearchParams live routing
    let mut page = render_html(html, "https://app.example.com/workspace?tab=overview", VP);
    let js = r#"
        "use cinder";
        var form = document.getElementById('upload-form');
        var btn = document.getElementById('send-btn');
        var fd = new FormData(form, btn);
        fd.append('extra', '1');
        fetch('/api/upload-fetch', { method: 'POST', body: fd });

        var popped = '';
        window.addEventListener('popstate', function(ev) {
            popped = (ev.state ? ev.state.tab : 'none') + '@' + location.pathname + location.search;
            document.getElementById('route-out').textContent = popped;
        });
        var params = new URLSearchParams(location.search);
        params.set('tab', 'analytics');
        params.append('sort', 'desc');
        params.sort();
        history.pushState({ tab: 'analytics' }, '', '/workspace?' + params.toString());
        history.replaceState({ tab: 'analytics-v2' }, '', '/workspace/v2?' + params.toString());
        history.pushState({ tab: 'settings' }, '', '/workspace/settings');
        history.back();
    "#;
    let res = page.runtime.eval_with_cinder_engine(&mut page.document, js);
    assert!(res.is_ok(), "cinder script failed: {res:?}");
    page.restage(VP);

    let (fetch_url, fetch_method, fetch_ct, fetch_body) = page
        .runtime
        .last_fetch_request()
        .expect("fetch request captured");
    assert_eq!(fetch_url, "/api/upload-fetch");
    assert_eq!(fetch_method, "POST");
    assert!(fetch_ct.starts_with("multipart/form-data; boundary="));
    assert!(fetch_body.contains("Quarterly Report"));
    assert!(fetch_body.contains("name=\"extra\""));

    assert_eq!(
        page.url,
        "https://app.example.com:443/workspace/v2?sort=desc&tab=analytics"
    );
    let route_out = page.document.get_element_by_id("route-out").unwrap();
    assert_eq!(
        page.document.text_content(route_out),
        "analytics-v2@/workspace/v2?sort=desc&tab=analytics"
    );

    // 3. CSS position: sticky scroll clamping, hit_test_in_view, and scrolled getBoundingClientRect()
    session.scroll_by(250.0);
    {
        let view = anvil_layout::Rect {
            x: 0.0,
            y: session.scroll(),
            width: VP.width,
            height: VP.height,
        };
        let active_page = session.active().current().unwrap();
        let hdr_id = active_page
            .document
            .get_element_by_id("sticky-hdr")
            .unwrap();
        let rect_in_view = active_page.find_layout_rect_in_view(hdr_id, view).unwrap();
        assert!(
            (rect_in_view.y - 250.0).abs() < 1.0,
            "expected sticky header to stay pinned at scroll_y=250, got y={}",
            rect_in_view.y
        );
        let hit = active_page.hit_test_in_view(40.0, 265.0, view).unwrap();
        assert!(
            hit == hdr_id || active_page.document.get(hit).parent() == Some(hdr_id),
            "expected hit_test_in_view to hit sticky header or its text child, got {hit:?}"
        );
    }
    page.runtime
        .notify_layout_and_scroll(&mut page.document, VP.width, VP.height, 250.0);
    let bcr_top = page
        .runtime
        .eval_with_cinder_engine(
            &mut page.document,
            "Math.round(document.getElementById('sticky-hdr').getBoundingClientRect().top)",
        )
        .unwrap();
    assert_eq!(bcr_top, "0");
}
