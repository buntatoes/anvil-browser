// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Sticky boxes, transitions, and animations, seen from the page and the session.

use anvil_browser::{Session, Viewport, render_html};

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
};

const STICKY_PAGE: &str = r#"<html><head><style>
  body { margin: 0; }
  #bar { position: sticky; top: 0; height: 40px; background: #333; }
  #bar a { display: block; height: 40px; }
  .tall { height: 2000px; }
  .tall a { display: block; height: 40px; }
</style></head><body>
  <div id="bar"><a href="https://example.com/bar">bar</a></div>
  <div class="tall"><a href="https://example.com/under">under</a></div>
</body></html>"#;

#[test]
fn a_sticky_header_is_clicked_where_it_is_drawn() {
    let mut session = Session::new(VP);
    session.open_html(STICKY_PAGE, "https://example.com/");
    // Not scrolled: the header is at the top in both placements.
    assert_eq!(
        session.hit_link(20.0, 10.0).as_deref(),
        Some("https://example.com/bar")
    );
    // Scrolled down: the header still sits at the top of the view, so a
    // click there hits it, not the content that scrolled underneath.
    session.scroll_by(600.0);
    assert!(session.scroll() > 500.0);
    assert_eq!(
        session.hit_link(20.0, 10.0).as_deref(),
        Some("https://example.com/bar")
    );
}

#[test]
fn page_hit_test_in_view_moves_only_sticky_boxes() {
    let page = render_html(STICKY_PAGE, "https://example.com/", VP);
    let view = anvil_layout::Rect {
        x: 0.0,
        y: 600.0,
        width: VP.width,
        height: VP.height,
    };
    // At page y 610 the in-flow test finds the tall block; the view-aware
    // test finds the sticky header drawn there.
    assert_eq!(page.link_at(20.0, 610.0), None);
    assert_eq!(
        page.link_at_in_view(20.0, 610.0, Some(view)),
        Some("https://example.com/bar")
    );
    // The link in the tall block is still where it was.
    assert_eq!(
        page.link_at_in_view(20.0, 60.0, Some(anvil_layout::Rect { y: 0.0, ..view })),
        Some("https://example.com/under")
    );
}

const TRANSITION_PAGE: &str = r#"<html><head><style>
  body { margin: 0; }
  #box { width: 100px; height: 20px; transition: width 100ms linear; }
</style></head><body><div id="box"></div></body></html>"#;

fn box_width(page: &anvil_browser::Page) -> f32 {
    let id = page.document.get_element_by_id("box").expect("box");
    page.find_layout_rect(id).expect("laid out").width
}

#[test]
fn a_style_change_runs_its_transition_frame_by_frame() {
    let mut page = render_html(TRANSITION_PAGE, "https://example.com/", VP);
    assert!(!page.is_animating(), "nothing runs on the first pass");
    assert!(
        !page.animation_frame_at(VP, 10.0),
        "a still page is not restyled"
    );
    assert_eq!(box_width(&page), 100.0);

    let id = page.document.get_element_by_id("box").unwrap();
    page.document.set_attribute(id, "style", "width: 200px");
    page.restage_at(VP, 1000.0);
    assert!(page.is_animating());
    assert_eq!(box_width(&page), 100.0);

    assert!(page.animation_frame_at(VP, 1050.0));
    let mid = box_width(&page);
    assert!((mid - 150.0).abs() < 1.0, "halfway: {mid}");

    // The frame where it finishes still draws, with the end value.
    assert!(page.animation_frame_at(VP, 1100.0));
    assert_eq!(box_width(&page), 200.0);
    assert!(!page.is_animating());
    assert!(!page.animation_frame_at(VP, 1200.0));
}

const KEYFRAMES_PAGE: &str = r#"<html><head><style>
  body { margin: 0; }
  @keyframes grow { from { width: 0px; } to { width: 100px; } }
  #box { height: 20px; width: 50px; animation-name: grow; animation-duration: 200ms;
         animation-timing-function: linear; }
</style></head><body><div id="box"></div></body></html>"#;

#[test]
fn a_keyframes_animation_starts_at_load_and_stops_at_its_end() {
    let mut page = render_html(KEYFRAMES_PAGE, "https://example.com/", VP);
    assert!(
        page.is_animating(),
        "a named animation starts with the page"
    );
    assert!(page.animation_frame_at(VP, 100.0));
    let mid = box_width(&page);
    assert!((mid - 50.0).abs() < 1.0, "halfway: {mid}");
    assert!(page.animation_frame_at(VP, 250.0));
    assert_eq!(
        box_width(&page),
        50.0,
        "after the end the normal width is back"
    );
    assert!(!page.is_animating());
    assert!(!page.animation_frame_at(VP, 300.0));
}

#[test]
fn the_session_drives_frames_only_while_something_runs() {
    let mut session = Session::new(VP);
    session.open_html(TRANSITION_PAGE, "https://example.com/");
    assert!(!session.is_animating());
    assert!(!session.animation_frame());

    // Real time here, so the animation is long enough to still be running
    // after a slow debug-build load.
    let long = KEYFRAMES_PAGE.replace("200ms", "1500ms");
    session.open_html(&long, "https://example.com/k");
    assert!(session.is_animating());
    assert!(session.animation_frame());
    // Let it run out, then one last frame draws the end state.
    std::thread::sleep(std::time::Duration::from_millis(1600));
    assert!(session.animation_frame());
    assert!(!session.is_animating());
    assert!(!session.animation_frame());
}
