// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Robustness checks for the lexer and parser.
//!
//! Inputs come from a seeded generator, so a failure names a case number
//! that reproduces on every run. Each test stops after a short time budget.
//! The parser may reject any of these inputs. It must not panic or hang.

#![forbid(unsafe_code)]

use anvil_js::lexer::{Goal, Lexer, TokenKind};
use anvil_js::{ParseOptions, parse_module, parse_script};

// ---- shared harness (same text in each repo's robustness test) ----

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Wall-clock budget for one robustness test. Cases run until it is spent.
/// Set `ANVIL_ROBUSTNESS_MS` for a longer local soak run.
fn test_budget() -> Duration {
    let millis = std::env::var("ANVIL_ROBUSTNESS_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1500);
    Duration::from_millis(millis)
}

/// One input that runs longer than this counts as a hang.
const CASE_LIMIT: Duration = Duration::from_secs(10);

/// Upper bound on cases per test, so a fast machine does not loop forever.
const MAX_CASES: usize = 200_000;

/// xorshift64 generator. A fixed seed gives the same inputs on every run.
struct Rng(u64);

#[allow(dead_code)] // Not every test file uses every helper.
impl Rng {
    fn new(seed: u64) -> Self {
        Rng((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Value in `0..n`. Returns 0 when `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }

    fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len + 1);
        (0..len).map(|_| self.byte()).collect()
    }
}

/// Apply 1 to 8 random edits: bit flips, byte overwrites, boundary values,
/// inserts, deletes, duplicated runs, and truncation.
fn mutate(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut out = seed.to_vec();
    let edits = 1 + rng.below(8);
    for _ in 0..edits {
        let len = out.len();
        match rng.below(8) {
            0 if len > 0 => {
                let i = rng.below(len);
                out[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let i = rng.below(len);
                out[i] = rng.byte();
            }
            2 if len > 0 => {
                let i = rng.below(len);
                out[i] = rng.pick(&[0x00, 0x01, 0x7F, 0x80, 0xFE, 0xFF]);
            }
            3 if len >= 4 => {
                let i = rng.below(len - 3);
                let v = rng.pick(&[0u32, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xFFFF]);
                out[i..i + 4].copy_from_slice(&v.to_be_bytes());
            }
            4 => {
                let i = rng.below(len + 1);
                let extra = rng.bytes(16);
                out.splice(i..i, extra);
            }
            5 if len > 0 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(32));
                out.drain(i..i + n);
            }
            6 if len > 0 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(64));
                let run = out[i..i + n].to_vec();
                let at = rng.below(len + 1);
                out.splice(at..at, run);
            }
            _ => {
                let keep = rng.below(len + 1);
                out.truncate(keep);
            }
        }
    }
    out
}

fn preview(input: &[u8]) -> String {
    let shown: Vec<String> = input.iter().take(96).map(|b| format!("{b:02x}")).collect();
    let more = if input.len() > 96 { " ..." } else { "" };
    format!("{} bytes: {}{}", input.len(), shown.join(" "), more)
}

/// Run `target` on `input` in its own thread. Fails the test on a panic or
/// when the case runs past [`CASE_LIMIT`]. A hung thread is left behind and
/// ends with the test process.
fn check_case(label: &str, case: usize, input: Vec<u8>, target: fn(&[u8])) {
    let (done_tx, done_rx) = mpsc::channel();
    let shared = std::sync::Arc::new(input);
    let worker_input = shared.clone();
    let handle = thread::Builder::new()
        .name(format!("{label}-{case}"))
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            target(&worker_input);
            let _ = done_tx.send(());
        })
        .expect("spawn case thread");
    match done_rx.recv_timeout(CASE_LIMIT) {
        Ok(()) => {
            let _ = handle.join();
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            panic!(
                "{label}: case {case} panicked on input {}",
                preview(&shared)
            );
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!(
                "{label}: case {case} did not finish within {:?} on input {}",
                CASE_LIMIT,
                preview(&shared)
            );
        }
    }
}

/// Generate cases from `seed` until the budget is spent and check each one.
fn run_cases(
    label: &str,
    seed: u64,
    mut generate: impl FnMut(&mut Rng) -> Vec<u8>,
    target: fn(&[u8]),
) {
    let mut rng = Rng::new(seed);
    let budget = test_budget();
    let started = Instant::now();
    let mut case = 0;
    while case < MAX_CASES && started.elapsed() < budget {
        let input = generate(&mut rng);
        check_case(label, case, input, target);
        case += 1;
    }
    eprintln!("{label}: {case} cases in {:?}", started.elapsed());
}

/// Token-ish fragments that hit every corner of the grammar.
const TOKENS: &[&str] = &[
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ";",
    ",",
    ".",
    "...",
    "?.",
    "?",
    ":",
    "=",
    "=>",
    "+",
    "-",
    "*",
    "**",
    "/",
    "/=",
    "%",
    "++",
    "--",
    "<<",
    ">>>",
    "<",
    ">=",
    "==",
    "===",
    "!",
    "~",
    "&&",
    "||",
    "??",
    "??=",
    "&&=",
    "|=",
    " ",
    "\n",
    "\r\n",
    "\u{2028}",
    "/*c*/",
    "//c\n",
    "a",
    "b",
    "x1",
    "$",
    "_",
    "café",
    "\\u0061",
    "\\u{62}",
    "#p",
    "0",
    "1.5",
    ".5",
    "0x1F",
    "0b1",
    "0o7",
    "07",
    "08",
    "1_0",
    "10n",
    "1e3",
    "'s'",
    "\"d\"",
    "'\\x41'",
    "'\\u{1F600}'",
    "'\\07'",
    "`t`",
    "`a${",
    "}b`",
    "`${x}`",
    "/re/g",
    "/[/]/",
    "var",
    "let",
    "const",
    "function",
    "function*",
    "async",
    "await",
    "yield",
    "class",
    "extends",
    "super",
    "new",
    "this",
    "null",
    "true",
    "if",
    "else",
    "for",
    "of",
    "in",
    "while",
    "do",
    "break",
    "continue",
    "return",
    "switch",
    "case",
    "default",
    "try",
    "catch",
    "finally",
    "throw",
    "with",
    "debugger",
    "import",
    "export",
    "from",
    "as",
    "static",
    "get",
    "set",
    "typeof",
    "delete",
    "void",
    "instanceof",
    "target",
    "meta",
    "'use strict';",
    "<!--",
    "-->",
    "#!",
    "\\",
    "@",
    "\u{0}",
];

/// Snippets of valid code to mutate.
const SEEDS: &[&str] = &[
    "function f(a, b = 1, ...c) { return a + b * c.length; }",
    "class A extends B { #x = 1; static { this.y = 2; } get z() { return this.#x; } constructor() { super(); } }",
    "for (const [k, v] of Object.entries(o)) { if (!v) continue; out[k] = v ?? null; }",
    "const {a, b: {c = 2}, ...rest} = obj; [x, y] = [y, x];",
    "async function* g() { for await (const x of src) yield* inner(x); }",
    "label: for (;;) { switch (n) { case 1: break label; default: n--; } }",
    "let s = `a${b}c${`d${e}f`}g`; tag`x\\u{zz}`; /a[/]b/gi.test(s) / 2;",
    "try { throw new Error('x'); } catch ({message}) { console.log(message); } finally {}",
    "x = a ? b : c ? d : e, y = (p, q) => ({p, q}), z = async v => await v;",
    "import def, {a as b} from 'm'; export default class {} export {b as 'c'};",
    "if (a) function f() {} else { var g = function* () { yield; }; }",
    "o?.a?.[b]?.(c); new A.B(...args); a ||= b &&= c ??= d; 2 ** -x;",
];

fn lex_all(input: &[u8]) {
    let text = String::from_utf8_lossy(input);
    for goal in [Goal::Script, Goal::Module] {
        let mut lx = Lexer::new(&text, goal);
        // Each successful token moves forward, so this bound can't be hit
        // by a working lexer; it is there to turn a stall into a failure.
        let mut budget = text.len() + 2;
        loop {
            let before = lx.save();
            match lx.next_token() {
                Ok(t) if t.kind == TokenKind::Eof => break,
                Ok(t) => {
                    if t.span.is_empty() {
                        panic!("empty token at {}", t.span.start);
                    }
                    // Exercise the parser-driven rescans too.
                    if matches!(t.kind, TokenKind::Punct(anvil_js::lexer::Punct::Slash)) {
                        let mut copy = lx.clone();
                        copy.restore(before);
                        let _ = copy.rescan_regex(&t);
                    }
                    if matches!(t.kind, TokenKind::Punct(anvil_js::lexer::Punct::RBrace)) {
                        let copy = lx.clone();
                        let mut c = copy;
                        let _ = c.rescan_template_continuation(&t);
                    }
                }
                Err(_) => break,
            }
            budget -= 1;
            assert!(budget > 0, "lexer did not make progress");
        }
    }
}

fn parse_both(input: &[u8]) {
    let text = String::from_utf8_lossy(input);
    let opts = ParseOptions {
        max_depth: 200,
        ..ParseOptions::default()
    };
    // A panic inside the parser comes back as "internal parser error"
    // instead of unwinding, so look for that message explicitly.
    for r in [parse_script(&text, &opts), parse_module(&text, &opts)] {
        if let Err(e) = r {
            assert!(e.message != "internal parser error", "parser panicked");
        }
    }
}

fn token_soup(rng: &mut Rng) -> Vec<u8> {
    let n = rng.below(40);
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(rng.pick(TOKENS));
        if rng.one_in(3) {
            s.push(' ');
        }
    }
    s.into_bytes()
}

fn mutated_seed(rng: &mut Rng) -> Vec<u8> {
    let seed = rng.pick(SEEDS);
    mutate(rng, seed.as_bytes())
}

fn spliced_seeds(rng: &mut Rng) -> Vec<u8> {
    let a = rng.pick(SEEDS).as_bytes();
    let b = rng.pick(SEEDS).as_bytes();
    let i = rng.below(a.len() + 1);
    let j = rng.below(b.len() + 1);
    let mut out = a[..i].to_vec();
    out.extend_from_slice(&b[j..]);
    out
}

#[test]
fn lexer_survives_random_bytes() {
    run_cases("lex-bytes", 1, |r| r.bytes(64), lex_all);
}

#[test]
fn lexer_survives_token_soup() {
    run_cases("lex-soup", 2, token_soup, lex_all);
}

#[test]
fn parser_survives_random_bytes() {
    run_cases("parse-bytes", 3, |r| r.bytes(64), parse_both);
}

#[test]
fn parser_survives_token_soup() {
    run_cases("parse-soup", 4, token_soup, parse_both);
}

#[test]
fn parser_survives_mutated_code() {
    run_cases("parse-mutated", 5, mutated_seed, parse_both);
}

#[test]
fn parser_survives_spliced_code() {
    run_cases("parse-spliced", 6, spliced_seeds, parse_both);
}

#[test]
fn seeds_are_valid() {
    for (i, s) in SEEDS.iter().enumerate() {
        let r = if s.starts_with("import") {
            parse_module(s, &ParseOptions::default())
        } else {
            parse_script(s, &ParseOptions::default())
        };
        if let Err(e) = r {
            panic!("seed {i} should parse: {e}");
        }
    }
}
