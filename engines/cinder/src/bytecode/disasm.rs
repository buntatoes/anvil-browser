// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! A text listing of a code block, for tests and debugging.

use super::{BinOp, CodeBlock, Op, UnOp};
use std::fmt::Write;

fn unop_name(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Plus => "plus",
        UnOp::Not => "not",
        UnOp::BitNot => "bitnot",
        UnOp::TypeOf => "typeof",
        UnOp::Inc => "inc",
        UnOp::Dec => "dec",
    }
}

fn binop_name(op: BinOp) -> &'static str {
    op.symbol()
}

fn key(block: &CodeBlock, k: u32) -> String {
    block
        .keys
        .get(k as usize)
        .map(|u| String::from_utf16_lossy(u))
        .unwrap_or_else(|| format!("<bad key {k}>"))
}

fn target(pc: usize, offset: i32) -> i64 {
    pc as i64 + 1 + i64::from(offset)
}

/// One op as text.
pub fn op_text(block: &CodeBlock, pc: usize, op: &Op) -> String {
    match *op {
        Op::Nop => "nop".to_string(),
        Op::LoadNumber { dst, idx } => {
            let n = block.numbers.get(idx as usize).copied().unwrap_or(f64::NAN);
            format!(
                "r{dst} = {}",
                crate::cinder::value::number::number_to_string(n)
            )
        }
        Op::LoadInt { dst, value } => format!("r{dst} = {value}"),
        Op::LoadString { dst, idx } => {
            let s = block
                .strings
                .get(idx as usize)
                .map(|u| String::from_utf16_lossy(u))
                .unwrap_or_default();
            format!("r{dst} = {s:?}")
        }
        Op::LoadUndefined { dst } => format!("r{dst} = undefined"),
        Op::LoadNull { dst } => format!("r{dst} = null"),
        Op::LoadTrue { dst } => format!("r{dst} = true"),
        Op::LoadFalse { dst } => format!("r{dst} = false"),
        Op::LoadUninit { dst } => format!("r{dst} = <uninitialized>"),
        Op::LoadThis { dst } => format!("r{dst} = this"),
        Op::Move { dst, src } => format!("r{dst} = r{src}"),
        Op::Binary { op, dst, a, b } => format!("r{dst} = r{a} {} r{b}", binop_name(op)),
        Op::Unary { op, dst, src } => format!("r{dst} = {} r{src}", unop_name(op)),
        Op::Jump { offset } => format!("jump {}", target(pc, offset)),
        Op::JumpIfTrue { cond, offset } => format!("if r{cond} jump {}", target(pc, offset)),
        Op::JumpIfFalse { cond, offset } => format!("unless r{cond} jump {}", target(pc, offset)),
        Op::JumpIfNullish { cond, offset } => {
            format!("if nullish r{cond} jump {}", target(pc, offset))
        }
        Op::LoopHeader { id } => format!("loop {id}"),
        Op::CheckTdz { src, name } => format!("check r{src} ({})", key(block, name)),
        Op::GetGlobal { dst, key: k, ic } => format!("r{dst} = global.{} [ic {ic}]", key(block, k)),
        Op::SetGlobal { src, key: k, ic } => format!("global.{} = r{src} [ic {ic}]", key(block, k)),
        Op::PushEnv { size } => format!("push env {size}"),
        Op::PopEnv => "pop env".to_string(),
        Op::GetEnv { dst, depth, slot } => format!("r{dst} = env{depth}[{slot}]"),
        Op::SetEnv { src, depth, slot } => format!("env{depth}[{slot}] = r{src}"),
        Op::NewObject { dst } => format!("r{dst} = {{}}"),
        Op::NewArray { dst } => format!("r{dst} = []"),
        Op::GetProp {
            dst,
            obj,
            key: k,
            ic,
        } => {
            format!("r{dst} = r{obj}.{} [ic {ic}]", key(block, k))
        }
        Op::SetProp {
            obj,
            src,
            key: k,
            ic,
        } => {
            format!("r{obj}.{} = r{src} [ic {ic}]", key(block, k))
        }
        Op::GetElem { dst, obj, key: k } => format!("r{dst} = r{obj}[r{k}]"),
        Op::SetElem { obj, key: k, src } => format!("r{obj}[r{k}] = r{src}"),
        Op::DeleteElem { dst, obj, key: k } => format!("r{dst} = delete r{obj}[r{k}]"),
        Op::Closure { dst, func } => {
            let name = block
                .functions
                .get(func as usize)
                .map(|f| f.display_name().to_string())
                .unwrap_or_default();
            format!("r{dst} = closure {func} ({name})")
        }
        Op::Call {
            dst,
            callee,
            this,
            args,
            argc,
        } => format!("r{dst} = call r{callee} this=r{this} args=r{args}+{argc}"),
        Op::New {
            dst,
            callee,
            args,
            argc,
        } => format!("r{dst} = new r{callee} args=r{args}+{argc}"),
        Op::Return { src } => format!("return r{src}"),
        Op::LoadGlobalObject { dst } => format!("r{dst} = global object"),
        Op::LoadCallee { dst } => format!("r{dst} = callee"),
        Op::TypeOfGlobal { dst, key: k } => format!("r{dst} = typeof global.{}", key(block, k)),
        Op::DeclareVar { key: k } => format!("declare var {}", key(block, k)),
        Op::DeclareLex { key: k, konst } => {
            format!(
                "declare {} {}",
                if konst { "const" } else { "let" },
                key(block, k)
            )
        }
        Op::InitLex { src, key: k } => format!("init {} = r{src}", key(block, k)),
        Op::ThrowError { kind, msg } => {
            let s = block
                .strings
                .get(msg as usize)
                .map(|u| String::from_utf16_lossy(u))
                .unwrap_or_default();
            format!("throw error {kind} {s:?}")
        }
        Op::DefineField { obj, key: k, src } => format!("define r{obj}[r{k}] = r{src}"),
        Op::DefineAccessor {
            obj,
            key: k,
            func,
            setter,
        } => format!(
            "define {} r{obj}[r{k}] = r{func}",
            if setter { "setter" } else { "getter" }
        ),
        Op::CopyProps { obj, src } => format!("copy props r{src} -> r{obj}"),
        Op::SetProto { obj, src } => format!("r{obj}.__proto__ = r{src}"),
        Op::ArrayPush { arr, src } => format!("push r{arr}, r{src}"),
        Op::ArraySpread { arr, src } => format!("push r{arr}, ...r{src}"),
        Op::CallSpread {
            dst,
            callee,
            this,
            arr,
        } => format!("r{dst} = call r{callee} this=r{this} args=...r{arr}"),
        Op::NewSpread { dst, callee, arr } => format!("r{dst} = new r{callee} args=...r{arr}"),
        Op::ForInKeys { dst, obj } => format!("r{dst} = for-in keys of r{obj}"),
        Op::IterPrepare { dst, src } => format!("r{dst} = iterable r{src}"),
        Op::MapArguments { src } => format!("map arguments r{src}"),
        Op::DirectEval { dst, args, argc } => format!("r{dst} = direct eval args=r{args}+{argc}"),
        Op::Throw { src } => format!("throw r{src}"),
    }
}

/// The whole block as text: a header, the ops, and the handler table.
/// Nested functions follow, each with its own header.
pub fn disassemble(block: &CodeBlock) -> String {
    let mut out = String::new();
    let mut stack: Vec<(&CodeBlock, String)> = vec![(block, String::new())];
    while let Some((b, path)) = stack.pop() {
        let mut flags = String::new();
        if !b.strict {
            flags.push_str(", sloppy");
        }
        if let Some(r) = b.arguments_reg {
            flags.push_str(&format!(", arguments r{r}"));
        }
        if let Some(r) = b.rest_reg {
            flags.push_str(&format!(", rest r{r}"));
        }
        let _ = writeln!(
            out,
            "function {}{} (params {}, registers {}, ic sites {}{flags})",
            b.display_name(),
            path,
            b.params,
            b.register_count,
            b.ic_sites
        );
        for (pc, op) in b.ops.iter().enumerate() {
            let _ = writeln!(out, "  {pc:04}  {}", op_text(b, pc, op));
        }
        for h in &b.handlers {
            let _ = writeln!(
                out,
                "  try {}..{} -> {} (catch r{}, env depth {})",
                h.start, h.end, h.target, h.catch_reg, h.env_depth
            );
        }
        for (i, f) in b.functions.iter().enumerate().rev() {
            stack.push((f, format!("{path} #{i}")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cinder::bytecode::Handler;

    #[test]
    fn listing_reads_plainly() {
        let mut b = CodeBlock::new("main", 0, 3);
        let x = b.add_key("x");
        let ic = b.add_ic();
        let half = b.add_number(0.5);
        b.emit(Op::NewObject { dst: 0 });
        b.emit(Op::LoadNumber { dst: 1, idx: half });
        b.emit(Op::SetProp {
            obj: 0,
            src: 1,
            key: x,
            ic,
        });
        b.emit(Op::Binary {
            op: BinOp::Add,
            dst: 2,
            a: 1,
            b: 1,
        });
        b.emit(Op::JumpIfFalse { cond: 2, offset: 1 });
        b.emit(Op::Throw { src: 2 });
        b.emit(Op::Return { src: 2 });
        b.handlers.push(Handler {
            start: 0,
            end: 5,
            target: 6,
            catch_reg: 2,
            env_depth: 0,
        });
        let mut inner = CodeBlock::new("", 1, 1);
        inner.emit(Op::Return { src: 0 });
        b.functions.push(inner);
        let text = disassemble(&b);
        let want = "\
function main (params 0, registers 3, ic sites 1)
  0000  r0 = {}
  0001  r1 = 0.5
  0002  r0.x = r1 [ic 0]
  0003  r2 = r1 + r1
  0004  unless r2 jump 6
  0005  throw r2
  0006  return r2
  try 0..5 -> 6 (catch r2, env depth 0)
function anonymous #0 (params 1, registers 1, ic sites 0)
  0000  return r0
";
        assert_eq!(text, want);
    }
}
