// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! House rules for the whole tree: every file carries the license header,
//! Rust code forbids `un`+`safe` code, no file names a machine-specific
//! path, and no file uses a word or matches a pattern from an optional
//! list of banned words.
//!
//! The banned words and patterns are not kept in the tree. `bundle house`
//! reads them from the files named by the `HOUSE_WORDS` environment
//! variable, when it is set (see [`Words::parse`] for the format). The
//! path prefixes are put together from parts, so this file does not trip
//! its own scan.

use std::path::{Path, PathBuf};

/// Banned words, read from a word list.
#[derive(Clone, Debug, Default)]
pub struct Words {
    /// Compared whole-word and ignoring case (stored lowercased).
    words: Vec<String>,
    /// Compared as text, spaces kept, ignoring case (stored lowercased).
    phrases: Vec<String>,
    /// Compared whole-word with their case.
    exact: Vec<String>,
    /// Patterns, compared with their case, where `#` stands for one digit.
    patterns: Vec<String>,
}

impl Words {
    /// Read a word list: one entry per line, blank lines and lines
    /// starting with `#` skipped. An entry starting with `=` is a word
    /// compared with its case; one starting with `@` is a pattern, compared
    /// with its case, in which `#` stands for one digit, that must not be
    /// preceded by a letter, digit or `_` nor followed by one of those, `+`
    /// or `=`; an entry with a space is a phrase; any other entry is a whole
    /// word compared ignoring case.
    pub fn parse(text: &str) -> Words {
        let mut w = Words::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(e) = line.strip_prefix('=') {
                w.exact.push(e.to_string());
            } else if let Some(e) = line.strip_prefix('@') {
                w.patterns.push(e.to_string());
            } else if line.contains(' ') {
                w.phrases.push(line.to_lowercase());
            } else {
                w.words.push(line.to_lowercase());
            }
        }
        w
    }

    /// Read and join the lists of every file in `spec`, a list of paths
    /// separated like `PATH`.
    pub fn read(spec: &std::ffi::OsStr) -> Result<Words, String> {
        let mut all = Words::default();
        for p in std::env::split_paths(spec) {
            if p.as_os_str().is_empty() {
                continue;
            }
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let w = Words::parse(&text);
            all.words.extend(w.words);
            all.phrases.extend(w.phrases);
            all.exact.extend(w.exact);
            all.patterns.extend(w.patterns);
        }
        Ok(all)
    }

    /// How many entries there are.
    pub fn count(&self) -> usize {
        self.words.len() + self.phrases.len() + self.exact.len() + self.patterns.len()
    }

    /// The first pattern that matches somewhere in `line`, as the text it
    /// matched.
    fn pattern_in(&self, line: &str) -> Option<String> {
        let b = line.as_bytes();
        for p in &self.patterns {
            let pb = p.as_bytes();
            if pb.is_empty() || pb.len() > b.len() {
                continue;
            }
            for i in 0..=b.len() - pb.len() {
                if i > 0 && is_word_byte(b[i - 1]) {
                    continue;
                }
                let fits = pb.iter().zip(&b[i..]).all(|(&want, &got)| {
                    if want == b'#' {
                        got.is_ascii_digit()
                    } else {
                        want == got
                    }
                });
                let end = i + pb.len();
                let next = b.get(end).copied().unwrap_or(b' ');
                if fits
                    && !is_word_byte(next)
                    && next != b'+'
                    && next != b'='
                    && line.is_char_boundary(end)
                {
                    return Some(line[i..end].to_string());
                }
            }
        }
        None
    }
}

/// Path prefixes that point at one machine.
fn machine_paths() -> Vec<String> {
    let s = "/";
    vec![
        [s, "home", s].concat(),
        [s, "Users", s].concat(),
        [s, "root", s].concat(),
        [s, "work", "space", s].concat(),
        [s, "tmp", s].concat(),
        ["C:", "\\", "Users"].concat(),
    ]
}

/// Extensions that are scanned.
const EXTS: &[&str] = &[
    "rs", "md", "js", "mjs", "expected", "txt", "sh", "toml", "map",
];

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// File, relative to the scanned root.
    pub file: String,
    /// 1-based line, 0 for the file as a whole.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Find `needle` in `hay` as a whole word. Both already lowercased when
/// the comparison ignores case.
fn find_word(hay: &str, needle: &str) -> bool {
    let h = hay.as_bytes();
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let at = from + i;
        let end = at + needle.len();
        let before = at == 0 || !is_word_byte(h[at - 1]);
        let after = end >= h.len() || !is_word_byte(h[end]);
        if before && after {
            return true;
        }
        from = at + 1;
        while !hay.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

/// The comment prefix the header uses for a file.
fn header_lines(ext: &str) -> Option<(String, String)> {
    let spdx = "SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial";
    let copy = "Copyright (c) 2026 Buntato";
    match ext {
        "rs" | "js" => Some((format!("// {spdx}"), format!("// {copy}"))),
        "sh" | "toml" | "txt" => Some((format!("# {spdx}"), format!("# {copy}"))),
        "md" => Some((format!("<!-- {spdx} -->"), format!("<!-- {copy} -->"))),
        _ => None,
    }
}

/// Check one file's text.
pub fn check_text(file: &str, ext: &str, text: &str, banned: &Words) -> Vec<Finding> {
    let mut out = Vec::new();
    let add = |out: &mut Vec<Finding>, line: usize, message: String| {
        out.push(Finding {
            file: file.to_string(),
            line,
            message,
        });
    };
    if let Some((a, b)) = header_lines(ext) {
        let mut lines = text.lines();
        let mut first = lines.next().unwrap_or("");
        if ext == "sh" && first.starts_with("#!") {
            first = lines.next().unwrap_or("");
        }
        let second = lines.next().unwrap_or("");
        if first != a || second != b {
            add(&mut out, 1, "missing the SPDX and copyright header".into());
        }
    }
    let paths = machine_paths();
    let marker = ["un", "safe"].concat();
    let forbid = format!("forbid({marker}_code)");
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let lower = line.to_lowercase();
        for w in &banned.words {
            if find_word(&lower, w) {
                add(&mut out, n, format!("banned word `{w}`"));
            }
        }
        for w in &banned.phrases {
            if lower.contains(w.as_str()) {
                add(&mut out, n, format!("banned phrase `{w}`"));
            }
        }
        for w in &banned.exact {
            if find_word(line, w) {
                add(&mut out, n, format!("banned name `{w}`"));
            }
        }
        for p in &paths {
            if line.contains(p.as_str()) {
                add(&mut out, n, "machine-specific path".into());
            }
        }
        if let Some(l) = banned.pattern_in(line) {
            add(&mut out, n, format!("banned pattern match `{l}`"));
        }
        if ext == "rs" && line.contains(&marker) && !line.contains(&forbid) {
            add(&mut out, n, format!("`{marker}` in Rust code"));
        }
    }
    out
}

/// Rust crate roots must forbid the marker at the top.
fn check_crate_root(file: &str, text: &str) -> Option<Finding> {
    let want = format!("#![forbid({}_code)]", ["un", "safe"].concat());
    if text.lines().any(|l| l.trim() == want) {
        None
    } else {
        Some(Finding {
            file: file.to_string(),
            line: 0,
            message: format!("crate root without `{want}`"),
        })
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // Cargo.lock is written by cargo on every build; it is not ours.
        if name.starts_with('.') || name == "target" || name == "Cargo.lock" {
            continue;
        }
        let p = e.path();
        if e.file_type()?.is_dir() {
            walk(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

/// Scan every file under `root`.
pub fn scan(root: &Path, banned: &Words) -> std::io::Result<(usize, Vec<Finding>)> {
    let mut files = Vec::new();
    walk(root, &mut files)?;
    let mut out = Vec::new();
    let mut count = 0;
    for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        let name = f.file_name().and_then(|e| e.to_str()).unwrap_or("");
        let scanned = EXTS.contains(&ext);
        let bytes = std::fs::read(&f)?;
        let Ok(text) = String::from_utf8(bytes) else {
            if scanned {
                out.push(Finding {
                    file: rel,
                    line: 0,
                    message: "not UTF-8".into(),
                });
            }
            continue;
        };
        count += 1;
        if let Some(l) = banned.pattern_in(&rel) {
            out.push(Finding {
                file: rel.clone(),
                line: 0,
                message: format!("banned pattern match `{l}` in the file name"),
            });
        }
        // Files of any other kind still get the word and path scan.
        out.extend(check_text(
            &rel,
            if scanned { ext } else { "" },
            &text,
            banned,
        ));
        if ext == "rs" && (name == "main.rs" || name == "lib.rs") {
            out.extend(check_crate_root(&rel, &text));
        }
    }
    Ok((count, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Words {
        Words::default()
    }

    #[test]
    fn word_lists_parse() {
        let w = Words::parse("# a comment\n\nfrobnicate\nwidget maker\n=Zork\n");
        assert_eq!(w.count(), 3);
        assert_eq!(w.words, vec!["frobnicate".to_string()]);
        assert_eq!(w.phrases, vec!["widget maker".to_string()]);
        assert_eq!(w.exact, vec!["Zork".to_string()]);
    }

    #[test]
    fn banned_words_are_found_whole() {
        let w = Words::parse("frobnicate\nwidget maker\n=Zork\n");
        assert_eq!(check_text("x.js", "", "// see FROBNICATE", &w).len(), 1);
        assert!(check_text("x.js", "", "// see frobnicateish", &w).is_empty());
        assert_eq!(check_text("x.js", "", "// a Widget Maker", &w).len(), 1);
        assert_eq!(check_text("x.js", "", "// Zork here", &w).len(), 1);
        assert!(check_text("x.js", "", "// zork here", &w).is_empty());
        assert!(check_text("x.js", "", "// see FROBNICATE", &none()).is_empty());
    }

    #[test]
    fn header_is_required() {
        let ok = "// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n// Copyright (c) 2026 Buntato\nlet x = 1;\n";
        assert!(check_text("a.js", "js", ok, &none()).is_empty());
        assert_eq!(check_text("a.js", "js", "let x = 1;\n", &none()).len(), 1);
        let sh = "#!/bin/sh\n# SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial\n# Copyright (c) 2026 Buntato\n";
        assert!(check_text("a.sh", "sh", sh, &none()).is_empty());
    }

    #[test]
    fn patterns_are_found() {
        let w = Words::parse("@Q#\n@Q##\n@step #\n@Z+\n");
        assert_eq!(w.count(), 4);
        assert_eq!(w.pattern_in("after Q4 lands").as_deref(), Some("Q4"));
        assert_eq!(w.pattern_in("see (Q12)").as_deref(), Some("Q12"));
        assert_eq!(w.pattern_in("the step 5 notes").as_deref(), Some("step 5"));
        assert_eq!(w.pattern_in("the Z+ part").as_deref(), Some("Z+"));
        assert_eq!(
            w.pattern_in("a Q; Q123; xQ1; Q1x; Z++ Z+= Z+c; Step 5"),
            None
        );
        let f = check_text("a.md", "", "after Q4 lands", &w);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].message, "banned pattern match `Q4`");
    }

    #[test]
    fn lists_are_read_from_files() {
        let dir = std::env::temp_dir().join(format!("house-words-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "# words\nfrobnicate\n").unwrap();
        std::fs::write(&b, "@Q#\n=Zork\n").unwrap();
        let spec = std::env::join_paths([&a, &b]).unwrap();
        let w = Words::read(&spec).unwrap();
        assert_eq!(w.count(), 3);
        assert_eq!(
            check_text("x.js", "", "// Zork, frobnicate, Q7", &w).len(),
            3
        );
        assert!(Words::read(std::ffi::OsStr::new("no-such-house-words-file")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn marker_and_paths_are_found() {
        let m = ["un", "safe"].concat();
        let line = format!("{m} {{ }}\n");
        assert_eq!(check_text("a.rs", "", &line, &none()).len(), 0);
        assert_eq!(check_text("a.rs", "rs", &line, &none()).len(), 2);
        let p = format!("see {}x", machine_paths()[4]);
        assert_eq!(check_text("a.md", "", &p, &none()).len(), 1);
    }
}
