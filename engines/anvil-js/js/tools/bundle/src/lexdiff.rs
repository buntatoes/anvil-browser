// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The token differential: the reference lexer (through `refdump
//! --tokens`) against the engine's own lexer running on the stand-in
//! (through `tests/drivers/lex_dump.js`).
//!
//! Both sides print the same dump format, byte offsets included (the JS
//! side converts its code-unit offsets), and both replay the rescans with
//! the same previous-token rule, so the dumps must be equal line for
//! line. When the reference rejects a file, the JS side must end with
//! `ERROR` and the same `line:col: message`; the tokens before an error
//! are not compared, because the reference prints none.
//!
//! Sources go to the stand-in in batches: the tool writes a module
//! `tests/drivers/lex_input.js` (in memory only) holding the batch as
//! string literals, links it with the driver and the lexer, and runs the
//! result once per batch.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::anvil_js::corejs::check::LEVEL_ALL;
use crate::anvil_js::link::{Module, link};
use crate::anvil_js::modules::ModuleList;
use crate::anvil_js::unit::{modules_for, rel_path, temp_file};

/// Where the driver is, relative to the root.
pub const DRIVER: &str = "tests/drivers/lex_dump.js";
const INPUT: &str = "tests/drivers/lex_input.js";

/// Settings for a run.
pub struct Opts {
    /// The stand-in binary.
    pub standin: PathBuf,
    /// The refdump binary.
    pub refdump: PathBuf,
    /// Files per stand-in run.
    pub batch: usize,
    /// Keep every n-th file only (1: all).
    pub sample: usize,
    /// Known differences: path suffix and the reason.
    pub known: Vec<(String, String)>,
}

/// Totals.
#[derive(Default, Debug)]
pub struct Summary {
    /// Files compared.
    pub files: usize,
    /// Files both sides accept.
    pub accepted: usize,
    /// Files both sides reject at the same place with the same message.
    pub rejected: usize,
    /// Token lines found equal.
    pub tokens: usize,
    /// Files that differ and are not known.
    pub mismatches: usize,
    /// Files that differ and are listed as known.
    pub known: usize,
    /// Files skipped (not UTF-8).
    pub skipped: usize,
    /// Known differences that were run and no longer differ.
    pub stale: usize,
}

/// `.js` and `.mjs` files under the given paths, sorted.
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
                } else if s.extension().is_some_and(|x| x == "js" || x == "mjs") {
                    out.push(s);
                }
            }
        } else {
            out.push(p.clone());
        }
    }
    Ok(out)
}

/// Read a known-differences file: `PATH-SUFFIX  reason` per line, `#`
/// for comments.
pub fn read_known(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (p, why) = l.split_once(char::is_whitespace).unwrap_or((l, ""));
        out.push((p.to_string(), why.trim().to_string()));
    }
    Ok(out)
}

/// A JS string literal for `s`: printable ASCII as is, every other code
/// unit as `\uXXXX`.
pub fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for u in s.encode_utf16() {
        match u {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(u as u8 as char),
            _ => out.push_str(&format!("\\u{u:04x}")),
        }
    }
    out.push('"');
    out
}

/// What the reference said about one file.
enum RefOut {
    Tokens(Vec<String>),
    Error(String),
}

fn reference(refdump: &Path, file: &Path, module: bool) -> Result<RefOut, String> {
    let mut c = Command::new(refdump);
    c.arg("--tokens").arg(file);
    if module {
        c.arg("--module");
    }
    let o = c.output().map_err(|e| format!("can't run refdump: {e}"))?;
    match o.status.code() {
        Some(0) => Ok(RefOut::Tokens(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect(),
        )),
        Some(1) => Ok(RefOut::Error(
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )),
        _ => Err(format!(
            "refdump failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
    }
}

/// The input module: the sources as string literals, their goals, and
/// any extra `export const` lines.
pub fn input_module(sources: &[(String, bool)], extra: &str) -> String {
    let mut input = String::from(
        "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n// Copyright (c) 2026 Buntato\nexport const SOURCES = [\n",
    );
    for (s, _) in sources {
        input.push_str(&js_string(s));
        input.push_str(",\n");
    }
    input.push_str("];\nexport const MODULE_GOAL = [");
    for (_, m) in sources {
        input.push_str(if *m { "true, " } else { "false, " });
    }
    input.push_str("];\n");
    input.push_str(extra);
    input
}

/// Link `driver` with its imports and an input module made on the fly,
/// run it on the stand-in, and return (success, stdout, stderr).
pub fn run_driver(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    driver: &str,
    input_path: &str,
    input: String,
    stats: bool,
) -> Result<(bool, String, String), String> {
    let src = std::fs::read_to_string(root.join(driver)).map_err(|e| format!("{driver}: {e}"))?;
    let extra = [Module {
        path: input_path.to_string(),
        level: LEVEL_ALL,
        builtins: true,
        src: input,
    }];
    let mods = modules_for(list, driver, &src, LEVEL_ALL, &extra).map_err(problems)?;
    let linked = link(&mods).map_err(problems)?;
    let tmp = temp_file("lexdiff");
    std::fs::write(&tmp, &linked.text).map_err(|e| format!("can't write the linked file: {e}"))?;
    let mut cmd = Command::new(standin);
    if stats {
        cmd.arg("--stats");
    }
    let run = cmd.arg(&tmp).output();
    let _ = std::fs::remove_file(&tmp);
    let run = run.map_err(|e| format!("can't start the stand-in: {e}"))?;
    Ok((
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).trim().to_string(),
    ))
}

/// Run the JS lexer on a batch of sources; returns each one's dump lines,
/// or an explanation for the whole batch.
fn js_batch(
    list: &ModuleList,
    root: &Path,
    standin: &Path,
    sources: &[(String, bool)],
) -> Result<Vec<Vec<String>>, String> {
    let input = input_module(sources, "");
    let (ok, stdout, stderr) = run_driver(list, root, standin, DRIVER, INPUT, input, false)?;
    let mut out: Vec<Vec<String>> = Vec::new();
    for l in stdout.lines() {
        if let Some(k) = l.strip_prefix("FILE ") {
            if k.parse::<usize>().ok() != Some(out.len()) {
                return Err(format!("the driver printed `{l}` out of order"));
            }
            out.push(Vec::new());
        } else if let Some(cur) = out.last_mut() {
            cur.push(l.to_string());
        } else {
            return Err(format!("the driver printed `{l}` before any file"));
        }
    }
    if !ok {
        return Err(format!(
            "the stand-in failed after {} of {} files: {stderr}",
            out.len(),
            sources.len()
        ));
    }
    if out.len() != sources.len() {
        return Err(format!(
            "the driver dumped {} of {} files",
            out.len(),
            sources.len()
        ));
    }
    Ok(out)
}

/// Steps and seconds from the stand-in's `--stats` line.
pub fn stats_of(stderr: &str) -> Option<(u64, f64)> {
    let l = stderr
        .lines()
        .find(|l| l.starts_with("standin: ") && l.contains(" steps in "))?;
    let rest = l.strip_prefix("standin: ")?;
    let (steps, rest) = rest.split_once(" steps in ")?;
    let secs = rest.strip_suffix(" s")?;
    Some((steps.parse().ok()?, secs.parse().ok()?))
}

/// Time the lexer on one file: a run that only reads the source into
/// code units, then one that also lexes it; the difference is the
/// lexer's share. Returns a one-line report.
pub fn time_file(
    list: &ModuleList,
    root: &Path,
    opts: &Opts,
    file: &Path,
) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let module = file.extension().is_some_and(|x| x == "mjs");
    let sources = [(text.clone(), module)];
    let mut runs = Vec::new();
    for lex in [false, true] {
        let input = input_module(&sources, &format!("export const LEX = {lex};\n"));
        let (ok, stdout, stderr) = run_driver(
            list,
            root,
            &opts.standin,
            "tests/drivers/lex_time.js",
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
        "{} bytes: {what}; lexing took {} steps and {:.2} s on the stand-in ({} steps and {:.2} s in all, budget 2000000000)",
        text.len(),
        s1.saturating_sub(*s0),
        (t1 - t0).max(0.0),
        s1,
        t1
    ))
}

pub fn problems(p: Vec<crate::anvil_js::link::Problem>) -> String {
    let v: Vec<String> = p.iter().map(|x| x.to_string()).collect();
    format!("the driver does not link:\n{}", v.join("\n"))
}

/// Compare one file: Ok((true, n)) when both accept it with the same n
/// token lines, Ok((false, 0)) when both reject it the same way, Err with
/// the first difference otherwise.
fn compare(r: &RefOut, js: &[String]) -> Result<(bool, usize), String> {
    match r {
        RefOut::Tokens(want) => {
            for (i, w) in want.iter().enumerate() {
                match js.get(i) {
                    Some(g) if g == w => {}
                    Some(g) => {
                        return Err(format!("token {}: reference `{w}`, lexer `{g}`", i + 1));
                    }
                    None => {
                        return Err(format!("token {}: reference `{w}`, lexer has none", i + 1));
                    }
                }
            }
            if js.len() > want.len() {
                return Err(format!("lexer has extra output `{}`", js[want.len()]));
            }
            Ok((true, want.len()))
        }
        RefOut::Error(msg) => {
            let want = format!("ERROR {msg}");
            match js.last() {
                Some(g) if *g == want => Ok((false, 0)),
                Some(g) if g.starts_with("ERROR ") => Err(format!(
                    "reference rejects `{msg}`, lexer rejects `{}`",
                    &g[6..]
                )),
                _ => Err(format!("reference rejects `{msg}`, lexer accepts")),
            }
        }
    }
}

/// Run the differential over `files`; prints the first difference of
/// each file that differs.
pub fn run(
    list: &ModuleList,
    root: &Path,
    files: &[PathBuf],
    opts: &Opts,
) -> Result<Summary, String> {
    let mut sum = Summary::default();
    let files: Vec<&PathBuf> = files.iter().step_by(opts.sample.max(1)).collect();
    for chunk in files.chunks(opts.batch.max(1)) {
        let mut srcs = Vec::new();
        let mut refs = Vec::new();
        let mut names = Vec::new();
        for f in chunk {
            let Ok(text) = std::fs::read(f).map(String::from_utf8) else {
                sum.skipped += 1;
                continue;
            };
            let Ok(text) = text else {
                sum.skipped += 1;
                continue;
            };
            let module = f.extension().is_some_and(|x| x == "mjs");
            refs.push(reference(&opts.refdump, f, module)?);
            srcs.push((text, module));
            names.push(rel_path(root, f));
        }
        if srcs.is_empty() {
            continue;
        }
        let js = js_batch(list, root, &opts.standin, &srcs)?;
        for ((name, r), dump) in names.iter().zip(&refs).zip(&js) {
            sum.files += 1;
            let known = opts
                .known
                .iter()
                .position(|(p, _)| name.ends_with(p.as_str()));
            let result = compare(r, dump);
            if let (Some(k), Ok(_)) = (known, &result) {
                sum.stale += 1;
                println!(
                    "STALE {name}: listed as known ({}), but it no longer differs",
                    opts.known[k].1
                );
            }
            match result {
                Ok((true, n)) => {
                    sum.accepted += 1;
                    sum.tokens += n;
                }
                Ok((false, _)) => sum.rejected += 1,
                Err(why) => match known {
                    Some(k) => {
                        sum.known += 1;
                        println!("known {name}: {why} ({})", opts.known[k].1);
                    }
                    None => {
                        sum.mismatches += 1;
                        println!("MISMATCH {name}: {why}");
                    }
                },
            }
        }
    }
    Ok(sum)
}

/// The identifier scan: `refdump --id-scan --step STEP` against
/// `tests/drivers/id_scan.js` on the stand-in. Returns the number of
/// range lines compared, or the first difference.
pub fn id_scan(list: &ModuleList, root: &Path, opts: &Opts, step: u32) -> Result<usize, String> {
    let o = Command::new(&opts.refdump)
        .args(["--id-scan", "--step", &step.to_string()])
        .output()
        .map_err(|e| format!("can't run refdump: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "refdump --id-scan failed: {}",
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    let want = String::from_utf8_lossy(&o.stdout).to_string();
    let driver_path = "tests/drivers/id_scan.js";
    let driver = std::fs::read_to_string(root.join(driver_path))
        .map_err(|e| format!("{driver_path}: {e}"))?;
    let extra = [Module {
        path: "tests/drivers/id_scan_input.js".to_string(),
        level: LEVEL_ALL,
        builtins: true,
        src: format!(
            "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n// Copyright (c) 2026 Buntato\nexport const STEP = {step};\n"
        ),
    }];
    let mods = modules_for(list, driver_path, &driver, LEVEL_ALL, &extra).map_err(problems)?;
    let linked = link(&mods).map_err(problems)?;
    let tmp = temp_file("idscan");
    std::fs::write(&tmp, &linked.text).map_err(|e| format!("can't write the linked file: {e}"))?;
    let run = Command::new(&opts.standin).arg(&tmp).output();
    let _ = std::fs::remove_file(&tmp);
    let run = run.map_err(|e| format!("can't start the stand-in: {e}"))?;
    if !run.status.success() {
        return Err(format!(
            "the stand-in exited with {}: {}",
            run.status,
            String::from_utf8_lossy(&run.stderr).trim()
        ));
    }
    let got = String::from_utf8_lossy(&run.stdout).to_string();
    let w: Vec<&str> = want.lines().collect();
    let g: Vec<&str> = got.lines().collect();
    for i in 0..w.len().max(g.len()) {
        let a = w.get(i).copied().unwrap_or("<none>");
        let b = g.get(i).copied().unwrap_or("<none>");
        if a != b {
            return Err(format!("range {}: reference `{a}`, lexer `{b}`", i + 1));
        }
    }
    Ok(w.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_literals_are_ascii() {
        assert_eq!(
            js_string("a\"\\\n\u{e9}\u{1f600}"),
            "\"a\\\"\\\\\\u000a\\u00e9\\ud83d\\ude00\""
        );
    }

    #[test]
    fn comparing_dumps() {
        let want = RefOut::Tokens(vec!["0 1 1:1 ident \"a\"".into(), "1 1 1:2 eof".into()]);
        let same: Vec<String> = vec!["0 1 1:1 ident \"a\"".into(), "1 1 1:2 eof".into()];
        assert_eq!(compare(&want, &same), Ok((true, 2)));
        let other: Vec<String> = vec!["0 1 1:1 ident \"b\"".into()];
        assert!(compare(&want, &other).unwrap_err().contains("token 1"));
        let err = RefOut::Error("1:3: unterminated string literal".into());
        let rejects: Vec<String> = vec!["ERROR 1:3: unterminated string literal".into()];
        assert_eq!(compare(&err, &rejects), Ok((false, 0)));
        let elsewhere: Vec<String> = vec!["ERROR 1:4: unterminated string literal".into()];
        assert!(compare(&err, &elsewhere).is_err());
        assert!(compare(&err, &same).unwrap_err().contains("accepts"));
    }

    #[test]
    fn stats_lines() {
        assert_eq!(
            stats_of("x\nstandin: 1234 steps in 0.500 s\n"),
            Some((1234, 0.5))
        );
        assert_eq!(stats_of("nothing"), None);
    }
}
