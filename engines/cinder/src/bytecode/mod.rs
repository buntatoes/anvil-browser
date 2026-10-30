// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Register bytecode.
//!
//! A function compiles to a [`CodeBlock`]: a list of [`Op`]s working on a
//! window of registers, plus the tables the ops point into (numbers,
//! strings, property keys, nested functions), an exception handler table,
//! source spans and the number of inline cache sites.
//!
//! `Op` is a plain enum. Registers are `u16`, table indices and cache sites
//! `u32`, and jumps are `i32` offsets counted from the op after the jump.
//! Each op is at most 16 bytes (a test checks).
//!
//! Before a block runs, the VM checks it ([`CodeBlock::validate`]): every
//! register, table index, jump target, handler range and cache site must be
//! in range, so a bad block is refused with a reason instead of misbehaving.

pub mod disasm;

use crate::cinder::error::Span;

/// A register index within the current frame.
pub type Reg = u16;

/// Two-operand operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `**`
    Exp,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `>>>`
    UShr,
    /// `===`
    StrictEq,
    /// `!==`
    StrictNe,
    /// `==`
    LooseEq,
    /// `!=`
    LooseNe,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `instanceof`
    InstanceOf,
    /// `in`
    In,
}

impl BinOp {
    /// Every operator, for tests and generators.
    pub const ALL: [BinOp; 22] = [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Mod,
        BinOp::Exp,
        BinOp::BitAnd,
        BinOp::BitOr,
        BinOp::BitXor,
        BinOp::Shl,
        BinOp::Shr,
        BinOp::UShr,
        BinOp::StrictEq,
        BinOp::StrictNe,
        BinOp::LooseEq,
        BinOp::LooseNe,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::InstanceOf,
        BinOp::In,
    ];

    /// The operator as written in source.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Exp => "**",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::UShr => ">>>",
            BinOp::StrictEq => "===",
            BinOp::StrictNe => "!==",
            BinOp::LooseEq => "==",
            BinOp::LooseNe => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::InstanceOf => "instanceof",
            BinOp::In => "in",
        }
    }
}

/// One-operand operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `+x`
    Plus,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `typeof x`
    TypeOf,
    /// `x + 1` after `ToNumber` (for `++`)
    Inc,
    /// `x - 1` after `ToNumber` (for `--`)
    Dec,
}

impl UnOp {
    /// Every operator, for tests and generators.
    pub const ALL: [UnOp; 7] = [
        UnOp::Neg,
        UnOp::Plus,
        UnOp::Not,
        UnOp::BitNot,
        UnOp::TypeOf,
        UnOp::Inc,
        UnOp::Dec,
    ];
}

/// One instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Do nothing.
    Nop,
    /// `dst = numbers[idx]`
    LoadNumber {
        /// Destination.
        dst: Reg,
        /// Index into the number table.
        idx: u32,
    },
    /// `dst = value`
    LoadInt {
        /// Destination.
        dst: Reg,
        /// The integer.
        value: i32,
    },
    /// `dst = strings[idx]`
    LoadString {
        /// Destination.
        dst: Reg,
        /// Index into the string table.
        idx: u32,
    },
    /// `dst = undefined`
    LoadUndefined {
        /// Destination.
        dst: Reg,
    },
    /// `dst = null`
    LoadNull {
        /// Destination.
        dst: Reg,
    },
    /// `dst = true`
    LoadTrue {
        /// Destination.
        dst: Reg,
    },
    /// `dst = false`
    LoadFalse {
        /// Destination.
        dst: Reg,
    },
    /// `dst = <uninitialized>`, the marker for a `let` not set yet.
    LoadUninit {
        /// Destination.
        dst: Reg,
    },
    /// `dst = this`
    LoadThis {
        /// Destination.
        dst: Reg,
    },
    /// `dst = src`
    Move {
        /// Destination.
        dst: Reg,
        /// Source.
        src: Reg,
    },
    /// `dst = a op b`
    Binary {
        /// The operator.
        op: BinOp,
        /// Destination.
        dst: Reg,
        /// Left operand.
        a: Reg,
        /// Right operand.
        b: Reg,
    },
    /// `dst = op src`
    Unary {
        /// The operator.
        op: UnOp,
        /// Destination.
        dst: Reg,
        /// Operand.
        src: Reg,
    },
    /// Jump by `offset` ops, counted from the next op.
    Jump {
        /// Relative target.
        offset: i32,
    },
    /// Jump when `cond` is truthy.
    JumpIfTrue {
        /// Condition.
        cond: Reg,
        /// Relative target.
        offset: i32,
    },
    /// Jump when `cond` is falsy.
    JumpIfFalse {
        /// Condition.
        cond: Reg,
        /// Relative target.
        offset: i32,
    },
    /// Jump when `cond` is `null` or `undefined`.
    JumpIfNullish {
        /// Condition.
        cond: Reg,
        /// Relative target.
        offset: i32,
    },
    /// Top of a loop: a safepoint, and a trip counter for later profiling.
    LoopHeader {
        /// Loop number within the function.
        id: u32,
    },
    /// Throw `ReferenceError` if `src` holds the uninitialized marker.
    CheckTdz {
        /// The binding's register.
        src: Reg,
        /// Name of the binding, in the key table.
        name: u32,
    },
    /// `dst = global[keys[key]]`, `ReferenceError` when missing.
    GetGlobal {
        /// Destination.
        dst: Reg,
        /// Name, in the key table.
        key: u32,
        /// Inline cache site.
        ic: u32,
    },
    /// `global[keys[key]] = src`
    SetGlobal {
        /// Value to store.
        src: Reg,
        /// Name, in the key table.
        key: u32,
        /// Inline cache site.
        ic: u32,
    },
    /// Push a new environment with `size` slots onto the frame.
    PushEnv {
        /// Number of slots.
        size: u32,
    },
    /// Pop the innermost environment.
    PopEnv,
    /// `dst = env(depth).slots[slot]`; depth 0 is the innermost.
    GetEnv {
        /// Destination.
        dst: Reg,
        /// How many environments up.
        depth: u16,
        /// Slot.
        slot: u32,
    },
    /// `env(depth).slots[slot] = src`
    SetEnv {
        /// Value to store.
        src: Reg,
        /// How many environments up.
        depth: u16,
        /// Slot.
        slot: u32,
    },
    /// `dst = {}`
    NewObject {
        /// Destination.
        dst: Reg,
    },
    /// `dst = []`
    NewArray {
        /// Destination.
        dst: Reg,
    },
    /// `dst = obj[keys[key]]`
    GetProp {
        /// Destination.
        dst: Reg,
        /// Receiver.
        obj: Reg,
        /// Name, in the key table.
        key: u32,
        /// Inline cache site.
        ic: u32,
    },
    /// `obj[keys[key]] = src`
    SetProp {
        /// Receiver.
        obj: Reg,
        /// Value to store.
        src: Reg,
        /// Name, in the key table.
        key: u32,
        /// Inline cache site.
        ic: u32,
    },
    /// `dst = obj[key]` with a computed key.
    GetElem {
        /// Destination.
        dst: Reg,
        /// Receiver.
        obj: Reg,
        /// Key register.
        key: Reg,
    },
    /// `obj[key] = src` with a computed key.
    SetElem {
        /// Receiver.
        obj: Reg,
        /// Key register.
        key: Reg,
        /// Value to store.
        src: Reg,
    },
    /// `dst = delete obj[key]`
    DeleteElem {
        /// Destination.
        dst: Reg,
        /// Receiver.
        obj: Reg,
        /// Key register.
        key: Reg,
    },
    /// `dst = function` made from `functions[func]`, closing over the
    /// current environment.
    Closure {
        /// Destination.
        dst: Reg,
        /// Index into the nested function table.
        func: u32,
    },
    /// `dst = callee.call(this, args[0..argc])`, arguments in consecutive
    /// registers starting at `args`.
    Call {
        /// Destination.
        dst: Reg,
        /// Function.
        callee: Reg,
        /// `this` value.
        this: Reg,
        /// First argument register.
        args: Reg,
        /// Number of arguments.
        argc: u16,
    },
    /// `dst = new callee(args[0..argc])`
    New {
        /// Destination.
        dst: Reg,
        /// Constructor.
        callee: Reg,
        /// First argument register.
        args: Reg,
        /// Number of arguments.
        argc: u16,
    },
    /// `dst = the global object`
    LoadGlobalObject {
        /// Destination.
        dst: Reg,
    },
    /// `dst = the function being run` (for a named function expression's
    /// own name).
    LoadCallee {
        /// Destination.
        dst: Reg,
    },
    /// `dst = typeof global[keys[key]]`, `"undefined"` when missing.
    TypeOfGlobal {
        /// Destination.
        dst: Reg,
        /// Name, in the key table.
        key: u32,
    },
    /// Declare a global `var` or function name (on the global object).
    DeclareVar {
        /// Name, in the key table.
        key: u32,
    },
    /// Declare a global `let`, `const` or `class` name, uninitialized.
    DeclareLex {
        /// Name, in the key table.
        key: u32,
        /// True for `const`.
        konst: bool,
    },
    /// Initialize a global lexical binding.
    InitLex {
        /// Value.
        src: Reg,
        /// Name, in the key table.
        key: u32,
    },
    /// Throw a new error object of `kind` (an `ErrorKind` number) with the
    /// message `strings[msg]`.
    ThrowError {
        /// 0 Error, 1 TypeError, 2 RangeError, 3 ReferenceError, 4 SyntaxError.
        kind: u8,
        /// Message, in the string table.
        msg: u32,
    },
    /// Define a data property, as an object literal does (no setters run).
    DefineField {
        /// Object.
        obj: Reg,
        /// Key register.
        key: Reg,
        /// Value.
        src: Reg,
    },
    /// Define a getter or setter, as an object literal does.
    DefineAccessor {
        /// Object.
        obj: Reg,
        /// Key register.
        key: Reg,
        /// The function.
        func: Reg,
        /// True for a setter.
        setter: bool,
    },
    /// Copy own enumerable data properties of `src` onto `obj` (`{...src}`).
    CopyProps {
        /// Target.
        obj: Reg,
        /// Source.
        src: Reg,
    },
    /// `__proto__: src` in an object literal: set the prototype of `obj`
    /// when `src` is an object or `null`, else do nothing.
    SetProto {
        /// Object.
        obj: Reg,
        /// New prototype.
        src: Reg,
    },
    /// Append `src` to the array `arr`.
    ArrayPush {
        /// Array.
        arr: Reg,
        /// Value.
        src: Reg,
    },
    /// Append every element of `src` (an array, string or arguments
    /// object) to `arr`.
    ArraySpread {
        /// Array.
        arr: Reg,
        /// Values.
        src: Reg,
    },
    /// `dst = callee.call(this, ...arr)`
    CallSpread {
        /// Destination.
        dst: Reg,
        /// Function.
        callee: Reg,
        /// `this`.
        this: Reg,
        /// Array of arguments.
        arr: Reg,
    },
    /// `dst = new callee(...arr)`
    NewSpread {
        /// Destination.
        dst: Reg,
        /// Constructor.
        callee: Reg,
        /// Array of arguments.
        arr: Reg,
    },
    /// `dst = the keys a for-in loop over obj visits`, as an array.
    ForInKeys {
        /// Destination.
        dst: Reg,
        /// Object.
        obj: Reg,
    },
    /// `dst = something a for-of loop can walk by index`: arrays and
    /// arguments objects as they are, strings as an array of code points.
    /// Anything else throws a `TypeError`.
    IterPrepare {
        /// Destination.
        dst: Reg,
        /// Iterable.
        src: Reg,
    },
    /// Tie the arguments object in `src` to the parameters, which sit in
    /// slots `0..params` of the innermost environment (sloppy functions
    /// with simple parameters).
    MapArguments {
        /// The arguments object.
        src: Reg,
    },
    /// A call written `eval(...)`. When the global `eval` is the built-in
    /// one, the code runs as direct eval at global scope with the caller's
    /// strictness; otherwise it's an ordinary call.
    DirectEval {
        /// Destination.
        dst: Reg,
        /// First argument register.
        args: Reg,
        /// Number of arguments.
        argc: u16,
    },
    /// Return `src` to the caller.
    Return {
        /// Value to return.
        src: Reg,
    },
    /// Throw `src`.
    Throw {
        /// Value to throw.
        src: Reg,
    },
}

/// One entry of the exception handler table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handler {
    /// First op covered.
    pub start: u32,
    /// One past the last op covered.
    pub end: u32,
    /// Where to go when something is thrown inside.
    pub target: u32,
    /// Register that gets the thrown value.
    pub catch_reg: Reg,
    /// How many environments the frame had pushed when the `try` began.
    /// Unwinding pops back to this many.
    pub env_depth: u16,
}

/// A compiled function.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeBlock {
    /// Name for stacks and messages (empty for anonymous functions).
    pub name: String,
    /// Number of declared parameters. They arrive in registers `0..params`.
    pub params: u16,
    /// Registers the function uses.
    pub register_count: u32,
    /// The instructions.
    pub ops: Vec<Op>,
    /// Number table.
    pub numbers: Vec<f64>,
    /// String table, as UTF-16.
    pub strings: Vec<Vec<u16>>,
    /// Property key table, as UTF-16 (turned into keys when loaded).
    pub keys: Vec<Vec<u16>>,
    /// Nested functions.
    pub functions: Vec<CodeBlock>,
    /// Exception handlers, innermost first for nested ranges.
    pub handlers: Vec<Handler>,
    /// `(op index, span)`, sorted by op index. An op takes the span of the
    /// last entry at or before it.
    pub spans: Vec<(u32, Span)>,
    /// Number of inline cache sites.
    pub ic_sites: u32,
    /// Number of loops (for `LoopHeader` ids).
    pub loops: u32,
    /// Where the function itself starts in the source.
    pub span: Span,
    /// Strict mode code (the default from [`CodeBlock::new`]). Sloppy code
    /// coerces `this`, creates globals on assignment, and fails quietly
    /// where strict code throws.
    pub strict: bool,
    /// Register that gets the `arguments` object when the frame starts.
    pub arguments_reg: Option<Reg>,
    /// Register that gets an array of the arguments past `params` (a rest
    /// parameter).
    pub rest_reg: Option<Reg>,
}

impl CodeBlock {
    /// A new empty block.
    pub fn new(name: &str, params: u16, register_count: u32) -> CodeBlock {
        CodeBlock {
            name: name.to_string(),
            params,
            register_count,
            strict: true,
            ..CodeBlock::default()
        }
    }

    /// Add a number to the table and return its index.
    pub fn add_number(&mut self, n: f64) -> u32 {
        if let Some(i) = self.numbers.iter().position(|x| x.to_bits() == n.to_bits()) {
            return i as u32;
        }
        self.numbers.push(n);
        (self.numbers.len() - 1) as u32
    }

    /// Add a string to the table and return its index.
    pub fn add_string(&mut self, s: &str) -> u32 {
        let units: Vec<u16> = s.encode_utf16().collect();
        if let Some(i) = self.strings.iter().position(|x| *x == units) {
            return i as u32;
        }
        self.strings.push(units);
        (self.strings.len() - 1) as u32
    }

    /// Add a property key to the table and return its index.
    pub fn add_key(&mut self, s: &str) -> u32 {
        let units: Vec<u16> = s.encode_utf16().collect();
        if let Some(i) = self.keys.iter().position(|x| *x == units) {
            return i as u32;
        }
        self.keys.push(units);
        (self.keys.len() - 1) as u32
    }

    /// Hand out a new inline cache site.
    pub fn add_ic(&mut self) -> u32 {
        self.ic_sites += 1;
        self.ic_sites - 1
    }

    /// Append an op and return its index.
    pub fn emit(&mut self, op: Op) -> u32 {
        self.ops.push(op);
        (self.ops.len() - 1) as u32
    }

    /// The span for op `pc`, if any.
    pub fn span_at(&self, pc: u32) -> Option<Span> {
        let i = self.spans.partition_point(|(p, _)| *p <= pc);
        if i == 0 {
            None
        } else {
            Some(self.spans[i - 1].1)
        }
    }

    /// Name for messages: the function name, or `anonymous`.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "anonymous"
        } else {
            &self.name
        }
    }

    /// Check that every index in the block is in range. This doesn't look
    /// into nested functions; the VM checks each one as it loads it.
    pub fn validate(&self) -> Result<(), String> {
        let regs = self.register_count;
        let len = self.ops.len();
        let who = self.display_name();
        if u32::from(self.params) > regs {
            return Err(format!(
                "function '{who}' has {} parameters but only {regs} registers",
                self.params
            ));
        }
        if len == 0 {
            return Err(format!("function '{who}' has no ops"));
        }
        if !matches!(
            self.ops[len - 1],
            Op::Return { .. } | Op::Throw { .. } | Op::Jump { .. }
        ) {
            return Err(format!(
                "function '{who}' doesn't end with return, throw or jump, so it could run off the end"
            ));
        }
        for (pc, op) in self.ops.iter().enumerate() {
            let bad = |what: String| Err(format!("function '{who}', op {pc}: {what}"));
            let check_reg = |r: Reg| -> Result<(), String> {
                if u32::from(r) < regs {
                    Ok(())
                } else {
                    Err(format!(
                        "function '{who}', op {pc}: register {r} is past the function's {regs} registers"
                    ))
                }
            };
            let check_range = |first: Reg, count: u16| -> Result<(), String> {
                if u32::from(first) + u32::from(count) <= regs {
                    Ok(())
                } else {
                    Err(format!(
                        "function '{who}', op {pc}: arguments r{first}..r{} are past the function's {regs} registers",
                        u32::from(first) + u32::from(count)
                    ))
                }
            };
            let check_jump = |offset: i32| -> Result<(), String> {
                let target = pc as i64 + 1 + i64::from(offset);
                if target >= 0 && (target as usize) < len {
                    Ok(())
                } else {
                    Err(format!(
                        "function '{who}', op {pc}: jump to {target} is outside 0..{len}"
                    ))
                }
            };
            let check_ic = |ic: u32| -> Result<(), String> {
                if ic < self.ic_sites {
                    Ok(())
                } else {
                    Err(format!(
                        "function '{who}', op {pc}: cache site {ic} is past the {} sites",
                        self.ic_sites
                    ))
                }
            };
            let check_key = |k: u32| -> Result<(), String> {
                if (k as usize) < self.keys.len() {
                    Ok(())
                } else {
                    Err(format!(
                        "function '{who}', op {pc}: key {k} is past the {} keys",
                        self.keys.len()
                    ))
                }
            };
            match *op {
                Op::Nop | Op::PopEnv | Op::PushEnv { .. } => {}
                Op::LoopHeader { id } => {
                    if id >= self.loops {
                        return bad(format!("loop {id} is past the {} loops", self.loops));
                    }
                }
                Op::LoadNumber { dst, idx } => {
                    check_reg(dst)?;
                    if idx as usize >= self.numbers.len() {
                        return bad(format!(
                            "number {idx} is past the {} numbers",
                            self.numbers.len()
                        ));
                    }
                }
                Op::LoadString { dst, idx } => {
                    check_reg(dst)?;
                    if idx as usize >= self.strings.len() {
                        return bad(format!(
                            "string {idx} is past the {} strings",
                            self.strings.len()
                        ));
                    }
                }
                Op::LoadInt { dst, .. }
                | Op::LoadUndefined { dst }
                | Op::LoadNull { dst }
                | Op::LoadTrue { dst }
                | Op::LoadFalse { dst }
                | Op::LoadUninit { dst }
                | Op::LoadThis { dst }
                | Op::NewObject { dst }
                | Op::NewArray { dst } => check_reg(dst)?,
                Op::Move { dst, src } | Op::Unary { dst, src, .. } => {
                    check_reg(dst)?;
                    check_reg(src)?;
                }
                Op::Binary { dst, a, b, .. } => {
                    check_reg(dst)?;
                    check_reg(a)?;
                    check_reg(b)?;
                }
                Op::Jump { offset } => check_jump(offset)?,
                Op::JumpIfTrue { cond, offset }
                | Op::JumpIfFalse { cond, offset }
                | Op::JumpIfNullish { cond, offset } => {
                    check_reg(cond)?;
                    check_jump(offset)?;
                }
                Op::CheckTdz { src, name } => {
                    check_reg(src)?;
                    check_key(name)?;
                }
                Op::GetGlobal { dst: r, key, ic } | Op::SetGlobal { src: r, key, ic } => {
                    check_reg(r)?;
                    check_key(key)?;
                    check_ic(ic)?;
                }
                Op::GetEnv { dst: r, .. } | Op::SetEnv { src: r, .. } => check_reg(r)?,
                Op::GetProp { dst, obj, key, ic } => {
                    check_reg(dst)?;
                    check_reg(obj)?;
                    check_key(key)?;
                    check_ic(ic)?;
                }
                Op::SetProp { obj, src, key, ic } => {
                    check_reg(obj)?;
                    check_reg(src)?;
                    check_key(key)?;
                    check_ic(ic)?;
                }
                Op::GetElem { dst, obj, key } | Op::DeleteElem { dst, obj, key } => {
                    check_reg(dst)?;
                    check_reg(obj)?;
                    check_reg(key)?;
                }
                Op::SetElem { obj, key, src } => {
                    check_reg(obj)?;
                    check_reg(key)?;
                    check_reg(src)?;
                }
                Op::Closure { dst, func } => {
                    check_reg(dst)?;
                    if func as usize >= self.functions.len() {
                        return bad(format!(
                            "function {func} is past the {} nested functions",
                            self.functions.len()
                        ));
                    }
                }
                Op::Call {
                    dst,
                    callee,
                    this,
                    args,
                    argc,
                } => {
                    check_reg(dst)?;
                    check_reg(callee)?;
                    check_reg(this)?;
                    check_range(args, argc)?;
                }
                Op::New {
                    dst,
                    callee,
                    args,
                    argc,
                } => {
                    check_reg(dst)?;
                    check_reg(callee)?;
                    check_range(args, argc)?;
                }
                Op::Return { src } | Op::Throw { src } => check_reg(src)?,
                Op::LoadGlobalObject { dst } | Op::LoadCallee { dst } => check_reg(dst)?,
                Op::TypeOfGlobal { dst, key } => {
                    check_reg(dst)?;
                    check_key(key)?;
                }
                Op::DeclareVar { key } | Op::DeclareLex { key, .. } => check_key(key)?,
                Op::InitLex { src, key } => {
                    check_reg(src)?;
                    check_key(key)?;
                }
                Op::ThrowError { kind, msg } => {
                    if kind > 4 {
                        return bad(format!("error kind {kind} is not 0..4"));
                    }
                    if msg as usize >= self.strings.len() {
                        return bad(format!(
                            "string {msg} is past the {} strings",
                            self.strings.len()
                        ));
                    }
                }
                Op::DefineField { obj, key, src } => {
                    check_reg(obj)?;
                    check_reg(key)?;
                    check_reg(src)?;
                }
                Op::DefineAccessor { obj, key, func, .. } => {
                    check_reg(obj)?;
                    check_reg(key)?;
                    check_reg(func)?;
                }
                Op::CopyProps { obj: a, src: b }
                | Op::SetProto { obj: a, src: b }
                | Op::ArrayPush { arr: a, src: b }
                | Op::ArraySpread { arr: a, src: b }
                | Op::ForInKeys { dst: a, obj: b }
                | Op::IterPrepare { dst: a, src: b } => {
                    check_reg(a)?;
                    check_reg(b)?;
                }
                Op::CallSpread {
                    dst,
                    callee,
                    this,
                    arr,
                } => {
                    check_reg(dst)?;
                    check_reg(callee)?;
                    check_reg(this)?;
                    check_reg(arr)?;
                }
                Op::NewSpread { dst, callee, arr } => {
                    check_reg(dst)?;
                    check_reg(callee)?;
                    check_reg(arr)?;
                }
                Op::MapArguments { src } => check_reg(src)?,
                Op::DirectEval { dst, args, argc } => {
                    check_reg(dst)?;
                    check_range(args, argc)?;
                }
            }
        }
        for r in [self.arguments_reg, self.rest_reg].into_iter().flatten() {
            if u32::from(r) >= regs {
                return Err(format!(
                    "function '{who}': register {r} is past the function's {regs} registers"
                ));
            }
        }
        for (i, h) in self.handlers.iter().enumerate() {
            if h.start > h.end || h.end as usize > len || h.target as usize >= len {
                return Err(format!(
                    "function '{who}', handler {i}: range {}..{} or target {} is outside 0..{len}",
                    h.start, h.end, h.target
                ));
            }
            if u32::from(h.catch_reg) >= regs {
                return Err(format!(
                    "function '{who}', handler {i}: catch register {} is past the {regs} registers",
                    h.catch_reg
                ));
            }
        }
        if self.spans.windows(2).any(|w| w[0].0 > w[1].0) {
            return Err(format!("function '{who}': spans are not sorted by op"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_at_most_16_bytes() {
        assert!(
            std::mem::size_of::<Op>() <= 16,
            "Op is {} bytes",
            std::mem::size_of::<Op>()
        );
    }

    #[test]
    fn tables_dedupe() {
        let mut b = CodeBlock::new("f", 0, 1);
        assert_eq!(b.add_number(1.5), 0);
        assert_eq!(b.add_number(2.0), 1);
        assert_eq!(b.add_number(1.5), 0);
        assert_eq!(b.add_number(-0.0), 2, "-0 is not 0");
        assert_eq!(b.add_string("a"), 0);
        assert_eq!(b.add_string("a"), 0);
        assert_eq!(b.add_key("x"), 0);
        assert_eq!(b.add_key("y"), 1);
        assert_eq!(b.add_ic(), 0);
        assert_eq!(b.add_ic(), 1);
    }

    #[test]
    fn validate_catches_bad_indices() {
        let mut b = CodeBlock::new("f", 0, 2);
        b.emit(Op::LoadInt { dst: 1, value: 3 });
        b.emit(Op::Return { src: 1 });
        assert_eq!(b.validate(), Ok(()));

        let mut bad = b.clone();
        bad.ops[0] = Op::LoadInt { dst: 2, value: 3 };
        assert_eq!(
            bad.validate().unwrap_err(),
            "function 'f', op 0: register 2 is past the function's 2 registers"
        );

        let mut bad = b.clone();
        bad.ops.insert(0, Op::Jump { offset: 5 });
        assert!(
            bad.validate()
                .unwrap_err()
                .contains("jump to 6 is outside 0..3")
        );

        let mut bad = b.clone();
        bad.ops.insert(
            0,
            Op::GetProp {
                dst: 0,
                obj: 0,
                key: 0,
                ic: 0,
            },
        );
        assert!(
            bad.validate()
                .unwrap_err()
                .contains("key 0 is past the 0 keys")
        );

        let mut bad = b.clone();
        bad.ops.pop();
        assert!(bad.validate().unwrap_err().contains("run off the end"));

        let mut bad = b.clone();
        bad.handlers.push(Handler {
            start: 0,
            end: 9,
            target: 1,
            catch_reg: 0,
            env_depth: 0,
        });
        assert!(bad.validate().unwrap_err().contains("handler 0"));

        let mut bad = b.clone();
        bad.ops.insert(
            0,
            Op::Call {
                dst: 0,
                callee: 0,
                this: 0,
                args: 1,
                argc: 2,
            },
        );
        assert!(bad.validate().unwrap_err().contains("arguments r1..r3"));
    }

    #[test]
    fn spans_resolve_to_the_last_entry() {
        let mut b = CodeBlock::new("", 0, 1);
        let s1 = Span {
            start: 0,
            end: 1,
            line: 1,
            column: 1,
        };
        let s2 = Span {
            start: 5,
            end: 6,
            line: 2,
            column: 3,
        };
        b.spans = vec![(0, s1), (3, s2)];
        assert_eq!(b.span_at(0), Some(s1));
        assert_eq!(b.span_at(2), Some(s1));
        assert_eq!(b.span_at(3), Some(s2));
        assert_eq!(b.span_at(99), Some(s2));
        assert_eq!(b.display_name(), "anonymous");
    }
}
