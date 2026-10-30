// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! `bundle`: lints Core JS modules, flattens them into one file, checks
//! the house rules, and runs Core JS tests on the stand-in host.

#![forbid(unsafe_code)]

#[allow(dead_code)]
#[path = "../../corejs/mod.rs"]
mod corejs;

mod astdiff;
mod house;
mod lexdiff;
mod link;
mod modules;
mod scopes;
mod unit;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;

use modules::ModuleList;

const USAGE: &str = "usage:
  bundle build [--root DIR] [--out FILE]
      lint every module in DIR/MODULES.txt and write the flat bundle to
      FILE (default dist/cinder.bundle.js under DIR) and its line map
      next to it (.map)
  bundle lint [--root DIR]
      only lint: subset, levels and imports, as FILE:LINE:COL problems
  bundle house [--root DIR]
      house rules for every file under DIR: license header, machine
      paths, no un-safe Rust, and the banned words and patterns listed
      in the files named by HOUSE_WORDS, when it is set
  bundle unit [--root DIR] --standin PATH FILE|DIR...
      link each test with what it imports and run it on the stand-in
  bundle probes [--root DIR] --standin PATH DIR
      run DIR/run/*.js as level probes and check DIR/reject/*.js are
      rejected with the problems they name
  bundle lexdiff [--root DIR] --standin PATH --refdump PATH [--batch N]
          [--sample N] [--known FILE] FILE|DIR...
      token differential: `refdump --tokens` against the engine's lexer
      on the stand-in, for every .js and .mjs file (.mjs as modules);
      --batch files per stand-in run (default 50), --sample keeps every
      N-th file, --known lists accepted differences (path suffix and
      reason per line)
  bundle lexdiff [--root DIR] --standin PATH --refdump PATH --id-scan STEP
      identifier scan: which code points (every STEP-th) each lexer takes
      to start or continue a name, plain and escaped
  bundle lexdiff [--root DIR] --standin PATH --refdump PATH --time FILE...
      time the engine's lexer on the stand-in (steps and seconds)
  bundle astdiff [--root DIR] --standin PATH --refdump PATH [--batch N]
          [--sample N] [--known FILE] FILE|DIR...
      syntax tree differential: `refdump --ast-batch --spans` against the
      engine's parser on the stand-in, for every .js, .mjs (as modules)
      and .batch file; names with `.ok.` or `.deep.` must parse or
      hit the nesting limit on both sides
  bundle astdiff [--root DIR] --standin PATH --time FILE...
      time the engine's parser on the stand-in (steps and seconds)
  bundle scopecheck [--root DIR] --standin PATH [--batch N] [--sample N]
          FILE|DIR...
      run the engine's scope analysis on every source and check each
      result for consistency (src/syntax/scope_check.js)
  bundle scopediff [--root DIR] --standin PATH --refdump PATH [--batch N]
          [--sample N] [--known FILE] [--out DIR] FILE|DIR...
      compare the scope facts both sides have (`refdump --scopes-batch`)
      for scripts without classes or `with`; --out saves each mismatch
      (source and both sides' facts) in DIR
  bundle parse [--root DIR] --standin PATH FILE
      parse FILE (.mjs as a module) with the engine's parser on the
      stand-in: exit 0 when accepted, or print `line:col: message` on
      stderr and exit 1

DIR defaults to the current directory.";

struct Args {
    cmd: String,
    root: PathBuf,
    out: Option<PathBuf>,
    standin: Option<PathBuf>,
    refdump: Option<PathBuf>,
    known: Option<PathBuf>,
    batch: usize,
    sample: usize,
    id_scan: Option<usize>,
    time: bool,
    rest: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or("no command given")?;
    let mut a = Args {
        cmd,
        root: PathBuf::from("."),
        out: None,
        standin: None,
        refdump: None,
        known: None,
        batch: 50,
        sample: 1,
        id_scan: None,
        time: false,
        rest: Vec::new(),
    };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--root" => a.root = it.next().ok_or("--root needs a directory")?.into(),
            "--out" => a.out = Some(it.next().ok_or("--out needs a file")?.into()),
            "--standin" => a.standin = Some(it.next().ok_or("--standin needs a path")?.into()),
            "--refdump" => a.refdump = Some(it.next().ok_or("--refdump needs a path")?.into()),
            "--known" => a.known = Some(it.next().ok_or("--known needs a file")?.into()),
            "--batch" => a.batch = number(it.next(), "--batch")?,
            "--sample" => a.sample = number(it.next(), "--sample")?,
            "--id-scan" => a.id_scan = Some(number(it.next(), "--id-scan")?),
            "--time" => a.time = true,
            _ if x.starts_with("--") => return Err(format!("unknown option {x}")),
            _ => a.rest.push(x.into()),
        }
    }
    Ok(a)
}

fn number(v: Option<String>, what: &str) -> Result<usize, String> {
    v.and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("{what} needs a positive number"))
}

fn print_problems(p: &[link::Problem]) {
    for x in p {
        eprintln!("{x}");
    }
    eprintln!("{} problem(s)", p.len());
}

fn cmd_build(a: &Args, write: bool) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let mods = list.load_all()?;
    match link::link(&mods) {
        Err(p) => {
            print_problems(&p);
            Ok(false)
        }
        Ok(l) => {
            // The flat result must itself be valid Core JS.
            let po = corejs::parser::parse(&l.text, false, corejs::parser::DEFAULT_MAX_DEPTH);
            let ok = match po.program {
                Ok(p) => match corejs::scope::analyse(&p) {
                    Ok(an) => {
                        po.subset.is_empty()
                            && corejs::check::check(&p, &an, corejs::check::LEVEL_ALL, true)
                                .is_empty()
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            };
            if !ok {
                return Err("the flat bundle does not lint clean; this is a bundler bug".into());
            }
            if write {
                let out = a
                    .out
                    .clone()
                    .unwrap_or_else(|| a.root.join("dist").join("cinder.bundle.js"));
                if let Some(d) = out.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(&out, &l.text).map_err(|e| e.to_string())?;
                let map = out.with_extension("map");
                std::fs::write(&map, link::map_text(&l)).map_err(|e| e.to_string())?;
                println!(
                    "{} modules, {} lines, hash {:016x}",
                    mods.len(),
                    l.text.lines().count(),
                    link::fnv1a(l.text.as_bytes())
                );
            } else {
                println!("{} modules lint clean", mods.len());
            }
            Ok(true)
        }
    }
}

fn cmd_house(a: &Args) -> Result<bool, String> {
    let banned = match std::env::var_os("HOUSE_WORDS") {
        Some(p) if !p.is_empty() => house::Words::read(&p)?,
        _ => house::Words::default(),
    };
    println!(
        "{} banned word(s) and pattern(s) from HOUSE_WORDS",
        banned.count()
    );
    let (n, f) = house::scan(&a.root, &banned).map_err(|e| e.to_string())?;
    for x in &f {
        eprintln!("{x}");
    }
    println!("{n} files scanned, {} finding(s)", f.len());
    Ok(f.is_empty())
}

fn run_many(a: &Args, files: &[PathBuf], probe: Option<&str>) -> Result<(usize, usize), String> {
    let list = ModuleList::read(&a.root)?;
    let standin = a.standin.clone().ok_or("--standin is required")?;
    let mut pass = 0;
    let mut fail = 0;
    for f in files {
        let name = unit::rel_path(&a.root, f);
        match unit::run_test(&list, &a.root, &standin, f, probe) {
            unit::Outcome::Pass => {
                pass += 1;
            }
            unit::Outcome::Fail(why) => {
                fail += 1;
                eprintln!("FAIL {name}\n{}", indent(&why));
            }
        }
    }
    Ok((pass, fail))
}

fn indent(s: &str) -> String {
    let mut out = String::new();
    for l in s.lines() {
        out.push_str("    ");
        out.push_str(l);
        out.push('\n');
    }
    out
}

fn cmd_unit(a: &Args) -> Result<bool, String> {
    let files = unit::collect(&a.rest).map_err(|e| e.to_string())?;
    let (p, f) = run_many(a, &files, None)?;
    println!("unit: {p} passed, {f} failed");
    Ok(f == 0 && p > 0)
}

fn cmd_probes(a: &Args) -> Result<bool, String> {
    let dir = a.rest.first().ok_or("probes needs a directory")?;
    let run = unit::collect(&[dir.join("run")]).map_err(|e| e.to_string())?;
    let reject = unit::collect(&[dir.join("reject")]).map_err(|e| e.to_string())?;
    let (p1, f1) = run_many(a, &run, Some("run"))?;
    let (p2, f2) = run_many(a, &reject, Some("reject"))?;
    println!("probes: run {p1} passed, {f1} failed; reject {p2} passed, {f2} failed");
    Ok(f1 + f2 == 0 && p1 > 0 && p2 > 0)
}

fn cmd_lexdiff(a: &Args) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let known = match &a.known {
        Some(k) => lexdiff::read_known(k)?,
        None => Vec::new(),
    };
    let opts = lexdiff::Opts {
        standin: a.standin.clone().ok_or("--standin is required")?,
        refdump: a.refdump.clone().ok_or("--refdump is required")?,
        batch: a.batch,
        sample: a.sample,
        known,
    };
    if let Some(step) = a.id_scan {
        let step = u32::try_from(step).map_err(|_| "--id-scan step is too large")?;
        return match lexdiff::id_scan(&list, &a.root, &opts, step) {
            Ok(n) => {
                println!("id-scan: code points 0 to 10ffff in steps of {step}, {n} ranges equal");
                Ok(true)
            }
            Err(why) => {
                println!("MISMATCH id-scan: {why}");
                Ok(false)
            }
        };
    }
    let files = lexdiff::collect(&a.rest).map_err(|e| e.to_string())?;
    if a.time {
        for f in &files {
            println!(
                "time {}: {}",
                f.display(),
                lexdiff::time_file(&list, &a.root, &opts, f)?
            );
        }
        return Ok(!files.is_empty());
    }
    let s = lexdiff::run(&list, &a.root, &files, &opts)?;
    println!(
        "lexdiff: {} files ({} accepted, {} rejected alike), {} tokens equal, {} mismatches, {} known, {} stale, {} skipped",
        s.files, s.accepted, s.rejected, s.tokens, s.mismatches, s.known, s.stale, s.skipped
    );
    Ok(s.mismatches == 0 && s.stale == 0 && s.files > 0)
}

fn cmd_astdiff(a: &Args) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let standin = a.standin.clone().ok_or("--standin is required")?;
    if a.time {
        let files = lexdiff::collect(&a.rest).map_err(|e| e.to_string())?;
        for f in &files {
            println!(
                "time {}: {}",
                f.display(),
                astdiff::time_file(&list, &a.root, &standin, f)?
            );
        }
        return Ok(!files.is_empty());
    }
    let known = match &a.known {
        Some(k) => lexdiff::read_known(k)?,
        None => Vec::new(),
    };
    let opts = astdiff::Opts {
        standin,
        refdump: a.refdump.clone().ok_or("--refdump is required")?,
        batch: a.batch,
        sample: a.sample,
        known,
    };
    let mut s = astdiff::Summary::default();
    let sources = astdiff::collect(&a.root, &a.rest, &mut s.skipped)?;
    astdiff::run(&list, &a.root, &sources, &opts, &mut s)?;
    println!(
        "astdiff: {} sources ({} accepted, {} rejected alike), {} nodes equal, {} mismatches, {} known, {} stale, {} depth checks, {} skipped",
        s.sources,
        s.accepted,
        s.rejected,
        s.nodes,
        s.mismatches,
        s.known,
        s.stale,
        s.depth_checked,
        s.skipped
    );
    Ok(s.mismatches == 0 && s.stale == 0 && s.sources > 0)
}

fn cmd_parse(a: &Args) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let standin = a.standin.clone().ok_or("--standin is required")?;
    let [file] = a.rest.as_slice() else {
        return Err("parse needs one file".into());
    };
    match astdiff::parse_one(&list, &a.root, &standin, file)? {
        None => Ok(true),
        Some(e) => {
            eprintln!("{e}");
            Ok(false)
        }
    }
}

fn cmd_scopecheck(a: &Args) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let standin = a.standin.clone().ok_or("--standin is required")?;
    let mut skipped = 0;
    let sources = scopes::collect(&a.root, &a.rest, &mut skipped)?;
    let mut s = scopes::CheckSummary::default();
    scopes::check(
        &list, &a.root, &standin, &sources, a.sample, a.batch, &mut s,
    )?;
    println!(
        "scopecheck: {} sources, {} checked, {} not parsed, {} failed; {} identifier places, {} bindings, {} references, {} functions, {} scopes; {} skipped",
        s.sources,
        s.checked,
        s.not_parsed,
        s.failed,
        s.places,
        s.bindings,
        s.refs,
        s.funcs,
        s.scopes,
        skipped
    );
    Ok(s.failed == 0 && s.checked > 0)
}

fn cmd_scopediff(a: &Args) -> Result<bool, String> {
    let list = ModuleList::read(&a.root)?;
    let standin = a.standin.clone().ok_or("--standin is required")?;
    let refdump = a.refdump.clone().ok_or("--refdump is required")?;
    let known = match &a.known {
        Some(k) => lexdiff::read_known(k)?,
        None => Vec::new(),
    };
    let mut skipped = 0;
    let sources = scopes::collect(&a.root, &a.rest, &mut skipped)?;
    let mut s = scopes::DiffSummary::default();
    scopes::diff(
        &list,
        &a.root,
        &standin,
        &refdump,
        &sources,
        a.sample,
        a.batch,
        &known,
        a.out.as_deref(),
        &mut s,
    )?;
    println!(
        "scopediff: {} scripts, {} compared ({} functions), {} skipped (classes, with, not parsed), {} rejected only by the engine, {} equal, {} mismatches, {} known, {} stale",
        s.sources,
        s.compared,
        s.functions,
        s.skipped + skipped,
        s.engine_rejected,
        s.equal,
        s.mismatches,
        s.known,
        s.stale
    );
    Ok(s.mismatches == 0 && s.stale == 0 && s.compared > 0)
}

fn main() -> ExitCode {
    // The parser recurses; give it room (deep input is still capped by
    // the parser's own depth limit).
    let t = std::thread::Builder::new()
        .name("bundle".into())
        .stack_size(512 << 20)
        .spawn(real_main);
    match t.map(|h| h.join()) {
        Ok(Ok(code)) => ExitCode::from(code),
        _ => {
            eprintln!("bundle: internal error");
            ExitCode::from(3)
        }
    }
}

fn real_main() -> u8 {
    let a = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bundle: {e}\n{USAGE}");
            return 2;
        }
    };
    let r = match a.cmd.as_str() {
        "build" => cmd_build(&a, true),
        "lint" => cmd_build(&a, false),
        "house" => cmd_house(&a),
        "unit" => cmd_unit(&a),
        "probes" => cmd_probes(&a),
        "lexdiff" => cmd_lexdiff(&a),
        "astdiff" => cmd_astdiff(&a),
        "scopecheck" => cmd_scopecheck(&a),
        "scopediff" => cmd_scopediff(&a),
        "parse" => cmd_parse(&a),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(true)
        }
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    };
    match r {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("bundle: {e}");
            2
        }
    }
}
