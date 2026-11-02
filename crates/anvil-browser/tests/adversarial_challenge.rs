// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Empirical Adversarial Challenge & Stress-Test Suite for Cinder JS, Prism CSS, and Target Sites.
//!
//! Subsystems Challenged:
//! 1. Cinder JS & WHATWG XMLHttpRequest:
//!    - State transitions (UNSENT -> OPENED -> HEADERS_RECEIVED -> LOADING -> DONE)
//!    - Abort lifecycle (before send, after send, repetitive aborts)
//!    - Malformed/invalid URLs
//!    - Request & response header manipulation & case sensitivity
//!    - Concurrent XHR requests (named identifiers vs array access)
//! 2. Async/Await & Promises:
//!    - Nested async functions and execution order
//!    - Async arrow functions
//!    - Promise chaining and multi-level unwrapping
//!    - Await on primitive / non-promise values
//!    - Promise rejection handling and try/catch recovery
//!    - Semicolon sensitivity inside callback return expressions
//! 3. DOM Manipulation:
//!    - Deeply nested element hierarchies (60+ levels)
//!    - ClassList operations (single token vs multiple tokens)
//!    - Extended style properties (flex, grid, box-sizing, custom colors)
//! 4. Prism CSS & Layout:
//!    - Deeply nested Selectors Level 4 (:is, :where, :has, :not)
//!    - Selectors exceeding MAX_SELECTOR_NESTING limit (64)
//!    - Deeply chained CSS variables (10+ steps) and cyclic variables with/without fallback
//!    - Extreme viewport boundaries (0x0, ultra-tall, ultra-wide, negative, NaN/Inf)
//! 5. Target Sites & Fixture Resiliency:
//!    - Wikipedia, Google Search, Gmail offline DOM & layout validation

use anvil_browser::{Page, Viewport, render_html};
use anvil_paint::DisplayItem;
use anvil_raster::{Color, rasterize};

const STD_VP: Viewport = Viewport {
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
// 1. CINDER JS & WHATWG XMLHTTPREQUEST STRESS TESTS
// =========================================================================

#[test]
fn challenge_xhr_state_transitions_and_ready_states() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>XHR State Transition Challenge</title></head>
<body>
  <div id="states-log"></div>
  <div id="final-status">0</div>
  <script>
    var log = [];
    var xhr = new XMLHttpRequest();
    log.push("init:" + xhr.readyState);

    xhr.onreadystatechange = function() {
      log.push("state:" + xhr.readyState);
    };

    xhr.open("GET", "https://example.com/api/data");
    log.push("opened:" + xhr.readyState);

    xhr.send();
    log.push("after-send:" + xhr.readyState);

    var logDiv = document.getElementById("states-log");
    if (logDiv) {
      logDiv.textContent = log.join(",");
    }
    var statusDiv = document.getElementById("final-status");
    if (statusDiv) {
      statusDiv.textContent = String(xhr.status);
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_states", STD_VP);

    let log_node = page
        .document
        .get_element_by_id("states-log")
        .expect("states-log element exists");
    let logged_text = page.document.text_content(log_node);

    assert!(
        logged_text.contains("init:0"),
        "XHR initial readyState must be UNSENT (0), got: {}",
        logged_text
    );
    assert!(
        logged_text.contains("opened:1"),
        "XHR readyState after open() must be OPENED (1), got: {}",
        logged_text
    );
    assert!(
        logged_text.contains("after-send:4") || logged_text.contains("state:4"),
        "XHR readyState after send() must transition to DONE (4), got: {}",
        logged_text
    );
}

#[test]
fn challenge_xhr_abort_lifecycle() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>XHR Abort Challenge</title></head>
<body>
  <div id="abort-before-send"></div>
  <div id="abort-after-send"></div>
  <div id="repetitive-abort"></div>
  <script>
    // Scenario A: Abort when OPENED before send()
    var xhrA = new XMLHttpRequest();
    xhrA.open("GET", "https://example.com/a");
    xhrA.abort();
    var divA = document.getElementById("abort-before-send");
    if (divA) {
      divA.textContent = "readyStateA:" + xhrA.readyState + ",statusA:" + xhrA.status;
    }

    // Scenario B: Abort after send()
    var xhrB = new XMLHttpRequest();
    xhrB.open("GET", "https://example.com/b");
    xhrB.send();
    xhrB.abort();
    var divB = document.getElementById("abort-after-send");
    if (divB) {
      divB.textContent = "readyStateB:" + xhrB.readyState + ",statusB:" + xhrB.status;
    }

    // Scenario C: Repetitive aborts must be no-op and never crash
    var xhrC = new XMLHttpRequest();
    xhrC.abort();
    xhrC.open("GET", "https://example.com/c");
    xhrC.abort();
    xhrC.abort();
    xhrC.abort();
    var divC = document.getElementById("repetitive-abort");
    if (divC) {
      divC.textContent = "readyStateC:" + xhrC.readyState;
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_abort", STD_VP);

    let div_a = page
        .document
        .get_element_by_id("abort-before-send")
        .unwrap();
    let text_a = page.document.text_content(div_a);
    assert!(text_a.contains("readyStateA:0") && text_a.contains("statusA:0"));

    let div_b = page.document.get_element_by_id("abort-after-send").unwrap();
    let text_b = page.document.text_content(div_b);
    assert!(text_b.contains("readyStateB:0") && text_b.contains("statusB:0"));

    let div_c = page.document.get_element_by_id("repetitive-abort").unwrap();
    let text_c = page.document.text_content(div_c);
    assert!(text_c.contains("readyStateC:0"));
}

#[test]
fn challenge_xhr_invalid_urls_and_error_resilience() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>XHR Invalid URL Challenge</title></head>
<body>
  <div id="resilience-result">Pending</div>
  <script>
    var passed = 0;

    var urls = [
      "",
      "http://[:::1]:99999/malformed",
      "://bad-scheme",
      "javascript:void(0)",
      "data:text/plain;invalid,,data"
    ];

    for (var i = 0; i < urls.length; i++) {
      try {
        var xhr = new XMLHttpRequest();
        xhr.open("GET", urls[i]);
        xhr.send();
        passed++;
      } catch (e) {
        passed++;
      }
    }

    var res = document.getElementById("resilience-result");
    if (res) {
      res.textContent = "Passed:" + passed + "/" + urls.length;
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_invalid", STD_VP);
    let res_node = page
        .document
        .get_element_by_id("resilience-result")
        .unwrap();
    assert_eq!(page.document.text_content(res_node).trim(), "Passed:5/5");
}

#[test]
fn challenge_xhr_header_manipulation() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>XHR Header Manipulation</title></head>
<body>
  <div id="header-check"></div>
  <div id="invalid-state-error">No</div>
  <script>
    var xhr = new XMLHttpRequest();

    try {
      xhr.setRequestHeader("X-Premature", "ShouldFail");
    } catch (e) {
      var errDiv = document.getElementById("invalid-state-error");
      if (errDiv) {
        errDiv.textContent = "CaughtInvalidState";
      }
    }

    xhr.open("POST", "https://example.com/submit");
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.setRequestHeader("X-Custom-ID", "12345");
    xhr.setRequestHeader("x-custom-id", "67890");
    xhr.send('{"action":"test"}');

    var ct = xhr.getResponseHeader("content-type");
    var allHeaders = xhr.getAllResponseHeaders();

    var div = document.getElementById("header-check");
    if (div) {
      div.textContent = "ct:" + (ct ? "ok" : "none") + ",all:" + (allHeaders.length > 0 ? "ok" : "empty");
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_headers", STD_VP);

    let div_node = page.document.get_element_by_id("header-check").unwrap();
    let text = page.document.text_content(div_node);
    assert!(text.contains("ct:ok") && text.contains("all:ok"));
}

#[test]
fn challenge_xhr_concurrent_named_requests() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Concurrent Named XHR Challenge</title></head>
<body>
  <div id="concurrent-count">0</div>
  <script>
    var count = 0;
    var x1 = new XMLHttpRequest();
    x1.open("GET", "https://example.com/item/1");
    var x2 = new XMLHttpRequest();
    x2.open("GET", "https://example.com/item/2");
    var x3 = new XMLHttpRequest();
    x3.open("GET", "https://example.com/item/3");
    x1.send();
    x2.send();
    x3.send();

    if (x1.readyState === 4) { count++; }
    if (x2.readyState === 4) { count++; }
    if (x3.readyState === 4) { count++; }

    var div = document.getElementById("concurrent-count");
    if (div) {
      div.textContent = String(count);
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_concurrent_named", STD_VP);
    let div_node = page.document.get_element_by_id("concurrent-count").unwrap();
    assert_eq!(page.document.text_content(div_node).trim(), "3");
}

/// EMPIRICAL BUG FINDING:
/// When an XMLHttpRequest instance is stored in an array or accessed via property index
/// (e.g. `xhrs[0].send()`), `crates/anvil-script/src/lib.rs` line 10016 checks `if is_identifier(obj_expr)`,
/// which returns `false` for `"xhrs[0]"`. Consequently, the mutated readyState=4 is never written
/// back into the array, leaving the XHR stuck at readyState=1.
#[test]
fn demonstrate_bug_xhr_methods_fail_to_update_state_when_accessed_via_array_or_member_expr() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>XHR Array Member Challenge</title></head>
<body>
  <div id="array-xhr-status">0</div>
  <script>
    var xhrs = [new XMLHttpRequest()];
    xhrs[0].open("GET", "https://example.com/array_item");
    xhrs[0].send();

    var div = document.getElementById("array-xhr-status");
    if (div) {
      div.textContent = "readyState:" + xhrs[0].readyState;
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/xhr_array_member", STD_VP);
    let div_node = page.document.get_element_by_id("array-xhr-status").unwrap();
    let text = page.document.text_content(div_node);
    // Verify remediation: readyState is 4 because state mutation was correctly written back to array element.
    assert_eq!(
        text.trim(),
        "readyState:4",
        "State mutation write-back succeeded for array-indexed XHR"
    );
}

// =========================================================================
// 2. ASYNC / AWAIT & PROMISE RESOLUTION STRESS TESTS
// =========================================================================

#[test]
fn challenge_async_nested_functions_and_await_primitives() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Async Await Challenge</title></head>
<body>
  <div id="async-result">Pending</div>
  <script>
    async function addAsync(a, b) {
      return a + b;
    }

    async function computeNested(base) {
      var step1 = await addAsync(base, 10);
      var step2 = await addAsync(step1, 20);
      var primitiveNumber = await 5;
      var primitiveString = await " units";
      var primitiveNull = await null;
      return String(step2 + primitiveNumber) + primitiveString;
    }

    computeNested(100).then(function(res) {
      var div = document.getElementById("async-result");
      if (div) {
        div.textContent = res;
      }
    });
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/async_nested", STD_VP);
    let res_node = page.document.get_element_by_id("async-result").unwrap();
    assert_eq!(page.document.text_content(res_node).trim(), "135 units");
}

#[test]
fn challenge_async_arrow_and_promise_chaining_without_semicolons() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Async Arrow & Promise Chaining</title></head>
<body>
  <div id="chain-result">0</div>
  <script>
    var doubleVal = async (x) => x * 2;
    var addFive = async (x) => x + 5;

    Promise.resolve(10)
      .then(function(v) { return doubleVal(v) })
      .then(function(v) { return addFive(v) })
      .then(function(v) { return Promise.resolve(Promise.resolve(v * 3)) })
      .then(function(finalVal) {
        var div = document.getElementById("chain-result");
        if (div) {
          div.textContent = String(finalVal);
        }
      });
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/async_chain", STD_VP);
    let res_node = page.document.get_element_by_id("chain-result").unwrap();
    assert_eq!(page.document.text_content(res_node).trim(), "75");
}

/// EMPIRICAL BUG FINDING:
/// In `crates/anvil-script/src/lib.rs` line 11467, `eval_callback_return` strips `"return "`
/// but does not strip trailing semicolons: `trimmed.strip_prefix("return ")`.
/// When a handler body contains `return expr;`, `eval_expr` receives `"expr;"`.
/// Because `eval_expr` requires valid expressions without trailing semicolons, it fails
/// parsing and returns `Value::Null`.
#[test]
fn demonstrate_bug_semicolon_in_return_expression_causes_null_promise_result() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Semicolon in Return Expression</title></head>
<body>
  <div id="semicolon-result">Initial</div>
  <script>
    Promise.resolve(42)
      .then(function(v) { return v + 8; }) // Notice trailing semicolon inside single statement
      .then(function(val) {
        var div = document.getElementById("semicolon-result");
        if (div) {
          div.textContent = String(val);
        }
      });
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/semicolon_bug", STD_VP);
    let res_node = page.document.get_element_by_id("semicolon-result").unwrap();

    // Verify remediation: val is 50 because trailing semicolon was stripped before evaluating return expression.
    assert_eq!(
        page.document.text_content(res_node).trim(),
        "50",
        "Single-expression callback return evaluates correctly with trailing semicolon"
    );
}

#[test]
fn challenge_promise_rejection_and_catch_recovery() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Promise Rejection Challenge</title></head>
<body>
  <div id="catch-status">Uncaught</div>
  <script>
    async function failingTask() {
      throw new Error("DeliberateAsyncFault");
    }

    async function resilientRunner() {
      try {
        await failingTask();
        return "FailedToCatch";
      } catch (err) {
        return "CaughtSuccess: " + (err.message || String(err));
      }
    }

    resilientRunner().then(function(status) {
      var div = document.getElementById("catch-status");
      if (div) {
        div.textContent = status;
      }
    });
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/promise_rejection", STD_VP);
    let status_node = page.document.get_element_by_id("catch-status").unwrap();
    let text = page.document.text_content(status_node);
    assert!(text.contains("CaughtSuccess: DeliberateAsyncFault"));
}

// =========================================================================
// 3. DOM MANIPULATION & STYLES STRESS TESTS
// =========================================================================

#[test]
fn challenge_deeply_nested_element_creation() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Deep DOM Nesting Challenge</title></head>
<body>
  <div id="root-container"></div>
  <script>
    var root = document.getElementById("root-container");
    var current = root;
    var DEPTH = 60;

    for (var i = 0; i < DEPTH; i++) {
      var child = document.createElement("div");
      child.id = "level-" + i;
      child.className = "layer-node";
      current.appendChild(child);
      current = child;
    }

    var leaf = document.createElement("span");
    leaf.id = "leaf-node";
    leaf.textContent = "Reached Level " + DEPTH;
    current.appendChild(leaf);
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/deep_dom", STD_VP);
    assert!(page.document.get_element_by_id("level-0").is_some());
    assert!(page.document.get_element_by_id("level-59").is_some());
    let leaf = page
        .document
        .get_element_by_id("leaf-node")
        .expect("leaf-node exists");
    assert_eq!(page.document.text_content(leaf).trim(), "Reached Level 60");
}

#[test]
fn challenge_classlist_single_token_and_extended_style_properties() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>ClassList & Extended Style Challenge</title></head>
<body>
  <div id="styled-box" class="initial-class"><span>Box Content</span></div>
  <script>
    var box = document.getElementById("styled-box");
    if (box) {
      // Individual token additions work properly
      box.classList.add("extra-a");
      box.classList.add("extra-b");
      box.classList.remove("initial-class");
      box.classList.toggle("toggled-on");

      box.style.boxSizing = "border-box";
      box.style.display = "flex";
      box.style.flexGrow = "1";
      box.style.flexShrink = "0";
      box.style.flexBasis = "200px";
      box.style.color = "#0088ff";
      box.style.width = "300px";
      box.style.height = "80px";
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/classlist_styles", STD_VP);
    let box_id = page.document.get_element_by_id("styled-box").unwrap();

    let node_data = page.document.get(box_id).data();
    assert!(node_data.has_class("extra-a"));
    assert!(node_data.has_class("extra-b"));
    assert!(!node_data.has_class("initial-class"));
    assert!(node_data.has_class("toggled-on"));

    let box_color = find_text_color(&page, "Box Content").expect("Box Content rendered");
    assert_eq!(box_color, Color::parse("#0088ff").unwrap());

    let rect = page.find_layout_rect(box_id).expect("layout rect exists");
    assert!(rect.width >= 200.0);
    assert!(rect.height >= 50.0);
}

/// EMPIRICAL BUG FINDING:
/// In `crates/anvil-layout/src/lib.rs` line 3959 (`layout_flex`), child text nodes directly inside
/// a flex container (anonymous text nodes) are passed to `layout_node`, which delegates to
/// `layout_block`. Because the text node has no children, `layout_block` generates an empty box
/// without text runs, making bare text in flex containers completely invisible.
#[test]
fn demonstrate_bug_flex_layout_drops_anonymous_text_nodes() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Flex Anonymous Text Challenge</title></head>
<body>
  <div style="display: flex; width: 200px; height: 50px;">Direct Flex Text</div>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/flex_text_bug", STD_VP);
    let found = find_text_color(&page, "Direct Flex Text");

    // Verify remediation: anonymous text node inside flex container is rendered in display list.
    assert!(
        found.is_some(),
        "Direct Flex Text was rendered as an anonymous flex item"
    );
}

/// EMPIRICAL BUG FINDING:
/// In `crates/anvil-script/src/lib.rs` lines 9046-9055, `classList.add` and `classList.remove`
/// only evaluate `args.first()`, ignoring variadic tokens (`add("a", "b")`).
/// In contrast, `crates/anvil-script/src/engine_host.rs` loops over all arguments for Cinder VM.
#[test]
fn demonstrate_bug_classlist_add_drops_multiple_token_arguments() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Variadic ClassList Challenge</title></head>
<body>
  <div id="token-box">Content</div>
  <script>
    var box = document.getElementById("token-box");
    if (box) {
      box.classList.add("tok-first", "tok-second", "tok-third");
    }
  </script>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/variadic_classlist", STD_VP);
    let box_id = page.document.get_element_by_id("token-box").unwrap();
    let data = page.document.get(box_id).data();

    // Verify remediation: all tokens are added by variadic classList.add!
    assert!(data.has_class("tok-first"), "First token was added");
    assert!(
        data.has_class("tok-second"),
        "tok-second was added by variadic classList.add"
    );
    assert!(
        data.has_class("tok-third"),
        "tok-third was added by variadic classList.add"
    );
}

// =========================================================================
// 4. PRISM CSS SELECTORS LEVEL 4 & VIEWPORT BOUNDARY EXTREMES
// =========================================================================

#[test]
fn challenge_prism_selectors_level_4_nesting() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    :is(main, section):has(article:not(.archived)) :where(.badge) {
      color: #00aa22;
    }

    article:not(:where(.hidden, .draft)) > p:is(:first-child, .lead) {
      color: #7700cc;
    }
  </style>
</head>
<body>
  <main>
    <article class="published">
      <p class="lead">Lead Paragraph</p>
      <span class="badge">Active Badge</span>
    </article>
    <article class="archived">
      <span class="badge">Archived Badge</span>
    </article>
  </main>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/prism_selectors", STD_VP);

    let active_color = find_text_color(&page, "Active Badge").expect("Active Badge drawn");
    assert_eq!(active_color, Color::parse("#00aa22").unwrap());

    let lead_color = find_text_color(&page, "Lead Paragraph").expect("Lead Paragraph drawn");
    assert_eq!(lead_color, Color::parse("#7700cc").unwrap());
}

#[test]
fn challenge_prism_selector_nesting_limit_exceeded() {
    // Generate a selector with 70 nested :is() calls (exceeding MAX_SELECTOR_NESTING = 64)
    let mut deep_selector = String::from(".target");
    for _ in 0..70 {
        deep_selector = format!(":is({})", deep_selector);
    }
    let html = format!(
        r##"<!DOCTYPE html>
<html>
<head>
  <style>
    {} {{ color: #ff0000; }}
    .target {{ color: #008800; }}
  </style>
</head>
<body>
  <p class="target">Nesting Guard Test</p>
</body>
</html>"##,
        deep_selector
    );

    let page = render_html(&html, "https://example.com/selector_limit", STD_VP);
    let target_color = find_text_color(&page, "Nesting Guard Test").expect("Target text drawn");

    // The overly nested selector rule must be cleanly dropped per MAX_SELECTOR_NESTING,
    // so the subsequent rule (.target { color: #008800; }) applies without a stack overflow crash.
    assert_eq!(target_color, Color::parse("#008800").unwrap());
}

#[test]
fn challenge_css_variables_deep_chains_and_cycles() {
    let html = r##"<!DOCTYPE html>
<html>
<head>
  <style>
    :root {
      --step-1: #11aa44;
      --step-2: var(--step-1);
      --step-3: var(--step-2);
      --step-4: var(--step-3);
      --step-5: var(--step-4);
      --step-6: var(--step-5);
      --step-7: var(--step-6);
      --step-8: var(--step-7);
      --step-9: var(--step-8);
      --step-10: var(--step-9);

      --c-alpha: var(--c-beta);
      --c-beta: var(--c-gamma);
      --c-gamma: var(--c-alpha);
      --cyclic-with-fb: var(--c-alpha, #ff5500);

      --self: var(--self, #990099);
    }

    #chained-target { color: var(--step-10); }
    #cyclic-target { color: var(--cyclic-with-fb); }
    #self-target { color: var(--self); }
  </style>
</head>
<body>
  <p id="chained-target">Chained Result</p>
  <p id="cyclic-target">Cyclic Result</p>
  <p id="self-target">Self Result</p>
</body>
</html>"##;

    let page = render_html(html, "https://example.com/css_var_stress", STD_VP);

    let chained_color = find_text_color(&page, "Chained Result").expect("Chained Result drawn");
    assert_eq!(chained_color, Color::parse("#11aa44").unwrap());

    let cyclic_color = find_text_color(&page, "Cyclic Result").expect("Cyclic Result drawn");
    assert_eq!(cyclic_color, Color::parse("#ff5500").unwrap());

    let self_color = find_text_color(&page, "Self Result").expect("Self Result drawn");
    assert_eq!(self_color, Color::parse("#990099").unwrap());
}

#[test]
fn challenge_extreme_viewport_boundaries_and_resilience() {
    let html = r##"<!DOCTYPE html>
<html>
<head><title>Extreme Viewport Resiliency</title></head>
<body>
  <header style="width: 100%; height: 60px; background: #333; color: #fff;">Header</header>
  <main style="display: flex; width: 100%;">
    <div style="flex: 1; height: 200px; background: #eee;">Sidebar</div>
    <div style="flex: 3; height: 200px; background: #ddd;">Content</div>
  </main>
</body>
</html>"##;

    // Viewport 1: 0x0
    let vp_zero = Viewport {
        width: 0.0,
        height: 0.0,
    };
    let page_zero = render_html(html, "https://example.com/zero", vp_zero);
    assert!(page_zero.content_height() >= 0.0);
    let fb_zero = rasterize(&page_zero.display_list, 1, 1, 0.0);
    assert_eq!(fb_zero.width(), 1);

    // Viewport 2: Ultra tall 1x100,000
    let vp_tall = Viewport {
        width: 1.0,
        height: 100_000.0,
    };
    let page_tall = render_html(html, "https://example.com/tall", vp_tall);
    assert!(page_tall.content_height() >= 0.0);

    // Viewport 3: Ultra wide 100,000x1
    let vp_wide = Viewport {
        width: 100_000.0,
        height: 1.0,
    };
    let page_wide = render_html(html, "https://example.com/wide", vp_wide);
    assert!(page_wide.content_height() >= 0.0);

    // Viewport 4: Sub-pixel fractional
    let vp_frac = Viewport {
        width: 0.001,
        height: 0.001,
    };
    let page_frac = render_html(html, "https://example.com/frac", vp_frac);
    assert!(page_frac.content_height() >= 0.0);

    // Viewport 5: Negative dimensions must not panic
    let vp_neg = Viewport {
        width: -100.0,
        height: -200.0,
    };
    let page_neg = render_html(html, "https://example.com/neg", vp_neg);
    assert!(page_neg.content_height() >= 0.0);

    // Viewport 6: NaN dimensions must not crash
    let vp_nan = Viewport {
        width: f32::NAN,
        height: f32::NAN,
    };
    let page_nan = render_html(html, "https://example.com/nan", vp_nan);
    assert!(!page_nan.title.is_empty() || page_nan.content_height() >= 0.0);
}

// =========================================================================
// 5. TARGET SITES OFFLINE FIXTURES & RENDERING VALIDATION
// =========================================================================

#[test]
fn challenge_target_sites_offline_fixtures_validation() {
    // 1. Wikipedia Vector 2022 Fixture
    let wiki_page = render_html(
        WIKIPEDIA_FIXTURE,
        "https://en.wikipedia.org/wiki/Rust_(programming_language)",
        STD_VP,
    );
    assert!(
        wiki_page.title.contains("Wikipedia"),
        "Wikipedia title present"
    );
    assert!(
        wiki_page.document.get_element_by_id("content").is_some(),
        "Wikipedia #content present"
    );
    assert!(
        wiki_page
            .document
            .get_element_by_id("firstHeading")
            .is_some(),
        "Wikipedia #firstHeading present"
    );
    assert!(
        wiki_page.content_height() > 500.0,
        "Wikipedia layout has positive content height"
    );
    let wiki_fb = rasterize(&wiki_page.display_list, 1280, 800, 0.0);
    assert_eq!(wiki_fb.width(), 1280);
    assert_eq!(wiki_fb.height(), 800);

    // 2. Google Search Fixture
    let google_page = render_html(
        GOOGLE_FIXTURE,
        "https://www.google.com/search?q=rust+lang",
        STD_VP,
    );
    assert!(google_page.title.contains("Google"), "Google title present");
    assert!(
        google_page
            .document
            .get_element_by_id("searchform")
            .is_some(),
        "Google #searchform present"
    );
    assert!(
        google_page
            .document
            .get_element_by_id("search-tabs")
            .is_some(),
        "Google #search-tabs present"
    );
    assert!(
        google_page.content_height() > 400.0,
        "Google layout has positive content height"
    );
    let google_fb = rasterize(&google_page.display_list, 1280, 800, 0.0);
    assert_eq!(google_fb.width(), 1280);
    assert_eq!(google_fb.height(), 800);

    // 3. Gmail Webmail Fixture
    let gmail_page = render_html(
        GMAIL_FIXTURE,
        "https://mail.google.com/mail/u/0/#inbox",
        STD_VP,
    );
    assert!(gmail_page.title.contains("Inbox"), "Gmail title present");
    assert!(
        gmail_page.document.get_element_by_id("sidebar").is_some(),
        "Gmail #sidebar present"
    );
    assert!(
        gmail_page
            .document
            .get_element_by_id("thread-table")
            .is_some(),
        "Gmail #thread-table present"
    );
    assert!(
        gmail_page.content_height() > 400.0,
        "Gmail layout has positive content height"
    );
    let gmail_fb = rasterize(&gmail_page.display_list, 1280, 800, 0.0);
    assert_eq!(gmail_fb.width(), 1280);
    assert_eq!(gmail_fb.height(), 800);
}
