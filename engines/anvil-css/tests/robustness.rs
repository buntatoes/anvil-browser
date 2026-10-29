// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Robustness checks for the CSS tokenizer, parser, selectors, values, and
//! conditions.
//!
//! Inputs come from a seeded generator, so a failure names a case number
//! that reproduces on every run. Each test stops after a short time budget.
//! Any input may produce diagnostics. Nothing may panic or hang, and
//! serialized component values must read back as the same tokens.

mod common;

use anvil_css::media::{MediaEnv, parse_media_query_list_str};
use anvil_css::parser::parse_component_values;
use anvil_css::serialize::to_css;
use anvil_css::tokenizer::{Token, Tokenizer};
use anvil_css::{
    Options, Rule, parse_declarations, parse_selector_list, parse_stylesheet, parse_value,
};

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
        .stack_size(64 * 1024 * 1024)
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

const TOKENS: &[&str] = &[
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ";",
    ":",
    ",",
    " ",
    "\n",
    "\t",
    "'",
    "\"",
    "\\",
    "/*",
    "*/",
    "<!--",
    "-->",
    "@media",
    "@supports",
    "@import",
    "@layer",
    "@container",
    "@keyframes",
    "@font-face",
    "@page",
    "@namespace",
    "@x-unknown",
    "a",
    "div",
    "p",
    ".c",
    "#id",
    "*",
    ">",
    "+",
    "~",
    "&",
    "|",
    "[x]",
    "[x=y i]",
    ":hover",
    ":is(",
    ":not(",
    ":has(",
    ":where(",
    ":nth-child(",
    "2n+1",
    "of",
    "::before",
    ":before",
    "color",
    "width",
    "--x",
    "red",
    "#fff",
    "rgb(",
    "hsl(",
    "oklch(",
    "calc(",
    "min(",
    "clamp(",
    "var(",
    "env(",
    "url(",
    "url(a b)",
    "10px",
    "1e999",
    "-0",
    ".5",
    "50%",
    "1.5em",
    "!important",
    "screen",
    "print",
    "and",
    "or",
    "not",
    "only",
    "(min-width: 1px)",
    "(width < 10px)",
    "U+0-7F",
    "u+4??",
    "from",
    "to",
    "\u{0}",
    "\u{e9}",
    "\u{1F600}",
    "\u{FEFF}",
    "\r\n",
];

/// Token soup in CSS's alphabet, so most inputs reach the parser's deeper
/// paths.
fn token_soup(rng: &mut Rng) -> Vec<u8> {
    let mut out = String::new();
    let n = rng.below(80);
    for _ in 0..n {
        out.push_str(rng.pick(TOKENS));
        if rng.one_in(3) {
            out.push(' ');
        }
    }
    out.into_bytes()
}

const SEEDS: &[&str] = &[
    "a { color: red; margin: 0 auto !important }",
    "@media screen and (min-width: 600px) { .a > .b:hover::before { content: '}' } }",
    "@supports (display: grid) and (not (x: y)) { @layer base { p { --x: { a; b } } } }",
    "@import url(a.css) layer(x) supports(display: flex) print;",
    "@keyframes k { from { opacity: 0 } 50% { opacity: .5 } to { opacity: 1 } }",
    "@font-face { font-family: X; src: url(x.woff2) format('woff2'); unicode-range: U+0-7F }",
    ".a { &:hover { color: blue } > .b { top: 0 } @media print { left: 0 } }",
    ":is(a, b) :where(.c) :not(#d) :has(> e + f) :nth-child(2n+1 of .g) { width: calc(1px + 2em * 3) }",
    "@container card (inline-size > 30em) { a { color: oklch(0.5 0.1 200 / 50%) } }",
    "@page :first { margin: 1in; @top-center { content: 'x' } }",
    "@namespace svg url(http://www.w3.org/2000/svg); svg|rect { fill: red }",
    "a[href$='.pdf' i], a[x|=y] { background: url(\"a b\") no-repeat, linear-gradient(red, blue) }",
];

fn mutated_seed(rng: &mut Rng) -> Vec<u8> {
    let seed = rng.pick(SEEDS);
    mutate(rng, seed.as_bytes())
}

fn random_bytes(rng: &mut Rng) -> Vec<u8> {
    rng.bytes(256)
}

fn deep_nesting(rng: &mut Rng) -> Vec<u8> {
    let opens = [
        "{",
        "(",
        "[",
        "a{",
        "@media screen{",
        ":is(",
        "calc(",
        "f(",
        "((",
        "@supports ((",
    ];
    let open = rng.pick(&opens);
    let n = rng.below(3000);
    let mut s = open.repeat(n);
    if rng.one_in(2) {
        s.push_str(&"})]".repeat(rng.below(n + 1)));
    }
    s.into_bytes()
}

fn walk(rules: &[Rule], out: &mut usize) {
    let tree = selector_tree();
    let env = MediaEnv::default();
    let mut stack: Vec<&Rule> = rules.iter().collect();
    while let Some(r) = stack.pop() {
        *out += 1;
        match r {
            Rule::Style(s) => {
                for d in &s.declarations {
                    let _ = d.parse_value();
                }
                for i in 0..tree.nodes.len() {
                    let _ = s.selectors.matches(&common::El::new(&tree, i));
                }
            }
            Rule::Media(m) => {
                let _ = m.queries.evaluate(&env);
            }
            Rule::Supports(s) => {
                let _ = s.condition.evaluate();
            }
            Rule::FontFace(f) => {
                for d in &f.declarations {
                    let _ = d.parse_value();
                }
            }
            _ => {}
        }
        stack.extend(r.children());
    }
}

fn selector_tree() -> std::rc::Rc<common::Tree> {
    let mut t = common::Tree::default();
    let root = t.add(None, "html");
    let body = t.add(Some(root), "body.c");
    let a = t.add(Some(body), "div#id.c");
    t.attr(a, "x", "y");
    let b = t.add(Some(a), "p");
    t.add(Some(a), "a");
    t.add(Some(b), "span");
    t.finish()
}

fn text(input: &[u8]) -> String {
    String::from_utf8_lossy(input).into_owned()
}

fn non_ws_tokens(src: &str) -> Vec<Token> {
    Tokenizer::new(src)
        .map(|t| t.token)
        .filter(|t| !t.is_whitespace())
        .collect()
}

fn target_stylesheet(input: &[u8]) {
    let src = text(input);
    let sheet = parse_stylesheet(&src, &Options::default()).expect("default limits are large");
    for d in &sheet.diagnostics {
        assert!(
            d.span.start <= d.span.end && d.span.end <= src.len(),
            "{d:?}"
        );
    }
    let mut count = 0;
    walk(&sheet.rules, &mut count);
}

fn target_pieces(input: &[u8]) {
    let src = text(input);
    let _ = parse_declarations(&src);
    let _ = parse_selector_list(&src);
    let _ = parse_value("width", &src);
    let _ = parse_value("color", &src);
    let _ = parse_value("--x", &src);
    let _ = parse_media_query_list_str(&src).evaluate(&MediaEnv::default());
}

fn target_round_trip(input: &[u8]) {
    let src = text(input);
    let Ok(values) = parse_component_values(&src, &Options::default()) else {
        return;
    };
    let out = to_css(&values);
    // Bad strings read back with a trailing newline token, and blocks left
    // open get closed; compare only inputs without those.
    let original = non_ws_tokens(&src);
    if original.iter().any(|t| matches!(t, Token::BadString)) {
        return;
    }
    let balanced = {
        let mut depth: Vec<char> = Vec::new();
        let mut ok = true;
        for t in &original {
            match t {
                Token::LeftParen | Token::Function(_) => depth.push(')'),
                Token::LeftBracket => depth.push(']'),
                Token::LeftBrace => depth.push('}'),
                Token::RightParen | Token::RightBracket | Token::RightBrace => {
                    let c = match t {
                        Token::RightParen => ')',
                        Token::RightBracket => ']',
                        _ => '}',
                    };
                    if depth.last() == Some(&c) {
                        depth.pop();
                    } else {
                        ok = false;
                    }
                }
                _ => {}
            }
        }
        ok && depth.is_empty()
    };
    if balanced {
        assert_eq!(non_ws_tokens(&out), original, "written as {out:?}");
    }
}

#[test]
fn stylesheet_token_soup() {
    run_cases("stylesheet-soup", 1, token_soup, target_stylesheet);
}

#[test]
fn stylesheet_mutated_seeds() {
    run_cases("stylesheet-mutated", 2, mutated_seed, target_stylesheet);
}

#[test]
fn stylesheet_random_bytes() {
    run_cases("stylesheet-bytes", 3, random_bytes, target_stylesheet);
}

#[test]
fn stylesheet_deep_nesting() {
    run_cases("stylesheet-deep", 4, deep_nesting, target_stylesheet);
}

#[test]
fn pieces_token_soup() {
    run_cases("pieces-soup", 5, token_soup, target_pieces);
}

#[test]
fn pieces_deep_nesting() {
    run_cases("pieces-deep", 6, deep_nesting, target_pieces);
}

#[test]
fn round_trip_token_soup() {
    run_cases("round-trip-soup", 7, token_soup, target_round_trip);
}

#[test]
fn round_trip_mutated_seeds() {
    run_cases("round-trip-mutated", 8, mutated_seed, target_round_trip);
}
