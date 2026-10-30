// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Linting and flattening Core JS modules.
//!
//! Every module is parsed in module mode, checked against the subset and
//! its level, and then its top-level names are renamed to
//! `<module key>$<name>` so that all modules can share one top-level scope
//! with no wrapper functions. Imports are blanked (their lines are kept, so
//! line numbers inside a module do not move), and the `export` keyword is
//! removed. Renaming follows the scope analysis, so a nested binding that
//! shadows a top-level name is left alone.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::anvil_js::corejs::ast::{Program, StmtKind};
use crate::anvil_js::corejs::check::{Level, check, level_name};
use crate::anvil_js::corejs::lexer::Diag;
use crate::anvil_js::corejs::parser::{DEFAULT_MAX_DEPTH, parse};
use crate::anvil_js::corejs::scope::{Analysis, BindKind, Res, analyse};

/// One module to link.
#[derive(Clone, Debug)]
pub struct Module {
    /// Path relative to the root, with `/` separators.
    pub path: String,
    /// Declared level.
    pub level: Level,
    /// May use the extra built-ins.
    pub builtins: bool,
    /// Source text.
    pub src: String,
}

/// A problem, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// File the problem is in.
    pub file: String,
    /// 1-based line (0 when it is about the file as a whole).
    pub line: u32,
    /// 1-based column.
    pub col: u32,
    /// Error name (`SyntaxError`, `SubsetError`, `LevelError`,
    /// `ModuleError`).
    pub name: String,
    /// Description.
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.file, self.line, self.col, self.name, self.message
        )
    }
}

fn from_diag(file: &str, d: &Diag) -> Problem {
    Problem {
        file: file.to_string(),
        line: d.line,
        col: d.col,
        name: d.name().to_string(),
        message: d.message.clone(),
    }
}

fn module_problem(file: &str, line: u32, col: u32, message: String) -> Problem {
    Problem {
        file: file.to_string(),
        line,
        col,
        name: "ModuleError".to_string(),
        message,
    }
}

/// An import statement.
struct Import {
    /// Index of the module it names.
    from: usize,
    /// Imported names, with the name id of each local binding.
    names: Vec<(String, u32, u32, u32)>,
    /// Byte range of the statement.
    start: usize,
    end: usize,
}

struct Parsed {
    prog: Program,
    an: Analysis,
    imports: Vec<Import>,
    exports: HashSet<String>,
}

/// The `$`-free key a module's names get as a prefix: the path without
/// `src/` and `.js`, other characters folded to `_`.
pub fn module_key(path: &str) -> String {
    let p = path.strip_prefix("src/").unwrap_or(path);
    let p = p.strip_suffix(".js").unwrap_or(p);
    let mut out: String = p
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'm');
    }
    out
}

/// Resolve an import specifier against the importing file. Only `./` and
/// `../` paths ending in `.js` are allowed.
pub fn resolve_spec(from: &str, spec: &str) -> Option<String> {
    if !(spec.starts_with("./") || spec.starts_with("../")) || !spec.ends_with(".js") {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in spec.split('/') {
        match seg {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            "" => return None,
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Parse, check and collect the imports and exports of one module.
/// `index` maps paths to module numbers; imports must name an earlier
/// module.
fn load(
    i: usize,
    m: &Module,
    mods: &[Module],
    index: &HashMap<String, usize>,
    out: &mut Vec<Problem>,
) -> Option<Parsed> {
    let po = parse(&m.src, true, DEFAULT_MAX_DEPTH);
    let prog = match po.program {
        Ok(p) => p,
        Err(e) => {
            out.push(from_diag(&m.path, &e));
            return None;
        }
    };
    let an = match analyse(&prog) {
        Ok(a) => a,
        Err(e) => {
            out.push(from_diag(&m.path, &e));
            return None;
        }
    };
    let mut diags = po.subset;
    diags.extend(check(&prog, &an, m.level, m.builtins));
    diags.sort_by_key(|d| d.start);
    let clean = diags.is_empty();
    out.extend(diags.iter().map(|d| from_diag(&m.path, d)));
    let mut imports = Vec::new();
    let mut exports = HashSet::new();
    let mut seen_other = false;
    for s in &prog.body {
        match &s.kind {
            StmtKind::Import(names, spec) => {
                if seen_other {
                    out.push(module_problem(
                        &m.path,
                        s.pos.line,
                        s.pos.col,
                        "imports must come before everything else in the module".into(),
                    ));
                }
                let Some(target) = resolve_spec(&m.path, spec) else {
                    out.push(module_problem(
                        &m.path,
                        s.pos.line,
                        s.pos.col,
                        format!("`{spec}` is not a relative `.js` path"),
                    ));
                    continue;
                };
                let Some(&from) = index.get(&target) else {
                    out.push(module_problem(
                        &m.path,
                        s.pos.line,
                        s.pos.col,
                        format!("`{target}` is not listed in MODULES.txt"),
                    ));
                    continue;
                };
                if from >= i {
                    out.push(module_problem(
                        &m.path,
                        s.pos.line,
                        s.pos.col,
                        format!("`{target}` must come before `{}` in MODULES.txt", m.path),
                    ));
                    continue;
                }
                if mods[from].level > m.level {
                    out.push(module_problem(
                        &m.path,
                        s.pos.line,
                        s.pos.col,
                        format!(
                            "a `{}` module can't import from `{target}`, which needs `{}`",
                            level_name(m.level),
                            level_name(mods[from].level)
                        ),
                    ));
                }
                imports.push(Import {
                    from,
                    names: names
                        .iter()
                        .map(|n| {
                            (
                                n.imported.to_string(),
                                n.local.id,
                                n.local.pos.line,
                                n.local.pos.col,
                            )
                        })
                        .collect(),
                    start: s.pos.start as usize,
                    end: s.pos.end as usize,
                });
            }
            StmtKind::Export(inner) => {
                seen_other = true;
                match &inner.kind {
                    StmtKind::Func(f) => {
                        if let Some(n) = &f.name {
                            exports.insert(n.name.to_string());
                        }
                    }
                    StmtKind::Class(c) => {
                        if let Some(n) = &c.name {
                            exports.insert(n.name.to_string());
                        }
                    }
                    StmtKind::Var(_, ds) => {
                        for (n, _) in ds {
                            exports.insert(n.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
            _ => seen_other = true,
        }
    }
    if !clean {
        return None;
    }
    Some(Parsed {
        prog,
        an,
        imports,
        exports,
    })
}

/// The flat bundle and where each module landed.
#[derive(Clone, Debug)]
pub struct Linked {
    /// Bundle text.
    pub text: String,
    /// `(first bundle line, module path, line count)` per module.
    pub map: Vec<(u32, String, u32)>,
}

/// The header placed at the top of every bundle.
pub const HEADER: &str = "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n// Copyright (c) 2026 Buntato\n// Built by tools/bundle from MODULES.txt; do not edit.\n";

/// Lint and flatten `mods`, in order.
pub fn link(mods: &[Module]) -> Result<Linked, Vec<Problem>> {
    let mut problems = Vec::new();
    let mut index = HashMap::new();
    for (i, m) in mods.iter().enumerate() {
        if index.insert(m.path.clone(), i).is_some() {
            problems.push(module_problem(&m.path, 0, 0, "listed twice".into()));
        }
    }
    let parsed: Vec<Option<Parsed>> = mods
        .iter()
        .enumerate()
        .map(|(i, m)| load(i, m, mods, &index, &mut problems))
        .collect();
    // Imported names must be exported.
    for (m, p) in mods.iter().zip(&parsed) {
        let Some(p) = p else { continue };
        for imp in &p.imports {
            let Some(t) = &parsed[imp.from] else { continue };
            for (name, _, line, col) in &imp.names {
                if !t.exports.contains(name) {
                    problems.push(module_problem(
                        &m.path,
                        *line,
                        *col,
                        format!("`{}` does not export `{name}`", mods[imp.from].path),
                    ));
                }
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let parsed: Vec<Parsed> = parsed
        .into_iter()
        .map(|p| p.expect("checked above"))
        .collect();

    // Every binding name in every module: new names must not clash.
    let mut taken: HashSet<String> = HashSet::new();
    for p in &parsed {
        for b in &p.an.bindings {
            taken.insert(b.name.to_string());
        }
    }
    // Rename top-level bindings that are not imports.
    let mut renames: Vec<BTreeMap<String, String>> = Vec::new();
    for (m, p) in mods.iter().zip(&parsed) {
        let key = module_key(&m.path);
        let mut r = BTreeMap::new();
        let mut names: Vec<&str> = p.an.scopes[0]
            .bindings
            .iter()
            .map(|b| &p.an.bindings[*b as usize])
            .filter(|b| !matches!(b.kind, BindKind::Special | BindKind::Import))
            .map(|b| &*b.name)
            .collect();
        names.sort_unstable();
        for n in names {
            let base = format!("{key}${n}");
            let mut cand = base.clone();
            let mut k = 2;
            while taken.contains(&cand) {
                cand = format!("{base}${k}");
                k += 1;
            }
            taken.insert(cand.clone());
            r.insert(n.to_string(), cand);
        }
        renames.push(r);
    }

    let mut text = String::from(HEADER);
    let mut line = HEADER.lines().count() as u32 + 1;
    let mut map = Vec::new();
    for (i, (m, p)) in mods.iter().zip(&parsed).enumerate() {
        let banner = format!("// ---- {} (needs={}) ----\n", m.path, level_name(m.level));
        text.push_str(&banner);
        line += 1;
        let body = rewrite(m, p, &renames[i], &renames);
        let mut body = body;
        if !body.ends_with('\n') {
            body.push('\n');
        }
        let n = body.matches('\n').count() as u32;
        map.push((line, m.path.clone(), n));
        line += n;
        text.push_str(&body);
    }
    Ok(Linked { text, map })
}

/// New text for one module.
fn rewrite(
    m: &Module,
    p: &Parsed,
    own: &BTreeMap<String, String>,
    all: &[BTreeMap<String, String>],
) -> String {
    let an = &p.an;
    // Imported name -> new name.
    let mut import_binding: HashMap<String, String> = HashMap::new();
    for imp in &p.imports {
        for (name, _, _, _) in &imp.names {
            let new = all[imp.from]
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone());
            import_binding.insert(name.clone(), new);
        }
    }
    let new_name_of = |b: u32| -> Option<String> {
        let bd = &an.bindings[b as usize];
        if bd.scope == 0 {
            return match bd.kind {
                BindKind::Special => None,
                BindKind::Import => import_binding.get(&*bd.name).cloned(),
                _ => own.get(&*bd.name).cloned(),
            };
        }
        if bd.kind == BindKind::ClassInner {
            // A top-level class's own name inside its body.
            let top = an.scopes[0].bindings.iter().find(|t| {
                let t = &an.bindings[**t as usize];
                t.name == bd.name && t.pos == bd.pos && t.kind == BindKind::Class
            });
            if top.is_some() {
                return own.get(&*bd.name).cloned();
            }
        }
        None
    };
    // Edits: (start, end, replacement).
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let in_import = |at: usize| p.imports.iter().any(|i| at >= i.start && at < i.end);
    for (id, pos) in an.name_pos.iter().enumerate() {
        let Some(pos) = pos else { continue };
        let start = pos.start as usize;
        if in_import(start) {
            continue;
        }
        let new = match an.res[id] {
            Res::Binding(b) => new_name_of(b),
            _ => None,
        };
        let Some(new) = new else { continue };
        let old = &m.src[start..pos.end as usize];
        let rep = if an.shorthand[id] {
            format!("{old}: {new}")
        } else {
            new
        };
        edits.push((start, pos.end as usize, rep));
    }
    // ClassInner bindings carry no name occurrence of their own; the
    // declaration name is shared with the outer binding, so it is covered.
    for imp in &p.imports {
        let blank: String = m.src[imp.start..imp.end]
            .chars()
            .map(|c| if c == '\n' { '\n' } else { ' ' })
            .collect();
        let blank = blank.trim_end_matches(' ').to_string();
        edits.push((imp.start, imp.end, blank));
    }
    for s in &p.prog.body {
        if let StmtKind::Export(_) = s.kind {
            let st = s.pos.start as usize;
            edits.push((st, st + "export ".len(), String::new()));
        }
    }
    edits.sort_by_key(|e| (e.0, e.1));
    edits.dedup_by_key(|e| e.0);
    let mut out = String::with_capacity(m.src.len() + edits.len() * 16);
    let mut at = 0;
    for (s, e, rep) in edits {
        if s < at {
            continue;
        }
        out.push_str(&m.src[at..s]);
        out.push_str(&rep);
        at = e;
    }
    out.push_str(&m.src[at..]);
    out
}

/// 64-bit FNV-1a, used as the bundle's content hash.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Text of the line map written next to the bundle.
pub fn map_text(l: &Linked) -> String {
    let mut s = String::new();
    s.push_str(&format!("hash fnv1a64 {:016x}\n", fnv1a(l.text.as_bytes())));
    for (first, path, n) in &l.map {
        s.push_str(&format!("{first} {n} {path}\n"));
    }
    s
}

/// Map a bundle line back to `(module path, line)`.
pub fn source_line(l: &Linked, line: u32) -> Option<(String, u32)> {
    l.map
        .iter()
        .find(|(first, _, n)| line >= *first && line < first + n)
        .map(|(first, path, _)| (path.clone(), line - first + 1))
}
