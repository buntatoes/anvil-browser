// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The syntax tree differential: the reference parser (through `refdump
//! --ast-batch --spans`) against the engine's own parser running on the
//! stand-in (through `tests/drivers/parse_dump.js`).
//!
//! Both sides print the same dump format with byte spans, so the dumps
//! must be equal line for line. When the reference rejects a source the
//! engine must reject it too, at the same `line:col` with the same
//! message.
//!
//! Sources whose name contains `.ok.` must be accepted by both sides and
//! sources whose name contains `.deep.` must be rejected by both with the
//! nesting limit message; `progen depth` writes such pairs.
//!
//! Inputs are `.js` and `.mjs` files (`.mjs` as modules) and `.batch`
//! files, which hold many sources each (the format is in
//! `tools/refdump/FORMAT.md`); a source in a batch is named `FILE#k`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::anvil_js::lexdiff::{input_module, run_driver, stats_of};
use crate::anvil_js::modules::ModuleList;
use crate::anvil_js::unit::{rel_path, temp_file};

/// Where the driver is, relative to the root.
pub const DRIVER: &str = "tests/drivers/parse_dump.js";
const INPUT: &str = "tests/drivers/parse_input.js";

/// The nesting limit message both parsers use.
const DEPTH_MESSAGE: &str = "nesting is too deep (limit ";

/// Settings for a run.
pub struct Opts {
    /// The stand-in binary.
    pub standin: PathBuf,
    /// The refdump binary.
    pub refdump: PathBuf,
    /// Sources per run of each side.
    pub batch: usize,
    /// Keep every n-th source only (1: all).
    pub sample: usize,
    /// Known differences: name suffix and the reason.
    pub known: Vec<(String, String)>,
}

/// Totals.
#[derive(Default, Debug)]
pub struct Summary {
    /// Sources compared.
    pub sources: usize,
    /// Sources both sides accept with equal dumps.
    pub accepted: usize,
    /// Sources both sides reject at the same place with the same message.
    pub rejected: usize,
    /// Syntax tree nodes found equal (not counting lists and placeholders).
    pub nodes: usize,
    /// Sources that differ and are not known.
    pub mismatches: usize,
    /// Sources that differ and are listed as known.
    pub known: usize,
    /// Known differences that were run and no longer differ.
    pub stale: usize,
    /// Depth pairs checked (`.ok.` and `.deep.` sources).
    pub depth_checked: usize,
    /// Files skipped (not UTF-8).
    pub skipped: usize,
}

/// One source to compare.
pub struct Source {
    /// Its name in reports.
    pub name: String,
    /// The text.
    pub text: String,
    /// Module goal.
    pub module: bool,
}

/// Read a batch file: `#source script|module BYTES`, a line break, the
/// source, and a line break, repeated.
pub fn read_batch(path: &Path, name: &str) -> Result<Vec<Source>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let bad = || format!("{}: bad batch entry {}", path.display(), out.len());
        let eol = bytes[at..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| at + i)
            .ok_or_else(bad)?;
        let head = std::str::from_utf8(&bytes[at..eol]).map_err(|_| bad())?;
        let mut parts = head.split(' ');
        let (Some("#source"), Some(goal), Some(len), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        let module = match goal {
            "script" => false,
            "module" => true,
            _ => return Err(bad()),
        };
        let len: usize = len.parse().map_err(|_| bad())?;
        let start = eol + 1;
        let end = start + len;
        if bytes.get(end) != Some(&b'\n') {
            return Err(bad());
        }
        let text = String::from_utf8(bytes[start..end].to_vec()).map_err(|_| bad())?;
        out.push(Source {
            name: format!("{name}#{}", out.len()),
            text,
            module,
        });
        at = end + 1;
    }
    Ok(out)
}

/// Write sources in the batch format.
pub fn batch_text(sources: &[&Source]) -> String {
    let mut out = String::new();
    for s in sources {
        let goal = if s.module { "module" } else { "script" };
        out.push_str(&format!("#source {goal} {}\n", s.text.len()));
        out.push_str(&s.text);
        out.push('\n');
    }
    out
}

/// All sources under the given paths, in order; `.js`, `.mjs` and
/// `.batch` files, directories searched in sorted order.
pub fn collect(root: &Path, paths: &[PathBuf], skipped: &mut usize) -> Result<Vec<Source>, String> {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, &mut files).map_err(|e| format!("{}: {e}", p.display()))?;
        } else {
            files.push(p.clone());
        }
    }
    let mut out = Vec::new();
    for f in files {
        let name = rel_path(root, &f);
        if f.extension().is_some_and(|x| x == "batch") {
            out.extend(read_batch(&f, &name)?);
            continue;
        }
        match std::fs::read(&f).map(String::from_utf8) {
            Ok(Ok(text)) => out.push(Source {
                name,
                text,
                module: f.extension().is_some_and(|x| x == "mjs"),
            }),
            Ok(Err(_)) => *skipped += 1,
            Err(e) => return Err(format!("{}: {e}", f.display())),
        }
    }
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut sub = Vec::new();
    for e in std::fs::read_dir(dir)? {
        sub.push(e?.path());
    }
    sub.sort();
    for s in sub {
        if s.is_dir() {
            walk(&s, out)?;
        } else if s
            .extension()
            .is_some_and(|x| x == "js" || x == "mjs" || x == "batch")
        {
            out.push(s);
        }
    }
    Ok(())
}

/// Split `FILE k` sections; `n` must be the number of sources.
pub fn sections(text: &str, n: usize, who: &str) -> Result<Vec<Vec<String>>, String> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for l in text.lines() {
        if let Some(k) = l.strip_prefix("FILE ") {
            if k.parse::<usize>().ok() != Some(out.len()) {
                return Err(format!("{who} printed `{l}` out of order"));
            }
            out.push(Vec::new());
        } else if let Some(cur) = out.last_mut() {
            cur.push(l.to_string());
        } else {
            return Err(format!("{who} printed `{l}` before any source"));
        }
    }
    if out.len() != n {
        return Err(format!("{who} dumped {} of {n} sources", out.len()));
    }
    Ok(out)
}

fn reference(
    refdump: &Path,
    sources: &[&Source],
    parse_only: bool,
) -> Result<Vec<Vec<String>>, String> {
    let tmp = temp_file("astdiff-batch");
    std::fs::write(&tmp, batch_text(sources)).map_err(|e| format!("can't write a batch: {e}"))?;
    let mut c = Command::new(refdump);
    c.arg("--ast-batch").arg(&tmp).arg("--spans");
    if parse_only {
        c.arg("--parse-only");
    }
    let o = c.output();
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

fn engine(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    sources: &[&Source],
    parse_only: bool,
) -> Result<Vec<Vec<String>>, String> {
    let pairs: Vec<(String, bool)> = sources.iter().map(|s| (s.text.clone(), s.module)).collect();
    let extra = format!("export const SPANS = true;\nexport const PARSE_ONLY = {parse_only};\n");
    let input = input_module(&pairs, &extra);
    let (ok, stdout, stderr) = run_driver(list, root, standin, DRIVER, INPUT, input, false)?;
    if !ok {
        let done = stdout.lines().filter(|l| l.starts_with("FILE ")).count();
        return Err(format!(
            "the stand-in failed in source {} of {}: {stderr}",
            done,
            sources.len()
        ));
    }
    sections(&stdout, sources.len(), "the driver")
}

/// How one source compares.
#[derive(Debug, PartialEq)]
enum Verdict {
    /// Both accept; this many nodes.
    Accepted(usize),
    /// Both reject alike.
    Rejected,
    /// Differs.
    Differs(String),
}

fn compare(want: &[String], got: &[String]) -> Verdict {
    let ref_err = want.first().and_then(|l| l.strip_prefix("ERROR "));
    let js_err = got.first().and_then(|l| l.strip_prefix("ERROR "));
    match (ref_err, js_err) {
        (Some(r), Some(j)) if r == j => Verdict::Rejected,
        (Some(r), Some(j)) => {
            Verdict::Differs(format!("reference rejects `{r}`, parser rejects `{j}`"))
        }
        (Some(r), None) => Verdict::Differs(format!("reference rejects `{r}`, parser accepts")),
        (None, Some(j)) => Verdict::Differs(format!("reference accepts, parser rejects `{j}`")),
        (None, None) => {
            for (i, w) in want.iter().enumerate() {
                match got.get(i) {
                    Some(g) if g == w => {}
                    Some(g) => {
                        return Verdict::Differs(format!(
                            "dump line {}: reference `{}`, parser `{}`",
                            i + 1,
                            w.trim(),
                            g.trim()
                        ));
                    }
                    None => {
                        return Verdict::Differs(format!(
                            "dump line {}: reference `{}`, parser has none",
                            i + 1,
                            w.trim()
                        ));
                    }
                }
            }
            if got.len() > want.len() {
                return Verdict::Differs(format!(
                    "parser has extra output `{}`",
                    got[want.len()].trim()
                ));
            }
            Verdict::Accepted(want.iter().filter(|l| is_node(l)).count())
        }
    }
}

/// Whether a dump line opens a tree node: a kind such as `(Expr.Num`,
/// not a list such as `(body` or a placeholder `(None)` or `(Hole)`.
fn is_node(line: &str) -> bool {
    let rest = line.trim_start();
    let Some(kind) = rest.strip_prefix('(') else {
        return false;
    };
    kind.starts_with(|c: char| c.is_ascii_uppercase())
        && !kind.starts_with("None)")
        && !kind.starts_with("Hole)")
}

/// What a `.ok.` or `.deep.` name requires, if anything is wrong.
fn depth_problem(name: &str, want: &[String], got: &[String]) -> Option<String> {
    let deep = name.contains(".deep.");
    if !deep && !name.contains(".ok.") {
        return None;
    }
    for (who, lines) in [("reference", want), ("parser", got)] {
        let err = lines.first().and_then(|l| l.strip_prefix("ERROR "));
        match (deep, err) {
            (false, Some(e)) => {
                return Some(format!("{who} rejects `{e}`, but it is within the limit"));
            }
            (true, None) => return Some(format!("{who} accepts it, but it is over the limit")),
            (true, Some(e)) if !e.contains(DEPTH_MESSAGE) => {
                return Some(format!("{who} rejects `{e}`, not with the limit message"));
            }
            _ => {}
        }
    }
    None
}

/// Run the differential; prints each difference.
pub fn run(
    list: &ModuleList,
    root: &Path,
    sources: &[Source],
    opts: &Opts,
    summary: &mut Summary,
) -> Result<(), String> {
    let picked: Vec<&Source> = sources.iter().step_by(opts.sample.max(1)).collect();
    for chunk in picked.chunks(opts.batch.max(1)) {
        let want = reference(&opts.refdump, chunk, false)?;
        let got = engine(list, root, &opts.standin, chunk, false)?;
        for ((s, w), g) in chunk.iter().zip(&want).zip(&got) {
            summary.sources += 1;
            let known = opts
                .known
                .iter()
                .position(|(p, _)| s.name.ends_with(p.as_str()));
            let mut verdict = compare(w, g);
            if let Some(why) = depth_problem(&s.name, w, g) {
                verdict = Verdict::Differs(why);
            } else if s.name.contains(".ok.") || s.name.contains(".deep.") {
                summary.depth_checked += 1;
            }
            if let (Some(k), Verdict::Accepted(_) | Verdict::Rejected) = (known, &verdict) {
                summary.stale += 1;
                println!(
                    "STALE {}: listed as known ({}), but it no longer differs",
                    s.name, opts.known[k].1
                );
            }
            match verdict {
                Verdict::Accepted(n) => {
                    summary.accepted += 1;
                    summary.nodes += n;
                }
                Verdict::Rejected => summary.rejected += 1,
                Verdict::Differs(why) => match known {
                    Some(k) => {
                        summary.known += 1;
                        println!("known {}: {why} ({})", s.name, opts.known[k].1);
                    }
                    None => {
                        summary.mismatches += 1;
                        println!("MISMATCH {}: {why}", s.name);
                    }
                },
            }
        }
    }
    Ok(())
}

/// Parse one file with the engine's parser only: Ok(None) when it is
/// accepted, Ok(Some(error line)) when it is rejected.
pub fn parse_one(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    file: &Path,
) -> Result<Option<String>, String> {
    let text = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8(text).map_err(|_| format!("{} is not UTF-8", file.display()))?;
    let s = Source {
        name: rel_path(root, file),
        text,
        module: file.extension().is_some_and(|x| x == "mjs"),
    };
    let out = engine(list, root, standin, &[&s], true)?;
    match out[0].first().map(String::as_str) {
        Some("OK") => Ok(None),
        Some(l) if l.starts_with("ERROR ") => Ok(Some(l[6..].to_string())),
        _ => Err("the driver printed no verdict".into()),
    }
}

/// Time the parser on one file: a run that only reads the source into
/// code units, then one that also parses it; the difference is the
/// parser's share. Returns a one-line report.
pub fn time_file(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    file: &Path,
) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let module = file.extension().is_some_and(|x| x == "mjs");
    let sources = [(text.clone(), module)];
    let mut runs = Vec::new();
    for parse in [false, true] {
        let input = input_module(&sources, &format!("export const PARSE = {parse};\n"));
        let (ok, stdout, stderr) = run_driver(
            list,
            root,
            standin,
            "tests/drivers/parse_time.js",
            INPUT,
            input,
            true,
        )?;
        if !ok || stdout.starts_with("ERROR") {
            return Err(format!("the timing run failed: {}{stderr}", stdout.trim()));
        }
        let (steps, secs) = stats_of(&stderr).ok_or("no --stats line from the stand-in")?;
        runs.push((steps, secs, stdout.trim().to_string()));
    }
    let (s0, t0, _) = &runs[0];
    let (s1, t1, what) = &runs[1];
    Ok(format!(
        "{} bytes: {what}; parsing took {} steps and {:.2} s on the stand-in ({} steps and {:.2} s in all, budget 2000000000)",
        text.len(),
        s1.saturating_sub(*s0),
        (t1 - t0).max(0.0),
        s1,
        t1
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn comparing() {
        let a = lines(&[
            "(Program @0..2:1:1 script strict=false",
            "  (body",
            "    (Stmt.Empty @0..1:1:1))",
        ]);
        assert_eq!(compare(&a, &a), Verdict::Accepted(2));
        let e = lines(&["ERROR 1:2: unexpected `)`"]);
        assert_eq!(compare(&e, &e), Verdict::Rejected);
        assert!(matches!(compare(&e, &a), Verdict::Differs(_)));
        assert!(matches!(compare(&a, &e), Verdict::Differs(_)));
        let d = lines(&["ERROR 2:5: `x` has already been declared"]);
        assert!(matches!(compare(&d, &a), Verdict::Differs(_)));
        let later = lines(&["ERROR 3:1: unexpected end of input"]);
        assert!(matches!(compare(&d, &later), Verdict::Differs(_)));
    }

    #[test]
    fn depth_names() {
        let ok = lines(&["(Program"]);
        let deep = lines(&["ERROR 1:999: nesting is too deep (limit 1000)"]);
        assert_eq!(depth_problem("a/paren.ok.js", &ok, &ok), None);
        assert_eq!(depth_problem("a/paren.deep.js", &deep, &deep), None);
        assert!(depth_problem("a/paren.ok.js", &ok, &deep).is_some());
        assert!(depth_problem("a/paren.deep.js", &ok, &ok).is_some());
        assert_eq!(depth_problem("a/plain.js", &ok, &deep), None);
    }

    #[test]
    fn batches_round_trip() {
        let a = Source {
            name: "a".into(),
            text: "x;\n\u{e9}".into(),
            module: false,
        };
        let b = Source {
            name: "b".into(),
            text: String::new(),
            module: true,
        };
        let text = batch_text(&[&a, &b]);
        let tmp = temp_file("astdiff-test");
        std::fs::write(&tmp, &text).expect("write");
        let back = read_batch(&tmp, "t").expect("reads");
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].text, a.text);
        assert_eq!(back[1].name, "t#1");
        assert!(back[1].module);
    }
}
