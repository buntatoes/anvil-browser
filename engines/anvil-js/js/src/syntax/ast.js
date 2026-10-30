// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The syntax tree. The node set follows the reference's ast.rs: the same
// kinds, with the same fields in the same order, so the canonical dump
// (ast_dump.js) prints what `refdump --ast` prints.
//
// A node is a plain record { kind, start, end, line, col, ...fields }
// built by one constructor per kind. `kind` is a number (the constants
// below; ast_dump.js has their names). `start` and `end` are code-unit
// offsets, `line` and `col` the 1-based place of `start`. Names, strings,
// template pieces and regex parts are arrays of UTF-16 code units; a
// missing optional child is null, and so is an array hole. Nodes the
// reference keeps without a span (ForInit, ForHead, ArrowBody, PropKey
// variants other than literals, MemberProp, Template, PatProp, ImportDecl,
// ImportAttribute, ModuleName, ImportSpec, ExportDecl, ExportSpec,
// ExportDefault) still get one, which the dump leaves out.

export const N_PROGRAM = 0;
export const N_IDENT = 1;
export const S_EXPR = 2;
export const S_VAR = 3;
export const S_FUNCTION = 4;
export const S_CLASS = 5;
export const S_BLOCK = 6;
export const S_EMPTY = 7;
export const S_IF = 8;
export const S_FOR = 9;
export const S_FOR_IN = 10;
export const S_FOR_OF = 11;
export const S_WHILE = 12;
export const S_DO_WHILE = 13;
export const S_CONTINUE = 14;
export const S_BREAK = 15;
export const S_RETURN = 16;
export const S_WITH = 17;
export const S_SWITCH = 18;
export const S_LABELED = 19;
export const S_THROW = 20;
export const S_TRY = 21;
export const S_DEBUGGER = 22;
export const S_IMPORT = 23;
export const S_EXPORT = 24;
export const N_VAR_DECL = 25;
export const N_VAR_DECLARATOR = 26;
export const FI_VAR = 27;
export const FI_EXPR = 28;
export const FH_VAR = 29;
export const FH_PAT = 30;
export const N_SWITCH_CASE = 31;
export const N_CATCH = 32;
export const N_FUNCTION = 33;
export const N_ARROW = 34;
export const AB_EXPR = 35;
export const AB_BLOCK = 36;
export const N_CLASS = 37;
export const CM_METHOD = 38;
export const CM_FIELD = 39;
export const CM_STATIC_BLOCK = 40;
export const PK_IDENT = 41;
export const PK_STR = 42;
export const PK_NUM = 43;
export const PK_BIGINT = 44;
export const PK_COMPUTED = 45;
export const PK_PRIVATE = 46;
export const E_IDENT = 47;
export const E_THIS = 48;
export const E_SUPER = 49;
export const E_NULL = 50;
export const E_BOOL = 51;
export const E_NUM = 52;
export const E_BIGINT = 53;
export const E_STR = 54;
export const E_REGEX = 55;
export const E_TEMPLATE = 56;
export const E_TAGGED = 57;
export const E_ARRAY = 58;
export const E_OBJECT = 59;
export const E_FUNCTION = 60;
export const E_ARROW = 61;
export const E_CLASS = 62;
export const E_UNARY = 63;
export const E_UPDATE = 64;
export const E_BINARY = 65;
export const E_LOGICAL = 66;
export const E_ASSIGN = 67;
export const E_COND = 68;
export const E_CALL = 69;
export const E_NEW = 70;
export const E_MEMBER = 71;
export const E_OPT_CHAIN = 72;
export const E_SEQ = 73;
export const E_SPREAD = 74;
export const E_YIELD = 75;
export const E_AWAIT = 76;
export const E_NEW_TARGET = 77;
export const E_IMPORT_META = 78;
export const E_IMPORT = 79;
export const E_PAREN = 80;
export const E_PRIVATE_IN = 81;
export const MP_NAME = 82;
export const MP_COMPUTED = 83;
export const MP_PRIVATE = 84;
export const N_TEMPLATE = 85;
export const N_TEMPLATE_ELEMENT = 86;
export const PR_INIT = 87;
export const PR_SHORTHAND = 88;
export const PR_METHOD = 89;
export const PR_SPREAD = 90;
export const PT_IDENT = 91;
export const PT_ARRAY = 92;
export const PT_OBJECT = 93;
export const PT_ASSIGN = 94;
export const PT_REST = 95;
export const PT_EXPR = 96;
export const PP_PROP = 97;
export const PP_REST = 98;
export const N_IMPORT_DECL = 99;
export const N_STR_LIT = 100;
export const N_IMPORT_ATTRIBUTE = 101;
export const MN_IDENT = 102;
export const MN_STR = 103;
export const IS_DEFAULT = 104;
export const IS_NAMESPACE = 105;
export const IS_NAMED = 106;
export const ED_NAMED = 107;
export const ED_ALL = 108;
export const ED_DECL = 109;
export const ED_DEFAULT = 110;
export const N_EXPORT_SPEC = 111;
export const EX_FUNCTION = 112;
export const EX_CLASS = 113;
export const EX_EXPR = 114;
export const NODE_KIND_COUNT = 115;

// Prefix operators (UnaryOp), in the reference's order.
export const U_MINUS = 0;
export const U_PLUS = 1;
export const U_NOT = 2;
export const U_BIT_NOT = 3;
export const U_TYPEOF = 4;
export const U_VOID = 5;
export const U_DELETE = 6;

// `++` and `--` (UpdateOp), in the reference's order.
export const UP_INC = 0;
export const UP_DEC = 1;

// Binary operators (BinaryOp), in the reference's order.
export const B_EQ = 0;
export const B_NOT_EQ = 1;
export const B_STRICT_EQ = 2;
export const B_STRICT_NOT_EQ = 3;
export const B_LT = 4;
export const B_LT_EQ = 5;
export const B_GT = 6;
export const B_GT_EQ = 7;
export const B_SHL = 8;
export const B_SHR = 9;
export const B_USHR = 10;
export const B_ADD = 11;
export const B_SUB = 12;
export const B_MUL = 13;
export const B_DIV = 14;
export const B_MOD = 15;
export const B_EXP = 16;
export const B_BIT_OR = 17;
export const B_BIT_XOR = 18;
export const B_BIT_AND = 19;
export const B_IN = 20;
export const B_INSTANCEOF = 21;

// Short-circuit operators (LogicalOp), in the reference's order.
export const L_AND = 0;
export const L_OR = 1;
export const L_NULLISH = 2;

// Assignment operators (AssignOp), in the reference's order.
export const A_ASSIGN = 0;
export const A_ADD = 1;
export const A_SUB = 2;
export const A_MUL = 3;
export const A_DIV = 4;
export const A_MOD = 5;
export const A_EXP = 6;
export const A_SHL = 7;
export const A_SHR = 8;
export const A_USHR = 9;
export const A_BIT_AND = 10;
export const A_BIT_OR = 11;
export const A_BIT_XOR = 12;
export const A_AND = 13;
export const A_OR = 14;
export const A_NULLISH = 15;

// Method flavours (MethodKind), in the reference's order.
export const MK_METHOD = 0;
export const MK_GET = 1;
export const MK_SET = 2;
export const MK_CONSTRUCTOR = 3;

// Declaration keywords (VarKind), in the reference's order.
export const VK_VAR = 0;
export const VK_LET = 1;
export const VK_CONST = 2;

export function makeProgram(module, strict, body, start, end, line, col) {
  return {
    kind: N_PROGRAM,
    start: start,
    end: end,
    line: line,
    col: col,
    module: module,
    strict: strict,
    body: body,
  };
}

export function makeIdent(name, start, end, line, col) {
  return {
    kind: N_IDENT,
    start: start,
    end: end,
    line: line,
    col: col,
    name: name,
  };
}

export function makeStmtExpr(expr, directive, start, end, line, col) {
  return {
    kind: S_EXPR,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
    directive: directive,
  };
}

export function makeStmtVar(decl, start, end, line, col) {
  return {
    kind: S_VAR,
    start: start,
    end: end,
    line: line,
    col: col,
    decl: decl,
  };
}

export function makeStmtFunction(func, start, end, line, col) {
  return {
    kind: S_FUNCTION,
    start: start,
    end: end,
    line: line,
    col: col,
    func: func,
  };
}

export function makeStmtClass(cls, start, end, line, col) {
  return {
    kind: S_CLASS,
    start: start,
    end: end,
    line: line,
    col: col,
    cls: cls,
  };
}

export function makeStmtBlock(body, start, end, line, col) {
  return {
    kind: S_BLOCK,
    start: start,
    end: end,
    line: line,
    col: col,
    body: body,
  };
}

export function makeStmtEmpty(start, end, line, col) {
  return {
    kind: S_EMPTY,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeStmtIf(test, cons, alt, start, end, line, col) {
  return {
    kind: S_IF,
    start: start,
    end: end,
    line: line,
    col: col,
    test: test,
    cons: cons,
    alt: alt,
  };
}

export function makeStmtFor(init, test, update, body, start, end, line, col) {
  return {
    kind: S_FOR,
    start: start,
    end: end,
    line: line,
    col: col,
    init: init,
    test: test,
    update: update,
    body: body,
  };
}

export function makeStmtForIn(left, right, body, start, end, line, col) {
  return {
    kind: S_FOR_IN,
    start: start,
    end: end,
    line: line,
    col: col,
    left: left,
    right: right,
    body: body,
  };
}

export function makeStmtForOf(left, right, body, isAwait, start, end, line, col) {
  return {
    kind: S_FOR_OF,
    start: start,
    end: end,
    line: line,
    col: col,
    left: left,
    right: right,
    body: body,
    isAwait: isAwait,
  };
}

export function makeStmtWhile(test, body, start, end, line, col) {
  return {
    kind: S_WHILE,
    start: start,
    end: end,
    line: line,
    col: col,
    test: test,
    body: body,
  };
}

export function makeStmtDoWhile(body, test, start, end, line, col) {
  return {
    kind: S_DO_WHILE,
    start: start,
    end: end,
    line: line,
    col: col,
    body: body,
    test: test,
  };
}

export function makeStmtContinue(label, start, end, line, col) {
  return {
    kind: S_CONTINUE,
    start: start,
    end: end,
    line: line,
    col: col,
    label: label,
  };
}

export function makeStmtBreak(label, start, end, line, col) {
  return {
    kind: S_BREAK,
    start: start,
    end: end,
    line: line,
    col: col,
    label: label,
  };
}

export function makeStmtReturn(arg, start, end, line, col) {
  return {
    kind: S_RETURN,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makeStmtWith(object, body, start, end, line, col) {
  return {
    kind: S_WITH,
    start: start,
    end: end,
    line: line,
    col: col,
    object: object,
    body: body,
  };
}

export function makeStmtSwitch(disc, cases, start, end, line, col) {
  return {
    kind: S_SWITCH,
    start: start,
    end: end,
    line: line,
    col: col,
    disc: disc,
    cases: cases,
  };
}

export function makeStmtLabeled(label, body, start, end, line, col) {
  return {
    kind: S_LABELED,
    start: start,
    end: end,
    line: line,
    col: col,
    label: label,
    body: body,
  };
}

export function makeStmtThrow(arg, start, end, line, col) {
  return {
    kind: S_THROW,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makeStmtTry(block, handler, finalizer, start, end, line, col) {
  return {
    kind: S_TRY,
    start: start,
    end: end,
    line: line,
    col: col,
    block: block,
    handler: handler,
    finalizer: finalizer,
  };
}

export function makeStmtDebugger(start, end, line, col) {
  return {
    kind: S_DEBUGGER,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeStmtImport(decl, start, end, line, col) {
  return {
    kind: S_IMPORT,
    start: start,
    end: end,
    line: line,
    col: col,
    decl: decl,
  };
}

export function makeStmtExport(decl, start, end, line, col) {
  return {
    kind: S_EXPORT,
    start: start,
    end: end,
    line: line,
    col: col,
    decl: decl,
  };
}

export function makeVarDecl(varKind, decls, start, end, line, col) {
  return {
    kind: N_VAR_DECL,
    start: start,
    end: end,
    line: line,
    col: col,
    varKind: varKind,
    decls: decls,
  };
}

export function makeVarDeclarator(target, init, start, end, line, col) {
  return {
    kind: N_VAR_DECLARATOR,
    start: start,
    end: end,
    line: line,
    col: col,
    target: target,
    init: init,
  };
}

export function makeForInitVar(decl, start, end, line, col) {
  return {
    kind: FI_VAR,
    start: start,
    end: end,
    line: line,
    col: col,
    decl: decl,
  };
}

export function makeForInitExpr(expr, start, end, line, col) {
  return {
    kind: FI_EXPR,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makeForHeadVar(decl, start, end, line, col) {
  return {
    kind: FH_VAR,
    start: start,
    end: end,
    line: line,
    col: col,
    decl: decl,
  };
}

export function makeForHeadPat(pat, start, end, line, col) {
  return {
    kind: FH_PAT,
    start: start,
    end: end,
    line: line,
    col: col,
    pat: pat,
  };
}

export function makeSwitchCase(test, body, start, end, line, col) {
  return {
    kind: N_SWITCH_CASE,
    start: start,
    end: end,
    line: line,
    col: col,
    test: test,
    body: body,
  };
}

export function makeCatch(param, body, start, end, line, col) {
  return {
    kind: N_CATCH,
    start: start,
    end: end,
    line: line,
    col: col,
    param: param,
    body: body,
  };
}

export function makeFunction(id, params, body, isAsync, isGenerator, strict, simpleParams, start, end, line, col) {
  return {
    kind: N_FUNCTION,
    start: start,
    end: end,
    line: line,
    col: col,
    id: id,
    params: params,
    body: body,
    isAsync: isAsync,
    isGenerator: isGenerator,
    strict: strict,
    simpleParams: simpleParams,
  };
}

export function makeArrow(params, body, isAsync, strict, start, end, line, col) {
  return {
    kind: N_ARROW,
    start: start,
    end: end,
    line: line,
    col: col,
    params: params,
    body: body,
    isAsync: isAsync,
    strict: strict,
  };
}

export function makeArrowBodyExpr(expr, start, end, line, col) {
  return {
    kind: AB_EXPR,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makeArrowBodyBlock(body, start, end, line, col) {
  return {
    kind: AB_BLOCK,
    start: start,
    end: end,
    line: line,
    col: col,
    body: body,
  };
}

export function makeClass(id, superClass, body, start, end, line, col) {
  return {
    kind: N_CLASS,
    start: start,
    end: end,
    line: line,
    col: col,
    id: id,
    superClass: superClass,
    body: body,
  };
}

export function makeMemberMethod(key, methodKind, isStatic, value, start, end, line, col) {
  return {
    kind: CM_METHOD,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    methodKind: methodKind,
    isStatic: isStatic,
    value: value,
  };
}

export function makeMemberField(key, isStatic, value, start, end, line, col) {
  return {
    kind: CM_FIELD,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    isStatic: isStatic,
    value: value,
  };
}

export function makeMemberStaticBlock(body, start, end, line, col) {
  return {
    kind: CM_STATIC_BLOCK,
    start: start,
    end: end,
    line: line,
    col: col,
    body: body,
  };
}

export function makeKeyIdent(ident, start, end, line, col) {
  return {
    kind: PK_IDENT,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeKeyStr(value, start, end, line, col) {
  return {
    kind: PK_STR,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeKeyNum(value, start, end, line, col) {
  return {
    kind: PK_NUM,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeKeyBigint(radix, digits, start, end, line, col) {
  return {
    kind: PK_BIGINT,
    start: start,
    end: end,
    line: line,
    col: col,
    radix: radix,
    digits: digits,
  };
}

export function makeKeyComputed(expr, start, end, line, col) {
  return {
    kind: PK_COMPUTED,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makeKeyPrivate(ident, start, end, line, col) {
  return {
    kind: PK_PRIVATE,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeExprIdent(name, start, end, line, col) {
  return {
    kind: E_IDENT,
    start: start,
    end: end,
    line: line,
    col: col,
    name: name,
  };
}

export function makeExprThis(start, end, line, col) {
  return {
    kind: E_THIS,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeExprSuper(start, end, line, col) {
  return {
    kind: E_SUPER,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeExprNull(start, end, line, col) {
  return {
    kind: E_NULL,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeExprBool(value, start, end, line, col) {
  return {
    kind: E_BOOL,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeExprNum(value, start, end, line, col) {
  return {
    kind: E_NUM,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeExprBigint(radix, digits, start, end, line, col) {
  return {
    kind: E_BIGINT,
    start: start,
    end: end,
    line: line,
    col: col,
    radix: radix,
    digits: digits,
  };
}

export function makeExprStr(value, start, end, line, col) {
  return {
    kind: E_STR,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeExprRegex(pattern, flags, start, end, line, col) {
  return {
    kind: E_REGEX,
    start: start,
    end: end,
    line: line,
    col: col,
    pattern: pattern,
    flags: flags,
  };
}

export function makeExprTemplate(quasi, start, end, line, col) {
  return {
    kind: E_TEMPLATE,
    start: start,
    end: end,
    line: line,
    col: col,
    quasi: quasi,
  };
}

export function makeExprTagged(tag, quasi, start, end, line, col) {
  return {
    kind: E_TAGGED,
    start: start,
    end: end,
    line: line,
    col: col,
    tag: tag,
    quasi: quasi,
  };
}

export function makeExprArray(items, start, end, line, col) {
  return {
    kind: E_ARRAY,
    start: start,
    end: end,
    line: line,
    col: col,
    items: items,
  };
}

export function makeExprObject(props, start, end, line, col) {
  return {
    kind: E_OBJECT,
    start: start,
    end: end,
    line: line,
    col: col,
    props: props,
  };
}

export function makeExprFunction(func, start, end, line, col) {
  return {
    kind: E_FUNCTION,
    start: start,
    end: end,
    line: line,
    col: col,
    func: func,
  };
}

export function makeExprArrow(arrow, start, end, line, col) {
  return {
    kind: E_ARROW,
    start: start,
    end: end,
    line: line,
    col: col,
    arrow: arrow,
  };
}

export function makeExprClass(cls, start, end, line, col) {
  return {
    kind: E_CLASS,
    start: start,
    end: end,
    line: line,
    col: col,
    cls: cls,
  };
}

export function makeExprUnary(op, arg, start, end, line, col) {
  return {
    kind: E_UNARY,
    start: start,
    end: end,
    line: line,
    col: col,
    op: op,
    arg: arg,
  };
}

export function makeExprUpdate(op, prefix, arg, start, end, line, col) {
  return {
    kind: E_UPDATE,
    start: start,
    end: end,
    line: line,
    col: col,
    op: op,
    prefix: prefix,
    arg: arg,
  };
}

export function makeExprBinary(op, left, right, start, end, line, col) {
  return {
    kind: E_BINARY,
    start: start,
    end: end,
    line: line,
    col: col,
    op: op,
    left: left,
    right: right,
  };
}

export function makeExprLogical(op, left, right, start, end, line, col) {
  return {
    kind: E_LOGICAL,
    start: start,
    end: end,
    line: line,
    col: col,
    op: op,
    left: left,
    right: right,
  };
}

export function makeExprAssign(op, target, value, start, end, line, col) {
  return {
    kind: E_ASSIGN,
    start: start,
    end: end,
    line: line,
    col: col,
    op: op,
    target: target,
    value: value,
  };
}

export function makeExprCond(test, cons, alt, start, end, line, col) {
  return {
    kind: E_COND,
    start: start,
    end: end,
    line: line,
    col: col,
    test: test,
    cons: cons,
    alt: alt,
  };
}

export function makeExprCall(callee, args, optional, start, end, line, col) {
  return {
    kind: E_CALL,
    start: start,
    end: end,
    line: line,
    col: col,
    callee: callee,
    args: args,
    optional: optional,
  };
}

export function makeExprNew(callee, args, start, end, line, col) {
  return {
    kind: E_NEW,
    start: start,
    end: end,
    line: line,
    col: col,
    callee: callee,
    args: args,
  };
}

export function makeExprMember(object, prop, optional, start, end, line, col) {
  return {
    kind: E_MEMBER,
    start: start,
    end: end,
    line: line,
    col: col,
    object: object,
    prop: prop,
    optional: optional,
  };
}

export function makeExprOptChain(expr, start, end, line, col) {
  return {
    kind: E_OPT_CHAIN,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makeExprSeq(items, start, end, line, col) {
  return {
    kind: E_SEQ,
    start: start,
    end: end,
    line: line,
    col: col,
    items: items,
  };
}

export function makeExprSpread(arg, start, end, line, col) {
  return {
    kind: E_SPREAD,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makeExprYield(arg, delegate, start, end, line, col) {
  return {
    kind: E_YIELD,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
    delegate: delegate,
  };
}

export function makeExprAwait(arg, start, end, line, col) {
  return {
    kind: E_AWAIT,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makeExprNewTarget(start, end, line, col) {
  return {
    kind: E_NEW_TARGET,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeExprImportMeta(start, end, line, col) {
  return {
    kind: E_IMPORT_META,
    start: start,
    end: end,
    line: line,
    col: col,
  };
}

export function makeExprImport(source, options, start, end, line, col) {
  return {
    kind: E_IMPORT,
    start: start,
    end: end,
    line: line,
    col: col,
    source: source,
    options: options,
  };
}

export function makeExprParen(expr, start, end, line, col) {
  return {
    kind: E_PAREN,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makeExprPrivateIn(name, right, start, end, line, col) {
  return {
    kind: E_PRIVATE_IN,
    start: start,
    end: end,
    line: line,
    col: col,
    name: name,
    right: right,
  };
}

export function makePropName(ident, start, end, line, col) {
  return {
    kind: MP_NAME,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makePropComputed(expr, start, end, line, col) {
  return {
    kind: MP_COMPUTED,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makePropPrivate(ident, start, end, line, col) {
  return {
    kind: MP_PRIVATE,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeTemplate(quasis, exprs, start, end, line, col) {
  return {
    kind: N_TEMPLATE,
    start: start,
    end: end,
    line: line,
    col: col,
    quasis: quasis,
    exprs: exprs,
  };
}

export function makeTemplateElement(cooked, raw, start, end, line, col) {
  return {
    kind: N_TEMPLATE_ELEMENT,
    start: start,
    end: end,
    line: line,
    col: col,
    cooked: cooked,
    raw: raw,
  };
}

export function makeObjPropInit(key, value, start, end, line, col) {
  return {
    kind: PR_INIT,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    value: value,
  };
}

export function makeObjPropShorthand(ident, start, end, line, col) {
  return {
    kind: PR_SHORTHAND,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeObjPropMethod(key, methodKind, value, start, end, line, col) {
  return {
    kind: PR_METHOD,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    methodKind: methodKind,
    value: value,
  };
}

export function makeObjPropSpread(arg, start, end, line, col) {
  return {
    kind: PR_SPREAD,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makePatIdent(name, start, end, line, col) {
  return {
    kind: PT_IDENT,
    start: start,
    end: end,
    line: line,
    col: col,
    name: name,
  };
}

export function makePatArray(items, start, end, line, col) {
  return {
    kind: PT_ARRAY,
    start: start,
    end: end,
    line: line,
    col: col,
    items: items,
  };
}

export function makePatObject(props, start, end, line, col) {
  return {
    kind: PT_OBJECT,
    start: start,
    end: end,
    line: line,
    col: col,
    props: props,
  };
}

export function makePatAssign(target, def, start, end, line, col) {
  return {
    kind: PT_ASSIGN,
    start: start,
    end: end,
    line: line,
    col: col,
    target: target,
    def: def,
  };
}

export function makePatRest(arg, start, end, line, col) {
  return {
    kind: PT_REST,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makePatExpr(expr, start, end, line, col) {
  return {
    kind: PT_EXPR,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}

export function makePatPropProp(key, value, shorthand, start, end, line, col) {
  return {
    kind: PP_PROP,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    value: value,
    shorthand: shorthand,
  };
}

export function makePatPropRest(arg, start, end, line, col) {
  return {
    kind: PP_REST,
    start: start,
    end: end,
    line: line,
    col: col,
    arg: arg,
  };
}

export function makeImportDecl(specifiers, source, attributes, start, end, line, col) {
  return {
    kind: N_IMPORT_DECL,
    start: start,
    end: end,
    line: line,
    col: col,
    specifiers: specifiers,
    source: source,
    attributes: attributes,
  };
}

export function makeStrLit(value, start, end, line, col) {
  return {
    kind: N_STR_LIT,
    start: start,
    end: end,
    line: line,
    col: col,
    value: value,
  };
}

export function makeImportAttribute(key, value, start, end, line, col) {
  return {
    kind: N_IMPORT_ATTRIBUTE,
    start: start,
    end: end,
    line: line,
    col: col,
    key: key,
    value: value,
  };
}

export function makeModuleNameIdent(ident, start, end, line, col) {
  return {
    kind: MN_IDENT,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeModuleNameStr(str, start, end, line, col) {
  return {
    kind: MN_STR,
    start: start,
    end: end,
    line: line,
    col: col,
    str: str,
  };
}

export function makeImportSpecDefault(ident, start, end, line, col) {
  return {
    kind: IS_DEFAULT,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeImportSpecNamespace(ident, start, end, line, col) {
  return {
    kind: IS_NAMESPACE,
    start: start,
    end: end,
    line: line,
    col: col,
    ident: ident,
  };
}

export function makeImportSpecNamed(imported, local, start, end, line, col) {
  return {
    kind: IS_NAMED,
    start: start,
    end: end,
    line: line,
    col: col,
    imported: imported,
    local: local,
  };
}

export function makeExportNamed(specifiers, source, attributes, start, end, line, col) {
  return {
    kind: ED_NAMED,
    start: start,
    end: end,
    line: line,
    col: col,
    specifiers: specifiers,
    source: source,
    attributes: attributes,
  };
}

export function makeExportAll(exported, source, attributes, start, end, line, col) {
  return {
    kind: ED_ALL,
    start: start,
    end: end,
    line: line,
    col: col,
    exported: exported,
    source: source,
    attributes: attributes,
  };
}

export function makeExportDecl(stmt, start, end, line, col) {
  return {
    kind: ED_DECL,
    start: start,
    end: end,
    line: line,
    col: col,
    stmt: stmt,
  };
}

export function makeExportDefault(def, start, end, line, col) {
  return {
    kind: ED_DEFAULT,
    start: start,
    end: end,
    line: line,
    col: col,
    def: def,
  };
}

export function makeExportSpec(local, exported, start, end, line, col) {
  return {
    kind: N_EXPORT_SPEC,
    start: start,
    end: end,
    line: line,
    col: col,
    local: local,
    exported: exported,
  };
}

export function makeExportDefaultFunction(func, start, end, line, col) {
  return {
    kind: EX_FUNCTION,
    start: start,
    end: end,
    line: line,
    col: col,
    func: func,
  };
}

export function makeExportDefaultClass(cls, start, end, line, col) {
  return {
    kind: EX_CLASS,
    start: start,
    end: end,
    line: line,
    col: col,
    cls: cls,
  };
}

export function makeExportDefaultExpr(expr, start, end, line, col) {
  return {
    kind: EX_EXPR,
    start: start,
    end: end,
    line: line,
    col: col,
    expr: expr,
  };
}
