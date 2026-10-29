// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Tests for linking and renaming.

use crate::anvil_js::corejs::check::{HOST, PLAIN};
use crate::anvil_js::link::{Module, link, module_key, resolve_spec};
use crate::anvil_js::modules::parse_list;

const H: &str = "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n// Copyright (c) 2026 Buntato\n";

fn m(path: &str, level: u8, body: &str) -> Module {
    Module {
        path: path.to_string(),
        level,
        builtins: false,
        src: format!("{H}{body}"),
    }
}

#[test]
fn keys_and_specs() {
    assert_eq!(module_key("src/core/errors.js"), "core_errors");
    assert_eq!(module_key("tests/unit/a-b.js"), "tests_unit_a_b");
    assert_eq!(
        resolve_spec("src/core/x.js", "./y.js").as_deref(),
        Some("src/core/y.js")
    );
    assert_eq!(
        resolve_spec("tests/unit/t.js", "../../src/core/y.js").as_deref(),
        Some("src/core/y.js")
    );
    assert_eq!(resolve_spec("a.js", "y.js"), None);
    assert_eq!(resolve_spec("a.js", "../../y.js"), None);
}

#[test]
fn renames_top_level_and_keeps_shadowing() {
    let a = m(
        "src/a.js",
        HOST,
        "export const n = 1;\nexport function f(x) { const n = 2; return n + x; }\n",
    );
    let b = m(
        "src/b.js",
        HOST,
        "import { n, f } from \"./a.js\";\nexport const o = { n, g: f };\nconsole.log(f(n), o.n);\n",
    );
    let l = link(&[a, b]).expect("links");
    assert!(l.text.contains("const a$n = 1;"), "{}", l.text);
    assert!(
        l.text
            .contains("function a$f(x) { const n = 2; return n + x; }")
    );
    assert!(l.text.contains("const b$o = { n: a$n, g: a$f };"));
    assert!(l.text.contains("console.log(a$f(a$n), b$o.n);"));
    assert!(!l.text.contains("import"));
    assert!(!l.text.contains("export"));
}

#[test]
fn lines_are_kept() {
    let a = m("src/a.js", HOST, "export const x = 1;\n");
    let b = m(
        "src/b.js",
        HOST,
        "import {\n  x\n} from \"./a.js\";\nconsole.log(x);\n",
    );
    let l = link(&[a, b]).expect("links");
    let (first, path, n) = &l.map[1];
    assert_eq!(path, "src/b.js");
    assert_eq!(*n, 6);
    let line = l.text.lines().nth(*first as usize - 1 + 5).unwrap();
    assert_eq!(line, "console.log(a$x);");
}

#[test]
fn class_names_inside_their_body_follow() {
    let a = m(
        "src/a.js",
        HOST,
        "export class P { static make() { return new P(); } }\nclass Q extends P {}\nconsole.log(new Q() instanceof P);\n",
    );
    let l = link(&[a]).expect("links");
    assert!(
        l.text
            .contains("class a$P { static make() { return new a$P(); } }")
    );
    assert!(l.text.contains("class a$Q extends a$P {}"));
}

#[test]
fn clashing_names_are_bumped() {
    let a = m("src/a.js", HOST, "export const x = 1;\nconst b$y = 2;\n");
    let b = m("src/b.js", HOST, "export const y = 3;\n");
    let l = link(&[a, b]).expect("links");
    assert!(l.text.contains("const b$y$2 = 3;"), "{}", l.text);
}

#[test]
fn import_problems() {
    let a = m("src/a.js", HOST, "export const x = 1;\nconst hidden = 2;\n");
    let b = m("src/b.js", HOST, "import { hidden } from \"./a.js\";\n");
    let p = link(&[a.clone(), b]).unwrap_err();
    assert!(p[0].message.contains("does not export `hidden`"), "{p:?}");
    let c = m("src/c.js", HOST, "import { x } from \"./later.js\";\n");
    let later = m("src/later.js", HOST, "export const x = 1;\n");
    let p = link(&[c, later]).unwrap_err();
    assert!(p[0].message.contains("must come before"), "{p:?}");
    let low = m("src/low.js", PLAIN, "import { x } from \"./a.js\";\n");
    let p = link(&[a.clone(), low]).unwrap_err();
    assert!(p[0].message.contains("can't import"), "{p:?}");
    let late = m(
        "src/late.js",
        HOST,
        "const y = 1;\nimport { x } from \"./a.js\";\n",
    );
    let p = link(&[a, late]).unwrap_err();
    assert!(p[0].message.contains("before everything else"), "{p:?}");
}

#[test]
fn level_problems_carry_positions() {
    let a = m(
        "src/a.js",
        PLAIN,
        "export function f() { return () => 1; }\n",
    );
    let p = link(&[a]).unwrap_err();
    assert_eq!(p[0].name, "LevelError");
    assert_eq!((p[0].line, p[0].col), (3, 30));
    assert!(p[0].to_string().starts_with("src/a.js:3:30: LevelError: "));
}

#[test]
fn builtins_patch_needs_the_marker() {
    let src = "export const a = Math.max(1, 2) + Math.floor(1.5);\n";
    let p = link(&[m("src/a.js", PLAIN, src)]).unwrap_err();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].name, "LevelError");
    assert!(
        p[0].message
            .contains("`Math.max` is one of the extra built-ins")
    );
    let mut marked = m("src/a.js", PLAIN, src);
    marked.builtins = true;
    assert!(link(&[marked]).is_ok());
    let l = parse_list("src/a.js needs=plain builtins=yes\nsrc/b.js\n").unwrap();
    assert!(l[0].builtins && !l[1].builtins);
}

#[test]
fn output_is_deterministic() {
    let mk = || {
        vec![
            m(
                "src/a.js",
                HOST,
                "export const z = 1;\nexport const a = 2;\nexport function q() { return z + a; }\n",
            ),
            m(
                "src/b.js",
                HOST,
                "import { q, a, z } from \"./a.js\";\nconsole.log(q(), a, z);\n",
            ),
        ]
    };
    let x = link(&mk()).unwrap().text;
    let y = link(&mk()).unwrap().text;
    assert_eq!(x, y);
}

#[test]
fn module_list() {
    let l = parse_list("# c\nsrc/a.js needs=plain\n\nsrc/b.js # note\n").unwrap();
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].level, PLAIN);
    assert_eq!(l[1].level, HOST);
    assert!(parse_list("src/a.js needs=fancy\n").is_err());
    assert!(parse_list("src/a.js\nsrc/a.js\n").is_err());
    assert!(parse_list("../a.js\n").is_err());
}
