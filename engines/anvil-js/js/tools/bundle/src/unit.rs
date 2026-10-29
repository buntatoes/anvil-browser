// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Running Core JS tests on the stand-in host.
//!
//! A test is a module. Its imports, and theirs, are linked in the order of
//! MODULES.txt with the test last, and the flat result runs on the
//! stand-in. Expectations come from whole-line comments in the test:
//!
//! * `// expect: TEXT`, one per output line, in order; or a sibling file
//!   with the same name and `.expected` in place of `.js`, holding the
//!   exact output (when it exists, `// expect:` lines are not allowed);
//! * `// expect-error: Name`: the run must end with `Uncaught Name` on
//!   stderr and a nonzero exit;
//! * `// needs: LEVEL`: the test's own level (default `host`).
//!
//! Probes under `tests/core` add two more forms. A probe in `run/` exports
//! `main()`, which returns an array; each item is printed as one line.
//! There `// needs: LEVEL` is required: the probe must lint clean at that
//! level and, unless it is `plain`, fail the level check at the level
//! before. In `reject/`, each
//! `// expect-lint: TEXT` must appear in one of the problems the lint
//! reports, and the probe is not run.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::anvil_js::corejs::ast::StmtKind;
use crate::anvil_js::corejs::check::{LEVEL_ALL, Level, PLAIN, level_name, parse_level};
use crate::anvil_js::corejs::parser::{DEFAULT_MAX_DEPTH, parse};
use crate::anvil_js::link::{Module, Problem, link, resolve_spec, source_line};
use crate::anvil_js::modules::ModuleList;

/// What a test expects.
#[derive(Debug, Default)]
pub struct Expect {
    /// Output lines, when given.
    pub out: Option<Vec<String>>,
    /// Error name, when the test must end with an uncaught error.
    pub error: Option<String>,
    /// Declared level.
    pub level: Option<Level>,
    /// `expect-lint` fragments.
    pub lint: Vec<String>,
    /// `// builtins: yes` or `no`.
    pub builtins: Option<bool>,
}

fn directive<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix("//")?.trim_start();
    let rest = rest.strip_prefix(name)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// Read the expectations of a test.
pub fn expectations(src: &str, sibling: Option<String>) -> Result<Expect, String> {
    let mut e = Expect::default();
    let mut lines = Vec::new();
    for l in src.lines() {
        if let Some(v) = directive(l, "expect") {
            lines.push(v.to_string());
        } else if let Some(v) = directive(l, "expect-error") {
            e.error = Some(v.trim().to_string());
        } else if let Some(v) = directive(l, "expect-lint") {
            e.lint.push(v.trim().to_string());
        } else if let Some(v) = directive(l, "builtins") {
            e.builtins = match v.trim() {
                "yes" => Some(true),
                "no" => Some(false),
                other => return Err(format!("bad builtins `{other}`")),
            };
        } else if let Some(v) = directive(l, "needs") {
            e.level =
                Some(parse_level(v.trim()).ok_or_else(|| format!("bad level `{}`", v.trim()))?);
        }
    }
    match sibling {
        Some(text) => {
            if !lines.is_empty() {
                return Err("has both `// expect:` lines and an .expected file".into());
            }
            e.out = Some(text.lines().map(str::to_string).collect());
        }
        None => {
            if !lines.is_empty() {
                e.out = Some(lines);
            }
        }
    }
    Ok(e)
}

/// The modules a test needs: its imports, transitively, in list order,
/// then `extra` (modules made on the fly, which the test may import by
/// path), then the test itself.
pub fn modules_for(
    list: &ModuleList,
    test_path: &str,
    src: &str,
    level: Level,
    extra: &[Module],
) -> Result<Vec<Module>, Vec<Problem>> {
    let mut need = vec![false; list.entries.len()];
    let mut stack = vec![(test_path.to_string(), src.to_string())];
    let mut problems = Vec::new();
    while let Some((path, text)) = stack.pop() {
        let po = parse(&text, true, DEFAULT_MAX_DEPTH);
        let Ok(prog) = po.program else { continue };
        for s in &prog.body {
            let StmtKind::Import(_, spec) = &s.kind else {
                continue;
            };
            let Some(target) = resolve_spec(&path, spec) else {
                continue;
            };
            if extra.iter().any(|m| m.path == target) {
                continue;
            }
            match list.entries.iter().position(|e| e.path == target) {
                Some(i) => {
                    if !need[i] {
                        need[i] = true;
                        match list.source(i) {
                            Ok(t) => stack.push((target, t)),
                            Err(m) => problems.push(Problem {
                                file: target,
                                line: 0,
                                col: 0,
                                name: "ModuleError".into(),
                                message: m,
                            }),
                        }
                    }
                }
                None => problems.push(Problem {
                    file: path.clone(),
                    line: s.pos.line,
                    col: s.pos.col,
                    name: "ModuleError".into(),
                    message: format!("`{target}` is not listed in MODULES.txt"),
                }),
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut mods = Vec::new();
    for (i, e) in list.entries.iter().enumerate() {
        if need[i] {
            mods.push(Module {
                path: e.path.clone(),
                level: e.level,
                builtins: e.builtins,
                src: list.source(i).map_err(|m| {
                    vec![Problem {
                        file: e.path.clone(),
                        line: 0,
                        col: 0,
                        name: "ModuleError".into(),
                        message: m,
                    }]
                })?,
            });
        }
    }
    mods.extend(extra.iter().cloned());
    mods.push(Module {
        path: test_path.to_string(),
        level,
        builtins: true,
        src: src.to_string(),
    });
    Ok(mods)
}

/// Collect `.js` files under the given files and directories, sorted.
pub fn collect(paths: &[PathBuf]) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut sub = Vec::new();
            for e in std::fs::read_dir(p)? {
                sub.push(e?.path());
            }
            sub.sort();
            for s in sub {
                if s.is_dir() {
                    out.extend(collect(&[s])?);
                } else if s.extension().is_some_and(|x| x == "js") {
                    out.push(s);
                }
            }
        } else {
            out.push(p.clone());
        }
    }
    Ok(out)
}

/// The path of `p` relative to `root`, with `/` separators.
pub fn rel_path(root: &Path, p: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let full = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    full.strip_prefix(&root)
        .unwrap_or(&full)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Result of one test.
pub enum Outcome {
    /// Passed.
    Pass,
    /// Failed, with an explanation.
    Fail(String),
}

pub fn temp_file(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("anvil-bundle-{}-{n}-{tag}.js", std::process::id()))
}

/// Lint, link and run one test (or probe).
pub fn run_test(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    file: &Path,
    probe: Option<&str>,
) -> Outcome {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("can't read it: {e}")),
    };
    let sibling = file.with_extension("expected");
    let sibling = std::fs::read_to_string(&sibling).ok();
    let exp = match expectations(&src, sibling) {
        Ok(e) => e,
        Err(m) => return Outcome::Fail(m),
    };
    let path = rel_path(root, file);
    let level = exp.level.unwrap_or(LEVEL_ALL);
    let with_driver = |t: Level| {
        modules_for(list, &path, &src, t, &[]).and_then(|mut m| {
            // Unit tests may use every built-in; probes only when marked.
            if let Some(last) = m.last_mut() {
                last.builtins = exp.builtins.unwrap_or(probe.is_none());
            }
            if probe == Some("run") {
                m.push(driver(&path));
            }
            link(&m)
        })
    };
    let linked = with_driver(level);
    if probe == Some("reject") {
        if exp.lint.is_empty() {
            return Outcome::Fail("a reject probe needs `// expect-lint:` lines".into());
        }
        let problems = match linked {
            Ok(_) => return Outcome::Fail("the lint accepted it".into()),
            Err(p) => p,
        };
        for want in &exp.lint {
            if !problems.iter().any(|p| p.message.contains(want.as_str())) {
                let got: Vec<String> = problems.iter().map(|p| format!("  {p}")).collect();
                return Outcome::Fail(format!(
                    "no problem mentions `{want}`; got:\n{}",
                    got.join("\n")
                ));
            }
        }
        return Outcome::Pass;
    }
    let linked = match linked {
        Ok(l) => l,
        Err(p) => {
            let got: Vec<String> = p.iter().map(|p| format!("  {p}")).collect();
            return Outcome::Fail(format!("lint failed:\n{}", got.join("\n")));
        }
    };
    if probe == Some("run") {
        let Some(t) = exp.level else {
            return Outcome::Fail("a run probe needs `// needs:`".into());
        };
        if t > PLAIN {
            let earlier = with_driver(t - 1);
            match earlier {
                Ok(_) => {
                    return Outcome::Fail(format!(
                        "it also lints clean at level {}, so it does not test level {}",
                        level_name(t - 1),
                        level_name(t)
                    ));
                }
                Err(p) => {
                    if !p.iter().any(|p| p.name == "LevelError") {
                        return Outcome::Fail(format!(
                            "at level {} it fails, but not with a level problem",
                            level_name(t - 1)
                        ));
                    }
                }
            }
        }
    }
    if exp.out.is_none() && exp.error.is_none() {
        return Outcome::Fail("the test has no expectations".into());
    }
    let tmp = temp_file("unit");
    if let Err(e) = std::fs::write(&tmp, &linked.text) {
        return Outcome::Fail(format!("can't write the linked file: {e}"));
    }
    let run = Command::new(standin).arg(&tmp).output();
    let _ = std::fs::remove_file(&tmp);
    let run = match run {
        Ok(r) => r,
        Err(e) => return Outcome::Fail(format!("can't start the stand-in: {e}")),
    };
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    let stderr = remap(&linked, &tmp, &stderr);
    match &exp.error {
        Some(name) => {
            let want = format!("Uncaught {name}");
            if run.status.success() || !stderr.lines().any(|l| l.starts_with(&want)) {
                return Outcome::Fail(format!("expected `{want}`; stderr was:\n{stderr}"));
            }
        }
        None => {
            if !run.status.success() {
                return Outcome::Fail(format!(
                    "it exited with {}; stderr was:\n{stderr}",
                    run.status
                ));
            }
        }
    }
    if let Some(want) = &exp.out {
        let got: Vec<&str> = stdout.lines().collect();
        if got.len() != want.len() || got.iter().zip(want).any(|(a, b)| a != b) {
            let mut msg = String::from("output differs\n");
            let n = got.len().max(want.len());
            for i in 0..n {
                let g = got.get(i).copied().unwrap_or("<none>");
                let w = want.get(i).map(String::as_str).unwrap_or("<none>");
                let mark = if g == w { " " } else { "!" };
                msg.push_str(&format!(
                    "{mark} {:>4}  want: {w}\n        got:  {g}\n",
                    i + 1
                ));
            }
            return Outcome::Fail(msg);
        }
    }
    Outcome::Pass
}

/// The `host` level module that runs a probe: it calls the probe's exported
/// `main()` and prints each item of the array it returns.
fn driver(probe_path: &str) -> Module {
    let (dir, file) = probe_path.rsplit_once('/').unwrap_or(("", probe_path));
    let path = if dir.is_empty() {
        "probe_driver.js".to_string()
    } else {
        format!("{dir}/probe_driver.js")
    };
    Module {
        path,
        level: LEVEL_ALL,
        builtins: false,
        src: format!(
            "import {{ main }} from \"./{file}\";\nconst out = main();\nfor (let i = 0; i < out.length; i++) {{\n  console.log(out[i]);\n}}\n"
        ),
    }
}

/// Rewrite `FILE:line:col` in stand-in messages to module positions.
fn remap(linked: &crate::anvil_js::link::Linked, tmp: &Path, text: &str) -> String {
    let name = tmp.to_string_lossy().to_string();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(at) = line.find(&name) {
            let rest = &line[at + name.len()..];
            let mut parts = rest.splitn(3, ':');
            let _ = parts.next();
            let l = parts.next().and_then(|v| v.parse::<u32>().ok());
            if let Some((path, ml)) = l.and_then(|l| source_line(linked, l)) {
                let tail = parts.next().unwrap_or("");
                out.push_str(&format!("{}{path}:{ml}:{tail}\n", &line[..at]));
                continue;
            }
            out.push_str(&line.replace(&name, "<linked>"));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
