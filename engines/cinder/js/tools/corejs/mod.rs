// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! The Core JS front end shared by the stand-in host and the bundler:
//! tokenizer, parser, scope analysis and the subset and level checks. Each
//! tool includes these files as its own module; there is no shared crate.

pub mod ast;
pub mod check;
pub mod lexer;
pub mod num;
pub mod parser;
pub mod scope;

/// Parse, scope-analyse, and Core JS verify `src` at `LEVEL_ALL`.
pub fn verify(file: &str, src: &str) -> Result<(), String> {
    let po = parser::parse(src, false, parser::DEFAULT_MAX_DEPTH);
    let prog = po
        .program
        .map_err(|e| format!("{}:{}:{}: {}", file, e.line, e.col, e.message))?;
    let an = scope::analyse(&prog)
        .map_err(|e| format!("{}:{}:{}: {}", file, e.line, e.col, e.message))?;
    let mut diags = po.subset;
    diags.extend(check::check(&prog, &an, check::LEVEL_ALL, true));
    if let Some(d) = diags.first() {
        return Err(format!("{}:{}:{}: {}", file, d.line, d.col, d.message));
    }
    Ok(())
}
