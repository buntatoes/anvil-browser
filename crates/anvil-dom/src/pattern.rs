// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Bounded `pattern` matcher for form controls.
//!
//! The value must match the whole pattern. This is not a regular-expression
//! engine. A pattern this parser does not support is ignored, and the control
//! stays valid.
//!
//! Supported: literals, `.` (not CR or LF), greedy `*`, `+`, `?`, `{n}`,
//! `{n,m}`, and `{n,}` with bounds at most 64, classes `[abc]`, `[a-z]`,
//! `[^...]`, escapes `\d \D \w \W \s \S` and escaped punctuation, grouping
//! `()` and `(?:)`, alternation `|`, and `^` / `$` anchors.
//!
//! Not supported, and treated as no pattern: backreferences, lookahead,
//! lookbehind, lazy quantifiers, word boundaries, unicode properties, named
//! groups, a pattern longer than 256 characters, a quantifier above 64, and
//! a match that would exceed 8,192 steps.

/// Result of comparing one control value with its `pattern` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternCheck {
    /// Empty value, or a pattern this matcher does not implement.
    Skip,
    /// The whole value matches.
    Matched,
    /// The pattern is supported and the value does not match.
    Failed,
}

/// Compare `value` with `pattern`.
///
/// An empty `value` is [`PatternCheck::Skip`]. The caller still applies
/// `required` on its own.
pub fn check_pattern(pattern: &str, value: &str) -> PatternCheck {
    if value.is_empty() {
        return PatternCheck::Skip;
    }
    if pattern.chars().count() > 256 {
        return PatternCheck::Skip;
    }
    let Some(expr) = compile(pattern) else {
        return PatternCheck::Skip;
    };
    let chars: Vec<char> = value.chars().collect();
    let mut fuel = 8192u32;
    match exec(&expr, &chars, 0, &mut fuel) {
        Exec::Positions(positions) if positions.contains(&chars.len()) => PatternCheck::Matched,
        Exec::Fuel => PatternCheck::Skip,
        Exec::Positions(_) => PatternCheck::Failed,
    }
}

enum Expr {
    Empty,
    Lit(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Seq(Vec<Expr>),
    Alt(Vec<Expr>),
    Repeat {
        inner: Box<Expr>,
        min: u8,
        max: u8,
    },
    Start,
    End,
}

enum ClassItem {
    Char(char),
    Range(char, char),
    Digit,
    Word,
    Space,
    NotDigit,
    NotWord,
    NotSpace,
}

enum Exec {
    Positions(Vec<usize>),
    Fuel,
}

struct Parser<'a> {
    chars: &'a [char],
    index: usize,
    nodes: usize,
}

fn compile(pattern: &str) -> Option<Expr> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut parser = Parser {
        chars: &chars,
        index: 0,
        nodes: 0,
    };
    let expr = parser.parse_alt()?;
    if parser.index != parser.chars.len() {
        return None;
    }
    Some(expr)
}

impl<'a> Parser<'a> {
    fn bump(&mut self) -> Option<Expr> {
        if self.nodes >= 128 {
            return None;
        }
        self.nodes += 1;
        Some(Expr::Empty)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn parse_alt(&mut self) -> Option<Expr> {
        let mut items = vec![self.parse_seq()?];
        while self.eat('|') {
            items.push(self.parse_seq()?);
        }
        let _ = self.bump()?;
        if items.len() == 1 {
            items.pop()
        } else {
            Some(Expr::Alt(items))
        }
    }

    fn parse_seq(&mut self) -> Option<Expr> {
        let mut items = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            items.push(self.parse_piece()?);
        }
        let _ = self.bump()?;
        if items.is_empty() {
            Some(Expr::Empty)
        } else if items.len() == 1 {
            items.pop()
        } else {
            Some(Expr::Seq(items))
        }
    }

    fn parse_piece(&mut self) -> Option<Expr> {
        let atom = self.parse_atom()?;
        let (min, max) = self.parse_quant()?;
        let _ = self.bump()?;
        if min == 1 && max == 1 {
            Some(atom)
        } else {
            Some(Expr::Repeat {
                inner: Box::new(atom),
                min,
                max,
            })
        }
    }

    fn parse_quant(&mut self) -> Option<(u8, u8)> {
        let Some(ch) = self.peek() else {
            return Some((1, 1));
        };
        let bounds = match ch {
            '*' => (0, 64),
            '+' => (1, 64),
            '?' => (0, 1),
            '{' => return self.parse_brace(),
            _ => return Some((1, 1)),
        };
        self.index += 1;
        if self.peek() == Some('?') {
            return None;
        }
        Some(bounds)
    }

    fn parse_brace(&mut self) -> Option<(u8, u8)> {
        self.index += 1;
        let start = self.read_u8()?;
        let bounds = if self.eat(',') {
            if self.peek() == Some('}') {
                (start, 64)
            } else {
                let end = self.read_u8()?;
                if end < start || end > 64 {
                    return None;
                }
                (start, end)
            }
        } else if start > 64 {
            return None;
        } else {
            (start, start)
        };
        if !self.eat('}') || self.peek() == Some('?') {
            return None;
        }
        Some(bounds)
    }

    fn read_u8(&mut self) -> Option<u8> {
        let start = self.index;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.index += 1;
        }
        if start == self.index {
            return None;
        }
        self.chars[start..self.index]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn parse_atom(&mut self) -> Option<Expr> {
        let ch = self.peek()?;
        self.index += 1;
        let _ = self.bump()?;
        match ch {
            '.' => Some(Expr::Any),
            '^' => Some(Expr::Start),
            '$' => Some(Expr::End),
            '\\' => self.parse_escape(),
            '(' => self.parse_group(),
            '[' => self.parse_class(),
            '*' | '+' | '?' | '{' | ')' | '|' => None,
            other => Some(Expr::Lit(other)),
        }
    }

    fn parse_group(&mut self) -> Option<Expr> {
        if self.chars.get(self.index) == Some(&'?') {
            if self.chars.get(self.index + 1) == Some(&':') {
                self.index += 2;
            } else {
                return None;
            }
        }
        let inner = self.parse_alt()?;
        if !self.eat(')') {
            return None;
        }
        Some(inner)
    }

    fn parse_escape(&mut self) -> Option<Expr> {
        let ch = self.peek()?;
        self.index += 1;
        if ch.is_ascii_digit() {
            return None;
        }
        match ch {
            'd' => Some(class_escape(false, ClassItem::Digit)),
            'D' => Some(class_escape(false, ClassItem::NotDigit)),
            'w' => Some(class_escape(false, ClassItem::Word)),
            'W' => Some(class_escape(false, ClassItem::NotWord)),
            's' => Some(class_escape(false, ClassItem::Space)),
            'S' => Some(class_escape(false, ClassItem::NotSpace)),
            'b' | 'B' | 'p' | 'u' | 'k' => None,
            'n' => Some(Expr::Lit('\n')),
            'r' => Some(Expr::Lit('\r')),
            't' => Some(Expr::Lit('\t')),
            other => Some(Expr::Lit(other)),
        }
    }

    fn parse_class(&mut self) -> Option<Expr> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            let ch = self.peek()?;
            if ch == ']' && !items.is_empty() {
                self.index += 1;
                break;
            }
            self.index += 1;
            if ch == '\\' {
                let esc = self.peek()?;
                self.index += 1;
                items.push(match esc {
                    'd' => ClassItem::Digit,
                    'D' => ClassItem::NotDigit,
                    'w' => ClassItem::Word,
                    'W' => ClassItem::NotWord,
                    's' => ClassItem::Space,
                    'S' => ClassItem::NotSpace,
                    'n' => ClassItem::Char('\n'),
                    'r' => ClassItem::Char('\r'),
                    't' => ClassItem::Char('\t'),
                    other => ClassItem::Char(other),
                });
                continue;
            }
            if self.peek() == Some('-') && self.chars.get(self.index + 1).is_some_and(|n| *n != ']')
            {
                self.index += 1;
                let end = self.peek()?;
                self.index += 1;
                if end < ch {
                    return None;
                }
                items.push(ClassItem::Range(ch, end));
            } else {
                items.push(ClassItem::Char(ch));
            }
        }
        let _ = self.bump()?;
        Some(Expr::Class { negated, items })
    }
}

fn class_escape(negated: bool, item: ClassItem) -> Expr {
    Expr::Class {
        negated,
        items: vec![item],
    }
}

fn exec(expr: &Expr, input: &[char], index: usize, fuel: &mut u32) -> Exec {
    if *fuel == 0 {
        return Exec::Fuel;
    }
    *fuel -= 1;
    match expr {
        Expr::Empty => Exec::Positions(vec![index]),
        Expr::Lit(ch) => {
            if input.get(index) == Some(ch) {
                Exec::Positions(vec![index + 1])
            } else {
                Exec::Positions(Vec::new())
            }
        }
        Expr::Any => match input.get(index) {
            Some(ch) if *ch != '\n' && *ch != '\r' => Exec::Positions(vec![index + 1]),
            _ => Exec::Positions(Vec::new()),
        },
        Expr::Start => {
            if index == 0 {
                Exec::Positions(vec![index])
            } else {
                Exec::Positions(Vec::new())
            }
        }
        Expr::End => {
            if index == input.len() {
                Exec::Positions(vec![index])
            } else {
                Exec::Positions(Vec::new())
            }
        }
        Expr::Class { negated, items } => {
            let Some(ch) = input.get(index).copied() else {
                return Exec::Positions(Vec::new());
            };
            let hit = items.iter().any(|item| class_hit(item, ch));
            if hit != *negated {
                Exec::Positions(vec![index + 1])
            } else {
                Exec::Positions(Vec::new())
            }
        }
        Expr::Seq(items) => {
            let mut current = vec![index];
            for item in items {
                let mut next = Vec::new();
                for at in current {
                    match exec(item, input, at, fuel) {
                        Exec::Positions(positions) => next.extend(positions),
                        Exec::Fuel => return Exec::Fuel,
                    }
                }
                next.sort_unstable();
                next.dedup();
                current = next;
            }
            Exec::Positions(current)
        }
        Expr::Alt(items) => {
            let mut all = Vec::new();
            for item in items {
                match exec(item, input, index, fuel) {
                    Exec::Positions(positions) => all.extend(positions),
                    Exec::Fuel => return Exec::Fuel,
                }
            }
            all.sort_unstable();
            all.dedup();
            Exec::Positions(all)
        }
        Expr::Repeat { inner, min, max } => {
            let mut reached = vec![vec![index]];
            for _ in 0..*max {
                let mut next = Vec::new();
                for at in reached.last().unwrap() {
                    match exec(inner, input, *at, fuel) {
                        Exec::Positions(positions) => {
                            for position in positions {
                                if position > *at {
                                    next.push(position);
                                }
                            }
                        }
                        Exec::Fuel => return Exec::Fuel,
                    }
                }
                next.sort_unstable();
                next.dedup();
                if next.is_empty() {
                    break;
                }
                reached.push(next);
            }
            let mut all = Vec::new();
            for (count, positions) in reached.iter().enumerate() {
                if count >= *min as usize {
                    all.extend(positions);
                }
            }
            all.sort_unstable();
            all.dedup();
            Exec::Positions(all)
        }
    }
}

fn class_hit(item: &ClassItem, ch: char) -> bool {
    match item {
        ClassItem::Char(expected) => ch == *expected,
        ClassItem::Range(start, end) => ch >= *start && ch <= *end,
        ClassItem::Digit => ch.is_ascii_digit(),
        ClassItem::Word => ch.is_ascii_alphanumeric() || ch == '_',
        ClassItem::Space => ch.is_whitespace(),
        ClassItem::NotDigit => !ch.is_ascii_digit(),
        ClassItem::NotWord => !(ch.is_ascii_alphanumeric() || ch == '_'),
        ClassItem::NotSpace => !ch.is_whitespace(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_literals_and_quantifiers() {
        assert_eq!(check_pattern("a+", "b"), PatternCheck::Failed);
        assert_eq!(check_pattern("a+", "aaa"), PatternCheck::Matched);
        assert_eq!(check_pattern("a+", "aaab"), PatternCheck::Failed);
        assert_eq!(check_pattern("a+", ""), PatternCheck::Skip);
        assert_eq!(check_pattern("a{1,2}", "aa"), PatternCheck::Matched);
        assert_eq!(check_pattern("a{1,2}", "aaa"), PatternCheck::Failed);
        assert_eq!(check_pattern("a*b", "aaab"), PatternCheck::Matched);
        assert_eq!(check_pattern("^a+$", "aaa"), PatternCheck::Matched);
    }

    #[test]
    fn pattern_a_plus_does_not_match_b_on_the_control() {
        // HTML compiles `pattern` as `^(?:` + pattern + `)$` with the `v` flag.
        // `a+` against `b` does not match, so the control is pattern-mismatched.
        assert_eq!(check_pattern("a+", "b"), PatternCheck::Failed);
        let doc = crate::parse_html("<form><input id=\"q\" pattern=\"a+\" value=\"b\"></form>");
        let id = doc.get_element_by_id("q").unwrap();
        assert_eq!(doc.is_control_valid(id), Some(false));
    }

    #[test]
    fn unsupported_pattern_is_not_a_constraint() {
        assert_eq!(check_pattern("(?=a)", "x"), PatternCheck::Skip);
        assert_eq!(check_pattern(r"\1", "1"), PatternCheck::Skip);
        assert_eq!(check_pattern("a*?", "a"), PatternCheck::Skip);
    }
}
