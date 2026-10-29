// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Parse-speed check for a large style sheet. Ignored by default; run with
//! `cargo test --release --test bench -- --ignored --nocapture`.

use std::fmt::Write;
use std::time::Instant;

use anvil_css::{Options, parse_stylesheet};

fn big_sheet(rules: usize) -> String {
    let mut s = String::new();
    for i in 0..rules {
        match i % 5 {
            0 => writeln!(s, ".c{i} > a:hover, #id{i} .x {{ color: #{:06x}; margin: 0 {}px; }}", i * 97 % 0xffffff, i % 40),
            1 => writeln!(s, "div.item-{i}:nth-child(2n+1) span[data-k='{i}'] {{ width: calc(100% - {}em); display: flex }}", i % 9),
            2 => writeln!(s, "@media (min-width: {}px) {{ .m{i} {{ padding: 1px 2px 3px 4px; background: url(img{i}.png) no-repeat }} }}", 300 + i % 900),
            3 => writeln!(s, ".n{i} {{ color: rgb({} {} {} / 50%); &:focus-visible {{ outline: 2px solid currentColor }} }}", i % 255, (i * 3) % 255, (i * 7) % 255),
            _ => writeln!(s, ":is(h1, h2, h3).t{i}:not(.skip) {{ font: 700 1.25rem/1.4 system-ui, sans-serif; --v{i}: {{ a: b }} }}"),
        }
        .unwrap();
    }
    s
}

#[test]
#[ignore = "timing check; run explicitly"]
fn parse_ten_thousand_rules() {
    let src = big_sheet(10_000);
    let start = Instant::now();
    let sheet = parse_stylesheet(&src, &Options::default()).unwrap();
    let elapsed = start.elapsed();
    assert!(
        sheet.diagnostics.is_empty(),
        "{:?}",
        &sheet.diagnostics[..sheet.diagnostics.len().min(3)]
    );
    let rules = sheet.rule_count();
    eprintln!(
        "parsed {} bytes, {} rules in {:?} ({:.1} MB/s)",
        src.len(),
        rules,
        elapsed,
        src.len() as f64 / elapsed.as_secs_f64() / 1e6
    );
    assert!(rules >= 10_000);
}
