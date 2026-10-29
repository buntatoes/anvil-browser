// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! A small in-memory element tree for selector tests.

#![allow(dead_code)]

use std::rc::Rc;

use anvil_css::selectors::{Direction, Element, ElementState};

#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub namespace: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub states: Vec<ElementState>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub has_text: bool,
    pub rtl: bool,
}

#[derive(Debug, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

pub const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
pub const SVG_NS: &str = "http://www.w3.org/2000/svg";

impl Tree {
    /// Add an element described as `name#id.class1.class2`, under `parent`.
    pub fn add(&mut self, parent: Option<usize>, spec: &str) -> usize {
        let mut node = Node {
            namespace: HTML_NS.to_string(),
            ..Node::default()
        };
        let mut cur = String::new();
        let mut kind = 'n';
        let flush = |kind: char, cur: &mut String, node: &mut Node| {
            let text = std::mem::take(cur);
            match kind {
                'n' => node.name = text,
                '#' => node.id = Some(text),
                _ => node.classes.push(text),
            }
        };
        for c in spec.chars() {
            if c == '#' || c == '.' {
                flush(kind, &mut cur, &mut node);
                kind = c;
            } else {
                cur.push(c);
            }
        }
        flush(kind, &mut cur, &mut node);
        if let Some(id) = &node.id {
            node.attrs.push(("id".into(), id.clone()));
        }
        if !node.classes.is_empty() {
            node.attrs.push(("class".into(), node.classes.join(" ")));
        }
        node.parent = parent;
        let i = self.nodes.len();
        self.nodes.push(node);
        if let Some(p) = parent {
            self.nodes[p].children.push(i);
        }
        i
    }

    pub fn attr(&mut self, i: usize, name: &str, value: &str) {
        self.nodes[i]
            .attrs
            .push((name.to_string(), value.to_string()));
    }

    pub fn state(&mut self, i: usize, s: ElementState) {
        self.nodes[i].states.push(s);
    }

    pub fn finish(self) -> Rc<Tree> {
        Rc::new(self)
    }
}

#[derive(Clone, Debug)]
pub struct El {
    pub tree: Rc<Tree>,
    pub i: usize,
}

impl El {
    pub fn new(tree: &Rc<Tree>, i: usize) -> El {
        El {
            tree: tree.clone(),
            i,
        }
    }
    fn node(&self) -> &Node {
        &self.tree.nodes[self.i]
    }
    fn at(&self, i: usize) -> El {
        El {
            tree: self.tree.clone(),
            i,
        }
    }
    fn sibling(&self, delta: isize) -> Option<El> {
        let p = self.node().parent?;
        let kids = &self.tree.nodes[p].children;
        let pos = kids.iter().position(|&k| k == self.i)? as isize + delta;
        if pos < 0 {
            return None;
        }
        kids.get(pos as usize).map(|&k| self.at(k))
    }
}

impl Element for El {
    fn parent_element(&self) -> Option<Self> {
        self.node().parent.map(|p| self.at(p))
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.sibling(-1)
    }
    fn next_sibling_element(&self) -> Option<Self> {
        self.sibling(1)
    }
    fn first_child_element(&self) -> Option<Self> {
        self.node().children.first().map(|&c| self.at(c))
    }
    fn is_same(&self, other: &Self) -> bool {
        self.i == other.i && Rc::ptr_eq(&self.tree, &other.tree)
    }
    fn local_name(&self) -> &str {
        &self.node().name
    }
    fn namespace_url(&self) -> &str {
        &self.node().namespace
    }
    fn id(&self) -> Option<&str> {
        self.node().id.as_deref()
    }
    fn has_class(&self, name: &str) -> bool {
        self.node().classes.iter().any(|c| c == name)
    }
    fn for_each_class<F: FnMut(&str)>(&self, mut f: F) {
        for c in &self.node().classes {
            f(c);
        }
    }
    fn attribute(&self, name: &str) -> Option<&str> {
        self.node()
            .attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    fn has_state(&self, state: ElementState) -> bool {
        self.node().states.contains(&state)
    }
    fn is_empty(&self) -> bool {
        self.node().children.is_empty() && !self.node().has_text
    }
    fn is_html(&self) -> bool {
        self.node().namespace == HTML_NS
    }
    fn direction(&self) -> Direction {
        if self.node().rtl {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }
}

/// A deterministic xorshift generator for tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
