// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Every conformance probe must parse, and must carry a goal line and at
//! least one expected-output line.

#![forbid(unsafe_code)]

use cinder::{ParseOptions, parse_module, parse_script};
use std::fs;
use std::path::Path;

#[test]
fn conformance_probes_parse() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("conformance");
    let mut count = 0;
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("conformance dir")
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "js" && ext != "mjs" {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read probe");
        let module = src.contains("// goal: module");
        assert!(
            module || src.contains("// goal: script"),
            "{}: missing goal line",
            path.display()
        );
        assert_eq!(
            module,
            ext == "mjs",
            "{}: goal and extension disagree",
            path.display()
        );
        assert!(
            src.contains("// expect: "),
            "{}: missing expect line",
            path.display()
        );
        let result = if module {
            parse_module(&src, &ParseOptions::default())
        } else {
            parse_script(&src, &ParseOptions::default())
        };
        if let Err(e) = result {
            panic!("{}: {e}", path.display());
        }
        count += 1;
    }
    assert!(count >= 60, "only {count} probes");
}

#[test]
fn every_manifest_module_has_spdx_header_and_non_empty_source() {
    let mods = cinder::load_embedded_modules().expect("must load modules");
    for m in &mods {
        let src = cinder::embedded_module_source(&m.path).expect("embedded source");
        let mut lines = src.lines();
        assert_eq!(
            lines.next(),
            Some(
                "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial"
            ),
            "missing SPDX line in {}",
            m.path
        );
        assert_eq!(
            lines.next(),
            Some("// Copyright (c) 2026 Buntato"),
            "missing copyright line in {}",
            m.path
        );
    }
}
