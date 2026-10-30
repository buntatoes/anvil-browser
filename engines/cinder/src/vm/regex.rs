// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Built-in ECMAScript regular expression parser and backtracking matcher.

const MAX_MATCH_STEPS: usize = 100_000;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ReFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub sticky: bool,
}

impl ReFlags {
    pub fn parse(raw: &str) -> Result<(Self, String), String> {
        let mut f = Self::default();
        for ch in raw.chars() {
            match ch {
                'g' if !f.global => f.global = true,
                'i' if !f.ignore_case => f.ignore_case = true,
                'm' if !f.multiline => f.multiline = true,
                's' if !f.dot_all => f.dot_all = true,
                'u' if !f.unicode => f.unicode = true,
                'y' if !f.sticky => f.sticky = true,
                'd' | 'v' => {}
                _ => return Err(format!("Invalid regular expression flags: '{raw}'")),
            }
        }
        let mut canonical = String::new();
        if f.global {
            canonical.push('g');
        }
        if f.ignore_case {
            canonical.push('i');
        }
        if f.multiline {
            canonical.push('m');
        }
        if f.dot_all {
            canonical.push('s');
        }
        if f.unicode {
            canonical.push('u');
        }
        if f.sticky {
            canonical.push('y');
        }
        Ok((f, canonical))
    }
}

#[derive(Clone, Debug)]
enum ClassAtom {
    Single(char),
    Range(char, char),
    Builtin(char),
}

#[derive(Clone, Debug)]
enum ReAst {
    Empty,
    Lit(char),
    Dot,
    Builtin(char),
    Anchor(char),
    Class {
        negated: bool,
        atoms: Vec<ClassAtom>,
    },
    Seq(Vec<ReAst>),
    Alt(Vec<ReAst>),
    Group {
        index: Option<usize>,
        inner: Box<ReAst>,
    },
    Lookahead {
        positive: bool,
        inner: Box<ReAst>,
    },
    Backref(usize),
    Repeat {
        inner: Box<ReAst>,
        min: usize,
        max: Option<usize>,
        greedy: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledRegex {
    ast: ReAst,
    pub group_count: usize,
    pub flags: ReFlags,
}

#[derive(Clone, Debug)]
pub(crate) struct ReMatch {
    /// Character index in `chars` where the match starts.
    pub start_char: usize,
    /// Character index in `chars` where the match ends.
    pub end_char: usize,
    /// Captured group ranges `(start_char, end_char)` for groups `1..=group_count`.
    pub groups: Vec<Option<(usize, usize)>>,
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    groups: usize,
    _raw: &'a str,
}

impl<'a> Parser<'a> {
    fn new(pattern: &'a str) -> Self {
        Self {
            chars: pattern.chars().collect(),
            pos: 0,
            groups: 0,
            _raw: pattern,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn parse_top(&mut self) -> Result<ReAst, String> {
        let ast = self.parse_alt(false)?;
        if self.pos < self.chars.len() {
            return Err("Unexpected closing parenthesis in regular expression".to_string());
        }
        Ok(ast)
    }

    fn parse_alt(&mut self, in_group: bool) -> Result<ReAst, String> {
        let mut branches = vec![self.parse_seq(in_group)?];
        while self.peek() == Some('|') {
            self.next();
            branches.push(self.parse_seq(in_group)?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap_or(ReAst::Empty))
        } else {
            Ok(ReAst::Alt(branches))
        }
    }

    fn parse_seq(&mut self, in_group: bool) -> Result<ReAst, String> {
        let mut items = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || (in_group && ch == ')') {
                break;
            }
            if ch == ')' {
                return Err("Unmatched ')' in regular expression".to_string());
            }
            let atom = self.parse_atom()?;
            let quantified = self.parse_quantifier(atom)?;
            items.push(quantified);
        }
        match items.len() {
            0 => Ok(ReAst::Empty),
            1 => Ok(items.pop().unwrap()),
            _ => Ok(ReAst::Seq(items)),
        }
    }

    fn parse_atom(&mut self) -> Result<ReAst, String> {
        let Some(ch) = self.next() else {
            return Ok(ReAst::Empty);
        };
        match ch {
            '^' | '$' => Ok(ReAst::Anchor(ch)),
            '.' => Ok(ReAst::Dot),
            '[' => self.parse_class(),
            '(' => {
                if self.peek() == Some('?') {
                    self.next();
                    match self.peek() {
                        Some(':') => {
                            self.next();
                            let inner = self.parse_alt(true)?;
                            if self.next() != Some(')') {
                                return Err("Unterminated group in regular expression".to_string());
                            }
                            Ok(ReAst::Group {
                                index: None,
                                inner: Box::new(inner),
                            })
                        }
                        Some('=') => {
                            self.next();
                            let inner = self.parse_alt(true)?;
                            if self.next() != Some(')') {
                                return Err(
                                    "Unterminated lookahead in regular expression".to_string()
                                );
                            }
                            Ok(ReAst::Lookahead {
                                positive: true,
                                inner: Box::new(inner),
                            })
                        }
                        Some('!') => {
                            self.next();
                            let inner = self.parse_alt(true)?;
                            if self.next() != Some(')') {
                                return Err(
                                    "Unterminated negative lookahead in regular expression"
                                        .to_string(),
                                );
                            }
                            Ok(ReAst::Lookahead {
                                positive: false,
                                inner: Box::new(inner),
                            })
                        }
                        Some('<') => {
                            self.next();
                            while let Some(c) = self.next() {
                                if c == '>' {
                                    break;
                                }
                            }
                            self.groups += 1;
                            let idx = self.groups;
                            let inner = self.parse_alt(true)?;
                            if self.next() != Some(')') {
                                return Err("Unterminated named group".to_string());
                            }
                            Ok(ReAst::Group {
                                index: Some(idx),
                                inner: Box::new(inner),
                            })
                        }
                        _ => Err("Invalid group modifier in regular expression".to_string()),
                    }
                } else {
                    self.groups += 1;
                    let idx = self.groups;
                    let inner = self.parse_alt(true)?;
                    if self.next() != Some(')') {
                        return Err("Unterminated '(' in regular expression".to_string());
                    }
                    Ok(ReAst::Group {
                        index: Some(idx),
                        inner: Box::new(inner),
                    })
                }
            }
            '\\' => self.parse_escape(),
            '*' | '+' | '?' => Err(format!("Nothing to repeat before '{ch}'")),
            other => Ok(ReAst::Lit(other)),
        }
    }

    fn parse_escape(&mut self) -> Result<ReAst, String> {
        let Some(esc) = self.next() else {
            return Err("Trailing backslash in regular expression".to_string());
        };
        match esc {
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => Ok(ReAst::Builtin(esc)),
            'b' | 'B' => Ok(ReAst::Anchor(esc)),
            '1'..='9' => Ok(ReAst::Backref((esc as u8 - b'0') as usize)),
            'n' => Ok(ReAst::Lit('\n')),
            'r' => Ok(ReAst::Lit('\r')),
            't' => Ok(ReAst::Lit('\t')),
            'v' => Ok(ReAst::Lit('\x0B')),
            'f' => Ok(ReAst::Lit('\x0C')),
            '0' => Ok(ReAst::Lit('\0')),
            'x' => {
                let c = self.read_hex(2)?;
                Ok(ReAst::Lit(c))
            }
            'u' => {
                if self.peek() == Some('{') {
                    self.next();
                    let mut val = 0u32;
                    while let Some(c) = self.next() {
                        if c == '}' {
                            break;
                        }
                        let d = c
                            .to_digit(16)
                            .ok_or_else(|| "Invalid unicode escape".to_string())?;
                        val = val.saturating_mul(16).saturating_add(d);
                    }
                    let ch = char::from_u32(val).ok_or_else(|| "Invalid code point".to_string())?;
                    Ok(ReAst::Lit(ch))
                } else {
                    let c = self.read_hex(4)?;
                    Ok(ReAst::Lit(c))
                }
            }
            other => Ok(ReAst::Lit(other)),
        }
    }

    fn read_hex(&mut self, digits: usize) -> Result<char, String> {
        let mut val = 0u32;
        for _ in 0..digits {
            let Some(c) = self.next() else {
                return Err("Incomplete hex escape in regular expression".to_string());
            };
            let d = c
                .to_digit(16)
                .ok_or_else(|| "Invalid hex escape in regular expression".to_string())?;
            val = (val << 4) | d;
        }
        char::from_u32(val).ok_or_else(|| "Invalid character code".to_string())
    }

    fn parse_class(&mut self) -> Result<ReAst, String> {
        let negated = if self.peek() == Some('^') {
            self.next();
            true
        } else {
            false
        };
        let mut atoms = Vec::new();
        while let Some(ch) = self.next() {
            if ch == ']' {
                return Ok(ReAst::Class { negated, atoms });
            }
            let first = if ch == '\\' {
                let Some(esc) = self.next() else {
                    return Err("Unterminated character class".to_string());
                };
                match esc {
                    'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                        atoms.push(ClassAtom::Builtin(esc));
                        continue;
                    }
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'v' => '\x0B',
                    'f' => '\x0C',
                    '0' => '\0',
                    'b' => '\x08',
                    'x' => self.read_hex(2)?,
                    'u' => self.read_hex(4)?,
                    other => other,
                }
            } else {
                ch
            };

            if self.peek() == Some('-') && self.chars.get(self.pos + 1).copied() != Some(']') {
                self.next(); // consume '-'
                let Some(end_raw) = self.next() else {
                    return Err("Unterminated character class range".to_string());
                };
                let second = if end_raw == '\\' {
                    let Some(esc) = self.next() else {
                        return Err("Unterminated escape in character class".to_string());
                    };
                    match esc {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',
                        'x' => self.read_hex(2)?,
                        'u' => self.read_hex(4)?,
                        other => other,
                    }
                } else {
                    end_raw
                };
                atoms.push(ClassAtom::Range(first, second));
            } else {
                atoms.push(ClassAtom::Single(first));
            }
        }
        Err("Unterminated '[' in regular expression".to_string())
    }

    fn parse_quantifier(&mut self, atom: ReAst) -> Result<ReAst, String> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.next();
                (0, None)
            }
            Some('+') => {
                self.next();
                (1, None)
            }
            Some('?') => {
                self.next();
                (0, Some(1))
            }
            Some('{') => {
                let save = self.pos;
                self.next();
                if let Some((lo, hi)) = self.try_parse_braces() {
                    (lo, hi)
                } else {
                    self.pos = save;
                    return Ok(atom);
                }
            }
            _ => return Ok(atom),
        };
        let greedy = if self.peek() == Some('?') {
            self.next();
            false
        } else {
            true
        };
        Ok(ReAst::Repeat {
            inner: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    fn try_parse_braces(&mut self) -> Option<(usize, Option<usize>)> {
        let min = self.read_uint()?;
        match self.next()? {
            '}' => Some((min, Some(min))),
            ',' => {
                if self.peek() == Some('}') {
                    self.next();
                    Some((min, None))
                } else {
                    let max = self.read_uint()?;
                    if self.next() == Some('}') && max >= min {
                        Some((min, Some(max)))
                    } else {
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn read_uint(&mut self) -> Option<usize> {
        let mut n = 0usize;
        let mut any = false;
        while let Some(c) = self.peek() {
            if let Some(d) = c.to_digit(10) {
                any = true;
                n = n.saturating_mul(10).saturating_add(d as usize);
                self.next();
            } else {
                break;
            }
        }
        any.then_some(n)
    }
}

impl CompiledRegex {
    pub fn compile(pattern: &str, flags_str: &str) -> Result<(Self, String), String> {
        let (flags, canonical_flags) = ReFlags::parse(flags_str)?;
        let mut parser = Parser::new(pattern);
        let ast = parser.parse_top()?;
        Ok((
            Self {
                ast,
                group_count: parser.groups,
                flags,
            },
            canonical_flags,
        ))
    }

    pub fn exec_at(&self, input: &[char], start_pos: usize) -> Option<ReMatch> {
        if start_pos > input.len() {
            return None;
        }
        if self.flags.sticky {
            let mut groups = vec![None; self.group_count];
            let mut steps = 0;
            if let Some(end) = self.match_node(&self.ast, input, start_pos, &mut groups, &mut steps)
            {
                return Some(ReMatch {
                    start_char: start_pos,
                    end_char: end,
                    groups,
                });
            }
            return None;
        }
        for pos in start_pos..=input.len() {
            let mut groups = vec![None; self.group_count];
            let mut steps = 0;
            if let Some(end) = self.match_node(&self.ast, input, pos, &mut groups, &mut steps) {
                return Some(ReMatch {
                    start_char: pos,
                    end_char: end,
                    groups,
                });
            }
        }
        None
    }

    fn match_node(
        &self,
        node: &ReAst,
        input: &[char],
        pos: usize,
        groups: &mut Vec<Option<(usize, usize)>>,
        steps: &mut usize,
    ) -> Option<usize> {
        self.match_seq(std::slice::from_ref(node), input, pos, groups, steps)
    }

    fn match_seq(
        &self,
        nodes: &[ReAst],
        input: &[char],
        pos: usize,
        groups: &mut Vec<Option<(usize, usize)>>,
        steps: &mut usize,
    ) -> Option<usize> {
        *steps += 1;
        if *steps > MAX_MATCH_STEPS {
            return None;
        }
        let Some((first, rest)) = nodes.split_first() else {
            return Some(pos);
        };
        match first {
            ReAst::Empty => self.match_seq(rest, input, pos, groups, steps),
            ReAst::Lit(expected) => {
                let actual = *input.get(pos)?;
                if self.char_eq(actual, *expected) {
                    self.match_seq(rest, input, pos + 1, groups, steps)
                } else {
                    None
                }
            }
            ReAst::Dot => {
                let actual = *input.get(pos)?;
                if !self.flags.dot_all && matches!(actual, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                    return None;
                }
                self.match_seq(rest, input, pos + 1, groups, steps)
            }
            ReAst::Builtin(kind) => {
                let actual = *input.get(pos)?;
                if matches_builtin(*kind, actual) {
                    self.match_seq(rest, input, pos + 1, groups, steps)
                } else {
                    None
                }
            }
            ReAst::Anchor(kind) => {
                let ok = match *kind {
                    '^' => {
                        pos == 0
                            || (self.flags.multiline
                                && pos > 0
                                && matches!(input[pos - 1], '\n' | '\r'))
                    }
                    '$' => {
                        pos == input.len()
                            || (self.flags.multiline
                                && pos < input.len()
                                && matches!(input[pos], '\n' | '\r'))
                    }
                    'b' => is_word_boundary(input, pos),
                    'B' => !is_word_boundary(input, pos),
                    _ => false,
                };
                if ok {
                    self.match_seq(rest, input, pos, groups, steps)
                } else {
                    None
                }
            }
            ReAst::Class { negated, atoms } => {
                let actual = *input.get(pos)?;
                let mut matched = false;
                for atom in atoms {
                    let hit = match atom {
                        ClassAtom::Single(c) => self.char_eq(actual, *c),
                        ClassAtom::Range(lo, hi) => {
                            if self.flags.ignore_case {
                                let a = actual.to_ascii_lowercase();
                                let l = lo.to_ascii_lowercase();
                                let h = hi.to_ascii_lowercase();
                                a >= l && a <= h
                            } else {
                                actual >= *lo && actual <= *hi
                            }
                        }
                        ClassAtom::Builtin(b) => matches_builtin(*b, actual),
                    };
                    if hit {
                        matched = true;
                        break;
                    }
                }
                if matched != *negated {
                    self.match_seq(rest, input, pos + 1, groups, steps)
                } else {
                    None
                }
            }
            ReAst::Seq(inner) => {
                let mut combined = Vec::with_capacity(inner.len() + rest.len());
                combined.extend_from_slice(inner);
                combined.extend_from_slice(rest);
                self.match_seq(&combined, input, pos, groups, steps)
            }
            ReAst::Alt(branches) => {
                for branch in branches {
                    let saved = groups.clone();
                    let mut combined = Vec::with_capacity(1 + rest.len());
                    combined.push(branch.clone());
                    combined.extend_from_slice(rest);
                    if let Some(end) = self.match_seq(&combined, input, pos, groups, steps) {
                        return Some(end);
                    }
                    *groups = saved;
                }
                None
            }
            ReAst::Group { index, inner } => {
                let saved = groups.clone();
                let mut inner_groups = saved.clone();
                if let Some(mid) = self.match_node(inner, input, pos, &mut inner_groups, steps) {
                    if let Some(idx) = index
                        && *idx >= 1
                        && *idx <= inner_groups.len()
                    {
                        inner_groups[*idx - 1] = Some((pos, mid));
                    }
                    if let Some(end) = self.match_seq(rest, input, mid, &mut inner_groups, steps) {
                        *groups = inner_groups;
                        return Some(end);
                    }
                }
                *groups = saved;
                None
            }
            ReAst::Lookahead { positive, inner } => {
                let mut temp_groups = groups.clone();
                let hit = self
                    .match_node(inner, input, pos, &mut temp_groups, steps)
                    .is_some();
                if hit == *positive {
                    if *positive {
                        *groups = temp_groups;
                    }
                    self.match_seq(rest, input, pos, groups, steps)
                } else {
                    None
                }
            }
            ReAst::Backref(idx) => {
                let Some(Some((gs, ge))) = groups.get(idx.saturating_sub(1)).copied() else {
                    return self.match_seq(rest, input, pos, groups, steps);
                };
                let slice = &input[gs..ge];
                if pos + slice.len() > input.len() {
                    return None;
                }
                for (i, &c) in slice.iter().enumerate() {
                    if !self.char_eq(input[pos + i], c) {
                        return None;
                    }
                }
                self.match_seq(rest, input, pos + slice.len(), groups, steps)
            }
            ReAst::Repeat {
                inner,
                min,
                max,
                greedy,
            } => {
                let mut positions: Vec<(usize, Vec<Option<(usize, usize)>>)> = Vec::new();
                let mut cur_pos = pos;
                let mut cur_groups = groups.clone();
                let mut count = 0usize;

                while count < *min {
                    let Some(next_pos) =
                        self.match_node(inner, input, cur_pos, &mut cur_groups, steps)
                    else {
                        return None;
                    };
                    cur_pos = next_pos;
                    count += 1;
                }
                positions.push((cur_pos, cur_groups.clone()));

                let limit = max.unwrap_or(input.len().saturating_sub(pos) + 1);
                while count < limit {
                    let Some(next_pos) =
                        self.match_node(inner, input, cur_pos, &mut cur_groups, steps)
                    else {
                        break;
                    };
                    if next_pos == cur_pos {
                        break;
                    }
                    cur_pos = next_pos;
                    count += 1;
                    positions.push((cur_pos, cur_groups.clone()));
                }

                if *greedy {
                    for (p, g) in positions.into_iter().rev() {
                        let mut try_g = g;
                        if let Some(end) = self.match_seq(rest, input, p, &mut try_g, steps) {
                            *groups = try_g;
                            return Some(end);
                        }
                    }
                } else {
                    for (p, g) in positions {
                        let mut try_g = g;
                        if let Some(end) = self.match_seq(rest, input, p, &mut try_g, steps) {
                            *groups = try_g;
                            return Some(end);
                        }
                    }
                }
                None
            }
        }
    }

    fn char_eq(&self, a: char, b: char) -> bool {
        if self.flags.ignore_case {
            a.eq_ignore_ascii_case(&b) || a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_word_boundary(input: &[char], pos: usize) -> bool {
    let left = pos > 0 && is_word_char(input[pos - 1]);
    let right = pos < input.len() && is_word_char(input[pos]);
    left != right
}

fn matches_builtin(kind: char, c: char) -> bool {
    match kind {
        'd' => c.is_ascii_digit(),
        'D' => !c.is_ascii_digit(),
        'w' => is_word_char(c),
        'W' => !is_word_char(c),
        's' => c.is_whitespace(),
        'S' => !c.is_whitespace(),
        _ => false,
    }
}
