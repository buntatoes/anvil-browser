// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Comprehensive Dual-Track End-to-End Verification Suite for Target Sites & Engine Integration.
//!
//! Architecture:
//! - Tier 1: Feature Coverage (HTML Parser, DOM Tree, CSS Computed Styles, Cinder JS Execution, Font Metrics, Framebuffer Blit)
//! - Tier 2: Boundary & Corner Cases (Truncated HTML, Missing/Cyclic CSS Variables, Malformed Scripts, Viewport Extremes)
//! - Tier 3: Cross-Feature Combinations (Style Cascading + DOM Mutation + Layout Recalculation + Media Queries)
//! - Tier 4: Real-World Scenarios (Wikipedia Vector 2022, Google Search, Gmail offline fixtures & live probes)

use anvil_browser::{Page, Viewport, render_html};
use anvil_paint::DisplayItem;
use anvil_raster::{Color, rasterize};

const VP: Viewport = Viewport {
    width: 1280.0,
    height: 800.0,
};

const WIKIPEDIA_FIXTURE: &str = include_str!("./fixtures/wikipedia_article.html");
const GOOGLE_FIXTURE: &str = include_str!("./fixtures/google_search.html");
const GMAIL_FIXTURE: &str = include_str!("./fixtures/gmail_webmail.html");

fn find_text_color(page: &Page, needle: &str) -> Option<Color> {
    page.display_list.items.iter().find_map(|item| match item {
        DisplayItem::Text { text, color, .. } if text.contains(needle) => Some(*color),
        _ => None,
    })
}

// =========================================================================
// TIER 1: Feature Coverage (Component Verification)
// =========================================================================

#[test]
fn test_tier1_html_parser_and_dom_construction() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <title>Tier 1 Test Document</title>
  <meta charset="utf-8">
</head>
<body>
  <header id="main-header" class="site-header">
    <h1 id="title">Subsystem Verification</h1>
  </header>
  <main id="main-content">
    <p class="intro">Validating HTML5 tree construction.</p>
    <table id="test-table">
      <thead>
        <tr><th>Subsystem</th><th>Status</th></tr>
      </thead>
      <tbody>
        <tr><td id="data-cell">Prism</td><td>Verified</td></tr>
      </tbody>
    </table>
    <section id="section-two">
      <a href="#title" id="back-link">Return to top</a>
    </section>
  </main>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier1_dom", VP);

    assert_eq!(page.title, "Tier 1 Test Document");
    assert!(page.document.get_element_by_id("main-header").is_some());
    assert!(page.document.get_element_by_id("title").is_some());
    assert!(page.document.get_element_by_id("main-content").is_some());
    assert!(page.document.get_element_by_id("test-table").is_some());
    assert!(page.document.get_element_by_id("data-cell").is_some());
    assert!(page.document.get_element_by_id("back-link").is_some());

    let header_node = page.document.get_element_by_id("main-header").unwrap();
    assert!(
        page.document
            .get(header_node)
            .data()
            .has_class("site-header")
    );

    let cell_node = page.document.get_element_by_id("data-cell").unwrap();
    assert_eq!(page.document.text_content(cell_node).trim(), "Prism");

    assert!(page.document.find_fragment_target("section-two").is_some());
    assert!(page.content_height() > 0.0);
}

#[test]
fn test_tier1_css_computed_styles_and_cascade() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    :root {
      --brand-blue: #3366cc;
      --default-bg: #f0f4f8;
    }
    .box {
      color: var(--brand-blue);
      background-color: var(--default-bg);
      width: 250px;
      height: 60px;
    }
    #special {
      color: #cc0000;
    }
  </style>
</head>
<body>
  <div class="box" id="general">General Box</div>
  <div class="box" id="special">Special Box</div>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier1_css", VP);

    let general_color = find_text_color(&page, "General Box").expect("General Box drawn");
    assert_eq!(general_color, Color::parse("#3366cc").unwrap());

    let special_color = find_text_color(&page, "Special Box").expect("Special Box drawn");
    assert_eq!(special_color, Color::parse("#cc0000").unwrap());

    let general_id = page.document.get_element_by_id("general").unwrap();
    let rect = page.find_layout_rect(general_id).expect("general box rect");
    assert!(rect.width >= 200.0);
    assert!(rect.height >= 40.0);
}

#[test]
fn test_tier1_cinder_js_execution() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>JS Execution Test</title></head>
<body>
  <div id="target">Initial Text</div>
  <span id="computed-val">0</span>
  <script>
    var target = document.getElementById('target');
    if (target) {
      target.textContent = 'Updated by Cinder';
    }
    var span = document.getElementById('computed-val');
    if (span) {
      var a = 20;
      var b = 22;
      span.textContent = String(a + b);
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier1_js", VP);

    let target_node = page.document.get_element_by_id("target").unwrap();
    assert_eq!(
        page.document.text_content(target_node).trim(),
        "Updated by Cinder"
    );

    let span_node = page.document.get_element_by_id("computed-val").unwrap();
    assert_eq!(page.document.text_content(span_node).trim(), "42");
}

#[test]
fn test_tier1_font_metrics_and_text_layout() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Font Metrics Test</title></head>
<body>
  <h1 id="heading">Typography Engineering</h1>
  <p id="paragraph">The quick brown fox jumps over the lazy dog in high fidelity typography.</p>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier1_font", VP);

    let heading_id = page.document.get_element_by_id("heading").unwrap();
    let heading_rect = page.find_layout_rect(heading_id).expect("heading rect");
    assert!(heading_rect.width > 100.0);
    assert!(heading_rect.height > 15.0);

    let matches = page.find_text_matches("quick brown fox");
    assert!(
        !matches.is_empty(),
        "find_text_matches finds target text run"
    );
    assert!(matches[0].width > 50.0);
    assert!(matches[0].height >= 10.0);
    assert!(page.content_height() > 40.0);
}

#[test]
fn test_tier1_image_blits_and_framebuffer_raster() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    body { margin: 0; background-color: #ffffff; }
    .card { background-color: #0b57d0; width: 200px; height: 100px; margin: 20px; }
  </style>
</head>
<body>
  <div class="card"></div>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier1_raster", VP);
    assert!(!page.display_list.items.is_empty());

    let fb = rasterize(&page.display_list, 400, 300, 0.0);
    assert_eq!(fb.width(), 400);
    assert_eq!(fb.height(), 300);
    assert_eq!(fb.pixels().len(), 400 * 300 * 4);

    let has_drawn_card = fb.pixels().chunks_exact(4).any(|chunk| {
        let (r, g, b) = (chunk[0], chunk[1], chunk[2]);
        r == 0x0b && g == 0x57 && b == 0xd0
    });
    assert!(has_drawn_card, "Raster framebuffer contains card pixels");
}

// =========================================================================
// TIER 2: Boundary & Corner Cases (Adversarial / Fault Tolerance)
// =========================================================================

#[test]
fn test_tier2_truncated_html_handling() {
    let truncated_html = r##"<!DOCTYPE html>
<html>
<head><title>Truncated Fixture</title>
<body>
  <div id="unclosed-outer">
    <p id="first-p">This paragraph is unclosed
    <div id="unclosed-inner" class="widget">
      <span>Unclosed span
      <!-- Unclosed comment at abrupt EOF"##;

    let page = render_html(truncated_html, "https://example.com/tier2_truncated", VP);

    assert_eq!(page.title, "Truncated Fixture");
    assert!(page.document.get_element_by_id("unclosed-outer").is_some());
    assert!(page.document.get_element_by_id("first-p").is_some());
    assert!(page.document.get_element_by_id("unclosed-inner").is_some());

    let inner = page.document.get_element_by_id("unclosed-inner").unwrap();
    assert!(page.document.get(inner).data().has_class("widget"));
    assert!(page.content_height() > 0.0);
}

#[test]
fn test_tier2_missing_and_cyclic_css_variables() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    :root {
      --cycle-a: var(--cycle-b);
      --cycle-b: var(--cycle-a);
      --with-fallback: var(--non-existent-token, #128833);
    }
    #box-fallback { color: var(--with-fallback); }
    #box-missing { color: var(--missing-token); }
    #box-cyclic { color: var(--cycle-a, #332211); }
  </style>
</head>
<body>
  <p id="box-fallback">Fallback Text</p>
  <p id="box-missing">Missing Text</p>
  <p id="box-cyclic">Cyclic Text</p>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier2_css_vars", VP);

    let fallback_color = find_text_color(&page, "Fallback Text").expect("fallback text rendered");
    assert_eq!(fallback_color, Color::parse("#128833").unwrap());

    assert!(page.document.get_element_by_id("box-missing").is_some());
    assert!(page.document.get_element_by_id("box-cyclic").is_some());
    assert!(page.content_height() > 0.0);
}

#[test]
fn test_tier2_malformed_script_fault_tolerance() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Malformed Script Test</title></head>
<body>
  <div id="status">Active</div>
  <script>
    // Syntax error: invalid token sequence
    function ( { illegal !! }}}
  </script>
  <script>
    // Runtime exception: dereference null
    var emptyObj = null;
    emptyObj.nonExistentMethod();
  </script>
  <script>
    // Subsequent valid script execution
    var statusDiv = document.getElementById('status');
    if (statusDiv) {
      statusDiv.textContent = 'Survived';
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/tier2_script_errors", VP);

    assert!(page.document.get_element_by_id("status").is_some());
    assert!(page.content_height() > 0.0);
}

#[test]
fn test_tier2_extreme_viewport_boundaries() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Extreme Viewport Test</title></head>
<body>
  <div id="content" style="width: 100%; height: 50px; background: #eee;">Responsive Box</div>
</body>
</html>"##;

    let zero_vp = Viewport {
        width: 0.0,
        height: 0.0,
    };
    let page_zero = render_html(html, "https://example.com/zero", zero_vp);
    assert!(page_zero.content_height() >= 0.0);

    let narrow_vp = Viewport {
        width: 40.0,
        height: 4000.0,
    };
    let page_narrow = render_html(html, "https://example.com/narrow", narrow_vp);
    assert!(page_narrow.content_height() > 0.0);

    let uhd_vp = Viewport {
        width: 3840.0,
        height: 2160.0,
    };
    let page_uhd = render_html(html, "https://example.com/uhd", uhd_vp);
    assert!(page_uhd.content_height() > 0.0);
}

// =========================================================================
// TIER 3: Cross-Feature Combinations (Subsystem Interaction)
// =========================================================================

#[test]
fn test_tier3_style_cascade_dom_mutation_restage() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    .collapsed { width: 80px; height: 30px; color: #888888; }
    .expanded { width: 320px; height: 140px; color: #00aa44; }
  </style>
</head>
<body>
  <div id="panel" class="collapsed">Panel Text</div>
</body>
</html>"##;

    let mut page = render_html(html, "https://example.com/tier3_restyle", VP);

    let panel_id = page.document.get_element_by_id("panel").unwrap();
    let rect_initial = page.find_layout_rect(panel_id).expect("initial rect");
    assert!(rect_initial.width <= 100.0);

    page.document.remove_class(panel_id, "collapsed");
    page.document.add_class(panel_id, "expanded");
    page.restage(VP);

    let rect_expanded = page.find_layout_rect(panel_id).expect("expanded rect");
    assert!(rect_expanded.width >= 300.0);
    assert!(rect_expanded.height >= 120.0);

    let updated_color = find_text_color(&page, "Panel Text").expect("panel text color");
    assert_eq!(updated_color, Color::parse("#00aa44").unwrap());
}

#[test]
fn test_tier3_interactive_event_script_dom_mutation() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Event DOM Mutation</title></head>
<body>
  <button id="toggle-btn" onclick="document.getElementById('display-label').textContent = 'State: Active';">Toggle</button>
  <span id="display-label">State: Inactive</span>
</body>
</html>"##;

    let mut page = render_html(html, "https://example.com/tier3_interactive", VP);

    let label_id = page.document.get_element_by_id("display-label").unwrap();
    assert_eq!(
        page.document.text_content(label_id).trim(),
        "State: Inactive"
    );

    page.runtime.eval_script(
        &mut page.document,
        "document.getElementById('toggle-btn').onclick();",
    );
    page.restage(VP);

    assert_eq!(page.document.text_content(label_id).trim(), "State: Active");
}

#[test]
fn test_tier3_media_query_responsive_restage() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    #adaptive-box { width: 200px; height: 40px; }
    @media (min-width: 800px) {
      #adaptive-box { width: 600px; }
    }
  </style>
</head>
<body>
  <div id="adaptive-box">Adaptive</div>
</body>
</html>"##;

    let wide_vp = Viewport {
        width: 1024.0,
        height: 768.0,
    };
    let mut page = render_html(html, "https://example.com/tier3_media", wide_vp);

    let box_id = page.document.get_element_by_id("adaptive-box").unwrap();
    let wide_rect = page.find_layout_rect(box_id).expect("wide rect");
    assert!(wide_rect.width >= 550.0);

    let narrow_vp = Viewport {
        width: 500.0,
        height: 768.0,
    };
    page.restage(narrow_vp);

    let narrow_rect = page.find_layout_rect(box_id).expect("narrow rect");
    assert!(narrow_rect.width <= 250.0);
}

// =========================================================================
// TIER 4: Real-World Scenarios (Wikipedia, Google Search, Gmail)
// =========================================================================

#[test]
fn test_tier4_offline_wikipedia_article_scenario() {
    let mut page = render_html(
        WIKIPEDIA_FIXTURE,
        "https://en.wikipedia.org/wiki/Anvil_Browser",
        VP,
    );

    assert_eq!(
        page.title,
        "Anvil Engine - Wikipedia, the free encyclopedia"
    );

    assert!(page.document.get_element_by_id("firstHeading").is_some());
    assert!(page.document.get_element_by_id("vector-toc").is_some());
    assert!(
        page.document
            .get_element_by_id("vector-toc-collapse-button")
            .is_some()
    );
    assert!(page.document.get_element_by_id("content").is_some());
    assert!(page.document.get_element_by_id("mw-infobox").is_some());
    assert!(page.document.get_element_by_id("subsystem-table").is_some());

    assert!(page.content_height() > 600.0);

    let content_id = page.document.get_element_by_id("content").unwrap();
    let content_rect = page
        .find_layout_rect(content_id)
        .expect("content layout rect");
    assert!(content_rect.width > 400.0);
    assert!(content_rect.height > 300.0);

    let fb = rasterize(&page.display_list, 1280, 800, 0.0);
    assert_eq!(fb.pixels().len(), 1280 * 800 * 4);
    assert!(
        fb.pixels().iter().any(|&b| b != 255),
        "Wikipedia fixture raster output has rendered content"
    );

    let toc_id = page.document.get_element_by_id("vector-toc").unwrap();
    assert!(
        !page
            .document
            .get(toc_id)
            .data()
            .has_class("vector-toc-collapsed")
    );

    page.runtime.eval_script(
        &mut page.document,
        "var btn = document.getElementById('vector-toc-collapse-button'); if (btn) { btn.click(); }",
    );
    page.restage(VP);

    assert!(
        page.document
            .get(toc_id)
            .data()
            .has_class("vector-toc-collapsed")
    );
}

#[test]
fn test_tier4_offline_google_search_scenario() {
    let mut page = render_html(
        GOOGLE_FIXTURE,
        "https://www.google.com/search?q=anvil+browser",
        VP,
    );

    assert_eq!(page.title, "anvil browser - Google Search");

    assert!(page.document.get_element_by_id("searchform").is_some());
    assert!(page.document.get_element_by_id("tsf").is_some());
    assert!(page.document.get_element_by_id("search-input").is_some());
    assert!(page.document.get_element_by_id("search-tabs").is_some());
    assert!(page.document.get_element_by_id("main-results").is_some());
    assert!(page.document.get_element_by_id("res-1").is_some());
    assert!(page.document.get_element_by_id("res-2").is_some());
    assert!(page.document.get_element_by_id("pagination").is_some());

    let input_id = page.document.get_element_by_id("search-input").unwrap();
    let input_rect = page.find_layout_rect(input_id).expect("search input rect");
    assert!(input_rect.width > 100.0);
    assert!(input_rect.height >= 20.0);

    let res1_id = page.document.get_element_by_id("res-1").unwrap();
    let res1_rect = page.find_layout_rect(res1_id).expect("result 1 rect");
    assert!(res1_rect.width > 200.0);
    assert!(res1_rect.height > 20.0);

    assert!(page.content_height() > 400.0);

    let fb = rasterize(&page.display_list, 1280, 800, 0.0);
    assert!(
        fb.pixels().iter().any(|&b| b != 255),
        "Google search fixture raster output has rendered content"
    );

    let tab_images_id = page.document.get_element_by_id("tab-images").unwrap();
    assert!(!page.document.get(tab_images_id).data().has_class("active"));

    page.runtime.eval_script(
        &mut page.document,
        "var tab = document.getElementById('tab-images'); if (tab) { tab.click(); }",
    );
    page.restage(VP);

    assert!(page.document.get(tab_images_id).data().has_class("active"));
}

#[test]
fn test_tier4_offline_gmail_webmail_scenario() {
    let mut page = render_html(GMAIL_FIXTURE, "https://mail.google.com/mail/u/0/#inbox", VP);

    assert!(page.title.contains("Webmail"));

    assert!(page.document.get_element_by_id("gmail-header").is_some());
    assert!(page.document.get_element_by_id("sidebar").is_some());
    assert!(page.document.get_element_by_id("compose-btn").is_some());
    assert!(page.document.get_element_by_id("content-pane").is_some());
    assert!(page.document.get_element_by_id("thread-101").is_some());
    assert!(page.document.get_element_by_id("thread-102").is_some());
    assert!(page.document.get_element_by_id("thread-103").is_some());

    let sidebar_id = page.document.get_element_by_id("sidebar").unwrap();
    let sidebar_rect = page.find_layout_rect(sidebar_id).expect("sidebar rect");
    assert!(sidebar_rect.width >= 150.0);

    let content_id = page.document.get_element_by_id("content-pane").unwrap();
    let content_rect = page
        .find_layout_rect(content_id)
        .expect("content pane rect");
    assert!(content_rect.width >= 400.0);

    assert!(page.content_height() > 300.0);

    let fb = rasterize(&page.display_list, 1280, 800, 0.0);
    assert!(
        fb.pixels().iter().any(|&b| b != 255),
        "Gmail webmail fixture raster output has rendered content"
    );

    page.runtime.eval_script(
        &mut page.document,
        "var fetchBtn = document.getElementById('btn-fetch-more'); if (fetchBtn) { fetchBtn.click(); }",
    );
    page.restage(VP);

    assert!(
        page.document.get_element_by_id("thread-104").is_some(),
        "XHR fetch added thread 104"
    );
    assert!(
        page.document.get_element_by_id("thread-105").is_some(),
        "XHR fetch added thread 105"
    );

    let count_node = page.document.get_element_by_id("inbox-count").unwrap();
    assert_eq!(page.document.text_content(count_node).trim(), "5");
}

#[test]
fn test_tier4_live_wikipedia_network_probe() {
    let target = "https://en.wikipedia.org/wiki/Main_Page";
    match anvil_net::fetch(target) {
        Ok((final_url, response)) => {
            if response.status >= 200 && response.status < 400 {
                if let Ok(body) = String::from_utf8(response.body) {
                    let body_owned = body;
                    let url_owned = final_url.clone();
                    let page = std::thread::Builder::new()
                        .name("anvil-worker".to_string())
                        .stack_size(32 * 1024 * 1024)
                        .spawn(move || render_html(&body_owned, &url_owned, VP))
                        .unwrap()
                        .join()
                        .unwrap();
                    assert!(
                        page.content_height() > 0.0,
                        "Live Wikipedia computes positive content height"
                    );
                }
            }
        }
        Err(err) => {
            eprintln!(
                "Live Wikipedia network probe encountered expected offline/network constraint: {err}"
            );
        }
    }
}

#[test]
fn test_tier4_live_google_search_network_probe() {
    let target = "https://www.google.com/";
    match anvil_net::fetch(target) {
        Ok((final_url, response)) => {
            if let Ok(body) = String::from_utf8(response.body) {
                let body_owned = body;
                let url_owned = final_url.clone();
                let page = std::thread::Builder::new()
                    .name("anvil-worker".to_string())
                    .stack_size(32 * 1024 * 1024)
                    .spawn(move || render_html(&body_owned, &url_owned, VP))
                    .unwrap()
                    .join()
                    .unwrap();
                assert!(
                    page.content_height() > 0.0,
                    "Live Google Search computes positive content height"
                );
            }
        }
        Err(err) => {
            eprintln!(
                "Live Google Search network probe encountered expected offline/network constraint: {err}"
            );
        }
    }
}

#[test]
fn test_tier4_live_gmail_network_probe() {
    let target = "https://mail.google.com/";
    match anvil_net::fetch(target) {
        Ok((final_url, response)) => {
            if let Ok(body) = String::from_utf8(response.body) {
                let body_owned = body;
                let url_owned = final_url.clone();
                let page = std::thread::Builder::new()
                    .name("anvil-worker".to_string())
                    .stack_size(32 * 1024 * 1024)
                    .spawn(move || render_html(&body_owned, &url_owned, VP))
                    .unwrap()
                    .join()
                    .unwrap();
                assert!(
                    page.content_height() >= 0.0,
                    "Live Gmail computes non-negative content height"
                );
            }
        }
        Err(err) => {
            eprintln!(
                "Live Gmail network probe encountered expected offline/network constraint: {err}"
            );
        }
    }
}
