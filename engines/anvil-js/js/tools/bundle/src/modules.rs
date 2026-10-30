// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! MODULES.txt: the modules of the engine, in load order.
//!
//! One module per line: a path relative to the file, then optionally
//! `needs=LEVEL` with LEVEL one of `plain`, `closures`, `objects`,
//! `strings`, `errors`, `syntax`, `collections` and `host` (the default),
//! and `builtins=yes` for a module that may use the extra built-ins (see
//! `corejs::check::NEEDS_EXTRA_BUILTINS`). `#` starts a comment. A module
//! may import only from modules listed before it, and only from modules
//! of the same or an earlier level.

use std::path::{Path, PathBuf};

use crate::anvil_js::corejs::check::{LEVEL_ALL, Level, parse_level};
use crate::anvil_js::link::Module;

/// One listed module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the root.
    pub path: String,
    /// Level.
    pub level: Level,
    /// Marked `builtins=yes`: may use the extra built-ins.
    pub builtins: bool,
    /// Line in MODULES.txt.
    pub line: usize,
}

/// The parsed list.
#[derive(Clone, Debug)]
pub struct ModuleList {
    /// The directory paths are relative to.
    pub root: PathBuf,
    /// Entries in order.
    pub entries: Vec<Entry>,
}

/// Parse the text of MODULES.txt.
pub fn parse_list(text: &str) -> Result<Vec<Entry>, String> {
    let mut out: Vec<Entry> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let path = words.next().unwrap_or("").to_string();
        let mut level = LEVEL_ALL;
        let mut builtins = false;
        for w in words {
            if w == "builtins=yes" {
                builtins = true;
                continue;
            }
            match w.strip_prefix("needs=").and_then(parse_level) {
                Some(t) => level = t,
                None => return Err(format!("MODULES.txt:{}: unknown setting `{w}`", i + 1)),
            }
        }
        if !path.ends_with(".js") || path.starts_with('/') || path.contains("..") {
            return Err(format!(
                "MODULES.txt:{}: `{path}` must be a relative `.js` path inside the tree",
                i + 1
            ));
        }
        if out.iter().any(|e| e.path == path) {
            return Err(format!("MODULES.txt:{}: `{path}` is listed twice", i + 1));
        }
        out.push(Entry {
            path,
            level,
            builtins,
            line: i + 1,
        });
    }
    Ok(out)
}

impl ModuleList {
    /// Read `ROOT/MODULES.txt`.
    pub fn read(root: &Path) -> Result<Self, String> {
        let f = root.join("MODULES.txt");
        let text =
            std::fs::read_to_string(&f).map_err(|e| format!("can't read MODULES.txt: {e}"))?;
        Ok(Self {
            root: root.to_path_buf(),
            entries: parse_list(&text)?,
        })
    }

    /// Source of entry `i`.
    pub fn source(&self, i: usize) -> Result<String, String> {
        let p = &self.entries[i].path;
        std::fs::read_to_string(self.root.join(p)).map_err(|e| format!("can't read {p}: {e}"))
    }

    /// Every module with its source.
    pub fn load_all(&self) -> Result<Vec<Module>, String> {
        let mut out = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            out.push(Module {
                path: e.path.clone(),
                level: e.level,
                builtins: e.builtins,
                src: self.source(i)?,
            });
        }
        Ok(out)
    }
}
