// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// The canonical dump of a syntax tree: the S-expression format of
// `refdump --ast` (tools/refdump/FORMAT.md), so the engine's parser and
// the reference can be compared with a plain diff. Spans are left out
// unless asked for; then every node that has one in the reference gets
// `@START..END:LINE:COL` after its kind, with byte offsets when a byte
// table (from byteOffsets in token_dump.js) is given.

import {
  S_EXPR,
  S_VAR,
  S_FUNCTION,
  S_CLASS,
  S_BLOCK,
  S_IF,
  S_FOR,
  S_FOR_IN,
  S_FOR_OF,
  S_WHILE,
  S_DO_WHILE,
  S_CONTINUE,
  S_BREAK,
  S_RETURN,
  S_WITH,
  S_SWITCH,
  S_LABELED,
  S_THROW,
  S_TRY,
  S_IMPORT,
  S_EXPORT,
  FI_VAR,
  FH_VAR,
  AB_EXPR,
  CM_METHOD,
  CM_FIELD,
  PK_STR,
  PK_NUM,
  PK_BIGINT,
  PK_COMPUTED,
  E_IDENT,
  E_BOOL,
  E_NUM,
  E_BIGINT,
  E_STR,
  E_REGEX,
  E_TEMPLATE,
  E_TAGGED,
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
  E_IMPORT,
  E_PAREN,
  E_PRIVATE_IN,
  MP_COMPUTED,
  PR_INIT,
  PR_SHORTHAND,
  PR_METHOD,
  PT_IDENT,
  PT_ARRAY,
  PT_OBJECT,
  PT_ASSIGN,
  PT_REST,
  PP_PROP,
  MN_IDENT,
  IS_NAMED,
  ED_NAMED,
  ED_ALL,
  ED_DECL,
  EX_FUNCTION,
  EX_CLASS,
} from "./ast.js";
import { quoteUnits, numberText } from "./token_dump.js";

const KIND_NAMES = [
  "Program",
  "Ident",
  "Stmt.Expr",
  "Stmt.Var",
  "Stmt.Function",
  "Stmt.Class",
  "Stmt.Block",
  "Stmt.Empty",
  "Stmt.If",
  "Stmt.For",
  "Stmt.ForIn",
  "Stmt.ForOf",
  "Stmt.While",
  "Stmt.DoWhile",
  "Stmt.Continue",
  "Stmt.Break",
  "Stmt.Return",
  "Stmt.With",
  "Stmt.Switch",
  "Stmt.Labeled",
  "Stmt.Throw",
  "Stmt.Try",
  "Stmt.Debugger",
  "Stmt.Import",
  "Stmt.Export",
  "VarDecl",
  "VarDeclarator",
  "ForInit.Var",
  "ForInit.Expr",
  "ForHead.Var",
  "ForHead.Pat",
  "SwitchCase",
  "CatchClause",
  "Function",
  "ArrowFunction",
  "ArrowBody.Expr",
  "ArrowBody.Block",
  "Class",
  "ClassMember.Method",
  "ClassMember.Field",
  "ClassMember.StaticBlock",
  "PropKey.Ident",
  "PropKey.Str",
  "PropKey.Num",
  "PropKey.BigInt",
  "PropKey.Computed",
  "PropKey.Private",
  "Expr.Ident",
  "Expr.This",
  "Expr.Super",
  "Expr.Null",
  "Expr.Bool",
  "Expr.Num",
  "Expr.BigInt",
  "Expr.Str",
  "Expr.Regex",
  "Expr.Template",
  "Expr.TaggedTemplate",
  "Expr.Array",
  "Expr.Object",
  "Expr.Function",
  "Expr.Arrow",
  "Expr.Class",
  "Expr.Unary",
  "Expr.Update",
  "Expr.Binary",
  "Expr.Logical",
  "Expr.Assign",
  "Expr.Cond",
  "Expr.Call",
  "Expr.New",
  "Expr.Member",
  "Expr.OptionalChain",
  "Expr.Seq",
  "Expr.Spread",
  "Expr.Yield",
  "Expr.Await",
  "Expr.NewTarget",
  "Expr.ImportMeta",
  "Expr.Import",
  "Expr.Paren",
  "Expr.PrivateIn",
  "MemberProp.Name",
  "MemberProp.Computed",
  "MemberProp.Private",
  "Template",
  "TemplateElement",
  "Prop.Init",
  "Prop.Shorthand",
  "Prop.Method",
  "Prop.Spread",
  "Pat.Ident",
  "Pat.Array",
  "Pat.Object",
  "Pat.Assign",
  "Pat.Rest",
  "Pat.Expr",
  "PatProp.Prop",
  "PatProp.Rest",
  "ImportDecl",
  "StrLit",
  "ImportAttribute",
  "ModuleName.Ident",
  "ModuleName.Str",
  "ImportSpec.Default",
  "ImportSpec.Namespace",
  "ImportSpec.Named",
  "ExportDecl.Named",
  "ExportDecl.All",
  "ExportDecl.Decl",
  "ExportDecl.Default",
  "ExportSpec",
  "ExportDefault.Function",
  "ExportDefault.Class",
  "ExportDefault.Expr",
];

const UNARY_OPS = ["Minus", "Plus", "Not", "BitNot", "Typeof", "Void", "Delete"];
const UPDATE_OPS = ["Inc", "Dec"];
const BINARY_OPS = [
  "Eq",
  "NotEq",
  "StrictEq",
  "StrictNotEq",
  "Lt",
  "LtEq",
  "Gt",
  "GtEq",
  "Shl",
  "Shr",
  "UShr",
  "Add",
  "Sub",
  "Mul",
  "Div",
  "Mod",
  "Exp",
  "BitOr",
  "BitXor",
  "BitAnd",
  "In",
  "Instanceof",
];
const LOGICAL_OPS = ["And", "Or", "Nullish"];
const ASSIGN_OPS = [
  "Assign",
  "Add",
  "Sub",
  "Mul",
  "Div",
  "Mod",
  "Exp",
  "Shl",
  "Shr",
  "UShr",
  "BitAnd",
  "BitOr",
  "BitXor",
  "And",
  "Or",
  "Nullish",
];
const METHOD_KINDS = ["Method", "Get", "Set", "Constructor"];
const VAR_KINDS = ["var", "let", "const"];

// The name of a node kind, as the dump writes it.
export function kindName(kind) {
  return KIND_NAMES[kind];
}

function bool(b) {
  if (b) {
    return "true";
  }
  return "false";
}

// ---- the writer ----

// A writer. `out(line)` receives each finished line; `spans` asks for
// spans; `bytes` maps code-unit offsets to byte offsets (null: keep
// code-unit offsets).
function newDump(out, spans, bytes) {
  return { out: out, spans: spans, bytes: bytes, line: "", started: false, indent: 0 };
}

function offset(d, i) {
  if (d.bytes === null) {
    return i;
  }
  return d.bytes[i];
}

// Start a node: a new line, indented, with `(KIND` and the span of `n`
// when spans are on and `n` is not null.
function open(d, kind, n) {
  if (d.started) {
    d.out(d.line);
  }
  d.started = true;
  let s = "";
  let i = 0;
  while (i < d.indent) {
    s += "  ";
    i += 1;
  }
  s += "(" + kind;
  if (d.spans && n !== null) {
    s += ` @${offset(d, n.start)}..${offset(d, n.end)}:${n.line}:${n.col}`;
  }
  d.line = s;
  d.indent += 1;
}

function attr(d, text) {
  d.line += " " + text;
}

function close(d) {
  d.indent -= 1;
  d.line += ")";
}

function leaf(d, kind, n) {
  open(d, kind, n);
  close(d);
}

function none(d) {
  leaf(d, "None", null);
}

// ---- nodes ----

// Dump a program; calls out(line) for each line.
export function dumpProgram(prog, spans, bytes, out) {
  const d = newDump(out, spans, bytes);
  open(d, "Program", prog);
  if (prog.module) {
    attr(d, "module");
  } else {
    attr(d, "script");
  }
  attr(d, "strict=" + bool(prog.strict));
  stmts(d, "body", prog.body);
  close(d);
  d.out(d.line);
}

function stmts(d, name, list) {
  open(d, name, null);
  for (const s of list) {
    stmt(d, s);
  }
  close(d);
}

function exprs(d, name, list) {
  open(d, name, null);
  for (const e of list) {
    expr(d, e);
  }
  close(d);
}

function pats(d, name, list) {
  open(d, name, null);
  for (const p of list) {
    pat(d, p);
  }
  close(d);
}

function ident(d, i) {
  open(d, "Ident", i);
  attr(d, quoteUnits(i.name));
  close(d);
}

function optIdent(d, i) {
  if (i === null) {
    none(d);
  } else {
    ident(d, i);
  }
}

function optExpr(d, e) {
  if (e === null) {
    none(d);
  } else {
    expr(d, e);
  }
}

function stmt(d, s) {
  const k = s.kind;
  open(d, KIND_NAMES[k], s);
  if (k === S_EXPR) {
    if (s.directive !== null) {
      attr(d, "directive=" + quoteUnits(s.directive));
    }
    expr(d, s.expr);
  } else if (k === S_VAR) {
    varDecl(d, s.decl);
  } else if (k === S_FUNCTION) {
    func(d, s.func);
  } else if (k === S_CLASS) {
    cls(d, s.cls);
  } else if (k === S_BLOCK) {
    stmts(d, "body", s.body);
  } else if (k === S_IF) {
    expr(d, s.test);
    stmt(d, s.cons);
    if (s.alt === null) {
      none(d);
    } else {
      stmt(d, s.alt);
    }
  } else if (k === S_FOR) {
    if (s.init === null) {
      none(d);
    } else if (s.init.kind === FI_VAR) {
      open(d, "ForInit.Var", null);
      varDecl(d, s.init.decl);
      close(d);
    } else {
      open(d, "ForInit.Expr", null);
      expr(d, s.init.expr);
      close(d);
    }
    optExpr(d, s.test);
    optExpr(d, s.update);
    stmt(d, s.body);
  } else if (k === S_FOR_IN) {
    forHead(d, s.left);
    expr(d, s.right);
    stmt(d, s.body);
  } else if (k === S_FOR_OF) {
    attr(d, "await=" + bool(s.isAwait));
    forHead(d, s.left);
    expr(d, s.right);
    stmt(d, s.body);
  } else if (k === S_WHILE) {
    expr(d, s.test);
    stmt(d, s.body);
  } else if (k === S_DO_WHILE) {
    stmt(d, s.body);
    expr(d, s.test);
  } else if (k === S_CONTINUE || k === S_BREAK) {
    optIdent(d, s.label);
  } else if (k === S_RETURN) {
    optExpr(d, s.arg);
  } else if (k === S_WITH) {
    expr(d, s.object);
    stmt(d, s.body);
  } else if (k === S_SWITCH) {
    expr(d, s.disc);
    open(d, "cases", null);
    for (const c of s.cases) {
      open(d, "SwitchCase", c);
      optExpr(d, c.test);
      stmts(d, "body", c.body);
      close(d);
    }
    close(d);
  } else if (k === S_LABELED) {
    ident(d, s.label);
    stmt(d, s.body);
  } else if (k === S_THROW) {
    expr(d, s.arg);
  } else if (k === S_TRY) {
    stmts(d, "block", s.block);
    if (s.handler === null) {
      none(d);
    } else {
      const h = s.handler;
      open(d, "CatchClause", h);
      if (h.param === null) {
        none(d);
      } else {
        pat(d, h.param);
      }
      stmts(d, "body", h.body);
      close(d);
    }
    if (s.finalizer === null) {
      none(d);
    } else {
      stmts(d, "finalizer", s.finalizer);
    }
  } else if (k === S_IMPORT) {
    importDecl(d, s.decl);
  } else if (k === S_EXPORT) {
    exportDecl(d, s.decl);
  }
  close(d);
}

function varDecl(d, v) {
  open(d, "VarDecl", v);
  attr(d, VAR_KINDS[v.varKind]);
  for (const x of v.decls) {
    open(d, "VarDeclarator", x);
    pat(d, x.target);
    optExpr(d, x.init);
    close(d);
  }
  close(d);
}

function forHead(d, h) {
  if (h.kind === FH_VAR) {
    open(d, "ForHead.Var", null);
    varDecl(d, h.decl);
  } else {
    open(d, "ForHead.Pat", null);
    pat(d, h.pat);
  }
  close(d);
}

function func(d, f) {
  open(d, "Function", f);
  attr(
    d,
    `async=${bool(f.isAsync)} generator=${bool(f.isGenerator)} strict=${bool(f.strict)} simple_params=${bool(f.simpleParams)}`,
  );
  optIdent(d, f.id);
  pats(d, "params", f.params);
  stmts(d, "body", f.body);
  close(d);
}

function cls(d, c) {
  open(d, "Class", c);
  optIdent(d, c.id);
  optExpr(d, c.superClass);
  open(d, "body", null);
  for (const m of c.body) {
    open(d, KIND_NAMES[m.kind], m);
    if (m.kind === CM_METHOD) {
      attr(d, `kind=${METHOD_KINDS[m.methodKind]} static=${bool(m.isStatic)}`);
      propKey(d, m.key);
      func(d, m.value);
    } else if (m.kind === CM_FIELD) {
      attr(d, "static=" + bool(m.isStatic));
      propKey(d, m.key);
      optExpr(d, m.value);
    } else {
      stmts(d, "body", m.body);
    }
    close(d);
  }
  close(d);
  close(d);
}

function bigintText(radix, digits) {
  let s = `${radix}:`;
  for (const u of digits) {
    s += String.fromCharCode(u);
  }
  return s;
}

function propKey(d, k) {
  const kind = k.kind;
  if (kind === PK_STR) {
    open(d, "PropKey.Str", k);
    attr(d, quoteUnits(k.value));
  } else if (kind === PK_NUM) {
    open(d, "PropKey.Num", k);
    attr(d, numberText(k.value));
  } else if (kind === PK_BIGINT) {
    open(d, "PropKey.BigInt", k);
    attr(d, bigintText(k.radix, k.digits));
  } else if (kind === PK_COMPUTED) {
    open(d, "PropKey.Computed", null);
    expr(d, k.expr);
  } else {
    open(d, KIND_NAMES[kind], null);
    ident(d, k.ident);
  }
  close(d);
}

function template(d, t) {
  open(d, "Template", null);
  open(d, "quasis", null);
  for (const q of t.quasis) {
    open(d, "TemplateElement", q);
    if (q.cooked === null) {
      attr(d, "cooked=None");
    } else {
      attr(d, "cooked=" + quoteUnits(q.cooked));
    }
    attr(d, "raw=" + quoteUnits(q.raw));
    close(d);
  }
  close(d);
  exprs(d, "exprs", t.exprs);
  close(d);
}

function expr(d, e) {
  const k = e.kind;
  open(d, KIND_NAMES[k], e);
  if (k === E_IDENT || k === E_STR) {
    if (k === E_IDENT) {
      attr(d, quoteUnits(e.name));
    } else {
      attr(d, quoteUnits(e.value));
    }
  } else if (k === E_BOOL) {
    attr(d, bool(e.value));
  } else if (k === E_NUM) {
    attr(d, numberText(e.value));
  } else if (k === E_BIGINT) {
    attr(d, bigintText(e.radix, e.digits));
  } else if (k === E_REGEX) {
    attr(d, quoteUnits(e.pattern));
    attr(d, quoteUnits(e.flags));
  } else if (k === E_TEMPLATE) {
    template(d, e.quasi);
  } else if (k === E_TAGGED) {
    expr(d, e.tag);
    template(d, e.quasi);
  } else if (k === E_ARRAY) {
    for (const x of e.items) {
      if (x === null) {
        leaf(d, "Hole", null);
      } else {
        expr(d, x);
      }
    }
  } else if (k === E_OBJECT) {
    for (const p of e.props) {
      objProp(d, p);
    }
  } else if (k === E_FUNCTION) {
    func(d, e.func);
  } else if (k === E_ARROW) {
    const a = e.arrow;
    open(d, "ArrowFunction", a);
    attr(d, `async=${bool(a.isAsync)} strict=${bool(a.strict)}`);
    pats(d, "params", a.params);
    if (a.body.kind === AB_EXPR) {
      open(d, "ArrowBody.Expr", null);
      expr(d, a.body.expr);
    } else {
      open(d, "ArrowBody.Block", null);
      stmts(d, "body", a.body.body);
    }
    close(d);
    close(d);
  } else if (k === E_CLASS) {
    cls(d, e.cls);
  } else if (k === E_UNARY) {
    attr(d, "op=" + UNARY_OPS[e.op]);
    expr(d, e.arg);
  } else if (k === E_UPDATE) {
    attr(d, `op=${UPDATE_OPS[e.op]} prefix=${bool(e.prefix)}`);
    expr(d, e.arg);
  } else if (k === E_BINARY || k === E_LOGICAL) {
    if (k === E_BINARY) {
      attr(d, "op=" + BINARY_OPS[e.op]);
    } else {
      attr(d, "op=" + LOGICAL_OPS[e.op]);
    }
    expr(d, e.left);
    expr(d, e.right);
  } else if (k === E_ASSIGN) {
    attr(d, "op=" + ASSIGN_OPS[e.op]);
    pat(d, e.target);
    expr(d, e.value);
  } else if (k === E_COND) {
    expr(d, e.test);
    expr(d, e.cons);
    expr(d, e.alt);
  } else if (k === E_CALL) {
    attr(d, "optional=" + bool(e.optional));
    expr(d, e.callee);
    exprs(d, "args", e.args);
  } else if (k === E_NEW) {
    expr(d, e.callee);
    exprs(d, "args", e.args);
  } else if (k === E_MEMBER) {
    attr(d, "optional=" + bool(e.optional));
    expr(d, e.object);
    const p = e.prop;
    open(d, KIND_NAMES[p.kind], null);
    if (p.kind === MP_COMPUTED) {
      expr(d, p.expr);
    } else {
      ident(d, p.ident);
    }
    close(d);
  } else if (k === E_OPT_CHAIN || k === E_PAREN) {
    expr(d, e.expr);
  } else if (k === E_SEQ) {
    for (const x of e.items) {
      expr(d, x);
    }
  } else if (k === E_SPREAD || k === E_AWAIT) {
    expr(d, e.arg);
  } else if (k === E_YIELD) {
    attr(d, "delegate=" + bool(e.delegate));
    optExpr(d, e.arg);
  } else if (k === E_IMPORT) {
    expr(d, e.source);
    optExpr(d, e.options);
  } else if (k === E_PRIVATE_IN) {
    ident(d, e.name);
    expr(d, e.right);
  }
  close(d);
}

function objProp(d, p) {
  const k = p.kind;
  open(d, KIND_NAMES[k], p);
  if (k === PR_INIT) {
    propKey(d, p.key);
    expr(d, p.value);
  } else if (k === PR_SHORTHAND) {
    ident(d, p.ident);
  } else if (k === PR_METHOD) {
    attr(d, "kind=" + METHOD_KINDS[p.methodKind]);
    propKey(d, p.key);
    func(d, p.value);
  } else {
    expr(d, p.arg);
  }
  close(d);
}

function pat(d, p) {
  const k = p.kind;
  open(d, KIND_NAMES[k], p);
  if (k === PT_IDENT) {
    attr(d, quoteUnits(p.name));
  } else if (k === PT_ARRAY) {
    for (const x of p.items) {
      if (x === null) {
        leaf(d, "Hole", null);
      } else {
        pat(d, x);
      }
    }
  } else if (k === PT_OBJECT) {
    for (const pp of p.props) {
      if (pp.kind === PP_PROP) {
        open(d, "PatProp.Prop", null);
        attr(d, "shorthand=" + bool(pp.shorthand));
        propKey(d, pp.key);
        pat(d, pp.value);
      } else {
        open(d, "PatProp.Rest", null);
        pat(d, pp.arg);
      }
      close(d);
    }
  } else if (k === PT_ASSIGN) {
    pat(d, p.target);
    expr(d, p.def);
  } else if (k === PT_REST) {
    pat(d, p.arg);
  } else {
    expr(d, p.expr);
  }
  close(d);
}

function strLit(d, s) {
  open(d, "StrLit", s);
  attr(d, quoteUnits(s.value));
  close(d);
}

function moduleName(d, m) {
  if (m.kind === MN_IDENT) {
    open(d, "ModuleName.Ident", null);
    ident(d, m.ident);
  } else {
    open(d, "ModuleName.Str", null);
    strLit(d, m.str);
  }
  close(d);
}

function attributes(d, list) {
  open(d, "attributes", null);
  for (const a of list) {
    open(d, "ImportAttribute", null);
    moduleName(d, a.key);
    strLit(d, a.value);
    close(d);
  }
  close(d);
}

function importDecl(d, i) {
  open(d, "ImportDecl", null);
  open(d, "specifiers", null);
  for (const s of i.specifiers) {
    open(d, KIND_NAMES[s.kind], null);
    if (s.kind === IS_NAMED) {
      moduleName(d, s.imported);
      ident(d, s.local);
    } else {
      ident(d, s.ident);
    }
    close(d);
  }
  close(d);
  strLit(d, i.source);
  attributes(d, i.attributes);
  close(d);
}

function exportDecl(d, e) {
  const k = e.kind;
  open(d, KIND_NAMES[k], null);
  if (k === ED_NAMED) {
    open(d, "specifiers", null);
    for (const s of e.specifiers) {
      open(d, "ExportSpec", null);
      moduleName(d, s.local);
      moduleName(d, s.exported);
      close(d);
    }
    close(d);
    if (e.source === null) {
      none(d);
    } else {
      strLit(d, e.source);
    }
    attributes(d, e.attributes);
  } else if (k === ED_ALL) {
    if (e.exported === null) {
      none(d);
    } else {
      moduleName(d, e.exported);
    }
    strLit(d, e.source);
    attributes(d, e.attributes);
  } else if (k === ED_DECL) {
    stmt(d, e.stmt);
  } else {
    const x = e.def;
    open(d, KIND_NAMES[x.kind], null);
    if (x.kind === EX_FUNCTION) {
      func(d, x.func);
    } else if (x.kind === EX_CLASS) {
      cls(d, x.cls);
    } else {
      expr(d, x.expr);
    }
    close(d);
  }
  close(d);
}
