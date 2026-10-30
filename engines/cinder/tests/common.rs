// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Helpers shared by the integration tests.

#![forbid(unsafe_code)]
#![allow(dead_code)]

use cinder::ast::Program;
use cinder::{ParseOptions, SyntaxError, parse_module, parse_script};

pub fn script(src: &str) -> Program {
    match parse_script(src, &ParseOptions::default()) {
        Ok(p) => p,
        Err(e) => panic!("script should parse: {src:?}\n  error: {e}"),
    }
}

pub fn module(src: &str) -> Program {
    match parse_module(src, &ParseOptions::default()) {
        Ok(p) => p,
        Err(e) => panic!("module should parse: {src:?}\n  error: {e}"),
    }
}

pub fn script_err(src: &str) -> SyntaxError {
    match parse_script(src, &ParseOptions::default()) {
        Ok(p) => panic!("script should fail: {src:?}\n  got: {:?}", p.body),
        Err(e) => e,
    }
}

pub fn module_err(src: &str) -> SyntaxError {
    match parse_module(src, &ParseOptions::default()) {
        Ok(p) => panic!("module should fail: {src:?}\n  got: {:?}", p.body),
        Err(e) => e,
    }
}
