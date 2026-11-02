// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Grid row tracks, line and span placement, and `ch` widths, seen through
//! the page the browser renders.

use anvil_browser::{Page, Viewport, render_html};
use anvil_layout::Rect;

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
};

fn rect(page: &Page, id: &str) -> Rect {
    let node = page.document.get_element_by_id(id).expect(id);
    page.find_layout_rect(node).expect("laid out")
}

#[test]
fn grid_rows_spans_and_auto_rows_are_laid_out() {
    let page = render_html(
        r#"<html><head><style>
  body { margin: 0; }
  #g { display: grid; width: 200px; grid-template-columns: 100px 100px;
       grid-template-rows: 30px minmax(50px, auto); grid-auto-rows: 20px; }
  #wide { grid-column: 1 / span 2; }
  #placed { grid-row: 2; grid-column: 2; }
</style></head><body><div id="g">
  <div id="wide"></div><div id="placed"></div><div id="flow"></div><div id="extra"></div>
</div></body></html>"#,
        "https://example.com/",
        VP,
    );
    let wide = rect(&page, "wide");
    assert_eq!(
        (wide.x, wide.y, wide.width, wide.height),
        (0.0, 0.0, 200.0, 30.0)
    );
    let placed = rect(&page, "placed");
    assert_eq!(
        (placed.x, placed.y, placed.width, placed.height),
        (100.0, 30.0, 100.0, 50.0)
    );
    let flow = rect(&page, "flow");
    assert_eq!((flow.x, flow.y, flow.height), (0.0, 30.0, 50.0));
    let extra = rect(&page, "extra");
    assert_eq!((extra.y, extra.height), (80.0, 20.0));
}

#[test]
fn ch_widths_scale_with_the_count() {
    let page = render_html(
        r#"<html><head><style>
  body { margin: 0; font-size: 16px; }
  #ten { width: 10ch; height: 10px; }
  #twenty { width: 20ch; height: 10px; }
</style></head><body><div id="ten"></div><div id="twenty"></div></body></html>"#,
        "https://example.com/",
        VP,
    );
    let ten = rect(&page, "ten").width;
    let twenty = rect(&page, "twenty").width;
    assert!(ten > 0.0 && ten < 400.0, "10ch resolved to {ten}");
    assert!(
        (twenty - 2.0 * ten).abs() < 0.01,
        "20ch is {twenty}, 10ch is {ten}"
    );
}
