// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Expressions and functions.
//!
//! Every expression is built into a destination register the caller
//! picks. Locals are copied out rather than read in place, so a later
//! operand that assigns the same name can't change an earlier one.

use super::scope::{BindKind, Tag};
use super::{Cg, Fs, Place, R, Store, err, simple_param, unsupported_fn};
use crate::cinder::ast::{
    ArrowBody, ArrowFunction, AssignOp, BinaryOp, Expr, ExprKind, Function, LogicalOp, MemberProp,
    MethodKind, Pat, PatKind, PatProp, Prop, PropKey, PropKind, Template, UnaryOp, UpdateOp,
};
use crate::cinder::bytecode::{BinOp, CodeBlock, Op, Reg, UnOp};
use crate::cinder::error::Span;
use crate::cinder::limits::group_digits;
use crate::cinder::vm::ErrorKind;
use std::collections::HashMap;

/// A function to compile.
#[derive(Clone, Copy)]
pub(crate) enum FnRef<'a> {
    /// A declaration (no self-name binding).
    Decl(&'a Function),
    /// An expression or method.
    Expr(&'a Function),
    /// An arrow.
    Arrow(&'a ArrowFunction),
}

fn bin_op(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Eq => BinOp::LooseEq,
        BinaryOp::NotEq => BinOp::LooseNe,
        BinaryOp::StrictEq => BinOp::StrictEq,
        BinaryOp::StrictNotEq => BinOp::StrictNe,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::LtEq => BinOp::Le,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::GtEq => BinOp::Ge,
        BinaryOp::Shl => BinOp::Shl,
        BinaryOp::Shr => BinOp::Shr,
        BinaryOp::UShr => BinOp::UShr,
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Mod,
        BinaryOp::Exp => BinOp::Exp,
        BinaryOp::BitOr => BinOp::BitOr,
        BinaryOp::BitXor => BinOp::BitXor,
        BinaryOp::BitAnd => BinOp::BitAnd,
        BinaryOp::In => BinOp::In,
        BinaryOp::Instanceof => BinOp::InstanceOf,
    }
}

fn assign_bin(op: AssignOp) -> Option<BinOp> {
    Some(match op {
        AssignOp::Add => BinOp::Add,
        AssignOp::Sub => BinOp::Sub,
        AssignOp::Mul => BinOp::Mul,
        AssignOp::Div => BinOp::Div,
        AssignOp::Mod => BinOp::Mod,
        AssignOp::Exp => BinOp::Exp,
        AssignOp::Shl => BinOp::Shl,
        AssignOp::Shr => BinOp::Shr,
        AssignOp::UShr => BinOp::UShr,
        AssignOp::BitOr => BinOp::BitOr,
        AssignOp::BitXor => BinOp::BitXor,
        AssignOp::BitAnd => BinOp::BitAnd,
        _ => return None,
    })
}

/// An assignment target, with its parts already in registers.
enum Target {
    Name(String, Place),
    Prop { obj: Reg, key: u32 },
    Elem { obj: Reg, key: Reg },
}

/// The name an anonymous function gets from where it's written.
fn anonymous_fn(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Function(f) => f.id.is_none(),
        ExprKind::Arrow(_) => true,
        _ => false,
    }
}

impl Cg<'_> {
    /// Build `e` into `dst`.
    pub fn expr(&mut self, e: &Expr, dst: Reg) -> R<()> {
        self.depth.enter(e.span)?;
        let r = self.expr_inner(e, dst, None);
        self.depth.leave();
        r
    }

    /// Build `e` into `dst`, naming it `name` if it's an anonymous function.
    pub fn expr_named(&mut self, e: &Expr, dst: Reg, name: &str) -> R<()> {
        if anonymous_fn(e) {
            self.depth.enter(e.span)?;
            let r = self.expr_inner(e, dst, Some(name));
            self.depth.leave();
            r
        } else {
            self.expr(e, dst)
        }
    }

    /// Build `e` into a fresh temporary.
    pub fn expr_tmp(&mut self, e: &Expr) -> R<Reg> {
        let t = self.tmp(e.span)?;
        self.expr(e, t)?;
        Ok(t)
    }

    fn expr_inner(&mut self, e: &Expr, dst: Reg, name: Option<&str>) -> R<()> {
        let span = e.span;
        match &e.kind {
            ExprKind::Ident(n) => {
                if n == "undefined"
                    && let Place::Global = self.resolve(n)
                {
                    self.emit(Op::LoadUndefined { dst });
                    return Ok(());
                }
                self.at(span);
                self.load_name(n, dst, span)?;
            }
            ExprKind::This => self.load_this(dst, span)?,
            ExprKind::Null => {
                self.emit(Op::LoadNull { dst });
            }
            ExprKind::Bool(b) => {
                self.emit(if *b {
                    Op::LoadTrue { dst }
                } else {
                    Op::LoadFalse { dst }
                });
            }
            ExprKind::Num(n) => self.load_num(*n, dst),
            ExprKind::Str(s) => {
                let idx = self.string_units(s);
                self.emit(Op::LoadString { dst, idx });
            }
            ExprKind::Template(t) => self.template(t, dst, span)?,
            ExprKind::Array(items) => self.array(items, dst, span)?,
            ExprKind::Object(props) => self.object(props, dst, span)?,
            ExprKind::Function(f) => {
                if f.is_async || f.is_generator {
                    return err(unsupported_fn(f.is_async, f.is_generator), span);
                }
                let nm =
                    f.id.as_ref()
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| name.unwrap_or("").to_string());
                self.closure(FnRef::Expr(f), &nm, dst)?;
            }
            ExprKind::Arrow(a) => {
                if a.is_async {
                    return err("async functions are not supported yet", span);
                }
                self.closure(FnRef::Arrow(a), name.unwrap_or(""), dst)?;
            }
            ExprKind::Unary { op, arg } => self.unary(*op, arg, dst, span)?,
            ExprKind::Update { op, prefix, arg } => self.update(*op, *prefix, arg, dst, span)?,
            ExprKind::Binary { op, left, right } => {
                self.expr(left, dst)?;
                let m = self.mark();
                let t = self.expr_tmp(right)?;
                self.at(span);
                self.emit(Op::Binary {
                    op: bin_op(*op),
                    dst,
                    a: dst,
                    b: t,
                });
                self.release(m);
            }
            ExprKind::Logical { op, left, right } => {
                self.expr(left, dst)?;
                match op {
                    LogicalOp::And => {
                        let j = self.emit(Op::JumpIfFalse {
                            cond: dst,
                            offset: 0,
                        });
                        self.expr(right, dst)?;
                        self.patch(j);
                    }
                    LogicalOp::Or => {
                        let j = self.emit(Op::JumpIfTrue {
                            cond: dst,
                            offset: 0,
                        });
                        self.expr(right, dst)?;
                        self.patch(j);
                    }
                    LogicalOp::Nullish => {
                        let j = self.emit(Op::JumpIfNullish {
                            cond: dst,
                            offset: 0,
                        });
                        let end = self.jump();
                        self.patch(j);
                        self.expr(right, dst)?;
                        self.patch(end);
                    }
                }
            }
            ExprKind::Assign { op, target, value } => self.assign(*op, target, value, dst, span)?,
            ExprKind::Cond { test, cons, alt } => {
                self.expr(test, dst)?;
                let j = self.emit(Op::JumpIfFalse {
                    cond: dst,
                    offset: 0,
                });
                self.expr(cons, dst)?;
                let end = self.jump();
                self.patch(j);
                self.expr(alt, dst)?;
                self.patch(end);
            }
            ExprKind::Call {
                callee,
                args,
                optional,
            } => self.call(callee, args, *optional, dst, span)?,
            ExprKind::New { callee, args } => {
                let m = self.mark();
                let f = self.expr_tmp(callee)?;
                if args.iter().any(|a| matches!(a.kind, ExprKind::Spread(_))) {
                    let arr = self.spread_args(args, span)?;
                    self.at(span);
                    self.emit(Op::NewSpread {
                        dst,
                        callee: f,
                        arr,
                    });
                } else {
                    let base = self.args(args, span)?;
                    self.at(span);
                    self.emit(Op::New {
                        dst,
                        callee: f,
                        args: base,
                        argc: args.len() as u16,
                    });
                }
                self.release(m);
            }
            ExprKind::Member {
                object,
                prop,
                optional,
            } => {
                let m = self.mark();
                let o = self.expr_tmp(object)?;
                if *optional {
                    self.optional_jump(o, span)?;
                }
                self.get_member(o, prop, dst, span)?;
                self.release(m);
            }
            ExprKind::OptionalChain(inner) => {
                self.f().chains.push(Vec::new());
                let r = self.expr(inner, dst);
                let jumps = self.f().chains.pop().unwrap_or_default();
                r?;
                if !jumps.is_empty() {
                    let end = self.jump();
                    for j in jumps {
                        self.patch(j);
                    }
                    self.emit(Op::LoadUndefined { dst });
                    self.patch(end);
                }
            }
            ExprKind::Seq(xs) => {
                for x in xs {
                    self.expr(x, dst)?;
                }
            }
            ExprKind::Paren(inner) => {
                return match name {
                    Some(n) => self.expr_named(inner, dst, n),
                    None => self.expr(inner, dst),
                };
            }
            ExprKind::Spread(_) => return err("unexpected spread", span),
            ExprKind::Regex { pattern, flags } => {
                let m = self.mark();
                let f = self.tmp(span)?;
                let key = self.key("RegExp");
                let ic = self.ic();
                self.emit(Op::GetGlobal { dst: f, key, ic });
                let args = self.tmps(2, span)?;
                self.load_str(pattern, args);
                self.load_str(flags, args + 1);
                self.at(span);
                self.emit(Op::New {
                    dst,
                    callee: f,
                    args,
                    argc: 2,
                });
                self.release(m);
            }
            ExprKind::BigInt(lit) => {
                let prefix = match lit.radix {
                    16 => "0x",
                    8 => "0o",
                    2 => "0b",
                    _ => "",
                };
                let text = format!("{prefix}{}", lit.digits);
                let m = self.mark();
                let f = self.tmp(span)?;
                let key = self.key("BigInt");
                let ic = self.ic();
                self.emit(Op::GetGlobal { dst: f, key, ic });
                let base = self.tmps(2, span)?;
                self.emit(Op::LoadUndefined { dst: base });
                self.load_str(&text, base + 1);
                self.at(span);
                self.emit(Op::Call {
                    dst,
                    callee: f,
                    this: base,
                    args: base + 1,
                    argc: 1,
                });
                self.release(m);
            }
            ExprKind::TaggedTemplate { tag, quasi } => {
                self.tagged_template(tag, quasi, dst, span)?;
            }
            ExprKind::Class(_) => return err("class expressions are not supported yet", span),
            ExprKind::Super => return err("'super' is not supported yet", span),
            ExprKind::Yield { .. } => {
                return err("generator functions are not supported yet", span);
            }
            ExprKind::Await(_) => return err("async functions are not supported yet", span),
            ExprKind::NewTarget => return err("'new.target' is not supported yet", span),
            ExprKind::ImportMeta | ExprKind::Import { .. } => {
                return err("modules are not supported yet", span);
            }
            ExprKind::PrivateIn { .. } => return err("private names are not supported yet", span),
        }
        Ok(())
    }

    fn load_this(&mut self, dst: Reg, span: Span) -> R<()> {
        let func = self.fr().func;
        if self.a.funcs[func].arrow {
            match self.resolve("this") {
                Place::Local { cs, bi } => {
                    self.load_place(Place::Local { cs, bi }, "this", dst, span)
                }
                Place::Global => err("internal compiler error: no 'this' binding", span),
            }
        } else {
            self.emit(Op::LoadThis { dst });
            Ok(())
        }
    }

    /// Jump to the end of the innermost optional chain when `r` is
    /// nullish.
    fn optional_jump(&mut self, r: Reg, span: Span) -> R<()> {
        let j = self.emit(Op::JumpIfNullish { cond: r, offset: 0 });
        match self.f().chains.last_mut() {
            Some(c) => {
                c.push(j);
                Ok(())
            }
            None => err(
                "internal compiler error: '?.' outside an optional chain",
                span,
            ),
        }
    }

    fn prop_key_name(&mut self, prop: &MemberProp, span: Span) -> R<Option<u32>> {
        match prop {
            MemberProp::Name(id) => Ok(Some(self.key(&id.name))),
            MemberProp::Computed(_) => Ok(None),
            MemberProp::Private(_) => err("private names are not supported yet", span),
        }
    }

    fn get_member(&mut self, obj: Reg, prop: &MemberProp, dst: Reg, span: Span) -> R<()> {
        match prop {
            MemberProp::Computed(k) => {
                let m = self.mark();
                let kr = self.expr_tmp(k)?;
                self.at(span);
                self.emit(Op::GetElem { dst, obj, key: kr });
                self.release(m);
            }
            _ => {
                let key = self.prop_key_name(prop, span)?.unwrap_or(0);
                let ic = self.ic();
                self.at(span);
                self.emit(Op::GetProp { dst, obj, key, ic });
            }
        }
        Ok(())
    }

    fn template(&mut self, t: &Template, dst: Reg, span: Span) -> R<()> {
        let cooked = |i: usize| -> R<&[u16]> {
            match t.quasis.get(i).and_then(|q| q.cooked.as_deref()) {
                Some(c) => Ok(c),
                None => err("invalid escape in template", span),
            }
        };
        let first = cooked(0)?.to_vec();
        let idx = self.string_units(&first);
        self.emit(Op::LoadString { dst, idx });
        for (i, x) in t.exprs.iter().enumerate() {
            let m = self.mark();
            let v = self.expr_tmp(x)?;
            // `${v}` is ToString(v); "" + v differs only for objects whose
            // valueOf and toString disagree.
            self.emit(Op::Binary {
                op: BinOp::Add,
                dst,
                a: dst,
                b: v,
            });
            let q = cooked(i + 1)?.to_vec();
            if !q.is_empty() {
                let idx = self.string_units(&q);
                self.emit(Op::LoadString { dst: v, idx });
                self.emit(Op::Binary {
                    op: BinOp::Add,
                    dst,
                    a: dst,
                    b: v,
                });
            }
            self.release(m);
        }
        Ok(())
    }

    fn tagged_template(&mut self, tag: &Expr, quasi: &Template, dst: Reg, span: Span) -> R<()> {
        let m = self.mark();
        let (f, this) = match &strip_parens(tag).kind {
            ExprKind::Member {
                object,
                prop,
                optional,
            } => {
                let this = self.expr_tmp(object)?;
                if *optional {
                    self.optional_jump(this, span)?;
                }
                let f = self.tmp(span)?;
                self.get_member(this, prop, f, tag.span)?;
                (f, this)
            }
            _ => {
                let f = self.expr_tmp(tag)?;
                let this = self.tmp(span)?;
                self.emit(Op::LoadUndefined { dst: this });
                (f, this)
            }
        };
        let strings = self.tmp(span)?;
        self.emit(Op::NewArray { dst: strings });
        let raw = self.tmp(span)?;
        self.emit(Op::NewArray { dst: raw });
        let t = self.tmp(span)?;
        for q in &quasi.quasis {
            match &q.cooked {
                Some(c) => {
                    let idx = self.string_units(c);
                    self.emit(Op::LoadString { dst: t, idx });
                }
                None => {
                    self.emit(Op::LoadUndefined { dst: t });
                }
            }
            self.emit(Op::ArrayPush {
                arr: strings,
                src: t,
            });
            self.load_str(&q.raw, t);
            self.emit(Op::ArrayPush { arr: raw, src: t });
        }
        let raw_key = self.key("raw");
        let ic = self.ic();
        self.emit(Op::SetProp {
            obj: strings,
            src: raw,
            key: raw_key,
            ic,
        });
        let argc = 1 + quasi.exprs.len();
        let base = self.tmps(argc, span)?;
        self.emit(Op::Move {
            dst: base,
            src: strings,
        });
        for (i, x) in quasi.exprs.iter().enumerate() {
            self.expr(x, base + 1 + i as Reg)?;
        }
        self.at(span);
        self.emit(Op::Call {
            dst,
            callee: f,
            this,
            args: base,
            argc: argc as u16,
        });
        self.release(m);
        Ok(())
    }

    fn array(&mut self, items: &[Option<Expr>], dst: Reg, span: Span) -> R<()> {
        let m = self.mark();
        let arr = self.tmp(span)?;
        self.emit(Op::NewArray { dst: arr });
        for it in items {
            let im = self.mark();
            match it {
                None => {
                    // A hole: bump the length.
                    let t = self.tmp(span)?;
                    let one = self.tmp(span)?;
                    let key = self.key("length");
                    let ic = self.ic();
                    self.emit(Op::GetProp {
                        dst: t,
                        obj: arr,
                        key,
                        ic,
                    });
                    self.emit(Op::LoadInt { dst: one, value: 1 });
                    self.emit(Op::Binary {
                        op: BinOp::Add,
                        dst: t,
                        a: t,
                        b: one,
                    });
                    let ic = self.ic();
                    self.emit(Op::SetProp {
                        obj: arr,
                        src: t,
                        key,
                        ic,
                    });
                }
                Some(Expr {
                    kind: ExprKind::Spread(x),
                    span: sp,
                }) => {
                    let v = self.expr_tmp(x)?;
                    self.at(*sp);
                    self.emit(Op::ArraySpread { arr, src: v });
                }
                Some(x) => {
                    let v = self.expr_tmp(x)?;
                    self.emit(Op::ArrayPush { arr, src: v });
                }
            }
            self.release(im);
        }
        self.emit(Op::Move { dst, src: arr });
        self.release(m);
        Ok(())
    }

    /// A property key into a register, and its text when it's known now
    /// (for function names).
    fn prop_key(&mut self, k: &PropKey, dst: Reg, span: Span) -> R<Option<String>> {
        Ok(match k {
            PropKey::Ident(id) => {
                self.load_str(&id.name, dst);
                Some(id.name.clone())
            }
            PropKey::Str(s, _) => {
                let idx = self.string_units(s);
                self.emit(Op::LoadString { dst, idx });
                Some(String::from_utf16_lossy(s))
            }
            PropKey::Num(n, _) => {
                self.load_num(*n, dst);
                Some(crate::cinder::value::number::number_to_string(*n))
            }
            PropKey::Computed(e) => {
                self.expr(e, dst)?;
                None
            }
            PropKey::BigInt(lit, _) => {
                self.load_str(&lit.digits, dst);
                Some(lit.digits.clone())
            }
            PropKey::Private(_) => return err("private names are not supported yet", span),
        })
    }

    fn object(&mut self, props: &[Prop], dst: Reg, span: Span) -> R<()> {
        let m = self.mark();
        let obj = self.tmp(span)?;
        self.emit(Op::NewObject { dst: obj });
        for p in props {
            let pm = self.mark();
            match &p.kind {
                PropKind::Init { key, value } if is_proto_key(key) => {
                    let v = self.expr_tmp(value)?;
                    self.emit(Op::SetProto { obj, src: v });
                }
                PropKind::Init { key, value } => {
                    let k = self.tmp(p.span)?;
                    let name = self.prop_key(key, k, p.span)?;
                    let v = self.tmp(p.span)?;
                    match &name {
                        Some(n) => self.expr_named(value, v, n)?,
                        None => self.expr(value, v)?,
                    }
                    self.emit(Op::DefineField {
                        obj,
                        key: k,
                        src: v,
                    });
                }
                PropKind::Shorthand(id) => {
                    let k = self.tmp(p.span)?;
                    self.load_str(&id.name, k);
                    let v = self.tmp(p.span)?;
                    self.load_name(&id.name, v, id.span)?;
                    self.emit(Op::DefineField {
                        obj,
                        key: k,
                        src: v,
                    });
                }
                PropKind::Method { key, kind, value } => {
                    if value.is_async || value.is_generator {
                        return err(unsupported_fn(value.is_async, value.is_generator), p.span);
                    }
                    let k = self.tmp(p.span)?;
                    let name = self.prop_key(key, k, p.span)?.unwrap_or_default();
                    let v = self.tmp(p.span)?;
                    let fname = match kind {
                        MethodKind::Get => format!("get {name}"),
                        MethodKind::Set => format!("set {name}"),
                        _ => name,
                    };
                    self.closure(FnRef::Decl(value), &fname, v)?;
                    match kind {
                        MethodKind::Get | MethodKind::Set => {
                            self.emit(Op::DefineAccessor {
                                obj,
                                key: k,
                                func: v,
                                setter: *kind == MethodKind::Set,
                            });
                        }
                        _ => {
                            self.emit(Op::DefineField {
                                obj,
                                key: k,
                                src: v,
                            });
                        }
                    }
                }
                PropKind::Spread(e) => {
                    let v = self.expr_tmp(e)?;
                    self.emit(Op::CopyProps { obj, src: v });
                }
            }
            self.release(pm);
        }
        self.emit(Op::Move { dst, src: obj });
        self.release(m);
        Ok(())
    }

    fn unary(&mut self, op: UnaryOp, arg: &Expr, dst: Reg, span: Span) -> R<()> {
        let uop = match op {
            UnaryOp::Minus => UnOp::Neg,
            UnaryOp::Plus => UnOp::Plus,
            UnaryOp::Not => UnOp::Not,
            UnaryOp::BitNot => UnOp::BitNot,
            UnaryOp::Typeof => {
                if let ExprKind::Ident(n) = &strip_parens(arg).kind {
                    if let Place::Global = self.resolve(n) {
                        let key = self.key(n);
                        self.emit(Op::TypeOfGlobal { dst, key });
                        return Ok(());
                    }
                    if let Place::Local { cs, bi } = self.resolve(n)
                        && self.scopes[cs].stores[bi] == Store::Global
                    {
                        let key = self.key(n);
                        self.emit(Op::TypeOfGlobal { dst, key });
                        return Ok(());
                    }
                }
                UnOp::TypeOf
            }
            UnaryOp::Void => {
                self.expr(arg, dst)?;
                self.emit(Op::LoadUndefined { dst });
                return Ok(());
            }
            UnaryOp::Delete => return self.delete(arg, dst, span),
        };
        self.expr(arg, dst)?;
        self.at(span);
        self.emit(Op::Unary {
            op: uop,
            dst,
            src: dst,
        });
        Ok(())
    }

    fn delete(&mut self, arg: &Expr, dst: Reg, span: Span) -> R<()> {
        match &strip_parens(arg).kind {
            ExprKind::Member {
                object,
                prop,
                optional: false,
            } => {
                let m = self.mark();
                let o = self.expr_tmp(object)?;
                let k = self.tmp(span)?;
                match prop {
                    MemberProp::Name(id) => self.load_str(&id.name, k),
                    MemberProp::Computed(e) => self.expr(e, k)?,
                    MemberProp::Private(_) => {
                        return err("private names are not supported yet", span);
                    }
                }
                self.at(span);
                self.emit(Op::DeleteElem {
                    dst,
                    obj: o,
                    key: k,
                });
                self.release(m);
            }
            ExprKind::OptionalChain(_) | ExprKind::Member { .. } => {
                return err("'delete' of an optional chain is not supported yet", span);
            }
            ExprKind::Ident(n) => match self.resolve(n) {
                Place::Local { cs, bi } if self.scopes[cs].stores[bi] != Store::Global => {
                    self.emit(Op::LoadFalse { dst });
                }
                _ => {
                    let m = self.mark();
                    let o = self.tmp(span)?;
                    let k = self.tmp(span)?;
                    self.emit(Op::LoadGlobalObject { dst: o });
                    self.load_str(n, k);
                    self.emit(Op::DeleteElem {
                        dst,
                        obj: o,
                        key: k,
                    });
                    self.release(m);
                }
            },
            _ => {
                self.expr(arg, dst)?;
                self.emit(Op::LoadTrue { dst });
            }
        }
        Ok(())
    }

    /// Evaluate the parts of an assignment target (object, key) into
    /// registers, in source order.
    fn target(&mut self, p: &Pat) -> R<Target> {
        match &p.kind {
            PatKind::Ident(n) => Ok(Target::Name(n.clone(), self.resolve(n))),
            PatKind::Expr(e) => self.target_expr(e, p.span),
            _ => err("destructuring patterns are not supported yet", p.span),
        }
    }

    fn target_expr(&mut self, e: &Expr, span: Span) -> R<Target> {
        match &strip_parens(e).kind {
            ExprKind::Ident(n) => Ok(Target::Name(n.clone(), self.resolve(n))),
            ExprKind::Member {
                object,
                prop,
                optional: false,
            } => {
                let o = self.expr_tmp(object)?;
                match prop {
                    MemberProp::Name(id) => Ok(Target::Prop {
                        obj: o,
                        key: self.key(&id.name),
                    }),
                    MemberProp::Computed(k) => {
                        let kr = self.expr_tmp(k)?;
                        Ok(Target::Elem { obj: o, key: kr })
                    }
                    MemberProp::Private(_) => err("private names are not supported yet", span),
                }
            }
            _ => err("invalid assignment target", span),
        }
    }

    fn read_target(&mut self, t: &Target, dst: Reg, span: Span) -> R<()> {
        match t {
            Target::Name(n, place) => self.load_place(*place, n, dst, span),
            Target::Prop { obj, key } => {
                let ic = self.ic();
                self.emit(Op::GetProp {
                    dst,
                    obj: *obj,
                    key: *key,
                    ic,
                });
                Ok(())
            }
            Target::Elem { obj, key } => {
                self.emit(Op::GetElem {
                    dst,
                    obj: *obj,
                    key: *key,
                });
                Ok(())
            }
        }
    }

    fn write_target(&mut self, t: &Target, src: Reg, span: Span) -> R<()> {
        self.at(span);
        match t {
            Target::Name(n, place) => self.store_place(*place, n, src, false, span),
            Target::Prop { obj, key } => {
                let ic = self.ic();
                self.emit(Op::SetProp {
                    obj: *obj,
                    src,
                    key: *key,
                    ic,
                });
                Ok(())
            }
            Target::Elem { obj, key } => {
                self.emit(Op::SetElem {
                    obj: *obj,
                    key: *key,
                    src,
                });
                Ok(())
            }
        }
    }

    /// Store `src` into the pattern `p`.
    pub fn assign_pat(&mut self, p: &Pat, src: Reg, init: bool) -> R<()> {
        self.depth.enter(p.span)?;
        let r = self.assign_pat_inner(p, src, init);
        self.depth.leave();
        r
    }

    fn assign_pat_inner(&mut self, p: &Pat, src: Reg, init: bool) -> R<()> {
        let m = self.mark();
        match &p.kind {
            PatKind::Ident(n) if init => {
                self.store_name(n, src, true, p.span)?;
            }
            PatKind::Ident(_) | PatKind::Expr(_) => {
                let t = self.target(p)?;
                self.write_target(&t, src, p.span)?;
            }
            PatKind::Assign { target, default } => {
                let val = self.tmp(p.span)?;
                self.emit(Op::Move { dst: val, src });
                let u = self.tmp(p.span)?;
                self.emit(Op::LoadUndefined { dst: u });
                self.emit(Op::Binary {
                    op: BinOp::StrictEq,
                    dst: u,
                    a: val,
                    b: u,
                });
                let skip = self.emit(Op::JumpIfFalse { cond: u, offset: 0 });
                if let PatKind::Ident(n) = &target.kind {
                    self.expr_named(default, val, n)?;
                } else {
                    self.expr(default, val)?;
                }
                self.patch(skip);
                self.assign_pat(target, val, init)?;
            }
            PatKind::Array(items) => {
                let arr = self.tmp(p.span)?;
                self.at(p.span);
                self.emit(Op::IterPrepare { dst: arr, src });
                for (idx, item) in items.iter().enumerate() {
                    let Some(sub) = item else { continue };
                    let im = self.mark();
                    if let PatKind::Rest(rest_target) = &sub.kind {
                        let slice_fn = self.tmp(sub.span)?;
                        let key = self.key("slice");
                        let ic = self.ic();
                        self.emit(Op::GetProp {
                            dst: slice_fn,
                            obj: arr,
                            key,
                            ic,
                        });
                        let arg = self.tmp(sub.span)?;
                        self.load_num(idx as f64, arg);
                        let rest_arr = self.tmp(sub.span)?;
                        self.emit(Op::Call {
                            dst: rest_arr,
                            callee: slice_fn,
                            this: arr,
                            args: arg,
                            argc: 1,
                        });
                        self.assign_pat(rest_target, rest_arr, init)?;
                    } else {
                        let k = self.tmp(sub.span)?;
                        self.load_num(idx as f64, k);
                        let elem = self.tmp(sub.span)?;
                        self.emit(Op::GetElem {
                            dst: elem,
                            obj: arr,
                            key: k,
                        });
                        self.assign_pat(sub, elem, init)?;
                    }
                    self.release(im);
                }
            }
            PatKind::Object(props) => {
                let bad = self.emit(Op::JumpIfNullish {
                    cond: src,
                    offset: 0,
                });
                let ok = self.jump();
                self.patch(bad);
                self.throw_error(ErrorKind::Type, "Cannot destructure null or undefined");
                self.patch(ok);
                let has_rest = props.iter().any(|pr| matches!(pr, PatProp::Rest(_)));
                let mut used_keys: Vec<Reg> = Vec::new();
                for pr in props {
                    match pr {
                        PatProp::Prop { key, value, .. } => {
                            if has_rest {
                                let k = self.tmp(value.span)?;
                                self.prop_key(key, k, value.span)?;
                                used_keys.push(k);
                                let pm = self.mark();
                                let v = self.tmp(value.span)?;
                                self.at(value.span);
                                self.emit(Op::GetElem {
                                    dst: v,
                                    obj: src,
                                    key: k,
                                });
                                self.assign_pat(value, v, init)?;
                                self.release(pm);
                            } else {
                                let pm = self.mark();
                                let v = self.tmp(value.span)?;
                                if let PropKey::Ident(id) = key {
                                    let k = self.key(&id.name);
                                    let ic = self.ic();
                                    self.at(value.span);
                                    self.emit(Op::GetProp {
                                        dst: v,
                                        obj: src,
                                        key: k,
                                        ic,
                                    });
                                } else {
                                    let k = self.tmp(value.span)?;
                                    self.prop_key(key, k, value.span)?;
                                    self.at(value.span);
                                    self.emit(Op::GetElem {
                                        dst: v,
                                        obj: src,
                                        key: k,
                                    });
                                }
                                self.assign_pat(value, v, init)?;
                                self.release(pm);
                            }
                        }
                        PatProp::Rest(rest_target) => {
                            let pm = self.mark();
                            let rest_obj = self.tmp(rest_target.span)?;
                            self.emit(Op::NewObject { dst: rest_obj });
                            self.emit(Op::CopyProps { obj: rest_obj, src });
                            let dummy = self.tmp(rest_target.span)?;
                            for &k in &used_keys {
                                self.emit(Op::DeleteElem {
                                    dst: dummy,
                                    obj: rest_obj,
                                    key: k,
                                });
                            }
                            self.assign_pat(rest_target, rest_obj, init)?;
                            self.release(pm);
                        }
                    }
                }
            }
            PatKind::Rest(inner) => {
                self.assign_pat(inner, src, init)?;
            }
        }
        self.release(m);
        Ok(())
    }

    fn assign(&mut self, op: AssignOp, target: &Pat, value: &Expr, dst: Reg, span: Span) -> R<()> {
        if op == AssignOp::Assign
            && matches!(
                target.kind,
                PatKind::Array(_) | PatKind::Object(_) | PatKind::Assign { .. }
            )
        {
            self.expr(value, dst)?;
            self.assign_pat(target, dst, false)?;
            return Ok(());
        }
        let m = self.mark();
        let t = self.target(target)?;
        let name = match &t {
            Target::Name(n, _) => Some(n.clone()),
            _ => None,
        };
        match op {
            AssignOp::Assign => {
                match &name {
                    Some(n) => self.expr_named(value, dst, n)?,
                    None => self.expr(value, dst)?,
                }
                self.write_target(&t, dst, span)?;
            }
            AssignOp::And | AssignOp::Or | AssignOp::Nullish => {
                self.read_target(&t, dst, span)?;
                let skip = match op {
                    AssignOp::And => self.emit(Op::JumpIfFalse {
                        cond: dst,
                        offset: 0,
                    }),
                    AssignOp::Or => self.emit(Op::JumpIfTrue {
                        cond: dst,
                        offset: 0,
                    }),
                    _ => {
                        let j = self.emit(Op::JumpIfNullish {
                            cond: dst,
                            offset: 0,
                        });
                        let skip = self.jump();
                        self.patch(j);
                        skip
                    }
                };
                match &name {
                    Some(n) => self.expr_named(value, dst, n)?,
                    None => self.expr(value, dst)?,
                }
                self.write_target(&t, dst, span)?;
                self.patch(skip);
            }
            _ => {
                let bop = assign_bin(op).unwrap_or(BinOp::Add);
                self.read_target(&t, dst, span)?;
                let v = self.expr_tmp(value)?;
                self.at(span);
                self.emit(Op::Binary {
                    op: bop,
                    dst,
                    a: dst,
                    b: v,
                });
                self.write_target(&t, dst, span)?;
            }
        }
        self.release(m);
        Ok(())
    }

    fn update(&mut self, op: UpdateOp, prefix: bool, arg: &Expr, dst: Reg, span: Span) -> R<()> {
        let m = self.mark();
        let t = self.target_expr(arg, arg.span)?;
        let uop = match op {
            UpdateOp::Inc => UnOp::Inc,
            UpdateOp::Dec => UnOp::Dec,
        };
        self.read_target(&t, dst, span)?;
        self.at(span);
        if prefix {
            self.emit(Op::Unary {
                op: uop,
                dst,
                src: dst,
            });
            self.write_target(&t, dst, span)?;
        } else {
            self.emit(Op::Unary {
                op: UnOp::Plus,
                dst,
                src: dst,
            });
            let n = self.tmp(span)?;
            self.emit(Op::Unary {
                op: uop,
                dst: n,
                src: dst,
            });
            self.write_target(&t, n, span)?;
        }
        self.release(m);
        Ok(())
    }

    /// Arguments into consecutive registers; returns the first.
    fn args(&mut self, args: &[Expr], span: Span) -> R<Reg> {
        let base = self.tmps(args.len(), span)?;
        for (i, a) in args.iter().enumerate() {
            self.expr(a, base + i as Reg)?;
        }
        Ok(base)
    }

    /// Arguments with spread, into a new array.
    fn spread_args(&mut self, args: &[Expr], span: Span) -> R<Reg> {
        let arr = self.tmp(span)?;
        self.emit(Op::NewArray { dst: arr });
        for a in args {
            let m = self.mark();
            match &a.kind {
                ExprKind::Spread(x) => {
                    let v = self.expr_tmp(x)?;
                    self.at(a.span);
                    self.emit(Op::ArraySpread { arr, src: v });
                }
                _ => {
                    let v = self.expr_tmp(a)?;
                    self.emit(Op::ArrayPush { arr, src: v });
                }
            }
            self.release(m);
        }
        Ok(arr)
    }

    fn call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        optional: bool,
        dst: Reg,
        span: Span,
    ) -> R<()> {
        let m = self.mark();
        let spread = args.iter().any(|a| matches!(a.kind, ExprKind::Spread(_)));
        if args.len() > u16::MAX as usize && !spread {
            return err(
                format!(
                    "too many arguments: {} (at most 65,535)",
                    group_digits(args.len() as u64)
                ),
                span,
            );
        }
        let (f, this) = match &strip_parens(callee).kind {
            ExprKind::Member {
                object,
                prop,
                optional: opt,
            } => {
                let this = self.expr_tmp(object)?;
                if *opt {
                    self.optional_jump(this, span)?;
                }
                let f = self.tmp(span)?;
                self.get_member(this, prop, f, callee.span)?;
                (f, this)
            }
            ExprKind::Ident(n) if n == "eval" && !spread => {
                if let Place::Global = self.resolve(n) {
                    self.direct_eval_allowed(span)?;
                    let base = self.args(args, span)?;
                    self.at(span);
                    self.emit(Op::DirectEval {
                        dst,
                        args: base,
                        argc: args.len() as u16,
                    });
                    self.release(m);
                    return Ok(());
                }
                let f = self.expr_tmp(callee)?;
                let this = self.tmp(span)?;
                self.emit(Op::LoadUndefined { dst: this });
                (f, this)
            }
            _ => {
                let f = self.expr_tmp(callee)?;
                let this = self.tmp(span)?;
                self.emit(Op::LoadUndefined { dst: this });
                (f, this)
            }
        };
        if optional {
            self.optional_jump(f, span)?;
        }
        if spread {
            let arr = self.spread_args(args, span)?;
            self.at(span);
            self.emit(Op::CallSpread {
                dst,
                callee: f,
                this,
                arr,
            });
        } else {
            let base = self.args(args, span)?;
            self.at(span);
            self.emit(Op::Call {
                dst,
                callee: f,
                this,
                args: base,
                argc: args.len() as u16,
            });
        }
        self.release(m);
        Ok(())
    }

    /// Direct eval runs its code at global scope, so it's only allowed
    /// where that is the scope it would see: the program's top level, with
    /// no block-scoped names around it.
    fn direct_eval_allowed(&self, span: Span) -> R<()> {
        let ok = self.fns.len() == 1
            && self.scopes.iter().all(|cs| {
                self.a.scopes[cs.id]
                    .bindings
                    .iter()
                    .zip(&cs.stores)
                    .all(|(b, st)| *st == Store::Global || b.kind == BindKind::This)
            });
        if ok {
            Ok(())
        } else {
            err(
                "direct eval is only supported at the top level of a script, outside \
                 functions and blocks with their own declarations; use indirect eval, \
                 such as (0, eval)(code), which runs the code at global scope",
                span,
            )
        }
    }

    // ---- functions --------------------------------------------------------

    /// Compile a function and put a closure for it in `dst`.
    pub fn closure(&mut self, f: FnRef<'_>, name: &str, dst: Reg) -> R<()> {
        let idx = self.function(f, name)?;
        self.emit(Op::Closure { dst, func: idx });
        Ok(())
    }

    fn function(&mut self, f: FnRef<'_>, name: &str) -> R<u32> {
        let (span, params, strict) = match f {
            FnRef::Decl(x) | FnRef::Expr(x) => (x.span, &x.params, x.strict),
            FnRef::Arrow(a) => (a.span, &a.params, a.strict),
        };
        self.depth.enter(span)?;
        let Some(sid) = self.a.scope_at(span, Tag::Func) else {
            return err("internal compiler error: function scope not found", span);
        };
        let func = self.a.scopes[sid].func;
        // Plain parameters sit in registers 0..n; a rest parameter gets
        // the remaining arguments as an array.
        let mut fixed = 0usize;
        for p in params.iter() {
            if !matches!(p.kind, PatKind::Rest(_)) {
                fixed += 1;
            }
        }
        if fixed > u16::MAX as usize {
            return err("too many parameters (at most 65,535)", span);
        }
        let mut b = CodeBlock::new(name, fixed as u16, 0);
        b.strict = strict;
        b.span = span;
        self.fns.push(Fs {
            b,
            regs: super::regalloc::RegAlloc::with_fixed(fixed as u32),
            func,
            env_depth: 0,
            controls: Vec::new(),
            ret_reg: 0,
            completion: None,
            chains: Vec::new(),
            strings: HashMap::new(),
            last_span: None,
        });
        let outer_scopes = self.scopes.len();
        let r = self.function_body(f, sid, params, span);
        self.scopes.truncate(outer_scopes);
        let mut fs = match self.fns.pop() {
            Some(fs) => fs,
            None => return err("internal compiler error: no function", span),
        };
        r?;
        fs.b.register_count = fs.regs.high_water();
        self.depth.leave();
        let parent = self.f();
        let idx = parent.b.functions.len() as u32;
        parent.b.functions.push(fs.b);
        Ok(idx)
    }

    fn function_body(&mut self, f: FnRef<'_>, sid: usize, raw_params: &[Pat], span: Span) -> R<()> {
        let mut params = HashMap::new();
        let mut reg = 0 as Reg;
        for p in raw_params {
            if !matches!(p.kind, PatKind::Rest(_)) {
                if let Some((n, _, false)) = simple_param(p) {
                    params.insert(n.to_string(), reg);
                }
                reg += 1;
            }
        }
        let ret = self.tmp(span)?;
        self.f().ret_reg = ret;
        self.enter_scope(sid, &params, span)?;
        let cs = self.scopes.len() - 1;
        let func = self.fr().func;
        // arguments
        if let Some(bi) = self.a.scopes[sid].find("arguments")
            && self.a.scopes[sid].bindings[bi].kind == BindKind::Arguments
        {
            let r = match self.scopes[cs].stores[bi] {
                Store::Reg(r) => r,
                _ => self.tmp(span)?,
            };
            self.f().b.arguments_reg = Some(r);
            let place = Place::Local { cs, bi };
            self.store_place(place, "arguments", r, true, span)?;
            if self.a.funcs[func].mapped {
                self.emit(Op::MapArguments { src: r });
            }
        }
        // rest parameter
        let mut complex_rest_reg: Option<Reg> = None;
        for p in raw_params {
            if let PatKind::Rest(inner) = &p.kind {
                if let PatKind::Ident(n) = &inner.kind {
                    let place = self.resolve(n);
                    let r = match place {
                        Place::Local { cs, bi } => match self.scopes[cs].stores[bi] {
                            Store::Reg(r) => r,
                            _ => self.tmp(span)?,
                        },
                        Place::Global => self.tmp(span)?,
                    };
                    self.f().b.rest_reg = Some(r);
                    self.store_place(place, n, r, true, span)?;
                } else {
                    let r = self.tmp(span)?;
                    self.f().b.rest_reg = Some(r);
                    complex_rest_reg = Some(r);
                }
            }
        }
        self.pseudo_bindings(span)?;
        // defaults & destructured parameters
        let mut pos_reg = 0 as Reg;
        for p in raw_params {
            if let PatKind::Rest(inner) = &p.kind {
                if let Some(r) = complex_rest_reg {
                    self.assign_pat(inner, r, true)?;
                }
            } else {
                if let Some((n, def, false)) = simple_param(p) {
                    if let Some(d) = def {
                        let m = self.mark();
                        let t = self.tmp(span)?;
                        self.load_name(n, t, span)?;
                        let u = self.tmp(span)?;
                        self.emit(Op::LoadUndefined { dst: u });
                        self.emit(Op::Binary {
                            op: BinOp::StrictEq,
                            dst: u,
                            a: t,
                            b: u,
                        });
                        let skip = self.emit(Op::JumpIfFalse { cond: u, offset: 0 });
                        self.expr_named(d, t, n)?;
                        self.store_name(n, t, true, span)?;
                        self.patch(skip);
                        self.release(m);
                    }
                } else {
                    self.assign_pat(p, pos_reg, true)?;
                }
                pos_reg += 1;
            }
        }
        match f {
            FnRef::Decl(x) | FnRef::Expr(x) => {
                self.hoist_functions(&x.body)?;
                self.stmts(&x.body)?;
                self.emit(Op::LoadUndefined { dst: ret });
                self.emit(Op::Return { src: ret });
            }
            FnRef::Arrow(a) => match &a.body {
                ArrowBody::Expr(e) => {
                    self.expr(e, ret)?;
                    self.emit(Op::Return { src: ret });
                }
                ArrowBody::Block(body) => {
                    self.hoist_functions(body)?;
                    self.stmts(body)?;
                    self.emit(Op::LoadUndefined { dst: ret });
                    self.emit(Op::Return { src: ret });
                }
            },
        }
        Ok(())
    }
}

/// `__proto__` written as a plain or string key (not computed).
fn is_proto_key(k: &PropKey) -> bool {
    match k {
        PropKey::Ident(id) => id.name == "__proto__",
        PropKey::Str(s, _) => *s == "__proto__".encode_utf16().collect::<Vec<u16>>(),
        _ => false,
    }
}

/// `(((e)))` is `e`.
pub(crate) fn strip_parens(mut e: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &e.kind {
        e = inner;
    }
    e
}
