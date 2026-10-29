// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Core JS code emitter: turns a lowered syntax tree (ast.js) into Core JS
// source text ready for the Anvil VM compiler and x86-64 JIT.

import {
  S_EXPR,
  S_VAR,
  S_FUNCTION,
  S_CLASS,
  S_BLOCK,
  S_EMPTY,
  S_IF,
  S_FOR,
  S_FOR_IN,
  S_FOR_OF,
  S_WHILE,
  S_DO_WHILE,
  S_CONTINUE,
  S_BREAK,
  S_RETURN,
  S_SWITCH,
  S_LABELED,
  S_THROW,
  S_TRY,
  S_EXPORT,
  FI_VAR,
  FH_VAR,
  AB_EXPR,
  CM_METHOD,
  PK_IDENT,
  PK_STR,
  PK_NUM,
  E_IDENT,
  E_THIS,
  E_SUPER,
  E_NULL,
  E_BOOL,
  E_NUM,
  E_BIGINT,
  E_STR,
  E_REGEX,
  E_TEMPLATE,
  E_ARRAY,
  E_OBJECT,
  E_FUNCTION,
  E_ARROW,
  E_CLASS,
  E_UNARY,
  E_UPDATE,
  E_BINARY,
  E_LOGICAL,
  E_ASSIGN,
  E_COND,
  E_CALL,
  E_NEW,
  E_MEMBER,
  E_OPT_CHAIN,
  E_SEQ,
  E_SPREAD,
  E_YIELD,
  E_AWAIT,
  E_NEW_TARGET,
  E_PAREN,
  MP_NAME,
  MP_COMPUTED,
  MP_PRIVATE,
  PR_INIT,
  PR_SHORTHAND,
  PR_METHOD,
  PT_IDENT,
  PT_EXPR,
  ED_DECL,
  ED_DEFAULT,
  EX_FUNCTION,
  EX_CLASS,
  MK_CONSTRUCTOR,
  U_MINUS,
  U_PLUS,
  U_NOT,
  U_BIT_NOT,
  U_TYPEOF,
  U_VOID,
  U_DELETE,
  UP_INC,
  VK_VAR,
  VK_LET,
} from "../syntax/ast.js";
import { quoteUnits, numberText } from "../syntax/token_dump.js";

const BIN_TEXT = [
  "==",
  "!=",
  "===",
  "!==",
  "<",
  "<=",
  ">",
  ">=",
  "<<",
  ">>",
  ">>>",
  "+",
  "-",
  "*",
  "/",
  "%",
  "**",
  "|",
  "^",
  "&",
  "in",
  "instanceof",
];

const LOG_TEXT = ["&&", "||", "??"];

const ASSIGN_TEXT = [
  "=",
  "+=",
  "-=",
  "*=",
  "/=",
  "%=",
  "**=",
  "<<=",
  ">>=",
  ">>>=",
  "&=",
  "|=",
  "^=",
  "&&=",
  "||=",
  "??=",
];

export function unitsToText(units) {
  let s = "";
  for (const u of units) {
    s += String.fromCharCode(u);
  }
  return s;
}

function varKeyword(vk) {
  if (vk === VK_VAR) {
    return "var";
  }
  if (vk === VK_LET) {
    return "let";
  }
  return "const";
}

function propKeyName(key) {
  if (key.kind === PK_IDENT) {
    return unitsToText(key.ident.name);
  }
  if (key.kind === PK_STR) {
    return unitsToText(key.value);
  }
  if (key.kind === PK_NUM) {
    return numberText(key.value);
  }
  return "_key";
}

function emitParams(params) {
  let s = "";
  let i = 0;
  while (i < params.length) {
    if (i > 0) {
      s += ", ";
    }
    const p = params[i];
    if (p.kind === PT_IDENT) {
      s += unitsToText(p.name);
    } else {
      s += `__p${i}`;
    }
    i += 1;
  }
  return s;
}

function emitBodyList(stmts) {
  let out = "";
  for (const st of stmts) {
    const code = emitStmt(st);
    if (code.length > 0) {
      out += code + "\n";
    }
  }
  return out;
}

function emitFunctionDecl(f, asExpr) {
  let name = "";
  if (f.id !== null) {
    name = " " + unitsToText(f.id.name);
  } else if (!asExpr) {
    name = " __anon_fn";
  }
  const params = emitParams(f.params);
  const body = emitBodyList(f.body);
  return `function${name}(${params}) {\n${body}}`;
}

function emitClassDecl(c, asExpr) {
  let name = "";
  if (c.id !== null) {
    name = " " + unitsToText(c.id.name);
  } else if (!asExpr) {
    name = " __AnonClass";
  }
  let ext = "";
  if (c.superClass !== null) {
    ext = " extends " + emitAtom(c.superClass);
  }
  let members = "";
  for (const m of c.body) {
    if (m.kind === CM_METHOD && !m.isStatic) {
      const mname = m.methodKind === MK_CONSTRUCTOR ? "constructor" : propKeyName(m.key);
      const params = emitParams(m.value.params);
      const body = emitBodyList(m.value.body);
      members += `  ${mname}(${params}) {\n${body}  }\n`;
    }
  }
  return `class${name}${ext} {\n${members}}`;
}

function emitTarget(t) {
  if (t.kind === PT_IDENT) {
    return unitsToText(t.name);
  }
  if (t.kind === PT_EXPR) {
    return emitTargetExpr(t.expr);
  }
  return emitTargetExpr(t);
}

function emitTargetExpr(e) {
  let x = e;
  while (x.kind === E_PAREN) {
    x = x.expr;
  }
  if (x.kind === E_IDENT) {
    return unitsToText(x.name);
  }
  if (x.kind === E_MEMBER) {
    const obj = emitAtom(x.object);
    const p = x.prop;
    if (p.kind === MP_NAME) {
      return `${obj}.${unitsToText(p.ident.name)}`;
    }
    if (p.kind === MP_PRIVATE) {
      return `${obj}.__priv_${unitsToText(p.ident.name)}`;
    }
    if (p.kind === MP_COMPUTED) {
      return `${obj}[${emitExpr(p.expr)}]`;
    }
  }
  return emitExpr(x);
}

function isSimpleAtom(e) {
  const k = e.kind;
  return (
    k === E_IDENT ||
    k === E_THIS ||
    k === E_SUPER ||
    k === E_NULL ||
    k === E_BOOL ||
    k === E_STR ||
    k === E_ARRAY ||
    k === E_MEMBER ||
    k === E_CALL ||
    k === E_NEW
  );
}

function emitAtom(e) {
  if (e.kind === E_PAREN) {
    return emitAtom(e.expr);
  }
  if (e.kind === E_NUM) {
    if (e.value < 0 || (e.value === 0 && 1 / e.value < 0)) {
      return `(${numberText(e.value)})`;
    }
    return numberText(e.value);
  }
  if (isSimpleAtom(e)) {
    return emitExpr(e);
  }
  return `(${emitExpr(e)})`;
}

function emitTemplateExpr(t) {
  const quasis = t.quasis;
  const exprs = t.exprs;
  if (exprs.length === 0) {
    const q0 = quasis[0];
    return quoteUnits(q0.cooked !== null ? q0.cooked : q0.raw);
  }
  let out = "(";
  let i = 0;
  while (i < quasis.length) {
    if (i > 0) {
      out += " + ";
    }
    const q = quasis[i];
    const units = q.cooked !== null ? q.cooked : q.raw;
    out += quoteUnits(units);
    if (i < exprs.length) {
      out += " + (\"\" + (" + emitExpr(exprs[i]) + "))";
    }
    i += 1;
  }
  return out + ")";
}

export function emitExpr(e) {
  const k = e.kind;
  if (k === E_IDENT) {
    return unitsToText(e.name);
  }
  if (k === E_THIS) {
    return "this";
  }
  if (k === E_SUPER) {
    return "super";
  }
  if (k === E_NULL) {
    return "null";
  }
  if (k === E_BOOL) {
    return e.value ? "true" : "false";
  }
  if (k === E_NUM) {
    if (e.value === 0 && 1 / e.value < 0) {
      return "(-0)";
    }
    return numberText(e.value);
  }
  if (k === E_BIGINT) {
    return unitsToText(e.digits);
  }
  if (k === E_STR) {
    return quoteUnits(e.value);
  }
  if (k === E_REGEX) {
    return `{ source: ${quoteUnits(e.pattern)}, flags: ${quoteUnits(e.flags)} }`;
  }
  if (k === E_TEMPLATE) {
    return emitTemplateExpr(e.quasi);
  }
  if (k === E_ARRAY) {
    let s = "[";
    let i = 0;
    while (i < e.items.length) {
      if (i > 0) {
        s += ", ";
      }
      const item = e.items[i];
      if (item === null) {
        s += "undefined";
      } else if (item.kind === E_SPREAD) {
        s += emitExpr(item.arg);
      } else {
        s += emitExpr(item);
      }
      i += 1;
    }
    return s + "]";
  }
  if (k === E_OBJECT) {
    let s = "{";
    let first = true;
    for (const p of e.props) {
      if (!first) {
        s += ", ";
      }
      first = false;
      if (p.kind === PR_INIT) {
        s += `${propKeyName(p.key)}: ${emitExpr(p.value)}`;
      } else if (p.kind === PR_SHORTHAND) {
        const id = unitsToText(p.ident.name);
        s += `${id}: ${id}`;
      } else if (p.kind === PR_METHOD) {
        s += `${propKeyName(p.key)}: ${emitFunctionDecl(p.value, true)}`;
      }
    }
    return s + "}";
  }
  if (k === E_FUNCTION) {
    return `(${emitFunctionDecl(e.func, true)})`;
  }
  if (k === E_ARROW) {
    const a = e.arrow;
    const params = emitParams(a.params);
    if (a.body.kind === AB_EXPR) {
      const b = a.body.expr;
      if (b.kind === E_OBJECT) {
        return `((${params}) => (${emitExpr(b)}))`;
      }
      return `((${params}) => ${emitExpr(b)})`;
    }
    return `((${params}) => {\n${emitBodyList(a.body.body)}})`;
  }
  if (k === E_CLASS) {
    return `(${emitClassDecl(e.cls, true)})`;
  }
  if (k === E_UNARY) {
    const op = e.op;
    if (op === U_VOID) {
      return `((${emitExpr(e.arg)}), undefined)`;
    }
    if (op === U_DELETE) {
      return `((${emitExpr(e.arg)}), true)`;
    }
    if (op === U_PLUS) {
      return `(0 + (${emitExpr(e.arg)}))`;
    }
    if (op === U_MINUS) {
      return `(-${emitAtom(e.arg)})`;
    }
    if (op === U_NOT) {
      return `(!${emitAtom(e.arg)})`;
    }
    if (op === U_BIT_NOT) {
      return `(~${emitAtom(e.arg)})`;
    }
    if (op === U_TYPEOF) {
      return `(typeof ${emitAtom(e.arg)})`;
    }
  }
  if (k === E_UPDATE) {
    const opStr = e.op === UP_INC ? "++" : "--";
    const tgt = emitTargetExpr(e.arg);
    if (e.prefix) {
      return `${opStr}${tgt}`;
    }
    return `${tgt}${opStr}`;
  }
  if (k === E_BINARY) {
    const opStr = BIN_TEXT[e.op];
    return `(${emitAtom(e.left)} ${opStr} ${emitAtom(e.right)})`;
  }
  if (k === E_LOGICAL) {
    const opStr = LOG_TEXT[e.op];
    return `(${emitAtom(e.left)} ${opStr} ${emitAtom(e.right)})`;
  }
  if (k === E_ASSIGN) {
    const opStr = ASSIGN_TEXT[e.op];
    return `${emitTarget(e.target)} ${opStr} ${emitExpr(e.value)}`;
  }
  if (k === E_COND) {
    return `(${emitAtom(e.test)} ? ${emitAtom(e.cons)} : ${emitAtom(e.alt)})`;
  }
  if (k === E_CALL) {
    let calleeStr = "";
    if (
      e.callee.kind === E_IDENT ||
      e.callee.kind === E_MEMBER ||
      e.callee.kind === E_SUPER
    ) {
      calleeStr = emitExpr(e.callee);
    } else {
      calleeStr = `(${emitExpr(e.callee)})`;
    }
    let argsStr = "";
    let i = 0;
    while (i < e.args.length) {
      if (i > 0) {
        argsStr += ", ";
      }
      const a = e.args[i];
      if (a.kind === E_SPREAD) {
        argsStr += emitExpr(a.arg);
      } else {
        argsStr += emitExpr(a);
      }
      i += 1;
    }
    return `${calleeStr}(${argsStr})`;
  }
  if (k === E_NEW) {
    const calleeStr = emitAtom(e.callee);
    let argsStr = "";
    let i = 0;
    while (i < e.args.length) {
      if (i > 0) {
        argsStr += ", ";
      }
      const a = e.args[i];
      if (a.kind === E_SPREAD) {
        argsStr += emitExpr(a.arg);
      } else {
        argsStr += emitExpr(a);
      }
      i += 1;
    }
    return `new ${calleeStr}(${argsStr})`;
  }
  if (k === E_MEMBER) {
    const objStr = emitAtom(e.object);
    const p = e.prop;
    if (p.kind === MP_NAME) {
      return `${objStr}.${unitsToText(p.ident.name)}`;
    }
    if (p.kind === MP_PRIVATE) {
      return `${objStr}.__priv_${unitsToText(p.ident.name)}`;
    }
    if (p.kind === MP_COMPUTED) {
      return `${objStr}[${emitExpr(p.expr)}]`;
    }
  }
  if (k === E_OPT_CHAIN || k === E_PAREN) {
    return `(${emitExpr(e.expr)})`;
  }
  if (k === E_SEQ) {
    let s = "(";
    let i = 0;
    while (i < e.items.length) {
      if (i > 0) {
        s += ", ";
      }
      s += emitExpr(e.items[i]);
      i += 1;
    }
    return s + ")";
  }
  if (k === E_SPREAD || k === E_AWAIT) {
    return emitExpr(e.arg);
  }
  if (k === E_YIELD) {
    if (e.arg === null) {
      return "undefined";
    }
    return emitExpr(e.arg);
  }
  if (k === E_NEW_TARGET) {
    return "undefined";
  }
  return "undefined";
}

function emitVarDecl(v, withSemi) {
  const kw = varKeyword(v.varKind);
  let s = kw + " ";
  let i = 0;
  while (i < v.decls.length) {
    if (i > 0) {
      s += ", ";
    }
    const d = v.decls[i];
    s += emitTarget(d.target);
    if (d.init !== null) {
      s += " = " + emitExpr(d.init);
    }
    i += 1;
  }
  if (withSemi) {
    s += ";";
  }
  return s;
}

function emitBlockWrapped(s) {
  if (s.kind === S_BLOCK) {
    return emitStmt(s);
  }
  return `{\n${emitStmt(s)}\n}`;
}

export function emitStmt(s) {
  const k = s.kind;
  if (k === S_EMPTY) {
    return "";
  }
  if (k === S_EXPR) {
    if (s.directive !== null) {
      return "";
    }
    const inner = s.expr;
    if (inner.kind === E_OBJECT || inner.kind === E_FUNCTION) {
      return `(${emitExpr(inner)});`;
    }
    return `${emitExpr(inner)};`;
  }
  if (k === S_VAR) {
    return emitVarDecl(s.decl, true);
  }
  if (k === S_FUNCTION) {
    return emitFunctionDecl(s.func, false);
  }
  if (k === S_CLASS) {
    return emitClassDecl(s.cls, false);
  }
  if (k === S_BLOCK) {
    return `{\n${emitBodyList(s.body)}}`;
  }
  if (k === S_IF) {
    let out = `if (${emitExpr(s.test)}) ${emitBlockWrapped(s.cons)}`;
    if (s.alt !== null) {
      if (s.alt.kind === S_IF) {
        out += ` else ${emitStmt(s.alt)}`;
      } else {
        out += ` else ${emitBlockWrapped(s.alt)}`;
      }
    }
    return out;
  }
  if (k === S_WHILE) {
    return `while (${emitExpr(s.test)}) ${emitBlockWrapped(s.body)}`;
  }
  if (k === S_DO_WHILE) {
    return `do ${emitBlockWrapped(s.body)} while (${emitExpr(s.test)});`;
  }
  if (k === S_FOR) {
    let initStr = "";
    if (s.init !== null) {
      if (s.init.kind === FI_VAR) {
        initStr = emitVarDecl(s.init.decl, false);
      } else {
        initStr = emitExpr(s.init.expr);
      }
    }
    const testStr = s.test !== null ? emitExpr(s.test) : "";
    const updStr = s.update !== null ? emitExpr(s.update) : "";
    return `for (${initStr}; ${testStr}; ${updStr}) ${emitBlockWrapped(s.body)}`;
  }
  if (k === S_FOR_OF || k === S_FOR_IN) {
    let headStr = "let __item";
    if (s.left.kind === FH_VAR) {
      const d = s.left.decl;
      const kw = varKeyword(d.varKind);
      const t = d.decls[0].target;
      headStr = `${kw} ${emitTarget(t)}`;
    } else {
      headStr = `let ${emitTarget(s.left.pat)}`;
    }
    return `for (${headStr} of ${emitExpr(s.right)}) ${emitBlockWrapped(s.body)}`;
  }
  if (k === S_CONTINUE) {
    if (s.label !== null) {
      return `continue ${unitsToText(s.label.name)};`;
    }
    return "continue;";
  }
  if (k === S_BREAK) {
    if (s.label !== null) {
      return `break ${unitsToText(s.label.name)};`;
    }
    return "break;";
  }
  if (k === S_RETURN) {
    if (s.arg === null) {
      return "return;";
    }
    return `return ${emitExpr(s.arg)};`;
  }
  if (k === S_THROW) {
    return `throw ${emitExpr(s.arg)};`;
  }
  if (k === S_TRY) {
    let out = `try {\n${emitBodyList(s.block)}}`;
    if (s.handler !== null) {
      const h = s.handler;
      let pstr = "__err";
      if (h.param !== null && h.param.kind === PT_IDENT) {
        pstr = unitsToText(h.param.name);
      }
      out += ` catch (${pstr}) {\n${emitBodyList(h.body)}}`;
    }
    if (s.finalizer !== null) {
      out += ` finally {\n${emitBodyList(s.finalizer)}}`;
    }
    return out;
  }
  if (k === S_SWITCH) {
    let casesStr = "";
    for (const c of s.cases) {
      if (c.test === null) {
        casesStr += "  default:\n";
      } else {
        casesStr += `  case ${emitExpr(c.test)}:\n`;
      }
      casesStr += emitBodyList(c.body);
    }
    return `switch (${emitExpr(s.disc)}) {\n${casesStr}}`;
  }
  if (k === S_LABELED) {
    return `${unitsToText(s.label.name)}: ${emitStmt(s.body)}`;
  }
  if (k === S_EXPORT) {
    const d = s.decl;
    if (d.kind === ED_DECL) {
      return emitStmt(d.stmt);
    }
    if (d.kind === ED_DEFAULT) {
      const x = d.def;
      if (x.kind === EX_FUNCTION) {
        return emitFunctionDecl(x.func, false);
      }
      if (x.kind === EX_CLASS) {
        return emitClassDecl(x.cls, false);
      }
      return `${emitExpr(x.expr)};`;
    }
    return "";
  }
  return "";
}

export function emitProgram(prog) {
  return emitBodyList(prog.body);
}
