// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Statements: declarations, control flow, exceptions.

use super::expr::FnRef;
use super::scope::{Tag, annex_b_target};
use super::{Cg, Control, CtlKind, Exit, R, err, unsupported_fn};
use crate::cinder::ast::{
    CatchClause, Expr, ForHead, ForInit, PatKind, Stmt, StmtKind, SwitchCase, VarDecl, VarKind,
};
use crate::cinder::bytecode::{BinOp, Handler, Op, Reg};
use crate::cinder::error::Span;

impl Cg<'_> {
    /// A statement list.
    pub fn stmts(&mut self, body: &[Stmt]) -> R<()> {
        for st in body {
            self.stmt(st)?;
        }
        Ok(())
    }

    /// One statement.
    pub fn stmt(&mut self, st: &Stmt) -> R<()> {
        self.depth.enter(st.span)?;
        let r = self.stmt_inner(st, Vec::new());
        self.depth.leave();
        r
    }

    fn push_control(&mut self, labels: Vec<String>, kind: CtlKind) -> usize {
        let d = self.fr().env_depth;
        let f = self.f();
        f.controls.push(Control {
            labels,
            kind,
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: d,
            cont_depth: d,
        });
        f.controls.len() - 1
    }

    /// Pop the innermost control and point its breaks here and its
    /// continues at `cont`.
    fn pop_control(&mut self, cont: Option<u32>) -> Option<Control> {
        let c = self.f().controls.pop()?;
        for j in &c.breaks {
            self.patch(*j);
        }
        if let Some(t) = cont {
            for j in &c.continues {
                self.patch_to(*j, t);
            }
        }
        Some(c)
    }

    fn loop_header(&mut self) {
        let f = self.f();
        let id = f.b.loops;
        f.b.loops += 1;
        f.b.emit(Op::LoopHeader { id });
    }

    fn cond_jump_false(&mut self, test: &Expr) -> R<u32> {
        let m = self.mark();
        let t = self.expr_tmp(test)?;
        let j = self.emit(Op::JumpIfFalse { cond: t, offset: 0 });
        self.release(m);
        Ok(j)
    }

    fn stmt_inner(&mut self, st: &Stmt, labels: Vec<String>) -> R<()> {
        let span = st.span;
        self.at(span);
        match &st.kind {
            StmtKind::Expr { expr, .. } => {
                let m = self.mark();
                match self.fr().completion {
                    Some(c) => self.expr(expr, c)?,
                    None => {
                        self.expr_tmp(expr)?;
                    }
                }
                self.release(m);
            }
            StmtKind::Var(d) => self.var_decl(d)?,
            StmtKind::Function(f) => {
                if f.is_async || f.is_generator {
                    return err(unsupported_fn(f.is_async, f.is_generator), span);
                }
                self.annex_b_copy(f.id.as_ref().map(|i| i.name.as_str()), span)?;
            }
            StmtKind::Class(_) => return err("class declarations are not supported yet", span),
            StmtKind::Block(body) => {
                if labels.is_empty() {
                    self.block(span, Tag::Block, body)?;
                } else {
                    self.push_control(labels, CtlKind::Block);
                    self.block(span, Tag::Block, body)?;
                    self.pop_control(None);
                }
            }
            StmtKind::Empty | StmtKind::Debugger => {}
            StmtKind::If { test, cons, alt } => {
                let lab = !labels.is_empty();
                if lab {
                    self.push_control(labels, CtlKind::Block);
                }
                let j = self.cond_jump_false(test)?;
                self.branch(cons)?;
                match alt {
                    Some(a) => {
                        let end = self.jump();
                        self.patch(j);
                        self.branch(a)?;
                        self.patch(end);
                    }
                    None => self.patch(j),
                }
                if lab {
                    self.pop_control(None);
                }
            }
            StmtKind::While { test, body } => {
                let c = self.push_control(labels, CtlKind::Loop);
                let top = self.pc();
                self.loop_header();
                let j = self.cond_jump_false(test)?;
                self.stmt(body)?;
                self.jump_to(top);
                self.patch(j);
                let _ = c;
                self.pop_control(Some(top));
            }
            StmtKind::DoWhile { body, test } => {
                self.push_control(labels, CtlKind::Loop);
                let top = self.pc();
                self.loop_header();
                self.stmt(body)?;
                let cont = self.pc();
                let m = self.mark();
                let t = self.expr_tmp(test)?;
                let j = self.emit(Op::JumpIfTrue { cond: t, offset: 0 });
                self.patch_to(j, top);
                self.release(m);
                self.pop_control(Some(cont));
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => self.for_loop(
                st,
                labels,
                init.as_ref(),
                test.as_deref(),
                update.as_deref(),
                body,
            )?,
            StmtKind::ForIn { left, right, body } => {
                self.for_each(st, labels, left, right, body, false)?
            }
            StmtKind::ForOf {
                left,
                right,
                body,
                is_await,
            } => {
                if *is_await {
                    return err("'for await' is not supported yet", span);
                }
                self.for_each(st, labels, left, right, body, true)?
            }
            StmtKind::Continue(label) => {
                let target =
                    self.find_target(label.as_ref().map(|l| l.name.as_str()), true, span)?;
                self.emit_exit(Exit::Continue(target), None, span)?;
            }
            StmtKind::Break(label) => {
                let target =
                    self.find_target(label.as_ref().map(|l| l.name.as_str()), false, span)?;
                self.emit_exit(Exit::Break(target), None, span)?;
            }
            StmtKind::Return(e) => {
                let m = self.mark();
                let v = match e {
                    Some(e) => self.expr_tmp(e)?,
                    None => {
                        let t = self.tmp(span)?;
                        self.emit(Op::LoadUndefined { dst: t });
                        t
                    }
                };
                self.at(span);
                self.emit_exit(Exit::Return, Some(v), span)?;
                self.release(m);
            }
            StmtKind::With { .. } => return err("'with' statements are not supported", span),
            StmtKind::Switch { disc, cases } => self.switch(st, labels, disc, cases)?,
            StmtKind::Labeled { label, body } => {
                let mut labels = labels;
                labels.push(label.name.clone());
                self.depth.enter(body.span)?;
                let r = self.stmt_inner(body, labels);
                self.depth.leave();
                r?;
            }
            StmtKind::Throw(e) => {
                let m = self.mark();
                let v = self.expr_tmp(e)?;
                self.at(span);
                self.emit(Op::Throw { src: v });
                self.release(m);
            }
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => {
                let lab = !labels.is_empty();
                if lab {
                    self.push_control(labels, CtlKind::Block);
                }
                self.try_stmt(span, block, handler.as_ref(), finalizer.as_deref())?;
                if lab {
                    self.pop_control(None);
                }
            }
            StmtKind::Import(_) | StmtKind::Export(_) => {
                return err("modules are not supported yet", span);
            }
        }
        Ok(())
    }

    /// An `if` branch; see the matching pass in scope analysis.
    fn branch(&mut self, st: &Stmt) -> R<()> {
        if matches!(st.kind, StmtKind::Function(_)) {
            self.block(st.span, Tag::Block, std::slice::from_ref(st))
        } else {
            self.stmt(st)
        }
    }

    fn find_target(&self, label: Option<&str>, cont: bool, span: Span) -> R<usize> {
        let cs = &self.fr().controls;
        for i in (0..cs.len()).rev() {
            let c = &cs[i];
            let fits = match label {
                Some(l) => c.labels.iter().any(|x| x == l),
                None => match c.kind {
                    CtlKind::Loop => true,
                    CtlKind::Switch => !cont,
                    _ => false,
                },
            };
            if fits {
                if cont && !matches!(c.kind, CtlKind::Loop) {
                    return err("'continue' target is not a loop", span);
                }
                return Ok(i);
            }
        }
        err(
            match (label, cont) {
                (Some(l), _) => format!("undefined label '{l}'"),
                (None, true) => "'continue' outside a loop".to_string(),
                (None, false) => "'break' outside a loop or switch".to_string(),
            },
            span,
        )
    }

    /// A block with its own scope.
    fn block(&mut self, span: Span, tag: Tag, body: &[Stmt]) -> R<()> {
        self.enter_at(span, tag)?;
        self.hoist_functions(body)?;
        self.stmts(body)?;
        self.exit_scope();
        Ok(())
    }

    /// Annex B: at a block function's declaration, copy the block binding
    /// to the function-level `var` of the same name.
    fn annex_b_copy(&mut self, name: Option<&str>, span: Span) -> R<()> {
        let Some(name) = name else { return Ok(()) };
        let Some(cs) = self.scopes.len().checked_sub(1) else {
            return Ok(());
        };
        let id = self.scopes[cs].id;
        let func = self.a.scopes[id].func;
        if self.a.funcs[func].scope == id {
            return Ok(());
        }
        if !annex_b_target(self.a, id, name) {
            return Ok(());
        }
        let m = self.mark();
        let t = self.tmp(span)?;
        self.load_name(name, t, span)?;
        let outer = self.resolve_below(cs, name);
        self.store_place(outer, name, t, false, span)?;
        self.release(m);
        Ok(())
    }

    fn var_decl(&mut self, d: &VarDecl) -> R<()> {
        let init_kind = d.kind != VarKind::Var;
        for dc in &d.decls {
            if let PatKind::Ident(n) = &dc.target.kind {
                match &dc.init {
                    Some(e) => {
                        let m = self.mark();
                        let t = self.tmp(dc.span)?;
                        self.expr_named(e, t, n)?;
                        self.at(dc.span);
                        self.store_name(n, t, init_kind, dc.target.span)?;
                        self.release(m);
                    }
                    None if init_kind => {
                        let m = self.mark();
                        let t = self.tmp(dc.span)?;
                        self.emit(Op::LoadUndefined { dst: t });
                        self.store_name(n, t, true, dc.target.span)?;
                        self.release(m);
                    }
                    None => {}
                }
            } else {
                let m = self.mark();
                let t = self.tmp(dc.span)?;
                if let Some(e) = &dc.init {
                    self.expr(e, t)?;
                } else {
                    self.emit(Op::LoadUndefined { dst: t });
                }
                self.at(dc.span);
                self.assign_pat(&dc.target, t, init_kind)?;
                self.release(m);
            }
        }
        Ok(())
    }

    fn for_loop(
        &mut self,
        st: &Stmt,
        labels: Vec<String>,
        init: Option<&ForInit>,
        test: Option<&Expr>,
        update: Option<&Expr>,
        body: &Stmt,
    ) -> R<()> {
        let span = st.span;
        self.enter_at(span, Tag::For)?;
        match init {
            Some(ForInit::Var(d)) => self.var_decl(d)?,
            Some(ForInit::Expr(e)) => {
                let m = self.mark();
                self.expr_tmp(e)?;
                self.release(m);
            }
            None => {}
        }
        self.push_control(labels, CtlKind::Loop);
        let top = self.pc();
        self.loop_header();
        let exit = match test {
            Some(t) => Some(self.cond_jump_false(t)?),
            None => None,
        };
        self.stmt(body)?;
        let cont = self.pc();
        self.copy_iteration_env(span)?;
        if let Some(u) = update {
            let m = self.mark();
            self.expr_tmp(u)?;
            self.release(m);
        }
        self.jump_to(top);
        if let Some(j) = exit {
            self.patch(j);
        }
        self.pop_control(Some(cont));
        self.exit_scope();
        Ok(())
    }

    /// `for (let ...)`: each iteration gets its own copy of the loop's
    /// captured bindings, so closures made in one iteration keep its
    /// values.
    fn copy_iteration_env(&mut self, span: Span) -> R<()> {
        let Some(cs) = self.scopes.last() else {
            return Ok(());
        };
        if !cs.env {
            return Ok(());
        }
        let slots: Vec<u32> = cs
            .stores
            .iter()
            .filter_map(|s| match s {
                super::Store::Env(x) => Some(*x),
                _ => None,
            })
            .collect();
        let m = self.mark();
        let base = self.tmps(slots.len(), span)?;
        for (i, s) in slots.iter().enumerate() {
            self.emit(Op::GetEnv {
                dst: base + i as Reg,
                depth: 0,
                slot: *s,
            });
        }
        self.emit(Op::PopEnv);
        self.emit(Op::PushEnv {
            size: slots.len() as u32,
        });
        for (i, s) in slots.iter().enumerate() {
            self.emit(Op::SetEnv {
                src: base + i as Reg,
                depth: 0,
                slot: *s,
            });
        }
        self.release(m);
        Ok(())
    }

    fn for_each(
        &mut self,
        st: &Stmt,
        labels: Vec<String>,
        left: &ForHead,
        right: &Expr,
        body: &Stmt,
        of: bool,
    ) -> R<()> {
        let span = st.span;
        let m = self.mark();
        let src = self.expr_tmp(right)?;
        let list = self.tmp(span)?;
        self.at(right.span);
        if of {
            self.emit(Op::IterPrepare { dst: list, src });
        } else {
            self.emit(Op::ForInKeys {
                dst: list,
                obj: src,
            });
        }
        let i = self.tmp(span)?;
        self.emit(Op::LoadInt { dst: i, value: 0 });
        let len = self.tmp(span)?;
        let one = self.tmp(span)?;
        let v = self.tmp(span)?;
        let ctl = self.push_control(labels, CtlKind::Loop);
        let top = self.pc();
        self.loop_header();
        let key = self.key("length");
        let ic = self.ic();
        self.emit(Op::GetProp {
            dst: len,
            obj: list,
            key,
            ic,
        });
        self.emit(Op::Binary {
            op: BinOp::Lt,
            dst: len,
            a: i,
            b: len,
        });
        let exit = self.emit(Op::JumpIfFalse {
            cond: len,
            offset: 0,
        });
        self.emit(Op::GetElem {
            dst: v,
            obj: list,
            key: i,
        });
        self.emit(Op::LoadInt { dst: one, value: 1 });
        self.emit(Op::Binary {
            op: BinOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        self.enter_at(span, Tag::ForEach)?;
        let d = self.fr().env_depth;
        self.f().controls[ctl].cont_depth = d;
        match left {
            ForHead::Var(decl) => {
                let Some(dc) = decl.decls.first() else {
                    return err("missing loop variable", span);
                };
                if dc.init.is_some() {
                    return err(
                        "for-in loop variable initializers are not supported",
                        dc.span,
                    );
                }
                let init = decl.kind != VarKind::Var;
                self.assign_pat(&dc.target, v, init)?;
            }
            ForHead::Pat(p) => self.assign_pat(p, v, false)?,
        }
        self.stmt(body)?;
        let cont = self.pc();
        self.exit_scope();
        self.jump_to(top);
        self.patch(exit);
        self.pop_control(Some(cont));
        self.release(m);
        Ok(())
    }

    fn switch(
        &mut self,
        st: &Stmt,
        labels: Vec<String>,
        disc: &Expr,
        cases: &[SwitchCase],
    ) -> R<()> {
        let span = st.span;
        let m = self.mark();
        let d = self.expr_tmp(disc)?;
        self.enter_at(span, Tag::Switch)?;
        for c in cases {
            self.hoist_functions(&c.body)?;
        }
        self.push_control(labels, CtlKind::Switch);
        let mut jumps = Vec::with_capacity(cases.len());
        let t = self.tmp(span)?;
        for c in cases {
            match &c.test {
                Some(e) => {
                    self.expr(e, t)?;
                    self.emit(Op::Binary {
                        op: BinOp::StrictEq,
                        dst: t,
                        a: d,
                        b: t,
                    });
                    jumps.push(Some(self.emit(Op::JumpIfTrue { cond: t, offset: 0 })));
                }
                None => jumps.push(None),
            }
        }
        let fallback = self.jump();
        let mut default_at = None;
        for (c, j) in cases.iter().zip(&jumps) {
            let here = self.pc();
            match j {
                Some(j) => self.patch_to(*j, here),
                None => default_at = Some(here),
            }
            self.stmts(&c.body)?;
        }
        match default_at {
            Some(at) => self.patch_to(fallback, at),
            None => self.patch(fallback),
        }
        self.pop_control(None);
        self.exit_scope();
        self.release(m);
        Ok(())
    }

    fn try_stmt(
        &mut self,
        span: Span,
        block: &[Stmt],
        handler: Option<&CatchClause>,
        finalizer: Option<&[Stmt]>,
    ) -> R<()> {
        let m = self.mark();
        let depth0 = self.fr().env_depth;
        let fin = match finalizer {
            Some(_) => {
                let kind_reg = self.tmp(span)?;
                let exc = self.tmp(span)?;
                self.push_control(
                    Vec::new(),
                    CtlKind::Finally {
                        kind_reg,
                        exits: Vec::new(),
                        jumps: Vec::new(),
                    },
                );
                Some((kind_reg, exc))
            }
            None => None,
        };
        let start = self.pc();
        self.block(span, Tag::TryBlock, block)?;
        let try_end = self.pc();
        let mut to_finally = Vec::new();
        let mut to_end = Vec::new();
        match fin {
            Some((k, _)) => {
                self.emit(Op::LoadInt { dst: k, value: 0 });
                to_finally.push(self.jump());
            }
            None => to_end.push(self.jump()),
        }
        let mut covered_end = try_end;
        if let Some(h) = handler {
            let target = self.pc();
            let hm = self.mark();
            let exc = self.tmp(h.span)?;
            self.f().b.handlers.push(Handler {
                start,
                end: try_end,
                target,
                catch_reg: exc,
                env_depth: depth0,
            });
            self.enter_at(h.span, Tag::Catch)?;
            if let Some(p) = &h.param {
                self.assign_pat(p, exc, true)?;
            }
            self.hoist_functions(&h.body)?;
            self.stmts(&h.body)?;
            self.exit_scope();
            self.release(hm);
            covered_end = self.pc();
            if let Some((k, _)) = fin {
                self.emit(Op::LoadInt { dst: k, value: 0 });
                to_finally.push(self.jump());
            }
        }
        if let (Some((k, exc)), Some(body)) = (fin, finalizer) {
            let stub = self.pc();
            self.f().b.handlers.push(Handler {
                start,
                end: covered_end,
                target: stub,
                catch_reg: exc,
                env_depth: depth0,
            });
            self.emit(Op::LoadInt { dst: k, value: 1 });
            let entry = self.pc();
            for j in to_finally {
                self.patch_to(j, entry);
            }
            let ctl = self.f().controls.pop();
            let exits = match ctl {
                Some(Control {
                    kind: CtlKind::Finally { exits, jumps, .. },
                    ..
                }) => {
                    for j in jumps {
                        self.patch_to(j, entry);
                    }
                    exits
                }
                _ => return err("internal compiler error: finally control missing", span),
            };
            self.block(span, Tag::Finally, body)?;
            // After the finally block: carry on with whatever got us here.
            let t = self.tmp(span)?;
            self.emit(Op::LoadInt { dst: t, value: 1 });
            self.emit(Op::Binary {
                op: BinOp::StrictEq,
                dst: t,
                a: k,
                b: t,
            });
            let skip = self.emit(Op::JumpIfFalse { cond: t, offset: 0 });
            self.emit(Op::Throw { src: exc });
            self.patch(skip);
            for (i, exit) in exits.iter().enumerate() {
                let code =
                    i32::try_from(2 + i).or_else(|_| err("too many exits from one try", span))?;
                self.emit(Op::LoadInt {
                    dst: t,
                    value: code,
                });
                self.emit(Op::Binary {
                    op: BinOp::StrictEq,
                    dst: t,
                    a: k,
                    b: t,
                });
                let skip = self.emit(Op::JumpIfFalse { cond: t, offset: 0 });
                let value = match exit {
                    Exit::Return => Some(self.fr().ret_reg),
                    _ => None,
                };
                self.emit_exit(*exit, value, span)?;
                self.patch(skip);
            }
        }
        for j in to_end {
            self.patch(j);
        }
        self.release(m);
        Ok(())
    }
}

#[allow(dead_code)]
fn _uses(_: FnRef<'_>) {}
