// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! AST desugaring pass for ES2022-ES2024 classes (`class`, `extends`, `super`,
//! `static` fields/blocks, getters/setters, `#private` fields/methods, `#x in obj`),
//! `async`/`await` functions and arrows, `for await...of`, and `new.target`.

use std::collections::HashMap;

use super::{Depth, R, err};
use crate::anvil_js::ast::{
    ArrowBody, ArrowFunction, AssignOp, BinaryOp, CatchClause, Class, ClassMemberKind, ExportDecl,
    ExportDefault, Expr, ExprKind, ForHead, ForInit, Function, Ident, ImportDecl, ImportSpec,
    LogicalOp, MemberProp, MethodKind, Pat, PatKind, PatProp, Program, Prop, PropKey, PropKind,
    SourceType, Stmt, StmtKind, SwitchCase, Template, UnaryOp, VarDecl, VarDeclarator, VarKind,
};
use crate::anvil_js::error::Span;

/// Returns `true` when `program` contains any AST construct handled by the
/// desugaring pass (`class`, `#private`, `super`, `async`/`await`, `for await`,
/// generators, modules, or `new.target`).
pub fn needs_lowering(program: &Program) -> bool {
    program.source_type == SourceType::Module || stmts_need_lowering(&program.body)
}

fn stmts_need_lowering(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_needs_lowering)
}

fn stmt_needs_lowering(st: &Stmt) -> bool {
    match &st.kind {
        StmtKind::Class(_) | StmtKind::Import(_) | StmtKind::Export(_) => true,
        StmtKind::Function(f) => fn_needs_lowering(f),
        StmtKind::Expr { expr, .. } | StmtKind::Throw(expr) => expr_needs_lowering(expr),
        StmtKind::Return(e) => e.as_deref().is_some_and(expr_needs_lowering),
        StmtKind::Var(d) => d.decls.iter().any(|dc| {
            pat_needs_lowering(&dc.target) || dc.init.as_deref().is_some_and(expr_needs_lowering)
        }),
        StmtKind::Block(b) => stmts_need_lowering(b),
        StmtKind::If { test, cons, alt } => {
            expr_needs_lowering(test)
                || stmt_needs_lowering(cons)
                || alt.as_deref().is_some_and(stmt_needs_lowering)
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            init.as_ref().is_some_and(|i| match i {
                ForInit::Var(d) => d.decls.iter().any(|dc| {
                    pat_needs_lowering(&dc.target)
                        || dc.init.as_deref().is_some_and(expr_needs_lowering)
                }),
                ForInit::Expr(e) => expr_needs_lowering(e),
            }) || test.as_deref().is_some_and(expr_needs_lowering)
                || update.as_deref().is_some_and(expr_needs_lowering)
                || stmt_needs_lowering(body)
        }
        StmtKind::ForIn { left, right, body } => {
            for_head_needs_lowering(left) || expr_needs_lowering(right) || stmt_needs_lowering(body)
        }
        StmtKind::ForOf {
            left,
            right,
            body,
            is_await,
        } => {
            *is_await
                || for_head_needs_lowering(left)
                || expr_needs_lowering(right)
                || stmt_needs_lowering(body)
        }
        StmtKind::While { test, body } | StmtKind::DoWhile { body, test } => {
            expr_needs_lowering(test) || stmt_needs_lowering(body)
        }
        StmtKind::With { object, body } => expr_needs_lowering(object) || stmt_needs_lowering(body),
        StmtKind::Switch { disc, cases } => {
            expr_needs_lowering(disc)
                || cases.iter().any(|c| {
                    c.test.as_deref().is_some_and(expr_needs_lowering)
                        || stmts_need_lowering(&c.body)
                })
        }
        StmtKind::Labeled { body, .. } => stmt_needs_lowering(body),
        StmtKind::Try {
            block,
            handler,
            finalizer,
        } => {
            stmts_need_lowering(block)
                || handler.as_ref().is_some_and(|h| {
                    h.param.as_ref().is_some_and(pat_needs_lowering) || stmts_need_lowering(&h.body)
                })
                || finalizer.as_deref().is_some_and(stmts_need_lowering)
        }
        _ => false,
    }
}

fn for_head_needs_lowering(h: &ForHead) -> bool {
    match h {
        ForHead::Var(d) => d.decls.iter().any(|dc| {
            pat_needs_lowering(&dc.target) || dc.init.as_deref().is_some_and(expr_needs_lowering)
        }),
        ForHead::Pat(p) => pat_needs_lowering(p),
    }
}

fn pat_needs_lowering(p: &Pat) -> bool {
    match &p.kind {
        PatKind::Ident(_) => false,
        PatKind::Array(items) => items.iter().flatten().any(pat_needs_lowering),
        PatKind::Object(props) => props.iter().any(|pr| match pr {
            PatProp::Prop { key, value, .. } => {
                prop_key_needs_lowering(key) || pat_needs_lowering(value)
            }
            PatProp::Rest(r) => pat_needs_lowering(r),
        }),
        PatKind::Assign { target, default } => {
            pat_needs_lowering(target) || expr_needs_lowering(default)
        }
        PatKind::Rest(r) => pat_needs_lowering(r),
        PatKind::Expr(e) => expr_needs_lowering(e),
    }
}

fn prop_key_needs_lowering(k: &PropKey) -> bool {
    match k {
        PropKey::Computed(e) => expr_needs_lowering(e),
        PropKey::Private(_) => true,
        _ => false,
    }
}

fn fn_needs_lowering(f: &Function) -> bool {
    f.is_async
        || f.is_generator
        || f.params.iter().any(pat_needs_lowering)
        || stmts_need_lowering(&f.body)
}

fn expr_needs_lowering(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Class(_)
        | ExprKind::Super
        | ExprKind::Await(_)
        | ExprKind::Yield { .. }
        | ExprKind::NewTarget
        | ExprKind::ImportMeta
        | ExprKind::Import { .. }
        | ExprKind::PrivateIn { .. } => true,
        ExprKind::Function(f) => fn_needs_lowering(f),
        ExprKind::Arrow(a) => {
            a.is_async
                || a.params.iter().any(pat_needs_lowering)
                || match &a.body {
                    ArrowBody::Expr(x) => expr_needs_lowering(x),
                    ArrowBody::Block(b) => stmts_need_lowering(b),
                }
        }
        ExprKind::Template(t) => t.exprs.iter().any(expr_needs_lowering),
        ExprKind::TaggedTemplate { tag, quasi } => {
            expr_needs_lowering(tag) || quasi.exprs.iter().any(expr_needs_lowering)
        }
        ExprKind::Array(items) => items.iter().flatten().any(expr_needs_lowering),
        ExprKind::Object(props) => props.iter().any(|p| match &p.kind {
            PropKind::Init { key, value } => {
                prop_key_needs_lowering(key) || expr_needs_lowering(value)
            }
            PropKind::Shorthand(_) => false,
            PropKind::Method { key, value, .. } => {
                prop_key_needs_lowering(key) || fn_needs_lowering(value)
            }
            PropKind::Spread(x) => expr_needs_lowering(x),
        }),
        ExprKind::Unary { arg, .. }
        | ExprKind::Update { arg, .. }
        | ExprKind::Spread(arg)
        | ExprKind::Paren(arg)
        | ExprKind::OptionalChain(arg) => expr_needs_lowering(arg),
        ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. } => {
            expr_needs_lowering(left) || expr_needs_lowering(right)
        }
        ExprKind::Assign { target, value, .. } => {
            pat_needs_lowering(target) || expr_needs_lowering(value)
        }
        ExprKind::Cond { test, cons, alt } => {
            expr_needs_lowering(test) || expr_needs_lowering(cons) || expr_needs_lowering(alt)
        }
        ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
            expr_needs_lowering(callee) || args.iter().any(expr_needs_lowering)
        }
        ExprKind::Member { object, prop, .. } => {
            expr_needs_lowering(object)
                || match prop {
                    MemberProp::Computed(k) => expr_needs_lowering(k),
                    MemberProp::Private(_) => true,
                    MemberProp::Name(_) => false,
                }
        }
        ExprKind::Seq(xs) => xs.iter().any(expr_needs_lowering),
        _ => false,
    }
}

struct ClassCtx {
    super_var: Option<String>,
    init_fn_var: String,
    in_static: bool,
    privates: HashMap<String, String>,
}

struct Lowerer<'d> {
    depth: &'d mut Depth,
    next_id: u32,
    classes: Vec<ClassCtx>,
    in_function: bool,
    in_async: bool,
    in_generator: bool,
}

/// Desugar `program` into standard AST nodes.
pub fn lower_program(program: &Program, depth: &mut Depth) -> R<Program> {
    let mut low = Lowerer {
        depth,
        next_id: 1,
        classes: Vec::new(),
        in_function: false,
        in_async: false,
        in_generator: false,
    };
    let body = low.lower_stmts(&program.body)?;
    Ok(Program {
        source_type: SourceType::Script,
        body,
        strict: program.strict || program.source_type == SourceType::Module,
        span: program.span,
    })
}

impl Lowerer<'_> {
    fn fresh_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn fresh_span(&mut self, base: Span) -> Span {
        let id = self.fresh_id() as usize;
        Span {
            start: base.start.saturating_add(1_000_000 + id * 4),
            end: base.end.saturating_add(1_000_002 + id * 4),
            line: base.line,
            column: base.column,
        }
    }

    fn resolve_private(&self, name: &str, span: Span) -> R<String> {
        for ctx in self.classes.iter().rev() {
            if let Some(hidden) = ctx.privates.get(name) {
                return Ok(hidden.clone());
            }
        }
        err("private names are not supported yet", span)
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> R<Vec<Stmt>> {
        let mut out = Vec::with_capacity(stmts.len());
        for st in stmts {
            match &st.kind {
                StmtKind::Import(imp) => {
                    self.lower_import_into(imp, st.span, &mut out)?;
                }
                StmtKind::Export(exp) => {
                    self.lower_export_into(exp, st.span, &mut out)?;
                }
                _ => out.push(self.lower_stmt(st)?),
            }
        }
        Ok(out)
    }

    fn lower_import_into(&mut self, imp: &ImportDecl, span: Span, out: &mut Vec<Stmt>) -> R<()> {
        let mid = self.fresh_id();
        let mod_var = format!("__anvil_mod_{mid}");
        let src_str = String::from_utf16_lossy(&imp.source.value);
        let import_call = Expr {
            span,
            kind: ExprKind::Call {
                callee: Box::new(ident_expr("__anvil_import_module", span)),
                args: vec![str_expr(&src_str, span)],
                optional: false,
            },
        };
        let mut decls = vec![VarDeclarator {
            target: Pat {
                kind: PatKind::Ident(mod_var.clone()),
                span,
            },
            init: Some(Box::new(import_call)),
            span,
        }];
        for spec in &imp.specifiers {
            match spec {
                ImportSpec::Default(local) => {
                    let def_prop = member_name(ident_expr(&mod_var, span), "default", span);
                    let init_expr = Expr {
                        span,
                        kind: ExprKind::Cond {
                            test: Box::new(Expr {
                                span,
                                kind: ExprKind::Binary {
                                    op: BinaryOp::StrictNotEq,
                                    left: Box::new(def_prop.clone()),
                                    right: Box::new(ident_expr("undefined", span)),
                                },
                            }),
                            cons: Box::new(def_prop),
                            alt: Box::new(ident_expr(&mod_var, span)),
                        },
                    };
                    decls.push(VarDeclarator {
                        target: Pat {
                            kind: PatKind::Ident(local.name.clone()),
                            span: local.span,
                        },
                        init: Some(Box::new(init_expr)),
                        span: local.span,
                    });
                }
                ImportSpec::Namespace(local) => {
                    decls.push(VarDeclarator {
                        target: Pat {
                            kind: PatKind::Ident(local.name.clone()),
                            span: local.span,
                        },
                        init: Some(Box::new(ident_expr(&mod_var, span))),
                        span: local.span,
                    });
                }
                ImportSpec::Named { imported, local } => {
                    let imp_name = imported.text();
                    let prop = Expr {
                        span: local.span,
                        kind: ExprKind::Member {
                            object: Box::new(ident_expr(&mod_var, span)),
                            prop: MemberProp::Computed(Box::new(str_expr(&imp_name, span))),
                            optional: false,
                        },
                    };
                    decls.push(VarDeclarator {
                        target: Pat {
                            kind: PatKind::Ident(local.name.clone()),
                            span: local.span,
                        },
                        init: Some(Box::new(prop)),
                        span: local.span,
                    });
                }
            }
        }
        out.push(Stmt {
            span,
            kind: StmtKind::Var(VarDecl {
                kind: VarKind::Var,
                decls,
                span,
            }),
        });
        Ok(())
    }

    fn lower_export_into(&mut self, exp: &ExportDecl, span: Span, out: &mut Vec<Stmt>) -> R<()> {
        match exp {
            ExportDecl::Decl(st) => {
                let names = collect_decl_names(st);
                let lowered = self.lower_stmt(st)?;
                out.push(lowered);
                for name in names {
                    out.push(assign_export_stmt(&name, ident_expr(&name, span), span));
                }
            }
            ExportDecl::Default(def) => match def {
                ExportDefault::Function(f) => {
                    let lf = self.lower_function(f)?;
                    if let Some(id) = &lf.id {
                        let name = id.name.clone();
                        out.push(Stmt {
                            span,
                            kind: StmtKind::Function(Box::new(lf)),
                        });
                        out.push(assign_export_stmt("default", ident_expr(&name, span), span));
                    } else {
                        out.push(assign_export_stmt(
                            "default",
                            Expr {
                                span,
                                kind: ExprKind::Function(Box::new(lf)),
                            },
                            span,
                        ));
                    }
                }
                ExportDefault::Class(cls) => {
                    let cls_expr = self.lower_class(cls, Some("default"))?;
                    if let Some(id) = &cls.id {
                        let name = id.name.clone();
                        out.push(let_stmt(&name, cls_expr, span));
                        out.push(assign_export_stmt("default", ident_expr(&name, span), span));
                    } else {
                        out.push(assign_export_stmt("default", cls_expr, span));
                    }
                }
                ExportDefault::Expr(e) => {
                    let le = self.lower_expr(e)?;
                    out.push(assign_export_stmt("default", le, span));
                }
            },
            ExportDecl::Named {
                specifiers, source, ..
            } => {
                if let Some(src) = source {
                    let mid = self.fresh_id();
                    let mod_var = format!("__anvil_mod_{mid}");
                    let src_str = String::from_utf16_lossy(&src.value);
                    let import_call = Expr {
                        span,
                        kind: ExprKind::Call {
                            callee: Box::new(ident_expr("__anvil_import_module", span)),
                            args: vec![str_expr(&src_str, span)],
                            optional: false,
                        },
                    };
                    out.push(let_stmt(&mod_var, import_call, span));
                    for spec in specifiers {
                        let prop = Expr {
                            span,
                            kind: ExprKind::Member {
                                object: Box::new(ident_expr(&mod_var, span)),
                                prop: MemberProp::Computed(Box::new(str_expr(
                                    &spec.local.text(),
                                    span,
                                ))),
                                optional: false,
                            },
                        };
                        out.push(assign_export_stmt(&spec.exported.text(), prop, span));
                    }
                } else {
                    for spec in specifiers {
                        let local_name = spec.local.text();
                        out.push(assign_export_stmt(
                            &spec.exported.text(),
                            ident_expr(&local_name, span),
                            span,
                        ));
                    }
                }
            }
            ExportDecl::All {
                exported, source, ..
            } => {
                let src_str = String::from_utf16_lossy(&source.value);
                let import_call = Expr {
                    span,
                    kind: ExprKind::Call {
                        callee: Box::new(ident_expr("__anvil_import_module", span)),
                        args: vec![str_expr(&src_str, span)],
                        optional: false,
                    },
                };
                if let Some(exp_name) = exported {
                    out.push(assign_export_stmt(&exp_name.text(), import_call, span));
                } else {
                    let assign = call_object_static(
                        "assign",
                        vec![ident_expr("__anvil_exports", span), import_call],
                        span,
                    );
                    out.push(expr_stmt(assign, span));
                }
            }
        }
        Ok(())
    }

    fn lower_stmt(&mut self, st: &Stmt) -> R<Stmt> {
        self.depth.enter(st.span)?;
        let span = st.span;
        let kind = match &st.kind {
            StmtKind::Expr { expr, directive } => StmtKind::Expr {
                expr: Box::new(self.lower_expr(expr)?),
                directive: directive.clone(),
            },
            StmtKind::Var(d) => StmtKind::Var(self.lower_var_decl(d)?),
            StmtKind::Function(f) => StmtKind::Function(Box::new(self.lower_function(f)?)),
            StmtKind::Class(cls) => {
                let cls_expr = self.lower_class(cls, None)?;
                if let Some(id) = &cls.id {
                    StmtKind::Var(VarDecl {
                        kind: VarKind::Let,
                        decls: vec![VarDeclarator {
                            target: Pat {
                                kind: PatKind::Ident(id.name.clone()),
                                span: id.span,
                            },
                            init: Some(Box::new(cls_expr)),
                            span,
                        }],
                        span,
                    })
                } else {
                    StmtKind::Expr {
                        expr: Box::new(cls_expr),
                        directive: None,
                    }
                }
            }
            StmtKind::Block(b) => StmtKind::Block(self.lower_stmts(b)?),
            StmtKind::Empty => StmtKind::Empty,
            StmtKind::Debugger => StmtKind::Debugger,
            StmtKind::If { test, cons, alt } => StmtKind::If {
                test: Box::new(self.lower_expr(test)?),
                cons: Box::new(self.lower_stmt(cons)?),
                alt: match alt {
                    Some(a) => Some(Box::new(self.lower_stmt(a)?)),
                    None => None,
                },
            },
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => StmtKind::For {
                init: match init {
                    Some(ForInit::Var(d)) => Some(ForInit::Var(self.lower_var_decl(d)?)),
                    Some(ForInit::Expr(e)) => Some(ForInit::Expr(Box::new(self.lower_expr(e)?))),
                    None => None,
                },
                test: match test {
                    Some(t) => Some(Box::new(self.lower_expr(t)?)),
                    None => None,
                },
                update: match update {
                    Some(u) => Some(Box::new(self.lower_expr(u)?)),
                    None => None,
                },
                body: Box::new(self.lower_stmt(body)?),
            },
            StmtKind::ForIn { left, right, body } => StmtKind::ForIn {
                left: self.lower_for_head(left)?,
                right: Box::new(self.lower_expr(right)?),
                body: Box::new(self.lower_stmt(body)?),
            },
            StmtKind::ForOf {
                left,
                right,
                body,
                is_await,
            } => {
                if *is_await && !self.in_async {
                    self.depth.leave();
                    return err("'for await' is not supported yet", span);
                }
                let lowered_right = self.lower_expr(right)?;
                let right_expr = if *is_await {
                    call_promise_static("__unwrap_all", vec![lowered_right], right.span)
                } else {
                    lowered_right
                };
                StmtKind::ForOf {
                    left: self.lower_for_head(left)?,
                    right: Box::new(right_expr),
                    body: Box::new(self.lower_stmt(body)?),
                    is_await: false,
                }
            }
            StmtKind::While { test, body } => StmtKind::While {
                test: Box::new(self.lower_expr(test)?),
                body: Box::new(self.lower_stmt(body)?),
            },
            StmtKind::DoWhile { body, test } => StmtKind::DoWhile {
                body: Box::new(self.lower_stmt(body)?),
                test: Box::new(self.lower_expr(test)?),
            },
            StmtKind::Continue(id) => StmtKind::Continue(id.clone()),
            StmtKind::Break(id) => StmtKind::Break(id.clone()),
            StmtKind::Return(e) => StmtKind::Return(match e {
                Some(x) => Some(Box::new(self.lower_expr(x)?)),
                None => None,
            }),
            StmtKind::With { object, body } => StmtKind::With {
                object: Box::new(self.lower_expr(object)?),
                body: Box::new(self.lower_stmt(body)?),
            },
            StmtKind::Switch { disc, cases } => {
                let mut out_cases = Vec::with_capacity(cases.len());
                for c in cases {
                    out_cases.push(SwitchCase {
                        test: match &c.test {
                            Some(t) => Some(Box::new(self.lower_expr(t)?)),
                            None => None,
                        },
                        body: self.lower_stmts(&c.body)?,
                        span: c.span,
                    });
                }
                StmtKind::Switch {
                    disc: Box::new(self.lower_expr(disc)?),
                    cases: out_cases,
                }
            }
            StmtKind::Labeled { label, body } => StmtKind::Labeled {
                label: label.clone(),
                body: Box::new(self.lower_stmt(body)?),
            },
            StmtKind::Throw(e) => StmtKind::Throw(Box::new(self.lower_expr(e)?)),
            StmtKind::Try {
                block,
                handler,
                finalizer,
            } => StmtKind::Try {
                block: self.lower_stmts(block)?,
                handler: match handler {
                    Some(h) => Some(CatchClause {
                        param: match &h.param {
                            Some(p) => Some(self.lower_pat(p)?),
                            None => None,
                        },
                        body: self.lower_stmts(&h.body)?,
                        span: h.span,
                    }),
                    None => None,
                },
                finalizer: match finalizer {
                    Some(f) => Some(self.lower_stmts(f)?),
                    None => None,
                },
            },
            StmtKind::Import(i) => StmtKind::Import(i.clone()),
            StmtKind::Export(e) => StmtKind::Export(e.clone()),
        };
        self.depth.leave();
        Ok(Stmt { kind, span })
    }

    fn lower_var_decl(&mut self, d: &VarDecl) -> R<VarDecl> {
        let mut decls = Vec::with_capacity(d.decls.len());
        for dc in &d.decls {
            decls.push(VarDeclarator {
                target: self.lower_pat(&dc.target)?,
                init: match &dc.init {
                    Some(e) => Some(Box::new(self.lower_expr(e)?)),
                    None => None,
                },
                span: dc.span,
            });
        }
        Ok(VarDecl {
            kind: d.kind,
            decls,
            span: d.span,
        })
    }

    fn lower_for_head(&mut self, h: &ForHead) -> R<ForHead> {
        Ok(match h {
            ForHead::Var(d) => ForHead::Var(self.lower_var_decl(d)?),
            ForHead::Pat(p) => ForHead::Pat(self.lower_pat(p)?),
        })
    }

    fn lower_pat(&mut self, p: &Pat) -> R<Pat> {
        let kind = match &p.kind {
            PatKind::Ident(n) => PatKind::Ident(n.clone()),
            PatKind::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(match it {
                        Some(sub) => Some(self.lower_pat(sub)?),
                        None => None,
                    });
                }
                PatKind::Array(out)
            }
            PatKind::Object(props) => {
                let mut out = Vec::with_capacity(props.len());
                for pr in props {
                    out.push(match pr {
                        PatProp::Prop {
                            key,
                            value,
                            shorthand,
                        } => PatProp::Prop {
                            key: self.lower_prop_key(key)?,
                            value: self.lower_pat(value)?,
                            shorthand: *shorthand,
                        },
                        PatProp::Rest(r) => PatProp::Rest(self.lower_pat(r)?),
                    });
                }
                PatKind::Object(out)
            }
            PatKind::Assign { target, default } => PatKind::Assign {
                target: Box::new(self.lower_pat(target)?),
                default: Box::new(self.lower_expr(default)?),
            },
            PatKind::Rest(r) => PatKind::Rest(Box::new(self.lower_pat(r)?)),
            PatKind::Expr(e) => PatKind::Expr(Box::new(self.lower_expr(e)?)),
        };
        Ok(Pat { kind, span: p.span })
    }

    fn lower_prop_key(&mut self, k: &PropKey) -> R<PropKey> {
        Ok(match k {
            PropKey::Computed(e) => PropKey::Computed(Box::new(self.lower_expr(e)?)),
            PropKey::Private(id) => {
                let hidden = self.resolve_private(&id.name, id.span)?;
                PropKey::Ident(Ident {
                    name: hidden,
                    span: id.span,
                })
            }
            other => other.clone(),
        })
    }

    fn lower_function(&mut self, f: &Function) -> R<Function> {
        let prev_fn = self.in_function;
        let prev_async = self.in_async;
        let prev_gen = self.in_generator;
        self.in_function = true;
        self.in_async = f.is_async;
        self.in_generator = f.is_generator;

        let mut params = Vec::with_capacity(f.params.len());
        for p in &f.params {
            params.push(self.lower_pat(p)?);
        }
        let body_stmts = self.lower_stmts(&f.body)?;

        self.in_function = prev_fn;
        self.in_async = prev_async;
        self.in_generator = prev_gen;

        let final_body = if f.is_generator {
            vec![self.wrap_generator_body(body_stmts, f.strict, f.span)]
        } else if f.is_async {
            vec![self.wrap_async_body(body_stmts, f.strict, f.span)]
        } else {
            body_stmts
        };

        Ok(Function {
            id: f.id.clone(),
            params,
            body: final_body,
            is_async: false,
            is_generator: false,
            strict: f.strict,
            simple_params: f.simple_params,
            span: f.span,
        })
    }

    fn wrap_generator_body(&mut self, body_stmts: Vec<Stmt>, strict: bool, span: Span) -> Stmt {
        let inner_span = self.fresh_span(span);
        let step_fn = Expr {
            span: inner_span,
            kind: ExprKind::Arrow(Box::new(ArrowFunction {
                params: vec![Pat {
                    kind: PatKind::Ident("__anvil_yields".to_string()),
                    span: inner_span,
                }],
                body: ArrowBody::Block(body_stmts),
                is_async: false,
                strict,
                span: inner_span,
            })),
        };
        Stmt {
            span,
            kind: StmtKind::Return(Some(Box::new(Expr {
                span,
                kind: ExprKind::Call {
                    callee: Box::new(ident_expr("__anvil_make_generator", span)),
                    args: vec![
                        step_fn,
                        Expr {
                            span,
                            kind: ExprKind::This,
                        },
                    ],
                    optional: false,
                },
            }))),
        }
    }

    fn lower_arrow(&mut self, a: &ArrowFunction) -> R<ArrowFunction> {
        let prev_async = self.in_async;
        self.in_async = a.is_async;

        let mut params = Vec::with_capacity(a.params.len());
        for p in &a.params {
            params.push(self.lower_pat(p)?);
        }
        let lowered_body = match &a.body {
            ArrowBody::Expr(e) => ArrowBody::Expr(Box::new(self.lower_expr(e)?)),
            ArrowBody::Block(b) => ArrowBody::Block(self.lower_stmts(b)?),
        };

        self.in_async = prev_async;

        let final_body = if a.is_async {
            let stmts = match lowered_body {
                ArrowBody::Expr(e) => vec![Stmt {
                    span: a.span,
                    kind: StmtKind::Return(Some(e)),
                }],
                ArrowBody::Block(b) => b,
            };
            ArrowBody::Block(vec![self.wrap_async_body(stmts, a.strict, a.span)])
        } else {
            lowered_body
        };

        Ok(ArrowFunction {
            params,
            body: final_body,
            is_async: false,
            strict: a.strict,
            span: a.span,
        })
    }

    fn wrap_async_body(&mut self, body_stmts: Vec<Stmt>, strict: bool, span: Span) -> Stmt {
        let inner_arrow_span = self.fresh_span(span);
        let try_span = self.fresh_span(span);
        let catch_span = self.fresh_span(span);
        let outer_arrow_span = self.fresh_span(span);

        let inner_call = Expr {
            span: inner_arrow_span,
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    span: inner_arrow_span,
                    kind: ExprKind::Arrow(Box::new(ArrowFunction {
                        params: Vec::new(),
                        body: ArrowBody::Block(body_stmts),
                        is_async: false,
                        strict,
                        span: inner_arrow_span,
                    })),
                }),
                args: Vec::new(),
                optional: false,
            },
        };

        let resolve_call = Stmt {
            span: try_span,
            kind: StmtKind::Expr {
                expr: Box::new(Expr {
                    span: try_span,
                    kind: ExprKind::Call {
                        callee: Box::new(ident_expr("__anvil_resolve", try_span)),
                        args: vec![inner_call],
                        optional: false,
                    },
                }),
                directive: None,
            },
        };

        let reject_call = Stmt {
            span: catch_span,
            kind: StmtKind::Expr {
                expr: Box::new(Expr {
                    span: catch_span,
                    kind: ExprKind::Call {
                        callee: Box::new(ident_expr("__anvil_reject", catch_span)),
                        args: vec![ident_expr("__anvil_err", catch_span)],
                        optional: false,
                    },
                }),
                directive: None,
            },
        };

        let try_stmt = Stmt {
            span: try_span,
            kind: StmtKind::Try {
                block: vec![resolve_call],
                handler: Some(CatchClause {
                    param: Some(Pat {
                        kind: PatKind::Ident("__anvil_err".to_string()),
                        span: catch_span,
                    }),
                    body: vec![reject_call],
                    span: catch_span,
                }),
                finalizer: None,
            },
        };

        let executor = Expr {
            span: outer_arrow_span,
            kind: ExprKind::Arrow(Box::new(ArrowFunction {
                params: vec![
                    Pat {
                        kind: PatKind::Ident("__anvil_resolve".to_string()),
                        span: outer_arrow_span,
                    },
                    Pat {
                        kind: PatKind::Ident("__anvil_reject".to_string()),
                        span: outer_arrow_span,
                    },
                ],
                body: ArrowBody::Block(vec![try_stmt]),
                is_async: false,
                strict,
                span: outer_arrow_span,
            })),
        };

        Stmt {
            span,
            kind: StmtKind::Return(Some(Box::new(Expr {
                span,
                kind: ExprKind::New {
                    callee: Box::new(ident_expr("Promise", span)),
                    args: vec![executor],
                },
            }))),
        }
    }

    fn lower_class(&mut self, cls: &Class, hint_name: Option<&str>) -> R<Expr> {
        let span = cls.span;
        let cid = self.fresh_id();
        let iife_span = self.fresh_span(span);
        let ctor_var = cls
            .id
            .as_ref()
            .map(|i| i.name.clone())
            .or_else(|| hint_name.map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("__AnvilClass_{cid}"));
        let super_var = cls
            .super_class
            .as_ref()
            .map(|_| format!("__anvil_super_{cid}"));
        let init_fn_var = format!("__anvil_init_{cid}");

        // 1. Collect all private names declared on this class.
        let mut privates = HashMap::new();
        for m in &cls.body {
            match &m.kind {
                ClassMemberKind::Method {
                    key: PropKey::Private(id),
                    ..
                }
                | ClassMemberKind::Field {
                    key: PropKey::Private(id),
                    ..
                } => {
                    privates.insert(id.name.clone(), format!("__anvil_priv_{cid}_{}", id.name));
                }
                _ => {}
            }
        }

        let lowered_super = match &cls.super_class {
            Some(s) => Some(self.lower_expr(s)?),
            None => None,
        };

        self.classes.push(ClassCtx {
            super_var: super_var.clone(),
            init_fn_var: init_fn_var.clone(),
            in_static: false,
            privates,
        });

        let mut iife_stmts: Vec<Stmt> = Vec::new();
        if let (Some(sv), Some(se)) = (&super_var, lowered_super) {
            let s_span = self.fresh_span(span);
            iife_stmts.push(let_stmt(sv, se, s_span));
        }

        // 2. Build `__anvil_init_<cid>(__self)` for instance fields and private instance methods.
        let mut init_body: Vec<Stmt> = Vec::new();
        for m in &cls.body {
            match &m.kind {
                ClassMemberKind::Method {
                    key: PropKey::Private(id),
                    is_static: false,
                    ..
                } => {
                    let hidden = self.resolve_private(&id.name, id.span)?;
                    let m_span = self.fresh_span(m.span);
                    // Copy descriptor from prototype onto `this` (`__self`) so `#m in obj` works.
                    let proto_expr =
                        member_name(ident_expr(&ctor_var, m_span), "prototype", m_span);
                    let get_desc = call_object_static(
                        "getOwnPropertyDescriptor",
                        vec![proto_expr, str_expr(&hidden, m_span)],
                        m_span,
                    );
                    let def_prop = call_object_static(
                        "defineProperty",
                        vec![
                            ident_expr("__self", m_span),
                            str_expr(&hidden, m_span),
                            get_desc,
                        ],
                        m_span,
                    );
                    init_body.push(expr_stmt(def_prop, m_span));
                }
                ClassMemberKind::Field {
                    key,
                    is_static: false,
                    value,
                } => {
                    let f_span = self.fresh_span(m.span);
                    let key_expr = self.prop_key_to_expr(key, f_span)?;
                    let val_expr = match value {
                        Some(v) => {
                            let lv = self.lower_expr(v)?;
                            call_with_this(
                                lv,
                                ident_expr("__self", f_span),
                                self.fresh_span(f_span),
                            )
                        }
                        None => ident_expr("undefined", f_span),
                    };
                    let def_prop = call_object_static(
                        "defineProperty",
                        vec![
                            ident_expr("__self", f_span),
                            key_expr,
                            data_descriptor_expr(val_expr, true, f_span),
                        ],
                        f_span,
                    );
                    init_body.push(expr_stmt(def_prop, f_span));
                }
                _ => {}
            }
        }
        let init_fn_span = self.fresh_span(span);
        init_body.push(Stmt {
            span: init_fn_span,
            kind: StmtKind::Return(Some(Box::new(ident_expr("__self", init_fn_span)))),
        });
        iife_stmts.push(Stmt {
            span: init_fn_span,
            kind: StmtKind::Function(Box::new(Function {
                id: Some(Ident {
                    name: init_fn_var.clone(),
                    span: init_fn_span,
                }),
                params: vec![Pat {
                    kind: PatKind::Ident("__self".to_string()),
                    span: init_fn_span,
                }],
                body: init_body,
                is_async: false,
                is_generator: false,
                strict: true,
                simple_params: true,
                span: init_fn_span,
            })),
        });

        // 3. Constructor function
        let explicit_ctor = cls.body.iter().find_map(|m| match &m.kind {
            ClassMemberKind::Method {
                kind: MethodKind::Constructor,
                value,
                ..
            } => Some(value),
            _ => None,
        });

        let ctor_fn_span = self.fresh_span(span);
        let ctor_fn = if let Some(user_ctor) = explicit_ctor {
            let mut lowered_ctor = self.lower_function(user_ctor)?;
            lowered_ctor.id = Some(Ident {
                name: ctor_var.clone(),
                span: ctor_fn_span,
            });
            lowered_ctor.strict = true;
            if super_var.is_none() {
                let init_call = expr_stmt(
                    Expr {
                        span: ctor_fn_span,
                        kind: ExprKind::Call {
                            callee: Box::new(ident_expr(&init_fn_var, ctor_fn_span)),
                            args: vec![Expr {
                                span: ctor_fn_span,
                                kind: ExprKind::This,
                            }],
                            optional: false,
                        },
                    },
                    ctor_fn_span,
                );
                lowered_ctor.body.insert(0, init_call);
            }
            lowered_ctor
        } else {
            let mut body = Vec::new();
            if let Some(sv) = &super_var {
                let s_call = Expr {
                    span: ctor_fn_span,
                    kind: ExprKind::Call {
                        callee: Box::new(member_name(
                            ident_expr(sv, ctor_fn_span),
                            "apply",
                            ctor_fn_span,
                        )),
                        args: vec![
                            Expr {
                                span: ctor_fn_span,
                                kind: ExprKind::This,
                            },
                            ident_expr("__args", ctor_fn_span),
                        ],
                        optional: false,
                    },
                };
                body.push(let_stmt("__sup_ret", s_call, ctor_fn_span));
                body.push(copy_super_return_stmt("__sup_ret", ctor_fn_span));
            }
            body.push(expr_stmt(
                Expr {
                    span: ctor_fn_span,
                    kind: ExprKind::Call {
                        callee: Box::new(ident_expr(&init_fn_var, ctor_fn_span)),
                        args: vec![Expr {
                            span: ctor_fn_span,
                            kind: ExprKind::This,
                        }],
                        optional: false,
                    },
                },
                ctor_fn_span,
            ));
            Function {
                id: Some(Ident {
                    name: ctor_var.clone(),
                    span: ctor_fn_span,
                }),
                params: vec![Pat {
                    kind: PatKind::Rest(Box::new(Pat {
                        kind: PatKind::Ident("__args".to_string()),
                        span: ctor_fn_span,
                    })),
                    span: ctor_fn_span,
                }],
                body,
                is_async: false,
                is_generator: false,
                strict: true,
                simple_params: false,
                span: ctor_fn_span,
            }
        };
        iife_stmts.push(Stmt {
            span: ctor_fn_span,
            kind: StmtKind::Function(Box::new(ctor_fn)),
        });

        // 4. Prototype linkage for `extends`
        if let Some(sv) = &super_var {
            let p_span = self.fresh_span(span);
            let set_inst_proto = call_object_static(
                "setPrototypeOf",
                vec![
                    member_name(ident_expr(&ctor_var, p_span), "prototype", p_span),
                    member_name(ident_expr(sv, p_span), "prototype", p_span),
                ],
                p_span,
            );
            let set_ctor_proto = call_object_static(
                "setPrototypeOf",
                vec![ident_expr(&ctor_var, p_span), ident_expr(sv, p_span)],
                p_span,
            );
            iife_stmts.push(expr_stmt(set_inst_proto, p_span));
            iife_stmts.push(expr_stmt(set_ctor_proto, p_span));
        }

        // 5. Methods, accessors, static fields, and static blocks
        for m in &cls.body {
            match &m.kind {
                ClassMemberKind::Method {
                    kind: MethodKind::Constructor,
                    ..
                }
                | ClassMemberKind::Field {
                    is_static: false, ..
                } => {}
                ClassMemberKind::Method {
                    key,
                    kind,
                    is_static,
                    value,
                } => {
                    let m_span = self.fresh_span(m.span);
                    let key_expr = self.prop_key_to_expr(key, m_span)?;
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = *is_static;
                    }
                    let mut lowered_fn = self.lower_function(value)?;
                    lowered_fn.strict = true;
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = false;
                    }
                    let fn_expr = Expr {
                        span: m_span,
                        kind: ExprKind::Function(Box::new(lowered_fn)),
                    };
                    let target_expr = if *is_static {
                        ident_expr(&ctor_var, m_span)
                    } else {
                        member_name(ident_expr(&ctor_var, m_span), "prototype", m_span)
                    };
                    match kind {
                        MethodKind::Method => {
                            let def = call_object_static(
                                "defineProperty",
                                vec![
                                    target_expr,
                                    key_expr,
                                    data_descriptor_expr(fn_expr, false, m_span),
                                ],
                                m_span,
                            );
                            iife_stmts.push(expr_stmt(def, m_span));
                        }
                        MethodKind::Get | MethodKind::Set => {
                            let acc_field = if *kind == MethodKind::Get {
                                "get"
                            } else {
                                "set"
                            };
                            let blk_span = self.fresh_span(m_span);
                            let existing = call_object_static(
                                "getOwnPropertyDescriptor",
                                vec![target_expr.clone(), ident_expr("__ak", blk_span)],
                                blk_span,
                            );
                            let fallback_desc = Expr {
                                span: blk_span,
                                kind: ExprKind::Object(vec![
                                    prop_init("enumerable", bool_expr(false, blk_span), blk_span),
                                    prop_init("configurable", bool_expr(true, blk_span), blk_span),
                                ]),
                            };
                            let desc_init = Expr {
                                span: blk_span,
                                kind: ExprKind::Logical {
                                    op: LogicalOp::Or,
                                    left: Box::new(existing),
                                    right: Box::new(fallback_desc),
                                },
                            };
                            let set_acc = expr_stmt(
                                Expr {
                                    span: blk_span,
                                    kind: ExprKind::Assign {
                                        op: AssignOp::Assign,
                                        target: Box::new(Pat {
                                            span: blk_span,
                                            kind: PatKind::Expr(Box::new(member_name(
                                                ident_expr("__ad", blk_span),
                                                acc_field,
                                                blk_span,
                                            ))),
                                        }),
                                        value: Box::new(fn_expr),
                                    },
                                },
                                blk_span,
                            );
                            let def = expr_stmt(
                                call_object_static(
                                    "defineProperty",
                                    vec![
                                        target_expr,
                                        ident_expr("__ak", blk_span),
                                        ident_expr("__ad", blk_span),
                                    ],
                                    blk_span,
                                ),
                                blk_span,
                            );
                            iife_stmts.push(Stmt {
                                span: blk_span,
                                kind: StmtKind::Block(vec![
                                    let_stmt("__ak", key_expr, blk_span),
                                    let_stmt("__ad", desc_init, blk_span),
                                    set_acc,
                                    def,
                                ]),
                            });
                        }
                        MethodKind::Constructor => {}
                    }
                }
                ClassMemberKind::Field {
                    key,
                    is_static: true,
                    value,
                } => {
                    let f_span = self.fresh_span(m.span);
                    let key_expr = self.prop_key_to_expr(key, f_span)?;
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = true;
                    }
                    let val_expr = match value {
                        Some(v) => {
                            let lv = self.lower_expr(v)?;
                            call_with_this(
                                lv,
                                ident_expr(&ctor_var, f_span),
                                self.fresh_span(f_span),
                            )
                        }
                        None => ident_expr("undefined", f_span),
                    };
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = false;
                    }
                    let def = call_object_static(
                        "defineProperty",
                        vec![
                            ident_expr(&ctor_var, f_span),
                            key_expr,
                            data_descriptor_expr(val_expr, true, f_span),
                        ],
                        f_span,
                    );
                    iife_stmts.push(expr_stmt(def, f_span));
                }
                ClassMemberKind::StaticBlock(stmts) => {
                    let b_span = self.fresh_span(m.span);
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = true;
                    }
                    let lowered_stmts = self.lower_stmts(stmts)?;
                    if let Some(top) = self.classes.last_mut() {
                        top.in_static = false;
                    }
                    let fn_expr = Expr {
                        span: b_span,
                        kind: ExprKind::Function(Box::new(Function {
                            id: None,
                            params: Vec::new(),
                            body: lowered_stmts,
                            is_async: false,
                            is_generator: false,
                            strict: true,
                            simple_params: true,
                            span: b_span,
                        })),
                    };
                    let call_expr = Expr {
                        span: b_span,
                        kind: ExprKind::Call {
                            callee: Box::new(member_name(fn_expr, "call", b_span)),
                            args: vec![ident_expr(&ctor_var, b_span)],
                            optional: false,
                        },
                    };
                    iife_stmts.push(expr_stmt(call_expr, b_span));
                }
            }
        }

        self.classes.pop();

        iife_stmts.push(Stmt {
            span: iife_span,
            kind: StmtKind::Return(Some(Box::new(ident_expr(&ctor_var, iife_span)))),
        });

        Ok(Expr {
            span,
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    span: iife_span,
                    kind: ExprKind::Function(Box::new(Function {
                        id: None,
                        params: Vec::new(),
                        body: iife_stmts,
                        is_async: false,
                        is_generator: false,
                        strict: true,
                        simple_params: true,
                        span: iife_span,
                    })),
                }),
                args: Vec::new(),
                optional: false,
            },
        })
    }

    fn prop_key_to_expr(&mut self, key: &PropKey, _span: Span) -> R<Expr> {
        Ok(match key {
            PropKey::Ident(id) => str_expr(&id.name, id.span),
            PropKey::Str(units, sp) => Expr {
                span: *sp,
                kind: ExprKind::Str(units.clone()),
            },
            PropKey::Num(n, sp) => Expr {
                span: *sp,
                kind: ExprKind::Num(*n),
            },
            PropKey::Computed(e) => self.lower_expr(e)?,
            PropKey::Private(id) => {
                let hidden = self.resolve_private(&id.name, id.span)?;
                str_expr(&hidden, id.span)
            }
            PropKey::BigInt(_, sp) => return err("BigInt literals are not supported yet", *sp),
        })
    }

    fn lower_expr(&mut self, e: &Expr) -> R<Expr> {
        self.depth.enter(e.span)?;
        let span = e.span;
        let kind = match &e.kind {
            ExprKind::Class(cls) => {
                let out = self.lower_class(cls, None)?;
                self.depth.leave();
                return Ok(out);
            }
            ExprKind::Function(f) => ExprKind::Function(Box::new(self.lower_function(f)?)),
            ExprKind::Arrow(a) => ExprKind::Arrow(Box::new(self.lower_arrow(a)?)),
            ExprKind::Await(arg) => {
                if !self.in_async {
                    self.depth.leave();
                    return err("async functions are not supported yet", span);
                }
                let lowered = self.lower_expr(arg)?;
                let out = call_promise_static("__unwrap", vec![lowered], span);
                self.depth.leave();
                return Ok(out);
            }
            ExprKind::PrivateIn { name, right } => {
                let hidden = self.resolve_private(&name.name, name.span)?;
                let r = self.lower_expr(right)?;
                ExprKind::Binary {
                    op: BinaryOp::In,
                    left: Box::new(str_expr(&hidden, name.span)),
                    right: Box::new(r),
                }
            }
            ExprKind::NewTarget => {
                if !self.in_function && self.classes.is_empty() {
                    self.depth.leave();
                    return err("'new.target' is not supported yet", span);
                }
                ExprKind::Logical {
                    op: LogicalOp::And,
                    left: Box::new(Expr {
                        span,
                        kind: ExprKind::This,
                    }),
                    right: Box::new(member_name(
                        Expr {
                            span,
                            kind: ExprKind::This,
                        },
                        "constructor",
                        span,
                    )),
                }
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } => {
                let mut lowered_args = Vec::with_capacity(args.len());
                for a in args {
                    lowered_args.push(self.lower_expr(a)?);
                }
                let stripped = super::expr::strip_parens(callee);
                if matches!(stripped.kind, ExprKind::Super) {
                    let Some((super_var, init_fn_var)) = self.classes.last().and_then(|c| {
                        c.super_var
                            .as_ref()
                            .map(|sv| (sv.clone(), c.init_fn_var.clone()))
                    }) else {
                        self.depth.leave();
                        return err("'super' is not supported yet", span);
                    };
                    let helper_span = self.fresh_span(span);
                    let mut call_args = vec![Expr {
                        span: helper_span,
                        kind: ExprKind::This,
                    }];
                    call_args.extend(lowered_args);
                    let sup_call = Expr {
                        span: helper_span,
                        kind: ExprKind::Call {
                            callee: Box::new(member_name(
                                ident_expr(&super_var, helper_span),
                                "call",
                                helper_span,
                            )),
                            args: call_args,
                            optional: false,
                        },
                    };
                    let init_call = Expr {
                        span: helper_span,
                        kind: ExprKind::Call {
                            callee: Box::new(ident_expr(&init_fn_var, helper_span)),
                            args: vec![Expr {
                                span: helper_span,
                                kind: ExprKind::This,
                            }],
                            optional: false,
                        },
                    };
                    let arrow = Expr {
                        span: helper_span,
                        kind: ExprKind::Arrow(Box::new(ArrowFunction {
                            params: Vec::new(),
                            body: ArrowBody::Block(vec![
                                let_stmt("__sup_ret", sup_call, helper_span),
                                copy_super_return_stmt("__sup_ret", helper_span),
                                Stmt {
                                    span: helper_span,
                                    kind: StmtKind::Return(Some(Box::new(init_call))),
                                },
                            ]),
                            is_async: false,
                            strict: true,
                            span: helper_span,
                        })),
                    };
                    self.depth.leave();
                    return Ok(Expr {
                        span,
                        kind: ExprKind::Call {
                            callee: Box::new(arrow),
                            args: Vec::new(),
                            optional: false,
                        },
                    });
                }
                if let ExprKind::Member {
                    object,
                    prop,
                    optional: mem_opt,
                } = &stripped.kind
                    && matches!(super::expr::strip_parens(object).kind, ExprKind::Super)
                {
                    let base = self.super_base_expr(object.span)?;
                    let lowered_prop = self.lower_member_prop(prop)?;
                    let method_get = Expr {
                        span: callee.span,
                        kind: ExprKind::Member {
                            object: Box::new(base),
                            prop: lowered_prop,
                            optional: *mem_opt,
                        },
                    };
                    let mut call_args = vec![Expr {
                        span,
                        kind: ExprKind::This,
                    }];
                    call_args.extend(lowered_args);
                    self.depth.leave();
                    return Ok(Expr {
                        span,
                        kind: ExprKind::Call {
                            callee: Box::new(member_name(method_get, "call", span)),
                            args: call_args,
                            optional: *optional,
                        },
                    });
                }
                ExprKind::Call {
                    callee: Box::new(self.lower_expr(callee)?),
                    args: lowered_args,
                    optional: *optional,
                }
            }
            ExprKind::New { callee, args } => {
                let mut lowered_args = Vec::with_capacity(args.len());
                for a in args {
                    lowered_args.push(self.lower_expr(a)?);
                }
                ExprKind::New {
                    callee: Box::new(self.lower_expr(callee)?),
                    args: lowered_args,
                }
            }
            ExprKind::Member {
                object,
                prop,
                optional,
            } => {
                let obj_expr = if matches!(super::expr::strip_parens(object).kind, ExprKind::Super)
                {
                    self.super_base_expr(object.span)?
                } else {
                    self.lower_expr(object)?
                };
                let lowered_prop = self.lower_member_prop(prop)?;
                ExprKind::Member {
                    object: Box::new(obj_expr),
                    prop: lowered_prop,
                    optional: *optional,
                }
            }
            ExprKind::Super => {
                self.depth.leave();
                return err("'super' is not supported yet", span);
            }
            ExprKind::Template(t) => {
                let mut exprs = Vec::with_capacity(t.exprs.len());
                for x in &t.exprs {
                    exprs.push(self.lower_expr(x)?);
                }
                ExprKind::Template(Template {
                    quasis: t.quasis.clone(),
                    exprs,
                })
            }
            ExprKind::TaggedTemplate { tag, quasi } => {
                let mut exprs = Vec::with_capacity(quasi.exprs.len());
                for x in &quasi.exprs {
                    exprs.push(self.lower_expr(x)?);
                }
                ExprKind::TaggedTemplate {
                    tag: Box::new(self.lower_expr(tag)?),
                    quasi: Template {
                        quasis: quasi.quasis.clone(),
                        exprs,
                    },
                }
            }
            ExprKind::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(match it {
                        Some(x) => Some(self.lower_expr(x)?),
                        None => None,
                    });
                }
                ExprKind::Array(out)
            }
            ExprKind::Object(props) => {
                let mut out = Vec::with_capacity(props.len());
                for p in props {
                    let pk = match &p.kind {
                        PropKind::Init { key, value } => PropKind::Init {
                            key: self.lower_prop_key(key)?,
                            value: Box::new(self.lower_expr(value)?),
                        },
                        PropKind::Shorthand(id) => PropKind::Shorthand(id.clone()),
                        PropKind::Method { key, kind, value } => PropKind::Method {
                            key: self.lower_prop_key(key)?,
                            kind: *kind,
                            value: Box::new(self.lower_function(value)?),
                        },
                        PropKind::Spread(x) => PropKind::Spread(Box::new(self.lower_expr(x)?)),
                    };
                    out.push(Prop {
                        kind: pk,
                        span: p.span,
                    });
                }
                ExprKind::Object(out)
            }
            ExprKind::Unary { op, arg } => ExprKind::Unary {
                op: *op,
                arg: Box::new(self.lower_expr(arg)?),
            },
            ExprKind::Update { op, prefix, arg } => ExprKind::Update {
                op: *op,
                prefix: *prefix,
                arg: Box::new(self.lower_expr(arg)?),
            },
            ExprKind::Binary { op, left, right } => ExprKind::Binary {
                op: *op,
                left: Box::new(self.lower_expr(left)?),
                right: Box::new(self.lower_expr(right)?),
            },
            ExprKind::Logical { op, left, right } => ExprKind::Logical {
                op: *op,
                left: Box::new(self.lower_expr(left)?),
                right: Box::new(self.lower_expr(right)?),
            },
            ExprKind::Assign { op, target, value } => ExprKind::Assign {
                op: *op,
                target: Box::new(self.lower_pat(target)?),
                value: Box::new(self.lower_expr(value)?),
            },
            ExprKind::Cond { test, cons, alt } => ExprKind::Cond {
                test: Box::new(self.lower_expr(test)?),
                cons: Box::new(self.lower_expr(cons)?),
                alt: Box::new(self.lower_expr(alt)?),
            },
            ExprKind::OptionalChain(inner) => {
                ExprKind::OptionalChain(Box::new(self.lower_expr(inner)?))
            }
            ExprKind::Seq(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.lower_expr(x)?);
                }
                ExprKind::Seq(out)
            }
            ExprKind::Spread(inner) => ExprKind::Spread(Box::new(self.lower_expr(inner)?)),
            ExprKind::Paren(inner) => ExprKind::Paren(Box::new(self.lower_expr(inner)?)),
            ExprKind::Yield { arg, delegate } => {
                let lowered_arg = match arg {
                    Some(a) => self.lower_expr(a)?,
                    None => ident_expr("undefined", span),
                };
                ExprKind::Call {
                    callee: Box::new(ident_expr("__anvil_yield", span)),
                    args: vec![
                        lowered_arg,
                        bool_expr(*delegate, span),
                        ident_expr("__anvil_yields", span),
                    ],
                    optional: false,
                }
            }
            ExprKind::ImportMeta => {
                ExprKind::Object(vec![prop_init("url", str_expr("about:blank", span), span)])
            }
            ExprKind::Import { source, .. } => {
                let s = self.lower_expr(source)?;
                let mod_call = Expr {
                    span,
                    kind: ExprKind::Call {
                        callee: Box::new(ident_expr("__anvil_import_module", span)),
                        args: vec![s],
                        optional: false,
                    },
                };
                call_promise_static("resolve", vec![mod_call], span)
                    .kind
                    .clone()
            }
            other => other.clone(),
        };
        self.depth.leave();
        Ok(Expr { kind, span })
    }

    fn lower_member_prop(&mut self, prop: &MemberProp) -> R<MemberProp> {
        Ok(match prop {
            MemberProp::Name(id) => MemberProp::Name(id.clone()),
            MemberProp::Computed(k) => MemberProp::Computed(Box::new(self.lower_expr(k)?)),
            MemberProp::Private(id) => {
                let hidden = self.resolve_private(&id.name, id.span)?;
                MemberProp::Name(Ident {
                    name: hidden,
                    span: id.span,
                })
            }
        })
    }

    fn super_base_expr(&self, span: Span) -> R<Expr> {
        let Some(ctx) = self.classes.last() else {
            return err("'super' is not supported yet", span);
        };
        let Some(sv) = &ctx.super_var else {
            return err("'super' is not supported yet", span);
        };
        if ctx.in_static {
            Ok(ident_expr(sv, span))
        } else {
            Ok(member_name(ident_expr(sv, span), "prototype", span))
        }
    }
}

fn ident_expr(name: &str, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Ident(name.to_string()),
    }
}

fn str_expr(s: &str, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Str(s.encode_utf16().collect()),
    }
}

fn bool_expr(b: bool, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Bool(b),
    }
}

fn member_name(obj: Expr, prop: &str, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Member {
            object: Box::new(obj),
            prop: MemberProp::Name(Ident {
                name: prop.to_string(),
                span,
            }),
            optional: false,
        },
    }
}

fn prop_init(name: &str, value: Expr, span: Span) -> Prop {
    Prop {
        span,
        kind: PropKind::Init {
            key: PropKey::Ident(Ident {
                name: name.to_string(),
                span,
            }),
            value: Box::new(value),
        },
    }
}

fn data_descriptor_expr(value: Expr, enumerable: bool, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Object(vec![
            prop_init("value", value, span),
            prop_init("writable", bool_expr(true, span), span),
            prop_init("enumerable", bool_expr(enumerable, span), span),
            prop_init("configurable", bool_expr(true, span), span),
        ]),
    }
}

fn call_object_static(method: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Call {
            callee: Box::new(member_name(ident_expr("Object", span), method, span)),
            args,
            optional: false,
        },
    }
}

fn call_promise_static(method: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr {
        span,
        kind: ExprKind::Call {
            callee: Box::new(member_name(ident_expr("Promise", span), method, span)),
            args,
            optional: false,
        },
    }
}

fn call_with_this(value_expr: Expr, this_expr: Expr, fn_span: Span) -> Expr {
    let fn_node = Expr {
        span: fn_span,
        kind: ExprKind::Function(Box::new(Function {
            id: None,
            params: Vec::new(),
            body: vec![Stmt {
                span: fn_span,
                kind: StmtKind::Return(Some(Box::new(value_expr))),
            }],
            is_async: false,
            is_generator: false,
            strict: true,
            simple_params: true,
            span: fn_span,
        })),
    };
    Expr {
        span: fn_span,
        kind: ExprKind::Call {
            callee: Box::new(member_name(fn_node, "call", fn_span)),
            args: vec![this_expr],
            optional: false,
        },
    }
}

fn expr_stmt(expr: Expr, span: Span) -> Stmt {
    Stmt {
        span,
        kind: StmtKind::Expr {
            expr: Box::new(expr),
            directive: None,
        },
    }
}

fn let_stmt(name: &str, init: Expr, span: Span) -> Stmt {
    Stmt {
        span,
        kind: StmtKind::Var(VarDecl {
            kind: VarKind::Var,
            decls: vec![VarDeclarator {
                target: Pat {
                    kind: PatKind::Ident(name.to_string()),
                    span,
                },
                init: Some(Box::new(init)),
                span,
            }],
            span,
        }),
    }
}

fn copy_super_return_stmt(ret_var: &str, span: Span) -> Stmt {
    let is_non_null = Expr {
        span,
        kind: ExprKind::Binary {
            op: BinaryOp::StrictNotEq,
            left: Box::new(ident_expr(ret_var, span)),
            right: Box::new(Expr {
                span,
                kind: ExprKind::Null,
            }),
        },
    };
    let typeof_ret = Expr {
        span,
        kind: ExprKind::Unary {
            op: UnaryOp::Typeof,
            arg: Box::new(ident_expr(ret_var, span)),
        },
    };
    let is_obj = Expr {
        span,
        kind: ExprKind::Binary {
            op: BinaryOp::StrictEq,
            left: Box::new(typeof_ret.clone()),
            right: Box::new(str_expr("object", span)),
        },
    };
    let is_fn = Expr {
        span,
        kind: ExprKind::Binary {
            op: BinaryOp::StrictEq,
            left: Box::new(typeof_ret),
            right: Box::new(str_expr("function", span)),
        },
    };
    let cond = Expr {
        span,
        kind: ExprKind::Logical {
            op: LogicalOp::And,
            left: Box::new(is_non_null),
            right: Box::new(Expr {
                span,
                kind: ExprKind::Logical {
                    op: LogicalOp::Or,
                    left: Box::new(is_obj),
                    right: Box::new(is_fn),
                },
            }),
        },
    };
    let assign_call = call_object_static(
        "assign",
        vec![
            Expr {
                span,
                kind: ExprKind::This,
            },
            ident_expr(ret_var, span),
        ],
        span,
    );
    Stmt {
        span,
        kind: StmtKind::If {
            test: Box::new(cond),
            cons: Box::new(expr_stmt(assign_call, span)),
            alt: None,
        },
    }
}

fn assign_export_stmt(exported_name: &str, value: Expr, span: Span) -> Stmt {
    let target = Expr {
        span,
        kind: ExprKind::Member {
            object: Box::new(ident_expr("__anvil_exports", span)),
            prop: MemberProp::Computed(Box::new(str_expr(exported_name, span))),
            optional: false,
        },
    };
    expr_stmt(
        Expr {
            span,
            kind: ExprKind::Assign {
                op: AssignOp::Assign,
                target: Box::new(Pat {
                    kind: PatKind::Expr(Box::new(target)),
                    span,
                }),
                value: Box::new(value),
            },
        },
        span,
    )
}

fn collect_decl_names(st: &Stmt) -> Vec<String> {
    let mut out = Vec::new();
    match &st.kind {
        StmtKind::Var(d) => {
            for dc in &d.decls {
                collect_pat_names(&dc.target, &mut out);
            }
        }
        StmtKind::Function(f) => {
            if let Some(id) = &f.id {
                out.push(id.name.clone());
            }
        }
        StmtKind::Class(c) => {
            if let Some(id) = &c.id {
                out.push(id.name.clone());
            }
        }
        _ => {}
    }
    out
}

fn collect_pat_names(pat: &Pat, out: &mut Vec<String>) {
    match &pat.kind {
        PatKind::Ident(n) => out.push(n.clone()),
        PatKind::Array(items) => {
            for p in items.iter().flatten() {
                collect_pat_names(p, out);
            }
        }
        PatKind::Object(props) => {
            for pr in props {
                match pr {
                    PatProp::Prop { value, .. } => collect_pat_names(value, out),
                    PatProp::Rest(r) => collect_pat_names(r, out),
                }
            }
        }
        PatKind::Assign { target, .. } => collect_pat_names(target, out),
        PatKind::Rest(r) => collect_pat_names(r, out),
        PatKind::Expr(_) => {}
    }
}
