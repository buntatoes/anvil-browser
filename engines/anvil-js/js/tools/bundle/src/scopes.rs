// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The scope analysis checks.
//!
//! `scopecheck` runs the engine's scope analysis (`src/syntax/scope.js`)
//! on the stand-in over the given sources and checks every result with
//! `src/syntax/scope_check.js` (through `tests/drivers/scope_check.js`).
//! Sources that do not parse are counted and skipped.
//!
//! `scopediff` compares what can be compared with the reference
//! compiler's scope pass: per function, the declared names with their
//! kinds and whether an inner function captures them, and the
//! `arguments`, mapping, strictness, simple-parameter and arrow-`this`
//! flags (`refdump --scopes-batch` against `tests/drivers/scope_facts.js`).
//! Only scripts without classes and `with` are compared: the reference
//! pass leaves classes out and supports neither `with` nor modules.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::anvil_js::astdiff::{Source, batch_text, sections};
use crate::anvil_js::lexdiff::{input_module, run_driver};
use crate::anvil_js::modules::ModuleList;
use crate::anvil_js::unit::temp_file;

const CHECK_DRIVER: &str = "tests/drivers/scope_check.js";
const FACTS_DRIVER: &str = "tests/drivers/scope_facts.js";
const INPUT: &str = "tests/drivers/scope_input.js";

/// Counts of a `scopecheck` run.
#[derive(Default, Debug)]
pub struct CheckSummary {
    pub sources: usize,
    pub checked: usize,
    pub not_parsed: usize,
    pub places: u64,
    pub bindings: u64,
    pub refs: u64,
    pub funcs: u64,
    pub scopes: u64,
    pub failed: usize,
}

fn engine(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    driver: &str,
    sources: &[&Source],
) -> Result<Vec<Vec<String>>, String> {
    let pairs: Vec<(String, bool)> = sources.iter().map(|s| (s.text.clone(), s.module)).collect();
    let input = input_module(&pairs, "");
    let (ok, stdout, stderr) = run_driver(list, root, standin, driver, INPUT, input, false)?;
    if !ok {
        let done = stdout.lines().filter(|l| l.starts_with("FILE ")).count();
        return Err(format!(
            "the stand-in failed in source {} of {}: {}",
            done,
            sources.len(),
            stderr.trim()
        ));
    }
    sections(&stdout, sources.len(), "the driver")
}

pub fn check(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    sources: &[Source],
    sample: usize,
    batch: usize,
    s: &mut CheckSummary,
) -> Result<(), String> {
    let picked: Vec<&Source> = sources.iter().step_by(sample.max(1)).collect();
    for chunk in picked.chunks(batch.max(1)) {
        let out = engine(list, root, standin, CHECK_DRIVER, chunk)?;
        for (src, lines) in chunk.iter().zip(&out) {
            s.sources += 1;
            match lines.first().map(String::as_str) {
                Some(l) if l.starts_with("SKIP ") => s.not_parsed += 1,
                Some(l) if l.starts_with("OK ") => {
                    let n: Vec<u64> = l[3..].split(' ').filter_map(|x| x.parse().ok()).collect();
                    if n.len() != 5 {
                        return Err(format!("{}: bad driver line `{l}`", src.name));
                    }
                    s.checked += 1;
                    s.places += n[0];
                    s.bindings += n[1];
                    s.refs += n[2];
                    s.funcs += n[3];
                    s.scopes += n[4];
                }
                _ => {
                    s.failed += 1;
                    for l in lines.iter().take(5) {
                        println!("FAIL {}: {l}", src.name);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Counts of a `scopediff` run.
#[derive(Default, Debug)]
pub struct DiffSummary {
    pub sources: usize,
    pub compared: usize,
    pub skipped: usize,
    pub functions: usize,
    pub equal: usize,
    pub known: usize,
    /// Accepted by the reference, rejected by the engine's early errors.
    pub engine_rejected: usize,
    pub mismatches: usize,
    pub stale: usize,
}

fn reference(refdump: &Path, sources: &[&Source]) -> Result<Vec<Vec<String>>, String> {
    let tmp = temp_file("scopediff-batch");
    std::fs::write(&tmp, batch_text(sources)).map_err(|e| format!("can't write a batch: {e}"))?;
    let o = Command::new(refdump)
        .arg("--scopes-batch")
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let o = o.map_err(|e| format!("can't run refdump: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "refdump failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    sections(
        &String::from_utf8_lossy(&o.stdout),
        sources.len(),
        "refdump",
    )
}

/// The first differing line of two fact lists, for the report.
fn first_difference(want: &[String], got: &[String]) -> String {
    let n = want.len().max(got.len());
    for i in 0..n {
        let w = want.get(i).map(String::as_str).unwrap_or("<none>");
        let g = got.get(i).map(String::as_str).unwrap_or("<none>");
        if w != g {
            return format!("line {}: reference `{w}`, engine `{g}`", i + 1);
        }
    }
    String::new()
}

#[allow(clippy::too_many_arguments)]
pub fn diff(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    refdump: &Path,
    sources: &[Source],
    sample: usize,
    batch: usize,
    known: &[(String, String)],
    save: Option<&Path>,
    s: &mut DiffSummary,
) -> Result<(), String> {
    if let Some(dir) = save {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let picked: Vec<&Source> = sources
        .iter()
        .filter(|x| !x.module)
        .step_by(sample.max(1))
        .collect();
    for chunk in picked.chunks(batch.max(1)) {
        let want = reference(refdump, chunk)?;
        let keep: Vec<usize> = (0..chunk.len())
            .filter(|&i| !want[i].first().is_some_and(|l| l.starts_with("SKIP ")))
            .collect();
        s.sources += chunk.len();
        s.skipped += chunk.len() - keep.len();
        if keep.is_empty() {
            continue;
        }
        let sub: Vec<&Source> = keep.iter().map(|&i| chunk[i]).collect();
        let got = engine(list, root, standin, FACTS_DRIVER, &sub)?;
        for (j, &i) in keep.iter().enumerate() {
            let src = chunk[i];
            if got[j].first().is_some_and(|l| l.starts_with("SKIP ")) {
                s.engine_rejected += 1;
                continue;
            }
            s.compared += 1;
            s.functions += want[i].iter().filter(|l| l.starts_with("func ")).count();
            let k = known
                .iter()
                .position(|(p, _)| src.name.ends_with(p.as_str()));
            if want[i] == got[j] {
                s.equal += 1;
                if let Some(k) = k {
                    s.stale += 1;
                    println!(
                        "STALE {}: listed as known ({}), but it no longer differs",
                        src.name, known[k].1
                    );
                }
            } else if let Some(k) = k {
                s.known += 1;
                println!(
                    "known {}: {} ({})",
                    src.name,
                    first_difference(&want[i], &got[j]),
                    known[k].1
                );
            } else {
                s.mismatches += 1;
                println!(
                    "MISMATCH {}: {}",
                    src.name,
                    first_difference(&want[i], &got[j])
                );
                if let Some(dir) = save {
                    save_mismatch(dir, s.mismatches, src, &want[i], &got[j])?;
                }
            }
        }
    }
    Ok(())
}

/// Writes one mismatch as `N.js` (the source), `N.reference` and
/// `N.engine` (the facts each side printed).
fn save_mismatch(
    dir: &Path,
    n: usize,
    src: &Source,
    want: &[String],
    got: &[String],
) -> Result<(), String> {
    let write = |ext: &str, text: String| {
        let path = dir.join(format!("{n}.{ext}"));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write("js", src.text.clone())?;
    write("reference", want.join("\n") + "\n")?;
    write("engine", got.join("\n") + "\n")
}

/// The sources for either check (same rules as `astdiff`).
pub fn collect(root: &Path, paths: &[PathBuf], skipped: &mut usize) -> Result<Vec<Source>, String> {
    crate::anvil_js::astdiff::collect(root, paths, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn first_difference_names_the_line() {
        let a = lines(&["func 0", "  \"a\" var -"]);
        let b = lines(&["func 0", "  \"a\" var captured"]);
        assert_eq!(
            first_difference(&a, &b),
            "line 2: reference `  \"a\" var -`, engine `  \"a\" var captured`"
        );
        assert_eq!(
            first_difference(&a, &a[..1]),
            "line 2: reference `  \"a\" var -`, engine `<none>`"
        );
        assert_eq!(first_difference(&a, &a), "");
    }
}
